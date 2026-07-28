import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import * as ts from "typescript";
import { describe, expect, it } from "vitest";

type RawInvokeOccurrence = {
  command: string | undefined;
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
  backup_database: "system backup/export action, not a PMS business write wrapper target",
  calculate_room_price_preview: "read-only pricing preview lookup",
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
  get_experimental_runtime_status: "runtime profile read used to hide experimental surfaces",
  get_guest_history: "read-only guest history lookup",
  get_housekeeping_tasks: "read-only housekeeping task lookup",
  get_invoice: "read-only invoice lookup checked before falling back to generate_invoice",
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

const RAW_INVOKE_ALLOWED_NON_LITERAL_SITES: Record<string, string> = {
  "src/lib/command/invoke.ts": "central invoke wrapper dispatch boundary",
};

let rawInvokeOccurrencesCache: RawInvokeOccurrence[] | undefined;

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

function sourceKindForFile(file: string): ts.ScriptKind {
  return file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
}

function findTauriInvokeImports(sourceFile: ts.SourceFile): {
  invokeLocals: Set<string>;
  namespaceLocals: Set<string>;
} {
  const invokeLocals = new Set<string>();
  const namespaceLocals = new Set<string>();

  for (const statement of sourceFile.statements) {
    if (!ts.isImportDeclaration(statement)) {
      continue;
    }
    if (
      !ts.isStringLiteral(statement.moduleSpecifier) ||
      statement.moduleSpecifier.text !== "@tauri-apps/api/core"
    ) {
      continue;
    }

    const importClause = statement.importClause;
    const namedBindings = importClause?.namedBindings;
    if (!namedBindings) {
      continue;
    }

    if (ts.isNamespaceImport(namedBindings)) {
      namespaceLocals.add(namedBindings.name.text);
      continue;
    }

    for (const element of namedBindings.elements) {
      const importedName = element.propertyName?.text ?? element.name.text;
      if (importedName === "invoke") {
        invokeLocals.add(element.name.text);
      }
    }
  }

  return { invokeLocals, namespaceLocals };
}

function isImportedInvokeCall(
  expression: ts.Expression,
  invokeLocals: Set<string>,
  namespaceLocals: Set<string>,
): boolean {
  if (ts.isIdentifier(expression)) {
    return invokeLocals.has(expression.text);
  }

  return (
    ts.isPropertyAccessExpression(expression) &&
    expression.name.text === "invoke" &&
    ts.isIdentifier(expression.expression) &&
    namespaceLocals.has(expression.expression.text)
  );
}

function stringCommandFromExpression(expression: ts.Expression): string | undefined {
  if (ts.isStringLiteral(expression) || ts.isNoSubstitutionTemplateLiteral(expression)) {
    return expression.text;
  }

  return undefined;
}

function findRawInvokeOccurrencesInSource(
  file: string,
  source: string,
): RawInvokeOccurrence[] {
  const sourceFile = ts.createSourceFile(
    file,
    source,
    ts.ScriptTarget.Latest,
    true,
    sourceKindForFile(file),
  );
  const { invokeLocals, namespaceLocals } = findTauriInvokeImports(sourceFile);
  const occurrences: RawInvokeOccurrence[] = [];

  function visit(node: ts.Node): void {
    if (
      ts.isCallExpression(node) &&
      isImportedInvokeCall(node.expression, invokeLocals, namespaceLocals)
    ) {
      const command = node.arguments[0]
        ? stringCommandFromExpression(node.arguments[0])
        : undefined;

      occurrences.push({
        command,
        file: relative(process.cwd(), file),
        line: sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1,
      });
    }

    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return occurrences;
}

function findRawInvokeOccurrences(): RawInvokeOccurrence[] {
  return listSourceFiles(FRONTEND_SRC_ROOT).flatMap((file) => {
    const source = readFileSync(file, "utf8");
    return findRawInvokeOccurrencesInSource(file, source);
  });
}

function getRawInvokeOccurrences(): RawInvokeOccurrence[] {
  rawInvokeOccurrencesCache ??= findRawInvokeOccurrences();
  return rawInvokeOccurrencesCache;
}

function formatOccurrences(occurrences: RawInvokeOccurrence[]): string {
  return occurrences
    .map(
      ({ command, file, line }) =>
        `${command ?? "<non-literal command>"} at ${file}:${line}`,
    )
    .join("\n");
}

describe("frontend invoke wrapper guardrails", () => {
  it("detects aliased, namespaced, and typed raw Tauri invoke calls", () => {
    const source = `
      import { invoke, invoke as tauriInvoke } from "@tauri-apps/api/core";
      import * as tauriCore from "@tauri-apps/api/core";

      invoke<Result<Foo>>("save_pricing_rule");
      tauriInvoke("save_settings");
      tauriCore.invoke(\`update_housekeeping\`);
    `;

    expect(
      findRawInvokeOccurrencesInSource("src/example.ts", source).map(
        ({ command }) => command,
      ),
    ).toEqual([
      "save_pricing_rule",
      "save_settings",
      "update_housekeeping",
    ]);
  });

  it("flags raw Tauri invoke calls without literal command names", () => {
    const source = `
      import { invoke } from "@tauri-apps/api/core";

      const command = "save_settings";
      invoke(command);
    `;

    expect(
      findRawInvokeOccurrencesInSource("src/example.ts", source).map(
        ({ command }) => command ?? "<non-literal command>",
      ),
    ).toEqual(["<non-literal command>"]);
  });

  it("keeps Batch 1 PMS writes out of raw Tauri invoke calls", () => {
    const forbidden = getRawInvokeOccurrences().filter(({ command }) =>
      command ? PMS_WRITE_COMMANDS_REQUIRING_WRAPPER.has(command) : false,
    );

    expect(formatOccurrences(forbidden)).toBe("");
  });

  it("keeps every remaining raw Tauri invoke explicitly categorized", () => {
    const rawCommands = new Set(
      getRawInvokeOccurrences()
        .map(({ command }) => command)
        .filter((command): command is string => command !== undefined),
    );

    for (const [command, reason] of Object.entries(RAW_INVOKE_ALLOWED_COMMANDS)) {
      expect(reason, `${command} needs an allowlist reason`).toMatch(/\S.{10,}/);
      expect(rawCommands.has(command), `${command} is not a remaining raw invoke`).toBe(true);
    }

    const nonLiteralSites = new Set(
      getRawInvokeOccurrences()
        .filter(({ command }) => command === undefined)
        .map(({ file }) => file),
    );
    for (const [file, reason] of Object.entries(RAW_INVOKE_ALLOWED_NON_LITERAL_SITES)) {
      expect(reason, `${file} needs an allowlist reason`).toMatch(/\S.{10,}/);
      expect(nonLiteralSites.has(file), `${file} is not a remaining non-literal raw invoke`).toBe(
        true,
      );
    }

    const unknown = getRawInvokeOccurrences().filter(({ command, file }) =>
      command === undefined
        ? !(file in RAW_INVOKE_ALLOWED_NON_LITERAL_SITES)
        : !PMS_WRITE_COMMANDS_REQUIRING_WRAPPER.has(command) &&
          !(command in RAW_INVOKE_ALLOWED_COMMANDS),
    );

    expect(formatOccurrences(unknown)).toBe("");
  });
});
