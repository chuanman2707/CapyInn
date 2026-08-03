import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
  createIdempotencyKey: (command: string) => `${command}:test`,
}));

import { useAssistantStore } from "./useAssistantStore";
import type { ChatMessage, ProposedAction } from "@/types/assistant";
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

// Mốc "hiện tại" giả lập cho cả file: 1 giây sau khi sampleAction được dựng,
// nên sampleAction luôn còn hạn trừ khi một test tự dời built_at_ms đi chỗ khác.
const NOW_MS = sampleAction.built_at_ms + 1_000;

describe("useAssistantStore", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW_MS);
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

  afterEach(() => {
    vi.useRealTimers();
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

  it("thẻ đã hết hạn thì approve() từ chối, không gọi PMS và giữ nguyên thẻ", async () => {
    const expiredAction: ProposedAction = {
      ...sampleAction,
      built_at_ms: NOW_MS - CARD_TTL_MS - 1,
    };
    useAssistantStore.setState({ pendingAction: expiredAction });

    await useAssistantStore.getState().approve();

    expect(invokeWriteCommand).not.toHaveBeenCalled();
    expect(useAssistantStore.getState().pendingAction).toEqual(expiredAction);
    expect(useAssistantStore.getState().error).toMatch(/hết hạn/);
  });

  it("phát lại lịch sử y nguyên hai chiều: gửi đúng lịch sử đã có, nhận đúng lịch sử backend trả về", async () => {
    const seededHistory: ChatMessage[] = [
      { role: "user", content: "phòng nào trống" },
      {
        role: "assistant",
        content: null,
        tool_calls: [
          { id: "call_1", type: "function", function: { name: "list_rooms", arguments: "{}" } },
        ],
      },
      { role: "tool", content: "[]", tool_call_id: "call_1" },
    ];
    const returnedHistory: ChatMessage[] = [
      ...seededHistory,
      { role: "user", content: "check-in phòng R1" },
      { role: "assistant", content: "Đã tạo thẻ xác nhận." },
    ];

    useAssistantStore.setState({ history: seededHistory });
    invokeCommand.mockResolvedValue({
      reply: "Đã tạo thẻ xác nhận.",
      proposed_action: null,
      history: returnedHistory,
    });

    await useAssistantStore.getState().send("check-in phòng R1", { route: "rooms" });

    // Chiều đi: lịch sử gửi lên đúng bằng lịch sử đã có trong store, không rỗng,
    // không bị lọc bớt vai "tool".
    expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
      request: expect.objectContaining({ history: seededHistory }),
    });
    // Chiều về: store phải thay bằng đúng lịch sử backend trả, không giữ lại
    // lịch sử cũ và không tự chế thêm.
    expect(useAssistantStore.getState().history).toEqual(returnedHistory);
  });
});
