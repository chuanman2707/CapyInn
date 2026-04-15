use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

use crate::domain::booking::BookingResult;

use super::billing_service::record_payment;

pub async fn test_pool() -> Pool<Sqlite> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("failed to open sqlite test pool");

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("failed to enable foreign keys");

    sqlx::query(
        "CREATE TABLE rooms (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            type TEXT NOT NULL,
            floor INTEGER NOT NULL,
            has_balcony INTEGER NOT NULL DEFAULT 0,
            base_price REAL NOT NULL DEFAULT 0,
            max_guests INTEGER NOT NULL DEFAULT 2,
            extra_person_fee REAL NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'vacant'
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create rooms table");

    sqlx::query(
        "CREATE TABLE guests (
            id TEXT PRIMARY KEY,
            full_name TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create guests table");

    sqlx::query(
        "CREATE TABLE bookings (
            id TEXT PRIMARY KEY,
            room_id TEXT NOT NULL REFERENCES rooms(id),
            primary_guest_id TEXT NOT NULL REFERENCES guests(id),
            check_in_at TEXT NOT NULL,
            expected_checkout TEXT NOT NULL,
            actual_checkout TEXT,
            nights INTEGER NOT NULL,
            total_price REAL NOT NULL,
            paid_amount REAL,
            status TEXT NOT NULL,
            source TEXT,
            notes TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create bookings table");

    sqlx::query(
        "CREATE TABLE transactions (
            id TEXT PRIMARY KEY,
            booking_id TEXT NOT NULL REFERENCES bookings(id),
            amount REAL NOT NULL,
            type TEXT NOT NULL,
            note TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create transactions table");

    pool
}

pub async fn seed_room(pool: &Pool<Sqlite>, room_id: &str) -> BookingResult<()> {
    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, ?, 0, 0, 2, 0, 'vacant')",
    )
    .bind(room_id)
    .bind(format!("Room {}", room_id))
    .bind("standard")
    .bind(1_i32)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_active_booking(pool: &Pool<Sqlite>, booking_id: &str, room_id: &str) -> BookingResult<()> {
    let guest_id = format!("guest-{}", booking_id);
    let now = "2026-04-15T10:00:00+07:00";

    sqlx::query("INSERT INTO guests (id, full_name, created_at) VALUES (?, ?, ?)")
        .bind(&guest_id)
        .bind(format!("Guest {}", booking_id))
        .bind(now)
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            actual_checkout, nights, total_price, paid_amount, status,
            source, notes, created_at
        ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, NULL, 'active', ?, ?, ?)",
    )
    .bind(booking_id)
    .bind(room_id)
    .bind(&guest_id)
    .bind(now)
    .bind("2026-04-16T10:00:00+07:00")
    .bind(1_i64)
    .bind(250_000.0_f64)
    .bind("walk-in")
    .bind("seed booking")
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

use sqlx::Row;

#[tokio::test]
async fn record_payment_updates_paid_amount_cache() {
    let pool = test_pool().await;
    seed_room(&pool, "R101").await.unwrap();
    seed_active_booking(&pool, "B101", "R101").await.unwrap();

    record_payment(&pool, "B101", 25_000.0, "deposit")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B101")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), Some(25_000.0));

    let txn = sqlx::query("SELECT type, amount, note FROM transactions WHERE booking_id = ?")
        .bind("B101")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "payment");
    assert_eq!(txn.get::<f64, _>("amount"), 25_000.0);
    assert_eq!(txn.get::<String, _>("note"), "deposit");
}
