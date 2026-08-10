use crate::{
    app_error::{codes, CommandError, CommandResult},
    models::{
        status, BackfillStayRequest, CheckInRequest, CreateGuestRequest, CreateReservationRequest,
    },
    queries::rooms::assistant_queries::{load_free_rooms_between, load_room_status_now},
    services::booking::{pricing_service, reservation_lifecycle::MAX_RESERVATION_NIGHTS},
};
use chrono::NaiveDate;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Pool, Sqlite};
use std::collections::BTreeMap;

pub const CHECK_IN_ACTION_KIND: &str = "check_in";
pub const RESERVE_ACTION_KIND: &str = "reserve";
pub const BACKFILL_ACTION_KIND: &str = "backfill";

/// Payload của một thẻ — **chính** kiểu request mà lệnh PMS tương ứng nhận.
///
/// `#[serde(untagged)]` nên nó serialize ra đúng object của biến thể bên trong,
/// không thêm lớp bọc nào: frontend đọc `{ kind, payload, … }` và chuyển thẳng
/// `payload` sang lệnh (`invokeWriteCommand(command, { req: payload })`).
/// `kind` ở `ProposedAction` mới là thứ phân biệt, đúng như union
/// `ProposedAction` khai bên `types/assistant.ts`.
///
/// Là enum chứ không phải `serde_json::Value`: mỗi loại thẻ mang đúng hình dạng
/// lệnh nó gọi, nên trộn nhầm là lỗi biên dịch chứ không phải một lệnh PMS hỏng
/// lúc lễ tân bấm *Đồng ý*. Và vì payload **là** kiểu thật, thêm một trường vào
/// `CreateReservationRequest` sẽ làm test "thẻ hiện đủ trường payload" đỏ.
/// **Về `untagged` và thứ tự khai báo:** luật "biến thể khai trước nuốt biến thể
/// khai sau nếu hình dạng khớp" là luật của **`Deserialize`**, và enum này chỉ
/// derive `Serialize` — serialize một biến thể untagged chỉ là serialize giá trị
/// bên trong, không có phép thử-lần-lượt nào để nuốt ai cả. Ghi ra vì hai biến
/// thể `CheckIn` và `Backfill` **có** trùng hình dạng một phần (`room_id` +
/// `guests`), nên ngày ai đó thêm `Deserialize` vào đây thì `Backfill` sẽ bị
/// `CheckIn` nuốt trong im lặng. `the_backfill_payload_goes_on_the_wire_flat…`
/// ghim bộ khoá thật của từng biến thể, nên phép trộn ấy sẽ làm test đỏ.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ActionPayload {
    CheckIn(CheckInRequest),
    Reserve(CreateReservationRequest),
    Backfill(BackfillStayRequest),
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposedAction {
    pub kind: String,
    pub payload: ActionPayload,
    pub display: BTreeMap<String, String>,
    pub preview: Value,
    pub warnings: Vec<String>,
    pub built_at_ms: i64,
}

#[derive(Debug)]
pub enum DraftOutcome {
    Ready(Box<ProposedAction>),
    MissingFields(Vec<String>),
    /// Người dùng nêu một ngày nhận phòng **không phải hôm nay**. Không dựng
    /// thẻ: lệnh `check_in` đóng dấu `Local::now()` lúc bấm nút, nên một thẻ
    /// nhận phòng cho ngày khác là một thẻ nói dối.
    ///
    /// `requested` giữ **nguyên văn** chuỗi model gửi lên, để câu trả về cho
    /// model nhắc lại đúng ngày người dùng đã nêu chứ không phải một ngày đã
    /// bị chuẩn hoá lại.
    WrongDateForCheckIn {
        requested: String,
        is_future: bool,
    },
    /// `check_in_date` có mặt nhưng không đọc được thành một ngày lịch (model
    /// gửi thẳng "ngày 8" chẳng hạn).
    ///
    /// Tách khỏi `WrongDateForCheckIn` chứ không gộp vào với một `is_future`
    /// đoán bừa: gộp thì "ngày 8" sẽ bị đoán là quá khứ rồi đẩy sang
    /// `draft_backfill`, tức ghi bù cho một kỳ ở còn chưa xảy ra. Đoán sai
    /// hướng còn tệ hơn không đoán.
    UnreadableCheckInDate {
        requested: String,
    },
    /// `draft_reserve` được gọi với một ngày nhận **không ở tương lai**.
    ///
    /// Đối xứng với `WrongDateForCheckIn`, và cần thiết vì cùng một lý do: đặt
    /// phòng trước cho hôm nay là nhận phòng (`check_in` đóng dấu `Local::now()`
    /// và bắt đầu tính tiền ngay), còn đặt phòng trước cho hôm qua là ghi bù.
    /// Dựng một `booking_type='reservation'` cho ngày đã qua thì phòng bị giữ
    /// chỗ ngược về quá khứ và không lượt ở nào khớp với nó.
    ///
    /// `is_today` chứ không `is_future`: ở đây chỉ có hai hướng sai, và gọi
    /// đúng tên hướng nào giúp vòng lặp chỉ sang đúng tool.
    WrongDateForReserve {
        requested: String,
        is_today: bool,
    },
    /// Ngày trả không sau ngày nhận. Không tự sửa thành +1 đêm: số đêm là thứ
    /// khách trả tiền, và đoán hộ một đêm là đoán hộ một khoản tiền.
    CheckOutNotAfterCheckIn {
        check_in_date: String,
        check_out_date: String,
    },
    /// Một trong hai ngày của `draft_reserve` không đọc được thành ngày lịch.
    ///
    /// Tách khỏi `UnreadableCheckInDate` (của `draft_check_in`) vì nó phải nói
    /// ra **ô nào** hỏng: thẻ đặt phòng có hai ô ngày, và một lời "ngày không
    /// đọc được" không chỉ rõ ô nào là lời mời model sửa nhầm ô còn lại.
    UnreadableReserveDate {
        field: &'static str,
        requested: String,
    },
    /// `draft_backfill` được gọi với một ngày nhận **không ở quá khứ**.
    ///
    /// Cạnh thứ ba của cùng một luật, và `backfill_stay` tự kiểm lại y hệt
    /// (`validate_backfill_request`: `check_in >= today` ⇒ lỗi). Chặn ở đây để
    /// lời từ chối là một câu chỉ đường cho model, không phải một lỗi ràng buộc
    /// thô nổ ra sau khi lễ tân đã bấm *Đồng ý*.
    ///
    /// `is_today` chứ không `is_future`: hai hướng sai chỉ về hai tool khác nhau
    /// (`draft_check_in` cho hôm nay, `draft_reserve` cho tương lai).
    WrongDateForBackfill {
        requested: String,
        is_today: bool,
    },
    /// Một trong **ba** ô ngày của `draft_backfill` không đọc được thành ngày
    /// lịch. Gọi tên đúng ô, cùng lý do như `UnreadableReserveDate`.
    UnreadableBackfillDate {
        field: &'static str,
        requested: String,
    },
    /// Khách **còn ở** nhưng ngày trả dự kiến không sau hôm nay.
    ///
    /// `backfill_stay` từ chối ca này ("Ngày ra dự kiến phải sau hôm nay") vì
    /// một lượt ở đang mở mà đã hết hạn từ hôm qua thì không phải khách còn ở —
    /// nó là một lượt ở đã kết thúc mà chưa ai trả phòng.
    ExpectedCheckoutNotAfterToday {
        requested: String,
        today: String,
    },
    /// Khách **đã trả phòng** nhưng ngày trả nằm ở tương lai.
    ///
    /// `backfill_stay` từ chối ("Khách đã trả phòng thì ngày ra không được ở
    /// tương lai"). Đây gần như luôn là model điền nhầm ô: khách chưa rời phòng
    /// thì `check_out_date` phải bỏ trống và ngày ấy thuộc về
    /// `expected_checkout_date`. Nói ra đúng chỗ nhầm, không tự chuyển ô hộ —
    /// chuyển hộ là quyết định thay người dùng rằng khách vẫn còn nằm trong
    /// phòng, và cái quyết định ấy đổi cả trạng thái phòng lẫn dòng tiền.
    BackfillCheckOutInTheFuture {
        requested: String,
        today: String,
    },
    /// Một ô tiền có mặt nhưng không đọc được thành số nguyên đồng.
    ///
    /// Trước biến thể này, `and_then(Value::as_i64)` trả `None` cho `"400000"`
    /// và `400000.0` y như khi ô ấy vắng mặt, rồi `unwrap_or(0)` biến `None`
    /// thành "chưa trả đồng nào": khách đưa 400.000₫ ở quầy, PMS ghi đã trả 0,
    /// và trên thẻ không có một chữ nào nói ra. Đó là ảnh soi gương của sự cố
    /// 06/08 — lần đó **tạo** một khoản thu không có thật, ca này **xoá** một
    /// khoản thu có thật.
    ///
    /// `requested` giữ nguyên văn JSON model gửi, để lời từ chối chỉ đúng vào
    /// hình dạng nó vừa gửi chứ không mô tả chung chung.
    UnreadableAmount {
        field: &'static str,
        requested: String,
    },
    /// Một ô tiền âm. `minimum: 0` trong JSON schema **không** phải hàng rào —
    /// không tầng nào kiểm lại nó, nên `paid_amount = -500000` dựng ra thẻ ghi
    /// "-500.000 ₫" rồi lệnh mới từ chối, **sau** khi lễ tân đã bấm *Đồng ý*.
    ///
    /// Cảnh báo "đã thu quá tiền phòng" không bắt được ca này: số âm luôn nhỏ
    /// hơn tổng.
    NegativeAmount {
        field: &'static str,
        requested: i64,
    },
    /// Một hoặc nhiều mục trong ô `guests` không cho ra được tên khách.
    ///
    /// `positions` đếm từ 1 theo đúng thứ tự model gửi, để lời từ chối chỉ được
    /// đúng mục hỏng. Trước biến thể này những mục ấy bị **bỏ trong im lặng**:
    /// ba khách vào, hai khách ra, model không nhận được lời nào và người thứ ba
    /// biến mất khỏi hồ sơ khai báo tạm trú.
    UnreadableGuestName {
        positions: Vec<usize>,
    },
    /// Khoảng ngày đặt phòng dài hơn trần `MAX_RESERVATION_NIGHTS`.
    ///
    /// `create_reservation` từ chối ("Number of nights must not exceed 90"),
    /// nhưng tầng thẻ không kiểm gì: một lỗi gõ năm (`2027` thay `2026`) dựng ra
    /// cái thẻ "122 đêm" kèm đủ tiền phòng cho 122 đêm, rồi lệnh mới từ chối —
    /// **sau** khi lễ tân đã bấm *Đồng ý*.
    TooManyNights {
        nights: i32,
        max: i64,
    },
}

/// Đổi số đêm thành ngày trả phòng, theo lịch địa phương.
pub fn check_out_date_from_nights(check_in: &str, nights: i32) -> CommandResult<String> {
    if nights < 1 {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Số đêm phải từ 1 trở lên.",
        ));
    }

    let start = NaiveDate::parse_from_str(check_in, "%Y-%m-%d").map_err(|_| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Ngày nhận phòng không hợp lệ.",
        )
    })?;

    let end = start
        .checked_add_days(chrono::Days::new(nights as u64))
        .ok_or_else(|| {
            CommandError::user(codes::VALIDATION_INVALID_INPUT, "Khoảng ngày quá xa.")
        })?;

    Ok(end.format("%Y-%m-%d").to_string())
}

/// `YYYY-MM-DD` → `DD/MM/YYYY`, cách người Việt đọc một ngày.
///
/// Không đọc được thì trả nguyên chuỗi vào. Thẻ thà hiện một chuỗi lạ còn hơn
/// hiện một ngày bịa ra hoặc nuốt luôn dòng ngày — nuốt im lặng chính là lớp
/// lỗi cả đợt này đang sửa.
fn format_vn_date(date: &str) -> String {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|value| value.format("%d/%m/%Y").to_string())
        .unwrap_or_else(|_| date.to_string())
}

/// `check_in_date` / `check_out_date` là **khoảng ngày đã dùng để hỏi giá**,
/// không phải hai trường mới của payload — xem ghi chú trong thân hàm.
pub fn build_check_in_display(
    payload: &CheckInRequest,
    preview: &Value,
    check_in_date: &str,
    check_out_date: &str,
) -> BTreeMap<String, String> {
    let mut display = BTreeMap::new();

    display.insert("room_id".to_string(), payload.room_id.clone());

    // Hai dòng ngày là thứ đã thiếu khi con bug 06/08 xảy ra: thẻ hiện `nights`
    // nhưng không hiện ngày nào, nên lễ tân đọc kỹ tới đâu cũng không có gì để
    // đối chiếu với câu mình vừa gõ ("checkin 8 out 9"). Một cái thẻ ghi sai
    // ngày trông y hệt một cái thẻ ghi đúng.
    //
    // Hai giá trị này **dẫn xuất**, không nằm trong payload — `CheckInRequest`
    // không có trường ngày nào và giữ nguyên đúng bảy trường như cũ.
    //
    // Vì sao điều đó KHÔNG vi phạm bất biến "display không bỏ sót trường
    // payload": bất biến ấy cấm **bỏ bớt**, không cấm **thêm**. Nó đọc theo
    // chiều payload → display (mỗi trường payload phải có mặt trên thẻ), nên
    // một khoá thừa trên thẻ không làm nó sai — xem
    // `the_card_shows_every_field_of_the_payload`: nó duyệt khoá của *payload*
    // rồi tìm trên thẻ, chứ không duyệt khoá của *display* rồi tìm trong
    // payload. Thêm ≠ bớt.
    //
    // Nhãn "Hôm nay" viết cứng được là nhờ nhánh từ chối trong
    // `build_check_in_draft`: thẻ nhận phòng chỉ dựng được khi ngày nhận đúng
    // là `now_local_date`. Ai gỡ nhánh ấy thì phải quay lại sửa dòng này —
    // `a_draft_for_tomorrow_is_refused_and_points_at_the_reservation_tool` là
    // chỗ báo động.
    display.insert(
        "check_in_date".to_string(),
        format!("Hôm nay, {}", format_vn_date(check_in_date)),
    );
    display.insert("check_out_date".to_string(), format_vn_date(check_out_date));

    // Khách: một dòng đếm đầu người, rồi **mỗi khách một dòng riêng mang mọi
    // trường đã điền** — không phải chỉ họ tên ghép bằng dấu phẩy.
    //
    // Số giấy tờ (CCCD) do model đọc ra là thứ `check_in` ghi thẳng vào
    // `guests.doc_number` rồi đi vào khai báo tạm trú. Thẻ giấu nó đi nghĩa là
    // con người bấm "Đồng ý" cho một con số họ chưa từng nhìn thấy — model gõ
    // sai một chữ số, hay bịa ra cả dãy, cũng không ai chặn được.
    display.insert(
        "guests".to_string(),
        format!("{} người", payload.guests.len()),
    );

    // `display` là `BTreeMap`, thẻ hiện theo thứ tự chuỗi của khoá: không đệm
    // 0 thì "Khách 10" đứng trước "Khách 2".
    let index_width = payload.guests.len().to_string().len();
    for (index, guest) in payload.guests.iter().enumerate() {
        display.insert(
            format!("Khách {:0width$}", index + 1, width = index_width),
            guest_display_line(guest),
        );
    }

    display.insert("nights".to_string(), format!("{} đêm", payload.nights));
    display.insert(
        "source".to_string(),
        payload.source.clone().unwrap_or_else(|| "—".to_string()),
    );
    display.insert(
        "notes".to_string(),
        payload.notes.clone().unwrap_or_else(|| "—".to_string()),
    );
    display.insert(
        "paid_amount".to_string(),
        payload
            .paid_amount
            .map(format_vnd)
            .unwrap_or_else(|| "0 ₫".to_string()),
    );
    display.insert(
        "pricing_type".to_string(),
        payload
            .pricing_type
            .clone()
            .unwrap_or_else(|| "nightly".to_string()),
    );
    // Trợ lý luôn gửi `None` ở trường này (xem `build_check_in_draft`), nên
    // "—" mới là giá trị thật sẽ hiện ra. Không dùng "0 ₫" như `paid_amount`:
    // `None` ở đây nghĩa là "không đè giá", không phải "đè giá bằng 0".
    display.insert(
        "rate_override_per_night".to_string(),
        payload
            .rate_override_per_night
            .map(format_vnd)
            .unwrap_or_else(|| "—".to_string()),
    );
    display.insert(
        "total".to_string(),
        preview
            .get("total")
            .and_then(Value::as_i64)
            .map(format_vnd)
            .unwrap_or_else(|| "—".to_string()),
    );

    display
}

/// Thẻ **đặt phòng trước**.
///
/// Khác thẻ nhận phòng ở một điểm đáng nói: `check_in_date`/`check_out_date` ở
/// đây **là trường payload thật** của `CreateReservationRequest`, không phải giá
/// trị dẫn xuất. Nên chúng vừa thoả bất biến "display không bỏ sót trường
/// payload", vừa thoả bất biến mới "mọi thẻ đều hiện ngày nhận và ngày trả".
///
/// Và **không** có nhãn "Hôm nay" ở dòng ngày nhận: thẻ này chỉ dựng được cho
/// một ngày ở tương lai (`build_reserve_draft` từ chối mọi ngày khác), nên viết
/// "Hôm nay" ở đây là viết một câu luôn sai.
pub fn build_reserve_display(
    payload: &CreateReservationRequest,
    preview: &Value,
) -> BTreeMap<String, String> {
    let mut display = BTreeMap::new();

    display.insert("room_id".to_string(), payload.room_id.clone());
    display.insert(
        "guest_name".to_string(),
        payload.guest_name.trim().to_string(),
    );
    display.insert(
        "guest_phone".to_string(),
        or_dash(payload.guest_phone.as_deref()),
    );
    // Số giấy tờ phải nằm ngay trên thẻ, cùng lý do như thẻ nhận phòng: nó đi
    // thẳng vào `guests.doc_number` rồi vào hồ sơ khai báo tạm trú, và model gõ
    // sai một chữ số thì không ai chặn được nếu thẻ giấu nó đi.
    display.insert(
        "guest_doc_number".to_string(),
        or_dash(payload.guest_doc_number.as_deref()),
    );
    display.insert(
        "check_in_date".to_string(),
        format_vn_date(&payload.check_in_date),
    );
    display.insert(
        "check_out_date".to_string(),
        format_vn_date(&payload.check_out_date),
    );
    display.insert("nights".to_string(), format!("{} đêm", payload.nights));
    display.insert(
        "deposit_amount".to_string(),
        payload
            .deposit_amount
            .map(format_vnd)
            .unwrap_or_else(|| "0 ₫".to_string()),
    );
    display.insert("source".to_string(), or_dash(payload.source.as_deref()));
    display.insert("notes".to_string(), or_dash(payload.notes.as_deref()));
    // `guests` là trường payload nên nó **phải** có một dòng trên thẻ, và dòng
    // ấy phải nói ra sự thật: `build_reserve_draft` luôn gửi `None`, tức lệnh
    // sẽ không thu phụ thu thêm người. Ghi "—" ở đây thì lễ tân đọc là "chưa
    // điền", còn dòng dưới nói rõ đó là một lựa chọn về tiền.
    //
    // Nhánh `Some` không phải mã chết vô nghĩa: nó là cái lưới cho ngày ai đó
    // đổi ý và cho model đưa số khách vào — thẻ sẽ hiện đúng con số đó thay vì
    // im lặng khoe một dòng nói ngược lại.
    display.insert(
        "guests".to_string(),
        match payload.guests {
            None => "Không ghi (không thu phụ thu thêm người)".to_string(),
            Some(count) => format!("{count} người"),
        },
    );
    // Trợ lý luôn gửi `None` ở trường này (xem `build_reserve_draft`), nên
    // "—" mới là giá trị thật sẽ hiện ra. Không dùng "0 ₫" như `deposit_amount`:
    // `None` ở đây nghĩa là "không đè giá", không phải "đè giá bằng 0". Cùng
    // luật `build_check_in_display`.
    display.insert(
        "rate_override_per_night".to_string(),
        payload
            .rate_override_per_night
            .map(format_vnd)
            .unwrap_or_else(|| "—".to_string()),
    );
    display.insert(
        "total".to_string(),
        preview
            .get("total")
            .and_then(Value::as_i64)
            .map(format_vnd)
            .unwrap_or_else(|| "—".to_string()),
    );

    display
}

/// Thẻ **ghi bù**.
///
/// Mọi trường của `BackfillStayRequest` đều có một dòng, kể cả hai trường có thể
/// là `None`. `check_out_date` đặc biệt: bất biến mới của cả đợt đòi **mọi** thẻ
/// hiện ngày nhận và ngày trả, mà `None` ở đây mang một nghĩa thật — khách còn
/// nằm trong phòng — chứ không phải "chưa điền". Nên dòng ấy nói ra nghĩa đó
/// bằng chữ thay vì một gạch ngang câm.
///
/// `total_price` là dòng nguy hiểm nhất trong cả ba thẻ: đây là lệnh ghi duy
/// nhất bắt người gọi đưa tiền vào, nên con số lễ tân đọc trên dòng này chính là
/// con số sẽ thành khoản nợ của khách. Nó tới từ preview — xem
/// `build_backfill_draft`.
pub fn build_backfill_display(payload: &BackfillStayRequest) -> BTreeMap<String, String> {
    let mut display = BTreeMap::new();

    display.insert("room_id".to_string(), payload.room_id.clone());
    display.insert(
        "check_in_date".to_string(),
        format_vn_date(&payload.check_in_date),
    );
    display.insert(
        "check_out_date".to_string(),
        match payload.check_out_date.as_deref() {
            Some(date) => format_vn_date(date),
            None => "Chưa trả phòng (khách còn ở)".to_string(),
        },
    );
    display.insert(
        "expected_checkout_date".to_string(),
        payload
            .expected_checkout_date
            .as_deref()
            .map(format_vn_date)
            .unwrap_or_else(|| "—".to_string()),
    );

    // Khách: một dòng đếm đầu người rồi mỗi khách một dòng mang mọi trường đã
    // điền — cùng khuôn thẻ nhận phòng, vì `backfill_stay` ghi vào đúng bảng
    // `guests` ấy rồi đi vào hồ sơ khai báo tạm trú.
    display.insert(
        "guests".to_string(),
        format!("{} người", payload.guests.len()),
    );
    let index_width = payload.guests.len().to_string().len();
    for (index, guest) in payload.guests.iter().enumerate() {
        display.insert(
            format!("Khách {:0width$}", index + 1, width = index_width),
            guest_display_line(guest),
        );
    }

    // KHÔNG có dòng `total` lấy từ preview bên cạnh dòng này: với thẻ ghi bù,
    // `total_price` **là** số của preview (`build_backfill_draft` chép thẳng
    // sang), nên hai dòng sẽ luôn in cùng một số. Một thẻ có hai dòng tiền giống
    // hệt nhau là một thẻ người ta ngừng đọc kỹ.
    display.insert("total_price".to_string(), format_vnd(payload.total_price));
    display.insert("paid_amount".to_string(), format_vnd(payload.paid_amount));
    display.insert("source".to_string(), or_dash(payload.source.as_deref()));
    display.insert("notes".to_string(), or_dash(payload.notes.as_deref()));

    display
}

/// Trường tuỳ chọn chưa điền hiện thành gạch ngang, cùng khuôn thẻ nhận phòng.
fn or_dash(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("—")
        .to_string()
}

/// Gói mọi trường **đã điền** của một khách thành một dòng đọc được.
///
/// Đây là điểm sửa duy nhất khi `CreateGuestRequest` mọc thêm trường: thêm
/// trường mà quên thêm nhãn ở đây thì `the_card_shows_every_field_of_the_payload`
/// đỏ. Trường rỗng hoặc `None` bị bỏ qua — không có gì để người duyệt nhìn, và
/// một dòng đầy nhãn trống chỉ làm số giấy tờ khó thấy hơn. Riêng `doc_number`
/// rỗng thì `build_warnings` nói hộ.
///
/// Nhãn bám theo form nhận phòng làm tay (`CheckinSheet.tsx`) để lễ tân đọc
/// thẻ và đọc form thấy cùng một thứ tiếng.
fn guest_display_line(guest: &CreateGuestRequest) -> String {
    let mut parts = vec![guest.full_name.trim().to_string()];

    let mut push = |label: &str, value: Option<&str>| {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            parts.push(format!("{label}: {value}"));
        }
    };

    // Thứ tự cố ý: hai thứ lễ tân đối chiếu với giấy tờ trên tay đứng trước.
    push("CCCD", Some(guest.doc_number.as_str()));
    push("SĐT", guest.phone.as_deref());
    push("Loại khách", guest.guest_type.as_deref());
    push("Ngày sinh", guest.dob.as_deref());
    push("Giới tính", guest.gender.as_deref());
    push("Quốc tịch", guest.nationality.as_deref());
    push("Địa chỉ", guest.address.as_deref());
    push("Visa hết hạn", guest.visa_expiry.as_deref());
    push("Ảnh giấy tờ", guest.scan_path.as_deref());

    parts.join(" · ")
}

fn format_vnd(amount: i64) -> String {
    let digits = amount.abs().to_string();
    let mut grouped = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    if amount < 0 {
        format!("-{grouped} ₫")
    } else {
        format!("{grouped} ₫")
    }
}

pub async fn build_check_in_draft(
    pool: &Pool<Sqlite>,
    args: &Value,
    now_local_date: &str,
) -> CommandResult<DraftOutcome> {
    let mut missing = Vec::new();

    let room_id = args
        .get("room_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if room_id.is_none() {
        missing.push("room_id".to_string());
    }

    let GuestList { guests, unreadable } = parse_guest_list(args);
    if !unreadable.is_empty() {
        // Trước cả `missing`: một mục khách không đọc được là một CON NGƯỜI sắp
        // rơi khỏi hồ sơ khai báo tạm trú, còn `missing` chỉ là một ô trống.
        return Ok(DraftOutcome::UnreadableGuestName {
            positions: unreadable,
        });
    }
    if guests.is_empty() {
        missing.push("guests".to_string());
    }

    let nights = args.get("nights").and_then(Value::as_i64).unwrap_or(0) as i32;
    if nights < 1 {
        missing.push("nights".to_string());
    }

    // ─── Ngày nhận phòng: soi ô ngày trước cả việc thiếu trường ───
    //
    // `check_in` đóng dấu `Local::now()` lúc bấm nút (`stay_lifecycle.rs`), nên
    // thẻ nhận phòng chỉ đúng cho HÔM NAY. Ngày 06/08/2026 lễ tân gõ "checkin 8
    // out 9 tháng 8": model không có ô nào để đặt số 8 vào nên bỏ luôn, thẻ ghi
    // ngày hôm nay, phòng 4B bị khoá khỏi tra phòng trống hai ngày và một đêm
    // chưa xảy ra bị thu 400.000₫. Giờ có ô, và đây là chỗ soi cái ô đó.
    //
    // Kiểm **trước** `missing` là cố ý: chọn nhầm tool là lỗi to hơn thiếu một
    // trường. Hỏi lại tên khách cho một cái thẻ sẽ không bao giờ dựng được vừa
    // tiêu mất một vòng trong ngân sách `MAX_TOOL_ROUNDS` (4), vừa xác nhận
    // ngược cho model rằng `draft_check_in` là đường đúng — nó chỉ cần điền nốt
    // là xong.
    //
    // Đọc qua `read_date`, **không** qua `as_str()`: một ô ngày không phải chuỗi
    // (model nghe "ngày 8" rồi gửi số `8`) đi qua `as_str()` thành `None`, và
    // `None` ở đây nghĩa là "không nêu ngày nào" — tức bỏ qua trọn khối này rồi
    // đóng dấu hôm nay lên thẻ. Xem [`ArgValue`].
    let requested_check_in = match read_date(args, "check_in_date") {
        Ok(value) => value,
        Err(requested) => return Ok(DraftOutcome::UnreadableCheckInDate { requested }),
    };

    if let Some((requested, requested_date)) = requested_check_in {
        // So theo NGÀY LỊCH, không so chuỗi và không so timestamp. "Hôm nay" là
        // một ngày, không phải một thời điểm; và `2026-8-6` với `2026-08-06` là
        // cùng một ngày, một dấu 0 thiếu không được biến thành lời từ chối.
        let today = NaiveDate::parse_from_str(now_local_date, "%Y-%m-%d").map_err(|_| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Ngày hôm nay `{now_local_date}` không đọc được."),
            )
        })?;

        if requested_date != today {
            return Ok(DraftOutcome::WrongDateForCheckIn {
                requested,
                is_future: requested_date > today,
            });
        }
    }

    // Ô tiền cũng soi **trước** `missing`, cùng một lý do như ô ngày: một khoản
    // tiền không đọc được mà rơi vào `missing_fields` thì model được bảo "thiếu
    // `paid_amount`" trong khi nó vừa gửi `"400000"` — lời bảo ấy mời nó gửi
    // lại đúng hình dạng cũ.
    let paid_amount = match read_amount(args, "paid_amount") {
        Ok(value) => value,
        Err(outcome) => return Ok(outcome),
    };

    if !missing.is_empty() {
        return Ok(DraftOutcome::MissingFields(missing));
    }

    let room_id = room_id.expect("đã kiểm ở trên").to_string();
    let check_out = check_out_date_from_nights(now_local_date, nights)?;
    let pricing_type = args
        .get("pricing_type")
        .and_then(Value::as_str)
        .unwrap_or("nightly")
        .to_string();

    // Số tiền trên thẻ đến từ preview, không từ model. Preview hỏng thì không
    // có thẻ — không có số mặc định nào.
    //
    // `None` cho số khách, **không** phải `guests.len()`: nút "Đồng ý" gọi
    // `check_in`, và `stay_lifecycle::check_in` truyền `None` xuống engine
    // (`stay_lifecycle.rs`, chỗ gọi `calculate_stay_price_tx`), tức quầy không
    // thu phụ thu thêm người. Gửi số khách ở đây thì thẻ báo cao hơn số thực
    // thu — lễ tân đọc con số đó cho khách nghe. Cùng luật với form làm tay
    // (`CheckinSheet.tsx` truyền `guests: null`) và với ghi chú ở
    // `hooks/usePricePreview.ts`.
    let preview_result = pricing_service::calculate_room_price_preview(
        pool,
        &room_id,
        now_local_date,
        &check_out,
        &pricing_type,
        None,
    )
    .await
    .map_err(|error| {
        CommandError::user(
            codes::AGENT_PREVIEW_UNAVAILABLE,
            format!("Không tra được giá phòng nên chưa dựng được thẻ: {error}"),
        )
    })?;

    let preview = serde_json::to_value(&preview_result).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không mã hoá được báo giá: {error}"),
        )
    })?;

    let payload = CheckInRequest {
        room_id: room_id.clone(),
        guests,
        nights,
        source: args
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some("walk-in".to_string())),
        notes: args
            .get("notes")
            .and_then(Value::as_str)
            .map(str::to_string),
        paid_amount,
        pricing_type: Some(pricing_type),
        // Trợ lý không tự đặt giá tay: đó là việc lễ tân làm ở quầy khi mặc cả
        // với khách, không phải việc một mô hình ngôn ngữ quyết định thay. Thẻ
        // này vẫn đi qua đường engine như cũ.
        rate_override_per_night: None,
    };

    let warnings =
        build_warnings(pool, &room_id, &payload.guests, now_local_date, &check_out).await?;
    // Đúng khoảng ngày vừa dùng để hỏi giá — tiền trên thẻ và ngày trên thẻ
    // phải nói về cùng một kỳ ở, không phải hai nguồn sự thật tính riêng.
    let display = build_check_in_display(&payload, &preview, now_local_date, &check_out);

    Ok(DraftOutcome::Ready(Box::new(ProposedAction {
        kind: CHECK_IN_ACTION_KIND.to_string(),
        payload: ActionPayload::CheckIn(payload),
        display,
        preview,
        warnings,
        built_at_ms: chrono::Utc::now().timestamp_millis(),
    })))
}

/// Cảnh báo tra từ PMS, không phải lời model viết ra.
///
/// `check_in_date`/`check_out_date` là **đúng khoảng ngày vừa dùng để hỏi giá**
/// và đúng khoảng ngày in trên thẻ — xem khối dò trùng lịch trong thân hàm.
async fn build_warnings(
    pool: &Pool<Sqlite>,
    room_id: &str,
    guests: &[CreateGuestRequest],
    check_in_date: &str,
    check_out_date: &str,
) -> CommandResult<Vec<String>> {
    let rooms = load_room_status_now(pool).await.map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không đọc được trạng thái phòng: {error}"),
        )
    })?;

    // Hai câu dưới đây nói giọng **từ chối**, không phải giọng "lưu ý":
    // `check_in_tx` bắt `room_status != status::room::VACANT` là `Conflict` và
    // trả về ngay ("Phòng {} không trống (status: {})"), nên **mọi** trạng thái
    // khác `vacant` đều không nhận phòng được. Cảnh báo của
    // `draft_reserve`/`draft_backfill` viết thẳng "lệnh sẽ từ chối"; hai câu này
    // từng không nói ra, nên lễ tân đọc chúng như lời khuyên có thể bỏ qua rồi
    // bấm *Đồng ý* để nhận một lỗi.
    //
    // Câu trạng thái so với `status::room::VACANT` — đúng phép so `check_in_tx`
    // làm, đúng hình dạng `draft_backfill` đã dùng — chứ không bắt một chuỗi cụ
    // thể. Nó từng bắt `"dirty"`, một giá trị **chưa từng** là thành viên của
    // `status::room` và không đường ghi nào sinh ra, nên nhánh không bao giờ
    // chạy: phòng còn kẹt ở `booked` hay `occupied` ra thẻ Sẵn sàng không một
    // câu cảnh báo trạng thái nào, rồi bị lệnh chặn ở quầy.
    //
    // Phòng đang có khách đi trước và **thay** câu trạng thái chung: nó cũng là
    // một phòng khác `vacant`, nhưng câu riêng nói thêm được việc phải làm. Hai
    // câu cùng kể một lần từ chối là nhiễu trên cái thẻ lễ tân đọc.
    let mut warnings = Vec::new();
    if let Some(room) = rooms.iter().find(|room| room.room_id == room_id) {
        if room.booking_id.is_some() {
            warnings.push(
                "Phòng đang có khách ở. `check_in` sẽ TỪ CHỐI: nó chỉ nhận phòng đang trống. \
                 Phải trả phòng cho khách hiện tại trước."
                    .to_string(),
            );
        } else if !room.status.eq_ignore_ascii_case(status::room::VACANT) {
            warnings.push(format!(
                "Phòng đang ở trạng thái «{}», không phải trống. `check_in` sẽ TỪ CHỐI: nó chỉ \
                 nhận phòng đang trống.",
                room.status
            ));
        }
    }

    // ─── Dò trùng lịch trên CẢ KỲ Ở, không chỉ "ngay bây giờ" ───
    //
    // Hai câu trên đọc `load_room_status_now` — trạng thái *lúc này*. Một phòng
    // trống hôm nay mà có người đặt từ ngày kia thì không câu nào ở trên bắt
    // được, trong khi `check_in_tx` quét `room_calendar` trên trọn khoảng
    // `[nhận, trả)` và trả `Conflict` ("Room has a reservation starting …").
    // Đo được: phòng trống, reservation từ 08/08, `nights = 4` ⇒ thẻ ghi ngày
    // trả 10/08 với `warnings []` rồi lệnh từ chối — **một ngày sai in trên
    // thẻ**, đúng lớp lỗi cả nhánh này sinh ra để diệt.
    //
    // Dùng đúng truy vấn và đúng biên `[from, to)` mà `draft_reserve` và
    // `draft_backfill` đã dùng, nên ba cái thẻ nói về phòng trống bằng cùng một
    // nguồn sự thật.
    let free_rooms = load_free_rooms_between(pool, check_in_date, check_out_date)
        .await
        .map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Không đọc được lịch phòng trống: {error}"),
            )
        })?;
    if !free_rooms.iter().any(|room| room.room_id == room_id) {
        warnings.push(format!(
            "Phòng đã có lượt ở hoặc lượt đặt khác trong khoảng {} – {}. `check_in` sẽ TỪ CHỐI \
             vì trùng lịch, dù ngay lúc này phòng có vẻ trống.",
            format_vn_date(check_in_date),
            format_vn_date(check_out_date),
        ));
    }

    // Trợ lý dựng được một khách mà chính form của PMS sẽ từ chối: ở trên,
    // `doc_number` vắng mặt được mặc định thành `""`, và
    // `stay_lifecycle::validate_check_in_request` không kiểm trường đó — trong
    // khi `CheckinSheet.tsx` khoá nút lưu khi thiếu. Cố ý **chỉ cảnh báo**,
    // không từ chối: con người là bước duyệt theo đúng thiết kế, còn siết
    // thành lỗi cứng là quyết định sản phẩm của chủ nhà, không phải của chỗ
    // này.
    for guest in guests {
        if guest.doc_number.trim().is_empty() {
            warnings.push(format!(
                "Khách «{}» chưa có số giấy tờ. Form nhận phòng làm tay không cho lưu như vậy \
                 (chế độ nhanh phải có số điện thoại thay thế), và hồ sơ khai báo tạm trú sẽ thiếu.",
                guest.full_name.trim()
            ));
        }
    }

    Ok(warnings)
}

/// Đọc ô `guests` của một tool dựng thẻ thành danh sách khách của PMS.
///
/// Dùng chung cho `draft_check_in` và `draft_backfill` — hai tool nhận **cùng
/// một** hình dạng danh sách khách và ghi vào cùng một bảng `guests`. Hai bản
/// chép tay là hai chỗ trôi độc lập: bản này bỏ khách có `full_name` rỗng, và
/// một bản chép nhầm sẽ để lọt một hàng khách không tên vào hồ sơ khai báo tạm
/// trú mà không test nào của tool kia nhìn thấy.
///
/// Bảy trường còn lại của `CreateGuestRequest` để `None` vì schema tool chỉ mở
/// ba ô (`full_name`, `doc_number`, `phone`): thêm ô là thêm chỗ cho model bịa,
/// và những trường ấy lễ tân điền ở form làm tay khi có giấy tờ trên tay.
///
/// ─── KHÔNG BỎ AI TRONG IM LẶNG ───
///
/// Hàm này từng `filter_map` thẳng: một mục có `full_name` không phải chuỗi (hay
/// rỗng) bị **loại khỏi danh sách mà không ai được báo** — ba khách vào, hai
/// khách ra, model không nhận được lời nào, và người thứ ba biến mất khỏi hồ sơ
/// khai báo tạm trú. Trên thẻ cũng không có gì để đối chiếu: nó chỉ ghi "2
/// người". Giờ những mục ấy đi ra ngoài theo `unreadable` và người gọi phải xử
/// lý.
fn parse_guest_list(args: &Value) -> GuestList {
    let Some(entries) = args.get("guests").and_then(Value::as_array) else {
        return GuestList::default();
    };

    let mut list = GuestList::default();
    for (index, entry) in entries.iter().enumerate() {
        let full_name = entry
            .get("full_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let Some(full_name) = full_name else {
            // Đếm từ 1: con số này đi thẳng vào lời từ chối cho model, và "mục
            // khách thứ 0" là câu không ai đọc được.
            list.unreadable.push(index + 1);
            continue;
        };

        list.guests.push(CreateGuestRequest {
            guest_type: None,
            full_name: full_name.to_string(),
            doc_number: entry
                .get("doc_number")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            dob: None,
            gender: None,
            nationality: None,
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: entry
                .get("phone")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    list
}

/// Ô `guests` đã đọc: người dựng được, và **vị trí** những mục không dựng được.
#[derive(Default)]
struct GuestList {
    guests: Vec<CreateGuestRequest>,
    /// Vị trí (đếm từ 1) của những mục không cho ra được tên khách.
    unreadable: Vec<usize>,
}

/// Đọc một tham số chuỗi đã cắt khoảng trắng; rỗng coi như vắng mặt.
///
/// CHỈ dùng cho ô CHỮ (`room_id`, `guest_name`, `notes`…). Ô **ngày** dùng
/// [`date_arg`] và ô **tiền** dùng [`amount_arg`]: ở hai loại ấy, gộp "vắng mặt"
/// với "có mà không đọc được" là cách con bug 06/08 sống lại — xem ghi chú của
/// hai hàm đó.
fn trimmed_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Một ô của model, đã tách "vắng mặt" khỏi "có mà không đọc được".
///
/// ─── VÌ SAO PHẢI CÓ KIỂU NÀY ───
///
/// `Value::as_str()` trả `None` cho **mọi thứ không phải chuỗi JSON**, và ở chỗ
/// đọc `check_in_date` thì `None` nghĩa là *"người dùng không nêu ngày nào"* —
/// tức bỏ qua trọn vẹn khối soi ngày rồi đóng dấu hôm nay lên thẻ. Model nghe
/// "ngày 8" và điền `"check_in_date": 8` (số JSON, hình dạng model thật vẫn gửi)
/// là đủ để dựng lại đúng sự cố 06/08, lần này qua cửa kiểu dữ liệu thay vì cửa
/// ngày — và model không nhận được lời từ chối nào nên vòng tự sửa cũng không
/// khởi động.
///
/// `null` cố ý xếp vào `Absent`, không phải `Unreadable`: model nào cũng điền
/// `null` cho một ô tuỳ chọn nó bỏ trống, và xử `null` như rác thì mọi lượt
/// nhận phòng bình thường tốn thêm một vòng trong ngân sách bốn vòng.
enum ArgValue {
    /// Không có khoá, hoặc có mà là `null`.
    Absent,
    /// Đọc được, đã cắt khoảng trắng.
    Text(String),
    /// Có mặt nhưng không dùng được: sai kiểu, hoặc chuỗi rỗng/toàn khoảng
    /// trắng. Mang theo **nguyên văn JSON** model gửi để lời từ chối chỉ đúng
    /// vào hình dạng nó vừa gửi.
    Unreadable(String),
}

/// Đọc một ô NGÀY. Xem [`ArgValue`] cho lý do không dùng thẳng `as_str()`.
fn date_arg(args: &Value, key: &str) -> ArgValue {
    match args.get(key) {
        None | Some(Value::Null) => ArgValue::Absent,
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                // Chuỗi toàn khoảng trắng là "có gửi mà không nói gì", không
                // phải "không gửi": đo được nó cũng ra thẻ mang ngày hôm nay.
                ArgValue::Unreadable(text.clone())
            } else {
                ArgValue::Text(trimmed.to_string())
            }
        }
        Some(other) => ArgValue::Unreadable(other.to_string()),
    }
}

/// Đọc một ô ngày rồi quy ra ngày lịch.
///
/// `Ok(Some((nguyên_văn, ngày)))` — đọc được; nguyên văn đi kèm vì mọi lời từ
/// chối đều nhắc lại **đúng chuỗi người dùng nêu**, không phải một ngày đã bị
/// chuẩn hoá lại.
/// `Ok(None)` — vắng mặt (hoặc `null`).
/// `Err(nguyên_văn)` — có mặt mà không đọc được; người gọi bọc nó vào biến thể
/// `Unreadable*Date` gọi đúng tên ô của tool mình.
fn read_date(args: &Value, field: &str) -> Result<Option<(String, NaiveDate)>, String> {
    match date_arg(args, field) {
        ArgValue::Absent => Ok(None),
        ArgValue::Unreadable(requested) => Err(requested),
        ArgValue::Text(requested) => match NaiveDate::parse_from_str(&requested, "%Y-%m-%d") {
            Ok(parsed) => Ok(Some((requested, parsed))),
            // Không đoán, và tuyệt đối không rơi về hôm nay. "ngày 8" có thể là
            // mùng 8 tháng này, tháng sau hay năm sau — rơi về hôm nay đúng là
            // con bug 06/08.
            Err(_) => Err(requested),
        },
    }
}

/// Một ô TIỀN của model, đã soi kiểu.
enum AmountArg {
    Absent,
    Amount(i64),
    Unreadable(String),
}

/// Đọc một ô TIỀN thành số nguyên đồng.
///
/// ─── ẢNH SOI GƯƠNG CỦA SỰ CỐ 06/08 ───
///
/// `and_then(Value::as_i64)` trả `None` cho `400000.0` và cho `"400000"` y hệt
/// khi ô ấy vắng mặt, rồi `.unwrap_or(0)` biến `None` thành "chưa trả đồng nào".
/// Khách đưa 400.000₫ ở quầy, PMS ghi **đã trả 0**, khách gánh nguyên khoản nợ —
/// và trên thẻ không có một chữ nào nói ra. Sự cố 06/08 *tạo* một khoản thu
/// không có thật; ca này *xoá* một khoản thu có thật. Cùng một lớp lỗi: bỏ im
/// lặng.
///
/// **Số nguyên gửi dạng số thực (`400000.0`) được nhận**, đúng giá trị: model
/// hay gửi tiền như vậy, và từ chối cả ca ấy là bắt lễ tân gõ tay một khoản đã
/// đúng. Đó là quyết định có chủ ý và có test riêng
/// (`a_whole_number_sent_as_a_float_reaches_the_payload_at_its_exact_value`).
/// Phần thập phân khác 0 thì KHÔNG nhận: tiền Việt không có đơn vị nhỏ hơn đồng,
/// nên `400000.5` là một con số hỏng chứ không phải một con số cần làm tròn.
///
/// Chuỗi cũng KHÔNG nhận, kể cả `"400000"`: `"400.000"` là bốn trăm nghìn theo
/// cách viết Việt và bốn trăm phẩy không theo cách đọc số của máy. Đoán giữa hai
/// nghĩa ấy là đoán hộ một khoản tiền.
fn amount_arg(args: &Value, key: &str) -> AmountArg {
    match args.get(key) {
        None | Some(Value::Null) => AmountArg::Absent,
        Some(Value::Number(number)) => {
            if let Some(value) = number.as_i64() {
                return AmountArg::Amount(value);
            }
            match number.as_f64() {
                // `as_i64` đã loại hết số nguyên vừa `i64`, nên tới đây chỉ còn
                // số thực và số nguyên tràn `i64`. Cả hai đều phải qua cùng một
                // cửa: chỉ nhận khi không có phần lẻ VÀ vừa `i64`.
                Some(value)
                    if value.fract() == 0.0
                        && value >= i64::MIN as f64
                        && value <= i64::MAX as f64 =>
                {
                    AmountArg::Amount(value as i64)
                }
                _ => AmountArg::Unreadable(number.to_string()),
            }
        }
        Some(other) => AmountArg::Unreadable(other.to_string()),
    }
}

/// Soi một ô tiền và trả về `DraftOutcome` từ chối nếu nó hỏng.
///
/// Gộp cả hai luật vào một chỗ để ba tool không trôi khỏi nhau: không đọc được
/// ⇒ [`DraftOutcome::UnreadableAmount`]; âm ⇒ [`DraftOutcome::NegativeAmount`].
///
/// Số âm cần một hàng rào riêng vì `minimum: 0` trong JSON schema **không** phải
/// hàng rào — không tầng nào kiểm lại nó. Đo được `paid_amount = -500000` dựng
/// ra thẻ ghi "-500.000 ₫" với `warnings []`, rồi lệnh mới từ chối, **sau** khi
/// lễ tân đã bấm *Đồng ý*. Cảnh báo "đã thu quá tiền phòng" cũng không bắt được
/// ca ấy: số âm luôn nhỏ hơn tổng.
fn read_amount(args: &Value, field: &'static str) -> Result<Option<i64>, DraftOutcome> {
    match amount_arg(args, field) {
        AmountArg::Absent => Ok(None),
        AmountArg::Unreadable(requested) => {
            Err(DraftOutcome::UnreadableAmount { field, requested })
        }
        AmountArg::Amount(value) if value < 0 => Err(DraftOutcome::NegativeAmount {
            field,
            requested: value,
        }),
        AmountArg::Amount(value) => Ok(Some(value)),
    }
}

/// Thẻ **đặt phòng trước** → lệnh `create_reservation`.
///
/// Đây là đích mà lời từ chối của `build_check_in_draft` chỉ tới. Không có nó,
/// trợ lý chỉ biết nói "không làm được": con bug cũ hết xảy ra nhưng lễ tân cũng
/// không đặt được phòng, và một trợ lý luôn từ chối thì người ta ngừng dùng —
/// rồi quay về gõ tay, đúng chỗ con bug 06/08 đã sinh ra.
pub async fn build_reserve_draft(
    pool: &Pool<Sqlite>,
    args: &Value,
    now_local_date: &str,
) -> CommandResult<DraftOutcome> {
    let today = NaiveDate::parse_from_str(now_local_date, "%Y-%m-%d").map_err(|_| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Ngày hôm nay `{now_local_date}` không đọc được."),
        )
    })?;

    let room_id = trimmed_arg(args, "room_id");
    let guest_name = trimmed_arg(args, "guest_name");

    // ─── Soi hai ô ngày TRƯỚC khi kể trường thiếu ───
    //
    // Cùng lý do như `build_check_in_draft`: chọn nhầm tool là lỗi to hơn thiếu
    // một trường. Hỏi lại tên khách cho một cái thẻ sẽ không bao giờ dựng được
    // vừa tiêu một vòng trong ngân sách `MAX_TOOL_ROUNDS`, vừa xác nhận ngược
    // cho model rằng `draft_reserve` là đường đúng — nó chỉ cần điền nốt là
    // xong.
    //
    // `read_date`, không `trimmed_arg`: `trimmed_arg` đứng trên `as_str()` nên
    // một ô ngày không phải chuỗi (`8`) tụt xuống `MissingFields` — model **có**
    // thấy, nhưng nó bị bảo "thiếu ngày" trong khi vừa gửi một ngày, và lời bảo
    // ấy mời nó gửi lại đúng hình dạng cũ. Xem [`ArgValue`].
    let check_in = match read_date(args, "check_in_date") {
        Err(requested) => {
            return Ok(DraftOutcome::UnreadableReserveDate {
                field: "check_in_date",
                requested,
            })
        }
        Ok(None) => None,
        Ok(Some((requested, parsed))) => {
            // So theo NGÀY LỊCH. Hôm nay là hôm nay bất kể người dùng gọi nó là
            // "đặt" hay "nhận": `create_reservation` ghi `status='booked'` và
            // giữ chỗ, còn khách đang đứng ở quầy cần một lượt ở đang mở.
            if parsed <= today {
                return Ok(DraftOutcome::WrongDateForReserve {
                    requested,
                    is_today: parsed == today,
                });
            }
            Some(parsed)
        }
    };

    let check_out = match read_date(args, "check_out_date") {
        Err(requested) => {
            return Ok(DraftOutcome::UnreadableReserveDate {
                field: "check_out_date",
                requested,
            })
        }
        Ok(value) => value.map(|(_, parsed)| parsed),
    };

    if let (Some(check_in), Some(check_out)) = (check_in, check_out) {
        if check_out <= check_in {
            // Không tự cộng một đêm cho đủ. Số đêm là thứ khách trả tiền, và
            // đoán hộ một đêm là đoán hộ một khoản tiền.
            return Ok(DraftOutcome::CheckOutNotAfterCheckIn {
                check_in_date: check_in.format("%Y-%m-%d").to_string(),
                check_out_date: check_out.format("%Y-%m-%d").to_string(),
            });
        }
    }

    // Soi ô tiền trước `missing`, cùng lý do như ô ngày — xem
    // `build_check_in_draft`.
    let deposit_amount = match read_amount(args, "deposit_amount") {
        Ok(value) => value,
        Err(outcome) => return Ok(outcome),
    };

    let mut missing = Vec::new();
    if room_id.is_none() {
        missing.push("room_id".to_string());
    }
    if guest_name.is_none() {
        missing.push("guest_name".to_string());
    }
    if check_in.is_none() {
        missing.push("check_in_date".to_string());
    }
    if check_out.is_none() {
        missing.push("check_out_date".to_string());
    }
    if !missing.is_empty() {
        return Ok(DraftOutcome::MissingFields(missing));
    }

    let room_id = room_id.expect("đã kiểm ở trên").to_string();
    let guest_name = guest_name.expect("đã kiểm ở trên").to_string();
    let check_in = check_in.expect("đã kiểm ở trên");
    let check_out = check_out.expect("đã kiểm ở trên");

    // Chuẩn hoá về `YYYY-MM-DD` có đệm 0, **không** dùng lại nguyên văn chuỗi
    // model gửi. `create_reservation_tx` dò trùng lịch bằng so sánh CHUỖI
    // (`room_calendar.date >= ?`), và `'2026-08-08' < '2026-8-8'` theo thứ tự
    // chuỗi — một dấu 0 thiếu làm phép dò trùng ấy quét nhầm khoảng. Đây không
    // phải "sửa ngày người dùng nêu": cùng một ngày lịch, chỉ khác cách viết.
    let check_in_date = check_in.format("%Y-%m-%d").to_string();
    let check_out_date = check_out.format("%Y-%m-%d").to_string();

    // Số đêm **dẫn xuất**, không nhận từ model — schema cũng không có ô cho nó.
    // Vắt tháng hay vắt năm đều đúng vì đây là hiệu hai ngày lịch, không phải
    // phép trừ trên số ngày trong tháng.
    let nights = i32::try_from((check_out - check_in).num_days()).map_err(|_| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Khoảng ngày đặt phòng quá dài.",
        )
    })?;

    // Trần đọc từ **chính hằng số của lệnh**, không chép tay con số 90: chép tay
    // là dựng ra hai chính sách trôi độc lập, rồi một hôm cái thẻ cho qua đúng
    // thứ lệnh vừa siết. Không có nhánh này thì một lỗi gõ năm (`2027` thay
    // `2026`) dựng ra cái thẻ "122 đêm" kèm tiền phòng của 122 đêm, và lời từ
    // chối chỉ tới sau khi lễ tân đã bấm *Đồng ý*.
    if i64::from(nights) > MAX_RESERVATION_NIGHTS {
        return Ok(DraftOutcome::TooManyNights {
            nights,
            max: MAX_RESERVATION_NIGHTS,
        });
    }

    // Số tiền trên thẻ đến từ preview, KHÔNG từ model — schema không có ô tiền
    // phòng, và preview hỏng thì không có thẻ, không có số mặc định.
    //
    // Ba tham số phải khớp đúng cái `create_reservation_tx` sẽ dùng, không thì
    // thẻ báo một số và sổ sách ghi một số khác:
    //   • cùng khoảng ngày;
    //   • `"nightly"` — lệnh viết cứng kiểu này, thẻ không được cho model chọn;
    //   • `None` cho số khách — quầy không thu phụ thu thêm người. Gửi số khách
    //     vào đây là thẻ báo cao hơn số thực thu, và lễ tân đọc con số đó cho
    //     khách nghe. Cùng luật `check_in`/`CheckinSheet.tsx`.
    let preview_result = pricing_service::calculate_room_price_preview(
        pool,
        &room_id,
        &check_in_date,
        &check_out_date,
        "nightly",
        None,
    )
    .await
    .map_err(|error| {
        CommandError::user(
            codes::AGENT_PREVIEW_UNAVAILABLE,
            format!("Không tra được giá phòng nên chưa dựng được thẻ: {error}"),
        )
    })?;

    let preview = serde_json::to_value(&preview_result).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không mã hoá được báo giá: {error}"),
        )
    })?;

    let payload = CreateReservationRequest {
        room_id: room_id.clone(),
        guest_name,
        guest_phone: trimmed_arg(args, "guest_phone").map(str::to_string),
        guest_doc_number: trimmed_arg(args, "guest_doc_number").map(str::to_string),
        check_in_date: check_in_date.clone(),
        check_out_date: check_out_date.clone(),
        nights,
        deposit_amount,
        // Viết cứng, không cho model điền: `create_reservation_tx` mặc định
        // `"phone"` khi vắng, nên gửi `None` thì thẻ hiện "—" trong khi sổ ghi
        // "phone" — thẻ nói một đằng, bản ghi một nẻo. Nêu thẳng ra để hai bên
        // khớp nhau, cùng cách `build_check_in_draft` viết cứng `"walk-in"`.
        source: Some("phone".to_string()),
        notes: trimmed_arg(args, "notes").map(str::to_string),
        // LUÔN `None`. Xem ghi chú ở lời gọi preview ngay trên.
        guests: None,
        // Trợ lý không tự đặt giá tay: đó là việc lễ tân làm ở quầy khi mặc
        // cả với khách qua điện thoại, không phải việc một mô hình ngôn ngữ
        // quyết định thay. Thẻ này vẫn đi qua đường engine như cũ. Cùng luật
        // `build_check_in_draft`.
        rate_override_per_night: None,
    };

    let warnings = build_reserve_warnings(pool, &payload).await?;
    let display = build_reserve_display(&payload, &preview);

    Ok(DraftOutcome::Ready(Box::new(ProposedAction {
        kind: RESERVE_ACTION_KIND.to_string(),
        payload: ActionPayload::Reserve(payload),
        display,
        preview,
        warnings,
        built_at_ms: chrono::Utc::now().timestamp_millis(),
    })))
}

/// Cảnh báo cho thẻ đặt phòng trước — tra từ PMS, không phải lời model viết ra.
async fn build_reserve_warnings(
    pool: &Pool<Sqlite>,
    payload: &CreateReservationRequest,
) -> CommandResult<Vec<String>> {
    let mut warnings = Vec::new();

    // Phòng bận **trong khoảng ngày đặt**, không phải "phòng đang có khách ở"
    // của thẻ nhận phòng. Hai câu hỏi khác hẳn nhau: một phòng có khách hôm nay
    // mà trống ngày 8 thì đặt ngày 8 hoàn toàn hợp lệ, và một cảnh báo bật lên
    // ở ca hợp lệ ấy dạy lễ tân bỏ qua cảnh báo — tệ hơn hẳn không có cảnh báo.
    //
    // Dùng đúng truy vấn của tool đọc `check_room_availability`, nên câu trên
    // thẻ và câu trợ lý trả lời khi được hỏi "phòng đó còn trống không" đến từ
    // một nguồn.
    let free_rooms = load_free_rooms_between(pool, &payload.check_in_date, &payload.check_out_date)
        .await
        .map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Không đọc được lịch phòng trống: {error}"),
            )
        })?;
    if !free_rooms
        .iter()
        .any(|room| room.room_id == payload.room_id)
    {
        warnings.push(format!(
            "Phòng đã có khách ở hoặc đã có người đặt trong khoảng {} – {}. \
             `create_reservation` sẽ từ chối nếu trùng lịch.",
            format_vn_date(&payload.check_in_date),
            format_vn_date(&payload.check_out_date),
        ));
    }

    // Giữ nguyên cảnh báo thiếu giấy tờ của thẻ nhận phòng: cùng một hồ sơ khai
    // báo tạm trú, cùng một chỗ thiếu.
    if payload
        .guest_doc_number
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        warnings.push(format!(
            "Khách «{}» chưa có số giấy tờ. Hồ sơ khai báo tạm trú sẽ thiếu khi khách tới nhận phòng.",
            payload.guest_name.trim()
        ));
    }

    // Nhiều khách thì **nói ra**, không im lặng bỏ bớt — bỏ im lặng đúng là lớp
    // lỗi cả đợt này đang sửa.
    //
    // GIỚI HẠN, ghi ra để không ai tưởng đây là hàng rào: tool chỉ nhìn thấy ô
    // `guest_name`. Model nghe "anh Nam với chị Hoa" rồi tự bỏ chị Hoa và gửi
    // đúng một tên thì ở đây KHÔNG có gì để bắt — chuyện ấy chỉ chặn được bằng
    // mô tả tool và system prompt, mà prompt không phải hàng rào. Câu dưới bắt
    // ca còn lại, ca model dồn cả hai tên vào một ô.
    if looks_like_more_than_one_name(&payload.guest_name) {
        warnings.push(format!(
            "Đặt phòng trước chỉ ghi được MỘT tên khách, và ô tên đang là «{}». \
             Nếu đó là nhiều người thì chỉ tên này vào PMS — những người còn lại \
             phải ghi tay ở màn hình Đặt phòng.",
            payload.guest_name.trim()
        ));
    }

    Ok(warnings)
}

/// Thẻ **ghi bù** → lệnh `backfill_stay`.
///
/// Cạnh thứ ba của luật định tuyến: ngày nhận ở quá khứ. Không có nó thì lời từ
/// chối của `build_check_in_draft` cho một ngày đã qua chỉ vào chỗ trống.
///
/// ─── CHỖ NGUY HIỂM NHẤT CỦA CẢ ĐỢT ───
///
/// `backfill_stay` là lệnh ghi **duy nhất** bắt người gọi đưa tiền phòng vào:
/// `check_in` và `create_reservation` tự tính lấy, còn `BackfillStayRequest`
/// có một ô `total_price` bắt buộc. Kiểu dữ liệu mời người viết code lấy con số
/// ấy từ tham số model gửi — và làm thế là để một mô hình ngôn ngữ quyết định
/// khách nợ bao nhiêu tiền.
///
/// Luật: `total_price` **luôn** lấy từ `calculate_room_price_preview`, kể cả khi
/// model gửi kèm một con số. Schema `draft_backfill` cũng không có ô tiền phòng,
/// nên đây là hai lớp cùng canh một chỗ. Preview hỏng ⇒ **không có thẻ**, không
/// có số mặc định, không đoán.
pub async fn build_backfill_draft(
    pool: &Pool<Sqlite>,
    args: &Value,
    now_local_date: &str,
) -> CommandResult<DraftOutcome> {
    let today = NaiveDate::parse_from_str(now_local_date, "%Y-%m-%d").map_err(|_| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Ngày hôm nay `{now_local_date}` không đọc được."),
        )
    })?;

    let room_id = trimmed_arg(args, "room_id");
    let GuestList { guests, unreadable } = parse_guest_list(args);
    if !unreadable.is_empty() {
        // Cùng luật với `build_check_in_draft`: một mục khách không đọc được là
        // một CON NGƯỜI sắp rơi khỏi hồ sơ khai báo tạm trú, không phải một ô
        // trống.
        return Ok(DraftOutcome::UnreadableGuestName {
            positions: unreadable,
        });
    }

    // ─── Soi ba ô ngày TRƯỚC khi kể trường thiếu ───
    //
    // Cùng lý do như hai tool kia: chọn nhầm tool là lỗi to hơn thiếu một
    // trường, và hỏi lại tên khách cho một cái thẻ không bao giờ dựng được vừa
    // tiêu một vòng trong ngân sách `MAX_TOOL_ROUNDS` vừa xác nhận ngược cho
    // model rằng `draft_backfill` là đường đúng.
    // `read_date` cho cả ba ô, không `trimmed_arg` — xem `build_reserve_draft`.
    let check_in = match read_date(args, "check_in_date") {
        Err(requested) => {
            return Ok(DraftOutcome::UnreadableBackfillDate {
                field: "check_in_date",
                requested,
            })
        }
        Ok(None) => None,
        Ok(Some((requested, parsed))) => {
            // So theo NGÀY LỊCH. `backfill_stay` kiểm lại đúng bất đẳng thức này
            // (`check_in >= today` ⇒ lỗi): ghi bù cho hôm nay là nhận phòng, và
            // ghi bù cho ngày mai là ghi lại một kỳ ở chưa xảy ra.
            if parsed >= today {
                return Ok(DraftOutcome::WrongDateForBackfill {
                    requested,
                    is_today: parsed == today,
                });
            }
            Some(parsed)
        }
    };

    let check_out = match read_date(args, "check_out_date") {
        Err(requested) => {
            return Ok(DraftOutcome::UnreadableBackfillDate {
                field: "check_out_date",
                requested,
            })
        }
        Ok(value) => value.map(|(_, parsed)| parsed),
    };

    let expected_checkout = match read_date(args, "expected_checkout_date") {
        Err(requested) => {
            return Ok(DraftOutcome::UnreadableBackfillDate {
                field: "expected_checkout_date",
                requested,
            })
        }
        Ok(value) => value.map(|(_, parsed)| parsed),
    };

    if let (Some(check_in), Some(check_out)) = (check_in, check_out) {
        if check_out <= check_in {
            return Ok(DraftOutcome::CheckOutNotAfterCheckIn {
                check_in_date: check_in.format("%Y-%m-%d").to_string(),
                check_out_date: check_out.format("%Y-%m-%d").to_string(),
            });
        }
    }

    // **Ô `check_out_date` có mặt = khách ĐÃ trả phòng.** Một ngày trả ở tương
    // lai thì mâu thuẫn với chính điều đó, và `backfill_stay` từ chối. Không tự
    // chuyển giá trị ấy sang `expected_checkout_date`: chuyển hộ là tự quyết
    // rằng khách vẫn còn trong phòng, mà quyết định ấy đổi cả trạng thái phòng
    // lẫn dòng tiền.
    if let Some(check_out) = check_out {
        if check_out > today {
            return Ok(DraftOutcome::BackfillCheckOutInTheFuture {
                requested: check_out.format("%Y-%m-%d").to_string(),
                today: now_local_date.to_string(),
            });
        }
    }

    let still_staying = check_out.is_none();
    if still_staying {
        if let Some(expected_checkout) = expected_checkout {
            if expected_checkout <= today {
                return Ok(DraftOutcome::ExpectedCheckoutNotAfterToday {
                    requested: expected_checkout.format("%Y-%m-%d").to_string(),
                    today: now_local_date.to_string(),
                });
            }
        }
    }

    // Soi ô tiền trước `missing`, cùng lý do như ô ngày — xem
    // `build_check_in_draft`.
    let paid_amount = match read_amount(args, "paid_amount") {
        Ok(value) => value,
        Err(outcome) => return Ok(outcome),
    };

    let mut missing = Vec::new();
    if room_id.is_none() {
        missing.push("room_id".to_string());
    }
    if guests.is_empty() {
        missing.push("guests".to_string());
    }
    if check_in.is_none() {
        missing.push("check_in_date".to_string());
    }
    // Bắt buộc **có điều kiện**: chỉ khi khách còn ở. JSON schema không nói được
    // câu điều kiện ấy (mô tả ô có nói, nhưng mô tả không phải hàng rào), nên
    // đây là chỗ duy nhất canh nó. Thiếu mà vẫn dựng thẻ thì `backfill_stay` nổ
    // "Thiếu ngày ra dự kiến cho khách còn ở" **sau** khi lễ tân bấm *Đồng ý*.
    if still_staying && expected_checkout.is_none() {
        missing.push("expected_checkout_date".to_string());
    }
    if !missing.is_empty() {
        return Ok(DraftOutcome::MissingFields(missing));
    }

    let room_id = room_id.expect("đã kiểm ở trên").to_string();
    let check_in = check_in.expect("đã kiểm ở trên");

    // Mốc cuối của kỳ ở: ngày trả thật nếu khách đã đi, ngày trả dự kiến nếu còn
    // ở. Đúng `BackfillDates.end` mà `validate_backfill_request` dựng, nên số
    // đêm lệnh tính ra và khoảng ngày thẻ hỏi giá là một.
    let stay_end = check_out
        .or(expected_checkout)
        .expect("khách còn ở đã bắt buộc có ngày trả dự kiến ở trên");

    // Chuẩn hoá về `YYYY-MM-DD` có đệm 0 trước khi vào payload — `backfill_stay`
    // dò trùng lịch bằng so sánh CHUỖI (`rc.date >= ? AND rc.date < ?`), y như
    // `create_reservation_tx`, nên một dấu 0 thiếu làm phép dò quét nhầm khoảng.
    let check_in_date = check_in.format("%Y-%m-%d").to_string();
    let stay_end_date = stay_end.format("%Y-%m-%d").to_string();

    // Ba tham số phải khớp đúng cái lệnh sẽ dùng:
    //   • cùng khoảng ngày (`check_in` → `stay_end`);
    //   • `"nightly"` — `backfill_stay_tx` ghi cứng `pricing_type='nightly'`;
    //   • `None` cho số khách — quầy không thu phụ thu thêm người. **Chú ý:**
    //     `payload.guests` là DANH SÁCH KHÁCH, không phải tham số `guests:
    //     Option<i32>` của preview; truyền `guests.len()` vào đây là thẻ báo cao
    //     hơn số thực thu. Cùng luật `check_in`/`draft_reserve`/`CheckinSheet`.
    let preview_result = pricing_service::calculate_room_price_preview(
        pool,
        &room_id,
        &check_in_date,
        &stay_end_date,
        "nightly",
        None,
    )
    .await
    .map_err(|error| {
        CommandError::user(
            codes::AGENT_PREVIEW_UNAVAILABLE,
            format!("Không tra được giá phòng nên chưa dựng được thẻ: {error}"),
        )
    })?;

    // Đọc `total` ở dạng ĐÃ CÓ KIỂU, trước khi mã hoá sang JSON. `MoneyVnd` là
    // `i64`; đi vòng qua `preview["total"].as_i64()` là mở đường cho một
    // `Option` phải xử lý và một chỗ để ai đó nhét số mặc định vào.
    let total_price = preview_result.total;

    let preview = serde_json::to_value(&preview_result).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không mã hoá được báo giá: {error}"),
        )
    })?;

    let payload = BackfillStayRequest {
        room_id: room_id.clone(),
        guests,
        check_in_date,
        check_out_date: check_out.map(|date| date.format("%Y-%m-%d").to_string()),
        expected_checkout_date: expected_checkout.map(|date| date.format("%Y-%m-%d").to_string()),
        // ĐÂY. Số của preview, không phải `args["total_price"]`. Không có nhánh
        // nào đọc tham số ấy, kể cả khi model gửi kèm.
        total_price,
        // Tiền khách đã trả thì model **được** đưa: đó là một sự kiện đã xảy ra
        // ở quầy, không phải một con số tính ra được. Vắng ⇒ 0, chứ không suy ra
        // "chắc trả đủ rồi" — suy hộ ở đây là xoá một khoản nợ.
        paid_amount: paid_amount.unwrap_or(0),
        // Viết cứng, khớp mặc định của `backfill_stay_tx`
        // (`req.source.as_deref().unwrap_or("walk-in")`): gửi `None` thì thẻ
        // hiện "—" trong khi sổ ghi "walk-in" — thẻ nói một đằng, bản ghi một
        // nẻo. Cùng cách `build_check_in_draft`/`build_reserve_draft` làm.
        source: Some("walk-in".to_string()),
        notes: trimmed_arg(args, "notes").map(str::to_string),
    };

    let warnings = build_backfill_warnings(pool, &payload, &stay_end_date, still_staying).await?;
    let display = build_backfill_display(&payload);

    Ok(DraftOutcome::Ready(Box::new(ProposedAction {
        kind: BACKFILL_ACTION_KIND.to_string(),
        payload: ActionPayload::Backfill(payload),
        display,
        preview,
        warnings,
        built_at_ms: chrono::Utc::now().timestamp_millis(),
    })))
}

/// Cảnh báo cho thẻ ghi bù — tra từ PMS, không phải lời model viết ra.
///
/// Mỗi câu dưới đây ứng với một nhánh `backfill_stay` sẽ **từ chối** sau khi lễ
/// tân bấm *Đồng ý*. Đó là tiêu chuẩn để một câu được nằm ở đây: cảnh báo bật
/// lên ở ca hợp lệ dạy lễ tân bỏ qua cảnh báo, còn tệ hơn không cảnh báo.
async fn build_backfill_warnings(
    pool: &Pool<Sqlite>,
    payload: &BackfillStayRequest,
    stay_end_date: &str,
    still_staying: bool,
) -> CommandResult<Vec<String>> {
    let mut warnings = Vec::new();

    // Phòng bận **trong khoảng ngày ghi bù**. Dùng đúng truy vấn của tool đọc
    // `check_room_availability`, và nó có cùng biên `[from, to)` với phép dò
    // trùng của `backfill_stay_tx` — nên câu này đúng là câu lệnh sẽ nói.
    let free_rooms = load_free_rooms_between(pool, &payload.check_in_date, stay_end_date)
        .await
        .map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Không đọc được lịch phòng trống: {error}"),
            )
        })?;
    if !free_rooms
        .iter()
        .any(|room| room.room_id == payload.room_id)
    {
        warnings.push(format!(
            "Phòng đã có lượt ở khác trong khoảng {} – {}. `backfill_stay` sẽ từ chối vì trùng lịch.",
            format_vn_date(&payload.check_in_date),
            format_vn_date(stay_end_date),
        ));
    }

    // Khách còn ở thì lệnh đòi phòng đang TRỐNG ngay lúc này — nó sắp bật phòng
    // sang "có khách". Đây là câu "phòng đang có khách ở" của thẻ nhận phòng,
    // nhưng chỉ đúng cho nhánh còn-ở: với một kỳ ở đã kết thúc, trạng thái phòng
    // bây giờ không liên quan gì.
    if still_staying {
        let rooms = load_room_status_now(pool).await.map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Không đọc được trạng thái phòng: {error}"),
            )
        })?;
        if let Some(room) = rooms.iter().find(|room| room.room_id == payload.room_id) {
            if !room.status.eq_ignore_ascii_case(status::room::VACANT) {
                warnings.push(format!(
                    "Khách còn ở nhưng phòng đang ở trạng thái «{}», không phải trống. \
                     `backfill_stay` chỉ ghi bù khách còn ở vào phòng đang trống.",
                    room.status
                ));
            }
        }
    }

    // Đã thu nhiều hơn tiền phòng: `validate_backfill_request` từ chối thẳng.
    // KHÔNG tự kẹp số về cho vừa — số tiền đã thu là một sự kiện, không phải một
    // ô để làm tròn.
    if payload.paid_amount > payload.total_price {
        warnings.push(format!(
            "Đã thu {} nhưng tiền phòng CapyInn tính ra là {}. `backfill_stay` sẽ từ chối \
             vì số đã thu không được vượt tiền phòng.",
            format_vnd(payload.paid_amount),
            format_vnd(payload.total_price),
        ));
    }

    // Model điền cả hai ô ngày trả: lệnh lấy ô "đã trả" và **bỏ qua** ô dự kiến
    // (`match (&req.check_out_date, _)` khớp nhánh đầu). Nói ra, không lặng lẽ
    // gỡ ô thừa ra khỏi payload — bỏ im lặng đúng là lớp lỗi cả đợt này sửa.
    if !still_staying && payload.expected_checkout_date.is_some() {
        warnings.push(
            "Khách đã trả phòng nên CapyInn dùng ngày trả thật; ngày trả dự kiến trên thẻ \
             sẽ không được ghi."
                .to_string(),
        );
    }

    // Cảnh báo thiếu giấy tờ giữ nguyên từ thẻ nhận phòng — cùng một hồ sơ khai
    // báo tạm trú, cùng một chỗ thiếu.
    for guest in &payload.guests {
        if guest.doc_number.trim().is_empty() {
            warnings.push(format!(
                "Khách «{}» chưa có số giấy tờ. Hồ sơ khai báo tạm trú của lượt ở này sẽ thiếu.",
                guest.full_name.trim()
            ));
        }
    }

    Ok(warnings)
}

/// Ô `guest_name` trông như đang cõng nhiều hơn một tên.
///
/// Cố ý chỉ dò dấu phân cách, không tách tên: mục đích là **nói ra một nghi
/// ngờ** cho người duyệt, không phải tự quyết định. Một dương tính giả chỉ tốn
/// của lễ tân một dòng cảnh báo phải đọc; một âm tính giả là một khách bị bỏ đi
/// không ai biết.
fn looks_like_more_than_one_name(name: &str) -> bool {
    const SEPARATORS: [char; 5] = [',', ';', '/', '&', '+'];

    if name.contains(SEPARATORS) {
        return true;
    }
    // " và " có khoảng trắng hai bên: không thì "Hoàng Văn Đà" (chứa "và"
    // không dấu cách) cũng bị báo. Hạ chữ thường vì "Và" đầu câu là có thật.
    name.to_lowercase().contains(" và ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateGuestRequest;

    /// Mở `ActionPayload` ra đúng biến thể mong đợi, và **panic khi sai biến
    /// thể** thay vì im lặng trả về `None`. Một thẻ mang nhầm payload là một
    /// lệnh PMS sai loại — nó phải làm test đỏ ngay tại dòng gọi.
    fn check_in_payload(action: &ProposedAction) -> &CheckInRequest {
        match &action.payload {
            ActionPayload::CheckIn(payload) => payload,
            other => panic!("mong đợi payload nhận phòng, nhận {other:?}"),
        }
    }

    fn reserve_payload(action: &ProposedAction) -> &CreateReservationRequest {
        match &action.payload {
            ActionPayload::Reserve(payload) => payload,
            other => panic!("mong đợi payload đặt phòng, nhận {other:?}"),
        }
    }

    fn backfill_payload(action: &ProposedAction) -> &BackfillStayRequest {
        match &action.payload {
            ActionPayload::Backfill(payload) => payload,
            other => panic!("mong đợi payload ghi bù, nhận {other:?}"),
        }
    }

    /// Khách điền **đủ mười trường** của `CreateGuestRequest`, mỗi trường một
    /// giá trị không trùng nhau và không là chuỗi con của nhau. Đây là điều
    /// kiện để `the_card_shows_every_field_of_the_payload` có nghĩa: nó dò
    /// từng giá trị lá trên thẻ, nên một trường bỏ trống trong fixture sẽ được
    /// bỏ qua và che mất đúng cái lỗi cần bắt.
    ///
    /// Thêm trường mới vào `CreateGuestRequest` sẽ làm literal này **không
    /// biên dịch được** — buộc người thêm phải cho nó một giá trị, rồi test
    /// bên dưới bắt tiếp nếu thẻ không hiện nó ra.
    fn sample_guest(full_name: &str) -> CreateGuestRequest {
        CreateGuestRequest {
            guest_type: Some("domestic".to_string()),
            full_name: full_name.to_string(),
            doc_number: "079201001234".to_string(),
            dob: Some("1992-03-15".to_string()),
            gender: Some("Nữ".to_string()),
            nationality: Some("Việt Nam".to_string()),
            address: Some("12 Lê Lợi, Đà Nẵng".to_string()),
            visa_expiry: Some("2027-01-31".to_string()),
            scan_path: Some("/anh/giay-to-1.jpg".to_string()),
            phone: Some("0909000111".to_string()),
        }
    }

    /// Khách thứ hai chỉ có ba trường như model thật vẫn gửi, và không trường
    /// nào trùng khách thứ nhất — để một thẻ chỉ hiện khách đầu tiên bị bắt.
    fn second_sample_guest() -> CreateGuestRequest {
        CreateGuestRequest {
            guest_type: None,
            full_name: "Lê Văn Cường".to_string(),
            doc_number: "079088007766".to_string(),
            dob: None,
            gender: None,
            nationality: None,
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: Some("0912345678".to_string()),
        }
    }

    fn sample_payload() -> CheckInRequest {
        CheckInRequest {
            room_id: "R1".to_string(),
            guests: vec![sample_guest("Trần Thị Bích"), second_sample_guest()],
            nights: 2,
            source: Some("walk-in".to_string()),
            notes: Some("khách quen".to_string()),
            paid_amount: Some(500_000),
            pricing_type: Some("nightly".to_string()),
            // Trợ lý không bao giờ tự đặt giá tay (xem `build_check_in_draft`),
            // nên `None` ở đây mới đúng hình dạng payload thật đi qua đường này.
            rate_override_per_night: None,
        }
    }

    /// Mọi giá trị lá nằm dưới một trường lồng của payload phải xuất hiện
    /// nguyên văn ở đâu đó trên thẻ.
    ///
    /// `null` bị bỏ qua vì không mang thông tin nào để hiện. Chuỗi rỗng cũng
    /// vậy — `doc_number` rỗng đã có cảnh báo riêng lo, xem
    /// `build_warnings`.
    fn assert_nested_leaves_are_on_the_card(path: &str, value: &Value, shown: &str) {
        match value {
            Value::Object(fields) => {
                for (key, nested) in fields {
                    assert_nested_leaves_are_on_the_card(&format!("{path}.{key}"), nested, shown);
                }
            }
            Value::Array(entries) => {
                for (index, nested) in entries.iter().enumerate() {
                    assert_nested_leaves_are_on_the_card(
                        &format!("{path}[{index}]"),
                        nested,
                        shown,
                    );
                }
            }
            Value::Null => {}
            leaf => {
                let text = leaf
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| leaf.to_string());
                if text.trim().is_empty() {
                    return;
                }
                assert!(
                    shown.contains(&text),
                    "giá trị `{path}` = `{text}` của payload không hiện trên thẻ xác nhận.\nThẻ đang hiện:\n{shown}"
                );
            }
        }
    }

    /// Đây là luật số một của thiết kế: người dùng duyệt đúng cái sẽ được gửi.
    /// Thêm trường vào `CheckInRequest` **hoặc vào `CreateGuestRequest`** mà
    /// quên hiện lên thẻ là test này đỏ.
    ///
    /// Bản cũ chỉ duyệt bảy khoá tầng ngoài của `CheckInRequest`. `guests` có
    /// mặt trong `display` nên nó xanh, trong khi số giấy tờ và số điện thoại
    /// của từng khách — thứ sẽ được ghi thẳng vào `guests.doc_number` rồi đi
    /// vào khai báo tạm trú — không hề hiện ra. Giờ nó đệ quy xuống từng lá.
    #[test]
    fn the_card_shows_every_field_of_the_payload() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-08");

        let encoded = serde_json::to_value(&payload).expect("payload phải serialize được");
        let fields = encoded.as_object().expect("payload là một object");
        let shown = display
            .values()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");

        for (key, value) in fields {
            assert!(
                display.contains_key(key),
                "trường `{key}` của payload không hiện trên thẻ xác nhận"
            );
            if value.is_object() || value.is_array() {
                assert_nested_leaves_are_on_the_card(key, value, &shown);
            }
        }
    }

    /// Nói thẳng ra cái mà test đệ quy ở trên chỉ nói gián tiếp: con số CCCD
    /// mà con người sắp cho phép ghi vào hồ sơ khai báo tạm trú phải nằm ngay
    /// trên thẻ họ đang nhìn.
    #[test]
    fn the_card_shows_the_document_number_it_is_about_to_write() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-08");

        let first = display
            .get("Khách 1")
            .expect("phải có dòng riêng cho khách thứ nhất");
        assert!(first.contains("Trần Thị Bích"), "{first}");
        assert!(first.contains("079201001234"), "{first}");
        assert!(first.contains("0909000111"), "{first}");

        let second = display
            .get("Khách 2")
            .expect("phải có dòng riêng cho khách thứ hai");
        assert!(second.contains("Lê Văn Cường"), "{second}");
        assert!(second.contains("079088007766"), "{second}");
    }

    /// Khoá của `display` là `BTreeMap`, tức thẻ hiện theo thứ tự chuỗi. Không
    /// đệm 0 thì "Khách 10" đứng trước "Khách 2" và danh sách khách đọc lộn
    /// xộn đúng lúc đông người nhất.
    #[test]
    fn ten_guests_stay_in_order_on_the_card() {
        let payload = CheckInRequest {
            guests: (1..=10)
                .map(|index| {
                    let mut guest = second_sample_guest();
                    guest.full_name = format!("Khách số {index}");
                    guest.doc_number = format!("0790000000{index:02}");
                    guest.phone = None;
                    guest
                })
                .collect(),
            ..sample_payload()
        };

        let display = build_check_in_display(
            &payload,
            &serde_json::json!({ "total": 0 }),
            "2026-08-06",
            "2026-08-08",
        );

        let order: Vec<&str> = display
            .keys()
            .filter(|key| key.starts_with("Khách "))
            .map(String::as_str)
            .collect();
        assert_eq!(order.first(), Some(&"Khách 01"), "{order:?}");
        assert_eq!(order.last(), Some(&"Khách 10"), "{order:?}");
    }

    #[test]
    fn the_card_shows_the_preview_total_not_a_model_number() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-08");

        assert!(display
            .get("total")
            .expect("phải có dòng tổng tiền")
            .contains("700"));
    }

    /// Lễ tân phải đối chiếu được ngày trên thẻ với câu mình vừa gõ, **trước**
    /// khi bấm Đồng ý. Đây đúng là thứ đã thiếu hôm 06/08: thẻ hiện `nights`
    /// nhưng không hiện ngày nào, nên "checkin 8 out 9" và một thẻ ghi hôm nay
    /// nhìn giống hệt nhau.
    ///
    /// Định dạng Việt Nam `DD/MM/YYYY`, không phải `YYYY-MM-DD` của máy: người
    /// đọc thẻ là người, và `08/06` với `06/08` là hai ngày khác nhau.
    #[test]
    fn the_card_shows_the_stay_dates_in_vietnamese_format() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-07");

        assert_eq!(
            display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 06/08/2026")
        );
        assert_eq!(
            display.get("check_out_date").map(String::as_str),
            Some("07/08/2026")
        );
    }

    #[tokio::test]
    async fn a_draft_without_guests_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({ "room_id": "R1", "nights": 2 });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => assert!(fields.contains(&"guests".to_string())),
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_draft_without_a_room_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({ "nights": 2, "guests": [{ "full_name": "Nam" }] });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => {
                assert!(fields.contains(&"room_id".to_string()))
            }
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_draft_without_nights_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({ "room_id": "R1", "guests": [{ "full_name": "Nam" }] });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => {
                assert!(fields.contains(&"nights".to_string()))
            }
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    /// `nights < 1` đi chung nhánh với vắng mặt — `unwrap_or(0)` biến "0" thành
    /// cùng một con số 0 rồi cùng rớt vào điều kiện `nights < 1`.
    #[tokio::test]
    async fn a_draft_with_zero_nights_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({
            "room_id": "R1",
            "nights": 0,
            "guests": [{ "full_name": "Nam" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => {
                assert!(fields.contains(&"nights".to_string()))
            }
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_draft_for_an_unknown_room_fails_instead_of_quoting_a_default() {
        let pool = test_pool().await;
        let args = serde_json::json!({
            "room_id": "khong-ton-tai",
            "nights": 2,
            "guests": [{ "full_name": "Nam" }]
        });

        let error = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect_err("không tra được giá thì không được dựng thẻ");

        assert_eq!(error.code, codes::AGENT_PREVIEW_UNAVAILABLE);
    }

    // ─── Happy-path coverage for `DraftOutcome::Ready` ───
    //
    // Every test above stops at `MissingFields` or the unknown-room `Err`, so
    // `build_warnings` and the way `payload`/`preview`/`warnings` get assembled
    // into a `ProposedAction` never actually ran. These seed a real room,
    // following the same minimum recipe proven in `tools.rs`'s
    // `quote_room_price_prices_a_seeded_room_over_two_weekday_nights`: just a
    // `rooms` row, no `pricing_rules`/`room_types` row required for
    // `calculate_room_price_preview` to succeed.

    /// Chứng minh cả đường `Ready`: thẻ hiện đúng phòng/khách đã seed, tổng
    /// tiền trên thẻ là tổng của preview thật (không phải 0, không phải mặc
    /// định house 350k/400k), và `payload` mang đúng những gì đã truyền vào.
    /// Phòng seed sạch và còn trống nên `warnings` phải rỗng — đối chứng cho
    /// test cảnh báo ngay bên dưới, để test đó không thể đúng một cách vô
    /// nghĩa (lúc nào cũng có cảnh báo bất kể trạng thái phòng).
    #[tokio::test]
    async fn a_draft_with_a_seeded_room_and_guest_is_ready_with_the_preview_total() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-ready",
            "P701",
            "Deluxe Balcony",
            500_000,
            "vacant",
        )
        .await;

        // 2026-06-01 là thứ Hai, 2026-06-03 là thứ Tư (đã kiểm bằng `date`/
        // `datetime`, không chỉ đọc comment) — kỳ ở này không dính đêm cuối
        // tuần nào, nên mức uplift cuối tuần mặc định 20% mà một phòng không
        // có `pricing_rules` sẽ rơi vào phải ra 0, và tổng phải đúng bằng 2
        // đêm x base_price, không hơn không kém.
        let args = serde_json::json!({
            "room_id": "room-ready",
            "nights": 2,
            "guests": [
                {
                    "full_name": "Nguyễn Văn Nam",
                    "doc_number": "079201001234",
                    "phone": "0909000111"
                }
            ],
            "source": "OTA",
            "notes": "khách quen",
            "paid_amount": 300_000,
            "pricing_type": "nightly"
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("dữ liệu hợp lệ với phòng có thật không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(action.kind, CHECK_IN_ACTION_KIND);

        // Thẻ hiện đúng phòng và khách đã seed.
        assert_eq!(
            action.display.get("room_id").map(String::as_str),
            Some("room-ready")
        );
        assert_eq!(
            action.display.get("guests").map(String::as_str),
            Some("1 người")
        );
        assert_eq!(
            action.display.get("Khách 1").map(String::as_str),
            Some("Nguyễn Văn Nam · CCCD: 079201001234 · SĐT: 0909000111")
        );

        // Tổng trên thẻ phải là tổng của preview thật: 2 đêm x 500.000 base
        // price, không cuối tuần, không phụ thu — không phải 0 và không phải
        // một trong hai số mặc định house (350k hay daily_rate mặc định 400k
        // của `PricingRule::default()`) mà một rule bị rớt về default sẽ lộ ra.
        let preview_total = action
            .preview
            .get("total")
            .and_then(Value::as_i64)
            .expect("preview phải có total");
        assert_eq!(preview_total, 1_000_000);
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some("1.000.000 ₫")
        );

        // payload round-trip đúng những gì đã truyền vào.
        let payload = check_in_payload(&action);
        assert_eq!(payload.room_id, "room-ready");
        assert_eq!(payload.nights, 2);
        assert_eq!(payload.guests.len(), 1);
        assert_eq!(payload.guests[0].full_name, "Nguyễn Văn Nam");
        assert_eq!(payload.guests[0].doc_number, "079201001234");
        assert_eq!(payload.guests[0].phone.as_deref(), Some("0909000111"));
        assert_eq!(payload.source.as_deref(), Some("OTA"));
        assert_eq!(payload.notes.as_deref(), Some("khách quen"));
        assert_eq!(payload.paid_amount, Some(300_000));
        assert_eq!(payload.pricing_type.as_deref(), Some("nightly"));

        // Phòng sạch, không ai đang ở — không được có cảnh báo nào.
        assert!(
            action.warnings.is_empty(),
            "phòng sạch và trống không được có cảnh báo: {:?}",
            action.warnings
        );
    }

    // ─── Số trên thẻ phải bằng số lệnh nhận phòng thật sẽ ghi ───
    //
    // Mọi fixture `seed_room` ở trên bỏ trống `max_guests`/`extra_person_fee`,
    // nên schema điền mặc định (2, 0) và khoản phụ thu thêm người **luôn bằng
    // 0** — chênh lệch giữa hai cách gọi preview bị triệt tiêu về cấu trúc, dù
    // có sai. Hai test dưới đây dựng đúng cái phòng làm khoản đó khác 0.

    /// Preview của thẻ phải hỏi giá y như `stay_lifecycle::check_in` hỏi, tức
    /// **không** kèm số khách. Phòng dưới đây chuẩn 2 khách, phụ thu 150.000₫
    /// mỗi khách vượt mốc mỗi đêm; 3 khách × 2 đêm sẽ đội thêm 300.000₫ nếu
    /// thẻ lỡ gửi số khách đi.
    #[tokio::test]
    async fn the_card_does_not_quote_an_extra_person_fee_the_check_in_will_not_charge() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(
            &pool,
            "room-extra",
            "P703",
            "Family Room",
            500_000,
            "vacant",
        )
        .await;

        // 2026-06-01 thứ Hai → 2026-06-03 thứ Tư: không đêm cuối tuần nào, nên
        // 1.000.000₫ là con số duy nhất đúng.
        let args = serde_json::json!({
            "room_id": "room-extra",
            "nights": 2,
            "guests": [
                { "full_name": "Nguyễn Văn Nam" },
                { "full_name": "Trần Thị Hoa" },
                { "full_name": "Lê Văn Cường" }
            ]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("dữ liệu hợp lệ không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(
            action.preview.get("total").and_then(Value::as_i64),
            Some(1_000_000),
            "preview của thẻ đang tính phụ thu thêm người mà quầy không thu"
        );
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some("1.000.000 ₫")
        );
    }

    /// Đường nối thật: lấy đúng `payload` mà thẻ mang, chạy qua chính
    /// `stay_lifecycle::check_in` — hàm mà nút "Đồng ý" gọi tới — rồi so tổng
    /// trên thẻ với tổng ghi vào `bookings.total_price`.
    ///
    /// Đây là chỗ khách hàng nghe một con số và sổ sách ghi một con số khác,
    /// nên nó phải có một test bám vào cả hai đầu, không phải hai test rời
    /// nhau mỗi bên tự khẳng định mình đúng.
    #[tokio::test]
    async fn the_card_total_is_the_total_the_real_check_in_records() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(
            &pool,
            "room-seam",
            "P704",
            "Family Room",
            500_000,
            "vacant",
        )
        .await;

        // `check_in_tx` chốt kỳ ở theo `Local::now()`, không nhận ngày truyền
        // vào, nên thẻ phải được dựng cho đúng hôm nay thì hai bên mới báo giá
        // cùng một khoảng ngày. Cả hai đi qua cùng phụ thu cuối tuần / ngày lễ
        // nên khẳng định "bằng nhau" đúng bất kể hôm nay là thứ mấy.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let args = serde_json::json!({
            "room_id": "room-seam",
            "nights": 2,
            "guests": [
                { "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" },
                { "full_name": "Trần Thị Hoa", "doc_number": "079301005678" },
                { "full_name": "Lê Văn Cường", "doc_number": "079201009999" }
            ]
        });

        let outcome = build_check_in_draft(&pool, &args, &today)
            .await
            .expect("dữ liệu hợp lệ không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        let card_total = action
            .preview
            .get("total")
            .and_then(Value::as_i64)
            .expect("thẻ phải có tổng tiền");

        let booking = crate::services::booking::stay_lifecycle::check_in(
            &pool,
            check_in_payload(&action).clone(),
            Some("user-test".to_string()),
        )
        .await
        .expect("payload của thẻ phải nhận phòng được");

        assert_eq!(
            card_total, booking.total_price,
            "thẻ báo {card_total} nhưng lượt ở ghi {}",
            booking.total_price
        );
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some(format_vnd(booking.total_price).as_str()),
            "dòng tổng tiền lễ tân đọc cho khách phải là số sổ sách ghi"
        );
    }

    /// `build_warnings` đọc `rooms.status` thật từ PMS, không phải câu do model
    /// tự viết ra — test này là bằng chứng tự động cho đúng luật đó, thay vì
    /// chỉ dựa vào người đọc code. Không so sánh rỗng/không-rỗng chung chung:
    /// so khớp đúng nội dung tiếng Việt và đúng số lượng, để một cảnh báo giả
    /// hoặc một cảnh báo thứ hai lọt vào cũng bị bắt.
    ///
    /// Câu cảnh báo phải nói ra rằng lệnh sẽ **từ chối**, không phải một lời
    /// "lưu ý": `check_in_tx` bắt mọi `status` khác `vacant` là `Conflict` và
    /// trả về ngay, nên phòng không trống thì không nhận phòng được, chấm hết.
    /// Cảnh báo của `draft_reserve`/`draft_backfill` viết thẳng điều đó; câu này
    /// từng không, nên lễ tân đọc ra là lời khuyên có thể bỏ qua.
    ///
    /// Fixture cố tình seed `occupied` — một **thành viên thật** của
    /// `status::room` mà đường ghi sinh ra được. Bản trước seed `"dirty"`, thứ
    /// chưa từng có trong `status::room`, nên nó xanh cho một nhánh mà PMS thật
    /// không bao giờ chạm tới. Phòng không có `bookings` hàng `active` nào, nên
    /// đây đúng là câu trạng thái chứ không phải câu "đang có khách ở".
    #[tokio::test]
    async fn a_ready_draft_carries_a_pms_warning_when_the_room_is_not_vacant() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-occupied",
            "P702",
            "Standard Room",
            300_000,
            status::room::OCCUPIED,
        )
        .await;

        // Khách có số giấy tờ: test này canh đúng một cảnh báo trạng thái
        // phòng, không được vô tình cõng thêm cảnh báo thiếu giấy tờ.
        let args = serde_json::json!({
            "room_id": "room-occupied",
            "nights": 1,
            "guests": [{ "full_name": "Trần Thị Hoa", "doc_number": "079301005678" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("phòng không trống vẫn tra được giá — không phải lỗi hệ thống");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(
            action.warnings,
            vec![
                "Phòng đang ở trạng thái «occupied», không phải trống. `check_in` sẽ TỪ CHỐI: \
                 nó chỉ nhận phòng đang trống."
                    .to_string()
            ]
        );
    }

    /// Trợ lý dựng được một khách mà chính form nhận phòng của PMS sẽ từ chối:
    /// `draft.rs` mặc định `doc_number` thành `""` và
    /// `validate_check_in_request` không kiểm trường đó. Đây **không** phải
    /// chặn cứng — con người vẫn là bước duyệt theo đúng thiết kế — nhưng thẻ
    /// phải nói ra sự chênh lệch đó thay vì để nó lặng lẽ trôi qua.
    #[tokio::test]
    async fn a_guest_without_a_document_number_gets_a_warning_naming_the_manual_form() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-nodoc",
            "P705",
            "Standard Room",
            300_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-nodoc",
            "nights": 1,
            "guests": [
                { "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" },
                { "full_name": "Phạm Thị Dung" }
            ]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("thiếu giấy tờ không phải lỗi hệ thống");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        // Đúng một cảnh báo, và nó phải gọi tên đúng người thiếu giấy tờ —
        // không phải người đã có.
        assert_eq!(action.warnings.len(), 1, "{:?}", action.warnings);
        let warning = &action.warnings[0];
        assert!(warning.contains("Phạm Thị Dung"), "{warning}");
        assert!(!warning.contains("Nguyễn Văn Nam"), "{warning}");
        assert!(warning.contains("giấy tờ"), "{warning}");
        assert!(
            warning.contains("làm tay"),
            "cảnh báo phải nói rõ form làm tay không nhận: {warning}"
        );
    }

    // ─── Ô ngày nhận phòng, và luật từ chối ───
    //
    // Ngày 06/08/2026 lễ tân gõ "có booking mới phòng 4B, checkin 8 out 9 tháng
    // 8". Trợ lý nhận phòng ngay lúc đó — `check_in_at` trùng `created_at` tới
    // micro-giây. Ngày 8 không đi tới đâu cả.
    //
    // Mọi test dưới đây **seed phòng thật**, kể cả những test chỉ mong một lời
    // từ chối. Phòng không tồn tại thì preview hỏng và test vẫn đỏ ngay cả khi
    // luật ngày bị gỡ sạch — đỏ vì `AGENT_PREVIEW_UNAVAILABLE`, tức đỏ vì một
    // vế khác và không canh gì. Có phòng thì nhánh ngày là thứ **duy nhất**
    // đứng giữa tool call và một cái thẻ.

    /// Ngày nhận đúng hôm nay: đường thường, thẻ dựng bình thường, và hai dòng
    /// ngày trên thẻ nói đúng kỳ ở vừa dùng để hỏi giá.
    #[tokio::test]
    async fn a_draft_for_today_still_builds_the_card() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-today",
            "P801",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-today",
            "nights": 2,
            "check_in_date": "2026-06-01",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày nhận đúng hôm nay không phải lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        assert_eq!(action.kind, CHECK_IN_ACTION_KIND);
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 01/06/2026")
        );
        assert_eq!(
            action.display.get("check_out_date").map(String::as_str),
            Some("03/06/2026")
        );
    }

    /// Đường cũ không gãy. Khách đứng ở quầy và không ai nêu ngày nào là ca
    /// thường nhất của một quầy lễ tân; một trường tuỳ chọn không được biến nó
    /// thành một vòng `missing_fields` thừa trong ngân sách 4 vòng.
    #[tokio::test]
    async fn a_draft_without_a_date_still_builds_the_card_the_old_way() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-nodate",
            "P802",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        // Không có `check_in_date` — đúng hình dạng tool call trước bản vá này.
        let args = serde_json::json!({
            "room_id": "room-nodate",
            "nights": 2,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("không nêu ngày là ca hợp lệ");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        // Vắng ngày thì thẻ vẫn phải nói ra nó đang nhận phòng cho hôm nay —
        // im lặng chính là chỗ con bug cũ trốn được.
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 01/06/2026")
        );
    }

    /// Chiều tương lai. Ngày mai **không** dựng được thẻ nhận phòng, và ngày
    /// người dùng nêu đi ra nguyên văn để vòng lặp nhắc lại đúng nó cho model
    /// khi chỉ sang `draft_reserve`.
    #[tokio::test]
    async fn a_draft_for_tomorrow_is_refused_and_points_at_the_reservation_tool() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-tomorrow",
            "P803",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-tomorrow",
            "nights": 1,
            "check_in_date": "2026-06-02",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày tương lai không phải lỗi hệ thống");

        // Nhánh `Ready` panic ở đây: không có `ProposedAction` nào được dựng.
        match outcome {
            DraftOutcome::WrongDateForCheckIn {
                requested,
                is_future,
            } => {
                assert_eq!(requested, "2026-06-02");
                assert!(is_future, "ngày mai phải được nhận ra là tương lai");
            }
            other => panic!("mong đợi WrongDateForCheckIn, nhận {other:?}"),
        }
    }

    /// Chiều quá khứ — cấm ngang chiều tương lai. Lấy hôm nay thay cho hôm qua
    /// cũng là thay một ngày người dùng đã nêu, và nó ghi sai luôn cả số đêm đã
    /// ở thật.
    #[tokio::test]
    async fn a_draft_for_yesterday_is_refused_as_a_past_date() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-yesterday",
            "P804",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-yesterday",
            "nights": 1,
            "check_in_date": "2026-05-31",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày quá khứ không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::WrongDateForCheckIn {
                requested,
                is_future,
            } => {
                assert_eq!(requested, "2026-05-31");
                assert!(!is_future, "hôm qua không được coi là tương lai");
            }
            other => panic!("mong đợi WrongDateForCheckIn, nhận {other:?}"),
        }
    }

    /// "ngày 8" là đúng chuỗi model có thể gửi khi nó chép lại lời lễ tân. Đọc
    /// không ra thì từ chối — và **không** rơi về hôm nay.
    ///
    /// Cũng ghim luôn việc nó không bị gộp vào `WrongDateForCheckIn`: gộp thì
    /// `is_future` phải đoán, đoán "quá khứ" sẽ đẩy sang `draft_backfill`, tức
    /// ghi bù cho một kỳ ở còn chưa xảy ra.
    #[tokio::test]
    async fn an_unreadable_date_is_refused_instead_of_falling_back_to_today() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-junkdate",
            "P805",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-junkdate",
            "nights": 1,
            "check_in_date": "ngày 8",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày rác không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::UnreadableCheckInDate { requested } => {
                assert_eq!(requested, "ngày 8");
            }
            other => panic!("mong đợi UnreadableCheckInDate, nhận {other:?}"),
        }
    }

    /// So theo **ngày lịch**, không so chuỗi và không so timestamp. `2026-6-1`
    /// và `2026-06-01` là cùng một ngày; so chuỗi thì một dấu 0 thiếu biến một
    /// lượt nhận phòng hợp lệ thành lời từ chối, và lễ tân không có cách nào
    /// hiểu vì sao.
    #[tokio::test]
    async fn a_date_written_without_leading_zeros_is_still_today() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-shortdate",
            "P806",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-shortdate",
            "nights": 1,
            "check_in_date": "2026-6-1",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("cùng một ngày lịch thì không phải lỗi");

        assert!(
            matches!(outcome, DraftOutcome::Ready(_)),
            "cùng một ngày lịch viết thiếu số 0 vẫn phải dựng được thẻ: {outcome:?}"
        );
    }

    /// **Ca thật đã xảy ra**, ghim chiều nguy hiểm: ngày ở tương lai thì không
    /// có action nào mang ngày hôm nay.
    ///
    /// Không dừng ở "outcome khác `Ready`". Soi cả `Debug` của outcome và bắt
    /// nó không được chứa hôm nay dưới bất kỳ dạng nào — `2026-08-06` (payload,
    /// preview) hay `06/08/2026` (thẻ). Gỡ nhánh từ chối thì `Ready` mang cả
    /// hai chuỗi ấy, và test đỏ ngay tại dòng nói đúng lý do nó tồn tại, chứ
    /// không đỏ nhờ một khẳng định phụ nào khác.
    #[tokio::test]
    async fn a_future_date_never_becomes_a_card_stamped_with_today() {
        let pool = test_pool().await;
        seed_room(&pool, "room-4b", "4B", "Standard Room", 400_000, "vacant").await;

        // Đúng câu lễ tân đã gõ: "có booking mới phòng 4B, checkin 8 out 9
        // tháng 8" — trong khi hôm nay là 06/08/2026.
        let args = serde_json::json!({
            "room_id": "room-4b",
            "nights": 1,
            "check_in_date": "2026-08-08",
            "guests": [{ "full_name": "Hyungchul Lee", "doc_number": "M12345678" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-06")
            .await
            .expect("ngày tương lai không phải lỗi hệ thống");

        let dump = format!("{outcome:?}");
        assert!(
            !dump.contains("2026-08-06") && !dump.contains("06/08/2026"),
            "ngày 08/08 bị nuốt mất và hôm nay chui vào kết quả:\n{dump}"
        );
        assert!(
            dump.contains("2026-08-08"),
            "ngày người dùng nêu phải đi tiếp nguyên vẹn:\n{dump}"
        );
        match outcome {
            DraftOutcome::WrongDateForCheckIn { is_future, .. } => {
                assert!(is_future, "08/08 sau 06/08 nên phải là tương lai");
            }
            other => panic!("mong đợi WrongDateForCheckIn, nhận {other:?}"),
        }
    }

    // ─── Thẻ đặt phòng trước (`draft_reserve` → `create_reservation`) ───
    //
    // Đây là **đích** mà lời từ chối của `build_check_in_draft` chỉ tới. Không
    // có nó thì trợ lý chỉ biết nói "không làm được": bug cũ hết xảy ra nhưng
    // lễ tân cũng không đặt được phòng, rồi người ta ngừng dùng trợ lý.
    //
    // Mọi test dưới đây **seed phòng thật**, kể cả những test chỉ mong một lời
    // từ chối — cùng lý do đã ghi ở khối test ngày nhận phòng: phòng không tồn
    // tại thì preview hỏng và test vẫn đỏ ngay cả khi luật bị gỡ sạch, tức đỏ
    // vì fixture chứ không canh gì.

    fn reserve_args(check_in: &str, check_out: &str) -> Value {
        serde_json::json!({
            "room_id": "room-res",
            "guest_name": "Hyungchul Lee",
            "guest_doc_number": "M12345678",
            "check_in_date": check_in,
            "check_out_date": check_out
        })
    }

    /// Đường `Ready` đầy đủ: đúng `kind`, đúng payload, số đêm dẫn xuất, tiền
    /// của preview thật, và `guests` **luôn** `None`.
    ///
    /// 10/06/2026 là thứ Tư và 12/06 là thứ Sáu (kiểm bằng `date`, không chỉ
    /// đọc comment) — hai đêm 10 và 11 đều là ngày thường, nên uplift cuối tuần
    /// mặc định 20% phải ra 0 và tổng phải đúng 2 × base_price.
    #[tokio::test]
    async fn a_reservation_for_a_future_date_is_ready_with_the_preview_total() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P901",
            "Deluxe Balcony",
            500_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("ngày tương lai với phòng có thật không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(action.kind, RESERVE_ACTION_KIND);
        assert_ne!(
            action.kind, CHECK_IN_ACTION_KIND,
            "thẻ đặt phòng đi qua đường nhận phòng là đóng dấu hôm nay lên một kỳ ở của ngày mai"
        );

        let payload = reserve_payload(&action);
        assert_eq!(payload.room_id, "room-res");
        assert_eq!(payload.guest_name, "Hyungchul Lee");
        assert_eq!(payload.check_in_date, "2026-06-10");
        assert_eq!(payload.check_out_date, "2026-06-12");
        // Số đêm DẪN XUẤT từ hai ngày — schema không có ô cho nó.
        assert_eq!(payload.nights, 2);
        // `guests` luôn `None`: quầy không thu phụ thu thêm người.
        assert_eq!(payload.guests, None);

        let preview_total = action
            .preview
            .get("total")
            .and_then(Value::as_i64)
            .expect("preview phải có total");
        assert_eq!(preview_total, 1_000_000);
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some("1.000.000 ₫")
        );
    }

    /// Bất biến mới của cả đợt: **mọi** thẻ đều hiện ngày nhận và ngày trả, định
    /// dạng Việt Nam `DD/MM/YYYY` — người đọc thẻ là người, và `08/06` với
    /// `06/08` là hai ngày khác nhau.
    ///
    /// Và **không** có nhãn "Hôm nay": thẻ này chỉ dựng được cho ngày ở tương
    /// lai, nên nhãn ấy ở đây luôn là một câu sai.
    #[tokio::test]
    async fn the_reservation_card_shows_both_stay_dates_in_vietnamese_format() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P902",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("dữ liệu hợp lệ không được lỗi");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("10/06/2026")
        );
        assert_eq!(
            action.display.get("check_out_date").map(String::as_str),
            Some("12/06/2026")
        );
        assert_eq!(
            action.display.get("nights").map(String::as_str),
            Some("2 đêm")
        );
    }

    /// Luật số một của thiết kế, bản cho thẻ đặt phòng: người dùng duyệt đúng
    /// cái sẽ được gửi. Thêm trường vào `CreateReservationRequest` mà quên hiện
    /// lên thẻ là test này đỏ.
    #[test]
    fn the_reservation_card_shows_every_field_of_the_payload() {
        let payload = CreateReservationRequest {
            room_id: "R201".to_string(),
            guest_name: "Hyungchul Lee".to_string(),
            guest_phone: Some("0909000111".to_string()),
            guest_doc_number: Some("M12345678".to_string()),
            check_in_date: "2026-08-08".to_string(),
            check_out_date: "2026-08-09".to_string(),
            nights: 1,
            deposit_amount: Some(200_000),
            source: Some("phone".to_string()),
            notes: Some("khách quen".to_string()),
            guests: None,
            rate_override_per_night: None,
        };
        let preview = serde_json::json!({ "total": 400_000 });

        let display = build_reserve_display(&payload, &preview);

        let encoded = serde_json::to_value(&payload).expect("payload phải serialize được");
        let fields = encoded.as_object().expect("payload là một object");

        // Vế một: **không bỏ sót trường nào**. Thêm trường vào
        // `CreateReservationRequest` mà quên thẻ là đỏ ngay dòng này.
        for key in fields.keys() {
            assert!(
                display.contains_key(key),
                "trường `{key}` của payload không hiện trên thẻ xác nhận: {display:?}"
            );
        }

        // Vế hai: mỗi dòng nói đúng giá trị của nó. Không dò "giá trị lá có
        // xuất hiện đâu đó trên thẻ" như thẻ nhận phòng — thẻ này **cố ý** định
        // dạng lại ngày (`2026-08-08` → `08/08/2026`) và tiền (`200000` →
        // `200.000 ₫`), nên phép dò nguyên văn sẽ bắt nhầm đúng những dòng làm
        // đúng. So từng dòng thì chặt hơn hẳn: một dòng lấy nhầm giá trị của
        // trường bên cạnh cũng bị bắt.
        let expected: Vec<(&str, &str)> = vec![
            ("room_id", "R201"),
            ("guest_name", "Hyungchul Lee"),
            ("guest_phone", "0909000111"),
            // Số giấy tờ đi thẳng vào hồ sơ khai báo tạm trú — nó phải nằm ngay
            // trên thẻ người ta đang nhìn, không phải chỉ trong payload.
            ("guest_doc_number", "M12345678"),
            ("check_in_date", "08/08/2026"),
            ("check_out_date", "09/08/2026"),
            ("nights", "1 đêm"),
            ("deposit_amount", "200.000 ₫"),
            ("source", "phone"),
            ("notes", "khách quen"),
            ("guests", "Không ghi (không thu phụ thu thêm người)"),
            // Trợ lý luôn gửi `None` (xem `build_reserve_draft`), nên "—" mới
            // là giá trị thật — cùng luật `rate_override_per_night` trên thẻ
            // nhận phòng.
            ("rate_override_per_night", "—"),
            ("total", "400.000 ₫"),
        ];
        for (key, value) in &expected {
            assert_eq!(
                display.get(*key).map(String::as_str),
                Some(*value),
                "dòng `{key}` trên thẻ sai"
            );
        }

        // Thẻ không được có dòng thừa nào ngoài danh sách trên: một khoá lạ là
        // một dòng không ai kiểm nội dung.
        assert_eq!(display.len(), expected.len(), "{display:?}");
    }

    /// Hợp đồng dây: frontend đọc `{ kind, payload, … }` rồi chuyển **thẳng**
    /// `payload` sang lệnh (`invokeWriteCommand(command, { req: payload })`).
    ///
    /// `#[serde(untagged)]` là thứ giữ cho `payload` phẳng. Gỡ nó đi thì JSON
    /// thành `{"Reserve": {…}}` và `create_reservation` nhận một object không có
    /// trường nào nó biết — mà không một test Rust nào khác nhìn thấy, vì chúng
    /// đều đọc `ActionPayload` ở dạng struct chứ không ở dạng dây.
    #[tokio::test]
    async fn the_reservation_payload_goes_on_the_wire_flat_the_way_the_command_expects() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P916",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("dữ liệu hợp lệ không được lỗi");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        let wire = serde_json::to_value(&*action).expect("thẻ phải serialize được");
        assert_eq!(wire["kind"], serde_json::json!("reserve"));

        let payload = wire["payload"]
            .as_object()
            .expect("`payload` phải là object phẳng, không có lớp bọc biến thể");
        let mut keys: Vec<&str> = payload.keys().map(String::as_str).collect();
        keys.sort_unstable();
        // Đúng bộ trường `CreateReservationRequest` — cùng bộ mà `ReservePayload`
        // khai bên `types/assistant.ts` và `ReservationSheet.tsx` gửi khi làm tay.
        //
        // `rate_override_per_night` (Task 14) CHƯA có trong `ReservePayload` —
        // đó là khoảng cách đã có sẵn từ Task 13 (`CheckInPayload` cũng chưa
        // biết `rate_override_per_night`), để dành cho UI task (16-18). Assert
        // ở đây theo dõi hình dạng thật của `CreateReservationRequest`, không
        // phải hình dạng TS; JSON thiếu key này (form cũ chưa gửi) vẫn
        // deserialize ra `None` — serde tự đặc cách cho trường `Option<T>`.
        assert_eq!(
            keys,
            [
                "check_in_date",
                "check_out_date",
                "deposit_amount",
                "guest_doc_number",
                "guest_name",
                "guest_phone",
                "guests",
                "nights",
                "notes",
                "rate_override_per_night",
                "room_id",
                "source",
            ]
        );
        assert_eq!(payload["check_in_date"], serde_json::json!("2026-06-10"));
        assert_eq!(payload["nights"], serde_json::json!(2));
        assert!(payload["guests"].is_null(), "{:?}", payload["guests"]);
    }

    /// Không có thẻ nào cho hôm nay đi đường đặt phòng trước. "Đặt phòng cho
    /// hôm nay" là nhận phòng — `create_reservation` ghi `status='booked'` và
    /// giữ chỗ, còn khách đang đứng ở quầy cần một lượt ở đang mở.
    #[tokio::test]
    async fn a_reservation_for_today_is_refused_and_points_at_the_check_in_tool() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P903",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-01", "2026-06-03"),
            "2026-06-01",
        )
        .await
        .expect("ngày hôm nay không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::WrongDateForReserve {
                requested,
                is_today,
            } => {
                assert_eq!(requested, "2026-06-01");
                assert!(is_today, "hôm nay phải được nhận ra là hôm nay");
            }
            other => panic!("mong đợi WrongDateForReserve, nhận {other:?}"),
        }
    }

    /// Chiều quá khứ: giữ chỗ ngược về một ngày đã qua thì không lượt ở nào
    /// khớp với nó, và phòng bị khoá khỏi tra phòng trống cho một kỳ đã hết.
    #[tokio::test]
    async fn a_reservation_for_a_past_date_is_refused_and_points_at_the_backfill_tool() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P904",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-05-30", "2026-05-31"),
            "2026-06-01",
        )
        .await
        .expect("ngày quá khứ không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::WrongDateForReserve {
                requested,
                is_today,
            } => {
                assert_eq!(requested, "2026-05-30");
                assert!(!is_today, "hôm qua không được coi là hôm nay");
            }
            other => panic!("mong đợi WrongDateForReserve, nhận {other:?}"),
        }
    }

    /// Ngày trả bằng hoặc trước ngày nhận: từ chối, **không** tự cộng một đêm
    /// cho đủ. Số đêm là tiền khách trả.
    #[tokio::test]
    async fn a_check_out_that_is_not_after_the_check_in_is_refused() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P905",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        for (check_in, check_out) in [("2026-06-10", "2026-06-10"), ("2026-06-10", "2026-06-09")] {
            let outcome =
                build_reserve_draft(&pool, &reserve_args(check_in, check_out), "2026-06-01")
                    .await
                    .expect("ngày trả sai không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::CheckOutNotAfterCheckIn {
                    check_in_date,
                    check_out_date,
                } => {
                    assert_eq!(check_in_date, check_in);
                    assert_eq!(check_out_date, check_out);
                }
                other => panic!(
                    "mong đợi CheckOutNotAfterCheckIn cho {check_in}→{check_out}, nhận {other:?}"
                ),
            }
        }
    }

    /// "ngày 8" là đúng chuỗi model gửi khi nó chép lại lời lễ tân. Đọc không ra
    /// thì từ chối — và lời từ chối phải gọi tên **đúng ô** hỏng, không thì model
    /// sửa nhầm ô còn lại.
    #[tokio::test]
    async fn an_unreadable_reservation_date_names_the_field_it_could_not_read() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P906",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome =
            build_reserve_draft(&pool, &reserve_args("ngày 8", "2026-06-12"), "2026-06-01")
                .await
                .expect("ngày rác không phải lỗi hệ thống");
        match outcome {
            DraftOutcome::UnreadableReserveDate { field, requested } => {
                assert_eq!(field, "check_in_date");
                assert_eq!(requested, "ngày 8");
            }
            other => panic!("mong đợi UnreadableReserveDate, nhận {other:?}"),
        }

        let outcome =
            build_reserve_draft(&pool, &reserve_args("2026-06-10", "ngày 9"), "2026-06-01")
                .await
                .expect("ngày rác không phải lỗi hệ thống");
        match outcome {
            DraftOutcome::UnreadableReserveDate { field, requested } => {
                assert_eq!(field, "check_out_date");
                assert_eq!(requested, "ngày 9");
            }
            other => panic!("mong đợi UnreadableReserveDate, nhận {other:?}"),
        }
    }

    /// Bốn trường bắt buộc, mỗi trường vắng mặt là một `MissingFields` — không
    /// mặc định, không suy ra. Model không có đường nào dựng được thẻ mà không
    /// nêu rõ hai ngày.
    #[tokio::test]
    async fn every_required_reservation_field_is_reported_when_missing() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P907",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        for field in ["room_id", "guest_name", "check_in_date", "check_out_date"] {
            let mut args = reserve_args("2026-06-10", "2026-06-12");
            args.as_object_mut().expect("args là object").remove(field);

            let outcome = build_reserve_draft(&pool, &args, "2026-06-01")
                .await
                .expect("thiếu trường không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::MissingFields(fields) => assert!(
                    fields.contains(&field.to_string()),
                    "thiếu `{field}` mà không được báo: {fields:?}"
                ),
                other => panic!("thiếu `{field}`: mong đợi MissingFields, nhận {other:?}"),
            }
        }
    }

    /// Số đêm là hiệu **hai ngày lịch**, nên khoảng vắt tháng ra đúng mà không
    /// cần biết tháng 8 có bao nhiêu ngày: 30/08 → 02/09 là 3 đêm.
    #[tokio::test]
    async fn nights_are_derived_correctly_across_a_month_boundary() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P908",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-08-30", "2026-09-02"),
            "2026-08-06",
        )
        .await
        .expect("khoảng vắt tháng là hợp lệ");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(reserve_payload(&action).nights, 3);
        assert_eq!(
            action.display.get("nights").map(String::as_str),
            Some("3 đêm")
        );
    }

    /// Preview hỏng ⇒ **không có thẻ**. Không có số mặc định nào, và tuyệt đối
    /// không có số nào do model đưa — schema `draft_reserve` cũng không có ô
    /// tiền phòng để nó đưa.
    #[tokio::test]
    async fn a_reservation_for_an_unknown_room_fails_instead_of_quoting_a_default() {
        let pool = test_pool().await;

        let mut args = reserve_args("2026-06-10", "2026-06-12");
        args["room_id"] = serde_json::json!("khong-ton-tai");

        let error = build_reserve_draft(&pool, &args, "2026-06-01")
            .await
            .expect_err("không tra được giá thì không được dựng thẻ");

        assert_eq!(error.code, codes::AGENT_PREVIEW_UNAVAILABLE);
    }

    /// Số trên thẻ phải bằng số `create_reservation` thật sẽ ghi. Phòng dưới đây
    /// có phụ thu 150.000₫/khách vượt mốc/đêm — đúng cái phòng mà `seed_room`
    /// thường **không** dựng được, nên nó là fixture duy nhất nhìn thấy được
    /// chênh lệch nếu số khách lọt vào một trong hai lời gọi giá.
    #[tokio::test]
    async fn the_reservation_card_ignores_a_guest_count_the_desk_will_not_charge() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(
            &pool,
            "room-res",
            "P909",
            "Family Room",
            500_000,
            "vacant",
        )
        .await;

        // Model gửi kèm số khách dù schema không có ô ấy — phải bị bỏ qua.
        let mut args = reserve_args("2026-06-10", "2026-06-12");
        args["guests"] = serde_json::json!(4);

        let outcome = build_reserve_draft(&pool, &args, "2026-06-01")
            .await
            .expect("dữ liệu hợp lệ không được lỗi");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(
            reserve_payload(&action).guests,
            None,
            "số khách của model lọt vào payload"
        );
        assert_eq!(
            action.preview.get("total").and_then(Value::as_i64),
            Some(1_000_000),
            "preview của thẻ đang tính phụ thu thêm người mà quầy không thu"
        );
    }

    /// Đường nối thật: lấy đúng `payload` mà thẻ mang, chạy qua chính
    /// `create_reservation` — lệnh mà nút *Đồng ý* gọi tới — rồi so tổng trên
    /// thẻ với tổng ghi vào `bookings.total_price`.
    ///
    /// Hai test rời nhau mỗi bên tự khẳng định mình đúng không bắt được ca
    /// khách nghe một con số và sổ sách ghi một con số khác.
    #[tokio::test]
    async fn the_reservation_card_total_is_the_total_create_reservation_records() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(
            &pool,
            "room-res",
            "P910",
            "Family Room",
            500_000,
            "vacant",
        )
        .await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("dữ liệu hợp lệ không được lỗi");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        let card_total = action
            .preview
            .get("total")
            .and_then(Value::as_i64)
            .expect("thẻ phải có tổng tiền");

        let booking = crate::services::booking::reservation_lifecycle::create_reservation(
            &pool,
            reserve_payload(&action).clone(),
        )
        .await
        .expect("payload của thẻ phải đặt phòng được");

        assert_eq!(
            card_total, booking.total_price,
            "thẻ báo {card_total} nhưng đặt phòng ghi {}",
            booking.total_price
        );
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some(format_vnd(booking.total_price).as_str()),
            "dòng tổng tiền lễ tân đọc cho khách phải là số sổ sách ghi"
        );
        // Ngày ghi vào PMS phải là ngày trên thẻ, không phải hôm nay. Đây là
        // đúng chỗ con bug 06/08 đã xảy ra, chỉ khác đường ghi.
        assert_eq!(booking.check_in_at, "2026-06-10");
        assert_eq!(booking.nights, 2);
    }

    /// **Cảnh báo phải hỏi đúng câu hỏi.** Một phòng đang có khách HÔM NAY mà
    /// trống trong khoảng ngày đặt thì đặt phòng đó hoàn toàn hợp lệ — cảnh báo
    /// ở đây là cảnh báo sai, và cảnh báo sai dạy lễ tân bỏ qua cảnh báo.
    ///
    /// Đây chính là chỗ dễ chép nhầm luật của thẻ nhận phòng (`build_warnings`
    /// đọc `rooms.status`/`booking_id` **ngay lúc này**) sang thẻ đặt phòng.
    #[tokio::test]
    async fn a_room_occupied_today_but_free_on_the_requested_dates_gets_no_warning() {
        let pool = test_pool().await;
        // `status = 'occupied'` và có khách đang ở: thẻ NHẬN PHÒNG sẽ kêu "Phòng
        // đang có khách ở." cho đúng phòng này.
        seed_room(
            &pool,
            "room-res",
            "P911",
            "Standard Room",
            400_000,
            "occupied",
        )
        .await;
        seed_guest(&pool, "guest-now", "Khách đang ở").await;
        seed_booking(
            &pool,
            "book-now",
            "room-res",
            "guest-now",
            "2026-06-01",
            "2026-06-03",
        )
        .await;
        seed_room_calendar_day(&pool, "room-res", "2026-06-01", "book-now").await;
        seed_room_calendar_day(&pool, "room-res", "2026-06-02", "book-now").await;

        // Đặt cho 10/06–12/06: phòng trống hẳn trong khoảng ấy.
        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("dữ liệu hợp lệ không được lỗi");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert!(
            action.warnings.is_empty(),
            "phòng trống trong khoảng ngày đặt mà vẫn bị cảnh báo: {:?}",
            action.warnings
        );
    }

    /// Chiều ngược lại, để test trên không đúng một cách vô nghĩa: bận **trong
    /// khoảng ngày đặt** thì phải có cảnh báo, và câu cảnh báo phải nêu đúng
    /// khoảng ngày ấy.
    #[tokio::test]
    async fn a_room_already_taken_inside_the_requested_dates_gets_a_warning() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P912",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;
        seed_guest(&pool, "guest-later", "Khách đặt trước").await;
        seed_booking(
            &pool,
            "book-later",
            "room-res",
            "guest-later",
            "2026-06-11",
            "2026-06-12",
        )
        .await;
        seed_room_calendar_day(&pool, "room-res", "2026-06-11", "book-later").await;

        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("phòng bận vẫn tra được giá — không phải lỗi hệ thống");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(action.warnings.len(), 1, "{:?}", action.warnings);
        let warning = &action.warnings[0];
        assert!(
            warning.contains("10/06/2026") && warning.contains("12/06/2026"),
            "{warning}"
        );
        // KHÔNG được là câu của thẻ nhận phòng: nó nói về "bây giờ", còn đây nói
        // về một khoảng ngày.
        assert!(
            !warning.contains("Phòng đang có khách ở."),
            "cảnh báo của thẻ nhận phòng bị chép sang thẻ đặt phòng: {warning}"
        );
    }

    /// Cảnh báo thiếu giấy tờ giữ nguyên từ thẻ nhận phòng — cùng một hồ sơ khai
    /// báo tạm trú, cùng một chỗ thiếu.
    #[tokio::test]
    async fn a_reservation_without_a_document_number_gets_a_warning() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P913",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let mut args = reserve_args("2026-06-10", "2026-06-12");
        args.as_object_mut()
            .expect("args là object")
            .remove("guest_doc_number");

        let outcome = build_reserve_draft(&pool, &args, "2026-06-01")
            .await
            .expect("thiếu giấy tờ không phải lỗi hệ thống");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(action.warnings.len(), 1, "{:?}", action.warnings);
        assert!(
            action.warnings[0].contains("Hyungchul Lee"),
            "{:?}",
            action.warnings
        );
        assert!(
            action.warnings[0].contains("giấy tờ"),
            "{:?}",
            action.warnings
        );
    }

    /// Nhiều khách ⇒ **nói ra**. Bỏ im lặng phần còn lại đúng là lớp lỗi cả đợt
    /// này đang sửa: một giới hạn người dùng nhìn thấy thì họ đi đường khác, một
    /// giới hạn vô hình thì hệ thống tự ý làm việc gần đúng và không nói gì.
    #[tokio::test]
    async fn more_than_one_name_in_the_guest_field_is_called_out_not_silently_dropped() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P914",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        for name in [
            "Hyungchul Lee và Trần Thị Bích",
            "Hyungchul Lee, Trần Thị Bích",
            "Hyungchul Lee & Trần Thị Bích",
        ] {
            let mut args = reserve_args("2026-06-10", "2026-06-12");
            args["guest_name"] = serde_json::json!(name);

            let outcome = build_reserve_draft(&pool, &args, "2026-06-01")
                .await
                .expect("nhiều tên không phải lỗi hệ thống");
            let action = match outcome {
                DraftOutcome::Ready(action) => action,
                other => panic!("mong đợi Ready, nhận {other:?}"),
            };

            // Thẻ vẫn dựng được — từ chối hẳn thì lễ tân không đặt được phòng —
            // nhưng nó phải NÓI RA giới hạn.
            let called_out = action
                .warnings
                .iter()
                .any(|warning| warning.contains("MỘT tên khách"));
            assert!(
                called_out,
                "«{name}» bị lặng lẽ ghi thành một khách: {:?}",
                action.warnings
            );
        }

        // Đối chứng: một tên bình thường KHÔNG được kêu. Không có dòng này thì
        // một hàm luôn trả `true` cũng làm vòng lặp trên xanh.
        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-06-12"),
            "2026-06-01",
        )
        .await
        .expect("một tên là ca thường");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        assert!(
            action.warnings.is_empty(),
            "một tên duy nhất không được sinh cảnh báo nào: {:?}",
            action.warnings
        );
    }

    /// `2026-6-1` và `2026-06-01` là cùng một ngày lịch, nhưng
    /// `create_reservation_tx` dò trùng lịch bằng so sánh **chuỗi**
    /// (`room_calendar.date >= ?`), và `'2026-06-10' < '2026-6-10'` theo thứ tự
    /// chuỗi — một dấu 0 thiếu làm phép dò trùng quét nhầm khoảng. Payload phải
    /// mang dạng đã chuẩn hoá.
    #[tokio::test]
    async fn a_date_written_without_leading_zeros_reaches_the_payload_normalised() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P915",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let outcome =
            build_reserve_draft(&pool, &reserve_args("2026-6-10", "2026-6-12"), "2026-06-01")
                .await
                .expect("cùng một ngày lịch viết thiếu số 0 vẫn phải dựng được thẻ");
        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        let payload = reserve_payload(&action);
        assert_eq!(payload.check_in_date, "2026-06-10");
        assert_eq!(payload.check_out_date, "2026-06-12");
        assert_eq!(payload.nights, 2);
    }

    // ─── Thẻ GHI BÙ (`build_backfill_draft`) ───
    //
    // "Hôm nay" của cả khối là **thứ Năm 11/06/2026**, chọn để mọi khoảng ngày
    // dưới đây chỉ gồm đêm ngày thường: uplift cuối tuần mặc định 20% phải ra 0
    // và tổng luôn là `số đêm × base_price`. Một fixture vắt qua đêm thứ Bảy sẽ
    // làm mọi con số kỳ vọng ở đây sai mà không ai hiểu vì sao.
    //
    // Mọi test đều **seed phòng thật**, kể cả những test chỉ mong một lời từ
    // chối: phòng không tồn tại thì preview hỏng và test vẫn đỏ ngay cả khi luật
    // bị gỡ sạch — tức đỏ vì fixture, không canh gì.

    const BACKFILL_TODAY: &str = "2026-06-11";

    /// Ghi bù cho một khách **đã trả phòng**: vào 08/06 (thứ Hai), ra 10/06
    /// (thứ Tư) — hai đêm ngày thường, và ngày ra không ở tương lai.
    fn backfill_args_checked_out() -> Value {
        serde_json::json!({
            "room_id": "room-bf",
            "guests": [{
                "full_name": "Trần Thị Bích",
                "doc_number": "079301005678",
                "phone": "0909000111"
            }],
            "check_in_date": "2026-06-08",
            "check_out_date": "2026-06-10"
        })
    }

    /// Ghi bù cho một khách **còn đang ở**: vào 10/06 (thứ Tư, quá khứ), dự kiến
    /// ra 12/06 (thứ Sáu, sau hôm nay) — hai đêm ngày thường.
    fn backfill_args_still_staying() -> Value {
        serde_json::json!({
            "room_id": "room-bf",
            "guests": [{
                "full_name": "Trần Thị Bích",
                "doc_number": "079301005678",
                "phone": "0909000111"
            }],
            "check_in_date": "2026-06-10",
            "expected_checkout_date": "2026-06-12"
        })
    }

    fn ready(outcome: DraftOutcome) -> Box<ProposedAction> {
        match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        }
    }

    /// Đường `Ready` của nhánh đã-trả-phòng: đúng `kind`, đúng payload, tiền của
    /// preview thật, và ngày ghi vào là ngày người dùng nêu chứ không phải hôm
    /// nay.
    #[tokio::test]
    async fn a_backfill_for_a_finished_stay_is_ready_with_the_preview_total() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-bf",
            "P920",
            "Deluxe Balcony",
            500_000,
            "vacant",
        )
        .await;

        let outcome = build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
            .await
            .expect("ngày quá khứ với phòng có thật không được lỗi");
        let action = ready(outcome);

        assert_eq!(action.kind, BACKFILL_ACTION_KIND);
        assert_ne!(
            action.kind, CHECK_IN_ACTION_KIND,
            "thẻ ghi bù đi qua đường nhận phòng là đóng dấu hôm nay lên một kỳ ở đã qua"
        );

        let payload = backfill_payload(&action);
        assert_eq!(payload.room_id, "room-bf");
        assert_eq!(payload.check_in_date, "2026-06-08");
        assert_eq!(payload.check_out_date.as_deref(), Some("2026-06-10"));
        assert_eq!(payload.expected_checkout_date, None);
        assert_eq!(payload.guests.len(), 1);
        // Hai đêm ngày thường × 500.000₫.
        assert_eq!(payload.total_price, 1_000_000);
        assert_eq!(payload.paid_amount, 0, "chưa nêu thì chưa trả đồng nào");
        assert_eq!(
            action.display.get("total_price").map(String::as_str),
            Some("1.000.000 ₫")
        );
    }

    /// Đường `Ready` của nhánh còn-ở: giá hỏi cho khoảng **tới ngày trả dự
    /// kiến**, đúng `BackfillDates.end` mà `backfill_stay` dựng để tính số đêm.
    #[tokio::test]
    async fn a_backfill_for_a_guest_still_in_the_room_prices_up_to_the_expected_checkout() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P921", "Standard Room", 400_000, "vacant").await;

        let outcome = build_backfill_draft(&pool, &backfill_args_still_staying(), BACKFILL_TODAY)
            .await
            .expect("khách còn ở là ca hợp lệ");
        let action = ready(outcome);

        let payload = backfill_payload(&action);
        assert_eq!(payload.check_in_date, "2026-06-10");
        assert_eq!(
            payload.check_out_date, None,
            "khách còn ở thì KHÔNG có ngày trả thật"
        );
        assert_eq!(
            payload.expected_checkout_date.as_deref(),
            Some("2026-06-12")
        );
        // 10/06 → 12/06 là hai đêm ngày thường × 400.000₫. Nếu preview bị hỏi
        // cho khoảng tới HÔM NAY (11/06) thì con số này là 400.000₫.
        assert_eq!(payload.total_price, 800_000);
    }

    /// ─── TEST QUAN TRỌNG NHẤT CỦA CẢ TASK ───
    ///
    /// `backfill_stay` bắt người gọi đưa tiền phòng vào, nên đây là chỗ duy nhất
    /// trong ba đường ghi mà một mô hình ngôn ngữ có thể quyết định khách nợ bao
    /// nhiêu. Model gửi kèm 1₫ trong khi bảng giá ra 400.000₫ ⇒ thẻ phải dùng
    /// 400.000₫, và con số của model không được để lại dấu vết nào.
    #[tokio::test]
    async fn a_total_price_sent_by_the_model_is_ignored_in_favour_of_the_preview() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P922", "Standard Room", 400_000, "vacant").await;

        // 09/06 (thứ Ba) → 10/06 (thứ Tư): đúng MỘT đêm ngày thường ⇒ 400.000₫.
        let mut args = backfill_args_checked_out();
        args["check_in_date"] = serde_json::json!("2026-06-09");
        args["total_price"] = serde_json::json!(1);

        let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("dữ liệu hợp lệ không được lỗi");
        let action = ready(outcome);

        let payload = backfill_payload(&action);
        assert_eq!(
            payload.total_price, 400_000,
            "số tiền của model lọt vào payload — một mô hình ngôn ngữ vừa quyết định khách nợ bao nhiêu"
        );
        assert_eq!(
            action.preview.get("total").and_then(Value::as_i64),
            Some(400_000)
        );
        assert_eq!(
            action.display.get("total_price").map(String::as_str),
            Some("400.000 ₫")
        );
        // Vế âm: con số của model không được xuất hiện ở BẤT CỨ đâu trên thẻ,
        // kể cả một dòng phụ "model đề nghị 1₫".
        let shown = action
            .display
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!shown.contains("1 ₫"), "{shown}");
    }

    /// Preview hỏng ⇒ **không có thẻ**. Không có số mặc định, và đặc biệt không
    /// rơi về con số model gửi kèm — với `backfill_stay` thì "số mặc định" ấy
    /// đúng nghĩa là một khoản nợ bịa ra.
    #[tokio::test]
    async fn a_backfill_for_an_unknown_room_fails_instead_of_quoting_a_default() {
        let pool = test_pool().await;

        let mut args = backfill_args_checked_out();
        args["room_id"] = serde_json::json!("khong-ton-tai");
        args["total_price"] = serde_json::json!(999_999);

        let error = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect_err("không tra được giá thì không được dựng thẻ");

        assert_eq!(error.code, codes::AGENT_PREVIEW_UNAVAILABLE);
    }

    /// Số khách là DANH SÁCH ở tool này, và nó tuyệt đối không được rơi xuống
    /// tham số `guests: Option<i32>` của engine giá: quầy không thu phụ thu thêm
    /// người. Phòng dưới đây có phụ thu 150.000₫/khách vượt mốc/đêm — fixture
    /// duy nhất nhìn thấy được chênh lệch.
    #[tokio::test]
    async fn the_backfill_card_ignores_the_head_count_the_desk_will_not_charge_for() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(&pool, "room-bf", "P923", "Family Room", 500_000, "vacant")
            .await;

        let mut args = backfill_args_checked_out();
        args["guests"] = serde_json::json!([
            { "full_name": "Trần Thị Bích", "doc_number": "079301005678" },
            { "full_name": "Lê Văn Cường", "doc_number": "079088007766" },
            { "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" },
            { "full_name": "Phạm Thị Dung", "doc_number": "079300112233" }
        ]);

        let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("dữ liệu hợp lệ không được lỗi");
        let action = ready(outcome);

        let payload = backfill_payload(&action);
        assert_eq!(payload.guests.len(), 4, "danh sách khách phải vào payload");
        assert_eq!(
            payload.total_price, 1_000_000,
            "thẻ đang tính phụ thu thêm người mà quầy không thu"
        );
    }

    /// Đường nối thật: lấy đúng `payload` mà thẻ mang, chạy nó qua chính lệnh
    /// `backfill_stay` — lệnh mà nút *Đồng ý* gọi tới — rồi so tổng trên thẻ với
    /// tổng ghi vào `bookings.total_price`.
    ///
    /// Ngày phải tính từ **hôm nay thật**: `backfill_stay_tx` đọc
    /// `Local::now().date_naive()` bên trong, nó không nhận `now_local_date` như
    /// hàm dựng thẻ. Nên tổng kỳ vọng cũng không viết cứng được — test so **hai
    /// vế với nhau**, đó mới là thứ cần canh.
    #[tokio::test]
    async fn the_backfill_card_total_is_the_total_backfill_stay_records() {
        use crate::command_idempotency::WriteCommandContext;

        let pool = test_pool().await;
        seed_room_charging_extra_guests(&pool, "room-bf", "P924", "Family Room", 500_000, "vacant")
            .await;

        let today = chrono::Local::now().date_naive();
        let day = |offset: i64| {
            (today + chrono::Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string()
        };

        let mut args = backfill_args_checked_out();
        args["check_in_date"] = serde_json::json!(day(-3));
        args["check_out_date"] = serde_json::json!(day(-1));

        let outcome = build_backfill_draft(&pool, &args, &today.format("%Y-%m-%d").to_string())
            .await
            .expect("dữ liệu hợp lệ không được lỗi");
        let action = ready(outcome);
        let card_total = backfill_payload(&action).total_price;

        let context =
            WriteCommandContext::for_internal_test("req-bf-card", "idem-bf-card", "backfill_stay");
        let result = crate::services::booking::backfill::backfill_stay_idempotent(
            &pool,
            &context,
            backfill_payload(&action).clone(),
            None,
        )
        .await
        .expect("payload của thẻ phải ghi bù được");
        let booking: crate::models::Booking =
            serde_json::from_value(result.response).expect("lệnh trả về một booking");

        assert_eq!(
            card_total, booking.total_price,
            "thẻ báo {card_total} nhưng sổ ghi {}",
            booking.total_price
        );
        assert_eq!(
            action.display.get("total_price").map(String::as_str),
            Some(format_vnd(booking.total_price).as_str()),
            "dòng tiền lễ tân đọc cho khách phải là số sổ sách ghi"
        );
        // Ngày ghi vào PMS là ngày trên thẻ, không phải hôm nay — đúng chỗ con
        // bug 06/08 đã xảy ra, chỉ khác đường ghi.
        assert!(
            booking.check_in_at.starts_with(&day(-3)),
            "PMS ghi {} thay vì {}",
            booking.check_in_at,
            day(-3)
        );
        assert_eq!(booking.nights, 2);
    }

    /// Ngày vào **hôm nay** không phải ghi bù — khách đang đứng ở quầy.
    ///
    /// Thử **cả hai** hình dạng của thẻ, và đây không phải cho đủ bộ. Đo được:
    /// gỡ nhánh từ chối này ra thì bản **đã trả phòng** vẫn đỏ, nhưng đỏ vì một
    /// vế KHÁC bắt hộ (`BackfillCheckOutInTheFuture`), trong khi bản **còn ở**
    /// dựng ra được một cái thẻ ghi bù mang ngày vào là hôm nay — đúng con bug
    /// này, không vế nào bắt hộ. Nhánh còn-ở đứng trước trong vòng lặp để dòng
    /// đỏ đầu tiên đọc lên là bằng chứng đúng lý do, không phải bằng chứng vay
    /// của một luật khác.
    #[tokio::test]
    async fn a_backfill_for_today_is_refused_and_points_at_the_check_in_tool() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P925", "Standard Room", 400_000, "vacant").await;

        let mut checked_out = backfill_args_checked_out();
        checked_out["check_in_date"] = serde_json::json!(BACKFILL_TODAY);
        checked_out["check_out_date"] = serde_json::json!("2026-06-13");

        let mut still_staying = backfill_args_still_staying();
        still_staying["check_in_date"] = serde_json::json!(BACKFILL_TODAY);

        for (shape, args) in [
            ("khách còn ở", still_staying),
            ("khách đã trả phòng", checked_out),
        ] {
            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("ngày hôm nay không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::WrongDateForBackfill {
                    requested,
                    is_today,
                } => {
                    assert_eq!(requested, BACKFILL_TODAY);
                    assert!(is_today, "hôm nay phải được nhận ra là hôm nay");
                }
                other => panic!("{shape}: mong đợi WrongDateForBackfill, nhận {other:?}"),
            }
        }
    }

    /// Ngày vào ở **tương lai** là đặt phòng trước, không phải ghi bù: ghi lại
    /// một kỳ ở chưa xảy ra thì phòng bị khoá cho một khách chưa tới.
    ///
    /// Cả hai hình dạng thẻ, cùng lý do như test ngay trên.
    #[tokio::test]
    async fn a_backfill_for_a_future_date_is_refused_and_points_at_the_reservation_tool() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P926", "Standard Room", 400_000, "vacant").await;

        let mut checked_out = backfill_args_checked_out();
        checked_out["check_in_date"] = serde_json::json!("2026-06-15");
        checked_out["check_out_date"] = serde_json::json!("2026-06-17");

        let mut still_staying = backfill_args_still_staying();
        still_staying["check_in_date"] = serde_json::json!("2026-06-15");
        still_staying["expected_checkout_date"] = serde_json::json!("2026-06-17");

        for (shape, args) in [
            ("khách còn ở", still_staying),
            ("khách đã trả phòng", checked_out),
        ] {
            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("ngày tương lai không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::WrongDateForBackfill {
                    requested,
                    is_today,
                } => {
                    assert_eq!(requested, "2026-06-15");
                    assert!(!is_today, "ngày mai không được coi là hôm nay");
                }
                other => panic!("{shape}: mong đợi WrongDateForBackfill, nhận {other:?}"),
            }
        }
    }

    /// Khách còn ở mà thiếu ngày trả dự kiến ⇒ `MissingFields`. Không mặc định
    /// một ngày nào: `backfill_stay` sẽ nổ "Thiếu ngày ra dự kiến cho khách còn
    /// ở" **sau** khi lễ tân đã bấm *Đồng ý*, và một ngày đoán hộ ở đây là đoán
    /// hộ số đêm, tức đoán hộ một khoản tiền.
    #[tokio::test]
    async fn a_still_staying_backfill_without_an_expected_checkout_reports_the_missing_field() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P927", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_still_staying();
        args.as_object_mut()
            .expect("args là object")
            .remove("expected_checkout_date");

        let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => assert!(
                fields.contains(&"expected_checkout_date".to_string()),
                "{fields:?}"
            ),
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }

        // Đối chứng: khách ĐÃ trả phòng thì ô ấy không bắt buộc, và một thẻ vẫn
        // dựng được. Không có vế này thì một luật "luôn bắt buộc" cũng xanh, và
        // nó sẽ chặn đúng nửa số ca hợp lệ.
        let outcome = build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
            .await
            .expect("khách đã trả phòng là ca hợp lệ");
        assert_eq!(
            backfill_payload(&ready(outcome)).expected_checkout_date,
            None
        );
    }

    /// Khách còn ở mà ngày trả dự kiến không **sau** hôm nay ⇒ từ chối. Hôm nay
    /// và hôm qua đều sai, và cả hai đều là lời từ chối của `backfill_stay`.
    #[tokio::test]
    async fn an_expected_checkout_that_is_not_after_today_is_refused() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P928", "Standard Room", 400_000, "vacant").await;

        for requested_date in [BACKFILL_TODAY, "2026-06-10"] {
            let mut args = backfill_args_still_staying();
            args["expected_checkout_date"] = serde_json::json!(requested_date);
            // Ngày vào phải trước ngày trả dự kiến để test không đỏ vì một vế
            // khác.
            args["check_in_date"] = serde_json::json!("2026-06-09");

            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("ngày trả dự kiến sai không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::ExpectedCheckoutNotAfterToday { requested, today } => {
                    assert_eq!(requested, requested_date);
                    assert_eq!(today, BACKFILL_TODAY);
                }
                other => panic!(
                    "`{requested_date}`: mong đợi ExpectedCheckoutNotAfterToday, nhận {other:?}"
                ),
            }
        }
    }

    /// Ô `check_out_date` nghĩa là khách ĐÃ rời phòng, nên một ngày ở tương lai
    /// mâu thuẫn với chính nó. Từ chối và chỉ sang ô đúng — **không** tự chuyển
    /// giá trị sang `expected_checkout_date`, vì chuyển hộ là tự quyết rằng
    /// khách vẫn còn nằm trong phòng.
    #[tokio::test]
    async fn a_backfill_check_out_in_the_future_is_refused_and_names_the_other_slot() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P929", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["check_out_date"] = serde_json::json!("2026-06-13");

        let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("ngày trả ở tương lai không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::BackfillCheckOutInTheFuture { requested, today } => {
                assert_eq!(requested, "2026-06-13");
                assert_eq!(today, BACKFILL_TODAY);
            }
            other => panic!("mong đợi BackfillCheckOutInTheFuture, nhận {other:?}"),
        }
    }

    /// Ngày trả không sau ngày vào: từ chối, không tự cộng một đêm cho đủ.
    #[tokio::test]
    async fn a_backfill_check_out_that_is_not_after_the_check_in_is_refused() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P930", "Standard Room", 400_000, "vacant").await;

        for (check_in, check_out) in [("2026-06-08", "2026-06-08"), ("2026-06-08", "2026-06-07")] {
            let mut args = backfill_args_checked_out();
            args["check_in_date"] = serde_json::json!(check_in);
            args["check_out_date"] = serde_json::json!(check_out);

            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("ngày trả sai không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::CheckOutNotAfterCheckIn {
                    check_in_date,
                    check_out_date,
                } => {
                    assert_eq!(check_in_date, check_in);
                    assert_eq!(check_out_date, check_out);
                }
                other => panic!(
                    "mong đợi CheckOutNotAfterCheckIn cho {check_in}→{check_out}, nhận {other:?}"
                ),
            }
        }
    }

    /// "ngày 4" là đúng chuỗi model gửi khi nó chép lại lời lễ tân. Thẻ này có
    /// **ba** ô ngày, nên lời từ chối phải gọi tên đúng ô — không thì model sửa
    /// nhầm ô còn lại.
    #[tokio::test]
    async fn an_unreadable_backfill_date_names_the_field_it_could_not_read() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P931", "Standard Room", 400_000, "vacant").await;

        for (field, mut args) in [
            ("check_in_date", backfill_args_checked_out()),
            ("check_out_date", backfill_args_checked_out()),
            ("expected_checkout_date", backfill_args_still_staying()),
        ] {
            args[field] = serde_json::json!("ngày 4");

            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("ngày rác không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::UnreadableBackfillDate {
                    field: named,
                    requested,
                } => {
                    assert_eq!(named, field);
                    assert_eq!(requested, "ngày 4");
                }
                other => panic!("`{field}`: mong đợi UnreadableBackfillDate, nhận {other:?}"),
            }
        }
    }

    /// Ba trường bắt buộc vô điều kiện, mỗi trường vắng mặt là một
    /// `MissingFields` — không mặc định, không suy ra.
    #[tokio::test]
    async fn every_required_backfill_field_is_reported_when_missing() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P932", "Standard Room", 400_000, "vacant").await;

        for field in ["room_id", "guests", "check_in_date"] {
            let mut args = backfill_args_checked_out();
            args.as_object_mut().expect("args là object").remove(field);

            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("thiếu trường không phải lỗi hệ thống");

            match outcome {
                DraftOutcome::MissingFields(fields) => assert!(
                    fields.contains(&field.to_string()),
                    "thiếu `{field}` mà không được báo: {fields:?}"
                ),
                other => panic!("thiếu `{field}`: mong đợi MissingFields, nhận {other:?}"),
            }
        }
    }

    /// Luật số một của thiết kế, bản cho thẻ ghi bù: người dùng duyệt đúng cái
    /// sẽ được gửi. Thêm trường vào `BackfillStayRequest` mà quên hiện lên thẻ
    /// là test này đỏ.
    #[test]
    fn the_backfill_card_shows_every_field_of_the_payload() {
        let payload = BackfillStayRequest {
            room_id: "R201".to_string(),
            guests: vec![sample_guest("Trần Thị Bích"), second_sample_guest()],
            check_in_date: "2026-08-04".to_string(),
            check_out_date: Some("2026-08-06".to_string()),
            expected_checkout_date: None,
            total_price: 600_000,
            paid_amount: 200_000,
            source: Some("walk-in".to_string()),
            notes: Some("khách quen".to_string()),
        };

        let display = build_backfill_display(&payload);

        let encoded = serde_json::to_value(&payload).expect("payload phải serialize được");
        let fields = encoded.as_object().expect("payload là một object");

        // Vế một: **không bỏ sót trường nào của payload**.
        for key in fields.keys() {
            assert!(
                display.contains_key(key),
                "trường `{key}` của payload không hiện trên thẻ xác nhận: {display:?}"
            );
        }

        // Vế hai: mỗi lá của `guests` (số CCCD, số điện thoại, địa chỉ…) phải
        // hiện nguyên văn — nó đi thẳng vào hồ sơ khai báo tạm trú.
        let shown = display
            .values()
            .cloned()
            .collect::<Vec<String>>()
            .join("\n");
        assert_nested_leaves_are_on_the_card("guests", &encoded["guests"], &shown);

        // Vế ba: mỗi dòng nói đúng giá trị của nó — thẻ này định dạng lại ngày
        // và tiền, nên phép dò nguyên văn sẽ bắt nhầm đúng những dòng làm đúng.
        for (key, value) in [
            ("room_id", "R201"),
            ("check_in_date", "04/08/2026"),
            ("check_out_date", "06/08/2026"),
            ("expected_checkout_date", "—"),
            ("guests", "2 người"),
            ("total_price", "600.000 ₫"),
            ("paid_amount", "200.000 ₫"),
            ("source", "walk-in"),
            ("notes", "khách quen"),
        ] {
            assert_eq!(
                display.get(key).map(String::as_str),
                Some(value),
                "dòng `{key}` trên thẻ sai"
            );
        }

        // Không có dòng `total` thứ hai bên cạnh `total_price`: hai dòng tiền in
        // cùng một số là một thẻ người ta ngừng đọc kỹ.
        assert!(!display.contains_key("total"), "{display:?}");
        // Chín trường payload + hai dòng khách, không dòng thừa nào.
        assert_eq!(
            display.len(),
            fields.len() + payload.guests.len(),
            "{display:?}"
        );
    }

    /// Bất biến mới của cả đợt, bản khó nhất: thẻ ghi bù có thể **không có** ngày
    /// trả (khách còn ở), mà dòng ngày trả vẫn phải có mặt. Nó nói ra nghĩa thật
    /// bằng chữ, không phải một gạch ngang câm mà lễ tân đọc thành "chưa điền".
    #[tokio::test]
    async fn the_backfill_card_always_shows_both_stay_dates() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P933", "Standard Room", 400_000, "vacant").await;

        let action = ready(
            build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
                .await
                .expect("khách đã trả phòng là ca hợp lệ"),
        );
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("08/06/2026")
        );
        assert_eq!(
            action.display.get("check_out_date").map(String::as_str),
            Some("10/06/2026")
        );
        // Nhãn "Hôm nay" là của thẻ nhận phòng; trên thẻ ghi bù nó luôn sai.
        assert!(
            !action
                .display
                .values()
                .any(|value| value.contains("Hôm nay")),
            "{:?}",
            action.display
        );

        let action = ready(
            build_backfill_draft(&pool, &backfill_args_still_staying(), BACKFILL_TODAY)
                .await
                .expect("khách còn ở là ca hợp lệ"),
        );
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("10/06/2026")
        );
        assert_eq!(
            action.display.get("check_out_date").map(String::as_str),
            Some("Chưa trả phòng (khách còn ở)"),
            "dòng ngày trả biến mất hoặc câm trên thẻ khách-còn-ở"
        );
        assert_eq!(
            action
                .display
                .get("expected_checkout_date")
                .map(String::as_str),
            Some("12/06/2026")
        );
    }

    /// Hợp đồng dây: frontend đọc `{ kind, payload, … }` rồi chuyển **thẳng**
    /// `payload` sang `backfill_stay` (`invokeWriteCommand(command, { req: payload })`).
    ///
    /// `#[serde(untagged)]` là thứ giữ cho `payload` phẳng. Và test này là chỗ
    /// duy nhất bắt được việc `Backfill` bị một biến thể khác nuốt: `CheckIn` có
    /// cùng `room_id` + `guests`, nên bộ khoá dưới đây — có `total_price`,
    /// không có `nights`/`pricing_type` — là thứ phân biệt hai hình dạng ấy.
    #[tokio::test]
    async fn the_backfill_payload_goes_on_the_wire_flat_the_way_the_command_expects() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P934", "Standard Room", 400_000, "vacant").await;

        let action = ready(
            build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
                .await
                .expect("dữ liệu hợp lệ không được lỗi"),
        );

        let wire = serde_json::to_value(&*action).expect("thẻ phải serialize được");
        assert_eq!(wire["kind"], serde_json::json!("backfill"));

        let payload = wire["payload"]
            .as_object()
            .expect("`payload` phải là object phẳng, không có lớp bọc biến thể");
        let mut keys: Vec<&str> = payload.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "check_in_date",
                "check_out_date",
                "expected_checkout_date",
                "guests",
                "notes",
                "paid_amount",
                "room_id",
                "source",
                "total_price",
            ]
        );
        // Hình dạng này KHÔNG được lẫn với `CheckInRequest` (có `nights` và
        // `pricing_type`, không có `total_price`) hay `CreateReservationRequest`
        // (có `guest_name`).
        assert!(payload.get("nights").is_none(), "{payload:?}");
        assert!(payload.get("pricing_type").is_none(), "{payload:?}");
        assert!(payload.get("guest_name").is_none(), "{payload:?}");
        // Hai đêm ngày thường × 400.000₫ — số của preview, đi ra dây nguyên vẹn.
        assert_eq!(payload["total_price"], serde_json::json!(800_000));
        assert_eq!(payload["check_in_date"], serde_json::json!("2026-06-08"));
    }

    /// `2026-6-8` và `2026-06-08` là cùng một ngày lịch, nhưng `backfill_stay`
    /// dò trùng lịch bằng so sánh **chuỗi** (`rc.date >= ? AND rc.date < ?`) y
    /// như `create_reservation_tx` — một dấu 0 thiếu làm phép dò quét nhầm
    /// khoảng. Payload phải mang dạng đã chuẩn hoá.
    ///
    /// **Cả ba** ô ngày, không phải hai. `expected_checkout_date` từng được
    /// chuẩn hoá mà không có ai canh: thay nó bằng nguyên văn chuỗi model gửi
    /// thì không test nào đỏ. Hậu quả nằm ở
    /// `backfill::build_backfill_hash_payload` — nó nhét chuỗi ấy vào khoá
    /// idempotency, nên `2026-6-20` và `2026-06-20` thành hai khoá khác nhau và
    /// **cùng một lượt ghi bù gửi hai lần sẽ không khử trùng**: hai lượt ở, hai
    /// lần tính tiền, cho một khách.
    #[tokio::test]
    async fn a_backfill_date_written_without_leading_zeros_reaches_the_payload_normalised() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P935", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["check_in_date"] = serde_json::json!("2026-6-8");
        args["check_out_date"] = serde_json::json!("2026-6-10");

        let action = ready(
            build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("cùng một ngày lịch viết thiếu số 0 vẫn phải dựng được thẻ"),
        );

        let payload = backfill_payload(&action);
        assert_eq!(payload.check_in_date, "2026-06-08");
        assert_eq!(payload.check_out_date.as_deref(), Some("2026-06-10"));

        // Ô thứ ba, ở nhánh duy nhất nó được điền: khách **còn ở**.
        let mut args = backfill_args_still_staying();
        args["check_in_date"] = serde_json::json!("2026-6-10");
        args["expected_checkout_date"] = serde_json::json!("2026-6-12");

        let action = ready(
            build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("cùng một ngày lịch viết thiếu số 0 vẫn phải dựng được thẻ"),
        );

        let payload = backfill_payload(&action);
        assert_eq!(payload.check_in_date, "2026-06-10");
        assert_eq!(
            payload.expected_checkout_date.as_deref(),
            Some("2026-06-12"),
            "ngày trả dự kiến đi vào khoá idempotency — nguyên văn `2026-6-12` \
             làm hai lần gửi cùng một lượt ghi bù không khử trùng được"
        );
    }

    /// Cảnh báo của thẻ ghi bù chỉ được nói những câu `backfill_stay` sẽ nói khi
    /// từ chối. Ca thường — phòng trống, không trùng lịch, đã thu ít hơn tiền
    /// phòng — phải **im lặng**, không thì lễ tân học cách bỏ qua cảnh báo.
    #[tokio::test]
    async fn a_clean_backfill_carries_no_warning() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P936", "Standard Room", 400_000, "vacant").await;

        let action = ready(
            build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
                .await
                .expect("ca thường không được lỗi"),
        );

        assert!(action.warnings.is_empty(), "{:?}", action.warnings);
    }

    /// Khách **còn ở** thì `backfill_stay` đòi phòng đang trống ngay lúc này —
    /// nó sắp bật phòng sang "có khách". Phòng đang bận ⇒ lệnh từ chối, nên thẻ
    /// phải nói trước.
    #[tokio::test]
    async fn a_still_staying_backfill_into_an_occupied_room_is_warned_about() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-bf",
            "P937",
            "Standard Room",
            400_000,
            "occupied",
        )
        .await;

        let action = ready(
            build_backfill_draft(&pool, &backfill_args_still_staying(), BACKFILL_TODAY)
                .await
                .expect("phòng bận vẫn tra được giá"),
        );

        assert!(
            action
                .warnings
                .iter()
                .any(|warning| warning.contains("phòng đang ở trạng thái")),
            "{:?}",
            action.warnings
        );

        // Đối chứng: cùng phòng bận ấy, nhưng khách **đã trả phòng** thì trạng
        // thái phòng bây giờ không liên quan gì — lệnh không kiểm, nên thẻ không
        // được kêu.
        let action = ready(
            build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
                .await
                .expect("kỳ ở đã kết thúc không quan tâm phòng bây giờ có ai"),
        );
        assert!(
            !action
                .warnings
                .iter()
                .any(|warning| warning.contains("phòng đang ở trạng thái")),
            "cảnh báo của nhánh còn-ở bị bắn sang một kỳ ở đã kết thúc: {:?}",
            action.warnings
        );
    }

    /// Đã thu nhiều hơn tiền phòng: `validate_backfill_request` từ chối thẳng.
    /// Thẻ nói ra và **không** tự kẹp số về cho vừa — số tiền đã thu là một sự
    /// kiện, không phải một ô để làm tròn.
    #[tokio::test]
    async fn a_paid_amount_above_the_room_charge_is_warned_about_not_clamped() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P938", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["paid_amount"] = serde_json::json!(5_000_000);

        let action = ready(
            build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("số tiền lệch không phải lỗi hệ thống"),
        );

        assert_eq!(
            backfill_payload(&action).paid_amount,
            5_000_000,
            "số đã thu bị sửa lại cho vừa tiền phòng"
        );
        assert!(
            action
                .warnings
                .iter()
                .any(|warning| warning.contains("không được vượt tiền phòng")),
            "{:?}",
            action.warnings
        );
    }

    /// Phòng đã có lượt ở khác trong khoảng ngày ghi bù ⇒ `backfill_stay` từ
    /// chối vì trùng lịch. Câu cảnh báo phải nêu đúng khoảng ngày ấy.
    #[tokio::test]
    async fn a_room_already_taken_inside_the_backfilled_dates_gets_a_warning() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P939", "Standard Room", 400_000, "vacant").await;
        seed_guest(&pool, "guest-old", "Khách cũ").await;
        seed_booking(
            &pool,
            "book-old",
            "room-bf",
            "guest-old",
            "2026-06-08",
            "2026-06-09",
        )
        .await;
        seed_room_calendar_day(&pool, "room-bf", "2026-06-08", "book-old").await;

        let action = ready(
            build_backfill_draft(&pool, &backfill_args_checked_out(), BACKFILL_TODAY)
                .await
                .expect("phòng bận vẫn tra được giá"),
        );

        let overlap = action
            .warnings
            .iter()
            .find(|warning| warning.contains("trùng lịch"))
            .unwrap_or_else(|| panic!("thiếu cảnh báo trùng lịch: {:?}", action.warnings));
        assert!(
            overlap.contains("08/06/2026") && overlap.contains("10/06/2026"),
            "{overlap}"
        );
    }

    // ─── Ô ngày KHÔNG PHẢI CHUỖI ───
    //
    // Con bug 06/08 sống lại qua cửa sau: `Value::as_str()` trả `None` cho mọi
    // thứ không phải chuỗi JSON, và `None` ở chỗ đọc `check_in_date` nghĩa là
    // "người dùng không nêu ngày nào" — tức bỏ trọn khối soi ngày rồi đóng dấu
    // hôm nay lên thẻ. Model nghe "ngày 8" và điền `8` (số) là hình dạng model
    // thật vẫn gửi.
    //
    // Mọi test dưới đây seed phòng thật, cùng lý do đã ghi ở khối trên: phòng
    // không tồn tại thì preview hỏng và test đỏ vì fixture chứ không canh gì.

    /// Bảng các hình dạng sai, chạy chung một khẳng định: **có mặt mà không đọc
    /// được ⇒ từ chối**, không phải một cái thẻ mang ngày hôm nay.
    ///
    /// Soi cả `Debug` của outcome: hôm nay không được xuất hiện dưới bất kỳ
    /// dạng nào (`2026-06-01` của payload/preview hay `01/06/2026` của thẻ).
    /// Chỉ khẳng định "khác `Ready`" thì một biến thể từ chối nào đó vẫn có thể
    /// cõng ngày hôm nay đi tiếp mà không ai thấy.
    #[tokio::test]
    async fn a_check_in_date_that_is_not_text_is_refused_instead_of_counting_as_absent() {
        for (ten, gia_tri) in [
            ("số", serde_json::json!(8)),
            ("object", serde_json::json!({ "day": 8 })),
            ("mảng", serde_json::json!(["2026-06-08"])),
            ("số thực", serde_json::json!(8.0)),
            ("bool", serde_json::json!(true)),
            ("chuỗi toàn khoảng trắng", serde_json::json!("   ")),
        ] {
            let pool = test_pool().await;
            seed_room(
                &pool,
                "room-typed",
                "P810",
                "Standard Room",
                400_000,
                "vacant",
            )
            .await;

            let args = serde_json::json!({
                "room_id": "room-typed",
                "nights": 1,
                "check_in_date": gia_tri,
                "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
            });

            let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
                .await
                .unwrap_or_else(|error| {
                    panic!("{ten}: kiểu sai không phải lỗi hệ thống: {error:?}")
                });

            let dump = format!("{outcome:?}");
            assert!(
                !dump.contains("2026-06-01") && !dump.contains("01/06/2026"),
                "{ten}: ô ngày bị bỏ qua và hôm nay chui vào kết quả:\n{dump}"
            );
            match outcome {
                DraftOutcome::UnreadableCheckInDate { .. } => {}
                other => panic!("{ten}: mong đợi UnreadableCheckInDate, nhận {other:?}"),
            }
        }
    }

    /// `null` là "không có giá trị", không phải "có mà không đọc được": model
    /// nào cũng điền `null` cho một ô tuỳ chọn nó bỏ trống. Xử như rác thì mọi
    /// lượt nhận phòng hôm nay tốn thêm một vòng trong ngân sách bốn vòng.
    ///
    /// Ghim lại quyết định ấy: `null` đi cùng đường với **vắng mặt** — thẻ hôm
    /// nay, y hệt `a_draft_without_a_date_still_builds_the_card_the_old_way`.
    #[tokio::test]
    async fn a_null_check_in_date_counts_as_absent_and_still_builds_todays_card() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-null",
            "P811",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-null",
            "nights": 1,
            "check_in_date": serde_json::Value::Null,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let action = ready(
            build_check_in_draft(&pool, &args, "2026-06-01")
                .await
                .expect("`null` không phải lỗi hệ thống"),
        );
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 01/06/2026")
        );
    }

    /// Cùng lỗ hổng kiểu dữ liệu, hai tool còn lại. `trimmed_arg` cũng đứng trên
    /// `as_str()`, nên một ô ngày không phải chuỗi rơi vào `MissingFields` —
    /// model **có** thấy, nhưng nó được bảo "thiếu ngày" trong khi nó vừa gửi
    /// một ngày, và lời bảo ấy mời nó gửi lại đúng hình dạng cũ. Phải gọi đúng
    /// tên: đọc không được.
    #[tokio::test]
    async fn a_reserve_date_that_is_not_text_names_the_field_instead_of_calling_it_missing() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P812",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let mut args = reserve_args("2026-06-10", "2026-06-12");
        args["check_in_date"] = serde_json::json!(8);

        let outcome = build_reserve_draft(&pool, &args, "2026-06-01")
            .await
            .expect("kiểu sai không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::UnreadableReserveDate { field, .. } => {
                assert_eq!(field, "check_in_date");
            }
            other => panic!("mong đợi UnreadableReserveDate, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_backfill_date_that_is_not_text_names_the_field_instead_of_calling_it_missing() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P813", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["check_out_date"] = serde_json::json!({ "day": 10 });

        let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("kiểu sai không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::UnreadableBackfillDate { field, .. } => {
                assert_eq!(field, "check_out_date");
            }
            other => panic!("mong đợi UnreadableBackfillDate, nhận {other:?}"),
        }
    }

    // ─── Ô TIỀN không đọc được ───
    //
    // Ảnh soi gương của sự cố 06/08: lần đó **tạo** một khoản thu không có thật,
    // đây **xoá** một khoản thu có thật. `and_then(Value::as_i64)` trả `None`
    // cho `400000.0` và cho `"400000"`, rồi `.unwrap_or(0)` biến `None` thành
    // "chưa trả đồng nào" — khách đưa tiền ở quầy xong vẫn gánh nguyên khoản nợ,
    // và trên thẻ không có một chữ nào nói ra chuyện đó.

    /// Ô tiền có mặt mà không đọc được thành số nguyên đồng ⇒ **không dựng thẻ**.
    /// Không có nhánh nào được im lặng về 0.
    #[tokio::test]
    async fn a_money_field_that_is_not_a_whole_number_is_refused_not_silently_zero() {
        for (ten, gia_tri) in [
            ("chuỗi", serde_json::json!("400000")),
            ("chuỗi có dấu chấm", serde_json::json!("400.000")),
            ("số thực lẻ", serde_json::json!(400_000.5)),
            ("object", serde_json::json!({ "vnd": 400_000 })),
            ("mảng", serde_json::json!([400_000])),
            ("bool", serde_json::json!(true)),
        ] {
            let pool = test_pool().await;
            seed_room(&pool, "room-bf", "P820", "Standard Room", 400_000, "vacant").await;

            let mut args = backfill_args_checked_out();
            args["paid_amount"] = gia_tri.clone();

            let outcome = build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .unwrap_or_else(|error| {
                    panic!("{ten}: kiểu sai không phải lỗi hệ thống: {error:?}")
                });

            match outcome {
                DraftOutcome::UnreadableAmount { field, .. } => {
                    assert_eq!(field, "paid_amount", "{ten}");
                }
                DraftOutcome::Ready(action) => panic!(
                    "{ten}: tiền bị nuốt về 0 và thẻ vẫn dựng: {:?} / {:?}",
                    action.display, action.warnings
                ),
                other => panic!("{ten}: mong đợi UnreadableAmount, nhận {other:?}"),
            }
        }
    }

    /// Số nguyên gửi dạng `400000.0` **được** nhận, đúng giá trị. Model hay gửi
    /// tiền dạng số thực; từ chối cả ca này là bắt lễ tân gõ tay một khoản đã
    /// đúng. Nhận là quyết định có chủ ý, nên nó có test riêng — và test này
    /// cũng là chỗ báo động nếu ai đó siết lại.
    #[tokio::test]
    async fn a_whole_number_sent_as_a_float_reaches_the_payload_at_its_exact_value() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P821", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["paid_amount"] = serde_json::json!(400_000.0);

        let action = ready(
            build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("400000.0 là một số tiền hợp lệ"),
        );
        assert_eq!(backfill_payload(&action).paid_amount, 400_000);
        assert_eq!(
            action.display.get("paid_amount").map(String::as_str),
            Some("400.000 ₫")
        );
    }

    #[tokio::test]
    async fn a_check_in_paid_amount_that_is_not_a_number_is_refused_not_silently_zero() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-paid",
            "P822",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-paid",
            "nights": 1,
            "paid_amount": "400000",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("kiểu sai không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::UnreadableAmount { field, .. } => assert_eq!(field, "paid_amount"),
            DraftOutcome::Ready(action) => panic!(
                "tiền bị nuốt mất và thẻ vẫn dựng: {:?}",
                check_in_payload(&action).paid_amount
            ),
            other => panic!("mong đợi UnreadableAmount, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_reserve_deposit_that_is_not_a_number_is_refused_not_silently_dropped() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P823",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let mut args = reserve_args("2026-06-10", "2026-06-12");
        args["deposit_amount"] = serde_json::json!("200000");

        let outcome = build_reserve_draft(&pool, &args, "2026-06-01")
            .await
            .expect("kiểu sai không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::UnreadableAmount { field, .. } => assert_eq!(field, "deposit_amount"),
            DraftOutcome::Ready(action) => panic!(
                "cọc bị nuốt mất và thẻ vẫn dựng: {:?}",
                reserve_payload(&action).deposit_amount
            ),
            other => panic!("mong đợi UnreadableAmount, nhận {other:?}"),
        }
    }

    /// `null` ở ô tiền = vắng mặt, cùng luật với ô ngày.
    #[tokio::test]
    async fn a_null_money_field_counts_as_absent() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P824", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["paid_amount"] = serde_json::Value::Null;

        let action = ready(
            build_backfill_draft(&pool, &args, BACKFILL_TODAY)
                .await
                .expect("`null` không phải lỗi hệ thống"),
        );
        assert_eq!(backfill_payload(&action).paid_amount, 0);
    }

    // ─── TIỀN ÂM ───
    //
    // `minimum: 0` trong JSON schema không phải hàng rào — không tầng nào kiểm
    // lại nó. Đo được: `paid_amount = -500000` dựng ra thẻ ghi "-500.000 ₫",
    // `warnings []`, rồi lệnh từ chối **sau** khi lễ tân đã bấm Đồng ý. Cảnh báo
    // "đã thu quá tiền phòng" không bắt được vì số âm luôn nhỏ hơn tổng.

    #[tokio::test]
    async fn a_negative_amount_never_becomes_a_card() {
        let pool = test_pool().await;

        // Ghi bù.
        seed_room(&pool, "room-bf", "P830", "Standard Room", 400_000, "vacant").await;
        let mut args = backfill_args_checked_out();
        args["paid_amount"] = serde_json::json!(-500_000);
        match build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("số âm không phải lỗi hệ thống")
        {
            DraftOutcome::NegativeAmount { field, .. } => assert_eq!(field, "paid_amount"),
            other => panic!("ghi bù: mong đợi NegativeAmount, nhận {other:?}"),
        }

        // Nhận phòng.
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-neg",
            "P831",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;
        let args = serde_json::json!({
            "room_id": "room-neg",
            "nights": 1,
            "paid_amount": -500_000,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });
        match build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("số âm không phải lỗi hệ thống")
        {
            DraftOutcome::NegativeAmount { field, .. } => assert_eq!(field, "paid_amount"),
            other => panic!("nhận phòng: mong đợi NegativeAmount, nhận {other:?}"),
        }

        // Đặt phòng trước.
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P832",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;
        let mut args = reserve_args("2026-06-10", "2026-06-12");
        args["deposit_amount"] = serde_json::json!(-500_000);
        match build_reserve_draft(&pool, &args, "2026-06-01")
            .await
            .expect("số âm không phải lỗi hệ thống")
        {
            DraftOutcome::NegativeAmount { field, .. } => assert_eq!(field, "deposit_amount"),
            other => panic!("đặt phòng: mong đợi NegativeAmount, nhận {other:?}"),
        }
    }

    // ─── Danh sách khách: không được rơi ai trong im lặng ───

    /// Ba khách vào, hai khách ra, model không được báo — đúng lớp lỗi cả nhánh
    /// này tồn tại để diệt. Một khách bị bỏ khỏi hồ sơ khai báo tạm trú, và trên
    /// thẻ chỉ hiện "2 người" chứ không có chỗ nào nói người thứ ba đã biến mất.
    #[tokio::test]
    async fn a_guest_entry_that_yields_no_name_is_reported_not_dropped() {
        for (ten, gia_tri) in [
            ("số", serde_json::json!(123)),
            (
                "object lồng",
                serde_json::json!({ "ho": "Lê", "ten": "Cường" }),
            ),
            ("chuỗi rỗng", serde_json::json!("")),
            ("chuỗi toàn khoảng trắng", serde_json::json!("   ")),
        ] {
            let pool = test_pool().await;
            seed_room(
                &pool,
                "room-glist",
                "P840",
                "Standard Room",
                400_000,
                "vacant",
            )
            .await;

            let args = serde_json::json!({
                "room_id": "room-glist",
                "nights": 1,
                "guests": [
                    { "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" },
                    { "full_name": gia_tri, "doc_number": "079088007766" },
                    { "full_name": "Phạm Thị Dung", "doc_number": "079301005678" }
                ]
            });

            let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
                .await
                .unwrap_or_else(|error| panic!("{ten}: không phải lỗi hệ thống: {error:?}"));

            match outcome {
                DraftOutcome::UnreadableGuestName { positions } => {
                    assert_eq!(positions, vec![2], "{ten}: phải gọi đúng vị trí khách hỏng");
                }
                DraftOutcome::Ready(action) => panic!(
                    "{ten}: khách thứ hai bị bỏ trong im lặng, thẻ chỉ còn {} người: {:?}",
                    check_in_payload(&action).guests.len(),
                    action.display
                ),
                other => panic!("{ten}: mong đợi UnreadableGuestName, nhận {other:?}"),
            }
        }
    }

    /// Cùng hàm đọc, tool kia — `parse_guest_list` dùng chung nên lỗ hổng cũng
    /// dùng chung.
    #[tokio::test]
    async fn a_backfill_guest_entry_that_yields_no_name_is_reported_not_dropped() {
        let pool = test_pool().await;
        seed_room(&pool, "room-bf", "P841", "Standard Room", 400_000, "vacant").await;

        let mut args = backfill_args_checked_out();
        args["guests"] = serde_json::json!([
            { "full_name": "Trần Thị Bích", "doc_number": "079301005678" },
            { "full_name": 123 }
        ]);

        match build_backfill_draft(&pool, &args, BACKFILL_TODAY)
            .await
            .expect("không phải lỗi hệ thống")
        {
            DraftOutcome::UnreadableGuestName { positions } => assert_eq!(positions, vec![2]),
            DraftOutcome::Ready(action) => panic!(
                "khách bị bỏ trong im lặng, còn {} người",
                backfill_payload(&action).guests.len()
            ),
            other => panic!("mong đợi UnreadableGuestName, nhận {other:?}"),
        }
    }

    // ─── Thẻ nhận phòng phải dò trùng lịch trên CẢ KỲ Ở ───
    //
    // `build_warnings` chỉ đọc `load_room_status_now` — trạng thái *ngay lúc
    // này*. Phòng trống hôm nay nhưng có người đặt từ ngày kia thì thẻ vẫn ghi
    // một ngày trả mà `check_in` sẽ không bao giờ nhận: đo được `nights = 4` ra
    // thẻ "10/08/2026" rồi lệnh trả `Conflict: Room has a reservation starting
    // 2026-08-08`. Một ngày sai trên thẻ, ở đúng cái tool sinh ra vì con bug
    // ngày. Hai tool mới đã hỏi `load_free_rooms_between`; tool này thì chưa.

    #[tokio::test]
    async fn a_room_free_today_but_booked_inside_the_stay_is_warned_about() {
        let pool = test_pool().await;
        seed_room(&pool, "room-4b", "4B", "Standard Room", 400_000, "vacant").await;
        seed_guest(&pool, "guest-later", "Khách đặt trước").await;
        seed_reservation(
            &pool,
            "book-later",
            "room-4b",
            "guest-later",
            "2026-08-08",
            "2026-08-09",
        )
        .await;
        seed_room_calendar_day(&pool, "room-4b", "2026-08-08", "book-later").await;

        let args = serde_json::json!({
            "room_id": "room-4b",
            "nights": 4,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let action = ready(
            build_check_in_draft(&pool, &args, "2026-08-06")
                .await
                .expect("phòng bận vẫn tra được giá — không phải lỗi hệ thống"),
        );

        // Thẻ ghi 10/08 — ngày `check_in` sẽ từ chối. Nó phải nói ra, và phải
        // nêu đúng khoảng ngày đang nói tới.
        let overlap = action
            .warnings
            .iter()
            .find(|warning| warning.contains("10/08/2026"))
            .unwrap_or_else(|| {
                panic!(
                    "thẻ ghi ngày trả 10/08/2026 mà không cảnh báo trùng lịch: {:?}",
                    action.warnings
                )
            });
        assert!(overlap.contains("06/08/2026"), "{overlap}");
        assert!(
            overlap.contains("check_in"),
            "cảnh báo phải nói rõ lệnh nào sẽ từ chối: {overlap}"
        );
    }

    /// Chiều ngược lại, để test trên không đúng một cách vô nghĩa: trống suốt kỳ
    /// ở thì **im lặng**. Một cảnh báo bật lên ở ca hợp lệ dạy lễ tân bỏ qua
    /// cảnh báo — tệ hơn hẳn không có cảnh báo.
    #[tokio::test]
    async fn a_room_free_across_the_whole_stay_gets_no_overlap_warning() {
        let pool = test_pool().await;
        seed_room(&pool, "room-4b", "4B", "Standard Room", 400_000, "vacant").await;
        seed_guest(&pool, "guest-later", "Khách đặt trước").await;
        seed_reservation(
            &pool,
            "book-later",
            "room-4b",
            "guest-later",
            "2026-08-20",
            "2026-08-21",
        )
        .await;
        seed_room_calendar_day(&pool, "room-4b", "2026-08-20", "book-later").await;

        let args = serde_json::json!({
            "room_id": "room-4b",
            "nights": 2,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let action = ready(
            build_check_in_draft(&pool, &args, "2026-08-06")
                .await
                .expect("ca thường không được lỗi"),
        );
        assert!(
            action.warnings.is_empty(),
            "phòng trống suốt kỳ ở mà vẫn bị cảnh báo: {:?}",
            action.warnings
        );
    }

    /// Giọng của cảnh báo phải khớp cái lệnh thật sự làm.
    ///
    /// `check_in_tx` bắt **mọi** `status` khác `vacant` là `Conflict` và trả về
    /// ngay — phòng đang có khách thì không nhận phòng được, chấm hết. Câu cảnh
    /// báo cũ ("Phòng đang có khách ở.") đọc như một lời lưu ý có thể cân nhắc
    /// rồi bấm *Đồng ý*, trong khi cảnh báo của `draft_reserve`/`draft_backfill`
    /// viết thẳng "lệnh sẽ từ chối". Test này ghim sự chênh lệch ấy đã đóng.
    #[tokio::test]
    async fn the_check_in_card_says_the_command_will_refuse_an_occupied_room() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-busy",
            "P850",
            "Standard Room",
            400_000,
            "occupied",
        )
        .await;
        seed_guest(&pool, "guest-now", "Khách đang ở").await;
        seed_booking(
            &pool,
            "book-now",
            "room-busy",
            "guest-now",
            "2026-06-01",
            "2026-06-03",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-busy",
            "nights": 1,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let action = ready(
            build_check_in_draft(&pool, &args, "2026-06-01")
                .await
                .expect("phòng bận vẫn tra được giá"),
        );

        let warning = action
            .warnings
            .iter()
            .find(|warning| warning.contains("Phòng đang có khách ở."))
            .unwrap_or_else(|| panic!("thiếu cảnh báo phòng bận: {:?}", action.warnings));
        assert!(
            warning.contains("TỪ CHỐI") && warning.contains("check_in"),
            "cảnh báo phải nói rõ lệnh nào sẽ từ chối, không nói giọng lưu ý: {warning}"
        );
        // Phòng này vừa `occupied` vừa có khách, nên nó khớp **cả hai** điều
        // kiện của khối trạng thái phòng. Chỉ được ra một câu: câu riêng ở trên,
        // vì nó nói thêm được việc phải làm. Thêm câu "đang ở trạng thái
        // «occupied»" vào đây là kể cùng một lần từ chối hai lần trên một cái
        // thẻ lễ tân đọc lúc có khách đứng trước mặt.
        assert_eq!(
            action.warnings.len(),
            1,
            "phòng bận chỉ được một câu cảnh báo, không kèm câu trạng thái trùng ý: {:?}",
            action.warnings
        );
    }

    // ─── Trần số đêm của đặt phòng trước ───

    /// `create_reservation` từ chối quá 90 đêm
    /// (`reservation_lifecycle::MAX_RESERVATION_NIGHTS`), nhưng tầng thẻ không
    /// kiểm gì: một lỗi gõ năm (`2027` thay `2026`) dựng ra cái thẻ "122 đêm"
    /// với đủ tiền phòng cho 122 đêm, rồi lệnh mới từ chối **sau** khi lễ tân
    /// đã bấm *Đồng ý*.
    ///
    /// Cùng khuôn với mọi luật khác của tầng này: lời từ chối là một câu chỉ
    /// đường cho model, không phải một lỗi ràng buộc thô nổ ra ở cuối.
    #[tokio::test]
    async fn a_reservation_longer_than_the_cap_never_becomes_a_card() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P851",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        // Đúng hình dạng lỗi gõ năm: nhận 10/06/2026, trả 10/10/2026 — 122 đêm.
        let outcome = build_reserve_draft(
            &pool,
            &reserve_args("2026-06-10", "2026-10-10"),
            "2026-06-01",
        )
        .await
        .expect("khoảng ngày quá dài không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::TooManyNights { nights, max } => {
                assert_eq!(nights, 122);
                assert_eq!(max, MAX_RESERVATION_NIGHTS);
            }
            DraftOutcome::Ready(action) => panic!(
                "thẻ 122 đêm dựng được rồi lệnh mới từ chối: {:?}",
                action.display
            ),
            other => panic!("mong đợi TooManyNights, nhận {other:?}"),
        }
    }

    /// Biên: đúng trần thì vẫn dựng được thẻ. Không có test này thì một hàng rào
    /// lệch một đơn vị (`>=` thay `>`) vẫn xanh, và nó chặn mất một lượt đặt
    /// phòng hoàn toàn hợp lệ.
    #[tokio::test]
    async fn a_reservation_exactly_at_the_cap_still_builds_a_card() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-res",
            "P852",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        // 10/06 + 90 đêm = 08/09/2026.
        let action = ready(
            build_reserve_draft(
                &pool,
                &reserve_args("2026-06-10", "2026-09-08"),
                "2026-06-01",
            )
            .await
            .expect("đúng trần vẫn hợp lệ"),
        );
        assert_eq!(reserve_payload(&action).nights, 90);
    }

    #[test]
    fn nights_become_a_local_date_range() {
        assert_eq!(
            check_out_date_from_nights("2026-08-03", 2).expect("hợp lệ"),
            "2026-08-05"
        );
        assert_eq!(
            check_out_date_from_nights("2026-12-31", 1).expect("hợp lệ"),
            "2027-01-01"
        );
        assert!(check_out_date_from_nights("2026-08-03", 0).is_err());
        assert!(check_out_date_from_nights("khong-phai-ngay", 1).is_err());
    }

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        use sqlx::sqlite::SqlitePoolOptions;

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

    /// Cùng công thức seed đã dùng ở `tools.rs`'s `seed_room`: chỉ một dòng
    /// `rooms` là đủ cho `calculate_room_price_preview` chạy được, không cần
    /// `room_types`/`pricing_rules`. `type` cố ý mang tên nhiều từ như thật
    /// (`"Standard Room"`, `"Deluxe Balcony"`) — `rooms.type` là tên hiển thị
    /// có khoảng trắng, một fixture một từ có thể che mất lỗi ghép chuỗi.
    async fn seed_room(
        pool: &Pool<Sqlite>,
        id: &str,
        name: &str,
        room_type: &str,
        base_price: i64,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, status)
             VALUES (?, ?, ?, 1, 0, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(room_type)
        .bind(base_price)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed room");
    }

    /// Ba hàm dưới đây dựng một phòng **đã bận** đúng cách PMS ghi: một hàng
    /// `guests`, một hàng `bookings`, rồi các hàng `room_calendar` trỏ về nó.
    /// Không đi tắt bằng một hàng `room_calendar` có `booking_id` NULL:
    /// `load_free_rooms_between` lọc `booking_id IS NOT NULL`, nên đường tắt ấy
    /// dựng ra một phòng mà truy vấn coi là **trống** — fixture xanh mà thực tế
    /// bận, đúng kiểu che mất lỗi cần bắt.
    async fn seed_guest(pool: &Pool<Sqlite>, id: &str, full_name: &str) {
        sqlx::query(
            "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
             VALUES (?, 'domestic', ?, ?, '2026-05-01T08:00:00+07:00')",
        )
        .bind(id)
        .bind(full_name)
        .bind(format!("DOC-{id}"))
        .execute(pool)
        .await
        .expect("seed guest");
    }

    async fn seed_booking(
        pool: &Pool<Sqlite>,
        id: &str,
        room_id: &str,
        guest_id: &str,
        check_in_at: &str,
        expected_checkout: &str,
    ) {
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                nights, total_price, paid_amount, status, created_at
             ) VALUES (?, ?, ?, ?, ?, 1, 0, 0, 'active', ?)",
        )
        .bind(id)
        .bind(room_id)
        .bind(guest_id)
        .bind(check_in_at)
        .bind(expected_checkout)
        .bind(check_in_at)
        .execute(pool)
        .await
        .expect("seed booking");
    }

    /// Một **đặt phòng trước** (`status='booked'`), khác hẳn `seed_booking` ở
    /// trên (`'active'` = khách đang nằm trong phòng).
    ///
    /// Phân biệt này không phải chi tiết vụn: `load_room_status_now` LEFT JOIN
    /// `bookings` với `b.status = 'active'`, nên một reservation của tuần sau mà
    /// seed nhầm thành `'active'` làm phòng trông như **đang có khách ngay lúc
    /// này**. Test dò trùng lịch dùng fixture ấy sẽ đỏ/xanh nhờ cảnh báo
    /// trạng-thái-bây-giờ chứ không nhờ phép dò khoảng ngày — đúng kiểu đỏ vì
    /// một guard hàng xóm.
    async fn seed_reservation(
        pool: &Pool<Sqlite>,
        id: &str,
        room_id: &str,
        guest_id: &str,
        check_in_at: &str,
        expected_checkout: &str,
    ) {
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                nights, total_price, paid_amount, status, created_at
             ) VALUES (?, ?, ?, ?, ?, 1, 0, 0, 'booked', ?)",
        )
        .bind(id)
        .bind(room_id)
        .bind(guest_id)
        .bind(check_in_at)
        .bind(expected_checkout)
        .bind(check_in_at)
        .execute(pool)
        .await
        .expect("seed reservation");
    }

    async fn seed_room_calendar_day(
        pool: &Pool<Sqlite>,
        room_id: &str,
        date: &str,
        booking_id: &str,
    ) {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status)
             VALUES (?, ?, ?, 'occupied')",
        )
        .bind(room_id)
        .bind(date)
        .bind(booking_id)
        .execute(pool)
        .await
        .expect("seed room_calendar");
    }

    /// Phòng có phụ thu thêm người khác 0 — thứ `seed_room` ở trên **không**
    /// dựng được vì nó để schema điền `max_guests`/`extra_person_fee` mặc định
    /// (2, 0). Với mặc định đó, gọi preview kèm số khách hay không kèm đều ra
    /// cùng một số, nên không fixture nào ở trên nhìn thấy được sai lệch.
    async fn seed_room_charging_extra_guests(
        pool: &Pool<Sqlite>,
        id: &str,
        name: &str,
        room_type: &str,
        base_price: i64,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, 1, 0, ?, 2, 150000, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(room_type)
        .bind(base_price)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed room có phụ thu thêm người");
    }
}
