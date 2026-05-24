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
- `mhm/src-tauri/src/services/booking/tests/support.rs` or `tests/support/mod.rs`: shared test-only helpers.
- Optional submodules if the support file grows too large: `db`, `seed`, `request`, `command`, and `assertions`.

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

Success criteria:

- Total booking test LOC decreases, including any new support files.
- `tests.rs` is materially smaller and easier to navigate.
- Booking service tests pass.
- No runtime business files are changed.
- PMS safety assertions around command boundaries, idempotency, ledger/folio rows, and outbox events remain covered.

## Risks and Mitigations

- Risk: A helper hides a critical business value and makes tests less diagnostic. Mitigation: keep behavior-defining values inline at call sites.
- Risk: Moving `test_pool` or seed helpers breaks many tests at once. Mitigation: preserve existing entry points first, then layer new helpers on top.
- Risk: Scenario-style builders become too broad and mask setup differences. Mitigation: use lightweight builders with explicit overrides, not a large DSL.
- Risk: Test-only SQL diverges from current schema setup. Mitigation: move SQL mechanically and avoid schema edits unless required by compile or existing test behavior.
- Risk: Unrelated dirty worktree changes are accidentally included. Mitigation: stage and commit only files created or edited for this refactor.

## Approved Direction

Use layered helper extraction plus targeted rewrite by clusters across the whole booking service test file. Optimize for real total LOC reduction while preserving readable test intent and full PMS safety coverage.
