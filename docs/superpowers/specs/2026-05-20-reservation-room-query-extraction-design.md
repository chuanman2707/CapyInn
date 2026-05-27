# Reservation And Room Query Extraction Design

Date: 2026-05-20
Status: Approved for spec review
Issues: #151, #152

## Purpose

Move selected reservation, room, calendar, and availability read SQL out of Tauri command modules and into a booking query module without changing behavior. This combines #151 and #152 in one implementation because the selected reads are small and share the same command/query boundary goal.

## Scope

This design uses the narrow #152 scope approved during brainstorming:

- reservation availability reads;
- room calendar reads;
- room availability overview reads;
- room list reads;
- room detail reads.

The implementation should touch:

- `mhm/src-tauri/src/commands/reservations.rs`
- `mhm/src-tauri/src/commands/rooms.rs`
- `mhm/src-tauri/src/queries/booking/mod.rs`
- new `mhm/src-tauri/src/queries/booking/room_queries.rs`

## Non-Goals

- Do not change reservation write lifecycle behavior.
- Do not change check-in, checkout, extend-stay, housekeeping, expense, or stay-info behavior.
- Do not change command names, gateway tool names, frontend payloads, or response DTO shapes.
- Do not change SQL result shape, ordering, or date inclusivity.
- Do not introduce a broad command-layer cleanup beyond the selected reads.

## Current Context

`commands/reservations.rs` currently contains read SQL for:

- `do_check_availability`;
- `get_room_calendar`;
- `do_get_rooms_availability`.

`commands/rooms.rs` currently contains read SQL for:

- `do_get_rooms`;
- `do_get_room_detail`.

The same command helpers are used by Tauri commands and read-only gateway tools. GitNexus impact analysis for the selected helpers reported LOW risk, with direct callers limited mainly to command wrappers and gateway read tools.

## Architecture

Add one query module:

```text
mhm/src-tauri/src/queries/booking/room_queries.rs
```

Expose it from:

```rust
pub mod room_queries;
```

The module owns these read functions:

- `load_rooms`
- `load_room_detail`
- `check_room_availability`
- `load_room_calendar`
- `load_rooms_availability`

The command modules remain the public orchestration boundary:

- `commands::rooms::do_get_rooms` calls `room_queries::load_rooms`.
- `commands::rooms::do_get_room_detail` calls `room_queries::load_room_detail`.
- `commands::reservations::do_check_availability` calls `room_queries::check_room_availability`.
- `commands::reservations::get_room_calendar` calls `room_queries::load_room_calendar`.
- `commands::reservations::do_get_rooms_availability` calls `room_queries::load_rooms_availability`.

Commands should not retain the selected SQL or row-to-DTO mapping after extraction. They should keep their existing public signatures and map query errors into the same `String` contract used today.

## Data Flow

The read flow remains:

1. UI or gateway calls the existing Tauri command or gateway helper.
2. Command passes `&Pool<Sqlite>` and request parameters to `room_queries`.
3. Query module executes the existing SQL with the same bind values.
4. Query module maps rows into the existing model DTOs.
5. Command returns the existing response shape.

Behavior details to preserve:

- `check_availability` keeps `date >= from_date AND date < to_date`, conflicts ordered ascending, and `max_nights` calculated from the first conflict.
- `get_room_calendar` keeps `date >= from AND date <= to`, entries ordered ascending.
- `get_rooms` keeps ordering by `floor, id`.
- `get_room_detail` keeps the same room lookup, active booking lookup, and booking guest lookup.
- `get_rooms_availability` keeps room ordering by `id`, active booking lookup, booked upcoming reservations from today onward, and `next_available_until` from the first upcoming reservation.

## Error Handling

Most query functions should return `Result<T, sqlx::Error>` and let commands convert errors with `e.to_string()`.

`check_room_availability` may return `Result<AvailabilityResult, String>` because the existing command combines database errors and date parse errors into the same `String` surface. This avoids introducing a new error abstraction for a behavior-preserving extraction.

No new command error codes, command safety behavior, or write-path behavior should be introduced.

## Testing And Validation

Run:

```bash
cargo test --manifest-path mhm/src-tauri/Cargo.toml
cargo clippy --manifest-path mhm/src-tauri/Cargo.toml --all-targets -- -D warnings
```

Run targeted SQL checks:

```bash
rg -n "sqlx::query|query_as|query_scalar" mhm/src-tauri/src/commands/reservations.rs
rg -n "sqlx::query|query_as|query_scalar" mhm/src-tauri/src/commands/rooms.rs
```

Expected grep results after the implementation:

- `commands/reservations.rs` should have no selected read SQL remaining.
- `commands/rooms.rs` should no longer contain SQL for `do_get_rooms` or `do_get_room_detail`.
- `commands/rooms.rs` may still contain SQL for housekeeping, expenses, stay info, and write-adjacent flows because those are outside this approved scope.

Before committing the implementation, run GitNexus change detection to confirm only expected symbols and flows are affected.

## Acceptance Criteria

- #151 and #152 are addressed in a single implementation PR.
- Selected read SQL and row-to-DTO mapping move into `queries::booking::room_queries`.
- Command helpers remain stable orchestration entry points for Tauri and gateway callers.
- Frontend and gateway payloads are unchanged.
- Reservation and room write lifecycle code is untouched except for import cleanup if needed.
- Validation commands pass, or any pre-existing baseline failure is recorded explicitly.
