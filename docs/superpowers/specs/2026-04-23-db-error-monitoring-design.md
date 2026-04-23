# CapyInn DB Error Monitoring Design

## Summary

CapyInn will extend the existing command-failure monitoring pipeline so database-related failures are grouped into a small set of stable labels.

This release keeps the user-facing error contract unchanged:

- frontend still receives the same `code`, `message`, `kind`, and `support_id`
- existing toasts and error formatting stay as-is
- the new DB grouping is added only to backend-owned monitoring data

The new grouping will apply only to the issue `#30` command-failure scope that already uses the shared monitoring pipeline:

- `check_in`
- `check_out`
- `create_reservation`
- `run_night_audit`

The product outcome is simple: when one of those commands fails for a database reason, the diagnostics stream should say whether the failure was caused by a constraint problem, a locked database, a missing record, a write failure, or an unknown database issue.

## Goals

- Reuse the existing `command failure` pipeline instead of creating a second database-error log.
- Keep the user-facing command error contract stable.
- Add one backend-owned source of truth for DB error grouping.
- Record a stable DB error group in local diagnostics for monitored commands.
- Preserve existing `support_id`-based root-cause logging for system failures.
- Avoid scattering SQLite string matching across multiple command files.

## Non-Goals

- No new frontend error codes.
- No new toast variants or UI copy changes.
- No rollout to commands outside the current command-failure monitoring scope.
- No broad refactor of all `sqlx` call sites across the app.
- No attempt to classify every possible SQLite error in the first release.
- No remote Sentry parity in this release.
- No app-wide change to the generic `log_system_error(...)` contract.

Remote reporting already exists for command failures, but DB grouping will stay backend-only for this release. Extending the same label to browser-side Sentry would require surfacing new backend metadata across the Tauri boundary or moving remote submission behind the backend. That is intentionally deferred to avoid contract churn while issue `#30` is still focused on practical diagnostics value.

## Current Baseline

The current command-failure pipeline already has the right shape:

- frontend wraps monitored commands through [invokeCommand.ts](/Users/binhan/HotelManager/mhm/src/lib/invokeCommand.ts)
- backend writes normalized command-failure records through [app_error.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/app_error.rs)
- local command failures are appended to [command_failure_log.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/command_failure_log.rs)
- system root causes are still written separately through `support-errors.jsonl`

The remaining gap is classification:

- monitored booking and audit flows collapse DB execution failures into broad system errors
- [domain/booking/error.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/domain/booking/error.rs) currently treats database failures as `BookingError::Database(String)`
- one-off DB recognition already exists in [room_management.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/room_management.rs) for unique-constraint handling, but that logic is local to one command area

This means the app can already tell that a monitored command failed, but it cannot yet tell whether the database problem was a locked file, a constraint violation, a missing record, or a generic write failure.

## Chosen Approach

CapyInn will add a shared backend classifier for monitored DB failures and record the classifier output inside the existing command-failure diagnostics record.

The design has four parts:

1. Introduce one shared DB error grouping helper in the backend.
2. Use that helper only from the monitored command-failure paths in this release.
3. Add optional DB monitoring metadata to the local command-failure record.
4. Keep the existing user-visible `CommandError` unchanged.

This is preferred over adding new frontend error codes because it gives clearer diagnostics without expanding the error contract, registry, and UI behavior at the same time.

## Rejected Alternatives

### Add New Command Error Codes For DB Groups

Rejected because it would force a wider change set:

- Rust error registry
- shared frontend error registry
- error normalization
- UI copy decisions
- tests for every new code path

That is more surface area than issue `#30` needs.

### Detect DB Groups Ad Hoc In Each Command File

Rejected because it creates copy-pasted string checks and inconsistent labels over time.

The existing `is_unique_constraint_error(...)` helper in `room_management.rs` is a good example of useful logic that should be moved toward a shared home rather than repeated.

### Build A Separate DB Monitoring Pipeline

Rejected because command-failure monitoring already provides:

- command name
- normalized code
- `kind`
- `correlation_id`
- optional `support_id`

Adding one DB grouping field to that record is enough for the current roadmap need.

## Scope

### Included Commands

- `check_in`
- `check_out`
- `create_reservation`
- `run_night_audit`

### Included Record Destinations

- local `command-failures.jsonl`
- backend support-error context for monitored system-failure paths only

### Explicitly Excluded

- browser-side Sentry event enrichment with DB grouping
- commands outside the monitored issue `#30` scope
- non-database validation and auth failures
- room management and other admin flows for this release

## DB Error Group Contract

### New Field

The local command-failure record gains one optional top-level field:

- `db_error_group`

If the failure is not database-related, the field is omitted.

If the failure is database-related, the field is one of:

- `constraint`
- `locked`
- `not_found`
- `write_failed`
- `unknown`

### Meaning Of Each Group

- `constraint`
  The database rejected the operation because stored-data rules were violated, such as a unique or foreign-key constraint.
- `locked`
  SQLite could not complete the operation because the database file or transaction state was locked.
- `not_found`
  The backend expected a record to exist in the database for the command to finish, but that record was not available.
- `write_failed`
  A database write operation failed for a reason that was not better explained as `constraint` or `locked`.
- `unknown`
  The failure is database-related, but the current release cannot safely place it in a more specific group.

### Why `not_found` Belongs Here

`not_found` in this design means a missing database record needed for command completion. It is not a general UI-level “item not found” label.

Some record-missing cases still map to user-facing codes like `ROOM_NOT_FOUND` or `BOOKING_NOT_FOUND`. Those stay unchanged. The new DB grouping may still be written to the local command-failure record for those failures, because the monitoring goal is to classify the backend cause without changing the UI contract.

Support-log enrichment is narrower:

- local command-failure record may carry `db_error_group=not_found` for both user-facing and system-facing monitored failures
- support-log context only carries `db_error_group` when the monitored failure already goes through the system-error path

## Record Shape Changes

### Local Command Failure Record

The current record shape remains valid. This release adds one optional field:

- `db_error_group`

Example:

```json
{
  "schema_version": 2,
  "timestamp": "2026-04-23T09:10:11Z",
  "command": "create_reservation",
  "code": "SYSTEM_INTERNAL_ERROR",
  "kind": "system",
  "correlation_id": "COR-1A2B3C4D",
  "support_id": "SUP-ABCD1234",
  "db_error_group": "locked",
  "context": {
    "room_id": "R101",
    "check_in_date": "2026-04-23",
    "check_out_date": "2026-04-25",
    "nights": 2,
    "deposit_present": true,
    "source": "walk_in",
    "notes_present": false
  }
}
```

### Schema Version

Because the serialized JSONL shape changes, `schema_version` should increment from `1` to `2`.

This keeps downstream parsing honest:

- old records remain readable as version `1`
- new records explicitly signal support for `db_error_group`

## Backend Design

### Shared Classifier

Add a new backend helper that accepts normalized internal failure information and returns an optional DB group.

The helper owns all first-release recognition rules. Command handlers should not perform raw message matching directly.

The classifier input should support both of these sources:

- low-level database execution failures
- record-missing failures that reach the monitored command-failure path

### Internal Types

This release should introduce a small internal model for classification, for example:

- `DbErrorGroup`
- `DbFailureKind`

`DbFailureKind` exists to separate “true DB engine failure” from “record missing while fulfilling a DB-backed command.” The exact Rust names can differ, but the separation should remain.

One workable shape is:

- database engine error with the original message
- database-backed record missing

That keeps the classifier logic explicit and avoids pretending that every `not_found` is a raw SQLite engine error.

### Booking And Audit Error Mapping

The monitored booking and audit command mappers already act as the narrowing point between domain errors and command-failure monitoring:

- [rooms.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/rooms.rs)
- [reservations.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/reservations.rs)
- [audit.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/audit.rs)

Those mappers should continue deciding:

- which failures are user-facing
- which failures are system-facing

They will now also pass optional DB failure metadata into the monitoring pipeline before the final command-failure record is written.

For monitored system failures that also need support-log enrichment, the same metadata must be available before the support record is written. In practice, this means monitored flows should use a monitored-only helper or wrapper that combines:

- DB group classification
- support-log write
- `CommandError` creation

This avoids widening the generic `log_system_error(...)` path across unrelated commands.

### Structured Path For `write_failed`

`write_failed` should not be assigned from raw message guessing alone.

To keep the design logically sound, the monitored flows need a reliable signal for whether a failure happened during a write step. The design therefore adds a small amount of internal structure for monitored DB failures:

- read-path database failures stay `unknown` unless they match `constraint` or `locked`
- write-path database failures map to `write_failed` when they are not better classified as `constraint` or `locked`

The write/read distinction should be attached at the backend call site that already knows whether it is performing an insert, update, delete, or transaction-commit operation. This avoids fragile text matching like “guess from the error string whether a write happened.”

If a write path discovers that the required record is missing, `not_found` takes precedence over `write_failed`. The rule is to classify by the clearest final cause, not by the fact that the code was in a write section.

### Minimal Service-Layer Change

To support the write/read distinction without a broad refactor, the release should only add explicit DB-failure wrapping in the monitored booking and audit service paths.

That means:

- no app-wide `sqlx` wrapper
- no change to every command in the repo
- only the monitored service functions attach the small amount of extra structure needed for classification

## Classification Rules

### `constraint`

Match when the DB failure clearly signals a constraint violation, including common SQLite messages for:

- unique constraint failure
- foreign key constraint failure
- check constraint failure
- not-null constraint failure

### `locked`

Match when the DB failure clearly signals a lock or busy-state problem, including common SQLite messages for:

- database is locked
- database table is locked
- database is busy

### `not_found`

Match when a monitored command determines that a required database-backed record was missing.

This can still apply even if the final `CommandError` returned to the frontend is a user-facing code such as `ROOM_NOT_FOUND` or `BOOKING_NOT_FOUND`.

This rule uses explicit internal failure kind, not SQLite string matching.

### `write_failed`

Match when:

- the failure happened during a known write step
- the failure is not better explained as `not_found`
- it did not already match `constraint`
- it did not already match `locked`

### `unknown`

Fallback when:

- the failure is DB-related
- the current release cannot safely classify it more specifically

## Pipeline Integration

### Command Failure Recording

`record_command_failure(...)` should accept optional DB monitoring metadata and serialize `db_error_group` when present.

The monitoring write path remains:

1. command handler catches domain error
2. command handler maps it to `CommandError`
3. command handler forwards optional DB failure metadata
4. backend writes one command-failure record

### System Support Logging

When a monitored system failure is DB-related, the support log context should also include the same DB group label.

This should be implemented through monitored-only plumbing used by the issue `#30` commands, not by widening the generic `log_system_error(...)` behavior for the whole app.

That keeps the shallow record and deep root-cause record aligned without changing the user-facing error shape or broadening scope beyond the monitored commands.

## File-Level Change Plan

Primary files expected to change:

- [mhm/src-tauri/src/app_error.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/app_error.rs)
- [mhm/src-tauri/src/command_failure_log.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/command_failure_log.rs)
- [mhm/src-tauri/src/domain/booking/error.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/domain/booking/error.rs)
- [mhm/src-tauri/src/commands/rooms.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/rooms.rs)
- [mhm/src-tauri/src/commands/reservations.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/reservations.rs)
- [mhm/src-tauri/src/commands/audit.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/audit.rs)

Optional extraction target if it keeps `app_error.rs` focused:

- `mhm/src-tauri/src/db_error_monitoring.rs`
- `mhm/src-tauri/src/commands/monitored_system_error.rs`

That extraction is preferred if the helper would otherwise make `app_error.rs` carry too many unrelated responsibilities or if monitored system-error handling needs to stay separate from the app-wide helper path.

## Testing

### Unit Tests

- DB classifier maps known constraint messages to `constraint`
- DB classifier maps known lock messages to `locked`
- record-missing monitored failures map to `not_found`
- known write-step DB failures map to `write_failed`
- unrecognized DB failures map to `unknown`

### Serialization Tests

- `CommandFailureRecord` omits `db_error_group` for non-DB failures
- `CommandFailureRecord` includes `db_error_group` for DB failures
- `schema_version` is `2` for the new record shape

### Command-Level Tests

Add or update tests in the monitored command files so the command-failure log contains the expected DB group for representative failures in scope.

The release does not need exhaustive coverage for every SQLite message variant. It does need one proving test for each supported group.

## Rollout Notes

- Start with the monitored issue `#30` commands only.
- Keep the classifier helper reusable so future commands can opt in.
- If later product needs justify it, browser-side Sentry can be enriched in a follow-up once there is a deliberate plan for exposing backend-owned DB metadata across the command boundary.

## Risks And Mitigations

### Risk: Over-Classifying From Fragile String Matching

Mitigation:

- keep message matching narrow
- prefer explicit failure kind where possible
- fall back to `unknown` instead of guessing

### Risk: `write_failed` Becomes A Catch-All Label

Mitigation:

- only assign `write_failed` when the code already knows the failed step was a write
- let `not_found` win when a missing record is the clearer cause
- otherwise use `unknown`

### Risk: Backend And Support Logs Drift

Mitigation:

- derive both command-failure and support-log DB group from the same shared helper

## Success Criteria

- monitored DB failures are grouped into stable labels in local diagnostics
- the shared UI error contract remains unchanged
- grouping logic lives in one backend-owned helper instead of scattered message checks
- monitored commands in issue `#30` can prove the grouping through tests
