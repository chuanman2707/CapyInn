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
                guest.guest_type.as_deref().unwrap_or("domestic"),
                &guest.full_name,
                &guest.doc_number,
                guest.dob.as_deref(),
                guest.gender.as_deref(),
                guest.nationality.as_deref(),
                guest.address.as_deref(),
                guest.visa_expiry.as_deref(),
                guest.scan_path.as_deref(),
                guest.phone.as_deref(),
                created_at,
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
        "domestic",
        guest_name,
        guest_doc_number.unwrap_or(""),
        None,
        None,
        None,
        None,
        None,
        None,
        guest_phone,
        created_at,
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
            "domestic",
            placeholder_name,
            "",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at,
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
    guest_type: &str,
    full_name: &str,
    doc_number: &str,
    dob: Option<&str>,
    gender: Option<&str>,
    nationality: Option<&str>,
    address: Option<&str>,
    visa_expiry: Option<&str>,
    scan_path: Option<&str>,
    phone: Option<&str>,
    created_at: &str,
) -> BookingResult<String> {
    let guest_id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO guests (
            id, guest_type, full_name, doc_number, dob, gender, nationality,
            address, visa_expiry, scan_path, phone, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&guest_id)
    .bind(guest_type)
    .bind(full_name)
    .bind(doc_number)
    .bind(dob)
    .bind(gender)
    .bind(nationality)
    .bind(address)
    .bind(visa_expiry)
    .bind(scan_path)
    .bind(phone)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;

    Ok(guest_id)
}
