# SPEC — CapyInn / Module "Khai báo tạm trú"

Ngày: 2026-07-26
Trạng thái: đã duyệt, sẵn sàng lập kế hoạch
Thay thế: spec v1 (vá schema `guests`) và spec v2 (bản nháp do người dùng soạn)

**Mục tiêu:** từ ảnh giấy tờ của khách → xuất file khai báo đúng định dạng cho
`https://tbltkbtt.bocongan.gov.vn/`, để người vận hành chỉ cần login + upload.

**Stack:** Rust + Tauri (CapyInn hiện tại), cross-platform macOS + Windows.

**Nguyên tắc bao trùm:** module này **không sửa một dòng nào** trong bảng
`guests` / `bookings` / `booking_guests` / `rooms` đang chạy. Chỉ thêm bảng mới,
và đọc bảng cũ ở chế độ chỉ-đọc. PMS đang vận hành thật — không migrate nó vì
một tính năng phụ.

---

## 0. Quan hệ với spec v2

Spec v2 do người dùng soạn là nền của tài liệu này. Bản này giữ nguyên toàn bộ
sự thật đã kiểm chứng của v2 và sửa những chỗ v2 mô tả sai codebase, sau khi
khảo sát mã nguồn thật.

| Chỗ v2 nói | Thực tế | Xử lý |
|---|---|---|
| Đọc bảng `reservations` | Không tồn tại. Có `bookings` + `booking_guests` + `rooms` | `stay_id` = `bookings.id`, kiểu TEXT |
| `id INTEGER PRIMARY KEY` | Toàn app dùng TEXT uuid | Đổi sang TEXT uuid |
| MRZ OCR bằng tesseract | Repo đã có `ocr-rs` (PP-OCRv5) | Tách trait `MrzOcr`, đo rồi mới chốt |
| 2 file JSON config "kèm theo" | Không tồn tại | Sinh từ chính template chính thức |
| `E15` phường không thuộc tỉnh | v1 để trống cột K/L nên không có dữ liệu để kiểm | Bỏ `E15` khỏi v1 |
| §12.5 xóa `declaration_identity` | Xóa sẽ phá lịch sử lô | Che (redact) thay vì xóa |
| §9 ưu tiên `umya-spreadsheet` | Rủi ro round-trip, nhưng người dùng đã chốt | Giữ `umya-spreadsheet`, mở rộng gate |

Lý do đổi hướng từ v1 sang v2 vẫn nguyên giá trị: tên trong CapyInn là tên gõ
tay để tiện gọi khách ("Andrei", "Công"), không phải tên trên giấy tờ. Cơ quan
công an cần cái thứ hai. Ảnh giấy tờ là nguồn duy nhất đúng.

---

## 1. Sự thật đã kiểm chứng

Không được thiết kế trái với mục này.

### 1.1 Cổng Bộ Công an — F (test upload thật, tài khoản HKD Nhà trọ Bình An, 26/07/2026)

| # | Sự thật | Bằng chứng |
|---|---|---|
| F1 | Hai định dạng khác nhau hoàn toàn. NNN = **XML**. Khách VN = **XLSX**. | 2 file "Danh sách mẫu" từ 2 tab khác nhau |
| F2 | XLSX: server **bỏ qua row 4 theo vị trí**, đọc từ **row 5**. Bất kể row 4 chứa gì. | Upload 1: data ở row 4 → "thành công", 0 record. Upload 2: giữ `[EXAMPLE]` ở row 4, data ở row 5 → vào |
| F3 | **Server báo "import thành công" khi import 0 record.** Không có thông báo lỗi. | Upload 1 |
| F4 | Ghi file bằng thư viện (openpyxl) rồi re-save không phá file. Server vẫn nhận. | File row 5 do openpyxl ghi → import OK |
| F5 | Enum ghi **nguyên chuỗi `mã - nhãn`**, không phải chỉ mã. | `1 - Thẻ CCCD` → hiện "Thẻ CCCD" |
| F6 | Ngày lưu dạng **text** `dd/MM/yyyy`, cell format `@`. | `18/10/1974`, `26/07/2026` hiện đúng |
| F7 | Số giấy tờ giữ được số 0 đầu khi lưu dạng text. | `079174011721` đủ 12 số |
| F8 | Cột `I, K, L, M, P` **thật sự optional**. | Upload không có số phòng, không có phường/xã → vào |
| F9 | XML có `ngay_tra_phong` khác `ngay_di_du_kien` → khai bù được khách đã checkout. | Record mẫu: dự kiến 28/12, trả phòng 27/12 |

**Hệ quả của F3 — quan trọng nhất trong spec:** cổng không cho feedback. Một lô
có thể thất bại im lặng. Module **bắt buộc** có vòng đối chiếu (§9). Không có nó
thì đây là bẫy pháp lý, không phải tiện ích.

### 1.2 Trích xuất từ ảnh — G (test trên ảnh thật)

| # | Sự thật | Bằng chứng |
|---|---|---|
| G1 | **QR thẻ CCCD: 7 trường, phân cách `\|`.** Dữ liệu số, không qua OCR → không sai. | Decode thành công |
| G2 | **OpenCV `QRCodeDetector` fail cả 7 mức tiền xử lý. zxing đọc được grayscale thô.** | Test song song |
| G3 | **Tiền xử lý làm HỎNG dữ liệu.** MRZ: grayscale thô → 3/3 checksum. Upscale×3+Otsu → tụt còn 2/3. | Test 9 tổ hợp |
| G4 | **MRZ dòng 2 OCR chính xác, 3/3 checksum pass.** Có thể tin tự động. | tesseract psm=6, grayscale |
| G5 | **MRZ dòng 1 (TÊN) OCR sai 2 ký tự, và KHÔNG có checksum nào bảo vệ.** ICAO chỉ đặt check digit trên dòng 2 → sai im lặng. | OCR ra `ZOLOCHEVSKAIA<X<VERONIKAK`, thật là `ZOLOCHEVSKAIA<<VERONIKA` |
| G6 | MRZ chỉ có **2 chữ số năm**. Luật suy thế kỷ cho ngày sinh và ngày hết hạn **phải khác nhau**. | Dùng chung 1 hàm → `351209` thành 09/12/**1935** |
| G7 | Địa chỉ trong QR CCCD **đã lạc hậu** so với danh mục hiện hành. | QR trả `Phan Rang-Tháp Chàm, Ninh Thuận` — tỉnh Ninh Thuận không còn, đã nhập Khánh Hòa `511`; phường `Mỹ Đông` không còn trong 3.323 phường |
| G8 | QR **không phân biệt** `1 - Thẻ CCCD` với `8 - Thẻ Căn Cước`. | Payload 2 loại giống nhau |
| G9 | QR và MRZ trả giới tính **khác format**: QR = `Nữ`, MRZ = `F`. | |
| G10 | MRZ **không chứa** thời hạn được phép tạm trú. Chỉ có ngày hết hạn hộ chiếu (ví dụ 2035) — hoàn toàn khác. | |

### 1.3 Khảo sát codebase và artifact — H (26/07/2026)

| # | Sự thật | Bằng chứng |
|---|---|---|
| H1 | **Không có bảng `reservations`.** Lượt lưu trú nằm ở `bookings` (id TEXT uuid) + `booking_guests` + `rooms`. | `.schema` trên `capyinn.db` |
| H2 | **`guests.guest_type` không tin được.** 9/9 khách là `domestic`, gồm cả "Hoseok Lee", "Andrei", "Geonhoo Park". `doc_number` trống toàn bộ. | Truy vấn DB thật |
| H3 | `bookings.check_in_at` là **timestamp có offset** (`2026-07-25T17:45:27+07:00`); `expected_checkout` là **date trần** (`2026-07-26`). | Truy vấn DB thật |
| H4 | `rooms.name` có tiền tố — `"Phòng 5B"`, trong khi cổng nhận `5A` / `102A9`. | Truy vấn DB thật + XML mẫu |
| H5 | **Template chính thức chứa toàn bộ danh mục.** 4 sheet: `DS_KHACH_VIET_NAM_LUU_TRU`, `TINH_THANH` (34), `PHUONG_XA` (3.323), `DANH_MUC` (9 loại giấy tờ / 3 nơi cư trú / 20 lý do / 2 giới tính / 205 quốc tịch). 40 defined name, gồm `PX_<mã tỉnh>` nhóm phường theo tỉnh. | Giải nén `tblt_vn_import.xlsx` |
| H6 | **Form web VN yêu cầu đúng 9 field**, khớp chính xác 9 cột có `(*)` trong template: `B C D E F H N O Q`. Tỉnh/phường/địa chỉ chi tiết/số điện thoại/số phòng/nơi cư trú **không** có dấu `*`. | Ảnh chụp form + header template |
| H7 | Repo **đã có** `ocr-rs` 2.1 (PP-OCRv5, MNN), `src/ocr.rs`, `watcher.rs` theo dõi thư mục `Scans/`, và `src/bin/test_ocr.rs`. `Backend::Metal` hardcode → **macOS-only**. | `Cargo.toml`, source |
| H8 | Schema version hiện tại **19**, migration đăng ký tuần tự trong `db.rs`. | `db.rs`, `schema_version` |
| H9 | XML mẫu của cổng có **2 record**, cả hai đều mang tiền tố `[TEST]` trong `ho_ten`. | `Danh_Sach_Mau.xml` |

### 1.4 Ranh giới tin cậy — thiết kế phải phản ánh đúng bảng này

| Dữ liệu | Cơ chế | Xử lý |
|---|---|---|
| Toàn bộ 7 trường thẻ CCCD | QR, dữ liệu số | **Tin tự động** |
| HC: số, quốc tịch, ngày sinh, giới tính, hết hạn HC | MRZ dòng 2 + 5 checksum | **Tin tự động sau verify** |
| **HC: HỌ TÊN** | MRZ dòng 1, không checksum | **Người xác nhận, LUÔN LUÔN** |
| Loại giấy tờ (CCCD vs Căn Cước) | heuristic ngày cấp | Máy đoán, người sửa được |
| Thời hạn tạm trú (NNN) | không tồn tại ở đâu | **Nhập tay** |
| Lý do lưu trú | không tồn tại ở đâu | Chọn từ enum, mặc định Du lịch |
| Số phòng, ngày đến/đi | `bookings` + `rooms` của CapyInn | Đọc từ DB |
| Phân loại NNN/VN | `nationality_iso3` từ ảnh | **Không dùng `guests.guest_type`** (H2) |

---

## 2. Phạm vi

**LÀM:**

1. Tab `Khai báo tạm trú` ở sidebar, nhóm MANAGEMENT
2. Kéo-thả ảnh giấy tờ vào cửa sổ → trích danh tính (QR / MRZ / nhập tay)
3. Ghép danh tính với lượt lưu trú đang có trong CapyInn
4. Validator chặn cứng
5. Xuất **XML** (NNN) và **XLSX** (khách VN)
6. Vòng đối chiếu thủ công

**KHÔNG LÀM ở v1** — ghi rõ để chống scope creep:

- **Không tự động login/upload lên cổng.** Cổng có captcha + Google
  Authenticator mỗi lần đăng nhập. Người vận hành upload tay. **Đây là quyết
  định vĩnh viễn, không phải hạn chế tạm.**
- **Không dùng PhotoKit / truy cập thư viện Photos.app.** Cần entitlement +
  sidecar Swift. Kéo-thả từ Photos ra app khác là thao tác macOS hỗ trợ sẵn,
  đạt 90% giá trị với 10% công.
- **Không đụng `watcher.rs` / thư mục `Scans/`.** Đường máy scan hiện tại nuôi
  `CheckinSheet`, để nguyên. Module này chỉ nhận kéo-thả và chọn file.
- **Không map phường/xã hành chính.** F8 và H6 cho phép bỏ trống, G7 chứng minh
  không map được. Ghi nguyên chuỗi địa chỉ vào `ĐỊA CHỈ CHI TIẾT`. Cột K/L để
  trống. Không có `E15`.
- **Không lưu ảnh, không lưu payload thô.** §12.
- Không xử lý chi nhánh / tài khoản phụ.
- Không dùng LLM cho luồng chính.
- Không sửa gì trên màn check-in hiện tại.

---

## 3. Kiến trúc

```
  Ảnh giấy tờ (kéo-thả hoặc file picker)
        │
        ▼
  IdentityExtractor  ── trait, 3 implementation:
        │                 QrCccdExtractor   (rxing)
        │                 MrzExtractor      (trait MrzOcr + checksum scoring)
        │                 ManualExtractor   (form nhập tay)
        ▼
  Normalizer         ── chuẩn hóa ngày, giới tính, enum, suy thế kỷ
        │
        ▼
  declaration_identity   (bảng mới — KHÔNG lưu ảnh, KHÔNG lưu payload thô)
        │
        │◄──── bookings / booking_guests / rooms / guests  (CHỈ ĐỌC)
        ▼
  declaration_link       (nối danh tính ↔ stay + lý do lưu trú)
        │
        ▼
  Validator          ── chặn cứng, không cho xuất nếu còn lỗi blocking
        │
        ▼
  Writer             ── XmlWriter (NNN) | XlsxWriter (VN, ghi thêm vào template)
        │
        ▼
  declaration_batch  ── ghi lô, mở thư mục chứa file
        │
        ▼
  Reconciler         ── người vận hành nhập số record thấy trên cổng
```

### 3.1 Bố cục file

```
mhm/src-tauri/src/declaration/
  mod.rs           — re-export, không chứa logic
  catalog.rs       — nạp kbtt_catalog.json, tra cứu enum
  extractor/
    mod.rs         — trait IdentityExtractor, thứ tự thử
    qr_cccd.rs     — rxing
    mrz.rs         — trait MrzOcr, checksum TD3, scoring
    manual.rs
  normalizer.rs    — ngày, giới tính, suy thế kỷ, ISO3
  validator.rs     — E01..E14, W01..W06
  writer/
    mod.rs
    xml.rs         — NNN
    xlsx.rs        — VN, umya-spreadsheet
  repo.rs          — 4 bảng mới + đọc bảng cũ
mhm/src-tauri/src/db/declaration.rs   — migrate_v20_declaration_tables
mhm/src-tauri/src/commands/declaration.rs   — lớp Tauri mỏng
mhm/src-tauri/src/bin/kbtt_probe.rs         — CLI harness
mhm/src-tauri/resources/kbtt_catalog.json   — danh mục sinh từ template
mhm/src-tauri/resources/tblt_vn_import.xlsx — template chính thức
scripts/gen_kbtt_catalog.py                 — sinh catalog từ template
mhm/src/pages/Declaration/                  — UI 4 khối
```

### 3.2 Ràng buộc kiến trúc

- `catalog`, `extractor`, `normalizer`, `validator`, `writer` **không import
  `tauri`, không import `sqlx`**. Nhận struct vào, trả struct ra. Nhờ vậy
  validator chạy được trên màn danh sách mà không cần ghi file, và toàn bộ test
  là unit test thuần.
- Chỉ `repo.rs` và `commands/` chạm DB và app handle.
- Đường đọc dữ liệu cũ đi qua đúng một hàm: `repo::load_stays_for_declaration()`.
- **Không một câu `INSERT` / `UPDATE` / `DELETE` / `ALTER` nào chạm
  `guests` / `bookings` / `booking_guests` / `rooms`.** Kiểm bằng test đọc source.

---

## 4. Danh mục — `kbtt_catalog.json`

**Không hardcode, không viết tay.** Sinh từ chính template chính thức (H5).

`scripts/gen_kbtt_catalog.py` đọc `tblt_vn_import.xlsx`, xuất:

```json
{
  "_source_file": "tblt_vn_import.xlsx",
  "_source_sha256": "…",
  "_source_date": "2026-07-26",
  "loai_giay_to":  [{"code": "1", "display": "1 - Thẻ CCCD"}, "…9 mục"],
  "noi_cu_tru":    ["…3 mục"],
  "ly_do_cu_tru":  ["…20 mục"],
  "gioi_tinh":     [{"code": "F", "display": "F - Nữ"},
                    {"code": "M", "display": "M - Nam"}],
  "quoc_tich":     [{"code": "AFG", "display": "AFG - Afghanistan"}, "…205 mục"],
  "tinh_thanh":    [{"code": "511", "display": "511 - Khánh Hòa"}, "…34 mục"],
  "phuong_xa":     [{"code": "101900127",
                     "display": "101900127 - Phường Việt Hưng",
                     "tinh": "101"}, "…3323 mục"]
}
```

Quy tắc:

- `code` tách từ `display` bằng cách cắt ở `" - "` **đầu tiên**.
- `display` là thứ ghi vào file (F5). **Không bao giờ tự ghép lại chuỗi từ
  `code` + tên** — sai một dấu cách là cổng không nhận mà không báo.
- Quan hệ phường↔tỉnh lấy từ cột `MATT` của sheet `PHUONG_XA`.
- Script **assert chéo**: số dòng mỗi named range `PX_<mã>` phải khớp số phường
  có `MATT = <mã>`. Lệch → script fail, không xuất file.
- Script assert số lượng: 9 / 3 / 20 / 2 / 205 / 34 / 3323. Lệch → fail kèm
  thông báo "template đã đổi, xem lại §13.6".

`catalog.rs` nạp JSON một lần lúc khởi động, lỗi sớm và rõ nếu thiếu/hỏng.

Khi cổng phát hành template mới: thay file, chạy lại script, `git diff` chỉ ra
đúng cái gì đổi. Đó là câu trả lời cho §13.6 — schema drift trở thành một commit
nhìn thấy được, thay vì một khai báo sai im lặng.

Cổng có tab **"Quản lý danh mục"**, có thể là nguồn cập nhật về sau. Chưa khảo sát.

### 4.1 Tìm file resource

`tauri.conf.json` hiện **chưa** có khóa `bundle.resources`. Phải thêm:

```json
"bundle": { "resources": ["resources/*"] }
```

Nhưng `bin/kbtt_probe.rs` và unit test chạy ngoài Tauri, không có app handle để
gọi `BaseDirectory::Resource`. Dùng lại đúng cách `ocr.rs::find_models_dir()` đã
làm — hàm `find_kbtt_resource(name)` thử lần lượt:

1. thư mục resource của Tauri (khi có app handle)
2. `app_identity::runtime_root().join("resources")`
3. `cwd/resources`, `cwd/../resources`
4. `env!("CARGO_MANIFEST_DIR")/resources` — cho dev và test

Trả lỗi rõ ràng liệt kê mọi đường đã thử nếu không tìm thấy, giống `find_models_dir`.

### 4.2 Khóa settings

Bốn khóa, đặt trong bảng `settings` đang có:

| Khóa | Mặc định | Dùng ở |
|---|---|---|
| `declaration.export_dir` | `~/CapyInn/exports/khai-bao-tam-tru` | §9.2 |
| `declaration.cslt_name` | `CSLT` | tên file §9.2 |
| `declaration.xml_lead_example` | `false` | §9.1, §14.1 |
| `declaration.redact_after_days` | `90` | §12.5 |

---

## 5. Data model

Bốn bảng mới. **Không `ALTER` bảng nào đang có.** Migration
`migrate_v20_declaration_tables` trong `db/declaration.rs`, đăng ký vào `db.rs`
sau `migrate_v19_agent_digest_runs` (H8). Thuần `CREATE TABLE`.

Khóa chính dùng **TEXT uuid** cho khớp quy ước phần còn lại của app.

```sql
CREATE TABLE IF NOT EXISTS declaration_identity (
  id                      TEXT PRIMARY KEY,
  source                  TEXT NOT NULL,   -- 'qr_cccd' | 'mrz_td3' | 'manual'
  extract_confidence      TEXT NOT NULL,   -- 'verified' | 'needs_review'
  full_name               TEXT NOT NULL,
  dob                     TEXT NOT NULL,   -- ISO yyyy-MM-dd
  gender                  TEXT NOT NULL,   -- 'M' | 'F'
  nationality_iso3        TEXT NOT NULL,
  -- khách Việt Nam
  doc_type_code           TEXT,            -- '1'..'9'
  doc_type_source         TEXT,            -- 'heuristic' | 'human'
  doc_type_name           TEXT,            -- bắt buộc khi doc_type_code = '9'
  doc_no                  TEXT,            -- TEXT, giữ số 0 đầu
  phone                   TEXT,
  residence_status        TEXT,            -- '1'|'2'|'3'
  address_detail          TEXT,            -- chuỗi thô từ QR, KHÔNG parse
  -- khách nước ngoài
  passport_no             TEXT,
  passport_expiry         TEXT,            -- từ MRZ. KHÁC visa_valid_until.
  visa_valid_until        TEXT,            -- NHẬP TAY -> thoi_han_tam_tru
  -- kiểm soát
  name_confirmed_by_human INTEGER NOT NULL DEFAULT 0,   -- G5
  single_token_name_ok    INTEGER NOT NULL DEFAULT 0,   -- gỡ chặn E02
  redacted_at             TEXT,
  created_at              TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS declaration_link (
  id               TEXT PRIMARY KEY,
  identity_id      TEXT NOT NULL REFERENCES declaration_identity(id),
  stay_id          TEXT NOT NULL,   -- = bookings.id. FK logic, KHÔNG ràng buộc cứng
  stay_reason      TEXT NOT NULL,   -- '1'..'20'
  stay_reason_note TEXT,            -- bắt buộc khi stay_reason = '20'
  actual_check_out TEXT,            -- khai bù, chỉ NNN (F9)
  created_at       TEXT NOT NULL,
  UNIQUE(identity_id, stay_id)
);

CREATE TABLE IF NOT EXISTS declaration_batch (
  id             TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,    -- 'NNN' | 'VN'
  date_from      TEXT,
  date_to        TEXT,
  file_path      TEXT NOT NULL,
  row_count      INTEGER NOT NULL,
  status         TEXT NOT NULL,    -- 'exported'|'uploaded'|'verified'|'failed'
  verified_count INTEGER,
  verified_at    TEXT,
  note           TEXT,
  created_at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS declaration_entry (
  batch_id  TEXT NOT NULL REFERENCES declaration_batch(id),
  link_id   TEXT NOT NULL REFERENCES declaration_link(id),
  row_index INTEGER NOT NULL,
  PRIMARY KEY (batch_id, link_id)
);
```

### 5.1 Vì sao có `doc_type_source`

`W06` cảnh báo khi `doc_type_code` do heuristic ngày cấp suy ra mà chưa ai xác
nhận (G8). Không có cột này thì `W06` không cài được. `QrCccdExtractor` đặt
`'heuristic'`; người sửa trong UI đổi thành `'human'`.

### 5.2 Vì sao không ràng buộc FK cứng tới `bookings`

Ràng buộc cứng sẽ khiến việc xóa hoặc dọn một booking trong PMS thất bại vì một
module phụ. Nguyên tắc bao trùm là module này không được làm PMS gãy.

### 5.3 Truy vấn "khách chưa khai báo"

Không cần cột mới ở bảng cũ.

```sql
SELECT b.id,
  (SELECT COUNT(*) FROM booking_guests bg
     WHERE bg.booking_id = b.id) AS guest_count,
  (SELECT COUNT(*) FROM declaration_link dl
     JOIN declaration_entry de  ON de.link_id = dl.id
     JOIN declaration_batch dbt ON dbt.id     = de.batch_id
    WHERE dl.stay_id = b.id AND dbt.status = 'verified') AS declared_count
FROM bookings b
WHERE b.status = 'active';
```

Badge sidebar = tổng `MAX(0, guest_count − declared_count)` trên các booking có
`check_in_at` trong 48h.

Lô `failed` không có `declaration_entry` nào ở trạng thái `verified`, nên khách
tự động giữ nguyên trạng thái chưa khai — không cần code hoàn tác.

### 5.4 Phân loại NNN / VN

`declaration_identity.nationality_iso3 = 'VNM'` → XLSX. Khác → XML.
**Không dùng `guests.guest_type`** (H2).

---

## 6. Trích xuất và chuẩn hóa

### 6.1 Trait

```rust
pub trait IdentityExtractor {
    fn try_extract(&self, image: &DynamicImage) -> Option<ExtractResult>;
}

pub struct ExtractResult {
    pub source: Source,            // QrCccd | MrzTd3 | Manual
    pub confidence: Confidence,    // Verified | NeedsReview
    pub fields: ExtractedFields,   // struct có kiểu, KHÔNG HashMap
    pub review_hints: Vec<Field>,
    pub crop_for_review: Option<DynamicImage>,  // chỉ trong RAM (§12.4)
}
```

`fields` là struct có kiểu chứ không phải `HashMap<Field, String>`: quên một
field thì compiler bắt, không phải đợi lúc chạy.

Thứ tự thử: `QrCccd` → `MrzTd3` → `Manual`. Một ảnh có thể chứa cả hai (thẻ CCCD
có QR, hộ chiếu có MRZ) — QR thắng vì là dữ liệu số, không qua OCR.

### 6.2 QrCccdExtractor

Thư viện **`rxing`** (bản Rust của zxing). **Không dùng OpenCV** (G2).

```
1. Đọc ảnh → grayscale
2. rxing đọc ảnh THÔ trước (G2, G3: không tiền xử lý)
3. Nếu fail, thử lần lượt: upscale×2 → upscale×4 → Otsu → upscale×3+Otsu
   (dừng ngay khi decode được)
4. Split payload theo '|' → phải đúng 7 phần; khác 7 => fail
```

Không cố cứu payload lệch. Payload lệch nghĩa là đang đoán, mà đoán ở đây là
khai sai.

| # | Trường | Ví dụ | Đi vào |
|---|---|---|---|
| 0 | Số CCCD | `058195006173` | `doc_no` |
| 1 | Số CMND cũ | `264445725` | bỏ |
| 2 | Họ và tên | `Phan Thị Mỹ Hà` | `full_name`, **giữ nguyên dấu** |
| 3 | Ngày sinh | `28071995` (`ddMMyyyy`) | `dob` |
| 4 | Giới tính | `Nữ` | `gender` → `F` |
| 5 | Nơi thường trú | `KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận` | `address_detail`, **thô, không parse** (G7) |
| 6 | Ngày cấp | `27062021` | chỉ để suy loại giấy tờ, rồi bỏ |

Suy `doc_type_code` (G8 — QR không cho biết):

- ngày cấp `< 01/07/2024` → `1 - Thẻ CCCD`
- ngày cấp `>= 01/07/2024` → `8 - Thẻ Căn Cước`
- `doc_type_source = 'heuristic'`. **Phải cho người sửa.** Không khóa.

`confidence = Verified`. `nationality_iso3 = "VNM"` (CCCD chỉ cấp cho công dân VN).

### 6.3 MrzExtractor

**Nguyên tắc: checksum là hàm chấm điểm, không phải bước kiểm cuối.**

Tầng OCR nằm sau trait riêng vì engine chưa chốt:

```rust
pub trait MrzOcr {
    fn recognize_lines(&self, img: &DynamicImage) -> Vec<String>;
}
```

Hai implementation:

- `OcrRsMrz` — dùng `ocr-rs` sẵn có (H7), **0 dependency mới**, mặc định.
- `TesseractMrz` — sau cargo feature `mrz-tesseract`, mặc định **tắt**.

`bin/kbtt_probe.rs` chạy cả hai trên ảnh hộ chiếu thật và in bảng điểm checksum.
Ai đạt gate ≥3/5 và rẻ hơn thì thắng. **Chỉ bật dep tesseract nếu `ocr-rs` trượt.**
Quyết định bằng số đo, không bằng phỏng đoán.

Thuật toán:

```
1. Tìm vùng MRZ: OCR cả ảnh, giữ dòng có >=30 ký tự thuộc [A-Z0-9<]
2. Với mỗi biến thể tiền xử lý, theo đúng thứ tự G3:
      a. grayscale THÔ        <-- ưu tiên, cho kết quả tốt nhất
      b. upscale×2
      c. upscale×3 + Otsu     <-- thường TỆ HƠN, để cuối
3. Mỗi kết quả: pad/cắt về 44 ký tự/dòng, tính 5 checksum, ĐẾM số pass
4. Chọn biến thể điểm cao nhất
5. 5/5   -> confidence = Verified
   3-4/5 -> dòng 2 tin được, tên NeedsReview
   <3/5  -> fail, chuyển sang ManualExtractor
6. LUÔN LUÔN đặt name_confirmed_by_human = 0 (G5)
```

#### Hàm checksum

```
weights = [7, 3, 1] lặp lại
giá trị ký tự:  '<' = 0 | '0'-'9' = số | 'A'-'Z' = 10..35
check = (Σ giá_trị[i] × weights[i mod 3]) mod 10
```

#### TD3 (hộ chiếu, 2 dòng × 44)

Dòng 1:

| Vị trí | Nội dung |
|---|---|
| 1 | Loại giấy tờ (`P`) |
| 2 | Phân loại phụ / `<` |
| 3–5 | Nước cấp (ISO3) |
| 6–44 | `HỌ<<TÊN` + đệm `<` |

Dòng 2:

| Vị trí | Nội dung | Checksum |
|---|---|---|
| 1–9 | Số hộ chiếu | vị trí 10 |
| 11–13 | Quốc tịch ISO3 | – |
| 14–19 | Ngày sinh `yyMMdd` | vị trí 20 |
| 21 | Giới tính `M`/`F`/`<` | – |
| 22–27 | Hết hạn hộ chiếu `yyMMdd` | vị trí 28 |
| 29–42 | Số cá nhân | vị trí 43 |
| – | Composite trên `1–10, 14–20, 22–43` | vị trí 44 |

Ví dụ đã verify 5/5:

```
P<RUSZOLOCHEVSKAIA<<VERONIKA<<<<<<<<<<<<<<<<
7777856719RUS9003082F3512090<<<<<<<<<<<<<<00
→ ho_ten "ZOLOCHEVSKAIA VERONIKA" · HC 777785671 · RUS
  · sinh 08/03/1990 · F · HC hết hạn 09/12/2035
```

`ho_ten` = dòng 1 vị trí 6–44, `<<` → khoảng trắng, `<` → khoảng trắng, trim.
**Giữ nguyên HOA, không dấu** — đúng dạng cổng cần cho NNN.

TD1 (3×30) / TD2 (2×36): v1 không làm, fail sang nhập tay. TODO v2.

### 6.4 ManualExtractor

Form nhập tay đầy đủ, dùng khi: ảnh không có QR/MRZ (CMND cũ, giấy khai sinh,
GPLX), QR/MRZ decode fail, hoặc người vận hành chủ động chọn.
`confidence = NeedsReview`, `source = 'manual'`.

### 6.5 Normalizer

**Không map trực tiếp giữa hai nguồn** (G9).

| Dữ liệu | Từ QR | Từ MRZ | Lưu nội bộ | Xuất XLSX | Xuất XML |
|---|---|---|---|---|---|
| Giới tính | `Nữ` / `Nam` | `F` / `M` / `<` | `F` / `M` | `F - Nữ` | `F` |
| Ngày | `ddMMyyyy` | `yyMMdd` | ISO `yyyy-MM-dd` | `dd/MM/yyyy` text | `dd/MM/yyyy` |
| Quốc tịch | suy `VNM` | ISO3 dòng 2 | ISO3 | `VNM - Viet Nam` | `VNM` |

`<` trong ô giới tính MRZ → `NeedsReview`, người chọn.

#### Suy thế kỷ — HAI hàm khác nhau (G6)

```rust
// Ngày sinh: luôn trong quá khứ, cửa sổ 100 năm
fn century_dob(yy: u32, today_year: u32) -> u32 {
    if 2000 + yy <= today_year { 2000 + yy } else { 1900 + yy }
}

// Ngày hết hạn: gần như luôn tương lai, cửa sổ [nay-10, nay+90]
fn century_expiry(yy: u32, today_year: u32) -> u32 {
    if 2000 + yy >= today_year - 10 { 2000 + yy } else { 1900 + yy }
}
```

Test bắt buộc: `900308` → `1990-03-08`; `351209` → `2035-12-09`; và một test
khẳng định hai hàm cho kết quả **khác nhau** trên cùng `yy = 35`. Dùng chung một
hàm sẽ ra 1935 — bug thật đã xảy ra khi soạn spec v2.

#### Đọc từ bảng cũ

- `check_in_at` là timestamp có offset, `expected_checkout` là date trần (H3).
  Chuẩn hóa cả hai về date địa phương trước khi so sánh hoặc ghi file.
- Số phòng = `rooms.name` cắt tiền tố `"Phòng "` (H4).

---

## 7. Ghép danh tính ↔ lượt lưu trú

Danh tính từ ảnh **không có**: số phòng, ngày đến, ngày đi, lý do lưu trú.
Mấy cái đó nằm trong `bookings` / `rooms`.

1. Lọc booking `status = 'active'` hoặc `check_in_at` trong ±2 ngày
2. Xếp hạng theo độ giống tên (bỏ dấu, lowercase, so token)
3. **Người chọn, app chỉ gợi ý thứ tự. Không auto-confirm.** Tên trong CapyInn
   là "Andrei", tên khai báo là "ZOLOCHEVSKAIA VERONIKA" — thuật toán nào tự tin
   ghép được cặp đó là thuật toán sẽ ghép sai cặp khác.
4. Sau khi chọn: nhập `stay_reason` (mặc định `1 - Du lịch`), và
   `visa_valid_until` nếu là NNN.

**Không cho tạo `declaration_link` thiếu `stay_id`.** Nếu khách không có trong
`bookings` thì phải tạo booking trước — nếu không, số phòng và ngày đến phải
nhập tay và mất luôn lợi ích chống-nhập-trùng.

---

## 8. Validator

Cổng không báo lỗi (F3) → toàn bộ việc kiểm nằm ở đây.

Chạy trên `Vec<DeclarationRow>` đã ghép, trả `Vec<Finding>` gồm `code`,
`severity`, `link_id`, `field`. Không ghi file, không chạm DB.

### 8.1 Blocking — không cho xuất file

| Mã | Điều kiện |
|---|---|
| `E01` | Thiếu field bắt buộc của định dạng tương ứng. **VN:** `B C D E F H N O Q` (H6). **NNN:** `ho_ten`, `ngay_sinh`, `gioi_tinh`, `ma_quoc_tich`, `so_ho_chieu`, `ngay_den`, `ngay_di_du_kien`, `thoi_han_tam_tru` |
| `E02` | `full_name` chỉ có 1 token, **và** `single_token_name_ok = 0` |
| `E03` | **NNN:** `full_name` có dấu tiếng Việt hoặc chữ thường |
| `E04` | **NNN:** `name_confirmed_by_human = 0` (G5) |
| `E05` | `nationality_iso3` không thuộc 205 mã trong catalog |
| `E06` | Giá trị enum không khớp danh mục |
| `E07` | `expected_check_out` < `check_in_date` |
| `E08` | **NNN:** thiếu `visa_valid_until` |
| `E09` | **NNN:** `visa_valid_until` < `expected_check_out` → khách sẽ quá hạn tạm trú |
| `E10` | **NNN:** `visa_valid_until` == `passport_expiry` → nghi lấy sai nguồn (G10) |
| `E11` | `doc_type_code = '9'` mà thiếu `doc_type_name` |
| `E12` | `stay_reason = '20'` mà thiếu `stay_reason_note` |
| `E13` | `doc_no` / `passport_no` rỗng hoặc chứa ký tự lạ |
| `E14` | Trùng trong cùng lô (cùng số giấy tờ + cùng ngày đến) |

`E02` bắt đúng lỗi thường gặp là OCR cụt tên, nhưng sẽ chặn nhầm tên đơn có
thật — hộ chiếu Indonesia dạng mononym ghi `SUHARTO<<`, tách ra đúng một token và
hoàn toàn hợp lệ. Ô tick *"Tên trên giấy tờ chỉ có một chữ"* trong UI set
`single_token_name_ok = 1` để gỡ chặn. **Người phải tick, không tự gỡ.**

`E09` không phải lỗi format — là cảnh báo nghiệp vụ thật. Khách ở quá hạn visa
là chuyện cơ sở lưu trú phải biết **trước** khi khai.

`E10` chặn đúng cái bẫy G10. Về lý thuyết hai ngày này có thể trùng thật, nhưng
xác suất đó nhỏ hơn nhiều so với xác suất ai đó copy nhầm 2035 từ MRZ sang ô
thời hạn tạm trú, và hậu quả của cái sau là khai sai dưới ô "Tôi xin chịu trách
nhiệm trước pháp luật".

**Không có `E15`.** v1 để trống cột K/L (F8, H6, G7) nên không có
`province_code` / `ward_code` để kiểm. TODO v2 nếu sau này cho chọn tay.

### 8.2 Cảnh báo — cho xuất, phải xác nhận

| Mã | Điều kiện |
|---|---|
| `W01` | Thiếu số phòng |
| `W02` | Thiếu số điện thoại (khách VN) |
| `W03` | `stay_reason` vẫn là mặc định, chưa ai đổi |
| `W04` | `check_in_at` cách hôm nay > 24h mà chưa có lô `verified` → **đã quá hạn khai báo** |
| `W05` | `extract_confidence = 'needs_review'` |
| `W06` | `doc_type_source = 'heuristic'` (G8) |

`W04` trả về từ cùng một hàm validator để Dashboard dùng lại, không viết truy
vấn thứ hai.

---

## 9. Writer

### 9.1 XmlWriter — khách nước ngoài

Sinh từ đầu, không cần template. 12 field phẳng, 1 enum. **Không thêm
dependency** — `quick-xml` là thừa cho 12 tag không thuộc tính. Build string,
escape đủ 5 thực thể `& < > " '`.

Tên từ MRZ chỉ có `A-Z` nên hiếm khi cần escape, nhưng đường nhập tay thì có, và
một dấu `&` chưa escape làm hỏng cả file — mà cổng sẽ nhận file hỏng đó rồi báo
"thành công" (F3).

```xml
<?xml version="1.0" encoding="UTF-8"?>
<KHAI_BAO_TAM_TRU>
    <THONG_TIN_KHACH>
        <so_thu_tu>1</so_thu_tu>
        <ho_ten>ZOLOCHEVSKAIA VERONIKA</ho_ten>
        <ngay_sinh>08/03/1990</ngay_sinh>
        <ngay_sinh_dung_den>D</ngay_sinh_dung_den>
        <gioi_tinh>F</gioi_tinh>
        <ma_quoc_tich>RUS</ma_quoc_tich>
        <so_ho_chieu>777785671</so_ho_chieu>
        <so_phong>5A</so_phong>
        <ngay_den>25/07/2026</ngay_den>
        <ngay_di_du_kien>30/07/2026</ngay_di_du_kien>
        <ngay_tra_phong>29/07/2026</ngay_tra_phong>
        <thoi_han_tam_tru>15/09/2026</thoi_han_tam_tru>
    </THONG_TIN_KHACH>
</KHAI_BAO_TAM_TRU>
```

- `ma_quoc_tich` = **chỉ mã ISO3**, không kèm tên (khác XLSX — F5)
- `gioi_tinh` = `M`/`F` trần, không kèm nhãn
- Đây là chỗ duy nhất cùng một dữ liệu ghi ra hai dạng khác nhau tùy định dạng.
  Tách hẳn thành hai hàm format, **không dùng chung**.
- `ngay_sinh_dung_den` = `D`. v1 chỉ cho xuất khách có ngày sinh đủ (§14.2)
- `ngay_tra_phong` = **bỏ hẳn tag** nếu khách chưa checkout; có giá trị thì là
  khai bù (F9). Không ghi tag rỗng.
- `so_thu_tu` 1-based liên tiếp

#### Cờ `xml_lead_example`

§14.1 chưa có câu trả lời và chỉ người vận hành test được (captcha + Google
Authenticator). XmlWriter có setting `xml_lead_example`, **mặc định `false`**.
Khi bật, writer chèn một record mồi `[EXAMPLE]` ở vị trí 1 và data bắt đầu từ vị
trí 2 — đối xứng với luật row 4 của XLSX (F2). Lật cờ là một dòng setting, không
phải sửa code.

### 9.2 XlsxWriter — khách Việt Nam

**Nguyên tắc số 1: KHÔNG tạo file từ đầu. Ghi thêm vào template.**
Template `tblt_vn_import.xlsx` đóng gói trong app resources. Thư viện
`umya-spreadsheet`.

```
1. Copy template → TBLT_<cslt>_<yyyyMMdd>_<HHmm>.xlsx
2. Mở sheet "DS_KHACH_VIET_NAM_LUU_TRU"
3. TUYỆT ĐỐI KHÔNG SỬA row 1, 2, 3, 4.
   Row 4 là hàng [EXAMPLE]. Server bỏ nó THEO VỊ TRÍ (F2). Giữ nguyên.
4. Ghi data từ row 5, liên tiếp, không để dòng trống ở giữa
5. STT (cột A) bắt đầu từ 2 (row 4 đã là STT 1) — đã test, chạy được
6. Cột C, N, O (ngày) và H (số giấy tờ): ghi dạng STRING + number_format '@'
7. Enum: ghi nguyên chuỗi `display` từ catalog (F5)
8. Save
```

`<cslt>` lấy từ bảng `settings`, fallback `CSLT`. Thư mục xuất mặc định là
`app_identity::exports_dir().join("khai-bao-tam-tru")` = `~/CapyInn/exports/khai-bao-tam-tru/`
— dùng lại đúng quy ước thư mục sẵn có của app, không tự đẻ đường dẫn mới. Người
dùng đổi được, lưu vào `settings`. Mở thư mục bằng `tauri-plugin-opener` đã có sẵn.

#### Mapping cột

| Cột | Header | Nguồn | Bắt buộc |
|---|---|---|---|
| A | STT | index, từ 2 | ✱ |
| B | HỌ TÊN (*) | `full_name` (từ QR, có dấu) | ✱ |
| C | NGÀY SINH (*) | `dob` → `dd/MM/yyyy` text | ✱ |
| D | GIỚI TÍNH (*) | `F - Nữ` / `M - Nam` | ✱ |
| E | QUỐC TỊCH (*) | `VNM - Viet Nam` | ✱ |
| F | LOẠI GIẤY TỜ (*) | `doc_type_code` → display | ✱ |
| G | TÊN GIẤY TỜ | `doc_type_name` | khi F = `9 - Giấy Tờ Khác` |
| H | SỐ GIẤY TỜ (*) | `doc_no` (string) | ✱ |
| I | SỐ ĐIỆN THOẠI | `phone` | – |
| J | NƠI CƯ TRÚ HIỆN NAY | `residence_status` → display | – |
| K | TỈNH/ THÀNH PHỐ | – | – (**v1 để trống**, G7) |
| L | PHƯỜNG/ XÃ/ ĐẶC KHU | – | – (**v1 để trống**, G7) |
| M | ĐỊA CHỈ CHI TIẾT | `address_detail` — chuỗi thô từ QR | – |
| N | NGÀY ĐẾN (*) | từ `bookings.check_in_at` | ✱ |
| O | NGÀY ĐI DỰ KIẾN (*) | từ `bookings.expected_checkout` | ✱ |
| P | SỐ PHÒNG/ KHOA | `rooms.name` cắt tiền tố | – |
| Q | LÝ DO CƯ TRÚ (*) | `stay_reason` → display | ✱ |
| R | NHẬP LÝ DO | `stay_reason_note` | khi Q = `20 - Mục đích khác` |
| S | GHI CHÚ | – | – |

**Header có dấu cách lạ** — `'TỈNH/ THÀNH PHỐ '`, `' LÝ DO CƯ TRÚ (*)'` có space
đầu/cuối. **Ghi theo vị trí cột, không match theo tên header.**

#### Gate bắt buộc sau khi ghi

Đọc lại file vừa tạo và assert. Vì `umya-spreadsheet` ghi lại **toàn bộ**
workbook chứ không chỉ sheet 1, gate phải kiểm cả những thứ nó có thể làm rơi.
F4 chỉ chứng minh **openpyxl** re-save không phá file — không nói gì về umya.

| # | Assert | Bắt được gì |
|---|---|---|
| 1 | Row 4 cột A vẫn đúng là `[EXAMPLE]` | Ghi đè hàng mồi → 0 record, "thành công" |
| 2 | Row 5 cột B khớp khách đầu tiên | Lệch dòng |
| 3 | Mọi cell cột C, N, O là string dạng `dd/MM/yyyy`, không phải số | F6, cạm bẫy 13.4 |
| 4 | Cột H giữ đủ ký tự, kể cả số 0 đầu | F7 |
| 5 | Sheet `DANH_MUC` còn đủ 9 / 3 / 20 / 2 / 205 dòng | umya làm rơi sheet danh mục |
| 6 | `TINH_THANH` còn 34 dòng, `PHUONG_XA` còn 3.323 dòng | như trên |
| 7 | Còn đủ 40 defined name (`LOAI_GIAY_TO` … `PX_823`) | umya làm rơi named range |

Fail bất kỳ assert nào → **xóa file, báo lỗi, KHÔNG ghi `declaration_batch`.**
Không có file nửa vời nào ra khỏi app.

Nếu gate 5–7 đỏ ngay lần chạy đầu, ta biết ở Bước 4 chứ không phải sau khi cổng
im lặng nuốt một lô. Khi đó fallback là coi xlsx như zip, patch riêng
`xl/worksheets/sheet1.xml`, giữ nguyên byte các entry khác.

---

## 10. Vòng đối chiếu

**Lý do tồn tại:** F3. Cổng nói "thành công" khi import 0 dòng. Nếu người vận
hành tin câu đó, khách không được khai mà không ai biết. Chuyện này **đã xảy ra
thật** trong lần test đầu.

### v1 — thủ công, có kiểm soát

Sau khi xuất file, hiện checklist **bắt buộc hoàn thành**:

```
Lô #12 · Khách Việt Nam · 26/07/2026 · 3 hồ sơ
Đã xuất: TBLT_BinhAn_20260726_1030.xlsx          [ Mở thư mục ]
□ Đã upload lên cổng
□ Đã bấm "Làm mới" trên màn danh sách của cổng
  Số hồ sơ thấy trên cổng:  [ ___ ]   (cần đúng 3)
```

- gõ `3` → lô `verified`, 3 khách chuyển sang đã khai báo
- gõ khác `3` → lô `failed`, 3 khách **giữ nguyên** trạng thái chưa khai, badge
  sidebar tiếp tục đếm họ

**Không cho đánh dấu hoàn thành bằng một cú bấm.** Con số phải được gõ vào. Đây
là chủ ý — checkbox sẽ được tick theo phản xạ, ô nhập số buộc phải nhìn màn hình
cổng.

Cơ chế này rơi ra miễn phí từ truy vấn §5.3: chỉ `declaration_entry` thuộc lô
`verified` mới được tính.

### v2 — tự động

Cổng có nút **"Tải danh sách"** (xuất Excel hồ sơ đã khai). Cho người vận hành
kéo file đó vào CapyInn → tự diff theo `(số giấy tờ + ngày đến)` → tự đánh
`verified` và chỉ ra khách bị thiếu.

---

## 11. UI

### Sidebar

Thêm vào `NAV_MANAGEMENT` trong `mhm/src/app/MainShell.tsx`, dưới `Night Audit`:

```ts
{ key: "declaration", label: "Khai báo tạm trú", icon: ShieldCheck }
```

Nav hiện toàn tiếng Anh, nhưng nội dung bên trong app đã là tiếng Việt
(`CheckinSheet` viết "Điền thông tin khách hàng…"). "Khai báo tạm trú" là thuật
ngữ pháp lý không có bản dịch tiếng Anh dùng được. Thêm entry vào `PAGE_TITLES`.

Badge số đỏ = tổng khách chưa khai, check-in trong 48h (§5.3). Con số này làm
module hữu ích ngay từ trước khi ai bấm vào.

### Màn chính — 4 khối

**Khối 1 — Kéo ảnh vào đây.** Vùng drop lớn, nhận nhiều ảnh cùng lúc. Mỗi ảnh →
một thẻ kết quả:

- Nguồn (QR CCCD / MRZ hộ chiếu / cần nhập tay)
- Badge `✓ Verified` hoặc `⚠ Cần xác nhận`
- Với MRZ: **crop 2 dòng MRZ hiện ngay cạnh ô tên**, cùng một tầm mắt
- Nút `Xác nhận tên` → `name_confirmed_by_human = 1`
- Ô tick `Tên trên giấy tờ chỉ có một chữ` → `single_token_name_ok = 1`
- Dropdown ghép với khách đang ở (§7)
- Ô `Thời hạn tạm trú` nếu là NNN — **bắt buộc, nhập tay**

Vị trí của crop MRZ là chi tiết UI duy nhất trong module có hậu quả pháp lý.
`E04` chặn xuất cho tới khi người ta thật sự nhìn hai dòng đó. Nếu crop nằm chỗ
khác, phải cuộn, hoặc phải bấm mở — thao tác xác nhận sẽ thành phản xạ và `E04`
mất hết ý nghĩa.

**Khối 2 — Cần khai báo.** Danh sách đã ghép, nhóm theo NNN / VN (xuất 2 file
khác nhau). Cột: tên · loại · phòng · ngày đến · trạng thái validate. Dòng có
lỗi blocking hiện mã lỗi, click → nhảy tới chỗ sửa.

**Khối 3 — Xuất file.**

- Chọn loại: `Khách nước ngoài (XML)` / `Khách Việt Nam (XLSX)`
- Tick từng khách hoặc chọn khoảng ngày
- Nút `Kiểm tra` → bảng lỗi/cảnh báo
- Nút `Xuất file` — **disabled khi còn lỗi blocking**
- Xuất xong: mở thư mục + hiện checklist §10
- **Cảnh báo cố định, không đóng được:** "Không mở/sửa file này bằng Excel trước
  khi upload. Excel sẽ làm mất số 0 đầu của số giấy tờ và đổi định dạng ngày.
  Cần sửa thì sửa trong CapyInn rồi xuất lại."

**Khối 4 — Lịch sử lô.** Bảng `declaration_batch`. Lô `failed` hoặc `uploaded`
chưa verified: nổi lên đầu, màu cảnh báo.

### Chỗ khác trong app

- **Dashboard:** thẻ "Chưa khai báo tạm trú: N", dùng lại đúng hàm validator
  (`W04`), không viết truy vấn thứ hai.
- **Không sửa gì trên màn check-in hiện tại.** Đây là khác biệt lớn nhất so với
  v1 và là thứ giữ cho PMS đang chạy không bị động vào.

---

## 12. Bảo mật dữ liệu cá nhân

Nghị định 13/2023 về bảo vệ dữ liệu cá nhân áp dụng — nó nằm ngay trong mục Văn
bản, quy định của cổng. Ảnh giấy tờ và nội dung QR đều là dữ liệu cá nhân.

Quy tắc cứng:

1. **Không copy ảnh vào storage của CapyInn.** Đọc từ đường dẫn tạm, trích xuất,
   rồi bỏ. Không có cột `photo_path`.
2. **Không lưu payload QR/MRZ thô.** Payload QR chứa nguyên số CCCD + tên + ngày
   sinh + địa chỉ. Chỉ lưu các trường đã parse mà khai báo cần. `raw_payload`
   không tồn tại trong schema.
3. **Không log payload, không log đường dẫn ảnh** ra file log hay console ở
   production build.
4. `crop_for_review` chỉ tồn tại trong RAM, không ghi ra đĩa.
5. **Dọn dữ liệu cũ bằng cách che, không xóa.** Sau khi lô `verified` + N ngày
   (mặc định 90, đặt trong Settings): null hóa `full_name`, `dob`, `doc_no`,
   `passport_no`, `address_detail`, `phone`; set `redacted_at`.

Điểm 5 khác spec v2, vốn ghi "tự xóa `declaration_identity`". Xóa dòng sẽ phá
`declaration_link` → `declaration_entry` → lịch sử lô, tức là mất luôn bằng
chứng "khách này đã khai ngày nào, lô nào" — mà bằng chứng đó chính là thứ cần
giữ khi có ai hỏi. Che giữ được quan hệ, bỏ được dữ liệu cá nhân.

Điểm 1–4 rẻ nếu làm từ đầu, đắt nếu phải đi dọn dữ liệu cũ sau.

---

## 13. Cạm bẫy đã biết

**13.1 — Đừng tiền xử lý ảnh.** G2, G3. Thư viện đúng quan trọng hơn mọi thủ
thuật làm sạch. Thứ tự thử: thô trước, xử lý mạnh sau, và **dùng checksum để
chấm điểm** thay vì tin biến thể "nhìn sạch nhất".

**13.2 — Tên trong MRZ không có checksum.** G5. Đây là trường duy nhất bắt buộc
người xác nhận. "MRZ pass 5/5" và "tên đúng" là hai mệnh đề độc lập — 5 checksum
nằm hết ở dòng 2. Đừng gộp chúng lại.

**13.3 — `thoi_han_tam_tru` ≠ ngày hết hạn hộ chiếu.** G10. MRZ cho 2035, thời
hạn tạm trú thực tế có thể là 45 ngày. Lấy sai là khai sai với công an dưới ô
tick "Tôi xin chịu trách nhiệm trước pháp luật". `E10` chặn đúng chỗ này.

**13.4 — Excel tự phá dữ liệu.** Mở file đã xuất bằng Excel rồi sửa tay:
`079174011721` → `79174011721`, `18/10/1974` → serial number.

**13.5 — Danh mục hành chính đã đổi và dữ liệu trên giấy tờ thì chưa.** G7. Còn
34 tỉnh, 3.323 phường/xã, có loại `Đặc khu`. Khánh Hòa `511` bao gồm cả Ninh
Thuận cũ. Nha Trang có 4 phường tên gần nhau (`Phường Nha Trang`,
`Bắc/Nam/Tây Nha Trang`). **Không fuzzy-match tên phường.** Sai địa chỉ không ai
báo lỗi — nó chỉ là một khai báo sai.

**13.6 — Schema cổng đang thay đổi.** Tài liệu HDSD chính thức đã sai 2 trường
bắt buộc so với form thật chỉ sau vài tháng. Danh mục nằm trong
`kbtt_catalog.json` sinh từ template, có `_source_sha256` và `_source_date`.
Kiểm lại mỗi quý hoặc khi import fail bất thường.

**13.7 — Cross-platform.** `rxing` là pure Rust, chạy cả hai OS. Nhưng
`ocr.rs` hiện hardcode `Backend::Metal` (H7) — **đây là bug cross-platform có sẵn
trong repo, không do module này gây ra.** Nếu `OcrRsMrz` thắng ở Bước 1 thì phải
xử lý backend theo `cfg!(target_os)` trước khi build Windows. Ghi TODO riêng,
không gộp vào phạm vi v1.

**13.8 — `guest_type` nói dối.** H2. Đừng dùng nó để phân loại NNN/VN. Nguồn duy
nhất đúng là `nationality_iso3` trích từ ảnh.

---

## 14. Ẩn số — chưa test, đừng giả định

| # | Câu hỏi | Cách trả lời |
|---|---|---|
| 14.1 | **XML chưa được test lần nào.** Có bị skip record đầu như XLSX skip row 4 (F2)? | Sửa 2 `ho_ten` trong `Danh_Sach_Mau.xml` thành hai tên phân biệt được, upload, đếm cổng nhận mấy hồ sơ |
| 14.2 | `ngay_sinh_dung_den` khi chỉ biết năm sinh = gì? | Thử `Y`, `N`, rỗng trên form web trước |
| 14.3 | Server reject cả file hay skip dòng lỗi? | Upload 3 dòng, cố ý sai 1 |
| 14.4 | Giới hạn số dòng / dung lượng? | Thử 50 dòng |
| 14.5 | XLSX: STT ở row 5 nên từ 1 hay 2? | Đã chạy với `2`. Thử `1` xem có khác |
| 14.6 | Thẻ Căn Cước mới (2024+) có payload QR khác không? | Cần 1 ảnh thẻ mới để test |
| 14.7 | `ocr-rs` có đọc được MRZ không? | `kbtt_probe` trên 5–10 ảnh hộ chiếu thật |

**14.1 là ẩn số nguy hiểm nhất.** Ba kết quả, ba hệ quả:

| Cổng nhận | Nghĩa | Hệ quả |
|---|---|---|
| 2 | Không có luật bỏ record | §9.1 giữ nguyên, `xml_lead_example = false` |
| 1 | Bỏ record đầu theo vị trí, giống F2 | Bật `xml_lead_example = true` |
| 0 | File mẫu cũng không vào | Có luật khác chưa biết, phải điều tra trước khi dùng |

Cờ `xml_lead_example` khiến cả ba nhánh chỉ là một dòng setting.

---

## 15. Thứ tự thực thi

### Bước 0 — trước khi dùng XmlWriter thật

Người vận hành test §14.1 trên cổng. Cần login captcha + Google Authenticator
nên không tự động hóa được.

### Kế hoạch A — lõi, không UI

1. `scripts/gen_kbtt_catalog.py` + `catalog.rs`
2. Migration v20 + `repo.rs`
3. `extractor` (QR, MRZ, manual) + `normalizer`
4. `bin/kbtt_probe.rs`
5. `validator`
6. `XmlWriter`
7. `XlsxWriter` + gate 7 assert

**Nghiệm thu kế hoạch A:** `kbtt_probe` chạy trên ảnh thật đạt gate; validator
chạy trên 100% khách đang ở và ra được con số "bao nhiêu khách khai báo được
ngay hôm nay"; xuất được một file XML và một file XLSX thật để upload thử. Chưa
cần một dòng React nào.

### Kế hoạch B — UI và vòng đối chiếu

1. `commands/declaration.rs`
2. Trang 4 khối
3. Badge sidebar
4. Thẻ Dashboard
5. Checklist đối chiếu

Thứ tự XmlWriter trước XlsxWriter là cố ý: NNN chỉ có 12 field phẳng, một enum,
không địa chỉ hành chính — và nghĩa vụ với khách NNN nặng hơn (Thông tư 53/2016,
hạn 24h).

### v2 — sau này

Parse "Tải danh sách" để đối chiếu tự động; PhotoKit; TD1/TD2; cho chọn tay
tỉnh/phường và khôi phục `E15`.

---

## 16. Chiến lược test

| Loại | Nội dung |
|---|---|
| Unit | `catalog`, `normalizer`, `validator` — hàm thuần, không DB, không Tauri |
| Suy thế kỷ | `900308` → `1990-03-08`; `351209` → `2035-12-09`; và một test khẳng định hai hàm cho kết quả **khác nhau** trên `yy = 35` |
| MRZ checksum | Trên chuỗi mẫu đã verify 5/5 |
| MRZ tên | MRZ 5/5 với **dòng 1 bị sửa ký tự** → `confidence` vẫn `Verified` nhưng `name_confirmed_by_human` vẫn `0` |
| QR | Payload 7 trường → đúng 7 field; payload 6 hoặc 8 trường → fail, không đoán |
| XmlWriter | Golden file — input cố định, so byte với XML kỳ vọng. Có test cho escape `&` và cho việc bỏ tag `ngay_tra_phong` |
| XlsxWriter | Chạy đủ 7 assert của gate trên file thật vừa ghi |
| Test âm | App **không bao giờ** ghi vào row 4 |
| Test ranh giới | Đọc source `declaration/`: không có `INSERT`/`UPDATE`/`DELETE`/`ALTER` nào chạm `guests`/`bookings`/`booking_guests`/`rooms` |
| Test bảo mật | Đọc source: không có `photo_path`, `raw_payload`; không ghi file ngoài `writer/` |
| Đo thực địa | `kbtt_probe` trên 5–10 ảnh thật. Gate: QR CCCD ra đúng 7 trường; MRZ đạt ≥3/5 checksum |

---

## 17. Định nghĩa "xong" cho v1

- [ ] Kéo 1 ảnh thẻ CCCD vào → ra đúng 7 trường, không sửa gì
- [ ] Kéo 1 ảnh hộ chiếu vào → dòng 2 pass ≥3/5 checksum, tên hiện kèm crop MRZ
- [ ] Không có ảnh nào bị copy vào storage của app (kiểm bằng đọc đĩa)
- [ ] `raw_payload` và `photo_path` không tồn tại trong DB schema
- [ ] Validator chạy trên 100% khách đang ở → 0 lỗi blocking
- [ ] Gate 7 assert của XlsxWriter xanh
- [ ] Test ranh giới xác nhận không có write nào chạm bảng cũ
- [ ] Xuất 1 file XML NNN → upload → số record trên cổng khớp
- [ ] Xuất 1 file XLSX VN → upload → số record trên cổng khớp
- [ ] Test âm: app **không bao giờ** ghi vào row 4 của XLSX
- [ ] Lô `failed` vẫn giữ khách ở trạng thái chưa khai báo
- [ ] Badge sidebar đếm đúng số khách chưa khai trong 48h
- [ ] Build chạy được trên cả macOS và Windows
