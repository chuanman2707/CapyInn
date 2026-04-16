# CapyInn

Free, offline-first property management software for small hotels in Vietnam.

> Note: `CapyInn` is a clean-slate rename from `MHM`. Current builds use the new runtime root at `~/CapyInn` and do not auto-migrate legacy local data from `~/MHM`.

## What It Does

- first-run onboarding for hotel setup and room generation
- room dashboard for a configurable property layout
- Vietnamese ID card OCR with local processing
- check-in and check-out flows
- pricing, folio, and debt tracking
- revenue and expense analytics
- housekeeping workflow
- reservations and night audit

## Stack

- Tauri 2
- Rust
- React 19
- TypeScript
- SQLite
- PaddleOCR via `ocr-rs`

## Repository Layout

- `mhm/` — core application
- `docs/plans/` — implementation plans and release prep notes

## Local Setup

```bash
git clone https://github.com/chuanman2707/Hotel-Manager.git
cd Hotel-Manager/mhm
npm ci
npm run tauri dev
```

## Verification

```bash
cd mhm
npm test
npm run build

cd src-tauri
cargo check
cargo test
```

## Limitations

- verified primarily on macOS / Apple Silicon
- Windows and Linux are not first-class targets yet
- OCR is currently optimized for Vietnamese national ID cards

## More Docs

- [Product requirements](PRD.md)
- [Implementation plans](docs/plans)
- [Contributing guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

## License

MIT. See [LICENSE](LICENSE).
