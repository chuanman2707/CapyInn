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
