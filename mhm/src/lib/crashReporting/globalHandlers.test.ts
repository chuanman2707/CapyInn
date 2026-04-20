import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import { installGlobalCrashHandlers } from "./globalHandlers";
import { hasRemoteCrashReporting } from "./sentry";

describe("installGlobalCrashHandlers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("records window errors through Tauri", () => {
    installGlobalCrashHandlers();

    window.dispatchEvent(
      new ErrorEvent("error", {
        message: "boom",
        error: new Error("boom"),
      }),
    );

    expect(invoke).toHaveBeenCalledWith(
      "record_js_crash",
      expect.objectContaining({
        report: expect.objectContaining({
          crash_type: "js_unhandled_error",
          message: "boom",
        }),
      }),
    );
  });

  it("records unhandled promise rejections through Tauri", () => {
    installGlobalCrashHandlers();

    window.dispatchEvent(
      new PromiseRejectionEvent("unhandledrejection", {
        promise: Promise.resolve(),
        reason: new Error("async boom"),
      }),
    );

    expect(invoke).toHaveBeenCalledWith(
      "record_js_crash",
      expect.objectContaining({
        report: expect.objectContaining({
          crash_type: "unhandled_rejection",
          message: "async boom",
        }),
      }),
    );
  });

  it("treats an empty DSN as remote reporting disabled", () => {
    expect(hasRemoteCrashReporting()).toBe(false);
  });
});
