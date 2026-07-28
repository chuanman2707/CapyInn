import { useState, type ButtonHTMLAttributes, type HTMLAttributes } from "react";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";
import { clearMockResponses, setMockResponses } from "./__mocks__/tauri-core";
import { resetEventMocks } from "./__mocks__/tauri-event";
import { useAuthStore } from "./stores/useAuthStore";
import { useHotelStore } from "./stores/useHotelStore";

// FINDING D: `MainShell` từng render trang Khai báo tạm trú bằng
// `{activeTab === "declaration" && <Declaration />}` — rời tab là unmount
// toàn bộ cây, xóa sạch thẻ vừa quét chưa lưu và form đang mở dở không một
// tiếng báo. Test này thay `Declaration` thật bằng một stub có state nội bộ
// (giống thẻ chưa lưu của DropZone) để xác nhận: chuyển sang tab khác rồi
// quay lại KHÔNG unmount trang — state phải còn nguyên, không reset về 0.
let declarationMountCount = 0;
function DeclarationStub() {
  useState(() => {
    declarationMountCount += 1;
    return null;
  });
  const [unsavedCards, setUnsavedCards] = useState(0);
  return (
    <div>
      <p>Declaration stub — unsaved: {unsavedCards}</p>
      <button onClick={() => setUnsavedCards((n) => n + 1)}>Thả ảnh giấy tờ (giả lập)</button>
    </div>
  );
}

vi.mock("./pages/Declaration", () => ({ default: () => <DeclarationStub /> }));
vi.mock("./pages/Dashboard", () => ({ default: () => <div>Dashboard page</div> }));
vi.mock("./pages/Rooms", () => ({ default: () => <div>Rooms page</div> }));
vi.mock("./pages/Reservations", () => ({ default: () => <div>Reservations page</div> }));
vi.mock("./pages/Guests", () => ({ default: () => <div>Guests page</div> }));
vi.mock("./pages/Housekeeping", () => ({ default: () => <div>Housekeeping page</div> }));
vi.mock("./pages/Analytics", () => ({ default: () => <div>Analytics page</div> }));
vi.mock("./pages/settings", () => ({ default: () => <div>Settings page</div> }));
vi.mock("./pages/NightAudit", () => ({ default: () => <div>Night Audit page</div> }));
vi.mock("@/pages/LoginScreen", () => ({ default: () => <div>Login page</div> }));
vi.mock("@/pages/onboarding", () => ({ default: () => <div>Onboarding page</div> }));
vi.mock("./components/CheckinSheet", () => ({ default: () => null }));
vi.mock("./components/GroupCheckinSheet", () => ({ default: () => null }));
vi.mock("./pages/GroupManagement", () => ({ default: () => <div>Group page</div> }));
vi.mock("./components/AppLogo", () => ({ default: () => <div>Logo</div> }));
vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => <div {...props}>{children}</div>,
}));
vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn() }),
  Toaster: () => <div data-testid="toaster" />,
}));

async function renderReadyApp() {
  render(<App />);
  await act(async () => {
    await Promise.resolve();
  });
}

describe("MainShell keeps the declaration tab alive across tab switches", () => {
  beforeEach(() => {
    declarationMountCount = 0;
    clearMockResponses();
    resetEventMocks();
    vi.clearAllMocks();

    useHotelStore.setState({
      rooms: [],
      stats: null,
      roomDetail: null,
      activeTab: "dashboard",
      housekeepingTasks: [],
      loading: false,
      isCheckinOpen: false,
      checkinRoomId: null,
      isGroupCheckinOpen: false,
      groups: [],
    });
    useAuthStore.setState({
      user: null,
      isAuthenticated: false,
      loading: false,
      error: null,
    });

    setMockResponses({
      get_bootstrap_status: () => ({
        setup_completed: true,
        app_lock_enabled: false,
        current_user: {
          id: "admin-1",
          name: "Owner",
          role: "admin",
          active: true,
          created_at: "2026-04-18T00:00:00.000Z",
        },
      }),
    });
  });

  it("không mount trang khai báo cho tới khi được xem lần đầu", async () => {
    await renderReadyApp();
    expect(screen.getByText("Overview")).toBeInTheDocument();
    expect(declarationMountCount).toBe(0);
  });

  it("chuyển sang tab khác rồi quay lại: trang khai báo không unmount, state không mất", async () => {
    const user = userEvent.setup();
    await renderReadyApp();

    await user.click(screen.getByRole("button", { name: /khai báo tạm trú/i }));
    expect(declarationMountCount).toBe(1);
    expect(screen.getByText(/unsaved: 0/)).toBeInTheDocument();

    // Giả lập vừa thả 2 ảnh — hai thẻ chưa lưu.
    await user.click(screen.getByRole("button", { name: /thả ảnh giấy tờ/i }));
    await user.click(screen.getByRole("button", { name: /thả ảnh giấy tờ/i }));
    expect(screen.getByText(/unsaved: 2/)).toBeInTheDocument();

    // Ngó sang Dashboard rồi quay lại — kịch bản đúng của FINDING D. Cây vẫn
    // còn trong DOM (chỉ ẩn bằng CSS, KHÔNG unmount) nên vẫn tìm thấy được,
    // nhưng khung bọc nó phải mang lớp ẩn.
    await user.click(screen.getByRole("button", { name: /^dashboard$/i }));
    const hiddenWrapper = screen.getByText(/declaration stub/i).closest(".hidden");
    expect(hiddenWrapper).not.toBeNull();

    await user.click(screen.getByRole("button", { name: /khai báo tạm trú/i }));

    // Vẫn đúng 1 lần mount — không unmount/remount — và hai thẻ vẫn còn đó.
    expect(declarationMountCount).toBe(1);
    expect(screen.getByText(/unsaved: 2/)).toBeInTheDocument();
  });
});
