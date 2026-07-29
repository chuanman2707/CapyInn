use super::prelude::*;

#[tokio::test]
async fn extend_stay_prices_the_extra_night_with_the_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R320", 500_000, 2, 50_000)
        .await
        .unwrap();
    let booking_id = uuid::Uuid::new_v4().to_string();
    seed_active_booking(&pool, &booking_id, "R320")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET guests = 4, total_price = 1200000, nights = 2 WHERE id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .unwrap();

    let booking = stay_lifecycle::extend_stay(&pool, &booking_id)
        .await
        .unwrap();

    // 1.200.000₫ cũ + 600.000₫ cho đêm thêm, không phải + 500.000₫.
    assert_eq!(booking.total_price, 1_800_000);
}

#[tokio::test]
async fn extend_stay_uses_existing_expected_checkout() {
    let pool = test_pool().await;
    seed_room(&pool, "R203").await.unwrap();
    seed_active_booking(&pool, "B203", "R203").await.unwrap();

    let booking = stay_lifecycle::extend_stay(&pool, "B203").await.unwrap();

    assert_eq!(booking.nights, 2);
    assert_eq!(booking.expected_checkout, "2026-04-17T10:00:00+07:00");
    assert_eq!(booking.total_price, 500_000);

    let extended_day =
        sqlx::query("SELECT status FROM room_calendar WHERE room_id = ? AND date = '2026-04-16'")
            .bind("R203")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(extended_day.get::<String, _>("status"), "occupied");

    let charge = sqlx::query(
        "SELECT amount FROM transactions WHERE booking_id = ? AND note = 'Extended stay +1 night'",
    )
    .bind("B203")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge.get::<i64, _>("amount"), 250_000);
}

#[tokio::test]
async fn extend_stay_idempotent_retry_replays_without_extra_night_or_charge() {
    let pool = test_pool().await;
    seed_room(&pool, "R-EXT-IDEM").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B-EXT-IDEM",
        "R-EXT-IDEM",
        "2026-04-15T10:00:00+07:00",
        "2026-04-17T10:00:00+07:00",
        2,
        500_000,
        Some(0),
    )
    .await
    .unwrap();
    let ctx = cmd_with_request("extend_stay", "req-extend-idem", "idem-extend-1");

    let first = stay_lifecycle::extend_stay_idempotent(&pool, &ctx, "B-EXT-IDEM")
        .await
        .unwrap();
    let second = stay_lifecycle::extend_stay_idempotent(&pool, &ctx, "B-EXT-IDEM")
        .await
        .unwrap();

    assert_replayed_pair(&first, &second);

    let nights: i32 = sqlx::query_scalar("SELECT nights FROM bookings WHERE id = ?")
        .bind("B-EXT-IDEM")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(nights, 3);

    assert_eq!(
        transaction_count_for_note(&pool, "B-EXT-IDEM", "Extended stay +1 night").await,
        1
    );
    assert_single_outbox_event(&pool, &ctx, "booking.stay_extended").await;
}

#[tokio::test]
async fn extend_stay_idempotent_same_key_different_booking_conflicts() {
    let pool = test_pool().await;
    seed_room(&pool, "R-EXT-HASH-A").await.unwrap();
    seed_room(&pool, "R-EXT-HASH-B").await.unwrap();
    seed_active_booking(&pool, "B-EXT-HASH-A", "R-EXT-HASH-A")
        .await
        .unwrap();
    seed_active_booking(&pool, "B-EXT-HASH-B", "R-EXT-HASH-B")
        .await
        .unwrap();
    let ctx = cmd_with_request("extend_stay", "req-extend-hash", "idem-extend-hash");

    stay_lifecycle::extend_stay_idempotent(&pool, &ctx, "B-EXT-HASH-A")
        .await
        .unwrap();
    let error = stay_lifecycle::extend_stay_idempotent(&pool, &ctx, "B-EXT-HASH-B")
        .await
        .unwrap_err();

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn extend_stay_idempotent_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_room(&pool, "R-EXT-LIVE").await.unwrap();
    seed_active_booking(&pool, "B-EXT-LIVE", "R-EXT-LIVE")
        .await
        .unwrap();
    let ctx = cmd_with_request("extend_stay", "req-extend-live", "idem-extend-live");
    let payload = serde_json::json!({
        "schema": "stay.extend.v1",
        "booking_id": "B-EXT-LIVE",
        "operation": "add_one_night",
    });
    seed_live_in_progress_command(&pool, &ctx.command_name, &ctx.idempotency_key, &payload).await;

    let error = stay_lifecycle::extend_stay_idempotent(&pool, &ctx, "B-EXT-LIVE")
        .await
        .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn extend_stay_fails_when_second_pool_checked_out_booking_first() {
    let (pool_a, pool_b, db_path) =
        shared_file_test_pools("second-pool-extend-after-checkout").await;
    seed_room(&pool_a, "R-2POOL-EXTEND").await.unwrap();
    seed_active_booking(&pool_a, "B-2POOL-EXTEND", "R-2POOL-EXTEND")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind("B-2POOL-EXTEND")
        .execute(&pool_b)
        .await
        .unwrap();

    let ctx = cmd_with_request(
        "extend_stay",
        "req-extend-2pool-checkout",
        "idem-extend-2pool-checkout",
    );
    let error = stay_lifecycle::extend_stay_idempotent(&pool_a, &ctx, "B-2POOL-EXTEND")
        .await
        .expect_err("extend stay should reject booking checked out by second pool");

    assert_eq!(error.code, crate::app_error::codes::BOOKING_INVALID_STATE);

    let charge_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND note = ?")
            .bind("B-2POOL-EXTEND")
            .bind("Extended stay +1 night")
            .fetch_one(&pool_a)
            .await
            .unwrap();
    assert_eq!(charge_count, 0);

    pool_a.close().await;
    pool_b.close().await;
    let _ = std::fs::remove_file(db_path);
}
