//! End-to-end QA cho mùa cao điểm, chạy trên **database thật đã migrate**.
//!
//! Khác `support::db::test_pool`: chỗ đó dựng bảng bằng `CREATE TABLE` viết tay,
//! nên một test xanh ở đó vẫn có thể xanh khi migration thật tạo ra schema khác.
//! Mọi test trong file này mở một file SQLite thật qua đúng đường kết nối của
//! production (`db::connect_configured_sqlite_pool`) rồi chạy `db::run_migrations`,
//! và đi qua đúng các hàm service mà lệnh Tauri gọi.
//!
//! Mọi con số kỳ vọng đều được tính tay trước và ghi phép tính vào comment.

use super::prelude::*;
use crate::queries::booking::pricing_queries;
use sqlx::{Pool, Sqlite};
use std::path::PathBuf;

// ─── Fixtures ───

/// Một database thật, đã chạy đúng bộ migration của app.
struct MigratedDb {
    pool: Pool<Sqlite>,
    path: PathBuf,
}

impl MigratedDb {
    async fn close(self) {
        self.pool.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", self.path.display())));
        }
    }
}

async fn migrated_db(label: &str) -> MigratedDb {
    let path = std::env::temp_dir().join(format!(
        "capyinn-qa-{label}-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let url = format!("sqlite:{}?mode=rwc", path.display());

    let pool = crate::db::connect_configured_sqlite_pool(&url)
        .await
        .expect("mở pool sqlite như production");
    crate::db::run_migrations(&pool)
        .await
        .expect("chạy migration thật");

    MigratedDb { pool, path }
}

/// Phòng đôi 500.000₫/đêm, giá base đã gồm `max_guests` khách, mỗi khách vượt
/// mốc chịu `extra_person_fee` một đêm. Không khai `pricing_rules`, nên engine
/// suy rule từ `rooms.base_price` — đúng cảnh nhà nghỉ nhỏ trong lời phàn nàn.
async fn seed_room(
    pool: &Pool<Sqlite>,
    room_id: &str,
    room_type: &str,
    base_price: i64,
    max_guests: i64,
    extra_person_fee: i64,
) {
    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price,
                            max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, 1, 0, ?, ?, ?, 'vacant')",
    )
    .bind(room_id)
    .bind(format!("Phòng {room_id}"))
    .bind(room_type)
    .bind(base_price)
    .bind(max_guests)
    .bind(extra_person_fee)
    .execute(pool)
    .await
    .expect("seed room");
}

async fn declare_season(pool: &Pool<Sqlite>, from: &str, to: &str, label: &str, pct: f64) {
    pricing_service::save_special_date_range(
        pool,
        pricing_service::SaveSpecialDateRange {
            remove: Vec::new(),
            from: from.to_string(),
            to: to.to_string(),
            label: label.to_string(),
            uplift_pct: pct,
        },
        uuid::Uuid::new_v4().to_string(),
        "2026-01-15T09:00:00+07:00".to_string(),
    )
    .await
    .expect("khai mùa cao điểm");
}

async fn stored_dates(pool: &Pool<Sqlite>) -> Vec<(String, String, f64)> {
    sqlx::query_as::<_, (String, String, f64)>(
        "SELECT date, label, CAST(uplift_pct AS REAL) FROM special_dates ORDER BY date",
    )
    .fetch_all(pool)
    .await
    .expect("đọc special_dates")
}

// ─── 1. Ca của chủ nhà ───

/// *"Phòng ghi 500k, mà ngày đặt là mùa cao điểm lại 4 người, nên tôi thu 600k,
/// và không có chỗ nào sửa giá phòng."*
///
/// Cả hai nửa phải cộng vào nhau đúng cách: phần trăm cao điểm chỉ ăn vào tiền
/// phòng, **không** nhân lên phụ thu thêm người.
///
/// Phòng đôi 500.000₫/đêm, `max_guests` = 2, phụ thu 50.000₫/khách/đêm.
/// Mùa cao điểm 01/03→05/03/2026, +20%.
/// Kỳ ở 02/03→04/03/2026 = 2 đêm (02/03 thứ Hai, 03/03 thứ Ba — không đêm nào
/// rơi vào cuối tuần, nên uplift cuối tuần 20% mặc định không tham gia).
/// 4 khách.
///
///   base               = 500.000 × 2 đêm                  = 1.000.000
///   phụ thu cao điểm   = 1.000.000 × (20 + 20) / 2 %      =   200.000
///   phụ thu cuối tuần  = 0 đêm cuối tuần                  =         0
///   phụ thu thêm người = 50.000 × (4 − 2) khách × 2 đêm   =   200.000
///   ─────────────────────────────────────────────────────────────────
///   tổng                                                  = 1.400.000
///
/// Nếu uplift nhân cả phần thêm người, tổng sẽ là
/// (1.000.000 + 200.000) × 1,20 = 1.440.000 — con số đó phải không xuất hiện.
#[tokio::test]
async fn the_owners_case_peak_season_and_four_guests_compose_without_multiplying_each_other() {
    let db = migrated_db("owner-case").await;
    seed_room(&db.pool, "P101", "Phòng đôi", 500_000, 2, 50_000).await;
    declare_season(&db.pool, "2026-03-01", "2026-03-05", "Cao điểm hè", 20.0).await;

    let mut tx = db.pool.begin().await.expect("begin");
    let charged = calculate_stay_price_tx(
        &mut tx,
        "P101",
        "2026-03-02",
        "2026-03-04",
        "nightly",
        Some(4),
    )
    .await
    .expect("tính giá kỳ ở");
    tx.rollback().await.expect("rollback");

    assert_eq!(charged.base_amount, 1_000_000, "tiền phòng 2 đêm");
    assert_eq!(charged.surcharge_amount, 200_000, "cao điểm 20% trên base");
    assert_eq!(charged.weekend_amount, 0, "hai đêm đều là ngày thường");
    assert_eq!(charged.total, 1_400_000, "tổng chủ nhà mong đợi");
    assert_ne!(
        charged.total, 1_440_000,
        "cao điểm không được nhân lên phụ thu thêm người"
    );

    let extra_line = charged
        .breakdown
        .iter()
        .find(|line| line.label.contains("khách"))
        .expect("phải có dòng phụ thu thêm người");
    assert_eq!(extra_line.label, "Phụ thu 2 khách");
    assert_eq!(
        extra_line.amount, 200_000,
        "phụ thu thêm người là khoản phẳng, không bị uplift chạm vào"
    );

    db.close().await;
}

/// Cùng ca trên nhưng đi qua vòng đời đặt phòng thật: số ghi vào
/// `bookings.total_price` phải đúng bằng 1.400.000 đã tính tay ở trên.
#[tokio::test]
async fn the_owners_case_is_what_gets_written_to_the_booking() {
    let db = migrated_db("owner-case-booking").await;
    seed_room(&db.pool, "P101", "Phòng đôi", 500_000, 2, 50_000).await;
    declare_season(&db.pool, "2026-03-01", "2026-03-05", "Cao điểm hè", 20.0).await;

    let booking = reservation_lifecycle::create_reservation(
        &db.pool,
        CreateReservationRequest {
            room_id: "P101".to_string(),
            guest_name: "Nguyễn Văn A".to_string(),
            guest_phone: Some("0900000000".to_string()),
            guest_doc_number: Some("001234567890".to_string()),
            check_in_date: "2026-03-02".to_string(),
            check_out_date: "2026-03-04".to_string(),
            nights: 2,
            deposit_amount: None,
            source: Some("phone".to_string()),
            notes: None,
            guests: Some(4),
        },
    )
    .await
    .expect("tạo đặt phòng");

    assert_eq!(booking.total_price, 1_400_000);

    let stored: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind(&booking.id)
        .fetch_one(&db.pool)
        .await
        .expect("đọc lại booking");
    assert_eq!(stored, 1_400_000, "số ghi xuống bảng phải khớp");

    db.close().await;
}

// ─── 2. Kỳ ở bắt đầu trước mùa ───

/// Mùa cao điểm 04/03→08/03/2026, +30%.
/// Kỳ ở 02/03→06/03 = 4 đêm: 02(T2), 03(T3), 04(T4), 05(T5). Không đêm cuối tuần.
/// Chỉ hai đêm 04 và 05 nằm trong mùa.
///
///   base            = 500.000 × 4 đêm              = 2.000.000
///   mức bình quân   = (0 + 0 + 30 + 30) / 4        =        15%
///   phụ thu cao điểm= 2.000.000 × 15%              =   300.000
///                   ( = 2 đêm × 500.000 × 30%  ✓)
///   ───────────────────────────────────────────────────────────
///   tổng                                           = 2.300.000
///
/// Luật cũ (đọc phần trăm ở ngày đến) sẽ ra 0₫ vì 02/03 chưa khai.
#[tokio::test]
async fn a_stay_starting_before_the_season_is_surcharged_only_for_the_nights_inside() {
    let db = migrated_db("before-season").await;
    seed_room(&db.pool, "P201", "Phòng đôi", 500_000, 2, 0).await;
    declare_season(&db.pool, "2026-03-04", "2026-03-08", "Giỗ Tổ", 30.0).await;

    let mut tx = db.pool.begin().await.expect("begin");
    let charged =
        calculate_stay_price_tx(&mut tx, "P201", "2026-03-02", "2026-03-06", "nightly", None)
            .await
            .expect("tính giá");
    tx.rollback().await.expect("rollback");

    assert_eq!(charged.base_amount, 2_000_000);
    assert_eq!(charged.weekend_amount, 0);
    assert_eq!(charged.surcharge_amount, 300_000);
    assert_eq!(charged.total, 2_300_000);
    assert_ne!(
        charged.surcharge_amount, 0,
        "luật cũ đọc ngày đến sẽ ra 0₫ — đây chính là lỗi được sửa"
    );

    db.close().await;
}

// ─── 3. Kỳ ở kéo qua khỏi mùa ───

/// Mùa cao điểm 02/03→04/03/2026, +40%.
/// Kỳ ở 03/03→07/03 = 4 đêm: 03(T3), 04(T4), 05(T5), 06(T6). Không đêm cuối tuần.
/// Chỉ hai đêm 03 và 04 nằm trong mùa.
///
///   base            = 500.000 × 4 đêm              = 2.000.000
///   mức bình quân   = (40 + 40 + 0 + 0) / 4        =        20%
///   phụ thu cao điểm= 2.000.000 × 20%              =   400.000
///                   ( = 2 đêm × 500.000 × 40%  ✓)
///   ───────────────────────────────────────────────────────────
///   tổng                                           = 2.400.000
///
/// Luật cũ sẽ thu 40% cho cả bốn đêm: 2.000.000 × 40% = 800.000 — thu lố.
#[tokio::test]
async fn a_stay_running_past_the_end_of_the_season_stops_being_surcharged() {
    let db = migrated_db("after-season").await;
    seed_room(&db.pool, "P301", "Phòng đôi", 500_000, 2, 0).await;
    declare_season(&db.pool, "2026-03-02", "2026-03-04", "Cao điểm", 40.0).await;

    let mut tx = db.pool.begin().await.expect("begin");
    let charged =
        calculate_stay_price_tx(&mut tx, "P301", "2026-03-03", "2026-03-07", "nightly", None)
            .await
            .expect("tính giá");
    tx.rollback().await.expect("rollback");

    assert_eq!(charged.base_amount, 2_000_000);
    assert_eq!(charged.weekend_amount, 0);
    assert_eq!(charged.surcharge_amount, 400_000);
    assert_eq!(charged.total, 2_400_000);
    assert_ne!(
        charged.surcharge_amount, 800_000,
        "luật cũ thu 40% cho cả bốn đêm"
    );

    db.close().await;
}

// ─── 4. Khai rồi rút ngắn ───

/// Khai 01/03→10/03 (+25%), rồi lưu lại thành 01/03→05/03 (+30%) với năm ngày
/// 06/03–10/03 nằm trong `remove`.
///
/// Sau lần lưu thứ hai bảng phải còn đúng năm dòng 01–05, uplift 30, và các
/// dòng ấy phải là dòng cũ được sửa tại chỗ (`created_at` giữ nguyên mốc lần
/// khai đầu), chứ không phải dòng mới chèn đè.
#[tokio::test]
async fn shortening_a_declared_season_drops_exactly_the_removed_days() {
    let db = migrated_db("shorten").await;

    pricing_service::save_special_date_range(
        &db.pool,
        pricing_service::SaveSpecialDateRange {
            remove: Vec::new(),
            from: "2026-03-01".to_string(),
            to: "2026-03-10".to_string(),
            label: "Cao điểm hè".to_string(),
            uplift_pct: 25.0,
        },
        "seed-base".to_string(),
        "2026-01-15T09:00:00+07:00".to_string(),
    )
    .await
    .expect("khai lần đầu");

    assert_eq!(
        stored_dates(&db.pool).await.len(),
        10,
        "10 ngày 01/03–10/03"
    );

    let dropped: Vec<String> = (6..=10).map(|day| format!("2026-03-{day:02}")).collect();
    pricing_service::save_special_date_range(
        &db.pool,
        pricing_service::SaveSpecialDateRange {
            remove: dropped.clone(),
            from: "2026-03-01".to_string(),
            to: "2026-03-05".to_string(),
            label: "Cao điểm hè".to_string(),
            uplift_pct: 30.0,
        },
        "edit-base".to_string(),
        "2026-02-20T09:00:00+07:00".to_string(),
    )
    .await
    .expect("lưu lại ngắn hơn");

    let rows = stored_dates(&db.pool).await;
    let kept: Vec<String> = (1..=5).map(|day| format!("2026-03-{day:02}")).collect();
    assert_eq!(
        rows.iter().map(|row| row.0.clone()).collect::<Vec<_>>(),
        kept,
        "chỉ còn 01/03–05/03"
    );
    for row in &rows {
        assert_eq!(row.1, "Cao điểm hè", "nhãn phải được cập nhật");
        assert_eq!(row.2, 30.0, "uplift phải được cập nhật");
    }
    for date in &dropped {
        assert!(
            !rows.iter().any(|row| &row.0 == date),
            "{date} phải bị xoá hẳn khỏi bảng"
        );
    }

    let created_at: Vec<String> =
        sqlx::query_scalar("SELECT created_at FROM special_dates ORDER BY date")
            .fetch_all(&db.pool)
            .await
            .expect("đọc created_at");
    assert!(
        created_at
            .iter()
            .all(|value| value == "2026-01-15T09:00:00+07:00"),
        "ngày giữ lại phải là dòng cũ được sửa tại chỗ, không phải dòng mới: {created_at:?}"
    );

    db.close().await;
}

// ─── 5. Xoá hẳn một mùa ───

/// Khai 01/03→05/03 (+30%), tính giá kỳ ở 02/03→04/03 (2 đêm, cả hai trong mùa):
///
///   base             = 500.000 × 2 đêm      = 1.000.000
///   mức bình quân    = (30 + 30) / 2        =        30%
///   phụ thu cao điểm = 1.000.000 × 30%      =   300.000
///   tổng                                    = 1.300.000
///
/// Xoá cả mùa rồi tính lại: không còn ngày nào được khai, nên
///   tổng = base = 1.000.000, phụ thu = 0.
#[tokio::test]
async fn deleting_a_whole_season_puts_the_price_back_to_no_uplift() {
    let db = migrated_db("delete-season").await;
    seed_room(&db.pool, "P401", "Phòng đôi", 500_000, 2, 0).await;
    declare_season(&db.pool, "2026-03-01", "2026-03-05", "Cao điểm hè", 30.0).await;

    let mut tx = db.pool.begin().await.expect("begin");
    let with_season =
        calculate_stay_price_tx(&mut tx, "P401", "2026-03-02", "2026-03-04", "nightly", None)
            .await
            .expect("tính giá khi còn mùa");
    tx.rollback().await.expect("rollback");

    assert_eq!(with_season.base_amount, 1_000_000);
    assert_eq!(with_season.surcharge_amount, 300_000);
    assert_eq!(with_season.total, 1_300_000);

    let whole_season: Vec<String> = (1..=5).map(|day| format!("2026-03-{day:02}")).collect();
    pricing_service::delete_special_dates(&db.pool, whole_season)
        .await
        .expect("xoá cả mùa");

    assert!(
        stored_dates(&db.pool).await.is_empty(),
        "bảng phải trống sau khi xoá mùa"
    );

    let mut tx = db.pool.begin().await.expect("begin");
    let without_season =
        calculate_stay_price_tx(&mut tx, "P401", "2026-03-02", "2026-03-04", "nightly", None)
            .await
            .expect("tính giá sau khi xoá");
    tx.rollback().await.expect("rollback");

    assert_eq!(without_season.base_amount, 1_000_000);
    assert_eq!(
        without_season.surcharge_amount, 0,
        "hết mùa thì hết phụ thu"
    );
    assert_eq!(without_season.total, 1_000_000);

    db.close().await;
}

// ─── 6. Xem trước phải bằng số thu ───

/// Lời hứa mà cả thiết kế đặt cược vào. Cùng phòng, cùng ngày, cùng số khách:
/// `calculate_room_price_preview` và `calculate_stay_price_tx` phải trả về y
/// hệt nhau — từng dòng breakdown, không chỉ tổng.
///
/// Dùng lại đúng ca của chủ nhà, nên con số phải là 1.400.000 (xem phép tính ở
/// test đầu file). So sánh mà cả hai cùng rỗng thì vô nghĩa, nên tổng được
/// khoá vào con số tính tay.
#[tokio::test]
async fn the_preview_and_the_transactional_charge_return_the_identical_total() {
    let db = migrated_db("preview-equals-charge").await;
    seed_room(&db.pool, "P501", "Phòng đôi", 500_000, 2, 50_000).await;
    declare_season(&db.pool, "2026-03-01", "2026-03-05", "Cao điểm hè", 20.0).await;

    let preview = pricing_service::calculate_room_price_preview(
        &db.pool,
        "P501",
        "2026-03-02",
        "2026-03-04",
        "nightly",
        Some(4),
    )
    .await
    .expect("xem trước");

    let mut tx = db.pool.begin().await.expect("begin");
    let charged = calculate_stay_price_tx(
        &mut tx,
        "P501",
        "2026-03-02",
        "2026-03-04",
        "nightly",
        Some(4),
    )
    .await
    .expect("thu tiền");
    tx.rollback().await.expect("rollback");

    assert_eq!(preview.total, charged.total);
    assert_eq!(preview.base_amount, charged.base_amount);
    assert_eq!(preview.surcharge_amount, charged.surcharge_amount);
    assert_eq!(preview.weekend_amount, charged.weekend_amount);
    assert_eq!(preview.pricing_type, charged.pricing_type);
    assert_eq!(preview.breakdown.len(), charged.breakdown.len());
    for (left, right) in preview.breakdown.iter().zip(charged.breakdown.iter()) {
        assert_eq!(left.label, right.label);
        assert_eq!(left.amount, right.amount);
    }

    assert_eq!(charged.total, 1_400_000, "so sánh phải neo vào số tính tay");
    assert_eq!(charged.surcharge_amount, 200_000);

    db.close().await;
}

// ─── 7. Schema thật ───

/// `special_dates` do migration thật tạo ra phải đúng những gì code giả định:
/// `id TEXT PRIMARY KEY`, `date TEXT NOT NULL` có `UNIQUE`, `label TEXT NOT
/// NULL`, `uplift_pct REAL NOT NULL`, `created_at TEXT NOT NULL`.
#[tokio::test]
async fn the_migrated_special_dates_table_matches_what_the_code_assumes() {
    let db = migrated_db("schema").await;

    let columns: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT name, type, \"notnull\", pk FROM pragma_table_info('special_dates')",
    )
    .fetch_all(&db.pool)
    .await
    .expect("pragma_table_info");

    let names: Vec<&str> = columns.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        names,
        vec!["id", "date", "label", "uplift_pct", "created_at"],
        "tên và thứ tự cột"
    );

    let by_name = |name: &str| {
        columns
            .iter()
            .find(|row| row.0 == name)
            .unwrap_or_else(|| panic!("thiếu cột {name}"))
            .clone()
    };
    assert_eq!(by_name("id").1, "TEXT");
    assert_eq!(by_name("id").3, 1, "id là PRIMARY KEY");
    assert_eq!(by_name("date").1, "TEXT");
    assert_eq!(by_name("date").2, 1, "date NOT NULL");
    assert_eq!(by_name("label").1, "TEXT");
    assert_eq!(by_name("label").2, 1, "label NOT NULL");
    assert_eq!(by_name("uplift_pct").1, "REAL");
    assert_eq!(by_name("uplift_pct").2, 1, "uplift_pct NOT NULL");
    assert_eq!(by_name("created_at").1, "TEXT");
    assert_eq!(by_name("created_at").2, 1, "created_at NOT NULL");

    // UNIQUE(date): có index unique và index ấy đúng trên cột `date`.
    let unique_indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_index_list('special_dates') WHERE \"unique\" = 1",
    )
    .fetch_all(&db.pool)
    .await
    .expect("pragma_index_list");
    let mut unique_columns: Vec<String> = Vec::new();
    for index in &unique_indexes {
        let cols: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                .bind(index)
                .fetch_all(&db.pool)
                .await
                .expect("pragma_index_info");
        if cols.len() == 1 {
            unique_columns.push(cols[0].clone());
        }
    }
    assert!(
        unique_columns.iter().any(|column| column == "date"),
        "phải có UNIQUE trên cột date, thấy: {unique_columns:?}"
    );

    // Và ràng buộc ấy phải thật sự chặn, không chỉ có tên.
    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES ('a', '2026-03-01', 'Lễ', 10.0, '2026-01-01T00:00:00+07:00')",
    )
    .execute(&db.pool)
    .await
    .expect("dòng đầu");
    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES ('b', '2026-03-01', 'Lễ', 10.0, '2026-01-01T00:00:00+07:00')",
    )
    .execute(&db.pool)
    .await
    .expect_err("ngày trùng phải bị UNIQUE(date) chặn");

    db.close().await;
}

/// Nhánh này không được thêm hay sửa migration nào ngoài phạm vi của nó: phiên
/// bản schema sau khi migrate phải khớp `LATEST_SCHEMA_VERSION` hiện hành.
#[tokio::test]
async fn the_branch_does_not_move_the_schema_version() {
    let db = migrated_db("schema-version").await;

    let version: i64 = sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
        .fetch_one(&db.pool)
        .await
        .expect("đọc schema_version");

    assert_eq!(
        version,
        i64::from(crate::db::LATEST_SCHEMA_VERSION),
        "nhánh mùa cao điểm không đụng vào migration"
    );

    db.close().await;
}

// ─── 8. Tương thích ngược ───

/// Dòng do lệnh `save_special_date` (đã bị xoá) ghi ra: một ngày lẻ, `id` là
/// uuid, `created_at` là rfc3339 của `chrono::Local::now()`.
async fn insert_the_old_way(pool: &Pool<Sqlite>, date: &str, label: &str, pct: f64) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(date) DO UPDATE SET label = excluded.label, uplift_pct = excluded.uplift_pct",
    )
    .bind(&id)
    .bind(date)
    .bind(label)
    .bind(pct)
    .bind(Local::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("ghi kiểu lệnh cũ");
    id
}

/// Dữ liệu do bản cũ ghi ra phải đọc được, tính giá đúng, và sửa được bằng lệnh
/// mới mà không mất `id` — mọi tham chiếu bên ngoài còn dùng được.
///
/// Ba ngày lẻ 02/03, 03/03, 04/03 cùng +30%. Kỳ ở 02/03→04/03 = 2 đêm (02, 03).
///
///   base             = 500.000 × 2 đêm  = 1.000.000
///   mức bình quân    = (30 + 30) / 2    =        30%
///   phụ thu          = 1.000.000 × 30%  =   300.000
///   tổng                                = 1.300.000
#[tokio::test]
async fn rows_written_by_the_deleted_single_day_command_still_read_and_price_correctly() {
    let db = migrated_db("backcompat").await;
    seed_room(&db.pool, "P601", "Phòng đôi", 500_000, 2, 0).await;

    let mut old_ids = Vec::new();
    for date in ["2026-03-02", "2026-03-03", "2026-03-04"] {
        old_ids.push(insert_the_old_way(&db.pool, date, "Lễ cũ", 30.0).await);
    }

    // Đọc lại qua đúng query mà màn hình cài đặt dùng.
    let listed = pricing_queries::load_special_dates(&db.pool)
        .await
        .expect("load_special_dates");
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].date, "2026-03-02");
    assert_eq!(listed[0].label, "Lễ cũ");
    assert_eq!(listed[0].uplift_pct, 30.0);
    assert_eq!(
        listed[0].id, old_ids[0],
        "id uuid cũ phải đọc lại nguyên vẹn"
    );

    let mut tx = db.pool.begin().await.expect("begin");
    let charged =
        calculate_stay_price_tx(&mut tx, "P601", "2026-03-02", "2026-03-04", "nightly", None)
            .await
            .expect("tính giá trên dữ liệu cũ");
    tx.rollback().await.expect("rollback");

    assert_eq!(charged.base_amount, 1_000_000);
    assert_eq!(charged.surcharge_amount, 300_000);
    assert_eq!(charged.total, 1_300_000);

    // Lệnh mới sửa đè lên dữ liệu cũ: giữ id, đổi nhãn và mức.
    declare_season(&db.pool, "2026-03-02", "2026-03-04", "Cao điểm mới", 45.0).await;

    let rows: Vec<(String, String, String, f64)> = sqlx::query_as(
        "SELECT id, date, label, CAST(uplift_pct AS REAL) FROM special_dates ORDER BY date",
    )
    .fetch_all(&db.pool)
    .await
    .expect("đọc lại sau khi sửa");
    assert_eq!(rows.len(), 3, "không được đẻ thêm dòng cho ngày đã có");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.0, old_ids[index], "id uuid cũ phải được giữ");
        assert_eq!(row.2, "Cao điểm mới");
        assert_eq!(row.3, 45.0);
    }

    // Và xoá được bằng lệnh mới.
    pricing_service::delete_special_dates(&db.pool, vec!["2026-03-03".to_string()])
        .await
        .expect("xoá một ngày cũ");
    let remaining = stored_dates(&db.pool).await;
    assert_eq!(
        remaining
            .iter()
            .map(|row| row.0.as_str())
            .collect::<Vec<_>>(),
        vec!["2026-03-02", "2026-03-04"]
    );

    db.close().await;
}

/// Database đã có sẵn dữ liệu mùa cao điểm từ trước: chạy lại migration (đúng
/// việc app làm mỗi lần khởi động) không được đụng vào dòng nào, và giá vẫn
/// phải ra đúng con số cũ.
#[tokio::test]
async fn a_database_that_already_had_special_dates_survives_a_migration_rerun() {
    let db = migrated_db("existing-rows").await;
    seed_room(&db.pool, "P701", "Phòng đôi", 500_000, 2, 0).await;
    for date in ["2026-03-02", "2026-03-03"] {
        insert_the_old_way(&db.pool, date, "Lễ cũ", 30.0).await;
    }

    let before = stored_dates(&db.pool).await;

    crate::db::run_migrations(&db.pool)
        .await
        .expect("chạy lại migration trên db đã có dữ liệu");

    let after = stored_dates(&db.pool).await;
    assert_eq!(before, after, "migration chạy lại không được sửa dữ liệu");

    let mut tx = db.pool.begin().await.expect("begin");
    let charged =
        calculate_stay_price_tx(&mut tx, "P701", "2026-03-02", "2026-03-04", "nightly", None)
            .await
            .expect("tính giá sau khi migrate lại");
    tx.rollback().await.expect("rollback");

    // Y hệt phép tính ở test trên: 1.000.000 base + 30% = 1.300.000.
    assert_eq!(charged.total, 1_300_000);

    db.close().await;
}
