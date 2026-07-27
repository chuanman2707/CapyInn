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

use super::{execute_compat_alter, set_schema_version};

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

/// Migration v22 — "băng chuyền một chiều" (spec 2026-07-27).
///
/// 1. `held_at`: dấu "gác lại" của một khai báo. NULL = đang trong danh sách
///    chờ xuất. Cột additive nên chỉ cần ALTER, không rebuild bảng.
/// 2. Backfill: khái niệm "hồ sơ chờ chưa ghép" biến mất khỏi UI, nên danh
///    tính nào đang mồ côi (dữ liệu của bản cũ để lại) phải được tạo link mặc
///    định — nếu không, nâng cấp xong khách biến mất vô hình. Link đó sinh ra
///    GÁC LẠI (xem `backfill_orphan_identities`) chứ không phải chờ xuất bình
///    thường — orphan là ảnh quét dở, không chắc là khách thật.
///
/// Đây là bảng của riêng module này — luật "không migrate PMS" không bị đụng.
pub(super) async fn migrate_v22_conveyor(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    // Dùng execute_compat_alter (không sqlx::query trần) để ALTER an toàn khi
    // chạy lại: nếu tiến trình chết sau khi ALTER commit nhưng trước khi
    // schema_version=22 commit, lần khởi động sau sẽ chạy lại migration này —
    // ALTER trần sẽ báo lỗi "duplicate column name" và app không khởi động
    // được nữa nếu không sửa DB thủ công.
    let mut tx = pool.begin().await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE declaration_link ADD COLUMN held_at TEXT",
    )
    .await?;
    tx.commit().await?;

    backfill_orphan_identities(pool).await?;

    let mut tx = pool.begin().await?;
    set_schema_version(&mut tx, 22).await?;
    tx.commit().await?;
    Ok(())
}

/// Tạo link mặc định cho mọi danh tính chưa có link nào.
///
/// Tách khỏi `migrate_v22_conveyor` để test gọi được đúng code production:
/// bản thân migration không chạy lại được (ALTER lần hai báo trùng cột), còn
/// hàm này idempotent nhờ `NOT EXISTS`.
///
/// Link backfill LUÔN sinh ra ở trạng thái GÁC LẠI (`held_at` có giá trị).
/// Danh tính mồ côi trên máy thật là ảnh quét rồi bỏ dở, chưa từng gắn vào
/// lượt lưu trú nào — nếu để chúng hiện ra như thẻ chờ bình thường, UI cũ vốn
/// mặc định chọn sẵn mọi dòng chờ để xuất, và bản nâng cấp sẽ tự động xếp một
/// người chưa từng thật sự là khách vào file khai báo tiếp theo. Gác lại buộc
/// người vận hành phải chủ động rà rồi mới thả ra.
pub(crate) async fn backfill_orphan_identities(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let held_at = chrono::Local::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO declaration_link (id, identity_id, stay_id, stay_reason, held_at, created_at)
         SELECT lower(hex(randomblob(16))), di.id, NULL, '1', ?, di.created_at
           FROM declaration_identity di
          WHERE di.redacted_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM declaration_link dl WHERE dl.identity_id = di.id)",
    )
    .bind(&held_at)
    .execute(pool)
    .await?;
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

    /// Đúng tình huống đã kẹt trên máy vận hành: thả cùng một tấm CCCD nhiều
    /// lần rồi ghép cả hai → hai khai báo cùng số giấy tờ cùng ngày đến → E14
    /// chặn xuất file, và không có đường nào đi tiếp.
    #[tokio::test]
    async fn dropping_the_same_card_twice_reuses_the_identity() {
        let pool = seeded_pool().await;

        let card = crate::declaration::model::Identity {
            full_name: "Phạm Thị Minh Hiền".into(),
            dob: "1988-12-16".into(),
            gender: "F".into(),
            nationality_iso3: "VNM".into(),
            doc_no: Some("056188011500".into()),
            ..Default::default()
        };

        let first = crate::declaration::repo::insert_identity(&pool, &card, "qr_cccd", "verified")
            .await
            .expect("lần thả thứ nhất");
        let second = crate::declaration::repo::insert_identity(&pool, &card, "qr_cccd", "verified")
            .await
            .expect("lần thả thứ hai");

        assert_eq!(first, second, "cùng số giấy tờ phải là cùng một danh tính");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_identity")
            .fetch_one(&pool)
            .await
            .expect("đếm danh tính");
        assert_eq!(count, 1, "không được đẻ thêm dòng");
    }

    /// Không có số giấy tờ thì không có gì để nhận ra người trùng — nhập tay
    /// thiếu số vẫn phải lưu được, mỗi lần một dòng.
    #[tokio::test]
    async fn an_identity_without_a_document_number_is_never_merged() {
        let pool = seeded_pool().await;

        let no_doc = crate::declaration::model::Identity {
            full_name: "Khách nhập tay".into(),
            dob: "1990-01-01".into(),
            nationality_iso3: "VNM".into(),
            ..Default::default()
        };

        let a = crate::declaration::repo::insert_identity(&pool, &no_doc, "manual", "needs_review")
            .await
            .expect("lưu lần 1");
        let b = crate::declaration::repo::insert_identity(&pool, &no_doc, "manual", "needs_review")
            .await
            .expect("lưu lần 2");

        assert_ne!(a, b);
    }

    /// Đã nộp cho công an rồi thì bản ghi là bằng chứng của cái đã nộp — lần
    /// thả sau không được sửa nó sau lưng.
    #[tokio::test]
    async fn a_declared_identity_is_not_rewritten_by_a_later_scan() {
        let pool = seeded_pool().await;

        let card = crate::declaration::model::Identity {
            full_name: "Phạm Thị Minh Hiền".into(),
            dob: "1988-12-16".into(),
            gender: "F".into(),
            nationality_iso3: "VNM".into(),
            doc_no: Some("056188011500".into()),
            ..Default::default()
        };
        let id = crate::declaration::repo::insert_identity(&pool, &card, "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");
        let link =
            crate::declaration::repo::insert_link(&pool, &id, Some("booking-1"), "2", None)
                .await
                .expect("ghép");
        let batch = crate::declaration::repo::insert_batch(&pool, "VN", "/tmp/x.xlsx", 1)
            .await
            .expect("lô");
        crate::declaration::repo::insert_entries(&pool, &batch, std::slice::from_ref(&link))
            .await
            .expect("dòng của lô");
        crate::declaration::repo::set_batch_verified(&pool, &batch, 1)
            .await
            .expect("đối soát xong");

        let mut misread = card.clone();
        misread.full_name = "TÊN ĐỌC SAI".into();
        let again = crate::declaration::repo::insert_identity(&pool, &misread, "qr_cccd", "verified")
            .await
            .expect("thả lại");

        assert_eq!(again, id, "vẫn là cùng một người");
        let name: String =
            sqlx::query_scalar("SELECT full_name FROM declaration_identity WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("đọc tên");
        assert_eq!(name, "Phạm Thị Minh Hiền", "tên đã khai không được ghi đè");
    }

    /// Ghép nhầm thì phải gỡ được — nếu không, một dòng thừa chặn cả lô.
    #[tokio::test]
    async fn a_declaration_can_be_unlinked_until_it_has_been_reconciled() {
        let pool = seeded_pool().await;

        let id = crate::declaration::repo::insert_identity(
            &pool,
            &crate::declaration::model::Identity {
                full_name: "Phạm Thị Minh Hiền".into(),
                nationality_iso3: "VNM".into(),
                doc_no: Some("056188011500".into()),
                ..Default::default()
            },
            "qr_cccd",
            "verified",
        )
        .await
        .expect("danh tính");
        let link =
            crate::declaration::repo::insert_link(&pool, &id, Some("booking-1"), "2", None)
                .await
                .expect("ghép");

        // Đã xuất file nhưng chưa đối soát: vẫn gỡ được.
        let batch = crate::declaration::repo::insert_batch(&pool, "VN", "/tmp/x.xlsx", 1)
            .await
            .expect("lô");
        crate::declaration::repo::insert_entries(&pool, &batch, std::slice::from_ref(&link))
            .await
            .expect("dòng của lô");
        crate::declaration::repo::delete_link(&pool, &link)
            .await
            .expect("lô chưa đối soát thì gỡ được");

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link")
            .fetch_one(&pool)
            .await
            .expect("đếm");
        assert_eq!(left, 0);
    }

    /// Đã đối soát = đã khai lên cổng. Xóa đi là mất dấu vết.
    #[tokio::test]
    async fn a_reconciled_declaration_refuses_to_be_unlinked() {
        let pool = seeded_pool().await;

        let id = crate::declaration::repo::insert_identity(
            &pool,
            &crate::declaration::model::Identity {
                full_name: "Phạm Thị Minh Hiền".into(),
                nationality_iso3: "VNM".into(),
                doc_no: Some("056188011500".into()),
                ..Default::default()
            },
            "qr_cccd",
            "verified",
        )
        .await
        .expect("danh tính");
        let link =
            crate::declaration::repo::insert_link(&pool, &id, Some("booking-1"), "2", None)
                .await
                .expect("ghép");
        let batch = crate::declaration::repo::insert_batch(&pool, "VN", "/tmp/x.xlsx", 1)
            .await
            .expect("lô");
        crate::declaration::repo::insert_entries(&pool, &batch, std::slice::from_ref(&link))
            .await
            .expect("dòng của lô");
        crate::declaration::repo::set_batch_verified(&pool, &batch, 1)
            .await
            .expect("đối soát");

        assert!(
            crate::declaration::repo::delete_link(&pool, &link)
                .await
                .is_err(),
            "đã đối soát thì không gỡ được"
        );
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
        assert_eq!(version, crate::db::SCHEMA_VERSION);
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

    /// Badge = bốn nguồn cộng lại: `not_scanned` (khách PMS check-in trong 48h
    /// chưa hề được quét — báo động ngay cả khi chưa ai quét ai) cộng ba nhóm
    /// link đã quét mà chưa nằm trong lô `verified` ("chưa xuất", "gác lại",
    /// "chờ đối chiếu", kể cả lô `failed`).
    ///
    /// `seeded_pool` dựng sẵn `booking-1` (active, check-in "bây giờ") với hai
    /// khách PMS (`guest-1`, `guest-2`) — nên `not_scanned` khởi điểm là 2 và
    /// không đổi cho tới khi một link được GẮN vào đúng lượt lưu trú đó.
    #[tokio::test]
    async fn the_badge_counts_every_link_not_yet_verified() {
        let pool = seeded_pool().await;
        use crate::declaration::repo;

        let card = crate::declaration::model::Identity {
            full_name: "Phan Thị Mỹ Hà".into(),
            dob: "1995-07-28".into(),
            gender: "F".into(),
            nationality_iso3: "VNM".into(),
            doc_no: Some("058195006173".into()),
            ..Default::default()
        };
        repo::save_identity_ensuring_link(&pool, &card, "qr_cccd", "verified")
            .await
            .expect("khách 1");
        let card2 = crate::declaration::model::Identity {
            full_name: "Khách Thứ Hai".into(),
            dob: "1991-02-02".into(),
            gender: "M".into(),
            nationality_iso3: "VNM".into(),
            doc_no: Some("012345678901".into()),
            ..Default::default()
        };
        repo::save_identity_ensuring_link(&pool, &card2, "qr_cccd", "verified")
            .await
            .expect("khách 2");

        // Hai khách vừa quét chưa gắn vào phòng nào (stay_id NULL) nên không
        // đụng tới hai khách PMS của booking-1 — not_scanned giữ nguyên 2.
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!(
            (b.total, b.not_scanned, b.not_exported, b.held, b.awaiting),
            (4, 2, 2, 0, 0)
        );

        // Gác một khách.
        let links = repo::pending_link_ids(&pool).await.expect("đọc");
        repo::set_link_held(&pool, &links[0], true).await.expect("gác");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!(
            (b.total, b.not_scanned, b.not_exported, b.held, b.awaiting),
            (4, 2, 1, 1, 0)
        );

        // Xuất khách còn lại.
        let batch = repo::insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        repo::insert_entries(&pool, &batch, std::slice::from_ref(&links[1]))
            .await
            .expect("dòng");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!(
            (b.total, b.not_scanned, b.not_exported, b.held, b.awaiting),
            (4, 2, 0, 1, 1)
        );

        // Chống đếm trùng: gắn khách vừa xuất vào ĐÚNG booking-1 (khách này
        // hóa ra chính là một trong hai khách PMS của lượt đó). Nếu badge trừ
        // sai (chỉ trừ link `verified`) thì not_scanned vẫn là 2 và người này
        // bị đếm hai lần — vừa "chờ đối chiếu" vừa "chưa quét". Trừ đúng
        // (theo MỌI link, không chỉ verified) thì not_scanned giảm còn 1 và
        // total giảm đúng 1, không phải giữ nguyên.
        repo::update_link(&pool, &links[1], Some("booking-1"), "1", None)
            .await
            .expect("gắn phòng");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!(
            (b.total, b.not_scanned, b.not_exported, b.held, b.awaiting),
            (3, 1, 0, 1, 1),
            "khách đã quét và đã gắn phòng chỉ được đếm một lần"
        );

        // Lô fail: vẫn đếm (khách chưa được khai thật).
        repo::set_batch_failed(&pool, &batch, 0).await.expect("fail");
        assert_eq!(repo::undeclared_breakdown(&pool).await.expect("đếm").total, 3);

        // Đối soát khớp: khách rời ba nhóm link, nhưng vẫn là "đã quét" nên
        // không quay lại not_scanned — guest-1 (khách PMS còn lại của
        // booking-1, chưa từng được quét) vẫn giữ not_scanned ở 1.
        repo::set_batch_verified(&pool, &batch, 1).await.expect("chốt");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!(
            (b.total, b.not_scanned, b.not_exported, b.held, b.awaiting),
            (2, 1, 0, 1, 0)
        );
    }

    /// v22 — khái niệm "hồ sơ chờ chưa ghép" biến mất: danh tính mồ côi từ
    /// bản cũ phải được tạo link mặc định, không khách nào kẹt vô hình.
    #[tokio::test]
    async fn v22_backfills_a_default_link_for_every_orphan_identity() {
        let pool = seeded_pool().await;

        // Danh tính mồ côi (giả lập dữ liệu để lại từ bản cũ).
        sqlx::query(
            "INSERT INTO declaration_identity (
                id, source, extract_confidence, full_name, dob, gender,
                nationality_iso3, created_at
             ) VALUES ('orphan-1', 'qr_cccd', 'verified', 'Khách Mồ Côi', '1990-01-01',
                       'M', 'VNM', '2026-07-20T09:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds orphan");

        // Gọi ĐÚNG hàm production, không chép SQL sang test — chép thì test chỉ
        // chứng minh SQLite chạy được, không chứng minh migration đúng.
        // Hàm idempotent nhờ NOT EXISTS nên gọi lại sau migration là hợp lệ.
        super::backfill_orphan_identities(&pool)
            .await
            .expect("backfill chạy lại được");

        let links: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM declaration_link WHERE identity_id = 'orphan-1' AND stay_id IS NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("đếm link");
        assert_eq!(
            links, 1,
            "mỗi danh tính mồ côi phải có đúng một link mặc định"
        );

        let held_at: Option<String> = sqlx::query_scalar(
            "SELECT held_at FROM declaration_link WHERE identity_id = 'orphan-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("đọc held_at");
        assert!(
            held_at.is_some(),
            "link backfill phải sinh ra GÁC LẠI — orphan chưa chắc là khách thật, \
             không được tự động lọt vào file xuất kế tiếp"
        );

        // Gọi lại lần hai để chứng minh hàm thực sự idempotent (đây chính là
        // lý do hàm được tách khỏi migration), không chỉ suy diễn từ NOT EXISTS.
        super::backfill_orphan_identities(&pool)
            .await
            .expect("backfill chạy lại được lần hai");

        let links_after_second_call: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM declaration_link WHERE identity_id = 'orphan-1' AND stay_id IS NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("đếm link lần hai");
        assert_eq!(
            links_after_second_call, 1,
            "gọi lại backfill lần hai không được tạo thêm link"
        );
    }

    #[tokio::test]
    async fn v22_adds_a_nullable_held_at_column() {
        let pool = seeded_pool().await;
        // Cột tồn tại và ghi/đọc được — đủ để chứng minh migration đã chạy.
        sqlx::query(
            "INSERT INTO declaration_identity (id, source, extract_confidence, full_name,
                dob, gender, nationality_iso3, created_at)
             VALUES ('id-h', 'manual', 'needs_review', 'A', '1990-01-01', 'M', 'VNM', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("seed");
        sqlx::query(
            "INSERT INTO declaration_link (id, identity_id, stay_id, stay_reason, held_at, created_at)
             VALUES ('l-h', 'id-h', NULL, '1', '2026-07-27T10:00:00+07:00', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("ghi held_at được");
        let held: Option<String> =
            sqlx::query_scalar("SELECT held_at FROM declaration_link WHERE id = 'l-h'")
                .fetch_one(&pool)
                .await
                .expect("đọc lại");
        assert!(held.is_some());
    }
}
