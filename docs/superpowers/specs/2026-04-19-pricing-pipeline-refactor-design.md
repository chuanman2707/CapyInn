# HotelManager Pricing Pipeline Refactor Design

Date: 2026-04-19
Owner: Codex
Status: Draft approved for spec write-up

## Goal

Refactor `mhm/src-tauri/src/domain/booking/pricing.rs` so the stay-pricing pipeline is split into three explicit phases:

- load inputs
- build effective pricing rule
- calculate price

The refactor should make four things true:

- `calculate_stay_price_tx(...)` stops mixing database reads, fallback-rule construction, and pricing-engine invocation in one function
- `calculate_stay_price(...)` and `calculate_stay_price_tx(...)` share one core pricing path after data loading
- fallback pricing behavior is defined in one internal place instead of being duplicated across `tx` and non-`tx` helpers
- the current booking flows and pricing results keep working without public API changes

This design is intentionally a focused backend refactor. It does not introduce new pricing features, new pricing models, or a new public contract in this pass.

## User Decisions Locked In

The following decisions were explicitly chosen during brainstorming and are part of this spec:

- selected approach: structured hybrid
- public behavior: preserve current public API and booking-flow behavior
- internal contract flexibility: helper and struct names may change if that produces cleaner boundaries
- file layout: keep the refactor inside `mhm/src-tauri/src/domain/booking/pricing.rs`
- architectural target: separate the pipeline into `load inputs -> build effective rule -> calculate`

## Constraints

- `calculate_stay_price_tx(...)` currently has a `CRITICAL` upstream blast radius in GitNexus, with direct callers in:
  - `reservation_lifecycle::create_reservation`
  - `reservation_lifecycle::confirm_reservation`
  - `reservation_lifecycle::modify_reservation`
  - `stay_lifecycle::check_in`
  - `stay_lifecycle::extend_stay`
  - `group_lifecycle::group_checkin`
  - booking tests
- the `tx` path must keep reading uncommitted pricing data inside the current transaction
- the regression guard for that invariant already exists in `calculate_stay_price_tx_reads_uncommitted_pricing_rule`
- `crate::pricing::calculate_price(...)` in `mhm/src-tauri/src/pricing.rs` is already the pure pricing engine and should remain the calculation endpoint
- the duplicate logic today is not only in the two public entrypoints, but also in:
  - `load_pricing_rule(...)` vs `load_pricing_rule_tx(...)`
  - fallback rule construction embedded inside those loaders
- the refactor should avoid introducing `sqlx` executor abstractions or lifetime-heavy generic plumbing unless absolutely necessary

## Existing State

Today both public entrypoints in `mhm/src-tauri/src/domain/booking/pricing.rs` perform the same sequence:

1. load room type from `rooms`
2. load pricing rule from `pricing_rules`, with fallback derivation from `rooms.base_price`
3. load special-date uplift from `special_dates`
4. call `crate::pricing::calculate_price(...)`

The only material difference is whether the reads happen via `Pool<Sqlite>` or `Transaction<'_, Sqlite>`.

This creates three concrete problems:

1. The public functions own too much orchestration detail.
2. Fallback-rule construction is duplicated and hidden inside the data-loading helpers.
3. The `tx` and non-`tx` code paths can drift even though they should make identical pricing decisions after data is loaded.

Functionally the current code works. The problem is boundary clarity and duplication in a high-impact path.

## Chosen Approach

Use a structured hybrid design:

- keep separate thin loaders for `Pool<Sqlite>` and `Transaction<'_, Sqlite>`
- make those loaders return one shared internal input shape
- move effective-rule construction into one internal function
- move final pricing invocation into one internal function

Why this approach:

- it reaches the desired architecture without fighting `sqlx` executor and lifetime complexity
- it preserves the key `tx` invariant of seeing uncommitted rows
- it removes the real duplication, which is pricing decision logic rather than the mechanical choice of database handle
- it keeps the refactor localized to one file and minimizes churn across booking services

## Non-Goals

- changing the signatures of `calculate_stay_price(...)` or `calculate_stay_price_tx(...)`
- changing pricing outputs, surcharge semantics, or fallback numbers
- moving pricing pipeline logic into a new file or module in this pass
- replacing `crate::pricing::calculate_price(...)`
- introducing traits, repositories, or generic executors for `Pool` and `Transaction`
- changing the call sites in booking flows except for compile-preserving internal cleanup if required

## Target Shape Inside `pricing.rs`

The refactored file should read as three layers.

### 1. Public entrypoints

These remain:

- `calculate_stay_price(pool, room_id, check_in, check_out, pricing_type)`
- `calculate_stay_price_tx(tx, room_id, check_in, check_out, pricing_type)`

After the refactor, each public function should only:

- call its matching loader
- pass the loaded inputs into the shared core path

They should stop building pricing rules directly.

### 2. Loaders

Two thin loaders remain because the repo needs both database contexts:

- `load_stay_pricing_inputs(...)`
- `load_stay_pricing_inputs_tx(...)`

These loaders should only gather facts from storage. They should not decide how to build the final `PricingRule`.

Expected facts:

- `room_type`
- pricing rule row if present
- fallback `base_price` if needed
- `special_uplift_pct`
- original `check_in`
- original `check_out`
- requested `pricing_type`

### 3. Shared pricing core

The shared core should consist of two internal steps:

- `build_effective_pricing_rule(...)`
- `calculate_from_loaded_inputs(...)`

`build_effective_pricing_rule(...)` is the only place allowed to decide:

- use the stored pricing rule from `pricing_rules` when present
- otherwise derive the fallback rule from `rooms.base_price`
- otherwise fall back to the hardcoded default base price of `350_000`

`calculate_from_loaded_inputs(...)` should:

- obtain the effective `PricingRule`
- call `crate::pricing::calculate_price(...)`
- map pricing parse errors to `BookingError::datetime_parse(...)` exactly as today

## Internal Data Contracts

The names may change during implementation, but the boundaries should look like this.

### `StayPricingInputs`

This struct represents loaded facts, not pricing decisions.

It should contain:

- `room_type: String`
- `stored_rule: Option<StoredPricingRule>`
- `fallback_base_price: Option<f64>`
- `special_uplift_pct: f64`
- `check_in: String`
- `check_out: String`
- `pricing_type: String`

This allows the shared core to operate without knowing whether the data came from `Pool` or `Transaction`.

### `StoredPricingRule`

This internal struct represents a concrete row loaded from `pricing_rules` before fallback decisions are applied.

It should contain the fields needed to build `crate::pricing::PricingRule`, including:

- `room_type`
- `hourly_rate`
- `overnight_rate`
- `daily_rate`
- `overnight_start`
- `overnight_end`
- `daily_checkin`
- `daily_checkout`
- `early_checkin_surcharge_pct`
- `late_checkout_surcharge_pct`
- `weekend_uplift_pct`

This boundary makes the fallback decision explicit instead of encoding it in the loader implementation.

## Detailed Data Flow

The intended internal flow is:

1. public entrypoint receives `room_id`, `check_in`, `check_out`, `pricing_type`
2. loader resolves `room_type`
3. loader attempts to read the matching `pricing_rules` row
4. loader reads fallback `base_price` from `rooms` when needed for rule construction
5. loader reads special-date uplift for the requested check-in date
6. loader returns `StayPricingInputs`
7. shared core builds the effective `PricingRule`
8. shared core calls `crate::pricing::calculate_price(...)`
9. shared core maps engine errors into `BookingError`

The critical design rule is:

- loaders gather facts
- the core decides pricing

That rule is the main protection against duplicate fallback logic reappearing later.

## Fallback Rule Semantics

The refactor must preserve the current fallback behavior exactly:

- when a pricing rule row exists for the room type, use it
- when no pricing rule row exists, look at `rooms.base_price`
- when `rooms.base_price` is absent, use `350_000`
- derive fallback values exactly as today:
  - `hourly_rate = fallback_price / 5.0`
  - `overnight_rate = fallback_price * 0.75`
  - `daily_rate = fallback_price`
  - all other rule fields come from `PricingRule::default()`

This logic should exist in one place only after the refactor.

## Error Handling and Invariants

The refactor should preserve the current error model:

- missing room still returns `BookingError::not_found(...)`
- SQL failures still map to `BookingError::database(...)`
- invalid date or datetime strings from the pricing engine still map to `BookingError::datetime_parse(...)`

The refactor must also preserve these invariants:

- `calculate_stay_price_tx(...)` reads uncommitted pricing data from the active transaction
- the `tx` loader keeps all pricing facts inside the transaction boundary, including:
  - room type reads
  - stored pricing rule reads
  - fallback `rooms.base_price` reads
  - `special_dates` uplift reads
- `tx` and non-`tx` paths produce the same effective pricing decisions when they see the same stored facts
- no caller in booking flows needs to know whether the pricing rule was loaded from DB or derived as fallback

## Implementation Notes

The implementation should prefer private helper extraction over broad rewrites.

Practical guidance for the edit:

- keep `read_f64(...)` or an equivalent narrow helper
- keep SQL text local to the current file
- avoid introducing generic helper signatures parameterized over executor types
- prefer converting `sqlx::Row` into internal structs quickly rather than passing rows deeper into the pipeline
- keep the public entrypoints near the top of the file so the file still reads from API to implementation

## Verification Strategy

Verification should cover both boundary correctness and regression safety.

### Unit-level checks for the new boundary

Add targeted tests for the shared core logic:

- building the effective rule from an existing stored rule
- building the effective fallback rule from `fallback_base_price`
- building the default fallback rule when both stored rule and base price are absent
- asserting that fallback-derived rules still inherit the expected `PricingRule::default()` fields beyond the three computed rates

These tests should focus on the new core helpers rather than re-testing `crate::pricing::calculate_price(...)` exhaustively.

### Regression checks for transactional behavior

Keep and run:

- `calculate_stay_price_tx_reads_uncommitted_pricing_rule`
- add a transaction regression that proves fallback `rooms.base_price` is still read from uncommitted transactional state
- add a transaction regression that proves `special_dates` uplift is still read from uncommitted transactional state

This remains the key guard for the transaction-specific contract.

### Direct parity and pricing-path regression checks

Add direct pricing-entrypoint coverage for cases not protected by booking services:

- at least one direct parity test showing `calculate_stay_price(...)` and `calculate_stay_price_tx(...)` return the same result for the same stored facts
- one direct regression covering holiday or special-date uplift
- pricing-specific error mapping checks for:
  - missing room -> `BookingError::not_found(...)`
  - invalid datetime input -> `BookingError::datetime_parse(...)`

### Booking-flow regression checks

Run the affected booking integration tests that exercise the high-risk callers:

- `create_reservation_blocks_calendar_and_posts_deposit`
- the `group_checkin*` tests because `group_checkin` is a direct transactional caller
- the `confirm_reservation*` tests that reprice reservations
- the `modify_reservation*` tests
- `check_in` and `extend_stay` tests that depend on transactional pricing

The `check_in` tests should be treated as booking-flow smoke coverage, not as the primary regression guard for pricing correctness. The direct pricing-entrypoint tests above should carry that responsibility.

## Completion Criteria

This refactor is complete when:

- `calculate_stay_price(...)` and `calculate_stay_price_tx(...)` remain public and behavior-compatible
- the file clearly reflects `load inputs -> build effective rule -> calculate`
- fallback-rule construction exists in one shared internal place
- duplicated pricing decision logic between `tx` and non-`tx` paths is removed
- the uncommitted-data transaction test still passes
- booking-flow regression tests still pass

## Risks

The main risks are:

- accidentally changing fallback pricing values while moving the logic
- accidentally changing error mapping by moving `map_err(...)`
- accidentally reading fallback data outside the active transaction in the `tx` path

The implementation should optimize for preserving behavior over reducing every line of duplication.
