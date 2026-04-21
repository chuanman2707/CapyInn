import { resetRuntimeRoot, run } from "./shared.mjs";

const cwd = process.cwd();

await resetRuntimeRoot();
await run("quick", "npm", ["run", "verify:quick"], { cwd });
await run("frontend-suite", "npm", ["test"], { cwd });
await run(
  "booking-scenarios",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "services::booking::tests::", "--", "--nocapture"],
  { cwd },
);
await run(
  "backup-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "backup::tests::", "--", "--nocapture"],
  { cwd },
);
await run("native-smoke", "node", ["./scripts/verify/native-smoke.mjs"], { cwd });
