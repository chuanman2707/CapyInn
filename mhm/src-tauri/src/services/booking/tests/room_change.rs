use super::prelude::*;
use crate::services::booking::invoice_generation;

/// Both invoice renderers (`InvoicePDF.tsx`, `InvoiceDialog.tsx`) print
/// **every** breakdown line with its amount under a "PRICE BREAKDOWN" header
/// and then print "Subtotal" from `invoice.total` right underneath. A guest
/// therefore reads the lines as an itemisation of the subtotal: if they do not
/// add up to it, the printed document contradicts itself.
///
/// Deliberately sums *all* lines, not a `starts_with("Phòng ")` subset — a
/// filtered assertion cannot see a duplicated charge, which is exactly the
/// defect this guards against.
fn assert_breakdown_sums_to_subtotal(invoice: &crate::models::InvoiceData) {
    let lines: Vec<String> = invoice
        .pricing_breakdown
        .iter()
        .map(|line| format!("{} = {:?}", line.label, line.amount))
        .collect();
    let sum: i64 = invoice
        .pricing_breakdown
        .iter()
        .map(|line| line.amount)
        .sum();
    assert_eq!(
        sum, invoice.subtotal,
        "mọi dòng in ra phải cộng đúng bằng subtotal, không được thừa dòng nào: {lines:?}"
    );
}

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
    assert!(
        !ids.contains(&"R-OLD"),
        "phòng hiện tại không được là phương án"
    );
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
async fn load_options_hides_an_occupied_room_when_the_guest_moves_in_tonight() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R-NEW'")
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
    assert!(
        !ids.contains(&"R-NEW"),
        "không đưa khách vào phòng đang có người"
    );
}

/// `load_options` only applied the vacancy filter when `moving_in_today`. But
/// `change_room_tx` requires the new room to be `vacant` whenever the booking
/// is ACTIVE (see the final `UPDATE rooms ... WHERE status = 'vacant'` guard),
/// regardless of whether the first remaining night is tonight or later. An
/// active guest whose earliest movable night is tomorrow (not tonight) could
/// otherwise be offered an occupied room that the write then rejects.
///
/// Drop the 16/04 row so the only remaining night is 17/04 — `moving_in_today`
/// is false — while the booking stays ACTIVE.
#[tokio::test]
async fn load_options_requires_vacant_room_for_an_active_booking_moving_later() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    sqlx::query("DELETE FROM room_calendar WHERE booking_id = 'B-OPT' AND date = '2026-04-16'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R-NEW'")
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

    assert_eq!(
        options.from_date, "2026-04-17",
        "đêm chuyển đầu tiên phải là 17/04, không phải hôm nay — tiền đề của test"
    );
    let ids: Vec<&str> = options.rooms.iter().map(|r| r.room_id.as_str()).collect();
    assert!(
        !ids.contains(&"R-NEW"),
        "booking đang active thì phòng mới phải trống, dù đêm chuyển đầu tiên không phải tối nay"
    );
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
async fn change_room_frees_the_old_room_immediately() {
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
    assert_eq!(old_status, "vacant");

    let new_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-NEW'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(new_status, "occupied");

    let task_count: i32 =
        sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE room_id = 'R-OLD'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(task_count, 0, "đổi phòng không sinh phiếu dọn nào nữa");
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

    assert_eq!(
        booking.total_price, 750_000,
        "giữ giá cũ thì tổng không đổi"
    );

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
    assert!(labels
        .iter()
        .any(|l| l.contains("R-OLD") && l.contains("1 đêm")));
    assert!(labels
        .iter()
        .any(|l| l.contains("R-NEW") && l.contains("2 đêm")));

    assert_breakdown_sums_to_subtotal(&invoice);
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
    assert_breakdown_sums_to_subtotal(&invoice);
    assert_eq!(
        invoice.settlement_note, None,
        "hoá đơn một dòng không cần khối GHI CHÚ: dòng tiền duy nhất đã nói đúng điều đó"
    );
}

/// The decisive regression test: `check_out_tx` deletes every `room_calendar`
/// row for the booking (stay_lifecycle.rs), and in the real production
/// database invoices are always generated *after* checkout — so by the time
/// `generate_invoice_tx` runs, `room_calendar` has nothing left to group.
/// The split must come from `bookings.pricing_snapshot.room_stays`, written
/// by `change_room_tx` at move time, not from `room_calendar`.
#[tokio::test]
async fn move_then_real_checkout_still_splits_invoice_lines() {
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

    // Real checkout path, not a direct SQL fixture — this is what production
    // actually runs, and it wipes room_calendar as a side effect.
    stay_lifecycle::check_out(
        &pool,
        checkout_req("B-OPT", CheckoutSettlementMode::BookedNights, 750_000),
    )
    .await
    .unwrap();

    let calendar_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = 'B-OPT'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        calendar_rows, 0,
        "checkout phải xoá hết room_calendar — nếu còn dòng thì test này không còn đại diện cho path thật"
    );

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
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-OLD") && l.contains("1 đêm")),
        "labels: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-NEW") && l.contains("2 đêm")),
        "labels: {labels:?}"
    );

    assert_breakdown_sums_to_subtotal(&invoice);

    // The settlement wording ("Thanh toán theo số đêm đã đặt") must survive
    // the split — this is exactly the booking whose settlement is least
    // obvious (moved mid-stay), so losing the wording that explains the total
    // is worst here. It lands in `settlement_note`, NOT in
    // `pricing_breakdown`: as a money line it would be printed alongside the
    // room lines that already sum to subtotal, and the guest would read a
    // document adding up to twice its own stated subtotal.
    assert_eq!(
        invoice.settlement_note.as_deref(),
        Some("Thanh toán theo số đêm đã đặt"),
        "hoá đơn tách phòng phải giữ lời quyết toán ở settlement_note"
    );
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("Thanh toán theo số đêm đã đặt")),
        "lời quyết toán không được ở lại danh sách dòng tiền: {labels:?}"
    );

    // checkout_settlement must survive the pricing_snapshot merge alongside
    // room_stays — revenue_queries.rs reads it via json_extract and those
    // paths must keep resolving.
    let snapshot: String =
        sqlx::query_scalar("SELECT pricing_snapshot FROM bookings WHERE id = 'B-OPT'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let value: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert!(
        value.get("room_stays").is_some(),
        "room_stays phải sống sót qua checkout: {value}"
    );
    assert!(
        value.get("checkout_settlement").is_some(),
        "checkout_settlement phải còn nguyên: {value}"
    );

    let mode: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(pricing_snapshot, '$.checkout_settlement.mode') FROM bookings WHERE id = 'B-OPT'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mode.as_deref(), Some("booked_nights"));

    let reporting_checkout: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(pricing_snapshot, '$.checkout_settlement.reporting_checkout') FROM bookings WHERE id = 'B-OPT'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        reporting_checkout.is_some(),
        "revenue_queries.rs đọc path này qua json_extract, phải còn resolve được"
    );
}

/// Finding 1(a): `change_room_tx` snapshots `room_stays` at move time, but
/// nothing refreshes it afterwards. `extend_stay_tx` inserts new
/// `room_calendar` rows on the guest's *current* room without touching
/// `pricing_snapshot` — so a guest who moves and then extends ends up with a
/// snapshot that undercounts the current room's nights while `room_calendar`
/// (still intact — checkout has not run) holds the true split. The invoice
/// must prefer `room_calendar` over the stale snapshot whenever the calendar
/// still has rows for this booking.
#[tokio::test]
async fn move_then_extend_reflects_the_real_split_not_the_stale_snapshot() {
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
    // Snapshot right after the move: R-OLD × 1, R-NEW × 2 (3 nights total).

    // Extend twice — both extra nights land on R-NEW, the current room.
    // change_room_tx has no way to know about these; only room_calendar does.
    stay_lifecycle::extend_stay(&pool, "B-OPT").await.unwrap();
    stay_lifecycle::extend_stay(&pool, "B-OPT").await.unwrap();

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
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-OLD") && l.contains("1 đêm")),
        "labels: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-NEW") && l.contains("4 đêm")),
        "phòng hiện tại phải cộng dồn cả 2 đêm gia hạn (2 gốc + 2 gia hạn = 4), \
         không dừng lại ở snapshot 2 đêm chụp lúc chuyển phòng: {labels:?}"
    );

    assert_breakdown_sums_to_subtotal(&invoice);
}

/// Finding 1(b): the snapshot written at move time assumes the guest stays
/// every remaining booked night. An early checkout settles fewer nights than
/// that — `check_out_tx` must truncate `room_stays` to `settled_nights`,
/// walking rooms in occupancy order, not leave the move-time snapshot
/// (which oversells the new room) as the post-checkout source of truth.
///
/// 1 night stayed on R-OLD, 2 remaining nights moved to R-NEW (snapshot:
/// R-OLD × 1, R-NEW × 2 ⇒ 3). Guest actually leaves after 2 real nights
/// (ActualNights settlement, checkout 2 days after check-in) ⇒ settled_nights
/// = 2. Truncated: R-OLD keeps its 1 (fits under the 2-night budget), R-NEW
/// is capped to the 1 night left in the budget — not the 2 nights the stale
/// snapshot would still claim.
#[tokio::test]
async fn move_then_early_checkout_truncates_room_stays_to_settled_nights() {
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

    let checkout_at = Local
        .with_ymd_and_hms(2026, 4, 17, 9, 0, 0)
        .single()
        .unwrap();
    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B-OPT".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        checkout_at,
    )
    .await
    .unwrap();
    assert_eq!(
        preview.settled_nights, 2,
        "tiền đề của test: khách rời sau đúng 2 đêm thật"
    );

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req(
            "B-OPT",
            CheckoutSettlementMode::ActualNights,
            preview.recommended_total,
        ),
        checkout_at,
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
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-OLD") && l.contains("1 đêm")),
        "labels: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-NEW") && l.contains("1 đêm")),
        "R-NEW phải bị cắt xuống 1 đêm (ngân sách 2 đêm settled trừ 1 đêm R-OLD đã dùng), \
         không phải 2 đêm như snapshot lúc chuyển phòng còn nhớ: {labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("R-NEW") && l.contains("2 đêm")),
        "labels: {labels:?}"
    );

    assert_breakdown_sums_to_subtotal(&invoice);

    // The checkout_settlement key must still be intact and still readable by
    // revenue_queries.rs — the room_stays truncation must merge, not clobber.
    let mode: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(pricing_snapshot, '$.checkout_settlement.mode') FROM bookings WHERE id = 'B-OPT'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mode.as_deref(), Some("actual_nights"));
}

/// The existing split test (750.000 / 3 nights = 250.000 exactly) never
/// exercises the remainder-on-last-line branch. Pick a subtotal that does
/// not divide evenly across the split nights so integer truncation would
/// silently drop money if the remainder handling were wrong.
#[tokio::test]
async fn invoice_split_remainder_lands_on_last_line_for_non_divisible_subtotal() {
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

    // 1 night on R-OLD, 2 nights on R-NEW ⇒ 3 nights total. 100.000 / 3 does
    // not divide evenly (33.333,33...).
    sqlx::query("UPDATE bookings SET total_price = 100000 WHERE id = 'B-OPT'")
        .execute(&pool)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // The split replaces the settlement line, so the breakdown is exactly the
    // two room lines — nothing else may carry an amount.
    assert_eq!(invoice.pricing_breakdown.len(), 2);
    assert_eq!(invoice.pricing_breakdown[0].amount, 33_333);
    assert_eq!(invoice.pricing_breakdown[1].amount, 66_667);

    assert_breakdown_sums_to_subtotal(&invoice);
}

/// Finding A: the settlement wording is not allowed to just vanish when the
/// split takes over the money lines — it moves to `settlement_note`, which
/// both renderers print as a "GHI CHÚ" block. It must survive a re-read too:
/// `get_invoice` is what the UI calls for an already-issued invoice.
#[tokio::test]
async fn split_invoice_carries_the_settlement_wording_in_a_settlement_note() {
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

    stay_lifecycle::check_out(
        &pool,
        checkout_req("B-OPT", CheckoutSettlementMode::BookedNights, 750_000),
    )
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        invoice.settlement_note.as_deref(),
        Some("Thanh toán theo số đêm đã đặt"),
        "lời quyết toán phải đứng một mình trong settlement_note"
    );
    assert_breakdown_sums_to_subtotal(&invoice);

    let reread = invoice_generation::get_invoice(&pool, "B-OPT")
        .await
        .unwrap()
        .expect("hoá đơn vừa phát hành phải đọc lại được");
    assert_eq!(
        reread.settlement_note, invoice.settlement_note,
        "settlement_note phải sống sót qua vòng ghi/đọc lại — UI mở hoá đơn cũ bằng get_invoice"
    );
}

/// `invoices.notes` copies `bookings.notes` verbatim, and in the live database
/// that column holds internal front-desk shorthand: "Agoda thanh toan",
/// "cọc 600k", scribbles about who is paying. The renderers print
/// `settlement_note` and nothing else, so the guard has to be that no scrap of
/// the booking's own notes can ever reach that field — not merely that the
/// renderers currently happen to read the right one.
#[tokio::test]
async fn settlement_note_never_carries_the_bookings_internal_notes() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;
    // Shaped after real rows in the production database.
    let internal_note = "Agoda thanh toan | cọc 600k, chị Hằng thu";
    sqlx::query("UPDATE bookings SET notes = ? WHERE id = 'B-OPT'")
        .bind(internal_note)
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

    stay_lifecycle::check_out(
        &pool,
        checkout_req("B-OPT", CheckoutSettlementMode::BookedNights, 750_000),
    )
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let settlement_note = invoice.settlement_note.clone().unwrap_or_default();
    for fragment in ["Agoda", "cọc 600k", "chị Hằng", "Chuyển phòng"] {
        assert!(
            !settlement_note.contains(fragment),
            "ghi chú nội bộ ({fragment:?}) không được lọt vào phần in cho khách: {settlement_note:?}"
        );
    }
    assert_eq!(
        settlement_note, "Thanh toán theo số đêm đã đặt",
        "phần in cho khách chỉ chứa đúng lời quyết toán"
    );

    // `notes` itself is still carried on the invoice row — it is data the back
    // office may want; it is simply never rendered.
    assert!(
        invoice
            .notes
            .as_deref()
            .is_some_and(|notes| notes.starts_with(internal_note)),
        "notes vẫn giữ nguyên ghi chú của booking: {:?}",
        invoice.notes
    );

    assert_breakdown_sums_to_subtotal(&invoice);
}

/// An in-house guest can be handed an invoice — `RoomDetailPanel.tsx` and
/// `RoomDrawer.tsx` both offer it — and before checkout there is no
/// `checkout_settlement`, so the settlement label falls back to the English
/// developer string "3 night(s) x 250000d". That is tolerable as a breakdown
/// line (it has always been one) but not under a Vietnamese "GHI CHÚ" heading,
/// so `settlement_note` must stay empty until checkout gives it real wording.
#[tokio::test]
async fn a_pre_checkout_split_invoice_has_no_settlement_note() {
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

    // No check_out call: the guest is still in the room.
    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert!(
        invoice.pricing_breakdown.len() > 1,
        "tiền đề của test: hoá đơn này phải là loại tách theo phòng"
    );
    assert_eq!(
        invoice.settlement_note, None,
        "chưa check-out thì không có lời quyết toán tiếng Việt nào để in — \
         thà bỏ trống còn hơn in chuỗi tiếng Anh của lập trình viên"
    );
    assert_breakdown_sums_to_subtotal(&invoice);
}

/// Finding B: a guest who moves A → B and later back to A.
///
/// `room_calendar_stays_tx` used to `GROUP BY rc.room_id`, which collapsed the
/// two A stays into one row carrying A's *earliest* night. Walking that list,
/// `truncate_room_stays_to_settled_nights` handed the whole settled-night
/// budget to A and dropped B entirely — an early-checkout invoice then billed
/// two nights to a room the guest slept in once, and never mentioned the room
/// they actually slept in on the second night.
///
/// Nights 15/04 (R-OLD), 16/04 (R-NEW), 17/04 (R-OLD). The guest leaves on the
/// morning of 17/04 ⇒ 2 settled nights ⇒ R-OLD × 1 + R-NEW × 1.
#[tokio::test]
async fn move_out_and_back_bills_each_stay_separately_on_early_checkout() {
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

    // Buồng phòng dọn xong R-OLD trước khi khách quay lại — `change_room_tx`
    // chỉ nhận phòng mới ở trạng thái `vacant`, đúng như check-in.
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-OLD'")
        .execute(&pool)
        .await
        .unwrap();

    room_change::change_room(
        &pool,
        "B-OPT",
        "R-OLD",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 17).unwrap(),
    )
    .await
    .unwrap();

    let checkout_at = Local
        .with_ymd_and_hms(2026, 4, 17, 9, 0, 0)
        .single()
        .unwrap();
    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B-OPT".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        checkout_at,
    )
    .await
    .unwrap();
    assert_eq!(
        preview.settled_nights, 2,
        "tiền đề của test: khách rời sau đúng 2 đêm thật"
    );

    stay_lifecycle::check_out_at(
        &pool,
        checkout_req(
            "B-OPT",
            CheckoutSettlementMode::ActualNights,
            preview.recommended_total,
        ),
        checkout_at,
    )
    .await
    .unwrap();

    let snapshot: String =
        sqlx::query_scalar("SELECT pricing_snapshot FROM bookings WHERE id = 'B-OPT'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let stays = serde_json::from_str::<serde_json::Value>(&snapshot).unwrap()["room_stays"].clone();
    let stay_pairs: Vec<(String, i64)> = stays
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["room_id"].as_str().unwrap().to_string(),
                entry["nights"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        stay_pairs,
        vec![("R-OLD".to_string(), 1), ("R-NEW".to_string(), 1)],
        "hai đêm đã quyết toán là 15/04 ở R-OLD và 16/04 ở R-NEW, \
         không phải 2 đêm dồn hết cho R-OLD: {stays}"
    );

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
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-OLD") && l.contains("1 đêm")),
        "labels: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("R-NEW") && l.contains("1 đêm")),
        "R-NEW phải còn trên hoá đơn — khách ngủ ở đó đúng đêm 16/04: {labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("R-OLD") && l.contains("2 đêm")),
        "R-OLD không được nuốt cả ngân sách 2 đêm: {labels:?}"
    );
    assert_breakdown_sums_to_subtotal(&invoice);
}

/// Finding C: `group_checkout_tx` bulk-deletes `room_calendar` for every
/// booking in the group without re-deriving `room_stays` into
/// `pricing_snapshot` first — the move `check_out_tx` already makes on the
/// single-booking path.
///
/// The move-time snapshot alone is not enough, for the same reason
/// `move_then_extend_reflects_the_real_split_not_the_stale_snapshot` documents
/// above: `extend_stay_tx` adds `room_calendar` rows on the guest's current
/// room without touching `pricing_snapshot`. So this test moves *and* extends.
/// With the calendar deleted and the snapshot never refreshed, the invoice
/// bills the extended night to the wrong room — it splits 750.000 over the two
/// nights the stale snapshot remembers instead of the three actually charged.
#[tokio::test]
async fn group_checkout_keeps_the_room_split_for_a_booking_that_moved() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-A", "G-B", "G-C"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G-A", "G-B"]),
    )
    .await
    .unwrap();

    let moved_booking_id: String =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? AND room_id = 'G-A'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let other_booking_id: String =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? AND room_id = 'G-B'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    // group_checkin books 2 nights from today; moving "tomorrow" leaves the
    // first night on G-A and puts the second on G-C — the only shape that
    // makes a per-room split meaningful.
    let tomorrow = Local::now().date_naive() + Duration::days(1);
    room_change::change_room(&pool, &moved_booking_id, "G-C", true, None, tomorrow)
        .await
        .unwrap();

    // The extra night lands on G-C and is invisible to the snapshot
    // `change_room_tx` took at move time — only `room_calendar` knows about
    // it, and group checkout is about to delete that.
    stay_lifecycle::extend_stay(&pool, &moved_booking_id)
        .await
        .unwrap();

    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![moved_booking_id.clone(), other_booking_id.clone()],
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let calendar_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind(&moved_booking_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        calendar_rows, 0,
        "group checkout phải xoá room_calendar — nếu còn thì test này không đại diện cho path thật"
    );

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, &moved_booking_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let labels: Vec<String> = invoice
        .pricing_breakdown
        .iter()
        .map(|line| line.label.clone())
        .collect();
    assert!(
        labels
            .iter()
            .any(|l| l.contains("G-A") && l.contains("1 đêm")),
        "labels: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|l| l.contains("G-C") && l.contains("2 đêm")),
        "G-C phải cộng cả đêm gia hạn — group checkout phải chốt lại room_stays \
         từ room_calendar trước khi xoá, đúng như check_out_tx: {labels:?}"
    );
    assert_breakdown_sums_to_subtotal(&invoice);

    // The booking that never moved keeps its single line — the loop must not
    // invent a split where there was no move.
    let mut tx = pool.begin().await.unwrap();
    let other_invoice = invoice_generation::generate_invoice_tx(&mut tx, &other_booking_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(other_invoice.pricing_breakdown.len(), 1);
    assert_breakdown_sums_to_subtotal(&other_invoice);
}

/// H4: the split must fire only on a genuine move (`len() > 1`), not on any
/// non-empty result.
///
/// `invoice_keeps_one_line_when_no_move_happened` above cannot see the
/// difference: its booking never checked out, so there is no
/// `checkout_settlement`, the single breakdown line is the English fallback
/// either way, and `settlement_note` is `None` in both worlds. Running the
/// same scenario THROUGH checkout gives the settlement label something to say,
/// and then the two behaviours diverge — relaxing the threshold rewrites the
/// line as "Phòng … × 3 đêm" and adds a GHI CHÚ block to an invoice that never
/// needed one.
#[tokio::test]
async fn a_checked_out_booking_that_never_moved_keeps_its_settlement_line() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    stay_lifecycle::check_out(
        &pool,
        checkout_req("B-OPT", CheckoutSettlementMode::BookedNights, 750_000),
    )
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let invoice = invoice_generation::generate_invoice_tx(&mut tx, "B-OPT")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        invoice.pricing_breakdown.len(),
        1,
        "khách không đổi phòng thì không có gì để tách: {:?}",
        invoice.pricing_breakdown
    );
    assert_eq!(
        invoice.pricing_breakdown[0].label, "Thanh toán theo số đêm đã đặt",
        "dòng duy nhất phải là lời quyết toán, không phải một dòng 'Phòng …' tự dựng"
    );
    assert_eq!(
        invoice.settlement_note, None,
        "một dòng thì không cần khối GHI CHÚ lặp lại đúng câu đó"
    );
    assert_breakdown_sums_to_subtotal(&invoice);
}

/// H3: pins the optimistic guard added by commit 0736d14 — the repricing
/// `UPDATE` must stay conditioned on the `status` and `total_price` read
/// before the change, so `ensure_one_row_affected` can actually detect a
/// stale-state write instead of matching any existing row.
///
/// Deliberately a source-level assertion, which is unusual here and needs the
/// reason stated: inside one `BEGIN IMMEDIATE` transaction nothing can change
/// the row between the `SELECT` at the top of `change_room_tx` and this
/// `UPDATE`, and a second writer is blocked by SQLite's write lock. The guard
/// is therefore unreachable at runtime by construction, so no behavioural test
/// can turn it red and a tautology mutation survives the whole suite unseen.
/// What is testable is the property the guard exists for: that the write names
/// the pre-read values in its `WHERE`.
#[test]
fn the_repricing_update_stays_guarded_on_pre_read_state() {
    const SOURCE: &str = include_str!("../room_change.rs");

    let start = SOURCE
        .find("UPDATE bookings SET total_price = ?")
        .expect("câu UPDATE định giá lại phải còn trong room_change.rs");
    let statement = &SOURCE[start..start + SOURCE[start..].find('"').expect("hết chuỗi SQL")];

    // Chỉ soi phần sau WHERE. Soi cả câu thì `contains("total_price = ?")` đã
    // được chính mệnh đề SET thoả mãn, và assert cuối trở thành vô nghĩa: xoá
    // `AND total_price = ?` khỏi WHERE mà cả bộ test vẫn xanh.
    let where_clause = statement
        .split_once("WHERE")
        .map(|(_, rest)| rest)
        .expect("câu UPDATE định giá lại phải có mệnh đề WHERE");

    assert!(
        where_clause.contains("id = ?"),
        "câu UPDATE phải khoá theo booking: {statement}"
    );
    assert!(
        where_clause.contains("status = ?"),
        "thiếu điều kiện status đọc trước khi đổi — ensure_one_row_affected sẽ \
         khớp mọi dòng còn tồn tại và không phát hiện được ghi đè lên state cũ: {statement}"
    );
    assert!(
        where_clause.contains("total_price = ?"),
        "thiếu điều kiện total_price đọc trước khi đổi trong WHERE: {statement}"
    );
}

/// C2: a `room_calendar` row with a NULL `booking_id` — a maintenance block,
/// which the schema allows since `booking_id` is nullable — must be reported
/// as "phòng đã có lịch", not leak out as an internal error.
///
/// The in-transaction re-check used a bare `booking_id != ?`, and SQL
/// three-valued logic makes `NULL != 'B-OPT'` evaluate to NULL, so the row was
/// invisible to it. No double booking ever resulted — `PRIMARY KEY (room_id,
/// date)` rejects the UPDATE and the transaction rolls back — but the
/// receptionist got a `SYSTEM_INTERNAL_ERROR` for an ordinary "that room is
/// taken", with no idea what to do next.
#[tokio::test]
async fn a_maintenance_block_without_a_booking_reads_as_a_room_conflict() {
    let pool = test_pool().await;
    seed_stay_in_progress(&pool).await;

    // Phòng khoá để bảo trì: giữ chỗ trong lịch nhưng không thuộc booking nào.
    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status)
         VALUES ('R-NEW', '2026-04-17', NULL, 'blocked')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = room_change::change_room(
        &pool,
        "B-OPT",
        "R-NEW",
        true,
        None,
        NaiveDate::from_ymd_opt(2026, 4, 16).unwrap(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(error, BookingError::Conflict(_)),
        "phải là xung đột lịch phòng đọc được, không phải lỗi hệ thống: {error:?}"
    );
    assert!(
        error.to_string().contains("đã có lịch"),
        "thông báo phải nói rõ phòng đã có lịch: {error}"
    );

    // Và khách vẫn ở nguyên phòng cũ: giao dịch phải rollback trọn vẹn.
    let room_id: String = sqlx::query_scalar("SELECT room_id FROM bookings WHERE id = 'B-OPT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room_id, "R-OLD");
}
