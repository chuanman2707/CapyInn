# Khai báo tạm trú — Kế hoạch B: UI và vòng đối chiếu

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Đưa lõi của Kế hoạch A ra màn hình — kéo-thả ảnh, ghép khách, xuất file, và ép người vận hành đối chiếu số hồ sơ thật trên cổng trước khi coi một lô là xong.

**Architecture:** `commands/declaration.rs` là lớp Tauri mỏng, không chứa logic — nó dựng `DeclarationRow` từ DB rồi gọi thẳng `validator` và `writer` của Kế hoạch A. Frontend là một trang mới bốn khối, cộng một badge sidebar và một thẻ Dashboard dùng lại cùng một truy vấn.

**Tech Stack:** React 18, Zustand (`useHotelStore`), Tailwind, shadcn/ui, `@tauri-apps/api` (`invoke`, `listen`), Vitest.

**Điều kiện tiên quyết:** Kế hoạch A đã hoàn tất và `cargo test declaration` xanh.

**Spec:** `docs/superpowers/specs/2026-07-26-khai-bao-tam-tru-design.md`

## Global Constraints

- **Không sửa gì trên màn check-in hiện tại** (`CheckinSheet.tsx`). Đây là điểm khác lớn nhất so với spec v1 và là thứ giữ cho PMS đang chạy không bị động vào.
- **Không đụng `watcher.rs` / thư mục `Scans/`.** Đường máy scan hiện tại nuôi `CheckinSheet`, để nguyên.
- **Ảnh không bao giờ được copy vào storage của app.** Đọc từ đường dẫn tạm, trích xuất, rồi bỏ.
- **Không log payload QR/MRZ, không log đường dẫn ảnh** ở production build.
- Nút `Xuất file` **disabled khi còn lỗi blocking**. Không có đường vòng.
- Vòng đối chiếu **không được có nút một-cú-bấm**. Con số phải được gõ vào.
- Nhãn tab tiếng Việt (`Khai báo tạm trú`) — thuật ngữ pháp lý, không dịch.
- Test frontend: `cd mhm && npx vitest run <đường dẫn>`.
- Mọi commit kết thúc bằng `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.

---

## File Structure

| File | Trách nhiệm |
|---|---|
| `mhm/src-tauri/src/commands/declaration.rs` | Lớp Tauri mỏng: 8 command |
| `mhm/src-tauri/src/lib.rs` | Đăng ký command |
| `mhm/src/types/index.ts` | Thêm `"declaration"` vào `HotelTab`, thêm type của module |
| `mhm/src/app/MainShell.tsx` | Nav item + badge + route trang |
| `mhm/src/pages/Declaration/index.tsx` | Khung trang, 4 khối |
| `mhm/src/pages/Declaration/DropZone.tsx` | Khối 1 — kéo-thả ảnh |
| `mhm/src/pages/Declaration/IdentityCard.tsx` | Thẻ kết quả trích xuất + xác nhận tên |
| `mhm/src/pages/Declaration/PendingList.tsx` | Khối 2 — danh sách cần khai |
| `mhm/src/pages/Declaration/ExportPanel.tsx` | Khối 3 — kiểm tra và xuất |
| `mhm/src/pages/Declaration/ReconcileChecklist.tsx` | Vòng đối chiếu |
| `mhm/src/pages/Declaration/BatchHistory.tsx` | Khối 4 — lịch sử lô |
| `mhm/src/pages/Dashboard.tsx` | Thêm thẻ "Chưa khai báo tạm trú" |

---

## Task 12: Lớp command Tauri

**Files:**
- Create: `mhm/src-tauri/src/commands/declaration.rs`
- Modify: `mhm/src-tauri/src/commands/mod.rs`, `mhm/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `declaration::{catalog, extractor, model, repo, validator, writer}` từ Kế hoạch A
- Produces (tất cả `#[tauri::command]`):
  - `kbtt_extract_from_image(path: String) -> Result<ExtractedDto, String>`
  - `kbtt_list_stays() -> Result<Vec<StayInfo>, String>`
  - `kbtt_save_identity(identity: Identity, source: String, confidence: String) -> Result<String, String>`
  - `kbtt_link(identity_id: String, stay_id: String, stay_reason: String, note: Option<String>) -> Result<String, String>`
  - `kbtt_pending_rows() -> Result<Vec<DeclarationRow>, String>`
  - `kbtt_validate(link_ids: Vec<String>) -> Result<Vec<Finding>, String>`
  - `kbtt_export(kind: String, link_ids: Vec<String>) -> Result<BatchDto, String>`
  - `kbtt_reconcile(batch_id: String, seen_count: i64) -> Result<String, String>`
  - `kbtt_undeclared_count() -> Result<i64, String>`
  - `kbtt_open_export_dir(batch_id: String) -> Result<(), String>`

- [ ] **Step 1: Viết `ExtractedDto` và command trích xuất**

`crop_for_review` là ảnh trong RAM. Gửi sang frontend dưới dạng data URL base64 **trong response**, không ghi ra đĩa và không lưu vào DB.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ExtractedDto {
    pub source: String,
    pub confidence: String,
    pub identity: crate::declaration::model::Identity,
    pub review_hints: Vec<crate::declaration::model::Field>,
    /// data:image/png;base64,... — chỉ đi qua IPC, KHÔNG ghi đĩa (§12.4)
    pub crop_data_url: Option<String>,
}

#[tauri::command]
pub async fn kbtt_extract_from_image(path: String) -> Result<ExtractedDto, String> {
    use crate::declaration::extractor::{
        mrz::MrzExtractor, ocr_rs_mrz::OcrRsMrz, qr_cccd::QrCccdExtractor, IdentityExtractor,
    };

    // KHÔNG log `path` — đường dẫn ảnh là dữ liệu cá nhân (§12.3)
    let img = image::open(&path).map_err(|_| "Không mở được ảnh".to_string())?;

    let result = QrCccdExtractor.try_extract(&img).or_else(|| {
        let year = chrono::Local::now().format("%Y").to_string().parse().unwrap_or(2026);
        OcrRsMrz::new()
            .ok()
            .and_then(|ocr| MrzExtractor::new(ocr, year).try_extract(&img))
    });

    let res = result.ok_or_else(|| {
        "Không đọc được QR hay MRZ trong ảnh. Dùng form nhập tay.".to_string()
    })?;

    let crop_data_url = res.crop_for_review.as_ref().and_then(|c| {
        let mut buf = std::io::Cursor::new(Vec::new());
        c.write_to(&mut buf, image::ImageFormat::Png).ok()?;
        use base64::Engine;
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
        ))
    });

    Ok(ExtractedDto {
        source: res.source.as_db().to_string(),
        confidence: res.confidence.as_db().to_string(),
        identity: res.identity,
        review_hints: res.review_hints,
        crop_data_url,
    })
}
```

Thêm `base64 = "0.22"` vào `Cargo.toml`.

- [ ] **Step 2: Viết command xuất file**

Đây là chỗ `W05` được sinh — `DeclarationRow` không mang `extract_confidence`, nên
lớp này đọc cột đó từ `declaration_identity` và thêm finding trước khi trả về.

```rust
#[derive(Debug, Serialize)]
pub struct BatchDto {
    pub batch_id: String,
    pub file_path: String,
    pub row_count: usize,
    pub kind: String,
}

#[tauri::command]
pub async fn kbtt_export(
    state: tauri::State<'_, crate::AppState>,
    app: tauri::AppHandle,
    kind: String,
    link_ids: Vec<String>,
) -> Result<BatchDto, String> {
    use crate::declaration::{catalog::Catalog, validator, writer};

    let pool = &state.db;
    let catalog = Catalog::load()?;
    let rows = crate::declaration::repo::load_rows_by_link_ids(&pool, &link_ids).await?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let findings = validator::validate(&rows, &catalog, &today);
    if validator::has_blocking(&findings) {
        let codes: Vec<String> = findings
            .iter()
            .filter(|f| f.severity == crate::declaration::model::Severity::Blocking)
            .map(|f| f.code.clone())
            .collect();
        return Err(format!(
            "Còn lỗi chặn, không xuất được: {}",
            codes.join(", ")
        ));
    }

    let dir = crate::declaration::repo::export_dir(&pool).await?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Không tạo được thư mục xuất: {e}"))?;
    let cslt = crate::declaration::repo::cslt_name(&pool).await?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M").to_string();

    let file_path = match kind.as_str() {
        "NNN" => {
            let lead = crate::declaration::repo::xml_lead_example(&pool).await?;
            let xml = writer::xml::render(&rows, lead)?;
            let p = dir.join(format!("KBTT_{cslt}_{stamp}.xml"));
            std::fs::write(&p, xml).map_err(|e| format!("Không ghi được XML: {e}"))?;
            p
        }
        "VN" => {
            let template = crate::declaration::find_kbtt_resource("tblt_vn_import.xlsx")?;
            let p = dir.join(format!("TBLT_{cslt}_{stamp}.xlsx"));
            // write_batch tự chạy gate 7 assert và tự xóa file nếu fail
            writer::xlsx::write_batch(&rows, &catalog, &template, &p)?;
            p
        }
        other => return Err(format!("Loại lô không hợp lệ: {other}")),
    };

    let batch_id = crate::declaration::repo::insert_batch(
        &pool,
        &kind,
        &file_path.to_string_lossy(),
        rows.len() as i64,
    )
    .await?;
    crate::declaration::repo::insert_entries(&pool, &batch_id, &link_ids).await?;

    let _ = app; // dùng cho opener ở command riêng
    Ok(BatchDto {
        batch_id,
        file_path: file_path.to_string_lossy().to_string(),
        row_count: rows.len(),
        kind,
    })
}
```

- [ ] **Step 3: Viết command đối chiếu**

Đây là nơi F3 được chặn. Số gõ vào khớp thì `verified`, lệch thì `failed`.

```rust
#[tauri::command]
pub async fn kbtt_reconcile(
    state: tauri::State<'_, crate::AppState>,
    batch_id: String,
    seen_count: i64,
) -> Result<String, String> {
    let pool = &state.db;
    let expected = crate::declaration::repo::batch_row_count(&pool, &batch_id).await?;

    if seen_count == expected {
        crate::declaration::repo::set_batch_verified(&pool, &batch_id, seen_count).await?;
        Ok("verified".to_string())
    } else {
        // Lô failed KHÔNG có declaration_entry nào thuộc lô verified, nên khách
        // tự động giữ nguyên trạng thái chưa khai. Không cần code hoàn tác.
        crate::declaration::repo::set_batch_failed(&pool, &batch_id, seen_count).await?;
        Ok("failed".to_string())
    }
}
```

- [ ] **Step 4: Đăng ký command**

Thêm `pub mod declaration;` vào `commands/mod.rs`, và thêm 10 command vào
`tauri::generate_handler![...]` trong `lib.rs`, dưới một comment nhóm mới:

```rust
            // Khai báo tạm trú
            commands::declaration::kbtt_extract_from_image,
            commands::declaration::kbtt_list_stays,
            commands::declaration::kbtt_save_identity,
            commands::declaration::kbtt_link,
            commands::declaration::kbtt_pending_rows,
            commands::declaration::kbtt_validate,
            commands::declaration::kbtt_export,
            commands::declaration::kbtt_reconcile,
            commands::declaration::kbtt_undeclared_count,
            commands::declaration::kbtt_open_export_dir,
```

- [ ] **Step 5: Xác nhận biên dịch và test**

Run: `cd mhm/src-tauri && cargo build && cargo test declaration`
Expected: build thành công, test của Kế hoạch A vẫn xanh.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src/commands mhm/src-tauri/src/lib.rs mhm/src-tauri/Cargo.toml
git commit -m "feat(kbtt): expose declaration commands to the frontend

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 13: Tab sidebar, badge và khung trang

**Files:**
- Modify: `mhm/src/types/index.ts`, `mhm/src/app/MainShell.tsx`
- Create: `mhm/src/pages/Declaration/index.tsx`
- Create: `mhm/src/pages/Declaration/Declaration.test.tsx`

- [ ] **Step 1: Viết test thất bại**

`mhm/src/pages/Declaration/Declaration.test.tsx`:

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import Declaration from "./index";

describe("Declaration page", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_undeclared_count") return Promise.resolve(0);
      return Promise.resolve([]);
    });
  });

  it("renders all four blocks", async () => {
    render(<Declaration />);
    await waitFor(() => {
      expect(screen.getByText(/Kéo ảnh giấy tờ vào đây/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/Cần khai báo/i)).toBeInTheDocument();
    expect(screen.getByText(/Xuất file/i)).toBeInTheDocument();
    expect(screen.getByText(/Lịch sử lô/i)).toBeInTheDocument();
  });

  it("always shows the Excel warning", async () => {
    render(<Declaration />);
    await waitFor(() => {
      expect(
        screen.getByText(/Không mở\/sửa file này bằng Excel trước khi upload/i),
      ).toBeInTheDocument();
    });
  });
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Declaration/Declaration.test.tsx`
Expected: FAIL — không tìm thấy module `./index`.

- [ ] **Step 3: Thêm tab vào type**

`mhm/src/types/index.ts`:

```ts
export type HotelTab =
  | "dashboard"
  | "rooms"
  | "reservations"
  | "guests"
  | "groups"
  | "housekeeping"
  | "analytics"
  | "settings"
  | "declaration"
  | "audit";
```

- [ ] **Step 4: Thêm nav item, badge và route**

`mhm/src/app/MainShell.tsx`:

Thêm `ShieldCheck` vào import từ `lucide-react`, thêm `import Declaration from "@/pages/Declaration";`, rồi:

```ts
const NAV_MANAGEMENT = [
  { key: "housekeeping" as const, label: "Housekeeping", icon: Sparkles },
  { key: "analytics" as const, label: "Analytics", icon: BarChart3 },
  { key: "audit" as const, label: "Night Audit", icon: Moon },
  { key: "declaration" as const, label: "Khai báo tạm trú", icon: ShieldCheck },
];
```

Thêm vào `PAGE_TITLES`: `declaration: "Khai báo tạm trú",`

Thêm route cạnh các dòng khác: `{activeTab === "declaration" && <Declaration />}`

Badge số đỏ trên nav item — thêm state trong `MainShell`:

```tsx
const [undeclared, setUndeclared] = useState(0);

useEffect(() => {
  const load = () => {
    invoke<number>("kbtt_undeclared_count").then(setUndeclared).catch(() => {});
  };
  load();
  const timer = setInterval(load, 60_000);
  return () => clearInterval(timer);
}, []);
```

Trong `renderNavItem`, hiện badge khi `item.key === "declaration" && undeclared > 0`:

```tsx
{item.key === "declaration" && undeclared > 0 && !collapsed && (
  <Badge variant="destructive" className="ml-auto">
    {undeclared}
  </Badge>
)}
```

- [ ] **Step 5: Viết khung trang**

`mhm/src/pages/Declaration/index.tsx` — bốn khối, mỗi khối là một component riêng
(Task 14–17 điền ruột). Cảnh báo Excel là text cố định, không có nút đóng:

```tsx
export default function Declaration() {
  return (
    <div className="flex flex-col gap-6 p-6">
      <DropZone />
      <PendingList />
      <div className="rounded-xl border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900">
        <strong>Không mở/sửa file này bằng Excel trước khi upload.</strong> Excel
        sẽ làm mất số 0 đầu của số giấy tờ và đổi định dạng ngày. Cần sửa thì sửa
        trong CapyInn rồi xuất lại.
      </div>
      <ExportPanel />
      <BatchHistory />
    </div>
  );
}
```

- [ ] **Step 6: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Declaration/Declaration.test.tsx`
Expected: PASS, 2 test.

- [ ] **Step 7: Commit**

```bash
git add mhm/src
git commit -m "feat(kbtt): add declaration tab with undeclared badge

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 14: Khối 1 — kéo-thả ảnh và thẻ kết quả

**Files:**
- Create: `mhm/src/pages/Declaration/DropZone.tsx`
- Create: `mhm/src/pages/Declaration/IdentityCard.tsx`
- Create: `mhm/src/pages/Declaration/IdentityCard.test.tsx`

**Interfaces:**
- Consumes: command `kbtt_extract_from_image`
- Produces: `type Extracted = { source; confidence; identity; review_hints; crop_data_url }`

- [ ] **Step 1: Viết test thất bại**

`IdentityCard.test.tsx` — bài test quan trọng nhất của cả kế hoạch B là chỗ crop MRZ:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import IdentityCard from "./IdentityCard";

const mrzExtract = {
  source: "mrz_td3",
  confidence: "verified",
  identity: {
    id: "i1",
    full_name: "ZOLOCHEVSKAIA VERONIKA",
    dob: "1990-03-08",
    gender: "F",
    nationality_iso3: "RUS",
    passport_no: "777785671",
    passport_expiry: "2035-12-09",
    name_confirmed_by_human: false,
  },
  review_hints: ["full_name"],
  crop_data_url: "data:image/png;base64,AAAA",
};

describe("IdentityCard", () => {
  it("shows the MRZ crop right next to the name field", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);
    const crop = screen.getByAltText(/hai dòng MRZ/i);
    const nameInput = screen.getByLabelText(/Họ và tên/i);
    // cùng một khối để mắt đối chiếu trong một giây — E04 mất ý nghĩa nếu
    // người dùng phải cuộn hay bấm mở mới thấy
    expect(crop.closest("[data-mrz-review]")).toBe(
      nameInput.closest("[data-mrz-review]"),
    );
    expect(crop.closest("[data-mrz-review]")).not.toBeNull();
  });

  it("requires an explicit click to confirm the name", () => {
    const onChange = vi.fn();
    render(<IdentityCard extracted={mrzExtract} onChange={onChange} />);
    expect(screen.getByText(/Chưa xác nhận tên/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Xác nhận tên/i }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ name_confirmed_by_human: true }),
    );
  });

  it("never pre-confirms the name even at full checksum", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);
    // confidence là "verified" nhưng tên vẫn phải chờ người
    expect(screen.getByText(/Chưa xác nhận tên/i)).toBeInTheDocument();
  });

  it("offers the single-token override for mononyms", () => {
    const mono = {
      ...mrzExtract,
      identity: { ...mrzExtract.identity, full_name: "SUHARTO" },
    };
    const onChange = vi.fn();
    render(<IdentityCard extracted={mono} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText(/chỉ có một chữ/i));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ single_token_name_ok: true }),
    );
  });

  it("asks for the residence deadline on foreign guests only", () => {
    render(<IdentityCard extracted={mrzExtract} onChange={vi.fn()} />);
    expect(screen.getByLabelText(/Thời hạn tạm trú/i)).toBeInTheDocument();

    const vn = {
      ...mrzExtract,
      source: "qr_cccd",
      identity: { ...mrzExtract.identity, nationality_iso3: "VNM" },
    };
    render(<IdentityCard extracted={vn} onChange={vi.fn()} />);
    expect(screen.queryAllByLabelText(/Thời hạn tạm trú/i)).toHaveLength(1);
  });
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Declaration/IdentityCard.test.tsx`
Expected: FAIL — chưa có component.

- [ ] **Step 3: Viết `IdentityCard`**

Điểm bắt buộc: `<img>` crop MRZ và `<input>` tên nằm **trong cùng một phần tử
`data-mrz-review`**, cạnh nhau theo chiều ngang.

```tsx
export default function IdentityCard({ extracted, onChange }: Props) {
  const id = extracted.identity;
  const isForeign = id.nationality_iso3 !== "VNM";

  return (
    <div className="rounded-xl border bg-white p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="text-xs font-semibold uppercase text-slate-500">
          {extracted.source === "qr_cccd" ? "QR CCCD" :
           extracted.source === "mrz_td3" ? "MRZ hộ chiếu" : "Nhập tay"}
        </span>
        {extracted.confidence === "verified"
          ? <Badge variant="secondary">✓ Verified</Badge>
          : <Badge variant="destructive">⚠ Cần xác nhận</Badge>}
      </div>

      <div data-mrz-review className="mb-3 flex items-start gap-4">
        <div className="flex-1">
          <label htmlFor={`name-${id.id}`} className="text-xs text-slate-600">
            Họ và tên
          </label>
          <input
            id={`name-${id.id}`}
            className="w-full rounded-lg border px-3 py-2 font-mono"
            value={id.full_name}
            onChange={(e) => onChange({ ...id, full_name: e.target.value })}
          />
          {!id.name_confirmed_by_human && (
            <p className="mt-1 text-xs text-amber-700">
              Chưa xác nhận tên — dòng 1 của MRZ không có checksum bảo vệ.
            </p>
          )}
        </div>
        {extracted.crop_data_url && (
          <img
            src={extracted.crop_data_url}
            alt="Ảnh cắt hai dòng MRZ để đối chiếu"
            className="h-16 rounded border object-contain"
          />
        )}
      </div>

      <Button
        variant={id.name_confirmed_by_human ? "secondary" : "default"}
        onClick={() => onChange({ ...id, name_confirmed_by_human: true })}
      >
        Xác nhận tên
      </Button>

      <label className="mt-3 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={id.single_token_name_ok ?? false}
          onChange={(e) =>
            onChange({ ...id, single_token_name_ok: e.target.checked })
          }
        />
        Tên trên giấy tờ chỉ có một chữ
      </label>

      {isForeign && (
        <div className="mt-3">
          <label htmlFor={`visa-${id.id}`} className="text-xs text-slate-600">
            Thời hạn tạm trú (bắt buộc, nhập tay)
          </label>
          <input
            id={`visa-${id.id}`}
            type="date"
            className="w-full rounded-lg border px-3 py-2"
            value={id.visa_valid_until ?? ""}
            onChange={(e) => onChange({ ...id, visa_valid_until: e.target.value })}
          />
          <p className="mt-1 text-xs text-slate-500">
            Không phải ngày hết hạn hộ chiếu ({id.passport_expiry ?? "—"}).
          </p>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Viết `DropZone`**

Dùng Tauri drag-drop event. Ảnh chỉ được đọc từ đường dẫn tạm rồi bỏ — **không
copy vào storage của app**.

```tsx
useEffect(() => {
  const un = getCurrentWebview().onDragDropEvent(async (event) => {
    if (event.payload.type !== "drop") return;
    for (const path of event.payload.paths) {
      try {
        const res = await invoke<Extracted>("kbtt_extract_from_image", { path });
        setCards((c) => [...c, res]);
      } catch (e) {
        toast.error(String(e));
      }
    }
  });
  return () => { void un.then((f) => f()); };
}, []);
```

- [ ] **Step 5: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Declaration/IdentityCard.test.tsx`
Expected: PASS, 5 test.

- [ ] **Step 6: Commit**

```bash
git add mhm/src/pages/Declaration
git commit -m "feat(kbtt): drag-drop ID photos and review extracted identities

The MRZ crop sits beside the name field on purpose. E04 only means
something if confirming the name requires actually looking at it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 15: Khối 2 — ghép khách và danh sách cần khai

**Files:**
- Create: `mhm/src/pages/Declaration/PendingList.tsx`
- Create: `mhm/src/pages/Declaration/PendingList.test.tsx`

- [ ] **Step 1: Viết test thất bại**

```tsx
it("suggests bookings by name similarity but never auto-confirms", async () => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_list_stays")
      return Promise.resolve([
        { stay_id: "b1", room_no: "5A", check_in: "2026-07-25", expected_out: "2026-08-03" },
        { stay_id: "b2", room_no: "5B", check_in: "2026-07-25", expected_out: "2026-07-26" },
      ]);
    return Promise.resolve([]);
  });

  render(<PendingList identity={{ full_name: "ZOLOCHEVSKAIA VERONIKA" }} />);
  const select = await screen.findByLabelText(/Ghép với khách đang ở/i);
  // app gợi ý thứ tự, nhưng không tự chọn giúp
  expect((select as HTMLSelectElement).value).toBe("");
});

it("groups rows by NNN and VN because they export to different files", async () => {
  // ...render với một khách VNM và một khách RUS
  expect(screen.getByText(/Khách nước ngoài \(XML\)/i)).toBeInTheDocument();
  expect(screen.getByText(/Khách Việt Nam \(XLSX\)/i)).toBeInTheDocument();
});

it("shows blocking error codes on the row", async () => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_validate")
      return Promise.resolve([
        { code: "E04", severity: "blocking", link_id: "l1", message: "Chưa ai xác nhận tên" },
      ]);
    return Promise.resolve([]);
  });
  render(<PendingList />);
  expect(await screen.findByText("E04")).toBeInTheDocument();
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Declaration/PendingList.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Viết hàm xếp hạng tên và component**

Chuẩn hóa: bỏ dấu, lowercase, so token. Tên trong CapyInn có thể chỉ là `Andrei`.

```ts
export function normaliseName(s: string): string[] {
  return s
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/đ/gi, "d")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

export function nameScore(a: string, b: string): number {
  const ta = new Set(normaliseName(a));
  const tb = normaliseName(b);
  if (tb.length === 0) return 0;
  return tb.filter((t) => ta.has(t)).length / tb.length;
}
```

Dropdown mặc định `value=""` — người chọn, app chỉ sắp thứ tự. Nhóm NNN/VN theo
`nationality_iso3 === "VNM"`.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Declaration/PendingList.test.tsx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mhm/src/pages/Declaration
git commit -m "feat(kbtt): rank booking matches without auto-confirming them

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 16: Khối 3 — kiểm tra và xuất file

**Files:**
- Create: `mhm/src/pages/Declaration/ExportPanel.tsx`
- Create: `mhm/src/pages/Declaration/ExportPanel.test.tsx`

- [ ] **Step 1: Viết test thất bại**

```tsx
it("disables export while any blocking finding remains", async () => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_validate")
      return Promise.resolve([{ code: "E08", severity: "blocking", link_id: "l1", message: "x" }]);
    return Promise.resolve([]);
  });
  render(<ExportPanel linkIds={["l1"]} kind="NNN" />);
  fireEvent.click(screen.getByRole("button", { name: /Kiểm tra/i }));
  await waitFor(() => {
    expect(screen.getByRole("button", { name: /Xuất file/i })).toBeDisabled();
  });
});

it("allows export when only warnings remain", async () => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_validate")
      return Promise.resolve([{ code: "W03", severity: "warning", link_id: "l1", message: "x" }]);
    return Promise.resolve([]);
  });
  render(<ExportPanel linkIds={["l1"]} kind="VN" />);
  fireEvent.click(screen.getByRole("button", { name: /Kiểm tra/i }));
  await waitFor(() => {
    expect(screen.getByRole("button", { name: /Xuất file/i })).toBeEnabled();
  });
});

it("shows the reconcile checklist after a successful export", async () => {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_validate") return Promise.resolve([]);
    if (cmd === "kbtt_export")
      return Promise.resolve({ batch_id: "b1", file_path: "/x/y.xlsx", row_count: 3, kind: "VN" });
    return Promise.resolve([]);
  });
  render(<ExportPanel linkIds={["l1", "l2", "l3"]} kind="VN" />);
  fireEvent.click(screen.getByRole("button", { name: /Xuất file/i }));
  expect(await screen.findByText(/Số hồ sơ thấy trên cổng/i)).toBeInTheDocument();
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Declaration/ExportPanel.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Viết component**

`disabled={findings.some(f => f.severity === "blocking")}` trên nút `Xuất file`.
Xuất xong gọi `kbtt_open_export_dir` và render `<ReconcileChecklist />`.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Declaration/ExportPanel.test.tsx`
Expected: PASS, 3 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src/pages/Declaration
git commit -m "feat(kbtt): gate export on blocking findings

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 17: Vòng đối chiếu và lịch sử lô

**Files:**
- Create: `mhm/src/pages/Declaration/ReconcileChecklist.tsx`
- Create: `mhm/src/pages/Declaration/ReconcileChecklist.test.tsx`
- Create: `mhm/src/pages/Declaration/BatchHistory.tsx`

- [ ] **Step 1: Viết test thất bại**

Đây là chốt chặn cho F3 — cổng báo "thành công" khi import 0 record.

```tsx
it("has no one-click completion button", () => {
  render(<ReconcileChecklist batchId="b1" expected={3} />);
  const buttons = screen.getAllByRole("button").map((b) => b.textContent ?? "");
  expect(buttons.some((t) => /hoàn thành|xong|đã upload xong/i.test(t))).toBe(false);
  // con số phải được gõ vào
  expect(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i)).toBeInTheDocument();
});

it("marks the batch verified when the typed count matches", async () => {
  invokeMock.mockResolvedValue("verified");
  render(<ReconcileChecklist batchId="b1" expected={3} />);
  fireEvent.change(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i), {
    target: { value: "3" },
  });
  fireEvent.click(screen.getByRole("button", { name: /Xác nhận/i }));
  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("kbtt_reconcile", {
      batchId: "b1",
      seenCount: 3,
    });
  });
});

it("marks the batch failed and keeps guests undeclared when the count differs", async () => {
  invokeMock.mockResolvedValue("failed");
  render(<ReconcileChecklist batchId="b1" expected={3} />);
  fireEvent.change(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i), {
    target: { value: "0" },
  });
  fireEvent.click(screen.getByRole("button", { name: /Xác nhận/i }));
  expect(await screen.findByText(/Lô thất bại/i)).toBeInTheDocument();
  expect(screen.getByText(/vẫn ở trạng thái chưa khai báo/i)).toBeInTheDocument();
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Declaration/ReconcileChecklist.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Viết component**

Hai checkbox là nhắc việc, nhưng **nút `Xác nhận` chỉ enable khi ô số đã có giá
trị**. Không có nút nào đánh dấu lô xong mà không cần gõ số.

```tsx
<label htmlFor="seen">Số hồ sơ thấy trên cổng</label>
<input id="seen" type="number" value={seen} onChange={...} />
<span>(cần đúng {expected})</span>
<Button disabled={seen === ""} onClick={submit}>Xác nhận</Button>
```

`BatchHistory` sắp lô `failed` và `uploaded` chưa verified lên đầu, tô màu cảnh báo.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Declaration/ReconcileChecklist.test.tsx`
Expected: PASS, 3 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src/pages/Declaration
git commit -m "feat(kbtt): require a typed record count to close a batch

The portal reports success on a zero-record import. A checkbox gets ticked
by reflex; a number field forces someone to read the screen.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 18: Thẻ Dashboard

**Files:**
- Modify: `mhm/src/pages/Dashboard.tsx`

- [ ] **Step 1: Viết test thất bại**

```tsx
it("shows the undeclared guest count", async () => {
  invokeMock.mockImplementation((cmd: string) =>
    cmd === "kbtt_undeclared_count" ? Promise.resolve(4) : Promise.resolve({}),
  );
  render(<Dashboard />);
  expect(await screen.findByText(/Chưa khai báo tạm trú/i)).toBeInTheDocument();
  expect(await screen.findByText("4")).toBeInTheDocument();
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Dashboard.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Thêm thẻ**

Dùng `StatCard` sẵn có trong `components/shared/StatCard.tsx`, gọi
`kbtt_undeclared_count` — **cùng một truy vấn** với badge sidebar, không viết
truy vấn thứ hai.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Dashboard.test.tsx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mhm/src/pages/Dashboard.tsx
git commit -m "feat(kbtt): surface undeclared guests on the dashboard

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 19: Form nhập tay (§6.4)

**Files:**
- Create: `mhm/src/pages/Declaration/ManualForm.tsx`
- Create: `mhm/src/pages/Declaration/ManualForm.test.tsx`

Spec §6.4 gọi đây là `ManualExtractor`, nhưng nó không nhận ảnh nên implement như
một `IdentityExtractor` trong Rust là gượng ép. Thay vào đó: form dựng `Identity`
ở frontend rồi lưu qua `kbtt_save_identity` với `source="manual"`,
`confidence="needs_review"`. Kết quả trong DB giống hệt điều spec mô tả.

Dùng khi: ảnh không có QR/MRZ (CMND cũ, giấy khai sinh, GPLX), QR/MRZ decode
fail, hoặc người vận hành chủ động chọn.

- [ ] **Step 1: Viết test thất bại**

```tsx
it("saves manual entries as needs_review", async () => {
  invokeMock.mockResolvedValue("new-id");
  render(<ManualForm onSaved={vi.fn()} />);
  fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
    target: { value: "Nguyễn Văn A" },
  });
  fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
    target: { value: "1980-05-02" },
  });
  fireEvent.click(screen.getByRole("button", { name: /Lưu/i }));
  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith(
      "kbtt_save_identity",
      expect.objectContaining({ source: "manual", confidence: "needs_review" }),
    );
  });
});

it("appears when extraction fails", async () => {
  invokeMock.mockRejectedValue("Không đọc được QR hay MRZ trong ảnh");
  render(<DropZone />);
  // sau khi trích xuất thất bại, người dùng phải có đường đi tiếp
  expect(await screen.findByRole("button", { name: /Nhập tay/i })).toBeInTheDocument();
});
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm && npx vitest run src/pages/Declaration/ManualForm.test.tsx`
Expected: FAIL.

- [ ] **Step 3: Viết form**

Các ô: họ tên, ngày sinh, giới tính, quốc tịch (dropdown 205 mã), loại giấy tờ
(dropdown 9 mục), số giấy tờ, điện thoại, nơi cư trú, địa chỉ chi tiết. Với khách
nước ngoài thêm số hộ chiếu, ngày hết hạn hộ chiếu, thời hạn tạm trú.

`doc_type_source = "human"` — người tự chọn thì không phải heuristic, nên `W06`
không nổ.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cd mhm && npx vitest run src/pages/Declaration/ManualForm.test.tsx`
Expected: PASS, 2 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src/pages/Declaration
git commit -m "feat(kbtt): add manual identity entry fallback

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 20: Che dữ liệu cá nhân cũ (§12.5)

**Files:**
- Modify: `mhm/src-tauri/src/lib.rs` (gọi lúc khởi động)
- Modify: `mhm/src/pages/settings/` (thêm ô số ngày)

Nghị định 13/2023 áp dụng. Sau khi lô `verified` + N ngày (mặc định 90), che các
cột định danh và set `redacted_at`.

**Che, không xóa.** Xóa dòng sẽ phá `declaration_link` → `declaration_entry` →
lịch sử lô, tức là mất bằng chứng "khách này đã khai ngày nào, lô nào" — mà đó
chính là thứ cần giữ khi có ai hỏi.

- [ ] **Step 1: Viết test thất bại**

Trong `repo.rs` của Kế hoạch A:

```rust
#[tokio::test]
async fn redaction_keeps_batch_history_intact() {
    let pool = test_pool().await; // dựng DB tạm, chạy migration v20
    // ... seed một identity + link + batch verified cách đây 100 ngày

    let n = redact_old_identities(&pool, 90).await.unwrap();
    assert_eq!(n, 1);

    // dữ liệu cá nhân đã đi
    let row = sqlx::query("SELECT full_name, doc_no, redacted_at FROM declaration_identity")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<String, _>("full_name"), "");
    assert!(row.get::<Option<String>, _>("doc_no").is_none());
    assert!(row.get::<Option<String>, _>("redacted_at").is_some());

    // nhưng quan hệ và lịch sử lô còn nguyên
    let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(links, 1);
    let entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_entry")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(entries, 1);
}

#[tokio::test]
async fn redaction_spares_recent_and_unverified_batches() {
    let pool = test_pool().await;
    // seed: một lô verified hôm nay, một lô failed cách đây 200 ngày
    assert_eq!(redact_old_identities(&pool, 90).await.unwrap(), 0);
}
```

- [ ] **Step 2: Chạy test để xác nhận nó fail**

Run: `cd mhm/src-tauri && cargo test declaration::repo::tests::redaction`
Expected: FAIL.

- [ ] **Step 3: Cài đặt và gọi lúc khởi động**

Trong `lib.rs`, sau khi migration chạy xong, gọi một lần:

```rust
let days = declaration::repo::redact_after_days(&pool).await.unwrap_or(90);
if days > 0 {
    match declaration::repo::redact_old_identities(&pool, days).await {
        Ok(0) => {}
        Ok(n) => log::info!("Đã che {n} danh tính khai báo quá {days} ngày"),
        Err(e) => log::warn!("Không che được danh tính cũ: {e}"),
    }
}
```

Không log tên, không log số giấy tờ — chỉ log số lượng.

- [ ] **Step 4: Thêm ô Settings**

Một ô số `declaration.redact_after_days`, mặc định 90, đặt `0` để tắt.

- [ ] **Step 5: Chạy test để xác nhận pass**

Run: `cd mhm/src-tauri && cargo test declaration::repo`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src mhm/src/pages/settings
git commit -m "feat(kbtt): redact old declaration identities instead of deleting

Deleting the row would take the batch history with it, and that history is
the evidence that the guest was declared at all.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Nghiệm thu Kế hoạch B

- [ ] `cd mhm && npx vitest run` — toàn bộ xanh, không hồi quy test cũ
- [ ] `cd mhm/src-tauri && cargo test` — toàn bộ xanh
- [ ] `cd mhm && npx tsc --noEmit` — không lỗi type
- [ ] Badge sidebar hiện đúng số khách chưa khai trong 48h
- [ ] Nút `Xuất file` thật sự disabled khi còn lỗi blocking
- [ ] Không có nút một-cú-bấm nào đóng được một lô
- [ ] Lô `failed` giữ khách ở trạng thái chưa khai báo
- [ ] `CheckinSheet.tsx` và `watcher.rs` **không có thay đổi nào** — kiểm bằng `git diff main --stat`
