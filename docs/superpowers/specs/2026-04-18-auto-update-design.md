# CapyInn Auto Update Design

Date: 2026-04-18
Owner: Codex
Status: Draft approved for spec write-up

## Goal

Add a production auto-update flow for CapyInn so released desktop users can discover, install, and apply a new app version with minimal friction.

The feature should make four things true:

- production releases are created from version tags, not every push to `main`
- the app checks for updates automatically on launch and manually from Settings
- when an update is available, the app exposes a small in-app `UPDATE` entrypoint instead of forcing a modal flow
- after installation completes, the app waits for the user to click `RESTART NOW` before switching to the new version

This design is intentionally limited to desktop app updates. It does not introduce staged rollout, beta channels, percentage rollout, or backend-driven targeting.

## User Decisions Locked In

The following product decisions were explicitly chosen during brainstorming and are part of this spec:

- release model: production updates are published only from version tags after CI passes, not from every push to `main`
- release trigger: push a tag such as `v0.2.0` and let GitHub Actions build and publish automatically
- auto-check behavior: check once when the app reaches the main authenticated shell
- manual check behavior: expose a `Check for updates` action in Settings
- supported updater targets in v1: Windows and macOS
- Linux updater scope: out of scope for this pass
- main badge flow:
  - `UPDATE`
  - `UPDATING...`
  - `RESTART NOW`
- interaction model: clicking `UPDATE` immediately starts download and install with no modal, toast, or detailed progress UI
- completion model: after install finishes, wait for an explicit `RESTART NOW` click
- failure UX: show minimal error feedback only on failure, then allow retry
- platform expectation:
  - Windows aims for a seamless production experience
  - macOS is supported, but may still require OS approval because the project has no Apple Developer signing or notarization

## Constraints

- CapyInn is a Tauri 2 desktop app with a React frontend and Rust backend
- the repo currently has a CI workflow in `.github/workflows/ci.yml` but no active published release workflow in the checked-in tree
- `mhm/src-tauri/tauri.conf.json` does not yet enable updater configuration
- the frontend already has an app-shell header in `mhm/src/App.tsx` that is a natural home for a compact update badge
- the app already uses Sonner toasts, but the chosen update UX avoids toast noise during normal download and install
- the project does not have Apple Developer credentials, so notarized macOS distribution is not available in v1
- updater security still requires Tauri updater artifact signing, which is separate from OS code signing
- the work must not stage unrelated dirty files already present in the worktree

## Chosen Approach

Use Tauri's official updater plugin with a GitHub Releases-backed static manifest flow.

The release pipeline will publish updater artifacts and a `latest.json` manifest whenever a version tag is pushed. The app shell will check for updates on launch, surface a compact `UPDATE` badge when a newer version exists, install the update in the background when the badge is clicked, and switch the badge to `RESTART NOW` after installation succeeds.

Why this approach:

- it matches the locked release model of tagging production versions instead of shipping every `main` commit
- it keeps infrastructure simple by reusing GitHub Releases instead of introducing a separate update server
- it fits naturally into the current Tauri architecture
- it delivers the desired user-facing flow without a noisy modal or progress-heavy UX
- it keeps the unavoidable macOS limitation isolated to platform packaging rather than to the core updater design

## Non-Goals

- pushing updates from every merge to `main`
- beta and stable channels in the same release pipeline
- rollout percentages, phased rollout, or update cohorts
- forced restart immediately after install
- in-app release notes UI in v1
- visible download percentage or progress bar in v1
- Linux auto-update in this pass
- notarized macOS distribution in this pass

## Existing State

Today CapyInn already has:

- a Tauri app shell in `mhm/src/App.tsx` with a header badge cluster
- Sonner integrated in the root app for lightweight notifications
- versions aligned at `0.1.0` in both `mhm/package.json` and `mhm/src-tauri/tauri.conf.json`
- a release-signing guidance document in `docs/release-signing.md`

The current gap is that the checked-in project state does not yet include an updater-enabled Tauri config, an updater-aware frontend state machine, or a release workflow that publishes Windows and macOS updater artifacts plus a stable manifest.

There is also a documentation mismatch today: `docs/release-signing.md` describes a release workflow and notes that macOS binaries are not officially published, while the repository tree currently exposes only `ci.yml`. This implementation must resolve that drift.

## Architecture

### 1. Release model

Production releases are created only from pushed tags matching `v*`.

Required policy:

- `main` remains the integration branch
- a production update becomes visible to users only after a tagged workflow completes successfully
- the workflow must fail early if the pushed tag version does not match the checked-in app version
- the workflow should verify both:
  - `mhm/package.json`
  - `mhm/src-tauri/tauri.conf.json`

Recommended tag example:

- `v0.2.0`

This keeps app versioning intentional and prevents accidental production updates from arbitrary commits.

### 2. Release pipeline

Add a dedicated publish workflow, for example `.github/workflows/release.yml`, triggered by pushed version tags.

Required jobs:

- `windows-latest`
- `macos-latest`

Required responsibilities:

- checkout code
- install Node and Rust dependencies
- run the same verification baseline used for release confidence
- build Tauri bundles for each target platform
- generate updater artifacts and signatures
- publish a GitHub Release automatically for the tag
- attach all required updater assets plus `latest.json`

Workflow policy:

- release publication is automatic, not draft-only
- the workflow should publish only after all required platform jobs pass
- if one required target fails, no partial production release should be published
- implementation may use an internal draft release while collecting assets, but that draft must not become the public production release until all required jobs succeed
- implementation may keep or restore Linux direct-download publishing separately, but Linux updater support is not required by this spec

### 3. Updater artifact and manifest strategy

The app will use a GitHub Releases static manifest endpoint.

Required output per production release:

- Windows installer and updater signature files
- macOS updater bundle and updater signature files
- a canonical `latest.json` manifest describing the newest stable release

The app should query a stable URL equivalent to:

- `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`

Manifest policy:

- it must represent the newest published stable production release
- it must include both Windows and macOS platform entries used by Tauri updater
- release publication should update this manifest as part of the same workflow so users do not see mismatched assets and metadata

### 4. Signing strategy

Two signing layers must be treated separately.

#### Updater artifact signing

This is required for secure update verification.

Required secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the key is password-protected

Required app config:

- embed the corresponding public key in Tauri updater configuration

Without updater signing, production auto-update must be considered disabled.

#### OS package signing

Windows:

- Windows code signing remains optional in v1
- unsigned Windows builds may still be published if no Windows certificate secrets are available
- Windows signing should remain compatible with the current direct-download release guidance

macOS:

- v1 should build macOS artifacts on GitHub-hosted macOS runners without requiring Apple Developer credentials
- implementation should prefer ad-hoc signing for the macOS app bundle when feasible, rather than a completely unsigned bundle, because it gives the app a consistent local code signature without needing Apple-issued certificates
- no notarization is available in v1
- the spec must continue to treat macOS approval friction as expected behavior, not as a release failure

### 5. App-side update lifecycle

The app should manage updates through one root-level update controller, owned near the app shell rather than by individual pages.

Required states:

- `idle`
- `checking`
- `available`
- `installing`
- `ready_to_restart`
- `error`

Required transitions:

- app enters main shell -> `checking`
- no update found -> `idle`
- update found -> `available`
- user clicks `UPDATE` -> `installing`
- install succeeds -> `ready_to_restart`
- user clicks `RESTART NOW` -> app relaunches
- any check or install failure -> `error`, then recover to `available` or `idle` depending on whether retry is possible

Single-flight policy:

- do not run more than one check at a time
- do not run more than one install at a time
- ignore repeated `UPDATE` clicks while `installing`
- if a manual Settings check is requested during an active check, reuse the existing request rather than starting another

### 6. Auto-check timing

Automatic update check should happen once per launch, only after the user reaches the main app shell.

Required behavior:

- do not check while the onboarding wizard is still active
- do not check on the locked login screen before authentication completes
- after bootstrap completes and the main shell renders, trigger one background update check
- if that check fails due to network or manifest issues, fail silently in the auto-check path

This avoids noisy failures during startup while still satisfying the product expectation that opening the app reveals newly published releases.

### 7. Header badge UX

The app shell header should host the primary update interaction.

Required labels:

- `UPDATE`
- `UPDATING...`
- `RESTART NOW`

Required behavior:

- when state is `available`, show `UPDATE`
- clicking `UPDATE` must immediately start background download and install
- while `installing`, show `UPDATING...`
- do not show modal dialogs, progress bars, or toasts during a normal successful install path
- when installation completes, show `RESTART NOW`
- clicking `RESTART NOW` should relaunch the app into the new version

Visual design guidance:

- place the badge alongside the existing system badges in the header
- give it stronger emphasis than passive status badges so the action is discoverable
- keep it compact and consistent with the current badge/button language in `App.tsx`

### 8. Settings UX

Add a `Software Update` section to Settings.

Required content:

- current app version
- current update status
- a `Check for updates` action
- when applicable, the newest available version string

Manual check behavior:

- if no update is available, show a positive confirmation such as `You are on the latest version`
- if the check fails, show a clear error
- if an update is already `available`, `installing`, or `ready_to_restart`, Settings should reflect that same root-level state instead of creating a second flow

Settings is the secondary control surface. It must not diverge from the header badge state machine.

### 9. Failure handling

The update feature should stay quiet during happy-path background work and become explicit only on failure.

Required rules:

- auto-check failures: silent
- manual-check failures: explicit UI feedback
- install failures after clicking `UPDATE`: explicit UI feedback
- after install failure, return the badge to a retryable state

Recommended badge fallback:

- if metadata about the available version is still valid, return to `UPDATE`
- if the failure invalidates the available release information, return to `idle` and require a fresh check

The app should never get stuck permanently on `UPDATING...` after an error path.

### 10. Platform behavior expectations

Windows:

- target the intended seamless experience
- users should see the app-installed update and only need the final `RESTART NOW` action

macOS:

- support updater artifacts and in-app install flow
- do not promise zero-friction behavior
- if a machine or update path requires additional macOS approval, that is expected in v1
- users who already approved the app once may often avoid repeated prompts on in-place updates, but this is not guaranteed without notarization

This expectation must be documented in release docs so the operational behavior matches what the product can truly deliver.

## Testing

Minimum verification for implementation:

- frontend state tests for:
  - `idle -> checking -> available`
  - `available -> installing -> ready_to_restart`
  - install failure recovery
- manual Settings check test for no-update and failure cases
- release workflow dry-run validation on a test tag
- one real Windows update path test from an older installed version to a newer tagged release
- one real macOS update path test from an older installed version to a newer tagged release on a machine that has already approved the app once

Operational verification should confirm:

- the published tag version matches the app version shown in Settings
- `latest.json` points to the just-published release
- updater signatures are present for required platforms
- the app never exposes `RESTART NOW` before installation actually succeeds

## Implementation Boundaries

This spec covers:

- Tauri updater configuration
- frontend app-shell update state and UI
- Settings integration
- GitHub Actions release automation for Windows and macOS
- release manifest and updater signing setup
- documentation alignment for release behavior

This spec does not cover:

- release-note authoring process
- beta channels
- Linux updater rollout
- notarization onboarding
- backend services outside the updater path
