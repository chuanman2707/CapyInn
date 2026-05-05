# Agentic AI Roadmap

Date: 2026-05-05

## Status

Approved roadmap direction, pending implementation planning.

This is a roadmap spec. It does not implement code, change gateway behavior, add Telegram, add OpenAI calls, or expose any new PMS write capability.

## Plain-Language Goal

CapyInn should grow an agentic AI layer without weakening the PMS safety boundary.

The first useful product milestone is a Telegram-based CEO secretary. It can chat in Vietnamese, answer hotel operations questions, run read-only operational reports, produce hourly digests, and summarize audit readiness and revenue snapshots for the owner.

Later milestones can add observer-driven alerts, durable approval-gated writes, an online AI receptionist for guest messaging channels, and a voice receptionist channel at the physical front desk.

The core rule is unchanged:

- PMS SQLite state remains the source of truth.
- Agent memory is never PMS truth.
- Phase 1 agents may autonomously read and report.
- PMS writes require a later durable approval and command execution path.

## Relationship To Existing Specs

This roadmap builds on, but does not replace, these existing designs:

- `docs/superpowers/specs/2026-05-04-agentic-integration-guardrails-design.md`
- `docs/superpowers/specs/2026-05-04-mcp-observer-stream-design.md`
- `docs/superpowers/specs/2026-05-02-verification-gate-supervised-write-enablement-design.md`

Those specs already define important foundations:

- gateway loopback exposure by default
- PMS truth versus observer facts versus agent memory
- high-risk write policy behavior
- outbox-backed observer stream
- supervised write verification gates

This roadmap adds the product and capability sequence for agentic AI on top of those foundations.

## External Reference Boundary

Agent platform reference research is internal project context, not public roadmap material.

Public roadmap and implementation issues should describe only CapyInn-owned design decisions:

- use CapyInn-owned provider, channel, and tool boundaries
- keep tool registries static and role-scoped
- keep Phase 1 read/report-only
- keep agent memory non-authoritative
- scrub credentials and sensitive tool output
- require durable CapyInn approval ledgers for future writes

Do not vendor, copy, or transplant source code from external agent platforms into CapyInn unless a future task performs a specific license, dependency, security, and architecture review for that exact code.

## Roadmap Principles

1. **YOLO is scoped to the toolset.**

   The CEO secretary may operate autonomously inside its read-only/reporting tools. It must not receive PMS write tools in Phase 1.

2. **Gateway permission is not business approval.**

   An authenticated MCP client, Telegram channel, or future paired device is allowed to connect. That does not mean it can mutate PMS state.

3. **Every PMS write remains command-boundary-first.**

   Future writes must use durable idempotency, aggregate locks, SQL transactions, audit, and outbox events.

4. **LLM output is advisory unless backed by trusted tool output.**

   The agent may summarize and explain. It may not invent room state, revenue, availability, booking status, payment status, or audit status.

5. **Memory is convenience, not truth.**

   Agent memory may store preferences, summaries, and conversational context. It must not store canonical booking, room, folio, invoice, ledger, payment, housekeeping, or audit state.

6. **Guest-facing agents are separate from CEO agents.**

   Future receptionist agents must use separate roles, tools, sessions, and memory scopes.

## Permission Model

The roadmap separates four different permission layers.

### Channel Permission

Channel permission answers: is this Telegram sender allowed to talk to this agent?

Phase 1 rules:

- Telegram is the first channel.
- The CEO identity is bound to numeric Telegram user ID.
- Telegram display name and username are display metadata only.
- Unknown or unpaired Telegram users receive no PMS data.
- Public webhook ingress is out of scope for Phase 1; use a local outbound connector or polling model.

### Agent Role Permission

Agent role permission answers: what class of agent is this?

Planned roles:

- `CeoSecretary`
- `GuestReceptionist`
- `VoiceReceptionistChannel`
- future integration-specific agents

Phase 1 only implements the CEO secretary role.

### Tool Risk Permission

Tool risk permission answers: what tools can this role see?

Risk classes:

- `ReadOnly`: safe to auto-run in Phase 1.
- `LowWrite`: future, still policy-controlled.
- `HighWrite`: future draft-only before approval.

`ReadOnly` only means "does not mutate PMS state." It does not automatically mean "safe for every channel or role." Read tools still need data-sensitivity classification.

Data sensitivity classes:

- `PublicHotelInfo`: safe for public guest answers.
- `GuestScoped`: safe only after guest verification for that guest's booking or quote.
- `StaffOperational`: safe for authenticated staff workflows.
- `CeoSensitive`: safe only for the CEO or explicitly authorized owner/admin identities.

Phase 1 `CeoSecretary` receives only static read-only/reporting tools, and those tools may include `CeoSensitive` data by explicit CEO opt-in. It does not receive shell, file, browser, generic HTTP, generic MCP discovery, or direct PMS write tools.

Future guest receptionist tools must be both read-only and guest-safe. They must not inherit CEO read tools simply because those tools are non-mutating.

### Business Write Approval

Business write approval answers: did a human approve this exact mutation?

This does not exist in Phase 1.

Future approval-gated writes require:

- durable pending action row
- canonical payload hash
- approval expiry
- approving human identity
- channel ID and message ID
- replay-safe callback handling
- execution through existing command idempotency, aggregate locks, transaction, audit, and outbox

## Phase 1: CEO Telegram Read-Only Secretary

### Goal

Give the CEO a useful Telegram AI secretary for hotel operations without any PMS mutation path.

### Capabilities

The CEO secretary can:

- chat in Vietnamese through Telegram
- answer tool-gated natural language questions
- show current room status
- report arrivals and checkouts
- report unpaid balances
- report operational revenue snapshots
- run audit readiness checks
- summarize operational risks
- send an hourly digest every hour, 24/7

The CEO secretary cannot:

- create a booking
- modify a booking
- cancel a booking
- check in a guest
- check out a guest
- update room status
- record payment
- mutate folio or ledger records
- close or post night audit
- run arbitrary MCP tools
- run shell, file, browser, or HTTP tools

### Data And Privacy Posture

Phase 1 uses OpenAI first.

The CEO explicitly allows cloud LLM processing of detailed CEO-level PMS data, including guest, booking, room, folio, balance, and revenue details needed to answer the CEO's question.

This opt-in does not allow careless persistence:

- bot tokens must not appear in logs, memory, audit, tool output, or model feedback
- OpenAI API keys must not appear in logs, memory, audit, tool output, or Telegram responses
- raw sensitive tool outputs should not be stored in long-term memory
- audit should store metadata and sanitized summaries, not become a second raw PII database
- product settings must make the cloud data processing posture explicit before enabling the agent
- the opt-in must be persisted and revocable
- disabling the opt-in must stop cloud LLM calls that include CEO-sensitive PMS data
- prompts, responses, tool outputs, and session history must have an explicit retention policy
- data sent to OpenAI should be limited to fields needed for the requested answer or digest
- provider configuration must document the selected provider's data-use and retention posture
- sanitized audit/session records must be enough to explain what happened without storing full raw PMS extracts by default

### Architecture

Phase 1 flow:

```text
CEO Telegram
  -> Telegram adapter
  -> owner identity check
  -> CEO secretary runtime
  -> static read-only PMS tool registry
  -> PMS query/services
  -> SQLite PMS truth
  -> OpenAI reasoning/summarization
  -> Telegram response
```

Core components:

- `AiProvider`: CapyInn-owned OpenAI wrapper for chat and tool calls.
- `AiChannel`: CapyInn-owned channel abstraction, initially Telegram only.
- `AiTool`: CapyInn-owned typed read-only tool boundary.
- `CeoSecretaryRuntime`: message loop, tool authorization, tool execution, model call, response delivery.
- `CeoReadToolRegistry`: static Phase 1 tool list.
- `AgentSessionStore`: durable session/message metadata.
- `AgentMemoryStore`: optional non-authoritative preferences and summaries.
- `AgentAuditStore`: sanitized tool-call and delivery metadata.
- `AgentDigestScheduler`: hourly digest job with fixed allowed tools and `uses_memory=false`.

### Tool-Gated Natural Chat

Natural chat is allowed, but it must be grounded in tool output.

Rules:

- The runtime exposes only read-only tool specs to OpenAI.
- The runtime uses a small maximum tool-iteration limit.
- Identical repeated tool calls in the same turn are deduplicated or stopped.
- Tool results are returned as explicit structured envelopes.
- If no suitable tool exists for a PMS data question, the assistant says it does not have enough data.
- The model never receives a SQL handle, repository handle, transaction, or raw database access.
- The model never writes SQL.

### Phase 1 Read Tools

The Phase 1 tool catalog should include:

- `get_hotel_status`
- `list_room_status`
- `list_today_arrivals`
- `list_today_checkouts`
- `list_unpaid_balances`
- `get_revenue_snapshot`
- `get_audit_readiness`
- `summarize_operational_risks`

`get_audit_readiness` is a report/check tool. It does not close the day, post ledger records, mutate room state, or perform night-audit writes.

`get_revenue_snapshot` is an operational snapshot for the current day/month and unpaid/deposit/balance state. Deeper CEO analytics such as ADR, RevPAR, historical trend comparisons, channel mix, and anomaly analytics are future work.

### Hourly Digest

The hourly digest runs every hour, 24/7.

Digest requirements:

- use only Phase 1 read-only tools
- not depend on agent memory for PMS facts
- record last-run status and delivery metadata
- avoid infinite retry spam on delivery failure
- make clear when data is unavailable

The digest should cover:

- current occupancy and room status
- arrivals and checkouts
- unpaid balances
- revenue snapshot
- audit readiness
- operational risks needing CEO attention

## Phase 2: Observer Alerts And Digest Hardening

### Goal

Add near-real-time operational awareness without adding write capability.

### Capabilities

Phase 2 uses CapyInn's outbox-backed observer stream to trigger alerts and stronger digests.

Examples:

- new booking observed
- checkout completed
- unpaid checkout risk
- dirty room assigned soon
- room readiness mismatch
- payment or folio event requiring attention

### Rules

- Phase 2 remains read-only.
- Observer events are committed facts, not detail data APIs.
- Alerts must refresh detail through PMS read tools when needed.
- Alerts must dedupe by observer event ID or deterministic risk key.
- Alerts must have rate limits to avoid Telegram spam.
- Restart must not duplicate already acknowledged alerts.
- Observer cursor state must be persisted.
- Alert state must distinguish delivered, failed, suppressed, and explicitly acknowledged alerts.
- Acknowledgement means an alert state row has been persisted, not merely that a Telegram send was attempted.
- Cursor-expired recovery must rebuild the operational snapshot from PMS read tools before resuming alerts.

## Phase 3: Approval-Gated CEO Writes

### Goal

Allow the CEO secretary to help prepare exact PMS actions while preserving human approval and the command boundary.

Phase 3 cannot start until the supervised-write verification gate from `docs/superpowers/specs/2026-05-02-verification-gate-supervised-write-enablement-design.md` has passed for the relevant command families.

### Capabilities

The agent may draft actions such as:

- mark room clean
- mark room maintenance
- cancel booking
- modify booking
- create booking
- record payment
- close night audit

The agent does not execute these actions directly.

### Required Write Flow

```text
CEO request
  -> agent drafts normalized action
  -> policy classifies risk
  -> pending action persisted with canonical payload hash
  -> CEO approves or denies exact action
  -> approval callback verifies owner, expiry, and hash
  -> command executes through existing command boundary
  -> idempotency, locks, transaction, audit, and outbox commit atomically
  -> CEO receives result
```

### Rules

- Direct write tools are not exposed to the LLM.
- The LLM can only create draft actions.
- Approval applies to one exact normalized payload.
- Approval expires.
- Denied or expired actions cannot be reused.
- Duplicate approval callbacks replay idempotently.
- Business state is revalidated inside the transaction.
- Existing `command_idempotency`, `aggregate_locks`, command audit, and `outbox_events` remain the write center of gravity.
- MCP supervised mode must continue returning `APPROVAL_REQUIRED` for high-risk direct write attempts unless the durable approval path handles the exact action.
- Full autonomous mode remains disabled for high-risk PMS writes until a separate launch decision.
- Tool schemas must not accept self-declared approval fields such as `approved: true`, approval tokens, idempotency claim tokens, or raw command claim tokens from the LLM.

## Phase 4: Online AI Receptionist

### Goal

Add guest-facing AI support on messaging platforms without exposing CEO tools or unsafe PMS writes.

Target platforms:

- Zalo
- Facebook Messenger
- WhatsApp

### Capability Stages

Stage 1:

- answer hotel policy and service questions
- check room price and availability through guest-safe read/quote tools
- create a lead or draft booking request
- escalate unsupported or sensitive requests to staff

Stage 2:

- create real bookings only after additional safety gates exist
- recheck availability before final booking
- verify guest identity or contact channel as required
- enforce deposit/payment policy
- prevent overbooking
- record guest consent and source channel
- use approval or strict business rules for high-risk actions

### Rules

- Guest receptionist uses a separate role, session scope, memory scope, and tool registry.
- Guest receptionist cannot access CEO tools or CEO memory.
- Guest-facing answers must not leak another guest's booking, folio, identity, payment, or audit data.
- Price and availability answers must come from PMS tools, not memory.
- A lead or draft is not a confirmed room hold unless the PMS creates a real booking through the approved business path.

## Phase 5: Voice Receptionist Channel

### Goal

Extend the online receptionist into a voice channel for physical guest support at the front desk.

### Initial Model

Treat voice as a new channel for the guest receptionist, not as a separate agent.

```text
guest speech
  -> speech-to-text
  -> guest receptionist policy and tools
  -> response text
  -> text-to-speech
```

### Rules

- Voice transcripts are conversation context, not PMS truth.
- Voice uses guest-safe tools by default.
- Identity-sensitive actions still require verification.
- Staff handoff must be available for ambiguous, angry, payment, overbooking, or identity-sensitive cases.

If front-desk kiosk behavior later requires different permissions, device controls, staff override, or local-only policies, voice can become a separate agent role in a later roadmap revision.

## Acceptance Gates

### Phase 1 Gates

- Unknown Telegram users receive no PMS data.
- Unpaired Telegram users cannot trigger LLM calls.
- Unpaired Telegram users cannot cause prompt construction with PMS context.
- Paired CEO can receive status, room, checkout, balance, revenue, and audit readiness answers.
- The Phase 1 registry contains no write tools.
- The Phase 1 registry contains no shell, file, browser, generic HTTP, or generic MCP discovery tools.
- Every Phase 1 tool has both a mutation-risk class and a data-sensitivity class.
- CEO-sensitive tools are available only to the paired CEO role.
- Cloud LLM use for CEO-sensitive PMS data requires persisted explicit opt-in.
- Revoking the cloud-data opt-in prevents cloud LLM calls containing CEO-sensitive PMS data.
- Prompt, response, session, and tool-output retention is explicit.
- Data sent to OpenAI is limited to fields needed for the requested answer or digest.
- Provider data-use and retention posture is documented in settings or operator docs.
- Sanitized audit/session records do not store full raw PMS extracts by default.
- Natural chat is tool-gated; unsupported PMS questions receive a data-unavailable answer.
- Hourly digest uses read-only tools and `uses_memory=false`.
- Bot token and OpenAI key never appear in logs, memory, audit, tool output, or Telegram responses.
- Telegram flow cannot mutate PMS tables.
- Agent memory cannot affect PMS query results.

### Phase 2 Gates

- Observer events dedupe after reconnect.
- Restart does not duplicate already acknowledged alerts.
- Observer cursor state persists across restart.
- Alert state persists delivered, failed, suppressed, and acknowledged states.
- Cursor-expired recovery rebuilds the operational snapshot from PMS read tools.
- Synthesized risk alerts use deterministic dedupe keys.
- Alerts are traceable to an observer event or deterministic read query.
- Alerts do not forward raw outbox payloads.
- Alerts respect rate limits.
- Phase 2 still has no PMS write path.

### Phase 3 Gates

- Approval is tied to exact canonical payload hash.
- Approval with modified args is rejected.
- Expired approval is rejected.
- Denied approval cannot execute.
- Duplicate callback replays idempotently.
- Non-owner callback is rejected.
- Command execution uses durable idempotency.
- Command execution acquires aggregate locks.
- Business state revalidates inside the SQL transaction.
- Mutation, audit, command ledger, and outbox commit atomically.
- Direct write tools are not visible to the LLM.
- The supervised-write verification gate has passed for each command family exposed through draft actions.
- Direct high-risk MCP write attempts still return policy outcomes such as `APPROVAL_REQUIRED` or `WRITE_TOOL_DISABLED`.
- Full autonomous high-risk writes remain disabled.
- Tool schemas reject self-declared approval fields.

### Phase 4 Gates

- Guest role cannot access CEO tools.
- Guest role cannot access CEO memory.
- Guest cannot see another guest's data without verification.
- Quotes use PMS availability and pricing tools.
- Lead/draft creation does not silently reserve inventory.
- Booking creation, if added, rechecks availability at write time.

### Phase 5 Gates

- Voice transcripts do not become PMS truth.
- Voice uses guest-safe tools by default.
- Identity-sensitive requests require verification.
- Staff handoff is available for unsupported or high-risk conversations.

## Out Of Scope For This Roadmap Spec

This spec does not:

- implement Telegram
- implement OpenAI calls
- add an agent runtime
- add DB migrations
- change MCP gateway routes
- change PMS command behavior
- add approval UI
- enable autonomous PMS writes
- implement guest messaging integrations
- implement voice transcription or text-to-speech
- vendor or copy external agent-platform source code

## Strategic Recommendation

Start with Phase 1.

The fastest safe path is to implement a small CapyInn-native CEO secretary with narrow provider/channel/tool interfaces, static read-only tools, tool-gated chat, Telegram numeric identity, digest scheduling, credential scrubbing, and strict memory separation.

Only after Phase 1 is stable should the roadmap move to observer alerts, and only after durable approval exists should the CEO secretary be allowed to prepare write actions.
