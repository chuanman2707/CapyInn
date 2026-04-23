# CapyInn Command Failure Monitoring Design

## Summary

CapyInn will add command failure monitoring for a narrow set of operator actions under issue `#30`.

This release will monitor failures for exactly these four commands:

- `check_in`
- `check_out`
- `create_reservation`
- `run_night_audit`

The design uses one canonical local diagnostics stream plus one optional remote reporting path:

- backend attempts to record one sanitized local command-failure record for every failure in scope
- frontend may additionally send one scrubbed Sentry event for the same failed action
- remote reporting is gated by the existing `Send crash reports` consent toggle

This release intentionally monitors both `user` and `system` failures because the value of the new signal comes from the combination of:

- stable `code`
- action-scoped `correlation_id`
- optional `support_id` for system failures

The design keeps the scope narrow on purpose. It does not try to create a general observability platform, a background retry queue, or a whole-app rollout across every Tauri command.

## Goals

- Attempt to capture one local diagnostics record for every failed action in the selected scope.
- Reuse the existing shared error contract from PR `#50` instead of creating another command-failure format.
- Reuse the existing correlation-ID design so each monitored failure can be traced across frontend and backend with one action reference.
- Reuse the current `Send crash reports` consent instead of adding a second diagnostics toggle.
- Add optional remote Sentry reporting that is best-effort and never blocks operator UX.
- Bring `create_reservation` onto the shared `CommandError` and `correlation_id` path so the four selected commands use one monitoring contract.

## Non-Goals

- No rollout to `group_checkin`, `group_checkout`, or other booking commands in this release.
- No DB schema changes and no persistence into `bookings`, `booking_groups`, or `audit_logs`.
- No command-monitoring dashboard, search UI, retry queue, or offline upload worker.
- No backfill or replay of historical command failures.
- No raw PII export to Sentry.
- No replacement of the existing system root-cause log written through [support_log.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/support_log.rs).

## Current Baseline

Recent work already created the right foundation for this release:

- PR `#50` standardized command failures around `code`, `message`, `kind`, and `support_id`
- the 2026-04-22 correlation-ID design added action-scoped `correlation_id` for selected booking and audit flows
- crash reporting already has:
  - one local diagnostics area under `~/CapyInn/diagnostics`
  - one optional remote Sentry path
  - one user-controlled consent toggle in [DiagnosticsSection.tsx](/Users/binhan/HotelManager/mhm/src/pages/settings/DiagnosticsSection.tsx)

The current implementation still has an observability gap:

- `check_in`, `check_out`, and `run_night_audit` already have meaningful structured failure data
- those failures are visible in logs and toasts, but they are not yet recorded as one stable command-failure event stream
- `create_reservation` still uses raw `Result<_, String>` handling and direct `invoke(...)`, so it is outside the standardized command-error and correlation-ID path

That means issue `#30` is still missing one practical thing: a reliable, scoped place to inspect failed operator actions by code and correlation ID instead of stitching together ad hoc logs.

## Chosen Approach

CapyInn will treat backend local logging as the canonical source for command-failure monitoring, then layer optional remote reporting on top from the frontend.

The selected approach has four parts:

1. Backend writes one sanitized local record for every failed command in scope.
2. Existing system-error support logging remains in place for technical root causes.
3. Frontend best-effort sends a remote Sentry event for the same failure when consent and DSN are available.
4. `create_reservation` is migrated onto the shared command-error and correlation-ID contract before monitoring is enabled for it.

This is preferred over a frontend-only solution because:

- local diagnostics remain available even without remote configuration
- backend is the natural place to record the final normalized error code and context
- the design reuses the current diagnostics runtime root and does not depend on browser state to preserve the local source of truth

## Rejected Alternatives

### Frontend-Only Monitoring

Rejected because it would make the monitoring stream depend entirely on the frontend wrapper and would leave backend diagnostics fragmented.

It also would not solve the biggest gap in scope, which is that `create_reservation` still sits on a legacy string-error path.

### Backend-Only Monitoring

Rejected because issue `#30` is about observability, not just local logging.

If remote reporting is never added to the design, the product would miss an already available channel that the app has for sanitized diagnostics through Sentry and the current consent model.

### Separate Consent Toggle For Command Failure Monitoring

Rejected for this release because it adds UI, wording, and preference-storage overhead without enough product value yet.

The current crash-reporting preference already expresses the right user intent: allow sanitized diagnostics to leave the device.

This release should reuse that decision and keep the scope light.

### Monitoring Only System Failures

Rejected because the issue scope became more valuable only after the app gained stable `error code` and `correlation_id`.

Those two fields make `user` failures useful to inspect:

- repeated validation patterns become visible
- support can trace one failed action without needing a `support_id`
- product can distinguish real operator friction from unexpected infrastructure faults

System failures still matter most operationally, but the first release should capture both kinds.

## Scope

### Included Commands

This release applies only to:

- `check_in`
- `check_out`
- `create_reservation`
- `run_night_audit`

### Included Command Boundaries

Monitoring is attached only at the Tauri command boundary for the four selected actions.

In practice this means:

- [rooms.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/rooms.rs): `check_in`, `check_out`
- [reservations.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/reservations.rs): `create_reservation`
- [audit.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/audit.rs): `run_night_audit`
- [useHotelStore.ts](/Users/binhan/HotelManager/mhm/src/stores/useHotelStore.ts): existing `check_in` and `check_out` wrapper call sites
- [NightAudit.tsx](/Users/binhan/HotelManager/mhm/src/pages/NightAudit.tsx): existing `run_night_audit` wrapper call site
- [invokeCommand.ts](/Users/binhan/HotelManager/mhm/src/lib/invokeCommand.ts): remote bridge for failures in scope
- [ReservationSheet.tsx](/Users/binhan/HotelManager/mhm/src/components/ReservationSheet.tsx): migrate reservation create call to the shared tracked-command path

`check_in`, `check_out`, and `run_night_audit` already use `invokeCommand(...)` in the current codebase. `create_reservation` is the only selected command that still needs migration onto that wrapper.

### Explicitly Excluded For Now

This release does not yet cover:

- `group_checkin`
- `group_checkout`
- `confirm_reservation`
- `cancel_reservation`
- `modify_reservation`
- `extend_stay`
- read-only fetch commands
- settings, auth, export, or diagnostics commands outside the selected scope

## Failure Record Model

### Local Record Purpose

The local command-failure record is the product-facing observability record for one failed action attempt.

It answers:

- which command failed
- what normalized error code it failed with
- whether the failure was `user` or `system`
- which `correlation_id` identifies the action attempt
- which `support_id` links to the deeper system-error log when relevant

It is intentionally not the place for raw stack traces or low-level root cause strings.

### Local Record Shape

Backend should write one JSONL line per failed action in a new diagnostics stream:

- `~/CapyInn/diagnostics/command-failures.jsonl`

Proposed record shape:

- `schema_version`
- `timestamp`
- `command`
- `code`
- `kind`
- `correlation_id`
- `support_id`
- `context`

Example user-failure record:

```json
{
  "schema_version": 1,
  "timestamp": "2026-04-22T09:10:11Z",
  "command": "create_reservation",
  "code": "BOOKING_INVALID_NIGHTS",
  "kind": "user",
  "correlation_id": "COR-1A2B3C4D",
  "support_id": null,
  "context": {
    "nights": 0,
    "deposit_present": false,
    "source": "phone",
    "notes_present": false
  }
}
```

Example system-failure record:

```json
{
  "schema_version": 1,
  "timestamp": "2026-04-22T09:10:11Z",
  "command": "check_out",
  "code": "SYSTEM_INTERNAL_ERROR",
  "kind": "system",
  "correlation_id": "COR-8F3A1C7D",
  "support_id": "SUP-ABCD1234",
  "context": {
    "settlement_mode": "manual"
  }
}
```

### Local Record Schema Contract

The local record contract should be treated as stable for this release.

Required field rules:

- `schema_version`:
  integer
  fixed value `1`
- `timestamp`:
  string
  RFC 3339 UTC timestamp generated backend-side
- `command`:
  string literal
  one of `check_in`, `check_out`, `create_reservation`, `run_night_audit`
- `code`:
  string
  normalized command error code returned to the frontend
- `kind`:
  string literal
  one of `user` or `system`
- `correlation_id`:
  string
  the effective backend correlation ID after normalization
- `support_id`:
  string or `null`
  `null` for `user` failures
- `context`:
  JSON object
  never `null`, never array, never primitive
  one of the command-specific object shapes defined below

`context` is a command-tagged union keyed by the top-level `command` field. This release should not treat it as an unbounded free-form map.

### Relationship To `support-errors.jsonl`

The new command-failure log does not replace the current system root-cause log.

Rules:

- `command-failures.jsonl` exists for every failure in scope, whether `user` or `system`
- `support-errors.jsonl` continues to exist only for system failures
- both records share the same `support_id` when the failure is `system`
- only `support-errors.jsonl` keeps the deeper technical root cause

This split avoids turning the command-failure log into a second copy of low-level diagnostics.

## Sanitization Rules

### Local Context Rules

Local records should still be sanitized. The goal is useful diagnostics, not full request dumps.

Allowed command context:

- `check_in`:
  - `room_id: string`
  - `guest_count: integer`
  - `nights: integer`
  - `source: string | null`
  - `notes_present: boolean`
- `check_out`:
  - `booking_id: string`
  - `settlement_mode: string`
  - `final_total: number`
- `create_reservation`:
  - `room_id: string`
  - `check_in_date: string`
  - `check_out_date: string`
  - `nights: integer`
  - `deposit_present: boolean`
  - `source: string | null`
  - `notes_present: boolean`
- `run_night_audit`:
  - `audit_date: string`
  - `notes_present: boolean`

Do not log:

- guest name
- phone number
- document number
- raw notes
- address
- raw OCR output

### Remote Sentry Event Rules

Remote reporting must be thinner than the local log.

Each remote event should use:

- level:
  - `warning` for `user`
  - `error` for `system`
- tags:
  - `event_type=command_failure`
  - `command`
  - `code`
  - `kind`
- extra:
  - `correlation_id`
  - `support_id`
  - one scrubbed remote-safe context object

Remote-safe context should be reduced to low-sensitivity fields:

- `check_in`:
  - `guest_count: integer`
  - `nights: integer`
  - `source: string | null`
  - `notes_present: boolean`
- `check_out`:
  - `settlement_mode: string`
- `create_reservation`:
  - `nights: integer`
  - `deposit_present: boolean`
  - `source: string | null`
  - `notes_present: boolean`
- `run_night_audit`:
  - `notes_present: boolean`

Do not send to Sentry:

- `room_id`
- `booking_id`
- exact dates
- exact money values
- guest or document fields
- raw root-cause strings

### Grouping Rules

Sentry fingerprint should group by:

- `command`
- `code`

This keeps grouping stable enough for issue triage while leaving `correlation_id` available for one-off trace lookup.

## Backend Design

### New Logging Module

Backend should add a focused module:

- [command_failure_log.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/command_failure_log.rs)

Responsibilities:

- define `CommandFailureRecord`
- resolve the diagnostics path for `command-failures.jsonl`
- append JSONL safely under one mutex

The module should not decide how commands map domain errors. It only records the final normalized failure information.

### Shared Helper

Backend should add a small shared helper that records one failure after the command has already classified it into `CommandError`.

Required inputs:

- `command`
- `code`
- `kind`
- `correlation_id`
- `support_id`
- sanitized `context`

Behavior:

- write one local command-failure record
- never panic the command path if file append fails
- log append failures as internal logging errors only

### Command Flow

For each selected command:

1. Normalize incoming `correlation_id`.
2. Build a sanitized command context.
3. Run the service call.
4. If the service fails, map the failure into `CommandError`.
5. Record the command failure locally from the normalized error shape.
6. Return the same `CommandError` to the caller.

For `system` failures:

1. `log_system_error(...)` keeps writing `support-errors.jsonl`
2. command failure recording writes `command-failures.jsonl`
3. both records share the same `support_id`

For `user` failures:

1. no support log is written
2. command failure recording still writes `command-failures.jsonl`

### `create_reservation` Migration

This release requires [reservations.rs](/Users/binhan/HotelManager/mhm/src-tauri/src/commands/reservations.rs) to move away from `Result<Booking, String>` for `create_reservation`.

Required changes:

- add `correlation_id: Option<String>` to the command signature
- return `CommandResult<Booking>` instead of `Result<Booking, String>`
- normalize correlation ID at the command boundary
- map reservation failures into the shared error-code contract
- record command-failure events through the same helper as the other commands

Frontend must also switch reservation creation from direct `invoke(...)` to `invokeCommand(...)` with a generated correlation ID.

This is the only functional migration required to make the selected scope consistent.

`create_reservation` must adopt the same correlation-ID rules already approved for the tracked stay and audit flows:

- frontend-generated IDs in the `COR-XXXXXXXX` format
- wrapper-local attachment of `correlation_id` on failure
- backend validation and fallback generation rules
- backend use of the effective normalized ID for local diagnostics

This release should import those rules as-is rather than define a lighter reservation-specific variant.

### Reservation Error-Code Rule

This release should reuse existing booking-domain error codes for reservation create failures whenever the operator-facing meaning already matches.

Examples:

- invalid nights:
  `BOOKING_INVALID_NIGHTS`
- missing room:
  `ROOM_NOT_FOUND`
- invalid booking state or conflict:
  `BOOKING_INVALID_STATE`

Rules:

- do not add new reservation-specific error codes in this release
- if a reservation failure does not map cleanly to an existing operator-facing code, fall back to `SYSTEM_INTERNAL_ERROR`
- do not widen the monitoring slice just to perfect reservation taxonomy in Phase 1

## Frontend Design

### Shared Remote Bridge

[invokeCommand.ts](/Users/binhan/HotelManager/mhm/src/lib/invokeCommand.ts) should become the single remote-reporting bridge for monitored command failures.

Required behavior in the `catch` path:

1. normalize the backend error into the shared app error shape
2. attach local `correlation_id` as it already does
3. best-effort call a new remote helper such as `captureCommandFailure(...)`
4. rethrow the original normalized local error

The helper must not change the user-visible error flow. If remote reporting itself fails, that failure is swallowed and the original command failure continues unchanged.

The frontend wrapper must not inspect raw command args to invent its own scrubbed payload. The call site that knows the domain semantics should provide the remote-safe context explicitly.

Implementation direction:

- extend the third `invokeCommand(...)` options object to include both:
  - `correlationId`
  - `monitoringContext`
- `monitoringContext` should already be scrubbed and command-specific at the call site
- `invokeCommand(...)` should pass only that scrubbed object to `captureCommandFailure(...)`

That keeps the generic wrapper generic and makes data ownership explicit.

### Allowlist And Consent Rules

The remote helper should send only when all of the following are true:

- the command is in the four-command allowlist
- the error normalized successfully into the shared app error shape
- a `correlation_id` exists
- `Send crash reports` is enabled
- Sentry DSN is configured

This keeps remote scope explicit and prevents monitoring from silently spreading to unrelated commands.

In-scope call sites therefore need one small update:

- `useHotelStore.ts` passes `monitoringContext` for `check_in` and `check_out`
- `NightAudit.tsx` passes `monitoringContext` for `run_night_audit`
- `ReservationSheet.tsx` passes `monitoringContext` when migrating `create_reservation`

### Diagnostics Copy

[DiagnosticsSection.tsx](/Users/binhan/HotelManager/mhm/src/pages/settings/DiagnosticsSection.tsx) should update its explanatory copy.

The current wording is crash-only. After this release, the toggle should clearly describe that consent covers:

- severe crash reports
- sanitized command failure diagnostics

The copy should still emphasize:

- no usage analytics
- no session replay
- no guest-record payloads

Because this broadens the meaning of the current diagnostics toggle beyond crash-only reporting, the implementation PR should call out that copy change explicitly in review notes for product/privacy confirmation.

### Reservation UI Path

[ReservationSheet.tsx](/Users/binhan/HotelManager/mhm/src/components/ReservationSheet.tsx) should move only the create flow in scope to the shared wrapper:

- generate a correlation ID immediately before `create_reservation`
- build one scrubbed `monitoringContext` object for the reservation create attempt
- call `invokeCommand("create_reservation", ..., { correlationId, monitoringContext })`
- keep existing success UX
- switch failure display to `formatAppError(error)` instead of `String(error)`

Reservation modify, confirm, and cancel remain outside this release.

## Testing Strategy

### Backend Unit Tests

Add focused tests for the new command-failure logging module:

- path stays under `diagnostics/command-failures.jsonl`
- append writes valid JSONL
- concurrent appends do not interleave
- top-level record keys remain stable

### Backend Command Tests

Add or extend tests for the selected commands so that:

- user failures write `command-failures.jsonl`
- system failures write both `command-failures.jsonl` and `support-errors.jsonl`
- shared `support_id` is preserved across both logs for system failures
- `correlation_id` is present in every command-failure record
- `create_reservation` now returns the shared `CommandError` contract

### Frontend Tests

Frontend tests should cover:

- remote helper ignores commands outside the allowlist
- remote helper does not send when consent is disabled
- remote helper does not send when DSN is absent
- `invokeCommand(...)` preserves the original thrown error even if remote reporting fails
- reservation create flow now uses the tracked command path

### Verification

This is a docs-first design, but the eventual implementation verification should target the changed surfaces:

- reservation create tests
- night audit tests
- shared command wrapper tests
- Rust command tests for stay, reservation, and audit error paths

## Rollout Boundaries

This release stays intentionally narrow.

Rules:

- do not expand to group commands in the same PR just because they are technically similar
- do not add a retry queue for remote events
- do not upload historical local records
- do not add database tables or settings schema just for monitoring
- do not redesign the diagnostics page beyond copy updates needed for accurate consent wording
- keep PR and release notes explicit about the monitored-command set so the temporary gap versus deferred group flows is visible

Remote send remains best-effort at failure time only. Local logging is the durable fallback.

## Correlation-ID Consistency Rules

Normal operation for this release assumes all four in-scope frontend call sites generate a valid correlation ID through the shared frontend helper before calling `invokeCommand(...)`.

Expected steady state:

- frontend remote event uses the frontend-generated `correlation_id`
- backend local log uses the effective normalized `correlation_id`
- those values are identical for normal in-scope operation

Defensive fallback rule:

- if backend receives a missing or invalid correlation ID, it should replace it with a backend-generated fallback as already defined in the correlation-ID design
- in that defensive path, backend local diagnostics may carry a different correlation ID from any frontend-local state
- this release does not attempt to reconcile that mismatch because it represents caller bug or partial rollout, not the intended steady-state path

The implementation plan should treat backend fallback as a guardrail, not as a normal monitored flow.

## Roadmap Positioning

This design should be understood as a separate observability track under issue `#30`, not as part of the behavior-preserving refactor roadmap in [2026-04-17-refactor-roadmap.md](/Users/binhan/HotelManager/docs/superpowers/plans/2026-04-17-refactor-roadmap.md).

That distinction matters because this work intentionally adds runtime behavior:

- one new diagnostics stream
- one optional remote-reporting path
- one consent-copy change
- one command migration for `create_reservation`

So the right sequencing claim is narrower:

- it builds directly on the shared error-contract work from PR `#50`
- it builds directly on the correlation-ID design for tracked booking and audit flows
- it reuses the existing crash-reporting runtime and consent model

It does not claim to be part of the cleanup/refactor roadmap. It is a separate product-observability slice that depends on those earlier foundations.

### Scope Override Relative To The Previous Rollout

This design also intentionally diverges from the previous tracked-command sequence.

The prior correlation-ID design covered:

- `check_in`
- `check_out`
- `group_checkin`
- `group_checkout`
- `run_night_audit`

This monitoring design instead covers:

- `check_in`
- `check_out`
- `create_reservation`
- `run_night_audit`

That is a deliberate scope override based on product priority, not because `create_reservation` is already the cleanest technical fit today.

Reasoning:

- the issue request for this slice explicitly prioritized `create_reservation`
- reservation create failures become valuable only now that the app has shared error codes and can be brought onto correlation IDs
- keeping exact scope to the requested four commands is more important for this release than preserving the previous stay-group-audit rollout order

This tradeoff is acceptable only because the design keeps the migration bounded:

- `create_reservation` is the only new prerequisite migration
- group flows are explicitly deferred, not silently abandoned
- the design does not expand the scope beyond that one override

The main roadmap risk would be if this release drifted further into:

- global command middleware
- broad reservation lifecycle migration beyond `create_reservation`
- separate preferences and diagnostics UI
- richer remote analytics or dashboards

Those remain follow-up work.

## Implementation Constraints

- Use `command-failures.jsonl` as the file name for this release. Do not rename the stream inside the implementation PR.
- Use `command_failure_log.rs` as the backend module name for this release so the file, type, and test names stay easy to follow.
- If remote-send consent is not already readable from a shared frontend helper, implementation should add one small reusable read path instead of duplicating settings fetch logic inside multiple call sites.
