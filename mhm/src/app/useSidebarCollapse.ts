import { useCallback, useEffect, useRef, useState } from "react";

const SIDEBAR_COLLAPSED_KEY = "sidebar-collapsed";
const NARROW_WIDTH = 1200;

/// Ba nguồn, cố ý không trộn:
///
/// - `userCollapsed` — thiết lập của lễ tân, nằm trên đĩa (localStorage)
/// - `autoReasons` — máy tự đặt: trợ lý đang mở, hoặc cửa sổ hẹp. Chỉ sống
///   trong phiên, và hai lý do này độc lập nhau chứ không gộp một cờ
/// - `userOverride` — lễ tân bấm ngược lại một lý do tự động. Chỉ sống trong
///   phiên, và tự hết hiệu lực khi lý do tự động cuối cùng tắt
///
/// Bệnh cũ: đường tự động (`resize`) ghi thẳng `"true"` vào localStorage, nên
/// máy tự thu một lần là thiết lập của lễ tân mất vĩnh viễn — phóng cửa sổ to
/// lại, thanh vẫn thu, và mọi lần khởi động sau vẫn thu. Nó ra đời vì trạng
/// thái tự động và thiết lập tay dùng chung đúng một biến `collapsed`; tách ba
/// nguồn ra chính là cách chặn nó quay lại. Sau bản này, cả file chỉ còn ĐÚNG
/// MỘT câu ghi xuống đĩa, nằm ở nhánh không-có-lý-do-tự-động của
/// `toggleCollapse` — grep tên hàm ghi trong file này phải ra đúng một dòng.
export function useSidebarCollapse(assistantOpen: boolean) {
  const [userCollapsed, setUserCollapsed] = useState(
    () => localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true",
  );
  const [narrow, setNarrow] = useState(() => window.innerWidth < NARROW_WIDTH);
  const [userOverride, setUserOverride] = useState<boolean | null>(null);

  useEffect(() => {
    // Chỉ đặt `narrow`. KHÔNG đụng `userOverride` — handler này bắn theo mọi sự
    // kiện resize, nên nếu nó xoá override thì lễ tân bấm mở rộng xong kéo cửa
    // sổ một cái là mất.
    const handleResize = () => setNarrow(window.innerWidth < NARROW_WIDTH);
    window.addEventListener("resize", handleResize);
    handleResize();
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const autoReasons = { assistant: assistantOpen, narrow };
  const hasAutoReason = autoReasons.assistant || autoReasons.narrow;
  const collapsed = userOverride ?? (userCollapsed || hasAutoReason);

  // Mọi lý do tự động tắt hết thì thả override, để lần sau đọc lại đúng thiết
  // lập thật của lễ tân — và để lần mở trợ lý kế tiếp lại thu thanh như thường,
  // chứ không sống nhờ một cú bấm từ lần trước.
  const previousAuto = useRef(hasAutoReason);
  useEffect(() => {
    if (previousAuto.current && !hasAutoReason) {
      setUserOverride(null);
    }
    previousAuto.current = hasAutoReason;
  }, [hasAutoReason]);

  const toggleCollapse = useCallback(() => {
    if (hasAutoReason) {
      // Đang có lý do tự động → chỉ đặt override, nghịch với giá trị ĐANG HIỂN
      // THỊ (không phải nghịch với `userCollapsed`), và không ghi đĩa.
      setUserOverride(!collapsed);
      return;
    }
    setUserCollapsed((current) => {
      const next = !current;
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next));
      return next;
    });
  }, [collapsed, hasAutoReason]);

  return { collapsed, toggleCollapse };
}
