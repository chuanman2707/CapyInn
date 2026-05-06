# CEO Agent Safety Foundation

Issues: #117, #118, #119, and part of #125

Parent scope: #76 Agentic AI and outbox integration roadmap

Date: 2026-05-05

## Status

Approved design direction, pending implementation plan.

This spec implements the safety foundation for the Phase 1 CEO secretary. It creates CapyInn-owned agent boundaries, data-sensitivity policy, persisted CEO cloud-data opt-in, session/audit/memory storage, and verification checks.

This spec does not enable Telegram, OpenAI calls, tool-gated chat, hourly digest delivery, or PMS writes through an agent.

## Plain-Language Goal

CapyInn needs a CEO agent foundation before it adds a real Telegram secretary runtime.

The foundation must answer four questions before any cloud LLM sees PMS data:

- Who is the agent acting as?
- Which channel and actor is allowed to talk to it?
- Which tools are visible, and what data sensitivity do those tools carry?
- Has the CEO explicitly allowed cloud processing of CEO-sensitive PMS data?

The foundation also records session, audit, and memory boundaries in the database so later runtime work does not invent a separate truth store.

## Chosen Approach

Use a contract-first runtime skeleton.

The implementation will add runtime-shaped modules and database storage now, but all external execution remains disabled. This keeps the architecture ready for #120 through #123 without violating #117's acceptance criterion that no Telegram, OpenAI, or PMS write behavior is enabled by this slice.

Rejected alternatives:

- A policy-only slice would be faster, but would leave session, audit, and memory boundaries too vague for #117 and #119.
- A real runtime slice would overlap #120, #121, #122, #123, and possibly #124.

## Relationship To Existing Specs

This spec builds on:

- `docs/superpowers/specs/2026-05-04-agentic-integration-guardrails-design.md`
- `docs/superpowers/specs/2026-05-05-agentic-ai-roadmap-design.md`
- `docs/superpowers/specs/2026-05-02-verification-gate-supervised-write-enablement-design.md`

Those specs already establish:

- PMS SQLite state is the source of truth.
- Agent memory is not PMS truth.
- The gateway is loopback-only by default.
- High-risk MCP writes remain policy-gated.
- Phase 1 CEO secretary is read/report-only.

This spec adds concrete agent contracts, persistence, opt-in controls, and verification checks for the first CEO secretary foundation.

## Architecture

Add a new backend `agent` module with narrow CapyInn-owned contracts.

Core model types:

- `AgentRole`: initially only `CeoSecretary`.
- `AgentChannel`: initially represents `Telegram`, but Telegram execution is disabled.
- `AgentProvider`: initially represents `OpenAI`, but provider execution is disabled.
- `MutationRisk`: `ReadOnly`, `LowWrite`, `HighWrite`.
- `DataSensitivity`: `PublicHotelInfo`, `GuestScoped`, `StaffOperational`, `CeoSensitive`.
- `AgentToolMeta`: name, description, mutation risk, data sensitivity, role allowlist, and forbidden dynamic capability flags.
- `AgentSession`: durable metadata for a conversation or scheduled run.
- `AgentAuditEvent`: sanitized event metadata.
- `AgentMemoryItem`: non-authoritative preferences, summaries, and notes.

The runtime skeleton may expose internal functions such as `handle_agent_message(...)` or equivalent service methods. In this slice, those methods fail closed:

- unknown or unpaired channel actors cannot create prompts
- missing or revoked cloud opt-in blocks provider request construction for CEO-sensitive data
- provider execution returns a disabled or not-configured policy result
- channel delivery returns a disabled or not-configured policy result
- no Telegram polling, webhook, or send path is started
- no OpenAI HTTP request is made
- no PMS mutation path is exposed

## Component Boundaries

### Agent Provider Boundary

The provider boundary is a narrow interface for future cloud LLM calls.

This slice defines:

- provider identity, initially `OpenAI`
- scrubbed provider error shape
- policy checks that must pass before provider request construction
- tests proving no provider call can be made when runtime is disabled or cloud opt-in is false

This slice does not include:

- OpenAI API client
- API key loading
- prompt sending
- streaming responses
- model tool-call loop

### Agent Channel Boundary

The channel boundary represents where a message came from and where a future response would go.

This slice defines:

- channel identity, initially `Telegram`
- stable numeric channel actor ID requirement for Telegram CEO binding
- display name and username as metadata only
- unknown or unpaired actor denial before prompt construction

This slice does not include:

- Telegram bot token
- polling
- webhook ingress
- message sending
- public network ingress

### Agent Tool Boundary

The agent tool boundary is separate from dynamic MCP tool discovery.

The CEO secretary will eventually use a static read-only/reporting catalog. This slice creates the metadata and verification boundary first. Runtime tool execution is not implemented here.

The model must never receive:

- SQL/database handles
- shell tools
- file tools
- browser tools
- generic HTTP tools
- generic MCP discovery tools
- PMS write tools

### Session, Audit, And Memory Boundary

Sessions, audit, and memory are durable, but they are not PMS truth.

Sessions store conversation or run metadata. Audit events store sanitized facts about policy decisions and future tool/provider activity. Memory stores non-authoritative preferences, summaries, and notes.

None of these tables should store raw prompts, raw responses, raw PMS extracts, canonical booking state, room availability truth, payment truth, folio truth, invoice truth, ledger truth, housekeeping truth, or audit truth by default.

## Retention Policy

This slice must make retention explicit even though it does not run the real agent runtime.

Required retention constants:

- `raw_prompt_retention`: `not_stored`
- `raw_response_retention`: `not_stored`
- `raw_tool_output_retention`: `not_stored`
- `raw_provider_error_retention`: `not_stored`
- `session_metadata_retention`: `local_metadata_until_operator_cleanup_v1`
- `audit_metadata_retention`: `local_metadata_until_operator_cleanup_v1`
- `memory_retention`: `local_non_authoritative_until_operator_cleanup_v1`

Required behavior:

- raw prompts are not persisted in this slice
- raw responses are not persisted in this slice
- raw tool outputs are not persisted in this slice
- raw provider errors are not persisted in this slice
- session rows store metadata only
- audit rows store sanitized summaries only
- memory rows store non-authoritative preferences, summaries, and notes only
- future raw prompt, response, tool-output, or provider-error persistence requires a separate design, UI consent, and verification gate

The `agent_sessions.retention_policy` value must use `metadata_only_v1` for this slice. Any other retention policy value should be rejected until a future issue defines it.

## Database Schema

Add migration v18.

The migration must create these tables.

### `agent_sessions`

Purpose: durable metadata for agent conversations and scheduled runs.

Required columns:

- `id TEXT PRIMARY KEY`
- `role TEXT NOT NULL`
- `channel TEXT NOT NULL`
- `channel_actor_id TEXT`
- `status TEXT NOT NULL`
- `uses_memory INTEGER NOT NULL DEFAULT 0`
- `retention_policy TEXT NOT NULL`
- `metadata_json TEXT NOT NULL DEFAULT '{}'`
- `started_at TEXT NOT NULL`
- `last_seen_at TEXT`
- `ended_at TEXT`

Indexes:

- role, channel, channel actor, last seen
- status and started time

### `agent_audit_events`

Purpose: sanitized event stream for opt-in changes, policy denials, future tool calls, and future provider decisions.

Required columns:

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `session_id TEXT`
- `event_type TEXT NOT NULL`
- `actor_id TEXT`
- `role TEXT`
- `channel TEXT`
- `tool_name TEXT`
- `provider TEXT`
- `policy_outcome TEXT NOT NULL`
- `mutation_risk TEXT`
- `data_sensitivity TEXT`
- `summary_json TEXT NOT NULL DEFAULT '{}'`
- `created_at TEXT NOT NULL`

Indexes:

- event type and created time
- session and created time
- role, channel, and created time

### `agent_memory_items`

Purpose: non-authoritative agent memory.

Required columns:

- `id TEXT PRIMARY KEY`
- `role TEXT NOT NULL`
- `scope TEXT NOT NULL`
- `key TEXT NOT NULL`
- `value_json TEXT NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Constraints:

- unique role, scope, key

The memory service must reject forbidden PMS truth categories, including canonical booking, room availability, payment, folio, invoice, ledger, housekeeping, and night-audit truth.

## CEO Cloud-Data Opt-In

Add a persisted, revocable setting for CEO cloud-data processing.

Setting key:

- `ceo_cloud_data_opt_in`

Default:

- `false`

Backend commands:

- `get_ceo_cloud_data_opt_in() -> bool`
- `set_ceo_cloud_data_opt_in(enabled: bool)`

The setter must enforce backend authorization before it starts the command transaction. Allowed actors are authenticated admins or explicitly modeled owner/CEO actors. A receptionist, unauthenticated actor, agent runtime, or integration key must not be able to enable or revoke CEO cloud-data opt-in.

The setter must use an explicit low-risk command boundary, following the existing crash-reporting preference pattern. It should record actor, command name, idempotency, canonical payload hash, timestamp, and command ledger metadata through the existing command executor.

The setter must also write a sanitized `agent_audit_events` row:

- `ceo_cloud_data_opt_in.enabled`
- `ceo_cloud_data_opt_in.revoked`

The setting update and its agent audit event must be written in the same command transaction. If either write fails, both must roll back.

Audit summary must not include raw PMS data, prompts, responses, provider keys, bot tokens, or API keys.

Revoking opt-in must prevent construction of any cloud provider request containing CEO-sensitive PMS data.

## Settings UI

Add a minimal admin-facing Settings UI for CEO cloud-data opt-in.

The UI should:

- show current enabled/disabled state
- let an admin enable or revoke opt-in
- call the new Tauri commands
- rely on backend authorization, not UI-only hiding, for privacy enforcement
- show concise text that cloud LLM processing may receive CEO-sensitive PMS data only when enabled
- state that revoking opt-in blocks cloud calls containing CEO-sensitive PMS data
- state retention explicitly: raw prompts, raw responses, raw tool outputs, and raw provider errors are not stored in this slice; sanitized session/audit metadata is stored locally under `metadata_only_v1`

The UI should not:

- collect OpenAI keys
- collect Telegram bot tokens
- start the CEO secretary
- imply runtime chat is enabled
- expose raw audit rows

## Tool Sensitivity Model

`MutationRisk` values:

- `ReadOnly`: does not mutate PMS state
- `LowWrite`: low-risk business or configuration write through command boundary
- `HighWrite`: high-risk PMS business write through command boundary and approval policy

`DataSensitivity` values:

- `PublicHotelInfo`: safe for public hotel policy or descriptive answers
- `GuestScoped`: safe only after verifying the guest and scope
- `StaffOperational`: safe for authenticated staff operations
- `CeoSensitive`: safe only for CEO or explicitly authorized owner/admin identities

Important rule:

`ReadOnly` does not mean guest-safe. A read-only tool can still reveal guest, revenue, balance, audit, or operational information and must carry the correct sensitivity class.

## Static CEO Tool Metadata

This slice should create static metadata for the CEO secretary tool catalog. It does not need to implement all #122 CEO reporting tools yet.

The metadata must make these rules testable:

- every tool has mutation risk
- every tool has data sensitivity
- CEO-sensitive tools are role-scoped to `CeoSecretary`
- Phase A CEO registry has no write tools
- Phase A CEO registry has no shell, file, browser, generic HTTP, or dynamic MCP discovery tools

Existing MCP write tools such as `create_reservation`, `modify_reservation`, and `cancel_reservation` remain `HighWrite` in the MCP policy model and must not appear in the Phase A CEO registry.

Existing MCP read tools should be classified conservatively. Tools that may reveal bookings, guest details, invoices, unpaid balances, revenue, audit readiness, or operational risk should not be `PublicHotelInfo`.

## Verification Gate Scope

This spec implements the static and policy portion of #125.

Required checks:

- CEO registry contains no write tools
- CEO registry contains no shell/file/browser/generic HTTP/dynamic MCP discovery tools
- every agent tool has mutation risk and data sensitivity
- unknown or unpaired Telegram actors cannot create prompt/provider requests
- cloud provider request construction is blocked when CEO-sensitive data is involved and opt-in is false
- revoked opt-in blocks future CEO-sensitive cloud request construction
- runtime skeleton is disabled by default
- no OpenAI API key or Telegram bot token appears in logs, memory, audit, tool output, or UI text
- agent memory cannot be used as PMS truth
- session and audit records do not store raw PMS extracts by default

Existing high-risk MCP policy behavior must remain unchanged:

- supervised high-risk write attempts return `APPROVAL_REQUIRED`
- read-only mode returns `WRITE_TOOL_DISABLED`
- represented full autonomy still does not launch high-risk writes

## Error Handling

Agent policy failures should use stable, machine-readable errors.

Required error codes, or exact equivalents mapped through the existing app error system:

- `AGENT_RUNTIME_DISABLED`
- `AGENT_CHANNEL_UNPAIRED`
- `AGENT_PROVIDER_DISABLED`
- `AGENT_CLOUD_DATA_OPT_IN_REQUIRED`
- `AGENT_TOOL_NOT_ALLOWED`
- `AGENT_MEMORY_FORBIDDEN_TRUTH`

Errors must be scrubbed before storage or display. They must not include provider secrets, bot tokens, raw prompts, raw responses, raw PMS extracts, SQL errors with sensitive details, or stack traces.

## Testing Strategy

Backend tests:

- migration v18 creates agent tables, columns, indexes, and latest schema version
- opt-in defaults to false
- opt-in setter persists enabled and revoked states through command boundary
- opt-in setter rejects unauthenticated, receptionist, agent runtime, and integration-key actors
- unauthorized opt-in attempts do not change the setting and do not write opt-in audit events
- exact opt-in command retry replays idempotently
- same idempotency key with different opt-in payload conflicts
- opt-in setter writes sanitized agent audit event
- retention constants are exposed and match `not_stored` for raw prompt, response, tool output, and provider error data
- session creation rejects unknown retention policies and accepts `metadata_only_v1`
- runtime skeleton is disabled by default
- unpaired Telegram actor denial happens before prompt construction
- CEO-sensitive provider request construction requires opt-in
- revoked opt-in blocks provider request construction
- memory service rejects forbidden PMS truth categories
- CEO registry contains only read-only allowed tool metadata

Frontend tests:

- Settings UI renders disabled opt-in by default
- admin can toggle opt-in on and off
- failed toggle reverts UI state
- explanatory copy does not imply runtime is enabled

Docs and manifest tests:

- skill, OpenAPI, or manifest document data sensitivity classes
- docs state `ReadOnly` is not automatically guest-safe
- docs state cloud-data opt-in is required and revocable
- docs state audit/session records avoid raw PMS extracts by default

Suggested verification commands after implementation:

```bash
cargo test --manifest-path mhm/src-tauri/Cargo.toml agent:: -- --nocapture
cargo test --manifest-path mhm/src-tauri/Cargo.toml db::tests:: -- --nocapture
cargo test --manifest-path mhm/src-tauri/Cargo.toml command_idempotency::tests::set_ceo_cloud_data_opt_in -- --nocapture
npm test -- --run src/pages/settings
npm test -- --run tests/agentic-guardrails.test.ts
```

Exact command names may change during implementation, but coverage must preserve the behaviors above.

## Acceptance Mapping

#117:

- Provider, channel, and tool boundaries are explicit.
- Session, audit, and memory tables exist with clear retention and truth boundaries.
- Agent memory is non-authoritative and forbidden from PMS truth categories.
- Telegram, OpenAI, and PMS write behavior remain disabled.

#118:

- Tool metadata includes mutation risk and data sensitivity.
- `ReadOnly` is not treated as guest-safe.
- Data classes include `PublicHotelInfo`, `GuestScoped`, `StaffOperational`, and `CeoSensitive`.
- CEO-sensitive tools are CEO-role scoped.
- Future guest tools cannot inherit CEO read tools by default.

#119:

- CEO-sensitive cloud provider request construction requires persisted opt-in.
- Revoking opt-in blocks future cloud requests containing CEO-sensitive PMS data.
- Retention posture is explicit: raw prompts, responses, tool outputs, and provider errors are `not_stored`; sanitized session/audit metadata uses `metadata_only_v1`.
- Provider data-use and retention posture is documented at a high level.
- Audit/session records avoid full raw PMS extracts by default.
- Backend authorization prevents non-admin, non-owner, agent runtime, or integration actors from changing CEO cloud-data opt-in.

#125 partial:

- Telegram actor denial is tested before prompt construction.
- CEO registry contains no write, shell, file, browser, generic HTTP, or dynamic discovery tools.
- Secret markers are not persisted in audit/session/memory/UI text.
- Agent memory cannot affect PMS query results because it is not connected to PMS query services and rejects PMS truth categories.

## Out Of Scope

This spec does not:

- implement Telegram polling
- implement Telegram webhooks
- implement Telegram message delivery
- implement OpenAI API calls
- implement model tool calling
- implement static CEO reporting tools from #122
- implement tool-gated chat loop from #123
- implement hourly digest from #124
- add guest receptionist tools
- add voice receptionist tools
- expose agent PMS writes
- add approval UI
- loosen MCP high-risk write policy
- add generic tool discovery

## Implementation Notes

Before editing existing functions, classes, or methods, run GitNexus impact analysis for the target symbol and report direct callers, affected processes, and risk level. If risk is HIGH or CRITICAL, warn before editing.

Run GitNexus `detect_changes` before committing implementation changes.

Keep edits targeted. This slice should create the foundation and tests without broad refactoring of the PMS command boundary, gateway server, or existing read/write business flows.
