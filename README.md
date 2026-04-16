<div align="center">

# CapyInn

**Phần mềm quản lý khách sạn mini, offline-first**

*Desktop PMS miễn phí cho khách sạn nhỏ tại Việt Nam.*

[![CI](https://github.com/chuanman2707/CapyInn/actions/workflows/ci.yml/badge.svg)](https://github.com/chuanman2707/CapyInn/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri_2-FFC131?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React_19-61DAFB?style=for-the-badge&logo=react&logoColor=black)](https://react.dev)
[![SQLite](https://img.shields.io/badge/SQLite-003B57?style=for-the-badge&logo=sqlite&logoColor=white)](https://sqlite.org)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org)

**Onboarding khởi tạo khách sạn · OCR quét CCCD · Check-in/check-out · Reservations · Night audit**

[English README](README.en.md)

</div>

CapyInn là ứng dụng desktop cho mô hình khách sạn mini hoặc nhà nghỉ cần vận hành cục bộ, không phụ thuộc server riêng và không cần Internet để xử lý nghiệp vụ hằng ngày. Project tập trung vào các luồng thật sự cần ở quầy lễ tân: tạo sơ đồ phòng, nhập khách nhanh, OCR CCCD, tính tiền theo đêm, housekeeping, báo cáo doanh thu và đối soát cuối ngày.

> Ghi chú: `CapyInn` là clean-slate rename từ `MHM`. Build hiện tại dùng runtime root mới tại `~/CapyInn` và không tự động migrate dữ liệu cũ từ `~/MHM`.

<details>
<summary>Mục lục</summary>

- [CapyInn giải quyết gì](#capyinn-giải-quyết-gì)
- [Tính năng chính](#tính-năng-chính)
- [Tech stack](#tech-stack)
- [Yêu cầu hệ thống](#yêu-cầu-hệ-thống)
- [Chạy local](#chạy-local)
- [Verification](#verification)
- [Cấu trúc repository](#cấu-trúc-repository)
- [Known limitations](#known-limitations)
- [Tài liệu liên quan](#tài-liệu-liên-quan)
- [Đóng góp](#đóng-góp)
- [License](#license)

</details>

## CapyInn giải quyết gì

CapyInn sinh ra cho bối cảnh rất cụ thể: khách sạn nhỏ cần một hệ thống đủ dùng ngay, chạy local, dễ kiểm soát dữ liệu, và không bị khóa vào SaaS.

| Trước khi có app | Với CapyInn |
| --- | --- |
| Ghi sổ tay, dễ sai và khó tra cứu | Trạng thái phòng, booking và giao dịch đều nằm trong một app |
| Nhập lưu trú thủ công từ CCCD | OCR trích xuất thông tin, copy sang web lưu trú nhanh hơn |
| Tính tiền theo đêm bằng tay | Luồng check-in, extend stay, check-out và folio được tính tự động |
| Cuối ngày tổng hợp doanh thu thủ công | Dashboard, analytics, expenses và night audit có sẵn |
| Thiết lập ban đầu mất công | Onboarding sinh room types, room layout và cấu hình vận hành ngay trong app |

## Tính năng chính

### Onboarding và cấu hình ban đầu

- Khởi tạo tên khách sạn, giờ check-in/check-out, thông tin hóa đơn và app lock
- Tạo room types và giá mặc định ngay trong wizard đầu tiên
- Sinh sơ đồ phòng theo tầng, số phòng mỗi tầng và naming scheme

### Lễ tân và vận hành khách ở

- Dashboard theo sơ đồ phòng đã cấu hình
- Check-in, check-out, extend stay và reservation flow trong cùng một app
- Hỗ trợ nhiều khách trên một booking
- Copy nhanh thông tin lưu trú để nhập sang cổng khai báo

### OCR CCCD và nhập liệu nhanh

- OCR nội bộ bằng PaddleOCR v5 qua `ocr-rs`
- Theo dõi thư mục `~/CapyInn/Scans/` để nhận ảnh mới
- Trích xuất họ tên, số CCCD, ngày sinh và địa chỉ cho luồng check-in

### Doanh thu, thanh toán và báo cáo

- Tính tiền theo đêm và theo room type
- Ghi nhận charge, payment, deposit và công nợ
- Báo cáo doanh thu, chi phí, analytics và export CSV

### Housekeeping và night audit

- Theo dõi trạng thái dọn phòng sau check-out
- Ghi chú bảo trì cho từng phòng
- Night audit để đối soát giao dịch cuối ngày

## Tech stack

| Layer | Công nghệ |
| --- | --- |
| App shell | Tauri 2 |
| Backend | Rust + SQLite (`sqlx`) |
| Frontend | React 19 + TypeScript |
| State | Zustand |
| UI | Tailwind CSS 4 + shadcn/ui |
| OCR | `ocr-rs` + PaddleOCR v5 + MNN |
| Charts | Recharts |
| Tests | Vitest + Rust tests + Clippy |

## Yêu cầu hệ thống

| Thành phần | Yêu cầu |
| --- | --- |
| macOS | 12+ |
| Node.js | 20+ |
| Rust | stable qua `rustup` |
| Xcode CLT | bản mới |
| Dung lượng | khoảng 25MB, chưa tính dữ liệu phát sinh |

Hiện tại project được verify mạnh nhất trên macOS / Apple Silicon.

## Chạy local

### Cài prerequisites

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
xcode-select --install
node --version
```

### Clone và chạy app desktop

```bash
git clone https://github.com/chuanman2707/CapyInn.git
cd CapyInn/mhm
npm ci
npm run tauri dev
```

### Build release

```bash
cd mhm
npm run tauri build
```

Bundle release sẽ nằm trong `mhm/src-tauri/target/release/bundle/`.

## Verification

```bash
cd mhm
npm test
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Nếu chỉ cần chạy frontend web thay vì full desktop app:

```bash
cd mhm
npm run dev
```

## Cấu trúc repository

```text
CapyInn/
├── mhm/
│   ├── src/                # React UI, stores, pages, components
│   ├── src-tauri/          # Rust backend, IPC commands, DB, gateway, OCR
│   ├── tests/              # Vitest suites và mocked desktop flows
│   ├── public/             # Static assets
│   └── models/             # OCR models
├── docs/plans/             # Kế hoạch sản phẩm và refactor cấp cao
├── PRD.md                  # Product requirements
├── CONTRIBUTING.md
├── SECURITY.md
└── README.md
```

## Known limitations

- Luồng OCR hiện tối ưu cho CCCD Việt Nam; passport và giấy tờ quốc tế chưa hoàn chỉnh
- Windows và Linux chưa phải target chính thức
- Project đang phù hợp nhất cho quy mô mini hotel, không nhắm tới chuỗi khách sạn lớn

## Tài liệu liên quan

- [PRD](PRD.md)
- [Implementation plans](docs/plans)
- [Contributing guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

## Đóng góp

Nếu muốn đóng góp, đọc [CONTRIBUTING.md](CONTRIBUTING.md) trước khi mở pull request.

Checklist ngắn:

1. Fork repo
2. Tạo branch mới từ `main`
3. Giữ commit message theo Conventional Commits
4. Chạy lại `npm test`, `npm run build`, `cargo check`, `cargo test`, `cargo clippy`
5. Mở pull request với mô tả scope và verification

## License

CapyInn được phát hành dưới giấy phép [MIT](LICENSE).
