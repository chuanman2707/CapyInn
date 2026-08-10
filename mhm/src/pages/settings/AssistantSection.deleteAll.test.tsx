import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();
vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
}));

import { useAssistantStore } from "@/stores/useAssistantStore";
import type { AssistantSettings, ProposedAction } from "@/types/assistant";
import { AssistantSection } from "./AssistantSection";

const settings: AssistantSettings = {
  config: {
    preset: "deep_seek",
    base_url: "https://api.deepseek.com/v1/chat/completions",
    model: "deepseek-chat",
  },
  has_api_key: true,
  cloud_data_opt_in: true,
  gate: { ready: true, missing: [] },
};

function makeAction(): ProposedAction {
  return {
    kind: "check_in",
    payload: { room_id: "R1", guests: [{ full_name: "Nguyễn Văn Nam" }], nights: 1 },
    display: { room_id: "Phòng 201" },
    preview: {},
    warnings: [],
    built_at_ms: Date.now(),
  };
}

/// Tên truy cập của cửa ở Cài đặt cố ý KHÁC cửa trong panel — xem chú thích
/// cạnh `PHRASE_INPUT_ID` trong `AssistantSection.tsx`. Hai bề mặt render cùng
/// lúc được, và hai nút đỏ cùng tên trên một màn hình là lỗi của người dùng
/// trước khi là lỗi của test.
const OPEN_BUTTON = "Xoá sổ hội thoại trợ lý";
const CONFIRM_BUTTON = "Xoá sổ vĩnh viễn";
const CANCEL_BUTTON = "Giữ lại sổ";
const PHRASE_BOX = "Gõ XOÁ HẾT để xoá sổ";
const PENDING_WARNING = "Thẻ đang chờ duyệt trên panel cũng sẽ mất.";

/// Cửa xoá sạch THỨ HAI mà spec dòng 359 đòi: nút phải có ở **cả** cuối danh
/// sách lịch sử **và** ở Cài đặt → Trợ lý quầy. Task 9 làm cửa thứ nhất; cửa
/// này trước đó không task nào nhận.
///
/// File riêng chứ không nối vào `AssistantSection.test.tsx`: bộ test ở đó không
/// đụng `useAssistantStore` nên `beforeEach` của nó không dọn store, còn cả
/// khối này thì đọc `busy` và `pendingAction` từ store. Dòng hard-reset dưới đây
/// là thứ thật sự cô lập các test — `vi.restoreAllMocks()` KHÔNG cô lập được
/// spy đặt trên state zustand, vì `setState` sao chép prop sang object mới còn
/// restore thì phục hồi trên object cũ đã bị vứt (bẫy vòng duyệt Task 7).
describe("AssistantSection — xoá toàn bộ hội thoại", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    invokeWriteCommand.mockReset();
    useAssistantStore.setState(useAssistantStore.getInitialState(), true);

    invokeCommand.mockImplementation(async (command: string) => {
      if (command === "get_assistant_settings") return settings;
      if (command === "list_assistant_conversations") return [];
      throw new Error(`Lệnh đọc ngoài dự kiến: ${command}`);
    });
    invokeWriteCommand.mockImplementation(async (command: string) => {
      if (command === "delete_all_assistant_conversations") return undefined;
      throw new Error(`Lệnh ghi ngoài dự kiến: ${command}`);
    });
  });

  async function openBox() {
    render(<AssistantSection />);
    await userEvent.click(await screen.findByRole("button", { name: OPEN_BUTTON }));
  }

  it("màn hình Cài đặt có đường xoá sạch, không bắt admin đi vòng qua panel", async () => {
    render(<AssistantSection />);

    // Hệ thống KHÔNG tự xoá — spec chốt như thế — nên đường xoá tay là lối ra
    // duy nhất của dữ liệu khách. Nó phải với tới được từ đây.
    expect(await screen.findByRole("button", { name: OPEN_BUTTON })).toBeEnabled();
    // Và không mở sẵn hộp: cú bấm đầu tiên phải là "mở hộp", không phải "xoá".
    expect(screen.queryByRole("button", { name: CONFIRM_BUTTON })).not.toBeInTheDocument();
  });

  it("chưa gõ đúng chữ thì không xoá được, gõ đúng mới gọi lệnh", async () => {
    await openBox();

    const confirm = screen.getByRole("button", { name: CONFIRM_BUTTON });
    expect(confirm).toBeDisabled();

    await userEvent.type(screen.getByRole("textbox", { name: PHRASE_BOX }), "XOÁ HẾT");

    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);

    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("delete_all_assistant_conversations"),
    );
  });

  it("gõ gần đúng thì nút xoá sạch vẫn tắt", async () => {
    // Ba biến thể "gần đúng": thường hoá, bỏ dấu, và dính khoảng trắng. Một bản
    // so bằng `toUpperCase()`, bằng chuẩn hoá bỏ dấu, hoặc bằng `trim()` sẽ mở
    // cổng cho chúng — mà cổng này là thứ duy nhất đứng giữa một cú bấm nhầm và
    // toàn bộ sổ hội thoại có tên khách và số CCCD.
    await openBox();

    const confirm = screen.getByRole("button", { name: CONFIRM_BUTTON });
    const box = screen.getByRole("textbox", { name: PHRASE_BOX });

    await userEvent.type(box, "xoá hết");
    expect(confirm).toBeDisabled();

    await userEvent.clear(box);
    await userEvent.type(box, "XOA HET");
    expect(confirm).toBeDisabled();

    await userEvent.clear(box);
    await userEvent.type(box, " XOÁ HẾT ");
    expect(confirm).toBeDisabled();

    expect(invokeWriteCommand).not.toHaveBeenCalled();
  });

  it("đóng hộp rồi mở lại thì phải gõ lại từ đầu", async () => {
    // Để nguyên chữ đã gõ thì lần mở sau hộp đã sẵn ở trạng thái BẬT NÚT, và cú
    // bấm thứ hai — cú bấm của người tưởng mình đã huỷ — không còn đi qua hàng
    // rào nào nữa.
    await openBox();

    await userEvent.type(screen.getByRole("textbox", { name: PHRASE_BOX }), "XOÁ HẾT");
    // Vế dương: đúng chữ thì nút BẬT. Thiếu câu này thì một bản không bao giờ
    // bật nút cũng làm hai khẳng định cuối xanh.
    expect(screen.getByRole("button", { name: CONFIRM_BUTTON })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: CANCEL_BUTTON }));
    await userEvent.click(screen.getByRole("button", { name: OPEN_BUTTON }));

    expect(screen.getByRole("textbox", { name: PHRASE_BOX })).toHaveValue("");
    expect(screen.getByRole("button", { name: CONFIRM_BUTTON })).toBeDisabled();
    expect(invokeWriteCommand).not.toHaveBeenCalled();
  });

  it("trợ lý đang bận thì không mở được hộp xoá sạch", async () => {
    // `deleteAllConversations()` gọi `startNewChat()` VÔ ĐIỀU KIỆN → mint khoá
    // phiên mới → kết quả `check_in` đang bay về bị store bỏ (đúng thiết kế của
    // lớp 4), NHƯNG phòng thì đã nhận thật và màn hình không nói một chữ. Không
    // mất tiền, mất tin. Ba nút xoá trong danh sách lịch sử đã khoá vì đúng lý
    // do này; màn hình Cài đặt không có sẵn `busy` nên phải tự đọc từ store.
    render(<AssistantSection />);

    // Vế dương trước: khoá cứng vĩnh viễn thì admin không xoá được gì nữa, mà
    // xoá tay là lối ra DUY NHẤT của dữ liệu khách.
    expect(await screen.findByRole("button", { name: OPEN_BUTTON })).toBeEnabled();

    act(() => {
      useAssistantStore.setState({ busy: true });
    });

    expect(screen.getByRole("button", { name: OPEN_BUTTON })).toBeDisabled();
  });

  it("bận nổi lên giữa chừng thì nút Xoá vĩnh viễn khoá theo", async () => {
    // Ca thật: mở hộp lúc rảnh, gõ xong chữ, rồi lễ tân bấm *Đồng ý* trên thẻ ở
    // panel → `busy` bật lên trong lúc hộp này vẫn đang mở. Khoá ở cửa vào
    // không với tới được ca này.
    await openBox();
    await userEvent.type(screen.getByRole("textbox", { name: PHRASE_BOX }), "XOÁ HẾT");
    expect(screen.getByRole("button", { name: CONFIRM_BUTTON })).toBeEnabled();

    act(() => {
      useAssistantStore.setState({ busy: true });
    });

    expect(screen.getByRole("button", { name: CONFIRM_BUTTON })).toBeDisabled();
  });

  it("đang treo thẻ thì hộp nói luôn là thẻ sẽ mất", async () => {
    useAssistantStore.setState({ pendingAction: makeAction() });

    await openBox();

    expect(screen.getByText(PENDING_WARNING)).toBeInTheDocument();
  });

  it("không treo thẻ thì hộp không doạ chuyện không có", async () => {
    // Vế âm, và là vế quan trọng hơn: một câu in cứng sẽ hiện cả những lúc
    // chẳng có gì để mất, và cảnh báo thường trực là cảnh báo người ta thôi
    // đọc — đúng lúc nó nói thật thì không ai còn nhìn.
    await openBox();

    expect(screen.getByText("Xoá sạch sổ hội thoại trợ lý?")).toBeInTheDocument();
    expect(screen.queryByText(PENDING_WARNING)).not.toBeInTheDocument();
  });

  it("xoá sạch bị từ chối thì màn hình Cài đặt nói ra lý do", async () => {
    // `deleteAllConversations()` nuốt lỗi vào `error` của store, mà `error` đó
    // CHỈ được vẽ trong `AssistantPanel` — panel gần như luôn đóng khi chủ nhà
    // đang ở màn hình Cài đặt. Không bê sang bề mặt lỗi của chính màn hình này
    // thì `AUTH_FORBIDDEN` hay DB khoá là im lặng tuyệt đối: admin bấm xong,
    // không thấy gì, tưởng đã xoá.
    invokeWriteCommand.mockRejectedValue(new Error("Chỉ admin mới được thực hiện"));

    await openBox();
    await userEvent.type(screen.getByRole("textbox", { name: PHRASE_BOX }), "XOÁ HẾT");
    await userEvent.click(screen.getByRole("button", { name: CONFIRM_BUTTON }));

    expect(await screen.findByText("Chỉ admin mới được thực hiện")).toBeInTheDocument();
  });

  it("xoá sạch xong thì không để lại viên lỗi nào", async () => {
    // Đối xứng với test trên: một bề mặt lỗi luôn hiện chữ cũng làm test kia
    // xanh mà chẳng đo được gì.
    await openBox();
    await userEvent.type(screen.getByRole("textbox", { name: PHRASE_BOX }), "XOÁ HẾT");
    await userEvent.click(screen.getByRole("button", { name: CONFIRM_BUTTON }));

    await waitFor(() =>
      expect(invokeWriteCommand).toHaveBeenCalledWith("delete_all_assistant_conversations"),
    );
    expect(screen.queryByText(/Chỉ admin mới được thực hiện/)).not.toBeInTheDocument();
    // Hộp đóng lại sau khi xoá: để nguyên hộp đang mở với chữ đã gõ là mời cú
    // bấm thứ hai đi thẳng qua hàng rào.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: CONFIRM_BUTTON })).not.toBeInTheDocument(),
    );
  });

  /// `deleteAll()` không có `try/finally` thì `busy` của CHÍNH màn hình này kẹt
  /// `true` vĩnh viễn, và cả mục *Trợ lý quầy* xám tới khi khởi động lại app.
  ///
  /// Không dựng lại bằng `invokeWriteCommand.mockRejectedValue`: store đã bắt
  /// hết lỗi của lệnh ghi và nuốt vào `error` của nó, nên đường đó không bao
  /// giờ ném ra tới đây. Phải thay chính hàm trên store — đó mới là hình dạng
  /// thật của "một promise không ai lường tới lại reject" (đường bất kỳ trong
  /// `deleteAllConversations` ném ra ngoài hai khối `catch` sẵn có).
  ///
  /// Thay bằng `setState` chứ không bằng `vi.spyOn`: `setState` chép prop sang
  /// object mới nên `restoreAllMocks` phục hồi trên object cũ đã bị vứt — bẫy
  /// đã bắt được ở vòng duyệt Task 7. `beforeEach` ở trên hard-reset cả store
  /// nên hàm thật quay lại nguyên vẹn cho test kế tiếp.
  it("lệnh xoá ném lỗi thì cả mục Trợ lý quầy vẫn dùng được", async () => {
    useAssistantStore.setState({
      deleteAllConversations: async () => {
        throw new Error("Cơ sở dữ liệu đang bị khoá");
      },
    });

    await openBox();
    await userEvent.type(screen.getByRole("textbox", { name: PHRASE_BOX }), "XOÁ HẾT");
    // Vế dương: trước cú bấm, mọi thứ đang bật.
    expect(screen.getByRole("button", { name: "Lưu cấu hình" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: CONFIRM_BUTTON }));

    // Lỗi phải nói ra...
    expect(await screen.findByText("Cơ sở dữ liệu đang bị khoá")).toBeInTheDocument();
    // ...và `busy` phải nhả. Đo tới tận hậu quả, không chỉ một cờ boolean: đây
    // là những nút thật sự chết theo khi cờ kẹt.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Lưu cấu hình" })).toBeEnabled(),
    );
    expect(screen.getByRole("button", { name: OPEN_BUTTON })).toBeEnabled();
    expect(screen.getByRole("checkbox")).toBeEnabled();
    // Hộp cũng phải đóng lại — `closeDeleteAll()` nằm cùng khối `finally`.
    expect(screen.queryByRole("button", { name: CONFIRM_BUTTON })).not.toBeInTheDocument();
  });
});
