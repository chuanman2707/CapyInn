---
name: pms-command-safety
description: Rules for writing or changing a PMS business write in mhm/src-tauri — commands, services, repositories, idempotency, outbox, locking, audit.
paths:
  - "mhm/src-tauri/src/commands/**"
  - "mhm/src-tauri/src/services/**"
  - "mhm/src-tauri/src/repositories/**"
  - "mhm/src-tauri/src/command_idempotency/**"
  - "mhm/src-tauri/src/command_ledger.rs"
  - "mhm/src-tauri/src/command_recovery.rs"
  - "mhm/src-tauri/src/outbox.rs"
---

# PMS command safety

CapyInn runs real hotels offline. A lost or double-applied write is money or a room
that does not exist. These rules are why the command layer looks heavier than the
feature would suggest — do not simplify them away.

## The command boundary

Every PMS business write goes through an explicit command. UI, bots, agents, and
integrations must never mutate PMS tables directly.

Every command carries: actor, command name, idempotency key, canonical payload hash,
timestamp, and request context. A command missing any of these is not a command.

## Atomicity

One business write is one unit:

```
validate → authorize → lock/serialize → mutate → audit → outbox → commit or rollback
```

All of it, or none of it. No partial commit, no "we'll fix it on the next run".

## Idempotency

Retryable commands must be idempotent:

- same key + same canonical payload → replay the prior result, do not re-apply
- same key + **different** payload → reject

The canonical payload hash is what makes the second case detectable. Never hash a
serialization whose field order or whitespace can vary.

## Locking

Serialize high-risk writes by stable lock keys: `booking_id`, `room_id`/date,
`folio_id`, `invoice_id`, `ledger_id`, `audit_date`. Build keys through the shared
lock-key builder rather than formatting strings at the call site.

## Validation

Validate before mutation, and fail closed. Unknown fields, invalid money, invalid
dates, invalid statuses, illegal transitions, missing permissions, and ambiguity are
all errors — never a best-effort guess.

Booking, room, housekeeping, invoice, and group statuses move through explicit state
machines. Never assign a raw status string.

## Money

Integer VND (`MoneyVnd = i64`). Never `f64` for an amount. Validate through
`money.rs` before it crosses the IPC boundary. See `mhm/src-tauri/CLAUDE.md`.

## Financial records

Ledger and folio rows are append-only. A correction is a reversal or adjustment row,
never an `UPDATE` over history.

## External effects

Anything leaving the process — OTA sync, notification, webhook, backup trigger,
invoice delivery — is an outbox event written in the same transaction as the mutation.
No direct side effect inside a business mutation, ever. The dispatcher owns delivery
and retry.

## Audit

Every important mutation is reconstructable from actor, command, payload hash,
timestamp, lock keys, and affected aggregate.

## Agents and integrations

Least privilege applies to AI agents and integration keys exactly as it does to staff.
Agent memory is not PMS truth: an LLM may *suggest* a command, but state changes only
through a validated, authorized, audited command. Booking, payment, and availability
truth come from CapyInn read tools, never from memory.

`mhm/tests/agentic-guardrails.test.ts` enforces the documented form of this and the
loopback-only gateway binding.

## Before you claim it is done

```bash
cargo test --manifest-path src-tauri/Cargo.toml architecture_guard -- --nocapture
npm run verify:money
npm run verify:quick
```
