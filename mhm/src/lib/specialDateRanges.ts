/**
 * Dưới DB, `special_dates` là một dòng cho một ngày. Chủ nhà thì nghĩ theo kỳ
 * nghỉ. Module này bắc cầu giữa hai cách nhìn ấy, và chỉ làm việc đó.
 */

export type SpecialDateRow = {
    id: string;
    date: string;
    label: string;
    uplift_pct: number;
};

export type SpecialDateRange = {
    from: string;
    to: string;
    days: number;
    label: string;
    uplift_pct: number;
    /** Mọi ngày trong cụm — thứ phải gửi đi khi xoá. */
    dates: string[];
};

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Ngày kế tiếp của một `YYYY-MM-DD`.
 *
 * Tính trên UTC rồi cắt mười ký tự đầu. Tuyệt đối không dùng
 * `new Date("2026-02-14T00:00:00")` — nó phân tích theo giờ địa phương rồi in
 * ra UTC, và ở UTC+7 thì lệch mất một ngày.
 *
 * Hàm này cố ý để cục bộ chứ không tách sang `src/lib/`:
 * `ReservationSheet.tsx` đã có một `addDays` nhận và trả `string`, còn nhánh
 * `refactor/pricing-preview-honesty` đang thêm một `addDays` nhận và trả
 * `Date`. Dựng thêm một module dùng chung lúc này là chuốc lấy đụng độ trùng
 * tên với kiểu không tương thích.
 */
function nextDay(date: string): string {
    const [year, month, day] = date.split("-").map(Number);
    return new Date(Date.UTC(year, month - 1, day + 1)).toISOString().slice(0, 10);
}

/**
 * Gom ngày liền nhau, cùng nhãn, cùng mức thành một khoảng.
 *
 * Đây là suy đoán chứ không phải dữ liệu: hai kỳ khác nhau tình cờ liền ngày,
 * cùng nhãn, cùng mức sẽ hiện thành một dòng. Vô hại — xoá cụm ấy đúng là xoá
 * chừng đó ngày — nhưng đừng coi đó là lỗi.
 */
export function groupSpecialDates(rows: SpecialDateRow[]): SpecialDateRange[] {
    const sorted = rows
        .filter((row) => DATE_ONLY.test(row.date))
        .slice()
        .sort((left, right) => left.date.localeCompare(right.date));

    const ranges: SpecialDateRange[] = [];
    for (const row of sorted) {
        const open = ranges[ranges.length - 1];
        const joinsOpenRange =
            open !== undefined &&
            open.label === row.label &&
            open.uplift_pct === row.uplift_pct &&
            nextDay(open.to) === row.date;

        if (open !== undefined && joinsOpenRange) {
            open.to = row.date;
            open.days += 1;
            open.dates.push(row.date);
        } else {
            ranges.push({
                from: row.date,
                to: row.date,
                days: 1,
                label: row.label,
                uplift_pct: row.uplift_pct,
                dates: [row.date],
            });
        }
    }

    return ranges;
}

/**
 * Những ngày đã khai mà một khoảng mới sẽ ghi đè lên.
 *
 * `exclude` là các ngày của chính cụm đang sửa — chúng không phải là trùng.
 * So sánh bằng chuỗi vì `YYYY-MM-DD` xếp theo từ điển đúng bằng xếp theo thời
 * gian.
 */
export function overlappingDates(
    rows: SpecialDateRow[],
    from: string,
    to: string,
    exclude: string[] = [],
): SpecialDateRow[] {
    const excluded = new Set(exclude);

    return rows
        .filter(
            (row) =>
                DATE_ONLY.test(row.date) &&
                !excluded.has(row.date) &&
                row.date >= from &&
                row.date <= to,
        )
        .sort((left, right) => left.date.localeCompare(right.date));
}
