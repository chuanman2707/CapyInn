import { fmtMoney } from "@/lib/format";
import type { MoneyVnd } from "@/lib/money";
import type { RoomTypeRate } from "@/types";

/**
 * Giá niêm yết một màn hình được phép in cạnh một phòng.
 *
 * Một chỗ duy nhất quyết định chuyện này, vì bốn màn hình đang hỏi cùng câu hỏi
 * và câu trả lời có một luật khó chịu: **không có số dự phòng.** Chưa đọc được
 * bảng giá thì in "—". `room.base_price` không phải giá — engine bỏ qua nó ngay
 * khi loại phòng có `pricing_rules`, nên lấy nó ra lấp chỗ trống là quay lại
 * đúng thứ vừa gỡ: thẻ phòng đọc 300k trong khi quầy thu 480k.
 */
export interface NightlyRateDisplay {
  /** Đã format sẵn, hoặc "—" khi chưa biết. */
  text: string;
  /** Chưa đọc được giá — khác với "giá bằng 0". */
  unknown: boolean;
  /**
   * Giá này suy ra từ `base_price` của một phòng (hoặc mặc định nhà), không ai
   * đặt. Màn hình nào có chỗ thì nói rõ, vì cách sửa là vào cấu hình bảng giá.
   */
  derived: boolean;
}

const UNKNOWN: NightlyRateDisplay = { text: "—", unknown: true, derived: false };

export function nightlyRateDisplay(
  rates: Record<string, RoomTypeRate> | null,
  roomType: string,
): NightlyRateDisplay {
  const rate = rates?.[roomType];
  if (!rate) return UNKNOWN;

  return {
    text: fmtMoney(rate.nightly_rate),
    unknown: false,
    derived: !rate.configured,
  };
}

/**
 * `rooms.base_price` thực sự làm gì với một loại phòng cụ thể.
 *
 * Có ba khả năng và người sửa phòng không có cách nào đoán được đang ở khả năng
 * nào — trong khi khác biệt là "số vừa gõ có thành tiền khách trả hay không":
 *
 * - `ignored`: loại phòng đã có `pricing_rules`. Engine bỏ qua `base_price`
 *   hoàn toàn. Gõ 800.000 vào đây rồi tưởng đã tăng giá là mất tiền thật.
 * - `derives-type-price`: loại phòng chưa có bảng giá, nên `base_price` của
 *   phòng có **mã nhỏ nhất** trong loại trở thành giá của cả loại. Sửa phòng
 *   khác thì không có tác dụng gì.
 * - `unknown`: chưa đọc được bảng giá, nên không khẳng định được điều gì.
 */
export type BasePriceRole =
  | { kind: "unknown" }
  | { kind: "ignored"; typeRateText: string }
  | { kind: "derives-type-price" };

/**
 * `true` khi giá gốc của một phòng không phải số khách trả cho phòng đó.
 *
 * Đúng cho cả hai lý do: loại phòng đã có bảng giá (nên `base_price` là dữ liệu
 * chết), hoặc chưa có bảng giá nhưng một phòng khác cùng loại có mã nhỏ hơn và
 * giá của nó thắng. Cả hai đều nghĩa là số admin gõ vào phòng này không được thu.
 *
 * Không đọc được bảng giá thì trả `false`: không biết giá loại phòng thì không
 * kết luận được `base_price` có được dùng hay không, và một cảnh báo đoán mò còn
 * tệ hơn không cảnh báo.
 */
export function basePriceIsUnused(
  rates: Record<string, RoomTypeRate> | null,
  roomType: string,
  basePrice: MoneyVnd,
): boolean {
  const rate = rates?.[roomType];
  if (!rate) return false;

  return rate.nightly_rate !== basePrice;
}

export function basePriceRole(
  rates: Record<string, RoomTypeRate> | null,
  roomType: string,
): BasePriceRole {
  const rate = rates?.[roomType];
  if (!rate) return { kind: "unknown" };
  if (!rate.configured) return { kind: "derives-type-price" };

  return { kind: "ignored", typeRateText: fmtMoney(rate.nightly_rate) };
}
