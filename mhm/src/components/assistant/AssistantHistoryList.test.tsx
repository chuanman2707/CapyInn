import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";

import type { AssistantConversationSummary } from "@/types/assistant";
import { AssistantHistoryList } from "./AssistantHistoryList";

function summary(
  overrides: Partial<AssistantConversationSummary> = {},
): AssistantConversationSummary {
  return {
    id: "c1",
    user_id: "u1",
    user_name: "Lễ tân A",
    title: "Phòng 201 còn trống không?",
    updated_at: "2026-08-05T09:30:00+07:00",
    ...overrides,
  };
}

/// Tiêu đề fixture cố ý **không** chứa chữ "xoá": test đầu tiên dò
/// `queryByRole("button", { name: /xoá/i })` trên cả cây, nên một tiêu đề như
/// "Xoá booking giúp tôi" sẽ làm nút mở dòng khớp và test xanh vì lý do sai.
const HAI_HOI_THOAI = [
  summary({ id: "c1", user_name: "Lễ tân A", title: "Phòng 201 còn trống không?" }),
  summary({ id: "c2", user_name: "Lễ tân B", title: "Hôm nay ai trả phòng?" }),
];

function props(
  overrides: Partial<ComponentProps<typeof AssistantHistoryList>> = {},
): ComponentProps<typeof AssistantHistoryList> {
  return {
    conversations: [summary()],
    isAdmin: false,
    busy: false,
    onOpen: vi.fn(),
    onDelete: vi.fn(),
    onDeleteAll: vi.fn(),
    ...overrides,
  };
}

describe("AssistantHistoryList", () => {
  it("nút xoá không hiện với tài khoản không phải admin", () => {
    render(<AssistantHistoryList {...props({ isAdmin: false })} />);

    // Khẳng định dòng CÓ được vẽ trước: thiếu câu này thì một component trả
    // `null` — hoặc trả danh sách rỗng vì đọc sai prop — cũng xanh, và cái
    // xanh đó không đo được gì về quyền.
    expect(screen.getByRole("listitem")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /xoá/i })).not.toBeInTheDocument();
  });

  it("admin thấy nút xoá trên dòng và nút xoá tất cả", () => {
    // Vế dương của test trên. Thiếu nó thì một bản không bao giờ vẽ nút xoá —
    // tức đường xoá tay, lối ra DUY NHẤT của dữ liệu khách vì hệ thống không tự
    // xoá — cũng làm test âm xanh.
    render(<AssistantHistoryList {...props({ isAdmin: true })} />);

    expect(
      screen.getByRole("button", { name: "Xoá hội thoại Phòng 201 còn trống không?" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Xoá tất cả hội thoại" })).toBeInTheDocument();
  });

  it("admin thấy tên người tạo NGAY TRÊN dòng của người đó", () => {
    // Khẳng định trong phạm vi từng dòng, không `getByText` trên cả cây: danh
    // sách của admin trộn hội thoại của mọi người, nên "tên có xuất hiện đâu
    // đó" không phải điều cần đo. Một bản in cả hai tên ở chân danh sách, hoặc
    // in tên của dòng đầu lên mọi dòng, vẫn làm `getByText("Lễ tân A")` xanh —
    // mà admin thì bấm xoá theo dòng.
    render(<AssistantHistoryList {...props({ conversations: HAI_HOI_THOAI, isAdmin: true })} />);

    const rows = screen.getAllByRole("listitem");

    expect(within(rows[0]).getByText("Lễ tân A")).toBeInTheDocument();
    expect(within(rows[1]).getByText("Lễ tân B")).toBeInTheDocument();
    expect(within(rows[0]).queryByText("Lễ tân B")).not.toBeInTheDocument();
  });

  it("lễ tân không thấy cột tên người tạo", () => {
    // Lễ tân chỉ nhận hội thoại của chính mình (backend lọc), nên cột này chỉ
    // là nhiễu. Cặp với test trên để `isAdmin` thật sự phải được đọc.
    render(<AssistantHistoryList {...props({ isAdmin: false })} />);

    expect(screen.getByRole("listitem")).toBeInTheDocument();
    expect(screen.queryByText("Lễ tân A")).not.toBeInTheDocument();
  });

  it("mỗi dòng hiện thời điểm cập nhật", () => {
    render(<AssistantHistoryList {...props()} />);

    // TZ ghim Asia/Ho_Chi_Minh trong `vitest.config.ts`, mốc fixture là +07:00.
    expect(screen.getByRole("listitem")).toHaveTextContent("05/08/2026");
  });

  it("bấm một dòng thì mở ĐÚNG hội thoại của dòng đó", async () => {
    const onOpen = vi.fn();
    render(<AssistantHistoryList {...props({ conversations: HAI_HOI_THOAI, onOpen })} />);

    // Neo `^`: tên truy cập của nút xoá là "Xoá hội thoại Hôm nay ai trả
    // phòng?", nên một regex không neo sẽ khớp cả hai nút ngay khi `isAdmin`
    // bật — đỏ vì "tìm thấy nhiều phần tử", tức đỏ vì lý do sai.
    await userEvent.click(screen.getByRole("button", { name: /^Hôm nay ai trả phòng/ }));

    // Hai dòng, bấm dòng THỨ HAI: bản "luôn truyền `conversations[0].id`" —
    // mở nhầm hội thoại, tức mở transcript có tên khách và CCCD của người khác
    // — sẽ đỏ ở đây mà một danh sách một dòng không bắt được.
    expect(onOpen).toHaveBeenCalledWith("c2");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("xoá một hội thoại phải xác nhận trước", async () => {
    const onDelete = vi.fn();
    render(<AssistantHistoryList {...props({ isAdmin: true, onDelete })} />);

    await userEvent.click(
      screen.getByRole("button", { name: "Xoá hội thoại Phòng 201 còn trống không?" }),
    );

    // Bản gọi thẳng `onDelete` từ nút trên dòng sẽ đỏ ở câu này. Xoá là không
    // hoàn tác được và không có bản sao nào ngoài Data & Backup.
    expect(onDelete).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Xoá hội thoại này" }));

    expect(onDelete).toHaveBeenCalledWith("c1");
  });

  it("bấm Giữ lại thì không xoá gì và hộp xác nhận đóng lại", async () => {
    const onDelete = vi.fn();
    render(<AssistantHistoryList {...props({ isAdmin: true, onDelete })} />);

    await userEvent.click(
      screen.getByRole("button", { name: "Xoá hội thoại Phòng 201 còn trống không?" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Giữ lại" }));

    expect(onDelete).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Xoá hội thoại này" })).not.toBeInTheDocument();
  });

  it("Xoá tất cả chỉ bật sau khi gõ đúng XOÁ HẾT", async () => {
    const onDeleteAll = vi.fn();
    render(<AssistantHistoryList {...props({ isAdmin: true, onDeleteAll })} />);

    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));
    const confirmButton = screen.getByRole("button", { name: "Xoá vĩnh viễn" });
    expect(confirmButton).toBeDisabled();
    expect(onDeleteAll).not.toHaveBeenCalled();

    await userEvent.type(
      screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" }),
      "XOÁ HẾT",
    );

    expect(confirmButton).toBeEnabled();
    await userEvent.click(confirmButton);
    expect(onDeleteAll).toHaveBeenCalled();
  });

  it("gõ gần đúng thì nút xoá sạch vẫn tắt", async () => {
    // Hai biến thể "gần đúng" đúng kiểu người Việt gõ: thường hoá, và bỏ dấu.
    // Một bản so bằng `toUpperCase()` hoặc bằng chuẩn hoá bỏ dấu sẽ mở cổng cho
    // cả hai — mà cổng này là cái duy nhất đứng giữa một cú bấm nhầm và toàn bộ
    // sổ hội thoại.
    const onDeleteAll = vi.fn();
    render(<AssistantHistoryList {...props({ isAdmin: true, onDeleteAll })} />);

    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));
    const confirmButton = screen.getByRole("button", { name: "Xoá vĩnh viễn" });
    const box = screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" });

    await userEvent.type(box, "xoá hết");
    expect(confirmButton).toBeDisabled();

    await userEvent.clear(box);
    await userEvent.type(box, "XOA HET");
    expect(confirmButton).toBeDisabled();

    expect(onDeleteAll).not.toHaveBeenCalled();
  });

  it("đang chờ trả lời thì không mở được hội thoại nào", async () => {
    // Spec: `busy` khoá mọi dòng. Chuyển hội thoại giữa lúc câu trả lời đang
    // bay về đẻ ra tranh chấp mà không đổi lại được gì.
    const onOpen = vi.fn();
    render(<AssistantHistoryList {...props({ busy: true, onOpen })} />);

    const row = screen.getByRole("button", { name: /^Phòng 201 còn trống không/ });
    expect(row).toBeDisabled();

    await userEvent.click(row);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("danh sách rỗng thì nói rõ, và không mời admin xoá cái không có", () => {
    render(<AssistantHistoryList {...props({ conversations: [], isAdmin: true })} />);

    expect(screen.getByText("Chưa có hội thoại nào.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Xoá tất cả hội thoại" })).not.toBeInTheDocument();
  });
});
