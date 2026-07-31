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
