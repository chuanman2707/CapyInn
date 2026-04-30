use sqlx::{Pool, Sqlite, Transaction};

use crate::models::FolioLine;
use crate::repositories::booking::folio_repository;
use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        SanitizedLedgerIntent, WriteCommandContext, WriteCommandExecutor, WriteCommandRequest,
    },
    db_error_monitoring::classify_db_error_code,
    domain::booking::{BookingError, BookingResult, OriginSideEffect},
    money::{validate_transport_money_vnd, MoneyVnd},
};
use serde_json::json;

use super::support::{begin_tx, rfc3339_now};

fn validate_whole_positive_vnd(amount: MoneyVnd) -> BookingResult<MoneyVnd> {
    let amount = validate_whole_vnd(amount, "amount")?;
    if amount <= 0 {
        return Err(BookingError::validation(
            "Folio amount must be a whole positive VND amount",
        ));
    }
    Ok(amount)
}

fn validate_whole_vnd(amount: MoneyVnd, field: &str) -> BookingResult<MoneyVnd> {
    validate_transport_money_vnd(amount, field)
        .map_err(|error| BookingError::validation(error.message))
}

#[allow(dead_code)]
pub async fn add_folio_line(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: MoneyVnd,
    created_by: Option<&str>,
) -> BookingResult<FolioLine> {
    let amount = validate_whole_positive_vnd(amount)?;

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

fn map_add_folio_line_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::Conflict(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message)
            if message.starts_with("Không tìm thấy booking ")
                || message.starts_with("Booking not found:") =>
        {
            CommandError::user(codes::BOOKING_NOT_FOUND, message)
        }
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            if message.contains("FOREIGN KEY constraint failed") {
                return CommandError::user(codes::BOOKING_NOT_FOUND, "Booking not found");
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) | BookingError::NotFound(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

fn add_folio_line_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("folio add line lock payload missing booking_id"))?;

    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

pub async fn add_folio_line_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
    category: &str,
    description: &str,
    amount: MoneyVnd,
    created_by: Option<&str>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let amount = validate_whole_positive_vnd(amount).map_err(|error| {
        map_add_folio_line_command_error(error).with_request_id(ctx.request_id.clone())
    })?;

    let amount_minor_units = (i128::from(amount) * 100).to_string();
    let hash_payload = json!({
        "schema": "folio.add_line.v1",
        "booking_id": booking_id,
        "category": category,
        "description": description,
        "amount_minor_units": amount_minor_units,
        "created_by": created_by,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("folio.add_line.v1")),
        ("booking_present", json!(true)),
        ("category_present", json!(!category.trim().is_empty())),
        ("amount_present", json!(true)),
        ("description_present", json!(!description.trim().is_empty())),
        ("created_by_present", json!(created_by.is_some())),
    ])?;
    let summary = CommandLedgerSummary::new("Add folio line")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(add_folio_line_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Folio line added")?);

    let booking_id = booking_id.to_string();
    let category = category.to_string();
    let description = description.to_string();
    let created_by = created_by.map(ToString::to_string);
    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);

    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                let origin = OriginSideEffect::new(origin_key, 0).map_err(system_error)?;
                let line = folio_repository::insert_folio_line_with_origin_tx(
                    tx,
                    &booking_id,
                    &category,
                    &description,
                    amount,
                    created_by.as_deref(),
                    &rfc3339_now(),
                    &origin,
                )
                .await
                .map_err(map_add_folio_line_command_error)?;
                serde_json::to_value(&line).map_err(system_error)
            })
        })
        .await
}

#[allow(dead_code)]
pub async fn record_payment(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
) -> BookingResult<()> {
    let mut tx = begin_tx(pool).await?;
    record_payment_tx(&mut tx, booking_id, amount, note).await?;

    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}

fn map_record_payment_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message) => CommandError::user(codes::BOOKING_NOT_FOUND, message),
        BookingError::Conflict(message) => {
            CommandError::user(codes::CONFLICT_INVALID_STATE_TRANSITION, message)
        }
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

pub async fn record_payment_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
    amount: MoneyVnd,
    note: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let amount = validate_whole_positive_vnd(amount).map_err(|error| {
        map_record_payment_command_error(error).with_request_id(ctx.request_id.clone())
    })?;
    let hash_payload = json!({
        "schema": "payment.record.v1",
        "booking_id": booking_id,
        "amount": amount,
        "note": note,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("booking_credit.record.v1")),
        ("booking_present", json!(true)),
        ("amount_vnd_units", json!(amount)),
        ("note_present", json!(!note.trim().is_empty())),
    ])?;
    let summary = CommandLedgerSummary::new("Record credit")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(|payload| {
            let booking_id = payload
                .get("booking_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| system_error("payment lock payload missing booking_id"))?;
            Ok(vec![
                crate::aggregate_locks::booking_key(booking_id)?,
                crate::aggregate_locks::folio_key(booking_id)?,
            ])
        })
        .with_success_summary(CommandLedgerResultSummary::success("Credit recorded")?);

    let booking_id_for_guard = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let note = note.to_string();
    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_pre_transaction_guard(
            ctx,
            request,
            move || async move {
                crate::aggregate_locks::global_manager()
                    .acquire([
                        crate::aggregate_locks::booking_key(&booking_id_for_guard)?,
                        crate::aggregate_locks::folio_key(&booking_id_for_guard)?,
                    ])
                    .await
            },
            move |tx| {
                Box::pin(async move {
                    let origin = OriginSideEffect::new(origin_key, 0).map_err(system_error)?;
                    let transaction_id = record_payment_with_origin_returning_id_tx(
                        tx,
                        &booking_id_for_service,
                        amount,
                        note,
                        &origin,
                    )
                    .await
                    .map_err(map_record_payment_command_error)?;
                    let response = crate::models::RecordPaymentResponse {
                        ok: true,
                        booking_id: booking_id_for_service,
                        transaction_id,
                        amount,
                    };
                    serde_json::to_value(response).map_err(system_error)
                })
            },
        )
        .await
}

pub async fn record_charge_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    created_at: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(
        tx, booking_id, amount, note, "charge", created_at, false, None,
    )
    .await
}

pub async fn record_charge_with_origin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    created_at: impl Into<String>,
    origin: &OriginSideEffect,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "charge",
        created_at,
        false,
        Some(origin),
    )
    .await
}

pub async fn record_payment_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "payment",
        rfc3339_now(),
        true,
        None,
    )
    .await
}

#[allow(dead_code)]
pub async fn record_payment_with_origin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    origin: &OriginSideEffect,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "payment",
        rfc3339_now(),
        true,
        Some(origin),
    )
    .await
}

#[allow(dead_code)]
pub async fn record_payment_returning_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
) -> BookingResult<String> {
    record_payment_money_returning_id_tx(tx, booking_id, amount, note, None).await
}

#[allow(dead_code)]
pub async fn record_payment_with_origin_returning_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    origin: &OriginSideEffect,
) -> BookingResult<String> {
    record_payment_money_returning_id_tx(tx, booking_id, amount, note, Some(origin)).await
}

async fn record_payment_money_returning_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let note = note.into();
    let created_at = rfc3339_now();
    let amount = validate_whole_vnd(amount, "transaction amount")?;
    let origin_idempotency_key = origin.map(OriginSideEffect::key);
    let origin_transaction_ordinal = origin.map(OriginSideEffect::ordinal).unwrap_or(0);

    sqlx::query(
        "INSERT INTO transactions (
            id, booking_id, amount, type, note, origin_idempotency_key,
            origin_transaction_ordinal, created_at
        )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(booking_id)
    .bind(amount)
    .bind("payment")
    .bind(&note)
    .bind(origin_idempotency_key)
    .bind(origin_transaction_ordinal)
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

    Ok(id)
}

pub async fn record_deposit_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "deposit",
        rfc3339_now(),
        true,
        None,
    )
    .await
}

#[allow(dead_code)]
pub async fn record_deposit_with_origin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    origin: &OriginSideEffect,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "deposit",
        rfc3339_now(),
        true,
        Some(origin),
    )
    .await
}

pub async fn record_cancellation_fee_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
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
        None,
    )
    .await
}

pub async fn record_cancellation_fee_with_origin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    origin: &OriginSideEffect,
) -> BookingResult<()> {
    record_money_tx(
        tx,
        booking_id,
        amount,
        note,
        "cancellation_fee",
        rfc3339_now(),
        false,
        Some(origin),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn record_money_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    note: impl Into<String>,
    txn_type: &str,
    created_at: impl Into<String>,
    update_paid_amount: bool,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let note = note.into();
    let created_at = created_at.into();
    let amount = validate_whole_vnd(amount, "transaction amount")?;
    let origin_idempotency_key = origin.map(OriginSideEffect::key);
    let origin_transaction_ordinal = origin.map(OriginSideEffect::ordinal).unwrap_or(0);

    sqlx::query(
        "INSERT INTO transactions (
            id, booking_id, amount, type, note, origin_idempotency_key,
            origin_transaction_ordinal, created_at
        )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(booking_id)
    .bind(amount)
    .bind(txn_type)
    .bind(&note)
    .bind(origin_idempotency_key)
    .bind(origin_transaction_ordinal)
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
