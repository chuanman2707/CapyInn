import { run } from "./shared.mjs";

const cwd = process.cwd();
const env = { CAPYINN_DISABLE_CEO_TELEGRAM: "true" };

await run(
  "agent-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "agent::", "--", "--nocapture"],
  { cwd, env },
);

await run(
  "agent-settings-tests",
  "cargo",
  [
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "commands::agent_settings::tests::",
    "--",
    "--nocapture",
  ],
  { cwd, env },
);

await run(
  "ceo-agent-settings-ui",
  "npm",
  ["test", "--", "--run", "src/pages/settings/CeoAgentSection.test.tsx"],
  { cwd, env },
);
