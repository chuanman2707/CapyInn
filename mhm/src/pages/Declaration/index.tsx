import { useState } from "react";

import type { DeclarationFinding, DeclarationRow } from "@/types";

import BatchHistory from "./BatchHistory";
import DropZone from "./DropZone";
import ExportPanel from "./ExportPanel";
import GuestList from "./GuestList";
import ReconcilePanel from "./ReconcilePanel";

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

      <ReconcilePanel reloadKey={reloadKey} onSettled={bump} />

      <BatchHistory refreshKey={reloadKey} />
    </div>
  );
}
