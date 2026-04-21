# CapyInn Verification Suite Wave 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ground CapyInn's verification strategy in the current repo by greening the existing baseline, adding deterministic runtime-test seams, shipping Wave 1 verification commands, and expanding the current GitHub-hosted CI with a stable first-wave check.

**Architecture:** Extend the existing CI and mocked app-flow suite instead of building a new stack. First stabilize the current red baseline, then add an explicit runtime contract (`CAPYINN_RUNTIME_ROOT`, watcher/gateway toggles, frozen time), script the verification entry points under `mhm/scripts/verify/`, add first-wave Rust business scenarios, and finish with a file-based native smoke probe and CI artifact capture.

**Tech Stack:** Tauri 2, Rust, SQLx, React 19, TypeScript, Vitest, Node.js scripts, GitHub Actions

---

## File Structure

- Create: `mhm/src-tauri/src/runtime_config.rs`
  Responsibility: parse verification env vars, expose `runtime_root_override()`, boolean feature toggles, and frozen-time parsing.
- Modify: `mhm/src-tauri/src/app_identity.rs`
  Responsibility: route all runtime paths through `runtime_config::runtime_root_override()` when set.
- Modify: `mhm/src-tauri/src/lib.rs`
  Responsibility: honor `CAPYINN_DISABLE_WATCHER`, `CAPYINN_DISABLE_GATEWAY`, and smoke-probe env vars during app startup.
- Modify: `mhm/src-tauri/src/backup.rs`
  Responsibility: read frozen time when present and keep backup naming deterministic in verification mode.
- Modify: `mhm/src-tauri/src/services/setup/tests.rs`
  Responsibility: extend onboarding/login bootstrap coverage for Wave 1.
- Modify: `mhm/src-tauri/src/services/booking/tests.rs`
  Responsibility: add reservation → check-in and checkout/night-audit Wave 1 scenarios using the existing SQLite-backed test helpers.
- Modify: `mhm/vitest.config.ts`
  Responsibility: derive app version and Sentry release from `package.json` and remove config drift from the test environment.
- Modify: `mhm/src/App.updateFlow.test.tsx`
  Responsibility: stabilize the current shell update-flow regression tests against the chosen update contract.
- Modify: `mhm/tests/e2e/08-settings.test.tsx`
  Responsibility: stabilize the current settings update-flow tests and keep the mocked app-flow suite green.
- Modify: `mhm/package.json`
  Responsibility: add `verify:quick`, `verify:full`, and `verify:repeat` entry points.
- Create: `mhm/scripts/verify/shared.mjs`
  Responsibility: common process spawning, env construction, artifact capture, and runtime root reset helpers.
- Create: `mhm/scripts/verify/quick.mjs`
  Responsibility: run the fast local signal suite.
- Create: `mhm/scripts/verify/full.mjs`
  Responsibility: orchestrate runtime reset, Rust scenarios, frontend app-flow tests, and native smoke.
- Create: `mhm/scripts/verify/repeat.mjs`
  Responsibility: repeat `verify:full` in clean isolated runs.
- Create: `mhm/scripts/verify/native-smoke.mjs`
  Responsibility: launch the app with a file-based smoke readiness probe, capture artifacts, and fail deterministically.
- Modify: `.github/workflows/ci.yml`
  Responsibility: add a Wave 1 verification job and upload artifacts from failed runs.

## Verification Note

The repo-wide frontend baseline is currently red in targeted update-flow areas, so Phase 0 and Task 1 intentionally start by using the existing failing tests as the red phase. Do not build new verification layers on top of a known-red baseline.

## Roadmap Context

This file implements `Wave 1` only.

The broader roadmap is:

- `Wave 1`: foundation plus first useful CI gate. This plan covers baseline cleanup, runtime isolation, verification entry points, core Rust scenarios, native smoke, and CI expansion.
- `Wave 2`: business coverage expansion after Wave 1 proves stable. Expected follow-on scope includes group flows, room moves, extend-stay and cancellation edges, deeper housekeeping and guest-history coverage, restore verification, and targeted analytics sanity checks.
- `Wave 3`: operational maturity after Wave 2 proves valuable. Expected follow-on scope includes richer artifacts, stronger release gating, optional broader platform coverage, and stricter runtime-budget and flake-control work.

The purpose of this split is to ship a verification system that becomes useful early, instead of waiting for exhaustive coverage.

---

### Task 1: Green The Existing Update And Settings Baseline

**Files:**
- Modify: `mhm/vitest.config.ts`
- Modify: `mhm/src/App.updateFlow.test.tsx`
- Modify: `mhm/tests/e2e/08-settings.test.tsx`
- Modify: `mhm/src/pages/settings/SoftwareUpdateSection.tsx`
- Modify: `mhm/src/hooks/useAppUpdateController.ts`

- [ ] **Step 1: Reproduce the current red baseline with the already-failing targeted tests**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- src/App.updateFlow.test.tsx tests/e2e/08-settings.test.tsx
```

Expected: FAIL in the known update-flow tests before any code change.

- [ ] **Step 2: Remove config drift from the Vitest environment before changing product code**

Update the Vitest define block so it derives version metadata from `package.json` instead of hardcoding `0.1.0`.

```ts
// mhm/vitest.config.ts
import fs from "node:fs";

const packageJson = JSON.parse(
  fs.readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string };

const appVersion = packageJson.version;

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
    __UPDATER_ENABLED__: JSON.stringify(false),
    __SENTRY_DSN__: JSON.stringify(""),
    __SENTRY_RELEASE__: JSON.stringify(`capyinn@${appVersion}`),
    __SENTRY_ENVIRONMENT__: JSON.stringify("development"),
  },
  plugins: [react()],
});
```

- [ ] **Step 3: Freeze the current update contract instead of letting tests and UI drift apart**

Keep the shell badge contract uppercase and the settings-section CTA contract title case. Make the hook and settings UI expose stable states for:

- update available → `Update`
- update downloaded → `Restart to update`
- no update available → `You are on the latest version.`

Use the existing tests as the red suite and make the minimal code change needed.

```ts
// mhm/src/pages/settings/SoftwareUpdateSection.tsx
{update.phase === "available" && (
  <Button onClick={() => void update.downloadUpdate()}>Update</Button>
)}

{update.phase === "downloaded" && (
  <Button onClick={update.openRestartPrompt}>Restart to update</Button>
)}

{!updatesUnavailable && !update.availableVersion && update.phase === "idle" && (
  <p className="text-xs text-emerald-600">You are on the latest version.</p>
)}
```

```ts
// mhm/src/hooks/useAppUpdateController.ts
if (!update) {
  setPendingUpdate(null);
  setAvailableVersion(null);
  setPhase("idle");
  setErrorMessage(null);
  return;
}
```

- [ ] **Step 4: Re-run the targeted tests until they pass cleanly**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- src/App.updateFlow.test.tsx tests/e2e/08-settings.test.tsx
```

Expected: PASS for the update-flow and settings tests, with no new warnings introduced.

- [ ] **Step 5: Commit the baseline green-up**

```bash
git add mhm/vitest.config.ts mhm/src/App.updateFlow.test.tsx mhm/tests/e2e/08-settings.test.tsx mhm/src/pages/settings/SoftwareUpdateSection.tsx mhm/src/hooks/useAppUpdateController.ts
git commit -m "test: green update verification baseline"
```

---

### Task 2: Add The Runtime Contract And Deterministic Runtime Seams

**Files:**
- Create: `mhm/src-tauri/src/runtime_config.rs`
- Modify: `mhm/src-tauri/src/app_identity.rs`
- Modify: `mhm/src-tauri/src/lib.rs`
- Modify: `mhm/src-tauri/src/backup.rs`
- Test: `mhm/src-tauri/src/runtime_config.rs`
- Test: `mhm/src-tauri/src/app_identity.rs`

- [ ] **Step 1: Write the failing runtime-config tests first**

```rust
// mhm/src-tauri/src/runtime_config.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn runtime_root_override_reads_from_env() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("CAPYINN_RUNTIME_ROOT", "/tmp/capyinn-test-suite");
        assert_eq!(
            runtime_root_override().as_deref(),
            Some(std::path::Path::new("/tmp/capyinn-test-suite"))
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");
    }

    #[test]
    fn truthy_flags_enable_subsystem_disables() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var("CAPYINN_DISABLE_WATCHER", "true");
        std::env::set_var("CAPYINN_DISABLE_GATEWAY", "1");
        assert!(env_flag("CAPYINN_DISABLE_WATCHER"));
        assert!(env_flag("CAPYINN_DISABLE_GATEWAY"));
        std::env::remove_var("CAPYINN_DISABLE_WATCHER");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");
    }
}
```

- [ ] **Step 2: Run the targeted Rust tests to verify they fail**

Run:

```bash
cargo test --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml runtime_config::tests:: -- --nocapture
```

Expected: FAIL because `runtime_config.rs` does not exist yet.

- [ ] **Step 3: Implement the runtime contract and route app identity through it**

```rust
// mhm/src-tauri/src/runtime_config.rs
use chrono::{DateTime, FixedOffset};
use std::path::PathBuf;

pub fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

pub fn runtime_root_override() -> Option<PathBuf> {
    std::env::var_os("CAPYINN_RUNTIME_ROOT").map(PathBuf::from)
}

pub fn test_now() -> Option<DateTime<FixedOffset>> {
    std::env::var("CAPYINN_TEST_NOW")
        .ok()
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
}
```

```rust
// mhm/src-tauri/src/app_identity.rs
pub fn runtime_root_opt() -> Option<PathBuf> {
    crate::runtime_config::runtime_root_override().or_else(|| {
        dirs::home_dir().map(|home| home.join(APP_RUNTIME_DIR))
    })
}
```

```rust
// mhm/src-tauri/src/lib.rs
mod runtime_config;

let gateway_runtime = if runtime_config::env_flag("CAPYINN_DISABLE_GATEWAY") {
    None
} else {
    rt.block_on(async {
        match gateway::start_gateway(gateway_pool, gateway_handle).await {
            Ok(gateway) => Some(gateway),
            Err(error) => {
                error!("Failed to start MCP Gateway: {}", error);
                None
            }
        }
    })
};

if !runtime_config::env_flag("CAPYINN_DISABLE_WATCHER") {
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        if let Err(e) = watcher::start_watcher(handle) {
            error!("Failed to start file watcher: {}", e);
        }
    });
}
```

```rust
// mhm/src-tauri/src/backup.rs
pub fn backup_timestamp_now() -> chrono::NaiveDateTime {
    crate::runtime_config::test_now()
        .map(|value| value.naive_local())
        .unwrap_or_else(|| chrono::Utc::now().naive_utc())
}
```

- [ ] **Step 4: Re-run the targeted Rust tests and app-identity tests**

Run:

```bash
cargo test --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml runtime_config::tests:: -- --nocapture
cargo test --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml app_identity::tests:: -- --nocapture
```

Expected: PASS for the new runtime-config tests and the existing app-identity tests.

- [ ] **Step 5: Commit the runtime seam foundation**

```bash
git add mhm/src-tauri/src/runtime_config.rs mhm/src-tauri/src/app_identity.rs mhm/src-tauri/src/lib.rs mhm/src-tauri/src/backup.rs
git commit -m "feat: add deterministic verification runtime contract"
```

---

### Task 3: Add Verification Entry Points And Shared Artifact Capture

**Files:**
- Modify: `mhm/package.json`
- Create: `mhm/scripts/verify/shared.mjs`
- Create: `mhm/scripts/verify/quick.mjs`
- Create: `mhm/scripts/verify/full.mjs`
- Create: `mhm/scripts/verify/repeat.mjs`
- Create: `mhm/scripts/verify/native-smoke.mjs`
- Test: `mhm/tests/scripts/generate-latest-json.test.ts`

- [ ] **Step 1: Create the shared verification runner utilities first**

```js
// mhm/scripts/verify/shared.mjs
import { mkdir, rm, cp } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import os from "node:os";

export const runtimeRoot = path.join(os.homedir(), "CapyInn-TestSuite");
export const artifactsRoot = path.join(runtimeRoot, "artifacts");

export async function resetRuntimeRoot() {
  await rm(runtimeRoot, { recursive: true, force: true });
  await mkdir(runtimeRoot, { recursive: true });
  await mkdir(artifactsRoot, { recursive: true });
}

export function verificationEnv(extra = {}) {
  return {
    ...process.env,
    CAPYINN_RUNTIME_ROOT: runtimeRoot,
    CAPYINN_DISABLE_GATEWAY: "true",
    CAPYINN_DISABLE_WATCHER: "true",
    ...extra,
  };
}

export async function run(label, command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      stdio: "inherit",
      env: verificationEnv(options.env),
      cwd: options.cwd,
    });
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${label} exited ${code}`))));
  });
}
```

- [ ] **Step 2: Wire package entry points to dedicated scripts**

```json
// mhm/package.json
{
  "scripts": {
    "verify:quick": "node ./scripts/verify/quick.mjs",
    "verify:full": "node ./scripts/verify/full.mjs",
    "verify:repeat": "node ./scripts/verify/repeat.mjs"
  }
}
```

- [ ] **Step 3: Implement the entry points with the Wave 1 order**

```js
// mhm/scripts/verify/quick.mjs
import { run } from "./shared.mjs";

await run("frontend-update-baseline", "npm", ["test", "--", "src/App.updateFlow.test.tsx", "tests/e2e/08-settings.test.tsx"], { cwd: process.cwd() });
await run("setup-tests", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "services::setup::tests::"], { cwd: process.cwd() });
```

```js
// mhm/scripts/verify/full.mjs
import { resetRuntimeRoot, run } from "./shared.mjs";

await resetRuntimeRoot();
await run("quick", "npm", ["run", "verify:quick"], { cwd: process.cwd() });
await run(
  "wave1-booking-scenarios",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "services::booking::tests::"],
  { cwd: process.cwd() },
);
await run(
  "wave1-backup-tests",
  "cargo",
  ["test", "--manifest-path", "src-tauri/Cargo.toml", "backup::tests::"],
  { cwd: process.cwd() },
);
await run("native-smoke", "node", ["./scripts/verify/native-smoke.mjs"], { cwd: process.cwd() });
```

```js
// mhm/scripts/verify/repeat.mjs
import { run } from "./shared.mjs";

const iterations = Number(process.argv[2] ?? "3");
for (let index = 0; index < iterations; index += 1) {
  console.log(`verification iteration ${index + 1}/${iterations}`);
  await run("full", "npm", ["run", "verify:full"], { cwd: process.cwd() });
}
```

- [ ] **Step 4: Verify the new commands boot and fail in the right place before full implementation lands**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm run verify:quick
```

Expected: either PASS on the already-implemented subset or FAIL in the next missing planned layer, not because the scripts themselves are malformed.

- [ ] **Step 5: Commit the orchestration layer**

```bash
git add mhm/package.json mhm/scripts/verify/shared.mjs mhm/scripts/verify/quick.mjs mhm/scripts/verify/full.mjs mhm/scripts/verify/repeat.mjs mhm/scripts/verify/native-smoke.mjs
git commit -m "feat: add verification suite entry points"
```

---

### Task 4: Add Wave 1 Rust Business Scenarios In Existing Test Modules

**Files:**
- Modify: `mhm/src-tauri/src/services/setup/tests.rs`
- Modify: `mhm/src-tauri/src/services/booking/tests.rs`
- Modify: `mhm/src-tauri/src/backup.rs`

- [ ] **Step 1: Extend setup tests to cover onboarding → login handoff explicitly**

```rust
// mhm/src-tauri/src/services/setup/tests.rs
#[tokio::test]
async fn complete_setup_without_app_lock_creates_a_default_user_ready_for_login() {
    let pool = test_pool().await;

    let status = complete_setup(&pool, sample_onboarding_request(false))
        .await
        .expect("complete_setup should succeed");

    let default_user_id = crate::services::settings_store::get_setting(&pool, "default_user_id")
        .await
        .expect("default_user_id should load")
        .expect("default_user_id should exist");

    assert!(status.current_user.is_some());
    assert_eq!(status.current_user.unwrap().id, default_user_id);
}
```

- [ ] **Step 2: Add reservation → check-in and checkout settlement scenarios in booking tests**

```rust
// mhm/src-tauri/src/services/booking/tests.rs
#[tokio::test]
async fn reservation_to_checkin_marks_room_occupied_and_booking_active() {
    let pool = test_pool().await;
    seed_room(&pool, "R202").await.unwrap();
    seed_booked_reservation(&pool, "B202", "R202").await.unwrap();

    let result = stay_lifecycle::check_in(
        &pool,
        minimal_checkin_request("R202"),
        Some("user-1".to_string()),
    )
    .await
    .expect("check in should succeed");

    let room_status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = ?")
        .bind("R202")
        .fetch_one(&pool)
        .await
        .expect("room status should load");

    assert_eq!(room_status, "occupied");
    assert_eq!(result.status, "active");
}
```

```rust
#[tokio::test]
async fn checkout_preview_and_checkout_leave_a_balanced_booking_and_housekeeping_task() {
    let pool = test_pool().await;
    seed_room(&pool, "R420").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000.0).await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B420",
        "R420",
        "2026-04-20T08:00:00+07:00",
        "2026-04-22T12:00:00+07:00",
        2,
        1_000_000.0,
        Some(200_000.0),
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B420", 100_000.0, "2026-04-20T12:00:00+07:00")
        .await
        .unwrap();

    let preview = stay_lifecycle::preview_checkout_settlement(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B420".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
    )
    .await
    .expect("preview should succeed");

    assert!(preview.recommended_total >= 1_000_000.0);

    stay_lifecycle::check_out(
        &pool,
        CheckOutRequest {
            booking_id: "B420".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
            final_total: preview.recommended_total,
        },
    )
    .await
    .expect("checkout should succeed");

    let booking = sqlx::query("SELECT status, paid_amount FROM bookings WHERE id = ?")
        .bind("B420")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking.get::<String, _>("status"), "completed");
    assert!(booking.get::<f64, _>("paid_amount") >= preview.recommended_total);
}
```

- [ ] **Step 3: Add deterministic backup and night-audit assertions**

```rust
// mhm/src-tauri/src/backup.rs
#[test]
fn build_backup_filename_uses_frozen_time_when_present() {
    std::env::set_var("CAPYINN_TEST_NOW", "2026-04-21T09:15:00+07:00");
    let timestamp = backup_timestamp_now();
    let name = build_backup_filename(BackupReason::NightAudit, timestamp);
    std::env::remove_var("CAPYINN_TEST_NOW");

    assert_eq!(name, "capyinn_backup_night_audit_20260421_091500.db");
}
```

- [ ] **Step 4: Run the targeted Rust Wave 1 scenarios**

Run:

```bash
cargo test --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml services::setup::tests:: -- --nocapture
cargo test --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml services::booking::tests:: -- --nocapture
cargo test --manifest-path /Users/binhan/HotelManager/mhm/src-tauri/Cargo.toml backup::tests:: -- --nocapture
```

Expected: PASS for the new onboarding/login, reservation/check-in, checkout settlement, and deterministic backup assertions.

- [ ] **Step 5: Commit the Wave 1 Rust scenarios**

```bash
git add mhm/src-tauri/src/services/setup/tests.rs mhm/src-tauri/src/services/booking/tests.rs mhm/src-tauri/src/backup.rs
git commit -m "test: add wave 1 rust verification scenarios"
```

---

### Task 5: Prove A Deterministic Native Smoke Mechanism

**Files:**
- Modify: `mhm/src-tauri/src/lib.rs`
- Create: `mhm/scripts/verify/native-smoke.mjs`

- [ ] **Step 1: Add a file-based readiness probe instead of scraping arbitrary logs**

```rust
// mhm/src-tauri/src/lib.rs
fn write_smoke_ready_file() -> Result<(), String> {
    if let Some(path) = std::env::var_os("CAPYINN_SMOKE_READY_FILE") {
        let payload = serde_json::json!({
            "status": "ready",
            "runtime_root": crate::app_identity::runtime_root(),
        });
        std::fs::write(std::path::PathBuf::from(path), payload.to_string())
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}
```

Call `write_smoke_ready_file()` at the end of successful `setup()` after DB init and optional subsystem startup.

- [ ] **Step 2: Implement the native smoke script around the readiness file**

```js
// mhm/scripts/verify/native-smoke.mjs
import { access, readFile, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import { artifactsRoot, runtimeRoot, verificationEnv } from "./shared.mjs";

const readyFile = path.join(artifactsRoot, "smoke-ready.json");
await rm(readyFile, { force: true });

const child = spawn("npm", ["run", "tauri", "dev", "--", "--no-watch"], {
  cwd: process.cwd(),
  env: verificationEnv({
    CAPYINN_SMOKE_READY_FILE: readyFile,
  }),
  stdio: "inherit",
});

for (let index = 0; index < 60; index += 1) {
  try {
    await access(readyFile);
    const payload = JSON.parse(await readFile(readyFile, "utf8"));
    if (payload.status === "ready") {
      child.kill("SIGTERM");
      process.exit(0);
    }
  } catch {
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}

child.kill("SIGTERM");
throw new Error(`native smoke never became ready under ${runtimeRoot}`);
```

- [ ] **Step 3: Run the smoke script locally and verify it fails only on true startup problems**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
node ./scripts/verify/native-smoke.mjs
```

Expected: PASS with a created readiness file and clean process shutdown after boot.

- [ ] **Step 4: Fold native smoke into `verify:full` and artifact capture**

```js
// mhm/scripts/verify/full.mjs
await run("native-smoke", "node", ["./scripts/verify/native-smoke.mjs"], { cwd: process.cwd() });
```

Ensure failures keep:

- the readiness file
- the relevant `~/CapyInn-TestSuite` subtree
- diagnostics files, if any

- [ ] **Step 5: Commit the smoke mechanism**

```bash
git add mhm/src-tauri/src/lib.rs mhm/scripts/verify/native-smoke.mjs mhm/scripts/verify/full.mjs
git commit -m "feat: add deterministic native smoke probe"
```

---

### Task 6: Expand The Existing GitHub Actions CI With Wave 1 Verification

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add a dedicated Wave 1 verification job without removing the current build-test job**

```yaml
# .github/workflows/ci.yml
  verify-wave1:
    runs-on: macos-latest
    needs: build-test
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
          cache-dependency-path: mhm/package-lock.json

      - name: Setup Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Install frontend dependencies
        working-directory: mhm
        run: npm ci

      - name: Resolve verification artifact root
        run: echo "CAPYINN_ARTIFACT_ROOT=$HOME/CapyInn-TestSuite" >> "$GITHUB_ENV"

      - name: Run Wave 1 verification
        working-directory: mhm
        run: npm run verify:full

      - name: Upload verification artifacts on failure
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: verify-wave1-artifacts
          path: ${{ env.CAPYINN_ARTIFACT_ROOT }}
```

- [ ] **Step 2: Verify the workflow syntax locally before pushing**

Run:

```bash
rg -n "verify-wave1|upload-artifact|CAPYINN_ARTIFACT_ROOT" /Users/binhan/HotelManager/.github/workflows/ci.yml
```

Expected: the new job appears once, depends on `build-test`, and uploads `~/CapyInn-TestSuite` on failure.

- [ ] **Step 3: Re-run the full local suite once more before relying on CI**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm run verify:repeat -- 3
```

Expected: PASS three clean isolated runs before the CI job becomes the branch-protection candidate.

- [ ] **Step 4: Commit the CI expansion**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add wave 1 verification job"
```

---

## Self-Review

- Spec coverage:
  - baseline grounding and config drift: Task 1
  - runtime contract and subsystem gating: Task 2
  - verification commands and artifact capture: Task 3
  - Wave 1 Rust scenarios: Task 4
  - native smoke mechanism: Task 5
  - GitHub-hosted CI expansion: Task 6
- Placeholder scan:
  - no `TBD`, `TODO`, or “implement later” markers remain
  - each task contains explicit files, commands, and code snippets
- Type consistency:
  - env names match the updated spec contract
  - script names match `package.json` entries
  - CI job name `verify-wave1` matches the intended branch-protection target

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-21-verification-suite-wave1.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
