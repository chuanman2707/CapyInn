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
          // "verified" và "reopened" đã xong việc — không dựng thẻ cho chúng.
          // Một lô "failed" được mở lại chuyển hẳn sang "reopened" (không còn
          // "failed"), nên tự động rớt khỏi danh sách này, không kẹt lại
          // thành thẻ ma.
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
