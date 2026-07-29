import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useHotelStore } from "@/stores/useHotelStore";

vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

import CheckinSheet from "./CheckinSheet";

describe("CheckinSheet", () => {
  it("điền sẵn số đêm khi mở từ calendar", async () => {
    useHotelStore.setState({ isCheckinOpen: true });
    render(<CheckinSheet preSelectedRoomId="1A" preSelectedNights={3} />);

    // FormField (shared) chỉ đặt <label> cạnh <input>, không có htmlFor/id
    // liên kết chúng — nên getByLabelText("Số đêm") không tìm ra input này
    // dù component đã prefill đúng. Đây là lối thoát chính brief cho phép:
    // xác minh qua display value của input số đêm.
    expect(await screen.findByDisplayValue("3")).toBeInTheDocument();
  });
});
