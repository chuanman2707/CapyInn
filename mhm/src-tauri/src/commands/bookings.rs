use super::{emit_db_update, require_admin, AppState};
use crate::{
    app_error::{
        codes, log_system_error, normalize_correlation_id, record_command_failure,
        record_command_failure_with_db_group, CommandError, CommandResult,
    },
    command_idempotency::WriteCommandContext,
    models::*,
    queries::booking::{booking_list_queries, void_queries},
    services::booking::void_lifecycle,
};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};
use tauri::State;

// ─── A1: Get All Bookings (Reservations) ───

pub async fn do_get_all_bookings(
    pool: &Pool<Sqlite>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, String> {
    booking_list_queries::load_bookings_with_guest(pool, filter)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_all_bookings(
    state: State<'_, AppState>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, String> {
    do_get_all_bookings(&state.db, filter).await
}

// ─── Void Booking ───

fn void_booking_failure_context(req: &VoidBookingRequest) -> Value {
    json!({
        "booking_id": req.booking_id.clone(),
        "reason_present": req.reason.is_some(),
    })
}

/// `RowNotFound` là chuyện thường ngày — `booking_id` cũ hoặc sai (ví dụ
/// người dùng đứng nhìn hộp thoại trong khi lượt đó đã bị xoá từ cửa sổ
/// khác) — không phải lỗi hệ thống. Người dùng cần một câu nói được vì sao,
/// không phải "lỗi hệ thống, liên hệ hỗ trợ" kèm support id. Mọi lỗi DB khác
/// mới thật sự là lỗi hệ thống: log qua `log_system_error` như các lệnh khác
/// trong crate, đừng nối thẳng `sqlx::Error` (tiếng Anh, có thể lộ chi tiết
/// DB) vào câu tiếng Việt hiển thị cho người dùng.
fn map_preview_void_booking_error(booking_id: &str, error: sqlx::Error) -> CommandError {
    match error {
        sqlx::Error::RowNotFound => CommandError::user(
            codes::BOOKING_NOT_FOUND,
            "Không tìm thấy lượt này — vui lòng tải lại trang",
        ),
        other => log_system_error(
            "preview_void_booking",
            other.to_string(),
            json!({ "booking_id": booking_id }),
        ),
    }
}

async fn preview_void_booking_inner(
    state: &AppState,
    booking_id: String,
) -> CommandResult<VoidBookingPreview> {
    // Chỉ admin xoá được, nên chỉ admin cần xem trước hậu quả.
    //
    // Gọi `require_admin(state)`, không phải `require_admin(&state)`: `state`
    // ở đây đã là `&AppState` (không phải `State<'_, AppState>` như ở shim
    // phía dưới), nên thêm `&` là borrow thừa — `clippy::needless_borrow` báo
    // lỗi dưới `-D warnings`. Xem test dò chuỗi nguồn tương ứng.
    require_admin(state)?;
    void_queries::load_void_preview(&state.db, &booking_id)
        .await
        .map_err(|error| map_preview_void_booking_error(&booking_id, error))
}

#[tauri::command]
pub async fn preview_void_booking(
    state: State<'_, AppState>,
    booking_id: String,
) -> CommandResult<VoidBookingPreview> {
    preview_void_booking_inner(&state, booking_id).await
}

async fn void_booking_inner(
    state: &AppState,
    req: VoidBookingRequest,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<VoidBookingResponse> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = void_booking_failure_context(&req);

    // Hàng rào chính của tính năng này: không giới hạn thời gian, nên phân
    // quyền là thứ duy nhất còn chặn. UI có khoá nút hay không không tính.
    //
    // `require_admin(state)`, không phải `require_admin(&state)` — cùng lý do
    // với `preview_void_booking_inner`: `state` đã là `&AppState`.
    let admin = require_admin(state).inspect_err(|command_error| {
        record_command_failure(
            "void_booking",
            command_error,
            &effective_correlation_id.value,
            error_context.clone(),
        );
    })?;

    let mut write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "void_booking",
    )?;
    write_command_context.actor_id = Some(admin.id.clone());

    log::info!(
        "void_booking start correlation_id={} booking_id={}",
        effective_correlation_id.value,
        req.booking_id
    );

    let result =
        void_lifecycle::void_booking_idempotent(&state.db, &write_command_context, req, admin.id)
            .await
            .inspect_err(|command_error| {
                record_command_failure_with_db_group(
                    "void_booking",
                    command_error,
                    &effective_correlation_id.value,
                    None,
                    error_context.clone(),
                );
            })?;

    let response: VoidBookingResponse =
        serde_json::from_value(result.response).map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Invalid void_booking idempotent response: {error}"),
            )
            .with_request_id(write_command_context.request_id.clone())
        })?;

    log::info!(
        "void_booking success correlation_id={} booking_id={} previous_status={}",
        effective_correlation_id.value,
        response.booking_id,
        response.previous_status
    );

    Ok(response)
}

#[tauri::command]
pub async fn void_booking(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: VoidBookingRequest,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<VoidBookingResponse> {
    let response = void_booking_inner(&state, req, correlation_id, idempotency_key).await?;
    emit_db_update(&app, "rooms");
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{
        map_preview_void_booking_error, preview_void_booking_inner, void_booking_failure_context,
        void_booking_inner, AppState,
    };
    use crate::app_error::{codes, AppErrorKind, CommandError};
    use crate::models::{User, VoidBookingRequest};
    use serde_json::json;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
    use std::sync::{Arc, Mutex};

    #[test]
    fn void_booking_failure_context_keeps_only_booking_id_and_reason_flag() {
        let context = void_booking_failure_context(&VoidBookingRequest {
            booking_id: "booking-1".to_string(),
            reason: Some("Nhập trùng khách".to_string()),
        });

        assert_eq!(
            context,
            json!({
                "booking_id": "booking-1",
                "reason_present": true,
            })
        );
        assert!(context.get("reason").is_none());
    }

    // ─── Cửa an ninh: require_admin ───
    //
    // Lớp 1 (chính, chạy code thật): các test `..._inner_rejects_...` ở dưới
    // dựng một `AppState` thật — `AppState` là `pub` với hai trường `pub`
    // (`commands/mod.rs`), khác `State<'_, AppState>` (tuple struct riêng của
    // tauri, trường private, không constructor công khai) — rồi gọi thẳng
    // `preview_void_booking_inner`/`void_booking_inner`, khẳng định lỗi trả về
    // đúng loại + đúng mã VÀ việc nhạy cảm (đọc dữ liệu preview, ghi void)
    // không hề xảy ra khi bị chặn.
    //
    // Lớp 2 (rẻ, tức thì, không cần async/DB): hai test dưới đọc lại chính mã
    // nguồn file. Không thay được lớp 1 — chỉ bắt hình dạng "gọi đúng chỗ",
    // không chạy code thật — nhưng bắt được ngay một lỗi cụ thể mà lớp 1 cũng
    // bắt được, chỉ chậm hơn: gọi `require_admin(state)` rồi VỨT LUÔN kết quả
    // (`let _ = require_admin(state);`, hay `require_admin(state).ok()
    // .map(|u| u.id).unwrap_or_default()`) — biên dịch sạch, chuỗi con
    // `"require_admin(state)"` vẫn còn nguyên trong file, nhưng không còn
    // chặn gì. Vì vậy so khớp cả `?` (preview) / `let admin = ` (write) — so
    // khớp mỗi chuỗi con `"require_admin(state)"` không đủ để bắt dạng bypass
    // này. (Gọi `require_admin(state)`, không `require_admin(&state)`: `state`
    // trong `..._inner` đã là `&AppState`, thêm `&` là borrow thừa —
    // `clippy::needless_borrow` chặn dưới `-D warnings`.)
    //
    // Bỏ comment `//` trước khi so khớp: đo được bằng tay — chỉ COMMENT dòng
    // gọi (`// require_admin(state)?;`) mà không xoá, hai test dưới vẫn xanh
    // vì chuỗi con vẫn còn trong file, dù không còn là code chạy được. Test
    // "đỏ khi xoá gate" mà không tự vệ trước hình dạng này thì không có răng.
    const SOURCE: &str = include_str!("bookings.rs");

    fn strip_line_comments(source: &str) -> String {
        source
            .lines()
            .map(|line| match line.find("//") {
                Some(index) => &line[..index],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn strip_line_comments_removes_a_commented_out_call_but_keeps_real_code() {
        let source = "require_admin(state)?;\n// require_admin(state)?; disabled\nlet x = 1;";
        let cleaned = strip_line_comments(source);
        assert_eq!(
            cleaned.matches("require_admin(state)").count(),
            1,
            "a commented-out call must not still read as a live call"
        );
        assert!(cleaned.contains("let x = 1;"));
    }

    #[test]
    fn preview_void_booking_requires_admin_before_reading() {
        let source = strip_line_comments(SOURCE);
        // Không `pub` — `..._inner` chỉ được gọi trong file này (shim + test).
        let preview_marker = "async fn preview_void_booking_inner(";
        let void_marker = "async fn void_booking_inner(";

        let preview_start = source
            .find(preview_marker)
            .expect("expected to find preview_void_booking_inner in bookings.rs");
        let void_start = source[preview_start..]
            .find(void_marker)
            .map(|offset| preview_start + offset)
            .expect(
                "expected void_booking_inner to follow preview_void_booking_inner in this file",
            );
        let preview_body = &source[preview_start..void_start];

        // Khớp cả dấu `?`: `let _ = require_admin(state);` vẫn chứa chuỗi con
        // `"require_admin(state)"` nhưng vứt luôn kết quả — biên dịch sạch mà
        // không còn chặn gì (Finding 1).
        let admin_pos = preview_body.find("require_admin(state)?").expect(
            "preview_void_booking_inner must propagate require_admin(state)? with `?` — a \
                 discarded Result must not satisfy this check",
        );
        let read_pos = preview_body
            .find("void_queries::load_void_preview")
            .expect("preview_void_booking_inner must call load_void_preview");

        assert!(
            admin_pos < read_pos,
            "require_admin must run before load_void_preview, so a non-admin \
             never reaches the guest/room/revenue data in the preview"
        );
    }

    #[test]
    fn void_booking_requires_admin_before_writing() {
        let source = strip_line_comments(SOURCE);
        // `..._inner` không `pub`; shim `void_booking` thì có (đăng ký lệnh Tauri).
        let void_marker = "async fn void_booking_inner(";
        let shim_marker = "pub async fn void_booking(";

        let void_start = source
            .find(void_marker)
            .expect("expected to find void_booking_inner in bookings.rs");
        // Chặn trên tại nơi shim `void_booking` bắt đầu, KHÔNG chạy tới cuối
        // file (Finding 5): không chặn, cửa sổ tìm kiếm lọt luôn vào chính
        // `mod tests` — nơi các chuỗi con y hệt (`"require_admin(state)"`,
        // `"pub async fn void_booking("`, …) nằm sẵn trong các test khác dưới
        // dạng string literal, nên có thể khớp nhầm vào đó thay vì code gate
        // thật.
        let void_end = source[void_start..]
            .find(shim_marker)
            .map(|offset| void_start + offset)
            .expect("expected the void_booking shim to follow void_booking_inner in this file");
        let void_body = &source[void_start..void_end];

        // Khớp cả `let admin = `: `require_admin(state).ok().map(|u| u.id)
        // .unwrap_or_default()` vẫn chứa chuỗi con `"require_admin(state)"`
        // và biên dịch sạch (chỉ cần đổi `admin.id` thành `admin` ở hai chỗ
        // dùng phía dưới), nhưng không còn propagate lỗi auth nào cả nữa
        // (Finding 1).
        let admin_pos = void_body.find("let admin = require_admin(state)").expect(
            "void_booking_inner must bind `let admin = require_admin(state)` — a discarded \
                 Result (e.g. `.ok().map(|u| u.id).unwrap_or_default()`) must not satisfy this \
                 check",
        );
        let write_pos = void_body
            .find("void_lifecycle::void_booking_idempotent")
            .expect("void_booking_inner must call void_booking_idempotent");

        assert!(
            admin_pos < write_pos,
            "require_admin must run before void_booking_idempotent, so a \
             non-admin can never void a booking regardless of UI state"
        );
    }

    // ─── Cửa an ninh, lớp 1: chạy code thật ───

    async fn test_pool() -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");
        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");
        pool
    }

    fn app_state_for(pool: Pool<Sqlite>, user: Option<User>) -> AppState {
        AppState {
            db: pool,
            current_user: Arc::new(Mutex::new(user)),
        }
    }

    fn receptionist() -> User {
        User {
            id: "u-reception".to_string(),
            name: "Reception".to_string(),
            role: "receptionist".to_string(),
            active: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn assert_is_auth_error(error: &CommandError, expected_code: &str) {
        assert_eq!(
            error.kind,
            AppErrorKind::User,
            "gate rejection must be a user-kind error, got {error:?}"
        );
        assert_eq!(
            error.code, expected_code,
            "unexpected error code, got {error:?}"
        );
    }

    async fn seed_room(pool: &Pool<Sqlite>, room_id: &str) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, 'standard', 1, 0, 250000, 2, 0, 'vacant')",
        )
        .bind(room_id)
        .bind(format!("Room {room_id}"))
        .execute(pool)
        .await
        .expect("seed room");
    }

    async fn seed_active_booking(pool: &Pool<Sqlite>, booking_id: &str, room_id: &str) {
        let guest_id = format!("guest-{booking_id}");
        sqlx::query(
            "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
             VALUES (?, 'domestic', 'Test Guest', 'DOC', '2026-04-27T08:00:00+07:00')",
        )
        .bind(&guest_id)
        .execute(pool)
        .await
        .expect("seed guest");

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                nights, total_price, paid_amount, status, created_at
             ) VALUES (?, ?, ?, '2026-04-27', '2026-04-28', 1, 250000, 0, 'active', '2026-04-27T08:00:00+07:00')",
        )
        .bind(booking_id)
        .bind(room_id)
        .bind(&guest_id)
        .execute(pool)
        .await
        .expect("seed booking");

        // Một lượt `active` thật sự luôn có phòng `occupied` — `void_booking_tx`
        // (ACTIVE branch) trả phòng về `vacant` bằng một UPDATE có điều kiện
        // `WHERE status = 'occupied'`, nên phòng `vacant` sẵn (giá trị mặc định
        // của `seed_room`) làm nhánh đó report nhầm "trạng thái vừa đổi bởi
        // thao tác khác" thay vì phản ánh đúng bug đang được test.
        sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = ?")
            .bind(room_id)
            .execute(pool)
            .await
            .expect("mark seeded room occupied");
    }

    async fn booking_status(pool: &Pool<Sqlite>, booking_id: &str) -> String {
        sqlx::query_scalar::<_, String>("SELECT status FROM bookings WHERE id = ?")
            .bind(booking_id)
            .fetch_one(pool)
            .await
            .expect("read booking status")
    }

    // preview_void_booking_inner: không seed booking nào cả. Nếu gate bị bỏ
    // qua, lỗi trả về sẽ là BOOKING_NOT_FOUND (Finding 3) chứ không phải lỗi
    // auth — so đúng mã auth kỳ vọng là đủ chứng minh DB chưa từng bị đọc.

    #[tokio::test]
    async fn preview_void_booking_inner_rejects_receptionist_before_reading() {
        let pool = test_pool().await;
        let state = app_state_for(pool, Some(receptionist()));

        let error = preview_void_booking_inner(&state, "does-not-exist".to_string())
            .await
            .expect_err("receptionist must be rejected");

        assert_is_auth_error(&error, codes::AUTH_FORBIDDEN);
    }

    #[tokio::test]
    async fn preview_void_booking_inner_rejects_missing_user_before_reading() {
        let pool = test_pool().await;
        let state = app_state_for(pool, None);

        let error = preview_void_booking_inner(&state, "does-not-exist".to_string())
            .await
            .expect_err("missing user must be rejected");

        assert_is_auth_error(&error, codes::AUTH_NOT_AUTHENTICATED);
    }

    #[test]
    fn map_preview_void_booking_error_treats_row_not_found_as_a_user_error() {
        let error = map_preview_void_booking_error("booking-1", sqlx::Error::RowNotFound);

        assert_eq!(error.kind, AppErrorKind::User);
        assert_eq!(error.code, codes::BOOKING_NOT_FOUND);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn map_preview_void_booking_error_keeps_other_db_errors_generic_and_off_the_raw_text() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-preview-void-system-error-{}",
            uuid::Uuid::new_v4()
        ));
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);

        let db_error = sqlx::Error::Protocol("disk I/O error".to_string());
        let raw_text = db_error.to_string();
        let error = map_preview_void_booking_error("booking-1", db_error);

        std::env::remove_var("CAPYINN_RUNTIME_ROOT");
        let _ = std::fs::remove_dir_all(&runtime_root);

        assert_eq!(error.kind, AppErrorKind::System);
        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert!(
            !error.message.contains(&raw_text),
            "system error message must not splice in the raw sqlx Display text: {}",
            error.message
        );
        assert!(error.support_id.is_some());
    }

    // void_booking_inner: `require_admin` thất bại còn gọi
    // `record_command_failure`, chạm filesystem qua
    // `app_identity::runtime_root_opt()` — khoá bằng `CAPYINN_RUNTIME_ROOT`
    // trỏ vào thư mục tạm, giống mọi test khác trong repo chạm
    // `record_command_failure`/`log_system_error` (`app_error.rs`), để không
    // ghi nhầm vào thư mục dữ liệu thật của máy đang chạy test. Chạy qua
    // `Runtime::block_on` (không `.await`) để không giữ `MutexGuard`
    // (`env_lock()`) qua một điểm suspend — `clippy::await_holding_lock`.
    fn run_void_booking_gate_check(user: Option<User>) -> (CommandError, String) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("builds a single-threaded tokio runtime for this test");

        let pool = rt.block_on(test_pool());
        rt.block_on(seed_room(&pool, "room-1"));
        rt.block_on(seed_active_booking(&pool, "booking-1", "room-1"));
        let state = app_state_for(pool.clone(), user);

        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-void-booking-gate-{}",
            uuid::Uuid::new_v4()
        ));
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);

        let result = rt.block_on(void_booking_inner(
            &state,
            VoidBookingRequest {
                booking_id: "booking-1".to_string(),
                reason: None,
            },
            None,
            "idem-key-1".to_string(),
        ));

        std::env::remove_var("CAPYINN_RUNTIME_ROOT");
        let _ = std::fs::remove_dir_all(&runtime_root);
        drop(_guard);

        let error = result.expect_err("gate must reject this caller");
        let status = rt.block_on(booking_status(&pool, "booking-1"));
        (error, status)
    }

    #[test]
    fn void_booking_inner_rejects_receptionist_and_leaves_booking_untouched() {
        let (error, status) = run_void_booking_gate_check(Some(receptionist()));
        assert_is_auth_error(&error, codes::AUTH_FORBIDDEN);
        assert_eq!(
            status, "active",
            "void must not have run against the booking"
        );
    }

    #[test]
    fn void_booking_inner_rejects_missing_user_and_leaves_booking_untouched() {
        let (error, status) = run_void_booking_gate_check(None);
        assert_is_auth_error(&error, codes::AUTH_NOT_AUTHENTICATED);
        assert_eq!(
            status, "active",
            "void must not have run against the booking"
        );
    }
}
