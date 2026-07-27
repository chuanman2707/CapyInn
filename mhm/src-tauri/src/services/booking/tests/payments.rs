use super::prelude::*;

#[tokio::test]
async fn record_payment_updates_paid_amount_cache() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B101", "R101")
        .await
        .unwrap();

    record_payment(&pool, "B101", 25_000, "deposit")
        .await
        .unwrap();

    assert_eq!(booking_paid_amount(&pool, "B101").await, Some(25_000));

    let txn = sqlx::query("SELECT type, amount, note FROM transactions WHERE booking_id = ?")
        .bind("B101")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "payment");
    assert_eq!(txn.get::<i64, _>("amount"), 25_000);
    assert_eq!(txn.get::<String, _>("note"), "deposit");
}

#[tokio::test]
async fn record_payment_returning_id_tx_returns_inserted_transaction_id() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-PAY-ID", "R-PAY-ID")
        .await
        .unwrap();

    let mut tx = crate::services::booking::support::begin_tx(&pool).await.unwrap();
    let transaction_id =
        record_payment_returning_id_tx(&mut tx, "B-PAY-ID", 25_000, "payment id test")
            .await
            .unwrap();
    tx.commit().await.unwrap();

    let stored: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE id = ? AND booking_id = ? AND amount = ?",
    )
    .bind(&transaction_id)
    .bind("B-PAY-ID")
    .bind(25_000_i64)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(stored.0, 1);

    let txn = sqlx::query("SELECT type, note FROM transactions WHERE id = ?")
        .bind(&transaction_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "payment");
    assert_eq!(txn.get::<String, _>("note"), "payment id test");

    assert_eq!(booking_paid_amount(&pool, "B-PAY-ID").await, Some(25_000));
}

#[tokio::test]
async fn record_payment_idempotent_retry_replays_and_does_not_double_post() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-PAY-IDEM", "R-PAY-IDEM")
        .await
        .unwrap();
    let ctx = cmd_with_request("record_payment", "req-payment-idem", "idem-payment-1");

    let first = record_payment_idempotent(&pool, &ctx, "B-PAY-IDEM", 125_000, "Payment retry test")
        .await
        .unwrap();
    let second =
        record_payment_idempotent(&pool, &ctx, "B-PAY-IDEM", 125_000, "Payment retry test")
            .await
            .unwrap();

    assert_replayed_pair(&first, &second);

    assert_eq!(
        transaction_count_for_booking_type(&pool, "B-PAY-IDEM", "payment").await,
        1
    );

    assert_eq!(
        booking_paid_amount(&pool, "B-PAY-IDEM").await,
        Some(125_000)
    );

    let payload = assert_single_outbox_event(&pool, &ctx, "folio.payment_recorded").await;
    assert_eq!(payload["aggregate"]["type"], "folio");
    assert_eq!(payload["aggregate"]["id"], "B-PAY-IDEM");
    assert_eq!(payload["refresh"], serde_json::json!(["folio", "bookings"]));
}

#[tokio::test]
async fn record_payment_idempotent_same_key_different_amount_conflicts() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-PAY-HASH", "R-PAY-HASH")
        .await
        .unwrap();
    let ctx = cmd("record_payment", "idem-payment-hash");

    record_payment_idempotent(&pool, &ctx, "B-PAY-HASH", 50_000, "first")
        .await
        .unwrap();
    let error = record_payment_idempotent(&pool, &ctx, "B-PAY-HASH", 60_000, "first")
        .await
        .unwrap_err();

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn record_payment_idempotent_distinct_keys_sum_paid_amount() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-PAY-SUM", "R-PAY-SUM")
        .await
        .unwrap();
    let first_ctx = cmd("record_payment", "idem-payment-sum-1");
    let second_ctx = cmd("record_payment", "idem-payment-sum-2");

    record_payment_idempotent(&pool, &first_ctx, "B-PAY-SUM", 40_000, "first")
        .await
        .unwrap();
    record_payment_idempotent(&pool, &second_ctx, "B-PAY-SUM", 60_000, "second")
        .await
        .unwrap();

    assert_eq!(booking_paid_amount(&pool, "B-PAY-SUM").await, Some(100_000));
}

#[tokio::test]
async fn record_payment_idempotent_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-PAY-LIVE", "R-PAY-LIVE")
        .await
        .unwrap();
    let ctx = cmd("record_payment", "idem-payment-live");
    let payload = serde_json::json!({
        "schema": "payment.record.v1",
        "booking_id": "B-PAY-LIVE",
        "amount": 75_000,
        "note": "live payment",
    });
    seed_live_in_progress_command(&pool, &ctx.command_name, &ctx.idempotency_key, &payload).await;

    let error = record_payment_idempotent(&pool, &ctx, "B-PAY-LIVE", 75_000, "live payment")
        .await
        .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn record_payment_tx_can_compose_inside_outer_transaction() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B102", "R102")
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_payment_tx(&mut tx, "B102", 12_500, "deposit")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B102")
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<i64>, _>("paid_amount"), Some(12_500));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn record_deposit_tx_updates_paid_amount_cache() {
    let pool = test_pool().await;
    seed_room(&pool, "R103").await.unwrap();
    seed_booked_reservation(&pool, "B103", "R103")
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_deposit_tx(&mut tx, "B103", 25_000, "extra deposit")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(booking_paid_amount(&pool, "B103").await, Some(75_000));

    let txn = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND note = ?",
    )
    .bind("B103")
    .bind("extra deposit")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "deposit");
    assert_eq!(txn.get::<i64, _>("amount"), 25_000);
    assert_eq!(txn.get::<String, _>("note"), "extra deposit");
}

#[tokio::test]
async fn record_deposit_with_origin_writes_origin_key_and_ordinal() {
    let pool = test_pool().await;
    seed_room(&pool, "R501").await.unwrap();
    let booking_id = seed_booking_for_origin_tests(&pool, "R501").await.unwrap();
    let origin = OriginSideEffect::new("idem-deposit-1", 0).unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_deposit_with_origin_tx(&mut tx, &booking_id, 25_000, "origin deposit", &origin)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let row = sqlx::query(
        "SELECT origin_idempotency_key, origin_transaction_ordinal
         FROM transactions
         WHERE booking_id = ? AND note = ?",
    )
    .bind(&booking_id)
    .bind("origin deposit")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        row.get::<String, _>("origin_idempotency_key"),
        "idem-deposit-1"
    );
    assert_eq!(row.get::<i64, _>("origin_transaction_ordinal"), 0);
}

#[tokio::test]
async fn record_deposit_with_origin_rejects_blank_key_before_write() {
    let pool = test_pool().await;
    seed_room(&pool, "R502").await.unwrap();
    let booking_id = seed_booking_for_origin_tests(&pool, "R502").await.unwrap();

    let err = OriginSideEffect::new(" ", 0).expect_err("blank key should be rejected");
    assert!(err
        .to_string()
        .contains("Origin idempotency key is required"));

    assert_eq!(transaction_count_for_booking(&pool, &booking_id).await, 0);
}

#[tokio::test]
async fn duplicate_transaction_origin_is_blocked_by_unique_origin_ordinal() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-TXN-ORIGIN-DUP", "R-TXN-ORIGIN-DUP")
        .await
        .unwrap();
    let origin = OriginSideEffect::new("origin-duplicate-transaction", 0).unwrap();

    let mut first_tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    record_payment_with_origin_tx(
        &mut first_tx,
        "B-TXN-ORIGIN-DUP",
        25_000,
        "Duplicate origin payment",
        &origin,
    )
    .await
    .unwrap();
    first_tx.commit().await.unwrap();

    let mut second_tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    let duplicate = record_payment_with_origin_tx(
        &mut second_tx,
        "B-TXN-ORIGIN-DUP",
        25_000,
        "Duplicate origin payment",
        &origin,
    )
    .await;
    assert!(duplicate.is_err(), "duplicate transaction origin must fail");
    second_tx.rollback().await.unwrap();

    assert_transaction_origin(&pool, "origin-duplicate-transaction", 0, 1).await;
}

#[tokio::test]
async fn record_cancellation_fee_tx_does_not_change_paid_amount() {
    let pool = test_pool().await;
    seed_room(&pool, "R104").await.unwrap();
    seed_booked_reservation(&pool, "B104", "R104")
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_cancellation_fee_tx(&mut tx, "B104", 25_000, "retained deposit")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(booking_paid_amount(&pool, "B104").await, Some(50_000));

    let txn = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND note = ?",
    )
    .bind("B104")
    .bind("retained deposit")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "cancellation_fee");
    assert_eq!(txn.get::<i64, _>("amount"), 25_000);
    assert_eq!(txn.get::<String, _>("note"), "retained deposit");
}
