use super::prelude::*;

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
    assert_eq!(
        housekeeping_count, 0,
        "trả phòng đoàn không được sinh phiếu dọn nào nữa"
    );
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
    seed_live_in_progress_command(&pool, &ctx.command_name, &ctx.idempotency_key, &payload).await;

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

async fn seed_user_group_checkin_idempotent(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    ctx: &crate::command_idempotency::WriteCommandContext,
    req: crate::models::GroupCheckinRequest,
) -> crate::app_error::CommandResult<
    crate::command_idempotency::IdempotentCommandResult<serde_json::Value>,
> {
    group_lifecycle::group_checkin_idempotent(pool, Some("seed-user".to_string()), ctx, req).await
}

#[tokio::test]
async fn group_checkin_idempotent_normalizes_room_order_and_assigns_payment_ordinals() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI601", "GI602"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-1");

    let first = seed_user_group_checkin_idempotent(
        &pool,
        &ctx,
        rich_group_checkin_request(&["GI602", "GI601"], "GI602", Some(100_001)),
    )
    .await
    .expect("first group checkin succeeds");
    let second = seed_user_group_checkin_idempotent(
        &pool,
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

    seed_user_group_checkin_idempotent(&pool, &ctx, req)
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

    let first = seed_user_group_checkin_idempotent(
        &pool,
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

    let retry = seed_user_group_checkin_idempotent(
        &pool,
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

    let result = seed_user_group_checkin_idempotent(
        &pool,
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

    let first = seed_user_group_checkin_idempotent(
        &pool,
        &ctx,
        rich_group_checkin_request(&["GI620", "GI621"], "GI620", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");
    let replay = seed_user_group_checkin_idempotent(
        &pool,
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

    let error = seed_user_group_checkin_idempotent(&pool, &ctx, req)
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
    let zero_paid = seed_user_group_checkin_idempotent(
        &pool,
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

    let first = seed_user_group_checkin_idempotent(
        &pool,
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

    let replay = seed_user_group_checkin_idempotent(
        &pool,
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

    seed_user_group_checkin_idempotent(
        &pool,
        &ctx,
        rich_group_checkin_request(&["GI640", "GI641"], "GI640", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");

    let mut changed = rich_group_checkin_request(&["GI640", "GI641"], "GI640", Some(100_000));
    changed.nights = 3;
    let error = seed_user_group_checkin_idempotent(&pool, &ctx, changed)
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

    seed_user_group_checkin_idempotent(
        &pool,
        &ctx,
        rich_group_checkin_request(&["GI642", "GI643"], "GI642", Some(100_000)),
    )
    .await
    .expect("first group checkin succeeds");

    let mut changed = rich_group_checkin_request(&["GI642", "GI643"], "GI642", Some(100_000));
    changed.guests_per_room.get_mut("GI642").unwrap()[0].full_name =
        "Changed Guest Name".to_string();
    let error = seed_user_group_checkin_idempotent(&pool, &ctx, changed)
        .await
        .expect_err("same key with changed guest name conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

/// `rate_override_per_room` ảnh hưởng trực tiếp tới `total_price` của từng
/// phòng như mọi trường khác trong hash — thiếu nó thì hai lượt nhận đoàn
/// dưới cùng idempotency key nhưng GIÁ khác nhau sẽ lặp lại kết quả CŨ (giá
/// cũ, sai) trong im lặng thay vì bị báo `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`
/// — một bug tiền bạc, cùng lý do
/// `create_reservation_same_key_different_rate_override_conflicts`
/// (reservation_idempotency.rs, Task 14) pin cho reservation.
#[tokio::test]
async fn group_checkin_same_key_different_rate_override_conflicts() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI692", "GI693"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-rate-conflict");

    seed_user_group_checkin_idempotent(
        &pool,
        &ctx,
        rich_group_checkin_request(&["GI692", "GI693"], "GI692", None),
    )
    .await
    .expect("first group checkin succeeds");

    let mut changed = rich_group_checkin_request(&["GI692", "GI693"], "GI692", None);
    changed
        .rate_override_per_room
        .insert("GI692".to_string(), 350_000);

    let error = seed_user_group_checkin_idempotent(&pool, &ctx, changed)
        .await
        .expect_err("same key with a different rate override conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

/// Kiểm biên giá tay phải chạy TRƯỚC khi `group_checkin_idempotent` ghi bất cứ
/// gì — không chỉ qua wrapper test-only `group_checkin` (đã pin bởi
/// `group_checkin_rejects_duplicate_room_ids` cho validation khác của cùng
/// hàm), mà qua đúng đường lệnh thật production gọi
/// (`commands::groups::group_checkin` → `group_checkin_idempotent`).
#[tokio::test]
async fn group_checkin_idempotent_rejects_out_of_bounds_rate_override() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["GI694", "GI695"], 250_000)
        .await
        .unwrap();
    let ctx = cmd("group_checkin", "idem-group-checkin-bad-rate");

    let mut req = rich_group_checkin_request(&["GI694", "GI695"], "GI694", None);
    req.rate_override_per_room
        .insert("GI694".to_string(), -500_000);

    let error = seed_user_group_checkin_idempotent(&pool, &ctx, req)
        .await
        .expect_err("out-of-bounds rate override is rejected before any write");

    assert_eq!(error.code, crate::app_error::codes::BOOKING_INVALID_STATE);

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        group_count, 0,
        "request rejected before any group is created"
    );
}

#[tokio::test]
async fn group_checkout_idempotent_reads_rooms_after_taking_the_group_lock() {
    let pool = test_pool().await;
    seed_room(&pool, "R931").await.unwrap();
    seed_room(&pool, "R932").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();

    let checkin_ctx = cmd("group_checkin", "idem-group-race-checkin");
    let group = group_lifecycle::group_checkin_idempotent(
        &pool,
        None,
        &checkin_ctx,
        rich_group_checkin_request(&["R931"], "R931", None),
    )
    .await
    .expect("group check-in succeeds");
    let group_id = group.response["id"]
        .as_str()
        .expect("group id in response")
        .to_string();

    let booking_id: String =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? LIMIT 1")
            .bind(&group_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let blocker = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::group_key(&group_id).unwrap()])
        .await
        .expect("blocker lock");

    let ctx = cmd("group_checkout", "idem-group-race-checkout");
    let pool_for_task = pool.clone();
    let req = GroupCheckoutRequest {
        group_id: group_id.clone(),
        booking_ids: vec![booking_id.clone()],
        final_paid: None,
    };
    let command = tokio::spawn(async move {
        group_lifecycle::group_checkout_idempotent(&pool_for_task, &ctx, req).await
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !command.is_finished(),
        "lệnh phải kẹt ở pha 1, chưa được phép đọc phòng của các booking"
    );

    sqlx::query("UPDATE room_calendar SET room_id = 'R932' WHERE booking_id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET room_id = 'R932' WHERE id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R932'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R931'")
        .execute(&pool)
        .await
        .unwrap();

    drop(blocker);

    command
        .await
        .expect("task joins")
        .expect("group checkout phải thành công trên phòng mới");

    assert_room_status(&pool, "R932", "vacant").await;
}

#[tokio::test]
async fn group_checkout_idempotent_reads_rooms_after_taking_the_booking_locks() {
    let pool = test_pool().await;
    seed_room(&pool, "R941").await.unwrap();
    seed_room(&pool, "R942").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();

    let checkin_ctx = cmd("group_checkin", "idem-group-race2-checkin");
    let group = group_lifecycle::group_checkin_idempotent(
        &pool,
        None,
        &checkin_ctx,
        rich_group_checkin_request(&["R941"], "R941", None),
    )
    .await
    .expect("group check-in succeeds");
    let group_id = group.response["id"]
        .as_str()
        .expect("group id in response")
        .to_string();

    let booking_id: String =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? LIMIT 1")
            .bind(&group_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    // Chặn bằng khoá BOOKING, không phải khoá group — đúng bộ khoá mà
    // change_room sẽ cầm. Lệnh phải qua được pha 1 rồi kẹt ở pha 2.
    let blocker = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::booking_key(&booking_id).unwrap()])
        .await
        .expect("blocker lock");

    let ctx = cmd("group_checkout", "idem-group-race2-checkout");
    let pool_for_task = pool.clone();
    let req = GroupCheckoutRequest {
        group_id: group_id.clone(),
        booking_ids: vec![booking_id.clone()],
        final_paid: None,
    };
    let command = tokio::spawn(async move {
        group_lifecycle::group_checkout_idempotent(&pool_for_task, &ctx, req).await
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !command.is_finished(),
        "lệnh phải kẹt ở pha 2, chưa được phép đọc phòng"
    );

    sqlx::query("UPDATE room_calendar SET room_id = 'R942' WHERE booking_id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET room_id = 'R942' WHERE id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R942'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R941'")
        .execute(&pool)
        .await
        .unwrap();

    drop(blocker);

    command
        .await
        .expect("task joins")
        .expect("group checkout phải thành công trên phòng mới");

    assert_room_status(&pool, "R942", "vacant").await;
}
