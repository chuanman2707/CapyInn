import fs from "node:fs";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

const packageJson = JSON.parse(
  fs.readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string };

const appVersion = packageJson.version;

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
    __UPDATER_ENABLED__: JSON.stringify(false),
    __EXPERIMENTAL_GATEWAY_UI__: JSON.stringify(false),
    __SENTRY_DSN__: JSON.stringify(""),
    __SENTRY_RELEASE__: JSON.stringify(`capyinn@${appVersion}`),
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
    // Pinned because several tests exist precisely to prove the app reads the
    // *local* calendar day rather than a UTC-converted one. CI runs at UTC,
    // where those two are the same string and the tests quietly stop
    // discriminating. This is the product's own timezone.
    env: {
      TZ: "Asia/Ho_Chi_Minh",
    },
  },
});
