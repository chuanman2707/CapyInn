# Đơn giản hóa màn Khai báo tạm trú — "băng chuyền một chiều"

**Ngày:** 2026-07-27
**Trạng thái:** đã duyệt thiết kế, chờ lập kế hoạch triển khai
**Spec gốc của tính năng:** [2026-07-26-khai-bao-tam-tru-design.md](2026-07-26-khai-bao-tam-tru-design.md) — spec này chỉ sửa lớp trải nghiệm, không đổi luật nghiệp vụ.

## 1. Vấn đề

Người vận hành (chủ khách sạn, người đặt hàng tính năng) dùng thử và không hiểu
cách dùng. Màn hiện tại bắt người dùng học sáu khái niệm trước khi xuất nổi một
file: hồ sơ chờ → gắn phòng → tick chọn từng dòng → chọn tab VN/nước ngoài →
bấm "Kiểm tra" → xuất → màn đối chiếu gõ số. Mỗi bước có lý do kỹ thuật, nhưng
cộng lại thì máy đang bắt người làm việc thay vì ngược lại.

## 2. Bối cảnh sử dụng (chốt qua hỏi đáp)

| Câu hỏi | Trả lời | Hệ quả thiết kế |
|---|---|---|
| Nhịp dùng? | "Lúc nào rảnh thì làm" — không định kỳ | Mở tab lên là màn hình phải tự trả lời "giờ tôi làm gì?"; mọi trạng thái dở dang phải sống qua tắt/mở app |
| Có cần khai một phần? | Thỉnh thoảng cần chừa vài khách lại | Không bỏ được chọn lọc, nhưng đảo mặc định: mặc định khai hết, ngoại lệ mới cần thao tác ("Gác lại") |
| Khách nước ngoài? | Có đều, lẫn cả hai loại | Bỏ tab VN/NN — máy tự chia theo quốc tịch, một cú bấm ra đủ file |
| Bước gõ số đối chiếu? | Giữ (cổng từng báo thành công khi nhận 0 record), nhưng làm dễ hiểu | Bỏ 2 checkbox, viết lại thành hướng dẫn ①②③, giải thích tại chỗ vì sao phải đếm |

## 3. Nguyên tắc bất di bất dịch (kế thừa spec gốc, không đổi)

- Chỉ **SELECT** trên `guests` / `bookings` / `booking_guests` / `rooms`.
- Không lưu ảnh giấy tờ, không lưu raw QR/MRZ payload.
- Không tự đăng nhập/upload lên cổng (captcha + Google Authenticator — quyết định vĩnh viễn).
- Không đụng màn check-in, `watcher.rs`, thư mục `Scans/`.
- Không đổi bộ luật kiểm tra W/E, không đổi bộ ghi file XLSX/XML.
- Cột K/L (phường/xã) vẫn để trống ở v1.

## 4. Thiết kế

Một trang, một dòng chảy từ trên xuống, mỗi thời điểm đúng một nút chính.

### 4.1 Danh sách khách hợp nhất

Khái niệm "hồ sơ chờ chưa gắn phòng" **biến mất**. Thả ảnh giấy tờ vào là
khách vào thẳng danh sách **"Chưa khai báo"**, một thẻ một khách:

```
Chưa khai báo (5)
┌─────────────────────────────────────────────────┐
│ Nguyễn Văn A · 12/03/1990 · CCCD 056188…        │
│ Phòng: [P.201 ▾]  Lý do: [Du lịch ▾]            │
│                              [Gác lại] [Xóa]    │
├─────────────────────────────────────────────────┤
│ 🌐 John SMITH · 05/07/1985 · Hộ chiếu K1234…    │
│ Phòng: [Chưa xác định ▾]  Lý do: [Du lịch ▾]    │
│ ⚠ Thiếu ngày hết hạn visa — bấm để bổ sung      │
│                              [Gác lại] [Xóa]    │
└─────────────────────────────────────────────────┘
Đã gác lại (1) ▸   (thu gọn; mỗi dòng có nút "Đưa lại")
```

- Lưu danh tính xong là **tự tạo link ngay** trong cùng lệnh backend
  (atomic — không còn cửa sổ mồ côi giữa hai lệnh): phòng = chưa xác định
  (`stay_id NULL`), lý do = `STAY_REASON_DEFAULT` ("1" — Du lịch).
- **Phòng và lý do sửa tại chỗ** trên thẻ (lệnh `kbtt_update_link`).
  Danh sách phòng lấy từ `kbtt_list_stays` như hiện tại, cộng lựa chọn
  "Chưa xác định phòng".
- **Gác lại** = đặt `held_at` trên link. Khách xuống mục thu gọn, không vào
  file xuất, vẫn được badge đếm. Sống qua tắt/mở app. "Đưa lại" xóa `held_at`.
- **Xóa** = một lệnh `kbtt_discard(link_id)` xóa link **và** danh tính trong
  một transaction (danh tính chỉ xóa khi không còn link nào khác trỏ tới).
  Giữ luật cũ: link đã nằm trong lô đã chốt đối chiếu thì từ chối xóa.

### 4.2 Một nút xuất, máy tự kiểm tra, lỗi nói tiếng người

- **Bỏ nút "Kiểm tra"**: `kbtt_validate` tự chạy (debounce) mỗi khi danh sách
  đổi. Bộ luật W/E giữ nguyên — chỉ đổi cách hiển thị.
- Lỗi hiện **ngay trên thẻ khách**, câu tiếng Việt kèm cách sửa; mã W/E thu
  nhỏ cuối câu để tra cứu khi cần hỗ trợ. Lỗi chặn = viền đỏ, cảnh báo = viền
  vàng (vàng vẫn xuất được). Cần một bảng ánh xạ mã → câu tiếng Việt + hành
  động sửa trong catalog frontend.
- Bấm vào dòng lỗi mở form sửa thông tin khách — tái dùng `ManualForm.tsx`
  hiện có, focus sẵn vào đúng trường thiếu.
- **Bỏ tab VN/NN**: một nút xuất duy nhất. Frontend chia khách hợp lệ theo
  `nationality_iso3`, gọi `kbtt_export` tối đa hai lần (VN → XLSX,
  NNN → XML). Chỉ có một loại khách thì chỉ ra một file, không hỏi gì thêm.
- Nút luôn nói thật nó sắp làm gì:
  - Sạch: `[ Xuất file cho 5 khách ]`
  - Có khách lỗi chặn: `[ Xuất file cho 4 khách ]` + dòng "1 khách còn lỗi sẽ
    ở lại danh sách — sửa xong xuất bổ sung sau". **Khách lỗi ở lại danh sách
    với viền đỏ, không chặn cả đoàn, không bị bỏ rơi im lặng.**
  - Tất cả đều lỗi hoặc danh sách rỗng: nút mờ, kèm tóm tắt lý do.
- Kết quả xuất là một thẻ liệt kê từng file kèm số khách, nút "Mở thư mục"
  (lệnh `kbtt_open_export_dir` hiện có). Cảnh báo "đừng mở file bằng Excel"
  chuyển vào thẻ này — hiện đúng lúc người dùng sắp cầm file, không treo cố
  định giữa trang nữa.

### 4.3 Đối chiếu ①②③

Mỗi lô đã xuất mà chưa chốt hiện một thẻ:

```
┌ Đối chiếu file khách Việt Nam (3 khách) ─────────────┐
│ ① Mở cổng, upload file này                           │
│ ② Trên màn danh sách của cổng, bấm "Làm mới"         │
│ ③ Đếm số hồ sơ cổng hiển thị, gõ vào đây:            │
│    Cổng hiện [___] hồ sơ   (file này có 3)  [Chốt]   │
│ ❓ Vì sao phải đếm tay? Cổng từng báo "thành công"    │
│    trong khi thực tế nhận 0 khách. Con số tự đếm là  │
│    bằng chứng duy nhất khách đã được khai thật.      │
└──────────────────────────────────────────────────────┘
```

- Bỏ hai checkbox "đã upload / đã làm mới" — thành bước ① ② trong hướng dẫn.
- Gõ đúng → thẻ xanh "N khách đã khai xong". Gõ lệch → lô chuyển `failed`,
  thẻ đỏ: "Cổng nhận thiếu — N khách vẫn tính là chưa khai. Kiểm tra file rồi
  upload lại." (`kbtt_reconcile` giữ nguyên.)
- **Thẻ mọc lại khi mở app** nếu còn lô trạng thái `exported` hoặc `failed`:
  dựng từ `kbtt_list_batches` lúc vào trang, không phụ thuộc state trong
  phiên. Thẻ của lô `failed` giữ nguyên hướng dẫn ①②③ (upload lại chính file
  đó rồi gõ số lần nữa) — khách của lô này KHÔNG quay lại danh sách "Chưa
  khai báo", tránh xuất trùng một khách ra hai file.

### 4.4 Badge và dòng diễn giải

- Badge menu = **số khách chưa khai xong** = số link đang hoạt động chưa nằm
  trong lô `verified` (một biểu thức duy nhất — tự nhiên gồm cả "chưa xuất",
  "gác lại", "chờ đối chiếu" lẫn khách trong lô `failed`). Về 0 thì badge ẩn.
  (`kbtt_undeclared_count` sửa theo công thức này.)
- Dưới tiêu đề trang có dòng diễn giải: ví dụ *"6 khách chưa khai xong:
  3 chưa xuất file · 2 chờ đối chiếu · 1 gác lại"*.
- Lịch sử xuất file thu gọn thành một dòng cuối trang, bấm mới xổ.

## 5. Thay đổi kỹ thuật

### 5.1 Dữ liệu — migration v22 (chỉ bảng của tính năng)

- `ALTER TABLE declaration_link ADD COLUMN held_at TEXT NULL` (additive,
  không cần rebuild bảng).
- **Backfill:** danh tính đang tồn tại mà chưa có link (khái niệm cũ "hồ sơ
  chờ") được tạo link mặc định ngay trong migration — không khách nào bị kẹt
  vô hình khi nâng cấp. (Máy anh đang có 3 danh tính như vậy.)
- Cập nhật các test đang ghim schema version.

### 5.2 Lệnh Tauri

| Lệnh | Số phận |
|---|---|
| `kbtt_save_identity` | Sửa: tự tạo link mặc định trong cùng transaction, trả về link |
| `kbtt_update_link` | **Mới**: sửa phòng / lý do / ghi chú tại chỗ |
| `kbtt_hold` / `kbtt_release` | **Mới**: đặt / xóa `held_at` |
| `kbtt_discard` | **Mới**: xóa link + danh tính (transaction; từ chối nếu lô đã chốt) |
| `kbtt_undeclared_count` | Sửa: công thức badge mới |
| `kbtt_unlinked_identities`, `kbtt_discard_identity`, `kbtt_unlink`, `kbtt_link` | **Gỡ bỏ ở PR 3** — khái niệm hồ sơ chờ không còn, việc tạo link đã nằm trong `kbtt_save_identity`; phần xóa dùng chung repo với `kbtt_discard` |
| Còn lại (`kbtt_extract_from_image`, `kbtt_list_stays`, `kbtt_validate`, `kbtt_export`, `kbtt_list_batches`, `kbtt_reconcile`, `kbtt_open_export_dir`, `kbtt_pending_rows`) | Giữ nguyên |

### 5.3 Giao diện

- `PendingList.tsx` (417 dòng, đang ôm khu chờ + form gắn + bảng tick) tách
  thành `GuestList` + `GuestCard` (+ khu "Đã gác lại" thu gọn).
- `ExportPanel.tsx`: bỏ nút Kiểm tra, nút xuất tự-chia, thẻ kết quả kèm cảnh
  báo Excel.
- `ReconcileChecklist.tsx` → thẻ ①②③, dựng lại từ `kbtt_list_batches` khi
  vào trang.
- `index.tsx`: bố cục bốn khu theo §4; bỏ toggle NNN/VN, bỏ cảnh báo Excel
  cố định; thêm dòng diễn giải badge.
- Catalog: bảng ánh xạ mã W/E → câu tiếng Việt + hành động.

### 5.4 Kiểm thử

Mỗi hành vi mới một test, đặt tên theo hành vi:

- Thả ảnh xong khách có mặt trong danh sách ngay (không qua khu chờ).
- Gác lại sống qua khởi động lại (unmount/remount + đọc từ DB).
- Một cú bấm ra hai file đúng quốc tịch; chỉ một loại khách thì một file.
- Khách lỗi chặn ở lại danh sách, không chui vào file, không chặn khách sạch.
- Thẻ đối chiếu mọc lại khi vào trang còn lô `exported`.
- Badge khớp công thức mới (chưa xuất + gác lại + chờ đối chiếu).
- Migration v22: backfill link cho danh tính mồ côi; link cũ giữ nguyên.
- `kbtt_discard` từ chối khi lô đã chốt; xóa sạch khi chưa.
- Test hợp đồng TS↔Rust hiện có giữ nguyên vai trò gác cổng.

## 6. Trình tự triển khai — 3 PR, app chạy được sau mỗi PR

1. **Nền dữ liệu**: migration v22 + backfill, lệnh mới, sửa
   `kbtt_save_identity` / `kbtt_undeclared_count`. UI cũ vẫn chạy trên lệnh
   cũ (lệnh cũ chỉ gỡ ở PR 3).
2. **Danh sách hợp nhất + xuất một cú**: `GuestList`/`GuestCard`, bỏ nút
   Kiểm tra, bỏ toggle VN/NN, lỗi tiếng người.
3. **Đối chiếu ①②③ + badge + dọn dẹp**: thẻ đối chiếu bền, dòng diễn giải,
   gỡ lệnh và component cũ.

## 7. Ngoài phạm vi

- Tự động upload/đăng nhập cổng (vĩnh viễn không làm).
- Đổi luật kiểm tra W/E hay định dạng file xuất.
- Ánh xạ phường/xã (cột K/L).
- Mọi thay đổi trên bảng dữ liệu của PMS.
