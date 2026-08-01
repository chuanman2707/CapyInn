use super::prelude::*;
use crate::domain::booking::BookingResult;
use sqlx::{Pool, Sqlite};

/// `seed_active_booking` tạo booking 1 đêm, tổng 250.000₫.
/// Nâng lên 2 đêm / 1.000.000₫ để phép chia giá-mỗi-đêm có ý nghĩa.
async fn seed_two_night_booking(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
) -> BookingResult<()> {
    seed_room(pool, room_id).await?;
    seed_active_booking(pool, booking_id, room_id).await?;
    sqlx::query("UPDATE bookings SET nights = 2, total_price = 1000000 WHERE id = ?")
        .bind(booking_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tokio::test]
async fn set_booking_rate_recomputes_the_total_from_nights() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B801", "R801").await.unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B801", 450_000)
        .await
        .unwrap();

    let row = sqlx::query(
        "SELECT nights, total_price, expected_checkout FROM bookings WHERE id = ?",
    )
    .bind("B801")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<i64, _>("total_price"), 900_000);
    assert_eq!(row.get::<i32, _>("nights"), 2, "số đêm không được đổi");
    assert_eq!(
        row.get::<String, _>("expected_checkout"),
        "2026-04-16T10:00:00+07:00",
        "ngày checkout không được đổi"
    );
}

#[tokio::test]
async fn set_booking_rate_records_the_difference_as_a_charge() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B802", "R802").await.unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B802", 450_000)
        .await
        .unwrap();

    let amount: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B802")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(amount, -100_000, "1.000.000 → 900.000 là chênh lệch -100.000");
}

#[tokio::test]
async fn set_booking_rate_raises_the_total_and_records_a_positive_charge() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B811", "R811").await.unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B811", 600_000)
        .await
        .unwrap();

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind("B811")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1_200_000, "2 đêm x 600.000 phải nâng tổng tiền lên");

    let amount: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B811")
    .fetch_one(&pool)
    .await
    .unwrap();
    // Mọi test khác ở đây đều giảm giá hoặc giữ nguyên; nếu tham số của
    // `checked_sub_money` bị đảo (new_total, current_total) → (current_total,
    // new_total), test giảm giá vẫn xanh vì trị tuyệt đối trùng nhau, chỉ có
    // ca tăng giá này mới lộ dấu bị lật.
    assert_eq!(
        amount, 200_000,
        "1.000.000 → 1.200.000 là chênh lệch +200.000"
    );
}

#[tokio::test]
async fn set_booking_rate_rejects_a_corrupted_zero_nights_booking() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B813", "R813").await.unwrap();

    // `bookings.nights` has no CHECK constraint at the schema level — corrupt
    // it directly via SQL to simulate a value the app itself would never
    // write, mirroring the equivalent guard test in `shorten_stay.rs`.
    sqlx::query("UPDATE bookings SET nights = 0 WHERE id = ?")
        .bind("B813")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::set_booking_rate(&pool, "B813", 450_000)
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(_)),
        "mong đợi lỗi validation cho nights=0, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B813")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row.get::<i32, _>("nights"),
        0,
        "thất bại thì không được đổi giá trị nights đã hỏng"
    );
    assert_eq!(
        row.get::<i64, _>("total_price"),
        1_000_000,
        "thất bại thì total_price không đổi"
    );

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B813")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 0, "thất bại thì không được ghi giao dịch nào");
}

#[tokio::test]
async fn set_booking_rate_rejects_a_corrupted_negative_nights_booking() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B814", "R814").await.unwrap();

    // Same defence-in-depth guard, negative side: without it, `rate_per_night
    // * nights` still "succeeds" arithmetically but flips the sign, inventing
    // a negative total_price and a matching negative audit row instead of
    // rejecting the corrupt row.
    sqlx::query("UPDATE bookings SET nights = -1 WHERE id = ?")
        .bind("B814")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::set_booking_rate(&pool, "B814", 450_000)
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(_)),
        "mong đợi lỗi validation cho nights âm, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B814")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row.get::<i32, _>("nights"),
        -1,
        "thất bại thì không được đổi giá trị nights đã hỏng"
    );
    assert_eq!(
        row.get::<i64, _>("total_price"),
        1_000_000,
        "thất bại thì total_price không được bịa thêm tiền"
    );

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B814")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        charges, 0,
        "thất bại thì không được ghi giao dịch phát sinh tiền ảo nào"
    );
}

#[tokio::test]
async fn set_booking_rate_idempotent_retry_applies_the_rate_change_once() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B815", "R815").await.unwrap();

    let ctx = cmd("set_booking_rate", "rate-key-1");

    stay_lifecycle::set_booking_rate_idempotent(&pool, &ctx, "B815", 450_000)
        .await
        .unwrap();
    stay_lifecycle::set_booking_rate_idempotent(&pool, &ctx, "B815", 450_000)
        .await
        .unwrap();

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind("B815")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 900_000, "gửi lại không được áp lại giá lần hai");

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B815")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 1, "chỉ được một dòng đối ứng duy nhất");
}

#[tokio::test]
async fn set_booking_rate_writes_no_charge_when_the_total_is_unchanged() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B803", "R803").await.unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B803", 500_000)
        .await
        .unwrap();

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B803")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 0);
}

#[tokio::test]
async fn set_booking_rate_leaves_the_room_base_price_alone() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B804", "R804").await.unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B804", 450_000)
        .await
        .unwrap();

    let base_price: i64 = sqlx::query_scalar("SELECT base_price FROM rooms WHERE id = ?")
        .bind("R804")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        base_price, 250_000,
        "sửa giá một booking không được lan sang giá mặc định của phòng"
    );
}

#[tokio::test]
async fn set_booking_rate_rejects_zero_and_negative_rates() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B805", "R805").await.unwrap();

    for rate in [0_i64, -1_000] {
        let error = stay_lifecycle::set_booking_rate(&pool, "B805", rate)
            .await
            .unwrap_err();
        assert!(
            matches!(error, BookingError::Validation(_)),
            "giá {rate} phải bị từ chối, nhận được: {error:?}"
        );
    }

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind("B805")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1_000_000, "lệnh bị từ chối thì DB không được đổi");
}

#[tokio::test]
async fn set_booking_rate_rejects_a_rate_above_the_cap() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B806", "R806").await.unwrap();

    let error = stay_lifecycle::set_booking_rate(&pool, "B806", 100_000_001)
        .await
        .unwrap_err();
    assert!(matches!(error, BookingError::Validation(_)));
}

#[tokio::test]
async fn set_booking_rate_refuses_a_total_below_what_the_guest_already_paid() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B807", "R807").await.unwrap();
    sqlx::query("UPDATE bookings SET paid_amount = 1000000 WHERE id = ?")
        .bind("B807")
        .execute(&pool)
        .await
        .unwrap();

    // Đảo ngược quyết định cũ: hạ giá từng được phép đưa tổng tiền xuống dưới
    // số khách đã trả (2 đêm x 300.000 = 600.000 < 1.000.000 đã thu), với giả
    // định lễ tân sẽ hoàn tiền mặt tại quầy. Giả định đó sai — check_out_tx
    // (stay_lifecycle.rs) từ chối thẳng khi already_paid > final_total, nên
    // booking rơi vào trạng thái không lối thoát. Quyết định mới: từ chối
    // ngay tại đây, trước khi ghi bất kỳ thay đổi nào.
    let error = stay_lifecycle::set_booking_rate(&pool, "B807", 300_000)
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(ref message) if message.contains("hoàn tiền")),
        "mong đợi lỗi validation nhắc hoàn tiền, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT total_price, paid_amount FROM bookings WHERE id = ?")
        .bind("B807")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row.get::<i64, _>("total_price"),
        1_000_000,
        "bị từ chối thì tổng tiền không được đổi"
    );
    assert_eq!(
        row.get::<i64, _>("paid_amount"),
        1_000_000,
        "bị từ chối thì số đã trả không bị đụng tới"
    );

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B807")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 0, "bị từ chối thì không được ghi giao dịch nào");
}

#[tokio::test]
async fn set_booking_rate_allows_a_total_that_still_covers_what_the_guest_already_paid() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B816", "R816").await.unwrap();
    sqlx::query("UPDATE bookings SET paid_amount = 500000 WHERE id = ?")
        .bind("B816")
        .execute(&pool)
        .await
        .unwrap();

    // 2 đêm x 300.000 = 600.000, vẫn >= 500.000 đã trả — phải cho qua.
    stay_lifecycle::set_booking_rate(&pool, "B816", 300_000)
        .await
        .unwrap();

    let row = sqlx::query("SELECT total_price, paid_amount FROM bookings WHERE id = ?")
        .bind("B816")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("total_price"), 600_000);
    assert_eq!(row.get::<i64, _>("paid_amount"), 500_000);
}

#[tokio::test]
async fn set_booking_rate_allows_a_total_exactly_equal_to_paid_amount() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B817", "R817").await.unwrap();
    sqlx::query("UPDATE bookings SET paid_amount = 600000 WHERE id = ?")
        .bind("B817")
        .execute(&pool)
        .await
        .unwrap();

    // Biên chính xác: 2 đêm x 300.000 = 600.000, đúng bằng số đã trả — guard
    // dùng `<` chứ không phải `<=`, nên ca vừa khít này phải được cho qua.
    stay_lifecycle::set_booking_rate(&pool, "B817", 300_000)
        .await
        .unwrap();

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind("B817")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 600_000);
}

#[tokio::test]
async fn set_booking_rate_writes_nothing_when_the_booking_is_reserved_not_active() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B809", "R809").await.unwrap();

    sqlx::query("UPDATE bookings SET status = 'reserved' WHERE id = ?")
        .bind("B809")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::set_booking_rate(&pool, "B809", 450_000)
        .await
        .unwrap_err();
    // `set_booking_rate_tx` rejects a non-active booking via `invalid_state_transition`,
    // which always maps to `BookingError::Conflict`, not `Validation`/`NotFound` — see
    // `invalid_state_transition` in support.rs and the same correction in shorten_stay.rs.
    assert!(matches!(
        error,
        BookingError::Conflict(_)
    ));

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind("B809")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1_000_000, "lệnh bị chặn thì tổng tiền không được đổi");

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B809")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 0, "bị chặn thì không được để lại dòng đối ứng");
}

#[tokio::test]
async fn set_booking_rate_leaves_the_room_calendar_alone() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B810", "R810").await.unwrap();

    // So khớp toàn bộ nội dung dòng, không chỉ đếm số dòng — một UPDATE
    // room_calendar SET status = ... tại chỗ vẫn giữ nguyên COUNT(*) nhưng
    // đổi status, nên phải so cả cột mới bắt được.
    let before: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT room_id, date, booking_id, status FROM room_calendar
         WHERE booking_id = ? ORDER BY date",
    )
    .bind("B810")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !before.is_empty(),
        "seed phải tạo ít nhất một dòng lịch phòng để test này có ý nghĩa"
    );

    stay_lifecycle::set_booking_rate(&pool, "B810", 450_000)
        .await
        .unwrap();

    let after: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT room_id, date, booking_id, status FROM room_calendar
         WHERE booking_id = ? ORDER BY date",
    )
    .bind("B810")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "sửa giá không được đụng tới lịch phòng, kể cả sửa tại chỗ giữ nguyên số dòng"
    );
}

#[tokio::test]
async fn set_booking_rate_refuses_a_booking_that_is_not_active() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B808", "R808").await.unwrap();
    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind("B808")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::set_booking_rate(&pool, "B808", 450_000)
        .await
        .unwrap_err();
    // Same as above: a checked-out booking is rejected via `invalid_state_transition`,
    // which is `BookingError::Conflict`.
    assert!(matches!(
        error,
        BookingError::Conflict(_)
    ));
}

#[tokio::test]
async fn update_booking_notes_trims_and_saves() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B901", "R901").await.unwrap();

    stay_lifecycle::update_booking_notes(
        &pool,
        "B901",
        Some("  Khách xin thêm gối  ".to_string()),
    )
    .await
    .unwrap();

    let notes: Option<String> = sqlx::query_scalar("SELECT notes FROM bookings WHERE id = ?")
        .bind("B901")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(notes.as_deref(), Some("Khách xin thêm gối"));
}

#[tokio::test]
async fn update_booking_notes_stores_blank_input_as_null() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B902", "R902").await.unwrap();

    stay_lifecycle::update_booking_notes(&pool, "B902", Some("   ".to_string()))
        .await
        .unwrap();

    let notes: Option<String> = sqlx::query_scalar("SELECT notes FROM bookings WHERE id = ?")
        .bind("B902")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(notes, None);
}

#[tokio::test]
async fn update_booking_notes_rejects_notes_over_the_limit() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B903", "R903").await.unwrap();

    let too_long = "a".repeat(2_001);
    let error = stay_lifecycle::update_booking_notes(&pool, "B903", Some(too_long))
        .await
        .unwrap_err();
    assert!(matches!(error, BookingError::Validation(_)));

    let notes: Option<String> = sqlx::query_scalar("SELECT notes FROM bookings WHERE id = ?")
        .bind("B903")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        notes.as_deref(),
        Some("seed booking"),
        "bị từ chối thì ghi chú cũ phải còn nguyên, không bị cắt bớt"
    );
}

#[tokio::test]
async fn update_booking_notes_refuses_a_booking_that_is_not_active() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B904", "R904").await.unwrap();
    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind("B904")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::update_booking_notes(&pool, "B904", Some("x".to_string()))
        .await
        .unwrap_err();
    // `update_booking_notes` has no separate SELECT+status check before the
    // UPDATE — it relies on `WHERE status = ACTIVE` plus `ensure_one_row_affected`,
    // same as `set_booking_rate`'s equivalent test above. That path always
    // reports a state-transition problem via `invalid_state_transition`, i.e.
    // `BookingError::Conflict`, never `Validation`/`NotFound`.
    assert!(matches!(error, BookingError::Conflict(_)));
}
