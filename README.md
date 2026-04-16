<div align="center">

# 🏨 CapyInn

**Phần mềm quản lý khách sạn mini, miễn phí, chạy offline hoàn toàn**

*Free, offline-first Property Management System for mini hotels in Vietnam 🇻🇳*

[![CI](https://github.com/chuanman2707/Hotel-Manager/actions/workflows/ci.yml/badge.svg)](https://github.com/chuanman2707/Hotel-Manager/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri_2.0-FFC131?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React_19-61DAFB?style=for-the-badge&logo=react&logoColor=black)](https://react.dev)
[![SQLite](https://img.shields.io/badge/SQLite-003B57?style=for-the-badge&logo=sqlite&logoColor=white)](https://sqlite.org)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org)

---

**Onboarding khởi tạo khách sạn · OCR quét CCCD tự động · Tính tiền thông minh · Báo cáo doanh thu**

[English README](README.en.md)

</div>

---

## 📋 Mục lục

- [Giới thiệu](#-giới-thiệu)
- [Tính năng](#-tính-năng)
- [Tech Stack](#-tech-stack)
- [Yêu cầu hệ thống](#-yêu-cầu-hệ-thống)
- [Cài đặt & Chạy](#-cài-đặt--chạy)
- [Cấu trúc dự án](#-cấu-trúc-dự-án)
- [Hướng dẫn sử dụng](#-hướng-dẫn-sử-dụng)
- [Database Schema](#-database-schema)
- [Known Limitations](#-known-limitations)
- [Roadmap](#-roadmap)
- [Đóng góp](#-đóng-góp)
- [License](#-license)

---

## 🎯 Giới thiệu

**CapyInn** là ứng dụng desktop quản lý khách sạn mini, được thiết kế cho các khách sạn nhỏ tại Việt Nam. Ứng dụng chạy **offline hoàn toàn**, không cần internet, thay thế quy trình quản lý thủ công bằng sổ tay và hỗ trợ **first-run onboarding** để cấu hình khách sạn, loại phòng, và sơ đồ phòng ngay lần mở đầu tiên.

> Lưu ý: `CapyInn` là clean-slate rename từ `MHM`. Build hiện tại dùng runtime root mới tại `~/CapyInn` và **không tự động migrate** dữ liệu cũ từ `~/MHM`.

### Documentation

- [PRD](PRD.md)
- [Implementation Plans](docs/plans)

### Vấn đề giải quyết

| Trước (thủ công) | Sau (CapyInn) |
|---|---|
| Ghi sổ tay, dễ sai sót | Quản lý số hóa, chính xác |
| Nhập web lưu trú thủ công | OCR quét CCCD → copy 1 click |
| Tính tiền bằng tay | Tính tiền tự động theo đêm |
| Không có báo cáo | Dashboard + thống kê realtime |
| ~5 phút / khách | **~60 giây / khách** ⚡ |

---

## ✨ Tính năng

### 🏠 Dashboard
- Bảng trạng thái phòng theo sơ đồ đã cấu hình
- Mã màu trực quan: 🟢 Trống · 🔴 Có khách · 🟡 Cần dọn · 🔵 Đặt trước
- Tổng doanh thu hôm nay, công suất phòng

### 🚀 First-Run Onboarding
- Thiết lập tên khách sạn, giờ check-in/check-out, thông tin hóa đơn
- Tạo loại phòng và giá mặc định ngay trong app
- Sinh sơ đồ phòng theo số tầng, số phòng mỗi tầng, và naming scheme
- Hỗ trợ bật hoặc bỏ qua PIN admin khi khởi tạo

### 📷 OCR — Quét CCCD tự động
- Tích hợp **PaddleOCR v5** (Metal GPU trên macOS)
- File Watcher: quét ảnh mới trong `~/CapyInn/Scans/` tự động
- Nhận diện: Họ tên, Số CCCD, Ngày sinh, Địa chỉ
- Tốc độ: **~200-300ms / ảnh** trên Apple Silicon

### 🛎️ Check-in / Check-out
- Assign khách vào phòng sau OCR
- Hỗ trợ nhiều khách cùng phòng
- Check-out tự động tính tiền
- Nút **"Copy thông tin lưu trú"** cho web Bộ Công An

### 💰 Tính tiền & Thanh toán
- Tính theo đêm (check-in 12:00 → check-out 11:00)
- Giá config theo loại phòng (Deluxe / Standard)
- Extend thêm đêm, thanh toán từng phần
- Quản lý công nợ

### 📊 Thống kê & Báo cáo
- Doanh thu theo ngày / tuần / tháng
- Nhập chi phí: điện, nước, rác, internet, bảo trì
- Lợi nhuận = Doanh thu - Chi phí
- **Export CSV** cho kế toán

### 🧹 Housekeeping
- Danh sách phòng cần dọn sau check-out
- Workflow: 🟡 Cần dọn → 🔄 Đang dọn → 🟢 Sạch
- Ghi chú bảo trì (điều hòa, bóng đèn, vòi nước...)

### 🌙 Night Audit
- Kiểm tra cuối ngày tự động
- Tổng hợp giao dịch, đối soát doanh thu

### ⚙️ Quản lý hệ thống
- Quản lý thông tin khách sạn
- Cấu hình giá phòng
- Quản lý đặt phòng (Reservations)
- Timeline trực quan

---

## 🛠️ Tech Stack

| Layer | Công nghệ | Lý do |
|---|---|---|
| **App Shell** | Tauri 2.0 | Nhẹ hơn Electron 10-20x, dùng WebView native |
| **Backend** | Rust | Performance, memory safety, tích hợp OCR native |
| **Frontend** | React 19 + TypeScript | UI hiện đại, type-safe |
| **Styling** | Tailwind CSS 4 + shadcn/ui | Component library đẹp, responsive |
| **Database** | SQLite (sqlx) | Offline-first, không cần server |
| **OCR** | ocr-rs (PaddleOCR v5 + MNN) | Pure Rust, Metal GPU, hỗ trợ tiếng Việt |
| **State** | Zustand | Lightweight, simple API |
| **Charts** | Recharts | Biểu đồ responsive |
| **Router** | React Router v7 | SPA navigation |
| **Build** | Vite 7 | Fast HMR, ESM-first |

---

## 💻 Yêu cầu hệ thống

| Yêu cầu | Phiên bản |
|---|---|
| **macOS** | 12+ (Monterey trở lên) |
| **Rust** | Stable (via rustup) |
| **Node.js** | 20+ |
| **Xcode CLT** | Latest |
| **Dung lượng** | ~25MB (bao gồm OCR models ~15MB) |

---

## 🚀 Cài đặt & Chạy

### 1. Cài đặt prerequisites

```bash
# Cài Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Cài Xcode Command Line Tools
xcode-select --install

# Kiểm tra Node.js (cần v20+)
node --version
```

### 2. Clone repository

```bash
git clone https://github.com/chuanman2707/Hotel-Manager.git
cd Hotel-Manager/mhm
```

### 3. Cài đặt dependencies

```bash
# Cài Node dependencies
npm ci
```

### 4. Chạy Development

```bash
# Chạy ứng dụng (Tauri dev mode)
npm run tauri dev
```

Ứng dụng sẽ tự động:
- Khởi động Vite dev server (frontend) tại `http://localhost:1420`
- Compile Rust backend
- Mở cửa sổ ứng dụng native

### 5. Build Production

```bash
# Build bản release
npm run tauri build
```

File `.dmg` sẽ được tạo tại `src-tauri/target/release/bundle/`.

### 6. Chạy Tests

```bash
# Chạy tất cả tests
npm test

# Chạy tests ở watch mode
npm run test:watch

# Chạy tests với coverage
npm run test:coverage
```

---

## 📁 Cấu trúc dự án

```
Hotel-Manager/
├── mhm/                          # Ứng dụng chính (Tauri + React + Rust)
│   ├── src/                      # Frontend
│   ├── src-tauri/                # Backend, DB, IPC, OCR
│   ├── tests/                    # Vitest suites
│   ├── models/                   # OCR models
│   └── docs/                     # Feature plans và architecture notes
├── docs/
│   └── plans/                    # Root-level implementation plans
├── PRD.md
├── CONTRIBUTING.md
├── SECURITY.md
└── README.md
```

---

## 📖 Hướng dẫn sử dụng

### Check-in khách

```
1. Cấu hình máy scan Canon LiDE 300 → output vào `~/CapyInn/Scans/`
2. Khách đưa CCCD → Scan
3. App tự động phát hiện file mới → OCR trích xuất thông tin
4. Popup hiện thông tin khách → Chọn phòng → Confirm
5. Bấm "Copy thông tin lưu trú" → Paste vào web Bộ Công An
```

### Check-out khách

```
1. Vào Dashboard → Click phòng cần check-out
2. Xem chi tiết: số đêm, tổng tiền, đã trả, còn nợ
3. Bấm "Check-out" → Confirm
4. Phòng tự động chuyển sang 🟡 Cần dọn
```

### Khởi tạo lần đầu

```
1. Mở app lần đầu → wizard onboarding xuất hiện
2. Nhập thông tin khách sạn, giờ check-in/check-out
3. Tạo loại phòng và giá mặc định
4. Sinh sơ đồ phòng theo số tầng / số phòng / naming scheme
5. Chọn bật hoặc bỏ qua PIN admin
6. Xác nhận để bắt đầu sử dụng hệ thống
```

---

## 🗄️ Database Schema

Ứng dụng sử dụng SQLite với các bảng chính:

| Bảng | Mô tả |
|---|---|
| `rooms` | Danh sách phòng được tạo từ onboarding, gồm loại, trạng thái, giá |
| `guests` | Thông tin khách (CCCD, passport, quốc tịch) |
| `bookings` | Đặt phòng, check-in/out, thanh toán |
| `booking_guests` | Nhiều khách trong 1 phòng |
| `transactions` | Giao dịch thanh toán / hoàn tiền |
| `expenses` | Chi phí vận hành (điện, nước, rác...) |
| `housekeeping` | Trạng thái dọn phòng + ghi chú bảo trì |

> Chi tiết schema xem tại [PRD.md](PRD.md#8-database-schema-sqlite)

---

## ⚠️ Known Limitations

- Hiện tại dự án được verify chủ yếu trên **macOS / Apple Silicon**.
- Windows và Linux chưa được test/ship như platform chính thức.
- OCR flow hiện tối ưu cho **CCCD Việt Nam**; passport/ID quốc tế chưa hoàn chỉnh.

---

## 🗺️ Roadmap

### ✅ MVP (Hiện tại)
- [x] Onboarding cấu hình khách sạn và sơ đồ phòng
- [x] Dashboard trạng thái phòng
- [x] OCR quét CCCD (khách trong nước)
- [x] File watcher tự động (`~/CapyInn/Scans/`)
- [x] Check-in / Check-out flow
- [x] Tính tiền tự động theo đêm
- [x] Copy thông tin lưu trú
- [x] Thống kê doanh thu
- [x] Nhập chi phí vận hành
- [x] Export CSV
- [x] Housekeeping tracking
- [x] Night Audit
- [x] Quản lý đặt phòng (Reservations)
- [x] i18n (Tiếng Việt / English)

### 🔮 V2 (Tương lai)
- [ ] OCR Passport (khách nước ngoài)
- [ ] OTA integration (Agoda, Booking.com)
- [ ] Tích hợp thanh toán ngân hàng (Vietcombank via Casso)
- [ ] Dark mode
- [ ] Windows & Linux support
- [ ] Thông báo sắp check-out
- [ ] Backup & Data export nâng cao

---

## 🤝 Đóng góp

Contributions are welcome! Nếu bạn muốn đóng góp, xem [CONTRIBUTING.md](CONTRIBUTING.md) trước.

Tóm tắt nhanh:

1. **Fork** repo này
2. Tạo branch: `git checkout -b feature/ten-tinh-nang`
3. Commit: `git commit -m "feat: thêm tính năng XYZ"`
4. Push: `git push origin feature/ten-tinh-nang`
5. Tạo **Pull Request**

### Conventions

- Commit messages: [Conventional Commits](https://www.conventionalcommits.org/)
- Code style: TypeScript strict mode, Rust clippy
- Tests: Vitest (frontend), mỗi tính năng cần có test

---

## 📄 License

Dự án này được phát hành dưới giấy phép [MIT](LICENSE).

---

<div align="center">

**Made with ❤️ for mini hotels in Vietnam 🇻🇳**

*Giảm thời gian xử lý từ 5 phút xuống 60 giây*

</div>
