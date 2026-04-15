# Backend Booking Domain Refactor Design

Date: 2026-04-15
Project: HotelManager / MHM
Status: Approved in brainstorming, ready for implementation planning after spec review

## Goal

Refactor the backend booking domain to make the codebase simpler, leaner, and more reliable by moving business rules out of Tauri command handlers and into explicit domain and service boundaries.

This phase is not a rewrite. It is a controlled refactor focused on:
- reducing duplicated lifecycle logic
- making transaction boundaries explicit
- defining a single source of truth for occupancy and money movement
- standardizing booking state transitions
- making future frontend and reporting work depend on cleaner backend contracts

## Why This Refactor Exists

The current backend already improved from the old monolithic `commands.rs`, but the booking domain is still spread across multiple command modules:
- `commands/rooms.rs`
- `commands/reservations.rs`
- `commands/groups.rs`
- `commands/billing.rs`
- `commands/audit.rs`

The same concepts are currently handled in several places:
- occupancy and room availability
- booking lifecycle transitions
- guest creation and guest reuse
- price calculation
- charge, deposit, and payment posting
- room status synchronization
- report and audit revenue logic

That creates logic drift. Similar flows mutate the same tables with slightly different assumptions, which makes the code harder to reason about and easier to break.

## Phase 1 Scope

Phase 1 is intentionally narrowed to avoid turning the refactor into a rewrite.

### In Scope

- booking stay lifecycle
- reservation lifecycle
- billing and folio write rules
- guest assignment and guest identity handling
- group-aware behavior only where it shares the same booking invariants
- read-side query unification where needed to support correctness

### Out of Scope

- frontend redesign
- full gateway redesign
- OCR and file watcher redesign
- full analytics rewrite
- full audit/reporting service extraction
- generic repository abstraction across the whole backend

## Refactor Strategy

Use a service-boundary refactor.

This means:
- keep Tauri commands as thin transport adapters
- move lifecycle orchestration into service modules
- move invariants, status rules, and validation into domain modules
- isolate SQL and row mapping in repositories
- separate read-side queries from write flows

This is preferred over:
- guardrail-only cleanup, because that would reduce duplication without fixing the core boundary problem
- ledger-first re-architecture, because that would be too large for a first phase

## Target Architecture

### Layers

#### `commands/*`

Responsibilities:
- receive Tauri payloads
- call one service entry point
- map typed domain errors to transport-friendly responses

Non-responsibilities:
- no business rule decisions
- no ad hoc SQL orchestration
- no direct lifecycle branching
- no direct price calculation

#### `domain/booking/*`

Responsibilities:
- booking status model
- transition rules
- invariant checks
- validation logic
- pricing policy integration
- error taxonomy
- shared lifecycle policies

#### `services/booking/*`

Responsibilities:
- orchestrate use cases
- own transaction boundaries
- call repositories and domain policies
- update cached operational fields through one code path

#### `repositories/booking/*`

Responsibilities:
- SQL reads and writes
- locking and row selection
- persistence mapping

Non-responsibilities:
- no lifecycle rules
- no cross-aggregate orchestration

#### `queries/booking/*`

Responsibilities:
- read models for dashboard, exports, analytics, and audit views
- reporting queries based on approved source-of-truth rules

## Concrete Module Plan

### Services in Phase 1

#### `services/booking/stay_lifecycle.rs`

Owns:
- `check_in`
- `check_out`
- `extend_stay`
- room-transfer behavior if it shares the same occupancy invariants

#### `services/booking/reservation_lifecycle.rs`

Owns:
- `create_reservation`
- `confirm_reservation`
- `modify_reservation`
- `cancel_reservation`
- group reservation behavior when still in booked state

#### `services/booking/billing_service.rs`

Owns:
- posting `charge`
- posting `payment`
- posting `deposit`
- posting `refund`
- posting `adjustment`
- posting `cancellation_fee`
- outstanding balance calculations
- settlement policy for checkout and group-aware checkout

#### `services/booking/guest_service.rs`

Owns:
- guest reuse policy
- guest creation policy
- placeholder guest policy
- primary guest assignment rules

### Modules That Stay Policy-Oriented in Phase 1

#### `domain/booking/pricing.rs`

The existing pricing engine should remain a domain policy module unless orchestration proves otherwise.

Phase 1 requirement:
- every write flow that computes price must route through this policy
- no write flow may calculate `base_price * nights` directly

#### `queries/booking/audit_queries.rs`

Night audit and reporting stay read-side in phase 1.

Phase 1 requirement:
- reporting must read through a unified revenue policy
- reporting must not invent separate revenue meanings per command

## Source of Truth and Write Ownership

The refactor must explicitly define which data is canonical and which data is cached or operational.

| Concept | Canonical Source | Cached / Operational Field | Write Owner |
|---|---|---|---|
| Occupancy by date | `room_calendar` + booking lifecycle rules | `rooms.status` | `stay_lifecycle`, `reservation_lifecycle` |
| Booking lifecycle | `bookings.status` | none | `stay_lifecycle`, `reservation_lifecycle` |
| Money movement | `transactions` | `bookings.paid_amount` if retained | `billing_service` |
| Ancillary itemization | `folio_lines` | derived totals in read models | `billing_service` |
| Group membership and metadata | `booking_groups` + `bookings.group_id` | master room markers | lifecycle services |
| Guest-to-booking relation | `booking_guests` | `primary_guest_id` convenience field | `guest_service` + lifecycle services |
| Reservation schedule | `scheduled_checkin_date`, `scheduled_checkout_date` or existing equivalents until migration completes | none | `reservation_lifecycle` |

Rules:
- only lifecycle services may write `rooms.status`
- only `billing_service` may write `transactions`
- commands may not write cached fields directly
- cached fields must be synchronized only by their owning service

## Booking State Model

The refactor must make lifecycle transitions explicit.

### Booking statuses

- `booked`
- `active`
- `checked_out`
- `cancelled`
- `no_show` if the product keeps this state

### Required transition matrix

| Current State | Event | Next State | Owner |
|---|---|---|---|
| `booked` | confirm reservation / arrival | `active` | `reservation_lifecycle` |
| `booked` | cancel reservation | `cancelled` | `reservation_lifecycle` |
| `booked` | mark no-show | `no_show` | `reservation_lifecycle` |
| `active` | checkout | `checked_out` | `stay_lifecycle` |
| `active` | extend stay | `active` | `stay_lifecycle` |
| `active` | transfer room | `active` | `stay_lifecycle` |

Additional explicit rules required in implementation planning:
- early confirmation before scheduled date
- late arrival after scheduled date
- partial group checkout
- master-room reassignment
- reservation date modification after conflicts are checked

## Core Invariants

Phase 1 must preserve these invariants:

1. No booking may become `active` if occupancy conflicts with existing `room_calendar` state.
2. No reservation may be created or modified into a conflicting date range.
3. No write flow may partially persist guest, booking, calendar, ledger, and room status changes outside a transaction boundary.
4. No command may compute price outside the pricing policy module.
5. No reporting path may use a revenue definition different from the approved revenue policy.
6. No direct SQL update may bypass the booking transition rules once the lifecycle services exist.

## Time Model

The current backend mixes:
- RFC3339 datetimes
- `YYYY-MM-DD` date strings
- hand-built timestamp strings

Phase 1 standardization:

### Operational datetimes

Persist as RFC3339:
- `check_in_at`
- `expected_checkout`
- `actual_checkout`
- ledger event timestamps

### Scheduled reservation dates

Persist as `YYYY-MM-DD`:
- `scheduled_checkin_date`
- `scheduled_checkout_date`

### Calendar occupancy dates

Persist as `YYYY-MM-DD`:
- `room_calendar.date`

Meaning:
- operational datetime answers "when did the real event happen"
- scheduled date answers "what date was reserved"
- calendar date answers "what dates are blocked or occupied"

## Financial Model

Phase 1 treats `transactions` as the canonical ledger for money movement.

### Ledger vocabulary

Allowed types should be normalized around:
- `charge`
- `payment`
- `deposit`
- `refund`
- `adjustment`
- `cancellation_fee`

### Folio model

`folio_lines` remains the canonical itemization layer for ancillary charges.

It is not a separate ledger. It feeds:
- billing summaries
- invoices
- reporting queries

### Cached financial fields

`bookings.paid_amount` may remain temporarily to reduce migration pressure, but it must be treated as cached state only.

Rules:
- it is updated only through `billing_service`
- it is never the sole source of truth for settlement or reporting

## Guest Identity Contract

Phase 1 should support safer guest reuse.

Preferred request contract direction:
- `guest_id?`
- `doc_number?`
- `phone?`

Guest policy:
- reuse by `guest_id` if present
- otherwise match using policy on `doc_number` and `phone`
- placeholder guest creation must be explicit, not hidden behind empty strings

## Error Model

Internally, phase 1 should introduce typed domain errors, even if Tauri commands still return strings initially.

Recommended internal taxonomy:
- `ValidationError`
- `ConflictError`
- `NotFoundError`
- `AuthorizationError`
- `InvariantViolation`
- `PersistenceError`

Transport rule:
- command layer maps typed errors to strings or transport-safe payloads
- domain and service logic should stop returning ad hoc free-form error strings directly

## Migration Strategy

Phase 1 must not use a big-bang schema rewrite.

Required migration sequence:
1. Additive columns and indexes first
2. Backfill old rows if new semantics are introduced
3. Dual-read and dual-write where necessary
4. Move read paths to the new canonical rules
5. Remove deprecated writes later, not in the same risky slice unless trivial

Examples of likely migration-sensitive areas:
- `paid_amount`
- transaction type normalization
- any new ledger event timestamp such as `occurred_at`
- scheduled reservation date fields
- any renamed time fields used by invoices or exports

## Rollout Plan

Phase 1 should be delivered in thin vertical slices.

### Slice 1: Foundation

- domain errors
- transaction runner pattern
- time normalization helpers
- shared lifecycle utilities
- pricing policy integration point

### Slice 2: Stay Lifecycle

- `check_in`
- `check_out`
- `extend_stay`
- room status synchronization
- occupancy conflict enforcement

### Slice 3: Reservation Lifecycle

- `create_reservation`
- `confirm_reservation`
- `modify_reservation`
- `cancel_reservation`
- group-aware booked flows where they share reservation invariants

### Slice 4: Billing and Folio

- canonical ledger posting through `billing_service`
- cached payment field synchronization
- settlement rules for checkout and group-aware checkout
- invoice dependencies updated to read the approved financial model

### Slice 5: Read-Side Unification

- night audit queries
- analytics queries
- export queries
- remove mixed revenue logic from scattered command paths

## Testing Strategy

The current test suite is mostly frontend E2E with mocked invokes. Phase 1 requires backend-focused coverage.

### Domain unit tests

Test:
- status transitions
- validation rules
- guest matching rules
- pricing normalization rules
- settlement edge cases

### Backend integration tests with SQLite

Test:
- atomic check-in writes
- check-in rollback on failure
- reservation conflict prevention
- reservation confirm and cancel transitions
- extend-stay date correctness
- group-aware transitions that share lifecycle rules
- revenue consistency between booking, ledger, and folio state

### Thin command smoke tests

Test only:
- command-to-service mapping
- error translation at the transport edge

## Definition of Done

Phase 1 is done when:
- core booking write flows no longer keep business logic directly in Tauri commands
- lifecycle transitions are enforced through service and domain boundaries
- price calculation is routed through one pricing policy path
- occupancy writes and room status synchronization follow one write-owner model
- ledger posting flows go through one billing service path
- the backend has integration tests for the highest-risk invariants
- reporting queries stop mixing incompatible revenue definitions

## Non-Goals

This phase does not aim to:
- redesign the frontend architecture
- fully replace every legacy field immediately
- rebuild analytics from scratch
- redesign OCR or watcher internals
- extract every domain in the backend into the same architecture at once

## Approval Summary

Approved during brainstorming with the following final direction:
- backend/domain simplification comes first
- priority order is correctness boundary first, then domain boundary, then data boundary
- phase 1 may change schema and command contracts when justified
- phase 1 uses service-boundary refactor, narrowed after review to avoid rewrite risk

