# CapyInn Filesystem Autobackup Design

Date: 2026-04-18
Owner: Codex
Status: Draft approved for spec write-up

## Goal

Add an always-on automatic backup flow for CapyInn's local SQLite database so operators get a recent filesystem backup without manual action.

The feature should make two things true:

- the app continuously protects the local database through trigger-based backups
- the UI clearly communicates that the app is saving data when an automatic backup is running

This design is intentionally limited to local filesystem backups. It does not introduce cloud sync, external storage providers, or user-configurable scheduling.

## User Decisions Locked In

The following product decisions were explicitly chosen during brainstorming and are part of this spec:

- backup medium: local filesystem only
- backup scope: only the SQLite database file, not `Scans/`, `models/`, or the full runtime directory
- backup triggers:
  - app exit
  - successful `night audit`
  - successful `check-out`
  - successful `group check-out`
  - successful settings changes
- retention policy: keep the newest `30` backup files and delete older CapyInn backup files automatically
- cooldown: none
- configuration UX: none, feature is always on
- UI expectation: when backup is running, show a visible cloud-style saving indicator in the app shell so users understand data is being saved

## Constraints

- The current app is offline-first and uses a local SQLite database in `~/CapyInn/capyinn.db`
- The current database runs in `WAL` mode
- The current backup implementation is a manual file copy in `mhm/src-tauri/src/commands/audit.rs`
- Backup must not corrupt or partially copy a live WAL-backed database
- Backup failures must not roll back already-committed business operations
- Existing manual backup should not diverge into a separate implementation path
- The UI already uses `toast` notifications in the bottom-right corner, so the save indicator must avoid that area

## Chosen Approach

Introduce a dedicated Rust backup service that creates a SQLite-safe snapshot backup into `~/CapyInn/backups/`, emits global backup lifecycle events for the frontend, and is invoked automatically after selected successful mutations plus once during app shutdown.

Why this approach:

- it keeps the feature filesystem-only, matching the product decision
- it avoids unsafe raw copying of a live WAL-backed database
- it centralizes retention, event emission, and backup naming in one place
- it gives the frontend a clean global status signal instead of overloading per-screen loading states

This is explicitly not a configurable scheduler and not a cloud backup system.

## Non-Goals

- user-facing backup settings, toggles, or trigger customization
- backup encryption or password protection
- uploading backups to remote storage
- restoring backups from within the app in this pass
- backing up OCR scans, models, CSV exports, or other runtime assets
- deduplicating or coalescing frequent backup triggers

## Existing State

Today CapyInn already has:

- runtime root helpers in `mhm/src-tauri/src/app_identity.rs`
- a manual `backup_database` command that writes to `~/CapyInn/backups/`
- Tauri commands for `save_settings`, `check_out`, `group_checkout`, and `run_night_audit`
- an app shell in [App.tsx](/Users/binhan/HotelManager/mhm/src/App.tsx) that can host a global status component
- a `db-updated` event pattern that already informs the frontend about completed data mutations

The main design issue is that current backup logic uses `std::fs::copy()` on the database file while the app runs SQLite in WAL mode. That is not a reliable backup strategy for a live database because committed state may still reside in the WAL file. The new design replaces that raw copy path with a SQLite-safe snapshot flow.

## Architecture

### 1. Backend service

Create a new Rust module dedicated to backups, for example `mhm/src-tauri/src/backup.rs`.

Its responsibilities:

- create the backup directory if missing
- serialize backup work so only one backup runs at a time
- create a SQLite-consistent snapshot backup as a single `.db` file
- emit backup lifecycle events to the frontend
- enforce retention after each successful backup
- expose a shared entrypoint reusable by auto and manual backup flows

Recommended public API shape:

- `backup_now(reason: BackupReason, app: Option<&AppHandle>) -> Result<BackupResult, BackupError>`
- `backup_on_exit(app: &AppHandle) -> Result<(), BackupError>`
- `prune_old_backups(max_files: usize) -> Result<PruneResult, BackupError>`

The exact function names can change, but the design intent should remain: one centralized service owns the policy.

### 2. Backup mechanism

The service should create snapshot backups using a SQLite-safe mechanism rather than raw filesystem copy of `capyinn.db`.

Chosen mechanism:

- use `VACUUM INTO` as the required snapshot mechanism
- run it through a dedicated backup connection path, not through arbitrary feature command code
- write to a temporary file first, for example `capyinn_backup_checkout_20260418_232015.db.tmp`
- after the snapshot completes successfully, atomically rename the temp file to the final `.db` filename
- ensure the output is a single standalone `.db` file inside `~/CapyInn/backups/`

Design requirements for this step:

- the resulting backup must be readable without requiring sibling `-wal` or `-shm` files
- the mechanism must tolerate the app's normal WAL mode
- the operation must be isolated behind the backup service so the command layer never manipulates SQLite backup details directly
- if the runtime SQLite build does not support `VACUUM INTO`, backup must fail explicitly with a clear error instead of falling back to raw file copy
- the service should best-effort flush the completed temp file before rename so a finished backup is not only present in process memory

If implementation details require a short pre-backup checkpoint or a dedicated connection configuration, that belongs inside the service, not in feature commands.

### 3. Backup file naming

Backups should include both timestamp and trigger reason in the filename so operators can understand why a file exists.

Recommended format:

- `capyinn_backup_settings_20260418_231500.db`
- `capyinn_backup_checkout_20260418_232015.db`
- `capyinn_backup_group_checkout_20260418_233040.db`
- `capyinn_backup_night_audit_20260418_235900.db`
- `capyinn_backup_app_exit_20260419_000102.db`

This naming scheme also makes retention and debugging easier.

### 4. Retention

After every successful backup, the service scans `~/CapyInn/backups/` and deletes older CapyInn backup files until only the newest `30` remain.

Retention rules:

- only touch files matching this exact CapyInn backup pattern:
  - `^capyinn_backup_(settings|checkout|group_checkout|night_audit|app_exit|manual)_\d{8}_\d{6}\.db$`
- never delete unrelated user files placed in the backup directory
- prefer sorting by parsed timestamp from the filename
- if filename parsing fails for a matching file, fall back to modified time
- never prune `.tmp`, `.partial`, or any non-`.db` artifacts as part of normal retention

Temporary backup files created by the service should be deleted by the service itself on failure. Retention is only for completed backup files.

Retention failures should be treated as warnings, not as a failure of the just-created backup.

## Trigger Integration

### 1. Successful settings changes

After `save_settings` successfully persists the new value, trigger an autobackup with reason `settings`.

This should happen after the SQL write succeeds. If the settings write fails, no backup should run.

### 2. Successful checkout

After `check_out` completes successfully and after the domain/service mutation commits, trigger an autobackup with reason `checkout`.

### 3. Successful group checkout

After `group_checkout` completes successfully, trigger an autobackup with reason `group_checkout`.

### 4. Successful night audit

After `run_night_audit` completes successfully, trigger an autobackup with reason `night_audit`.

### 5. App exit

On app shutdown, CapyInn should run one final autobackup with reason `app_exit`.

Exit behavior policy:

- if a backup job is already running, the app waits for that job to finish
- if queued backup jobs already exist when shutdown begins, the app drains those queued jobs first in FIFO order
- after the queue is drained, the app runs exactly one final `app_exit` backup
- if no backup job is running and no queued jobs exist, the app starts the final `app_exit` backup immediately
- the shutdown path should have a bounded wait to avoid hanging forever on a broken filesystem
  - use one global shutdown budget for the entire drain-plus-exit-backup sequence, not a fresh timeout per queued job
  - if that global budget is exhausted, cancel any still-pending queued work, allow the in-flight job to stop best-effort, log the timeout, and let the app close
- if the exit backup fails, log the failure, emit a final error event if possible, and allow the app to close

The app should not silently skip already-queued backups or the final exit backup unless shutdown constraints make completion impossible.

## Concurrency Model

The backup service must serialize work with a mutex, queue, or equivalent single-flight mechanism.

Required behavior:

- never run two backups in parallel
- preserve trigger order well enough that the UI does not oscillate unpredictably
- if a second trigger arrives while one backup is already running, enqueue it rather than dropping it
- once shutdown has begun, reject any new non-exit backup requests that arrive afterward and log them as skipped due to shutdown mode

No cooldown is applied. If the user performs multiple triggering actions in quick succession, each should still produce its own retained backup file.

## Failure Policy

Business mutations remain the source of truth. Backup is a required post-success side effect, but not part of the business transaction boundary.

Therefore:

- if `save_settings`, `check_out`, `group_checkout`, or `run_night_audit` fails, no backup runs
- if one of those operations succeeds but autobackup fails, the successful operation stays committed
- backup failures must be logged with trigger reason and error detail
- the frontend should show a failure state, but the operation itself should still be treated as saved

This policy avoids data loss from rolling back business logic due to a local backup failure.

## Frontend UX

### 1. Global save indicator

Add a global save indicator component rendered by the app shell, not by individual feature screens.

Recommended placement:

- fixed near the top-right of the main content area, below or visually attached to the header
- outside the bottom-right toast area so it does not overlap Sonner notifications

Recommended visual states:

- `Saving...`
  - cloud icon
  - subtle spinner or pulse treatment
  - text such as `Dang sao luu du lieu...`
- `Saved`
  - cloud-check icon
  - brief success hold of roughly `1.5-2s`
  - text such as `Da sao luu`
- `Backup failed`
  - warning or cloud-off icon
  - visible failure styling
  - paired with a toast error

The indicator should be present enough to explain what the app is doing, but not modal and not blocking.

### 2. State ownership

The frontend should keep backup status in a dedicated global store or local root state. It should not reuse `useHotelStore.loading`, which currently represents screen-level action loading rather than system-level persistence.

This separation matters because:

- backup state spans the whole app
- multiple screens can trigger backups
- backup may continue briefly after a feature-level action already appears completed

### 3. Event contract

Use a single global Tauri event, for example `backup-status`, with a compact payload.

Recommended payload shape:

```ts
type BackupStatusPayload = {
  job_id: string;
  state: "started" | "completed" | "failed";
  reason: "settings" | "checkout" | "group_checkout" | "night_audit" | "app_exit" | "manual";
  pending_jobs: number;
  path?: string;
  message?: string;
};
```

Semantics:

- emit `started` when a specific backup job begins
- emit `completed` when that job's final `.db` file is fully written
- emit `failed` when that specific job fails
- `job_id` uniquely identifies one backup attempt across all three states
- `pending_jobs` is the total number of unfinished backup jobs immediately after the event is emitted
  - on `started`, this includes the currently running job plus any queued jobs behind it
  - on `completed` or `failed`, this includes only jobs still unfinished after the current job ended
- on `failed`, `path` should be omitted unless a final `.db` backup file actually exists; temporary file paths must never be surfaced to the frontend as successful artifacts

If multiple backups run back-to-back, the UI should keep showing the saving state until `pending_jobs` reaches `0`, rather than flashing start-stop for every file.

### 4. Manual backup button

The existing manual backup action in `Settings > Data & Backup` should reuse the same backend service and emit the same events, with reason `manual`.

That keeps the UX and reliability model consistent:

- one backup implementation
- one retention policy
- one saving indicator pattern

## Testing

### Backend tests

Add Rust tests covering:

- backup directory creation
- successful creation of a new backup file
- retention prunes down to `30` CapyInn backup files
- retention does not delete non-matching files
- serialized backup execution when two requests arrive close together
- business command success is preserved even if backup fails afterward

Where direct filesystem testing is cumbersome, test lower-level helpers independently and keep end-to-end filesystem coverage focused.

### Frontend tests

Add frontend tests covering:

- the app shell reacts to `backup-status` started, completed, and failed events
- the indicator renders in the expected location with the expected labels
- repeated events do not cause broken flicker behavior
- failure emits visible error feedback

Existing Tauri event mocks should be extended rather than bypassed.

### Manual verification

Verify the following flows locally:

1. Save a settings section and confirm a new backup file appears
2. Perform a normal checkout and confirm a new backup file appears
3. Perform a group checkout and confirm a new backup file appears
4. Run night audit and confirm a new backup file appears
5. Trigger more than `30` backups and confirm older CapyInn backups are pruned
6. Close the app and confirm a final `app_exit` backup appears
7. Confirm the UI shows `Saving...` while the backup is active

## Risks And Mitigations

### Risk: backup implementation still behaves incorrectly with a live WAL database

Mitigation:

- centralize snapshot creation in one backend service
- avoid raw file copying of only `capyinn.db`
- validate the resulting `.db` backup can be opened independently

### Risk: app shutdown hangs because exit backup never completes

Mitigation:

- add a bounded wait on shutdown
- log timeout failures and let the app exit

### Risk: UI save indicator becomes noisy because many triggers happen in sequence

Mitigation:

- use a stable global indicator instead of repeated toast-only feedback
- keep the saving state continuous across queued backups
- keep the success hold brief

### Risk: retention deletes user-created files

Mitigation:

- prune only files that match the CapyInn backup naming convention

## Deliverables

- a new centralized Rust backup service
- trigger integration for settings, checkout, group checkout, night audit, and app exit
- a shared manual backup path that uses the same service
- automatic retention of the newest `30` CapyInn backup files
- a global frontend saving indicator backed by backup lifecycle events
