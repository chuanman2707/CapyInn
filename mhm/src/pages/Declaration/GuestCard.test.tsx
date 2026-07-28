import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationFinding, DeclarationRow, StayInfo } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

import GuestCard from "./GuestCard";

function row(over: Partial<DeclarationRow> = {}): DeclarationRow {
  return {
    link_id: "l1",
    identity_id: "i1",
    full_name: "Nguyễn Văn A",
    dob: "1980-05-02",
    gender: "M",
    nationality_iso3: "VNM",
    doc_type_code: "1",
    doc_type_name: null,
    doc_no: "058195006173",
    phone: null,
    residence_status: null,
    address_detail: null,
    passport_no: null,
    passport_expiry: null,
    visa_valid_until: null,
    room_no: null,
    check_in_date: "2026-07-27",
    expected_check_out: "2026-07-28",
    stay_reason: "1",
    stay_reason_note: null,
    name_confirmed_by_human: true,
    single_token_name_ok: false,
    held: false,
    ...over,
  };
}

const stays: StayInfo[] = [
  { stay_id: "b1", room_no: "5A", check_in: "2026-07-27", expected_out: "2026-07-30" },
];

describe("GuestCard", () => {
  it("đổi phòng ngay trên thẻ qua kbtt_update_link", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onChanged = vi.fn();
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={onChanged} />);

    fireEvent.change(screen.getByLabelText(/phòng/i), { target: { value: "b1" } });

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_link",
        expect.objectContaining({ linkId: "l1", stayId: "b1", stayReason: "1" }),
      ),
    );
    expect(onChanged).toHaveBeenCalled();
  });

  it("lỗi hiện thành câu tiếng người, mã thu nhỏ phía sau", () => {
    const findings: DeclarationFinding[] = [
      { code: "W02", severity: "warning", link_id: "l1", message: "Thiếu số điện thoại." },
    ];
    render(<GuestCard row={row()} stays={stays} findings={findings} onChanged={() => {}} />);

    expect(screen.getByText(/điện thoại/)).toBeTruthy();
    expect(screen.getByText("W02")).toBeTruthy();
  });

  it("Gác lại gọi kbtt_hold; thẻ đang gác thì có Đưa lại gọi kbtt_release", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onChanged = vi.fn();
    const { rerender } = render(
      <GuestCard row={row()} stays={stays} findings={[]} onChanged={onChanged} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /gác lại/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_hold", { linkId: "l1" }),
    );

    rerender(
      <GuestCard row={row({ held: true })} stays={stays} findings={[]} onChanged={onChanged} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /đưa lại/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_release", { linkId: "l1" }),
    );
  });

  it("Xóa gọi kbtt_discard", async () => {
    invokeCommand.mockResolvedValue(undefined);
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xóa/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_discard", { linkId: "l1" }),
    );
  });

  it("bấm vào dòng lỗi mở form sửa thông tin khách", () => {
    const findings: DeclarationFinding[] = [
      { code: "E13", severity: "blocking", link_id: "l1", message: "x" },
    ];
    render(<GuestCard row={row()} stays={stays} findings={findings} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /số giấy tờ/i }));
    // ManualForm prefill hiện tên khách trong input
    expect(screen.getByDisplayValue("Nguyễn Văn A")).toBeTruthy();
  });
});
