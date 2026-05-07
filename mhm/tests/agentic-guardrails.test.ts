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

    it("keeps CEO digest runtime bounded to the fixed read registry", () => {
        const runtime = readWorkspaceFile("src-tauri/src/agent/digest/runtime.rs");

        expect(runtime).toContain("pub const CEO_DIGEST_TOOL_NAMES");
        expect(runtime).toContain("crate::agent::registry::CEO_PHASE_A_TOOLS");
        expect(runtime).toContain(
            "digest_tool_list_matches_phase_one_read_registry",
        );
        expect(runtime).toMatch(
            /ProviderRequest::new\([\s\S]*CEO_DIGEST_SYSTEM_PROMPT[\s\S]*Vec::new\(\)/,
        );
        expect(runtime).toContain("ProviderTurn::ToolCalls");
        expect(runtime).toContain("codes::AGENT_TOOL_NOT_ALLOWED");
    });

    it("keeps Telegram CEO channel sender denials before runtime calls", () => {
        const telegram = readWorkspaceFile(
            "src-tauri/src/agent/channel/telegram.rs",
        );

        expect(telegram).toContain(
            "unknown_sender_gets_denial_without_runtime_call",
        );
        expect(telegram).toContain(
            "missing_sender_gets_denial_without_runtime_call",
        );
        expect(telegram).toContain("assert_eq!(runtime.call_count(), 0)");
        expect(telegram).toContain("owner_denial_message(sender.id)");
        expect(telegram).toContain("MISSING_SENDER_DENIAL.to_string()");
    });

    it("keeps agent secret scrubbing for Telegram bot URLs", () => {
        const secrets = readWorkspaceFile("src-tauri/src/agent/secrets.rs");

        expect(secrets).toContain("redaction_removes_telegram_bot_url_tokens");
        expect(secrets).toContain("redaction_preserves_benign_botanical_urls");
        expect(secrets).toContain("redaction_preserves_telegram_api_botanical_urls");
        expect(secrets).toContain("api\\.telegram\\.org");
        expect(secrets).toContain("bot\\d+:");
        expect(secrets).toContain("bot[redacted]");
    });

    it("keeps CEO chat and digest runtime business tables read-only", () => {
        const chatRuntime = readWorkspaceFile(
            "src-tauri/src/agent/runtime/ceo_chat.rs",
        );
        const digestRuntime = readWorkspaceFile(
            "src-tauri/src/agent/digest/runtime.rs",
        );

        expect(chatRuntime).toContain(
            "business_table_snapshots_are_unchanged_across_mocked_chat_turn",
        );
        expect(chatRuntime).toContain(
            "business_table_snapshots_detect_existing_chat_row_updates",
        );
        expect(chatRuntime).toContain("phase_one_pms_table_snapshots");
        expect(digestRuntime).toContain(
            "digest_runtime_does_not_mutate_business_tables",
        );
        expect(digestRuntime).toContain("business_table_snapshots");
        expect(digestRuntime).toContain(
            "business_table_snapshots_detect_existing_row_updates",
        );
        expect(digestRuntime).toContain(
            "digest_business_fixture_covers_digest_source_tables",
        );
        expect(digestRuntime).toContain(
            "digest_snapshot_includes_outbox_and_agent_memory_tables",
        );
    });

    it("keeps CEO digest readiness cases in the phase gate", () => {
        const digestConfig = readWorkspaceFile(
            "src-tauri/src/agent/digest/config.rs",
        );
        const settingsUi = readWorkspaceFile(
            "src/pages/settings/CeoAgentSection.tsx",
        );

        expect(digestConfig).toContain("digest_gate_requires_openai_model");
        expect(digestConfig).toContain(
            "digest_gate_reports_every_missing_dependency",
        );
        expect(digestConfig).toContain(
            "digest_gate_does_not_require_chat_runtime_enabled",
        );
        expect(digestConfig).toContain("digest_gate_requires_delivery_chat_id");
        expect(digestConfig).toContain("CeoDigestGateMissing::OpenAiModel");
        expect(settingsUi).toContain('open_ai_model: "OpenAI model"');
    });

    it("keeps CEO agent final replies behind secret redaction", () => {
        const chatRuntime = readWorkspaceFile(
            "src-tauri/src/agent/runtime/ceo_chat.rs",
        );
        const digestRuntime = readWorkspaceFile(
            "src-tauri/src/agent/digest/runtime.rs",
        );

        expect(chatRuntime).toContain(
            "final_reply_redacts_secret_like_markers_before_delivery",
        );
        expect(chatRuntime).toContain("redact_agent_secret_markers(&text)");
        expect(digestRuntime).toContain(
            "digest_final_reply_redacts_secret_like_markers_before_telegram_send",
        );
        expect(digestRuntime).toContain("redact_agent_secret_markers(&text)");
    });

    it("runs agent guardrails from verify:agent with Telegram disabled", () => {
        const verifyAgent = readWorkspaceFile("scripts/verify/agent.mjs");

        expect(verifyAgent).toContain(
            'const env = { CAPYINN_DISABLE_CEO_TELEGRAM: "true" }',
        );
        expect(verifyAgent).toContain('"agent-guardrails-tests"');
        expect(verifyAgent).toContain('"tests/agentic-guardrails.test.ts"');
    });
});
