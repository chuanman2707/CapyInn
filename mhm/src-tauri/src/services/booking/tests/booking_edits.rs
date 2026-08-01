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
async fn set_booking_rate_allows_a_total_below_what_the_guest_already_paid() {
    let pool = test_pool().await;
    seed_two_night_booking(&pool, "B807", "R807").await.unwrap();
    sqlx::query("UPDATE bookings SET paid_amount = 1000000 WHERE id = ?")
        .bind("B807")
        .execute(&pool)
        .await
        .unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B807", 300_000)
        .await
        .unwrap();

    let row = sqlx::query("SELECT total_price, paid_amount FROM bookings WHERE id = ?")
        .bind("B807")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("total_price"), 600_000);
    assert_eq!(
        row.get::<i64, _>("paid_amount"),
        1_000_000,
        "trả dư là hợp lệ; số đã trả không bị đụng tới"
    );
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

    let before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B810")
            .fetch_one(&pool)
            .await
            .unwrap();

    stay_lifecycle::set_booking_rate(&pool, "B810", 450_000)
        .await
        .unwrap();

    let after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B810")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after, "sửa giá không được đụng tới lịch phòng");
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
