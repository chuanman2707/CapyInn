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
/// giữa chừng (ví dụ một lượt đặt trước khác giữ phòng này cho tuần sau,
/// đổi trạng thái sang "booked", trong lúc lệnh này đang chạy): guard trên
/// UPDATE rooms phải bắt được và từ chối, không được âm thầm ghi "vacant" đè
/// lên trạng thái mới hơn — đó là đường sinh ra phòng "phantom" (không khoá
/// bởi booking nào nhưng cũng không đúng trạng thái thật).
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
        "CREATE TRIGGER mark_room_booked_after_void_update
         AFTER UPDATE ON bookings
         WHEN NEW.id = 'B-CAS-VOID' AND NEW.status = 'voided'
         BEGIN
           UPDATE rooms SET status = 'booked' WHERE id = NEW.room_id;
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
        room_status, "booked",
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
/// check-out cách nhau 0 phút). Từ 09/08/2026 `check_out_tx` đặt phòng
/// `vacant` ngay và không còn sinh phiếu dọn nào — xoá lượt này không có gì
/// để hoàn tác trên `rooms`, phòng phải giữ nguyên `vacant`.
#[tokio::test]
async fn voiding_a_checked_out_stay_leaves_the_vacant_room_alone() {
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
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-5'")
        .execute(&pool)
        .await
        .expect("marks room vacant, matching a real checkout");

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
}

/// Phòng đã bán lại cho khách khác: KHÔNG được đụng vào `rooms.status`. Từ
/// 09/08/2026 nhánh `checked_out` của `void_booking_tx` không còn câu UPDATE
/// rooms nào cả (`void_lifecycle.rs:133`) — phòng đã bán lại an toàn theo cấu
/// trúc: không có câu lệnh nào để đụng vào nó. Đây không phải chuyện nới lỏng
/// một guard; nhánh `active` ngay bên trên vẫn ghi `rooms` và vẫn bắt buộc
/// đúng một dòng (`ensure_one_row_affected`, `void_lifecycle.rs:126`) — nhánh
/// này đơn giản là không còn gì để guard.
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
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-8'")
        .execute(&pool)
        .await
        .expect("marks room vacant, matching a real checkout");
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

/// Lượt đã trả phòng: gỡ toàn bộ tiền, ngày ghi nhận là ngày trả phòng.
#[tokio::test]
async fn preview_reports_the_full_total_for_a_checked_out_stay() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-9", "R-9")
        .await
        .expect("seeds booking");
    sqlx::query(
        "UPDATE bookings SET status = 'checked_out',
                actual_checkout = '2026-04-16T10:00:00+07:00', total_price = 500000
         WHERE id = 'B-9'",
    )
    .execute(&pool)
    .await
    .expect("marks checked out");
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-9'")
        .execute(&pool)
        .await
        .expect("marks room vacant, matching a real checkout");

    let preview = crate::queries::booking::void_queries::load_void_preview(&pool, "B-9")
        .await
        .expect("loads preview");

    assert_eq!(preview.previous_status, "checked_out");
    assert_eq!(preview.revenue_impact, 500_000);
    assert_eq!(preview.revenue_date, "2026-04-16");
    // Từ 09/08/2026 nhánh checked_out của `void_booking_tx` không còn đụng
    // `rooms` — cờ này luôn true cho một lượt đã trả phòng, xem
    // `void_queries.rs`.
    assert!(preview.room_status_unchanged);
    assert!(!preview.is_group_booking);
    // Ngoài phạm vi hai assert của brief: `seed_active_booking_with_room` cố
    // định `nights = 1`, nên trả phòng xong phải ghi nhận đúng 1/1 — không
    // test nào khác trong file này đụng tới hai trường này ở nhánh checked_out.
    assert_eq!(preview.nights_total, 1);
    assert_eq!(preview.nights_recognized, 1);
}

/// Trả phòng có cả dịch vụ ghi vào folio lẫn phí huỷ: cả hai bị `status !=
/// 'voided'` lọc khỏi báo cáo giống hệt tiền phòng (`load_folio_revenue`,
/// `load_cancellation_fee_revenue`, `revenue_queries.rs`), nên voided gỡ cả
/// ba khoản chứ không riêng tiền phòng. Ba số tiền phòng/folio/phí huỷ khác
/// nhau và không tròn để một phép cộng thiếu một số hạng (hay cộng nhầm cặp)
/// ra một tổng rõ ràng sai, không trùng ngẫu nhiên với tổng của hai số kia.
#[tokio::test]
async fn preview_reports_room_revenue_plus_folio_plus_cancellation_fee_for_a_checked_out_stay() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-FOLIO-FEE", "R-FOLIO-FEE")
        .await
        .expect("seeds booking");
    sqlx::query(
        "UPDATE bookings SET status = 'checked_out',
                actual_checkout = '2026-04-16T10:00:00+07:00', total_price = 317000
         WHERE id = 'B-FOLIO-FEE'",
    )
    .execute(&pool)
    .await
    .expect("marks checked out");
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-FOLIO-FEE'")
        .execute(&pool)
        .await
        .expect("marks room vacant, matching a real checkout");
    seed_folio_line(&pool, "B-FOLIO-FEE", 142_500, "2026-04-15T20:00:00+07:00")
        .await
        .expect("seeds folio line");
    seed_transaction(
        &pool,
        "B-FOLIO-FEE",
        68_300,
        "cancellation_fee",
        "Phí huỷ",
        "2026-04-16T09:00:00+07:00",
    )
    .await
    .expect("seeds cancellation fee");

    let preview = void_queries::load_void_preview(&pool, "B-FOLIO-FEE")
        .await
        .expect("loads preview");

    assert_eq!(preview.previous_status, "checked_out");
    assert_eq!(
        preview.revenue_impact,
        317_000 + 142_500 + 68_300,
        "phải cộng đủ tiền phòng (317.000) + folio (142.500) + phí huỷ (68.300) — thiếu một \
         số hạng nào cũng ra một tổng khác hẳn, không phải cùng con số làm tròn khác đi"
    );
}

/// Phòng đã bán lại cho khách khác (`occupied`) — ca đầu trong ba ca đại diện
/// cùng khoá một điều: từ 09/08/2026 nhánh `checked_out` của `void_booking_tx`
/// không còn câu UPDATE rooms nào (`void_lifecycle.rs`), nên
/// `room_status_unchanged` không còn so bằng bất kỳ giá trị cụ thể nào của
/// `rooms.status` — nó luôn true cho một lượt đã trả phòng, bất kể phòng đang
/// `occupied`, `booked`, hay `vacant`. Ba test này (ca `occupied` ở đây, cộng
/// hai ca `booked`/`vacant` ngay bên dưới) tồn tại song song dù không còn thể
/// bất đồng với nhau: bỏ ca `vacant` đi rồi thu hẹp cờ lại thành `&&
/// room_status != VACANT` sẽ vẫn lọt qua hai test còn lại mà không ai hay —
/// giữ đủ ba là cách duy nhất để bắt kiểu tái thu hẹp đó.
#[tokio::test]
async fn preview_flags_a_room_that_was_already_reused() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-10", "R-10")
        .await
        .expect("seeds booking");
    sqlx::query(
        "UPDATE bookings SET status = 'checked_out',
                actual_checkout = '2026-04-16T10:00:00+07:00' WHERE id = 'B-10'",
    )
    .execute(&pool)
    .await
    .expect("marks checked out");
    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = 'R-10'")
        .execute(&pool)
        .await
        .expect("simulates a new guest");

    let preview = crate::queries::booking::void_queries::load_void_preview(&pool, "B-10")
        .await
        .expect("loads preview");

    assert!(
        preview.room_status_unchanged,
        "lượt đã trả phòng luôn có room_status_unchanged = true — nhánh checked_out không \
         còn UPDATE rooms nào để so sánh trạng thái phòng, giống hệt ca 'booked'/'vacant'"
    );
}

/// Phòng đang giữ cho một lượt đặt trong tương lai (`booked`) — KHÔNG chỉ
/// `occupied` mới là "đã bán lại". Từ 09/08/2026 nhánh `checked_out` của
/// `void_booking_tx` không còn câu UPDATE rooms nào (`void_lifecycle.rs`), nên
/// `room_status_unchanged` không còn so bằng bất kỳ giá trị cụ thể nào của
/// `rooms.status` — nó luôn true cho một lượt đã trả phòng, bất kể phòng đang
/// `occupied`, `booked`, hay `vacant`. Test này chọn `booked` làm một trong ba
/// ca đại diện để khoá đúng điều đó.
#[tokio::test]
async fn preview_flags_a_room_that_is_now_booked_for_a_future_reservation() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-BOOKED-REUSE", "R-BOOKED-REUSE")
        .await
        .expect("seeds booking");
    sqlx::query(
        "UPDATE bookings SET status = 'checked_out',
                actual_checkout = '2026-04-16T10:00:00+07:00' WHERE id = 'B-BOOKED-REUSE'",
    )
    .execute(&pool)
    .await
    .expect("marks checked out");
    sqlx::query("UPDATE rooms SET status = 'booked' WHERE id = 'R-BOOKED-REUSE'")
        .execute(&pool)
        .await
        .expect("simulates the room held for a future reservation");

    let preview = crate::queries::booking::void_queries::load_void_preview(&pool, "B-BOOKED-REUSE")
        .await
        .expect("loads preview");

    assert!(
        preview.room_status_unchanged,
        "lượt đã trả phòng luôn có room_status_unchanged = true — nhánh checked_out không \
         còn UPDATE rooms nào để so sánh trạng thái phòng, giống hệt ca 'occupied'"
    );
}

/// Phòng đã về `vacant` — vẫn phải báo `room_status_unchanged = true`, y hệt
/// ca `occupied`/`booked` ở trên. Đây chính là ca chứng minh một cái tên ngụ ý
/// "phòng đang có khách khác" là sai: từ 09/08/2026 cờ này không nói gì về
/// TRẠNG THÁI phòng — nó chỉ nói "trả phòng không đụng gì tới `rooms`", và
/// luôn true cho mọi lượt đã trả phòng. Phía hiển thị (`VoidBookingDialog.tsx`)
/// không được suy đoán "có khách khác" chỉ vì cờ này bật.
#[tokio::test]
async fn preview_flags_room_status_unchanged_when_room_is_already_vacant() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-VACANT-UNCHANGED", "R-VACANT-UNCHANGED")
        .await
        .expect("seeds booking");
    sqlx::query(
        "UPDATE bookings SET status = 'checked_out',
                actual_checkout = '2026-04-16T10:00:00+07:00' WHERE id = 'B-VACANT-UNCHANGED'",
    )
    .execute(&pool)
    .await
    .expect("marks checked out");
    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = 'R-VACANT-UNCHANGED'")
        .execute(&pool)
        .await
        .expect("simulates housekeeping already finished before the void");

    let preview =
        crate::queries::booking::void_queries::load_void_preview(&pool, "B-VACANT-UNCHANGED")
            .await
            .expect("loads preview");

    assert!(
        preview.room_status_unchanged,
        "phòng 'vacant' vẫn phải báo true, y hệt 'occupied'/'booked' — cờ này luôn true cho \
         một lượt đã trả phòng, KHÔNG phải suy luận 'phòng đã trống sẵn nên không cần cảnh báo'"
    );
}

/// Đêm thứ 2 trên 3: đêm bắt đầu ngày nào tính vào ngày đó — khớp
/// `recognized_room_revenue_amount_sql` (`revenue_queries.rs`), nơi ngày
/// check-in đã ghi nhận 1 đêm ngay cả khi đêm đó chưa trôi qua. Vậy nên nhận
/// phòng từ HÔM QUA (còn 3 đêm) nghĩa là đã ghi nhận 2 đêm — đêm 1 xong, đêm 2
/// vừa bắt đầu — không phải 1. Chọn 300.000/3 đêm để một công thức thiếu "+1"
/// (chỉ đếm số ngày đã trôi qua, không tính đêm đang bắt đầu) ra 100.000 — một
/// con số KHÁC hẳn, không phải cùng con số làm tròn khác đi.
///
/// Cũng khoá `revenue_date` bám ngày check-in, KHÔNG phải hôm nay: `is_audited`
/// chỉ bật theo ngày check-in <= ngày đã chạy audit, nên hai trường phải mô tả
/// cùng một ngày — xem chú thích ở nhánh `active` của `revenue_date` trong
/// `void_queries.rs`.
#[tokio::test]
async fn preview_reports_partial_recognition_for_an_active_stay_on_its_second_of_three_nights() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-ACTIVE-2OF3", "R-ACTIVE-2OF3")
        .await
        .expect("seeds active booking");

    let today = Local::now().date_naive();
    let check_in_date = today - Duration::days(1);
    let check_in_at = format!("{}T10:00:00+07:00", check_in_date.format("%Y-%m-%d"));
    sqlx::query(
        "UPDATE bookings SET check_in_at = ?, nights = 3, total_price = 300000
         WHERE id = 'B-ACTIVE-2OF3'",
    )
    .bind(&check_in_at)
    .execute(&pool)
    .await
    .expect("sets a 3-night stay checked in yesterday");

    let preview = void_queries::load_void_preview(&pool, "B-ACTIVE-2OF3")
        .await
        .expect("loads preview");

    assert_eq!(preview.previous_status, "active");
    assert_eq!(preview.nights_total, 3);
    assert_eq!(
        preview.nights_recognized, 2,
        "nhận phòng hôm qua, còn 3 đêm: đã qua đêm 1, đang ở đêm 2 — 2 đêm ghi nhận"
    );
    assert_eq!(preview.revenue_impact, 200_000);
    assert_eq!(
        preview.revenue_date,
        check_in_date.format("%Y-%m-%d").to_string(),
        "revenue_date phải bám ngày check-in — is_audited chỉ chốt theo ngày check-in, \
         không phải hôm nay, nên hai trường phải mô tả cùng một ngày"
    );
    assert!(
        !preview.room_status_unchanged,
        "phòng đang có khách CHÍNH lượt này ở — không phải bị bán lại"
    );
}

/// Nhận phòng NGAY HÔM NAY: đã ghi nhận 1 đêm dù đêm đó chưa trôi qua nửa.
/// Biên này lộ một dạng bug KHÁC với "đêm thứ 2 trên 3" ở trên — test đó dùng
/// elapsed = 1 ngày, test này dùng elapsed = 0. Một công thức kiểu "chỉ cộng 1
/// khi elapsed > 0" vẫn qua được test đêm-2-trên-3 (1 + 1 = 2, đúng) nhưng sẽ
/// báo 0 đêm ở đây thay vì 1 — hai test cùng cần thì "+1" mới thật sự bị khoá.
#[tokio::test]
async fn preview_recognizes_one_night_immediately_when_check_in_is_today() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-ACTIVE-TODAY", "R-ACTIVE-TODAY")
        .await
        .expect("seeds active booking");

    let today = Local::now().date_naive();
    let check_in_at = format!("{}T10:00:00+07:00", today.format("%Y-%m-%d"));
    sqlx::query(
        "UPDATE bookings SET check_in_at = ?, nights = 3, total_price = 300000
         WHERE id = 'B-ACTIVE-TODAY'",
    )
    .bind(&check_in_at)
    .execute(&pool)
    .await
    .expect("sets a 3-night stay checked in today");

    let preview = void_queries::load_void_preview(&pool, "B-ACTIVE-TODAY")
        .await
        .expect("loads preview");

    assert_eq!(
        preview.nights_recognized, 1,
        "nhận phòng hôm nay: đêm 1 đã bắt đầu, phải ghi nhận ngay dù chưa qua nửa đêm"
    );
    assert_eq!(preview.revenue_impact, 100_000);
}

/// Ở lại quá ngày trả phòng dự kiến mà chưa check-out — thực tế thường gặp
/// (lễ tân chưa xử lý trả phòng), khác hẳn hai test elapsed nhỏ ở trên: nhận
/// phòng cách đây 5 ngày cho một lượt 3 đêm vẫn chỉ được ghi nhận đúng 3 đêm,
/// không phải 6 — số ghi nhận không được vượt tổng số đêm đã đặt.
#[tokio::test]
async fn preview_caps_recognition_at_nights_total_for_an_overstaying_active_guest() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-ACTIVE-OVERSTAY", "R-ACTIVE-OVERSTAY")
        .await
        .expect("seeds active booking");

    let today = Local::now().date_naive();
    let check_in_date = today - Duration::days(5);
    let check_in_at = format!("{}T10:00:00+07:00", check_in_date.format("%Y-%m-%d"));
    sqlx::query(
        "UPDATE bookings SET check_in_at = ?, nights = 3, total_price = 300000
         WHERE id = 'B-ACTIVE-OVERSTAY'",
    )
    .bind(&check_in_at)
    .execute(&pool)
    .await
    .expect("sets a 3-night stay checked in five days ago");

    let preview = void_queries::load_void_preview(&pool, "B-ACTIVE-OVERSTAY")
        .await
        .expect("loads preview");

    assert_eq!(preview.nights_total, 3);
    assert_eq!(
        preview.nights_recognized, 3,
        "quá hạn 2 ngày không được ghi nhận vượt tổng số đêm đã đặt"
    );
    assert_eq!(preview.revenue_impact, 300_000);
}

/// Lượt đặt trước có cọc: cọc KHÔNG bị gỡ khỏi doanh thu vì nó chưa từng ở
/// trong đó — `recognized_room_revenue_filter_sql` (`revenue_queries.rs`) đòi
/// `status IN ('active', 'checked_out')`, nên một lượt `booked` chưa đóng góp
/// đồng doanh thu phòng nào. `revenue_impact` phải là 0 (không seed folio hay
/// phí huỷ); tiền cọc hiển thị RIÊNG qua `deposit_amount`, không cộng vào.
/// Cũng là bài duy nhất khẳng định `booking_id`/`room_id`/`guest_name` không
/// phải giá trị giả — hai test gốc của Task 7 không đụng tới ba trường này.
#[tokio::test]
async fn preview_reports_the_deposit_for_a_booked_reservation_with_a_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BOOKED-DEP").await.expect("seeds room");
    seed_booked_reservation(&pool, "B-BOOKED-DEP", "R-BOOKED-DEP")
        .await
        .expect("seeds reservation with deposit");

    let preview = void_queries::load_void_preview(&pool, "B-BOOKED-DEP")
        .await
        .expect("loads preview");

    assert_eq!(preview.booking_id, "B-BOOKED-DEP");
    assert_eq!(preview.room_id, "R-BOOKED-DEP");
    assert_eq!(preview.guest_name, "Reserved Guest B-BOOKED-DEP");
    assert_eq!(preview.previous_status, "booked");
    assert_eq!(
        preview.revenue_impact, 0,
        "chưa nhận phòng nên chưa đồng doanh thu phòng nào; không seed folio/phí huỷ nên cũng 0"
    );
    assert_eq!(
        preview.deposit_amount, 50_000,
        "đúng bằng deposit_amount đã seed — tách riêng khỏi revenue_impact"
    );
    assert_eq!(
        preview.revenue_date, "2026-04-20",
        "rơi về ngày nhận phòng dự kiến — cọc không có ngày ghi nhận thật"
    );
    assert_eq!(
        preview.nights_recognized, 0,
        "chưa nhận phòng thì chưa ghi nhận đêm nào"
    );
    assert_eq!(preview.nights_total, 2);
    assert!(!preview.room_status_unchanged);
    assert!(!preview.is_group_booking);
}

/// Lượt đặt trước không cọc (cột NULL — chưa từng thu): không gỡ đồng nào.
/// Hai test gốc của Task 7 không seed ca "không cọc"; nếu nhánh xử lý NULL bị
/// đổi thành hoảng loạn hoặc một mặc định khác 0, test này đỏ.
#[tokio::test]
async fn preview_reports_zero_for_a_booked_reservation_without_a_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "R-BOOKED-NODEP")
        .await
        .expect("seeds room");
    seed_booked_reservation(&pool, "B-BOOKED-NODEP", "R-BOOKED-NODEP")
        .await
        .expect("seeds reservation");
    sqlx::query("UPDATE bookings SET deposit_amount = NULL WHERE id = 'B-BOOKED-NODEP'")
        .execute(&pool)
        .await
        .expect("clears the deposit");

    let preview = void_queries::load_void_preview(&pool, "B-BOOKED-NODEP")
        .await
        .expect("loads preview");

    assert_eq!(preview.previous_status, "booked");
    assert_eq!(preview.revenue_impact, 0);
    assert_eq!(
        preview.deposit_amount, 0,
        "cột NULL phải đọc ra 0, không panic"
    );
}

/// `is_audited` đọc thẳng từ cột, không phải hằng số: mặc định seed là 0
/// (false), UPDATE lên 1 phải đổi kết quả sang true.
#[tokio::test]
async fn preview_reflects_the_is_audited_flag() {
    let pool = test_pool().await;
    seed_active_booking_with_room(&pool, "B-AUDITED", "R-AUDITED")
        .await
        .expect("seeds active booking");

    let before = void_queries::load_void_preview(&pool, "B-AUDITED")
        .await
        .expect("loads preview before audit");
    assert!(
        !before.is_audited,
        "mặc định is_audited = 0 phải đọc ra false"
    );

    sqlx::query("UPDATE bookings SET is_audited = 1 WHERE id = 'B-AUDITED'")
        .execute(&pool)
        .await
        .expect("marks the booking audited");

    let after = void_queries::load_void_preview(&pool, "B-AUDITED")
        .await
        .expect("loads preview after audit");
    assert!(after.is_audited, "is_audited = 1 phải đọc ra true");
}

/// Lượt thuộc đoàn: `is_group_booking` phải lên true. Hai test gốc của Task 7
/// chỉ phủ ca KHÔNG thuộc đoàn (`assert!(!is_group_booking)`); chưa gì bắt
/// được nếu `group_id.is_some()` bị đổi ngược thành `is_none()`.
#[tokio::test]
async fn preview_flags_a_group_booking() {
    let pool = test_pool().await;
    seed_room(&pool, "R-GRP-PREVIEW").await.expect("seeds room");
    seed_booked_reservation(&pool, "B-GRP-PREVIEW", "R-GRP-PREVIEW")
        .await
        .expect("seeds reservation");
    sqlx::query(
        "INSERT INTO booking_groups (id, group_name, master_booking_id, organizer_name,
                                     total_rooms, status, created_at)
         VALUES ('G-PREVIEW', 'Đoàn thử', 'B-GRP-PREVIEW', 'Trưởng đoàn', 1, 'active',
                 '2026-04-15T10:00:00+07:00')",
    )
    .execute(&pool)
    .await
    .expect("seeds booking group");
    sqlx::query("UPDATE bookings SET group_id = 'G-PREVIEW' WHERE id = 'B-GRP-PREVIEW'")
        .execute(&pool)
        .await
        .expect("links booking to group");

    let preview = void_queries::load_void_preview(&pool, "B-GRP-PREVIEW")
        .await
        .expect("loads preview");

    assert!(preview.is_group_booking);
}
