# Outbox Dispatch, Startup Recovery, and Verification

Issues: #79, #82, and #85

Parent scope: #76 Outbox and agentic integration

Source roadmap: `docs/superpowers/specs/capyinn-concurrency-spec-v2.2.md`

Date: 2026-05-03

## Status

Approved design for the outbox dispatcher foundation.

This spec covers only dispatcher core, startup notification recovery, and verification tests. It deliberately does not add the real MCP observer subscriber from #80 or the real UI refresh subscriber from #81.

## Plain-Language Goal

CapyInn already writes committed business events into `outbox_events`. The next step is to deliver those events safely.

The business promise is:

- committed notifications are not lost after app crash or restart
- notification retry never re-runs the original business command
- events for one booking, group, folio, invoice, or room are delivered in order
- duplicate delivery is possible only in the safe, expected way, and subscribers must dedupe by event id

This is a delivery foundation, not a new business workflow engine.

## Scope Decisions

Chosen scope:

- Implement dispatcher core for `outbox_events`.
- Claim events before dispatching.
- Reclaim expired `processing` events.
- Retry subscriber failures with bounded attempts.
- Mark events `failed` after the retry limit.
- Preserve FIFO order per `aggregate_key`.
- Auto-resume pending and expired-processing outbox notifications on startup.
- Keep command recovery read-only on startup.
- Add Rust tests that prove rollback, crash reclaim, retry, FIFO, and startup behavior.
- Use test subscribers to verify delivery behavior.

Out of scope:

- Real MCP observer stream subscriber.
- Real UI refresh subscriber.
- Replacing existing direct `emit_db_update` calls.
- OTA sync, webhooks, notifications, or external integration delivery.
- A generic background task queue.
- Automatic replay of business commands.
- Operator UI for failed outbox rows.
- Outbox retention cleanup.

## Current Baseline

The existing foundation already provides:

- `outbox_events` schema with `pending`, `processing`, `dispatched`, and `failed`-ready columns.
- indexes for pending claim and expired processing scans.
- origin command metadata: request id, idempotency key, command name, and request hash.
- executor-owned event insertion in the same transaction as the business write.
- tests proving successful commands persist one event and failed commands persist no event.
- command startup recovery that scans expired `in_progress` and `failed_retryable` rows without replaying commands.

The missing part is the worker that turns durable event rows into safe delivery attempts.

## Recommended Approach

Use a small backend dispatcher owned by `outbox.rs`.

The dispatcher should expose clear, testable operations:

- claim the next eligible event
- dispatch one claimed event to subscribers
- mark success
- record retry
- reclaim expired processing rows through the same claim path
- run one batch for startup and background polling

This is cleaner than wiring real UI or MCP subscribers immediately because it keeps #79, #82, and #85 focused on delivery correctness. Real subscribers can later plug into the same dispatcher without changing the claim and retry rules.

## Event State Model

Use the existing `status` column:

| Status | Meaning |
|---|---|
| `pending` | committed and ready for a future delivery attempt |
| `processing` | claimed by one worker token until `processing_expires_at` |
| `dispatched` | delivered successfully to all subscribers in this dispatch scope |
| `failed` | retry limit reached; needs later inspection or manual repair |

`failed` rows do not block later events for the same aggregate. The FIFO guard blocks older `pending` and `processing` rows only. This keeps one permanently bad notification from freezing the whole aggregate stream.

## Claim Rule

Workers must claim before dispatching.

An event is eligible when:

- status is `pending`, or status is `processing` and `processing_expires_at` has passed
- `next_attempt_at` is empty or due
- no older event for the same `aggregate_key` is still `pending` or `processing`

Claiming sets:

- `status = 'processing'`
- a fresh `worker_token`
- `processing_started_at`
- `processing_expires_at`

The dispatcher must finalize only when the stored `worker_token` still matches. A stale worker cannot mark a row dispatched or failed after another worker has reclaimed it.

## Delivery Contract

Delivery is at least once.

This means the same event may be delivered more than once if the app crashes after a subscriber receives it but before the row is marked `dispatched`. That is acceptable because subscribers must dedupe by `outbox_events.id`.

There is no global ordering promise. Events for different aggregate keys may be delivered independently.

Per-aggregate FIFO is required. For one `aggregate_key`, a later event cannot pass an older `pending` or `processing` event.

## Retry Policy

Use a small bounded retry policy in the dispatcher.

Recommended defaults:

- max attempts: 5
- processing lease: 30 seconds
- retry delays: short exponential backoff with a cap, stored as `next_attempt_at`

On subscriber failure:

1. increment `attempts`
2. clear `worker_token`
3. clear processing timestamps
4. store a safe `last_error`
5. set `next_attempt_at` if attempts remain
6. mark `failed` if attempts reached the limit

The stored error must be short and safe. It must not include guest PII, secrets, raw payloads, or large subscriber output.

## Startup Behavior

Startup may resume outbox notifications automatically.

Startup must not replay business commands.

Startup should do two separate things:

1. run the existing command recovery startup scan and log counts for command rows that need attention
2. start or trigger the outbox dispatcher so pending and expired notification rows can be delivered

Logs and future UI wording must keep the difference clear:

- outbox notification replay resumed
- business command recovery attention required

This separation preserves the PMS safety rule: hotel truth changes only through explicit, validated, authorized business commands.

## Dispatcher Runtime

The runtime should be conservative:

- run once on startup to drain immediately eligible rows
- continue polling on a modest interval while the app is open
- process a bounded batch per tick
- stop cleanly with the app runtime
- expose a test helper that can run one batch without spawning a background loop

The first implementation does not need multiple concurrent dispatcher workers. The SQL claim rule should still be correct if a later version adds more workers or another app process attempts delivery.

## Subscriber Boundary

For this scope, use test subscribers only.

A subscriber receives a safe event envelope:

- event id
- event type
- aggregate key
- payload JSON
- origin request id
- origin command name
- origin idempotency key
- origin request hash
- created timestamp
- current attempt number

Subscriber code must treat the event as a read-only fact. It must not mutate PMS tables directly and must not issue business commands.

Future real subscribers:

- #80 MCP observer stream
- #81 UI refresh subscriber, if useful

Those subscribers should dedupe by event id and re-read canonical state from SQLite when they need details.

## Error Handling

The dispatcher fails closed.

Database claim failure:

- leave rows unchanged
- return/log a structured system error
- try again on the next tick

Subscriber failure:

- retry according to policy
- mark `failed` after the limit
- do not replay business command

Stale worker finalize:

- update affects zero rows
- treat as a lost race
- do not overwrite the current row state

Malformed event payload:

- count as subscriber/dispatch failure for that event
- retry up to the limit
- mark `failed` after the limit

## Verification Plan

Required Rust tests:

- same-transaction rollback creates no outbox row
- pending event is claimed before dispatch
- expired `processing` event is reclaimed after lease expiry
- active `processing` event is not reclaimed before lease expiry
- subscriber success marks event `dispatched`
- subscriber failure retries with `next_attempt_at`
- repeated subscriber failure marks event `failed` after the retry limit
- stale worker token cannot mark a reclaimed event dispatched
- two events with the same `aggregate_key` dispatch in id order
- later event with the same `aggregate_key` does not pass an older pending or processing event
- events for different aggregate keys do not require global ordering
- startup outbox recovery processes pending or expired notification rows
- startup command recovery remains scan/log only and does not execute business command paths

Existing rollback tests from the persistence foundation should remain in place. Add new dispatcher tests near the dispatcher code so the core behavior is easy to reason about without UI or MCP setup.

## Acceptance Mapping

#79:

- worker claims events before dispatching through the claim operation
- expired processing events are reclaimable through the same claim path
- per-aggregate FIFO is enforced by the older pending/processing guard
- subscriber failure retries and then marks failed
- delivery is at least once and subscriber dedupe is by event id

#82:

- startup resumes pending and expired outbox notifications
- startup does not auto-execute business commands
- logs distinguish notification replay from command recovery
- high-risk command recovery remains under existing operator/caller intent rules

#85:

- rollback behavior remains covered
- worker crash is modeled by expired `processing` rows
- retry and failed behavior are covered
- FIFO is covered
- restart behavior is covered without business command replay

## Definition of Done

This slice is done when:

- dispatcher core exists behind a small API in backend code
- startup starts or triggers outbox notification recovery
- real UI and MCP subscribers remain out of scope
- tests cover crash, retry, rollback, FIFO, and startup separation
- relevant Rust tests pass
- GitNexus detect changes shows only the expected outbox/startup verification scope before implementation commits
