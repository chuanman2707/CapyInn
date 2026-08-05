import { Trash2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { fmtDate } from "@/lib/format";
import type { AssistantConversationSummary } from "@/types/assistant";

/// Chuỗi phải gõ đúng thì nút xoá sạch mới bật.
///
/// So bằng `===` trên chuỗi thô: **không** `trim()`, **không** `toUpperCase()`,
/// **không** bỏ dấu. Hàng rào này tồn tại để gây khó — nó xoá bản duy nhất,
/// không hoàn tác, và chủ nhà đã chọn hệ thống không tự xoá nên đây là lối ra
/// duy nhất. Mỗi cách nới lỏng biến nó thành một ô nhập trang trí: chuẩn hoá bỏ
/// dấu cho "xoa het" đi lọt, mà "xoa het" là thứ gõ nhầm ra được.
const DELETE_ALL_PHRASE = "XOÁ HẾT";

const PHRASE_INPUT_ID = "assistant-delete-all-phrase";

type AssistantHistoryListProps = {
  conversations: AssistantConversationSummary[];
  /// Chỉ admin thấy hai đường xoá. Đây là **lớp lịch sự**, không phải hàng rào:
  /// `delete_assistant_conversation` và `delete_all_assistant_conversations`
  /// đều kiểm quyền phía Rust và trả `AUTH_FORBIDDEN` cho lễ tân. Giấu nút chỉ
  /// để lễ tân đừng bấm vào thứ chắc chắn bị từ chối.
  isAdmin: boolean;
  /// Đang chờ trả lời → khoá mọi dòng. Chuyển hội thoại giữa lúc câu trả lời
  /// đang bay về đẻ ra một mớ tình huống tranh chấp mà không đổi lại được gì.
  busy: boolean;
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
                  disabled={phrase !== DELETE_ALL_PHRASE}
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
