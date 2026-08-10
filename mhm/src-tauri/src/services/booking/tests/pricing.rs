use super::prelude::*;
use crate::models::CheckInRequest;

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_pricing_rule() {
    let pool = test_pool().await;
    seed_room(&pool, "R150").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    seed_pricing_rule_tx(&mut tx, "standard", 600_000)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R150",
        "2026-04-15T10:00:00+07:00",
        "2026-04-17T10:00:00+07:00",
        "nightly",
        None,
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_200_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_applies_special_date_uplift() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R149", 600_000).await.unwrap();
    seed_special_date(&pool, "2026-04-20", 10.0).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R149",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
        None,
    )
    .await
    .unwrap();

    assert_eq!(pricing.pricing_type, "nightly");
    // Kỳ ở 20/04→22/04 là hai đêm 20 và 21. Chỉ 20/04 được khai là ngày lễ
    // (10%); 21/04 thì không. Mức bình quân (10 + 0) / 2 = 5%, không phải 10%
    // cho cả hai đêm như luật cũ (tính theo ngày check-in).
    assert_eq!(pricing.total, 1_260_000);
    assert_eq!(pricing.base_amount, 1_200_000);
    assert_eq!(pricing.surcharge_amount, 60_000);
    assert_eq!(pricing.weekend_amount, 0);
    assert_eq!(pricing.breakdown.len(), 2);
    assert_eq!(pricing.breakdown[0].amount, 1_200_000);
    assert!(pricing.breakdown[0].label.contains("đêm"));
    assert!(pricing
        .breakdown
        .iter()
        .any(|line| line.label == "Phụ thu ngày lễ"));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_returns_not_found_for_missing_room() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let error = calculate_stay_price_tx(
        &mut tx,
        "missing-room",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
        None,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::NotFound(message) if message.contains("Không tìm thấy phòng missing-room")
    ));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_returns_datetime_parse_for_invalid_check_in() {
    let pool = test_pool().await;
    seed_room(&pool, "R153").await.unwrap();
    let mut tx = pool.begin().await.unwrap();

    let error = calculate_stay_price_tx(
        &mut tx,
        "R153",
        "not-a-datetime",
        "2026-04-22T10:00:00+07:00",
        "nightly",
        None,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
    ));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_room_base_price() {
    let pool = test_pool().await;
    seed_room(&pool, "R151").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE rooms SET base_price = ? WHERE id = ?")
        .bind(600_000)
        .bind("R151")
        .execute(&mut *tx)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R151",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
        None,
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_200_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn room_price_preview_matches_what_the_reservation_will_be_charged() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R340", 500_000, 2, 50_000)
        .await
        .unwrap();

    let preview = pricing_service::calculate_room_price_preview(
        &pool,
        "R340",
        "2026-08-06",
        "2026-08-08",
        "nightly",
        Some(4),
    )
    .await
    .unwrap();

    assert_eq!(preview.total, 1_200_000);
    assert_eq!(preview.breakdown.len(), 2);
    assert_eq!(preview.breakdown[1].label, "Phụ thu 2 khách");
}

/// Two rooms of the same type with different surcharges. The room-keyed
/// preview must quote the room actually asked for, not whichever same-type
/// row a bare `LIMIT 1` would happen to return.
///
/// `R339` is seeded first so it lands on the lower rowid: `rooms` has no
/// index on `type`, so `WHERE LOWER(type) = ? LIMIT 1` (the type-keyed query)
/// resolves via a full table scan in rowid order and would return `R339` —
/// not `R340`, the room the preview is actually for. If the room-keyed loader
/// were ever swapped to the type-keyed query, this test would silently start
/// quoting `R339`'s surcharge for a preview requested against `R340`.
#[tokio::test]
async fn room_price_preview_uses_its_own_rooms_surcharge_not_a_same_type_sibling() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R339", 500_000, 2, 900_000)
        .await
        .unwrap();
    seed_room_with_guest_pricing(&pool, "R340", 500_000, 2, 50_000)
        .await
        .unwrap();

    let preview = pricing_service::calculate_room_price_preview(
        &pool,
        "R340",
        "2026-08-06",
        "2026-08-08",
        "nightly",
        Some(4),
    )
    .await
    .unwrap();

    // base: 500_000 * 2 nights = 1_000_000
    // surcharge: R340's own 50_000/night/extra-guest * 2 extra guests * 2 nights = 200_000
    // If R339's 900_000 fee leaked in instead, surcharge would be 3_600_000 and total 4_600_000.
    assert_eq!(preview.total, 1_200_000);
    assert_eq!(preview.breakdown.len(), 2);
    assert_eq!(preview.breakdown[1].label, "Phụ thu 2 khách");
    assert_eq!(preview.breakdown[1].amount, 200_000);
}

/// Cái toàn bộ thiết kế của nhánh này đặt cược vào: bản xem trước theo phòng
/// (`calculate_room_price_preview`) phải trả đúng con số sẽ bị tính khi ghi
/// sổ (`calculate_stay_price_tx`) — kể cả khi có phụ thu thêm khách, không
/// chỉ khi `guests: None`. Hai bài `room_price_preview_matches_...` ở trên chỉ
/// so `preview.total` với một con số hằng (`1_200_000`); bài
/// `the_preview_and_the_lifecycle_charge_agree_on_every_reachable_rule_source`
/// trong `pricing_service.rs` so cấu trúc thật nhưng chỉ chạy với
/// `guests: None`. Bài này là bài giữ lời hứa đó khỏi mục rữa: so cấu trúc
/// (breakdown nhãn + số tiền, không chỉ tổng) với `guests: Some(4)`.
#[tokio::test]
async fn room_price_preview_matches_the_lifecycle_charge_structurally_with_extra_guests() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R341", 500_000, 2, 50_000)
        .await
        .unwrap();

    let preview = pricing_service::calculate_room_price_preview(
        &pool,
        "R341",
        "2026-08-06",
        "2026-08-08",
        "nightly",
        Some(4),
    )
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let charged = calculate_stay_price_tx(
        &mut tx,
        "R341",
        "2026-08-06",
        "2026-08-08",
        "nightly",
        Some(4),
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(preview.total, charged.total);
    assert_eq!(preview.base_amount, charged.base_amount);
    assert_eq!(preview.surcharge_amount, charged.surcharge_amount);
    assert_eq!(preview.weekend_amount, charged.weekend_amount);
    assert_eq!(preview.breakdown.len(), charged.breakdown.len());
    for (preview_line, charged_line) in preview.breakdown.iter().zip(charged.breakdown.iter()) {
        assert_eq!(preview_line.label, charged_line.label);
        assert_eq!(preview_line.amount, charged_line.amount);
    }

    // Con số thật phải khác 0 và có mặt dòng phụ thu — nếu không, so sánh
    // cấu trúc ở trên có thể trùng khớp một cách vô nghĩa (cả hai đều rỗng).
    assert!(charged.total > 0);
    assert!(charged
        .breakdown
        .iter()
        .any(|line| line.label.contains("khách")));
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_special_date() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R152", 600_000).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    seed_special_date_tx(&mut tx, "2026-04-20", 10.0)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R152",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
        None,
    )
    .await
    .unwrap();

    // Cùng lý do như test ở trên: chỉ đêm 20/04 nằm trong ngày lễ, đêm 21/04
    // thì không, nên mức bình quân là 5% chứ không phải 10%.
    assert_eq!(pricing.total, 1_260_000);

    tx.rollback().await.unwrap();
}

/// `CreateGuestRequest` không derive `Default`, nên ba test dưới dùng chung
/// helper này thay vì liệt kê mười trường mỗi lần.
fn guest(full_name: &str, doc_number: &str) -> CreateGuestRequest {
    CreateGuestRequest {
        guest_type: None,
        full_name: full_name.to_string(),
        doc_number: doc_number.to_string(),
        dob: None,
        gender: None,
        nationality: None,
        address: None,
        visa_expiry: None,
        scan_path: None,
        phone: None,
    }
}

/// Giá tay đè giá engine, và đè PHẲNG: `total_price` luôn đúng bằng
/// `rate × nights`, không cộng thêm dòng nào engine tính.
///
/// Fixture dùng phòng CÓ phụ thu thêm người thật (`seed_room_with_guest_pricing`,
/// không phải `seed_room` mặc định phụ thu 0) — nhưng phụ thu đó không có
/// đường nào lọt vào `total_price` để mà kiểm: `check_in_tx` gọi engine với
/// `guests: None` ở CẢ HAI nhánh (có/không override), và nhánh phụ thu của
/// engine trả về 0 ngay khi thấy `None`, trước khi kịp nhìn tới
/// `extra_person_fee`. Một bug giả định "override nuốt mất phụ thu" (giá tay
/// đáng lẽ phải cộng thêm phụ thu nhưng lại bỏ qua) sẽ xanh với bài test này
/// dù dùng phòng phụ thu 0 hay phụ thu thật — phòng phụ thu thật ở đây chỉ để
/// tài liệu hoá rằng lựa chọn phòng không ảnh hưởng kết quả, không phải một
/// guard chặn bug đó.
#[tokio::test]
async fn manual_rate_at_check_in_overrides_the_engine_price() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R-MR", 500_000, 2, 100_000)
        .await
        .expect("seeds room with an extra-person fee");

    let booking = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR".to_string(),
            guests: vec![guest("Khách mặc cả", "DOC-MR1")],
            nights: 3,
            source: None,
            notes: None,
            paid_amount: None,
            pricing_type: None,
            rate_override_per_night: Some(400_000),
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .expect("checks in with a manual rate");

    assert_eq!(booking.total_price, 1_200_000, "3 đêm × 400.000");
    assert!(
        booking.rate_overridden_at.is_some(),
        "phải đánh dấu là giá tay để quyết toán check-out biết"
    );

    let charges: Vec<(i64, String)> = sqlx::query_as(
        "SELECT amount, note FROM transactions WHERE booking_id = ? AND type = 'charge'",
    )
    .bind(&booking.id)
    .fetch_all(&pool)
    .await
    .expect("reads charges");
    assert_eq!(
        charges.len(),
        1,
        "chỉ một dòng tiền phòng, không có dòng đổi giá"
    );
    assert_eq!(charges[0].0, 1_200_000);

    // Giá engine gốc phải còn dấu vết trong pricing_snapshot — không dùng để
    // tính tiền, nhưng để chủ khách sạn tra được sau này đã giảm giá bao
    // nhiêu. Không ghim giá trị `engine_total` cụ thể: nó phụ thuộc ngày chạy
    // test có rơi vào cuối tuần hay không (mức uplift 20% mặc định cho phòng
    // chưa cấu hình `pricing_rules`), nên chỉ kiểm nó có mặt và là một số.
    let pricing_snapshot: Option<String> =
        sqlx::query_scalar("SELECT pricing_snapshot FROM bookings WHERE id = ?")
            .bind(&booking.id)
            .fetch_one(&pool)
            .await
            .expect("reads pricing_snapshot");
    let snapshot: serde_json::Value = serde_json::from_str(
        &pricing_snapshot.expect("giá tay phải để lại vết tích trong pricing_snapshot"),
    )
    .expect("pricing_snapshot phải là JSON hợp lệ");
    assert_eq!(
        snapshot["manual_rate"]["rate_per_night"], 400_000,
        "phải ghi đúng giá tay đã gõ, không phải giá engine: {snapshot}"
    );
    assert!(
        snapshot["manual_rate"]["engine_total"].is_number(),
        "phải giữ giá engine gốc để chủ khách sạn tra được đã giảm bao nhiêu: {snapshot}"
    );
}

/// Thu nhiều hơn tổng tiền sẽ đẩy booking vào ngõ cụt: `check_out_tx` từ chối
/// thẳng khi `already_paid > final_total`. Chặn ngay lúc tạo.
#[tokio::test]
async fn manual_rate_rejects_paying_more_than_the_total() {
    let pool = test_pool().await;
    seed_room(&pool, "R-MR2").await.expect("seeds room");

    let error = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR2".to_string(),
            guests: vec![guest("Khách trả dư", "DOC-MR2")],
            nights: 1,
            source: None,
            notes: None,
            paid_amount: Some(500_000),
            pricing_type: None,
            rate_override_per_night: Some(400_000),
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .expect_err("overpayment must be rejected at creation");

    assert!(
        error.to_string().contains("cao hơn tổng tiền"),
        "thông báo phải nói rõ thu nhiều hơn tổng, thấy: {error}"
    );
}

/// Giá tay tràn số VÀ có `paid_amount` phải bị chặn ở guard biên, không phải
/// lọt qua rồi vỡ ở phép nhân bên dưới: `checked_mul_money` (qua
/// `validate_transport_money_vnd`) trả thông báo TIẾNG ANH
/// ("total_price must be a safe integer VND value"), và thông báo đó đi thẳng
/// ra người dùng qua command error mapper — dự án này bắt buộc tiếng Việt cho
/// mọi thông báo tới người dùng. Phải có `paid_amount` để chạm đúng nhánh
/// từng vỡ (không có `paid_amount` thì luôn được `check_in_tx` chặn đúng —
/// xem `manual_rate_at_check_in_rejects_a_rate_above_the_cap`).
#[tokio::test]
async fn manual_rate_at_check_in_rejects_a_huge_rate_even_with_paid_amount() {
    let pool = test_pool().await;
    seed_room(&pool, "R-MR7").await.expect("seeds room");

    let error = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR7".to_string(),
            guests: vec![guest("Khách gõ thừa nhiều số 0", "DOC-MR7")],
            nights: 1,
            source: None,
            notes: None,
            paid_amount: Some(100_000),
            pricing_type: None,
            // Vượt xa biên an toàn số nguyên (9_007_199_254_740_991) lẫn
            // MAX_RATE_PER_NIGHT_VND (100_000_000) — mô phỏng dán nhầm/gõ
            // thừa số 0.
            rate_override_per_night: Some(9_500_000_000_000_000),
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Giá mỗi đêm không hợp lệ",
        "phải là thông báo biên tiếng Việt, không phải lỗi tràn số/không an toàn từ phép nhân: {error}"
    );
    assert_room_status(&pool, "R-MR7", "vacant").await;
    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&pool)
        .await
        .expect("đếm booking");
    assert_eq!(
        booking_count, 0,
        "bị từ chối thì không được ghi booking nào"
    );
}

/// Giá âm VÀ có `paid_amount` từng lọt qua phép nhân (âm × đêm dương vẫn ra
/// một số "hợp lệ", không tràn số) rồi vỡ ở guard thu-quá-tổng — guard đó đọc
/// tổng ÂM ra như một mức "tổng tiền" trong câu thông báo, sai cả nội dung
/// lẫn thủ phạm (lỗi thật là giá, không phải số tiền thu). Phải bị chặn ở
/// guard biên trước khi tới đó.
#[tokio::test]
async fn manual_rate_at_check_in_rejects_a_negative_rate_even_with_paid_amount() {
    let pool = test_pool().await;
    seed_room(&pool, "R-MR8").await.expect("seeds room");

    let error = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR8".to_string(),
            guests: vec![guest("Khách gõ nhầm dấu trừ", "DOC-MR8")],
            nights: 1,
            source: None,
            notes: None,
            paid_amount: Some(100_000),
            pricing_type: None,
            rate_override_per_night: Some(-500_000),
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Giá mỗi đêm không hợp lệ",
        "phải là thông báo biên tiếng Việt, không phải guard thu-quá-tổng đọc tổng âm ra như một mức giá: {error}"
    );
    assert_room_status(&pool, "R-MR8", "vacant").await;
    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&pool)
        .await
        .expect("đếm booking");
    assert_eq!(
        booking_count, 0,
        "bị từ chối thì không được ghi booking nào"
    );
}

/// Đường thường (không override) cũng phải bị chặn khi thu vượt tổng — trước
/// khi sửa, guard chỉ nằm ở `validate_check_in_request`, nơi CHỈ biết được
/// tổng khi có giá tay; đường engine tính (không override, con đường mọi lượt
/// check-in bình thường đi qua) lọt qua hoàn toàn, tạo đúng cái ngõ cụt guard
/// này sinh ra để chặn. Dùng `seed_room_with_price` để tổng engine không mơ
/// hồ: phòng không có `pricing_rules` cấu hình sẵn thì ăn theo `base_price`,
/// và `seed_pricing_rule` khai `weekend_uplift_pct = 0` tường minh, nên tổng
/// luôn đúng bằng `base_price × nights` bất kể ngày chạy test.
#[tokio::test]
async fn check_in_without_override_rejects_paying_more_than_the_engine_total() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R-MR9", 500_000)
        .await
        .expect("seeds room with a deterministic engine price");

    let error = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR9".to_string(),
            guests: vec![guest("Khách trả dư không giá tay", "DOC-MR9")],
            nights: 1,
            source: None,
            notes: None,
            // Engine: 1 đêm × 500.000 = 500.000 (không mơ hồ, xem comment
            // trên). Trả 600.000 là vượt tổng.
            paid_amount: Some(600_000),
            pricing_type: None,
            rate_override_per_night: None,
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(error, BookingError::Validation(_)),
        "phải bị từ chối vì thu vượt tổng, nhận được: {error:?}"
    );
    assert!(
        error.to_string().contains("cao hơn tổng tiền"),
        "thông báo phải nói rõ thu nhiều hơn tổng, thấy: {error}"
    );

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&pool)
        .await
        .expect("đếm booking");
    assert_eq!(
        booking_count, 0,
        "bị từ chối thì không được ghi booking nào"
    );

    let transaction_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
        .fetch_one(&pool)
        .await
        .expect("đếm transaction");
    assert_eq!(
        transaction_count, 0,
        "bị từ chối thì không được ghi dòng tiền nào"
    );

    assert_room_status(&pool, "R-MR9", "vacant").await;
}

/// Biên dưới: 0 và âm đều là gõ nhầm, không phải một mức giá thật — cùng luật
/// với `set_booking_rate_rejects_zero_and_negative_rates`. Bị từ chối thì
/// không được để lại booking hay đổi trạng thái phòng nào.
#[tokio::test]
async fn manual_rate_at_check_in_rejects_zero_and_negative_rates() {
    let pool = test_pool().await;
    seed_room(&pool, "R-MR4").await.expect("seeds room");

    for (index, rate) in [0_i64, -1_000].into_iter().enumerate() {
        let error = stay_lifecycle::check_in(
            &pool,
            CheckInRequest {
                room_id: "R-MR4".to_string(),
                guests: vec![guest("Khách gõ nhầm", &format!("DOC-MR4-{index}"))],
                nights: 1,
                source: None,
                notes: None,
                paid_amount: None,
                pricing_type: None,
                rate_override_per_night: Some(rate),
                guest_count: None,
            },
            Some("admin-1".to_string()),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, BookingError::Validation(_)),
            "giá {rate} phải bị từ chối, nhận được: {error:?}"
        );
    }

    assert_room_status(&pool, "R-MR4", "vacant").await;
    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&pool)
        .await
        .expect("đếm booking");
    assert_eq!(
        booking_count, 0,
        "bị từ chối thì không được ghi booking nào"
    );
}

/// Biên trên: trần chống gõ nhầm thừa số 0 (`MAX_RATE_PER_NIGHT_VND`), cùng
/// luật với `set_booking_rate_rejects_a_rate_above_the_cap`.
#[tokio::test]
async fn manual_rate_at_check_in_rejects_a_rate_above_the_cap() {
    let pool = test_pool().await;
    seed_room(&pool, "R-MR5").await.expect("seeds room");

    let error = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR5".to_string(),
            guests: vec![guest("Khách gõ thừa số 0", "DOC-MR5")],
            nights: 1,
            source: None,
            notes: None,
            paid_amount: None,
            pricing_type: None,
            rate_override_per_night: Some(100_000_001),
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, BookingError::Validation(_)));
    assert_room_status(&pool, "R-MR5", "vacant").await;
}

/// Biên chính xác: guard thu-quá-tổng dùng `>` chứ không phải `>=`, nên trả
/// đúng bằng tổng tiền phải được cho qua. Cùng luật với
/// `set_booking_rate_allows_a_total_exactly_equal_to_paid_amount`.
#[tokio::test]
async fn manual_rate_at_check_in_allows_paying_exactly_the_total() {
    let pool = test_pool().await;
    seed_room(&pool, "R-MR6").await.expect("seeds room");

    let booking = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR6".to_string(),
            guests: vec![guest("Khách trả vừa khít", "DOC-MR6")],
            nights: 2,
            source: None,
            notes: None,
            // 2 đêm × 400.000 = 800.000, đúng bằng số trả — phải được cho qua.
            paid_amount: Some(800_000),
            pricing_type: None,
            rate_override_per_night: Some(400_000),
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .expect("trả đúng bằng tổng tiền phải được chấp nhận");

    assert_eq!(booking.total_price, 800_000);
    assert_eq!(booking.paid_amount, 800_000);
}

/// Không truyền override thì đường cũ giữ nguyên — engine tính, không có dấu
/// `rate_overridden_at`.
#[tokio::test]
async fn check_in_without_override_still_uses_the_engine() {
    let pool = test_pool().await;
    seed_room_with_price(&pool, "R-MR3", 500_000)
        .await
        .expect("seeds room");

    let booking = stay_lifecycle::check_in(
        &pool,
        CheckInRequest {
            room_id: "R-MR3".to_string(),
            guests: vec![guest("Khách thường", "DOC-MR3")],
            nights: 1,
            source: None,
            notes: None,
            paid_amount: None,
            pricing_type: None,
            rate_override_per_night: None,
            guest_count: None,
        },
        Some("admin-1".to_string()),
    )
    .await
    .expect("checks in without override");

    assert!(booking.rate_overridden_at.is_none());
}
