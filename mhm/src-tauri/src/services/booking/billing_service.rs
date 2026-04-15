use sqlx::{Pool, Sqlite};

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
    .execute(&mut *tx)
    .await?;

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
    .execute(&mut *tx)
    .await?;

    let result = sqlx::query(
        "UPDATE bookings
         SET paid_amount = COALESCE(paid_amount, 0) + ?
         WHERE id = ?",
    )
    .bind(amount)
    .bind(booking_id)
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        return Err(BookingError::not_found(format!(
            "Không tìm thấy booking {}",
            booking_id
        )));
    }

    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}
