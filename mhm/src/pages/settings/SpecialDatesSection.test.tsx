import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import SpecialDatesSection from "./SpecialDatesSection";

const invokeWriteCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeWriteCommand }));

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

function tetRows() {
    return Array.from({ length: 9 }, (_, index) => ({
        id: `id-${index}`,
        date: `2026-02-${14 + index}`,
        label: "Tết Nguyên đán",
        uplift_pct: 40,
    }));
}

describe("SpecialDatesSection", () => {
    beforeEach(() => {
        invokeWriteCommand.mockReset().mockResolvedValue(undefined);
        invoke.mockReset().mockResolvedValue(tetRows());
    });

    it("hiện chín ngày Tết thành một dòng", async () => {
        render(<SpecialDatesSection />);

        expect(await screen.findByText("Tết Nguyên đán")).toBeInTheDocument();
        expect(screen.getByText(/9 ngày/)).toBeInTheDocument();
        expect(screen.getAllByRole("button", { name: "Xoá" })).toHaveLength(1);
    });

    it("khai khoảng không trùng thì gọi thẳng lệnh ghi", async () => {
        invoke.mockResolvedValue([]);
        render(<SpecialDatesSection />);
        await screen.findByText(/Chưa khai đợt cao điểm nào/);

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Lễ 30/4" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-04-30" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-05-03" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "30" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        expect(invokeWriteCommand).toHaveBeenCalledWith("save_special_date_range", {
            remove: [],
            from: "2026-04-30",
            to: "2026-05-03",
            label: "Lễ 30/4",
            upliftPct: 30,
        });
    });

    it("khai đè lên ngày đã có thì hỏi trước, và huỷ thì không ghi gì", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Hè" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "25" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        expect(await screen.findByText(/3 ngày đã khai sẽ bị ghi đè/)).toBeInTheDocument();
        expect(screen.getByText(/2026-02-20/)).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: "Huỷ" }));

        expect(invokeWriteCommand).not.toHaveBeenCalled();
    });

    it("bấm tiếp tục ở hộp trùng thì mới ghi", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Hè" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "25" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));
        fireEvent.click(await screen.findByRole("button", { name: "Tiếp tục" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
    });

    it("xoá một cụm gửi đúng chín ngày trong một lần gọi", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Xoá" }));
        fireEvent.click(await screen.findByRole("button", { name: "Xoá đợt này" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        const [command, args] = invokeWriteCommand.mock.calls[0];
        expect(command).toBe("delete_special_dates");
        expect(args.dates).toHaveLength(9);
    });

    it("sửa cụm cho ngắn lại thì ngày rơi ra đi trong `remove`, không có lệnh xoá riêng", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Sửa" }));
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-19" } });
        fireEvent.click(screen.getByRole("button", { name: "Cập nhật" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        const [command, args] = invokeWriteCommand.mock.calls[0];
        expect(command).toBe("save_special_date_range");
        // 20, 21, 22 rơi ra khỏi khoảng mới 14–19.
        expect(args.remove).toEqual(["2026-02-20", "2026-02-21", "2026-02-22"]);
        expect(
            invokeWriteCommand.mock.calls.some(([name]) => name === "delete_special_dates"),
        ).toBe(false);
    });

    it("lệnh lỗi thì báo toast và không đổi danh sách ngầm", async () => {
        const { toast } = await import("sonner");
        invokeWriteCommand.mockRejectedValue(new Error("Không đủ quyền"));
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Xoá" }));
        fireEvent.click(await screen.findByRole("button", { name: "Xoá đợt này" }));

        await waitFor(() => expect(toast.error).toHaveBeenCalled());
        expect(screen.getByText("Tết Nguyên đán")).toBeInTheDocument();
    });
});
