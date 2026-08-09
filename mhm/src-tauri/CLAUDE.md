# Rust backend (`mhm/src-tauri`)

Tauri v2 + SQLite. ~71k lines. `lib.rs` registers the command surface; `main.rs` is the native entrypoint.

## Layer map

Sizes are a rough guide to where the weight sits, measured 2026-08-01. They drift — do not treat them as current.

| Directory | Size | Role |
| --- | --- | --- |
| `commands/` | ~6k | Outermost boundary — the `#[tauri::command]` surface. Nothing inner may depend on it. |
| `services/` | ~19k | Business lifecycles (booking, stay, group, setup). The bulk of the domain logic. |
| `queries/` | ~4k | Read path. UI → command → query → SQLite, bypassing services. |
| `repositories/` | ~1k | Write path persistence + transactions. |
| `domain/` | ~1k | Shared business rules and types. |
| `db/` | ~2k | Connection, `migrations.rs`, `money.rs`, `outbox.rs`, row mapping. |
| `agent/` | ~10k | Experimental runtime — agent loop, tools, memory. |
| `gateway/` | ~3k | Experimental runtime — MCP gateway, loopback-only by default. |
| `declaration/` | ~4k | Khai báo tạm trú (temporary residence declaration). |
| `backup/` | ~2k | Backup, restore, and restore drills. |

Cross-cutting files at `src/` root: `money.rs`, `money_migration.rs`, `outbox.rs`, `command_ledger.rs`, `command_idempotency.rs`, `command_recovery.rs`, `app_error.rs`, `architecture_guard.rs`.

## Money

`MoneyVnd = i64` in `money.rs`. Integer VND, always. Never `f64` for an amount.

- Validate through `validate_transport_money_vnd` / `validate_non_negative_money_vnd` — values must stay inside the JS-safe integer range because they cross the Tauri IPC boundary as JSON numbers.
- Percentages take `f64` (`percentage_money_line`), the resulting amount does not.
- `npm run verify:money` (from `mhm/`) scans this tree for decimal literals on money-named fields. It matches by **field name** — a new money column whose name contains no money keyword slips through the net. When you add one, register it in `money_migration.rs` and in `scripts/verify/no-float-money.mjs`.
- `bookings.guests` is a headcount, not money. Do not register it anywhere as a money column.
- A manual rate is a **rate per night** (`bookings.rate_overridden_at` + `pricing_snapshot.manual_rate`), never a stored total. Anything that recomputes `total_price` for an overridden booking must multiply the stored rate by *that operation's* nights and re-validate `MAX_RATE_PER_NIGHT_VND` on the way out of the JSON column — the value comes from a free-form column, not from freshly validated input. Falling back to the engine when the snapshot is unreadable recreates the bug the column exists to prevent.

## Command safety

Every business write is one atomic unit: validate → authorize → lock → mutate → audit → outbox → commit. Retryable commands are idempotent by key + canonical payload hash (`command_idempotency/`). External effects go through `outbox.rs` inside the same transaction — never a direct HTTP call, webhook, or notification inside a mutation. Ledger and folio rows are append-only; correct with reversal rows.

The `pms-command-safety` skill carries the full rule set and loads automatically when you edit here.

## Tests

```bash
cargo test                                    # from this directory
cargo clippy --all-targets -- -D warnings     # clippy is deny-warnings in CI
cargo fmt -- --check
```

Targeted runs, from `mhm/`:

```bash
cargo test --manifest-path src-tauri/Cargo.toml services::booking::tests:: -- --nocapture
```

Tests are inline `mod tests` next to the code, not a separate tree. `architecture_guard.rs` holds the fitness tests for layer direction — if it goes red, the fix is the dependency, not the test.
