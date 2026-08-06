import { Trash2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { fmtDate } from "@/lib/format";
import { DELETE_ALL_PHRASE, isDeleteAllPhrase } from "@/types/assistant";
import type { AssistantConversationSummary } from "@/types/assistant";

/// Cửa xoá sạch thứ hai (Cài đặt → Trợ lý quầy) dùng chung chuỗi và chung luật
/// so sánh — xem `isDeleteAllPhrase` trong `types/assistant.ts`. Id của ô nhập
/// thì KHÔNG dùng chung: hai ô cùng id trên một trang là một id trùng, và
/// `getByLabelText` sẽ trỏ nhầm.
const PHRASE_INPUT_ID = "assistant-delete-all-phrase";

type AssistantHistoryListProps = {
  conversations: AssistantConversationSummary[];
  /// Chỉ admin thấy hai đường xoá. Đây là **lớp lịch sự**, không phải hàng rào:
  /// `delete_assistant_conversation` và `delete_all_assistant_conversations`
  /// đều kiểm quyền phía Rust và trả `AUTH_FORBIDDEN` cho lễ tân. Giấu nút chỉ
  /// để lễ tân đừng bấm vào thứ chắc chắn bị từ chối.
  isAdmin: boolean;
  /// Đang chờ trả lời → khoá mọi dòng **và cả ba nút xoá**. Chuyển hội thoại
  /// giữa lúc câu trả lời đang bay về đẻ ra một mớ tình huống tranh chấp mà
  /// không đổi lại được gì; xoá thì tệ hơn một bậc, vì trước bản vá cả hai lệnh
  /// xoá đều gọi `startNewChat()` **vô điều kiện** → mint khoá phiên mới → kết
  /// quả `check_in` đang bay về bị store bỏ (đúng thiết kế, lớp 4), nhưng phòng
  /// thì ĐÃ nhận thật. Không mất tiền, mất tin.
  ///
  /// Hàng rào thật giờ nằm ở TẦNG STORE (xem bất biến sở hữu `busy` trong
  /// `useAssistantStore.ts`): cả bốn đường mint đều tự từ chối khi đang bận,
  /// trừ đăng xuất. Prop này còn lại đúng vai lịch sự — đừng mời người ta bấm
  /// một thứ chắc chắn bị từ chối — chứ không còn là thứ duy nhất đứng giữa cú
  /// bấm và cú mint. Nó vốn không làm nổi việc đó: `disabled` là **lấy mẫu lúc
  /// render**, cú bấm đã đi rồi thì không thu lại được.
  busy: boolean;
  /// Có thẻ nào đang chờ duyệt hay không — bất kể loại. Chỉ dùng để CẢNH BÁO trong hộp
  /// xoá sạch, **không** dựng thêm một cửa hỏi kiểu lớp 1: spec dòng 474-475
  /// liệt kê đúng hai hành vi phải hỏi trước — bấm *hội thoại mới* và mở một hội
  /// thoại từ lịch sử — còn xoá là hành động khác và đã có hàng rào riêng
  /// (gõ đúng `XOÁ HẾT`).
  hasPendingAction: boolean;
  onOpen: (conversationId: string) => void;
  onDelete: (conversationId: string) => void;
  onDeleteAll: () => void;
};

/// Danh sách lịch sử: 50 hội thoại mới nhất, **không phân trang**.
///
/// Trần 50 chốt cứng ở tầng query phía Rust (`conversation_queries`), không có
/// tham số nào để tầng này vượt qua — nên ở đây cũng không có nút "tải thêm":
/// vẽ một cái nút không bao giờ lấy được dòng thứ 51 là hứa suông.
///
/// Lễ tân chỉ nhận hội thoại của chính mình, admin nhận của mọi người — việc
/// lọc nằm phía Rust, component này chỉ vẽ đúng thứ được đưa cho.
export function AssistantHistoryList({
  conversations,
  isAdmin,
  busy,
  hasPendingAction,
  onOpen,
  onDelete,
  onDeleteAll,
}: AssistantHistoryListProps) {
  const [deleting, setDeleting] = useState<AssistantConversationSummary | null>(null);
  const [confirmingAll, setConfirmingAll] = useState(false);
  const [phrase, setPhrase] = useState("");

  const closeDeleteAll = () => {
    setConfirmingAll(false);
    // Dọn luôn chữ đã gõ: để nguyên thì lần mở sau hộp đã sẵn ở trạng thái bật
    // nút, và cú bấm thứ hai không còn phải đi qua hàng rào nào.
    setPhrase("");
  };

  return (
    <div className="flex flex-col gap-3">
      {deleting && (
        <div className="space-y-3 rounded-2xl border border-red-200 bg-red-50 p-4">
          <p className="text-sm font-semibold">Xoá hội thoại “{deleting.title}”?</p>
          <p className="text-xs text-red-700">
            Xoá cả tin nhắn bên trong và không hoàn tác được.
          </p>
          <div className="flex gap-2">
            <Button
              size="sm"
              className="bg-red-600 text-white"
              disabled={busy}
              onClick={() => {
                onDelete(deleting.id);
                setDeleting(null);
              }}
            >
              Xoá hội thoại này
            </Button>
            <Button size="sm" variant="outline" onClick={() => setDeleting(null)}>
              Giữ lại
            </Button>
          </div>
        </div>
      )}

      {conversations.length === 0 ? (
        <p className="py-6 text-center text-sm text-brand-muted">Chưa có hội thoại nào.</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {conversations.map((conversation) => (
            <li key={conversation.id} className="flex items-center gap-1">
              <button
                type="button"
                disabled={busy}
                onClick={() => onOpen(conversation.id)}
                className="min-w-0 flex-1 rounded-xl px-2 py-2 text-left hover:bg-slate-50 disabled:opacity-50 disabled:hover:bg-transparent"
              >
                <span className="block truncate text-sm">{conversation.title}</span>
                <span className="mt-0.5 flex items-center gap-1.5 text-[11px] text-brand-muted">
                  {/* Tên người tạo CHỈ hiện với admin, và hiện ngay trên dòng của
                      người đó: danh sách của admin trộn hội thoại của mọi người
                      nên không có nó thì admin bấm xoá mà không biết đang xoá
                      của ai. Lễ tân chỉ thấy hội thoại của mình — cột này thành
                      nhiễu. */}
                  {isAdmin && (
                    <span className="font-medium text-brand-text">{conversation.user_name}</span>
                  )}
                  <span>{fmtDate(conversation.updated_at)}</span>
                </span>
              </button>

              {isAdmin && (
                <button
                  type="button"
                  disabled={busy}
                  aria-label={`Xoá hội thoại ${conversation.title}`}
                  onClick={() => setDeleting(conversation)}
                  className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-brand-muted hover:bg-red-50 hover:text-red-600 disabled:opacity-50"
                >
                  <Trash2 size={14} />
                </button>
              )}
            </li>
          ))}
        </ul>
      )}

      {/* Không có hội thoại nào thì cũng không mời ai xoá sạch. */}
      {isAdmin && conversations.length > 0 && (
        <div className="border-t border-slate-100 pt-3">
          {confirmingAll ? (
            <div className="space-y-3 rounded-2xl border border-red-200 bg-red-50 p-4">
              <p className="text-sm font-semibold">Xoá sạch toàn bộ hội thoại?</p>
              <p className="text-xs text-red-700">
                Không hoàn tác. Ngoài bản sao lưu ở Data &amp; Backup thì đây là bản duy nhất.
              </p>

              {/* Xoá sạch dọn luôn phiên đang mở, nên thẻ đang chờ mất theo —
                  kể cả ở ca `conversationId === null` (ghi sổ hỏng, hội thoại
                  chưa hề vào sổ), tức mất thẻ vì một lệnh xoá không xoá nổi một
                  dòng nào của chính hội thoại đó. Chỉ một câu, có điều kiện:
                  câu cảnh báo thường trực là câu người ta thôi đọc.

                  TRUNG TÍNH với loại thẻ: `hasPendingAction` là một `boolean`,
                  chỗ này không biết thẻ đang treo là nhận phòng, đặt phòng hay
                  ghi bù — và gọi bừa nó là "thẻ nhận phòng" như bản trước thì
                  hai phần ba số lần là nói sai về thứ sắp mất.

                  Cố ý KHÔNG đặt câu này trong hộp xoá từng dòng: ở đó
                  `deleteConversation` chỉ dọn phiên khi xoá đúng hội thoại đang
                  mở, nên muốn cảnh báo đúng thì phải biết thêm hội thoại nào đang
                  mở — một prop nữa để đổi lấy một cảnh báo sai phần lớn thời
                  gian. */}
              {hasPendingAction && (
                <p className="text-xs font-medium text-red-700">
                  Thẻ đang chờ duyệt cũng sẽ mất.
                </p>
              )}
              <label htmlFor={PHRASE_INPUT_ID} className="block text-xs text-red-700">
                Gõ {DELETE_ALL_PHRASE} để xác nhận
              </label>
              <input
                id={PHRASE_INPUT_ID}
                type="text"
                value={phrase}
                onChange={(event) => setPhrase(event.target.value)}
                className="w-full rounded-xl border border-red-200 bg-white px-3 py-2 text-sm outline-none focus:border-red-400"
              />
              <div className="flex gap-2">
                <Button
                  size="sm"
                  className="bg-red-600 text-white"
                  disabled={busy || !isDeleteAllPhrase(phrase)}
                  onClick={() => {
                    onDeleteAll();
                    closeDeleteAll();
                  }}
                >
                  Xoá vĩnh viễn
                </Button>
                <Button size="sm" variant="outline" onClick={closeDeleteAll}>
                  Huỷ
                </Button>
              </div>
            </div>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              className="text-red-600"
              disabled={busy}
              onClick={() => setConfirmingAll(true)}
            >
              Xoá tất cả hội thoại
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
