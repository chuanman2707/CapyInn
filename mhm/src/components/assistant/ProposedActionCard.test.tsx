import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CARD_TTL_MS, type ProposedAction, type ProposedActionKind } from "@/types/assistant";
import { ProposedActionCard } from "./ProposedActionCard";

/// Ba tiêu đề, viết ra NGUYÊN VĂN ở đây chứ không import từ `ACTION_KIND_COPY`.
///
/// Import bảng chữ là để test so bảng với chính nó: đổi "Xác nhận đặt phòng"
/// thành "Xác nhận nhận phòng" cho cả hai loại thì bảng vẫn khớp bảng và không
/// một test nào đỏ, trong khi thứ đang phải canh chính là **ba chuỗi ấy khác
/// nhau**. Cùng bẫy mà `SUGGESTIONS` đã dính, và `AssistantPanel.test.tsx` đã
/// ghi lại ở hằng `SAVE_NOTICE`.
const TITLES: Record<ProposedActionKind, string> = {
  check_in: "Xác nhận nhận phòng",
  reserve: "Xác nhận đặt phòng",
  backfill: "Xác nhận ghi bù",
};

const action: ProposedAction = {
  kind: "check_in",
  payload: {
    room_id: "R1",
    guests: [
      { full_name: "Nguyễn Văn Nam", doc_number: "079201001234", phone: "0909000111" },
      { full_name: "Trần Thị Bích", doc_number: "079301005678", phone: null },
    ],
    nights: 2,
    source: "walk-in",
    notes: null,
    paid_amount: 500000,
    pricing_type: "nightly",
  },
  // Đúng hình dạng `build_check_in_display` (Rust) sinh ra: một dòng đếm đầu
  // người, rồi mỗi khách một dòng mang đủ trường đã điền. Khoá "Khách N" cố ý
  // không có trong FIELD_LABELS — thẻ phải render generic theo display.
  display: {
    room_id: "Phòng 201",
    guests: "2 người",
    "Khách 1": "Nguyễn Văn Nam · CCCD: 079201001234 · SĐT: 0909000111",
    "Khách 2": "Trần Thị Bích · CCCD: 079301005678",
    nights: "2 đêm",
    source: "walk-in",
    notes: "—",
    paid_amount: "500.000 ₫",
    pricing_type: "nightly",
    total: "700.000 ₫",
    // Key cố ý vắng mặt trong FIELD_LABELS: khoá test vào việc render generic
    // theo action.display, không cho component lùi về danh sách trường cứng.
    extra_field: "Giá trị lạ",
  },
  preview: { total: 700000 },
  warnings: ["Phòng đang ở trạng thái bẩn, chưa dọn."],
  built_at_ms: 1_000_000,
};

/// Thẻ đặt phòng trước. Payload đúng `CreateReservationRequest` — `guests: null`
/// (quầy không thu phụ thu thêm người) và `nights` dẫn xuất từ hai ngày.
const reserveAction: ProposedAction = {
  kind: "reserve",
  payload: {
    room_id: "R4B",
    guest_name: "Hyungchul Lee",
    guest_phone: null,
    guest_doc_number: null,
    check_in_date: "2026-08-08",
    check_out_date: "2026-08-09",
    nights: 1,
    deposit_amount: 200000,
    source: "walk-in",
    notes: null,
    guests: null,
  },
  display: {
    guest_name: "Hyungchul Lee",
    room_id: "Phòng 4B",
    check_in_date: "08/08/2026",
    check_out_date: "09/08/2026",
    nights: "1 đêm",
    deposit_amount: "200.000 ₫",
    total: "400.000 ₫",
  },
  preview: { total: 400000 },
  warnings: [],
  built_at_ms: 1_000_000,
};

/// Thẻ ghi bù. `check_out_date: null` = khách còn ở, nên có
/// `expected_checkout_date`; `total_price` là số của preview, không phải số model
/// đưa.
const backfillAction: ProposedAction = {
  kind: "backfill",
  payload: {
    room_id: "R1",
    guests: [{ full_name: "Trần Thị Bích", doc_number: "079301005678" }],
    check_in_date: "2026-08-04",
    check_out_date: null,
    expected_checkout_date: "2026-08-07",
    total_price: 600000,
    paid_amount: 0,
    source: "walk-in",
    notes: null,
  },
  display: {
    room_id: "Phòng 201",
    guests: "1 người",
    check_in_date: "04/08/2026",
    expected_checkout_date: "07/08/2026",
    total: "600.000 ₫",
    paid_amount: "0 ₫",
  },
  preview: { total: 600000 },
  warnings: [],
  built_at_ms: 1_000_000,
};

const ACTION_BY_KIND: Record<ProposedActionKind, ProposedAction> = {
  check_in: action,
  reserve: reserveAction,
  backfill: backfillAction,
};

/// Một thẻ mang `kind` NGOÀI hợp đồng — thứ `tsc` không cho viết thẳng, nên phải
/// ép kiểu.
///
/// Ép ở đây không phải để lách kiểu cho tiện: `kind` ngoài đời tới từ backend qua
/// IPC, chỗ **không có kiểu nào được kiểm lúc chạy**. Ép kiểu chính là mô phỏng
/// đúng cái ranh giới ấy.
function actionWithKind(kind: string): ProposedAction {
  return { ...action, kind } as unknown as ProposedAction;
}

describe("ProposedActionCard", () => {
  // ── BA LOẠI THẺ, BA TIÊU ĐỀ ────────────────────────────────────────────────
  //
  // Mỗi loại một test riêng, và mỗi test khẳng định **hai vế**: tiêu đề của
  // chính nó CÓ, tiêu đề của hai loại kia KHÔNG. Vế âm mới là vế phân biệt được
  // — thiếu nó thì một bản in cả ba tiêu đề lên mọi thẻ vẫn xanh, và ba tiêu đề
  // cùng lúc thì lễ tân không đọc ra mình sắp làm gì.
  //
  // Đây cũng là chỗ đo "khác nhau BẰNG CHỮ": ba chuỗi này phải đọc ra khác nhau
  // trên DOM. Màu thì jsdom không thấy, và người mù màu cũng không — nên nếu ba
  // thẻ chỉ khác nhau bằng màu, không test nào ở đây xanh được.

  it("thẻ nhận phòng có tiêu đề Xác nhận nhận phòng", () => {
    render(
      <ProposedActionCard
        action={ACTION_BY_KIND.check_in}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByText(TITLES.check_in)).toBeInTheDocument();
    expect(screen.queryByText(TITLES.reserve)).not.toBeInTheDocument();
    expect(screen.queryByText(TITLES.backfill)).not.toBeInTheDocument();
  });

  it("thẻ đặt phòng trước có tiêu đề Xác nhận đặt phòng", () => {
    render(
      <ProposedActionCard
        action={ACTION_BY_KIND.reserve}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByText(TITLES.reserve)).toBeInTheDocument();
    expect(screen.queryByText(TITLES.check_in)).not.toBeInTheDocument();
    expect(screen.queryByText(TITLES.backfill)).not.toBeInTheDocument();
  });

  it("thẻ ghi bù có tiêu đề Xác nhận ghi bù", () => {
    render(
      <ProposedActionCard
        action={ACTION_BY_KIND.backfill}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByText(TITLES.backfill)).toBeInTheDocument();
    expect(screen.queryByText(TITLES.check_in)).not.toBeInTheDocument();
    expect(screen.queryByText(TITLES.reserve)).not.toBeInTheDocument();
  });

  it("dòng trạng thái lúc gửi cũng đi theo loại thẻ, không ghim cứng chữ nhận phòng", () => {
    // Cùng lớp lỗi với tiêu đề, ở một dòng chữ khác: thẻ đặt phòng mà báo "Đang
    // gửi lệnh nhận phòng…" là nói sai đúng lúc lệnh đang bay và không ai rút
    // lại được. Có test riêng vì test "đang chạy thì hiện thông báo trạng thái"
    // bên dưới chỉ chạy trên thẻ nhận phòng, nên nó xanh với cả bản ghim cứng.
    render(
      <ProposedActionCard
        action={ACTION_BY_KIND.reserve}
        busy
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent("Đang gửi lệnh đặt phòng…");
    expect(screen.getByRole("status")).not.toHaveTextContent("nhận phòng");
  });

  it("kind lạ thì thẻ vẫn vẽ được và nói thẳng là không rõ loại", () => {
    // Hợp đồng frontend↔backend lệch (bản cũ gặp bản mới) là trạng thái đáng lẽ
    // không tồn tại — hai bên đóng gói chung một app — nhưng nếu nó xảy ra thì
    // `ACTION_KIND_COPY[kind].title` trần sẽ ném `undefined.title` và thổi bay
    // **cả panel**, không chỉ cái thẻ. Mất trắng panel tệ hơn hẳn một dòng tiêu
    // đề nói "không rõ loại".
    //
    // Và nó KHÔNG được mượn tên của loại nào: `approve()` đằng nào cũng từ chối
    // thẻ này, nên gọi nó là "nhận phòng" là dán nhãn sai lên thứ sắp bị chặn.
    render(
      <ProposedActionCard
        action={actionWithKind("modify_reservation")}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByText("Thẻ không rõ loại, không duyệt được")).toBeInTheDocument();
    for (const title of Object.values(TITLES)) {
      expect(screen.queryByText(title)).not.toBeInTheDocument();
    }
  });

  it("hiện mọi dòng trong display", () => {
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    for (const value of Object.values(action.display)) {
      expect(screen.getByText(value)).toBeInTheDocument();
    }
  });

  it("hiện số giấy tờ của từng khách, không chỉ tên", () => {
    // Con số này sẽ được ghi vào guests.doc_number rồi đi vào khai báo tạm
    // trú. Người bấm "Đồng ý" phải nhìn thấy nó trước khi bấm; thẻ là toàn bộ
    // cơ chế cho phép, nên cái gì thẻ không hiện là cái được duyệt mù.
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    for (const guest of action.payload.guests) {
      expect(screen.getByText(new RegExp(guest.doc_number as string))).toBeInTheDocument();
      expect(screen.getByText(new RegExp(guest.full_name))).toBeInTheDocument();
    }
  });

  it("hiện trường mà FIELD_LABELS chưa biết, dùng key làm nhãn dự phòng", () => {
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByText("Giá trị lạ")).toBeInTheDocument();
    expect(screen.getByText("extra_field")).toBeInTheDocument();
  });

  it("hiện cảnh báo lấy từ PMS", () => {
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByText("Phòng đang ở trạng thái bẩn, chưa dọn.")).toBeInTheDocument();
  });

  it("cảnh báo nằm TRONG viên cảnh báo, không trôi ra ngoài", () => {
    // Test "hiện cảnh báo lấy từ PMS" phía trên chỉ hỏi chữ có mặt ở đâu đó
    // trên thẻ hay không, nên nó xanh với cả bản để cảnh báo trôi lạc giữa bảng
    // giá trị. Từ khi cảnh báo được gom vào một viên riêng thì "nằm ở đâu" mới
    // là thứ phải đo — người bấm *Đồng ý* quét thẻ theo khối, không đọc từng
    // dòng.
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByRole("list", { name: "Cảnh báo từ PMS" })).toHaveTextContent(
      "Phòng đang ở trạng thái bẩn, chưa dọn.",
    );
  });

  it("không có cảnh báo nào thì KHÔNG vẽ viên cảnh báo rỗng", () => {
    // Vế âm, và là vế duy nhất bắt được bản "bọc luôn vẽ, chỉ nội dung có điều
    // kiện": một viên nền vàng RỖNG thường trực làm mọi test dò chữ xanh, vì
    // jsdom không nhìn thấy nền. Đây đúng là loại test rỗng thứ năm mà nhánh
    // này đã dính ở `historyNotice`.
    render(
      <ProposedActionCard
        action={{ ...action, warnings: [] }}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    // Vế dương kèm theo: thẻ VẪN được vẽ, chỉ là không có viên cảnh báo. Thiếu
    // câu này thì một component trả `null` cũng làm khẳng định trên xanh.
    expect(screen.getByText("Xác nhận nhận phòng")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Cảnh báo từ PMS" })).not.toBeInTheDocument();
  });

  it("hai cảnh báo trùng nội dung vẫn hiện đủ hai dòng, không phát cảnh báo key trùng", () => {
    const errorSpy = vi.spyOn(console, "error");
    const duplicateWarningAction: ProposedAction = {
      ...action,
      warnings: ["Trùng cảnh báo", "Trùng cảnh báo"],
    };

    render(
      <ProposedActionCard
        action={duplicateWarningAction}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getAllByText("Trùng cảnh báo")).toHaveLength(2);
    expect(errorSpy).not.toHaveBeenCalled();

    errorSpy.mockRestore();
  });

  it("bấm Đồng ý thì gọi onApprove", async () => {
    const onApprove = vi.fn();
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={onApprove}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: /đồng ý/i }));

    expect(onApprove).toHaveBeenCalledTimes(1);
  });

  it("bấm Huỷ thì gọi onDismiss", async () => {
    const onDismiss = vi.fn();
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={onDismiss}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: /huỷ/i }));

    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("đang chạy thì khoá nút Đồng ý", () => {
    render(
      <ProposedActionCard
        action={action}
        busy
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: /đồng ý/i })).toBeDisabled();
  });

  it("đang chạy thì hiện thông báo trạng thái cho việc gửi lệnh", () => {
    render(
      <ProposedActionCard
        action={action}
        busy
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent("Đang gửi lệnh nhận phòng…");
  });

  it("không chạy thì không hiện thông báo trạng thái", () => {
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("thẻ quá 5 phút thì chặn Đồng ý và mời Tính lại", () => {
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms + CARD_TTL_MS + 1}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: /đồng ý/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /tính lại/i })).toBeInTheDocument();
  });

  it("thẻ hết hạn mà vẫn đang chạy thì khoá luôn nút Tính lại", () => {
    render(
      <ProposedActionCard
        action={action}
        busy
        nowMs={action.built_at_ms + CARD_TTL_MS + 1}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: /tính lại/i })).toBeDisabled();
  });

  it("thẻ hết hạn, bấm Tính lại thì gọi onRebuild", async () => {
    const onRebuild = vi.fn();
    render(
      <ProposedActionCard
        action={action}
        busy={false}
        nowMs={action.built_at_ms + CARD_TTL_MS + 1}
        onApprove={vi.fn()}
        onRebuild={onRebuild}
        onDismiss={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: /tính lại/i }));

    expect(onRebuild).toHaveBeenCalledTimes(1);
  });

  /// Mọi nhãn trên thẻ phải là tiếng Việt. Khoá nào thiếu trong `FIELD_LABELS`
  /// rơi về **tên trường thô** (`FIELD_LABELS[key] ?? key`), nên lễ tân đọc
  /// `guest_doc_number` giữa một thẻ tiếng Việt — và không có gì báo cho người
  /// thêm khoá `display` mới phía Rust biết là họ vừa làm thế.
  ///
  /// `display` dưới đây mang **đủ** bộ khoá mà `build_reserve_display`
  /// (`draft.rs`) sinh ra, không phải bộ rút gọn của fixture ở đầu file: một
  /// fixture thiếu khoá thì test này chỉ kiểm những khoá nó tình cờ có.
  it("thẻ đặt phòng không để lọt tên trường thô lên nhãn", () => {
    const fullyLabelledReserve: ProposedAction = {
      ...reserveAction,
      display: {
        room_id: "Phòng 4B",
        guest_name: "Hyungchul Lee",
        guest_phone: "0909000111",
        guest_doc_number: "M12345678",
        check_in_date: "08/08/2026",
        check_out_date: "09/08/2026",
        nights: "1 đêm",
        deposit_amount: "200.000 ₫",
        source: "phone",
        notes: "—",
        guests: "Không ghi (không thu phụ thu thêm người)",
        total: "400.000 ₫",
      },
    };

    const { container } = render(
      <ProposedActionCard
        action={fullyLabelledReserve}
        busy={false}
        nowMs={action.built_at_ms}
        onApprove={vi.fn()}
        onRebuild={vi.fn()}
        onDismiss={vi.fn()}
      />,
    );

    const labels = Array.from(container.querySelectorAll("dt")).map((node) => node.textContent);
    expect(labels).toHaveLength(12);
    // Dấu gạch dưới chỉ có ở tên trường máy — nhãn tiếng Việt không bao giờ có.
    expect(labels.filter((label) => label?.includes("_"))).toEqual([]);
    expect(labels).toContain("Số điện thoại");
    expect(labels).toContain("Số CCCD");
  });
});
