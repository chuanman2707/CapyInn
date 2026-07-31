use sqlx::{Pool, Sqlite, Transaction};

use crate::db::local_day::local_date_sql;

use crate::{
    domain::booking::BookingResult,
    models::{AuditLog, NightAuditSnapshot},
};

pub async fn find_audit_log_id(
    pool: &Pool<Sqlite>,
    audit_date: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM night_audit_logs WHERE audit_date = ?")
        .bind(audit_date)
        .fetch_optional(pool)
        .await
}

pub async fn insert_night_audit_log_tx(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &NightAuditSnapshot,
    notes: Option<&str>,
    created_by: &str,
) -> BookingResult<AuditLog> {
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Local::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO night_audit_logs
         (id, audit_date, total_revenue, room_revenue, folio_revenue,
          total_expenses, occupancy_pct, rooms_sold, total_rooms,
          notes, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&snapshot.audit_date)
    .bind(snapshot.total_revenue)
    .bind(snapshot.room_revenue)
    .bind(snapshot.folio_revenue)
    .bind(snapshot.total_expenses)
    .bind(snapshot.occupancy_pct)
    .bind(snapshot.rooms_sold)
    .bind(snapshot.total_rooms)
    .bind(notes)
    .bind(created_by)
    .bind(&created_at)
    .execute(&mut **tx)
    .await?;

    Ok(AuditLog {
        id,
        audit_date: snapshot.audit_date.clone(),
        total_revenue: snapshot.total_revenue,
        room_revenue: snapshot.room_revenue,
        folio_revenue: snapshot.folio_revenue,
        total_expenses: snapshot.total_expenses,
        occupancy_pct: snapshot.occupancy_pct,
        rooms_sold: snapshot.rooms_sold,
        total_rooms: snapshot.total_rooms,
        notes: notes.map(str::to_string),
        created_at,
    })
}

/// Stamps every stay up to and including `audit_date` as audited.
///
/// The day comes from `local_day`, not from `DATE()` on the raw stamp. `DATE()`
/// converts an offset stamp to UTC, so a 02:00 arrival on the 17th read as the
/// 16th and a close of the 16th stamped it — every check-in before 07:00 local,
/// and a write rather than a display.
pub async fn mark_bookings_audited_tx(
    tx: &mut Transaction<'_, Sqlite>,
    audit_date: &str,
) -> Result<(), sqlx::Error> {
    let check_in_day = local_date_sql("check_in_at");
    let audit_day = local_date_sql("?");

    sqlx::query(&format!(
        "UPDATE bookings
         SET is_audited = 1
         WHERE {check_in_day} <= {audit_day}
           AND status IN ('active', 'checked_out')",
    ))
    .bind(audit_date)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
