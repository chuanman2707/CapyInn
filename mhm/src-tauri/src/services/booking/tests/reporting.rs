use super::prelude::*;

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
    let cancelled_row = booking_export_row(&pool, "2026-04-01", "2026-04-30", "B305").await;

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

#[allow(clippy::too_many_arguments)]
async fn seed_checked_out_actual_nights_settlement(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    booking_id: &str,
    room_id: &str,
    paid_amount: i64,
    original_charge: i64,
    adjustment: i64,
) {
    seed_room(pool, room_id).await.unwrap();
    seed_active_booking_with_terms(
        pool,
        booking_id,
        room_id,
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
             paid_amount = ?,
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(paid_amount)
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind(booking_id)
    .execute(pool)
    .await
    .unwrap();
    seed_transaction(
        pool,
        booking_id,
        original_charge,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        pool,
        booking_id,
        adjustment,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn same_day_checkout_settlement_counts_one_room_sold_and_full_revenue() {
    let pool = test_pool().await;
    seed_checked_out_actual_nights_settlement(&pool, "B420", "R420", 500_000, 250_000, -1_750_000)
        .await;

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
    seed_checked_out_actual_nights_settlement(
        &pool, "B422", "R422", 500_000, 2_500_000, -2_000_000,
    )
    .await;

    let row = booking_export_row(&pool, "2026-04-01", "2026-04-30", "B422").await;

    assert_eq!(row.room_price, 500_000);
    assert_eq!(row.charge_total, 500_000);
    assert_eq!(row.recognized_revenue, 500_000);
}

#[tokio::test]
async fn checkout_settlement_export_rows_follow_reporting_checkout_boundary() {
    let pool = test_pool().await;
    seed_checked_out_actual_nights_settlement(
        &pool, "B423", "R423", 500_000, 2_500_000, -2_000_000,
    )
    .await;

    let row = booking_export_row(&pool, "2026-04-21", "2026-04-21", "B423").await;

    assert_eq!(row.expected_checkout, "2026-04-21");
    assert_eq!(row.actual_checkout, "2026-04-20T18:00:00+07:00");
}

#[tokio::test]
async fn checkout_settlement_export_rows_exclude_original_checkin_window_after_shift() {
    let pool = test_pool().await;
    seed_checked_out_actual_nights_settlement(
        &pool, "B424", "R424", 500_000, 2_500_000, -2_000_000,
    )
    .await;

    assert!(missing_booking_export_row(&pool, "2026-04-20", "2026-04-20", "B424").await);
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

    let row = booking_export_row(&pool, "2026-04-15", "2026-04-15", "B425").await;

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

    let row = booking_export_row(&pool, "2026-05-06", "2026-05-06", "B426").await;
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

    let row = booking_export_row(&pool, "2026-05-06", "2026-05-06", "B427").await;
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

/// Closing a day must not stamp a stay that belongs to the next one.
///
/// `mark_bookings_audited_tx` selected on `DATE(check_in_at)`, and SQLite's
/// `DATE()` converts an offset stamp to **UTC**: a 02:00 arrival on the 17th is
/// 19:00 on the 16th in UTC, so it read as the 16th and the audit for the 16th
/// swept it up. That is every check-in before 07:00 local, every day — the late
/// arrivals a small hotel actually takes — and it is a write, so the wrong day
/// is recorded on the row rather than merely displayed.
#[tokio::test]
async fn the_night_audit_does_not_stamp_a_stay_that_arrived_after_local_midnight() {
    let pool = test_pool().await;
    seed_room(&pool, "R304").await.unwrap();
    seed_active_booking(&pool, "B304", "R304").await.unwrap();
    sqlx::query(
        "UPDATE bookings
         SET check_in_at = '2026-04-17T02:00:00+07:00',
             expected_checkout = '2026-04-18T10:00:00+07:00'
         WHERE id = ?",
    )
    .bind("B304")
    .execute(&pool)
    .await
    .unwrap();

    audit_service::run_night_audit(&pool, "2026-04-16", None, "admin-1")
        .await
        .unwrap();

    let audited: i32 = sqlx::query_scalar("SELECT is_audited FROM bookings WHERE id = ?")
        .bind("B304")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        audited, 0,
        "a stay that checked in at 02:00 on the 17th is not part of the 16th",
    );
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

/// Đổi `bookings.status` thành `voided` một mình nó KHÔNG đủ: ba nhánh trong
/// `revenue_queries` đọc thẳng `folio_lines` / `transactions` mà không hề join
/// `bookings`. Test này canh đúng ba nhánh đó.
///
/// PHẠM VI: chỉ `revenue_queries` (tổng doanh thu + phân tích). Tên cũ của hàm
/// này ("...every_revenue_path") từng bị đọc nhầm thành "mọi đường đọc
/// bookings" — vòng review cuối chỉ ra đó là một lời hứa sai: thân hàm chưa
/// từng gọi `booking_list_queries`, `guest_queries`, `activity_queries`,
/// `group_queries`, hay `assistant_queries`. Bài kiểm phủ ĐỦ các đường đó nằm ở
/// `voided_booking_disappears_from_every_booking_read_path` bên dưới; hàm này
/// giữ nguyên phạm vi hẹp và đổi tên cho khớp.
#[tokio::test]
async fn voided_booking_disappears_from_the_recognized_revenue_totals() {
    let pool = test_pool().await;
    seed_room(&pool, "R-VOID").await.expect("seeds room");
    seed_active_booking(&pool, "B-VOID", "R-VOID")
        .await
        .expect("seeds booking");

    seed_folio_line(&pool, "B-VOID", 120_000, "2026-04-15T11:00:00+07:00")
        .await
        .expect("seeds folio line");
    seed_transaction(
        &pool,
        "B-VOID",
        80_000,
        "cancellation_fee",
        "Phí huỷ",
        "2026-04-15T11:00:00+07:00",
    )
    .await
    .expect("seeds cancellation fee");

    let before = revenue_queries::load_total_revenue(&pool, "2026-04-15", "2026-04-15")
        .await
        .expect("reads revenue before void");
    // Giá trị fixture xác định tuyệt đối, không phải chặn dưới:
    //   250,000 tiền phòng (seed_active_booking: total_price=250_000, 1 đêm,
    //     check-in 2026-04-15 → checkout 2026-04-16, ghi nhận trọn đêm 04-15)
    // +  120,000 dòng folio (seed_folio_line)
    // +   80,000 phí huỷ (seed_transaction cancellation_fee)
    // = 450,000
    assert_eq!(
        before, 450_000,
        "fixture phải cộng đúng 450,000 để void làm mất đi, thấy {before}"
    );

    sqlx::query("UPDATE bookings SET status = 'voided' WHERE id = 'B-VOID'")
        .execute(&pool)
        .await
        .expect("voids booking");

    let after = revenue_queries::load_total_revenue(&pool, "2026-04-15", "2026-04-15")
        .await
        .expect("reads revenue after void");
    assert_eq!(
        after, 0,
        "lượt đã xoá không được còn đồng nào trong doanh thu"
    );

    let analytics = revenue_queries::load_analytics(&pool, "2026-04-15", "2026-04-15", 1)
        .await
        .expect("reads analytics after void");
    assert_eq!(
        analytics
            .revenue_by_source
            .iter()
            .map(|row| row.value)
            .sum::<i64>(),
        0,
        "doanh thu theo nguồn cũng không được đếm lượt đã xoá"
    );
    assert_eq!(
        analytics
            .top_rooms
            .iter()
            .map(|row| row.revenue)
            .sum::<i64>(),
        0,
        "top_rooms cũng không được đếm lượt đã xoá"
    );
}

/// Bản xuất báo cáo booking có `WHERE` ba nhánh nối bằng `OR`; hai nhánh cho
/// lượt `voided` lọt qua. Mỗi hàng mang `recognized_revenue` cộng từ charge +
/// phí huỷ + folio, nên lượt đã xoá sẽ hiện nguyên tiền nếu không lọc.
#[tokio::test]
async fn voided_booking_disappears_from_the_booking_export() {
    let pool = test_pool().await;
    seed_room(&pool, "R-EXP").await.expect("seeds room");
    seed_active_booking(&pool, "B-EXP", "R-EXP")
        .await
        .expect("seeds booking");
    seed_transaction(
        &pool,
        "B-EXP",
        250_000,
        "charge",
        "Tiền phòng",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .expect("seeds charge");
    seed_transaction(
        &pool,
        "B-EXP",
        90_000,
        "cancellation_fee",
        "Phí huỷ",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .expect("seeds cancellation fee");

    let before = audit_queries::load_booking_export_rows(&pool, "2026-04-15", "2026-04-15")
        .await
        .expect("reads export before void");
    assert!(
        before.iter().any(|row| row.id == "B-EXP"),
        "fixture phải xuất hiện trong bản xuất trước khi xoá"
    );

    sqlx::query("UPDATE bookings SET status = 'voided' WHERE id = 'B-EXP'")
        .execute(&pool)
        .await
        .expect("voids booking");

    let after = audit_queries::load_booking_export_rows(&pool, "2026-04-15", "2026-04-15")
        .await
        .expect("reads export after void");
    assert!(
        !after.iter().any(|row| row.id == "B-EXP"),
        "lượt đã xoá không được còn trong bản xuất báo cáo"
    );
}

/// DANH SÁCH TRẮNG các đường đọc `bookings` đã được dạy bộ lọc `voided`, tính
/// đến vòng rà cuối trước khi merge (2026-08-09). Đây là hàng rào chống hồi
/// quy cho CẢ LỚP bug, không phải một hàm đơn lẻ — ba vòng review độc lập đã
/// tìm ra sáu đường rò khác nhau qua ba lần, đúng vì mỗi lần chỉ có một test
/// hẹp canh đúng một hàm. Ai thêm một đường đọc `bookings` mới (hay sửa một
/// đường cũ) mà quên lọc `voided` PHẢI làm một khối assert ở đây đỏ — nếu
/// không, tên hàm "every" lại thành lời hứa sai y như lần trước.
///
/// Danh sách (khớp đúng comment sửa ở từng file `queries/`):
///  1. `booking_list_queries::load_bookings_with_guest` — mọi giá trị filter,
///     kể cả `None`/rác, vì bộ lọc voided không được phép đi qua `status_clause`.
///  2. `export_queries::load_booking_export_rows` — bản xuất CSV cho admin,
///     KHÔNG PHẢI `audit_queries::load_booking_export_rows` (đã vá ở Task 2,
///     hai hàm trùng tên khác file).
///  3. `guest_queries::load_guest_summaries` — total_stays/total_spent/last_visit.
///  4. `guest_queries::load_guest_bookings` — lịch sử ở của một khách.
///  5. `activity_queries::load_recent_check_ins` — feed Dashboard.
///  6. `activity_queries::load_recent_check_outs` — feed Dashboard.
///  7. `group_queries::load_group_bookings` (qua `load_group_detail`) — tổng
///     tiền đoàn. Void một booking thuộc đoàn hiện bị `void_booking_tx` chặn
///     ở tầng service, nên trạng thái này chỉ tạo được bằng UPDATE thẳng như
///     dưới đây — đúng thứ test này cố tình làm, vì lớp filter ở tầng đọc
///     phải đứng vững bất kể `voided` sinh ra bằng đường nào, không chỉ đường
///     duy nhất mà service hôm nay cho phép.
///  8. `assistant_queries::load_stay_charges` — tool trợ lý AI đọc theo
///     booking_id trực tiếp; không có gì chặn model hỏi lại một mã đã xoá nếu
///     mã đó còn nằm trong lịch sử hội thoại từ trước khi bị xoá.
#[tokio::test]
async fn voided_booking_disappears_from_every_booking_read_path() {
    let pool = test_pool().await;

    seed_room(&pool, "R-ALL").await.expect("seeds room");
    seed_active_booking(&pool, "B-ALL", "R-ALL")
        .await
        .expect("seeds booking");
    seed_folio_line(&pool, "B-ALL", 120_000, "2026-04-15T11:00:00+07:00")
        .await
        .expect("seeds folio line");
    seed_transaction(
        &pool,
        "B-ALL",
        80_000,
        "cancellation_fee",
        "Phí huỷ",
        "2026-04-15T11:00:00+07:00",
    )
    .await
    .expect("seeds cancellation fee");
    sqlx::query(
        "UPDATE bookings SET actual_checkout = '2026-04-16T10:00:00+07:00' WHERE id = 'B-ALL'",
    )
    .execute(&pool)
    .await
    .expect("sets actual_checkout so the check-out feed has a row to hide");

    sqlx::query(
        "INSERT INTO booking_groups (id, group_name, master_booking_id, organizer_name,
                                      total_rooms, status, created_at)
         VALUES ('GRP-ALL', 'Đoàn thử', 'B-ALL', 'Trưởng đoàn', 1, 'active',
                 '2026-04-15T10:00:00+07:00')",
    )
    .execute(&pool)
    .await
    .expect("seeds booking group");
    sqlx::query("UPDATE bookings SET group_id = 'GRP-ALL' WHERE id = 'B-ALL'")
        .execute(&pool)
        .await
        .expect("links booking to group");

    // Trước khi xoá: fixture phải thật sự xuất hiện ở mọi nơi, nếu không các
    // assert "biến mất" bên dưới sẽ xanh vì lý do sai (chưa từng có mặt).
    let guest_id = "guest-B-ALL";
    assert!(
        booking_list_queries::load_bookings_with_guest(&pool, None)
            .await
            .expect("list before void")
            .iter()
            .any(|b| b.id == "B-ALL"),
        "fixture phải có mặt trong danh sách đặt phòng trước khi xoá"
    );
    assert!(
        guest_queries::load_guest_summaries(&pool, None)
            .await
            .expect("guest summary before void")
            .iter()
            .any(|g| g.id == guest_id && g.total_stays == 1 && g.total_spent == 250_000),
        "fixture phải cộng vào tổng chi tiêu của khách trước khi xoá"
    );
    assert!(
        !activity_queries::load_recent_check_ins(&pool, 50)
            .await
            .expect("check-ins before void")
            .is_empty(),
        "fixture phải có mặt trong feed nhận phòng trước khi xoá"
    );
    assert!(
        !activity_queries::load_recent_check_outs(&pool, 50)
            .await
            .expect("check-outs before void")
            .is_empty(),
        "fixture phải có mặt trong feed trả phòng trước khi xoá"
    );
    let detail_before = group_queries::load_group_detail(&pool, "GRP-ALL")
        .await
        .expect("group detail before void");
    assert_eq!(
        detail_before.grand_total, 250_000,
        "fixture phải cộng vào tổng tiền đoàn trước khi xoá"
    );
    assert!(
        assistant_queries::load_stay_charges(&pool, "B-ALL")
            .await
            .expect("assistant stay charges before void")
            .is_some(),
        "trợ lý phải đọc được lượt này trước khi xoá"
    );

    sqlx::query("UPDATE bookings SET status = 'voided' WHERE id = 'B-ALL'")
        .execute(&pool)
        .await
        .expect("voids booking");

    // 1. Danh sách đặt phòng — bất kể filter là gì, kể cả filter khớp trạng
    //    thái cũ của booking (đã có trạng thái mới là 'voided' rồi nên không
    //    filter nào khớp được nữa, kể cả 'active').
    for status in [
        None,
        Some("active"),
        Some("booked"),
        Some("completed"),
        Some("bậy bạ"),
    ] {
        let filter = BookingFilter {
            status: status.map(str::to_string),
            from: None,
            to: None,
        };
        let list = booking_list_queries::load_bookings_with_guest(&pool, Some(filter))
            .await
            .unwrap_or_else(|e| panic!("list with filter {status:?}: {e}"));
        assert!(
            !list.iter().any(|b| b.id == "B-ALL"),
            "lượt đã xoá lọt qua danh sách đặt phòng với filter {status:?}"
        );
    }
    let unfiltered = booking_list_queries::load_bookings_with_guest(&pool, None)
        .await
        .expect("list with no filter object at all");
    assert!(
        !unfiltered.iter().any(|b| b.id == "B-ALL"),
        "lượt đã xoá lọt qua danh sách đặt phòng khi không truyền filter"
    );

    // 2. Bản xuất CSV cho admin (`export_queries`, khác `audit_queries`).
    let export = export_queries::load_booking_export_rows(&pool)
        .await
        .expect("export after void");
    assert!(
        !export.iter().any(|row| row.id == "B-ALL"),
        "lượt đã xoá lọt qua bản xuất CSV admin"
    );

    // 3 & 4. Tổng chi tiêu và lịch sử ở của khách.
    let guest_after = guest_queries::load_guest_summaries(&pool, None)
        .await
        .expect("guest summary after void")
        .into_iter()
        .find(|g| g.id == guest_id)
        .expect("guest itself must still exist");
    assert_eq!(
        guest_after.total_stays, 0,
        "total_stays vẫn đếm lượt đã xoá"
    );
    assert_eq!(
        guest_after.total_spent, 0,
        "total_spent vẫn cộng tiền của lượt đã xoá"
    );
    assert_eq!(
        guest_after.last_visit, None,
        "last_visit vẫn trỏ vào lượt đã xoá"
    );
    let guest_bookings_after = guest_queries::load_guest_bookings(&pool, guest_id)
        .await
        .expect("guest bookings after void");
    assert!(
        guest_bookings_after.is_empty(),
        "lịch sử ở của khách vẫn còn lượt đã xoá"
    );

    // 5 & 6. Feed hoạt động gần đây trên Dashboard.
    let check_ins_after = activity_queries::load_recent_check_ins(&pool, 50)
        .await
        .expect("check-ins after void");
    assert!(
        check_ins_after.is_empty(),
        "feed nhận phòng vẫn còn lượt đã xoá"
    );
    let check_outs_after = activity_queries::load_recent_check_outs(&pool, 50)
        .await
        .expect("check-outs after void");
    assert!(
        check_outs_after.is_empty(),
        "feed trả phòng vẫn còn lượt đã xoá"
    );

    // 7. Tổng tiền đoàn.
    let detail_after = group_queries::load_group_detail(&pool, "GRP-ALL")
        .await
        .expect("group detail after void");
    assert!(
        detail_after.bookings.is_empty(),
        "danh sách phòng của đoàn vẫn còn lượt đã xoá"
    );
    assert_eq!(
        detail_after.grand_total, 0,
        "tổng tiền đoàn vẫn cộng lượt đã xoá"
    );

    // 8. Tool trợ lý AI đọc theo booking_id.
    let charges_after = assistant_queries::load_stay_charges(&pool, "B-ALL")
        .await
        .expect("assistant stay charges after void");
    assert!(
        charges_after.is_none(),
        "trợ lý vẫn đọc được tiền của lượt đã xoá qua booking_id"
    );
}
