# Booking Test LOC Reduction Pass 2 Design

## Context

The previous booking test refactor split support helpers out of
`mhm/src-tauri/src/services/booking/tests.rs` and compacted several booking
test clusters. That pass reduced `tests.rs` from 8428 lines to 6861 lines, but
the total booking test/support area only moved from 8428 lines to 8334 lines.
The net total reduction is 94 lines, or about 1.1%.

This pass must reduce real total LOC, not just move code from `tests.rs` into
`tests/support`. The preferred target is another 500 to 750 total lines removed.
The 10% total-reduction threshold is about 7585 total lines across
`tests.rs` plus `tests/support/*.rs`.

Current relevant files:

- `mhm/src-tauri/src/services/booking/tests.rs`
- `mhm/src-tauri/src/services/booking/tests/support/assertions.rs`
- `mhm/src-tauri/src/services/booking/tests/support/command.rs`
- `mhm/src-tauri/src/services/booking/tests/support/db.rs`
- `mhm/src-tauri/src/services/booking/tests/support/mod.rs`
- `mhm/src-tauri/src/services/booking/tests/support/request.rs`
- `mhm/src-tauri/src/services/booking/tests/support/seed.rs`

Implementation edit whitelist:

- `mhm/src-tauri/src/services/booking/tests.rs`
- Existing or newly added Rust files under
  `mhm/src-tauri/src/services/booking/tests/support/`

No other implementation files may be edited for this LOC-reduction pass.
Additional support files are allowed only under `tests/support/` and only if
they reduce net total LOC after their call sites are compacted.

Out of scope:

- Runtime/business files.
- Other test files, docs/spec files, config files, generated files, and
  repository metadata during implementation.
- `mhm/src/stores/useHotelStore.test.ts`, which is an unrelated dirty file and
  must not be edited, staged, or committed.
- Whole-repository formatting, because `cargo fmt --manifest-path
  mhm/src-tauri/Cargo.toml -- --check` is already known to fail outside the
  booking test scope.

## Goals

- Reduce total booking test/support LOC by deleting repeated boilerplate.
- Keep assertions as specific as they are today, especially PMS safety
  assertions around command boundaries, idempotency, outbox, ledger, folio,
  calendar, status transitions, dates, and money.
- Keep behavior-defining literals visible in tests: command names,
  idempotency keys, request ids, booking ids, room ids, dates, money amounts,
  origin keys, origin ordinals, notes, descriptions, and event names.
- Do not delete or merge away behavior scenarios only to reduce LOC. If two
  tests are merged, every invariant and assertion from each original scenario
  must remain covered with equal specificity.
- Commit each safe implementation batch only after targeted tests pass and the
  diff has been reviewed for assertion weakening.
- End with full booking tests, full Rust tests, scoped rustfmt check, LOC
  measurement, and GitNexus change detection.

## Non-Goals

- Do not change any production service, query, command, schema, model, or
  runtime behavior.
- Do not create broad scenario DSLs that hide the details each test is meant to
  prove.
- Do not replace direct SQL assertions when the SQL filters are unique,
  behavior-defining, or more diagnostic inline.
- Do not table-drive complex PMS safety tests that have different side effects
  or different database invariants.
- Do not chase the 10% target by weakening assertions.

## Current Helper Baseline

Existing support already includes useful primitives:

- Command helpers: `cmd`, `cmd_with_request`, `cmd_at`, and
  `seed_live_in_progress_command`.
- Request builders for check-in, reservation, group check-in, and checkout.
- Seed helpers for rooms, pricing rules, active bookings, booked
  reservations, transactions, folio lines, expenses, and room-with-pricing
  setup.
- Assertion helpers for replay pairs, single outbox events, room status,
  booking status, calendar rows with status, housekeeping rows with status,
  transaction origin rows, transaction count/sum, folio origin rows, and folio
  line count by origin key.

Pass 2 should extend this layer only where a helper removes more repeated
lines than it adds and preserves the exact filters currently asserted.

## Proposed Approach

Use narrow helper extraction plus conservative call-site compaction in small
verified batches. This is the recommended approach because it reduces real
duplication while keeping each test's PMS safety intent readable.

Rejected approaches:

- A large scenario DSL. It could remove more lines, but it would hide too many
  behavior-defining values and repeat the assertion-specificity problems found
  in prior reviews.
- A table-driven-only pass. It is safe for a few simple validations, but it
  will not produce enough total LOC reduction by itself.

## Batch 1: SQL Assertion Helpers

Add very narrow helpers in `tests/support/assertions.rs` only when they preserve
the full current SQL predicate. Good candidates:

- Table-specific row-count helpers where the helper signature contains every
  behavior-visible predicate from the current assertion. For example, a
  calendar helper that currently filters by status must keep status in the
  signature, and an origin helper that currently filters by booking id and
  ordinal must keep both values in the signature.
- `command_claim_count(pool, command_name, idempotency_key)`.
- `command_claim_count_by_request(pool, command_name, request_id)`.
- `outbox_count(pool, command_name, idempotency_key)`.
- `origin_transaction_count(pool, origin_key)`.
- `origin_folio_line_count(pool, origin_key)`.
- `transaction_count(pool, booking_id, txn_type, note)` only for assertions
  whose current predicate is exactly booking id, type, and optionally note.
- `transaction_sum(pool, booking_id, txn_type, note)` only for repeated sum
  assertions with the same booking/type/note predicate.
- Checkout settlement money counts must either keep the SQL inline or pass the
  expected note set explicitly at the call site. Do not hide checkout money
  note literals inside a helper.

Avoid helpers that drop filters. If an existing assertion includes
`booking_id`, `note`, `description`, `status`, `origin_idempotency_key`,
`origin_*_ordinal`, or event metadata, the helper must either include the same
filter or the assertion must stay inline. Do not use optional helper arguments
to combine meaningfully different predicates into one broad helper.

Expected impact: moderate LOC reduction with low risk, mainly in idempotency
pre-claim/no-write tests, retry duplicate checks, and simple table count
assertions.

## Batch 2: Small Scenario Fixtures

Add small fixtures only for setup repeated many times with no custom
behavior-defining dates or state changes between setup and action. Good
candidates:

- Reuse the existing `seed_room_with_price(pool, room_id, daily_rate)` where it
  removes repeated `seed_room` plus `seed_pricing_rule` in single-room tests
  without changing literal visibility.
- `seed_active_booking_with_prior_payment(pool, booking_id, room_id, amount,
  note)` for tests that always seed an active booking and immediately record a
  prior payment before checkout.
- `seed_booked_reservation_with_pricing(pool, booking_id, room_id, daily_rate)`
  for reservation lifecycle tests that always seed room, pricing, and the same
  booked reservation fixture.

These fixtures are only allowed when all behavior-visible values remain either
explicit at the call site or irrelevant to the assertion under test. Dates,
status, money amounts, transaction type, origin key/ordinal, notes, and
descriptions must be parameters when the test depends on them.

Do not use fixtures for tests where setup details are the behavior under test:

- Date-boundary reporting tests.
- Custom booking terms or corrupted booking state.
- Manual checkout settlement tests.
- Trigger, rollback, two-pool, race, stale room/calendar, or lock conflict
  tests.
- Night audit setup where explicit rows document revenue recognition.

Expected impact: moderate LOC reduction in reservation and checkout sections,
without changing runtime paths.

## Batch 3: Table-Driven Simple Validation Tests

Use table-driven loops only for structurally identical negative cases:

- Validation rejects invalid money before a command claim.
- The setup, service call shape, expected error code, and no-write assertions
  are identical.
- Each case has a clear label or idempotency key so failures are debuggable.
- Each table row keeps the invalid amount, command name, idempotency key,
  request id when relevant, expected error code, and no-write assertion target
  explicit.

Do not table-drive tests with different side effects, different command
metadata assertions, different outbox expectations, or different rollback
invariants.

Expected impact: small to moderate LOC reduction with good readability if
limited to simple cases.

## Batch 4: Retry and Request Compaction

Compact duplicated request construction and duplicated idempotent service calls
where doing so keeps behavior-visible values at the call site. Examples:

- Reuse one request value by cloning only if the model type supports cloning and
  the test is not proving canonical serialization from separately built
  requests.
- Use short local closures for identical repeated calls inside a single test
  when the command name, request id, idempotency key, booking id, money amount,
  and note remain visible.
- Keep `cmd_with_request` whenever request id or outbox
  `origin_request_id` is behavior-visible.

Do not use `assert_replayed_pair` unless the current test asserts all of:
first response is not replayed, second response is replayed, and full response
equality. If the current test only compares `response["id"]` or another subset,
keep that narrower assertion shape.

Expected impact: moderate LOC reduction in replay-heavy sections.

## Batch 5: Conservative Reporting Cleanup

Reporting, export, and night audit tests contain repeated seed/update/query
shapes, but many literals are behavior-defining. This batch should only remove
mechanical repetition that does not hide date-boundary or revenue-recognition
intent.

Allowed examples:

- A helper to mark a booking checked out with a provided pricing snapshot,
  actual checkout, nights, total, and paid amount when all those values remain
  explicit at the call site. The helper must not seed or modify ledger,
  folio, outbox, or origin rows.
- A helper to fetch one booking export row by id after calling
  `load_booking_export_rows`.

Avoid hiding explicit transaction rows in night audit and reporting tests when
the row date, type, note, or amount documents the expected financial behavior.
Do not helperize explicit ledger, folio, transaction, or expense rows in night
audit/reporting tests when those rows define revenue recognition behavior.

Expected impact: limited but safe LOC reduction.

## GitNexus and Review Requirements

Before editing any existing function, method, class, or helper symbol, run
GitNexus impact analysis for that symbol with upstream direction and report the
blast radius. If GitNexus reports HIGH or CRITICAL risk, warn the user before
proceeding. If that HIGH or CRITICAL risk reaches outside the edit whitelist,
stop and report before editing.

Before each implementation commit:

- Run the relevant targeted tests.
- Review the diff for assertion weakening.
- Run `gitnexus detect_changes` for the changed scope.
- Stage and commit only the files edited for that batch.

The GitNexus index may lag behind this branch. Local files are the source of
truth for current code shape, but GitNexus impact and change detection still
must be used to satisfy project rules.

## Final Verification

At the end of implementation, run:

```bash
cargo test --manifest-path mhm/src-tauri/Cargo.toml services::booking::tests
cargo test --manifest-path mhm/src-tauri/Cargo.toml
rustfmt --edition 2021 --check mhm/src-tauri/src/services/booking/tests.rs mhm/src-tauri/src/services/booking/tests/support/*.rs
wc -l mhm/src-tauri/src/services/booking/tests.rs
(printf '%s\n' mhm/src-tauri/src/services/booking/tests.rs; find mhm/src-tauri/src/services/booking/tests -type f -name '*.rs' 2>/dev/null) | sort | xargs wc -l
```

Then run GitNexus `detect_changes` and report the affected scope.

Do not run whole-repository formatting as a pass/fail gate for this task,
because unrelated files outside booking already fail that check.

## Reporting Requirements

The final implementation report must include:

- `tests.rs` line count versus original 8428.
- Total booking test/support line count versus original 8428.
- Total booking test/support line count versus pre-pass-2 baseline 8334.
- Whether the 10% target was reached.
- If the target was not reached, the exact reason safe refactoring stopped.
- Verification commands run and their results.
- Confirmation that `mhm/src/stores/useHotelStore.test.ts` was not touched or
  staged.

## Success Criteria

- Total booking test/support LOC is reduced by real deletion, not support-file
  line shifting.
- Booking service tests pass after each committed batch.
- Full Rust tests pass at the end.
- Scoped rustfmt check passes for booking test/support files.
- Runtime/business files remain unchanged.
- Assertion specificity is preserved for PMS safety behavior.
