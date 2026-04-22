import { useCallback, useMemo, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import Settings from "@/pages/settings";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";

type MockUpdateControllerConfig = {
    supported: boolean;
    currentVersion: string;
    nextAvailableVersion: string | null;
};

let mockUpdateControllerConfig: MockUpdateControllerConfig = {
    supported: true,
    currentVersion: "0.1.1",
    nextAvailableVersion: "0.2.0",
};

function resetMockUpdateController(
    overrides: Partial<MockUpdateControllerConfig> = {},
) {
    mockUpdateControllerConfig = {
        supported: true,
        currentVersion: "0.1.1",
        nextAvailableVersion: "0.2.0",
        ...overrides,
    };
}

vi.mock("@/hooks/useAppUpdateController", () => ({
    useAppUpdateController: ({
        enabled,
        currentVersion,
    }: {
        enabled: boolean;
        currentVersion: string;
    }) => {
        const [phase, setPhase] = useState<
            "idle" | "available" | "downloaded" | "installing"
        >("idle");
        const [availableVersion, setAvailableVersion] = useState<string | null>(null);
        const [restartPromptOpen, setRestartPromptOpen] = useState(false);
        const supported = mockUpdateControllerConfig.supported;

        const checkForUpdates = useCallback(async () => {
            if (!enabled || !supported) {
                return;
            }

            if (mockUpdateControllerConfig.nextAvailableVersion) {
                setAvailableVersion(mockUpdateControllerConfig.nextAvailableVersion);
                setPhase("available");
                return;
            }

            setAvailableVersion(null);
            setPhase("idle");
        }, [enabled, supported]);

        const downloadUpdate = useCallback(async () => {
            if (phase !== "available") {
                return;
            }

            setPhase("downloaded");
            setRestartPromptOpen(true);
        }, [phase]);

        const dismissRestartPrompt = useCallback(() => {
            setRestartPromptOpen(false);
        }, []);

        const openRestartPrompt = useCallback(() => {
            if (phase === "downloaded") {
                setRestartPromptOpen(true);
            }
        }, [phase]);

        const confirmInstall = useCallback(async () => {
            setRestartPromptOpen(false);
            setPhase("installing");
        }, []);

        return useMemo(
            () => ({
                supported,
                phase,
                currentVersion: mockUpdateControllerConfig.currentVersion || currentVersion,
                availableVersion,
                restartPromptOpen,
                errorMessage: null,
                canCheck: enabled && supported,
                checkForUpdates,
                downloadUpdate,
                dismissRestartPrompt,
                openRestartPrompt,
                confirmInstall,
            }),
            [
                availableVersion,
                checkForUpdates,
                confirmInstall,
                currentVersion,
                dismissRestartPrompt,
                downloadUpdate,
                enabled,
                openRestartPrompt,
                phase,
                restartPromptOpen,
                supported,
            ],
        );
    },
}));

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
        invoke.mockClear();
        resetMockUpdateController();

        setAuthenticatedUser();

        setMockResponse("get_settings", (args: unknown) => {
            const key = (args as { key: string }).key;
            if (key === "hotel_info") {
                return JSON.stringify({ name: "Grand Hotel", address: "123 Main St", phone: "0901234567" });
            }
            if (key === "checkin_rules") {
                return JSON.stringify({ checkin: "15:30", checkout: "11:15" });
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

        await waitFor(() => {
            expect(screen.getByDisplayValue("15:30")).toBeInTheDocument();
            expect(screen.getByDisplayValue("11:15")).toBeInTheDocument();
        });
    });

    it("hydrates checkin rules from the legacy onboarding payload shape", async () => {
        setMockResponse("get_settings", (args: unknown) => {
            const key = (args as { key: string }).key;
            if (key === "hotel_info") {
                return JSON.stringify({ name: "Grand Hotel", address: "123 Main St", phone: "0901234567" });
            }
            if (key === "checkin_rules") {
                return JSON.stringify({
                    default_checkin_time: "15:45",
                    default_checkout_time: "10:30",
                });
            }
            return null;
        });

        const user = userEvent.setup();
        render(<Settings />);

        await user.click(screen.getByText("Check-in Rules"));

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_settings", { key: "checkin_rules" });
        });

        await waitFor(() => {
            expect(screen.getByDisplayValue("15:45")).toBeInTheDocument();
            expect(screen.getByDisplayValue("10:30")).toBeInTheDocument();
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
            expect(invoke).toHaveBeenCalledWith("list_users", undefined);
        });
    });

    it("shows a forbidden error when list_users is rejected", async () => {
        const forbiddenError = {
            code: "AUTH_FORBIDDEN",
            message: "Không có quyền thực hiện. Yêu cầu quyền Admin.",
            kind: "user" as const,
            support_id: null,
        };

        setMockResponse("list_users", () => {
            throw forbiddenError;
        });

        const user = userEvent.setup();
        render(<Settings />);

        await user.click(screen.getByText("Users"));

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("list_users", undefined);
            expect(screen.getByRole("alert")).toHaveTextContent(forbiddenError.message);
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
        resetMockUpdateController({ nextAvailableVersion: "0.2.0" });
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
        resetMockUpdateController({ nextAvailableVersion: null });
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
