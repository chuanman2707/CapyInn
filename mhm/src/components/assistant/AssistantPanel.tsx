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
import { actionKindCopy, type ScreenContext } from "@/types/assistant";

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
  const saveFailed = useAssistantStore((state) => state.saveFailed);
  const error = useAssistantStore((state) => state.error);
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

  // Hộp hỏi của lớp 1 phải chết theo mọi đường RÚT LUI, không chỉ theo hai nút
  // bên trong nó. `switchIntent` sống trong state của component, và component
  // này KHÔNG unmount khi thu panel — câu `return null` ngay dưới giữ nguyên
  // toàn bộ hook — nên không dọn tay thì hộp treo lại: mở panel lần sau, câu
  // hỏi cũ còn nguyên, và cú bấm *Bỏ thẻ và đi tiếp* vứt thẻ theo một ý định
  // người dùng đã bỏ dở từ lúc nào không rõ.
  //
  // Vế `!pendingAction` là đường thứ ba: bấm *Bỏ thẻ* ngay trên chính thẻ (nút
  // của `ProposedActionCard`, vẫn bấm được trong lúc hộp hỏi đang mở) làm thẻ
  // biến mất, và một hộp hỏi "Bỏ thẻ nhận phòng đang chờ?" về cái thẻ không còn
  // tồn tại là câu hỏi suông — nhưng trả lời "Bỏ thẻ và đi tiếp" cho nó thì vẫn
  // đổi hội thoại thật.
  //
  // Đây KHÔNG phải nới lỏng lớp 1: nó chỉ gỡ hộp hỏi ở đúng những lúc không còn
  // gì để mất hoặc người dùng đã rời đi. Đường mở hộp (`requestSwitch`) không
  // đổi, và `runSwitch` vẫn chỉ chạy từ nút đồng ý.
  useEffect(() => {
    if (!open || !pendingAction) setSwitchIntent(null);
  }, [open, pendingAction]);

  // Đổi người dùng phải dọn cả state của COMPONENT, không chỉ state của store.
  //
  // `resetForLogout()` với tới store, nhưng `draft` — câu đang gõ dở, mang tên
  // khách và số CCCD — và `view` sống trong component này. Và component này
  // **không unmount khi đăng xuất** ở cấu hình `app_lock_enabled = false`:
  // `AuthGate.tsx:26` chỉ thay MainShell bằng LoginScreen khi app lock BẬT; app
  // lock tắt thì AuthGate luôn render children. Cùng trục với lỗ hổng vòng
  // duyệt Task 6 đã bắt (lễ tân B duyệt được thẻ của lễ tân A) — chỉ khác là ở
  // tầng component chứ không phải tầng store, nên `resetForLogout()` không với
  // tới bằng bất kỳ cách nào.
  //
  // Bám vào ID người dùng chứ không vào `isAuthenticated`: với app lock tắt,
  // `isAuthenticated` có thể đứng yên ở `false` suốt vòng đời app, còn `user`
  // thì `logout()` LUÔN đặt về `null` ở cả hai cấu hình.
  //
  // `switchIntent` cố ý không dọn ở đây: effect ngay trên đã dọn nó, vì
  // `resetForLogout()` đặt `open: false`. Dọn lần hai là viết một no-op.
  const userId = useAuthStore((state) => state.user?.id ?? null);
  useEffect(() => {
    setDraft("");
    setView("chat");
  }, [userId]);

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
            onClick={() => {
              // Quay lại là RÚT LUI, không phải đồng ý. Không dọn `switchIntent`
              // ở đây thì hộp hỏi — vốn nằm ngoài máy chuyển view — đi THEO sang
              // khung chat, và cú bấm *Bỏ thẻ và đi tiếp* ở bên đó vứt thẻ **và**
              // mở đúng cái hội thoại người dùng vừa từ chối mở.
              setSwitchIntent(null);
              setView("chat");
            }}
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
            {/* CỬA THỨ TƯ mint khoá phiên, và là cửa duy nhất mở được SAU khi
                `busy` đã bật. Ba nút xoá ở `AssistantHistoryList` đã khoá theo
                `busy` vì cùng một lý do, nhưng hộp này mở từ lúc còn rảnh nên
                khoá ở nút *Hội thoại mới* không với tới: bấm *Hội thoại mới*
                lúc `busy=false` → hộp hiện → bấm *Đồng ý* trên thẻ → `check_in`
                bay đi, `busy=true`, hộp VẪN mở → bấm nút này → `startNewChat()`
                mint khoá phiên mới → lớp 4 vứt kết quả `check_in`, nhưng PHÒNG
                ĐÃ NHẬN THẬT. Đo được: `messages=[]`, `error=null`, `busy=false`
                — màn hình sạch trơn sau một lượt nhận phòng có thật.

                `disabled` ở đây giờ là lớp lịch sự, không phải hàng rào: hàng
                rào nằm trong `startNewChat()` (bất biến sở hữu `busy` ở
                `useAssistantStore.ts`), vì `disabled` chỉ lấy mẫu lúc render và
                cú bấm đã đi rồi thì không thu lại được. Bị từ chối thì store
                đặt `error`, và viên `role="alert"` ngay dưới vẽ nó ra.

                Nút *Ở lại* cố ý KHÔNG khoá: rút lui phải luôn mở, kể cả lúc
                bận, không thì `busy` treo một hộp hỏi không đường thoát. */}
            <Button
              size="sm"
              disabled={busy}
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

      {/* Lỗi của store. Nằm NGOÀI máy chuyển view vì **phần lớn** đường đặt
          `error` không đi qua khung chat: `loadConversations`,
          `openConversation` và hai lệnh xoá đều nổ trong lúc lễ tân đang đứng ở
          màn hình lịch sử; chỉ `send()` và `approve()` là của khung chat. Vẽ nó
          trong khung chat thôi là để y nguyên cái câm mà nó sinh ra để chữa —
          nặng nhất là `loadConversations` hỏng: danh sách rỗng, và panel nói
          "Chưa có hội thoại nào.", một câu SAI SỰ THẬT về dữ liệu khách.

          Cố ý KHÔNG đếm số chỗ ở đây. Bản trước viết "năm trong sáu chỗ" và con
          số đã trôi mất trong vòng vài commit (nay `set({… error …})` nhiều hơn
          gấp đôi) trong khi kết luận thì không đổi. Một con số trong chú thích
          là một thứ phải bảo trì mà không gì nhắc khi nó sai.

          `role="alert"` chứ không phải `role="status"` như `historyNotice`: đây
          là hỏng việc, không phải tin phụ trợ. Nó cũng khác `role="alertdialog"`
          của hộp lớp 1 (đo được: `getByRole("alert")` không khớp alertdialog),
          nên test hai thứ này không giẫm lên nhau.

          Đường `send()` hỏng vẽ hai lần — một bong bóng đỏ trong dòng hội thoại
          và một viên này — và đó là đánh đổi có chủ ý: bong bóng nằm trong vùng
          cuộn nên cuộn qua là mất, còn viên này ghim tại chỗ và được trình đọc
          màn hình xướng lên. */}
      {error && (
        <p
          role="alert"
          className="mx-5 mb-3 shrink-0 rounded-xl bg-red-50 px-3 py-2 text-xs text-red-700"
        >
          {error}
        </p>
      )}

      {view === "history" ? (
        <div className="flex-1 overflow-y-auto px-5 pb-4">
          <AssistantHistoryList
            conversations={conversations}
            isAdmin={isAdmin}
            busy={busy}
            hasPendingAction={pendingAction !== null}
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
                vì jsdom không thấy nền. Cùng idiom với viên `Cảnh báo từ PMS`
                trong `ProposedActionCard.tsx` — trỏ bằng tên, không bằng số
                dòng, vì số dòng trôi mà không gì nhắc. */}
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

            {/* Spec dòng 446-447. Nằm NGAY DƯỚI câu trả lời nó nói về, và nằm
                TRONG vùng cuộn — cố ý không ghim cứng như viên `error`.

                Hai lý do. (1) Nó nói về một LƯỢT cụ thể chứ không về trạng thái
                của cả panel; ghim nó lên đầu là tách nó khỏi thứ nó đang tố.
                (2) Phần cao cố định của panel đã là ~390px khi mọi thứ cùng
                hiện (header 88 + hộp lớp 1 ~120 + viên lỗi ~36 + composer 143),
                và vùng chat co về 0 là con đường đẩy ô nhập xuống dưới đáy. Thêm
                một viên `shrink-0` nữa là kéo ngưỡng ấy gần lại, mà jsdom không
                tính layout nên không test nào bắt được — đúng loại rủi ro chỉ QA
                tay mới thấy.

                `role="status"` chứ không `role="alert"` như viên `error`, và đó
                là một lựa chọn về nghĩa chứ không phải sao chép: `error` nghĩa
                là "việc anh vừa làm HỎNG, làm lại đi"; dòng này thì ngược lại —
                lượt chat đã XONG, câu trả lời đang nằm ngay trên nó, chỉ cái sổ
                là không giữ được. Spec dòng 422-427 chốt rõ "không chặn lượt
                chat, trợ lý vẫn trả lời", dẫn lại bài học `lib.rs:136-143`: tiện
                ích hỏng không được lấy mất công cụ của người dùng. `alert` là
                assertive — nó cắt ngang trình đọc màn hình đúng lúc câu trả lời
                đang được xướng lên, tức lấy mất đúng cái thứ lễ tân vừa hỏi.

                Bọc nằm TRONG câu điều kiện, không được bọc-luôn-vẽ-nội-dung-
                có-điều-kiện — bẫy mà `historyNotice` và `ProposedActionCard`
                đều đã dính: một viên nền vàng RỖNG thường trực vẫn làm mọi test
                dò chữ xanh, vì jsdom không nhìn thấy nền. `role="status"` là
                thứ cho test đếm được số viên đang có, tức khẳng định được
                "KHÔNG có viên nào" — `name` thì không dùng được, vai `status`
                không lấy tên truy cập từ nội dung (đã đo: `getByRole("status",
                { name })` không khớp ngay cả với `historyNotice`). */}
            {saveFailed && (
              <p
                role="status"
                aria-live="polite"
                className="rounded-xl bg-amber-50 px-3 py-2 text-[11px] text-amber-800"
              >
                Không lưu được hội thoại này.
              </p>
            )}

            {busy && <p className="text-xs text-brand-muted">Đang tra dữ liệu…</p>}

            {showAction && pendingAction && (
              <ProposedActionCard
                action={pendingAction}
                busy={busy}
                nowMs={nowMs}
                onApprove={approve}
                // Câu *Tính lại* đi theo LOẠI THẺ. Gửi "Tính lại thẻ nhận phòng
                // vừa rồi." cho một thẻ đặt phòng trước là tự tay bảo model dựng
                // một thẻ nhận phòng — tức đẩy nó về đúng cái búa nó đã đóng
                // nhầm, và lần này là do chính panel gợi ý chứ không phải do
                // model hiểu sai. Xem `ACTION_KIND_COPY` (`types/assistant.ts`).
                onRebuild={() => sendMessage(actionKindCopy(pendingAction.kind).rebuildPrompt)}
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
