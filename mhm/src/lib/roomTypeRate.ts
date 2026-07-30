import { fmtMoney } from "@/lib/format";
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
