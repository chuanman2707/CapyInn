use super::prelude::*;

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
