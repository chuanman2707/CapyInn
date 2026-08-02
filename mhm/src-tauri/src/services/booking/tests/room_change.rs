use super::prelude::*;
use crate::services::booking::invoice_generation;

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

/// Đặt trước nhận phòng 20/04, hôm nay 16/04. Chưa đêm nào trôi qua.
async fn seed_future_reservation(pool: &sqlx::Pool<sqlx::Sqlite>) {
    seed_room(pool, "R-RES-OLD").await.unwrap();
    seed_room(pool, "R-RES-NEW").await.unwrap();
    // Khác loại phòng thì mới có chênh lệch — xem ghi chú ở `seed_stay_in_progress`.
    sqlx::query("UPDATE rooms SET type = 'deluxe' WHERE id = 'R-RES-NEW'")
        .execute(pool)
        .await
        .unwrap();
    seed_pricing_rule(pool, "standard", 250_000).await.unwrap();
    seed_pricing_rule(pool, "deluxe", 350_000).await.unwrap();

    seed_active_booking_with_terms(
        pool,
        "B-RES",
        "R-RES-OLD",
        "2026-04-20T14:00:00+07:00",
        "2026-04-22T12:00:00+07:00",
        2,
        500_000,
        Some(0),
    )
    .await
    .unwrap();

    sqlx::query("UPDATE bookings SET status = 'booked' WHERE id = 'B-RES'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-RES-OLD'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM room_calendar WHERE booking_id = 'B-RES'")
        .execute(pool)
        .await
        .unwrap();
    for date in ["2026-04-20", "2026-04-21"] {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status)
             VALUES ('R-RES-OLD', ?, 'B-RES', 'booked')",
        )
        .bind(date)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn changing_a_reservation_leaves_both_rooms_alone() {
    let pool = test_pool().await;
    seed_future_reservation(&pool).await;

    room_change::change_room(
        &pool,
        "B-RES",
        "R-RES-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    for room_id in ["R-RES-OLD", "R-RES-NEW"] {
        let status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = ?")
            .bind(room_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "vacant", "{room_id} không được đổi trạng thái");
    }

    let tasks: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tasks, 0, "đặt trước không sinh phiếu dọn");
}

#[tokio::test]
async fn reservation_prices_the_real_stay_not_from_today() {
    let pool = test_pool().await;
    seed_future_reservation(&pool).await;
    // R-RES-NEW hạng deluxe, đắt hơn 100.000/đêm. Kỳ ở thật 2 đêm ⇒ chênh 200.000.

    let booking = room_change::change_room(
        &pool,
        "B-RES",
        "R-RES-NEW",
        false,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap();

    // Chênh 100.000 × 2 đêm thật, không phải 6 đêm tính từ 16/04.
    assert_eq!(booking.total_price, 500_000 + 200_000);
}

#[tokio::test]
async fn change_room_rejects_a_room_that_is_too_small() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    sqlx::query("UPDATE rooms SET max_guests = 1 WHERE id = 'R-NEW'")
        .execute(&pool)
        .await
        .unwrap();
    // Thêm khách thứ hai vào booking.
    sqlx::query(
        "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
         VALUES ('g2', 'domestic', 'Khách 2', 'DOC-g2', '2026-04-15T10:00:00+07:00')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES ('B-OPT', 'g2')")
        .execute(&pool)
        .await
        .unwrap();

    let result = room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await;

    assert!(matches!(result, Err(BookingError::Validation(_))));
}

#[tokio::test]
async fn change_room_rejects_a_room_taken_mid_transaction() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    seed_active_booking_with_room(&pool, "B-RIVAL", "R-RIVAL")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status)
         VALUES ('R-NEW', '2026-04-17', 'B-RIVAL', 'booked')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let result = room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await;

    assert!(matches!(result, Err(BookingError::Conflict(_))));

    // Không được đổi gì cả.
    let room_id: String = sqlx::query_scalar("SELECT room_id FROM bookings WHERE id = 'B-OPT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room_id, "R-OLD");
}

#[tokio::test]
async fn change_room_keeps_the_group_link_intact() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    sqlx::query(
        "INSERT INTO booking_groups (id, group_name, master_booking_id, organizer_name,
                                     total_rooms, status, created_at)
         VALUES ('G1', 'Đoàn thử', 'B-OPT', 'Trưởng đoàn', 1, 'active', '2026-04-15T10:00:00+07:00')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE bookings SET group_id = 'G1' WHERE id = 'B-OPT'")
        .execute(&pool)
        .await
        .unwrap();

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

    let group_id: String = sqlx::query_scalar("SELECT group_id FROM bookings WHERE id = 'B-OPT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_id, "G1");

    let master: String =
        sqlx::query_scalar("SELECT master_booking_id FROM booking_groups WHERE id = 'G1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(master, "B-OPT");
}

/// Same shape as `seed_stay_in_progress` (one night already stayed, two nights
/// remaining, R-OLD standard vs. R-NEW deluxe at a 100.000/night difference),
/// but with dates computed from the real wall clock instead of hard-coded to
/// April 2026.
///
/// `change_room_idempotent` — unlike the pool-level `change_room` test helper
/// above, which takes `today` as an explicit argument — resolves `today` from
/// `Local::now()` (mirroring `extend_stay_idempotent`/`check_out_idempotent`),
/// because the Tauri command has no `today` input either. A fixture pinned to
/// a fixed past date would leave zero `room_calendar` rows with
/// `date >= today` once the real date moves past it, and
/// `change_room_tx` would reject the move outright.
async fn seed_stay_in_progress_around_today(pool: &sqlx::Pool<sqlx::Sqlite>) {
    let today = Local::now().date_naive();
    let yesterday = today - Duration::days(1);
    let tomorrow = today + Duration::days(1);

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
        &format!("{}T10:00:00+07:00", yesterday.format("%Y-%m-%d")),
        &format!(
            "{}T10:00:00+07:00",
            (tomorrow + Duration::days(1)).format("%Y-%m-%d")
        ),
        3,
        750_000,
        Some(0),
    )
    .await
    .unwrap();

    // `seed_active_booking` hard-codes a single 2026-04-15 calendar row;
    // replace it with the three real nights of this stay.
    sqlx::query("DELETE FROM room_calendar WHERE booking_id = 'B-OPT'")
        .execute(pool)
        .await
        .unwrap();
    for date in [yesterday, today, tomorrow] {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status)
             VALUES ('R-OLD', ?, 'B-OPT', 'occupied')",
        )
        .bind(date.format("%Y-%m-%d").to_string())
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn change_room_idempotent_retry_does_not_charge_twice() {
    let pool = test_pool().await;
    seed_stay_in_progress_around_today(&pool).await;

    let ctx = cmd_with_request("change_room", "req-change-room-idem", "idem-room-change-1");

    let first = room_change::change_room_idempotent(&pool, &ctx, "B-OPT", "R-NEW", false, None)
        .await
        .unwrap();
    let second = room_change::change_room_idempotent(&pool, &ctx, "B-OPT", "R-NEW", false, None)
        .await
        .unwrap();

    assert_replayed_pair(&first, &second);

    let charges: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = 'B-OPT' AND type = 'charge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 1, "gọi lại cùng khoá không được tính tiền lần hai");

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = 'B-OPT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 950_000);

    assert_single_outbox_event(&pool, &ctx, "booking.room_changed").await;
}

#[tokio::test]
async fn invoice_splits_lines_per_room_after_a_move() {
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

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let labels: Vec<String> = invoice
        .pricing_breakdown
        .iter()
        .map(|line| line.label.clone())
        .collect();
    assert!(labels.iter().any(|l| l.contains("R-OLD") && l.contains("1 đêm")));
    assert!(labels.iter().any(|l| l.contains("R-NEW") && l.contains("2 đêm")));

    let sum: i64 = invoice.pricing_breakdown.iter().map(|l| l.amount).sum();
    assert_eq!(sum, invoice.subtotal, "tổng các dòng phải khớp subtotal");
}

#[tokio::test]
async fn invoice_keeps_one_line_when_no_move_happened() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        invoice.pricing_breakdown.len(),
        1,
        "chưa đổi phòng thì hoá đơn chỉ có một dòng, đúng như trước đây"
    );
}
