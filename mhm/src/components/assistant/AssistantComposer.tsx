import { ArrowUp } from "lucide-react";
import type { ChangeEvent, KeyboardEvent } from "react";

type AssistantComposerProps = {
  value: string;
  contextLabel: string;
  busy: boolean;
  onChange: (value: string) => void;
  onSubmit: () => void;
};

/// Một hộp chứa cả ô nhập, hàng công cụ và nút gửi — thay cho kiểu ô-nhập-cạnh-nút.
///
/// Airtable để "Tools" và ghim file ở hàng dưới. CapyInn không có hai thứ đó,
/// nên hàng dưới dùng cho nhãn ngữ cảnh: đúng vị trí, và là thông tin thật
/// thay vì nút chết.
export function AssistantComposer({
  value,
  contextLabel,
  busy,
  onChange,
  onSubmit,
}: AssistantComposerProps) {
  const canSend = !busy && value.trim().length > 0;

  /// Enter gửi, Shift+Enter xuống dòng.
  ///
  /// `canSend` canh cả phím lẫn nút: tắt nút mà để Enter đi thẳng là chừa một
  /// đường thứ hai vào cùng chỗ, và lượt gõ trong lúc `busy` sẽ chồng lên lượt
  /// đang chờ trả lời.
  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    if (canSend) onSubmit();
  };

  return (
    <div className="m-4 rounded-2xl border border-slate-200 bg-white p-3 focus-within:border-brand-primary">
      <textarea
        value={value}
        rows={2}
        onChange={(event: ChangeEvent<HTMLTextAreaElement>) => onChange(event.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Hỏi hoặc ra việc…"
        className="max-h-40 w-full resize-none border-0 bg-transparent text-sm outline-none"
      />
      <div className="mt-2 flex items-center justify-between gap-2">
        <span className="rounded-full border border-slate-200 px-2.5 py-1 text-[11px] text-brand-muted">
          {contextLabel}
        </span>
        <button
          type="button"
          onClick={onSubmit}
          disabled={!canSend}
          aria-label="Gửi tin nhắn"
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-brand-primary text-white disabled:bg-slate-200 disabled:text-slate-400"
        >
          <ArrowUp size={16} />
        </button>
      </div>
    </div>
  );
}
