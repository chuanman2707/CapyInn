//! Reads behind the group screens.
//!
//! `load_group_detail` composes three reads and totals them. That composition
//! is a read, so it belongs here rather than in a service — there is no state
//! machine, only arithmetic over rows.

use sqlx::{Pool, Row, Sqlite};

use crate::db::row::{get_money_vnd, get_optional_money_vnd};
use crate::models::{BookingGroup, BookingWithGuest, GroupDetailResponse, GroupService};

// `b.status != 'voided'`: `void_booking_tx` (`void_lifecycle.rs`) hiện chặn
// xóa một booking có `group_id` ở tầng service ("Lượt này thuộc đoàn — chưa hỗ
// trợ xóa từng phòng"), nên hôm nay không đường ghi nào tạo ra được một hàng
// vừa `voided` vừa thuộc đoàn. Bộ lọc ở tầng đọc không được phép dựa vào đó:
// nó phải đứng vững bất kể `voided` sinh ra bằng đường nào — kể cả một sửa DB
// tay, hay một tính năng "xoá từng phòng trong đoàn" sau này — không chỉ
// đường duy nhất mà service hôm nay cho phép. Thiếu dòng này thì tổng tiền
// đoàn (`total_group_detail`) sẽ cộng nhầm ngay khi guard kia được nới.
const GROUP_BOOKINGS_SQL: &str =
    "SELECT b.id, b.room_id, r.name as room_name, g.full_name as guest_name,
            b.check_in_at, b.expected_checkout, b.actual_checkout, b.nights,
            b.total_price, b.paid_amount, b.status, b.source,
            b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout,
            b.guest_phone, b.guests, b.group_id
     FROM bookings b
     JOIN rooms r ON r.id = b.room_id
     JOIN guests g ON g.id = b.primary_guest_id
     WHERE b.group_id = ? AND b.status != 'voided'
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
pub(crate) fn total_group_detail(
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
        guests: row.get("guests"),
        group_id: row.get("group_id"),
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

#[cfg(test)]
mod tests {
    use super::total_group_detail;
    use crate::models::{BookingGroup, BookingWithGuest, GroupService};

    fn group() -> BookingGroup {
        BookingGroup {
            id: "GRP-1".to_string(),
            group_name: "Đoàn Hà Nội".to_string(),
            master_booking_id: None,
            organizer_name: "Nguyen Van A".to_string(),
            organizer_phone: Some("0901234567".to_string()),
            total_rooms: 2,
            status: "active".to_string(),
            notes: None,
            created_by: Some("user-1".to_string()),
            created_at: "2026-04-01T00:00:00+07:00".to_string(),
        }
    }

    fn booking(id: &str, total_price: i64, paid_amount: i64) -> BookingWithGuest {
        BookingWithGuest {
            id: id.to_string(),
            room_id: "101".to_string(),
            room_name: "Room 101".to_string(),
            guest_name: "Guest".to_string(),
            check_in_at: "2026-04-10T14:00:00+07:00".to_string(),
            expected_checkout: "2026-04-12T12:00:00+07:00".to_string(),
            actual_checkout: None,
            nights: 2,
            total_price,
            paid_amount,
            status: "active".to_string(),
            source: None,
            booking_type: Some("walk-in".to_string()),
            deposit_amount: None,
            scheduled_checkin: None,
            scheduled_checkout: None,
            guest_phone: None,
            guests: None,
            // `load_group_bookings` chỉ trả về hàng có `WHERE b.group_id = ?`,
            // nên mọi booking từ hàm này thực tế luôn thuộc một đoàn — khớp id
            // với fixture `group()` ở trên thay vì để trống cho đúng bản chất.
            group_id: Some("GRP-1".to_string()),
        }
    }

    fn service(id: &str, total_price: i64) -> GroupService {
        GroupService {
            id: id.to_string(),
            group_id: "GRP-1".to_string(),
            booking_id: None,
            name: "Giặt là".to_string(),
            quantity: 1,
            unit_price: total_price,
            total_price,
            note: None,
            created_by: Some("user-1".to_string()),
            created_at: "2026-04-11T09:00:00+07:00".to_string(),
        }
    }

    #[test]
    fn the_totals_add_rooms_and_services_separately_then_together() {
        let detail = total_group_detail(
            group(),
            vec![booking("B1", 600_000, 200_000), booking("B2", 400_000, 0)],
            vec![service("S1", 150_000), service("S2", 50_000)],
        );

        assert_eq!(detail.total_room_cost, 1_000_000);
        assert_eq!(detail.total_service_cost, 200_000);
        assert_eq!(detail.grand_total, 1_200_000);
    }

    #[test]
    fn paid_amount_counts_room_payments_only_never_services() {
        // The group invoice's balance_due is grand_total - paid_amount, so a
        // service payment counted here would under-bill the group. Services
        // carry no payment field at all; this pins that reading.
        let detail = total_group_detail(
            group(),
            vec![
                booking("B1", 600_000, 200_000),
                booking("B2", 400_000, 100_000),
            ],
            vec![service("S1", 900_000)],
        );

        assert_eq!(detail.paid_amount, 300_000, "200k + 100k, the rooms only");
        assert_eq!(
            detail.grand_total - detail.paid_amount,
            1_600_000,
            "the 900k service is owed, not paid"
        );
    }

    #[test]
    fn an_empty_group_totals_to_zero_rather_than_failing() {
        let detail = total_group_detail(group(), vec![], vec![]);

        assert_eq!(detail.total_room_cost, 0);
        assert_eq!(detail.total_service_cost, 0);
        assert_eq!(detail.grand_total, 0);
        assert_eq!(detail.paid_amount, 0);
    }

    #[test]
    fn the_rows_are_passed_through_untouched() {
        let detail = total_group_detail(
            group(),
            vec![booking("B1", 600_000, 0)],
            vec![service("S1", 150_000)],
        );

        assert_eq!(detail.group.id, "GRP-1");
        assert_eq!(detail.bookings.len(), 1);
        assert_eq!(detail.bookings[0].id, "B1");
        assert_eq!(detail.services.len(), 1);
        assert_eq!(detail.services[0].id, "S1");
    }
}
