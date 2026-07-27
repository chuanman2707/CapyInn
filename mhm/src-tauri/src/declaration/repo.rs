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

/// Khách chưa khai = lượt lưu trú không có link nào thuộc lô `verified`.
/// Tính bằng query, không cần cột mới ở bảng cũ (§5.3).
pub async fn count_undeclared_within_48h(pool: &Pool<Sqlite>) -> Result<i64, String> {
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
    .map_err(|e| format!("Không đếm được khách chưa khai: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| {
            let g: i64 = r.get("guest_count");
            let d: i64 = r.get("declared_count");
            (g - d).max(0)
        })
        .sum())
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

/// Gỡ một khai báo khỏi danh sách chờ.
///
/// Người vận hành ghép nhầm phòng, hoặc ghép trùng, thì phải có đường lùi —
/// không có nó, một dòng thừa chặn E14 vĩnh viễn và cả lô không xuất được.
///
/// Đã nằm trong lô ĐÃ ĐỐI SOÁT thì từ chối: đó là bằng chứng khách này đã được
/// khai lên cổng, xóa đi là mất dấu vết. Lô chưa đối soát (`exported`) thì gỡ
/// được — file xuất ra vẫn còn trên đĩa, và lô sẽ tự lệch số khi đối soát.
pub async fn delete_link(pool: &Pool<Sqlite>, link_id: &str) -> Result<(), String> {
    let declared: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM declaration_entry de
           JOIN declaration_batch dbt ON dbt.id = de.batch_id
          WHERE de.link_id = ? AND dbt.status = 'verified'",
    )
    .bind(link_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không kiểm được lô của khai báo: {e}"))?;

    if declared > 0 {
        return Err(
            "Khai báo này đã nằm trong một lô đã đối soát — không gỡ được, vì đó là bằng chứng đã khai."
                .into(),
        );
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Không mở được giao dịch: {e}"))?;

    // Entry của lô chưa đối soát phải đi trước — declaration_entry.link_id có FK.
    sqlx::query("DELETE FROM declaration_entry WHERE link_id = ?")
        .bind(link_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Không gỡ được dòng khỏi lô: {e}"))?;

    let affected = sqlx::query("DELETE FROM declaration_link WHERE id = ?")
        .bind(link_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Không gỡ được khai báo: {e}"))?
        .rows_affected();

    tx.commit()
        .await
        .map_err(|e| format!("Không lưu được thay đổi: {e}"))?;

    if affected == 0 {
        return Err("Không tìm thấy khai báo cần gỡ.".into());
    }
    Ok(())
}

/// Danh tính đã lưu nhưng chưa ghép vào lượt lưu trú nào.
///
/// Tồn tại vì trước đây danh tính vừa trích chỉ sống trong `useState` của màn
/// khai báo: đổi sang tab khác là component bị hủy và người vận hành tưởng mất
/// dữ liệu, trong khi bản ghi vẫn nằm yên trong DB, không đường nào lấy ra.
/// Nguồn sự thật là DB, không phải state của React.
pub async fn list_unlinked_identities(pool: &Pool<Sqlite>) -> Result<Vec<Identity>, String> {
    let rows = sqlx::query(
        "SELECT id, full_name, dob, gender, nationality_iso3, doc_type_code, doc_type_source,
                doc_type_name, doc_no, phone, residence_status, address_detail, passport_no,
                passport_expiry, visa_valid_until, name_confirmed_by_human, single_token_name_ok
           FROM declaration_identity di
          WHERE di.redacted_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM declaration_link dl WHERE dl.identity_id = di.id)
          ORDER BY di.created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đọc được danh tính chờ ghép: {e}"))?;

    Ok(rows
        .iter()
        .map(|r| Identity {
            id: r.get("id"),
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
        })
        .collect())
}

/// Xóa một danh tính chưa ghép — người vận hành bỏ ảnh quét nhầm.
///
/// Chỉ xóa được khi chưa có link nào trỏ tới: đã ghép thì đó là bằng chứng đã
/// khai báo, phải đi đường thu hồi (§12.5) chứ không xóa thẳng.
pub async fn delete_unlinked_identity(pool: &Pool<Sqlite>, identity_id: &str) -> Result<(), String> {
    let affected = sqlx::query(
        "DELETE FROM declaration_identity
          WHERE id = ?
            AND NOT EXISTS (SELECT 1 FROM declaration_link dl WHERE dl.identity_id = ?)",
    )
    .bind(identity_id)
    .bind(identity_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Không xóa được danh tính: {e}"))?
    .rows_affected();

    if affected == 0 {
        return Err("Danh tính đã được ghép vào một lượt lưu trú — không xóa thẳng được.".into());
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

/// Cổng nhận thiếu hoặc không nhận. §5.3: không cần code hoàn tác — lô hỏng
/// không còn entry `verified` nào nên khách tự quay lại trạng thái chưa khai.
pub async fn set_batch_failed(
    pool: &Pool<Sqlite>,
    batch_id: &str,
    seen: i64,
) -> Result<(), String> {
    set_batch_outcome(pool, batch_id, "failed", seen).await
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

/// Link đã ghép nhưng chưa nằm trong lô `verified` nào.
///
/// Đây là định nghĩa "còn phải khai" ở mức từng hồ sơ, khác với
/// `count_undeclared_within_48h` vốn đếm theo lượt lưu trú cho badge sidebar.
pub async fn pending_link_ids(pool: &Pool<Sqlite>) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>(
        "SELECT dl.id
           FROM declaration_link dl
          WHERE NOT EXISTS (
                SELECT 1
                  FROM declaration_entry de
                  JOIN declaration_batch db ON db.id = de.batch_id
                 WHERE de.link_id = dl.id AND db.status = 'verified'
          )
          ORDER BY dl.created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Không đọc được danh sách chờ khai: {e}"))
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
/// lô đã `verified`. Người vận hành cần thấy cái đang hỏng trước.
pub async fn list_batches(pool: &Pool<Sqlite>) -> Result<Vec<BatchSummary>, String> {
    let rows = sqlx::query(
        "SELECT id, kind, file_path, row_count, status,
                verified_count, verified_at, created_at
           FROM declaration_batch
          ORDER BY CASE status
                     WHEN 'failed'   THEN 0
                     WHEN 'exported' THEN 1
                     WHEN 'uploaded' THEN 1
                     ELSE 2
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
}
