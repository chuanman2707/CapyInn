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

/// Một dòng chờ khai, **phẳng**, đúng tên trường mà `DeclarationRow` bên
/// TypeScript đọc (`mhm/src/types/index.ts`).
///
/// Tồn tại vì `model::DeclarationRow` lồng danh tính trong `identity` và lượt
/// lưu trú trong `stay`, còn giao diện lại đọc `row.full_name`, `row.room_no`.
/// Serde không bắc cầu giúp hai hình dạng đó: mọi ô hiển thị ra `undefined`,
/// khách Việt bị `isForeign(undefined)` xếp nhầm sang ngăn XML, và nút xuất
/// file tắt vì ngăn XLSX rỗng. Test hai bên đều xanh vì mỗi bên tự dựng dữ
/// liệu mẫu theo hình dạng của mình, không bên nào chạm chỗ giáp ranh.
///
/// Tên trường ở đây là HỢP ĐỒNG với giao diện. Đổi tên thì phải đổi cả
/// `DeclarationRow` bên TS và `row_dto_matches_the_typescript_contract`.
#[derive(Debug, Serialize)]
pub struct DeclarationRowDto {
    pub link_id: String,
    pub identity_id: String,
    pub full_name: String,
    pub dob: String,
    pub gender: String,
    pub nationality_iso3: String,
    pub doc_type_code: Option<String>,
    pub doc_type_name: Option<String>,
    pub doc_no: Option<String>,
    pub phone: Option<String>,
    pub residence_status: Option<String>,
    pub address_detail: Option<String>,
    pub passport_no: Option<String>,
    pub passport_expiry: Option<String>,
    pub visa_valid_until: Option<String>,
    /// `None` khi khai báo chưa gắn vào lượt lưu trú nào (chưa xác định phòng).
    pub room_no: Option<String>,
    pub check_in_date: String,
    pub expected_check_out: String,
    pub stay_reason: String,
    pub stay_reason_note: Option<String>,
    pub name_confirmed_by_human: bool,
    pub single_token_name_ok: bool,
}

impl From<&DeclarationRow> for DeclarationRowDto {
    fn from(row: &DeclarationRow) -> Self {
        let id = &row.identity;
        let stay = &row.stay;
        DeclarationRowDto {
            link_id: row.link_id.clone(),
            identity_id: id.id.clone(),
            full_name: id.full_name.clone(),
            dob: id.dob.clone(),
            gender: id.gender.clone(),
            nationality_iso3: id.nationality_iso3.clone(),
            doc_type_code: id.doc_type_code.clone(),
            doc_type_name: id.doc_type_name.clone(),
            doc_no: id.doc_no.clone(),
            phone: id.phone.clone(),
            residence_status: id.residence_status.clone(),
            address_detail: id.address_detail.clone(),
            passport_no: id.passport_no.clone(),
            passport_expiry: id.passport_expiry.clone(),
            visa_valid_until: id.visa_valid_until.clone(),
            room_no: if stay.room_no.trim().is_empty() {
                None
            } else {
                Some(stay.room_no.clone())
            },
            check_in_date: stay.check_in.clone(),
            expected_check_out: stay.expected_out.clone(),
            stay_reason: row.stay_reason.clone(),
            stay_reason_note: row.stay_reason_note.clone(),
            name_confirmed_by_human: id.name_confirmed_by_human,
            single_token_name_ok: id.single_token_name_ok,
        }
    }
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
    repo::save_identity_ensuring_link(&state.db, &identity, &source, &confidence).await
}

/// `stay_id = None` — khai báo chưa gắn phòng. Cổng không bắt buộc cột phòng;
/// validator vẫn nhắc bằng W01 để không ai quên.
#[tauri::command]
pub async fn kbtt_link(
    state: State<'_, AppState>,
    identity_id: String,
    stay_id: Option<String>,
    stay_reason: String,
    note: Option<String>,
) -> Result<String, String> {
    repo::insert_link(
        &state.db,
        &identity_id,
        stay_id.as_deref().filter(|s| !s.trim().is_empty()),
        &stay_reason,
        note.as_deref(),
    )
    .await
}

/// Danh tính đã lưu nhưng chưa ghép — nguồn sự thật là DB, không phải state của
/// React, nên đổi tab không làm mất.
#[tauri::command]
pub async fn kbtt_unlinked_identities(
    state: State<'_, AppState>,
) -> Result<Vec<Identity>, String> {
    repo::list_unlinked_identities(&state.db).await
}

#[tauri::command]
pub async fn kbtt_discard_identity(
    state: State<'_, AppState>,
    identity_id: String,
) -> Result<(), String> {
    repo::delete_unlinked_identity(&state.db, &identity_id).await
}

/// Gỡ một khai báo đã ghép — ghép nhầm phòng, hoặc ghép trùng. Từ chối nếu nó
/// đã nằm trong lô đã đối soát.
#[tauri::command]
pub async fn kbtt_unlink(state: State<'_, AppState>, link_id: String) -> Result<(), String> {
    repo::delete_link(&state.db, &link_id).await
}

#[tauri::command]
pub async fn kbtt_pending_rows(
    state: State<'_, AppState>,
) -> Result<Vec<DeclarationRowDto>, String> {
    let link_ids = repo::pending_link_ids(&state.db).await?;
    let rows = repo::load_rows_by_link_ids(&state.db, &link_ids).await?;
    Ok(rows.iter().map(DeclarationRowDto::from).collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::StayInfo;

    /// Trích tên trường của một `interface` trong file .ts.
    ///
    /// Đọc thẳng file nguồn thay vì chép tay danh sách: chép tay thì hai bên lại
    /// trôi lệch nhau lần nữa, đúng thứ đã sinh ra lỗi này.
    fn typescript_interface_fields(source: &str, name: &str) -> Vec<String> {
        let head = format!("export interface {name} {{");
        let start = source
            .find(&head)
            .unwrap_or_else(|| panic!("không tìm thấy `{head}` trong types/index.ts"))
            + head.len();
        let body = &source[start..];
        let end = body.find("\n}").expect("interface phải có dấu đóng");

        body[..end]
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
                    return None;
                }
                let field = line.split(':').next()?.trim().trim_end_matches('?');
                if field.is_empty() {
                    None
                } else {
                    Some(field.to_string())
                }
            })
            .collect()
    }

    fn types_index_ts() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src")
            .join("types")
            .join("index.ts");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("đọc {}: {e}", path.display()))
    }

    fn sample_row() -> DeclarationRow {
        DeclarationRow {
            link_id: "l1".into(),
            identity: Identity {
                id: "i1".into(),
                full_name: "Phạm Thị Minh Hiền".into(),
                dob: "1988-12-16".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                doc_no: Some("056188011500".into()),
                ..Identity::default()
            },
            stay: StayInfo {
                stay_id: "s1".into(),
                room_no: "3B".into(),
                check_in: "2026-07-27".into(),
                expected_out: "2026-07-28".into(),
                ..StayInfo::default()
            },
            stay_reason: "1".into(),
            stay_reason_note: None,
        }
    }

    fn json_keys<T: Serialize>(value: &T) -> Vec<String> {
        let json = serde_json::to_value(value).expect("serialize được");
        let mut keys: Vec<String> = json
            .as_object()
            .expect("phải là object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// Giao diện đọc `row.full_name`; nếu backend gửi lên hình dạng khác thì mọi
    /// ô hiện trống mà không có lỗi nào nổ ra. Test này là chỗ duy nhất phát
    /// hiện được điều đó.
    #[test]
    fn row_dto_matches_the_typescript_contract() {
        let mut expected = typescript_interface_fields(&types_index_ts(), "DeclarationRow");
        expected.sort();

        let actual = json_keys(&DeclarationRowDto::from(&sample_row()));

        assert_eq!(
            actual, expected,
            "bộ khóa JSON phải trùng khít interface DeclarationRow bên TypeScript"
        );
    }

    /// Ghi lại chính lỗi đã xảy ra: gửi thẳng `DeclarationRow` thì giao diện
    /// không thấy `full_name` ở đâu cả.
    #[test]
    fn the_nested_row_is_not_what_the_ui_can_read() {
        let keys = json_keys(&sample_row());

        assert!(
            !keys.contains(&"full_name".to_string()),
            "DeclarationRow lồng danh tính trong `identity` — đừng gửi thẳng ra giao diện"
        );
        assert!(keys.contains(&"identity".to_string()));
    }

    /// Khách Việt phải ra đúng `VNM` để `isForeign` xếp vào ngăn XLSX.
    #[test]
    fn a_vietnamese_guest_keeps_its_nationality_through_the_dto() {
        let dto = DeclarationRowDto::from(&sample_row());

        assert_eq!(dto.nationality_iso3, "VNM");
        assert_eq!(dto.room_no.as_deref(), Some("3B"));
        assert_eq!(dto.full_name, "Phạm Thị Minh Hiền");
    }

    /// Khai báo chưa gắn phòng: `room_no` phải là `null`, không phải chuỗi rỗng
    /// — giao diện hiện "—" dựa vào đúng chỗ đó.
    #[test]
    fn a_declaration_without_a_room_reports_no_room() {
        let mut row = sample_row();
        row.stay = StayInfo::default();

        let dto = DeclarationRowDto::from(&row);

        assert_eq!(dto.room_no, None);
    }
}
