# Verification Suite Wave 1 Implementation Report

## Executive Summary

This change set implements the first shippable verification pipeline for CapyInn.

The goal of Wave 1 is to replace most manual "run `npm run tauri dev` and click around" validation with repeatable local commands and a CI job that exercise the highest-value hotel workflows without relying on OCR or full native UI automation.

Wave 1 now provides:

- isolated test runtime state under `~/CapyInn-TestSuite`
- deterministic runtime toggles for gateway, watcher, and time-sensitive flows
- fast local verification entry points
- a full local verification command
- a repeat command to prove stability
- a narrow native smoke test for Tauri startup under the isolated runtime
- a GitHub Actions verification job with artifact upload on failure

## Scope

Included in Wave 1:

- onboarding bootstrap verification
- login bootstrap readiness verification
- update-flow and settings-flow frontend verification
- existing frontend mocked app-flow suite
- existing Rust booking and night-audit business scenarios
- backup verification
- native Tauri smoke boot verification
- CI integration for the verification command

Explicitly excluded from Wave 1:

- OCR verification
- restore-flow coverage beyond backup smoke
- broad crash-recovery workflow coverage
- broad updater lifecycle coverage beyond existing app/update flow checks
- full native desktop automation of all hotel workflows

## What Changed

### 1. Runtime Isolation And Determinism

Added a dedicated runtime configuration module at:

- `mhm/src-tauri/src/runtime_config.rs`

This module introduces:

- `CAPYINN_RUNTIME_ROOT`
- `CAPYINN_DISABLE_GATEWAY`
- `CAPYINN_DISABLE_WATCHER`
- `CAPYINN_TEST_NOW`
- `CAPYINN_SMOKE_READY_FILE`

This allows verification runs to execute against `~/CapyInn-TestSuite` instead of the real `~/CapyInn` runtime directory and allows background services to be disabled or frozen for deterministic runs.

### 2. Runtime Root Override

Updated:

- `mhm/src-tauri/src/app_identity.rs`

`runtime_root_opt()` now prefers `CAPYINN_RUNTIME_ROOT` and falls back to the normal user runtime directory only when the override is absent.

This change ensures database, diagnostics, exports, models, and backup behavior can be redirected into an isolated runtime root during automated verification.

### 3. Native Harness Controls In App Startup

Updated:

- `mhm/src-tauri/src/lib.rs`

Changes:

- gateway startup can be disabled by `CAPYINN_DISABLE_GATEWAY`
- watcher startup can be disabled by `CAPYINN_DISABLE_WATCHER`
- updater enablement is explicitly gated by `CAPYINN_ENABLE_UPDATER`
- a smoke-ready payload can be written when `CAPYINN_SMOKE_READY_FILE` is set

The smoke-ready file is the handshake used by the native smoke script to confirm that the Tauri app booted successfully under the isolated runtime root.

### 4. Deterministic Backup Timestamps

Updated:

- `mhm/src-tauri/src/backup.rs`

Added `backup_timestamp_now()` so backup naming can use `CAPYINN_TEST_NOW` during tests instead of wall-clock time.

This keeps backup-related assertions stable across repeated runs.

### 5. Baseline Frontend Test Stabilization

Updated:

- `mhm/vitest.config.ts`
- `mhm/src/App.updateFlow.test.tsx`
- `mhm/tests/e2e/08-settings.test.tsx`

Changes:

- test version metadata now comes from `package.json` instead of hardcoded values
- update flow tests no longer depend on fragile mocked updater plugin state
- app-level update behavior is modeled through a local mocked `useAppUpdateController`
- settings update tests use the same deterministic controller model

This was required because the repo already had an existing mocked app-flow suite, but the update-related tests were not green in the current runtime/configuration reality.

### 6. New Verification Entry Points

Added:

- `mhm/scripts/verify/shared.mjs`
- `mhm/scripts/verify/quick.mjs`
- `mhm/scripts/verify/full.mjs`
- `mhm/scripts/verify/repeat.mjs`
- `mhm/scripts/verify/native-smoke.mjs`

Updated:

- `mhm/package.json`

New commands:

- `npm run verify:quick`
- `npm run verify:full`
- `npm run verify:repeat -- <n>`

### 7. CI Expansion

Updated:

- `.github/workflows/ci.yml`

Added a `verify-wave1` job that:

- installs dependencies
- runs `npm run verify:full`
- resolves artifact root via `$HOME/CapyInn-TestSuite`
- uploads verification artifacts on failure

## How The Suite Works

### `verify:quick`

Purpose:

- fast feedback before pushing or before a full verification run

Runs:

- targeted frontend update/settings regression tests
- runtime config Rust tests
- runtime root Rust tests
- setup/bootstrap Rust tests

### `verify:full`

Purpose:

- full local Wave 1 verification

Sequence:

1. reset `~/CapyInn-TestSuite`
2. run `verify:quick`
3. run the full frontend mocked app-flow suite
4. run the Rust booking scenario suite
5. run the Rust backup suite
6. run the native Tauri smoke test

### `verify:repeat`

Purpose:

- stability proving

Behavior:

- loops `verify:full` for the requested number of iterations

### Native Smoke Mechanism

`native-smoke.mjs` launches:

- `npm run tauri -- dev --no-watch`

Environment:

- isolated runtime root
- watcher disabled
- gateway disabled
- updater disabled
- smoke-ready file path configured

Pass condition:

- the app writes a JSON readiness payload under `~/CapyInn-TestSuite/artifacts/smoke-ready.json`
- payload contains `status = "ready"`
- payload runtime root matches the isolated test runtime root

## Coverage Summary

The Wave 1 suite currently verifies these categories.

### Frontend App-Flow Coverage

Covered by existing Vitest + Testing Library + mocked Tauri APIs:

- onboarding shell gating
- login shell gating
- settings flows
- update flow UI state and restart prompt behavior
- dashboard, navigation, reservations, check-in, checkout, housekeeping, guests, analytics, night audit, and other existing mocked flows already present in the repo

Current full-suite frontend result during verification:

- `32` test files passed
- `138` tests passed

### Rust Business Coverage

Covered by existing Rust suites plus one new bootstrap assertion:

- reservation creation
- reservation confirmation
- check-in
- checkout settlement
- group booking and group checkout behavior already present in the booking suite
- night audit
- revenue and export invariants
- backup behavior
- setup/bootstrap persistence behavior

Current Rust results during verification:

- booking suite: `48` tests passed
- backup suite: `12` tests passed
- setup/runtime targeted quick-suite tests passed

### Native Runtime Coverage

Covered by the narrow smoke test:

- Tauri boot under isolated runtime root
- database initialization under isolated runtime root
- successful startup without gateway/watcher
- readiness handshake artifact creation

This is intentionally a smoke layer, not a full desktop end-to-end business automation layer.

## Review Findings Addressed

The implementation fixes the three audit findings that blocked execution-readiness of the plan.

### Finding 1: `verify:repeat` Crash

Issue:

- `repeat.mjs` called `run(...)` without importing it

Fix:

- `repeat.mjs` now imports `run` from `./shared.mjs`

Validation:

- `npm run verify:repeat -- 5` completed successfully

### Finding 2: CI Artifact Path Expansion

Issue:

- GitHub Actions artifact upload does not expand `~`

Fix:

- the workflow now resolves `CAPYINN_ARTIFACT_ROOT=$HOME/CapyInn-TestSuite` into `GITHUB_ENV`
- `actions/upload-artifact` uses the resolved environment variable

### Finding 3: Env-Mutating Rust Test Races

Issue:

- Rust tests mutating process env are race-prone under parallel cargo test execution

Fix:

- `runtime_config.rs` exposes a shared test-only env mutex
- env-mutating tests in runtime/identity/backup test paths now serialize env access with that lock

## Local Verification Evidence

Commands executed successfully during implementation:

- `npm run verify:quick`
- `npm test`
- `cargo test --manifest-path src-tauri/Cargo.toml services::booking::tests:: -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml backup::tests:: -- --nocapture`
- `node ./scripts/verify/native-smoke.mjs`
- `npm run verify:full`
- `npm run verify:repeat -- 5`

Stability result:

- on the clean PR worktree, `verify:repeat -- 5` passed `5/5`

This is the strongest local evidence collected for Wave 1 stability on the exact branch prepared for review.

## Known Non-Blocking Caveats

### Recharts jsdom Warnings

The frontend suite still emits non-failing warnings such as:

- chart width or height reported as `0` or `-1` in jsdom

These are noisy but did not cause test failures across repeated runs.

### Native Smoke Teardown Noise

During native smoke teardown, Vite may log esbuild messages such as:

- `The service was stopped`

This occurs during process-tree shutdown after the smoke-ready signal is already captured.

Observed behavior:

- the smoke script still exits successfully
- `verify:full` still passes
- `verify:repeat -- 5` still passes

This is currently treated as teardown noise, not a failing verification condition.

### First-Run Compile Cost In A Clean Worktree

The first `verify:full` run in a fresh clean worktree can take significantly longer than repeated runs because the Rust build recompiles transitive dependencies from scratch.

In practice, the largest one-time compile cost came from the existing `ocr-rs` dependency tree even though OCR verification is out of scope for Wave 1.

Observed behavior:

- first clean-worktree run spent several minutes in Rust compilation
- repeated runs were materially faster
- this affects cold-start latency, not steady-state verification correctness

## Files Added Or Changed

Primary implementation files:

- `.github/workflows/ci.yml`
- `mhm/package.json`
- `mhm/vitest.config.ts`
- `mhm/src/App.updateFlow.test.tsx`
- `mhm/tests/e2e/08-settings.test.tsx`
- `mhm/src-tauri/src/runtime_config.rs`
- `mhm/src-tauri/src/app_identity.rs`
- `mhm/src-tauri/src/lib.rs`
- `mhm/src-tauri/src/backup.rs`
- `mhm/src-tauri/src/services/setup/tests.rs`
- `mhm/scripts/verify/shared.mjs`
- `mhm/scripts/verify/quick.mjs`
- `mhm/scripts/verify/full.mjs`
- `mhm/scripts/verify/repeat.mjs`
- `mhm/scripts/verify/native-smoke.mjs`

Related design documents in the repo:

- `docs/superpowers/specs/2026-04-20-verification-suite-design.md`
- `docs/superpowers/plans/2026-04-21-verification-suite-wave1.md`

## Audit Position

Based on implementation and repeated local execution, the current Wave 1 suite is:

- implementation-complete for the selected Wave 1 scope
- locally stable across repeated runs
- ready for third-party review
- ready for GitHub-hosted CI validation on the matching branch line

The next meaningful gate is CI confirmation on the new `verify-wave1` job, not more local redesign work.
