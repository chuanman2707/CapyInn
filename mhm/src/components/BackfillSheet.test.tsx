import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const invokeWriteCommand = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const fetchRooms = vi.hoisted(() => vi.fn());

// Chỉ một phòng, đang trống — đúng như brief yêu cầu (rooms: một phòng trống
// R101 loại standard). status "vacant" khớp với backend status::room::VACANT.
const ORIGINAL_ROOMS = vi.hoisted(() => [
    {
        id: "R101",
        name: "R101",
        type: "standard",
        floor: 1,
        has_balcony: false,
        base_price: 250000,
        max_guests: 2,
        extra_person_fee: 50000,
        status: "vacant",
    },
]);
let rooms = ORIGINAL_ROOMS;

vi.mock("@tauri-apps/api/core", () => ({
    invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
    invokeWriteCommand,
}));

vi.mock("@/lib/correlationId", () => ({
    createCorrelationId,
}));

vi.mock("@/stores/useHotelStore", () => ({
    useHotelStore: () => ({
        rooms,
        fetchRooms,
    }),
}));

vi.mock("@/components/ui/sheet", () => ({
    Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    SheetContent: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => (
        <div {...props}>{children}</div>
    ),
    SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
    SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
    Button: ({
        children,
        ...props
    }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("sonner", () => ({
    toast: {
        error: toastError,
        success: toastSuccess,
    },
}));

import BackfillSheet from "./BackfillSheet";

describe("BackfillSheet", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        rooms = ORIGINAL_ROOMS;
        invokeWriteCommand.mockResolvedValue({});
        createCorrelationId.mockReturnValue("COR-BACKFILL01");
        // Xác thực đúng phòng/ngày được truyền vào engine giá — nếu component
        // tính sai nights hoặc gửi sai roomId/checkIn/checkOut, mock trả về một
        // tổng khác hẳn (999999) và các assertion về "500000" bên dưới sẽ rớt.
        // Một mock luôn trả cùng một hằng số bất kể input sẽ không chứng minh
        // được gì — đây buộc phải khớp đúng tham số thật.
        invoke.mockImplementation(async (command: string, args: unknown) => {
            if (command === "calculate_room_price_preview") {
                const a = args as { roomId?: string; checkIn?: string; checkOut?: string };
                const matches =
                    a.roomId === "R101" && a.checkIn === "2026-07-26" && a.checkOut === "2026-07-28";
                return {
                    pricing_type: "nightly",
                    base_amount: matches ? 500000 : 999999,
                    surcharge_amount: 0,
                    weekend_amount: 0,
                    total: matches ? 500000 : 999999,
                    capped: false,
                    breakdown: [{ label: "2 đêm x 250.000", amount: matches ? 500000 : 999999 }],
                };
            }
            return null;
        });
    });

    it("điền sẵn theo vùng kéo và gợi ý tiền phòng", async () => {
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-26", checkOutDate: "2026-07-28", stillStaying: false }}
            />,
        );
        expect(await screen.findByDisplayValue("2026-07-26")).toBeInTheDocument();
        expect(screen.getByDisplayValue("2026-07-28")).toBeInTheDocument();

        // Gợi ý từ bảng giá, và "Đã thu" mặc định bằng tiền phòng khi khách đã trả.
        await waitFor(() => {
            expect(screen.getAllByDisplayValue("500000").length).toBe(2);
        });
    });

    it("khách còn ở: đổi nhãn ngày ra thành dự kiến", async () => {
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-27", checkOutDate: "2026-07-31", stillStaying: true }}
            />,
        );
        expect(await screen.findByText("Ngày ra dự kiến")).toBeInTheDocument();
        expect(screen.queryByText(/^Ngày ra$/)).not.toBeInTheDocument();
    });

    it("gửi đúng request backfill_stay", async () => {
        const user = userEvent.setup();
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-26", checkOutDate: "2026-07-28", stillStaying: false }}
            />,
        );
        await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyễn Văn Bù");
        await waitFor(() => {
            expect(screen.getAllByDisplayValue("500000").length).toBe(2);
        });
        await user.click(screen.getByRole("button", { name: /Ghi bù/ }));

        await waitFor(() => {
            expect(invokeWriteCommand).toHaveBeenCalledWith(
                "backfill_stay",
                expect.objectContaining({
                    req: expect.objectContaining({
                        room_id: "R101",
                        check_in_date: "2026-07-26",
                        check_out_date: "2026-07-28",
                        expected_checkout_date: null,
                        total_price: 500000,
                        paid_amount: 500000,
                    }),
                }),
                expect.objectContaining({ correlationId: "COR-BACKFILL01" }),
            );
        });
    });

    it("khách còn ở: gửi expected_checkout_date, để trống check_out_date", async () => {
        const user = userEvent.setup();
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-20", checkOutDate: "2026-08-05", stillStaying: true }}
            />,
        );
        await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyễn Văn Còn Ở");
        await user.click(screen.getByRole("button", { name: /Ghi bù/ }));

        await waitFor(() => {
            expect(invokeWriteCommand).toHaveBeenCalledWith(
                "backfill_stay",
                expect.objectContaining({
                    req: expect.objectContaining({
                        room_id: "R101",
                        check_in_date: "2026-07-20",
                        check_out_date: null,
                        expected_checkout_date: "2026-08-05",
                    }),
                }),
                expect.anything(),
            );
        });
    });

    it("chặn đã thu vượt tiền phòng", async () => {
        const user = userEvent.setup();
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-26", checkOutDate: "2026-07-28", stillStaying: false }}
            />,
        );
        await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyễn Văn Bù");
        await waitFor(() => {
            expect(screen.getAllByDisplayValue("500000").length).toBe(2);
        });
        const paidInput = screen.getByLabelText("Đã thu");
        await user.clear(paidInput);
        await user.type(paidInput, "600000");
        expect(screen.getByRole("button", { name: /Ghi bù/ })).toBeDisabled();
        expect(screen.getByText(/không được vượt quá/i)).toBeInTheDocument();
    });

    it("sửa tay tiền phòng thì gợi ý từ engine không còn ghi đè", async () => {
        // Nếu effect nạp gợi ý không tôn trọng cờ "đã sửa tay", một cập nhật giá
        // sau đó (vd đổi ngày) sẽ ghi đè số chủ vừa gõ — test này bắt lỗi đó.
        const user = userEvent.setup();
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-26", checkOutDate: "2026-07-28", stillStaying: false }}
            />,
        );
        await waitFor(() => {
            expect(screen.getAllByDisplayValue("500000").length).toBe(2);
        });

        const totalInput = screen.getByLabelText(/Tiền phòng/i);
        await user.clear(totalInput);
        await user.type(totalInput, "700000");
        expect(totalInput).toHaveValue(700000);

        // Đổi ngày ra — kích hoạt việc engine tính lại giá (dù mock vẫn trả một
        // giá trị "khác" vì thay đổi tham số, total do chủ gõ vẫn phải giữ nguyên).
        const checkOutInput = screen.getByLabelText(/^Ngày ra$/);
        await user.clear(checkOutInput);
        await user.type(checkOutInput, "2026-07-29");

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith(
                "calculate_room_price_preview",
                expect.objectContaining({ checkOut: "2026-07-29" }),
            );
        });
        expect(totalInput).toHaveValue(700000);
    });

    it("chỉ liệt kê phòng trống khi ghi bù khách còn ở", async () => {
        rooms = [
            ...ORIGINAL_ROOMS,
            {
                id: "R102",
                name: "R102",
                type: "deluxe",
                floor: 2,
                has_balcony: true,
                base_price: 800000,
                max_guests: 3,
                extra_person_fee: 80000,
                status: "occupied",
            },
        ];
        render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-27", checkOutDate: "2026-07-31", stillStaying: true }}
            />,
        );
        const select = screen.getByLabelText(/^Phòng$/) as HTMLSelectElement;
        const optionValues = Array.from(select.options).map((o) => o.value);
        expect(optionValues).toContain("R101");
        expect(optionValues).not.toContain("R102");
    });

    it("reset form khi mở lại sheet với vùng kéo khác", async () => {
        const user = userEvent.setup();
        const { rerender } = render(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-26", checkOutDate: "2026-07-28", stillStaying: false }}
            />,
        );
        await user.type(screen.getByPlaceholderText("Họ và tên *"), "Khách Cũ");
        expect(screen.getByPlaceholderText("Họ và tên *")).toHaveValue("Khách Cũ");

        // Đóng rồi mở lại với một vùng kéo khác (roomId, ngày khác) — mô phỏng
        // đúng luồng thật: cha đặt backfillPrefill=null lúc đóng rồi set giá trị
        // mới khi người dùng kéo chọn ô khác.
        rerender(<BackfillSheet open={false} onOpenChange={() => {}} prefill={undefined} />);
        rerender(
            <BackfillSheet
                open
                onOpenChange={() => {}}
                prefill={{ roomId: "R101", checkInDate: "2026-07-10", checkOutDate: "2026-07-12", stillStaying: false }}
            />,
        );

        expect(screen.getByPlaceholderText("Họ và tên *")).toHaveValue("");
        expect(await screen.findByDisplayValue("2026-07-10")).toBeInTheDocument();
    });
});
