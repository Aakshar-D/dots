# Report a problem — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** From Settings, a person can preview, copy or save a plain-text problem report with no private data in it.

**Architecture:** A new core module `report` reads the log tail, scrubs it, counts a few facts from the store and renders the text. Two app commands: one builds the report, one saves given text through the native save dialog. A dialog in Settings shows the preview and the two actions.

**Tech Stack:** Rust (sqlx/SQLite, regex), Tauri 2 (`tauri-plugin-dialog`), React + TypeScript (zod, TanStack Query, vitest).

**Spec:** `docs/superpowers/specs/2026-11-03-report-problem-design.md`

## Global Constraints

- The report reads nothing but the store counts, distinct engine kinds, and the log files. No message, payload, instruction, memory, dot name, project path or vault value is read.
- 200 log lines at most, oldest first, tail across files newest first.
- Lines from MCP server stderr and the browser helper's stderr are dropped and counted.
- Scrub order: redactor, secret patterns → `[secret]`, emails → `[email]`, home folder → `~`.
- The saved or copied text is exactly the previewed text.
- No network. Commits carry no AI trailer.

## Review Focus

1. A key, bearer token, Authorization header or `password=` in a log line never reaches the report. Task 1.
2. MCP and browser helper stderr never reaches the report. Task 1.
3. The UI cannot make Rust write to a path of its choosing (the dialog is in Rust). Task 3.

---

### Task 1: Core: log tail, scrubbing, render

**Files:**
- Create: `crates/dots-core/src/report.rs` (register in `lib.rs`)
- Test: `crates/dots-core/tests/report_test.rs`

- [ ] **Step 1: Failing tests** for `recent_log_lines` (across files, short newest file, missing folder), `scrub_line` (every pattern of spec §3, redactor secret, home path), `is_foreign_output`, `LogTail::from_lines` (drops and counts), `render` (sections, not described, logs off, no log).
- [ ] **Step 2: Run** `cargo test -p dots-core --test report_test`: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** again: PASS.
- [ ] **Step 5: Commit** `feat(core): a problem report with scrubbed recent log lines`.

### Task 2: Core: counts and `Runtime::problem_report`

**Files:**
- Create: `crates/dots-core/src/store/report.rs` (register in `store/mod.rs`)
- Modify: `crates/dots-core/src/runtime.rs`
- Test: `crates/dots-core/tests/report_test.rs`

- [ ] **Step 1: Failing tests:** `report_counts` (distinct engines sorted, dots count, runs in window, failed in window, nothing before `since`).
- [ ] **Step 2: Run:** fail. **Step 3: Implement** `Store::report_counts` and `Runtime::problem_report`. **Step 4: Run:** pass.
- [ ] **Step 5: Commit** `feat(core): the problem report counts dots and recent runs`.

### Task 3: App commands

**Files:** `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs`, `ui/src/api/commands.ts`, `ui/src/api/__fixtures__/contract.json` (the two names, by hand).

- [ ] `problem_report(description, include_logs) -> String`; `save_problem_report(text) -> Option<String>` (Rust save dialog, default name `dots-report-YYYYMMDD-HHMM.txt`).
- [ ] Run `cargo test -p dots-app` and `vitest run src/api`: the commands test passes.
- [ ] **Commit** `feat(app): commands to build and save a problem report`.

### Task 4: UI

**Files:** Create `ui/src/components/ReportProblemDialog.tsx` + test; modify `ui/src/screens/SettingsScreen.tsx`.

- [ ] **Step 1: Failing tests:** preview shows the command's text; unchecking refetches with `includeLogs: false`; Copy writes the previewed text; Save calls `save_problem_report` with it and toasts the path; the Settings button opens the dialog.
- [ ] **Step 2–4:** run, implement, run.
- [ ] **Step 5: Commit** `feat(ui): report a problem from Settings`.

### Task 5: Verify

`cargo test -p dots-core`, `cargo test -p dots-app`, clippy `--all-targets` on both, `vitest run`, `tsc --noEmit`.
