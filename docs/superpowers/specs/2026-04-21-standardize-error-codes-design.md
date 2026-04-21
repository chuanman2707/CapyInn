# CapyInn Standardize FE/BE Error Codes Design

## Summary

CapyInn will standardize frontend and backend error handling around one shared error contract at the Tauri command boundary.

From this point forward, the product should stop treating command failures as free-form strings that each screen interprets differently. Instead, backend commands will return a consistent error payload with:

- `code`
- `message`
- `kind`
- `support_id`

Phase 1 will focus on user-facing errors that operators actually need to understand on screen. Those errors will receive stable, domain-specific codes such as `AUTH_INVALID_PIN` or `GROUP_NOT_ENOUGH_VACANT_ROOMS`.

Unexpected lower-level failures such as database, filesystem, or runtime issues will still be debuggable, but they will no longer leak raw technical detail into the UI. The frontend will show a generic system message plus a `support_id`, while backend logging keeps the original root cause for diagnosis.

The rollout will start with shared infrastructure, then apply it to `auth`, `permission`, `room management`, and selected `booking/group` flows before expanding to the rest of the app.

## Goals

- Make user-visible errors consistent across screens instead of relying on ad hoc `String(err)` handling.
- Give predictable, stable error codes to the most common business and validation failures.
- Preserve support and debugging ability for system failures by attaching a `support_id`.
- Centralize frontend command-error parsing so screens stop implementing their own string handling.
- Create a reliable foundation for later work in issue `#30`, including crash dashboarding, monitoring, and error aggregation by code.

## Non-Goals

- No attempt to standardize every command and every failure path in one release.
- No attempt in Phase 1 to expose raw database, filesystem, or network failure details directly to operators.
- No full observability platform in this design. Crash dashboard, DB monitoring, command failure monitoring, and correlation IDs remain separate follow-up work under the parent roadmap.
- No rewrite of all existing UI error presentations in one sweep if a screen is outside the selected rollout domains.
- No guarantee that all old free-form backend errors disappear immediately; Phase 1 is intentionally incremental.

## Current Baseline

Current repository behavior is inconsistent at both layers.

Backend baseline:

- many public Tauri commands currently return `Result<_, String>`
- expected business failures and unexpected runtime failures are mixed together as plain strings
- several commands return custom Vietnamese business messages, while many others forward `sqlx`, filesystem, or runtime errors using `to_string()`

Frontend baseline:

- many screens call `invoke(...)` directly
- many screens display errors through `toast.error(String(err))` or similar one-off logic
- some screens swallow failures entirely and fall back to empty state
- there is no shared parser or normalizer for Tauri command failures

This means the same logical failure can be:

- shown with different wording on different screens
- hidden completely
- impossible to aggregate cleanly by type
- hard to distinguish from a real system failure

The crash-reporting work from PR `#27` is also a concrete example of why this matters: command failures around submit, cleanup, export, and retry behavior are still handled with local ad hoc logic instead of a shared contract.

## Chosen Approach

CapyInn will standardize errors at the backend command boundary rather than trying to patch the problem in each screen.

The selected approach has three parts:

1. Backend introduces one shared application error model.
2. Every migrated Tauri command converts failures into that shared model before rejecting to the frontend.
3. Frontend introduces one shared command wrapper that parses the rejection into a reusable app error object.

This is the preferred approach because it fixes the source of inconsistency instead of teaching each screen to decode fragile free-form strings.

Rejected alternatives:

- frontend-only mapping of existing strings:
  fast to start, but brittle and impossible to trust long term because backend wording changes would silently break error handling
- domain-by-domain formats with no shared envelope:
  easier to start, but likely to drift and recreate inconsistency under different names
- full-app big-bang migration:
  too much blast radius for a first rollout, given the current number of commands and call sites

## Standard Error Contract

Every migrated command failure should resolve to the same external shape:

- `code`: stable machine-readable identifier
- `message`: short user-facing message that is safe to show directly
- `kind`: one of `user` or `system`
- `support_id`: nullable reference ID for support/debug lookup

Expected behavior:

- `user` errors:
  use a domain-specific `code`
  use a clear message
  set `support_id` to `null`
- `system` errors:
  use a system-level `code`
  use a generic safe message
  include `support_id`

Phase 1 should keep system codes intentionally narrow. Unless a migrated flow has a strong reason to distinguish one infrastructure failure class from another, the default system code should be `SYSTEM_INTERNAL_ERROR`.

Example user-facing payloads:

```json
{
  "code": "AUTH_INVALID_PIN",
  "message": "Mã PIN không đúng",
  "kind": "user",
  "support_id": null
}
```

```json
{
  "code": "SYSTEM_INTERNAL_ERROR",
  "message": "Có lỗi hệ thống, vui lòng thử lại",
  "kind": "system",
  "support_id": "SUP-9J4K2Q"
}
```

Phase 1 removes that ambiguity for migrated commands: the backend command boundary must reject with one JSON-serialized error string produced by a shared backend helper. The frontend wrapper for migrated commands must parse that JSON string into the app-level error object. Migrated commands must not mix object-shaped and string-shaped rejections.

## Backend Error Model

### Shared Types

Backend should introduce a shared error module used by migrated command code.

The model should include:

- one internal app error type representing known business failures and wrapped system failures
- one serializable response shape matching the external contract
- one helper for generating `support_id` values for system failures

The internal model should keep enough structure to distinguish:

- validation or business-rule failures that deserve stable, specific codes
- permission failures
- authentication failures
- wrapped lower-level runtime failures

The internal model must preserve the original lower-level error for logging even when the UI receives only the generic system message.

### Mapping Rules

Mapping rules for migrated commands:

- if the failure is expected and meaningful to the operator:
  map it to a stable domain code
- if the failure comes from lower-level infrastructure:
  log the original error, generate `support_id`, map to a generic system response

Classification is based on business meaning, not the technical layer where the failure surfaced. If a known operator-facing condition is detected through a lower-level mechanism such as a database uniqueness violation, missing row, or invalid state discovered inside a service call, the command must reclassify that failure into the correct stable domain code instead of passing it through as a generic system failure.

Examples:

- invalid PIN:
  `AUTH_INVALID_PIN`
- missing current user:
  `AUTH_NOT_AUTHENTICATED`
- admin-only action denied:
  `AUTH_FORBIDDEN`
- room already exists:
  `ROOM_ALREADY_EXISTS`
- cannot delete occupied room:
  `ROOM_DELETE_OCCUPIED`
- requested room count exceeds vacant supply:
  `GROUP_NOT_ENOUGH_VACANT_ROOMS`
- unexpected DB lock or file rename failure:
  `SYSTEM_INTERNAL_ERROR` plus `support_id`

### Support ID Rules

`support_id` exists for traceability, not for user interpretation.

Rules:

- required for `system` errors in migrated flows
- always present in the external contract
- `null` for `user` errors
- unique enough for practical support lookup
- logged together with:
  command name
  domain error code
  underlying root cause
  relevant safe identifiers such as `room_id`, `booking_id`, or `group_id`

The UI should show the `support_id` as part of the generic system failure message whenever the value is non-null so support can use what the operator sees on screen.

## Error Code Naming

Phase 1 should use specific codes rather than broad category codes.

Naming rule:

- uppercase
- domain prefix first
- scenario second

Examples:

- `AUTH_INVALID_PIN`
- `AUTH_NOT_AUTHENTICATED`
- `AUTH_FORBIDDEN`
- `ROOM_ALREADY_EXISTS`
- `ROOM_NOT_FOUND`
- `ROOM_DELETE_OCCUPIED`
- `ROOM_DELETE_ACTIVE_BOOKING`
- `GROUP_INVALID_ROOM_COUNT`
- `GROUP_NOT_ENOUGH_VACANT_ROOMS`
- `BOOKING_NOT_FOUND`
- `BOOKING_INVALID_STATE`

Phase 1 should keep the code list curated instead of auto-generating codes from messages. New codes should be added deliberately when a migrated flow introduces a genuinely distinct operator-facing outcome.

### Canonical Registry

Phase 1 should keep one canonical checked-in registry of externally visible error codes at `mhm/shared/error-codes.json`.

Rules:

- every user-facing code emitted by backend must appear in that registry
- frontend branching logic should only use codes listed in that registry
- Rust and TypeScript may each define local helpers or types, but tests must fail if a code is emitted or consumed outside the registry

This keeps the code list curated in one place without requiring a full code-generation pipeline in the first rollout.

## Frontend Handling Model

### Shared Invoke Wrapper

Frontend should add one shared command wrapper that becomes the preferred path for migrated screens.

Its responsibilities:

- call Tauri commands
- normalize command rejection payloads into one frontend app error object
- return success payloads unchanged
- preserve `code`, `message`, `kind`, and `support_id` for callers

This wrapper should be the only place where the app knows how to interpret backend command failures.

If the wrapper receives a rejection that does not match the standard migrated contract, it must fall back to one safe synthesized app error instead of trying to infer meaning from raw string text. In Phase 1, that fallback should behave like a generic `system` error with `code = SYSTEM_INTERNAL_ERROR`, a generic safe message, and `support_id = null`. This fallback is only a safety net for accidental misuse, not the intended path for unmigrated commands.

### Shared Error Presentation

Frontend should also add a small reusable helper for deciding how to show errors.

Expected behavior:

- for `user` errors:
  show `message` directly
- for `system` errors:
  show generic message plus `support_id`

This still allows screen-specific behavior when needed.

Examples:

- login screen can use `AUTH_INVALID_PIN` to mark the form as invalid
- room management can use `ROOM_ALREADY_EXISTS` to keep the form open and highlight the duplication problem
- group booking can use `GROUP_NOT_ENOUGH_VACANT_ROOMS` to preserve the current input and ask the operator to reduce room count

The important rule is that screen-specific behavior must key off the normalized `code`, not raw error strings.

### Migration Rule

Migrated screens should stop doing any of the following directly:

- `toast.error(String(err))`
- `toast.error("prefix: " + err)`
- parsing raw command failure strings locally
- swallowing command failures without a deliberate reason

If a screen needs fallback handling for a command that has not been migrated yet, that fallback should remain local until its backend command joins the new contract.

The key rule is that migration happens per interaction, not per whole screen. A screen may temporarily contain both old and new command paths, but one user interaction must use exactly one strategy:

- migrated interaction:
  migrated backend command plus shared wrapper plus normalized code handling
- unmigrated interaction:
  legacy backend command plus local fallback handling

The project should avoid hybrid logic where the same interaction partly depends on standard codes and partly depends on legacy string parsing.

## Phase 1 Scope

Phase 1 covers the shared infrastructure plus a focused set of command domains.

### In Scope

- shared backend error module
- shared frontend command-error wrapper
- shared frontend error presentation helper
- migrated `auth` errors
- migrated permission errors
- migrated `room management` user-facing errors
- migrated selected `booking` and `group` user-facing errors

### Recommended First Commands

- `login`
- `list_users`
- `create_user`
- `create_room`
- `delete_room`
- `create_room_type`
- `auto_assign_rooms`
- selected `check_in`, `check_out`, `group_checkin`, and `group_checkout` failures that already have clear business meanings

### Explicitly Out of Scope for Phase 1

- full migration of all commands that still return `Result<_, String>`
- crash dashboard implementation
- command failure monitoring pipeline
- database error monitoring pipeline
- cross-request correlation IDs
- retrofitting every existing screen before the shared infrastructure is proven stable

## Rollout Plan

The rollout should be incremental.

### Step 1: Shared Infrastructure

Add the backend and frontend shared error pieces first without trying to convert the whole app.

Deliverables:

- shared backend error definitions and serializers
- shared frontend wrapper and app-error types
- shared UI helper for operator-safe error display

### Step 2: Authentication And Permission

Migrate the easiest and highest-signal flows first.

Target outcomes:

- in migrated auth flows, wrong PIN becomes `AUTH_INVALID_PIN`
- in migrated auth flows, missing session becomes `AUTH_NOT_AUTHENTICATED`
- in migrated permission-gated flows touched in Phase 1, admin-only denials become `AUTH_FORBIDDEN`

This gives a narrow first success case that is easy to test and verify.

### Step 3: Room Management

Migrate predictable room-management failures.

Target outcomes:

- duplicate room ID
- room missing
- delete blocked by occupancy
- delete blocked by active booking
- duplicate room type

These are good candidates because they already represent clear business conditions that operators understand.

### Step 4: Booking And Group Flows

Expand to booking and group operations with the clearest operator-facing failures first.

Target outcomes:

- invalid room count
- not enough vacant rooms
- invalid booking state for check-in or checkout
- missing booking or missing group references where applicable

### Step 5: Wider Adoption

Once the shared path is stable:

- replace direct `invoke(...)` use in migrated screens with the shared wrapper
- remove leftover string-based error display from those screens
- inventory remaining commands and screens for later waves

## Testing Strategy

### Backend Tests

Backend tests should assert that migrated failures map to the right external contract.

Required checks:

- known user-facing failures return the expected `code`
- known user-facing failures return the intended safe `message`
- system failures return `kind = system`
- system failures generate `support_id`
- raw infrastructure failure detail is not leaked into the external user message

### Frontend Tests

Frontend tests should assert that the wrapper and screens behave consistently.

Required checks:

- wrapper normalizes backend command failures into one reusable error shape
- `user` errors show the backend message directly
- `system` errors show generic message plus `support_id`
- migrated screens no longer rely on `String(err)` formatting

Priority migrated-screen tests:

- login screen
- room management flows
- selected group booking flows

### Repository Hygiene Checks

After each migration wave, the implementation should search for leftover patterns such as:

- `toast.error(String(`
- `String(err)`
- `String(error)`
- command-specific free-form parsing in components or stores

Those checks are important because a partial migration that leaves string parsing spread across the UI will recreate the original problem.

## Risks And Mitigations

### Risk: Mixed Old And New Error Paths Cause Confusion

If only part of the app uses the shared contract, teams may assume the whole app is already standardized when it is not.

Mitigation:

- clearly mark migrated flows
- convert selected vertical slices fully, not half way
- keep a tracked inventory of remaining old-style command paths

### Risk: Too Many Specific Codes Too Early

If every small variation gets a new code immediately, the list becomes noisy and hard to govern.

Mitigation:

- only create a new code when it represents a distinct operator action or decision
- keep generic system handling for unexpected low-level failures in Phase 1

### Risk: Frontend Starts Depending On Message Text Again

If screens branch on `message` instead of `code`, the standardization effort fails.

Mitigation:

- treat `code` as the only branching key
- treat `message` as display text only

### Risk: Support ID Exists But Is Not Logged With Enough Context

If `support_id` is shown to operators but logs do not capture the same ID with safe command context, support still cannot diagnose issues.

Mitigation:

- make `support_id` generation and logging part of the shared backend path, not an optional per-command behavior

## Success Criteria

Phase 1 is successful when all of the following are true:

- migrated commands stop rejecting with free-form strings
- migrated screens stop displaying raw `String(err)` output
- common operator-facing failures in `auth`, `room`, and selected `booking/group` flows show stable messages and stable codes
- system failures in migrated flows show a generic safe message plus `support_id`
- backend logs keep enough context to trace a displayed `support_id` back to the underlying failure
- the project has a credible base for later observability work under issue `#30`

## Implementation Exit State

At the end of this design's implementation plan, CapyInn should have:

- one shared backend error contract
- one shared frontend command-error wrapper
- one shared frontend error-display rule
- a proven first rollout across the selected high-value flows

This is intentionally the foundation layer for the larger stability and observability roadmap, not the entire roadmap itself.
