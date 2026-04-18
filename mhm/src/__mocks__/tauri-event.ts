/**
 * Mock for @tauri-apps/api/event
 */
import { vi } from "vitest";

type EventCallback<T = unknown> = (event: { payload: T }) => void | Promise<void>;

const globalState = globalThis as typeof globalThis & {
    __tauriEventListeners__?: Map<string, Set<EventCallback>>;
};

const listeners =
    globalState.__tauriEventListeners__ ?? (globalState.__tauriEventListeners__ = new Map());

export const listen = vi.fn(async (eventName: string, callback: EventCallback) => {
    let eventListeners = listeners.get(eventName);
    if (!eventListeners) {
        eventListeners = new Set();
        listeners.set(eventName, eventListeners);
    }

    eventListeners.add(callback);

    return () => {
        eventListeners?.delete(callback);
        if (eventListeners?.size === 0) {
            listeners.delete(eventName);
        }
    };
});

export const emit = vi.fn(async (eventName: string, payload?: unknown) => {
    await emitTestEvent(eventName, payload);
});

export const once = vi.fn(async (eventName: string, callback: EventCallback) => {
    const unlisten = await listen(eventName, async (event) => {
        unlisten();
        await callback(event);
    });

    return unlisten;
});

export async function emitTestEvent<T>(eventName: string, payload: T) {
    const eventListeners = [...(listeners.get(eventName) ?? [])];
    for (const callback of eventListeners) {
        await callback({ payload });
    }
}

export function resetEventMocks() {
    listeners.clear();
    listen.mockClear();
    emit.mockClear();
    once.mockClear();
}
