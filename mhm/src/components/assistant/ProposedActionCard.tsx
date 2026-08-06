import { AlertTriangle } from "lucide-react";

import { Button } from "@/components/ui/button";
import { actionKindCopy, isActionExpired, type ProposedAction } from "@/types/assistant";

const FIELD_LABELS: Record<string, string> = {
  room_id: "Phòng",
  guests: "Khách",
  guest_name: "Khách",
  nights: "Số đêm",
  check_in_date: "Ngày nhận",
  check_out_date: "Ngày trả",
  expected_checkout_date: "Dự kiến trả",
  source: "Nguồn",
  notes: "Ghi chú",
  paid_amount: "Trả trước",
  deposit_amount: "Đặt cọc",
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
  // Ba loại thẻ, ba bộ chữ — xem `ACTION_KIND_COPY` (`types/assistant.ts`).
  //
  // Tra bằng hàm chứ không viết `if (kind === …)` ở đây: dòng tiêu đề, dòng
  // trạng thái và câu *Tính lại* phải đổi CÙNG NHAU theo loại thẻ, và ba câu
  // `if` rời nhau là ba chỗ trôi độc lập.
  const copy = actionKindCopy(action.kind);

  return (
    <div className="rounded-xl border border-brand-primary/30 bg-white p-5 shadow-soft">
      {/* Tiêu đề KHÁC NHAU BẰNG CHỮ giữa ba loại thẻ, không phân biệt bằng màu:
          lễ tân mù màu không có tín hiệu màu, mà dòng này là thứ duy nhất nói
          cho người sắp bấm *Đồng ý* biết họ đang ghi cái gì vào PMS. Thẻ này
          giữ nguyên một khung và một màu cho cả ba loại, nên chữ là toàn bộ
          tín hiệu — cố ý. */}
      <p className="mb-3 text-sm font-semibold">{copy.title}</p>

      {/* `divide-y` thay `space-y`: các dòng của thẻ là một BẢNG giá trị mà
          người bấm phải soi từng dòng (có cả số CCCD), nên đường kẻ giữa các
          dòng giúp mắt bám hàng tốt hơn khoảng trắng. Đệm dọc chuyển từ khoảng
          cách giữa các dòng (`space-y-1.5`) sang đệm trong từng dòng (`py-1.5`)
          — nếu không thì đường kẻ dính sát chữ. */}
      {/* `min-w-0` + `break-words` ở `dd`, `shrink-0` ở `dt`.

          Hàng `guests` là chuỗi DÀI NHẤT của thẻ: `draft.rs` dựng một dòng mỗi
          khách, giá trị là họ tên + CCCD + địa chỉ. Bề ngang có thật rất hẹp —
          panel 380px, `px-5` còn 340px, đệm `p-5` của thẻ còn ~300px — và một
          dãy CCCD 12 chữ số là token KHÔNG NGẮT ĐƯỢC. Mặc định flex item có
          `min-width: auto`, nghĩa là `dd` từ chối co lại dưới bề rộng nội dung,
          nên nó tràn thay vì xuống dòng; `min-w-0` gỡ đúng cái đó, còn
          `break-words` là thứ duy nhất bẻ được dãy số. `dt` giữ `shrink-0` để
          nhãn ("Khách", "Tổng tiền") không bị bóp theo.

          CHƯA ĐO ĐƯỢC TRÊN TRÌNH DUYỆT THẬT: jsdom không tính layout nên không
          test nào ở đây khẳng định được là chữ hết tràn — mọi test chỉ đọc lại
          được đúng cái className vừa viết. Cần một lượt QA tay: mở thẻ có một
          khách tên dài kèm CCCD 12 số ở panel 380px và nhìn cột phải. */}
      <dl className="divide-y divide-slate-100 text-sm">
        {Object.entries(action.display).map(([key, value]) => (
          <div key={key} className="flex justify-between gap-4 py-1.5">
            <dt className="shrink-0 text-brand-muted">{FIELD_LABELS[key] ?? key}</dt>
            <dd className="min-w-0 text-right font-medium break-words">{value}</dd>
          </div>
        ))}
      </dl>

      {/* Gom cảnh báo vào một viên nền vàng thay vì để chúng trôi giữa bảng và
          hàng nút.

          Bọc phải nằm TRONG câu điều kiện, không được bọc-luôn-vẽ-nội-dung-có-
          điều-kiện: một viên nền vàng RỖNG thường trực vẫn làm mọi test dò chữ
          xanh, vì jsdom không nhìn thấy nền. `aria-label` ở đây không phải
          trang trí — nó là thứ duy nhất cho test khẳng định được "KHÔNG có viên
          cảnh báo nào". Cùng bẫy mà `historyNotice` đã dính — xem viên
          `{historyNotice && …}` trong `AssistantPanel.tsx`.

          Trỏ bằng TÊN chứ không bằng số dòng: bản trước ghi
          `AssistantPanel.tsx:346`, mà dòng 346 nay là viên `role="alert"` của
          `error` — một thứ khác hẳn (báo hỏng việc, không phải tin phụ trợ).
          Người đọc đi theo con số ấy học nhầm bài học rồi bê `alert` sang đây. */}
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
                {copy.sending}
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
