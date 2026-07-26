//! Khai báo tạm trú — xuất file khai báo cho cổng Bộ Công an.
//!
//! Ranh giới kiến trúc: `catalog`, `extractor`, `normalizer`, `validator` và
//! `writer` KHÔNG import `tauri` và KHÔNG import `sqlx`. Chúng nhận struct vào,
//! trả struct ra. Chỉ `repo` chạm DB.
//!
//! Module này đọc `bookings` / `booking_guests` / `rooms` / `guests` ở chế độ
//! CHỈ ĐỌC. PMS đang vận hành thật — không migrate nó vì một tính năng phụ.

pub mod catalog;
pub mod extractor;
pub mod model;
pub mod normalizer;
pub mod repo;
pub mod validator;
pub mod writer;

use std::path::PathBuf;

/// Tìm file resource của module khai báo.
///
/// `bin/kbtt_probe` và unit test chạy ngoài Tauri nên không có app handle để
/// gọi `BaseDirectory::Resource`. Thử lần lượt như `ocr::find_models_dir`.
pub fn find_kbtt_resource(name: &str) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(root) = crate::app_identity::runtime_root_opt() {
        candidates.push(root.join("resources").join(name));
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    candidates.push(cwd.join("resources").join(name));
    candidates.push(cwd.join("..").join("resources").join(name));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(name),
    );

    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .ok_or_else(|| {
            format!(
                "Không tìm thấy resource {name}. Đã thử: {}",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}
