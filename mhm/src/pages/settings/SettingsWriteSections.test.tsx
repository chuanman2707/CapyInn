import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const invokeWriteCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  invokeWriteCommand,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import CheckinRulesSection from "./CheckinRulesSection";
import HotelInfoSection from "./HotelInfoSection";

describe("settings write sections", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeWriteCommand.mockResolvedValue(undefined);
    invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command !== "get_settings") {
        throw new Error(`Unexpected raw invoke ${command}`);
      }

      if (args?.key === "hotel_info") {
        return JSON.stringify({
          name: "Grand Hotel",
          address: "123 Main St",
          phone: "0901234567",
          rating: "4.8",
        });
      }

      if (args?.key === "checkin_rules") {
        return JSON.stringify({
          checkin: "15:30",
          checkout: "11:15",
        });
      }

      return null;
    });
  });

  it("saves hotel info through invokeWriteCommand", async () => {
    const user = userEvent.setup();
    render(<HotelInfoSection />);

    const hotelNameInput = await screen.findByDisplayValue("Grand Hotel");
    await user.clear(hotelNameInput);
    await user.type(hotelNameInput, "New Hotel");

    await user.click(screen.getByRole("button", { name: "Lưu thay đổi" }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith("save_settings", {
        key: "hotel_info",
        value: JSON.stringify({
          name: "New Hotel",
          address: "123 Main St",
          phone: "0901234567",
          rating: "4.8",
        }),
      });
    });
    expect(invoke).not.toHaveBeenCalledWith("save_settings", expect.anything());
    expect(toastSuccess).toHaveBeenCalledWith("Đã lưu thông tin khách sạn!");
  });

  it("saves check-in rules through invokeWriteCommand", async () => {
    const user = userEvent.setup();
    render(<CheckinRulesSection />);

    const checkinInput = await screen.findByDisplayValue("15:30");
    const checkoutInput = await screen.findByDisplayValue("11:15");

    fireEvent.change(checkinInput, { target: { value: "16:00" } });
    fireEvent.change(checkoutInput, { target: { value: "10:45" } });

    await user.click(screen.getByRole("button", { name: "Lưu thay đổi" }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith("save_settings", {
        key: "checkin_rules",
        value: JSON.stringify({ checkin: "16:00", checkout: "10:45" }),
      });
    });
    expect(invoke).not.toHaveBeenCalledWith("save_settings", expect.anything());
    expect(toastSuccess).toHaveBeenCalledWith("Đã lưu quy tắc check-in!");
  });
});
