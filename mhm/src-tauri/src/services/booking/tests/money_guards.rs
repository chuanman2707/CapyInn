use super::prelude::*;

#[tokio::test]
async fn check_in_rejects_negative_paid_amount() {
    let pool = test_pool().await;
    let mut req = minimal_checkin_request("R-NEG-PAID");
    req.paid_amount = Some(-1);

    let error = stay_lifecycle::check_in(&pool, req, None)
        .await
        .expect_err("negative paid_amount must fail");

    assert!(
        error.to_string().contains("paid_amount"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn check_out_rejects_negative_final_total() {
    let pool = test_pool().await;

    let error = stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B-NEG-FINAL".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: -1,
        },
        Local::now(),
    )
    .await
    .expect_err("negative final_total must fail");

    assert!(
        error.to_string().contains("final_total"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn reservation_rejects_negative_deposit_amount() {
    let pool = test_pool().await;
    let mut req = minimal_reservation_request("R-NEG-DEPOSIT");
    req.deposit_amount = Some(-1);

    let error = reservation_lifecycle::create_reservation(&pool, req)
        .await
        .expect_err("negative deposit_amount must fail");

    assert!(
        error.to_string().contains("deposit_amount"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn group_checkin_rejects_negative_paid_amount() {
    let pool = test_pool().await;
    let mut req = minimal_group_checkin_request(&["G-NEG-PAID"]);
    req.paid_amount = Some(-1);

    let error = group_lifecycle::group_checkin(&pool, Some("seed-user".to_string()), req)
        .await
        .expect_err("negative group paid_amount must fail");

    assert!(
        error.to_string().contains("paid_amount"),
        "error should name the invalid field: {error}"
    );
}

#[tokio::test]
async fn group_checkout_rejects_negative_final_paid() {
    let pool = test_pool().await;

    let error = group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: "G-NEG-FINAL".to_string(),
            booking_ids: vec!["B-NEG-FINAL".to_string()],
            final_paid: Some(-1),
        },
    )
    .await
    .expect_err("negative final_paid must fail");

    assert!(
        error.to_string().contains("final_paid"),
        "error should name the invalid field: {error}"
    );
}
