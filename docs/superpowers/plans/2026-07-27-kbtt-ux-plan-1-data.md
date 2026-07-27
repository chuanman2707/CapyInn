# KBTT "băng chuyền một chiều" — Kế hoạch 1/3: nền dữ liệu (v22 + lệnh mới)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migration v22 (`held_at` + backfill danh tính mồ côi) và toàn bộ lệnh backend mà UI mới cần — UI cũ vẫn chạy nguyên trên lệnh cũ (lệnh cũ chỉ gỡ ở PR 3).

**Architecture:** Mọi thay đổi DB đi qua `declaration/repo.rs` (module duy nhất chạm DB) và `db/declaration.rs` (migrations). Lớp command (`commands/declaration.rs`) vẫn mỏng, không logic. Spec: `docs/superpowers/specs/2026-07-27-kbtt-ux-simplify-design.md`.

**Tech Stack:** Rust + sqlx/SQLite, Tauri 2, test bằng `#[tokio::test]` với `sqlite::memory:`.

## Global Constraints

- `guests` / `bookings` / `booking_guests` / `rooms`: CHỈ `SELECT`. Có test ranh giới `declaration_module_never_writes_to_legacy_tables` quét source — đừng làm nó đỏ.
- Không có cột/biến tên `photo_path`, `raw_payload` trong phần production của `src/declaration/` (test `declaration_module_stores_no_images_or_raw_payloads` quét).
- Chuỗi thông báo lỗi bằng tiếng Việt, cùng giọng với chuỗi hiện có.
- Chạy test: `cd mhm/src-tauri && cargo test --lib 2>&1 | tee /tmp/kbtt-test.log; echo "EXIT=$?"` — LUÔN redirect ra file và echo exit code thật (bài học: `| tail` nuốt exit code, đã dính 2 lần).
- Worktree làm việc: `/Users/binhan/HotelManager/.worktrees/kbtt-ux`, nhánh `design/kbtt-ux-simplify`. Mọi đường dẫn dưới đây tương đối từ gốc worktree.

---

### Task 1: Hằng `SCHEMA_VERSION` — gom 13 literal về một chỗ

Hiện có 12 chỗ `assert_eq!(version, 21)` trong `mhm/src-tauri/src/db.rs` và 1 chỗ trong `mhm/src-tauri/src/db/declaration.rs`. Migration v22 sẽ phải sửa cả 13 — gom về một hằng trước, để từ nay chỉ sửa một chỗ.

**Files:**
- Modify: `mhm/src-tauri/src/db.rs`
- Modify: `mhm/src-tauri/src/db/declaration.rs`

**Interfaces:**
- Produces: `pub(crate) const SCHEMA_VERSION: i32 = 21;` trong `db.rs` (Task 2 bump lên 22).

- [ ] **Step 1: Thêm hằng vào `db.rs`**

Ngay trên `pub(crate) async fn run_migrations` (dòng ~205):

```rust
/// Version schema hiện hành. Mọi assert trong test đọc hằng này —
/// thêm migration mới thì chỉ bump ở đây.
pub(crate) const SCHEMA_VERSION: i32 = 21;
```

- [ ] **Step 2: Thay 12 literal trong `db.rs` và 1 trong `db/declaration.rs`**

```bash
cd /Users/binhan/HotelManager/.worktrees/kbtt-ux/mhm/src-tauri
grep -rln "assert_eq!(version, 21)" src/ | xargs sed -i '' 's/assert_eq!(version, 21)/assert_eq!(version, crate::db::SCHEMA_VERSION)/g'
```

Kiểm tra lại `grep -rn "version, 21" src/` phải ra 0 kết quả.

- [ ] **Step 3: Chạy test xác nhận không đổi hành vi**

Run: `cargo test --lib db:: 2>&1 | tee /tmp/kbtt-t1.log; echo "EXIT=$?"`
Expected: PASS toàn bộ, EXIT=0.

- [ ] **Step 4: Commit**

```bash
git add src/db.rs src/db/declaration.rs
git commit -m "refactor(db): single SCHEMA_VERSION constant for test asserts"
```

---

### Task 2: Migration v22 — `held_at` + backfill danh tính mồ côi

**Files:**
- Modify: `mhm/src-tauri/src/db/declaration.rs` (thêm hàm + test)
- Modify: `mhm/src-tauri/src/db.rs` (đăng ký `if current < 22`, bump `SCHEMA_VERSION` lên 22)

**Interfaces:**
- Produces: cột `declaration_link.held_at TEXT NULL`; mọi danh tính chưa redact và chưa có link nào sẽ có một link mặc định (`stay_id NULL`, `stay_reason '1'`).

- [ ] **Step 1: Viết test đỏ trong `db/declaration.rs` mod tests**

```rust
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
        assert_eq!(links, 1, "mỗi danh tính mồ côi phải có đúng một link mặc định");
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
```

- [ ] **Step 2: Chạy, xác nhận đỏ**

Run: `cargo test --lib v22 2>&1 | tee /tmp/kbtt-t2.log; echo "EXIT=$?"`
Expected: FAIL — `held_at` chưa tồn tại ("no such column: held_at").

- [ ] **Step 3: Viết migration trong `db/declaration.rs`**

Sau `migrate_v21_optional_stay`:

```rust
/// Migration v22 — "băng chuyền một chiều" (spec 2026-07-27).
///
/// 1. `held_at`: dấu "gác lại" của một khai báo. NULL = đang trong danh sách
///    chờ xuất. Cột additive nên chỉ cần ALTER, không rebuild bảng.
/// 2. Backfill: khái niệm "hồ sơ chờ chưa ghép" biến mất khỏi UI, nên danh
///    tính nào đang mồ côi (dữ liệu của bản cũ để lại) phải được tạo link mặc
///    định — nếu không, nâng cấp xong khách biến mất vô hình.
///
/// Đây là bảng của riêng module này — luật "không migrate PMS" không bị đụng.
pub(super) async fn migrate_v22_conveyor(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query("ALTER TABLE declaration_link ADD COLUMN held_at TEXT")
        .execute(pool)
        .await?;

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
pub(crate) async fn backfill_orphan_identities(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO declaration_link (id, identity_id, stay_id, stay_reason, created_at)
         SELECT lower(hex(randomblob(16))), di.id, NULL, '1', di.created_at
           FROM declaration_identity di
          WHERE di.redacted_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM declaration_link dl WHERE dl.identity_id = di.id)",
    )
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Đăng ký trong `db.rs`**

Sau khối `if current < 21 { ... }` (file KHÔNG gán lại `current` giữa các khối — theo đúng pattern hiện có):

```rust
    // -- V22: băng chuyền một chiều — held_at + backfill danh tính mồ côi --
    if current < 22 {
        declaration::migrate_v22_conveyor(pool).await?;
    }
```

Và bump:

```rust
pub(crate) const SCHEMA_VERSION: i32 = 22;
```

- [ ] **Step 5: Chạy toàn bộ test**

Run: `cargo test --lib 2>&1 | tee /tmp/kbtt-t2b.log; echo "EXIT=$?"`
Expected: PASS toàn bộ (nhờ Task 1, không còn literal 21 nào phải sửa tay).

- [ ] **Step 6: Commit**

```bash
git add src/db.rs src/db/declaration.rs
git commit -m "feat(kbtt): migration v22 - held_at column and orphan-identity backfill"
```

---

### Task 3: Lưu danh tính là có mặt ngay — `save_identity_ensuring_link`

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs`
- Modify: `mhm/src-tauri/src/commands/declaration.rs` (`kbtt_save_identity` gọi hàm mới)

**Interfaces:**
- Consumes: `insert_identity`, `insert_link` (đã có).
- Produces: `pub async fn save_identity_ensuring_link(pool, identity: &Identity, source: &str, confidence: &str) -> Result<String, String>` — trả `identity_id` (giữ nguyên hợp đồng trả về của `kbtt_save_identity`, UI không phải đổi).

- [ ] **Step 1: Viết test đỏ trong `repo.rs` mod tests**

```rust
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
```

- [ ] **Step 2: Chạy, xác nhận đỏ**

Run: `cargo test --lib saving_a_scan 2>&1 | tee /tmp/kbtt-t3.log; echo "EXIT=$?"`
Expected: FAIL — `save_identity_ensuring_link` chưa tồn tại (compile error).

- [ ] **Step 3: Sửa read-back của `insert_link` (bug tiềm ẩn với 2 link NULL)**

Trong `insert_link` (repo.rs ~dòng 409), câu đọc lại hiện là `SELECT id ... WHERE identity_id = ? AND stay_id IS ?` với `fetch_one` — khi một danh tính có HAI link `stay_id NULL` (một đã verified, một mới), câu này trả dòng tùy ý. Sửa thành:

```rust
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM declaration_link
          WHERE identity_id = ? AND stay_id IS ?
          ORDER BY rowid DESC LIMIT 1",
    )
```

(rowid tăng dần theo INSERT — dòng mới nhất thắng; `created_at` cùng giây thì không phân định được.)

- [ ] **Step 4: Viết `save_identity_ensuring_link` trong `repo.rs`**

Đặt ngay sau `insert_identity`:

```rust
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
```

- [ ] **Step 5: Đổi `kbtt_save_identity` trong `commands/declaration.rs`**

```rust
#[tauri::command]
pub async fn kbtt_save_identity(
    state: State<'_, AppState>,
    identity: Identity,
    source: String,
    confidence: String,
) -> Result<String, String> {
    repo::save_identity_ensuring_link(&state.db, &identity, &source, &confidence).await
}
```

- [ ] **Step 6: Chạy test, chú ý `a_returning_guest...` — nếu đỏ ở "phải là link MỚI" thì Step 3 chưa ăn**

Run: `cargo test --lib 2>&1 | tee /tmp/kbtt-t3b.log; echo "EXIT=$?"`
Expected: PASS toàn bộ. Lưu ý: test cũ `an_identity_waiting_to_be_linked_can_be_listed_and_discarded` vẫn dùng `insert_identity` trực tiếp nên vẫn xanh — nó chỉ bị gỡ ở PR 3.

- [ ] **Step 7: Commit**

```bash
git add src/declaration/repo.rs src/commands/declaration.rs
git commit -m "feat(kbtt): saving an identity auto-creates its pending declaration"
```

---

### Task 4: Sửa phòng/lý do tại chỗ — `kbtt_update_link`

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs`
- Modify: `mhm/src-tauri/src/commands/declaration.rs`
- Modify: `mhm/src-tauri/src/lib.rs` (đăng ký lệnh, thêm vào `generate_handler!` cạnh các `kbtt_*` khác, dòng ~402)

**Interfaces:**
- Produces: repo `pub async fn update_link(pool, link_id: &str, stay_id: Option<&str>, stay_reason: &str, note: Option<&str>) -> Result<(), String>`; command `kbtt_update_link(link_id, stay_id?, stay_reason, note?)`.
- Đồng thời tách guard dùng chung: `async fn link_is_declared(pool, link_id) -> Result<bool, String>` (đang nằm inline trong `delete_link` — Task 5, 6 cùng dùng).

- [ ] **Step 1: Test đỏ trong `repo.rs`**

```rust
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
```

- [ ] **Step 2: Chạy, xác nhận đỏ (compile error: `update_link` chưa có)**

Run: `cargo test --lib room_and_reason 2>&1 | tee /tmp/kbtt-t4.log; echo "EXIT=$?"`

- [ ] **Step 3: Tách guard + viết `update_link` trong `repo.rs`**

Trong `delete_link`, thay đoạn `let declared: i64 = ...` + `if declared > 0 {...}` bằng lời gọi guard mới; guard đặt ngay trên `delete_link`:

```rust
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
```

Trong `delete_link`, phần đầu thành:

```rust
    if link_is_declared(pool, link_id).await? {
        return Err(
            "Khai báo này đã nằm trong một lô đã đối soát — không gỡ được, vì đó là bằng chứng đã khai."
                .into(),
        );
    }
```

- [ ] **Step 4: Command + đăng ký**

`commands/declaration.rs`, cạnh `kbtt_link`:

```rust
/// Sửa phòng / lý do tại chỗ trên thẻ khách. `stay_id = None` = chưa xác định phòng.
#[tauri::command]
pub async fn kbtt_update_link(
    state: State<'_, AppState>,
    link_id: String,
    stay_id: Option<String>,
    stay_reason: String,
    note: Option<String>,
) -> Result<(), String> {
    repo::update_link(
        &state.db,
        &link_id,
        stay_id.as_deref().filter(|s| !s.trim().is_empty()),
        &stay_reason,
        note.as_deref(),
    )
    .await
}
```

`lib.rs`: thêm `commands::declaration::kbtt_update_link,` vào `generate_handler!`.

- [ ] **Step 5: Chạy toàn bộ, PASS, commit**

Run: `cargo test --lib 2>&1 | tee /tmp/kbtt-t4b.log; echo "EXIT=$?"` → EXIT=0.

```bash
git add src/declaration/repo.rs src/commands/declaration.rs src/lib.rs
git commit -m "feat(kbtt): edit a declaration's room and reason in place"
```

---

### Task 5: Gác lại / đưa lại — `kbtt_hold` / `kbtt_release`

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs`
- Modify: `mhm/src-tauri/src/commands/declaration.rs`
- Modify: `mhm/src-tauri/src/lib.rs`

**Interfaces:**
- Produces: repo `pub async fn set_link_held(pool, link_id: &str, held: bool) -> Result<(), String>`; commands `kbtt_hold(link_id)`, `kbtt_release(link_id)`.

- [ ] **Step 1: Test đỏ**

```rust
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
```

- [ ] **Step 2: Chạy đỏ** — `cargo test --lib holding_a_guest 2>&1 | tee /tmp/kbtt-t5.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết repo fn**

```rust
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
```

- [ ] **Step 4: Commands + đăng ký**

```rust
#[tauri::command]
pub async fn kbtt_hold(state: State<'_, AppState>, link_id: String) -> Result<(), String> {
    repo::set_link_held(&state.db, &link_id, true).await
}

#[tauri::command]
pub async fn kbtt_release(state: State<'_, AppState>, link_id: String) -> Result<(), String> {
    repo::set_link_held(&state.db, &link_id, false).await
}
```

`lib.rs`: thêm cả hai vào `generate_handler!`.

- [ ] **Step 5: PASS + commit**

```bash
git add src/declaration/repo.rs src/commands/declaration.rs src/lib.rs
git commit -m "feat(kbtt): hold and release a pending declaration"
```

---

### Task 6: Xóa khách — `kbtt_discard` (link + danh tính, một transaction)

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs`
- Modify: `mhm/src-tauri/src/commands/declaration.rs`
- Modify: `mhm/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `link_is_declared` (Task 4).
- Produces: repo `pub async fn discard_link(pool, link_id: &str) -> Result<(), String>`; command `kbtt_discard(link_id)`.

- [ ] **Step 1: Test đỏ**

```rust
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

    /// Đã đối soát = bằng chứng — từ chối xóa (luật cũ của delete_link giữ nguyên).
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
```

- [ ] **Step 2: Chạy đỏ** — `cargo test --lib discarding 2>&1 | tee /tmp/kbtt-t6.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết `discard_link` (đặt cạnh `delete_link`)**

```rust
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
```

- [ ] **Step 4: Command + đăng ký**

```rust
/// Xóa hẳn một thẻ khách (scan nhầm / khách không ở). Từ chối nếu đã đối soát.
#[tauri::command]
pub async fn kbtt_discard(state: State<'_, AppState>, link_id: String) -> Result<(), String> {
    repo::discard_link(&state.db, &link_id).await
}
```

- [ ] **Step 5: PASS + commit**

```bash
git add src/declaration/repo.rs src/commands/declaration.rs src/lib.rs
git commit -m "feat(kbtt): discard a pending guest card in one transaction"
```

---

### Task 7: Sửa thông tin khách — `kbtt_update_identity`

Bấm vào dòng lỗi trên thẻ sẽ mở `ManualForm` prefill (PR 2). Form đó cần một lệnh sửa-theo-id: `insert_identity` chỉ merge theo số giấy tờ, và với danh tính KHÔNG có số giấy tờ nó sẽ đẻ dòng mới.

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs`
- Modify: `mhm/src-tauri/src/commands/declaration.rs`
- Modify: `mhm/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `update_identity_fields` (private, đã có).
- Produces: repo `pub async fn update_identity(pool, identity_id: &str, identity: &Identity, source: &str, confidence: &str) -> Result<(), String>`; command `kbtt_update_identity(identity_id, identity, source, confidence)`.

- [ ] **Step 1: Test đỏ**

```rust
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

    /// Đã có lô verified → bản ghi là bằng chứng, không sửa sau lưng.
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

        assert!(
            update_identity(&pool, &id, &vn_identity(), "manual", "needs_review")
                .await
                .is_err()
        );
    }
```

- [ ] **Step 2: Chạy đỏ** — `cargo test --lib edited_by_id 2>&1 | tee /tmp/kbtt-t7.log; echo "EXIT=$?"`

- [ ] **Step 3: Viết repo fn (cạnh `update_identity_fields`)**

```rust
/// Sửa một danh tính theo id (form sửa của thẻ khách). Cùng luật với
/// `insert_identity`: đã nằm trong lô `verified` thì bản ghi là bằng chứng của
/// cái đã nộp, không sửa sau lưng.
pub async fn update_identity(
    pool: &Pool<Sqlite>,
    identity_id: &str,
    identity: &Identity,
    source: &str,
    confidence: &str,
) -> Result<(), String> {
    let declared: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM declaration_link dl
           JOIN declaration_entry de  ON de.link_id = dl.id
           JOIN declaration_batch dbt ON dbt.id     = de.batch_id
          WHERE dl.identity_id = ? AND dbt.status = 'verified'",
    )
    .bind(identity_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Không kiểm được lịch sử khai của danh tính: {e}"))?;

    if declared > 0 {
        return Err("Khách này đã có lô khai đã đối soát — thông tin cũ là bằng chứng, không sửa được.".into());
    }

    update_identity_fields(pool, identity_id, identity, source, confidence).await
}
```

- [ ] **Step 4: Command + đăng ký**

```rust
#[tauri::command]
pub async fn kbtt_update_identity(
    state: State<'_, AppState>,
    identity_id: String,
    identity: Identity,
    source: String,
    confidence: String,
) -> Result<(), String> {
    repo::update_identity(&state.db, &identity_id, &identity, &source, &confidence).await
}
```

- [ ] **Step 5: PASS + commit**

```bash
git add src/declaration/repo.rs src/commands/declaration.rs src/lib.rs
git commit -m "feat(kbtt): edit an identity by id from the guest card"
```

---

### Task 8: Danh sách chờ mới + cờ `held` trong DTO

Định nghĩa mới của "Chưa khai báo": link **chưa có entry nào** (đã xuất là rời danh sách, sống trên thẻ đối chiếu — kể cả lô `failed`, xem spec §4.3). DTO phải mang thêm `held` để UI chia hai khu.

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs` (`pending_link_ids`, thêm `held_by_link`)
- Modify: `mhm/src-tauri/src/commands/declaration.rs` (DTO + `kbtt_pending_rows`)
- Modify: `mhm/src/types/index.ts` (interface `DeclarationRow` thêm `held: boolean;`)

**Interfaces:**
- Produces: `DeclarationRowDto` có thêm `pub held: bool`; `DeclarationRowDto::from(&DeclarationRow)` thay bằng `DeclarationRowDto::new(&DeclarationRow, held: bool)`; repo `pub async fn held_by_link(pool, link_ids: &[String]) -> Result<HashMap<String, bool>, String>`.
- Test hợp đồng `row_dto_matches_the_typescript_contract` TỰ bắt hai bên khớp — thêm field Rust mà quên TS (hoặc ngược lại) là test đỏ.

- [ ] **Step 1: Test đỏ trong `repo.rs`**

```rust
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
```

- [ ] **Step 2: Chạy đỏ** — `cargo test --lib exported_guests_leave 2>&1 | tee /tmp/kbtt-t8.log; echo "EXIT=$?"`
(`exported_guests_leave` fail ở assert đầu — định nghĩa cũ vẫn giữ link exported trong pending; `held_by_link` compile error.)

- [ ] **Step 3: Sửa `pending_link_ids` + thêm `held_by_link` trong `repo.rs`**

```rust
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
```

- [ ] **Step 4: DTO — thêm `held`, đổi constructor**

`commands/declaration.rs`: thêm field vào struct (cuối, cạnh `single_token_name_ok`):

```rust
    pub name_confirmed_by_human: bool,
    pub single_token_name_ok: bool,
    /// Khai báo đang "gác lại" — UI xếp vào khu thu gọn, loại khỏi file xuất.
    pub held: bool,
```

Thay `impl From<&DeclarationRow> for DeclarationRowDto` bằng:

```rust
impl DeclarationRowDto {
    fn new(row: &DeclarationRow, held: bool) -> Self {
        let id = &row.identity;
        let stay = &row.stay;
        DeclarationRowDto {
            link_id: row.link_id.clone(),
            identity_id: id.id.clone(),
            full_name: id.full_name.clone(),
            dob: id.dob.clone(),
            gender: id.gender.clone(),
            nationality_iso3: id.nationality_iso3.clone(),
            doc_type_code: id.doc_type_code.clone(),
            doc_type_name: id.doc_type_name.clone(),
            doc_no: id.doc_no.clone(),
            phone: id.phone.clone(),
            residence_status: id.residence_status.clone(),
            address_detail: id.address_detail.clone(),
            passport_no: id.passport_no.clone(),
            passport_expiry: id.passport_expiry.clone(),
            visa_valid_until: id.visa_valid_until.clone(),
            room_no: if stay.room_no.trim().is_empty() {
                None
            } else {
                Some(stay.room_no.clone())
            },
            check_in_date: stay.check_in.clone(),
            expected_check_out: stay.expected_out.clone(),
            stay_reason: row.stay_reason.clone(),
            stay_reason_note: row.stay_reason_note.clone(),
            name_confirmed_by_human: id.name_confirmed_by_human,
            single_token_name_ok: id.single_token_name_ok,
            held,
        }
    }
}
```

`kbtt_pending_rows` thành:

```rust
#[tauri::command]
pub async fn kbtt_pending_rows(
    state: State<'_, AppState>,
) -> Result<Vec<DeclarationRowDto>, String> {
    let link_ids = repo::pending_link_ids(&state.db).await?;
    let rows = repo::load_rows_by_link_ids(&state.db, &link_ids).await?;
    let held = repo::held_by_link(&state.db, &link_ids).await?;
    Ok(rows
        .iter()
        .map(|r| DeclarationRowDto::new(r, held.get(&r.link_id).copied().unwrap_or(false)))
        .collect())
}
```

Trong mod tests của file này, mọi chỗ `DeclarationRowDto::from(&sample_row())` đổi thành `DeclarationRowDto::new(&sample_row(), false)`.

- [ ] **Step 5: TS interface**

`mhm/src/types/index.ts`, interface `DeclarationRow` (dòng ~496), thêm sau `single_token_name_ok: boolean;`:

```typescript
  held: boolean;
```

- [ ] **Step 6: Chạy toàn bộ Rust test — hợp đồng phải xanh**

Run: `cargo test --lib 2>&1 | tee /tmp/kbtt-t8b.log; echo "EXIT=$?"`
Expected: PASS, đặc biệt `row_dto_matches_the_typescript_contract` (nó đọc `types/index.ts` thật — quên Step 5 là nó đỏ và nêu tên field thiếu).

Lưu ý: frontend test cũ (`PendingList.test.tsx`, `ExportPanel.test.tsx`) dựng fixture `DeclarationRow` sẽ thiếu `held` → `npx tsc --noEmit` có thể đỏ. Thêm `held: false` vào các fixture trong `mhm/src/pages/Declaration/*.test.tsx` (grep `link_id:` để tìm). Chạy `cd mhm && npx tsc --noEmit 2>&1 | tee /tmp/kbtt-tsc.log; echo "EXIT=$?"` tới khi EXIT=0.

- [ ] **Step 7: Commit**

```bash
git add src/declaration/repo.rs src/commands/declaration.rs ../src/types/index.ts ../src/pages/Declaration
git commit -m "feat(kbtt): pending list excludes exported guests, rows carry a held flag"
```

---

### Task 9: Badge công thức mới + `kbtt_undeclared_breakdown`

**Files:**
- Modify: `mhm/src-tauri/src/declaration/repo.rs` (thay `count_undeclared_within_48h`)
- Modify: `mhm/src-tauri/src/commands/declaration.rs`
- Modify: `mhm/src-tauri/src/lib.rs`
- Modify: `mhm/src/types/index.ts`

**Interfaces:**
- Produces: repo

```rust
#[derive(Debug, serde::Serialize)]
pub struct UndeclaredBreakdown {
    pub total: i64,          // badge = con số này
    pub not_exported: i64,   // trong danh sách, chưa gác
    pub held: i64,           // đang gác lại
    pub awaiting: i64,       // đã xuất, chờ đối chiếu (gồm cả lô failed)
}
pub async fn undeclared_breakdown(pool) -> Result<UndeclaredBreakdown, String>
```

- Command `kbtt_undeclared_count` trả `breakdown.total` (MainShell.tsx không phải đổi); command mới `kbtt_undeclared_breakdown` trả cả struct. TS: interface `DeclarationUndeclaredBreakdown { total: number; not_exported: number; held: number; awaiting: number; }`.
- **Xóa** `count_undeclared_within_48h` (và test `undeclared_count_drops_only_when_a_batch_is_verified` trong `db/declaration.rs` viết lại theo công thức mới — xem Step 1).

- [ ] **Step 1: Viết test mới (thay test cũ) trong `db/declaration.rs`**

Xóa test `undeclared_count_drops_only_when_a_batch_is_verified`, thêm:

```rust
    /// Badge = một biểu thức duy nhất: link chưa nằm trong lô `verified`.
    /// Tự nhiên gồm cả "chưa xuất", "gác lại", "chờ đối chiếu" và lô `failed`.
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

        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!((b.total, b.not_exported, b.held, b.awaiting), (2, 2, 0, 0));

        // Gác một khách.
        let links = repo::pending_link_ids(&pool).await.expect("đọc");
        repo::set_link_held(&pool, &links[0], true).await.expect("gác");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!((b.total, b.not_exported, b.held, b.awaiting), (2, 1, 1, 0));

        // Xuất khách còn lại.
        let batch = repo::insert_batch(&pool, "VN", "/tmp/x.xlsx", 1).await.expect("lô");
        repo::insert_entries(&pool, &batch, std::slice::from_ref(&links[1]))
            .await
            .expect("dòng");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!((b.total, b.not_exported, b.held, b.awaiting), (2, 0, 1, 1));

        // Lô fail: vẫn đếm (khách chưa được khai thật).
        repo::set_batch_failed(&pool, &batch, 0).await.expect("fail");
        assert_eq!(repo::undeclared_breakdown(&pool).await.expect("đếm").total, 2);

        // Đối soát khớp: khách rời badge.
        repo::set_batch_verified(&pool, &batch, 1).await.expect("chốt");
        let b = repo::undeclared_breakdown(&pool).await.expect("đếm");
        assert_eq!((b.total, b.not_exported, b.held, b.awaiting), (1, 0, 1, 0));
    }
```

- [ ] **Step 2: Chạy đỏ** — `cargo test --lib the_badge_counts 2>&1 | tee /tmp/kbtt-t9.log; echo "EXIT=$?"` (compile error: `undeclared_breakdown` chưa có).

- [ ] **Step 3: Viết repo fn, xóa `count_undeclared_within_48h`**

```rust
#[derive(Debug, serde::Serialize)]
pub struct UndeclaredBreakdown {
    pub total: i64,
    pub not_exported: i64,
    pub held: i64,
    pub awaiting: i64,
}

/// Badge + dòng diễn giải. Một nguồn duy nhất: link chưa thuộc lô `verified`.
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
    Ok(UndeclaredBreakdown {
        total: not_exported + held + awaiting,
        not_exported,
        held,
        awaiting,
    })
}
```

Xóa hàm `count_undeclared_within_48h` (dòng ~73-97) và comment của nó.

- [ ] **Step 4: Commands + đăng ký + TS**

```rust
#[tauri::command]
pub async fn kbtt_undeclared_count(state: State<'_, AppState>) -> Result<i64, String> {
    Ok(repo::undeclared_breakdown(&state.db).await?.total)
}

#[tauri::command]
pub async fn kbtt_undeclared_breakdown(
    state: State<'_, AppState>,
) -> Result<repo::UndeclaredBreakdown, String> {
    repo::undeclared_breakdown(&state.db).await
}
```

`lib.rs`: thêm `kbtt_undeclared_breakdown`. `types/index.ts` (cuối khối Khai báo tạm trú):

```typescript
export interface DeclarationUndeclaredBreakdown {
  total: number;
  not_exported: number;
  held: number;
  awaiting: number;
}
```

- [ ] **Step 5: Toàn bộ test + clippy + fmt trước khi chốt PR**

```bash
cargo clippy --all-targets 2>&1 | tee /tmp/kbtt-clippy.log; echo "EXIT=$?"
cargo test --lib 2>&1 | tee /tmp/kbtt-t9b.log; echo "EXIT=$?"
cd .. && npx tsc --noEmit 2>&1 | tee /tmp/kbtt-tsc2.log; echo "EXIT=$?"
```

Expected: cả ba EXIT=0. KHÔNG chạy `cargo fmt` trên toàn repo (rustfmt 1.9.0 sẽ reformat 12 file không liên quan — bài học phiên trước); chỉ `git diff --check` cho file mình sửa.

- [ ] **Step 6: Commit + mở PR 1**

```bash
git add -A src ../src/types/index.ts
git commit -m "feat(kbtt): one-expression badge count with breakdown"
git push -u origin design/kbtt-ux-simplify
gh pr create --title "feat(kbtt): conveyor data layer - v22, held flag, in-place edits" \
  --body "PR 1/3 của spec docs/superpowers/specs/2026-07-27-kbtt-ux-simplify-design.md. UI cũ vẫn chạy nguyên; lệnh cũ gỡ ở PR 3."
```

Expected: CI (`build-test`, `verify-wave1`) xanh trước khi sang Kế hoạch 2.
