//! Bọc engine `ocr-rs` sẵn có trong repo sau trait `MrzOcr` — 0 dependency mới.

use super::mrz::MrzOcr;
use ocr_rs::OcrEngine;

pub struct OcrRsMrz {
    engine: OcrEngine,
}

impl OcrRsMrz {
    pub fn new() -> Result<Self, String> {
        Ok(OcrRsMrz {
            engine: crate::ocr::create_engine()?,
        })
    }
}

impl MrzOcr for OcrRsMrz {
    fn recognize_lines(&self, img: &image::DynamicImage) -> Vec<String> {
        match self.engine.recognize(img) {
            Ok(results) => results
                .iter()
                .map(|r| r.text.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
