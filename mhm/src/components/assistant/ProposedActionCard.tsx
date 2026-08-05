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
    <div className="rounded-xl border border-brand-primary/30 bg-white p-5 shadow-soft">
      <p className="mb-3 text-sm font-semibold">Xác nhận nhận phòng</p>

      {/* `divide-y` thay `space-y`: các dòng của thẻ là một BẢNG giá trị mà
          người bấm phải soi từng dòng (có cả số CCCD), nên đường kẻ giữa các
          dòng giúp mắt bám hàng tốt hơn khoảng trắng. Đệm dọc chuyển từ khoảng
          cách giữa các dòng (`space-y-1.5`) sang đệm trong từng dòng (`py-1.5`)
          — nếu không thì đường kẻ dính sát chữ. */}
      <dl className="divide-y divide-slate-100 text-sm">
        {Object.entries(action.display).map(([key, value]) => (
          <div key={key} className="flex justify-between gap-4 py-1.5">
            <dt className="text-brand-muted">{FIELD_LABELS[key] ?? key}</dt>
            <dd className="text-right font-medium">{value}</dd>
          </div>
        ))}
      </dl>

      {/* Gom cảnh báo vào một viên nền vàng thay vì để chúng trôi giữa bảng và
          hàng nút.

          Bọc phải nằm TRONG câu điều kiện, không được bọc-luôn-vẽ-nội-dung-có-
          điều-kiện: một viên nền vàng RỖNG thường trực vẫn làm mọi test dò chữ
          xanh, vì jsdom không nhìn thấy nền. `aria-label` ở đây không phải
          trang trí — nó là thứ duy nhất cho test khẳng định được "KHÔNG có viên
          cảnh báo nào". Cùng bẫy mà `historyNotice` đã dính (AssistantPanel.tsx:346). */}
      {action.warnings.length > 0 && (
        <ul aria-label="Cảnh báo từ PMS" className="mt-3 space-y-1 rounded-lg bg-amber-50 p-3">
          {action.warnings.map((warning, index) => (
            <li
              key={`${warning}-${index}`}
              className="flex items-start gap-2 text-xs text-amber-700"
            >
              <AlertTriangle size={14} className="mt-0.5 shrink-0" />
              {warning}
            </li>
          ))}
        </ul>
      )}

      <div className="mt-5 flex items-center gap-2">
        {expired ? (
          <>
            <p className="flex-1 text-xs text-brand-muted">
              Thẻ đã quá 5 phút, giá có thể đã đổi. Tính lại trước khi duyệt.
            </p>
            <Button size="sm" disabled={busy} onClick={onRebuild}>
              Tính lại
            </Button>
          </>
        ) : (
          <>
            {busy && (
              <p role="status" aria-live="polite" className="flex-1 text-xs text-brand-muted">
                Đang gửi lệnh nhận phòng…
              </p>
            )}
            <Button size="sm" disabled={busy} onClick={onApprove}>
              Đồng ý
            </Button>
          </>
        )}
        <Button size="sm" variant="ghost" disabled={busy} onClick={onDismiss}>
          Huỷ
        </Button>
      </div>
    </div>
  );
}
