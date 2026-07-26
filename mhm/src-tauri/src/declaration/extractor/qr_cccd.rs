//! Trích danh tính từ mã QR mặt sau thẻ CCCD / Căn Cước.
//!
//! Không log payload (§12.3).

use super::IdentityExtractor;
use crate::declaration::model::{Confidence, ExtractResult, Field, Identity, Source};
use crate::declaration::normalizer::{gender_from_qr, qr_date_to_iso};

pub struct QrCccdExtractor;

/// Ngày cấp từ mốc này trở đi là Thẻ Căn Cước, trước đó là Thẻ CCCD.
const CAN_CUOC_FROM: &str = "2024-07-01";

pub fn infer_doc_type(issue_date_iso: &str) -> &'static str {
    if issue_date_iso >= CAN_CUOC_FROM {
        "8"
    } else {
        "1"
    }
}

/// Payload QR thẻ CCCD: 7 trường phân cách `|`.
///
/// Khác 7 trường => fail. Không cố cứu payload lệch: payload lệch nghĩa là
/// đang đoán, và đoán ở đây là khai sai với cơ quan công an.
pub fn parse_cccd_payload(payload: &str) -> Option<Identity> {
    let parts: Vec<&str> = payload.split('|').collect();
    if parts.len() != 7 {
        return None;
    }

    let doc_no = parts[0].trim();
    let full_name = parts[2].trim();
    let dob = qr_date_to_iso(parts[3].trim())?;
    let gender = gender_from_qr(parts[4])?;
    let address = parts[5].trim();
    let issue_date = qr_date_to_iso(parts[6].trim())?;

    if doc_no.is_empty() || full_name.is_empty() {
        return None;
    }

    Some(Identity {
        id: uuid::Uuid::new_v4().to_string(),
        full_name: full_name.to_string(),
        dob,
        gender: gender.to_string(),
        nationality_iso3: "VNM".to_string(), // CCCD chỉ cấp cho công dân VN
        doc_type_code: Some(infer_doc_type(&issue_date).to_string()),
        doc_type_source: Some("heuristic".to_string()),
        doc_no: Some(doc_no.to_string()),
        // G7: chuỗi thô, KHÔNG parse thành tỉnh/phường
        address_detail: Some(address.to_string()),
        ..Default::default()
    })
}

/// Thang biến thể tiền xử lý. G2/G3: ảnh THÔ trước, xử lý mạnh sau.
fn decode_qr(image: &image::DynamicImage) -> Option<String> {
    let attempts: Vec<image::DynamicImage> = vec![
        image.clone(),
        image.resize(
            image.width() * 2,
            image.height() * 2,
            image::imageops::FilterType::Lanczos3,
        ),
        image.resize(
            image.width() * 4,
            image.height() * 4,
            image::imageops::FilterType::Lanczos3,
        ),
    ];

    for candidate in attempts {
        let luma = candidate.to_luma8();
        let (w, h) = (luma.width(), luma.height());
        if let Ok(res) = rxing::helpers::detect_in_luma(
            luma.into_raw(),
            w,
            h,
            Some(rxing::BarcodeFormat::QR_CODE),
        ) {
            return Some(res.getText().to_string());
        }
    }
    None
}

impl IdentityExtractor for QrCccdExtractor {
    fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult> {
        let payload = decode_qr(image)?;
        let identity = parse_cccd_payload(&payload)?;
        Some(ExtractResult {
            source: Source::QrCccd,
            confidence: Confidence::Verified,
            identity,
            // Loại giấy tờ là heuristic (G8) nên luôn cần người ngó qua
            review_hints: vec![Field::DocType],
            crop_for_review: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = "058195006173|264445725|Phan Thị Mỹ Hà|28071995|Nữ|KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận|27062021";

    #[test]
    fn parses_all_seven_fields() {
        let id = parse_cccd_payload(PAYLOAD).expect("payload hợp lệ");
        assert_eq!(id.doc_no.as_deref(), Some("058195006173"));
        assert_eq!(id.full_name, "Phan Thị Mỹ Hà"); // giữ nguyên dấu
        assert_eq!(id.dob, "1995-07-28");
        assert_eq!(id.gender, "F"); // QR trả "Nữ", không phải "F"
        assert_eq!(id.nationality_iso3, "VNM");
        assert_eq!(
            id.address_detail.as_deref(),
            Some("KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận")
        );
    }

    // G7 — địa chỉ trong QR đã lạc hậu so với danh mục. Không được parse.
    #[test]
    fn address_is_kept_raw_never_parsed() {
        let id = parse_cccd_payload(PAYLOAD).unwrap();
        let addr = id.address_detail.unwrap();
        assert!(addr.contains("Ninh Thuận")); // tỉnh này không còn tồn tại
        assert!(addr.contains("Mỹ Đông")); // phường này không còn trong 3323
    }

    // Payload lệch nghĩa là đang đoán, mà đoán ở đây là khai sai.
    #[test]
    fn wrong_field_count_is_a_failure_not_a_guess() {
        assert!(parse_cccd_payload("a|b|c|d|e|f").is_none()); // 6
        assert!(parse_cccd_payload("a|b|c|d|e|f|g|h").is_none()); // 8
        assert!(parse_cccd_payload("").is_none());
    }

    #[test]
    fn malformed_date_rejects_payload() {
        let bad = "058195006173|264445725|Nguyễn Văn A|99999999|Nam|Hà Nội|27062021";
        assert!(parse_cccd_payload(bad).is_none());
    }

    // G8 — QR không phân biệt CCCD với Căn Cước; suy từ ngày cấp.
    #[test]
    fn doc_type_inferred_from_issue_date() {
        assert_eq!(infer_doc_type("2021-06-27"), "1"); // Thẻ CCCD
        assert_eq!(infer_doc_type("2024-06-30"), "1");
        assert_eq!(infer_doc_type("2024-07-01"), "8"); // Thẻ Căn Cước
        assert_eq!(infer_doc_type("2025-01-15"), "8");
    }

    #[test]
    fn doc_type_is_marked_as_a_guess() {
        let id = parse_cccd_payload(PAYLOAD).unwrap();
        assert_eq!(id.doc_type_code.as_deref(), Some("1"));
        // W06 dựa vào đúng cột này
        assert_eq!(id.doc_type_source.as_deref(), Some("heuristic"));
    }

    #[test]
    fn empty_old_id_number_is_allowed() {
        let p = "058195006173||Phan Thị Mỹ Hà|28071995|Nữ|Nha Trang|27062021";
        assert!(parse_cccd_payload(p).is_some());
    }

    #[test]
    fn decodes_a_real_qr_image() {
        use rxing::Writer;
        let writer = rxing::MultiFormatWriter;
        let matrix = writer
            .encode(PAYLOAD, &rxing::BarcodeFormat::QR_CODE, 600, 600)
            .unwrap();
        let img: image::DynamicImage = (&matrix).into();

        let res = QrCccdExtractor
            .try_extract(&img)
            .expect("phải decode được QR");
        assert_eq!(res.source, Source::QrCccd);
        assert_eq!(res.confidence, Confidence::Verified);
        assert_eq!(res.identity.full_name, "Phan Thị Mỹ Hà");
        assert!(res.review_hints.contains(&Field::DocType)); // heuristic
    }
}
