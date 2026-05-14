# Frontend Invoke Wrapper Design

Date: 2026-05-14

Issue: #140

Planned PR title: `frontend: normalize PMS invoke wrapper usage`

## Goal

Normalize obvious frontend PMS write calls through the existing Tauri invocation wrapper without changing backend command names, business request payloads, response shapes, or frontend store architecture.

The work should make raw `invoke` usage easier to audit. PMS business writes should go through `invokeWriteCommand` when practical. Raw `invoke` may remain for system, runtime, export, diagnostics, gateway, bootstrap, and other non-PMS or low-risk calls when there is a clear reason.

## Scope

In scope:

- Convert clear raw frontend PMS writes to `invokeWriteCommand`.
- Preserve business payload fields and command names at each converted call site.
- Keep `invokeWriteCommand` as the canonical place that adds `idempotencyKey`, optional `correlationId`, app-error normalization, and monitored command failure capture.
- Add lightweight guard coverage or documentation so remaining raw `invoke` calls are intentional and explainable.
- Update focused frontend tests for converted call sites where the existing test surface can verify wrapper usage and payload shape.
- Validate with `npm test`, `npm run build`, and an `rg` scan for remaining raw invokes.

Out of scope:

- Backend command renames.
- Backend request or response schema changes.
- Zustand or page architecture rewrites.
- A full migration of every read command.
- A full migration of writes that already use `invokeCommand` but may not safely accept an added idempotency field yet.
- Runtime/system invoke cleanup for crash reporting, gateway, bootstrap, update, backup, export, or diagnostics flows.

## Current State

`mhm/src/lib/invokeCommand.ts` already provides the intended boundary:

- `invokeCommand` wraps Tauri `invoke`, merges an optional `correlationId`, normalizes app errors, and sends command failure monitoring for monitored commands.
- `invokeWriteCommand` calls `invokeCommand` after adding a command-scoped `idempotencyKey`.
- `createIdempotencyKey` formats keys as `command:<random>`.

Several frontend PMS writes already use `invokeWriteCommand`, including reservation confirmation/cancel, reservation create/modify, check-in, check-out, group checkout, group services, invoice generation, and CEO agent settings.

The remaining raw `invoke` usage contains a mix of reads, system/runtime calls, exports, diagnostics, gateway calls, and a small number of obvious PMS writes.

## Invocation Categories

### PMS business writes

PMS business writes are commands that mutate hotel operational state or persistent settings exposed as PMS configuration. These should use `invokeWriteCommand` when the backend command can safely accept the wrapper's added `idempotencyKey`.

Batch 1 candidates:

- `save_pricing_rule` in `mhm/src/pages/settings/PricingSection.tsx`.
- `save_settings` for `checkin_rules` in `mhm/src/pages/settings/CheckinRulesSection.tsx`.
- `save_settings` for `hotel_info` in `mhm/src/pages/settings/HotelInfoSection.tsx`.
- `update_housekeeping` in `mhm/src/stores/useHotelStore.ts`.

These calls currently use raw `invoke` and are direct writes. The implementation must keep their existing business payload fields intact.

### Read calls

Read calls may remain raw `invoke` in this batch unless the file being changed benefits from using `invokeCommand` for local consistency. This avoids turning #140 Batch 1 into a broad frontend cleanup.

Examples that can stay raw in this batch include dashboard reads, analytics reads, guest searches, availability checks, room detail reads, and other command calls that only retrieve data.

### System and runtime calls

System/runtime calls may remain raw `invoke` because they are not PMS business writes and often sit below or beside the business command boundary.

Allowed examples include:

- crash reporting lifecycle commands,
- JavaScript crash recording,
- pending crash report export and submission state,
- gateway status and key generation,
- bootstrap status,
- backup and CSV export commands,
- update/runtime support commands.

This batch should make the reason for these remaining raw invokes clear through focused tests, a small classification helper used by tests, or concise local documentation. It should not force these calls through `invokeWriteCommand`.

## Architecture

The existing wrapper remains the only frontend abstraction:

```ts
await invokeWriteCommand(commandName, businessArgs, options);
```

The implementation should not introduce a second command client, a command registry, or generated command API. The issue is a normalization pass, not a new architecture layer.

Converted call sites should follow the existing local style:

- import `invokeWriteCommand` from `@/lib/invokeCommand`;
- keep the command string unchanged;
- keep the business argument object unchanged except for the wrapper-added metadata;
- keep UI refresh and toast behavior in the same order;
- use `formatAppError(error)` when the edited file already uses app-error formatting or when the conversion makes normalized errors available without broad UI behavior changes.

## Data Flow

For converted writes, the data flow is:

1. UI/store validates or prepares the same business fields it already sends today.
2. UI/store calls `invokeWriteCommand`.
3. `invokeWriteCommand` adds `idempotencyKey`.
4. `invokeCommand` optionally adds `correlationId`.
5. Tauri receives the same command name and the original business fields plus wrapper metadata.
6. Success handling, refreshes, and toasts continue as before.
7. Errors are normalized through `normalizeAppError` and thrown as `AppError` exceptions.

No caller should manually create an idempotency key for the converted Batch 1 calls. Manual `createIdempotencyKey` usage should be left alone unless it is part of an explicitly converted call.

## Error Handling

Converted writes should route backend and Tauri failures through `invokeCommand` error normalization. This gives callers a normalized exception with:

- app error code,
- message,
- kind,
- optional support id,
- optional correlation id,
- original cause.

UI behavior should stay close to today. If a component currently displays a simple generic error toast and the conversion enables `formatAppError`, the implementation may improve that one local toast without changing surrounding flows.

Command failure monitoring remains limited to commands listed in `mhm/src/lib/crashReporting/commandFailure.ts`. Batch 1 does not expand the monitored command list unless a converted command already has monitoring requirements in existing code.

## GitNexus Guardrails

Before editing any function, class, or method, implementation must run GitNexus impact analysis for the target symbol and report:

- direct callers,
- affected processes,
- risk level.

If impact analysis reports HIGH or CRITICAL risk, implementation must pause and warn before editing.

Before committing implementation changes, implementation must run `gitnexus_detect_changes()` to verify the affected symbols and flows match the planned scope.

## Testing

Focused test updates should verify the converted write paths at the wrapper boundary where practical:

- converted calls still send the same business fields,
- converted calls include a command-scoped `idempotencyKey`,
- existing success refresh/toast behavior remains intact,
- normalized errors are displayed through existing UI error paths where tests cover them.

Validation commands:

```bash
cd mhm && npm test
cd mhm && npm run build
rg -n "invoke<|invoke\\(" mhm/src
```

Expected raw invoke scan result:

- no remaining raw `invoke` for the Batch 1 PMS write candidates,
- remaining raw invokes are reads or documented system/runtime/export/diagnostics/gateway/bootstrap calls,
- `mhm/src/lib/invokeCommand.ts` remains the low-level wrapper that directly calls Tauri `invoke`.

## Acceptance Criteria

- `save_pricing_rule`, settings saves for hotel info and check-in rules, and housekeeping updates no longer use raw frontend `invoke` if backend argument handling is compatible.
- Converted write payloads preserve their existing business fields.
- Raw `invoke` remains only where it is read-only or intentionally system/runtime/export/diagnostics/gateway/bootstrap oriented.
- No backend command names, backend payload semantics, response shapes, or Zustand architecture are changed.
- Tests and build pass.
- The final `rg` scan is reviewed and remaining raw invoke usage is explainable.

## Risks And Mitigations

Risk: adding `idempotencyKey` to a backend command that does not accept extra arguments could break the call.

Mitigation: confirm compatibility before conversion. If a command cannot safely accept wrapper metadata without backend changes, leave it raw in Batch 1 and document it as a follow-up instead of expanding scope.

Risk: converting too many call sites turns a narrow refactor into an architecture sweep.

Mitigation: limit Batch 1 to obvious raw PMS writes and lightweight guard coverage. Defer writes currently using `invokeCommand` to a later batch.

Risk: raw invoke scan still shows many matches.

Mitigation: judge the scan by category, not by zero matches. Reads and system/runtime calls may remain raw under this design.
