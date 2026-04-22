import { run } from "./shared.mjs";

const cwd = process.cwd();

await run(
  "frontend-wave1-targets",
  "npm",
  [
    "test",
    "--",
    "src/lib/appError.test.ts",
    "src/pages/settings/useRoomConfig.test.tsx",
    "src/components/GroupCheckinSheet.test.tsx",
    "src/pages/NightAudit.test.tsx",
    "src/App.updateFlow.test.tsx",
    "src/hooks/useAppUpdateController.test.tsx",
    "tests/e2e/01-login.test.tsx",
    "tests/e2e/03-checkin.test.tsx",
    "tests/e2e/05-checkout.test.tsx",
    "tests/e2e/08-settings.test.tsx",
    "tests/e2e/11-night-audit.test.tsx",
  ],
  { cwd },
);

await run(
  "app-error-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "app_error::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "command-helper-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "commands::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "audit-command-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "commands::audit::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "room-management-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "commands::room_management::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "stay-group-mapping-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "commands::rooms::tests::", "--", "--nocapture"],
  { cwd },
);

await run(
  "group-mapping-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "commands::groups::tests::", "--", "--nocapture"],
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
