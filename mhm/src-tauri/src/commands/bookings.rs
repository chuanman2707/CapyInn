use super::{emit_db_update, require_admin, AppState};
use crate::{
    app_error::{
        codes, normalize_correlation_id, record_command_failure,
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

#[tauri::command]
pub async fn preview_void_booking(
    state: State<'_, AppState>,
    booking_id: String,
) -> CommandResult<VoidBookingPreview> {
    // Chỉ admin xoá được, nên chỉ admin cần xem trước hậu quả.
    require_admin(&state)?;
    void_queries::load_void_preview(&state.db, &booking_id)
        .await
        .map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Không đọc được thông tin lượt ở: {error}"),
            )
        })
}

#[tauri::command]
pub async fn void_booking(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: VoidBookingRequest,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<VoidBookingResponse> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = void_booking_failure_context(&req);

    // Hàng rào chính của tính năng này: không giới hạn thời gian, nên phân
    // quyền là thứ duy nhất còn chặn. UI có khoá nút hay không không tính.
    let admin = require_admin(&state).inspect_err(|command_error| {
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

    emit_db_update(&app, "rooms");

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::void_booking_failure_context;
    use crate::models::VoidBookingRequest;
    use serde_json::json;

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
    // Không dựng được `State<'_, AppState>` thật trong test đơn vị — `State` là
    // tuple struct riêng của crate `tauri` (`State<'r, T>(&'r T)`), trường bị
    // private và không có constructor công khai; `tauri = { features = [] }`
    // trong Cargo.toml cũng không bật feature `test` (không có
    // `tauri::test::mock_builder`). Không có tests/ tích hợp nào trong repo
    // dựng App/AppHandle thật, và không command nào khác từng gọi thẳng một
    // `#[tauri::command]` nhận `State` trong test — kể cả `commands::audit`,
    // sibling gần nhất (cũng require_admin + correlation id + ghi log lỗi):
    // test của nó chỉ gọi thẳng `require_admin_user`/helper trích ra
    // (`record_audit_auth_error`), chưa từng gọi `run_night_audit` chính nó.
    //
    // Nên cách còn lại để một test đỏ khi ai đó lỡ xoá `require_admin` khỏi
    // MỘT TRONG HAI lệnh này là đọc lại chính mã nguồn file — cùng kiểu dựa
    // vào quy ước rustfmt mà `architecture_guard.rs` và
    // `commands::assistant_conversations` đã dùng cho đúng lớp vấn đề này.
    //
    // Bỏ comment `//` trước khi so khớp: đo được bằng tay — chỉ COMMENT dòng
    // gọi (`// require_admin(&state)?;`) mà không xoá, hai test dưới vẫn xanh
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
        let source = "require_admin(&state)?;\n// require_admin(&state)?; disabled\nlet x = 1;";
        let cleaned = strip_line_comments(source);
        assert_eq!(
            cleaned.matches("require_admin(&state)").count(),
            1,
            "a commented-out call must not still read as a live call"
        );
        assert!(cleaned.contains("let x = 1;"));
    }

    #[test]
    fn preview_void_booking_requires_admin_before_reading() {
        let source = strip_line_comments(SOURCE);
        let preview_marker = "pub async fn preview_void_booking(";
        let void_marker = "pub async fn void_booking(";

        let preview_start = source
            .find(preview_marker)
            .expect("expected to find preview_void_booking in bookings.rs");
        let void_start = source[preview_start..]
            .find(void_marker)
            .map(|offset| preview_start + offset)
            .expect("expected void_booking to follow preview_void_booking in this file");
        let preview_body = &source[preview_start..void_start];

        let admin_pos = preview_body
            .find("require_admin(&state)")
            .expect("preview_void_booking must call require_admin");
        let read_pos = preview_body
            .find("void_queries::load_void_preview")
            .expect("preview_void_booking must call load_void_preview");

        assert!(
            admin_pos < read_pos,
            "require_admin must run before load_void_preview, so a non-admin \
             never reaches the guest/room/revenue data in the preview"
        );
    }

    #[test]
    fn void_booking_requires_admin_before_writing() {
        let source = strip_line_comments(SOURCE);
        let void_marker = "pub async fn void_booking(";
        let void_start = source
            .find(void_marker)
            .expect("expected to find void_booking in bookings.rs");
        let void_body = &source[void_start..];

        let admin_pos = void_body
            .find("require_admin(&state)")
            .expect("void_booking must call require_admin");
        let write_pos = void_body
            .find("void_lifecycle::void_booking_idempotent")
            .expect("void_booking must call void_booking_idempotent");

        assert!(
            admin_pos < write_pos,
            "require_admin must run before void_booking_idempotent, so a \
             non-admin can never void a booking regardless of UI state"
        );
    }
}
