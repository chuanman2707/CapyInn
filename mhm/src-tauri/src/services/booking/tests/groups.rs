use super::prelude::*;
use crate::models::GroupCheckinRequest;

/// Đoàn: mỗi phòng một giá riêng, phòng không có trong map thì engine tính.
///
/// BA phòng, không phải hai — hai phòng không đủ để lộ hai lớp bug review chỉ
/// ra:
/// - G-R1/G-R2 cùng hạng `standard` cùng giá engine 500.000đ/đêm: nếu chỉ có
///   hai phòng này, `engine_total` LƯU LẠI trong snapshot của phòng override
///   (G-R1) và tổng THẬT của phòng không override (G-R2) là CÙNG MỘT SỐ
///   (1.000.000) — một cài đặt lỡ tính giá engine từ NHẦM phòng vẫn ra đúng
///   số, xanh giả.
/// - G-R1 vừa là phòng override DUY NHẤT vừa là MASTER: một cài đặt lỡ áp giá
///   tay theo `is_master` thay vì tra đúng theo map `rate_override_per_room`
///   vẫn ra đúng số, xanh giả.
///
/// G-R3 phá cả hai: khác hạng (`deluxe`, engine 350.000đ/đêm — KHÁC 500.000
/// của G-R1/G-R2, nên `engine_total` của nó không thể trùng số với bất cứ
/// tổng nào khác trong test), override (300.000đ/đêm — khác giá override của
/// G-R1 lẫn hai giá engine), và KHÔNG phải master (G-R1 vẫn là master).
#[tokio::test]
async fn group_checkin_applies_per_room_manual_rates() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-R1", "G-R2"], 500_000)
        .await
        .expect("seeds rooms");
    // `seed_room` luôn tạo phòng hạng `standard` — phải đổi `type` rồi seed
    // riêng một pricing_rule `deluxe` thì G-R3 mới có giá engine THẬT SỰ khác
    // G-R1/G-R2 (sửa thẳng `base_price` không đủ: engine định giá theo
    // `room_type`, không theo `rooms.base_price` — xem
    // `room_change.rs::seed_stay_in_progress`).
    seed_room(&pool, "G-R3").await.expect("seeds room");
    sqlx::query("UPDATE rooms SET type = 'deluxe' WHERE id = 'G-R3'")
        .execute(&pool)
        .await
        .expect("sets G-R3 to a distinct room type");
    seed_pricing_rule(&pool, "deluxe", 350_000)
        .await
        .expect("seeds deluxe pricing rule");

    let mut rate_override_per_room = std::collections::HashMap::new();
    rate_override_per_room.insert("G-R1".to_string(), 400_000);
    rate_override_per_room.insert("G-R3".to_string(), 300_000);

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn thử".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-R1".to_string(), "G-R2".to_string(), "G-R3".to_string()],
            master_room_id: "G-R1".to_string(),
            guests_per_room: Default::default(),
            nights: 2,
            source: None,
            notes: None,
            paid_amount: None,
            rate_override_per_room,
        },
    )
    .await
    .expect("checks in the group");

    let overridden: i64 = sqlx::query_scalar(
        "SELECT total_price FROM bookings WHERE group_id = ? AND room_id = 'G-R1'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads overridden room total");
    assert_eq!(overridden, 800_000, "2 đêm × 400.000");

    let engine_priced: i64 = sqlx::query_scalar(
        "SELECT total_price FROM bookings WHERE group_id = ? AND room_id = 'G-R2'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads engine-priced room total");
    assert_eq!(engine_priced, 1_000_000, "2 đêm × 500.000 do engine tính");

    let overridden_not_master: i64 = sqlx::query_scalar(
        "SELECT total_price FROM bookings WHERE group_id = ? AND room_id = 'G-R3'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads overridden-but-not-master room total");
    assert_eq!(
        overridden_not_master, 600_000,
        "2 đêm × 300.000 — phòng override phải lấy đúng giá của CHÍNH nó dù \
         không phải master"
    );

    let overridden_marked: Option<String> = sqlx::query_scalar(
        "SELECT rate_overridden_at FROM bookings WHERE group_id = ? AND room_id = 'G-R1'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads override marker for overridden room");
    assert!(
        overridden_marked.is_some(),
        "phòng đã sửa giá phải được đánh dấu rate_overridden_at"
    );

    let engine_marked: Option<String> = sqlx::query_scalar(
        "SELECT rate_overridden_at FROM bookings WHERE group_id = ? AND room_id = 'G-R2'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads override marker for engine-priced room");
    assert!(
        engine_marked.is_none(),
        "phòng không sửa giá thì không được đánh dấu"
    );

    let not_master_marked: (Option<String>, i64) = sqlx::query_as(
        "SELECT rate_overridden_at, is_master_room FROM bookings WHERE group_id = ? AND room_id = 'G-R3'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads override marker for non-master overridden room");
    assert!(
        not_master_marked.0.is_some(),
        "G-R3 override giá thì phải được đánh dấu dù không phải master"
    );
    assert_eq!(
        not_master_marked.1, 0,
        "G-R3 không phải master — override không được ăn theo is_master"
    );

    // Câu hỏi "tổng đoàn liên hệ thế nào với tổng từng phòng": booking_groups
    // không có cột tổng riêng — GroupDetailResponse.total_room_cost
    // (queries/groups/group_queries.rs) cộng SUM(bookings.total_price), nên
    // đây là bằng chứng trực tiếp SUM đã phản ánh đúng override, không tính
    // lại từ engine.
    let group_total: i64 =
        sqlx::query_scalar("SELECT SUM(total_price) FROM bookings WHERE group_id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .expect("reads group total");
    assert_eq!(group_total, 800_000 + 1_000_000 + 600_000);

    let snapshot: Option<String> = sqlx::query_scalar(
        "SELECT pricing_snapshot FROM bookings WHERE group_id = ? AND room_id = 'G-R1'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads pricing_snapshot for overridden room");
    let snapshot: serde_json::Value =
        serde_json::from_str(&snapshot.expect("overridden room has a snapshot"))
            .expect("snapshot is valid JSON");
    assert_eq!(snapshot["manual_rate"]["rate_per_night"], 400_000);
    assert_eq!(snapshot["manual_rate"]["engine_total"], 1_000_000);

    // G-R3 hạng `deluxe`: 2 đêm × 350.000 = 700.000 — một số KHÔNG trùng với
    // bất cứ tổng nào khác trong test này (800.000 / 1.000.000 / 600.000 /
    // 1.000.000 của G-R1's engine_total). Nếu cài đặt lỡ tính engine_total
    // của G-R3 từ giá của phòng khác (G-R1 hay G-R2, đều hạng `standard`
    // 500.000/đêm), số đọc ra sẽ là 1.000.000 — sai và bị assert dưới bắt.
    let deluxe_snapshot: Option<String> = sqlx::query_scalar(
        "SELECT pricing_snapshot FROM bookings WHERE group_id = ? AND room_id = 'G-R3'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads pricing_snapshot for non-master overridden room");
    let deluxe_snapshot: serde_json::Value =
        serde_json::from_str(&deluxe_snapshot.expect("G-R3 has a snapshot"))
            .expect("snapshot is valid JSON");
    assert_eq!(deluxe_snapshot["manual_rate"]["rate_per_night"], 300_000);
    assert_eq!(
        deluxe_snapshot["manual_rate"]["engine_total"], 700_000,
        "engine_total phải là giá engine CỦA CHÍNH G-R3 (hạng deluxe, 2×350.000), \
         không phải giá của phòng standard nào khác trong đoàn"
    );

    let unoverridden_snapshot: Option<String> = sqlx::query_scalar(
        "SELECT pricing_snapshot FROM bookings WHERE group_id = ? AND room_id = 'G-R2'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .expect("reads pricing_snapshot for engine-priced room");
    assert!(
        unoverridden_snapshot.is_none(),
        "phòng không override thì pricing_snapshot phải giữ NULL như trước"
    );
}

/// Không test nào phủ HAI phòng cùng override nhưng HAI GIÁ KHÁC NHAU qua
/// đúng transaction thật (`group_checkin`, không gọi thẳng hàm nội bộ) —
/// đúng hình dạng lỗi mà một giá LẠC SANG PHÒNG KHÁC mới lộ ra: nếu code lỡ
/// hoán giá giữa hai phòng (zip sai thứ tự, tra map bằng khoá sai...), một
/// override DUY NHẤT hoặc hai override CÙNG giá đều không thể bắt được, vì
/// hoán xong vẫn ra đúng số. `group_checkin_applies_per_room_manual_rates` ở
/// trên đã có 3 phòng nên cũng phủ hình dạng này, nhưng bài test này cô lập
/// đúng MỘT tình huống — tên test tự nói rõ lỗi nó chặn, không lẫn với các so
/// sánh is_master/engine_total khác.
#[tokio::test]
async fn group_checkin_applies_distinct_rates_to_two_overridden_rooms() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-DR1", "G-DR2"], 500_000)
        .await
        .unwrap();

    let mut rate_override_per_room = std::collections::HashMap::new();
    rate_override_per_room.insert("G-DR1".to_string(), 300_000);
    rate_override_per_room.insert("G-DR2".to_string(), 450_000);

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn hai giá tay".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-DR1".to_string(), "G-DR2".to_string()],
            master_room_id: "G-DR1".to_string(),
            guests_per_room: Default::default(),
            nights: 2,
            source: None,
            notes: None,
            paid_amount: None,
            rate_override_per_room,
        },
    )
    .await
    .expect("checks in the group");

    let totals = sqlx::query(
        "SELECT room_id, total_price FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(totals.len(), 2);
    assert_eq!(totals[0].get::<String, _>("room_id"), "G-DR1");
    assert_eq!(
        totals[0].get::<i64, _>("total_price"),
        600_000,
        "G-DR1 phải lấy đúng giá của CHÍNH nó (2×300.000), không phải của G-DR2"
    );
    assert_eq!(totals[1].get::<String, _>("room_id"), "G-DR2");
    assert_eq!(
        totals[1].get::<i64, _>("total_price"),
        900_000,
        "G-DR2 phải lấy đúng giá của CHÍNH nó (2×450.000), không phải của G-DR1"
    );
}

/// Ghim đúng ranh giới review Task 15 chỉ ra, nguyên số liệu từ ví dụ trong
/// báo cáo review: G-R1 override 400.000×2 đêm = 800.000, G-R2 giá engine
/// 500.000×2 đêm = 1.000.000, tổng đoàn 1.800.000. Khách trả ĐỦ cả đoàn
/// (`paid_amount: Some(1_800_000)`) qua đúng transaction thật
/// (`group_checkin`).
///
/// Trước khi vá: `allocate_positive_money_evenly_by_room` chia đều theo SỐ
/// LƯỢNG phòng — 900.000/phòng — G-R1 (tổng chỉ 800.000) nhận 900.000, vượt
/// tổng của chính nó, guard thu-vượt từ chối CẢ LƯỢT nhận đoàn dù khách trả
/// đúng khớp tổng. Sau khi vá: `allocate_paid_amount_by_room_price` chia theo
/// TỈ LỆ, mỗi phòng nhận đúng bằng tổng của chính mình — check-in phải QUA.
#[tokio::test]
async fn group_checkin_allocates_full_prepayment_proportionally_across_unequal_room_totals() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-PR1", "G-PR2"], 500_000)
        .await
        .unwrap();

    let mut rate_override_per_room = std::collections::HashMap::new();
    rate_override_per_room.insert("G-PR1".to_string(), 400_000);

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn thu đủ".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-PR1".to_string(), "G-PR2".to_string()],
            master_room_id: "G-PR1".to_string(),
            guests_per_room: Default::default(),
            nights: 2,
            source: None,
            notes: None,
            // = 800.000 (G-PR1) + 1.000.000 (G-PR2) — thu ĐỦ cả đoàn.
            paid_amount: Some(1_800_000),
            rate_override_per_room,
        },
    )
    .await
    .expect("thu đủ đúng bằng tổng cả đoàn phải QUA, không bị guard thu-vượt từ chối");

    let rows = sqlx::query(
        "SELECT room_id, total_price, paid_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("room_id"), "G-PR1");
    assert_eq!(rows[0].get::<i64, _>("total_price"), 800_000);
    assert_eq!(
        rows[0].get::<i64, _>("paid_amount"),
        800_000,
        "phòng override phải được phân bổ đúng bằng tổng của chính nó, không \
         phải chia đều theo số phòng (900.000 sẽ vượt tổng 800.000 của phòng này)"
    );
    assert_eq!(rows[1].get::<String, _>("room_id"), "G-PR2");
    assert_eq!(rows[1].get::<i64, _>("total_price"), 1_000_000);
    assert_eq!(rows[1].get::<i64, _>("paid_amount"), 1_000_000);
}

/// Trả MỘT PHẦN, không phải toàn bộ — buộc phần chia tỉ lệ KHÔNG chia hết
/// (1.000.000 × 800.000/1.800.000 = 444.444,44...; xem quy tắc làm tròn
/// trong doc-comment `allocate_paid_amount_by_room_price`). Ghim hai điều qua
/// đúng transaction thật: (1) tổng các phần phân bổ cộng lại đúng bằng
/// `paid_amount`, không lệch một đồng dù làm tròn; (2) không phòng nào vượt
/// tổng của chính nó dù chưa thu đủ cả đoàn.
#[tokio::test]
async fn group_checkin_proportional_allocation_sums_exactly_when_not_evenly_divisible() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-PP1", "G-PP2"], 500_000)
        .await
        .unwrap();

    let mut rate_override_per_room = std::collections::HashMap::new();
    rate_override_per_room.insert("G-PP1".to_string(), 400_000);

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn trả một phần".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-PP1".to_string(), "G-PP2".to_string()],
            master_room_id: "G-PP1".to_string(),
            guests_per_room: Default::default(),
            nights: 2,
            source: None,
            notes: None,
            // Tổng đoàn 1.800.000 (800.000 + 1.000.000); trả 1.000.000 — tỉ lệ
            // đúng ra 444.444,44 / 555.555,56, không chia hết.
            paid_amount: Some(1_000_000),
            rate_override_per_room,
        },
    )
    .await
    .expect("trả một phần trong khả năng chi trả của cả đoàn phải QUA");

    let rows = sqlx::query("SELECT room_id, total_price, paid_amount FROM bookings WHERE group_id = ? ORDER BY room_id")
        .bind(&group.id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("room_id"), "G-PP1");
    assert_eq!(rows[1].get::<String, _>("room_id"), "G-PP2");

    let paid_pp1 = rows[0].get::<i64, _>("paid_amount");
    let paid_pp2 = rows[1].get::<i64, _>("paid_amount");
    assert_eq!(
        paid_pp1 + paid_pp2,
        1_000_000,
        "tổng phân bổ phải khớp đúng paid_amount, không lệch một đồng dù làm tròn"
    );
    assert_eq!(
        paid_pp1, 444_445,
        "444.444 làm tròn xuống + 1 đồng dư rải theo room_id đã sắp"
    );
    assert_eq!(paid_pp2, 555_555);
    assert!(paid_pp1 <= rows[0].get::<i64, _>("total_price"));
    assert!(paid_pp2 <= rows[1].get::<i64, _>("total_price"));
}

/// Thu vượt tổng tiền CỦA MỘT PHÒNG phải bị chặn khi phòng đó theo giá tay —
/// cùng lỗ hổng `check_in_tx`/`create_reservation_tx` đã vá: một booking có
/// `paid_amount > total_price` không có lối thoát nếu sau này bị `check_out_tx`
/// từ chối vì `already_paid > final_total`.
///
/// `paid_amount` ở đây phải vượt TỔNG CẢ ĐOÀN (1.020.000), không chỉ vượt
/// tổng một phòng — bản test trước Task 15's review dùng `paid_amount:
/// Some(100_000)` (nhỏ hơn hẳn tổng cả đoàn), tưởng là ghim guard thu-vượt
/// nhưng thực ra chỉ ghim ĐÚNG lỗ hổng review chỉ ra: chia đều theo số lượng
/// cấp 50.000 cho G-OP1 dù phòng đó chỉ có tổng 20.000, trong khi 100.000 hoàn
/// toàn nằm trong khả năng chi trả của cả đoàn. `allocate_paid_amount_by_room_price`
/// chia theo TỈ LỆ giá từng phòng nên khi `paid_amount <= Σ total_price`
/// không phòng nào có thể nhận quá tổng của chính nó (chứng minh trong
/// doc-comment hàm đó) — guard dưới đây giờ chỉ còn là lưới đỡ cho một
/// `paid_amount` thu vượt CẢ ĐOÀN.
#[tokio::test]
async fn group_checkin_rejects_overpayment_on_overridden_room() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-OP1", "G-OP2"], 500_000)
        .await
        .unwrap();

    let mut rate_override_per_room = std::collections::HashMap::new();
    // 2 đêm × 10.000 = 20.000; G-OP2 giá engine 2 đêm × 500.000 = 1.000.000.
    // Tổng cả đoàn 1.020.000.
    rate_override_per_room.insert("G-OP1".to_string(), 10_000);

    let error = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn quá tay".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-OP1".to_string(), "G-OP2".to_string()],
            master_room_id: "G-OP1".to_string(),
            guests_per_room: Default::default(),
            nights: 2,
            source: None,
            notes: None,
            // > 1.020.000 tổng cả đoàn — thu vượt CẢ ĐOÀN thật, không phải
            // chỉ chia lệch một phòng.
            paid_amount: Some(1_100_000),
            rate_override_per_room,
        },
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("G-OP1"),
        "lỗi phải nêu rõ phòng thu vượt, nhận được: {error}"
    );

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_count, 0, "cả group phải rollback, không tạo dở dang");
}

/// Guard trên áp dụng cả cho phòng theo giá ENGINE, không chỉ phòng override —
/// "the overpayment guard where a real total is in scope for both the
/// override and engine paths" (task brief).
#[tokio::test]
async fn group_checkin_rejects_overpayment_on_engine_priced_room() {
    let pool = test_pool().await;
    // Giá phòng rẻ, không phòng nào override — nhưng paid_amount chia đều vượt
    // hẳn giá 1 đêm.
    seed_rooms_with_price(&pool, &["G-OP3", "G-OP4"], 10_000)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn quá tay engine".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-OP3".to_string(), "G-OP4".to_string()],
            master_room_id: "G-OP3".to_string(),
            guests_per_room: Default::default(),
            nights: 1,
            source: None,
            notes: None,
            paid_amount: Some(100_000), // chia đều 50.000/phòng > 10.000 giá engine
            rate_override_per_room: Default::default(),
        },
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("G-OP3"),
        "lỗi phải nêu rõ phòng thu vượt, nhận được: {error}"
    );

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_count, 0, "cả group phải rollback, không tạo dở dang");
}

/// `checked_mul_money`, không phải `*` trần. `validate_group_checkin_request`
/// chỉ chặn `rate` ngoài biên và `nights <= 0` — KHÔNG có trần trên cho
/// `nights` (giống `validate_check_in_request`, khác đặt trước có
/// `MAX_RESERVATION_NIGHTS`) — nên một giá tay hợp lệ (đúng bằng
/// `MAX_RATE_PER_NIGHT_VND`) nhân với một `nights` hợp lệ nhưng rất lớn vẫn có
/// thể vượt trần an toàn truyền tải (`MAX_TRANSPORT_SAFE_MONEY_VND`, không
/// tràn i64 thật — `rate ≤ 1e8` và `nights: i32 ≤ ~2,1 tỷ` không đủ để tràn
/// i64).
///
/// Đã KIỂM CHỨNG BẰNG MUTATION: đổi `checked_mul_money` thành `*` trần khiến
/// test này đỏ — nhưng KHÔNG phải vì phép nhân tràn số (không tràn, như trên).
/// Nó đỏ vì thông báo đổi từ `"total_price must be a safe integer VND value"`
/// (từ chính `checked_mul_money`, chặn NGAY sau khi tính `total`, trước khi
/// tạo booking) sang `"transaction amount must be a safe integer VND value"`
/// (từ `record_charge_tx` ở downstream, SAU KHI đã INSERT một dòng booking
/// với `total_price` ngoài biên an toàn — vẫn bị rollback nên không có dữ
/// liệu xấu tồn tại thật, nhưng muộn hơn và ghi dở nhiều hơn hẳn). Tức là có
/// PHÒNG THỦ HAI LỚP thật ở đây (`record_charge_tx` tự kiểm biên tiền của
/// chính nó, độc lập với `checked_mul_money`) — assert theo `"total_price"`
/// bên dưới ghim đúng LỚP `group_checkin_tx` sở hữu (chặn sớm, trước khi ghi
/// gì), không phải chỉ ghim "có lỗi nào đó được trả về".
#[tokio::test]
async fn group_checkin_rejects_overridden_total_beyond_transport_safe_ceiling() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-OF1"], 500_000)
        .await
        .unwrap();

    let mut rate_override_per_room = std::collections::HashMap::new();
    // Đúng bằng trần, hợp lệ theo validate_group_checkin_request.
    rate_override_per_room.insert("G-OF1".to_string(), 100_000_000);

    let error = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        GroupCheckinRequest {
            group_name: "Đoàn tràn biên truyền tải".to_string(),
            organizer_name: "Trưởng đoàn".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: vec!["G-OF1".to_string()],
            master_room_id: "G-OF1".to_string(),
            guests_per_room: Default::default(),
            // 100_000_000 × 90_100_000 = 9,01×10^15, vượt MAX_TRANSPORT_SAFE_MONEY_VND
            // (~9,0072×10^15) nhưng không tràn i64 (~9,223×10^18) — và vẫn đủ
            // nhỏ để `checkin_naive + Duration::days(nights)` (chạy TRƯỚC khối
            // giá, ở đầu `group_checkin_tx`) không tự tràn NaiveDate.
            nights: 90_100_000,
            source: None,
            notes: None,
            paid_amount: None,
            rate_override_per_room,
        },
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("total_price"),
        "phải là lỗi từ chính checked_mul_money (field 'total_price'), chặn TRƯỚC khi ghi \
         booking nào — không phải một lớp downstream chặn muộn hơn, nhận được: {error}"
    );

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_count, 0, "cả group phải rollback, không tạo dở dang");
}

#[tokio::test]
async fn group_checkin_creates_active_group_and_placeholder_guest_manifest() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G101", "G102"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G101", "G102"]),
    )
    .await
    .unwrap();

    assert_eq!(group.status, "active");
    assert!(group.master_booking_id.is_some());

    let room_statuses =
        sqlx::query("SELECT id, status FROM rooms WHERE id IN ('G101', 'G102') ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(room_statuses[0].get::<String, _>("status"), "occupied");
    assert_eq!(room_statuses[1].get::<String, _>("status"), "occupied");

    let booking_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = 'active'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(booking_count.0, 2);

    let paid_amounts = sqlx::query(
        "SELECT paid_amount, deposit_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(paid_amounts.len(), 2);
    assert_eq!(
        paid_amounts[0].get::<Option<i64>, _>("paid_amount"),
        Some(50_000)
    );
    assert_eq!(
        paid_amounts[1].get::<Option<i64>, _>("paid_amount"),
        Some(50_000)
    );
    assert_eq!(
        paid_amounts[0].get::<Option<i64>, _>("deposit_amount"),
        Some(0)
    );

    let placeholder = sqlx::query(
        "SELECT g.full_name, g.doc_number
         FROM guests g
         JOIN bookings b ON b.primary_guest_id = g.id
         WHERE b.group_id = ? AND b.room_id = 'G102'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        placeholder.get::<String, _>("full_name"),
        "Khách đoàn Test Group - G102"
    );
    assert_eq!(placeholder.get::<String, _>("doc_number"), "");
}

#[tokio::test]
async fn group_checkin_reservation_blocks_calendar_and_tracks_deposit() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G201", "G202"], 300_000)
        .await
        .unwrap();

    let mut req = minimal_group_checkin_request(&["G201", "G202"]);
    req.check_in_date = Some(
        (Local::now().date_naive() + Duration::days(1))
            .format("%Y-%m-%d")
            .to_string(),
    );
    req.paid_amount = Some(60_000);

    let group = group_lifecycle::group_checkin(&pool, None, req)
        .await
        .unwrap();

    assert_eq!(group.status, "booked");

    let room_statuses =
        sqlx::query("SELECT status FROM rooms WHERE id IN ('G201', 'G202') ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(room_statuses[0].get::<String, _>("status"), "vacant");
    assert_eq!(room_statuses[1].get::<String, _>("status"), "vacant");

    let calendar_rows: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id IN (SELECT id FROM bookings WHERE group_id = ?) AND status = 'booked'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_rows.0, 4);

    let amounts = sqlx::query(
        "SELECT paid_amount, deposit_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        amounts[0].get::<Option<i64>, _>("paid_amount"),
        Some(30_000)
    );
    assert_eq!(
        amounts[0].get::<Option<i64>, _>("deposit_amount"),
        Some(30_000)
    );
}

#[tokio::test]
async fn group_checkin_rejects_duplicate_room_ids() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G250"], 250_000)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["G250", "G250"]),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Phòng không được lặp trong cùng một group"
    );
}

#[tokio::test]
async fn group_checkin_lock_keys_are_stable_for_room_order() {
    let left = crate::aggregate_locks::canonicalize_lock_keys(vec![
        crate::aggregate_locks::room_key("R2").unwrap(),
        crate::aggregate_locks::room_key("R1").unwrap(),
    ])
    .unwrap();
    let right = crate::aggregate_locks::canonicalize_lock_keys(vec![
        crate::aggregate_locks::room_key("R1").unwrap(),
        crate::aggregate_locks::room_key("R2").unwrap(),
    ])
    .unwrap();

    assert_eq!(left, right);
}

#[tokio::test]
async fn group_checkout_reassigns_master_and_updates_group_payment() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G301", "G302"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G301", "G302"]),
    )
    .await
    .unwrap();

    let master_booking_id = group.master_booking_id.clone().unwrap();
    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![master_booking_id.clone()],
            final_paid: Some(40_000),
        },
    )
    .await
    .unwrap();

    let group_row =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(group_row.get::<String, _>("status"), "partial_checkout");
    assert_ne!(
        group_row.get::<Option<String>, _>("master_booking_id"),
        Some(master_booking_id.clone())
    );

    let checked_out = sqlx::query("SELECT status FROM bookings WHERE id = ?")
        .bind(&master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(checked_out.get::<String, _>("status"), "checked_out");

    let housekeeping_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM housekeeping WHERE room_id = 'G301'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(housekeeping_count.0, 1);

    let remaining_paid: (i64,) = sqlx::query_as(
        "SELECT paid_amount FROM bookings WHERE group_id = ? AND status = 'active' LIMIT 1",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining_paid.0, 90_000);
}

#[tokio::test]
async fn group_checkout_clears_master_flag_when_group_completes() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G401"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G401"]),
    )
    .await
    .unwrap();

    let master_booking_id = group.master_booking_id.clone().unwrap();
    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![master_booking_id.clone()],
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let group_row =
        sqlx::query("SELECT master_booking_id, status FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        group_row.get::<Option<String>, _>("master_booking_id"),
        None
    );
    assert_eq!(group_row.get::<String, _>("status"), "completed");

    let booking_row = sqlx::query("SELECT is_master_room FROM bookings WHERE id = ?")
        .bind(&master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking_row.get::<i64, _>("is_master_room"), 0);
}

#[tokio::test]
async fn group_booking_lifecycle_smoke_covers_partial_and_final_checkout() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G-SMOKE-1", "G-SMOKE-2"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G-SMOKE-1", "G-SMOKE-2"]),
    )
    .await
    .unwrap();

    assert_eq!(group.status, "active");

    let initial_group =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(initial_group.get::<String, _>("status"), "active");
    let first_master_booking_id = initial_group
        .get::<Option<String>, _>("master_booking_id")
        .expect("group check-in should assign a master booking");

    let initial_bookings = sqlx::query(
        "SELECT id, room_id, status, is_master_room
         FROM bookings
         WHERE group_id = ?
         ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(initial_bookings.len(), 2);
    assert!(initial_bookings.iter().any(|row| {
        row.get::<String, _>("id") == first_master_booking_id
            && row.get::<String, _>("status") == "active"
            && row.get::<i64, _>("is_master_room") == 1
    }));
    assert!(initial_bookings
        .iter()
        .all(|row| row.get::<String, _>("status") == "active"));

    let occupied_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'occupied'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(occupied_count.0, 2);

    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![first_master_booking_id.clone()],
            final_paid: Some(40_000),
        },
    )
    .await
    .unwrap();

    let partial_group =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(partial_group.get::<String, _>("status"), "partial_checkout");
    let second_master_booking_id = partial_group
        .get::<Option<String>, _>("master_booking_id")
        .expect("partial checkout should reassign a master booking");
    assert_ne!(second_master_booking_id, first_master_booking_id);

    let checked_out_status: String = sqlx::query_scalar("SELECT status FROM bookings WHERE id = ?")
        .bind(&first_master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(checked_out_status, "checked_out");

    let remaining_master = sqlx::query(
        "SELECT id, is_master_room
         FROM bookings
         WHERE group_id = ? AND status = 'active'
         LIMIT 1",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        remaining_master.get::<String, _>("id"),
        second_master_booking_id
    );
    assert_eq!(remaining_master.get::<i64, _>("is_master_room"), 1);

    let first_room_cleaning_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id = 'G-SMOKE-1' AND status = 'cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first_room_cleaning_count.0, 1);

    let first_housekeeping_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM housekeeping
         WHERE room_id = 'G-SMOKE-1' AND status = 'needs_cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first_housekeeping_count.0, 1);

    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![second_master_booking_id],
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let final_group =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(final_group.get::<String, _>("status"), "completed");
    assert_eq!(
        final_group.get::<Option<String>, _>("master_booking_id"),
        None
    );

    let active_booking_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = 'active'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active_booking_count.0, 0);

    let checked_out_booking_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM bookings WHERE group_id = ? AND status = 'checked_out'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(checked_out_booking_count.0, 2);

    let remaining_occupied_rooms: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'occupied'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining_occupied_rooms.0, 0);

    let cleaning_room_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM rooms
         WHERE id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cleaning_room_count.0, 2);

    let housekeeping_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM housekeeping
         WHERE room_id IN ('G-SMOKE-1', 'G-SMOKE-2')
           AND status = 'needs_cleaning'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(housekeeping_count.0, 2);
}

#[tokio::test]
async fn group_checkout_rejects_stale_selected_booking() {
    let pool = test_pool().await;
    seed_rooms_with_price(&pool, &["G501", "G502"], 250_000)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G501", "G502"]),
    )
    .await
    .unwrap();

    let booking_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? ORDER BY room_id")
            .bind(&group.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind(&booking_ids[0])
        .execute(&pool)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids,
            final_paid: None,
        },
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[test]
fn group_checkout_locked_room_map_rejects_changed_room_mapping() {
    let locked = std::collections::HashMap::from([
        ("booking-1".to_string(), "room-old".to_string()),
        ("booking-2".to_string(), "room-stable".to_string()),
    ]);
    let current = std::collections::HashMap::from([
        ("booking-1".to_string(), "room-new".to_string()),
        ("booking-2".to_string(), "room-stable".to_string()),
    ]);

    let error = group_lifecycle::ensure_group_checkout_room_map_still_locked(
        "group-1",
        &["booking-1".to_string(), "booking-2".to_string()],
        &locked,
        &current,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
    assert!(error
        .to_string()
        .contains("one or more bookings in group group-1 changed rooms before checkout"));
}
