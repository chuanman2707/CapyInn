//! Đo extractor trên ảnh giấy tờ thật.
//!
//! Chạy: cargo run --bin kbtt_probe -- <đường-dẫn-ảnh> [thêm ảnh...]
//!       cargo run --bin kbtt_probe -- --check-resources
//!
//! Gate của Bước 1: QR CCCD phải ra đúng 7 trường; MRZ phải đạt >=3/5
//! checksum trên ảnh chụp bình thường.
//!
//! KHÔNG in payload thô và KHÔNG in đường dẫn ảnh đầy đủ — đó là dữ liệu cá
//! nhân (§12.3). Chỉ in tên file và các trường đã parse.

use capyinn_lib::declaration::catalog::Catalog;
use capyinn_lib::declaration::extractor::{
    mrz::MrzExtractor, ocr_rs_mrz::OcrRsMrz, qr_cccd::QrCccdExtractor, IdentityExtractor,
};
use capyinn_lib::declaration::find_kbtt_resource;

/// Xác nhận một bản đã đóng gói thật sự tìm được resource của nó.
///
/// Đây là thứ mà "test xanh + build thành công" KHÔNG chứng minh được: unit
/// test chạy trong thư mục source nơi file vô tình có sẵn, còn app mở từ
/// Finder có thư mục làm việc là `/`. Chạy lệnh này từ trong bản .app mới biết.
fn check_resources() -> i32 {
    let mut failed = false;

    for name in ["kbtt_catalog.json", "tblt_vn_import.xlsx"] {
        match find_kbtt_resource(name) {
            Ok(path) => println!("OK   {name} -> {}", path.display()),
            Err(e) => {
                println!("LỖI  {name}: {e}");
                failed = true;
            }
        }
    }

    match Catalog::load() {
        Ok(c) => println!(
            "OK   catalog nạp được: {} quốc tịch, {} tỉnh, {} phường (nguồn {})",
            c.quoc_tich.len(),
            c.tinh_thanh.len(),
            c.phuong_xa.len(),
            c.source_date
        ),
        Err(e) => {
            println!("LỖI  không nạp được catalog: {e}");
            failed = true;
        }
    }

    if failed {
        println!("\nKHÔNG ĐẠT — bản build này sẽ hỏng khi khai báo.");
        1
    } else {
        println!("\nĐẠT — bản build tìm được đủ resource.");
        0
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Dùng: kbtt_probe <ảnh> [ảnh...]");
        eprintln!("      kbtt_probe --check-resources");
        std::process::exit(2);
    }

    if args[0] == "--check-resources" {
        std::process::exit(check_resources());
    }

    let today_year = 2026;
    // Engine OCR nặng: khởi tạo MỘT lần rồi dùng lại cho mọi ảnh.
    let mrz_extractor = match OcrRsMrz::new() {
        Ok(ocr) => Some(MrzExtractor::new(ocr, today_year)),
        Err(e) => {
            eprintln!("Không khởi tạo được OCR: {e}");
            None
        }
    };

    let mut qr_ok = 0;
    let mut mrz_ok = 0;
    let mut total = 0;

    for path in &args {
        total += 1;
        let name = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "?".into());

        let img = match image::open(path) {
            Ok(i) => i,
            Err(e) => {
                println!("{name}: KHÔNG MỞ ĐƯỢC ({e})");
                continue;
            }
        };

        if let Some(res) = QrCccdExtractor.try_extract(&img) {
            qr_ok += 1;
            let id = res.identity;
            println!(
                "{name}: QR_CCCD ok | tên={} | sinh={} | giới={} | loại={:?} ({:?})",
                id.full_name, id.dob, id.gender, id.doc_type_code, id.doc_type_source
            );
            continue;
        }

        if let Some(extractor) = &mrz_extractor {
            if let Some(res) = extractor.try_extract(&img) {
                mrz_ok += 1;
                let id = res.identity;
                println!(
                    "{name}: MRZ_TD3 {:?} | tên={} | HC={:?} | quốc tịch={} | sinh={} | HC hết hạn={:?} | tên đã xác nhận={}",
                    res.confidence,
                    id.full_name,
                    id.passport_no,
                    id.nationality_iso3,
                    id.dob,
                    id.passport_expiry,
                    id.name_confirmed_by_human
                );
                continue;
            }
        }

        println!("{name}: KHÔNG TRÍCH ĐƯỢC — cần nhập tay");
    }

    println!("\n--- Tổng kết ---");
    println!("Ảnh: {total} | QR ok: {qr_ok} | MRZ ok: {mrz_ok}");
    println!("Gate Bước 1: QR ra đúng 7 trường, MRZ >=3/5 checksum.");
    if qr_ok + mrz_ok == 0 {
        println!("CHƯA ĐẠT GATE — nếu MRZ trượt hết, cân nhắc bật feature mrz-tesseract.");
    }
}
