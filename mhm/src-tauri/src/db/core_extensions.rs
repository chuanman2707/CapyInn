use sqlx::{Pool, Sqlite};

use super::{execute_compat_alter, set_schema_version};

pub(super) async fn migrate_v7_gateway_api_keys(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS gateway_api_keys (
                id TEXT PRIMARY KEY,
                key_hash TEXT NOT NULL,
                label TEXT DEFAULT 'default',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_used_at TEXT
            )",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 7).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v8_invoice_pdf_system(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS invoices (
                id                TEXT PRIMARY KEY,
                invoice_number    TEXT NOT NULL UNIQUE,
                booking_id        TEXT NOT NULL REFERENCES bookings(id),
                hotel_name        TEXT NOT NULL,
                hotel_address     TEXT NOT NULL,
                hotel_phone       TEXT NOT NULL,
                guest_name        TEXT NOT NULL,
                guest_phone       TEXT,
                room_name         TEXT NOT NULL,
                room_type         TEXT NOT NULL,
                check_in          TEXT NOT NULL,
                check_out         TEXT NOT NULL,
                nights            INTEGER NOT NULL,
                pricing_breakdown TEXT NOT NULL,
                subtotal          INTEGER NOT NULL,
                deposit_amount    INTEGER NOT NULL DEFAULT 0,
                total             INTEGER NOT NULL,
                balance_due       INTEGER NOT NULL,
                policy_text       TEXT,
                notes             TEXT,
                status            TEXT NOT NULL DEFAULT 'issued',
                created_at        TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_invoices_booking ON invoices(booking_id)")
        .execute(&mut *tx)
        .await?;

    set_schema_version(&mut tx, 8).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v9_group_booking_system(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // booking_groups: group metadata
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS booking_groups (
                id                TEXT PRIMARY KEY,
                group_name        TEXT NOT NULL,
                master_booking_id TEXT,
                organizer_name    TEXT NOT NULL,
                organizer_phone   TEXT,
                total_rooms       INTEGER NOT NULL,
                status            TEXT NOT NULL DEFAULT 'active',
                notes             TEXT,
                created_by        TEXT,
                created_at        TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    // group_services: per-group add-on charges
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS group_services (
                id          TEXT PRIMARY KEY,
                group_id    TEXT NOT NULL REFERENCES booking_groups(id),
                booking_id  TEXT REFERENCES bookings(id),
                name        TEXT NOT NULL,
                quantity    INTEGER NOT NULL DEFAULT 1,
                unit_price  INTEGER NOT NULL,
                total_price INTEGER NOT NULL,
                note        TEXT,
                created_by  TEXT,
                created_at  TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    // Add group columns to bookings
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN group_id TEXT REFERENCES booking_groups(id)",
    )
    .await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN is_master_room INTEGER DEFAULT 0",
    )
    .await?;

    // Indexes
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookings_group ON bookings(group_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_group_services_group ON group_services(group_id)")
        .execute(&mut *tx)
        .await?;

    set_schema_version(&mut tx, 9).await?;
    tx.commit().await?;
    Ok(())
}

/// Số khách trên booking. Là **số đếm, không phải tiền** — cố ý đứng ngoài
/// `money_migration`, và tên cột cố ý không chứa từ khoá tiền tệ nào.
///
/// Cho phép NULL: booking tạo trước phiên bản này không khai số khách, và NULL
/// có nghĩa là không phụ thu, nên giá của chúng không đổi.
pub(super) async fn migrate_v22_booking_guest_count(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN guests INTEGER").await?;

    set_schema_version(&mut tx, 22).await?;
    tx.commit().await?;
    Ok(())
}

/// Lời quyết toán in cho khách trên hoá đơn tách theo phòng.
///
/// Cố ý KHÔNG dùng lại `invoices.notes`: cột đó chép nguyên `bookings.notes`,
/// tức là ghi chú nội bộ của lễ tân ("cọc 600k", "Agoda thanh toan"), thứ không
/// bao giờ được in cho khách đọc. Tách riêng một cột thì thứ hiển thị được và
/// thứ nội bộ không thể lẫn vào nhau.
///
/// Cho phép NULL: hoá đơn không tách phòng không có gì để nói thêm, và hoá đơn
/// phát hành trước phiên bản này cũng vậy.
pub(super) async fn migrate_v25_invoice_settlement_note(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    execute_compat_alter(&mut tx, "ALTER TABLE invoices ADD COLUMN settlement_note TEXT").await?;

    set_schema_version(&mut tx, 25).await?;
    tx.commit().await?;
    Ok(())
}
