import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import {
  artifactsRoot,
  runtimeRoot,
  sleep,
  spawnLoggedProcess,
  terminateChild,
} from "./shared.mjs";

const cwd = process.cwd();
const readyFile = path.join(artifactsRoot, "smoke-ready.json");
const smokeConfigPath = path.join(artifactsRoot, "tauri.smoke.conf.json");
const baseConfigPath = path.join(cwd, "src-tauri", "tauri.conf.json");
const smokeUpdaterPubkey =
  process.env.CAPYINN_SMOKE_UPDATER_PUBLIC_KEY ??
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDY2QzJBRjMwN0JDODIxRjAKUldUd0ljaDdNSy9DWnFydkdtM3BVQ1g0WXh2aWo5S0NkejhMbkxkeER2clZRVHFHVWtzZHJnZzMK";

await rm(readyFile, { force: true });
await mkdir(artifactsRoot, { recursive: true });

const baseConfig = JSON.parse(await readFile(baseConfigPath, "utf8"));
const smokeConfig = {
  ...baseConfig,
  plugins: {
    ...(baseConfig.plugins ?? {}),
    updater: {
      ...(baseConfig.plugins?.updater ?? {}),
      pubkey: smokeUpdaterPubkey,
    },
  },
};

await writeFile(smokeConfigPath, `${JSON.stringify(smokeConfig, null, 2)}\n`);

const { child, logPath } = await spawnLoggedProcess(
  "native-smoke-app",
  "npm",
  ["run", "tauri", "--", "dev", "--no-watch", "--config", smokeConfigPath],
  {
    cwd,
    env: {
      CAPYINN_ENABLE_UPDATER: "1",
      CAPYINN_SMOKE_READY_FILE: readyFile,
    },
  },
);

let ready = false;

try {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (child.exitCode !== null) {
      throw new Error(`native smoke exited early with ${child.exitCode}; see ${logPath}`);
    }

    try {
      const payload = JSON.parse(await readFile(readyFile, "utf8"));
      if (payload.status !== "ready") {
        throw new Error(`unexpected smoke payload in ${readyFile}`);
      }
      if (payload.runtime_root !== runtimeRoot) {
        throw new Error(
          `native smoke used ${payload.runtime_root} instead of isolated root ${runtimeRoot}`,
        );
      }
      ready = true;
      break;
    } catch (error) {
      if (error?.code !== "ENOENT") {
        throw error;
      }
    }

    await sleep(1_000);
  }

  if (!ready) {
    throw new Error(`native smoke never became ready under ${runtimeRoot}; see ${logPath}`);
  }
} finally {
  await terminateChild(child);
}
