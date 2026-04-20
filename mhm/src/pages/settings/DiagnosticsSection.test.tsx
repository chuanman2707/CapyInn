import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import DiagnosticsSection from "./DiagnosticsSection";
import { clearMockResponses, setMockResponses } from "@/__mocks__/tauri-core";

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

describe("DiagnosticsSection", () => {
  beforeEach(() => {
    clearMockResponses();
    toastSuccess.mockReset();
    toastError.mockReset();
  });

  it("loads and toggles the crash reporting preference for non-admin users", async () => {
    const user = userEvent.setup();

    setMockResponses({
      get_crash_reporting_preference: () => false,
      set_crash_reporting_preference: () => undefined,
    });

    render(<DiagnosticsSection />);

    const checkbox = await screen.findByRole("checkbox", { name: "Send crash reports" });
    expect(checkbox).not.toBeChecked();

    await user.click(checkbox);

    await waitFor(() =>
      expect(screen.getByText("Severe crash reports are enabled")).toBeInTheDocument(),
    );
  });
});
