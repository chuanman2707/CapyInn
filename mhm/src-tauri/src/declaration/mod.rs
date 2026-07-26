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

/// Đường dẫn resource nằm cạnh file thực thi, theo cách Tauri đóng gói.
///
/// macOS: `CapyInn.app/Contents/MacOS/capyinn` → `../Resources/resources/`.
/// Windows và Linux: `resources/` nằm ngay cạnh file exe.
///
/// Phải có nhánh này thì bản đóng gói mới chạy được: khi mở app từ Finder,
/// thư mục làm việc là `/`, nên mọi ứng viên dựa trên `cwd` đều trượt.
fn bundled_resource_candidates(name: &str) -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(exe_dir) = exe.parent() else {
        return Vec::new();
    };

    let mut out = vec![exe_dir.join("resources").join(name)];
    if let Some(contents) = exe_dir.parent() {
        out.push(contents.join("Resources").join("resources").join(name));
    }
    out
}

/// Tìm file resource của module khai báo.
///
/// `bin/kbtt_probe` và unit test chạy ngoài Tauri nên không có app handle để
/// gọi `BaseDirectory::Resource`. Thử lần lượt như `ocr::find_models_dir`.
pub fn find_kbtt_resource(name: &str) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = bundled_resource_candidates(name);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Bản đóng gói đặt resource cạnh file thực thi, không phải cạnh thư mục
    /// làm việc — mở app từ Finder thì cwd là `/`. Thiếu nhánh này thì test
    /// vẫn xanh (test chạy trong CARGO_MANIFEST_DIR) còn bản build thì chết.
    #[test]
    fn looks_beside_the_executable_before_anything_else() {
        let exe = std::env::current_exe().expect("có đường dẫn file thực thi");
        let exe_dir = exe.parent().expect("có thư mục chứa");

        let candidates = bundled_resource_candidates("kbtt_catalog.json");

        assert!(
            candidates.contains(&exe_dir.join("resources").join("kbtt_catalog.json")),
            "phải thử thư mục resources cạnh exe (Windows/Linux)"
        );
        assert!(
            candidates.iter().any(|p| p.ends_with(
                std::path::Path::new("Resources")
                    .join("resources")
                    .join("kbtt_catalog.json")
            )),
            "phải thử Contents/Resources/resources (macOS .app)"
        );
    }

    #[test]
    fn resolves_the_catalog_in_this_checkout() {
        let path = find_kbtt_resource("kbtt_catalog.json").expect("phải tìm thấy catalog");
        assert!(path.exists());
    }
}
