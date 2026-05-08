# Core PMS Boundaries

Date: 2026-05-08
Status: Canonical guardrail

## Purpose

CapyInn is an offline-first PMS for mini hotels. This document defines the boundary between stable PMS core, command safety infrastructure, and experimental runtime surfaces so contributors can keep cleanup and feature work reviewable.

Normal PMS operation must not require gateway, MCP, OpenAI, Telegram, CEO-agent, digest, or other experimental runtime configuration.

## Core PMS

Core PMS is the stable product surface required for day-to-day hotel operations:

- rooms and room status;
- stays, check-in, extend-stay, and check-out;
- reservations and reservation lifecycle;
- guests and guest registration data;
- housekeeping and maintenance state;
- billing, folios, payments, invoices, and financial reporting;
- group booking and group invoices;
- night audit and end-of-day reconciliation;
- settings, authentication, and app lock;
- SQLite schema, migrations, and queries required for normal PMS workflows.

Core PMS work should be understandable, testable, and able to run without experimental runtime enabled.

## Experimental Runtime

Experimental runtime is not part of the stable PMS product surface unless a later issue explicitly promotes it:

- gateway runtime and gateway consumers;
- MCP surfaces;
- agent runtime, tools, providers, supervisors, and memory;
- observer streams and external consumers;
- digest jobs and digest delivery;
- Telegram surfaces;
- CEO and OpenAI surfaces;
- experimental UI badges, panels, toasts, or background tasks.

Experimental runtime code may exist in the repository, but it must not be required by the normal PMS profile.

## Command Safety Core

Command safety is PMS core infrastructure, not experimental platform work. Every business write must cross an explicit command boundary and preserve the command metadata needed for idempotency, authorization, auditability, and recovery:

- actor;
- command name;
- idempotency key;
- canonical payload hash;
- timestamp;
- request context;
- stable lock keys for high-risk aggregates such as booking, room/date, folio, invoice, ledger, or audit date;
- command ledger metadata;
- audit writes;
- transactional outbox writes when outbox records are part of the same business mutation.

Retryable commands must be idempotent. The same idempotency key with the same canonical payload hash replays the prior result; the same key with a different payload hash must fail closed. High-risk writes must be serialized by stable lock keys. Financial records must remain append-only where the domain requires reversals or adjustments.

The important distinction is that transactional outbox writes can be core safety, while outbox dispatchers, observers, gateway consumers, and external delivery workers are experimental runtime.

## Experimental Disabled

Experimental disabled means:

- no experimental background task starts by default;
- no external API key is required;
- no Telegram, OpenAI, MCP, or gateway config is required;
- no agent, bot, UI surface, or integration mutates PMS tables directly;
- no experimental UI entry appears in the normal app profile;
- core PMS smoke flows can run with experimental flags and config absent.

Any external effect from a business mutation must go through a durable outbox record written in the same transaction as the mutation. Dispatching that event is a separate runtime concern.

## Command Orchestration Convention

Tauri commands are the boundary between UI/integrations and the backend. They should orchestrate work instead of becoming the permanent home for large business rules or shared SQL.

Write flow:

```text
UI
  -> command
  -> service / lifecycle
  -> repository / transaction
  -> audit + idempotency + lock + transactional outbox write
  -> SQLite
```

Read flow:

```text
UI
  -> command
  -> query
  -> SQLite
```

New write behavior should have a clear service or lifecycle home. New read behavior should have a query-module home when the SQL is shared, growing, or part of a command module that is already a review hotspot.

Small command-local read helpers are acceptable when they are genuinely narrow, private, and unlikely to grow. Do not create abstractions just to move a few lines.

## SQL Placement

Use these defaults when adding or moving SQL:

- mutation SQL belongs behind service, lifecycle, repository, or transaction boundaries;
- reusable read SQL belongs in query modules;
- command modules may adapt request/response shapes, validate command-level inputs, call services or queries, and map errors;
- command modules should not accumulate unrelated read SQL, write SQL, and business state-machine logic in the same function;
- direct PMS table writes from UI, bots, agents, or integrations are forbidden.

## Implementation Path And Rename Debt

`mhm/` is the current implementation path. It is not the product name.

The product and repository story should use CapyInn. Renaming `mhm/` remains postponed cleanup until canonical docs, CI, smoke tests, and normal-profile runtime boundaries are stable. Do not combine an `mhm/` folder rename with command, migration, frontend shell, or experimental-runtime changes.

## Review Checklist

Before approving a cleanup or feature PR, check:

- Does the change affect core PMS, experimental runtime, or both?
- If it writes business data, does it cross the command boundary with actor, command name, idempotency key, canonical payload hash, timestamp, and request context?
- Are high-risk writes serialized by stable lock keys?
- Are audit and command ledger records preserved where required?
- Are transactional outbox writes kept with the business mutation when they are part of the safety contract?
- Are outbox dispatchers, observers, gateway, MCP, agent, digest, Telegram, CEO, and OpenAI surfaces disabled from the normal PMS profile?
- Does the PR avoid mixing folder rename debt with runtime or command-safety changes?
