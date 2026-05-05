import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import SettingsPage from "./index";
import CeoAgentSection from "./CeoAgentSection";
import { clearMockResponses, invoke, setMockResponses } from "@/__mocks__/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";

const { toastSuccess, toastError } = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: {
    success: toastSuccess,
    error: toastError,
  },
}));

describe("CeoAgentSection", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    toastSuccess.mockReset();
    toastError.mockReset();

    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });
  });

  it("renders the disabled opt-in state and required copy", async () => {
    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
      set_ceo_cloud_data_opt_in: () => undefined,
    });

    render(<CeoAgentSection />);

    expect(
      screen.getByText(/opt-in is required before future CEO-sensitive cloud LLM processing/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/opt-in is revocable/i)).toBeInTheDocument();
    expect(
      screen.getByText(/revoking blocks cloud calls containing CEO-sensitive PMS data/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /raw prompts, raw responses, raw tool outputs, and raw provider errors are not stored/i,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/runtime remains disabled in this foundation slice/i)).toBeInTheDocument();

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    expect(checkbox).not.toBeChecked();
  });

  it("allows an admin to toggle opt-in on via an idempotent write command", async () => {
    const user = userEvent.setup();

    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
      set_ceo_cloud_data_opt_in: () => undefined,
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    await user.click(checkbox);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_cloud_data_opt_in",
        expect.objectContaining({
          enabled: true,
          idempotencyKey: expect.stringMatching(/^set_ceo_cloud_data_opt_in:/),
        }),
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("CEO cloud-data opt-in enabled");
  });

  it("allows an admin to revoke opt-in and calls the backend with enabled=false", async () => {
    const user = userEvent.setup();

    setMockResponses({
      get_ceo_cloud_data_opt_in: () => true,
      set_ceo_cloud_data_opt_in: () => undefined,
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    expect(checkbox).toBeChecked();

    await user.click(checkbox);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "set_ceo_cloud_data_opt_in",
        expect.objectContaining({
          enabled: false,
          idempotencyKey: expect.stringMatching(/^set_ceo_cloud_data_opt_in:/),
        }),
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("CEO cloud-data opt-in revoked");
  });

  it("reverts the checkbox and shows an error toast when the update fails", async () => {
    const user = userEvent.setup();

    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
      set_ceo_cloud_data_opt_in: () => {
        throw new Error("boom");
      },
    });

    render(<CeoAgentSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: "Allow CEO cloud-data processing",
    });
    expect(checkbox).not.toBeChecked();

    await user.click(checkbox);

    await waitFor(() => expect(toastError).toHaveBeenCalledWith("Unable to update CEO cloud-data opt-in"));
    expect(checkbox).not.toBeChecked();
  });
});

describe("SettingsPage CEO Agent nav", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
  });

  it("shows CEO Agent in settings nav for admins and renders the section", async () => {
    const user = userEvent.setup();
    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });

    setMockResponses({
      get_ceo_cloud_data_opt_in: () => false,
      set_ceo_cloud_data_opt_in: () => undefined,
    });

    render(<SettingsPage />);

    const navButton = screen.getByRole("button", { name: "CEO Agent" });
    await user.click(navButton);

    expect(await screen.findByRole("checkbox", { name: "Allow CEO cloud-data processing" })).toBeInTheDocument();
  });

  it("does not show CEO Agent in settings nav for receptionist users", () => {
    useAuthStore.setState({
      user: { id: "u2", name: "Reception", role: "receptionist", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });

    render(<SettingsPage />);

    expect(screen.queryByRole("button", { name: "CEO Agent" })).not.toBeInTheDocument();
  });
});

