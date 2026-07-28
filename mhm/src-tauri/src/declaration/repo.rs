//! Nơi DUY NHẤT của module khai báo chạm cơ sở dữ liệu.
//!
//! Hai luật cứng, có test ranh giới ở cuối file canh:
//!
//! 1. `guests` / `bookings` / `booking_guests` / `rooms` chỉ được `SELECT`.
//!    PMS đang vận hành thật với dữ liệu thật của khách sạn — một tính năng phụ
//!    không được ghi vào nó, không được migrate nó.
//! 2. Không lưu ảnh, không lưu payload QR/MRZ thô (§12). Không log payload,
//!    không log đường dẫn ảnh.

use std::collections::HashMap;
use std::path::PathBuf;

use sqlx::{Pool, Row, Sqlite};

use crate::app_identity;
use crate::declaration::model::{DeclarationRow, Identity, StayInfo};
use crate::declaration::normalizer::{booking_ts_to_iso_date, strip_room_prefix};

const KEY_EXPORT_DIR: &str = "declaration.export_dir";
const KEY_CSLT_NAME: &str = "declaration.cslt_name";
const KEY_XML_LEAD_EXAMPLE: &str = "declaration.xml_lead_example";
const KEY_REDACT_AFTER_DAYS: &str = "declaration.redact_after_days";

const DEFAULT_CSLT_NAME: &str = "CSLT";
const DEFAULT_REDACT_AFTER_DAYS: i64 = 90;

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

// ─── Đọc PMS — CHỈ SELECT ───────────────────────────────────────────────────

/// Đường DUY NHẤT module này đọc dữ liệu của PMS. Chỉ SELECT.
pub async fn load_stays_for_declaration(pool: &Pool<Sqlite>) -> Result<Vec<StayInfo>, String> {
    let rows = sqlx::query(
        "SELECT b.id AS stay_id, r.name AS room_name, b.check_in_at,
                b.expected_checkout, b.actual_checkout
           FROM bookings b
           JOIN rooms r ON r.id = b.room_id
          WHERE b.status = 'active'
          ORDER BY b.check_in_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đọc được lượt lưu trú: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| {
            let check_in_raw: String = r.get("check_in_at");
            StayInfo {
                stay_id: r.get("stay_id"),
                room_no: strip_room_prefix(&r.get::<String, _>("room_name")),
                check_in: booking_ts_to_iso_date(&check_in_raw).unwrap_or_default(),
                expected_out: booking_ts_to_iso_date(&r.get::<String, _>("expected_checkout"))
                    .unwrap_or_default(),
                actual_out: r
                    .get::<Option<String>, _>("actual_checkout")
                    .and_then(|s| booking_ts_to_iso_date(&s)),
                check_in_raw,
            }
        })
        .collect())
}

#[derive(Debug, serde::Serialize)]
pub struct UndeclaredBreakdown {
    pub total: i64,
    /// Khách PMS đã check-in trong 48h mà CHƯA có khai báo nào của họ được
    /// đối soát khớp (`verified`) — báo động "khách đã tới, chưa ai đi khai
    /// xong". Xem `count_undeclared_within_48h` để biết vì sao chỉ trừ theo
    /// link `verified`, không trừ theo link đang quét dở.
    pub not_scanned: i64,
    pub not_exported: i64,
    pub held: i64,
    pub awaiting: i64,
}

/// Khách PMS chưa từng được khai báo XONG (đối soát khớp) trong 48h qua.
///
/// Đây là báo động gốc trước khi có khái niệm "link": khách đã check-in mà
/// chưa ai quét CCCD/hộ chiếu của họ thì badge phải kêu, kể cả khi người vận
/// hành chưa quét ai — `undeclared_breakdown` phía dưới trước đây chỉ đếm
/// link đã quét nên badge im lặng đúng lúc cần kêu nhất.
///
/// CHỈ trừ theo link đã nằm trong một lô `verified` — KHÔNG trừ theo mọi link
/// gắn với lượt lưu trú. Từng thử trừ theo mọi link để một khách vừa quét
/// khỏi bị đếm hai lần, nhưng `booking_guests` chỉ có đúng một dòng cho
/// booking đứng tên (ghi lúc tạo booking, không ai thêm khách vào một booking
/// đã tồn tại, check-in cũng không re-link) — phòng đứng tên một người mà ba
/// người ở, quét một người gán phòng là `declared_count` chạm luôn
/// `guest_count`, hai người còn lại thành vô hình trên badge dù chưa hề được
/// khai. Một khách legally-thiếu khai bị badge giấu đi là phạt hành chính
/// treo lơ lửng; badge đếm cao hơn thực tế một chút chỉ là ồn — chấp nhận
/// đếm-thừa trong lúc chờ đối soát, không bao giờ đếm-thiếu.
async fn count_undeclared_within_48h(pool: &Pool<Sqlite>) -> Result<i64, String> {
    let rows = sqlx::query(
        "SELECT
            (SELECT COUNT(*) FROM booking_guests bg WHERE bg.booking_id = b.id) AS guest_count,
            (SELECT COUNT(*) FROM declaration_link dl
               JOIN declaration_entry de  ON de.link_id = dl.id
               JOIN declaration_batch dbt ON dbt.id     = de.batch_id
              WHERE dl.stay_id = b.id AND dbt.status = 'verified') AS declared_count
           FROM bookings b
          WHERE b.status = 'active'
            AND julianday('now') - julianday(b.check_in_at) <= 2",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đếm được khách PMS chưa khai: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| {
            let g: i64 = r.get("guest_count");
            let d: i64 = r.get("declared_count");
            (g - d).max(0)
        })
        .sum())
}

/// Badge + dòng diễn giải. Bốn nguồn cộng lại, CÓ THỂ chồng lấn có chủ ý:
/// `not_scanned` (PMS, chưa có khai báo verified) + `not_exported`/`held`/
/// `awaiting` (link đã quét, chưa thuộc lô `verified`).
///
/// Một khách đã quét nhưng chưa đối soát khớp nằm trong CẢ HAI nhóm — vừa
/// `not_scanned` (PMS chưa thấy khai xong) vừa một trong ba nhóm link — nên
/// `total` đếm thừa trong cửa sổ đó. Đây là đánh đổi có chủ ý: đếm thừa chỉ
/// gây badge đọc hơi cao, còn đếm thiếu nghĩa là giấu một khách chưa khai —
/// mà thiếu một khách trong file nộp công an là bị phạt hành chính, nên thà
/// badge ồn còn hơn badge im lặng sai lúc.
pub async fn undeclared_breakdown(pool: &Pool<Sqlite>) -> Result<UndeclaredBreakdown, String> {
    let row = sqlx::query(
        "SELECT
            COALESCE(SUM(CASE WHEN has_entry = 0 AND held = 0 THEN 1 ELSE 0 END), 0) AS not_exported,
            COALESCE(SUM(CASE WHEN has_entry = 0 AND held = 1 THEN 1 ELSE 0 END), 0) AS held,
            COALESCE(SUM(CASE WHEN has_entry = 1 THEN 1 ELSE 0 END), 0) AS awaiting
           FROM (
             SELECT EXISTS(SELECT 1 FROM declaration_entry de WHERE de.link_id = dl.id) AS has_entry,
                    dl.held_at IS NOT NULL AS held
               FROM declaration_link dl
              WHERE NOT EXISTS (
                    SELECT 1 FROM declaration_entry de
                    JOIN declaration_batch b ON b.id = de.batch_id
                   WHERE de.link_id = dl.id AND b.status = 'verified')
           )",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không đếm được khách chưa khai: {e}"))?;

    let not_exported: i64 = row.get("not_exported");
    let held: i64 = row.get("held");
    let awaiting: i64 = row.get("awaiting");
    let not_scanned = count_undeclared_within_48h(pool).await?;
    Ok(UndeclaredBreakdown {
        total: not_scanned + not_exported + held + awaiting,
        not_scanned,
        not_exported,
        held,
        awaiting,
    })
}

// ─── Ghi — chỉ bốn bảng declaration_* ───────────────────────────────────────

/// Số giấy tờ định danh một con người. CCCD trước, hộ chiếu sau.
///
/// Trả `None` khi cả hai đều trống — nhập tay thiếu số giấy tờ thì không có gì
/// để nhận ra người trùng, đành để mỗi lần lưu là một dòng.
fn document_key(identity: &Identity) -> Option<String> {
    [&identity.doc_no, &identity.passport_no]
        .into_iter()
        .flatten()
        .map(|v| v.trim())
        .find(|v| !v.is_empty())
        .map(str::to_string)
}

/// Lưu một danh tính đã trích. Trả về id (uuid TEXT).
///
/// **Thả trùng một tấm giấy tờ thì dùng lại dòng cũ, không đẻ dòng mới.** Mỗi
/// lần thả ảnh sinh một danh tính riêng sẽ dẫn tới hai khai báo cùng số giấy tờ
/// cùng ngày đến, và validator chặn bằng E14 — đúng, vì nộp trùng lên cổng là
/// sai — nhưng người vận hành không còn đường nào đi tiếp. Chặn ngay từ đây rẻ
/// hơn dọn ở cuối.
///
/// Dữ liệu mới chỉ được ghi đè khi danh tính CHƯA nằm trong lô nào đã đối soát.
/// Đã nộp cho công an rồi thì bản ghi đó là bằng chứng của cái đã nộp, không
/// được sửa sau lưng.
///
/// KHÔNG có cột ảnh và KHÔNG có cột payload thô — xem §12.
pub async fn insert_identity(
    pool: &Pool<Sqlite>,
    identity: &Identity,
    source: &str,
    confidence: &str,
) -> Result<String, String> {
    if let Some(key) = document_key(identity) {
        let existing = sqlx::query(
            "SELECT id,
                    EXISTS (SELECT 1
                              FROM declaration_link dl
                              JOIN declaration_entry de  ON de.link_id = dl.id
                              JOIN declaration_batch dbt ON dbt.id     = de.batch_id
                             WHERE dl.identity_id = di.id
                               AND dbt.status = 'verified') AS already_declared
               FROM declaration_identity di
              WHERE redacted_at IS NULL
                AND (doc_no = ? OR passport_no = ?)
              ORDER BY created_at
              LIMIT 1",
        )
        .bind(&key)
        .bind(&key)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Không tra được danh tính đã có: {e}"))?;

        if let Some(row) = existing {
            let existing_id: String = row.get("id");
            if row.get::<i64, _>("already_declared") == 0 {
                update_identity_fields(pool, &existing_id, identity, source, confidence).await?;
            }
            return Ok(existing_id);
        }
    }

    let id = if identity.id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        identity.id.clone()
    };

    sqlx::query(
        "INSERT INTO declaration_identity (
            id, source, extract_confidence, full_name, dob, gender, nationality_iso3,
            doc_type_code, doc_type_source, doc_type_name, doc_no, phone,
            residence_status, address_detail, passport_no, passport_expiry,
            visa_valid_until, name_confirmed_by_human, single_token_name_ok, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(source)
    .bind(confidence)
    .bind(&identity.full_name)
    .bind(&identity.dob)
    .bind(&identity.gender)
    .bind(&identity.nationality_iso3)
    .bind(&identity.doc_type_code)
    .bind(&identity.doc_type_source)
    .bind(&identity.doc_type_name)
    .bind(&identity.doc_no)
    .bind(&identity.phone)
    .bind(&identity.residence_status)
    .bind(&identity.address_detail)
    .bind(&identity.passport_no)
    .bind(&identity.passport_expiry)
    .bind(&identity.visa_valid_until)
    .bind(i64::from(identity.name_confirmed_by_human))
    .bind(i64::from(identity.single_token_name_ok))
    .bind(now())
    .execute(pool)
    .await
    .map_err(|e| format!("Không lưu được danh tính: {e}"))?;

    Ok(id)
}

/// Lưu danh tính VÀ bảo đảm nó có mặt trong danh sách chờ khai — "băng chuyền
/// một chiều": thả ảnh xong là khách hiện ra, không có khu chờ trung gian.
///
/// Link "đang hoạt động" = chưa nằm trong lô `verified`. Còn một link như vậy
/// thì lần quét này là quét lại trong cùng lượt ở — dùng lại, không đẻ thêm.
/// Mọi link đều đã verified (khách quay lại sau lượt ở trước) thì lượt mới
/// cần link mới.
pub async fn save_identity_ensuring_link(
    pool: &Pool<Sqlite>,
    identity: &Identity,
    source: &str,
    confidence: &str,
) -> Result<String, String> {
    let identity_id = insert_identity(pool, identity, source, confidence).await?;

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM declaration_link dl
          WHERE dl.identity_id = ?
            AND NOT EXISTS (
                  SELECT 1 FROM declaration_entry de
                  JOIN declaration_batch b ON b.id = de.batch_id
                 WHERE de.link_id = dl.id AND b.status = 'verified')",
    )
    .bind(&identity_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không kiểm được khai báo đang hoạt động: {e}"))?;

    if active == 0 {
        insert_link(pool, &identity_id, None, "1", None).await?;
    }
    Ok(identity_id)
}

/// Cập nhật một danh tính đã có bằng lần trích mới nhất.
///
/// Giữ nguyên `id` và `created_at`: link đang trỏ vào id đó, và `created_at` là
/// lúc người này lần đầu được đưa vào hệ thống.
async fn update_identity_fields(
    pool: &Pool<Sqlite>,
    id: &str,
    identity: &Identity,
    source: &str,
    confidence: &str,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE declaration_identity SET
            source = ?, extract_confidence = ?, full_name = ?, dob = ?, gender = ?,
            nationality_iso3 = ?, doc_type_code = ?, doc_type_source = ?, doc_type_name = ?,
            doc_no = ?, phone = ?, residence_status = ?, address_detail = ?,
            passport_no = ?, passport_expiry = ?, visa_valid_until = ?,
            name_confirmed_by_human = ?, single_token_name_ok = ?
          WHERE id = ?",
    )
    .bind(source)
    .bind(confidence)
    .bind(&identity.full_name)
    .bind(&identity.dob)
    .bind(&identity.gender)
    .bind(&identity.nationality_iso3)
    .bind(&identity.doc_type_code)
    .bind(&identity.doc_type_source)
    .bind(&identity.doc_type_name)
    .bind(&identity.doc_no)
    .bind(&identity.phone)
    .bind(&identity.residence_status)
    .bind(&identity.address_detail)
    .bind(&identity.passport_no)
    .bind(&identity.passport_expiry)
    .bind(&identity.visa_valid_until)
    .bind(i64::from(identity.name_confirmed_by_human))
    .bind(i64::from(identity.single_token_name_ok))
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| format!("Không cập nhật được danh tính: {e}"))?;

    Ok(())
}

/// Sửa một danh tính theo id (form sửa của thẻ khách). Khác `insert_identity`
/// ở chỗ không merge theo số giấy tờ — sửa thẳng theo `identity_id`. Cần đường
/// này vì danh tính nhập tay thiếu đúng số giấy tờ để merge thì
/// `insert_identity` sẽ đẻ dòng mới thay vì sửa dòng cũ.
///
/// **Luật hẹp, không phải "có link verified nào thì chặn hết":** cái đã nộp
/// công an được giữ bằng chứng ở file xuất trên đĩa cộng dòng
/// `declaration_batch`/`declaration_entry` — dòng `declaration_identity` có
/// thể đổi thì KHÔNG phải bằng chứng đó. Khách nước ngoài ở tháng 3 (đã khai,
/// đã verified) quay lại tháng 7 với visa mới thì tái sử dụng đúng dòng danh
/// tính, còn hạn visa cũ vẫn nằm đó và validator chặn E09 — đường sửa duy nhất
/// là "bấm lỗi → sửa form → lưu", tức gọi đúng hàm này. Vì vậy chỉ chặn khi
/// KHÔNG còn link nào sống ngoài lô verified: còn một link sống nghĩa là còn
/// một lượt lưu trú đang chờ khai, và chính lượt đó cần được sửa.
pub async fn update_identity(
    pool: &Pool<Sqlite>,
    identity_id: &str,
    identity: &Identity,
    source: &str,
    confidence: &str,
) -> Result<(), String> {
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM declaration_link dl
          WHERE dl.identity_id = ?
            AND NOT EXISTS (
                  SELECT 1 FROM declaration_entry de
                  JOIN declaration_batch b ON b.id = de.batch_id
                 WHERE de.link_id = dl.id AND b.status = 'verified')",
    )
    .bind(identity_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không kiểm được lịch sử khai của danh tính: {e}"))?;

    if live == 0 {
        return Err(
            "Khách này chỉ còn khai báo đã đối soát — thông tin cũ là bằng chứng, không sửa được."
                .into(),
        );
    }

    update_identity_fields(pool, identity_id, identity, source, confidence).await
}

/// Link đã nằm trong lô `verified` = bằng chứng đã khai lên cổng.
async fn link_is_declared(pool: &Pool<Sqlite>, link_id: &str) -> Result<bool, String> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM declaration_entry de
           JOIN declaration_batch dbt ON dbt.id = de.batch_id
          WHERE de.link_id = ? AND dbt.status = 'verified'",
    )
    .bind(link_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không kiểm được lô của khai báo: {e}"))?;
    Ok(n > 0)
}

/// Sửa phòng / lý do / ghi chú của một khai báo tại chỗ (thẻ khách của UI mới).
pub async fn update_link(
    pool: &Pool<Sqlite>,
    link_id: &str,
    stay_id: Option<&str>,
    stay_reason: &str,
    note: Option<&str>,
) -> Result<(), String> {
    if link_is_declared(pool, link_id).await? {
        return Err(
            "Khai báo này đã nằm trong một lô đã đối soát — không sửa được, vì đó là bằng chứng đã khai."
                .into(),
        );
    }

    let affected = sqlx::query(
        "UPDATE declaration_link
            SET stay_id = ?, stay_reason = ?, stay_reason_note = ?
          WHERE id = ?",
    )
    .bind(stay_id)
    .bind(stay_reason)
    .bind(note)
    .bind(link_id)
    .execute(pool)
    .await
    .map_err(|e| {
        // UNIQUE(identity_id, stay_id): khách này đã có khai báo cho đúng phòng đó.
        if e.to_string().contains("UNIQUE") {
            "Khách này đã có một khai báo cho lượt lưu trú đó rồi.".to_string()
        } else {
            format!("Không sửa được khai báo: {e}")
        }
    })?
    .rows_affected();

    if affected == 0 {
        return Err("Không tìm thấy khai báo cần sửa.".into());
    }
    Ok(())
}

/// "Xóa" trên thẻ khách: gỡ link, và nếu danh tính không còn link nào khác thì
/// xóa luôn danh tính — không để lại bản ghi mồ côi vô hình.
pub async fn discard_link(pool: &Pool<Sqlite>, link_id: &str) -> Result<(), String> {
    if link_is_declared(pool, link_id).await? {
        return Err(
            "Khai báo này đã nằm trong một lô đã đối soát — không xóa được, vì đó là bằng chứng đã khai."
                .into(),
        );
    }

    let identity_id: Option<String> =
        sqlx::query_scalar("SELECT identity_id FROM declaration_link WHERE id = ?")
            .bind(link_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("Không đọc được khai báo: {e}"))?;
    let Some(identity_id) = identity_id else {
        return Err("Không tìm thấy khai báo cần xóa.".into());
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Không mở được giao dịch: {e}"))?;

    // FK: entry (lô chưa đối soát) đi trước, rồi link, rồi danh tính nếu mồ côi.
    sqlx::query("DELETE FROM declaration_entry WHERE link_id = ?")
        .bind(link_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Không gỡ được dòng khỏi lô: {e}"))?;
    sqlx::query("DELETE FROM declaration_link WHERE id = ?")
        .bind(link_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Không gỡ được khai báo: {e}"))?;
    sqlx::query(
        "DELETE FROM declaration_identity
          WHERE id = ?
            AND NOT EXISTS (SELECT 1 FROM declaration_link dl WHERE dl.identity_id = ?)",
    )
    .bind(&identity_id)
    .bind(&identity_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Không xóa được danh tính: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("Không lưu được thay đổi: {e}"))
}

/// Gác một khai báo sang một bên (held = true) hoặc đưa lại (false).
/// Khách gác lại không vào file xuất nhưng badge vẫn đếm — họ chưa được khai.
pub async fn set_link_held(pool: &Pool<Sqlite>, link_id: &str, held: bool) -> Result<(), String> {
    let held_at = if held { Some(now()) } else { None };
    let affected = sqlx::query("UPDATE declaration_link SET held_at = ? WHERE id = ?")
        .bind(held_at)
        .bind(link_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Không cập nhật được trạng thái gác: {e}"))?
        .rows_affected();
    if affected == 0 {
        return Err("Không tìm thấy khai báo.".into());
    }
    Ok(())
}

/// Ghép một danh tính với một lượt lưu trú.
///
/// `stay_id` = `bookings.id` nhưng KHÔNG có FK cứng (§5.2). Gọi lại với cùng
/// cặp (identity, stay) thì cập nhật lý do lưu trú và trả về đúng link cũ —
/// không đẻ thêm dòng, vì `UNIQUE(identity_id, stay_id)`.
/// `stay_id = None` khi khai báo chưa gắn vào lượt lưu trú nào (chưa xác định
/// phòng) — xem migration v21.
pub async fn insert_link(
    pool: &Pool<Sqlite>,
    identity_id: &str,
    stay_id: Option<&str>,
    stay_reason: &str,
    note: Option<&str>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO declaration_link (
            id, identity_id, stay_id, stay_reason, stay_reason_note, created_at
         ) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(identity_id, stay_id) DO UPDATE SET
            stay_reason      = excluded.stay_reason,
            stay_reason_note = excluded.stay_reason_note",
    )
    .bind(&id)
    .bind(identity_id)
    .bind(stay_id)
    .bind(stay_reason)
    .bind(note)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|e| format!("Không ghép được danh tính với lượt lưu trú: {e}"))?;

    // `IS` chứ không phải `=`: với stay_id NULL thì `= NULL` không bao giờ đúng
    // và câu này sẽ không tìm thấy chính dòng vừa ghi.
    //
    // `ORDER BY rowid DESC LIMIT 1`: một danh tính có thể có HAI link cùng
    // `stay_id NULL` (một đã verified, một vừa tạo) — không có ràng buộc nào
    // chặn việc đó, vì `UNIQUE(identity_id, stay_id)` chỉ chặn khi cả hai vế
    // giống hệt SQLite coi là bằng nhau, còn khách quay lại sau khi đã khai
    // xong lượt trước lại cần một link NULL khác. `rowid` tăng dần theo INSERT
    // nên dòng mới nhất luôn thắng; `created_at` cùng giây thì không phân định
    // được.
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM declaration_link
          WHERE identity_id = ? AND stay_id IS ?
          ORDER BY rowid DESC LIMIT 1",
    )
    .bind(identity_id)
    .bind(stay_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không đọc lại được link vừa ghép: {e}"))
}

/// Một lô vừa xuất file. Trạng thái khởi tạo luôn là `exported` — chỉ vòng
/// đối chiếu (§10) mới được đưa nó lên `verified`.
pub async fn insert_batch(
    pool: &Pool<Sqlite>,
    kind: &str,
    file_path: &str,
    row_count: i64,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO declaration_batch (
            id, kind, file_path, row_count, status, created_at
         ) VALUES (?, ?, ?, ?, 'exported', ?)",
    )
    .bind(&id)
    .bind(kind)
    .bind(file_path)
    .bind(row_count)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|e| format!("Không lưu được lô khai báo: {e}"))?;

    Ok(id)
}

/// Các dòng của một lô, `row_index` theo đúng thứ tự ghi ra file.
pub async fn insert_entries(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    link_ids: &[String],
) -> Result<(), String> {
    if link_ids.is_empty() {
        return Ok(());
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Không mở được giao dịch: {e}"))?;

    for (index, link_id) in link_ids.iter().enumerate() {
        sqlx::query("INSERT INTO declaration_entry (batch_id, link_id, row_index) VALUES (?, ?, ?)")
            .bind(batch_id)
            .bind(link_id)
            .bind(index as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("Không lưu được dòng của lô: {e}"))?;
    }

    tx.commit()
        .await
        .map_err(|e| format!("Không chốt được giao dịch: {e}"))
}

/// Số dòng app đã ghi ra file của lô này. Vòng đối chiếu so con số này với số
/// record cổng báo đã nhận.
pub async fn batch_row_count(pool: &Pool<Sqlite>, batch_id: &str) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>("SELECT row_count FROM declaration_batch WHERE id = ?")
        .bind(batch_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Không đọc được số dòng của lô: {e}"))
}

/// Cổng đã nhận đủ. Từ giây phút này khách trong lô mới được tính là đã khai.
pub async fn set_batch_verified(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    seen: i64,
) -> Result<(), String> {
    set_batch_outcome(pool, batch_id, "verified", seen).await
}

/// Cổng nhận thiếu hoặc không nhận. Khách vẫn nằm ngoài danh sách chờ (tránh
/// ghi vào hai file xuất khác nhau), cần `kbtt_reopen_batch` để quay lại.
pub async fn set_batch_failed(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    seen: i64,
) -> Result<(), String> {
    set_batch_outcome(pool, batch_id, "failed", seen).await
}

/// Mở lại một lô `failed`: gỡ entry để khách quay về danh sách chờ, đưa lô
/// sang trạng thái chót `reopened`. Chỉ cho lô `failed` — `exported` phải qua
/// đối chiếu trước, `verified` là bằng chứng đã khai, và `reopened` chính nó
/// không mở lại lần hai được (không phải cửa quay vòng).
///
/// `reopened` là trạng thái RIÊNG, không phải quay lại `failed`: nếu để
/// nguyên `status = 'failed'` sau khi gỡ entry, `kbtt_list_batches` vẫn trả về
/// lô này và `ReconcilePanel` vẫn dựng thẻ đỏ cho nó — một thẻ ma không bao
/// giờ biến mất dù khách đã được sửa, xuất lại và đối chiếu khớp dưới một lô
/// MỚI. `list_batches` xếp `reopened` chung nhóm với `verified` ở cuối danh
/// sách vì cả hai đều đã xong việc.
///
/// Đọc trạng thái rồi xóa+cập nhật nằm chung một `pool.begin()` (như
/// `discard_link`): nếu tách câu lệnh, một lượt đối soát chen
/// vào giữa có thể chốt lô này `verified` ngay sau khi ta vừa đọc "failed", và
/// các câu ghi đứng riêng sẽ xóa mất entry của một lô giờ đã là bằng chứng đã
/// khai. Gộp chung transaction thì SQLite giữ nguyên ảnh chụp lúc đọc cho tới
/// khi commit — có tranh chấp ghi thật thì commit lỗi thay vì âm thầm xóa
/// nhầm.
pub async fn reopen_failed_batch(pool: &Pool<Sqlite>, batch_id: &str) -> Result<(), String> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Không mở được giao dịch: {e}"))?;

    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM declaration_batch WHERE id = ?")
            .bind(batch_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| format!("Không đọc được lô: {e}"))?;

    match status.as_deref() {
        None => return Err("Không tìm thấy lô.".into()),
        Some("failed") => {}
        Some(_) => {
            return Err("Chỉ mở lại được lô đã đối chiếu lệch (failed).".into());
        }
    }

    sqlx::query("DELETE FROM declaration_entry WHERE batch_id = ?")
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Không gỡ được khách khỏi lô: {e}"))?;

    sqlx::query("UPDATE declaration_batch SET status = 'reopened' WHERE id = ?")
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Không cập nhật được trạng thái lô: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("Không lưu được thay đổi: {e}"))
}

async fn set_batch_outcome(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    status: &str,
    seen: i64,
) -> Result<(), String> {
    let affected = sqlx::query(
        "UPDATE declaration_batch
            SET status = ?, verified_count = ?, verified_at = ?
          WHERE id = ?",
    )
    .bind(status)
    .bind(seen)
    .bind(now())
    .bind(batch_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Không cập nhật được trạng thái lô: {e}"))?
    .rows_affected();

    if affected == 0 {
        return Err(format!("Không thấy lô khai báo {batch_id}"));
    }
    Ok(())
}

// ─── Đọc để dựng dòng khai báo ──────────────────────────────────────────────

/// Dựng `DeclarationRow` đầy đủ: join `declaration_link` + `declaration_identity`,
/// rồi ghép `StayInfo` tương ứng từ `load_stays_for_declaration`.
///
/// Booking đã bị PMS xóa thì dòng vẫn trả về với `StayInfo` rỗng (chỉ có
/// `stay_id`) — validator sẽ chặn, còn lịch sử lô thì không được biến mất.
pub async fn load_rows_by_link_ids(
    pool: &Pool<Sqlite>,
    link_ids: &[String],
) -> Result<Vec<DeclarationRow>, String> {
    if link_ids.is_empty() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT dl.id AS link_id, dl.stay_id, dl.stay_reason, dl.stay_reason_note,
                di.id AS identity_id, di.full_name, di.dob, di.gender, di.nationality_iso3,
                di.doc_type_code, di.doc_type_source, di.doc_type_name, di.doc_no, di.phone,
                di.residence_status, di.address_detail, di.passport_no, di.passport_expiry,
                di.visa_valid_until, di.name_confirmed_by_human, di.single_token_name_ok
           FROM declaration_link dl
           JOIN declaration_identity di ON di.id = dl.identity_id
          WHERE dl.id IN ({})",
        placeholders(link_ids.len())
    );

    let mut query = sqlx::query(&sql);
    for id in link_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Không đọc được dòng khai báo: {e}"))?;

    let stays: HashMap<String, StayInfo> = load_stays_for_declaration(pool)
        .await?
        .into_iter()
        .map(|s| (s.stay_id.clone(), s))
        .collect();

    let mut by_link: HashMap<String, DeclarationRow> = HashMap::new();
    for r in rows.iter() {
        let link_id: String = r.get("link_id");
        // NULL = khai báo chưa gắn phòng (v21). Cũng rơi vào đây khi booking đã
        // bị PMS xóa — link cố ý không có FK cứng.
        let stay = match r.get::<Option<String>, _>("stay_id") {
            Some(stay_id) => stays.get(&stay_id).cloned().unwrap_or(StayInfo {
                stay_id,
                ..Default::default()
            }),
            None => StayInfo::default(),
        };

        by_link.insert(
            link_id.clone(),
            DeclarationRow {
                link_id,
                identity: Identity {
                    id: r.get("identity_id"),
                    full_name: r.get("full_name"),
                    dob: r.get("dob"),
                    gender: r.get("gender"),
                    nationality_iso3: r.get("nationality_iso3"),
                    doc_type_code: r.get("doc_type_code"),
                    doc_type_source: r.get("doc_type_source"),
                    doc_type_name: r.get("doc_type_name"),
                    doc_no: r.get("doc_no"),
                    phone: r.get("phone"),
                    residence_status: r.get("residence_status"),
                    address_detail: r.get("address_detail"),
                    passport_no: r.get("passport_no"),
                    passport_expiry: r.get("passport_expiry"),
                    visa_valid_until: r.get("visa_valid_until"),
                    name_confirmed_by_human: r.get::<i64, _>("name_confirmed_by_human") != 0,
                    single_token_name_ok: r.get::<i64, _>("single_token_name_ok") != 0,
                },
                stay,
                stay_reason: r.get("stay_reason"),
                stay_reason_note: r.get("stay_reason_note"),
            },
        );
    }

    // Giữ đúng thứ tự người gọi đưa vào — thứ tự đó là thứ tự dòng trong file.
    Ok(link_ids
        .iter()
        .filter_map(|id| by_link.remove(id))
        .collect())
}

/// Trả kèm `extract_confidence` để lớp command sinh W05 (`DeclarationRow`
/// không mang cột này).
pub async fn confidence_by_link(
    pool: &Pool<Sqlite>,
    link_ids: &[String],
) -> Result<HashMap<String, String>, String> {
    if link_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "SELECT dl.id AS link_id, di.extract_confidence
           FROM declaration_link dl
           JOIN declaration_identity di ON di.id = dl.identity_id
          WHERE dl.id IN ({})",
        placeholders(link_ids.len())
    );

    let mut query = sqlx::query(&sql);
    for id in link_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Không đọc được độ tin cậy: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| (r.get("link_id"), r.get("extract_confidence")))
        .collect())
}

/// "Chưa khai báo" của băng chuyền: link chưa từng được xuất (không có entry
/// nào). Đã xuất — kể cả lô sau đó fail — thì sống trên thẻ đối chiếu, không
/// quay lại đây để tránh xuất trùng.
pub async fn pending_link_ids(pool: &Pool<Sqlite>) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>(
        "SELECT dl.id
           FROM declaration_link dl
          WHERE NOT EXISTS (SELECT 1 FROM declaration_entry de WHERE de.link_id = dl.id)
          ORDER BY dl.created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đọc được danh sách chờ khai: {e}"))
}

/// Cờ "gác lại" của từng link — DTO cần nó, `DeclarationRow` (model) không mang.
pub async fn held_by_link(
    pool: &Pool<Sqlite>,
    link_ids: &[String],
) -> Result<HashMap<String, bool>, String> {
    if link_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT id, held_at FROM declaration_link WHERE id IN ({})",
        placeholders(link_ids.len())
    );
    let mut query = sqlx::query(&sql);
    for id in link_ids {
        query = query.bind(id);
    }
    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Không đọc được trạng thái gác: {e}"))?;
    Ok(rows
        .iter()
        .map(|r| (r.get("id"), r.get::<Option<String>, _>("held_at").is_some()))
        .collect())
}

#[derive(Debug, serde::Serialize)]
pub struct BatchSummary {
    pub id: String,
    pub kind: String,
    pub file_path: String,
    pub row_count: i64,
    pub status: String,
    pub verified_count: Option<i64>,
    pub verified_at: Option<String>,
    pub created_at: String,
}

/// Lô chưa xong nổi lên đầu: `failed` rồi `exported`/`uploaded`, sau đó mới tới
/// lô đã xong việc — `verified` (đối chiếu khớp) và `reopened` (đã mở lại, khách
/// về danh sách để sửa) — đều rơi vào nhánh `ELSE` bên dưới. Người vận hành cần
/// thấy cái đang hỏng trước, không phải lô nào đã yên rồi.
pub async fn list_batches(pool: &Pool<Sqlite>) -> Result<Vec<BatchSummary>, String> {
    let rows = sqlx::query(
        "SELECT id, kind, file_path, row_count, status,
                verified_count, verified_at, created_at
           FROM declaration_batch
          ORDER BY CASE status
                     WHEN 'failed'   THEN 0
                     WHEN 'exported' THEN 1
                     WHEN 'uploaded' THEN 1
                     ELSE 2 -- 'verified' và 'reopened': lô đã xong việc
                   END,
                   created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đọc được lịch sử lô: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| BatchSummary {
            id: r.get("id"),
            kind: r.get("kind"),
            file_path: r.get("file_path"),
            row_count: r.get("row_count"),
            status: r.get("status"),
            verified_count: r.get("verified_count"),
            verified_at: r.get("verified_at"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn batch_file_path(pool: &Pool<Sqlite>, batch_id: &str) -> Result<String, String> {
    sqlx::query_scalar::<_, String>("SELECT file_path FROM declaration_batch WHERE id = ?")
        .bind(batch_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Không đọc được lô: {e}"))?
        .ok_or_else(|| "Không tìm thấy lô.".to_string())
}

// ─── Bốn khóa settings (§4.2) ───────────────────────────────────────────────

async fn setting(pool: &Pool<Sqlite>, key: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ? LIMIT 1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Không đọc được cấu hình {key}: {e}"))
}

pub async fn export_dir(pool: &Pool<Sqlite>) -> Result<PathBuf, String> {
    match setting(pool, KEY_EXPORT_DIR).await? {
        Some(v) if !v.trim().is_empty() => Ok(PathBuf::from(v.trim())),
        _ => Ok(app_identity::exports_dir().join("khai-bao-tam-tru")),
    }
}

pub async fn cslt_name(pool: &Pool<Sqlite>) -> Result<String, String> {
    match setting(pool, KEY_CSLT_NAME).await? {
        Some(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => Ok(DEFAULT_CSLT_NAME.to_string()),
    }
}

pub async fn xml_lead_example(pool: &Pool<Sqlite>) -> Result<bool, String> {
    Ok(matches!(
        setting(pool, KEY_XML_LEAD_EXAMPLE)
            .await?
            .as_deref()
            .map(str::trim),
        Some("true") | Some("1")
    ))
}

pub async fn redact_after_days(pool: &Pool<Sqlite>) -> Result<i64, String> {
    Ok(setting(pool, KEY_REDACT_AFTER_DAYS)
        .await?
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|d| *d > 0)
        .unwrap_or(DEFAULT_REDACT_AFTER_DAYS))
}

// ─── §12.5 — che, KHÔNG xóa ─────────────────────────────────────────────────

/// Che dữ liệu cá nhân của những danh tính đã khai xong từ lâu.
///
/// CHE chứ không XÓA. Xóa dòng sẽ phá `declaration_link` -> `declaration_entry`
/// -> lịch sử lô, tức là mất bằng chứng "khách này đã khai ngày nào, lô nào" —
/// mà đó chính là thứ cần giữ khi có ai hỏi.
///
/// Chỉ che khi MỌI link của danh tính đều đã thuộc một lô `verified` cũ hơn
/// `after_days` ngày. Một link còn chờ đối chiếu là chưa che.
///
/// Trả về số dòng đã che.
pub async fn redact_old_identities(pool: &Pool<Sqlite>, after_days: i64) -> Result<u64, String> {
    let affected = sqlx::query(
        "UPDATE declaration_identity
            SET full_name      = '',
                dob            = '',
                doc_no         = NULL,
                passport_no    = NULL,
                address_detail = NULL,
                phone          = NULL,
                redacted_at    = ?
          WHERE redacted_at IS NULL
            AND EXISTS (SELECT 1 FROM declaration_link dl
                         WHERE dl.identity_id = declaration_identity.id)
            AND NOT EXISTS (
                  SELECT 1 FROM declaration_link dl
                   WHERE dl.identity_id = declaration_identity.id
                     AND NOT EXISTS (
                           SELECT 1
                             FROM declaration_entry de
                             JOIN declaration_batch b ON b.id = de.batch_id
                            WHERE de.link_id = dl.id
                              AND b.status = 'verified'
                              AND b.verified_at IS NOT NULL
                              AND julianday('now') - julianday(b.verified_at) >= ?
                         )
                )",
    )
    .bind(now())
    .bind(after_days)
    .execute(pool)
    .await
    .map_err(|e| format!("Không che được dữ liệu cũ: {e}"))?
    .rows_affected();

    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        crate::db::run_migrations(&pool)
            .await
            .expect("runs migrations");
        pool
    }

    fn vn_identity() -> Identity {
        Identity {
            full_name: "Phan Thị Mỹ Hà".into(),
            dob: "1995-07-28".into(),
            gender: "F".into(),
            nationality_iso3: "VNM".into(),
            doc_type_code: Some("1".into()),
            doc_type_source: Some("heuristic".into()),
            doc_no: Some("058195006173".into()),
            phone: Some("0901234567".into()),
            address_detail: Some("KP6, Mỹ Đông, Phan Rang-Tháp Chàm, Ninh Thuận".into()),
            ..Default::default()
        }
    }

    /// Nguyên tắc bao trùm của module: PMS đang vận hành thật, không được
    /// migrate hay ghi vào nó vì một tính năng phụ. Test này đọc source và
    /// bắt mọi câu ghi chạm bảng cũ.
    #[test]
    fn declaration_module_never_writes_to_legacy_tables() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/declaration");
        let legacy = ["guests", "bookings", "booking_guests", "rooms"];
        let writes = ["insert into", "update ", "delete from", "alter table"];

        let mut offences = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap().to_lowercase();
                for stmt in writes {
                    for (idx, _) in text.match_indices(stmt) {
                        let window = &text[idx..text.len().min(idx + 120)];
                        for table in legacy {
                            let hit = window.contains(&format!(" {table} "))
                                || window.contains(&format!(" {table}("))
                                || window.contains(&format!(" {table}\n"));
                            if hit {
                                offences.push(format!("{}: {stmt} ... {table}", p.display()));
                            }
                        }
                    }
                }
            }
        }

        assert!(
            offences.is_empty(),
            "Module khai báo ghi vào bảng của PMS: {offences:#?}"
        );
    }

    /// §12 — không lưu ảnh, không lưu payload thô.
    ///
    /// Chỉ quét phần production của mỗi file (cắt tại `#[cfg(test)]`): tên của
    /// chính test này chứa từ cấm, mà test quét CẢ `src/declaration/`, kể cả
    /// `repo.rs`. Cột và schema chỉ sống ở phần production nên cắt như vậy
    /// không làm mất hiệu lực của luật.
    #[test]
    fn declaration_module_stores_no_images_or_raw_payloads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/declaration");
        let banned = ["photo_path", "raw_payload"];
        let mut offences = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&p).unwrap();
                let text = match source.find("#[cfg(test)]") {
                    Some(cut) => &source[..cut],
                    None => source.as_str(),
                };
                for word in banned {
                    if text.contains(word) {
                        offences.push(format!("{}: {word}", p.display()));
                    }
                }
            }
        }
        assert!(offences.is_empty(), "Vi phạm §12: {offences:#?}");
    }

    #[tokio::test]
    async fn identity_round_trips_through_link_lookup() {
        let pool = pool().await;

        let identity_id = insert_identity(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");
        let link_id = insert_link(&pool, &identity_id, Some("booking-1"), "2", None)
            .await
            .expect("ghép link");

        let rows = load_rows_by_link_ids(&pool, std::slice::from_ref(&link_id))
            .await
            .expect("đọc dòng");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].link_id, link_id);
        assert_eq!(rows[0].identity.full_name, "Phan Thị Mỹ Hà");
        assert_eq!(rows[0].identity.doc_no.as_deref(), Some("058195006173"));
        assert_eq!(rows[0].stay_reason, "2");
        // Booking chưa tồn tại trong PMS: dòng vẫn về, stay rỗng để validator chặn.
        assert_eq!(rows[0].stay.stay_id, "booking-1");
        assert!(rows[0].stay.room_no.is_empty());

        let conf = confidence_by_link(&pool, std::slice::from_ref(&link_id))
            .await
            .expect("đọc độ tin cậy");
        assert_eq!(conf.get(&link_id).map(String::as_str), Some("verified"));
    }

    /// `UNIQUE(identity_id, stay_id)`: quét lại cùng một khách cho cùng một
    /// lượt lưu trú không được đẻ ra dòng thứ hai.
    #[tokio::test]
    async fn relinking_the_same_stay_updates_instead_of_duplicating() {
        let pool = pool().await;
        let identity_id = insert_identity(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");

        let first = insert_link(&pool, &identity_id, Some("booking-1"), "1", None)
            .await
            .expect("ghép lần đầu");
        let second = insert_link(&pool, &identity_id, Some("booking-1"), "20", Some("Đi công tác"))
            .await
            .expect("ghép lại");

        assert_eq!(first, second, "phải là cùng một link");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link")
            .fetch_one(&pool)
            .await
            .expect("đếm link");
        assert_eq!(count, 1);

        let rows = load_rows_by_link_ids(&pool, &[second]).await.expect("đọc");
        assert_eq!(rows[0].stay_reason, "20");
        assert_eq!(rows[0].stay_reason_note.as_deref(), Some("Đi công tác"));
    }

    #[tokio::test]
    async fn rows_come_back_in_the_order_they_were_asked_for() {
        let pool = pool().await;
        let mut links = Vec::new();
        for n in 0..3 {
            let identity_id = insert_identity(&pool, &vn_identity(), "manual", "needs_review")
                .await
                .expect("lưu danh tính");
            links.push(
                insert_link(&pool, &identity_id, Some(&format!("booking-{n}")), "1", None)
                    .await
                    .expect("ghép link"),
            );
        }
        links.reverse();

        let rows = load_rows_by_link_ids(&pool, &links).await.expect("đọc");
        let got: Vec<String> = rows.iter().map(|r| r.link_id.clone()).collect();
        assert_eq!(got, links, "thứ tự dòng là thứ tự ghi ra file");
    }

    #[tokio::test]
    async fn batch_lifecycle_records_what_the_portal_reported() {
        let pool = pool().await;
        let identity_id = insert_identity(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");
        let link_id = insert_link(&pool, &identity_id, Some("booking-1"), "2", None)
            .await
            .expect("ghép link");

        let batch = insert_batch(&pool, "VN", "/tmp/kbtt.xlsx", 1)
            .await
            .expect("lưu lô");
        insert_entries(&pool, &batch, &[link_id])
            .await
            .expect("lưu dòng của lô");

        assert_eq!(batch_row_count(&pool, &batch).await.unwrap(), 1);

        let status: String = sqlx::query_scalar("SELECT status FROM declaration_batch WHERE id = ?")
            .bind(&batch)
            .fetch_one(&pool)
            .await
            .expect("đọc trạng thái");
        assert_eq!(status, "exported", "xuất file chưa phải là đã khai");

        set_batch_verified(&pool, &batch, 1).await.expect("verified");
        let (status, seen): (String, i64) =
            sqlx::query_as("SELECT status, verified_count FROM declaration_batch WHERE id = ?")
                .bind(&batch)
                .fetch_one(&pool)
                .await
                .expect("đọc lại");
        assert_eq!(status, "verified");
        assert_eq!(seen, 1);

        // F2: cổng báo "thành công" cả khi nhận 0 record — người vận hành đếm
        // được 0 thì lô phải thành 'failed'.
        set_batch_failed(&pool, &batch, 0).await.expect("failed");
        let (status, seen): (String, i64) =
            sqlx::query_as("SELECT status, verified_count FROM declaration_batch WHERE id = ?")
                .bind(&batch)
                .fetch_one(&pool)
                .await
                .expect("đọc lại");
        assert_eq!(status, "failed");
        assert_eq!(seen, 0);

        assert!(
            set_batch_verified(&pool, "khong-co-lo-nay", 1).await.is_err(),
            "lô không tồn tại phải báo lỗi chứ không im lặng"
        );
    }

    #[tokio::test]
    async fn settings_fall_back_to_documented_defaults() {
        let pool = pool().await;

        assert_eq!(cslt_name(&pool).await.unwrap(), "CSLT");
        assert!(!xml_lead_example(&pool).await.unwrap());
        assert_eq!(redact_after_days(&pool).await.unwrap(), 90);
        assert!(export_dir(&pool)
            .await
            .unwrap()
            .ends_with("exports/khai-bao-tam-tru"));

        for (key, value) in [
            (KEY_CSLT_NAME, "Nhà trọ Bình An"),
            (KEY_XML_LEAD_EXAMPLE, "true"),
            (KEY_REDACT_AFTER_DAYS, "30"),
            (KEY_EXPORT_DIR, "/tmp/kbtt-export"),
        ] {
            sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
                .bind(key)
                .bind(value)
                .execute(&pool)
                .await
                .expect("ghi cấu hình");
        }

        assert_eq!(cslt_name(&pool).await.unwrap(), "Nhà trọ Bình An");
        assert!(xml_lead_example(&pool).await.unwrap());
        assert_eq!(redact_after_days(&pool).await.unwrap(), 30);
        assert_eq!(
            export_dir(&pool).await.unwrap(),
            PathBuf::from("/tmp/kbtt-export")
        );
    }

    /// §12.5 — che giữ được quan hệ, bỏ được dữ liệu cá nhân.
    #[tokio::test]
    async fn redaction_masks_personal_data_but_keeps_the_paper_trail() {
        let pool = pool().await;
        let identity_id = insert_identity(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");
        let link_id = insert_link(&pool, &identity_id, Some("booking-1"), "2", None)
            .await
            .expect("ghép link");
        let batch = insert_batch(&pool, "VN", "/tmp/kbtt.xlsx", 1)
            .await
            .expect("lưu lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link_id))
            .await
            .expect("lưu dòng");

        // Lô mới đối chiếu xong hôm nay: chưa đến hạn che.
        set_batch_verified(&pool, &batch, 1).await.expect("verified");
        assert_eq!(redact_old_identities(&pool, 90).await.unwrap(), 0);

        // Đẩy ngày đối chiếu lùi 200 ngày.
        sqlx::query(
            "UPDATE declaration_batch SET verified_at = datetime('now', '-200 days') WHERE id = ?",
        )
        .bind(&batch)
        .execute(&pool)
        .await
        .expect("lùi ngày đối chiếu");

        assert_eq!(redact_old_identities(&pool, 90).await.unwrap(), 1);

        let row = sqlx::query(
            "SELECT full_name, dob, doc_no, phone, address_detail, passport_no, redacted_at
               FROM declaration_identity WHERE id = ?",
        )
        .bind(&identity_id)
        .fetch_one(&pool)
        .await
        .expect("đọc lại danh tính");
        assert_eq!(row.get::<String, _>("full_name"), "");
        assert_eq!(row.get::<String, _>("dob"), "");
        assert!(row.get::<Option<String>, _>("doc_no").is_none());
        assert!(row.get::<Option<String>, _>("phone").is_none());
        assert!(row.get::<Option<String>, _>("address_detail").is_none());
        assert!(row.get::<Option<String>, _>("passport_no").is_none());
        assert!(row.get::<Option<String>, _>("redacted_at").is_some());

        // Bằng chứng "khách này đã khai lô nào" vẫn còn nguyên.
        let entries: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM declaration_entry de
               JOIN declaration_link dl ON dl.id = de.link_id
              WHERE dl.identity_id = ?",
        )
        .bind(&identity_id)
        .fetch_one(&pool)
        .await
        .expect("đếm entry");
        assert_eq!(
            entries, 1,
            "xóa dòng sẽ phá lịch sử lô — phải che, không xóa"
        );

        // Chạy lại không che lần hai.
        assert_eq!(redact_old_identities(&pool, 90).await.unwrap(), 0);
    }

    /// Một link còn chờ đối chiếu thì cả danh tính chưa được che — nếu không,
    /// dữ liệu biến mất trước khi khai xong.
    #[tokio::test]
    async fn redaction_waits_for_every_link_of_an_identity() {
        let pool = pool().await;
        let identity_id = insert_identity(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");

        let old_link = insert_link(&pool, &identity_id, Some("booking-cu"), "2", None)
            .await
            .expect("link cũ");
        let old_batch = insert_batch(&pool, "VN", "/tmp/cu.xlsx", 1)
            .await
            .expect("lô cũ");
        insert_entries(&pool, &old_batch, &[old_link])
            .await
            .expect("dòng lô cũ");
        set_batch_verified(&pool, &old_batch, 1).await.expect("ok");
        sqlx::query(
            "UPDATE declaration_batch SET verified_at = datetime('now', '-200 days') WHERE id = ?",
        )
        .bind(&old_batch)
        .execute(&pool)
        .await
        .expect("lùi ngày");

        // Lượt lưu trú mới của cùng khách, chưa đối chiếu.
        insert_link(&pool, &identity_id, Some("booking-moi"), "2", None)
            .await
            .expect("link mới");

        assert_eq!(
            redact_old_identities(&pool, 90).await.unwrap(),
            0,
            "còn một lượt chưa khai xong thì chưa được che"
        );
    }

    /// Băng chuyền: thả ảnh xong khách phải CÓ MẶT trong danh sách chờ ngay,
    /// không qua khu "hồ sơ chờ ghép" trung gian nào.
    #[tokio::test]
    async fn saving_a_scan_puts_the_guest_straight_into_the_pending_list() {
        let pool = pool().await;

        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");

        let pending = pending_link_ids(&pool).await.expect("đọc danh sách chờ");
        assert_eq!(pending.len(), 1, "một lần thả ảnh = một dòng chờ khai");

        let rows = load_rows_by_link_ids(&pool, &pending).await.expect("đọc dòng");
        assert_eq!(rows[0].identity.full_name, "Phan Thị Mỹ Hà");
        assert_eq!(rows[0].stay_reason, "1", "lý do mặc định là Du lịch");
        assert!(rows[0].stay.room_no.is_empty(), "phòng mặc định: chưa xác định");
    }

    /// Quét lại cùng tấm giấy tờ khi link cũ còn hoạt động: KHÔNG đẻ link thứ hai.
    #[tokio::test]
    async fn rescanning_while_a_link_is_active_does_not_duplicate() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lần 1");
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lần 2");

        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link")
            .fetch_one(&pool)
            .await
            .expect("đếm");
        assert_eq!(n, 1);
    }

    /// Khách quay lại sau khi lượt trước ĐÃ khai xong: lần quét mới là lượt ở
    /// mới — phải có link mới, và `insert_link` phải trả về đúng link MỚI chứ
    /// không phải link cũ (hai link cùng identity + stay_id NULL cùng tồn tại).
    #[tokio::test]
    async fn a_returning_guest_gets_a_fresh_link_after_the_old_one_was_declared() {
        let pool = pool().await;
        let id = save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lượt 1");
        let old_link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&old_link))
            .await
            .expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("đã khai xong");

        let id2 = save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lượt 2");
        assert_eq!(id, id2, "vẫn là cùng một con người");

        let pending = pending_link_ids(&pool).await.expect("đọc lại");
        assert_eq!(pending.len(), 1, "lượt ở mới phải chờ khai");
        assert_ne!(pending[0], old_link, "phải là link MỚI");
    }

    /// Thẻ khách cho sửa phòng và lý do tại chỗ — không còn form ghép riêng.
    #[tokio::test]
    async fn room_and_reason_can_be_edited_in_place() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();

        update_link(&pool, &link, Some("booking-9"), "20", Some("Đi công tác"))
            .await
            .expect("sửa được");

        let rows = load_rows_by_link_ids(&pool, std::slice::from_ref(&link))
            .await
            .expect("đọc lại");
        assert_eq!(rows[0].stay.stay_id, "booking-9");
        assert_eq!(rows[0].stay_reason, "20");
        assert_eq!(rows[0].stay_reason_note.as_deref(), Some("Đi công tác"));
    }

    /// Đã nằm trong lô verified thì bản ghi là bằng chứng — không sửa được nữa.
    #[tokio::test]
    async fn a_declared_link_refuses_edits() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("chốt");

        assert!(update_link(&pool, &link, None, "1", None).await.is_err());
    }

    /// `insert_link` tự nó (không qua `save_identity_ensuring_link`) phải trả
    /// về đúng link VỪA TẠO khi một danh tính đang giữ hai link cùng
    /// `stay_id NULL` (một đã verified, một vừa ghi) — không phải link cũ đã
    /// khai xong. `save_identity_ensuring_link` dùng thẳng giá trị `insert_link`
    /// trả về để ghép lượt lưu trú, nên trả nhầm link cũ nghĩa là ghép nhầm
    /// phòng cho một khai báo đã đóng.
    #[tokio::test]
    async fn insert_link_returns_the_freshly_created_link_not_the_already_declared_one() {
        let pool = pool().await;
        let identity_id = insert_identity(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu danh tính");

        let old_link = insert_link(&pool, &identity_id, None, "1", None)
            .await
            .expect("tạo link lượt 1");
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&old_link))
            .await
            .expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("đã khai xong");

        let new_link = insert_link(&pool, &identity_id, None, "1", None)
            .await
            .expect("tạo link lượt 2");

        assert_ne!(
            new_link, old_link,
            "insert_link phải trả về link MỚI vừa ghi, không phải link cũ đã verified"
        );

        let newest: String = sqlx::query_scalar(
            "SELECT id FROM declaration_link WHERE identity_id = ? ORDER BY rowid DESC LIMIT 1",
        )
        .bind(&identity_id)
        .fetch_one(&pool)
        .await
        .expect("đọc lại dòng mới nhất");
        assert_eq!(new_link, newest, "giá trị trả về phải đúng là dòng vừa ghi");
    }

    /// "Gác lại" sống trong DB — nhịp "rảnh thì làm" có thể cách nhau nhiều
    /// ngày và nhiều lần tắt app.
    #[tokio::test]
    async fn holding_a_guest_is_persisted_and_reversible() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();

        set_link_held(&pool, &link, true).await.expect("gác");
        let held: Option<String> =
            sqlx::query_scalar("SELECT held_at FROM declaration_link WHERE id = ?")
                .bind(&link)
                .fetch_one(&pool)
                .await
                .expect("đọc");
        assert!(held.is_some());

        set_link_held(&pool, &link, false).await.expect("đưa lại");
        let held: Option<String> =
            sqlx::query_scalar("SELECT held_at FROM declaration_link WHERE id = ?")
                .bind(&link)
                .fetch_one(&pool)
                .await
                .expect("đọc lại");
        assert!(held.is_none());
    }

    /// Nút "Xóa" trên thẻ: scan nhầm / khách không ở. Xóa link VÀ danh tính
    /// trong một transaction — không để lại danh tính mồ côi (khái niệm đó đã
    /// chết cùng v22).
    #[tokio::test]
    async fn discarding_a_card_removes_link_and_identity_together() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();

        discard_link(&pool, &link).await.expect("xóa");

        let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_link")
            .fetch_one(&pool)
            .await
            .expect("đếm link");
        let ids: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_identity")
            .fetch_one(&pool)
            .await
            .expect("đếm danh tính");
        assert_eq!((links, ids), (0, 0));
    }

    /// Danh tính còn link khác (lượt ở trước đã khai) thì CHỈ xóa link này —
    /// lịch sử lô của lượt trước là bằng chứng, phải còn nguyên.
    #[tokio::test]
    async fn discarding_keeps_an_identity_that_other_links_still_need() {
        let pool = pool().await;
        let id = save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lượt 1");
        let old_link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&old_link)).await.expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("chốt");

        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lượt 2");
        let new_link = pending_link_ids(&pool).await.expect("đọc")[0].clone();

        discard_link(&pool, &new_link).await.expect("xóa lượt 2");

        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_identity WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .expect("đếm");
        assert_eq!(kept, 1, "danh tính của lượt đã khai phải còn");
    }

    /// Đã đối soát = bằng chứng — từ chối xóa.
    #[tokio::test]
    async fn a_reconciled_card_refuses_to_be_discarded() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("chốt");

        assert!(discard_link(&pool, &link).await.is_err());
    }

    /// Bấm vào lỗi trên thẻ → sửa trong form → lưu theo id, kể cả khi danh
    /// tính không có số giấy tờ (đường merge theo doc_no không dùng được).
    #[tokio::test]
    async fn an_identity_can_be_edited_by_id() {
        let pool = pool().await;
        let no_doc = Identity {
            full_name: "Khách nhập tay".into(),
            dob: "1990-01-01".into(),
            gender: "M".into(),
            nationality_iso3: "VNM".into(),
            ..Default::default()
        };
        let id = save_identity_ensuring_link(&pool, &no_doc, "manual", "needs_review")
            .await
            .expect("lưu");

        let mut fixed = no_doc.clone();
        fixed.phone = Some("0912345678".into());
        update_identity(&pool, &id, &fixed, "manual", "needs_review")
            .await
            .expect("sửa được");

        let phone: Option<String> =
            sqlx::query_scalar("SELECT phone FROM declaration_identity WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("đọc");
        assert_eq!(phone.as_deref(), Some("0912345678"));

        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM declaration_identity")
            .fetch_one(&pool)
            .await
            .expect("đếm");
        assert_eq!(n, 1, "sửa chứ không đẻ dòng mới");
    }

    /// Cả hai link của danh tính đều verified (không chỉ một) → không còn link
    /// nào sống để sửa, bản ghi là bằng chứng, không sửa sau lưng.
    #[tokio::test]
    async fn a_declared_identity_refuses_edits_by_id() {
        let pool = pool().await;
        let id = save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("chốt");

        // Quay lại lượt sau: active == 0 nên đẻ link mới — cũng đưa nó tới
        // verified, để cả hai link của danh tính đều là bằng chứng đã khai.
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu lượt hai");
        let link2 = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch2 = insert_batch(&pool, "VN", "/tmp/y.xlsx", 1).await.expect("lô 2");
        insert_entries(&pool, &batch2, std::slice::from_ref(&link2)).await.expect("dòng 2");
        set_batch_verified(&pool, &batch2, 1).await.expect("chốt 2");

        assert!(
            update_identity(&pool, &id, &vn_identity(), "manual", "needs_review")
                .await
                .is_err(),
            "cả hai link đều verified — không còn link nào sống để sửa"
        );
    }

    /// Khách nước ngoài verified tháng 3, quay lại tháng 7 visa mới: link cũ
    /// verified vẫn là bằng chứng, nhưng link của lượt ở hiện tại còn sống —
    /// đó chính là đường sửa E09 (hạn visa cũ chặn xuất file).
    #[tokio::test]
    async fn an_identity_with_a_live_link_alongside_a_verified_one_accepts_edits() {
        let pool = pool().await;
        let id = save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu tháng 3");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");
        set_batch_verified(&pool, &batch, 1).await.expect("chốt tháng 3");

        // Quay lại tháng 7: active == 0 nên đẻ link mới, link này còn sống.
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu tháng 7");

        let mut renewed = vn_identity();
        renewed.visa_valid_until = Some("2027-12-31".into());
        update_identity(&pool, &id, &renewed, "manual", "needs_review")
            .await
            .expect("còn link sống ngoài lô verified nên sửa được");

        let visa: Option<String> =
            sqlx::query_scalar("SELECT visa_valid_until FROM declaration_identity WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("đọc");
        assert_eq!(visa.as_deref(), Some("2027-12-31"));
    }

    /// Đã xuất là rời danh sách "Chưa khai báo" — khách sống trên thẻ đối
    /// chiếu, kể cả khi lô fail (tránh xuất trùng một khách ra hai file).
    #[tokio::test]
    async fn exported_guests_leave_the_pending_list() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();

        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");

        assert!(
            pending_link_ids(&pool).await.expect("đọc lại").is_empty(),
            "đã xuất thì không còn trong danh sách chờ"
        );

        set_batch_failed(&pool, &batch, 0).await.expect("lô fail");
        assert!(
            pending_link_ids(&pool).await.expect("đọc lần ba").is_empty(),
            "lô fail cũng KHÔNG tự quay lại danh sách — phải qua kbtt_reopen_batch (PR 3)"
        );
    }

    /// Lô fail vì dữ liệu sai thì phải có đường sửa: mở lại lô đưa khách về
    /// danh sách, còn dòng lô ở lại làm lịch sử "đã từng xuất và fail".
    #[tokio::test]
    async fn reopening_a_failed_batch_returns_its_guests_to_the_list() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");
        set_batch_failed(&pool, &batch, 0).await.expect("fail");

        reopen_failed_batch(&pool, &batch).await.expect("mở lại");

        assert_eq!(
            pending_link_ids(&pool).await.expect("đọc lại"),
            vec![link],
            "khách quay lại danh sách chờ"
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM declaration_batch WHERE id = ?")
                .bind(&batch)
                .fetch_one(&pool)
                .await
                .expect("đọc lô");
        assert_eq!(
            status, "reopened",
            "lô phải chuyển sang trạng thái chót 'reopened', không kẹt ở 'failed' mãi \
             (thẻ đối chiếu mới không lọc theo 'reopened' nên sẽ không mọc thẻ ma)"
        );
    }

    /// Lô chưa fail thì không mở lại được — 'exported' phải đi qua đối chiếu
    /// trước, 'verified' là bằng chứng. Lô đã 'reopened' cũng không mở lại lần
    /// hai được — đó là trạng thái chót, không phải cửa quay vòng.
    #[tokio::test]
    async fn only_failed_batches_can_be_reopened() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch = insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        insert_entries(&pool, &batch, std::slice::from_ref(&link)).await.expect("dòng");

        assert!(reopen_failed_batch(&pool, &batch).await.is_err(), "exported: chưa được");
        set_batch_verified(&pool, &batch, 1).await.expect("chốt");
        assert!(reopen_failed_batch(&pool, &batch).await.is_err(), "verified: không bao giờ");

        // Khách vừa verified giờ tạo lại một link mới (quay lại ở lượt sau),
        // dùng nó để dựng một lô 'failed' -> 'reopened' độc lập.
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu lại");
        let link2 = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        let batch2 = insert_batch(&pool, "VN", "/tmp/y.xlsx", 1).await.expect("lô 2");
        insert_entries(&pool, &batch2, std::slice::from_ref(&link2)).await.expect("dòng 2");
        set_batch_failed(&pool, &batch2, 0).await.expect("fail");

        reopen_failed_batch(&pool, &batch2).await.expect("mở lần đầu: được");
        assert!(
            reopen_failed_batch(&pool, &batch2).await.is_err(),
            "reopened: không mở lại lần hai"
        );
    }

    #[tokio::test]
    async fn held_flags_ride_along_with_the_rows() {
        let pool = pool().await;
        save_identity_ensuring_link(&pool, &vn_identity(), "qr_cccd", "verified")
            .await
            .expect("lưu");
        let link = pending_link_ids(&pool).await.expect("đọc")[0].clone();
        set_link_held(&pool, &link, true).await.expect("gác");

        assert_eq!(
            pending_link_ids(&pool).await.expect("đọc").len(),
            1,
            "khách gác lại vẫn thuộc danh sách (UI xếp xuống khu thu gọn)"
        );
        let held = held_by_link(&pool, std::slice::from_ref(&link)).await.expect("map");
        assert_eq!(held.get(&link), Some(&true));
    }
}
