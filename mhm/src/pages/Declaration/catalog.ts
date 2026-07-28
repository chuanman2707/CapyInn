/**
 * Danh mục dùng cho các ô chọn của module khai báo tạm trú.
 *
 * QUAN TRỌNG — nguồn sự thật là `kbtt_catalog.json` sinh từ template chính thức
 * (spec §4). Chuỗi `display` ghi vào file XLSX/XML **do Rust dựng từ `code`**,
 * frontend chỉ lưu `code`. Nhãn ở đây là nhãn cho người vận hành đọc, không bao
 * giờ được ghi thẳng vào file xuất.
 *
 * Frontend hiện KHÔNG có command nào đọc `kbtt_catalog.json`, nên chỉ liệt kê
 * những mã spec khẳng định chắc chắn. Khi backend có `kbtt_catalog`, thay các
 * hằng dưới đây bằng dữ liệu nạp từ đó.
 */

import type { DeclarationFinding } from "@/types";

export interface CatalogOption {
  code: string;
  label: string;
}

/** §4 nói có 9 mục; spec chỉ nêu đích danh 1, 8, 9. */
export const DOC_TYPES: CatalogOption[] = [
  { code: "1", label: "Thẻ CCCD" },
  { code: "8", label: "Thẻ Căn Cước" },
  { code: "9", label: "Giấy Tờ Khác" },
];

export const DOC_TYPE_OTHER = "9";

/** §4 nói có 20 mục; spec chỉ nêu đích danh 1 (mặc định) và 20. */
export const STAY_REASONS: CatalogOption[] = [
  { code: "1", label: "Du lịch" },
  { code: "20", label: "Mục đích khác" },
];

export const STAY_REASON_DEFAULT = "1";
export const STAY_REASON_OTHER = "20";

export const GENDERS: CatalogOption[] = [
  { code: "F", label: "Nữ" },
  { code: "M", label: "Nam" },
];

export const RESIDENCE_STATUSES: CatalogOption[] = [
  { code: "1", label: "Thường trú" },
  { code: "2", label: "Tạm trú" },
  { code: "3", label: "Nơi ở hiện tại" },
];

/**
 * Gợi ý cho ô quốc tịch. `E05` chỉ đòi ISO3 3 chữ HOA và `W07` cảnh báo mã
 * ngoài danh mục, nên ô này để nhập tự do kèm datalist thay vì khóa 205 mã.
 */
export const NATIONALITY_HINTS: CatalogOption[] = [
  { code: "VNM", label: "Việt Nam" },
  { code: "USA", label: "Hoa Kỳ" },
  { code: "GBR", label: "Anh" },
  { code: "FRA", label: "Pháp" },
  { code: "DEU", label: "Đức" },
  { code: "RUS", label: "Nga" },
  { code: "CHN", label: "Trung Quốc" },
  { code: "KOR", label: "Hàn Quốc" },
  { code: "JPN", label: "Nhật Bản" },
  { code: "AUS", label: "Úc" },
  { code: "IDN", label: "Indonesia" },
  { code: "THA", label: "Thái Lan" },
  { code: "MYS", label: "Malaysia" },
  { code: "SGP", label: "Singapore" },
  { code: "IND", label: "Ấn Độ" },
];

export const VIETNAM_ISO3 = "VNM";

export function isForeign(nationalityIso3: string | null | undefined): boolean {
  return (nationalityIso3 ?? "").toUpperCase() !== VIETNAM_ISO3;
}

export function sourceLabel(source: string): string {
  switch (source) {
    case "qr_cccd":
      return "QR CCCD";
    case "mrz_td3":
      return "MRZ hộ chiếu";
    default:
      return "Nhập tay";
  }
}

/** Cảnh báo Excel — text cố định, không có nút đóng (spec §11 khối 3). */
export const EXCEL_WARNING_HEAD =
  "Không mở/sửa file này bằng Excel trước khi upload.";
export const EXCEL_WARNING_BODY =
  "Excel sẽ làm mất số 0 đầu của số giấy tờ và đổi định dạng ngày. Cần sửa thì sửa trong CapyInn rồi xuất lại.";

/**
 * Câu tiếng người cho từng mã của validator (spec UX §4.2). Validator giữ
 * message kỹ thuật của nó; đây là lớp dịch cho người vận hành, kèm hướng sửa.
 * Mã không có trong bảng thì dùng message gốc — không bao giờ hiện rỗng.
 */
const FINDING_TEXT: Record<string, string> = {
  E01: "Thiếu thông tin bắt buộc — bấm để bổ sung.",
  E02: "Tên chỉ có một chữ — nếu giấy tờ đúng như vậy, bấm để xác nhận.",
  E03: "Tên khách nước ngoài phải viết HOA không dấu — bấm để sửa.",
  E04: "Chưa ai xác nhận tên đọc từ hộ chiếu — bấm để xác nhận.",
  E05: "Mã quốc tịch sai dạng (phải là 3 chữ HOA) — bấm để sửa.",
  E06: "Mã danh mục không hợp lệ — bấm để chọn lại.",
  E07: "Ngày đi dự kiến sớm hơn ngày đến — kiểm tra lại phòng đã ghép.",
  E08: "Thiếu thời hạn tạm trú (visa) — bấm để nhập.",
  E09: "Thời hạn tạm trú hết trước ngày đi dự kiến — khách sẽ quá hạn.",
  E10: "Thời hạn tạm trú trùng ngày hết hạn hộ chiếu — nghi nhập nhầm, bấm để sửa.",
  E11: "Chọn 'Giấy Tờ Khác' thì phải ghi tên giấy tờ — bấm để nhập.",
  E12: "Chọn 'Mục đích khác' thì phải ghi lý do cụ thể — bấm để nhập.",
  E13: "Số giấy tờ rỗng hoặc có ký tự lạ — bấm để sửa.",
  E14: "Trùng hồ sơ: cùng số giấy tờ, cùng ngày đến — xóa bớt một thẻ.",
  W01: "Chưa chọn phòng — vẫn xuất được, nhưng nên chọn.",
  W02: "Thiếu số điện thoại — vẫn xuất được.",
  W03: "Lý do vẫn là mặc định 'Du lịch' — đổi nếu không đúng.",
  W04: "Khách đến đã quá 24h mà chưa khai xong.",
  W05: "Thông tin trích từ ảnh cần người xem lại — bấm để kiểm.",
  W06: "Loại giấy tờ do máy đoán — bấm để xác nhận.",
  W07: "Quốc tịch này không có trong danh mục của cổng — kiểm tra lại sau khi nộp.",
};

export function findingText(f: DeclarationFinding): string {
  return FINDING_TEXT[f.code] ?? f.message;
}
