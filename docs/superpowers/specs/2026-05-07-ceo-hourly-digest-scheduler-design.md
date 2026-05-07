# CEO Hourly Digest Scheduler

Issues: #124 và phần còn lại của #125 verification gate

Parent scope: #76 Agentic AI and outbox integration roadmap

Ngày: 2026-05-07

## Trạng thái

Design direction đã được duyệt, đang chờ implementation plan.

Spec này thêm workflow digest hàng giờ riêng cho Phase 1 CEO secretary. Workflow này dùng lại nền hiện có của CEO Telegram chat, OpenAI provider wrapper, static read-only CEO tool registry, metadata-only session/audit storage, và verification command. Scheduling, retry, và delivery status được tách riêng để chat MVP không phình thành scheduler.

Spec này không thêm PMS write, Telegram approval flow, public webhook, generic MCP discovery, observer-driven alert, guest-facing agent, hoặc raw chat history persistence.

## Mục tiêu dễ hiểu

CEO nhận một digest qua Telegram mỗi giờ, 24/7, khi tính năng digest được bật và các gate CEO Telegram hiện có đều sẵn sàng.

Digest tóm tắt trạng thái khách sạn hiện tại:

- occupancy và room status
- arrivals và checkouts
- unpaid balances
- revenue snapshot
- audit readiness
- operational risks cần CEO chú ý

Nếu CapyInn bị tắt hoặc offline vài giờ, khi mở lại app chỉ gửi tối đa một digest hiện tại nếu lần gửi thành công cuối đã quá 1 giờ. Không backfill từng giờ đã miss.

## Hướng chọn

Dùng workflow riêng `agent::digest`.

Digest có toggle bật/tắt riêng, persisted run state riêng, và gate riêng. Gate này dùng chung các dependency an toàn của CEO Telegram, nhưng không phụ thuộc vào chat runtime toggle:

- CEO cloud-data opt-in
- bound numeric Telegram CEO user ID
- persisted Telegram delivery chat ID
- Telegram bot token presence
- OpenAI API key presence
- OpenAI model selection

Các hướng không chọn:

- Nhét digest vào chat loop sẽ nhỏ hơn lúc đầu, nhưng làm interactive reply bị dính với scheduled delivery, retry, và status tracking.
- Dùng OS cron hoặc script ngoài sẽ làm scheduling truth nằm ngoài desktop app, khó giữ offline-first behavior và Settings status rõ ràng.

## Kiến trúc

Thêm backend boundary riêng cho digest dưới `agent`.

Core modules:

- `agent::digest::config`: digest toggle, setting keys, gate evaluation, và Settings DTOs.
- `agent::digest::store`: durable digest run và delivery status persistence.
- `agent::digest::runtime`: fixed-tool digest payload builder, OpenAI summarization, Telegram delivery, và audit/session metadata.
- `agent::digest::scheduler`: startup catch-up decision, hourly due-run creation, claim/retry loop, và shutdown khi gate bị revoke.

`AgentSupervisor` hiện có sẽ quản lý cả hai workflow:

- CEO Telegram chat polling chỉ chạy khi chat gate sẵn sàng.
- CEO hourly digest chỉ chạy khi digest gate sẵn sàng.
- Tắt chat không tự động tắt digest, trừ khi một shared dependency gate bị thiếu.
- Tắt digest không làm dừng interactive chat.

Implementation phải tách rõ `CeoTelegramConfig::evaluate_gate(...)` cho interactive chat khỏi digest gate mới, ví dụ `CeoDigestGateStatus` hoặc `evaluate_digest_gate(...)`. Digest gate không được dùng `ceo_telegram_runtime_enabled`; nó phải dùng setting riêng như `ceo_hourly_digest_enabled`.

Supervisor reconcile cũng phải tách hai lifecycle:

- chat task start/stop theo chat gate
- digest task start/stop theo digest gate
- supervisor object vẫn có thể quản lý cả hai, nhưng readiness của workflow này không được vô tình stop workflow kia

Verification phải cover case `CEO Telegram Chat` off nhưng `CEO Hourly Digest` on và digest gate đủ dependency thì digest scheduler vẫn chạy.

Digest runtime không được expose SQL handle, repository, transaction, command executor, shell tool, file tool, browser tool, generic HTTP tool, generic MCP discovery, hoặc PMS write tool cho model.

## Settings và gates

Thêm toggle admin-only `CEO Hourly Digest` trong CEO Agent Settings hiện có.

Gate requirements:

- CEO cloud-data opt-in đang bật.
- CEO Telegram owner binding tồn tại.
- Telegram delivery chat ID tồn tại.
- Telegram bot token present.
- OpenAI API key present.
- OpenAI model đã cấu hình.
- Toggle CEO Hourly Digest đang bật.

Digest toggle là low-risk configuration write. Khi thực tế triển khai, nó nên đi qua command-boundary pattern hiện có nếu phù hợp và phải ghi sanitized audit metadata.

Receptionist và non-admin user không được cấu hình digest. Ẩn UI không đủ; backend authorization vẫn bắt buộc.

### Telegram delivery target

Scheduled digest không có incoming `message.chat.id`, nên phải có delivery target persisted rõ ràng.

Add non-secret setting:

- `ceo_telegram_delivery_chat_id`

Nguồn giá trị:

- Khi paired CEO gửi message qua interactive chat, backend lưu `message.chat.id` vào `ceo_telegram_delivery_chat_id` bằng low-risk command-boundary path hoặc một internal idempotent system command.
- Admin có thể xem trạng thái present/missing trong Settings. Nếu cần nhập tay, input này vẫn phải là admin-only low-risk config write.
- CEO phải từng start bot hoặc gửi message cho bot trước khi Telegram cho phép bot gửi private message.

Digest delivery luôn dùng `ceo_telegram_delivery_chat_id`, không dùng raw Telegram username/display name để authorize hoặc deliver. Nếu chat ID missing, digest gate không ready và Settings phải nói rõ CEO cần gửi một message tới bot hoặc admin cấu hình chat ID.

## Data flow của digest

Khi scheduler start:

1. Đọc digest gate.
2. Nếu gate chưa ready, không tạo và không deliver run mới.
3. Đọc lần digest delivered thành công gần nhất.
4. Nếu lần delivered thành công gần nhất đã quá 1 giờ, tạo một immediate due run cho trạng thái hiện tại.
5. Không tạo missed run cho từng giờ app offline.

Trong runtime bình thường:

1. Tạo một due run cho mỗi hourly window.
2. Claim một pending hoặc retry-ready run để overlapping tick không gửi trùng.
3. Tạo `agent_sessions` row metadata-only với `uses_memory=false`.
4. Chạy fixed CEO read-only tool list:
   - `get_hotel_status`
   - `list_room_status`
   - `list_today_arrivals`
   - `list_today_checkouts`
   - `list_unpaid_balances`
   - `get_revenue_snapshot`
   - `get_audit_readiness`
   - `summarize_operational_risks`
5. Biểu diễn tool data unavailable rõ ràng trong digest payload.
6. Gửi compact structured payload sang OpenAI qua provider wrapper hiện có, với provider-side storage disabled.
7. Deliver final Vietnamese digest text qua Telegram transport hiện có tới bound CEO chat.
8. Persist delivery status, last-run metadata, và sanitized audit metadata.

OpenAI chỉ đóng vai trò summarizer trong workflow này. Model không được chọn arbitrary tools để generate digest.

## Persistence

Thêm migration cho digest state.

Table đề xuất: `agent_digest_runs`

Required columns:

- `id TEXT PRIMARY KEY`
- `role TEXT NOT NULL`
- `channel TEXT NOT NULL`
- `channel_actor_id TEXT`
- `delivery_chat_id TEXT`
- `due_at TEXT NOT NULL`
- `status TEXT NOT NULL`
- `attempt_count INTEGER NOT NULL DEFAULT 0`
- `max_attempts INTEGER NOT NULL`
- `next_retry_at TEXT`
- `claimed_at TEXT`
- `claim_token TEXT`
- `delivered_at TEXT`
- `last_error_code TEXT`
- `last_error_summary_json TEXT NOT NULL DEFAULT '{}'`
- `delivery_summary_json TEXT NOT NULL DEFAULT '{}'`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Allowed statuses:

- `pending`
- `in_progress`
- `retry_waiting`
- `delivered`
- `failed`
- `skipped_gate_not_ready`

Indexes:

- due time và status
- next retry time và status
- delivered time
- channel actor và due time
- delivery chat ID và due time

Table này chỉ lưu delivery metadata. Nó không được lưu raw prompt, raw OpenAI response, raw PMS tool output, Telegram bot token, OpenAI API key, hoặc raw Telegram message.

## Retry và delivery status

Mỗi due digest có bounded attempts. Default nên nhỏ, ví dụ 3 attempts total.

Retry behavior:

- Retryable Telegram, OpenAI, và network failure chuyển run sang `retry_waiting`.
- `next_retry_at` dùng short bounded backoff.
- Sau max attempts, run chuyển thành `failed`.
- Failed run không block hourly run tiếp theo.
- Scheduler không bao giờ retry một failed digest vô hạn.

Unavailable PMS data không tự động là delivery failure. Nếu một tool fail hoặc trả unavailable data, digest vẫn gửi khi có thể và đánh dấu section đó unavailable. Nếu toàn bộ dữ liệu digest required đều unavailable, digest gửi một thông báo ngắn kiểu data-unavailable thay vì bịa dữ liệu.

## Audit, retention, và memory

Digest sessions dùng:

- `role=ceo_secretary`
- `channel=telegram`
- `uses_memory=false`
- `retention_policy=metadata_only_v1`

Audit và run metadata được phép lưu:

- digest run id
- due time
- attempt count
- delivery status
- policy outcome
- tool names
- unavailable tool names
- provider name
- model name
- scrubbed error code
- elapsed time
- response length

Persisted content bị cấm:

- Telegram bot token
- OpenAI API key
- raw prompt
- raw model response
- raw PMS tool output
- raw Telegram message text
- guest document numbers, trừ khi có spec tương lai cho phép dạng redacted rõ ràng
- full booking, payment, folio, invoice, ledger, housekeeping, hoặc audit truth snapshots

Agent memory không được dùng để trả lời PMS facts trong digest. Mỗi digest run phải đọc PMS state mới qua PMS read tools.

## Error handling

Digest errors fail closed.

Required behavior:

- Missing digest toggle: không tạo hoặc deliver due run mới.
- Missing cloud opt-in: không build provider request.
- Missing owner binding: không attempt Telegram delivery.
- Missing Telegram token: không attempt delivery.
- Missing OpenAI key: không build provider request.
- Gate bị revoke khi đang chạy: scheduler dừng trước khi claim run tiếp theo.
- Provider/network failure: bounded retry, sau đó `failed`.
- Telegram delivery failure: bounded retry, sau đó `failed`.
- Unsupported hoặc unavailable PMS data: đánh dấu unavailable; không hallucinate facts.

Error summaries phải được scrub trước khi log, audit, persist, trả về Settings, hoặc gửi qua Telegram.

Log scrubbing là một phần của #125 gate, không chỉ là best effort. Provider và Telegram channel failures phải được test/probe bằng secret-like markers để chứng minh OpenAI key và Telegram bot token không xuất hiện trong logs, kể cả Telegram URL dạng `/bot<TOKEN>/...`.

## Verification gate

`npm run verify:agent` trở thành focused completion gate cho slice này.

Backend coverage phải chứng minh:

- digest gate yêu cầu digest toggle và toàn bộ shared CEO readiness gates
- startup tạo tối đa một immediate digest nếu last delivery đã quá 1 giờ
- missed hours không bị backfill
- scheduler tạo một hourly due run, không duplicate
- claim logic chặn overlapping sends
- bounded retry đi tới `failed` và không loop vô hạn
- successful delivery persist last-run và delivery metadata
- digest sessions luôn dùng `uses_memory=false`
- digest chỉ dùng đúng fixed CEO read-only tool set
- unavailable tool data được biểu diễn rõ ràng
- `chat off + digest on + digest gate ready` vẫn cho phép digest scheduler chạy
- missing Telegram delivery chat ID làm digest gate not ready
- digest flow không mutate PMS business tables
- chat và digest registries không chứa write, generic, shell, file, browser, HTTP, hoặc dynamic MCP tools
- unknown và unpaired Telegram users không trigger provider calls
- bot token và OpenAI key không xuất hiện trong logs, audit, session, memory, tool output, digest run state, hoặc Telegram responses
- log-capture test hoặc probe cover Telegram/OpenAI failure path với secret-like markers
- agent memory không ảnh hưởng PMS query results

Frontend coverage phải chứng minh:

- admin bật/tắt được CEO Hourly Digest
- Settings hiển thị gate requirement còn thiếu
- receptionist không thấy hoặc cấu hình được CEO Hourly Digest

Suggested verification commands:

```bash
npm run verify:agent
npm run verify:quick
```

Hoàn tất branch implementation phải chạy GitNexus `detect_changes` trước khi commit implementation changes.

## Acceptance mapping

#124:

- Digest dùng fixed allowed read-only tools và `uses_memory=false`.
- Digest cover occupancy/room status, arrivals/checkouts, balances, revenue snapshot, audit readiness, và operational risks.
- Last-run và delivery status được persist.
- Delivery failures dùng bounded retry và không thể gây infinite retry spam.
- Digest đánh dấu unavailable data rõ ràng.

#125:

- Telegram chat và digest flows không mutate PMS tables.
- Registry không chứa write, generic, shell, file, browser, HTTP, hoặc dynamic MCP tools.
- Unknown và unpaired Telegram users không trigger LLM calls.
- Bot token và OpenAI key không xuất hiện trong logs, memory, audit, tool output, digest run state, hoặc Telegram responses.
- Agent memory không ảnh hưởng PMS query results.

## Ngoài scope

Spec này không:

- thêm public Telegram webhook ingress
- expose PMS gateway ra remote
- thêm Telegram approval for writes
- thêm PMS write tools hoặc draft action tools
- implement observer-driven alerts
- backfill từng giờ app offline
- thêm guest receptionist behavior
- thêm voice receptionist behavior
- persist raw chat history hoặc raw PMS extracts
- store secrets trong SQLite

## Ghi chú triển khai

Trước khi edit function, class, hoặc method hiện có, phải chạy GitNexus impact analysis cho target symbol và report direct callers, affected processes, risk level. Nếu risk là HIGH hoặc CRITICAL, phải cảnh báo trước khi edit.

Chạy GitNexus `detect_changes` trước khi commit implementation changes.

Giữ digest workflow tách khỏi interactive chat loop. Shared dependencies nên đi qua provider, channel, tool, config, session, và audit boundaries rõ ràng thay vì coupling trực tiếp giữa chat internals và scheduler internals.
