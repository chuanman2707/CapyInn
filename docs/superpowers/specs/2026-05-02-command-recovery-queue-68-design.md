# Issue #68 Command Recovery Queue Design

Source issue: https://github.com/chuanman2707/CapyInn/issues/68

Source roadmap: `docs/superpowers/specs/capyinn-concurrency-spec-v2.2.md`

Related designs:

- `docs/superpowers/specs/2026-04-26-write-command-executor-65-design.md`
- `docs/superpowers/specs/2026-04-26-command-ledger-66-design.md`

## Plain-Language Goal

Issue #68 exposes safe recovery handling for command rows that need operator attention after a crash, app restart, database lock, or retryable failure.

The app must be able to find:

- expired `in_progress` command rows
- `failed_retryable` command rows

It must not replay business commands during startup. A recovery action may inspect, request a retry, dismiss the current queue item, or mark the command terminal, but hotel truth still changes only through the existing command boundary.

## Scope Decision

Chosen scope: **backend recovery actions with no new UI screen**.

This issue adds:

- read-only startup scan
- backend recovery queue and inspection APIs
- Tauri admin recovery actions
- read-only MCP/gateway recovery tools
- recovery audit table
- dismiss markers for queue filtering
- structured `RECOVERY_REQUIRED` outcomes where the system refuses to auto-mutate hotel truth

This issue does not add:

- a new recovery screen
- frontend buttons
- automatic command replay
- command-specific replay handlers
- MCP/gateway write recovery tools
- outbox recovery behavior

## Current Baseline

The codebase already has:

- `command_idempotency` as the durable command ledger
- statuses `in_progress`, `completed`, `failed_retryable`, and `failed_terminal`
- read-only command ledger APIs for list, attention list, and detail
- expired `in_progress` detection through `lease_expires_at`
- retryable reclaim behavior inside the write command executor
- sanitized ledger intent, summary, result summary, and error summary
- Tauri admin gating for command ledger reads

#68 should build on that baseline instead of replacing the ledger or executor.

## Design Principles

1. **Startup is read-only for business commands.**
   Startup may identify recovery rows and log counts, but it must not replay, dismiss, mark terminal, or otherwise mutate hotel truth.

2. **Retry request is not replay.**
   A recovery retry action records an operator decision and returns a structured `RECOVERY_REQUIRED` outcome. The original business command must still be sent again through its normal command boundary with a valid payload.

3. **Audit decisions, not reads.**
   `inspect` is read-only and does not write an audit action. `retry_requested`, `dismissed`, and `marked_terminal` are operator decisions and must be audited.

4. **Dismiss is reversible queue cleanup.**
   `dismiss` hides the current recovery item from the default queue. It does not block future inspect or retry.

5. **Mark terminal closes the command.**
   `mark terminal` intentionally changes the command row to `failed_terminal` so the executor cannot reclaim that same command row later.

6. **MCP is read-only.**
   Agents may inspect recovery problems, but they cannot retry, dismiss, or mark terminal through MCP.

## Data Model

Add a recovery action audit table:

```sql
CREATE TABLE command_recovery_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command_idempotency_id INTEGER NOT NULL,
    action TEXT NOT NULL,
    operator_id TEXT,
    operator_role TEXT,
    reason TEXT,
    confirmed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    FOREIGN KEY(command_idempotency_id) REFERENCES command_idempotency(id)
);
```

Allowed action values:

- `retry_requested`
- `dismissed`
- `marked_terminal`

Add queue-filter markers to `command_idempotency`:

```sql
ALTER TABLE command_idempotency ADD COLUMN recovery_dismissed_at TEXT;
ALTER TABLE command_idempotency ADD COLUMN recovery_dismissed_by TEXT;
```

Add indexes focused on the default queue and action history:

```sql
CREATE INDEX IF NOT EXISTS command_recovery_actions_command_idx
    ON command_recovery_actions(command_idempotency_id, created_at);

CREATE INDEX IF NOT EXISTS command_idempotency_recovery_queue_idx
    ON command_idempotency(status, lease_expires_at, updated_at)
    WHERE status IN ('in_progress', 'failed_retryable')
      AND recovery_dismissed_at IS NULL;
```

The action table is the audit history. The dismiss columns are only the current queue-filter state.

## Recovery Queue

The default recovery queue includes only rows that can still require an operator decision:

- `in_progress` rows whose `lease_expires_at` is present and expired
- `failed_retryable` rows
- rows where `recovery_dismissed_at IS NULL`

The default queue excludes:

- live `in_progress`
- `completed`
- `failed_terminal`
- dismissed rows

`failed_terminal` rows remain available through command ledger detail. They are not recovery queue items by default.

Queue items should reuse the safe ledger fields from #66 and add recovery-specific metadata:

- `recovery_status`
- `risk_level`
- `requires_confirmation`
- `allowed_actions`

Suggested `recovery_status` values:

- `expired_in_progress`
- `failed_retryable`
- `dismissed`
- `terminal`

The default queue should normally return only `expired_in_progress` and `failed_retryable`.

## Risk and Confirmation

#68 uses a hybrid risk model.

For now, risk is computed from `command_name` with a small backend function. The DTO still returns `risk_level` and `requires_confirmation` so a later manifest-based implementation can replace the source without changing the API contract.

Initial high-risk examples:

- checkout
- payment posting
- folio charge posting
- room move or stay modification
- reservation cancellation
- check-in
- group checkout
- night audit
- invoice generation

High-risk `request retry` and `mark terminal` actions require:

- `confirmed: true`
- a non-empty `reason`

`inspect` never requires confirmation. `dismiss` does not require confirmation because it only hides the current queue item and does not block future recovery.

## Backend APIs

Add backend recovery functions in a focused module such as `command_recovery.rs`.

Tauri/admin commands:

- `list_command_recovery_queue`
- `inspect_command_recovery`
- `request_command_recovery_retry`
- `dismiss_command_recovery`
- `mark_command_recovery_terminal`

MCP/gateway read-only tools:

- list recovery queue
- inspect recovery detail

MCP/gateway must not expose:

- retry request
- dismiss
- mark terminal

## Action Behavior

### Inspect

`inspect_command_recovery(id)` returns sanitized command detail plus recovery metadata and action history. It does not write an audit row.

### Request Retry

`request_command_recovery_retry(id, confirmed, reason)`:

1. Loads and validates the command row.
2. Allows only expired `in_progress` or `failed_retryable` rows.
3. Applies high-risk confirmation rules.
4. Inserts `command_recovery_actions(action = 'retry_requested')`.
5. Leaves the command row status unchanged.
6. Does not run the business command.
7. Returns a structured outcome with `RECOVERY_REQUIRED`.

The response must say that the command must be retried through the normal command boundary with the original valid business payload.

### Dismiss

`dismiss_command_recovery(id, reason)`:

1. Loads and validates the command row.
2. Allows only rows currently eligible for the recovery queue.
3. Inserts `command_recovery_actions(action = 'dismissed')`.
4. Sets `recovery_dismissed_at` and `recovery_dismissed_by`.
5. Leaves command status unchanged.

Dismiss does not prevent later inspect or retry. It only hides the current recovery item from the default queue.

### Mark Terminal

`mark_command_recovery_terminal(id, confirmed, reason)`:

1. Loads and validates the command row.
2. Allows only expired `in_progress` or `failed_retryable` rows.
3. Applies high-risk confirmation rules.
4. Inserts `command_recovery_actions(action = 'marked_terminal')`.
5. Updates the command row to `failed_terminal`.
6. Writes `error_code = 'RECOVERY_REQUIRED'`.
7. Writes a safe structured `error_json` and `error_summary_json`.
8. Clears `retryable`, `lease_expires_at`, and dismiss markers.
9. Sets `completed_at` and `updated_at`.

After this action, retrying the same command and idempotency key should return the stored terminal error instead of reclaiming the command.

## Dismiss Marker Clearing

Dismiss applies only to the current recovery condition.

The executor should clear `recovery_dismissed_at` and `recovery_dismissed_by` when a command row gets a new attempt or a new failure, including reclaim paths that rotate the claim token. This prevents an old dismiss decision from hiding a later failure on the same command row.

## Startup Behavior

After database initialization, startup should run a read-only scan that counts rows needing command recovery by reason:

- expired `in_progress`
- `failed_retryable`

Startup may log a safe message such as:

```text
Command recovery attention required: expired_in_progress=1 failed_retryable=2
```

Startup must not:

- run business commands
- reclaim command rows
- mark rows terminal
- dismiss rows
- write recovery actions
- change hotel truth

## Error Model

Add `RECOVERY_REQUIRED` to the command error registry.

Use `attention_reason` and `recovery_status` for queue/list state. Do not use `RECOVERY_REQUIRED` for every row that appears in the queue.

Use `RECOVERY_REQUIRED` when a recovery action needs to explain that safe automatic completion is not possible. Examples:

- `request retry` returns `RECOVERY_REQUIRED` because the operator decision has been recorded but the business command must be sent again through the command boundary.
- `mark terminal` stores a terminal structured error with `RECOVERY_REQUIRED` so a later exact retry receives a clear terminal outcome.

All recovery errors and outcomes should use the structured command error envelope:

- `code`
- `message`
- `kind`
- `retryable`
- optional `support_id`
- optional `request_id`

## Security and Privacy

Recovery APIs must keep the #66 privacy boundary:

- do not expose `claim_token`
- do not expose `request_hash`
- do not expose raw `lock_keys_json`
- do not expose raw request payloads
- do not expose raw replay `response_json` or unsafe `error_json`
- do not expose idempotency internals through MCP tool arguments

Operator and MCP inspection use sanitized ledger intent, summary, result summary, and error summary.

MCP read tools may help an agent understand the failure and propose next steps. MCP must not mutate recovery state.

## Out Of Scope

- Recovery UI
- One-click replay from ledger
- Storing raw payloads for replay
- Command-specific replay handlers
- MCP recovery write tools
- Outbox event recovery
- Retention cleanup
- Risk manifests or policy metadata migration
- Full supervised high-risk write enablement

## Required Tests

Migration tests:

- fresh database creates `command_recovery_actions`
- fresh database adds `recovery_dismissed_at` and `recovery_dismissed_by`
- fresh database creates recovery queue and action history indexes
- existing database upgrades cleanly

Queue tests:

- expired `in_progress` appears
- live `in_progress` does not appear
- `failed_retryable` appears
- `completed` does not appear
- `failed_terminal` does not appear in the default recovery queue
- dismissed rows do not appear

Startup tests:

- startup scan returns counts by reason
- startup scan is read-only and does not change command rows
- startup does not call business command execution paths

Action tests:

- inspect is read-only and writes no recovery action
- retry request writes `retry_requested` audit action
- retry request does not change command status
- retry request does not run a business command
- retry request returns `RECOVERY_REQUIRED`
- dismiss writes `dismissed` audit action
- dismiss sets dismiss markers
- dismiss does not change command status
- mark terminal writes `marked_terminal` audit action
- mark terminal updates status to `failed_terminal`
- mark terminal stores structured `RECOVERY_REQUIRED` error data
- mark terminal prevents later reclaim of the same row
- high-risk retry request and mark terminal require confirmation and non-empty reason
- low-risk retry request and mark terminal do not require confirmation
- dismiss reason is optional
- dismiss marker clears on new attempt or new failure

MCP/gateway tests:

- MCP can list recovery queue
- MCP can inspect recovery detail
- MCP does not expose retry, dismiss, or mark-terminal tools
- MCP recovery DTOs do not expose raw idempotency internals

## Acceptance Mapping

Issue #68 acceptance criteria:

- Startup can identify expired `in_progress` and `failed_retryable` command rows.
  Covered by read-only startup scan and recovery queue query.

- Business commands are not automatically replayed on startup.
  Covered by startup rules and tests that assert no business execution path is called.

- Operator-facing actions exist for inspect, retry now, dismiss, or mark terminal.
  Covered by Tauri/admin recovery commands.

- High-risk recovery actions require explicit operator confirmation.
  Covered by risk calculation plus `confirmed` and non-empty `reason` checks for high-risk retry request and mark terminal.

- Recovery errors use the structured error envelope and `RECOVERY_REQUIRED` where appropriate.
  Covered by registry addition, structured action outcomes, and terminal recovery error storage.

## Implementation Notes

Keep the implementation small and layered:

- `command_ledger` remains the read-only ledger surface.
- `command_recovery` owns recovery queue, startup scan, action validation, audit writes, and mark-terminal update.
- `command_idempotency` only needs targeted changes to clear dismiss markers on new attempts or failures.
- `commands/command_recovery.rs` exposes Tauri admin commands.
- `gateway/tools.rs` exposes only read-only recovery tools.

Before editing any function, class, or method, run GitNexus upstream impact analysis for that symbol as required by the project rules. Before committing implementation work, run `gitnexus_detect_changes()`.
