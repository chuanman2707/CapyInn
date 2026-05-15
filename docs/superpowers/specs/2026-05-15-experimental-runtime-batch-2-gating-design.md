# Experimental Runtime Batch 2 Gating Design

Date: 2026-05-15

Issue: #143

Planned PR title: `refactor: gate experimental agent and observer runtime surfaces`

## Goal

Finish the second runtime-gating slice for experimental surfaces without adding a confusing new universal flag. Normal PMS operation must run without digest, Telegram, CEO agent, OpenAI, gateway, MCP, observer, or outbox consumer runtime dependencies.

Core command safety must remain intact. Business commands may still write transactional `outbox_events` inside the same atomic mutation, because those records are part of the PMS safety boundary. Runtime consumers of those records are experimental and must not run in the normal profile.

## Approved Direction

Use the existing clearly named runtime gates:

- `CAPYINN_EXPERIMENTAL_RUNTIME=true` enables every experimental runtime surface.
- `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME=true` enables CEO agent, Telegram chat runtime, digest scheduler, and OpenAI-backed paths.
- `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME=true` enables MCP gateway and the observer stream route hosted by that gateway.

Do not use `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME` for this issue. The flag name is too broad for digest, Telegram, and CEO runtime, and using it as a universal plug would create naming debt. If the helper remains in `runtime_config.rs`, it should be treated as reserved and unused by #143.

Existing disable overrides still win:

- `CAPYINN_DISABLE_CEO_TELEGRAM=true` force-disables CEO Telegram and digest workflows even when agent experimental runtime is enabled.
- `CAPYINN_DISABLE_GATEWAY=true` force-disables gateway and observer exposure even when gateway experimental runtime is enabled.

## Current State

Batch 1 already gates gateway startup, agent supervisor reconciliation, gateway UI, CEO Agent UI, and MCP reservation frontend listeners by backend runtime status. The current branch already contains `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME`, `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME`, and a reserved `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME` helper.

The remaining issue #143 risk is not the core outbox write. The risk is accidental startup or exposure of the surrounding runtime paths:

- CEO Telegram polling.
- CEO digest scheduler.
- OpenAI-backed agent runtime work.
- Observer stream exposure at `/observer/events`.
- Outbox dispatcher loops when real subscribers are registered.

## Scope

In scope:

- Verify digest and Telegram startup remain controlled by `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME`.
- Verify observer exposure remains controlled by `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME` through gateway startup.
- Add guardrails so observer is not exposed in a future standalone route without an experimental gateway gate.
- Clarify outbox boundaries: transactional outbox writes are core; dispatcher subscribers and observer streaming are experimental consumers.
- Add focused tests for inactive outbox dispatcher behavior when no subscribers are registered.
- Document that `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME` is not consumed by this issue.

Out of scope:

- Do not remove transactional outbox writes from business commands.
- Do not change command idempotency, locking, audit, or PMS state machine behavior.
- Do not add new agent, gateway, observer, Telegram, digest, or OpenAI capabilities.
- Do not rename `mhm/`.
- Do not turn `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME` into a universal batch flag.

## Runtime Design

### Core PMS Flow

Normal PMS writes keep their existing command boundary:

```text
UI
  -> Tauri command
  -> validate, authorize, idempotency, lock
  -> mutate PMS tables
  -> audit
  -> insert transactional outbox event when required
  -> commit
```

This path must work with all experimental runtime flags absent.

### Agent, Telegram, Digest, And OpenAI

`reconcile_managed_supervisor` remains the central boundary for starting or stopping CEO chat and digest workflows.

When `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME` and `CAPYINN_EXPERIMENTAL_RUNTIME` are both absent:

- chat workflow shuts down;
- digest workflow shuts down;
- no Telegram polling starts;
- no digest scheduler starts;
- no Telegram or OpenAI secret is required for normal startup.

When agent experimental runtime is enabled:

- existing CEO cloud opt-in, Telegram config, digest config, token, model, and delivery-chat readiness gates continue to apply;
- missing readiness dependencies should behave exactly as they do today;
- `CAPYINN_DISABLE_CEO_TELEGRAM=true` still force-disables runtime workflows.

CEO Agent config commands may still persist admin configuration while runtime is disabled, but they must not start workflows unless the effective agent runtime gate is enabled.

### Gateway, MCP, And Observer

Observer belongs to the gateway runtime for this slice because `/observer/events` is mounted inside the gateway server. If gateway startup is disabled, observer is not exposed.

When `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME` and `CAPYINN_EXPERIMENTAL_RUNTIME` are both absent:

- gateway server does not start;
- `/mcp` is unavailable;
- `/observer/events` is unavailable;
- gateway API key management stays disabled.

When gateway experimental runtime is enabled:

- existing gateway auth remains required;
- `/observer/events` remains behind the same protected gateway router;
- `CAPYINN_DISABLE_GATEWAY=true` force-disables both gateway and observer exposure.

Add a lightweight guardrail test so future contributors do not mount `observe_events` outside the protected gateway router without adding an explicit experimental gate.

### Outbox Dispatcher

Transactional `outbox_events` writes are core PMS safety. The dispatcher is a runtime consumer.

The current app starts `start_outbox_dispatcher(pool, Vec::new())`, which is inactive because there are no subscribers. Preserve this behavior and cover it with focused tests:

- no subscribers means inactive handle;
- inactive handle has no running task and can be shut down safely;
- future subscribers must be started only from an explicit experimental runtime path.

Do not gate or remove `insert_outbox_event_tx`.

## Error Handling

Experimental runtime disabled states are normal and quiet:

- startup should log disabled runtime paths at info level, not error level;
- normal profile should not show gateway-off or CEO-agent UI warnings;
- missing Telegram/OpenAI/gateway configuration must not be required in the normal profile;
- observer being unavailable in the normal profile is expected because gateway is not started;
- outbox dispatcher with no subscribers should report inactive, not failed.

If a runtime is explicitly enabled but readiness dependencies are missing, preserve the current readiness gate behavior and user-facing errors.

## GitNexus Impact Notes

Pre-design impact checks:

- `start_outbox_dispatcher`: LOW risk. One direct caller, app startup.
- `observe_events`: LOW risk. No indexed upstream callers.
- `build_observer_sse_stream`: LOW risk. Direct caller is `observe_events`.
- `run_ceo_digest_scheduler`: LOW risk. Direct callers include supervisor digest task and a scheduler test.
- `experimental_peripheral_runtime_enabled`: LOW risk. No upstream callers.
- `run` in `lib.rs`: LOW risk. No upstream callers.
- `reconcile_managed_supervisor`: HIGH risk. Direct callers include app startup and multiple CEO agent settings commands. Affected flows include `run`, `set_ceo_cloud_data_opt_in`, and `set_ceo_telegram_config`.

Because `reconcile_managed_supervisor` is HIGH risk, implementation should avoid changing its semantics unless a focused guard test proves a missing runtime boundary. Any edit there must stay narrow and preserve existing readiness behavior after the experimental gate passes.

## Testing

Rust tests:

- `runtime_config` confirms `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME` does not enable gateway or agent runtime.
- `agent::supervisor` confirms disabled agent runtime shuts down chat and digest workflows.
- `agent::supervisor` confirms `CAPYINN_DISABLE_CEO_TELEGRAM` force-disables workflows even when agent runtime is enabled.
- `gateway::server` confirms `/observer/events` stays inside the protected gateway router.
- `outbox` confirms `start_outbox_dispatcher(pool, Vec::new())` returns an inactive handle and does not spawn a loop.
- `lib.rs` confirms runtime status reports disabled agent and gateway gates by default.

Frontend and guardrail tests:

- normal profile does not render Gateway or CEO Agent settings.
- normal profile does not call gateway status or subscribe to MCP reservation events.
- agent guardrail tests continue to assert digest and chat runtimes do not mutate PMS business tables.
- a guardrail test states that transactional outbox writes are core while dispatcher and observer are experimental runtime consumers.

Validation commands:

```bash
cd mhm && npm test
cd mhm/src-tauri && cargo test
cd mhm/src-tauri && cargo clippy --all-targets -- -D warnings
rg -n "CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME|experimental_peripheral_runtime_enabled|start_outbox_dispatcher|observer/events|run_ceo_digest_scheduler|reconcile_managed_supervisor" mhm/src-tauri/src docs
```

Before committing implementation changes, run GitNexus change detection and confirm affected flows match this runtime-gating scope.

## Acceptance Criteria

- Normal PMS starts with no gateway, MCP, observer, Telegram, digest, CEO, OpenAI, or outbox subscriber runtime requirement.
- Business commands still write transactional outbox events where required.
- Digest and Telegram workflows require `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME=true` or `CAPYINN_EXPERIMENTAL_RUNTIME=true`.
- Gateway and observer exposure require `CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME=true` or `CAPYINN_EXPERIMENTAL_RUNTIME=true`.
- `CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME` is not used as a universal plug for #143.
- `CAPYINN_DISABLE_CEO_TELEGRAM` and `CAPYINN_DISABLE_GATEWAY` remain force-disable overrides.
- Outbox dispatcher with no subscribers is explicitly inactive and safe in the normal profile.
- Core PMS tests pass with experimental runtime flags absent.
