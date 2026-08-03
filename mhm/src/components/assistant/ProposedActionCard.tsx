import { AlertTriangle } from "lucide-react";

import { Button } from "@/components/ui/button";
import { isActionExpired, type ProposedAction } from "@/types/assistant";

const FIELD_LABELS: Record<string, string> = {
  room_id: "Phòng",
  guests: "Khách",
  nights: "Số đêm",
  source: "Nguồn",
  notes: "Ghi chú",
  paid_amount: "Trả trước",
  pricing_type: "Kiểu tính giá",
  total: "Tổng tiền",
};

type ProposedActionCardProps = {
  action: ProposedAction;
  busy: boolean;
  nowMs: number;
  onApprove: () => void;
  onRebuild: () => void;
  onDismiss: () => void;
};

export function ProposedActionCard({
  action,
  busy,
  nowMs,
  onApprove,
  onRebuild,
  onDismiss,
}: ProposedActionCardProps) {
  const expired = isActionExpired(action, nowMs);

  return (
    <div className="rounded-xl border border-brand-primary/30 bg-white p-4 shadow-soft">
      <p className="mb-3 text-sm font-semibold">Xác nhận nhận phòng</p>

      <dl className="space-y-1.5 text-sm">
        {Object.entries(action.display).map(([key, value]) => (
          <div key={key} className="flex justify-between gap-4">
            <dt className="text-brand-muted">{FIELD_LABELS[key] ?? key}</dt>
            <dd className="text-right font-medium">{value}</dd>
          </div>
        ))}
      </dl>

      {action.warnings.length > 0 && (
        <ul className="mt-3 space-y-1">
          {action.warnings.map((warning) => (
            <li key={warning} className="flex items-start gap-2 text-xs text-amber-700">
              <AlertTriangle size={14} className="mt-0.5 shrink-0" />
              {warning}
            </li>
          ))}
        </ul>
      )}

      <div className="mt-4 flex gap-2">
        {expired ? (
          <>
            <p className="flex-1 text-xs text-brand-muted">
              Thẻ đã quá 5 phút, giá có thể đã đổi. Tính lại trước khi duyệt.
            </p>
            <Button size="sm" onClick={onRebuild}>
              Tính lại
            </Button>
          </>
        ) : (
          <Button size="sm" disabled={busy} onClick={onApprove}>
            Đồng ý
          </Button>
        )}
        <Button size="sm" variant="ghost" disabled={busy} onClick={onDismiss}>
          Huỷ
        </Button>
      </div>
    </div>
  );
}
