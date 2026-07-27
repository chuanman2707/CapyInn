use super::prelude::*;

async fn add_staff_laundry_line_idempotent(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    ctx: &crate::command_idempotency::WriteCommandContext,
    booking_id: &str,
    description: &str,
    amount: i64,
) -> crate::app_error::CommandResult<
    crate::command_idempotency::IdempotentCommandResult<serde_json::Value>,
> {
    add_folio_line_idempotent(
        pool,
        ctx,
        booking_id,
        "laundry",
        description,
        amount,
        Some("staff-1"),
    )
    .await
}

#[tokio::test]
async fn add_folio_line_idempotent_retry_replays_and_does_not_duplicate_row() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-IDEM-1", "FOLIO-IDEM-1")
        .await
        .unwrap();
    let ctx = cmd_with_request("add_folio_line", "req-folio-idem-1", "idem-folio-line-1");

    let first =
        add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-1", "Laundry bundle", 25_000)
            .await
            .expect("first folio line succeeds");
    let second =
        add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-1", "Laundry bundle", 25_000)
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

    let result =
        add_staff_laundry_line_idempotent(&pool, &ctx, &booking_id, "Laundry bundle", 25_000)
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

    add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-2", "Laundry bundle", 25_000)
        .await
        .expect("first folio line succeeds");

    let error = add_staff_laundry_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-2",
        "Different description",
        25_000,
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

    add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-AMOUNT", "Laundry bundle", 25_000)
        .await
        .expect("first folio line succeeds");

    let error = add_staff_laundry_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-AMOUNT",
        "Laundry bundle",
        30_000,
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

    let first =
        add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-3", "Snapshot line", 25_000)
            .await
            .expect("first folio line succeeds");
    let line_id = first.response["id"].as_str().unwrap().to_string();
    let first_amount = first.response["amount"].as_i64().unwrap();

    sqlx::query("UPDATE folio_lines SET amount = 99999 WHERE id = ?")
        .bind(&line_id)
        .execute(&pool)
        .await
        .unwrap();

    let replay =
        add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-3", "Snapshot line", 25_000)
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

    let error = add_staff_laundry_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-INFLIGHT",
        "Laundry bundle",
        25_000,
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

    let error =
        add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-5", "Invalid amount", 0)
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

    let success =
        add_staff_laundry_line_idempotent(&pool, &ctx, "B-FOLIO-IDEM-5", "Valid amount", 15_000)
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

    let error = add_staff_laundry_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-FRACTION",
        "Unsafe amount",
        MAX_TRANSPORT_SAFE_MONEY_VND + 1,
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

    let count = folio_line_count_for_booking(&pool, "B-FOLIO-1").await;

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
