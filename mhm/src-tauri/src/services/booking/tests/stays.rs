use super::prelude::*;

#[tokio::test]
async fn calendar_insert_conflict_returns_room_unavailable_without_overwrite() {
    let pool = test_pool().await;
    seed_room(&pool, "CAL-1").await.unwrap();
    seed_booked_reservation(&pool, "existing-booking", "CAL-1")
        .await
        .unwrap();
    seed_active_booking(&pool, "new-booking", "CAL-1")
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let error = crate::services::booking::support::insert_room_calendar_rows(
        &mut tx,
        "CAL-1",
        "new-booking",
        NaiveDate::from_ymd_opt(2026, 4, 20).unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 21).unwrap(),
        crate::models::status::calendar::BOOKED,
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE));
}

#[tokio::test]
async fn check_in_posts_charge_and_marks_room_occupied() {
    let pool = test_pool().await;
    seed_room(&pool, "R201").await.unwrap();

    let booking = stay_lifecycle::check_in(
        &pool,
        minimal_checkin_request("R201"),
        Some("user-1".to_string()),
    )
    .await
    .unwrap();

    assert_room_status(&pool, "R201", "occupied").await;

    let charge = sqlx::query(
        "SELECT type, amount FROM transactions WHERE booking_id = ? AND type = 'charge' LIMIT 1",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge.get::<String, _>("type"), "charge");
    assert_eq!(charge.get::<i64, _>("amount"), booking.total_price);

    assert_calendar_rows(&pool, &booking.id, "occupied", 2).await;
}

#[tokio::test]
async fn stay_lifecycle_smoke_covers_checkin_extend_and_checkout() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R-SMOKE-STAY", 250_000)
        .await
        .expect("seed stay room/pricing");

    let check_in_ctx = cmd_with_request(
        "check_in",
        "req-smoke-stay-checkin",
        "idem-smoke-stay-checkin",
    );
    let check_in_req = paid_checkin_req("R-SMOKE-STAY", 50_000);

    let checked_in = stay_lifecycle::check_in_idempotent(
        &pool,
        &check_in_ctx,
        check_in_req,
        Some("user-smoke".to_string()),
    )
    .await
    .expect("stay check-in succeeds");
    let booking_id = checked_in.response["id"]
        .as_str()
        .expect("checked-in booking id")
        .to_string();
    let initial_expected_checkout = checked_in.response["expected_checkout"]
        .as_str()
        .expect("initial expected checkout")
        .to_string();

    assert_eq!(checked_in.response["status"], serde_json::json!("active"));
    assert_booking_status(&pool, &booking_id, "active").await;
    assert_room_status(&pool, "R-SMOKE-STAY", "occupied").await;
    assert_calendar_rows(&pool, &booking_id, "occupied", 2).await;
    assert_eq!(
        transaction_sum(&pool, &booking_id, "charge", None).await,
        500_000
    );
    assert_eq!(
        transaction_sum(
            &pool,
            &booking_id,
            "payment",
            Some("Thanh toán khi check-in")
        )
        .await,
        50_000
    );
    assert_single_outbox_event(&pool, &check_in_ctx, "booking.checked_in").await;

    let extend_ctx = cmd_with_request(
        "extend_stay",
        "req-smoke-stay-extend",
        "idem-smoke-stay-extend",
    );
    let extended = stay_lifecycle::extend_stay_idempotent(&pool, &extend_ctx, &booking_id)
        .await
        .expect("stay extend succeeds");
    let extended_expected_checkout = extended.response["expected_checkout"]
        .as_str()
        .expect("extended expected checkout");
    let initial_checkout = chrono::DateTime::parse_from_rfc3339(&initial_expected_checkout)
        .expect("initial checkout parses");
    let extended_checkout = chrono::DateTime::parse_from_rfc3339(extended_expected_checkout)
        .expect("extended checkout parses");

    assert_eq!(extended.response["nights"], serde_json::json!(3));
    assert_eq!(extended.response["total_price"], serde_json::json!(750_000));
    assert_eq!(extended_checkout, initial_checkout + Duration::days(1));
    assert_calendar_rows(&pool, &booking_id, "occupied", 3).await;
    assert_eq!(
        transaction_sum(&pool, &booking_id, "charge", Some("Extended stay +1 night")).await,
        250_000
    );
    assert_single_outbox_event(&pool, &extend_ctx, "booking.stay_extended").await;

    let check_out_ctx = cmd_with_request(
        "check_out",
        "req-smoke-stay-checkout",
        "idem-smoke-stay-checkout",
    );
    let checked_out = stay_lifecycle::check_out_idempotent(
        &pool,
        &check_out_ctx,
        checkout_req(&booking_id, CheckoutSettlementMode::BookedNights, 750_000),
    )
    .await
    .expect("stay checkout succeeds");

    assert_eq!(checked_out.response["ok"], serde_json::json!(true));
    assert_booking_status(&pool, &booking_id, "checked_out").await;
    assert_room_status(&pool, "R-SMOKE-STAY", "vacant").await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind(&booking_id)
            .fetch_one(&pool)
            .await
            .expect("calendar rows removed after checkout"),
        0
    );
    assert_eq!(
        transaction_sum(
            &pool,
            &booking_id,
            "payment",
            Some("Thanh toán khi check-out")
        )
        .await,
        700_000
    );
    assert_eq!(booking_paid_amount(&pool, &booking_id).await, Some(750_000));
    assert_single_outbox_event(&pool, &check_out_ctx, "booking.checked_out").await;
}

#[tokio::test]
async fn check_in_idempotent_retry_replays_and_does_not_duplicate_rows() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R-CHECKIN-IDEM", 250_000)
        .await
        .unwrap();
    let ctx = cmd_with_request("check_in", "req-checkin-idem", "idem-checkin-1");
    let first_req = paid_checkin_req("R-CHECKIN-IDEM", 50_000);
    let second_req = paid_checkin_req("R-CHECKIN-IDEM", 50_000);

    let first =
        stay_lifecycle::check_in_idempotent(&pool, &ctx, first_req, Some("user-1".to_string()))
            .await
            .unwrap();
    let second =
        stay_lifecycle::check_in_idempotent(&pool, &ctx, second_req, Some("user-1".to_string()))
            .await
            .unwrap();

    assert_replayed_pair(&first, &second);

    let booking_id = first.response["id"].as_str().unwrap();
    assert_eq!(booking_count_for_room(&pool, "R-CHECKIN-IDEM").await, 1);
    assert_eq!(booking_guest_count_for_booking(&pool, booking_id).await, 1);
    assert_eq!(transaction_count_for_booking(&pool, booking_id).await, 2);

    assert_eq!(booking_paid_amount(&pool, booking_id).await, Some(50_000));

    assert_eq!(calendar_count_for_booking(&pool, booking_id).await, 2);
    assert_single_outbox_event(&pool, &ctx, "booking.checked_in").await;
}

#[tokio::test]
async fn two_check_in_commands_for_same_room_leave_one_booking_and_consistent_calendar() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CHECKIN-RACE").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000).await.unwrap();
    let first_ctx = cmd_with_request("check_in", "req-checkin-race-1", "idem-checkin-race-1");
    let second_ctx = cmd_with_request("check_in", "req-checkin-race-2", "idem-checkin-race-2");

    let (first, second) = tokio::join!(
        stay_lifecycle::check_in_idempotent(
            &pool,
            &first_ctx,
            minimal_checkin_request("R-CHECKIN-RACE"),
            Some("user-1".to_string())
        ),
        stay_lifecycle::check_in_idempotent(
            &pool,
            &second_ctx,
            minimal_checkin_request("R-CHECKIN-RACE"),
            Some("user-2".to_string())
        )
    );

    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "exactly one concurrent check-in should succeed"
    );
    assert_eq!(
        usize::from(first.is_err()) + usize::from(second.is_err()),
        1,
        "exactly one concurrent check-in should fail"
    );

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
        .bind("R-CHECKIN-RACE")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking_count, 1);

    assert_room_status(&pool, "R-CHECKIN-RACE", "occupied").await;

    let occupied_calendar_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND status = 'occupied'",
    )
    .bind("R-CHECKIN-RACE")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(occupied_calendar_count, 2);
}

#[tokio::test]
async fn check_in_idempotent_same_key_changed_guest_conflicts() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R-CHECKIN-HASH", 250_000)
        .await
        .unwrap();
    let ctx = cmd_with_request("check_in", "req-checkin-hash", "idem-checkin-hash");

    stay_lifecycle::check_in_idempotent(
        &pool,
        &ctx,
        minimal_checkin_request("R-CHECKIN-HASH"),
        Some("user-1".to_string()),
    )
    .await
    .unwrap();

    let mut changed = minimal_checkin_request("R-CHECKIN-HASH");
    changed.guests[0].full_name = "Nguyen Van Changed".to_string();
    let error =
        stay_lifecycle::check_in_idempotent(&pool, &ctx, changed, Some("user-1".to_string()))
            .await
            .expect_err("same key with changed guest conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn check_in_idempotent_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CHECKIN-LIVE").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000).await.unwrap();
    let ctx = cmd_with_request("check_in", "req-checkin-live", "idem-checkin-live");
    let payload = serde_json::json!({
        "schema": "stay.check_in.v1",
        "room_id": "R-CHECKIN-LIVE",
        "guests": [{
            "guest_type": "domestic",
            "full_name": "Nguyen Van A",
            "doc_number": "079123456789",
            "dob": null,
            "gender": null,
            "nationality": "VN",
            "address": null,
            "visa_expiry": null,
            "scan_path": null,
            "phone": "0900000000",
        }],
        "nights": 2,
        "source": "walk-in",
        "notes": "test check-in",
        "paid_amount": 0,
        "pricing_type": "nightly",
        "rate_override_per_night": null,
    });
    seed_live_in_progress_command(&pool, &ctx.command_name, &ctx.idempotency_key, &payload).await;

    let error = stay_lifecycle::check_in_idempotent(
        &pool,
        &ctx,
        minimal_checkin_request("R-CHECKIN-LIVE"),
        Some("user-1".to_string()),
    )
    .await
    .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn check_in_fails_when_second_pool_blocks_room_calendar_first() {
    let (pool_a, pool_b, db_path) = shared_file_test_pools("second-pool-calendar").await;
    seed_room(&pool_a, "R-2POOL-CALENDAR").await.unwrap();
    seed_pricing_rule(&pool_a, "standard", 250_000)
        .await
        .unwrap();

    let today = Local::now().date_naive().to_string();
    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, NULL, 'occupied')",
    )
    .bind("R-2POOL-CALENDAR")
    .bind(today)
    .execute(&pool_b)
    .await
    .unwrap();

    let ctx = cmd_with_request(
        "check_in",
        "req-checkin-2pool-calendar",
        "idem-checkin-2pool-calendar",
    );
    let error = stay_lifecycle::check_in_idempotent(
        &pool_a,
        &ctx,
        minimal_checkin_request("R-2POOL-CALENDAR"),
        Some("user-1".to_string()),
    )
    .await
    .expect_err("check-in should reject calendar row inserted by second pool");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE
    );

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
        .bind("R-2POOL-CALENDAR")
        .fetch_one(&pool_a)
        .await
        .unwrap();
    assert_eq!(booking_count, 0);

    pool_a.close().await;
    pool_b.close().await;
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn check_out_idempotent_retry_replays_without_duplicate_money_or_housekeeping() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CHECKOUT-IDEM").await.unwrap();
    seed_active_booking(&pool, "B-CHECKOUT-IDEM", "R-CHECKOUT-IDEM")
        .await
        .unwrap();
    let ctx = cmd_with_request("check_out", "req-checkout-idem", "idem-checkout-1");
    let replay_checkout = || {
        checkout_req(
            "B-CHECKOUT-IDEM",
            CheckoutSettlementMode::BookedNights,
            1_000_000,
        )
    };

    let first = stay_lifecycle::check_out_idempotent(&pool, &ctx, replay_checkout())
        .await
        .unwrap();
    let second = stay_lifecycle::check_out_idempotent(&pool, &ctx, replay_checkout())
        .await
        .unwrap();

    assert_replayed_pair(&first, &second);

    let housekeeping_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE room_id = ?")
            .bind("R-CHECKOUT-IDEM")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        housekeeping_count, 0,
        "trả phòng không được sinh phiếu dọn nào nữa, kể cả khi replay"
    );

    let checkout_money_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND note IN (
             'Điều chỉnh tăng tiền phòng khi quyết toán check-out',
             'Điều chỉnh giảm tiền phòng khi quyết toán check-out',
             'Thanh toán khi check-out'
         )",
    )
    .bind("B-CHECKOUT-IDEM")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(checkout_money_count, 2);
    assert_single_outbox_event(&pool, &ctx, "booking.checked_out").await;
}

#[tokio::test]
async fn check_out_idempotent_same_key_changed_total_conflicts() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CHECKOUT-HASH").await.unwrap();
    seed_active_booking(&pool, "B-CHECKOUT-HASH", "R-CHECKOUT-HASH")
        .await
        .unwrap();
    let ctx = cmd_with_request("check_out", "req-checkout-hash", "idem-checkout-hash");
    let first_req = CheckOutRequest {
        booking_id: "B-CHECKOUT-HASH".to_string(),
        settlement_mode: CheckoutSettlementMode::BookedNights,
        final_total: 1_000_000,
    };
    let second_req = CheckOutRequest {
        booking_id: "B-CHECKOUT-HASH".to_string(),
        settlement_mode: CheckoutSettlementMode::BookedNights,
        final_total: 1_100_000,
    };

    stay_lifecycle::check_out_idempotent(&pool, &ctx, first_req)
        .await
        .unwrap();
    let error = stay_lifecycle::check_out_idempotent(&pool, &ctx, second_req)
        .await
        .unwrap_err();

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn check_out_idempotent_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CHECKOUT-LIVE").await.unwrap();
    seed_active_booking(&pool, "B-CHECKOUT-LIVE", "R-CHECKOUT-LIVE")
        .await
        .unwrap();
    let ctx = cmd_with_request("check_out", "req-checkout-live", "idem-checkout-live");
    let payload = serde_json::json!({
        "schema": "stay.check_out.v1",
        "booking_id": "B-CHECKOUT-LIVE",
        "settlement_mode": "booked_nights",
        "final_total": 1_000_000,
    });
    seed_live_in_progress_command(&pool, &ctx.command_name, &ctx.idempotency_key, &payload).await;

    let error = stay_lifecycle::check_out_idempotent(
        &pool,
        &ctx,
        checkout_req(
            "B-CHECKOUT-LIVE",
            CheckoutSettlementMode::BookedNights,
            1_000_000,
        ),
    )
    .await
    .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn check_in_rolls_back_when_room_status_changes_before_guarded_room_update() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-CHECKIN").await.unwrap();
    seed_pricing_rule(&pool, "standard", 100_000).await.unwrap();

    sqlx::query(
        "CREATE TRIGGER occupy_room_after_booking_insert
         AFTER INSERT ON bookings
         WHEN NEW.room_id = 'R-CAS-CHECKIN'
         BEGIN
           UPDATE rooms SET status = 'occupied' WHERE id = NEW.room_id;
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = stay_lifecycle::check_in(&pool, minimal_checkin_request("R-CAS-CHECKIN"), None)
        .await
        .expect_err("guarded room update should catch stale state");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
        .bind("R-CAS-CHECKIN")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking_count, 0);
}

#[tokio::test]
async fn checkout_fails_when_second_pool_checked_out_booking_first() {
    let (pool_a, pool_b, db_path) = shared_file_test_pools("second-pool-checkout").await;
    seed_room(&pool_a, "R-2POOL").await.unwrap();
    seed_active_booking(&pool_a, "B-2POOL", "R-2POOL")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind("B-2POOL")
        .execute(&pool_b)
        .await
        .unwrap();

    let error = stay_lifecycle::check_out(
        &pool_a,
        checkout_req("B-2POOL", CheckoutSettlementMode::BookedNights, 100_000),
    )
    .await
    .expect_err("checkout should reject stale booking state");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));

    pool_a.close().await;
    pool_b.close().await;
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn a_room_can_be_checked_in_again_immediately_after_checkout() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R-TURNOVER", 250_000)
        .await
        .expect("seed turnover room/pricing");

    let check_in_ctx = cmd_with_request("check_in", "req-turnover-in-1", "idem-turnover-in-1");
    let first = stay_lifecycle::check_in_idempotent(
        &pool,
        &check_in_ctx,
        paid_checkin_req("R-TURNOVER", 0),
        Some("user-turnover".to_string()),
    )
    .await
    .expect("first guest checks in");
    let first_id = first.response["id"]
        .as_str()
        .expect("first booking id")
        .to_string();

    let check_out_ctx = cmd_with_request("check_out", "req-turnover-out", "idem-turnover-out");
    stay_lifecycle::check_out_idempotent(
        &pool,
        &check_out_ctx,
        checkout_req(&first_id, CheckoutSettlementMode::BookedNights, 500_000),
    )
    .await
    .expect("first guest checks out");

    // Đây là toàn bộ lý do tồn tại của thay đổi này: không có bước trung gian
    // nào giữa trả phòng và nhận phòng. Trước 09/08/2026 phòng rơi vào
    // `cleaning` và lời gọi dưới đây trả về Conflict "Phòng R-TURNOVER không
    // trống (status: cleaning)".
    assert_room_status(&pool, "R-TURNOVER", "vacant").await;

    let second_ctx = cmd_with_request("check_in", "req-turnover-in-2", "idem-turnover-in-2");
    let second = stay_lifecycle::check_in_idempotent(
        &pool,
        &second_ctx,
        paid_checkin_req("R-TURNOVER", 0),
        Some("user-turnover".to_string()),
    )
    .await
    .expect("second guest checks in with no housekeeping step in between");

    assert_eq!(second.response["status"], serde_json::json!("active"));
    assert_room_status(&pool, "R-TURNOVER", "occupied").await;

    let housekeeping_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE room_id = 'R-TURNOVER'")
            .fetch_one(&pool)
            .await
            .expect("counts housekeeping rows");
    assert_eq!(
        housekeeping_rows, 0,
        "trả phòng không được sinh phiếu dọn nào nữa"
    );
}
