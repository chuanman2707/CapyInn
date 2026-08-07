use super::prelude::*;
use crate::services::booking::{support::begin_tx, void_lifecycle};

/// Xoá một lượt đặt trước: booking thành `voided`, `room_calendar` sạch, và
/// `rooms.status` KHÔNG bị đụng — phòng của một reservation chưa nhận vẫn đang
/// trống hoặc đang có khách khác, cả hai đều không phải việc của lệnh này.
#[tokio::test]
async fn voiding_a_reservation_clears_the_calendar_and_leaves_the_room_alone() {
    let pool = test_pool().await;
    seed_room(&pool, "R-1").await.expect("seeds room");
    seed_booked_reservation(&pool, "B-1", "R-1")
        .await
        .expect("seeds reservation");

    let room_status_before: String =
        sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-1'")
            .fetch_one(&pool)
            .await
            .expect("reads room status before");

    let req = VoidBookingRequest {
        booking_id: "B-1".to_string(),
        reason: Some("Nhập trùng".to_string()),
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    let response = void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-1")
        .await
        .expect("voids reservation");
    tx.commit().await.expect("commits");

    assert_eq!(response.previous_status, "booked");
    assert_eq!(response.room_id, "R-1");

    let (status, voided_by, reason): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, voided_by, void_reason FROM bookings WHERE id = 'B-1'")
            .fetch_one(&pool)
            .await
            .expect("reads booking after void");
    assert_eq!(status, "voided");
    assert_eq!(voided_by.as_deref(), Some("admin-1"));
    assert_eq!(reason.as_deref(), Some("Nhập trùng"));

    let calendar_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = 'B-1'")
            .fetch_one(&pool)
            .await
            .expect("counts calendar rows");
    assert_eq!(calendar_rows, 0);

    let room_status_after: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-1'")
        .fetch_one(&pool)
        .await
        .expect("reads room status after");
    assert_eq!(room_status_after, room_status_before);
}

/// Xoá hai lần: lần thứ hai phải là lỗi người dùng nói rõ, không phải panic
/// và cũng không phải "thành công" im lặng.
#[tokio::test]
async fn voiding_an_already_voided_booking_is_rejected() {
    let pool = test_pool().await;
    seed_room(&pool, "R-2").await.expect("seeds room");
    seed_booked_reservation(&pool, "B-2", "R-2")
        .await
        .expect("seeds reservation");

    let req = VoidBookingRequest {
        booking_id: "B-2".to_string(),
        reason: None,
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-2")
        .await
        .expect("first void succeeds");
    tx.commit().await.expect("commits");

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    let error = void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-2")
        .await
        .expect_err("second void must fail");

    assert!(
        error.to_string().contains("đã được xóa"),
        "thông báo phải nói rõ lượt đã bị xoá, thấy: {error}"
    );
}
