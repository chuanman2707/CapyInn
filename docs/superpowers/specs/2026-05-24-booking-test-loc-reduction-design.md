# Booking Test LOC Reduction Design

## Context

`mhm/src-tauri/src/services/booking/tests.rs` is currently the largest test file in the repository at about 8.4k lines. It mixes database schema setup, seed helpers, request factories, command/idempotency fixtures, outbox assertions, direct SQL assertions, and roughly 150 booking service tests.

The target for this session is total test LOC reduction, not just moving lines out of `tests.rs`. The refactor should make repeated setup and assertions shorter across the whole booking test area while preserving the behavior coverage that protects PMS write safety.

GitNexus was refreshed before this design because the index was stale. It identifies `test_pool`, `seed_room`, and request factory helpers as high-fan-in test symbols used by many booking tests. Implementation must run impact analysis before modifying existing symbols, as required by the project rules.

## Goals

- Reduce total booking test LOC by extracting real duplication into reusable fixtures, builders, and assertion helpers.
- Reduce `tests.rs` substantially below its current size while keeping individual test intent readable.
- Preserve every runtime service path and business assertion currently covered by the tests.
- Keep important PMS safety signals visible in tests: command name, idempotency key, actor/request context when relevant, lock/idempotency behavior, ledger/folio/outbox effects, room/calendar status, and settlement amounts.
- Rewrite the whole file by clusters, using focused helper layers and repeated verification.

## Non-Goals

- Do not change runtime booking, billing, reservation, group, outbox, command idempotency, or query behavior.
- Do not weaken tests by removing meaningful assertions.
- Do not hide business-critical values behind a broad scenario DSL when those values are what the test is proving.
- Do not introduce a new Rust dependency just for test builders.
- Do not touch unrelated dirty worktree changes such as `mhm/src/stores/useHotelStore.test.ts`.

## Architecture

Keep `tests.rs` as the main test-case file, but move reusable support code into a small support module under the booking service test area. The support module should be organized by purpose rather than by runtime domain ownership.

Planned layout:

- `mhm/src-tauri/src/services/booking/tests.rs`: test cases and edge-case-specific setup.
- `mhm/src-tauri/src/services/booking/tests/support/mod.rs`: shared test-only helper entry point.
- `mhm/src-tauri/src/services/booking/tests/support/db.rs`, `seed.rs`, `request.rs`, `command.rs`, and `assertions.rs`: focused helper modules, added as each cluster needs them.

The helpers should remain test-only and private to the booking test module unless another test module already imports an existing helper. Existing public helper visibility can be preserved where needed to avoid expanding the refactor surface.

## Components

### Database Support

`test_pool` remains the standard entry point so the first pass does not force every test to change at once. The manual in-memory schema setup may move into a support module, but SQL should stay behaviorally identical unless a compile or migration alignment issue requires a narrowly scoped fix.

Shared file database setup stays available for concurrency tests that need two pools pointing at the same SQLite file.

### Seed Helpers

Existing seed functions remain available, with small helpers added where they remove repeated setup:

- `seed_standard_room(&pool, room_id)`
- `seed_standard_rooms(&pool, &[room_ids])`
- `seed_room_with_price(&pool, room_id, daily_rate)`
- `seed_active_booking_case(...)`
- `seed_booked_reservation_case(...)`
- focused helpers for transactions, folio lines, expenses, group bookings, and calendar rows

Seed helpers may use direct SQL because they set test preconditions. They must not replace service calls in tests where the service mutation path is the behavior under test.

### Request Builders

Keep current minimal request factories and add lightweight builders only where they shorten repeated overrides:

- check-in request builder for room id, nights, paid amount, guest fields, and pricing type
- reservation request builder for dates, nights, deposit, guest fields, source, and notes
- group check-in request builder for room list, master room, check-in date, paid amount, and guests per room
- checkout request helper for common settlement modes

Builders should expose defaults matching the current minimal factories. Tests should still show the values that make the case unique.

### Command Fixtures

Add helpers around `WriteCommandContext::for_internal_test` to reduce repeated request/idempotency boilerplate while keeping the command boundary explicit.

Expected shapes:

- `cmd(command_name, idempotency_key)`
- `cmd_with_request(command_name, request_id, idempotency_key)`
- `cmd_at(command_name, idempotency_key, issued_at)`
- `seed_live_command(&pool, &ctx, payload)`

Fixture contracts:

- `cmd` may derive a readable request id from command name and idempotency key, but the call site must still show command name and idempotency key.
- `cmd_with_request` is required when the test asserts request id persistence, outbox `origin_request_id`, or command ledger request metadata.
- `cmd_at` is required when the test depends on issued timestamp behavior, such as omitted check-in date materialization or retry across issued-at rollover.
- Tests that assert actor metadata, client/session/channel context, canonical request hashes, lock keys, intent JSON, summary JSON, or command ledger fields must keep those assertions inline or use narrowly named helpers that expose the expected values.
- Defaults must match `WriteCommandContext::for_internal_test`: actor type `System`, actor id `test`, and issued at `2026-04-24T10:00:00+07:00`.

Payload hash helpers for group check-in and folio line idempotency should be retained or moved without changing canonical payload semantics.

### Assertion Helpers

Add narrow assertion helpers for high-frequency database checks:

- `assert_outbox_event(&pool, &ctx, event_type)`
- `assert_room_status(&pool, room_id, status)`
- `assert_booking_status(&pool, booking_id, status)`
- `assert_calendar_rows(&pool, booking_id, status, expected_count)`
- `assert_housekeeping_rows(&pool, room_id, expected_count)`
- `assert_replayed_pair(first, second)`
- transaction and folio helpers for count, sum, origin key, and ordinal assertions

Prefer specific helpers over generic SQL-string assertion helpers. Direct SQL should remain in tests when the assertion is unusual or central to the case.

## Data Flow

Refactored tests should follow this pattern:

1. Create a pool with `test_pool().await`.
2. Seed preconditions with named helpers such as `seed_room_with_price`.
3. Build the request from a minimal factory or builder, overriding only case-specific fields.
4. Create command context through a command fixture when exercising idempotent command boundaries.
5. Call the same service function currently tested.
6. Assert common database effects through helper assertions and keep unique assertions inline.

No helper should bypass a service call that the test is meant to exercise. For example, a check-in idempotency test should still call `stay_lifecycle::check_in_idempotent`; helper code can only seed the initial room/pricing state and assert the resulting outbox, calendar, and ledger rows.

## Implementation Clusters

Implementation should move through these clusters in order. Each cluster should reduce duplication only after the relevant helper contract is clear.

1. Support foundation and measurement: create the support module layout, move or wrap `test_pool`, shared file pools, outbox assertion, command fixture, and LOC accounting helpers. Verify the booking test module still compiles and runs.
2. Group lifecycle: group check-in, group checkout, duplicate room, lock-key, in-flight, group payment, and group smoke tests. Extract room/pricing seeding, group request builders, replay assertions, lock-key assertions, outbox assertions, housekeeping checks, and group booking row helpers.
3. Group service management: add/remove group service idempotency and validation tests. Extract group seed setup, service row count assertions, and idempotent replay/conflict helpers.
4. Payment, deposit, and origin ledgers: record payment/deposit/cancellation fee tests. Extract active booking setup, transaction count/sum/origin ordinal assertions, and outer transaction helpers where they shorten repeated setup.
5. Pricing: stay price and special-date tests. Extract priced room setup, special-date setup, and shared price assertion helpers only where repeated.
6. Reservation lifecycle and idempotency: create, modify, cancel, confirm, replay, conflict, stale calendar, and missing booking tests. Extract reservation builders, booked reservation setup, calendar assertions, outbox assertions, and command metadata helpers.
7. Stay lifecycle: check-in, check-out, extend stay, settlement, rollback, race, and smoke tests. Extract check-in builders, active booking terms setup, checkout request helpers, room/calendar/housekeeping assertions, and transaction sum assertions.
8. Analytics, export, and night audit: revenue, billing/export, local date, cancellation fee, and night audit tests. Extract repeated revenue setup and query assertion helpers without hiding date-boundary values.
9. Folio idempotency and origin rows: add folio line replay/conflict/metadata/invalid amount/rollback/origin tests. Extract folio request/hash helpers, folio count/origin ordinal assertions, and command metadata assertions.

Preferred verification after each cluster is the narrowest useful cargo filter for the changed test names. If a narrow filter is impractical, run `cargo test --manifest-path mhm/src-tauri/Cargo.toml services::booking::tests` before moving to the next cluster.

## PMS Assertion Preservation Checklist

The refactor must preserve coverage for these signals:

- Command boundary: command name, request id, idempotency key, actor metadata when asserted, issued timestamp when asserted, and canonical payload hash when asserted.
- Idempotency: first-run versus replay flags, same-key/different-payload conflicts, duplicate in-flight conflicts, retryable reclaim behavior, and terminal error replay behavior.
- Locking: stable lock keys, room/date locks, booking locks, group locks, selected folio locks, and tests that reject stale room or booking mappings.
- Ledger and folio: transaction and folio row counts, sums, append-only origin keys, origin ordinals, safe metadata, and no-write behavior after invalid input.
- Outbox: exactly one event where expected, event type, pending status, attempts, origin request id, origin idempotency key, origin command name, non-empty request hash, schema version, command name, and refresh payload.
- Room and calendar state: room status transitions, calendar row counts, calendar statuses, date ranges, no overwrite on conflicts, release/removal on checkout/cancel, and local date boundary behavior.
- Housekeeping: cleanup task creation, room id, status, and no duplicate rows on idempotent replay.
- Money: integer VND amounts, no floats in assertions, total price, paid amount cache, settlement totals, payment deltas, cancellation fees, and revenue projections.

## Error Handling

Setup helpers may fail hard with `expect(...)` messages because they are test infrastructure. Messages should name the operation, such as `seed standard room`, `read room status`, or `assert outbox event`.

Tests that verify business errors should keep `unwrap_err()` and `matches!` close to the service call. Do not hide `BookingError::Validation`, room-unavailable errors, idempotency conflicts, or invalid-state assertions behind broad helper names when the exact error is the point of the test.

Command/idempotency helpers must keep the current canonical hash behavior. They should call existing hash utilities rather than reimplementing serialization rules.

## Testing

Implementation should be verified incrementally:

- Run GitNexus impact analysis before modifying existing test symbols.
- After extracting support helpers, run booking service tests or the narrowest available Rust test filter that compiles the booking test module.
- After each major cluster rewrite, rerun the relevant filtered test set.
- At the end, run `cargo test --manifest-path mhm/src-tauri/Cargo.toml services::booking::tests`.
- If time is reasonable, run the broader Rust test suite or the repository verification script relevant to native tests.
- Before committing implementation changes, run `gitnexus_detect_changes()` and confirm affected scope is test/support code only.

LOC accounting must be reported before and after implementation. The baseline command is:

```bash
wc -l mhm/src-tauri/src/services/booking/tests.rs
```

After helper extraction, report total booking test LOC with:

```bash
(printf '%s\n' mhm/src-tauri/src/services/booking/tests.rs; find mhm/src-tauri/src/services/booking/tests -type f -name '*.rs' 2>/dev/null) | sort | xargs wc -l
```

The implementation should target at least a 10% net reduction across `tests.rs` plus any new support files. If all safe clusters are exhausted before that threshold, report the measured reduction and the remaining duplication rather than weakening coverage to hit a number.

Success criteria:

- Total booking test LOC decreases, including any new support files.
- `tests.rs` is materially smaller and easier to navigate.
- Booking service tests pass.
- No runtime business files are changed.
- PMS safety assertions around command boundaries, idempotency, locks, ledger/folio rows, outbox events, room state, calendar rows/status, housekeeping rows/status, and money projections remain covered.

## Risks and Mitigations

- Risk: A helper hides a critical business value and makes tests less diagnostic. Mitigation: keep behavior-defining values inline at call sites.
- Risk: Moving `test_pool` or seed helpers breaks many tests at once. Mitigation: preserve existing entry points first, then layer new helpers on top.
- Risk: Scenario-style builders become too broad and mask setup differences. Mitigation: use lightweight builders with explicit overrides, not a large DSL.
- Risk: Test-only SQL diverges from current schema setup. Mitigation: move SQL mechanically and avoid schema edits unless required by compile or existing test behavior.
- Risk: Unrelated dirty worktree changes are accidentally included. Mitigation: stage and commit only files created or edited for this refactor.

## Approved Direction

Use layered helper extraction plus targeted rewrite by clusters across the whole booking service test file. Optimize for real total LOC reduction while preserving readable test intent and full PMS safety coverage.
