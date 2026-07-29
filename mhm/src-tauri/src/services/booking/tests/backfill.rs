use super::prelude::*;

fn day(offset: i64) -> String {
    (Local::now().date_naive() + Duration::days(offset))
        .format("%Y-%m-%d")
        .to_string()
}

fn guest(name: &str) -> CreateGuestRequest {
    CreateGuestRequest {
        guest_type: Some("domestic".to_string()),
        full_name: name.to_string(),
        doc_number: "012345678901".to_string(),
        dob: None,
        gender: Some("Nam".to_string()),
        nationality: Some("VN".to_string()),
        address: None,
        visa_expiry: None,
        scan_path: None,
        phone: Some("0900000001".to_string()),
    }
}

fn req(room: &str, cin: i64, cout: Option<i64>, expected: Option<i64>) -> BackfillStayRequest {
    BackfillStayRequest {
        room_id: room.to_string(),
        guests: vec![guest("Khách Ghi Bù")],
        check_in_date: day(cin),
        check_out_date: cout.map(day),
        expected_checkout_date: expected.map(day),
        total_price: 600_000,
        paid_amount: 600_000,
        source: Some("walk-in".to_string()),
        notes: None,
    }
}

async fn count(pool: &sqlx::Pool<sqlx::Sqlite>, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn backfill_checked_out_stay_records_full_history() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF1").await.unwrap();
    let ctx = cmd_with_request("backfill_stay", "req-bf-1", "idem-bf-1");

    let result =
        backfill::backfill_stay_idempotent(&pool, &ctx, req("R-BF1", -3, Some(-1), None), None)
            .await
            .unwrap();
    let booking: crate::models::Booking = serde_json::from_value(result.response).unwrap();

    assert_eq!(booking.status, "checked_out");
    assert_eq!(booking.nights, 2);
    assert!(booking.actual_checkout.is_some());
    assert_eq!(booking.total_price, 600_000);
    assert_eq!(booking.paid_amount, 600_000);

    // Trạng thái thật trong DB, không chỉ struct trả về.
    let row = sqlx::query(
        "SELECT status, nights, total_price, paid_amount, actual_checkout, check_in_at,
                expected_checkout, source
         FROM bookings WHERE id = ?",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "checked_out");
    assert_eq!(row.get::<i32, _>("nights"), 2);
    assert_eq!(row.get::<i64, _>("total_price"), 600_000);
    assert_eq!(row.get::<i64, _>("paid_amount"), 600_000);
    assert_eq!(row.get::<String, _>("source"), "walk-in");
    // Giờ chuẩn khách sạn: nhận 14:00 ngày vào, trả 12:00 ngày ra.
    assert!(row.get::<String, _>("check_in_at").contains(&day(-3)));
    assert!(row.get::<String, _>("check_in_at").contains("T14:00:00"));
    let actual_checkout: String = row.get::<Option<String>, _>("actual_checkout").unwrap();
    assert!(actual_checkout.contains(&day(-1)));
    assert!(actual_checkout.contains("T12:00:00"));
    assert_eq!(
        row.get::<String, _>("expected_checkout"),
        actual_checkout,
        "khách đã trả thì ngày ra dự kiến trùng ngày ra thực tế"
    );

    let cal: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE room_id = 'R-BF1' AND booking_id = ?",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cal, 2);

    // Sổ tiền: 1 dòng thu tiền phòng + 1 dòng thanh toán.
    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let payments: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'payment'",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((charges, payments), (1, 1));

    let linked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM booking_guests WHERE booking_id = ?")
            .bind(&booking.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(linked, 1);

    // Khách đã trả → phòng vẫn trống.
    let room_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-BF1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room_status, "vacant");
}

#[tokio::test]
async fn backfill_still_staying_marks_room_occupied() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF2").await.unwrap();
    let ctx = cmd_with_request("backfill_stay", "req-bf-2", "idem-bf-2");

    let result =
        backfill::backfill_stay_idempotent(&pool, &ctx, req("R-BF2", -1, None, Some(2)), None)
            .await
            .unwrap();
    let booking: crate::models::Booking = serde_json::from_value(result.response).unwrap();

    assert_eq!(booking.status, "active");
    assert_eq!(booking.nights, 3);
    assert!(booking.actual_checkout.is_none());

    let row =
        sqlx::query("SELECT status, actual_checkout, expected_checkout FROM bookings WHERE id = ?")
            .bind(&booking.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("status"), "active");
    assert!(row.get::<Option<String>, _>("actual_checkout").is_none());
    assert!(row.get::<String, _>("expected_checkout").contains(&day(2)));

    let cal: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE room_id = 'R-BF2' AND booking_id = ?",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cal, 3);

    let room_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-BF2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room_status, "occupied");
}

#[tokio::test]
async fn backfill_still_staying_rejects_room_that_is_not_vacant() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF7").await.unwrap();
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R-BF7'")
        .execute(&pool)
        .await
        .unwrap();
    let ctx = cmd_with_request("backfill_stay", "req-bf-7", "idem-bf-7");

    let error =
        backfill::backfill_stay_idempotent(&pool, &ctx, req("R-BF7", -1, None, Some(2)), None)
            .await
            .unwrap_err();
    assert!(
        error.message.contains("đang có khách"),
        "unexpected: {}",
        error.message
    );

    assert_eq!(count(&pool, "SELECT COUNT(*) FROM bookings").await, 0);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM guests").await, 0);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM room_calendar").await, 0);
}

#[tokio::test]
async fn backfill_rejects_check_in_today_or_future() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF3").await.unwrap();
    let ctx = cmd_with_request("backfill_stay", "req-bf-3", "idem-bf-3");

    let error =
        backfill::backfill_stay_idempotent(&pool, &ctx, req("R-BF3", 0, Some(1), None), None)
            .await
            .unwrap_err();
    assert!(
        error.message.contains("quá khứ"),
        "unexpected: {}",
        error.message
    );

    assert_eq!(count(&pool, "SELECT COUNT(*) FROM bookings").await, 0);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM guests").await, 0);
}

#[tokio::test]
async fn backfill_rejects_overlapping_stay() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF4").await.unwrap();
    let first = cmd_with_request("backfill_stay", "req-bf-4a", "idem-bf-4a");
    backfill::backfill_stay_idempotent(&pool, &first, req("R-BF4", -5, Some(-3), None), None)
        .await
        .unwrap();

    let second = cmd_with_request("backfill_stay", "req-bf-4b", "idem-bf-4b");
    let error =
        backfill::backfill_stay_idempotent(&pool, &second, req("R-BF4", -4, Some(-2), None), None)
            .await
            .unwrap_err();
    assert!(
        error.message.contains("đã có khách"),
        "unexpected: {}",
        error.message
    );

    // Lần ghi bù hỏng không được để lại mảnh dữ liệu nào.
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM bookings").await, 1);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM guests").await, 1);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM room_calendar").await, 2);
    assert_eq!(count(&pool, "SELECT COUNT(*) FROM transactions").await, 2);
}

#[tokio::test]
async fn backfill_rejects_paid_above_total() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF5").await.unwrap();
    let ctx = cmd_with_request("backfill_stay", "req-bf-5", "idem-bf-5");
    let mut request = req("R-BF5", -2, Some(-1), None);
    request.paid_amount = 700_000;

    let error = backfill::backfill_stay_idempotent(&pool, &ctx, request, None)
        .await
        .unwrap_err();
    assert!(
        error.message.contains("vượt quá"),
        "unexpected: {}",
        error.message
    );

    assert_eq!(count(&pool, "SELECT COUNT(*) FROM bookings").await, 0);
}

#[tokio::test]
async fn backfill_idempotent_retry_replays_without_duplicates() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BF6").await.unwrap();
    let ctx = cmd_with_request("backfill_stay", "req-bf-6", "idem-bf-6");

    let first =
        backfill::backfill_stay_idempotent(&pool, &ctx, req("R-BF6", -3, Some(-1), None), None)
            .await
            .unwrap();
    let second =
        backfill::backfill_stay_idempotent(&pool, &ctx, req("R-BF6", -3, Some(-1), None), None)
            .await
            .unwrap();
    assert_replayed_pair(&first, &second);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = 'R-BF6'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let calendar_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE room_id = 'R-BF6'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(calendar_rows, 2);

    let transactions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(transactions, 2);
}
