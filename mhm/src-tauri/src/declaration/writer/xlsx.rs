//! Khai báo khách Việt Nam — ghi vào template chính thức của cổng.
//!
//! Không bao giờ dựng workbook từ đầu: template mang 40 named range, 9 data
//! validation và ba sheet danh mục mà cổng đọc bằng công thức. Ta chỉ mở nó ra,
//! ghi thêm dòng từ row 5, rồi đọc lại kiểm tra 7 điều trước khi cho file tồn
//! tại.
//!
//! Row 1-3 là tiêu đề, row 4 là hàng `[EXAMPLE]` — cổng bỏ row 4 THEO VỊ TRÍ.
//! Ghi đè nó nghĩa là mất một khách trong khi cổng vẫn báo "thành công".

use crate::declaration::catalog::{Catalog, CatalogList};
use crate::declaration::model::DeclarationRow;
use crate::declaration::normalizer::iso_to_portal;
use std::io::Read;
use std::path::Path;

const SHEET: &str = "DS_KHACH_VIET_NAM_LUU_TRU";
/// Row 4 là hàng [EXAMPLE]; cổng bỏ nó THEO VỊ TRÍ. Data bắt đầu từ row 5.
const FIRST_DATA_ROW: u32 = 5;
/// Row 4 đã mang STT 1, nên dòng đầu của ta là 2.
const FIRST_STT: u32 = 2;

fn cell(col: &str, row: u32) -> String {
    format!("{col}{row}")
}

pub fn write_batch(
    rows: &[DeclarationRow],
    catalog: &Catalog,
    template: &Path,
    out: &Path,
) -> Result<(), String> {
    if rows.is_empty() {
        return Err("Lô rỗng, không có gì để ghi.".to_string());
    }

    let mut book = umya_spreadsheet::reader::xlsx::read(template)
        .map_err(|e| format!("Không đọc được template: {e:?}"))?;

    {
        let sheet = book
            .sheet_by_name_mut(SHEET)
            .map_err(|e| format!("Template thiếu sheet {SHEET}: {e:?}"))?;

        for (i, r) in rows.iter().enumerate() {
            let row_no = FIRST_DATA_ROW + i as u32;
            let stt = FIRST_STT + i as u32;
            let id = &r.identity;

            sheet
                .cell_mut(cell("A", row_no).as_str())
                .set_value_number(stt);
            sheet
                .cell_mut(cell("B", row_no).as_str())
                .set_value_string(&id.full_name);
            sheet.cell_mut(cell("C", row_no).as_str()).set_value_string(
                iso_to_portal(&id.dob).ok_or_else(|| format!("Ngày sinh hỏng: {}", id.dob))?,
            );
            sheet.cell_mut(cell("D", row_no).as_str()).set_value_string(
                catalog
                    .display_for(CatalogList::GioiTinh, &id.gender)
                    .ok_or_else(|| format!("Giới tính không có trong danh mục: {}", id.gender))?,
            );
            sheet.cell_mut(cell("E", row_no).as_str()).set_value_string(
                catalog
                    .display_for(CatalogList::QuocTich, &id.nationality_iso3)
                    .ok_or_else(|| format!("Quốc tịch không có: {}", id.nationality_iso3))?,
            );
            if let Some(code) = id.doc_type_code.as_deref() {
                sheet.cell_mut(cell("F", row_no).as_str()).set_value_string(
                    catalog
                        .display_for(CatalogList::LoaiGiayTo, code)
                        .ok_or_else(|| format!("Loại giấy tờ không có: {code}"))?,
                );
            }
            if let Some(v) = id.doc_type_name.as_deref() {
                sheet.cell_mut(cell("G", row_no).as_str()).set_value_string(v);
            }
            sheet
                .cell_mut(cell("H", row_no).as_str())
                .set_value_string(id.doc_no.as_deref().unwrap_or(""));
            if let Some(v) = id.phone.as_deref() {
                sheet.cell_mut(cell("I", row_no).as_str()).set_value_string(v);
            }
            if let Some(code) = id.residence_status.as_deref() {
                if let Some(d) = catalog.display_for(CatalogList::NoiCuTru, code) {
                    sheet.cell_mut(cell("J", row_no).as_str()).set_value_string(d);
                }
            }
            // K, L để trống có chủ ý: danh mục hành chính đã đổi mà giấy tờ
            // thì chưa, và fuzzy-match tên phường tạo ra khai báo sai im lặng.
            if let Some(v) = id.address_detail.as_deref() {
                sheet.cell_mut(cell("M", row_no).as_str()).set_value_string(v);
            }
            sheet.cell_mut(cell("N", row_no).as_str()).set_value_string(
                iso_to_portal(&r.stay.check_in)
                    .ok_or_else(|| format!("Ngày đến hỏng: {}", r.stay.check_in))?,
            );
            sheet.cell_mut(cell("O", row_no).as_str()).set_value_string(
                iso_to_portal(&r.stay.expected_out)
                    .ok_or_else(|| format!("Ngày đi hỏng: {}", r.stay.expected_out))?,
            );
            if !r.stay.room_no.is_empty() {
                sheet
                    .cell_mut(cell("P", row_no).as_str())
                    .set_value_string(&r.stay.room_no);
            }
            sheet.cell_mut(cell("Q", row_no).as_str()).set_value_string(
                catalog
                    .display_for(CatalogList::LyDoCuTru, &r.stay_reason)
                    .ok_or_else(|| format!("Lý do cư trú không có: {}", r.stay_reason))?,
            );
            if let Some(v) = r.stay_reason_note.as_deref() {
                sheet.cell_mut(cell("R", row_no).as_str()).set_value_string(v);
            }
        }
    }

    umya_spreadsheet::writer::xlsx::write(&book, out)
        .map_err(|e| format!("Không ghi được file: {e:?}"))?;

    // Gate: fail thì XÓA FILE và không cho ai ghi declaration_batch.
    if let Err(e) = verify_output(out, rows) {
        let _ = std::fs::remove_file(out);
        return Err(e);
    }

    Ok(())
}

/// Đếm `<definedName` trực tiếp trong zip.
///
/// `umya::defined_names()` trả 0 dù file có 40 — đã đo. Không dùng API đó.
pub fn count_defined_names(path: &Path) -> Result<usize, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Không mở được file: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("File không phải zip: {e}"))?;
    let mut xml = String::new();
    zip.by_name("xl/workbook.xml")
        .map_err(|e| format!("Thiếu xl/workbook.xml: {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("Không đọc được workbook.xml: {e}"))?;
    Ok(xml.matches("<definedName ").count())
}

fn count_col(sheet: &umya_spreadsheet::Worksheet, col: &str, upto: u32) -> usize {
    (2..=upto)
        .filter(|r| !sheet.value(cell(col, *r).as_str()).is_empty())
        .count()
}

/// Bảy assert của §9.2. Cổng không báo lỗi, nên đây là chốt chặn cuối cùng.
pub fn verify_output(out: &Path, rows: &[DeclarationRow]) -> Result<(), String> {
    if rows.is_empty() {
        return Err("Không có dòng nào để đối chiếu.".to_string());
    }

    let book = umya_spreadsheet::reader::xlsx::read(out)
        .map_err(|e| format!("Không đọc lại được file vừa ghi: {e:?}"))?;
    let sheet = book
        .sheet_by_name(SHEET)
        .map_err(|e| format!("File thiếu sheet {SHEET}: {e:?}"))?;

    // 1 — row 4 còn nguyên
    let a4 = sheet.value("A4");
    if a4 != "[EXAMPLE]" {
        return Err(format!(
            "Gate 1 fail: row 4 cột A là {a4:?}, phải là \"[EXAMPLE]\". \
             Cổng bỏ row 4 theo vị trí — ghi đè nó là mất khách."
        ));
    }

    // 2 — dòng đầu khớp
    let b5 = sheet.value("B5");
    if b5 != rows[0].identity.full_name {
        return Err(format!(
            "Gate 2 fail: row 5 cột B là {b5:?}, phải là {:?}",
            rows[0].identity.full_name
        ));
    }

    for (i, r) in rows.iter().enumerate() {
        let row_no = FIRST_DATA_ROW + i as u32;

        // 3 — ngày là text dd/MM/yyyy
        for col in ["C", "N", "O"] {
            let v = sheet.value(cell(col, row_no).as_str());
            let ok = v.len() == 10
                && v.as_bytes()[2] == b'/'
                && v.as_bytes()[5] == b'/'
                && v.chars().filter(|c| c.is_ascii_digit()).count() == 8;
            if !ok {
                return Err(format!(
                    "Gate 3 fail: {col}{row_no} = {v:?}, phải là text dd/MM/yyyy. \
                     Excel đã đổi nó thành serial number."
                ));
            }
        }

        // 4 — số giấy tờ giữ đủ ký tự, kể cả số 0 đầu
        let want = r.identity.doc_no.as_deref().unwrap_or("");
        let got = sheet.value(cell("H", row_no).as_str());
        if got != want {
            return Err(format!(
                "Gate 4 fail: H{row_no} = {got:?}, phải là {want:?} (mất số 0 đầu?)"
            ));
        }
    }

    // 5 — sheet danh mục còn nguyên
    let dm = book
        .sheet_by_name("DANH_MUC")
        .map_err(|_| "Gate 5 fail: mất sheet DANH_MUC".to_string())?;
    for (col, upto, want, name) in [
        ("A", 10u32, 9usize, "LOAI_GIAY_TO"),
        ("B", 4, 3, "NOI_CU_TRU"),
        ("C", 21, 20, "LY_DO_CU_TRU"),
        ("D", 3, 2, "GIOI_TINH"),
        ("E", 206, 205, "QUOC_TICH"),
    ] {
        let got = count_col(dm, col, upto);
        if got != want {
            return Err(format!("Gate 5 fail: {name} còn {got} dòng, phải là {want}"));
        }
    }

    // 6 — tỉnh và phường còn nguyên
    let tt = book
        .sheet_by_name("TINH_THANH")
        .map_err(|_| "Gate 6 fail: mất sheet TINH_THANH".to_string())?;
    let tt_n = count_col(tt, "C", 35);
    if tt_n != 34 {
        return Err(format!("Gate 6 fail: TINH_THANH còn {tt_n} dòng, phải là 34"));
    }
    let px = book
        .sheet_by_name("PHUONG_XA")
        .map_err(|_| "Gate 6 fail: mất sheet PHUONG_XA".to_string())?;
    let px_n = count_col(px, "D", 3324);
    if px_n != 3323 {
        return Err(format!(
            "Gate 6 fail: PHUONG_XA còn {px_n} dòng, phải là 3323"
        ));
    }

    // 7 — named range còn nguyên
    let names = count_defined_names(out)?;
    if names != 40 {
        return Err(format!("Gate 7 fail: còn {names} definedName, phải là 40"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::{Identity, StayInfo};

    fn vn_row(name: &str, doc: &str) -> DeclarationRow {
        DeclarationRow {
            link_id: "l".into(),
            identity: Identity {
                full_name: name.into(),
                dob: "1995-07-28".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                doc_type_code: Some("1".into()),
                doc_no: Some(doc.into()),
                phone: Some("0901234567".into()),
                address_detail: Some("KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận".into()),
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "b".into(),
                room_no: "5B".into(),
                check_in: "2026-07-26".into(),
                expected_out: "2026-07-29".into(),
                check_in_raw: "2026-07-26T09:00:00+07:00".into(),
                ..Default::default()
            },
            stay_reason: "2".into(),
            stay_reason_note: None,
        }
    }

    fn template() -> std::path::PathBuf {
        crate::declaration::find_kbtt_resource("tblt_vn_import.xlsx").unwrap()
    }

    fn write_to_temp(rows: &[DeclarationRow]) -> std::path::PathBuf {
        let cat = Catalog::load().unwrap();
        let out = std::env::temp_dir().join(format!("kbtt_test_{}.xlsx", uuid::Uuid::new_v4()));
        write_batch(rows, &cat, &template(), &out).expect("ghi được");
        out
    }

    /// F2 — TEST ÂM QUAN TRỌNG NHẤT. Server bỏ row 4 theo vị trí. Ghi đè nó
    /// nghĩa là mất một khách mà cổng vẫn báo "thành công".
    #[test]
    fn never_writes_over_the_example_row() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let sheet = book.sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(sheet.value("A4"), "[EXAMPLE]");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn data_starts_at_row_five_with_stt_two() {
        let out = write_to_temp(&[
            vn_row("Phan Thị Mỹ Hà", "058195006173"),
            vn_row("Lê Đình Lực", "079174011721"),
        ]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.value("A5"), "2");
        assert_eq!(s.value("B5"), "Phan Thị Mỹ Hà");
        assert_eq!(s.value("A6"), "3");
        assert_eq!(s.value("B6"), "Lê Đình Lực");
        let _ = std::fs::remove_file(out);
    }

    /// F7 — Excel sẽ ăn mất số 0 đầu nếu cell là số.
    #[test]
    fn document_number_keeps_its_leading_zero() {
        let out = write_to_temp(&[vn_row("Lê Đình Lực", "079174011721")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.value("H5"), "079174011721");
        let _ = std::fs::remove_file(out);
    }

    /// F6 — ngày phải là text `dd/MM/yyyy`, không phải serial number.
    #[test]
    fn dates_are_text_in_portal_format() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.value("C5"), "28/07/1995");
        assert_eq!(s.value("N5"), "26/07/2026");
        assert_eq!(s.value("O5"), "29/07/2026");
        let _ = std::fs::remove_file(out);
    }

    /// F5 — ghi nguyên chuỗi `mã - nhãn`, không phải chỉ mã.
    #[test]
    fn enums_are_written_as_full_display_strings() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.value("D5"), "F - Nữ");
        assert_eq!(s.value("E5"), "VNM - Viet Nam");
        assert_eq!(s.value("F5"), "1 - Thẻ CCCD");
        assert_eq!(s.value("Q5"), "2 - Công tác");
        let _ = std::fs::remove_file(out);
    }

    /// G7 — v1 để trống tỉnh/phường, địa chỉ đi nguyên chuỗi vào cột M.
    #[test]
    fn province_and_ward_stay_empty_address_goes_raw() {
        let out = write_to_temp(&[vn_row("Phan Thị Mỹ Hà", "058195006173")]);
        let book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        let s = book.sheet_by_name("DS_KHACH_VIET_NAM_LUU_TRU").unwrap();
        assert_eq!(s.value("K5"), "");
        assert_eq!(s.value("L5"), "");
        assert!(s.value("M5").contains("Ninh Thuận"));
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn gate_passes_on_a_well_formed_file() {
        let rows = vec![vn_row("Phan Thị Mỹ Hà", "058195006173")];
        let out = write_to_temp(&rows);
        assert!(verify_output(&out, &rows).is_ok());
        let _ = std::fs::remove_file(out);
    }

    /// umya `defined_names()` trả 0 dù file có 40 — nên gate phải đọc zip.
    #[test]
    fn gate_counts_defined_names_from_the_zip_not_the_api() {
        let rows = vec![vn_row("Phan Thị Mỹ Hà", "058195006173")];
        let out = write_to_temp(&rows);
        assert_eq!(count_defined_names(&out).unwrap(), 40);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn gate_rejects_a_file_whose_example_row_was_clobbered() {
        let rows = vec![vn_row("Phan Thị Mỹ Hà", "058195006173")];
        let out = write_to_temp(&rows);

        let mut book = umya_spreadsheet::reader::xlsx::read(&out).unwrap();
        book.sheet_by_name_mut("DS_KHACH_VIET_NAM_LUU_TRU")
            .unwrap()
            .cell_mut("A4")
            .set_value_string("phá hoại");
        umya_spreadsheet::writer::xlsx::write(&book, &out).unwrap();

        let err = verify_output(&out, &rows).unwrap_err();
        assert!(err.contains("row 4"), "báo lỗi phải nói rõ row 4: {err}");
        let _ = std::fs::remove_file(out);
    }
}
