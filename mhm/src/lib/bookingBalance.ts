import { fmtMoney } from "@/lib/format";
import type { MoneyVnd } from "@/lib/money";

/**
 * Một booking đứng ở đâu giữa số đã thu và số phải thu.
 *
 * Ba trạng thái, và trạng thái thứ ba **chặn** việc trả phòng:
 * `CheckoutSettlementModal` không cho bấm xác nhận khi đã thu quá và bảo xử lý
 * hoàn tiền trước. Nhưng luật đó chỉ nằm trong phần render của đúng cái modal ấy
 * — nên hai màn hình mà lễ tân nhìn thấy trước lại vẽ cùng trạng thái đó theo hai
 * kiểu khác nhau, không kiểu nào nói rằng nó đang chặn:
 *
 * - `RoomDetailPanel` hiện "Còn nợ −3.800.000đ" màu xám nhạt — đọc như *không nợ gì*.
 * - `GroupManagement` hiện "Còn lại −3.800.000đ" đỏ đậm — đọc như khách nợ một số âm.
 *
 * Cả hai đều là số vô nghĩa. Số có nghĩa là **3.800.000đ phải trả lại khách**.
 */
export type BookingBalance =
  | { kind: "owed"; amount: MoneyVnd }
  | { kind: "settled" }
  | { kind: "overpaid"; refund: MoneyVnd };

export function bookingBalance(total: MoneyVnd, paid: MoneyVnd): BookingBalance {
  if (paid > total) return { kind: "overpaid", refund: paid - total };
  if (paid === total) return { kind: "settled" };

  return { kind: "owed", amount: total - paid };
}

export type BalanceTone = "owed" | "settled" | "overpaid";

export interface BalanceDisplay {
  /** Nhãn đổi theo trạng thái: "Còn nợ" và "Trả lại khách" không cùng nghĩa. */
  label: string;
  /** Luôn là số dương — dấu đã nằm trong nhãn. */
  text: string;
  tone: BalanceTone;
}

export function balanceDisplay(total: MoneyVnd, paid: MoneyVnd): BalanceDisplay {
  const balance = bookingBalance(total, paid);

  switch (balance.kind) {
    case "overpaid":
      return { label: "Trả lại khách", text: fmtMoney(balance.refund), tone: "overpaid" };
    case "settled":
      return { label: "Đã đủ", text: fmtMoney(0), tone: "settled" };
    default:
      return { label: "Còn nợ", text: fmtMoney(balance.amount), tone: "owed" };
  }
}

/**
 * Một bảng màu duy nhất. Trước đây mỗi màn hình tự chọn: `text-red-600` ở panel,
 * `text-red-500` ở màn hình đoàn, và cả hai dùng đỏ cho một trạng thái mà đỏ là
 * sai nghĩa. Trả về `tone` mà để mỗi màn hình tự map thì chúng lại lệch nhau lần
 * nữa, nên map nằm ở đây.
 */
export const BALANCE_TONE_CLASS: Record<BalanceTone, string> = {
  owed: "text-red-600",
  settled: "text-slate-400",
  // Hổ phách, không phải đỏ: đây không phải khoản khách nợ, mà là việc nhà phải
  // làm trước khi trả phòng được.
  overpaid: "text-amber-600",
};
