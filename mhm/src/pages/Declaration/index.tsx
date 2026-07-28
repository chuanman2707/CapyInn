import { useEffect, useRef, useState } from "react";

import { invokeCommand } from "@/lib/invokeCommand";
import type { DeclarationFinding, DeclarationRow, DeclarationUndeclaredBreakdown } from "@/types";

import BatchHistory from "./BatchHistory";
import DropZone from "./DropZone";
import ExportPanel from "./ExportPanel";
import GuestList from "./GuestList";
import ReconcilePanel from "./ReconcilePanel";

interface DeclarationProps {
  /**
   * FINDING C2: trang này giờ sống qua tab switch (`KeepMounted` ở
   * `MainShell`), nên không có gì unmount/remount để tự tải lại dữ liệu khi
   * quay lại tab — badge, `GuestList` (`kbtt_pending_rows`/`kbtt_list_stays`)
   * và `ReconcilePanel` (`kbtt_list_batches`) đều chỉ tải lại theo
   * `reloadKey`/`localReload`. `reactivateSignal` là số đếm cha bơm vào mỗi
   * lần tab này chuyển từ ẩn sang hiện (xem `KeepMounted.onActivate` ở
   * `MainShell.tsx`) — component tự bump `reloadKey` của mình khi thấy giá
   * trị này đổi. Giữ state React sống qua tab switch KHÔNG có nghĩa dữ liệu
   * server bên trong vẫn còn đúng — đây là hai việc tách bạch.
   */
  reactivateSignal?: number;
  /**
   * FINDING I2: có đang là tab đang xem hay không — cùng tín hiệu visibility
   * với `reactivateSignal`, truyền tiếp xuống `DropZone` để listener
   * `tauri://drag-drop` (đăng ký một lần, sống suốt phiên) không xử lý ảnh
   * thả vào lúc trang này đang ẩn sau một tab khác.
   */
  visible?: boolean;
}

/**
 * Màn khai báo tạm trú — "băng chuyền một chiều"
 * (spec docs/superpowers/specs/2026-07-27-kbtt-ux-simplify-design.md §4):
 * thả ảnh → danh sách khách → một nút xuất → đối chiếu. Mỗi lúc một nút.
 *
 * Vẫn không đụng gì tới luồng check-in đang chạy: không sửa `CheckinSheet`,
 * không đụng `watcher.rs` hay `Scans/`.
 */
export default function Declaration({ reactivateSignal, visible = true }: DeclarationProps) {
  const [reloadKey, setReloadKey] = useState(0);
  const [rows, setRows] = useState<DeclarationRow[]>([]);
  const [findings, setFindings] = useState<DeclarationFinding[]>([]);
  const [breakdown, setBreakdown] = useState<DeclarationUndeclaredBreakdown | null>(null);
  // FINDING I5 — nếu kbtt_validate thất bại (hoặc chưa kịp trả lời cho lô
  // khách hiện tại), trang này KHÔNG được coi các khách đó là đủ điều kiện
  // xuất. `checkFailed` chặn cả lô một cách rõ ràng; `uncheckedLinkIds` chỉ
  // chặn đúng những khách chưa có kết quả (vd. vừa thả ảnh xong) mà không
  // đụng tới các khách đã biết chắc là sạch — tránh nút xuất nhấp nháy mờ
  // trên mỗi lần sửa một thẻ bình thường.
  const [checkFailed, setCheckFailed] = useState(false);
  const [uncheckedLinkIds, setUncheckedLinkIds] = useState<string[]>([]);

  const bump = () => setReloadKey((k) => k + 1);

  // FINDING C2: bỏ qua giá trị `reactivateSignal` LÚC MOUNT — mount ban đầu
  // đã tự tải đúng một lần qua `reloadKey` khởi tạo (0), bump thêm ở đây sẽ
  // gọi trùng. Chỉ những lần giá trị này ĐỔI sau đó (quay lại tab) mới bump.
  const isFirstReactivate = useRef(true);
  useEffect(() => {
    if (isFirstReactivate.current) {
      isFirstReactivate.current = false;
      return;
    }
    bump();
    // bump() không stable qua closure ở trên nhưng luôn cùng một hành vi
    // (tăng reloadKey) — không cần trong deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reactivateSignal]);

  useEffect(() => {
    invokeCommand<DeclarationUndeclaredBreakdown>("kbtt_undeclared_breakdown")
      .then(setBreakdown)
      .catch(() => setBreakdown(null));
  }, [reloadKey]);

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

  // Dòng diễn giải badge sidebar: badge cộng bốn nguồn, có chủ ý đếm thừa
  // hơn đếm thiếu (xem `undeclared_breakdown` phía Rust). `not_scanned` là
  // lưu trú PMS chưa xác nhận khai báo — bucket duy nhất còn khác 0 ngay lần
  // mở app đầu tiên sau khi nâng cấp, lúc danh sách "Cần khai" còn trống.
  // Bỏ qua bucket = 0 để dòng không dài dòng vô ích.
  const breakdownParts: string[] = [];
  if (breakdown?.not_scanned) breakdownParts.push(`${breakdown.not_scanned} lưu trú chưa xác nhận`);
  if (breakdown?.not_exported) breakdownParts.push(`${breakdown.not_exported} chưa xuất file`);
  if (breakdown?.awaiting) breakdownParts.push(`${breakdown.awaiting} chờ đối chiếu`);
  if (breakdown?.held) breakdownParts.push(`${breakdown.held} gác lại`);

  // Caveat khi có overlap: một khách PMS có thể nằm ở cả not_scanned và một
  // bucket link khác (chưa xuất, chờ đối chiếu, gác lại) nên số này có thể
  // cao hơn số khách thực tế. Chỉ hiện caveat khi có thể overlap xảy ra.
  const hasOverlap = breakdown && breakdown.not_scanned > 0 && (breakdown.not_exported > 0 || breakdown.awaiting > 0 || breakdown.held > 0);
  const caveat = hasOverlap ? ", một số khách ghi nhận ở nhiều mục" : "";

  return (
    <div className="flex flex-col gap-6">
      {breakdown && breakdown.total > 0 && (
        <p className="text-sm text-brand-muted">
          {breakdown.total} khách chưa khai xong: {breakdownParts.join(" · ")}{caveat}
        </p>
      )}

      <DropZone onIdentitySaved={bump} active={visible} />

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
