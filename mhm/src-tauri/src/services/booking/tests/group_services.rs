use super::prelude::*;

async fn checked_in_group_for_service_tests(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    room_ids: &[&str],
    daily_rate: crate::money::MoneyVnd,
) -> crate::models::BookingGroup {
    seed_rooms_with_price(pool, room_ids, daily_rate)
        .await
        .unwrap();
    group_lifecycle::group_checkin(pool, None, group_checkin_req(room_ids))
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
