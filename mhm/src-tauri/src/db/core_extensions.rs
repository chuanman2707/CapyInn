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

    execute_compat_alter(
        &mut tx,
        "ALTER TABLE invoices ADD COLUMN settlement_note TEXT",
    )
    .await?;

    set_schema_version(&mut tx, 25).await?;
    tx.commit().await?;
    Ok(())
}

/// Thời điểm gần nhất giá/đêm của booking bị lễ tân đổi tay qua
/// `set_booking_rate`. NULL nghĩa là booking chưa từng bị đổi giá thủ công —
/// tổng tiền của nó vẫn là giá được tính bình thường từ pricing engine.
///
/// Cho phép NULL, không có default: booking có sẵn trong DB trước migration
/// này chưa từng được đổi giá thủ công (tính năng đổi giá chưa tồn tại), nên
/// coi chúng là "chưa từng override" là đúng thực tế — không có default nào
/// khác an toàn hơn NULL ở đây.
pub(super) async fn migrate_v26_booking_rate_override(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN rate_overridden_at TEXT",
    )
    .await?;

    set_schema_version(&mut tx, 26).await?;
    tx.commit().await?;
    Ok(())
}

/// V27: sổ hội thoại của trợ lý quầy.
///
/// Bảng này **cố ý không có cột nào chứa `payload` hay `built_at_ms`** của
/// `ProposedAction`. Thẻ nhận phòng chỉ được lưu thành một hàng
/// `kind = 'action'` với `text` là bản tóm tắt người đọc được. Nhờ vậy không
/// tồn tại đường nào duyệt lại một thẻ cũ từ lịch sử: `approve()` cần đúng
/// `action.payload` để gọi `check_in`, mà database không giữ thứ đó. Đây là
/// bảo đảm cấu trúc, cùng tinh thần với luật "tool ghi có schema nhưng không
/// có executor". Đừng thêm cột JSON vào đây.
///
/// Nội dung lưu **nguyên văn, kể cả số CCCD**, và **không tự xoá** — lựa chọn
/// của chủ nhà, đã cân nhắc đánh đổi. Lối ra là hai lệnh xoá tay của admin.
///
/// Bảng này nằm **ngoài** phạm vi `RAW_PROMPT_RETENTION` / `RAW_RESPONSE_RETENTION`
/// trong `agent/retention.rs` — hai hằng số đó là cam kết của CEO Agent
/// (`agent/settings.rs:80-89`), không phải của trợ lý quầy. Đừng sửa chúng.
pub(super) async fn migrate_v27_assistant_conversations(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS assistant_conversations (
            id          TEXT PRIMARY KEY,
            user_id     TEXT NOT NULL,
            title       TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS assistant_messages (
            id              TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL
                            REFERENCES assistant_conversations(id) ON DELETE CASCADE,
            kind            TEXT NOT NULL,
            text            TEXT NOT NULL,
            created_at      TEXT NOT NULL
        )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_assistant_messages_conversation
         ON assistant_messages(conversation_id, created_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_assistant_conversations_user
         ON assistant_conversations(user_id, updated_at DESC)",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 27).await?;
    tx.commit().await?;
    Ok(())
}

/// V28: dấu vết xoá một lượt nhập sai.
///
/// Ba cột này chỉ có nghĩa khi `bookings.status = 'voided'`. Trạng thái mới —
/// chứ không phải cờ boolean — là chủ đích: mọi query doanh thu đang lọc
/// `status IN ('active','checked_out')`, nên một giá trị lạ tự động bị loại mà
/// không phải sửa từng câu SQL.
pub(super) async fn migrate_v28_booking_void(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN voided_at TEXT").await?;
    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN voided_by TEXT").await?;
    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN void_reason TEXT").await?;

    set_schema_version(&mut tx, 28).await?;
    tx.commit().await?;
    Ok(())
}

/// V29: bỏ trạng thái phòng `cleaning`.
///
/// Housekeeping được gỡ khỏi sản phẩm ngày 09/08/2026: trả phòng xong là phòng
/// trống ngay, không còn màn hình nào đưa phòng ra khỏi `cleaning` nữa. Một cơ
/// sở dữ liệu đang chạy có thể có phòng vừa trả nằm ở đúng trạng thái đó —
/// không có migration này thì phòng ấy kẹt vĩnh viễn ở một giá trị mà cả
/// `check_in_tx` lẫn giao diện đều không xử lý được.
///
/// Chuỗi SQL viết trần, KHÔNG dùng `status::room::CLEANING`: hằng đó bị xoá
/// cùng đợt này, và một migration đã chạy phải biên dịch được mãi mãi về sau.
///
/// Bảng `housekeeping` giữ nguyên. Nó là lịch sử, không phải trạng thái.
pub(super) async fn migrate_v29_drop_cleaning_room_status(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE rooms SET status = 'vacant' WHERE status = 'cleaning'")
        .execute(&mut *tx)
        .await?;

    set_schema_version(&mut tx, 29).await?;
    tx.commit().await?;
    Ok(())
}
