import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/lib/invokeCommand";

interface ReconcileChecklistProps {
  batchId: string;
  expected: number;
  filePath?: string;
  onSettled?: (status: "verified" | "failed") => void;
}

/**
 * Vòng đối chiếu sau khi xuất file.
 *
 * Lý do tồn tại: cổng Bộ Công an báo "import thành công" ngay cả khi import 0
 * record, và chuyện đó đã xảy ra thật. Nếu người vận hành tin câu đó thì khách
 * không được khai mà không ai biết.
 *
 * Vì thế ở đây KHÔNG có nút nào đóng được một lô bằng một cú bấm. Hai checkbox
 * chỉ là nhắc việc; thứ duy nhất chốt được lô là con số gõ tay vào ô input. Đây
 * là chủ ý: checkbox sẽ bị tick theo phản xạ, còn ô nhập số buộc người ta phải
 * thật sự nhìn màn hình danh sách của cổng.
 */
export default function ReconcileChecklist({
  batchId,
  expected,
  filePath,
  onSettled,
}: ReconcileChecklistProps) {
  const [uploaded, setUploaded] = useState(false);
  const [refreshed, setRefreshed] = useState(false);
  const [seen, setSeen] = useState("");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<"verified" | "failed" | null>(null);

  const submit = async () => {
    if (seen === "") return;
    setBusy(true);
    try {
      const result = await invokeCommand<"verified" | "failed">("kbtt_reconcile", {
        batchId,
        seenCount: Number(seen),
      });
      setStatus(result);
      onSettled?.(result);
      if (result === "verified") {
        toast.success("Lô đã đối chiếu khớp.");
      }
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded-xl border border-slate-200 bg-white p-4">
      <div className="mb-3 flex flex-wrap items-baseline gap-2">
        <h3 className="font-semibold">Đối chiếu với cổng</h3>
        <span className="text-sm text-brand-muted">{expected} hồ sơ đã xuất</span>
        {filePath && (
          <code className="text-xs text-brand-muted">{filePath}</code>
        )}
      </div>

      <label className="mb-2 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={uploaded}
          onChange={(e) => setUploaded(e.target.checked)}
        />
        Đã upload lên cổng
      </label>
      <label className="mb-4 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={refreshed}
          onChange={(e) => setRefreshed(e.target.checked)}
        />
        Đã bấm &quot;Làm mới&quot; trên màn danh sách của cổng
      </label>

      <div className="flex flex-wrap items-end gap-3">
        <div>
          <label htmlFor="kbtt-seen" className="block text-xs text-brand-muted">
            Số hồ sơ thấy trên cổng
          </label>
          <input
            id="kbtt-seen"
            type="number"
            min={0}
            value={seen}
            onChange={(e) => setSeen(e.target.value)}
            className="mt-1 w-32 rounded-lg border border-slate-200 px-3 py-2"
          />
        </div>
        <span className="pb-2 text-sm text-brand-muted">
          (cần đúng {expected})
        </span>
        <Button onClick={submit} disabled={seen === "" || busy}>
          Xác nhận
        </Button>
      </div>

      {status === "verified" && (
        <p className="mt-3 rounded-lg bg-emerald-50 p-3 text-sm text-emerald-900">
          Lô đã đối chiếu khớp. {expected} khách chuyển sang trạng thái đã khai báo.
        </p>
      )}
      {status === "failed" && (
        <p className="mt-3 rounded-lg bg-red-50 p-3 text-sm text-red-900">
          <strong>Lô thất bại.</strong> Số trên cổng không khớp số đã xuất, nên
          {" "}{expected} khách vẫn ở trạng thái chưa khai báo và badge tiếp tục
          đếm họ. Kiểm tra lại file rồi upload lần nữa.
        </p>
      )}
    </div>
  );
}
