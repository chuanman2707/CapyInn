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
    hasPendingAction: false,
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

  it("gõ đúng chữ nhưng dính khoảng trắng thì nút xoá sạch vẫn tắt", async () => {
    // Vế thứ ba của cùng hàng rào, và là vế DUY NHẤT chưa được đo: đổi câu so
    // sánh thành `phrase.trim() !== DELETE_ALL_PHRASE` thì cả bộ vẫn xanh (đo
    // được), tức phần "không trim" của bản cài đặt đúng đang không có ai canh.
    //
    // Hàng rào này tồn tại để bắt người ta CỐ Ý — spec đòi gõ đúng chữ
    // `XOÁ HẾT`. Một khoảng trắng dính vào là dấu hiệu của dán chuỗi từ chỗ
    // khác, chứ không phải của một người vừa đọc xong câu cảnh báo và gõ tay.
    const onDeleteAll = vi.fn();
    render(<AssistantHistoryList {...props({ isAdmin: true, onDeleteAll })} />);

    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));
    const confirmButton = screen.getByRole("button", { name: "Xoá vĩnh viễn" });
    const box = screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" });

    await userEvent.type(box, " XOÁ HẾT");
    expect(confirmButton).toBeDisabled();

    await userEvent.clear(box);
    await userEvent.type(box, "XOÁ HẾT ");
    expect(confirmButton).toBeDisabled();

    expect(onDeleteAll).not.toHaveBeenCalled();
  });

  it("đóng hộp xoá sạch rồi mở lại thì phải gõ lại từ đầu", async () => {
    // `closeDeleteAll()` dọn `phrase`, và cho tới giờ chỉ có COMMENT canh việc
    // đó: chèn `return;` ngay trước `setPhrase("")` thì cả bộ vẫn xanh (đo
    // được). Tức một nửa hàng rào duy nhất của lệnh xoá sạch không ai đo.
    //
    // Để nguyên chữ đã gõ thì lần mở sau hộp đã sẵn ở trạng thái BẬT NÚT, và cú
    // bấm thứ hai — cú bấm nhầm, cú bấm của người tưởng mình đã huỷ — không còn
    // phải đi qua hàng rào nào nữa.
    const onDeleteAll = vi.fn();
    render(<AssistantHistoryList {...props({ isAdmin: true, onDeleteAll })} />);

    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" }),
      "XOÁ HẾT",
    );
    // Đúng chữ rồi: nút bật. Không có câu này thì một bản không bao giờ bật nút
    // cũng làm khẳng định cuối cùng bên dưới xanh.
    expect(screen.getByRole("button", { name: "Xoá vĩnh viễn" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Huỷ" }));
    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));

    expect(screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" })).toHaveValue("");
    expect(screen.getByRole("button", { name: "Xoá vĩnh viễn" })).toBeDisabled();
    expect(onDeleteAll).not.toHaveBeenCalled();
  });

  it("đang chờ trả lời thì không mở được hộp xoá sạch", () => {
    // Cửa vào của lệnh xoá sạch. Hai nút bên trong hộp được đo ở test kế tiếp —
    // tách ra vì mở hộp là THAY luôn nút này, nên một test không giữ được cả ba
    // trên màn hình cùng lúc.
    const { rerender } = render(<AssistantHistoryList {...props({ isAdmin: true })} />);

    // Vế dương trước: một bản khoá cứng vĩnh viễn thì admin không xoá được gì
    // nữa, mà xoá tay là lối ra DUY NHẤT của dữ liệu khách.
    expect(screen.getByRole("button", { name: "Xoá tất cả hội thoại" })).toBeEnabled();

    rerender(<AssistantHistoryList {...props({ isAdmin: true, busy: true })} />);

    expect(screen.getByRole("button", { name: "Xoá tất cả hội thoại" })).toBeDisabled();
  });

  it("bận nổi lên giữa chừng thì hai nút xác nhận xoá khoá theo", async () => {
    // Hai hộp xác nhận chỉ MỞ được khi đang rảnh (hai nút mở chúng đều khoá
    // theo `busy`), nên ca thật là: mở hộp lúc rảnh, rồi `busy` bật lên giữa
    // chừng — đúng hình dạng của "duyệt thẻ nhận phòng xong quay ra dọn lịch
    // sử, `check_in` vẫn đang bay về". Vì thế phải dựng bằng `rerender`, không
    // dựng bằng `busy: true` ngay từ đầu.
    //
    // Hậu quả nếu không khoá: cả hai lệnh xoá đều gọi `startNewChat()` → mint
    // khoá phiên mới → kết quả `check_in` bị store bỏ (đúng thiết kế của lớp
    // 4), nhưng phòng thì ĐÃ nhận thật. Lễ tân không thấy gì nên tưởng lệnh
    // chưa chạy. Không mất tiền, mất tin.
    const { rerender } = render(<AssistantHistoryList {...props({ isAdmin: true })} />);

    await userEvent.click(
      screen.getByRole("button", { name: "Xoá hội thoại Phòng 201 còn trống không?" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" }),
      "XOÁ HẾT",
    );

    // Lúc rảnh cả hai đều bấm được — vế dương, cùng lý do như test trên.
    expect(screen.getByRole("button", { name: "Xoá hội thoại này" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Xoá vĩnh viễn" })).toBeEnabled();

    rerender(<AssistantHistoryList {...props({ isAdmin: true, busy: true })} />);

    expect(screen.getByRole("button", { name: "Xoá hội thoại này" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Xoá vĩnh viễn" })).toBeDisabled();
  });

  it("đang treo thẻ thì hộp xoá sạch nói luôn là thẻ sẽ mất", async () => {
    // `deleteAllConversations` gọi `startNewChat()` VÔ ĐIỀU KIỆN, nên thẻ nhận
    // phòng đang chờ mất theo — kể cả khi `conversationId === null`, tức lệnh
    // xoá không xoá nổi một dòng nào của chính hội thoại đang mở.
    render(<AssistantHistoryList {...props({ isAdmin: true, hasPendingAction: true })} />);

    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));

    expect(screen.getByText("Thẻ nhận phòng đang chờ cũng sẽ mất.")).toBeInTheDocument();
  });

  it("không treo thẻ thì hộp xoá sạch không doạ chuyện không có", async () => {
    // Vế âm, và là vế quan trọng hơn: một câu cảnh báo in cứng sẽ hiện cả những
    // lúc chẳng có gì để mất, và cảnh báo thường trực là cảnh báo người ta thôi
    // đọc — đúng lúc nó nói thật thì không ai còn nhìn.
    render(<AssistantHistoryList {...props({ isAdmin: true, hasPendingAction: false })} />);

    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));

    expect(screen.getByText("Xoá sạch toàn bộ hội thoại?")).toBeInTheDocument();
    expect(screen.queryByText("Thẻ nhận phòng đang chờ cũng sẽ mất.")).not.toBeInTheDocument();
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
