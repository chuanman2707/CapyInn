import { formatAppError } from "@/lib/appError";

/**
 * Lớp lệnh khai báo tạm trú trả `Result<T, String>` thẳng, không đi qua
 * registry `AppError` dùng chung (`commands/declaration.rs` cố ý mỏng, không
 * kéo theo bộ máy đó — xem đầu file Rust). `formatAppError` chỉ nhận diện
 * được lỗi có đúng hình `{code, message, kind, support_id}` nằm trong
 * registry; một chuỗi lỗi tiếng Việt trần (đúng thứ Tauri trả về khi lệnh
 * `Err(String)`) bị nó rơi vào nhánh dự phòng chung chung "Có lỗi hệ thống,
 * vui lòng thử lại" — đúng lúc cần rõ nhất (FINDING I1/I2: người vận hành
 * phải đọc được vì sao, không phải một câu chung chung không nói gì).
 *
 * Dùng hàm này thay `formatAppError` ở những chỗ bắt lỗi từ lệnh `kbtt_*`.
 */
export function declarationErrorMessage(e: unknown): string {
  return typeof e === "string" ? e : formatAppError(e);
}
