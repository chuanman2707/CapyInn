# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- Temporary residence declaration workspace (khai báo tạm trú): its own declaration
  module, extraction and validation, XLSX and XML writers generated from the official
  template, a declaration page with a sidebar badge and reconcile loop, and a
  `--check-resources` mode on the probe CLI
- Mid-stay room change for a guest already in house, with the valid target rooms
  listed for the booking and an option to keep the original price or charge the
  difference between the two rooms
- Reservation calendar timeline: click or drag across empty cells to open a check-in
  or a reservation with those dates already prefilled
- Backfill sheet for recording a stay that already happened, with the matching
  `backfill_stay` service and Tauri command behind it
- Read-only booking detail popup for checked-out guests, and a read-first
  `viewInvoice` that shows an already-issued invoice without creating a new one
- Peak seasons: declare one as a date range in Settings, delete it, and have
  contiguous declared days group back into a single season
- Extra-guest pricing: a flat per-guest, per-night charge above the room's included
  headcount, with reservations storing a guest count and being priced with it
- Room-keyed price preview so the reservation sheet quotes what the engine will
  actually charge, with the breakdown in Vietnamese
- Restore drill now asserts backup freshness, schema version, and a row baseline
- Frontend experimental runtime profile
- open-source repository metadata and community files
- CI workflow and GitHub issue / PR templates

### Changed

- Room cards show the room type's configured rate instead of `rooms.base_price`,
  which the pricing model does not honour as a price
- The reservation sheet takes a checkout date from a calendar instead of a nights box
- Night audit and the sheets close the day by the local day rather than a
  UTC-derived one
- public repository cleanup for internal agent files and docs layout
- README restructuring for public contributors and onboarding-based setup

### Fixed

- Peak-season uplift is charged per night inside the season, not once per check-in day
- Vietnamese room type names no longer lose their configured price: room types are
  matched case-insensitively with the same folding on both sides of the comparison
- The guest charge survives extend-stay, early checkout, modify, and check-in
- Previews fail visibly instead of quoting a default the front desk will not honour

### Security

- removed internal/experimental assets that should not ship in the public tree

## [0.1.6] - 2026-05-09

> Release-line note: `v0.1.6` is not a descendant of `v0.1.5`. Both tags branch
> from a common point 36 commits after `v0.1.4`. The CEO-agent groundwork that
> `v0.1.5` shipped as incremental commits was re-landed in `v0.1.6` as reviewed
> pull requests (#126, #130, #131), and `v0.1.6` is the line that main carries.
> The entries below cover what `v0.1.6` adds on top of that shared base.

### Added

- Read-only Telegram chat for the CEO agent, so an owner can ask about hotel
  state from Telegram without granting any write access to the property data
- Hourly CEO digest scheduler that sends recurring summaries to the Telegram
  chat it was last configured from
- Phase 1 verification gate that agent-initiated actions must pass before they
  are allowed to run

### Changed

- Core PMS boundaries are now written down as explicit guardrails in
  [docs/architecture/core-pms-boundaries.md](docs/architecture/core-pms-boundaries.md),
  defining what the property-management core owns and what agent features may
  not reach into

### Fixed

- Telegram polling now keeps its update offset when a later update in the same
  batch fails, so messages are no longer skipped or processed twice

### Security

- CEO digest command intent is sanitized before it is acted on, so digest
  content cannot smuggle in an instruction the agent would execute
- Digest configuration is held behind an explicit gate, and review follow-ups
  on the digest scheduler's permission handling were addressed

## [0.1.5] - 2026-05-06

### Added

- Checkout settlement flow that closes out a stay's outstanding balance at
  departure
- Detail drawer on the dashboard activity feed for inspecting an entry without
  leaving the dashboard
- Scheduled automatic backups with a retention policy, plus an in-app alert
  when a backup fails instead of failing silently
- Weekly restore-drill script that verifies existing backups can actually be
  restored, rather than assuming they are good
- Privacy-first crash reporting
- Command recovery queue, with operator actions to inspect a stuck write and
  retry or dismiss it
- Read-only observer event stream for MCP clients
- CEO agent groundwork: safety schema, session/audit stores, and a cloud data
  opt-in setting; the agent runtime itself remains disabled in this release

### Changed

- Money is stored and computed as integer VND across pricing, folios, and
  invoices, removing floating-point rounding drift from monetary totals
- Reservation, stay, folio, payment, group service, and invoice writes all run
  through one write-command executor with idempotency keys, so a retried or
  duplicated request no longer creates duplicate records
- Pricing rules were consolidated onto a single shared pipeline instead of
  several parallel paths
- Frontend and backend now report errors in one standard shape, carrying
  correlation IDs across stay, group, and audit flows so a failure can be traced
  end to end

### Fixed

- Concurrent edits to the same booking are guarded, and a stale lease refresh
  now fails instead of quietly proceeding on outdated state
- Dashboard activity entries and reservation dates display correctly
- Dismissed recovery follow-up actions stay dismissed
- Outbox no longer treats unrelated records as identifier matches

### Security

- Supervised MCP writes must pass a verification gate before they are applied
- Agent stores redact sensitive values and normalize secret key names, keeping
  credentials out of agent metadata and audit records
- Blank agent actor IDs are rejected rather than recorded as an anonymous actor

## [0.1.4] - 2026-04-19

### Added

- Best-effort Apple Silicon macOS builds: a `.dmg` for manual installation and
  an `.app.tar.gz` consumed only by the in-app updater. These builds are not
  Apple-notarized, so macOS may need to be told to open them on first launch.

### Changed

- Setup bootstrap was split into separate status-read and provisioning
  services; behavior is unchanged

### Fixed

- MCP HTTP configuration and tool behavior now agree, so a gateway configured
  over HTTP exposes the tools it claims to
- Check-in rules saved by earlier versions survive settings hydration

## [0.1.3] - 2026-04-19

### Fixed

- Releases publish the complete set of platform installers; the previous
  release was missing assets for some platforms

## [0.1.2] - 2026-04-19

### Changed

- Backup subsystem split into separate shared-types, storage, runner, and
  coordinator modules; behavior is unchanged

### Fixed

- Release builds produce macOS Intel artifacts again, after moving to a
  CI runner image that is still supported

## [0.1.1] - 2026-04-18

### Added

- In-app auto-update flow, backed by a release pipeline that publishes signed
  update artifacts
- Automatic local filesystem backups: SQLite snapshots taken on a schedule,
  with validated filenames and pruning of old snapshots

### Fixed

- Weekend pricing uplift no longer counts the checkout day as an occupied
  night, which had been overcharging weekend stays by one night's uplift
- Same-day weekend stays, such as an hourly Saturday booking, receive the
  weekend uplift again
- Invalid date/time input is rejected with an explicit error instead of falling
  back to the current time, which could silently produce a zero-price booking

### Security

- All MCP gateway `/mcp` routes require a valid API key. Requests are only
  allowed through without one while no keys have been configured yet, so
  initial setup still works
- Gateway authentication is enforced once setup completes, closing a window
  where a fresh install stayed reachable without a key
- Saving app settings requires an admin session, preventing an unauthenticated
  caller from overwriting the app lock, setup state, or default user
- Re-running onboarding against an installation that already completed setup is
  rejected; previously this wiped all live data
- Creating, confirming, cancelling, and modifying reservations require a
  logged-in session

## [0.1.0] - 2026-04-17

Initial public open-source release.

### Added

- CapyInn, an offline-first desktop property-management app for mini hotels and
  guesthouses, built on Tauri 2, React, and a local SQLite database, covering
  room setup, guest intake, reservations, check-in/check-out, nightly pricing,
  housekeeping, and end-of-day reconciliation
- Signed multi-platform release builds produced by a GitHub release workflow

### Changed

- Renamed the application from MHM to CapyInn. Runtime data now lives under
  `~/CapyInn`; existing data under `~/MHM` is **not** migrated automatically
- README rewritten English-first, with a refreshed Vietnamese version
- Booking, reservation, and pricing code reorganized into domain services
  (guest, group lifecycle, billing, reporting) with transaction-aware pricing;
  behavior is unchanged

### Fixed

- Reservations search is wired up and works; a dead timeline view was removed
- Reservation lifecycle invariants and date coherence are enforced
- Removed a stray room-transfer command registration
- Event listener cleanup and store loading no longer leave stale UI state
- Group checkout batches its queries instead of querying per room
- Startup fails immediately with a clear error when the home directory cannot
  be resolved, rather than misbehaving later
- Database migrations fail loudly instead of continuing past a failed step

### Security

- Privileged backup and gateway actions are gated behind an admin check
- Hardened gateway lifecycle handling and lockfile recovery
- Tightened release metadata and the app's Content Security Policy

[Unreleased]: https://github.com/chuanman2707/CapyInn/compare/v0.1.6...main
[0.1.6]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.6
[0.1.5]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.5
[0.1.4]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.4
[0.1.3]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.3
[0.1.2]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.2
[0.1.1]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.1
[0.1.0]: https://github.com/chuanman2707/CapyInn/releases/tag/v0.1.0
