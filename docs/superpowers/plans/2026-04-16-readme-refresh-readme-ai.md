# CapyInn README Refresh With readme-ai Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refresh the public-facing Vietnamese `README.md` for CapyInn before open-source launch by using `readme-ai` as a draft generator, then manually polishing the final README so it reads like a deliberate project document rather than a raw AI template.

**Architecture:** Keep the generator isolated from the main repository. Clone and prepare `readme-ai` in `/Users/binhan/readme-ai`, run it from a Python 3.11 virtualenv because the current upstream source does not actually work on Python 3.9, generate one temporary draft from the local CapyInn repository into `/tmp`, compare the generated structure against the current README, then rewrite only the tracked `README.md` inside the CapyInn repo. Keep `README.en.md` unchanged unless a clearly stale public link must be corrected.

**Tech Stack:** Git worktrees, Python 3.11, `venv`, `pip`, `readme-ai`, Markdown, `rg`, `git diff`

---

### Task 1: Prepare An Isolated README Refresh Workspace

**Files:**
- Verify only: `.worktrees/readme-refresh/`
- Verify only: `docs/superpowers/specs/2026-04-16-readme-refresh-readme-ai-design.md`
- Create: `docs/superpowers/plans/2026-04-16-readme-refresh-readme-ai.md`

- [ ] **Step 1: Confirm the worktree branch starts from the approved spec commit**

Run:

```bash
cd /Users/binhan/HotelManager/.worktrees/readme-refresh
git status --short --branch
git log --oneline -1
```

Expected: clean `codex/readme-refresh` branch rooted at the current `main` head that already contains the README refresh design spec.

- [ ] **Step 2: Verify the spec decisions one more time before generation**

Run:

```bash
cd /Users/binhan/HotelManager/.worktrees/readme-refresh
sed -n '1,220p' docs/superpowers/specs/2026-04-16-readme-refresh-readme-ai-design.md
```

Expected: spec still matches the approved direction: Vietnamese README first, `readme-ai` outside the repo, temporary draft first, manual polish before commit.

- [ ] **Step 3: Keep the plan file itself in the branch**

Expected: this plan file exists in `docs/superpowers/plans/2026-04-16-readme-refresh-readme-ai.md` before implementation continues.

### Task 2: Set Up `readme-ai` Outside The Repository

**Files:**
- Create outside repo: `/Users/binhan/readme-ai`
- Create outside repo: `/Users/binhan/readme-ai/.venv311`
- Verify only: `/Users/binhan/readme-ai`

- [ ] **Step 1: Clone `eli64s/readme-ai` into `/Users/binhan/readme-ai`**

Run:

```bash
git clone https://github.com/eli64s/readme-ai /Users/binhan/readme-ai
```

Expected: local generator checkout exists outside the CapyInn repo.

- [ ] **Step 2: Create a Python 3.11 virtualenv for the tool**

Run:

```bash
cd /Users/binhan/readme-ai
/opt/homebrew/bin/python3.11 -m venv .venv311
```

Expected: `.venv311` exists under `/Users/binhan/readme-ai`.

- [ ] **Step 3: Install `readme-ai` from source instead of using the broken requirements file**

Run:

```bash
cd /Users/binhan/readme-ai
.venv311/bin/pip install -e .
```

Expected: editable install succeeds without touching the CapyInn repo.

- [ ] **Step 4: Inspect the actual CLI help before generating**

Run:

```bash
cd /Users/binhan/readme-ai
.venv311/bin/readmeai --help
```

Expected: confirm the repository/path, output, header, navigation, badge, and offline options available in the installed version.

### Task 3: Generate A Draft README Into `/tmp`

**Files:**
- Create temporary output: `/tmp/capyinn-readme-generated.vi.md`
- Verify only: `/Users/binhan/HotelManager/.worktrees/readme-refresh/README.md`

- [ ] **Step 1: Generate one offline draft against the local CapyInn repository path**

Run:

```bash
cd /Users/binhan/readme-ai
.venv311/bin/readmeai \
  --api offline \
  --repository /Users/binhan/HotelManager/.worktrees/readme-refresh \
  --output /tmp/capyinn-readme-generated.vi.md \
  --header-style compact \
  --navigation-style accordion \
  --badge-style for-the-badge \
  --badge-color 0F766E \
  --system-message 'Write the README in Vietnamese for an offline-first mini hotel management desktop app called CapyInn. Keep the tone concrete, honest, and maintainable. Avoid generic AI marketing language.'
```

Expected: a generated Markdown draft appears at `/tmp/capyinn-readme-generated.vi.md`.

- [ ] **Step 2: Inspect the generated output quickly for obvious breakage**

Run:

```bash
sed -n '1,260p' /tmp/capyinn-readme-generated.vi.md
```

Expected: a coherent Markdown document with usable structure, even if the prose is still generic or English-leaning.

- [ ] **Step 3: Compare the generated draft against the current README**

Run:

```bash
diff -u /Users/binhan/HotelManager/.worktrees/readme-refresh/README.md /tmp/capyinn-readme-generated.vi.md | sed -n '1,260p'
```

Expected: enough structural contrast to identify which generated sections are worth borrowing and which existing sections should remain authoritative.

### Task 4: Manually Rewrite And Polish `README.md`

**Files:**
- Modify: `README.md`
- Verify only: `README.en.md`

- [ ] **Step 1: Preserve the strongest parts of the current README**

Preserve or tighten:

- the offline-first positioning
- current setup and test commands
- current architecture and project-structure truth
- honest limitations
- contributing/security/license links

- [ ] **Step 2: Pull only the useful scaffolding from the generated draft**

Use the generated draft for:

- stronger section ordering
- tighter feature grouping
- cleaner header/navigation structure
- clearer badge/header treatment if it actually reads well on GitHub

Do **not** copy generic claims or unverifiable marketing language.

- [ ] **Step 3: Rewrite the tracked README**

Update `README.md` so it:

- leads with CapyInn’s value proposition in Vietnamese
- uses `CapyInn` consistently
- fixes any stale repo tree names such as `Hotel-Manager/`
- keeps commands current and copy-pasteable
- adds or sharpens a short `Known limitations` section if needed
- stays readable on GitHub without becoming bloated

- [ ] **Step 4: Only touch `README.en.md` if a clearly stale public link must be corrected**

Expected: the English README remains unchanged unless there is a factual repo-path problem worth fixing in the same pass.

### Task 5: Verify The Final README And Commit Cleanly

**Files:**
- Verify: `README.md`
- Verify maybe: `README.en.md`

- [ ] **Step 1: Run README hygiene checks**

Run:

```bash
cd /Users/binhan/HotelManager/.worktrees/readme-refresh
rg -n "Hotel-Manager|CabyInn" README.md README.en.md CONTRIBUTING.md
rg -n "TODO|TBD|FIXME|lorem ipsum" README.md
```

Expected: no stale repo spelling in tracked public docs relevant to this pass, and no placeholder text in `README.md`.

- [ ] **Step 2: Re-read the final README top-to-bottom**

Run:

```bash
cd /Users/binhan/HotelManager/.worktrees/readme-refresh
sed -n '1,260p' README.md
```

Expected: the first screenful is clearer than before, setup/test instructions still look accurate, and the tone feels like CapyInn rather than a generic generator template.

- [ ] **Step 3: Review the exact diff**

Run:

```bash
cd /Users/binhan/HotelManager/.worktrees/readme-refresh
git diff -- README.md README.en.md docs/superpowers/plans/2026-04-16-readme-refresh-readme-ai.md
```

Expected: only the planned documentation files changed.

- [ ] **Step 4: Commit the README refresh branch**

Run:

```bash
cd /Users/binhan/HotelManager/.worktrees/readme-refresh
git add README.md README.en.md docs/superpowers/plans/2026-04-16-readme-refresh-readme-ai.md
git commit -m "docs: refresh Vietnamese README"
```

Expected: one clean documentation commit for the README refresh work.
