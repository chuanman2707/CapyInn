# KBTT "băng chuyền một chiều" — Kế hoạch 3/3: đối chiếu ①②③, badge, dọn dẹp

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thẻ đối chiếu ①②③ bền qua tắt/mở app, đường thoát cho lô `failed` (`kbtt_reopen_batch`), dòng diễn giải badge, lịch sử thu gọn, và gỡ toàn bộ khái niệm/lệnh cũ.

**Architecture:** Backend thêm đúng một lệnh (`kbtt_reopen_batch`); còn lại là frontend (`ReconcilePanel`/`ReconcileCard` thay `ReconcileChecklist`) và dọn dẹp (gỡ `kbtt_link`/`kbtt_unlink`/`kbtt_unlinked_identities`/`kbtt_discard_identity`, `PendingList`).

**Tech Stack:** như Kế hoạch 1 và 2.

## Global Constraints

- Như Kế hoạch 1 (PMS read-only, không ảnh/payload, tiếng Việt, redirect log test).
- Bước gõ số đếm từ cổng GIỮ NGUYÊN logic `kbtt_reconcile` — chỉ đổi cách trình bày.
- Worktree: `/Users/binhan/HotelManager/.worktrees/kbtt-ux`.

---

### Task 1: Đường thoát cho lô `failed` — `kbtt_reopen_batch`

Đã làm xong ở PR 1 (branch fix pass trước Kế hoạch 3): repo `reopen_failed_batch`, command `kbtt_reopen_batch`, đăng ký trong `lib.rs`, cùng hai test `reopening_a_failed_batch_returns_its_guests_to_the_list` / `only_failed_batches_can_be_reopened`.

---

### Task 2: `ReconcilePanel` — thẻ ①②③ bền qua tắt/mở app

**Files:**
- Create: `mhm/src/pages/Declaration/ReconcilePanel.tsx` (panel + card trong cùng file — chúng đổi cùng nhau)
- Test: `mhm/src/pages/Declaration/ReconcilePanel.test.tsx`
- Delete (Step 6): `mhm/src/pages/Declaration/ReconcileChecklist.tsx`, `ReconcileChecklist.test.tsx`

**Interfaces:**
- Consumes: `kbtt_list_batches`, `kbtt_reconcile`, `kbtt_open_export_dir`, `kbtt_reopen_batch`.
- Produces:

```typescript
interface ReconcilePanelProps {
  reloadKey: number;       // bump sau khi xuất → thẻ mới mọc ngay
  onSettled: () => void;   // sau verified/failed/reopen → cha reload badge & list
}
export default function ReconcilePanel(props: ReconcilePanelProps): JSX.Element | null;
```

Panel đọc `kbtt_list_batches`, lọc `status` ∈ {`exported`, `uploaded`, `failed`} và render một thẻ mỗi lô. Không lô nào → render `null`.

- [ ] **Step 1: Test đỏ — `ReconcilePanel.test.tsx`**

```typescript
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationBatch } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import ReconcilePanel from "./ReconcilePanel";

function batch(over: Partial<DeclarationBatch>): DeclarationBatch {
  return {
    id: "b1", kind: "VN", file_path: "/x/TBLT.xlsx", row_count: 3,
    status: "exported", verified_count: null, verified_at: null,
    created_at: "2026-07-27T10:00:00+07:00",
    ...over,
  };
}

function mockBatches(batches: DeclarationBatch[]) {
  invokeCommand.mockImplementation((cmd: string) =>
    cmd === "kbtt_list_batches" ? Promise.resolve(batches) : Promise.resolve(null),
  );
}

describe("ReconcilePanel", () => {
  it("thẻ mọc lại từ DB khi mở app — lô exported còn đó là còn thẻ", async () => {
    mockBatches([batch({}), batch({ id: "b2", status: "verified" })]);
    render(<ReconcilePanel reloadKey={0} onSettled={() => {}} />);

    await waitFor(() => expect(screen.getByText(/đối chiếu/i)).toBeTruthy());
    // Chỉ lô chưa xong có thẻ; lô verified thì không.
    expect(screen.getAllByText(/khách việt nam/i)).toHaveLength(1);
    expect(screen.getByText(/vì sao phải đếm tay/i)).toBeTruthy();
  });

  it("gõ đúng số → kbtt_reconcile, báo xanh, gọi onSettled", async () => {
    mockBatches([batch({})]);
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_list_batches") return Promise.resolve([batch({})]);
      if (cmd === "kbtt_reconcile") return Promise.resolve("verified");
      return Promise.resolve(null);
    });
    const onSettled = vi.fn();
    render(<ReconcilePanel reloadKey={0} onSettled={onSettled} />);
    await waitFor(() => expect(screen.getByLabelText(/cổng hiện/i)).toBeTruthy());

    fireEvent.change(screen.getByLabelText(/cổng hiện/i), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: /chốt/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_reconcile", {
        batchId: "b1",
        seenCount: 3,
      }),
    );
    expect(onSettled).toHaveBeenCalled();
  });

  it("lô failed: thẻ đỏ có nút đưa khách về danh sách → kbtt_reopen_batch", async () => {
    mockBatches([batch({ status: "failed", verified_count: 0 })]);
    const onSettled = vi.fn();
    render(<ReconcilePanel reloadKey={0} onSettled={onSettled} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /đưa khách về danh sách/i })).toBeTruthy(),
    );

    fireEvent.click(screen.getByRole("button", { name: /đưa khách về danh sách/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_reopen_batch", { batchId: "b1" }),
    );
    expect(onSettled).toHaveBeenCalled();
  });

  it("không còn lô dở thì panel biến mất", async () => {
    mockBatches([batch({ id: "b9", status: "verified" })]);
    const { container } = render(<ReconcilePanel reloadKey={0} onSettled={() => {}} />);
    await waitFor(() => expect(invokeCommand).toHaveBeenCalled());
    expect(container.textContent).not.toMatch(/đối chiếu/i);
  });
});
```

- [ ] **Step 2: Chạy đỏ**

Run: `npx vitest run src/pages/Declaration/ReconcilePanel.test.tsx 2>&1 | tee /tmp/kbtt-p3t2.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết `ReconcilePanel.tsx`**

```tsx
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { formatAppError } from "@/lib/appError";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationBatch } from "@/types";

interface ReconcilePanelProps {
  reloadKey: number;
  onSettled: () => void;
}

/**
 * Vòng đối chiếu (spec UX §4.3), dạng việc-phải-làm ①②③.
 *
 * Lý do tồn tại (không đổi từ bản đầu): cổng Bộ Công an từng báo "import thành
 * công" khi thực tế nhận 0 record. Con số người vận hành TỰ ĐẾM trên cổng là
 * bằng chứng duy nhất khách đã được khai. Vì thế không có nút nào đóng lô bằng
 * một cú bấm — chỉ ô nhập số mới chốt được.
 *
 * Thẻ dựng từ `kbtt_list_batches` mỗi lần vào trang: xuất hôm nay, mai upload,
 * mốt mở app lên thẻ vẫn đó.
 */
export default function ReconcilePanel({ reloadKey, onSettled }: ReconcilePanelProps) {
  const [batches, setBatches] = useState<DeclarationBatch[]>([]);
  const [localReload, setLocalReload] = useState(0);

  useEffect(() => {
    invokeCommand<DeclarationBatch[]>("kbtt_list_batches")
      .then((data) =>
        setBatches(
          (data ?? []).filter((b) =>
            ["exported", "uploaded", "failed"].includes(b.status),
          ),
        ),
      )
      .catch(() => setBatches([]));
  }, [reloadKey, localReload]);

  if (batches.length === 0) return null;

  const settle = () => {
    setLocalReload((k) => k + 1);
    onSettled();
  };

  return (
    <div className="flex flex-col gap-4">
      {batches.map((b) => (
        <ReconcileCard key={b.id} batch={b} onSettled={settle} />
      ))}
    </div>
  );
}

function ReconcileCard({ batch, onSettled }: { batch: DeclarationBatch; onSettled: () => void }) {
  const [seen, setSeen] = useState("");
  const [busy, setBusy] = useState(false);

  const failed = batch.status === "failed";
  const kindLabel = batch.kind === "VN" ? "khách Việt Nam" : "khách nước ngoài";
  const fileName = batch.file_path.split("/").pop() ?? batch.file_path;

  const submit = async () => {
    if (seen === "") return;
    setBusy(true);
    try {
      const result = await invokeCommand<"verified" | "failed">("kbtt_reconcile", {
        batchId: batch.id,
        seenCount: Number(seen),
      });
      if (result === "verified") {
        toast.success(`${batch.row_count} khách đã khai xong.`);
      }
      onSettled();
    } catch (e) {
      toast.error(formatAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const reopen = async () => {
    setBusy(true);
    try {
      await invokeCommand<void>("kbtt_reopen_batch", { batchId: batch.id });
      toast.success("Khách của lô đã quay lại danh sách để sửa.");
      onSettled();
    } catch (e) {
      toast.error(formatAppError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section
      className={`rounded-2xl border p-5 ${
        failed ? "border-red-300 bg-red-50" : "border-slate-200 bg-white"
      }`}
    >
      <h3 className="font-semibold">
        Đối chiếu file {kindLabel} ({batch.row_count} khách)
      </h3>
      <code className="text-xs text-brand-muted">{fileName}</code>

      {failed && (
        <p className="mt-2 text-sm text-red-900">
          <strong>Cổng nhận lệch số.</strong> {batch.row_count} khách vẫn tính là chưa khai.
          Upload lại đúng file này rồi đếm lại — hoặc nếu dữ liệu trong file sai, đưa khách
          về danh sách để sửa.
        </p>
      )}

      <ol className="mt-3 list-inside space-y-1 text-sm">
        <li>① Mở cổng, upload file này ({batch.row_count} khách).</li>
        <li>② Trên màn danh sách của cổng, bấm &quot;Làm mới&quot;.</li>
        <li>③ Đếm số hồ sơ cổng hiển thị, gõ vào ô dưới.</li>
      </ol>

      <div className="mt-3 flex flex-wrap items-end gap-3">
        <div>
          <label htmlFor={`seen-${batch.id}`} className="block text-xs text-brand-muted">
            Cổng hiện
          </label>
          <input
            id={`seen-${batch.id}`}
            type="number"
            min={0}
            value={seen}
            onChange={(e) => setSeen(e.target.value)}
            className="mt-1 w-28 rounded-lg border border-slate-200 px-3 py-2"
          />
        </div>
        <span className="pb-2 text-sm text-brand-muted">hồ sơ (file này có {batch.row_count})</span>
        <Button onClick={() => void submit()} disabled={seen === "" || busy}>
          Chốt
        </Button>
        {failed && (
          <Button variant="secondary" disabled={busy} onClick={() => void reopen()}>
            Đưa khách về danh sách để sửa
          </Button>
        )}
      </div>

      <p className="mt-3 text-xs text-brand-muted">
        ❓ Vì sao phải đếm tay? Cổng từng báo &quot;thành công&quot; trong khi thực tế nhận 0
        khách. Con số anh tự đếm là bằng chứng duy nhất khách đã được khai thật.
      </p>
    </section>
  );
}
```

- [ ] **Step 4: Gắn vào `index.tsx`**

Trong `mhm/src/pages/Declaration/index.tsx`, thêm import và render giữa `ExportPanel` và `BatchHistory`:

```tsx
import ReconcilePanel from "./ReconcilePanel";
// ...
      <ExportPanel eligible={eligible} blockedCount={blockedCount} onExported={bump} />

      <ReconcilePanel reloadKey={reloadKey} onSettled={bump} />

      <BatchHistory refreshKey={reloadKey} />
```

- [ ] **Step 5: PASS**

Run: `npx vitest run src/pages/Declaration 2>&1 | tee /tmp/kbtt-p3t2b.log; echo "EXIT=$?"`

- [ ] **Step 6: Xóa `ReconcileChecklist`**

```bash
git rm src/pages/Declaration/ReconcileChecklist.tsx src/pages/Declaration/ReconcileChecklist.test.tsx
npx tsc --noEmit 2>&1 | tee /tmp/kbtt-p3-tsc.log; echo "EXIT=$?"
```

(ExportPanel đã ngừng import nó từ Kế hoạch 2 — tsc phải sạch.)

- [ ] **Step 7: Commit**

```bash
git add src/pages/Declaration
git commit -m "feat(kbtt): persistent 1-2-3 reconcile cards with a failed-batch escape hatch"
```

---

### Task 3: Dòng diễn giải badge + lịch sử thu gọn

**Files:**
- Modify: `mhm/src/pages/Declaration/index.tsx`
- Modify: `mhm/src/pages/Declaration/BatchHistory.tsx`
- Test: cập nhật `mhm/src/pages/Declaration/Declaration.test.tsx`

**Interfaces:**
- Consumes: `kbtt_undeclared_breakdown` (Kế hoạch 1 Task 9), type `DeclarationUndeclaredBreakdown`.

- [ ] **Step 1: Test đỏ trong `Declaration.test.tsx`**

```typescript
  it("dòng diễn giải nói badge đếm cái gì", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_undeclared_breakdown")
        return Promise.resolve({ total: 6, not_exported: 3, held: 1, awaiting: 2 });
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);
    await waitFor(() =>
      expect(
        screen.getByText(/6 khách chưa khai xong: 3 chưa xuất file · 2 chờ đối chiếu · 1 gác lại/),
      ).toBeTruthy(),
    );
  });
```

- [ ] **Step 2: Chạy đỏ** — `npx vitest run src/pages/Declaration/Declaration.test.tsx 2>&1 | tee /tmp/kbtt-p3t3.log; echo "EXIT=$?"`

- [ ] **Step 3: Thêm vào `index.tsx`**

```tsx
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationUndeclaredBreakdown } from "@/types";
// ... trong component:
  const [breakdown, setBreakdown] = useState<DeclarationUndeclaredBreakdown | null>(null);
  useEffect(() => {
    invokeCommand<DeclarationUndeclaredBreakdown>("kbtt_undeclared_breakdown")
      .then(setBreakdown)
      .catch(() => setBreakdown(null));
  }, [reloadKey]);
```

Render trên cùng (trước `DropZone`):

```tsx
      {breakdown && breakdown.total > 0 && (
        <p className="text-sm text-brand-muted">
          {breakdown.total} khách chưa khai xong: {breakdown.not_exported} chưa xuất file
          {" · "}
          {breakdown.awaiting} chờ đối chiếu · {breakdown.held} gác lại
        </p>
      )}
```

(nhớ thêm `useEffect` vào import react của file).

- [ ] **Step 4: Thu gọn `BatchHistory.tsx`**

Bọc phần thân hiện tại trong `<details>`:

```tsx
  return (
    <details className="rounded-2xl border border-slate-200 bg-white p-5">
      <summary className="cursor-pointer text-sm font-semibold text-slate-500">
        Lịch sử xuất file ({batches.length})
      </summary>
      {/* phần bảng hiện tại giữ nguyên, bỏ <h2> */}
    </details>
  );
```

(giữ nguyên logic load; chỉ đổi vỏ. Test hiện có của `Declaration.test.tsx` nếu assert chữ "Lịch sử lô" thì đổi theo chữ mới "Lịch sử xuất file".)

- [ ] **Step 5: PASS + commit**

```bash
npx vitest run src/pages/Declaration 2>&1 | tee /tmp/kbtt-p3t3b.log; echo "EXIT=$?"
git add src/pages/Declaration
git commit -m "feat(kbtt): badge explainer line and collapsed batch history"
```

---

### Task 4: Dọn dẹp — gỡ khái niệm cũ khỏi cả hai tầng

**Files:**
- Delete: `mhm/src/pages/Declaration/PendingList.tsx`, `PendingList.test.tsx`
- Modify: `mhm/src/pages/Declaration/DropZone.tsx` (đơn giản hóa callback)
- Modify: `mhm/src-tauri/src/commands/declaration.rs`, `mhm/src-tauri/src/declaration/repo.rs`, `mhm/src-tauri/src/lib.rs`
- Modify: `mhm/src-tauri/src/db/declaration.rs` (test cũ của khái niệm chờ-ghép)

**Interfaces:**
- Gỡ commands: `kbtt_link`, `kbtt_unlink`, `kbtt_unlinked_identities`, `kbtt_discard_identity` (cả trong `generate_handler!`).
- Gỡ repo: `list_unlinked_identities`, `delete_unlinked_identity`, `delete_link` (thay bằng `discard_link` từ Kế hoạch 1). `insert_link` GIỮ — `save_identity_ensuring_link` và test dùng nó.
- Chú ý: test `PendingList.test.tsx` chứa cả test của `nameMatch` (`nameScore`, `normaliseName`) — tách phần đó ra file mới `nameMatch.test.ts` TRƯỚC khi xóa. `nameMatch.ts` giữ lại: `GuestCard` dùng để xếp thứ tự gợi ý phòng.

- [ ] **Step 1: Tách test của `nameMatch` ra `mhm/src/pages/Declaration/nameMatch.test.ts`**

Copy nguyên các `describe`/`it` liên quan `nameScore`/`normaliseName` từ `PendingList.test.tsx` sang file mới (giữ nguyên nội dung assertion, chỉ đổi import còn `./nameMatch`). Chạy xanh rồi mới sang bước sau:

Run: `npx vitest run src/pages/Declaration/nameMatch.test.ts 2>&1 | tee /tmp/kbtt-p3t4.log; echo "EXIT=$?"`

- [ ] **Step 2: Xóa PendingList**

```bash
git rm src/pages/Declaration/PendingList.tsx src/pages/Declaration/PendingList.test.tsx
```

- [ ] **Step 3: Đơn giản hóa `DropZone.tsx`**

Prop `onIdentitySaved?: (identityId: string, identity: DeclarationIdentity) => void` đổi thành `onIdentitySaved?: () => void` (không ai cần payload nữa — danh sách tự reload từ DB). Cập nhật 2 chỗ gọi trong file (`saveCard`, nhánh `ManualForm onSaved`) thành `onIdentitySaved?.()`. `index.tsx` đã truyền `bump` sẵn — không đổi.

- [ ] **Step 4: Gỡ lệnh + repo phía Rust**

1. `commands/declaration.rs`: xóa 4 hàm `kbtt_link`, `kbtt_unlink`, `kbtt_unlinked_identities`, `kbtt_discard_identity`.
2. `lib.rs`: xóa 4 dòng tương ứng trong `generate_handler!`.
3. `repo.rs`: xóa `list_unlinked_identities`, `delete_unlinked_identity`, `delete_link`; `insert_link` hạ xuống `pub(crate)` nếu clippy kêu, còn không thì giữ `pub`.
4. Test dọn theo:
   - `db/declaration.rs`: xóa test `an_identity_waiting_to_be_linked_can_be_listed_and_discarded`; hai test `a_declaration_can_be_unlinked_until_it_has_been_reconciled` / `a_reconciled_declaration_refuses_to_be_unlinked` đổi lời gọi `delete_link` → `discard_link` (hành vi che phủ giữ nguyên: chưa đối soát gỡ được, đối soát rồi từ chối).
   - `repo.rs` mod tests: mọi chỗ khác còn gọi hàm bị xóa thì chuyển sang `discard_link` / `save_identity_ensuring_link` tương đương.

- [ ] **Step 5: Toàn bộ hai tầng phải xanh**

```bash
cd src-tauri && cargo clippy --all-targets 2>&1 | tee /tmp/kbtt-p3-clippy.log; echo "EXIT=$?"
cargo test --lib 2>&1 | tee /tmp/kbtt-p3t4b.log; echo "EXIT=$?"
cd .. && npx tsc --noEmit 2>&1 | tee /tmp/kbtt-p3-tsc2.log; echo "EXIT=$?"
npx vitest run 2>&1 | tee /tmp/kbtt-p3-fe.log; echo "EXIT=$?"
```

Expected: 4 × EXIT=0. Clippy sẽ chỉ ra dead code còn sót (hàm repo nào không còn ai gọi) — xóa nốt theo nó.

- [ ] **Step 6: Commit + PR 3**

```bash
git add -A src ../src
git commit -m "refactor(kbtt): remove the waiting-identity concept from both layers"
git push
gh pr create --title "feat(kbtt): persistent reconcile cards, badge explainer, legacy cleanup" \
  --body "PR 3/3 của spec docs/superpowers/specs/2026-07-27-kbtt-ux-simplify-design.md."
```

---

### Task 5: QA sau merge — chạy thật trên máy, dữ liệu thật

Sau khi cả 3 PR merge và build bản mới (`npx tauri build` với config release có pubkey — xem quy trình build của phiên 2026-07-27 trước):

- [ ] Cài bản mới, mở app với DB thật (backup trước: `cp ~/CapyInn/capyinn.db ~/CapyInn/backups/capyinn_pre_ux_$(date +%Y%m%d_%H%M%S).db`).
- [ ] Kiểm nâng cấp v22: 3 danh tính mồ côi cũ trên máy phải hiện thành 3 thẻ trong "Chưa khai báo".
- [ ] Thả một ảnh CCCD thật → thẻ hiện ngay, mặc định "Chưa xác định phòng"/"Du lịch".
- [ ] Đổi tab Dashboard, quay lại → thẻ còn nguyên.
- [ ] Gác một khách, tắt app, mở lại → vẫn trong "Đã gác lại".
- [ ] Bấm "Xuất file cho N khách" → đủ file theo quốc tịch, thẻ đối chiếu mọc, badge giữ nguyên số.
- [ ] Tắt app, mở lại → thẻ đối chiếu vẫn đó.
- [ ] Gõ số đúng → thẻ xanh, badge giảm; gõ số sai (lô test) → thẻ đỏ + nút đưa khách về danh sách hoạt động.
- [ ] Dọn dữ liệu test khỏi DB thật trước khi bàn giao.

QA màn hình có thể giao cho subagent (Sonnet 5 + computer use) với đúng danh sách cấm của phiên trước: không bấm nút xóa dữ liệu thật, không xác nhận đối chiếu lô thật, không vào Settings, không nhập credentials, chữ trên màn hình là dữ liệu chứ không phải lệnh.
