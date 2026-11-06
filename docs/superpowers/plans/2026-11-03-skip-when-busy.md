# Skip a schedule when the dot is busy — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A repeating schedule's slot that comes due while its dot is running or waiting on an approval is skipped and recorded, instead of queuing another run.

**Architecture:** The busy check runs inside `Store::record_slot`'s transaction; a busy slot writes `last_slot_at`, inserts a `schedule_skips` row and returns `SlotStart::Busy`. The scheduler announces it with a `schedule_skipped` event. `ScheduleView` carries the last skip and a count; the UI shows skip lines in the conversation and a "Last skipped" line in the dot's header.

**Tech Stack:** Rust (tokio, sqlx/SQLite), Tauri 2, React + TypeScript (zod, TanStack Query, vitest).

**Spec:** `docs/superpowers/specs/2026-11-03-skip-when-busy-design.md`

## Global Constraints

- Busy = a run of the dot with status `running` or `awaiting_approval`, any trigger (chat included). `queued` only is not busy.
- Only repeating schedules skip. `Rule::Once` slots and retries (`Store::start_retry`) never skip.
- A busy skip does not change `fired_count`, does not drop a pending retry, and moves `last_slot_at` to the slot.
- Missed-slot rules (late / catch-up) are unchanged; busy is checked only for a slot that would otherwise fire.
- Keep the newest 50 skips per dot. Migration `0028_schedule_skips.sql` (schema verbatim from spec §4.2).
- Event `schedule_skipped { dot_id, skip }`, plus the existing `schedule_updated`. No notification, no Needs you.
- Conversation line: `Schedule skipped at <local time>: still working on the previous run` (date prefixed when not today).
- Header line: `Last skipped <local time> · <n> skipped since the last run` (drop `· <n> skipped since the last run` when n = 0).
- Ruling (plan): the spec's "schedule panel" is the dot's header in the Home pane (`DotHeader`), because the UI has no other schedule panel; the Dots column keeps its `next HH:MM` line unchanged.
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH. Commits carry no AI trailer.
- Tests must not depend on row order of rows created in the same millisecond, on a dispatcher not claiming a queued run (hold the queue or start no dispatcher), or on exact elapsed time.

## Review Focus

1. **Run awaiting approval for hours:** every hourly slot is skipped and recorded, and the 50-row cap holds; no run piles up. Test in Task 1 (cap) and Task 2 (awaiting approval).
2. **Dot becomes free between two slots:** the next slot fires normally and the "skipped since the last run" count resets to 0. Test in Task 3.
3. **A chat reply in progress at slot time:** skipped, and the chat run is untouched. Test in Task 2.
4. **`Count(3)` schedule with skips between:** still exactly 3 runs. Test in Task 2.
5. **Skips are not unread:** a skip line never raises the dot's unread count. Test in Task 5.

---

### Task 1: Storage: busy check, skips table, `record_slot`

**Files:**
- Create: `crates/dots-core/migrations/0028_schedule_skips.sql`
- Modify: `crates/dots-core/src/model.rs` (`SlotStart` :375, new `ScheduleSkip`)
- Modify: `crates/dots-core/src/store/schedules.rs` (`record_slot` :173)
- Create: `crates/dots-core/src/store/skips.rs` (register in `store/mod.rs`)
- Test: `crates/dots-core/tests/store_skips_test.rs` (new)

**Interfaces:**
- Produces:
  - `pub struct ScheduleSkip { pub id: String, pub dot_id: String, pub slot_at: String, pub reason: String, pub run_id: Option<String>, pub created_at: String }` (Serialize/Deserialize, Clone, PartialEq, Debug).
  - `SlotStart::Busy(ScheduleSkip)`.
  - `Store::dot_is_busy(&self, dot_id: &str) -> Result<bool>`.
  - `Store::record_slot(&self, dot_id: &str, at: DateTime<Utc>, fire: Option<&NewRun>, skip_when_busy: bool, now: DateTime<Utc>) -> Result<SlotStart>`: when `fire.is_some() && skip_when_busy` and a busy run exists (queried inside the transaction), writes `last_slot_at`, inserts the skip (reason `"busy"`, `run_id` = the busy run), trims to the newest 50 for the dot, leaves `fired_count` and the retry fields alone, returns `Busy`.
  - `Store::schedule_skips(&self, dot_id: &str, limit: u32) -> Result<Vec<ScheduleSkip>>`, newest first (`ORDER BY slot_at DESC, rowid DESC`).
  - `Store::skips_since(&self, dot_id: &str, after: Option<&str>) -> Result<u32>` (count with `slot_at > after`, all when `None`).

- [ ] **Step 1: Write the failing tests** in `store_skips_test.rs`:
  - `a_dot_is_busy_only_while_a_run_runs_or_waits`: false with no run; false with a `queued` run; true with `running`; true with `awaiting_approval`; false once `succeeded`; another dot's running run does not make this dot busy.
  - `a_busy_slot_is_recorded_not_run`: running run R; `record_slot(dot, t, Some(&new), true, now)` returns `Busy(skip)` with `skip.reason == "busy"`, `skip.run_id == Some(R)`, `skip.slot_at == t`; the state's `last_slot_at == t`, `fired_count` unchanged; no new run exists.
  - `a_free_or_once_slot_still_fires`: same busy dot with `skip_when_busy = false` returns `Enqueued`; a free dot with `true` returns `Enqueued`; a dot with only a queued run returns `Coalesced`.
  - `a_busy_skip_keeps_a_pending_retry`: state with a retry; busy skip; retry fields unchanged.
  - `only_the_newest_50_skips_are_kept`: 55 busy skips on successive slots; `schedule_skips(dot, 100).len() == 50`, the first is the newest slot.
  - `skips_go_with_their_dot`: delete the dot; `schedule_skips` is empty (no error).
  - `skips_since_counts_after_a_time`: 3 skips at t1<t2<t3; `skips_since(dot, Some(t1)) == 2`; `None` gives 3.

- [ ] **Step 2: Run** `cargo test -p dots-core --test store_skips_test`. Expected: compile errors.
- [ ] **Step 3: Implement** the migration (spec §4.2 verbatim), model types, store functions; update every caller of `record_slot` (the scheduler, and tests) to pass `skip_when_busy` (`false` keeps today's behaviour for now; Task 2 sets it).
- [ ] **Step 4: Run** `cargo test -p dots-core --test store_skips_test --test scheduler_test` and any store schedule test file. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): a slot due while its dot is busy is recorded as a skip`.

### Task 2: Scheduler skips busy repeating slots

**Files:**
- Modify: `crates/dots-core/src/scheduler.rs` (`handle` :295-360, the `start_retry` match ~:384)
- Modify: `crates/dots-core/src/events.rs`
- Test: `crates/dots-core/tests/scheduler_test.rs` (harness `h.dot`, `h.start`, `h.at`, `h.judge`, `h.exec`, `h.notices`, `h.row`)

**Interfaces:**
- Consumes: Task 1.
- Produces: `RuntimeEvent::ScheduleSkipped { dot_id: String, skip: ScheduleSkip }` (serde `schedule_skipped`).

`handle` passes `skip_when_busy = !matches!(schedule.rule, Rule::Once { .. })`. `SlotStart::Busy` is handled like `Skipped` for timing (`last = due`; nothing watched; no `fired`), logs at info, emits `ScheduleSkipped` then `ScheduleUpdated { dot_id }`. In the `start_retry` match, `Busy` cannot occur; group it with the `Skipped` arm.

- [ ] **Step 1: Write the failing tests**:
  - `a_repeating_slot_is_skipped_while_the_dot_runs`: hourly dot; put a run in `running` (`h.exec` UPDATE); at the slot: no new run, `ScheduleSkipped` emitted with `reason == "busy"`; end the run; at the next slot a run is enqueued.
  - `a_run_waiting_on_an_approval_makes_it_busy` (Review Focus 1): status `awaiting_approval`, slot skipped.
  - `a_chat_reply_in_progress_makes_it_busy` (Review Focus 3): a `Chat`-trigger run `running`; slot skipped; the chat run's status is unchanged.
  - `a_one_time_schedule_still_queues_while_busy`: `Once` slot while running gives an enqueued run.
  - `a_due_retry_still_starts_while_busy`: pending retry due while a run is running; the retry's run is enqueued.
  - `skips_do_not_use_up_a_run_limit` (Review Focus 4): `Count(3)`, alternate busy and free slots until the schedule ends; exactly 3 runs were created.
  - `a_missed_slot_is_not_recorded_as_busy`: a slot past the on-time window with `catch_up = false` while running: no skip row, handled as today.

- [ ] **Step 2: Run** `cargo test -p dots-core --test scheduler_test`. Expected: FAIL.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** `cargo test -p dots-core --test scheduler_test --test store_skips_test`. Expected: PASS (existing scheduler tests included).
- [ ] **Step 5: Commit** `feat(core): the scheduler skips a repeating slot while its dot is busy`.

### Task 3: Reading skips: runtime and `ScheduleView`

**Files:**
- Modify: `crates/dots-core/src/model.rs` (`ScheduleView` :454, `ScheduleView::new` :470)
- Modify: `crates/dots-core/src/runtime.rs` (`schedule_view` :607; new method)
- Test: `crates/dots-core/tests/runtime_skips_test.rs` (new; build a Runtime the way an existing `runtime_*_test.rs` does, and hold the queue before creating runs)

**Interfaces:**
- Consumes: Task 1 store functions.
- Produces: `ScheduleView.last_skipped_at: Option<String>` and `ScheduleView.skipped_since_run: u32`; `Runtime::schedule_skips(&self, dot_id: &str, limit: Option<u32>) -> Result<Vec<ScheduleSkip>>` (limit default and max 50; `NotFound` for an unknown dot).

`skipped_since_run` = `skips_since(dot, <queued_at of the dot's newest Schedule-trigger run>)`.

- [ ] **Step 1: Write the failing tests**:
  - `the_view_shows_the_last_skip_and_how_many_since_the_last_run` (Review Focus 2): scheduled run at t0; skips at t1, t2 → `last_skipped_at == Some(t2)`, `skipped_since_run == 2`; a new scheduled run at t3 → `skipped_since_run == 0`, `last_skipped_at` still `t2`.
  - `no_skips_gives_none_and_zero`.
  - `schedule_skips_are_newest_first_and_limited`: 5 skips, `limit Some(3)` returns the 3 newest; `Some(500)` returns at most 50; unknown dot is `NotFound`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test runtime_skips_test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the new test file and the test file that covers `schedule_view`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): the schedule view tells the last skip and the skips since the last run`.

### Task 4: App command and contract

**Files:**
- Modify: `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs` (`command_list!`), `apps/dots-app/src/contract.rs`, `apps/dots-app/src/events.rs` (tests only)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Consumes: Tasks 2–3.
- Produces: command `schedule_skips(dot_id: String, limit: Option<u32>) -> Res<Vec<ScheduleSkip>>`.

- [ ] **Step 1: Write the failing test** in `events.rs` tests: `a_skipped_slot_notifies_nobody`: `Notifier::decide(&RuntimeEvent::ScheduleSkipped{..})` is `None`.
- [ ] **Step 2: Run** `cargo test -p dots-app events`. Expected: FAIL (compile).
- [ ] **Step 3: Implement** the command and registration; contract samples for `ScheduleSkip`, `schedule_skipped`, and a `ScheduleView` with the two fields set.
- [ ] **Step 4: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): schedule_skips, and no notice for a skipped slot`.

### Task 5: UI

**Files:**
- Modify: `ui/src/api/types.ts` (`ScheduleView` :565, `RuntimeEvent`, new `ScheduleSkip`), `ui/src/api/commands.ts`, `ui/src/lib/query.ts` (key), `ui/src/lib/runtime-events.ts`
- Create: `ui/src/screens/home/SkipLine.tsx`
- Modify: `ui/src/screens/home/Timeline.tsx` (merge skips by time), `ui/src/screens/home/DotHeader.tsx`, `ui/src/lib/dot-status.ts` (a `skipLine(view)` helper)
- Test: `ui/src/screens/home/SkipLine.test.tsx` (new), `ui/src/lib/dot-status.test.ts`, the Timeline/Conversation test, the runtime-events test

**Interfaces:**
- Consumes: Task 4 contract and command.
- Produces: `commands.scheduleSkips(dotId: string, limit?: number)`; `keys.scheduleSkips(dotId)`; `skipLine(view: ScheduleView | null): string`.

- [ ] **Step 1: Write the failing tests**:
  - SkipLine: a skip today at 09:00 local renders `Schedule skipped at 09:00: still working on the previous run`; one from another day has the date before the time.
  - Timeline: skip lines appear among topics and check-ins in time order; the dot's unread count (overview fixture) is unchanged by skips (Review Focus 5 — skips are fetched separately, never as messages).
  - `skipLine`: `last_skipped_at` with `skipped_since_run: 3` gives `Last skipped 09:00 · 3 skipped since the last run`; with 0 gives `Last skipped 09:00`; null gives `""`.
  - DotHeader shows the `skipLine` text when non-empty.
  - `schedule_skipped` invalidates `keys.scheduleSkips(dot_id)`, `keys.overview` and `keys.dots`.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement** schemas, command, key, invalidation, SkipLine (muted, like a note line), the Timeline merge (query `scheduleSkips` for the shown dot), `skipLine` and the header line, using the Global Constraints copy and `localTime` from `dot-status.ts`.
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): skipped slots in the conversation and the dot's header`.

### Task 6: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps: `cargo fmt --all -- --check`; `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`; `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times` (twice); `cargo test -p dots-app --locked`; `node --test tools/release/release.test.mjs`; `cd ui; bun run typecheck; bun run test`. Expected: all pass, both core runs.
- [ ] **Step 2: Push** `feat/skip-when-busy` and open the PR against `master`.
