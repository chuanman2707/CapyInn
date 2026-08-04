import { useEffect, useState, type FormEvent } from "react";
import { Send } from "lucide-react";

import { ProposedActionCard } from "@/components/assistant/ProposedActionCard";
import { Button } from "@/components/ui/button";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { useHotelStore } from "@/stores/useHotelStore";
import type { ScreenContext } from "@/types/assistant";

type HotelSlice = {
  activeTab: string;
  roomDetail: { room?: { id?: string; name?: string } } | null;
  roomChangeBookingId: string | null;
};

/// Ngữ cảnh đọc từ hotel store, không bắt từng trang khai lại.
export function buildScreenContext(state: HotelSlice): ScreenContext {
  return {
    route: state.activeTab,
    selectedRoomId: state.roomDetail?.room?.id,
    selectedRoomNumber: state.roomDetail?.room?.name,
    selectedBookingId: state.roomChangeBookingId ?? undefined,
  };
}

export function AssistantPanel() {
  const { open, messages, pendingAction, busy, settings, send, approve, dismissAction } =
    useAssistantStore();
  const hotel = useHotelStore();
  const [draft, setDraft] = useState("");
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Đồng hồ để thẻ tự chuyển sang trạng thái hết hạn mà không cần thao tác.
  useEffect(() => {
    if (!pendingAction) return;
    const timer = setInterval(() => setNowMs(Date.now()), 10_000);
    return () => clearInterval(timer);
  }, [pendingAction]);

  if (!settings?.gate.ready || !open) return null;

  const context = buildScreenContext(hotel as unknown as HotelSlice);
  const contextLabel = context.selectedRoomNumber
    ? `Đang xem: ${context.selectedRoomNumber}`
    : `Đang xem: ${context.route}`;

  const onSubmit = async (event: FormEvent) => {
    event.preventDefault();
    const message = draft;
    setDraft("");
    await send(message, context);
  };

  return (
    <aside className="flex w-[380px] shrink-0 flex-col border-l border-slate-100 bg-white">
      <header className="flex h-[88px] items-center px-5 text-sm font-semibold">Trợ lý quầy</header>

      <div className="flex-1 space-y-3 overflow-y-auto px-5 pb-4">
        {messages.map((message) => (
          <p
            key={message.id}
            className={
              message.kind === "user"
                ? "ml-auto max-w-[85%] rounded-xl bg-brand-primary/10 px-3 py-2 text-sm"
                : message.kind === "error"
                  ? "max-w-[85%] rounded-xl bg-red-50 px-3 py-2 text-sm text-red-600"
                  : "max-w-[85%] rounded-xl bg-slate-50 px-3 py-2 text-sm"
            }
          >
            {message.text}
          </p>
        ))}

        {busy && <p className="text-xs text-brand-muted">Đang tra dữ liệu…</p>}

        {pendingAction && (
          <ProposedActionCard
            action={pendingAction}
            busy={busy}
            nowMs={nowMs}
            onApprove={approve}
            onRebuild={() => void send("Tính lại thẻ nhận phòng vừa rồi.", context)}
            onDismiss={dismissAction}
          />
        )}
      </div>

      <form onSubmit={onSubmit} className="border-t border-slate-100 p-4">
        <p className="mb-2 text-[11px] text-brand-muted">{contextLabel}</p>
        <div className="flex gap-2">
          <input
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="Hỏi hoặc ra việc…"
            className="flex-1 rounded-xl border border-slate-200 px-3 py-2 text-sm outline-none focus:border-brand-primary"
          />
          <Button type="submit" size="sm" disabled={busy || !draft.trim()}>
            <Send size={16} />
          </Button>
        </div>
      </form>
    </aside>
  );
}
