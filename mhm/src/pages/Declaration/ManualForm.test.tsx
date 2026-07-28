import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import DropZone from "./DropZone";
import ManualForm from "./ManualForm";

describe("ManualForm", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeCommand.mockResolvedValue("new-id");
  });

  it("saves manual entries as needs_review", async () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn A" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_save_identity",
        expect.objectContaining({
          source: "manual",
          confidence: "needs_review",
        }),
      );
    });
  });

  it("marks the document type as chosen by a human, not a heuristic", async () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn A" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_save_identity",
        expect.objectContaining({
          identity: expect.objectContaining({ doc_type_source: "human" }),
        }),
      );
    });
  });

  it("asks foreign guests for the residence deadline, not the passport expiry", () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Quốc tịch/i), {
      target: { value: "RUS" },
    });

    expect(screen.getByLabelText(/Thời hạn tạm trú/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Ngày hết hạn hộ chiếu/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Không phải ngày hết hạn hộ chiếu/i),
    ).toBeInTheDocument();
  });

  it("refuses to save without a name and a date of birth", () => {
    render(<ManualForm onSaved={vi.fn()} />);

    expect(screen.getByRole("button", { name: /^Lưu/i })).toBeDisabled();
  });

  it("sửa khách đã có: prefill và lưu qua kbtt_update_identity, giữ nguyên id", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onSaved = vi.fn();
    render(
      <ManualForm
        initial={{
          id: "i9",
          full_name: "Nguyễn Văn A",
          dob: "1980-05-02",
          gender: "M",
          nationality_iso3: "VNM",
          doc_type_code: "1",
          doc_no: "058195006173",
          name_confirmed_by_human: true,
        }}
        onSaved={onSaved}
      />,
    );

    expect(screen.getByDisplayValue("Nguyễn Văn A")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_identity",
        expect.objectContaining({ identityId: "i9" }),
      ),
    );
    expect(onSaved).toHaveBeenCalledWith("i9", expect.objectContaining({ id: "i9" }));
  });

  it("preserves name_confirmed_by_human when editing without changing the name", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onSaved = vi.fn();
    render(
      <ManualForm
        initial={{
          id: "i10",
          full_name: "Nguyễn Văn B",
          dob: "1985-03-15",
          gender: "F",
          nationality_iso3: "VNM",
          doc_type_code: "1",
          doc_no: "123456789",
          name_confirmed_by_human: true,
          single_token_name_ok: false,
        }}
        onSaved={onSaved}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_identity",
        expect.objectContaining({
          identityId: "i10",
          identity: expect.objectContaining({
            name_confirmed_by_human: true,
            single_token_name_ok: false,
          }),
        }),
      ),
    );
  });

  // FINDING C2 — E02 và E04 chỉ gỡ được qua hai flag trên Identity, nhưng
  // form sửa (mở ra khi bấm đúng finding đó trên thẻ) không có control nào
  // cho chúng. Không có hai control này thì lời hứa "bấm để xác nhận" của
  // catalog.ts (E02/E04) không có gì để giữ, và khách vừa nhập tay/sửa tay sẽ
  // vĩnh viễn không xuất được.
  it("tên một chữ: có ô tick để xác nhận và flag đó lên tới payload", async () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Sukarno" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });

    const singleTokenCheckbox = screen.getByLabelText(/chỉ có một chữ/i);
    fireEvent.click(singleTokenCheckbox);
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_save_identity",
        expect.objectContaining({
          identity: expect.objectContaining({ single_token_name_ok: true }),
        }),
      ),
    );
  });

  it("khách nước ngoài: có ô xác nhận tên đọc từ hộ chiếu và flag đó lên tới payload", async () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "IVANOV IVAN" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });
    fireEvent.change(screen.getByLabelText(/Quốc tịch/i), {
      target: { value: "RUS" },
    });

    const confirmCheckbox = screen.getByLabelText(/đối chiếu tên này/i);
    fireEvent.click(confirmCheckbox);
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_save_identity",
        expect.objectContaining({
          identity: expect.objectContaining({ name_confirmed_by_human: true }),
        }),
      ),
    );
  });

  it("hai ô xác nhận không hiện khi không liên quan (khách Việt, tên nhiều chữ)", () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn A" },
    });

    expect(screen.queryByLabelText(/chỉ có một chữ/i)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/đối chiếu tên này/i)).not.toBeInTheDocument();
  });

  it("ô xác nhận tên (E04) không hiện cho khách Việt dù tên một chữ", () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Hà" },
    });

    expect(screen.getByLabelText(/chỉ có một chữ/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/đối chiếu tên này/i)).not.toBeInTheDocument();
  });

  it("ô tên một chữ (E02) không hiện cho khách nước ngoài tên nhiều chữ", () => {
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "IVANOV IVAN" },
    });
    fireEvent.change(screen.getByLabelText(/Quốc tịch/i), {
      target: { value: "RUS" },
    });

    expect(screen.queryByLabelText(/chỉ có một chữ/i)).not.toBeInTheDocument();
    expect(screen.getByLabelText(/đối chiếu tên này/i)).toBeInTheDocument();
  });

  it("sửa khách nước ngoài chưa xác nhận: prefill ô xác nhận từ initial, tick rồi lưu gửi lên payload", async () => {
    invokeCommand.mockResolvedValue(undefined);
    const onSaved = vi.fn();
    render(
      <ManualForm
        initial={{
          id: "i11",
          full_name: "ZOLOCHEVSKAIA VERONIKA",
          dob: "1990-03-08",
          gender: "F",
          nationality_iso3: "RUS",
          passport_no: "777785671",
          name_confirmed_by_human: false,
        }}
        onSaved={onSaved}
      />,
    );

    const confirmCheckbox = screen.getByLabelText(/đối chiếu tên này/i) as HTMLInputElement;
    expect(confirmCheckbox.checked).toBe(false);
    fireEvent.click(confirmCheckbox);
    fireEvent.click(screen.getByRole("button", { name: /lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_identity",
        expect.objectContaining({
          identityId: "i11",
          identity: expect.objectContaining({ name_confirmed_by_human: true }),
        }),
      ),
    );
  });
});

describe("DropZone", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("always offers a manual-entry route when extraction is impossible", async () => {
    invokeCommand.mockRejectedValue(
      new Error("Không đọc được QR hay MRZ trong ảnh"),
    );

    render(<DropZone />);

    // sau khi trích xuất thất bại, người dùng phải có đường đi tiếp
    expect(
      await screen.findByRole("button", { name: /Nhập tay/i }),
    ).toBeInTheDocument();
  });

  it("opens the manual form from the drop zone", async () => {
    invokeCommand.mockResolvedValue("new-id");

    render(<DropZone />);

    fireEvent.click(screen.getByRole("button", { name: /Nhập tay/i }));

    expect(await screen.findByLabelText(/Họ và tên/i)).toBeInTheDocument();
  });
});
