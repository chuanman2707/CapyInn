use chrono::{Duration, Local, NaiveDate};
use sqlx::{Pool, Row, Sqlite};

use crate::{
    domain::booking::{pricing::calculate_stay_price_tx, BookingError, BookingResult},
    models::{status, BookingGroup, GroupCheckinRequest, GroupCheckoutRequest},
};

use super::{
    billing_service::{record_charge_tx, record_payment_tx},
    guest_service::{create_group_guest_manifest, link_booking_guests},
    support::{begin_immediate_tx, insert_room_calendar_rows},
};

const GROUP_ACTIVE: &str = "active";
const GROUP_BOOKED: &str = "booked";
const GROUP_COMPLETED: &str = "completed";
const GROUP_PARTIAL_CHECKOUT: &str = "partial_checkout";

pub async fn group_checkin(
    pool: &Pool<Sqlite>,
    user_id: Option<String>,
    req: GroupCheckinRequest,
) -> BookingResult<BookingGroup> {
    validate_group_checkin_request(&req)?;

    let now = Local::now();
    let now_rfc3339 = now.to_rfc3339();
    let today_str = now.format("%Y-%m-%d").to_string();
    let is_reservation = req
        .check_in_date
        .as_ref()
        .map(|date| date != &today_str)
        .unwrap_or(false);
    let checkin_date = req.check_in_date.clone().unwrap_or(today_str);
    let checkin_naive = parse_date(&checkin_date)?;
    let checkout_naive = checkin_naive + Duration::days(req.nights as i64);
    let checkout_date = checkout_naive.format("%Y-%m-%d").to_string();

    let mut tx = begin_immediate_tx(pool).await?;
    validate_rooms_for_group(
        &mut tx,
        &req.room_ids,
        is_reservation,
        &checkin_date,
        &checkout_date,
    )
    .await?;

    let group_id = uuid::Uuid::new_v4().to_string();
    let group_status = if is_reservation {
        GROUP_BOOKED
    } else {
        GROUP_ACTIVE
    };
    sqlx::query(
        "INSERT INTO booking_groups (
            id, group_name, organizer_name, organizer_phone, total_rooms, status, notes, created_by, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&group_id)
    .bind(&req.group_name)
    .bind(&req.organizer_name)
    .bind(req.organizer_phone.as_deref())
    .bind(req.room_ids.len() as i32)
    .bind(group_status)
    .bind(req.notes.as_deref())
    .bind(user_id.as_deref())
    .bind(&now_rfc3339)
    .execute(&mut *tx)
    .await?;

    let paid_total = req.paid_amount.unwrap_or(0.0);
    let paid_per_room = if req.room_ids.is_empty() {
        0.0
    } else {
        paid_total / req.room_ids.len() as f64
    };
    let mut master_booking_id: Option<String> = None;

    for room_id in &req.room_ids {
        let is_master = room_id == &req.master_room_id;
        let room_guests = req
            .guests_per_room
            .get(room_id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let guest_manifest = create_group_guest_manifest(
            &mut tx,
            room_guests,
            &format!("Khách đoàn {} - {}", req.group_name, room_id),
            &now_rfc3339,
        )
        .await?;

        let booking_id = uuid::Uuid::new_v4().to_string();
        let booking_status = if is_reservation {
            status::booking::BOOKED
        } else {
            status::booking::ACTIVE
        };
        let booking_type = if is_reservation {
            "reservation"
        } else {
            "walk-in"
        };
        let booking_checkin_at = if is_reservation {
            format!("{}T14:00:00+07:00", &checkin_date)
        } else {
            now_rfc3339.clone()
        };
        let booking_checkout_at = if is_reservation {
            format!("{}T12:00:00+07:00", &checkout_date)
        } else {
            (now + Duration::days(req.nights as i64)).to_rfc3339()
        };
        let pricing = calculate_stay_price_tx(
            &mut tx,
            room_id,
            if is_reservation {
                &checkin_date
            } else {
                &booking_checkin_at
            },
            if is_reservation {
                &checkout_date
            } else {
                &booking_checkout_at
            },
            "nightly",
        )
        .await?;
        let deposit_amount = if is_reservation { paid_per_room } else { 0.0 };
        let guest_phone = room_guests.first().and_then(|guest| guest.phone.as_deref());

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout,
                nights, total_price, paid_amount, status, source, notes, created_by,
                booking_type, pricing_type, deposit_amount, guest_phone, scheduled_checkin,
                scheduled_checkout, group_id, is_master_room, pricing_snapshot, created_at
             ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, ?, ?, 'nightly', ?, ?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(&booking_id)
        .bind(room_id)
        .bind(&guest_manifest.primary_guest_id)
        .bind(&booking_checkin_at)
        .bind(&booking_checkout_at)
        .bind(req.nights)
        .bind(pricing.total)
        .bind(booking_status)
        .bind(req.source.as_deref().unwrap_or("walk-in"))
        .bind(req.notes.as_deref())
        .bind(user_id.as_deref())
        .bind(booking_type)
        .bind(deposit_amount)
        .bind(guest_phone)
        .bind(if is_reservation {
            Some(checkin_date.as_str())
        } else {
            None
        })
        .bind(if is_reservation {
            Some(checkout_date.as_str())
        } else {
            None
        })
        .bind(&group_id)
        .bind(if is_master { 1 } else { 0 })
        .bind(&now_rfc3339)
        .execute(&mut *tx)
        .await?;

        if is_master {
            master_booking_id = Some(booking_id.clone());
        }

        link_booking_guests(&mut tx, &booking_id, &guest_manifest.guest_ids).await?;

        if !is_reservation {
            record_charge_tx(
                &mut tx,
                &booking_id,
                pricing.total,
                "Tiền phòng (đoàn)",
                booking_checkin_at.clone(),
            )
            .await?;

            if paid_per_room > 0.0 {
                record_payment_tx(
                    &mut tx,
                    &booking_id,
                    paid_per_room,
                    "Thanh toán group check-in",
                )
                .await?;
            }
        } else if paid_per_room > 0.0 {
            record_payment_tx(&mut tx, &booking_id, paid_per_room, "Đặt cọc đoàn").await?;
        }

        insert_group_calendar_rows(
            &mut tx,
            room_id,
            &booking_id,
            checkin_naive,
            checkout_naive,
            if is_reservation {
                status::calendar::BOOKED
            } else {
                status::calendar::OCCUPIED
            },
        )
        .await?;

        if !is_reservation {
            sqlx::query("UPDATE rooms SET status = ? WHERE id = ?")
                .bind(status::room::OCCUPIED)
                .bind(room_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    if let Some(ref booking_id) = master_booking_id {
        sqlx::query("UPDATE booking_groups SET master_booking_id = ? WHERE id = ?")
            .bind(booking_id)
            .bind(&group_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await.map_err(BookingError::from)?;

    fetch_group(pool, &group_id).await
}

pub async fn group_checkout(pool: &Pool<Sqlite>, req: GroupCheckoutRequest) -> BookingResult<()> {
    if req.booking_ids.is_empty() {
        return Err(BookingError::validation(
            "Phải chọn ít nhất 1 phòng để checkout".to_string(),
        ));
    }

    let now = Local::now().to_rfc3339();
    let mut tx = begin_immediate_tx(pool).await?;

    let mut query_builder: sqlx::QueryBuilder<Sqlite> = sqlx::QueryBuilder::new(
        "SELECT id, room_id FROM bookings WHERE status = "
    );
    query_builder.push_bind(status::booking::ACTIVE);
    query_builder.push(" AND group_id = ");
    query_builder.push_bind(&req.group_id);
    query_builder.push(" AND id IN (");
    let mut separated = query_builder.separated(", ");
    for id in &req.booking_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    let rows = query_builder.build().fetch_all(&mut *tx).await?;
    let mut booking_room_map = std::collections::HashMap::new();
    for row in rows {
        let id: String = row.get("id");
        let room_id: String = row.get("room_id");
        booking_room_map.insert(id, room_id);
    }

    for id in &req.booking_ids {
        if !booking_room_map.contains_key(id) {
            return Err(BookingError::not_found(format!(
                "Booking {} không tìm thấy hoặc đã checkout",
                id
            )));
        }
    }

    let room_ids: Vec<String> = booking_room_map.values().cloned().collect();

    let mut qb = sqlx::QueryBuilder::new("UPDATE bookings SET status = ");
    qb.push_bind(status::booking::CHECKED_OUT);
    qb.push(", actual_checkout = ");
    qb.push_bind(&now);
    qb.push(" WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in &req.booking_ids {
        sep.push_bind(id);
    }
    sep.push_unseparated(")");
    qb.build().execute(&mut *tx).await?;

    let mut qb = sqlx::QueryBuilder::new("UPDATE rooms SET status = ");
    qb.push_bind(status::room::CLEANING);
    qb.push(" WHERE id IN (");
    let mut sep = qb.separated(", ");
    for rid in &room_ids {
        sep.push_bind(rid);
    }
    sep.push_unseparated(")");
    qb.build().execute(&mut *tx).await?;

    let mut qb = sqlx::QueryBuilder::new(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at) "
    );
    qb.push_values(&room_ids, |mut b, rid| {
        b.push_bind(uuid::Uuid::new_v4().to_string())
            .push_bind(rid)
            .push_bind("needs_cleaning")
            .push_bind(&now)
            .push_bind(&now);
    });
    qb.build().execute(&mut *tx).await?;

    let mut qb = sqlx::QueryBuilder::new("DELETE FROM room_calendar WHERE booking_id IN (");
    let mut sep = qb.separated(", ");
    for id in &req.booking_ids {
        sep.push_bind(id);
    }
    sep.push_unseparated(")");
    qb.build().execute(&mut *tx).await?;

    maybe_reassign_master_booking(&mut tx, &req.group_id, &req.booking_ids).await?;

    let remaining_active: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = ?")
            .bind(&req.group_id)
            .bind(status::booking::ACTIVE)
            .fetch_one(&mut *tx)
            .await?;

    sqlx::query("UPDATE booking_groups SET status = ? WHERE id = ?")
        .bind(if remaining_active.0 == 0 {
            GROUP_COMPLETED
        } else {
            GROUP_PARTIAL_CHECKOUT
        })
        .bind(&req.group_id)
        .execute(&mut *tx)
        .await?;

    if let Some(final_paid) = req.final_paid.filter(|amount| *amount > 0.0) {
        let target_booking: (String,) = sqlx::query_as(
            "SELECT id
             FROM bookings
             WHERE group_id = ?
             ORDER BY CASE WHEN status = ? THEN 0 ELSE 1 END, created_at ASC
             LIMIT 1",
        )
        .bind(&req.group_id)
        .bind(status::booking::ACTIVE)
        .fetch_one(&mut *tx)
        .await?;

        record_payment_tx(
            &mut tx,
            &target_booking.0,
            final_paid,
            "Thanh toán group checkout",
        )
        .await?;
    }

    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}

fn validate_group_checkin_request(req: &GroupCheckinRequest) -> BookingResult<()> {
    if req.room_ids.is_empty() {
        return Err(BookingError::validation(
            "Phải chọn ít nhất 1 phòng".to_string(),
        ));
    }
    if req.nights <= 0 {
        return Err(BookingError::validation("Số đêm phải > 0".to_string()));
    }
    if !req.room_ids.contains(&req.master_room_id) {
        return Err(BookingError::validation(
            "Phòng đại diện phải nằm trong danh sách phòng".to_string(),
        ));
    }
    let unique_room_count = req
        .room_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    if unique_room_count != req.room_ids.len() {
        return Err(BookingError::validation(
            "Phòng không được lặp trong cùng một group".to_string(),
        ));
    }

    Ok(())
}

async fn validate_rooms_for_group(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_ids: &[String],
    is_reservation: bool,
    checkin_date: &str,
    checkout_date: &str,
) -> BookingResult<()> {
    for room_id in room_ids {
        let room_status = sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
            .bind(room_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| BookingError::not_found(format!("Phòng {} không tồn tại", room_id)))?;

        if !is_reservation && room_status != status::room::VACANT {
            return Err(BookingError::conflict(format!(
                "Phòng {} không trống (status: {})",
                room_id, room_status
            )));
        }

        let conflicts: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ?",
        )
        .bind(room_id)
        .bind(checkin_date)
        .bind(checkout_date)
        .fetch_one(&mut **tx)
        .await?;

        if conflicts.0 > 0 {
            return Err(BookingError::conflict(format!(
                "Phòng {} có lịch trùng trong khoảng ngày đã chọn",
                room_id
            )));
        }
    }

    Ok(())
}

async fn maybe_reassign_master_booking(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    group_id: &str,
    checked_out_booking_ids: &[String],
) -> BookingResult<()> {
    let current_master = sqlx::query_scalar::<_, String>(
        "SELECT master_booking_id FROM booking_groups WHERE id = ? LIMIT 1",
    )
    .bind(group_id)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(current_master) = current_master else {
        return Ok(());
    };

    if !checked_out_booking_ids.contains(&current_master) {
        return Ok(());
    }

    let next_master = sqlx::query_scalar::<_, String>(
        "SELECT id FROM bookings WHERE group_id = ? AND status = ? ORDER BY created_at ASC LIMIT 1",
    )
    .bind(group_id)
    .bind(status::booking::ACTIVE)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(next_master) = next_master {
        sqlx::query("UPDATE bookings SET is_master_room = 0 WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE bookings SET is_master_room = 1 WHERE id = ?")
            .bind(&next_master)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE booking_groups SET master_booking_id = ? WHERE id = ?")
            .bind(&next_master)
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query("UPDATE bookings SET is_master_room = 0 WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE booking_groups SET master_booking_id = NULL WHERE id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

async fn insert_group_calendar_rows(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    calendar_status: &str,
) -> BookingResult<()> {
    insert_room_calendar_rows(tx, room_id, booking_id, from, to, calendar_status).await
}

async fn fetch_group(pool: &Pool<Sqlite>, group_id: &str) -> BookingResult<BookingGroup> {
    let row = sqlx::query(
        "SELECT id, group_name, master_booking_id, organizer_name, organizer_phone,
                total_rooms, status, notes, created_by, created_at
         FROM booking_groups
         WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(pool)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Không tìm thấy group {}", group_id)))?;

    Ok(BookingGroup {
        id: row.get("id"),
        group_name: row.get("group_name"),
        master_booking_id: row.get("master_booking_id"),
        organizer_name: row.get("organizer_name"),
        organizer_phone: row.get("organizer_phone"),
        total_rooms: row.get("total_rooms"),
        status: row.get("status"),
        notes: row.get("notes"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

fn parse_date(value: &str) -> BookingResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| BookingError::datetime_parse(error.to_string()))
}
