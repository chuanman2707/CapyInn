//! Trích danh tính từ ảnh giấy tờ.
//!
//! Ranh giới: KHÔNG import `tauri`, KHÔNG import `sqlx`. Nhận ảnh vào, trả
//! struct ra. Không log payload và không log đường dẫn ảnh (§12.3).

pub mod mrz;
pub mod ocr_rs_mrz;
pub mod qr_cccd;

use crate::declaration::model::ExtractResult;

pub trait IdentityExtractor {
    fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult>;
}
