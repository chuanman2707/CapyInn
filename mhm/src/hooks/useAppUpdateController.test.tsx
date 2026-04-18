import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { relaunch } from "@/__mocks__/tauri-process";
import {
  clearMockUpdate,
  setMockAvailableUpdate,
  setMockCheckError,
} from "@/__mocks__/tauri-updater";

import { useAppUpdateController } from "./useAppUpdateController";

function setUserAgent(value: string) {
  Object.defineProperty(window.navigator, "userAgent", {
    value,
    configurable: true,
  });
}

describe("useAppUpdateController", () => {
  beforeEach(() => {
    clearMockUpdate();
    vi.useFakeTimers();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("downloads an available update, opens the restart modal, and keeps the update pending after Later", async () => {
    setUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    setMockAvailableUpdate({ version: "0.2.0" });

    const { result } = renderHook(() =>
      useAppUpdateController({ enabled: true, currentVersion: "0.1.0" }),
    );

    await act(async () => {
      await result.current.checkForUpdates({ silent: true });
    });
    expect(result.current.phase).toBe("available");

    await act(async () => {
      await result.current.downloadUpdate();
    });

    expect(result.current.phase).toBe("downloaded");
    expect(result.current.restartPromptOpen).toBe(true);

    act(() => {
      result.current.dismissRestartPrompt();
    });

    expect(result.current.phase).toBe("downloaded");
    expect(result.current.restartPromptOpen).toBe(false);
    expect(result.current.availableVersion).toBe("0.2.0");
  });

  it("maps a signature verification failure to idle and requires a fresh check", async () => {
    setUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    setMockAvailableUpdate({
      version: "0.2.0",
      installError: new Error("signature verification failed"),
    });

    const { result } = renderHook(() =>
      useAppUpdateController({ enabled: true, currentVersion: "0.1.0" }),
    );

    await act(async () => {
      await result.current.checkForUpdates({ silent: true });
    });

    await act(async () => {
      await result.current.downloadUpdate();
    });

    await act(async () => {
      await result.current.confirmInstall();
    });

    expect(result.current.phase).toBe("idle");
    expect(result.current.errorMessage).toMatch(/signature/i);
  });

  it("installs and relaunches on macOS when restart is confirmed", async () => {
    setUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    setMockAvailableUpdate({ version: "0.2.0" });

    const { result } = renderHook(() =>
      useAppUpdateController({ enabled: true, currentVersion: "0.1.0" }),
    );

    await act(async () => {
      await result.current.checkForUpdates({ silent: true });
    });

    await act(async () => {
      await result.current.downloadUpdate();
    });

    await act(async () => {
      await result.current.confirmInstall();
    });

    expect(relaunch).toHaveBeenCalledTimes(1);
  });

  it("does not call relaunch on Windows because installer takes over after install", async () => {
    setUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
    setMockAvailableUpdate({ version: "0.2.0" });

    const { result } = renderHook(() =>
      useAppUpdateController({ enabled: true, currentVersion: "0.1.0" }),
    );

    await act(async () => {
      await result.current.checkForUpdates({ silent: true });
    });

    await act(async () => {
      await result.current.downloadUpdate();
    });

    await act(async () => {
      await result.current.confirmInstall();
    });

    expect(relaunch).not.toHaveBeenCalled();
    expect(result.current.phase).toBe("installing");
  });

  it("returns to available after a download timeout so the user can retry", async () => {
    setUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
    setMockAvailableUpdate({
      version: "0.2.0",
      downloadDelayMs: 31_000,
    });

    const { result } = renderHook(() =>
      useAppUpdateController({ enabled: true, currentVersion: "0.1.0", timeoutMs: 30_000 }),
    );

    await act(async () => {
      await result.current.checkForUpdates({ silent: true });
    });

    await act(async () => {
      const promise = result.current.downloadUpdate();
      vi.advanceTimersByTime(30_001);
      await promise;
    });

    expect(result.current.phase).toBe("available");
    expect(result.current.errorMessage).toMatch(/timeout/i);
  });

  it("keeps auto-check failures silent but stores a visible error for manual checks", async () => {
    setMockCheckError(new Error("manifest 404"));

    const { result } = renderHook(() =>
      useAppUpdateController({ enabled: true, currentVersion: "0.1.0" }),
    );

    await act(async () => {
      await result.current.checkForUpdates({ silent: true });
    });
    expect(result.current.errorMessage).toBeNull();

    await act(async () => {
      await result.current.checkForUpdates({ silent: false });
    });
    expect(result.current.errorMessage).toMatch(/404/i);
  });
});
