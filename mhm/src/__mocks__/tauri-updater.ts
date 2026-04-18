import { vi } from "vitest";

type MockUpdateConfig = {
    version: string;
    body?: string | null;
    date?: string | null;
    downloadDelayMs?: number;
    downloadError?: Error;
    installError?: Error;
};

type MockUpdate = {
    version: string;
    body: string | null;
    date: string | null;
    download: () => Promise<void>;
    install: () => Promise<void>;
};

let mockUpdate: MockUpdateConfig | null = null;
let checkError: Error | null = null;

function sleep(ms: number) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

export function setMockAvailableUpdate(config: MockUpdateConfig) {
    mockUpdate = config;
    checkError = null;
}

export function setMockCheckError(error: Error) {
    checkError = error;
}

export function clearMockUpdate() {
    mockUpdate = null;
    checkError = null;
}

export const check = vi.fn(async () => {
    if (checkError) {
        throw checkError;
    }

    if (!mockUpdate) {
        return null;
    }

    const update: MockUpdate = {
        version: mockUpdate.version,
        body: mockUpdate.body ?? null,
        date: mockUpdate.date ?? null,
        async download() {
            if (mockUpdate?.downloadDelayMs) {
                await sleep(mockUpdate.downloadDelayMs);
            }

            if (mockUpdate?.downloadError) {
                throw mockUpdate.downloadError;
            }
        },
        async install() {
            if (mockUpdate?.installError) {
                throw mockUpdate.installError;
            }
        },
    };

    return update;
});
