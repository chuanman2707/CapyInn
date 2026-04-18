/// <reference types="vite/client" />

declare const __APP_VERSION__: string;

declare module "@tauri-apps/plugin-updater" {
    export interface Update {
        version: string;
        body: string | null;
        date: string | null;
        download(): Promise<void>;
        install(): Promise<void>;
    }

    export function check(): Promise<Update | null>;
}

declare module "@tauri-apps/plugin-process" {
    export function relaunch(): Promise<void>;
}

declare module "*.mjs";

declare module "*.ttf" {
    const src: string;
    export default src;
}
