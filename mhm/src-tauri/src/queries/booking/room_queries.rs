use crate::db::row::{get_money_vnd, get_optional_money_vnd};
use crate::models::{
    AvailabilityResult, Booking, CalendarConflict, CalendarEntry, Guest, Room,
    RoomWithAvailability, RoomWithBooking, UpcomingReservation,
};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite};

pub async fn load_rooms(pool: &Pool<Sqlite>) -> Result<Vec<Room>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms ORDER BY floor, id"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(map_room).collect())
}

pub async fn load_room_detail(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> Result<RoomWithBooking, sqlx::Error> {
    let row = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms WHERE id = ?")
        .bind(room_id)
        .fetch_one(pool)
        .await?;

    let room = map_room(&row);

    let booking_row = sqlx::query(
        "SELECT id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout, nights, total_price, paid_amount, status, source, notes, created_at, rate_overridden_at, group_id
         FROM bookings WHERE room_id = ? AND status = 'active' LIMIT 1"
    )
    .bind(room_id)
    .fetch_optional(pool)
    .await?;

    // `group_id` không thuộc `Booking` dùng chung (xem models.rs) nên phải đọc
    // ra khỏi hàng thô ở đây, trước khi `map_booking` bỏ qua cột này.
    let group_id: Option<String> = booking_row
        .as_ref()
        .and_then(|r| r.get::<Option<String>, _>("group_id"));
    let booking = booking_row.map(|r| map_booking(&r));

    let guests = if let Some(ref b) = booking {
        let rows = sqlx::query(
            "SELECT g.* FROM guests g
             JOIN booking_guests bg ON bg.guest_id = g.id
             WHERE bg.booking_id = ?",
        )
        .bind(&b.id)
        .fetch_all(pool)
        .await?;

        rows.iter().map(map_guest).collect()
    } else {
        vec![]
    };

    Ok(RoomWithBooking {
        room,
        booking,
        guests,
        group_id,
    })
}

pub async fn check_room_availability(
    pool: &Pool<Sqlite>,
    room_id: &str,
    from_date: &str,
    to_date: &str,
) -> Result<AvailabilityResult, String> {
    let rows = sqlx::query(
        "SELECT rc.date, rc.status, rc.booking_id, COALESCE(g.full_name, '') as guest_name
         FROM room_calendar rc
         LEFT JOIN bookings b ON b.id = rc.booking_id
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE rc.room_id = ? AND rc.date >= ? AND rc.date < ?
         ORDER BY rc.date ASC",
    )
    .bind(room_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(AvailabilityResult {
            available: true,
            conflicts: vec![],
            max_nights: None,
        });
    }

    let conflicts: Vec<CalendarConflict> = rows
        .iter()
        .map(|r| CalendarConflict {
            date: r.get("date"),
            status: r.get("status"),
            guest_name: r.get("guest_name"),
            booking_id: r.get("booking_id"),
        })
        .collect();

    let first_date = &conflicts[0].date;
    let from_naive =
        chrono::NaiveDate::parse_from_str(from_date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let first_naive =
        chrono::NaiveDate::parse_from_str(first_date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let max_nights = (first_naive - from_naive).num_days() as i32;

    Ok(AvailabilityResult {
        available: false,
        conflicts,
        max_nights: Some(max_nights),
    })
}

pub async fn load_room_calendar(
    pool: &Pool<Sqlite>,
    room_id: &str,
    from_date: &str,
    to_date: &str,
) -> Result<Vec<CalendarEntry>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT room_id, date, booking_id, status FROM room_calendar
         WHERE room_id = ? AND date >= ? AND date <= ?
         ORDER BY date ASC",
    )
    .bind(room_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| CalendarEntry {
            room_id: r.get("room_id"),
            date: r.get("date"),
            booking_id: r.get("booking_id"),
            status: r.get("status"),
        })
        .collect())
}

pub async fn load_rooms_availability(
    pool: &Pool<Sqlite>,
) -> Result<Vec<RoomWithAvailability>, sqlx::Error> {
    let room_rows = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms ORDER BY id")
        .fetch_all(pool)
        .await?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut results = Vec::new();

    for rr in &room_rows {
        let room = map_room(rr);

        let current_booking =
            sqlx::query("SELECT * FROM bookings WHERE room_id = ? AND status = 'active' LIMIT 1")
                .bind(&room.id)
                .fetch_optional(pool)
                .await?
                .map(|r| map_booking(&r));

        let res_rows = sqlx::query(
            "SELECT b.id, g.full_name, b.scheduled_checkin, b.scheduled_checkout, b.deposit_amount, b.status
             FROM bookings b
             JOIN guests g ON g.id = b.primary_guest_id
             WHERE b.room_id = ? AND b.status = 'booked' AND b.scheduled_checkin >= ?
             ORDER BY b.scheduled_checkin ASC"
        )
        .bind(&room.id)
        .bind(&today)
        .fetch_all(pool)
        .await?;

        let upcoming: Vec<UpcomingReservation> = res_rows
            .iter()
            .map(|r| UpcomingReservation {
                booking_id: r.get("id"),
                guest_name: r.get("full_name"),
                scheduled_checkin: r
                    .get::<Option<String>, _>("scheduled_checkin")
                    .unwrap_or_default(),
                scheduled_checkout: r
                    .get::<Option<String>, _>("scheduled_checkout")
                    .unwrap_or_default(),
                deposit_amount: get_optional_money_vnd(r, "deposit_amount").unwrap_or(0),
                status: r.get("status"),
            })
            .collect();

        let next_until = upcoming.first().map(|u| u.scheduled_checkin.clone());

        results.push(RoomWithAvailability {
            room,
            current_booking,
            upcoming_reservations: upcoming,
            next_available_until: next_until,
        });
    }

    Ok(results)
}

pub(crate) fn map_room(row: &SqliteRow) -> Room {
    Room {
        id: row.get("id"),
        name: row.get("name"),
        room_type: row.get("type"),
        floor: row.get("floor"),
        has_balcony: row.get::<i32, _>("has_balcony") == 1,
        base_price: get_money_vnd(row, "base_price"),
        max_guests: row.try_get::<i32, _>("max_guests").unwrap_or(2),
        extra_person_fee: get_money_vnd(row, "extra_person_fee"),
        status: row.get("status"),
    }
}

fn map_booking(row: &SqliteRow) -> Booking {
    Booking {
        id: row.get("id"),
        room_id: row.get("room_id"),
        primary_guest_id: row.get("primary_guest_id"),
        check_in_at: row.get("check_in_at"),
        expected_checkout: row.get("expected_checkout"),
        actual_checkout: row.get("actual_checkout"),
        nights: row.get("nights"),
        total_price: get_money_vnd(row, "total_price"),
        paid_amount: get_money_vnd(row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        rate_overridden_at: row.get("rate_overridden_at"),
    }
}

fn map_guest(row: &SqliteRow) -> Guest {
    Guest {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    async fn migrated_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connects to in-memory sqlite");

        crate::db::run_migrations(&pool)
            .await
            .expect("runs migrations");

        pool
    }

    async fn insert_room(pool: &Pool<Sqlite>, id: &str, floor: i32, status: &str) {
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests,
                extra_person_fee, status
            ) VALUES (?, ?, 'standard', ?, 0, 100000, 2, 25000, ?)",
        )
        .bind(id)
        .bind(format!("Room {id}"))
        .bind(floor)
        .bind(status)
        .execute(pool)
        .await
        .expect("inserts room");
    }

    async fn insert_guest(pool: &Pool<Sqlite>, id: &str, name: &str) {
        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, dob, gender, nationality,
                address, visa_expiry, scan_path, phone, created_at
            ) VALUES (?, 'domestic', ?, ?, NULL, NULL, 'VN', NULL, NULL, NULL, NULL, '2026-05-20T00:00:00Z')",
        )
        .bind(id)
        .bind(name)
        .bind(format!("DOC-{id}"))
        .execute(pool)
        .await
        .expect("inserts guest");
    }

    struct BookingFixture<'a> {
        id: &'a str,
        room_id: &'a str,
        guest_id: &'a str,
        status: &'a str,
        scheduled_checkin: &'a str,
        scheduled_checkout: &'a str,
        deposit_amount: i64,
    }

    async fn insert_booking(pool: &Pool<Sqlite>, booking: BookingFixture<'_>) {
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status, source,
                notes, created_at, booking_type, deposit_amount, guest_phone,
                scheduled_checkin, scheduled_checkout
            ) VALUES (?, ?, ?, ?, ?, NULL, 2, 200000, 0, ?, 'direct', NULL,
                '2026-05-20T00:00:00Z', 'reservation', ?, NULL, ?, ?)",
        )
        .bind(booking.id)
        .bind(booking.room_id)
        .bind(booking.guest_id)
        .bind(format!("{}T14:00:00Z", booking.scheduled_checkin))
        .bind(format!("{}T12:00:00Z", booking.scheduled_checkout))
        .bind(booking.status)
        .bind(booking.deposit_amount)
        .bind(booking.scheduled_checkin)
        .bind(booking.scheduled_checkout)
        .execute(pool)
        .await
        .expect("inserts booking");
    }

    async fn insert_calendar_entry(
        pool: &Pool<Sqlite>,
        room_id: &str,
        date: &str,
        booking_id: &str,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status)
             VALUES (?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(date)
        .bind(booking_id)
        .bind(status)
        .execute(pool)
        .await
        .expect("inserts calendar entry");
    }

    #[tokio::test]
    async fn load_rooms_orders_by_floor_then_id() {
        let pool = migrated_pool().await;
        insert_room(&pool, "203", 2, "vacant").await;
        insert_room(&pool, "101", 1, "occupied").await;
        insert_room(&pool, "102", 1, "vacant").await;

        let rooms = load_rooms(&pool).await.expect("loads rooms");

        let ids: Vec<_> = rooms.iter().map(|room| room.id.as_str()).collect();
        assert_eq!(ids, vec!["101", "102", "203"]);
    }

    #[tokio::test]
    async fn load_room_detail_returns_active_booking_and_guests() {
        let pool = migrated_pool().await;
        insert_room(&pool, "101", 1, "occupied").await;
        insert_guest(&pool, "guest-1", "Mai Nguyen").await;
        insert_guest(&pool, "guest-2", "An Tran").await;
        insert_booking(
            &pool,
            BookingFixture {
                id: "booking-1",
                room_id: "101",
                guest_id: "guest-1",
                status: "active",
                scheduled_checkin: "2026-05-20",
                scheduled_checkout: "2026-05-22",
                deposit_amount: 0,
            },
        )
        .await;
        sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?), (?, ?)")
            .bind("booking-1")
            .bind("guest-1")
            .bind("booking-1")
            .bind("guest-2")
            .execute(&pool)
            .await
            .expect("inserts booking guests");

        let detail = load_room_detail(&pool, "101")
            .await
            .expect("loads room detail");

        assert_eq!(detail.room.id, "101");
        assert_eq!(
            detail.booking.as_ref().map(|booking| booking.id.as_str()),
            Some("booking-1")
        );
        let mut guest_names: Vec<_> = detail
            .guests
            .iter()
            .map(|guest| guest.full_name.as_str())
            .collect();
        guest_names.sort_unstable();
        assert_eq!(guest_names, vec!["An Tran", "Mai Nguyen"]);
        assert_eq!(detail.group_id, None);
    }

    // RoomDrawer (Task 12) mirrors BookingDetailPopup's (Task 11) group gate —
    // xem RoomDrawer.tsx — và cần đúng `group_id` này để tự khoá nút xóa
    // TRƯỚC khi bấm, thay vì đợi preview trả lời rồi mới biết bị chặn. Thiếu
    // cột `group_id` trong SELECT của `load_room_detail` thì trường luôn ra
    // `None` dù booking thật sự thuộc đoàn — im lặng, không lỗi biên dịch.
    #[tokio::test]
    async fn load_room_detail_returns_the_active_bookings_group_id() {
        let pool = migrated_pool().await;
        insert_room(&pool, "101", 1, "occupied").await;
        insert_guest(&pool, "guest-1", "Mai Nguyen").await;
        insert_booking(
            &pool,
            BookingFixture {
                id: "booking-1",
                room_id: "101",
                guest_id: "guest-1",
                status: "active",
                scheduled_checkin: "2026-05-20",
                scheduled_checkout: "2026-05-22",
                deposit_amount: 0,
            },
        )
        .await;
        sqlx::query(
            "INSERT INTO booking_groups (id, group_name, master_booking_id, organizer_name,
                                          total_rooms, status, created_at)
             VALUES ('GRP-1', 'Đoàn thử', 'booking-1', 'Trưởng đoàn', 1, 'active',
                     '2026-05-20T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("inserts booking group");
        sqlx::query("UPDATE bookings SET group_id = 'GRP-1' WHERE id = 'booking-1'")
            .execute(&pool)
            .await
            .expect("links booking to group");

        let detail = load_room_detail(&pool, "101")
            .await
            .expect("loads room detail");

        assert_eq!(detail.group_id.as_deref(), Some("GRP-1"));
    }

    #[tokio::test]
    async fn check_room_availability_reports_first_conflict_max_nights() {
        let pool = migrated_pool().await;
        insert_room(&pool, "101", 1, "booked").await;
        insert_guest(&pool, "guest-1", "Mai Nguyen").await;
        insert_booking(
            &pool,
            BookingFixture {
                id: "booking-1",
                room_id: "101",
                guest_id: "guest-1",
                status: "booked",
                scheduled_checkin: "2026-05-22",
                scheduled_checkout: "2026-05-24",
                deposit_amount: 50000,
            },
        )
        .await;
        insert_calendar_entry(&pool, "101", "2026-05-22", "booking-1", "booked").await;
        insert_calendar_entry(&pool, "101", "2026-05-23", "booking-1", "booked").await;

        let result = check_room_availability(&pool, "101", "2026-05-20", "2026-05-25")
            .await
            .expect("checks availability");

        assert!(!result.available);
        assert_eq!(result.max_nights, Some(2));
        assert_eq!(result.conflicts.len(), 2);
        assert_eq!(result.conflicts[0].date, "2026-05-22");
        assert_eq!(
            result.conflicts[0].guest_name.as_deref(),
            Some("Mai Nguyen")
        );
    }

    #[tokio::test]
    async fn load_room_calendar_includes_to_date() {
        let pool = migrated_pool().await;
        insert_room(&pool, "101", 1, "booked").await;
        insert_guest(&pool, "guest-1", "Mai Nguyen").await;
        insert_booking(
            &pool,
            BookingFixture {
                id: "booking-1",
                room_id: "101",
                guest_id: "guest-1",
                status: "booked",
                scheduled_checkin: "2026-05-20",
                scheduled_checkout: "2026-05-22",
                deposit_amount: 50000,
            },
        )
        .await;
        insert_calendar_entry(&pool, "101", "2026-05-20", "booking-1", "booked").await;
        insert_calendar_entry(&pool, "101", "2026-05-21", "booking-1", "booked").await;
        insert_calendar_entry(&pool, "101", "2026-05-22", "booking-1", "booked").await;

        let entries = load_room_calendar(&pool, "101", "2026-05-20", "2026-05-22")
            .await
            .expect("loads room calendar");

        let dates: Vec<_> = entries.iter().map(|entry| entry.date.as_str()).collect();
        assert_eq!(dates, vec!["2026-05-20", "2026-05-21", "2026-05-22"]);
    }

    #[tokio::test]
    async fn load_rooms_availability_returns_active_booking_and_upcoming_reservations() {
        let pool = migrated_pool().await;
        let today = chrono::Local::now().date_naive();
        let active_checkin = today.format("%Y-%m-%d").to_string();
        let active_checkout = (today + chrono::Duration::days(2))
            .format("%Y-%m-%d")
            .to_string();
        let upcoming_checkin = (today + chrono::Duration::days(4))
            .format("%Y-%m-%d")
            .to_string();
        let upcoming_checkout = (today + chrono::Duration::days(6))
            .format("%Y-%m-%d")
            .to_string();

        insert_room(&pool, "101", 1, "occupied").await;
        insert_guest(&pool, "guest-active", "Mai Nguyen").await;
        insert_guest(&pool, "guest-upcoming", "An Tran").await;
        insert_booking(
            &pool,
            BookingFixture {
                id: "active-booking",
                room_id: "101",
                guest_id: "guest-active",
                status: "active",
                scheduled_checkin: &active_checkin,
                scheduled_checkout: &active_checkout,
                deposit_amount: 0,
            },
        )
        .await;
        insert_booking(
            &pool,
            BookingFixture {
                id: "upcoming-booking",
                room_id: "101",
                guest_id: "guest-upcoming",
                status: "booked",
                scheduled_checkin: &upcoming_checkin,
                scheduled_checkout: &upcoming_checkout,
                deposit_amount: 75000,
            },
        )
        .await;

        let rooms = load_rooms_availability(&pool)
            .await
            .expect("loads rooms availability");

        assert_eq!(rooms.len(), 1);
        let availability = &rooms[0];
        assert_eq!(availability.room.id, "101");
        assert_eq!(
            availability
                .current_booking
                .as_ref()
                .map(|booking| booking.id.as_str()),
            Some("active-booking")
        );
        assert_eq!(availability.upcoming_reservations.len(), 1);
        assert_eq!(
            availability.upcoming_reservations[0].booking_id,
            "upcoming-booking"
        );
        assert_eq!(
            availability.next_available_until.as_deref(),
            Some(upcoming_checkin.as_str())
        );
    }
}
