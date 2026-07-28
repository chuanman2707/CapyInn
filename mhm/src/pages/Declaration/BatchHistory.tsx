import { useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationBatch } from "@/types";

const STATUS_LABEL: Record<string, string> = {
  exported: "Đã xuất, chưa upload",
  uploaded: "Đã upload, chưa đối chiếu",
  verified: "Đã đối chiếu khớp",
  failed: "Thất bại",
  reopened: "Đã mở lại, khách về danh sách để sửa",
};

function statusTone(status: string): string {
  if (status === "verified") return "bg-emerald-50 text-emerald-900";
  if (status === "failed") return "bg-red-50 text-red-900";
  if (status === "reopened") return "bg-slate-100 text-slate-700";
  return "bg-amber-50 text-amber-900";
}

/**
 * Backend đã sắp lô `failed` và lô chưa đối chiếu lên đầu — người vận hành cần
 * thấy cái đang hỏng trước, không phải cuộn tìm.
 */
export default function BatchHistory({ refreshKey }: { refreshKey?: number }) {
  const [batches, setBatches] = useState<DeclarationBatch[]>([]);

  useEffect(() => {
    invokeCommand<DeclarationBatch[]>("kbtt_list_batches")
      .then(setBatches)
      .catch(() => setBatches([]));
  }, [refreshKey]);

  return (
    <details className="rounded-2xl border border-slate-200 bg-white p-5">
      <summary className="cursor-pointer text-sm font-semibold text-slate-500">
        Lịch sử xuất file ({batches.length})
      </summary>

      {batches.length === 0 ? (
        <p className="mt-4 text-sm text-brand-muted">Chưa có lô nào được xuất.</p>
      ) : (
        <div className="mt-4 overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs uppercase tracking-wide text-brand-muted">
                <th className="py-2">Ngày</th>
                <th>Loại</th>
                <th>Số hồ sơ</th>
                <th>Trạng thái</th>
                <th>File</th>
              </tr>
            </thead>
            <tbody>
              {batches.map((b) => (
                <tr key={b.id} className={`border-t border-slate-100 ${statusTone(b.status)}`}>
                  <td className="py-2">{b.created_at.slice(0, 10)}</td>
                  <td>
                    <Badge variant="secondary">{b.kind}</Badge>
                  </td>
                  <td>
                    {b.verified_count !== null && b.verified_count !== b.row_count
                      ? `${b.verified_count} / ${b.row_count}`
                      : b.row_count}
                  </td>
                  <td>{STATUS_LABEL[b.status] ?? b.status}</td>
                  <td className="max-w-[22rem] truncate">
                    <code className="text-xs">{b.file_path}</code>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </details>
  );
}
