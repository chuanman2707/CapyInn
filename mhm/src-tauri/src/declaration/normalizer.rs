//! Chuẩn hóa ngày, giới tính và tên phòng.
//!
//! Hai nguồn trả cùng một dữ liệu ở hai định dạng khác nhau (G9), nên không map
//! trực tiếp giữa chúng — cả hai đều đi qua đây về dạng nội bộ.

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    Birth,
    Expiry,
}

/// Ngày sinh: luôn trong quá khứ, cửa sổ 100 năm.
pub fn century_dob(yy: u32, today_year: u32) -> u32 {
    if 2000 + yy <= today_year {
        2000 + yy
    } else {
        1900 + yy
    }
}

/// Ngày hết hạn: gần như luôn ở tương lai, cửa sổ [nay-10, nay+90].
///
/// PHẢI tách khỏi `century_dob`. Dùng chung một hàm sẽ biến `351209`
/// (hộ chiếu hết hạn 09/12/2035) thành 09/12/1935 — bug thật đã xảy ra.
pub fn century_expiry(yy: u32, today_year: u32) -> u32 {
    if 2000 + yy >= today_year.saturating_sub(10) {
        2000 + yy
    } else {
        1900 + yy
    }
}

pub fn mrz_date_to_iso(yymmdd: &str, kind: DateKind, today_year: u32) -> Option<String> {
    if yymmdd.len() != 6 || !yymmdd.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let yy: u32 = yymmdd[0..2].parse().ok()?;
    let mm: u32 = yymmdd[2..4].parse().ok()?;
    let dd: u32 = yymmdd[4..6].parse().ok()?;
    let year = match kind {
        DateKind::Birth => century_dob(yy, today_year),
        DateKind::Expiry => century_expiry(yy, today_year),
    };
    NaiveDate::from_ymd_opt(year as i32, mm, dd).map(|d| d.format("%Y-%m-%d").to_string())
}

/// QR CCCD trả `ddMMyyyy` không dấu phân cách.
pub fn qr_date_to_iso(ddmmyyyy: &str) -> Option<String> {
    if ddmmyyyy.len() != 8 || !ddmmyyyy.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let dd: u32 = ddmmyyyy[0..2].parse().ok()?;
    let mm: u32 = ddmmyyyy[2..4].parse().ok()?;
    let yyyy: i32 = ddmmyyyy[4..8].parse().ok()?;
    NaiveDate::from_ymd_opt(yyyy, mm, dd).map(|d| d.format("%Y-%m-%d").to_string())
}

/// ISO nội bộ -> `dd/MM/yyyy` mà cổng cần. Chỉ writer gọi hàm này.
pub fn iso_to_portal(iso: &str) -> Option<String> {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%d/%m/%Y").to_string())
}

pub fn gender_from_qr(s: &str) -> Option<&'static str> {
    match s.trim() {
        "Nữ" | "nữ" => Some("F"),
        "Nam" | "nam" => Some("M"),
        _ => None,
    }
}

pub fn gender_from_mrz(c: char) -> Option<&'static str> {
    match c {
        'F' => Some("F"),
        'M' => Some("M"),
        _ => None,
    }
}

/// `rooms.name` là `Phòng 5B`; cổng nhận `5B`.
pub fn strip_room_prefix(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    for prefix in ["phòng ", "phong "] {
        if lower.starts_with(prefix) {
            let cut = trimmed
                .char_indices()
                .nth(prefix.chars().count())
                .map(|(i, _)| i)
                .unwrap_or(trimmed.len());
            return trimmed[cut..].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// `bookings.check_in_at` là timestamp có offset; `expected_checkout` là date
/// trần. Cùng một bảng, hai định dạng (H3) — hàm này chịu cả hai.
pub fn booking_ts_to_iso_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some((date_part, _)) = raw.split_once('T') {
        return NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            .ok()
            .map(|d| d.format("%Y-%m-%d").to_string());
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// G6 — đây là bug thật đã xảy ra khi soạn spec. Hai hàm PHẢI khác nhau.
    #[test]
    fn century_rules_differ_for_dob_and_expiry() {
        assert_eq!(century_dob(90, 2026), 1990);
        assert_eq!(century_expiry(35, 2026), 2035);
        assert_ne!(century_dob(35, 2026), century_expiry(35, 2026));
        assert_eq!(century_dob(35, 2026), 1935);
    }

    #[test]
    fn mrz_dates_resolve_correctly() {
        assert_eq!(
            mrz_date_to_iso("900308", DateKind::Birth, 2026).as_deref(),
            Some("1990-03-08")
        );
        assert_eq!(
            mrz_date_to_iso("351209", DateKind::Expiry, 2026).as_deref(),
            Some("2035-12-09")
        );
        assert_eq!(mrz_date_to_iso("9003", DateKind::Birth, 2026), None);
        assert_eq!(mrz_date_to_iso("901332", DateKind::Birth, 2026), None);
    }

    #[test]
    fn qr_and_portal_date_formats() {
        assert_eq!(qr_date_to_iso("28071995").as_deref(), Some("1995-07-28"));
        assert_eq!(qr_date_to_iso("2807199"), None);
        assert_eq!(iso_to_portal("1995-07-28").as_deref(), Some("28/07/1995"));
        assert_eq!(iso_to_portal("18/10/1974"), None);
    }

    /// G9 — hai nguồn trả giới tính khác format.
    #[test]
    fn gender_normalises_from_both_sources() {
        assert_eq!(gender_from_qr("Nữ"), Some("F"));
        assert_eq!(gender_from_qr("Nam"), Some("M"));
        assert_eq!(gender_from_qr("khác"), None);
        assert_eq!(gender_from_mrz('F'), Some("F"));
        assert_eq!(gender_from_mrz('M'), Some("M"));
        assert_eq!(gender_from_mrz('<'), None);
    }

    #[test]
    fn room_prefix_stripped_for_portal() {
        assert_eq!(strip_room_prefix("Phòng 5B"), "5B");
        assert_eq!(strip_room_prefix("phòng 102A9"), "102A9");
        assert_eq!(strip_room_prefix("5A"), "5A");
        assert_eq!(strip_room_prefix("  Phòng  3A "), "3A");
    }

    /// H3 — bookings có hai định dạng ngày khác nhau trong cùng một bảng.
    #[test]
    fn booking_timestamps_accept_both_shapes() {
        assert_eq!(
            booking_ts_to_iso_date("2026-07-25T17:45:27.232667+07:00").as_deref(),
            Some("2026-07-25")
        );
        assert_eq!(
            booking_ts_to_iso_date("2026-07-26").as_deref(),
            Some("2026-07-26")
        );
        assert_eq!(booking_ts_to_iso_date(""), None);
    }
}
