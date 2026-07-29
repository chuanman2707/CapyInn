use sqlx::{Sqlite, Transaction};

use crate::{
    domain::booking::{BookingError, BookingResult},
    models::CreateGuestRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestManifest {
    pub primary_guest_id: String,
    pub guest_ids: Vec<String>,
}

/// Ánh xạ khách sang JSON dùng cho request hash của các lệnh ghi.
///
/// Sống cạnh `create_guest_manifest` để mọi hiểu biết về các trường của
/// `CreateGuestRequest` nằm chung một chỗ: thêm một trường thì chỉ sửa ở đây.
pub(super) fn guest_hash_payload_entries(guests: &[CreateGuestRequest]) -> Vec<serde_json::Value> {
    guests
        .iter()
        .map(|guest| {
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
        .collect()
}

struct GuestRecordInput<'a> {
    guest_type: &'a str,
    full_name: &'a str,
    doc_number: &'a str,
    dob: Option<&'a str>,
    gender: Option<&'a str>,
    nationality: Option<&'a str>,
    address: Option<&'a str>,
    visa_expiry: Option<&'a str>,
    scan_path: Option<&'a str>,
    phone: Option<&'a str>,
    created_at: &'a str,
}

pub async fn create_guest_manifest(
    tx: &mut Transaction<'_, Sqlite>,
    guests: &[CreateGuestRequest],
    created_at: &str,
) -> BookingResult<GuestManifest> {
    if guests.is_empty() {
        return Err(BookingError::validation(
            "Phải có ít nhất 1 khách".to_string(),
        ));
    }

    let mut guest_ids = Vec::with_capacity(guests.len());
    for guest in guests {
        guest_ids.push(
            insert_guest_record(
                tx,
                GuestRecordInput {
                    guest_type: guest.guest_type.as_deref().unwrap_or("domestic"),
                    full_name: &guest.full_name,
                    doc_number: &guest.doc_number,
                    dob: guest.dob.as_deref(),
                    gender: guest.gender.as_deref(),
                    nationality: guest.nationality.as_deref(),
                    address: guest.address.as_deref(),
                    visa_expiry: guest.visa_expiry.as_deref(),
                    scan_path: guest.scan_path.as_deref(),
                    phone: guest.phone.as_deref(),
                    created_at,
                },
            )
            .await?,
        );
    }

    Ok(GuestManifest {
        primary_guest_id: guest_ids[0].clone(),
        guest_ids,
    })
}

pub async fn create_reservation_guest_manifest(
    tx: &mut Transaction<'_, Sqlite>,
    guest_name: &str,
    guest_doc_number: Option<&str>,
    guest_phone: Option<&str>,
    created_at: &str,
) -> BookingResult<GuestManifest> {
    let guest_id = insert_guest_record(
        tx,
        GuestRecordInput {
            guest_type: "domestic",
            full_name: guest_name,
            doc_number: guest_doc_number.unwrap_or(""),
            dob: None,
            gender: None,
            nationality: None,
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: guest_phone,
            created_at,
        },
    )
    .await?;

    Ok(GuestManifest {
        primary_guest_id: guest_id.clone(),
        guest_ids: vec![guest_id],
    })
}

pub async fn create_group_guest_manifest(
    tx: &mut Transaction<'_, Sqlite>,
    guests: &[CreateGuestRequest],
    placeholder_name: &str,
    created_at: &str,
) -> BookingResult<GuestManifest> {
    if guests.is_empty() {
        let guest_id = insert_guest_record(
            tx,
            GuestRecordInput {
                guest_type: "domestic",
                full_name: placeholder_name,
                doc_number: "",
                dob: None,
                gender: None,
                nationality: None,
                address: None,
                visa_expiry: None,
                scan_path: None,
                phone: None,
                created_at,
            },
        )
        .await?;

        return Ok(GuestManifest {
            primary_guest_id: guest_id.clone(),
            guest_ids: vec![guest_id],
        });
    }

    create_guest_manifest(tx, guests, created_at).await
}

pub async fn link_booking_guests(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    guest_ids: &[String],
) -> BookingResult<()> {
    for guest_id in guest_ids {
        sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?)")
            .bind(booking_id)
            .bind(guest_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

async fn insert_guest_record(
    tx: &mut Transaction<'_, Sqlite>,
    input: GuestRecordInput<'_>,
) -> BookingResult<String> {
    let guest_id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO guests (
            id, guest_type, full_name, doc_number, dob, gender, nationality,
            address, visa_expiry, scan_path, phone, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&guest_id)
    .bind(input.guest_type)
    .bind(input.full_name)
    .bind(input.doc_number)
    .bind(input.dob)
    .bind(input.gender)
    .bind(input.nationality)
    .bind(input.address)
    .bind(input.visa_expiry)
    .bind(input.scan_path)
    .bind(input.phone)
    .bind(input.created_at)
    .execute(&mut **tx)
    .await?;

    Ok(guest_id)
}
