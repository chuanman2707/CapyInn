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

    // Count kbtt_pending_rows calls before unmount
    const callCountBeforeUnmount = invokeCommand.mock.calls.filter(
      (call) => call[0] === "kbtt_pending_rows"
    ).length;

    unmount();

    render(<GuestList reloadKey={0} onStateChange={() => {}} />);
    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());

    // Assert backend was called again (nguồn sự thật là DB, không cache)
    const callCountAfterRemount = invokeCommand.mock.calls.filter(
      (call) => call[0] === "kbtt_pending_rows"
    ).length;
    expect(callCountAfterRemount).toBeGreaterThan(callCountBeforeUnmount);
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

  it("phản ứng không bị ảnh hưởng nếu phản hồi cũ đến sau phản hồi mới", async () => {
    // Các promise mà chúng ta kiểm soát được
    let resolveFirstRequest: ((data: DeclarationRow[]) => void) = () => {};
    let resolveSecondRequest: ((data: DeclarationRow[]) => void) = () => {};

    const firstRequestPromise = new Promise<DeclarationRow[]>((resolve) => {
      resolveFirstRequest = resolve;
    });
    const secondRequestPromise = new Promise<DeclarationRow[]>((resolve) => {
      resolveSecondRequest = resolve;
    });

    let requestCount = 0;
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") {
        requestCount++;
        if (requestCount === 1) return firstRequestPromise;
        if (requestCount === 2) return secondRequestPromise;
      }
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") return Promise.resolve([]);
      return Promise.resolve(null);
    });

    const { rerender } = render(<GuestList reloadKey={0} onStateChange={() => {}} />);

    // Trigger second reload (bumping localReload via reloadKey in this simple test case,
    // but really would come from onChanged)
    rerender(<GuestList reloadKey={1} onStateChange={() => {}} />);

    // Resolve SECOND request first
    resolveSecondRequest([row({ full_name: "Trần Thị B", link_id: "l2" })]);

    // Now resolve FIRST request last with different data
    resolveFirstRequest([row({ full_name: "Cũ - nên bị bỏ qua" })]);

    // UI should show the NEWER data (Trần Thị B), not the stale one
    await waitFor(() => expect(screen.getByText("Trần Thị B")).toBeTruthy());
    expect(screen.queryByText(/Cũ - nên bị bỏ qua/)).toBeFalsy();
  });

  // FINDING I5 — GuestList từng nuốt lỗi kbtt_validate thành setFindings([]),
  // khiến mọi khách trông như không còn lỗi chặn nào (blockedLinks rỗng) dù
  // backend vẫn từ chối xuất cả lô. Giờ báo lỗi lên cha thay vì im lặng giả
  // vờ "mọi thứ ổn".
  it("kbtt_validate lỗi: báo checkFailed lên cha, không âm thầm coi như hết lỗi", async () => {
    mockBackend([row({})]);
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") return Promise.resolve([row({})]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") return Promise.reject(new Error("mất kết nối"));
      return Promise.resolve(null);
    });
    const onStateChange = vi.fn();
    render(<GuestList reloadKey={0} onStateChange={onStateChange} />);

    await waitFor(() =>
      expect(onStateChange).toHaveBeenCalledWith(
        expect.objectContaining({ checkFailed: true }),
      ),
    );
  });

  // FINDING B: sau khi nâng cấp lên v22, migration gán "gác lại" cho các
  // danh tính cũ — người vận hành mở app lần đầu thấy danh sách "Chưa khai
  // báo" trống trong khi có khách thật đang nằm trong khu gác lại. Câu rỗng
  // cũ ("Không còn ai chờ khai") nói dối trong tình huống này; khu gác lại
  // phải tự giải thích nó là gì và cách đưa khách trở lại, đồng thời phải mở
  // sẵn vì nó là nội dung duy nhất trên trang.
  it("danh sách chính rỗng nhưng có khách gác lại: câu rỗng không nói 'không còn ai chờ', khu gác lại tự mở và tự giải thích", async () => {
    mockBackend([row({ held: true })]);
    render(<GuestList reloadKey={0} onStateChange={() => {}} />);

    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy();
    expect(screen.queryByText(/không còn ai chờ khai/i)).toBeNull();

    // Khu gác lại giải thích nó là gì và cách đưa khách trở lại.
    expect(screen.getByText(/không tính vào danh sách cần khai/i)).toBeTruthy();
    expect(screen.getByText(/"Đưa lại"/i)).toBeTruthy();

    // Là nội dung duy nhất trên trang nên phải mở sẵn, không cần bấm.
    const details = document.querySelector("details");
    expect(details).not.toBeNull();
    expect((details as HTMLDetailsElement).open).toBe(true);
  });

  it("có khách đang chờ và có khách gác lại: khu gác lại đóng mặc định", async () => {
    mockBackend([
      row({}),
      row({ link_id: "l2", identity_id: "i2", full_name: "Trần Thị B", held: true }),
    ]);
    render(<GuestList reloadKey={0} onStateChange={() => {}} />);

    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    const details = document.querySelector("details");
    expect(details).not.toBeNull();
    expect((details as HTMLDetailsElement).open).toBe(false);
  });

  it("khách vừa thả vào chưa có kết quả kiểm tra: không báo lên cha là đã kiểm xong", async () => {
    let resolveValidate: ((data: unknown[]) => void) | undefined;
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") return Promise.resolve([row({})]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") {
        return new Promise((resolve) => {
          resolveValidate = resolve;
        });
      }
      return Promise.resolve(null);
    });
    const onStateChange = vi.fn();
    render(<GuestList reloadKey={0} onStateChange={onStateChange} />);

    await waitFor(() => expect(screen.getByText("Nguyễn Văn A")).toBeTruthy());
    // Lô đầu tiên chưa từng được kiểm — link "l1" phải nằm trong
    // uncheckedLinkIds trong lúc kbtt_validate còn treo.
    await waitFor(() =>
      expect(onStateChange).toHaveBeenCalledWith(
        expect.objectContaining({ uncheckedLinkIds: ["l1"], checkFailed: false }),
      ),
    );

    resolveValidate?.([]);
    await waitFor(() =>
      expect(onStateChange).toHaveBeenCalledWith(
        expect.objectContaining({ uncheckedLinkIds: [] }),
      ),
    );
  });
});
