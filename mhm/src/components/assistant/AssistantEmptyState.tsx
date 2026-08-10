import { Sparkles } from "lucide-react";

/// Hằng số tĩnh, KHÔNG sinh từ dữ liệu và KHÔNG chứa số phòng cụ thể: gợi ý
/// kiểu "Khách phòng 201 còn nợ bao nhiêu?" sẽ sai ngay trên khách sạn không
/// có phòng 201, và sinh động từ dữ liệu thì phải thêm query chỉ để dựng ba
/// dòng chữ.
///
/// Câu thứ ba **không** dựng được thẻ ngay — `draft.rs` đòi `room_id`, danh
/// sách khách không rỗng và `nights >= 1` — nên trợ lý sẽ hỏi lại. Đó là chủ ý:
/// nó cho lễ tân thấy đường vào luồng nhận phòng tồn tại, không hứa rằng một
/// câu là xong.
export const SUGGESTIONS = [
  "Tối nay còn phòng nào trống?",
  "Hôm nay những phòng nào phải trả?",
  "Nhận phòng giúp tôi",
] as const;

export function AssistantEmptyState({ onPick }: { onPick: (text: string) => void }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-6 px-1 text-center">
      <Sparkles size={28} className="text-brand-primary/60" />
      <p className="text-base font-semibold">Cần em giúp gì?</p>
      <div className="flex w-full flex-col gap-2">
        {SUGGESTIONS.map((suggestion) => (
          <button
            key={suggestion}
            type="button"
            onClick={() => onPick(suggestion)}
            className="rounded-xl border border-slate-200 px-3 py-2 text-left text-sm text-brand-muted transition-colors hover:border-brand-primary hover:text-brand-text"
          >
            {suggestion}
          </button>
        ))}
      </div>
    </div>
  );
}
