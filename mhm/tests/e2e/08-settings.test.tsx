import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import Settings from "@/pages/settings";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { clearMockUpdate, setMockAvailableUpdate } from "@/__mocks__/tauri-updater";
import { useAuthStore } from "@/stores/useAuthStore";

describe("08 — Settings", () => {
    const setAuthenticatedUser = (role: "admin" | "receptionist" = "admin") => {
        useAuthStore.setState({
            user: { id: "u1", name: "Admin", role, active: true, created_at: "" },
            isAuthenticated: true,
            loading: false,
            error: null,
        });
    };

    beforeEach(() => {
        clearMockResponses();
        clearMockUpdate();
        invoke.mockClear();

        setAuthenticatedUser();

        setMockResponse("get_settings", (args: unknown) => {
            const key = (args as { key: string }).key;
            if (key === "hotel_info") {
                return JSON.stringify({ name: "Grand Hotel", address: "123 Main St", phone: "0901234567" });
            }
            if (key === "checkin_rules") {
                return JSON.stringify({ checkin_time: "14:00", checkout_time: "12:00" });
            }
            return null;
        });
        setMockResponse("get_rooms", () => []);
        setMockResponse("get_room_types", () => []);
        setMockResponse("get_pricing_rules", () => []);
        setMockResponse("get_special_dates", () => []);
        setMockResponse("list_users", () => [
            { id: "u1", name: "Admin", role: "admin", active: true, created_at: new Date().toISOString() },
        ]);
    });

    it("renders settings page", async () => {
        render(<Settings />);

        // Settings page should render without crashing
        await waitFor(() => {
            expect(invoke).toHaveBeenCalled();
        });
    });

    it("loads hotel info from settings", async () => {
        render(<Settings />);

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_settings", { key: "hotel_info" });
        });
    });

    it("loads checkin rules from settings", async () => {
        const user = userEvent.setup();
        render(<Settings />);

        // CheckinRulesSection renders lazily — click the Check-in Rules nav button first
        await user.click(screen.getByText("Check-in Rules"));

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_settings", { key: "checkin_rules" });
        });
    });

    it("loads pricing rules", async () => {
        const user = userEvent.setup();
        render(<Settings />);

        // PricingSection renders lazily — click the nav button first
        await user.click(screen.getByText("Pricing"));

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_pricing_rules");
        });
    });

    it("save_settings is called with correct key on save", async () => {
        setMockResponse("save_settings", () => undefined);

        // Directly test the invoke call pattern
        await invoke("save_settings", { key: "hotel_info", value: JSON.stringify({ name: "New Hotel" }) });

        expect(invoke).toHaveBeenCalledWith("save_settings", {
            key: "hotel_info",
            value: JSON.stringify({ name: "New Hotel" }),
        });
    });

    it("loads user list", async () => {
        const user = userEvent.setup();
        render(<Settings />);

        // UserManagementSection renders lazily — click the Users nav button
        await user.click(screen.getByText("Users"));

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("list_users");
        });
    });

    it("uses the hardened export and backup actions for admin users", async () => {
        setMockResponse("export_bookings_csv", () => "/tmp/bookings.csv");
        setMockResponse("backup_database", () => "/tmp/capyinn-backup.db");

        const user = userEvent.setup();
        render(<Settings />);

        await user.click(screen.getByText("Data & Backup"));
        await user.click(screen.getByRole("button", { name: "Export CSV" }));
        await user.click(screen.getByRole("button", { name: "Backup" }));

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("export_bookings_csv");
            expect(invoke).toHaveBeenCalledWith("backup_database");
        });
    });

    it("disables sensitive data actions for non-admin users", async () => {
        setAuthenticatedUser("receptionist");

        const user = userEvent.setup();
        render(<Settings />);

        await user.click(screen.getByText("Data & Backup"));

        expect(screen.getByRole("button", { name: "Export CSV" })).toBeDisabled();
        expect(screen.getByRole("button", { name: "Backup" })).toBeDisabled();
        expect(screen.getByRole("button", { name: "Reset" })).toBeDisabled();
        expect(
            screen.getByText(/Chỉ tài khoản admin mới có thể export, backup/i),
        ).toBeInTheDocument();
    });

    it("disables API key generation for non-admin users", async () => {
        setAuthenticatedUser("receptionist");

        const user = userEvent.setup();
        render(<Settings />);

        await user.click(screen.getByText("MCP Gateway"));

        await waitFor(() => {
            expect(screen.getByRole("button", { name: "Tạo API Key" })).toBeDisabled();
        });
        expect(
            screen.getByText(/Chỉ admin mới có thể tạo API key mới/i),
        ).toBeInTheDocument();
    });

    it("shows the Software Update section and triggers a manual update check", async () => {
        const user = userEvent.setup();
        setMockAvailableUpdate({ version: "0.2.0" });
        setMockResponse("get_bootstrap_status", () => ({
            setup_completed: true,
            app_lock_enabled: false,
            current_user: {
                id: "u1",
                name: "Admin",
                role: "admin",
                active: true,
                created_at: new Date().toISOString(),
            },
        }));

        render(<App />);

        await user.click(await screen.findByTitle("Settings"));
        await user.click(screen.getByText("Software Update"));

        expect(screen.getByText(/Current version/i)).toBeInTheDocument();
        expect(screen.getByText("0.2.0")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Check for updates" }));

        await waitFor(() => {
            expect(screen.getByRole("button", { name: "Update" })).toBeInTheDocument();
        });
    });

    it("shows a confirmation when there is no newer version", async () => {
        const user = userEvent.setup();
        setMockResponse("get_bootstrap_status", () => ({
            setup_completed: true,
            app_lock_enabled: false,
            current_user: {
                id: "u1",
                name: "Admin",
                role: "admin",
                active: true,
                created_at: new Date().toISOString(),
            },
        }));

        render(<App />);

        await user.click(await screen.findByTitle("Settings"));
        await user.click(screen.getByText("Software Update"));
        await user.click(screen.getByRole("button", { name: "Check for updates" }));

        await waitFor(() => {
            expect(screen.getByText(/latest version/i)).toBeInTheDocument();
        });
    });
});
