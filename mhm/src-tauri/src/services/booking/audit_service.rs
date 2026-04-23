use chrono::NaiveDate;
use sqlx::Pool;
use sqlx::Sqlite;

use crate::{
    domain::booking::{BookingError, BookingResult},
    models::AuditLog,
    queries::booking::audit_queries,
    repositories::booking::night_audit_repository,
};

use super::support::begin_tx;

fn mark_write_db_error(error: BookingError) -> BookingError {
    match error {
        BookingError::Database(message) => BookingError::database_write(message),
        other => other,
    }
}

pub async fn run_night_audit(
    pool: &Pool<Sqlite>,
    audit_date: &str,
    notes: Option<String>,
    created_by: &str,
) -> BookingResult<AuditLog> {
    NaiveDate::parse_from_str(audit_date, "%Y-%m-%d")
        .map_err(|_| BookingError::validation("Ngày audit không hợp lệ"))?;

    if night_audit_repository::find_audit_log_id(pool, audit_date)
        .await?
        .is_some()
    {
        return Err(BookingError::validation(format!(
            "Đã audit ngày {} rồi!",
            audit_date
        )));
    }

    let snapshot = audit_queries::load_night_audit_snapshot(pool, audit_date).await?;
    let mut tx = begin_tx(pool).await.map_err(mark_write_db_error)?;

    let log = night_audit_repository::insert_night_audit_log_tx(
        &mut tx,
        &snapshot,
        notes.as_deref(),
        created_by,
    )
    .await
    .map_err(mark_write_db_error)?;
    night_audit_repository::mark_bookings_audited_tx(&mut tx, audit_date)
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    tx.commit()
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::mark_write_db_error;
    use crate::domain::booking::BookingError;

    #[test]
    fn mark_write_db_error_promotes_database_errors_but_preserves_missing_record() {
        assert_eq!(
            mark_write_db_error(BookingError::database("disk full")),
            BookingError::database_write("disk full")
        );
        assert_eq!(
            mark_write_db_error(BookingError::not_found("Booking not found: booking-1")),
            BookingError::not_found("Booking not found: booking-1")
        );
        assert_eq!(
            mark_write_db_error(BookingError::database_write("disk full")),
            BookingError::database_write("disk full")
        );
    }
}
