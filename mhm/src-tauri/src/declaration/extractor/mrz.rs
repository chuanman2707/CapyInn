//! MRZ TD3 (hộ chiếu): chấm điểm bằng 5 check digit của dòng 2.
//!
//! G5: cả 5 check digit đều nằm ở dòng 2. Dòng 1 — nơi chứa họ tên — không
//! có checksum nào bảo vệ. Nên điểm 5/5 KHÔNG bao giờ được coi là đã xác nhận
//! tên. `name_confirmed_by_human` chỉ do người bấm nút mới thành true.

use super::IdentityExtractor;
use crate::declaration::model::{Confidence, ExtractResult, Field, Identity, Source};
use crate::declaration::normalizer::{gender_from_mrz, mrz_date_to_iso, DateKind};

/// Tầng OCR nằm sau trait riêng vì engine chưa chốt: `ocr-rs` đã có sẵn trong
/// repo (0 dependency mới) còn tesseract có bằng chứng đo thật. `kbtt_probe`
/// chạy cả hai trên ảnh thật rồi mới quyết định.
pub trait MrzOcr {
    fn recognize_lines(&self, img: &image::DynamicImage) -> Vec<String>;
}

pub fn char_value(c: char) -> Option<u32> {
    match c {
        '<' => Some(0),
        '0'..='9' => Some(c as u32 - '0' as u32),
        'A'..='Z' => Some(c as u32 - 'A' as u32 + 10),
        _ => None,
    }
}

/// weights = [7,3,1] lặp lại; check = (Σ giá_trị[i] × w[i mod 3]) mod 10
pub fn checksum(data: &str) -> u32 {
    const WEIGHTS: [u32; 3] = [7, 3, 1];
    data.chars()
        .enumerate()
        .map(|(i, c)| char_value(c).unwrap_or(0) * WEIGHTS[i % 3])
        .sum::<u32>()
        % 10
}

#[derive(Debug, Clone)]
pub struct Td3 {
    pub line1: String,
    pub line2: String,
}

fn pad44(s: &str) -> String {
    let cleaned: String = s.chars().filter(|c| char_value(*c).is_some()).collect();
    let mut out = cleaned;
    if out.len() > 44 {
        out.truncate(44);
    } else {
        while out.len() < 44 {
            out.push('<');
        }
    }
    out
}

impl Td3 {
    pub fn parse(line1: &str, line2: &str) -> Option<Td3> {
        let l1 = pad44(line1);
        let l2 = pad44(line2);
        if l1.len() != 44 || l2.len() != 44 {
            return None;
        }
        Some(Td3 {
            line1: l1,
            line2: l2,
        })
    }

    fn digit_at(&self, idx1: usize) -> Option<u32> {
        self.line2.chars().nth(idx1 - 1)?.to_digit(10)
    }

    fn slice(&self, from1: usize, to1: usize) -> String {
        self.line2
            .chars()
            .skip(from1 - 1)
            .take(to1 - from1 + 1)
            .collect()
    }

    /// Đếm số check digit khớp, 0..=5. Đây là HÀM CHẤM ĐIỂM, không phải
    /// bước kiểm cuối — biến thể tiền xử lý nào điểm cao nhất thì thắng.
    pub fn checksum_score(&self) -> u8 {
        let mut score = 0u8;

        if self.digit_at(10) == Some(checksum(&self.slice(1, 9))) {
            score += 1;
        }
        if self.digit_at(20) == Some(checksum(&self.slice(14, 19))) {
            score += 1;
        }
        if self.digit_at(28) == Some(checksum(&self.slice(22, 27))) {
            score += 1;
        }
        if self.digit_at(43) == Some(checksum(&self.slice(29, 42))) {
            score += 1;
        }
        let composite = format!(
            "{}{}{}",
            self.slice(1, 10),
            self.slice(14, 20),
            self.slice(22, 43)
        );
        if self.digit_at(44) == Some(checksum(&composite)) {
            score += 1;
        }

        score
    }

    /// Dòng 1 vị trí 6–44: `HỌ<<TÊN`. Giữ HOA không dấu — đúng dạng cổng cần.
    pub fn full_name(&self) -> String {
        let raw: String = self.line1.chars().skip(5).collect();
        raw.replace('<', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn to_identity(&self, today_year: u32) -> Option<Identity> {
        let passport_no = self.slice(1, 9).replace('<', "");
        let nationality = self.slice(11, 13);
        let dob = mrz_date_to_iso(&self.slice(14, 19), DateKind::Birth, today_year)?;
        let gender = gender_from_mrz(self.line2.chars().nth(20)?)?;
        let expiry = mrz_date_to_iso(&self.slice(22, 27), DateKind::Expiry, today_year)?;

        Some(Identity {
            id: uuid::Uuid::new_v4().to_string(),
            full_name: self.full_name(),
            dob,
            gender: gender.to_string(),
            nationality_iso3: nationality,
            passport_no: Some(passport_no),
            passport_expiry: Some(expiry),
            // G5: KHÔNG BAO GIỜ tự đặt thành true, kể cả khi 5/5.
            // 5 check digit nằm hết ở dòng 2; dòng 1 không được bảo vệ.
            name_confirmed_by_human: false,
            ..Default::default()
        })
    }
}

pub struct MrzExtractor<O: MrzOcr> {
    ocr: O,
    today_year: u32,
}

impl<O: MrzOcr> MrzExtractor<O> {
    pub fn new(ocr: O, today_year: u32) -> Self {
        MrzExtractor { ocr, today_year }
    }
}

fn looks_like_mrz(line: &str) -> bool {
    let cleaned: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    cleaned.len() >= 30 && cleaned.chars().all(|c| char_value(c).is_some())
}

impl<O: MrzOcr> IdentityExtractor for MrzExtractor<O> {
    fn try_extract(&self, image: &image::DynamicImage) -> Option<ExtractResult> {
        // Biến thể theo đúng thứ tự G3: THÔ trước, xử lý mạnh sau.
        let variants: Vec<image::DynamicImage> = vec![
            image.clone(),
            image.resize(
                image.width() * 2,
                image.height() * 2,
                image::imageops::FilterType::Lanczos3,
            ),
        ];

        let mut best: Option<Td3> = None;
        let mut best_score = 0u8;

        for variant in &variants {
            let lines: Vec<String> = self
                .ocr
                .recognize_lines(variant)
                .into_iter()
                .filter(|l| looks_like_mrz(l))
                .collect();

            for pair in lines.windows(2) {
                if let Some(td3) = Td3::parse(&pair[0], &pair[1]) {
                    let score = td3.checksum_score();
                    if score > best_score {
                        best_score = score;
                        best = Some(td3);
                    }
                }
            }
        }

        let td3 = best?;
        if best_score < 3 {
            return None; // rơi xuống ManualExtractor
        }

        let identity = td3.to_identity(self.today_year)?;
        Some(ExtractResult {
            source: Source::MrzTd3,
            confidence: if best_score == 5 {
                Confidence::Verified
            } else {
                Confidence::NeedsReview
            },
            identity,
            // Tên LUÔN cần người xác nhận, bất kể điểm checksum.
            review_hints: vec![Field::FullName],
            crop_for_review: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cặp dòng đã verify 5/5 trên hộ chiếu thật
    const L1: &str = "P<RUSZOLOCHEVSKAIA<<VERONIKA<<<<<<<<<<<<<<<<";
    const L2: &str = "7777856719RUS9003082F3512090<<<<<<<<<<<<<<00";

    #[test]
    fn char_values_follow_icao() {
        assert_eq!(char_value('<'), Some(0));
        assert_eq!(char_value('0'), Some(0));
        assert_eq!(char_value('9'), Some(9));
        assert_eq!(char_value('A'), Some(10));
        assert_eq!(char_value('Z'), Some(35));
        assert_eq!(char_value('a'), None);
    }

    #[test]
    fn checksum_matches_known_values() {
        assert_eq!(checksum("777785671"), 9);
        assert_eq!(checksum("900308"), 2);
        assert_eq!(checksum("351209"), 0);
    }

    #[test]
    fn known_good_mrz_scores_five_of_five() {
        let td3 = Td3::parse(L1, L2).expect("phải parse được");
        assert_eq!(td3.checksum_score(), 5);
    }

    #[test]
    fn extracts_line2_fields_correctly() {
        let td3 = Td3::parse(L1, L2).unwrap();
        let id = td3.to_identity(2026).unwrap();
        assert_eq!(id.passport_no.as_deref(), Some("777785671"));
        assert_eq!(id.nationality_iso3, "RUS");
        assert_eq!(id.dob, "1990-03-08");
        assert_eq!(id.gender, "F");
        // G6: 35 ở ô hết hạn là 2035, không phải 1935
        assert_eq!(id.passport_expiry.as_deref(), Some("2035-12-09"));
    }

    #[test]
    fn name_joins_surname_and_given_names() {
        let td3 = Td3::parse(L1, L2).unwrap();
        assert_eq!(td3.full_name(), "ZOLOCHEVSKAIA VERONIKA");
    }

    /// G5 — TEST THEN CHỐT CỦA CẢ MODULE.
    ///
    /// Cả 5 check digit nằm ở dòng 2. Dòng 1 (họ tên) không có checksum nào.
    /// Nên một MRZ hỏng tên vẫn đạt 5/5, và app KHÔNG được coi tên đó là đúng.
    #[test]
    fn perfect_checksum_never_vouches_for_the_name() {
        let corrupted_name = "P<RUSZOLOCHEVSKAIA<X<VERONIKAK<<<<<<<<<<<<<<";
        let td3 = Td3::parse(corrupted_name, L2).unwrap();

        assert_eq!(td3.checksum_score(), 5, "dòng 2 vẫn hoàn hảo");
        assert_ne!(
            td3.full_name(),
            "ZOLOCHEVSKAIA VERONIKA",
            "tên đã sai mà không checksum nào phát hiện"
        );

        let id = td3.to_identity(2026).unwrap();
        assert!(
            !id.name_confirmed_by_human,
            "5/5 checksum KHÔNG được tự xác nhận tên"
        );
    }

    #[test]
    fn short_lines_are_padded_not_rejected() {
        let short = "7777856719RUS9003082F3512090";
        let td3 = Td3::parse(L1, short).expect("pad về 44 ký tự");
        assert_eq!(td3.line2.len(), 44);
    }

    #[test]
    fn garbage_scores_low_enough_to_fall_through_to_manual() {
        let td3 = Td3::parse(
            "P<XXXAAAAAAAAAA<<BBBBBB<<<<<<<<<<<<<<<<<<<<<",
            "1111111111XXX1111111X1111111<<<<<<<<<<<<<<11",
        )
        .unwrap();
        assert!(td3.checksum_score() < 3);
    }

    struct FakeOcr(Vec<String>);
    impl MrzOcr for FakeOcr {
        fn recognize_lines(&self, _img: &image::DynamicImage) -> Vec<String> {
            self.0.clone()
        }
    }

    #[test]
    fn extractor_picks_mrz_lines_out_of_noise() {
        let ocr = FakeOcr(vec![
            "PASSPORT".to_string(),
            "Nơi cấp / Place of issue".to_string(),
            L1.to_string(),
            L2.to_string(),
        ]);
        let img = image::DynamicImage::new_luma8(10, 10);
        let res = MrzExtractor::new(ocr, 2026)
            .try_extract(&img)
            .expect("phải trích được");
        assert_eq!(res.source, Source::MrzTd3);
        assert_eq!(res.confidence, Confidence::Verified);
        assert!(res.review_hints.contains(&Field::FullName));
        assert!(!res.identity.name_confirmed_by_human);
    }

    #[test]
    fn extractor_returns_none_when_no_mrz_present() {
        let ocr = FakeOcr(vec!["Họ và tên".to_string(), "NGUYEN VAN A".to_string()]);
        let img = image::DynamicImage::new_luma8(10, 10);
        assert!(MrzExtractor::new(ocr, 2026).try_extract(&img).is_none());
    }
}
