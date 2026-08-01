use super::prelude::*;
use chrono::Datelike;
use crate::domain::booking::BookingResult;
use sqlx::{Pool, Sqlite};

/// Thứ Hai kế tiếp, luôn cách hôm nay ít nhất 1 ngày.
///
/// Neo vào thứ Hai để các đêm trong test (Hai, Ba, Tư, Năm) không bao giờ chạm
/// cuối tuần — `calculate_weekend_uplift` cộng 20% cho đêm Bảy/Chủ Nhật, và một
/// fixture neo thẳng vào `Local::now()` sẽ cho số tiền khác nhau tuỳ ngày chạy.
fn next_monday() -> NaiveDate {
    let today = Local::now().date_naive();
    let days_ahead = 7 - i64::from(today.weekday().num_days_from_monday());
    today + Duration::days(days_ahead)
}

/// Booking `nights` đêm bắt đầu từ `check_in`, kèm đủ dòng `room_calendar`.
/// Trả về `(check_in_at, expected_checkout)` dạng RFC3339.
async fn seed_booking_from(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
    check_in: NaiveDate,
    nights: i64,
) -> BookingResult<(String, String)> {
    seed_active_booking(pool, booking_id, room_id).await?;

    let checkout = check_in + Duration::days(nights);
    let check_in_at = format!("{}T14:00:00+07:00", check_in.format("%Y-%m-%d"));
    let expected_checkout = format!("{}T12:00:00+07:00", checkout.format("%Y-%m-%d"));

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, nights = ?, total_price = ?
         WHERE id = ?",
    )
    .bind(&check_in_at)
    .bind(&expected_checkout)
    .bind(nights)
    .bind(250_000 * nights)
    .bind(booking_id)
    .execute(pool)
    .await?;

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind(booking_id)
        .execute(pool)
        .await?;

    for offset in 0..nights {
        let night = check_in + Duration::days(offset);
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status)
             VALUES (?, ?, ?, 'occupied')",
        )
        .bind(room_id)
        .bind(night.format("%Y-%m-%d").to_string())
        .bind(booking_id)
        .execute(pool)
        .await?;
    }

    Ok((check_in_at, expected_checkout))
}

/// Booking `nights` đêm bắt đầu từ thứ Hai kế tiếp, kèm đủ dòng `room_calendar`.
/// Trả về `(check_in_at, expected_checkout)` dạng RFC3339.
async fn seed_future_booking(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
    nights: i64,
) -> BookingResult<(String, String)> {
    seed_booking_from(pool, booking_id, room_id, next_monday(), nights).await
}

/// Ngày (không giờ) từ một chuỗi `check_in_at`/`expected_checkout` RFC3339 mà
/// các fixture ở trên trả về — dùng để tính lại đêm bị rút mà không phải gọi
/// lại `next_monday()`/`Local::now()` một lần nữa giữa chừng test (xem mục 6
/// trong review: gọi lại có thể lệch múi giờ nếu đồng hồ vượt qua nửa đêm).
fn date_from_rfc3339(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(&value[..10], "%Y-%m-%d").expect("fixture date must be YYYY-MM-DD…")
}

async fn charge_total(pool: &Pool<Sqlite>, booking_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0) FROM transactions
         WHERE booking_id = ? AND type = 'charge'",
    )
    .bind(booking_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn shorten_stay_drops_the_last_night_and_its_charge() {
    let pool = test_pool().await;
    seed_room(&pool, "R701").await.unwrap();
    let (check_in, checkout) = seed_future_booking(&pool, "B701", "R701", 3)
        .await
        .unwrap();

    stay_lifecycle::shorten_stay(&pool, "B701").await.unwrap();

    let row = sqlx::query("SELECT nights, total_price, expected_checkout FROM bookings WHERE id = ?")
        .bind("B701")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 2);
    assert_eq!(row.get::<i64, _>("total_price"), 500_000);

    let check_in_date = date_from_rfc3339(&check_in);
    let freed_night = check_in_date + Duration::days(2);
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND date = ?",
    )
    .bind("B701")
    .bind(freed_night.format("%Y-%m-%d").to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 0, "đêm bị rút phải được nhả khỏi lịch phòng");

    // Checkout mới phải lùi lại đúng 1 ngày so với checkout ban đầu — tức
    // đúng bằng ngày của đêm vừa bị rút, giữ nguyên giờ trả phòng.
    let original_checkout_time = &checkout[10..];
    let expected_new_checkout = format!(
        "{}{}",
        freed_night.format("%Y-%m-%d"),
        original_checkout_time
    );
    assert_eq!(
        row.get::<String, _>("expected_checkout"),
        expected_new_checkout,
        "checkout mới phải lùi đúng 1 ngày so với checkout cũ"
    );

    let credit: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = ? AND note = 'Shortened stay -1 night'",
    )
    .bind("B701")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(credit, -250_000);
}

#[tokio::test]
async fn extend_then_shorten_restores_the_booking_exactly() {
    let pool = test_pool().await;
    seed_room(&pool, "R702").await.unwrap();
    let (_check_in, checkout) = seed_future_booking(&pool, "B702", "R702", 2)
        .await
        .unwrap();

    let before_charges = charge_total(&pool, "B702").await;

    stay_lifecycle::extend_stay(&pool, "B702").await.unwrap();

    // Chốt trạng thái trung gian: nếu cả extend lẫn shorten đều không ghi gì
    // (vd. bug âm thầm bỏ qua record_charge_tx), so sánh mù `before_charges ==
    // charge_total sau shorten` vẫn xanh vì cả hai đều bằng 0. Đọc lại đúng
    // khoản charge mà extend vừa ghi để khoá chặt lỗ hổng đó.
    let extend_charge: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = ? AND note = 'Extended stay +1 night'",
    )
    .bind("B702")
    .fetch_one(&pool)
    .await
    .unwrap();
    let after_extend_charges = charge_total(&pool, "B702").await;
    assert_eq!(
        after_extend_charges,
        before_charges + extend_charge,
        "sau extend, tổng charge phải cộng đúng khoản charge của đêm thêm"
    );

    stay_lifecycle::shorten_stay(&pool, "B702").await.unwrap();

    let row = sqlx::query("SELECT nights, total_price, expected_checkout FROM bookings WHERE id = ?")
        .bind("B702")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 2);
    assert_eq!(row.get::<i64, _>("total_price"), 500_000);
    assert_eq!(row.get::<String, _>("expected_checkout"), checkout);

    assert_eq!(
        charge_total(&pool, "B702").await,
        before_charges,
        "dòng charge của đêm thêm phải được dòng đối ứng bù hết"
    );

    let nights_held: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B702")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(nights_held, 2, "lịch phòng phải về đúng 2 đêm ban đầu");
}

#[tokio::test]
async fn shorten_stay_refuses_to_push_checkout_into_the_past() {
    let pool = test_pool().await;
    seed_room(&pool, "R703").await.unwrap();
    seed_active_booking(&pool, "B703", "R703").await.unwrap();

    // Ngày seed mặc định là tháng 4/2026 — đã là quá khứ, nên đây đúng là ca cần chặn.
    let today = Local::now().date_naive();
    let checkout = today.format("%Y-%m-%dT12:00:00+07:00").to_string();
    let check_in = (today - Duration::days(5))
        .format("%Y-%m-%dT14:00:00+07:00")
        .to_string();
    sqlx::query(
        "UPDATE bookings SET check_in_at = ?, expected_checkout = ?, nights = 5 WHERE id = ?",
    )
    .bind(&check_in)
    .bind(&checkout)
    .bind("B703")
    .execute(&pool)
    .await
    .unwrap();

    let error = stay_lifecycle::shorten_stay(&pool, "B703")
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(ref message) if message.contains("Check-out")),
        "mong đợi lỗi validation nhắc dùng Check-out, nhận được: {error:?}"
    );

    let unchanged = sqlx::query_scalar::<_, String>(
        "SELECT expected_checkout FROM bookings WHERE id = ?",
    )
    .bind("B703")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unchanged, checkout, "lệnh thất bại thì DB không được đổi");
}

#[tokio::test]
async fn shorten_stay_allows_moving_the_checkout_to_today() {
    let pool = test_pool().await;
    seed_room(&pool, "R708").await.unwrap();

    // Checkout là ngày mai — một khách trả phòng sớm hơn 1 ngày so với dự
    // kiến là ca hợp lệ (khách rời trong ngày hôm nay), phải được cho phép,
    // khác với B703 ở trên nơi checkout mới sẽ rơi vào quá khứ.
    let today = Local::now().date_naive();
    let check_in = today - Duration::days(1);
    let (_check_in_at, checkout) = seed_booking_from(&pool, "B708", "R708", check_in, 2)
        .await
        .unwrap();

    let booking = stay_lifecycle::shorten_stay(&pool, "B708").await.unwrap();

    let expected_new_checkout = format!("{}{}", today.format("%Y-%m-%d"), &checkout[10..]);
    assert_eq!(
        booking.expected_checkout, expected_new_checkout,
        "checkout mới phải là hôm nay"
    );

    let row = sqlx::query("SELECT nights, expected_checkout FROM bookings WHERE id = ?")
        .bind("B708")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 1);
    assert_eq!(
        row.get::<String, _>("expected_checkout"),
        expected_new_checkout,
        "trạng thái đọc lại từ DB phải khớp checkout hôm nay"
    );
}

#[tokio::test]
async fn shorten_stay_refuses_to_go_below_one_night() {
    let pool = test_pool().await;
    seed_room(&pool, "R704").await.unwrap();
    seed_future_booking(&pool, "B704", "R704", 1).await.unwrap();

    let error = stay_lifecycle::shorten_stay(&pool, "B704")
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(ref message) if message.contains("tối thiểu 1 đêm")),
        "mong đợi lỗi tối thiểu 1 đêm, nhận được: {error:?}"
    );

    let nights = sqlx::query_scalar::<_, i32>("SELECT nights FROM bookings WHERE id = ?")
        .bind("B704")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(nights, 1);
}

#[tokio::test]
async fn shorten_stay_refuses_a_booking_that_is_not_active() {
    let pool = test_pool().await;
    seed_room(&pool, "R705").await.unwrap();
    seed_future_booking(&pool, "B705", "R705", 3).await.unwrap();
    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind("B705")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::shorten_stay(&pool, "B705")
        .await
        .unwrap_err();
    // `invalid_state_transition` (used by the sibling `extend_stay_tx` for the
    // exact same not-active check) always maps to `BookingError::Conflict`, not
    // `Validation` — see `services::booking::support::invalid_state_transition`.
    assert!(
        matches!(error, BookingError::Conflict(_)),
        "mong đợi lỗi trạng thái không hợp lệ, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B705")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 3, "thất bại thì số đêm không đổi");
    assert_eq!(
        row.get::<i64, _>("total_price"),
        750_000,
        "thất bại thì tổng tiền không đổi"
    );
}

#[tokio::test]
async fn shorten_stay_credits_the_booking_average_not_the_current_rate() {
    let pool = test_pool().await;
    seed_room(&pool, "R709").await.unwrap();
    let (check_in, _checkout) = seed_future_booking(&pool, "B709", "R709", 2)
        .await
        .unwrap();

    // Booking 2 đêm x 250,000 (tổng 500,000) được chốt giá lúc nhận phòng.
    // Sau đó một dòng pricing_rules đẩy giá phòng lên 900,000/đêm — nếu
    // shorten_stay hỏi lại pricing engine, nó sẽ hoàn 900,000 cho một đêm chỉ
    // thu có 250,000. Quyết định của product owner: hoàn theo trung bình của
    // CHÍNH booking này (500,000 / 2 = 250,000), bỏ qua giá mới hoàn toàn.
    seed_pricing_rule(&pool, "standard", 900_000).await.unwrap();

    stay_lifecycle::shorten_stay(&pool, "B709").await.unwrap();

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B709")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 1);
    assert_eq!(
        row.get::<i64, _>("total_price"),
        250_000,
        "khoản hoàn phải bằng trung bình gốc của booking (250,000), không phải giá mới (900,000)"
    );

    let check_in_date = date_from_rfc3339(&check_in);
    let freed_night = check_in_date + Duration::days(1);
    let calendar_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND date = ?",
    )
    .bind("B709")
    .bind(freed_night.format("%Y-%m-%d").to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_rows, 0, "đêm bị rút phải được nhả khỏi lịch phòng");

    let credit: i64 = sqlx::query_scalar(
        "SELECT amount FROM transactions WHERE booking_id = ? AND note = 'Shortened stay -1 night'",
    )
    .bind("B709")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        credit, -250_000,
        "dòng charge phải trừ đúng trung bình gốc, không phải giá mới sau khi pricing_rules đổi"
    );
}

#[tokio::test]
async fn shorten_stay_rejects_a_zero_nights_booking_instead_of_panicking() {
    let pool = test_pool().await;
    seed_room(&pool, "R710").await.unwrap();
    seed_future_booking(&pool, "B710", "R710", 3).await.unwrap();

    // `bookings.nights` has no CHECK constraint at the schema level — corrupt
    // it directly via SQL to simulate a value the app itself would never
    // write, and pin that the guard in shorten_stay_tx turns this into a
    // validation error instead of "attempt to divide by zero".
    sqlx::query("UPDATE bookings SET nights = 0 WHERE id = ?")
        .bind("B710")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::shorten_stay(&pool, "B710")
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(_)),
        "mong đợi lỗi validation cho nights=0, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B710")
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
        750_000,
        "thất bại thì total_price không đổi"
    );

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B710")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charges, 0, "thất bại thì không được ghi giao dịch nào");
}

#[tokio::test]
async fn shorten_stay_rejects_a_negative_nights_booking_instead_of_inventing_money() {
    let pool = test_pool().await;
    seed_room(&pool, "R711").await.unwrap();
    seed_future_booking(&pool, "B711", "R711", 3).await.unwrap();

    // Same defence-in-depth guard, negative side: without it, `current_total /
    // current_nights` still "succeeds" arithmetically but flips the sign,
    // turning a credit into a charge and inflating total_price instead of
    // shrinking it — a reviewer measured a 750,000/3-night booking coming back
    // with total_price = 1,500,000 and a +750,000 transaction mislabeled
    // "Shortened stay -1 night".
    sqlx::query("UPDATE bookings SET nights = -1 WHERE id = ?")
        .bind("B711")
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::shorten_stay(&pool, "B711")
        .await
        .unwrap_err();
    assert!(
        matches!(error, BookingError::Validation(_)),
        "mong đợi lỗi validation cho nights âm, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B711")
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
        750_000,
        "thất bại thì total_price không được bịa thêm tiền"
    );

    let charges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B711")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        charges, 0,
        "thất bại thì không được ghi giao dịch phát sinh tiền ảo nào"
    );
}

#[tokio::test]
async fn shorten_stay_leaves_everything_alone_when_the_calendar_row_is_missing() {
    let pool = test_pool().await;
    seed_room(&pool, "R707").await.unwrap();
    let (check_in, _checkout) = seed_future_booking(&pool, "B707", "R707", 3)
        .await
        .unwrap();

    // Mô phỏng ai đó động vào lịch phòng giữa chừng: xoá đúng dòng mà
    // shorten_stay sắp gỡ. Chốt `ensure_one_row_affected` trên câu DELETE phải
    // bắt được và kéo ngược toàn bộ transaction.
    let check_in_date = date_from_rfc3339(&check_in);
    let freed_night = check_in_date + Duration::days(2);
    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ? AND date = ?")
        .bind("B707")
        .bind(freed_night.format("%Y-%m-%d").to_string())
        .execute(&pool)
        .await
        .unwrap();

    let error = stay_lifecycle::shorten_stay(&pool, "B707")
        .await
        .unwrap_err();
    // `ensure_one_row_affected` (mirroring `extend_stay_tx`'s own use of it)
    // always maps to `invalid_state_transition`, i.e. `BookingError::Conflict`,
    // not `Validation` — see `services::booking::support::ensure_one_row_affected`.
    assert!(
        matches!(error, BookingError::Conflict(_)),
        "mong đợi lỗi trạng thái không hợp lệ, nhận được: {error:?}"
    );

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B707")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 3, "rollback phải giữ nguyên số đêm");
    assert_eq!(row.get::<i64, _>("total_price"), 750_000);

    let credits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind("B707")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(credits, 0, "rollback thì không được để lại dòng đối ứng nào");
}

#[tokio::test]
async fn shorten_stay_idempotent_retry_replays_without_removing_a_second_night() {
    let pool = test_pool().await;
    seed_room(&pool, "R706").await.unwrap();
    seed_future_booking(&pool, "B706", "R706", 3).await.unwrap();

    let ctx = cmd("shorten_stay", "shorten-key-1");

    stay_lifecycle::shorten_stay_idempotent(&pool, &ctx, "B706")
        .await
        .unwrap();
    stay_lifecycle::shorten_stay_idempotent(&pool, &ctx, "B706")
        .await
        .unwrap();

    let row = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B706")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("nights"), 2, "gửi lại chỉ được rút một đêm");
    assert_eq!(row.get::<i64, _>("total_price"), 500_000);

    let credits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND note = 'Shortened stay -1 night'",
    )
    .bind("B706")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(credits, 1, "chỉ được một dòng đối ứng duy nhất");
}
