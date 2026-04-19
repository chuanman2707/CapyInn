# CapyInn Backup Subsystem Refactor Design

Date: 2026-04-18
Owner: Codex
Status: Draft approved for spec write-up

## Goal

Refactor the current backup subsystem so `mhm/src-tauri/src/backup.rs` stops acting as a god file while preserving the existing external behavior of CapyInn's autobackup feature.

The refactor should make four things true:

- the backup subsystem has clear module boundaries for coordination, backup execution, filesystem policy, and event payloads
- external callers continue to use a small stable facade instead of depending on internal backup details
- existing backup behavior remains unchanged unless a cleanup is explicitly required to preserve correctness during the split
- tests become easier to reason about because concurrency, snapshot creation, and filesystem rules are no longer mixed into one file

This design is intentionally a refactor of the existing backup implementation. It does not add new product capabilities.

## User Decisions Locked In

The following engineering decisions were explicitly chosen during brainstorming and are part of this spec:

- refactor scope: keep the public cross-module backup API stable wherever practical
- allowed change scope: internal backup APIs may be redesigned to create cleaner boundaries
- preferred decomposition model: one facade module plus focused submodules
- priority: architectural clarity and testability over preserving the current internal call graph
- risk policy: do not change business behavior, event contract, retention policy, naming format, or shutdown semantics unless required to preserve current correctness

## Constraints

- `mhm/src-tauri/src/backup.rs` is currently a single file of roughly 1,100+ lines and contains:
  - public types and errors
  - backup status payload construction
  - job coordination and shutdown draining
  - SQLite snapshot execution
  - backup filename parsing and retention
  - test fixtures and all subsystem tests
- external callers currently depend on a small portion of that file:
  - `BackupCoordinator` from `mhm/src-tauri/src/lib.rs`
  - `request_backup(...)` from command modules
  - `drain_and_backup_on_exit(...)` from app shutdown handling
  - `BackupReason`
  - `log_backup_request_error(...)`
- the existing backup subsystem already has behavior-sensitive tests for:
  - shutdown admission
  - queued backup serialization
  - event ordering and `pending_jobs`
  - reservation lock collision handling
  - retention pruning
  - SQLite snapshot correctness
- the current refactor must not introduce product drift relative to the already-approved filesystem autobackup design in `docs/superpowers/specs/2026-04-18-filesystem-autobackup-design.md`
- where the approved autobackup design and the shipped implementation differ today, this refactor preserves shipped behavior and records the mismatch explicitly instead of using the refactor to make product-policy changes
- this work is purely backend Rust restructuring inside `mhm/src-tauri/src/`

## Chosen Approach

Keep a thin `backup` facade and split the current god file into focused submodules that each own one concern:

- shared types and errors
- event payload construction and emission
- filesystem naming, reservation, and pruning rules
- SQLite snapshot execution
- backup job coordination and shutdown drain logic

Why this approach:

- it attacks the actual problem, which is responsibility sprawl inside one file
- it preserves the small public surface already used by the rest of the app
- it minimizes blast radius outside the backup subsystem
- it creates better isolation for both tests and future changes without turning the subsystem into an abstraction-heavy redesign

This is explicitly a structural refactor, not a service-model rewrite.

## Authoritative Behavior for This Refactor

This refactor is constrained by currently shipped behavior, not by aspirational behavior from earlier design text where the implementation later settled differently.

The following rules are authoritative for this refactor:

- collision-suffix backup files such as `capyinn_backup_manual_20260418_231500-1.db` remain managed backup files and remain eligible for retention pruning
- shutdown timeout behavior preserves the current shipped coordinator semantics; this refactor does not add new queued-job cancellation policy
- coordinator internals must remain testable with deterministic fake emitters and fake backup runners
- failure to emit `backup-status` remains best-effort and non-fatal to the backup request itself

Any attempt to align older design text with different runtime behavior is out of scope for this refactor and should be treated as a separate behavior-change proposal.

## Non-Goals

- changing when autobackups run
- changing the backup filename format
- changing retention count or retention policy
- renaming the `backup-status` event
- redesigning the frontend backup indicator UX
- switching away from `VACUUM INTO`
- introducing restore flows, scheduling, encryption, or cloud backup
- general cleanup unrelated to the backup subsystem

## Existing State

Today the backup subsystem is concentrated in `mhm/src-tauri/src/backup.rs`, which contains both policy and mechanism:

- backup request admission and shutdown drain policy
- backup lifecycle event payload creation
- SQLite snapshot execution
- filesystem reservation and pruning behavior
- helper parsing for managed backup files
- all unit and async tests for the subsystem

This concentration creates three concrete problems:

1. Changing one part of the subsystem requires loading unrelated concerns into context.
2. Tests for one behavior sit beside implementation details for unrelated behaviors, making intent harder to read.
3. Internal boundaries are implicit, so future edits are more likely to leak concurrency policy into filesystem code or vice versa.

The subsystem already works functionally. The issue is that the implementation shape makes safe maintenance harder than it needs to be.

## Target Structure

The refactor should move backup code to a dedicated module directory:

```text
mhm/src-tauri/src/backup/
  mod.rs
  types.rs
  events.rs
  storage.rs
  runner.rs
  coordinator.rs
  test_support.rs
```

Test layout should also be split so each concern is reviewed with the code it validates.

Chosen test grouping:

```text
backup/
  coordinator.rs
  runner.rs
  storage.rs
  test_support.rs
```

Each of `coordinator.rs`, `runner.rs`, and `storage.rs` should own focused inline `#[cfg(test)]` modules. Shared fixtures should move to `test_support.rs` only if duplication becomes material during the split.

## Module Boundaries

### 1. `backup/mod.rs`

`mod.rs` is the external facade for the rest of the app.

It should:

- define child modules
- re-export the public contract needed by other modules
- expose the existing top-level helper functions used by commands and app shutdown code

It should not contain coordination internals, storage internals, or test fixtures.

Expected outward-facing contract:

- `BackupCoordinator`
- `BackupReason`
- `BackupOutcome`
- `BackupPruneOutcome`
- `BackupError`
- `BackupRequestError`
- `BackupRequestErrorKind`
- `log_backup_request_error(...)`
- `request_backup(...)`
- `drain_and_backup_on_exit(...)`
- `run_backup_once(...)` if still needed outside the subsystem

### 2. `backup/types.rs`

This module owns shared data types and error types.

It should contain:

- `BackupReason`
- `BackupOutcome`
- `BackupPruneOutcome`
- `BackupError`
- `BackupRequestError`
- `BackupRequestErrorKind`
- shared formatting and conversion impls
- `log_backup_request_error(...)`

This module should not know about `AppHandle`, lock files, SQLite SQL text, or event names.

### 3. `backup/events.rs`

This module owns backup lifecycle payload construction and Tauri event emission.

It should contain:

- `BackupStatusPayload`
- constructors for `started`, `completed`, and `failed`
- a small helper to emit the `backup-status` event

This module should not run backups, manage queue admission, or inspect the filesystem.

### 4. `backup/storage.rs`

This module owns filesystem-facing backup policy and helpers.

It should contain:

- `build_backup_filename(...)`
- `is_managed_backup_file(...)`
- filename parsing helpers
- `BackupReservation`
- `prune_old_backups(...)`
- `sqlite_string_literal(...)`
- `sync_directory(...)`

This module owns the `.db`, `.db.tmp`, and `.db.lock` contract and must preserve existing behavior:

- reservation uses lock files to avoid collisions
- completed backups keep the same naming format
- collision-suffix backups such as `-1` are still treated as managed backup files
- retention only touches managed backup files
- directory sync behavior remains platform-appropriate

This module should not know about `AppHandle`, `pending_jobs`, or shutdown policy.

### 5. `backup/runner.rs`

This module owns one backup execution from database path to completed snapshot file.

It should contain:

- `run_backup_once(...)`
- internal helpers equivalent to today's `run_backup_once_at(...)`

Its responsibility is:

- ensure backup directory exists
- reserve a target path through storage
- execute `VACUUM INTO`
- finalize the temp file into the target path
- prune old backups afterward
- return `BackupOutcome`

This module should not emit UI events and should not make admission decisions.

### 6. `backup/coordinator.rs`

This module owns runtime coordination policy.

It should contain:

- `BackupCoordinator`
- request admission logic
- `pending_jobs` tracking
- serialized execution gate
- shutdown drain behavior
- calls to `events` and `runner`

This is the only place inside the subsystem that should coordinate:

- request rejection after shutdown begins
- started/completed/failed event order
- exit drain timeout policy
- the relationship between queued work and the final `app_exit` backup

It should not own filename parsing or SQLite string literal generation.

To preserve deterministic concurrency testing, `coordinator.rs` must keep internal seams that allow tests to inject:

- a fake event sink
- a fake backup future/runner
- an async hook before enqueue where needed to exercise shutdown races

The exact visibility can be crate-private or test-only, but the refactor must not collapse the coordinator into a shape that is only testable through real `AppHandle` emission and the real backup runner.

## Public API Stability

The refactor should preserve the current cross-module API shape unless the compiler or module organization makes a tiny facade-level adjustment necessary.

Expected stable behavior for external callers:

- commands still call one public `request_backup(...)`
- shutdown handling still calls one public `drain_and_backup_on_exit(...)`
- `lib.rs` still manages a `BackupCoordinator` in Tauri state
- `BackupReason` variants remain unchanged
- `log_backup_request_error(...)` keeps the same failure-vs-skip logging distinction

The point of this refactor is to change internal boundaries, not to spread backup details into more callers.

## Migration Plan

The split should happen in a low-risk sequence:

### Phase 1: Create the facade and shared types

- create `backup/`
- move shared public types and errors into `types.rs`
- make `mod.rs` re-export the current public contract
- keep behavior unchanged

### Phase 2: Extract storage logic

- move filename helpers, reservation logic, pruning, and path/fs helpers into `storage.rs`
- preserve current filename format and collision behavior exactly
- keep all storage-oriented tests green before moving on

### Phase 3: Extract runner logic

- move snapshot creation into `runner.rs`
- keep the same `VACUUM INTO` execution flow and finalization semantics
- preserve returned `BackupOutcome`

### Phase 4: Extract event helpers

- move `BackupStatusPayload` and emit helpers into `events.rs`
- preserve payload field names, state values, and event name

### Phase 5: Extract coordinator logic

- move `BackupCoordinator` and request/drain orchestration into `coordinator.rs`
- preserve queueing, admission, and timeout semantics
- keep the facade wrappers in `mod.rs`

### Phase 6: Re-home tests

- split tests by concern after the implementation modules are stable
- ensure no behavior-sensitive test is lost during reorganization

## Risk Areas

### 1. Shutdown and admission races

This is the highest-risk area because the current coordinator logic guards several subtle invariants:

- non-exit work is rejected after shutdown starts
- a request can still lose the race to shutdown after async pre-enqueue work
- queued work drains before the final `app_exit` backup
- the drain path uses one bounded timeout

The refactor must not weaken these semantics.

### 2. Event ordering and `pending_jobs`

The frontend-facing backup indicator depends on stable lifecycle sequencing.

The refactor must preserve:

- `started` before work begins
- `completed` or `failed` after work ends
- current `pending_jobs` semantics
- event name `backup-status`
- non-fatal event emission behavior; failure to emit must not convert a backup success into a backup failure

### 3. Filesystem reservation contract

The refactor must preserve:

- lock-file collision avoidance
- temp-file cleanup on drop/failure
- collision suffix behavior such as `-1`
- retention sorting semantics

### 4. Backup naming and backward compatibility

Existing backup files must continue to be recognized as managed backup files after the refactor. The filename parser and builder must therefore remain compatible with the already-shipped format.

This includes completed files that carry a collision suffix, which are already produced and tested by the shipped implementation.

## Verification Strategy

At minimum, the refactor must preserve today's backup behavior through automated verification.

Required verification after implementation:

- `cargo check --manifest-path mhm/src-tauri/Cargo.toml`
- `cargo test --manifest-path mhm/src-tauri/Cargo.toml backup`

Recommended full verification before merge:

- `cargo test --manifest-path mhm/src-tauri/Cargo.toml`

Behavior-sensitive tests that must remain covered:

- backup filename generation
- managed backup file detection
- snapshot database creation
- reservation collision handling
- concurrent backup serialization
- retention pruning
- shutdown request rejection
- skip-vs-failure request error typing
- queued backup event ordering with `pending_jobs`
- exit drain sequencing
- admission race lost to shutdown

## Blast Radius Expectations

Expected file changes should stay narrow:

- `mhm/src-tauri/src/backup/` new files
- `mhm/src-tauri/src/backup.rs` removed and replaced by `mhm/src-tauri/src/backup/mod.rs` as the thin facade
- small import-path adjustments in:
  - `mhm/src-tauri/src/lib.rs`
  - any backup-calling command modules if module paths need re-export cleanup

The refactor should not require domain, repository, query, or frontend changes.

## Definition of Done

This refactor is complete when all of the following are true:

- the backup subsystem is split into focused modules with the boundaries defined above
- external callers still use a small stable facade
- existing behavior-sensitive tests remain present and pass
- `cargo check` passes for the Tauri crate
- the resulting code is easier to read because each module has one clear responsibility
