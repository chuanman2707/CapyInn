use super::{get_f64, AppState};
use crate::models::*;
use crate::services::settings_store::get_setting;
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ═══════════════════════════════════════════════
// Invoice PDF Commands
// ═══════════════════════════════════════════════

const DEFAULT_POLICY_TEXT: &str = "• Check-in: 14:00 | Check-out: 12:00\n• Cancel 24h+ before: full deposit refund\n• Cancel within 24h: 50% deposit retained\n• No refund for no-show";

pub async fn do_generate_invoice(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<InvoiceData, String> {
    // Always regenerate: delete any existing invoice for this booking
    sqlx::query("DELETE FROM invoices WHERE booking_id = ?")
        .bind(booking_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    // Fetch booking
    let b = sqlx::query(
        "SELECT b.id, b.room_id, b.primary_guest_id, b.check_in_at, b.expected_checkout,
                b.nights, b.total_price, b.paid_amount, b.status, b.notes,
                b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout,
                b.pricing_snapshot, b.pricing_type,
                r.name as room_name, r.type as room_type, r.base_price,
                g.full_name as guest_name, g.phone as guest_phone
         FROM bookings b
         JOIN rooms r ON r.id = b.room_id
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.id = ?",
    )
    .bind(booking_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let b = b.ok_or_else(|| format!("Booking '{}' not found", booking_id))?;

    // Get hotel info from settings (stored as JSON blob under "hotel_info")
    let (hotel_name, hotel_address, hotel_phone) = match get_setting(pool, "hotel_info").await? {
        Some(json_str) => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) {
                (
                    v.get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or(crate::app_identity::APP_NAME)
                        .to_string(),
                    v.get("address")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    v.get("phone")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                (
                    crate::app_identity::APP_NAME.to_string(),
                    String::new(),
                    String::new(),
                )
            }
        }
        None => (
            crate::app_identity::APP_NAME.to_string(),
            String::new(),
            String::new(),
        ),
    };

    let room_name: String = b.get("room_name");
    let room_type: String = b.get("room_type");
    let guest_name: String = b.get("guest_name");
    let guest_phone: Option<String> = b.get("guest_phone");
    let nights: i32 = b.get("nights");
    let total_price: f64 = get_f64(&b, "total_price");
    let deposit_amount: f64 = b.try_get::<f64, _>("deposit_amount").unwrap_or(0.0);
    let notes: Option<String> = b.get("notes");

    let check_in: String = b
        .try_get::<String, _>("scheduled_checkin")
        .ok()
        .or_else(|| Some(b.get::<String, _>("check_in_at")))
        .unwrap();
    let check_out: String = b
        .try_get::<String, _>("scheduled_checkout")
        .ok()
        .or_else(|| Some(b.get::<String, _>("expected_checkout")))
        .unwrap();

    // Build pricing breakdown — always use fresh English labels
    let per_night = if nights > 0 {
        total_price / nights as f64
    } else {
        total_price
    };
    let breakdown: Vec<crate::pricing::PricingLine> = vec![crate::pricing::PricingLine {
        label: format!("{} night(s) x {}d", nights, per_night as i64),
        amount: total_price,
    }];

    let subtotal = total_price;
    let balance_due = total_price - deposit_amount;

    // Generate invoice number: INV-YYYYMMDD-XXX
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix = format!("INV-{}", today);
    let max_row: (Option<String>,) =
        sqlx::query_as("SELECT MAX(invoice_number) FROM invoices WHERE invoice_number LIKE ?")
            .bind(format!("{}-%", prefix))
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let next_seq = match max_row.0 {
        Some(ref last) => {
            last.rsplit('-')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0)
                + 1
        }
        None => 1,
    };
    let invoice_number = format!("{}-{:03}", prefix, next_seq);

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();
    let breakdown_json = serde_json::to_string(&breakdown).unwrap_or_default();

    sqlx::query(
        "INSERT INTO invoices (id, invoice_number, booking_id, hotel_name, hotel_address, hotel_phone,
         guest_name, guest_phone, room_name, room_type, check_in, check_out, nights,
         pricing_breakdown, subtotal, deposit_amount, total, balance_due, policy_text, notes,
         status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'issued', ?)"
    )
    .bind(&id).bind(&invoice_number).bind(booking_id)
    .bind(&hotel_name).bind(&hotel_address).bind(&hotel_phone)
    .bind(&guest_name).bind(&guest_phone)
    .bind(&room_name).bind(&room_type)
    .bind(&check_in).bind(&check_out).bind(nights)
    .bind(&breakdown_json)
    .bind(subtotal).bind(deposit_amount).bind(total_price).bind(balance_due)
    .bind(DEFAULT_POLICY_TEXT).bind(&notes)
    .bind(&now)
    .execute(pool).await.map_err(|e| e.to_string())?;

    Ok(InvoiceData {
        id,
        invoice_number,
        booking_id: booking_id.to_string(),
        hotel_name,
        hotel_address,
        hotel_phone,
        guest_name,
        guest_phone,
        room_name,
        room_type,
        check_in,
        check_out,
        nights,
        pricing_breakdown: breakdown,
        subtotal,
        deposit_amount,
        total: total_price,
        balance_due,
        policy_text: Some(DEFAULT_POLICY_TEXT.to_string()),
        notes,
        status: "issued".to_string(),
        created_at: now,
    })
}

pub async fn do_get_invoice(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<Option<InvoiceData>, String> {
    let row = sqlx::query(
        "SELECT id, invoice_number, booking_id, hotel_name, hotel_address, hotel_phone,
                guest_name, guest_phone, room_name, room_type, check_in, check_out, nights,
                pricing_breakdown, subtotal, deposit_amount, total, balance_due,
                policy_text, notes, status, created_at
         FROM invoices WHERE booking_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(booking_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let breakdown_json: String = r.get("pricing_breakdown");
            let breakdown: Vec<crate::pricing::PricingLine> =
                serde_json::from_str(&breakdown_json).unwrap_or_default();

            Ok(Some(InvoiceData {
                id: r.get("id"),
                invoice_number: r.get("invoice_number"),
                booking_id: r.get("booking_id"),
                hotel_name: r.get("hotel_name"),
                hotel_address: r.get("hotel_address"),
                hotel_phone: r.get("hotel_phone"),
                guest_name: r.get("guest_name"),
                guest_phone: r.get("guest_phone"),
                room_name: r.get("room_name"),
                room_type: r.get("room_type"),
                check_in: r.get("check_in"),
                check_out: r.get("check_out"),
                nights: r.get("nights"),
                pricing_breakdown: breakdown,
                subtotal: get_f64(&r, "subtotal"),
                deposit_amount: get_f64(&r, "deposit_amount"),
                total: get_f64(&r, "total"),
                balance_due: get_f64(&r, "balance_due"),
                policy_text: r.get("policy_text"),
                notes: r.get("notes"),
                status: r.get("status"),
                created_at: r.get("created_at"),
            }))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn generate_invoice(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<InvoiceData, String> {
    do_generate_invoice(&state.db, &booking_id).await
}

#[tauri::command]
pub async fn get_invoice(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<Option<InvoiceData>, String> {
    do_get_invoice(&state.db, &booking_id).await
}

#[cfg(test)]
mod tests {
    use super::{do_generate_invoice, do_get_invoice};
    use crate::app_identity;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to open sqlite test pool");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn seed_invoice_booking(pool: &Pool<Sqlite>, booking_id: &str) {
        let room_id = "room-1";
        let guest_id = "guest-1";
        let now = "2026-04-19T10:00:00+07:00";

        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests,
                extra_person_fee, status
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind("Room 1")
        .bind("standard")
        .bind(1_i32)
        .bind(0_i32)
        .bind(250_000.0_f64)
        .bind(2_i32)
        .bind(0.0_f64)
        .bind("vacant")
        .execute(pool)
        .await
        .expect("seed room");

        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, phone, created_at
            ) VALUES (?, 'domestic', ?, ?, ?, ?)",
        )
        .bind(guest_id)
        .bind("Guest 1")
        .bind("DOC-1")
        .bind("0901234567")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed guest");

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, created_by, booking_type, pricing_type,
                deposit_amount, guest_phone, scheduled_checkin, scheduled_checkout,
                pricing_snapshot, created_at
            ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, 'active', ?, ?, NULL, 'walk-in', 'nightly', NULL, ?, NULL, NULL, NULL, ?)",
        )
        .bind(booking_id)
        .bind(room_id)
        .bind(guest_id)
        .bind(now)
        .bind("2026-04-20T10:00:00+07:00")
        .bind(1_i64)
        .bind(250_000.0_f64)
        .bind(0.0_f64)
        .bind("walk-in")
        .bind("seed booking")
        .bind("0901234567")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed booking");
    }

    #[tokio::test]
    async fn generate_invoice_falls_back_when_hotel_info_is_missing() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-1").await;

        let invoice = do_generate_invoice(&pool, "booking-1")
            .await
            .expect("invoice should be generated");

        assert_eq!(invoice.hotel_name, app_identity::APP_NAME);
        assert_eq!(invoice.hotel_address, "");
        assert_eq!(invoice.hotel_phone, "");
    }

    #[tokio::test]
    async fn generate_invoice_falls_back_when_hotel_info_is_malformed() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-1").await;
        sqlx::query("INSERT INTO settings (key, value) VALUES ('hotel_info', '{not-json')")
            .execute(&pool)
            .await
            .expect("seed malformed setting");

        let invoice = do_generate_invoice(&pool, "booking-1")
            .await
            .expect("invoice should be generated");

        assert_eq!(invoice.hotel_name, app_identity::APP_NAME);
        assert_eq!(invoice.hotel_address, "");
        assert_eq!(invoice.hotel_phone, "");
    }

    #[tokio::test]
    async fn get_invoice_returns_none_when_no_invoice_exists() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-2").await;

        let invoice = do_get_invoice(&pool, "booking-2")
            .await
            .expect("lookup should succeed");

        assert!(invoice.is_none());
    }

    #[tokio::test]
    async fn get_invoice_returns_existing_invoice() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-3").await;
        let generated = do_generate_invoice(&pool, "booking-3")
            .await
            .expect("invoice should be generated");

        let invoice = do_get_invoice(&pool, "booking-3")
            .await
            .expect("lookup should succeed")
            .expect("invoice should exist");

        assert_eq!(invoice.invoice_number, generated.invoice_number);
        assert_eq!(invoice.booking_id, "booking-3");
    }
}
