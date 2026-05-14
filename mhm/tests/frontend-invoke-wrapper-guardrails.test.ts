import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

type RawInvokeOccurrence = {
  command: string;
  file: string;
  line: number;
};

const FRONTEND_SRC_ROOT = join(process.cwd(), "src");

const PMS_WRITE_COMMANDS_REQUIRING_WRAPPER = new Set([
  "save_pricing_rule",
  "save_settings",
  "update_housekeeping",
]);

const RAW_INVOKE_ALLOWED_COMMANDS: Record<string, string> = {
  auto_assign_rooms: "read-style room assignment preview; no PMS mutation is committed",
  backup_database: "system backup/export action, not a PMS business write wrapper target",
  calculate_price_preview: "read-only pricing preview",
  check_availability: "read-only reservation availability lookup",
  complete_onboarding: "bootstrap setup command excluded from Batch 1 PMS wrapper scope",
  export_bookings_csv: "system export action, not a PMS business write wrapper target",
  export_crash_report: "diagnostics export action",
  gateway_generate_key: "gateway runtime administration excluded from PMS write wrapper scope",
  gateway_get_status: "gateway runtime status read",
  generate_group_invoice: "read-only group invoice data generation; no invoice record is persisted",
  get_all_bookings: "read-only booking list lookup",
  get_all_groups: "read-only group list lookup",
  get_all_guests: "read-only guest list lookup",
  get_analytics: "read-only analytics lookup",
  get_audit_logs: "read-only audit lookup",
  get_bootstrap_status: "bootstrap runtime read",
  get_crash_reporting_preference: "diagnostics preference read",
  get_current_user: "auth session read",
  get_dashboard_stats: "read-only dashboard stats lookup",
  get_expenses: "read-only expense lookup",
  get_guest_history: "read-only guest history lookup",
  get_housekeeping_tasks: "read-only housekeeping task lookup",
  get_pending_crash_report: "diagnostics recovery read",
  get_pricing_rules: "read-only pricing rules lookup",
  get_recent_activity: "read-only activity feed lookup",
  get_room_detail: "read-only room detail lookup",
  get_room_types: "read-only room type lookup",
  get_rooms: "read-only room list lookup",
  get_rooms_availability: "read-only room availability lookup",
  get_settings: "read-only settings lookup",
  get_stay_info_text: "read-only stay info lookup",
  logout: "auth runtime action excluded from PMS write wrapper scope",
  mark_crash_report_dismissed: "diagnostics lifecycle action",
  mark_crash_report_send_failed: "diagnostics lifecycle action",
  mark_crash_report_submitted: "diagnostics lifecycle action",
  preview_checkout_settlement: "read-only checkout settlement preview",
  record_js_crash: "diagnostics crash recording path",
  search_guest_by_phone: "read-only guest search lookup",
  set_crash_reporting_preference: "diagnostics preference action excluded from PMS wrapper scope",
};

function listSourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    const stats = statSync(path);

    if (stats.isDirectory()) {
      if (entry === "__mocks__") {
        return [];
      }
      return listSourceFiles(path);
    }

    if (!/\.(ts|tsx)$/.test(entry) || /\.test\.(ts|tsx)$/.test(entry)) {
      return [];
    }

    return [path];
  });
}

function lineNumberForIndex(source: string, index: number): number {
  return source.slice(0, index).split("\n").length;
}

function findRawInvokeOccurrences(): RawInvokeOccurrence[] {
  const invokePattern = /\binvoke(?:<[^>]+>)?\(\s*["']([^"']+)["']/g;

  return listSourceFiles(FRONTEND_SRC_ROOT).flatMap((file) => {
    const source = readFileSync(file, "utf8");
    return Array.from(source.matchAll(invokePattern), (match) => ({
      command: match[1],
      file: relative(process.cwd(), file),
      line: lineNumberForIndex(source, match.index ?? 0),
    }));
  });
}

function formatOccurrences(occurrences: RawInvokeOccurrence[]): string {
  return occurrences
    .map(({ command, file, line }) => `${command} at ${file}:${line}`)
    .join("\n");
}

describe("frontend invoke wrapper guardrails", () => {
  it("keeps Batch 1 PMS writes out of raw Tauri invoke calls", () => {
    const forbidden = findRawInvokeOccurrences().filter(({ command }) =>
      PMS_WRITE_COMMANDS_REQUIRING_WRAPPER.has(command),
    );

    expect(formatOccurrences(forbidden)).toBe("");
  });

  it("keeps every remaining raw Tauri invoke explicitly categorized", () => {
    for (const [command, reason] of Object.entries(RAW_INVOKE_ALLOWED_COMMANDS)) {
      expect(reason, `${command} needs an allowlist reason`).toMatch(/\S.{10,}/);
    }

    const unknown = findRawInvokeOccurrences().filter(
      ({ command }) =>
        !PMS_WRITE_COMMANDS_REQUIRING_WRAPPER.has(command) &&
        !(command in RAW_INVOKE_ALLOWED_COMMANDS),
    );

    expect(formatOccurrences(unknown)).toBe("");
  });
});
