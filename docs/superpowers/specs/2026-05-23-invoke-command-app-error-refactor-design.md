# Invoke Command and App Error Refactor Design

## Context

`mhm/src/lib/invokeCommand.ts` and `mhm/src/lib/appError.ts` are small files with high impact. GitNexus reports `invokeCommand`, `invokeWriteCommand`, `normalizeAppError`, `formatAppError`, and `createAppErrorException` as critical symbols because they sit on common frontend command and error-handling paths.

The first refactor pass will preserve every public export and call site. The goal is to reduce hidden coupling and make each behavior easier to test without changing command payloads, idempotency keys, monitoring behavior, or displayed error messages.

## Goals

- Keep the public APIs exported by `invokeCommand.ts` and `appError.ts` unchanged.
- Split command invocation and app error behavior into focused internal modules.
- Preserve current runtime behavior exactly for payload shaping, idempotency key generation, correlation IDs, error normalization, error formatting, exception shape, and monitoring capture.
- Keep PMS write safety intact: write calls still go through `invokeWriteCommand`, and monitoring context never enters the Tauri payload.
- Add focused tests around the public contracts and critical internal helper boundaries where useful.

## Non-Goals

- Do not migrate existing raw `invoke` read or system calls.
- Do not change backend command names, payload field names, or response shapes.
- Do not change the shared error registry format.
- Do not add a command-client framework or dependency-injection layer.
- Do not change user-facing Vietnamese error text.

## Architecture

The existing files remain the public facade:

- `mhm/src/lib/invokeCommand.ts` exports `createIdempotencyKey`, `invokeCommand`, and `invokeWriteCommand`.
- `mhm/src/lib/appError.ts` exports existing app error types, constants, and helpers.

The implementation moves behind those facades into focused modules. This keeps current import paths stable while making each behavior independently understandable and testable.

The command facade delegates to internal helpers for idempotency key generation, payload shaping, Tauri invocation, monitoring capture, and exception wrapping. The app error facade delegates to internal helpers for registry lookup, normalization, formatting, and exception creation.

## Components

- `mhm/src/lib/appError.ts`: public facade that re-exports the current error contract.
- `mhm/src/lib/appError/types.ts`: `AppErrorKind`, `AppError`, `AppErrorRegistryEntry`, and `NormalizedAppErrorException`.
- `mhm/src/lib/appError/registry.ts`: shared error-code registry loading, immutable registry exports, fallback constants, `getAppErrorDefinition`, and `isKnownAppErrorCode`.
- `mhm/src/lib/appError/normalize.ts`: `normalizeAppError` and private shape guards.
- `mhm/src/lib/appError/format.ts`: `formatAppError` and private correlation-id extraction.
- `mhm/src/lib/appError/exception.ts`: `createAppErrorException`.
- `mhm/src/lib/command/idempotency.ts`: `createIdempotencyKey`.
- `mhm/src/lib/command/payload.ts`: command payload construction with optional `correlationId`.
- `mhm/src/lib/command/invoke.ts`: internal implementation of `invokeCommand` and `invokeWriteCommand`.

## Data Flow

Read command flow:

1. Caller invokes `invokeCommand(command, args, options)`.
2. Payload helper returns `args` unchanged when `options.correlationId` is absent.
3. If `correlationId` exists, payload helper returns `{ ...(args ?? {}), correlationId }`.
4. Tauri `invoke` runs with that payload.
5. On success, the response is returned unchanged.

Write command flow:

1. Caller invokes `invokeWriteCommand(command, args, options)`.
2. Idempotency helper creates `${command}:${random}`.
3. Write wrapper adds `idempotencyKey` into args.
4. The same command invoke flow runs.

Failure flow:

1. Raw thrown value is normalized into `AppError`.
2. `captureCommandFailure` is called fire-and-forget with command, normalized error, correlation id, and monitoring context.
3. Capture rejection is swallowed.
4. The public wrapper throws an `AppError`-named `Error` with current fields and `cause`.

## Error Handling

Current error semantics are preserved:

- Unknown, malformed, non-object, or unregistered backend errors normalize to `FALLBACK_SYSTEM_APP_ERROR`.
- Known user errors preserve backend `message`, `code`, `kind`, and `support_id`.
- Known system errors preserve `support_id`.
- `formatAppError` appends support id only for system errors with `support_id`.
- `formatAppError` appends `Mã theo dõi` when the input carries `correlation_id`.
- `createAppErrorException` keeps `name = "AppError"` plus `code`, `kind`, `support_id`, optional `correlation_id`, and optional `cause`.
- Monitoring capture remains best-effort and never masks the original command failure.

## Testing

The public facade tests remain the source of truth:

- `mhm/src/lib/appError.test.ts` covers registry alignment, fallback behavior, formatting, and exception shape.
- `mhm/src/lib/invokeCommand.test.ts` covers payload shaping, correlation id injection, idempotency key injection, monitoring context isolation, and structured error rethrow behavior.
- `mhm/tests/frontend-invoke-wrapper-guardrails.test.ts` continues to verify the central wrapper boundary.

Targeted internal helper tests may be added if they clarify behavior without coupling tests to implementation details.

Verification for implementation should include:

- Targeted Vitest for `appError.test.ts`, `invokeCommand.test.ts`, and `frontend-invoke-wrapper-guardrails.test.ts`.
- Broader frontend test/build checks if the implementation touches exports or import boundaries beyond the planned facades.
- `gitnexus_detect_changes` before committing implementation changes.

## Risks and Mitigations

- Risk: Changing payload identity when no correlation id is present could break tests or subtle call behavior. Mitigation: keep and test pass-through behavior.
- Risk: Moving registry exports could change object identity or immutability assumptions. Mitigation: preserve frozen public constants and registry alignment tests.
- Risk: Monitoring errors could leak or mask original failures. Mitigation: keep fire-and-forget capture and swallow capture rejection.
- Risk: Existing dirty worktree state could be mixed into this refactor. Mitigation: avoid touching unrelated dirty files, including `mhm/src/stores/useHotelStore.test.ts`.

## Approved Direction

Use the facade plus small internal modules approach. Keep all call sites and public imports stable for the first pass. Do not migrate raw read/system `invoke` calls in this refactor.
