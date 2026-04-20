import { invoke } from "@tauri-apps/api/core";

import type { JsCrashReportInput } from "./types";

let installed = false;

function normalizeStacktrace(error: unknown): string[] {
  if (error instanceof Error && error.stack) {
    return error.stack.split("\n").map((line) => line.trim());
  }

  return [];
}

function record(report: JsCrashReportInput) {
  void invoke("record_js_crash", { report }).catch(() => {
    // Never throw from global crash capture.
  });
}

export function installGlobalCrashHandlers() {
  if (installed) {
    return;
  }

  window.addEventListener("error", (event) => {
    record({
      crash_type: "js_unhandled_error",
      message: event.message || "Unknown window error",
      stacktrace: normalizeStacktrace(event.error),
      module_hint: null,
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason instanceof Error ? event.reason : new Error(String(event.reason));
    record({
      crash_type: "unhandled_rejection",
      message: reason.message,
      stacktrace: normalizeStacktrace(reason),
      module_hint: null,
    });
  });

  installed = true;
}
