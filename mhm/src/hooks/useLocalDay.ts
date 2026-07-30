import { useEffect, useState } from "react";

import { localDateIso } from "@/lib/timelineSelection";

/** Nửa đêm kế tiếp theo giờ địa phương, tính từ `from`. */
function nextLocalMidnight(from: Date): Date {
    const midnight = new Date(from);
    midnight.setHours(24, 0, 0, 0);
    return midnight;
}

/**
 * Ngày địa phương hiện tại (`YYYY-MM-DD`), **tự cập nhật khi qua nửa đêm**.
 *
 * Sinh ra vì mỗi form tự chụp `new Date()` một lần rồi giữ nguyên. Một sheet mở
 * lúc 23:5x và bấm nhận phòng lúc 00:0x sẽ báo giá theo ngày hôm qua, trong khi
 * backend thu tiền từ `Local::now()` — tức hôm nay. Hai mốc khác nhau nghĩa là
 * tra `special_dates` khác nhau và phụ thu cuối tuần khác nhau, đúng cái bất biến
 * mà cả mạch báo giá này dựng lên để bảo vệ.
 *
 * Quầy lễ tân trực đêm không phải trường hợp biên: đó là ca làm việc.
 *
 * Hẹn đúng tới nửa đêm kế tiếp chứ không polling: một `setTimeout` mỗi ngày,
 * không có tick nào chạy không. `setHours(24, 0, 0, 0)` cho nửa đêm địa phương kể
 * cả ngày đổi giờ, vì nó đi qua đúng lịch địa phương thay vì cộng 86.400.000ms.
 */
export function useLocalDay(): string {
    const [day, setDay] = useState(() => localDateIso(new Date()));

    useEffect(() => {
        // +1s: hẹn *sau* mốc nửa đêm, không phải đúng mốc, để lần đọc lại chắc
        // chắn đã sang ngày mới kể cả khi timer nhả sớm vài ms.
        const delay = nextLocalMidnight(new Date()).getTime() - Date.now() + 1_000;
        const timer = window.setTimeout(() => setDay(localDateIso(new Date())), delay);

        return () => clearTimeout(timer);
        // Chạy lại sau mỗi lần đổi ngày để hẹn tiếp nửa đêm sau.
    }, [day]);

    return day;
}
