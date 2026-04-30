import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("@/components/ui/input", () => ({
  Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("@/components/ui/label", () => ({
  Label: ({ children }: { children: ReactNode }) => <label>{children}</label>,
}));

vi.mock("./DynamicRoomTypeSelect", () => ({
  default: ({
    value,
    onChange,
    disabled,
  }: {
    value: string;
    onChange: (value: string) => void;
    disabled?: boolean;
  }) => (
    <select
      aria-label="Loại phòng"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      disabled={disabled}
    >
      <option value="">Chọn loại phòng...</option>
      <option value="standard">standard</option>
    </select>
  ),
}));

import PricingSection from "./PricingSection";

describe("PricingSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_pricing_rules") {
        return [];
      }
      if (command === "save_pricing_rule") {
        return undefined;
      }
      throw new Error(`Unexpected command: ${command}`);
    });
  });

  it("rejects fractional money rates before saving", async () => {
    const user = userEvent.setup();
    render(<PricingSection />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_pricing_rules");
    });

    fireEvent.change(screen.getByLabelText("Loại phòng"), {
      target: { value: "standard" },
    });
    const [hourlyInput] = screen.getAllByRole("spinbutton");
    fireEvent.change(hourlyInput, {
      target: { value: "80000.5" },
    });

    await user.click(screen.getByRole("button", { name: "Thêm" }));

    expect(invoke).not.toHaveBeenCalledWith(
      "save_pricing_rule",
      expect.anything(),
    );
    expect(toastError).toHaveBeenCalledWith(
      "hourly_rate must be a safe integer VND value",
    );
    expect(toastSuccess).not.toHaveBeenCalled();
  });
});
