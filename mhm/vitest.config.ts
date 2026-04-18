import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
    plugins: [react()],
    resolve: {
        alias: {
            "@": path.resolve(__dirname, "./src"),
            "@tauri-apps/api/core": path.resolve(__dirname, "./src/__mocks__/tauri-core.ts"),
            "@tauri-apps/api/event": path.resolve(__dirname, "./src/__mocks__/tauri-event.ts"),
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
