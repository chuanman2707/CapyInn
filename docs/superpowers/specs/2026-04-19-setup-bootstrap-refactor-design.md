# CapyInn Setup Bootstrap Refactor Design

Date: 2026-04-19
Owner: Codex
Status: Draft approved for spec write-up

## Goal

Refactor `mhm/src-tauri/src/commands/onboarding.rs` so it stops acting as a backend god file while preserving the current product-facing onboarding flow.

The refactor should make five things true:

- backend setup and bootstrap logic has a clear owner that is not a Tauri command module
- runtime bootstrap status and one-time setup provisioning stay part of one coherent capability
- `commands/onboarding.rs` becomes a thin compatibility adapter instead of the place where business logic lives
- the current frontend onboarding flow and public Tauri command names keep working during this pass
- tests move closer to the logic they validate, with app-state side effects isolated from database rules

This design is intentionally a backend architecture refactor. It does not add new onboarding steps, new product behavior, or a new public API in this pass.

## User Decisions Locked In

The following decisions were explicitly chosen during brainstorming and are part of this spec:

- priority: fix architectural boundaries over readability-only cleanup
- capability naming: backend logic should conceptually belong to setup/bootstrap rather than to onboarding UI
- scope of change: command and model contracts could change in principle, but this pass should avoid unnecessary frontend churn
- one-time setup and runtime bootstrap status remain part of the same capability
- shared DTO strategy: keep `mhm/src-tauri/src/models.rs` as the shared DTO bucket in this pass
- frontend naming: keep existing `onboarding` and `bootstrap` product-facing names in the UI and tests for now
- command compatibility: keep the public Tauri command names `get_bootstrap_status` and `complete_onboarding` in this pass

## Constraints

- `mhm/src-tauri/src/commands/onboarding.rs` is currently a single file of roughly 600 lines and contains:
  - bootstrap status reads
  - session hydration
  - onboarding validation
  - settings persistence
  - admin creation
  - room type, room, and pricing rule provisioning
  - Tauri command wrappers
  - inline tests
- the current file also depends on `crate::commands::settings::do_get_settings(...)`, which is a command-to-command dependency and the main boundary leak that this refactor must remove
- the frontend currently depends on only two command strings for this flow:
  - `get_bootstrap_status` in `mhm/src/App.tsx`
  - `complete_onboarding` in `mhm/src/pages/onboarding/index.tsx`
- `BootstrapStatus` and `Onboarding*` DTOs currently live in `mhm/src-tauri/src/models.rs` and are also mirrored on the frontend in `mhm/src/types/index.ts`
- the repo already has service-style backend decomposition under `mhm/src-tauri/src/services/booking/*`
- that existing service layout is shallow and capability-oriented; the repo does not currently use deep nested namespaces such as `services::<capability>::<subcapability>::*`
- this work should not introduce a repository layer just for symmetry unless the extracted persistence helpers are clearly reused outside setup/auth

## Chosen Approach

Create a new internal backend capability under `mhm/src-tauri/src/services/setup/*` and move all pure database and business rules there.

Keep `mhm/src-tauri/src/commands/onboarding.rs` as the Tauri-facing compatibility adapter that:

- owns the two existing public command names
- passes `State<AppState>` data into the setup service
- applies runtime session hydration through `current_user`

Why this approach:

- it fixes the real design problem, which is not just file length but wrong ownership of setup/bootstrap logic
- it preserves the current frontend and test contracts while cleaning the backend boundary
- it matches the repo's existing shallow `services::<capability>` pattern better than introducing `services/setup/bootstrap/*`
- it keeps app-state mutation in the command layer, which is where similar Tauri-specific side effects already live
- it avoids over-designing a one-pass refactor with unnecessary DTO churn or synthetic repository abstractions

## Non-Goals

- renaming the public Tauri command names in this pass
- changing the frontend onboarding flow or onboarding page naming
- splitting DTOs into transport, domain, and persistence layers in this pass
- moving bootstrap session hydration into a generic auth/session subsystem
- introducing a full repository layer for settings, users, rooms, or pricing just for this refactor
- adding new onboarding fields, setup steps, or pricing behavior
- changing the validation rules unless required to preserve current correctness

## Existing State

Today `mhm/src-tauri/src/commands/onboarding.rs` owns nearly the whole lifecycle of initial setup and runtime bootstrap:

- it decides whether setup is complete
- it reads app-lock state
- it loads the default current user for unlocked mode
- it validates the onboarding request
- it opens the SQL transaction and clears seedable tables
- it writes settings
- it provisions the owner/admin user
- it provisions room types, rooms, and pricing rules
- it mutates `AppState.current_user`
- it exposes the Tauri commands
- it hosts the tests

This creates three concrete problems:

1. The capability is owned by the wrong layer. A Tauri command module currently acts as both adapter and service.
2. The file depends on another command module through `commands::settings::do_get_settings`, which means the boundary leak remains even if the file is split mechanically.
3. Runtime app-state mutation, validation, persistence, and product bootstrap policy all sit in one place, making future maintenance riskier than necessary.

The current flow works functionally. The problem is architectural ownership and responsibility sprawl.

## Target Structure

The refactor should move setup/bootstrap logic into a dedicated shallow service module:

```text
mhm/src-tauri/src/services/
  mod.rs
  settings_store.rs
  setup/
    mod.rs
    status.rs
    provisioning.rs
    validation.rs   # only if provisioning.rs is still too large
    tests.rs
```

`commands/onboarding.rs` remains in place, but shrinks to a compatibility adapter.

## Module Boundaries

### 1. `services/setup/mod.rs`

This module is the public internal facade for the setup capability.

It should:

- declare child modules
- expose a small setup-facing service API
- keep the rest of the app from importing `status.rs` and `provisioning.rs` ad hoc

Expected public internal service surface:

- `read_bootstrap_status(pool: &Pool<Sqlite>) -> Result<BootstrapStatus, String>`
- `complete_setup(pool: &Pool<Sqlite>, req: OnboardingCompleteRequest) -> Result<BootstrapStatus, String>`

### 2. `services/setup/status.rs`

This module owns the read path for runtime bootstrap status.

It should contain:

- reading `setup_completed`
- reading `app_lock`
- reading `default_user_id`
- loading the current default user for unlocked mode
- building the final `BootstrapStatus`

It should not:

- mutate `AppState.current_user`
- expose Tauri commands
- depend on `commands::settings`

### 3. `services/setup/provisioning.rs`

This module owns the write path for initial setup completion.

It should contain:

- rejecting repeated setup after `setup_completed == true`
- validating the incoming request
- opening the SQL transaction
- deleting `pricing_rules`, `rooms`, `room_types`, and `users` in the locked provisioning order for this flow
- writing hotel and check-in settings
- writing app-lock settings
- creating the owner/admin user
- saving `default_user_id`
- inserting room types, rooms, and pricing rules
- marking `setup_completed`
- returning the final `BootstrapStatus`

It should not:

- know about `State<AppState>`
- mutate `current_user`
- expose Tauri commands

### 4. `services/setup/validation.rs`

Do not create this module by default.

Create it only if the extracted `provisioning.rs` still remains difficult to review because pure validation logic materially obscures the transaction flow after the main move.

If created, it should own:

- structural validation of hotel info
- room type and room uniqueness checks
- app-lock validation rules
- any pure helper functions such as time parsing

If `provisioning.rs` remains readable without this split, keep validation private there and do not add the file.

### 5. `services/settings_store.rs`

This module is the neutral settings access helper introduced by this refactor.

It should own the low-level settings reads and writes that are currently spread across command helpers and direct SQL call sites.

It should contain:

- `get_setting(pool, key)` style helpers for raw string values
- `save_setting(tx_or_pool, key, value)` style helpers where needed
- small JSON convenience helpers only if they reduce repeated parsing without hiding storage semantics

It should be used directly by:

- `services/setup/*`
- `commands/invoices.rs`
- `gateway/tools.rs`

`commands/settings.rs` remains the Tauri adapter for user-driven settings CRUD, but it should delegate to this neutral helper rather than owning the canonical non-command helper itself.

### 6. `commands/onboarding.rs`

This file remains the adapter layer.

It should contain:

- `#[tauri::command] get_bootstrap_status(...)`
- `#[tauri::command] complete_onboarding(...)`
- `sync_bootstrap_session(...)`

Its responsibilities are limited to:

- receiving `State<AppState>`
- calling the new setup service
- mutating `current_user` after a status result is returned
- preserving the existing public command names

It should not contain:

- SQL persistence logic
- validation logic
- settings read/write helpers
- room/pricing provisioning helpers

## Command Surface

This refactor intentionally keeps the current public Tauri command names unchanged:

- `get_bootstrap_status`
- `complete_onboarding`

Reason:

- the frontend and test blast radius is currently wider than the value of a rename
- renaming command strings does not improve backend boundaries
- keeping compatibility lets this pass focus on ownership and layering

Internal naming should still move toward setup-oriented language. The compatibility adapter is allowed to translate between old command names and new internal service naming.

## Settings Access Rule

This is a hard design rule for the refactor:

- the new setup service must not depend on `crate::commands::settings::do_get_settings(...)`

Unacceptable solution:

- moving code into `services/setup/*` while continuing to call `commands::settings::*`

If that dependency survives, the refactor has not actually fixed the main architectural issue.

This pass explicitly introduces a shared non-command settings helper rather than allowing setup-private duplication.

Reason:

- settings access is already reused outside onboarding by invoices and gateway tools
- setup should not fix its own boundary by inventing a second private settings access path
- the repo is cleaner if non-command logic stops depending on `commands::settings::*`

## Persisted Settings Contract

This refactor must preserve the runtime storage contract, not just the DTO shape returned by Tauri commands.

The following keys and value semantics are locked for this pass:

- `setup_completed`
  - stored in `settings.value` as the literal string `"true"` or `"false"`
  - bootstrap reads and gateway auth enforcement continue to treat this as a string flag
- `default_user_id`
  - stored as the user id string for the owner/admin created during setup
- `app_locale`
  - stored as the locale string coming from onboarding
- `app_lock`
  - stored as JSON with at least `{ "enabled": <bool> }`
  - bootstrap status continues to derive `app_lock_enabled` from `enabled`
- `hotel_info`
  - stored as JSON with keys `{ "name", "address", "phone", "rating" }`
  - this shape is consumed by the settings page, invoice generation, group flows, and gateway tools
- `checkin_rules`
  - the canonical stored JSON shape after this refactor is `{ "checkin": "HH:MM", "checkout": "HH:MM" }`
  - this intentionally aligns setup provisioning with the existing settings UI contract
  - preserving the old onboarding-only shape `{ "default_checkin_time", "default_checkout_time" }` is not a goal of this refactor

This `checkin_rules` normalization is the one allowed storage-contract cleanup in the pass because the current repo already has an inconsistency between onboarding writes and settings-page reads.

## Data Contract Strategy

Keep `mhm/src-tauri/src/models.rs` unchanged in this pass.

This means:

- `BootstrapStatus` stays where it is
- `OnboardingCompleteRequest` and related DTOs stay where they are
- frontend mirrored types stay stable in this pass

Trade-off:

- internal setup service naming will temporarily be cleaner than the DTO naming
- that naming mismatch is acceptable because it avoids high-churn edits with low architectural payoff

## Migration Plan

1. Add `services::setup` to `mhm/src-tauri/src/services/mod.rs`.
2. Extract bootstrap status read logic into `services/setup/status.rs`.
3. Extract setup completion and provisioning logic into `services/setup/provisioning.rs`.
4. Introduce `services/settings_store.rs` and migrate setup logic away from `commands::settings`.
5. Repoint `commands/invoices.rs` and `gateway/tools.rs` to the neutral settings helper so non-command logic no longer imports `commands::settings::do_get_settings`.
6. If needed, extract pure validation helpers into `services/setup/validation.rs`.
7. Update `commands/onboarding.rs` so it only delegates to setup services and performs `sync_bootstrap_session`.
8. Move DB-heavy setup tests into `services/setup/tests.rs`, leaving only app-state session tests in `commands/onboarding.rs`.

This migration is complete only when `commands/onboarding.rs` becomes a thin adapter and no setup business rules remain there.

## Provisioning Sequence

This refactor preserves the existing provisioning transaction semantics except for the intentional `checkin_rules` JSON shape cleanup described above.

Inside the setup completion transaction, the sequence is:

1. delete from `pricing_rules`
2. delete from `rooms`
3. delete from `room_types`
4. delete from `users`
5. upsert `hotel_info`
6. upsert `checkin_rules`
7. upsert `app_lock`
8. upsert `app_locale`
9. insert the owner/admin user
10. upsert `default_user_id`
11. insert room types
12. insert rooms
13. insert pricing rules
14. set `setup_completed` to the string `"true"`
15. commit

`setup_completed` must remain the last persisted state flag written inside the successful provisioning path.

## Test Strategy

The test split should follow responsibility ownership.

### Command-layer tests

Keep only tests that validate Tauri/app-state behavior, primarily:

- `sync_bootstrap_session` hydrates `current_user` in unlocked mode
- `sync_bootstrap_session` clears `current_user` in locked mode

### Service-layer tests

Place DB-heavy setup tests in `services/setup/tests.rs` so the structure matches the repo's existing `services/booking/tests.rs` convention.

Move or add regression coverage for:

- incomplete setup returns `setup_completed = false`
- completed unlocked setup returns the default user in `BootstrapStatus`
- completed locked setup returns `current_user = None`
- setup does not seed demo data outside the request payload
- duplicate room type names are rejected
- duplicate room ids are rejected
- rooms referencing missing room types are rejected
- invalid app-lock admin name or PIN is rejected
- repeating setup after completion is rejected
- successful setup creates the expected settings, user, rooms, and pricing rules
- `setup_completed` remains a string-backed settings flag with `"true"` semantics after successful setup
- `hotel_info` is written with the exact `{ name, address, phone, rating }` JSON shape
- `checkin_rules` is written with the canonical `{ checkin, checkout }` JSON shape

### Test placement rule

- tests that require `AppState.current_user` stay with the command adapter
- tests that only require a database pool live in `services/setup/tests.rs`

## Risks And Mitigations

### Risk 1: boundary cleanup is partial

If setup code still depends on `commands::settings`, the refactor will look cleaner without actually fixing ownership.

Mitigation:

- treat removal of command-to-command dependency as a success criterion, not as an optional cleanup

### Risk 2: over-splitting creates a taxonomy the repo does not use elsewhere

If the refactor creates too many small files or deep nested namespaces, the new design will feel artificial compared to the rest of the codebase.

Mitigation:

- keep the structure shallow under `services/setup/*`
- add `validation.rs` only if it materially improves readability

### Risk 3: setup service starts mutating Tauri state

If session hydration moves into the service layer, the new boundary will be worse than the old one.

Mitigation:

- keep `sync_bootstrap_session` in `commands/onboarding.rs` or another adapter-level helper

## Acceptance Criteria

This refactor is considered complete when all of the following are true:

- `commands/onboarding.rs` is reduced to Tauri command wiring plus session sync
- all setup/bootstrap database and validation logic lives under `services/setup/*`
- setup, invoices, and gateway tools no longer depend on `commands::settings::do_get_settings`
- a neutral `services/settings_store.rs` helper owns shared non-command settings access
- the public Tauri command names remain `get_bootstrap_status` and `complete_onboarding`
- `models.rs` remains unchanged for this pass unless a narrow change is required for correctness
- the persisted settings contract remains compatible for:
  - `setup_completed`
  - `default_user_id`
  - `app_locale`
  - `app_lock`
  - `hotel_info`
  - `checkin_rules` in its canonical normalized shape
- the provisioning transaction preserves the locked delete/write sequence from this spec
- service-layer tests cover the main bootstrap status and setup provisioning regressions
- command-layer tests only cover app-state side effects

## Follow-Up Work

The following items are intentionally deferred:

- renaming public Tauri commands from onboarding-oriented names to setup-oriented names
- deciding whether runtime session hydration should later move into a broader auth/session capability
- extracting shared settings access if another capability starts needing the same non-command helper
- revisiting whether `BootstrapStatus` and `Onboarding*` DTOs should later move out of `models.rs`
