import { ArrowUp } from "lucide-react";
import { useLayoutEffect, useRef } from "react";
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
  const boxRef = useRef<HTMLTextAreaElement>(null);
  const canSend = !busy && value.trim().length > 0;

  /// Hộp cao theo nội dung tới trần `max-h-40` rồi mới cuộn — đúng hình spec vẽ
  /// ("← textarea, cao theo nội dung").
  ///
  /// Không có đoạn này thì `rows={2}` ghim chiều cao ở ~2 dòng và `max-h-40`
  /// thành class chết: trần 160px không bao giờ chạm tới vì chẳng có gì đẩy
  /// chiều cao lên. Một trần giới hạn chỉ có nghĩa khi có thứ gì đó dâng lên.
  ///
  /// Phải trả `height` về "auto" TRƯỚC khi đo. `scrollHeight` của một ô đang bị
  /// `style.height` ghim không bao giờ nhỏ hơn chiều cao đang ghim, nên bỏ bước
  /// reset thì hộp chỉ phình ra mà không bao giờ co lại lúc lễ tân xoá bớt chữ.
  useLayoutEffect(() => {
    const box = boxRef.current;
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${box.scrollHeight}px`;
  }, [value]);

  /// Enter gửi, Shift+Enter xuống dòng.
  ///
  /// `canSend` canh cả phím lẫn nút: tắt nút mà để Enter đi thẳng là chừa một
  /// đường thứ hai vào cùng chỗ, và lượt gõ trong lúc `busy` sẽ chồng lên lượt
  /// đang chờ trả lời.
  ///
  /// Bỏ qua Enter khi bộ gõ đang dựng chữ, và để nó đi tiếp (không
  /// `preventDefault`). Lễ tân gõ Telex bấm Enter để CHỐT dấu chứ không phải để
  /// gửi; coi cú đó là "gửi" thì câu bay đi thiếu đúng ký tự cuối, mà với câu
  /// dựng thẻ nhận phòng thì đó là một lượt sai gửi lên nhà cung cấp AI. Bản
  /// `<form>` + `<input>` cũ được trình duyệt che hộ — implicit submission bỏ
  /// qua cú Enter chốt IME; bắt `keydown` bằng tay thì không còn cái che đó.
  ///
  /// `keyCode === 229` là cờ dự phòng: quy ước cũ báo "phím này đang đi qua bộ
  /// gõ". Trên Chromium nó thừa vì `isComposing` đã bật sẵn; giữ lại cho engine
  /// nào chưa dựng `isComposing` đúng. Không phím thật nào mang keyCode 229 nên
  /// nó không thể chặn nhầm một cú Enter thường.
  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey) return;
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    event.preventDefault();
    if (canSend) onSubmit();
  };

  return (
    <div className="m-4 rounded-2xl border border-slate-200 bg-white p-3 focus-within:border-brand-primary">
      <textarea
        ref={boxRef}
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
