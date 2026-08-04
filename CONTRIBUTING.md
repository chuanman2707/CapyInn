# Contributing to CapyInn

Thanks for contributing.

## Project Layout

- `mhm/` is the current implementation path for the CapyInn PMS application. It is rename debt, not the product name.
- `docs/architecture/core-pms-boundaries.md` is the canonical guardrail for core PMS, experimental runtime, command safety, and the postponed `mhm/` rename.
- `docs/superpowers/specs/` contains published design specs, and `docs/superpowers/plans/` contains the matching implementation plans, when present.
- `docs/release-checklist.md` and `docs/release-signing.md` cover the release preflight, signing, and updater manifest details.

## Prerequisites

- macOS 12 or newer for local development; Windows and Linux bundles are produced by the release workflow, not by the documented local setup
- Node.js 20 or newer
- Rust stable via `rustup`
- Xcode Command Line Tools

## Local Setup

```bash
git clone https://github.com/chuanman2707/CapyInn.git
cd CapyInn/mhm
npm ci
```

## Development

```bash
npm run tauri dev
```

## Verification

Run these before opening a PR:

```bash
cd mhm
npm test
npm run build

cd src-tauri
cargo check
cargo test
```

If you changed Rust code, also run:

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

CI gates every command in this section, `cargo fmt` included. A branch that skips
the format check locally fails the build even when every test passes.

If you touched a core PMS lifecycle — reservations, stays, groups, or backup — run the smoke gate from `mhm/`:

```bash
npm run verify:full
```

## Coding Conventions

- Keep changes scoped and easy to review.
- Prefer editing existing files over rewriting large areas.
- TypeScript should stay strict and type-safe.
- Rust should compile cleanly and pass clippy.
- Avoid committing secrets, local paths, exported browser cookies, or internal agent files.

## PMS Architecture Guardrails

- Core PMS includes rooms, stays, reservations, guests, housekeeping, billing, invoices, groups, night audit, settings, and auth.
- Experimental runtime includes gateway, MCP, agent runtime, observer streams, digest, Telegram, CEO, and OpenAI surfaces.
- Experimental disabled means normal PMS operation has no experimental background tasks, no required external API keys, no Telegram/OpenAI/MCP/gateway config, no agent direct PMS table mutation, and no experimental UI in the normal profile.
- Business writes must enter through Tauri commands and continue through service/lifecycle modules.
- Reads should use Tauri commands and query modules when read SQL is shared, growing, or part of a review hotspot.
- The intended orchestration is `UI -> command -> service/lifecycle` for writes and `UI -> command -> query` for reads.
- Command safety is core PMS infrastructure: preserve actor, command name, idempotency key, canonical payload hash, timestamp, request context, stable lock keys, audit writes, command ledger metadata, and transactional outbox writes.
- UI, bots, agents, and integrations must not mutate PMS tables directly.
- Do not rename `mhm/` until canonical docs, CI, smoke tests, and normal-profile runtime boundaries are stable.

## Commits

Use Conventional Commits where practical:

- `feat:`
- `fix:`
- `docs:`
- `refactor:`
- `test:`
- `chore:`

## Pull Requests

Before opening a PR:

- explain the problem and the chosen approach
- list user-visible changes
- list verification commands and results
- note any follow-up work or known limitations

Prefer small, focused PRs over broad mixed-scope changes.

## Issues

- Use bug reports for concrete defects with repro steps.
- Use feature requests for user-facing improvements.
- Use Discussions for open-ended questions and design discussion if enabled.

## Security

Do not open public issues for security-sensitive findings. See [SECURITY.md](SECURITY.md).
