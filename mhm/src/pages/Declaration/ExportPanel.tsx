import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationExportResult, DeclarationRow } from "@/types";

import { EXCEL_WARNING_BODY, EXCEL_WARNING_HEAD, isForeign } from "./catalog";
import { declarationErrorMessage } from "./declarationError";

interface ExportPanelProps {
  eligible: DeclarationRow[];
  blockedCount: number;
  onExported: () => void;
  /**
   * Lần gọi kbtt_validate gần nhất thất bại (FINDING I5) — không còn biết
   * chắc khách nào đủ điều kiện, nên KHÔNG mời xuất ai, kể cả khi `eligible`
   * (tính từ dữ liệu cũ) trông vẫn đầy đủ.
   */
  checkFailed?: boolean;
  /** Số khách đang hoạt động mà lần kiểm tra chưa xong — không phải lỗi. */
  pendingCount?: number;
  /**
   * FINDING 2: trang không unmount khi đổi tab (`KeepMounted`) và tự tải
   * lại dữ liệu khi quay lại — `reloadKey` đổi giá trị là tín hiệu DUY NHẤT
   * báo điều đó (xem `Declaration/index.tsx`). Banner kết quả xuất bên dưới
   * mô tả đúng MỘT lần xuất cụ thể; nếu không dọn theo, người vận hành rời
   * tab, quay lại thấy danh sách khách đã tải lại nhưng banner cũ (kèm nút
   * "Mở thư mục") vẫn còn, trỏ vào một lô không còn khớp màn hình.
   */
  reloadKey?: number;
}

/**
 * Một nút xuất duy nhất (spec UX §4.2). Máy tự chia theo quốc tịch: khách
 * Việt → XLSX, khách nước ngoài → XML — tối đa hai lần gọi `kbtt_export`.
 * Khách còn lỗi chặn KHÔNG chặn cả đoàn nhưng cũng không bị bỏ rơi im lặng:
 * họ ở lại danh sách với viền đỏ và nút này nói rõ điều đó.
 */
export default function ExportPanel({
  eligible,
  blockedCount,
  onExported,
  checkFailed = false,
  pendingCount = 0,
  reloadKey = 0,
}: ExportPanelProps) {
  const [exporting, setExporting] = useState(false);
  const [results, setResults] = useState<DeclarationExportResult[]>([]);

  // FINDING 2: `onExported()` (gọi bên dưới, cả nhánh thành công lẫn nhánh
  // lỗi-có-kết-quả-riêng) khiến CHA cũng bump `reloadKey` — lần đổi đó là hệ
  // quả trực tiếp của chính lần xuất vừa xong nên KHÔNG được dọn banner của
  // chính nó. `justExportedRef` đánh dấu đúng một lần đổi `reloadKey` sắp
  // tới là do vậy, bỏ qua; mọi lần đổi `reloadKey` KHÁC (đổi tab quay lại,
  // thả ảnh mới, đối soát xong ở ReconcilePanel...) đều dọn — không có lần
  // xuất mới nào vừa xảy ra để banner còn mô tả đúng.
  const justExportedRef = useRef(false);

  useEffect(() => {
    if (justExportedRef.current) {
      justExportedRef.current = false;
      return;
    }
    setResults([]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reloadKey]);

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
      justExportedRef.current = true;
      onExported();
    } catch (e) {
      toast.error(declarationErrorMessage(e));
      // File đã xuất xong trước lỗi vẫn hiện — người dùng cần biết cái gì đã ra.
      setResults(done);
      if (done.length > 0) {
        justExportedRef.current = true;
        onExported();
      }
    } finally {
      setExporting(false);
    }
  };

  return (
    <section className="rounded-2xl border border-slate-200 bg-white p-5">
      <Button
        onClick={() => void runExport()}
        disabled={eligible.length === 0 || exporting || checkFailed}
        className="rounded-xl"
      >
        {exporting
          ? "Đang xuất..."
          : eligible.length > 0
            ? `Xuất file cho ${eligible.length} khách`
            : "Xuất file"}
      </Button>

      {checkFailed && (
        <p className="mt-2 text-sm text-red-700">
          ⛔ Kiểm tra lỗi thất bại — chưa rõ khách nào đủ điều kiện xuất, chưa
          xuất được lúc này. Thử tải lại hoặc sửa một thẻ bất kỳ để kiểm lại.
        </p>
      )}

      {!checkFailed && pendingCount > 0 && (
        <p className="mt-2 text-sm text-slate-500">
          Đang kiểm tra {pendingCount} khách mới — chưa đưa vào lượt xuất này.
        </p>
      )}

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
