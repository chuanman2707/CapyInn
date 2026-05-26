use chrono::{Duration, Local, NaiveDate, TimeZone};
use sqlx::Row;

use crate::{
    commands::reservations,
    domain::booking::{
        pricing::{calculate_stay_price, calculate_stay_price_tx},
        BookingError, OriginSideEffect,
    },
    models::{
        AddGroupServiceRequest, CheckOutRequest, CheckoutSettlementMode,
        CheckoutSettlementPreviewRequest, CreateGuestRequest, CreateReservationRequest,
        GroupCheckoutRequest,
    },
    money::MAX_TRANSPORT_SAFE_MONEY_VND,
    queries::booking::{audit_queries, billing_queries, revenue_queries},
};

use super::{
    audit_service,
    billing_service::{
        add_folio_line, add_folio_line_idempotent, record_cancellation_fee_tx, record_deposit_tx,
        record_deposit_with_origin_tx, record_payment, record_payment_idempotent,
        record_payment_returning_id_tx, record_payment_tx, record_payment_with_origin_tx,
    },
    group_lifecycle, group_service_management, guest_service, reservation_lifecycle,
    stay_lifecycle,
};

mod support;

use support::*;

#[tokio::test]
async fn check_in_rejects_negative_paid_amount() {
    let pool = test_pool().await;
    let mut req = minimal_checkin_request("R-NEG-PAID");
    req.paid_amount = Some(-1);

    let error = stay_lifecycle::check_in(&pool, req, None)
        .await
        .expect_err("negative paid_amount must fail");

    assert!(
        error.to_string().contains("paid_amount"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn check_out_rejects_negative_final_total() {
    let pool = test_pool().await;

    let error = stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B-NEG-FINAL".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: -1,
        },
        Local::now(),
    )
    .await
    .expect_err("negative final_total must fail");

    assert!(
        error.to_string().contains("final_total"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn reservation_rejects_negative_deposit_amount() {
    let pool = test_pool().await;
    let mut req = minimal_reservation_request("R-NEG-DEPOSIT");
    req.deposit_amount = Some(-1);

    let error = reservation_lifecycle::create_reservation(&pool, req)
        .await
        .expect_err("negative deposit_amount must fail");

    assert!(
        error.to_string().contains("deposit_amount"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn group_checkin_rejects_negative_paid_amount() {
    let pool = test_pool().await;
    let mut req = minimal_group_checkin_request(&["G-NEG-PAID"]);
    req.paid_amount = Some(-1);

    let error = group_lifecycle::group_checkin(&pool, Some("seed-user".to_string()), req)
        .await
        .expect_err("negative group paid_amount must fail");

    assert!(
        error.to_string().contains("paid_amount"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn group_checkout_rejects_negative_final_paid() {
    let pool = test_pool().await;

    let error = group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: "G-NEG-FINAL".to_string(),
            booking_ids: vec!["B-NEG-FINAL".to_string()],
            final_paid: Some(-1),
        },
    )
    .await
    .expect_err("negative final_paid must fail");

    assert!(
        error.to_string().contains("final_paid"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn group_checkout_idempotent_retry_replays_without_duplicate_effects() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R-GCO-1", "R-GCO-2"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["R-GCO-1", "R-GCO-2"]),
    )
    .await
    .unwrap();
    let rows = sqlx::query("SELECT id FROM bookings WHERE group_id = ? ORDER BY id")
        .bind(&group.id)
        .fetch_all(&pool)
        .await
        .unwrap();
    let booking_id: String = rows[0].get("id");
    let ctx = cmd("group_checkout", "idem-group-checkout-1");

    let first = group_lifecycle::group_checkout_idempotent(
        &pool,
        &ctx,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![booking_id.clone()],
            final_paid: Some(50_000),
        },
    )
    .await
    .unwrap();
    let second = group_lifecycle::group_checkout_idempotent(
        &pool,
        &ctx,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![booking_id],
            final_paid: Some(50_000),
        },
    )
    .await
    .unwrap();

    assert_replayed_pair(&first, &second);

    let payment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE note = 'Thanh toán group checkout'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payment_count, 1);

    let housekeeping_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM housekeeping WHERE room_id IN ('R-GCO-1', 'R-GCO-2')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(housekeeping_count, 1);
}

#[tokio::test]
async fn group_checkout_idempotent_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R-GCO-LIVE-1", "R-GCO-LIVE-2"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["R-GCO-LIVE-1", "R-GCO-LIVE-2"]),
    )
    .await
    .unwrap();
    let booking_id: String =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? ORDER BY id LIMIT 1")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let ctx = cmd("group_checkout", "idem-group-checkout-live");
    let payload = serde_json::json!({
        "schema": "group.checkout.v1",
        "group_id": group.id,
        "booking_ids": [booking_id],
        "final_paid_vnd_units": 50_000,
    });
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();

    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds in-flight row");

    let error = group_lifecycle::group_checkout_idempotent(
        &pool,
        &ctx,
        GroupCheckoutRequest {
            group_id: payload["group_id"].as_str().unwrap().to_string(),
            booking_ids: vec![payload["booking_ids"][0].as_str().unwrap().to_string()],
            final_paid: Some(50_000),
        },
    )
    .await
    .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn group_checkout_idempotent_final_payment_locks_group_and_candidate_folios() {
    let pool = test_pool().await;
    seed_rooms_with_price(
        &pool,
        &["R-GCO-LOCK-1", "R-GCO-LOCK-2", "R-GCO-LOCK-3"],
        250_000,
    )
    .await
    .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["R-GCO-LOCK-1", "R-GCO-LOCK-2", "R-GCO-LOCK-3"]),
    )
    .await
    .unwrap();
    let booking_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? ORDER BY id")
            .bind(&group.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    let selected_booking_id = booking_ids[0].clone();
    let ctx = cmd("group_checkout", "idem-group-checkout-locks");

    group_lifecycle::group_checkout_idempotent(
        &pool,
        &ctx,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![selected_booking_id],
            final_paid: Some(50_000),
        },
    )
    .await
    .unwrap();

    let lock_keys_json: String = sqlx::query_scalar(
        "SELECT lock_keys_json FROM command_idempotency
         WHERE command_name = 'group_checkout' AND idempotency_key = ?",
    )
    .bind(&ctx.idempotency_key)
    .fetch_one(&pool)
    .await
    .unwrap();
    let lock_keys: Vec<String> = serde_json::from_str(&lock_keys_json).unwrap();
    let mut sorted_lock_keys = lock_keys.clone();
    sorted_lock_keys.sort();
    sorted_lock_keys.dedup();
    assert_eq!(
        lock_keys, sorted_lock_keys,
        "group checkout lock keys must be sorted and deduplicated: {lock_keys_json}"
    );

    assert!(lock_keys.contains(&format!("group:{}", group.id)));
    let folio_lock_count = lock_keys
        .iter()
        .filter(|key| key.starts_with("folio:"))
        .count();
    assert!(
        folio_lock_count > 1,
        "expected payment candidate folio locks, got {lock_keys_json}"
    );
    for booking_id in booking_ids {
        assert!(
            lock_keys.contains(&format!("folio:{booking_id}")),
            "missing folio lock for payment candidate {booking_id}: {lock_keys_json}"
        );
    }
}

#[tokio::test]
async fn group_checkout_tx_posts_final_payment_only_to_locked_candidate_set() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R-GCO-CAND-1", "R-GCO-CAND-2"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["R-GCO-CAND-1", "R-GCO-CAND-2"]),
    )
    .await
    .unwrap();
    let rows = sqlx::query("SELECT id, room_id FROM bookings WHERE group_id = ? ORDER BY id")
        .bind(&group.id)
        .fetch_all(&pool)
        .await
        .unwrap();
    let selected_booking_id: String = rows[0].get("id");
    let selected_room_id: String = rows[0].get("room_id");
    let remaining_booking_id: String = rows[1].get("id");
    let locked_booking_room_map =
        std::collections::HashMap::from([(selected_booking_id.clone(), selected_room_id)]);
    let locked_payment_candidate_booking_ids = vec![selected_booking_id.clone()];

    let mut tx = pool.begin().await.unwrap();
    group_lifecycle::group_checkout_tx(
        &mut tx,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![selected_booking_id.clone()],
            final_paid: Some(50_000),
        },
        &locked_booking_room_map,
        &locked_payment_candidate_booking_ids,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let selected_payment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND note = 'Thanh toán group checkout'",
    )
    .bind(&selected_booking_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let remaining_payment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND note = 'Thanh toán group checkout'",
    )
    .bind(&remaining_booking_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(selected_payment_count, 1);
    assert_eq!(remaining_payment_count, 0);
}

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
async fn create_guest_manifest_persists_primary_and_additional_guests() {
    let pool = test_pool().await;
    let mut request = minimal_checkin_request("R201");
    request.guests.push(CreateGuestRequest {
        guest_type: Some("foreign".to_string()),
        full_name: "Jane Doe".to_string(),
        doc_number: "P1234567".to_string(),
        dob: None,
        gender: Some("female".to_string()),
        nationality: Some("US".to_string()),
        address: Some("1 Test Street".to_string()),
        visa_expiry: None,
        scan_path: None,
        phone: Some("0909999999".to_string()),
    });

    let mut tx = pool.begin().await.unwrap();
    let manifest =
        guest_service::create_guest_manifest(&mut tx, &request.guests, "2026-04-15T10:00:00+07:00")
            .await
            .unwrap();

    assert_eq!(manifest.guest_ids.len(), 2);
    assert_eq!(manifest.primary_guest_id, manifest.guest_ids[0]);

    let rows = sqlx::query(
        "SELECT full_name, guest_type, doc_number, phone FROM guests ORDER BY full_name ASC",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("full_name"), "Jane Doe");
    assert_eq!(rows[0].get::<String, _>("guest_type"), "foreign");
    assert_eq!(rows[1].get::<String, _>("full_name"), "Nguyen Van A");
}

#[tokio::test]
async fn create_guest_manifest_rejects_empty_guest_list() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let error = guest_service::create_guest_manifest(&mut tx, &[], "2026-04-15T10:00:00+07:00")
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "Phải có ít nhất 1 khách");
}

#[tokio::test]
async fn create_reservation_guest_manifest_defaults_blank_doc_number() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let manifest = guest_service::create_reservation_guest_manifest(
        &mut tx,
        "Reservation Guest",
        None,
        Some("0901234567"),
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();

    let guest = sqlx::query("SELECT full_name, doc_number, phone FROM guests WHERE id = ?")
        .bind(&manifest.primary_guest_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(manifest.guest_ids, vec![manifest.primary_guest_id.clone()]);
    assert_eq!(guest.get::<String, _>("full_name"), "Reservation Guest");
    assert_eq!(guest.get::<String, _>("doc_number"), "");
    assert_eq!(
        guest.get::<Option<String>, _>("phone"),
        Some("0901234567".to_string())
    );
}

#[tokio::test]
async fn group_checkin_creates_active_group_and_placeholder_guest_manifest() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G101", "G102"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G101", "G102"]),
    )
    .await
    .unwrap();

    assert_eq!(group.status, "active");
    assert!(group.master_booking_id.is_some());

    let room_statuses =
        sqlx::query("SELECT id, status FROM rooms WHERE id IN ('G101', 'G102') ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(room_statuses[0].get::<String, _>("status"), "occupied");
    assert_eq!(room_statuses[1].get::<String, _>("status"), "occupied");

    let booking_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = 'active'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(booking_count.0, 2);

    let paid_amounts = sqlx::query(
        "SELECT paid_amount, deposit_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(paid_amounts.len(), 2);
    assert_eq!(
        paid_amounts[0].get::<Option<i64>, _>("paid_amount"),
        Some(50_000)
    );
    assert_eq!(
        paid_amounts[1].get::<Option<i64>, _>("paid_amount"),
        Some(50_000)
    );
    assert_eq!(
        paid_amounts[0].get::<Option<i64>, _>("deposit_amount"),
        Some(0)
    );

    let placeholder = sqlx::query(
        "SELECT g.full_name, g.doc_number
         FROM guests g
         JOIN bookings b ON b.primary_guest_id = g.id
         WHERE b.group_id = ? AND b.room_id = 'G102'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        placeholder.get::<String, _>("full_name"),
        "Khách đoàn Test Group - G102"
    );
    assert_eq!(placeholder.get::<String, _>("doc_number"), "");
}

#[tokio::test]
async fn group_checkin_reservation_blocks_calendar_and_tracks_deposit() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G201", "G202"], 300_000)
        .await
        .unwrap();

    let mut req = minimal_group_checkin_request(&["G201", "G202"]);
    req.check_in_date = Some(
        (Local::now().date_naive() + Duration::days(1))
            .format("%Y-%m-%d")
            .to_string(),
    );
    req.paid_amount = Some(60_000);

    let group = group_lifecycle::group_checkin(&pool, None, req)
        .await
        .unwrap();

    assert_eq!(group.status, "booked");

    let room_statuses =
        sqlx::query("SELECT status FROM rooms WHERE id IN ('G201', 'G202') ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(room_statuses[0].get::<String, _>("status"), "vacant");
    assert_eq!(room_statuses[1].get::<String, _>("status"), "vacant");

    let calendar_rows: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id IN (SELECT id FROM bookings WHERE group_id = ?) AND status = 'booked'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_rows.0, 4);

    let amounts = sqlx::query(
        "SELECT paid_amount, deposit_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        amounts[0].get::<Option<i64>, _>("paid_amount"),
        Some(30_000)
    );
    assert_eq!(
        amounts[0].get::<Option<i64>, _>("deposit_amount"),
        Some(30_000)
    );
}

#[tokio::test]
async fn group_checkin_rejects_duplicate_room_ids() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G250"], 250_000)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["G250", "G250"]),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Phòng không được lặp trong cùng một group"
    );
}

#[tokio::test]
async fn group_checkin_lock_keys_are_stable_for_room_order() {
    let left = crate::aggregate_locks::canonicalize_lock_keys(vec![
        crate::aggregate_locks::room_key("R2").unwrap(),
        crate::aggregate_locks::room_key("R1").unwrap(),
    ])
    .unwrap();
    let right = crate::aggregate_locks::canonicalize_lock_keys(vec![
        crate::aggregate_locks::room_key("R1").unwrap(),
        crate::aggregate_locks::room_key("R2").unwrap(),
    ])
    .unwrap();

    assert_eq!(left, right);
}

#[tokio::test]
async fn group_checkin_idempotent_normalizes_room_order_and_assigns_payment_ordinals() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI601", "GI602"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-1");

    let first = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI602", "GI601"], "GI602", Some(100_001)),
    )
    .await
    .expect("first group checkin succeeds");
    let second = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI601", "GI602"], "GI602", Some(100_001)),
    )
    .await
    .expect("same payload with different room order replays");

    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.response["id"], second.response["id"]);

    let rows = sqlx::query(
        "SELECT b.room_id, t.amount, t.origin_transaction_ordinal
         FROM transactions t
         JOIN bookings b ON b.id = t.booking_id
         WHERE t.origin_idempotency_key = ? AND t.type = 'payment'
         ORDER BY t.origin_transaction_ordinal ASC",
    )
    .bind("idem-group-checkin-1")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("room_id"), "GI601");
    assert_eq!(rows[0].get::<i64, _>("amount"), 50_001);
    assert_eq!(rows[0].get::<i64, _>("origin_transaction_ordinal"), 0);
    assert_eq!(rows[1].get::<String, _>("room_id"), "GI602");
    assert_eq!(rows[1].get::<i64, _>("amount"), 50_000);
    assert_eq!(rows[1].get::<i64, _>("origin_transaction_ordinal"), 1);

    let lock_keys_json: String = sqlx::query_scalar(
        "SELECT lock_keys_json
         FROM command_idempotency
         WHERE idempotency_key = ?",
    )
    .bind("idem-group-checkin-1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(lock_keys_json, r#"["room:GI601","room:GI602"]"#);
}

#[tokio::test]
async fn group_checkin_idempotent_materializes_omitted_checkin_date_in_hash() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI603", "GI604"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-materialized-date");

    let req = rich_group_checkin_request(&["GI603", "GI604"], "GI603", Some(100_000));
    assert!(req.check_in_date.is_none());

    group_lifecycle::group_checkin_idempotent(&pool, Some("seed-user".to_string()), &ctx, req)
        .await
        .expect("group checkin succeeds");

    let stored_hash: String = sqlx::query_scalar(
        "SELECT request_hash
         FROM command_idempotency
         WHERE idempotency_key = ?",
    )
    .bind("idem-group-checkin-materialized-date")
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut materialized = rich_group_checkin_request(&["GI603", "GI604"], "GI603", Some(100_000));
    materialized.check_in_date = Some(ctx.issued_at.format("%Y-%m-%d").to_string());
    let expected_hash = crate::command_idempotency::stable_request_hash(
        &group_checkin_hash_payload_for_test(&materialized),
    )
    .expect("materialized payload hashes");
    let null_date_hash =
        crate::command_idempotency::stable_request_hash(&group_checkin_hash_payload_for_test(
            &rich_group_checkin_request(&["GI603", "GI604"], "GI603", Some(100_000)),
        ))
        .expect("null date payload hashes");

    assert_eq!(stored_hash, expected_hash);
    assert_ne!(stored_hash, null_date_hash);
}

#[tokio::test]
async fn group_checkin_idempotent_omitted_date_replays_after_issued_at_rollover() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI605", "GI606"], 250_000)
        .await
        .unwrap();
    let first_ctx = cmd("group_checkin", "idem-group-checkin-rollover");

    let first = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &first_ctx,
        rich_group_checkin_request(&["GI605", "GI606"], "GI605", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");

    let retry_ctx = cmd_at(
        "group_checkin",
        "idem-group-checkin-rollover",
        "2026-04-25T01:00:00+07:00",
    );

    let retry = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &retry_ctx,
        rich_group_checkin_request(&["GI605", "GI606"], "GI605", Some(100_000)),
    )
    .await
    .expect("omitted-date retry replays across issued_at rollover");

    assert!(!first.replayed);
    assert!(retry.replayed);
    assert_eq!(first.response["id"], retry.response["id"]);
}

#[tokio::test]
async fn group_checkin_idempotent_reclaimed_omitted_date_uses_original_command_time() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI607", "GI608"], 250_000)
        .await
        .unwrap();

    let original_ctx = cmd_at(
        "group_checkin",
        "idem-group-checkin-reclaim-rollover",
        "2026-04-24T10:00:00+07:00",
    );
    let mut materialized = rich_group_checkin_request(&["GI607", "GI608"], "GI607", Some(100_000));
    materialized.check_in_date = Some("2026-04-24".to_string());
    let payload = group_checkin_hash_payload_for_test(&materialized);
    let request_hash =
        crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes");
    let now = chrono::Utc::now().to_rfc3339();
    let expired_lease = (chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339();
    let intent_json = serde_json::json!({
        "fields": {
            "schema": "group.checkin.v1",
            "room_count": 2,
            "guest_room_count": 2,
            "guest_form_count": 2,
            "nights": 2,
            "check_in_date": "2026-04-24",
            "has_organizer_contact": true,
            "has_source": true,
            "has_notes": true,
            "has_paid_amount": true,
            "paid_amount_positive": true,
        }
    })
    .to_string();

    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, issued_at,
            created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, ?, ?, 'in_progress', 'expired-claim', 0, ?, ?, ?, ?, ?)",
    )
    .bind("idem-group-checkin-reclaim-rollover")
    .bind("group_checkin")
    .bind(&request_hash)
    .bind(intent_json)
    .bind(r#"["room:GI607","room:GI608"]"#)
    .bind(&expired_lease)
    .bind(original_ctx.issued_at.to_rfc3339())
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seed expired in-progress command");

    let retry_ctx = cmd_at(
        "group_checkin",
        "idem-group-checkin-reclaim-rollover",
        "2026-04-25T01:00:00+07:00",
    );

    let result = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &retry_ctx,
        rich_group_checkin_request(&["GI607", "GI608"], "GI607", Some(100_000)),
    )
    .await
    .expect("reclaimed omitted-date command succeeds");

    assert!(!result.replayed);

    let statuses = sqlx::query(
        "SELECT status, booking_type, scheduled_checkin
         FROM bookings
         WHERE group_id = ?
         ORDER BY room_id",
    )
    .bind(result.response["id"].as_str().unwrap())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(statuses.len(), 2);
    for row in statuses {
        assert_eq!(
            row.get::<String, _>("status"),
            crate::models::status::booking::ACTIVE
        );
        assert_eq!(row.get::<String, _>("booking_type"), "walk-in");
        assert_eq!(
            row.get::<Option<String>, _>("scheduled_checkin"),
            None::<String>
        );
    }

    let room_statuses = sqlx::query_scalar::<_, String>(
        "SELECT status FROM rooms WHERE id IN ('GI607', 'GI608') ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        room_statuses,
        vec![
            crate::models::status::room::OCCUPIED.to_string(),
            crate::models::status::room::OCCUPIED.to_string()
        ]
    );
}

#[tokio::test]
async fn group_checkin_idempotent_retry_does_not_duplicate_groups_bookings_or_payments() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI620", "GI621"], 250_000)
        .await
        .unwrap();
    let ctx = cmd_with_request(
        "group_checkin",
        "req-group-idem-no-dup",
        "idem-group-checkin-no-dup",
    );

    let first = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI620", "GI621"], "GI620", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");
    let replay = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI620", "GI621"], "GI620", Some(100_000)),
    )
    .await
    .expect("retry replays");

    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(first.response["id"], replay.response["id"]);
    assert_single_outbox_event(&pool, &ctx, "group.checked_in").await;

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    let payment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE type = 'payment'")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(group_count, 1);
    assert_eq!(booking_count, 2);
    assert_eq!(payment_count, 2);
}

#[tokio::test]
async fn group_checkin_duplicate_in_flight_does_not_wait_for_room_lock() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI650", "GI651"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-inflight");
    let held_room_lock = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::room_key("GI650").unwrap()])
        .await
        .unwrap();

    let first_pool = pool.clone();
    let first_ctx = ctx.clone();
    let first = tokio::spawn(async move {
        group_lifecycle::group_checkin_idempotent(
            &first_pool,
            Some("seed-user".to_string()),
            &first_ctx,
            rich_group_checkin_request(&["GI650", "GI651"], "GI650", Some(100_000)),
        )
        .await
    });

    let mut claim_seen = false;
    for _ in 0..50 {
        let in_progress_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM command_idempotency
             WHERE idempotency_key = ? AND status = 'in_progress'",
        )
        .bind("idem-group-checkin-inflight")
        .fetch_one(&pool)
        .await
        .unwrap();
        if in_progress_count == 1 {
            claim_seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        claim_seen,
        "first command should claim before waiting for room lock"
    );

    let duplicate = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        group_lifecycle::group_checkin_idempotent(
            &pool,
            Some("seed-user".to_string()),
            &ctx,
            rich_group_checkin_request(&["GI650", "GI651"], "GI650", Some(100_000)),
        ),
    )
    .await
    .expect("duplicate should return without waiting for room lock")
    .expect_err("duplicate in-flight command should conflict");

    assert_eq!(
        duplicate.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );

    drop(held_room_lock);
    first.await.unwrap().expect("first command completes");
}

#[tokio::test]
async fn group_checkin_idempotent_duplicate_seeded_live_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI655", "GI656"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-seeded-inflight");
    let mut req = rich_group_checkin_request(&["GI655", "GI656"], "GI655", Some(100_000));
    req.check_in_date = Some(ctx.issued_at.format("%Y-%m-%d").to_string());
    seed_live_in_progress_command(
        &pool,
        "group_checkin",
        "idem-group-checkin-seeded-inflight",
        &group_checkin_hash_payload_for_test(&req),
    )
    .await;

    let error =
        group_lifecycle::group_checkin_idempotent(&pool, Some("seed-user".to_string()), &ctx, req)
            .await
            .expect_err("duplicate live in-flight command should conflict");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn group_checkin_idempotent_zero_paid_amount_writes_no_payment_origin_rows() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI610", "GI611"], 250_000)
        .await
        .unwrap();

    let zero_ctx = cmd("group_checkin", "idem-group-checkin-2-zero");
    let zero_paid = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &zero_ctx,
        rich_group_checkin_request(&["GI610", "GI611"], "GI610", Some(0)),
    )
    .await
    .expect("zero paid amount still creates group");
    assert_eq!(zero_paid.response["status"].as_str(), Some("active"));

    let zero_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE origin_idempotency_key = ?")
            .bind("idem-group-checkin-2-zero")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(zero_count, 0);
}

#[tokio::test]
async fn group_checkin_idempotent_blank_key_rejected_before_writes() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI620", "GI621"], 250_000)
        .await
        .unwrap();

    let error = crate::command_idempotency::WriteCommandContext::for_scoped_command(
        "req-group-idem-blank",
        " ",
        "group_checkin",
    )
    .expect_err("blank idempotency key rejected");
    assert_eq!(
        error.code,
        crate::app_error::codes::IDEMPOTENCY_KEY_REQUIRED
    );

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_count, 0);

    let claim_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency WHERE request_id = 'req-group-idem-blank'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claim_count, 0);
}

#[tokio::test]
async fn group_checkin_idempotent_replay_returns_stored_snapshot_after_db_mutation() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI630", "GI631"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-3");

    let first = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI630", "GI631"], "GI630", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");
    let group_id = first.response["id"].as_str().unwrap().to_string();
    let first_status = first.response["status"].as_str().unwrap().to_string();

    sqlx::query("UPDATE booking_groups SET status = 'completed' WHERE id = ?")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();

    let replay = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI630", "GI631"], "GI630", Some(100_000)),
    )
    .await
    .expect("replay succeeds");

    assert!(replay.replayed);
    assert_eq!(
        replay.response["status"].as_str(),
        Some(first_status.as_str())
    );
    assert_ne!(replay.response["status"].as_str(), Some("completed"));
}

#[tokio::test]
async fn group_checkin_idempotent_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI640", "GI641"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-4");

    group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI640", "GI641"], "GI640", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");

    let mut changed = rich_group_checkin_request(&["GI640", "GI641"], "GI640", Some(100_000));
    changed.nights = 3;
    let error = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        changed,
    )
    .await
    .expect_err("same key with different payload conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn group_checkin_idempotent_same_key_changed_guest_name_conflicts() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI642", "GI643"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-guest-change");

    group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI642", "GI643"], "GI642", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");

    let mut changed = rich_group_checkin_request(&["GI642", "GI643"], "GI642", Some(100_000));
    changed.guests_per_room.get_mut("GI642").unwrap()[0].full_name =
        "Changed Guest Name".to_string();
    let error = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        changed,
    )
    .await
    .expect_err("same key with changed guest name conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

async fn checked_in_group_for_service_tests(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    room_ids: &[&str],
    daily_rate: crate::money::MoneyVnd,
) -> crate::models::BookingGroup {
    seed_rooms_with_price(pool, room_ids, daily_rate)
        .await
        .unwrap();
    group_lifecycle::group_checkin(pool, None, group_checkin_req(room_ids).build())
        .await
        .unwrap()
}

#[tokio::test]
async fn add_group_service_idempotent_retry_replays_without_duplicate_row() {
    let pool = test_pool().await;
    let group = checked_in_group_for_service_tests(&pool, &["G-SVC-1", "G-SVC-2"], 250_000).await;
    let ctx = cmd_with_request(
        "add_group_service",
        "req-group-svc-idem",
        "idem-add-group-service-1",
    );
    let service_req = || AddGroupServiceRequest {
        group_id: group.id.clone(),
        booking_id: None,
        name: "Laundry".to_string(),
        quantity: 2,
        unit_price: 25_000,
        note: Some("same-day".to_string()),
    };

    let first = group_service_management::add_group_service_idempotent(
        &pool,
        &ctx,
        service_req(),
        "staff-1",
    )
    .await
    .expect("first service add succeeds");
    let second = group_service_management::add_group_service_idempotent(
        &pool,
        &ctx,
        service_req(),
        "staff-1",
    )
    .await
    .expect("retry replays");

    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.response["id"], second.response["id"]);
    assert_eq!(
        first.response["total_price"],
        second.response["total_price"]
    );
    assert_eq!(first.response["total_price"], 50_000);
    assert_single_outbox_event(&pool, &ctx, "group.service_added").await;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_services WHERE group_id = ?")
        .bind(&group.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn add_group_service_idempotent_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    let group =
        checked_in_group_for_service_tests(&pool, &["G-SVC-HASH-1", "G-SVC-HASH-2"], 250_000).await;
    let ctx = cmd("add_group_service", "idem-add-group-service-hash");
    let service_req = |quantity| AddGroupServiceRequest {
        group_id: group.id.clone(),
        booking_id: None,
        name: "Laundry".to_string(),
        quantity,
        unit_price: 25_000,
        note: None,
    };

    group_service_management::add_group_service_idempotent(&pool, &ctx, service_req(1), "staff-1")
        .await
        .expect("first service add succeeds");
    let error = group_service_management::add_group_service_idempotent(
        &pool,
        &ctx,
        service_req(2),
        "staff-1",
    )
    .await
    .expect_err("same key with changed quantity conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_services WHERE group_id = ?")
        .bind(&group.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn add_group_service_idempotent_rejects_negative_unit_price_without_writing() {
    let pool = test_pool().await;
    let group = checked_in_group_for_service_tests(
        &pool,
        &["G-SVC-NEG-PRICE-1", "G-SVC-NEG-PRICE-2"],
        250_000,
    )
    .await;
    let ctx = cmd("add_group_service", "idem-add-group-service-negative-price");
    let req = AddGroupServiceRequest {
        group_id: group.id.clone(),
        booking_id: None,
        name: "Laundry".to_string(),
        quantity: 1,
        unit_price: -25_000,
        note: None,
    };

    let error = group_service_management::add_group_service_idempotent(&pool, &ctx, req, "staff-1")
        .await
        .expect_err("negative unit price is rejected");

    assert_eq!(error.code, crate::app_error::codes::BOOKING_INVALID_STATE);

    assert_eq!(
        command_claim_count(&pool, &ctx.command_name, &ctx.idempotency_key).await,
        0
    );
    assert_eq!(group_service_count_for_group(&pool, &group.id).await, 0);

    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);
    assert_eq!(origin_transaction_count(&pool, &origin_key).await, 0);
    assert_eq!(folio_line_count_for_key(&pool, &origin_key).await, 0);
    assert_eq!(
        outbox_count_for_command(&pool, &ctx.command_name, &ctx.idempotency_key).await,
        0
    );
}

#[tokio::test]
async fn remove_group_service_idempotent_retry_replays_without_extra_delete() {
    let pool = test_pool().await;
    let group =
        checked_in_group_for_service_tests(&pool, &["G-SVC-REMOVE-1", "G-SVC-REMOVE-2"], 250_000)
            .await;
    sqlx::query(
        "INSERT INTO group_services (
            id, group_id, booking_id, name, quantity, unit_price,
            total_price, note, created_by, created_at
        ) VALUES ('SVC-REMOVE-1', ?, NULL, 'Laundry', 1, 25000, 25000, NULL, 'staff-1', '2026-05-01T09:00:00+07:00')",
    )
    .bind(&group.id)
    .execute(&pool)
    .await
    .unwrap();
    let ctx = cmd("remove_group_service", "idem-remove-group-service-1");

    let first =
        group_service_management::remove_group_service_idempotent(&pool, &ctx, "SVC-REMOVE-1")
            .await
            .expect("first remove succeeds");
    let second =
        group_service_management::remove_group_service_idempotent(&pool, &ctx, "SVC-REMOVE-1")
            .await
            .expect("retry replays");

    assert_replayed_pair(&first, &second);
    assert_eq!(first.response["service_id"], "SVC-REMOVE-1");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_services WHERE id = 'SVC-REMOVE-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn remove_group_service_idempotent_same_key_different_service_conflicts() {
    let pool = test_pool().await;
    let group = checked_in_group_for_service_tests(
        &pool,
        &["G-SVC-REMOVE-HASH-1", "G-SVC-REMOVE-HASH-2"],
        250_000,
    )
    .await;
    for service_id in ["SVC-REMOVE-A", "SVC-REMOVE-B"] {
        sqlx::query(
            "INSERT INTO group_services (
                id, group_id, booking_id, name, quantity, unit_price,
                total_price, note, created_by, created_at
            ) VALUES (?, ?, NULL, 'Laundry', 1, 25000, 25000, NULL, 'staff-1', '2026-05-01T09:00:00+07:00')",
        )
        .bind(service_id)
        .bind(&group.id)
        .execute(&pool)
        .await
        .unwrap();
    }
    let ctx = cmd("remove_group_service", "idem-remove-group-service-hash");

    group_service_management::remove_group_service_idempotent(&pool, &ctx, "SVC-REMOVE-A")
        .await
        .expect("first remove succeeds");
    let error =
        group_service_management::remove_group_service_idempotent(&pool, &ctx, "SVC-REMOVE-B")
            .await
            .expect_err("same key with changed service id conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );

    let remaining_b: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM group_services WHERE id = 'SVC-REMOVE-B'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_b, 1);
}

#[tokio::test]
async fn group_checkout_reassigns_master_and_updates_group_payment() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G301", "G302"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G301", "G302"]),
    )
    .await
    .unwrap();

    let master_booking_id = group.master_booking_id.clone().unwrap();
    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![master_booking_id.clone()],
            final_paid: Some(40_000),
        },
    )
    .await
    .unwrap();

    let group_row =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(group_row.get::<String, _>("status"), "partial_checkout");
    assert_ne!(
        group_row.get::<Option<String>, _>("master_booking_id"),
        Some(master_booking_id.clone())
    );

    let checked_out = sqlx::query("SELECT status FROM bookings WHERE id = ?")
        .bind(&master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(checked_out.get::<String, _>("status"), "checked_out");

    let housekeeping_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM housekeeping WHERE room_id = 'G301'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(housekeeping_count.0, 1);

    let remaining_paid: (i64,) = sqlx::query_as(
        "SELECT paid_amount FROM bookings WHERE group_id = ? AND status = 'active' LIMIT 1",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining_paid.0, 90_000);
}

#[tokio::test]
async fn group_checkout_clears_master_flag_when_group_completes() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G401"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G401"]),
    )
    .await
    .unwrap();

    let master_booking_id = group.master_booking_id.clone().unwrap();
    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![master_booking_id.clone()],
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let group_row =
        sqlx::query("SELECT master_booking_id, status FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        group_row.get::<Option<String>, _>("master_booking_id"),
        None
    );
    assert_eq!(group_row.get::<String, _>("status"), "completed");

    let booking_row = sqlx::query("SELECT is_master_room FROM bookings WHERE id = ?")
        .bind(&master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking_row.get::<i64, _>("is_master_room"), 0);
}

#[tokio::test]
async fn group_booking_lifecycle_smoke_covers_partial_and_final_checkout() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-SMOKE-1", "G-SMOKE-2"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G-SMOKE-1", "G-SMOKE-2"]),
    )
    .await
    .unwrap();

    assert_eq!(group.status, "active");

    let initial_group =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(initial_group.get::<String, _>("status"), "active");
    let first_master_booking_id = initial_group
        .get::<Option<String>, _>("master_booking_id")
        .expect("group check-in should assign a master booking");

    let initial_bookings = sqlx::query(
        "SELECT id, room_id, status, is_master_room
         FROM bookings
         WHERE group_id = ?
         ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(initial_bookings.len(), 2);
    assert!(initial_bookings.iter().any(|row| {
        row.get::<String, _>("id") == first_master_booking_id
            && row.get::<String, _>("status") == "active"
            && row.get::<i64, _>("is_master_room") == 1
    }));
    assert!(initial_bookings
        .iter()
        .all(|row| row.get::<String, _>("status") == "active"));

    let occupied_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'occupied'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(occupied_count.0, 2);

    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![first_master_booking_id.clone()],
            final_paid: Some(40_000),
        },
    )
    .await
    .unwrap();

    let partial_group =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(partial_group.get::<String, _>("status"), "partial_checkout");
    let second_master_booking_id = partial_group
        .get::<Option<String>, _>("master_booking_id")
        .expect("partial checkout should reassign a master booking");
    assert_ne!(second_master_booking_id, first_master_booking_id);

    let checked_out_status: String = sqlx::query_scalar("SELECT status FROM bookings WHERE id = ?")
        .bind(&first_master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(checked_out_status, "checked_out");

    let remaining_master = sqlx::query(
        "SELECT id, is_master_room
         FROM bookings
         WHERE group_id = ? AND status = 'active'
         LIMIT 1",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        remaining_master.get::<String, _>("id"),
        second_master_booking_id
    );
    assert_eq!(remaining_master.get::<i64, _>("is_master_room"), 1);

    let first_room_cleaning_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id = 'G-SMOKE-1' AND status = 'cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first_room_cleaning_count.0, 1);

    let first_housekeeping_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM housekeeping
         WHERE room_id = 'G-SMOKE-1' AND status = 'needs_cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first_housekeeping_count.0, 1);

    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![second_master_booking_id],
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let final_group =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(final_group.get::<String, _>("status"), "completed");
    assert_eq!(
        final_group.get::<Option<String>, _>("master_booking_id"),
        None
    );

    let active_booking_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = 'active'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active_booking_count.0, 0);

    let checked_out_booking_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM bookings WHERE group_id = ? AND status = 'checked_out'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(checked_out_booking_count.0, 2);

    let remaining_occupied_rooms: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'occupied'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining_occupied_rooms.0, 0);

    let cleaning_room_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cleaning_room_count.0, 2);

    let housekeeping_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM housekeeping
         WHERE room_id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'needs_cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(housekeeping_count.0, 2);
}

#[tokio::test]
async fn group_checkout_rejects_stale_selected_booking() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G501", "G502"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G501", "G502"]),
    )
    .await
    .unwrap();

    let booking_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? ORDER BY room_id")
            .bind(&group.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind(&booking_ids[0])
        .execute(&pool)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids,
            final_paid: None,
        },
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[test]
fn group_checkout_locked_room_map_rejects_changed_room_mapping() {
    let locked = std::collections::HashMap::from([
        ("booking-1".to_string(), "room-old".to_string()),
        ("booking-2".to_string(), "room-stable".to_string()),
    ]);
    let current = std::collections::HashMap::from([
        ("booking-1".to_string(), "room-new".to_string()),
        ("booking-2".to_string(), "room-stable".to_string()),
    ]);

    let error = group_lifecycle::ensure_group_checkout_room_map_still_locked(
        "group-1",
        &["booking-1".to_string(), "booking-2".to_string()],
        &locked,
        &current,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
    assert!(error
        .to_string()
        .contains("one or more bookings in group group-1 changed rooms before checkout"));
}

#[tokio::test]
async fn record_payment_updates_paid_amount_cache() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B101", "R101")
        .await
        .unwrap();

    record_payment(&pool, "B101", 25_000, "deposit")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B101")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking.get::<Option<i64>, _>("paid_amount"), Some(25_000));

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

    let mut tx = super::support::begin_tx(&pool).await.unwrap();
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

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B-PAY-ID")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<i64>, _>("paid_amount"), Some(25_000));
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

    let paid_amount: i64 = sqlx::query_scalar("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B-PAY-IDEM")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(paid_amount, 125_000);

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

    let paid_amount: i64 = sqlx::query_scalar("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B-PAY-SUM")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(paid_amount, 100_000);
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

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B103")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<i64>, _>("paid_amount"), Some(75_000));

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

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B104")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<i64>, _>("paid_amount"), Some(50_000));

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

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_pricing_rule() {
    let pool = test_pool().await;
    seed_room(&pool, "R150").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    seed_pricing_rule_tx(&mut tx, "standard", 600_000)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R150",
        "2026-04-15T10:00:00+07:00",
        "2026-04-17T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_200_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_matches_tx_path_and_applies_special_date_uplift() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R149", 600_000).await.unwrap();
    seed_special_date(&pool, "2026-04-20", 10.0).await.unwrap();

    let pool_pricing = calculate_stay_price(
        &pool,
        "R149",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pool_pricing.total, 1_320_000);
    assert_eq!(pool_pricing.base_amount, 1_200_000);
    assert_eq!(pool_pricing.surcharge_amount, 120_000);
    assert_eq!(pool_pricing.weekend_amount, 0);
    assert_eq!(pool_pricing.breakdown.len(), 2);
    assert_eq!(pool_pricing.breakdown[0].amount, 1_200_000);
    assert!(pool_pricing.breakdown[0].label.contains("night(s)"));
    assert!(pool_pricing
        .breakdown
        .iter()
        .any(|line| line.label == "Holiday surcharge"));

    let mut tx = pool.begin().await.unwrap();
    let tx_pricing = calculate_stay_price_tx(
        &mut tx,
        "R149",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(tx_pricing.pricing_type, pool_pricing.pricing_type);
    assert_eq!(tx_pricing.base_amount, pool_pricing.base_amount);
    assert_eq!(tx_pricing.surcharge_amount, pool_pricing.surcharge_amount);
    assert_eq!(tx_pricing.weekend_amount, pool_pricing.weekend_amount);
    assert_eq!(tx_pricing.total, pool_pricing.total);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_returns_not_found_for_missing_room() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let error = calculate_stay_price_tx(
        &mut tx,
        "missing-room",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::NotFound(message) if message.contains("Không tìm thấy phòng missing-room")
    ));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_returns_datetime_parse_for_invalid_check_in() {
    let pool = test_pool().await;
    seed_room(&pool, "R153").await.unwrap();
    let mut tx = pool.begin().await.unwrap();

    let error = calculate_stay_price_tx(
        &mut tx,
        "R153",
        "not-a-datetime",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
    ));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_room_base_price() {
    let pool = test_pool().await;
    seed_room(&pool, "R151").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE rooms SET base_price = ? WHERE id = ?")
        .bind(600_000)
        .bind("R151")
        .execute(&mut *tx)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R151",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_200_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_special_date() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R152", 600_000).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    seed_special_date_tx(&mut tx, "2026-04-20", 10.0)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R152",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_320_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_returns_not_found_for_missing_room() {
    let pool = test_pool().await;

    let error = calculate_stay_price(
        &pool,
        "missing-room",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::NotFound(message) if message.contains("Không tìm thấy phòng missing-room")
    ));
}

#[tokio::test]
async fn calculate_stay_price_returns_datetime_parse_for_invalid_check_in() {
    let pool = test_pool().await;
    seed_room(&pool, "R153").await.unwrap();

    let error = calculate_stay_price(
        &pool,
        "R153",
        "not-a-datetime",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
    ));
}

#[tokio::test]
async fn create_reservation_blocks_calendar_and_posts_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "R160").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000).await.unwrap();

    let booking =
        reservation_lifecycle::create_reservation(&pool, minimal_reservation_request("R160"))
            .await
            .unwrap();

    assert_eq!(booking.room_id, "R160");
    assert_eq!(booking.status, "booked");
    assert_eq!(booking.total_price, 1_200_000);
    assert_eq!(booking.paid_amount, 50_000);

    assert_calendar_rows(&pool, &booking.id, "booked", 2).await;

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R160")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let deposit = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND type = 'deposit' LIMIT 1",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(deposit.get::<String, _>("type"), "deposit");
    assert_eq!(deposit.get::<i64, _>("amount"), 50_000);
    assert_eq!(deposit.get::<String, _>("note"), "Reservation deposit");
}

#[tokio::test]
async fn create_reservation_rejects_inconsistent_nights_input() {
    let pool = test_pool().await;
    seed_room(&pool, "R160A").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000).await.unwrap();

    let error = reservation_lifecycle::create_reservation(
        &pool,
        CreateReservationRequest {
            room_id: "R160A".to_string(),
            guest_name: "Nguyen Van B".to_string(),
            guest_phone: Some("0900000001".to_string()),
            guest_doc_number: Some("079000000001".to_string()),
            check_in_date: "2026-04-20".to_string(),
            check_out_date: "2026-04-22".to_string(),
            nights: 3,
            deposit_amount: Some(50_000),
            source: Some("phone".to_string()),
            notes: Some("test reservation".to_string()),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        crate::domain::booking::BookingError::Validation(_)
    ));
}

#[tokio::test]
async fn create_reservation_idempotent_retry_does_not_duplicate_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "R601").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000)
        .await
        .expect("seeds pricing");
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
    seed_room(&pool, "R604").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000)
        .await
        .expect("seeds pricing");
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
    seed_room(&pool, "R602").await.expect("seeds room");
    seed_room(&pool, "R603").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000)
        .await
        .expect("seeds pricing");
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
    seed_room(&pool, "R690").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000)
        .await
        .expect("seeds pricing");
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
    seed_room(&pool, "R691").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000)
        .await
        .expect("seeds pricing");
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
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
            .bind("R691")
            .fetch_one(&pool)
            .await
            .expect("counts bookings"),
        1
    );
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

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B693")
            .fetch_one(&pool)
            .await
            .expect("counts calendar rows"),
        3
    );
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
    seed_room(&pool, "R697A").await.unwrap();
    seed_room(&pool, "R697B").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000).await.unwrap();
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
    seed_room(&pool, "R700A").await.unwrap();
    seed_room(&pool, "R700B").await.unwrap();
    seed_room(&pool, "R700C").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000).await.unwrap();

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
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds in-flight row");

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
    seed_room(&pool, "R704").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000).await.unwrap();
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

#[tokio::test]
async fn reservation_lifecycle_smoke_covers_confirm_and_cancel_paths() {
    let pool = test_pool().await;
    seed_room(&pool, "R-SMOKE-CONFIRM")
        .await
        .expect("seed confirm room");
    seed_room(&pool, "R-SMOKE-CANCEL")
        .await
        .expect("seed cancel room");
    seed_pricing_rule(&pool, "standard", 600_000)
        .await
        .expect("seed pricing");

    let today = Local::now().date_naive();
    let reservation_request = |room_id: &str, start_offset_days: i64| {
        let check_in = today + Duration::days(start_offset_days);
        let check_out = check_in + Duration::days(2);
        CreateReservationRequest {
            room_id: room_id.to_string(),
            guest_name: format!("Smoke Guest {room_id}"),
            guest_phone: Some("0900000137".to_string()),
            guest_doc_number: Some(format!("DOC-{room_id}")),
            check_in_date: check_in.format("%Y-%m-%d").to_string(),
            check_out_date: check_out.format("%Y-%m-%d").to_string(),
            nights: 2,
            deposit_amount: Some(50_000),
            source: Some("phone".to_string()),
            notes: Some("reservation smoke".to_string()),
        }
    };

    let create_confirm_ctx = cmd_with_request(
        "create_reservation",
        "req-smoke-reservation-create-confirm",
        "idem-smoke-reservation-create-confirm",
    );
    let created_for_confirm = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &create_confirm_ctx,
        reservation_request("R-SMOKE-CONFIRM", 0),
    )
    .await
    .expect("reservation create succeeds for confirm branch");
    let confirm_booking_id = created_for_confirm.response["id"]
        .as_str()
        .expect("created reservation id")
        .to_string();

    assert_eq!(
        created_for_confirm.response["status"],
        serde_json::json!("booked")
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
            .bind("R-SMOKE-CONFIRM")
            .fetch_one(&pool)
            .await
            .expect("confirm room status after create"),
        "vacant"
    );
    assert_calendar_rows(&pool, &confirm_booking_id, "booked", 2).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE booking_id = ? AND type = 'deposit'",
        )
        .bind(&confirm_booking_id)
        .fetch_one(&pool)
        .await
        .expect("reservation deposit total"),
        50_000
    );
    assert_single_outbox_event(&pool, &create_confirm_ctx, "booking.reservation_created").await;

    let confirm_ctx = cmd_with_request(
        "confirm_reservation",
        "req-smoke-reservation-confirm",
        "idem-smoke-reservation-confirm",
    );
    let confirmed = reservation_lifecycle::confirm_reservation_idempotent(
        &pool,
        &confirm_ctx,
        &confirm_booking_id,
    )
    .await
    .expect("reservation confirm succeeds");
    let confirmed_nights = confirmed.response["nights"]
        .as_i64()
        .expect("confirmed reservation nights");
    let confirmed_total_price = confirmed.response["total_price"]
        .as_i64()
        .expect("confirmed reservation total price");

    assert_eq!(confirmed.response["status"], serde_json::json!("active"));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM bookings WHERE id = ?")
            .bind(&confirm_booking_id)
            .fetch_one(&pool)
            .await
            .expect("confirmed booking status"),
        "active"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
            .bind("R-SMOKE-CONFIRM")
            .fetch_one(&pool)
            .await
            .expect("confirm room status"),
        "occupied"
    );
    assert_calendar_rows(&pool, &confirm_booking_id, "occupied", confirmed_nights).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE booking_id = ? AND type = 'charge'",
        )
        .bind(&confirm_booking_id)
        .fetch_one(&pool)
        .await
        .expect("reservation room charge total"),
        confirmed_total_price
    );
    assert_single_outbox_event(&pool, &confirm_ctx, "booking.reservation_confirmed").await;

    let create_cancel_ctx = cmd_with_request(
        "create_reservation",
        "req-smoke-reservation-create-cancel",
        "idem-smoke-reservation-create-cancel",
    );
    let created_for_cancel = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &create_cancel_ctx,
        reservation_request("R-SMOKE-CANCEL", 0),
    )
    .await
    .expect("reservation create succeeds for cancel branch");
    let cancel_booking_id = created_for_cancel.response["id"]
        .as_str()
        .expect("created cancel reservation id")
        .to_string();
    assert_single_outbox_event(&pool, &create_cancel_ctx, "booking.reservation_created").await;

    let cancel_ctx = cmd_with_request(
        "cancel_reservation",
        "req-smoke-reservation-cancel",
        "idem-smoke-reservation-cancel",
    );
    let cancelled = reservation_lifecycle::cancel_reservation_idempotent(
        &pool,
        &cancel_ctx,
        &cancel_booking_id,
    )
    .await
    .expect("reservation cancel succeeds");

    assert_eq!(cancelled.response["ok"], serde_json::json!(true));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM bookings WHERE id = ?")
            .bind(&cancel_booking_id)
            .fetch_one(&pool)
            .await
            .expect("cancelled booking status"),
        "cancelled"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
            .bind("R-SMOKE-CANCEL")
            .fetch_one(&pool)
            .await
            .expect("cancel room status"),
        "vacant"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind(&cancel_booking_id)
            .fetch_one(&pool)
            .await
            .expect("cancelled calendar rows"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE booking_id = ? AND type = 'cancellation_fee'",
        )
        .bind(&cancel_booking_id)
        .fetch_one(&pool)
        .await
        .expect("cancellation fee total"),
        50_000
    );
    assert_single_outbox_event(&pool, &cancel_ctx, "booking.reservation_cancelled").await;
}

#[tokio::test]
async fn cancel_reservation_releases_calendar_and_keeps_fee_record() {
    let pool = test_pool().await;
    seed_room(&pool, "R161").await.unwrap();
    seed_booked_reservation(&pool, "B161", "R161")
        .await
        .unwrap();

    sqlx::query("UPDATE rooms SET status = 'booked' WHERE id = ?")
        .bind("R161")
        .execute(&pool)
        .await
        .unwrap();

    reservation_lifecycle::cancel_reservation(&pool, "B161")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT status, paid_amount FROM bookings WHERE id = ?")
        .bind("B161")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking.get::<String, _>("status"), "cancelled");
    assert_eq!(booking.get::<Option<i64>, _>("paid_amount"), Some(50_000));

    let remaining_calendar: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B161")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_calendar.0, 0);

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R161")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let fee = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND type = 'cancellation_fee' LIMIT 1",
    )
    .bind("B161")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fee.get::<String, _>("type"), "cancellation_fee");
    assert_eq!(fee.get::<i64, _>("amount"), 50_000);
    assert_eq!(
        fee.get::<String, _>("note"),
        "Deposit retained (cancellation)"
    );
}

#[tokio::test]
async fn cancel_reservation_returns_invalid_state_when_booking_is_not_booked() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-CANCEL").await.unwrap();
    seed_booked_reservation(&pool, "B-CAS-CANCEL", "R-CAS-CANCEL")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'active' WHERE id = ?")
        .bind("B-CAS-CANCEL")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::cancel_reservation(&pool, "B-CAS-CANCEL")
        .await
        .expect_err("stale reservation should fail");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[tokio::test]
async fn do_create_reservation_returns_service_booking_and_leaves_room_vacant() {
    let pool = test_pool().await;
    seed_room(&pool, "R162").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000).await.unwrap();

    let ctx = cmd("create_reservation", "idem-do-create-reservation");
    let booking =
        reservations::do_create_reservation(&pool, None, &ctx, minimal_reservation_request("R162"))
            .await
            .unwrap();

    assert_eq!(booking.room_id, "R162");
    assert_eq!(booking.status, "booked");
    assert_eq!(booking.total_price, 1_200_000);
    assert_eq!(booking.paid_amount, 50_000);

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R162")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    assert_calendar_rows(&pool, &booking.id, "booked", 2).await;
}

#[tokio::test]
async fn do_cancel_reservation_cleans_legacy_booked_room_state() {
    let pool = test_pool().await;
    seed_room(&pool, "R163").await.unwrap();
    seed_booked_reservation(&pool, "B163", "R163")
        .await
        .unwrap();

    sqlx::query("UPDATE rooms SET status = 'booked' WHERE id = ?")
        .bind("R163")
        .execute(&pool)
        .await
        .unwrap();

    let ctx = cmd("cancel_reservation", "idem-do-cancel-reservation");
    let response = reservations::do_cancel_reservation(&pool, None, &ctx, "B163")
        .await
        .unwrap();
    assert!(response.ok);
    assert_eq!(response.booking_id, "B163");

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R163")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let remaining_calendar: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B163")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_calendar.0, 0);
}

#[tokio::test]
async fn confirm_reservation_reprices_and_marks_room_occupied() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B164", "R164", 600_000)
        .await
        .unwrap();

    let today = Local::now().date_naive();
    let scheduled_checkin = today + Duration::days(2);
    let scheduled_checkout = today + Duration::days(5);
    let scheduled_checkin_str = scheduled_checkin.format("%Y-%m-%d").to_string();
    let scheduled_checkout_str = scheduled_checkout.format("%Y-%m-%d").to_string();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?
         WHERE id = ?",
    )
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(3_i64)
    .bind(1_800_000)
    .bind("B164")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind("B164")
        .execute(&pool)
        .await
        .unwrap();

    let mut date = scheduled_checkin;
    while date < scheduled_checkout {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, 'booked')",
        )
        .bind("R164")
        .bind(date.format("%Y-%m-%d").to_string())
        .bind("B164")
        .execute(&pool)
        .await
        .unwrap();
        date += Duration::days(1);
    }

    let booking = reservation_lifecycle::confirm_reservation(&pool, "B164")
        .await
        .unwrap();

    assert_eq!(booking.status, "active");
    assert_eq!(booking.paid_amount, 50_000);
    assert_eq!(booking.expected_checkout, scheduled_checkout_str);
    assert_eq!(booking.nights, 5);
    assert_eq!(booking.total_price, 3_000_000);
    assert!(booking.check_in_at.contains('T'));

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R164")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "occupied");

    let calendar_rows = sqlx::query(
        "SELECT date, status FROM room_calendar WHERE booking_id = ? ORDER BY date ASC",
    )
    .bind("B164")
    .fetch_all(&pool)
    .await
    .unwrap();
    let actual_dates: Vec<String> = calendar_rows.iter().map(|row| row.get("date")).collect();
    let actual_statuses: Vec<String> = calendar_rows.iter().map(|row| row.get("status")).collect();
    let expected_dates: Vec<String> = (0..5)
        .map(|offset| {
            (today + Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    assert_eq!(actual_dates, expected_dates);
    assert!(actual_statuses.iter().all(|status| status == "occupied"));

    let charge = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND type = 'charge' LIMIT 1",
    )
    .bind("B164")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge.get::<String, _>("type"), "charge");
    assert_eq!(charge.get::<i64, _>("amount"), 3_000_000);
    assert_eq!(charge.get::<String, _>("note"), "Room charge (reservation)");
}

#[tokio::test]
async fn confirm_reservation_rejects_no_show_calendar_rows() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B165", "R165", 600_000)
        .await
        .unwrap();

    sqlx::query("UPDATE room_calendar SET status = ? WHERE booking_id = ?")
        .bind("no_show")
        .bind("B165")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::confirm_reservation(&pool, "B165")
        .await
        .unwrap_err();

    assert!(matches!(
        &error,
        crate::domain::booking::BookingError::Conflict(_)
    ));
    assert!(error.to_string().contains("B165"));
}

#[tokio::test]
async fn confirm_reservation_returns_invalid_state_when_booking_is_not_booked() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-CONFIRM").await.unwrap();
    seed_booked_reservation(&pool, "B-CAS-CONFIRM", "R-CAS-CONFIRM")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B-CAS-CONFIRM")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::confirm_reservation(&pool, "B-CAS-CONFIRM")
        .await
        .expect_err("stale reservation should fail");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[tokio::test]
async fn confirm_reservation_late_arrival_persists_effective_checkout() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B165A", "R165A", 600_000)
        .await
        .unwrap();

    let today = Local::now().date_naive();
    let scheduled_checkin = today - Duration::days(2);
    let scheduled_checkout = today;
    let scheduled_checkin_str = scheduled_checkin.format("%Y-%m-%d").to_string();
    let scheduled_checkout_str = scheduled_checkout.format("%Y-%m-%d").to_string();
    let effective_checkout_str = (today + Duration::days(1)).format("%Y-%m-%d").to_string();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?
         WHERE id = ?",
    )
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(2_i64)
    .bind(1_200_000)
    .bind("B165A")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind("B165A")
        .execute(&pool)
        .await
        .unwrap();

    let booking = reservation_lifecycle::confirm_reservation(&pool, "B165A")
        .await
        .unwrap();

    assert_eq!(booking.status, "active");
    assert_eq!(booking.nights, 1);
    assert_eq!(booking.expected_checkout, effective_checkout_str);
    assert_eq!(booking.total_price, 600_000);

    let calendar_rows = sqlx::query(
        "SELECT date, status FROM room_calendar WHERE booking_id = ? ORDER BY date ASC",
    )
    .bind("B165A")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_rows.len(), 1);
    assert_eq!(
        calendar_rows[0].get::<String, _>("date"),
        today.format("%Y-%m-%d").to_string()
    );
    assert_eq!(calendar_rows[0].get::<String, _>("status"), "occupied");
}

#[tokio::test]
async fn confirm_reservation_preserves_extra_precheckin_payment() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B165B", "R165B", 600_000)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
         VALUES (?, ?, ?, 'payment', ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind("B165B")
    .bind(25_000)
    .bind("Extra pre-check-in payment")
    .bind("2026-04-15T10:00:00+07:00")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("UPDATE bookings SET paid_amount = ? WHERE id = ?")
        .bind(75_000)
        .bind("B165B")
        .execute(&pool)
        .await
        .unwrap();

    let booking = reservation_lifecycle::confirm_reservation(&pool, "B165B")
        .await
        .unwrap();

    assert_eq!(booking.paid_amount, 75_000);
}

#[tokio::test]
async fn modify_reservation_rewrites_booked_calendar_range() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B166", "R166", 600_000)
        .await
        .unwrap();

    let booking = reservation_lifecycle::modify_reservation(
        &pool,
        reservation_modify_request("B166", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .unwrap();

    assert_eq!(booking.status, "booked");
    assert_eq!(booking.check_in_at, "2026-04-23");
    assert_eq!(booking.expected_checkout, "2026-04-26");
    assert_eq!(booking.nights, 3);
    assert_eq!(booking.total_price, 1_800_000);

    let booking_row =
        sqlx::query("SELECT scheduled_checkin, scheduled_checkout FROM bookings WHERE id = ?")
            .bind("B166")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        booking_row.get::<Option<String>, _>("scheduled_checkin"),
        Some("2026-04-23".to_string())
    );
    assert_eq!(
        booking_row.get::<Option<String>, _>("scheduled_checkout"),
        Some("2026-04-26".to_string())
    );

    let calendar_rows = sqlx::query(
        "SELECT date, status FROM room_calendar WHERE booking_id = ? ORDER BY date ASC",
    )
    .bind("B166")
    .fetch_all(&pool)
    .await
    .unwrap();
    let actual_dates: Vec<String> = calendar_rows.iter().map(|row| row.get("date")).collect();
    let actual_statuses: Vec<String> = calendar_rows.iter().map(|row| row.get("status")).collect();
    assert_eq!(
        actual_dates,
        vec![
            "2026-04-23".to_string(),
            "2026-04-24".to_string(),
            "2026-04-25".to_string(),
        ]
    );
    assert!(actual_statuses.iter().all(|status| status == "booked"));
}

#[tokio::test]
async fn modify_reservation_rejects_inconsistent_nights_input() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B166A", "R166A", 600_000)
        .await
        .unwrap();

    let error = reservation_lifecycle::modify_reservation(
        &pool,
        crate::models::ModifyReservationRequest {
            booking_id: "B166A".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-26".to_string(),
            new_nights: 2,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        crate::domain::booking::BookingError::Validation(_)
    ));
}

#[tokio::test]
async fn modify_reservation_returns_invalid_state_when_booking_is_not_booked() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-MOD").await.unwrap();
    seed_booked_reservation(&pool, "B-CAS-MOD", "R-CAS-MOD")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B-CAS-MOD")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::modify_reservation(
        &pool,
        reservation_modify_request("B-CAS-MOD", "2026-04-24", "2026-04-26", 2),
    )
    .await
    .expect_err("stale reservation should fail");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[tokio::test]
async fn do_modify_reservation_returns_service_booking_without_app_handle() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B167", "R167", 600_000)
        .await
        .unwrap();

    let ctx = cmd("modify_reservation", "idem-do-modify-reservation");
    let booking = reservations::do_modify_reservation(
        &pool,
        None,
        &ctx,
        reservation_modify_request("B167", "2026-04-24", "2026-04-26", 2),
    )
    .await
    .unwrap();

    assert_eq!(booking.status, "booked");
    assert_eq!(booking.check_in_at, "2026-04-24");
    assert_eq!(booking.expected_checkout, "2026-04-26");
    assert_eq!(booking.nights, 2);
    assert_eq!(booking.total_price, 1_200_000);

    assert_calendar_rows(&pool, "B167", "booked", 2).await;
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
    seed_room(&pool, "R-SMOKE-STAY")
        .await
        .expect("seed stay room");
    seed_pricing_rule(&pool, "standard", 250_000)
        .await
        .expect("seed stay pricing");

    let check_in_ctx = cmd_with_request(
        "check_in",
        "req-smoke-stay-checkin",
        "idem-smoke-stay-checkin",
    );
    let check_in_req = checkin_req("R-SMOKE-STAY").paid(50_000).build();

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
    assert_room_status(&pool, "R-SMOKE-STAY", "cleaning").await;
    assert_housekeeping_rows(&pool, "R-SMOKE-STAY", "needs_cleaning", 1).await;
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
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT paid_amount FROM bookings WHERE id = ?")
            .bind(&booking_id)
            .fetch_one(&pool)
            .await
            .expect("paid amount after checkout"),
        750_000
    );
    assert_single_outbox_event(&pool, &check_out_ctx, "booking.checked_out").await;
}

#[tokio::test]
async fn check_in_idempotent_retry_replays_and_does_not_duplicate_rows() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CHECKIN-IDEM").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000).await.unwrap();
    let ctx = cmd_with_request("check_in", "req-checkin-idem", "idem-checkin-1");
    let first_req = checkin_req("R-CHECKIN-IDEM").paid(50_000).build();
    let second_req = checkin_req("R-CHECKIN-IDEM").paid(50_000).build();

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

    let paid_amount: i64 = sqlx::query_scalar("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind(booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(paid_amount, 50_000);

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
    seed_room(&pool, "R-CHECKIN-HASH").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000).await.unwrap();
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
    });
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();

    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds in-flight row");

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
    let first_req = checkout_req(
        "B-CHECKOUT-IDEM",
        CheckoutSettlementMode::BookedNights,
        1_000_000,
    );
    let second_req = checkout_req(
        "B-CHECKOUT-IDEM",
        CheckoutSettlementMode::BookedNights,
        1_000_000,
    );
    let ctx = cmd_with_request("check_out", "req-checkout-idem", "idem-checkout-1");

    let first = stay_lifecycle::check_out_idempotent(&pool, &ctx, first_req)
        .await
        .unwrap();
    let second = stay_lifecycle::check_out_idempotent(&pool, &ctx, second_req)
        .await
        .unwrap();

    assert_replayed_pair(&first, &second);

    let housekeeping_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE room_id = ?")
            .bind("R-CHECKOUT-IDEM")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(housekeeping_count, 1);

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
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();

    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds in-flight row");

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

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B410".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 18, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 1);
    assert_eq!(preview.recommended_total, 500_000);

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req(
            "B410",
            CheckoutSettlementMode::ActualNights,
            preview.recommended_total,
        ),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 18, 0, 0)
            .single()
            .unwrap(),
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

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B411".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 5);
    assert_eq!(preview.recommended_total, 2_500_000);

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req(
            "B411",
            CheckoutSettlementMode::BookedNights,
            preview.recommended_total,
        ),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
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

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B413".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 1);

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B413", CheckoutSettlementMode::BookedNights, 0),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
            .single()
            .unwrap(),
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

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B414".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 2);
    assert_eq!(preview.recommended_total, 1_000_000);

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req(
            "B414",
            CheckoutSettlementMode::ActualNights,
            preview.recommended_total,
        ),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
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

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B415", CheckoutSettlementMode::Hourly, 500_000),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 10, 0, 0)
            .single()
            .unwrap(),
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

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B419", CheckoutSettlementMode::Hourly, 500_000),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
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

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B416".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.recommended_total, 1_000_000);

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B416", CheckoutSettlementMode::ActualNights, 800_000),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
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

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B417", CheckoutSettlementMode::ActualNights, 800_000),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
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

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B418", CheckoutSettlementMode::BookedNights, 2_500_000),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
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

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
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
    assert_eq!(booking.get::<i64, _>("paid_amount"), 2_500_000);
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

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req("B420", CheckoutSettlementMode::ActualNights, 75_000),
        chrono::Local
            .with_ymd_and_hms(2026, 4, 21, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B420")
        .fetch_one(&pool)
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
    assert_eq!(booking.get::<i64, _>("paid_amount"), ledger_total);
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

    let error = stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B412".to_string(),
            settlement_mode: CheckoutSettlementMode::Hourly,
            final_total: 500_000,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("refund"));
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

    let charge_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND note = ?")
            .bind("B-EXT-IDEM")
            .bind("Extended stay +1 night")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(charge_count, 1);
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
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();

    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds in-flight row");

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

#[tokio::test]
async fn revenue_queries_use_recognized_room_revenue_and_ignore_payments() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B301", "R301")
        .await
        .unwrap();
    seed_transaction(
        &pool,
        "B301",
        250_000,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B301",
        120_000,
        "payment",
        "Cash received",
        "2026-04-15T10:05:00+07:00",
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B301", 50_000, "2026-04-15T11:00:00+07:00")
        .await
        .unwrap();

    let dashboard = revenue_queries::load_dashboard_stats_for_date(&pool, "2026-04-15")
        .await
        .unwrap();
    let stats = revenue_queries::load_revenue_stats(
        &pool,
        "2026-04-15T00:00:00+07:00",
        "2026-04-15T23:59:59+07:00",
    )
    .await
    .unwrap();

    assert_eq!(dashboard.revenue_today, 300_000);
    assert_eq!(stats.total_revenue, 300_000);
    assert_eq!(stats.rooms_sold, 1);
    assert_eq!(stats.daily_revenue.len(), 1);
    assert_eq!(stats.daily_revenue[0].date, "2026-04-15");
    assert_eq!(stats.daily_revenue[0].revenue, 300_000);
}

#[tokio::test]
async fn analytics_breakdowns_reconcile_to_total_revenue() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B302", "R302")
        .await
        .unwrap();
    seed_transaction(
        &pool,
        "B302",
        250_000,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B302", 25_000, "2026-04-15T12:00:00+07:00")
        .await
        .unwrap();

    let analytics = revenue_queries::load_analytics(&pool, "2026-04-15", "2026-04-15", 1)
        .await
        .unwrap();

    assert_eq!(analytics.total_revenue, 275_000);
    assert_eq!(analytics.occupancy_rate, 100.0);
    assert_eq!(analytics.adr, 250_000.0);
    assert_eq!(analytics.revpar, 250_000.0);
    assert_eq!(analytics.daily_revenue.len(), 1);
    assert_eq!(analytics.revenue_by_source.len(), 1);
    assert_eq!(analytics.revenue_by_source[0].name, "walk-in");
    assert_eq!(analytics.revenue_by_source[0].value, 275_000);
    assert_eq!(analytics.top_rooms.len(), 1);
    assert_eq!(analytics.top_rooms[0].room, "R302");
    assert_eq!(analytics.top_rooms[0].revenue, 275_000);
}

#[tokio::test]
async fn revenue_queries_include_cancellation_fees_in_recognized_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R305").await.unwrap();
    seed_booked_reservation(&pool, "B305", "R305")
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B305")
        .execute(&pool)
        .await
        .unwrap();
    seed_transaction(
        &pool,
        "B305",
        50_000,
        "cancellation_fee",
        "Retained deposit",
        "2026-04-15T14:00:00+07:00",
    )
    .await
    .unwrap();

    let stats = revenue_queries::load_revenue_stats(
        &pool,
        "2026-04-15T00:00:00+07:00",
        "2026-04-15T23:59:59+07:00",
    )
    .await
    .unwrap();
    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-01", "2026-04-30")
        .await
        .unwrap();
    let cancelled_row = export_rows.iter().find(|row| row.id == "B305").unwrap();

    assert_eq!(stats.total_revenue, 50_000);
    assert_eq!(cancelled_row.charge_total, 0);
    assert_eq!(cancelled_row.cancellation_fee_total, 50_000);
    assert_eq!(cancelled_row.recognized_revenue, 50_000);
}

#[tokio::test]
async fn revenue_queries_use_local_rfc3339_booking_dates_for_business_day() {
    let pool = test_pool().await;
    seed_room(&pool, "R430").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B430",
        "R430",
        "2026-05-06T00:30:00+07:00",
        "2026-05-07T00:30:00+07:00",
        1,
        250_000,
        Some(0),
    )
    .await
    .unwrap();

    let stats = revenue_queries::load_revenue_stats(&pool, "2026-05-06", "2026-05-06")
        .await
        .unwrap();
    let total_revenue = revenue_queries::load_total_revenue(&pool, "2026-05-06", "2026-05-06")
        .await
        .unwrap();

    assert_eq!(stats.total_revenue, 250_000);
    assert_eq!(stats.rooms_sold, 1);
    assert_eq!(total_revenue, 250_000);
}

#[tokio::test]
async fn night_audit_snapshot_uses_local_rfc3339_booking_dates_for_occupancy() {
    let pool = test_pool().await;
    seed_room(&pool, "R431").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B431",
        "R431",
        "2026-05-06T00:30:00+07:00",
        "2026-05-07T00:30:00+07:00",
        1,
        250_000,
        Some(0),
    )
    .await
    .unwrap();

    let audit = audit_queries::load_night_audit_snapshot(&pool, "2026-05-06")
        .await
        .unwrap();

    assert_eq!(audit.room_revenue, 250_000);
    assert_eq!(audit.rooms_sold, 1);
    assert_eq!(audit.occupancy_pct, 100.0);
}

#[tokio::test]
async fn folio_and_cancellation_revenue_use_local_rfc3339_created_dates() {
    let pool = test_pool().await;
    seed_room(&pool, "R432").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B432",
        "R432",
        "2026-05-06",
        "2026-05-07",
        1,
        250_000,
        Some(0),
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B432", 40_000, "2026-05-06T00:30:00+07:00")
        .await
        .unwrap();
    seed_transaction(
        &pool,
        "B432",
        50_000,
        "cancellation_fee",
        "Retained deposit",
        "2026-05-06T00:30:00+07:00",
    )
    .await
    .unwrap();

    let folio_revenue = revenue_queries::load_folio_revenue(&pool, "2026-05-06", "2026-05-06")
        .await
        .unwrap();
    let cancellation_fee_revenue =
        revenue_queries::load_cancellation_fee_revenue(&pool, "2026-05-06", "2026-05-06")
            .await
            .unwrap();

    assert_eq!(folio_revenue, 40_000);
    assert_eq!(cancellation_fee_revenue, 50_000);
}

#[tokio::test]
async fn same_day_checkout_settlement_counts_one_room_sold_and_full_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R420").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B420",
        "R420",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

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
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B420")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B420",
        250_000,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B420",
        -1_750_000,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let stats = revenue_queries::load_revenue_stats(&pool, "2026-04-20", "2026-04-20")
        .await
        .unwrap();
    let audit = audit_queries::load_night_audit_snapshot(&pool, "2026-04-20")
        .await
        .unwrap();

    assert_eq!(stats.total_revenue, 500_000);
    assert_eq!(stats.rooms_sold, 1);
    assert_eq!(audit.room_revenue, 500_000);
    assert_eq!(audit.rooms_sold, 1);
}

#[tokio::test]
async fn booked_nights_settlement_uses_reporting_checkout_for_financial_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R421").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B421",
        "R421",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(2_500_000),
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET status = 'checked_out',
             actual_checkout = '2026-04-22T09:00:00+07:00',
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(r#"{"checkout_settlement":{"mode":"booked_nights","reporting_checkout":"2026-04-25","settled_nights":5,"settled_total":2500000}}"#)
    .bind("B421")
    .execute(&pool)
    .await
    .unwrap();

    let revenue = revenue_queries::load_room_revenue(&pool, "2026-04-20", "2026-04-24")
        .await
        .unwrap();

    assert_eq!(revenue, 2_500_000);
}

#[tokio::test]
async fn checkout_settlement_updates_booking_export_rows() {
    let pool = test_pool().await;
    seed_room(&pool, "R422").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B422",
        "R422",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

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
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B422")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B422",
        2_500_000,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B422",
        -2_000_000,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-01", "2026-04-30")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B422").unwrap();

    assert_eq!(row.room_price, 500_000);
    assert_eq!(row.charge_total, 500_000);
    assert_eq!(row.recognized_revenue, 500_000);
}

#[tokio::test]
async fn checkout_settlement_export_rows_follow_reporting_checkout_boundary() {
    let pool = test_pool().await;
    seed_room(&pool, "R423").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B423",
        "R423",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

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
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B423")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B423",
        2_500_000,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B423",
        -2_000_000,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-21", "2026-04-21")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B423").unwrap();

    assert_eq!(row.expected_checkout, "2026-04-21");
    assert_eq!(row.actual_checkout, "2026-04-20T18:00:00+07:00");
}

#[tokio::test]
async fn checkout_settlement_export_rows_exclude_original_checkin_window_after_shift() {
    let pool = test_pool().await;
    seed_room(&pool, "R424").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B424",
        "R424",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000,
        Some(0),
    )
    .await
    .unwrap();

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
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B424")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B424",
        2_500_000,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B424",
        -2_000_000,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-20", "2026-04-20")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B424");

    assert!(row.is_none());
}

#[tokio::test]
async fn cancellation_fee_export_uses_transaction_period_when_checkin_is_future() {
    let pool = test_pool().await;
    seed_room(&pool, "R425").await.unwrap();
    seed_booked_reservation(&pool, "B425", "R425")
        .await
        .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = '2026-05-20',
             expected_checkout = '2026-05-22',
             scheduled_checkin = '2026-05-20',
             scheduled_checkout = '2026-05-22',
             status = 'cancelled'
         WHERE id = ?",
    )
    .bind("B425")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B425",
        50_000,
        "cancellation_fee",
        "Retained deposit",
        "2026-04-15T14:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-15", "2026-04-15")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B425").unwrap();

    assert_eq!(row.cancellation_fee_total, 50_000);
    assert_eq!(row.recognized_revenue, 50_000);
}

#[tokio::test]
async fn booking_export_includes_local_rfc3339_non_checkout_checkin_date() {
    let pool = test_pool().await;
    seed_room(&pool, "R426").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B426",
        "R426",
        "2026-05-06T00:30:00+07:00",
        "2026-05-07T00:30:00+07:00",
        1,
        250_000,
        Some(0),
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-05-06", "2026-05-06")
        .await
        .unwrap();

    let row = export_rows.iter().find(|row| row.id == "B426").unwrap();
    assert_eq!(row.check_in_at, "2026-05-06T00:30:00+07:00");
}

#[tokio::test]
async fn booking_export_includes_local_rfc3339_cancellation_fee_date() {
    let pool = test_pool().await;
    seed_room(&pool, "R427").await.unwrap();
    seed_booked_reservation(&pool, "B427", "R427")
        .await
        .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = '2026-05-20',
             expected_checkout = '2026-05-22',
             scheduled_checkin = '2026-05-20',
             scheduled_checkout = '2026-05-22',
             status = 'cancelled'
         WHERE id = ?",
    )
    .bind("B427")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B427",
        50_000,
        "cancellation_fee",
        "Retained deposit",
        "2026-05-06T00:30:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-05-06", "2026-05-06")
        .await
        .unwrap();

    let row = export_rows.iter().find(|row| row.id == "B427").unwrap();
    assert_eq!(row.cancellation_fee_total, 50_000);
    assert_eq!(row.recognized_revenue, 50_000);
}

#[tokio::test]
async fn run_night_audit_uses_canonical_room_and_folio_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R303").await.unwrap();
    seed_active_booking(&pool, "B303", "R303").await.unwrap();
    sqlx::query(
        "UPDATE bookings
         SET nights = 2, total_price = 500000, expected_checkout = '2026-04-17T10:00:00+07:00'
         WHERE id = ?",
    )
    .bind("B303")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B303",
        500_000,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B303",
        90_000,
        "payment",
        "Cash received",
        "2026-04-16T10:05:00+07:00",
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B303", 40_000, "2026-04-16T13:00:00+07:00")
        .await
        .unwrap();
    seed_expense(&pool, "electricity", 10_000, "2026-04-16")
        .await
        .unwrap();

    let log = audit_service::run_night_audit(
        &pool,
        "2026-04-16",
        Some("Checked and closed".to_string()),
        "admin-1",
    )
    .await
    .unwrap();

    assert_eq!(log.audit_date, "2026-04-16");
    assert_eq!(log.room_revenue, 250_000);
    assert_eq!(log.folio_revenue, 40_000);
    assert_eq!(log.total_revenue, 290_000);
    assert_eq!(log.total_expenses, 10_000);
    assert_eq!(log.rooms_sold, 1);
    assert_eq!(log.total_rooms, 1);

    let audited: i32 = sqlx::query_scalar("SELECT is_audited FROM bookings WHERE id = ?")
        .bind("B303")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn billing_and_export_queries_preserve_canonical_revenue_columns() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B304", "R304")
        .await
        .unwrap();
    seed_transaction(
        &pool,
        "B304",
        250_000,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();

    let line = add_folio_line(
        &pool,
        "B304",
        "laundry",
        "Laundry bundle",
        35_000,
        Some("staff-1"),
    )
    .await
    .unwrap();
    let folio_lines = billing_queries::list_folio_lines(&pool, "B304")
        .await
        .unwrap();
    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-01", "2026-04-30")
        .await
        .unwrap();

    assert_eq!(line.amount, 35_000);
    assert_eq!(folio_lines.len(), 1);
    assert_eq!(folio_lines[0].category, "laundry");
    assert_eq!(export_rows.len(), 1);
    assert_eq!(export_rows[0].room_price, 250_000);
    assert_eq!(export_rows[0].charge_total, 250_000);
    assert_eq!(export_rows[0].cancellation_fee_total, 0);
    assert_eq!(export_rows[0].folio_total, 35_000);
    assert_eq!(export_rows[0].recognized_revenue, 285_000);
}

#[tokio::test]
async fn add_folio_line_idempotent_retry_replays_and_does_not_duplicate_row() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-1", "FOLIO-IDEM-1")
        .await
        .unwrap();
    let ctx = cmd_with_request("add_folio_line", "req-folio-idem-1", "idem-folio-line-1");

    let first = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-1",
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");
    let second = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-1",
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("retry replays");

    assert_replayed_pair(&first, &second);
    assert_eq!(first.response["id"], second.response["id"]);

    assert_eq!(
        folio_line_count_for_key(&pool, "add_folio_line:idem-folio-line-1").await,
        1
    );
    assert_single_outbox_event(&pool, &ctx, "folio.line_added").await;
}

#[tokio::test]
async fn add_folio_line_idempotent_accepts_uuid_booking_id_in_safe_ledger_metadata() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-UUID").await.unwrap();
    let booking_id = uuid::Uuid::new_v4().to_string();
    seed_active_booking(&pool, &booking_id, "FOLIO-IDEM-UUID")
        .await
        .unwrap();
    let ctx = cmd_with_request(
        "add_folio_line",
        "req-folio-idem-uuid",
        "idem-folio-line-uuid",
    );

    let result = add_folio_line_idempotent(
        &pool,
        &ctx,
        &booking_id,
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("uuid booking id should not be rejected by safe ledger metadata");

    assert!(!result.replayed);
    assert_eq!(
        result.response["booking_id"].as_str(),
        Some(booking_id.as_str())
    );

    let primary_aggregate_key: Option<String> = sqlx::query_scalar(
        "SELECT primary_aggregate_key FROM command_idempotency
         WHERE command_name = 'add_folio_line' AND idempotency_key = ?",
    )
    .bind("idem-folio-line-uuid")
    .fetch_one(&pool)
    .await
    .unwrap();
    let expected_primary_aggregate_key = format!("booking:{booking_id}");
    assert_eq!(
        primary_aggregate_key.as_deref(),
        Some(expected_primary_aggregate_key.as_str())
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_metadata_is_sanitized_and_contains_lock_keys() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-META", "FOLIO-IDEM-META")
        .await
        .unwrap();
    let ctx = cmd_with_request(
        "add_folio_line",
        "req-folio-idem-meta",
        "idem-folio-line-meta",
    );

    add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-META",
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("folio line succeeds");

    let row = sqlx::query(
        "SELECT intent_json, lock_keys_json, primary_aggregate_key
         FROM command_idempotency
         WHERE command_name = 'add_folio_line' AND idempotency_key = ?",
    )
    .bind("idem-folio-line-meta")
    .fetch_one(&pool)
    .await
    .unwrap();
    let intent_json = row.get::<String, _>("intent_json");
    let lock_keys_json = row.get::<String, _>("lock_keys_json");

    assert!(intent_json.contains("\"schema\":\"folio.add_line.v1\""));
    assert!(intent_json.contains("\"category_present\":true"));
    assert!(intent_json.contains("\"description_present\":true"));
    assert!(intent_json.contains("\"created_by_present\":true"));
    assert!(!intent_json.contains("Laundry bundle"));
    assert!(!intent_json.contains("staff-1"));
    assert_eq!(
        row.get::<String, _>("primary_aggregate_key"),
        "booking:B-FOLIO-IDEM-META"
    );
    assert_eq!(
        lock_keys_json,
        r#"["booking:B-FOLIO-IDEM-META","folio:B-FOLIO-IDEM-META"]"#
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-2", "FOLIO-IDEM-2")
        .await
        .unwrap();
    let ctx = cmd("add_folio_line", "idem-folio-line-2");

    add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-2",
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-2",
        "laundry",
        "Different description",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect_err("same key with different payload conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_same_key_changed_amount_conflicts() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-AMOUNT", "FOLIO-IDEM-AMOUNT")
        .await
        .unwrap();
    let ctx = cmd("add_folio_line", "idem-folio-line-amount");

    add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-AMOUNT",
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-AMOUNT",
        "laundry",
        "Laundry bundle",
        30_000,
        Some("staff-1"),
    )
    .await
    .expect_err("same key with changed amount conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_replay_returns_stored_snapshot() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-3", "FOLIO-IDEM-3")
        .await
        .unwrap();
    let ctx = cmd("add_folio_line", "idem-folio-line-3");

    let first = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-3",
        "laundry",
        "Snapshot line",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");
    let line_id = first.response["id"].as_str().unwrap().to_string();
    let first_amount = first.response["amount"].as_i64().unwrap();

    sqlx::query("UPDATE folio_lines SET amount = 99999 WHERE id = ?")
        .bind(&line_id)
        .execute(&pool)
        .await
        .unwrap();

    let replay = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-3",
        "laundry",
        "Snapshot line",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect("replay succeeds");

    assert!(replay.replayed);
    assert_eq!(replay.response["amount"].as_i64(), Some(first_amount));
    assert_ne!(replay.response["amount"].as_i64(), Some(99_999));
}

#[tokio::test]
async fn add_folio_line_idempotent_duplicate_seeded_live_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-INFLIGHT", "FOLIO-IDEM-INFLIGHT")
        .await
        .unwrap();
    let ctx = cmd("add_folio_line", "idem-folio-line-inflight");
    seed_live_in_progress_command(
        &pool,
        "add_folio_line",
        "idem-folio-line-inflight",
        &add_folio_line_hash_payload_for_test(
            "B-FOLIO-IDEM-INFLIGHT",
            "laundry",
            "Laundry bundle",
            25_000,
            Some("staff-1"),
        ),
    )
    .await;

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-INFLIGHT",
        "laundry",
        "Laundry bundle",
        25_000,
        Some("staff-1"),
    )
    .await
    .expect_err("duplicate live in-flight command should conflict");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_rejects_blank_key_before_any_write() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-4", "FOLIO-IDEM-4")
        .await
        .unwrap();

    let error = crate::command_idempotency::WriteCommandContext::for_scoped_command(
        "req-folio-idem-4",
        "   ",
        "add_folio_line",
    )
    .expect_err("blank idempotency key rejected");
    assert_eq!(
        error.code,
        crate::app_error::codes::IDEMPOTENCY_KEY_REQUIRED
    );

    assert_eq!(
        folio_line_count_for_booking(&pool, "B-FOLIO-IDEM-4").await,
        0
    );
    assert_eq!(
        command_claim_count_by_request(&pool, "add_folio_line", "req-folio-idem-4").await,
        0
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_invalid_amount_does_not_consume_claim_or_ordinal() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-5", "FOLIO-IDEM-5")
        .await
        .unwrap();
    let ctx = cmd("add_folio_line", "idem-folio-line-5");

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-5",
        "laundry",
        "Invalid amount",
        0,
        Some("staff-1"),
    )
    .await
    .expect_err("invalid amount rejected");
    assert_eq!(error.code, crate::app_error::codes::BOOKING_INVALID_STATE);

    assert_eq!(
        folio_line_count_for_booking(&pool, "B-FOLIO-IDEM-5").await,
        0
    );
    assert_eq!(
        command_claim_count(&pool, "add_folio_line", "idem-folio-line-5").await,
        0
    );

    let success = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-5",
        "laundry",
        "Valid amount",
        15_000,
        Some("staff-1"),
    )
    .await
    .expect("valid amount succeeds");
    assert!(!success.replayed);

    let row = sqlx::query(
        "SELECT origin_idempotency_key, origin_line_ordinal
         FROM folio_lines
         WHERE booking_id = ?",
    )
    .bind("B-FOLIO-IDEM-5")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<String, _>("origin_idempotency_key"),
        "add_folio_line:idem-folio-line-5"
    );
    assert_eq!(row.get::<i64, _>("origin_line_ordinal"), 0);
}

#[tokio::test]
async fn add_folio_line_idempotent_unsafe_amount_does_not_consume_claim_or_write() {
    let pool = test_pool().await;
    seed_room(&pool, "B-FOLIO-FRACTION").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-FRACTION", "B-FOLIO-FRACTION")
        .await
        .unwrap();
    let ctx = cmd_with_request("add_folio_line", "req-folio-unsafe", "idem-folio-unsafe");

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-FRACTION",
        "laundry",
        "Unsafe amount",
        MAX_TRANSPORT_SAFE_MONEY_VND + 1,
        Some("staff-1"),
    )
    .await
    .expect_err("unsafe amount rejected");
    assert_eq!(error.code, crate::app_error::codes::BOOKING_INVALID_STATE);

    assert_eq!(
        folio_line_count_for_booking(&pool, "B-FOLIO-FRACTION").await,
        0
    );
    assert_eq!(
        command_claim_count(&pool, "add_folio_line", "idem-folio-unsafe").await,
        0
    );
}

#[tokio::test]
async fn folio_line_insert_rolls_back_with_parent_transaction() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-1", "FOLIO-1")
        .await
        .unwrap();

    let mut tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    crate::repositories::booking::folio_repository::insert_folio_line_tx(
        &mut tx,
        "B-FOLIO-1",
        "laundry",
        "Rollback laundry",
        25_000,
        Some("staff-1"),
        "2026-04-15T12:00:00+07:00",
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE booking_id = ?")
        .bind("B-FOLIO-1")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count, 0);
}

#[tokio::test]
async fn insert_folio_line_with_origin_writes_origin_key_and_ordinal() {
    let pool = test_pool().await;
    seed_room(&pool, "R503").await.unwrap();
    let booking_id = seed_booking_for_origin_tests(&pool, "R503").await.unwrap();
    let origin = OriginSideEffect::new("idem-folio-1", 0).unwrap();

    let mut tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    crate::repositories::booking::folio_repository::insert_folio_line_with_origin_tx(
        &mut tx,
        &booking_id,
        "laundry",
        "Laundry with origin",
        25_000,
        Some("staff-1"),
        "2026-04-27T08:00:00+07:00",
        &origin,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let row = sqlx::query(
        "SELECT origin_idempotency_key, origin_line_ordinal
         FROM folio_lines
         WHERE booking_id = ? AND description = ?",
    )
    .bind(&booking_id)
    .bind("Laundry with origin")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        row.get::<String, _>("origin_idempotency_key"),
        "idem-folio-1"
    );
    assert_eq!(row.get::<i64, _>("origin_line_ordinal"), 0);
}

#[tokio::test]
async fn duplicate_folio_origin_is_blocked_by_unique_origin_ordinal() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-ORIGIN-DUP", "R-FOLIO-ORIGIN-DUP")
        .await
        .unwrap();
    let origin = OriginSideEffect::new("origin-duplicate-folio", 0).unwrap();

    let mut first_tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    crate::repositories::booking::folio_repository::insert_folio_line_with_origin_tx(
        &mut first_tx,
        "B-FOLIO-ORIGIN-DUP",
        "laundry",
        "Duplicate origin folio",
        25_000,
        Some("staff-1"),
        "2026-04-27T08:00:00+07:00",
        &origin,
    )
    .await
    .unwrap();
    first_tx.commit().await.unwrap();

    let mut second_tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    let duplicate =
        crate::repositories::booking::folio_repository::insert_folio_line_with_origin_tx(
            &mut second_tx,
            "B-FOLIO-ORIGIN-DUP",
            "laundry",
            "Duplicate origin folio",
            25_000,
            Some("staff-1"),
            "2026-04-27T08:00:00+07:00",
            &origin,
        )
        .await;
    assert!(duplicate.is_err(), "duplicate folio origin must fail");
    second_tx.rollback().await.unwrap();

    assert_folio_origin(&pool, "origin-duplicate-folio", 0, 1).await;
}
