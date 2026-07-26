//! Kiểu dữ liệu dùng chung cho cả module khai báo.
//!
//! KHÔNG chứa ảnh và KHÔNG chứa payload QR/MRZ thô — xem §12 của spec.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    QrCccd,
    MrzTd3,
    Manual,
}

impl Source {
    pub fn as_db(&self) -> &'static str {
        match self {
            Source::QrCccd => "qr_cccd",
            Source::MrzTd3 => "mrz_td3",
            Source::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Verified,
    NeedsReview,
}

impl Confidence {
    pub fn as_db(&self) -> &'static str {
        match self {
            Confidence::Verified => "verified",
            Confidence::NeedsReview => "needs_review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    FullName,
    Dob,
    Gender,
    Nationality,
    DocType,
    DocNo,
    PassportNo,
    PassportExpiry,
    VisaValidUntil,
    AddressDetail,
    Phone,
}

/// Danh tính trích từ ảnh giấy tờ.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub full_name: String,
    /// ISO yyyy-MM-dd. Chỉ writer đổi sang dd/MM/yyyy.
    pub dob: String,
    /// "M" | "F" — chuẩn hóa nội bộ, khác cả QR ("Nữ") lẫn MRZ ("F").
    pub gender: String,
    pub nationality_iso3: String,
    pub doc_type_code: Option<String>,
    /// "heuristic" | "human" — W06 đọc đúng cột này.
    pub doc_type_source: Option<String>,
    pub doc_type_name: Option<String>,
    pub doc_no: Option<String>,
    pub phone: Option<String>,
    pub residence_status: Option<String>,
    /// Chuỗi thô từ QR. KHÔNG parse thành tỉnh/phường — xem G7.
    pub address_detail: Option<String>,
    pub passport_no: Option<String>,
    /// Từ MRZ. KHÁC visa_valid_until — xem G10 và E10.
    pub passport_expiry: Option<String>,
    /// Nhập tay. Không tồn tại ở bất kỳ nguồn tự động nào.
    pub visa_valid_until: Option<String>,
    /// G5: MRZ dòng 1 không có checksum nào bảo vệ.
    pub name_confirmed_by_human: bool,
    /// Gỡ chặn E02 cho tên đơn có thật (mononym).
    pub single_token_name_ok: bool,
}

impl Identity {
    /// Phân loại NNN/VN. KHÔNG dùng `guests.guest_type` — cột đó nói dối (H2).
    pub fn is_vietnamese(&self) -> bool {
        self.nationality_iso3 == "VNM"
    }
}

/// Lượt lưu trú, đọc từ `bookings` / `rooms`. Chỉ đọc.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StayInfo {
    /// = bookings.id (TEXT uuid)
    pub stay_id: String,
    /// Đã cắt tiền tố "Phòng " — cổng nhận "5A".
    pub room_no: String,
    pub check_in: String,
    pub expected_out: String,
    pub actual_out: Option<String>,
    /// Timestamp gốc có offset, giữ lại cho W04.
    pub check_in_raw: String,
}

/// Một dòng sẵn sàng validate và ghi ra file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarationRow {
    pub link_id: String,
    pub identity: Identity,
    pub stay: StayInfo,
    pub stay_reason: String,
    pub stay_reason_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub link_id: String,
    pub field: Option<Field>,
    pub message: String,
}

impl Finding {
    pub fn blocking(code: &str, link_id: &str, field: Option<Field>, message: &str) -> Self {
        Finding {
            code: code.to_string(),
            severity: Severity::Blocking,
            link_id: link_id.to_string(),
            field,
            message: message.to_string(),
        }
    }

    pub fn warning(code: &str, link_id: &str, field: Option<Field>, message: &str) -> Self {
        Finding {
            code: code.to_string(),
            severity: Severity::Warning,
            link_id: link_id.to_string(),
            field,
            message: message.to_string(),
        }
    }
}

/// Kết quả trích xuất. `crop_for_review` chỉ sống trong RAM (§12.4).
pub struct ExtractResult {
    pub source: Source,
    pub confidence: Confidence,
    pub identity: Identity,
    pub review_hints: Vec<Field>,
    pub crop_for_review: Option<image::DynamicImage>,
}
