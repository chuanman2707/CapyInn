# Frontend Invoke Wrapper Design

Date: 2026-05-14

Issue: #140

Planned PR title: `frontend: normalize PMS invoke wrapper usage`

## Goal

Normalize obvious frontend PMS write calls through the existing Tauri invocation wrapper without changing backend command names, business request fields, response shapes, or frontend store architecture.

The work should make raw `invoke` usage easier to audit. PMS business writes should go through `invokeWriteCommand` when practical. Raw `invoke` may remain for system, runtime, export, diagnostics, gateway, bootstrap, and other non-PMS or low-risk calls when there is a clear reason.

Issue #140 says not to change request/response payloads. This spec interprets that as: do not change business request fields, command names, response shapes, or UI behavior. Converted writes may add only the wrapper metadata that `invokeWriteCommand` already owns, currently `idempotencyKey` and optional `correlationId`, and only after the compatibility checks below show the command can tolerate that metadata. If a command cannot tolerate wrapper metadata, it must stay raw in Batch 1 and be recorded as a follow-up rather than forcing a backend change into this frontend batch.

## Scope

In scope:

- Convert clear raw frontend PMS writes to `invokeWriteCommand`.
- Preserve business payload fields and command names at each converted call site.
- Keep `invokeWriteCommand` as the canonical place that adds `idempotencyKey`, optional `correlationId`, app-error normalization, and monitored command failure capture.
- Add a static frontend guardrail test so remaining raw `invoke` calls are intentional and explainable.
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

That list is descriptive, not scope expansion. Existing wrapper users such as CEO agent settings are not Batch 1 candidates unless they are already touched by the explicit tasks below.

The remaining raw `invoke` usage contains a mix of reads, system/runtime calls, exports, diagnostics, gateway calls, and a small number of obvious PMS writes.

## PMS Safety Boundary

The frontend wrapper is not the full PMS command boundary described in `AGENTS.md`. It can supply or forward frontend metadata, but backend command handling is responsible for actor resolution, command name persistence, canonical payload hashing, timestamping, request context, authorization, locking, mutation, audit, outbox writes, and transactionality.

For backend commands that already use `WriteCommandContext` or an equivalent backend command executor, `invokeWriteCommand` participates in that boundary by supplying an idempotency key and optional correlation id. For legacy backend commands that do not consume wrapper metadata, this batch may normalize the frontend call only if the command is invocation-compatible, but it must not claim the command is fully PMS-safety-compliant. Any missing backend command-boundary work is a follow-up outside #140 Batch 1.

## Invocation Categories

### PMS business writes

PMS business writes are commands that mutate hotel operational state or persistent settings exposed as PMS configuration. These should use `invokeWriteCommand` when the backend command can safely accept the wrapper's added `idempotencyKey`.

Batch 1 candidates:

- `save_pricing_rule` in `mhm/src/pages/settings/PricingSection.tsx`.
- `save_settings` for `checkin_rules` in `mhm/src/pages/settings/CheckinRulesSection.tsx`.
- `save_settings` for `hotel_info` in `mhm/src/pages/settings/HotelInfoSection.tsx`.
- `update_housekeeping` in `mhm/src/stores/useHotelStore.ts`.

These calls currently use raw `invoke` and are direct writes. The implementation must keep their existing business payload fields intact.

Before converting any candidate, implementation must record compatibility evidence in the implementation notes or final summary:

| Candidate | Business fields that must remain unchanged | Compatibility check | If incompatible |
| --- | --- | --- | --- |
| `save_pricing_rule` | `roomType`, `hourlyRate`, `overnightRate`, `dailyRate`, `earlyPct`, `latePct`, `weekendPct` | Inspect the Rust `#[tauri::command]` signature and run focused frontend tests proving the converted call routes those fields through `invokeWriteCommand`. The existing wrapper tests prove wrapper metadata is added before the low-level Tauri invoke. | Leave raw, keep the test/guard documenting why, and create a follow-up note for backend command-boundary support. |
| `save_settings` for `checkin_rules` | `key`, `value` | Inspect the Rust `save_settings` signature and run focused settings tests proving both fields route through `invokeWriteCommand`. The existing wrapper tests prove wrapper metadata is added before the low-level Tauri invoke. | Leave raw for this key and record why. |
| `save_settings` for `hotel_info` | `key`, `value` | Inspect the Rust `save_settings` signature and run focused settings tests proving both fields route through `invokeWriteCommand`. The existing wrapper tests prove wrapper metadata is added before the low-level Tauri invoke. | Leave raw for this key and record why. |
| `update_housekeeping` | `taskId`, `newStatus`, `note` | Inspect the Rust `update_housekeeping` signature and run focused store tests proving those fields route through `invokeWriteCommand`. The existing wrapper tests prove wrapper metadata is added before the low-level Tauri invoke. | Leave raw and record why. |

The compatibility check has two levels:

- Invocation compatibility: the call still succeeds with wrapper metadata in the command argument object.
- PMS safety completeness: the backend consumes and persists the metadata as part of an explicit command boundary.

Batch 1 requires invocation compatibility for conversion. It does not require PMS safety completeness for legacy commands, but it must identify when that completeness is missing.

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

This batch must add one required validation mechanism: a static guardrail test at `mhm/tests/frontend-invoke-wrapper-guardrails.test.ts`.

The test should scan frontend source files for raw Tauri `invoke` calls and enforce two explicit lists:

- `PMS_WRITE_COMMANDS_REQUIRING_WRAPPER`: Batch 1 commands that must not appear as raw `invoke`.
- `RAW_INVOKE_ALLOWED_COMMANDS`: read/system/runtime/export/diagnostics/gateway/bootstrap commands allowed to remain raw, each with an inline reason string in the test data.

It should not force allowed calls through `invokeWriteCommand`.

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

For legacy backend commands, wrapper metadata may be accepted by the invocation layer without being consumed by backend safety tables. The implementation summary must distinguish those two cases.

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

Per-candidate evidence:

- `save_pricing_rule`: add or update a `PricingSection` test that performs a successful save and expects `invokeWriteCommand("save_pricing_rule", { ...business fields... })`; also assert raw `invoke` is not called with `save_pricing_rule`.
- `save_settings` for `hotel_info`: add or update a settings component test that clicks the hotel-info save path and expects `invokeWriteCommand("save_settings", { key: "hotel_info", value: ... })`.
- `save_settings` for `checkin_rules`: add or update a settings component test that clicks the check-in-rules save path and expects `invokeWriteCommand("save_settings", { key: "checkin_rules", value: ... })`.
- `update_housekeeping`: add or update a store test that expects `invokeWriteCommand("update_housekeeping", { taskId, newStatus, note })` and confirms raw `invoke` is not used for that write.
- Raw invoke guardrail: add `mhm/tests/frontend-invoke-wrapper-guardrails.test.ts` with explicit allow/deny command lists and reasons.

Wrapper metadata evidence remains in `mhm/src/lib/invokeCommand.test.ts`; call-site tests should not duplicate the wrapper unit test by asserting the generated random idempotency value.

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

- Each Batch 1 candidate has recorded compatibility evidence before conversion.
- If compatible, `save_pricing_rule` no longer uses raw frontend `invoke`, preserves `roomType`, `hourlyRate`, `overnightRate`, `dailyRate`, `earlyPct`, `latePct`, and `weekendPct`, and has focused test evidence for wrapper usage.
- If compatible, `save_settings` for `hotel_info` no longer uses raw frontend `invoke`, preserves `key` and `value`, and has focused test evidence for wrapper usage.
- If compatible, `save_settings` for `checkin_rules` no longer uses raw frontend `invoke`, preserves `key` and `value`, and has focused test evidence for wrapper usage.
- If compatible, `update_housekeeping` no longer uses raw frontend `invoke`, preserves `taskId`, `newStatus`, and `note`, and has focused test evidence for wrapper usage.
- Any incompatible candidate remains raw with a documented reason and follow-up; no backend command change is introduced to force compatibility.
- Converted write payloads preserve their existing business fields.
- `mhm/tests/frontend-invoke-wrapper-guardrails.test.ts` enforces forbidden raw PMS write commands and allowed raw read/system/runtime/export/diagnostics/gateway/bootstrap commands with reason strings.
- Raw `invoke` remains only where it is read-only or intentionally system/runtime/export/diagnostics/gateway/bootstrap oriented according to the guardrail test.
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
