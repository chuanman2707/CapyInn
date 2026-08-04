import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock nguyên module invokeCommand, không mock @/components/ui/button — Task 8
// và Task 9 (AssistantPanel, ProposedActionCard) đã dùng Button thật thành
// công, mock riêng cho nó là thừa.
const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();
vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
}));

import { AssistantSection } from "./AssistantSection";
import type { AssistantSettings } from "@/types/assistant";

const settings: AssistantSettings = {
  config: {
    preset: "deep_seek",
    base_url: "https://api.deepseek.com/v1/chat/completions",
    model: "deepseek-chat",
  },
  has_api_key: false,
  cloud_data_opt_in: false,
  gate: { ready: false, missing: ["api_key", "cloud_data_opt_in"] },
};

const optedInSettings: AssistantSettings = {
  ...settings,
  has_api_key: true,
  cloud_data_opt_in: true,
  gate: { ready: true, missing: [] },
};

describe("AssistantSection", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    invokeWriteCommand.mockReset();
    invokeCommand.mockResolvedValue(settings);
    invokeWriteCommand.mockResolvedValue(settings);
  });

  it("hiện trạng thái khoá là chưa cấu hình, không hiện giá trị khoá", async () => {
    render(<AssistantSection />);

    // Chuỗi khớp chính xác, không dùng regex rời: đoạn giới thiệu ở đầu
    // section cũng chứa cụm "chưa cấu hình" (mô tả PMS vẫn chạy được), nên
    // regex /chưa cấu hình/i khớp nhầm cả hai chỗ.
    await waitFor(() => expect(screen.getByText("chưa cấu hình")).toBeInTheDocument());
  });

  it("nói rõ dữ liệu khách sẽ rời khỏi máy khi bật công tắc", async () => {
    render(<AssistantSection />);

    await waitFor(() =>
      expect(screen.getByText(/rời khỏi máy này/i)).toBeInTheDocument(),
    );
  });

  it("lưu khoá thì gọi set_assistant_api_key qua invokeWriteCommand và không giữ lại trong ô nhập", async () => {
    render(<AssistantSection />);
    await waitFor(() => screen.getByLabelText(/khoá api/i));

    await userEvent.type(screen.getByLabelText(/khoá api/i), "sk-secret");
    await userEvent.click(screen.getByRole("button", { name: /lưu khoá/i }));

    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("set_assistant_api_key", {
        apiKey: "sk-secret",
      }),
    );
    // Khoá API là bí mật ghi (secret write) — phải đi qua invokeWriteCommand
    // (có idempotency key) như CeoAgentSection đang làm cho openai_api_key,
    // không phải invokeCommand trần.
    expect(invokeCommand).not.toHaveBeenCalledWith("set_assistant_api_key", expect.anything());
    expect(screen.getByLabelText(/khoá api/i)).toHaveValue("");
  });

  it("bật công tắc đồng ý gửi dữ liệu cloud thì gọi set_assistant_cloud_opt_in với enabled true", async () => {
    invokeWriteCommand.mockResolvedValue({ ...settings, cloud_data_opt_in: true });

    render(<AssistantSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: /đồng ý gửi dữ liệu/i,
    });
    expect(checkbox).not.toBeChecked();

    await userEvent.click(checkbox);

    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("set_assistant_cloud_opt_in", {
        enabled: true,
      }),
    );
    // Không chỉ gọi đúng lệnh — công tắc phải thật sự bật trên màn hình theo
    // trạng thái backend trả về, đối xứng với test tắt công tắc bên dưới.
    await waitFor(() => expect(checkbox).toBeChecked());
  });

  it("khách sạn đã đồng ý cloud thì tắt công tắc gọi set_assistant_cloud_opt_in với enabled false", async () => {
    invokeCommand.mockResolvedValue(optedInSettings);
    invokeWriteCommand.mockResolvedValue({ ...optedInSettings, cloud_data_opt_in: false });

    render(<AssistantSection />);

    const checkbox = await screen.findByRole("checkbox", {
      name: /đồng ý gửi dữ liệu/i,
    });
    await waitFor(() => expect(checkbox).toBeChecked());

    await userEvent.click(checkbox);

    // Thu hồi đồng ý phải thật sự gọi được, không chỉ bật được một chiều —
    // đây là thuộc tính "revocable" mà tính năng cam kết với chủ khách sạn.
    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("set_assistant_cloud_opt_in", {
        enabled: false,
      }),
    );
    await waitFor(() => expect(checkbox).not.toBeChecked());
  });

  it("tải cấu hình lỗi thì hiện lỗi thay vì để trống form như chưa có gì", async () => {
    invokeCommand.mockRejectedValueOnce(new Error("Không kết nối được máy chủ trợ lý"));

    render(<AssistantSection />);

    await waitFor(() =>
      expect(screen.getByText("Không kết nối được máy chủ trợ lý")).toBeInTheDocument(),
    );
  });

  it("đổi nhà cung cấp thì tự điền địa chỉ máy chủ và model theo preset", async () => {
    render(<AssistantSection />);
    await waitFor(() => screen.getByLabelText(/khoá api/i));

    // getByRole thay vì getByLabelText: câu đồng ý cloud phía dưới có cụm
    // "...tới nhà cung cấp đã chọn", khiến getByLabelText khớp nhầm cả ô
    // checkbox đó. Giới hạn theo role combobox loại checkbox ra.
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: /nhà cung cấp/i }),
      "open_router",
    );

    expect(screen.getByLabelText(/địa chỉ máy chủ/i)).toHaveValue(
      "https://openrouter.ai/api/v1/chat/completions",
    );
    expect(screen.getByLabelText(/model/i)).toHaveValue("");
  });

  it("sửa địa chỉ máy chủ chưa lưu rồi bật công tắc cloud thì địa chỉ vừa sửa không bị mất", async () => {
    invokeWriteCommand.mockResolvedValue({ ...settings, cloud_data_opt_in: true });

    render(<AssistantSection />);
    await waitFor(() => screen.getByLabelText(/khoá api/i));

    // Gõ địa chỉ mới nhưng KHÔNG bấm "Lưu cấu hình" — mô phỏng đúng tình huống
    // chủ khách sạn sửa dở rồi làm việc khác trên cùng màn hình.
    const baseUrlInput = screen.getByLabelText(/địa chỉ máy chủ/i);
    await userEvent.clear(baseUrlInput);
    await userEvent.type(baseUrlInput, "https://gateway-noi-bo.example.com/v1");

    const checkbox = screen.getByRole("checkbox", { name: /đồng ý gửi dữ liệu/i });
    await userEvent.click(checkbox);

    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("set_assistant_cloud_opt_in", {
        enabled: true,
      }),
    );

    // Bật công tắc cloud là một hành động không liên quan tới địa chỉ máy chủ —
    // nó không được phép ghi đè ô đang gõ dở bằng base_url cũ mà server trả về
    // kèm theo. Nếu không, chủ khách sạn sửa địa chỉ xong quên bấm "Lưu cấu
    // hình" rồi bật công tắc cloud sẽ mất trắng phần vừa gõ mà không có cảnh
    // báo nào.
    expect(baseUrlInput).toHaveValue("https://gateway-noi-bo.example.com/v1");
  });

  it("lưu cấu hình thành công thì nạp lại địa chỉ máy chủ và model từ phản hồi server", async () => {
    render(<AssistantSection />);
    await waitFor(() => screen.getByLabelText(/khoá api/i));

    const baseUrlInput = screen.getByLabelText(/địa chỉ máy chủ/i);
    await userEvent.clear(baseUrlInput);
    await userEvent.type(baseUrlInput, "https://gateway-noi-bo.example.com/v1  ");

    // Server có thể chuẩn hoá giá trị trước khi lưu (ví dụ cắt khoảng trắng
    // thừa) — trả về đúng cấu hình đã lưu, khác nguyên văn ô vừa gõ.
    invokeWriteCommand.mockResolvedValue({
      ...settings,
      config: {
        ...settings.config,
        base_url: "https://gateway-noi-bo.example.com/v1",
        model: "custom-model",
      },
    });

    await userEvent.click(screen.getByRole("button", { name: /lưu cấu hình/i }));

    // Đây là bài chặn "sửa quá tay": gom field-reset về đúng chỗ ở test trên
    // không được biến thành không bao giờ đồng bộ nữa — nút "Lưu cấu hình" vẫn
    // phải nạp lại đúng giá trị server trả về.
    await waitFor(() =>
      expect(baseUrlInput).toHaveValue("https://gateway-noi-bo.example.com/v1"),
    );
    expect(screen.getByLabelText(/model/i)).toHaveValue("custom-model");
  });

  it("lưu khoá xong thì chuỗi khoá vừa gõ không còn xuất hiện ở đâu trên màn hình", async () => {
    render(<AssistantSection />);
    await waitFor(() => screen.getByLabelText(/khoá api/i));

    const secretKey = "sk-chi-em-biet-3f9d8c1a";
    await userEvent.type(screen.getByLabelText(/khoá api/i), secretKey);
    await userEvent.click(screen.getByRole("button", { name: /lưu khoá/i }));

    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("set_assistant_api_key", {
        apiKey: secretKey,
      }),
    );

    // AssistantSettings chỉ mang has_api_key (boolean) — không trường nào chở
    // khoá thật, nên thuộc tính "khoá không bao giờ hiện lại" giữ được là nhờ
    // cấu trúc dữ liệu chứ không nhờ code UI cẩn thận. Chốt nó bằng cách quét
    // toàn bộ DOM sau khi lưu, không riêng gì ô nhập khoá.
    expect(screen.queryByText(secretKey)).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue(secretKey)).not.toBeInTheDocument();
  });
});
