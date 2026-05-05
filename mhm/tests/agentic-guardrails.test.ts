import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const readWorkspaceFile = (path: string) =>
    readFileSync(join(process.cwd(), path), "utf8");

describe("agentic integration guardrails", () => {
    it("documents that agent memory is non-authoritative", () => {
        const skill = readWorkspaceFile("skills/hotel-manager.skill.md");

        expect(skill).toContain("Agent memory is not PMS truth");
        expect(skill).toContain(
            "Booking, payment, and room availability truth must come from CapyInn read tools",
        );
        expect(skill).toContain(
            "Memory may store preferences, summaries, service context, city recommendations",
        );
        expect(skill).toContain(
            "Memory must not store canonical booking state, payment truth, room availability truth, or auto-mutating recovery commands",
        );
    });

    it("keeps the MCP manifest loopback-only and source-classed", () => {
        const manifest = JSON.parse(readWorkspaceFile("skills/mcp-manifest.json"));

        expect(manifest.transport["streamable-http"].url).toBe(
            "http://127.0.0.1:<gateway-port>/mcp",
        );
        expect(manifest.observer.sse.url).toBe(
            "http://127.0.0.1:<gateway-port>/observer/events",
        );
        expect(manifest.security.gateway_exposure.default_bind).toBe("127.0.0.1");
        expect(manifest.security.gateway_exposure.remote_enabled_by_default).toBe(false);
        expect(manifest.security.gateway_exposure.remote_requires).toEqual([
            "explicit_configuration",
            "auth_or_pairing",
            "policy_gate",
        ]);
        expect(
            manifest.security.gateway_exposure
                .high_risk_tools_policy_gated_even_when_authenticated,
        ).toBe(true);
        expect(manifest.data_sources.pms_truth.authoritative).toBe(true);
        expect(manifest.data_sources.observer_facts.authoritative).toBe(false);
        expect(manifest.data_sources.agent_memory.authoritative).toBe(false);
        expect(manifest.data_sources.agent_memory.forbidden).toContain(
            "canonical_booking_state",
        );
        expect(manifest.data_sources.agent_memory.forbidden).toContain(
            "room_availability_truth",
        );
        expect(manifest.data_sources.agent_memory.forbidden).toContain(
            "payment_truth",
        );
        expect(manifest.data_sources.agent_memory.forbidden).toContain(
            "auto_mutating_recovery_commands",
        );
    });

    it("documents loopback deployment assumptions in OpenAPI", () => {
        const openapi = readWorkspaceFile("skills/openapi.yaml");

        expect(openapi).toContain("Loopback-only by default");
        expect(openapi).toContain(
            "SQLite-backed PMS tools are the source of truth",
        );
        expect(openapi).toContain("Agent memory is non-authoritative");
        expect(openapi).toContain(
            "LAN or remote exposure requires explicit configuration, auth or pairing, and policy gates",
        );
    });

    it("documents CEO agent data sensitivity and cloud opt-in boundaries", () => {
        const skill = readWorkspaceFile("skills/hotel-manager.skill.md");
        const openapi = readWorkspaceFile("skills/openapi.yaml");
        const manifest = JSON.parse(readWorkspaceFile("skills/mcp-manifest.json"));

        expect(skill).toContain("ReadOnly does not mean guest-safe");
        expect(skill).toContain("CEO cloud-data opt-in is required");
        expect(skill).toContain(
            "raw prompts, raw responses, raw tool outputs, and raw provider errors are not stored",
        );

        expect(openapi).toContain("PublicHotelInfo");
        expect(openapi).toContain("GuestScoped");
        expect(openapi).toContain("StaffOperational");
        expect(openapi).toContain("CeoSensitive");
        expect(openapi).toContain(
            "CEO cloud-data opt-in is persisted and revocable",
        );

        expect(manifest.agent_safety.data_sensitivity_classes).toEqual([
            "PublicHotelInfo",
            "GuestScoped",
            "StaffOperational",
            "CeoSensitive",
        ]);
        expect(manifest.agent_safety.read_only_not_guest_safe).toBe(true);
        expect(manifest.agent_safety.cloud_data_opt_in.setting_key).toBe(
            "ceo_cloud_data_opt_in",
        );
        expect(manifest.agent_safety.cloud_data_opt_in.default).toBe(false);
        expect(manifest.agent_safety.retention.raw_prompt_retention).toBe(
            "not_stored",
        );
        expect(manifest.agent_safety.retention.raw_response_retention).toBe(
            "not_stored",
        );
        expect(manifest.agent_safety.retention.raw_tool_output_retention).toBe(
            "not_stored",
        );
        expect(
            manifest.agent_safety.retention.raw_provider_error_retention,
        ).toBe("not_stored");
    });
});
