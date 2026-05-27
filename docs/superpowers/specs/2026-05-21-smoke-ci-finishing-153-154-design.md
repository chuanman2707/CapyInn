# Smoke/CI Finishing Design (#153 + #154)

## Goal

Close the remaining smoke/CI finishing slice for:

- #153: add a canonical group booking smoke flow.
- #154: include core smoke flows in normal validation and add visible release criteria.

The combined slice is valid because #154 depends on reservation, stay, and group smoke coverage existing before the release checklist and normal validation story can be complete.

## Approved Approach

Use the smallest automated path:

1. Add one canonical backend group lifecycle smoke test.
2. Keep CI routed through the existing `npm run verify:full` path.
3. Add release checklist documentation and link it from existing release/contributor docs.

Do not add a new CI job or make the tag release workflow heavier unless implementation proves that `verify:full` does not clearly include the new smoke test.

## Current Context

The repository already has:

- `.github/workflows/ci.yml` with a `build-test` job and a `verify-wave1` job.
- `verify-wave1` runs `npm run verify:full` from `mhm`.
- `mhm/scripts/verify/full.mjs` runs `verify:quick`, frontend tests, Rust booking scenario tests, backup tests, and native Tauri smoke.
- `mhm/scripts/verify/full.mjs` currently includes the Rust filter `services::booking::tests::`, so a canonical group smoke test in that module is part of normal CI validation.
- Existing group lifecycle tests cover pieces of check-in, partial checkout, final checkout, idempotency, and group service behavior, but there is no single reviewer-facing canonical smoke named as the group booking lifecycle gate.

## Scope

Implementation may touch:

- `mhm/src-tauri/src/services/booking/tests.rs`
- `mhm/scripts/verify/full.mjs`, only if required for clear smoke inclusion
- `docs/release-checklist.md`
- `docs/release-signing.md`
- `README.md`

The implementation must not touch the currently dirty `mhm/src/stores/useHotelStore.test.ts` unless the user separately asks for that work.

## Non-Goals

- Do not expand group booking product behavior.
- Do not change invoice semantics.
- Do not rewrite `commands/groups.rs`.
- Do not require Telegram, OpenAI, MCP, gateway, watcher, or other experimental service configuration.
- Do not add a flaky native UI automation layer.
- Do not change release artifact publishing logic unless inspection proves documentation alone cannot satisfy #154.

## Group Smoke Flow

Add a backend service smoke test named clearly, for example:

`group_booking_lifecycle_smoke_covers_partial_and_final_checkout`

The test should reuse existing helpers such as `test_pool`, `seed_room`, `seed_pricing_rule`, and `minimal_group_checkin_request`.

Representative lifecycle:

1. Seed two rooms and a standard pricing rule.
2. Create a two-room group check-in.
3. Assert the group is `active`, both rooms are `occupied`, both bookings are `active`, and a master booking exists.
4. Check out the master or first selected booking.
5. Assert the group becomes `partial_checkout`, the selected booking is `checked_out`, housekeeping is created for the checked-out room, and a remaining active booking becomes or remains the master.
6. Check out the remaining active booking.
7. Assert the group becomes `completed`, no master booking remains, both bookings are checked out, and both rooms have left the occupied state according to existing checkout behavior.

The smoke test should assert only core lifecycle invariants. It should not assert incidental invoice formatting or UI text.

## CI And Validation Wiring

The preferred implementation is to keep `.github/workflows/ci.yml` unchanged because CI already runs `npm run verify:full`.

`verify:full` should continue to be the normal smoke gate. If the new group smoke test stays in `services::booking::tests`, the existing Rust booking scenario filter already includes it. If implementation places the test elsewhere, update `mhm/scripts/verify/full.mjs` with the narrowest additional Rust filter and a descriptive label so CI failures point to the group smoke flow.

No experimental service config should be required. The existing verification environment disables gateway and watcher through `scripts/verify/shared.mjs`; this behavior should remain unchanged.

## Release Checklist

Add a release checklist document that includes:

- baseline validation: frontend tests, frontend build, `cargo check`, `cargo test`, and clippy
- smoke validation: reservation, stay, group, backup, and native Tauri smoke through `npm run verify:full`
- core PMS profile: normal PMS must run without experimental services
- experimental disabled check: release validation must not require Telegram, OpenAI, MCP, gateway, watcher, or agent write configuration
- release workflow readiness: version alignment, updater signing key/variable presence, and release asset expectations already documented in `docs/release-signing.md`

Link the checklist from `docs/release-signing.md`. Add a README link if it improves discoverability without bloating the top-level contributing checklist.

## Error Handling And Follow-Ups

If the canonical group smoke exposes a real lifecycle bug, do not silently change business behavior in this slice. Record the bug as a separate follow-up unless the fix is small, directly required for the smoke test, and preserves existing documented semantics.

If an uncovered group path is identified during implementation, record it as a separate follow-up. #153 requires one representative group booking lifecycle smoke, not exhaustive group coverage.

## Acceptance Criteria

- One representative group booking lifecycle smoke test exists with a clear smoke-oriented name.
- The smoke covers group check-in, partial checkout, and final checkout.
- The smoke test is included in normal validation through `npm run verify:full`.
- CI remains independent from gateway, agent, Telegram, OpenAI, MCP, and watcher configuration.
- Release checklist documents baseline validation, smoke validation, core PMS profile, and experimental disabled criteria.
- Existing release docs link to the checklist.
- Any uncovered group path follow-up is recorded separately.

## Validation

Focused backend smoke:

```bash
cd mhm/src-tauri && cargo test group_booking_lifecycle_smoke
```

Normal validation:

```bash
cd mhm && npm run verify:full
```

Documentation and wiring check:

```bash
rg -n "reservation|stay|group|smoke|cargo test|npm test|release checklist|experimental disabled" .github/workflows docs mhm
```

Before committing implementation changes, run GitNexus change detection as required by the repository rules.

## Risk

Risk is medium for the backend smoke addition because it exercises core group lifecycle state transitions. The intended blast radius is limited to tests and validation wiring. Documentation risk is low. Any implementation that changes production group lifecycle code must run GitNexus impact analysis on the touched symbol before editing and must warn the user if the reported risk is high or critical.
