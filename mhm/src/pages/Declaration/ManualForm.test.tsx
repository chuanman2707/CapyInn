import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { emitTestEvent, resetEventMocks } from "@/__mocks__/tauri-event";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const toastInfo = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess, info: toastInfo },
}));

import DropZone from "./DropZone";
import ManualForm from "./ManualForm";

// FINDING I1: `kbtt_save_identity` giờ trả một outcome object, không phải id
// trần — mock mặc định phải phản ánh đúng hình dạng thật (`created_new_link:
// true` = đường thường, tạo khai báo mới) để các test dưới đây bắt được nếu
// ai đó lỡ quay lại đọc `result` như một chuỗi.
const NEW_DECLARATION_OUTCOME = {
  identity_id: "new-id",
  link_id: "link-new",
  created_new_link: true,
  existing_location: null,
};

describe("ManualForm", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeCommand.mockResolvedValue(NEW_DECLARATION_OUTCOME);
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

  // FINDING I1 — trước fix, `kbtt_save_identity` chỉ trả một chuỗi id nên
  // ManualForm không thể phân biệt "vừa tạo khai báo mới" với "khớp một
  // khách đã có khai báo đang hoạt động, không tạo gì thêm" — toast thành
  // công giống hệt nhau ở cả hai trường hợp.
  it("báo thành công bình thường khi vừa tạo một khai báo mới", async () => {
    const onSaved = vi.fn();
    render(<ManualForm onSaved={onSaved} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn A" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith("Đã lưu danh tính"));
    expect(toastInfo).not.toHaveBeenCalled();
    expect(onSaved).toHaveBeenCalledWith("new-id", expect.objectContaining({ full_name: "Nguyễn Văn A" }));
  });

  it("báo khách đã có khai báo đang chờ khi khớp lại một danh tính đang hoạt động, không phải toast thành công thường", async () => {
    invokeCommand.mockResolvedValue({
      identity_id: "existing-id",
      link_id: "link-existing",
      created_new_link: false,
      existing_location: "pending",
    });
    const onSaved = vi.fn();
    render(<ManualForm onSaved={onSaved} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn A" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() =>
      expect(toastInfo).toHaveBeenCalledWith(expect.stringMatching(/danh sách chờ/i)),
    );
    expect(toastSuccess).not.toHaveBeenCalled();
    // Phải trỏ đúng danh tính đã khớp, không phải một id mới bịa ra.
    expect(onSaved).toHaveBeenCalledWith("existing-id", expect.anything());
  });

  it("báo khách đang chờ đối chiếu khi khớp lại một khai báo đã xuất file", async () => {
    invokeCommand.mockResolvedValue({
      identity_id: "existing-id",
      link_id: "link-existing",
      created_new_link: false,
      existing_location: "awaiting_reconciliation",
    });
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn A" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1980-05-02" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() =>
      expect(toastInfo).toHaveBeenCalledWith(expect.stringMatching(/đối chiếu/i)),
    );
  });

  // FINDING I2 — lệnh kbtt_save_identity trả Err(String) trần khi số giấy tờ
  // đã thuộc về một khách khác. `formatAppError` (registry AppError dùng
  // chung) sẽ nuốt mất chuỗi này thành "Có lỗi hệ thống..."; toast phải hiện
  // đúng nguyên văn để người vận hành biết khách nào đang giữ số đó.
  it("hiện nguyên văn lỗi 'số giấy tờ đã thuộc về khách khác' thay vì thông báo hệ thống chung chung", async () => {
    const backendMessage =
      "Số giấy tờ 058195006173 đang thuộc về một khách khác trong hệ thống: " +
      "Phan Thị Mỹ Hà (sinh 1995-07-28). Kiểm tra lại số giấy tờ vừa nhập — có thể đã gõ nhầm.";
    invokeCommand.mockRejectedValue(backendMessage);
    render(<ManualForm onSaved={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/Họ và tên/i), {
      target: { value: "Nguyễn Văn B" },
    });
    fireEvent.change(screen.getByLabelText(/Ngày sinh/i), {
      target: { value: "1970-01-01" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() => expect(toastError).toHaveBeenCalledWith(backendMessage));
  });

  // FINDING I4 — sửa xong một khách quét sạch từ QR/MRZ luôn stamp lại
  // confidence: "needs_review", nên W05 nổ lên NGAY trên một hồ sơ vừa được
  // người xem lại. Edit mode = định nghĩa của "đã có người xem lại", nên nó
  // phải gửi "verified" để W05 tắt. Create mode (nhập tay từ đầu, chưa ai
  // soi) vẫn phải là "needs_review" — xem test "saves manual entries as
  // needs_review" ở trên.
  it("sửa xong một khách gửi confidence đã xem lại (verified), không phải needs_review", async () => {
    invokeCommand.mockResolvedValue(undefined);
    render(
      <ManualForm
        initial={{
          id: "i20",
          full_name: "Nguyễn Văn A",
          dob: "1980-05-02",
          gender: "M",
          nationality_iso3: "VNM",
          doc_type_code: "1",
          doc_no: "058195006173",
          phone: null,
          name_confirmed_by_human: true,
        }}
        onSaved={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText(/Điện thoại/i), {
      target: { value: "0901234567" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Lưu/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith(
        "kbtt_update_identity",
        expect.objectContaining({ confidence: "verified" }),
      ),
    );
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
    resetEventMocks();
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

  // FINDING I1 — cùng lỗi với ManualForm nhưng ở đường quét ảnh: DropZone
  // phải phân biệt "vừa tạo khai báo mới" với "khớp một khách đã có khai báo
  // đang hoạt động, không tạo gì thêm" thay vì cùng một toast "Đã lưu".
  it("báo khách đã có khai báo đang chờ khi thẻ vừa quét khớp một danh tính đang hoạt động", async () => {
    const extracted = {
      source: "qr_cccd",
      confidence: "verified",
      identity: {
        id: "i1",
        full_name: "Nguyễn Văn A",
        dob: "1980-05-02",
        gender: "M",
        nationality_iso3: "VNM",
        doc_no: "058195006173",
        name_confirmed_by_human: false,
      },
      review_hints: [],
      crop_data_url: null,
    };
    invokeCommand.mockImplementation((command: string) => {
      if (command === "kbtt_extract_from_image") return Promise.resolve(extracted);
      if (command === "kbtt_save_identity") {
        return Promise.resolve({
          identity_id: "existing-id",
          link_id: "link-existing",
          created_new_link: false,
          existing_location: "pending",
        });
      }
      return Promise.reject(new Error(`unexpected command ${command}`));
    });

    render(<DropZone />);
    await emitTestEvent("tauri://drag-drop", { type: "drop", paths: ["/tmp/cccd.jpg"] });

    const saveButton = await screen.findByRole("button", {
      name: /Lưu danh tính và ghép khách/i,
    });
    fireEvent.click(saveButton);

    await waitFor(() =>
      expect(toastInfo).toHaveBeenCalledWith(expect.stringMatching(/danh sách chờ/i)),
    );
    expect(toastSuccess).not.toHaveBeenCalled();
    // Không tạo dòng mới — thẻ vừa lưu phải biến mất khỏi khu "vừa quét",
    // đúng như đường thường (onIdentitySaved vẫn được gọi để list tự tải lại).
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: /Lưu danh tính và ghép khách/i }),
      ).not.toBeInTheDocument(),
    );
  });
});
