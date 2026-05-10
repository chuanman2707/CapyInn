# Migration Safety Guard Design

Issue: #136 ARCH-03 Batch 0: Add migration regression test before migration movement

Parent roadmap: #133 Core PMS Architecture Stabilization V2.1

Date: 2026-05-10
Status: Approved for implementation planning

## Purpose

Add a migration regression guard before any migration module movement. The guard should prove that a fresh database still migrates to the latest schema and creates the table surfaces that core PMS flows, command safety, and experimental integration layers depend on.

This slice is intentionally test-only. It must not move migrations, change production schema SQL, rewrite the migration test suite, or touch command safety behavior.

## Scope

- Add one fresh-database migration regression test in `mhm/src-tauri/src/db.rs`.
- Keep the test inside the existing `#[cfg(test)] mod tests` block.
- Assert the latest schema version after `run_migrations`.
- Assert required tables exist in explicit groups:
  - PMS core tables;
  - command safety tables;
  - experimental gateway tables;
  - experimental agent tables.
- Use existing migration test helpers where possible.

## Non-Goals

- Do not move inline migrations out of `db.rs`.
- Do not create a new migration module.
- Do not change production schema SQL.
- Do not rewrite existing version-specific migration tests.
- Do not add command executor, gateway, agent, or UI behavior.
- Do not expand validation beyond the issue #136 commands.

## Context

`run_migrations` currently lives in `mhm/src-tauri/src/db.rs` and owns inline migrations through schema version 19. GitNexus impact analysis for changing `run_migrations` returned CRITICAL risk: 51 direct callers, 223 impacted symbols, 9 affected execution flows, and 20 affected modules. The guard should therefore call `run_migrations` as a consumer and avoid production changes.

Existing tests already validate many version-specific details such as command idempotency columns, outbox shape, money column types, and agent/digest schema details. Issue #136 needs a broader fresh-database guard that is easier to read before migration extraction work begins.

## Chosen Approach

Use a grouped table-contract guard inside the existing `db.rs` test module.

The new test should be named:

```text
fresh_database_migration_creates_required_table_groups
```

The test will:

1. create an in-memory SQLite pool;
2. run `run_migrations(&pool)`;
3. read `schema_version.version`;
4. assert the version is `19`;
5. assert all tables in each required group exist.

Rejected alternatives:

- Create a new `migration_tests.rs` module. This would be cleaner later, but issue #136 exists specifically before migration movement, so a new test module is unnecessary boundary churn.
- Snapshot every table in the database. This would catch more changes but would be brittle for legitimate future additions.
- Assert grouped tables plus key indexes. Existing version-specific tests already cover command and outbox indexes, so repeating those checks here would duplicate maintenance surface.

## Table Groups

The test module should define local constants for the table groups.

`PMS_CORE_TABLES`:

```text
rooms
guests
bookings
booking_guests
transactions
expenses
housekeeping
settings
users
audit_logs
pricing_rules
special_dates
folio_lines
night_audit_logs
room_types
room_calendar
invoices
booking_groups
group_services
```

`COMMAND_SAFETY_TABLES`:

```text
command_idempotency
command_recovery_actions
outbox_events
```

`EXPERIMENTAL_GATEWAY_TABLES`:

```text
gateway_api_keys
```

`EXPERIMENTAL_AGENT_TABLES`:

```text
agent_sessions
agent_audit_events
agent_memory_items
agent_digest_runs
```

Experimental gateway and agent tables are asserted separately from PMS core tables so the test classifies those surfaces without implying they are core PMS state.

## Error Handling

Failures should point to the broken contract. Each table assertion should include the group and table name, such as:

```text
missing PMS core table rooms
```

The test should rely on the existing `table_exists` helper, which reads `sqlite_master`. No new production error handling is needed.

## Validation

Implementation validation for issue #136:

```bash
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test migration
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test
```

Before committing implementation changes, run GitNexus change detection:

```text
detect_changes(scope: "all", repo: "HotelManager")
```

## Risks

The implementation risk is low if the change remains test-only. The main risks are:

- accidentally changing production migration SQL while adding the guard;
- making the guard too broad by snapshotting every current table;
- making it too weak by omitting command safety tables;
- failing to classify experimental agent/gateway tables separately.

Keep the implementation narrow: one grouped fresh-database regression test and no migration movement.
