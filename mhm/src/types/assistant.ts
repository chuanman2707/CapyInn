export type ScreenContext = {
  route: string;
  selectedRoomId?: string;
  selectedRoomNumber?: string;
  selectedBookingId?: string;
  dateInView?: string;
};

export type CheckInGuestPayload = {
  full_name: string;
  doc_number?: string;
  phone?: string | null;
};

export type CheckInPayload = {
  room_id: string;
  guests: CheckInGuestPayload[];
  nights: number;
  source?: string | null;
  notes?: string | null;
  paid_amount?: number | null;
  pricing_type?: string | null;
};

export type ProposedAction = {
  kind: "check_in";
  payload: CheckInPayload;
  display: Record<string, string>;
  preview: Record<string, unknown>;
  warnings: string[];
  built_at_ms: number;
};

export type ChatMessage = {
  role: string;
  content?: string | null;
  tool_calls?: unknown[] | null;
  tool_call_id?: string | null;
};

export type AssistantTurnResponse = {
  reply: string | null;
  proposed_action: ProposedAction | null;
  history: ChatMessage[];
};

export type AssistantGateMissing = "api_key" | "cloud_data_opt_in" | "model" | "base_url";

export type AssistantSettings = {
  config: {
    preset: "deep_seek" | "open_router" | "custom";
    base_url: string;
    model: string;
  };
  has_api_key: boolean;
  cloud_data_opt_in: boolean;
  gate: { ready: boolean; missing: AssistantGateMissing[] };
};

export type AssistantMessage =
  | { id: string; kind: "user"; text: string }
  | { id: string; kind: "assistant"; text: string }
  | { id: string; kind: "error"; text: string };

export const CARD_TTL_MS = 5 * 60 * 1000;

export function isActionExpired(action: ProposedAction, nowMs: number): boolean {
  return nowMs - action.built_at_ms > CARD_TTL_MS;
}
