# Stuck-run watchdog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Warn when a run produces no output for its quiet limit, let the person stop it or keep waiting, and stop it automatically at twice the limit.

**Architecture:** A third timer in the runner's existing `select!` loop (`runner.rs` `execute_inner`) next to the time limit. The quiet mark lives on the `runs` row (`quiet_since`) so needs-you counts, the UI and the tray see it; a `run_quiet` runtime event announces changes. Keep waiting reaches the loop through a `Notify` held in the runner's `Active` registry next to the cancel token.

**Tech Stack:** Rust (tokio, sqlx/SQLite), Tauri 2, React + TypeScript (zod, TanStack Query, vitest).

**Spec:** `docs/superpowers/specs/2026-11-02-stuck-run-watchdog-design.md`

## Global Constraints

- Quiet limit bounds: `60..=86400` seconds; `Settings.quiet_limit_secs` default `600`; `DotSpec.quiet_limit_secs: Option<u64>`, `None` = app default, `Some(0)` = off.
- Auto-stop at exactly twice the effective quiet limit.
- Watchdog ending: status `failed`, error `stopped: no output for {N} min` where `N = 2 * limit_secs / 60` (integer).
- The time-limit ending stays `timeout after {N}s`; whichever timer fires first names the error.
- Notice: title `{dot} has gone quiet`, body `No output for {n} min. Open it to stop it or keep waiting.`, route `/chat/{dot_id}/{run_id}`.
- Card copy: `Quiet for {n} min`, `Stops by itself in {m} min`, buttons `Stop` and `Keep waiting`.
- Dot editor label `Quiet limit`, choices `App default (10 min)` (minutes from Settings), `Off`, minutes 1–1440; help: `If this dot shows no output for this long, Dots asks you; at twice this long it stops the run.`; hint when above the time limit: `Longer than the time limit; the time limit will end the run first.`
- Settings label `Quiet limit for every dot`, minutes 1–1440.
- Migration number 0026 (`0026_run_quiet.sql`).
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH (PowerShell: `$env:PATH = "C:\Program Files\LLVM\bin;$env:PATH"`).
- Commit messages: no AI co-author trailer.

## Review Focus

1. **Engine that ignores the watchdog's cancel** (a `Step::Hang` engine): the run must still end `failed` with the watchdog's message after the cancel grace, not lose its reason. Test in Task 3.
2. **Dot quiet limit edited while a run is quiet**: the card keeps the minutes the run started with (`run.quiet_limit_secs`), not the dot's new value. Test in Task 6.
3. **App restarted while a run is quiet**: `recover_interrupted` clears `quiet_since`, so the dot's needs-you drops back. Test in Task 1.
4. **A dot row from before 0026**: reads `quiet_limit_secs = None` and is saved back unchanged. Test in Task 1.
5. **Keep waiting on a run that just ended**: the UI shows the `Conflict` error on the card and the next refresh removes the card. Test in Task 6.

---

### Task 1: Schema, model and store

**Files:**
- Create: `crates/dots-core/migrations/0026_run_quiet.sql`
- Modify: `crates/dots-core/src/model.rs` (`DotSpec` :78-133, `Run` :211-246)
- Modify: `crates/dots-core/src/store/dots.rs` (`DOT_COLUMNS` :14, `dot_from_row` :20, `create_dot` :119, `update_dot` :207, `dot_counts` :332)
- Modify: `crates/dots-core/src/store/runs.rs` (`RUN_COLUMNS` :11, `run_from_row` :16, `finish_run` :573, `fail_if_running` :595, `cancel_if_inactive` :613, `recover_interrupted` :641)
- Modify: every `Run { .. }` literal: `crates/dots-core/tests/common/mod.rs` (~:107), `apps/dots-app/src/contract.rs` `fn run()` (:495), `apps/dots-app/src/events.rs` tests `fn run` (:371)
- Test: `crates/dots-core/tests/store_quiet_test.rs` (new)

**Interfaces:**
- Produces:
  - `DotSpec.quiet_limit_secs: Option<u64>` (`#[serde(default)]`, `None` in `DotSpec::new`)
  - `Run.quiet_since: Option<String>`, `Run.quiet_limit_secs: Option<u64>`, `Run.quiet_stopped: bool`
  - `Store::set_run_quiet_limit(&self, id: &str, secs: Option<u64>) -> Result<()>`
  - `Store::set_run_quiet(&self, id: &str, since: Option<&str>) -> Result<Run>`: sets/clears `quiet_since` only while `status = 'running'`; returns the row.
  - `Store::finish_run_with(&self, id: &str, status: RunStatus, summary: Option<&str>, error: Option<&str>, quiet_stopped: bool) -> Result<Run>`; `finish_run` delegates with `false`. Both clear `quiet_since`.
  - `fail_if_running`, `cancel_if_inactive`, `recover_interrupted` also set `quiet_since = NULL`.
  - `dot_counts` adds `(SELECT COUNT(*) FROM runs WHERE dot_id = ? AND status = 'running' AND quiet_since IS NOT NULL)` to `needs_you`.

- [ ] **Step 1: Write the failing tests** in `store_quiet_test.rs` (helpers from `tests/common`: `temp_store`, `spec`):
  - `a_dot_keeps_its_quiet_limit`: create a dot with `Some(900)`, read back `Some(900)`; update to `Some(0)`, read `Some(0)`; update to `None`, read `None`.
  - `a_dot_from_before_0026_reads_no_quiet_limit`: migrate to 0025 (pattern of `store_browser_test.rs:76`), insert a dot row, migrate fully, `get_dot(..).spec.quiet_limit_secs == None`; `update_dot` with the read spec keeps `None` (Review Focus 4).
  - `the_quiet_mark_is_set_cleared_and_counted`: running run; `set_run_quiet(id, Some(ts))` returns `quiet_since == Some(ts)`; `dot_counts(dot).0 == 1`; `set_run_quiet(id, None)` clears it; count back to 0.
  - `ending_a_run_clears_its_mark_and_records_the_watchdog`: mark set, `finish_run_with(id, Failed, None, Some("stopped: no output for 20 min"), true)` gives `quiet_since == None`, `quiet_stopped == true`; plain `finish_run` gives `quiet_stopped == false`.
  - `a_restart_clears_the_mark`: running run marked, `recover_interrupted()`, run is `failed`/`interrupted`, `quiet_since == None`, `dot_counts(dot).0 == 0` (Review Focus 3).
  - `the_quiet_limit_a_run_started_with_is_kept`: `set_run_quiet_limit(id, Some(600))` then `get_run` gives `Some(600)`.

- [ ] **Step 2: Run** `cargo test -p dots-core --test store_quiet_test`. Expected: compile errors (fields and functions missing).

- [ ] **Step 3: Write the migration**:

```sql
ALTER TABLE dots ADD COLUMN quiet_limit_secs INTEGER;
ALTER TABLE runs ADD COLUMN quiet_since TEXT;
ALTER TABLE runs ADD COLUMN quiet_limit_secs INTEGER;
ALTER TABLE runs ADD COLUMN quiet_stopped INTEGER NOT NULL DEFAULT 0;
```

- [ ] **Step 4: Implement** the model fields and store functions above, following the `browser` column pattern (commit 394575c). Add the new fields to every `Run` literal (`None`, `None`, `false`).

- [ ] **Step 5: Run** `cargo test -p dots-core --test store_quiet_test --test store_runs_test --test store_dots_test --test runtime_seen_test`. Expected: PASS.

- [ ] **Step 6: Commit** `feat(core): the quiet mark, its limit and the watchdog's ending on runs`.

### Task 2: Settings and the dot's validation

**Files:**
- Modify: `crates/dots-core/src/settings.rs` (struct :20, `Default` :52, `validate` :75)
- Modify: `crates/dots-core/src/model.rs` (`DotSpec::validate` :135; constants near `MAX_TIMEOUT_SECS` :75)
- Test: `crates/dots-core/tests/settings_test.rs`, and the test file holding the existing `timeout_secs must be at most` cases (find with `git grep "timeout_secs must be at most" crates/dots-core/tests`)

**Interfaces:**
- Produces: `pub const MIN_QUIET_LIMIT_SECS: u64 = 60;`, `pub const MAX_QUIET_LIMIT_SECS: u64 = 86_400;` in `model.rs`; `Settings.quiet_limit_secs: u64` default `600`; `pub fn effective_quiet_limit(spec: &DotSpec, settings: &Settings) -> Option<u64>` in `model.rs` (`None` when off).

- [ ] **Step 1: Write the failing tests**:
  - `an_empty_table_gives_the_defaults`: add `assert_eq!(s.quiet_limit_secs, 600)`.
  - `invalid_settings_are_rejected_and_not_stored`: add cases `quiet_limit_secs = 59` and `86401`, needle `"quiet_limit_secs must be between 60 and 86400"`.
  - DotSpec validation: `Some(59)` and `Some(86401)` rejected with the same needle; `Some(0)`, `None`, `Some(60)`, `Some(86400)` accepted.
  - `effective_quiet_limit`: `(None, 600) -> Some(600)`, `(Some(900), 600) -> Some(900)`, `(Some(0), 600) -> None`.

- [ ] **Step 2: Run** `cargo test -p dots-core --test settings_test` and the spec test file. Expected: FAIL.

- [ ] **Step 3: Implement** the field, default, both validations (message `quiet_limit_secs must be between 60 and 86400`; `Some(0)` allowed on the dot only) and `effective_quiet_limit`.

- [ ] **Step 4: Run** the same tests. Expected: PASS.

- [ ] **Step 5: Commit** `feat(core): a quiet limit for every dot and for each dot`.

### Task 3: The watchdog in the runner, and Keep waiting

**Files:**
- Modify: `crates/dots-core/src/events.rs` (new variant)
- Modify: `crates/dots-core/src/runner.rs` (`RunnerConfig`, `Active` :112, `dispatch_once` :608, `execute` :729, `execute_inner` :1092 loop :1342-1445)
- Modify: `crates/dots-core/src/runtime.rs` (next to `cancel_run` :1427; runner config construction)
- Test: `crates/dots-core/tests/runner_quiet_test.rs` (new; helpers copied from `runner_test.rs`: `runner_with`, `folder_dot`, `manual`)

**Interfaces:**
- Consumes: Task 1 store functions, Task 2 `effective_quiet_limit`.
- Produces:
  - `RuntimeEvent::RunQuiet { run_id: String, dot_id: String, quiet_since: Option<String>, quiet_limit_secs: Option<u64> }` (serde `run_quiet`).
  - `RunnerConfig.quiet_unit: Duration`, default `Duration::from_secs(1)`: the length of one limit-second. Tests set it to `Duration::from_millis(10)` so a 60 s limit takes 600 ms.
  - `Active.keep_waiting: Arc<tokio::sync::Notify>`.
  - `Runner::keep_waiting(&self, run_id: &str) -> Result<()>`: `Conflict` via `already(run)` if not active; notifies.
  - `Runtime::keep_waiting(&self, run_id: &str) -> Result<()>`: `get_run` (`NotFound`), `Conflict("run X is not quiet")` unless `status == Running && quiet_since.is_some()`, then `self.runner.keep_waiting`.

Loop rules (spec §4.2), the part the tests do not fully determine:

```
limit = effective_quiet_limit(..).map(|s| quiet_unit * s)   // set_run_quiet_limit(run.id, secs) at engine start
last_output = now; quiet_at: Option<Instant> = None; quiet_stopped = false
arm (if limit.is_some() && grace.is_none()): sleep_until(quiet_at.map_or(last_output + limit, |q| q + limit))
  if store.has_pending_for_run(run.id): clear mark if set; last_output = now; quiet_at = None
  else if quiet_at.is_none(): quiet_at = Some(now); set_run_quiet(id, Some(now_rfc3339)); emit RunQuiet + RunUpdated
  else: quiet_stopped = true; cancel.cancel(); grace = Some(now + cfg.cancel_grace)
on any engine event: last_output = now; if quiet_at.take().is_some(): set_run_quiet(id, None); emit
arm keep.notified(): same as an event
```

Ending: checked after `user_cancelled`. Only one of `timed_out` and `quiet_stopped` can be set, because each timer arm is disabled once `grace` is set. `quiet_stopped` gives `finish_run_with(.., Failed, .., Some(&format!("stopped: no output for {} min", 2 * secs / 60)), true)` and triggers `expire_pending_for_run` like `timed_out`.

- [ ] **Step 1: Write the failing tests** (dot `quiet_limit_secs = Some(60)`, `quiet_unit = 10ms`, `timeout_secs` large unless stated):
  - `a_quiet_run_is_marked_and_announced`: `ScriptedEngine` `[Emit(text), WaitForCancel]`; within 2 s the run has `quiet_since.is_some()`, `quiet_limit_secs == Some(60)`, and a `RunQuiet` with `quiet_since: Some(_)` was on the bus.
  - `output_clears_the_mark`: `[Sleep(800ms), Emit(text), WaitForCancel]`; mark appears, then `RunQuiet { quiet_since: None, .. }` and the row cleared.
  - `an_unanswered_quiet_run_is_stopped_at_twice_the_limit`: `[WaitForCancel]`; ends `failed`, error `"stopped: no output for 2 min"`, `quiet_stopped == true`.
  - `a_run_that_ignores_the_stop_still_ends_with_the_watchdogs_reason`: `[Hang]`; ends `failed` with `"stopped: no output for 2 min"` (Review Focus 1).
  - `keep_waiting_restarts_the_clock`: `[WaitForCancel]`; on the first mark call `runtime.keep_waiting` (or `runner.keep_waiting`); the mark clears; the run is still running 500 ms later; a second mark comes later.
  - `a_pending_approval_pauses_the_watchdog`: `[Ask{..}]` with `approval_wait_secs` long; after 1.5 s no mark and the run is running.
  - `the_watchdog_off_never_marks`: `Some(0)`, `[Sleep(2s), Emit(finished)]`; no `RunQuiet` seen; ends `succeeded`.
  - `the_time_limit_firing_first_keeps_its_reason`: `quiet_limit_secs = Some(86400)`, `timeout_secs = 1`, `[Hang]`; error `"timeout after 1s"`, `quiet_stopped == false`.
  - `keep_waiting_errors`: unknown id is `NotFound`; a succeeded run is `Conflict`; a running run that is not quiet is `Conflict`.

- [ ] **Step 2: Run** `cargo test -p dots-core --test runner_quiet_test`. Expected: compile errors, then FAIL.

- [ ] **Step 3: Implement** the event, config field, registry field, loop and ending; thread `Arc<Notify>` from `dispatch_once` through `execute` into `execute_inner` the way `by_user` travels.

- [ ] **Step 4: Run** `cargo test -p dots-core --test runner_quiet_test --test runner_test --test runner_limits_test --test runner_workflow_test`. Expected: PASS.

- [ ] **Step 5: Commit** `feat(core): the runner warns about a quiet run and stops it at twice the limit`.

### Task 4: No retry after a watchdog stop

**Files:**
- Modify: `crates/dots-core/src/scheduler.rs` (`failed` :609-643, before the retry branch at :621)
- Test: `crates/dots-core/tests/scheduler_test.rs`

**Interfaces:**
- Consumes: `Run.quiet_stopped` (Task 1).

- [ ] **Step 1: Write the failing test** `a_run_the_watchdog_stopped_needs_attention_at_once`, modelled on `a_failure_after_a_tool_call_needs_attention_until_a_scheduled_run_succeeds` (:770): start a scheduled run, end it with `h.exec("UPDATE runs SET status='failed', error='stopped: no output for 20 min', quiet_stopped=1, ended_at=? WHERE id=?", ..)` (no tool call, attempt 1), `h.judge(..)`: no retry pending, `attention_run_id == run.id`, one `ScheduleAttention` notice with error `Some("stopped: no output for 20 min")`; the next slot still fires.

- [ ] **Step 2: Run** `cargo test -p dots-core --test scheduler_test a_run_the_watchdog`. Expected: FAIL (a retry is scheduled).

- [ ] **Step 3: Implement**: in `failed`, treat `run.quiet_stopped` like `called` (no retry).

- [ ] **Step 4: Run** `cargo test -p dots-core --test scheduler_test`. Expected: PASS, including `a_failure_before_any_call_is_retried_up_to_the_sixth_attempt`.

- [ ] **Step 5: Commit** `feat(core): a scheduled run the watchdog stopped is not retried`.

### Task 5: App command, contract and notifications

**Files:**
- Modify: `apps/dots-app/src/commands.rs` (next to `cancel_run` :599)
- Modify: `apps/dots-app/src/lib.rs` (`command_list!` :35)
- Modify: `apps/dots-app/src/contract.rs` (sample `RuntimeEvent` list :1264-1336; `LIMITS` if the Rust constants are listed there)
- Modify: `apps/dots-app/src/events.rs` (`Notice` :30, `decide` :89, RunEnded rule :101-121, `notice_text` :196, `notice_route` :265, `notify` dot resolution :328)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Consumes: `Runtime::keep_waiting`, `RuntimeEvent::RunQuiet`, `Run.quiet_stopped`, `MIN/MAX_QUIET_LIMIT_SECS`.
- Produces: Tauri command `keep_waiting(run_id: String) -> Res<()>`; `Notice::RunQuiet { run_id: String, dot_id: String, minutes: u64 }`.

- [ ] **Step 1: Write the failing tests** in `events.rs` `mod tests`:
  - `a_quiet_run_notifies_once`: `RunQuiet { quiet_since: Some(_), quiet_limit_secs: Some(600) }` gives `Notice::RunQuiet { minutes: 10, .. }`; `quiet_since: None` gives nothing.
  - extend `notification_texts`: `("<dot> has gone quiet", "No output for 10 min. Open it to stop it or keep waiting.")`.
  - extend `a_notice_opens_the_conversation_it_is_about`: route `/chat/<dot>/<run>`.
  - `a_chat_run_the_watchdog_stopped_notifies_as_failed`: chat run `Running` then `Failed` with `quiet_stopped = true` gives `RunEnded`; with `false` stays quiet (keeps `chat_runs_and_runs_first_seen_finished_stay_quiet` green).
  - `a_failed_scheduled_run_is_quiet_and_its_attention_notifies` unchanged and passing with `quiet_stopped = true`.
  - extend `nothing_notifies_about_the_dot_being_viewed` with `RunQuiet`.

- [ ] **Step 2: Run** `cargo test -p dots-app events`. Expected: FAIL.

- [ ] **Step 3: Implement** the command, registration, notice, rule change (`run.trigger != TriggerKind::Chat || run.quiet_stopped`), text and route; add `RunQuiet` samples to the contract (one with a time, one cleared).

- [ ] **Step 4: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app`. Expected: PASS; `contract.json` changed.

- [ ] **Step 5: Commit** `feat(app): keep_waiting, and notices for a run gone quiet`.

### Task 6: UI

**Files:**
- Modify: `ui/src/api/types.ts` (`dotSpecShape` :105, `runShape` :169, `Settings` :512, `RuntimeEvent` :432, `LIMITS` :1453)
- Modify: `ui/src/api/commands.ts` (next to `cancelRun` :173)
- Modify: `ui/src/lib/runtime-events.ts` (`applyRuntimeEvent` :10)
- Create: `ui/src/screens/home/QuietCard.tsx`
- Modify: `ui/src/screens/home/TopicRuns.tsx` (:58-62), `ui/src/screens/home/RunCard.tsx` (:60-62), `ui/src/screens/RunScreen.tsx` (:118)
- Modify: `ui/src/screens/dot-editor/DotForm.tsx` (Limits section :273-296), `ui/src/lib/dot-rules.ts` (`Fields` :10, `specOf` :170)
- Modify: `ui/src/screens/SettingsScreen.tsx` (`settingsErrors` :36, runs section :262-291)
- Test: `ui/src/screens/home/QuietCard.test.tsx` (new), `ui/src/lib/dot-rules.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`, `ui/src/screens/SettingsScreen.test.tsx`, and the existing test of `applyRuntimeEvent`

**Interfaces:**
- Consumes: contract fields and event from Task 5; command `keep_waiting`.
- Produces: `commands.keepWaiting(runId: string)`; `<QuietCard run={run} />` (renders nothing unless `run.status === "running" && run.quiet_since && run.quiet_limit_secs`).

QuietCard numbers: `limitMin = quiet_limit_secs / 60`; `quietMin = limitMin + floor((now - quiet_since) / 60s)`; `stopsInMin = max(0, ceil((quiet_since + quiet_limit_secs - now) / 60s))`; re-render every 30 s.

- [ ] **Step 1: Write the failing tests**:
  - QuietCard: a quiet run with `quiet_limit_secs: 600` and `quiet_since` 3 min ago shows `Quiet for 13 min` and `Stops by itself in 7 min`; **Stop** calls `cancel_run`; **Keep waiting** calls `keep_waiting`; a rejected `keep_waiting` (`Conflict`) shows the error text on the card (Review Focus 5); not rendered for a run without `quiet_since`.
  - Review Focus 2: the card's minutes come from `run.quiet_limit_secs` (900 gives `Quiet for 15 min` at `quiet_since` = now) even when the dot's spec says `quiet_limit_secs: 60`.
  - `run_quiet` event invalidates the run, the topic and `keys.overview` queries.
  - dot-rules: `null` and `0` valid; `59` and `86401` give an error; `specOf` carries the field.
  - DotEditor: choosing *Off* saves `quiet_limit_secs: 0`; *App default* saves `null`; 15 minutes saves `900`; 60 minutes with `timeout_secs: 1800` shows the hint text.
  - Settings: the field shows 10 for `600`; entering 0 or 1441 shows an error and disables save; 20 saves `quiet_limit_secs: 1200`.

- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.

- [ ] **Step 3: Implement** schema fields (`quiet_since: z.string().nullable()`, `quiet_limit_secs: z.number().int().nullable()`, `quiet_stopped: z.boolean()` on runs; `quiet_limit_secs` nullable on the spec; a number on Settings), the event, the command, the invalidation, QuietCard (placed with the approval cards in all three places, waiting colour like `ApprovalCard`), the dot editor field and the Settings field with the Global Constraints copy.

- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS.

- [ ] **Step 5: Commit** `feat(ui): the quiet card and the quiet limit settings`.

### Task 7: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps locally: `cargo fmt --all -- --check`; `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`; `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times`; `cargo test -p dots-app --locked`; `node --test tools/release/release.test.mjs`; `cd ui; bun run typecheck; bun run test`. Expected: all pass.
- [ ] **Step 2: Manual check** in `bun run tauri dev` with a test dot (quiet limit 1 min, instructions that run `Start-Sleep 300`): the card and notification at 1 min, Keep waiting clears it, auto-stop at 2 min ends `failed` with `stopped: no output for 2 min`.
- [ ] **Step 3: Push** `feat/stuck-run-watchdog` and open the PR against `master`, body summarising the spec and the checks.
