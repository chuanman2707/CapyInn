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
