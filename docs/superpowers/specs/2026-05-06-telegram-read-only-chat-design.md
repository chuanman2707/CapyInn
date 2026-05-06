# Telegram Read-Only Chat

Issues: #120, #121, #122, #123, and #125

Parent scope: #76 Agentic AI and outbox integration roadmap

Date: 2026-05-06

## Status

Approved design direction, pending implementation plan.

This spec implements the first CEO-visible vertical slice for the Phase 1 agent: the CEO sends a Telegram message and receives a grounded answer from PMS read tools.

This slice enables a local outbound Telegram runtime, OpenAI tool-calling through a narrow CapyInn-owned wrapper, static CEO read-only PMS tools, a tool-gated chat loop, and the #125 verification gate.

This slice does not enable PMS writes, public webhooks, generic MCP discovery, shell/file/browser tools, generic HTTP tools, hourly digest delivery, guest-facing agents, or Telegram approval for writes.

## Plain-Language Goal

The CEO should be able to ask CapyInn questions in Telegram and get useful, grounded hotel operations answers.

Examples:

- "Hôm nay phòng nào đang trống?"
- "Có khách nào checkout hôm nay không?"
- "Doanh thu hôm nay thế nào?"
- "Còn ai chưa thanh toán?"
- "Night audit hôm nay có vướng gì không?"

The answer must be grounded in PMS read tools. If the agent cannot answer from an allowed PMS read tool, it says it does not have enough data instead of inventing facts.

## Chosen Approach

Use a CapyInn-native local outbound polling runtime.

The backend owns the Telegram adapter, OpenAI provider wrapper, CEO read-tool registry, and CEO chat runtime. The runtime talks outbound to Telegram using long polling. It does not add a public webhook and does not expose the local PMS gateway to the Internet.

Rejected alternatives:

- A separate MCP-client Telegram bot would reuse gateway tools, but it would mix the CEO runtime policy with generic MCP tooling and add more deployment surface.
- A mocked Telegram/dev-only runtime would be smaller, but it would miss the first user-value milestone: the CEO can actually message Telegram and receive a PMS-grounded answer.

## Relationship To Existing Work

This spec builds on:

- `docs/superpowers/specs/2026-05-05-ceo-agent-safety-foundation-design.md`
- `docs/superpowers/specs/2026-05-05-agentic-ai-roadmap-design.md`
- `docs/superpowers/specs/2026-05-04-agentic-integration-guardrails-design.md`
- `docs/superpowers/specs/2026-05-02-verification-gate-supervised-write-enablement-design.md`

The foundation already introduced:

- `agent` model, registry, retention, runtime, settings, and store boundaries
- CEO cloud-data opt-in
- metadata-only agent sessions and audit events
- non-authoritative agent memory with PMS-truth rejection
- static Phase A CEO read-tool metadata
- fail-closed runtime skeleton

This slice turns that skeleton into the first real read-only runtime.

## Runtime Gates

The CEO Telegram runtime may start only when all gates pass:

- CEO cloud-data opt-in is enabled.
- A numeric Telegram CEO user ID is bound.
- Telegram bot token is present in OS keychain.
- OpenAI API key is present in OS keychain.
- Admin has enabled the `CEO Telegram Chat` runtime toggle.

If any gate fails, Telegram polling must not start. If a gate is revoked while the runtime is active, the runtime must stop before processing more CEO-sensitive messages.

Cloud-data opt-in and runtime enablement are separate controls. Revoking cloud opt-in blocks cloud LLM calls containing CEO-sensitive PMS data. Disabling the runtime stops Telegram chat even if opt-in and secrets remain configured.

## Architecture

Add narrow backend components under the existing `agent` boundary.

Core components:

- `agent::channel::telegram`: outbound-only Telegram long-polling adapter.
- `agent::provider::openai`: OpenAI tool-calling wrapper with scrubbed errors and narrow DTOs.
- `agent::tools::ceo_read`: static typed CEO PMS read tools and executors.
- `agent::runtime::ceo_chat`: per-message policy checks, tool loop, provider orchestration, and Telegram reply delivery.
- `agent::settings`: non-secret runtime config, owner binding, runtime toggle, and gate status.
- `agent::secrets`: OS keychain wrapper for Telegram bot token and OpenAI API key.

The runtime must not pass SQL handles, repositories, transactions, command executors, generic MCP clients, shell tools, file tools, browser tools, or generic HTTP tools to the model.

## Telegram Owner Binding

Owner binding v1 is configured manually by an admin in Settings.

Persisted non-secret config:

- bound Telegram numeric user ID
- optional display label
- runtime enabled flag
- last validation status
- key/token presence flags
- last acknowledged Telegram update offset
- timestamps for configuration updates

Telegram display name and username are metadata only. They must not authorize access.

For each incoming Telegram message:

1. Read the numeric `from.id`.
2. If the ID is missing, unknown, or not equal to the bound CEO ID, send a "not paired" denial and stop. When Telegram provides a numeric sender ID, include that ID in the denial so an admin can bind it manually.
3. Do not read PMS data for unknown or unpaired users.
4. Do not construct prompts for unknown or unpaired users.
5. Do not call OpenAI for unknown or unpaired users.
6. Store at most sanitized audit metadata for denial events.

Public webhook ingress remains out of scope. The adapter uses local outbound long polling to Telegram APIs.

The Telegram adapter should store only non-secret polling state, such as the last acknowledged `update_id` offset. It must not persist raw Telegram message text by default.

## Settings And Secrets

Add an admin-only Settings panel for CEO Telegram Chat.

The panel should show:

- bound Telegram user ID input
- Telegram bot token entry and present/missing status
- OpenAI API key entry and present/missing status
- OpenAI model selector from an admin-controlled allowlist with a conservative default
- runtime enable/disable toggle
- gate status showing which requirement is missing
- concise privacy copy explaining that CEO-sensitive PMS data may be sent to OpenAI only when opt-in and runtime gates pass

Secrets must be stored outside SQLite using an OS keychain wrapper, through the Rust `keyring` crate or an equivalent platform credential store if implementation discovers a platform blocker.

SQLite may store only:

- secret presence booleans
- redacted key labels or fingerprints, if needed
- configuration timestamps
- validation status

SQLite, audit, session, memory, tool output, and Telegram responses must never store or reveal the raw Telegram bot token or OpenAI API key.

Removing a secret clears it from keychain and disables runtime eligibility until reconfigured.

Backend authorization remains required. UI hiding is not enough.

Owner binding, runtime toggle, model selection, and secret presence metadata are low-risk configuration writes. They should use the existing command-boundary pattern where practical and must write sanitized audit metadata without secret values.

## OpenAI Provider Wrapper

The provider wrapper is a CapyInn-owned abstraction, not direct OpenAI calls scattered through the runtime.

Responsibilities:

- load OpenAI API key from the secrets boundary
- build chat/tool-call requests from typed runtime inputs
- expose only static CEO read-tool schemas
- support model tool calls
- parse tool-call responses into typed runtime events
- enforce request timeout and response-size limits
- scrub provider errors before logs, audit, Telegram responses, or model feedback
- redact key-like strings defensively

The provider abstraction should stay narrow enough to add another provider later without widening PMS permissions.

Provider errors must not include:

- OpenAI API key
- Telegram bot token
- raw prompt
- raw model response
- raw PMS extract
- stack trace
- SQL error details with sensitive data

## Static CEO PMS Read Tools

The Phase 1 CEO registry implements exactly these tools:

- `get_hotel_status`
- `list_room_status`
- `list_today_arrivals`
- `list_today_checkouts`
- `list_unpaid_balances`
- `get_revenue_snapshot`
- `get_audit_readiness`
- `summarize_operational_risks`

Every tool must be:

- role-scoped to `CeoSecretary`
- `MutationRisk::ReadOnly`
- `DataSensitivity::CeoSensitive`
- `AgentToolCapability::PmsRead`
- implemented as a typed CapyInn read function

No Phase 1 CEO tool may be:

- a PMS write command
- a low-risk write command
- a shell tool
- a file tool
- a browser tool
- a generic HTTP tool
- a generic MCP discovery tool
- a raw SQL execution tool
- a direct repository or transaction handle

Tool executors should reuse existing read/query functions where possible:

- dashboard and room stats from existing room/revenue read paths
- room availability from existing room availability reads
- arrivals/checkouts from booking read queries
- unpaid balances from booking/invoice/folio read queries
- revenue snapshot from `queries::booking::revenue_queries`
- audit readiness from audit read queries and unresolved operational checks

Where no clean read exists, add a small focused query function rather than embedding ad hoc SQL inside the model loop.

Tool results must be compact structured envelopes, not raw database rows.

## Tool-Gated Chat Loop

Natural chat is allowed only through the static read-tool loop.

Per incoming CEO message:

1. Check runtime gates.
2. Check Telegram owner binding.
3. Create or update metadata-only session state.
4. Build a bounded prompt with hotel context, current local date/time, and static tool specs.
5. Call OpenAI through the provider wrapper.
6. For each model tool call, authorize against the static CEO registry.
7. Execute authorized PMS read tools.
8. Return structured tool envelopes to the model.
9. Stop when the model provides a final answer or the loop reaches its iteration limit.
10. Send the final answer to Telegram.
11. Write sanitized audit metadata.

Loop rules:

- use a small maximum tool-iteration limit
- dedupe identical repeated tool calls within one turn or stop the loop
- reject any tool name not in the CEO registry
- reject any tool with write, shell, file, browser, generic HTTP, or dynamic discovery capability
- answer data-unavailable for unsupported PMS questions
- do not let memory answer PMS truth questions
- do not persist raw prompt, raw response, or raw tool output by default

Structured tool envelopes should include:

- `ok`
- `tool`
- `data` for successful compact results
- `error.code` and scrubbed `error.message` for failures
- optional `metadata` such as date range or record count

## PMS Truth And Agent Memory

PMS SQLite state remains the source of truth.

Agent memory may store non-authoritative preferences, summaries, and conversation hints. It must not store or answer from canonical booking, availability, payment, folio, invoice, ledger, housekeeping, or audit state.

The chat loop must not consult memory to answer PMS facts. If memory is used later for style or preferences, PMS facts still come from read tools on every relevant turn.

## Audit And Retention

Reuse the existing metadata-only retention posture:

- raw prompts: `not_stored`
- raw responses: `not_stored`
- raw tool outputs: `not_stored`
- raw provider errors: `not_stored`
- session metadata: local metadata until operator cleanup
- audit metadata: local metadata until operator cleanup
- memory: local non-authoritative until operator cleanup

Audit events should be enough to explain what happened without becoming a second PMS database.

Allowed audit metadata examples:

- Telegram actor ID
- policy outcome
- tool names called
- tool count
- provider name
- model name
- token/request counts if available without sensitive content
- loop termination reason
- scrubbed error code
- elapsed time

Forbidden audit/session/memory content:

- Telegram bot token
- OpenAI API key
- raw prompt
- raw model response
- raw PMS tool output
- guest document numbers unless a future spec explicitly permits a redacted form
- full booking/payment/folio/invoice/ledger truth snapshots

## Error Handling

Errors fail closed.

Required outcomes:

- unknown or unpaired Telegram user: "not paired" denial with the sender numeric ID when available, no PMS read, no prompt, no OpenAI call
- missing opt-in: runtime does not start and no CEO-sensitive provider request is built
- missing binding: runtime does not start
- missing Telegram token: runtime does not start
- missing OpenAI key: runtime does not start
- provider/network failure: Telegram receives a short unavailable message
- unsupported PMS question: Telegram receives a data-unavailable answer
- max tool iterations reached: stop and answer with partial grounded data or data-unavailable
- repeated identical tool calls: dedupe or stop
- unauthorized tool call: reject and audit scrubbed metadata

Add stable machine-readable error codes or exact equivalents through the existing app error system:

- `AGENT_RUNTIME_NOT_CONFIGURED`
- `AGENT_TELEGRAM_OWNER_NOT_BOUND`
- `AGENT_TELEGRAM_USER_DENIED`
- `AGENT_SECRET_MISSING`
- `AGENT_PROVIDER_REQUEST_FAILED`
- `AGENT_TOOL_LOOP_LIMIT`
- `AGENT_UNSUPPORTED_PMS_QUESTION`

Existing codes such as `AGENT_CHANNEL_UNPAIRED`, `AGENT_CLOUD_DATA_OPT_IN_REQUIRED`, `AGENT_PROVIDER_DISABLED`, and `AGENT_TOOL_NOT_ALLOWED` should be reused where they match.

## Verification Gate

#125 becomes a concrete pre-enable gate for this slice.

Backend tests:

- unknown Telegram ID does not create prompt/provider requests
- unknown Telegram ID does not execute PMS tools
- unpaired Telegram ID does not create prompt/provider requests
- unpaired Telegram ID does not execute PMS tools
- paired CEO can complete an end-to-end mocked Telegram turn with OpenAI tool calls and a Telegram reply
- runtime cannot start unless cloud opt-in, owner binding, runtime toggle, Telegram secret, and OpenAI secret are all present
- registry has exactly the eight CEO read tools
- registry has no write, generic, shell, file, browser, HTTP, or dynamic MCP tools
- every CEO tool has mutation risk and data sensitivity
- tool loop enforces max iterations
- tool loop dedupes repeated identical tool calls
- unsupported PMS question returns data-unavailable
- provider errors are scrubbed
- Telegram errors are scrubbed
- key/token markers never appear in audit, session, memory, tool output, or Telegram responses
- Telegram chat turn cannot mutate PMS tables, checked with DB snapshot/count assertions around chat turns
- agent memory is not consulted for PMS truth and cannot affect tool results

Frontend tests:

- admin can save and revoke Telegram owner binding
- admin can save and clear Telegram token status
- admin can save and clear OpenAI key status
- runtime toggle is gated when requirements are missing
- gate status explains missing requirements
- receptionist cannot see or configure CEO Telegram Chat

Suggested verification commands after implementation:

```bash
cargo test --manifest-path mhm/src-tauri/Cargo.toml agent:: -- --nocapture
npm test -- --run src/pages/settings
npm run verify:quick
```

Add a focused `verify:agent` command if the agent test suite grows large enough to deserve a separate entry point. Full branch completion should include GitNexus `detect_changes` before commit.

## Acceptance Mapping

#120:

- CEO identity is bound to numeric Telegram user ID.
- Display name and username are metadata only.
- Unknown or unpaired users receive no PMS data.
- Unknown or unpaired users do not trigger LLM calls or prompt construction with PMS context.
- Public webhook ingress remains out of scope.

#121:

- OpenAI tool calls go through a narrow CapyInn-owned provider wrapper.
- Provider errors are scrubbed before logs, audit, Telegram responses, or model feedback.
- Bot tokens and API keys never appear in persisted records or responses.
- Provider abstraction does not widen PMS permissions.

#122:

- Static CEO tools cover hotel status, room status, arrivals, checkouts, unpaid balances, revenue snapshot, audit readiness, and operational risks.
- Registry contains no write tools.
- Registry contains no shell, file, browser, generic HTTP, or dynamic MCP discovery tools.
- Tools use PMS read/query services.
- The LLM never receives SQL/database handles.

#123:

- Runtime exposes only Phase 1 read-only CEO tool specs.
- Tool loop has a max iteration limit.
- Identical repeated tool calls in one turn are deduped or stopped.
- Tool results use structured envelopes.
- Unsupported PMS questions return data-unavailable rather than hallucinated facts.

#125:

- Telegram flow cannot mutate PMS tables.
- Registry contains no write/generic/shell/file/browser/HTTP/dynamic MCP tools.
- Unknown and unpaired Telegram users cannot trigger LLM calls.
- Bot token and OpenAI key never appear in logs, memory, audit, tool output, or Telegram responses.
- Agent memory cannot affect PMS query results.

## Out Of Scope

This spec does not:

- add public Telegram webhook ingress
- expose the PMS gateway remotely
- add Telegram approval for writes
- create, modify, cancel, check in, check out, post payment, update room status, mutate folios, generate invoices, or close night audit through Telegram
- add hourly digest scheduling
- add observer-driven alerts
- add guest receptionist tools
- add voice receptionist behavior
- add generic MCP tool discovery
- persist raw chat history or raw PMS extracts
- add encrypted SQLite secret storage

## Implementation Notes

Before editing existing functions, classes, or methods, run GitNexus impact analysis for the target symbol and report direct callers, affected processes, and risk level. If risk is HIGH or CRITICAL, warn before editing.

Run GitNexus `detect_changes` before committing implementation changes.

Keep edits targeted. This slice should add the local CEO Telegram read-only runtime without refactoring unrelated PMS write flows, gateway policy, booking services, or outbox behavior.
