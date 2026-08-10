/**
 * Mock for @tauri-apps/api/core
 *
 * All Tauri invoke() calls are intercepted here.
 * Tests can override responses via `setMockResponse()`.
 */
import { vi } from "vitest";

type MockHandler = (args?: Record<string, unknown>) => unknown;

const globalState = globalThis as typeof globalThis & {
    __tauriCoreMockResponses__?: Record<string, MockHandler>;
};

const mockResponses =
    globalState.__tauriCoreMockResponses__ ?? (globalState.__tauriCoreMockResponses__ = {});

/** Set a mock response for a specific Tauri command */
export function setMockResponse(command: string, handler: MockHandler) {
    mockResponses[command] = handler;
}

/** Set multiple mock responses at once */
export function setMockResponses(responses: Record<string, MockHandler>) {
    Object.assign(mockResponses, responses);
}

/** Clear all mock responses */
export function clearMockResponses() {
    Object.keys(mockResponses).forEach((key) => delete mockResponses[key]);
}

/** The mocked invoke function */
export const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    if (mockResponses[command]) {
        return mockResponses[command](args);
    }

    // Default responses for common commands (prevents test crashes)
    const defaults: Record<string, unknown> = {
        get_bootstrap_status: {
            setup_completed: true,
            app_lock_enabled: true,
            current_user: null,
        },
        get_crash_reporting_preference: false,
        set_crash_reporting_preference: undefined,
        get_ceo_cloud_data_opt_in: false,
        set_ceo_cloud_data_opt_in: undefined,
        get_ceo_telegram_config: {
            runtime_enabled: false,
            telegram_user_id: null,
            telegram_bot_token_present: false,
            openai_api_key_present: false,
            openai_model: "gpt-5",
            last_update_id: null,
        },
        get_ceo_digest_config: {
            digest_enabled: false,
            telegram_user_id: null,
            telegram_delivery_chat_id: null,
            telegram_bot_token_present: false,
            openai_api_key_present: false,
            openai_model: "gpt-5",
        },
        set_ceo_telegram_config: undefined,
        set_ceo_digest_config: {
            digest_enabled: false,
            telegram_user_id: null,
            telegram_delivery_chat_id: null,
            telegram_bot_token_present: false,
            openai_api_key_present: false,
            openai_model: "gpt-5",
        },
        set_ceo_telegram_bot_token: undefined,
        clear_ceo_telegram_bot_token: undefined,
        set_ceo_openai_api_key: undefined,
        clear_ceo_openai_api_key: undefined,
        get_ceo_telegram_gate_status: { ready: false, missing: ["runtime_enabled"] },
        get_ceo_digest_gate_status: { ready: false, missing: ["digest_enabled"] },
        get_experimental_runtime_status: {
            experimental_runtime_enabled: false,
            gateway_runtime_enabled: false,
            agent_runtime_enabled: false,
            gateway_disabled_by_override: false,
            agent_disabled_by_override: false,
        },
        local_receptionist_chat: {
            answer: "Local Gemma can answer with hotel pricing context.",
            provider: "local",
            model: "capyinn-gemma4-e2b-q5km",
        },
        complete_onboarding: {
            setup_completed: true,
            app_lock_enabled: false,
            current_user: {
                id: "bootstrap-admin",
                name: "Owner",
                role: "admin",
                active: true,
                created_at: new Date().toISOString(),
            },
        },
        get_rooms: [],
        get_dashboard_stats: { total_rooms: 10, occupied: 0, vacant: 10, revenue_today: 0 },
        get_settings: null,
        get_current_user: null,
        gateway_get_status: { running: false, port: null, has_api_keys: false },
        get_all_bookings: [],
        get_all_guests: [],
        get_analytics: { period: "today", total_revenue: 0, total_expenses: 0, net_profit: 0, occupancy_rate: 0, rooms_sold: 0, avg_rate: 0, revenue_by_day: [], top_rooms: [], source_breakdown: [], daily_revenue: [] },
        get_recent_activity: [],
        get_revenue_stats: { total_revenue: 0, rooms_sold: 0, occupancy_rate: 0, daily_revenue: [] },
        get_expenses: [],
        get_pricing_rules: [],
        get_room_type_rates: [],
        get_special_dates: [],
        get_audit_logs: [],
        record_js_crash: undefined,
        get_pending_crash_report: null,
        mark_crash_report_submitted: undefined,
        mark_crash_report_dismissed: undefined,
        mark_crash_report_send_failed: undefined,
        export_crash_report: "",
        list_users: [],
        export_csv: "",
        export_bookings_csv: "",
        backup_database: "",
        gateway_generate_key: "",
        logout: undefined,
        search_guest_by_phone: [],
        calculate_price_preview: { total: 0, breakdown: [] },
        calculate_room_price_preview: { total: 0, breakdown: [] },
        get_folio_lines: [],
        get_rooms_availability: [],
    };

    if (command === "login") {
        throw {
            code: "AUTH_INVALID_PIN",
            message: "Mã PIN không đúng",
            kind: "user",
            support_id: null,
        };
    }

    if (command in defaults) {
        const val = defaults[command];
        if (val instanceof Error) throw val;
        return val;
    }

    throw new Error(`[tauri-mock] Unhandled invoke: "${command}" with args: ${JSON.stringify(args ?? null)}`);
});
