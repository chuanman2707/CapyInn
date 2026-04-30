# Reservation Lifecycle Through WriteCommandExecutor

## Status

Approved design for issue #69.

This spec covers routing single-reservation create, modify, cancel, and confirm writes through the durable command boundary. It is part of the #64 M2.5 durable command boundary roadmap.

## Context

The current codebase already has the command foundation:

- #65 built `WriteCommandExecutor`.
- #66 promoted `command_idempotency` into a command ledger.
- #67 added origin idempotency keys for ledger and folio side effects.
- #71, #72, and #73 added aggregate locks, manifest lock metadata, and state-transition guards.

Reservation create is already partially idempotent through `create_reservation_idempotent`. Reservation modify, cancel, and confirm still have direct production paths from Tauri into reservation lifecycle services. The MCP gateway currently exposes reservation create, modify, and cancel write tools; those gateway writes also bypass the idempotent reservation service boundary.

Issue #69 finishes the reservation slice by making all single-reservation writes use the same service-level idempotent command boundary.

## Goals

- Route reservation create, modify, cancel, and confirm through `WriteCommandExecutor`.
- Make UI/Tauri and MCP gateway writes share the same idempotent service functions.
- Keep calendar conflict behavior strict and structured.
- Prevent repeated reservation commands from duplicating booking, calendar, folio, or ledger effects.
- Return `CONFLICT_IDEMPOTENCY_HASH_MISMATCH` for same key with different payload.
- Keep UI flows working without asking humans to type or understand idempotency keys.
- Keep LLM-facing MCP schemas free of idempotency internals.

## Non-Goals

- Full money schema/model conversion is out of scope. It is tracked by #102.
- Group reservation is out of scope for #69.
- Stay, group, folio, and payment command rollout is out of scope. It remains #70.
- Command recovery queue and operator inspection actions remain #68.
- Supervised high-risk MCP rollout remains #74.
- Durable outbox work remains #76 through #82.
- The modify flow will not gain room, guest, or deposit editing in this issue.

## Chosen Approach

Use service-level idempotent reservation lifecycle functions shared by UI/Tauri and MCP gateway:

- `create_reservation_idempotent`
- `modify_reservation_idempotent`
- `cancel_reservation_idempotent`
- `confirm_reservation_idempotent`

Tauri command handlers and MCP gateway write tools must call these idempotent functions for production writes. Existing non-idempotent service functions may remain for focused internal tests or as private transaction helpers, but they must not remain the production write path for #69 flows.

This is narrower than a full command-envelope rebuild and broader than a thin Tauri-only wrapper. It satisfies the PMS rule that UI, bots, agents, and integrations all cross the same explicit command boundary.

## Command Context

Each reservation write receives a `WriteCommandContext` containing:

- request id or correlation id
- idempotency key
- command name
- actor metadata
- timestamp and request context metadata already supported by the command ledger

UI commands use human actor context. MCP gateway reservation writes use hidden gateway-created command context with `ActorType::AiAgent`. The LLM must not see or supply idempotency keys as ordinary business tool arguments.

## UI Behavior

Add a shared frontend helper for write commands, conceptually `invokeWriteCommand`.

The helper:

- generates an idempotency key for each submit attempt
- attaches the key to the Tauri command args
- preserves existing correlation-id and monitoring behavior
- normalizes structured command errors through the existing app-error path

Reservation UI callers use this helper for:

- `create_reservation`
- `modify_reservation`
- `cancel_reservation`
- `confirm_reservation`

The UI treats same-key replay as a normal success. It may still refresh rooms/bookings after success so the screen reflects current read-model state. Button disabled/loading states remain useful for ergonomics but must not be required for correctness.

## MCP Gateway Behavior

Reservation write tool schemas must not expose `idempotency_key`.

The gateway creates hidden idempotency keys per tool call. The key is derived from gateway instance/session identity, command name, MCP request id, and the canonical tool-argument hash. If the gateway retries the same tool-call request with the same canonical arguments, it reuses the same hidden key. If the LLM asks for a new tool call, or a client reuses a JSON-RPC request id with different arguments, the gateway generates a different hidden key and normal business validation runs.

Gateway reservation writes must return structured JSON success/error envelopes instead of raw string-only `Error: ...` values. This keeps policy, idempotency, conflict, and validation results machine-readable.

## Canonical Payloads

Each command uses a versioned canonical payload schema:

- `reservation.create.v1`
- `reservation.modify.v1`
- `reservation.cancel.v1`
- `reservation.confirm.v1`

Canonical payloads contain only business input that defines the command. They do not include idempotency key, request id, correlation id, actor, timestamp, or UI monitoring context.

Create payload includes:

- room id
- guest name
- optional guest document number
- optional guest phone
- check-in date
- check-out date
- nights
- source
- notes
- deposit value canonicalized as integer VND units

Modify payload includes:

- booking id
- new check-in date
- new check-out date
- new nights

Cancel payload includes:

- booking id

Confirm payload includes:

- booking id

`confirm_reservation` intentionally does not include the current date in the payload. A replay of the same completed command returns the stored command result snapshot, not a recalculation against a later date.

## Money Handling In #69

The app currently stores and models VND amounts in several places as floating-point values. Full conversion to integer VND storage and models is issue #102.

#69 must not expand into full money migration. It must also avoid introducing new float-based idempotency behavior. For reservation command hashing and safe ledger intent, any reservation deposit is canonicalized to integer VND units before hashing. For example, a `deposit_amount` representing `500000` VND is hashed as integer `500000`, not as `500000.0`.

Command ledger safe fields must not contain raw booking UUIDs, guest identifiers, phone numbers, or other values rejected by the existing safe-field sanitizer. Exact booking and room ids belong in the canonical hash payload, primary aggregate key, and lock key metadata, not in sanitized ledger summary fields. Ledger summaries can use safe facts such as command schema, date fields, booleans, and small integer VND amounts.

## Idempotency Result Contract

Completed exact retry:

- Same command name, same idempotency key, same canonical payload hash.
- The executor returns the `response_json` snapshot stored in the command row.
- It does not query the booking or room tables again during replay.

Hash mismatch:

- Same command name and idempotency key but different canonical payload hash.
- Return `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`.

Duplicate in-flight:

- Same command name, same idempotency key, same payload hash, still in progress.
- Return `CONFLICT_DUPLICATE_IN_FLIGHT`.

Retryable system failure:

- Retryable system errors such as retryable DB lock failures are recorded as retryable and may be reclaimed with the same key.

Terminal user failure:

- Calendar conflicts, invalid state transitions, not-found user errors, and validation failures are terminal command results.
- A same-key replay returns the stored terminal error rather than re-running the mutation.

The response snapshot is the command result, not the current read-model state. UI and gateway callers can query read APIs after success if they need current state.

## Response Shapes

Create returns a `Booking` snapshot.

Modify returns a `Booking` snapshot.

Confirm returns a `Booking` snapshot.

Cancel returns a structured success snapshot:

```json
{
  "ok": true,
  "booking_id": "..."
}
```

This avoids ambiguous unit responses and gives cancel replays a concrete stored response.

## Locking

Create reservation derives:

- `room:{room_id}`

Modify, cancel, and confirm derive:

- `booking:{booking_id}`
- `room:{current_room_id}`

The current room id may require a pre-transaction lookup before acquiring the runtime lock. Because that lookup can go stale, the service must re-read and revalidate the booking room inside the transaction before mutating.

Lock keys are sorted and deduplicated by existing lock/executor conventions.

For modify, cancel, and confirm, the idempotency row must be claimed before the room lookup. The booking id is available from the canonical payload and can be recorded immediately as `booking:{booking_id}`. After the claimed command resolves the current room id, the executor path must acquire the combined booking and room runtime locks and refresh the command row lock metadata to include `room:{current_room_id}` before the business transaction runs. If the booking lookup fails, the failure is finalized as a terminal command error so same-key replay returns the stored not-found result.

Create already knows `room_id` from the canonical payload. It must acquire the real runtime room lock, not only persist lock metadata.

## Business Transaction Rules

Each reservation command runs as one business mutation:

1. validate command payload and build the canonical request hash
2. claim or replay the idempotency row before fallible business lookups
3. resolve and acquire stable runtime locks
4. refresh command lock metadata when post-claim lookup discovers additional lock keys
5. open `BEGIN IMMEDIATE`
6. revalidate booking, room, and state inside the transaction
7. mutate booking, calendar, and any ledger/folio side effects
8. write response snapshot and finalize the command row
9. commit, or roll back all business and command-finalize writes together

Where the executor path can make command claim, business mutation, and finalize atomic in one transaction, use that atomic path.

#69 is not production-complete under the durable outbox rule until the later outbox roadmap work (#76 through #82) is implemented. This issue must not add direct external side effects inside business mutations; existing UI refresh events remain adapter-level notifications after command success.

## Calendar Conflict Semantics

Create:

- checks `room_calendar` for the requested range before insert
- inserts booked calendar rows only after the range is clear
- relies on the `room_calendar` uniqueness constraint as the final safety net

Modify:

- only works for a booked reservation
- deletes only this booking's booked calendar rows inside the transaction
- checks the new range against remaining room calendar rows
- inserts the new booked range only after the range is clear

Conflict errors map to `CONFLICT_ROOM_UNAVAILABLE` with a structured `CommandError`.

## State Transition Semantics

Cancel:

- allowed only from `booked` to `cancelled`
- deletes this reservation's booked calendar rows
- records cancellation fee behavior once, if applicable
- exact replay does not write another cancellation fee

Confirm:

- allowed only from `booked` to `active`
- rejects no-show calendar state
- converts booked calendar rows into occupied rows for the effective stay
- records room charge once
- exact replay does not reprice or write another charge

Modify:

- allowed only while booking status is `booked`
- does not change room, guest, or deposit
- exact replay returns the original modify result snapshot

Stale state transitions return `CONFLICT_INVALID_STATE_TRANSITION`.

## Error Classification

Reservation command errors use the existing structured app error registry:

- `CONFLICT_ROOM_UNAVAILABLE` for calendar conflicts
- `CONFLICT_INVALID_STATE_TRANSITION` for stale booking/room state
- `CONFLICT_IDEMPOTENCY_HASH_MISMATCH` for key reuse with different payload
- `CONFLICT_DUPLICATE_IN_FLIGHT` for duplicate in-flight attempts
- `IDEMPOTENCY_KEY_REQUIRED` for missing UI/Tauri idempotency keys
- `DB_LOCKED_RETRYABLE` for retryable lock contention
- existing not-found and validation codes where already mapped

System errors must keep correlation/request id behavior and failure logging.

## Implementation Surface

Primary backend files:

- `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs`
- `mhm/src-tauri/src/commands/reservations.rs`
- `mhm/src-tauri/src/gateway/tools.rs`
- `mhm/src-tauri/src/gateway/models.rs`
- `mhm/src-tauri/src/gateway/policy.rs`
- `mhm/src-tauri/src/command_idempotency.rs` only if small helper additions are needed

Primary frontend files:

- `mhm/src/lib/invokeCommand.ts`
- `mhm/src/components/ReservationSheet.tsx`
- `mhm/src/pages/Reservations.tsx`

Primary tests:

- `mhm/src-tauri/src/services/booking/tests.rs`
- `mhm/src-tauri/src/commands/reservations.rs` unit tests
- `mhm/src-tauri/src/gateway/tools.rs` tests
- `mhm/src/components/ReservationSheet.test.tsx`
- reservation page/UI tests for confirm and cancel wiring

## Test Plan

Backend idempotency tests:

- create exact retry does not duplicate booking, calendar rows, or deposit transaction
- modify exact retry does not rewrite extra calendar effects and returns stored snapshot
- cancel exact retry does not write an extra cancellation fee
- confirm exact retry does not write an extra room charge and does not recalculate by retry date
- same key with changed payload returns `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`
- duplicate in-flight returns `CONFLICT_DUPLICATE_IN_FLIGHT`
- retryable DB lock failure can be reclaimed with the same key after the failed command row becomes reclaimable

Conflict and state tests:

- create calendar conflict returns `CONFLICT_ROOM_UNAVAILABLE`
- modify calendar conflict returns `CONFLICT_ROOM_UNAVAILABLE`
- conflict replay returns the stored terminal error
- cancel non-booked reservation returns `CONFLICT_INVALID_STATE_TRANSITION`
- modify non-booked reservation returns `CONFLICT_INVALID_STATE_TRANSITION`
- confirm non-booked reservation returns `CONFLICT_INVALID_STATE_TRANSITION`
- no-show confirmation remains rejected

UI tests:

- create uses the shared write-command helper
- modify uses the shared write-command helper
- cancel uses the shared write-command helper
- confirm uses the shared write-command helper
- UI does not expose or require human-entered idempotency keys

Gateway tests:

- reservation write schemas do not expose idempotency key
- create, modify, and cancel route through idempotent service functions
- confirm is not added as a new MCP tool in #69 unless a confirm tool already exists by implementation time
- gateway exact retry uses stored snapshot behavior
- gateway errors are structured JSON envelopes

## Acceptance Criteria Mapping

Reservation writes use canonical request hashing and idempotency keys:

- All four single-reservation writes build versioned canonical payloads and use `WriteCommandExecutor`.

Calendar conflict behavior remains strict and structured:

- Create and modify preserve strict calendar checks and map conflicts to `CONFLICT_ROOM_UNAVAILABLE`.

Repeating the same reservation command does not duplicate effects:

- Exact retry returns command snapshot and does not re-run booking, calendar, folio, or ledger writes.

Same key with different payload returns hash mismatch:

- Executor returns `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`.

UI flows continue to work without asking humans to type idempotency keys:

- Shared UI helper creates and passes idempotency keys automatically.

## Rollout Order

1. Add failing backend tests for modify/cancel/confirm idempotency.
2. Introduce idempotent service functions and structured cancel response.
3. Route Tauri reservation commands through the idempotent service functions.
4. Add shared frontend write-command helper and update reservation UI callers.
5. Route MCP reservation writes through the idempotent service functions with hidden gateway keys.
6. Add conflict, hash mismatch, duplicate in-flight, and replay tests.
7. Run GitNexus change detection before any commit.

## Open Decisions Closed During Brainstorming

- Same-key completed replay behaves as normal success in UI.
- Replay returns stored command response snapshot, not current DB state.
- UI and MCP gateway are both in #69 scope.
- UI uses a shared write-command helper.
- `confirm_reservation` replay does not recalculate by retry date.
- Calendar conflict is a terminal command result.
- Group reservation is excluded from #69.
- Money schema/model migration is #102, not #69.
- Gateway creates hidden idempotency keys; the LLM does not see them.
- Modify remains limited to date and night changes.
- Retryable system errors can reclaim; user conflicts and invalid states are terminal.
