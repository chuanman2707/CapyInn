use super::prelude::*;

#[tokio::test]
async fn create_guest_manifest_persists_primary_and_additional_guests() {
    let pool = test_pool().await;
    let mut request = minimal_checkin_request("R201");
    request.guests.push(CreateGuestRequest {
        guest_type: Some("foreign".to_string()),
        full_name: "Jane Doe".to_string(),
        doc_number: "P1234567".to_string(),
        dob: None,
        gender: Some("female".to_string()),
        nationality: Some("US".to_string()),
        address: Some("1 Test Street".to_string()),
        visa_expiry: None,
        scan_path: None,
        phone: Some("0909999999".to_string()),
    });

    let mut tx = pool.begin().await.unwrap();
    let manifest =
        guest_service::create_guest_manifest(&mut tx, &request.guests, "2026-04-15T10:00:00+07:00")
            .await
            .unwrap();

    assert_eq!(manifest.guest_ids.len(), 2);
    assert_eq!(manifest.primary_guest_id, manifest.guest_ids[0]);

    let rows = sqlx::query(
        "SELECT full_name, guest_type, doc_number, phone FROM guests ORDER BY full_name ASC",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("full_name"), "Jane Doe");
    assert_eq!(rows[0].get::<String, _>("guest_type"), "foreign");
    assert_eq!(rows[1].get::<String, _>("full_name"), "Nguyen Van A");
}

#[tokio::test]
async fn create_guest_manifest_rejects_empty_guest_list() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let error = guest_service::create_guest_manifest(&mut tx, &[], "2026-04-15T10:00:00+07:00")
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "Phải có ít nhất 1 khách");
}

#[tokio::test]
async fn create_reservation_guest_manifest_defaults_blank_doc_number() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let manifest = guest_service::create_reservation_guest_manifest(
        &mut tx,
        "Reservation Guest",
        None,
        Some("0901234567"),
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();

    let guest = sqlx::query("SELECT full_name, doc_number, phone FROM guests WHERE id = ?")
        .bind(&manifest.primary_guest_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(manifest.guest_ids, vec![manifest.primary_guest_id.clone()]);
    assert_eq!(guest.get::<String, _>("full_name"), "Reservation Guest");
    assert_eq!(guest.get::<String, _>("doc_number"), "");
    assert_eq!(
        guest.get::<Option<String>, _>("phone"),
        Some("0901234567".to_string())
    );
}
