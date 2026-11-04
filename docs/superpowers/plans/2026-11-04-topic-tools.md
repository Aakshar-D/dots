# Topic tools — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename, archive (by hand and after N idle days), move a message to another topic and merge two topics of a dot.

**Architecture:** An optional `topics` row per root run holds title, archive state and a restart mark. Moves and merges rewrite `root_run_id`/`parent_run_id` in one transaction and mark the changed topics to restart their session; the runner starts such a topic's next run on a fresh session with a recap. The scheduler's pass sweeps idle topics hourly.

**Tech Stack:** Rust (tokio, sqlx/SQLite), Tauri 2, React + TypeScript (zod, TanStack Query, vitest).

**Spec:** `docs/superpowers/specs/2026-11-04-topic-tools-design.md`

## Global Constraints

- Topic identity stays the root run id. No backfill; no row = active, untitled.
- Every refusal of spec §3.6 is checked inside the writing transaction.
- Restart: never on a `resume` run. Marks only set by move/merge, taken by the runner.
- Migration `0035_topics.sql`. Commits carry no AI trailer.

### Task 1: Storage

**Files:** create `migrations/0035_topics.sql`, `src/store/topics.rs`; modify `src/model.rs`, `src/store/mod.rs`. Test: `tests/store_topics_test.rs`.

- [ ] Failing tests: rename/archive rows; move to a topic and to a new topic (roots, parent, restart marks, proposals); merge; refusals (root, active run, other dot, same topic, workflow step, app topic); sweep exclusions.
- [ ] Implement; run `cargo test -p dots-core --test store_topics_test`; commit `feat(core): topics keep a title, an archive mark, and can be moved and merged`.

### Task 2: Restart a moved topic's session

**Files:** `src/runner.rs`, `src/prompt.rs`. Test: `tests/runtime_topics_test.rs`.

- [ ] Failing test: after a move, the next chat in the target has no `--resume` and its prompt has `## This conversation so far`; the run after it resumes again.
- [ ] Implement; commit `feat(core): a topic changed by a move starts its next run afresh with what it said so far`.

### Task 3: Runtime and sweep

**Files:** `src/runtime.rs`, `src/events.rs`, `src/settings.rs`, `src/scheduler.rs`. Tests: `tests/runtime_topics_test.rs`, scheduler test.

- [ ] Failing tests: runtime methods announce `topics_changed`; chat unarchives; settings validate `auto_archive_days`; the scheduler's pass archives an idle topic.
- [ ] Implement; commit.

### Task 4: App commands

**Files:** `apps/dots-app/src/commands.rs`, `lib.rs`, contract fixture. Commit `feat(app): topic commands`.

### Task 5: UI

**Files:** `ui/src/api/types.ts`, `commands.ts`, `lib/home-topics.ts`, `lib/runtime-events.ts`, `screens/home/{use-conversation.ts,HistoryMenu.tsx,DotPane.tsx,TopicRuns.tsx,Timeline.tsx}`, new `screens/home/TopicTools.tsx`, `components/Transcript.tsx` (a per-message slot), Settings screen. Tests beside them.

- [ ] Failing tests, implement, commit per piece: topics with info; History's Archived; topic menu; Move to topic; setting.

### Task 6: Verify

`cargo test -p dots-core`, `cargo test -p dots-app`, clippy `--all-targets -D warnings` on both, vitest, `tsc`.
