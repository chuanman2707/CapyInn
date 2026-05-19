# Issue 149: Command Lock-Key Builder Extraction Design

Date: 2026-05-18
Status: Approved for implementation planning

GitHub issue: <https://github.com/chuanman2707/CapyInn/issues/149>

## Purpose

Extract the command idempotency lock-key preparation internals into a focused module without changing the stable lock-key format, command serialization behavior, or high-risk write lock coverage.

This issue exists because lock keys are part of the PMS safety contract. A command that mutates currently covered resources such as bookings, rooms, folios, groups, and settings must keep using the same stable lock key strings so retries, duplicate detection, command recovery metadata, and aggregate locking continue to agree about the same resource.

## Problem

`mhm/src-tauri/src/command_idempotency.rs` currently prepares lock-key JSON in more than one place:

- initial command preparation runs the request lock-key deriver, sorts, deduplicates, and serializes `lock_keys_json`;
- resolved-guard refresh sorts, deduplicates, requires non-empty lock keys, and updates `command_idempotency.lock_keys_json`.

That behavior is correct but too easy to drift because it is embedded in the large executor file. The issue is not to invent a new lock format. The issue is to isolate the current behavior behind one internal command-idempotency boundary and add format tripwire tests.

## Goals

- Add a focused command idempotency lock-key helper module.
- Preserve exact lock-key strings and JSON output.
- Preserve the distinction between optional initial lock keys and required resolved-guard lock keys.
- Preserve existing command payload hashing, ledger intent serialization, summary serialization, replay behavior, and outbox behavior.
- Document the lock-key format in source comments or docs close to the helper module.
- Add or keep tests that assert exact persisted `lock_keys_json` output.

## Non-Goals

- Do not change stable lock-key strings.
- Do not change command request hashing or canonical payload semantics.
- Do not change `intent_json`, `summary_json`, `result_summary_json`, or response serialization.
- Do not change which aggregates each command locks.
- Do not migrate all booking, billing, group, invoice, or agent setting lock-key derivers into one large global API.
- Do not refactor `aggregate_locks.rs` unless compilation or tests prove a tiny compatibility change is required.

## Selected Approach

Use a command-specific helper module:

```text
mhm/src-tauri/src/command_idempotency/lock_keys.rs
```

The module will be a thin boundary for command idempotency persistence. It will not own the aggregate lock format itself. Existing constructors in `aggregate_locks.rs` remain the source for aggregate strings such as `room:{id}`, `booking:{id}`, `folio:{id}`, and `group:{id}`. Existing ad hoc command formats such as `settings:{setting_key}` must be preserved as existing command lock keys, not moved into a new global constructor as part of this issue.

This gives command idempotency one place to prepare lock-key JSON while keeping the implementation narrow enough for a safety-sensitive refactor.

## Stable Lock-Key Format

The implementation must preserve the currently persisted strings:

| Aggregate | Format |
| --- | --- |
| Room | `room:{room_id}` |
| Booking | `booking:{booking_id}` |
| Folio | `folio:{booking_id}` |
| Group | `group:{group_id}` |
| Setting | `settings:{setting_key}` |

The canonical persisted representation remains a stable JSON array of strings after sorting and deduplication:

```json
["booking:B1","room:R1"]
```

Low-risk commands may still persist an empty array:

```json
[]
```

## Architecture

`command_idempotency.rs` remains the public module root and executor home. `types.rs` continues to own command contract types. The new `lock_keys.rs` module owns only lock-key preparation helpers used by the executor.

Expected module shape:

```rust
mod lock_keys;
mod types;
```

The helper module should expose narrow parent-only functions, for example:

```rust
pub(super) fn optional_lock_keys_json<I, S>(keys: I) -> CommandResult<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>;

pub(super) fn required_lock_keys_json<I, S>(keys: I) -> CommandResult<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>;
```

Naming can change during implementation if the final names are clearer, but the boundary should stay narrow:

- optional helper allows empty input for low-risk/default lock derivation;
- required helper rejects empty input for resolved guard refresh with the current system error classification/code and exact message `Resolved idempotency lock keys are required`;
- both helpers produce stable JSON through the same serialization path.

## Data Flow

Initial command preparation:

```text
WriteCommandRequest
  -> request.lock_key_deriver(hash_payload)
  -> command_idempotency::lock_keys optional helper
  -> stable lock_keys_json
  -> command_idempotency row claim
```

Resolved guard refresh:

```text
ResolvedWriteCommandGuard.lock_keys
  -> command_idempotency::lock_keys required helper
  -> stable lock_keys_json
  -> UPDATE command_idempotency.lock_keys_json before business transaction
```

The lock-key deriver must continue to run against `hash_payload`, not sanitized ledger intent. This preserves the existing safety rule that operator-safe metadata cleanup cannot accidentally remove conflict keys needed for command locking.

## Error Handling

Initial lock keys may be empty. This preserves existing low-risk command behavior where `default_lock_key_deriver` returns no locks.

Resolved guard lock keys must be non-empty. If the guard resolves to an empty key set, the command must fail before running the guarded mutation and finalize the claimed idempotency row as it does today.

Deriver-specific errors must pass through unchanged. For example, if a command-specific deriver cannot find `booking_id` or `room_id` in its hash payload, the helper module should not mask that error.

Duplicate and ordering behavior must match current command idempotency persistence exactly:

- do not trim or otherwise normalize deriver-returned lock key strings inside the command idempotency helper;
- do not add new blank-key rejection for non-empty key lists in this issue;
- sort lexicographically;
- deduplicate;
- serialize with stable JSON.

The aggregate lock constructors may trim aggregate IDs before producing strings such as `room:{room_id}`, but #149 must not add a second normalization layer inside command idempotency. The command idempotency helper must not call `aggregate_locks::canonicalize_lock_keys`; that function is for aggregate lock acquisition semantics, not persisted command idempotency metadata. If future work wants to harden blank or whitespace-padded lock keys, that should be a separate behavior-change issue with explicit test updates.

## Testing

Tests should act as format tripwires. At minimum, implementation should keep or add coverage for:

- optional initial lock keys serialize sorted and deduplicated, for example `["booking:B1","room:R1"]`;
- optional initial lock keys allow empty output as `[]`;
- command idempotency helper behavior does not trim deriver-returned strings if a focused helper test covers malformed input;
- existing `settings:{setting_key}` lock output remains documented or covered by a focused tripwire test if the implementation touches that path;
- resolved guard lock keys serialize sorted and deduplicated before the service transaction runs;
- resolved guard empty lock keys still fail with the current system error classification/code and exact message `Resolved idempotency lock keys are required`;
- replay and hash-mismatch behavior remain unchanged.

Existing command idempotency tests already cover several of these behaviors. The implementation should add only focused tests where exact persisted output is missing.

Primary validation:

```bash
cd mhm/src-tauri && cargo test command_idempotency
rg -n "lock_key|lock keys|lock-key" docs mhm/src-tauri/src
```

If implementation touches `aggregate_locks.rs`, also run:

```bash
cd mhm/src-tauri && cargo test aggregate_locks
```

## Impact And Risk

Fresh GitNexus analysis before this design found high blast radius around the safety-sensitive symbols:

- `prepare_write_command_request`: CRITICAL risk, 7 direct callers, 9 affected execution flows, 3 affected modules.
- `refresh_claim_lock_keys`: LOW risk, 1 direct caller, 0 indexed execution flows, 2 affected modules.
- `canonicalize_lock_keys`: CRITICAL risk, 6 direct callers, 7 affected execution flows, 2 affected modules.
- `aggregate_key`: HIGH risk, 4 direct callers, 3 affected execution flows, 1 affected module.

Implementation must rerun GitNexus impact analysis before editing any symbol and must stop for user confirmation if fresh analysis reports HIGH or CRITICAL risk for an intended edit. The preferred implementation path avoids editing `aggregate_locks.rs` and keeps changes limited to command idempotency internals plus focused tests.

## Implementation Boundaries

Expected files:

- create `mhm/src-tauri/src/command_idempotency/lock_keys.rs`;
- modify `mhm/src-tauri/src/command_idempotency.rs` to declare the module and use its helpers;
- optionally modify tests in `mhm/src-tauri/src/command_idempotency.rs`;
- do not modify unrelated frontend files.

The existing dirty file `mhm/src/stores/useHotelStore.test.ts` is unrelated and must not be staged or changed for this issue.

## Success Criteria

- Lock-key preparation is isolated in a focused command idempotency module.
- Persisted lock-key JSON is byte-for-byte equivalent for covered cases.
- Low-risk empty lock keys still persist as `[]`.
- Resolved guard empty lock keys still fail before mutation.
- Existing command idempotency tests pass.
- Validation search shows the lock-key format and safety boundary are documented.
