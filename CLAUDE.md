# CapyInn

Offline-first PMS for mini hotels. Tauri v2 desktop app: Rust backend + React frontend, SQLite.

- Git root is this directory. The app lives in `mhm/` — that name is **rename debt, not the product name**.
- `mhm/src-tauri/` is the Rust backend (~71k lines). `mhm/src/` is the React frontend (~29k lines).
- Each has its own `CLAUDE.md` with conventions for that side. Read it before editing there.

## Dependency direction

`mhm/src-tauri/src/architecture_guard.rs` enforces this as a test. Violating it is a red build, not a review comment.

```
write:  UI → command → service/lifecycle → repository/transaction → SQLite
read:   UI → command → query → SQLite
```

`commands/` is the outermost boundary. Nothing further in may depend on it.

## Core PMS vs experimental runtime

- **Core PMS**: rooms, stays, reservations, guests, housekeeping, billing/folios/invoices, groups, night audit, settings, auth, schema/migrations.
- **Experimental runtime**: gateway, MCP, agent runtime, observer streams, digest, Telegram, CEO, OpenAI.

Normal PMS operation must never require experimental runtime to be configured. `docs/architecture/core-pms-boundaries.md` is the canonical guardrail.

## Non-negotiables

Each of these has an automated guard. Name the guard when you claim compliance.

| Rule | Guard |
| --- | --- |
| Money is integer VND (`MoneyVnd = i64`). Never `f64` for an amount. | `npm run verify:money` — **manual only, not in CI** |
| Layer dependency direction (above) | `cargo test architecture_guard` |
| PMS writes go through the `invokeCommand` wrapper, not raw `invoke` | `mhm/tests/frontend-invoke-wrapper-guardrails.test.ts` |
| Agent memory is not PMS truth; gateway stays loopback-only | `mhm/tests/agentic-guardrails.test.ts` |

Beyond the guarded rules: every PMS business write goes through a command boundary with actor, command name, idempotency key, payload hash, and timestamp; validate before mutate and fail closed; serialize high-risk writes by stable lock keys; keep ledger and folio rows append-only; emit external effects via the outbox in the same transaction. The `pms-command-safety` skill has the full set — it loads on demand when you touch `commands/`, `services/`, or `repositories/`.

## Commands

Run from `mhm/` unless stated otherwise.

```bash
npm run tauri dev          # dev app
npm test                   # frontend suite (vitest)
npm run build              # tsc + vite build
npm run verify:quick       # targeted frontend + cargo tests
npm run verify:full        # verify:quick + full frontend + booking/backup + native smoke
npm run verify:money       # money float scan — nothing else runs this, run it yourself
```

From `mhm/src-tauri/`:

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

CI (`.github/workflows/ci.yml`) runs `npm test`, `npm run build`, then the four cargo commands, then `verify:full` in a second job. It never runs `verify:money`.

## Gotchas that have cost real time

- **Never trust a GUI check without identifying the build first.** Two CapyInn builds share bundle id `io.capyinn.app`, and a reinstall silently does nothing if the old process is still up. Both traps have burned a full QA cycle. The `verifying-a-build` skill has the checks — use it before believing anything on screen.
- **`rooms.type` stores the display name, not a slug.** Live values include `"Standard Room"` and `"Deluxe Balcony"` — they contain spaces. Never use a character delimiter for a list of room types; serialize with `JSON.stringify`. An unknown room type silently falls back to the house default instead of erroring, so corruption passes tests. Use real multi-word names as fixtures.
- **The main checkout at `/Users/binhan/HotelManager` is shared with other sessions and changes branch mid-session.** For multi-step work, create a worktree off `main` first: `git worktree add .worktrees/<name> -b <branch> main`. Verify any path you read in the main checkout still exists on the branch you will actually build on.

## Working approach

- Read before writing. Prefer small targeted edits over rewriting files.
- Test before declaring done. When mutation-testing, prove the edit landed before believing a green result.
- Be concise in output, rigorous in reasoning.
- No sycophantic openers or closing fluff.
- Do not commit generated docs or specs unless asked.
- Suggest `/cost` when a session runs long; suggest a new session when switching to an unrelated task.
- User instructions override this file.
