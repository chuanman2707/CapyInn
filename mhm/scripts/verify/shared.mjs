import { spawn } from "node:child_process";
import { createWriteStream } from "node:fs";
import { mkdir, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export const runtimeRoot = path.join(os.homedir(), "CapyInn-TestSuite");
export const artifactsRoot = path.join(runtimeRoot, "artifacts");

export async function resetRuntimeRoot() {
  await rm(runtimeRoot, { recursive: true, force: true });
  await mkdir(artifactsRoot, { recursive: true });
}

export function verificationEnv(extra = {}) {
  return {
    ...process.env,
    CAPYINN_RUNTIME_ROOT: runtimeRoot,
    CAPYINN_DISABLE_GATEWAY: "true",
    CAPYINN_DISABLE_WATCHER: "true",
    CAPYINN_ENABLE_UPDATER: "false",
    ...extra,
  };
}

export async function spawnLoggedProcess(label, command, args, options = {}) {
  await mkdir(artifactsRoot, { recursive: true });
  const logPath = path.join(artifactsRoot, `${label}.log`);
  const log = createWriteStream(logPath, { flags: "w" });
  const child = spawn(command, args, {
    cwd: options.cwd,
    env: verificationEnv(options.env),
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
  });

  const forward = (stream, writer) => {
    if (!stream) {
      return;
    }
    stream.on("data", (chunk) => {
      writer.write(chunk);
      log.write(chunk);
    });
  };

  forward(child.stdout, process.stdout);
  forward(child.stderr, process.stderr);

  const closeLog = () => {
    if (!log.destroyed) {
      log.end();
    }
  };

  child.on("exit", closeLog);
  child.on("error", closeLog);

  return { child, logPath };
}

export async function run(label, command, args, options = {}) {
  const { child, logPath } = await spawnLoggedProcess(label, command, args, options);

  return await new Promise((resolve, reject) => {
    child.on("error", (error) => {
      reject(new Error(`${label} failed to start: ${error.message}`));
    });

    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(
        new Error(
          `${label} exited with ${code ?? signal ?? "unknown status"}; see ${logPath}`,
        ),
      );
    });
  });
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function terminateChild(child, timeoutMs = 5_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }

  const waitForExit = new Promise((resolve) => {
    child.once("exit", resolve);
  });

  const signalTree = (signal) => {
    if (process.platform !== "win32" && child.pid) {
      try {
        process.kill(-child.pid, signal);
        return;
      } catch {
        // Fall back to the direct child kill below.
      }
    }

    child.kill(signal);
  };

  signalTree("SIGTERM");
  const gracefulExit = await Promise.race([
    waitForExit.then(() => true),
    sleep(timeoutMs).then(() => false),
  ]);

  if (!gracefulExit && child.exitCode === null) {
    signalTree("SIGKILL");
    await waitForExit;
  }
}
