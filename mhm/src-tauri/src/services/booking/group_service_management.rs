use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        SanitizedLedgerIntent, WriteCommandContext, WriteCommandExecutor, WriteCommandRequest,
    },
    db_error_monitoring::classify_db_error_code,
    domain::booking::BookingError,
    models::{AddGroupServiceRequest, GroupService, RemoveGroupServiceResponse},
    money::{validate_non_negative_money_vnd, MoneyVnd},
};
use serde_json::json;

fn map_group_service_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy group ") => {
            CommandError::user(codes::GROUP_NOT_FOUND, message)
        }
        BookingError::NotFound(message)
            if message.starts_with("Không tìm thấy booking ")
                || message.starts_with("Booking not found:")
                || message.starts_with("Booking ") =>
        {
            CommandError::user(codes::BOOKING_NOT_FOUND, message)
        }
        BookingError::NotFound(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::Database(message) | BookingError::DatabaseWrite(message) => {
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

fn validate_quantity(quantity: i32) -> Result<i32, BookingError> {
    if quantity < 1 {
        return Err(BookingError::validation(
            "Quantity must be greater than zero",
        ));
    }
    Ok(quantity)
}

fn validate_unit_price(unit_price: MoneyVnd) -> Result<MoneyVnd, BookingError> {
    validate_non_negative_money_vnd(unit_price, "unit_price")
        .map_err(|error| BookingError::validation(error.message))
}

fn compute_total_price(quantity: i32, unit_price: MoneyVnd) -> Result<MoneyVnd, BookingError> {
    validate_quantity(quantity)?;
    let unit_price = validate_unit_price(unit_price)?;
    i64::from(quantity)
        .checked_mul(unit_price)
        .ok_or_else(|| BookingError::validation("total_price overflowed"))
}

fn add_group_service_lock_keys_from_payload(
    payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let group_id = payload
        .get("group_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("group service add lock payload missing group_id"))?;
    let mut keys = vec![crate::aggregate_locks::group_key(group_id)?];
    if let Some(booking_id) = payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
    {
        keys.push(crate::aggregate_locks::booking_key(booking_id)?);
        keys.push(crate::aggregate_locks::folio_key(booking_id)?);
    }
    Ok(keys)
}

fn remove_group_service_hash_payload(service_id: &str) -> serde_json::Value {
    json!({
        "schema": "group_service.remove.v1",
        "service_id": service_id,
    })
}

async fn ensure_group_exists_tx(
    tx: &mut Transaction<'_, Sqlite>,
    group_id: &str,
) -> Result<(), BookingError> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM booking_groups WHERE id = ?")
        .bind(group_id)
        .fetch_optional(&mut **tx)
        .await?;
    if exists.is_none() {
        return Err(BookingError::not_found(format!(
            "Không tìm thấy group {group_id}"
        )));
    }
    Ok(())
}

async fn ensure_booking_belongs_to_group_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    group_id: &str,
) -> Result<(), BookingError> {
    let stored_group_id: Option<Option<String>> =
        sqlx::query_scalar("SELECT group_id FROM bookings WHERE id = ?")
            .bind(booking_id)
            .fetch_optional(&mut **tx)
            .await?;
    match stored_group_id {
        Some(Some(stored)) if stored == group_id => Ok(()),
        Some(_) => Err(BookingError::conflict("Booking does not belong to group")),
        None => Err(BookingError::not_found(format!(
            "Không tìm thấy booking {booking_id}"
        ))),
    }
}

pub async fn add_group_service_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: AddGroupServiceRequest,
    actor_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let total_price = compute_total_price(req.quantity, req.unit_price).map_err(|error| {
        map_group_service_command_error(error).with_request_id(ctx.request_id.clone())
    })?;
    let hash_payload = json!({
        "schema": "group_service.add.v1",
        "group_id": req.group_id.clone(),
        "booking_id": req.booking_id.clone(),
        "name": req.name.clone(),
        "quantity": req.quantity,
        "unit_price": req.unit_price,
        "note": req.note.clone(),
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("group_service.add.v1")),
        ("group_present", json!(true)),
        (
            "booking_present",
            json!(hash_payload
                .get("booking_id")
                .and_then(serde_json::Value::as_str)
                .is_some()),
        ),
        ("quantity", json!(req.quantity)),
        ("unit_price_vnd_units", json!(req.unit_price)),
        (
            "note_present",
            json!(hash_payload
                .get("note")
                .and_then(serde_json::Value::as_str)
                .is_some()),
        ),
    ])?;
    let summary = CommandLedgerSummary::new("Add group service")?.with_aggregate_ref(
        "group",
        "group",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload.clone(), ledger_intent, summary)?
        .with_primary_aggregate_key(format!("group:{}", req.group_id))
        .with_lock_key_deriver(add_group_service_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Group service added")?);
    let runtime_lock_keys = add_group_service_lock_keys_from_payload(&hash_payload)?;
    let actor_id = actor_id.to_string();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_pre_transaction_guard(
            ctx,
            request,
            move || async move {
                crate::aggregate_locks::global_manager()
                    .acquire(runtime_lock_keys)
                    .await
            },
            move |tx| {
                Box::pin(async move {
                    ensure_group_exists_tx(tx, &req.group_id)
                        .await
                        .map_err(map_group_service_command_error)?;
                    if let Some(booking_id) = req.booking_id.as_deref() {
                        ensure_booking_belongs_to_group_tx(tx, booking_id, &req.group_id)
                            .await
                            .map_err(map_group_service_command_error)?;
                    }
                    let id = uuid::Uuid::new_v4().to_string();
                    let created_at = super::support::rfc3339_now();
                    sqlx::query(
                        "INSERT INTO group_services (
                            id, group_id, booking_id, name, quantity, unit_price,
                            total_price, note, created_by, created_at
                        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&id)
                    .bind(&req.group_id)
                    .bind(&req.booking_id)
                    .bind(&req.name)
                    .bind(req.quantity)
                    .bind(req.unit_price)
                    .bind(total_price)
                    .bind(&req.note)
                    .bind(&actor_id)
                    .bind(&created_at)
                    .execute(&mut **tx)
                    .await
                    .map_err(|error| map_group_service_command_error(BookingError::from(error)))?;

                    let service = GroupService {
                        id,
                        group_id: req.group_id,
                        booking_id: req.booking_id,
                        name: req.name,
                        quantity: req.quantity,
                        unit_price: req.unit_price,
                        total_price,
                        note: req.note,
                        created_by: Some(actor_id),
                        created_at,
                    };
                    serde_json::to_value(service).map_err(system_error)
                })
            },
        )
        .await
}

struct RemoveGroupServiceLockState {
    group_id: String,
    booking_id: Option<String>,
}

struct RemoveGroupServiceResolvedGuard {
    _guard: crate::aggregate_locks::AggregateLockGuard,
    group_id: String,
    booking_id: Option<String>,
}

async fn load_remove_group_service_lock_state(
    pool: &Pool<Sqlite>,
    service_id: &str,
) -> Result<RemoveGroupServiceLockState, BookingError> {
    let row = sqlx::query("SELECT group_id, booking_id FROM group_services WHERE id = ?")
        .bind(service_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Err(BookingError::not_found(format!(
            "Không tìm thấy service {service_id}"
        )));
    };
    Ok(RemoveGroupServiceLockState {
        group_id: row.get("group_id"),
        booking_id: row.get("booking_id"),
    })
}

async fn resolve_remove_group_service_guard(
    pool: Pool<Sqlite>,
    service_id: String,
) -> CommandResult<
    crate::command_idempotency::ResolvedWriteCommandGuard<RemoveGroupServiceResolvedGuard>,
> {
    let lock_state = load_remove_group_service_lock_state(&pool, &service_id)
        .await
        .map_err(map_group_service_command_error)?;
    let mut lock_keys = vec![crate::aggregate_locks::group_key(&lock_state.group_id)?];
    if let Some(booking_id) = &lock_state.booking_id {
        lock_keys.push(crate::aggregate_locks::booking_key(booking_id)?);
        lock_keys.push(crate::aggregate_locks::folio_key(booking_id)?);
    }
    let guard = crate::aggregate_locks::global_manager()
        .acquire(lock_keys.clone())
        .await?;
    Ok(crate::command_idempotency::ResolvedWriteCommandGuard::new(
        RemoveGroupServiceResolvedGuard {
            _guard: guard,
            group_id: lock_state.group_id,
            booking_id: lock_state.booking_id,
        },
        lock_keys,
    ))
}

pub async fn remove_group_service_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    service_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = remove_group_service_hash_payload(service_id);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("group_service.remove.v1")),
        ("service_present", json!(true)),
    ])?;
    let summary = CommandLedgerSummary::new("Remove group service")?.with_aggregate_ref(
        "group_service",
        "group_service",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("group_service:{service_id}"))
        .with_lock_key_deriver(|payload| {
            let service_id = payload
                .get("service_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    system_error("group service remove lock payload missing service_id")
                })?;
            Ok(vec![format!("group_service:{service_id}")])
        })
        .with_success_summary(CommandLedgerResultSummary::success(
            "Group service removed",
        )?);
    let pool_for_locks = pool.clone();
    let service_id = service_id.to_string();
    let service_id_for_locks = service_id.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || resolve_remove_group_service_guard(pool_for_locks, service_id_for_locks),
            move |tx, resolved| {
                Box::pin(async move {
                    let row =
                        sqlx::query("SELECT group_id, booking_id FROM group_services WHERE id = ?")
                            .bind(&service_id)
                            .fetch_optional(&mut **tx)
                            .await
                            .map_err(|error| {
                                map_group_service_command_error(BookingError::from(error))
                            })?;
                    let Some(row) = row else {
                        return Err(map_group_service_command_error(BookingError::not_found(
                            format!("Không tìm thấy service {service_id}"),
                        )));
                    };
                    let group_id: String = row.get("group_id");
                    let booking_id: Option<String> = row.get("booking_id");
                    debug_assert_eq!(group_id, resolved.group_id);
                    debug_assert_eq!(booking_id, resolved.booking_id);
                    sqlx::query("DELETE FROM group_services WHERE id = ?")
                        .bind(&service_id)
                        .execute(&mut **tx)
                        .await
                        .map_err(|error| {
                            map_group_service_command_error(BookingError::from(error))
                        })?;
                    let response = RemoveGroupServiceResponse {
                        ok: true,
                        service_id,
                        group_id,
                        booking_id,
                    };
                    serde_json::to_value(response).map_err(system_error)
                })
            },
        )
        .await
}
