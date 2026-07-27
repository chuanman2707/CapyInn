//! The read behind the "copy lưu trú" text block.
//!
//! Two lookups — the booking, then its primary guest — flattened into the exact
//! fields the clipboard text needs. Formatting stays in the command; this module
//! only knows where the values come from.

use sqlx::{Pool, Row, Sqlite};

pub struct StayInfo {
    pub room_id: String,
    pub full_name: String,
    pub doc_number: String,
    pub dob: String,
    pub gender: String,
    pub nationality: String,
    pub address: String,
    pub check_in: String,
    pub checkout: String,
}

/// Errors when the booking or its primary guest is missing: the caller is
/// working from a booking it just displayed, so either absence is a surprise.
pub async fn load_stay_info(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<StayInfo, sqlx::Error> {
    let booking = sqlx::query(
        "SELECT room_id, primary_guest_id, check_in_at, expected_checkout
         FROM bookings WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_one(pool)
    .await?;

    let guest = sqlx::query(
        "SELECT full_name, doc_number, dob, gender, nationality, address
         FROM guests WHERE id = ?",
    )
    .bind(booking.get::<String, _>("primary_guest_id"))
    .fetch_one(pool)
    .await?;

    Ok(StayInfo {
        room_id: booking.get("room_id"),
        full_name: guest.get("full_name"),
        doc_number: guest.get("doc_number"),
        dob: guest.get::<Option<String>, _>("dob").unwrap_or_default(),
        gender: guest.get::<Option<String>, _>("gender").unwrap_or_default(),
        // The overwhelmingly common case, and the form has to say something.
        nationality: guest
            .get::<Option<String>, _>("nationality")
            .unwrap_or_else(|| "Việt Nam".to_string()),
        address: guest
            .get::<Option<String>, _>("address")
            .unwrap_or_default(),
        check_in: booking.get("check_in_at"),
        checkout: booking.get("expected_checkout"),
    })
}
