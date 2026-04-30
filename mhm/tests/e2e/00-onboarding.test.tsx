import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import App from "@/App";
import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";
import userEvent from "@testing-library/user-event";

describe("00 — Onboarding", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        localStorage.clear();
        useAuthStore.setState({
            user: null,
            isAuthenticated: false,
            loading: false,
            error: null,
        });
    });

    it("shows onboarding before login when setup is incomplete", async () => {
        setMockResponse("get_bootstrap_status", () => ({
            setup_completed: false,
            app_lock_enabled: false,
            current_user: null,
        }));

        render(<App />);

        await waitFor(() => {
            expect(screen.getByText(/thiết lập lần đầu/i)).toBeInTheDocument();
        });
    });

    async function fillHotelInfo(user: ReturnType<typeof userEvent.setup>) {
        await user.click(await screen.findByRole("button", { name: /bắt đầu thiết lập/i }));
        await user.type(screen.getByLabelText(/tên khách sạn/i), "Sunrise Hotel");
        await user.type(screen.getByLabelText(/địa chỉ/i), "12 Tran Hung Dao");
        await user.type(screen.getByLabelText(/số điện thoại/i), "0909123456");
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
    }

    async function configureRoomSetup(user: ReturnType<typeof userEvent.setup>) {
        await user.type(screen.getByLabelText(/tên loại phòng 1/i), "Garden");
        await user.clear(screen.getByLabelText(/giá cơ bản 1/i));
        await user.type(screen.getByLabelText(/giá cơ bản 1/i), "450000");
        await user.clear(screen.getByLabelText(/số khách chuẩn 1/i));
        await user.type(screen.getByLabelText(/số khách chuẩn 1/i), "3");
        await user.clear(screen.getByLabelText(/phụ thu người thêm 1/i));
        await user.type(screen.getByLabelText(/phụ thu người thêm 1/i), "70000");

        await user.click(screen.getByRole("button", { name: /thêm loại phòng/i }));
        await user.type(screen.getByLabelText(/tên loại phòng 2/i), "Family");
        await user.clear(screen.getByLabelText(/giá cơ bản 2/i));
        await user.type(screen.getByLabelText(/giá cơ bản 2/i), "650000");
        await user.clear(screen.getByLabelText(/số khách chuẩn 2/i));
        await user.type(screen.getByLabelText(/số khách chuẩn 2/i), "5");
        await user.clear(screen.getByLabelText(/phụ thu người thêm 2/i));
        await user.type(screen.getByLabelText(/phụ thu người thêm 2/i), "90000");

        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));

        await user.clear(screen.getByLabelText(/số tầng/i));
        await user.type(screen.getByLabelText(/số tầng/i), "2");
        await user.clear(screen.getByLabelText(/số phòng mỗi tầng/i));
        await user.type(screen.getByLabelText(/số phòng mỗi tầng/i), "2");
        await user.selectOptions(screen.getByLabelText(/kiểu đánh số phòng/i), "floor_number");
        await user.selectOptions(screen.getByLabelText(/cột 1/i), "Garden");
        await user.selectOptions(screen.getByLabelText(/cột 2/i), "Family");
        await user.click(screen.getByRole("button", { name: /tạo sơ đồ phòng/i }));
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
    }

    it("submits onboarding with PIN and then shows login screen", async () => {
        const user = userEvent.setup();
        setMockResponse("get_bootstrap_status", () => ({
            setup_completed: false,
            app_lock_enabled: false,
            current_user: null,
        }));
        setMockResponse("complete_onboarding", () => ({
            setup_completed: true,
            app_lock_enabled: true,
            current_user: null,
        }));

        render(<App />);

        await fillHotelInfo(user);
        await configureRoomSetup(user);
        await user.type(screen.getByLabelText(/tên admin/i), "Owner");
        await user.type(screen.getByLabelText(/^pin$/i), "1234");
        await user.type(screen.getByLabelText(/xác nhận pin/i), "1234");
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
        await user.click(screen.getByRole("button", { name: /hoàn tất thiết lập/i }));

        const completeCall = invoke.mock.calls.find(([command]) => command === "complete_onboarding");
        expect(completeCall?.[1]).toMatchObject({
            req: {
                room_types: [
                    expect.objectContaining({ name: "Garden", base_price: 450000, max_guests: 3 }),
                    expect.objectContaining({ name: "Family", base_price: 650000, max_guests: 5 }),
                ],
                rooms: [
                    expect.objectContaining({ id: "101", room_type_name: "Garden" }),
                    expect.objectContaining({ id: "102", room_type_name: "Family" }),
                    expect.objectContaining({ id: "201", room_type_name: "Garden" }),
                    expect.objectContaining({ id: "202", room_type_name: "Family" }),
                ],
            },
        });

        await waitFor(() => {
            expect(screen.getByText(/nhập mã pin để đăng nhập/i)).toBeInTheDocument();
        });
    });

    it("submits onboarding without PIN and enters the app directly", async () => {
        const user = userEvent.setup();
        setMockResponse("get_bootstrap_status", () => ({
            setup_completed: false,
            app_lock_enabled: false,
            current_user: null,
        }));
        setMockResponse("complete_onboarding", () => ({
            setup_completed: true,
            app_lock_enabled: false,
            current_user: {
                id: "bootstrap-admin",
                name: "Owner",
                role: "admin",
                active: true,
                created_at: new Date().toISOString(),
            },
        }));

        render(<App />);

        await fillHotelInfo(user);
        await configureRoomSetup(user);
        await user.click(screen.getByRole("radio", { name: /không dùng pin/i }));
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
        await user.click(screen.getByRole("button", { name: /hoàn tất thiết lập/i }));

        await waitFor(() => {
            expect(screen.getByTitle("Dashboard")).toBeInTheDocument();
        });
    });

    it("shows clean money validation errors during onboarding", async () => {
        const user = userEvent.setup();
        setMockResponse("get_bootstrap_status", () => ({
            setup_completed: false,
            app_lock_enabled: false,
            current_user: null,
        }));

        render(<App />);

        await fillHotelInfo(user);
        await user.type(screen.getByLabelText(/tên loại phòng 1/i), "Garden");
        await user.clear(screen.getByLabelText(/giá cơ bản 1/i));
        await user.type(screen.getByLabelText(/giá cơ bản 1/i), "450000.5");
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
        await user.click(screen.getByRole("button", { name: /tạo sơ đồ phòng/i }));
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
        await user.click(screen.getByRole("radio", { name: /không dùng pin/i }));
        await user.click(screen.getByRole("button", { name: /tiếp tục/i }));
        await user.click(screen.getByRole("button", { name: /hoàn tất thiết lập/i }));

        expect(await screen.findByText("base_price must be a safe integer VND value")).toBeInTheDocument();
        expect(screen.queryByText(/^Error: base_price must be/)).not.toBeInTheDocument();
        expect(invoke).not.toHaveBeenCalledWith("complete_onboarding", expect.anything());
    });
});
