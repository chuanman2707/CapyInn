//! Reads over guests and their stay history.
//!
//! The guest-summary projection (stay count, lifetime spend, last visit) was
//! written out three times across two command modules — the unfiltered list,
//! the name/document search, and the phone lookup behind quick check-in — as
//! three copies of the same nine lines of joins and aggregates. They are one
//! query with three filters, and they are that here.

use sqlx::{Pool, Row, Sqlite};

use crate::db::row::get_money_vnd;
use crate::models::{BookingWithRoom, Guest, GuestSummary};

const GUEST_SUMMARY_SELECT: &str = "SELECT g.id, g.full_name, g.doc_number, g.nationality,
                COUNT(bg.booking_id) as total_stays,
                COALESCE(SUM(b.total_price), 0) as total_spent,
                MAX(b.check_in_at) as last_visit
         FROM guests g
         LEFT JOIN booking_guests bg ON bg.guest_id = g.id
         LEFT JOIN bookings b ON b.id = bg.booking_id";

const GUEST_SUMMARY_TAIL: &str = " GROUP BY g.id ORDER BY last_visit DESC";

fn summary_sql(where_clause: &str, limit: Option<i32>) -> String {
    let mut sql = String::from(GUEST_SUMMARY_SELECT);
    if !where_clause.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(where_clause);
    }
    sql.push_str(GUEST_SUMMARY_TAIL);
    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    sql
}

/// Every guest, or those whose name or document number contains `search`.
pub async fn load_guest_summaries(
    pool: &Pool<Sqlite>,
    search: Option<&str>,
) -> Result<Vec<GuestSummary>, sqlx::Error> {
    let rows = match search {
        Some(search) => {
            let pattern = format!("%{search}%");
            sqlx::query(&summary_sql(
                "g.full_name LIKE ? OR g.doc_number LIKE ?",
                None,
            ))
            .bind(&pattern)
            .bind(&pattern)
            .fetch_all(pool)
            .await?
        }
        None => sqlx::query(&summary_sql("", None)).fetch_all(pool).await?,
    };

    Ok(rows.iter().map(map_guest_summary).collect())
}

/// Guests whose phone number contains `phone`, most recent visit first.
pub async fn search_guest_summaries_by_phone(
    pool: &Pool<Sqlite>,
    phone: &str,
    limit: i32,
) -> Result<Vec<GuestSummary>, sqlx::Error> {
    let rows = sqlx::query(&summary_sql("g.phone LIKE ?", Some(limit)))
        .bind(format!("%{phone}%"))
        .fetch_all(pool)
        .await?;

    Ok(rows.iter().map(map_guest_summary).collect())
}

pub async fn load_guest(pool: &Pool<Sqlite>, guest_id: &str) -> Result<Guest, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM guests WHERE id = ?")
        .bind(guest_id)
        .fetch_one(pool)
        .await?;

    Ok(Guest {
        id: row.get("id"),
        guest_type: row.get("guest_type"),
        full_name: row.get("full_name"),
        doc_number: row.get("doc_number"),
        dob: row.get("dob"),
        gender: row.get("gender"),
        nationality: row.get("nationality"),
        address: row.get("address"),
        visa_expiry: row.get("visa_expiry"),
        scan_path: row.get("scan_path"),
        phone: row.get("phone"),
        created_at: row.get("created_at"),
    })
}

pub async fn load_guest_bookings(
    pool: &Pool<Sqlite>,
    guest_id: &str,
) -> Result<Vec<BookingWithRoom>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.id as booking_id, b.room_id, b.check_in_at, b.expected_checkout,
                b.total_price, b.status
         FROM bookings b
         JOIN booking_guests bg ON bg.booking_id = b.id
         WHERE bg.guest_id = ?
         ORDER BY b.check_in_at DESC",
    )
    .bind(guest_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| BookingWithRoom {
            booking_id: row.get("booking_id"),
            room_id: row.get("room_id"),
            check_in_at: row.get("check_in_at"),
            expected_checkout: row.get("expected_checkout"),
            total_price: get_money_vnd(row, "total_price"),
            status: row.get("status"),
        })
        .collect())
}

fn map_guest_summary(row: &sqlx::sqlite::SqliteRow) -> GuestSummary {
    GuestSummary {
        id: row.get("id"),
        full_name: row.get("full_name"),
        doc_number: row.get("doc_number"),
        nationality: row.get("nationality"),
        total_stays: row.get::<i32, _>("total_stays"),
        total_spent: get_money_vnd(row, "total_spent"),
        last_visit: row.get("last_visit"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        load_guest, load_guest_bookings, load_guest_summaries, search_guest_summaries_by_phone,
        summary_sql,
    };
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    async fn seeded_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");

        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES ('R1', 'Room 1', 'standard', 1, 0, 300000, 2, 0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seed room");

        for (id, name, doc, phone) in [
            ("G1", "Nguyen Van A", "DOC-111", "0901234567"),
            ("G2", "Tran Thi B", "DOC-222", "0909999999"),
            ("G3", "Le Van C", "DOC-333", "0901111111"),
        ] {
            sqlx::query(
                "INSERT INTO guests (id, guest_type, full_name, doc_number, phone, created_at)
                 VALUES (?, 'domestic', ?, ?, ?, '2026-04-01T00:00:00+07:00')",
            )
            .bind(id)
            .bind(name)
            .bind(doc)
            .bind(phone)
            .execute(&pool)
            .await
            .expect("seed guest");
        }

        // G1 stayed twice, G2 once, G3 never.
        for (booking, guest, check_in, price) in [
            ("B1", "G1", "2026-04-02T14:00:00+07:00", 600000),
            ("B2", "G1", "2026-04-08T14:00:00+07:00", 400000),
            ("B3", "G2", "2026-04-05T14:00:00+07:00", 250000),
        ] {
            sqlx::query(
                "INSERT INTO bookings (
                    id, room_id, primary_guest_id, check_in_at, expected_checkout, nights,
                    total_price, paid_amount, status, booking_type, pricing_type, created_at
                 ) VALUES (?, 'R1', ?, ?, '2026-04-30T12:00:00+07:00', 1, ?, 0, 'active',
                           'walk-in', 'nightly', '2026-04-01T00:00:00+07:00')",
            )
            .bind(booking)
            .bind(guest)
            .bind(check_in)
            .bind(price)
            .execute(&pool)
            .await
            .expect("seed booking");
            sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?)")
                .bind(booking)
                .bind(guest)
                .execute(&pool)
                .await
                .expect("link guest");
        }

        pool
    }

    #[test]
    fn the_three_filters_are_the_same_query_with_a_different_where() {
        let unfiltered = summary_sql("", None);
        let searched = summary_sql("g.full_name LIKE ? OR g.doc_number LIKE ?", None);
        let by_phone = summary_sql("g.phone LIKE ?", Some(5));

        // Asserted against the shared constant rather than against literals
        // copied out of it: this should fail when the three stop being one
        // query, not when someone reformats the SQL.
        for sql in [&unfiltered, &searched, &by_phone] {
            assert!(
                sql.starts_with(super::GUEST_SUMMARY_SELECT),
                "every filter must reuse the one projection"
            );
            assert!(sql.contains(super::GUEST_SUMMARY_TAIL));
        }

        assert_eq!(
            unfiltered,
            format!(
                "{}{}",
                super::GUEST_SUMMARY_SELECT,
                super::GUEST_SUMMARY_TAIL
            ),
            "no filter adds nothing at all"
        );
        assert!(by_phone.ends_with(" LIMIT 5"));
        assert!(!unfiltered.contains("LIMIT") && !searched.contains("LIMIT"));
    }

    #[tokio::test]
    async fn the_summary_aggregates_stays_and_lifetime_spend() {
        let pool = seeded_pool().await;

        let guests = load_guest_summaries(&pool, None).await.expect("load all");

        assert_eq!(guests.len(), 3);
        let g1 = guests.iter().find(|g| g.id == "G1").expect("G1");
        assert_eq!(g1.total_stays, 2);
        assert_eq!(g1.total_spent, 1_000_000, "600k + 400k");
        assert_eq!(g1.last_visit.as_deref(), Some("2026-04-08T14:00:00+07:00"));

        let g3 = guests.iter().find(|g| g.id == "G3").expect("G3");
        assert_eq!(
            g3.total_stays, 0,
            "the left join keeps guests with no stays"
        );
        assert_eq!(g3.total_spent, 0);
        assert_eq!(g3.last_visit, None);
    }

    #[tokio::test]
    async fn the_search_matches_the_name_or_the_document_number() {
        let pool = seeded_pool().await;

        let by_name = load_guest_summaries(&pool, Some("Tran"))
            .await
            .expect("by name");
        assert_eq!(
            by_name.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            vec!["G2"]
        );

        let by_doc = load_guest_summaries(&pool, Some("DOC-333"))
            .await
            .expect("by doc");
        assert_eq!(
            by_doc.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            vec!["G3"]
        );

        assert!(load_guest_summaries(&pool, Some("nobody"))
            .await
            .expect("no match")
            .is_empty());
    }

    #[tokio::test]
    async fn the_phone_lookup_is_a_substring_match_and_honours_its_limit() {
        let pool = seeded_pool().await;

        let matches = search_guest_summaries_by_phone(&pool, "0901", 5)
            .await
            .expect("by phone");
        let ids: Vec<&str> = matches.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["G1", "G3"], "newest visit first, G3 has none");

        let capped = search_guest_summaries_by_phone(&pool, "090", 1)
            .await
            .expect("capped");
        assert_eq!(capped.len(), 1);
    }

    #[tokio::test]
    async fn the_history_returns_the_guest_and_their_stays_newest_first() {
        let pool = seeded_pool().await;

        let guest = load_guest(&pool, "G1").await.expect("load guest");
        assert_eq!(guest.full_name, "Nguyen Van A");
        assert_eq!(guest.phone.as_deref(), Some("0901234567"));

        let bookings = load_guest_bookings(&pool, "G1").await.expect("load stays");
        let ids: Vec<&str> = bookings.iter().map(|b| b.booking_id.as_str()).collect();
        assert_eq!(ids, vec!["B2", "B1"]);
        assert_eq!(bookings[0].total_price, 400_000);

        assert!(load_guest_bookings(&pool, "G3")
            .await
            .expect("no stays")
            .is_empty());
    }

    #[tokio::test]
    async fn loading_an_unknown_guest_is_an_error_not_an_empty_guest() {
        let pool = seeded_pool().await;

        assert!(matches!(
            load_guest(&pool, "NOPE").await,
            Err(sqlx::Error::RowNotFound)
        ));
    }
}
