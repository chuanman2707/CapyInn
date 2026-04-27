use sqlx::{Pool, Sqlite, Transaction};

use crate::{
    domain::booking::{BookingResult, OriginSideEffect},
    models::FolioLine,
};

// Retained as the public pool-based compatibility API; composed flows use insert_folio_line_tx.
#[allow(dead_code)]
pub async fn insert_folio_line(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: f64,
    created_by: Option<&str>,
    created_at: &str,
) -> BookingResult<FolioLine> {
    let mut tx = pool.begin().await?;
    let line = insert_folio_line_tx(
        &mut tx,
        booking_id,
        category,
        description,
        amount,
        created_by,
        created_at,
    )
    .await?;

    tx.commit().await?;

    Ok(line)
}

pub async fn insert_folio_line_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: f64,
    created_by: Option<&str>,
    created_at: &str,
) -> BookingResult<FolioLine> {
    insert_folio_line_internal_tx(
        tx,
        booking_id,
        category,
        description,
        amount,
        created_by,
        created_at,
        None,
    )
    .await
}

#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub async fn insert_folio_line_with_origin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: f64,
    created_by: Option<&str>,
    created_at: &str,
    origin: &OriginSideEffect,
) -> BookingResult<FolioLine> {
    insert_folio_line_internal_tx(
        tx,
        booking_id,
        category,
        description,
        amount,
        created_by,
        created_at,
        Some(origin),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_folio_line_internal_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: f64,
    created_by: Option<&str>,
    created_at: &str,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<FolioLine> {
    let id = uuid::Uuid::new_v4().to_string();
    let origin_idempotency_key = origin.map(OriginSideEffect::key);
    let origin_line_ordinal = origin.map(OriginSideEffect::ordinal).unwrap_or(0);

    sqlx::query(
        "INSERT INTO folio_lines (
            id, booking_id, category, description, amount, created_by,
            origin_idempotency_key, origin_line_ordinal, created_at
        )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(booking_id)
    .bind(category)
    .bind(description)
    .bind(amount)
    .bind(created_by)
    .bind(origin_idempotency_key)
    .bind(origin_line_ordinal)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;

    Ok(FolioLine {
        id,
        booking_id: booking_id.to_string(),
        category: category.to_string(),
        description: description.to_string(),
        amount,
        created_by: created_by.map(str::to_string),
        created_at: created_at.to_string(),
    })
}
