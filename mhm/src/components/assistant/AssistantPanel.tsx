import { ArrowLeft, History, PanelRightClose, SquarePen } from "lucide-react";
import { useEffect, useState } from "react";

import { AssistantComposer } from "@/components/assistant/AssistantComposer";
import { AssistantEmptyState } from "@/components/assistant/AssistantEmptyState";
import { AssistantHistoryList } from "@/components/assistant/AssistantHistoryList";
import { ProposedActionCard } from "@/components/assistant/ProposedActionCard";
import { Button } from "@/components/ui/button";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { useAuthStore } from "@/stores/useAuthStore";
import { useHotelStore } from "@/stores/useHotelStore";
import type { ScreenContext } from "@/types/assistant";

/// Việc lễ tân định làm khi bấm *hội thoại mới* hoặc bấm một dòng trong lịch
/// sử. Giữ lại nguyên vẹn để chạy tiếp **sau** khi hộp hỏi được đồng ý — xem
/// lớp 1 bên dưới.
type SwitchIntent = { kind: "new" } | { kind: "open"; conversationId: string };

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
  const conversations = useAssistantStore((state) => state.conversations);
  const conversationId = useAssistantStore((state) => state.conversationId);
  const busy = useAssistantStore((state) => state.busy);
  const settings = useAssistantStore((state) => state.settings);
  const send = useAssistantStore((state) => state.send);
  const approve = useAssistantStore((state) => state.approve);
  const dismissAction = useAssistantStore((state) => state.dismissAction);
  const togglePanel = useAssistantStore((state) => state.togglePanel);
  const startNewChat = useAssistantStore((state) => state.startNewChat);
  const openConversation = useAssistantStore((state) => state.openConversation);
  const loadConversations = useAssistantStore((state) => state.loadConversations);
  const deleteConversation = useAssistantStore((state) => state.deleteConversation);
  const deleteAllConversations = useAssistantStore((state) => state.deleteAllConversations);

  // Chỉ admin thấy hai đường xoá. Đây là lớp lịch sự chứ **không** phải hàng
  // rào: cả hai lệnh xoá kiểm quyền phía Rust và trả `AUTH_FORBIDDEN` cho lễ
  // tân, còn store thì cố ý không dọn gì khi bị từ chối.
  const isAdmin = useAuthStore((state) => state.user?.role === "admin");

  // useHotelStore có ~14 field và loading bật/tắt ở gần như mọi lệnh ghi
  // trong app (qua beginAction/endAction dùng chung); panel chỉ đọc 3 field
  // dưới đây nên phải chọn riêng, không thì re-render theo cả 14.
  const activeTab = useHotelStore((state) => state.activeTab);
  const roomDetail = useHotelStore((state) => state.roomDetail);
  const roomChangeBookingId = useHotelStore((state) => state.roomChangeBookingId);

  const [draft, setDraft] = useState("");
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [view, setView] = useState<"chat" | "history">("chat");
  const [switchIntent, setSwitchIntent] = useState<SwitchIntent | null>(null);

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

  /// Đổi hội thoại thật sự. Chỉ gọi từ `requestSwitch` hoặc từ nút đồng ý của
  /// hộp hỏi — không gọi thẳng từ chỗ nào khác, kẻo mở thêm một cửa vòng qua
  /// lớp 1.
  const runSwitch = (intent: SwitchIntent) => {
    if (intent.kind === "new") {
      startNewChat();
    } else {
      void openConversation(intent.conversationId);
    }
    setView("chat");
  };

  // ── LỚP 1 của bốn lớp bảo vệ `pendingAction` ────────────────────────────
  //
  // Chặn **ở nguồn**. Đúng hai đường đổi hội thoại tồn tại trong panel — bấm
  // *hội thoại mới* và bấm một dòng trong lịch sử — và cả hai đều nằm sau hàm
  // này. Cả hai cùng dẫn tới `emptySession()` (lớp 2), tức dọn sạch
  // `pendingAction` + `pendingActionKey`. Không hỏi thì lễ tân mất cái thẻ nhận
  // phòng đang chờ duyệt mà không được nói một lời, và không dựng lại được:
  // hàng `action` trong sổ chỉ là CHỮ tóm tắt, không mang `payload`.
  //
  // Điều kiện là `pendingAction !== null` chứ không phải `showAction` — spec
  // dòng 474 viết đúng như thế, và đây là vế rộng hơn: hỏi thừa một lần thì
  // phiền, vứt nhầm một thẻ thì mất việc đã làm.
  //
  // Lớp này chỉ **hỏi**. Nó không thay lớp 3 (panel không vẽ thẻ ở sai hội
  // thoại) và tuyệt đối không thay lớp 4 (`approve()` không duyệt ở sai hội
  // thoại) — lớp 4 là lớp duy nhất nằm trên đường tiền, và bốn lớp trùng lặp có
  // chủ ý. Lớp 1 là lớp *lịch sự*; lớp 4 là lớp *giữ tiền*.
  const requestSwitch = (intent: SwitchIntent) => {
    if (pendingAction) {
      setSwitchIntent(intent);
      return;
    }
    runSwitch(intent);
  };

  const openHistory = () => {
    setView("history");
    // Tải lại mỗi lần mở: danh sách mang `updated_at` và tiêu đề, cả hai đổi
    // theo từng lượt chat. Trần 50 chốt cứng ở backend nên không có gì để phân
    // trang, chỉ một cú gọi.
    void loadConversations();
  };

  // Tên hội thoại đang mở, tra từ chính danh sách lịch sử — chỗ duy nhất
  // frontend có tiêu đề (backend đặt tên từ câu hỏi đầu tiên). Chưa mở lịch sử
  // lần nào thì `conversations` rỗng và rơi về "Hội thoại mới"; đó là nhãn đúng
  // cho một hội thoại chưa có tên, không phải chỗ thiếu dữ liệu.
  const openTitle = conversations.find((item) => item.id === conversationId)?.title;

  return (
    // Viền phải: panel là cột GIỮA, nên đường kẻ nằm ở mép giáp vùng nội dung.
    <aside
      aria-label="Trợ lý quầy"
      className="flex w-[380px] shrink-0 flex-col border-r border-slate-100 bg-white"
    >
      <header className="flex h-[88px] shrink-0 items-center gap-1 px-5">
        {view === "history" && (
          <button
            type="button"
            aria-label="Quay lại hội thoại"
            onClick={() => setView("chat")}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-brand-muted hover:bg-slate-50"
          >
            <ArrowLeft size={16} />
          </button>
        )}

        <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">
          {view === "history" ? "Lịch sử" : (openTitle ?? "Hội thoại mới")}
        </h2>

        {/* Hai nút này đổi hội thoại, nên `busy` khoá cả hai: chuyển hội thoại
            giữa lúc câu trả lời đang bay về đẻ ra một mớ tranh chấp mà không
            đổi lại được gì. Ở màn hình lịch sử thì chúng nhường chỗ cho nút
            quay lại — header "đổi thành Lịch sử", không cộng dồn. */}
        {view === "chat" && (
          <>
            <button
              type="button"
              aria-label="Hội thoại mới"
              disabled={busy}
              onClick={() => requestSwitch({ kind: "new" })}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-brand-muted hover:bg-slate-50 disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <SquarePen size={16} />
            </button>
            <button
              type="button"
              aria-label="Lịch sử"
              disabled={busy}
              onClick={openHistory}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-brand-muted hover:bg-slate-50 disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <History size={16} />
            </button>
          </>
        )}

        <button
          type="button"
          aria-label="Thu trợ lý"
          onClick={togglePanel}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-brand-muted hover:bg-slate-50"
        >
          <PanelRightClose size={16} />
        </button>
      </header>

      {/* Hộp hỏi của lớp 1. Nằm ngoài máy chuyển view vì nó phải với tới được
          từ CẢ HAI đường: nút *hội thoại mới* ở khung chat, và một dòng bấm
          được ở màn hình lịch sử. */}
      {switchIntent && (
        <div
          role="alertdialog"
          aria-label="Bỏ thẻ nhận phòng đang chờ?"
          className="mx-5 mb-3 shrink-0 space-y-3 rounded-2xl border border-amber-300 bg-amber-50 p-4"
        >
          <p className="text-sm font-semibold">Bỏ thẻ nhận phòng đang chờ?</p>
          <p className="text-xs text-amber-800">
            Đổi hội thoại là thẻ mất, và phải nhờ trợ lý tính lại từ đầu.
          </p>
          <div className="flex gap-2">
            <Button
              size="sm"
              onClick={() => {
                const intent = switchIntent;
                setSwitchIntent(null);
                runSwitch(intent);
              }}
            >
              Bỏ thẻ và đi tiếp
            </Button>
            <Button size="sm" variant="outline" onClick={() => setSwitchIntent(null)}>
              Ở lại
            </Button>
          </div>
        </div>
      )}

      {view === "history" ? (
        <div className="flex-1 overflow-y-auto px-5 pb-4">
          <AssistantHistoryList
            conversations={conversations}
            isAdmin={isAdmin}
            busy={busy}
            onOpen={(id) => requestSwitch({ kind: "open", conversationId: id })}
            onDelete={(id) => void deleteConversation(id)}
            onDeleteAll={() => void deleteAllConversations()}
          />
        </div>
      ) : (
        <>
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
        </>
      )}
    </aside>
  );
}
