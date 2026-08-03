use super::prelude::*;

fn checkout_dt(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> chrono::DateTime<Local> {
    chrono::Local
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .unwrap()
}

async fn preview_checkout_at(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    booking_id: &str,
    settlement_mode: CheckoutSettlementMode,
    now: chrono::DateTime<Local>,
) -> crate::domain::booking::BookingResult<crate::models::CheckoutSettlementPreview> {
    stay_lifecycle::preview_checkout_settlement_at(
        pool,
        CheckoutSettlementPreviewRequest {
            booking_id: booking_id.to_string(),
            settlement_mode,
        },
        now,
    )
    .await
}

async fn check_out_booking_at(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    booking_id: &str,
    settlement_mode: CheckoutSettlementMode,
    final_total: i64,
    now: chrono::DateTime<Local>,
) -> crate::domain::booking::BookingResult<()> {
    stay_lifecycle::check_out_at(
        pool,
        checkout_req(booking_id, settlement_mode, final_total),
        now,
    )
    .await
}

#[tokio::test]
async fn actual_nights_settlement_keeps_the_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R330", 500_000, 2, 50_000)
        .await
        .unwrap();
    // Zero out the weekend uplift the fallback pricing rule would otherwise
    // apply — without this, the assertion below is only correct on days whose
    // check-in date (today minus 2) does not land on a Sat/Sun.
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();
    let booking_id = uuid::Uuid::new_v4().to_string();
    seed_active_booking(&pool, &booking_id, "R330")
        .await
        .unwrap();

    let check_in = Local::now() - Duration::days(2);
    sqlx::query(
        "UPDATE bookings SET guests = 4, nights = 3, total_price = 1800000, check_in_at = ? WHERE id = ?",
    )
    .bind(check_in.to_rfc3339())
    .bind(&booking_id)
    .execute(&pool)
    .await
    .unwrap();

    let settlement = stay_lifecycle::preview_checkout_settlement(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: booking_id.clone(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
    )
    .await
    .unwrap();

    // Ở 2 đêm thật × 600.000₫, không phải × 500.000₫.
    assert_eq!(settlement.recommended_total, 1_200_000);
}

#[tokio::test]
async fn check_out_settles_same_day_actual_nights_to_minimum_one_night() {
    let pool = test_pool().await;
    seed_room(&pool, "R410").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B410",
        "R410",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

    let preview = preview_checkout_at(
        &pool,
        "B410",
        CheckoutSettlementMode::ActualNights,
        checkout_dt(2026, 4, 20, 18, 0),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 1);
    assert_eq!(preview.recommended_total, 500_000);

    check_out_booking_at(
        &pool,
        "B410",
        CheckoutSettlementMode::ActualNights,
        preview.recommended_total,
        checkout_dt(2026, 4, 20, 18, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT nights, total_price, paid_amount, pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind("B410")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
    assert_eq!(booking.get::<i64, _>("total_price"), 500_000);
    assert_eq!(booking.get::<i64, _>("paid_amount"), 500_000);
    assert!(booking
        .get::<Option<String>, _>("pricing_snapshot")
        .unwrap()
        .contains("\"reporting_checkout\""));
}

#[tokio::test]
async fn check_out_keeps_active_booking_values_for_booked_nights_mode() {
    let pool = test_pool().await;
    seed_room(&pool, "R411").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B411",
        "R411",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B411", 1_000_000, "prior payment")
        .await
        .unwrap();

    let preview = preview_checkout_at(
        &pool,
        "B411",
        CheckoutSettlementMode::BookedNights,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 5);
    assert_eq!(preview.recommended_total, 2_500_000);

    check_out_booking_at(
        &pool,
        "B411",
        CheckoutSettlementMode::BookedNights,
        preview.recommended_total,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights, total_price, paid_amount FROM bookings WHERE id = ?")
        .bind("B411")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 5);
    assert_eq!(booking.get::<i64, _>("total_price"), 2_500_000);
    assert_eq!(booking.get::<i64, _>("paid_amount"), 2_500_000);
}

#[tokio::test]
async fn check_out_booked_nights_enforces_minimum_one_night_for_corrupted_booking() {
    let pool = test_pool().await;
    seed_room(&pool, "R413").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B413",
        "R413",
        "2026-04-20T08:00:00+07:00",
        "2026-04-20T12:00:00+07:00",
        0,
        0,
        Some(0),
    )
    .await
    .unwrap();

    let preview = preview_checkout_at(
        &pool,
        "B413",
        CheckoutSettlementMode::BookedNights,
        checkout_dt(2026, 4, 20, 12, 0),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 1);

    check_out_booking_at(
        &pool,
        "B413",
        CheckoutSettlementMode::BookedNights,
        0,
        checkout_dt(2026, 4, 20, 12, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights FROM bookings WHERE id = ?")
        .bind("B413")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
}

#[tokio::test]
async fn check_out_actual_nights_uses_early_checkout_nights() {
    let pool = test_pool().await;
    seed_room(&pool, "R414").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B414",
        "R414",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

    let preview = preview_checkout_at(
        &pool,
        "B414",
        CheckoutSettlementMode::ActualNights,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 2);
    assert_eq!(preview.recommended_total, 1_000_000);

    check_out_booking_at(
        &pool,
        "B414",
        CheckoutSettlementMode::ActualNights,
        preview.recommended_total,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B414")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 2);
    assert_eq!(booking.get::<i64, _>("total_price"), 1_000_000);
}

#[tokio::test]
async fn check_out_hourly_persists_manual_settlement() {
    let pool = test_pool().await;
    seed_room(&pool, "R415").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B415",
        "R415",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

    check_out_booking_at(
        &pool,
        "B415",
        CheckoutSettlementMode::Hourly,
        500_000,
        checkout_dt(2026, 4, 20, 10, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT nights, total_price, paid_amount, pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind("B415")
    .fetch_one(&pool)
    .await
    .unwrap();

    let pricing_snapshot = booking
        .get::<Option<String>, _>("pricing_snapshot")
        .unwrap();
    let pricing_snapshot: serde_json::Value = serde_json::from_str(&pricing_snapshot).unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
    assert_eq!(booking.get::<i64, _>("total_price"), 500_000);
    assert_eq!(booking.get::<i64, _>("paid_amount"), 500_000);
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["mode"],
        serde_json::json!("hourly")
    );
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["settled_total"],
        serde_json::json!(500_000)
    );
}

#[tokio::test]
async fn check_out_hourly_multi_day_stay_still_persists_one_night() {
    let pool = test_pool().await;
    seed_room(&pool, "R419").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B419",
        "R419",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

    check_out_booking_at(
        &pool,
        "B419",
        CheckoutSettlementMode::Hourly,
        500_000,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B419")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
    assert_eq!(booking.get::<i64, _>("total_price"), 500_000);
}

#[tokio::test]
async fn check_out_persists_manual_override_when_final_total_differs_from_recommendation() {
    let pool = test_pool().await;
    seed_room(&pool, "R416").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B416",
        "R416",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B416", 300_000, "prior payment")
        .await
        .unwrap();

    let preview = preview_checkout_at(
        &pool,
        "B416",
        CheckoutSettlementMode::ActualNights,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    assert_eq!(preview.recommended_total, 1_000_000);

    check_out_booking_at(
        &pool,
        "B416",
        CheckoutSettlementMode::ActualNights,
        800_000,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT total_price, paid_amount, pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind("B416")
    .fetch_one(&pool)
    .await
    .unwrap();

    let pricing_snapshot = booking
        .get::<Option<String>, _>("pricing_snapshot")
        .unwrap();
    let pricing_snapshot: serde_json::Value = serde_json::from_str(&pricing_snapshot).unwrap();

    assert_eq!(booking.get::<i64, _>("total_price"), 800_000);
    assert_eq!(booking.get::<i64, _>("paid_amount"), 800_000);
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["manual_override"],
        serde_json::json!(true)
    );
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["settled_total"],
        serde_json::json!(800_000)
    );
}

#[tokio::test]
async fn check_out_writes_charge_adjustment_ledger_when_settled_total_drops() {
    let pool = test_pool().await;
    seed_room(&pool, "R417").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B417",
        "R417",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B417", 800_000, "prior payment")
        .await
        .unwrap();

    check_out_booking_at(
        &pool,
        "B417",
        CheckoutSettlementMode::ActualNights,
        800_000,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    let adjustment = sqlx::query(
        "SELECT amount, note FROM transactions
         WHERE booking_id = ? AND type = 'charge' AND note LIKE 'Điều chỉnh %'
         LIMIT 1",
    )
    .bind("B417")
    .fetch_one(&pool)
    .await
    .unwrap();

    let payment_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND type = 'payment' AND note = 'Thanh toán khi check-out'",
    )
    .bind("B417")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(adjustment.get::<i64, _>("amount"), -1_700_000);
    assert_eq!(
        adjustment.get::<String, _>("note"),
        "Điều chỉnh giảm tiền phòng khi quyết toán check-out"
    );
    assert_eq!(payment_count.0, 0);
}

#[tokio::test]
async fn check_out_writes_payment_delta_ledger_when_collecting_extra_payment() {
    let pool = test_pool().await;
    seed_room(&pool, "R418").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B418",
        "R418",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B418", 1_000_000, "prior payment")
        .await
        .unwrap();

    check_out_booking_at(
        &pool,
        "B418",
        CheckoutSettlementMode::BookedNights,
        2_500_000,
        checkout_dt(2026, 4, 22, 9, 0),
    )
    .await
    .unwrap();

    let payment = sqlx::query(
        "SELECT amount, note FROM transactions
         WHERE booking_id = ? AND type = 'payment' AND note = 'Thanh toán khi check-out'
         LIMIT 1",
    )
    .bind("B418")
    .fetch_one(&pool)
    .await
    .unwrap();

    let charge_adjustment_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND type = 'charge' AND note LIKE 'Điều chỉnh %'",
    )
    .bind("B418")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payment.get::<i64, _>("amount"), 1_500_000);
    assert_eq!(payment.get::<String, _>("note"), "Thanh toán khi check-out");
    assert_eq!(booking_paid_amount(&pool, "B418").await, Some(2_500_000));
    assert_eq!(charge_adjustment_count.0, 0);
}

#[tokio::test]
async fn checkout_paid_amount_is_ledger_projection_not_direct_overwrite() {
    let pool = test_pool().await;
    seed_room(&pool, "R420").await.unwrap();
    seed_pricing_rule(&pool, "standard", 75_000).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B420",
        "R420",
        "2026-04-20T08:00:00+07:00",
        "2026-04-21T12:00:00+07:00",
        1,
        75_000,
        Some(0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B420", 75_000, "prior payment")
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER forbid_paid_amount_direct_update_in_checkout_test
         BEFORE UPDATE OF paid_amount ON bookings
         BEGIN
             SELECT RAISE(ABORT, 'paid_amount direct update forbidden in checkout test');
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    check_out_booking_at(
        &pool,
        "B420",
        CheckoutSettlementMode::ActualNights,
        75_000,
        checkout_dt(2026, 4, 21, 9, 0),
    )
    .await
    .unwrap();

    let ledger_total: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(amount), 0) AS INTEGER)
         FROM transactions
         WHERE booking_id = ? AND type IN ('payment', 'deposit')",
    )
    .bind("B420")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(ledger_total, 75_000);
    assert_eq!(booking_paid_amount(&pool, "B420").await, Some(ledger_total));
}

#[tokio::test]
async fn check_out_rejects_overpaid_booking_until_refund_flow_exists() {
    let pool = test_pool().await;
    seed_room(&pool, "R412").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B412",
        "R412",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B412", 700_000, "prior payment")
        .await
        .unwrap();

    let error = check_out_booking_at(
        &pool,
        "B412",
        CheckoutSettlementMode::Hourly,
        500_000,
        checkout_dt(2026, 4, 20, 12, 0),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("refund"));
}

#[tokio::test]
async fn check_out_idempotent_reads_the_room_after_taking_the_booking_lock() {
    let pool = test_pool().await;
    seed_room(&pool, "R911").await.unwrap();
    seed_room(&pool, "R912").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B911",
        "R911",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

    let blocker = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::booking_key("B911").unwrap()])
        .await
        .expect("blocker lock");

    let ctx = cmd("check_out", "idem-checkout-race");
    let pool_for_task = pool.clone();
    let command = tokio::spawn(async move {
        stay_lifecycle::check_out_idempotent(
            &pool_for_task,
            &ctx,
            checkout_req("B911", CheckoutSettlementMode::ActualNights, 2_500_000),
        )
        .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !command.is_finished(),
        "lệnh phải kẹt ở pha 1, chưa được phép đọc phòng"
    );

    sqlx::query("UPDATE room_calendar SET room_id = 'R912' WHERE booking_id = 'B911'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET room_id = 'R912' WHERE id = 'B911'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R912'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R911'")
        .execute(&pool)
        .await
        .unwrap();

    drop(blocker);

    let result = command
        .await
        .expect("task joins")
        .expect("check out phải thành công trên phòng mới");

    assert_eq!(result.response["room_id"], "R912");
    assert_room_status(&pool, "R912", "cleaning").await;
}
