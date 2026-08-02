use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        SanitizedLedgerIntent, WriteCommandContext, WriteCommandExecutor, WriteCommandRequest,
    },
    db::row::{get_money_vnd, get_optional_money_vnd},
    models::InvoiceData,
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

const DEFAULT_POLICY_TEXT: &str = "• Check-in: 14:00 | Check-out: 12:00\n• Cancel 24h+ before: full deposit refund\n• Cancel within 24h: 50% deposit retained\n• No refund for no-show";

pub async fn generate_invoice_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
) -> Result<InvoiceData, String> {
    // Always regenerate: delete any existing invoice for this booking
    sqlx::query("DELETE FROM invoices WHERE booking_id = ?")
        .bind(booking_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

    // Fetch booking
    let b = sqlx::query(
        "SELECT b.id, b.room_id, b.primary_guest_id, b.check_in_at, b.expected_checkout,
                b.actual_checkout, b.nights, b.total_price, b.paid_amount, b.status, b.notes,
                b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout,
                b.pricing_snapshot, b.pricing_type,
                r.name as room_name, r.type as room_type, r.base_price,
                g.full_name as guest_name, g.phone as guest_phone
         FROM bookings b
         JOIN rooms r ON r.id = b.room_id
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    let b = b.ok_or_else(|| format!("Booking '{}' not found", booking_id))?;

    // Get hotel info from settings (stored as JSON blob under "hotel_info")
    let hotel_info: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'hotel_info'")
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
    let hotel = crate::domain::hotel_info::HotelInfo::from_settings_json(hotel_info.as_deref());
    let (hotel_name, hotel_address, hotel_phone) = (hotel.name, hotel.address, hotel.phone);

    let room_name: String = b.get("room_name");
    let room_type: String = b.get("room_type");
    let guest_name: String = b.get("guest_name");
    let guest_phone: Option<String> = b.get("guest_phone");
    let nights: i32 = b.get("nights");
    let total_price = get_money_vnd(&b, "total_price");
    let paid_amount = get_money_vnd(&b, "paid_amount");
    let deposit_amount = get_optional_money_vnd(&b, "deposit_amount").unwrap_or(0);
    let notes: Option<String> = b.get("notes");
    let pricing_snapshot: Option<String> = b.get("pricing_snapshot");
    let snapshot_value: Option<serde_json::Value> = pricing_snapshot
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());

    let checkout_settlement = snapshot_value
        .as_ref()
        .and_then(|value| value.get("checkout_settlement").cloned());

    let settlement_label = checkout_settlement
        .as_ref()
        .and_then(|value| value.get("label").cloned())
        .and_then(|value| value.as_str().map(str::to_string));
    let settlement_mode = checkout_settlement
        .as_ref()
        .and_then(|value| value.get("mode").cloned())
        .and_then(|value| value.as_str().map(str::to_string));
    let settlement_reporting_checkout = checkout_settlement
        .as_ref()
        .and_then(|value| value.get("reporting_checkout").cloned())
        .and_then(|value| value.as_str().map(str::to_string));

    let check_in: String = b
        .try_get::<String, _>("scheduled_checkin")
        .ok()
        .or_else(|| Some(b.get::<String, _>("check_in_at")))
        .unwrap();
    let scheduled_checkout = b
        .try_get::<Option<String>, _>("scheduled_checkout")
        .ok()
        .flatten();
    let expected_checkout = b.try_get::<String, _>("expected_checkout").ok();
    let actual_checkout = b
        .try_get::<Option<String>, _>("actual_checkout")
        .ok()
        .flatten();
    let check_out: String = if settlement_mode.as_deref() == Some("booked_nights") {
        settlement_reporting_checkout
            .or(scheduled_checkout)
            .or(expected_checkout)
            .or(actual_checkout)
            .unwrap_or_default()
    } else {
        actual_checkout
            .or(scheduled_checkout)
            .or(expected_checkout)
            .unwrap_or_default()
    };

    // Keep invoice wording aligned with the settlement metadata when checkout finalized with
    // an explicit settlement mode.
    let per_night = if nights > 0 {
        total_price / i64::from(nights)
    } else {
        total_price
    };
    let mut breakdown: Vec<crate::pricing::PricingLine> = vec![crate::pricing::PricingLine {
        label: settlement_label.unwrap_or_else(|| format!("{} night(s) x {}d", nights, per_night)),
        amount: total_price,
    }];

    let subtotal = total_price;
    let balance_due = (total_price - paid_amount).max(0);

    // If the booking moved rooms mid-stay, replace the single combined room
    // line with one line per room actually occupied, grouped by the night the
    // guest first slept there, in occupancy order.
    //
    // Resolution order for the (room name, nights) pairs:
    //  1. `pricing_snapshot.room_stays`, written by `change_room_tx`
    //     (room_change.rs) at move time — authoritative once present and
    //     non-empty, and the ONLY source left once the guest has checked out:
    //     `check_out_tx` (stay_lifecycle.rs) deletes every `room_calendar`
    //     row for the booking, so a booking invoiced after checkout has
    //     nothing left in `room_calendar` to group.
    //  2. `room_calendar`, grouped by room — still correct for a booking
    //     invoiced while the guest is in-house (checkout hasn't run yet).
    //  3. Neither present ⇒ the pre-existing single line built above.
    let room_stays_from_snapshot: Vec<(String, i64)> = snapshot_value
        .as_ref()
        .and_then(|value| value.get("room_stays"))
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let room_name = entry.get("room_name")?.as_str()?.to_string();
                    let nights = entry.get("nights")?.as_i64()?;
                    Some((room_name, nights))
                })
                .collect::<Vec<(String, i64)>>()
        })
        .unwrap_or_default();

    let room_entries: Vec<(String, i64)> = if !room_stays_from_snapshot.is_empty() {
        room_stays_from_snapshot
    } else {
        let room_nights = sqlx::query(
            "SELECT rc.room_id, r.name AS room_name, COUNT(*) AS nights
             FROM room_calendar rc
             JOIN rooms r ON r.id = rc.room_id
             WHERE rc.booking_id = ?
             GROUP BY rc.room_id, r.name
             ORDER BY MIN(rc.date)",
        )
        .bind(booking_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

        room_nights
            .into_iter()
            .map(|row| {
                let room_name: String = row.get("room_name");
                let nights: i64 = row.get("nights");
                (room_name, nights)
            })
            .collect()
    };

    if room_entries.len() > 1 {
        let total_nights: i64 = room_entries.iter().map(|(_, nights)| nights).sum();
        if total_nights > 0 {
            let mut allocated: crate::money::MoneyVnd = 0;
            let last_index = room_entries.len() - 1;
            let mut room_lines = Vec::with_capacity(room_entries.len());
            for (index, (room_name, nights_here)) in room_entries.into_iter().enumerate() {
                let amount = if index == last_index {
                    subtotal - allocated
                } else {
                    let share = subtotal * nights_here / total_nights;
                    allocated += share;
                    share
                };
                room_lines.push(crate::pricing::PricingLine {
                    label: format!("Phòng {room_name} × {nights_here} đêm"),
                    amount,
                });
            }
            // Preserve any non-room lines already present (surcharges, etc.):
            // only the original single combined room line is being replaced.
            let extra_lines: Vec<crate::pricing::PricingLine> =
                breakdown.into_iter().skip(1).collect();
            breakdown = room_lines;
            breakdown.extend(extra_lines);
        }
    }

    // Generate invoice number: INV-YYYYMMDD-XXX
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let prefix = format!("INV-{}", today);
    let max_row: (Option<String>,) =
        sqlx::query_as("SELECT MAX(invoice_number) FROM invoices WHERE invoice_number LIKE ?")
            .bind(format!("{}-%", prefix))
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
    let next_seq = match max_row.0 {
        Some(ref last) => {
            last.rsplit('-')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0)
                + 1
        }
        None => 1,
    };
    let invoice_number = format!("{}-{:03}", prefix, next_seq);

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();
    let breakdown_json = serde_json::to_string(&breakdown).unwrap_or_default();

    sqlx::query(
        "INSERT INTO invoices (id, invoice_number, booking_id, hotel_name, hotel_address, hotel_phone,
         guest_name, guest_phone, room_name, room_type, check_in, check_out, nights,
         pricing_breakdown, subtotal, deposit_amount, total, balance_due, policy_text, notes,
         status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'issued', ?)"
    )
    .bind(&id).bind(&invoice_number).bind(booking_id)
    .bind(&hotel_name).bind(&hotel_address).bind(&hotel_phone)
    .bind(&guest_name).bind(&guest_phone)
    .bind(&room_name).bind(&room_type)
    .bind(&check_in).bind(&check_out).bind(nights)
    .bind(&breakdown_json)
    .bind(subtotal).bind(deposit_amount).bind(total_price).bind(balance_due)
    .bind(DEFAULT_POLICY_TEXT).bind(&notes)
    .bind(&now)
    .execute(&mut **tx).await.map_err(|e| e.to_string())?;

    Ok(InvoiceData {
        id,
        invoice_number,
        booking_id: booking_id.to_string(),
        hotel_name,
        hotel_address,
        hotel_phone,
        guest_name,
        guest_phone,
        room_name,
        room_type,
        check_in,
        check_out,
        nights,
        pricing_breakdown: breakdown,
        subtotal,
        deposit_amount,
        total: total_price,
        balance_due,
        policy_text: Some(DEFAULT_POLICY_TEXT.to_string()),
        notes,
        status: "issued".to_string(),
        created_at: now,
    })
}

#[cfg(test)]
pub async fn generate_invoice_direct(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<InvoiceData, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let invoice = generate_invoice_tx(&mut tx, booking_id).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(invoice)
}

pub async fn get_invoice(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<Option<InvoiceData>, String> {
    let row = sqlx::query(
        "SELECT id, invoice_number, booking_id, hotel_name, hotel_address, hotel_phone,
                guest_name, guest_phone, room_name, room_type, check_in, check_out, nights,
                pricing_breakdown, subtotal, deposit_amount, total, balance_due,
                policy_text, notes, status, created_at
         FROM invoices WHERE booking_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(booking_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let breakdown_json: String = r.get("pricing_breakdown");
            let breakdown: Vec<crate::pricing::PricingLine> =
                serde_json::from_str(&breakdown_json).unwrap_or_default();

            Ok(Some(InvoiceData {
                id: r.get("id"),
                invoice_number: r.get("invoice_number"),
                booking_id: r.get("booking_id"),
                hotel_name: r.get("hotel_name"),
                hotel_address: r.get("hotel_address"),
                hotel_phone: r.get("hotel_phone"),
                guest_name: r.get("guest_name"),
                guest_phone: r.get("guest_phone"),
                room_name: r.get("room_name"),
                room_type: r.get("room_type"),
                check_in: r.get("check_in"),
                check_out: r.get("check_out"),
                nights: r.get("nights"),
                pricing_breakdown: breakdown,
                subtotal: get_money_vnd(&r, "subtotal"),
                deposit_amount: get_money_vnd(&r, "deposit_amount"),
                total: get_money_vnd(&r, "total"),
                balance_due: get_money_vnd(&r, "balance_due"),
                policy_text: r.get("policy_text"),
                notes: r.get("notes"),
                status: r.get("status"),
                created_at: r.get("created_at"),
            }))
        }
        None => Ok(None),
    }
}

pub async fn generate_invoice_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = json!({
        "schema": "invoice.generate.v1",
        "booking_id": booking_id,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("invoice.generate.v1")),
        ("booking_present", json!(true)),
    ])?;
    let booking_ref_id = booking_id.to_string();
    let summary = CommandLedgerSummary::new("Generate invoice")?.with_aggregate_ref(
        "booking",
        booking_ref_id,
        None::<String>,
    )?;
    let runtime_lock_keys = generate_invoice_lock_keys_from_payload(&hash_payload)?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(generate_invoice_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Invoice generated")?)
        .with_outbox_event(OutboxEventSpec::new(
            "invoice.generated",
            OutboxAggregateKeySource::response_field("invoice", "id"),
            &["invoice", "bookings", "folio"],
        )?);
    let booking_id = booking_id.to_string();

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
                    let invoice = generate_invoice_tx(tx, &booking_id)
                        .await
                        .map_err(map_generate_invoice_error)?;
                    serde_json::to_value(invoice).map_err(system_error)
                })
            },
        )
        .await
}

fn generate_invoice_lock_keys_from_payload(
    payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = payload
        .get("booking_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| system_error("invoice generation lock payload missing booking_id"))?;

    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

fn map_generate_invoice_error(message: String) -> CommandError {
    if crate::db_error_monitoring::classify_db_error_code(&message)
        == Some(codes::DB_LOCKED_RETRYABLE)
    {
        return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
    }

    if message.starts_with("Booking '") && message.ends_with(" not found") {
        return CommandError::user(codes::BOOKING_NOT_FOUND, message);
    }

    system_error(message)
}
