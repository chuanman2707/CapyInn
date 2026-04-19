import { useMemo, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

import type { AppUpdatePhase, AppUpdateState } from "@/types";

type Update = NonNullable<Awaited<ReturnType<typeof check>>>;

interface UseAppUpdateControllerOptions {
    enabled: boolean;
    supported: boolean;
    currentVersion: string;
    timeoutMs?: number;
}

interface CheckOptions {
    silent: boolean;
}

const DEFAULT_TIMEOUT_MS = 30_000;

function isWindows() {
    return /Windows/i.test(window.navigator.userAgent);
}

function normalizeErrorMessage(error: unknown) {
    return error instanceof Error ? error.message : String(error);
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number) {
    let timer: number | undefined;

    const settled = promise.then(
        (value) => ({ status: "fulfilled" as const, value }),
        (error) => ({ status: "rejected" as const, error }),
    );

    const timeout = new Promise<{ status: "timeout" }>((resolve) => {
        timer = window.setTimeout(() => {
            resolve({ status: "timeout" });
        }, timeoutMs);
    });

    return Promise.race([settled, timeout]).then((result) => {
        if (timer !== undefined) {
            window.clearTimeout(timer);
        }

        if (result.status === "timeout") {
            throw new Error(`timeout after ${timeoutMs}ms`);
        }

        if (result.status === "rejected") {
            throw result.error;
        }

        return result.value;
    });
}

function mapInstallFailure(error: unknown): { phase: AppUpdatePhase; message: string } {
    const message = normalizeErrorMessage(error);

    if (/signature/i.test(message) || /404/i.test(message) || /manifest/i.test(message)) {
        return { phase: "idle", message };
    }

    return { phase: "available", message };
}

export function useAppUpdateController({
    enabled,
    supported,
    currentVersion,
    timeoutMs = DEFAULT_TIMEOUT_MS,
}: UseAppUpdateControllerOptions): AppUpdateState & {
    canCheck: boolean;
    checkForUpdates: (options: CheckOptions) => Promise<void>;
    downloadUpdate: () => Promise<void>;
    dismissRestartPrompt: () => void;
    openRestartPrompt: () => void;
    confirmInstall: () => Promise<void>;
} {
    const [phase, setPhase] = useState<AppUpdatePhase>("idle");
    const [availableVersion, setAvailableVersion] = useState<string | null>(null);
    const [errorMessage, setErrorMessage] = useState<string | null>(null);
    const [restartPromptOpen, setRestartPromptOpen] = useState(false);
    const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);

    const canCheck =
        supported &&
        enabled &&
        phase !== "checking" &&
        phase !== "downloading" &&
        phase !== "installing";

    async function checkForUpdates({ silent }: CheckOptions) {
        if (!canCheck) {
            return;
        }

        setPhase("checking");
        setErrorMessage(null);

        try {
            const update = await withTimeout(check(), timeoutMs);

            if (!update) {
                setPendingUpdate(null);
                setAvailableVersion(null);
                setPhase("idle");
                return;
            }

            setPendingUpdate(update);
            setAvailableVersion(update.version);
            setPhase("available");
        } catch (error) {
            setPhase("idle");

            if (!silent) {
                setErrorMessage(normalizeErrorMessage(error));
            }
        }
    }

    async function downloadUpdate() {
        if (!pendingUpdate || (phase !== "available" && phase !== "error")) {
            return;
        }

        setPhase("downloading");
        setErrorMessage(null);

        try {
            await withTimeout(pendingUpdate.download(), timeoutMs);
            setPhase("downloaded");
            setRestartPromptOpen(true);
        } catch (error) {
            setPhase("available");
            setErrorMessage(normalizeErrorMessage(error));
        }
    }

    function dismissRestartPrompt() {
        setRestartPromptOpen(false);
    }

    function openRestartPrompt() {
        if (phase === "downloaded") {
            setRestartPromptOpen(true);
        }
    }

    async function confirmInstall() {
        if (!pendingUpdate || phase !== "downloaded") {
            return;
        }

        setPhase("installing");
        setRestartPromptOpen(false);
        setErrorMessage(null);

        try {
            await withTimeout(pendingUpdate.install(), timeoutMs);

            if (!isWindows()) {
                await relaunch();
            }
        } catch (error) {
            const mapped = mapInstallFailure(error);
            setPhase(mapped.phase);
            setErrorMessage(mapped.message);
        }
    }

    return useMemo(
        () => ({
            supported,
            phase,
            currentVersion,
            availableVersion,
            errorMessage,
            restartPromptOpen,
            canCheck,
            checkForUpdates,
            downloadUpdate,
            dismissRestartPrompt,
            openRestartPrompt,
            confirmInstall,
        }),
        [
            canCheck,
            currentVersion,
            availableVersion,
            checkForUpdates,
            confirmInstall,
            dismissRestartPrompt,
            downloadUpdate,
            errorMessage,
            openRestartPrompt,
            phase,
            restartPromptOpen,
            supported,
        ],
    );
}
