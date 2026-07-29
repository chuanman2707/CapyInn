use super::prelude::*;

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
    assert_eq!(pricing.total, 1_320_000);
    assert_eq!(pricing.base_amount, 1_200_000);
    assert_eq!(pricing.surcharge_amount, 120_000);
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

    assert_eq!(pricing.total, 1_320_000);

    tx.rollback().await.unwrap();
}
