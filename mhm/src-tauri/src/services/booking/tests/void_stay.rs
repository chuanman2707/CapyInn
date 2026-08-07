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

/// Xoá một lượt đang ở: phòng phải về trống ngay, nếu không thì phòng đó bị
/// khoá cứng — nhìn thì "đang có khách" mà thực ra không có ai.
#[tokio::test]
async fn voiding_an_active_stay_frees_the_room() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-3", "R-3")
        .await
        .expect("seeds active booking");

    let occupied: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-3'")
        .fetch_one(&pool)
        .await
        .expect("reads room status");
    assert_eq!(
        occupied, "occupied",
        "fixture phải bắt đầu từ phòng có khách"
    );

    let req = VoidBookingRequest {
        booking_id: "B-3".to_string(),
        reason: Some("Bấm nhầm".to_string()),
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    let response = void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-3")
        .await
        .expect("voids active stay");
    tx.commit().await.expect("commits");

    assert_eq!(response.previous_status, "active");

    let room_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-3'")
        .fetch_one(&pool)
        .await
        .expect("reads room status after");
    assert_eq!(room_status, "vacant");

    let calendar_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = 'B-3'")
            .fetch_one(&pool)
            .await
            .expect("counts calendar rows");
    assert_eq!(calendar_rows, 0);
}

/// Xoá một lượt đang ở nhưng phòng bị một thao tác khác đổi trạng thái ngay
/// giữa chừng (ví dụ buồng phòng đổi sang "đang dọn" trong lúc lệnh này đang
/// chạy): guard trên UPDATE rooms phải bắt được và từ chối, không được âm
/// thầm ghi "vacant" đè lên trạng thái mới hơn — đó là đường sinh ra phòng
/// "phantom" (không khoá bởi booking nào nhưng cũng không đúng trạng thái
/// thật).
///
/// Dùng đúng cơ chế trigger của
/// `check_in_rolls_back_when_room_status_changes_before_guarded_room_update`
/// (`tests/stays.rs`): một trigger SQLite nổ ngay khi câu UPDATE bookings của
/// chính `void_booking_tx` chạy (bước ghi ngay sau lần đọc ban đầu), và đổi
/// `rooms.status` trước khi hàm chạm tới UPDATE rooms được guard. Một
/// transaction, không cần hai pool — không giống
/// `checkout_fails_when_second_pool_checked_out_booking_first`, vốn cần hai
/// pool vì `check_out_tx` đọc và chặn theo trạng thái *booking* rất sớm nên
/// một thay đổi trước khi gọi hàm đã đủ; `void_booking_tx` không đọc trạng
/// thái *phòng* ở đâu cả ngoài chính câu UPDATE cuối, nên trigger giữa
/// transaction là cách duy nhất tạo đúng khoảng hở "đọc trước — ghi sau".
///
/// Đọc trạng thái phòng qua `&mut *tx` — trong transaction còn mở, TRƯỚC khi
/// rollback — cùng lý do đã nêu ở
/// `voiding_a_booking_in_a_group_is_rejected_and_changes_nothing` phía trên:
/// đọc qua pool sau khi rollback thì ghi của trigger cũng bị cuốn theo,
/// "guard chặn đúng lúc" và "guard đã bị xoá" sẽ trông giống hệt nhau.
#[tokio::test]
async fn voiding_an_active_stay_is_rejected_when_room_status_changes_before_guarded_update() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-CAS-VOID", "R-CAS-VOID")
        .await
        .expect("seeds active booking");

    sqlx::query(
        "CREATE TRIGGER mark_room_cleaning_after_void_update
         AFTER UPDATE ON bookings
         WHEN NEW.id = 'B-CAS-VOID' AND NEW.status = 'voided'
         BEGIN
           UPDATE rooms SET status = 'cleaning' WHERE id = NEW.room_id;
         END",
    )
    .execute(&pool)
    .await
    .expect("creates room-status race trigger");

    let req = VoidBookingRequest {
        booking_id: "B-CAS-VOID".to_string(),
        reason: Some("Bấm nhầm".to_string()),
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    let error =
        void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-CAS-VOID")
            .await
            .expect_err("guarded room update should catch the room status race");

    assert!(
        error
            .to_string()
            .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION),
        "lỗi phải là invalid-state-transition, thấy: {error}"
    );

    // Đọc qua &mut *tx — xem chú thích ở đầu test: đây là phần phân biệt
    // "chặn trước khi ghi" khỏi "chặn sau khi ghi rồi rollback xoá dấu vết".
    let room_status: String =
        sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-CAS-VOID'")
            .fetch_one(&mut *tx)
            .await
            .expect("reads room status after rejection");
    assert_eq!(
        room_status, "cleaning",
        "phòng phải giữ nguyên trạng thái do thao tác khác đặt — void không được ghi đè thành vacant"
    );

    tx.rollback().await.expect("rolls back");
}

/// Dòng tiền không bị đụng tới — append-only. Lượt biến mất khỏi báo cáo nhờ
/// bộ lọc trạng thái, không nhờ việc xoá dữ liệu.
#[tokio::test]
async fn voiding_never_deletes_money_rows() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-4", "R-4")
        .await
        .expect("seeds active booking");
    seed_transaction(
        &pool,
        "B-4",
        250_000,
        "charge",
        "Tiền phòng",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .expect("seeds charge");

    let req = VoidBookingRequest {
        booking_id: "B-4".to_string(),
        reason: None,
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-4")
        .await
        .expect("voids stay");
    tx.commit().await.expect("commits");

    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = 'B-4'")
            .fetch_one(&pool)
            .await
            .expect("counts transactions");
    assert_eq!(rows, 1, "dòng tiền phải còn nguyên");
}

/// Trả phòng xong rồi mới phát hiện nhập sai (ca trong ảnh: check-in và
/// check-out cách nhau 0 phút). Phòng đang `cleaning` vì lượt đó, task dọn
/// phòng cũng do lượt đó sinh ra — cả hai phải được gỡ.
#[tokio::test]
async fn voiding_a_checked_out_stay_undoes_cleaning_and_housekeeping() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-5", "R-5")
        .await
        .expect("seeds active booking");

    let actual_checkout = "2026-04-15T18:08:00+07:00";
    sqlx::query("UPDATE bookings SET status = 'checked_out', actual_checkout = ? WHERE id = 'B-5'")
        .bind(actual_checkout)
        .execute(&pool)
        .await
        .expect("marks booking checked out");
    sqlx::query("UPDATE rooms SET status = 'cleaning' WHERE id = 'R-5'")
        .execute(&pool)
        .await
        .expect("marks room cleaning");
    sqlx::query(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at)
         VALUES ('HK-5', 'R-5', 'needs_cleaning', ?, ?)",
    )
    .bind(actual_checkout)
    .bind(actual_checkout)
    .execute(&pool)
    .await
    .expect("seeds housekeeping task");

    let req = VoidBookingRequest {
        booking_id: "B-5".to_string(),
        reason: Some("Bấm nhầm".to_string()),
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    let response = void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-5")
        .await
        .expect("voids checked-out stay");
    tx.commit().await.expect("commits");

    assert_eq!(response.previous_status, "checked_out");

    let room_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-5'")
        .fetch_one(&pool)
        .await
        .expect("reads room status");
    assert_eq!(room_status, "vacant");

    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE id = 'HK-5'")
        .fetch_one(&pool)
        .await
        .expect("counts housekeeping tasks");
    assert_eq!(
        tasks, 0,
        "task dọn phòng do lượt nhập sai sinh ra phải bị gỡ"
    );
}

/// Phòng đã bán lại cho khách khác: KHÔNG được đụng vào `rooms.status`. Đây là
/// chỗ duy nhất trong lệnh này mà 0 dòng ảnh hưởng là hợp lệ.
#[tokio::test]
async fn voiding_does_not_touch_a_room_that_was_already_reused() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-6", "R-6")
        .await
        .expect("seeds active booking");

    sqlx::query(
        "UPDATE bookings SET status = 'checked_out',
                actual_checkout = '2026-04-15T18:08:00+07:00' WHERE id = 'B-6'",
    )
    .execute(&pool)
    .await
    .expect("marks booking checked out");
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R-6'")
        .execute(&pool)
        .await
        .expect("simulates a new guest already in the room");

    let req = VoidBookingRequest {
        booking_id: "B-6".to_string(),
        reason: None,
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-6")
        .await
        .expect("void must succeed even when the room was reused");
    tx.commit().await.expect("commits");

    let room_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = 'R-6'")
        .fetch_one(&pool)
        .await
        .expect("reads room status");
    assert_eq!(room_status, "occupied", "khách mới không được bị đá ra");
}

/// Task dọn phòng đã dọn xong thì giữ nguyên — việc dọn đã xảy ra thật.
#[tokio::test]
async fn voiding_keeps_a_housekeeping_task_that_was_already_cleaned() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-7", "R-7")
        .await
        .expect("seeds active booking");

    let actual_checkout = "2026-04-15T18:08:00+07:00";
    sqlx::query("UPDATE bookings SET status = 'checked_out', actual_checkout = ? WHERE id = 'B-7'")
        .bind(actual_checkout)
        .execute(&pool)
        .await
        .expect("marks booking checked out");
    sqlx::query("UPDATE rooms SET status = 'cleaning' WHERE id = 'R-7'")
        .execute(&pool)
        .await
        .expect("marks room cleaning");
    sqlx::query(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, cleaned_at, created_at)
         VALUES ('HK-7', 'R-7', 'clean', ?, ?, ?)",
    )
    .bind(actual_checkout)
    .bind("2026-04-15T19:00:00+07:00")
    .bind(actual_checkout)
    .execute(&pool)
    .await
    .expect("seeds a finished housekeeping task");

    let req = VoidBookingRequest {
        booking_id: "B-7".to_string(),
        reason: None,
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-7")
        .await
        .expect("voids stay");
    tx.commit().await.expect("commits");

    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE id = 'HK-7'")
        .fetch_one(&pool)
        .await
        .expect("counts housekeeping tasks");
    assert_eq!(tasks, 1, "việc dọn đã xảy ra thật, không được xoá dấu vết");
}

/// Hoá đơn đã xuất phải chuyển sang `voided`, và SỐ hoá đơn không được đụng —
/// dãy số thủng lỗ không giải thích được còn tệ hơn giữ một dòng đã huỷ. Không
/// test nào ở trên seed invoice, nên câu `UPDATE invoices` trong nhánh
/// `checked_out` chưa có test nào bắt nếu bị xoá hoặc gán nhầm cột — thêm ở
/// đây để lấp chỗ đó.
#[tokio::test]
async fn voiding_a_checked_out_stay_marks_the_invoice_voided_and_keeps_its_number() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-8", "R-8")
        .await
        .expect("seeds active booking");

    let actual_checkout = "2026-04-15T18:08:00+07:00";
    sqlx::query("UPDATE bookings SET status = 'checked_out', actual_checkout = ? WHERE id = 'B-8'")
        .bind(actual_checkout)
        .execute(&pool)
        .await
        .expect("marks booking checked out");
    sqlx::query("UPDATE rooms SET status = 'cleaning' WHERE id = 'R-8'")
        .execute(&pool)
        .await
        .expect("marks room cleaning");
    sqlx::query(
        "INSERT INTO invoices (
            id, invoice_number, booking_id, hotel_name, hotel_address, hotel_phone,
            guest_name, room_name, room_type, check_in, check_out, nights,
            pricing_breakdown, subtotal, total, balance_due, created_at
         ) VALUES (
            'INV-8', 'INV-20260415-008', 'B-8', 'Capy Hotel', '123 Đường ABC', '0909999999',
            'Khách B-8', 'R-8', 'Standard', '2026-04-15', '2026-04-16', 1,
            '[]', 500000, 500000, 0, ?
         )",
    )
    .bind(actual_checkout)
    .execute(&pool)
    .await
    .expect("seeds issued invoice");

    let req = VoidBookingRequest {
        booking_id: "B-8".to_string(),
        reason: Some("Bấm nhầm".to_string()),
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-8")
        .await
        .expect("voids checked-out stay");
    tx.commit().await.expect("commits");

    let (status, invoice_number): (String, String) =
        sqlx::query_as("SELECT status, invoice_number FROM invoices WHERE id = 'INV-8'")
            .fetch_one(&pool)
            .await
            .expect("reads invoice after void");
    assert_eq!(status, "voided", "hoá đơn phải chuyển sang voided");
    assert_eq!(
        invoice_number, "INV-20260415-008",
        "số hoá đơn không được đổi hay xoá — dãy số không được thủng lỗ"
    );
}

/// `DELETE` trong nhánh `checked_out` khoanh vùng bằng đúng `triggered_at` của
/// lượt đang xoá, không phải "bất kỳ task chưa dọn nào trong phòng". Một task
/// dọn phòng khác trong cùng phòng — giờ trigger khác, ví dụ còn sót từ trước
/// hoặc nhân viên tự tạo — phải sống sót. Ba test ở Step 1 chỉ seed một task
/// mỗi phòng nên không phân biệt được "khớp đúng thời điểm" khỏi "khớp cả
/// phòng, bỏ qua thời điểm"; thêm ở đây để vế "và chỉ nó" trong chú thích ở
/// `void_lifecycle.rs` có test bảo vệ thật.
#[tokio::test]
async fn voiding_a_checked_out_stay_leaves_an_unrelated_uncleaned_task_in_the_same_room_alone() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-9", "R-9")
        .await
        .expect("seeds active booking");

    let actual_checkout = "2026-04-15T18:08:00+07:00";
    sqlx::query("UPDATE bookings SET status = 'checked_out', actual_checkout = ? WHERE id = 'B-9'")
        .bind(actual_checkout)
        .execute(&pool)
        .await
        .expect("marks booking checked out");
    sqlx::query("UPDATE rooms SET status = 'cleaning' WHERE id = 'R-9'")
        .execute(&pool)
        .await
        .expect("marks room cleaning");
    sqlx::query(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at)
         VALUES ('HK-9', 'R-9', 'needs_cleaning', ?, ?)",
    )
    .bind(actual_checkout)
    .bind(actual_checkout)
    .execute(&pool)
    .await
    .expect("seeds the housekeeping task this checkout created");
    sqlx::query(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at)
         VALUES ('HK-9-DECOY', 'R-9', 'needs_cleaning', '2026-04-10T09:00:00+07:00',
                 '2026-04-10T09:00:00+07:00')",
    )
    .execute(&pool)
    .await
    .expect("seeds an unrelated uncleaned task in the same room");

    let req = VoidBookingRequest {
        booking_id: "B-9".to_string(),
        reason: None,
    };

    let mut tx = begin_tx(&pool).await.expect("begins tx");
    void_lifecycle::void_booking_tx(&mut tx, &req, "admin-1", Local::now(), "R-9")
        .await
        .expect("voids checked-out stay");
    tx.commit().await.expect("commits");

    let removed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE id = 'HK-9'")
        .fetch_one(&pool)
        .await
        .expect("counts the task this checkout created");
    assert_eq!(removed, 0, "task do đúng lượt này sinh ra phải bị gỡ");

    let survived: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE id = 'HK-9-DECOY'")
            .fetch_one(&pool)
            .await
            .expect("counts the unrelated task");
    assert_eq!(
        survived, 1,
        "task không liên quan trong cùng phòng không được bị cuốn theo"
    );
}

/// Gọi hai lần cùng `idempotency_key` chỉ được tác dụng một lần — mạng chập
/// hay người dùng bấm đúp không được xoá hai thứ.
#[tokio::test]
async fn voiding_twice_with_the_same_key_only_acts_once() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-8", "R-8")
        .await
        .expect("seeds active booking");

    let ctx = cmd("void_booking", "idem-void-1");

    let first = void_lifecycle::void_booking_idempotent(
        &pool,
        &ctx,
        VoidBookingRequest {
            booking_id: "B-8".to_string(),
            reason: None,
        },
        "admin-1".to_string(),
    )
    .await
    .expect("first void succeeds");

    let second = void_lifecycle::void_booking_idempotent(
        &pool,
        &ctx,
        VoidBookingRequest {
            booking_id: "B-8".to_string(),
            reason: None,
        },
        "admin-1".to_string(),
    )
    .await
    .expect("replay returns the stored result instead of failing");

    assert_replayed_pair(&first, &second);

    let status: String = sqlx::query_scalar("SELECT status FROM bookings WHERE id = 'B-8'")
        .fetch_one(&pool)
        .await
        .expect("reads booking status");
    assert_eq!(status, "voided");

    assert_single_outbox_event(&pool, &ctx, "booking.voided").await;
}
