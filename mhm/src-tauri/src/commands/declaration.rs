//! Lớp Tauri cho module khai báo tạm trú.
//!
//! Mỏng có chủ ý: không chứa logic nghiệp vụ. Nó dựng dữ liệu từ `repo` rồi gọi
//! thẳng `validator` và `writer`, để toàn bộ phần đáng test nằm ở các module
//! thuần không phụ thuộc Tauri.
//!
//! §12.3 — KHÔNG log payload QR/MRZ và KHÔNG log đường dẫn ảnh.

use serde::Serialize;
use tauri::State;

use crate::declaration::catalog::Catalog;
use crate::declaration::model::{Confidence, DeclarationRow, Finding, Identity, Severity, StayInfo};
use crate::declaration::{repo, validator, writer};
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct ExtractedDto {
    pub source: String,
    pub confidence: String,
    pub identity: Identity,
    pub review_hints: Vec<crate::declaration::model::Field>,
    /// `data:image/png;base64,...` — chỉ đi qua IPC, KHÔNG ghi đĩa (§12.4).
    pub crop_data_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchDto {
    pub batch_id: String,
    pub file_path: String,
    pub row_count: usize,
    pub kind: String,
}

fn current_year() -> u32 {
    chrono::Local::now()
        .format("%Y")
        .to_string()
        .parse()
        .unwrap_or(2026)
}

fn today_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[tauri::command]
pub async fn kbtt_extract_from_image(path: String) -> Result<ExtractedDto, String> {
    use crate::declaration::extractor::{
        mrz::MrzExtractor, ocr_rs_mrz::OcrRsMrz, qr_cccd::QrCccdExtractor, IdentityExtractor,
    };

    // Đường dẫn ảnh là dữ liệu cá nhân — không đưa vào thông báo lỗi.
    let img = image::open(&path).map_err(|_| "Không mở được ảnh.".to_string())?;

    // QR trước: dữ liệu số, không qua OCR nên không sai.
    let result = QrCccdExtractor.try_extract(&img).or_else(|| {
        OcrRsMrz::new()
            .ok()
            .and_then(|ocr| MrzExtractor::new(ocr, current_year()).try_extract(&img))
    });

    let res = result.ok_or_else(|| {
        "Không đọc được QR hay MRZ trong ảnh. Dùng form nhập tay.".to_string()
    })?;

    let crop_data_url = res.crop_for_review.as_ref().and_then(|c| {
        use base64::Engine;
        let mut buf = std::io::Cursor::new(Vec::new());
        c.write_to(&mut buf, image::ImageFormat::Png).ok()?;
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
        ))
    });

    Ok(ExtractedDto {
        source: res.source.as_db().to_string(),
        confidence: res.confidence.as_db().to_string(),
        identity: res.identity,
        review_hints: res.review_hints,
        crop_data_url,
    })
}

#[tauri::command]
pub async fn kbtt_list_stays(state: State<'_, AppState>) -> Result<Vec<StayInfo>, String> {
    repo::load_stays_for_declaration(&state.db).await
}

#[tauri::command]
pub async fn kbtt_save_identity(
    state: State<'_, AppState>,
    identity: Identity,
    source: String,
    confidence: String,
) -> Result<String, String> {
    repo::insert_identity(&state.db, &identity, &source, &confidence).await
}

#[tauri::command]
pub async fn kbtt_link(
    state: State<'_, AppState>,
    identity_id: String,
    stay_id: String,
    stay_reason: String,
    note: Option<String>,
) -> Result<String, String> {
    repo::insert_link(
        &state.db,
        &identity_id,
        &stay_id,
        &stay_reason,
        note.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn kbtt_pending_rows(state: State<'_, AppState>) -> Result<Vec<DeclarationRow>, String> {
    let link_ids = repo::pending_link_ids(&state.db).await?;
    repo::load_rows_by_link_ids(&state.db, &link_ids).await
}

/// `W05` sinh ở đây chứ không ở validator: `DeclarationRow` không mang
/// `extract_confidence`, cột đó nằm ở `declaration_identity`.
#[tauri::command]
pub async fn kbtt_validate(
    state: State<'_, AppState>,
    link_ids: Vec<String>,
) -> Result<Vec<Finding>, String> {
    let catalog = Catalog::load()?;
    let rows = repo::load_rows_by_link_ids(&state.db, &link_ids).await?;
    let mut findings = validator::validate(&rows, &catalog, &today_iso());

    let confidence = repo::confidence_by_link(&state.db, &link_ids).await?;
    for row in &rows {
        if confidence.get(&row.link_id).map(String::as_str)
            == Some(Confidence::NeedsReview.as_db())
        {
            findings.push(Finding::warning(
                "W05",
                &row.link_id,
                None,
                "Danh tính được đánh dấu cần xem lại lúc trích xuất.",
            ));
        }
    }

    Ok(findings)
}

#[tauri::command]
pub async fn kbtt_export(
    state: State<'_, AppState>,
    kind: String,
    link_ids: Vec<String>,
) -> Result<BatchDto, String> {
    let pool = &state.db;
    let catalog = Catalog::load()?;
    let rows = repo::load_rows_by_link_ids(pool, &link_ids).await?;

    if rows.is_empty() {
        return Err("Chưa chọn hồ sơ nào để xuất.".to_string());
    }

    // Cổng báo "thành công" khi import 0 record, nên chốt chặn nằm ở đây.
    let findings = validator::validate(&rows, &catalog, &today_iso());
    if validator::has_blocking(&findings) {
        let mut codes: Vec<&str> = findings
            .iter()
            .filter(|f| f.severity == Severity::Blocking)
            .map(|f| f.code.as_str())
            .collect();
        codes.sort_unstable();
        codes.dedup();
        return Err(format!("Còn lỗi chặn, không xuất được: {}", codes.join(", ")));
    }

    let dir = repo::export_dir(pool).await?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Không tạo được thư mục xuất: {e}"))?;
    let cslt = repo::cslt_name(pool).await?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M").to_string();

    let file_path = match kind.as_str() {
        "NNN" => {
            let lead = repo::xml_lead_example(pool).await?;
            let xml = writer::xml::render(&rows, lead)?;
            let p = dir.join(format!("KBTT_{cslt}_{stamp}.xml"));
            std::fs::write(&p, xml).map_err(|e| format!("Không ghi được XML: {e}"))?;
            p
        }
        "VN" => {
            let template = crate::declaration::find_kbtt_resource("tblt_vn_import.xlsx")?;
            let p = dir.join(format!("TBLT_{cslt}_{stamp}.xlsx"));
            // write_batch tự chạy 7 assert và tự xóa file nếu bất kỳ assert nào đỏ
            writer::xlsx::write_batch(&rows, &catalog, &template, &p)?;
            p
        }
        other => return Err(format!("Loại lô không hợp lệ: {other}")),
    };

    let batch_id = repo::insert_batch(
        pool,
        &kind,
        &file_path.to_string_lossy(),
        rows.len() as i64,
    )
    .await?;
    repo::insert_entries(pool, &batch_id, &link_ids).await?;

    Ok(BatchDto {
        batch_id,
        file_path: file_path.to_string_lossy().to_string(),
        row_count: rows.len(),
        kind,
    })
}

/// Vòng đối chiếu. Lý do tồn tại: cổng nói "import thành công" khi import 0
/// record, và chuyện đó đã xảy ra thật.
#[tauri::command]
pub async fn kbtt_reconcile(
    state: State<'_, AppState>,
    batch_id: String,
    seen_count: i64,
) -> Result<String, String> {
    let pool = &state.db;
    let expected = repo::batch_row_count(pool, &batch_id).await?;

    if seen_count == expected {
        repo::set_batch_verified(pool, &batch_id, seen_count).await?;
        Ok("verified".to_string())
    } else {
        // Không cần code hoàn tác: lô hỏng không còn entry `verified` nào nên
        // khách tự động giữ nguyên trạng thái chưa khai.
        repo::set_batch_failed(pool, &batch_id, seen_count).await?;
        Ok("failed".to_string())
    }
}

#[tauri::command]
pub async fn kbtt_undeclared_count(state: State<'_, AppState>) -> Result<i64, String> {
    repo::count_undeclared_within_48h(&state.db).await
}

#[tauri::command]
pub async fn kbtt_list_batches(
    state: State<'_, AppState>,
) -> Result<Vec<repo::BatchSummary>, String> {
    repo::list_batches(&state.db).await
}

#[tauri::command]
pub async fn kbtt_open_export_dir(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    batch_id: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let path = repo::batch_file_path(&state.db, &batch_id).await?;
    let dir = std::path::Path::new(&path)
        .parent()
        .ok_or_else(|| "Không xác định được thư mục chứa file.".to_string())?;

    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("Không mở được thư mục: {e}"))
}
