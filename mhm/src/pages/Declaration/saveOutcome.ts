import type { DeclarationExistingLocation } from "@/types";

/**
 * FINDING I1: quét lại một khách đã có khai báo đang hoạt động (chưa
 * verified) không tạo thêm dòng nào — `kbtt_save_identity` chỉ khớp lại và
 * trả `created_new_link: false`. Trước fix, toast vẫn nói "Đã lưu danh tính"
 * y hệt lần tạo mới, nên người vận hành không có cách nào biết là không có
 * gì mới xảy ra và khách đó không hề "biến mất" — nó chưa từng được tạo
 * thêm. Câu này thay vào đúng lúc đó, và nói rõ khách hiện đang ở đâu để họ
 * biết tìm ở màn nào (danh sách chờ, hay thẻ đối chiếu).
 */
export function matchedExistingDeclarationMessage(
  location: DeclarationExistingLocation | null,
): string {
  const where =
    location === "awaiting_reconciliation"
      ? "đang chờ đối chiếu trên một lô đã xuất"
      : "đang nằm trong danh sách chờ khai";
  return `Số giấy tờ này khớp một khách đã có khai báo — khách đó ${where}, không tạo thêm dòng nào.`;
}
