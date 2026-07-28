//! Reads behind the group screens.
//!
//! `load_group_detail` composes three reads and totals them. That composition
//! is a read, so it belongs here rather than in a service — there is no state
//! machine, only arithmetic over rows.

use sqlx::{Pool, Row, Sqlite};

use crate::db::row::{get_money_vnd, get_optional_money_vnd};
use crate::models::{BookingGroup, BookingWithGuest, GroupDetailResponse, GroupService};

const GROUP_BOOKINGS_SQL: &str =
    "SELECT b.id, b.room_id, r.name as room_name, g.full_name as guest_name,
            b.check_in_at, b.expected_checkout, b.actual_checkout, b.nights,
            b.total_price, b.paid_amount, b.status, b.source,
            b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout,
            b.guest_phone
     FROM bookings b
     JOIN rooms r ON r.id = b.room_id
     JOIN guests g ON g.id = b.primary_guest_id
     WHERE b.group_id = ?
     ORDER BY r.floor, r.id";

pub async fn load_group(pool: &Pool<Sqlite>, group_id: &str) -> Result<BookingGroup, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM booking_groups WHERE id = ?")
        .bind(group_id)
        .fetch_one(pool)
        .await?;

    Ok(map_group(&row))
}

/// Every group, or only those in `status`, newest first.
pub async fn load_groups(
    pool: &Pool<Sqlite>,
    status: Option<&str>,
) -> Result<Vec<BookingGroup>, sqlx::Error> {
    let rows = match status {
        Some(status) => {
            sqlx::query("SELECT * FROM booking_groups WHERE status = ? ORDER BY created_at DESC")
                .bind(status)
                .fetch_all(pool)
                .await?
        }
        None => {
            sqlx::query("SELECT * FROM booking_groups ORDER BY created_at DESC")
                .fetch_all(pool)
                .await?
        }
    };

    Ok(rows.iter().map(map_group).collect())
}

pub async fn load_group_bookings(
    pool: &Pool<Sqlite>,
    group_id: &str,
) -> Result<Vec<BookingWithGuest>, sqlx::Error> {
    let rows = sqlx::query(GROUP_BOOKINGS_SQL)
        .bind(group_id)
        .fetch_all(pool)
        .await?;

    Ok(rows.iter().map(map_group_booking).collect())
}

pub async fn load_group_services(
    pool: &Pool<Sqlite>,
    group_id: &str,
) -> Result<Vec<GroupService>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM group_services WHERE group_id = ? ORDER BY created_at")
        .bind(group_id)
        .fetch_all(pool)
        .await?;

    Ok(rows.iter().map(map_group_service).collect())
}

/// The group, its rooms, its services, and what they add up to.
pub async fn load_group_detail(
    pool: &Pool<Sqlite>,
    group_id: &str,
) -> Result<GroupDetailResponse, sqlx::Error> {
    let group = load_group(pool, group_id).await?;
    let bookings = load_group_bookings(pool, group_id).await?;
    let services = load_group_services(pool, group_id).await?;

    Ok(total_group_detail(group, bookings, services))
}

/// The arithmetic half of `load_group_detail`, kept separate so the totals can
/// be checked without a database.
fn total_group_detail(
    group: BookingGroup,
    bookings: Vec<BookingWithGuest>,
    services: Vec<GroupService>,
) -> GroupDetailResponse {
    let total_room_cost = bookings.iter().map(|b| b.total_price).sum();
    let total_service_cost = services.iter().map(|s| s.total_price).sum();
    let paid_amount = bookings.iter().map(|b| b.paid_amount).sum();

    GroupDetailResponse {
        group,
        bookings,
        services,
        total_room_cost,
        total_service_cost,
        grand_total: total_room_cost + total_service_cost,
        paid_amount,
    }
}

fn map_group(row: &sqlx::sqlite::SqliteRow) -> BookingGroup {
    BookingGroup {
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
    }
}

fn map_group_booking(row: &sqlx::sqlite::SqliteRow) -> BookingWithGuest {
    BookingWithGuest {
        id: row.get("id"),
        room_id: row.get("room_id"),
        room_name: row.get("room_name"),
        guest_name: row.get("guest_name"),
        check_in_at: row.get("check_in_at"),
        expected_checkout: row.get("expected_checkout"),
        actual_checkout: row.get("actual_checkout"),
        nights: row.get("nights"),
        total_price: get_money_vnd(row, "total_price"),
        paid_amount: get_money_vnd(row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        booking_type: row.get("booking_type"),
        deposit_amount: get_optional_money_vnd(row, "deposit_amount"),
        scheduled_checkin: row.get("scheduled_checkin"),
        scheduled_checkout: row.get("scheduled_checkout"),
        guest_phone: row.get("guest_phone"),
    }
}

fn map_group_service(row: &sqlx::sqlite::SqliteRow) -> GroupService {
    GroupService {
        id: row.get("id"),
        group_id: row.get("group_id"),
        booking_id: row.get("booking_id"),
        name: row.get("name"),
        quantity: row.get("quantity"),
        unit_price: get_money_vnd(row, "unit_price"),
        total_price: get_money_vnd(row, "total_price"),
        note: row.get("note"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    }
}
