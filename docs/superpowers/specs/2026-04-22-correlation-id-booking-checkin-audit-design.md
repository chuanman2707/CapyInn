# CapyInn Correlation ID For Booking, Check-In, Checkout, And Audit Design

## Summary

CapyInn will add one action-scoped correlation ID to the selected stay and audit flows under issue `#30`.

For this release, the scope is intentionally limited to:

- `check_in`
- `check_out`
- `group_checkin`
- `group_checkout`
- `run_night_audit`

Each time the operator runs one of those actions, the frontend will create a short correlation ID such as `COR-8F3A1C7D`, send it with the command, and keep that same ID attached to the failed action if the command rejects.

The backend will log the same ID on command start, command success, and command failure so developers can trace one operator action across the relevant log lines. The frontend will show the correlation ID when that action fails so support can ask for one visible reference even for ordinary business-rule errors.

This design deliberately keeps the FE/BE error contract from PR `#50` stable. `support_id` remains the system-error support code. Correlation ID is a separate action-tracking value and is not persisted to database records in this release.

## Goals

- Give every selected operator action one visible tracking ID from the moment the command is sent.
- Make backend logs for the selected flows easy to search by one shared ID.
- Show one tracking ID in the UI when an action fails, including user-facing business errors that do not produce `support_id`.
- Reuse the shared command-error foundation from PR `#50` instead of creating a second error system.
- Bring `run_night_audit` onto the same shared command-error path as the already migrated stay and group flows.

## Non-Goals

- No schema migration and no new database columns for correlation IDs.
- No persistence of correlation IDs into `bookings`, `booking_groups`, `audit_logs`, or any other business record.
- No full-app rollout across every Tauri command in this release.
- No new diagnostics screen or UI for searching correlation IDs.
- No replacement of `support_id`; it remains the identifier for system-error support lookup.
- No global Tauri command middleware layer. This release applies explicit instrumentation only to the selected commands.

## Current Baseline

PR `#50` already created a stable FE/BE error contract around:

- `code`
- `message`
- `kind`
- `support_id`

That rollout already covers the main stay and group command paths used by:

- [useHotelStore.ts](/Users/binhan/HotelManager/mhm/src/stores/useHotelStore.ts)
- [rooms.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/rooms.rs)
- [groups.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/groups.rs)

However, the current code still has three gaps relative to issue `#30`:

- there is no action-scoped ID shared between frontend and backend for `check_in`, `check_out`, `group_checkin`, or `group_checkout`
- backend system-error logs can include `support_id`, but there is no single ID that also covers user-facing business errors
- [audit.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/audit.rs) and [NightAudit.tsx](/Users/binhan/HotelManager/mhm/src/pages/NightAudit.tsx) still use raw string-error handling instead of the shared command wrapper from PR `#50`

This means support can get stuck in two common cases:

- the operator sees a business error such as invalid state or missing booking and has no stable tracking value to report
- backend logs for one failed action must be pieced together from timestamps and message text instead of one action ID

## Chosen Approach

CapyInn will add a frontend-generated correlation ID for the selected flows and thread it through the command call explicitly.

The chosen approach has five parts:

1. The frontend generates one correlation ID per selected action attempt.
2. The shared command wrapper sends that ID with the Tauri command.
3. The shared command wrapper re-attaches that same ID to the local thrown error object if the command fails.
4. The selected backend commands log that ID on start, success, and failure.
5. The selected frontend screens and stores show that ID in error text for failed actions.

This is the preferred approach because it guarantees the UI and backend refer to the same action even if the backend returns a malformed or legacy error payload. The frontend created the ID before the call, so it never loses that reference.

## Rejected Alternatives

### Reuse `support_id` As The Only Tracking Value

Rejected because `support_id` is intentionally tied to system errors in PR `#50`.

If CapyInn reused `support_id` as the only tracking value here, one of two bad outcomes would follow:

- ordinary user-facing business errors would still have no visible tracking ID
- or `support_id` would have to change meaning and start appearing on non-system errors, which would blur the contract just standardized in PR `#50`

### Let The Backend Generate Correlation IDs On Its Own

Rejected because the current app has no single backend wrapper around all Tauri commands where this can be added cleanly for only the selected flows.

Frontend would still need extra work to display the ID on failure, and malformed backend errors would still risk losing the action reference before the UI could show it.

### Persist Correlation IDs Into Booking Or Audit Records

Rejected for this release because it expands the scope too far:

- schema or storage questions appear immediately
- historical-data semantics become part of the design
- the issue asks first for observability, not for durable business reporting

This release only needs action-level tracing, not long-term record attribution.

## Correlation ID Model

### Purpose

Correlation ID is the identifier for one operator action attempt.

Examples:

- one click on `Check-in`
- one click on `Check-out`
- one click on `Group Check-in`
- one click on `Group Checkout`
- one click on `Run Night Audit`

If the operator retries after a failure, that retry must receive a new correlation ID. The ID belongs to an action attempt, not to a booking or audit entity.

### Format

For this release, the frontend will generate IDs in this format:

- prefix: `COR-`
- suffix: 8 uppercase hexadecimal characters

Example:

- `COR-8F3A1C7D`

The value only needs to be short, human-readable, and collision-resistant enough for local support and log lookup. This is not a security token.

### Source Of Truth

The frontend is the source of truth for correlation ID generation in this release.

Rules:

- create the ID immediately before sending the selected command
- pass it through the command call as `correlationId`
- if the command rejects, keep the same ID attached to the local thrown error object
- do not regenerate the ID during error formatting

## Relationship To `support_id`

Correlation ID and `support_id` serve different purposes and must both remain clear.

`support_id`:

- exists only for `system` errors in the PR `#50` contract
- is created by backend error handling
- is the support reference for an unexpected system failure

Correlation ID:

- exists for every selected action attempt, whether the error is `user` or `system`
- is created by the frontend before the command is sent
- is the action reference for log tracing and user-visible error follow-up

In a failed system error case, the UI may show both values:

- one `support_id` for the system failure
- one correlation ID for the action attempt

This is acceptable because the two IDs answer different questions:

- `support_id`: which internal system failure record is this
- correlation ID: which operator action are we talking about

## Scope

### Included Commands

This release applies only to:

- `check_in`
- `check_out`
- `group_checkin`
- `group_checkout`
- `run_night_audit`

### Included Frontend Call Sites

This release applies only to the frontend paths that either send those commands or render their failure toasts:

- [useHotelStore.ts](/Users/binhan/HotelManager/mhm/src/stores/useHotelStore.ts) for stay and group command dispatch
- [CheckinSheet.tsx](/Users/binhan/HotelManager/mhm/src/components/CheckinSheet.tsx) for `check_in` failure display
- [RoomDetailPanel.tsx](/Users/binhan/HotelManager/mhm/src/components/RoomDetailPanel.tsx) for `check_out` failure display
- [RoomDrawer.tsx](/Users/binhan/HotelManager/mhm/src/components/RoomDrawer.tsx) for `check_out` failure display
- [GroupCheckinSheet.tsx](/Users/binhan/HotelManager/mhm/src/components/GroupCheckinSheet.tsx) for `group_checkin` failure display
- [GroupManagement.tsx](/Users/binhan/HotelManager/mhm/src/pages/GroupManagement.tsx) for `group_checkout` failure display
- [NightAudit.tsx](/Users/binhan/HotelManager/mhm/src/pages/NightAudit.tsx) for `run_night_audit` dispatch and failure display

### Excluded For Now

This release does not yet cover:

- reservation create/modify/cancel flows
- extend-stay
- read-only fetch commands
- settings, auth, room management, diagnostics, or export commands outside the selected scope

## Frontend Design

### Shared Correlation Helper

Frontend should add one small helper module, for example:

- [correlationId.ts](/Users/binhan/HotelManager/mhm/src/lib/correlationId.ts)

Responsibility:

- generate a new correlation ID
- hide format details from screens and stores

The helper should use browser/runtime crypto rather than ad hoc `Math.random()` string building.

### Shared Command Wrapper

[invokeCommand.ts](/Users/binhan/HotelManager/mhm/src/lib/invokeCommand.ts) should be extended rather than bypassed.

Required behavior:

- keep the existing first two parameters unchanged: `invokeCommand(command, args?)`
- add an optional third parameter for tracked calls only:
  `invokeCommand(command, args?, options?: { correlationId?: string })`
- when present, merge `correlationId` into the outgoing command args for the selected commands
- if the command fails, normalize the backend error exactly as PR `#50` already does
- then attach the correlation ID to the thrown local error object before rethrowing

This keeps the serialized backend error contract stable while still letting the frontend carry extra local context.

The wrapper must not repurpose the second parameter or force unrelated callers onto a new shape. This release only opts selected tracked commands into the third-parameter options object.

### Local Error Shape

[appError.ts](/Users/binhan/HotelManager/mhm/src/lib/appError.ts) should keep the shared FE/BE contract unchanged for normalized backend payloads.

However, the local thrown exception type may be extended with one frontend-only optional field:

- `correlation_id`

Rules:

- do not require backend payloads to contain `correlation_id`
- do not add `correlation_id` to `normalizeAppError()` validation for backend payloads
- allow the local wrapper-created exception object to carry `correlation_id`

This distinction keeps the transport contract stable and makes the correlation ID an application-level wrapper feature.

### Error Formatting

Frontend should add one focused helper for the selected flows, for example:

- `formatTrackedAppError(error: unknown): string`

Expected behavior:

- preserve the existing output of `formatAppError(error)`
- append `Mã theo dõi: COR-XXXXXXX` when the local error object carries `correlation_id`
- if both `support_id` and `correlation_id` exist, show both without dropping either

The selected flows should use this tracked formatter instead of hand-building error strings.

### UI Rules

Success case:

- do not show the correlation ID on successful check-in, checkout, group flow completion, or audit completion

Failure case:

- show the existing error message
- add the correlation ID on a new line
- keep the formatting compact enough for toast usage

This keeps the feature useful without adding noise to normal operator work.

## Backend Design

### Command Signatures

The selected commands should accept `correlation_id: String` as an additional argument:

- [rooms.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/rooms.rs): `check_in`, `check_out`
- [groups.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/groups.rs): `group_checkin`, `group_checkout`
- [audit.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/audit.rs): `run_night_audit`

No service-layer or repository-layer signature changes are required unless they make logging materially cleaner. The ID may remain command-boundary context in this release.

### Logging Rules

Each selected command should emit log lines with the correlation ID at:

- start
- success
- failure

The minimum logging payload should include:

- command name
- correlation ID
- relevant entity hints already available at the command boundary, such as `room_id`, `booking_id`, `group_id`, `audit_date`, or booking count

Failure handling rules:

- for `system` errors, include `correlation_id` inside the structured context passed to `log_system_error(...)`
- for `user` errors, emit a warn-level or info-level log line that still includes `correlation_id`

This ensures log search works for both expected and unexpected failures.

### Audit Migration

[audit.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/audit.rs) should be migrated to the shared command-error pattern introduced in PR `#50`.

Required changes:

- `run_night_audit` should stop returning raw `Result<_, String>`
- admin checks should use the existing shared command-error path
- booking and query failures should be mapped into `CommandError` using the same style already used in `rooms.rs` and `groups.rs`
- backend logging for audit should include correlation ID on start, success, and failure

Read-only `get_audit_logs` may remain outside this migration unless implementation work proves a direct dependency.

### Audit Error Codes

Because `normalizeAppError()` only accepts codes from the shared registry, this release must add explicit audit user-error codes instead of relying on free-form strings.

Required new registry entries:

- `AUDIT_INVALID_DATE`
- `AUDIT_ALREADY_RUN`

Expected user-facing meanings:

- `AUDIT_INVALID_DATE`:
  the provided audit date cannot be parsed or is not acceptable input
- `AUDIT_ALREADY_RUN`:
  the requested date has already been audited

Mapping rules for `run_night_audit`:

- invalid date parsing or equivalent audit-date validation failure:
  `AUDIT_INVALID_DATE`
- duplicate audit attempt for the same date:
  `AUDIT_ALREADY_RUN`
- database, transaction, or query failures:
  `SYSTEM_INTERNAL_ERROR`

This keeps audit aligned with the shared PR `#50` contract and avoids forcing audit-specific business failures into misleading booking error codes.

## Data And Persistence Rules

This release does not store correlation IDs in business data.

Explicit rules:

- do not add columns to `bookings`
- do not add columns to `booking_groups`
- do not add columns to `audit_logs`
- do not write correlation IDs into `pricing_snapshot`
- do not treat correlation IDs as historical reporting fields

The only persistence side effect allowed in this release is indirect:

- correlation ID may appear inside support error logs or command logs

## Testing Strategy

### Frontend Tests

Frontend should add or update tests that cover:

- correlation ID generation format
- `invokeCommand(...)` attaching correlation ID to the thrown local error object
- tracked error formatting showing correlation ID without breaking existing `support_id` formatting
- `NightAudit.tsx` switching to the shared tracked-command path

If store-level tests are easier than component-level tests for the stay and group flows, that is acceptable. The important outcome is that selected actions pass a correlation ID and failed actions surface it.

### Rust Tests

Rust tests should cover:

- selected command failure paths include `correlation_id` in the structured failure context used for system-error logging
- migrated audit error mapping now returns the shared `CommandError` contract instead of raw free-form strings
- user-error mapping remains stable for stay and group flows after the extra parameter is added

Where direct log capture is awkward, helper-level tests that assert the generated error context includes `correlation_id` are acceptable.

### Verification

Run the smallest existing verification path that still exercises the changed surface area, including:

- frontend unit tests touched by the wrapper and formatter changes
- Rust tests for the selected commands and helpers
- quick verification covering check-in, checkout, group flow, and night audit if those slices already exist in the repo's current suite

## Rollout Notes

This design intentionally adds correlation IDs only where issue `#30` asked first and where PR `#50` already laid shared groundwork.

If this release works well, the next wave can reuse the same pattern for reservation commands and other operator actions. That future expansion should be a separate design or implementation-plan decision, not an implicit requirement in this release.
