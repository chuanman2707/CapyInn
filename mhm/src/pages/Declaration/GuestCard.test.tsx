import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { toast } from "sonner";

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
    stay_id: null,
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

  // FINDING E: "Xóa" xóa vĩnh viễn cả liên kết lẫn danh tính — nó nằm cách
  // "Gác lại" vài pixel trong cùng dòng chữ xám nhỏ. Một cú bấm nhầm không
  // được phép xóa thẳng; phải xác nhận, và câu xác nhận phải nêu đích danh
  // khách để người vận hành biết chắc mình đang xóa ai.
  it("Xóa hỏi xác nhận nêu tên khách trước, không gọi kbtt_discard ngay", () => {
    invokeCommand.mockResolvedValue(undefined);
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xóa nguyễn văn a/i }));

    expect(invokeCommand).not.toHaveBeenCalledWith(
      "kbtt_discard",
      expect.anything(),
    );
    expect(screen.getAllByText(/Nguyễn Văn A/).length).toBeGreaterThan(0);
    expect(
      screen.getByRole("button", { name: /^hủy$/i }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /xác nhận xóa vĩnh viễn nguyễn văn a/i }),
    ).toBeTruthy();
  });

  it("Xóa: bấm Hủy trong hộp xác nhận thì không xóa gì cả", () => {
    invokeCommand.mockResolvedValue(undefined);
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xóa nguyễn văn a/i }));
    fireEvent.click(screen.getByRole("button", { name: /^hủy$/i }));

    expect(invokeCommand).not.toHaveBeenCalledWith(
      "kbtt_discard",
      expect.anything(),
    );
    // Hộp xác nhận đã đóng.
    expect(screen.queryByRole("button", { name: /^hủy$/i })).toBeNull();
  });

  it("Xóa: xác nhận trong hộp thoại mới thật sự gọi kbtt_discard", async () => {
    invokeCommand.mockResolvedValue(undefined);
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /xóa nguyễn văn a/i }));

    fireEvent.click(
      screen.getByRole("button", { name: /xác nhận xóa vĩnh viễn nguyễn văn a/i }),
    );

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

  // FINDING C1: khách vừa quét, chưa gắn phòng — E16 phải chặn với câu nói
  // đúng nguyên nhân, không còn W01 mâu thuẫn ("vẫn xuất được"), và bấm vào
  // đó phải đưa người vận hành tới ô Phòng chứ không mở ManualForm (form đó
  // không có ô ngày để sửa cho lỗi này).
  it("thẻ chưa chọn phòng không hiện chữ 'vẫn xuất được', bấm vào lỗi thì focus ô Phòng chứ không mở form", () => {
    const findings: DeclarationFinding[] = [
      { code: "E16", severity: "blocking", link_id: "l1", message: "x" },
    ];
    render(
      <GuestCard
        row={row({ room_no: null, stay_id: null })}
        stays={stays}
        findings={findings}
        onChanged={() => {}}
      />,
    );

    expect(screen.queryByText(/vẫn xuất được/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /chưa chọn phòng/i }));

    expect(screen.getByLabelText("Phòng")).toHaveFocus();
    expect(screen.queryByText("Nhập tay danh tính")).toBeNull();
  });

  // Khách ĐÃ có phòng nhưng còn thiếu field danh tính thật (E01) phải giữ
  // nguyên hành vi cũ: bấm mở được ManualForm.
  it("E01 khi đã có phòng vẫn bấm mở được form sửa", () => {
    const findings: DeclarationFinding[] = [
      { code: "E01", severity: "blocking", link_id: "l1", message: "Thiếu field bắt buộc: ngày sinh" },
    ];
    render(
      <GuestCard
        row={row({ room_no: "5A", stay_id: "b1" })}
        stays={stays}
        findings={findings}
        onChanged={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /thiếu thông tin bắt buộc/i }));
    expect(screen.getByDisplayValue("Nguyễn Văn A")).toBeTruthy();
  });

  // FINDING 1: link đã có stay_id thật (booking đã trả phòng nên không còn
  // trong `stays` active) — đổi CHỈ lý do lưu trú không được gửi `stayId:
  // null` đè lên liên kết phòng đó.
  it("đổi lý do lưu trú không xóa mất stay_id đã hết active trong `stays`", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onChanged = vi.fn();
    render(
      <GuestCard
        row={row({ stay_id: "gone-1", room_no: null })}
        stays={stays}
        findings={[]}
        onChanged={onChanged}
      />,
    );

    // Select "Phòng" hiện đúng stay_id cũ, không rơi về rỗng.
    expect(screen.getByLabelText("Phòng")).toHaveValue("gone-1");
    expect(screen.getByText(/phòng cũ/i)).toBeTruthy();

    fireEvent.change(screen.getByLabelText(/lý do lưu trú/i), { target: { value: "20" } });

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_link",
        expect.objectContaining({ linkId: "l1", stayId: "gone-1", stayReason: "20" }),
      ),
    );
  });

  // FINDING 1: `kbtt_update_link` từ chối vì lượt lưu trú vừa chọn đã kết
  // thúc (danh sách phòng phía client cũ) — trước fix, thẻ giữ nguyên danh
  // sách phòng cũ đó và người vận hành không có cách nào tự sửa được từ màn
  // hình. Giờ mọi lỗi của `call()` đều kéo `onChanged()` để cha tải lại
  // danh sách khách + danh sách phòng, kể cả khi lệnh thất bại.
  it("đổi phòng thất bại vẫn gọi onChanged để tải lại danh sách phòng đang cũ", async () => {
    const err = "Lượt lưu trú vừa chọn đã kết thúc — danh sách phòng đã tự tải lại, chọn phòng khác cho khách này.";
    invokeCommand.mockRejectedValue(err);
    const onChanged = vi.fn();
    render(<GuestCard row={row()} stays={stays} findings={[]} onChanged={onChanged} />);

    fireEvent.change(screen.getByLabelText(/phòng/i), { target: { value: "b1" } });

    await waitFor(() => expect(toast.error).toHaveBeenCalledWith(err));
    expect(onChanged).toHaveBeenCalled();
  });

  // FINDING 2: "Mục đích khác" không có ô nhập lý do nào — E12 không thể
  // được gỡ. Ô nhập chỉ hiện khi lý do là "20", và lưu qua kbtt_update_link.
  it("chọn 'Mục đích khác' hiện ô nhập lý do, lưu khi rời ô", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const { rerender } = render(
      <GuestCard row={row()} stays={stays} findings={[]} onChanged={() => {}} />,
    );

    expect(screen.queryByLabelText(/lý do cụ thể/i)).toBeNull();

    fireEvent.change(screen.getByLabelText(/lý do lưu trú/i), { target: { value: "20" } });
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_link",
        expect.objectContaining({ linkId: "l1", stayReason: "20", note: null }),
      ),
    );
    invokeCommand.mockClear();

    // Server nạp lại dòng với lý do mới — ô nhập lý do cụ thể xuất hiện.
    rerender(
      <GuestCard row={row({ stay_reason: "20" })} stays={stays} findings={[]} onChanged={() => {}} />,
    );

    const noteInput = screen.getByLabelText(/lý do cụ thể/i);
    fireEvent.change(noteInput, { target: { value: "Thăm người thân ốm" } });
    fireEvent.blur(noteInput);

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_link",
        expect.objectContaining({
          linkId: "l1",
          stayReason: "20",
          note: "Thăm người thân ốm",
        }),
      ),
    );
  });
});
