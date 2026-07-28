//! Reads behind the dashboard activity feed.
//!
//! Three independent "most recent N" queries. They are not unioned in SQL
//! because each one carries different columns; the command merges and sorts
//! them, which it can do as a pure function once the rows are in hand.

use sqlx::{Pool, Row, Sqlite};

pub struct RecentCheckIn {
    pub room_id: String,
    pub guest_name: String,
    pub check_in_at: String,
}

pub struct RecentCheckOut {
    pub room_id: String,
    pub guest_name: String,
    pub actual_checkout: String,
}

pub struct RecentHousekeeping {
    pub room_id: String,
    pub status: String,
    pub triggered_at: String,
}

pub async fn load_recent_check_ins(
    pool: &Pool<Sqlite>,
    limit: i32,
) -> Result<Vec<RecentCheckIn>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.room_id, g.full_name, b.check_in_at
         FROM bookings b JOIN guests g ON g.id = b.primary_guest_id
         ORDER BY b.check_in_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| RecentCheckIn {
            room_id: row.get("room_id"),
            guest_name: row.get("full_name"),
            check_in_at: row.get("check_in_at"),
        })
        .collect())
}

pub async fn load_recent_check_outs(
    pool: &Pool<Sqlite>,
    limit: i32,
) -> Result<Vec<RecentCheckOut>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.room_id, g.full_name, b.actual_checkout
         FROM bookings b JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.actual_checkout IS NOT NULL
         ORDER BY b.actual_checkout DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| RecentCheckOut {
            room_id: row.get("room_id"),
            guest_name: row.get("full_name"),
            actual_checkout: row.get("actual_checkout"),
        })
        .collect())
}

pub async fn load_recent_housekeeping(
    pool: &Pool<Sqlite>,
    limit: i32,
) -> Result<Vec<RecentHousekeeping>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT room_id, status, triggered_at FROM housekeeping
         ORDER BY triggered_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| RecentHousekeeping {
            room_id: row.get("room_id"),
            status: row.get("status"),
            triggered_at: row.get("triggered_at"),
        })
        .collect())
}
