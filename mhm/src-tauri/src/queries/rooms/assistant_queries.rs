use serde::Serialize;
use sqlx::{Pool, Sqlite};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssistantRoomStatus {
    pub room_id: String,
    pub room_name: String,
    pub room_type: String,
    pub status: String,
    pub booking_id: Option<String>,
    pub guest_name: Option<String>,
    pub expected_checkout: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssistantFreeRoom {
    pub room_id: String,
    pub room_name: String,
    pub room_type: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssistantGuestStay {
    pub guest_id: String,
    pub guest_name: String,
    pub phone: Option<String>,
    pub booking_id: Option<String>,
    pub room_id: Option<String>,
    pub room_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssistantStayCharges {
    pub booking_id: String,
    pub guest_name: String,
    pub room_name: String,
    pub total_price_vnd: i64,
    pub paid_amount_vnd: i64,
}

/// Trạng thái mọi phòng ngay lúc này, kèm khách đang ở nếu có.
pub async fn load_room_status_now(
    pool: &Pool<Sqlite>,
) -> Result<Vec<AssistantRoomStatus>, sqlx::Error> {
    sqlx::query_as::<_, AssistantRoomStatus>(
        "SELECT r.id           AS room_id,
                r.name         AS room_name,
                r.type         AS room_type,
                r.status       AS status,
                b.id           AS booking_id,
                g.full_name    AS guest_name,
                b.expected_checkout AS expected_checkout
           FROM rooms r
           LEFT JOIN bookings b
             ON b.room_id = r.id AND b.status = 'active'
           LEFT JOIN guests g
             ON g.id = b.primary_guest_id
          ORDER BY r.name",
    )
    .fetch_all(pool)
    .await
}

/// Phòng không vướng lịch nào trong khoảng [from, to). Dùng `room_calendar`,
/// đúng bảng mà `room_queries::check_room_availability` đang dựa vào.
pub async fn load_free_rooms_between(
    pool: &Pool<Sqlite>,
    from_date: &str,
    to_date: &str,
) -> Result<Vec<AssistantFreeRoom>, sqlx::Error> {
    sqlx::query_as::<_, AssistantFreeRoom>(
        "SELECT r.id AS room_id, r.name AS room_name, r.type AS room_type
           FROM rooms r
          WHERE NOT EXISTS (
                SELECT 1 FROM room_calendar rc
                 WHERE rc.room_id = r.id
                   AND rc.date >= ?
                   AND rc.date < ?
                   AND rc.booking_id IS NOT NULL
          )
          ORDER BY r.name",
    )
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await
}

/// Tìm khách theo tên hoặc số điện thoại, kèm lượt ở đang mở nếu có.
pub async fn find_guest_stays(
    pool: &Pool<Sqlite>,
    term: &str,
) -> Result<Vec<AssistantGuestStay>, sqlx::Error> {
    let pattern = format!("%{term}%");

    sqlx::query_as::<_, AssistantGuestStay>(
        "SELECT g.id        AS guest_id,
                g.full_name AS guest_name,
                g.phone     AS phone,
                b.id        AS booking_id,
                b.room_id   AS room_id,
                r.name      AS room_name
           FROM guests g
           LEFT JOIN bookings b
             ON b.primary_guest_id = g.id AND b.status = 'active'
           LEFT JOIN rooms r
             ON r.id = b.room_id
          WHERE g.full_name LIKE ? OR IFNULL(g.phone, '') LIKE ?
          ORDER BY (b.id IS NULL), g.full_name
          LIMIT 20",
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(pool)
    .await
}

/// Tổng tiền và đã trả của một lượt ở. `None` nghĩa là không có booking nào
/// mang mã đó — khác hẳn với "có booking nhưng chưa phát sinh khoản nào".
pub async fn load_stay_charges(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<Option<AssistantStayCharges>, sqlx::Error> {
    sqlx::query_as::<_, AssistantStayCharges>(
        "SELECT b.id                       AS booking_id,
                g.full_name                AS guest_name,
                r.name                     AS room_name,
                b.total_price              AS total_price_vnd,
                IFNULL(b.paid_amount, 0)   AS paid_amount_vnd
           FROM bookings b
           JOIN guests g ON g.id = b.primary_guest_id
           JOIN rooms  r ON r.id = b.room_id
          WHERE b.id = ?",
    )
    .bind(booking_id)
    .fetch_optional(pool)
    .await
}
