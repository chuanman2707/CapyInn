import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import GatewaySection, { buildHttpMcpConfig } from "./GatewaySection";
import { clearMockResponses, invoke, setMockResponse } from "@/__mocks__/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";

describe("GatewaySection", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();

        useAuthStore.setState({
            user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
            isAuthenticated: true,
            loading: false,
            error: null,
        });

        setMockResponse("gateway_get_status", () => ({
            running: true,
            port: 61239,
            has_api_keys: true,
        }));
        setMockResponse("gateway_generate_key", () => "capyinn_sk_test_http_config");
    });

    it("builds an OpenClaw HTTP config using the live gateway port and bearer auth", () => {
        const copied = buildHttpMcpConfig(
            {
                running: true,
                port: 61239,
                has_api_keys: true,
            },
            "capyinn_sk_test_http_config",
        );

        expect(copied).toContain('"transport": "streamable-http"');
        expect(copied).toContain('"url": "http://127.0.0.1:61239/mcp"');
        expect(copied).toContain('"Authorization": "Bearer capyinn_sk_test_http_config"');
        expect(copied).not.toContain("--mcp-stdio");
    });

    it("shows the full HTTP MCP tool list including get_invoice", async () => {
        render(<GatewaySection />);

        await waitFor(() => {
            expect(screen.getByText(/15 MCP Tools available/i)).toBeInTheDocument();
        });

        expect(screen.getByText("• get_invoice")).toBeInTheDocument();
        expect(screen.getByText("• get_hotel_context")).toBeInTheDocument();
        expect(screen.getByText("• create_reservation ✏️")).toBeInTheDocument();
        expect(screen.getByText("• modify_reservation ✏️")).toBeInTheDocument();
        expect(screen.getByText("• cancel_reservation ✏️")).toBeInTheDocument();
    });
});
