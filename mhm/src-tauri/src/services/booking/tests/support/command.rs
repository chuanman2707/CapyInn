use sqlx::{Pool, Sqlite};

use crate::{
    command_idempotency::WriteCommandContext,
    models::{CreateGuestRequest, GroupCheckinRequest},
    money::MoneyVnd,
};

pub fn group_checkin_hash_payload_for_test(req: &GroupCheckinRequest) -> serde_json::Value {
    let paid_minor_units = req
        .paid_amount
        .map(|amount| serde_json::json!((i128::from(amount) * 100).to_string()))
        .unwrap_or(serde_json::Value::Null);
    let mut room_ids = req.room_ids.clone();
    room_ids.sort();
    let guests_per_room = req
        .guests_per_room
        .iter()
        .map(|(room_id, guests)| {
            let guests = guests
                .iter()
                .map(|guest: &CreateGuestRequest| {
                    serde_json::json!({
                        "guest_type": guest.guest_type.clone(),
                        "full_name": guest.full_name.clone(),
                        "doc_number": guest.doc_number.clone(),
                        "dob": guest.dob.clone(),
                        "gender": guest.gender.clone(),
                        "nationality": guest.nationality.clone(),
                        "address": guest.address.clone(),
                        "visa_expiry": guest.visa_expiry.clone(),
                        "scan_path": guest.scan_path.clone(),
                        "phone": guest.phone.clone(),
                    })
                })
                .collect::<Vec<_>>();
            (room_id.clone(), serde_json::json!(guests))
        })
        .collect::<serde_json::Map<_, _>>();

    serde_json::json!({
        "schema": "group.checkin.v1",
        "group_name": req.group_name.clone(),
        "organizer_name": req.organizer_name.clone(),
        "organizer_phone": req.organizer_phone.clone(),
        "check_in_date": req.check_in_date.clone(),
        "room_ids": room_ids,
        "master_room_id": req.master_room_id.clone(),
        "guests_per_room": guests_per_room,
        "nights": req.nights,
        "source": req.source.clone(),
        "notes": req.notes.clone(),
        "paid_minor_units": paid_minor_units,
    })
}

pub fn add_folio_line_hash_payload_for_test(
    booking_id: &str,
    category: &str,
    description: &str,
    amount: MoneyVnd,
    created_by: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "schema": "folio.add_line.v1",
        "booking_id": booking_id,
        "category": category,
        "description": description,
        "amount_minor_units": (i128::from(amount) * 100).to_string(),
        "created_by": created_by,
    })
}

pub async fn seed_live_in_progress_command(
    pool: &Pool<Sqlite>,
    command_name: &str,
    idempotency_key: &str,
    payload: &serde_json::Value,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, issued_at,
            created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?, ?)",
    )
    .bind(idempotency_key)
    .bind(command_name)
    .bind(crate::command_idempotency::stable_request_hash(payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed in-progress command");
}

pub fn cmd(command_name: &str, idempotency_key: &str) -> WriteCommandContext {
    let request_id = format!("req-{command_name}-{idempotency_key}");
    WriteCommandContext::for_internal_test(&request_id, idempotency_key, command_name)
}

pub fn cmd_with_request(
    command_name: &str,
    request_id: &str,
    idempotency_key: &str,
) -> WriteCommandContext {
    WriteCommandContext::for_internal_test(request_id, idempotency_key, command_name)
}

pub fn cmd_at(
    command_name: &str,
    idempotency_key: &str,
    issued_at: &str,
) -> WriteCommandContext {
    let mut ctx = cmd(command_name, idempotency_key);
    ctx.issued_at = chrono::DateTime::parse_from_rfc3339(issued_at)
        .expect("test issued_at parses");
    ctx
}
