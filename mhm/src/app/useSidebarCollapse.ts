import { useCallback, useEffect, useState } from "react";

const SIDEBAR_COLLAPSED_KEY = "sidebar-collapsed";

export function useSidebarCollapse() {
  const [collapsed, setCollapsed] = useState(() => {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true";
  });

  useEffect(() => {
    const handleResize = () => {
      if (window.innerWidth < 1200) {
        setCollapsed((current) => {
          if (current) {
            return current;
          }

          localStorage.setItem(SIDEBAR_COLLAPSED_KEY, "true");
          return true;
        });
      }
    };

    window.addEventListener("resize", handleResize);
    handleResize();

    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const toggleCollapse = useCallback(() => {
    setCollapsed((current) => {
      const next = !current;
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next));
      return next;
    });
  }, []);

  return { collapsed, toggleCollapse };
}
