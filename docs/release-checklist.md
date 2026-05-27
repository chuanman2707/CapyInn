# Release checklist

Use this checklist before pushing a `vX.Y.Z` release tag. The tag release workflow still runs its own release-build gate; this checklist is the human preflight that confirms the normal PMS and smoke gates are healthy before release publishing starts.

## 1. Version and source sanity

- `mhm/package.json`, `mhm/src-tauri/tauri.conf.json`, and `mhm/src-tauri/Cargo.toml` use the same version.
- The planned tag is `vX.Y.Z`, matching that shared version exactly.
- The release branch includes the intended changelog, docs, test, and release-signing updates when signing or release mechanics changed.
- No unrelated local changes are staged with the release prep.

## 2. Baseline validation

Run the baseline commands from `mhm/`:

```bash
npm test
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: every command passes without requiring Telegram, OpenAI, MCP, gateway, watcher, or agent configuration.

## 3. Smoke validation

Run the normal smoke gate from `mhm/`:

```bash
npm run verify:full
```

Expected: the suite passes and covers:

- reservation lifecycle smoke coverage
- stay lifecycle smoke coverage
- group booking lifecycle smoke coverage, including group check-in, partial checkout, and final checkout
- backup verification
- native Tauri startup smoke under the isolated `~/CapyInn-TestSuite` runtime root

## 4. Core PMS profile

Confirm the release is valid for the normal core PMS profile:

- rooms, reservations, stays, guests, housekeeping, billing, invoices, groups, night audit, settings, and auth remain available without experimental services
- normal app startup does not require external API keys
- disabled experimental runtime means no gateway, Telegram, OpenAI, MCP, watcher, or agent write configuration is required
- PMS state changes still enter through validated Tauri command boundaries and service/lifecycle modules

## 5. Release workflow readiness

Check repository Actions settings before pushing the tag:

- `TAURI_SIGNING_PRIVATE_KEY` is configured as an Actions secret
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is configured when the private key is encrypted
- `TAURI_UPDATER_PUBLIC_KEY` is configured as an Actions variable
- optional Windows signing secrets are either both present or both absent
- Actions workflow permissions allow the release job to write GitHub Releases

See `docs/release-signing.md` for signing, updater, asset, and generated manifest details.

## 6. Tag and publish

After the checklist passes:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

Expected: the release workflow verifies versions, builds platform artifacts, generates `latest.json`, and creates the GitHub Release only after required assets are present.
