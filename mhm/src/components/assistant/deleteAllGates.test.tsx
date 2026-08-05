import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();
vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
  createIdempotencyKey: (command: string) => `${command}:test`,
}));

// Panel đọc ba field của hotel store qua selector; mock nguyên store để test
// đứng độc lập. `vi.hoisted` vì factory của `vi.mock` bị hoist lên đầu file.
const { hotelState } = vi.hoisted(() => ({
  hotelState: {
    activeTab: "rooms",
    roomDetail: null as { room?: { id?: string; name?: string } } | null,
    roomChangeBookingId: null as string | null,
  },
}));
vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: (selector: (state: typeof hotelState) => unknown) => selector(hotelState),
}));

import { AssistantSection } from "@/pages/settings/AssistantSection";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { useAuthStore } from "@/stores/useAuthStore";
import type {
  AssistantConversationSummary,
  AssistantSettings,
  ProposedAction,
} from "@/types/assistant";
import { AssistantPanel } from "./AssistantPanel";

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

const summary: AssistantConversationSummary = {
  id: "c1",
  user_id: "u1",
  user_name: "Lễ tân A",
  title: "Phòng 201 còn trống không?",
  updated_at: "2026-08-05T09:30:00+07:00",
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

/// Tên của cửa trong panel (cuối danh sách lịch sử) — GIỮ NGUYÊN.
const PANEL_GATE = {
  open: "Xoá tất cả hội thoại",
  confirm: "Xoá vĩnh viễn",
  cancel: "Huỷ",
  phrase: "Gõ XOÁ HẾT để xác nhận",
  heading: "Xoá sạch toàn bộ hội thoại?",
  pendingWarning: "Thẻ nhận phòng đang chờ cũng sẽ mất.",
};

/// Tên của cửa ở Cài đặt → Trợ lý quầy — ĐỔI để phân biệt được.
const SETTINGS_GATE = {
  open: "Xoá sổ hội thoại trợ lý",
  confirm: "Xoá sổ vĩnh viễn",
  cancel: "Giữ lại sổ",
  phrase: "Gõ XOÁ HẾT để xoá sổ",
  heading: "Xoá sạch sổ hội thoại trợ lý?",
  pendingWarning: "Thẻ nhận phòng đang chờ trên panel cũng sẽ mất.",
};

/// HAI CỬA XOÁ SẠCH TRÊN CÙNG MỘT MÀN HÌNH.
///
/// Đây là test **duy nhất** bắt được lớp lỗi này, và lý do nằm ở chỗ nó tồn tại
/// như một file riêng: `AssistantHistoryList.test.tsx` và
/// `AssistantSection.deleteAll.test.tsx` render hai bề mặt TÁCH RỜI, nên trùng
/// tên bao nhiêu cũng không đỏ ở đó. Mà hai bề mặt này thì render cùng lúc được
/// **ngoài đời**: panel trợ lý là cột giữa của shell, còn tab Cài đặt nằm trong
/// `<main>` bên phải — mở Cài đặt mà không thu panel là xong.
///
/// Hậu quả nếu trùng: với test là `getByRole` ném "found multiple elements";
/// với người dùng là **hai nút đỏ y hệt nhau** cùng lúc trên một màn hình, một
/// nút xoá cả sổ và một nút cũng xoá cả sổ, không cách nào phân biệt bằng mắt.
///
/// Implementer bản đầu đã thấy trùng `id` và tránh
/// (`assistant-delete-all-phrase` vs `assistant-settings-delete-all-phrase`)
/// nhưng bỏ sót tên — nên test này khẳng định trên TÊN TRUY CẬP, đúng thứ đã
/// lọt.
describe("hai cửa xoá sạch render cùng lúc", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    invokeWriteCommand.mockReset();

    useAssistantStore.setState(useAssistantStore.getInitialState(), true);
    useAuthStore.setState({
      user: {
        id: "u1",
        name: "Chủ nhà",
        role: "admin",
        active: true,
        created_at: "2026-08-01T00:00:00+07:00",
      },
      isAuthenticated: true,
    });

    invokeCommand.mockImplementation(async (command: string) => {
      if (command === "get_assistant_settings") return settings;
      if (command === "list_assistant_conversations") return [summary];
      throw new Error(`Lệnh đọc ngoài dự kiến: ${command}`);
    });

    useAssistantStore.setState({
      open: true,
      settings,
      conversations: [summary],
      conversationKey: "phien-dang-mo",
      // Thẻ đang treo để CẢ HAI câu cảnh báo cùng hiện — vế khó nhất của bài
      // trùng tên, vì hai câu ấy là chữ thuần, không có role nào đỡ.
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
    });
  });

  /// Dựng đúng cảnh thật: cả hai bề mặt cùng trên một màn hình, panel đứng ở
  /// màn hình lịch sử.
  async function renderBoth() {
    render(
      <>
        <AssistantPanel />
        <AssistantSection />
      </>,
    );

    // Chờ Cài đặt nạp xong cấu hình trước khi bấm gì.
    await screen.findByRole("button", { name: SETTINGS_GATE.open });
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
  }

  /// Cả hai cửa đều thay nút *mở hộp* bằng chính cái hộp, nên hai bộ tên không
  /// bao giờ cùng trên màn hình một lúc — phải đo làm hai nhịp.
  async function openBothBoxes() {
    await userEvent.click(screen.getByRole("button", { name: PANEL_GATE.open }));
    await userEvent.click(screen.getByRole("button", { name: SETTINGS_GATE.open }));
  }

  it("hai nút mở hộp cùng trên màn hình vẫn tra được riêng từng cái", async () => {
    await renderBoth();

    expect(screen.getAllByRole("button", { name: PANEL_GATE.open })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: SETTINGS_GATE.open })).toHaveLength(1);
    // Và chúng là hai phần tử KHÁC nhau, không phải một cái tra được hai kiểu.
    expect(screen.getByRole("button", { name: PANEL_GATE.open })).not.toBe(
      screen.getByRole("button", { name: SETTINGS_GATE.open }),
    );
  });

  it("mọi tên bên trong hai hộp đều tra được đúng MỘT kết quả", async () => {
    await renderBoth();
    await openBothBoxes();

    // Vế dương trước: cả hai hộp thật sự đang mở. Thiếu câu này thì "mỗi tên
    // đúng một kết quả" cũng xanh khi một trong hai hộp chẳng render gì.
    expect(screen.getByText(PANEL_GATE.heading)).toBeInTheDocument();
    expect(screen.getByText(SETTINGS_GATE.heading)).toBeInTheDocument();

    for (const name of [
      PANEL_GATE.confirm,
      PANEL_GATE.cancel,
      SETTINGS_GATE.confirm,
      SETTINGS_GATE.cancel,
    ]) {
      expect(screen.getAllByRole("button", { name })).toHaveLength(1);
    }

    for (const name of [PANEL_GATE.phrase, SETTINGS_GATE.phrase]) {
      expect(screen.getAllByRole("textbox", { name })).toHaveLength(1);
    }

    for (const text of [
      PANEL_GATE.heading,
      SETTINGS_GATE.heading,
      PANEL_GATE.pendingWarning,
      SETTINGS_GATE.pendingWarning,
    ]) {
      expect(screen.getAllByText(text)).toHaveLength(1);
    }
  });

  /// Chuỗi `XOÁ HẾT` thì cố ý KHÔNG tách: nó là hàng rào dùng chung
  /// (`isDeleteAllPhrase`), không phải nhãn. Gõ đúng vào ô của cửa nào thì chỉ
  /// bật nút của cửa đó — hai hộp không được nối dây với nhau.
  it("gõ XOÁ HẾT ở cửa này không bật nút của cửa kia", async () => {
    await renderBoth();
    await openBothBoxes();

    expect(screen.getByRole("button", { name: PANEL_GATE.confirm })).toBeDisabled();
    expect(screen.getByRole("button", { name: SETTINGS_GATE.confirm })).toBeDisabled();

    await userEvent.type(
      screen.getByRole("textbox", { name: SETTINGS_GATE.phrase }),
      "XOÁ HẾT",
    );

    expect(screen.getByRole("button", { name: SETTINGS_GATE.confirm })).toBeEnabled();
    expect(screen.getByRole("button", { name: PANEL_GATE.confirm })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: PANEL_GATE.phrase })).toHaveValue("");
  });
});
