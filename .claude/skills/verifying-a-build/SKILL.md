---
name: verifying-a-build
description: Use when installing, relaunching, or checking a CapyInn build in the GUI — which of the two builds is on screen, and whether the new binary actually took.
---

# Verifying a CapyInn build

Two failure modes have each cost a full QA cycle. Both look like a broken feature and
are actually a stale process. Rule out both before you believe anything you see on
screen.

## 1. The reinstall that did nothing

The bundle is `CapyInn.app`; the executable inside is `Contents/MacOS/capyinn` —
**lowercase**.

`pkill -x CapyInn` matches nothing and exits `0`. The old process keeps running with
the old binary held open by inode, so copying a new `.app` into `/Applications`
appears to have no effect: `open -a` just reactivates the existing window, migrations
never run, and the schema version stays put.

```bash
pkill -x capyinn
ps aux | grep "[c]apyinn"     # expect no output before continuing
open -a CapyInn
```

Then confirm the new binary is the one running, from the database rather than the window:

```sql
SELECT version FROM schema_version;
```

On 2026-07-28 the DB kept reporting schema 21 after installing a build containing
migrations v22 and v23. It read as a broken migration. The app had simply never
restarted.

## 2. Two builds, one bundle id

Both of these run as `io.capyinn.app`:

- installed: `/Applications/CapyInn.app`
- dev: the debug binary started by `npm run tauri dev`

Screenshots and window titles cannot tell them apart, and launching "CapyInn" by name
brings up the **installed** one. A single stray click can front the stale production
window, after which every later screenshot silently shows old code.

```bash
ps aux | grep -E "debug/capyinn|CapyInn.app"
```

If both are up, **ask before quitting the installed one** — the owner may be running a
real hotel in it right now.

A decisive check for which build is on screen: change a visible string in the source
and watch the window. Vite HMR reaches the dev build only.

Merging to `main` does not update the installed app. That needs `npm run tauri build`
and a reinstall.

## 3. Before believing a green result

A passing test can also mean the edit never landed. When mutation-testing, assert that
the old string was actually found before you trust the outcome — `perl -pi` reports
nothing when a pattern matches nothing, and has silently written NUL bytes into source
here before.

## Releases

For release preflight, `docs/release-checklist.md` is the canonical list — version
sanity, baseline validation, `npm run verify:full`, core-PMS profile, and signing.
Follow it rather than reconstructing the steps.
