import { run } from "./shared.mjs";

const cwd = process.cwd();

await run(
  "frontend-wave1-targets",
  "npm",
  [
    "test",
    "--",
    "src/lib/appError.test.ts",
    "src/App.updateFlow.test.tsx",
    "src/hooks/useAppUpdateController.test.tsx",
    "tests/e2e/01-login.test.tsx",
    "tests/e2e/08-settings.test.tsx",
  ],
  { cwd },
);

await run(
  "runtime-config-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "runtime_config::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "app-identity-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "app_identity::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "setup-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "services::setup::tests::", "--", "--nocapture"],
  { cwd },
);
