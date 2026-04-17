use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_dir = manifest_dir
        .parent()
        .expect("src-tauri should live inside the app directory");
    let models_dir = app_dir.join("models");
    println!("Models dir: {}", models_dir.display());
    println!("Det exists: {}", models_dir.join("PP-OCRv5_mobile_det.mnn").exists());
    println!("Rec exists: {}", models_dir.join("PP-OCRv5_mobile_rec.mnn").exists());
    println!("Keys exists: {}", models_dir.join("ppocr_keys_v5.txt").exists());

    println!("\nCreating OCR engine...");
    let engine = ocr_rs::OcrEngine::new(
        models_dir.join("PP-OCRv5_mobile_det.mnn").to_str().unwrap(),
        models_dir.join("PP-OCRv5_mobile_rec.mnn").to_str().unwrap(),
        models_dir.join("ppocr_keys_v5.txt").to_str().unwrap(),
        None,
    ).expect("Failed to create OCR engine");
    println!("OCR engine ready!");

    let re_doc = regex::Regex::new(r"\b(\d{12})\b").unwrap();
    let re_dob = regex::Regex::new(r"\b(\d{2}/\d{2}/\d{4})\b").unwrap();

    let scans_dir = app_dir.join("Scans");
    if !scans_dir.exists() {
        println!("Scans dir does not exist: {}", scans_dir.display());
        return;
    }

    for entry in std::fs::read_dir(scans_dir).expect("Can't read Scans dir") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "png" || e == "jpg" || e == "jpeg").unwrap_or(false) {
            println!("\n--- OCR on: {} ---", path.display());
            let img = image::open(&path).expect("Failed to open image");

            let results = engine.recognize(&img).expect("OCR failed");

            println!("Found {} text results", results.len());
            let mut lines = Vec::new();
            for result in &results {
                let text = result.text.trim().to_string();
                if !text.is_empty() {
                    println!("  [{:.0}%] {}", result.confidence * 100.0, text);
                    lines.push(text);
                }
            }

            // Parse CCCD
            println!("\n--- CCCD Parse ---");
            let full_text = lines.join("\n");

            if let Some(m) = re_doc.find(&full_text) {
                println!("Số CCCD: {}", m.as_str());
            }

            if let Some(m) = re_dob.find(&full_text) {
                println!("Ngày sinh: {}", m.as_str());
            }

            if full_text.contains("Nam") { println!("Giới tính: Nam"); }
            else if full_text.contains("Nữ") { println!("Giới tính: Nữ"); }
        }
    }
}
