<div id="top" align="center">

<img src="mhm/public/app-logo.png" alt="CapyInn logo" width="120">

# CapyInn

**Offline-first property management software for mini hotels**

*A desktop PMS for small hotels and guesthouses in Vietnam.*

[![CI](https://img.shields.io/github/actions/workflow/status/chuanman2707/CapyInn/ci.yml?style=for-the-badge&label=CI)](https://github.com/chuanman2707/CapyInn/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow?style=for-the-badge)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri_2-FFC131?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React_19-61DAFB?style=for-the-badge&logo=react&logoColor=black)](https://react.dev)
[![SQLite](https://img.shields.io/badge/SQLite-003B57?style=for-the-badge&logo=sqlite&logoColor=white)](https://sqlite.org)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org)

**Property onboarding · Vietnamese ID OCR · Check-in/check-out · Reservations · Night audit**

<p>
  <a href="#what-capyinn-solves"><strong>Why CapyInn</strong></a> ·
  <a href="#product-demo"><strong>Demo</strong></a> ·
  <a href="#key-features"><strong>Features</strong></a> ·
  <a href="#local-development"><strong>Local development</strong></a> ·
  <a href="#verification"><strong>Verification</strong></a>
</p>

</div>

![CapyInn dashboard hero](Public/dashboard.png)

> Built for mini hotels that need one local app for room status, guest intake, nightly billing, housekeeping, and end-of-day reconciliation.

<p align="center">
  <img src="https://img.shields.io/badge/Offline--first-0F172A?style=flat-square" alt="Offline-first">
  <img src="https://img.shields.io/badge/Vietnamese%20ID-OCR-0F766E?style=flat-square" alt="Vietnamese ID OCR">
  <img src="https://img.shields.io/badge/Desktop-Tauri%202-C2410C?style=flat-square" alt="Desktop app">
  <img src="https://img.shields.io/badge/Storage-Local%20SQLite-1D4ED8?style=flat-square" alt="Local SQLite">
</p>

CapyInn is a desktop app for mini hotels and guesthouses that need a local-first operating tool without relying on a remote backend. The project focuses on real front-desk workflows: room layout setup, faster guest intake, Vietnamese ID OCR, nightly pricing, housekeeping, revenue reporting, and end-of-day reconciliation.

> Note: `CapyInn` is a clean-slate rename from `MHM`. Current builds use the new runtime root at `~/CapyInn` and do not auto-migrate legacy local data from `~/MHM`.

<details>
<summary>Table of contents</summary>

- [What CapyInn solves](#what-capyinn-solves)
- [Product demo](#product-demo)
- [Key features](#key-features)
- [Tech stack](#tech-stack)
- [System requirements](#system-requirements)
- [Local development](#local-development)
- [Verification](#verification)
- [Repository layout](#repository-layout)
- [Known limitations](#known-limitations)
- [Additional docs](#additional-docs)
- [Contributing](#contributing)
- [License](#license)

</details>

## What CapyInn solves

CapyInn is built for a narrow but practical use case: small hotels that need a system they can run locally, control directly, and adopt without a long setup project.

| Before | With CapyInn |
| --- | --- |
| Handwritten logs and fragmented tracking | Room status, bookings, and transactions live in one app |
| Manual guest registration entry | OCR extracts Vietnamese ID details and speeds up intake |
| Nightly pricing calculated by hand | Check-in, extend-stay, check-out, and folio flows are automated |
| End-of-day reporting done manually | Dashboard, analytics, expenses, and night audit are built in |
| Initial setup takes too much time | Onboarding generates room types, layouts, and operating defaults |

## Product demo

### Dashboard

![CapyInn dashboard](Public/dashboard.png)

### Guest check-in and booking flow

<p align="center">
  <img src="Public/Group-Check-In.png" alt="Group check-in flow" width="48%">
  <img src="Public/Group-Booking.png" alt="Group booking flow" width="48%">
</p>

### Guest profile and operations

<p align="center">
  <img src="Public/Guest.png" alt="Guest profile view" width="48%">
  <img src="Public/Night-Audit.png" alt="Night audit flow" width="48%">
</p>

### Analytics and settings

<p align="center">
  <img src="Public/Analytics.png" alt="Analytics dashboard" width="48%">
  <img src="Public/Settings.png" alt="Settings screen" width="48%">
</p>

<p align="center">
  <img src="Public/Settings2.png" alt="Advanced settings screen" width="48%">
</p>

## Key features

### Onboarding and property setup

- Configure hotel identity, check-in and check-out rules, invoice details, and app lock
- Create room types and default pricing during the first-run wizard
- Generate a room layout by floors, room count, and naming scheme

### Front-desk operations

- Dashboard organized around the configured room layout
- Check-in, check-out, extend-stay, and reservation flows in one desktop app
- Mid-stay room change for an in-house guest, listing the valid target rooms for that booking and offering a choice between keeping the original price and charging the difference
- Reservation calendar timeline: click or drag across empty cells to open a check-in or a reservation with those dates already filled in
- Backfill sheet for recording a stay that already happened, opened from the same calendar
- Read-only detail popup for a booking that has already checked out, including its issued invoice
- Void a booking entered by mistake: admin-only, behind a two-second hold, with a confirmation box that states the money and room impact before the action
- Support for multiple guests on the same booking
- Fast copy flow for guest registration details

### Vietnamese ID OCR

- Local OCR powered by PaddleOCR v5 through `ocr-rs`
- Watches `~/CapyInn/Scans/` for new scan files
- Extracts guest name, national ID number, birth date, and address for check-in

### Pricing, billing, and reporting

- Rates are a property of the room type, not of the individual room: hourly, overnight, and nightly/daily models, with an hourly total capped at the cheaper block
- Weekend uplift, peak-season uplift, and early check-in / late check-out surcharges
- Peak seasons are declared in Settings as date ranges; the uplift is charged only for the nights that fall inside one
- Extra-person surcharge per guest per night above the room's included headcount — reservations carry a guest count and are quoted with it
- Manual nightly rate: the front desk can override the engine price per night at check-in, on a reservation, and per room on a group check-in; the override survives confirm and modify because it is stored as a rate per night, not a total
- Prices shown before a stay come from the backend preview that will charge it, never from arithmetic in the UI — the one exception is a manually entered rate, where the sheet shows `rate × nights` and the backend recomputes and validates the same product before it charges
- Charge, payment, deposit, and balance tracking
- Revenue analytics, expense tracking, and CSV export

### Housekeeping and night audit

- Post-checkout housekeeping state tracking
- Maintenance notes per room
- Night-audit flow for daily reconciliation

### Temporary residence declaration (Khai báo tạm trú)

- Dedicated workspace that turns guest ID images into upload-ready files for the Ministry of Public Security portal at `tbltkbtt.bocongan.gov.vn`
- Identity capture reads the CCCD QR code first and falls back to passport MRZ through `ocr-rs`, with a manual form for anything neither can read
- Extracted identities are linked to an existing stay, then exported as XLSX for Vietnamese guests and XML for foreign guests
- Batch history and a reconciliation checklist exist because the portal reports "import successful" even when it accepts zero records
- The module reads PMS tables only; it adds its own tables and never mutates rooms, bookings, or guests

### Auto-update

- Release builds check the GitHub Releases `latest.json` feed through the Tauri updater plugin
- Update artifacts are signed, and the signing and manifest details are documented in [docs/release-signing.md](docs/release-signing.md)

### MCP and automation integrations

- CapyInn can be extended through MCP-friendly workflows for operator tooling and agent-driven automations
- These flows can be paired with OpenClaw and n8n for custom orchestration around hotel operations
- For Zalo personal chat automations, you can use the prebuilt community node [`n8n-nodes-zca-zalo`](https://www.npmjs.com/package/n8n-nodes-zca-zalo), published on npm and built on top of `zca-js`

### Crash reporting

- Severe crashes are always written locally under `~/CapyInn/diagnostics` so the app can recover on the next launch.
- Sending a sanitized crash report to Sentry is optional and controlled in Settings. The remote report flow does not include usage analytics, session replay, guest records, or raw OCR payloads.

## Tech stack

| Layer | Technology |
| --- | --- |
| App shell | Tauri 2 |
| Backend | Rust + SQLite (`sqlx`) |
| Frontend | React 19 + TypeScript |
| State | Zustand |
| UI | Tailwind CSS 4 + shadcn/ui |
| OCR | `ocr-rs` + PaddleOCR v5 + MNN |
| Charts | Recharts |
| Tests | Vitest + Rust tests + Clippy |

## System requirements

| Component | Requirement |
| --- | --- |
| macOS | 12+ |
| Node.js | 20+ |
| Rust | stable via `rustup` |
| Xcode CLT | recent version (macOS builds) |
| Disk footprint | roughly 25MB before operational data |

The install steps below describe a macOS development machine. The project is verified most heavily on macOS and Apple Silicon, which is the only macOS architecture the release workflow builds.

Tagged releases also publish a Windows NSIS installer and a Linux AppImage. Those bundles are produced by CI on `windows-latest` and `ubuntu-22.04` but receive far less hands-on testing than the macOS build.

## Local development

### Install prerequisites

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
xcode-select --install
node --version
```

### Clone and run the desktop app

```bash
git clone https://github.com/chuanman2707/CapyInn.git
cd CapyInn/mhm
npm ci
npm run tauri dev
```

### Build a release bundle

```bash
cd CapyInn/mhm
npm run tauri build
```

Release bundles are generated under `mhm/src-tauri/target/release/bundle/`.

## Verification

```bash
cd CapyInn/mhm
npm test
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Every command above is a hard CI gate, `cargo fmt` included — skipping the format check locally means a red build on a PR that otherwise passes.

The repository also ships scripted verification gates. `verify:full` is the smoke gate the release checklist expects to pass before a tag is pushed; it runs the quick wave, the frontend suite, booking and backup scenario tests, and a native Tauri startup smoke against an isolated runtime root.

```bash
cd CapyInn/mhm
npm run verify:quick
npm run verify:full
```

If you only need the web UI during frontend work:

```bash
cd CapyInn/mhm
npm run dev
```

## Repository layout

```text
CapyInn/
├── Public/                 # README demo screenshots
├── docs/                   # Architecture guardrails, release docs, plans and specs
├── mhm/
│   ├── src/                # React UI, stores, pages, components
│   ├── src-tauri/          # Rust backend, IPC commands, DB, gateway, OCR, declaration
│   ├── tests/              # Vitest suites and mocked desktop flows
│   ├── scripts/            # Verification and release helper scripts
│   ├── shared/             # Types shared between the frontend and helper scripts
│   ├── skills/             # Agent skill definition for the MCP gateway
│   ├── public/             # Static assets
│   └── models/             # OCR models
├── CHANGELOG.md
├── CONTRIBUTING.md
├── SECURITY.md
└── README.md
```

`mhm/` is the current implementation path, not the product name. Renaming it is deliberately postponed; see [docs/architecture/core-pms-boundaries.md](docs/architecture/core-pms-boundaries.md).

## Known limitations

- Check-in OCR is optimized for Vietnamese national ID cards; the passport MRZ reader currently lives in the temporary residence declaration workspace rather than the check-in scan flow
- macOS Apple Silicon is the primary target; Windows and Linux bundles are published by CI but are not verified as thoroughly
- The project is designed for mini-hotel scale, not large chain operations
- Voiding a booking is per booking: a room inside a group booking cannot be voided on its own yet

## Additional docs

- [Core PMS boundaries](docs/architecture/core-pms-boundaries.md)
- [Contributing guide](CONTRIBUTING.md)
- [Release checklist](docs/release-checklist.md)
- [Release signing and updater](docs/release-signing.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

Short checklist:

1. Fork the repository
2. Create a branch from `main`
3. Keep commit messages in Conventional Commits format
4. Re-run `npm test`, `npm run build`, `cargo check`, `cargo test`, `cargo clippy`, and `cargo fmt -- --check`
5. Open a pull request with scope and verification notes

## License

CapyInn is released under the [MIT License](LICENSE).
