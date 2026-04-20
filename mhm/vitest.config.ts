import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
    define: {
        __APP_VERSION__: JSON.stringify("0.1.0"),
        __UPDATER_ENABLED__: JSON.stringify(true),
        __SENTRY_DSN__: JSON.stringify(""),
        __SENTRY_RELEASE__: JSON.stringify("capyinn@0.1.0"),
        __SENTRY_ENVIRONMENT__: JSON.stringify("development"),
    },
    plugins: [react()],
    resolve: {
        alias: {
            "@": path.resolve(__dirname, "./src"),
            "@tauri-apps/api/core": path.resolve(__dirname, "./src/__mocks__/tauri-core.ts"),
            "@tauri-apps/api/event": path.resolve(__dirname, "./src/__mocks__/tauri-event.ts"),
            "@tauri-apps/plugin-updater": path.resolve(__dirname, "./src/__mocks__/tauri-updater.ts"),
            "@tauri-apps/plugin-process": path.resolve(__dirname, "./src/__mocks__/tauri-process.ts"),
            "@test-mocks": path.resolve(__dirname, "./src/__mocks__"),
        },
    },
    test: {
        globals: true,
        environment: "jsdom",
        setupFiles: ["./tests/setup.ts"],
        include: ["tests/**/*.test.{ts,tsx}", "src/**/*.test.{ts,tsx}"],
        css: false,
        reporters: ["verbose"],
    },
});
