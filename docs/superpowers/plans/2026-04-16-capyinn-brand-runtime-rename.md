# CapyInn Brand And Runtime Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the shipped app from `mhm` / `MHM` to `CapyInn`, including bundle metadata, runtime storage root, user-facing branding, and icon/logo sizing, while treating `CapyInn` as a fresh app with no `~/MHM` migration.

**Architecture:** Introduce explicit app-identity helpers for frontend and backend first, then wire metadata and runtime-path call sites to those helpers instead of scattered string literals. After the naming surface is stable, resize the master logo, regenerate Tauri icons, and verify that the app writes only into `~/CapyInn` and presents itself consistently as `CapyInn`.

**Tech Stack:** React 19, TypeScript, Vitest, Tauri 2, Rust, SQLite, Python 3 + Pillow, `sips`, `cargo check`, `cargo test`

---

### Task 1: Introduce Shared App Identity Constants

**Files:**
- Create: `mhm/src/lib/appIdentity.ts`
- Create: `mhm/src/lib/appIdentity.test.ts`
- Create: `mhm/src-tauri/src/app_identity.rs`
- Modify: `mhm/src-tauri/src/lib.rs`
- Test: `mhm/src/lib/appIdentity.test.ts`

- [ ] **Step 1: Write the failing frontend identity test**

```ts
// mhm/src/lib/appIdentity.test.ts
import { describe, expect, it } from "vitest";
import {
  APP_NAME,
  APP_LOGO_ALT,
  EXPORT_PREFIX,
  ONBOARDING_DRAFT_KEY,
} from "./appIdentity";

describe("appIdentity", () => {
  it("uses CapyInn branding constants", () => {
    expect(APP_NAME).toBe("CapyInn");
    expect(APP_LOGO_ALT).toBe("CapyInn logo");
    expect(EXPORT_PREFIX).toBe("CapyInn");
    expect(ONBOARDING_DRAFT_KEY).toBe("capyinn-onboarding-draft");
  });
});
```

- [ ] **Step 2: Run the frontend test to verify it fails**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- src/lib/appIdentity.test.ts
```

Expected: FAIL because `src/lib/appIdentity.ts` does not exist yet.

- [ ] **Step 3: Write the minimal shared identity modules**

```ts
// mhm/src/lib/appIdentity.ts
export const APP_NAME = "CapyInn";
export const APP_LOGO_ALT = "CapyInn logo";
export const EXPORT_PREFIX = "CapyInn";
export const ONBOARDING_DRAFT_KEY = "capyinn-onboarding-draft";
export const APP_RUNTIME_DIR = "CapyInn";
export const APP_DATABASE_FILENAME = "capyinn.db";
export const APP_BUNDLE_IDENTIFIER = "io.capyinn.app";
```

```rust
// mhm/src-tauri/src/app_identity.rs
use std::path::PathBuf;

pub const APP_NAME: &str = "CapyInn";
pub const APP_RUNTIME_DIR: &str = "CapyInn";
pub const APP_DATABASE_FILENAME: &str = "capyinn.db";
pub const APP_GATEWAY_LOCKFILE: &str = ".gateway-port";
pub const APP_BUNDLE_IDENTIFIER: &str = "io.capyinn.app";

pub fn runtime_root() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(APP_RUNTIME_DIR)
}

pub fn database_path() -> PathBuf {
    runtime_root().join(APP_DATABASE_FILENAME)
}

pub fn scans_dir() -> PathBuf {
    runtime_root().join("Scans")
}

pub fn models_dir() -> PathBuf {
    runtime_root().join("models")
}

pub fn exports_dir() -> PathBuf {
    runtime_root().join("exports")
}

pub fn gateway_lockfile() -> PathBuf {
    runtime_root().join(APP_GATEWAY_LOCKFILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_capyinn_runtime_names() {
        assert_eq!(APP_NAME, "CapyInn");
        assert_eq!(APP_RUNTIME_DIR, "CapyInn");
        assert_eq!(APP_DATABASE_FILENAME, "capyinn.db");
        assert_eq!(APP_BUNDLE_IDENTIFIER, "io.capyinn.app");
    }
}
```

```rust
// mhm/src-tauri/src/lib.rs
mod app_identity;
```

- [ ] **Step 4: Run the targeted tests to verify they pass**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- src/lib/appIdentity.test.ts
cd /Users/binhan/HotelManager
cargo test uses_capyinn_runtime_names --manifest-path mhm/src-tauri/Cargo.toml
```

Expected: PASS in both frontend and Rust targeted tests.

- [ ] **Step 5: Commit**

```bash
cd /Users/binhan/HotelManager
git add mhm/src/lib/appIdentity.ts mhm/src/lib/appIdentity.test.ts mhm/src-tauri/src/app_identity.rs mhm/src-tauri/src/lib.rs
git commit -m "refactor: add CapyInn app identity helpers"
```

### Task 2: Rename Tauri And Package Metadata

**Files:**
- Modify: `mhm/package.json`
- Modify: `mhm/src-tauri/tauri.conf.json`
- Modify: `mhm/src-tauri/Cargo.toml`
- Modify: `mhm/src-tauri/src/main.rs`
- Test: command assertions over `package.json`, `tauri.conf.json`, and `Cargo.toml`

- [ ] **Step 1: Write a failing metadata assertion command**

Run:

```bash
cd /Users/binhan/HotelManager
python3 - <<'PY'
import json, pathlib, tomllib

pkg = json.loads(pathlib.Path("mhm/package.json").read_text())
tauri = json.loads(pathlib.Path("mhm/src-tauri/tauri.conf.json").read_text())
cargo = tomllib.loads(pathlib.Path("mhm/src-tauri/Cargo.toml").read_text())

assert pkg["name"] == "capyinn"
assert tauri["productName"] == "CapyInn"
assert tauri["identifier"] == "io.capyinn.app"
assert tauri["app"]["windows"][0]["title"] == "CapyInn"
assert cargo["package"]["name"] == "capyinn"
assert cargo["package"]["default-run"] == "capyinn"
assert cargo["lib"]["name"] == "capyinn_lib"
PY
```

Expected: FAIL on the first old `mhm` value.

- [ ] **Step 2: Update package, Tauri, and Cargo metadata**

```json
// mhm/package.json
{
  "name": "capyinn",
  "private": true,
  "version": "0.1.0",
  "type": "module"
}
```

```json
// mhm/src-tauri/tauri.conf.json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "CapyInn",
  "version": "0.1.0",
  "identifier": "io.capyinn.app",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "title": "CapyInn",
        "width": 800,
        "height": 600
      }
    ],
    "security": {
      "csp": null
    }
  }
}
```

```toml
# mhm/src-tauri/Cargo.toml
[package]
name = "capyinn"
version = "0.1.0"
description = "CapyInn desktop app"
authors = ["CapyInn"]
edition = "2021"
default-run = "capyinn"

[lib]
name = "capyinn_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

```rust
// mhm/src-tauri/src/main.rs
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--mcp-stdio".to_string()) {
        capyinn_lib::run_proxy();
    } else {
        capyinn_lib::run()
    }
}
```

- [ ] **Step 3: Run metadata assertions and compile checks**

Run:

```bash
cd /Users/binhan/HotelManager
python3 - <<'PY'
import json, pathlib, tomllib

pkg = json.loads(pathlib.Path("mhm/package.json").read_text())
tauri = json.loads(pathlib.Path("mhm/src-tauri/tauri.conf.json").read_text())
cargo = tomllib.loads(pathlib.Path("mhm/src-tauri/Cargo.toml").read_text())

assert pkg["name"] == "capyinn"
assert tauri["productName"] == "CapyInn"
assert tauri["identifier"] == "io.capyinn.app"
assert tauri["app"]["windows"][0]["title"] == "CapyInn"
assert cargo["package"]["name"] == "capyinn"
assert cargo["package"]["default-run"] == "capyinn"
assert cargo["lib"]["name"] == "capyinn_lib"
PY
cargo check --manifest-path mhm/src-tauri/Cargo.toml
```

Expected: assertions PASS and `cargo check` succeeds.

- [ ] **Step 4: Commit**

```bash
cd /Users/binhan/HotelManager
git add mhm/package.json mhm/src-tauri/tauri.conf.json mhm/src-tauri/Cargo.toml mhm/src-tauri/src/main.rs
git commit -m "refactor: rename app metadata to CapyInn"
```

### Task 3: Move Runtime Paths And Filenames To `~/CapyInn`

**Files:**
- Modify: `mhm/src-tauri/src/db.rs`
- Modify: `mhm/src-tauri/src/gateway/mod.rs`
- Modify: `mhm/src-tauri/src/gateway/proxy.rs`
- Modify: `mhm/src-tauri/src/ocr.rs`
- Modify: `mhm/src-tauri/src/watcher.rs`
- Modify: `mhm/src-tauri/src/commands/audit.rs`
- Modify: `mhm/src-tauri/src/commands/room_management.rs`
- Modify: `mhm/src-tauri/src/lib.rs`
- Test: `mhm/src-tauri/src/app_identity.rs`

- [ ] **Step 1: Write a failing runtime-path regression test**

Add this test to `mhm/src-tauri/src/app_identity.rs`:

```rust
#[test]
fn runtime_paths_do_not_reference_mhm() {
    assert!(runtime_root().ends_with("CapyInn"));
    assert!(database_path().ends_with("CapyInn/capyinn.db"));
    assert!(scans_dir().ends_with("CapyInn/Scans"));
    assert!(models_dir().ends_with("CapyInn/models"));
    assert!(exports_dir().ends_with("CapyInn/exports"));
    assert!(gateway_lockfile().ends_with("CapyInn/.gateway-port"));
}
```

- [ ] **Step 2: Run the targeted Rust test to verify the old code still fails the path audit**

Run:

```bash
cd /Users/binhan/HotelManager
cargo test runtime_paths_do_not_reference_mhm --manifest-path mhm/src-tauri/Cargo.toml
rg -n 'join\\("MHM"\\)|mhm\\.db|mhm_backup_' mhm/src-tauri/src
```

Expected: Rust test may pass if helpers are already correct, but `rg` must still show old hardcoded `MHM`/`mhm.db` call sites before implementation.

- [ ] **Step 3: Replace hardcoded runtime roots with the shared helper**

```rust
// mhm/src-tauri/src/db.rs
let db_dir = crate::app_identity::runtime_root();
std::fs::create_dir_all(&db_dir).expect("Cannot create CapyInn directory");

let db_path = crate::app_identity::database_path();
let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
```

```rust
// mhm/src-tauri/src/gateway/mod.rs and proxy.rs
let lockfile = crate::app_identity::gateway_lockfile();
```

```rust
// mhm/src-tauri/src/ocr.rs / watcher.rs / room_management.rs / audit.rs / lib.rs
let scans_dir = crate::app_identity::scans_dir();
let models_dir = crate::app_identity::models_dir();
let exports_dir = crate::app_identity::exports_dir();
let backup_path = crate::app_identity::exports_dir().join(format!("capyinn_backup_{}.db", timestamp));
```

- [ ] **Step 4: Run runtime-path verification**

Run:

```bash
cd /Users/binhan/HotelManager
cargo test runtime_paths_do_not_reference_mhm --manifest-path mhm/src-tauri/Cargo.toml
rg -n 'join\\("MHM"\\)|mhm\\.db|mhm_backup_' mhm/src-tauri/src
```

Expected:
- Rust test PASS
- `rg` returns no matches

- [ ] **Step 5: Commit**

```bash
cd /Users/binhan/HotelManager
git add mhm/src-tauri/src/app_identity.rs mhm/src-tauri/src/db.rs mhm/src-tauri/src/gateway/mod.rs mhm/src-tauri/src/gateway/proxy.rs mhm/src-tauri/src/ocr.rs mhm/src-tauri/src/watcher.rs mhm/src-tauri/src/commands/audit.rs mhm/src-tauri/src/commands/room_management.rs mhm/src-tauri/src/lib.rs
git commit -m "refactor: move runtime storage to CapyInn"
```

### Task 4: Update Frontend Branding, Storage Keys, And Visible Fallback Strings

**Files:**
- Modify: `mhm/src/components/AppLogo.tsx`
- Modify: `mhm/src/App.tsx`
- Modify: `mhm/src/pages/LoginScreen.tsx`
- Modify: `mhm/src/pages/onboarding/useOnboardingDraft.ts`
- Modify: `mhm/src/pages/settings/HotelInfoSection.tsx`
- Modify: `mhm/src/pages/Statistics.tsx`
- Modify: `mhm/src/pages/settings/GatewaySection.tsx`
- Modify: `mhm/src/pages/settings/OcrConfigSection.tsx`
- Modify: `mhm/src-tauri/src/gateway/tools.rs`
- Modify: `mhm/src-tauri/src/gateway/proxy.rs`
- Modify: `mhm/src-tauri/src/commands/invoices.rs`
- Modify: `mhm/src-tauri/src/commands/groups.rs`
- Test: `mhm/tests/e2e/00-onboarding.test.tsx`

- [ ] **Step 1: Write the failing onboarding draft key assertion**

Add this assertion to `mhm/tests/e2e/00-onboarding.test.tsx` after the onboarding-with-PIN flow:

```ts
expect(localStorage.getItem("capyinn-onboarding-draft")).toBeNull();
expect(localStorage.getItem("mhm-onboarding-draft")).toBeNull();
```

Add this assertion to the unlocked onboarding flow after render settles:

```ts
expect(screen.getByTitle("Dashboard")).toBeInTheDocument();
```

Then add a new focused test:

```ts
it("stores onboarding draft under the CapyInn key", async () => {
  setMockResponse("get_bootstrap_status", () => ({
    setup_completed: false,
    app_lock_enabled: false,
    current_user: null,
  }));

  render(<App />);

  await userEvent.click(await screen.findByRole("button", { name: /bắt đầu thiết lập/i }));
  await userEvent.type(screen.getByLabelText(/tên khách sạn/i), "CapyInn");

  expect(localStorage.getItem("capyinn-onboarding-draft")).toContain("CapyInn");
  expect(localStorage.getItem("mhm-onboarding-draft")).toBeNull();
});
```

- [ ] **Step 2: Run the onboarding test to verify it fails**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- tests/e2e/00-onboarding.test.tsx
```

Expected: FAIL because the app still uses `mhm-onboarding-draft` and old branding strings.

- [ ] **Step 3: Apply the frontend and user-facing branding rename**

```ts
// mhm/src/components/AppLogo.tsx
import { APP_LOGO_ALT } from "@/lib/appIdentity";

export default function AppLogo({ className = "h-10 w-10" }: AppLogoProps) {
  return (
    <img
      src="/app-logo.png"
      alt={APP_LOGO_ALT}
      className={`${className} object-contain`}
    />
  );
}
```

```ts
// mhm/src/pages/onboarding/useOnboardingDraft.ts
import { ONBOARDING_DRAFT_KEY } from "@/lib/appIdentity";

const STORAGE_KEY = ONBOARDING_DRAFT_KEY;
```

```tsx
// mhm/src/App.tsx
import { APP_NAME } from "@/lib/appIdentity";

<AppLogo className={collapsed ? "h-10 w-10 shrink-0" : "h-14 w-14 shrink-0"} />
{PAGE_TITLES[activeTab] || APP_NAME}
```

```ts
// Replace user-facing fallback strings
// Examples:
// "MHM Hotel" -> "CapyInn"
// "Hotel Manager" -> "CapyInn"
// "MHM-BaoCao-..." -> "CapyInn-..."
// "~/MHM/Scans" -> "~/CapyInn/Scans"
// "hotel-manager" snippet/example -> "capyinn"
```

- [ ] **Step 4: Run frontend and grep verification**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- tests/e2e/00-onboarding.test.tsx
cd /Users/binhan/HotelManager
rg -n 'MHM Hotel|Hotel Manager|mhm-onboarding-draft|MHM-BaoCao|~/MHM/Scans|hotel-manager' mhm/src mhm/src-tauri --glob '!mhm/src-tauri/target/**'
```

Expected:
- onboarding tests PASS
- `rg` returns only intentionally preserved non-user-facing references, or ideally no matches

- [ ] **Step 5: Commit**

```bash
cd /Users/binhan/HotelManager
git add mhm/src/components/AppLogo.tsx mhm/src/App.tsx mhm/src/pages/LoginScreen.tsx mhm/src/pages/onboarding/useOnboardingDraft.ts mhm/src/pages/settings/HotelInfoSection.tsx mhm/src/pages/Statistics.tsx mhm/src/pages/settings/GatewaySection.tsx mhm/src/pages/settings/OcrConfigSection.tsx mhm/src-tauri/src/gateway/tools.rs mhm/src-tauri/src/gateway/proxy.rs mhm/src-tauri/src/commands/invoices.rs mhm/src-tauri/src/commands/groups.rs mhm/tests/e2e/00-onboarding.test.tsx
git commit -m "refactor: rename user-facing branding to CapyInn"
```

### Task 5: Resize The Master Logo And Regenerate Tauri Icons

**Files:**
- Modify: `mhm/public/app-logo.png`
- Modify: `mhm/src-tauri/icons/32x32.png`
- Modify: `mhm/src-tauri/icons/128x128.png`
- Modify: `mhm/src-tauri/icons/128x128@2x.png`
- Modify: `mhm/src-tauri/icons/icon.png`
- Modify: `mhm/src-tauri/icons/icon.icns`
- Modify: `mhm/src-tauri/icons/icon.ico`
- Modify: generated `mhm/src-tauri/icons/Square*.png` outputs if regenerated by Tauri
- Test: bbox/fill-ratio measurement command

- [ ] **Step 1: Write a failing fill-ratio check**

Run:

```bash
cd /Users/binhan/HotelManager
python3 - <<'PY'
from PIL import Image

im = Image.open("mhm/public/app-logo.png").convert("RGBA")
bbox = im.getchannel("A").getbbox()
bw = bbox[2] - bbox[0]
bh = bbox[3] - bbox[1]
fill = max(bw / im.size[0], bh / im.size[1])
assert fill >= 0.74, fill
assert fill <= 0.78, fill
PY
```

Expected: FAIL because the current artwork fill is around `0.60`.

- [ ] **Step 2: Resize the master logo to the new optical size**

Use this exact one-off Python command:

```bash
cd /Users/binhan/HotelManager
python3 - <<'PY'
from PIL import Image

path = "mhm/public/app-logo.png"
im = Image.open(path).convert("RGBA")
alpha = im.getchannel("A")
bbox = alpha.getbbox()
cropped = im.crop(bbox)

canvas = Image.new("RGBA", im.size, (0, 0, 0, 0))
target_fill = 0.76
target_size = int(im.size[0] * target_fill)
resized = cropped.resize((target_size, int(cropped.size[1] * target_size / cropped.size[0])), Image.LANCZOS)

x = (canvas.size[0] - resized.size[0]) // 2
y = (canvas.size[1] - resized.size[1]) // 2
canvas.alpha_composite(resized, (x, y))
canvas.save(path)
PY
```

- [ ] **Step 3: Regenerate the Tauri icon set from the resized source**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm exec tauri icon public/app-logo.png
```

Expected: regenerated files under `mhm/src-tauri/icons/`.

- [ ] **Step 4: Verify the fill ratio and icon outputs**

Run:

```bash
cd /Users/binhan/HotelManager
python3 - <<'PY'
from PIL import Image

for path in ["mhm/public/app-logo.png", "mhm/src-tauri/icons/icon.png"]:
    im = Image.open(path).convert("RGBA")
    bbox = im.getchannel("A").getbbox()
    bw = bbox[2] - bbox[0]
    bh = bbox[3] - bbox[1]
    fill = max(bw / im.size[0], bh / im.size[1])
    assert 0.74 <= fill <= 0.78, (path, fill)
    print(path, round(fill, 3))
PY
sips -g pixelWidth -g pixelHeight mhm/src-tauri/icons/32x32.png mhm/src-tauri/icons/128x128.png mhm/src-tauri/icons/icon.png
```

Expected:
- fill checks PASS
- generated icon files exist with expected dimensions

- [ ] **Step 5: Commit**

```bash
cd /Users/binhan/HotelManager
git add mhm/public/app-logo.png mhm/src-tauri/icons
git commit -m "design: resize CapyInn logo and regenerate app icons"
```

### Task 6: Run Full Rename Verification

**Files:**
- Modify: none
- Test: whole rename surface

- [ ] **Step 1: Run backend verification**

Run:

```bash
cd /Users/binhan/HotelManager
cargo check --manifest-path mhm/src-tauri/Cargo.toml
cargo test commands::onboarding::tests --manifest-path mhm/src-tauri/Cargo.toml
```

Expected: PASS.

- [ ] **Step 2: Run frontend verification**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm test -- tests/e2e/00-onboarding.test.tsx
```

Expected: PASS.

- [ ] **Step 3: Run rename audit**

Run:

```bash
cd /Users/binhan/HotelManager
rg -n 'com\\.binhan\\.mhm|productName\": \"mhm\"|title\": \"mhm\"|join\\("MHM"\\)|mhm\\.db|mhm-onboarding-draft|MHM Hotel|Hotel Manager|MHM-BaoCao' mhm --glob '!mhm/src-tauri/target/**' --glob '!mhm/src-tauri/icons/**'
```

Expected: no matches in live source files.

- [ ] **Step 4: Smoke-test the dev app**

Run:

```bash
cd /Users/binhan/HotelManager/mhm
npm run tauri dev
```

Manual checks:
- window title shows `CapyInn`
- sidebar logo is visibly larger in expanded and collapsed states
- new runtime root is `~/CapyInn`
- a new DB is created at `~/CapyInn/capyinn.db`
- no new file activity appears under `~/MHM`

- [ ] **Step 5: Commit**

```bash
cd /Users/binhan/HotelManager
git status --short
```

Expected: clean working tree after prior commits, or only intentional leftovers from unrelated in-flight work.
