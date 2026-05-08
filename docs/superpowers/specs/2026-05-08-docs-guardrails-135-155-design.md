# Docs Guardrails For Issues 135 And 155 Design

Date: 2026-05-08
Status: Approved for implementation planning

## Purpose

Implement GitHub issues #135 and #155 as a docs-only architecture guardrail pass. The work should define CapyInn's stable PMS core, separate experimental runtime surfaces from normal PMS operation, document the command safety contract, and establish the command/service/query convention without changing runtime behavior.

## Scope

- Create `docs/architecture/core-pms-boundaries.md` as the canonical architecture boundary document.
- Update `CONTRIBUTING.md` to use CapyInn naming, explain that `mhm/` is the current implementation path and rename debt, and summarize the contributor rules from the canonical doc.
- Do not edit `README.md` in this pass.
- Do not move files, rename `mhm/`, delete agent/gateway code, or change runtime behavior.

The repository ignores `/docs/`, so the new canonical architecture document must be force-added as part of implementation:

```bash
git add -f docs/architecture/core-pms-boundaries.md
```

## Canonical Boundary Document

`docs/architecture/core-pms-boundaries.md` will define:

- stable core PMS areas: rooms, stays, reservations, guests, housekeeping, billing, invoices, groups, night audit, settings, and auth;
- experimental runtime areas: gateway, MCP, agent runtime, observer streams, digest, Telegram, CEO, and OpenAI surfaces;
- command safety core: explicit command boundary, actor, command name, idempotency key, canonical payload hash, timestamp, request context, stable lock keys, audit writes, command ledger metadata, and transactional outbox writes;
- experimental disabled meaning: no experimental background tasks, no required external API keys, no Telegram/OpenAI/MCP/gateway config, no direct agent PMS table mutation, and no experimental UI in the normal profile;
- command orchestration convention: writes follow `UI -> command -> service/lifecycle`, reads follow `UI -> command -> query`;
- SQL placement convention: mutation SQL belongs behind service/lifecycle/repository boundaries, while reusable or growing read SQL belongs in query modules;
- `mhm/` as the current implementation path and postponed rename debt, not the product name.

## Contributor Guidance

`CONTRIBUTING.md` will stay concise and link or point contributors to the canonical boundary document. It should make the command boundary explicit enough for day-to-day review:

- business writes go through commands and service/lifecycle modules;
- reads go through commands and query modules when the logic is shared or the command module is growing;
- agents, bots, UI, and integrations must not mutate PMS tables directly;
- experimental runtime work must remain disabled from the normal profile unless a later issue explicitly promotes it.

## Validation

Run adapted validation searches. The original issue validation commands include `README.md`, but the user explicitly removed `README.md` from this pass, so validation should cover `CONTRIBUTING.md` and `docs` only:

```bash
rg -n "core PMS|experimental|MCP|agent|gateway|Telegram|OpenAI|idempotency|lock key|audit|outbox|experimental disabled" CONTRIBUTING.md docs
rg -n "command.*service|command.*query|orchestration|mhm" docs CONTRIBUTING.md
```

Also check the diff to confirm the implementation is docs-only and does not touch `README.md` or runtime files.

## Risks

The implementation risk is low because the change is documentation-only. The main review risk is accidental scope creep into README, runtime behavior, or broad cleanup docs. Keep the implementation edit set limited to the canonical architecture doc and `CONTRIBUTING.md`.
