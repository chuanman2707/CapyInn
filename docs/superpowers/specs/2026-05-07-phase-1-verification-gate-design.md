# Phase 1 Verification Gate

Issue: #125 AI-01.09 Phase 1 verification gate for CEO secretary

Parent scope: #76 Agentic AI and outbox integration roadmap

Date: 2026-05-07

## Status

Approved design direction, pending implementation plan.

Spec này là verification closeout thuần cho Phase 1 CEO secretary. Nó không thêm capability mới, không bật PMS write, không thêm Telegram/OpenAI live integration mới, và không thay đổi production behavior chủ động. Implementation chỉ được thêm hoặc sắp xếp tests, probes, verification script, và documentation. Nếu gate mới phát hiện safety bug thật, implementation plan phải thêm fix tối thiểu kèm regression test.

## Plain-Language Goal

Phase 1 CEO secretary đã được gom qua các slice trước:

- CEO agent safety foundation
- Telegram read-only chat
- CEO hourly digest scheduler

Spec này đóng phase bằng một gate rõ ràng: trước khi coi Phase 1 hoàn tất, CapyInn phải chứng minh CEO secretary không phá PMS safety boundary.

Gate phải chứng minh:

- Telegram chat và digest không mutate PMS business tables.
- CEO tool registry không có write, generic, shell, file, browser, HTTP, hoặc dynamic MCP tools.
- Unknown hoặc unpaired Telegram users không trigger LLM/provider call.
- Telegram bot token và OpenAI key không lọt vào logs, memory, audit, tool output, digest run state, hoặc Telegram response.
- Agent memory không ảnh hưởng PMS query result.
- Hourly digest dùng `uses_memory=false` và fixed read-only tools.
- CEO cloud-data opt-in và revocation vẫn chặn đúng CEO-sensitive payload.
- Status/chat/digest smoke flow chạy end-to-end theo read-only path.

## Chosen Approach

Chuẩn hóa `npm run verify:agent` thành required Phase 1 verification gate.

Gate này gom backend unit/integration tests, frontend settings tests, static guardrail tests, và deterministic smoke/probe tests không cần secrets thật. Mục tiêu là một lệnh local, offline-friendly, deterministic, không gọi Telegram/OpenAI thật.

Rejected alternatives:

- Chia gate theo subsystem riêng lẻ sẽ dễ review từng mảng, nhưng yếu hơn cho vai trò đóng phase vì không có một pass/fail command duy nhất.
- Live end-to-end test với Telegram/OpenAI thật sát thực tế hơn, nhưng flaky, tốn secrets thật, có chi phí, và không phù hợp làm required local/CI gate.

## Relationship To Existing Work

Spec này builds on:

- `docs/superpowers/specs/2026-05-05-agentic-ai-roadmap-design.md`
- `docs/superpowers/specs/2026-05-05-ceo-agent-safety-foundation-design.md`
- `docs/superpowers/specs/2026-05-06-telegram-read-only-chat-design.md`
- `docs/superpowers/specs/2026-05-07-ceo-hourly-digest-scheduler-design.md`

Các specs trước đã định nghĩa runtime, Telegram channel, OpenAI provider wrapper, CEO read registry, settings, session/audit/memory boundary, cloud-data opt-in, và hourly digest. Spec D không thay thế các specs đó. Nó biến các safety claims của Phase 1 thành một verification gate có pass/fail rõ ràng.

## Gate Architecture

`npm run verify:agent` phải chạy đủ bốn lớp kiểm chứng.

### Rust Backend Tests

Backend tests cover:

- `agent::` runtime, registry, provider, channel, digest, settings, session/audit/memory boundaries
- `commands::agent_settings::tests::` cho admin settings, secrets presence metadata, opt-in/revocation, digest/chat gate DTOs

Backend tests dùng fake provider, fake Telegram transport, và in-memory SQLite khi có thể. Gate không được phụ thuộc Telegram token hoặc OpenAI key thật.

### Frontend Settings Tests

Settings UI tests cover:

- admin-only CEO Telegram Chat controls
- admin-only CEO Hourly Digest controls
- gate status hiển thị missing requirements
- receptionist hoặc non-admin không thấy hoặc không cấu hình được CEO agent controls

UI hiding không được coi là authorization boundary. Backend command tests vẫn phải cover authorization hoặc fail-closed behavior tương ứng.

### Static Guardrail Tests

`mhm/tests/agentic-guardrails.test.ts` hoặc equivalent static guardrail suite cover invariants khó chứng minh chỉ bằng runtime test:

- `verify:agent` chạy với `CAPYINN_DISABLE_CEO_TELEGRAM=true`
- digest runtime bị bound vào fixed CEO read registry
- Telegram denial tests tồn tại và chặn trước runtime/provider call
- secret redaction tests tồn tại cho Telegram bot URL và OpenAI key-like markers
- chat và digest business-table mutation tests tồn tại
- agent memory được document là non-authoritative trong manifest/OpenAPI/skill docs

Static tests không thay thế runtime tests. Chúng là tripwire để tránh vô tình xóa hoặc bypass guardrails quan trọng.

### Deterministic Smoke/Probe Tests

Gate phải có smoke/probe tests không gọi network thật:

- paired CEO chat hỏi status và nhận reply qua fake provider/fake Telegram
- digest tạo fixed payload, dùng `uses_memory=false`, và deliver qua fake Telegram
- unknown/unpaired Telegram sender không gọi fake provider/runtime
- CEO cloud-data opt-in default false chặn CEO-sensitive provider request
- enabled opt-in cho phép request hợp lệ
- revoked opt-in chặn lại CEO-sensitive provider request

Smoke tests được phép mutate agent metadata tables cần thiết cho session/audit/digest run tracking. Chúng không được mutate PMS business tables.

## Safety Test Matrix

Mỗi claim của #125 phải map tới ít nhất một test/probe trong `verify:agent`.

| Safety claim | Required coverage |
| --- | --- |
| Telegram/agent flow không mutate PMS tables | Snapshot/count hoặc row-hash trước/sau cho chat và digest. Chỉ agent metadata tables được phép thay đổi. |
| Registry không có dangerous tools | Assert exact eight CEO tools, `ReadOnly`, `PmsRead`, `CeoSensitive`, role `CeoSecretary`; reject write/generic/shell/file/browser/HTTP/dynamic MCP capabilities. |
| Unknown/unpaired Telegram users không trigger LLM | Fake runtime/provider call count bằng 0; không prompt construction; không PMS read. |
| Secrets không leak | Secret-like markers không xuất hiện trong logs/errors/audit/session/memory/tool output/digest run state/Telegram response. |
| Memory không ảnh hưởng PMS query | Seed forbidden hoặc misleading memory rồi assert PMS read result vẫn đến từ DB/read tool output. |
| Digest fixed read-only path | `CEO_DIGEST_TOOL_NAMES` match CEO registry; provider không nhận model-selected tools; session `uses_memory=false`. |
| Cloud opt-in/revocation | Default false blocks CEO-sensitive provider request; enabled allows; revoked blocks again. |
| E2E smoke read-only path | Mocked status/chat/digest flow hoàn tất với fake provider/fake Telegram và PMS business tables unchanged. |

## PMS Business Table Mutation Boundary

Mutation checks phải phân biệt PMS business truth với agent metadata.

PMS business tables include booking, room, guest, invoice, folio, payment, ledger, housekeeping, audit, pricing, group, outbox, and other operational truth tables. Chat và digest tests phải prove các bảng này unchanged across mocked turns.

Allowed metadata changes during tests:

- `agent_sessions`
- `agent_audit_events`
- `agent_digest_runs`
- non-secret agent settings/config rows needed by setup
- command/audit metadata rows only when the test explicitly covers settings command behavior

Nếu snapshot test phát hiện PMS business table change, Phase 1 gate fails. Implementation phải investigate trước khi đóng issue #125.

## Registry Boundary

Phase 1 CEO registry is fixed to:

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
- `AgentToolCapability::PmsRead`
- `DataSensitivity::CeoSensitive`

No Phase 1 CEO registry may contain:

- PMS write tools
- low-write or high-write tools
- shell tools
- file tools
- browser tools
- generic HTTP tools
- generic MCP discovery tools
- dynamic tool-loading hooks
- raw SQL execution tools
- command executors, transaction handles, repositories, or database handles exposed to the model

Digest must use the same fixed registry by explicit equality check. It must not maintain a divergent hand-copied tool list without a test that fails on drift.

## Telegram Identity Boundary

Unknown and unpaired Telegram users must be denied before any LLM-related work.

Required behavior:

1. Read numeric Telegram `from.id`.
2. If missing, unknown, or different from bound CEO user ID, send a not-paired denial.
3. Do not construct provider request.
4. Do not construct PMS prompt.
5. Do not execute PMS read tools.
6. Do not call OpenAI wrapper.
7. Store only sanitized denial metadata if audit is written.

Telegram username, display name, and chat title are display metadata only. They must not authorize CEO access.

## Secret Redaction Boundary

Gate must use secret-like markers to prove raw secrets do not escape through common failure paths.

Markers should cover:

- OpenAI-style `sk-...` strings
- Telegram bot token strings
- Telegram API URL form such as `/bot<TOKEN>/sendMessage`
- key/value forms such as `openai_api_key=...`, `telegram_bot_token=...`, `Authorization: Bearer ...`

Forbidden sinks:

- logs
- provider errors
- Telegram channel errors
- audit summaries
- session metadata
- memory values
- tool output envelopes
- digest run error/delivery metadata
- Telegram responses
- model feedback

Tests should prefer asserting absence of the raw marker and presence of a stable redacted token such as `[redacted]` where applicable.

## Memory Boundary

Agent memory is never PMS truth.

Gate must prove memory cannot affect PMS query results by seeding misleading memory and then executing PMS read paths. Expected result must come from SQLite/read tools, not memory.

Memory may store preferences, summaries, or non-authoritative notes only. It must reject or ignore canonical booking, room availability, payment, folio, invoice, ledger, housekeeping, and audit truth.

Digest sessions must always use:

- `uses_memory=false`
- `retention_policy=metadata_only_v1`

Chat may keep metadata-only session/audit records, but it must not use memory to answer PMS fact questions in Phase 1.

## Cloud-Data Opt-In Boundary

CEO cloud-data opt-in is persisted, default false, and revocable.

Gate must cover:

- default false blocks CEO-sensitive provider request construction
- enabled opt-in allows otherwise valid CEO-sensitive request construction
- revocation blocks subsequent CEO-sensitive provider request construction
- digest gate requires opt-in
- chat gate requires opt-in
- opt-in audit metadata is sanitized and does not store secrets or raw PMS payloads

Cloud-data opt-in is not a write approval. It only allows cloud LLM processing for CEO-sensitive read/report data. PMS writes remain out of scope for Phase 1.

## End-To-End Smoke Paths

Gate must include read-only smoke paths for the CEO-facing workflows.

### Status Smoke

Use a fake/in-memory PMS fixture and assert a status-like read path returns structured data without changing PMS business tables.

### Chat Smoke

Use fake Telegram update, paired CEO ID, fake provider tool call/final answer, and fake Telegram transport. Assert:

- provider called only after owner binding and gate checks pass
- allowed PMS read tool executes
- Telegram reply is sent
- audit/session metadata is sanitized
- PMS business tables unchanged

### Digest Smoke

Use fake provider and fake Telegram transport. Assert:

- fixed digest tool list runs
- `uses_memory=false`
- provider receives summarization request with no model-selected tools
- final digest sends to configured delivery chat ID
- delivery metadata persists without raw prompt/response/tool output
- PMS business tables unchanged

## Pass/Fail Criteria

Phase 1 verification passes only when:

- `npm run verify:agent` passes locally without Telegram/OpenAI secrets.
- Gate makes no real network calls to Telegram or OpenAI.
- `CAPYINN_DISABLE_CEO_TELEGRAM=true` is set for verification runs.
- Every #125 claim maps to active tests in the gate.
- No test depends on raw prompt, raw response, or raw tool output persistence.
- Smoke paths complete through fake provider/fake Telegram read-only flows.
- Business-table mutation checks pass for both chat and digest.
- Secret markers are absent from all tested forbidden sinks.

If any safety assertion fails, Phase 1 is not closed. Do not treat partial pass as acceptance for #125.

Suggested commands after implementation:

```bash
npm run verify:agent
npm run verify:quick
```

Branch completion must also run GitNexus `detect_changes` before committing implementation changes.

## Implementation Boundaries

Implementation plan must stay inside verification closeout scope:

- Audit existing coverage and map each test to the matrix.
- Add missing fake-provider/fake-Telegram/in-memory SQLite tests.
- Strengthen `mhm/scripts/verify/agent.mjs` only as needed to include all gate tests.
- Keep static guardrail tests focused on invariants that runtime tests cannot cover cleanly.
- Avoid production behavior changes unless a new test exposes a real safety bug.
- If production fix is required, keep it minimal and add a regression test.

Implementation must not:

- add PMS write capability
- add approval flow
- add public Telegram webhook ingress
- call Telegram/OpenAI live services
- add generic MCP discovery to CEO runtime
- expose shell, file, browser, HTTP, SQL, repository, transaction, or command executor handles to the model
- persist raw prompts, raw responses, raw tool outputs, raw PMS extracts, Telegram bot token, or OpenAI API key

## Expected Artifacts

Potential implementation artifacts:

- `mhm/scripts/verify/agent.mjs`
- `mhm/tests/agentic-guardrails.test.ts`
- Rust tests under `mhm/src-tauri/src/agent/**`
- Rust tests under `mhm/src-tauri/src/commands/agent_settings.rs`
- Settings tests under `mhm/src/pages/settings/CeoAgentSection.test.tsx`

This spec itself is the only required doc artifact for Spec D. Generated notes or scratch reports should not be committed unless requested.

## Acceptance Mapping

#125:

- Telegram flow cannot mutate PMS tables: covered by chat and digest business-table snapshot tests.
- Registry contains no write/generic/shell/file/browser/HTTP/dynamic MCP tools: covered by exact registry tests and static guardrail tripwires.
- Unknown and unpaired Telegram users cannot trigger LLM calls: covered by fake runtime/provider call-count tests before prompt construction.
- Bot token and OpenAI key never appear in logs, memory, audit, tool output, or Telegram responses: covered by secret marker tests across provider/channel/runtime/digest failure paths and metadata sinks.
- Agent memory cannot affect PMS query results: covered by misleading-memory tests and digest `uses_memory=false` assertions.

Additional Phase 1 closeout:

- Hourly digest uses fixed read-only tools and no memory.
- Cloud-data opt-in and revocation block CEO-sensitive payloads.
- Status/chat/digest smoke paths prove end-to-end read-only behavior with fake provider/fake Telegram.

## Out Of Scope

Spec này không:

- add production agent capability
- add PMS write tools
- add supervised write approval
- add guest receptionist behavior
- add voice receptionist behavior
- add public Telegram webhook
- require live Telegram/OpenAI tests
- change provider data retention posture
- persist raw chat history or raw PMS extracts
- store secrets in SQLite

## Implementation Notes

Trước khi edit function, class, hoặc method hiện có trong implementation, phải chạy GitNexus impact analysis cho target symbol và report direct callers, affected processes, risk level. Nếu risk là HIGH hoặc CRITICAL, phải cảnh báo trước khi edit.

Trước khi commit implementation changes, phải chạy GitNexus `detect_changes`.

Spec D is successful when #125 can be closed by a deterministic verification gate rather than by manual inspection or trust in individual slice tests.
