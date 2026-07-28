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
  // FINDING I5 — nếu kbtt_validate thất bại (hoặc chưa kịp trả lời cho lô
  // khách hiện tại), trang này KHÔNG được coi các khách đó là đủ điều kiện
  // xuất. `checkFailed` chặn cả lô một cách rõ ràng; `uncheckedLinkIds` chỉ
  // chặn đúng những khách chưa có kết quả (vd. vừa thả ảnh xong) mà không
  // đụng tới các khách đã biết chắc là sạch — tránh nút xuất nhấp nháy mờ
  // trên mỗi lần sửa một thẻ bình thường.
  const [checkFailed, setCheckFailed] = useState(false);
  const [uncheckedLinkIds, setUncheckedLinkIds] = useState<string[]>([]);

  const bump = () => setReloadKey((k) => k + 1);

  const blockingLinks = new Set(
    findings.filter((f) => f.severity === "blocking").map((f) => f.link_id),
  );
  const unchecked = new Set(uncheckedLinkIds);
  const active = rows.filter((r) => !r.held);

  const blockedByFinding = active.filter((r) => blockingLinks.has(r.link_id));
  const pending = active.filter(
    (r) => !blockingLinks.has(r.link_id) && (checkFailed || unchecked.has(r.link_id)),
  );
  const eligible = active.filter(
    (r) => !blockingLinks.has(r.link_id) && !checkFailed && !unchecked.has(r.link_id),
  );

  return (
    <div className="flex flex-col gap-6">
      <DropZone onIdentitySaved={bump} />

      <GuestList
        reloadKey={reloadKey}
        onStateChange={({ rows: r, findings: f, checkFailed: cf, uncheckedLinkIds: u }) => {
          setRows(r);
          setFindings(f);
          setCheckFailed(cf);
          setUncheckedLinkIds(u);
        }}
      />

      <ExportPanel
        eligible={eligible}
        blockedCount={blockedByFinding.length}
        pendingCount={pending.length}
        checkFailed={checkFailed}
        onExported={bump}
      />

      <ReconcilePanel reloadKey={reloadKey} onSettled={bump} />

      <BatchHistory refreshKey={reloadKey} />
    </div>
  );
}
