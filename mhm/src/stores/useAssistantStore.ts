import { create } from "zustand";

import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
import type {
  AssistantMessage,
  AssistantSettings,
  AssistantTurnResponse,
  ChatMessage,
  ProposedAction,
  ScreenContext,
} from "@/types/assistant";

type AssistantState = {
  open: boolean;
  messages: AssistantMessage[];
  history: ChatMessage[];
  pendingAction: ProposedAction | null;
  busy: boolean;
  error: string | null;
  settings: AssistantSettings | null;

  togglePanel: () => void;
  refreshSettings: () => Promise<void>;
  send: (message: string, screenContext: ScreenContext) => Promise<void>;
  approve: () => Promise<void>;
  dismissAction: () => void;
};

function nextId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function readErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Trợ lý gặp lỗi không xác định.";
}

export const useAssistantStore = create<AssistantState>((set, get) => ({
  open: false,
  messages: [],
  history: [],
  pendingAction: null,
  busy: false,
  error: null,
  settings: null,

  togglePanel: () => set((state) => ({ open: !state.open })),

  refreshSettings: async () => {
    try {
      const settings = await invokeCommand<AssistantSettings>("get_assistant_settings");
      set({ settings });
    } catch {
      set({ settings: null });
    }
  },

  send: async (message, screenContext) => {
    const trimmed = message.trim();
    if (!trimmed || get().busy) return;

    set((state) => ({
      busy: true,
      error: null,
      messages: [...state.messages, { id: nextId(), kind: "user", text: trimmed }],
    }));

    try {
      const response = await invokeCommand<AssistantTurnResponse>("assistant_turn", {
        request: {
          message: trimmed,
          screen_context: screenContext,
          history: get().history,
        },
      });

      set((state) => ({
        busy: false,
        history: response.history,
        pendingAction: response.proposed_action,
        messages: response.reply
          ? [...state.messages, { id: nextId(), kind: "assistant", text: response.reply }]
          : state.messages,
      }));
    } catch (error) {
      const text = readErrorMessage(error);
      set((state) => ({
        busy: false,
        error: text,
        messages: [...state.messages, { id: nextId(), kind: "error", text }],
      }));
    }
  },

  approve: async () => {
    const action = get().pendingAction;
    if (!action || get().busy) return;

    set({ busy: true, error: null });
    try {
      await invokeWriteCommand("check_in", { req: action.payload });
      set((state) => ({
        busy: false,
        pendingAction: null,
        messages: [
          ...state.messages,
          { id: nextId(), kind: "assistant", text: "Đã nhận phòng xong." },
        ],
      }));
    } catch (error) {
      // Giữ nguyên thẻ: người dùng còn sửa hoặc mở form làm tay được.
      set({ busy: false, error: readErrorMessage(error) });
    }
  },

  dismissAction: () => set({ pendingAction: null }),
}));
