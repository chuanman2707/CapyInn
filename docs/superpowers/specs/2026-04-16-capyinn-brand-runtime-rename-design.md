# CapyInn Brand And Runtime Rename Design

Date: 2026-04-16
Project: HotelManager / MHM -> CapyInn
Status: Approved for implementation

## Goal

Rename the app from `mhm` / `MHM` to `CapyInn` in a way that is publishable, visually consistent, and operationally clean.

This pass covers:
- app branding and visible product naming
- Tauri bundle metadata
- runtime storage paths and filenames
- icon and logo resizing
- user-facing fallback strings that still expose the old name

This pass intentionally treats `CapyInn` as a fresh app. Old runtime data in `~/MHM` is not migrated.

## Approved Decisions

### Naming

- Product name: `CapyInn`
- Bundle identifier: `io.capyinn.app`
- Tauri window title: `CapyInn`
- NPM package name: `capyinn`
- Rust package/bin/lib names: `capyinn`

### Runtime Storage

- Runtime root directory changes from `~/MHM` to `~/CapyInn`
- Main database filename changes from `mhm.db` to `capyinn.db`
- Related runtime files also move under `~/CapyInn`, including:
  - `Scans`
  - `models`
  - `exports`
  - `.gateway-port`
- No migration or compatibility bridge from `~/MHM`

### Logo And Icon Strategy

- Use the current mascot artwork as the source of truth
- Increase artwork fill from roughly `60%` of the icon canvas to roughly `76%`
- Regenerate the Tauri icon set from the resized source
- Increase sidebar logo size:
  - expanded sidebar: from `48px` to about `56px`
  - collapsed sidebar: from `32px` to about `40px`

## Scope

### In Scope

- `mhm/src-tauri/tauri.conf.json`
- `mhm/package.json`
- `mhm/src-tauri/Cargo.toml`
- app shell branding in `mhm/src/App.tsx`
- hardcoded user-facing fallback strings such as `MHM Hotel` and `Hotel Manager`
- runtime path helpers that currently write into `~/MHM`
- onboarding draft storage keys using `mhm-*`
- export filename prefixes using `MHM-*`
- Tauri icon assets in `mhm/src-tauri/icons`
- shared public logo asset used by the app shell

### Out Of Scope

- renaming the repository folder `mhm/`
- broad symbol renames like `useHotelStore`, `HotelInfoSection`, or model/type names that are not user-facing
- data migration from `~/MHM`
- booking or reservation business logic changes unrelated to runtime path updates
- visual redesign beyond logo/icon sizing and app-name replacement

## Rename Matrix

| Surface | Old | New |
|---|---|---|
| Product name | `mhm` | `CapyInn` |
| Bundle identifier | `com.binhan.mhm` | `io.capyinn.app` |
| Window title | `mhm` | `CapyInn` |
| Runtime directory | `~/MHM` | `~/CapyInn` |
| Main DB file | `mhm.db` | `capyinn.db` |
| Onboarding draft key | `mhm-*` | `capyinn-*` |
| Export prefix | `MHM-*` | `CapyInn-*` |

## Runtime Reset Policy

Because this rename is being done before open source cleanup is finalized, the runtime rename should behave like a fresh install.

Rules:
- the app should stop creating new files under `~/MHM`
- the app should create and read only `~/CapyInn/*`
- if `~/MHM` exists, it is ignored
- onboarding on the renamed app starts from the new runtime root

This avoids carrying legacy local state into the renamed app and keeps the implementation simpler.

## Branding Surface Changes

The following must be updated to `CapyInn` or an equivalent neutral fallback:
- app window title
- shell title fallback text
- invoice and group-invoice fallback hotel/app naming where it currently defaults to `MHM Hotel` or `Hotel Manager`
- MCP gateway server info and proxy error messages mentioning the old product name
- settings snippets or examples that still reference `hotel-manager`

The rename should prioritize actual user-visible text. Internal domain words like `hotel` remain valid and should not be renamed for the sake of branding.

## Icon And Logo Design Notes

The current icon assets render small because the visible artwork occupies only about `60%` of the canvas. Other macOS apps commonly look larger because they use more of the available bounding box, not because of a fixed golden ratio.

Target treatment:
- preserve comfortable padding so the mascot does not feel cramped
- increase visible fill to roughly `74-78%`
- use the same artwork across Dock/Finder/app-shell contexts

Sidebar treatment:
- keep the current placement and alignment
- increase display size without changing the overall sidebar structure
- avoid making the collapsed state feel crowded

## Implementation Approach

### Metadata Layer

Update build metadata first:
- Tauri product name
- Tauri identifier
- Tauri title
- package names where safe

### Runtime Layer

Update all runtime path builders and filenames together so the app has exactly one new storage root:
- database init
- gateway lockfile path
- OCR model path
- watcher scan path
- audit backup/export path
- any file open/save helpers that assume `~/MHM`

### UI Layer

Update:
- app-shell logo sizing
- app name fallback strings
- onboarding draft storage key prefix

### Asset Layer

Generate a resized master logo and rebuild the Tauri icon set from it.

## Verification Plan

### Backend / Runtime

- `cargo check --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml`
- `cargo test commands::onboarding::tests --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml`

### Frontend

- `npm test -- tests/e2e/00-onboarding.test.tsx`
- rerun app-shell-adjacent mocked flow if app title/logo code is touched beyond onboarding

### Manual Runtime Checks

- dev app window title shows `CapyInn`
- new runtime directory is created under `~/CapyInn`
- new database file is `~/CapyInn/capyinn.db`
- no fresh files are written into `~/MHM`
- sidebar logo is visibly larger in both expanded and collapsed states
- Dock/Finder icon appears optically closer to surrounding apps

## Rollback Boundary

This work should stay isolated to:
- app metadata
- runtime path naming
- user-facing branding strings
- icon/logo assets and shell sizing

If rollback is needed, the entire rename can be reverted without touching backend booking-domain refactor behavior.

## Risks

- Renaming Rust package/bin/lib names can affect local commands or imports if done too broadly
- Tauri identifier changes may alter how macOS treats the app bundle as a distinct app
- Runtime path changes can look like “missing data” if someone expects the old local DB to appear automatically

Mitigations:
- do not rename the repository folder
- limit non-user-facing symbol churn
- verify the runtime root switch explicitly
- treat `CapyInn` as a fresh app by design

## Success Criteria

The rename is complete when:
- the app launches and presents itself as `CapyInn`
- no user-visible `mhm`, `MHM`, or `Hotel Manager` branding remains in normal flows
- runtime data is created only under `~/CapyInn`
- the logo is visually larger and the app icon no longer looks undersized beside other macOS apps
