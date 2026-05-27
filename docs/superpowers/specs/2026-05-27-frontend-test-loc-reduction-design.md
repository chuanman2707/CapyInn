# Thiết Kế Giảm LOC Frontend Tests

Ngày: 2026-05-27
Repo: `/Users/binhan/HotelManager`
Branch: `codex/invoke-command-app-error-refactor`

## Mục Tiêu

Giảm LOC thật một cách an toàn trong các frontend test đã chọn. Việc này chỉ xóa lặp và boilerplate, không đổi hành vi test, không làm yếu assertion, và không giấu các literal quan trọng.

Thứ tự xử lý:

1. `mhm/src/pages/settings/CeoAgentSection.test.tsx`
2. `mhm/tests/e2e/08-settings.test.tsx`
3. `mhm/src/App.backupStatus.test.tsx`
4. `mhm/src/pages/settings/useRoomConfig.test.tsx`

Các Rust runtime-inline tests như `mhm/src-tauri/src/outbox.rs` và `mhm/src-tauri/src/command_idempotency.rs` không nằm trong pass frontend này.

## Ràng Buộc

- Không sửa booking tests, trừ khi sau này được yêu cầu rõ.
- Không sửa hoặc stage file dirty không liên quan `mhm/src/stores/useHotelStore.test.ts`.
- Không sửa runtime/business files để giảm LOC trong pass này.
- Chỉ sửa các test file mục tiêu, hoặc test helper rất hẹp nếu thật sự cần và có giảm LOC.
- Ưu tiên xóa boilerplate lặp hơn là đẩy dòng sang helper.
- Không dùng matcher quá rộng làm assertion yếu đi.
- Giữ rõ các literal là hành vi cần test: text người dùng thấy, error/security/PMS-safety copy, command name, idempotency key, request hash, lock key, selector UX.
- Trước khi sửa symbol/helper/function/class/method đã tồn tại, chạy GitNexus impact analysis và báo blast radius. Nếu GitNexus báo HIGH hoặc CRITICAL ngoài test scope thì dừng trước khi sửa.
- Chạy GitNexus `detect_changes` trước mỗi commit.

## Cách Làm Được Chọn

Làm từng file, từng batch nhỏ. Dùng helper local trong chính test file khi helper đó giảm LOC thật và vẫn dễ đọc.

Không ưu tiên helper dùng chung nhiều file, vì dễ làm test phụ thuộc nhau và khó hiểu. Không table-drive mạnh tay nếu tên failure hoặc assertion trở nên mơ hồ.

## Kế Hoạch Từng File

### `CeoAgentSection.test.tsx`

Đây là file xử lý đầu tiên vì có nhiều lặp ở setup, render, thao tác form, chờ control enabled, và assert invoke.

Ứng viên compact:

- Helper nhỏ cho cặp `mockInitialState()` và `render(<CeoAgentSection />)` khi xuất hiện cùng nhau nhiều lần.
- Helper chờ control enabled.
- Helper rất hẹp cho flow save/click/assert nếu vẫn giữ command name, idempotency key, label, và error copy ngay trong test.
- Giữ rõ các label như `Allow CEO cloud-data processing`, `Runtime enabled`, `Telegram delivery chat ID`, và các validation copy của local receptionist.

### `08-settings.test.tsx`

Compact nhẹ và cẩn thận các phần navigation/setup lặp.

Ứng viên compact:

- Helper render Settings rồi mở một section theo tên.
- Helper setup bootstrap completed cho các test Software Update.
- Giữ rõ các selector/copy quan trọng: `Data & Backup`, `Software Update`, `Export CSV`, `Backup`, `Reset`, `MCP Gateway`, và message admin-only.

File này nằm trong `tests/e2e`, nhưng repo hiện không có Playwright config được phát hiện. Theo `vitest.config.ts`, file này đang chạy bằng Vitest, nên verification target sẽ dùng Vitest.

### `App.backupStatus.test.tsx`

Compact các phần render App, đợi app sẵn sàng, và emit backup event.

Ứng viên compact:

- Helper render `App` rồi đợi shell ban đầu sẵn sàng.
- Helper emit `backup-status` bên trong `act`.
- Helper nhỏ để lấy backup alert hoặc status region.

Không gộp các failure mode khác nghĩa nếu message, source label, timer behavior, hoặc queue behavior là điều test đang bảo vệ.

### `useRoomConfig.test.tsx`

Compact setup hook, fixture form room, và các thao tác lặp.

Ứng viên compact:

- Helper render `useRoomConfig()` rồi đợi room types load xong.
- Fixture cho room form payload dùng chung.
- Helper nhỏ cho set form, save room, delete room/type.
- Chỉ table-drive các error case thật sự cùng cấu trúc, với tên case rõ và assertion `formatAppError(...)` vẫn cụ thể.

Giữ nguyên assertion exact cho lỗi integer money.

## Verification

Với mỗi file đã sửa, chạy targeted Vitest trong `mhm`:

- `npm test -- src/pages/settings/CeoAgentSection.test.tsx`
- `npm test -- tests/e2e/08-settings.test.tsx`
- `npm test -- src/App.backupStatus.test.tsx`
- `npm test -- src/pages/settings/useRoomConfig.test.tsx`

Sau mỗi batch:

1. Review diff để kiểm tra không làm yếu assertion hoặc giấu literal quan trọng.
2. Chạy GitNexus `detect_changes` cho thay đổi hiện tại.
3. Commit riêng batch đó.
4. Không stage `mhm/src/stores/useHotelStore.test.ts`.

Cuối pass frontend, chạy GitNexus `detect_changes`, `git status --short --branch`, và báo LOC trước/sau từng file cùng tổng 4 file.

Nếu thực tế và không quá nặng, chạy thêm `npm run build` hoặc `npm run verify:quick` sau targeted tests.

## Báo Cáo Cuối

Báo cáo implementation cần có:

- Commit đã tạo.
- Test đã chạy và kết quả.
- LOC trước/sau:
  - `CeoAgentSection.test.tsx`
  - `08-settings.test.tsx`
  - `App.backupStatus.test.tsx`
  - `useRoomConfig.test.tsx`
- Tổng LOC trước/sau của 4 file.
- Những chỗ cố ý không compact vì sẽ làm test khó đọc hoặc assertion yếu đi.
- `git status --short --branch` cuối.
