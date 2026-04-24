use sqlx::{Pool, Sqlite, Transaction};

use crate::domain::booking::{BookingError, BookingResult};
use crate::models::FolioLine;
use crate::repositories::booking::folio_repository;

use super::support::{begin_tx, rfc3339_now};

pub async fn add_folio_line(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: f64,
    created_by: Option<&str>,
) -> BookingResult<FolioLine> {
    if amount <= 0.0 {
        return Err(BookingError::validation(
            "Folio amount must be greater than zero",
        ));
    }

    let mut tx = begin_tx(pool).await?;
    let created_at = rfc3339_now();
    let line = folio_repository::insert_folio_line_tx(
        &mut tx,
        booking_id,
        category,
        description,
        amount,
        created_by,
        &created_at,
    )
    .await?;

    tx.commit().await.map_err(BookingError::from)?;

    Ok(line)
}

#[allow(dead_code)]
pub async fn record_payment(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
) -> BookingResult<()> {
    let mut tx = begin_tx(pool).await?;
    record_payment_tx(&mut tx, booking_id, amount, note).await?;

    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}

pub async fn record_charge_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
    created_at: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(tx, booking_id, amount, note, "charge", created_at, false).await
}

pub async fn record_payment_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(tx, booking_id, amount, note, "payment", rfc3339_now(), true).await
}

pub async fn record_deposit_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(tx, booking_id, amount, note, "deposit", rfc3339_now(), true).await
}

pub async fn record_cancellation_fee_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "cancellation_fee",
        rfc3339_now(),
        false,
    )
    .await
}

async fn record_money_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
    txn_type: &str,
    created_at: impl Into<String>,
    update_paid_amount: bool,
) -> BookingResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let note = note.into();
    let created_at = created_at.into();

    sqlx::query(
        "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(booking_id)
    .bind(amount)
    .bind(txn_type)
    .bind(&note)
    .bind(&created_at)
    .execute(&mut **tx)
    .await?;

    if update_paid_amount {
        let result = sqlx::query(
            "UPDATE bookings
             SET paid_amount = COALESCE(paid_amount, 0) + ?
             WHERE id = ?",
        )
        .bind(amount)
        .bind(booking_id)
        .execute(&mut **tx)
        .await?;

        if result.rows_affected() == 0 {
            return Err(BookingError::not_found(format!(
                "Không tìm thấy booking {}",
                booking_id
            )));
        }
    }

    Ok(())
}
