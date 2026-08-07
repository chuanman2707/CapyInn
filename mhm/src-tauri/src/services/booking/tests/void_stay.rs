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

/// Một booking thuộc đoàn không được xoá riêng lẻ: xoá lẻ một phòng sẽ làm sai
/// tổng tiền và hoá đơn của cả đoàn. Guard phải chặn TRƯỚC khi ghi bất cứ gì —
/// một guard chặn nhưng lỡ ghi trước rồi còn tệ hơn không có guard nào cả, nên
/// bài test này siết cả hai vế: có lỗi đúng nội dung, và không có gì đổi.
///
/// Hai phép đọc trạng thái bên dưới chạy qua `&mut *tx` — đọc ngay trong
/// transaction còn mở, TRƯỚC khi rollback. Transaction thấy được ghi chưa
/// commit của chính nó, nên đây là cách duy nhất phân biệt "guard chặn trước
/// khi ghi" với "guard chặn sau khi ghi rồi rollback xoá dấu vết": đọc qua
/// pool (một kết nối khác) sau khi đã rollback thì hai trường hợp trông giống
/// hệt nhau — nếu guard bị dời xuống chạy sau UPDATE/DELETE, test vẫn sẽ xanh
/// trong khi bug đã lọt.
#[tokio::test]
async fn voiding_a_booking_in_a_group_is_rejected_and_changes_nothing() {
    let pool = test_pool().await;
    seed_room(&pool, "R-3").await.expect("seeds room");
    seed_booked_reservation(&pool, "B-3", "R-3")
        .await
        .expect("seeds reservation");
    sqlx::query(
        "INSERT INTO booking_groups (id, group_name, master_booking_id, organizer_name,
                                     total_rooms, status, created_at)
         VALUES ('G-3', 'Đoàn thử', 'B-3', 'Trưởng đoàn', 1, 'active', '2026-04-15T10:00:00+07:00')",
    )
    .execute(&pool)
    .await
    .expect("seeds booking group");
    sqlx::query("UPDATE bookings SET group_id = 'G-3' WHERE id = 'B-3'")
        .execute(&pool)
        .await
        .expect("links booking to group");

    let req = VoidBookingRequest {
        booking_id: "B-3".to_string(),
        reason: None,
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    let error = void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-3")
        .await
        .expect_err("grouped booking must be rejected");

    assert!(
        error.to_string().contains("thuộc đoàn"),
        "thông báo phải nói rõ lượt thuộc đoàn, thấy: {error}"
    );

    // Đọc qua &mut *tx — trong transaction còn mở, trước rollback — xem chú
    // thích ở đầu test: đây là phần phân biệt "chặn trước khi ghi" khỏi "chặn
    // sau khi ghi rồi rollback xoá dấu vết".
    let status: String = sqlx::query_scalar("SELECT status FROM bookings WHERE id = 'B-3'")
        .fetch_one(&mut *tx)
        .await
        .expect("reads booking status after rejection");
    assert_eq!(
        status, "booked",
        "trạng thái booking không được đổi khi lượt bị từ chối vì thuộc đoàn"
    );

    let calendar_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = 'B-3'")
            .fetch_one(&mut *tx)
            .await
            .expect("counts calendar rows");
    assert_eq!(
        calendar_rows, 2,
        "room_calendar không được xoá khi lượt bị từ chối vì thuộc đoàn"
    );

    tx.rollback().await.expect("rolls back");
}
