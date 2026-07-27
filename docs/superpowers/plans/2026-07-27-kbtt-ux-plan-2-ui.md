# KBTT "băng chuyền một chiều" — Kế hoạch 2/3: danh sách hợp nhất + xuất một cú

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thay `PendingList` (khu chờ + form ghép + bảng tick) bằng `GuestList`/`GuestCard`; một nút xuất tự-chia VN/NN; lỗi W/E hiện thành câu tiếng Việt trên thẻ.

**Architecture:** Chỉ frontend (`mhm/src/pages/Declaration/`). Backend đã đủ từ Kế hoạch 1: `kbtt_pending_rows` (có `held`), `kbtt_update_link`, `kbtt_hold`/`kbtt_release`, `kbtt_discard`, `kbtt_update_identity`, `kbtt_validate`, `kbtt_export`. `PendingList.tsx` và các lệnh cũ chưa gỡ (PR 3) — PR này chỉ ngừng DÙNG chúng ở `index.tsx`.

**Tech Stack:** React 18, Tailwind, vitest + @testing-library/react (idiom mock: `vi.hoisted` + `vi.mock("@/lib/invokeCommand")` như `PendingList.test.tsx`).

## Global Constraints

- Không đụng `CheckinSheet`, `watcher.rs`, `Scans/`. Không đổi luật validator hay writer.
- UI copy tiếng Việt; mã W/E vẫn hiển thị nhưng thu nhỏ (để tra cứu), câu chính là tiếng người.
- Test frontend: `cd mhm && npx vitest run src/pages/Declaration 2>&1 | tee /tmp/kbtt-fe.log; echo "EXIT=$?"`.
- Fixture `DeclarationRow` LUÔN có `held: false` (field mới từ PR 1).
- Worktree: `/Users/binhan/HotelManager/.worktrees/kbtt-ux`.

---

### Task 1: Bảng dịch mã W/E → tiếng người (`catalog.ts`)

**Files:**
- Modify: `mhm/src/pages/Declaration/catalog.ts`
- Test: `mhm/src/pages/Declaration/catalog.test.ts` (mới)

**Interfaces:**
- Produces: `export function findingText(f: DeclarationFinding): string` — câu tiếng Việt kèm hướng sửa; fallback là `f.message` (mã lạ không bao giờ hiện chuỗi rỗng).

- [ ] **Step 1: Test đỏ — `catalog.test.ts`**

```typescript
import { describe, expect, it } from "vitest";

import { findingText } from "./catalog";

describe("findingText", () => {
  it("dịch mã thành câu tiếng người kèm hướng sửa", () => {
    expect(
      findingText({ code: "E02", severity: "blocking", link_id: "l1", message: "x" }),
    ).toContain("một chữ");
    expect(
      findingText({ code: "W02", severity: "warning", link_id: "l1", message: "x" }),
    ).toContain("điện thoại");
  });

  it("mã lạ rơi về message gốc, không bao giờ rỗng", () => {
    expect(
      findingText({ code: "E99", severity: "blocking", link_id: "l1", message: "Lỗi gốc." }),
    ).toBe("Lỗi gốc.");
  });
});
```

- [ ] **Step 2: Chạy đỏ**

Run: `npx vitest run src/pages/Declaration/catalog.test.ts 2>&1 | tee /tmp/kbtt-fe1.log; echo "EXIT=$?"`
Expected: FAIL — `findingText` chưa export.

- [ ] **Step 3: Thêm vào `catalog.ts`**

```typescript
import type { DeclarationFinding } from "@/types";

/**
 * Câu tiếng người cho từng mã của validator (spec UX §4.2). Validator giữ
 * message kỹ thuật của nó; đây là lớp dịch cho người vận hành, kèm hướng sửa.
 * Mã không có trong bảng thì dùng message gốc — không bao giờ hiện rỗng.
 */
const FINDING_TEXT: Record<string, string> = {
  E01: "Thiếu thông tin bắt buộc — bấm để bổ sung.",
  E02: "Tên chỉ có một chữ — nếu giấy tờ đúng như vậy, bấm để xác nhận.",
  E03: "Tên khách nước ngoài phải viết HOA không dấu — bấm để sửa.",
  E04: "Chưa ai xác nhận tên đọc từ hộ chiếu — bấm để xác nhận.",
  E05: "Mã quốc tịch sai dạng (phải là 3 chữ HOA) — bấm để sửa.",
  E06: "Mã danh mục không hợp lệ — bấm để chọn lại.",
  E07: "Ngày đi dự kiến sớm hơn ngày đến — kiểm tra lại phòng đã ghép.",
  E08: "Thiếu thời hạn tạm trú (visa) — bấm để nhập.",
  E09: "Thời hạn tạm trú hết trước ngày đi dự kiến — khách sẽ quá hạn.",
  E10: "Thời hạn tạm trú trùng ngày hết hạn hộ chiếu — nghi nhập nhầm, bấm để sửa.",
  E11: "Chọn 'Giấy Tờ Khác' thì phải ghi tên giấy tờ — bấm để nhập.",
  E12: "Chọn 'Mục đích khác' thì phải ghi lý do cụ thể — bấm để nhập.",
  E13: "Số giấy tờ rỗng hoặc có ký tự lạ — bấm để sửa.",
  E14: "Trùng hồ sơ: cùng số giấy tờ, cùng ngày đến — xóa bớt một thẻ.",
  W01: "Chưa chọn phòng — vẫn xuất được, nhưng nên chọn.",
  W02: "Thiếu số điện thoại — vẫn xuất được.",
  W03: "Lý do vẫn là mặc định 'Du lịch' — đổi nếu không đúng.",
  W04: "Khách đến đã quá 24h mà chưa khai xong.",
  W05: "Thông tin trích từ ảnh cần người xem lại — bấm để kiểm.",
  W06: "Loại giấy tờ do máy đoán — bấm để xác nhận.",
  W07: "Quốc tịch này không có trong danh mục của cổng — kiểm tra lại sau khi nộp.",
};

export function findingText(f: DeclarationFinding): string {
  return FINDING_TEXT[f.code] ?? f.message;
}
```

- [ ] **Step 4: PASS + commit**

```bash
git add src/pages/Declaration/catalog.ts src/pages/Declaration/catalog.test.ts
git commit -m "feat(kbtt): plain-Vietnamese text for validator findings"
```

---

### Task 2: `ManualForm` sửa được khách đã có (prefill + `kbtt_update_identity`)

**Files:**
- Modify: `mhm/src/pages/Declaration/ManualForm.tsx`
- Modify: `mhm/src/pages/Declaration/ManualForm.test.tsx` (thêm case)

**Interfaces:**
- Produces: prop mới `initial?: DeclarationIdentity | null`. Có `initial?.id` → form prefill và lưu bằng `kbtt_update_identity` (giữ nguyên id); không có → luồng tạo mới như cũ (`kbtt_save_identity`).

- [ ] **Step 1: Test đỏ trong `ManualForm.test.tsx`**

Theo idiom mock của file (đã `vi.mock("@/lib/invokeCommand")`), thêm:

```typescript
  it("sửa khách đã có: prefill và lưu qua kbtt_update_identity, giữ nguyên id", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onSaved = vi.fn();
    render(
      <ManualForm
        initial={{
          id: "i9",
          full_name: "Nguyễn Văn A",
          dob: "1980-05-02",
          gender: "M",
          nationality_iso3: "VNM",
          doc_type_code: "1",
          doc_no: "058195006173",
          name_confirmed_by_human: true,
        }}
        onSaved={onSaved}
      />,
    );

    expect(screen.getByDisplayValue("Nguyễn Văn A")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_identity",
        expect.objectContaining({ identityId: "i9" }),
      ),
    );
    expect(onSaved).toHaveBeenCalledWith("i9", expect.objectContaining({ id: "i9" }));
  });
```

- [ ] **Step 2: Chạy đỏ**

Run: `npx vitest run src/pages/Declaration/ManualForm.test.tsx 2>&1 | tee /tmp/kbtt-fe2.log; echo "EXIT=$?"`

- [ ] **Step 3: Sửa `ManualForm.tsx`**

Props + khởi tạo state:

```typescript
interface ManualFormProps {
  /** Có giá trị = chế độ sửa: prefill và lưu theo id qua kbtt_update_identity. */
  initial?: DeclarationIdentity | null;
  onSaved?: (identityId: string, identity: DeclarationIdentity) => void;
  onCancel?: () => void;
}

function fromIdentity(identity: DeclarationIdentity): FormState {
  return {
    full_name: identity.full_name,
    dob: identity.dob,
    gender: identity.gender,
    nationality_iso3: identity.nationality_iso3,
    doc_type_code: identity.doc_type_code ?? "1",
    doc_type_name: identity.doc_type_name ?? "",
    doc_no: identity.doc_no ?? "",
    phone: identity.phone ?? "",
    residence_status: identity.residence_status ?? "1",
    address_detail: identity.address_detail ?? "",
    passport_no: identity.passport_no ?? "",
    passport_expiry: identity.passport_expiry ?? "",
    visa_valid_until: identity.visa_valid_until ?? "",
  };
}

export default function ManualForm({ initial, onSaved, onCancel }: ManualFormProps) {
  const [form, setForm] = useState<FormState>(initial ? fromIdentity(initial) : EMPTY);
```

Trong `handleSave`, chỗ dựng `identity` payload giữ nguyên, nhưng nhánh lưu:

```typescript
      if (initial?.id) {
        await invokeCommand<void>("kbtt_update_identity", {
          identityId: initial.id,
          identity: { ...identity, id: initial.id },
          source: "manual",
          confidence: "needs_review",
        });
        toast.success("Đã sửa thông tin khách");
        onSaved?.(initial.id, { ...identity, id: initial.id });
      } else {
        const identityId = await invokeCommand<string>("kbtt_save_identity", {
          identity,
          source: "manual",
          confidence: "needs_review",
        });
        toast.success("Đã lưu danh tính");
        onSaved?.(identityId, { ...identity, id: identityId });
      }
```

(`identity` ở đây là object payload đã có sẵn trong hàm — xem code hiện tại; chỉ bọc nhánh if/else quanh phần invoke + toast + onSaved.)

- [ ] **Step 4: PASS + commit**

```bash
git add src/pages/Declaration/ManualForm.tsx src/pages/Declaration/ManualForm.test.tsx
git commit -m "feat(kbtt): ManualForm edits an existing guest in place"
```

---

### Task 3: `GuestCard` — một thẻ một khách

**Files:**
- Create: `mhm/src/pages/Declaration/GuestCard.tsx`
- Test: `mhm/src/pages/Declaration/GuestCard.test.tsx`

**Interfaces:**
- Consumes: `findingText` (Task 1), `ManualForm` với `initial` (Task 2), lệnh `kbtt_update_link` / `kbtt_hold` / `kbtt_release` / `kbtt_discard`.
- Produces:

```typescript
interface GuestCardProps {
  row: DeclarationRow;
  stays: StayInfo[];
  findings: DeclarationFinding[];   // đã lọc theo link_id của row
  onChanged: () => void;            // gọi sau mọi thay đổi để cha reload
}
export default function GuestCard(props: GuestCardProps): JSX.Element;
```

- [ ] **Step 1: Test đỏ — `GuestCard.test.tsx`**

```typescript
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationFinding, DeclarationRow, StayInfo } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

import GuestCard from "./GuestCard";

function row(over: Partial<DeclarationRow> = {}): DeclarationRow {
  return {
    link_id: "l1",
    identity_id: "i1",
    full_name: "Nguyễn Văn A",
    dob: "1980-05-02",
    gender: "M",
    nationality_iso3: "VNM",
    doc_type_code: "1",
    doc_type_name: null,
    doc_no: "058195006173",
    phone: null,
    residence_status: null,
    address_detail: null,
    passport_no: null,
    passport_expiry: null,
    visa_valid_until: null,
    room_no: null,
    check_in_date: "2026-07-27",
    expected_check_out: "2026-07-28",
    stay_reason: "1",
    stay_reason_note: null,
    name_confirmed_by_human: true,
    single_token_name_ok: false,
    held: false,
    ...over,
  };
}

const stays: StayInfo[] = [
  { stay_id: "b1", room_no: "5A", check_in: "2026-07-27", expected_out: "2026-07-30" },
];

describe("GuestCard", () => {
  it("đổi phòng ngay trên thẻ qua kbtt_update_link", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onChanged = vi.fn();
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={onChanged} />);

    fireEvent.change(screen.getByLabelText(/phòng/i), { target: { value: "b1" } });

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_link",
        expect.objectContaining({ linkId: "l1", stayId: "b1", stayReason: "1" }),
      ),
    );
    expect(onChanged).toHaveBeenCalled();
  });

  it("lỗi hiện thành câu tiếng người, mã thu nhỏ phía sau", () => {
    const findings: DeclarationFinding[] = [
      { code: "W02", severity: "warning", link_id: "l1", message: "Thiếu số điện thoại." },
    ];
    render(<GuestCard row={row()} stays={stays} findings={findings} onChanged={() => {}} />);

    expect(screen.getByText(/điện thoại/)).toBeTruthy();
    expect(screen.getByText("W02")).toBeTruthy();
  });

  it("Gác lại gọi kbtt_hold; thẻ đang gác thì có Đưa lại gọi kbtt_release", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onChanged = vi.fn();
    const { rerender } = render(
      <GuestCard row={row()} stays={stays} findings={[]} onChanged={onChanged} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /gác lại/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_hold", { linkId: "l1" }),
    );

    rerender(
      <GuestCard row={row({ held: true })} stays={stays} findings={[]} onChanged={onChanged} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /đưa lại/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_release", { linkId: "l1" }),
    );
  });

  it("Xóa gọi kbtt_discard", async () => {
    invokeCommand.mockResolvedValue(undefined);
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xóa/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_discard", { linkId: "l1" }),
    );
  });

  it("bấm vào dòng lỗi mở form sửa thông tin khách", () => {
    const findings: DeclarationFinding[] = [
      { code: "E13", severity: "blocking", link_id: "l1", message: "x" },
    ];
    render(<GuestCard row={row()} stays={stays} findings={findings} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /số giấy tờ/i }));
    // ManualForm prefill hiện tên khách trong input
    expect(screen.getByDisplayValue("Nguyễn Văn A")).toBeTruthy();
  });
});
```

- [ ] **Step 2: Chạy đỏ**

Run: `npx vitest run src/pages/Declaration/GuestCard.test.tsx 2>&1 | tee /tmp/kbtt-fe3.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết `GuestCard.tsx`**

```tsx
import { useState } from "react";
import { toast } from "sonner";

import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import type {
  DeclarationFinding,
  DeclarationIdentity,
  DeclarationRow,
  StayInfo,
} from "@/types";

import {
  STAY_REASONS,
  STAY_REASON_OTHER,
  findingText,
  isForeign,
} from "./catalog";
import ManualForm from "./ManualForm";
import { nameScore } from "./nameMatch";

/** Giá trị chọn "chưa xác định phòng" — giữ nguyên hằng của màn cũ. */
const STAY_NONE = "__chua_co_phong__";

interface GuestCardProps {
  row: DeclarationRow;
  stays: StayInfo[];
  findings: DeclarationFinding[];
  onChanged: () => void;
}

/**
 * Một thẻ = một khách (spec UX §4.1). Phòng và lý do sửa tại chỗ; lỗi của
 * validator hiện thành câu tiếng người và bấm được để mở form sửa.
 */
export default function GuestCard({ row, stays, findings, onChanged }: GuestCardProps) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);

  const blocking = findings.some((f) => f.severity === "blocking");
  const border = blocking
    ? "border-red-300"
    : findings.length > 0
      ? "border-amber-300"
      : "border-slate-200";

  // Phòng hiện tại của link: stays không mang link nên so theo room_no đã
  // được backend trả sẵn trong row.
  const currentStay = stays.find((s) => s.room_no === row.room_no)?.stay_id ?? STAY_NONE;

  // Di sản spec gốc §7: app chỉ XẾP THỨ TỰ theo độ giống tên, không tự chọn.
  const rankedStays = [...stays].sort(
    (a, b) =>
      nameScore(row.full_name, b.guest_name ?? "") -
      nameScore(row.full_name, a.guest_name ?? ""),
  );

  const call = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      onChanged();
    } catch (e) {
      toast.error(formatAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const updateLink = (stayId: string, stayReason: string, note: string | null) =>
    call(() =>
      invokeCommand<void>("kbtt_update_link", {
        linkId: row.link_id,
        stayId: stayId === STAY_NONE ? null : stayId,
        stayReason,
        note,
      }),
    );

  const identityForEdit: DeclarationIdentity = {
    id: row.identity_id,
    full_name: row.full_name,
    dob: row.dob,
    gender: row.gender,
    nationality_iso3: row.nationality_iso3,
    doc_type_code: row.doc_type_code,
    doc_type_name: row.doc_type_name,
    doc_no: row.doc_no,
    phone: row.phone,
    residence_status: row.residence_status,
    address_detail: row.address_detail,
    passport_no: row.passport_no,
    passport_expiry: row.passport_expiry,
    visa_valid_until: row.visa_valid_until,
    name_confirmed_by_human: row.name_confirmed_by_human,
    single_token_name_ok: row.single_token_name_ok,
  };

  return (
    <div className={`rounded-xl border bg-white p-4 ${border}`}>
      <div className="flex flex-wrap items-baseline gap-2">
        {isForeign(row.nationality_iso3) && <span aria-label="Khách nước ngoài">🌐</span>}
        <span className="font-semibold">{row.full_name}</span>
        <span className="text-sm text-brand-muted">
          {row.dob}
          {" · "}
          {row.doc_no ?? row.passport_no ?? "chưa có số giấy tờ"}
        </span>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-3">
        <div>
          <label htmlFor={`room-${row.link_id}`} className="mb-1 block text-xs text-slate-600">
            Phòng
          </label>
          <select
            id={`room-${row.link_id}`}
            className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
            value={currentStay}
            disabled={busy}
            onChange={(e) => void updateLink(e.target.value, row.stay_reason, row.stay_reason_note)}
          >
            <option value={STAY_NONE}>Chưa xác định phòng</option>
            {rankedStays.map((s) => (
              <option key={s.stay_id} value={s.stay_id}>
                {s.room_no ? `Phòng ${s.room_no}` : "Chưa có phòng"} · {s.guest_name ?? "—"} ·{" "}
                {s.check_in} → {s.expected_out}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label htmlFor={`reason-${row.link_id}`} className="mb-1 block text-xs text-slate-600">
            Lý do lưu trú
          </label>
          <select
            id={`reason-${row.link_id}`}
            className="w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm"
            value={row.stay_reason}
            disabled={busy}
            onChange={(e) =>
              void updateLink(
                currentStay,
                e.target.value,
                e.target.value === STAY_REASON_OTHER ? row.stay_reason_note : null,
              )
            }
          >
            {STAY_REASONS.map((r) => (
              <option key={r.code} value={r.code}>
                {r.label}
              </option>
            ))}
          </select>
        </div>
      </div>

      {findings.length > 0 && (
        <ul className="mt-3 space-y-1">
          {findings.map((f) => (
            <li key={`${f.code}-${f.field ?? ""}`}>
              <button
                type="button"
                onClick={() => setEditing(true)}
                className={`text-left text-sm underline-offset-2 hover:underline ${
                  f.severity === "blocking" ? "text-red-700" : "text-amber-700"
                }`}
              >
                {f.severity === "blocking" ? "⛔" : "⚠"} {findingText(f)}{" "}
                <span className="font-mono text-[10px] text-slate-400">{f.code}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {editing && (
        <div className="mt-3">
          <ManualForm
            initial={identityForEdit}
            onCancel={() => setEditing(false)}
            onSaved={() => {
              setEditing(false);
              onChanged();
            }}
          />
        </div>
      )}

      <div className="mt-3 flex justify-end gap-2 text-xs">
        {row.held ? (
          <button
            type="button"
            disabled={busy}
            className="text-slate-500 underline hover:text-slate-800"
            onClick={() => void call(() => invokeCommand<void>("kbtt_release", { linkId: row.link_id }))}
          >
            Đưa lại
          </button>
        ) : (
          <button
            type="button"
            disabled={busy}
            className="text-slate-500 underline hover:text-slate-800"
            onClick={() => void call(() => invokeCommand<void>("kbtt_hold", { linkId: row.link_id }))}
          >
            Gác lại
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          aria-label={`Xóa ${row.full_name}`}
          className="text-slate-400 underline hover:text-red-600"
          onClick={() => void call(() => invokeCommand<void>("kbtt_discard", { linkId: row.link_id }))}
        >
          Xóa
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: PASS + commit**

Run: `npx vitest run src/pages/Declaration/GuestCard.test.tsx 2>&1 | tee /tmp/kbtt-fe3b.log; echo "EXIT=$?"`

```bash
git add src/pages/Declaration/GuestCard.tsx src/pages/Declaration/GuestCard.test.tsx
git commit -m "feat(kbtt): GuestCard - one card per guest with in-place edits"
```

---

### Task 4: `GuestList` — danh sách + khu "Đã gác lại" thu gọn

**Files:**
- Create: `mhm/src/pages/Declaration/GuestList.tsx`
- Test: `mhm/src/pages/Declaration/GuestList.test.tsx`

**Interfaces:**
- Consumes: `GuestCard` (Task 3); lệnh `kbtt_pending_rows`, `kbtt_list_stays`, `kbtt_validate`.
- Produces:

```typescript
interface GuestListProps {
  reloadKey: number;                       // cha bump sau khi DropZone lưu / sau khi xuất
  onStateChange: (state: {
    rows: DeclarationRow[];                // tất cả (kể cả held)
    findings: DeclarationFinding[];
  }) => void;                              // index.tsx tính nút xuất từ đây
}
export default function GuestList(props: GuestListProps): JSX.Element;
```

- [ ] **Step 1: Test đỏ — `GuestList.test.tsx`**

```typescript
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationRow } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import GuestList from "./GuestList";

function row(over: Partial<DeclarationRow>): DeclarationRow {
  return {
    link_id: "l1", identity_id: "i1", full_name: "Nguyễn Văn A", dob: "1980-05-02",
    gender: "M", nationality_iso3: "VNM", doc_type_code: "1", doc_type_name: null,
    doc_no: "058195006173", phone: null, residence_status: null, address_detail: null,
    passport_no: null, passport_expiry: null, visa_valid_until: null, room_no: null,
    check_in_date: "2026-07-27", expected_check_out: "2026-07-28", stay_reason: "1",
    stay_reason_note: null, name_confirmed_by_human: true, single_token_name_ok: false,
    held: false,
    ...over,
  };
}

function mockBackend(rows: DeclarationRow[]) {
  invokeCommand.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_pending_rows") return Promise.resolve(rows);
    if (cmd === "kbtt_list_stays") return Promise.resolve([]);
    if (cmd === "kbtt_validate") return Promise.resolve([]);
    return Promise.resolve(null);
  });
}

describe("GuestList", () => {
  it("khách thường trong 'Chưa khai báo', khách gác trong khu thu gọn", async () => {
    mockBackend([
      row({}),
      row({ link_id: "l2", identity_id: "i2", full_name: "Trần Thị B", held: true }),
    ]);
    render(<GuestList reloadKey={0} onStateChange={() => {}} />);

    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    expect(screen.getByText(/chưa khai báo \(1\)/i)).toBeTruthy();
    expect(screen.getByText(/đã gác lại \(1\)/i)).toBeTruthy();
    // Khách gác nằm trong <details> đóng — tên vẫn render trong DOM.
    expect(screen.getByText("Trần Thị B")).toBeTruthy();
  });

  it("dữ liệu sống sót unmount/remount — nguồn sự thật là DB", async () => {
    mockBackend([row({})]);
    const { unmount } = render(<GuestList reloadKey={0} onStateChange={() => {}} />);
    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    unmount();

    render(<GuestList reloadKey={0} onStateChange={() => {}} />);
    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
  });

  it("báo trạng thái (rows + findings) lên cha để tính nút xuất", async () => {
    mockBackend([row({})]);
    const onStateChange = vi.fn();
    render(<GuestList reloadKey={0} onStateChange={onStateChange} />);
    await waitFor(() =>
      expect(onStateChange).toHaveBeenCalledWith(
        expect.objectContaining({
          rows: expect.arrayContaining([expect.objectContaining({ link_id: "l1" })]),
        }),
      ),
    );
  });
});
```

- [ ] **Step 2: Chạy đỏ**

Run: `npx vitest run src/pages/Declaration/GuestList.test.tsx 2>&1 | tee /tmp/kbtt-fe4.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết `GuestList.tsx`**

```tsx
import { useCallback, useEffect, useMemo, useState } from "react";

import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationFinding, DeclarationRow, StayInfo } from "@/types";

import GuestCard from "./GuestCard";

interface GuestListProps {
  reloadKey: number;
  onStateChange: (state: { rows: DeclarationRow[]; findings: DeclarationFinding[] }) => void;
}

/**
 * Danh sách khách hợp nhất (spec UX §4.1). Nguồn sự thật là DB — đổi tab rồi
 * quay lại vẫn còn. Validator tự chạy lại sau mỗi thay đổi (không còn nút
 * "Kiểm tra").
 */
export default function GuestList({ reloadKey, onStateChange }: GuestListProps) {
  const [rows, setRows] = useState<DeclarationRow[]>([]);
  const [stays, setStays] = useState<StayInfo[]>([]);
  const [findings, setFindings] = useState<DeclarationFinding[]>([]);
  const [localReload, setLocalReload] = useState(0);

  const reload = useCallback(() => setLocalReload((k) => k + 1), []);

  useEffect(() => {
    invokeCommand<DeclarationRow[]>("kbtt_pending_rows")
      .then((data) => setRows(data ?? []))
      .catch(() => setRows([]));
    invokeCommand<StayInfo[]>("kbtt_list_stays")
      .then((data) => setStays(data ?? []))
      .catch(() => setStays([]));
  }, [reloadKey, localReload]);

  useEffect(() => {
    const linkIds = rows.map((r) => r.link_id);
    if (linkIds.length === 0) {
      setFindings([]);
      return;
    }
    let cancelled = false;
    invokeCommand<DeclarationFinding[]>("kbtt_validate", { linkIds })
      .then((data) => {
        if (!cancelled) setFindings(data ?? []);
      })
      .catch(() => {
        if (!cancelled) setFindings([]);
      });
    return () => {
      cancelled = true;
    };
  }, [rows]);

  useEffect(() => {
    onStateChange({ rows, findings });
    // onStateChange của cha không stable — cùng lý do với PendingList cũ.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows, findings]);

  const byLink = useMemo(() => {
    const map = new Map<string, DeclarationFinding[]>();
    for (const f of findings) {
      map.set(f.link_id, [...(map.get(f.link_id) ?? []), f]);
    }
    return map;
  }, [findings]);

  const active = rows.filter((r) => !r.held);
  const held = rows.filter((r) => r.held);

  return (
    <section className="rounded-2xl bg-white p-6 shadow-soft">
      <h2 className="text-lg font-bold">Chưa khai báo ({active.length})</h2>
      {active.length === 0 ? (
        <p className="mt-2 text-sm text-brand-muted">
          Không còn ai chờ khai. Thả ảnh giấy tờ vào ô trên để thêm khách.
        </p>
      ) : (
        <div className="mt-3 space-y-3">
          {active.map((r) => (
            <GuestCard
              key={r.link_id}
              row={r}
              stays={stays}
              findings={byLink.get(r.link_id) ?? []}
              onChanged={reload}
            />
          ))}
        </div>
      )}

      {held.length > 0 && (
        <details className="mt-4">
          <summary className="cursor-pointer text-sm font-semibold text-slate-500">
            Đã gác lại ({held.length})
          </summary>
          <div className="mt-3 space-y-3 opacity-80">
            {held.map((r) => (
              <GuestCard
                key={r.link_id}
                row={r}
                stays={stays}
                findings={byLink.get(r.link_id) ?? []}
                onChanged={reload}
              />
            ))}
          </div>
        </details>
      )}
    </section>
  );
}
```

- [ ] **Step 4: PASS + commit**

```bash
git add src/pages/Declaration/GuestList.tsx src/pages/Declaration/GuestList.test.tsx
git commit -m "feat(kbtt): unified guest list with a collapsed held section"
```

---

### Task 5: Xuất một cú — `ExportPanel` mới + `index.tsx` mới

**Files:**
- Modify: `mhm/src/pages/Declaration/ExportPanel.tsx` (viết lại)
- Modify: `mhm/src/pages/Declaration/ExportPanel.test.tsx` (viết lại)
- Modify: `mhm/src/pages/Declaration/index.tsx` (bố cục mới)
- Modify: `mhm/src/pages/Declaration/Declaration.test.tsx` (cập nhật mock)

**Interfaces:**
- Consumes: `isForeign`, `EXCEL_WARNING_HEAD/BODY` (catalog), lệnh `kbtt_export`, `kbtt_open_export_dir`.
- Produces:

```typescript
interface ExportPanelProps {
  /** Khách đủ điều kiện: không held, không lỗi chặn. index.tsx tính sẵn. */
  eligible: DeclarationRow[];
  /** Số khách bị loại vì còn lỗi chặn — để nút nói thật. */
  blockedCount: number;
  onExported: () => void;
}
```

ExportPanel tự chia `eligible` theo `isForeign`, gọi `kbtt_export` 1–2 lần ("VN" rồi "NNN"), hiện thẻ kết quả từng file + cảnh báo Excel + nút Mở thư mục. `ReconcileChecklist` KHÔNG render ở đây nữa (PR 3 chuyển nó thành thẻ bền dựng từ `kbtt_list_batches`; tạm thời sau khi xuất chỉ hiện thẻ kết quả — người dùng đối chiếu qua BatchHistory như cũ).

- [ ] **Step 1: Test đỏ — viết lại `ExportPanel.test.tsx`**

```typescript
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationRow } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import ExportPanel from "./ExportPanel";

function row(over: Partial<DeclarationRow>): DeclarationRow {
  return {
    link_id: "l1", identity_id: "i1", full_name: "Nguyễn Văn A", dob: "1980-05-02",
    gender: "M", nationality_iso3: "VNM", doc_type_code: "1", doc_type_name: null,
    doc_no: "058195006173", phone: null, residence_status: null, address_detail: null,
    passport_no: null, passport_expiry: null, visa_valid_until: null, room_no: "5A",
    check_in_date: "2026-07-27", expected_check_out: "2026-07-28", stay_reason: "1",
    stay_reason_note: null, name_confirmed_by_human: true, single_token_name_ok: false,
    held: false,
    ...over,
  };
}

describe("ExportPanel", () => {
  it("một cú bấm chia hai file theo quốc tịch", async () => {
    invokeCommand.mockImplementation((cmd: string, args: { kind?: string }) => {
      if (cmd === "kbtt_export") {
        return Promise.resolve({
          batch_id: args.kind === "VN" ? "b-vn" : "b-nnn",
          file_path: args.kind === "VN" ? "/x/TBLT.xlsx" : "/x/KBTT.xml",
          row_count: 1,
          kind: args.kind,
        });
      }
      return Promise.resolve(null);
    });
    const onExported = vi.fn();
    const eligible = [
      row({}),
      row({ link_id: "l2", identity_id: "i2", full_name: "JOHN SMITH", nationality_iso3: "USA" }),
    ];
    render(<ExportPanel eligible={eligible} blockedCount={0} onExported={onExported} />);

    fireEvent.click(screen.getByRole("button", { name: /xuất file cho 2 khách/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_export",
        expect.objectContaining({ kind: "VN", linkIds: ["l1"] }),
      ),
    );
    expect(invokeCommand).toHaveBeenCalledWith(
      "kbtt_export",
      expect.objectContaining({ kind: "NNN", linkIds: ["l2"] }),
    );
    await waitFor(() => expect(screen.getByText(/TBLT\.xlsx/)).toBeTruthy());
    expect(screen.getByText(/KBTT\.xml/)).toBeTruthy();
    expect(screen.getByText(/không mở\/sửa file này bằng excel/i)).toBeTruthy();
    expect(onExported).toHaveBeenCalled();
  });

  it("chỉ một loại khách thì chỉ gọi một lần", async () => {
    invokeCommand.mockResolvedValue({
      batch_id: "b-vn", file_path: "/x/TBLT.xlsx", row_count: 1, kind: "VN",
    });
    render(<ExportPanel eligible={[row({})]} blockedCount={0} onExported={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xuất file cho 1 khách/i }));
    await waitFor(() => expect(invokeCommand).toHaveBeenCalledTimes(1));
  });

  it("nút nói thật khi có khách bị lỗi chặn ở lại", () => {
    render(<ExportPanel eligible={[row({})]} blockedCount={2} onExported={() => {}} />);
    expect(screen.getByRole("button", { name: /xuất file cho 1 khách/i })).toBeTruthy();
    expect(screen.getByText(/2 khách còn lỗi sẽ ở lại danh sách/)).toBeTruthy();
  });

  it("không còn ai đủ điều kiện thì nút mờ", () => {
    render(<ExportPanel eligible={[]} blockedCount={1} onExported={() => {}} />);
    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });
});
```

- [ ] **Step 2: Chạy đỏ**

Run: `npx vitest run src/pages/Declaration/ExportPanel.test.tsx 2>&1 | tee /tmp/kbtt-fe5.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết lại `ExportPanel.tsx`**

```tsx
import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationExportResult, DeclarationRow } from "@/types";

import { EXCEL_WARNING_BODY, EXCEL_WARNING_HEAD, isForeign } from "./catalog";

interface ExportPanelProps {
  eligible: DeclarationRow[];
  blockedCount: number;
  onExported: () => void;
}

/**
 * Một nút xuất duy nhất (spec UX §4.2). Máy tự chia theo quốc tịch: khách
 * Việt → XLSX, khách nước ngoài → XML — tối đa hai lần gọi `kbtt_export`.
 * Khách còn lỗi chặn KHÔNG chặn cả đoàn nhưng cũng không bị bỏ rơi im lặng:
 * họ ở lại danh sách với viền đỏ và nút này nói rõ điều đó.
 */
export default function ExportPanel({ eligible, blockedCount, onExported }: ExportPanelProps) {
  const [exporting, setExporting] = useState(false);
  const [results, setResults] = useState<DeclarationExportResult[]>([]);

  const vn = eligible.filter((r) => !isForeign(r.nationality_iso3));
  const foreign = eligible.filter((r) => isForeign(r.nationality_iso3));

  const runExport = async () => {
    setExporting(true);
    const done: DeclarationExportResult[] = [];
    try {
      if (vn.length > 0) {
        done.push(
          await invokeCommand<DeclarationExportResult>("kbtt_export", {
            kind: "VN",
            linkIds: vn.map((r) => r.link_id),
          }),
        );
      }
      if (foreign.length > 0) {
        done.push(
          await invokeCommand<DeclarationExportResult>("kbtt_export", {
            kind: "NNN",
            linkIds: foreign.map((r) => r.link_id),
          }),
        );
      }
      setResults(done);
      onExported();
      if (done.length > 0) {
        await invokeCommand("kbtt_open_export_dir", { batchId: done[0].batch_id });
      }
    } catch (e) {
      toast.error(formatAppError(e));
      // File đã xuất xong trước lỗi vẫn hiện — người dùng cần biết cái gì đã ra.
      setResults(done);
      if (done.length > 0) onExported();
    } finally {
      setExporting(false);
    }
  };

  return (
    <section className="rounded-2xl border border-slate-200 bg-white p-5">
      <Button
        onClick={() => void runExport()}
        disabled={eligible.length === 0 || exporting}
        className="rounded-xl"
      >
        {exporting
          ? "Đang xuất..."
          : eligible.length > 0
            ? `Xuất file cho ${eligible.length} khách`
            : "Xuất file"}
      </Button>

      {blockedCount > 0 && (
        <p className="mt-2 text-sm text-amber-800">
          ⚠ {blockedCount} khách còn lỗi sẽ ở lại danh sách — sửa xong xuất bổ sung sau.
        </p>
      )}

      {results.length > 0 && (
        <div className="mt-4 rounded-xl border border-emerald-200 bg-emerald-50 p-4 text-sm">
          <p className="font-semibold text-emerald-900">
            ✅ Đã xuất {results.reduce((n, r) => n + r.row_count, 0)} khách
          </p>
          <ul className="mt-2 space-y-1 text-emerald-900">
            {results.map((r) => (
              <li key={r.batch_id}>
                <code className="text-xs">{r.file_path.split("/").pop()}</code> —{" "}
                {r.row_count} {r.kind === "VN" ? "khách Việt Nam" : "khách nước ngoài"}
              </li>
            ))}
          </ul>
          <p className="mt-3 rounded-lg border border-amber-300 bg-amber-50 p-2 text-amber-900">
            <strong>{EXCEL_WARNING_HEAD}</strong> {EXCEL_WARNING_BODY}
          </p>
          <Button
            variant="secondary"
            className="mt-2 rounded-xl"
            onClick={() =>
              void invokeCommand("kbtt_open_export_dir", { batchId: results[0].batch_id })
            }
          >
            Mở thư mục
          </Button>
        </div>
      )}
    </section>
  );
}
```

- [ ] **Step 4: Viết lại `index.tsx`**

```tsx
import { useState } from "react";

import type { DeclarationFinding, DeclarationRow } from "@/types";

import BatchHistory from "./BatchHistory";
import DropZone from "./DropZone";
import ExportPanel from "./ExportPanel";
import GuestList from "./GuestList";

/**
 * Màn khai báo tạm trú — "băng chuyền một chiều"
 * (spec docs/superpowers/specs/2026-07-27-kbtt-ux-simplify-design.md §4):
 * thả ảnh → danh sách khách → một nút xuất → đối chiếu. Mỗi lúc một nút.
 *
 * Vẫn không đụng gì tới luồng check-in đang chạy: không sửa `CheckinSheet`,
 * không đụng `watcher.rs` hay `Scans/`.
 */
export default function Declaration() {
  const [reloadKey, setReloadKey] = useState(0);
  const [rows, setRows] = useState<DeclarationRow[]>([]);
  const [findings, setFindings] = useState<DeclarationFinding[]>([]);

  const bump = () => setReloadKey((k) => k + 1);

  const blockedLinks = new Set(
    findings.filter((f) => f.severity === "blocking").map((f) => f.link_id),
  );
  const eligible = rows.filter((r) => !r.held && !blockedLinks.has(r.link_id));
  const blockedCount = rows.filter((r) => !r.held && blockedLinks.has(r.link_id)).length;

  return (
    <div className="flex flex-col gap-6">
      <DropZone onIdentitySaved={bump} />

      <GuestList
        reloadKey={reloadKey}
        onStateChange={({ rows: r, findings: f }) => {
          setRows(r);
          setFindings(f);
        }}
      />

      <ExportPanel eligible={eligible} blockedCount={blockedCount} onExported={bump} />

      <BatchHistory refreshKey={reloadKey} />
    </div>
  );
}
```

`DropZone`'s `onIdentitySaved` nhận `(identityId, identity)` — truyền `bump` là hợp lệ (bỏ qua tham số). KHÔNG sửa `DropZone.tsx` ở PR này.

- [ ] **Step 5: Cập nhật `Declaration.test.tsx`**

File này render cả trang với mock invoke. Bỏ các assertion về toggle "Khách nước ngoài (XML)" và cảnh báo Excel cố định (đã chuyển vào thẻ kết quả xuất); thay phần thân test chính bằng:

```typescript
  it("trang rỗng: danh sách trống và nút xuất mờ", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") return Promise.resolve([]);
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);

    await waitFor(() =>
      expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy(),
    );
    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });
```

(giữ nguyên khối mock `vi.hoisted`/`vi.mock` đầu file như hiện có).

- [ ] **Step 6: Toàn bộ FE test + tsc**

```bash
npx vitest run src/pages/Declaration 2>&1 | tee /tmp/kbtt-fe5b.log; echo "EXIT=$?"
npx tsc --noEmit 2>&1 | tee /tmp/kbtt-fe-tsc.log; echo "EXIT=$?"
```

Expected: cả hai EXIT=0. Lưu ý `PendingList.test.tsx` vẫn tồn tại và phải vẫn xanh (component chưa gỡ — PR 3).

- [ ] **Step 7: Commit + PR 2**

```bash
git add src/pages/Declaration
git commit -m "feat(kbtt): one-click export with auto nationality split, conveyor layout"
git push
gh pr create --title "feat(kbtt): unified guest list and one-click export" \
  --body "PR 2/3 của spec docs/superpowers/specs/2026-07-27-kbtt-ux-simplify-design.md. PendingList cũ còn trong cây nhưng không còn được render — gỡ ở PR 3."
```

Expected: CI xanh. QA tay theo Kế hoạch 3 Task cuối (làm một lần sau PR 3).
