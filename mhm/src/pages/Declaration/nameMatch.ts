/**
 * Xếp hạng gợi ý ghép danh tính ↔ lượt lưu trú (spec §7).
 *
 * Chỉ để SẮP THỨ TỰ. Không bao giờ tự chọn giúp: tên trong CapyInn có thể là
 * "Andrei" trong khi tên trên hộ chiếu là "ZOLOCHEVSKAIA VERONIKA" — thuật toán
 * nào tự tin ghép được cặp đó là thuật toán sẽ ghép sai cặp khác.
 */

export function normaliseName(s: string): string[] {
  return s
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/đ/gi, "d")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
}

/** Tỉ lệ token của `b` xuất hiện trong `a`. 0 khi `b` rỗng. */
export function nameScore(a: string, b: string): number {
  const ta = new Set(normaliseName(a));
  const tb = normaliseName(b);
  if (tb.length === 0) return 0;
  return tb.filter((t) => ta.has(t)).length / tb.length;
}
