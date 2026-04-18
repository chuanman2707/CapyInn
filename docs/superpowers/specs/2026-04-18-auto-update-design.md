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
- `mhm/package.json` does not yet include:
  - `@tauri-apps/plugin-updater`
  - `@tauri-apps/plugin-process`
- `mhm/src-tauri/Cargo.toml` does not yet include:
  - `tauri-plugin-updater`
  - `tauri-plugin-process`
- `mhm/src-tauri/capabilities/default.json` currently grants only `core:default` and `opener:default`, so updater and relaunch permissions are absent
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
- versions aligned at `0.1.0` in:
  - `mhm/package.json`
  - `mhm/src-tauri/tauri.conf.json`
  - `mhm/src-tauri/Cargo.toml`
- a release-signing guidance document in `docs/release-signing.md`

The current gap is that the checked-in project state does not yet include an updater-enabled Tauri config, an updater-aware frontend state machine, or a release workflow that publishes Windows and macOS updater artifacts plus a stable manifest.

There is also a documentation mismatch today: `docs/release-signing.md` describes a release workflow and notes that macOS binaries are not officially published, while the repository tree currently exposes only `ci.yml`. This implementation must resolve that drift.

## Concrete Integration Points

This spec intentionally changes more than UI. The implementation is expected to touch these integration points directly:

- `mhm/package.json`
  - add `@tauri-apps/plugin-updater`
  - add `@tauri-apps/plugin-process`
- `mhm/src-tauri/Cargo.toml`
  - add `tauri-plugin-updater`
  - add `tauri-plugin-process`
- `mhm/src-tauri/src/lib.rs`
  - register the updater and process plugins in the Tauri builder
- `mhm/src-tauri/tauri.conf.json`
  - add updater configuration
  - decide the Windows updater artifact source explicitly
- `mhm/src-tauri/capabilities/default.json`
  - add `updater:default`
  - add `process:allow-relaunch`
- `mhm/src/App.tsx`
  - create the root-level update lifecycle and the header badge entrypoint
- `mhm/src/pages/settings/*`
  - add a `Software Update` section that reflects the same root state
- `.github/workflows/release.yml`
  - add the publish flow triggered by version tags
- a dedicated workflow helper script, for example `scripts/generate-latest-json.mjs`
  - generate the canonical multi-platform `latest.json`

## Architecture

### 1. Release model

Production releases are created only from pushed tags matching `v*`.

Required policy:

- `main` remains the integration branch
- a production update becomes visible to users only after a tagged workflow completes successfully
- the workflow must fail early if the pushed tag version does not match the checked-in app version
- the workflow should verify all version declarations:
  - `mhm/package.json`
  - `mhm/src-tauri/tauri.conf.json`
  - `mhm/src-tauri/Cargo.toml`
- this is an intentional tightening of current release behavior, which historically only validated the Tauri app version

Recommended tag example:

- `v0.2.0`

This keeps app versioning intentional and prevents accidental production updates from arbitrary commits.

### 2. Release pipeline

Add a dedicated publish workflow, for example `.github/workflows/release.yml`, triggered by pushed version tags.

Required jobs:

- `windows-latest` for the Windows production updater artifact
- `macos-latest` targeting:
  - `darwin-aarch64`
  - `darwin-x86_64`

Required responsibilities:

- checkout code
- install Node and Rust dependencies
- run the same verification baseline used for release confidence
- build Tauri bundles for each target platform
- generate updater artifacts and signatures
- publish a GitHub Release automatically for the tag
- attach all required updater assets plus `latest.json`

Windows packaging policy:

- the updater contract should use the NSIS installer artifact for `windows-x86_64`
- MSI may still be attached as an optional direct-download asset if the project keeps `bundle.targets: "all"`, but the updater manifest must not be ambiguous about which Windows artifact it points to

Workflow policy:

- release publication is automatic, not draft-only
- the workflow should publish only after all required platform jobs pass
- if one required target fails, no partial production release should be published
- implementation may use an internal draft release while collecting assets, but that draft must not become the public production release until all required jobs succeed
- implementation may keep or restore Linux direct-download publishing separately, but Linux updater support is not required by this spec

### 3. Updater artifact and manifest strategy

The app will use a GitHub Releases static manifest endpoint with one canonical multi-platform `latest.json`.

Required output per production release:

- Windows NSIS updater artifact and signature
- macOS `darwin-aarch64` updater artifact and signature
- macOS `darwin-x86_64` updater artifact and signature
- a canonical `latest.json` manifest describing the newest stable release

The app should query a stable URL equivalent to:

- `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`

Manifest contract:

- this design does not use per-request endpoint templating such as `{{target}}/{{arch}}/{{current_version}}`
- instead, the endpoint always returns one static manifest whose `platforms` object contains every supported production updater target
- required platform keys are:
  - `windows-x86_64`
  - `darwin-aarch64`
  - `darwin-x86_64`

Manifest policy:

- it must represent the newest published stable production release
- it must include both Windows and macOS platform entries used by Tauri updater
- release publication should update this manifest as part of the same workflow so users do not see mismatched assets and metadata
- the URLs embedded inside `latest.json` must be immutable versioned asset URLs for the specific tag release, not `releases/latest/download/...` links

Manifest generation mechanism:

- add a dedicated workflow step or script that builds `latest.json` after all updater artifacts exist
- the generator must read:
  - the resolved app version
  - the release publication date
  - the final versioned asset URLs for each updater artifact
  - the raw contents of each `.sig` file
- the generator must inline the signature strings into `latest.json`
- the workflow must upload the generated `latest.json` as an asset on the same release

Using GitHub Releases alone is not enough here. The manifest generation step is a required part of the implementation, not an optional convenience.

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

### 4.1 Plugin and capability prerequisites

The updater flow is not only a workflow concern. The runtime must explicitly enable the required plugins and permissions.

Required Rust plugins:

- `tauri-plugin-updater`
- `tauri-plugin-process`

Required frontend packages:

- `@tauri-apps/plugin-updater`
- `@tauri-apps/plugin-process`

Required default capability permissions:

- `updater:default`
- `process:allow-relaunch`

If any of these are missing, the updater flow should be considered incomplete even if the release pipeline publishes valid artifacts.

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
- only trigger after all three current `App.tsx` shell gates have been passed:
  - `bootstrapLoading` is false
  - setup is complete
  - if app lock is enabled, the user is authenticated
- in the current app structure, that means the check starts only after the final early-return guard for onboarding/login has been crossed in `mhm/src/App.tsx`
- after the main shell renders, trigger one background update check
- if that check fails due to network or manifest issues, fail silently in the auto-check path

This avoids noisy failures during startup while still satisfying the product expectation that opening the app reveals newly published releases.

Long-running-session policy:

- the chosen v1 design does not include periodic background recheck
- an app session that stays open for many hours may not discover a newly released update until either:
  - the user manually checks from Settings
  - the app restarts

This is an intentional tradeoff locked in during brainstorming in favor of the simpler `once per launch + manual check` model.

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

- network failure during install or check timeout: keep the known release metadata and return to `UPDATE`
- download timeout during install: keep the known release metadata and return to `UPDATE`
- signature verification failure: return to `idle`, discard the cached available release state, and require a fresh check
- manifest schema error: return to `idle`, discard the cached available release state, and require a fresh check
- updater artifact `404` or missing asset: return to `idle`, discard the cached available release state, and require a fresh check

The app should never get stuck permanently on `UPDATING...` after an error path.

Install timeout policy:

- the install path must have a bounded timeout so `UPDATING...` cannot last forever on a weak or broken network
- the exact number can be finalized during implementation, but the spec requires a real timeout, not an infinite wait

Crash and restart recovery:

- UI update state does not need to persist across crashes or forced app exits in v1
- on the next launch, the app should reset to `idle` and run the normal update check again
- if the updater plugin reuses any partial cached download internally, that is acceptable, but the CapyInn UI state machine should still treat the new launch as a fresh check

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
- negative security test: a tampered or unsigned artifact referenced by `latest.json` must be rejected by the updater flow and surfaced as an error state
- negative manifest test: malformed `latest.json` must not leave the badge stuck in `UPDATING...`
- negative asset test: manifest points to a missing asset URL and the app returns cleanly to a retryable or fresh-check state
- permission regression test: build or runtime verification should confirm the default capability includes both updater and relaunch permissions before claiming the feature works

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
