use super::AppState;
use crate::{
    app_error::{codes, normalize_correlation_id, CommandError, CommandResult},
    command_idempotency::{system_error, WriteCommandContext},
    models::*,
    services::booking::invoice_generation,
};
use sqlx::{Pool, Sqlite};
use tauri::State;

// ═══════════════════════════════════════════════
// Invoice PDF Commands
// ═══════════════════════════════════════════════

#[cfg(test)]
pub async fn do_generate_invoice(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<InvoiceData, String> {
    invoice_generation::generate_invoice_direct(pool, booking_id).await
}

pub async fn do_get_invoice(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<Option<InvoiceData>, String> {
    invoice_generation::get_invoice(pool, booking_id).await
}

#[derive(serde::Deserialize)]
struct InvoiceDataWire {
    id: String,
    invoice_number: String,
    booking_id: String,
    hotel_name: String,
    hotel_address: String,
    hotel_phone: String,
    guest_name: String,
    guest_phone: Option<String>,
    room_name: String,
    room_type: String,
    check_in: String,
    check_out: String,
    nights: i32,
    pricing_breakdown: Vec<crate::pricing::PricingLine>,
    subtotal: crate::money::MoneyVnd,
    deposit_amount: crate::money::MoneyVnd,
    total: crate::money::MoneyVnd,
    balance_due: crate::money::MoneyVnd,
    policy_text: Option<String>,
    notes: Option<String>,
    status: String,
    created_at: String,
}

impl From<InvoiceDataWire> for InvoiceData {
    fn from(value: InvoiceDataWire) -> Self {
        Self {
            id: value.id,
            invoice_number: value.invoice_number,
            booking_id: value.booking_id,
            hotel_name: value.hotel_name,
            hotel_address: value.hotel_address,
            hotel_phone: value.hotel_phone,
            guest_name: value.guest_name,
            guest_phone: value.guest_phone,
            room_name: value.room_name,
            room_type: value.room_type,
            check_in: value.check_in,
            check_out: value.check_out,
            nights: value.nights,
            pricing_breakdown: value.pricing_breakdown,
            subtotal: value.subtotal,
            deposit_amount: value.deposit_amount,
            total: value.total,
            balance_due: value.balance_due,
            policy_text: value.policy_text,
            notes: value.notes,
            status: value.status,
            created_at: value.created_at,
        }
    }
}

#[tauri::command]
pub async fn generate_invoice(
    state: State<'_, AppState>,
    booking_id: String,
    idempotency_key: String,
    correlation_id: Option<String>,
) -> CommandResult<InvoiceData> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let actor_id = super::get_user_id(&state).ok_or_else(|| {
        CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập")
            .with_request_id(effective_correlation_id.value.clone())
    })?;
    let mut ctx = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "generate_invoice",
    )?;
    ctx.actor_id = Some(actor_id);

    let response = invoice_generation::generate_invoice_idempotent(&state.db, &ctx, &booking_id)
        .await
        .map_err(|error| error.with_request_id(ctx.request_id.clone()))?
        .response;
    serde_json::from_value::<InvoiceDataWire>(response)
        .map(Into::into)
        .map_err(|error| system_error(error).with_request_id(ctx.request_id.clone()))
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
    use crate::app_error::codes;
    use crate::app_identity;
    use crate::command_idempotency::WriteCommandContext;
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
        let room_id = format!("room-{booking_id}");
        let guest_id = format!("guest-{booking_id}");
        let now = "2026-04-19T10:00:00+07:00";

        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests,
                extra_person_fee, status
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&room_id)
        .bind("Room 1")
        .bind("standard")
        .bind(1_i32)
        .bind(0_i32)
        .bind(250_000)
        .bind(2_i32)
        .bind(0)
        .bind("vacant")
        .execute(pool)
        .await
        .expect("seed room");

        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, phone, created_at
            ) VALUES (?, 'domestic', ?, ?, ?, ?)",
        )
        .bind(&guest_id)
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
        .bind(&room_id)
        .bind(&guest_id)
        .bind(now)
        .bind("2026-04-20T10:00:00+07:00")
        .bind(1_i64)
        .bind(250_000)
        .bind(0)
        .bind("walk-in")
        .bind("seed booking")
        .bind("0901234567")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed booking");
    }

    #[tokio::test]
    async fn generate_invoice_idempotent_retry_replays_without_duplicate_rows() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-invoice-idem").await;
        let ctx = WriteCommandContext::for_internal_test(
            "req-invoice-idem",
            "idem-invoice-1",
            "generate_invoice",
        );

        let first = crate::services::booking::invoice_generation::generate_invoice_idempotent(
            &pool,
            &ctx,
            "booking-invoice-idem",
        )
        .await
        .expect("first invoice succeeds");
        let second = crate::services::booking::invoice_generation::generate_invoice_idempotent(
            &pool,
            &ctx,
            "booking-invoice-idem",
        )
        .await
        .expect("retry replays");

        assert!(!first.replayed);
        assert!(second.replayed);
        assert!(first.response["id"].is_string());
        assert!(second.response["id"].is_string());
        assert!(first.response["invoice_number"].is_string());
        assert!(second.response["invoice_number"].is_string());
        assert_eq!(first.response["id"], second.response["id"]);
        assert_eq!(
            first.response["invoice_number"],
            second.response["invoice_number"]
        );

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE booking_id = ?")
            .bind("booking-invoice-idem")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn generate_invoice_idempotent_same_key_different_booking_conflicts() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-invoice-a").await;
        seed_invoice_booking(&pool, "booking-invoice-b").await;
        let ctx = WriteCommandContext::for_internal_test(
            "req-invoice-hash",
            "idem-invoice-hash",
            "generate_invoice",
        );

        crate::services::booking::invoice_generation::generate_invoice_idempotent(
            &pool,
            &ctx,
            "booking-invoice-a",
        )
        .await
        .expect("first invoice succeeds");
        let error = crate::services::booking::invoice_generation::generate_invoice_idempotent(
            &pool,
            &ctx,
            "booking-invoice-b",
        )
        .await
        .expect_err("same key with different booking conflicts");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);

        let count_b: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE booking_id = ?")
            .bind("booking-invoice-b")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count_b, 0);
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
    async fn generate_invoice_prefers_actual_checkout_and_settlement_label() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-settled").await;

        sqlx::query(
            "UPDATE bookings
             SET status = 'checked_out',
                 actual_checkout = '2026-04-20T18:00:00+07:00',
                 nights = 1,
                 total_price = 500000,
                 paid_amount = 500000,
                 pricing_snapshot = ?
             WHERE id = ?",
        )
        .bind(r#"{"checkout_settlement":{"label":"Thanh toán theo số đêm thực tế"}}"#)
        .bind("booking-settled")
        .execute(&pool)
        .await
        .unwrap();

        let invoice = do_generate_invoice(&pool, "booking-settled").await.unwrap();

        assert_eq!(invoice.check_out, "2026-04-20T18:00:00+07:00");
        assert_eq!(invoice.total, 500_000);
        assert_eq!(invoice.balance_due, 0);
        assert_eq!(
            invoice.pricing_breakdown[0].label,
            "Thanh toán theo số đêm thực tế"
        );
    }

    #[tokio::test]
    async fn generate_invoice_booked_nights_uses_billed_checkout_boundary() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-booked-nights").await;

        sqlx::query(
            "UPDATE bookings
             SET status = 'checked_out',
                 expected_checkout = '2026-04-26T10:00:00+07:00',
                 actual_checkout = '2026-04-22T09:00:00+07:00',
                 nights = 5,
                 total_price = 2500000,
                 paid_amount = 2500000,
                 pricing_snapshot = ?
             WHERE id = ?",
        )
        .bind(
            r#"{"checkout_settlement":{"mode":"booked_nights","label":"Thanh toán theo số đêm đã đặt","reporting_checkout":"2026-04-25"}}"#,
        )
        .bind("booking-booked-nights")
        .execute(&pool)
        .await
        .unwrap();

        let invoice = do_generate_invoice(&pool, "booking-booked-nights")
            .await
            .unwrap();

        assert_eq!(invoice.check_out, "2026-04-25");
        assert_eq!(
            invoice.pricing_breakdown[0].label,
            "Thanh toán theo số đêm đã đặt"
        );
    }

    #[tokio::test]
    async fn generate_invoice_uses_paid_amount_not_deposit_amount_for_balance_due() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-balance").await;

        sqlx::query(
            "UPDATE bookings
             SET total_price = 500000, paid_amount = 500000, deposit_amount = 100000
             WHERE id = ?",
        )
        .bind("booking-balance")
        .execute(&pool)
        .await
        .unwrap();

        let invoice = do_generate_invoice(&pool, "booking-balance").await.unwrap();

        assert_eq!(invoice.balance_due, 0);
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
