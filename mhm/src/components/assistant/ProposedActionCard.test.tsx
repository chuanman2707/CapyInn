import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CARD_TTL_MS, type ProposedAction } from "@/types/assistant";
import { ProposedActionCard } from "./ProposedActionCard";

const action: ProposedAction = {
  kind: "check_in",
  payload: {
    room_id: "R1",
    guests: [{ full_name: "Nguyễn Văn Nam" }],
    nights: 2,
    source: "walk-in",
    notes: null,
    paid_amount: 500000,
    pricing_type: "nightly",
  },
  display: {
    room_id: "Phòng 201",
    guests: "Nguyễn Văn Nam",
    nights: "2 đêm",
    source: "walk-in",
    notes: "—",
    paid_amount: "500.000 ₫",
    pricing_type: "nightly",
    total: "700.000 ₫",
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
});
