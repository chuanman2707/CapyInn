use std::collections::HashMap;

use crate::models::{
    CheckInRequest, CreateGuestRequest, CreateReservationRequest, GroupCheckinRequest,
};

pub fn minimal_checkin_request(room_id: &str) -> CheckInRequest {
    CheckInRequest {
        room_id: room_id.to_string(),
        guests: vec![CreateGuestRequest {
            guest_type: Some("domestic".to_string()),
            full_name: "Nguyen Van A".to_string(),
            doc_number: "079123456789".to_string(),
            dob: None,
            gender: None,
            nationality: Some("VN".to_string()),
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: Some("0900000000".to_string()),
        }],
        nights: 2,
        source: Some("walk-in".to_string()),
        notes: Some("test check-in".to_string()),
        paid_amount: None,
        pricing_type: Some("nightly".to_string()),
    }
}

pub fn minimal_reservation_request(room_id: &str) -> CreateReservationRequest {
    CreateReservationRequest {
        room_id: room_id.to_string(),
        guest_name: "Nguyen Van B".to_string(),
        guest_phone: Some("0900000001".to_string()),
        guest_doc_number: Some("079000000001".to_string()),
        check_in_date: "2026-04-20".to_string(),
        check_out_date: "2026-04-22".to_string(),
        nights: 2,
        deposit_amount: Some(50_000),
        source: Some("phone".to_string()),
        notes: Some("test reservation".to_string()),
    }
}

pub fn minimal_group_checkin_request(room_ids: &[&str]) -> GroupCheckinRequest {
    let mut guests_per_room = HashMap::new();
    if let Some(first_room) = room_ids.first() {
        guests_per_room.insert(
            (*first_room).to_string(),
            vec![CreateGuestRequest {
                guest_type: Some("domestic".to_string()),
                full_name: "Group Guest 1".to_string(),
                doc_number: "079111111111".to_string(),
                dob: None,
                gender: None,
                nationality: Some("VN".to_string()),
                address: None,
                visa_expiry: None,
                scan_path: None,
                phone: Some("0901111111".to_string()),
            }],
        );
    }

    GroupCheckinRequest {
        group_name: "Test Group".to_string(),
        organizer_name: "Organizer".to_string(),
        organizer_phone: Some("0902222222".to_string()),
        check_in_date: None,
        room_ids: room_ids
            .iter()
            .map(|room_id| (*room_id).to_string())
            .collect(),
        master_room_id: room_ids[0].to_string(),
        guests_per_room,
        nights: 2,
        source: Some("walk-in".to_string()),
        notes: Some("group test".to_string()),
        paid_amount: Some(100_000),
    }
}

pub fn rich_group_checkin_request(
    room_ids: &[&str],
    master_room_id: &str,
    paid_amount: Option<i64>,
) -> GroupCheckinRequest {
    let mut guests_per_room = HashMap::new();
    for room_id in room_ids {
        guests_per_room.insert(
            (*room_id).to_string(),
            vec![CreateGuestRequest {
                guest_type: Some("domestic".to_string()),
                full_name: format!("Group Guest {}", room_id),
                doc_number: format!("DOC-{}", room_id),
                dob: None,
                gender: None,
                nationality: Some("VN".to_string()),
                address: None,
                visa_expiry: None,
                scan_path: None,
                phone: Some("0901111111".to_string()),
            }],
        );
    }

    GroupCheckinRequest {
        group_name: "Idempotent Group".to_string(),
        organizer_name: "Organizer".to_string(),
        organizer_phone: Some("0903333333".to_string()),
        check_in_date: None,
        room_ids: room_ids
            .iter()
            .map(|room_id| (*room_id).to_string())
            .collect(),
        master_room_id: master_room_id.to_string(),
        guests_per_room,
        nights: 2,
        source: Some("walk-in".to_string()),
        notes: Some("group checkin idempotent".to_string()),
        paid_amount,
    }
}
