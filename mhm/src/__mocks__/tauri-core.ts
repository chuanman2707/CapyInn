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
        get_dashboard_stats: { total_rooms: 10, occupied: 0, vacant: 10, cleaning: 0, revenue_today: 0 },
        get_settings: null,
        get_current_user: null,
        gateway_get_status: { running: false, port: null, has_api_keys: false },
        get_housekeeping_tasks: [],
        get_all_bookings: [],
        get_all_guests: [],
        get_analytics: { period: "today", total_revenue: 0, total_expenses: 0, net_profit: 0, occupancy_rate: 0, rooms_sold: 0, avg_rate: 0, revenue_by_day: [], top_rooms: [], source_breakdown: [], daily_revenue: [] },
        get_recent_activity: [],
        get_revenue_stats: { total_revenue: 0, rooms_sold: 0, occupancy_rate: 0, daily_revenue: [] },
        get_expenses: [],
        get_pricing_rules: [],
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
