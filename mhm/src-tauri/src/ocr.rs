use log::info;
use ocr_rs::{Backend, OcrEngine, OcrEngineConfig};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::app_identity;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CccdInfo {
    pub doc_number: String,
    pub full_name: String,
    pub dob: String,
    pub gender: String,
    pub nationality: String,
    pub address: String,
    pub raw_text: Vec<String>,
}

/// Find models directory from multiple candidates
pub fn find_models_dir() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(models_dir) = app_identity::models_dir_opt() {
        candidates.push(models_dir);
    }
    candidates.push(std::env::current_dir().unwrap_or_default().join("models"));
    candidates.push(
        std::env::current_dir()
            .unwrap_or_default()
            .join("..")
            .join("models"),
    );

    candidates
        .iter()
        .find(|p| p.join("PP-OCRv5_mobile_det.mnn").exists())
        .cloned()
        .ok_or_else(|| {
            format!(
                "OCR models not found. Searched: {}",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Create OcrEngine with Metal backend
pub fn create_engine() -> Result<OcrEngine, String> {
    let models_dir = find_models_dir()?;
    info!("Loading OCR models from: {}", models_dir.display());

    let det_path = models_dir.join("PP-OCRv5_mobile_det.mnn");
    let rec_path = models_dir.join("PP-OCRv5_mobile_rec.mnn");
    let keys_path = models_dir.join("ppocr_keys_v5.txt");

    let config = OcrEngineConfig::new().with_backend(Backend::Metal);

    let engine = OcrEngine::new(
        det_path.to_str().unwrap(),
        rec_path.to_str().unwrap(),
        keys_path.to_str().unwrap(),
        Some(config),
    )
    .map_err(|e| format!("Failed to create OCR engine: {}", e))?;

    info!("OCR engine ready (Metal backend)");
    Ok(engine)
}

/// Run OCR on an image file → return all recognized text lines
pub fn ocr_image(engine: &OcrEngine, image_path: &Path) -> Result<Vec<String>, String> {
    let img = image::open(image_path).map_err(|e| format!("Failed to open image: {}", e))?;

    let results = engine
        .recognize(&img)
        .map_err(|e| format!("OCR recognition failed: {}", e))?;

    let lines: Vec<String> = results
        .iter()
        .map(|r| r.text.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    Ok(lines)
}

/// Parse CCCD fields from raw OCR text lines
pub fn parse_cccd(lines: &[String]) -> CccdInfo {
    let full_text = lines.join("\n");

    let doc_number = Regex::new(r"\b(\d{12})\b")
        .ok()
        .and_then(|re| re.find(&full_text).map(|m| m.as_str().to_string()))
        .unwrap_or_default();

    let full_name =
        extract_field_value(lines, &["Họ và tên", "Full name", "Ho va ten"]).unwrap_or_default();

    let dob = Regex::new(r"\b(\d{2}/\d{2}/\d{4})\b")
        .ok()
        .and_then(|re| re.find(&full_text).map(|m| m.as_str().to_string()))
        .unwrap_or_default();

    let gender = if full_text.contains("Nam") || full_text.contains("Male") {
        "Nam".to_string()
    } else if full_text.contains("Nữ") || full_text.contains("Female") {
        "Nữ".to_string()
    } else {
        String::new()
    };

    let nationality = extract_field_value(lines, &["Quốc tịch", "Nationality"])
        .unwrap_or_else(|| "Việt Nam".to_string());

    let address = extract_field_value(
        lines,
        &["Nơi thường trú", "Place of residence", "Noi thuong tru"],
    )
    .unwrap_or_default();

    CccdInfo {
        doc_number,
        full_name,
        dob,
        gender,
        nationality,
        address,
        raw_text: lines.to_vec(),
    }
}

fn extract_field_value(lines: &[String], labels: &[&str]) -> Option<String> {
    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        for label in labels {
            if lower.contains(&label.to_lowercase()) {
                if let Some(pos) = line.find(':') {
                    let val = line[pos + 1..].trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
                if i + 1 < lines.len() {
                    let next = lines[i + 1].trim().to_string();
                    if !next.is_empty() {
                        return Some(next);
                    }
                }
            }
        }
    }
    None
}

/// Thread-safe wrapper for OcrEngine
#[allow(dead_code)]
pub struct OcrEngineWrapper(pub Mutex<OcrEngine>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cccd_happy_path_vietnamese_next_line() {
        let lines = vec![
            "CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM".to_string(),
            "Độc lập - Tự do - Hạnh phúc".to_string(),
            "CĂN CƯỚC CÔNG DÂN".to_string(),
            "Số/No: 012345678901".to_string(),
            "Họ và tên".to_string(),
            "NGUYỄN VĂN A".to_string(),
            "Ngày sinh: 01/01/1990".to_string(),
            "Giới tính: Nam".to_string(),
            "Nơi thường trú".to_string(),
            "123 Đường ABC, Quận XYZ, TP HCM".to_string(),
        ];

        let info = parse_cccd(&lines);

        assert_eq!(info.doc_number, "012345678901");
        assert_eq!(info.full_name, "NGUYỄN VĂN A");
        assert_eq!(info.dob, "01/01/1990");
        assert_eq!(info.gender, "Nam");
        assert_eq!(info.nationality, "Việt Nam"); // default since it's not in the text
        assert_eq!(info.address, "123 Đường ABC, Quận XYZ, TP HCM");
    }

    #[test]
    fn test_parse_cccd_english_labels_same_line() {
        let lines = vec![
            "CITIZEN IDENTITY CARD".to_string(),
            "098765432109".to_string(),
            "Full name: Jane Doe".to_string(),
            "Date of birth: 12/12/1985".to_string(),
            "Sex: Female".to_string(),
            "Nationality: United States".to_string(),
            "Place of residence: 456 Elm St, Anytown".to_string(),
        ];

        let info = parse_cccd(&lines);

        assert_eq!(info.doc_number, "098765432109");
        assert_eq!(info.full_name, "Jane Doe");
        assert_eq!(info.dob, "12/12/1985");
        assert_eq!(info.gender, "Nữ"); // It maps Female to Nữ
        assert_eq!(info.nationality, "United States");
        assert_eq!(info.address, "456 Elm St, Anytown");
    }

    #[test]
    fn test_parse_cccd_empty_and_missing() {
        let lines: Vec<String> = vec![];
        let info = parse_cccd(&lines);

        assert_eq!(info.doc_number, "");
        assert_eq!(info.full_name, "");
        assert_eq!(info.dob, "");
        assert_eq!(info.gender, "");
        assert_eq!(info.nationality, "Việt Nam");
        assert_eq!(info.address, "");
    }

    #[test]
    fn test_parse_cccd_random_text() {
        let lines = vec![
            "Just some random text".to_string(),
            "123456789".to_string(), // only 9 digits, should not be doc number
            "Ho va ten".to_string(),
            "  ".to_string(), // next line is empty, shouldn't panic
            "Maleish".to_string(), // contains Male, should parse as Nam
            "11-22-3333".to_string(), // not matching dob regex
        ];

        let info = parse_cccd(&lines);

        assert_eq!(info.doc_number, "");
        assert_eq!(info.full_name, "");
        assert_eq!(info.dob, "");
        assert_eq!(info.gender, "Nam");
        assert_eq!(info.nationality, "Việt Nam");
        assert_eq!(info.address, "");
    }
}
