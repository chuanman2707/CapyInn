use sqlx::{Pool, Sqlite, Transaction};

use crate::domain::booking::{BookingError, BookingResult};

use super::support::{begin_tx, rfc3339_now};

pub async fn record_charge(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
    created_at: impl Into<String>,
) -> BookingResult<()> {
    let mut tx = begin_tx(pool).await?;
    record_charge_tx(&mut tx, booking_id, amount, note, created_at).await?;

    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}

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
    let id = uuid::Uuid::new_v4().to_string();
    let note = note.into();
    let created_at = created_at.into();

    sqlx::query(
        "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
         VALUES (?, ?, ?, 'charge', ?, ?)",
    )
    .bind(&id)
    .bind(booking_id)
    .bind(amount)
    .bind(&note)
    .bind(&created_at)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn record_payment_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: f64,
    note: impl Into<String>,
) -> BookingResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let note = note.into();
    let created_at = rfc3339_now();

    sqlx::query(
        "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
         VALUES (?, ?, ?, 'payment', ?, ?)",
    )
    .bind(&id)
    .bind(booking_id)
    .bind(amount)
    .bind(&note)
    .bind(&created_at)
    .execute(&mut **tx)
    .await?;

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

    Ok(())
}
