//! Khai báo khách nước ngoài — định dạng XML của cổng.
//!
//! Khác XLSX ở hai chỗ dễ nhầm: `ma_quoc_tich` chỉ mang mã ISO3 trần ("RUS",
//! không phải "RUS - Russia"), và `ngay_tra_phong` bị BỎ HẲN tag khi khách
//! chưa trả phòng — không bao giờ ghi tag rỗng.

use crate::declaration::model::DeclarationRow;
use crate::declaration::normalizer::iso_to_portal;

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn record(seq: usize, r: &DeclarationRow) -> Result<String, String> {
    let id = &r.identity;
    let dob = iso_to_portal(&id.dob).ok_or_else(|| format!("Ngày sinh không hợp lệ: {}", id.dob))?;
    let check_in = iso_to_portal(&r.stay.check_in)
        .ok_or_else(|| format!("Ngày đến không hợp lệ: {}", r.stay.check_in))?;
    let expected = iso_to_portal(&r.stay.expected_out)
        .ok_or_else(|| format!("Ngày đi không hợp lệ: {}", r.stay.expected_out))?;
    let visa = id
        .visa_valid_until
        .as_deref()
        .and_then(iso_to_portal)
        .ok_or_else(|| "Thiếu thời hạn tạm trú".to_string())?;

    let mut s = String::new();
    s.push_str("    <THONG_TIN_KHACH>\n");
    s.push_str(&format!("        <so_thu_tu>{seq}</so_thu_tu>\n"));
    s.push_str(&format!("        <ho_ten>{}</ho_ten>\n", escape(&id.full_name)));
    s.push_str(&format!("        <ngay_sinh>{dob}</ngay_sinh>\n"));
    // v1 chỉ xuất khách có ngày sinh đủ, nên luôn là D
    s.push_str("        <ngay_sinh_dung_den>D</ngay_sinh_dung_den>\n");
    s.push_str(&format!(
        "        <gioi_tinh>{}</gioi_tinh>\n",
        escape(&id.gender)
    ));
    // Chỉ mã ISO3, KHÁC XLSX vốn ghi "RUS - Russia"
    s.push_str(&format!(
        "        <ma_quoc_tich>{}</ma_quoc_tich>\n",
        escape(&id.nationality_iso3)
    ));
    s.push_str(&format!(
        "        <so_ho_chieu>{}</so_ho_chieu>\n",
        escape(id.passport_no.as_deref().unwrap_or(""))
    ));
    s.push_str(&format!(
        "        <so_phong>{}</so_phong>\n",
        escape(&r.stay.room_no)
    ));
    s.push_str(&format!("        <ngay_den>{check_in}</ngay_den>\n"));
    s.push_str(&format!(
        "        <ngay_di_du_kien>{expected}</ngay_di_du_kien>\n"
    ));
    // F9: bỏ hẳn tag nếu khách chưa trả phòng
    if let Some(actual) = r.stay.actual_out.as_deref().and_then(iso_to_portal) {
        s.push_str(&format!("        <ngay_tra_phong>{actual}</ngay_tra_phong>\n"));
    }
    s.push_str(&format!(
        "        <thoi_han_tam_tru>{visa}</thoi_han_tam_tru>\n"
    ));
    s.push_str("    </THONG_TIN_KHACH>\n");
    Ok(s)
}

fn example_record(seq: usize) -> String {
    format!(
        "    <THONG_TIN_KHACH>\n\
         \x20       <so_thu_tu>{seq}</so_thu_tu>\n\
         \x20       <ho_ten>[EXAMPLE]</ho_ten>\n\
         \x20       <ngay_sinh>01/01/1980</ngay_sinh>\n\
         \x20       <ngay_sinh_dung_den>D</ngay_sinh_dung_den>\n\
         \x20       <gioi_tinh>M</gioi_tinh>\n\
         \x20       <ma_quoc_tich>USA</ma_quoc_tich>\n\
         \x20       <so_ho_chieu>[EXAMPLE]</so_ho_chieu>\n\
         \x20       <so_phong>0</so_phong>\n\
         \x20       <ngay_den>01/01/2000</ngay_den>\n\
         \x20       <ngay_di_du_kien>02/01/2000</ngay_di_du_kien>\n\
         \x20       <thoi_han_tam_tru>03/01/2000</thoi_han_tam_tru>\n\
         \x20   </THONG_TIN_KHACH>\n"
    )
}

/// `lead_example` bật khi cổng bỏ record đầu theo vị trí (§14.1 — chưa test).
pub fn render(rows: &[DeclarationRow], lead_example: bool) -> Result<String, String> {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<KHAI_BAO_TAM_TRU>\n");
    let mut seq = 1usize;
    if lead_example {
        out.push_str(&example_record(seq));
        seq += 1;
    }
    for r in rows {
        out.push_str(&record(seq, r)?);
        seq += 1;
    }
    out.push_str("</KHAI_BAO_TAM_TRU>\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::model::{Identity, StayInfo};

    fn row(name: &str, room: &str, actual_out: Option<&str>) -> DeclarationRow {
        DeclarationRow {
            link_id: "l".into(),
            identity: Identity {
                full_name: name.into(),
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
                stay_id: "b".into(),
                room_no: room.into(),
                check_in: "2026-07-25".into(),
                expected_out: "2026-07-30".into(),
                actual_out: actual_out.map(|s| s.to_string()),
                check_in_raw: "2026-07-25T09:00:00+07:00".into(),
            },
            stay_reason: "1".into(),
            stay_reason_note: None,
        }
    }

    #[test]
    fn renders_the_documented_shape() {
        let xml = render(
            &[row("ZOLOCHEVSKAIA VERONIKA", "5A", Some("2026-07-29"))],
            false,
        )
        .unwrap();
        let expected = r#"<?xml version="1.0" encoding="UTF-8"?>
<KHAI_BAO_TAM_TRU>
    <THONG_TIN_KHACH>
        <so_thu_tu>1</so_thu_tu>
        <ho_ten>ZOLOCHEVSKAIA VERONIKA</ho_ten>
        <ngay_sinh>08/03/1990</ngay_sinh>
        <ngay_sinh_dung_den>D</ngay_sinh_dung_den>
        <gioi_tinh>F</gioi_tinh>
        <ma_quoc_tich>RUS</ma_quoc_tich>
        <so_ho_chieu>777785671</so_ho_chieu>
        <so_phong>5A</so_phong>
        <ngay_den>25/07/2026</ngay_den>
        <ngay_di_du_kien>30/07/2026</ngay_di_du_kien>
        <ngay_tra_phong>29/07/2026</ngay_tra_phong>
        <thoi_han_tam_tru>15/09/2026</thoi_han_tam_tru>
    </THONG_TIN_KHACH>
</KHAI_BAO_TAM_TRU>
"#;
        assert_eq!(xml, expected);
    }

    // F9 — khách chưa trả phòng thì BỎ HẲN tag, không ghi tag rỗng
    #[test]
    fn omits_checkout_tag_when_guest_has_not_left() {
        let xml = render(&[row("A B", "5A", None)], false).unwrap();
        assert!(!xml.contains("ngay_tra_phong"));
    }

    #[test]
    fn nationality_is_bare_iso3_unlike_xlsx() {
        let xml = render(&[row("A B", "5A", None)], false).unwrap();
        assert!(xml.contains("<ma_quoc_tich>RUS</ma_quoc_tich>"));
        assert!(!xml.contains("RUS - "));
    }

    #[test]
    fn sequence_numbers_are_consecutive_from_one() {
        let xml = render(
            &[
                row("A B", "1", None),
                row("C D", "2", None),
                row("E F", "3", None),
            ],
            false,
        )
        .unwrap();
        assert!(xml.contains("<so_thu_tu>1</so_thu_tu>"));
        assert!(xml.contains("<so_thu_tu>2</so_thu_tu>"));
        assert!(xml.contains("<so_thu_tu>3</so_thu_tu>"));
    }

    /// Một dấu `&` chưa escape làm hỏng cả file — và cổng sẽ nhận file hỏng
    /// đó rồi báo "thành công".
    #[test]
    fn escapes_xml_metacharacters() {
        assert_eq!(escape("A & B"), "A &amp; B");
        assert_eq!(escape("<x>"), "&lt;x&gt;");
        assert_eq!(escape("say \"hi\""), "say &quot;hi&quot;");
        assert_eq!(escape("it's"), "it&apos;s");

        let xml = render(&[row("SMITH & SONS", "5A", None)], false).unwrap();
        assert!(xml.contains("SMITH &amp; SONS"));
        assert!(!xml.contains("SMITH & SONS"));
    }

    /// §14.1 chưa test được trên cổng. Cờ này biến cả ba nhánh kết quả
    /// thành một dòng setting thay vì một lần sửa code.
    #[test]
    fn lead_example_record_can_be_switched_on() {
        let xml = render(&[row("A B", "5A", None)], true).unwrap();
        assert!(xml.contains("[EXAMPLE]"));
        assert!(xml.contains("<so_thu_tu>1</so_thu_tu>"));
        assert!(xml.contains("<so_thu_tu>2</so_thu_tu>"));

        let off = render(&[row("A B", "5A", None)], false).unwrap();
        assert!(!off.contains("[EXAMPLE]"));
    }

    #[test]
    fn rejects_row_without_full_dob() {
        let mut r = row("A B", "5A", None);
        r.identity.dob = "1990".into();
        assert!(render(&[r], false).is_err());
    }
}
