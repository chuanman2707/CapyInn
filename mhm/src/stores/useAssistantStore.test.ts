import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
  createIdempotencyKey: (command: string) => `${command}:test`,
}));

import { useAssistantStore } from "./useAssistantStore";
import type { ProposedAction } from "@/types/assistant";
import { isActionExpired, CARD_TTL_MS } from "@/types/assistant";

const sampleAction: ProposedAction = {
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
    room_id: "R1",
    guests: "Nguyễn Văn Nam",
    nights: "2 đêm",
    source: "walk-in",
    notes: "—",
    paid_amount: "500.000 ₫",
    pricing_type: "nightly",
    total: "700.000 ₫",
  },
  preview: { total: 700000 },
  warnings: [],
  built_at_ms: 1_000_000,
};

describe("useAssistantStore", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    invokeWriteCommand.mockReset();
    useAssistantStore.setState({
      open: false,
      messages: [],
      pendingAction: null,
      busy: false,
      error: null,
      history: [],
    });
  });

  it("gửi kèm ngữ cảnh màn hình trong mỗi lượt", async () => {
    invokeCommand.mockResolvedValue({ reply: "ok", proposed_action: null, history: [] });

    await useAssistantStore
      .getState()
      .send("phòng nào trống", { route: "rooms", selectedRoomNumber: "201" });

    expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
      request: expect.objectContaining({
        message: "phòng nào trống",
        screen_context: { route: "rooms", selectedRoomNumber: "201" },
      }),
    });
  });

  it("giữ thẻ xác nhận khi backend trả về", async () => {
    invokeCommand.mockResolvedValue({
      reply: null,
      proposed_action: sampleAction,
      history: [],
    });

    await useAssistantStore.getState().send("check-in phòng R1", { route: "rooms" });

    expect(useAssistantStore.getState().pendingAction).toEqual(sampleAction);
  });

  it("duyệt thẻ thì gọi invokeWriteCommand với đúng payload", async () => {
    useAssistantStore.setState({ pendingAction: sampleAction });
    invokeWriteCommand.mockResolvedValue({ id: "B1" });

    await useAssistantStore.getState().approve();

    expect(invokeWriteCommand).toHaveBeenCalledWith("check_in", { req: sampleAction.payload });
    expect(useAssistantStore.getState().pendingAction).toBeNull();
  });

  it("lỗi PMS lúc duyệt thì giữ nguyên thẻ để sửa", async () => {
    useAssistantStore.setState({ pendingAction: sampleAction });
    invokeWriteCommand.mockRejectedValue(new Error("Phòng đã có khách"));

    await useAssistantStore.getState().approve();

    expect(useAssistantStore.getState().pendingAction).toEqual(sampleAction);
    expect(useAssistantStore.getState().error).toContain("Phòng đã có khách");
  });

  it("thẻ quá 5 phút bị coi là hết hạn", () => {
    expect(isActionExpired(sampleAction, sampleAction.built_at_ms + CARD_TTL_MS - 1)).toBe(false);
    expect(isActionExpired(sampleAction, sampleAction.built_at_ms + CARD_TTL_MS + 1)).toBe(true);
  });
});
