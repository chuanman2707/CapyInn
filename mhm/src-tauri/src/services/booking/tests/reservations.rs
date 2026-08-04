use super::prelude::*;

#[tokio::test]
async fn create_reservation_blocks_calendar_and_posts_deposit() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R160", 600_000).await.unwrap();

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
    seed_room_with_price(&pool, "R160A", 600_000).await.unwrap();

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
            guests: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        crate::domain::booking::BookingError::Validation(_)
    ));
}

/// Trước đây ngày đi chỉ đọc được, suy từ một ô số đêm mang `max={90}` — 90
/// đêm là trần cứng theo cấu trúc. Ô đó bị xoá để cho gõ tay ngày đi, và
/// không gì thay thế trần cũ ở tầng service: một lỗi gõ năm (`2036` thay vì
/// `2026`) tạo ra một đặt phòng khoảng 3650 đêm, khoá phòng đó cả thập kỷ.
/// Test này khôi phục kỳ vọng: vượt trần phải bị từ chối, bất kể ai gọi vào
/// (form, gateway, agent).
#[tokio::test]
async fn create_reservation_rejects_nights_beyond_the_ceiling() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R160B", 600_000).await.unwrap();

    let error = reservation_lifecycle::create_reservation(
        &pool,
        CreateReservationRequest {
            room_id: "R160B".to_string(),
            guest_name: "Nguyen Van C".to_string(),
            guest_phone: Some("0900000002".to_string()),
            guest_doc_number: Some("079000000002".to_string()),
            check_in_date: "2026-08-08".to_string(),
            check_out_date: "2036-08-08".to_string(), // ~3650 đêm — lỗi gõ năm
            nights: 3653,
            deposit_amount: None,
            source: Some("phone".to_string()),
            notes: None,
            guests: None,
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
async fn modify_reservation_rejects_nights_beyond_the_ceiling() {
    let pool = test_pool().await;
    seed_booked_reservation_with_price(&pool, "B166B", "R166B", 600_000)
        .await
        .unwrap();

    let error = reservation_lifecycle::modify_reservation(
        &pool,
        reservation_modify_request("B166B", "2026-08-08", "2036-08-08", 3653),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        crate::domain::booking::BookingError::Validation(_)
    ));
}

#[tokio::test]
async fn reservation_lifecycle_smoke_covers_confirm_and_cancel_paths() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["R-SMOKE-CONFIRM", "R-SMOKE-CANCEL"], 600_000)
        .await
        .expect("seed smoke rooms/pricing");

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
            guests: None,
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
    assert_room_status(&pool, "R-SMOKE-CONFIRM", "vacant").await;
    assert_calendar_rows(&pool, &confirm_booking_id, "booked", 2).await;
    assert_eq!(
        transaction_sum(&pool, &confirm_booking_id, "deposit", None).await,
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
    assert_booking_status(&pool, &confirm_booking_id, "active").await;
    assert_room_status(&pool, "R-SMOKE-CONFIRM", "occupied").await;
    assert_calendar_rows(&pool, &confirm_booking_id, "occupied", confirmed_nights).await;
    assert_eq!(
        transaction_sum(&pool, &confirm_booking_id, "charge", None).await,
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
    assert_booking_status(&pool, &cancel_booking_id, "cancelled").await;
    assert_room_status(&pool, "R-SMOKE-CANCEL", "vacant").await;
    assert_eq!(
        calendar_count_for_booking(&pool, &cancel_booking_id).await,
        0
    );
    assert_eq!(
        transaction_sum(&pool, &cancel_booking_id, "cancellation_fee", None).await,
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

    assert_eq!(calendar_count_for_booking(&pool, "B161").await, 0);

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
    seed_room_with_price(&pool, "R162", 600_000).await.unwrap();

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

    assert_eq!(calendar_count_for_booking(&pool, "B163").await, 0);
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
    // Booking đang ở giữ `expected_checkout` dạng RFC3339 để `extend_stay` đọc được.
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(&booking.expected_checkout)
            .unwrap()
            .date_naive(),
        scheduled_checkout
    );
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
    // Booking đang ở giữ `expected_checkout` dạng RFC3339 để `extend_stay` đọc được.
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(&booking.expected_checkout)
            .unwrap()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string(),
        effective_checkout_str
    );
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
            new_guests: None,
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
async fn create_reservation_charges_extra_guests() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R300", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R300".to_string(),
            guest_name: "Nguyễn Nhật Huy".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: Some("phone".to_string()),
            notes: None,
            guests: Some(4),
        },
        None,
    )
    .await
    .unwrap();

    let row = sqlx::query("SELECT total_price, guests FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(row.get::<i64, _>("total_price"), 1_200_000);
    assert_eq!(row.get::<Option<i32>, _>("guests"), Some(4));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn create_reservation_without_guests_prices_like_before() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R301", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R301".to_string(),
            guest_name: "Khách cũ".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: None,
        },
        None,
    )
    .await
    .unwrap();

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(total, 1_000_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn modify_reservation_keeps_the_extra_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R310", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R310".to_string(),
            guest_name: "Nguyễn Nhật Huy".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: Some(4),
        },
        None,
    )
    .await
    .unwrap();

    // Dời sang 3 đêm, không nói gì về số khách.
    let booking = reservation_lifecycle::modify_reservation_tx(
        &mut tx,
        ModifyReservationRequest {
            booking_id: booking_id.clone(),
            new_check_in_date: "2026-08-10".to_string(),
            new_check_out_date: "2026-08-13".to_string(),
            new_nights: 3,
            new_guests: None,
        },
        "R310",
    )
    .await
    .unwrap();

    // 600.000₫/đêm × 3 đêm — không tụt về 500.000₫.
    assert_eq!(booking.total_price, 1_800_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn confirm_reservation_keeps_the_extra_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R311", 500_000, 2, 50_000)
        .await
        .unwrap();
    // `seed_room_with_guest_pricing` là helper seed DUY NHẤT không chèn dòng
    // `pricing_rules` nào, nên phòng nó tạo rơi về `PricingRule::default()` —
    // vốn mang phụ thu cuối tuần 20%. Kỳ ở dưới đây bắt đầu từ HÔM NAY thật và
    // không đổi được: `confirm_reservation_tx` định giá lại từ lúc xác nhận đến
    // ngày đi, nên đẩy ngày đến ra một mốc cố định chỉ kéo dài kỳ ở. Vậy nên
    // chạy vào thứ Sáu/Bảy/Chủ nhật là có đêm cuối tuần lọt vào, cộng thêm
    // 500.000 × 20% = 100.000 và kỳ vọng 1.200.000 dưới kia đỏ — 3 ngày trong 7.
    //
    // Bảng giá tường minh này khoá phụ thu cuối tuần về 0 đúng như mọi helper
    // seed khác vẫn làm, giữ nguyên giá ngày 500.000. Test nói về phụ thu thêm
    // người; cuối tuần là biến lạ, không phải thứ nó kiểm.
    seed_pricing_rule(&pool, "standard", 500_000).await.unwrap();

    let today = Local::now().date_naive();
    let check_in = today.format("%Y-%m-%d").to_string();
    let check_out = (today + Duration::days(2)).format("%Y-%m-%d").to_string();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R311".to_string(),
            guest_name: "Nguyễn Nhật Huy".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: check_in,
            check_out_date: check_out,
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: Some(4),
        },
        None,
    )
    .await
    .unwrap();

    let booking = reservation_lifecycle::confirm_reservation_tx(&mut tx, &booking_id, "R311", None)
        .await
        .unwrap();

    assert_eq!(booking.total_price, 1_200_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn modify_reservation_prefers_explicit_new_guests_over_stored_count() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R320", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R320".to_string(),
            guest_name: "Trần Thị Mai".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            // Stored count starts within max_guests (2), so no extra-person fee applies yet.
            guests: Some(2),
        },
        None,
    )
    .await
    .unwrap();

    // Explicit new_guests (4) differs from the stored count (2). If `.or()` were
    // reversed to `reservation.guests.or(req.new_guests)`, the stored 2 would win
    // and the total would stay at 1_000_000 instead of reflecting the caller's 4.
    let booking = reservation_lifecycle::modify_reservation_tx(
        &mut tx,
        ModifyReservationRequest {
            booking_id: booking_id.clone(),
            new_check_in_date: "2026-08-10".to_string(),
            new_check_out_date: "2026-08-12".to_string(),
            new_nights: 2,
            new_guests: Some(4),
        },
        "R320",
    )
    .await
    .unwrap();

    // (500.000₫ base + 2 khách vượt mốc × 50.000₫)/đêm × 2 đêm = 1.200.000₫.
    assert_eq!(booking.total_price, 1_200_000);

    let row = sqlx::query("SELECT total_price, guests FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("total_price"), 1_200_000);
    assert_eq!(row.get::<Option<i32>, _>("guests"), Some(4));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn modify_reservation_keeps_null_guests_null() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R321", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R321".to_string(),
            guest_name: "Lê Văn Nam".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: None,
        },
        None,
    )
    .await
    .unwrap();

    let created_row = sqlx::query("SELECT total_price, guests FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(created_row.get::<i64, _>("total_price"), 1_000_000);
    assert_eq!(created_row.get::<Option<i32>, _>("guests"), None);

    let booking = reservation_lifecycle::modify_reservation_tx(
        &mut tx,
        ModifyReservationRequest {
            booking_id: booking_id.clone(),
            new_check_in_date: "2026-08-10".to_string(),
            new_check_out_date: "2026-08-12".to_string(),
            new_nights: 2,
            new_guests: None,
        },
        "R321",
    )
    .await
    .unwrap();

    assert_eq!(booking.total_price, 1_000_000);

    let row = sqlx::query("SELECT total_price, guests FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("total_price"), 1_000_000);
    assert_eq!(row.get::<Option<i32>, _>("guests"), None);

    tx.rollback().await.unwrap();
}
