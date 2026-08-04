import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CARD_TTL_MS, type ProposedAction } from "@/types/assistant";
import { ProposedActionCard } from "./ProposedActionCard";

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

describe("ProposedActionCard", () => {
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
});
