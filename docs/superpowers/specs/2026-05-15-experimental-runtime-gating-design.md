# Experimental Runtime Gating Design

Date: 2026-05-15

Issues: #141, #142

Planned PR title: `refactor: gate experimental gateway and agent runtime`

## Goal

Make gateway, MCP, agent, digest, Telegram, CEO, and OpenAI runtime surfaces disabled by default in the normal PMS profile. For this slice, normal PMS startup must not start gateway or agent/digest/Telegram background tasks, require external gateway/agent credentials, or expose gateway/agent UI by accident.

The implementation should combine #141 and #142 because both issues share the same runtime quarantine goal and the relevant startup paths are clear. The high-risk portion is agent supervisor reconciliation, so changes there must stay narrow and be verified carefully.

## Scope

In scope:

- Add explicit experimental runtime opt-in helpers in `mhm/src-tauri/src/runtime_config.rs`.
- Gate gateway startup in `mhm/src-tauri/src/lib.rs`.
- Gate agent supervisor reconciliation in `mhm/src-tauri/src/agent/supervisor.rs`.
- Hide gateway/MCP/CEO Agent UI in the normal frontend profile.
- Preserve existing gateway and agent code; do not delete experimental features.
- Audit direct SQL writes in `mhm/src-tauri/src/agent` and `mhm/src-tauri/src/gateway`.
- Refactor any production direct write from agent modules into PMS business tables if such a write bypasses the command/service boundary.
- Document the experimental disabled profile and the allowed agent-owned runtime tables.

Out of scope:

- New gateway, MCP, agent, digest, Telegram, CEO, or OpenAI capabilities.
- PMS command semantics changes.
- Replacing the command executor architecture.
- Removing existing gateway or agent modules.
- Gating unrelated runtime surfaces such as outbox dispatchers unless a tiny helper is required for naming consistency. Outbox dispatchers remain a documented residual risk for the separate F5 issue.
- Renaming `mhm/` or reorganizing unrelated docs.

## Current State

`mhm/src-tauri/src/lib.rs` currently starts the MCP gateway during app setup unless `CAPYINN_DISABLE_GATEWAY` is set. It also creates an `AgentSupervisor` and calls `agent::supervisor::reconcile_managed_supervisor` unless `CAPYINN_DISABLE_CEO_TELEGRAM` is set.

`mhm/src/App.tsx` checks `gateway_get_status` after authentication and always renders a gateway badge that says either `MCP Gateway` or `Gateway Off`. It also listens for `mcp_reservation_created` and shows an AI-agent toast. `mhm/src/pages/settings/index.tsx` always includes the `MCP Gateway` settings section and shows `CEO Agent` settings to admins.

The working tree already contains draft runtime flag helpers in `runtime_config.rs` and a supervisor test that expects agent workflows to stop when experimental agent runtime is absent. Implementation must preserve and complete those existing local changes rather than replacing them wholesale.

## Approved Approach

Use a single explicit experimental runtime gate and combine #141 plus #142 in one implementation.

Environment flags:

- `CAPYINN_EXPERIMENTAL_RUNTIME=true` enables every experimental runtime surface covered by this slice.
- `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME=true` enables only the gateway runtime surface.
- `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME=true` enables only the agent/digest/Telegram CEO runtime surface.
- `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME=true` is retained only as a shared helper for future peripheral runtime work; this issue does not consume it.

Existing disable flags remain safety overrides:

- `CAPYINN_DISABLE_GATEWAY=true` disables gateway even if gateway experimental runtime is enabled.
- `CAPYINN_DISABLE_CEO_TELEGRAM=true` disables agent supervisor workflows even if agent experimental runtime is enabled.

The effective rule is positive opt-in first, disable override second.

## Runtime Status Source Of Truth

Backend runtime config is the source of truth for frontend profile decisions. Do not use build-time `VITE_*` flags for this slice because packaged app UI could drift from backend process environment.

Add a narrow Tauri command named `get_experimental_runtime_status` that reads backend runtime config and returns the effective frontend gates:

- `experimental_runtime_enabled`
- `gateway_runtime_enabled`
- `agent_runtime_enabled`
- `gateway_disabled_by_override`
- `agent_disabled_by_override`

`gateway_runtime_enabled` should be true only when the gateway experimental flag is enabled and `CAPYINN_DISABLE_GATEWAY` is false. `agent_runtime_enabled` should be true only when the agent experimental flag is enabled and `CAPYINN_DISABLE_CEO_TELEGRAM` is false.

Frontend code should call this profile command once after authentication and use the returned booleans to decide whether to render gateway/MCP/CEO Agent surfaces. Calling this profile command in the normal profile is allowed because it does not start or expose an experimental runtime; it only reports whether the current process opted into one.

## Runtime Gate Design

`runtime_config.rs` should own the flag parsing helpers. The normal default is false for all experimental helpers.

Gateway startup:

1. App setup initializes the database and core PMS state as before.
2. Gateway startup checks `experimental_gateway_runtime_enabled()`.
3. If gateway experimental runtime is false, do not call `gateway::start_gateway`.
4. If gateway experimental runtime is true but `CAPYINN_DISABLE_GATEWAY` is true, do not start the gateway.
5. Only if the experimental gate is true and the disable override is false should the gateway start.

Agent startup and reconciliation:

1. App setup still manages `AgentSupervisor` so commands have a stable state object.
2. Startup calls `reconcile_managed_supervisor`.
3. `reconcile_managed_supervisor` first checks `experimental_agent_runtime_enabled()`.
4. If false, it shuts down existing workflows and returns `Ok(())` without starting chat or digest tasks.
5. If true but `CAPYINN_DISABLE_CEO_TELEGRAM` is true, it shuts down existing workflows and returns `Ok(())`.
6. Only after both gates pass should it read Telegram/digest config and evaluate existing readiness gates.

This preserves existing admin config commands. Changing CEO Telegram settings may update settings while experimental runtime is disabled, but it must not start background workflows until the process is launched with the explicit experimental agent flag.

## Frontend UI Gate

Normal PMS profile must not display gateway/MCP/CEO Agent as ordinary product surfaces.

Frontend behavior:

- `App.tsx` must not call `gateway_get_status` in the normal profile.
- `App.tsx` must not render a red `Gateway Off` badge in the normal profile.
- `App.tsx` must not subscribe to `mcp_reservation_created` or show AI-agent reservation toasts in the normal profile.
- `mhm/src/pages/settings/index.tsx` must not render the `MCP Gateway` sidebar item in the normal profile.
- `mhm/src/pages/settings/index.tsx` must not render the `CEO Agent` sidebar item in the normal profile.
- `GatewaySection` can keep its current behavior when rendered in an experimental gateway profile.
- `CeoAgentSection` can keep its current behavior when rendered in an experimental agent profile.
- `gateway_get_status` should expose `experimental_enabled` for the experimental settings panel, while returning `running: false` and `port: null` when the gateway runtime is not enabled.

The frontend should use one small profile helper or hook, for example `mhm/src/lib/experimentalProfile.ts`, that consumes `get_experimental_runtime_status` and exposes typed booleans to components. Components should import that helper rather than scattering command calls or environment checks.

## Agent And Gateway Write Audit

Direct SQL writes in `mhm/src-tauri/src/agent` and `mhm/src-tauri/src/gateway` should be classified into three groups.

Allowed agent-owned runtime state:

- `agent_sessions`
- `agent_audit_events`
- `agent_memory_items`
- `agent_digest_runs`

These tables are not PMS truth. They may remain in agent store/scheduler modules when they are narrow runtime metadata writes and do not mutate PMS business state.

Command-boundary writes:

- CEO cloud data opt-in.
- CEO Telegram config.
- CEO Telegram secret presence.
- CEO Telegram offset persistence.
- CEO digest config.
- CEO digest delivery chat ID.
- Gateway write tools that delegate to validated PMS commands or services.

These should keep using `WriteCommandExecutor`, `WriteCommandContext`, lock keys, idempotency, and audit where already present.

Gateway management writes:

- `gateway_generate_key` should reject with a controlled error when effective gateway runtime is disabled.
- When effective gateway runtime is enabled, `gateway_generate_key` may continue writing `gateway_api_keys` as gateway-owned experimental state.
- `gateway_get_status` remains read-only and may return disabled status without error.

Forbidden production direct writes:

- Any production write from `agent/` into PMS business tables such as `rooms`, `bookings`, `guests`, `transactions`, `folio_lines`, `invoices`, `room_calendar`, housekeeping tables, or other PMS truth tables.
- Any gateway or agent mutation exposed to external callers that bypasses the existing validated command/service boundary.

If implementation finds a forbidden production direct write, it must refactor that path through the existing command/service boundary. If a SQL write is test-only fixture setup, it may remain in test scope and should be called out as test-only in the implementation evidence.

Current audit expectation from exploration:

- CEO chat production path dispatches read-only CEO tools.
- Local receptionist demo reads guest-facing PMS context and does not write PMS business state.
- Direct PMS writes seen under agent modules appear to be test fixtures.
- Gateway reservation write tools call `commands::do_create_reservation`, `commands::do_cancel_reservation`, and `commands::do_modify_reservation`.
- Gateway API key storage writes `gateway_api_keys`, which is gateway-owned experimental state, not PMS business truth.

## Error Handling

Runtime disabled states should be quiet and normal:

- Startup should log that an experimental runtime is disabled by default, not emit an error.
- `gateway_get_status` should return a normal status object, not fail, when the runtime is disabled.
- `gateway_generate_key` should fail closed with a controlled user-facing error when effective gateway runtime is disabled.
- `get_experimental_runtime_status` should always return a status object and should not require gateway, Telegram, OpenAI, or agent config.
- Agent settings commands should still return their normal command results after config updates; reconcile should be a no-op shutdown when experimental agent runtime is disabled.
- If an experimental runtime is enabled but missing its existing readiness prerequisites, preserve the current gate behavior and errors.

Disable override flags should remain explicit in logs so operators can distinguish "not opted in" from "opted in but force-disabled."

## GitNexus Impact Notes

GitNexus index was refreshed before exploration with `npx gitnexus analyze`.

Pre-change impact findings:

- `start_gateway`: LOW risk. One direct caller, `mhm/src-tauri/src/lib.rs::run`; one affected process, app startup.
- `gateway_get_status`: LOW risk. No indexed upstream callers.
- `gateway_generate_key`: LOW risk. No indexed upstream callers.
- `GatewaySection`: LOW risk. No indexed upstream callers.
- `SettingsPage`: LOW risk. No indexed upstream callers.
- `experimental_gateway_runtime_enabled`: LOW risk in current draft state. No upstream callers yet.
- `experimental_agent_runtime_enabled`: LOW risk in current draft state. No upstream callers yet.
- `reconcile_managed_supervisor`: HIGH risk. Direct callers include app startup and agent settings commands. Affected processes include `run`, `set_ceo_cloud_data_opt_in`, and `set_ceo_telegram_config`.

Because `reconcile_managed_supervisor` is HIGH risk, implementation must keep the edit narrow, report the risk before editing, and cover startup/reconcile behavior with focused Rust tests.

Before committing implementation changes, run `gitnexus_detect_changes()` and verify affected symbols and flows match the planned runtime-gating scope.

## Testing

Rust tests:

- `runtime_config.rs` verifies all experimental flags are disabled by default.
- `runtime_config.rs` verifies the master experimental flag enables gateway, agent, and peripheral helpers.
- `runtime_config.rs` verifies individual gateway and agent flags only enable their matching surfaces.
- The experimental runtime status command reports effective gateway and agent gates, including disable overrides.
- Gateway startup logic is covered either through extracted helper tests or focused status/startup-adjacent tests.
- `gateway_generate_key` fails closed when effective gateway runtime is disabled.
- `agent::supervisor` verifies disabled experimental agent runtime shuts down chat and digest workflows.
- `agent::supervisor` verifies `CAPYINN_DISABLE_CEO_TELEGRAM` still force-disables workflows when experimental agent runtime is enabled.

Frontend tests:

- Normal profile does not call `gateway_get_status`.
- Normal profile does not render the gateway badge.
- Normal profile does not subscribe to `mcp_reservation_created` or show the AI-agent reservation toast.
- Normal profile does not render the `MCP Gateway` settings section.
- Normal profile does not render the `CEO Agent` settings section.
- Experimental gateway profile renders the `MCP Gateway` settings section and preserves `GatewaySection` behavior.
- Experimental agent profile renders the `CEO Agent` settings section and preserves `CeoAgentSection` behavior.

SQL audit validation:

```bash
rg -n "sqlx::query|query_as|query_scalar|SqlitePool|Pool<Sqlite>|Transaction<|execute\\(" mhm/src-tauri/src/agent mhm/src-tauri/src/gateway
rg -n "INSERT|UPDATE|DELETE|CREATE|DROP|ALTER" mhm/src-tauri/src/agent mhm/src-tauri/src/gateway
```

The implementation summary should classify every production write found by these scans as agent-owned runtime state, gateway-owned runtime state, command-boundary write, read-only query, or test fixture. Any production PMS business-table write found outside a command/service boundary must be fixed before completion.

Validation commands:

```bash
cd mhm && npm test
cd mhm/src-tauri && cargo test
rg -n "gateway" mhm/src mhm/src-tauri/src docs
rg -n "CAPYINN_EXPERIMENTAL|CAPYINN_DISABLE_GATEWAY|CAPYINN_DISABLE_CEO_TELEGRAM" mhm/src-tauri/src docs README.md
gitnexus_detect_changes()
```

## Acceptance Criteria

- Gateway runtime does not start unless `CAPYINN_EXPERIMENTAL_RUNTIME` or `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME` is explicitly enabled.
- `CAPYINN_DISABLE_GATEWAY` still disables gateway even when experimental gateway runtime is enabled.
- `gateway_generate_key` rejects when effective gateway runtime is disabled.
- Frontend profile gates come from backend runtime status, not build-time `VITE_*` flags.
- Normal app profile does not call gateway status, show a gateway badge, subscribe to MCP reservation events, or show the `MCP Gateway` settings entry.
- Agent chat and digest workflows do not start unless `CAPYINN_EXPERIMENTAL_RUNTIME` or `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME` is explicitly enabled.
- `CAPYINN_DISABLE_CEO_TELEGRAM` still disables agent workflows even when experimental agent runtime is enabled.
- Normal app profile does not show the `CEO Agent` settings entry.
- Normal PMS operation requires no gateway, MCP, OpenAI, Telegram, CEO-agent, or digest config.
- Agent-owned runtime tables are documented as allowed experimental state.
- No production agent module directly mutates PMS business tables outside the validated command/service boundary.
- Gateway write tools continue delegating to existing validated command/service entry points.
- Core PMS frontend and Rust tests pass with experimental runtime disabled.

## Non-Goals And Guardrails

- Do not delete gateway or agent code.
- Do not add any new agent capability.
- Do not change PMS command semantics.
- Do not weaken idempotency, lock, audit, or outbox behavior for existing command/service writes.
- Do not convert agent-owned metadata tables into PMS command-boundary writes unless they mutate PMS truth.
- Do not combine this work with folder renames or unrelated frontend shell refactors.
