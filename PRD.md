# CapyInn
## Product Requirements Document v1.0
> Audited: 14/03/2026

---

## 1. Tổng quan

**CapyInn** là desktop app quản lý khách sạn mini 10 phòng, chạy offline hoàn toàn trên macOS.  
Thay thế toàn bộ quy trình thủ công hiện tại: ghi sổ tay, nhập web lưu trú, tính tiền bằng tay.

**Mục tiêu:** Giảm thời gian xử lý 1 khách từ ~5 phút xuống còn ~60 giây.

---

## 2. Tech Stack

| Layer | Công nghệ |
|---|---|
| App shell | Tauri 2.0 |
| Backend | Rust |
| Frontend | React 18 + TypeScript + Tailwind CSS + shadcn/ui |
| Database | SQLite (via sqlx) |
| OCR | ocr-rs (PaddleOCR v5 + MNN backend) |
| State | Zustand |
| Charts | Recharts |
| Build size | ~25MB (bao gồm OCR model files ~15MB) |

**Lý do chọn Tauri thay Electron:** Nhẹ hơn 10-20x, dùng WKWebView native của macOS, backend Rust.  
**Lý do chọn ocr-rs thay Tesseract:** Cross-platform, pure Rust, thiết kế cho ID card/CCCD, hỗ trợ tiếng Việt tốt hơn, GPU Metal trên macOS.

---

## 3. Cấu trúc phòng

```
10 phòng — 5 tầng, mỗi tầng 2 phòng:

┌─────────────────────────────────────┐
│  Tầng 5 │  5A (Deluxe/ban công)  │  5B (Standard/cửa sổ)  │
│  Tầng 4 │  4A (Deluxe/ban công)  │  4B (Standard/cửa sổ)  │
│  Tầng 3 │  3A (Deluxe/ban công)  │  3B (Standard/cửa sổ)  │
│  Tầng 2 │  2A (Deluxe/ban công)  │  2B (Standard/cửa sổ)  │
│  Tầng 1 │  1A (Deluxe/ban công)  │  1B (Standard/cửa sổ)  │
└─────────────────────────────────────┘

Loại A — Deluxe:   2 giường đôi + ban công
Loại B — Standard: cửa sổ
```

---

## 4. Quy trình nghiệp vụ (Business Logic)

### 4.1 Check-in khách trong nước
1. Khách đưa CCCD → scan bằng Canon LiDE 300 → lưu vào `~/CapyInn/Scans/`
2. App phát hiện file mới → OCR tự động → extract: **Họ tên, Số CCCD, Ngày sinh, Địa chỉ**
3. Popup hiện thông tin khách → anh assign vào phòng
4. App tạo booking, ghi check-in time
5. Nút **"Copy thông tin lưu trú"** → copy sẵn format cho web Bộ Công An

### 4.2 Check-in khách nước ngoài
1. Scan **2 lần**: tờ thông tin cá nhân + tờ visa (2 file riêng)
2. OCR extract: **Họ tên, Số passport, Ngày sinh, Quốc tịch, Hạn visa**
3. Assign phòng tương tự khách trong nước
4. Nút **"Copy thông tin xuất nhập cảnh"** → format cho web xuất nhập cảnh

### 4.3 Assign nhiều khách cùng lúc
- Scan 3 CCCD → 3 file vào `~/CapyInn/Scans/` → OCR chạy song song
- Popup: "Phát hiện 3 khách mới" → anh drag/drop hoặc dropdown assign
- Linh hoạt: A+B chung phòng, C phòng riêng

### 4.4 Check-out
- Chọn phòng → bấm Check-out
- Hiện tổng tiền, số tiền đã trả, còn nợ
- Confirm → phòng chuyển sang 🟡 Cần dọn

### 4.5 Tính tiền
```
Qua đêm:  Check-in từ 12:00 trưa → Check-out 11:00 sáng hôm sau
Giá:      Config theo từng loại phòng (A hoặc B), có thể chỉnh tay
Extend:   Thêm 1 đêm từ thời điểm hiện tại
Trả sớm:  Tính theo số đêm thực tế
Nợ:       Đánh dấu, track riêng
```

### 4.6 Cuối ngày
- Dashboard hiện tổng: phòng trống / đang có khách / cần dọn
- Doanh thu hôm nay
- Nhắc nhở phòng nào sắp check-out ngày mai

### 4.7 Cuối tháng
- Tổng doanh thu (tiền phòng)
- Tổng chi phí (điện, nước, rác, internet, bảo trì thang máy, khác)
- Lợi nhuận = Doanh thu - Chi phí
- Export CSV cho kế toán/báo cáo thuế

---

## 5. File Watcher

```
Thư mục watch: ~/CapyInn/Scans/   (anh config Canon LiDE 300 output vào đây)
Trigger:        File mới xuất hiện (.jpg, .jpeg, .png, .pdf, .tiff)
Action:         OCR ngay lập tức → hiện popup assign
Fallback:       Nếu OCR sai → cho sửa tay trước khi save
```

---

## 6. OCR — Chi tiết kỹ thuật

```
Engine:   ocr-rs (PaddleOCR v5 + MNN)
Backend:  Metal (macOS Apple Silicon/Intel GPU)
Language: vi + en
Speed:    ~200-300ms / ảnh trên Apple Silicon

Parse CCCD:
├── Số CCCD (12 số)
├── Họ và tên
├── Ngày sinh (DD/MM/YYYY)
├── Giới tính
├── Quốc tịch
└── Địa chỉ thường trú

Parse Passport:
├── Số passport
├── Họ và tên (Latin)
├── Ngày sinh
├── Quốc tịch
├── Ngày hết hạn passport
└── Số visa + ngày hết hạn (từ tờ visa)
```

---

## 7. UI — 4 Tabs

### Tab 1 — Dashboard
```
┌─────────────────────────────────────────────┐
│  CapyInn                      Hôm nay: 14/3 │
│  7/10 phòng có khách  |  Doanh thu: 2.1tr   │
├─────────────────────────────────────────────┤
│                                             │
│   [1A] 🔴  [1B] 🟢  [2A] 🔴  [2B] 🔴       │
│   [3A] 🟢  [3B] 🟡  [4A] 🔴  [4B] 🟢       │
│   [5A] 🔴  [5B] 🔴                          │
│                                             │
│  🟢 Trống  🔴 Có khách  🟡 Cần dọn  🔵 Đặt trước │
└─────────────────────────────────────────────┘

Màu sắc:
🟢 Xanh lá  = Trống, sẵn sàng nhận khách
🔴 Đỏ       = Đang có khách
🟡 Vàng     = Vừa checkout, cần dọn
🔵 Xanh dương = Đặt trước (booking)
```

### Tab 2 — Chi tiết phòng
```
Khi click vào ô phòng trên Dashboard:
├── Thông tin khách: tên, CCCD/passport, quốc tịch
├── Check-in: ngày giờ
├── Check-out dự kiến: ngày giờ
├── Số đêm đã ở / còn lại
├── Thanh toán: đã trả Xđ / còn nợ Xđ
├── Dịch vụ phát sinh (nếu có)
├── [Copy thông tin lưu trú] — format web Bộ Công An
├── [Copy thông tin XNC]    — chỉ khách nước ngoài
├── [Extend]  — thêm 1 đêm
├── [Check-out]
└── [Ghi chú]
```

### Tab 3 — Thống kê
```
Toggle: Hôm nay / Tuần này / Tháng này / Tùy chọn

Revenue:
├── Tổng tiền phòng thu được
├── Số phòng đã bán
├── Công suất: X/10 phòng (X%)
└── Biểu đồ doanh thu theo ngày

Expense (nhập tay):
├── Điện
├── Nước
├── Rác
├── Internet
├── Bảo trì thang máy
├── Khác (ghi chú tự do)
└── Tổng chi phí

Profit = Revenue - Expense

[Export CSV]  ← cho kế toán / báo cáo thuế
```

### Tab 4 — Housekeeping
```
Danh sách phòng cần xử lý hôm nay:
├── Sort theo giờ checkout (dọn sớm = nhận khách mới sớm)
├── Trạng thái: 🟡 Cần dọn → 🔄 Đang dọn → 🟢 Sạch
├── Ghi chú bảo trì: bóng đèn, điều hòa, vòi nước...
└── Timestamp: dọn xong lúc mấy giờ
```

---

## 8. Database Schema (SQLite)

```sql
-- Phòng
CREATE TABLE rooms (
  id          TEXT PRIMARY KEY,  -- '1A', '1B', '2A'...
  name        TEXT NOT NULL,
  type        TEXT NOT NULL,     -- 'deluxe' | 'standard'
  floor       INTEGER NOT NULL,
  has_balcony BOOLEAN NOT NULL,
  base_price  REAL NOT NULL,
  status      TEXT NOT NULL      -- 'vacant'|'occupied'|'cleaning'|'booked'
);

-- Khách
CREATE TABLE guests (
  id              TEXT PRIMARY KEY,
  guest_type      TEXT NOT NULL,  -- 'domestic' | 'foreign'
  full_name       TEXT NOT NULL,
  doc_number      TEXT NOT NULL,  -- CCCD hoặc passport number
  dob             TEXT,           -- YYYY-MM-DD
  gender          TEXT,
  nationality     TEXT DEFAULT 'Việt Nam',
  address         TEXT,
  visa_expiry     TEXT,           -- chỉ khách nước ngoài
  scan_path       TEXT,           -- đường dẫn file ảnh scan
  created_at      TEXT NOT NULL
);

-- Booking
CREATE TABLE bookings (
  id                  TEXT PRIMARY KEY,
  room_id             TEXT NOT NULL REFERENCES rooms(id),
  primary_guest_id    TEXT NOT NULL REFERENCES guests(id),
  check_in_at         TEXT NOT NULL,   -- ISO 8601
  expected_checkout   TEXT NOT NULL,
  actual_checkout     TEXT,
  nights              INTEGER NOT NULL,
  total_price         REAL NOT NULL,
  paid_amount         REAL DEFAULT 0,
  status              TEXT NOT NULL,   -- 'active'|'checked_out'|'booked'|'cancelled'
  source              TEXT DEFAULT 'walk-in', -- 'walk-in'|'agoda'|'booking.com'|'phone'
  notes               TEXT,
  created_at          TEXT NOT NULL
);

-- Khách phụ (nhiều khách 1 phòng)
CREATE TABLE booking_guests (
  booking_id  TEXT NOT NULL REFERENCES bookings(id),
  guest_id    TEXT NOT NULL REFERENCES guests(id),
  PRIMARY KEY (booking_id, guest_id)
);

-- Giao dịch thanh toán
CREATE TABLE transactions (
  id          TEXT PRIMARY KEY,
  booking_id  TEXT NOT NULL REFERENCES bookings(id),
  amount      REAL NOT NULL,
  type        TEXT NOT NULL,  -- 'payment' | 'refund'
  note        TEXT,
  created_at  TEXT NOT NULL
);

-- Chi phí vận hành
CREATE TABLE expenses (
  id          TEXT PRIMARY KEY,
  category    TEXT NOT NULL,  -- 'electricity'|'water'|'garbage'|'internet'|'elevator'|'other'
  amount      REAL NOT NULL,
  note        TEXT,
  expense_date TEXT NOT NULL, -- YYYY-MM-DD
  created_at  TEXT NOT NULL
);

-- Housekeeping
CREATE TABLE housekeeping (
  id          TEXT PRIMARY KEY,
  room_id     TEXT NOT NULL REFERENCES rooms(id),
  status      TEXT NOT NULL,  -- 'needs_cleaning'|'cleaning'|'clean'
  note        TEXT,           -- ghi chú bảo trì
  triggered_at TEXT NOT NULL, -- lúc checkout
  cleaned_at  TEXT,           -- lúc dọn xong
  created_at  TEXT NOT NULL
);
```

---

## 9. MVP Scope

### ✅ Có trong MVP
- Dashboard 10 phòng với màu trạng thái
- File watcher `~/CapyInn/Scans/`
- OCR CCCD (khách trong nước)
- Assign phòng sau OCR (flexible, nhiều khách cùng lúc)
- Check-in / Check-out flow
- Tính tiền tự động (qua đêm)
- Đánh dấu nợ
- Copy thông tin lưu trú (web Bộ Công An)
- Báo cáo doanh thu ngày/tháng
- Nhập expense thủ công
- Export CSV
- Housekeeping tracking

### ❌ Không có trong MVP (v2 sau)
- OCR Passport / khách nước ngoài
- Booking trước (đặt phòng tương lai)
- Dark mode
- OTA integration (Agoda, Booking.com)
- Thông báo sắp check-out
- Multi-language

---

## 10. Cấu trúc Project

```
mhm/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/
│   │   │   ├── rooms.rs
│   │   │   ├── guests.rs
│   │   │   ├── bookings.rs
│   │   │   ├── expenses.rs
│   │   │   └── housekeeping.rs
│   │   ├── ocr/
│   │   │   └── mod.rs
│   │   ├── watcher/
│   │   │   └── mod.rs
│   │   └── db/
│   │       ├── mod.rs
│   │       └── migrations/
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── pages/
│   │   ├── Dashboard.tsx
│   │   ├── RoomDetail.tsx
│   │   ├── Statistics.tsx
│   │   └── Housekeeping.tsx
│   ├── components/
│   │   ├── RoomCard.tsx
│   │   ├── GuestForm.tsx
│   │   ├── CheckinModal.tsx
│   │   └── OcrPopup.tsx
│   └── stores/
│       └── useHotelStore.ts
├── models/          ← OCR model files (PaddleOCR v5)
│   ├── det.mnn
│   ├── rec.mnn
│   └── charset.txt
└── package.json
```

---

## 11. GitHub Release Strategy

```
Repo name:   mini-hotel-manager
Description: Free, offline, open-source PMS for mini hotels in Vietnam 🇻🇳
             Built with Tauri 2 + Rust + React

Tags:        hotel-management, tauri, rust, vietnam, pms,
             property-management, offline-first, cccd-ocr,
             southeast-asia, open-source

README:      Song ngữ Tiếng Việt + English
Demo:        GIF 30s — từ scan CCCD đến check-in xong
Hook:        "Quản lý khách sạn mini, miễn phí, không cần internet,
              chạy trên macOS/Windows/Linux"
```

---

## 12. Môi trường phát triển

```bash
# Yêu cầu
macOS 12+
Rust (rustup)
Node.js 18+
Xcode Command Line Tools

# Cài đặt
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
xcode-select --install
npm install -g @tauri-apps/cli

# Chạy dev
npm run tauri dev

# Build
npm run tauri build
```

---

*PRD v1.0 — CapyInn*  
*Audited & locked: 14/03/2026*
