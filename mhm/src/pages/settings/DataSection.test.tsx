import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import DataSection from "./DataSection";
import { useAuthStore } from "@/stores/useAuthStore";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("DataSection", () => {
  beforeEach(() => {
    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });
  });

  it("shows the backup retention policy", () => {
    render(<DataSection />);

    expect(
      screen.getByText(
        "Backup thủ công được giữ 30 ngày; backup tự động được giữ 7 ngày. Luôn giữ bản mới nhất để khôi phục.",
      ),
    ).toBeInTheDocument();
  });
});
