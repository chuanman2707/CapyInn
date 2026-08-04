import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@/__mocks__/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";
import SettingsPage from "./index";

function setAdmin() {
  useAuthStore.setState({
    user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
    isAuthenticated: true,
    loading: false,
    error: null,
  });
}

function setRuntimeStatus(gateway: boolean, agent: boolean) {
  setMockResponse("get_experimental_runtime_status", () => ({
    experimental_runtime_enabled: gateway || agent,
    gateway_runtime_enabled: gateway,
    agent_runtime_enabled: agent,
    gateway_disabled_by_override: false,
    agent_disabled_by_override: false,
  }));
}

describe("SettingsPage experimental runtime gates", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    setAdmin();
  });

  it("hides MCP Gateway and CEO Agent in the normal profile", async () => {
    setRuntimeStatus(false, false);

    render(<SettingsPage />);

    await waitFor(() => {
      expect(
        invoke.mock.calls.some(([command]) => command === "get_experimental_runtime_status"),
      ).toBe(true);
    });
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "MCP Gateway" })).not.toBeInTheDocument();
    });
    expect(screen.queryByRole("button", { name: "CEO Agent" })).not.toBeInTheDocument();
  });

  it("shows MCP Gateway when gateway runtime is enabled", async () => {
    setRuntimeStatus(true, false);

    render(<SettingsPage />);

    expect(await screen.findByRole("button", { name: "MCP Gateway" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "CEO Agent" })).not.toBeInTheDocument();
  });

  it("shows CEO Agent when agent runtime is enabled", async () => {
    setRuntimeStatus(false, true);

    render(<SettingsPage />);

    expect(await screen.findByRole("button", { name: "CEO Agent" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "MCP Gateway" })).not.toBeInTheDocument();
  });

  // Hai mục này từng dùng chung một cờ. Tách ra rồi thì phải có chỗ khoá lại,
  // nếu không lần sau ai đó gỡ cờ cho "CEO Agent" mà không ai hay: bot Telegram
  // bật lên trên máy khách sạn chỉ vì lễ tân cần khung chat.
  it("cờ agent runtime tắt: vẫn có Trợ lý quầy nhưng không có CEO Agent", async () => {
    setRuntimeStatus(false, false);

    render(<SettingsPage />);

    expect(await screen.findByRole("button", { name: "Trợ lý quầy" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "CEO Agent" })).not.toBeInTheDocument();
  });

  it("không phải admin thì không có Trợ lý quầy", async () => {
    useAuthStore.setState({
      user: { id: "u2", name: "Lễ tân", role: "receptionist", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });
    setRuntimeStatus(false, false);

    render(<SettingsPage />);

    await waitFor(() => {
      expect(
        invoke.mock.calls.some(([command]) => command === "get_experimental_runtime_status"),
      ).toBe(true);
    });
    expect(screen.queryByRole("button", { name: "Trợ lý quầy" })).not.toBeInTheDocument();
  });
});
