# CapyInn Verification Suite Design

## Summary

CapyInn will add a local-first verification pipeline that replaces most manual `npm run tauri dev` checks with a layered automated suite.

The selected approach combines:

- expansion of the existing repository test and CI baseline rather than a greenfield verification system
- a dedicated runtime root at `~/CapyInn-TestSuite` for all automated runs
- a layered `verify` pipeline covering business logic, app flows, and native smoke
- expansion of the existing GitHub-hosted Actions CI after local stability is proven

This gives the project a realistic path from "one maintainer manually clicking through the app" to "one command verifies the main hotel workflows before PRs."

## Goals

- Replace most manual desktop verification with one repeatable test command.
- Cover the core hotel workflows end to end: onboarding, login, reservations, check-in, checkout, housekeeping, guests, settings, analytics, night audit, backup, update flow, and crash-reporting flow.
- Keep local full-suite runtime within roughly 5 to 10 minutes.
- Make failures diagnosable by showing which layer broke: domain logic, app wiring, or native runtime integration.
- Build the suite in a way that can extend the existing GitHub Actions CI with a required verification check.

## Non-Goals

- No self-hosted runners.
- No dependency on the real user runtime directory at `~/CapyInn`.
- No attempt to make macOS native UI automation the primary business-verification layer.
- No full production-only backdoors or test-only code paths in release bundles.
- No OCR coverage in the core verification pipeline or Wave 1 rollout.

## Current Baseline

CapyInn already has meaningful verification infrastructure. This design extends that baseline instead of replacing it.

Current repository facts relevant to this design:

- GitHub-hosted CI already exists and runs frontend tests, frontend build, `cargo check`, `cargo test`, and `cargo clippy`
- the frontend already has a broad mocked app-flow suite under `mhm/tests/e2e`
- those `tests/e2e` cases are not real desktop end-to-end tests; they are app-flow and UI integration tests running under `Vitest` and `jsdom` with mocked Tauri APIs
- the frontend already has focused tests for update flow and crash-reporting-related behavior
- the backend already has meaningful Rust test coverage in selected modules
- diagnostics already has a real filesystem-oriented implementation and tests

Known baseline gaps and hygiene issues that matter before expansion:

- the existing suite is not fully green and reliable today
- test and release configuration drift exists in places such as hardcoded version and updater-related values in frontend test config versus package and runtime configuration
- the runtime root, watcher startup, and gateway startup do not yet expose the deterministic seams this design needs
- optional subsystems still rely on development-oriented runtime discovery paths that should stay out of the core verification pipeline until they have explicit deterministic seams

The design below assumes these baseline facts explicitly.

## Chosen Approach

CapyInn will use a layered scenario suite rather than a single all-purpose UI runner.

The suite will be built around four verification layers:

- Rust scenario tests for business logic and persistent state
- frontend app-flow tests for UI wiring and error states
- native smoke tests for real Tauri runtime startup and selected filesystem integrations
- verification reporting that summarizes pass or fail by stage

This is the only approach that matches all selected constraints:

- comprehensive coverage, not just happy-path UI tests
- local-first rollout
- GitHub-hosted runner compatibility for CI expansion
- manageable runtime within the selected 5 to 10 minute budget

## Runtime Isolation

Automated runs must never operate on the normal CapyInn runtime root.

The suite will run against a dedicated runtime root:

- `~/CapyInn-TestSuite`

This root will contain the test database, scans directory, exports, diagnostics, lockfiles, and any other runtime state normally created under `~/CapyInn`.

The source code isolation strategy and runtime isolation strategy are separate:

- source isolation uses a dedicated git worktree
- runtime isolation uses `~/CapyInn-TestSuite`

The runtime root is the architectural requirement. The worktree is only a maintainer workflow preference for isolating implementation work.

The worktree exists to keep experimental suite changes separate from the main working tree. The runtime root exists to keep automated runs deterministic and to prevent test pollution or accidental data loss.

## Verification Commands

The suite will expose distinct entry points instead of a single overloaded command.

### `verify:quick`

Fast local signal for frequent use during development.

Scope:

- selected Rust tests for core domain and service behavior
- selected frontend app-flow tests with mocked Tauri APIs

Target runtime:

- roughly 1 to 2 minutes

### `verify:full`

Primary pre-push and pre-PR command.

Execution order:

1. Reset `~/CapyInn-TestSuite`
2. Seed deterministic fixtures
3. Run Rust scenario suite
4. Run frontend app-flow suite
5. Run native smoke suite
6. Print a final verification summary

Target runtime:

- roughly 5 to 10 minutes

### `verify:repeat`

Stability proving command used before promoting the suite to CI.

Scope:

- repeat `verify:full` multiple times in a clean isolated local workspace

Purpose:

- prove the suite is not flaky before it becomes a GitHub Actions gate

## Verification Layers

## Layer 1: Rust Scenario Suite

The Rust scenario suite is the intended primary business-verification layer, but the first implementation wave must stay grounded in the current code structure.

It will run against a real SQLite database under `~/CapyInn-TestSuite` and verify the persistent effects of multi-step workflows.

Covered flows should include:

- onboarding
- login and lock flow
- reservations lifecycle
- single and group check-in
- room assignment and room changes
- folio lines and billing
- checkout settlement
- housekeeping transitions
- guest history updates
- settings changes that affect behavior
- backup and restore
- night audit
- analytics snapshot sanity checks

Medium-term target:

- most "does the business logic still work?" confidence should live here

First-wave realism:

- some scenarios will necessarily exercise command and service seams together because business behavior is still distributed across `commands`, `services`, and database-backed flows rather than a perfectly isolated domain layer
- the first wave should favor stable SQLite-backed integration scenarios over premature architectural purity

## Layer 2: Frontend App-Flow Suite

The frontend suite will continue using `Vitest + Testing Library + mocked Tauri APIs`.

This layer already exists in the repository today. The work here is primarily:

- inventory current coverage
- identify gaps
- rename the mental model from "E2E" to "app-flow" even if the folder name remains unchanged for now

Its responsibility is not to prove every business rule. Its responsibility is to catch UI and wiring regressions such as:

- wrong command invocation
- missing or broken route guards
- invalid loading and error states
- modal and sheet regressions
- settings UI regressions
- update and crash-report prompt regressions
- form validation regressions

This layer should remain fast and deterministic.

## Layer 3: Native Smoke Suite

The native smoke suite will launch the Tauri app in test mode against `~/CapyInn-TestSuite`.

It will verify only a narrow set of real-runtime assertions such as:

- app boots successfully
- database initializes under the test runtime root
- login succeeds with seeded data
- backup can create a file
- a small end-to-end flow does not crash the app

This layer is intentionally narrow. It exists to catch runtime integration issues that mocks cannot see.

It is not the place for the full business suite.

## Test Mode And Harness Hooks

The suite needs a small, explicit harness layer to become deterministic.

Required environment contract:

- `CAPYINN_RUNTIME_ROOT`
  Forces all runtime paths to live under the specified directory instead of `~/CapyInn`.

- `CAPYINN_DISABLE_WATCHER`
  Prevents automatic watcher startup in tests that do not need background file processing.

- `CAPYINN_DISABLE_GATEWAY`
  Prevents automatic gateway startup in tests that do not need MCP HTTP behavior.

- `CAPYINN_ENABLE_UPDATER`
  Reuses the existing backend updater flag and must become the single source of truth for updater enablement semantics in verification flows.

- `CAPYINN_TEST_NOW`
  Freezes time-sensitive behavior for backup naming, night audit timestamps, analytics windows, and diagnostics timestamps.

Required behavioral rules:

- when `CAPYINN_RUNTIME_ROOT` is set for verification, primary runtime state must resolve under the configured root instead of the normal user directory
- verification mode must prefer explicit configured paths over convenience fallbacks
- test-only controls must be deterministic and observable from scripts

Required harness helpers:

- reset runtime state
- seed fixtures
- inspect app state
- wait for idle background work
- ingest fixture files
- capture logs and artifacts on failure

The harness must be minimal.

The suite should not add broad test-only backdoors. It only needs enough surface area to:

- reset the environment
- seed known initial state
- inspect stable outcomes
- coordinate background work

Release safety rules:

- release bundles must not expose harness commands
- test-only controls must be limited to debug builds or a dedicated non-release feature

## Artifact And Failure Capture

When verification fails, the suite must preserve enough context to debug the failure without rerunning blindly.

Required captured artifacts should include:

- stdout and stderr logs for the failed stage
- the relevant portion of `~/CapyInn-TestSuite`
- diagnostics bundles if present
- any generated backup or export files relevant to the failing scenario

The final verification summary should link to or print the artifact locations, not just pass or fail status.

## External-Service Strategy

The full suite must not depend on live remote systems.

Selected service behavior:

- auto-update flows use deterministic stubs or fakes
- crash-report transport uses deterministic stubs or fakes

This keeps the suite comprehensive without making it flaky or network-dependent.

## GitHub-Hosted CI Expansion

The suite is intended to expand the existing GitHub Actions CI after local proving.

Promotion strategy:

1. Fix the current failing baseline tests before expanding coverage
2. Prove `verify:full` stability locally with repeated clean runs
3. Add or extend a dedicated GitHub Actions workflow or job for the new verification layer
4. Enable the resulting check as a required status check for protected branches

CI design constraints:

- use GitHub-hosted runners only
- avoid workflow path filters on required checks to prevent stuck pending states
- use stable job names for branch protection
- add `merge_group` later if merge queue is adopted

The repository already has GitHub-hosted CI. This design adds verification depth to that reality rather than introducing CI from scratch.

The CI suite does not need to mirror local hardware exactly. It needs to provide stable, actionable verification on GitHub-hosted infrastructure.

## Runtime Budget

The 5 to 10 minute target needs explicit per-layer budgets.

Initial budget targets:

- reset, seed, and summary: 30 seconds or less
- Rust scenario suite: 120 seconds or less
- frontend app-flow suite: 90 seconds or less
- native smoke suite: 90 seconds or less

These are budgets, not promises. If a layer exceeds budget, the verification design must explain why the extra coverage is worth the cost.

## First Implementation Wave

The first implementation wave must stay narrower than the full target surface.

Wave 1 should cover:

- onboarding to login
- reservation to check-in
- checkout settlement plus folio sanity
- night audit plus backup smoke

Wave 1 explicitly excludes from must-have status:

- group booking and group checkout scenarios
- full analytics scenario coverage
- restore flows beyond targeted backup smoke
- OCR verification in any form
- full updater lifecycle verification
- broad crash-report export and recovery coverage beyond what already exists

Those areas can follow after the harness and first-wave suite prove stable.

## Wave Roadmap

The verification program is intentionally phased. `Wave 1` is the first shippable slice, not a promise that all later work must happen immediately.

### Wave 1: Foundation And First CI Gate

Wave 1 exists to replace most manual local clicking with one stable command and one actionable CI check.

Wave 1 delivers:

- a green baseline for the current mocked app-flow suite
- a dedicated runtime root at `~/CapyInn-TestSuite`
- the runtime contract for root override, subsystem gating, and frozen time
- `verify:quick`, `verify:full`, and `verify:repeat`
- Rust scenarios for onboarding to login, reservation to check-in, checkout settlement plus folio sanity, and night audit plus backup smoke
- a narrow native smoke layer
- a GitHub-hosted `verify-wave1` style verification job

### Wave 2: Business Coverage Expansion

Wave 2 exists to widen workflow coverage after Wave 1 proves stable and useful.

Wave 2 should focus on:

- group booking, group check-in, and group checkout flows
- room move, extend-stay, cancellation, and no-show scenarios
- deeper housekeeping and guest-history coverage
- restore verification beyond backup smoke
- settings changes that alter business behavior
- analytics sanity scenarios where they produce business-significant outputs

The goal of Wave 2 is breadth. It should add more hotel operations and edge cases without making the suite flaky.

### Wave 3: Operational Maturity And Release Confidence

Wave 3 exists to turn the suite from a strong pre-merge check into durable release infrastructure.

Wave 3 may include:

- richer artifact capture and post-failure diagnostics
- stronger branch-protection and release gating rules that reuse the same harness
- broader platform coverage if the maintenance cost is justified
- performance and runtime-budget hardening
- tighter repeatability standards for native smoke and long-running verification

The goal of Wave 3 is not just more tests. It is operational confidence and maintainable release discipline.

## Rollout Plan

### Phase 0: Baseline Inventory And Hygiene

Inventory current verification assets and clean up the baseline before adding new architecture.

Success condition:

- current CI and existing test layers are documented
- failing baseline tests are understood
- major config drift affecting verification is identified

### Phase 1: Green Baseline

Before adding new verification layers, fix the currently failing tests and reduce avoidable noise in the existing suite.

Success condition:

- current baseline verification commands pass reliably

### Phase 2: Harness Foundation

Add runtime root override, subsystem toggles, deterministic controls, and the minimal harness helpers needed to run stable scenarios.

Success condition:

- automated runs fully use `~/CapyInn-TestSuite`
- background systems can be controlled deterministically

### Phase 3: First-Wave Scenario Coverage

Add the first wave of Rust scenarios, fill the highest-value frontend app-flow gaps, and add narrow native smoke tests.

Success condition:

- Wave 1 verification is available locally through `verify:full`

### Phase 4: Stability Proving

Run repeated clean-room executions of `verify:full` until the suite demonstrates stable behavior.

Success condition:

- repeated local runs pass without rerun-based masking

### Phase 5: CI Expansion

Move the stable suite into the existing GitHub Actions CI and wire it into protected-branch policy.

Success condition:

- GitHub-hosted verification workflow is green
- branch protection can require it

## File And Ownership Expectations

The implementation should follow the existing repository split between:

- frontend verification entry points under `mhm`
- Rust domain and integration verification under `mhm/src-tauri`
- orchestration scripts under a focused verification script directory

The package-level scripts should only expose entry points. They should not contain large inline orchestration logic.

Optional maintainer workflow:

- a dedicated git worktree may still be used while developing the suite, but it is not part of the verification architecture and is not a release criterion

## Stability Criteria

The suite is ready for CI promotion only when all of the following are true:

- the existing baseline suite is green
- `verify:full` passes repeatedly in clean isolated local runs
- failures clearly identify the broken layer
- native smoke remains small and deterministic
- full runtime stays within the selected 5 to 10 minute budget or close enough to justify the coverage
- required artifacts are captured for debugging failed runs

## Verification Strategy

The design itself should be considered successful only if implementation later demonstrates:

- local quick verification for daily work
- local full verification before push or PR
- repeated local stability proving
- clean GitHub-hosted runner execution
- branch-protection compatibility

## Future Expansion Path

Future work should follow the wave model above instead of adding isolated checks opportunistically.

Any expansion after Wave 1 should preserve three constraints:

- determinism stays more important than raw surface area
- new scenarios must map clearly to either business breadth or release hardening
- the suite must continue reducing, not reintroducing, manual desktop-only verification
