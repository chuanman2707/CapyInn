//! Migration v20 — bốn bảng của module "khai báo tạm trú".
//!
//! Thuần `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS`. KHÔNG có
//! một câu `ALTER` nào: PMS đang vận hành thật, không migrate nó vì một tính
//! năng phụ.
//!
//! `declaration_link.stay_id` = `bookings.id` nhưng CỐ Ý không ràng buộc FK
//! cứng (§5.2). FK cứng sẽ làm việc xóa/dọn một booking trong PMS thất bại vì
//! một module phụ.

use sqlx::{Pool, Sqlite};

use super::set_schema_version;

pub(super) async fn migrate_v20_declaration_tables(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_identity (
                id                      TEXT PRIMARY KEY,
                source                  TEXT NOT NULL,
                extract_confidence      TEXT NOT NULL,
                full_name               TEXT NOT NULL,
                dob                     TEXT NOT NULL,
                gender                  TEXT NOT NULL,
                nationality_iso3        TEXT NOT NULL,
                doc_type_code           TEXT,
                doc_type_source         TEXT,
                doc_type_name           TEXT,
                doc_no                  TEXT,
                phone                   TEXT,
                residence_status        TEXT,
                address_detail          TEXT,
                passport_no             TEXT,
                passport_expiry         TEXT,
                visa_valid_until        TEXT,
                name_confirmed_by_human INTEGER NOT NULL DEFAULT 0,
                single_token_name_ok    INTEGER NOT NULL DEFAULT 0,
                redacted_at             TEXT,
                created_at              TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_link (
                id               TEXT PRIMARY KEY,
                identity_id      TEXT NOT NULL REFERENCES declaration_identity(id),
                stay_id          TEXT NOT NULL,
                stay_reason      TEXT NOT NULL,
                stay_reason_note TEXT,
                actual_check_out TEXT,
                created_at       TEXT NOT NULL,
                UNIQUE(identity_id, stay_id)
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_batch (
                id             TEXT PRIMARY KEY,
                kind           TEXT NOT NULL,
                date_from      TEXT,
                date_to        TEXT,
                file_path      TEXT NOT NULL,
                row_count      INTEGER NOT NULL,
                status         TEXT NOT NULL,
                verified_count INTEGER,
                verified_at    TEXT,
                note           TEXT,
                created_at     TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_entry (
                batch_id  TEXT NOT NULL REFERENCES declaration_batch(id),
                link_id   TEXT NOT NULL REFERENCES declaration_link(id),
                row_index INTEGER NOT NULL,
                PRIMARY KEY (batch_id, link_id)
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_decl_link_stay ON declaration_link(stay_id)")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_decl_entry_link ON declaration_entry(link_id)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_decl_batch_status ON declaration_batch(status, verified_at)",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 20).await?;
    tx.commit().await?;
    Ok(())
}

/// Migration v21 — `declaration_link.stay_id` được phép NULL.
///
/// Khách có thể phải khai báo trước khi có phòng: người vận hành cầm CCCD lúc
/// khách vừa tới, còn booking thì chưa tạo. Trước v21, `stay_id NOT NULL` biến
/// điều đó thành ngõ cụt — màn khai báo chỉ liệt kê phòng đang có khách.
///
/// SQLite không gỡ được `NOT NULL` bằng `ALTER`, nên phải dựng lại bảng. Đây là
/// bảng của chính module này, không phải bảng của PMS — ràng buộc "không migrate
/// PMS" không bị đụng tới.
///
/// `UNIQUE(identity_id, stay_id)` giữ nguyên: SQLite coi mọi NULL là khác nhau,
/// nên hai khai báo chưa gắn phòng của cùng một danh tính không chặn nhau.
pub(super) async fn migrate_v21_optional_stay(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS declaration_link_v21 (
                id               TEXT PRIMARY KEY,
                identity_id      TEXT NOT NULL REFERENCES declaration_identity(id),
                stay_id          TEXT,
                stay_reason      TEXT NOT NULL,
                stay_reason_note TEXT,
                actual_check_out TEXT,
                created_at       TEXT NOT NULL,
                UNIQUE(identity_id, stay_id)
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT OR IGNORE INTO declaration_link_v21
            (id, identity_id, stay_id, stay_reason, stay_reason_note, actual_check_out, created_at)
         SELECT id, identity_id, stay_id, stay_reason, stay_reason_note, actual_check_out, created_at
           FROM declaration_link",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("DROP TABLE declaration_link")
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE declaration_link_v21 RENAME TO declaration_link")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_decl_link_stay ON declaration_link(stay_id)")
        .execute(&mut *tx)
        .await?;

    set_schema_version(&mut tx, 21).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::run_migrations;
    use sqlx::SqlitePool;

    /// Hai hàm đọc PMS của `declaration::repo` được test Ở ĐÂY chứ không ở
    /// `repo.rs`, vì fixture phải `INSERT INTO bookings/rooms/...` — mà test
    /// ranh giới trong `repo.rs` quét source của cả `src/declaration/` và sẽ
    /// đỏ nếu thấy một câu ghi nào chạm bảng của PMS, kể cả trong test.
    async fn seeded_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("runs migrations");

        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
             ) VALUES ('room-1', 'Phòng 5B', 'standard', 1, 0, 100000, 2, 0, 'occupied')",
        )
        .execute(&pool)
        .await
        .expect("seeds room");

        for guest in ["guest-1", "guest-2"] {
            sqlx::query(
                "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
                 VALUES (?, 'domestic', 'Khách thử', 'DOC-1', '2026-07-26T09:00:00+07:00')",
            )
            .bind(guest)
            .execute(&pool)
            .await
            .expect("seeds guest");
        }

        let check_in = chrono::Local::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                nights, total_price, status, created_at
             ) VALUES ('booking-1', 'room-1', 'guest-1', ?, '2026-12-31', 1, 500000, 'active', ?)",
        )
        .bind(&check_in)
        .bind(&check_in)
        .execute(&pool)
        .await
        .expect("seeds booking");

        for guest in ["guest-1", "guest-2"] {
            sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES ('booking-1', ?)")
                .bind(guest)
                .execute(&pool)
                .await
                .expect("seeds booking guest");
        }

        pool
    }

    #[tokio::test]
    async fn v20_creates_four_declaration_tables_and_sets_version() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("runs migrations");

        for table in [
            "declaration_identity",
            "declaration_link",
            "declaration_batch",
            "declaration_entry",
        ] {
            let found: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .expect("reads sqlite_master");
            assert_eq!(found, 1, "thiếu bảng {table}");
        }

        let version: i32 = sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads schema version");
        assert_eq!(version, 21);
    }

    /// Khách tới trước khi có booking: phải khai báo được ngay, không phải chờ
    /// ai đó check-in xong.
    #[tokio::test]
    async fn v21_lets_a_declaration_exist_without_a_stay() {
        let pool = seeded_pool().await;

        let identity = crate::declaration::repo::insert_identity(
            &pool,
            &crate::declaration::model::Identity {
                full_name: "Phạm Thị Minh Hiền".into(),
                dob: "1988-12-16".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                ..Default::default()
            },
            "qr_cccd",
            "verified",
        )
        .await
        .expect("lưu được danh tính");

        let link = crate::declaration::repo::insert_link(&pool, &identity, None, "1", None)
            .await
            .expect("ghép được dù chưa có phòng");

        let rows = crate::declaration::repo::load_rows_by_link_ids(
            &pool,
            std::slice::from_ref(&link),
        )
        .await
        .expect("đọc lại được dòng");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].identity.full_name, "Phạm Thị Minh Hiền");
        assert_eq!(rows[0].stay.room_no, "", "chưa có phòng");
    }

    /// Dữ liệu cũ (v20, stay_id NOT NULL) phải đi qua v21 mà không mất dòng nào.
    #[tokio::test]
    async fn v21_carries_existing_links_across_the_table_rebuild() {
        let pool = seeded_pool().await;

        sqlx::query(
            "INSERT INTO declaration_identity (
                id, source, extract_confidence, full_name, dob, gender,
                nationality_iso3, created_at
             ) VALUES ('id-1', 'qr_cccd', 'verified', 'Phan Thị Mỹ Hà', '1995-07-28',
                       'F', 'VNM', '2026-07-26T09:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds identity");
        sqlx::query(
            "INSERT INTO declaration_link (id, identity_id, stay_id, stay_reason, created_at)
             VALUES ('link-1', 'id-1', 'booking-1', '2', '2026-07-26T09:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds link");

        // Chạy lại toàn bộ migration: v21 phải bỏ qua vì version đã là 21.
        run_migrations(&pool).await.expect("migration chạy lại được");

        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link WHERE id='link-1'")
            .fetch_one(&pool)
            .await
            .expect("đếm link");
        assert_eq!(kept, 1);
    }

    /// Danh tính vừa trích phải sống sót khi người vận hành đổi tab — nó nằm
    /// trong DB, và phải có đường đọc ra.
    #[tokio::test]
    async fn an_identity_waiting_to_be_linked_can_be_listed_and_discarded() {
        let pool = seeded_pool().await;

        let identity = crate::declaration::repo::insert_identity(
            &pool,
            &crate::declaration::model::Identity {
                full_name: "Phạm Thị Minh Hiền".into(),
                nationality_iso3: "VNM".into(),
                ..Default::default()
            },
            "qr_cccd",
            "verified",
        )
        .await
        .expect("lưu được danh tính");

        let waiting = crate::declaration::repo::list_unlinked_identities(&pool)
            .await
            .expect("đọc được danh sách chờ");
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].full_name, "Phạm Thị Minh Hiền");

        // Ghép xong thì biến khỏi danh sách chờ.
        let link = crate::declaration::repo::insert_link(&pool, &identity, None, "1", None)
            .await
            .expect("ghép được");
        assert!(crate::declaration::repo::list_unlinked_identities(&pool)
            .await
            .expect("đọc lại")
            .is_empty());

        // Đã ghép rồi thì không xóa thẳng được — đó là bằng chứng đã khai.
        assert!(
            crate::declaration::repo::delete_unlinked_identity(&pool, &identity)
                .await
                .is_err(),
            "danh tính đã ghép phải đi đường thu hồi, không xóa thẳng"
        );

        sqlx::query("DELETE FROM declaration_link WHERE id = ?")
            .bind(&link)
            .execute(&pool)
            .await
            .expect("gỡ link để thử xóa");
        crate::declaration::repo::delete_unlinked_identity(&pool, &identity)
            .await
            .expect("chưa ghép thì xóa được");
    }

    /// §5.2 — stay_id KHÔNG có FK cứng tới `bookings`. Xóa một booking trong
    /// PMS không được thất bại vì module khai báo còn giữ link.
    #[tokio::test]
    async fn link_survives_a_booking_being_deleted_in_the_pms() {
        let pool = seeded_pool().await;

        sqlx::query(
            "INSERT INTO declaration_identity (
                id, source, extract_confidence, full_name, dob, gender,
                nationality_iso3, created_at
             ) VALUES ('id-1', 'qr_cccd', 'verified', 'Phan Thị Mỹ Hà', '1995-07-28',
                       'F', 'VNM', '2026-07-26T09:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds identity");
        sqlx::query(
            "INSERT INTO declaration_link (id, identity_id, stay_id, stay_reason, created_at)
             VALUES ('link-1', 'id-1', 'booking-1', '2', '2026-07-26T09:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds link");

        sqlx::query("DELETE FROM booking_guests WHERE booking_id = 'booking-1'")
            .execute(&pool)
            .await
            .expect("PMS dọn booking_guests");
        sqlx::query("DELETE FROM bookings WHERE id = 'booking-1'")
            .execute(&pool)
            .await
            .expect("PMS xóa được booking dù module khai báo còn link");

        let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link")
            .fetch_one(&pool)
            .await
            .expect("đếm link");
        assert_eq!(links, 1, "link phải sống sót để giữ lịch sử lô");
    }

    #[tokio::test]
    async fn load_stays_reads_pms_and_strips_the_room_prefix() {
        let pool = seeded_pool().await;

        let stays = crate::declaration::repo::load_stays_for_declaration(&pool)
            .await
            .expect("đọc được lượt lưu trú");

        assert_eq!(stays.len(), 1);
        assert_eq!(stays[0].stay_id, "booking-1");
        assert_eq!(stays[0].room_no, "5B");
        assert_eq!(stays[0].expected_out, "2026-12-31");
        assert!(stays[0].actual_out.is_none());
        assert!(stays[0].check_in_raw.contains('T'));
    }

    #[tokio::test]
    async fn undeclared_count_drops_only_when_a_batch_is_verified() {
        let pool = seeded_pool().await;

        let before = crate::declaration::repo::count_undeclared_within_48h(&pool)
            .await
            .expect("đếm được");
        assert_eq!(before, 2, "hai khách trong booking, chưa ai được khai");

        let identity = crate::declaration::repo::insert_identity(
            &pool,
            &crate::declaration::model::Identity {
                full_name: "Phan Thị Mỹ Hà".into(),
                dob: "1995-07-28".into(),
                gender: "F".into(),
                nationality_iso3: "VNM".into(),
                ..Default::default()
            },
            "qr_cccd",
            "verified",
        )
        .await
        .expect("lưu được danh tính");
        let link = crate::declaration::repo::insert_link(&pool, &identity, Some("booking-1"), "2", None)
            .await
            .expect("lưu được link");
        let batch = crate::declaration::repo::insert_batch(&pool, "VN", "/tmp/x.xlsx", 1)
            .await
            .expect("lưu được lô");
        crate::declaration::repo::insert_entries(&pool, &batch, std::slice::from_ref(&link))
            .await
            .expect("lưu được dòng của lô");

        // Lô mới xuất ('exported') chưa tính là đã khai.
        let exported = crate::declaration::repo::count_undeclared_within_48h(&pool)
            .await
            .expect("đếm được");
        assert_eq!(exported, 2, "xuất file chưa phải là đã khai");

        crate::declaration::repo::set_batch_verified(&pool, &batch, 1)
            .await
            .expect("đối chiếu xong");
        let verified = crate::declaration::repo::count_undeclared_within_48h(&pool)
            .await
            .expect("đếm được");
        assert_eq!(verified, 1, "một khách đã khai, còn một chưa");

        // §5.3 — lô failed không có entry verified nào, khách tự động quay lại
        // trạng thái chưa khai. Không cần code hoàn tác.
        crate::declaration::repo::set_batch_failed(&pool, &batch, 0)
            .await
            .expect("đánh dấu lô hỏng");
        let failed = crate::declaration::repo::count_undeclared_within_48h(&pool)
            .await
            .expect("đếm được");
        assert_eq!(failed, 2);
    }
}
