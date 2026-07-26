import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationExportResult, DeclarationFinding } from "@/types";

import ReconcileChecklist from "./ReconcileChecklist";

interface ExportPanelProps {
  linkIds: string[];
  kind: "NNN" | "VN";
  onExported?: () => void;
}

export default function ExportPanel({ linkIds, kind, onExported }: ExportPanelProps) {
  const [findings, setFindings] = useState<DeclarationFinding[]>([]);
  const [checking, setChecking] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [batch, setBatch] = useState<DeclarationExportResult | null>(null);

  const check = useCallback(async () => {
    if (linkIds.length === 0) {
      setFindings([]);
      return [];
    }
    setChecking(true);
    try {
      const result = await invokeCommand<DeclarationFinding[]>("kbtt_validate", {
        linkIds,
      });
      setFindings(result);
      return result;
    } catch (error) {
      toast.error(String(error));
      return [];
    } finally {
      setChecking(false);
    }
  }, [linkIds]);

  useEffect(() => {
    void check();
  }, [check]);

  const blocking = findings.filter((f) => f.severity === "blocking");
  const warnings = findings.filter((f) => f.severity === "warning");
  // Không có đường vòng: còn lỗi chặn là không xuất được.
  const canExport = linkIds.length > 0 && blocking.length === 0 && !exporting;

  const runExport = async () => {
    if (!canExport) return;
    setExporting(true);
    try {
      const result = await invokeCommand<DeclarationExportResult>("kbtt_export", {
        kind,
        linkIds,
      });
      setBatch(result);
      onExported?.();
      await invokeCommand("kbtt_open_export_dir", { batchId: result.batch_id });
    } catch (error) {
      toast.error(String(error));
    } finally {
      setExporting(false);
    }
  };

  return (
    <section className="rounded-2xl border border-slate-200 bg-white p-5">
      <div className="mb-4 flex flex-wrap items-center gap-3">
        <h2 className="text-lg font-semibold">Xuất file</h2>
        <Badge variant="secondary">
          {kind === "NNN" ? "Khách nước ngoài (XML)" : "Khách Việt Nam (XLSX)"}
        </Badge>
        <span className="text-sm text-brand-muted">
          {linkIds.length} hồ sơ đã chọn
        </span>
      </div>

      <div className="mb-4 flex gap-2">
        <Button variant="secondary" onClick={check} disabled={checking}>
          Kiểm tra
        </Button>
        <Button onClick={runExport} disabled={!canExport}>
          Xuất file
        </Button>
      </div>

      {blocking.length > 0 && (
        <div className="mb-3 rounded-xl border border-red-200 bg-red-50 p-3">
          <p className="mb-2 text-sm font-medium text-red-900">
            {blocking.length} lỗi chặn — phải sửa trước khi xuất
          </p>
          <ul className="space-y-1 text-sm text-red-900">
            {blocking.map((f, i) => (
              <li key={`${f.code}-${f.link_id}-${i}`} className="flex gap-2">
                <span className="font-mono font-semibold">{f.code}</span>
                <span>{f.message}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {warnings.length > 0 && (
        <div className="mb-3 rounded-xl border border-amber-200 bg-amber-50 p-3">
          <p className="mb-2 text-sm font-medium text-amber-900">
            {warnings.length} cảnh báo — vẫn xuất được, nhưng nên xem
          </p>
          <ul className="space-y-1 text-sm text-amber-900">
            {warnings.map((f, i) => (
              <li key={`${f.code}-${f.link_id}-${i}`} className="flex gap-2">
                <span className="font-mono font-semibold">{f.code}</span>
                <span>{f.message}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {batch && (
        <ReconcileChecklist
          batchId={batch.batch_id}
          expected={batch.row_count}
          filePath={batch.file_path}
        />
      )}
    </section>
  );
}
