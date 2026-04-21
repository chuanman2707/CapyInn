import { readFile, rm } from "node:fs/promises";
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

await rm(readyFile, { force: true });

const { child, logPath } = await spawnLoggedProcess(
  "native-smoke-app",
  "npm",
  ["run", "tauri", "--", "dev", "--no-watch"],
  {
    cwd,
    env: {
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
