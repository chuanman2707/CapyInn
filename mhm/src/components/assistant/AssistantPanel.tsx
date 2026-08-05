import { useEffect, useState } from "react";

import { AssistantComposer } from "@/components/assistant/AssistantComposer";
import { AssistantEmptyState } from "@/components/assistant/AssistantEmptyState";
import { ProposedActionCard } from "@/components/assistant/ProposedActionCard";
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
  // Field riêng, không destructure nguyên store: set() của Zustand thay cả
  // reference top-level bất kể field nào đổi, nên destructure nguyên store
  // khiến panel — mount suốt vòng đời app — re-render theo mọi set() ở bất
  // kỳ đâu trong PMS, kể cả lúc đang ẩn. Chọn từng field thì chỉ re-render
  // khi đúng field đó đổi. Cùng idiom với MainShell.tsx (dòng ~85-87).
  const open = useAssistantStore((state) => state.open);
  const messages = useAssistantStore((state) => state.messages);
  const pendingAction = useAssistantStore((state) => state.pendingAction);
  const pendingActionKey = useAssistantStore((state) => state.pendingActionKey);
  const conversationKey = useAssistantStore((state) => state.conversationKey);
  const historyNotice = useAssistantStore((state) => state.historyNotice);
  const busy = useAssistantStore((state) => state.busy);
  const settings = useAssistantStore((state) => state.settings);
  const send = useAssistantStore((state) => state.send);
  const approve = useAssistantStore((state) => state.approve);
  const dismissAction = useAssistantStore((state) => state.dismissAction);

  // useHotelStore có ~14 field và loading bật/tắt ở gần như mọi lệnh ghi
  // trong app (qua beginAction/endAction dùng chung); panel chỉ đọc 3 field
  // dưới đây nên phải chọn riêng, không thì re-render theo cả 14.
  const activeTab = useHotelStore((state) => state.activeTab);
  const roomDetail = useHotelStore((state) => state.roomDetail);
  const roomChangeBookingId = useHotelStore((state) => state.roomChangeBookingId);

  const [draft, setDraft] = useState("");
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Lớp 3 của bốn lớp bảo vệ: chỉ vẽ thẻ khi nó được dựng ở CHÍNH hội thoại
  // đang mở. Lớp 4 (`approve()`) canh việc GHI và là lớp duy nhất nằm trên
  // đường tiền; lớp này canh việc VẼ. Không có nó thì lễ tân đang đọc hội thoại
  // B vẫn thấy thẻ nhận phòng của hội thoại A, bấm *Đồng ý*, và tất cả những gì
  // xảy ra là thẻ biến mất — không một lời giải thích, vì lớp 4 trả về im lặng.
  //
  // So bằng `conversationKey` chứ KHÔNG bằng `conversationId`: id database là
  // `null` khi ghi hỏng, và `null === null` sẽ khớp — đúng cái phải chặn.
  const showAction = pendingAction !== null && pendingActionKey === conversationKey;

  // Đồng hồ để thẻ tự chuyển sang trạng thái hết hạn mà không cần thao tác.
  // Bám vào thẻ ĐANG VẼ, không phải `pendingAction` trần: thẻ của hội thoại
  // khác không hiện thì cũng không có gì để đếm giờ.
  useEffect(() => {
    if (!showAction) return;
    const timer = setInterval(() => setNowMs(Date.now()), 10_000);
    return () => clearInterval(timer);
  }, [showAction]);

  if (!settings?.gate.ready || !open) return null;

  const context = buildScreenContext({ activeTab, roomDetail, roomChangeBookingId });
  const contextLabel = context.selectedRoomNumber
    ? `Đang xem: ${context.selectedRoomNumber}`
    : `Đang xem: ${context.route}`;

  // Gợi ý ở màn hình trống phải GỬI thẳng câu đó, không đi vòng qua `draft`:
  // điền hộ vào ô nhập rồi bắt bấm gửi lần nữa là biến một cú bấm thành hai mà
  // chẳng thêm thông tin gì.
  const sendMessage = (message: string) => {
    void send(message, context);
  };

  const onSubmitDraft = () => {
    const message = draft;
    setDraft("");
    sendMessage(message);
  };

  return (
    // Viền phải: panel là cột GIỮA, nên đường kẻ nằm ở mép giáp vùng nội dung.
    <aside
      aria-label="Trợ lý quầy"
      className="flex w-[380px] shrink-0 flex-col border-r border-slate-100 bg-white"
    >
      <header className="flex h-[88px] items-center px-5 text-sm font-semibold">Trợ lý quầy</header>

      <div className="flex flex-1 flex-col gap-3 overflow-y-auto px-5 pb-4">
        {/* Dòng nhắc khi mở lại hội thoại cũ. Có điều kiện — nhắc thường trực
            thì thành nhiễu, và nhiễu thường trực thì người ta thôi đọc.
            `role="status"` vừa đúng nghĩa (tin phụ trợ, không tới mức cảnh báo)
            vừa cho test khẳng định được "KHÔNG có viên nhắc nào": chỉ dò chữ
            thì một bản bọc-luôn-vẽ — viên nền vàng rỗng thường trực — vẫn lọt,
            vì jsdom không thấy nền. Cùng idiom với ProposedActionCard.tsx:76. */}
        {historyNotice && (
          <p
            role="status"
            aria-live="polite"
            className="rounded-xl bg-amber-50 px-3 py-2 text-[11px] text-amber-800"
          >
            {historyNotice}
          </p>
        )}

        {messages.length === 0 && <AssistantEmptyState onPick={sendMessage} />}

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

        {showAction && pendingAction && (
          <ProposedActionCard
            action={pendingAction}
            busy={busy}
            nowMs={nowMs}
            onApprove={approve}
            onRebuild={() => sendMessage("Tính lại thẻ nhận phòng vừa rồi.")}
            onDismiss={dismissAction}
          />
        )}
      </div>

      <div className="border-t border-slate-100">
        <AssistantComposer
          value={draft}
          contextLabel={contextLabel}
          busy={busy}
          onChange={setDraft}
          onSubmit={onSubmitDraft}
        />
      </div>
    </aside>
  );
}
