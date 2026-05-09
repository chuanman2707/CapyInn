# Cleanup Baseline Report: Baseline CI Test Guard

- Date: 2026-05-09
- Issue: #134
- Parent issue: #133
- Scope: baseline CI/test guard evidence for cleanup work

## Purpose

This report records the known-good local baseline for issue #134 before cleanup work continues under parent issue #133. The baseline is intended to prevent cleanup changes from weakening the existing CI/test gate or introducing undocumented failing commands.

## Command Results

| Command | Working directory | Status | Evidence | Transient local log |
| --- | --- | --- | --- | --- |
| `npm test` | `mhm` | PASS | `Test Files 52 passed (52); Tests 255 passed (255)` | `/tmp/capyinn-134-baseline/npm-test.log` |
| `npm run build` | `mhm` | PASS | Vite built successfully in 3.81s; large chunk warning only | `/tmp/capyinn-134-baseline/npm-build.log` |
| `cargo check` | `mhm/src-tauri` | PASS | `Finished dev profile ... in 15.14s` | `/tmp/capyinn-134-baseline/cargo-check.log` |
| `cargo test` | `mhm/src-tauri` | PASS | `test result: ok. 701 passed; 0 failed`; bin and doc tests also passed | `/tmp/capyinn-134-baseline/cargo-test.log` |
| `cargo clippy --all-targets -- -D warnings` | `mhm/src-tauri` | PASS | `Finished dev profile ... in 14.31s` | `/tmp/capyinn-134-baseline/cargo-clippy.log` |

The `/tmp/capyinn-134-baseline/*` logs were transient local capture artifacts used to record the baseline evidence above. They are not committed.

## Known Failing Baseline

There are no known failing baseline commands as of 2026-05-09. All baseline commands listed above passed.

## CI Parity

The main baseline gate for issue #134 is the `build-test` job in `.github/workflows/ci.yml`, which runs on `macos-latest`.

Exact baseline command mapping:

| Local command | Local working directory | CI job | CI step | CI working directory |
| --- | --- | --- | --- | --- |
| `npm test` | `mhm` | `build-test` | `Run frontend tests` | `mhm` |
| `npm run build` | `mhm` | `build-test` | `Build frontend` | `mhm` |
| `cargo check` | `mhm/src-tauri` | `build-test` | `Run cargo check` | `mhm/src-tauri` |
| `cargo test` | `mhm/src-tauri` | `build-test` | `Run cargo test` | `mhm/src-tauri` |
| `cargo clippy --all-targets -- -D warnings` | `mhm/src-tauri` | `build-test` | `Run clippy` | `mhm/src-tauri` |

`cargo check` is included for CI parity even though the issue #134 minimum baseline names frontend tests, frontend build, Rust tests, and clippy.

`.github/workflows/ci.yml` also includes `verify-wave1`, which runs `npm run verify:full` from `mhm` after `build-test`. That job is additional isolated Wave 1 verification, not the main #134 baseline gate.

README.md and CONTRIBUTING.md both mention the baseline command family.

## Experimental Service Requirement

The #134 CI workflow in `.github/workflows/ci.yml` consists of `build-test` and `verify-wave1`. Neither job requires Telegram, OpenAI, MCP, gateway, live-network credentials, or experimental service configuration.

`verify-wave1` only adds the isolated Wave 1 verification job: it sets `CAPYINN_ARTIFACT_ROOT=$HOME/CapyInn-TestSuite` and runs `npm run verify:full` from `mhm`. The verification runner uses the isolated runtime root and disables gateway and watcher processes through `CAPYINN_DISABLE_GATEWAY=true` and `CAPYINN_DISABLE_WATCHER=true`.

This statement is scoped to the #134 CI workflow only. It is not a claim about release workflows.

## Cleanup Rule

Cleanup work under parent issue #133 must preserve this baseline gate. If a cleanup change causes any listed command, mapped `build-test` CI step, or `verify-wave1` step to fail, the cleanup change must be fixed or reverted before proceeding. Only failures reproduced on the unchanged baseline may be documented as pre-existing, and they must include concrete evidence and owner follow-up.
