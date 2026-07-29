//! Ghi bù một lượt khách đã ở nhưng chưa được nhập máy.
//!
//! Chủ nhà quên nhập khách vãng lai thì phải ghi lại sau: hoặc khách đã trả
//! phòng, hoặc khách vẫn còn nằm trong phòng. Cả hai trường hợp đều ghi trọn
//! một lượt ở (booking, sổ tiền, lịch phòng, hồ sơ khách) trong một giao dịch.

use chrono::{Local, NaiveDate, TimeZone};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    app_error::CommandResult,
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        SanitizedLedgerIntent, WriteCommandContext, WriteCommandExecutor, WriteCommandRequest,
    },
    domain::booking::{BookingError, BookingResult, OriginSideEffect},
    models::{status, BackfillStayRequest, Booking},
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};

use super::{
    billing_service::{record_charge_with_origin_tx, record_payment_with_origin_tx},
    guest_service::{create_guest_manifest, guest_hash_payload_entries, link_booking_guests},
    stay_lifecycle::{fetch_booking_tx, map_check_in_command_error, mark_write_db_error},
    support::{
        ensure_one_row_affected, insert_room_calendar_rows, validate_non_negative_booking_money,
    },
};

struct BackfillDates {
    check_in: NaiveDate,
    /// Ngày ra thực tế (đã trả) hoặc ngày ra dự kiến (còn ở).
    end: NaiveDate,
    still_staying: bool,
}

fn parse_iso_date(value: &str, label: &str) -> BookingResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| BookingError::validation(format!("{} không đúng định dạng YYYY-MM-DD", label)))
}

fn validate_backfill_request(
    req: &BackfillStayRequest,
    today: NaiveDate,
) -> BookingResult<BackfillDates> {
    if req.guests.is_empty() {
        return Err(BookingError::validation(
            "Phải có ít nhất 1 khách".to_string(),
        ));
    }
    validate_non_negative_booking_money(req.total_price, "total_price")?;
    validate_non_negative_booking_money(req.paid_amount, "paid_amount")?;
    if req.paid_amount > req.total_price {
        return Err(BookingError::validation(
            "Số tiền đã thu không được vượt quá tiền phòng".to_string(),
        ));
    }

    let check_in = parse_iso_date(&req.check_in_date, "Ngày vào")?;
    if check_in >= today {
        return Err(BookingError::validation(
            "Ngày vào của ghi bù phải trong quá khứ".to_string(),
        ));
    }

    match (&req.check_out_date, &req.expected_checkout_date) {
        (Some(check_out_date), _) => {
            let end = parse_iso_date(check_out_date, "Ngày ra")?;
            if end <= check_in {
                return Err(BookingError::validation(
                    "Ngày ra phải sau ngày vào".to_string(),
                ));
            }
            if end > today {
                return Err(BookingError::validation(
                    "Khách đã trả phòng thì ngày ra không được ở tương lai".to_string(),
                ));
            }
            Ok(BackfillDates {
                check_in,
                end,
                still_staying: false,
            })
        }
        (None, Some(expected)) => {
            let end = parse_iso_date(expected, "Ngày ra dự kiến")?;
            if end <= today {
                return Err(BookingError::validation(
                    "Ngày ra dự kiến phải sau hôm nay".to_string(),
                ));
            }
            Ok(BackfillDates {
                check_in,
                end,
                still_staying: true,
            })
        }
        (None, None) => Err(BookingError::validation(
            "Thiếu ngày ra dự kiến cho khách còn ở".to_string(),
        )),
    }
}

fn local_datetime_rfc3339(date: NaiveDate, hour: u32) -> BookingResult<String> {
    let naive = date
        .and_hms_opt(hour, 0, 0)
        .ok_or_else(|| BookingError::datetime_parse(format!("invalid time {hour}:00")))?;
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            Ok(dt.to_rfc3339())
        }
        chrono::LocalResult::None => Err(BookingError::datetime_parse(format!(
            "invalid local datetime {naive}"
        ))),
    }
}

async fn backfill_stay_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: &BackfillStayRequest,
    user_id: Option<&str>,
    origin_key: &str,
) -> BookingResult<Booking> {
    let today = Local::now().date_naive();
    let dates = validate_backfill_request(req, today)?;

    // Quy ước giờ chuẩn khách sạn: nhận 14:00, trả 12:00.
    let check_in_at = local_datetime_rfc3339(dates.check_in, 14)?;
    let end_at = local_datetime_rfc3339(dates.end, 12)?;
    let nights = (dates.end - dates.check_in).num_days() as i32;

    let room = sqlx::query("SELECT id, status FROM rooms WHERE id = ?")
        .bind(&req.room_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy phòng {}", req.room_id)))?;
    let room_status: String = room.get("status");
    if dates.still_staying && room_status != status::room::VACANT {
        return Err(BookingError::conflict(format!(
            "Phòng {} đang có khách, không thể ghi bù khách còn ở",
            req.room_id
        )));
    }

    let conflicts = sqlx::query(
        "SELECT rc.date, COALESCE(g.full_name, '') AS guest_name
         FROM room_calendar rc
         LEFT JOIN bookings b ON b.id = rc.booking_id
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE rc.room_id = ? AND rc.date >= ? AND rc.date < ?
         ORDER BY rc.date ASC",
    )
    .bind(&req.room_id)
    .bind(dates.check_in.format("%Y-%m-%d").to_string())
    .bind(dates.end.format("%Y-%m-%d").to_string())
    .fetch_all(&mut **tx)
    .await?;
    if let Some(first) = conflicts.first() {
        let date: String = first.get("date");
        let guest_name: String = first.get("guest_name");
        return Err(BookingError::conflict(format!(
            "Phòng {} đã có khách ngày {} ({})",
            req.room_id, date, guest_name
        )));
    }

    let booking_id = uuid::Uuid::new_v4().to_string();
    let guest_manifest = create_guest_manifest(tx, &req.guests, &check_in_at)
        .await
        .map_err(mark_write_db_error)?;

    let (booking_status, actual_checkout) = if dates.still_staying {
        (status::booking::ACTIVE, None)
    } else {
        (status::booking::CHECKED_OUT, Some(end_at.clone()))
    };

    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            actual_checkout, nights, total_price, paid_amount, status, source,
            notes, created_by, booking_type, pricing_type, pricing_snapshot, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, 'walk-in', 'nightly', NULL, ?)",
    )
    .bind(&booking_id)
    .bind(&req.room_id)
    .bind(&guest_manifest.primary_guest_id)
    .bind(&check_in_at)
    .bind(&end_at)
    .bind(&actual_checkout)
    .bind(nights)
    .bind(req.total_price)
    .bind(booking_status)
    .bind(req.source.as_deref().unwrap_or("walk-in"))
    .bind(&req.notes)
    .bind(user_id)
    .bind(&check_in_at)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;

    link_booking_guests(tx, &booking_id, &guest_manifest.guest_ids)
        .await
        .map_err(mark_write_db_error)?;

    let charge_origin = OriginSideEffect::new(origin_key, 0)?;
    record_charge_with_origin_tx(
        tx,
        &booking_id,
        req.total_price,
        "Tiền phòng (ghi bù)",
        check_in_at.clone(),
        &charge_origin,
    )
    .await
    .map_err(mark_write_db_error)?;

    if req.paid_amount > 0 {
        let payment_origin = OriginSideEffect::new(origin_key, 1)?;
        record_payment_with_origin_tx(
            tx,
            &booking_id,
            req.paid_amount,
            "Thanh toán (ghi bù)",
            &payment_origin,
        )
        .await
        .map_err(mark_write_db_error)?;
    }

    insert_room_calendar_rows(
        tx,
        &req.room_id,
        &booking_id,
        dates.check_in,
        dates.end,
        status::calendar::OCCUPIED,
    )
    .await
    .map_err(mark_write_db_error)?;

    if dates.still_staying {
        let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
            .bind(status::room::OCCUPIED)
            .bind(&req.room_id)
            .bind(status::room::VACANT)
            .execute(&mut **tx)
            .await
            .map_err(BookingError::from)
            .map_err(mark_write_db_error)?;
        ensure_one_row_affected(result, format!("room {} is no longer vacant", req.room_id))?;
    }

    fetch_booking_tx(tx, &booking_id).await
}

fn build_backfill_hash_payload(req: &BackfillStayRequest) -> serde_json::Value {
    json!({
        "schema": "stay.backfill.v1",
        "room_id": req.room_id.clone(),
        "guests": guest_hash_payload_entries(&req.guests),
        "check_in_date": req.check_in_date.clone(),
        "check_out_date": req.check_out_date.clone(),
        "expected_checkout_date": req.expected_checkout_date.clone(),
        "total_price": req.total_price,
        "paid_amount": req.paid_amount,
        "source": req.source.clone(),
        "notes": req.notes.clone(),
    })
}

fn backfill_lock_keys_from_payload(hash_payload: &serde_json::Value) -> CommandResult<Vec<String>> {
    let room_id = hash_payload
        .get("room_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("backfill lock payload missing room_id"))?;
    Ok(vec![crate::aggregate_locks::room_key(room_id)?])
}

// Lệnh Tauri gọi hàm này được nối ở bước sau; hiện chỉ có test gọi tới.
#[allow(dead_code)]
pub async fn backfill_stay_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: BackfillStayRequest,
    user_id: Option<String>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let today = Local::now().date_naive();
    validate_backfill_request(&req, today).map_err(|error| {
        map_check_in_command_error(error).with_request_id(ctx.request_id.clone())
    })?;

    let hash_payload = build_backfill_hash_payload(&req);
    let still_staying = req.check_out_date.is_none();
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.backfill.v1")),
        ("room_present", json!(true)),
        ("guest_count", json!(req.guests.len())),
        ("still_staying", json!(still_staying)),
        ("total_price_vnd", json!(req.total_price)),
        ("paid_amount_vnd", json!(req.paid_amount)),
        ("source_present", json!(req.source.is_some())),
    ])?;
    let summary = CommandLedgerSummary::new("Backfill stay")?.with_aggregate_ref(
        "room",
        "room",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload.clone(), ledger_intent, summary)?
        .with_primary_aggregate_key(format!("room:{}", req.room_id))
        .with_lock_key_deriver(backfill_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Backfilled")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.backfilled",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms", "folio"],
        )?);

    let runtime_lock_keys = backfill_lock_keys_from_payload(&hash_payload)?;
    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);
    let req_for_service = req;
    let user_id_for_service = user_id;

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
                    let booking = backfill_stay_tx(
                        tx,
                        &req_for_service,
                        user_id_for_service.as_deref(),
                        origin_key.as_str(),
                    )
                    .await
                    .map_err(map_check_in_command_error)?;
                    serde_json::to_value(&booking).map_err(system_error)
                })
            },
        )
        .await
}
