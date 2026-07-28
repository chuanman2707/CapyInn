import { type ReactNode, useRef } from "react";

interface KeepMountedProps {
  /** Có đang là tab đang xem hay không. */
  active: boolean;
  children: ReactNode;
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
export default function KeepMounted({ active, children }: KeepMountedProps) {
  const everActive = useRef(active);
  if (active) everActive.current = true;

  if (!everActive.current) return null;

  return <div className={active ? "" : "hidden"}>{children}</div>;
}
