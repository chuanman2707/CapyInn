# CapyInn Crash Reporting Design

## Summary

CapyInn will add privacy-first crash reporting for severe application failures only. The first release will not track product usage, screen views, feature adoption, or normal validation errors.

The selected approach combines:

- local crash bundle capture on the user's machine
- optional Sentry submission only after explicit user confirmation
- a persistent `Send crash reports` setting in the app, defaulting to off

This gives the product a minimal diagnostic path without violating the expectations of a local-first desktop PMS handling sensitive guest data.

## Goals

- Capture enough information to debug real crashes and unhandled failures.
- Avoid silent telemetry and avoid collecting product analytics.
- Keep guest and hotel operational data out of remote error reports.
- Preserve a clean upgrade path to broader technical error reporting later if support volume requires it.

## Non-Goals

- No usage analytics.
- No session replay, screenshots, DOM snapshots, or breadcrumbs containing workflow history.
- No automatic reporting of validation errors, user mistakes, or routine business-rule errors.
- No automatic upload of raw local logs or local database content.

## Chosen Approach

Phase 1 will support only:

- Rust panics
- JavaScript unhandled exceptions
- unhandled promise rejections

When one of these events occurs, the app writes a local crash bundle first. On the next successful launch, the app checks for a pending bundle and asks the user whether they want to send the sanitized report to Sentry.

If the user accepts:

- the app sends the sanitized summary to Sentry
- the app enables `Send crash reports` for future reports
- the sent local bundle is marked handled or removed

If the user declines:

- nothing is sent remotely
- the setting remains off
- the local bundle can still be exported later for manual support

## Privacy Model

CapyInn is a local-first property management app that may process highly sensitive information, including guest identity details and hotel operational records. Because of that, remote reporting must be strictly limited.

Remote reports may contain only:

- error category such as `rust_panic`, `js_unhandled_error`, or `unhandled_rejection`
- crash timestamp
- app version and release channel
- OS, platform, and architecture
- sanitized stack trace
- high-level module name when safely derivable, such as `Reservations`, `Checkin`, `OCR`, `Settings`, or `Onboarding`
- anonymous installation identifier

Remote reports must never contain:

- guest names
- national ID or passport numbers
- phone numbers
- addresses
- raw OCR text
- booking payloads
- invoice payloads
- hotel settings values
- SQL queries with parameters
- screenshots or scanned images
- copied application state dumps

## Local Crash Bundle

Each crash bundle should be written to the app runtime area under a dedicated diagnostics directory. The bundle is the source of truth after a crash and must be created before any network attempt.

Each local bundle should include:

- bundle identifier
- crash type
- timestamp
- app version metadata
- platform metadata
- sanitized error message
- sanitized stack trace
- optional high-level module hint
- pending submission status

The bundle should be split conceptually into:

- safe summary: eligible for Sentry submission after consent
- local-only detail: optional extra context kept on disk for manual support export, but never auto-uploaded

## Sanitization Rules

Before any data is stored for remote submission, the reporting layer must sanitize:

- likely identity strings such as phone-number-like and ID-number-like sequences
- absolute file paths under the user runtime directory, converting them to generalized placeholders like `<runtime>/...`
- oversized serialized objects
- accidental state dumps

The app should keep stack traces and messages only in the minimum form needed to identify the fault location.

## Consent and UI

The Settings page will include a new control:

- `Send crash reports`

Behavior:

- default value is off
- description clearly states that reports are only for severe crashes, are sanitized, and do not track usage behavior

After the app restarts following a detected crash, it should show a modal dialog if a pending bundle exists.

The dialog should:

- explain that the app encountered a serious error in the previous session
- explain that the report is anonymous and sanitized
- explicitly state that guest data and usage analytics are not included

The dialog actions should be:

- `Send report`
- `Don't send`
- `Export report`

`Send report` submits the sanitized summary to Sentry and turns on the setting for future crash reports. `Don't send` leaves reporting disabled. `Export report` writes the local bundle to a user-visible file for manual support sharing.

Consent should not be requested during onboarding. The user should only see this choice in Settings or when there is an actual crash to report.

## Architecture

The implementation should be organized around a small reporting adapter instead of scattering Sentry calls through UI code.

Recommended responsibilities:

- frontend crash capture entrypoints for unhandled JS failures
- backend crash capture entrypoints for Rust panics where feasible
- local bundle writer
- sanitization layer
- optional Sentry transport
- settings integration
- post-crash recovery check on startup

This keeps phase 1 small while leaving a stable place to extend into selected technical error reporting later.

## Future Expansion Path

If real-world support later shows that crash-only reporting is insufficient, phase 2 may expand to a narrow whitelist of severe technical failures such as:

- database initialization failure
- migration failure
- OCR engine startup failure
- core invoke failure on critical commands

That future phase must still reuse the same consent model and sanitization pipeline. It must not introduce product analytics by default.

## Verification Strategy

The implementation should be validated with tests for:

- crash bundle creation
- sanitization of known sensitive patterns
- startup detection of pending crash bundles
- settings persistence for `Send crash reports`
- consent dialog behavior
- remote submission disabled when the setting is off
- export flow without remote submission

Manual verification should also confirm:

- no usage events are emitted
- no guest-related values appear in serialized crash payloads
- the app behaves correctly when Sentry configuration is absent or network submission fails

## Rollout

Roll out crash reporting as an explicitly documented diagnostic feature.

Required product communication:

- Settings copy describing what is and is not sent
- README note describing privacy boundaries
- release note entry when the feature ships

This feature should be presented as optional crash reporting for debugging severe failures, not as telemetry.
