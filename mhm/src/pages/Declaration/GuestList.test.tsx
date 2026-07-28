import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationRow } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import GuestList from "./GuestList";

function row(over: Partial<DeclarationRow>): DeclarationRow {
  return {
    link_id: "l1", identity_id: "i1", full_name: "Nguyễn Văn A", dob: "1980-05-02",
    gender: "M", nationality_iso3: "VNM", doc_type_code: "1", doc_type_name: null,
    doc_no: "058195006173", phone: null, residence_status: null, address_detail: null,
    passport_no: null, passport_expiry: null, visa_valid_until: null, room_no: null,
    check_in_date: "2026-07-27", expected_check_out: "2026-07-28", stay_reason: "1",
    stay_reason_note: null, name_confirmed_by_human: true, single_token_name_ok: false,
    held: false, stay_id: null,
    ...over,
  };
}

function mockBackend(rows: DeclarationRow[]) {
  invokeCommand.mockImplementation((cmd: string) => {
    if (cmd === "kbtt_pending_rows") return Promise.resolve(rows);
    if (cmd === "kbtt_list_stays") return Promise.resolve([]);
    if (cmd === "kbtt_validate") return Promise.resolve([]);
    return Promise.resolve(null);
  });
}

describe("GuestList", () => {
  it("khách thường trong 'Chưa khai báo', khách gác trong khu thu gọn", async () => {
    mockBackend([
      row({}),
      row({ link_id: "l2", identity_id: "i2", full_name: "Trần Thị B", held: true }),
    ]);
    render(<GuestList reloadKey={0} onStateChange={() => {}} />);

    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    expect(screen.getByText(/chưa khai báo \(1\)/i)).toBeTruthy();
    expect(screen.getByText(/đã gác lại \(1\)/i)).toBeTruthy();
    // Khách gác nằm trong <details> đóng — tên vẫn render trong DOM.
    expect(screen.getByText("Trần Thị B")).toBeTruthy();
  });

  it("dữ liệu sống sót unmount/remount — nguồn sự thật là DB", async () => {
    mockBackend([row({})]);
    const { unmount } = render(<GuestList reloadKey={0} onStateChange={() => {}} />);
    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    unmount();

    render(<GuestList reloadKey={0} onStateChange={() => {}} />);
    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
  });

  it("báo trạng thái (rows + findings) lên cha để tính nút xuất", async () => {
    mockBackend([row({})]);
    const onStateChange = vi.fn();
    render(<GuestList reloadKey={0} onStateChange={onStateChange} />);
    await waitFor(() =>
      expect(onStateChange).toHaveBeenCalledWith(
        expect.objectContaining({
          rows: expect.arrayContaining([expect.objectContaining({ link_id: "l1" })]),
        }),
      ),
    );
  });
});
