import { useState } from "react";

import { Button } from "@/components/ui/button";
import type { DeclarationIdentity, DeclarationRow } from "@/types";

import BatchHistory from "./BatchHistory";
import DropZone from "./DropZone";
import ExportPanel from "./ExportPanel";
import PendingList from "./PendingList";

/**
 * Màn khai báo tạm trú — bốn khối theo §11 của spec.
 *
 * Trang này không đụng gì tới luồng check-in đang chạy: không sửa
 * `CheckinSheet`, không đụng `watcher.rs` hay thư mục `Scans/`. Đó là điều kiện
 * để PMS đang vận hành thật không bị ảnh hưởng bởi một tính năng phụ.
 */
export default function Declaration() {
  const [pendingIdentity, setPendingIdentity] = useState<DeclarationIdentity | null>(null);
  const [selectedLinkIds, setSelectedLinkIds] = useState<string[]>([]);
  const [rows, setRows] = useState<DeclarationRow[]>([]);
  const [reloadKey, setReloadKey] = useState(0);
  const [kind, setKind] = useState<"NNN" | "VN">("VN");

  const bump = () => setReloadKey((k) => k + 1);

  // Hai định dạng, hai file khác nhau. Phân loại theo quốc tịch trích từ ảnh,
  // KHÔNG theo `guests.guest_type` — cột đó không tin được.
  const selectedRows = rows.filter((r) => selectedLinkIds.includes(r.link_id));
  const foreignSelected = selectedRows.filter((r) => r.nationality_iso3 !== "VNM");
  const vietnameseSelected = selectedRows.filter((r) => r.nationality_iso3 === "VNM");

  return (
    <div className="flex flex-col gap-6">
      <DropZone
        onIdentitySaved={(_id, identity) => {
          setPendingIdentity(identity);
          bump();
        }}
      />

      <PendingList
        identity={pendingIdentity}
        reloadKey={reloadKey}
        onLinked={() => {
          setPendingIdentity(null);
          bump();
        }}
        onSelectionChange={setSelectedLinkIds}
        onRowsChange={setRows}
      />

      {/* Cảnh báo cố định, không có nút đóng — xem §11 Khối 3. */}
      <div
        data-excel-warning
        className="rounded-xl border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900"
      >
        <strong>
          Không mở/sửa file này bằng Excel trước khi upload.
        </strong>{" "}
        Excel sẽ làm mất số 0 đầu của số giấy tờ và đổi định dạng ngày.{" "}
        Cần sửa thì sửa trong CapyInn rồi xuất lại.
      </div>

      <div className="flex gap-2">
        <Button
          variant={kind === "NNN" ? "secondary" : "ghost"}
          onClick={() => setKind("NNN")}
        >
          Khách nước ngoài (XML) · {foreignSelected.length}
        </Button>
        <Button
          variant={kind === "VN" ? "secondary" : "ghost"}
          onClick={() => setKind("VN")}
        >
          Khách Việt Nam (XLSX) · {vietnameseSelected.length}
        </Button>
      </div>

      <ExportPanel
        kind={kind}
        linkIds={(kind === "NNN" ? foreignSelected : vietnameseSelected).map(
          (r) => r.link_id,
        )}
        onExported={bump}
      />

      <BatchHistory refreshKey={reloadKey} />
    </div>
  );
}
