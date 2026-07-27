use super::prelude::*;

#[tokio::test]
async fn create_reservation_idempotent_retry_does_not_duplicate_deposit() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R601", 600_000)
        .await
        .expect("seeds room/pricing");
    let ctx = cmd_with_request(
        "create_reservation",
        "req-reservation-1",
        "idem-reservation-1",
    );

    let first = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        reservation_req("R601")
            .guest("Retry Guest")
            .doc("DOC601")
            .phone(None)
            .dates("2026-05-01", "2026-05-02")
            .nights(1)
            .deposit(Some(50_000))
            .build(),
    )
    .await
    .expect("first reservation succeeds");
    let second = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        reservation_req("R601")
            .guest("Retry Guest")
            .doc("DOC601")
            .phone(None)
            .dates("2026-05-01", "2026-05-02")
            .nights(1)
            .deposit(Some(50_000))
            .build(),
    )
    .await
    .expect("retry replays");

    assert_eq!(first.response["id"], second.response["id"]);
    assert!(!first.replayed);
    assert!(second.replayed);
    assert_single_outbox_event(&pool, &ctx, "booking.reservation_created").await;

    assert_eq!(
        origin_transaction_count(&pool, "create_reservation:idem-reservation-1").await,
        1
    );
}

#[tokio::test]
async fn create_reservation_idempotent_replay_returns_stored_booking_snapshot() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R604", 600_000)
        .await
        .expect("seeds room/pricing");
    let ctx = cmd("create_reservation", "idem-reservation-snapshot");
    let first = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        reservation_req("R604")
            .guest("Snapshot Guest")
            .doc("DOC604")
            .phone(None)
            .dates("2026-05-01", "2026-05-02")
            .nights(1)
            .deposit(Some(50_000))
            .build(),
    )
    .await
    .expect("first reservation succeeds");
    let booking_id = first.response["id"]
        .as_str()
        .expect("id in first response")
        .to_string();
    let first_status = first.response["status"]
        .as_str()
        .expect("status in first response")
        .to_string();
    assert_eq!(first_status, "booked");

    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .expect("mutates booking status");

    let replay = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        reservation_req("R604")
            .guest("Snapshot Guest")
            .doc("DOC604")
            .phone(None)
            .dates("2026-05-01", "2026-05-02")
            .nights(1)
            .deposit(Some(50_000))
            .build(),
    )
    .await
    .expect("replay succeeds");

    assert!(replay.replayed);
    assert_eq!(
        replay.response["status"].as_str(),
        Some(first_status.as_str())
    );
    assert_ne!(replay.response["status"].as_str(), Some("cancelled"));
}

#[tokio::test]
async fn create_reservation_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R602", "R603"], 600_000)
        .await
        .expect("seeds rooms/pricing");
    let ctx = cmd("create_reservation", "idem-reservation-conflict");

    reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        reservation_req("R602")
            .guest("Conflict Guest")
            .doc("DOC602")
            .phone(None)
            .dates("2026-05-01", "2026-05-02")
            .nights(1)
            .deposit(Some(50_000))
            .build(),
    )
    .await
    .expect("first reservation succeeds");

    let error = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        reservation_req("R603")
            .guest("Conflict Guest")
            .doc("DOC602")
            .phone(None)
            .dates("2026-05-01", "2026-05-02")
            .nights(1)
            .deposit(Some(50_000))
            .build(),
    )
    .await
    .expect_err("same key with different payload conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn reservation_command_idempotency_create_hashes_deposit_as_integer_vnd_units() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R690", 600_000)
        .await
        .expect("seeds room/pricing");
    let ctx = cmd("create_reservation", "idem-reservation-deposit-vnd");
    let request = CreateReservationRequest {
        room_id: "R690".to_string(),
        guest_name: "Deposit Units Guest".to_string(),
        guest_doc_number: Some("DOC690".to_string()),
        guest_phone: None,
        check_in_date: "2026-05-01".to_string(),
        check_out_date: "2026-05-02".to_string(),
        nights: 1,
        source: Some("phone".to_string()),
        notes: None,
        deposit_amount: Some(500_000),
    };

    reservation_lifecycle::create_reservation_idempotent(&pool, &ctx, request)
        .await
        .expect("reservation succeeds");

    let row = sqlx::query(
        "SELECT request_hash, intent_json FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_one(&pool)
    .await
    .expect("reads command row");

    let expected_payload = serde_json::json!({
        "schema": "reservation.create.v1",
        "room_id": "R690",
        "guest_name": "Deposit Units Guest",
        "guest_doc_number": "DOC690",
        "guest_phone": null,
        "check_in_date": "2026-05-01",
        "check_out_date": "2026-05-02",
        "nights": 1,
        "source": "phone",
        "notes": null,
        "deposit_vnd_units": 500000,
    });
    let mut cents_payload = expected_payload.clone();
    cents_payload["deposit_vnd_units"] = serde_json::json!(50000000);
    let mut float_payload = expected_payload.clone();
    float_payload["deposit_vnd_units"] = serde_json::json!(500000.0);
    let mut string_payload = expected_payload.clone();
    string_payload["deposit_vnd_units"] = serde_json::json!("500000");

    assert_eq!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&expected_payload)
            .expect("expected payload hashes")
    );
    assert_ne!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&cents_payload)
            .expect("cents payload hashes")
    );
    assert_ne!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&float_payload)
            .expect("float payload hashes")
    );
    assert_ne!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&string_payload)
            .expect("string payload hashes")
    );

    let intent_json = row.get::<String, _>("intent_json");
    assert!(intent_json.contains("\"deposit_present\":true"));
    assert!(intent_json.contains("\"deposit_vnd_units\":500000"));
}

#[tokio::test]
async fn reservation_command_idempotency_rejects_invalid_deposit_before_claim() {
    let pool = test_pool().await;

    for (deposit_amount, idempotency_key) in [
        (-1, "idem-reservation-deposit-negative"),
        (
            crate::money::MAX_TRANSPORT_SAFE_MONEY_VND + 1,
            "idem-reservation-deposit-unsafe",
        ),
    ] {
        let ctx = cmd("create_reservation", idempotency_key);
        let mut request = minimal_reservation_request("R691");
        request.deposit_amount = Some(deposit_amount);

        let error = reservation_lifecycle::create_reservation_idempotent(&pool, &ctx, request)
            .await
            .expect_err("invalid deposit_amount must fail");

        assert_eq!(
            error.code,
            crate::app_error::codes::VALIDATION_INVALID_INPUT
        );
        assert!(
            error.message.contains("deposit_amount"),
            "error should name the invalid field: {error:?}"
        );

        assert_eq!(
            command_claim_count(&pool, &ctx.command_name, &ctx.idempotency_key).await,
            0
        );
    }
}

#[tokio::test]
async fn reservation_command_idempotency_create_replay_does_not_duplicate_booking_or_calendar() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R691", 600_000)
        .await
        .expect("seeds room/pricing");
    let ctx = cmd("reservation.create", "idem-create-replay-no-dup");

    let first = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        minimal_reservation_request("R691"),
    )
    .await
    .expect("first create succeeds");
    let replay = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        minimal_reservation_request("R691"),
    )
    .await
    .expect("create replays");

    assert_replayed_pair(&first, &replay);
    assert_eq!(booking_count_for_room(&pool, "R691").await, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM room_calendar WHERE room_id = ?")
            .bind("R691")
            .fetch_one(&pool)
            .await
            .expect("counts calendar rows"),
        2
    );
}

#[tokio::test]
async fn reservation_command_idempotency_modify_replay_returns_stored_snapshot() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B692", "R692", 600_000)
        .await
        .unwrap();
    let ctx = cmd("reservation.modify", "idem-modify-snapshot");

    let first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B692", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("first modify succeeds");
    sqlx::query("UPDATE bookings SET total_price = 999 WHERE id = ?")
        .bind("B692")
        .execute(&pool)
        .await
        .expect("mutates booking after first response");
    let replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B692", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("modify replays");

    assert_eq!(first.response, replay.response);
    assert!(replay.replayed);
    assert_eq!(replay.response["total_price"], serde_json::json!(1_800_000));
}

#[tokio::test]
async fn reservation_command_idempotency_modify_replay_does_not_duplicate_calendar() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B693", "R693", 600_000)
        .await
        .unwrap();
    let ctx = cmd("reservation.modify", "idem-modify-calendar");
    reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B693", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("first modify succeeds");
    reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B693", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("modify replays");

    assert_eq!(calendar_count_for_booking(&pool, "B693").await, 3);
}

#[tokio::test]
async fn reservation_command_idempotency_cancel_replay_does_not_duplicate_cancellation_fee() {
    let pool = test_pool().await;
    seed_room(&pool, "R694").await.unwrap();
    seed_booked_reservation(&pool, "B694", "R694")
        .await
        .unwrap();
    let ctx = cmd("reservation.cancel", "idem-cancel-fee");

    let first = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B694")
        .await
        .expect("first cancel succeeds");
    let replay = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B694")
        .await
        .expect("cancel replays");

    assert!(replay.replayed);
    assert_eq!(first.response, replay.response);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'cancellation_fee'",
        )
        .bind("B694")
        .fetch_one(&pool)
        .await
        .expect("counts cancellation fees"),
        1
    );
}

#[tokio::test]
async fn reservation_command_idempotency_confirm_replay_does_not_duplicate_room_charge() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B695", "R695", 600_000)
        .await
        .unwrap();
    let ctx = cmd("reservation.confirm", "idem-confirm-charge");

    reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B695")
        .await
        .expect("first confirm succeeds");
    reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B695")
        .await
        .expect("confirm replays");

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
        )
        .bind("B695")
        .fetch_one(&pool)
        .await
        .expect("counts room charges"),
        1
    );
}

#[tokio::test]
async fn reservation_command_idempotency_confirm_replay_does_not_requery_or_reprice_later_retry() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B696", "R696", 600_000)
        .await
        .unwrap();
    let ctx = cmd("reservation.confirm", "idem-confirm-no-reprice");

    let first = reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B696")
        .await
        .expect("first confirm succeeds");
    sqlx::query("UPDATE pricing_rules SET daily_rate = 9999999 WHERE room_type = 'standard'")
        .execute(&pool)
        .await
        .expect("mutates pricing");
    sqlx::query("UPDATE bookings SET total_price = 123 WHERE id = ?")
        .bind("B696")
        .execute(&pool)
        .await
        .expect("mutates booking");
    let replay = reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B696")
        .await
        .expect("confirm replays");

    assert!(replay.replayed);
    assert_eq!(first.response, replay.response);
    assert_ne!(replay.response["total_price"], serde_json::json!(123));
}

#[tokio::test]
async fn reservation_command_idempotency_modify_cancel_confirm_same_key_different_payload_conflicts(
) {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R697A", "R697B"], 600_000)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B697A", "R697A")
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B697B", "R697B")
        .await
        .unwrap();

    let modify_ctx = cmd("reservation.modify", "idem-modify-hash-conflict");
    reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        crate::models::ModifyReservationRequest {
            booking_id: "B697A".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-25".to_string(),
            new_nights: 2,
        },
    )
    .await
    .expect("first modify succeeds");
    let modify_error = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        crate::models::ModifyReservationRequest {
            booking_id: "B697B".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-25".to_string(),
            new_nights: 2,
        },
    )
    .await
    .expect_err("different modify payload conflicts");
    assert_eq!(
        modify_error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );

    let cancel_ctx = cmd("reservation.cancel", "idem-cancel-hash-conflict");
    reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B697A")
        .await
        .expect("first cancel succeeds");
    let cancel_error =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B697B")
            .await
            .expect_err("different cancel payload conflicts");
    assert_eq!(
        cancel_error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );

    let confirm_ctx = cmd("reservation.confirm", "idem-confirm-hash-conflict");
    reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B697B")
        .await
        .expect("first confirm succeeds");
    let confirm_error =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B697A")
            .await
            .expect_err("different confirm payload conflicts");
    assert_eq!(
        confirm_error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn reservation_command_idempotency_modify_conflict_replays_terminal_room_unavailable() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B698", "R698", 600_000)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status)
         VALUES (?, '2026-04-23', NULL, 'booked')",
    )
    .bind("R698")
    .execute(&pool)
    .await
    .expect("seeds conflicting calendar");
    let ctx = cmd("reservation.modify", "idem-modify-room-conflict");
    let first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B698", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("first conflict fails");
    sqlx::query("DELETE FROM room_calendar WHERE room_id = ? AND booking_id IS NULL")
        .bind("R698")
        .execute(&pool)
        .await
        .expect("removes conflict");
    let replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B698", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("terminal conflict replays");

    assert_eq!(
        first.code,
        crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE
    );
    assert_eq!(replay.code, first.code);
}

#[tokio::test]
async fn reservation_command_idempotency_cancel_confirm_invalid_state_replays_terminal() {
    let pool = test_pool().await;
    seed_room(&pool, "R699A").await.unwrap();
    seed_room(&pool, "R699B").await.unwrap();
    seed_booked_reservation(&pool, "B699A", "R699A")
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B699B", "R699B")
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET status = 'active' WHERE id = ?")
        .bind("B699A")
        .execute(&pool)
        .await
        .expect("makes cancel invalid");
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B699B")
        .execute(&pool)
        .await
        .expect("makes confirm invalid");

    let cancel_ctx = cmd("reservation.cancel", "idem-cancel-invalid-replay");
    let cancel_first =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B699A")
            .await
            .expect_err("cancel invalid state fails");
    sqlx::query("UPDATE bookings SET status = 'booked' WHERE id = ?")
        .bind("B699A")
        .execute(&pool)
        .await
        .expect("would make cancel valid");
    let cancel_replay =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B699A")
            .await
            .expect_err("cancel invalid state replays");

    let confirm_ctx = cmd("reservation.confirm", "idem-confirm-invalid-replay");
    let confirm_first =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B699B")
            .await
            .expect_err("confirm invalid state fails");
    sqlx::query("UPDATE bookings SET status = 'booked' WHERE id = ?")
        .bind("B699B")
        .execute(&pool)
        .await
        .expect("would make confirm valid");
    let confirm_replay =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B699B")
            .await
            .expect_err("confirm invalid state replays");

    assert_eq!(
        cancel_first.code,
        crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION
    );
    assert_eq!(cancel_replay.code, cancel_first.code);
    assert_eq!(
        confirm_first.code,
        crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION
    );
    assert_eq!(confirm_replay.code, confirm_first.code);
}

#[tokio::test]
async fn reservation_command_idempotency_missing_booking_for_cancel_modify_confirm_replays_terminal(
) {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R700A", "R700B", "R700C"], 600_000)
        .await
        .unwrap();

    let cancel_ctx = cmd("reservation.cancel", "idem-cancel-missing");
    let cancel_first =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B700A")
            .await
            .expect_err("missing cancel booking fails");
    seed_booked_reservation(&pool, "B700A", "R700A")
        .await
        .unwrap();
    let cancel_replay =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B700A")
            .await
            .expect_err("missing cancel booking replays");

    let modify_ctx = cmd("reservation.modify", "idem-modify-missing");
    let modify_first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        reservation_modify_request("B700B", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("missing modify booking fails");
    seed_booked_reservation(&pool, "B700B", "R700B")
        .await
        .unwrap();
    let modify_replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        reservation_modify_request("B700B", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("missing modify booking replays");

    let confirm_ctx = cmd("reservation.confirm", "idem-confirm-missing");
    let confirm_first =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B700C")
            .await
            .expect_err("missing confirm booking fails");
    seed_booked_reservation(&pool, "B700C", "R700C")
        .await
        .unwrap();
    let confirm_replay =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B700C")
            .await
            .expect_err("missing confirm booking replays");

    for error in [
        cancel_first,
        cancel_replay,
        modify_first,
        modify_replay,
        confirm_first,
        confirm_replay,
    ] {
        assert_eq!(error.code, crate::app_error::codes::BOOKING_NOT_FOUND);
    }
}

#[tokio::test]
async fn reservation_command_idempotency_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_room(&pool, "R701").await.unwrap();
    seed_booked_reservation(&pool, "B701", "R701")
        .await
        .unwrap();
    let ctx = cmd("reservation.cancel", "idem-cancel-in-flight");
    let payload = serde_json::json!({
        "schema": "reservation.cancel.v1",
        "booking_id": "B701",
    });
    seed_live_in_progress_command(&pool, &ctx.command_name, &ctx.idempotency_key, &payload).await;

    let error = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B701")
        .await
        .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn reservation_command_idempotency_retryable_reclaimable_failure_can_be_reclaimed() {
    let pool = test_pool().await;
    seed_room(&pool, "R702").await.unwrap();
    seed_booked_reservation(&pool, "B702", "R702")
        .await
        .unwrap();
    let ctx = cmd("reservation.cancel", "idem-cancel-reclaim");
    let payload = serde_json::json!({
        "schema": "reservation.cancel.v1",
        "booking_id": "B702",
    });
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, error_code, error_json, retryable, created_at, updated_at,
            last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'failed_retryable', 'failed-claim',
            'DB_LOCKED_RETRYABLE', '{}', 1, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds retryable row");

    let result = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B702")
        .await
        .expect("retryable row is reclaimed");

    assert!(!result.replayed);
    assert_eq!(result.response["ok"], true);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM command_idempotency WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads status"),
        "completed"
    );
}

#[tokio::test]
async fn reservation_command_idempotency_invalid_modify_nights_replays_terminal_error() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B703", "R703", 600_000)
        .await
        .unwrap();
    let ctx = cmd("reservation.modify", "idem-modify-invalid-nights");

    let first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B703", "2026-04-23", "2026-04-26", 2),
    )
    .await
    .expect_err("invalid nights should fail inside command boundary");

    let status: String = sqlx::query_scalar(
        "SELECT status FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_one(&pool)
    .await
    .expect("invalid command row is stored");
    assert_eq!(status, "failed_terminal");

    let replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B703", "2026-04-23", "2026-04-26", 2),
    )
    .await
    .expect_err("invalid nights should replay stored terminal error");

    assert_eq!(first.code, replay.code);
    assert_eq!(first.message, replay.message);
}

#[tokio::test]
async fn reservation_command_idempotency_same_plain_key_across_commands_scopes_origin_rows() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R704", 600_000).await.unwrap();
    let plain_key = "idem-shared-reservation-origin";
    let create_ctx = cmd("reservation.create", plain_key);
    let cancel_ctx = cmd("reservation.cancel", plain_key);

    let created = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &create_ctx,
        minimal_reservation_request("R704"),
    )
    .await
    .expect("create with deposit succeeds");
    let booking_id = created.response["id"]
        .as_str()
        .expect("booking id in create response")
        .to_string();

    reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, &booking_id)
        .await
        .expect("cancel with same plain key but different command succeeds");

    let origins = sqlx::query_scalar::<_, String>(
        "SELECT origin_idempotency_key
         FROM transactions
         WHERE booking_id = ? AND type IN ('deposit', 'cancellation_fee')
         ORDER BY type ASC",
    )
    .bind(&booking_id)
    .fetch_all(&pool)
    .await
    .expect("reads transaction origins");

    assert_eq!(origins.len(), 2);
    assert!(origins.contains(&format!("{}:{}", create_ctx.command_name, plain_key)));
    assert!(origins.contains(&format!("{}:{}", cancel_ctx.command_name, plain_key)));
}
