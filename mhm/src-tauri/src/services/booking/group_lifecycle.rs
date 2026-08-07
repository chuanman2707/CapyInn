use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        ResolvedWriteCommandGuard, SanitizedLedgerIntent, WriteCommandContext,
        WriteCommandExecutor, WriteCommandRequest,
    },
    db_error_monitoring::{classify_db_error_code, is_room_unavailable_conflict_message},
    domain::booking::{BookingError, BookingResult, OriginSideEffect},
    models::{
        status, BookingGroup, GroupCheckinRequest, GroupCheckoutRequest, GroupCheckoutResponse,
    },
    money::MoneyVnd,
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};

use super::{
    billing_service::{record_charge_tx, record_payment_tx, record_payment_with_origin_tx},
    guest_service::{create_group_guest_manifest, link_booking_guests},
    pricing_service::calculate_stay_price_tx,
    stay_lifecycle::MAX_RATE_PER_NIGHT_VND,
    support::{
        begin_immediate_tx, ensure_one_row_affected, ensure_rows_affected,
        insert_room_calendar_rows, invalid_state_transition, merge_pricing_snapshot,
        room_calendar_stays_tx, room_stays_to_json, validate_non_negative_booking_money,
    },
};

const GROUP_ACTIVE: &str = "active";
const GROUP_BOOKED: &str = "booked";
const GROUP_COMPLETED: &str = "completed";
const GROUP_PARTIAL_CHECKOUT: &str = "partial_checkout";

/// Cũng là cơ chế rải dư cho `allocate_paid_amount_by_room_price` bên dưới —
/// đọc doc-comment của hàm đó để biết vì sao gọi lại đúng hàm này (không viết
/// một vòng rải dư thứ hai) vẫn giữ đúng tính chất "tổng cộng lại đúng bằng
/// `total`, dư rải từng 1 đồng một theo thứ tự ổn định".
fn allocate_positive_money_evenly(total: MoneyVnd, count: usize) -> Vec<MoneyVnd> {
    if total <= 0 || count == 0 {
        return vec![0; count];
    }
    let count = count as MoneyVnd;
    let base = total / count;
    let remainder = total % count;
    (0..count)
        .map(|index| base + if index < remainder { 1 } else { 0 })
        .collect()
}

/// Phân bổ `paid_amount` cho các phòng trong đoàn theo TỈ LỆ tổng tiền từng
/// phòng — khác `allocate_positive_money_evenly` (chia đều theo SỐ LƯỢNG
/// phòng, không biết giá). Bắt buộc từ khi Task 15 cho phép giá tay khác
/// nhau giữa các phòng cùng đoàn: chia đều theo số lượng có thể cấp cho một
/// phòng NHIỀU HƠN chính tổng tiền phòng đó. Ví dụ thật từ báo cáo review:
/// G-R1 override 400.000đ × 2 đêm = 800.000đ, G-R2 giá engine 500.000đ × 2
/// đêm = 1.000.000đ, `paid_amount = 1.800.000đ` (khách trả đủ CẢ ĐOÀN) — chia
/// đều ra 900.000đ/phòng, vượt hẳn tổng 800.000đ của G-R1, khiến guard
/// thu-vượt bên dưới từ chối cả lượt nhận đoàn dù khách trả đúng khớp.
///
/// QUY TẮC LÀM TRÒN: phần nguyên `floor(paid_amount × room_total /
/// Σ room_total)` cho từng phòng — nhân trước bằng `i128` rồi mới chia, cùng
/// kỹ thuật `percentage_money_line` (money.rs) dùng cho tỉ lệ phần trăm, để
/// phép nhân không tràn `i64` trước khi chia (VND thật không đủ lớn để tràn
/// `i128`, nên không cần `checked_mul`). Phần dư — LUÔN nhỏ hơn số phòng, vì
/// tổng phần mất mát do làm tròn XUỐNG của N số hạng luôn nhỏ hơn N — rải cho
/// các phòng đã sắp theo `room_id` bằng chính `allocate_positive_money_evenly`
/// (dư, số phòng): hàm đó cho ra một vector toàn 0 trừ đúng `dư` phòng ĐẦU
/// (theo thứ tự đã sắp) nhận 1 — đúng quy tắc rải dư
/// `allocate_positive_money_evenly_by_room` (bản trước Task 15) từng dùng,
/// nên khi mọi phòng cùng giá, hàm này cho kết quả giống hệt bản cũ (xem
/// `group_checkin_reservation_blocks_calendar_and_tracks_deposit` và các test
/// khác chia tiền cọc đều trong `tests/groups.rs` — không sửa gì mà vẫn
/// xanh).
///
/// AN TOÀN — "không phòng nào nhận quá tổng tiền của chính nó" khi
/// `paid_amount <= Σ room_total` — suy ra trực tiếp từ hai điều trên:
/// - `paid_amount < Σ room_total` NGHIÊM NGẶT: làm tròn xuống một số thực nhỏ
///   hơn `room_total` (số nguyên) luôn cho kết quả `<= room_total - 1`, nên
///   mọi phòng còn dư ít nhất 1 đồng "chỗ trống" trước khi rải — rải thêm 1
///   đồng vẫn an toàn.
/// - `paid_amount == Σ room_total` (thu ĐỦ): phép chia của mỗi phòng ra ĐÚNG
///   số nguyên `room_total`, phần dư bằng 0 — không có gì để rải, mỗi phòng
///   nhận đúng tổng của chính mình, không hơn không kém (đây chính là ranh
///   giới review Task 15 chỉ ra).
///
/// Khi `paid_amount > Σ room_total` (thu vượt CẢ ĐOÀN — lỗi nhập liệu thật):
/// mọi phần nguyên đều `>= room_total` (chứng minh tương tự chiều ngược lại),
/// nên hoặc phần nguyên đã vượt tổng của chính phòng đó, hoặc phần dư (luôn
/// dương trong trường hợp này) rải thêm sẽ đẩy ít nhất một phòng vượt — guard
/// thu-vượt theo từng phòng ở `group_checkin_tx` (không đổi) vẫn bắt được,
/// đúng vai trò "lưới đỡ" báo cáo review yêu cầu giữ lại.
fn allocate_paid_amount_by_room_price(
    total_paid: MoneyVnd,
    room_totals: &[(String, MoneyVnd)],
) -> std::collections::HashMap<String, MoneyVnd> {
    let mut ordered = room_totals.to_vec();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let grand_total: i128 = ordered.iter().map(|(_, total)| i128::from(*total)).sum();

    if total_paid <= 0 || grand_total <= 0 {
        return ordered
            .into_iter()
            .map(|(room_id, _)| (room_id, 0))
            .collect();
    }

    let total_paid_i128 = i128::from(total_paid);
    let bases: Vec<MoneyVnd> = ordered
        .iter()
        .map(|(_, room_total)| {
            ((total_paid_i128 * i128::from(*room_total)) / grand_total) as MoneyVnd
        })
        .collect();

    let base_sum: MoneyVnd = bases.iter().sum();
    let leftover = total_paid - base_sum;
    let remainder_units = allocate_positive_money_evenly(leftover, ordered.len());

    let allocations: std::collections::HashMap<String, MoneyVnd> = ordered
        .into_iter()
        .zip(bases)
        .zip(remainder_units)
        .map(|(((room_id, _), base), extra)| (room_id, base + extra))
        .collect();

    debug_assert_eq!(
        allocations.values().sum::<MoneyVnd>(),
        total_paid,
        "phân bổ theo tỉ lệ phải cộng đúng bằng paid_amount, không lệch một đồng"
    );

    allocations
}

fn map_group_checkin_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) if message == "Phải chọn ít nhất 1 phòng" => {
            CommandError::user(codes::GROUP_INVALID_ROOM_COUNT, message)
        }
        BookingError::Validation(message) if message == "Số phòng phải > 0" => {
            CommandError::user(codes::GROUP_INVALID_ROOM_COUNT, message)
        }
        BookingError::Validation(message) if message == "Số đêm phải > 0" => {
            CommandError::user(codes::BOOKING_INVALID_NIGHTS, message)
        }
        BookingError::NotFound(message) if message.starts_with("Phòng ") => {
            CommandError::user(codes::ROOM_NOT_FOUND, message)
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if message.contains(codes::CONFLICT_INVALID_STATE_TRANSITION) {
                return CommandError::user(codes::CONFLICT_INVALID_STATE_TRANSITION, message);
            }
            if is_room_unavailable_conflict_message(&message) {
                return CommandError::user(codes::CONFLICT_ROOM_UNAVAILABLE, message);
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) | BookingError::NotFound(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

fn normalized_room_ids(room_ids: &[String]) -> Vec<String> {
    let mut normalized = room_ids.to_vec();
    normalized.sort();
    normalized
}

fn build_group_checkin_hash_payload(req: &GroupCheckinRequest) -> serde_json::Value {
    let paid_minor_units = req
        .paid_amount
        .map(|amount| json!((i128::from(amount) * 100).to_string()))
        .unwrap_or(serde_json::Value::Null);
    let guests_per_room = req
        .guests_per_room
        .iter()
        .map(|(room_id, guests)| {
            let guests = guests
                .iter()
                .map(|guest| {
                    json!({
                        "guest_type": guest.guest_type.clone(),
                        "full_name": guest.full_name.clone(),
                        "doc_number": guest.doc_number.clone(),
                        "dob": guest.dob.clone(),
                        "gender": guest.gender.clone(),
                        "nationality": guest.nationality.clone(),
                        "address": guest.address.clone(),
                        "visa_expiry": guest.visa_expiry.clone(),
                        "scan_path": guest.scan_path.clone(),
                        "phone": guest.phone.clone(),
                    })
                })
                .collect::<Vec<_>>();
            (room_id.clone(), json!(guests))
        })
        .collect::<serde_json::Map<_, _>>();
    // `HashMap` không có thứ tự lặp ổn định giữa hai lần dựng độc lập, kể cả
    // với đúng một nội dung (mỗi request đi qua deserialize riêng dựng một
    // `HashMap` với seed hash riêng) — nhưng một OBJECT JSON thì có: mọi object
    // đi qua `command_idempotency::canonicalize_json_value` trước khi băm
    // (`stable_json_string`), hàm đó sắp khoá object theo thứ tự chữ cái trước
    // khi serialize, y hệt cách `guests_per_room` ở trên đã an toàn từ trước.
    // Đổi trường này thành MẢNG `[[room_id, rate], ...]` sẽ làm mất tính chất
    // đó — mảng không bị sắp lại — nên hai lượt gọi lại giống hệt nhau dưới
    // cùng idempotency key có thể băm ra hai giá trị khác nhau, và một retry
    // hợp lệ sẽ dừng replay trong im lặng. Xem
    // `group_checkin_hash_payload_encodes_rate_override_as_object_not_array`.
    let rate_override_per_room = req
        .rate_override_per_room
        .iter()
        .map(|(room_id, rate)| (room_id.clone(), json!(rate)))
        .collect::<serde_json::Map<_, _>>();

    json!({
        "schema": "group.checkin.v1",
        "group_name": req.group_name.clone(),
        "organizer_name": req.organizer_name.clone(),
        "organizer_phone": req.organizer_phone.clone(),
        "check_in_date": req.check_in_date.clone(),
        "room_ids": normalized_room_ids(&req.room_ids),
        "master_room_id": req.master_room_id.clone(),
        "guests_per_room": guests_per_room,
        "nights": req.nights,
        "source": req.source.clone(),
        "notes": req.notes.clone(),
        "paid_minor_units": paid_minor_units,
        // Ảnh hưởng trực tiếp tới `total_price` của từng phòng, giống mọi
        // trường khác ở trên — thiếu nó thì hai lượt nhận đoàn dưới cùng
        // idempotency key nhưng GIÁ khác nhau sẽ lặp lại kết quả CŨ (giá cũ,
        // sai) trong im lặng thay vì bị báo `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`
        // — một bug tiền bạc, cùng lý do `rate_override_per_night` đã vào hash
        // của check-in (Task 13) và tạo đặt trước (Task 14).
        "rate_override_per_room": rate_override_per_room,
    })
}

fn group_checkin_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let room_ids = hash_payload
        .get("room_ids")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| system_error("group check-in lock payload missing room_ids"))?;

    room_ids
        .iter()
        .map(|value| {
            let room_id = value
                .as_str()
                .ok_or_else(|| system_error("group check-in lock room id must be a string"))?;
            crate::aggregate_locks::room_key(room_id)
        })
        .collect()
}

struct ExistingGroupCheckinCommandContext {
    check_in_date: Option<String>,
    issued_at: DateTime<FixedOffset>,
}

async fn existing_group_checkin_command_context(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
) -> CommandResult<Option<ExistingGroupCheckinCommandContext>> {
    let row = sqlx::query(
        "SELECT intent_json, issued_at
         FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(system_error)?;

    let Some(row) = row else {
        return Ok(None);
    };
    let intent_json: String = row.get("intent_json");
    let issued_at: String = row.get("issued_at");
    let issued_at = DateTime::parse_from_rfc3339(&issued_at).map_err(system_error)?;
    let intent: serde_json::Value = serde_json::from_str(&intent_json).map_err(system_error)?;
    let Some(check_in_date) = intent
        .get("fields")
        .and_then(|fields| fields.get("check_in_date"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(Some(ExistingGroupCheckinCommandContext {
            check_in_date: None,
            issued_at,
        }));
    };

    parse_date(check_in_date).map_err(map_group_checkin_command_error)?;
    Ok(Some(ExistingGroupCheckinCommandContext {
        check_in_date: Some(check_in_date.to_string()),
        issued_at,
    }))
}

fn build_group_checkin_payment_origins(
    idempotency_key: &str,
    room_ids: &[String],
) -> CommandResult<std::collections::HashMap<String, OriginSideEffect>> {
    let mut origins = std::collections::HashMap::new();
    for (ordinal, room_id) in normalized_room_ids(room_ids).into_iter().enumerate() {
        origins.insert(
            room_id,
            OriginSideEffect::new(idempotency_key, ordinal as i64).map_err(system_error)?,
        );
    }
    Ok(origins)
}

#[allow(dead_code)]
pub async fn group_checkin(
    pool: &Pool<Sqlite>,
    user_id: Option<String>,
    req: GroupCheckinRequest,
) -> BookingResult<BookingGroup> {
    validate_group_checkin_request(&req)?;

    let mut tx = begin_immediate_tx(pool).await?;
    let group_id = group_checkin_tx(
        &mut tx,
        user_id.as_deref(),
        &req,
        None,
        Local::now().fixed_offset(),
    )
    .await?;
    tx.commit().await.map_err(BookingError::from)?;
    fetch_group(pool, &group_id).await
}

pub async fn group_checkin_idempotent(
    pool: &Pool<Sqlite>,
    user_id: Option<String>,
    ctx: &WriteCommandContext,
    mut req: GroupCheckinRequest,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    validate_group_checkin_request(&req).map_err(map_group_checkin_command_error)?;

    let existing_command_context = existing_group_checkin_command_context(pool, ctx).await?;
    let command_now = existing_command_context
        .as_ref()
        .map(|existing| existing.issued_at)
        .unwrap_or(ctx.issued_at);
    let effective_checkin_date = match req.check_in_date.clone() {
        Some(check_in_date) => check_in_date,
        None => existing_command_context
            .as_ref()
            .and_then(|existing| existing.check_in_date.clone())
            .unwrap_or_else(|| command_now.format("%Y-%m-%d").to_string()),
    };
    if req.check_in_date.is_none() {
        req.check_in_date = Some(effective_checkin_date.clone());
    }
    let paid_amount = req.paid_amount.unwrap_or(0);

    let hash_payload = build_group_checkin_hash_payload(&req);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("group.checkin.v1")),
        ("room_count", json!(req.room_ids.len())),
        ("guest_room_count", json!(req.guests_per_room.len())),
        (
            "guest_form_count",
            json!(req
                .guests_per_room
                .values()
                .map(|guests| guests.len())
                .sum::<usize>()),
        ),
        ("nights", json!(req.nights)),
        ("check_in_date", json!(effective_checkin_date.clone())),
        (
            "has_organizer_contact",
            json!(req.organizer_phone.is_some()),
        ),
        ("has_source", json!(req.source.is_some())),
        ("has_notes", json!(req.notes.is_some())),
        ("has_paid_amount", json!(req.paid_amount.is_some())),
        ("paid_amount_positive", json!(paid_amount > 0)),
    ])?;
    let summary =
        CommandLedgerSummary::new("Group check-in")?.with_business_date(effective_checkin_date)?;
    let runtime_lock_keys = group_checkin_lock_keys_from_payload(&hash_payload)?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_lock_key_deriver(group_checkin_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Group checked in")?)
        .with_outbox_event(OutboxEventSpec::new(
            "group.checked_in",
            OutboxAggregateKeySource::response_field("group", "id"),
            &["groups", "bookings", "rooms", "folio"],
        )?);

    let req_for_service = req;
    let user_id_for_service = user_id;
    let origin_idempotency_key = ctx.idempotency_key.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_pre_transaction_guard(
            ctx,
            request,
            move || async move {
                crate::aggregate_locks::global_manager()
                    .acquire(runtime_lock_keys)
                    .await
            },
            move |tx| {
                Box::pin(async move {
                    let payment_origins = if req_for_service.paid_amount.unwrap_or(0) > 0 {
                        Some(build_group_checkin_payment_origins(
                            &origin_idempotency_key,
                            &req_for_service.room_ids,
                        )?)
                    } else {
                        None
                    };
                    let group_id = group_checkin_tx(
                        tx,
                        user_id_for_service.as_deref(),
                        &req_for_service,
                        payment_origins.as_ref(),
                        command_now,
                    )
                    .await
                    .map_err(map_group_checkin_command_error)?;
                    let group = fetch_group_tx(tx, &group_id)
                        .await
                        .map_err(map_group_checkin_command_error)?;
                    serde_json::to_value(&group).map_err(system_error)
                })
            },
        )
        .await
}

/// Giá đã tính cho một phòng ở LƯỢT 1 của `group_checkin_tx` (giá tay hoặc
/// engine — xem nhánh `match` trong đó). Giữ lại để LƯỢT 2 dùng khi ghi
/// booking mà không phải gọi lại `calculate_stay_price_tx`, và để phân bổ
/// `paid_amount` theo tỉ lệ (`allocate_paid_amount_by_room_price`) — muốn
/// chia theo tỉ lệ thì phải biết tổng của MỌI phòng trước, nên việc TÍNH giá
/// và việc GHI booking không còn gộp một lượt như trước Task 15's review.
struct GroupRoomPricing {
    total_price: MoneyVnd,
    rate_overridden_at: Option<String>,
    pricing_snapshot: Option<String>,
}

async fn group_checkin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: Option<&str>,
    req: &GroupCheckinRequest,
    payment_origins_by_room: Option<&std::collections::HashMap<String, OriginSideEffect>>,
    now: DateTime<FixedOffset>,
) -> BookingResult<String> {
    let now_rfc3339 = now.to_rfc3339();
    let today_str = now.format("%Y-%m-%d").to_string();
    let is_reservation = req
        .check_in_date
        .as_ref()
        .map(|date| date != &today_str)
        .unwrap_or(false);
    let checkin_date = req.check_in_date.clone().unwrap_or(today_str);
    let checkin_naive = parse_date(&checkin_date)?;
    let checkout_naive = checkin_naive + Duration::days(req.nights as i64);
    let checkout_date = checkout_naive.format("%Y-%m-%d").to_string();

    validate_rooms_for_group(
        tx,
        &req.room_ids,
        is_reservation,
        &checkin_date,
        &checkout_date,
    )
    .await?;

    let group_id = uuid::Uuid::new_v4().to_string();
    let group_status = if is_reservation {
        GROUP_BOOKED
    } else {
        GROUP_ACTIVE
    };
    sqlx::query(
        "INSERT INTO booking_groups (
            id, group_name, organizer_name, organizer_phone, total_rooms, status, notes, created_by, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&group_id)
    .bind(&req.group_name)
    .bind(&req.organizer_name)
    .bind(req.organizer_phone.as_deref())
    .bind(req.room_ids.len() as i32)
    .bind(group_status)
    .bind(req.notes.as_deref())
    .bind(user_id)
    .bind(&now_rfc3339)
    .execute(&mut **tx)
    .await?;

    // Bốn giá trị dưới đây (trạng thái booking, mốc giờ check-in/check-out,
    // cửa sổ ngày định giá) không phụ thuộc room_id — giống nhau cho MỌI
    // phòng trong đoàn — nên tính một lần ở đây, dùng chung cho cả hai lượt
    // bên dưới, thay vì tính lại mỗi vòng lặp.
    let booking_status = if is_reservation {
        status::booking::BOOKED
    } else {
        status::booking::ACTIVE
    };
    let booking_type = if is_reservation {
        "reservation"
    } else {
        "walk-in"
    };
    let booking_checkin_at = if is_reservation {
        format!("{}T14:00:00+07:00", checkin_date)
    } else {
        now_rfc3339.clone()
    };
    let booking_checkout_at = if is_reservation {
        format!("{}T12:00:00+07:00", checkout_date)
    } else {
        (now + Duration::days(req.nights as i64)).to_rfc3339()
    };
    let pricing_start = if is_reservation {
        checkin_date.as_str()
    } else {
        booking_checkin_at.as_str()
    };
    let pricing_end = if is_reservation {
        checkout_date.as_str()
    } else {
        booking_checkout_at.as_str()
    };

    // LƯỢT 1: giá từng phòng — chỉ ĐỌC (`calculate_stay_price_tx` chỉ
    // SELECT, không ghi gì). Bắt buộc tách khỏi LƯỢT 2 (ghi) bên dưới:
    // `paid_amount` giờ chia THEO TỈ LỆ tổng tiền từng phòng
    // (`allocate_paid_amount_by_room_price`, thay cho chia đều theo số lượng
    // — xem doc-comment hàm đó) — muốn chia theo tỉ lệ thì phải biết tổng của
    // MỌI phòng trước, không thể vừa tính vừa ghi như một vòng lặp duy nhất.
    let mut room_pricing: Vec<GroupRoomPricing> = Vec::with_capacity(req.room_ids.len());
    for room_id in &req.room_ids {
        // Giá tay đè giá engine theo TỪNG phòng, và đè PHẲNG: tổng tiền là
        // `rate × nights`, không cộng thêm dòng nào engine tính — cùng luật
        // `check_in_tx` (Task 13) / `create_reservation_tx` (Task 14). Đoàn
        // gần như luôn mặc cả, thường một giá cho cả đoàn hoặc riêng vài
        // phòng, nên tra theo `room_id`; phòng không có trong map đi đúng
        // đường engine như hôm nay (`req.rate_override_per_room` rỗng ⇒ hành
        // vi không đổi so với trước Task 15).
        let (total_price, rate_overridden_at, pricing_snapshot) = match req
            .rate_override_per_room
            .get(room_id)
            .copied()
        {
            Some(rate) => {
                // `validate_group_checkin_request` đã chặn giá ngoài biên
                // trước khi tới được đây — đó là gate DUY NHẤT (xem comment
                // ở đó), nên KHÔNG lặp lại phép kiểm biên ở đây.
                let total =
                    crate::pricing::checked_mul_money(rate, i64::from(req.nights), "total_price")
                        .map_err(BookingError::validation)?;

                // Giá engine chỉ tính để LƯU LẠI trong pricing_snapshot cho
                // chủ khách sạn tra cứu sau này (đã giảm giá cho ai bao
                // nhiêu), không dùng làm tiền thật — nên lỗi ở bước này
                // không được làm hỏng cả lượt nhận đoàn. Lỗi thì lưu `null`
                // (không rõ), không lưu 0 — 0 sẽ đọc nhầm thành "engine
                // định giá phòng này bằng không".
                let engine_total = calculate_stay_price_tx(
                    tx,
                    room_id,
                    pricing_start,
                    pricing_end,
                    "nightly",
                    None,
                )
                .await
                .map(|pricing| pricing.total)
                .ok();

                let snapshot = merge_pricing_snapshot(
                    None,
                    "manual_rate",
                    json!({
                        "rate_per_night": rate,
                        "engine_total": engine_total,
                        "set_at": now_rfc3339.clone(),
                    }),
                );

                (total, Some(now_rfc3339.clone()), Some(snapshot))
            }
            None => {
                let pricing = calculate_stay_price_tx(
                    tx,
                    room_id,
                    pricing_start,
                    pricing_end,
                    "nightly",
                    None,
                )
                .await?;
                (pricing.total, None, None)
            }
        };

        room_pricing.push(GroupRoomPricing {
            total_price,
            rate_overridden_at,
            pricing_snapshot,
        });
    }

    // `paid_amount` chia THEO TỈ LỆ tổng tiền từng phòng vừa tính ở LƯỢT 1 —
    // xem `allocate_paid_amount_by_room_price` để biết vì sao (chia đều theo
    // số lượng có thể cấp một phòng nhiều hơn chính tổng tiền phòng đó) và
    // quy tắc làm tròn/rải dư.
    let room_totals: Vec<(String, MoneyVnd)> = req
        .room_ids
        .iter()
        .cloned()
        .zip(room_pricing.iter().map(|pricing| pricing.total_price))
        .collect();
    let paid_allocations_by_room =
        allocate_paid_amount_by_room_price(req.paid_amount.unwrap_or(0), &room_totals);

    let mut master_booking_id: Option<String> = None;

    // LƯỢT 2: ghi. `room_pricing` cùng độ dài, cùng thứ tự với `req.room_ids`
    // (mỗi vòng lặp ở LƯỢT 1 trên đẩy đúng một phần tử, theo đúng thứ tự
    // duyệt) nên zip theo vị trí ở đây không thể lệch phòng; dùng lại giá đã
    // tính, không gọi lại `calculate_stay_price_tx`.
    for (room_id, room_price) in req.room_ids.iter().zip(room_pricing) {
        let GroupRoomPricing {
            total_price,
            rate_overridden_at,
            pricing_snapshot,
        } = room_price;

        let paid_for_room = paid_allocations_by_room.get(room_id).copied().unwrap_or(0);
        let is_master = room_id == &req.master_room_id;
        let room_guests = req
            .guests_per_room
            .get(room_id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let guest_manifest = create_group_guest_manifest(
            tx,
            room_guests,
            &format!("Khách đoàn {} - {}", req.group_name, room_id),
            &now_rfc3339,
        )
        .await?;

        let booking_id = uuid::Uuid::new_v4().to_string();

        // Thu vượt tổng tiền CỦA PHÒNG NÀY dẫn tới đúng lỗ hổng `check_in_tx`/
        // `create_reservation_tx` đã vá: một booking có `paid_amount >
        // total_price` không có lối thoát nếu sau này bị `check_out_tx` từ
        // chối vì `already_paid > final_total`. Đặt SAU khi `total_price` đã
        // biết ở CẢ hai nhánh (giá tay lẫn giá engine) — `validate_group_checkin_request`
        // không biết giá từng phòng (chưa mở transaction, chưa gọi engine) nên
        // không chặn được ở đó, đúng lý do guard tương ứng nằm trong
        // `check_in_tx` chứ không phải `validate_check_in_request`. Đặt TRƯỚC
        // các ghi CỦA PHÒNG NÀY (INSERT bookings, charge, payment ngay dưới) —
        // toàn bộ `group_checkin_tx` chạy trong một transaction nên các phòng
        // đã ghi trước đó trong vòng lặp cũng được rollback theo khi hàm này
        // trả lỗi.
        //
        // Với `allocate_paid_amount_by_room_price` chia theo tỉ lệ, nhánh này
        // KHÔNG THỂ còn xảy ra khi `paid_amount <= Σ total_price` (chứng minh
        // trong doc-comment của hàm đó) — chỉ còn là LƯỚI ĐỠ cho một
        // `paid_amount` thu vượt tổng CẢ ĐOÀN, đúng vai trò báo cáo review
        // yêu cầu giữ lại.
        if paid_for_room > total_price {
            return Err(BookingError::validation(format!(
                "Khách trả {paid_for_room}đ cho phòng {room_id}, cao hơn tổng tiền {total_price}đ — sửa lại giá hoặc số tiền thu"
            )));
        }

        let deposit_amount = if is_reservation { paid_for_room } else { 0 };
        let guest_phone = room_guests.first().and_then(|guest| guest.phone.as_deref());

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout,
                nights, total_price, paid_amount, status, source, notes, created_by,
                booking_type, pricing_type, deposit_amount, guest_phone, scheduled_checkin,
                scheduled_checkout, group_id, is_master_room, pricing_snapshot,
                rate_overridden_at, created_at
             ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, ?, ?, 'nightly', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&booking_id)
        .bind(room_id)
        .bind(&guest_manifest.primary_guest_id)
        .bind(&booking_checkin_at)
        .bind(&booking_checkout_at)
        .bind(req.nights)
        .bind(total_price)
        .bind(booking_status)
        .bind(req.source.as_deref().unwrap_or("walk-in"))
        .bind(req.notes.as_deref())
        .bind(user_id)
        .bind(booking_type)
        .bind(deposit_amount)
        .bind(guest_phone)
        .bind(if is_reservation {
            Some(checkin_date.as_str())
        } else {
            None
        })
        .bind(if is_reservation {
            Some(checkout_date.as_str())
        } else {
            None
        })
        .bind(&group_id)
        .bind(if is_master { 1 } else { 0 })
        .bind(&pricing_snapshot)
        .bind(&rate_overridden_at)
        .bind(&now_rfc3339)
        .execute(&mut **tx)
        .await?;

        if is_master {
            master_booking_id = Some(booking_id.clone());
        }

        link_booking_guests(tx, &booking_id, &guest_manifest.guest_ids).await?;

        if !is_reservation {
            record_charge_tx(
                tx,
                &booking_id,
                total_price,
                "Tiền phòng (đoàn)",
                booking_checkin_at.clone(),
            )
            .await?;

            if paid_for_room > 0 {
                if let Some(origins) = payment_origins_by_room {
                    if let Some(origin) = origins.get(room_id) {
                        record_payment_with_origin_tx(
                            tx,
                            &booking_id,
                            paid_for_room,
                            "Thanh toán group check-in",
                            origin,
                        )
                        .await?;
                    } else {
                        record_payment_tx(
                            tx,
                            &booking_id,
                            paid_for_room,
                            "Thanh toán group check-in",
                        )
                        .await?;
                    }
                } else {
                    record_payment_tx(tx, &booking_id, paid_for_room, "Thanh toán group check-in")
                        .await?;
                }
            }
        } else if paid_for_room > 0 {
            if let Some(origins) = payment_origins_by_room {
                if let Some(origin) = origins.get(room_id) {
                    record_payment_with_origin_tx(
                        tx,
                        &booking_id,
                        paid_for_room,
                        "Đặt cọc đoàn",
                        origin,
                    )
                    .await?;
                } else {
                    record_payment_tx(tx, &booking_id, paid_for_room, "Đặt cọc đoàn").await?;
                }
            } else {
                record_payment_tx(tx, &booking_id, paid_for_room, "Đặt cọc đoàn").await?;
            }
        }

        insert_group_calendar_rows(
            tx,
            room_id,
            &booking_id,
            checkin_naive,
            checkout_naive,
            if is_reservation {
                status::calendar::BOOKED
            } else {
                status::calendar::OCCUPIED
            },
        )
        .await?;

        if !is_reservation {
            let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
                .bind(status::room::OCCUPIED)
                .bind(room_id)
                .bind(status::room::VACANT)
                .execute(&mut **tx)
                .await?;
            ensure_one_row_affected(result, format!("room {room_id} is no longer vacant"))?;
        }
    }

    if let Some(ref booking_id) = master_booking_id {
        sqlx::query("UPDATE booking_groups SET master_booking_id = ? WHERE id = ?")
            .bind(booking_id)
            .bind(&group_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(group_id)
}

fn normalized_booking_ids(booking_ids: &[String]) -> Vec<String> {
    let mut normalized = booking_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn validate_group_checkout_request(req: &GroupCheckoutRequest) -> BookingResult<()> {
    if normalized_booking_ids(&req.booking_ids).is_empty() {
        return Err(BookingError::validation(
            "Phải chọn ít nhất 1 phòng để checkout".to_string(),
        ));
    }
    if let Some(final_paid) = req.final_paid {
        validate_non_negative_booking_money(final_paid, "final_paid")?;
    }

    Ok(())
}

fn map_group_checkout_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) if message == "Phải chọn ít nhất 1 phòng để checkout" => {
            CommandError::user(codes::GROUP_CHECKOUT_SELECTION_REQUIRED, message)
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if message.contains(codes::CONFLICT_INVALID_STATE_TRANSITION) {
                return CommandError::user(codes::CONFLICT_INVALID_STATE_TRANSITION, message);
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy group ") => {
            CommandError::user(codes::GROUP_NOT_FOUND, message)
        }
        BookingError::NotFound(message)
            if message.starts_with("Booking ") && message.contains("không tìm thấy") =>
        {
            CommandError::user(codes::BOOKING_NOT_FOUND, message)
        }
        BookingError::NotFound(message) => CommandError::user(codes::BOOKING_NOT_FOUND, message),
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

fn build_group_checkout_hash_payload(req: &GroupCheckoutRequest) -> serde_json::Value {
    json!({
        "schema": "group.checkout.v1",
        "group_id": req.group_id.clone(),
        "booking_ids": normalized_booking_ids(&req.booking_ids),
        "final_paid_vnd_units": req.final_paid.unwrap_or(0),
    })
}

fn group_checkout_initial_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let group_id = hash_payload
        .get("group_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("group checkout lock payload missing group_id"))?;

    Ok(vec![crate::aggregate_locks::group_key(group_id)?])
}

#[cfg(test)]
struct GroupCheckoutLockState {
    selected_booking_room_map: std::collections::HashMap<String, String>,
    booking_ids_to_lock: Vec<String>,
    room_ids_to_lock: Vec<String>,
}

struct GroupCheckoutResolvedGuard {
    _guard: crate::aggregate_locks::AggregateLockGuard,
    locked_booking_room_map: std::collections::HashMap<String, String>,
    locked_payment_candidate_booking_ids: Vec<String>,
}

/// Pha 1 (dưới khoá `group:G`): xác định danh sách booking phải khoá.
///
/// Chỉ đọc id, không đọc phòng — đọc phòng ở pha này vẫn có thể cũ ngay khi
/// vừa đọc xong, vì `change_room` không cầm `group:`, chỉ cầm
/// `booking:`/`folio:`/`room:`. Xem `load_group_checkout_room_lock_state`.
async fn load_group_checkout_booking_ids_to_lock(
    pool: &Pool<Sqlite>,
    group_id: &str,
    selected_booking_ids: &[String],
    final_paid: Option<MoneyVnd>,
) -> BookingResult<Vec<String>> {
    let group_exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM booking_groups WHERE id = ? LIMIT 1")
            .bind(group_id)
            .fetch_optional(pool)
            .await?;
    if group_exists.is_none() {
        return Err(BookingError::not_found(format!(
            "Không tìm thấy group {}",
            group_id
        )));
    }

    let mut selected_query: sqlx::QueryBuilder<Sqlite> =
        sqlx::QueryBuilder::new("SELECT id FROM bookings WHERE group_id = ");
    selected_query.push_bind(group_id);
    selected_query.push(" AND id IN (");
    let mut selected_sep = selected_query.separated(", ");
    for booking_id in selected_booking_ids {
        selected_sep.push_bind(booking_id);
    }
    selected_sep.push_unseparated(")");

    let selected_rows = selected_query.build().fetch_all(pool).await?;
    let existing_selected_ids = selected_rows
        .into_iter()
        .map(|row| row.get::<String, _>("id"))
        .collect::<std::collections::HashSet<_>>();

    for booking_id in selected_booking_ids {
        if !existing_selected_ids.contains(booking_id) {
            return Err(BookingError::not_found(format!(
                "Booking {} không tìm thấy hoặc đã checkout",
                booking_id
            )));
        }
    }

    let mut booking_ids_to_lock = if final_paid.unwrap_or(0) > 0 {
        let rows = sqlx::query("SELECT id FROM bookings WHERE group_id = ?")
            .bind(group_id)
            .fetch_all(pool)
            .await?;
        rows.into_iter()
            .map(|row| row.get::<String, _>("id"))
            .collect::<Vec<_>>()
    } else {
        selected_booking_ids.to_vec()
    };

    booking_ids_to_lock.sort();
    booking_ids_to_lock.dedup();

    Ok(booking_ids_to_lock)
}

/// Pha 2 (dưới khoá `booking:`/`folio:` của mọi booking trong
/// `booking_ids_to_lock`): đọc phòng của từng booking. Đây mới là chân lý —
/// mọi lệnh dời phòng (`change_room`) buộc phải cầm khoá `booking:` trước khi
/// đổi `bookings.room_id`, nên đọc dưới khoá này không thể cũ.
async fn load_group_checkout_room_lock_state(
    pool: &Pool<Sqlite>,
    selected_booking_ids: &[String],
    booking_ids_to_lock: &[String],
) -> BookingResult<(std::collections::HashMap<String, String>, Vec<String>)> {
    let mut room_query: sqlx::QueryBuilder<Sqlite> =
        sqlx::QueryBuilder::new("SELECT id, room_id FROM bookings WHERE id IN (");
    let mut room_sep = room_query.separated(", ");
    for booking_id in booking_ids_to_lock {
        room_sep.push_bind(booking_id);
    }
    room_sep.push_unseparated(")");

    let rows = room_query.build().fetch_all(pool).await?;
    let mut booking_room_map = std::collections::HashMap::new();
    for row in rows {
        booking_room_map.insert(row.get::<String, _>("id"), row.get::<String, _>("room_id"));
    }

    let selected_booking_room_map = selected_booking_ids
        .iter()
        .filter_map(|booking_id| {
            booking_room_map
                .get(booking_id)
                .cloned()
                .map(|room_id| (booking_id.clone(), room_id))
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut room_ids_to_lock = booking_ids_to_lock
        .iter()
        .filter_map(|booking_id| booking_room_map.get(booking_id).cloned())
        .collect::<Vec<_>>();
    room_ids_to_lock.sort();
    room_ids_to_lock.dedup();

    Ok((selected_booking_room_map, room_ids_to_lock))
}

/// Wrapper mỏng cho helper test `group_checkout` (single-shot, không qua ba
/// pha aggregate lock) — gọi lần lượt hai hàm trên.
#[cfg(test)]
async fn load_group_checkout_lock_state(
    pool: &Pool<Sqlite>,
    group_id: &str,
    selected_booking_ids: &[String],
    final_paid: Option<MoneyVnd>,
) -> BookingResult<GroupCheckoutLockState> {
    let booking_ids_to_lock =
        load_group_checkout_booking_ids_to_lock(pool, group_id, selected_booking_ids, final_paid)
            .await?;
    let (selected_booking_room_map, room_ids_to_lock) =
        load_group_checkout_room_lock_state(pool, selected_booking_ids, &booking_ids_to_lock)
            .await?;

    Ok(GroupCheckoutLockState {
        selected_booking_room_map,
        booking_ids_to_lock,
        room_ids_to_lock,
    })
}

async fn resolve_group_checkout_locks(
    pool: Pool<Sqlite>,
    req: GroupCheckoutRequest,
) -> CommandResult<ResolvedWriteCommandGuard<GroupCheckoutResolvedGuard>> {
    // Pha 1: khoá group trước. Danh sách booking phải khoá chỉ đứng yên khi
    // đã cầm khoá này.
    let guard = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::group_key(&req.group_id)?])
        .await?;

    let unique_booking_ids = normalized_booking_ids(&req.booking_ids);
    let booking_ids_to_lock = load_group_checkout_booking_ids_to_lock(
        &pool,
        &req.group_id,
        &unique_booking_ids,
        req.final_paid,
    )
    .await
    .map_err(map_group_checkout_command_error)?;

    // Pha 2: booking + folio cho mọi booking sẽ khoá, hạng cao hơn `group:`.
    // `change_room` không cầm `group:`, nên phòng chỉ đứng yên từ đây trở đi.
    let mut booking_phase_keys = Vec::new();
    for booking_id in &booking_ids_to_lock {
        booking_phase_keys.push(crate::aggregate_locks::booking_key(booking_id)?);
        booking_phase_keys.push(crate::aggregate_locks::folio_key(booking_id)?);
    }

    let guard = guard
        .acquire_next(crate::aggregate_locks::global_manager(), booking_phase_keys)
        .await?;

    let (selected_booking_room_map, room_ids_to_lock) =
        load_group_checkout_room_lock_state(&pool, &unique_booking_ids, &booking_ids_to_lock)
            .await
            .map_err(map_group_checkout_command_error)?;

    // Pha 3: room, hạng cao hơn booking/folio.
    let mut room_phase_keys = Vec::new();
    for room_id in &room_ids_to_lock {
        room_phase_keys.push(crate::aggregate_locks::room_key(room_id)?);
    }

    let guard = guard
        .acquire_next(crate::aggregate_locks::global_manager(), room_phase_keys)
        .await?;
    let lock_keys = guard.keys().to_vec();

    Ok(ResolvedWriteCommandGuard::new(
        GroupCheckoutResolvedGuard {
            _guard: guard,
            locked_booking_room_map: selected_booking_room_map,
            locked_payment_candidate_booking_ids: booking_ids_to_lock,
        },
        lock_keys,
    ))
}

pub async fn group_checkout_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: GroupCheckoutRequest,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    validate_group_checkout_request(&req).map_err(|error| {
        map_group_checkout_command_error(error).with_request_id(ctx.request_id.clone())
    })?;

    let final_paid = req.final_paid.unwrap_or(0);
    let hash_payload = build_group_checkout_hash_payload(&req);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("group.checkout.v1")),
        ("group_present", json!(true)),
        (
            "booking_count",
            json!(normalized_booking_ids(&req.booking_ids).len()),
        ),
        ("final_paid_present", json!(req.final_paid.is_some())),
        ("final_paid_positive", json!(final_paid > 0)),
    ])?;
    let summary = CommandLedgerSummary::new("Group checkout")?.with_aggregate_ref(
        "group",
        "group",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("group:{}", req.group_id))
        .with_lock_key_deriver(group_checkout_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Group checked out")?)
        .with_outbox_event(OutboxEventSpec::new(
            "group.checked_out",
            OutboxAggregateKeySource::response_field("group", "group_id"),
            &["groups", "bookings", "rooms", "folio"],
        )?);

    let pool_for_locks = pool.clone();
    let req_for_locks = GroupCheckoutRequest {
        group_id: req.group_id.clone(),
        booking_ids: req.booking_ids.clone(),
        final_paid: req.final_paid,
    };
    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || resolve_group_checkout_locks(pool_for_locks, req_for_locks),
            move |tx, resolved| {
                Box::pin(async move {
                    let response = group_checkout_tx(
                        tx,
                        req,
                        &resolved.locked_booking_room_map,
                        &resolved.locked_payment_candidate_booking_ids,
                        Some(origin_key),
                    )
                    .await
                    .map_err(map_group_checkout_command_error)?;
                    serde_json::to_value(response).map_err(system_error)
                })
            },
        )
        .await
}

#[cfg(test)]
pub async fn group_checkout(pool: &Pool<Sqlite>, req: GroupCheckoutRequest) -> BookingResult<()> {
    validate_group_checkout_request(&req)?;
    let unique_booking_ids = normalized_booking_ids(&req.booking_ids);
    let lock_state =
        load_group_checkout_lock_state(pool, &req.group_id, &unique_booking_ids, req.final_paid)
            .await?;
    let mut lock_keys = vec![crate::aggregate_locks::group_key(&req.group_id)
        .map_err(|error| BookingError::validation(error.message))?];
    for booking_id in &lock_state.booking_ids_to_lock {
        lock_keys.push(
            crate::aggregate_locks::booking_key(booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
        );
        lock_keys.push(
            crate::aggregate_locks::folio_key(booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
        );
    }
    for room_id in &lock_state.room_ids_to_lock {
        lock_keys.push(
            crate::aggregate_locks::room_key(room_id)
                .map_err(|error| BookingError::validation(error.message))?,
        );
    }
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire(lock_keys)
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    group_checkout_tx(
        &mut tx,
        req,
        &lock_state.selected_booking_room_map,
        &lock_state.booking_ids_to_lock,
        None,
    )
    .await?;
    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}

pub(crate) async fn group_checkout_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: GroupCheckoutRequest,
    locked_booking_room_map: &std::collections::HashMap<String, String>,
    locked_payment_candidate_booking_ids: &[String],
    origin_key: Option<String>,
) -> BookingResult<GroupCheckoutResponse> {
    let now = Local::now().to_rfc3339();
    let unique_booking_ids = normalized_booking_ids(&req.booking_ids);
    let payment_origin = origin_key
        .as_ref()
        .map(|key| OriginSideEffect::new(key.clone(), 0))
        .transpose()?;
    let mut query_builder: sqlx::QueryBuilder<Sqlite> =
        sqlx::QueryBuilder::new("SELECT id, room_id FROM bookings WHERE group_id = ");
    query_builder.push_bind(&req.group_id);
    query_builder.push(" AND id IN (");
    let mut separated = query_builder.separated(", ");
    for id in &unique_booking_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    let rows = query_builder.build().fetch_all(&mut **tx).await?;
    let mut current_booking_room_map = std::collections::HashMap::new();
    for row in rows {
        let id: String = row.get("id");
        let room_id: String = row.get("room_id");
        current_booking_room_map.insert(id, room_id);
    }
    ensure_group_checkout_room_map_still_locked(
        &req.group_id,
        &unique_booking_ids,
        locked_booking_room_map,
        &current_booking_room_map,
    )?;

    let mut room_ids = unique_booking_ids
        .iter()
        .filter_map(|booking_id| current_booking_room_map.get(booking_id).cloned())
        .collect::<Vec<_>>();
    room_ids.sort();
    room_ids.dedup();

    let mut qb = sqlx::QueryBuilder::new("UPDATE bookings SET status = ");
    qb.push_bind(status::booking::CHECKED_OUT);
    qb.push(", actual_checkout = ");
    qb.push_bind(&now);
    qb.push(" WHERE group_id = ");
    qb.push_bind(&req.group_id);
    qb.push(" AND status = ");
    qb.push_bind(status::booking::ACTIVE);
    qb.push(" AND id IN (");
    let mut sep = qb.separated(", ");
    for id in &unique_booking_ids {
        sep.push_bind(id);
    }
    sep.push_unseparated(")");
    let result = qb.build().execute(&mut **tx).await?;
    ensure_rows_affected(
        result,
        unique_booking_ids.len() as u64,
        format!(
            "one or more bookings in group {} are no longer active",
            req.group_id
        ),
    )?;

    let mut qb = sqlx::QueryBuilder::new("UPDATE rooms SET status = ");
    qb.push_bind(status::room::CLEANING);
    qb.push(" WHERE status = ");
    qb.push_bind(status::room::OCCUPIED);
    qb.push(" AND id IN (");
    let mut sep = qb.separated(", ");
    for rid in &room_ids {
        sep.push_bind(rid);
    }
    sep.push_unseparated(")");
    let result = qb.build().execute(&mut **tx).await?;
    ensure_rows_affected(
        result,
        room_ids.len() as u64,
        format!(
            "one or more rooms in group {} are no longer occupied",
            req.group_id
        ),
    )?;

    let mut qb = sqlx::QueryBuilder::new(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at) ",
    );
    qb.push_values(&room_ids, |mut b, rid| {
        b.push_bind(uuid::Uuid::new_v4().to_string())
            .push_bind(rid)
            .push_bind("needs_cleaning")
            .push_bind(&now)
            .push_bind(&now);
    });
    qb.build().execute(&mut **tx).await?;

    // Snapshot `room_stays` before the DELETE below wipes `room_calendar` —
    // the same move `check_out_tx` (stay_lifecycle.rs) makes on the
    // single-booking path, and for the same reason: after checkout the
    // snapshot is the only place the invoice can learn that a booking slept in
    // more than one room. `change_room_tx` did write a snapshot at move time,
    // but nothing has refreshed it since — an `extend_stay_tx` after the move
    // added `room_calendar` rows and left it behind — so skipping this leaves
    // a group booking's invoice splitting its money over the wrong nights.
    //
    // No truncation here, unlike `check_out_tx`: group checkout never settles
    // fewer nights than booked — it leaves `bookings.nights` and `total_price`
    // untouched (see the status UPDATE above), so `room_calendar` already
    // holds exactly the nights being charged. A group is at most ~10 bookings,
    // so a per-booking UPDATE is cheap.
    for booking_id in &unique_booking_ids {
        let room_stays = room_calendar_stays_tx(tx, booking_id).await?;
        if room_stays.is_empty() {
            continue;
        }

        let existing_snapshot: Option<String> =
            sqlx::query_scalar("SELECT pricing_snapshot FROM bookings WHERE id = ?")
                .bind(booking_id)
                .fetch_optional(&mut **tx)
                .await?
                .flatten();
        let merged_snapshot = merge_pricing_snapshot(
            existing_snapshot.as_deref(),
            "room_stays",
            room_stays_to_json(&room_stays),
        );

        sqlx::query("UPDATE bookings SET pricing_snapshot = ? WHERE id = ?")
            .bind(&merged_snapshot)
            .bind(booking_id)
            .execute(&mut **tx)
            .await?;
    }

    let mut qb = sqlx::QueryBuilder::new("DELETE FROM room_calendar WHERE booking_id IN (");
    let mut sep = qb.separated(", ");
    for id in &unique_booking_ids {
        sep.push_bind(id);
    }
    sep.push_unseparated(")");
    qb.build().execute(&mut **tx).await?;

    maybe_reassign_master_booking(tx, &req.group_id, &unique_booking_ids).await?;

    let remaining_active_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = ?")
            .bind(&req.group_id)
            .bind(status::booking::ACTIVE)
            .fetch_one(&mut **tx)
            .await?;
    let group_status = if remaining_active_count == 0 {
        GROUP_COMPLETED
    } else {
        GROUP_PARTIAL_CHECKOUT
    };

    sqlx::query("UPDATE booking_groups SET status = ? WHERE id = ?")
        .bind(group_status)
        .bind(&req.group_id)
        .execute(&mut **tx)
        .await?;

    if let Some(final_paid) = req.final_paid.filter(|amount| *amount > 0) {
        let payment_candidate_ids = normalized_booking_ids(locked_payment_candidate_booking_ids);
        if payment_candidate_ids.is_empty() {
            return Err(invalid_state_transition(
                "group checkout payment candidates changed before checkout",
            ));
        }

        let mut qb = sqlx::QueryBuilder::new("SELECT id FROM bookings WHERE group_id = ");
        qb.push_bind(&req.group_id);
        qb.push(" AND id IN (");
        {
            let mut sep = qb.separated(", ");
            for booking_id in &payment_candidate_ids {
                sep.push_bind(booking_id);
            }
            sep.push_unseparated(") ORDER BY CASE WHEN status = ");
        }
        qb.push_bind(status::booking::ACTIVE);
        qb.push(" THEN 0 ELSE 1 END, created_at ASC LIMIT 1");
        let target_booking: Option<(String,)> =
            qb.build_query_as().fetch_optional(&mut **tx).await?;
        let target_booking = target_booking.ok_or_else(|| {
            invalid_state_transition("group checkout payment target changed before checkout")
        })?;

        if let Some(origin) = payment_origin.as_ref() {
            record_payment_with_origin_tx(
                tx,
                &target_booking.0,
                final_paid,
                "Thanh toán group checkout",
                origin,
            )
            .await?;
        } else {
            record_payment_tx(
                tx,
                &target_booking.0,
                final_paid,
                "Thanh toán group checkout",
            )
            .await?;
        }
    }

    let checked_out_count = unique_booking_ids.len();
    Ok(GroupCheckoutResponse {
        ok: true,
        group_id: req.group_id,
        booking_ids: unique_booking_ids,
        checked_out_count,
        status: group_status.to_string(),
    })
}

pub(crate) fn ensure_group_checkout_room_map_still_locked(
    group_id: &str,
    booking_ids: &[String],
    locked_booking_room_map: &std::collections::HashMap<String, String>,
    current_booking_room_map: &std::collections::HashMap<String, String>,
) -> BookingResult<()> {
    for booking_id in booking_ids {
        if current_booking_room_map.get(booking_id) != locked_booking_room_map.get(booking_id) {
            return Err(invalid_state_transition(format!(
                "one or more bookings in group {group_id} changed rooms before checkout"
            )));
        }
    }

    Ok(())
}

fn validate_group_checkin_request(req: &GroupCheckinRequest) -> BookingResult<()> {
    if req.room_ids.is_empty() {
        return Err(BookingError::validation(
            "Phải chọn ít nhất 1 phòng".to_string(),
        ));
    }
    if req.nights <= 0 {
        return Err(BookingError::validation("Số đêm phải > 0".to_string()));
    }
    if let Some(paid_amount) = req.paid_amount {
        validate_non_negative_booking_money(paid_amount, "paid_amount")?;
    }
    if !req.room_ids.contains(&req.master_room_id) {
        return Err(BookingError::validation(
            "Phòng đại diện phải nằm trong danh sách phòng".to_string(),
        ));
    }
    let unique_room_count = req
        .room_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    if unique_room_count != req.room_ids.len() {
        return Err(BookingError::validation(
            "Phòng không được lặp trong cùng một group".to_string(),
        ));
    }

    // Biên trước, phép nhân sau — cùng thứ tự `validate_check_in_request`
    // (Task 13, stay_lifecycle.rs) / `validate_reservation_rate_override`
    // (Task 14, reservation_lifecycle.rs): chặn giá ngoài biên NGAY TẠI ĐÂY,
    // trước khi `group_checkin_tx` có cơ hội đưa bất cứ giá trị nào trong map
    // vào `checked_mul_money` — nếu không, một giá trị tràn số/không an toàn có
    // thể trả thẳng ra người dùng một thông báo overflow TIẾNG ANH, hoặc — với
    // giá âm — lọt xuống guard thu-vượt-tổng bên dưới và đọc nhầm một tổng âm
    // ra như một mức giá.
    //
    // Đây là GATE DUY NHẤT cho `rate_override_per_room` — KHÔNG có bản sao
    // "belt-and-braces" bên trong `group_checkin_tx` như `check_in_tx` có cho
    // `rate_override_per_night`. Lý do: mọi validation KHÁC của chính hàm này
    // (số đêm, phòng đại diện, phòng trùng ở trên) cũng chỉ kiểm một lần, một
    // chỗ — `group_checkin_tx` chỉ có đúng một lối vào, và cả hai người gọi nó
    // (`group_checkin` lẫn `group_checkin_idempotent`) đều bắt buộc gọi hàm
    // này trước. Thêm một bản sao thứ hai sẽ không có test nào khiến nó đỏ nếu
    // xoá đi (xem báo cáo self-review) — khác `check_in_tx`, nơi bản sao có lý
    // do phòng thủ chiều sâu vì hàm đó lịch sử được gọi từ nhiều chỗ hơn.
    //
    // Cũng kiểm luôn khoá của map phải nằm trong `room_ids`: một khoá lạ (gõ
    // sai `room_id`, hoặc phòng đã bị bỏ khỏi đoàn nhưng bảng giá tay chưa cập
    // nhật) sẽ bị `group_checkin_tx` ÂM THẦM bỏ qua vì vòng lặp ở đó chỉ tra
    // cứu map theo từng `room_id` trong `room_ids` — với một trường tiền bạc,
    // im lặng bỏ qua một mục có khả năng do nhầm lẫn còn rủi ro hơn từ chối rõ
    // ràng ngay từ đầu.
    for (room_id, rate) in &req.rate_override_per_room {
        if !req.room_ids.contains(room_id) {
            return Err(BookingError::validation(format!(
                "Phòng {room_id} trong bảng giá tay không nằm trong danh sách phòng của đoàn"
            )));
        }
        if *rate <= 0 || *rate > MAX_RATE_PER_NIGHT_VND {
            return Err(BookingError::validation(
                "Giá mỗi đêm không hợp lệ".to_string(),
            ));
        }
    }

    Ok(())
}

async fn validate_rooms_for_group(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_ids: &[String],
    is_reservation: bool,
    checkin_date: &str,
    checkout_date: &str,
) -> BookingResult<()> {
    for room_id in room_ids {
        let room_status = sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
            .bind(room_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| BookingError::not_found(format!("Phòng {} không tồn tại", room_id)))?;

        if !is_reservation && room_status != status::room::VACANT {
            return Err(BookingError::conflict(format!(
                "Phòng {} không trống (status: {})",
                room_id, room_status
            )));
        }

        let conflicts: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ?",
        )
        .bind(room_id)
        .bind(checkin_date)
        .bind(checkout_date)
        .fetch_one(&mut **tx)
        .await?;

        if conflicts.0 > 0 {
            return Err(BookingError::conflict(format!(
                "Phòng {} có lịch trùng trong khoảng ngày đã chọn",
                room_id
            )));
        }
    }

    Ok(())
}

async fn maybe_reassign_master_booking(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    group_id: &str,
    checked_out_booking_ids: &[String],
) -> BookingResult<()> {
    let current_master = sqlx::query_scalar::<_, String>(
        "SELECT master_booking_id FROM booking_groups WHERE id = ? LIMIT 1",
    )
    .bind(group_id)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(current_master) = current_master else {
        return Ok(());
    };

    if !checked_out_booking_ids.contains(&current_master) {
        return Ok(());
    }

    let next_master = sqlx::query_scalar::<_, String>(
        "SELECT id FROM bookings WHERE group_id = ? AND status = ? ORDER BY created_at ASC LIMIT 1",
    )
    .bind(group_id)
    .bind(status::booking::ACTIVE)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(next_master) = next_master {
        sqlx::query("UPDATE bookings SET is_master_room = 0 WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE bookings SET is_master_room = 1 WHERE id = ?")
            .bind(&next_master)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE booking_groups SET master_booking_id = ? WHERE id = ?")
            .bind(&next_master)
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query("UPDATE bookings SET is_master_room = 0 WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE booking_groups SET master_booking_id = NULL WHERE id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

async fn insert_group_calendar_rows(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    calendar_status: &str,
) -> BookingResult<()> {
    insert_room_calendar_rows(tx, room_id, booking_id, from, to, calendar_status).await
}

async fn fetch_group_tx(
    tx: &mut Transaction<'_, Sqlite>,
    group_id: &str,
) -> BookingResult<BookingGroup> {
    let row = sqlx::query(
        "SELECT id, group_name, master_booking_id, organizer_name, organizer_phone,
                total_rooms, status, notes, created_by, created_at
         FROM booking_groups
         WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(&mut **tx)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Không tìm thấy group {}", group_id)))?;

    Ok(BookingGroup {
        id: row.get("id"),
        group_name: row.get("group_name"),
        master_booking_id: row.get("master_booking_id"),
        organizer_name: row.get("organizer_name"),
        organizer_phone: row.get("organizer_phone"),
        total_rooms: row.get("total_rooms"),
        status: row.get("status"),
        notes: row.get("notes"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

#[allow(dead_code)]
async fn fetch_group(pool: &Pool<Sqlite>, group_id: &str) -> BookingResult<BookingGroup> {
    let row = sqlx::query(
        "SELECT id, group_name, master_booking_id, organizer_name, organizer_phone,
                total_rooms, status, notes, created_by, created_at
         FROM booking_groups
         WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(pool)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Không tìm thấy group {}", group_id)))?;

    Ok(BookingGroup {
        id: row.get("id"),
        group_name: row.get("group_name"),
        master_booking_id: row.get("master_booking_id"),
        organizer_name: row.get("organizer_name"),
        organizer_phone: row.get("organizer_phone"),
        total_rooms: row.get("total_rooms"),
        status: row.get("status"),
        notes: row.get("notes"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

fn parse_date(value: &str) -> BookingResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| BookingError::datetime_parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_positive_money_evenly_preserves_total_without_fractional_rows() {
        let allocation = allocate_positive_money_evenly(100_000, 3);

        assert_eq!(allocation, vec![33_334, 33_333, 33_333]);
        assert_eq!(allocation.iter().sum::<MoneyVnd>(), 100_000);
    }

    /// Ranh giới chính review Task 15 chỉ ra: thu ĐỦ (paid_amount = tổng cả
    /// đoàn) trên hai phòng CHÊNH giá nhau phải cấp cho mỗi phòng ĐÚNG BẰNG
    /// tổng của chính nó — không hơn, không kém. Số liệu lấy nguyên từ ví dụ
    /// trong báo cáo review (G-R1 override 400.000×2, G-R2 engine 500.000×2).
    #[test]
    fn allocate_paid_amount_by_room_price_gives_each_room_exactly_its_own_total_when_paid_in_full()
    {
        let room_totals = vec![
            ("G-R1".to_string(), 800_000),
            ("G-R2".to_string(), 1_000_000),
        ];

        let allocations = allocate_paid_amount_by_room_price(1_800_000, &room_totals);

        assert_eq!(allocations.get("G-R1").copied(), Some(800_000));
        assert_eq!(allocations.get("G-R2").copied(), Some(1_000_000));
    }

    /// Trả một phần, không chia hết theo tỉ lệ (1.000.000 × 800.000/1.800.000
    /// = 444.444,44...) — ghim quy tắc làm tròn XUỐNG + rải dư 1 đồng theo
    /// `room_id` đã sắp, và ghim tổng cộng lại đúng bằng `paid_amount`, không
    /// lệch một đồng dù làm tròn.
    #[test]
    fn allocate_paid_amount_by_room_price_sums_exactly_when_not_evenly_divisible() {
        let room_totals = vec![
            ("G-R1".to_string(), 800_000),
            ("G-R2".to_string(), 1_000_000),
        ];

        let allocations = allocate_paid_amount_by_room_price(1_000_000, &room_totals);

        assert_eq!(allocations.get("G-R1").copied(), Some(444_445));
        assert_eq!(allocations.get("G-R2").copied(), Some(555_555));
        assert_eq!(allocations.values().sum::<MoneyVnd>(), 1_000_000);
    }

    /// Mọi phòng cùng giá phải cho kết quả giống hệt `allocate_positive_money_evenly`
    /// — cùng số phòng, cùng `paid_amount`, cùng quy tắc rải dư theo thứ tự đã
    /// sắp (ở đây "A" < "B" < "C" nên đã đúng thứ tự `room_ids` luôn).
    #[test]
    fn allocate_paid_amount_by_room_price_matches_even_split_when_prices_equal() {
        let room_totals = vec![
            ("A".to_string(), 500_000),
            ("B".to_string(), 500_000),
            ("C".to_string(), 500_000),
        ];

        let allocations = allocate_paid_amount_by_room_price(100_000, &room_totals);
        let even_split = allocate_positive_money_evenly(100_000, 3);

        assert_eq!(allocations.get("A").copied(), Some(even_split[0]));
        assert_eq!(allocations.get("B").copied(), Some(even_split[1]));
        assert_eq!(allocations.get("C").copied(), Some(even_split[2]));
    }

    #[test]
    fn map_group_checkin_command_error_maps_invalid_state_transition_code() {
        let error = map_group_checkin_command_error(BookingError::conflict(format!(
            "{}: room R101 is no longer vacant",
            codes::CONFLICT_INVALID_STATE_TRANSITION
        )));

        assert_eq!(error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
    }

    fn minimal_group_request(room_ids: &[&str]) -> GroupCheckinRequest {
        GroupCheckinRequest {
            group_name: "Group".to_string(),
            organizer_name: "Organizer".to_string(),
            organizer_phone: None,
            check_in_date: None,
            room_ids: room_ids.iter().map(|id| id.to_string()).collect(),
            master_room_id: room_ids[0].to_string(),
            guests_per_room: Default::default(),
            nights: 2,
            source: None,
            notes: None,
            paid_amount: None,
            rate_override_per_room: Default::default(),
        }
    }

    /// Cùng cấu trúc `validate_check_in_request_rejects_bad_rates_before_any_multiply`
    /// (Task 13, stay_lifecycle.rs): `huge` đủ lớn để vượt cả
    /// `MAX_RATE_PER_NIGHT_VND` VÀ vượt biên an toàn số nguyên (giả sử biên bị
    /// gỡ, phép nhân sẽ ăn giá trị này và trả một thông báo overflow TIẾNG
    /// ANH); `negative` không tràn số (nhân với đêm dương vẫn ra một số "hợp
    /// lệ") nên nếu biên bị gỡ, request sẽ lọt qua `checked_mul_money` — bug
    /// thật đang chặn.
    #[test]
    fn validate_group_checkin_request_rejects_bad_rate_override_bounds() {
        let huge = 9_500_000_000_000_000_i64;
        let negative = -500_000_i64;

        for rate in [huge, negative] {
            let mut req = minimal_group_request(&["room-a", "room-b"]);
            req.rate_override_per_room
                .insert("room-a".to_string(), rate);

            let error = validate_group_checkin_request(&req).unwrap_err();
            assert_eq!(
                error,
                BookingError::validation("Giá mỗi đêm không hợp lệ".to_string()),
                "giá {rate} phải bị chặn bằng thông báo biên tiếng Việt, nhận được: {error:?}"
            );
        }
    }

    #[test]
    fn validate_group_checkin_request_rejects_zero_rate_override() {
        let mut req = minimal_group_request(&["room-a", "room-b"]);
        req.rate_override_per_room.insert("room-a".to_string(), 0);

        let error = validate_group_checkin_request(&req).unwrap_err();
        assert_eq!(
            error,
            BookingError::validation("Giá mỗi đêm không hợp lệ".to_string())
        );
    }

    #[test]
    fn validate_group_checkin_request_rejects_above_cap_rate_override() {
        let mut req = minimal_group_request(&["room-a", "room-b"]);
        req.rate_override_per_room
            .insert("room-a".to_string(), MAX_RATE_PER_NIGHT_VND + 1);

        let error = validate_group_checkin_request(&req).unwrap_err();
        assert_eq!(
            error,
            BookingError::validation("Giá mỗi đêm không hợp lệ".to_string())
        );
    }

    /// Biên dương: đúng bằng trần phải QUA — chứng minh phép so là `>`, không
    /// phải `>=`, nên không vô tình xiết chặt hơn `check_in_tx`/`create_reservation_tx`
    /// dùng cùng hằng số này.
    #[test]
    fn validate_group_checkin_request_accepts_rate_override_at_cap() {
        let mut req = minimal_group_request(&["room-a", "room-b"]);
        req.rate_override_per_room
            .insert("room-a".to_string(), MAX_RATE_PER_NIGHT_VND);

        assert!(validate_group_checkin_request(&req).is_ok());
    }

    /// Một khoá lạ trong map (gõ sai `room_id`, hoặc phòng đã bị bỏ khỏi đoàn
    /// nhưng bảng giá tay chưa cập nhật) phải bị từ chối rõ ràng — không bị
    /// `group_checkin_tx` âm thầm bỏ qua.
    #[test]
    fn validate_group_checkin_request_rejects_rate_override_for_room_not_in_group() {
        let mut req = minimal_group_request(&["room-a", "room-b"]);
        req.rate_override_per_room
            .insert("room-c".to_string(), 400_000);

        let error = validate_group_checkin_request(&req).unwrap_err();
        assert!(
            matches!(&error, BookingError::Validation(message) if message.contains("room-c")),
            "lỗi phải nêu rõ phòng room-c không thuộc đoàn, nhận được: {error:?}"
        );
    }

    /// `canonicalize_json_value` (dùng bởi `stable_request_hash`) chỉ sắp khoá
    /// của OBJECT, giữ nguyên thứ tự phần tử của MẢNG. `HashMap` không có thứ
    /// tự lặp ổn định giữa hai lần dựng độc lập, kể cả cùng nội dung — nếu
    /// trường này lỡ mã hoá thành mảng `[[room_id, rate], ...]` thay vì object,
    /// hai lượt gọi lại giống hệt nhau dưới cùng idempotency key có thể băm ra
    /// hai giá trị khác nhau, và một retry hợp lệ sẽ dừng replay trong im
    /// lặng — pin trực tiếp phần tích hợp Task 15 thêm vào, khác với test biên
    /// giá ở trên.
    #[test]
    fn group_checkin_hash_payload_encodes_rate_override_as_object_not_array() {
        let mut req = minimal_group_request(&["room-a", "room-b"]);
        req.rate_override_per_room
            .insert("room-a".to_string(), 400_000);
        req.rate_override_per_room
            .insert("room-b".to_string(), 500_000);

        let payload = build_group_checkin_hash_payload(&req);

        assert!(
            payload["rate_override_per_room"].is_object(),
            "rate_override_per_room phải là object để canonicalize_json_value sắp khoá trước khi băm, nhận được: {:?}",
            payload["rate_override_per_room"]
        );
        assert_eq!(payload["rate_override_per_room"]["room-a"], 400_000);
        assert_eq!(payload["rate_override_per_room"]["room-b"], 500_000);
    }
}
