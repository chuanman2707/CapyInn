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
});
