import { type ReactNode, useEffect, useRef } from "react";

interface KeepMountedProps {
  /** Có đang là tab đang xem hay không. */
  active: boolean;
  children: ReactNode;
  /**
   * FINDING C2: gọi đúng một lần mỗi khi `active` chuyển từ `false` sang
   * `true` SAU KHI children đã từng active ít nhất một lần trước đó — tức là
   * một lần THỰC SỰ QUAY LẠI, không phải lần đầu tiên được xem. Lần đầu tiên
   * được xem (dù mount thẳng ở trạng thái active, hay mount ẩn rồi mới được
   * xem lần đầu) không gọi — mount đã tự tải dữ liệu của nó rồi, gọi thêm ở
   * đây chỉ tạo một lượt tải trùng ngay sau lượt tải ban đầu. KHÔNG gọi lại
   * chừng nào `active` còn giữ nguyên `true` qua các lần re-render khác. Giữ
   * state React sống qua tab switch (xem doc dưới) không có nghĩa dữ liệu lấy
   * từ server bên trong vẫn còn đúng — `onActivate` là tín hiệu để cha bơm
   * lại reloadKey của mình đúng lúc quay lại tab, tách bạch "giữ state" khỏi
   * "tải lại dữ liệu server".
   */
  onActivate?: () => void;
}

/**
 * FINDING D: `{activeTab === "declaration" && <Declaration />}` unmount
 * toàn bộ cây khi rời tab — mọi state React sống trong đó (thẻ vừa quét chưa
 * lưu của `DropZone`, form `ManualForm` đang mở dở) biến mất không một tiếng
 * báo. Thiết kế gốc bắt buộc "mọi trạng thái dở dang phải sống qua tắt/mở
 * app" (§2 spec); riêng việc CHUYỂN TAB trong một phiên thì không có lý do
 * kỹ thuật nào bắt phải unmount.
 *
 * `KeepMounted` chỉ mount `children` lần đầu tab được xem, sau đó GIỮ
 * NGUYÊN cây React và chỉ ẩn/hiện bằng CSS — state bên trong sống qua mọi
 * lần chuyển tab tiếp theo trong CÙNG một phiên chạy app.
 *
 * KHÔNG sống qua khởi động lại app thật (đóng rồi mở lại tiến trình) — đó là
 * state trong bộ nhớ, không phải một lớp lưu trữ mới. Với `DropZone`/
 * `ManualForm`, đó là đánh đổi có chủ ý (xem doc-comment ở `MainShell.tsx`):
 * card chưa lưu là dữ liệu CHƯA XÁC NHẬN theo thiết kế, thêm một lớp lưu trữ
 * cho nó nằm ngoài phạm vi finding.
 */
export default function KeepMounted({ active, children, onActivate }: KeepMountedProps) {
  const everActive = useRef(active);
  if (active) everActive.current = true;

  // FINDING C2: `wasActive` bắt đầu bằng đúng giá trị `active` ban đầu, nên
  // lần render đầu tiên KHÔNG bao giờ trông như một "chuyển từ ẩn sang hiện"
  // — dù mount lần đầu ở trạng thái active hay không.
  const wasActive = useRef(active);
  // `hasActivatedBefore` phân biệt LẦN ĐẦU TIÊN được xem (mount đã tự tải dữ
  // liệu của nó — gọi `onActivate` ở đây chỉ tạo một lượt tải trùng ngay sau
  // lượt ban đầu) khỏi một lần QUAY LẠI thật sự (đã từng active, đã ẩn đi,
  // giờ hiện lại — dữ liệu có thể đã cũ, cần tín hiệu tải lại).
  const hasActivatedBefore = useRef(false);
  useEffect(() => {
    if (active && !wasActive.current && hasActivatedBefore.current) {
      onActivate?.();
    }
    if (active) hasActivatedBefore.current = true;
    wasActive.current = active;
  }, [active, onActivate]);

  if (!everActive.current) return null;

  return <div className={active ? "" : "hidden"}>{children}</div>;
}
