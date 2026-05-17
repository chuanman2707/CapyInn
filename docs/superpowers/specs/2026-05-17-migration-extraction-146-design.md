# Migration Extraction Follow-Up Design

Issue: #146 ARCH-13 Batch 3: Move command-safety and experimental migrations in follow-up batches

Parent roadmap: #133 Core PMS Architecture Stabilization V2.1

Date: 2026-05-17
Status: User-approved design; written spec pending review

## Purpose

Move the remaining inline migration bodies `V7` through `V19` out of `mhm/src-tauri/src/db.rs` without changing database behavior.

This is a no-behavior-change extraction. It must make `db.rs` a migration runner and database bootstrap module instead of a migration monolith, while preserving schema semantics, migration order, compatibility behavior, command safety behavior, money migration behavior, outbox behavior, gateway runtime behavior, agent runtime behavior, and frontend behavior.

## Scope

- Keep database bootstrap and the main migration runner in `mhm/src-tauri/src/db.rs`.
- Keep existing helper functions in `db.rs`, including `set_schema_version`, `execute_compat_alter`, and `restore_foreign_keys_after_v14_migration`.
- Move remaining migration bodies `V7` through `V19` into private modules under `mhm/src-tauri/src/db/`.
- Keep the existing `V1` through `V6` extraction in `mhm/src-tauri/src/db/migrations.rs`.
- Keep the final schema version at `19`.
- Keep existing migration tests and fresh database guards intact.

`V7` through `V19` cover:

- `V7`: gateway API key storage
- `V8`: invoice PDF system
- `V9`: group booking system
- `V10`: command idempotency
- `V11`: command terminal error replay payload
- `V12`: operator-ready command ledger metadata
- `V13`: origin idempotency on ledger and folio rows
- `V14`: integer VND money foundation
- `V15`: command recovery queue and audit actions
- `V16`: durable outbox events
- `V17`: outbox per-aggregate open-row FIFO support
- `V18`: agent safety session, audit, and memory schema
- `V19`: CEO hourly digest run state

## Non-Goals

- Do not change SQL semantics.
- Do not reorder migrations.
- Do not combine migrations.
- Do not change the final schema version.
- Do not change command idempotency logic.
- Do not change outbox dispatcher behavior.
- Do not change gateway, agent, digest, Telegram, OpenAI, or frontend runtime behavior.
- Do not gate or delete experimental schemas.
- Do not introduce a new migration framework or registry abstraction.
- Do not move or refactor unrelated backend modules.

## Context

Issues #144 and #145 introduced the migration module structure and moved `V1` through `V6` into `mhm/src-tauri/src/db/migrations.rs`. The remaining migrations `V7` through `V19` still live inline in `run_migrations`.

Issue #146 intentionally has higher risk than the first extraction because the remaining migrations include command-safety, money, outbox, gateway, agent, and digest schema. Those schemas are safety-sensitive even when the runtime surfaces are experimental.

GitNexus impact analysis for `run_migrations` returned CRITICAL risk: 53 direct callers, 8 affected execution flows, and 20 affected modules. The affected surface includes app startup, database migration tests, command idempotency tests, outbox tests, gateway tests, agent tests, digest tests, setup tests, and booking tests.

GitNexus impact analysis for `set_schema_version` and `execute_compat_alter` also returned CRITICAL risk. This extraction may widen visibility only as needed for private child migration modules, but it must not change helper behavior.

## Chosen Approach

Move all remaining inline migrations `V7` through `V19` in one issue, but split them into domain-focused private modules so the high-risk areas remain reviewable.

Use this module shape:

```text
mhm/src-tauri/src/db.rs
mhm/src-tauri/src/db/
  migrations.rs
  core_extensions.rs
  command_safety.rs
  outbox.rs
  agent.rs
```

Module ownership:

- `migrations.rs` keeps the existing `V1` through `V6` early PMS migrations.
- `core_extensions.rs` owns `V7` through `V9`.
- `command_safety.rs` owns `V10` through `V15`.
- `outbox.rs` owns `V16` and `V17`.
- `agent.rs` owns `V18` and `V19`.

Rejected alternatives:

- Keep everything in a single growing `migrations.rs`. This reduces `db.rs` but recreates a migration monolith.
- Create one file per migration version. This gives maximum isolation, but creates more file churn than the current migration count needs.
- Split #146 into multiple issues. This is safest per PR, but the chosen scope is to move all remaining migrations while preserving review boundaries inside the PR.

## Module Interfaces

Each new module should expose only parent-module-internal functions needed by `run_migrations`.

`core_extensions.rs`:

```rust
pub(super) async fn migrate_v7_gateway_api_keys(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v8_invoice_pdf_system(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v9_group_booking_system(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
```

`command_safety.rs`:

```rust
pub(super) async fn migrate_v10_command_idempotency(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v11_command_terminal_error_replay(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v12_command_ledger_metadata(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v13_origin_idempotency(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v14_integer_vnd_money(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v15_command_recovery(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
```

`outbox.rs`:

```rust
pub(super) async fn migrate_v16_durable_outbox_events(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v17_outbox_fifo_support(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
```

`agent.rs`:

```rust
pub(super) async fn migrate_v18_agent_safety_tables(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v19_agent_digest_runs(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
```

The modules should reuse shared helpers from `db.rs` instead of duplicating compatibility or schema-version logic.

## Data Flow

The runtime flow remains unchanged:

1. `init_db` creates the SQLite pool.
2. `init_db` calls `run_migrations`.
3. `run_migrations` reads the current schema version.
4. `run_migrations` checks each version gate in order.
5. For `V1` through `V19`, `run_migrations` delegates to private migration module functions.
6. `init_db` inserts default settings after migrations complete.

`run_migrations` remains the only place that decides migration order. The extracted modules own only migration bodies.

The `V14` flow is the existing exception and must remain behaviorally identical:

1. Acquire a connection.
2. Disable foreign keys.
3. Run the migration transaction callback.
4. Add `legacy_request_hash` through `execute_compat_alter`.
5. Call `crate::money_migration::migrate_integer_vnd_money`.
6. Set schema version `14`.
7. Restore foreign key behavior through `restore_foreign_keys_after_v14_migration`.

## Error Handling

Error behavior must stay unchanged.

Each extracted migration returns `Result<(), sqlx::Error>` and uses `?` exactly like the current inline code. If a query fails, the transaction is not committed and the error bubbles back to `run_migrations`.

`execute_compat_alter` keeps its current behavior: duplicate column and already-exists errors are logged and ignored; other errors fail the migration.

`V14` keeps the existing foreign-key restore path. The extraction must not add a new recovery path, repair path, fallback path, migration skip path, or best-effort schema creation path.

## Testing

Validation commands:

```bash
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test db::tests
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test migration
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo clippy --all-targets -- -D warnings
```

Expected coverage:

- fresh database migration still reaches schema version `19`;
- required PMS, core extension, command safety, outbox, and agent tables still exist;
- existing database upgrade tests for `V10` through `V19` still pass;
- V14 money conversion and rollback tests still pass;
- V16 and V17 outbox schema, index, and insert contract tests still pass;
- V18 and V19 agent and digest schema tests still pass;
- compatibility alters still ignore duplicate columns where intended;
- later runtime tests that create migrated test pools still pass.

Before committing implementation changes, run GitNexus change detection. The expected affected scope should be limited to `db.rs`, private `db/` migration module files, and migration execution flows.

## Review Guardrails

Implementation should be a mechanical move:

- preserve SQL strings exactly;
- preserve comments where they identify migration versions or existing behavior;
- preserve transaction boundaries;
- preserve version numbers;
- preserve call order;
- preserve V14 foreign-key handling;
- avoid unrelated formatting churn in untouched code;
- do not edit command idempotency, outbox, gateway, agent, digest, frontend, or service behavior;
- do not edit the unrelated dirty file `mhm/src/stores/useHotelStore.test.ts`.

Before editing implementation symbols, run GitNexus impact analysis for each edited function or helper. At minimum, run impact analysis for `run_migrations`, `set_schema_version`, `execute_compat_alter`, and `restore_foreign_keys_after_v14_migration` if their bodies or visibility are modified. `run_migrations`, `set_schema_version`, and `execute_compat_alter` are already known CRITICAL risk; implementation should report that blast radius before editing and continue only with the approved mechanical extraction scope.
