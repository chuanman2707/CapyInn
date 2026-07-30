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

    it("bấm tiếp tục ở hộp trùng thì mới ghi, dùng đúng yêu cầu đã chốt chứ không đọc lại form", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Hè" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "25" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));
        await screen.findByRole("button", { name: "Tiếp tục" });

        // Đổi form sau khi hộp trùng đã hiện — lệnh gửi đi phải là yêu cầu đã
        // chốt lúc bấm "Thêm", không phải đọc lại form lúc bấm "Tiếp tục".
        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Đã đổi ý" } });

        fireEvent.click(screen.getByRole("button", { name: "Tiếp tục" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        expect(invokeWriteCommand).toHaveBeenCalledWith("save_special_date_range", {
            remove: [],
            from: "2026-02-20",
            to: "2026-02-28",
            label: "Hè",
            upliftPct: 25,
        });
    });

    it("huỷ sửa khi hộp ghi đè đang mở thì hộp ghi đè cũng tắt theo, không còn armed", async () => {
        invoke.mockResolvedValue([
            ...tetRows(),
            { id: "he-1", date: "2026-03-01", label: "Hè", uplift_pct: 20 },
        ]);
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");
        await screen.findByText("Hè");

        // Sửa cụm Tết, kéo dài "Đến ngày" đè lên ngày của cụm Hè để hộp ghi
        // đè phải bật lên trong lúc đang sửa.
        fireEvent.click(screen.getAllByRole("button", { name: "Sửa" })[0]);
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-03-01" } });
        fireEvent.click(screen.getByRole("button", { name: "Cập nhật" }));

        expect(await screen.findByText(/1 ngày đã khai sẽ bị ghi đè/)).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: "Huỷ sửa" }));

        expect(screen.queryByText(/ngày đã khai sẽ bị ghi đè/)).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tiếp tục" })).not.toBeInTheDocument();
        expect(invokeWriteCommand).not.toHaveBeenCalled();
    });

    it("đổi sang khoảng không trùng rồi bấm thêm thì hộp ghi đè cũ của yêu cầu trước biến mất ngay", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Hè" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "25" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        expect(await screen.findByText(/3 ngày đã khai sẽ bị ghi đè/)).toBeInTheDocument();

        // Đổi sang khoảng không đụng ngày nào đã khai, rồi bấm Thêm lại —
        // hộp của yêu cầu cũ (đã bỏ) không được treo lại trên màn hình.
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-04-30" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-05-03" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        expect(screen.queryByText(/ngày đã khai sẽ bị ghi đè/)).not.toBeInTheDocument();

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        expect(invokeWriteCommand).toHaveBeenCalledWith("save_special_date_range", {
            remove: [],
            from: "2026-04-30",
            to: "2026-05-03",
            label: "Hè",
            upliftPct: 25,
        });
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

    it("sửa cụm cho dài ra thì `remove` rỗng, không ngày nào bị coi là rơi ra", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Sửa" }));
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-03-01" } });
        fireEvent.click(screen.getByRole("button", { name: "Cập nhật" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        const [command, args] = invokeWriteCommand.mock.calls[0];
        expect(command).toBe("save_special_date_range");
        expect(args.remove).toEqual([]);
        expect(
            invokeWriteCommand.mock.calls.some(([name]) => name === "delete_special_dates"),
        ).toBe(false);
    });

    it("sửa cụm cho dịch chuyển thì `remove` đúng là các ngày đầu rơi ra, không đụng ngày còn giữ", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Sửa" }));
        // 14–22 dịch thành 20–28: 14..19 rơi ra, 20..22 vẫn còn trong khoảng mới.
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.click(screen.getByRole("button", { name: "Cập nhật" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        const [command, args] = invokeWriteCommand.mock.calls[0];
        expect(command).toBe("save_special_date_range");
        expect(args.remove).toEqual([
            "2026-02-14",
            "2026-02-15",
            "2026-02-16",
            "2026-02-17",
            "2026-02-18",
            "2026-02-19",
        ]);
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

    it("tải danh sách lỗi thì báo lỗi thay vì nói chưa khai, và chặn lưu để khỏi ghi đè mù", async () => {
        const { toast } = await import("sonner");
        invoke.mockRejectedValue(new Error("network down"));
        render(<SpecialDatesSection />);

        expect(
            await screen.findByText(/Không tải được danh sách đợt cao điểm/),
        ).toBeInTheDocument();
        expect(screen.queryByText(/Chưa khai đợt cao điểm nào/)).not.toBeInTheDocument();
        await waitFor(() => expect(toast.error).toHaveBeenCalled());

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Lễ 30/4" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-04-30" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-05-03" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "30" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        expect(invokeWriteCommand).not.toHaveBeenCalled();
        expect(toast.error).toHaveBeenCalledWith(
            expect.stringMatching(/không thể so trùng|Tải lại/),
        );
    });

    it("để trống % phụ thu thì báo lỗi, không lưu 0% ngầm", async () => {
        const { toast } = await import("sonner");
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Lễ 30/4" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-04-30" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-05-03" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        expect(toast.error).toHaveBeenCalledWith(expect.stringMatching(/% phụ thu/));
        expect(invokeWriteCommand).not.toHaveBeenCalled();
    });
});
