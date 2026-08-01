use super::prelude::*;

/// Khách đã ở 1 đêm (15/04), còn 2 đêm (16/04, 17/04). Hôm nay là 16/04.
///
/// R-OLD hạng `standard` 250.000/đêm, R-NEW hạng `deluxe` 350.000/đêm.
///
/// Hai phòng **bắt buộc khác loại**: pricing engine lấy giá theo `room_type`
/// (`load_stay_pricing_inputs_tx` → `load_fallback_base_price_tx(room_type)`),
/// không theo `rooms.base_price` của từng phòng. `seed_room` tạo mọi phòng cùng
/// `type = 'standard'`, nên nếu chỉ sửa `base_price` của một phòng thì chênh
/// lệch luôn ra 0 và test sẽ xanh giả.
async fn seed_stay_in_progress(pool: &sqlx::Pool<sqlx::Sqlite>) {
    seed_room(pool, "R-OLD").await.unwrap();
    seed_room(pool, "R-NEW").await.unwrap();
    sqlx::query("UPDATE rooms SET type = 'deluxe' WHERE id = 'R-NEW'")
        .execute(pool)
        .await
        .unwrap();
    seed_pricing_rule(pool, "standard", 250_000).await.unwrap();
    seed_pricing_rule(pool, "deluxe", 350_000).await.unwrap();

    seed_active_booking_with_terms(
        pool,
        "B-OPT",
        "R-OLD",
        "2026-04-15T10:00:00+07:00",
        "2026-04-18T10:00:00+07:00",
        3,
        750_000,
        Some(0),
    )
    .await
    .unwrap();

    // seed_active_booking chỉ tạo dòng 15/04; thêm hai đêm còn lại.
    for date in ["2026-04-16", "2026-04-17"] {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status)
             VALUES ('R-OLD', ?, 'B-OPT', 'occupied')",
        )
        .bind(date)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn load_options_splits_nights_at_today() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    let options = room_change::load_options(
        &pool,
        "B-OPT",
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(options.nights_stayed, 1);
    assert_eq!(options.nights_remaining, 2);
    assert_eq!(options.from_date, "2026-04-16");
    assert_eq!(options.to_date, "2026-04-17");
    assert_eq!(options.current_room_id, "R-OLD");
}

#[tokio::test]
async fn load_options_excludes_current_room_and_lists_free_ones() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    let options = room_change::load_options(
        &pool,
        "B-OPT",
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    let ids: Vec<&str> = options.rooms.iter().map(|r| r.room_id.as_str()).collect();
    assert!(!ids.contains(&"R-OLD"), "phòng hiện tại không được là phương án");
    assert!(ids.contains(&"R-NEW"));
}

#[tokio::test]
async fn load_options_hides_room_taken_on_any_remaining_night() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    // R-NEW bị khách khác giữ đúng một đêm trong dải còn lại.
    seed_active_booking_with_room(&pool, "B-OTHER", "R-BUSY")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status)
         VALUES ('R-NEW', '2026-04-17', 'B-OTHER', 'booked')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let options = room_change::load_options(
        &pool,
        "B-OPT",
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    let ids: Vec<&str> = options.rooms.iter().map(|r| r.room_id.as_str()).collect();
    assert!(!ids.contains(&"R-NEW"), "vướng một đêm là đủ để loại");
}

#[tokio::test]
async fn load_options_hides_a_dirty_room_when_the_guest_moves_in_tonight() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    sqlx::query("UPDATE rooms SET status = 'cleaning' WHERE id = 'R-NEW'")
        .execute(&pool)
        .await
        .unwrap();

    let options = room_change::load_options(
        &pool,
        "B-OPT",
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    let ids: Vec<&str> = options.rooms.iter().map(|r| r.room_id.as_str()).collect();
    assert!(!ids.contains(&"R-NEW"), "không đưa khách vào phòng chưa dọn");
}

#[tokio::test]
async fn change_room_keeps_past_nights_on_the_old_room() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    let old_nights: Vec<String> = sqlx::query_scalar(
        "SELECT date FROM room_calendar WHERE booking_id = 'B-OPT' AND room_id = 'R-OLD' ORDER BY date",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(old_nights, vec!["2026-04-15".to_string()]);

    let new_nights: Vec<String> = sqlx::query_scalar(
        "SELECT date FROM room_calendar WHERE booking_id = 'B-OPT' AND room_id = 'R-NEW' ORDER BY date",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        new_nights,
        vec!["2026-04-16".to_string(), "2026-04-17".to_string()]
    );

    let room_id: String = sqlx::query_scalar("SELECT room_id FROM bookings WHERE id = 'B-OPT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room_id, "R-NEW");
}

#[tokio::test]
async fn change_room_sends_the_old_room_to_cleaning() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    let old_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-OLD'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_status, "cleaning");

    let new_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-NEW'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(new_status, "occupied");

    let task_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM housekeeping WHERE room_id = 'R-OLD' AND status = 'needs_cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(task_count, 1, "phòng cũ phải sinh đúng một phiếu dọn");
}

#[tokio::test]
async fn change_room_rejects_the_same_room() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    let result = room_change::change_room(
        &pool,
        "B-OPT",
        "R-OLD",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await;

    assert!(matches!(result, Err(BookingError::Validation(_))));
}

#[tokio::test]
async fn change_room_charges_only_the_difference_for_remaining_nights() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    // R-NEW hạng deluxe, đắt hơn 100.000/đêm. Còn 2 đêm ⇒ chênh 200.000.

    let booking = room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        false,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(booking.total_price, 750_000 + 200_000);

    let charge: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = 'B-OPT' AND type = 'charge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge, 200_000);
}

#[tokio::test]
async fn change_room_with_keep_price_records_no_money_at_all() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    let booking = room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(booking.total_price, 750_000, "giữ giá cũ thì tổng không đổi");

    let count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = 'B-OPT' AND type = 'charge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn change_room_to_a_cheaper_room_records_a_negative_charge() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    // Hạ giá hạng deluxe xuống dưới standard: 150.000 so với 250.000.
    // Sửa `pricing_rules`, không sửa `rooms.base_price` — engine đọc bảng này.
    sqlx::query("UPDATE pricing_rules SET daily_rate = 150000 WHERE room_type = 'deluxe'")
        .execute(&pool)
        .await
        .unwrap();

    let booking = room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        false,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(booking.total_price, 750_000 - 200_000);

    let charge: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = 'B-OPT' AND type = 'charge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge, -200_000);
}

#[tokio::test]
async fn change_room_appends_the_move_to_booking_notes() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        Some("máy lạnh hỏng"),
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    let notes: String = sqlx::query_scalar("SELECT notes FROM bookings WHERE id = 'B-OPT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(notes.starts_with("seed booking |"), "ghi chú cũ phải còn");
    assert!(notes.contains("Chuyển phòng R-OLD → R-NEW ngày 16/04/2026"));
    assert!(notes.contains("máy lạnh hỏng"));
}
