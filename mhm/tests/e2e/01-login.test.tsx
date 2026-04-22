import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import LoginScreen from "@/pages/LoginScreen";
import { APP_LOGO_ALT } from "@/lib/appIdentity";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";

describe("01 — Login Screen", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        // Reset auth store
        useAuthStore.setState({
            user: null,
            isAuthenticated: false,
            loading: false,
            error: null,
        });
        // Default: get_settings returns null
        setMockResponse("get_settings", () => null);
    });

    it("renders PIN pad with all digits", () => {
        render(<LoginScreen />);

        // Should show digits 0–9
        for (let i = 0; i <= 9; i++) {
            expect(screen.getByRole("button", { name: String(i) })).toBeInTheDocument();
        }

        // Should show app logo branding instead of text title
        expect(screen.getByAltText(APP_LOGO_ALT)).toBeInTheDocument();
        expect(screen.queryByText("MHM Hotel")).not.toBeInTheDocument();

        // Should show PIN instruction
        expect(screen.getByText(/Nhập mã PIN/)).toBeInTheDocument();
    });

    it("renders 4 empty PIN dots", () => {
        render(<LoginScreen />);

        // 4 dots (all empty/unfilled initially)
        const dots = document.querySelectorAll(".rounded-full");
        // At least 4 should be pin dots
        expect(dots.length).toBeGreaterThanOrEqual(4);
    });

    it("fills dots as digits are entered", async () => {
        render(<LoginScreen />);
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "1" }));
        await user.click(screen.getByRole("button", { name: "2" }));

        // After entering 2 digits, 2 dots should be filled
        const filledDots = document.querySelectorAll(".bg-brand-primary.scale-110");
        expect(filledDots.length).toBe(2);
    });

    it("backspace removes last digit", async () => {
        render(<LoginScreen />);
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "1" }));
        await user.click(screen.getByRole("button", { name: "2" }));

        // 2 dots filled
        expect(document.querySelectorAll(".bg-brand-primary.scale-110").length).toBe(2);

        // Click backspace (⌫ button — has aria-label="Xóa")
        const bsButton = screen.getByLabelText("Xóa");
        await user.click(bsButton);

        // Now only 1 dot filled
        expect(document.querySelectorAll(".bg-brand-primary.scale-110").length).toBe(1);
    });

    it("auto-submits when 4 digits are entered and login succeeds", async () => {
        const mockUser = {
            id: "u1",
            name: "Admin",
            role: "admin" as const,
            active: true,
            created_at: new Date().toISOString(),
        };

        setMockResponse("login", (args: unknown) => {
            const req = (args as { req: { pin: string } }).req;
            if (req.pin === "1234") {
                return { user: mockUser };
            }
            throw new Error("Invalid PIN");
        });

        render(<LoginScreen />);
        const user = userEvent.setup();

        // Enter 4 digits — should auto-submit
        await user.click(screen.getByRole("button", { name: "1" }));
        await user.click(screen.getByRole("button", { name: "2" }));
        await user.click(screen.getByRole("button", { name: "3" }));
        await user.click(screen.getByRole("button", { name: "4" }));

        // Wait for login to be called
        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("login", { req: { pin: "1234" } });
        });

        // Auth store should now be authenticated
        await waitFor(() => {
            expect(useAuthStore.getState().isAuthenticated).toBe(true);
            expect(useAuthStore.getState().user?.name).toBe("Admin");
        });
    });

    it("shows error message on wrong PIN", async () => {
        const invalidPinError = {
            code: "AUTH_INVALID_PIN",
            message: "Mã PIN không đúng",
            kind: "user" as const,
            support_id: null,
        };

        setMockResponse("login", () => {
            throw invalidPinError;
        });

        render(<LoginScreen />);
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "9" }));
        await user.click(screen.getByRole("button", { name: "9" }));
        await user.click(screen.getByRole("button", { name: "9" }));
        await user.click(screen.getByRole("button", { name: "9" }));

        // Error message should appear
        await waitFor(() => {
            expect(screen.getByText(invalidPinError.message)).toBeInTheDocument();
            expect(useAuthStore.getState().error).toEqual(invalidPinError);
        });
    });

    it("shows the app logo without hotel-name text branding", async () => {
        render(<LoginScreen />);

        await waitFor(() => {
            expect(screen.getByAltText(APP_LOGO_ALT)).toBeInTheDocument();
        });

        expect(screen.queryByText("Grand Palace")).not.toBeInTheDocument();
    });
});
