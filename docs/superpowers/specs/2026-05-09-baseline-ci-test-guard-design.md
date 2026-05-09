# Baseline CI Test Guard Design

Issue: #134 ARCH-02 Batch 0: Establish baseline validation and baseline CI gates

Parent roadmap: #133 Core PMS Architecture Stabilization V2.1

Date: 2026-05-09
Status: Approved for implementation planning

## Purpose

Record the current validation truth before architecture cleanup begins, and confirm that CI blocks obvious broken cleanup changes without requiring experimental service configuration.

This slice is intentionally a baseline and guardrail pass. It should not refactor code, rename folders, fix unrelated test failures, or expand CI into slow or flaky gates.

## Scope

- Create a baseline report under `docs/cleanup/`.
- Run and record the current result of the baseline validation commands.
- Confirm `.github/workflows/ci.yml` runs, or clearly documents, the same frontend and Rust gates.
- Record any current failure as a known failing baseline with a short summary.
- Keep experimental services out of the CI requirement set.

## Non-Goals

- Do not fix unrelated failures unless the failure is tiny, isolated, and clearly part of making the baseline guard trustworthy.
- Do not refactor application code.
- Do not add slow, flaky, live-network, or secret-dependent gates.
- Do not require Telegram, OpenAI, MCP, gateway, or other experimental runtime configuration.
- Do not rename `mhm/` or move source files.

## Chosen Approach

Use a docs-first baseline plus CI parity check.

The main artifact will be a new report in `docs/cleanup/` named:

```text
docs/cleanup/2026-05-09-baseline-ci-test-guard.md
```

The report will capture the result of the required issue #134 commands plus one CI-parity command:

```bash
cd mhm && npm test
cd mhm && npm run build
cd mhm/src-tauri && cargo check
cd mhm/src-tauri && cargo test
cd mhm/src-tauri && cargo clippy --all-targets -- -D warnings
```

The four commands named in #134 are the minimum baseline. `cargo check` is included because the existing CI workflow and contributor docs already run it, so the baseline should match the real gate contributors see.

Rejected alternatives:

- Split the existing CI into separate frontend and Rust jobs. This could improve diagnosis, but it is more workflow churn than #134 needs.
- Add a script that generates or refreshes the baseline report. That is repeatable, but it adds maintenance surface for a baseline that should be updated deliberately, not regenerated casually.

## Baseline Report

The report will include:

- issue and roadmap references;
- command matrix with command, working directory, status, and concise evidence;
- known-failing section only if one or more commands fail;
- CI parity mapping from each baseline command to the corresponding `.github/workflows/ci.yml` step;
- explicit note that experimental services are not required;
- short guidance that future cleanup issues must not claim green verification for a command that is still documented as known failing.

If all commands pass, the known-failing section will say there are no known failing baseline commands as of the report date.

If a command fails, the report will record only the useful failure signal: failing suite or compiler phase, representative error line, and whether the failure appears pre-existing. Full terminal logs should not be pasted into the report.

## CI Guard

The existing CI workflow already has a `build-test` job that installs Node dependencies, sets up Rust with clippy, and runs frontend tests, frontend build, cargo check, cargo test, and clippy.

Implementation must leave CI unchanged if that remains true after inspection. A workflow edit is justified only if the current file is missing a required baseline command or makes experimental service configuration necessary.

The `verify-wave1` job is related but not the main #134 baseline gate. It can remain in place as an additional verification job.

## Validation Flow

Run the baseline commands locally and record the result:

```bash
cd mhm && npm test
cd mhm && npm run build
cd mhm/src-tauri && cargo check
cd mhm/src-tauri && cargo test
cd mhm/src-tauri && cargo clippy --all-targets -- -D warnings
```

Then verify documentation and CI references:

```bash
rg -n "npm test|npm run build|cargo check|cargo test|clippy" .github/workflows docs/cleanup CONTRIBUTING.md README.md
```

Before committing implementation changes, run GitNexus change detection:

```text
gitnexus_detect_changes(scope: "all", repo: "HotelManager")
```

## Error Handling Policy

Failure discovery should be handled conservatively:

- If a baseline command passes, record pass and concise evidence.
- If a baseline command fails for an unrelated reason, record it as known failing and do not fix it in #134.
- If the failure is a tiny CI/doc mismatch directly blocking #134, make the narrowest correction.
- If the failure implies a larger product or architecture problem, stop and split a follow-up issue.

Warnings and noisy logs are not failures unless they make the command exit non-zero or materially reduce cleanup confidence.

## Risks

The implementation risk is low because the expected change is documentation-only. The main risks are:

- accidentally turning the baseline issue into unrelated test repair;
- omitting `cargo check` from the report even though CI runs it;
- making CI depend on experimental services;
- over-recording logs instead of writing a useful baseline summary.

Keep the implementation scoped to the cleanup baseline report and a narrow CI/doc edit only if inspection proves one is needed.
