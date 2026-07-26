# Khai báo tạm trú — Kế hoạch A: lõi (không UI)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Trích danh tính từ ảnh giấy tờ, kiểm tra, và xuất được file XML (khách nước ngoài) + XLSX (khách Việt Nam) đúng định dạng cổng Bộ Công an — nghiệm thu hoàn toàn bằng CLI và unit test, chưa cần một dòng React nào.

**Architecture:** Năm module thuần (`catalog`, `extractor`, `normalizer`, `validator`, `writer`) không import `tauri` và không import `sqlx`; một module `repo` là nơi duy nhất chạm DB. Danh mục enum và hành chính sinh từ chính template chính thức của cổng chứ không viết tay. Bảng mới hoàn toàn, không `ALTER` bảng nào đang chạy.

**Tech Stack:** Rust, `rxing 0.9.2` (QR), `umya-spreadsheet 3.0.1` (XLSX), `ocr-rs 2.1` (OCR MRZ, đã có trong repo), `sqlx` + SQLite, `chrono`, `image 0.25`.

**Spec:** `docs/superpowers/specs/2026-07-26-khai-bao-tam-tru-design.md`

## Global Constraints

- **Không `INSERT` / `UPDATE` / `DELETE` / `ALTER` chạm `guests`, `bookings`, `booking_guests`, `rooms`.** Chỉ `SELECT`. Có test đọc source bắt điều này.
- **Không lưu ảnh, không lưu payload QR/MRZ thô.** Không có cột `photo_path`, không có cột `raw_payload`. Không log payload hay đường dẫn ảnh.
- `catalog.rs`, `extractor/`, `normalizer.rs`, `validator.rs`, `writer/` **không được** `use tauri` hay `use sqlx`.
- Khóa chính bảng mới là **TEXT uuid** (`uuid::Uuid::new_v4().to_string()`), khớp quy ước app.
- Ngày lưu nội bộ luôn ISO `yyyy-MM-dd`. Chỉ writer đổi sang `dd/MM/yyyy`.
- Enum ghi ra file là **nguyên chuỗi `display`** từ catalog (`1 - Thẻ CCCD`), không bao giờ tự ghép lại từ code + tên.
- Test đặt nội tuyến `#[cfg(test)] mod tests` trong chính file được test — theo đúng quy ước repo.
- Chạy test: `cd mhm/src-tauri && cargo test <tên_test>`.
- Mọi commit kết thúc bằng dòng `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

## Sự thật đã đo, không được thiết kế trái

- Cổng **bỏ qua row 4 của XLSX theo vị trí**, đọc từ row 5. Row 4 phải giữ nguyên `[EXAMPLE]`.
- Cổng **báo "import thành công" khi import 0 record**. Không có feedback nào khác.
- MRZ dòng 1 (họ tên) **không có checksum nào bảo vệ**. 5 check digit nằm hết ở dòng 2.
- MRZ chỉ có 2 chữ số năm; luật suy thế kỷ cho **ngày sinh** và **ngày hết hạn** phải là hai hàm khác nhau.
- `guests.guest_type` không tin được — 9/9 khách trong DB thật là `domestic` kể cả người nước ngoài.
- `bookings.check_in_at` là timestamp có offset; `expected_checkout` là date trần.
- `rooms.name` có tiền tố `"Phòng "`; cổng nhận `5A`.
- umya-spreadsheet 3.0.1 round-trip giữ nguyên 40 definedName / 9 dataValidation / 4 sheet (đã đo).
- umya `defined_names()` trả 0 dù file có 40 → gate phải đọc file như zip, đếm trong `xl/workbook.xml`.
- umya đổi **thứ tự thuộc tính** XML của row 4 (`s="10" t="s"` → `t="s" s="10"`) nhưng không đổi giá trị → gate so **giá trị**, không so byte.

---

## File Structure

| File | Trách nhiệm |
|---|---|
| `scripts/gen_kbtt_catalog.py` | Sinh `kbtt_catalog.json` từ template chính thức |
| `mhm/src-tauri/resources/tblt_vn_import.xlsx` | Template chính thức của cổng |
| `mhm/src-tauri/resources/kbtt_catalog.json` | Danh mục enum + hành chính đã sinh |
| `mhm/src-tauri/src/declaration/mod.rs` | Re-export + `find_kbtt_resource` |
| `mhm/src-tauri/src/declaration/catalog.rs` | Nạp và tra cứu danh mục |
| `mhm/src-tauri/src/declaration/model.rs` | Struct dùng chung: `Identity`, `StayInfo`, `DeclarationRow` |
| `mhm/src-tauri/src/declaration/normalizer.rs` | Ngày, giới tính, suy thế kỷ, cắt tên phòng |
| `mhm/src-tauri/src/declaration/extractor/mod.rs` | Trait `IdentityExtractor`, thứ tự thử |
| `mhm/src-tauri/src/declaration/extractor/qr_cccd.rs` | rxing |
| `mhm/src-tauri/src/declaration/extractor/mrz.rs` | Trait `MrzOcr`, checksum TD3, scoring |
| `mhm/src-tauri/src/declaration/validator.rs` | E01–E14, W01–W06 |
| `mhm/src-tauri/src/declaration/writer/xml.rs` | XmlWriter (NNN) |
| `mhm/src-tauri/src/declaration/writer/xlsx.rs` | XlsxWriter (VN) + gate 7 assert |
| `mhm/src-tauri/src/declaration/repo.rs` | 4 bảng mới + đọc bảng cũ (chỉ SELECT) |
| `mhm/src-tauri/src/db/declaration.rs` | `migrate_v20_declaration_tables` |
| `mhm/src-tauri/src/bin/kbtt_probe.rs` | CLI harness đo extractor trên ảnh thật |

---

## Task 1: Sinh danh mục từ template

**Files:**
- Create: `scripts/gen_kbtt_catalog.py`
- Create: `mhm/src-tauri/resources/tblt_vn_import.xlsx` (copy từ `~/Downloads/tblt_vn_import.xlsx`)
- Create: `mhm/src-tauri/resources/kbtt_catalog.json` (sinh ra)
- Modify: `mhm/src-tauri/tauri.conf.json` (thêm `bundle.resources`)

**Interfaces:**
- Produces: file `resources/kbtt_catalog.json` với các khóa `_source_sha256`, `_source_date`, `loai_giay_to`, `noi_cu_tru`, `ly_do_cu_tru`, `gioi_tinh`, `quoc_tich`, `tinh_thanh`, `phuong_xa`. Mỗi mục là `{"code": str, "display": str}`; riêng `phuong_xa` có thêm `"tinh": str`.

- [ ] **Step 1: Copy template vào resources**

```bash
mkdir -p mhm/src-tauri/resources
cp ~/Downloads/tblt_vn_import.xlsx mhm/src-tauri/resources/
```

- [ ] **Step 2: Viết script sinh catalog**

Tạo `scripts/gen_kbtt_catalog.py`. Script đọc xlsx bằng `zipfile` + `re` (không cần dependency ngoài), tách `code` từ `display` ở `" - "` đầu tiên, và **assert chéo** số lượng.

```python
#!/usr/bin/env python3
"""Sinh kbtt_catalog.json từ template chính thức của cổng Bộ Công an.

Chạy lại script này mỗi khi cổng phát hành template mới. git diff sẽ chỉ ra
đúng cái gì đổi — đó là cách biến schema drift thành một thay đổi nhìn thấy
được thay vì một khai báo sai im lặng.
"""
import hashlib
import json
import re
import sys
import zipfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "mhm/src-tauri/resources/tblt_vn_import.xlsx"
OUT = ROOT / "mhm/src-tauri/resources/kbtt_catalog.json"

# (khóa json, sheet xml, cột, số dòng bắt buộc)
ENUMS = [
    ("loai_giay_to", "sheet4", "A", 9),
    ("noi_cu_tru", "sheet4", "B", 3),
    ("ly_do_cu_tru", "sheet4", "C", 20),
    ("gioi_tinh", "sheet4", "D", 2),
    ("quoc_tich", "sheet4", "E", 205),
    ("tinh_thanh", "sheet2", "C", 34),
]


def shared_strings(z):
    xml = z.read("xl/sharedStrings.xml").decode("utf-8")
    out = []
    for si in re.findall(r"<si>(.*?)</si>", xml, re.S):
        out.append("".join(re.findall(r"<t[^>]*>(.*?)</t>", si, re.S)))
    return out


def cells(z, sheet, strs):
    xml = z.read(f"xl/worksheets/{sheet}.xml").decode("utf-8")
    out = {}
    for m in re.finditer(r'<c r="([A-Z]+\d+)"([^>]*)>(.*?)</c>', xml, re.S):
        ref, attr, inner = m.groups()
        v = re.search(r"<v>(.*?)</v>", inner, re.S)
        if not v:
            continue
        val = v.group(1)
        if 't="s"' in attr:
            val = strs[int(val)]
        out[ref] = val
    return out


def split_code(display):
    """`511 - Khánh Hòa` -> (`511`, `511 - Khánh Hòa`). Cắt ở ' - ' ĐẦU TIÊN."""
    if " - " not in display:
        raise SystemExit(f"Mục danh mục không đúng dạng 'mã - nhãn': {display!r}")
    return display.split(" - ", 1)[0].strip()


def main():
    if not TEMPLATE.exists():
        raise SystemExit(f"Không thấy template: {TEMPLATE}")

    raw = TEMPLATE.read_bytes()
    z = zipfile.ZipFile(TEMPLATE)
    strs = shared_strings(z)

    catalog = {
        "_source_file": TEMPLATE.name,
        "_source_sha256": hashlib.sha256(raw).hexdigest(),
        "_source_date": date.today().isoformat(),
    }

    for key, sheet, col, want in ENUMS:
        data = cells(z, sheet, strs)
        items = []
        row = 2
        while True:
            val = data.get(f"{col}{row}")
            if not val:
                break
            items.append({"code": split_code(val), "display": val})
            row += 1
        if len(items) != want:
            raise SystemExit(
                f"{key}: có {len(items)} mục, cần {want}. "
                "Template đã đổi — xem §13.6 của spec trước khi sửa con số này."
            )
        catalog[key] = items

    # Phường/xã: DISPLAY ở cột D, mã tỉnh ở cột C
    px = cells(z, "sheet3", strs)
    wards = []
    row = 2
    while True:
        display = px.get(f"D{row}")
        if not display:
            break
        wards.append(
            {
                "code": split_code(display),
                "display": display,
                "tinh": px.get(f"C{row}", ""),
            }
        )
        row += 1
    if len(wards) != 3323:
        raise SystemExit(
            f"phuong_xa: có {len(wards)} mục, cần 3323. Template đã đổi."
        )
    catalog["phuong_xa"] = wards

    # Assert chéo: mỗi named range PX_<mã> phải khớp số phường có tinh = <mã>
    wb = z.read("xl/workbook.xml").decode("utf-8")
    ranges = re.findall(r'<definedName name="PX_(\d+)">PHUONG_XA!\$D\$(\d+):\$D\$(\d+)</definedName>', wb)
    if not ranges:
        raise SystemExit("Không thấy named range PX_* nào — template đã đổi cấu trúc.")
    for ma, lo, hi in ranges:
        want_n = int(hi) - int(lo) + 1
        got_n = sum(1 for w in wards if w["tinh"] == ma)
        if want_n != got_n:
            raise SystemExit(
                f"PX_{ma}: named range có {want_n} dòng nhưng cột MATT đếm được "
                f"{got_n}. Danh mục không nhất quán, dừng."
            )

    tinh_codes = {t["code"] for t in catalog["tinh_thanh"]}
    orphan = sorted({w["tinh"] for w in wards} - tinh_codes)
    if orphan:
        raise SystemExit(f"Phường thuộc tỉnh không có trong TINH_THANH: {orphan}")

    OUT.write_text(json.dumps(catalog, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Đã ghi {OUT}")
    for key, _, _, want in ENUMS:
        print(f"  {key}: {want}")
    print(f"  phuong_xa: {len(wards)}")
    print(f"  {len(ranges)} named range PX_* khớp cột MATT")


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 3: Chạy script, xác nhận số lượng**

Run: `python3 scripts/gen_kbtt_catalog.py`

Expected output chứa đúng các dòng:
```
  loai_giay_to: 9
  noi_cu_tru: 3
  ly_do_cu_tru: 20
  gioi_tinh: 2
  quoc_tich: 205
  tinh_thanh: 34
  phuong_xa: 3323
  34 named range PX_* khớp cột MATT
```

Nếu bất kỳ con số nào lệch → **dừng lại, không sửa con số trong script**. Con số lệch nghĩa là template đã đổi, và đó là tín hiệu phải đọc lại §13.6 của spec.

- [ ] **Step 4: Khai báo resources cho bundle**

Sửa `mhm/src-tauri/tauri.conf.json`, thêm khóa `resources` vào trong `bundle`:

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "resources": ["resources/*"],
    "icon": [
```

- [ ] **Step 5: Commit**

```bash
git add scripts/gen_kbtt_catalog.py mhm/src-tauri/resources mhm/src-tauri/tauri.conf.json
git commit -m "feat(kbtt): generate declaration catalog from official template

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 2: Nạp catalog trong Rust

**Files:**
- Create: `mhm/src-tauri/src/declaration/mod.rs`
- Create: `mhm/src-tauri/src/declaration/catalog.rs`
- Modify: `mhm/src-tauri/src/lib.rs` (thêm `mod declaration;`)

**Interfaces:**
- Consumes: `resources/kbtt_catalog.json` từ Task 1
- Produces:
  - `declaration::find_kbtt_resource(name: &str) -> Result<PathBuf, String>`
  - `declaration::catalog::Catalog` với `load_from(path: &Path) -> Result<Catalog, String>`, `load() -> Result<Catalog, String>`
  - `Catalog::display_for(&self, list: CatalogList, code: &str) -> Option<&str>`
  - `Catalog::has_code(&self, list: CatalogList, code: &str) -> bool`
  - `enum CatalogList { LoaiGiayTo, NoiCuTru, LyDoCuTru, GioiTinh, QuocTich, TinhThanh, PhuongXa }`

- [ ] **Step 1: Viết test thất bại**

Tạo `mhm/src-tauri/src/declaration/catalog.rs` với phần test trước:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        Catalog::load().expect("catalog phải nạp được")
    }

    #[test]
    fn catalog_has_expected_counts() {
        let c = cat();
        assert_eq!(c.loai_giay_to.len(), 9);
        assert_eq!(c.noi_cu_tru.len(), 3);
        assert_eq!(c.ly_do_cu_tru.len(), 20);
        assert_eq!(c.gioi_tinh.len(), 2);
        assert_eq!(c.quoc_tich.len(), 205);
        assert_eq!(c.tinh_thanh.len(), 34);
        assert_eq!(c.phuong_xa.len(), 3323);
    }

    #[test]
    fn display_is_verbatim_from_template() {
        let c = cat();
        // F5: ghi nguyên chuỗi, không tự ghép lại
        assert_eq!(
            c.display_for(CatalogList::LoaiGiayTo, "1"),
            Some("1 - Thẻ CCCD")
        );
        assert_eq!(
            c.display_for(CatalogList::LoaiGiayTo, "8"),
            Some("8 - Thẻ Căn Cước")
        );
        assert_eq!(c.display_for(CatalogList::GioiTinh, "F"), Some("F - Nữ"));
        assert_eq!(c.display_for(CatalogList::LyDoCuTru, "1"), Some("1 - Du lịch"));
        assert_eq!(
            c.display_for(CatalogList::LyDoCuTru, "20"),
            Some("20 - Mục đích khác")
        );
    }

    #[test]
    fn nationality_lookup_covers_e05() {
        let c = cat();
        assert!(c.has_code(CatalogList::QuocTich, "VNM"));
        assert!(c.has_code(CatalogList::QuocTich, "RUS"));
        assert!(!c.has_code(CatalogList::QuocTich, "XXX"));
    }

    #[test]
    fn source_provenance_recorded() {
        let c = cat();
        assert_eq!(c.source_sha256.len(), 64);
        assert!(!c.source_date.is_empty());
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::catalog`
Expected: FAIL — chưa có `Catalog`, lỗi compile.

- [ ] **Step 3: Viết implementation tối thiểu**

Phần trên của `catalog.rs`:

```rust
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogItem {
    pub code: String,
    pub display: String,
    #[serde(default)]
    pub tinh: String,
}

#[derive(Debug, Clone, Copy)]
pub enum CatalogList {
    LoaiGiayTo,
    NoiCuTru,
    LyDoCuTru,
    GioiTinh,
    QuocTich,
    TinhThanh,
    PhuongXa,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[serde(rename = "_source_sha256")]
    pub source_sha256: String,
    #[serde(rename = "_source_date")]
    pub source_date: String,
    pub loai_giay_to: Vec<CatalogItem>,
    pub noi_cu_tru: Vec<CatalogItem>,
    pub ly_do_cu_tru: Vec<CatalogItem>,
    pub gioi_tinh: Vec<CatalogItem>,
    pub quoc_tich: Vec<CatalogItem>,
    pub tinh_thanh: Vec<CatalogItem>,
    pub phuong_xa: Vec<CatalogItem>,
}

impl Catalog {
    pub fn load() -> Result<Catalog, String> {
        let path = super::find_kbtt_resource("kbtt_catalog.json")?;
        Catalog::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Catalog, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Không đọc được {}: {e}", path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("kbtt_catalog.json hỏng: {e}"))
    }

    fn list(&self, which: CatalogList) -> &[CatalogItem] {
        match which {
            CatalogList::LoaiGiayTo => &self.loai_giay_to,
            CatalogList::NoiCuTru => &self.noi_cu_tru,
            CatalogList::LyDoCuTru => &self.ly_do_cu_tru,
            CatalogList::GioiTinh => &self.gioi_tinh,
            CatalogList::QuocTich => &self.quoc_tich,
            CatalogList::TinhThanh => &self.tinh_thanh,
            CatalogList::PhuongXa => &self.phuong_xa,
        }
    }

    pub fn display_for(&self, which: CatalogList, code: &str) -> Option<&str> {
        self.list(which)
            .iter()
            .find(|i| i.code == code)
            .map(|i| i.display.as_str())
    }

    pub fn has_code(&self, which: CatalogList, code: &str) -> bool {
        self.list(which).iter().any(|i| i.code == code)
    }
}
```

Và `mhm/src-tauri/src/declaration/mod.rs`:

```rust
pub mod catalog;

use std::path::PathBuf;

/// Tìm file resource của module khai báo.
///
/// `bin/kbtt_probe` và unit test chạy ngoài Tauri nên không có app handle để
/// gọi `BaseDirectory::Resource`. Thử lần lượt như `ocr::find_models_dir` đã làm.
pub fn find_kbtt_resource(name: &str) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(root) = crate::app_identity::runtime_root_opt() {
        candidates.push(root.join("resources").join(name));
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    candidates.push(cwd.join("resources").join(name));
    candidates.push(cwd.join("..").join("resources").join(name));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(name),
    );

    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| {
            format!(
                "Không tìm thấy resource {name}. Đã thử: {}",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}
```

Thêm vào `mhm/src-tauri/src/lib.rs`, cạnh các `mod` khác:

```rust
mod declaration;
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::catalog`
Expected: PASS, 4 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src-tauri/src/declaration mhm/src-tauri/src/lib.rs
git commit -m "feat(kbtt): load declaration catalog with provenance

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 3: Normalizer — ngày, thế kỷ, giới tính, tên phòng

**Files:**
- Create: `mhm/src-tauri/src/declaration/normalizer.rs`
- Modify: `mhm/src-tauri/src/declaration/mod.rs` (thêm `pub mod normalizer;`)

**Interfaces:**
- Produces:
  - `century_dob(yy: u32, today_year: u32) -> u32`
  - `century_expiry(yy: u32, today_year: u32) -> u32`
  - `mrz_date_to_iso(yymmdd: &str, kind: DateKind, today_year: u32) -> Option<String>`
  - `enum DateKind { Birth, Expiry }`
  - `qr_date_to_iso(ddmmyyyy: &str) -> Option<String>` — nhận `28071995`
  - `iso_to_portal(iso: &str) -> Option<String>` — `1995-07-28` → `28/07/1995`
  - `gender_from_qr(s: &str) -> Option<&'static str>` — `Nữ`→`F`
  - `gender_from_mrz(c: char) -> Option<&'static str>`
  - `strip_room_prefix(name: &str) -> String` — `Phòng 5B` → `5B`
  - `booking_ts_to_iso_date(raw: &str) -> Option<String>` — chịu được cả timestamp có offset lẫn date trần

- [ ] **Step 1: Viết test thất bại**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // G6 — đây là bug thật đã xảy ra khi soạn spec. Hai hàm PHẢI khác nhau.
    #[test]
    fn century_rules_differ_for_dob_and_expiry() {
        assert_eq!(century_dob(90, 2026), 1990);
        assert_eq!(century_expiry(35, 2026), 2035);
        // cùng đầu vào, khác kết quả — nếu ai gộp hai hàm làm một, test này đỏ
        assert_ne!(century_dob(35, 2026), century_expiry(35, 2026));
        assert_eq!(century_dob(35, 2026), 1935);
    }

    #[test]
    fn mrz_dates_resolve_correctly() {
        assert_eq!(
            mrz_date_to_iso("900308", DateKind::Birth, 2026).as_deref(),
            Some("1990-03-08")
        );
        assert_eq!(
            mrz_date_to_iso("351209", DateKind::Expiry, 2026).as_deref(),
            Some("2035-12-09")
        );
        assert_eq!(mrz_date_to_iso("9003", DateKind::Birth, 2026), None);
        assert_eq!(mrz_date_to_iso("901332", DateKind::Birth, 2026), None);
    }

    #[test]
    fn qr_and_portal_date_formats() {
        assert_eq!(qr_date_to_iso("28071995").as_deref(), Some("1995-07-28"));
        assert_eq!(qr_date_to_iso("2807199").as_deref(), None);
        assert_eq!(iso_to_portal("1995-07-28").as_deref(), Some("28/07/1995"));
        assert_eq!(iso_to_portal("18/10/1974"), None);
    }

    // G9 — hai nguồn trả giới tính khác format, không map trực tiếp cho nhau
    #[test]
    fn gender_normalises_from_both_sources() {
        assert_eq!(gender_from_qr("Nữ"), Some("F"));
        assert_eq!(gender_from_qr("Nam"), Some("M"));
        assert_eq!(gender_from_qr("khác"), None);
        assert_eq!(gender_from_mrz('F'), Some("F"));
        assert_eq!(gender_from_mrz('M'), Some("M"));
        assert_eq!(gender_from_mrz('<'), None);
    }

    #[test]
    fn room_prefix_stripped_for_portal() {
        assert_eq!(strip_room_prefix("Phòng 5B"), "5B");
        assert_eq!(strip_room_prefix("phòng 102A9"), "102A9");
        assert_eq!(strip_room_prefix("5A"), "5A");
        assert_eq!(strip_room_prefix("  Phòng  3A "), "3A");
    }

    // H3 — bookings có hai định dạng ngày khác nhau trong cùng một bảng
    #[test]
    fn booking_timestamps_accept_both_shapes() {
        assert_eq!(
            booking_ts_to_iso_date("2026-07-25T17:45:27.232667+07:00").as_deref(),
            Some("2026-07-25")
        );
        assert_eq!(
            booking_ts_to_iso_date("2026-07-26").as_deref(),
            Some("2026-07-26")
        );
        assert_eq!(booking_ts_to_iso_date("").as_deref(), None);
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::normalizer`
Expected: FAIL — lỗi compile, chưa có hàm nào.

- [ ] **Step 3: Viết implementation**

```rust
use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    Birth,
    Expiry,
}

/// Ngày sinh: luôn trong quá khứ, cửa sổ 100 năm.
pub fn century_dob(yy: u32, today_year: u32) -> u32 {
    if 2000 + yy <= today_year {
        2000 + yy
    } else {
        1900 + yy
    }
}

/// Ngày hết hạn: gần như luôn ở tương lai, cửa sổ [nay-10, nay+90].
///
/// PHẢI tách khỏi `century_dob`. Dùng chung một hàm sẽ biến `351209`
/// (hết hạn 09/12/2035) thành 09/12/1935 — bug thật đã xảy ra.
pub fn century_expiry(yy: u32, today_year: u32) -> u32 {
    if 2000 + yy >= today_year.saturating_sub(10) {
        2000 + yy
    } else {
        1900 + yy
    }
}

pub fn mrz_date_to_iso(yymmdd: &str, kind: DateKind, today_year: u32) -> Option<String> {
    if yymmdd.len() != 6 || !yymmdd.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let yy: u32 = yymmdd[0..2].parse().ok()?;
    let mm: u32 = yymmdd[2..4].parse().ok()?;
    let dd: u32 = yymmdd[4..6].parse().ok()?;
    let year = match kind {
        DateKind::Birth => century_dob(yy, today_year),
        DateKind::Expiry => century_expiry(yy, today_year),
    };
    NaiveDate::from_ymd_opt(year as i32, mm, dd).map(|d| d.format("%Y-%m-%d").to_string())
}

/// QR CCCD trả `ddMMyyyy` không dấu phân cách.
pub fn qr_date_to_iso(ddmmyyyy: &str) -> Option<String> {
    if ddmmyyyy.len() != 8 || !ddmmyyyy.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let dd: u32 = ddmmyyyy[0..2].parse().ok()?;
    let mm: u32 = ddmmyyyy[2..4].parse().ok()?;
    let yyyy: i32 = ddmmyyyy[4..8].parse().ok()?;
    NaiveDate::from_ymd_opt(yyyy, mm, dd).map(|d| d.format("%Y-%m-%d").to_string())
}

/// ISO nội bộ -> `dd/MM/yyyy` mà cổng cần. Chỉ writer gọi hàm này.
pub fn iso_to_portal(iso: &str) -> Option<String> {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%d/%m/%Y").to_string())
}

pub fn gender_from_qr(s: &str) -> Option<&'static str> {
    match s.trim() {
        "Nữ" | "nữ" => Some("F"),
        "Nam" | "nam" => Some("M"),
        _ => None,
    }
}

pub fn gender_from_mrz(c: char) -> Option<&'static str> {
    match c {
        'F' => Some("F"),
        'M' => Some("M"),
        _ => None,
    }
}

/// `rooms.name` là `Phòng 5B`; cổng nhận `5B`.
pub fn strip_room_prefix(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    for prefix in ["phòng ", "phong "] {
        if lower.starts_with(prefix) {
            return trimmed[prefix.len()..].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// `bookings.check_in_at` là timestamp có offset; `expected_checkout` là date
/// trần. Cùng một bảng, hai định dạng — hàm này chịu cả hai.
pub fn booking_ts_to_iso_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some((date_part, _)) = raw.split_once('T') {
        return NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            .ok()
            .map(|d| d.format("%Y-%m-%d").to_string());
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}
```

Thêm `pub mod normalizer;` vào `declaration/mod.rs`.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::normalizer`
Expected: PASS, 6 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): normalise dates, gender and room names

Two separate century rules for birth vs expiry dates. Sharing one turns a
2035 passport expiry into 1935.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 4: Model dùng chung

**Files:**
- Create: `mhm/src-tauri/src/declaration/model.rs`
- Modify: `mhm/src-tauri/src/declaration/mod.rs`

**Interfaces:**
- Produces: các struct/enum mà Task 5–9 đều dùng.

- [ ] **Step 1: Viết model**

Không có test riêng — đây là kiểu dữ liệu thuần, được Task 5–9 kiểm gián tiếp.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    QrCccd,
    MrzTd3,
    Manual,
}

impl Source {
    pub fn as_db(&self) -> &'static str {
        match self {
            Source::QrCccd => "qr_cccd",
            Source::MrzTd3 => "mrz_td3",
            Source::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Verified,
    NeedsReview,
}

impl Confidence {
    pub fn as_db(&self) -> &'static str {
        match self {
            Confidence::Verified => "verified",
            Confidence::NeedsReview => "needs_review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    FullName,
    Dob,
    Gender,
    Nationality,
    DocType,
    DocNo,
    PassportNo,
    PassportExpiry,
    VisaValidUntil,
    AddressDetail,
    Phone,
}

/// Danh tính trích từ ảnh. KHÔNG chứa ảnh, KHÔNG chứa payload thô.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub full_name: String,
    pub dob: String,           // ISO yyyy-MM-dd
    pub gender: String,        // "M" | "F"
    pub nationality_iso3: String,
    pub doc_type_code: Option<String>,
    pub doc_type_source: Option<String>, // "heuristic" | "human"
    pub doc_type_name: Option<String>,
    pub doc_no: Option<String>,
    pub phone: Option<String>,
    pub residence_status: Option<String>,
    pub address_detail: Option<String>,
    pub passport_no: Option<String>,
    pub passport_expiry: Option<String>,
    pub visa_valid_until: Option<String>,
    pub name_confirmed_by_human: bool,
    pub single_token_name_ok: bool,
}

impl Identity {
    /// Phân loại NNN/VN. KHÔNG dùng `guests.guest_type` — cột đó nói dối.
    pub fn is_vietnamese(&self) -> bool {
        self.nationality_iso3 == "VNM"
    }
}

/// Thông tin lượt lưu trú, đọc từ `bookings` / `rooms`. Chỉ đọc.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StayInfo {
    pub stay_id: String,     // = bookings.id
    pub room_no: String,     // đã cắt tiền tố "Phòng "
    pub check_in: String,    // ISO yyyy-MM-dd
    pub expected_out: String, // ISO yyyy-MM-dd
    pub actual_out: Option<String>,
    pub check_in_raw: String, // timestamp gốc, dùng cho W04
}

/// Một dòng sẵn sàng validate và ghi ra file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarationRow {
    pub link_id: String,
    pub identity: Identity,
    pub stay: StayInfo,
    pub stay_reason: String,
    pub stay_reason_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub link_id: String,
    pub field: Option<Field>,
    pub message: String,
}

impl Finding {
    pub fn blocking(code: &str, link_id: &str, field: Option<Field>, message: &str) -> Self {
        Finding {
            code: code.to_string(),
            severity: Severity::Blocking,
            link_id: link_id.to_string(),
            field,
            message: message.to_string(),
        }
    }

    pub fn warning(code: &str, link_id: &str, field: Option<Field>, message: &str) -> Self {
        Finding {
            code: code.to_string(),
            severity: Severity::Warning,
            link_id: link_id.to_string(),
            field,
            message: message.to_string(),
        }
    }
}

/// Kết quả trích xuất. `crop_for_review` chỉ sống trong RAM, không ghi đĩa.
pub struct ExtractResult {
    pub source: Source,
    pub confidence: Confidence,
    pub identity: Identity,
    pub review_hints: Vec<Field>,
    pub crop_for_review: Option<image::DynamicImage>,
}
```

Thêm `pub mod model;` vào `declaration/mod.rs`.

- [ ] **Step 2: Xác nhận biên dịch**

Run: `cd mhm/src-tauri && cargo build`
Expected: thành công, không lỗi.

- [ ] **Step 3: Commit**

```bash
git add mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): add shared declaration model types

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 5: QrCccdExtractor

**Files:**
- Create: `mhm/src-tauri/src/declaration/extractor/mod.rs`
- Create: `mhm/src-tauri/src/declaration/extractor/qr_cccd.rs`
- Modify: `mhm/src-tauri/src/declaration/mod.rs`
- Modify: `mhm/src-tauri/Cargo.toml` (thêm `rxing = "0.9.2"`)

**Interfaces:**
- Consumes: `model::{ExtractResult, Identity, Source, Confidence, Field}`, `normalizer::{qr_date_to_iso, gender_from_qr}`
- Produces:
  - `trait IdentityExtractor { fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult>; }`
  - `struct QrCccdExtractor;` implement trait đó
  - `parse_cccd_payload(payload: &str) -> Option<Identity>` — public để test không cần ảnh
  - `infer_doc_type(issue_date_iso: &str) -> &'static str` — trả `"1"` hoặc `"8"`

- [ ] **Step 1: Viết test thất bại**

Trong `qr_cccd.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = "058195006173|264445725|Phan Thị Mỹ Hà|28071995|Nữ|KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận|27062021";

    #[test]
    fn parses_all_seven_fields() {
        let id = parse_cccd_payload(PAYLOAD).expect("payload hợp lệ");
        assert_eq!(id.doc_no.as_deref(), Some("058195006173"));
        assert_eq!(id.full_name, "Phan Thị Mỹ Hà"); // giữ nguyên dấu
        assert_eq!(id.dob, "1995-07-28");
        assert_eq!(id.gender, "F"); // QR trả "Nữ", không phải "F"
        assert_eq!(id.nationality_iso3, "VNM");
        assert_eq!(
            id.address_detail.as_deref(),
            Some("KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận")
        );
    }

    // G7 — địa chỉ trong QR đã lạc hậu so với danh mục. Không được parse.
    #[test]
    fn address_is_kept_raw_never_parsed() {
        let id = parse_cccd_payload(PAYLOAD).unwrap();
        let addr = id.address_detail.unwrap();
        assert!(addr.contains("Ninh Thuận")); // tỉnh này không còn tồn tại
        assert!(addr.contains("Mỹ Đông")); // phường này không còn trong 3323
    }

    // Payload lệch nghĩa là đang đoán, mà đoán ở đây là khai sai.
    #[test]
    fn wrong_field_count_is_a_failure_not_a_guess() {
        assert!(parse_cccd_payload("a|b|c|d|e|f").is_none()); // 6
        assert!(parse_cccd_payload("a|b|c|d|e|f|g|h").is_none()); // 8
        assert!(parse_cccd_payload("").is_none());
    }

    #[test]
    fn malformed_date_rejects_payload() {
        let bad = "058195006173|264445725|Nguyễn Văn A|99999999|Nam|Hà Nội|27062021";
        assert!(parse_cccd_payload(bad).is_none());
    }

    // G8 — QR không phân biệt CCCD với Căn Cước; suy từ ngày cấp.
    #[test]
    fn doc_type_inferred_from_issue_date() {
        assert_eq!(infer_doc_type("2021-06-27"), "1"); // Thẻ CCCD
        assert_eq!(infer_doc_type("2024-06-30"), "1");
        assert_eq!(infer_doc_type("2024-07-01"), "8"); // Thẻ Căn Cước
        assert_eq!(infer_doc_type("2025-01-15"), "8");
    }

    #[test]
    fn doc_type_is_marked_as_a_guess() {
        let id = parse_cccd_payload(PAYLOAD).unwrap();
        assert_eq!(id.doc_type_code.as_deref(), Some("1"));
        // W06 dựa vào đúng cột này
        assert_eq!(id.doc_type_source.as_deref(), Some("heuristic"));
    }

    #[test]
    fn empty_old_id_number_is_allowed() {
        let p = "058195006173||Phan Thị Mỹ Hà|28071995|Nữ|Nha Trang|27062021";
        assert!(parse_cccd_payload(p).is_some());
    }

    #[test]
    fn decodes_a_real_qr_image() {
        use rxing::Writer;
        let writer = rxing::MultiFormatWriter;
        let matrix = writer
            .encode(PAYLOAD, &rxing::BarcodeFormat::QR_CODE, 600, 600)
            .unwrap();
        let img: image::DynamicImage = (&matrix).into();

        let res = QrCccdExtractor
            .try_extract(&img)
            .expect("phải decode được QR");
        assert_eq!(res.source, Source::QrCccd);
        assert_eq!(res.confidence, Confidence::Verified);
        assert_eq!(res.identity.full_name, "Phan Thị Mỹ Hà");
        assert!(res.review_hints.contains(&Field::DocType)); // heuristic
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::extractor::qr_cccd`
Expected: FAIL — lỗi compile.

- [ ] **Step 3: Thêm dependency**

```bash
cd mhm/src-tauri && cargo add rxing@0.9.2
```

- [ ] **Step 4: Viết trait và implementation**

`extractor/mod.rs`:

```rust
pub mod qr_cccd;

use crate::declaration::model::ExtractResult;

pub trait IdentityExtractor {
    fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult>;
}
```

`extractor/qr_cccd.rs` (phần trên):

```rust
use super::IdentityExtractor;
use crate::declaration::model::{Confidence, ExtractResult, Field, Identity, Source};
use crate::declaration::normalizer::{gender_from_qr, qr_date_to_iso};

pub struct QrCccdExtractor;

/// Ngày cấp từ mốc này trở đi là Thẻ Căn Cước, trước đó là Thẻ CCCD.
const CAN_CUOC_FROM: &str = "2024-07-01";

pub fn infer_doc_type(issue_date_iso: &str) -> &'static str {
    if issue_date_iso >= CAN_CUOC_FROM {
        "8"
    } else {
        "1"
    }
}

/// Payload QR thẻ CCCD: 7 trường phân cách `|`.
///
/// Khác 7 trường => fail. Không cố cứu payload lệch: payload lệch nghĩa là
/// đang đoán, và đoán ở đây là khai sai với cơ quan công an.
pub fn parse_cccd_payload(payload: &str) -> Option<Identity> {
    let parts: Vec<&str> = payload.split('|').collect();
    if parts.len() != 7 {
        return None;
    }

    let doc_no = parts[0].trim();
    let full_name = parts[2].trim();
    let dob = qr_date_to_iso(parts[3].trim())?;
    let gender = gender_from_qr(parts[4])?;
    let address = parts[5].trim();
    let issue_date = qr_date_to_iso(parts[6].trim())?;

    if doc_no.is_empty() || full_name.is_empty() {
        return None;
    }

    Some(Identity {
        id: uuid::Uuid::new_v4().to_string(),
        full_name: full_name.to_string(),
        dob,
        gender: gender.to_string(),
        nationality_iso3: "VNM".to_string(), // CCCD chỉ cấp cho công dân VN
        doc_type_code: Some(infer_doc_type(&issue_date).to_string()),
        doc_type_source: Some("heuristic".to_string()),
        doc_no: Some(doc_no.to_string()),
        // G7: chuỗi thô, KHÔNG parse thành tỉnh/phường
        address_detail: Some(address.to_string()),
        ..Default::default()
    })
}

/// Thang biến thể tiền xử lý. G2/G3: ảnh THÔ trước, xử lý mạnh sau.
fn decode_qr(image: &image::DynamicImage) -> Option<String> {
    let attempts: Vec<image::DynamicImage> = vec![
        image.clone(),
        image.resize(
            image.width() * 2,
            image.height() * 2,
            image::imageops::FilterType::Lanczos3,
        ),
        image.resize(
            image.width() * 4,
            image.height() * 4,
            image::imageops::FilterType::Lanczos3,
        ),
    ];

    for candidate in attempts {
        let luma = candidate.to_luma8();
        let (w, h) = (luma.width(), luma.height());
        if let Ok(res) = rxing::helpers::detect_in_luma(
            luma.into_raw(),
            w,
            h,
            Some(rxing::BarcodeFormat::QR_CODE),
        ) {
            return Some(res.getText().to_string());
        }
    }
    None
}

impl IdentityExtractor for QrCccdExtractor {
    fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult> {
        let payload = decode_qr(image)?;
        let identity = parse_cccd_payload(&payload)?;
        Some(ExtractResult {
            source: Source::QrCccd,
            confidence: Confidence::Verified,
            identity,
            // Loại giấy tờ là heuristic (G8) nên luôn cần người ngó qua
            review_hints: vec![Field::DocType],
            crop_for_review: None,
        })
    }
}
```

- [ ] **Step 5: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::extractor::qr_cccd`
Expected: PASS, 8 test.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src/declaration mhm/src-tauri/Cargo.toml mhm/src-tauri/Cargo.lock
git commit -m "feat(kbtt): extract identity from CCCD QR payload

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 6: MRZ — checksum và scoring

**Files:**
- Create: `mhm/src-tauri/src/declaration/extractor/mrz.rs`
- Modify: `mhm/src-tauri/src/declaration/extractor/mod.rs`

**Interfaces:**
- Consumes: `model::*`, `normalizer::{mrz_date_to_iso, gender_from_mrz, DateKind}`
- Produces:
  - `trait MrzOcr { fn recognize_lines(&self, img: &image::DynamicImage) -> Vec<String>; }`
  - `char_value(c: char) -> Option<u32>`
  - `checksum(data: &str) -> u32`
  - `struct Td3 { pub line1: String, pub line2: String }`
  - `Td3::parse(line1: &str, line2: &str) -> Option<Td3>`
  - `Td3::checksum_score(&self) -> u8` — trả 0..=5
  - `Td3::to_identity(&self, today_year: u32) -> Option<Identity>`
  - `Td3::full_name(&self) -> String`
  - `struct MrzExtractor<O: MrzOcr>`

- [ ] **Step 1: Viết test thất bại**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Cặp dòng đã verify 5/5 trên hộ chiếu thật
    const L1: &str = "P<RUSZOLOCHEVSKAIA<<VERONIKA<<<<<<<<<<<<<<<<";
    const L2: &str = "7777856719RUS9003082F3512090<<<<<<<<<<<<<<00";

    #[test]
    fn char_values_follow_icao() {
        assert_eq!(char_value('<'), Some(0));
        assert_eq!(char_value('0'), Some(0));
        assert_eq!(char_value('9'), Some(9));
        assert_eq!(char_value('A'), Some(10));
        assert_eq!(char_value('Z'), Some(35));
        assert_eq!(char_value('a'), None);
    }

    #[test]
    fn checksum_matches_known_values() {
        assert_eq!(checksum("777785671"), 9);
        assert_eq!(checksum("900308"), 2);
        assert_eq!(checksum("351209"), 0);
    }

    #[test]
    fn known_good_mrz_scores_five_of_five() {
        let td3 = Td3::parse(L1, L2).expect("phải parse được");
        assert_eq!(td3.checksum_score(), 5);
    }

    #[test]
    fn extracts_line2_fields_correctly() {
        let td3 = Td3::parse(L1, L2).unwrap();
        let id = td3.to_identity(2026).unwrap();
        assert_eq!(id.passport_no.as_deref(), Some("777785671"));
        assert_eq!(id.nationality_iso3, "RUS");
        assert_eq!(id.dob, "1990-03-08");
        assert_eq!(id.gender, "F");
        // G6: 35 ở ô hết hạn là 2035, không phải 1935
        assert_eq!(id.passport_expiry.as_deref(), Some("2035-12-09"));
    }

    #[test]
    fn name_joins_surname_and_given_names() {
        let td3 = Td3::parse(L1, L2).unwrap();
        assert_eq!(td3.full_name(), "ZOLOCHEVSKAIA VERONIKA");
    }

    /// G5 — TEST THEN CHỐT CỦA CẢ MODULE.
    ///
    /// Cả 5 check digit nằm ở dòng 2. Dòng 1 (họ tên) không có checksum nào.
    /// Nên một MRZ hỏng tên vẫn đạt 5/5, và app KHÔNG được coi tên đó là đúng.
    #[test]
    fn perfect_checksum_never_vouches_for_the_name() {
        let corrupted_name = "P<RUSZOLOCHEVSKAIA<X<VERONIKAK<<<<<<<<<<<<<<";
        let td3 = Td3::parse(corrupted_name, L2).unwrap();

        assert_eq!(td3.checksum_score(), 5, "dòng 2 vẫn hoàn hảo");
        assert_ne!(
            td3.full_name(),
            "ZOLOCHEVSKAIA VERONIKA",
            "tên đã sai mà không checksum nào phát hiện"
        );

        let id = td3.to_identity(2026).unwrap();
        assert!(
            !id.name_confirmed_by_human,
            "5/5 checksum KHÔNG được tự xác nhận tên"
        );
    }

    #[test]
    fn short_lines_are_padded_not_rejected() {
        let short = "7777856719RUS9003082F3512090";
        let td3 = Td3::parse(L1, short).expect("pad về 44 ký tự");
        assert_eq!(td3.line2.len(), 44);
    }

    #[test]
    fn garbage_scores_low_enough_to_fall_through_to_manual() {
        let td3 = Td3::parse(
            "P<XXXAAAAAAAAAA<<BBBBBB<<<<<<<<<<<<<<<<<<<<<",
            "1111111111XXX1111111X1111111<<<<<<<<<<<<<<11",
        )
        .unwrap();
        assert!(td3.checksum_score() < 3);
    }

    struct FakeOcr(Vec<String>);
    impl MrzOcr for FakeOcr {
        fn recognize_lines(&self, _img: &image::DynamicImage) -> Vec<String> {
            self.0.clone()
        }
    }

    #[test]
    fn extractor_picks_mrz_lines_out_of_noise() {
        let ocr = FakeOcr(vec![
            "PASSPORT".to_string(),
            "Nơi cấp / Place of issue".to_string(),
            L1.to_string(),
            L2.to_string(),
        ]);
        let img = image::DynamicImage::new_luma8(10, 10);
        let res = MrzExtractor::new(ocr, 2026)
            .try_extract(&img)
            .expect("phải trích được");
        assert_eq!(res.source, Source::MrzTd3);
        assert_eq!(res.confidence, Confidence::Verified);
        assert!(res.review_hints.contains(&Field::FullName));
        assert!(!res.identity.name_confirmed_by_human);
    }

    #[test]
    fn extractor_returns_none_when_no_mrz_present() {
        let ocr = FakeOcr(vec!["Họ và tên".to_string(), "NGUYEN VAN A".to_string()]);
        let img = image::DynamicImage::new_luma8(10, 10);
        assert!(MrzExtractor::new(ocr, 2026).try_extract(&img).is_none());
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::extractor::mrz`
Expected: FAIL — lỗi compile.

- [ ] **Step 3: Viết implementation**

```rust
use super::IdentityExtractor;
use crate::declaration::model::{Confidence, ExtractResult, Field, Identity, Source};
use crate::declaration::normalizer::{gender_from_mrz, mrz_date_to_iso, DateKind};

/// Tầng OCR nằm sau trait riêng vì engine chưa chốt: `ocr-rs` đã có sẵn trong
/// repo (0 dependency mới) còn tesseract có bằng chứng đo thật. `kbtt_probe`
/// chạy cả hai trên ảnh thật rồi mới quyết định.
pub trait MrzOcr {
    fn recognize_lines(&self, img: &image::DynamicImage) -> Vec<String>;
}

pub fn char_value(c: char) -> Option<u32> {
    match c {
        '<' => Some(0),
        '0'..='9' => Some(c as u32 - '0' as u32),
        'A'..='Z' => Some(c as u32 - 'A' as u32 + 10),
        _ => None,
    }
}

/// weights = [7,3,1] lặp lại; check = (Σ giá_trị[i] × w[i mod 3]) mod 10
pub fn checksum(data: &str) -> u32 {
    const WEIGHTS: [u32; 3] = [7, 3, 1];
    data.chars()
        .enumerate()
        .map(|(i, c)| char_value(c).unwrap_or(0) * WEIGHTS[i % 3])
        .sum::<u32>()
        % 10
}

#[derive(Debug, Clone)]
pub struct Td3 {
    pub line1: String,
    pub line2: String,
}

fn pad44(s: &str) -> String {
    let cleaned: String = s.chars().filter(|c| char_value(*c).is_some()).collect();
    let mut out = cleaned;
    if out.len() > 44 {
        out.truncate(44);
    } else {
        while out.len() < 44 {
            out.push('<');
        }
    }
    out
}

impl Td3 {
    pub fn parse(line1: &str, line2: &str) -> Option<Td3> {
        let l1 = pad44(line1);
        let l2 = pad44(line2);
        if l1.len() != 44 || l2.len() != 44 {
            return None;
        }
        Some(Td3 { line1: l1, line2: l2 })
    }

    fn digit_at(&self, idx1: usize) -> Option<u32> {
        self.line2.chars().nth(idx1 - 1)?.to_digit(10)
    }

    fn slice(&self, from1: usize, to1: usize) -> String {
        self.line2
            .chars()
            .skip(from1 - 1)
            .take(to1 - from1 + 1)
            .collect()
    }

    /// Đếm số check digit khớp, 0..=5. Đây là HÀM CHẤM ĐIỂM, không phải
    /// bước kiểm cuối — biến thể tiền xử lý nào điểm cao nhất thì thắng.
    pub fn checksum_score(&self) -> u8 {
        let mut score = 0u8;

        if self.digit_at(10) == Some(checksum(&self.slice(1, 9))) {
            score += 1;
        }
        if self.digit_at(20) == Some(checksum(&self.slice(14, 19))) {
            score += 1;
        }
        if self.digit_at(28) == Some(checksum(&self.slice(22, 27))) {
            score += 1;
        }
        if self.digit_at(43) == Some(checksum(&self.slice(29, 42))) {
            score += 1;
        }
        let composite = format!(
            "{}{}{}",
            self.slice(1, 10),
            self.slice(14, 20),
            self.slice(22, 43)
        );
        if self.digit_at(44) == Some(checksum(&composite)) {
            score += 1;
        }

        score
    }

    /// Dòng 1 vị trí 6–44: `HỌ<<TÊN`. Giữ HOA không dấu — đúng dạng cổng cần.
    pub fn full_name(&self) -> String {
        let raw: String = self.line1.chars().skip(5).collect();
        raw.replace('<', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn to_identity(&self, today_year: u32) -> Option<Identity> {
        let passport_no = self.slice(1, 9).replace('<', "");
        let nationality = self.slice(11, 13);
        let dob = mrz_date_to_iso(&self.slice(14, 19), DateKind::Birth, today_year)?;
        let gender = gender_from_mrz(self.line2.chars().nth(20)?)?;
        let expiry = mrz_date_to_iso(&self.slice(22, 27), DateKind::Expiry, today_year)?;

        Some(Identity {
            id: uuid::Uuid::new_v4().to_string(),
            full_name: self.full_name(),
            dob,
            gender: gender.to_string(),
            nationality_iso3: nationality,
            passport_no: Some(passport_no),
            passport_expiry: Some(expiry),
            // G5: KHÔNG BAO GIỜ tự đặt thành true, kể cả khi 5/5.
            // 5 check digit nằm hết ở dòng 2; dòng 1 không được bảo vệ.
            name_confirmed_by_human: false,
            ..Default::default()
        })
    }
}

pub struct MrzExtractor<O: MrzOcr> {
    ocr: O,
    today_year: u32,
}

impl<O: MrzOcr> MrzExtractor<O> {
    pub fn new(ocr: O, today_year: u32) -> Self {
        MrzExtractor { ocr, today_year }
    }
}

fn looks_like_mrz(line: &str) -> bool {
    let cleaned: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    cleaned.len() >= 30 && cleaned.chars().all(|c| char_value(c).is_some())
}

impl<O: MrzOcr> IdentityExtractor for MrzExtractor<O> {
    fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult> {
        // Biến thể theo đúng thứ tự G3: THÔ trước, xử lý mạnh sau.
        let variants: Vec<image::DynamicImage> = vec![
            image.clone(),
            image.resize(
                image.width() * 2,
                image.height() * 2,
                image::imageops::FilterType::Lanczos3,
            ),
        ];

        let mut best: Option<Td3> = None;
        let mut best_score = 0u8;

        for variant in &variants {
            let lines: Vec<String> = self
                .ocr
                .recognize_lines(variant)
                .into_iter()
                .filter(|l| looks_like_mrz(l))
                .collect();

            for pair in lines.windows(2) {
                if let Some(td3) = Td3::parse(&pair[0], &pair[1]) {
                    let score = td3.checksum_score();
                    if score > best_score {
                        best_score = score;
                        best = Some(td3);
                    }
                }
            }
        }

        let td3 = best?;
        if best_score < 3 {
            return None; // rơi xuống ManualExtractor
        }

        let identity = td3.to_identity(self.today_year)?;
        Some(ExtractResult {
            source: Source::MrzTd3,
            confidence: if best_score == 5 {
                Confidence::Verified
            } else {
                Confidence::NeedsReview
            },
            identity,
            // Tên LUÔN cần người xác nhận, bất kể điểm checksum.
            review_hints: vec![Field::FullName],
            crop_for_review: None,
        })
    }
}
```

Thêm `pub mod mrz;` vào `extractor/mod.rs`.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::extractor::mrz`
Expected: PASS, 10 test. Đặc biệt `perfect_checksum_never_vouches_for_the_name` phải xanh.

- [ ] **Step 5: Commit**

```bash
git add mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): score passport MRZ by checksum

All five check digits live on line 2. The name on line 1 has none, so a
perfect score never vouches for the name.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 7: Validator

**Files:**
- Create: `mhm/src-tauri/src/declaration/validator.rs`
- Modify: `mhm/src-tauri/src/declaration/mod.rs`

**Interfaces:**
- Consumes: `model::{DeclarationRow, Finding, Severity, Field}`, `catalog::{Catalog, CatalogList}`
- Produces:
  - `validate(rows: &[DeclarationRow], catalog: &Catalog, today: &str) -> Vec<Finding>`
  - `has_blocking(findings: &[Finding]) -> bool`

- [ ] **Step 1: Viết test thất bại**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::{Identity, StayInfo};

    fn cat() -> Catalog {
        Catalog::load().unwrap()
    }

    fn vn_row() -> DeclarationRow {
        DeclarationRow {
            link_id: "link-1".into(),
            identity: Identity {
                id: "id-1".into(),
                full_name: "Phan Thị Mỹ Hà".into(),
                dob: "1995-07-28".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                doc_type_code: Some("1".into()),
                doc_type_source: Some("human".into()),
                doc_no: Some("058195006173".into()),
                phone: Some("0901234567".into()),
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "booking-1".into(),
                room_no: "5B".into(),
                check_in: "2026-07-26".into(),
                expected_out: "2026-07-29".into(),
                check_in_raw: "2026-07-26T09:00:00+07:00".into(),
                ..Default::default()
            },
            stay_reason: "2".into(),
            stay_reason_note: None,
        }
    }

    fn nnn_row() -> DeclarationRow {
        DeclarationRow {
            link_id: "link-2".into(),
            identity: Identity {
                id: "id-2".into(),
                full_name: "ZOLOCHEVSKAIA VERONIKA".into(),
                dob: "1990-03-08".into(),
                gender: "F".into(),
                nationality_iso3: "RUS".into(),
                passport_no: Some("777785671".into()),
                passport_expiry: Some("2035-12-09".into()),
                visa_valid_until: Some("2026-09-15".into()),
                name_confirmed_by_human: true,
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "booking-2".into(),
                room_no: "5A".into(),
                check_in: "2026-07-26".into(),
                expected_out: "2026-07-30".into(),
                check_in_raw: "2026-07-26T09:00:00+07:00".into(),
                ..Default::default()
            },
            stay_reason: "1".into(),
            stay_reason_note: None,
        }
    }

    fn codes(f: &[Finding]) -> Vec<String> {
        f.iter().map(|x| x.code.clone()).collect()
    }

    #[test]
    fn clean_rows_produce_no_blocking_findings() {
        let f = validate(&[vn_row(), nnn_row()], &cat(), "2026-07-26");
        assert!(!has_blocking(&f), "gặp: {:?}", codes(&f));
    }

    #[test]
    fn e02_single_token_name_blocks_unless_confirmed() {
        let mut r = vn_row();
        r.identity.full_name = "Hà".into();
        assert!(codes(&validate(&[r.clone()], &cat(), "2026-07-26")).contains(&"E02".to_string()));

        // mononym có thật (hộ chiếu Indonesia) — người tick để gỡ chặn
        r.identity.single_token_name_ok = true;
        assert!(!codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E02".to_string()));
    }

    #[test]
    fn e03_nnn_name_must_be_mrz_shaped() {
        let mut r = nnn_row();
        r.identity.full_name = "Nguyễn Văn A".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E03".to_string()));
    }

    // G5 — không ai xác nhận tên thì không được xuất
    #[test]
    fn e04_blocks_unconfirmed_foreign_name() {
        let mut r = nnn_row();
        r.identity.name_confirmed_by_human = false;
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E04".to_string()));
    }

    #[test]
    fn e05_rejects_unknown_nationality() {
        let mut r = nnn_row();
        r.identity.nationality_iso3 = "XXX".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E05".to_string()));
    }

    #[test]
    fn e07_rejects_checkout_before_checkin() {
        let mut r = vn_row();
        r.stay.expected_out = "2026-07-20".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E07".to_string()));
    }

    #[test]
    fn e08_and_e09_guard_the_visa_window() {
        let mut r = nnn_row();
        r.identity.visa_valid_until = None;
        assert!(codes(&validate(&[r.clone()], &cat(), "2026-07-26")).contains(&"E08".to_string()));

        // khách sẽ ở quá hạn tạm trú — chuyện cơ sở lưu trú phải biết TRƯỚC
        r.identity.visa_valid_until = Some("2026-07-28".into());
        r.stay.expected_out = "2026-07-30".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E09".to_string()));
    }

    /// G10 — cái bẫy nguy hiểm nhất: MRZ cho ngày hết hạn hộ chiếu 2035, rất
    /// dễ bị map nhầm thành thời hạn tạm trú.
    #[test]
    fn e10_catches_passport_expiry_copied_into_visa_field() {
        let mut r = nnn_row();
        r.identity.visa_valid_until = Some("2035-12-09".into());
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E10".to_string()));
    }

    #[test]
    fn e11_and_e12_require_free_text_when_code_is_other() {
        let mut r = vn_row();
        r.identity.doc_type_code = Some("9".into());
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E11".to_string()));

        let mut r2 = vn_row();
        r2.stay_reason = "20".into();
        assert!(codes(&validate(&[r2], &cat(), "2026-07-26")).contains(&"E12".to_string()));
    }

    #[test]
    fn e13_rejects_malformed_document_numbers() {
        let mut r = vn_row();
        r.identity.doc_no = Some("0581 950/06173".into());
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E13".to_string()));
    }

    #[test]
    fn e14_catches_duplicate_within_one_batch() {
        let f = validate(&[vn_row(), vn_row()], &cat(), "2026-07-26");
        assert!(codes(&f).contains(&"E14".to_string()));
    }

    #[test]
    fn e06_rejects_enum_value_outside_catalog() {
        let mut r = vn_row();
        r.stay_reason = "99".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E06".to_string()));
    }

    #[test]
    fn warnings_do_not_block_export() {
        let mut r = vn_row();
        r.stay.room_no = String::new();
        r.identity.phone = None;
        r.stay_reason = "1".into(); // vẫn là mặc định
        r.identity.doc_type_source = Some("heuristic".into());
        let f = validate(&[r], &cat(), "2026-07-26");
        let c = codes(&f);
        assert!(c.contains(&"W01".to_string()));
        assert!(c.contains(&"W02".to_string()));
        assert!(c.contains(&"W03".to_string()));
        assert!(c.contains(&"W06".to_string()));
        assert!(!has_blocking(&f), "cảnh báo không được chặn xuất file");
    }

    #[test]
    fn w04_flags_declaration_past_the_24h_deadline() {
        let mut r = vn_row();
        r.stay.check_in = "2026-07-24".into();
        r.stay.check_in_raw = "2026-07-24T09:00:00+07:00".into();
        let f = validate(&[r], &cat(), "2026-07-26");
        assert!(codes(&f).contains(&"W04".to_string()));
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::validator`
Expected: FAIL — lỗi compile.

- [ ] **Step 3: Viết implementation**

```rust
use crate::declaration::catalog::{Catalog, CatalogList};
use crate::declaration::model::{DeclarationRow, Field, Finding, Severity};
use std::collections::HashSet;

pub fn has_blocking(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Blocking)
}

fn days_between(from: &str, to: &str) -> Option<i64> {
    use chrono::NaiveDate;
    let a = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let b = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    Some((b - a).num_days())
}

fn doc_number_is_clean(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_mrz_shaped(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c == ' ' || c.is_ascii_digit())
}

pub fn validate(rows: &[DeclarationRow], catalog: &Catalog, today: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for row in rows {
        let id = &row.identity;
        let link = row.link_id.as_str();
        let is_vn = id.is_vietnamese();

        // E01 — field bắt buộc theo từng định dạng
        let mut missing: Vec<&str> = Vec::new();
        if id.full_name.trim().is_empty() {
            missing.push("họ tên");
        }
        if id.dob.trim().is_empty() {
            missing.push("ngày sinh");
        }
        if id.gender.trim().is_empty() {
            missing.push("giới tính");
        }
        if id.nationality_iso3.trim().is_empty() {
            missing.push("quốc tịch");
        }
        if row.stay.check_in.trim().is_empty() {
            missing.push("ngày đến");
        }
        if row.stay.expected_out.trim().is_empty() {
            missing.push("ngày đi dự kiến");
        }
        if row.stay_reason.trim().is_empty() {
            missing.push("lý do lưu trú");
        }
        if is_vn {
            if id.doc_type_code.as_deref().unwrap_or("").is_empty() {
                missing.push("loại giấy tờ");
            }
            if id.doc_no.as_deref().unwrap_or("").is_empty() {
                missing.push("số giấy tờ");
            }
        } else if id.passport_no.as_deref().unwrap_or("").is_empty() {
            missing.push("số hộ chiếu");
        }
        if !missing.is_empty() {
            out.push(Finding::blocking(
                "E01",
                link,
                None,
                &format!("Thiếu field bắt buộc: {}", missing.join(", ")),
            ));
        }

        // E02 — tên một token
        if id.full_name.split_whitespace().count() == 1 && !id.single_token_name_ok {
            out.push(Finding::blocking(
                "E02",
                link,
                Some(Field::FullName),
                "Tên chỉ có một chữ. Nếu giấy tờ đúng như vậy, tick xác nhận để gỡ chặn.",
            ));
        }

        if !is_vn {
            // E03 — NNN phải đúng dạng MRZ: HOA, không dấu
            if !is_mrz_shaped(&id.full_name) {
                out.push(Finding::blocking(
                    "E03",
                    link,
                    Some(Field::FullName),
                    "Tên khách nước ngoài phải viết HOA không dấu, đúng dạng MRZ.",
                ));
            }
            // E04 — G5
            if !id.name_confirmed_by_human {
                out.push(Finding::blocking(
                    "E04",
                    link,
                    Some(Field::FullName),
                    "Chưa ai xác nhận tên. Dòng 1 của MRZ không có checksum bảo vệ.",
                ));
            }
            // E08 / E09 / E10
            match id.visa_valid_until.as_deref() {
                None | Some("") => out.push(Finding::blocking(
                    "E08",
                    link,
                    Some(Field::VisaValidUntil),
                    "Thiếu thời hạn tạm trú. Giá trị này không có trong MRZ, phải nhập tay.",
                )),
                Some(visa) => {
                    if days_between(visa, &row.stay.expected_out).unwrap_or(0) > 0 {
                        out.push(Finding::blocking(
                            "E09",
                            link,
                            Some(Field::VisaValidUntil),
                            "Thời hạn tạm trú kết thúc trước ngày đi dự kiến — khách sẽ ở quá hạn.",
                        ));
                    }
                    if Some(visa) == id.passport_expiry.as_deref() {
                        out.push(Finding::blocking(
                            "E10",
                            link,
                            Some(Field::VisaValidUntil),
                            "Thời hạn tạm trú trùng ngày hết hạn hộ chiếu — nghi lấy nhầm nguồn.",
                        ));
                    }
                }
            }
        }

        // E05
        if !catalog.has_code(CatalogList::QuocTich, &id.nationality_iso3) {
            out.push(Finding::blocking(
                "E05",
                link,
                Some(Field::Nationality),
                &format!("Mã quốc tịch {} không có trong danh mục.", id.nationality_iso3),
            ));
        }

        // E06
        if !row.stay_reason.is_empty()
            && !catalog.has_code(CatalogList::LyDoCuTru, &row.stay_reason)
        {
            out.push(Finding::blocking(
                "E06",
                link,
                None,
                &format!("Lý do cư trú {} không có trong danh mục.", row.stay_reason),
            ));
        }
        if let Some(code) = id.doc_type_code.as_deref() {
            if !code.is_empty() && !catalog.has_code(CatalogList::LoaiGiayTo, code) {
                out.push(Finding::blocking(
                    "E06",
                    link,
                    Some(Field::DocType),
                    &format!("Loại giấy tờ {code} không có trong danh mục."),
                ));
            }
        }

        // E07
        if days_between(&row.stay.check_in, &row.stay.expected_out).unwrap_or(0) < 0 {
            out.push(Finding::blocking(
                "E07",
                link,
                None,
                "Ngày đi dự kiến sớm hơn ngày đến.",
            ));
        }

        // E11 / E12
        if id.doc_type_code.as_deref() == Some("9")
            && id.doc_type_name.as_deref().unwrap_or("").is_empty()
        {
            out.push(Finding::blocking(
                "E11",
                link,
                Some(Field::DocType),
                "Loại giấy tờ là 'Giấy Tờ Khác' nhưng chưa ghi tên giấy tờ.",
            ));
        }
        if row.stay_reason == "20" && row.stay_reason_note.as_deref().unwrap_or("").is_empty() {
            out.push(Finding::blocking(
                "E12",
                link,
                None,
                "Lý do là 'Mục đích khác' nhưng chưa nhập lý do cụ thể.",
            ));
        }

        // E13
        let number = if is_vn {
            id.doc_no.as_deref().unwrap_or("")
        } else {
            id.passport_no.as_deref().unwrap_or("")
        };
        if !doc_number_is_clean(number) {
            out.push(Finding::blocking(
                "E13",
                link,
                Some(if is_vn { Field::DocNo } else { Field::PassportNo }),
                "Số giấy tờ rỗng hoặc chứa ký tự lạ.",
            ));
        }

        // E14 — trùng trong cùng lô
        let key = (number.to_string(), row.stay.check_in.clone());
        if !number.is_empty() && !seen.insert(key) {
            out.push(Finding::blocking(
                "E14",
                link,
                None,
                "Trùng hồ sơ trong cùng lô: cùng số giấy tờ và cùng ngày đến.",
            ));
        }

        // Cảnh báo
        if row.stay.room_no.trim().is_empty() {
            out.push(Finding::warning("W01", link, None, "Thiếu số phòng."));
        }
        if is_vn && id.phone.as_deref().unwrap_or("").is_empty() {
            out.push(Finding::warning(
                "W02",
                link,
                Some(Field::Phone),
                "Thiếu số điện thoại.",
            ));
        }
        if row.stay_reason == "1" {
            out.push(Finding::warning(
                "W03",
                link,
                None,
                "Lý do lưu trú vẫn là mặc định 'Du lịch', chưa ai đổi.",
            ));
        }
        if days_between(&row.stay.check_in, today).unwrap_or(0) >= 1 {
            out.push(Finding::warning(
                "W04",
                link,
                None,
                "Đã quá 24h kể từ lúc khách đến mà chưa có lô nào được xác nhận.",
            ));
        }
        if id.doc_type_source.as_deref() == Some("heuristic") {
            out.push(Finding::warning(
                "W06",
                link,
                Some(Field::DocType),
                "Loại giấy tờ do máy suy từ ngày cấp, chưa ai xác nhận.",
            ));
        }
    }

    out
}
```

Ghi chú: `W05` (`confidence = needs_review`) được sinh ở lớp `repo`/`commands` khi
dựng `DeclarationRow` từ DB, vì `DeclarationRow` không mang `extract_confidence`.
Thêm nó vào Task 10 của Kế hoạch B.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::validator`
Expected: PASS, 14 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): validate declaration rows before export

The portal reports success on a zero-record import, so every check has to
happen here.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 8: XmlWriter

**Files:**
- Create: `mhm/src-tauri/src/declaration/writer/mod.rs`
- Create: `mhm/src-tauri/src/declaration/writer/xml.rs`
- Modify: `mhm/src-tauri/src/declaration/mod.rs`

**Interfaces:**
- Consumes: `model::DeclarationRow`, `normalizer::iso_to_portal`
- Produces:
  - `xml::render(rows: &[DeclarationRow], lead_example: bool) -> Result<String, String>`
  - `xml::escape(s: &str) -> String`

- [ ] **Step 1: Viết test thất bại**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::{Identity, StayInfo};

    fn row(name: &str, room: &str, actual_out: Option<&str>) -> DeclarationRow {
        DeclarationRow {
            link_id: "l".into(),
            identity: Identity {
                full_name: name.into(),
                dob: "1990-03-08".into(),
                gender: "F".into(),
                nationality_iso3: "RUS".into(),
                passport_no: Some("777785671".into()),
                passport_expiry: Some("2035-12-09".into()),
                visa_valid_until: Some("2026-09-15".into()),
                name_confirmed_by_human: true,
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "b".into(),
                room_no: room.into(),
                check_in: "2026-07-25".into(),
                expected_out: "2026-07-30".into(),
                actual_out: actual_out.map(|s| s.to_string()),
                check_in_raw: "2026-07-25T09:00:00+07:00".into(),
            },
            stay_reason: "1".into(),
            stay_reason_note: None,
        }
    }

    #[test]
    fn renders_the_documented_shape() {
        let xml = render(&[row("ZOLOCHEVSKAIA VERONIKA", "5A", Some("2026-07-29"))], false).unwrap();
        let expected = r#"<?xml version="1.0" encoding="UTF-8"?>
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
"#;
        assert_eq!(xml, expected);
    }

    // F9 — khách chưa trả phòng thì BỎ HẲN tag, không ghi tag rỗng
    #[test]
    fn omits_checkout_tag_when_guest_has_not_left() {
        let xml = render(&[row("A B", "5A", None)], false).unwrap();
        assert!(!xml.contains("ngay_tra_phong"));
    }

    #[test]
    fn nationality_is_bare_iso3_unlike_xlsx() {
        let xml = render(&[row("A B", "5A", None)], false).unwrap();
        assert!(xml.contains("<ma_quoc_tich>RUS</ma_quoc_tich>"));
        assert!(!xml.contains("RUS - "));
    }

    #[test]
    fn sequence_numbers_are_consecutive_from_one() {
        let xml = render(
            &[row("A B", "1", None), row("C D", "2", None), row("E F", "3", None)],
            false,
        )
        .unwrap();
        assert!(xml.contains("<so_thu_tu>1</so_thu_tu>"));
        assert!(xml.contains("<so_thu_tu>2</so_thu_tu>"));
        assert!(xml.contains("<so_thu_tu>3</so_thu_tu>"));
    }

    /// Một dấu `&` chưa escape làm hỏng cả file — và cổng sẽ nhận file hỏng
    /// đó rồi báo "thành công".
    #[test]
    fn escapes_xml_metacharacters() {
        assert_eq!(escape("A & B"), "A &amp; B");
        assert_eq!(escape("<x>"), "&lt;x&gt;");
        assert_eq!(escape("say \"hi\""), "say &quot;hi&quot;");
        assert_eq!(escape("it's"), "it&apos;s");

        let xml = render(&[row("SMITH & SONS", "5A", None)], false).unwrap();
        assert!(xml.contains("SMITH &amp; SONS"));
        assert!(!xml.contains("SMITH & SONS"));
    }

    /// §14.1 chưa test được trên cổng. Cờ này biến cả ba nhánh kết quả
    /// thành một dòng setting thay vì một lần sửa code.
    #[test]
    fn lead_example_record_can_be_switched_on() {
        let xml = render(&[row("A B", "5A", None)], true).unwrap();
        assert!(xml.contains("[EXAMPLE]"));
        assert!(xml.contains("<so_thu_tu>1</so_thu_tu>"));
        assert!(xml.contains("<so_thu_tu>2</so_thu_tu>"));

        let off = render(&[row("A B", "5A", None)], false).unwrap();
        assert!(!off.contains("[EXAMPLE]"));
    }

    #[test]
    fn rejects_row_without_full_dob() {
        let mut r = row("A B", "5A", None);
        r.identity.dob = "1990".into();
        assert!(render(&[r], false).is_err());
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::writer::xml`
Expected: FAIL — lỗi compile.

- [ ] **Step 3: Viết implementation**

`writer/mod.rs`:

```rust
pub mod xml;
pub mod xlsx;
```

`writer/xml.rs`:

```rust
use crate::declaration::model::DeclarationRow;
use crate::declaration::normalizer::iso_to_portal;

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn record(seq: usize, r: &DeclarationRow) -> Result<String, String> {
    let id = &r.identity;
    let dob = iso_to_portal(&id.dob)
        .ok_or_else(|| format!("Ngày sinh không hợp lệ: {}", id.dob))?;
    let check_in = iso_to_portal(&r.stay.check_in)
        .ok_or_else(|| format!("Ngày đến không hợp lệ: {}", r.stay.check_in))?;
    let expected = iso_to_portal(&r.stay.expected_out)
        .ok_or_else(|| format!("Ngày đi không hợp lệ: {}", r.stay.expected_out))?;
    let visa = id
        .visa_valid_until
        .as_deref()
        .and_then(iso_to_portal)
        .ok_or_else(|| "Thiếu thời hạn tạm trú".to_string())?;

    let mut s = String::new();
    s.push_str("    <THONG_TIN_KHACH>\n");
    s.push_str(&format!("        <so_thu_tu>{seq}</so_thu_tu>\n"));
    s.push_str(&format!("        <ho_ten>{}</ho_ten>\n", escape(&id.full_name)));
    s.push_str(&format!("        <ngay_sinh>{dob}</ngay_sinh>\n"));
    // v1 chỉ xuất khách có ngày sinh đủ, nên luôn là D
    s.push_str("        <ngay_sinh_dung_den>D</ngay_sinh_dung_den>\n");
    s.push_str(&format!("        <gioi_tinh>{}</gioi_tinh>\n", escape(&id.gender)));
    // Chỉ mã ISO3, KHÁC XLSX vốn ghi "RUS - Russia"
    s.push_str(&format!(
        "        <ma_quoc_tich>{}</ma_quoc_tich>\n",
        escape(&id.nationality_iso3)
    ));
    s.push_str(&format!(
        "        <so_ho_chieu>{}</so_ho_chieu>\n",
        escape(id.passport_no.as_deref().unwrap_or(""))
    ));
    s.push_str(&format!(
        "        <so_phong>{}</so_phong>\n",
        escape(&r.stay.room_no)
    ));
    s.push_str(&format!("        <ngay_den>{check_in}</ngay_den>\n"));
    s.push_str(&format!(
        "        <ngay_di_du_kien>{expected}</ngay_di_du_kien>\n"
    ));
    // F9: bỏ hẳn tag nếu khách chưa trả phòng
    if let Some(actual) = r.stay.actual_out.as_deref().and_then(iso_to_portal) {
        s.push_str(&format!(
            "        <ngay_tra_phong>{actual}</ngay_tra_phong>\n"
        ));
    }
    s.push_str(&format!(
        "        <thoi_han_tam_tru>{visa}</thoi_han_tam_tru>\n"
    ));
    s.push_str("    </THONG_TIN_KHACH>\n");
    Ok(s)
}

fn example_record(seq: usize) -> String {
    format!(
        "    <THONG_TIN_KHACH>\n\
         \x20       <so_thu_tu>{seq}</so_thu_tu>\n\
         \x20       <ho_ten>[EXAMPLE]</ho_ten>\n\
         \x20       <ngay_sinh>01/01/1980</ngay_sinh>\n\
         \x20       <ngay_sinh_dung_den>D</ngay_sinh_dung_den>\n\
         \x20       <gioi_tinh>M</gioi_tinh>\n\
         \x20       <ma_quoc_tich>USA</ma_quoc_tich>\n\
         \x20       <so_ho_chieu>[EXAMPLE]</so_ho_chieu>\n\
         \x20       <so_phong>0</so_phong>\n\
         \x20       <ngay_den>01/01/2000</ngay_den>\n\
         \x20       <ngay_di_du_kien>02/01/2000</ngay_di_du_kien>\n\
         \x20       <thoi_han_tam_tru>03/01/2000</thoi_han_tam_tru>\n\
         \x20   </THONG_TIN_KHACH>\n"
    )
}

/// `lead_example` bật khi cổng bỏ record đầu theo vị trí (§14.1 — chưa test).
pub fn render(rows: &[DeclarationRow], lead_example: bool) -> Result<String, String> {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<KHAI_BAO_TAM_TRU>\n");
    let mut seq = 1usize;
    if lead_example {
        out.push_str(&example_record(seq));
        seq += 1;
    }
    for r in rows {
        out.push_str(&record(seq, r)?);
        seq += 1;
    }
    out.push_str("</KHAI_BAO_TAM_TRU>\n");
    Ok(out)
}
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::writer::xml`
Expected: PASS, 7 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): render foreign guest declarations as XML

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 9: XlsxWriter và gate 7 assert

**Files:**
- Create: `mhm/src-tauri/src/declaration/writer/xlsx.rs`
- Modify: `mhm/src-tauri/Cargo.toml` (thêm `umya-spreadsheet = "3.0.1"`, `zip`)

**Interfaces:**
- Consumes: `model::DeclarationRow`, `catalog::{Catalog, CatalogList}`, `normalizer::iso_to_portal`
- Produces:
  - `xlsx::write_batch(rows: &[DeclarationRow], catalog: &Catalog, template: &Path, out: &Path) -> Result<(), String>`
  - `xlsx::verify_output(out: &Path, rows: &[DeclarationRow]) -> Result<(), String>` — 7 assert

- [ ] **Step 1: Viết test thất bại**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::{Identity, StayInfo};

    fn vn_row(name: &str, doc: &str) -> DeclarationRow {
        DeclarationRow {
            link_id: "l".into(),
            identity: Identity {
                full_name: name.into(),
                dob: "1995-07-28".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                doc_type_code: Some("1".into()),
                doc_no: Some(doc.into()),
                phone: Some("0901234567".into()),
                address_detail: Some("KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận".into()),
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "b".into(),
                room_no: "5B".into(),
                check_in: "2026-07-26".into(),
                expected_out: "2026-07-29".into(),
                check_in_raw: "2026-07-26T09:00:00+07:00".into(),
                ..Default::default()
            },
            stay_reason: "2".into(),
            stay_reason_note: None,
        }
    }

    fn template() -> std::path::PathBuf {
        crate::declaration::find_kbtt_resource("tblt_vn_import.xlsx").unwrap()
    }

    fn write_to_temp(rows: &[DeclarationRow]) -> std::path::PathBuf {
        let cat = Catalog::load().unwrap();
        let out = std::env::temp_dir().join(format!("kbtt_test_{}.xlsx", uuid::Uuid::new_v4()));
        write_batch(rows, &cat, &template(), &out).expect("ghi được");
        out
    }

    /// F2 — TEST ÂM QUAN TRỌNG NHẤT. Server bỏ row 4 theo vị trí. Ghi đè nó
    /// nghĩa là mất một khách mà cổng vẫn báo "thành công".
    #[test]
    fn never_writes_over_the_example_row() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let sheet = book.get_sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(sheet.get_value("A4"), "[EXAMPLE]");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn data_starts_at_row_five_with_stt_two() {
        let out = write_to_temp(&[
            vn_row("Phan Thị Mỹ Hà", "058195006173"),
            vn_row("Lê Đình Lực", "079174011721"),
        ]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.get_sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.get_value("A5"), "2");
        assert_eq!(s.get_value("B5"), "Phan Thị Mỹ Hà");
        assert_eq!(s.get_value("A6"), "3");
        assert_eq!(s.get_value("B6"), "Lê Đình Lực");
        let _ = std::fs::remove_file(out);
    }

    /// F7 — Excel sẽ ăn mất số 0 đầu nếu cell là số.
    #[test]
    fn document_number_keeps_its_leading_zero() {
        let out = write_to_temp(&[vn_row("Lê Đình Lực", "079174011721")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.get_sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.get_value("H5"), "079174011721");
        let _ = std::fs::remove_file(out);
    }

    /// F6 — ngày phải là text `dd/MM/yyyy`, không phải serial number.
    #[test]
    fn dates_are_text_in_portal_format() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.get_sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.get_value("C5"), "28/07/1995");
        assert_eq!(s.get_value("N5"), "26/07/2026");
        assert_eq!(s.get_value("O5"), "29/07/2026");
        let _ = std::fs::remove_file(out);
    }

    /// F5 — ghi nguyên chuỗi `mã - nhãn`, không phải chỉ mã.
    #[test]
    fn enums_are_written_as_full_display_strings() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.get_sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.get_value("D5"), "F - Nữ");
        assert_eq!(s.get_value("E5"), "VNM - Viet Nam");
        assert_eq!(s.get_value("F5"), "1 - Thẻ CCCD");
        assert_eq!(s.get_value("Q5"), "2 - Công tác");
        let _ = std::fs::remove_file(out);
    }

    /// G7 — v1 để trống tỉnh/phường, địa chỉ đi nguyên chuỗi vào cột M.
    #[test]
    fn province_and_ward_stay_empty_address_goes_raw() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.get_sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.get_value("K5"), "");
        assert_eq!(s.get_value("L5"), "");
        assert!(s.get_value("M5").contains("Ninh Thuận"));
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn gate_passes_on_a_well_formed_file() {
        let rows = vec![vn_row("Phan Thị Mỹ Hà", "058195006173")];
        let out = write_to_temp(&rows);
        assert!(verify_output(&out, &rows).is_ok());
        let _ = std::fs::remove_file(out);
    }

    /// umya `defined_names()` trả 0 dù file có 40 — nên gate phải đọc zip.
    #[test]
    fn gate_counts_defined_names_from_the_zip_not_the_api() {
        let rows = vec![vn_row("Phan Thị Mỹ Hà", "058195006173")];
        let out = write_to_temp(&rows);
        assert_eq!(count_defined_names(&out).unwrap(), 40);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn gate_rejects_a_file_whose_example_row_was_clobbered() {
        let rows = vec![vn_row("Phan Thị Mỹ Hà", "058195006173")];
        let out = write_to_temp(&rows);

        let mut book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        book.get_sheet_by_name_mut("DS_KHACH_VIET_NAM_LUU_TRU")
            .unwrap()
            .get_cell_mut("A4")
            .set_value_string("phá hoại");
        umya_spreadsheet::writer::xlsx::write(&book, &out).unwrap();

        let err = verify_output(&out, &rows).unwrap_err();
        assert!(err.contains("row 4"), "báo lỗi phải nói rõ row 4: {err}");
        let _ = std::fs::remove_file(out);
    }
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::writer::xlsx`
Expected: FAIL — lỗi compile.

- [ ] **Step 3: Thêm dependency**

```bash
cd mhm/src-tauri && cargo add umya-spreadsheet@3.0.1 zip@8
```

- [ ] **Step 4: Viết implementation**

```rust
use crate::declaration::catalog::{Catalog, CatalogList};
use crate::declaration::model::DeclarationRow;
use crate::declaration::normalizer::iso_to_portal;
use std::io::Read;
use std::path::Path;

const SHEET: &str = "DS_KHACH_VIET_NAM_LUU_TRU";
/// Row 4 là hàng [EXAMPLE]; cổng bỏ nó THEO VỊ TRÍ. Data bắt đầu từ row 5.
const FIRST_DATA_ROW: u32 = 5;
/// Row 4 đã mang STT 1, nên dòng đầu của ta là 2.
const FIRST_STT: u32 = 2;

fn cell(col: &str, row: u32) -> String {
    format!("{col}{row}")
}

pub fn write_batch(
    rows: &[DeclarationRow],
    catalog: &Catalog,
    template: &Path,
    out: &Path,
) -> Result<(), String> {
    if rows.is_empty() {
        return Err("Lô rỗng, không có gì để ghi.".to_string());
    }

    let mut book = umya_spreadsheet::reader::xlsx::read(template)
        .map_err(|e| format!("Không đọc được template: {e:?}"))?;

    {
        let sheet = book
            .get_sheet_by_name_mut(SHEET)
            .ok_or_else(|| format!("Template thiếu sheet {SHEET}"))?;

        for (i, r) in rows.iter().enumerate() {
            let row_no = FIRST_DATA_ROW + i as u32;
            let stt = FIRST_STT + i as u32;
            let id = &r.identity;

            sheet
                .get_cell_mut(cell("A", row_no).as_str())
                .set_value_number(stt);
            sheet
                .get_cell_mut(cell("B", row_no).as_str())
                .set_value_string(&id.full_name);
            sheet.get_cell_mut(cell("C", row_no).as_str()).set_value_string(
                iso_to_portal(&id.dob).ok_or_else(|| format!("Ngày sinh hỏng: {}", id.dob))?,
            );
            sheet.get_cell_mut(cell("D", row_no).as_str()).set_value_string(
                catalog
                    .display_for(CatalogList::GioiTinh, &id.gender)
                    .ok_or_else(|| format!("Giới tính không có trong danh mục: {}", id.gender))?,
            );
            sheet.get_cell_mut(cell("E", row_no).as_str()).set_value_string(
                catalog
                    .display_for(CatalogList::QuocTich, &id.nationality_iso3)
                    .ok_or_else(|| format!("Quốc tịch không có: {}", id.nationality_iso3))?,
            );
            if let Some(code) = id.doc_type_code.as_deref() {
                sheet.get_cell_mut(cell("F", row_no).as_str()).set_value_string(
                    catalog
                        .display_for(CatalogList::LoaiGiayTo, code)
                        .ok_or_else(|| format!("Loại giấy tờ không có: {code}"))?,
                );
            }
            if let Some(v) = id.doc_type_name.as_deref() {
                sheet.get_cell_mut(cell("G", row_no).as_str()).set_value_string(v);
            }
            sheet
                .get_cell_mut(cell("H", row_no).as_str())
                .set_value_string(id.doc_no.as_deref().unwrap_or(""));
            if let Some(v) = id.phone.as_deref() {
                sheet.get_cell_mut(cell("I", row_no).as_str()).set_value_string(v);
            }
            if let Some(code) = id.residence_status.as_deref() {
                if let Some(d) = catalog.display_for(CatalogList::NoiCuTru, code) {
                    sheet.get_cell_mut(cell("J", row_no).as_str()).set_value_string(d);
                }
            }
            // K, L để trống có chủ ý: danh mục hành chính đã đổi mà giấy tờ
            // thì chưa, và fuzzy-match tên phường tạo ra khai báo sai im lặng.
            if let Some(v) = id.address_detail.as_deref() {
                sheet.get_cell_mut(cell("M", row_no).as_str()).set_value_string(v);
            }
            sheet.get_cell_mut(cell("N", row_no).as_str()).set_value_string(
                iso_to_portal(&r.stay.check_in)
                    .ok_or_else(|| format!("Ngày đến hỏng: {}", r.stay.check_in))?,
            );
            sheet.get_cell_mut(cell("O", row_no).as_str()).set_value_string(
                iso_to_portal(&r.stay.expected_out)
                    .ok_or_else(|| format!("Ngày đi hỏng: {}", r.stay.expected_out))?,
            );
            if !r.stay.room_no.is_empty() {
                sheet
                    .get_cell_mut(cell("P", row_no).as_str())
                    .set_value_string(&r.stay.room_no);
            }
            sheet.get_cell_mut(cell("Q", row_no).as_str()).set_value_string(
                catalog
                    .display_for(CatalogList::LyDoCuTru, &r.stay_reason)
                    .ok_or_else(|| format!("Lý do cư trú không có: {}", r.stay_reason))?,
            );
            if let Some(v) = r.stay_reason_note.as_deref() {
                sheet.get_cell_mut(cell("R", row_no).as_str()).set_value_string(v);
            }
        }
    }

    umya_spreadsheet::writer::xlsx::write(&book, out)
        .map_err(|e| format!("Không ghi được file: {e:?}"))?;

    // Gate: fail thì XÓA FILE và không cho ai ghi declaration_batch.
    if let Err(e) = verify_output(out, rows) {
        let _ = std::fs::remove_file(out);
        return Err(e);
    }

    Ok(())
}

/// Đếm `<definedName` trực tiếp trong zip.
///
/// `umya::defined_names()` trả 0 dù file có 40 — đã đo. Không dùng API đó.
pub fn count_defined_names(path: &Path) -> Result<usize, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Không mở được file: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("File không phải zip: {e}"))?;
    let mut xml = String::new();
    zip.by_name("xl/workbook.xml")
        .map_err(|e| format!("Thiếu xl/workbook.xml: {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("Không đọc được workbook.xml: {e}"))?;
    Ok(xml.matches("<definedName ").count())
}

/// Bảy assert của §9.2. Cổng không báo lỗi, nên đây là chốt chặn cuối cùng.
pub fn verify_output(out: &Path, rows: &[DeclarationRow]) -> Result<(), String> {
    let book = umya_spreadsheet::reader::xlsx::read(out)
        .map_err(|e| format!("Không đọc lại được file vừa ghi: {e:?}"))?;
    let sheet = book
        .get_sheet_by_name(SHEET)
        .ok_or_else(|| format!("File thiếu sheet {SHEET}"))?;

    // 1 — row 4 còn nguyên
    let a4 = sheet.get_value("A4");
    if a4 != "[EXAMPLE]" {
        return Err(format!(
            "Gate 1 fail: row 4 cột A là {a4:?}, phải là \"[EXAMPLE]\". \
             Cổng bỏ row 4 theo vị trí — ghi đè nó là mất khách."
        ));
    }

    // 2 — dòng đầu khớp
    let b5 = sheet.get_value("B5");
    if b5 != rows[0].identity.full_name {
        return Err(format!(
            "Gate 2 fail: row 5 cột B là {b5:?}, phải là {:?}",
            rows[0].identity.full_name
        ));
    }

    for (i, r) in rows.iter().enumerate() {
        let row_no = FIRST_DATA_ROW + i as u32;

        // 3 — ngày là text dd/MM/yyyy
        for col in ["C", "N", "O"] {
            let v = sheet.get_value(cell(col, row_no).as_str());
            let ok = v.len() == 10
                && v.as_bytes()[2] == b'/'
                && v.as_bytes()[5] == b'/'
                && v.chars().filter(|c| c.is_ascii_digit()).count() == 8;
            if !ok {
                return Err(format!(
                    "Gate 3 fail: {col}{row_no} = {v:?}, phải là text dd/MM/yyyy. \
                     Excel đã đổi nó thành serial number."
                ));
            }
        }

        // 4 — số giấy tờ giữ đủ ký tự, kể cả số 0 đầu
        let want = r.identity.doc_no.as_deref().unwrap_or("");
        let got = sheet.get_value(cell("H", row_no).as_str());
        if got != want {
            return Err(format!(
                "Gate 4 fail: H{row_no} = {got:?}, phải là {want:?} (mất số 0 đầu?)"
            ));
        }
    }

    // 5 — sheet danh mục còn nguyên
    let dm = book
        .get_sheet_by_name("DANH_MUC")
        .ok_or("Gate 5 fail: mất sheet DANH_MUC")?;
    let count_col = |sheet: &umya_spreadsheet::Worksheet, col: &str, upto: u32| {
        (2..=upto)
            .filter(|r| !sheet.get_value(cell(col, *r).as_str()).is_empty())
            .count()
    };
    for (col, upto, want, name) in [
        ("A", 10u32, 9usize, "LOAI_GIAY_TO"),
        ("B", 4, 3, "NOI_CU_TRU"),
        ("C", 21, 20, "LY_DO_CU_TRU"),
        ("D", 3, 2, "GIOI_TINH"),
        ("E", 206, 205, "QUOC_TICH"),
    ] {
        let got = count_col(dm, col, upto);
        if got != want {
            return Err(format!("Gate 5 fail: {name} còn {got} dòng, phải là {want}"));
        }
    }

    // 6 — tỉnh và phường còn nguyên
    let tt = book
        .get_sheet_by_name("TINH_THANH")
        .ok_or("Gate 6 fail: mất sheet TINH_THANH")?;
    let tt_n = count_col(tt, "C", 35);
    if tt_n != 34 {
        return Err(format!("Gate 6 fail: TINH_THANH còn {tt_n} dòng, phải là 34"));
    }
    let px = book
        .get_sheet_by_name("PHUONG_XA")
        .ok_or("Gate 6 fail: mất sheet PHUONG_XA")?;
    let px_n = count_col(px, "D", 3324);
    if px_n != 3323 {
        return Err(format!("Gate 6 fail: PHUONG_XA còn {px_n} dòng, phải là 3323"));
    }

    // 7 — named range còn nguyên
    let names = count_defined_names(out)?;
    if names != 40 {
        return Err(format!("Gate 7 fail: còn {names} definedName, phải là 40"));
    }

    Ok(())
}
```

- [ ] **Step 5: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::writer::xlsx`
Expected: PASS, 9 test.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src/declaration mhm/src-tauri/Cargo.toml mhm/src-tauri/Cargo.lock
git commit -m "feat(kbtt): write Vietnamese guest declarations into the template

Never creates the workbook from scratch and never touches row 4. Seven
read-back assertions run before the file is allowed to exist.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 10: Migration v20 và repo

**Files:**
- Create: `mhm/src-tauri/src/db/declaration.rs`
- Create: `mhm/src-tauri/src/declaration/repo.rs`
- Modify: `mhm/src-tauri/src/db.rs` (đăng ký migration)

**Interfaces:**
- Produces:
  - `db::declaration::migrate_v20_declaration_tables(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error>`
  - `repo::load_stays_for_declaration(pool) -> Result<Vec<StayInfo>, String>` — CHỈ SELECT
  - `repo::insert_identity`, `repo::insert_link`, `repo::insert_batch`, `repo::insert_entries`
  - `repo::count_undeclared_within_48h(pool) -> Result<i64, String>`
  - `repo::set_batch_verified`, `repo::set_batch_failed`

- [ ] **Step 1: Viết migration**

`mhm/src-tauri/src/db/declaration.rs` — thuần `CREATE TABLE`, không `ALTER` gì:

```rust
use sqlx::{Pool, Sqlite};

use super::set_schema_version;

pub(super) async fn migrate_v20_declaration_tables(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_identity (
            id                      TEXT PRIMARY KEY,
            source                  TEXT NOT NULL,
            extract_confidence      TEXT NOT NULL,
            full_name               TEXT NOT NULL,
            dob                     TEXT NOT NULL,
            gender                  TEXT NOT NULL,
            nationality_iso3        TEXT NOT NULL,
            doc_type_code           TEXT,
            doc_type_source         TEXT,
            doc_type_name           TEXT,
            doc_no                  TEXT,
            phone                   TEXT,
            residence_status        TEXT,
            address_detail          TEXT,
            passport_no             TEXT,
            passport_expiry         TEXT,
            visa_valid_until        TEXT,
            name_confirmed_by_human INTEGER NOT NULL DEFAULT 0,
            single_token_name_ok    INTEGER NOT NULL DEFAULT 0,
            redacted_at             TEXT,
            created_at              TEXT NOT NULL
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_link (
            id               TEXT PRIMARY KEY,
            identity_id      TEXT NOT NULL REFERENCES declaration_identity(id),
            stay_id          TEXT NOT NULL,
            stay_reason      TEXT NOT NULL,
            stay_reason_note TEXT,
            actual_check_out TEXT,
            created_at       TEXT NOT NULL,
            UNIQUE(identity_id, stay_id)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_batch (
            id             TEXT PRIMARY KEY,
            kind           TEXT NOT NULL,
            date_from      TEXT,
            date_to        TEXT,
            file_path      TEXT NOT NULL,
            row_count      INTEGER NOT NULL,
            status         TEXT NOT NULL,
            verified_count INTEGER,
            verified_at    TEXT,
            note           TEXT,
            created_at     TEXT NOT NULL
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_entry (
            batch_id  TEXT NOT NULL REFERENCES declaration_batch(id),
            link_id   TEXT NOT NULL REFERENCES declaration_link(id),
            row_index INTEGER NOT NULL,
            PRIMARY KEY (batch_id, link_id)
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_decl_link_stay ON declaration_link(stay_id)")
        .execute(&mut *tx)
        .await?;

    set_schema_version(&mut tx, 20).await?;
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 2: Đăng ký migration**

Trong `mhm/src-tauri/src/db.rs`, thêm `pub mod declaration;` cạnh các mod khác của `db`, và thêm khối đăng ký ngay sau `migrate_v19_agent_digest_runs`, theo đúng dạng của những khối trước đó (đọc khối v19 rồi lặp lại cấu trúc với `version < 20` và `declaration::migrate_v20_declaration_tables(pool).await?;`).

- [ ] **Step 3: Viết test ranh giới — không được ghi vào bảng cũ**

Thêm vào `mhm/src-tauri/src/declaration/repo.rs`:

```rust
#[cfg(test)]
mod tests {
    /// Nguyên tắc bao trùm của module: PMS đang vận hành thật, không được
    /// migrate hay ghi vào nó vì một tính năng phụ. Test này đọc source và
    /// bắt mọi câu ghi chạm bảng cũ.
    #[test]
    fn declaration_module_never_writes_to_legacy_tables() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/declaration");
        let legacy = ["guests", "bookings", "booking_guests", "rooms"];
        let writes = ["insert into", "update ", "delete from", "alter table"];

        let mut offences = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap().to_lowercase();
                for stmt in writes {
                    for (idx, _) in text.match_indices(stmt) {
                        let window = &text[idx..text.len().min(idx + 120)];
                        for table in legacy {
                            let hit = window.contains(&format!(" {table} "))
                                || window.contains(&format!(" {table}("))
                                || window.contains(&format!(" {table}\n"));
                            if hit {
                                offences.push(format!("{}: {stmt} ... {table}", p.display()));
                            }
                        }
                    }
                }
            }
        }

        assert!(
            offences.is_empty(),
            "Module khai báo ghi vào bảng của PMS: {offences:#?}"
        );
    }

    /// §12 — không lưu ảnh, không lưu payload thô.
    #[test]
    fn declaration_module_stores_no_images_or_raw_payloads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/declaration");
        let banned = ["photo_path", "raw_payload"];
        let mut offences = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap();
                for word in banned {
                    if text.contains(word) {
                        offences.push(format!("{}: {word}", p.display()));
                    }
                }
            }
        }
        assert!(offences.is_empty(), "Vi phạm §12: {offences:#?}");
    }
}
```

- [ ] **Step 4: Viết repo**

`repo.rs` phần trên. Truy vấn đọc bảng cũ chỉ có `SELECT`:

```rust
use crate::declaration::model::StayInfo;
use crate::declaration::normalizer::{booking_ts_to_iso_date, strip_room_prefix};
use sqlx::{Pool, Row, Sqlite};

/// Đường DUY NHẤT module này đọc dữ liệu của PMS. Chỉ SELECT.
pub async fn load_stays_for_declaration(pool: &Pool<Sqlite>) -> Result<Vec<StayInfo>, String> {
    let rows = sqlx::query(
        "SELECT b.id AS stay_id, r.name AS room_name, b.check_in_at,
                b.expected_checkout, b.actual_checkout
           FROM bookings b
           JOIN rooms r ON r.id = b.room_id
          WHERE b.status = 'active'
          ORDER BY b.check_in_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đọc được lượt lưu trú: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| {
            let check_in_raw: String = r.get("check_in_at");
            StayInfo {
                stay_id: r.get("stay_id"),
                room_no: strip_room_prefix(&r.get::<String, _>("room_name")),
                check_in: booking_ts_to_iso_date(&check_in_raw).unwrap_or_default(),
                expected_out: booking_ts_to_iso_date(&r.get::<String, _>("expected_checkout"))
                    .unwrap_or_default(),
                actual_out: r
                    .get::<Option<String>, _>("actual_checkout")
                    .and_then(|s| booking_ts_to_iso_date(&s)),
                check_in_raw,
            }
        })
        .collect())
}

/// Khách chưa khai = lượt lưu trú không có link nào thuộc lô `verified`.
/// Tính bằng query, không cần cột mới ở bảng cũ.
pub async fn count_undeclared_within_48h(pool: &Pool<Sqlite>) -> Result<i64, String> {
    let rows = sqlx::query(
        "SELECT
            (SELECT COUNT(*) FROM booking_guests bg WHERE bg.booking_id = b.id) AS guest_count,
            (SELECT COUNT(*) FROM declaration_link dl
               JOIN declaration_entry de  ON de.link_id = dl.id
               JOIN declaration_batch dbt ON dbt.id     = de.batch_id
              WHERE dl.stay_id = b.id AND dbt.status = 'verified') AS declared_count
           FROM bookings b
          WHERE b.status = 'active'
            AND julianday('now') - julianday(b.check_in_at) <= 2",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đếm được khách chưa khai: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| {
            let g: i64 = r.get("guest_count");
            let d: i64 = r.get("declared_count");
            (g - d).max(0)
        })
        .sum())
}
```

Các hàm còn lại chỉ chạm bốn bảng `declaration_*`, viết theo dạng
`sqlx::query(...).bind(...).execute(pool)` như các repo khác. Chữ ký chính xác —
Kế hoạch B gọi đúng những tên này, đừng đổi:

```rust
pub async fn insert_identity(
    pool: &Pool<Sqlite>,
    identity: &crate::declaration::model::Identity,
    source: &str,
    confidence: &str,
) -> Result<String, String>;

pub async fn insert_link(
    pool: &Pool<Sqlite>,
    identity_id: &str,
    stay_id: &str,
    stay_reason: &str,
    note: Option<&str>,
) -> Result<String, String>;

pub async fn insert_batch(
    pool: &Pool<Sqlite>,
    kind: &str,
    file_path: &str,
    row_count: i64,
) -> Result<String, String>;

pub async fn insert_entries(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    link_ids: &[String],
) -> Result<(), String>;

pub async fn batch_row_count(pool: &Pool<Sqlite>, batch_id: &str) -> Result<i64, String>;

pub async fn set_batch_verified(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    seen: i64,
) -> Result<(), String>;

pub async fn set_batch_failed(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    seen: i64,
) -> Result<(), String>;

/// Dựng DeclarationRow đầy đủ: join declaration_link + declaration_identity,
/// rồi ghép StayInfo tương ứng từ `load_stays_for_declaration`.
pub async fn load_rows_by_link_ids(
    pool: &Pool<Sqlite>,
    link_ids: &[String],
) -> Result<Vec<crate::declaration::model::DeclarationRow>, String>;

/// Trả kèm extract_confidence để lớp command sinh W05 (DeclarationRow không
/// mang cột này).
pub async fn confidence_by_link(
    pool: &Pool<Sqlite>,
    link_ids: &[String],
) -> Result<std::collections::HashMap<String, String>, String>;

// Ba khóa settings của §4.2
pub async fn export_dir(pool: &Pool<Sqlite>) -> Result<std::path::PathBuf, String>;
pub async fn cslt_name(pool: &Pool<Sqlite>) -> Result<String, String>;
pub async fn xml_lead_example(pool: &Pool<Sqlite>) -> Result<bool, String>;

/// §12.5 — che, KHÔNG xóa. Xóa dòng sẽ phá declaration_link ->
/// declaration_entry -> lịch sử lô, tức là mất bằng chứng "khách này đã khai
/// ngày nào, lô nào" — mà đó chính là thứ cần giữ khi có ai hỏi.
pub async fn redact_old_identities(
    pool: &Pool<Sqlite>,
    after_days: i64,
) -> Result<u64, String>;
```

`export_dir` mặc định `app_identity::exports_dir().join("khai-bao-tam-tru")`;
`cslt_name` mặc định `"CSLT"`; `xml_lead_example` mặc định `false`.

`redact_old_identities` chạy `UPDATE declaration_identity SET full_name='',
dob='', doc_no=NULL, passport_no=NULL, address_detail=NULL, phone=NULL,
redacted_at=? WHERE redacted_at IS NULL AND id IN (...)` với danh sách id là
những danh tính mà **mọi** link của nó đều thuộc lô `verified` cũ hơn
`after_days` ngày.

- [ ] **Step 5: Chạy test**

Run: `cd mhm/src-tauri && cargo test declaration::repo`
Expected: PASS, 2 test ranh giới.

Run: `cd mhm/src-tauri && cargo build`
Expected: thành công.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src/db mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): add declaration tables and read-only PMS access

Migration v20 is pure CREATE TABLE. A source-level test fails the build if
anything under declaration/ ever writes to guests, bookings, booking_guests
or rooms.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 11: CLI harness `kbtt_probe`

**Files:**
- Create: `mhm/src-tauri/src/bin/kbtt_probe.rs`
- Create: `mhm/src-tauri/src/declaration/extractor/ocr_rs_mrz.rs`

**Interfaces:**
- Consumes: toàn bộ extractor
- Produces: binary in JSON các trường trích được + điểm checksum, để đo trên ảnh thật

- [ ] **Step 1: Viết implementation `OcrRsMrz`**

`extractor/ocr_rs_mrz.rs` — bọc engine `ocr-rs` sẵn có trong repo sau trait `MrzOcr`:

```rust
use super::mrz::MrzOcr;
use ocr_rs::OcrEngine;

pub struct OcrRsMrz {
    engine: OcrEngine,
}

impl OcrRsMrz {
    pub fn new() -> Result<Self, String> {
        Ok(OcrRsMrz {
            engine: crate::ocr::create_engine()?,
        })
    }
}

impl MrzOcr for OcrRsMrz {
    fn recognize_lines(&self, img: &image::DynamicImage) -> Vec<String> {
        match self.engine.recognize(img) {
            Ok(results) => results
                .iter()
                .map(|r| r.text.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
```

- [ ] **Step 2: Viết CLI**

```rust
//! Đo extractor trên ảnh giấy tờ thật.
//!
//! Chạy: cargo run --bin kbtt_probe -- <đường-dẫn-ảnh> [thêm ảnh...]
//!
//! Gate của Bước 1: QR CCCD phải ra đúng 7 trường; MRZ phải đạt >=3/5
//! checksum trên ảnh chụp bình thường.
//!
//! KHÔNG in payload thô và KHÔNG in đường dẫn ảnh đầy đủ — đó là dữ liệu cá
//! nhân (§12.3). Chỉ in tên file và các trường đã parse.

use capyinn_lib::declaration::extractor::{
    mrz::MrzExtractor, ocr_rs_mrz::OcrRsMrz, qr_cccd::QrCccdExtractor, IdentityExtractor,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Dùng: kbtt_probe <ảnh> [ảnh...]");
        std::process::exit(2);
    }

    let today_year = 2026;
    let mrz_ocr = match OcrRsMrz::new() {
        Ok(o) => Some(o),
        Err(e) => {
            eprintln!("Không khởi tạo được OCR: {e}");
            None
        }
    };

    let mut qr_ok = 0;
    let mut mrz_ok = 0;
    let mut total = 0;

    for path in &args {
        total += 1;
        let name = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "?".into());

        let img = match image::open(path) {
            Ok(i) => i,
            Err(e) => {
                println!("{name}: KHÔNG MỞ ĐƯỢC ({e})");
                continue;
            }
        };

        if let Some(res) = QrCccdExtractor.try_extract(&img) {
            qr_ok += 1;
            let id = res.identity;
            println!(
                "{name}: QR_CCCD ok | tên={} | sinh={} | giới={} | loại={:?} ({:?})",
                id.full_name, id.dob, id.gender, id.doc_type_code, id.doc_type_source
            );
            continue;
        }

        if let Some(ocr) = &mrz_ocr {
            let extractor = MrzExtractor::new(
                OcrRsMrz::new().unwrap_or_else(|_| unreachable!()),
                today_year,
            );
            let _ = ocr;
            if let Some(res) = extractor.try_extract(&img) {
                mrz_ok += 1;
                let id = res.identity;
                println!(
                    "{name}: MRZ_TD3 {:?} | tên={} | HC={:?} | quốc tịch={} | sinh={} | HC hết hạn={:?} | tên đã xác nhận={}",
                    res.confidence,
                    id.full_name,
                    id.passport_no,
                    id.nationality_iso3,
                    id.dob,
                    id.passport_expiry,
                    id.name_confirmed_by_human
                );
                continue;
            }
        }

        println!("{name}: KHÔNG TRÍCH ĐƯỢC — cần nhập tay");
    }

    println!("\n--- Tổng kết ---");
    println!("Ảnh: {total} | QR ok: {qr_ok} | MRZ ok: {mrz_ok}");
    println!("Gate Bước 1: QR ra đúng 7 trường, MRZ >=3/5 checksum.");
    if qr_ok + mrz_ok == 0 {
        println!("CHƯA ĐẠT GATE — nếu MRZ trượt hết, cân nhắc bật feature mrz-tesseract.");
    }
}
```

- [ ] **Step 3: Xác nhận biên dịch**

Run: `cd mhm/src-tauri && cargo build --bin kbtt_probe`
Expected: thành công.

- [ ] **Step 4: Chạy thử trên ảnh thật (cần ảnh của người vận hành)**

Run: `cd mhm/src-tauri && cargo run --bin kbtt_probe -- ~/CapyInn/Scans/*.jpg`

Đây là **gate nghiệm thu của Kế hoạch A**. Nếu chưa có ảnh giấy tờ thật thì
ghi rõ trong báo cáo là gate này CHƯA đo được — không được coi là đã đạt.

- [ ] **Step 5: Commit**

```bash
git add mhm/src-tauri/src/bin/kbtt_probe.rs mhm/src-tauri/src/declaration
git commit -m "feat(kbtt): add CLI probe to measure extractors on real photos

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Nghiệm thu Kế hoạch A

- [ ] `cargo test declaration` — toàn bộ xanh
- [ ] `cargo build` — không warning mới
- [ ] `cargo clippy --all-targets` — không lỗi mới
- [ ] Test âm `never_writes_over_the_example_row` xanh
- [ ] Test `perfect_checksum_never_vouches_for_the_name` xanh
- [ ] Test ranh giới `declaration_module_never_writes_to_legacy_tables` xanh
- [ ] `kbtt_probe` chạy được trên ảnh thật và đạt gate — **hoặc ghi rõ là chưa đo được vì thiếu ảnh**
- [ ] Xuất thử một file XLSX và một file XML để người vận hành upload lên cổng
