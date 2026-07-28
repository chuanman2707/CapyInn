//! Kiểm tra `DeclarationRow` trước khi xuất file.
//!
//! Cổng Bộ Công an báo "thành công" cả khi import 0 bản ghi (F3), nên mọi phép
//! kiểm phải nằm ở đây — không có vòng phản hồi nào từ phía cổng.
//!
//! Hàm thuần: vào `&[DeclarationRow]` + `&Catalog` + `today`, ra `Vec<Finding>`.
//! Không chạm DB, không ghi file, không import `tauri`/`sqlx`.

use crate::declaration::catalog::{Catalog, CatalogList};
use crate::declaration::model::{DeclarationRow, Field, Finding, Severity};
use std::collections::HashSet;

pub fn has_blocking(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Blocking)
}

fn days_between(from: &str, to: &str) -> Option<i64> {
    use chrono::NaiveDate;
    let a = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let b = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    Some((b - a).num_days())
}

fn doc_number_is_clean(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_mrz_shaped(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c == ' ' || c.is_ascii_digit())
}

/// Đúng DẠNG ISO3: ba chữ cái HOA ASCII. Cố ý KHÔNG tra danh mục — xem W07.
fn is_iso3_shaped(code: &str) -> bool {
    code.chars().count() == 3 && code.chars().all(|c| c.is_ascii_uppercase())
}

pub fn validate(rows: &[DeclarationRow], catalog: &Catalog, today: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for row in rows {
        let id = &row.identity;
        let link = row.link_id.as_str();
        let is_vn = id.is_vietnamese();

        // FINDING C1: chưa chọn phòng thì `check_in`/`expected_out` LUÔN rỗng —
        // hai ngày đó lấy từ lượt lưu trú, không phải từ danh tính. Đổ chúng
        // vào E01 khiến thẻ chỉ đường sai (mở ManualForm, form đó không có ô
        // ngày nào) trong khi nguyên nhân thật là chưa chọn phòng. E16 dưới
        // đây nói đúng nguyên nhân đó; ở đây chỉ còn xét ngày khi ĐÃ có phòng.
        let missing_room = row.stay.room_no.trim().is_empty();

        // E01 — field bắt buộc theo từng định dạng
        let mut missing: Vec<&str> = Vec::new();
        if id.full_name.trim().is_empty() {
            missing.push("họ tên");
        }
        if id.dob.trim().is_empty() {
            missing.push("ngày sinh");
        }
        if id.gender.trim().is_empty() {
            missing.push("giới tính");
        }
        if id.nationality_iso3.trim().is_empty() {
            missing.push("quốc tịch");
        }
        if !missing_room {
            if row.stay.check_in.trim().is_empty() {
                missing.push("ngày đến");
            }
            if row.stay.expected_out.trim().is_empty() {
                missing.push("ngày đi dự kiến");
            }
        }
        if row.stay_reason.trim().is_empty() {
            missing.push("lý do lưu trú");
        }
        if is_vn {
            if id.doc_type_code.as_deref().unwrap_or("").is_empty() {
                missing.push("loại giấy tờ");
            }
            if id.doc_no.as_deref().unwrap_or("").is_empty() {
                missing.push("số giấy tờ");
            }
        } else if id.passport_no.as_deref().unwrap_or("").is_empty() {
            missing.push("số hộ chiếu");
        }
        if !missing.is_empty() {
            out.push(Finding::blocking(
                "E01",
                link,
                None,
                &format!("Thiếu field bắt buộc: {}", missing.join(", ")),
            ));
        }

        // E16 — FINDING C1: một khai báo được PHÉP tồn tại chưa gắn phòng
        // (v21 — khách vừa tới, booking chưa tạo), nhưng KHÔNG xuất được nếu
        // vậy, vì ngày đến/ngày đi mà file nộp cần lấy từ lượt lưu trú đó.
        // Đây là finding CHẶN riêng, tách khỏi E01, để thẻ nói đúng nguyên
        // nhân và trỏ đúng vào ô Phòng — không mở ManualForm, vì form đó
        // không có gì để sửa cho lỗi này.
        if missing_room {
            out.push(Finding::blocking(
                "E16",
                link,
                None,
                "Chưa chọn phòng — ngày đến, ngày đi dự kiến lấy từ lượt lưu trú nên không xuất được.",
            ));
        }

        // E02 — tên một token
        if id.full_name.split_whitespace().count() == 1 && !id.single_token_name_ok {
            out.push(Finding::blocking(
                "E02",
                link,
                Some(Field::FullName),
                "Tên chỉ có một chữ. Nếu giấy tờ đúng như vậy, tick xác nhận để gỡ chặn.",
            ));
        }

        if !is_vn {
            // E03 — NNN phải đúng dạng MRZ: HOA, không dấu
            if !is_mrz_shaped(&id.full_name) {
                out.push(Finding::blocking(
                    "E03",
                    link,
                    Some(Field::FullName),
                    "Tên khách nước ngoài phải viết HOA không dấu, đúng dạng MRZ.",
                ));
            }
            // E04 — G5
            if !id.name_confirmed_by_human {
                out.push(Finding::blocking(
                    "E04",
                    link,
                    Some(Field::FullName),
                    "Chưa ai xác nhận tên. Dòng 1 của MRZ không có checksum bảo vệ.",
                ));
            }
            // E08 / E09 / E10
            match id.visa_valid_until.as_deref() {
                None | Some("") => out.push(Finding::blocking(
                    "E08",
                    link,
                    Some(Field::VisaValidUntil),
                    "Thiếu thời hạn tạm trú. Giá trị này không có trong MRZ, phải nhập tay.",
                )),
                Some(visa) => {
                    if days_between(visa, &row.stay.expected_out).unwrap_or(0) > 0 {
                        out.push(Finding::blocking(
                            "E09",
                            link,
                            Some(Field::VisaValidUntil),
                            "Thời hạn tạm trú kết thúc trước ngày đi dự kiến — khách sẽ ở quá hạn.",
                        ));
                    }
                    if Some(visa) == id.passport_expiry.as_deref() {
                        out.push(Finding::blocking(
                            "E10",
                            link,
                            Some(Field::VisaValidUntil),
                            "Thời hạn tạm trú trùng ngày hết hạn hộ chiếu — nghi lấy nhầm nguồn.",
                        ));
                    }
                }
            }
        }

        // E05 — chỉ kiểm DẠNG, cố ý không tra danh mục.
        //
        // H10: named range QUOC_TICH của template thiếu 7 nước (Brazil, Brunei,
        // Bulgaria...). Chặn theo danh mục sẽ khóa oan khách hợp lệ và người
        // vận hành không còn đường nào đi tiếp. Thiếu danh mục là W07.
        if !is_iso3_shaped(&id.nationality_iso3) {
            out.push(Finding::blocking(
                "E05",
                link,
                Some(Field::Nationality),
                &format!(
                    "Mã quốc tịch {:?} sai dạng — phải là 3 chữ cái HOA theo ISO 3166-1 alpha-3.",
                    id.nationality_iso3
                ),
            ));
        }

        // E06
        if !row.stay_reason.is_empty()
            && !catalog.has_code(CatalogList::LyDoCuTru, &row.stay_reason)
        {
            out.push(Finding::blocking(
                "E06",
                link,
                None,
                &format!("Lý do cư trú {} không có trong danh mục.", row.stay_reason),
            ));
        }
        if let Some(code) = id.doc_type_code.as_deref() {
            if !code.is_empty() && !catalog.has_code(CatalogList::LoaiGiayTo, code) {
                out.push(Finding::blocking(
                    "E06",
                    link,
                    Some(Field::DocType),
                    &format!("Loại giấy tờ {code} không có trong danh mục."),
                ));
            }
        }

        // E07
        if days_between(&row.stay.check_in, &row.stay.expected_out).unwrap_or(0) < 0 {
            out.push(Finding::blocking(
                "E07",
                link,
                None,
                "Ngày đi dự kiến sớm hơn ngày đến.",
            ));
        }

        // E11 / E12
        if id.doc_type_code.as_deref() == Some("9")
            && id.doc_type_name.as_deref().unwrap_or("").is_empty()
        {
            out.push(Finding::blocking(
                "E11",
                link,
                Some(Field::DocType),
                "Loại giấy tờ là 'Giấy Tờ Khác' nhưng chưa ghi tên giấy tờ.",
            ));
        }
        if row.stay_reason == "20" && row.stay_reason_note.as_deref().unwrap_or("").is_empty() {
            out.push(Finding::blocking(
                "E12",
                link,
                None,
                "Lý do là 'Mục đích khác' nhưng chưa nhập lý do cụ thể.",
            ));
        }

        // E13
        let number = if is_vn {
            id.doc_no.as_deref().unwrap_or("")
        } else {
            id.passport_no.as_deref().unwrap_or("")
        };
        if !doc_number_is_clean(number) {
            out.push(Finding::blocking(
                "E13",
                link,
                Some(if is_vn {
                    Field::DocNo
                } else {
                    Field::PassportNo
                }),
                "Số giấy tờ rỗng hoặc chứa ký tự lạ.",
            ));
        }

        // E14 — trùng trong cùng lô
        let key = (number.to_string(), row.stay.check_in.clone());
        if !number.is_empty() && !seen.insert(key) {
            out.push(Finding::blocking(
                "E14",
                link,
                None,
                "Trùng hồ sơ trong cùng lô: cùng số giấy tờ và cùng ngày đến.",
            ));
        }

        // Không có E15: v1 để trống cột tỉnh/phường (F8, H6, G7) nên không có
        // dữ liệu để kiểm. TODO v2 nếu sau này cho chọn tay.

        // Cảnh báo — KHÔNG BAO GIỜ được làm has_blocking() trả true.
        //
        // Không còn W01 ("Thiếu số phòng — vẫn xuất được"): FINDING C1 cho
        // thấy đó là lời nói dối — thiếu phòng thì KHÔNG xuất được, vì ngày
        // đến/ngày đi lấy từ lượt lưu trú. E16 ở trên đã chặn đúng chỗ đó;
        // giữ thêm W01 sẽ mâu thuẫn ngay trên cùng một thẻ.
        if is_vn && id.phone.as_deref().unwrap_or("").is_empty() {
            out.push(Finding::warning(
                "W02",
                link,
                Some(Field::Phone),
                "Thiếu số điện thoại.",
            ));
        }
        if row.stay_reason == "1" {
            out.push(Finding::warning(
                "W03",
                link,
                None,
                "Lý do lưu trú vẫn là mặc định 'Du lịch', chưa ai đổi.",
            ));
        }
        if days_between(&row.stay.check_in, today).unwrap_or(0) >= 1 {
            out.push(Finding::warning(
                "W04",
                link,
                None,
                "Đã quá 24h kể từ lúc khách đến mà chưa có lô nào được xác nhận.",
            ));
        }
        // W05 (extract_confidence = needs_review) sinh ở lớp repo/commands —
        // DeclarationRow không mang trường đó. Xem Task 10 của Kế hoạch B.
        if id.doc_type_source.as_deref() == Some("heuristic") {
            out.push(Finding::warning(
                "W06",
                link,
                Some(Field::DocType),
                "Loại giấy tờ do máy suy từ ngày cấp, chưa ai xác nhận.",
            ));
        }
        // W07 — H10: đúng dạng nhưng template của cổng không có mã này.
        if is_iso3_shaped(&id.nationality_iso3)
            && !catalog.has_code(CatalogList::QuocTich, &id.nationality_iso3)
        {
            out.push(Finding::warning(
                "W07",
                link,
                Some(Field::Nationality),
                &format!(
                    "Mã quốc tịch {} không có trong danh mục của template — cổng có thể không nhận. Kiểm tra lại sau khi nộp.",
                    id.nationality_iso3
                ),
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::{Identity, StayInfo};

    fn cat() -> Catalog {
        Catalog::load().unwrap()
    }

    fn vn_row() -> DeclarationRow {
        DeclarationRow {
            link_id: "link-1".into(),
            identity: Identity {
                id: "id-1".into(),
                full_name: "Phan Thị Mỹ Hà".into(),
                dob: "1995-07-28".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                doc_type_code: Some("1".into()),
                doc_type_source: Some("human".into()),
                doc_no: Some("058195006173".into()),
                phone: Some("0901234567".into()),
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "booking-1".into(),
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

    fn nnn_row() -> DeclarationRow {
        DeclarationRow {
            link_id: "link-2".into(),
            identity: Identity {
                id: "id-2".into(),
                full_name: "ZOLOCHEVSKAIA VERONIKA".into(),
                dob: "1990-03-08".into(),
                gender: "F".into(),
                nationality_iso3: "RUS".into(),
                passport_no: Some("777785671".into()),
                passport_expiry: Some("2035-12-09".into()),
                visa_valid_until: Some("2026-09-15".into()),
                name_confirmed_by_human: true,
                ..Default::default()
            },
            stay: StayInfo {
                stay_id: "booking-2".into(),
                room_no: "5A".into(),
                check_in: "2026-07-26".into(),
                expected_out: "2026-07-30".into(),
                check_in_raw: "2026-07-26T09:00:00+07:00".into(),
                ..Default::default()
            },
            stay_reason: "1".into(),
            stay_reason_note: None,
        }
    }

    fn codes(f: &[Finding]) -> Vec<String> {
        f.iter().map(|x| x.code.clone()).collect()
    }

    #[test]
    fn clean_rows_produce_no_blocking_findings() {
        let f = validate(&[vn_row(), nnn_row()], &cat(), "2026-07-26");
        assert!(!has_blocking(&f), "gặp: {:?}", codes(&f));
    }

    #[test]
    fn e02_single_token_name_blocks_unless_confirmed() {
        let mut r = vn_row();
        r.identity.full_name = "Hà".into();
        assert!(codes(&validate(&[r.clone()], &cat(), "2026-07-26")).contains(&"E02".to_string()));

        // mononym có thật (hộ chiếu Indonesia) — người tick để gỡ chặn
        r.identity.single_token_name_ok = true;
        assert!(!codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E02".to_string()));
    }

    #[test]
    fn e03_nnn_name_must_be_mrz_shaped() {
        let mut r = nnn_row();
        r.identity.full_name = "Nguyễn Văn A".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E03".to_string()));
    }

    // G5 — không ai xác nhận tên thì không được xuất
    #[test]
    fn e04_blocks_unconfirmed_foreign_name() {
        let mut r = nnn_row();
        r.identity.name_confirmed_by_human = false;
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E04".to_string()));
    }

    /// H10 — danh mục của cổng thiếu Brazil, Brunei, Bulgaria. Chặn cứng theo
    /// danh mục sẽ khóa oan một khách Brazil hợp lệ và người vận hành không có
    /// đường nào đi tiếp. E05 chỉ bắt lỗi DẠNG; thiếu trong danh mục là W07.
    #[test]
    fn e05_rejects_malformed_but_not_merely_missing_codes() {
        let mut bad = nnn_row();
        bad.identity.nationality_iso3 = "Ru".into();
        assert!(codes(&validate(&[bad], &cat(), "2026-07-26")).contains(&"E05".to_string()));

        let mut brazil = nnn_row();
        brazil.identity.nationality_iso3 = "BRA".into();
        let f = validate(&[brazil], &cat(), "2026-07-26");
        assert!(
            !codes(&f).contains(&"E05".to_string()),
            "khách Brazil phải khai được"
        );
        assert!(
            codes(&f).contains(&"W07".to_string()),
            "nhưng phải được cảnh báo"
        );
        assert!(!has_blocking(&f));
    }

    /// H10 — lỗ hổng template nằm giữa Botswana và Cameroon; Brunei cũng rơi
    /// vào đó. Mã đúng dạng mà vắng danh mục thì chỉ cảnh báo, không chặn.
    #[test]
    fn w07_warns_on_wellformed_code_missing_from_catalog() {
        let mut brunei = nnn_row();
        brunei.identity.nationality_iso3 = "BRN".into();
        let f = validate(&[brunei], &cat(), "2026-07-26");
        assert!(codes(&f).contains(&"W07".to_string()));
        assert!(!codes(&f).contains(&"E05".to_string()));
        assert!(!has_blocking(&f), "W07 không bao giờ được chặn xuất file");

        // mã có trong danh mục thì im lặng
        let f_ok = validate(&[nnn_row()], &cat(), "2026-07-26");
        assert!(!codes(&f_ok).contains(&"W07".to_string()));
    }

    #[test]
    fn e07_rejects_checkout_before_checkin() {
        let mut r = vn_row();
        r.stay.expected_out = "2026-07-20".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E07".to_string()));
    }

    #[test]
    fn e08_and_e09_guard_the_visa_window() {
        let mut r = nnn_row();
        r.identity.visa_valid_until = None;
        assert!(codes(&validate(&[r.clone()], &cat(), "2026-07-26")).contains(&"E08".to_string()));

        // khách sẽ ở quá hạn tạm trú — chuyện cơ sở lưu trú phải biết TRƯỚC
        r.identity.visa_valid_until = Some("2026-07-28".into());
        r.stay.expected_out = "2026-07-30".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E09".to_string()));
    }

    /// G10 — cái bẫy nguy hiểm nhất: MRZ cho ngày hết hạn hộ chiếu 2035, rất
    /// dễ bị map nhầm thành thời hạn tạm trú.
    #[test]
    fn e10_catches_passport_expiry_copied_into_visa_field() {
        let mut r = nnn_row();
        r.identity.visa_valid_until = Some("2035-12-09".into());
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E10".to_string()));
    }

    #[test]
    fn e11_and_e12_require_free_text_when_code_is_other() {
        let mut r = vn_row();
        r.identity.doc_type_code = Some("9".into());
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E11".to_string()));

        let mut r2 = vn_row();
        r2.stay_reason = "20".into();
        assert!(codes(&validate(&[r2], &cat(), "2026-07-26")).contains(&"E12".to_string()));
    }

    #[test]
    fn e13_rejects_malformed_document_numbers() {
        let mut r = vn_row();
        r.identity.doc_no = Some("0581 950/06173".into());
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E13".to_string()));
    }

    #[test]
    fn e14_catches_duplicate_within_one_batch() {
        let f = validate(&[vn_row(), vn_row()], &cat(), "2026-07-26");
        assert!(codes(&f).contains(&"E14".to_string()));
    }

    #[test]
    fn e06_rejects_enum_value_outside_catalog() {
        let mut r = vn_row();
        r.stay_reason = "99".into();
        assert!(codes(&validate(&[r], &cat(), "2026-07-26")).contains(&"E06".to_string()));
    }

    #[test]
    fn warnings_do_not_block_export() {
        let mut r = vn_row();
        r.identity.phone = None;
        r.stay_reason = "1".into(); // vẫn là mặc định
        r.identity.doc_type_source = Some("heuristic".into());
        let f = validate(&[r], &cat(), "2026-07-26");
        let c = codes(&f);
        assert!(c.contains(&"W02".to_string()));
        assert!(c.contains(&"W03".to_string()));
        assert!(c.contains(&"W06".to_string()));
        assert!(!has_blocking(&f), "cảnh báo không được chặn xuất file");
    }

    /// FINDING C1 — trước khi sửa: mọi khách vừa quét (chưa gắn phòng) rơi
    /// vào E01 với thông báo "thiếu ngày đến, ngày đi dự kiến", bấm vào mở
    /// ManualForm (không có ô ngày nào), còn W01 nói "vẫn xuất được" ngay bên
    /// dưới — mâu thuẫn với nút xuất đã tắt. Sau khi sửa: nguyên nhân thật
    /// (chưa chọn phòng) phải lộ ra ở đúng một finding, E01 im lặng vì danh
    /// tính ở đây đã đủ, và W01 biến mất hẳn.
    #[test]
    fn e16_blocks_a_roomless_link_instead_of_e01_blaming_the_dates() {
        let mut r = vn_row();
        r.stay = StayInfo::default(); // chưa gắn phòng — v21
        let f = validate(&[r], &cat(), "2026-07-26");
        let c = codes(&f);
        assert!(c.contains(&"E16".to_string()), "phải chặn vì chưa chọn phòng: {c:?}");
        assert!(
            !c.contains(&"E01".to_string()),
            "danh tính đầy đủ — E01 không được đổ lỗi thiếu ngày cho danh tính: {c:?}"
        );
        assert!(
            !c.contains(&"W01".to_string()),
            "W01 phải biến mất — nó từng nói 'vẫn xuất được', mâu thuẫn với E16: {c:?}"
        );
        assert!(has_blocking(&f), "chưa chọn phòng phải chặn xuất file");
    }

    #[test]
    fn e16_stays_silent_once_a_room_is_chosen() {
        let f = validate(&[vn_row()], &cat(), "2026-07-26");
        assert!(!codes(&f).contains(&"E16".to_string()));
    }

    /// Khách CÓ phòng nhưng thiếu field danh tính thật (không phải ngày
    /// tháng) vẫn phải giữ nguyên hành vi cũ của E01 — bấm mở được form sửa.
    #[test]
    fn e01_still_blocks_on_a_real_missing_identity_field_when_a_room_is_assigned() {
        let mut r = vn_row();
        r.identity.dob = String::new();
        let f = validate(&[r], &cat(), "2026-07-26");
        let c = codes(&f);
        assert!(c.contains(&"E01".to_string()));
        assert!(!c.contains(&"E16".to_string()), "đã có phòng — E16 không được nổ");
    }

    #[test]
    fn w04_flags_declaration_past_the_24h_deadline() {
        let mut r = vn_row();
        r.stay.check_in = "2026-07-24".into();
        r.stay.check_in_raw = "2026-07-24T09:00:00+07:00".into();
        let f = validate(&[r], &cat(), "2026-07-26");
        assert!(codes(&f).contains(&"W04".to_string()));
    }
}
