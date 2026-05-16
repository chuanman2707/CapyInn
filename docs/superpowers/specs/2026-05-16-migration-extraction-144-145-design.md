# Migration Extraction Setup Design

Issues: #144 ARCH-11 Batch 3: Introduce backend migration module structure; #145 ARCH-12 Batch 3: Move core PMS migrations in first small extraction batch

Parent roadmap: #133 Core PMS Architecture Stabilization V2.1

Date: 2026-05-16
Status: Approved for implementation planning

## Purpose

Introduce a backend migration module structure and use it to move the first early core PMS migration batch out of `mhm/src-tauri/src/db.rs`.

This change is a no-behavior-change extraction. It must make `db.rs` smaller and easier to review without changing schema semantics, migration order, compatibility behavior, command idempotency behavior, outbox behavior, gateway runtime behavior, agent runtime behavior, or frontend behavior.

## Scope

- Keep database bootstrap and the main migration runner in `mhm/src-tauri/src/db.rs`.
- Add a migration module under `mhm/src-tauri/src/db/`.
- Move migration bodies `V1` through `V6` into the new module.
- Keep migrations `V7` through `V19` in `db.rs` for this batch.
- Keep the existing migration tests and fresh database guard intact.

`V1` through `V6` cover the early PMS schema:

- `V1`: base schema
- `V2`: foundation and RBAC
- `V3`: pricing engine
- `V4`: folio, billing, and night audit
- `V5`: dynamic room config
- `V6`: reservation calendar block system

## Non-Goals

- Do not change SQL semantics.
- Do not reorder migrations.
- Do not combine migrations.
- Do not change the final schema version.
- Do not move gateway, command safety, money, outbox, agent, or digest migrations in this batch.
- Do not touch command idempotency logic.
- Do not introduce a new migration framework.
- Do not change app startup behavior.

## Context

`run_migrations` currently lives in `mhm/src-tauri/src/db.rs` and contains inline migrations through schema version `19`. The file is over 2,400 lines, and inline migration SQL begins near the top of `run_migrations`.

Issue #136 already added a fresh database migration guard. That test runs migrations, asserts schema version `19`, and checks required table groups for PMS core, command safety, experimental gateway, and experimental agent tables. This guard is the main protection before moving migration bodies.

GitNexus impact analysis for `run_migrations` returned CRITICAL risk: 53 direct callers, 8 affected execution flows, and 19 affected modules. The affected surface includes app startup and many backend tests. Because of that blast radius, this extraction must be mechanical and small.

## Chosen Approach

Create a `db` submodule for migrations and move only `V1` through `V6` into early core PMS migration functions.

The main runner should continue to read the current schema version once and evaluate version gates in the same order:

```rust
if current < 1 {
    migrations::migrate_v1_base_schema(pool).await?;
}

if current < 2 {
    migrations::migrate_v2_foundation_rbac(pool).await?;
}
```

The extracted functions should each own the same transaction boundary they own today inside `run_migrations`: begin transaction, run SQL, set schema version, commit.

Rejected alternatives:

- Move only `V1`. This is safest, but it gives too little value for issue #145.
- Move all migrations. This would make review too broad and would mix core PMS, gateway, command safety, money, outbox, and agent concerns.
- Introduce a generic migration registry. That adds abstraction before the project needs it and increases risk in a CRITICAL startup path.

## Module Shape

Use this shape:

```text
mhm/src-tauri/src/db.rs
mhm/src-tauri/src/db/
  migrations.rs
```

`db.rs` remains the public database module from the crate perspective and declares the child module with `mod migrations;`. The nested `db/` directory is private implementation detail for migration extraction.

`migrations.rs` should expose only parent-module-internal functions needed by `run_migrations`, such as:

```rust
pub(super) async fn migrate_v1_base_schema(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v2_foundation_rbac(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v3_pricing_engine(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v4_folio_billing_night_audit(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v5_dynamic_room_config(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
pub(super) async fn migrate_v6_reservation_calendar(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>;
```

The extracted module should reuse `set_schema_version` and `execute_compat_alter` rather than duplicating them.

## Data Flow

The runtime flow remains:

1. `init_db` creates the SQLite pool.
2. `init_db` calls `run_migrations`.
3. `run_migrations` reads the current schema version.
4. `run_migrations` checks each version gate in order.
5. For `V1` through `V6`, `run_migrations` delegates to the new migration module.
6. For `V7` through `V19`, `run_migrations` keeps the existing inline code.
7. `init_db` inserts default settings after migrations complete.

The only intended code movement is the body of the `current < 1` through `current < 6` blocks.

## Error Handling

Error behavior must stay unchanged.

Each extracted migration returns `Result<(), sqlx::Error>` and uses `?` exactly like the current inline code. If a query fails, the transaction is not committed and the error bubbles back to `run_migrations`.

`execute_compat_alter` keeps its current behavior: duplicate column or already-exists errors are logged and ignored; other errors fail the migration. The new migration module must call the existing helper rather than reimplementing compatibility handling.

No new recovery path, repair path, fallback, or migration skipping should be added.

## Testing

Validation commands:

```bash
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test db::tests
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo test
cd /Users/binhan/HotelManager/mhm/src-tauri && cargo clippy --all-targets -- -D warnings
```

Expected coverage:

- fresh database migration still reaches schema version `19`;
- required PMS core tables still exist;
- compatibility alters still ignore duplicate columns;
- later migrations `V7` through `V19` still run after the extracted batch;
- V14 money migration tests still pass;
- V16 outbox migration tests still pass;
- V18 and V19 agent/digest migration tests still pass;
- clippy reports no visibility or module warnings.

Before committing implementation changes, run GitNexus change detection:

```bash
gitnexus_detect_changes
```

The expected affected scope should be limited to `db.rs`, the new migration module files, and migration execution flows.

Before editing implementation symbols, run GitNexus impact analysis for each edited function or helper. At minimum, run impact analysis for `run_migrations`, `set_schema_version`, and `execute_compat_alter` if those symbols are modified or their visibility changes. `run_migrations` is already known to be CRITICAL risk; implementation should report that blast radius before editing and continue only with the approved mechanical extraction scope.

## Review Guardrails

Implementation should use a mechanical move:

- preserve SQL strings exactly;
- preserve comments where they help identify the migration version;
- preserve transaction boundaries;
- preserve version numbers;
- preserve call order;
- avoid unrelated formatting churn in untouched migrations;
- do not edit frontend or command safety files.

The existing dirty file `mhm/src/stores/useHotelStore.test.ts` is unrelated to this work and must not be included in commits for this extraction.
