# Dots — Scheduler v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scheduled dots are reliable on a desktop that sleeps, restarts and is closed: a schedule can run once, every few minutes, daily, weekly or by cron, with a bound; a slot missed while the PC was off runs once when Dots is back; a scheduled run that failed before it did anything is retried; and a schedule whose run failed in the end says so in the Inbox instead of failing silently.

**Architecture:** A dot's schedule becomes an object (`schedule::Schedule`: a rule, `catch_up`, `until`), and a pure module computes its slots in the PC's local time with written DST rules. Each scheduled dot has one row in a new table, `schedule_state`, which records the newest slot handled, how many slots fired, the run whose outcome decides the current occurrence, a pending retry and the attention flag. The runtime keeps the row in line with the dot; the scheduler loop reads the rows on a clock that tests move by hand, fires or skips each due slot in one guarded transaction, judges the watched run when it ends, starts retries and sets or clears the flag. The app gets one command and one notification; the UI builds the schedule by kind and shows the row's state in the Dots list, the Inbox and the run view.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, Tauri 2.12, chrono and the `cron` crate already in use; Bun 1.3, Vite 8.3, React 19.3, TypeScript 6.0 (strict), TanStack Query 5.104, zod 4.6, Vitest 5.0 with Testing Library and jsdom. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-14-scheduler-v2-design.md` (commit `c0c603a`, the base of this plan). It extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md`; the tasks edit the main spec's passages (Scheduler v2 spec section 11) in the commits that change the behaviour. Where this plan does something else than the Scheduler v2 spec, the table "Deliberate deviations from the spec" below says what and why; the Scheduler v2 spec itself is not edited by the tasks.

**Previous plans:** `docs/superpowers/plans/2026-10-08-dots-core.md` (Plan 1), `2026-10-09-dots-local-engine.md` (Plan 2), `2026-10-09-dots-app-3a.md`, `2026-10-12-dots-app-3b.md`, `2026-10-13-dots-app-3c.md` (Plan 3), `2026-10-13-approvals-v2.md` (Approvals v2) and its open points `2026-10-14-approvals-v2-followups.md`.

**Where the tasks come from:** the Scheduler v2 spec, whose ideas come from the review of Cloudflare OS (Approvals v2 spec, section 14). Tasks 1 to 5 are the core (Task 1 the pure schedule module, Task 2 the new type end to end, Task 3 the state row, Task 4 the loop, Task 5 judging, retry and attention), Task 6 the app's command and the contract, Tasks 7 and 8 the UI, Task 9 the smoke test.

**How this plan's code was checked:** the code blocks of Tasks 1 to 8 were generated from a throwaway worktree of commit `c0c603a`, one commit per task, where they compiled and passed on Windows 11. At the last task: `cargo test -p dots-core` passed 411 tests with 0 failures, `cargo test -p dots-app` passed 49, `bun run --cwd ui test` passed 294 tests in 21 files, `cargo clippy --workspace --all-targets` reported no warnings, `cargo fmt --check` and `tsc --noEmit` were clean. Every task's own state was measured the same way, in a separate checkout of that task's commit: the counts in each task's "Run everything" step come from there. Where a task has a "verify they fail" step, its result was measured by putting that task's test files onto the commit before it; the decisive lines quoted in those steps are from that measurement, except one line of Task 6 that says where it comes from. The plan was then replayed: its code blocks were applied by a script to a fresh checkout of `c0c603a`, and after every task the tree was the same as the prototype's commit (331 edits over 8 tasks). Not checked: Task 9 (the smoke test was not run; its steps are written from the spec's section 12 and the smokes of Plan 3 and Approvals v2), a build on Linux or macOS, and a run with the real `claude` CLI.

## Global Constraints

- Toolchain: Rust 1.95 stable; Node 24; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/scheduler-v2`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\scheduler-v2`, based on `c0c603a`. All commands run from the repo root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- No new dependency (Cargo or npm). One migration, `0005_scheduler_v2.sql` (Task 2), which holds the whole schema of spec section 4. `sqlx` runtime queries only. Timestamps the core writes are RFC 3339 UTC with milliseconds (`util::now()`, and `util::timestamp(t)` from Task 3 for a time the clock gives); the times a schedule names are local (spec section 2).
- How the code blocks are written. A step changes files in three ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
  - **Append to `path`:** the block is added at the end of the file.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`, plus `open` from `@tauri-apps/plugin-dialog`. No `fetch`, no other plugin, no new capability.
- Live data is parsed, not cast. A type the UI parses changes only together with the contract fixture: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then the same test without the variable, then `ui/src/api/types.ts`. The fixture `ui/src/api/__fixtures__/contract.json` is written by that command and never by hand.
- A query is marked stale after "the backend changed" with `invalidate(qc, key)` from `@/lib/query`, and a change to a dot with `invalidateDots(qc)`. Every action that cannot be undone asks first in an in-app dialog (`@/components/ConfirmButton`). The UI parses no schedule: what the core can be sent is checked in the UI, whether it would run is the core's answer (`preview_schedule`).
- Files under `ui/src/components/ui/` are generated by the shadcn CLI and are not edited. Imports inside `ui/src` use the `@/` alias.
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched, `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. Test output must be free of warnings. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first.
- The scheduler's tests run on a fake clock (`tests/common/clock.rs`, Task 4) and take well under a second; none of them sleeps for real. A scheduler test that hangs for 10 seconds and then fails in `nth_sleep` means the loop did not begin the sleep the test waited for: read the test, do not raise the wait.
- Known on this machine: a `vitest` run started right after an install, a `tsc` run, a cargo build or a large write can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`, or show 30 s timeouts in the tests that type into forms. Run it again; do not change the pool settings. Run the UI tests before the typecheck.
- Known flaky under load on this machine: `local_tools_test::shell_stops_at_its_timeout_and_on_cancel` (a 1 s timing test), `runtime_local_test::live_approval_lets_the_command_run`, `claude_engine_test::a_finished_run_leaves_no_process_behind`, some `resume_test` tests that make worktrees, and one form test of `DotEditorScreen.test.tsx`. None of them is touched by this plan. If one fails, run it alone; if it passes, note it and go on.
- A running `dots-app.exe` or `fake-claude.exe` under `target/` blocks cargo builds. An installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) may be running on this machine; it is the user's, it does not block builds, and nobody stops it. A smoke run uses a temporary `DOTS_DATA_DIR`, never the real `%LOCALAPPDATA%\dots`, and stops only the processes it started.
- The Fact-Forcing Gate hook blocks the first Edit or Write of each file and the first Bash command of a session. State the facts it asks for and repeat the identical call. For a large new file, send a one-line stub first.
- Subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials. An agent never runs the real `claude` CLI; the tests use the fake CLI and a scripted model endpoint.
- Commit messages: Conventional Commits; no AI co-author or attribution trailers.

### Changes a user will notice

- A dot's schedule is built by kind in the dot editor: None, Once, Every few minutes or hours, Daily, Weekly or Cron expression, with "Catch up a missed run" and a Stop (never, after a number of runs, after a date). The next runs are previewed, and a schedule that would never run is refused with the core's words before it is saved (Task 7). From Task 2 to Task 6 the editor still edits only a cron expression; a schedule of another kind is shown in words and kept on save.
- Existing cron dots keep their behaviour: the upgrade turns each cron expression into a cron schedule with catch-up off (Task 2).
- A slot missed while Dots was closed or the PC slept runs once when Dots is back, if catch-up is on and the slot is less than a day old; older missed slots are skipped. A resume of paused schedules still catches up nothing (Task 4).
- A scheduled run that failed before it made any tool call is retried 1, 2, 4, 8 and 16 minutes after each failure, six attempts in all. A failed scheduled run no longer notifies by itself; when an occurrence fails in the end, one notification says "<dot>: scheduled run failed" with the error (Task 5).
- The Inbox has a group "Needs attention": the dot, the failed run, its error, "Run now" and "Dismiss"; the Inbox count in the sidebar and in the tray include it (Tasks 5 and 8). A failed scheduled run that called no tool is no longer listed under "Runs to review" (Task 6) (changed during execution: the filter is gone; the scheduler marks such a run reviewed when it judges it into a retry or into "Needs attention", and a failure it never judged is listed). Between Task 5 and Task 8 the tray counts an attention item that the window does not show yet.
- The Dots list says "Completed", "Completed (missed)" or "Expired" for an ended schedule, shows a "Needs attention" badge, and "Retrying at 09:04 (attempt 2 of 6)" while a retry waits (Task 8).
- The run view says under a scheduled run's header which slot it was for, that it was caught up, or which retry it is, with a link to the run that failed (Task 8).

### Deliberate deviations from the spec

| Spec | This plan | Why |
|---|---|---|
| §2: `validate()` refuses "a time or date that does not parse" | Refused when the JSON is read: the fields are typed (`NaiveTime`, `NaiveDateTime`, `Weekday`), and a bad value is a serde error (`time "25:00": expected HH:MM`). Through the app it is an argument error of the command (Task 1) | A field that holds the parsed value cannot hold a bad one; `validate()` covers the rest of §2 |
| §2: `Every { minutes, anchor: DateTime<Utc> }` | `anchor: Option<DateTime<Utc>>`, `null` until the core fills it on save; a slot function given an `Every` without an anchor is `Invalid`, not a panic (Task 1) | The UI never sends an anchor (§2); the type has to say that a schedule on its way in has none yet |
| §3: Cron across DST "as the crate computes it", recorded | Recorded in `cron_across_the_gap_and_the_overlap`: a time in the gap is skipped, a time in the overlap fires twice (hourly at 02:00+02 and 02:00+01). One change: the slots are kept strictly increasing, so a cron with minute or second fields, which the crate steps back in the overlap, loses the rest of the first pass of that hour (Task 1) (changed during execution: an ambiguous wall time fires once, at its earlier instant, like Daily, and a gap time is skipped, so nothing of the first pass is lost; from the final review a wall time counts only at an instant that reads back as itself, for chrono's Windows `Local` at the end of the overlap and the start of the gap, `a_wall_time_counts_only_if_it_reads_back_as_itself`) | `last_slot_at` only moves forward; a slot before the last one would be refused by the store's guard anyway |
| §3: `Until::Date` is "inclusive" | Compared as an instant: an ambiguous date is its later instant, a date in the gap moves forward like a slot (Task 1) | Wall-clock values are not monotonic across the overlap |
| §4: `watch_run_id`, `attention_run_id` | Foreign keys to `runs(id)` `ON DELETE SET NULL` (Task 2). Deleting the watched run ends the occurrence (§10); deleting the flagged run also clears the flag | §10 "the watched run is deleted: the occurrence counts as done" then holds in the database |
| §4 lifecycle and §12 "schedule_state is empty until the first pass or save" | The runtime creates, resets and deletes the row with one statement after it wrote the dot, not in the same transaction (Task 3). The rows of dots upgraded by migration 0005 are made by `Runtime::start` before the scheduler starts, not by a pass (Task 4) | A pass that made rows would race the runtime's own writes and their events. The start is the same moment in practice. A lost enable-reset (the app stopped between the two writes) is not healed |
| §4 Count: completed "after the occurrence's outcome is judged" | Every completion waits for the last outcome: a count, a `Once` and a passed stop date become `completed` only when no run is watched and no retry waits (Task 5). Until Task 5 there is no judging, and Task 4 completes on the slot | A pending retry still runs, whatever the bound is |
| §5 Pause: on resume every active row gets `last_slot_at = now` | Done at the first pass after the resume, with the clock's time of that pass; it moves the rows of disabled dots too, and never moves a row back (Task 4) | `set_paused` is synchronous and has no store; a flag carries the resume to the loop, so a quick pause and resume is not lost |
| §5 Clock: "`now()` and a sleep that can be woken" | `Clock::now()` and `sleep_until(deadline)`; the loop's other wakes (reload, pause, shutdown, the end of a watched run) are a `biased` `select!` that drops the sleep (Task 4) | A small trait keeps the fake clock simple |
| §6: the retry needs "the failed run" | The failed run stays the watched run while its retry waits, and judging is skipped meanwhile (Task 5) | The row has no column for it; `retry_of` is taken from the watch |
| §6: attempts | The slot and the attempt come from the occurrence's first run (the head of the resume chain): its payload's `scheduled_for` and `attempt` (1 when missing). A retry joined into a queued manual or webhook run that then fails before a tool call starts again at attempt 2 (Task 5) (changed during execution: a slot that drops a pending retry carries its attempt in its run's payload, so a schedule that fires more often than its retries come reaches the sixth attempt; from the final review a due retry whose slot is 24 h old, or that is 60 s overdue on a schedule without catch-up, is dropped into "Needs attention") | No column and no extra state; the spec clears the retry fields when the retry starts. The total stays bounded: each join needs a run queued from outside |
| §6: "the run has no `tool_call` event" | The run and the chain of parked runs it resumes (Task 5) | A resume child of a run that called a tool may continue what that run changed |
| §6: the flag clears when "a later scheduled run of the dot succeeds" | Also when a later manual run succeeds ("Run now"); not a chat, webhook or resume run. The pass looks for it, so it is cleared within 60 s (Task 5) (changed during execution: the pass looks at every flagged row, whatever its status and whether the dot is enabled, and a resume run counts when its chain starts with a manual or scheduled run) | §9's Inbox group offers "Run now" and §12's smoke expects it to clear the flag |
| §7: "Setting `attention_run_id` notifies once" | A new event, `RuntimeEvent::ScheduleAttention { dot_id, run_id, error }`, published when the flag goes from clear to set; a further failure moves the flag to the newer run without a second notice. Title "<dot>: scheduled run failed", body the error's first line ("The run failed." without one) (Task 5) | `ScheduleUpdated` carries only the dot; the existing notifications are a title and a body |
| §7: "A failed run with trigger `schedule` no longer notifies" | Only trigger `schedule`: a manual run a slot joined, or a resume child, still notifies its own failure (Task 5) | The spec's wording; such a run was started by a person or decided by one |
| Not in the spec (main spec §12 Inbox) | A failed run with trigger `schedule` that made no tool call is left out of "Runs to review", without being marked reviewed (Task 6) (changed during execution: replaced by marking reviewed, in the judging write, a failure that made no tool call of a run with trigger `schedule` that the scheduler judges into a retry or into "Needs attention"; a failure after a tool call, and a joined manual or webhook run, stay for review; the filter is gone, so a failure the scheduler never judged, such as an overrun run, is listed) | Ruling of this plan: there is nothing to review, and its occurrence is retried or ends in "Needs attention". Without it six failed attempts would be six review items |
| §8: `preview_schedule(schedule, n: u32)` answers `[]` when no slot is left | `count: usize`, clamped to 1..=10 (Task 2); from Task 7 a schedule with no slot left is refused with "this schedule would never run", the same refusal as a save | The editor says why before a save, in the core's words |
| §8: `DotOverview.schedule_state: Option<ScheduleState>` | The type is `ScheduleView` (the JSON field is `schedule_state`), the attention `AttentionRun { run_id, error }`. `retry_at` is RFC 3339 local like `next_fire`. `next_fire`, `retry_at` and `retry_attempt` are null while the dot is disabled or the row not `active`; `next_fire` is null once the count is reached, and is the first slot after `max(now, last_slot_at)` (Task 6) | `ScheduleState` is the row type (Task 3). A next run that cannot happen, or one in the past, would read wrong |
| §8: `Inbox.attention` | Lists the flags of disabled dots too (Task 5) | The flag stays until it is dismissed or the row is reset; enabling the dot resets it |
| §4 Migration: new schedules default to catch-up on | A new cron schedule saved from the editor has `catch_up: true` (Task 7); from Task 2 to Task 6 the interim cron field saved `false`. Converted cron schedules keep `false` and show the switch off | Only converted schedules are `false` |
| §9: "Catch up a missed run" is a checkbox | A `Switch` (Task 7) | The form's boolean control |
| §9: the core's objection shows under the field | It blocks a save only for a new or changed schedule; for a stored schedule that is unchanged (a `Once` that has passed) it is a note and the save goes through. An emptied cron field is an error; the kind None removes the schedule (Task 7) | The core keeps an unchanged schedule without the check, and a dot whose `Once` passed must still be editable |
| §9: "Retry 2 of 6 of run <link>"; "Scheduled for 09:00" | "Retry 2 of 6 of the failed run", the words being the link; a retry shows only that line. "Scheduled for" gives the date and the time. The 6 is `Limits.max_schedule_attempts` from the contract (Task 8) (changed during execution: a slot's run that carries an attempt, without `retry_of`, shows "Scheduled for … · attempt k of 6"; "Retry k of 6 of the failed run" is for a run with `retry_of`) | A run id says nothing to the reader; a caught-up or retried run can be a day away from its slot |
| §10: "completed (missed)" | Any completed row with `fired_count == 0`, also a stop date that passed before the first slot. The labels are capitalised; an ended state wins over "Paused"; the retry line is hidden while schedules are paused or the dot is off (Task 8) | The view has no separate "missed" flag; nothing can retry while paused |
| §12: the manual smoke | Task 9 holds the steps; the prototype did not run them | The smoke needs LM Studio, a built app and about an hour |

## Review Focus

1. **A slot run twice, or a missed slot run more than once after a sleep, a close or a restart.** One slot must start or join at most one run, a guarded write must refuse a slot that is not newer than the last one, and a gap of any length must give at most one catch-up run, for the newest slot only. → Task 3 `a_slot_that_is_not_newer_than_the_last_one_is_refused`, `a_slot_joins_the_queued_run`; Task 4 `a_slot_fires_on_time_and_a_later_one_joins_the_queued_run`, `a_slot_up_to_59_seconds_late_is_on_time_and_60_seconds_is_missed`, `only_the_newest_missed_slot_is_caught_up_once`, `a_missed_slot_is_skipped_without_catch_up_or_when_a_day_old`, `a_resume_does_not_catch_up_the_slots_of_the_pause`, `a_resume_moves_every_active_row_to_now_and_keeps_the_rest`; Task 5 `a_restart_while_a_retry_waits_keeps_it`, `a_watched_run_that_ended_while_dots_was_closed_is_judged_at_start`.
2. **A retry that repeats what a run already did, or retries without end.** Only a failure before any tool call may be retried, also across a parked run and its resume child; at most six attempts; a new slot or a deleted dot drops the retry. Look at the attempt counting through a joined manual run (deviation above). → Task 5 `a_failure_before_any_call_is_retried_up_to_the_sixth_attempt`, `a_failure_after_a_tool_call_needs_attention_until_a_scheduled_run_succeeds`, `the_resume_child_of_a_parked_run_takes_over_the_watch`, `a_cancelled_or_deleted_watched_run_ends_the_occurrence_quietly`, `a_due_retry_joins_the_queued_run`, `a_slot_that_fires_drops_a_waiting_retry`, `a_dot_deleted_while_a_retry_waits_is_never_retried`, `a_retry_that_came_due_during_a_pause_starts_on_resume`; Task 3 `a_retry_starts_a_run_without_counting`, `a_retry_joins_the_queued_run`.
3. **DST days.** A daily time in the spring-forward gap must move forward, one in the fall-back overlap must fire once, `Every` must count elapsed time, a stop date must be inclusive, and the newest-slot search must apply the same rules as the forward walk. The scheduler's own tests run in `Local` on UTC instants of 2030, so the rules are checked at the level of the slot functions, with a test zone; a person checks a real DST day only by chance (Task 9, B4). → Task 1 `a_time_in_the_gap_moves_forward_by_the_gap`, `a_time_in_the_overlap_fires_once_at_the_earlier_instant`, `every_does_not_follow_the_wall_clock_across_the_gap_and_the_overlap`, `cron_across_the_gap_and_the_overlap`, `newest_slot_in_applies_the_dst_rules`, `until_date_is_inclusive_and_resolved_like_a_slot`, `slots_work_in_the_local_zone`.
4. **A schedule that silently never runs, or never stops.** A schedule that would never run must be refused on save but a stored one must still load; an upgraded dot must get its row; a schedule that no longer parses must say `expired`; a bound must complete only after the last outcome; a paused scheduler must do nothing, and a reload or the end of a watched run must wake the loop. → Task 2 `a_schedule_that_would_never_run_is_refused_on_create_and_on_change`, `a_stored_schedule_whose_date_passed_loads_and_saves_unchanged`, `migration_0005_turns_a_cron_string_into_a_schedule_without_catch_up`, `the_core_sets_the_anchor_of_an_every_schedule`; Task 3 `the_schedule_row_follows_the_dot`, `only_a_dot_with_a_schedule_has_a_row_and_it_goes_with_the_dot`; Task 4 `the_runtime_takes_the_times_of_its_rows_from_its_clock`, `a_schedule_that_no_longer_parses_expires`, `a_scheduler_started_paused_does_nothing_until_resumed`, `reload_wakes_the_loop_for_a_new_schedule`, `a_reached_count_waits_for_a_pending_retry`; Task 5 `a_count_completes_the_schedule_after_the_last_outcome`, `once_is_completed_when_its_slot_fired_and_its_run_is_judged`, `a_stop_date_completes_the_schedule_after_its_last_slot`, `the_end_of_a_watched_run_wakes_the_loop`.
5. **The screen saying something that is not so.** A next run that cannot happen, a retry shown while nothing can run, a flag that stays after a run succeeded or goes without a word, a past `Once` the editor refuses although it is only kept, a notification for every failed attempt. → Task 5 `attention_is_listed_in_the_inbox_until_dismissed`, `a_run_now_that_succeeds_clears_the_attention`, and in `apps/dots-app/src/events.rs` `a_failed_scheduled_run_is_quiet_and_its_attention_notifies`; Task 6 `a_schedule_view_says_what_comes_next`, `the_overview_shows_where_each_schedule_stands`, `a_failed_scheduled_run_that_called_no_tool_is_not_for_review` (changed during execution: that test is replaced by `a_failed_run_is_for_review_whatever_its_trigger`, `the_write_that_judges_a_failed_run_marks_it_reviewed`, `an_overrun_run_that_fails_is_for_review_and_a_judged_one_is_not` and `a_joined_manual_run_that_fails_stays_for_review`); Task 7 "says why a once in the past would never run, before a save", "keeps a stored once that has passed, and refuses another past date", "previews a stored every schedule from its anchor, kept while the minutes stay"; Task 8 "says when a retry waits, unless nothing can run", "shows how an ended schedule ended in place of the next run", "says which attempt a retry is, and links the run that failed", "keeps the flag and says why when the core refuses the dismissal".

## File Map

```
crates/dots-core/
  migrations/0005_scheduler_v2.sql     dots.schedule as JSON, runs.retry_of, schedule_state (new)      (Task 2)
  src/schedule.rs                      Schedule, Rule, Until, validation, slots, DST rules (new)       (Task 1)
                                       Schedule::cron, prepare_save                                    (Task 2)
                                       rule_key                                                        (Task 3)
  src/lib.rs                           pub mod schedule                                                (Task 1)
  src/model.rs                         DotSpec.schedule: Option<Schedule>                              (Task 2)
                                       Run/NewRun.retry_of, ScheduleStatus, ScheduleState, PendingRetry,
                                       ScheduleChange, SlotStart                                       (Task 3)
                                       NeedsAttention, Inbox.attention                                 (Task 5)
                                       ScheduleView, AttentionRun, DotOverview.schedule_state          (Task 6)
  src/store/dots.rs                    the schedule column as JSON; anchor and would-never-run check   (Task 2)
  src/store/schedules.rs               the schedule_state row: sync, record_slot, start_retry, update  (Task 3)
                                       resume_schedules                                                (Task 4)
                                       resume_child, called_a_tool, succeeded_after, attention list    (Task 5)
  src/store/runs.rs, src/store/mod.rs  retry_of; insert_run shared                                     (Task 3)
                                       failed scheduled runs without a tool call not for review        (Task 6)
  src/util.rs                          timestamp                                                       (Task 3)
  src/events.rs                        RuntimeEvent::ScheduleUpdated                                   (Task 3)
                                       RuntimeEvent::ScheduleAttention                                 (Task 5)
  src/scheduler.rs                     slots through the new module                                    (Task 2)
                                       the loop: Clock, passes, catch-up, pause, completion            (Task 4)
                                       judging, retry, attention, MAX_ATTEMPTS                         (Task 5)
  src/runner.rs                        Runner::enqueued                                                (Task 4)
  src/runtime.rs                       preview_schedule(schedule, count), next_fire                    (Task 2)
                                       the row's lifecycle                                             (Task 3)
                                       Config.clock, rows made at start                                (Task 4)
                                       inbox attention, dismiss_schedule_attention                     (Task 5)
                                       schedule_state in the overview                                  (Task 6)
                                       preview refuses a schedule that would never run                 (Task 7)
  tests/common/{tz,clock,mod}.rs       TestTz (Task 1), FakeClock (Task 4), retry_of (Task 3)
  tests/{schedule,store_dots,scheduler,runtime_review,runtime_status,store_schedules,
         runtime_schedule,store_runs}_test.rs                                                          (Tasks 1-7)
apps/dots-app/
  src/commands.rs, src/lib.rs          preview_schedule's object (Task 2); dismiss_schedule_attention  (Task 6)
  src/contract.rs                      Schedule samples (2), events (3, 5), schedule_state, enums (6),
                                       Limits.max_schedule_attempts (8)
  src/events.rs                        a failed scheduled run is quiet; attention notifies             (Tasks 3, 5)
ui/src/
  api/__fixtures__/contract.json       regenerated                                                     (Tasks 2, 3, 5, 6, 8)
  api/types.ts                         Schedule (2), retry_of, schedule_updated (3), NeedsAttention (5),
                                       ScheduleView (6), LIMITS.max_schedule_attempts (8)
  api/commands.ts                      previewSchedule(schedule) (2), dismissScheduleAttention (6)
  lib/query.ts, lib/runtime-events.ts  keys.schedulePreview (2); schedule events (3, 5)
  lib/dot-rules.ts                     scheduleOf (2); kinds, scheduleError, withKind, everyRule (7)
  lib/format.ts                        localTime                                                       (Task 8)
  screens/dot-editor/ScheduleEditor.tsx  the schedule builder (new)                                    (Task 7)
  screens/dot-editor/DotForm.tsx, screens/DotEditorScreen.tsx  cron field (2); the builder (7)
  screens/DotsScreen.tsx               next_fire from schedule_state (6); states, badge, retry line (8)
  screens/InboxScreen.tsx, app/Layout.tsx  "Needs attention" group; the badge count                    (Task 8)
  components/Transcript.tsx            the line under a scheduled run's trigger                        (Task 8)
  test/fixtures.ts, api/*.test.ts, lib/*.test.ts, app/Layout.test.tsx, screens/**/*.test.tsx           (Tasks 2-8)
docs/
  superpowers/specs/2026-10-08-dots-design.md   the passages Scheduler v2 changes                      (Tasks 1-8)
  verification/2026-10-14-scheduler-v2-smoke.md the smoke of this plan (new)                           (Task 9)
```

---

### Task 1: Schedules and their slots, with written DST rules

Everything else stands on a pure module: the `Schedule` type with its JSON form (spec section 2), its structural checks, and the functions that say when a schedule is due, in a time zone given as a type parameter, with the DST rules of spec section 3. The tests run in a hand-written test zone, `TestTz`, which has a one-hour spring-forward gap and a one-hour fall-back overlap in 2026, so the rules are checked without a new dependency and whatever zone the machine is in. One test records what the `cron` crate does across the gap and the overlap. `DotSpec` does not change yet and nothing calls the module. The main spec gets the module in its list (section 2) and the slot tests in its testing section.

**Files:**
- Create: `crates/dots-core/src/schedule.rs`, `crates/dots-core/tests/common/tz.rs`, `crates/dots-core/tests/schedule_test.rs`
- Modify: `crates/dots-core/src/lib.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`
- Test: `crates/dots-core/tests/common/mod.rs`, `crates/dots-core/tests/common/tz.rs`, `crates/dots-core/tests/schedule_test.rs`

**Interfaces:**
- Consumes: `scheduler::parse_cron` (Plan 1; it stays in `scheduler.rs`), `Error::Invalid`.
- Produces, in `dots_core::schedule`:
  - `Schedule { rule: Rule, catch_up: bool, until: Option<Until> }`; `catch_up` is `true` when the JSON leaves it out, `until` is `None`.
  - `Rule` (tag `kind`, snake case): `Once { at: NaiveDateTime }` (`"2026-10-20T09:00"`), `Every { minutes: u32, anchor: Option<DateTime<Utc>> }` (`"anchor": null` when `None`), `Daily { time: NaiveTime }` (`"09:00"`), `Weekly { days: Vec<Weekday>, time: NaiveTime }` (`["mon", "fri"]`), `Cron { expr: String }`.
  - `Until`: `Count(u32)` (`{"count": 10}`) or `Date(NaiveDateTime)` (`{"date": "2026-10-20T09:00"}`).
  - `MAX_EVERY_MINUTES = 10080`; `type Slots<Tz> = Box<dyn Iterator<Item = DateTime<Tz>>>` (not `Send`: collect it before an `.await`).
  - `Schedule::validate(&self) -> Result<()>`: `Invalid` with "every: minutes must be 1 to 10080 (7 days)", "weekly: pick at least one day", the cron parser's message, "once: a stop (until) is not allowed", "until: count must be at least 1".
  - `Schedule::check_runs_after(&self, now, tz) -> Result<()>`: `Invalid("this schedule would never run")` when no slot is left after `now`; a missing `Every` anchor counts as `now`.
  - `Schedule::fill_anchor(&mut self, previous: Option<&Schedule>, now: DateTime<Utc>)`: an `Every` without an anchor keeps `previous`'s anchor while the minutes are the same, and takes `now` otherwise; an anchor sent back unchanged with other minutes becomes `now`; any other anchor is kept.
  - `Schedule::until_instant(&self, tz) -> Option<DateTime<Tz>>`: `Until::Date` as an instant (an ambiguous time is its later instant; a time in the gap moves forward like a slot).
  - `slots_after(rule, after, tz) -> Result<Slots<Tz>>`: the slots strictly after `after`, strictly increasing.
  - `newest_slot_in(rule, after, upto, tz) -> Result<Option<DateTime<Tz>>>`: the newest slot with `after < slot <= upto`, without walking forward from `after` (a window of years is cheap).
  - `next_slot(schedule, after, tz)`: the first slot after `after` within `Until::Date`; `None` when no slot is left. `due_slot(schedule, after, upto, tz)`: `newest_slot_in` with `upto` capped at `Until::Date`. `upcoming(schedule, after, tz) -> Result<Slots<Tz>>`: the slots within the date bound, taking `Until::Count` of them.
  - `after` and `upto` may be in any zone; results are in `tz`; `Tz: TimeZone + 'static`. The scheduler will pass `&Local`.
  - DST rules of `Daily` and `Weekly`: a time in the gap moves forward by the gap; a time in the overlap fires once, at the earlier instant. `Every` is `anchor + k * minutes` for whole `k`, in elapsed time.
- Test helpers, in `tests/common/tz.rs`: `TestTz` (UTC+1, UTC+2 from 2026-03-29 01:00Z to 2026-10-25 01:00Z; `Copy + Send + Sync + 'static`), `at("2026-10-25T02:30:00+02:00") -> DateTime<TestTz>`, `rfc(iter) -> Vec<String>`. Every test binary with `mod common;` can use them.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/common/mod.rs`, replace:**

```rust
#![allow(dead_code)]

use std::path::Path;
```

**with:**

```rust
#![allow(dead_code)]

pub mod tz;

use std::path::Path;
```

**Create `crates/dots-core/tests/common/tz.rs`:**

```rust
//! A hand-written time zone for schedule tests, so they do not depend on the PC's zone.
//!
//! `TestTz` is UTC+1, with summer time (UTC+2) from 2026-03-29 01:00 UTC to 2026-10-25
//! 01:00 UTC: local clocks jump from 02:00 to 03:00 on 29 March 2026 (a one-hour gap) and
//! fall back from 03:00 to 02:00 on 25 October 2026 (a one-hour overlap). Other years have
//! no summer time.

use chrono::{DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestTz;

fn utc(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap()
}

fn winter() -> FixedOffset {
    FixedOffset::east_opt(3600).unwrap()
}

fn summer() -> FixedOffset {
    FixedOffset::east_opt(7200).unwrap()
}

impl TimeZone for TestTz {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        TestTz
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        // The larger offset first: it gives the earlier instant.
        let fits: Vec<FixedOffset> = [summer(), winter()]
            .into_iter()
            .filter(|o| {
                let at = *local - TimeDelta::seconds(o.local_minus_utc() as i64);
                self.offset_from_utc_datetime(&at) == *o
            })
            .collect();
        match fits[..] {
            [o] => LocalResult::Single(o),
            [earliest, latest] => LocalResult::Ambiguous(earliest, latest),
            _ => LocalResult::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, at: &NaiveDateTime) -> FixedOffset {
        if *at >= utc("2026-03-29T01:00") && *at < utc("2026-10-25T01:00") {
            summer()
        } else {
            winter()
        }
    }
}

/// An instant written in RFC 3339 with its offset, e.g. `2026-10-25T02:30:00+02:00`.
pub fn at(s: &str) -> DateTime<TestTz> {
    DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&TestTz)
}

/// Instants as RFC 3339 with their `TestTz` offset, for readable assertions.
pub fn rfc(times: impl IntoIterator<Item = DateTime<TestTz>>) -> Vec<String> {
    times.into_iter().map(|t| t.to_rfc3339()).collect()
}
```

**Create `crates/dots-core/tests/schedule_test.rs`:**

```rust
mod common;

use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, TimeDelta, Utc, Weekday};
use common::tz::{at, rfc, TestTz};
use dots_core::schedule::{
    due_slot, newest_slot_in, next_slot, slots_after, upcoming, Rule, Schedule, Until,
};
use dots_core::{Error, Result};
use serde_json::{json, Value};

fn hm(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s, "%H:%M").unwrap()
}

fn local(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn once(s: &str) -> Rule {
    Rule::Once { at: local(s) }
}

fn every(minutes: u32, anchor: &str) -> Rule {
    Rule::Every {
        minutes,
        anchor: Some(utc(anchor)),
    }
}

fn daily(time: &str) -> Rule {
    Rule::Daily { time: hm(time) }
}

fn weekly(days: &[Weekday], time: &str) -> Rule {
    Rule::Weekly {
        days: days.to_vec(),
        time: hm(time),
    }
}

fn cron(expr: &str) -> Rule {
    Rule::Cron { expr: expr.into() }
}

fn on(rule: Rule, until: Option<Until>) -> Schedule {
    Schedule {
        rule,
        catch_up: true,
        until,
    }
}

/// The first `n` slots of `rule` strictly after `after`, in `TestTz`.
fn first(rule: &Rule, after: &str, n: usize) -> Vec<String> {
    rfc(slots_after(rule, &at(after), &TestTz).unwrap().take(n))
}

fn newest(rule: &Rule, after: &str, upto: &str) -> Option<String> {
    newest_slot_in(rule, &at(after), &at(upto), &TestTz)
        .unwrap()
        .map(|t| t.to_rfc3339())
}

fn invalid(r: Result<()>) -> String {
    match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

// --- JSON ---

#[test]
fn a_schedule_is_json_with_a_kind_tag_as_the_spec_writes_it() {
    let v = json!({"rule": {"kind": "daily", "time": "09:00"}, "catch_up": true, "until": {"count": 10}});
    let s: Schedule = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(s, on(daily("09:00"), Some(Until::Count(10))));
    assert_eq!(serde_json::to_value(&s).unwrap(), v);
}

#[test]
fn every_kind_and_bound_round_trips() {
    let rules = [
        json!({"kind": "once", "at": "2026-10-20T09:00"}),
        json!({"kind": "every", "minutes": 90, "anchor": "2026-10-14T08:00:00Z"}),
        json!({"kind": "daily", "time": "23:59"}),
        json!({"kind": "weekly", "days": ["mon", "fri", "sun"], "time": "07:30"}),
        json!({"kind": "cron", "expr": "0 0 9 * * *"}),
    ];
    for rule in rules {
        let v = json!({"rule": rule, "catch_up": false, "until": null});
        let s: Schedule = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap(), v);
    }
    let v = json!({"rule": {"kind": "daily", "time": "09:00"}, "catch_up": true, "until": {"date": "2026-10-20T09:00"}});
    let s: Schedule = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(s.until, Some(Until::Date(local("2026-10-20T09:00"))));
    assert_eq!(serde_json::to_value(&s).unwrap(), v);
    let s: Schedule = serde_json::from_value(
        json!({"rule": {"kind": "weekly", "days": ["sat"], "time": "10:00"}}),
    )
    .unwrap();
    assert_eq!(s.rule, weekly(&[Weekday::Sat], "10:00"));
}

#[test]
fn catch_up_defaults_to_true_and_a_missing_anchor_to_none() {
    let s: Schedule =
        serde_json::from_value(json!({"rule": {"kind": "every", "minutes": 30}})).unwrap();
    assert_eq!(
        s,
        on(
            Rule::Every {
                minutes: 30,
                anchor: None
            },
            None
        )
    );
    assert_eq!(
        serde_json::to_value(&s).unwrap()["rule"]["anchor"],
        Value::Null
    );
}

#[test]
fn times_dates_and_days_that_do_not_parse_are_refused() {
    let bad = [
        json!({"kind": "daily", "time": "25:00"}),
        json!({"kind": "daily", "time": "09:00:30"}),
        json!({"kind": "daily", "time": "9am"}),
        json!({"kind": "once", "at": "2026-10-20 09:00"}),
        json!({"kind": "once", "at": "2026-02-30T09:00"}),
        json!({"kind": "weekly", "days": ["mon", "funday"], "time": "09:00"}),
        json!({"kind": "monthly"}),
    ];
    for rule in bad {
        let r = serde_json::from_value::<Schedule>(json!({ "rule": rule }));
        assert!(r.is_err(), "{rule} should not parse");
    }
    let e = serde_json::from_value::<Schedule>(json!({"rule": {"kind": "daily", "time": "25:00"}}))
        .unwrap_err();
    assert!(e.to_string().contains("expected HH:MM"), "{e}");
    let e = serde_json::from_value::<Schedule>(
        json!({"rule": {"kind": "daily", "time": "09:00"}, "until": {"date": "tomorrow"}}),
    )
    .unwrap_err();
    assert!(e.to_string().contains("expected YYYY-MM-DDTHH:MM"), "{e}");
}

// --- Validation ---

#[test]
fn validate_refuses_each_structural_mistake() {
    let anchor = "2026-10-14T08:00:00Z";
    assert!(invalid(on(every(0, anchor), None).validate()).contains("1 to 10080"));
    assert!(invalid(on(every(10081, anchor), None).validate()).contains("1 to 10080"));
    assert!(invalid(on(weekly(&[], "09:00"), None).validate()).contains("at least one day"));
    assert!(invalid(on(cron("* * *"), None).validate()).contains("cron"));
    assert!(invalid(on(cron("0 0 25 * * *"), None).validate()).contains("cron"));
    assert!(invalid(on(daily("09:00"), Some(Until::Count(0))).validate()).contains("at least 1"));
    let once_until = on(
        once("2026-10-20T09:00"),
        Some(Until::Date(local("2026-10-21T09:00"))),
    );
    assert!(invalid(once_until.validate()).contains("once"));
}

#[test]
fn validate_accepts_each_kind_within_bounds() {
    let anchor = "2026-10-14T08:00:00Z";
    for s in [
        on(once("2020-01-01T00:00"), None),
        on(every(1, anchor), Some(Until::Count(1))),
        on(every(10080, anchor), None),
        on(
            Rule::Every {
                minutes: 5,
                anchor: None,
            },
            None,
        ),
        on(daily("00:00"), Some(Until::Date(local("2026-10-20T09:00")))),
        on(weekly(&[Weekday::Mon], "09:00"), None),
        on(cron("0 0 9 * * Mon-Fri"), None),
        on(cron("0 0 9 1 1 * 2030"), None),
    ] {
        s.validate().unwrap_or_else(|e| panic!("{s:?}: {e}"));
    }
}

#[test]
fn a_schedule_that_would_never_run_is_refused() {
    let now = at("2026-10-14T10:00:00+02:00");
    let never = |s: Schedule| {
        let m = invalid(s.check_runs_after(&now, &TestTz));
        assert!(m.contains("this schedule would never run"), "{m}");
    };
    never(on(once("2026-10-14T09:00"), None));
    never(on(once("2026-10-14T10:00"), None));
    never(on(
        daily("09:00"),
        Some(Until::Date(local("2026-10-15T08:59"))),
    ));
    never(on(
        Rule::Every {
            minutes: 60,
            anchor: None,
        },
        Some(Until::Date(local("2026-10-14T10:30"))),
    ));
    never(on(cron("0 0 9 1 1 * 2020"), None));

    let runs = |s: Schedule| s.check_runs_after(&now, &TestTz).unwrap();
    runs(on(once("2026-10-14T10:01"), None));
    runs(on(
        daily("09:00"),
        Some(Until::Date(local("2026-10-15T09:00"))),
    ));
    runs(on(
        Rule::Every {
            minutes: 60,
            anchor: None,
        },
        None,
    ));
    runs(on(daily("09:00"), Some(Until::Count(1))));
}

#[test]
fn the_core_fills_the_anchor_and_keeps_it_while_minutes_stay() {
    let now = utc("2026-10-14T08:00:00.123Z");
    let old = utc("2026-10-01T06:30:00Z");
    let bare = |minutes| {
        on(
            Rule::Every {
                minutes,
                anchor: None,
            },
            None,
        )
    };
    let anchor_of = |s: &Schedule| match s.rule {
        Rule::Every { anchor, .. } => anchor,
        _ => panic!("not every"),
    };
    let previous = on(
        Rule::Every {
            minutes: 30,
            anchor: Some(old),
        },
        None,
    );

    let mut s = bare(30);
    s.fill_anchor(None, now);
    assert_eq!(anchor_of(&s), Some(now), "a new schedule starts now");

    let mut s = bare(30);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(anchor_of(&s), Some(old), "same minutes keep the phase");

    let mut s = bare(45);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(anchor_of(&s), Some(now), "new minutes restart the phase");

    let mut s = previous.clone();
    if let Rule::Every { minutes, .. } = &mut s.rule {
        *minutes = 45;
    }
    s.fill_anchor(Some(&previous), now);
    assert_eq!(
        anchor_of(&s),
        Some(now),
        "the old anchor sent back with new minutes"
    );

    let mut s = on(every(30, "2026-10-10T00:00:00Z"), None);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(anchor_of(&s), Some(utc("2026-10-10T00:00:00Z")));

    let mut s = on(daily("09:00"), None);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(s, on(daily("09:00"), None));
}

// --- Slots ---

#[test]
fn once_is_one_slot_strictly_after_after() {
    let r = once("2026-10-20T09:00");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 3),
        ["2026-10-20T09:00:00+02:00"]
    );
    assert!(first(&r, "2026-10-20T09:00:00+02:00", 3).is_empty());
    assert_eq!(
        first(&once("2026-03-29T02:30"), "2026-03-28T00:00:00+01:00", 3),
        ["2026-03-29T03:30:00+02:00"]
    );
}

#[test]
fn every_counts_elapsed_time_from_its_anchor() {
    let r = every(15, "2026-10-14T07:07:00Z");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 2),
        ["2026-10-14T10:07:00+02:00", "2026-10-14T10:22:00+02:00"]
    );
    // Whole k, so also before the anchor.
    assert_eq!(
        first(&r, "2026-10-14T08:50:00+02:00", 2),
        ["2026-10-14T08:52:00+02:00", "2026-10-14T09:07:00+02:00"]
    );
    assert_eq!(
        first(&r, "2026-10-14T10:07:00+02:00", 1),
        ["2026-10-14T10:22:00+02:00"]
    );
}

#[test]
fn every_does_not_follow_the_wall_clock_across_the_gap_and_the_overlap() {
    let spring = every(60, "2026-03-28T23:00:00Z");
    assert_eq!(
        first(&spring, "2026-03-29T00:00:00+01:00", 3),
        [
            "2026-03-29T01:00:00+01:00",
            "2026-03-29T03:00:00+02:00",
            "2026-03-29T04:00:00+02:00"
        ]
    );
    let autumn = every(60, "2026-10-24T22:00:00Z");
    assert_eq!(
        first(&autumn, "2026-10-25T00:30:00+02:00", 4),
        [
            "2026-10-25T01:00:00+02:00",
            "2026-10-25T02:00:00+02:00",
            "2026-10-25T02:00:00+01:00",
            "2026-10-25T03:00:00+01:00"
        ]
    );
}

#[test]
fn every_without_an_anchor_has_no_slots() {
    let r = Rule::Every {
        minutes: 5,
        anchor: None,
    };
    assert!(matches!(
        slots_after(&r, &at("2026-10-14T10:00:00+02:00"), &TestTz),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn daily_is_the_wall_clock_time_on_each_day() {
    let r = daily("09:00");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 2),
        ["2026-10-15T09:00:00+02:00", "2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(
        first(&r, "2026-10-15T09:00:00+02:00", 1),
        ["2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(
        first(&r, "2026-10-14T08:59:59+02:00", 1),
        ["2026-10-14T09:00:00+02:00"]
    );
}

#[test]
fn a_time_in_the_gap_moves_forward_by_the_gap() {
    assert_eq!(
        first(&daily("02:30"), "2026-03-28T12:00:00+01:00", 2),
        ["2026-03-29T03:30:00+02:00", "2026-03-30T02:30:00+02:00"]
    );
    assert_eq!(
        first(&daily("02:00"), "2026-03-28T12:00:00+01:00", 1),
        ["2026-03-29T03:00:00+02:00"]
    );
}

#[test]
fn a_time_in_the_overlap_fires_once_at_the_earlier_instant() {
    let r = daily("02:30");
    assert_eq!(
        first(&r, "2026-10-24T12:00:00+02:00", 2),
        ["2026-10-25T02:30:00+02:00", "2026-10-26T02:30:00+01:00"]
    );
    assert_eq!(
        first(&r, "2026-10-25T02:30:00+02:00", 1),
        ["2026-10-26T02:30:00+01:00"]
    );
}

#[test]
fn weekly_fires_on_its_days_only() {
    let r = weekly(&[Weekday::Fri, Weekday::Mon], "09:00");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 4),
        [
            "2026-10-16T09:00:00+02:00",
            "2026-10-19T09:00:00+02:00",
            "2026-10-23T09:00:00+02:00",
            "2026-10-26T09:00:00+01:00"
        ]
    );
}

#[test]
fn cron_is_what_the_cron_crate_computes() {
    let r = cron("0 0 9 * * Mon-Fri");
    assert_eq!(
        first(&r, "2026-10-16T10:00:00+02:00", 2),
        ["2026-10-19T09:00:00+02:00", "2026-10-20T09:00:00+02:00"]
    );
    assert!(matches!(
        slots_after(&cron("nonsense"), &at("2026-10-14T10:00:00+02:00"), &TestTz),
        Err(Error::Invalid(_))
    ));
}

/// Records what the `cron` crate does across DST (spec §3); Dots does not change it, except
/// that a slot earlier than the one before it is dropped, so slots stay in order.
#[test]
fn cron_across_the_gap_and_the_overlap() {
    // Gap: a time that does not exist is skipped, not moved.
    assert_eq!(
        first(&cron("0 0 * * * *"), "2026-03-29T00:30:00+01:00", 3),
        [
            "2026-03-29T01:00:00+01:00",
            "2026-03-29T03:00:00+02:00",
            "2026-03-29T04:00:00+02:00"
        ]
    );
    assert_eq!(
        first(&cron("0 30 2 * * *"), "2026-03-28T12:00:00+01:00", 2),
        ["2026-03-30T02:30:00+02:00", "2026-03-31T02:30:00+02:00"]
    );
    // Overlap: a time that occurs twice fires twice.
    assert_eq!(
        first(&cron("0 0 * * * *"), "2026-10-25T00:30:00+02:00", 4),
        [
            "2026-10-25T01:00:00+02:00",
            "2026-10-25T02:00:00+02:00",
            "2026-10-25T02:00:00+01:00",
            "2026-10-25T03:00:00+01:00"
        ]
    );
    assert_eq!(
        first(&cron("0 30 2 * * *"), "2026-10-24T12:00:00+02:00", 3),
        [
            "2026-10-25T02:30:00+02:00",
            "2026-10-25T02:30:00+01:00",
            "2026-10-26T02:30:00+01:00"
        ]
    );
    // Every minute: after the second 02:00 the crate goes back to the first pass of the hour
    // (02:01+02:00); those slots are dropped, so the first pass ends at 02:00.
    assert_eq!(
        first(&cron("0 * * * * *"), "2026-10-25T01:58:00+02:00", 5),
        [
            "2026-10-25T01:59:00+02:00",
            "2026-10-25T02:00:00+02:00",
            "2026-10-25T02:00:00+01:00",
            "2026-10-25T02:01:00+01:00",
            "2026-10-25T02:02:00+01:00"
        ]
    );
}

// --- Newest slot in a window ---

#[test]
fn newest_slot_in_is_the_newest_in_after_exclusive_to_upto_inclusive() {
    let d = daily("09:00");
    assert_eq!(
        newest(&d, "2026-10-10T12:00:00+02:00", "2026-10-14T10:00:00+02:00").as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(&d, "2026-10-10T12:00:00+02:00", "2026-10-14T09:00:00+02:00").as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(&d, "2026-10-14T09:00:00+02:00", "2026-10-14T10:00:00+02:00"),
        None
    );
    assert_eq!(
        newest(&d, "2026-10-14T09:00:00+02:00", "2026-10-15T09:00:00+02:00").as_deref(),
        Some("2026-10-15T09:00:00+02:00")
    );

    let e = every(60, "2026-10-14T08:00:00Z");
    assert_eq!(
        newest(&e, "2026-10-14T10:30:00+02:00", "2026-10-14T14:15:00+02:00").as_deref(),
        Some("2026-10-14T14:00:00+02:00")
    );
    assert_eq!(
        newest(&e, "2026-10-14T14:00:00+02:00", "2026-10-14T14:15:00+02:00"),
        None
    );

    let w = weekly(&[Weekday::Mon, Weekday::Fri], "09:00");
    assert_eq!(
        newest(&w, "2026-10-14T10:00:00+02:00", "2026-10-21T10:00:00+02:00").as_deref(),
        Some("2026-10-19T09:00:00+02:00")
    );

    let o = once("2026-10-20T09:00");
    assert_eq!(
        newest(&o, "2026-10-14T10:00:00+02:00", "2026-10-21T10:00:00+02:00").as_deref(),
        Some("2026-10-20T09:00:00+02:00")
    );
    assert_eq!(
        newest(&o, "2026-10-20T09:00:00+02:00", "2026-10-21T10:00:00+02:00"),
        None
    );
    assert_eq!(
        newest(&o, "2026-10-14T10:00:00+02:00", "2026-10-20T08:59:00+02:00"),
        None
    );

    let c = cron("0 0 9 * * *");
    assert_eq!(
        newest(&c, "2026-10-10T12:00:00+02:00", "2026-10-14T09:00:00+02:00").as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(&c, "2026-10-14T09:00:00+02:00", "2026-10-14T10:00:00+02:00"),
        None
    );
}

#[test]
fn newest_slot_in_applies_the_dst_rules() {
    assert_eq!(
        newest(
            &daily("02:30"),
            "2026-10-25T00:00:00+02:00",
            "2026-10-25T04:00:00+01:00"
        )
        .as_deref(),
        Some("2026-10-25T02:30:00+02:00")
    );
    assert_eq!(
        newest(
            &daily("02:30"),
            "2026-10-25T02:30:00+02:00",
            "2026-10-25T04:00:00+01:00"
        ),
        None,
        "the repeated 02:30 is not a second slot"
    );
    assert_eq!(
        newest(
            &daily("02:30"),
            "2026-03-29T00:00:00+01:00",
            "2026-03-29T12:00:00+02:00"
        )
        .as_deref(),
        Some("2026-03-29T03:30:00+02:00")
    );
}

#[test]
fn newest_slot_in_a_long_window_does_not_walk_every_slot() {
    let started = std::time::Instant::now();
    assert_eq!(
        newest(
            &cron("* * * * * *"),
            "2020-01-01T00:00:00+01:00",
            "2026-10-14T09:00:00+02:00"
        )
        .as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(
            &every(1, "2020-01-01T00:00:30Z"),
            "2020-01-01T00:00:00+01:00",
            "2026-10-14T09:00:00+02:00"
        )
        .as_deref(),
        Some("2026-10-14T08:59:30+02:00")
    );
    assert_eq!(
        newest(
            &daily("09:00"),
            "2020-01-01T00:00:00+01:00",
            "2026-10-14T09:00:00+02:00"
        )
        .as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

// --- Bounds ---

#[test]
fn until_date_is_inclusive_and_resolved_like_a_slot() {
    let s = on(daily("09:00"), Some(Until::Date(local("2026-10-16T09:00"))));
    let next = |after: &str| {
        next_slot(&s, &at(after), &TestTz)
            .unwrap()
            .map(|t| t.to_rfc3339())
    };
    assert_eq!(
        next("2026-10-15T10:00:00+02:00").as_deref(),
        Some("2026-10-16T09:00:00+02:00")
    );
    assert_eq!(next("2026-10-16T09:00:00+02:00"), None);
    assert_eq!(
        due_slot(
            &s,
            &at("2026-10-14T10:00:00+02:00"),
            &at("2026-10-18T10:00:00+02:00"),
            &TestTz
        )
        .unwrap()
        .map(|t| t.to_rfc3339())
        .as_deref(),
        Some("2026-10-16T09:00:00+02:00")
    );

    let overlap = on(daily("09:00"), Some(Until::Date(local("2026-10-25T02:30"))));
    assert_eq!(
        overlap.until_instant(&TestTz).unwrap().to_rfc3339(),
        "2026-10-25T02:30:00+01:00",
        "inclusive: the later of the two instants"
    );
    let gap = on(daily("09:00"), Some(Until::Date(local("2026-03-29T02:30"))));
    assert_eq!(
        gap.until_instant(&TestTz).unwrap().to_rfc3339(),
        "2026-03-29T03:30:00+02:00"
    );
    assert_eq!(on(daily("09:00"), None).until_instant(&TestTz), None);
    assert_eq!(
        on(daily("09:00"), Some(Until::Count(3))).until_instant(&TestTz),
        None
    );
}

#[test]
fn next_slot_without_a_bound_is_the_first_slot() {
    let s = on(once("2026-10-20T09:00"), None);
    let next = |after: &str| next_slot(&s, &at(after), &TestTz).unwrap();
    assert_eq!(
        next("2026-10-14T10:00:00+02:00")
            .map(|t| t.to_rfc3339())
            .as_deref(),
        Some("2026-10-20T09:00:00+02:00")
    );
    assert_eq!(next("2026-10-20T09:00:00+02:00"), None);
}

#[test]
fn upcoming_honours_count_and_date_from_a_fresh_start() {
    let after = at("2026-10-14T10:00:00+02:00");
    let take = |s: &Schedule| rfc(upcoming(s, &after, &TestTz).unwrap().take(5));
    assert_eq!(
        take(&on(daily("09:00"), Some(Until::Count(2)))),
        ["2026-10-15T09:00:00+02:00", "2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(
        take(&on(
            daily("09:00"),
            Some(Until::Date(local("2026-10-17T08:00")))
        )),
        ["2026-10-15T09:00:00+02:00", "2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(take(&on(daily("09:00"), None)).len(), 5);
    assert!(take(&on(once("2026-10-14T09:00"), None)).is_empty());
}

// --- The PC's zone ---

#[test]
fn slots_work_in_the_local_zone() {
    let now = Utc::now();
    for rule in [daily("09:00"), cron("0 0 9 * * *")] {
        let next = slots_after(&rule, &now, &Local).unwrap().next().unwrap();
        assert!(next > now && next <= now + TimeDelta::hours(26), "{next}");
        let prev = newest_slot_in(&rule, &(now - TimeDelta::days(2)), &now, &Local)
            .unwrap()
            .unwrap();
        assert!(prev <= now && prev > now - TimeDelta::hours(26), "{prev}");
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test schedule_test 2>&1 | grep -E "^error|could not compile" | sort | uniq -c
```

Expected: the test target does not compile: `` error[E0432]: unresolved import `dots_core::schedule` `` and ``could not compile `dots-core` (test "schedule_test") due to 1 previous error``.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod runner;
pub mod runtime;
pub mod scheduler;
pub mod server;
```

**with:**

```rust
pub mod runner;
pub mod runtime;
pub mod schedule;
pub mod scheduler;
pub mod server;
```

**Create `crates/dots-core/src/schedule.rs`:**

```rust
//! A dot's schedule (spec "Scheduler v2" §2) and its slots (§3): the instants at which it is
//! due. Pure: every function takes the time and the time zone as arguments, so the scheduler
//! passes `Local` and tests pass a zone of their own.

use chrono::{
    DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeDelta,
    TimeZone, Utc, Weekday,
};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub rule: Rule,
    /// One run for the most recent missed slot when Dots comes back. Default true.
    #[serde(default = "yes")]
    pub catch_up: bool,
    /// None: no bound. Not allowed with `Rule::Once`.
    #[serde(default)]
    pub until: Option<Until>,
}

fn yes() -> bool {
    true
}

/// JSON: `{"kind": "daily", "time": "09:00"}`. Times are `HH:MM` and date-times
/// `YYYY-MM-DDTHH:MM`, both local and without an offset; `anchor` is RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rule {
    /// Once, at a local date and time.
    Once {
        #[serde(with = "local_date_time")]
        at: NaiveDateTime,
    },
    /// Every `minutes` (1 to 10080) of elapsed time, phase counted from `anchor`. The core sets
    /// `anchor` (`Schedule::fill_anchor`); the UI never sends it.
    Every {
        minutes: u32,
        #[serde(default)]
        anchor: Option<DateTime<Utc>>,
    },
    /// Every day at a local time.
    Daily {
        #[serde(with = "local_time")]
        time: NaiveTime,
    },
    /// On the given weekdays (at least one; `"mon"` to `"sun"`) at a local time.
    Weekly {
        #[serde(with = "weekdays")]
        days: Vec<Weekday>,
        #[serde(with = "local_time")]
        time: NaiveTime,
    },
    /// A 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
    Cron { expr: String },
}

/// JSON: `{"count": 10}` or `{"date": "2026-10-20T09:00"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Until {
    /// Stop after this many occurrences (at least 1).
    Count(u32),
    /// Stop after this local date and time, inclusive.
    Date(#[serde(with = "local_date_time")] NaiveDateTime),
}

/// Slots in order, each strictly later than the one before.
pub type Slots<Tz> = Box<dyn Iterator<Item = DateTime<Tz>>>;

pub const MAX_EVERY_MINUTES: u32 = 7 * 24 * 60;

fn invalid<T>(m: &str) -> Result<T> {
    Err(Error::Invalid(m.to_string()))
}

impl Schedule {
    /// The structural rules of spec §2. Times and dates that do not parse are refused earlier,
    /// when the JSON is read.
    pub fn validate(&self) -> Result<()> {
        match &self.rule {
            Rule::Every { minutes, .. } if !(1..=MAX_EVERY_MINUTES).contains(minutes) => {
                return invalid("every: minutes must be 1 to 10080 (7 days)");
            }
            Rule::Weekly { days, .. } if days.is_empty() => {
                return invalid("weekly: pick at least one day");
            }
            Rule::Cron { expr } => {
                crate::scheduler::parse_cron(expr)?;
            }
            _ => {}
        }
        match (&self.rule, &self.until) {
            (Rule::Once { .. }, Some(_)) => invalid("once: a stop (until) is not allowed"),
            (_, Some(Until::Count(0))) => invalid("until: count must be at least 1"),
            _ => Ok(()),
        }
    }

    /// Refuses a schedule with no slot after `now` within its `until` date: a past `Once`, an
    /// `Until::Date` before the first slot, a cron expression whose years are over. Used when a
    /// dot is created or its schedule changes, not when it is loaded. An `Every` without an
    /// anchor is checked as if it were saved now.
    pub fn check_runs_after<A: TimeZone, Tz: TimeZone + 'static>(
        &self,
        now: &DateTime<A>,
        tz: &Tz,
    ) -> Result<()> {
        let mut schedule = self.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        match next_slot(&schedule, now, tz)? {
            Some(_) => Ok(()),
            None => invalid("this schedule would never run"),
        }
    }

    /// Sets `Every.anchor` as the core does on save: a schedule without one keeps the previous
    /// schedule's anchor while `minutes` stay the same, and starts at `now` otherwise. An
    /// anchor sent back unchanged with new minutes starts at `now` too. Other rules are left
    /// as they are.
    pub fn fill_anchor(&mut self, previous: Option<&Schedule>, now: DateTime<Utc>) {
        let Rule::Every { minutes, anchor } = &mut self.rule else {
            return;
        };
        let before = match previous.map(|p| &p.rule) {
            Some(Rule::Every {
                minutes: m,
                anchor: a,
            }) => Some((*m, *a)),
            _ => None,
        };
        *anchor = match (*anchor, before) {
            (None, Some((m, Some(a)))) if m == *minutes => Some(a),
            (None, _) => Some(now),
            (Some(a), Some((m, Some(b)))) if a == b && m != *minutes => Some(now),
            (Some(a), _) => Some(a),
        };
    }

    /// The instant of `Until::Date` in `tz`: a time that occurs twice is its later instant (the
    /// bound is inclusive), a time in a gap moves forward like a slot.
    pub fn until_instant<Tz: TimeZone>(&self, tz: &Tz) -> Option<DateTime<Tz>> {
        match self.until {
            Some(Until::Date(date)) => match tz.from_local_datetime(&date) {
                LocalResult::Ambiguous(_, latest) => Some(latest),
                _ => resolve(tz, date),
            },
            _ => None,
        }
    }
}

/// The slots of `rule` strictly after `after`, in `tz`, in order (spec §3). `Invalid` for a
/// cron expression that does not parse and for an `Every` without an anchor.
pub fn slots_after<A: TimeZone, Tz: TimeZone + 'static>(
    rule: &Rule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Slots<Tz>> {
    let after = after.with_timezone(tz);
    Ok(match rule {
        Rule::Once { at } => Box::new(resolve(tz, *at).filter(|t| *t > after).into_iter()),
        Rule::Every { minutes, anchor } => {
            let (anchor, step) = every_parts(*minutes, *anchor)?;
            let first = slot_index_upto(anchor, step, &after.with_timezone(&Utc)) + 1;
            let tz = tz.clone();
            Box::new(
                (first..)
                    .map_while(move |k| every_slot(anchor, step, k))
                    .map(move |t| t.with_timezone(&tz)),
            )
        }
        Rule::Daily { time } => wall_clock_slots(tz, after, *time, None),
        Rule::Weekly { days, time } => wall_clock_slots(tz, after, *time, Some(days.clone())),
        Rule::Cron { expr } => {
            let mut last = after.clone();
            Box::new(
                crate::scheduler::parse_cron(expr)?
                    .after_owned(after)
                    .filter(move |t| {
                        // The crate can step back in a fall-back overlap; keep slots in order.
                        let later = *t > last;
                        if later {
                            last = t.clone();
                        }
                        later
                    }),
            )
        }
    })
}

/// The newest slot of `rule` with `after < slot <= upto`, in `tz`. It does not walk the
/// window from its start, so a long window (Dots closed for months) is cheap.
pub fn newest_slot_in<A: TimeZone, B: TimeZone, Tz: TimeZone + 'static>(
    rule: &Rule,
    after: &DateTime<A>,
    upto: &DateTime<B>,
    tz: &Tz,
) -> Result<Option<DateTime<Tz>>> {
    let after = after.with_timezone(tz);
    let upto = upto.with_timezone(tz);
    if upto <= after {
        return Ok(None);
    }
    let newest = match rule {
        Rule::Once { at } => resolve(tz, *at).filter(|t| *t <= upto),
        Rule::Every { minutes, anchor } => {
            let (anchor, step) = every_parts(*minutes, *anchor)?;
            let k = slot_index_upto(anchor, step, &upto.with_timezone(&Utc));
            every_slot(anchor, step, k).map(|t| t.with_timezone(tz))
        }
        Rule::Daily { time } => newest_wall_clock(tz, &after, &upto, *time, None),
        Rule::Weekly { days, time } => {
            newest_wall_clock(tz, &after, &upto, *time, Some(days.as_slice()))
        }
        Rule::Cron { expr } => {
            // The crate's backward iterator starts strictly before its start unless the start
            // has a fraction of a second: one nanosecond more makes `upto` itself a candidate.
            let mut back = crate::scheduler::parse_cron(expr)?
                .after_owned(upto.clone() + TimeDelta::nanoseconds(1));
            std::iter::from_fn(|| back.next_back()).find(|t| *t <= upto)
        }
    };
    Ok(newest.filter(|t| *t > after))
}

/// The first slot of `schedule` after `after`, or `None` when none is left within its
/// `Until::Date`. `Until::Count` is the scheduler's (it knows how many slots fired).
pub fn next_slot<A: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Option<DateTime<Tz>>> {
    Ok(upcoming_within_date(schedule, after, tz)?.next())
}

/// `newest_slot_in` for a schedule: the newest slot in `after < slot <= upto` that is within
/// its `Until::Date`.
pub fn due_slot<A: TimeZone, B: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    upto: &DateTime<B>,
    tz: &Tz,
) -> Result<Option<DateTime<Tz>>> {
    let upto = upto.with_timezone(tz);
    let upto = match schedule.until_instant(tz) {
        Some(until) if until < upto => until,
        _ => upto,
    };
    newest_slot_in(&schedule.rule, after, &upto, tz)
}

/// The slots of a schedule that starts after `after` (a preview): within its `Until::Date`
/// and at most `Until::Count` of them.
pub fn upcoming<A: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Slots<Tz>> {
    let slots = upcoming_within_date(schedule, after, tz)?;
    Ok(match schedule.until {
        Some(Until::Count(n)) => Box::new(slots.take(n as usize)),
        _ => slots,
    })
}

fn upcoming_within_date<A: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Slots<Tz>> {
    let slots = slots_after(&schedule.rule, after, tz)?;
    Ok(match schedule.until_instant(tz) {
        Some(until) => Box::new(slots.take_while(move |t| *t <= until)),
        None => slots,
    })
}

/// A local date and time as an instant in `tz`, with the DST rules of spec §3: a time that
/// occurs twice is its earlier instant; a time in a gap moves forward by the length of the gap
/// (02:30 on a night when clocks go from 02:00 to 03:00 becomes 03:30).
fn resolve<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => {
            // Read the time with the offset from before the gap.
            let before = tz
                .from_local_datetime(&(local - TimeDelta::days(1)))
                .earliest()?;
            let offset = TimeDelta::seconds(before.offset().fix().local_minus_utc() as i64);
            Some(tz.from_utc_datetime(&(local - offset)))
        }
    }
}

fn every_parts(minutes: u32, anchor: Option<DateTime<Utc>>) -> Result<(DateTime<Utc>, TimeDelta)> {
    let Some(anchor) = anchor else {
        return invalid("every: the schedule has no anchor yet");
    };
    if minutes == 0 {
        return invalid("every: minutes must be 1 to 10080 (7 days)");
    }
    Ok((anchor, TimeDelta::minutes(minutes as i64)))
}

fn every_slot(anchor: DateTime<Utc>, step: TimeDelta, k: i64) -> Option<DateTime<Utc>> {
    let ms = step.num_milliseconds().checked_mul(k)?;
    anchor.checked_add_signed(TimeDelta::try_milliseconds(ms)?)
}

/// The largest `k` with `anchor + k * step <= t`.
fn slot_index_upto(anchor: DateTime<Utc>, step: TimeDelta, t: &DateTime<Utc>) -> i64 {
    let mut k = (*t - anchor)
        .num_milliseconds()
        .div_euclid(step.num_milliseconds());
    // Milliseconds drop the nanoseconds: settle the last step exactly.
    while every_slot(anchor, step, k).is_some_and(|s| s > *t) {
        k -= 1;
    }
    while every_slot(anchor, step, k + 1).is_some_and(|s| s <= *t) {
        k += 1;
    }
    k
}

fn on_day(days: Option<&[Weekday]>, date: NaiveDate) -> bool {
    days.is_none_or(|d| d.contains(&date.weekday()))
}

fn wall_clock_slots<Tz: TimeZone + 'static>(
    tz: &Tz,
    after: DateTime<Tz>,
    time: NaiveTime,
    days: Option<Vec<Weekday>>,
) -> Slots<Tz> {
    if days.as_ref().is_some_and(|d| d.is_empty()) {
        return Box::new(std::iter::empty());
    }
    // From the day before: a time in a gap can move past midnight.
    let start = after
        .naive_local()
        .date()
        .pred_opt()
        .unwrap_or(NaiveDate::MIN);
    let tz = tz.clone();
    Box::new(
        start
            .iter_days()
            .filter(move |d| on_day(days.as_deref(), *d))
            .filter_map(move |d| resolve(&tz, d.and_time(time)))
            .filter(move |t| *t > after),
    )
}

fn newest_wall_clock<Tz: TimeZone>(
    tz: &Tz,
    after: &DateTime<Tz>,
    upto: &DateTime<Tz>,
    time: NaiveTime,
    days: Option<&[Weekday]>,
) -> Option<DateTime<Tz>> {
    // From the day after `upto` back to the day before `after`, to cover a moved time.
    let first = after.naive_local().date().pred_opt()?;
    let last = upto.naive_local().date().succ_opt()?;
    std::iter::successors(Some(last), |d| d.pred_opt())
        .take_while(|d| *d >= first)
        .filter(|d| on_day(days, *d))
        .filter_map(|d| resolve(tz, d.and_time(time)))
        .find(|t| t <= upto)
}

mod local_time {
    use chrono::NaiveTime;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&t.format("%H:%M"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let s = String::deserialize(d)?;
        NaiveTime::parse_from_str(&s, "%H:%M")
            .map_err(|_| D::Error::custom(format!("time {s:?}: expected HH:MM")))
    }
}

mod local_date_time {
    use chrono::NaiveDateTime;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&t.format("%Y-%m-%dT%H:%M"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
        let s = String::deserialize(d)?;
        NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M")
            .map_err(|_| D::Error::custom(format!("date {s:?}: expected YYYY-MM-DDTHH:MM")))
    }
}

mod weekdays {
    use chrono::Weekday;
    use serde::de::Error as _;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    const NAMES: [(&str, Weekday); 7] = [
        ("mon", Weekday::Mon),
        ("tue", Weekday::Tue),
        ("wed", Weekday::Wed),
        ("thu", Weekday::Thu),
        ("fri", Weekday::Fri),
        ("sat", Weekday::Sat),
        ("sun", Weekday::Sun),
    ];

    pub fn serialize<S: Serializer>(days: &[Weekday], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(days.len()))?;
        for day in days {
            seq.serialize_element(NAMES[day.num_days_from_monday() as usize].0)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Weekday>, D::Error> {
        Vec::<String>::deserialize(d)?
            .into_iter()
            .map(|s| {
                NAMES
                    .iter()
                    .find(|(name, _)| *name == s)
                    .map(|(_, day)| *day)
                    .ok_or_else(|| D::Error::custom(format!("day {s:?}: expected mon to sun")))
            })
            .collect()
    }
}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  crates/dots-core/          library: all logic, no UI dependency
    src/store/               SQLite via sqlx, embedded migrations
    src/scheduler/           tokio-cron-scheduler -> enqueue runs
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
```

**with:**

```markdown
  crates/dots-core/          library: all logic, no UI dependency
    src/store/               SQLite via sqlx, embedded migrations
    src/schedule.rs          schedules (once, every, daily, weekly, cron) and their slots in local time
    src/scheduler/           tokio-cron-scheduler -> enqueue runs
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
## 9. Testing

- **Unit (dots-core):** policy resolution and pattern matching; cron parsing / next fire; run state
  transitions; path confinement including `..`, absolute paths, symlinks, junctions; prompt
  assembly.
```

**with:**

```markdown
## 9. Testing

- **Unit (dots-core):** policy resolution and pattern matching; schedule slots, with DST checked in a
  hand-written test time zone; run state
  transitions; path confinement including `..`, absolute paths, symlinks, junctions; prompt
  assembly.
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p dots-core --test schedule_test 2>&1 | grep -E "^test result|FAILED"
```

Expected: 25 passed.

- [ ] **Step 5: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

Expected: `dots-core` 365 passed, `dots-app` 47 passed, no clippy line, `cargo fmt --check` silent.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/lib.rs crates/dots-core/src/schedule.rs crates/dots-core/tests/common/mod.rs crates/dots-core/tests/common/tz.rs crates/dots-core/tests/schedule_test.rs docs/superpowers/specs/2026-10-08-dots-design.md
git commit -m "feat(core): schedules and their slots, with written DST rules"
```

### Task 2: A dot's schedule is a schedule object, not a cron string

`DotSpec.schedule` becomes `Option<Schedule>`, end to end. Migration 0005 holds the whole schema of spec section 4 at once: it turns every stored cron string into a cron schedule with catch-up off, so that nothing changes on upgrade, and it adds `runs.retry_of` and the `schedule_state` table, which later tasks fill. The store fills the anchor of an `Every` and refuses a schedule that would never run when a dot is created or its schedule changes; a stored schedule whose date has passed still loads and saves (spec section 2). `preview_schedule` takes the object, and the overview's next run comes from the new slot functions. The scheduler loop keeps today's behaviour (missed slots skipped, coalescing, the last fire time in memory) but takes its slots from the new module; Task 4 replaces it. In the UI the types change, and the editor's cron field maps to a cron schedule; a schedule of another kind is shown in words, kept on save, and can be removed, until Task 7 gives the editor its builder. The main spec's sections 1, 3 and the `preview_schedule` command follow.

**Files:**
- Create: `crates/dots-core/migrations/0005_scheduler_v2.sql`
- Modify: `apps/dots-app/src/commands.rs`, `apps/dots-app/src/contract.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/schedule.rs`, `crates/dots-core/src/scheduler.rs`, `crates/dots-core/src/store/dots.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/commands.ts`, `ui/src/api/types.ts`, `ui/src/lib/dot-rules.ts`, `ui/src/lib/query.ts`, `ui/src/screens/DotEditorScreen.tsx`, `ui/src/screens/dot-editor/DotForm.tsx`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/runtime_review_test.rs`, `crates/dots-core/tests/runtime_status_test.rs`, `crates/dots-core/tests/scheduler_test.rs`, `crates/dots-core/tests/store_dots_test.rs`, `ui/src/api/contract.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`

**Interfaces:**
- Consumes: Task 1's `Schedule`, `fill_anchor`, `check_runs_after`, `next_slot`, `upcoming`.
- Produces:
  - Migration `0005_scheduler_v2.sql`: `dots.schedule` rewritten to `{"rule":{"kind":"cron","expr":<expr>},"catch_up":false,"until":null}`; `runs.retry_of TEXT REFERENCES runs(id) ON DELETE SET NULL`; table `schedule_state` (`dot_id` primary key and foreign key to `dots` `ON DELETE CASCADE`; `rule_key`, `status`, `last_slot_at`, `updated_at` text not null; `fired_count` integer not null default 0; `watch_run_id`, `attention_run_id` foreign keys to `runs(id)` `ON DELETE SET NULL`; `retry_slot_at`, `retry_at` text; `retry_attempt` integer). Nothing writes the table yet.
  - `Schedule::cron(expr: &str) -> Schedule` (catch-up off, no bound): the test helper that replaces `"…".into()`.
  - `Schedule::prepare_save(&mut self, previous: Option<&Schedule>, now: DateTime<Utc>, tz) -> Result<()>`: `fill_anchor(previous, now)`, then `check_runs_after(now, tz)` unless the schedule equals `previous`.
  - `DotSpec.schedule: Option<Schedule>`; `DotSpec::validate` calls `Schedule::validate`. The store reads and writes the column as JSON; `create_dot` and `update_dot` (which first reads the stored schedule as `previous`) call `prepare_save` with `Utc::now()` to the millisecond and `Local`.
  - `Runtime::preview_schedule(schedule: &Schedule, count: usize) -> Result<Vec<String>>`: `validate`, `fill_anchor(None, now)`, then up to `count.clamp(1, 10)` slots of `upcoming`, RFC 3339 local; `[]` when no slot is left (Task 7 changes that).
  - `dot_overview`'s `next_fire` is `next_slot(schedule, now, Local)`. `scheduler::next_fire` is removed.
  - Command `preview_schedule(schedule: Schedule, count: usize)`. Contract: the spec sample's schedule is `Schedule::cron("0 0 2 * * *")`; a new `Schedule` array of six samples (once; every with an anchor and a count; every without an anchor; daily until a date; weekly; cron).
  - UI: `T.Weekday`, `T.ScheduleRule` (discriminated union on `kind`), `T.Until`, `T.Schedule` and their types (`Rule` is the policy's rule, hence `ScheduleRule`); `DotSpec.schedule: Schedule | null`; `commands.previewSchedule(schedule, count)`; `keys.schedulePreview(schedule | null)`; in `@/lib/dot-rules`, `cronSchedule(text)` and `scheduleOf(schedule)` (trims a cron expression; only spaces give `null`).
  - Editor: the cron field reads and writes a cron schedule; a schedule of another kind is described in words ("Weekly on mon, fri at 09:00, at most 10 runs") with the hint "This editor cannot change this kind of schedule yet. Saving keeps it as it is." and a button "Remove schedule". The core's objection from the preview shows in place of the hint.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
use common::{init_repo, spec, wait_status};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use llm::{call, text, Llm};
```

**with:**

```rust
use common::{init_repo, spec, wait_status};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::{Config, Error, Runtime};
use llm::{call, text, Llm};
```

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, repo, rt }
}

```

**with:**

```rust
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, repo, rt }
}

fn schedule(v: serde_json::Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

```

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
    let t = start().await;
    let mut s = folder_spec(&t, "sched");
    s.schedule = Some("0 0 9 * * *".into());
    let dot = t.rt.create_dot(s).await.unwrap();
    let idle = t.rt.create_dot(folder_spec(&t, "idle")).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
```

**with:**

```rust
    let t = start().await;
    let mut s = folder_spec(&t, "sched");
    s.schedule = Some(Schedule::cron("0 0 9 * * *"));
    let dot = t.rt.create_dot(s).await.unwrap();
    let mut s = folder_spec(&t, "daily");
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "daily", "time": "07:15"}}),
    ));
    let daily = t.rt.create_dot(s).await.unwrap();
    let idle = t.rt.create_dot(folder_spec(&t, "idle")).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
```

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
    let next = row.next_fire.as_deref().expect("next fire");
    assert!(next.contains("T09:00:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == idle.id).unwrap();
    assert!(row.last_run.is_none() && row.next_fire.is_none());
```

**with:**

```rust
    let next = row.next_fire.as_deref().expect("next fire");
    assert!(next.contains("T09:00:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == daily.id).unwrap();
    let next = row.next_fire.as_deref().expect("next fire");
    assert!(next.contains("T07:15:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == idle.id).unwrap();
    assert!(row.last_run.is_none() && row.next_fire.is_none());
```

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
#[test]
fn preview_schedule_lists_the_next_fire_times() {
    let times = Runtime::preview_schedule("0 30 9 * * Mon", 3).unwrap();
    assert_eq!(times.len(), 3);
    assert!(times.iter().all(|t| t.contains("T09:30:00")), "{times:?}");
    assert!(times[0] < times[1] && times[1] < times[2]);
    assert_eq!(
        Runtime::preview_schedule("0 0 * * * *", 50).unwrap().len(),
        10
    );
    assert!(matches!(
        Runtime::preview_schedule("every day", 3),
        Err(Error::Invalid(_))
    ));
}

```

**with:**

```rust
#[test]
fn preview_schedule_lists_the_next_fire_times() {
    let preview = |v: serde_json::Value, n| Runtime::preview_schedule(&schedule(v), n);
    let times = Runtime::preview_schedule(&Schedule::cron("0 30 9 * * Mon"), 3).unwrap();
    assert_eq!(times.len(), 3);
    assert!(times.iter().all(|t| t.contains("T09:30:00")), "{times:?}");
    assert!(times[0] < times[1] && times[1] < times[2]);
    assert_eq!(
        Runtime::preview_schedule(&Schedule::cron("0 0 * * * *"), 50)
            .unwrap()
            .len(),
        10
    );
    assert!(matches!(
        Runtime::preview_schedule(&Schedule::cron("every day"), 3),
        Err(Error::Invalid(_))
    ));

    // The builder's kinds, within their bounds.
    let daily = json!({"rule": {"kind": "daily", "time": "07:15"}, "until": {"count": 2}});
    let times = preview(daily, 3).unwrap();
    assert_eq!(times.len(), 2, "{times:?}");
    assert!(times.iter().all(|t| t.contains("T07:15:00")), "{times:?}");
    // An every schedule as the UI sends it, without an anchor, starts now.
    let before = chrono::Local::now();
    let times = preview(json!({"rule": {"kind": "every", "minutes": 90}}), 2).unwrap();
    let first = chrono::DateTime::parse_from_rfc3339(&times[0]).unwrap();
    let gap = first.with_timezone(&chrono::Local) - before;
    assert!(
        gap > chrono::TimeDelta::minutes(89) && gap <= chrono::TimeDelta::minutes(91),
        "{times:?}"
    );
    // Refused like `DotSpec::validate`; empty when no slot is left.
    let no_day = json!({"rule": {"kind": "weekly", "days": [], "time": "09:00"}});
    assert!(matches!(preview(no_day, 3), Err(Error::Invalid(_))));
    let past = json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}});
    assert_eq!(preview(past, 3).unwrap(), Vec::<String>::new());
}

```

**In `crates/dots-core/tests/runtime_status_test.rs`, replace:**

```rust
use common::{spec, wait_status};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::settings::{Settings, TESTED_CLAUDE_VERSION};
use dots_core::{Config, Error, Runtime};
```

**with:**

```rust
use common::{spec, wait_status};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::settings::{Settings, TESTED_CLAUDE_VERSION};
use dots_core::{Config, Error, Runtime};
```

**In `crates/dots-core/tests/runtime_status_test.rs`, replace:**

```rust
    assert!(rt.settings().await.schedules_paused);
    let mut s = folder_spec(&dir, "ticker");
    s.schedule = Some("* * * * * *".into());
    let dot = rt.create_dot(s).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;
```

**with:**

```rust
    assert!(rt.settings().await.schedules_paused);
    let mut s = folder_spec(&dir, "ticker");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = rt.create_dot(s).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
use dots_core::model::{EngineKind, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
```

**with:**

```rust
use dots_core::model::{EngineKind, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    let (dir, store) = temp_store().await;
    let mut s = spec("tick");
    s.schedule = Some("* * * * * *".into());
    let dot = store.create_dot(&s).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
```

**with:**

```rust
    let (dir, store) = temp_store().await;
    let mut s = spec("tick");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = store.create_dot(&s).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    let (dir, store) = temp_store().await;
    let mut off = spec("off");
    off.schedule = Some("* * * * * *".into());
    off.enabled = false;
    store.create_dot(&off).await.unwrap();
    store.create_dot(&spec("manual-only")).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
```

**with:**

```rust
    let (dir, store) = temp_store().await;
    let mut off = spec("off");
    off.schedule = Some(Schedule::cron("* * * * * *"));
    off.enabled = false;
    store.create_dot(&off).await.unwrap();
    store.create_dot(&spec("manual-only")).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

/// The loop takes its slots from the schedule module, so a stop date is kept.
#[tokio::test]
async fn a_schedule_past_its_stop_date_fires_nothing() {
    let (dir, store) = temp_store().await;
    let mut s = spec("stopped");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = store.create_dot(&s).await.unwrap();
    // The stop date passed (written behind the store's back: a past date is refused on save).
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE dots SET schedule = ? WHERE id = ?")
        .bind(
            r#"{"rule":{"kind":"cron","expr":"* * * * * *"},"catch_up":false,
               "until":{"date":"2020-01-01T09:00"}}"#,
        )
        .bind(&dot.id)
        .execute(&pool)
        .await
        .unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut s = spec("late");
    s.schedule = Some("* * * * * *".into());
    let dot = store.create_dot(&s).await.unwrap();
    sched.reload();
```

**with:**

```rust
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut s = spec("late");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = store.create_dot(&s).await.unwrap();
    sched.reload();
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
mod common;

use chrono::{Local, TimeZone};
use common::{spec, temp_store};
use dots_core::model::EngineKind;
use dots_core::policy::Preset;
use dots_core::scheduler::{next_fire, parse_cron};
use dots_core::Error;
use serde_json::json;

#[tokio::test]
```

**with:**

```rust
mod common;

use chrono::{Local, TimeDelta, TimeZone, Utc};
use common::{spec, temp_store};
use dots_core::model::{Dot, EngineKind};
use dots_core::policy::{Policy, Preset};
use dots_core::schedule::{next_slot, Rule, Schedule};
use dots_core::scheduler::parse_cron;
use dots_core::store::Store;
use dots_core::Error;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

fn schedule(v: Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

async fn raw_pool(dir: &tempfile::TempDir) -> SqlitePool {
    SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap()
}

fn never_runs(e: &Error) -> bool {
    matches!(e, Error::Invalid(m) if m == "this schedule would never run")
}

#[tokio::test]
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    let (_d, store) = temp_store().await;
    let mut s = spec("triage");
    s.schedule = Some("0 0 8 * * Mon-Fri".into());
    s.mcp_servers = Some(json!({"sf": {"type": "stdio", "command": "sf-mcp"}}));
    let dot = store.create_dot(&s).await.unwrap();
```

**with:**

```rust
    let (_d, store) = temp_store().await;
    let mut s = spec("triage");
    s.schedule = Some(Schedule::cron("0 0 8 * * Mon-Fri"));
    s.mcp_servers = Some(json!({"sf": {"type": "stdio", "command": "sf-mcp"}}));
    let dot = store.create_dot(&s).await.unwrap();
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    local_no_endpoint.engine = EngineKind::Local;
    let mut five_field_cron = spec("cron5");
    five_field_cron.schedule = Some("0 8 * * *".into());
    let mut reserved_mcp = spec("mcp");
    reserved_mcp.mcp_servers = Some(json!({"dots": {}}));
```

**with:**

```rust
    local_no_endpoint.engine = EngineKind::Local;
    let mut five_field_cron = spec("cron5");
    five_field_cron.schedule = Some(Schedule::cron("0 8 * * *"));
    let mut every_zero = spec("every0");
    every_zero.schedule = Some(schedule(json!({"rule": {"kind": "every", "minutes": 0}})));
    let mut weekly_no_day = spec("weekly");
    weekly_no_day.schedule = Some(schedule(
        json!({"rule": {"kind": "weekly", "days": [], "time": "09:00"}}),
    ));
    let mut once_bounded = spec("once");
    once_bounded.schedule = Some(schedule(json!({
        "rule": {"kind": "once", "at": "2100-01-01T09:00"},
        "until": {"count": 2}
    })));
    let mut count_zero = spec("count0");
    count_zero.schedule = Some(schedule(json!({
        "rule": {"kind": "daily", "time": "09:00"},
        "until": {"count": 0}
    })));
    let mut reserved_mcp = spec("mcp");
    reserved_mcp.mcp_servers = Some(json!({"dots": {}}));
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
        local_no_host,
        five_field_cron,
        reserved_mcp,
        zero_turns,
```

**with:**

```rust
        local_no_host,
        five_field_cron,
        every_zero,
        weekly_no_day,
        once_bounded,
        count_zero,
        reserved_mcp,
        zero_turns,
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust

#[test]
fn next_fire_is_strictly_after() {
    let at = Local.with_ymd_and_hms(2026, 10, 8, 7, 0, 0).unwrap();
    let next = next_fire("0 0 8 * * *", at).unwrap().unwrap();
    assert_eq!(next, Local.with_ymd_and_hms(2026, 10, 8, 8, 0, 0).unwrap());
    let again = next_fire("0 0 8 * * *", next).unwrap().unwrap();
    assert_eq!(again, Local.with_ymd_and_hms(2026, 10, 9, 8, 0, 0).unwrap());
}

```

**with:**

```rust

#[test]
fn the_next_cron_slot_is_strictly_after() {
    let cron = Schedule::cron("0 0 8 * * *");
    let at = Local.with_ymd_and_hms(2026, 10, 8, 7, 0, 0).unwrap();
    let next = next_slot(&cron, &at, &Local).unwrap().unwrap();
    assert_eq!(next, Local.with_ymd_and_hms(2026, 10, 8, 8, 0, 0).unwrap());
    let again = next_slot(&cron, &next, &Local).unwrap().unwrap();
    assert_eq!(again, Local.with_ymd_and_hms(2026, 10, 9, 8, 0, 0).unwrap());
}

#[tokio::test]
async fn a_schedule_that_would_never_run_is_refused_on_create_and_on_change() {
    let (_d, store) = temp_store().await;
    for (name, past) in [
        (
            "once",
            json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}),
        ),
        (
            "until",
            json!({
                "rule": {"kind": "daily", "time": "09:00"},
                "until": {"date": "2020-01-01T09:00"}
            }),
        ),
    ] {
        let mut s = spec(name);
        s.schedule = Some(schedule(past));
        let err = store.create_dot(&s).await.unwrap_err();
        assert!(never_runs(&err), "{name}: {err:?}");
    }
    assert!(store.list_dots().await.unwrap().is_empty());

    let mut s = spec("later");
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "once", "at": "2100-01-01T09:00"}}),
    ));
    let dot = store.create_dot(&s).await.unwrap();
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}),
    ));
    let err = store.update_dot(&dot.id, &s).await.unwrap_err();
    assert!(never_runs(&err), "{err:?}");
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);
}

#[tokio::test]
async fn a_stored_schedule_whose_date_passed_loads_and_saves_unchanged() {
    let (dir, store) = temp_store().await;
    let mut s = spec("once");
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "once", "at": "2100-01-01T09:00"}}),
    ));
    let dot = store.create_dot(&s).await.unwrap();
    // The date passes (written behind the store's back, as time would do it).
    sqlx::query("UPDATE dots SET schedule = ? WHERE id = ?")
        .bind(r#"{"rule":{"kind":"once","at":"2020-01-01T09:00"},"catch_up":true,"until":null}"#)
        .bind(&dot.id)
        .execute(&raw_pool(&dir).await)
        .await
        .unwrap();
    let loaded = store.get_dot(&dot.id).await.unwrap();
    let past = schedule(json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}));
    assert_eq!(loaded.spec.schedule.as_ref(), Some(&past));

    // A change to something else saves: the schedule did not change, so it is not checked.
    let mut s = loaded.spec.clone();
    s.instructions = "Something else.".into();
    let saved = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(saved.spec.instructions, "Something else.");
    assert_eq!(saved.spec.schedule, Some(past));
}

#[tokio::test]
async fn the_core_sets_the_anchor_of_an_every_schedule() {
    fn anchor(dot: &Dot) -> chrono::DateTime<Utc> {
        match &dot.spec.schedule.as_ref().unwrap().rule {
            Rule::Every {
                anchor: Some(a), ..
            } => *a,
            other => panic!("no anchor: {other:?}"),
        }
    }
    let (_d, store) = temp_store().await;
    let mut s = spec("every");
    // As the UI sends it: no anchor.
    s.schedule = Some(schedule(json!({"rule": {"kind": "every", "minutes": 30}})));
    let before = Utc::now();
    let dot = store.create_dot(&s).await.unwrap();
    let first = anchor(&dot);
    assert!(first >= before - TimeDelta::milliseconds(1) && first <= Utc::now());
    assert_eq!(first.timestamp_subsec_nanos() % 1_000_000, 0, "whole ms");
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);

    // Saved again without an anchor: the phase stays.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let saved = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(anchor(&saved), first);

    // New minutes: a new phase.
    s.schedule = Some(schedule(json!({"rule": {"kind": "every", "minutes": 60}})));
    let changed = store.update_dot(&dot.id, &s).await.unwrap();
    assert!(anchor(&changed) > first);
}

/// Migration 0005 on a database that Dots left with a cron string (Scheduler v2 spec §4).
#[tokio::test]
async fn migration_0005_turns_a_cron_string_into_a_schedule_without_catch_up() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 5)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    let policy = serde_json::to_string(&Policy::preset(Preset::Sandboxed)).unwrap();
    for (id, name, schedule) in [
        ("D1", "cron", Some("0 0 9 * * Mon-Fri")),
        ("D2", "manual", None),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             schedule, webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', ?, 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(schedule)
        .bind(&policy)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let cron = store.get_dot("D1").await.unwrap().spec.schedule.unwrap();
    assert_eq!(cron, Schedule::cron("0 0 9 * * Mon-Fri"));
    assert!(!cron.catch_up);
    assert_eq!(store.get_dot("D2").await.unwrap().spec.schedule, None);

    let pool = raw_pool(&dir).await;
    let raw: String = sqlx::query_scalar("SELECT schedule FROM dots WHERE id = 'D1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        raw,
        r#"{"rule":{"kind":"cron","expr":"0 0 9 * * Mon-Fri"},"catch_up":false,"until":null}"#
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schedule_state")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "schedule_state is empty until the first pass or save"
    );
    let retries: Vec<Option<String>> = sqlx::query_scalar("SELECT retry_of FROM runs")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(retries.is_empty());
}

```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
const schemas: Record<SampleName, { parse: (v: unknown) => unknown }> = {
  DotSpec: T.DotSpec,
  NewDotSpec: T.DotSpec,
  Dot: T.Dot,
```

**with:**

```ts
const schemas: Record<SampleName, { parse: (v: unknown) => unknown }> = {
  DotSpec: T.DotSpec,
  Schedule: T.Schedule.array(),
  NewDotSpec: T.DotSpec,
  Dot: T.Dot,
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  });

  it("mirrors the core's limits", () => {
    expect(T.LIMITS).toEqual(fixture.Limits);
```

**with:**

```ts
  });

  it("has a sample of every schedule kind and bound", () => {
    const kinds = T.ScheduleRule.options.map((o) => o.shape.kind.value);
    expect([...new Set(fixture.Schedule.map((s) => s.rule.kind))].sort()).toEqual(
      [...kinds].sort(),
    );
    expect(fixture.Schedule.map((s) => s.until && Object.keys(s.until)[0])).toEqual(
      expect.arrayContaining([null, "count", "date"]),
    );
  });

  it("mirrors the core's limits", () => {
    expect(T.LIMITS).toEqual(fixture.Limits);
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
import { describe, expect, it } from "vitest";

import type { Inbox } from "@/api/types";
import { specOf } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
```

**with:**

```tsx
import { describe, expect, it } from "vitest";

import type { Inbox, Schedule } from "@/api/types";
import { specOf } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
const dot = sample.dot;
const path = `/dots/${dot.id}`;

describe("the dot editor, new dot", () => {
```

**with:**

```tsx
const dot = sample.dot;
const path = `/dots/${dot.id}`;

/// What the cron field saves.
const cron = (expr: string) =>
  ({ rule: { kind: "cron", expr }, catch_up: false, until: null }) satisfies Schedule;

describe("the dot editor, new dot", () => {
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
      ["2026-10-13", "2026-10-14", "2026-10-15"].map((d) => localDateTime(`${d}T02:00:00+01:00`)),
    );
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({ expr: "0 0 2 * * *", count: 3 });

    backend.on("preview_schedule", () => {
```

**with:**

```tsx
      ["2026-10-13", "2026-10-14", "2026-10-15"].map((d) => localDateTime(`${d}T02:00:00+01:00`)),
    );
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({
      schedule: cron("0 0 2 * * *"),
      count: 3,
    });

    backend.on("preview_schedule", () => {
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(backend.callsTo("create_dot")).toEqual([]);
  });

```

**with:**

```tsx
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(backend.callsTo("create_dot")).toEqual([]);
  });

  it("saves the cron expression as a cron schedule without catch-up", async () => {
    backend.on("create_dot", (args) => ({ ...dot, ...(args.spec as object), id: "dot-new" }));
    backend.on("get_dot", () => ({ ...dot, id: "dot-new", name: "nightly" }));
    const { user, router } = renderApp("/dots/new");
    await user.type(await screen.findByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
    await user.type(screen.getByLabelText("Cron expression"), " 0 0 2 * * * ");
    await user.click(screen.getByRole("button", { name: "Create dot" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      {
        spec: {
          ...blankSpec,
          name: "nightly",
          instructions: "Run the tests.",
          workdir: "C:/dev/app",
          schedule: cron("0 0 2 * * *"),
        },
      },
    ]);
  });

```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    expect(await screen.findByRole("heading", { name: dot.name })).toBeVisible();
    expect(screen.getByLabelText("Name")).toHaveValue(dot.name);
    expect(screen.getByLabelText("Cron expression")).toHaveValue(dot.schedule);
    expect(screen.getByLabelText("MCP servers")).toHaveValue(
      JSON.stringify(dot.mcp_servers, null, 2),
```

**with:**

```tsx
    expect(await screen.findByRole("heading", { name: dot.name })).toBeVisible();
    expect(screen.getByLabelText("Name")).toHaveValue(dot.name);
    expect(dot.schedule?.rule.kind).toBe("cron");
    expect(screen.getByLabelText("Cron expression")).toHaveValue(cron("0 0 2 * * *").rule.expr);
    expect(screen.getByLabelText("MCP servers")).toHaveValue(
      JSON.stringify(dot.mcp_servers, null, 2),
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    await user.click(await screen.findByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("only allowed as a trailing");
  });

```

**with:**

```tsx
    await user.click(await screen.findByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("only allowed as a trailing");
  });

  it("clears the schedule when the cron expression is emptied", async () => {
    stored();
    const { user } = renderApp(path);
    await user.clear(await screen.findByLabelText("Cron expression"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(dot), schedule: null } },
    ]);
  });

  it("shows a schedule it cannot edit yet and keeps it on save", async () => {
    const daily: Schedule = {
      rule: { kind: "weekly", days: ["mon", "fri"], time: "09:00" },
      catch_up: true,
      until: { count: 10 },
    };
    const held = stored();
    held.dot = { ...dot, schedule: daily };
    const { user } = renderApp(path);
    expect(await screen.findByText("Weekly on mon, fri at 09:00, at most 10 runs")).toBeVisible();
    expect(screen.queryByLabelText("Cron expression")).toBeNull();
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({ schedule: daily, count: 3 });

    const instructions = screen.getByLabelText("Instructions");
    await user.clear(instructions);
    await user.type(instructions, "Only summarize.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(held.dot), instructions: "Only summarize." } },
    ]);

    // Removed, the cron field is back.
    await user.click(screen.getByRole("button", { name: "Remove schedule" }));
    expect(screen.getByLabelText("Cron expression")).toHaveValue("");
  });

```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test runtime_review_test --test runtime_status_test --test scheduler_test --test store_dots_test 2>&1 | grep -E "^error|could not compile" | sort | uniq -c | sort -rn
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/contract.test.ts src/screens/DotEditorScreen.test.tsx 2>&1 | grep -E "Tests |Test Files|× ")
```

Expected: the four Rust test targets do not compile. The decisive lines: 14 × `error[E0308]: mismatched types`, 13 × `` error[E0599]: no function or associated item named `cron` found for struct `dots_core::schedule::Schedule` ``, `` no field `rule` on type `&std::string::String` ``, `` no field `catch_up` on type `std::string::String` ``, and `could not compile` for `store_dots_test`, `scheduler_test`, `runtime_status_test` and `runtime_review_test`. The UI: `Test Files  2 failed (2)`, `Tests  4 failed | 19 passed (23)`; `contract.test.ts` fails as a file, and in `DotEditorScreen.test.tsx` "previews the next runs of a schedule and shows the core's objection to a bad one", "saves the cron expression as a cron schedule without catch-up", "loads the dot and saves a change" and "shows a schedule it cannot edit yet and keeps it on save" fail.

- [ ] **Step 3: Implement**

The core first, then the app, then the UI files: `types.ts` is checked against the fixture that the next step regenerates.

**Create `crates/dots-core/migrations/0005_scheduler_v2.sql`:**

```sql
-- Scheduler v2 (spec 2026-10-14-scheduler-v2-design.md, section 4).

-- `dots.schedule` held a cron expression; it now holds a schedule as JSON. A converted schedule
-- does not catch up, so that it behaves as before the upgrade (new schedules default to true).
UPDATE dots
SET schedule = json_object(
    'rule', json_object('kind', 'cron', 'expr', schedule),
    'catch_up', json('false'),
    'until', NULL
)
WHERE schedule IS NOT NULL;

-- The failed run that a retry repeats.
ALTER TABLE runs ADD COLUMN retry_of TEXT REFERENCES runs(id) ON DELETE SET NULL;

-- One row per dot with a schedule: where the schedule stands. A deleted run clears the
-- columns that point at it: a watched run that is gone counts as done (spec section 10).
CREATE TABLE schedule_state (
    dot_id           TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE,
    rule_key         TEXT NOT NULL,
    status           TEXT NOT NULL,
    last_slot_at     TEXT NOT NULL,
    fired_count      INTEGER NOT NULL DEFAULT 0,
    watch_run_id     TEXT REFERENCES runs(id) ON DELETE SET NULL,
    retry_slot_at    TEXT,
    retry_attempt    INTEGER,
    retry_at         TEXT,
    attention_run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    updated_at       TEXT NOT NULL
);
```

**In `crates/dots-core/src/schedule.rs`, replace:**

```rust

impl Schedule {
    /// The structural rules of spec §2. Times and dates that do not parse are refused earlier,
    /// when the JSON is read.
```

**with:**

```rust

impl Schedule {
    /// A cron schedule as Dots had them before Scheduler v2: no catch-up, no bound. Migration
    /// 0005 converts a stored cron string to this.
    pub fn cron(expr: &str) -> Schedule {
        Schedule {
            rule: Rule::Cron {
                expr: expr.to_string(),
            },
            catch_up: false,
            until: None,
        }
    }

    /// The structural rules of spec §2. Times and dates that do not parse are refused earlier,
    /// when the JSON is read.
```

**In `crates/dots-core/src/schedule.rs`, replace:**

```rust
            (Some(a), _) => Some(a),
        };
    }

```

**with:**

```rust
            (Some(a), _) => Some(a),
        };
    }

    /// What the core does to a schedule that is saved (spec §2): `fill_anchor` from
    /// `previous`, the stored schedule (`None` on create), then `check_runs_after(now)` when the
    /// schedule is new or changed. A schedule saved unchanged is not checked again, so a dot
    /// whose `Once` has passed can still be edited.
    pub fn prepare_save<Tz: TimeZone + 'static>(
        &mut self,
        previous: Option<&Schedule>,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> Result<()> {
        self.fill_anchor(previous, now);
        if previous != Some(self) {
            self.check_runs_after(&now, tz)?;
        }
        Ok(())
    }

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust

use crate::policy::{Policy, Preset};
use crate::{Error, Result};

```

**with:**

```rust

use crate::policy::{Policy, Preset};
use crate::schedule::Schedule;
use crate::{Error, Result};

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub workdir: String,
    pub workspace_mode: WorkspaceMode,
    pub schedule: Option<String>,
    pub policy: Policy,
    pub mcp_servers: Option<Value>,
```

**with:**

```rust
    pub workdir: String,
    pub workspace_mode: WorkspaceMode,
    pub schedule: Option<Schedule>,
    pub policy: Policy,
    pub mcp_servers: Option<Value>,
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
            }
        }
        if let Some(expr) = &self.schedule {
            crate::scheduler::parse_cron(expr)?;
        }
        if self.max_turns == 0 {
```

**with:**

```rust
            }
        }
        if let Some(schedule) = &self.schedule {
            schedule.validate()?;
        }
        if self.max_turns == 0 {
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
```

**with:**

```rust
use chrono::{Local, SubsecRound, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use super::{opt_json, Store};
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
use crate::util::{new_id, new_token, now};
use crate::{Error, Result};
```

**with:**

```rust
use super::{opt_json, Store};
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
use crate::schedule::Schedule;
use crate::util::{new_id, new_token, now};
use crate::{Error, Result};
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            workdir: row.try_get("workdir")?,
            workspace_mode: WorkspaceMode::parse(&row.try_get::<String, _>("workspace_mode")?)?,
            schedule: row.try_get("schedule")?,
            policy: serde_json::from_str(&row.try_get::<String, _>("policy")?)?,
            mcp_servers: opt_json(row.try_get("mcp_servers")?)?,
```

**with:**

```rust
            workdir: row.try_get("workdir")?,
            workspace_mode: WorkspaceMode::parse(&row.try_get::<String, _>("workspace_mode")?)?,
            schedule: row
                .try_get::<Option<String>, _>("schedule")?
                .map(|s| serde_json::from_str(&s))
                .transpose()?,
            policy: serde_json::from_str(&row.try_get::<String, _>("policy")?)?,
            mcp_servers: opt_json(row.try_get("mcp_servers")?)?,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        },
    })
}

```

**with:**

```rust
        },
    })
}

/// The spec as it is stored: validated, its schedule prepared against `previous`, the stored
/// one (`Schedule::prepare_save`: the `Every` anchor, and the refusal of a new or changed
/// schedule that would never run).
fn prepared(spec: &DotSpec, previous: Option<&Schedule>) -> Result<DotSpec> {
    spec.validate()?;
    let mut spec = spec.clone();
    if let Some(schedule) = &mut spec.schedule {
        // Whole milliseconds, as `util::now()`: the stored anchor reads back the same.
        schedule.prepare_save(previous, Utc::now().trunc_subsecs(3), &Local)?;
    }
    Ok(spec)
}

fn schedule_json(spec: &DotSpec) -> Result<Option<String>> {
    Ok(spec
        .schedule
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?)
}

```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
impl Store {
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        spec.validate()?;
        let id = new_id();
        let ts = now();
```

**with:**

```rust
impl Store {
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        let spec = &prepared(spec, None)?;
        let id = new_id();
        let ts = now();
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(&spec.schedule)
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
```

**with:**

```rust
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(schedule_json(spec)?)
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
    /// Updates a dot. `workdir`/`workspace_mode` may only change while the dot has no
    /// queued, running or awaiting runs (their workspaces were derived from the old values);
    /// otherwise nothing changes and `Error::Conflict` is returned. The check is atomic.
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        spec.validate()?;
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
```

**with:**

```rust
    /// Updates a dot. `workdir`/`workspace_mode` may only change while the dot has no
    /// queued, running or awaiting runs (their workspaces were derived from the old values);
    /// otherwise nothing changes and `Error::Conflict` is returned. The check is atomic. The
    /// schedule is prepared against the stored one (`prepared`).
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        let previous = self.get_dot(id).await?.spec.schedule;
        let spec = &prepared(spec, previous.as_ref())?;
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(&spec.schedule)
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(spec.mcp_servers.as_ref().map(|v| v.to_string()))
```

**with:**

```rust
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(schedule_json(spec)?)
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(spec.mcp_servers.as_ref().map(|v| v.to_string()))
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
use crate::model::{Dot, NewRun, Run, TriggerKind};
use crate::runner::Runner;
use crate::store::Store;
use crate::{Error, Result};
```

**with:**

```rust
use crate::model::{Dot, NewRun, Run, TriggerKind};
use crate::runner::Runner;
use crate::schedule::next_slot;
use crate::store::Store;
use crate::{Error, Result};
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
    }
    cron::Schedule::from_str(expr).map_err(|e| Error::Invalid(format!("cron {expr:?}: {e}")))
}

/// Next fire time strictly after `after`, in local time.
pub fn next_fire(expr: &str, after: DateTime<Local>) -> Result<Option<DateTime<Local>>> {
    Ok(parse_cron(expr)?.after(&after).next())
}

```

**with:**

```rust
    }
    cron::Schedule::from_str(expr).map_err(|e| Error::Invalid(format!("cron {expr:?}: {e}")))
}

```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
        let mut due: Vec<(DateTime<Local>, Dot)> = Vec::new();
        for dot in self.store.list_dots().await? {
            let Some(expr) = dot.spec.schedule.clone() else {
                continue;
            };
```

**with:**

```rust
        let mut due: Vec<(DateTime<Local>, Dot)> = Vec::new();
        for dot in self.store.list_dots().await? {
            let Some(schedule) = &dot.spec.schedule else {
                continue;
            };
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
                continue;
            }
            match next_fire(&expr, from) {
                Ok(Some(t)) => due.push((t, dot)),
                Ok(None) => {}
```

**with:**

```rust
                continue;
            }
            match next_slot(schedule, &from, &Local) {
                Ok(Some(t)) => due.push((t, dot)),
                Ok(None) => {}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast, Mutex};
```

**with:**

```rust
use std::time::Duration;

use chrono::{Local, Utc};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
use crate::runner::{Runner, RunnerConfig};
use crate::scheduler::Scheduler;
use crate::server::{self, ServerState};
```

**with:**

```rust
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{next_slot, upcoming, Schedule};
use crate::scheduler::Scheduler;
use crate::server::{self, ServerState};
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    /// Every dot with its active and last run and its next scheduled fire time.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = chrono::Local::now();
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
            let next_fire = match (&dot.spec.schedule, dot.spec.enabled) {
                (Some(expr), true) => crate::scheduler::next_fire(expr, now)
                    .ok()
                    .flatten()
```

**with:**

```rust
    /// Every dot with its active and last run and its next scheduled fire time.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = Local::now();
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
            let next_fire = match (&dot.spec.schedule, dot.spec.enabled) {
                (Some(schedule), true) => next_slot(schedule, &now, &Local)
                    .ok()
                    .flatten()
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    }

    /// The next `count` (1 to 10) fire times of a cron expression, RFC 3339 in local time.
    pub fn preview_schedule(expr: &str, count: usize) -> Result<Vec<String>> {
        let schedule = crate::scheduler::parse_cron(expr)?;
        Ok(schedule
            .after(&chrono::Local::now())
            .take(count.clamp(1, 10))
            .map(|t| t.to_rfc3339())
```

**with:**

```rust
    }

    /// The next `count` (1 to 10) slots of a schedule saved now, RFC 3339 in local time, within
    /// its bounds; empty when it has none. Refuses what `DotSpec::validate` refuses, so the UI
    /// parses no schedule.
    pub fn preview_schedule(schedule: &Schedule, count: usize) -> Result<Vec<String>> {
        schedule.validate()?;
        let now = Local::now();
        let mut schedule = schedule.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        Ok(upcoming(&schedule, &now, &Local)?
            .take(count.clamp(1, 10))
            .map(|t| t.to_rfc3339())
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
};
use dots_core::policy::{Policy, Preset, RuleCandidate};
use dots_core::settings::Settings;
use dots_core::Runtime;
```

**with:**

```rust
};
use dots_core::policy::{Policy, Preset, RuleCandidate};
use dots_core::schedule::Schedule;
use dots_core::settings::Settings;
use dots_core::Runtime;
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

#[tauri::command]
pub fn preview_schedule(expr: String, count: usize) -> Res<Vec<String>> {
    Ok(Runtime::preview_schedule(&expr, count)?)
}

```

**with:**

```rust

#[tauri::command]
pub fn preview_schedule(schedule: Schedule, count: usize) -> Res<Vec<String>> {
    Ok(Runtime::preview_schedule(&schedule, count)?)
}

```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS};
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
use dots_core::schedule::Schedule;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS};
use serde_json::{json, Value};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        "C:/dev/app",
    );
    s.schedule = Some("0 0 2 * * *".into());
    s.mcp_servers = Some(json!({"github": {"type": "http", "url": "http://127.0.0.1:9000/mcp"}}));
    s
```

**with:**

```rust
        "C:/dev/app",
    );
    s.schedule = Some(Schedule::cron("0 0 2 * * *"));
    s.mcp_servers = Some(json!({"github": {"type": "http", "url": "http://127.0.0.1:9000/mcp"}}));
    s
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
}

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
/// shared enum and `Limits` the validation limits the editor mirrors: the UI parses live data
```

**with:**

```rust
}

/// One schedule of every rule kind, with every kind of bound. The `every` without an anchor is
/// what the UI sends; the core fills the anchor when the dot is saved. Read through the core's
/// type, so a sample in a form the core does not accept fails here.
fn schedules() -> Value {
    let samples = [
        json!({"rule": {"kind": "once", "at": "2026-10-20T09:00"}, "catch_up": true, "until": null}),
        json!({
            "rule": {"kind": "every", "minutes": 90, "anchor": "2026-10-09T12:00:00Z"},
            "catch_up": true,
            "until": {"count": 10}
        }),
        json!({"rule": {"kind": "every", "minutes": 30}, "catch_up": false, "until": null}),
        json!({
            "rule": {"kind": "daily", "time": "09:00"},
            "catch_up": true,
            "until": {"date": "2026-12-31T18:00"}
        }),
        json!({
            "rule": {"kind": "weekly", "days": ["mon", "fri"], "time": "07:30"},
            "catch_up": true,
            "until": null
        }),
        to(Schedule::cron("0 0 2 * * *")),
    ];
    let read: Vec<Schedule> = samples
        .into_iter()
        .map(|v| serde_json::from_value(v).expect("sample schedule"))
        .collect();
    to(read)
}

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
/// shared enum and `Limits` the validation limits the editor mirrors: the UI parses live data
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    json!({
        "DotSpec": to(local_spec()),
        "Dot": to(dot()),
        "Run": to(run()),
```

**with:**

```rust
    json!({
        "DotSpec": to(local_spec()),
        "Schedule": schedules(),
        "Dot": to(dot()),
        "Run": to(run()),
```

**In `ui/src/api/types.ts`, replace:**

```ts
});

const dotSpecShape = {
  name: z.string(),
```

**with:**

```ts
});

// A dot's schedule (Scheduler v2 spec §2). Times are "HH:MM" and date-times
// "YYYY-MM-DDTHH:MM", local and without an offset; the core checks them. `anchor` (RFC 3339
// UTC) is the core's: the UI sends null, and the core keeps the stored phase.
export const Weekday = z.enum(["mon", "tue", "wed", "thu", "fri", "sat", "sun"]);
export const ScheduleRule = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("once"), at: z.string() }),
  z.strictObject({
    kind: z.literal("every"),
    minutes: z.number().int(),
    anchor: nullable(z.string()),
  }),
  z.strictObject({ kind: z.literal("daily"), time: z.string() }),
  z.strictObject({ kind: z.literal("weekly"), days: z.array(Weekday), time: z.string() }),
  z.strictObject({ kind: z.literal("cron"), expr: z.string() }),
]);
export const Until = z.union([
  z.strictObject({ count: z.number().int() }),
  z.strictObject({ date: z.string() }),
]);
export const Schedule = z.strictObject({
  rule: ScheduleRule,
  catch_up: z.boolean(),
  until: nullable(Until),
});

const dotSpecShape = {
  name: z.string(),
```

**In `ui/src/api/types.ts`, replace:**

```ts
  workdir: z.string(),
  workspace_mode: WorkspaceMode,
  schedule: nullable(z.string()),
  policy: Policy,
  mcp_servers: nullable(z.record(z.string(), z.unknown())),
```

**with:**

```ts
  workdir: z.string(),
  workspace_mode: WorkspaceMode,
  schedule: nullable(Schedule),
  policy: Policy,
  mcp_servers: nullable(z.record(z.string(), z.unknown())),
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
export type DotSpec = z.infer<typeof DotSpec>;
export type Dot = z.infer<typeof Dot>;
```

**with:**

```ts
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
export type Weekday = z.infer<typeof Weekday>;
export type ScheduleRule = z.infer<typeof ScheduleRule>;
export type Until = z.infer<typeof Until>;
export type Schedule = z.infer<typeof Schedule>;
export type DotSpec = z.infer<typeof DotSpec>;
export type Dot = z.infer<typeof Dot>;
```

**In `ui/src/api/commands.ts`, replace:**

```ts

import * as T from "./types";
import type { CmdErrorKind, DotSpec, Preset, Settings } from "./types";

/// A command failure. `kind` mirrors the Rust `CmdError`.
```

**with:**

```ts

import * as T from "./types";
import type { CmdErrorKind, DotSpec, Preset, Schedule, Settings } from "./types";

/// A command failure. `kind` mirrors the Rust `CmdError`.
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  /// Rejects with kind `conflict` while the local server is not running.
  webhookUrl: (id: string) => call(text, "webhook_url", { id }),
  previewSchedule: (expr: string, count: number) =>
    call(text.array(), "preview_schedule", { expr, count }),
  policyPreset: (preset: Preset) => call(T.Policy, "policy_preset", { preset }),
  newDotSpec: () => call(T.DotSpec, "new_dot_spec"),
```

**with:**

```ts
  /// Rejects with kind `conflict` while the local server is not running.
  webhookUrl: (id: string) => call(text, "webhook_url", { id }),
  /// The next `count` slots of a schedule saved now; the core's objection when it is not valid.
  previewSchedule: (schedule: Schedule, count: number) =>
    call(text.array(), "preview_schedule", { schedule, count }),
  policyPreset: (preset: Preset) => call(T.Policy, "policy_preset", { preset }),
  newDotSpec: () => call(T.DotSpec, "new_dot_spec"),
```

**In `ui/src/lib/query.ts`, replace:**

```ts

import { commands } from "@/api/commands";
import type { RunEventRecord, Status } from "@/api/types";

/// Data stays fresh until a runtime event or a mutation marks it stale (lib/runtime-events.ts):
```

**with:**

```ts

import { commands } from "@/api/commands";
import type { RunEventRecord, Schedule, Status } from "@/api/types";

/// Data stays fresh until a runtime event or a mutation marks it stale (lib/runtime-events.ts):
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  dot: (id: string) => ["dot", id] as const,
  newDotSpec: ["new-dot-spec"] as const,
  schedulePreview: (expr: string) => ["schedule-preview", expr] as const,
  webhookUrl: (dotId: string) => ["webhook-url", dotId] as const,
  runs: (dotId: string) => ["runs", dotId] as const,
```

**with:**

```ts
  dot: (id: string) => ["dot", id] as const,
  newDotSpec: ["new-dot-spec"] as const,
  schedulePreview: (schedule: Schedule | null) => ["schedule-preview", schedule] as const,
  webhookUrl: (dotId: string) => ["webhook-url", dotId] as const,
  runs: (dotId: string) => ["runs", dotId] as const,
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
import { z } from "zod";

import { LIMITS, type DotSpec } from "@/api/types";

export type SpecErrors = Partial<Record<keyof DotSpec, string>>;
```

**with:**

```ts
import { z } from "zod";

import { LIMITS, type DotSpec, type Schedule } from "@/api/types";

export type SpecErrors = Partial<Record<keyof DotSpec, string>>;
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
}

/// The editable part of a stored dot.
export function specOf(dot: DotSpec): DotSpec {
```

**with:**

```ts
}

/// The schedule the cron field holds once `text` is typed into it: a cron schedule without
/// catch-up or stop, as cron schedules were before Scheduler v2. An empty field is no schedule.
export function cronSchedule(text: string): Schedule | null {
  return text === "" ? null : { rule: { kind: "cron", expr: text }, catch_up: false, until: null };
}

/// The schedule a draft saves and previews: a cron expression without its surrounding spaces,
/// none when only spaces are left. Other kinds as they are.
export function scheduleOf(schedule: Schedule | null): Schedule | null {
  if (schedule?.rule.kind !== "cron") return schedule;
  const expr = schedule.rule.expr.trim();
  return expr === "" ? null : { ...schedule, rule: { kind: "cron", expr } };
}

/// The editable part of a stored dot.
export function specOf(dot: DotSpec): DotSpec {
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import type { ReactNode } from "react";

import { EngineKind, WorkspaceMode, type Dot, type DotSpec } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
```

**with:**

```tsx
import type { ReactNode } from "react";

import {
  EngineKind,
  WorkspaceMode,
  type Dot,
  type DotSpec,
  type Schedule,
  type ScheduleRule,
} from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SpecErrors } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
```

**with:**

```tsx
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { cronSchedule, type SpecErrors } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx

export type SchedulePreview = { times: string[]; pending: boolean };

function Section(props: { title: string; children: ReactNode }) {
```

**with:**

```tsx

export type SchedulePreview = { times: string[]; pending: boolean };

const localDate = (dateTime: string) => dateTime.replace("T", " ");

function describeRule(rule: ScheduleRule): string {
  switch (rule.kind) {
    case "once":
      return `Once at ${localDate(rule.at)}`;
    case "every":
      return `Every ${rule.minutes} minutes`;
    case "daily":
      return `Daily at ${rule.time}`;
    case "weekly":
      return `Weekly on ${rule.days.join(", ")} at ${rule.time}`;
    case "cron":
      return `Cron ${rule.expr}`;
  }
}

/// A schedule in words, for the kinds this form cannot edit yet.
function describeSchedule({ rule, until }: Schedule): string {
  const what = describeRule(rule);
  if (until === null) return what;
  return "count" in until
    ? `${what}, at most ${until.count} runs`
    : `${what}, until ${localDate(until.date)}`;
}

function Section(props: { title: string; children: ReactNode }) {
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx

      <Section title="Schedule">
        <Field
          label="Cron expression"
          htmlFor="dot-schedule"
          error={errors.schedule}
          hint="Six fields: second minute hour day-of-month month day-of-week. Empty: no schedule."
        >
          <Input
            id="dot-schedule"
            className="font-mono"
            placeholder="0 0 2 * * *"
            value={spec.schedule ?? ""}
            onChange={(e) => set("schedule", e.target.value === "" ? null : e.target.value)}
          />
        </Field>
        {props.schedule.times.length > 0 && (
          <div className="text-xs text-muted-foreground">
```

**with:**

```tsx

      <Section title="Schedule">
        {spec.schedule && spec.schedule.rule.kind !== "cron" ? (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-3">
              <span className="text-sm">{describeSchedule(spec.schedule)}</span>
              <Button size="sm" variant="outline" onClick={() => set("schedule", null)}>
                Remove schedule
              </Button>
            </div>
            {errors.schedule ? (
              <p className="text-xs text-destructive">{errors.schedule}</p>
            ) : (
              <p className="text-xs text-muted-foreground">
                This editor cannot change this kind of schedule yet. Saving keeps it as it is.
              </p>
            )}
          </div>
        ) : (
          <Field
            label="Cron expression"
            htmlFor="dot-schedule"
            error={errors.schedule}
            hint="Six fields: second minute hour day-of-month month day-of-week. Empty: no schedule."
          >
            <Input
              id="dot-schedule"
              className="font-mono"
              placeholder="0 0 2 * * *"
              value={spec.schedule?.rule.kind === "cron" ? spec.schedule.rule.expr : ""}
              onChange={(e) => set("schedule", cronSchedule(e.target.value))}
            />
          </Field>
        )}
        {props.schedule.times.length > 0 && (
          <div className="text-xs text-muted-foreground">
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { mcpOf, specErrors, specOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
```

**with:**

```tsx
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { mcpOf, scheduleOf, specErrors, specOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
  const save = useMutation({ mutationFn: props.onSave });

  // The core validates the cron expression and computes the preview; the UI parses no cron.
  const expr = useDebounced((draft.spec.schedule ?? "").trim(), 300);
  const preview = useQuery({
    queryKey: keys.schedulePreview(expr),
    queryFn: () => commands.previewSchedule(expr, 3),
    enabled: expr !== "",
  });

  const errors = specErrors(draft.spec, draft.mcpText);
  if (expr !== "" && preview.isError) errors.schedule = errorMessage(preview.error);

  function submit() {
```

**with:**

```tsx
  const save = useMutation({ mutationFn: props.onSave });

  // The core validates the schedule and computes the preview; the UI parses no schedule.
  const schedule = scheduleOf(useDebounced(draft.spec.schedule, 300));
  const preview = useQuery({
    queryKey: keys.schedulePreview(schedule),
    queryFn: () => (schedule ? commands.previewSchedule(schedule, 3) : []),
    enabled: schedule !== null,
  });

  const errors = specErrors(draft.spec, draft.mcpText);
  if (schedule !== null && preview.isError) errors.schedule = errorMessage(preview.error);

  function submit() {
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
    save.mutate({
      ...draft.spec,
      schedule: draft.spec.schedule?.trim() || null,
      endpoint_url: draft.spec.endpoint_url?.trim() || null,
      mcp_servers: "value" in mcp ? mcp.value : null,
```

**with:**

```tsx
    save.mutate({
      ...draft.spec,
      schedule: scheduleOf(draft.spec.schedule),
      endpoint_url: draft.spec.endpoint_url?.trim() || null,
      mcp_servers: "value" in mcp ? mcp.value : null,
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
        endpoints={settings?.local_endpoints ?? []}
        schedule={{
          times: expr !== "" && preview.data ? preview.data : [],
          pending: preview.isFetching,
        }}
```

**with:**

```tsx
        endpoints={settings?.local_endpoints ?? []}
        schedule={{
          times: schedule !== null && preview.data ? preview.data : [],
          pending: preview.isFetching,
        }}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
### Success criteria

- A dot configured with a cron schedule runs unattended while the app sits in the tray, and its
  result appears in the inbox with a Windows notification.
- A dot can be triggered by `POST` to a local webhook URL with its token.
```

**with:**

```markdown
### Success criteria

- A dot configured with a schedule runs unattended while the app sits in the tray, and its
  result appears in the inbox with a Windows notification.
- A dot can be triggered by `POST` to a local webhook URL with its token.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
| Hosting | Local Windows machine |
| Model backends | Hybrid: Claude via `claude` CLI (subscription); local models via own tool loop over OpenAI-compatible API |
| Triggers (v1) | Schedule (cron), webhook, manual (Run now + chat) |
| Autonomy | Per-dot policy: each tool is `allow`, `ask`, or `deny`; new dots default to "sandboxed" preset |
| Language | Rust |
```

**with:**

```markdown
| Hosting | Local Windows machine |
| Model backends | Hybrid: Claude via `claude` CLI (subscription); local models via own tool loop over OpenAI-compatible API |
| Triggers (v1) | Schedule (once, every, daily, weekly, cron; Scheduler v2), webhook, manual (Run now + chat) |
| Autonomy | Per-dot policy: each tool is `allow`, `ask`, or `deny`; new dots default to "sandboxed" preset |
| Language | Rust |
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- `engine` TEXT (`claude` | `local`), `model` TEXT, `endpoint_url` TEXT NULL (local only)
- `workdir` TEXT, `workspace_mode` TEXT (`worktree` | `folder`)
- `schedule` TEXT NULL (cron, 6-field with seconds), `timezone` TEXT (default local)
- `webhook_token` TEXT (random 32 bytes, base64url)
- `policy` TEXT (JSON, see §5), `mcp_servers` TEXT NULL (JSON, Claude only)
```

**with:**

```markdown
- `engine` TEXT (`claude` | `local`), `model` TEXT, `endpoint_url` TEXT NULL (local only)
- `workdir` TEXT, `workspace_mode` TEXT (`worktree` | `folder`)
- `schedule` TEXT NULL: the schedule as JSON (`2026-10-14-scheduler-v2-design.md` §2), e.g.
  `{"rule": {"kind": "daily", "time": "09:00"}, "catch_up": true, "until": {"count": 10}}`; rule
  kinds `once`, `every`, `daily`, `weekly` and `cron` (6-field with seconds, or 7 with year). A
  `once` in the past or a stop date before the first slot is refused when the dot is created or
  its schedule changes, not when it is loaded. The core sets an `every` rule's `anchor`.
  `timezone` TEXT (default local, unused: schedules use the PC's local time)
- `webhook_token` TEXT (random 32 bytes, base64url)
- `policy` TEXT (JSON, see §5), `mcp_servers` TEXT NULL (JSON, Claude only)
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- `reviewed_at` TEXT NULL (migration 0002; set by "Mark reviewed" or "Discard worktree", see
  §12 Inbox)

**run_events**
```

**with:**

```markdown
- `reviewed_at` TEXT NULL (migration 0002; set by "Mark reviewed" or "Discard worktree", see
  §12 Inbox)
- `retry_of` TEXT NULL, FK `runs(id)` ON DELETE SET NULL (migration 0005): the failed scheduled
  run that this run retries

**run_events**
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  `2026-10-13-approvals-v2-design.md` §3.4)

**settings** — key/value, each value a JSON document: `port` (default 47321),
`max_concurrent_runs` (default 2), `claude_path` (null = search PATH), `tested_claude_version`
```

**with:**

```markdown
  `2026-10-13-approvals-v2-design.md` §3.4)

**schedule_state** (migration 0005; Scheduler v2 spec §4) — one row per dot with a schedule:
`dot_id` PK (FK `dots`, ON DELETE CASCADE), `rule_key` (the schedule's JSON when the row was
made), `status` (`active` | `completed` | `expired`), `last_slot_at`, `fired_count`,
`watch_run_id`, `retry_slot_at`, `retry_attempt`, `retry_at`, `attention_run_id` (the run
columns are FKs to `runs`, ON DELETE SET NULL), `updated_at`.

**settings** — key/value, each value a JSON document: `port` (default 47321),
`max_concurrent_runs` (default 2), `claude_path` (null = search PATH), `tested_claude_version`
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown

Migrations: `0002` adds `runs.reviewed_at`; `0003` adds the index
`runs(root_run_id, queued_at, id)` that the Inbox query needs.

All timestamps are RFC 3339 UTC strings.
```

**with:**

```markdown

Migrations: `0002` adds `runs.reviewed_at`; `0003` adds the index
`runs(root_run_id, queued_at, id)` that the Inbox query needs; `0004` adds the approval apply
columns; `0005` converts each cron string in `dots.schedule` to
`{"rule":{"kind":"cron","expr":...},"catch_up":false,"until":null}` (no catch-up, so an upgraded
schedule behaves as before), adds `runs.retry_of` and creates `schedule_state`.

All timestamps are RFC 3339 UTC strings.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   oldest first), `list_run_events(run_id, after_seq)`, `approvals_for_run`,
   `inbox() -> { approvals, review_runs }`, `inbox_count()`, and
   `preview_schedule(expr, n) -> next n fire times` (validates the cron; the UI parses no cron).
7. Review actions on a thread's shared worktree:
   - `worktree_diff(run_id) -> { files: [{ path, status, patch, binary }], truncated, exists }` — `git
```

**with:**

```markdown
   oldest first), `list_run_events(run_id, after_seq)`, `approvals_for_run`,
   `inbox() -> { approvals, review_runs }`, `inbox_count()`, and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, answers `[]` when no slot is left, and
   previews an `every` rule from now; the UI parses no schedule).
7. Review actions on a thread's shared worktree:
   - `worktree_diff(run_id) -> { files: [{ path, status, patch, binary }], truncated, exists }` — `git
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: the first run fails with "contract.json is stale"; the second and third pass. The spec samples' `schedule` becomes `{"catch_up": false, "rule": {"expr": "0 0 2 * * *", "kind": "cron"}, "until": null}`, and the fixture gains the `Schedule` array of six samples.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test runtime_review_test --test runtime_status_test --test scheduler_test --test store_dots_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/contract.test.ts src/screens/DotEditorScreen.test.tsx 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `runtime_review_test` 5 passed, `runtime_status_test` 9 passed, `scheduler_test` 5 passed, `store_dots_test` 13 passed; `dots-app` 47 passed; both UI files pass.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 370 passed, `dots-app` 47 passed, no clippy line, `cargo fmt --check` silent, UI 251 passed in 20 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/contract.rs crates/dots-core/migrations/0005_scheduler_v2.sql crates/dots-core/src/model.rs crates/dots-core/src/runtime.rs crates/dots-core/src/schedule.rs crates/dots-core/src/scheduler.rs crates/dots-core/src/store/dots.rs crates/dots-core/tests/runtime_review_test.rs crates/dots-core/tests/runtime_status_test.rs crates/dots-core/tests/scheduler_test.rs crates/dots-core/tests/store_dots_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/lib/dot-rules.ts ui/src/lib/query.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/DotEditorScreen.tsx ui/src/screens/dot-editor/DotForm.tsx
git commit -m "feat(core): a dot's schedule is a schedule object, not a cron string"
```

### Task 3: A schedule's state row, kept in line with its dot

Each scheduled dot gets its `schedule_state` row (spec section 4), and the store gets the writes the loop will need, each one guarded so that a slot cannot be recorded twice and a judging cannot overwrite a newer watch. The runtime creates the row when a dot with a schedule is created, resets it when the schedule changes or a disabled dot is enabled, and deletes it with the schedule or the dot; every change is announced with a new event, `ScheduleUpdated`, on which the UI refetches the dots, the overview and the Inbox (spec section 7). `Run` gains `retry_of`. Nothing in the loop uses the row yet: the old loop still fires through `Runner::enqueue`. The main spec's section 3, its events and its data layer follow.

**Files:**
- Create: `crates/dots-core/src/store/schedules.rs`, `crates/dots-core/tests/runtime_schedule_test.rs`, `crates/dots-core/tests/store_schedules_test.rs`
- Modify: `apps/dots-app/src/contract.rs`, `apps/dots-app/src/events.rs`, `crates/dots-core/src/events.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/schedule.rs`, `crates/dots-core/src/store/mod.rs`, `crates/dots-core/src/store/runs.rs`, `crates/dots-core/src/util.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`, `ui/src/lib/runtime-events.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/common/mod.rs`, `crates/dots-core/tests/runtime_schedule_test.rs`, `crates/dots-core/tests/store_schedules_test.rs`, `ui/src/api/contract.test.ts`, `ui/src/lib/runtime-events.test.ts`

**Interfaces:**
- Consumes: Task 2's table and `DotSpec.schedule`; `insert_run` (Plan 1, now `pub(super)` so that `schedules.rs` inserts a run in its own transaction).
- Produces:
  - `Run.retry_of`, `NewRun.retry_of: Option<String>` (`NewRun::new` sets `None`), read and written by the store.
  - `ScheduleStatus` (`str_enum!`: `active`, `completed`, `expired`). `PendingRetry { slot_at, attempt: u32, at }` (RFC 3339 UTC). `ScheduleState { dot_id, rule_key, status, last_slot_at, fired_count: u32, watch_run_id: Option<String>, retry: Option<PendingRetry>, attention_run_id: Option<String>, updated_at }`: the row; the three retry columns are all set or all null. `ScheduleChange { status, watch_run_id, retry, attention_run_id: Option<Option<…>>, if_watching: Option<String> }` (`Default`; `Some(None)` clears; `if_watching` writes only while the row watches that run). `SlotStart { Skipped, Enqueued(Run), Coalesced(Run) }`.
  - `Schedule::rule_key(&self) -> String`: the schedule's JSON, anchor included. `util::timestamp(DateTime<Utc>) -> String` ("2026-10-14T09:00:00.123Z"); `util::now()` uses it.
  - `Store::get_schedule_state(dot_id) -> Result<Option<ScheduleState>>`; `Store::list_schedule_states() -> Result<Vec<ScheduleState>>` (rows of enabled dots, by `dot_id`).
  - `Store::sync_schedule_state(dot_id, schedule: Option<&Schedule>, reset: bool, now) -> Result<bool>`: one statement; `None` deletes, `Some` inserts or, when `reset` or the `rule_key` differs, resets the row (`active`, `last_slot_at = updated_at = now`, count 0, no watch, retry or attention). `true` when the row changed.
  - `Store::record_slot(dot_id, at, fire: Option<&NewRun>, now) -> Result<SlotStart>`: one transaction whose first write moves `last_slot_at` to `at` only when it is newer (else `Conflict`, nothing written); with `fire`, joins the dot's oldest queued run or inserts the run, counts the slot, watches the run and drops a pending retry. Attention is untouched.
  - `Store::start_retry(dot_id, new: &NewRun, now) -> Result<SlotStart>`: clears a pending retry (`Conflict` without one), joins the queued run or inserts `new`, watches it; the count and `last_slot_at` are untouched.
  - `Store::update_schedule_state(dot_id, change: &ScheduleChange, now) -> Result<Option<ScheduleState>>`: one `UPDATE … RETURNING`; `None` without a row or when `if_watching` does not match.
  - `RuntimeEvent::ScheduleUpdated { dot_id }` (JSON `{"type": "schedule_updated", "dot_id": …}`), published exactly when a row changed or a deleted dot had one. The app's notifier ignores it; `affects_inbox` is already true for it.
  - Lifecycle, in `Runtime`: `create_dot` syncs without a reset; `update_dot` resets when the dot is enabled by the save, deletes when the schedule was removed, and otherwise resets only for another `rule_key`; `set_dot_enabled` resets on an enable of a disabled dot; `delete_dot` lets the foreign key remove the row and announces it.
  - UI: `retry_of` in the run shape; the `schedule_updated` variant of `RuntimeEvent`; `applyRuntimeEvent` invalidates `keys.dots`, `keys.overview` and `keys.inbox` for it.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/common/mod.rs`, replace:**

```rust
        ended_at: None,
        reviewed_at: None,
    }
}
```

**with:**

```rust
        ended_at: None,
        reviewed_at: None,
        retry_of: None,
    }
}
```

**Create `crates/dots-core/tests/runtime_schedule_test.rs`:**

```rust
mod common;

use std::path::PathBuf;

use chrono::Utc;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, ScheduleChange, ScheduleState, ScheduleStatus, WorkspaceMode,
};
use dots_core::schedule::Schedule;
use dots_core::util::now;
use dots_core::{Config, Runtime};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

async fn start(dir: &TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    Runtime::start(cfg).await.unwrap()
}

/// Every `minutes`, as the editor sends it: without an anchor. The first slot is hours away,
/// so nothing fires during a test.
fn every(minutes: u32) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": minutes},
        "catch_up": true,
        "until": null
    }))
    .unwrap()
}

fn folder_spec(dir: &TempDir, name: &str, schedule: Option<Schedule>) -> DotSpec {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.schedule = schedule;
    s
}

/// The dots named by the `ScheduleUpdated` events published since the last call.
fn updated(rx: &mut Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::ScheduleUpdated { dot_id } = ev {
            ids.push(dot_id);
        }
    }
    ids
}

async fn row(rt: &Runtime, dot: &Dot) -> Option<ScheduleState> {
    rt.store().get_schedule_state(&dot.id).await.unwrap()
}

/// Marks the row completed behind the runtime's back, so that a reset shows.
async fn complete(rt: &Runtime, dot: &Dot) {
    let change = ScheduleChange {
        status: Some(ScheduleStatus::Completed),
        ..Default::default()
    };
    rt.store()
        .update_schedule_state(&dot.id, &change, Utc::now())
        .await
        .unwrap()
        .unwrap();
}

async fn status(rt: &Runtime, dot: &Dot) -> ScheduleStatus {
    row(rt, dot).await.expect("a row").status
}

/// The row is as a reset at or after `since` leaves it, for the dot's stored schedule.
async fn assert_fresh(rt: &Runtime, dot: &Dot, since: &str) {
    let row = row(rt, dot).await.expect("a row");
    assert_eq!(row.rule_key, dot.spec.schedule.as_ref().unwrap().rule_key());
    assert_eq!(row.status, ScheduleStatus::Active);
    assert!(
        row.last_slot_at.as_str() >= since,
        "{row:?} is older than {since}"
    );
    assert_eq!(row.updated_at, row.last_slot_at);
    assert_eq!(row.fired_count, 0);
    assert_eq!(
        (row.watch_run_id, row.retry, row.attention_run_id),
        (None, None, None)
    );
}

#[tokio::test]
async fn the_schedule_row_follows_the_dot() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let mut rx = rt.subscribe();

    // Created with a schedule: a fresh row.
    let since = now();
    let dot = rt
        .create_dot(folder_spec(&dir, "a", Some(every(600))))
        .await
        .unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // A save that leaves the schedule as it is leaves the row alone. The editor sends no
    // anchor; the core keeps the stored one, so the rule key is the same.
    complete(&rt, &dot).await;
    let mut edit = dot.spec.clone();
    edit.instructions = "Do another thing.".into();
    edit.schedule = Some(every(600));
    rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());

    // A changed schedule resets it.
    edit.schedule = Some(every(720));
    let since = now();
    let dot = rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // Disabling leaves it alone; enabling resets it; enabling an enabled dot does not.
    complete(&rt, &dot).await;
    rt.set_dot_enabled(&dot.id, false).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());
    let since = now();
    rt.set_dot_enabled(&dot.id, true).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    complete(&rt, &dot).await;
    rt.set_dot_enabled(&dot.id, true).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());

    // So does enabling it in the editor.
    edit.enabled = false;
    rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());
    edit.enabled = true;
    let since = now();
    rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // The schedule removed: no row.
    edit.schedule = None;
    rt.update_dot(&dot.id, edit).await.unwrap();
    assert_eq!(row(&rt, &dot).await, None);
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    rt.shutdown();
}

#[tokio::test]
async fn only_a_dot_with_a_schedule_has_a_row_and_it_goes_with_the_dot() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let mut rx = rt.subscribe();

    let plain = rt
        .create_dot(folder_spec(&dir, "plain", None))
        .await
        .unwrap();
    assert_eq!(row(&rt, &plain).await, None);
    assert!(updated(&mut rx).is_empty());

    // A disabled dot gets its row too; it is not processed while the dot is disabled.
    let mut s = folder_spec(&dir, "off", Some(every(600)));
    s.enabled = false;
    let since = now();
    let dot = rt.create_dot(s).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    rt.delete_dot(&dot.id).await.unwrap();
    assert_eq!(row(&rt, &dot).await, None);
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    rt.delete_dot(&plain.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    rt.shutdown();
}
```

**Create `crates/dots-core/tests/store_schedules_test.rs`:**

```rust
mod common;

use chrono::{DateTime, Utc};
use common::{spec, temp_store};
use dots_core::model::{
    Dot, NewRun, PendingRetry, RunStatus, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart,
    TriggerKind,
};
use dots_core::schedule::Schedule;
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;

const T0: &str = "2026-10-14T09:00:00.000Z";
const T1: &str = "2026-10-14T10:00:00.000Z";
const T2: &str = "2026-10-14T11:00:00.000Z";
const T3: &str = "2026-10-14T12:00:00.000Z";

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn daily_at_nine() -> Schedule {
    Schedule::cron("0 0 9 * * *")
}

/// A dot with a schedule and its row, made at `T0`.
async fn scheduled(store: &Store, name: &str) -> Dot {
    let mut s = spec(name);
    s.schedule = Some(daily_at_nine());
    let dot = store.create_dot(&s).await.unwrap();
    assert!(store
        .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, t(T0))
        .await
        .unwrap());
    dot
}

async fn state(store: &Store, dot: &Dot) -> ScheduleState {
    store.get_schedule_state(&dot.id).await.unwrap().unwrap()
}

/// A row as a reset leaves it.
fn fresh(dot: &Dot, schedule: &Schedule, at: &str) -> ScheduleState {
    ScheduleState {
        dot_id: dot.id.clone(),
        rule_key: serde_json::to_string(schedule).unwrap(),
        status: ScheduleStatus::Active,
        last_slot_at: at.into(),
        fired_count: 0,
        watch_run_id: None,
        retry: None,
        attention_run_id: None,
        updated_at: at.into(),
    }
}

fn retry(attempt: u32) -> PendingRetry {
    PendingRetry {
        slot_at: T1.into(),
        attempt,
        at: T2.into(),
    }
}

async fn failed_run(store: &Store, dot: &Dot) -> String {
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    store
        .finish_run(&run.id, RunStatus::Failed, None, Some("boom"))
        .await
        .unwrap();
    run.id
}

/// Watches `run_id`, with a retry pending and attention set: a row in the middle of things.
async fn busy(store: &Store, dot: &Dot, run_id: &str) {
    let change = ScheduleChange {
        watch_run_id: Some(Some(run_id.into())),
        retry: Some(Some(retry(2))),
        attention_run_id: Some(Some(run_id.into())),
        ..Default::default()
    };
    store
        .update_schedule_state(&dot.id, &change, t(T0))
        .await
        .unwrap()
        .unwrap();
}

fn scheduled_run(dot: &Dot, at: &str) -> NewRun {
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.payload = Some(json!({ "scheduled_for": at }));
    new
}

#[test]
fn the_rule_key_is_the_schedules_json() {
    let schedule = daily_at_nine();
    assert_eq!(
        schedule.rule_key(),
        r#"{"rule":{"kind":"cron","expr":"0 0 9 * * *"},"catch_up":false,"until":null}"#
    );
}

#[tokio::test]
async fn a_row_is_created_reset_and_deleted_with_the_schedule() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let schedule = dot.spec.schedule.clone().unwrap();
    assert_eq!(state(&store, &dot).await, fresh(&dot, &schedule, T0));

    // The same schedule: nothing to do.
    let run = failed_run(&store, &dot).await;
    busy(&store, &dot, &run).await;
    let before = state(&store, &dot).await;
    assert!(!store
        .sync_schedule_state(&dot.id, Some(&schedule), false, t(T1))
        .await
        .unwrap());
    assert_eq!(state(&store, &dot).await, before);

    // A reset (the dot was enabled) forgets the watch, the retry, the attention and the count.
    assert!(store
        .sync_schedule_state(&dot.id, Some(&schedule), true, t(T1))
        .await
        .unwrap());
    assert_eq!(state(&store, &dot).await, fresh(&dot, &schedule, T1));

    // A different schedule resets the row too.
    busy(&store, &dot, &run).await;
    let other = Schedule::cron("0 0 10 * * *");
    assert!(store
        .sync_schedule_state(&dot.id, Some(&other), false, t(T2))
        .await
        .unwrap());
    assert_eq!(state(&store, &dot).await, fresh(&dot, &other, T2));

    // No schedule, no row.
    assert!(store
        .sync_schedule_state(&dot.id, None, false, t(T3))
        .await
        .unwrap());
    assert_eq!(store.get_schedule_state(&dot.id).await.unwrap(), None);
    assert!(!store
        .sync_schedule_state(&dot.id, None, false, t(T3))
        .await
        .unwrap());
}

#[tokio::test]
async fn deleting_the_dot_deletes_its_row() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    store.delete_dot(&dot.id).await.unwrap();
    assert_eq!(store.get_schedule_state(&dot.id).await.unwrap(), None);
}

#[tokio::test]
async fn a_pass_works_on_the_rows_of_enabled_dots() {
    let (_d, store) = temp_store().await;
    let on = scheduled(&store, "on").await;
    let off = scheduled(&store, "off").await;
    store.set_dot_enabled(&off.id, false).await.unwrap();
    // A dot without a schedule has no row.
    store.create_dot(&spec("plain")).await.unwrap();

    let rows = store.list_schedule_states().await.unwrap();
    assert_eq!(rows, [state(&store, &on).await]);
    // The disabled dot's row is left as it is.
    assert!(store.get_schedule_state(&off.id).await.unwrap().is_some());
}

#[tokio::test]
async fn a_skipped_slot_moves_last_slot_at_only() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let before = state(&store, &dot).await;

    let start = store
        .record_slot(&dot.id, t(T1), None, t(T2))
        .await
        .unwrap();
    assert_eq!(start, SlotStart::Skipped);
    assert_eq!(
        state(&store, &dot).await,
        ScheduleState {
            last_slot_at: T1.into(),
            updated_at: T2.into(),
            ..before
        }
    );
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_fired_slot_enqueues_counts_watches_and_drops_the_retry() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    busy(&store, &dot, &failed).await;

    let new = scheduled_run(&dot, "2026-10-14T12:00:00+02:00");
    let SlotStart::Enqueued(run) = store
        .record_slot(&dot.id, t(T1), Some(&new), t(T2))
        .await
        .unwrap()
    else {
        panic!("expected a new run")
    };
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.trigger, TriggerKind::Schedule);
    assert_eq!(run.payload, new.payload);
    assert_eq!(store.get_run(&run.id).await.unwrap(), run);

    let row = state(&store, &dot).await;
    assert_eq!(row.last_slot_at, T1);
    assert_eq!(row.fired_count, 1);
    assert_eq!(row.watch_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(row.retry, None);
    // Attention stays until a scheduled run succeeds (judged later).
    assert_eq!(row.attention_run_id.as_deref(), Some(failed.as_str()));
    assert_eq!(row.updated_at, T2);
}

#[tokio::test]
async fn a_slot_joins_the_queued_run() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let queued = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();

    let start = store
        .record_slot(&dot.id, t(T1), Some(&scheduled_run(&dot, T1)), t(T1))
        .await
        .unwrap();
    assert_eq!(start, SlotStart::Coalesced(queued.clone()));
    assert_eq!(
        store.list_runs(None, 10).await.unwrap(),
        vec![queued.clone()]
    );
    let row = state(&store, &dot).await;
    assert_eq!(row.fired_count, 1);
    assert_eq!(row.watch_run_id, Some(queued.id));
}

#[tokio::test]
async fn a_slot_that_is_not_newer_than_the_last_one_is_refused() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    store
        .record_slot(&dot.id, t(T1), None, t(T1))
        .await
        .unwrap();
    let before = state(&store, &dot).await;

    // The same slot again, an older one (the row was reset after the pass read it), and a dot
    // without a row: nothing is written and no run is made.
    for (dot_id, at) in [
        (&dot.id, T1),
        (&dot.id, T0),
        (&"no-such-dot".to_string(), T2),
    ] {
        let err = store
            .record_slot(dot_id, t(at), Some(&scheduled_run(&dot, at)), t(T3))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    }
    assert_eq!(state(&store, &dot).await, before);
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_retry_starts_a_run_without_counting() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    store
        .record_slot(&dot.id, t(T1), None, t(T1))
        .await
        .unwrap();
    busy(&store, &dot, &failed).await;
    let before = state(&store, &dot).await;

    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.retry_of = Some(failed.clone());
    new.payload = Some(json!({ "scheduled_for": T1, "attempt": 2 }));
    let SlotStart::Enqueued(run) = store.start_retry(&dot.id, &new, t(T2)).await.unwrap() else {
        panic!("expected a new run")
    };
    assert_eq!(run.retry_of.as_deref(), Some(failed.as_str()));
    assert_eq!(run.payload, new.payload);
    assert_eq!(
        state(&store, &dot).await,
        ScheduleState {
            watch_run_id: Some(run.id.clone()),
            retry: None,
            updated_at: T2.into(),
            ..before
        }
    );

    // No retry is pending any more.
    let err = store.start_retry(&dot.id, &new, t(T3)).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(store.list_runs(None, 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_retry_joins_the_queued_run() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    busy(&store, &dot, &failed).await;
    let queued = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();

    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.retry_of = Some(failed);
    let start = store.start_retry(&dot.id, &new, t(T2)).await.unwrap();
    assert_eq!(start, SlotStart::Coalesced(queued.clone()));
    let row = state(&store, &dot).await;
    assert_eq!(row.watch_run_id, Some(queued.id));
    assert_eq!((row.retry, row.fired_count), (None, 0));
}

#[tokio::test]
async fn an_update_writes_only_the_fields_it_names() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    busy(&store, &dot, &failed).await;
    let before = state(&store, &dot).await;

    let done = ScheduleChange {
        status: Some(ScheduleStatus::Completed),
        ..Default::default()
    };
    let row = store
        .update_schedule_state(&dot.id, &done, t(T1))
        .await
        .unwrap();
    let expected = ScheduleState {
        status: ScheduleStatus::Completed,
        updated_at: T1.into(),
        ..before
    };
    assert_eq!(row.as_ref(), Some(&expected));
    assert_eq!(state(&store, &dot).await, expected);

    // `Some(None)` clears; a condition on another watched run writes nothing.
    let clear = ScheduleChange {
        watch_run_id: Some(None),
        retry: Some(None),
        attention_run_id: Some(None),
        if_watching: Some("another-run".into()),
        ..Default::default()
    };
    assert_eq!(
        store
            .update_schedule_state(&dot.id, &clear, t(T2))
            .await
            .unwrap(),
        None
    );
    assert_eq!(state(&store, &dot).await, expected);
    let clear = ScheduleChange {
        if_watching: Some(failed.clone()),
        ..clear
    };
    let row = store
        .update_schedule_state(&dot.id, &clear, t(T2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.watch_run_id, row.retry, row.attention_run_id),
        (None, None, None)
    );

    // A pending retry is written whole.
    let later = ScheduleChange {
        retry: Some(Some(retry(6))),
        ..Default::default()
    };
    let row = store
        .update_schedule_state(&dot.id, &later, t(T3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.retry, Some(retry(6)));

    // No row, nothing written.
    assert_eq!(
        store
            .update_schedule_state("no-such-dot", &done, t(T3))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_run_records_the_run_it_retries() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let plain = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(plain.retry_of, None);
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.retry_of = Some(plain.id.clone());
    let retry = store.create_run(&new).await.unwrap();
    assert_eq!(retry.retry_of, Some(plain.id));
}

#[test]
fn timestamps_have_milliseconds_and_a_z() {
    assert_eq!(
        timestamp(t("2026-10-14T11:00:00.1234+02:00")),
        "2026-10-14T09:00:00.123Z"
    );
}
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  });

  it("has a sample of every schedule kind and bound", () => {
    const kinds = T.ScheduleRule.options.map((o) => o.shape.kind.value);
```

**with:**

```ts
  });

  it("has a sample of every runtime event type", () => {
    const types = T.RuntimeEvent.options.map((o) => o.shape.type.value);
    expect(fixture.RuntimeEvent.map((e) => e.type).sort()).toEqual([...types].sort());
  });

  it("has a sample of every schedule kind and bound", () => {
    const kinds = T.ScheduleRule.options.map((o) => o.shape.kind.value);
```

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts
      false,
    ]);
  });

```

**with:**

```ts
      false,
    ]);
  });

  it("marks the dots, the overview and the inbox stale on a schedule update", () => {
    const qc = createQueryClient();
    const lists = [keys.dots, keys.overview, keys.inbox];
    const untouched = [keys.dot(sample.dot.id), keys.runs(sample.dot.id), keys.settings];
    for (const key of [...lists, ...untouched]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test runtime_schedule_test --test store_schedules_test 2>&1 | grep -E "^error|could not compile" | sort | uniq -c | sort -rn
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/contract.test.ts src/lib/runtime-events.test.ts 2>&1 | grep -E "Tests |Test Files|× ")
```

Expected: the two Rust test targets do not compile. The decisive lines: `` no method named `record_slot` found for struct `Store` ``, the same for `update_schedule_state`, `sync_schedule_state`, `start_retry`, `get_schedule_state` and `list_schedule_states`, `` no field `retry_of` on type `NewRun` ``, `` no variant named `ScheduleUpdated` found for enum `RuntimeEvent` ``, `` unresolved import `dots_core::util::timestamp` ``, then ``could not compile `dots-core` (test "store_schedules_test") due to 35 previous errors`` and `(test "runtime_schedule_test") due to 6 previous errors`. The UI: `Test Files  1 failed | 1 passed (2)`, `Tests  1 failed | 51 passed (52)`; the failing test is "marks the dots, the overview and the inbox stale on a schedule update". The new contract test passes before the task: it compares the fixture's event types with the UI's, and neither has the new one yet.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    /// Set when the user marked the run reviewed or discarded its worktree.
    pub reviewed_at: Option<String>,
}

```

**with:**

```rust
    /// Set when the user marked the run reviewed or discarded its worktree.
    pub reviewed_at: Option<String>,
    /// The failed scheduled run that this run retries.
    pub retry_of: Option<String>,
}

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub branch: Option<String>,
    pub base_commit: Option<String>,
}

```

**with:**

```rust
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    pub retry_of: Option<String>,
}

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
            branch: None,
            base_commit: None,
        }
    }
}

```

**with:**

```rust
            branch: None,
            base_commit: None,
            retry_of: None,
        }
    }
}

str_enum!(ScheduleStatus {
    Active => "active",
    Completed => "completed",
    Expired => "expired",
});

/// A retry that waits (Scheduler v2 spec §6). Times are RFC 3339 UTC (`util::timestamp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRetry {
    /// The slot whose occurrence is retried.
    pub slot_at: String,
    /// The attempt the retry is, 2 to 6.
    pub attempt: u32,
    /// When it starts.
    pub at: String,
}

/// Where a dot's schedule stands: its `schedule_state` row (Scheduler v2 spec §4). Times are
/// RFC 3339 UTC (`util::timestamp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleState {
    pub dot_id: String,
    /// The schedule's JSON when the row was made or reset (`Schedule::rule_key`).
    pub rule_key: String,
    pub status: ScheduleStatus,
    /// The newest slot handled: fired, coalesced or skipped.
    pub last_slot_at: String,
    /// Slots that started a run or joined a queued one.
    pub fired_count: u32,
    /// The run whose outcome decides the current occurrence.
    pub watch_run_id: Option<String>,
    pub retry: Option<PendingRetry>,
    /// The last failed run of an occurrence that failed in the end ("Needs attention").
    pub attention_run_id: Option<String>,
    pub updated_at: String,
}

/// Input for `Store::update_schedule_state`: the fields to write, in one statement. `None`
/// leaves a field as it is; `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScheduleChange {
    pub status: Option<ScheduleStatus>,
    pub watch_run_id: Option<Option<String>>,
    pub retry: Option<Option<PendingRetry>>,
    pub attention_run_id: Option<Option<String>>,
    /// Writes only while the row watches this run: the change follows from judging it.
    pub if_watching: Option<String>,
}

/// What `Store::record_slot` and `Store::start_retry` did with a slot or a retry.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotStart {
    /// The slot was skipped: no run.
    Skipped,
    /// A new run was queued. It is announced and the dispatcher woken by the caller.
    Enqueued(Run),
    /// The dot's queued run serves it.
    Coalesced(Run),
}

```

**In `crates/dots-core/src/schedule.rs`, replace:**

```rust
            until: None,
        }
    }

```

**with:**

```rust
            until: None,
        }
    }

    /// The schedule's JSON, which the schedule row keeps as its `rule_key` (spec §4): a row
    /// whose key differs from the dot's schedule is reset. It includes the `Every` anchor, so
    /// it is taken after `prepare_save`.
    pub fn rule_key(&self) -> String {
        serde_json::to_string(self).expect("a schedule always serialises")
    }

```

**In `crates/dots-core/src/util.rs`, replace:**

```rust
/// Current time as RFC 3339 UTC with milliseconds, e.g. `2026-10-08T18:24:30.129Z`.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

```

**with:**

```rust
/// Current time as RFC 3339 UTC with milliseconds, e.g. `2026-10-08T18:24:30.129Z`.
pub fn now() -> String {
    timestamp(chrono::Utc::now())
}

/// `t` as the store writes times: RFC 3339 UTC with milliseconds (`now()`'s form). Strings of
/// this form sort as their times do.
pub fn timestamp(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
const RUN_COLUMNS: &str = "id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, status, \
    session_id, workspace_path, branch, base_commit, summary, error, tokens_in, tokens_out, \
    queued_at, started_at, ended_at, reviewed_at";

fn run_from_row(row: &SqliteRow) -> Result<Run> {
```

**with:**

```rust
const RUN_COLUMNS: &str = "id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, status, \
    session_id, workspace_path, branch, base_commit, summary, error, tokens_in, tokens_out, \
    queued_at, started_at, ended_at, reviewed_at, retry_of";

fn run_from_row(row: &SqliteRow) -> Result<Run> {
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
        ended_at: row.try_get("ended_at")?,
        reviewed_at: row.try_get("reviewed_at")?,
    })
}
```

**with:**

```rust
        ended_at: row.try_get("ended_at")?,
        reviewed_at: row.try_get("reviewed_at")?,
        retry_of: row.try_get("retry_of")?,
    })
}
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
}

async fn insert_run(conn: &mut SqliteConnection, new: &NewRun) -> Result<String> {
    let id = new_id();
    let root = new.root_run_id.clone().unwrap_or_else(|| id.clone());
    sqlx::query(
        "INSERT INTO runs (id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, \
         status, session_id, workspace_path, branch, base_commit, queued_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
```

**with:**

```rust
}

pub(super) async fn insert_run(conn: &mut SqliteConnection, new: &NewRun) -> Result<String> {
    let id = new_id();
    let root = new.root_run_id.clone().unwrap_or_else(|| id.clone());
    sqlx::query(
        "INSERT INTO runs (id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, \
         status, session_id, workspace_path, branch, base_commit, queued_at, retry_of) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
    .bind(&new.base_commit)
    .bind(now())
    .execute(conn)
    .await?;
```

**with:**

```rust
    .bind(&new.base_commit)
    .bind(now())
    .bind(&new.retry_of)
    .execute(conn)
    .await?;
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod dots;
mod runs;
mod settings;

```

**with:**

```rust
mod dots;
mod runs;
mod schedules;
mod settings;

```

**Create `crates/dots-core/src/store/schedules.rs`:**

```rust
//! The `schedule_state` rows: where each dot's schedule stands (Scheduler v2 spec §4). The
//! runtime keeps a row in line with its dot (`sync_schedule_state`); the scheduler's pass
//! handles slots and retries with `record_slot` and `start_retry` and writes the rest with
//! `update_schedule_state`, each in one transaction. Callers pass `now`, so the scheduler's
//! clock decides every time written here.

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{
    NewRun, PendingRetry, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart,
};
use crate::schedule::Schedule;
use crate::util::timestamp;
use crate::{Error, Result};

const STATE_COLUMNS: &str = "dot_id, rule_key, status, last_slot_at, fired_count, \
    watch_run_id, retry_slot_at, retry_attempt, retry_at, attention_run_id, updated_at";

fn state_from_row(row: &SqliteRow) -> Result<ScheduleState> {
    let retry = match (
        row.try_get::<Option<String>, _>("retry_slot_at")?,
        row.try_get::<Option<i64>, _>("retry_attempt")?,
        row.try_get::<Option<String>, _>("retry_at")?,
    ) {
        (Some(slot_at), Some(attempt), Some(at)) => Some(PendingRetry {
            slot_at,
            attempt: attempt as u32,
            at,
        }),
        _ => None,
    };
    Ok(ScheduleState {
        dot_id: row.try_get("dot_id")?,
        rule_key: row.try_get("rule_key")?,
        status: ScheduleStatus::parse(&row.try_get::<String, _>("status")?)?,
        last_slot_at: row.try_get("last_slot_at")?,
        fired_count: row.try_get::<i64, _>("fired_count")? as u32,
        watch_run_id: row.try_get("watch_run_id")?,
        retry,
        attention_run_id: row.try_get("attention_run_id")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// The dot's queued run if it has one (the oldest), or else `new` inserted; true when joined.
async fn start_or_join(conn: &mut SqliteConnection, new: &NewRun) -> Result<(String, bool)> {
    let queued: Option<String> = sqlx::query_scalar(
        "SELECT id FROM runs WHERE dot_id = ? AND status = 'queued' \
         ORDER BY queued_at, id LIMIT 1",
    )
    .bind(&new.dot_id)
    .fetch_optional(&mut *conn)
    .await?;
    match queued {
        Some(id) => Ok((id, true)),
        None => Ok((insert_run(conn, new).await?, false)),
    }
}

impl Store {
    /// The dot's schedule row, if it has one.
    pub async fn get_schedule_state(&self, dot_id: &str) -> Result<Option<ScheduleState>> {
        let row = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM schedule_state WHERE dot_id = ?"
        ))
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(state_from_row).transpose()
    }

    /// The rows of the enabled dots, by dot id: what a scheduler pass works on. A disabled
    /// dot's row is left as it is.
    pub async fn list_schedule_states(&self) -> Result<Vec<ScheduleState>> {
        let rows = sqlx::query(&format!(
            "SELECT {STATE_COLUMNS} FROM schedule_state \
             WHERE dot_id IN (SELECT id FROM dots WHERE enabled = 1) ORDER BY dot_id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(state_from_row).collect()
    }

    /// Brings the dot's row in line with its schedule, in one statement (spec §4, lifecycle of
    /// a row). Without a schedule the row is deleted. With one, a missing row is created, and
    /// a row is reset when `reset` is set (the dot was enabled) or when its `rule_key` is not
    /// the schedule's. A new or reset row is `active`, with `last_slot_at = now`, no count, no
    /// watched run, no retry and no attention, so it never catches up a slot from before.
    /// True when the row changed.
    pub async fn sync_schedule_state(
        &self,
        dot_id: &str,
        schedule: Option<&Schedule>,
        reset: bool,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(schedule) = schedule else {
            let res = sqlx::query("DELETE FROM schedule_state WHERE dot_id = ?")
                .bind(dot_id)
                .execute(&self.pool)
                .await?;
            return Ok(res.rows_affected() > 0);
        };
        let ts = timestamp(now);
        let res = sqlx::query(
            "INSERT INTO schedule_state (dot_id, rule_key, status, last_slot_at, fired_count, \
             updated_at) VALUES (?, ?, ?, ?, 0, ?) \
             ON CONFLICT (dot_id) DO UPDATE SET rule_key = excluded.rule_key, \
             status = excluded.status, last_slot_at = excluded.last_slot_at, fired_count = 0, \
             watch_run_id = NULL, retry_slot_at = NULL, retry_attempt = NULL, retry_at = NULL, \
             attention_run_id = NULL, updated_at = excluded.updated_at \
             WHERE ? OR schedule_state.rule_key <> excluded.rule_key",
        )
        .bind(dot_id)
        .bind(schedule.rule_key())
        .bind(ScheduleStatus::Active.as_str())
        .bind(&ts)
        .bind(&ts)
        .bind(reset)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Handles the due slot `at`, in one transaction (spec §5). `fire: None` skips it.
    /// `Some(new)` fires it: the slot joins the dot's queued run if there is one, and `new`
    /// is inserted otherwise. Either way `last_slot_at = at`; a fired slot also counts, its
    /// run becomes the watched run, and a pending retry of an older slot is dropped.
    /// `Conflict`, and nothing is written, when the dot has no row or its `last_slot_at` is
    /// not before `at` (the row was reset or handled the slot after the caller read it).
    pub async fn record_slot(
        &self,
        dot_id: &str,
        at: DateTime<Utc>,
        fire: Option<&NewRun>,
        now: DateTime<Utc>,
    ) -> Result<SlotStart> {
        let slot = timestamp(at);
        let mut tx = self.pool.begin().await?;
        // The guarded update goes first: it takes the write lock, so the check and the run
        // below are one step.
        let res = sqlx::query(
            "UPDATE schedule_state SET last_slot_at = ?, updated_at = ? \
             WHERE dot_id = ? AND last_slot_at < ?",
        )
        .bind(&slot)
        .bind(timestamp(now))
        .bind(dot_id)
        .bind(&slot)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "the schedule of dot {dot_id} has no row or has handled {slot} already"
            )));
        }
        let Some(new) = fire else {
            tx.commit().await?;
            return Ok(SlotStart::Skipped);
        };
        let (run_id, joined) = start_or_join(&mut tx, new).await?;
        sqlx::query(
            "UPDATE schedule_state SET fired_count = fired_count + 1, watch_run_id = ?, \
             retry_slot_at = NULL, retry_attempt = NULL, retry_at = NULL WHERE dot_id = ?",
        )
        .bind(&run_id)
        .bind(dot_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let run = self.get_run(&run_id).await?;
        Ok(if joined {
            SlotStart::Coalesced(run)
        } else {
            SlotStart::Enqueued(run)
        })
    }

    /// Starts the pending retry, in one transaction (spec §6): it joins the dot's queued run
    /// if there is one, and `new` (which names the failed run in `retry_of`) is inserted
    /// otherwise. The run becomes the watched run and the retry fields are cleared; a retry
    /// does not count. `Conflict`, and nothing is written, when no retry is pending.
    pub async fn start_retry(
        &self,
        dot_id: &str,
        new: &NewRun,
        now: DateTime<Utc>,
    ) -> Result<SlotStart> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE schedule_state SET retry_slot_at = NULL, retry_attempt = NULL, \
             retry_at = NULL, updated_at = ? WHERE dot_id = ? AND retry_at IS NOT NULL",
        )
        .bind(timestamp(now))
        .bind(dot_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "the schedule of dot {dot_id} has no pending retry"
            )));
        }
        let (run_id, joined) = start_or_join(&mut tx, new).await?;
        sqlx::query("UPDATE schedule_state SET watch_run_id = ? WHERE dot_id = ?")
            .bind(&run_id)
            .bind(dot_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let run = self.get_run(&run_id).await?;
        Ok(if joined {
            SlotStart::Coalesced(run)
        } else {
            SlotStart::Enqueued(run)
        })
    }

    /// Writes the fields `change` names, in one statement, and returns the row. `None` when
    /// nothing was written: the dot has no row, or the row does not watch
    /// `change.if_watching`.
    pub async fn update_schedule_state(
        &self,
        dot_id: &str,
        change: &ScheduleChange,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduleState>> {
        let retry = change.retry.as_ref().map(Option::as_ref);
        let row = sqlx::query(&format!(
            "UPDATE schedule_state SET \
             status = CASE WHEN ? THEN ? ELSE status END, \
             watch_run_id = CASE WHEN ? THEN ? ELSE watch_run_id END, \
             retry_slot_at = CASE WHEN ? THEN ? ELSE retry_slot_at END, \
             retry_attempt = CASE WHEN ? THEN ? ELSE retry_attempt END, \
             retry_at = CASE WHEN ? THEN ? ELSE retry_at END, \
             attention_run_id = CASE WHEN ? THEN ? ELSE attention_run_id END, \
             updated_at = ? \
             WHERE dot_id = ? AND (? IS NULL OR watch_run_id = ?) \
             RETURNING {STATE_COLUMNS}"
        ))
        .bind(change.status.is_some())
        .bind(change.status.map(|s| s.as_str()))
        .bind(change.watch_run_id.is_some())
        .bind(change.watch_run_id.clone().flatten())
        .bind(retry.is_some())
        .bind(retry.flatten().map(|r| r.slot_at.clone()))
        .bind(retry.is_some())
        .bind(retry.flatten().map(|r| r.attempt as i64))
        .bind(retry.is_some())
        .bind(retry.flatten().map(|r| r.at.clone()))
        .bind(change.attention_run_id.is_some())
        .bind(change.attention_run_id.clone().flatten())
        .bind(timestamp(now))
        .bind(dot_id)
        .bind(&change.if_watching)
        .bind(&change.if_watching)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(state_from_row).transpose()
    }
}
```

**In `crates/dots-core/src/events.rs`, replace:**

```rust
        approval: Approval,
    },
}

```

**with:**

```rust
        approval: Approval,
    },
    /// The dot's schedule row (`schedule_state`) was made, changed or deleted.
    ScheduleUpdated {
        dot_id: String,
    },
}

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    }

    pub async fn create_dot(&self, spec: DotSpec) -> Result<Dot> {
        let dot = self.store.create_dot(&spec).await?;
        self.scheduler.reload();
        Ok(dot)
```

**with:**

```rust
    }

    /// Brings the dot's schedule row in line with the dot after a change of it (Scheduler v2
    /// spec §4): made for a new schedule, reset for a changed one or when `enabled_now` (the
    /// dot was disabled and is enabled now), deleted without a schedule. A change of the row
    /// is published as `ScheduleUpdated`.
    async fn sync_schedule(&self, dot: &Dot, enabled_now: bool) -> Result<()> {
        let schedule = dot.spec.schedule.as_ref();
        if self
            .store
            .sync_schedule_state(&dot.id, schedule, enabled_now, Utc::now())
            .await?
        {
            self.schedule_updated(&dot.id);
        }
        Ok(())
    }

    fn schedule_updated(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::ScheduleUpdated {
            dot_id: dot_id.to_string(),
        });
    }

    pub async fn create_dot(&self, spec: DotSpec) -> Result<Dot> {
        let dot = self.store.create_dot(&spec).await?;
        self.sync_schedule(&dot, false).await?;
        self.scheduler.reload();
        Ok(dot)
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    pub async fn update_dot(&self, id: &str, spec: DotSpec) -> Result<Dot> {
        let dot = self.store.update_dot(id, &spec).await?;
        self.scheduler.reload();
        Ok(dot)
    }

    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        self.store.delete_dot_if_idle(id).await?;
        self.scheduler.reload();
        Ok(())
```

**with:**

```rust

    pub async fn update_dot(&self, id: &str, spec: DotSpec) -> Result<Dot> {
        let was_enabled = self.store.get_dot(id).await?.spec.enabled;
        let dot = self.store.update_dot(id, &spec).await?;
        self.sync_schedule(&dot, dot.spec.enabled && !was_enabled)
            .await?;
        self.scheduler.reload();
        Ok(dot)
    }

    /// Deletes the dot; its schedule row goes with it (`ON DELETE CASCADE`).
    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        let had_row = self.store.get_schedule_state(id).await?.is_some();
        self.store.delete_dot_if_idle(id).await?;
        if had_row {
            self.schedule_updated(id);
        }
        self.scheduler.reload();
        Ok(())
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
        let dot = self.store.set_dot_enabled(id, enabled).await?;
        self.scheduler.reload();
        Ok(dot)
```

**with:**

```rust

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
        let was_enabled = self.store.get_dot(id).await?.spec.enabled;
        let dot = self.store.set_dot_enabled(id, enabled).await?;
        self.sync_schedule(&dot, enabled && !was_enabled).await?;
        self.scheduler.reload();
        Ok(dot)
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        ended_at: Some(TS.into()),
        reviewed_at: None,
    }
}
```

**with:**

```rust
        ended_at: Some(TS.into()),
        reviewed_at: None,
        retry_of: None,
    }
}
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
                ..approval()
            }}),
        ],
        // What `approval_rule_candidates` answers for the `Approval` sample.
```

**with:**

```rust
                ..approval()
            }}),
            to(RuntimeEvent::ScheduleUpdated { dot_id: "01J0DOT".into() }),
        ],
        // What `approval_rule_candidates` answers for the `Approval` sample.
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
            ended_at: None,
            reviewed_at: None,
        }
    }
```

**with:**

```rust
            ended_at: None,
            reviewed_at: None,
            retry_of: None,
        }
    }
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust

    #[test]
    fn notification_texts() {
        let ended = |s| Notice::RunEnded {
```

**with:**

```rust

    #[test]
    fn a_schedule_update_does_not_notify_but_may_change_the_inbox() {
        let mut n = Notifier::default();
        let ev = RuntimeEvent::ScheduleUpdated { dot_id: "d".into() };
        assert_eq!(n.decide(&ev), None);
        // The row holds "Needs attention", which the Inbox lists.
        assert!(affects_inbox(&ev));
    }

    #[test]
    fn notification_texts() {
        let ended = |s| Notice::RunEnded {
```

**In `ui/src/api/types.ts`, replace:**

```ts
  ended_at: nullable(z.string()),
  reviewed_at: nullable(z.string()),
};
export const Run = z.strictObject(runShape);
```

**with:**

```ts
  ended_at: nullable(z.string()),
  reviewed_at: nullable(z.string()),
  // The failed scheduled run that this run retries.
  retry_of: nullable(z.string()),
};
export const Run = z.strictObject(runShape);
```

**In `ui/src/api/types.ts`, replace:**

```ts
  z.strictObject({ type: z.literal("approval_decided"), approval: Approval }),
  z.strictObject({ type: z.literal("approval_updated"), approval: Approval }),
]);

```

**with:**

```ts
  z.strictObject({ type: z.literal("approval_decided"), approval: Approval }),
  z.strictObject({ type: z.literal("approval_updated"), approval: Approval }),
  // The dot's schedule row was made, changed or deleted.
  z.strictObject({ type: z.literal("schedule_updated"), dot_id: z.string() }),
]);

```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      invalidate(qc, keys.approvals(event.approval.run_id));
      break;
  }
}
```

**with:**

```ts
      invalidate(qc, keys.approvals(event.approval.run_id));
      break;
    case "schedule_updated":
      // Where the dot's schedule stands: its next run, retry and "Needs attention", which the
      // Dots list and the Inbox show.
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
  }
}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
made), `status` (`active` | `completed` | `expired`), `last_slot_at`, `fired_count`,
`watch_run_id`, `retry_slot_at`, `retry_attempt`, `retry_at`, `attention_run_id` (the run
columns are FKs to `runs`, ON DELETE SET NULL), `updated_at`.

**settings** — key/value, each value a JSON document: `port` (default 47321),
```

**with:**

```markdown
made), `status` (`active` | `completed` | `expired`), `last_slot_at`, `fired_count`,
`watch_run_id`, `retry_slot_at`, `retry_attempt`, `retry_at`, `attention_run_id` (the run
columns are FKs to `runs`, ON DELETE SET NULL), `updated_at`. The runtime keeps the row in line
with its dot: creating a dot with a schedule makes it; changing the schedule (its JSON differs
from `rule_key`) or enabling a disabled dot resets it (`active`, `last_slot_at` = now, no count,
watched run, retry or attention); removing the schedule deletes it, and so does deleting the dot.
Disabling a dot leaves its row as it is. Each change of a row is published as `ScheduleUpdated`.

**settings** — key/value, each value a JSON document: `port` (default 47321),
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - From Approvals v2 there is a fifth event, `ApprovalUpdated`: a decided approval whose call
    Dots performs changed its `apply_state`. It never notifies.
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
```

**with:**

```markdown
  - From Approvals v2 there is a fifth event, `ApprovalUpdated`: a decided approval whose call
    Dots performs changed its `apply_state`. It never notifies.
  - From Scheduler v2 there is a sixth event, `ScheduleUpdated { dot_id }`: the dot's
    `schedule_state` row was made, changed or deleted. It updates the tray's Inbox count.
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    event lists, deduplicated by `seq`. `ApprovalUpdated` invalidates `inbox` and the
    `approvals` of the approval's own run, which is the parent of the run that performs the
    call: that run's own `RunUpdated` does not reach those approvals.
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
```

**with:**

```markdown
    event lists, deduplicated by `seq`. `ApprovalUpdated` invalidates `inbox` and the
    `approvals` of the approval's own run, which is the parent of the run that performs the
    call: that run's own `RunUpdated` does not reach those approvals. `ScheduleUpdated`
    invalidates `dots`, `overview` and `inbox`.
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  variants, so a new variant without a sample does not compile. From Approvals v2 it also
  holds a sample of `ApprovalUpdated`, every `ApplyState`, and the rule candidates the core
  derives for the `Approval` sample (`RuleCandidate`).
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
```

**with:**

```markdown
  variants, so a new variant without a sample does not compile. From Approvals v2 it also
  holds a sample of `ApprovalUpdated`, every `ApplyState`, and the rule candidates the core
  derives for the `Approval` sample (`RuleCandidate`). From Scheduler v2 it holds a sample of
  every `RuntimeEvent` type, `ScheduleUpdated` among them, and the UI test checks that.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. Every run in the fixture gains `"retry_of": null`, and `RuntimeEvent` gains a `schedule_updated` sample.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test runtime_schedule_test --test store_schedules_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/contract.test.ts src/lib/runtime-events.test.ts 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `runtime_schedule_test` 2 passed, `store_schedules_test` 13 passed; `dots-app` 48 passed (the new `events::tests::a_schedule_update_does_not_notify_but_may_change_the_inbox` among them); both UI files pass.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 385 passed, `dots-app` 48 passed, no clippy line, `cargo fmt --check` silent, UI 253 passed in 20 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs apps/dots-app/src/events.rs crates/dots-core/src/events.rs crates/dots-core/src/model.rs crates/dots-core/src/runtime.rs crates/dots-core/src/schedule.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/runs.rs crates/dots-core/src/store/schedules.rs crates/dots-core/src/util.rs crates/dots-core/tests/common/mod.rs crates/dots-core/tests/runtime_schedule_test.rs crates/dots-core/tests/store_schedules_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/lib/runtime-events.test.ts ui/src/lib/runtime-events.ts
git commit -m "feat(core): a schedule's state row, kept in line with its dot"
```

### Task 4: The scheduler loop of Scheduler v2, on a clock tests can move

The loop is rewritten as spec section 5 describes it, without judging, which is Task 5. It takes a clock, so that the tests set the time by hand and the whole file runs in well under a second. Each pass reads the rows of enabled dots and, for an active row, finds the newest due slot: on time (less than 60 s late) it fires, missed it fires once as a catch-up when catch-up is on and the slot is less than 24 h old, and otherwise it is skipped; either way the row records it in one guarded transaction. A bound or a schedule without a slot left completes the row, a schedule that no longer parses expires it. The loop sleeps until the next slot, a waiting retry or 60 s, whichever is first; a reload wakes it. A resume of paused schedules moves every active row to now, so a pause catches up nothing. The rows of dots upgraded by migration 0005 are made at start, before the first pass. The main spec's module list, section 4 step 1 and the pause passages follow.

**Files:**
- Create: `crates/dots-core/tests/common/clock.rs`
- Modify: `crates/dots-core/src/runner.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/scheduler.rs`, `crates/dots-core/src/store/schedules.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`
- Test: `crates/dots-core/tests/common/clock.rs`, `crates/dots-core/tests/common/mod.rs`, `crates/dots-core/tests/runtime_schedule_test.rs`, `crates/dots-core/tests/scheduler_test.rs`, `crates/dots-core/tests/store_schedules_test.rs`

**Interfaces:**
- Consumes: Task 1's `due_slot` and `next_slot`, Task 3's store methods and `ScheduleUpdated`.
- Produces:
  - In `dots_core::scheduler`: `ON_TIME` (60 s), `CATCH_UP_WINDOW` (24 h), `MAX_SLEEP` (60 s), `ERROR_WAIT` (30 s), all `TimeDelta`. `parse_cron` stays here.
  - `trait Clock: Send + Sync + Debug { fn now(&self) -> DateTime<Utc>; async fn sleep_until(&self, deadline: DateTime<Utc>); }` (`async_trait`); `SystemClock` (`Utc::now()` to the millisecond, a tokio sleep).
  - `Scheduler::new(store: Store, runner: Arc<Runner>, bus: Bus, clock: Arc<dyn Clock>) -> Arc<Scheduler>`; `set_paused(paused)` records a resume on a change from paused to running and reloads; `is_paused`, `reload`, `spawn(shutdown)` as before.
  - A pass, per active row of an enabled dot with a schedule: `due_slot(schedule, last_slot_at, now, &Local)`; it fires when the count is not reached and the slot is on time, or is caught up (catch-up on, less than 24 h late); otherwise it is skipped. `record_slot`: `Enqueued` is announced through `Runner::enqueued`, `Coalesced` and `Skipped` are logged, each publishes `ScheduleUpdated`, a `Conflict` (the row was reset meanwhile) ends the row's turn quietly. The run's payload is `{"scheduled_for": <slot, local RFC 3339>}`, plus `"caught_up": true` for a slot 60 s or more late. Then the status: `completed` when no slot is left (or the count is reached) and no retry waits; `expired` with a warning when the slots cannot be computed. A slot after a reached count is recorded and not fired. A store error makes the loop wait `ERROR_WAIT`.
  - The wake time: the earliest next slot of the active rows, a waiting `retry_at` that is still in the future, and `now + MAX_SLEEP`.
  - The first turn after a resume calls `Store::resume_schedules(now) -> Result<Vec<String>>` (one `UPDATE … WHERE status = 'active' AND last_slot_at < now RETURNING dot_id`) and announces each row. While paused no pass runs.
  - `Runner::enqueued(&self, run: &Run)`: announces the run and wakes the dispatcher, for a run inserted outside `Runner::enqueue`, which now uses it too.
  - `Config.clock: Arc<dyn Clock>` (`SystemClock` by default); `Runtime::start` syncs the row of every dot before the scheduler starts, and the runtime's row writes and `dot_overview` take their time from the clock.
- Test helpers: `tests/common/clock.rs` `FakeClock` (`new(t) -> Arc<FakeClock>`, `set(t)`, `nth_sleep(n).await -> deadline`, which waits up to 10 real seconds for the n-th sleep to begin, `sleeps()`). In `scheduler_test.rs` a `Harness` (clock at 2030-01-07T09:00Z, a runner without a dispatcher, so runs stay `queued`) with `dot`, `start`, `settle`, `at`, `row`, `runs`, `events` and `raw`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/common/clock.rs`:**

```rust
//! A clock the test moves by hand, for the scheduler (Scheduler v2 spec §5, "Clock"). Time
//! stands still until `set` moves it; a sleep ends when the time reaches its deadline. Each
//! sleep is counted when it begins, which is when the scheduler's pass is over, so a test
//! waits for the `n`th sleep to know that the loop is idle again.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dots_core::scheduler::Clock;
use tokio::sync::watch;

#[derive(Debug, Clone)]
struct State {
    now: DateTime<Utc>,
    /// Sleeps begun so far.
    sleeps: u64,
    /// The deadline of the newest sleep.
    deadline: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct FakeClock {
    state: watch::Sender<State>,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>) -> Arc<FakeClock> {
        Arc::new(FakeClock {
            state: watch::Sender::new(State {
                now,
                sleeps: 0,
                deadline: None,
            }),
        })
    }

    /// Moves the time to `t`; the sleeps whose deadline it reaches end.
    pub fn set(&self, t: DateTime<Utc>) {
        self.state.send_modify(|s| s.now = t);
    }

    /// Waits (at most 10 real seconds) until `n` sleeps have begun, and returns the deadline
    /// of the newest one.
    pub async fn nth_sleep(&self, n: u64) -> DateTime<Utc> {
        let mut rx = self.state.subscribe();
        let state = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|s| s.sleeps >= n))
            .await
            .unwrap_or_else(|_| panic!("the scheduler did not begin sleep {n}"))
            .unwrap()
            .clone();
        state.deadline.unwrap()
    }

    pub fn sleeps(&self) -> u64 {
        self.state.borrow().sleeps
    }
}

#[async_trait]
impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.state.borrow().now
    }

    async fn sleep_until(&self, deadline: DateTime<Utc>) {
        let mut rx = self.state.subscribe();
        self.state.send_modify(|s| {
            s.sleeps += 1;
            s.deadline = Some(deadline);
        });
        let _ = rx.wait_for(|s| s.now >= deadline).await;
    }
}
```

**In `crates/dots-core/tests/common/mod.rs`, replace:**

```rust
#![allow(dead_code)]

pub mod tz;

```

**with:**

```rust
#![allow(dead_code)]

pub mod clock;
pub mod tz;

```

**In `crates/dots-core/tests/runtime_schedule_test.rs`, replace:**

```rust
use std::path::PathBuf;

use chrono::Utc;
use common::spec;
use dots_core::events::RuntimeEvent;
```

**with:**

```rust
use std::path::PathBuf;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::events::RuntimeEvent;
```

**In `crates/dots-core/tests/runtime_schedule_test.rs`, replace:**

```rust
};
use dots_core::schedule::Schedule;
use dots_core::util::now;
use dots_core::{Config, Runtime};
use serde_json::json;
```

**with:**

```rust
};
use dots_core::schedule::Schedule;
use dots_core::store::{Store, DB_FILE};
use dots_core::util::{now, timestamp};
use dots_core::{Config, Runtime};
use serde_json::json;
```

**In `crates/dots-core/tests/runtime_schedule_test.rs`, replace:**

```rust
use tokio::sync::broadcast::Receiver;

async fn start(dir: &TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    Runtime::start(cfg).await.unwrap()
}

```

**with:**

```rust
use tokio::sync::broadcast::Receiver;

fn config(dir: &TempDir) -> Config {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg
}

async fn start(dir: &TempDir) -> Runtime {
    Runtime::start(config(dir)).await.unwrap()
}

```

**Append to `crates/dots-core/tests/runtime_schedule_test.rs`:**

```rust

/// The runtime writes the times of a schedule's row from its clock: at start, where it makes
/// the row of a dot upgraded by migration 0005, on a change of the dot, and on a resume after
/// a pause, which does not catch up the slots of the pause.
#[tokio::test]
async fn the_runtime_takes_the_times_of_its_rows_from_its_clock() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = DateTime::parse_from_rfc3339("2030-01-07T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    // A dot from before the upgrade: a schedule, no row.
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let store = Store::open(&data.join(DB_FILE)).await.unwrap();
    let old = store
        .create_dot(&folder_spec(&dir, "old", Some(every(600))))
        .await
        .unwrap();
    store.close().await;

    let clock = FakeClock::new(t0);
    let mut cfg = config(&dir);
    cfg.clock = clock.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    assert_eq!(row(&rt, &old).await.unwrap().last_slot_at, timestamp(t0));

    let dot = rt
        .create_dot(folder_spec(&dir, "new", Some(every(600))))
        .await
        .unwrap();
    assert_eq!(row(&rt, &dot).await.unwrap().last_slot_at, timestamp(t0));

    rt.set_schedules_paused(true).await.unwrap();
    let resumed = t0 + TimeDelta::hours(30);
    clock.set(resumed);
    rt.set_schedules_paused(false).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while row(&rt, &dot).await.unwrap().last_slot_at != timestamp(resumed) {
        assert!(
            std::time::Instant::now() < deadline,
            "the resume did not move the row"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        row(&rt, &old).await.unwrap().last_slot_at,
        timestamp(resumed)
    );
    assert!(rt.list_runs(None, 10).await.unwrap().is_empty());
    rt.shutdown();
}
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::{EngineKind, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
use tokio_util::sync::CancellationToken;

/// Runner without a dispatcher: scheduled runs stay queued, which makes coalescing visible.
fn idle_runner(store: &Store, dir: &tempfile::TempDir) -> Arc<Runner> {
    Runner::new(
        store.clone(),
        new_bus(),
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
```

**with:**

```rust
use std::time::Duration;

use chrono::{DateTime, Local, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{
    Dot, EngineKind, PendingRetry, Run, ScheduleChange, ScheduleState, ScheduleStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

/// The fake clock starts here. It is years ahead, so a `Once` or a stop date written relative
/// to it is in the future when the store checks it on save.
const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// `secs` seconds after `T0`.
fn after(secs: i64) -> DateTime<Utc> {
    t0() + TimeDelta::seconds(secs)
}

/// `after(secs)` as the store writes it.
fn ts(secs: i64) -> String {
    timestamp(after(secs))
}

/// `after(secs)` as a local date and time (`Once.at`, `Until::Date`); `secs` is whole minutes.
fn local(secs: i64) -> String {
    after(secs)
        .with_timezone(&Local)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

/// The payload of a run that serves the slot at `after(secs)`.
fn slot_payload(secs: i64, caught_up: bool) -> Value {
    let mut payload = json!({ "scheduled_for": after(secs).with_timezone(&Local).to_rfc3339() });
    if caught_up {
        payload["caught_up"] = json!(true);
    }
    payload
}

fn schedule(v: Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

/// Every `minutes`, with slots at `after(offset)` and every `minutes` from there.
fn every(minutes: u32, offset: i64, catch_up: bool) -> Schedule {
    schedule(json!({
        "rule": {"kind": "every", "minutes": minutes, "anchor": ts(offset)},
        "catch_up": catch_up,
        "until": null
    }))
}

/// Runner without a dispatcher: scheduled runs stay queued, which makes coalescing visible.
fn idle_runner(store: &Store, dir: &TempDir, bus: Bus) -> Arc<Runner> {
    Runner::new(
        store.clone(),
        bus,
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
}

#[tokio::test]
async fn every_second_schedule_enqueues_once_and_coalesces() {
    let (dir, store) = temp_store().await;
    let mut s = spec("tick");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = store.create_dot(&s).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(2600)).await;
    let runs = store.list_runs(Some(&dot.id), 10).await.unwrap();
    assert_eq!(
        runs.len(),
        1,
        "later ticks must coalesce into the queued run"
    );
    assert_eq!(runs[0].trigger, TriggerKind::Schedule);
    assert!(runs[0].payload.as_ref().unwrap()["scheduled_for"].is_string());
}

#[tokio::test]
async fn disabled_and_unscheduled_dots_are_ignored() {
    let (dir, store) = temp_store().await;
    let mut off = spec("off");
    off.schedule = Some(Schedule::cron("* * * * * *"));
    off.enabled = false;
    store.create_dot(&off).await.unwrap();
    store.create_dot(&spec("manual-only")).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

/// The loop takes its slots from the schedule module, so a stop date is kept.
#[tokio::test]
async fn a_schedule_past_its_stop_date_fires_nothing() {
    let (dir, store) = temp_store().await;
    let mut s = spec("stopped");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = store.create_dot(&s).await.unwrap();
    // The stop date passed (written behind the store's back: a past date is refused on save).
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE dots SET schedule = ? WHERE id = ?")
        .bind(
            r#"{"rule":{"kind":"cron","expr":"* * * * * *"},"catch_up":false,
               "until":{"date":"2020-01-01T09:00"}}"#,
        )
        .bind(&dot.id)
        .execute(&pool)
        .await
        .unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn reload_picks_up_new_schedules() {
    let (dir, store) = temp_store().await;
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut s = spec("late");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = store.create_dot(&s).await.unwrap();
    sched.reload();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while store.list_runs(Some(&dot.id), 10).await.unwrap().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "no scheduled run after reload"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn fire_coalesces_when_a_run_is_already_queued() {
    let (dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("f")).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    let now = chrono::Local::now();
    assert!(sched.fire(&dot, now).await.unwrap().is_some());
    assert!(sched.fire(&dot, now).await.unwrap().is_none());
}
```

**with:**

```rust
}

/// A scheduler on a fake clock at `T0`, not started yet.
struct Harness {
    dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    sched: Arc<Scheduler>,
    events: Receiver<RuntimeEvent>,
    shutdown: CancellationToken,
    sleeps: u64,
}

impl Harness {
    async fn new() -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let sched = Scheduler::new(store.clone(), runner, bus.clone(), clock.clone());
        Harness {
            events: bus.subscribe(),
            dir,
            store,
            clock,
            sched,
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
    }

    /// A dot with `schedule` whose row was made at `after(made)`, as the runtime makes it.
    async fn dot(&self, name: &str, schedule: Option<Schedule>, made: i64) -> Dot {
        let mut s = spec(name);
        s.schedule = schedule;
        let dot = self.store.create_dot(&s).await.unwrap();
        self.store
            .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, after(made))
            .await
            .unwrap();
        dot
    }

    fn start(&self) {
        self.sched.spawn(self.shutdown.clone());
    }

    /// Waits until the loop sleeps again after its pass, and returns when it means to wake,
    /// in seconds after `T0`.
    async fn settle(&mut self) -> i64 {
        self.sleeps += 1;
        (self.clock.nth_sleep(self.sleeps).await - t0()).num_seconds()
    }

    /// Moves the clock to `after(secs)`, which must reach the loop's wake time, and waits for
    /// the pass that follows.
    async fn at(&mut self, secs: i64) -> i64 {
        self.clock.set(after(secs));
        self.settle().await
    }

    async fn row(&self, dot: &Dot) -> ScheduleState {
        self.store
            .get_schedule_state(&dot.id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn runs(&self, dot: &Dot) -> Vec<Run> {
        self.store.list_runs(Some(&dot.id), 10).await.unwrap()
    }

    /// The dots named by `ScheduleUpdated` and the runs announced by `RunUpdated` since the
    /// last call.
    fn events(&mut self) -> (Vec<String>, Vec<String>) {
        let (mut schedules, mut runs) = (Vec::new(), Vec::new());
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                RuntimeEvent::ScheduleUpdated { dot_id } => schedules.push(dot_id),
                RuntimeEvent::RunUpdated { run } => runs.push(run.id),
                _ => {}
            }
        }
        (schedules, runs)
    }

    /// Writes the row behind the store's back, for a state only Task 5's judging makes.
    async fn raw(&self, sql: &str, dot: &Dot) {
        let pool = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            self.dir.path().join("dots.db").display()
        ))
        .await
        .unwrap();
        sqlx::query(sql).bind(&dot.id).execute(&pool).await.unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn a_disabled_dot_and_a_dot_without_a_row_are_left_alone() {
    let mut h = Harness::new().await;
    // Disabled, with a row from an hour ago and missed slots.
    let mut off = spec("off");
    off.schedule = Some(every(10, 0, true));
    off.enabled = false;
    let off = h.store.create_dot(&off).await.unwrap();
    h.store
        .sync_schedule_state(&off.id, off.spec.schedule.as_ref(), false, after(-3600))
        .await
        .unwrap();
    let off_row = h.row(&off).await;
    // A schedule without a row: the runtime makes the rows (at start and on each change).
    let mut s = spec("no-row");
    s.schedule = Some(every(10, -3600, true));
    let no_row = h.store.create_dot(&s).await.unwrap();

    h.start();
    assert_eq!(
        h.settle().await,
        60,
        "nothing is due: the loop sleeps 60 s at most"
    );
    assert_eq!(h.row(&off).await, off_row);
    assert!(h
        .store
        .get_schedule_state(&no_row.id)
        .await
        .unwrap()
        .is_none());
    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
    assert_eq!(h.events(), (vec![], vec![]));
}

#[tokio::test]
async fn a_slot_fires_on_time_and_a_later_one_joins_the_queued_run() {
    let mut h = Harness::new().await;
    let dot = h.dot("tick", Some(every(10, 30, true)), 0).await;
    h.start();
    assert_eq!(h.settle().await, 30, "the loop sleeps until the slot");
    h.events();

    assert_eq!(
        h.at(30).await,
        90,
        "the next slot is 10 min away: 60 s at most"
    );
    let runs = h.runs(&dot).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].trigger, TriggerKind::Schedule);
    assert_eq!(runs[0].payload, Some(slot_payload(30, false)));
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(30));
    assert_eq!(row.fired_count, 1);
    assert_eq!(row.watch_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(
        h.events(),
        (vec![dot.id.clone()], vec![runs[0].id.clone()]),
        "the row change and the new run are announced"
    );

    // The runner is idle, so the run is still queued at the next slot: the slot joins it.
    assert_eq!(h.at(630).await, 690);
    assert_eq!(h.runs(&dot).await.len(), 1);
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(630));
    assert_eq!(row.fired_count, 2);
    assert_eq!(row.watch_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(h.events(), (vec![dot.id.clone()], vec![]));
}

#[tokio::test]
async fn a_slot_up_to_59_seconds_late_is_on_time_and_60_seconds_is_missed() {
    let mut h = Harness::new().await;
    let dot = h.dot("strict", Some(every(10, 0, false)), 0).await;
    h.start();
    h.settle().await;

    assert_eq!(h.at(659).await, 719);
    let runs = h.runs(&dot).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].payload, Some(slot_payload(600, false)));

    // 60 s late without catch-up: skipped, but handled.
    h.at(1260).await;
    assert_eq!(h.runs(&dot).await.len(), 1);
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(1200));
    assert_eq!(row.fired_count, 1);
}

#[tokio::test]
async fn only_the_newest_missed_slot_is_caught_up_once() {
    let mut h = Harness::new().await;
    let dot = h.dot("hourly", Some(every(60, 0, true)), 0).await;
    // Dots was closed from T0 until 5 h 10 min later: five hourly slots were missed.
    h.clock.set(after(5 * 3600 + 600));
    h.start();
    h.settle().await;

    let runs = h.runs(&dot).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].payload, Some(slot_payload(5 * 3600, true)));
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(5 * 3600));
    assert_eq!(row.fired_count, 1);
}

#[tokio::test]
async fn a_missed_slot_is_skipped_without_catch_up_or_when_a_day_old() {
    let mut h = Harness::new().await;
    let no_catch_up = h.dot("no-catch-up", Some(every(60, 0, false)), 0).await;
    let once = schedule(json!({
        "rule": {"kind": "once", "at": local(3600)},
        "catch_up": true,
        "until": null
    }));
    let once = h.dot("once", Some(once), 0).await;
    // The once slot is 24 h and 60 s old when Dots starts.
    h.clock.set(after(25 * 3600 + 60));
    h.start();
    h.settle().await;

    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
    let row = h.row(&no_catch_up).await;
    assert_eq!(row.last_slot_at, ts(25 * 3600));
    assert_eq!((row.status, row.fired_count), (ScheduleStatus::Active, 0));
    let row = h.row(&once).await;
    assert_eq!(row.last_slot_at, ts(3600));
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 0),
        "completed (missed): no run"
    );
}

#[tokio::test]
async fn once_is_completed_when_its_slot_fired() {
    let mut h = Harness::new().await;
    let once = schedule(json!({
        "rule": {"kind": "once", "at": local(60)},
        "catch_up": true,
        "until": null
    }));
    let dot = h.dot("once", Some(once), 0).await;
    h.start();
    assert_eq!(h.settle().await, 60);
    assert_eq!(h.at(60).await, 120, "no slot is left: 60 s at most");
    assert_eq!(h.runs(&dot).await.len(), 1);
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 1)
    );
    assert_eq!(h.events().0, vec![dot.id.clone(), dot.id.clone()]);

    // A completed schedule fires nothing.
    h.at(3600).await;
    assert_eq!(h.row(&dot).await, row);
}

#[tokio::test]
async fn a_count_completes_the_schedule_when_it_is_reached() {
    let mut h = Harness::new().await;
    let mut twice = every(1, 0, true);
    twice.until = Some(dots_core::schedule::Until::Count(2));
    let dot = h.dot("twice", Some(twice), 0).await;
    h.start();
    h.settle().await;

    h.at(60).await;
    let row = h.row(&dot).await;
    assert_eq!((row.status, row.fired_count), (ScheduleStatus::Active, 1));
    h.at(120).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 2)
    );

    h.at(300).await;
    assert_eq!(h.row(&dot).await, row);
    assert_eq!(
        h.runs(&dot).await.len(),
        1,
        "the second slot joined the queued run"
    );
}

/// The count is reached, but a retry of the last occurrence still waits: the schedule stays
/// active until no retry can follow (Task 5 judges and retries; here the row is written by
/// hand). The loop wakes for the retry.
#[tokio::test]
async fn a_reached_count_waits_for_a_pending_retry() {
    let mut h = Harness::new().await;
    let mut once_more = every(1, 0, true);
    once_more.until = Some(dots_core::schedule::Until::Count(1));
    let dot = h.dot("retrying", Some(once_more), 0).await;
    h.raw(
        &format!(
            "UPDATE schedule_state SET fired_count = 1, retry_slot_at = '{}', \
             retry_attempt = 2, retry_at = '{}' WHERE dot_id = ?",
            ts(0),
            ts(40)
        ),
        &dot,
    )
    .await;
    h.start();
    assert_eq!(h.settle().await, 40, "the loop wakes for the retry");
    let row = h.row(&dot).await;
    assert_eq!(row.status, ScheduleStatus::Active);
    assert_eq!(
        row.retry,
        Some(PendingRetry {
            slot_at: ts(0),
            attempt: 2,
            at: ts(40)
        })
    );

    // No retry left: the schedule is completed.
    let cleared = ScheduleChange {
        retry: Some(None),
        ..Default::default()
    };
    h.store
        .update_schedule_state(&dot.id, &cleared, after(0))
        .await
        .unwrap();
    h.sched.reload();
    h.settle().await;
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Completed);
    assert!(h.runs(&dot).await.is_empty());
}

#[tokio::test]
async fn a_stop_date_completes_the_schedule_after_its_last_slot() {
    let mut h = Harness::new().await;
    let mut until = every(1, 0, true);
    until.until = Some(serde_json::from_value(json!({ "date": local(60) })).unwrap());
    let dot = h.dot("until", Some(until), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 1)
    );
    assert_eq!(h.runs(&dot).await.len(), 1);
}

#[tokio::test]
async fn a_schedule_that_no_longer_parses_expires() {
    let mut h = Harness::new().await;
    let dot = h.dot("old", Some(Schedule::cron("0 0 9 * * *")), 0).await;
    // Only an old cron string a newer crate refuses can get here.
    h.raw(
        r#"UPDATE dots SET schedule = '{"rule":{"kind":"cron","expr":"not cron"},
           "catch_up":false,"until":null}' WHERE id = ?"#,
        &dot,
    )
    .await;
    h.start();
    assert_eq!(h.settle().await, 60);
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Expired);
    assert!(h.runs(&dot).await.is_empty());
}

#[tokio::test]
async fn a_resume_does_not_catch_up_the_slots_of_the_pause() {
    let mut h = Harness::new().await;
    let dot = h.dot("paused", Some(every(10, 0, true)), 0).await;
    h.start();
    h.settle().await;
    h.events();

    h.sched.set_paused(true);
    // Three slots pass while paused.
    h.clock.set(after(35 * 60));
    h.sched.set_paused(false);
    assert_eq!(h.settle().await, 35 * 60 + 60);

    assert!(h.runs(&dot).await.is_empty());
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(35 * 60), "the resume time");
    assert_eq!(row.fired_count, 0);
    assert_eq!(h.events().0, vec![dot.id.clone()]);

    // The slots after the resume fire as usual.
    h.at(40 * 60).await;
    assert_eq!(h.runs(&dot).await.len(), 1);
}

#[tokio::test]
async fn a_scheduler_started_paused_does_nothing_until_resumed() {
    let mut h = Harness::new().await;
    let dot = h.dot("hourly", Some(every(60, 0, true)), 0).await;
    h.clock.set(after(3600 + 300));
    h.sched.set_paused(true);
    h.start();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.clock.sleeps(), 0, "no pass while paused");
    assert_eq!(h.row(&dot).await.last_slot_at, T0);

    h.sched.set_paused(false);
    h.settle().await;
    assert!(
        h.runs(&dot).await.is_empty(),
        "the missed slot is not caught up"
    );
    assert_eq!(h.row(&dot).await.last_slot_at, ts(3600 + 300));
}

#[tokio::test]
async fn reload_wakes_the_loop_for_a_new_schedule() {
    let mut h = Harness::new().await;
    h.start();
    assert_eq!(h.settle().await, 60);
    let dot = h.dot("late", Some(every(10, 20, true)), 0).await;
    h.sched.reload();
    assert_eq!(h.settle().await, 20, "the sleep follows the new slot");
    h.at(20).await;
    assert_eq!(h.runs(&dot).await.len(), 1);
}
```

**In `crates/dots-core/tests/store_schedules_test.rs`, replace:**

```rust
}

#[test]
fn timestamps_have_milliseconds_and_a_z() {
```

**with:**

```rust
}

#[tokio::test]
async fn a_resume_moves_every_active_row_to_now_and_keeps_the_rest() {
    let (_d, store) = temp_store().await;
    let paused = scheduled(&store, "paused").await;
    let run = failed_run(&store, &paused).await;
    busy(&store, &paused, &run).await;
    let done = scheduled(&store, "done").await;
    let complete = ScheduleChange {
        status: Some(ScheduleStatus::Completed),
        ..Default::default()
    };
    store
        .update_schedule_state(&done.id, &complete, t(T0))
        .await
        .unwrap();
    // A row reset after the resume time is not moved back.
    let later = scheduled(&store, "later").await;
    store
        .sync_schedule_state(&later.id, later.spec.schedule.as_ref(), true, t(T3))
        .await
        .unwrap();
    let (done_before, later_before) = (state(&store, &done).await, state(&store, &later).await);

    assert_eq!(
        store.resume_schedules(t(T2)).await.unwrap(),
        vec![paused.id.clone()]
    );

    // Only the time moves: the watch, the retry and the attention stay.
    let row = state(&store, &paused).await;
    assert_eq!(
        (row.last_slot_at.as_str(), row.updated_at.as_str()),
        (T2, T2)
    );
    assert_eq!(row.watch_run_id.as_deref(), Some(run.as_str()));
    assert_eq!(row.retry, Some(retry(2)));
    assert_eq!(row.attention_run_id.as_deref(), Some(run.as_str()));
    assert_eq!(state(&store, &done).await, done_before);
    assert_eq!(state(&store, &later).await, later_before);
}

#[test]
fn timestamps_have_milliseconds_and_a_z() {
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test runtime_schedule_test --test scheduler_test --test store_schedules_test 2>&1 | grep -E "^error|could not compile" | sort | uniq -c
```

Expected: the three test targets do not compile: `` error[E0432]: unresolved import `dots_core::scheduler::Clock` ``, and `could not compile` for `store_schedules_test`, `scheduler_test` and `runtime_schedule_test`, one error each.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Local};
use serde_json::json;
use tokio::sync::Notify;
```

**with:**

```rust
//! The scheduler (Scheduler v2 spec §5): one task that runs a pass over the schedule rows of
//! the enabled dots, then sleeps until the next slot, a waiting retry or 60 s, whichever is
//! first. `reload()` and a pause or resume wake it at once.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Local, SubsecRound, TimeDelta, Utc};
use serde_json::json;
use tokio::sync::Notify;
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
use tokio_util::sync::CancellationToken;

use crate::model::{Dot, NewRun, Run, TriggerKind};
use crate::runner::Runner;
use crate::schedule::next_slot;
use crate::store::Store;
use crate::{Error, Result};

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
```

**with:**

```rust
use tokio_util::sync::CancellationToken;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    Dot, NewRun, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart, TriggerKind,
};
use crate::runner::Runner;
use crate::schedule::{due_slot, next_slot, Schedule, Until};
use crate::store::Store;
use crate::{Error, Result};

/// A slot handled less than this after its time is on time; later it is missed.
pub const ON_TIME: TimeDelta = TimeDelta::seconds(60);
/// A missed slot younger than this is caught up (when the schedule says so).
pub const CATCH_UP_WINDOW: TimeDelta = TimeDelta::hours(24);
/// The longest sleep between passes. Timers do not follow the wall clock across system sleep
/// or a clock change, so the loop looks at the clock again at least this often.
pub const MAX_SLEEP: TimeDelta = TimeDelta::seconds(60);
/// The wait before a pass that failed is tried again.
pub const ERROR_WAIT: TimeDelta = TimeDelta::seconds(30);

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
}

pub struct Scheduler {
    store: Store,
    runner: Arc<Runner>,
    reload: Notify,
    last_fired: Mutex<Option<DateTime<Local>>>,
    paused: AtomicBool,
}

impl Scheduler {
    pub fn new(store: Store, runner: Arc<Runner>) -> Arc<Scheduler> {
        Arc::new(Scheduler {
            store,
            runner,
            reload: Notify::new(),
            last_fired: Mutex::new(None),
            paused: AtomicBool::new(false),
        })
    }

    /// While paused nothing fires; ticks missed meanwhile are skipped, not caught up.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        self.reload();
    }
```

**with:**

```rust
}

/// Where the scheduler and the runtime get the time. The runtime uses `SystemClock`; tests
/// use a clock they move by hand, so a day of slots passes without waiting.
#[async_trait]
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The time now, to the millisecond (the store's precision).
    fn now(&self) -> DateTime<Utc>;
    /// Returns once `now()` has reached `deadline`. The scheduler wakes it early by dropping
    /// it (on `reload()`, a pause or a resume, and at shutdown).
    async fn sleep_until(&self, deadline: DateTime<Utc>);
}

/// The PC's clock.
#[derive(Debug, Default)]
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(3)
    }

    async fn sleep_until(&self, deadline: DateTime<Utc>) {
        let wait = (deadline - Utc::now()).to_std().unwrap_or_default();
        tokio::time::sleep(wait).await;
    }
}

pub struct Scheduler {
    store: Store,
    runner: Arc<Runner>,
    bus: Bus,
    clock: Arc<dyn Clock>,
    reload: Notify,
    paused: AtomicBool,
    /// Set by a resume, cleared by the pass that follows it.
    resumed: AtomicBool,
}

impl Scheduler {
    pub fn new(
        store: Store,
        runner: Arc<Runner>,
        bus: Bus,
        clock: Arc<dyn Clock>,
    ) -> Arc<Scheduler> {
        Arc::new(Scheduler {
            store,
            runner,
            bus,
            clock,
            reload: Notify::new(),
            paused: AtomicBool::new(false),
            resumed: AtomicBool::new(false),
        })
    }

    /// While paused no pass runs: nothing fires and no retry starts. A resume does not catch
    /// up: the first pass after it moves every active row's `last_slot_at` to now.
    pub fn set_paused(&self, paused: bool) {
        if self.paused.swap(paused, Ordering::SeqCst) && !paused {
            self.resumed.store(true, Ordering::SeqCst);
        }
        self.reload();
    }
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
    }

    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    r = this.tick() => {
                        if let Err(e) = r {
                            tracing::error!("scheduler error: {e}");
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                                _ = shutdown.cancelled() => break,
                            }
                        }
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
```

**with:**

```rust
    }

    /// Runs the loop until `shutdown`. The first pass runs at once; the runtime has recovered
    /// interrupted runs before it starts the scheduler.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let wake = match this.step().await {
                    Ok(wake) => wake,
                    Err(e) => {
                        tracing::error!("scheduler error: {e}");
                        Some(this.clock.now() + ERROR_WAIT)
                    }
                };
                let sleep = async {
                    match wake {
                        Some(at) => this.clock.sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                // A reload that came during the pass is taken before the clock is asked to
                // sleep.
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = this.reload.notified() => {}
                    _ = sleep => {}
                }
            }
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
    }

    async fn tick(&self) -> Result<()> {
        if self.is_paused() {
            self.reload.notified().await;
            return Ok(());
        }
        let now = Local::now();
        let from = match *self.last_fired.lock().unwrap() {
            Some(last) if last > now => last,
            _ => now,
        };
        let mut due: Vec<(DateTime<Local>, Dot)> = Vec::new();
        for dot in self.store.list_dots().await? {
            let Some(schedule) = &dot.spec.schedule else {
                continue;
            };
            if !dot.spec.enabled {
                continue;
            }
            match next_slot(schedule, &from, &Local) {
                Ok(Some(t)) => due.push((t, dot)),
                Ok(None) => {}
                Err(e) => tracing::warn!(dot = %dot.spec.name, "bad schedule: {e}"),
            }
        }
        let Some(next) = due.iter().map(|(t, _)| *t).min() else {
            self.reload.notified().await;
            return Ok(());
        };
        let wait = (next - Local::now()).to_std().unwrap_or_default();
        tokio::select! {
            _ = tokio::time::sleep(wait) => {
                *self.last_fired.lock().unwrap() = Some(next);
                for (_, dot) in due.iter().filter(|(t, _)| *t == next) {
                    self.fire(dot, next).await?;
                }
            }
            _ = self.reload.notified() => {}
        }
        Ok(())
    }

    /// Enqueues a scheduled run; `None` when coalesced into an already queued run.
    pub async fn fire(&self, dot: &Dot, at: DateTime<Local>) -> Result<Option<Run>> {
        if self.store.has_queued_run(&dot.id).await? {
            tracing::info!(dot = %dot.spec.name, "schedule tick coalesced into the queued run");
            return Ok(None);
        }
        let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
        new.payload = Some(json!({ "scheduled_for": at.to_rfc3339() }));
        Ok(Some(self.runner.enqueue(new).await?))
    }
}
```

**with:**

```rust
    }

    /// One turn of the loop: `None` while paused (only a reload wakes it), else a pass and the
    /// time to wake for the next one.
    async fn step(&self) -> Result<Option<DateTime<Utc>>> {
        if self.is_paused() {
            return Ok(None);
        }
        let now = self.clock.now();
        if self.resumed.swap(false, Ordering::SeqCst) {
            for dot_id in self.store.resume_schedules(now).await? {
                self.updated(&dot_id);
            }
        }
        self.pass(now).await.map(Some)
    }

    /// A pass (spec §5) over the active rows of the enabled dots; returns when to wake.
    async fn pass(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let mut dots: std::collections::HashMap<String, Dot> = self
            .store
            .list_dots()
            .await?
            .into_iter()
            .map(|dot| (dot.id.clone(), dot))
            .collect();
        let mut wake = now + MAX_SLEEP;
        for state in self.store.list_schedule_states().await? {
            if state.status != ScheduleStatus::Active {
                continue;
            }
            let Some(dot) = dots.remove(&state.dot_id) else {
                continue;
            };
            let Some(schedule) = &dot.spec.schedule else {
                continue;
            };
            if let Some(next) = self.handle(&dot, schedule, state, now).await? {
                wake = wake.min(next);
            }
        }
        Ok(wake)
    }

    /// Steps 1, 2 and 5 of a pass for one row: the due slot is fired, caught up or skipped,
    /// then the status is updated. Returns the row's next wake time, `None` when it has none.
    async fn handle(
        &self,
        dot: &Dot,
        schedule: &Schedule,
        state: ScheduleState,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut last = parse_time(&state.last_slot_at)?;
        let mut fired = state.fired_count;
        let mut retry = state.retry;
        let limit = match schedule.until {
            Some(Until::Count(n)) => Some(n),
            _ => None,
        };
        let due = match due_slot(schedule, &last, &now, &Local) {
            Ok(due) => due.map(|t| t.with_timezone(&Utc)),
            Err(e) => return self.expire(dot, &e).await,
        };
        if let Some(due) = due {
            let late = now - due;
            let counted = limit.is_some_and(|n| fired >= n);
            let fire =
                !counted && (late < ON_TIME || (schedule.catch_up && late < CATCH_UP_WINDOW));
            let new = fire.then(|| scheduled_run(dot, due, late >= ON_TIME));
            match self
                .store
                .record_slot(&dot.id, due, new.as_ref(), now)
                .await
            {
                Ok(start) => {
                    last = due;
                    match start {
                        SlotStart::Skipped => {
                            tracing::info!(dot = %dot.spec.name, slot = %due, "missed slot skipped");
                        }
                        SlotStart::Enqueued(run) => {
                            fired += 1;
                            retry = None;
                            self.runner.enqueued(&run);
                        }
                        SlotStart::Coalesced(run) => {
                            fired += 1;
                            retry = None;
                            tracing::info!(
                                dot = %dot.spec.name, run = %run.id,
                                "schedule slot coalesced into the queued run"
                            );
                        }
                    }
                    self.updated(&dot.id);
                }
                // The row was reset or removed after the pass read it; the reload that
                // follows such a change brings the next pass.
                Err(Error::Conflict(_)) => return Ok(None),
                Err(e) => return Err(e),
            }
        }

        // Step 3, a due retry, and step 4, judging the watched run, are Task 5's (spec §6).

        let next = if limit.is_some_and(|n| fired >= n) {
            None
        } else {
            match next_slot(schedule, &last, &Local) {
                Ok(next) => next.map(|t| t.with_timezone(&Utc)),
                Err(e) => return self.expire(dot, &e).await,
            }
        };
        let retry_at = retry.as_ref().map(|r| parse_time(&r.at)).transpose()?;
        if next.is_none() && retry_at.is_none() {
            // No slot is left and no retry can follow: the count is reached, a `Once` is
            // handled, or the stop date has passed.
            self.set_status(dot, ScheduleStatus::Completed, now).await?;
            tracing::info!(dot = %dot.spec.name, "schedule completed");
            return Ok(None);
        }
        // A retry that is due already waits for Task 5's step 3; it is no reason to wake.
        let retry_at = retry_at.filter(|at| *at > now);
        Ok([next, retry_at].into_iter().flatten().min())
    }

    /// The schedule no longer parses (spec §10): the row is `expired` and fires nothing.
    async fn expire(&self, dot: &Dot, e: &Error) -> Result<Option<DateTime<Utc>>> {
        tracing::warn!(dot = %dot.spec.name, "schedule expired: {e}");
        self.set_status(dot, ScheduleStatus::Expired, self.clock.now())
            .await?;
        Ok(None)
    }

    async fn set_status(
        &self,
        dot: &Dot,
        status: ScheduleStatus,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let change = ScheduleChange {
            status: Some(status),
            ..Default::default()
        };
        if self
            .store
            .update_schedule_state(&dot.id, &change, now)
            .await?
            .is_some()
        {
            self.updated(&dot.id);
        }
        Ok(())
    }

    fn updated(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::ScheduleUpdated {
            dot_id: dot_id.to_string(),
        });
    }
}

/// The run that serves the slot at `at`; `caught_up` when the slot was missed.
fn scheduled_run(dot: &Dot, at: DateTime<Utc>, caught_up: bool) -> NewRun {
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    let mut payload = json!({ "scheduled_for": at.with_timezone(&Local).to_rfc3339() });
    if caught_up {
        payload["caught_up"] = json!(true);
    }
    new.payload = Some(payload);
    new
}

/// A time the store wrote (`util::timestamp`).
fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Invalid(format!("stored time {s:?}: {e}")))
}
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    pub async fn enqueue(&self, new: NewRun) -> Result<Run> {
        let run = self.store.create_run(&new).await?;
        self.emit_run(&run);
        self.wake.notify_one();
        Ok(run)
    }

```

**with:**

```rust
    pub async fn enqueue(&self, new: NewRun) -> Result<Run> {
        let run = self.store.create_run(&new).await?;
        self.enqueued(&run);
        Ok(run)
    }

    /// Announces a run queued by someone else (the scheduler queues its runs in the same
    /// transaction as its schedule row) and wakes the dispatcher for it.
    pub fn enqueued(&self, run: &Run) {
        self.emit_run(run);
        self.wake.notify_one();
    }

```

**In `crates/dots-core/src/store/schedules.rs`, replace:**

```rust
        .await?;
        Ok(res.rows_affected() > 0)
    }

```

**with:**

```rust
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// A resume after a pause (spec §5, "Pause"): every active row's `last_slot_at` becomes
    /// `now`, so the slots of the pause are not caught up. A row that is already at or after
    /// `now` is left alone; the watch, a waiting retry and the attention stay. Returns the dots
    /// whose row moved.
    pub async fn resume_schedules(&self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let ts = timestamp(now);
        let ids = sqlx::query_scalar(
            "UPDATE schedule_state SET last_slot_at = ?, updated_at = ? \
             WHERE status = ? AND last_slot_at < ? RETURNING dot_id",
        )
        .bind(&ts)
        .bind(&ts)
        .bind(ScheduleStatus::Active.as_str())
        .bind(&ts)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{next_slot, upcoming, Schedule};
use crate::scheduler::Scheduler;
use crate::server::{self, ServerState};
use crate::settings::Settings;
```

**with:**

```rust
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{next_slot, upcoming, Schedule};
use crate::scheduler::{Clock, Scheduler, SystemClock};
use crate::server::{self, ServerState};
use crate::settings::Settings;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    /// Delays between attempts when a local endpoint is unreachable or answers 5xx.
    pub local_retry_delays: Vec<Duration>,
}

```

**with:**

```rust
    /// Delays between attempts when a local endpoint is unreachable or answers 5xx.
    pub local_retry_delays: Vec<Duration>,
    /// Where the scheduler and the schedule rows get the time. Tests move a fake one by hand.
    pub clock: Arc<dyn Clock>,
}

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            claude_env: Vec::new(),
            local_retry_delays: DEFAULT_RETRY_DELAYS.to_vec(),
        }
    }
```

**with:**

```rust
            claude_env: Vec::new(),
            local_retry_delays: DEFAULT_RETRY_DELAYS.to_vec(),
            clock: Arc::new(SystemClock),
        }
    }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    port: u16,
    server_error: Option<String>,
```

**with:**

```rust
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    clock: Arc<dyn Clock>,
    port: u16,
    server_error: Option<String>,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        if !recovered.is_empty() {
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }

```

**with:**

```rust
        if !recovered.is_empty() {
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
        // migration 0005 gets one now, and a row a failed change left stale is reset.
        for dot in store.list_dots().await? {
            store
                .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, cfg.clock.now())
                .await?;
        }

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        }
        runner.spawn_dispatcher(shutdown.clone());
        let scheduler = Scheduler::new(store.clone(), runner.clone());
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
```

**with:**

```rust
        }
        runner.spawn_dispatcher(shutdown.clone());
        let scheduler = Scheduler::new(
            store.clone(),
            runner.clone(),
            bus.clone(),
            cfg.clock.clone(),
        );
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            runner,
            scheduler,
            port,
            server_error,
```

**with:**

```rust
            runner,
            scheduler,
            clock: cfg.clock.clone(),
            port,
            server_error,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    /// Every dot with its active and last run and its next scheduled fire time.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = Local::now();
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
```

**with:**

```rust
    /// Every dot with its active and last run and its next scheduled fire time.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = self.clock.now().with_timezone(&Local);
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        if self
            .store
            .sync_schedule_state(&dot.id, schedule, enabled_now, Utc::now())
            .await?
        {
```

**with:**

```rust
        if self
            .store
            .sync_schedule_state(&dot.id, schedule, enabled_now, self.clock.now())
            .await?
        {
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    src/store/               SQLite via sqlx, embedded migrations
    src/schedule.rs          schedules (once, every, daily, weekly, cron) and their slots in local time
    src/scheduler/           tokio-cron-scheduler -> enqueue runs
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
```

**with:**

```markdown
    src/store/               SQLite via sqlx, embedded migrations
    src/schedule.rs          schedules (once, every, daily, weekly, cron) and their slots in local time
    src/scheduler.rs         the loop: a pass over the schedule rows (fire, catch up, skip,
                             complete), sleeping on a clock that tests move by hand
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
### Steps

1. **Trigger.** Scheduler tick, `POST /dots/{id}/trigger`, "Run now", or a chat message inserts a
   `queued` run with its payload. A schedule tick for a dot that already has a `queued` run is
   coalesced (dropped, logged).
2. **Dispatch.** The runner starts queued runs FIFO while running count < `max_concurrent_runs`
   and the dot has no other `running` run.
```

**with:**

```markdown
### Steps

1. **Trigger.** A schedule slot, `POST /dots/{id}/trigger`, "Run now", or a chat message inserts
   a `queued` run with its payload. From Scheduler v2 (`2026-10-14-scheduler-v2-design.md` §5) a
   slot fires when it is due (less than 60 s late); a slot missed while Dots was closed or the
   PC slept fires once as a catch-up (payload `caught_up: true`) when the schedule has
   `catch_up` and the slot is less than 24 h old, and only the newest missed slot does; other
   missed slots are skipped. A slot for a dot that already has a `queued` run is coalesced into
   it (logged). The run that serves a slot, new or joined, is the schedule's watched run
   (`schedule_state.watch_run_id`): its outcome decides the occurrence.
2. **Dispatch.** The runner starts queued runs FIFO while running count < `max_concurrent_runs`
   and the dot has no other `running` run.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   - `mark_reviewed(run_id)` — sets `reviewed_at` on the lineage.
8. `set_schedules_paused(bool)` — persisted as `schedules_paused`; the scheduler fires nothing
   while paused.
9. `settings()` and `update_settings(Settings) -> { settings, restart_required }`.
   `restart_required` is true when `port`, `max_concurrent_runs` or `claude_path` differ from
```

**with:**

```markdown
   - `mark_reviewed(run_id)` — sets `reviewed_at` on the lineage.
8. `set_schedules_paused(bool)` — persisted as `schedules_paused`; the scheduler fires nothing
   while paused. From Scheduler v2 a resume does not catch up: every active schedule's last
   handled slot becomes the time of the resume, so the slots of the pause are skipped.
9. `settings()` and `update_settings(Settings) -> { settings, restart_required }`.
   `restart_required` is true when `port`, `max_concurrent_runs` or `claude_path` differ from
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - Open.
  - Inbox (n) — shows the window on `/inbox` through a `navigate` event.
  - Pause all schedules — check item.
  - Quit — `Runtime::shutdown`, then exit.
  Left click shows the window; closing the window hides it.
```

**with:**

```markdown
  - Open.
  - Inbox (n) — shows the window on `/inbox` through a `navigate` event.
  - Pause all schedules — check item. Unchecking it does not catch up the slots of the pause.
  - Quit — `Runtime::shutdown`, then exit.
  Left click shows the window; closing the window hides it.
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p dots-core --test runtime_schedule_test --test scheduler_test --test store_schedules_test 2>&1 | grep -E "^test result|FAILED"
```

Expected: `runtime_schedule_test` 3 passed, `scheduler_test` 13 passed, `store_schedules_test` 14 passed. The five real-time tests the old `scheduler_test.rs` had are replaced by the 13 fake-clock tests.

- [ ] **Step 5: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

Expected: `dots-core` 395 passed, `dots-app` 48 passed, no clippy line, `cargo fmt --check` silent.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/runner.rs crates/dots-core/src/runtime.rs crates/dots-core/src/scheduler.rs crates/dots-core/src/store/schedules.rs crates/dots-core/tests/common/clock.rs crates/dots-core/tests/common/mod.rs crates/dots-core/tests/runtime_schedule_test.rs crates/dots-core/tests/scheduler_test.rs crates/dots-core/tests/store_schedules_test.rs docs/superpowers/specs/2026-10-08-dots-design.md
git commit -m "feat(core): the scheduler loop of Scheduler v2, on a clock tests can move"
```

### Task 5: Judge each scheduled occurrence, retry it, or flag it for attention

The loop now follows the run that decides each occurrence (spec section 6). When the watched run ends, the pass judges it: a success ends the occurrence and clears the flag, a cancel ends it quietly, a parked run hands the watch to its resume child, and a failure is retried when neither the run nor a run it resumes called a tool. A retry starts 1, 2, 4, 8 or 16 minutes after the failure, joins a queued run if there is one, and is dropped by a slot that fires; the sixth failed attempt, or a failure after a tool call, sets the flag. A completion now waits for the last outcome, and the end of a watched run wakes the loop. The flag is listed in the Inbox and counted by the tray, it notifies once through a new event, and it can be dismissed; a failed scheduled run no longer notifies by itself (spec section 7). The main spec's section 4, the Inbox, the events and the notifications follow.

**Files:**
- Modify: `apps/dots-app/src/contract.rs`, `apps/dots-app/src/events.rs`, `crates/dots-core/src/events.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/scheduler.rs`, `crates/dots-core/src/store/schedules.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`, `ui/src/lib/runtime-events.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/runtime_schedule_test.rs`, `crates/dots-core/tests/scheduler_test.rs`, `ui/src/lib/runtime-events.test.ts`

**Interfaces:**
- Consumes: Tasks 3 and 4; `resume_parked` and the `resume` trigger (Plan 3c), the `tool_call` run events.
- Produces:
  - `scheduler::MAX_ATTEMPTS = 6`; `scheduler::retry_delay(attempt: u32) -> TimeDelta`: 1, 2, 4, 8, 16 minutes for attempts 1 to 5.
  - In a pass, after the slot: a due retry is started with `start_retry` (`Enqueued` announced); then, when no retry waits, the watched run is judged. `queued`, `running`, `awaiting_approval`: nothing. `succeeded` with a resume child: the watch moves to the child, which is judged in the same pass. `succeeded`: watch and flag cleared. `cancelled`, or the run is gone: watch cleared. `failed` and retryable (`called_a_tool` is false): `retry = { slot_at, attempt: k + 1, at: ended_at + retry_delay(k) }` while `k < 6`, the watch kept; otherwise the watch cleared and `attention_run_id` set. Every judging write is guarded by `if_watching` and publishes `ScheduleUpdated`.
  - A retry run: trigger `schedule`, `retry_of` the watched run, payload `{"scheduled_for": <slot>, "attempt": k + 1}`. The slot and the attempt `k` come from the head of the occurrence's resume chain (its payload's `attempt`, 1 when missing).
  - A flagged row is cleared, with `ScheduleUpdated`, when `succeeded_after(dot, flagged)`: a `manual` or `schedule` run queued at or after the flagged one, not that run, that succeeded and is not a parked parent closed by its decision.
  - Completion: no slot left (or the count reached), no retry waiting and no watched run.
  - `RuntimeEvent::ScheduleAttention { dot_id, run_id, error: Option<String> }` (JSON type `schedule_attention`), published when the flag goes from clear to set.
  - Wake: the end of a run that a row watched (a terminal `RunUpdated`, or `Lagged`) wakes the loop; a due `retry_at` makes it pass again at once.
  - Store: `resume_child(run_id) -> Result<Option<String>>`, `called_a_tool(run_id) -> Result<bool>` (the run and the parked runs it resumes), `succeeded_after(dot_id, run_id) -> Result<bool>`, `list_schedule_attention() -> Result<Vec<NeedsAttention>>` (every row with a flag, newest failure first).
  - `NeedsAttention { dot_id, dot_name, run_id, error: Option<String>, failed_at }`; `Inbox.attention: Vec<NeedsAttention>`; `Runtime::inbox_count` counts it. `Runtime::dismiss_schedule_attention(&self, dot_id: &str) -> Result<()>`: `NotFound("schedule of dot <id>")` without a row, `Ok` and no event without a flag, otherwise clears it and publishes `ScheduleUpdated`.
  - App: `Notice::ScheduleFailed { dot_id, error }`; a failed run with trigger `schedule` does not notify; `ScheduleAttention` notifies with the title "<dot>: scheduled run failed" and the error's first line (or "The run failed.").
  - UI: the `schedule_attention` variant of `RuntimeEvent` (it changes no cache; the `schedule_updated` sent with it does); `T.NeedsAttention`; `Inbox.attention` optional until Task 6.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/runtime_schedule_test.rs`, replace:**

```rust
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, ScheduleChange, ScheduleState, ScheduleStatus, WorkspaceMode,
};
use dots_core::schedule::Schedule;
use dots_core::store::{Store, DB_FILE};
use dots_core::util::{now, timestamp};
use dots_core::{Config, Runtime};
use serde_json::json;
use tempfile::TempDir;
```

**with:**

```rust
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, NeedsAttention, NewRun, Run, RunStatus, ScheduleChange, ScheduleState,
    ScheduleStatus, TriggerKind, WorkspaceMode,
};
use dots_core::schedule::Schedule;
use dots_core::store::{Store, DB_FILE};
use dots_core::util::{now, timestamp};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;
```

**Append to `crates/dots-core/tests/runtime_schedule_test.rs`:**

```rust

/// A scheduled run of `dot` that failed with `error`, set as the schedule's "Needs
/// attention" (as the scheduler sets it).
async fn flag(rt: &Runtime, dot: &Dot, error: &str) -> Run {
    let run = rt
        .store()
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let run = rt
        .store()
        .finish_run(&run.id, RunStatus::Failed, None, Some(error))
        .await
        .unwrap();
    let change = ScheduleChange {
        attention_run_id: Some(Some(run.id.clone())),
        ..Default::default()
    };
    rt.store()
        .update_schedule_state(&dot.id, &change, Utc::now())
        .await
        .unwrap()
        .unwrap();
    run
}

fn attention(dot: &Dot, run: &Run) -> NeedsAttention {
    NeedsAttention {
        dot_id: dot.id.clone(),
        dot_name: dot.spec.name.clone(),
        run_id: run.id.clone(),
        error: run.error.clone(),
        failed_at: run.ended_at.clone().unwrap(),
    }
}

/// Scheduler v2 spec §8: the Inbox lists the schedules that need attention, newest failure
/// first, and counts them; `dismiss_schedule_attention` clears a flag, quietly when there is
/// none, and refuses a dot without a schedule.
#[tokio::test]
async fn attention_is_listed_in_the_inbox_until_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let older = rt
        .create_dot(folder_spec(&dir, "older", Some(every(600))))
        .await
        .unwrap();
    let newer = rt
        .create_dot(folder_spec(&dir, "newer", Some(every(600))))
        .await
        .unwrap();
    let plain = rt
        .create_dot(folder_spec(&dir, "plain", None))
        .await
        .unwrap();
    let first = flag(&rt, &older, "model server down").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = flag(&rt, &newer, "claude: not found").await;

    let inbox = rt.inbox().await.unwrap();
    assert_eq!(
        inbox.attention,
        [attention(&newer, &second), attention(&older, &first)]
    );
    assert_eq!(
        rt.inbox_count().await.unwrap(),
        inbox.approvals.len() + inbox.review_runs.len() + 2
    );

    let mut rx = rt.subscribe();
    rt.dismiss_schedule_attention(&older.id).await.unwrap();
    assert_eq!(row(&rt, &older).await.unwrap().attention_run_id, None);
    assert_eq!(updated(&mut rx), [older.id.as_str()]);
    assert_eq!(
        rt.inbox().await.unwrap().attention,
        [attention(&newer, &second)]
    );

    // No flag: nothing to do, and no error.
    rt.dismiss_schedule_attention(&older.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    // No schedule row: not found.
    for id in [plain.id.as_str(), "no-such-dot"] {
        assert!(matches!(
            rt.dismiss_schedule_attention(id).await,
            Err(Error::NotFound(_))
        ));
    }
    rt.shutdown();
}
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{
    Dot, EngineKind, PendingRetry, Run, ScheduleChange, ScheduleState, ScheduleStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
use dots_core::util::timestamp;
```

**with:**

```rust
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{
    Dot, EngineKind, NewRun, PendingRetry, Run, RunStatus, ScheduleChange, ScheduleState,
    ScheduleStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::{Clock, Scheduler};
use dots_core::store::Store;
use dots_core::util::timestamp;
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
}

/// A scheduler on a fake clock at `T0`, not started yet.
struct Harness {
```

**with:**

```rust
}

/// What the scheduler announced: the dots named by `ScheduleUpdated`, the runs announced by
/// `RunUpdated`, and the `ScheduleAttention` notices as (dot, run, error).
#[derive(Default)]
struct Seen {
    schedules: Vec<String>,
    runs: Vec<String>,
    notices: Vec<(String, String, Option<String>)>,
}

/// The error of the runs the tests fail.
const DOWN: &str = "model server down";

/// A scheduler on a fake clock at `T0`, not started yet.
struct Harness {
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    store: Store,
    clock: Arc<FakeClock>,
    sched: Arc<Scheduler>,
    events: Receiver<RuntimeEvent>,
    shutdown: CancellationToken,
    sleeps: u64,
```

**with:**

```rust
    store: Store,
    clock: Arc<FakeClock>,
    bus: Bus,
    runner: Arc<Runner>,
    sched: Arc<Scheduler>,
    events: Receiver<RuntimeEvent>,
    seen: Seen,
    shutdown: CancellationToken,
    sleeps: u64,
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let sched = Scheduler::new(store.clone(), runner, bus.clone(), clock.clone());
        Harness {
            events: bus.subscribe(),
```

**with:**

```rust
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let sched = Scheduler::new(store.clone(), runner.clone(), bus.clone(), clock.clone());
        Harness {
            events: bus.subscribe(),
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
            store,
            clock,
            sched,
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
    }

```

**with:**

```rust
            store,
            clock,
            bus,
            runner,
            sched,
            seen: Seen::default(),
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
    }

    /// Stops the scheduler and starts a new one on the same database and clock, as a restart
    /// of Dots does.
    fn restart(&mut self) {
        self.stop();
        self.sched = Scheduler::new(
            self.store.clone(),
            self.runner.clone(),
            self.bus.clone(),
            self.clock.clone(),
        );
        self.start();
    }

    /// Stops the scheduler (Dots is closed).
    fn stop(&mut self) {
        self.shutdown.cancel();
        self.shutdown = CancellationToken::new();
    }

```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    }

    /// The dots named by `ScheduleUpdated` and the runs announced by `RunUpdated` since the
    /// last call.
    fn events(&mut self) -> (Vec<String>, Vec<String>) {
        let (mut schedules, mut runs) = (Vec::new(), Vec::new());
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                RuntimeEvent::ScheduleUpdated { dot_id } => schedules.push(dot_id),
                RuntimeEvent::RunUpdated { run } => runs.push(run.id),
                _ => {}
            }
        }
        (schedules, runs)
    }

    /// Writes the row behind the store's back, for a state only Task 5's judging makes.
    async fn raw(&self, sql: &str, dot: &Dot) {
        let pool = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            self.dir.path().join("dots.db").display()
        ))
        .await
        .unwrap();
        sqlx::query(sql).bind(&dot.id).execute(&pool).await.unwrap();
    }
}
```

**with:**

```rust
    }

    /// The run the row watches.
    async fn watched(&self, dot: &Dot) -> Run {
        let id = self.row(dot).await.watch_run_id.expect("a watched run");
        self.store.get_run(&id).await.unwrap()
    }

    fn drain(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                RuntimeEvent::ScheduleUpdated { dot_id } => self.seen.schedules.push(dot_id),
                RuntimeEvent::RunUpdated { run } => self.seen.runs.push(run.id),
                RuntimeEvent::ScheduleAttention {
                    dot_id,
                    run_id,
                    error,
                } => self.seen.notices.push((dot_id, run_id, error)),
                _ => {}
            }
        }
    }

    /// The dots named by `ScheduleUpdated` and the runs announced by `RunUpdated` since the
    /// last call.
    fn events(&mut self) -> (Vec<String>, Vec<String>) {
        self.drain();
        (
            std::mem::take(&mut self.seen.schedules),
            std::mem::take(&mut self.seen.runs),
        )
    }

    /// The `ScheduleAttention` notices since the last call, as (dot, run, error).
    fn notices(&mut self) -> Vec<(String, String, Option<String>)> {
        self.drain();
        std::mem::take(&mut self.seen.notices)
    }

    /// Ends `run` with `status` at the clock's time, as the runner does, but without
    /// announcing it. A failed run gets the error `DOWN`.
    async fn end(&self, run: &str, status: RunStatus) {
        let error = (status == RunStatus::Failed).then_some(DOWN);
        self.store
            .finish_run(run, status, None, error)
            .await
            .unwrap();
        self.exec(
            "UPDATE runs SET ended_at = ? WHERE id = ?",
            &[&timestamp(self.clock.now()), run],
        )
        .await;
    }

    /// Ends the watched `run` with `status`, then reloads the scheduler and waits for the pass
    /// that judges it; returns when the loop means to wake next.
    async fn judge(&mut self, run: &str, status: RunStatus) -> i64 {
        self.end(run, status).await;
        self.sched.reload();
        self.settle().await
    }

    /// The run called a tool (an engine event of kind `tool_call`).
    async fn tool_call(&self, run: &str) {
        let call = json!({ "type": "tool_call", "id": "c1", "tool": "Bash", "input": {} });
        self.store
            .append_event(run, "tool_call", &call)
            .await
            .unwrap();
    }

    /// Runs `sql` behind the store's back, with `args` bound in order.
    async fn exec(&self, sql: &str, args: &[&str]) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(self.dir.path().join("dots.db"))
            .foreign_keys(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        let mut query = sqlx::query(sql);
        for arg in args {
            query = query.bind(*arg);
        }
        query.execute(&pool).await.unwrap();
        pool.close().await;
    }

    /// Writes the row behind the store's back, for a state the scheduler makes only later.
    async fn raw(&self, sql: &str, dot: &Dot) {
        self.exec(sql, &[&dot.id]).await;
    }
}
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust

#[tokio::test]
async fn once_is_completed_when_its_slot_fired() {
    let mut h = Harness::new().await;
    let once = schedule(json!({
```

**with:**

```rust

#[tokio::test]
async fn once_is_completed_when_its_slot_fired_and_its_run_is_judged() {
    let mut h = Harness::new().await;
    let once = schedule(json!({
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 1)
    );
    assert_eq!(h.events().0, vec![dot.id.clone(), dot.id.clone()]);
```

**with:**

```rust
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Active, 1),
        "its run may still fail and be retried"
    );
    assert_eq!(h.events().0, vec![dot.id.clone()]);

    let run = h.watched(&dot).await;
    h.judge(&run.id, RunStatus::Succeeded).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count, row.watch_run_id.clone()),
        (ScheduleStatus::Completed, 1, None)
    );
    assert_eq!(h.events().0, vec![dot.id.clone(), dot.id.clone()]);
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust

#[tokio::test]
async fn a_count_completes_the_schedule_when_it_is_reached() {
    let mut h = Harness::new().await;
    let mut twice = every(1, 0, true);
```

**with:**

```rust

#[tokio::test]
async fn a_count_completes_the_schedule_after_the_last_outcome() {
    let mut h = Harness::new().await;
    let mut twice = every(1, 0, true);
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 2)
    );

    h.at(300).await;
    assert_eq!(h.row(&dot).await, row);
    assert_eq!(
        h.runs(&dot).await.len(),
```

**with:**

```rust
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Active, 2),
        "the count is reached, the outcome is not known yet"
    );
    assert_eq!(
        h.runs(&dot).await.len(),
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
        "the second slot joined the queued run"
    );
}

```

**with:**

```rust
        "the second slot joined the queued run"
    );

    // The last occurrence fails before any call: its retry still runs.
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 180);
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Active);
    h.at(180).await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    let row = h.row(&dot).await;
    assert_eq!((row.status, row.fired_count), (ScheduleStatus::Active, 2));

    h.judge(&retry.id, RunStatus::Succeeded).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 2)
    );
    h.at(600).await;
    assert_eq!(h.row(&dot).await, row);
    assert_eq!(h.runs(&dot).await.len(), 2);
}

```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
    h.settle().await;
    h.at(60).await;
    let row = h.row(&dot).await;
    assert_eq!(
```

**with:**

```rust
    h.settle().await;
    h.at(60).await;
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Active);
    let run = h.watched(&dot).await;
    h.judge(&run.id, RunStatus::Cancelled).await;
    let row = h.row(&dot).await;
    assert_eq!(
```

**Append to `crates/dots-core/tests/scheduler_test.rs`:**

```rust

/// Spec §6: a failure before any tool call is retried 1, 2, 4, 8 and 16 minutes after the
/// failed run ended, by a run that names the run it repeats. The sixth failed attempt sets
/// "Needs attention", announced once, and the schedule goes on.
#[tokio::test]
async fn a_failure_before_any_call_is_retried_up_to_the_sixth_attempt() {
    let mut h = Harness::new().await;
    let dot = h.dot("flaky", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let mut failed = h.watched(&dot).await;
    let mut now = 60;
    for (attempt, delay) in [(2, 60), (3, 120), (4, 240), (5, 480), (6, 960)] {
        assert_eq!(
            h.judge(&failed.id, RunStatus::Failed).await,
            now + delay.min(60),
            "the loop wakes for attempt {attempt}, or after 60 s"
        );
        let row = h.row(&dot).await;
        assert_eq!(
            row.retry,
            Some(PendingRetry {
                slot_at: ts(60),
                attempt,
                at: ts(now + delay)
            })
        );
        assert_eq!(
            row.watch_run_id.as_deref(),
            Some(failed.id.as_str()),
            "the failed run is kept until its retry starts"
        );
        assert_eq!(row.attention_run_id, None);

        now += delay;
        h.at(now).await;
        let retry = h.watched(&dot).await;
        assert_eq!(retry.trigger, TriggerKind::Schedule);
        assert_eq!(retry.retry_of.as_deref(), Some(failed.id.as_str()));
        let mut payload = slot_payload(60, false);
        payload["attempt"] = json!(attempt);
        assert_eq!(retry.payload, Some(payload));
        let row = h.row(&dot).await;
        assert_eq!(
            (row.retry, row.fired_count),
            (None, 1),
            "a retry does not count"
        );
        failed = retry;
    }
    assert!(h.notices().is_empty());

    h.judge(&failed.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.watch_run_id, row.retry),
        (ScheduleStatus::Active, None, None),
        "no seventh attempt"
    );
    assert_eq!(row.attention_run_id.as_deref(), Some(failed.id.as_str()));
    assert_eq!(
        h.notices(),
        [(dot.id.clone(), failed.id.clone(), Some(DOWN.to_string()))]
    );
    assert_eq!(h.runs(&dot).await.len(), 6);

    // The schedule goes on: the next slot fires.
    h.at(60 + 86400).await;
    assert_eq!(h.runs(&dot).await.len(), 7);
    assert_eq!(h.row(&dot).await.fired_count, 2);
}

/// Spec §6: a run that called a tool may have changed something, so its failure is not
/// retried: the schedule needs attention at once, and keeps firing. A second failure moves
/// the flag without a second notice; a scheduled run that succeeds clears it.
#[tokio::test]
async fn a_failure_after_a_tool_call_needs_attention_until_a_scheduled_run_succeeds() {
    let mut h = Harness::new().await;
    let dot = h.dot("careful", Some(every(10, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.tool_call(&first.id).await;
    h.events();
    h.judge(&first.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.retry), (None, None), "not retried");
    assert_eq!(row.attention_run_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(h.events(), (vec![dot.id.clone()], vec![]));
    assert_eq!(
        h.notices(),
        [(dot.id.clone(), first.id.clone(), Some(DOWN.to_string()))]
    );

    h.at(660).await;
    let second = h.watched(&dot).await;
    assert_ne!(second.id, first.id);
    h.tool_call(&second.id).await;
    h.judge(&second.id, RunStatus::Failed).await;
    assert_eq!(
        h.row(&dot).await.attention_run_id.as_deref(),
        Some(second.id.as_str())
    );
    assert!(h.notices().is_empty(), "the flag was set already");

    h.at(1260).await;
    let third = h.watched(&dot).await;
    h.events();
    h.judge(&third.id, RunStatus::Succeeded).await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.attention_run_id), (None, None));
    assert_eq!(h.events().0, vec![dot.id.clone()]);
    assert!(h.notices().is_empty(), "clearing the flag is quiet");
}

/// Spec §6 and §10: a cancelled watched run, and a watched run that was deleted (its thread
/// discarded), end the occurrence without a retry or "Needs attention".
#[tokio::test]
async fn a_cancelled_or_deleted_watched_run_ends_the_occurrence_quietly() {
    let mut h = Harness::new().await;
    let mut once = every(10, 60, true);
    once.until = Some(dots_core::schedule::Until::Count(1));
    let cancelled = h.dot("cancelled", Some(once.clone()), 0).await;
    let deleted = h.dot("deleted", Some(once), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let run = h.watched(&cancelled).await;
    h.end(&run.id, RunStatus::Cancelled).await;
    let gone = h.watched(&deleted).await;
    h.exec("DELETE FROM runs WHERE id = ?", &[&gone.id]).await;
    h.sched.reload();
    h.settle().await;

    for dot in [&cancelled, &deleted] {
        let row = h.row(dot).await;
        assert_eq!(
            (
                row.status,
                row.watch_run_id,
                row.retry,
                row.attention_run_id
            ),
            (ScheduleStatus::Completed, None, None, None),
            "{}",
            dot.spec.name
        );
    }
    assert!(h.notices().is_empty());
    assert_eq!(h.runs(&cancelled).await.len(), 1);
    assert!(h.runs(&deleted).await.is_empty());
}

/// Spec §6: a run parked on an approval has not ended. When the decision resumes it, the
/// parked run is closed and its resume child decides the occurrence. The parked run called a
/// tool, so the child's failure is not retried even though the child made no call.
#[tokio::test]
async fn the_resume_child_of_a_parked_run_takes_over_the_watch() {
    let mut h = Harness::new().await;
    let dot = h.dot("asks", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let parent = h.watched(&dot).await;
    h.tool_call(&parent.id).await;
    h.store
        .set_status(&parent.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    h.sched.reload();
    h.settle().await;
    assert_eq!(
        h.row(&dot).await.watch_run_id.as_deref(),
        Some(parent.id.as_str()),
        "a parked run has not ended"
    );

    let mut resume = NewRun::new(&dot.id, TriggerKind::Resume);
    resume.parent_run_id = Some(parent.id.clone());
    resume.root_run_id = Some(parent.id.clone());
    let child = h
        .store
        .resume_parked(&parent.id, &[], &[], &[], Some(&resume))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        h.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded,
        "the resume closes the parked run"
    );
    h.sched.reload();
    h.settle().await;
    let row = h.row(&dot).await;
    assert_eq!(row.watch_run_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(row.attention_run_id, None);

    h.judge(&child.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.retry), (None, None));
    assert_eq!(row.attention_run_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(h.notices().len(), 1);
}

/// Spec §6: a retry that comes due while the dot has a queued run joins that run, which is
/// watched from then on.
#[tokio::test]
async fn a_due_retry_joins_the_queued_run() {
    let mut h = Harness::new().await;
    let dot = h.dot("joined", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 120);
    let manual = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    h.events();

    h.at(120).await;
    let row = h.row(&dot).await;
    assert_eq!(row.watch_run_id.as_deref(), Some(manual.id.as_str()));
    assert_eq!((row.retry, row.fired_count), (None, 1));
    assert_eq!(h.runs(&dot).await.len(), 2);
    assert_eq!(
        h.events(),
        (vec![dot.id.clone()], vec![]),
        "no run was added"
    );
}

/// Spec §5 "Fire": a slot that fires while a retry waits drops the retry; the newer
/// occurrence serves.
#[tokio::test]
async fn a_slot_that_fires_drops_a_waiting_retry() {
    let mut h = Harness::new().await;
    let dot = h.dot("busy", Some(every(10, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.at(620).await, 660);
    assert_eq!(
        h.judge(&first.id, RunStatus::Failed).await,
        660,
        "the slot comes before the retry"
    );
    assert_eq!(h.row(&dot).await.retry.unwrap().at, ts(680));

    h.at(660).await;
    let second = h.watched(&dot).await;
    assert_ne!(second.id, first.id);
    assert_eq!(second.payload, Some(slot_payload(660, false)));
    assert_eq!(second.retry_of, None);
    let row = h.row(&dot).await;
    assert_eq!((row.retry, row.fired_count), (None, 2));
    h.at(720).await;
    assert_eq!(
        h.runs(&dot).await.len(),
        2,
        "the dropped retry never starts"
    );
}

/// The Inbox offers "Run now" for a schedule that needs attention (spec §9), and the smoke
/// test expects it to clear the flag (spec §12): a later manual or scheduled run of the dot
/// that succeeds clears it.
#[tokio::test]
async fn a_run_now_that_succeeds_clears_the_attention() {
    let mut h = Harness::new().await;
    let dot = h.dot("fixed", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.tool_call(&first.id).await;
    h.judge(&first.id, RunStatus::Failed).await;
    assert!(h.row(&dot).await.attention_run_id.is_some());
    assert_eq!(h.notices().len(), 1);

    // A chat run that succeeds says nothing about the schedule.
    let chat = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    h.judge(&chat.id, RunStatus::Succeeded).await;
    assert!(h.row(&dot).await.attention_run_id.is_some());

    let manual = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    h.events();
    h.judge(&manual.id, RunStatus::Succeeded).await;
    assert_eq!(h.row(&dot).await.attention_run_id, None);
    assert_eq!(h.events().0, vec![dot.id.clone()]);
    assert!(h.notices().is_empty());
}

/// Spec §5 "Pause": no retry starts while schedules are paused; one that came due during the
/// pause starts on the first pass after the resume.
#[tokio::test]
async fn a_retry_that_came_due_during_a_pause_starts_on_resume() {
    let mut h = Harness::new().await;
    let dot = h.dot("paused", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.judge(&first.id, RunStatus::Failed).await;

    h.sched.set_paused(true);
    h.clock.set(after(600));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.runs(&dot).await.len(), 1, "nothing starts while paused");
    h.sched.set_paused(false);
    h.settle().await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    let row = h.row(&dot).await;
    assert_eq!((row.retry, row.last_slot_at), (None, ts(600)));
}

/// Spec §6 "Watching": the row remembers the retry, so a restart while it waits keeps it.
#[tokio::test]
async fn a_restart_while_a_retry_waits_keeps_it() {
    let mut h = Harness::new().await;
    let dot = h.dot("restarted", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 120);

    h.restart();
    assert_eq!(h.settle().await, 120, "the new loop wakes for the retry");
    h.at(120).await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    assert_eq!(retry.payload.unwrap()["attempt"], json!(2));
}

/// Spec §6 "Watching": a watched run that ended while Dots was closed is judged by the first
/// pass after the start; its retry, already due, starts at once.
#[tokio::test]
async fn a_watched_run_that_ended_while_dots_was_closed_is_judged_at_start() {
    let mut h = Harness::new().await;
    let dot = h.dot("closed", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.stop();
    h.clock.set(after(300));
    h.end(&first.id, RunStatus::Failed).await;

    h.clock.set(after(3600));
    h.restart();
    assert_eq!(
        h.settle().await,
        360,
        "the retry was due a minute after the run ended"
    );
    assert_eq!(h.settle().await, 3660);
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    assert_eq!(h.row(&dot).await.retry, None);
}

/// Spec §10: a dot deleted while a retry waits takes its row along; the retry never starts.
#[tokio::test]
async fn a_dot_deleted_while_a_retry_waits_is_never_retried() {
    let mut h = Harness::new().await;
    let dot = h.dot("deleted", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.judge(&first.id, RunStatus::Failed).await;

    h.store.delete_dot_if_idle(&dot.id).await.unwrap();
    h.sched.reload();
    h.settle().await;
    assert!(h.store.get_schedule_state(&dot.id).await.unwrap().is_none());
    h.at(120).await;
    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
}

/// Spec §5: the end of a watched run wakes the loop at once, without a reload; the end of
/// another run does not.
#[tokio::test]
async fn the_end_of_a_watched_run_wakes_the_loop() {
    let mut h = Harness::new().await;
    let dot = h.dot("woken", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let watched = h.watched(&dot).await;

    let other = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let other = h
        .store
        .finish_run(&other.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    h.bus.send(RuntimeEvent::RunUpdated { run: other }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.clock.sleeps(), h.sleeps, "not a watched run: no pass");

    h.end(&watched.id, RunStatus::Failed).await;
    let ended = h.store.get_run(&watched.id).await.unwrap();
    h.bus.send(RuntimeEvent::RunUpdated { run: ended }).unwrap();
    assert_eq!(h.settle().await, 120);
    assert_eq!(h.row(&dot).await.retry.unwrap().attempt, 2);
}
```

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts
    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("refetches the approvals of the approval's own run when Dots performs its call", () => {
```

**with:**

```ts
    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("leaves the caches to the schedule update that comes with a schedule's attention", () => {
    const qc = createQueryClient();
    const all = [keys.dots, keys.overview, keys.inbox, keys.dot(sample.dot.id)];
    for (const key of all) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, {
      type: "schedule_attention",
      dot_id: sample.dot.id,
      run_id: sample.run.id,
      error: "model server unreachable",
    });
    for (const key of all) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("refetches the approvals of the approval's own run when Dots performs its call", () => {
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test runtime_schedule_test --test scheduler_test 2>&1 | grep -E "^error|could not compile" | sort | uniq -c | sort -rn
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/runtime-events.test.ts 2>&1 | grep -E "Tests |Test Files|× ")
```

Expected: the two Rust test targets do not compile: 3 × `` no method named `dismiss_schedule_attention` found for struct `dots_core::Runtime` ``, 2 × `` no field `attention` on type `Inbox` ``, `` no variant named `ScheduleAttention` found for enum `RuntimeEvent` ``, `` unresolved import `dots_core::model::NeedsAttention` ``, then `could not compile` for `scheduler_test` and `runtime_schedule_test`. The UI file passes before the task (`Tests  20 passed (20)`): its new test checks that the new event changes no cache, which an event type the handler does not know does not do either; it guards that the case added in this task stays empty. The notifier's test is in `apps/dots-app/src/events.rs`, the file it tests, and comes with the next step.

- [ ] **Step 3: Implement**

`Inbox.attention` is optional in `types.ts` in this task: making it required means changing every Inbox literal in the UI tests, which Task 6 does.

**In `crates/dots-core/src/events.rs`, replace:**

```rust
        dot_id: String,
    },
}

```

**with:**

```rust
        dot_id: String,
    },
    /// The dot's schedule needs attention: an occurrence failed in the end (Scheduler v2 spec
    /// §6). Published once, when the flag goes from clear to set, with the failed run and its
    /// error; a `ScheduleUpdated` for the row goes with it. The app notifies on it.
    ScheduleAttention {
        dot_id: String,
        run_id: String,
        error: Option<String>,
    },
}

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
}

/// Pending approvals and finished runs awaiting review (spec §12 Inbox).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inbox {
    pub approvals: Vec<PendingApproval>,
    pub review_runs: Vec<ReviewRun>,
}

```

**with:**

```rust
}

/// A schedule whose occurrence failed in the end (Scheduler v2 spec §6, "Needs attention").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedsAttention {
    pub dot_id: String,
    pub dot_name: String,
    /// The last failed run of the occurrence.
    pub run_id: String,
    pub error: Option<String>,
    /// When that run ended, RFC 3339 UTC.
    pub failed_at: String,
}

/// Pending approvals, finished runs awaiting review and schedules that need attention (spec
/// §12 Inbox; Scheduler v2 spec §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inbox {
    pub approvals: Vec<PendingApproval>,
    pub review_runs: Vec<ReviewRun>,
    /// Newest failure first.
    pub attention: Vec<NeedsAttention>,
}

```

**In `crates/dots-core/src/store/schedules.rs`, replace:**

```rust
//! handles slots and retries with `record_slot` and `start_retry` and writes the rest with
//! `update_schedule_state`, each in one transaction. Callers pass `now`, so the scheduler's
//! clock decides every time written here.

use chrono::{DateTime, Utc};
```

**with:**

```rust
//! handles slots and retries with `record_slot` and `start_retry` and writes the rest with
//! `update_schedule_state`, each in one transaction. Callers pass `now`, so the scheduler's
//! clock decides every time written here. The judging of a watched run asks the runs what
//! it needs here too (`resume_child`, `called_a_tool`, `succeeded_after`), and the Inbox
//! lists the flags with `list_schedule_attention`.

use chrono::{DateTime, Utc};
```

**In `crates/dots-core/src/store/schedules.rs`, replace:**

```rust
use super::Store;
use crate::model::{
    NewRun, PendingRetry, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart,
};
use crate::schedule::Schedule;
```

**with:**

```rust
use super::Store;
use crate::model::{
    NeedsAttention, NewRun, PendingRetry, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart,
};
use crate::schedule::Schedule;
```

**In `crates/dots-core/src/store/schedules.rs`, replace:**

```rust
    }

    /// Writes the fields `change` names, in one statement, and returns the row. `None` when
    /// nothing was written: the dot has no row, or the row does not watch
```

**with:**

```rust
    }

    /// The resume run that took over from `run_id` after its approvals were decided, if any
    /// (the newest, should there be more than one).
    pub async fn resume_child(&self, run_id: &str) -> Result<Option<String>> {
        let id = sqlx::query_scalar(
            "SELECT id FROM runs WHERE parent_run_id = ? AND trigger_kind = 'resume' \
             ORDER BY queued_at DESC, id DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(id)
    }

    /// Whether `run_id`, or a run it resumes (up its chain of parked parents), has a
    /// `tool_call` event: it may have changed something, so it is not retried (spec §6).
    pub async fn called_a_tool(&self, run_id: &str) -> Result<bool> {
        let called = sqlx::query_scalar(
            "WITH RECURSIVE chain(id) AS ( \
               SELECT ? \
               UNION SELECT r.parent_run_id FROM runs r JOIN chain c ON r.id = c.id \
               WHERE r.trigger_kind = 'resume' AND r.parent_run_id IS NOT NULL) \
             SELECT EXISTS (SELECT 1 FROM run_events \
               WHERE kind = 'tool_call' AND run_id IN (SELECT id FROM chain))",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(called)
    }

    /// Whether a manual or scheduled run of `dot_id` queued after `run_id` has succeeded. A
    /// parked run closed by its decision does not count: its resume run carries the outcome.
    pub async fn succeeded_after(&self, dot_id: &str, run_id: &str) -> Result<bool> {
        let found = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM runs r, runs f \
             WHERE f.id = ? AND r.dot_id = ? AND r.id <> f.id \
             AND r.trigger_kind IN ('manual', 'schedule') AND r.status = 'succeeded' \
             AND r.queued_at >= f.queued_at \
             AND NOT EXISTS (SELECT 1 FROM runs c WHERE c.parent_run_id = r.id \
                             AND c.trigger_kind = 'resume'))",
        )
        .bind(run_id)
        .bind(dot_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(found)
    }

    /// The schedules that need attention (spec §8, the Inbox), newest failure first. A dot
    /// that is disabled keeps its flag until it is dismissed or the row is reset.
    pub async fn list_schedule_attention(&self) -> Result<Vec<NeedsAttention>> {
        let rows = sqlx::query(
            "SELECT s.dot_id, d.name, r.id, r.error, COALESCE(r.ended_at, r.queued_at) \
             AS failed_at FROM schedule_state s \
             JOIN dots d ON d.id = s.dot_id JOIN runs r ON r.id = s.attention_run_id \
             ORDER BY failed_at DESC, s.dot_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(NeedsAttention {
                    dot_id: row.try_get("dot_id")?,
                    dot_name: row.try_get("name")?,
                    run_id: row.try_get("id")?,
                    error: row.try_get("error")?,
                    failed_at: row.try_get("failed_at")?,
                })
            })
            .collect()
    }

    /// Writes the fields `change` names, in one statement, and returns the row. `None` when
    /// nothing was written: the dot has no row, or the row does not watch
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
//! The scheduler (Scheduler v2 spec §5): one task that runs a pass over the schedule rows of
//! the enabled dots, then sleeps until the next slot, a waiting retry or 60 s, whichever is
//! first. `reload()` and a pause or resume wake it at once.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Local, SubsecRound, TimeDelta, Utc};
use serde_json::json;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
```

**with:**

```rust
//! The scheduler (Scheduler v2 spec §5): one task that runs a pass over the schedule rows of
//! the enabled dots, then sleeps until the next slot, a waiting retry or 60 s, whichever is
//! first. `reload()`, a pause or resume, and the end of a watched run wake it at once. The
//! pass also judges the outcome of each occurrence and retries it or flags it (spec §6).

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Local, SubsecRound, TimeDelta, Utc};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    Dot, NewRun, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart, TriggerKind,
};
use crate::runner::Runner;
use crate::schedule::{due_slot, next_slot, Schedule, Until};
use crate::store::Store;
use crate::{Error, Result};

```

**with:**

```rust
use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    Dot, NewRun, PendingRetry, Run, RunStatus, ScheduleChange, ScheduleState, ScheduleStatus,
    SlotStart, TriggerKind,
};
use crate::runner::Runner;
use crate::schedule::{due_slot, next_slot, Schedule, Until};
use crate::store::Store;
use crate::util::timestamp;
use crate::{Error, Result};

```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
/// The wait before a pass that failed is tried again.
pub const ERROR_WAIT: TimeDelta = TimeDelta::seconds(30);

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
```

**with:**

```rust
/// The wait before a pass that failed is tried again.
pub const ERROR_WAIT: TimeDelta = TimeDelta::seconds(30);
/// The attempts an occurrence has at most: the run of its slot and five retries (spec §6).
pub const MAX_ATTEMPTS: u32 = 6;

/// How long after attempt `attempt` (1 to 5) ended the next one starts: 1, 2, 4, 8 and 16
/// minutes.
pub fn retry_delay(attempt: u32) -> TimeDelta {
    TimeDelta::minutes(1 << (attempt.clamp(1, MAX_ATTEMPTS - 1) - 1))
}

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
    /// Set by a resume, cleared by the pass that follows it.
    resumed: AtomicBool,
}

```

**with:**

```rust
    /// Set by a resume, cleared by the pass that follows it.
    resumed: AtomicBool,
    /// The runs the rows watched at the end of the last pass: the end of one wakes the loop.
    watched: Mutex<HashSet<String>>,
}

```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
            paused: AtomicBool::new(false),
            resumed: AtomicBool::new(false),
        })
    }
```

**with:**

```rust
            paused: AtomicBool::new(false),
            resumed: AtomicBool::new(false),
            watched: Mutex::new(HashSet::new()),
        })
    }
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
```

**with:**

```rust
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        // Subscribed before the first pass, so no end of a run it watches is missed.
        let mut events = self.bus.subscribe();
        tokio::spawn(async move {
            loop {
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
                    _ = shutdown.cancelled() => break,
                    _ = this.reload.notified() => {}
                    _ = sleep => {}
                }
            }
        })
    }

```

**with:**

```rust
                    _ = shutdown.cancelled() => break,
                    _ = this.reload.notified() => {}
                    _ = this.watched_run_ended(&mut events) => {}
                    _ = sleep => {}
                }
            }
        })
    }

    /// Returns when a run the rows watch has ended (its `RunUpdated`), or when events were
    /// lost and one may have.
    async fn watched_run_ended(&self, events: &mut Receiver<RuntimeEvent>) {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::RunUpdated { run })
                    if run.status.is_terminal()
                        && self.watched.lock().unwrap().contains(&run.id) =>
                {
                    return
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => return,
                Err(RecvError::Closed) => std::future::pending().await,
            }
        }
    }

```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
    /// A pass (spec §5) over the active rows of the enabled dots; returns when to wake.
    async fn pass(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let mut dots: std::collections::HashMap<String, Dot> = self
            .store
            .list_dots()
```

**with:**

```rust
    /// A pass (spec §5) over the active rows of the enabled dots; returns when to wake.
    async fn pass(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let mut dots: HashMap<String, Dot> = self
            .store
            .list_dots()
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
            .collect();
        let mut wake = now + MAX_SLEEP;
        for state in self.store.list_schedule_states().await? {
            if state.status != ScheduleStatus::Active {
```

**with:**

```rust
            .collect();
        let mut wake = now + MAX_SLEEP;
        let mut watched = HashSet::new();
        for state in self.store.list_schedule_states().await? {
            if state.status != ScheduleStatus::Active {
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
                continue;
            };
            if let Some(next) = self.handle(&dot, schedule, state, now).await? {
                wake = wake.min(next);
            }
        }
        Ok(wake)
    }

    /// Steps 1, 2 and 5 of a pass for one row: the due slot is fired, caught up or skipped,
    /// then the status is updated. Returns the row's next wake time, `None` when it has none.
    async fn handle(
        &self,
```

**with:**

```rust
                continue;
            };
            if let Some(next) = self
                .handle(&dot, schedule, state, now, &mut watched)
                .await?
            {
                wake = wake.min(next);
            }
        }
        *self.watched.lock().unwrap() = watched;
        Ok(wake)
    }

    /// A pass for one row (spec §5): the due slot is fired, caught up or skipped, a due retry
    /// starts, the watched run is judged once it has ended, then the status is updated. Adds
    /// the run the row watches to `watched`. Returns the row's next wake time, `None` when it
    /// has none.
    async fn handle(
        &self,
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
        state: ScheduleState,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut last = parse_time(&state.last_slot_at)?;
        let mut fired = state.fired_count;
        let mut retry = state.retry;
        let limit = match schedule.until {
            Some(Until::Count(n)) => Some(n),
```

**with:**

```rust
        state: ScheduleState,
        now: DateTime<Utc>,
        watched: &mut HashSet<String>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut last = parse_time(&state.last_slot_at)?;
        let mut fired = state.fired_count;
        let mut watch = state.watch_run_id.clone();
        let mut retry = state.retry.clone();
        let mut attention = state.attention_run_id.clone();
        let limit = match schedule.until {
            Some(Until::Count(n)) => Some(n),
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
                            retry = None;
                            self.runner.enqueued(&run);
                        }
                        SlotStart::Coalesced(run) => {
```

**with:**

```rust
                            retry = None;
                            self.runner.enqueued(&run);
                            watch = Some(run.id);
                        }
                        SlotStart::Coalesced(run) => {
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
                                "schedule slot coalesced into the queued run"
                            );
                        }
                    }
```

**with:**

```rust
                                "schedule slot coalesced into the queued run"
                            );
                            watch = Some(run.id);
                        }
                    }
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
        }

        // Step 3, a due retry, and step 4, judging the watched run, are Task 5's (spec §6).

        let next = if limit.is_some_and(|n| fired >= n) {
```

**with:**

```rust
        }

        // Step 3: a retry that is due starts. While it waited, the row watched the failed run
        // it repeats.
        if let Some(due) = &retry {
            if parse_time(&due.at)? <= now {
                let new = retry_run(dot, due, watch.clone())?;
                match self.store.start_retry(&dot.id, &new, now).await {
                    Ok(start) => {
                        match start {
                            SlotStart::Enqueued(run) => {
                                tracing::info!(
                                    dot = %dot.spec.name, run = %run.id, attempt = due.attempt,
                                    "scheduled run retried"
                                );
                                self.runner.enqueued(&run);
                                watch = Some(run.id);
                            }
                            SlotStart::Coalesced(run) => {
                                tracing::info!(
                                    dot = %dot.spec.name, run = %run.id, attempt = due.attempt,
                                    "retry coalesced into the queued run"
                                );
                                watch = Some(run.id);
                            }
                            SlotStart::Skipped => {}
                        }
                        retry = None;
                        self.updated(&dot.id);
                    }
                    Err(Error::Conflict(_)) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
        }

        // Step 4: the watched run is judged once it has ended. A failed run that a retry
        // repeats was judged already.
        if retry.is_none() {
            if let Some(id) = &watch {
                if let Some(row) = self
                    .judge(dot, id, &state.last_slot_at, attention.is_some(), now)
                    .await?
                {
                    watch = row.watch_run_id;
                    retry = row.retry;
                    attention = row.attention_run_id;
                }
            }
        }

        // "Needs attention" also clears when a later manual or scheduled run of the dot
        // succeeds: the Inbox offers "Run now" next to the flag.
        if let Some(flagged) = &attention {
            if self.store.succeeded_after(&dot.id, flagged).await? {
                let change = ScheduleChange {
                    attention_run_id: Some(None),
                    ..Default::default()
                };
                if self
                    .store
                    .update_schedule_state(&dot.id, &change, now)
                    .await?
                    .is_some()
                {
                    tracing::info!(dot = %dot.spec.name, "a later run succeeded: attention cleared");
                    self.updated(&dot.id);
                }
            }
        }

        let next = if limit.is_some_and(|n| fired >= n) {
```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
        };
        let retry_at = retry.as_ref().map(|r| parse_time(&r.at)).transpose()?;
        if next.is_none() && retry_at.is_none() {
            // No slot is left and no retry can follow: the count is reached, a `Once` is
            // handled, or the stop date has passed.
            self.set_status(dot, ScheduleStatus::Completed, now).await?;
            tracing::info!(dot = %dot.spec.name, "schedule completed");
            return Ok(None);
        }
        // A retry that is due already waits for Task 5's step 3; it is no reason to wake.
        let retry_at = retry_at.filter(|at| *at > now);
        Ok([next, retry_at].into_iter().flatten().min())
    }

```

**with:**

```rust
        };
        let retry_at = retry.as_ref().map(|r| parse_time(&r.at)).transpose()?;
        if next.is_none() && retry_at.is_none() && watch.is_none() {
            // No slot is left, the last occurrence is judged and no retry can follow: the
            // count is reached, a `Once` is handled, or the stop date has passed.
            self.set_status(dot, ScheduleStatus::Completed, now).await?;
            tracing::info!(dot = %dot.spec.name, "schedule completed");
            return Ok(None);
        }
        watched.extend(watch);
        // A retry that is due already (its run ended while Dots was closed) starts on the next
        // pass, at once.
        Ok([next, retry_at].into_iter().flatten().min())
    }

    /// Step 4 (spec §6): judges the watched run `id` if it has ended, with a write guarded by
    /// the watch. A parked run closed by its decision hands the watch to its resume run, which
    /// is judged in turn. `attention_set`: the flag was set before. Returns the row as last
    /// written, `None` when nothing was.
    async fn judge(
        &self,
        dot: &Dot,
        id: &str,
        last_slot_at: &str,
        attention_set: bool,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduleState>> {
        let mut id = id.to_string();
        let mut written = None;
        loop {
            let run = match self.store.get_run(&id).await {
                Ok(run) => run,
                // Deleted meanwhile: the row lost its watch with it (ON DELETE SET NULL), and
                // the occurrence counts as done.
                Err(Error::NotFound(_)) => return Ok(written),
                Err(e) => return Err(e),
            };
            let mut change = match run.status {
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval => {
                    return Ok(written)
                }
                RunStatus::Succeeded => match self.store.resume_child(&id).await? {
                    Some(child) => {
                        let change = ScheduleChange {
                            watch_run_id: Some(Some(child.clone())),
                            if_watching: Some(id.clone()),
                            ..Default::default()
                        };
                        match self
                            .store
                            .update_schedule_state(&dot.id, &change, now)
                            .await?
                        {
                            Some(row) => {
                                self.updated(&dot.id);
                                written = Some(row);
                                id = child;
                                continue;
                            }
                            None => return Ok(written),
                        }
                    }
                    // The occurrence is done.
                    None => ScheduleChange {
                        watch_run_id: Some(None),
                        attention_run_id: Some(None),
                        ..Default::default()
                    },
                },
                // The person stopped it: done, without a retry or attention.
                RunStatus::Cancelled => ScheduleChange {
                    watch_run_id: Some(None),
                    ..Default::default()
                },
                RunStatus::Failed => self.failed(&run, last_slot_at, now).await?,
            };
            change.if_watching = Some(id);
            let Some(row) = self
                .store
                .update_schedule_state(&dot.id, &change, now)
                .await?
            else {
                return Ok(written);
            };
            self.updated(&dot.id);
            if !attention_set && row.attention_run_id.as_deref() == Some(run.id.as_str()) {
                tracing::warn!(
                    dot = %dot.spec.name, run = %run.id,
                    "scheduled run failed in the end: the schedule needs attention"
                );
                let _ = self.bus.send(RuntimeEvent::ScheduleAttention {
                    dot_id: dot.id.clone(),
                    run_id: run.id.clone(),
                    error: run.error.clone(),
                });
            }
            return Ok(Some(row));
        }
    }

    /// What a failed watched run leads to (spec §6). Failed before any tool call, with
    /// attempts left: the next attempt, `retry_delay` after it ended; the row keeps watching
    /// it, the run the retry repeats. Otherwise "Needs attention".
    async fn failed(
        &self,
        run: &Run,
        last_slot_at: &str,
        now: DateTime<Utc>,
    ) -> Result<ScheduleChange> {
        let (slot, attempt) = self.occurrence(run, last_slot_at).await?;
        if attempt < MAX_ATTEMPTS && !self.store.called_a_tool(&run.id).await? {
            let ended = match &run.ended_at {
                Some(t) => parse_time(t)?,
                None => now,
            };
            let retry = PendingRetry {
                slot_at: timestamp(slot),
                attempt: attempt + 1,
                at: timestamp(ended + retry_delay(attempt)),
            };
            tracing::info!(
                run = %run.id, attempt = retry.attempt, at = %retry.at,
                "scheduled run failed before any tool call: retry"
            );
            return Ok(ScheduleChange {
                retry: Some(Some(retry)),
                ..Default::default()
            });
        }
        Ok(ScheduleChange {
            watch_run_id: Some(None),
            attention_run_id: Some(Some(run.id.clone())),
            ..Default::default()
        })
    }

    /// The slot whose occurrence `run` serves, and which attempt it is. A resume run speaks
    /// for the run it resumes; a scheduled run names both in its payload (`scheduled_for`,
    /// and `attempt` for a retry). A run of another trigger that a slot or retry joined is
    /// taken as attempt 1 of the newest handled slot.
    async fn occurrence(&self, run: &Run, last_slot_at: &str) -> Result<(DateTime<Utc>, u32)> {
        let mut head = run.clone();
        while head.trigger == TriggerKind::Resume {
            let Some(parent) = head.parent_run_id.clone() else {
                break;
            };
            head = self.store.get_run(&parent).await?;
        }
        let payload = head
            .payload
            .as_ref()
            .filter(|_| head.trigger == TriggerKind::Schedule);
        let field = |key: &str| payload.and_then(|p| p.get(key));
        let slot = field("scheduled_for")
            .and_then(Value::as_str)
            .and_then(|s| parse_time(s).ok());
        let attempt = field("attempt")
            .and_then(Value::as_u64)
            .map_or(1, |a| a as u32);
        let slot = match slot {
            Some(slot) => slot,
            None => parse_time(last_slot_at)?,
        };
        Ok((slot, attempt))
    }

```

**In `crates/dots-core/src/scheduler.rs`, replace:**

```rust
}

/// A time the store wrote (`util::timestamp`).
fn parse_time(s: &str) -> Result<DateTime<Utc>> {
```

**with:**

```rust
}

/// The run that retries the failed run `failed` for the pending retry `retry` (spec §6).
fn retry_run(dot: &Dot, retry: &PendingRetry, failed: Option<String>) -> Result<NewRun> {
    let slot = parse_time(&retry.slot_at)?;
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.payload = Some(json!({
        "scheduled_for": slot.with_timezone(&Local).to_rfc3339(),
        "attempt": retry.attempt,
    }));
    new.retry_of = failed;
    Ok(new)
}

/// A time the store wrote (`util::timestamp`).
fn parse_time(s: &str) -> Result<DateTime<Utc>> {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::model::{
    Approval, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun, PendingApproval,
    ReviewRun, Run, RunEventRecord, RunStatus, SettingsSaved, Status, TriggerKind, WorktreeDiff,
};
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
```

**with:**

```rust
use crate::model::{
    Approval, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun, PendingApproval,
    ReviewRun, Run, RunEventRecord, RunStatus, ScheduleChange, SettingsSaved, Status, TriggerKind,
    WorktreeDiff,
};
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    }

    /// Pending approvals, plus the finished, unreviewed thread ends that failed or kept a
    /// worktree with changes.
    pub async fn inbox(&self) -> Result<Inbox> {
        let mut names = std::collections::HashMap::new();
```

**with:**

```rust
    }

    /// Pending approvals, the finished, unreviewed thread ends that failed or kept a worktree
    /// with changes, and the schedules that need attention.
    pub async fn inbox(&self) -> Result<Inbox> {
        let mut names = std::collections::HashMap::new();
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            approvals,
            review_runs,
        })
    }
```

**with:**

```rust
            approvals,
            review_runs,
            attention: self.store.list_schedule_attention().await?,
        })
    }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub async fn inbox_count(&self) -> Result<usize> {
        let inbox = self.inbox().await?;
        Ok(inbox.approvals.len() + inbox.review_runs.len())
    }

```

**with:**

```rust
    pub async fn inbox_count(&self) -> Result<usize> {
        let inbox = self.inbox().await?;
        Ok(inbox.approvals.len() + inbox.review_runs.len() + inbox.attention.len())
    }

    /// Clears the dot's "Needs attention" (Scheduler v2 spec §8); the schedule is not
    /// otherwise changed. `NotFound` when the dot has no schedule row; nothing happens when
    /// the flag is not set.
    pub async fn dismiss_schedule_attention(&self, dot_id: &str) -> Result<()> {
        let Some(state) = self.store.get_schedule_state(dot_id).await? else {
            return Err(Error::NotFound(format!("schedule of dot {dot_id}")));
        };
        if state.attention_run_id.is_none() {
            return Ok(());
        }
        let change = ScheduleChange {
            attention_run_id: Some(None),
            ..Default::default()
        };
        if self
            .store
            .update_schedule_state(dot_id, &change, self.clock.now())
            .await?
            .is_some()
        {
            self.schedule_updated(dot_id);
        }
        Ok(())
    }

```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    ApprovalNeeded { run_id: String, tool: String },
    RunEnded { run: Box<Run> },
}

/// Decides which events notify: approval requests, and a run moving from queued or running
/// into succeeded, failed or awaiting approval. Chat runs never notify (the user is watching
/// them). A run first seen in a final state (e.g. marked reviewed) and a parked run closed by
/// a decision do not notify either. A run that parks on an approval request this notifier
/// has announced does not notify a second time: one park, one notification.
#[derive(Default)]
pub struct Notifier {
```

**with:**

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    ApprovalNeeded {
        run_id: String,
        tool: String,
    },
    RunEnded {
        run: Box<Run>,
    },
    /// A schedule needs attention: its occurrence failed in the end (Scheduler v2 spec §7).
    ScheduleFailed {
        dot_id: String,
        error: Option<String>,
    },
}

/// Decides which events notify: approval requests, a run moving from queued or running into
/// succeeded, failed or awaiting approval, and a schedule that needs attention. Chat runs
/// never notify (the user is watching them). A scheduled run that fails does not notify by
/// itself: it is retried, or its schedule needs attention, which notifies. A run first seen
/// in a final state (e.g. marked reviewed) and a parked run closed by a decision do not
/// notify either. A run that parks on an approval request this notifier has announced does
/// not notify a second time: one park, one notification.
#[derive(Default)]
pub struct Notifier {
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
                let was_working = matches!(prev, Some(RunStatus::Queued | RunStatus::Running));
                let ended = match run.status {
                    RunStatus::Succeeded | RunStatus::Failed => true,
                    RunStatus::AwaitingApproval => !told,
                    _ => false,
```

**with:**

```rust
                let was_working = matches!(prev, Some(RunStatus::Queued | RunStatus::Running));
                let ended = match run.status {
                    RunStatus::Succeeded => true,
                    RunStatus::Failed => run.trigger != TriggerKind::Schedule,
                    RunStatus::AwaitingApproval => !told,
                    _ => false,
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
                })
            }
            _ => None,
        }
```

**with:**

```rust
                })
            }
            // The core publishes this once, when the flag goes from clear to set.
            RuntimeEvent::ScheduleAttention { dot_id, error, .. } => Some(Notice::ScheduleFailed {
                dot_id: dot_id.clone(),
                error: error.clone(),
            }),
            _ => None,
        }
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
            ),
        },
    }
}
```

**with:**

```rust
            ),
        },
        Notice::ScheduleFailed { error, .. } => (
            format!("{dot_name}: scheduled run failed"),
            first_line(error.as_deref(), "The run failed."),
        ),
    }
}
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust

async fn notify(app: &AppHandle, rt: &Runtime, notice: &Notice) {
    let run_id = match notice {
        Notice::ApprovalNeeded { run_id, .. } => run_id,
        Notice::RunEnded { run } => &run.id,
    };
    let name = match rt.get_run(run_id).await {
        Ok(run) => rt.get_dot(&run.dot_id).await.map(|d| d.spec.name).ok(),
        Err(_) => None,
    };
    let (title, body) = notice_text(notice, name.as_deref().unwrap_or("A dot"));
```

**with:**

```rust

async fn notify(app: &AppHandle, rt: &Runtime, notice: &Notice) {
    let dot_id = match notice {
        Notice::ApprovalNeeded { run_id, .. } => rt.get_run(run_id).await.map(|r| r.dot_id).ok(),
        Notice::RunEnded { run } => Some(run.dot_id.clone()),
        Notice::ScheduleFailed { dot_id, .. } => Some(dot_id.clone()),
    };
    let name = match dot_id {
        Some(id) => rt.get_dot(&id).await.map(|d| d.spec.name).ok(),
        None => None,
    };
    let (title, body) = notice_text(notice, name.as_deref().unwrap_or("A dot"));
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust

    #[test]
    fn notification_texts() {
        let ended = |s| Notice::RunEnded {
```

**with:**

```rust

    #[test]
    fn a_failed_scheduled_run_is_quiet_and_its_attention_notifies() {
        let mut n = Notifier::default();
        let s = TriggerKind::Schedule;
        // Retried, or it ends in "Needs attention", which notifies.
        assert_eq!(
            notified(&mut n, "f", s, &[Queued, Running, Failed]),
            [false, false, false]
        );
        assert_eq!(
            notified(&mut n, "ok", s, &[Queued, Running, Succeeded]),
            [false, false, true]
        );
        // Other runs of a scheduled dot notify their failure as before.
        assert_eq!(
            notified(&mut n, "m", TriggerKind::Manual, &[Running, Failed]),
            [false, true]
        );

        // The core publishes this once, when the flag goes from clear to set.
        let ev = RuntimeEvent::ScheduleAttention {
            dot_id: "d".into(),
            run_id: "f".into(),
            error: Some("model server down\nat 09:00".into()),
        };
        let notice = n.decide(&ev).expect("a notice");
        assert_eq!(
            notice,
            Notice::ScheduleFailed {
                dot_id: "d".into(),
                error: Some("model server down\nat 09:00".into()),
            }
        );
        assert_eq!(
            notice_text(&notice, "nightly"),
            (
                "nightly: scheduled run failed".into(),
                "model server down".into()
            )
        );
        let unknown = Notice::ScheduleFailed {
            dot_id: "d".into(),
            error: None,
        };
        assert_eq!(notice_text(&unknown, "nightly").1, "The run failed.");
        assert!(affects_inbox(&ev));
    }

    #[test]
    fn notification_texts() {
        let ended = |s| Notice::RunEnded {
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            }}),
            to(RuntimeEvent::ScheduleUpdated { dot_id: "01J0DOT".into() }),
        ],
        // What `approval_rule_candidates` answers for the `Approval` sample.
```

**with:**

```rust
            }}),
            to(RuntimeEvent::ScheduleUpdated { dot_id: "01J0DOT".into() }),
            to(RuntimeEvent::ScheduleAttention {
                dot_id: "01J0DOT".into(),
                run_id: "01J0RUN".into(),
                error: Some("model server unreachable".into()),
            }),
        ],
        // What `approval_rule_candidates` answers for the `Approval` sample.
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
                dot_name: "nightly-tests".into(),
                worktree_exists: true,
            }],
        }),
```

**with:**

```rust
                dot_name: "nightly-tests".into(),
                worktree_exists: true,
            }],
            attention: vec![NeedsAttention {
                dot_id: "01J0DOT".into(),
                dot_name: "nightly-tests".into(),
                run_id: "01J0RUN".into(),
                error: Some("model server unreachable".into()),
                failed_at: TS.into(),
            }],
        }),
```

**In `ui/src/api/types.ts`, replace:**

```ts
  // The dot's schedule row was made, changed or deleted.
  z.strictObject({ type: z.literal("schedule_updated"), dot_id: z.string() }),
]);

```

**with:**

```ts
  // The dot's schedule row was made, changed or deleted.
  z.strictObject({ type: z.literal("schedule_updated"), dot_id: z.string() }),
  // The dot's schedule needs attention now (sent once, with a `schedule_updated`).
  z.strictObject({
    type: z.literal("schedule_attention"),
    dot_id: z.string(),
    run_id: z.string(),
    error: nullable(z.string()),
  }),
]);

```

**In `ui/src/api/types.ts`, replace:**

```ts
  worktree_exists: z.boolean(),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
});

```

**with:**

```ts
  worktree_exists: z.boolean(),
});
// A schedule whose occurrence failed in the end ("Needs attention").
export const NeedsAttention = z.strictObject({
  dot_id: z.string(),
  dot_name: z.string(),
  run_id: z.string(),
  error: nullable(z.string()),
  failed_at: z.string(),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
  // Newest failure first. Optional until the Inbox screen shows it.
  attention: z.array(NeedsAttention).optional(),
});

```

**In `ui/src/api/types.ts`, replace:**

```ts
export type PendingApproval = z.infer<typeof PendingApproval>;
export type ReviewRun = z.infer<typeof ReviewRun>;
export type Inbox = z.infer<typeof Inbox>;
export type FileDiff = z.infer<typeof FileDiff>;
```

**with:**

```ts
export type PendingApproval = z.infer<typeof PendingApproval>;
export type ReviewRun = z.infer<typeof ReviewRun>;
export type NeedsAttention = z.infer<typeof NeedsAttention>;
export type Inbox = z.infer<typeof Inbox>;
export type FileDiff = z.infer<typeof FileDiff>;
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
  }
}
```

**with:**

```ts
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
      break;
  }
}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   missed slots are skipped. A slot for a dot that already has a `queued` run is coalesced into
   it (logged). The run that serves a slot, new or joined, is the schedule's watched run
   (`schedule_state.watch_run_id`): its outcome decides the occurrence.
2. **Dispatch.** The runner starts queued runs FIFO while running count < `max_concurrent_runs`
   and the dot has no other `running` run.
```

**with:**

```markdown
   missed slots are skipped. A slot for a dot that already has a `queued` run is coalesced into
   it (logged). The run that serves a slot, new or joined, is the schedule's watched run
   (`schedule_state.watch_run_id`): its outcome decides the occurrence (Scheduler v2 spec §6).
   A parked watched run hands the watch to its resume run. A watched run that failed before
   any `tool_call` event (in it or in the parked runs it resumes) is retried 1, 2, 4, 8 and
   16 minutes after it ended, up to 6 attempts, by a `schedule` run with `retry_of` and the
   payload `{scheduled_for, attempt}` (joined into a queued run of the dot if there is one;
   dropped when a newer slot fires). A failure after a tool call, or of the sixth attempt,
   sets "Needs attention" (`schedule_state.attention_run_id`); the schedule keeps firing. A
   later manual or scheduled run of the dot that succeeds, a dismiss, or a reset of the row
   clears it. A cancelled or deleted watched run ends the occurrence quietly.
2. **Dispatch.** The runner starts queued runs FIFO while running count < `max_concurrent_runs`
   and the dot has no other `running` run.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
5. **Execute.** Engine events are appended to `run_events` and broadcast to the UI.
6. **Finish.** Final assistant text becomes `summary`. The app notifies on `failed`,
   `awaiting_approval`, and `succeeded` (§12). A worktree with no commits and no uncommitted changes is
   removed (`git worktree remove`, branch deleted); otherwise it is kept for review. The same
   cleanup runs when a parked run is closed by a plain deny or cancelled.
```

**with:**

```markdown
5. **Execute.** Engine events are appended to `run_events` and broadcast to the UI.
6. **Finish.** Final assistant text becomes `summary`. The app notifies on `failed`,
   `awaiting_approval`, and `succeeded` (§12); from Scheduler v2 not on a failed `schedule`
   run, which is retried or ends in "Needs attention" (notified once). A worktree with no commits and no uncommitted changes is
   removed (`git worktree remove`, branch deleted); otherwise it is kept for review. The same
   cleanup runs when a parked run is closed by a plain deny or cancelled.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   `list_runs(dot_id?, limit)`, `get_run`, `run_thread(root_run_id)` (all runs of a lineage,
   oldest first), `list_run_events(run_id, after_seq)`, `approvals_for_run`,
   `inbox() -> { approvals, review_runs }`, `inbox_count()`, and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, answers `[]` when no slot is left, and
```

**with:**

```markdown
   `list_runs(dot_id?, limit)`, `get_run`, `run_thread(root_run_id)` (all runs of a lineage,
   oldest first), `list_run_events(run_id, after_seq)`, `approvals_for_run`,
   `inbox() -> { approvals, review_runs, attention }` (from Scheduler v2, `attention`
   lists the schedules that need attention: `dot_id, dot_name, run_id, error, failed_at`,
   newest first), `inbox_count()` (counts all three), and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, answers `[]` when no slot is left, and
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown

- pending approvals;
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.

A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review.

### Tauri shell (`apps/dots-app`)
```

**with:**

```markdown

- pending approvals;
- from Scheduler v2, schedules that need attention (an occurrence failed in the end);
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.

A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review + schedules that need
attention.

### Tauri shell (`apps/dots-app`)
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - From Scheduler v2 there is a sixth event, `ScheduleUpdated { dot_id }`: the dot's
    `schedule_state` row was made, changed or deleted. It updates the tray's Inbox count.
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
```

**with:**

```markdown
  - From Scheduler v2 there is a sixth event, `ScheduleUpdated { dot_id }`: the dot's
    `schedule_state` row was made, changed or deleted. It updates the tray's Inbox count.
  - And a seventh, `ScheduleAttention { dot_id, run_id, error }`: the dot's schedule needs
    attention now. The core sends it once, when the flag goes from clear to set, together
    with a `ScheduleUpdated`. Only the notifier uses it.
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  re-published by "Mark reviewed". From Plan 3c a park shows one notification, not two: the
  update into `awaiting_approval` does not notify when the approval request of that run was
  already announced.
- After each relevant event, `inbox_count()` updates the tray tooltip ("Dots: n to review") and
  the "Inbox (n)" menu item.
```

**with:**

```markdown
  re-published by "Mark reviewed". From Plan 3c a park shows one notification, not two: the
  update into `awaiting_approval` does not notify when the approval request of that run was
  already announced. From Scheduler v2 a failed run with trigger `schedule` does not notify;
  `ScheduleAttention` does, once: "<dot>: scheduled run failed" with the error's first line.
- After each relevant event, `inbox_count()` updates the tray tooltip ("Dots: n to review") and
  the "Inbox (n)" menu item.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    `approvals` of the approval's own run, which is the parent of the run that performs the
    call: that run's own `RunUpdated` does not reach those approvals. `ScheduleUpdated`
    invalidates `dots`, `overview` and `inbox`.
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
```

**with:**

```markdown
    `approvals` of the approval's own run, which is the parent of the run that performs the
    call: that run's own `RunUpdated` does not reach those approvals. `ScheduleUpdated`
    invalidates `dots`, `overview` and `inbox`; `ScheduleAttention` invalidates nothing (the
    `ScheduleUpdated` sent with it does).
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. The `Inbox` sample gains an `attention` item (`nightly-tests`, "model server unreachable"), and `RuntimeEvent` gains a `schedule_attention` sample.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test runtime_schedule_test --test scheduler_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/runtime-events.test.ts src/api/contract.test.ts 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `runtime_schedule_test` 4 passed, `scheduler_test` 25 passed; `dots-app` 49 passed (the new `events::tests::a_failed_scheduled_run_is_quiet_and_its_attention_notifies` among them); both UI files pass.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 408 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 254 passed in 20 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs apps/dots-app/src/events.rs crates/dots-core/src/events.rs crates/dots-core/src/model.rs crates/dots-core/src/runtime.rs crates/dots-core/src/scheduler.rs crates/dots-core/src/store/schedules.rs crates/dots-core/tests/runtime_schedule_test.rs crates/dots-core/tests/scheduler_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/lib/runtime-events.test.ts ui/src/lib/runtime-events.ts
git commit -m "feat(core): judge each scheduled occurrence, retry it, or flag it for attention"
```

### Task 6: Show where each schedule stands and dismiss its attention

The overview tells the UI where each schedule stands (spec section 8): its status, its count, the next run, a waiting retry and the flag with the error of the run that failed. It replaces the overview's top-level `next_fire`. The app gets the command `dismiss_schedule_attention`, and the contract samples every field, so the UI tests use what the core sends. A failed scheduled run that called no tool is no longer listed under "Runs to review": its occurrence is retried or ends in "Needs attention", and there is nothing to review (this plan's ruling). In the UI the types and commands change and every Inbox literal of the tests gains `attention`; the Dots list reads its next run from the new place and shows nothing else new until Task 8. The main spec's Inbox, command and contract passages follow.

**Files:**
- Modify: `apps/dots-app/src/commands.rs`, `apps/dots-app/src/contract.rs`, `apps/dots-app/src/lib.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/store/runs.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/commands.ts`, `ui/src/api/types.ts`, `ui/src/screens/DotsScreen.tsx`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/runtime_review_test.rs`, `crates/dots-core/tests/runtime_schedule_test.rs`, `crates/dots-core/tests/store_runs_test.rs`, `ui/src/api/commands.test.ts`, `ui/src/api/contract.test.ts`, `ui/src/app/Layout.test.tsx`, `ui/src/lib/runtime-events.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`, `ui/src/screens/DotsScreen.test.tsx`, `ui/src/screens/InboxScreen.test.tsx`, `ui/src/test/fixtures.ts`

**Interfaces:**
- Consumes: Tasks 3 to 5 (`get_schedule_state`, `Runtime::dismiss_schedule_attention`, `next_slot`).
- Produces:
  - `AttentionRun { run_id: String, error: Option<String> }`.
  - `ScheduleView { status: ScheduleStatus, fired_count: u32, next_fire: Option<String>, retry_at: Option<String>, retry_attempt: Option<u32>, attention: Option<AttentionRun> }`, every field always present in JSON. `ScheduleView::new(state, schedule, enabled, attention, now, tz)`: while the dot is enabled and the row `active`, `next_fire` is `next_slot(schedule, max(now, last_slot_at), tz)` (null once `Until::Count` is reached, when no slot is left, or on a slot error) and `retry_at`/`retry_attempt` come from the row's retry, `retry_at` converted to `tz`; otherwise the three are null. `status`, `fired_count` and `attention` are always the row's.
  - `DotOverview.schedule_state: Option<ScheduleView>` replaces `DotOverview.next_fire`; `None` without a schedule or a row. `Runtime::dot_overview` builds it in `Local` at the clock's time; the attention's error is read from its run (a run deleted meanwhile gives no attention).
  - `Store::unreviewed_thread_ends` leaves out a failed run with trigger `schedule` that has no `tool_call` event of its own. It is not marked reviewed.
  - Command `dismiss_schedule_attention(dot_id: String) -> Res<()>`, registered after `inbox_count`.
  - Contract: the `DotOverview` sample has every field of `schedule_state` (active, count 3, a next run, a retry at attempt 2, attention on `01J0RUN0`); the `RunUpdated` sample's run is a queued retry (`01J0RUN2`, `retry_of: "01J0RUN"`, payload `attempt: 2`); `Enums.ScheduleStatus`.
  - UI: `T.ScheduleStatus`, `T.ScheduleView` and their types; `DotOverview.schedule_state: ScheduleView | null`; `Inbox.attention` required; `commands.dismissScheduleAttention(dotId)`. The attention object has no schema of its own: `NonNullable<ScheduleView["attention"]>`. `nextRun` in `DotsScreen.tsx` reads `row.schedule_state?.next_fire`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust

use common::{init_repo, spec, wait_status};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::{Config, Error, Runtime};
```

**with:**

```rust

use common::{init_repo, spec, wait_status};
use dots_core::model::{DotOverview, DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::{Config, Error, Runtime};
```

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
#[tokio::test]
async fn dot_overview_shows_active_and_last_runs_and_the_next_fire() {
    let t = start().await;
    let mut s = folder_spec(&t, "sched");
```

**with:**

```rust
#[tokio::test]
async fn dot_overview_shows_active_and_last_runs_and_the_next_fire() {
    let next_fire = |row: &DotOverview| row.schedule_state.as_ref()?.next_fire.clone();
    let t = start().await;
    let mut s = folder_spec(&t, "sched");
```

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
        Some(run.id.as_str())
    );
    let next = row.next_fire.as_deref().expect("next fire");
    assert!(next.contains("T09:00:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == daily.id).unwrap();
    let next = row.next_fire.as_deref().expect("next fire");
    assert!(next.contains("T07:15:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == idle.id).unwrap();
    assert!(row.last_run.is_none() && row.next_fire.is_none());

    // A run waiting on its model shows as active.
```

**with:**

```rust
        Some(run.id.as_str())
    );
    let next = next_fire(row).expect("next fire");
    assert!(next.contains("T09:00:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == daily.id).unwrap();
    let next = next_fire(row).expect("next fire");
    assert!(next.contains("T07:15:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == idle.id).unwrap();
    assert!(row.last_run.is_none() && row.schedule_state.is_none());

    // A run waiting on its model shows as active.
```

**In `crates/dots-core/tests/runtime_schedule_test.rs`, replace:**

```rust
use std::path::PathBuf;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, NeedsAttention, NewRun, Run, RunStatus, ScheduleChange, ScheduleState,
    ScheduleStatus, TriggerKind, WorkspaceMode,
};
use dots_core::schedule::Schedule;
```

**with:**

```rust
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, Local, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    AttentionRun, Dot, DotSpec, NeedsAttention, NewRun, PendingRetry, Run, RunStatus,
    ScheduleChange, ScheduleState, ScheduleStatus, ScheduleView, TriggerKind, WorkspaceMode,
};
use dots_core::schedule::Schedule;
```

**Append to `crates/dots-core/tests/runtime_schedule_test.rs`:**

```rust

/// Every hour from 2030-01-07T09:00Z, bounded by `until`.
fn hourly(until: serde_json::Value) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": 60, "anchor": "2030-01-07T09:00:00Z"},
        "catch_up": true,
        "until": until
    }))
    .unwrap()
}

fn row_at(last_slot_at: &str, fired_count: u32) -> ScheduleState {
    ScheduleState {
        dot_id: "d".into(),
        rule_key: String::new(),
        status: ScheduleStatus::Active,
        last_slot_at: last_slot_at.into(),
        fired_count,
        watch_run_id: None,
        retry: None,
        attention_run_id: None,
        updated_at: last_slot_at.into(),
    }
}

fn utc(t: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc)
}

/// Scheduler v2 spec §8: what the Dots screen is told about a schedule. The next fire time is
/// the first slot after now, none once nothing more fires; times are in the view's zone.
#[test]
fn a_schedule_view_says_what_comes_next() {
    let now = utc("2030-01-07T10:30:00Z");
    let s = hourly(json!(null));
    let view = ScheduleView::new(
        &row_at("2030-01-07T10:00:00.000Z", 1),
        &s,
        true,
        None,
        now,
        &Utc,
    );
    assert_eq!(
        view,
        ScheduleView {
            status: ScheduleStatus::Active,
            fired_count: 1,
            next_fire: Some("2030-01-07T11:00:00+00:00".into()),
            retry_at: None,
            retry_attempt: None,
            attention: None,
        }
    );
    // A slot the pass has not handled yet is past: it is not the next one.
    let view = ScheduleView::new(
        &row_at("2030-01-07T09:00:00.000Z", 0),
        &s,
        true,
        None,
        now,
        &Utc,
    );
    assert_eq!(view.next_fire.as_deref(), Some("2030-01-07T11:00:00+00:00"));

    // A waiting retry and the attention, in the view's zone.
    let mut row = row_at("2030-01-07T10:00:00.000Z", 1);
    row.retry = Some(PendingRetry {
        slot_at: "2030-01-07T10:00:00.000Z".into(),
        attempt: 2,
        at: "2030-01-07T10:31:00.000Z".into(),
    });
    let flag = AttentionRun {
        run_id: "r1".into(),
        error: Some("down".into()),
    };
    let plus_one = FixedOffset::east_opt(3600).unwrap();
    let view = ScheduleView::new(&row, &s, true, Some(flag.clone()), now, &plus_one);
    assert_eq!(
        (
            view.next_fire.as_deref(),
            view.retry_at.as_deref(),
            view.retry_attempt
        ),
        (
            Some("2030-01-07T12:00:00+01:00"),
            Some("2030-01-07T11:31:00+01:00"),
            Some(2)
        )
    );
    assert_eq!(view.attention, Some(flag.clone()));

    // A disabled dot fires and retries nothing; its flag stays.
    let view = ScheduleView::new(&row, &s, false, Some(flag.clone()), now, &Utc);
    assert_eq!(
        (
            view.next_fire,
            view.retry_at,
            view.retry_attempt,
            view.attention
        ),
        (None, None, None, Some(flag))
    );
    // Neither does a schedule whose count is reached (its last outcome may still be pending),
    // nor a completed or expired one.
    let counted = hourly(json!({"count": 1}));
    let view = ScheduleView::new(
        &row_at("2030-01-07T10:00:00.000Z", 1),
        &counted,
        true,
        None,
        now,
        &Utc,
    );
    assert_eq!(
        (view.status, view.next_fire),
        (ScheduleStatus::Active, None)
    );
    for status in [ScheduleStatus::Completed, ScheduleStatus::Expired] {
        let mut row = row_at("2030-01-07T10:00:00.000Z", 1);
        row.status = status;
        let view = ScheduleView::new(&row, &s, true, None, now, &Utc);
        assert_eq!((view.status, view.next_fire), (status, None));
    }
}

/// `dot_overview` gives each dot with a schedule the view of its row, with the error of the
/// run that needs attention, at the runtime clock's time; a dot without a schedule has none.
#[tokio::test]
async fn the_overview_shows_where_each_schedule_stands() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = utc("2030-01-07T09:00:00Z");
    let mut cfg = config(&dir);
    cfg.clock = FakeClock::new(t0);
    let rt = Runtime::start(cfg).await.unwrap();
    let on = rt
        .create_dot(folder_spec(&dir, "on", Some(hourly(json!(null)))))
        .await
        .unwrap();
    let mut s = folder_spec(&dir, "off", Some(hourly(json!(null))));
    s.enabled = false;
    let off = rt.create_dot(s).await.unwrap();
    let plain = rt
        .create_dot(folder_spec(&dir, "plain", None))
        .await
        .unwrap();
    let failed = flag(&rt, &on, "model server down").await;
    let change = ScheduleChange {
        retry: Some(Some(PendingRetry {
            slot_at: timestamp(t0),
            attempt: 2,
            at: timestamp(t0 + TimeDelta::minutes(5)),
        })),
        ..Default::default()
    };
    rt.store()
        .update_schedule_state(&on.id, &change, t0)
        .await
        .unwrap()
        .unwrap();

    let rows = rt.dot_overview().await.unwrap();
    let view = |dot: &Dot| {
        rows.iter()
            .find(|r| r.dot.id == dot.id)
            .unwrap()
            .schedule_state
            .clone()
    };
    let local = |t: DateTime<Utc>| t.with_timezone(&Local).to_rfc3339();
    assert_eq!(
        view(&on),
        Some(ScheduleView {
            status: ScheduleStatus::Active,
            fired_count: 0,
            next_fire: Some(local(t0 + TimeDelta::hours(1))),
            retry_at: Some(local(t0 + TimeDelta::minutes(5))),
            retry_attempt: Some(2),
            attention: Some(AttentionRun {
                run_id: failed.id,
                error: Some("model server down".into()),
            }),
        })
    );
    assert_eq!(
        view(&off).map(|v| (v.status, v.next_fire)),
        Some((ScheduleStatus::Active, None))
    );
    assert_eq!(view(&plain), None);
    rt.shutdown();
}
```

**Append to `crates/dots-core/tests/store_runs_test.rs`:**

```rust

/// A failed run of `trigger` for `dot`, retrying `retry_of`; with a `tool_call` event when
/// `called` is set.
async fn failed_run(
    store: &Store,
    dot: &str,
    trigger: TriggerKind,
    retry_of: Option<&str>,
    called: bool,
) -> Run {
    let mut new = NewRun::new(dot, trigger);
    new.retry_of = retry_of.map(str::to_string);
    let run = store.create_run(&new).await.unwrap();
    if called {
        let call = json!({"kind": "tool_call", "id": "c1", "tool": "Bash", "input": {}});
        store
            .append_event(&run.id, "tool_call", &call)
            .await
            .unwrap();
    }
    store
        .finish_run(&run.id, RunStatus::Failed, None, Some("down"))
        .await
        .unwrap()
}

/// A failed scheduled run that called no tool changed nothing, and its occurrence is retried
/// or ends in "Needs attention", so it is not a run to review (Scheduler v2). Any other
/// failure is.
#[tokio::test]
async fn a_failed_scheduled_run_that_called_no_tool_is_not_for_review() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let quiet = failed_run(&store, &dot.id, TriggerKind::Schedule, None, false).await;
    let retry = failed_run(
        &store,
        &dot.id,
        TriggerKind::Schedule,
        Some(&quiet.id),
        false,
    )
    .await;
    let called = failed_run(&store, &dot.id, TriggerKind::Schedule, None, true).await;
    let manual = failed_run(&store, &dot.id, TriggerKind::Manual, None, false).await;
    let hook = failed_run(&store, &dot.id, TriggerKind::Webhook, None, false).await;

    let mut ids: Vec<String> = store
        .unreviewed_thread_ends()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    ids.sort();
    let mut want = vec![called.id, manual.id, hook.id];
    want.sort();
    assert_eq!(ids, want, "not {} nor its retry {}", quiet.id, retry.id);
}
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
      ["decide_run_approvals", { runId: "r1", approved: true, note: null }],
    ]);
  });

```

**with:**

```ts
      ["decide_run_approvals", { runId: "r1", approved: true, note: null }],
    ]);
  });

  it("dismisses a schedule's attention by its dot", async () => {
    reply = async () => null;
    expect(await commands.dismissScheduleAttention("d1")).toBeNull();
    expect(calls).toEqual([["dismiss_schedule_attention", { dotId: "d1" }]]);
  });

```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  Preset: T.Preset,
  FileStatus: T.FileStatus,
  CmdErrorKind: T.CmdErrorKind,
};
```

**with:**

```ts
  Preset: T.Preset,
  FileStatus: T.FileStatus,
  ScheduleStatus: T.ScheduleStatus,
  CmdErrorKind: T.CmdErrorKind,
};
```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
    const next = "2026-10-20T01:00:00.000Z";
    const read = later<DotOverview[]>();
    backend.on("dot_overview", () =>
      backend.callsTo("dot_overview").length === 1 ? read.answer : [{ ...idle, next_fire: next }],
    );
    await user.click(screen.getByRole("link", { name: "Dots" }));
```

**with:**

```tsx
    const next = "2026-10-20T01:00:00.000Z";
    const read = later<DotOverview[]>();
    const moved: DotOverview = {
      ...idle,
      schedule_state: { ...idle.schedule_state!, next_fire: next },
    };
    backend.on("dot_overview", () =>
      backend.callsTo("dot_overview").length === 1 ? read.answer : [moved],
    );
    await user.click(screen.getByRole("link", { name: "Dots" }));
```

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts

describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [] };
  /// Shows the Inbox as the app's frame does.
  const watchInbox = (qc: Client) =>
```

**with:**

```ts

describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  /// Shows the Inbox as the app's frame does.
  const watchInbox = (qc: Client) =>
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  it("drops the deleted dot's runs from the Inbox and its count", async () => {
    stored();
    let waiting: Inbox = { approvals: [], review_runs: sample.inbox.review_runs };
    backend.on("inbox", () => waiting);
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [] };
      return null;
    });
```

**with:**

```tsx
  it("drops the deleted dot's runs from the Inbox and its count", async () => {
    stored();
    let waiting: Inbox = { approvals: [], review_runs: sample.inbox.review_runs, attention: [] };
    backend.on("inbox", () => waiting);
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [], attention: [] };
      return null;
    });
```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
  active_run: { ...sample.run, id: "run-active", dot_id: "dot-busy", status: "running" },
  last_run: null,
  next_fire: null,
};
const row = (name: string) => screen.getByRole("link", { name }).closest("tr")!;

```

**with:**

```tsx
  active_run: { ...sample.run, id: "run-active", dot_id: "dot-busy", status: "running" },
  last_run: null,
  schedule_state: null,
};
/// `scheduled` with its next fire time at `next_fire`.
const firing = (next_fire: string | null): DotOverview => ({
  ...scheduled,
  schedule_state: { ...scheduled.schedule_state!, next_fire },
});
const row = (name: string) => screen.getByRole("link", { name }).closest("tr")!;

```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
    expect(first.getByText(`${scheduled.dot.engine} · ${scheduled.dot.model}`)).toBeVisible();
    expect(first.getByText("Idle")).toBeVisible();
    expect(first.getByText(localDateTime(scheduled.next_fire))).toBeVisible();
    expect(
      first.getByRole("link", { name: `Open the last run of ${scheduled.dot.name}` }),
```

**with:**

```tsx
    expect(first.getByText(`${scheduled.dot.engine} · ${scheduled.dot.model}`)).toBeVisible();
    expect(first.getByText("Idle")).toBeVisible();
    expect(first.getByText(localDateTime(scheduled.schedule_state!.next_fire))).toBeVisible();
    expect(
      first.getByRole("link", { name: `Open the last run of ${scheduled.dot.name}` }),
```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
  it("says why a scheduled dot will not run", () => {
    const disabled = { ...scheduled, dot: { ...scheduled.dot, enabled: false } };
    expect(nextRun(scheduled, false)).toBe(localDateTime(scheduled.next_fire));
    expect(nextRun(scheduled, true)).toBe("Paused");
    expect(nextRun(disabled, true)).toBe("Disabled");
    expect(nextRun(busy, false)).toBe("No schedule");
    expect(nextRun({ ...scheduled, next_fire: null }, false)).toBe("Not scheduled");
  });

```

**with:**

```tsx
  it("says why a scheduled dot will not run", () => {
    const disabled = { ...scheduled, dot: { ...scheduled.dot, enabled: false } };
    expect(nextRun(scheduled, false)).toBe(localDateTime(scheduled.schedule_state!.next_fire));
    expect(nextRun(scheduled, true)).toBe("Paused");
    expect(nextRun(disabled, true)).toBe("Disabled");
    expect(nextRun(busy, false)).toBe("No schedule");
    expect(nextRun(firing(null), false)).toBe("Not scheduled");
    expect(nextRun({ ...scheduled, schedule_state: null }, false)).toBe("Not scheduled");
  });

```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
    await listening();

    overview = [{ ...scheduled, next_fire: next }];
    backend.on("status", () => quietStatus);
    act(() => backend.emit("status", quietStatus));
```

**with:**

```tsx
    await listening();

    overview = [firing(next)];
    backend.on("status", () => quietStatus);
    act(() => backend.emit("status", quietStatus));
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
describe("the Inbox", () => {
  it("lists what waits: approvals with their dot, and runs to review", async () => {
    inbox({ approvals: [approval], review_runs: [worktreeRun, failedRun] });
    renderApp("/inbox");
    const approvals = await screen.findByRole("region", { name: "Approvals" });
```

**with:**

```tsx
describe("the Inbox", () => {
  it("lists what waits: approvals with their dot, and runs to review", async () => {
    inbox({ approvals: [approval], review_runs: [worktreeRun, failedRun], attention: [] });
    renderApp("/inbox");
    const approvals = await screen.findByRole("region", { name: "Approvals" });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
      approvals: [],
      review_runs: [{ ...failedRun, branch: "dots/gone" }],
    });
    renderApp("/inbox");
```

**with:**

```tsx
      approvals: [],
      review_runs: [{ ...failedRun, branch: "dots/gone" }],
      attention: [],
    });
    renderApp("/inbox");
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("approves a tool call and drops it from the list", async () => {
    const box = inbox({ approvals: [approval], review_runs: [] });
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**with:**

```tsx

  it("approves a tool call and drops it from the list", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("keeps both buttons of a decided approval off until the list has reloaded", async () => {
    inbox({ approvals: [approval], review_runs: [] });
    backend.on("decide_approval", () => ({ ...sample.approval, status: "approved" }));
    const { user } = renderApp("/inbox");
```

**with:**

```tsx

  it("keeps both buttons of a decided approval off until the list has reloaded", async () => {
    inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("decide_approval", () => ({ ...sample.approval, status: "approved" }));
    const { user } = renderApp("/inbox");
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    backend.on("inbox", async () => {
      await held;
      return { approvals: [], review_runs: [] };
    });

```

**with:**

```tsx
    backend.on("inbox", async () => {
      await held;
      return { approvals: [], review_runs: [], attention: [] };
    });

```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    async (button) => {
      const next = { ...approval, id: "apr-next", tool: "Write", input: { file_path: "a.txt" } };
      const box = inbox({ approvals: [approval, next], review_runs: [] });
      backend.on("decide_approval", (args) => {
        box.set({ approvals: [next], review_runs: [] });
        return { ...sample.approval, id: args.id, status: "approved" };
      });
```

**with:**

```tsx
    async (button) => {
      const next = { ...approval, id: "apr-next", tool: "Write", input: { file_path: "a.txt" } };
      const box = inbox({ approvals: [approval, next], review_runs: [], attention: [] });
      backend.on("decide_approval", (args) => {
        box.set({ approvals: [next], review_runs: [], attention: [] });
        return { ...sample.approval, id: args.id, status: "approved" };
      });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("marks a run as reviewed and drops it from the list", async () => {
    const box = inbox({ approvals: [], review_runs: [failedRun] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.run, id: failedRun.id, reviewed_at: sample.run.ended_at };
    });
```

**with:**

```tsx

  it("marks a run as reviewed and drops it from the list", async () => {
    const box = inbox({ approvals: [], review_runs: [failedRun], attention: [] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.run, id: failedRun.id, reviewed_at: sample.run.ended_at };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  it("marks one run as reviewed when the button is double-clicked and the next row moves up", async () => {
    const next = { ...failedRun, id: "run-next" };
    const box = inbox({ approvals: [], review_runs: [failedRun, next] });
    backend.on("mark_reviewed", (args) => {
      box.set({ approvals: [], review_runs: [next] });
      return { ...sample.run, id: args.runId, reviewed_at: sample.run.ended_at };
    });
```

**with:**

```tsx
  it("marks one run as reviewed when the button is double-clicked and the next row moves up", async () => {
    const next = { ...failedRun, id: "run-next" };
    const box = inbox({ approvals: [], review_runs: [failedRun, next], attention: [] });
    backend.on("mark_reviewed", (args) => {
      box.set({ approvals: [], review_runs: [next], attention: [] });
      return { ...sample.run, id: args.runId, reviewed_at: sample.run.ended_at };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("discards a worktree only after a confirmation", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.run, reviewed_at: sample.run.ended_at };
    });
```

**with:**

```tsx

  it("discards a worktree only after a confirmation", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.run, reviewed_at: sample.run.ended_at };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("shows a refused action and keeps the run in the list", async () => {
    inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("mark_reviewed", () => {
      throw { kind: "conflict", message: "the thread is still running" };
```

**with:**

```tsx

  it("shows a refused action and keeps the run in the list", async () => {
    inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("mark_reviewed", () => {
      throw { kind: "conflict", message: "the thread is still running" };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    // The run is gone (its dot was deleted elsewhere): the core refuses, and the Inbox it
    // answers next no longer holds the run.
    const box = inbox({ approvals: [], review_runs: [failedRun] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [] });
      throw { kind: "not_found", message: `run ${failedRun.id}` };
    });
```

**with:**

```tsx
    // The run is gone (its dot was deleted elsewhere): the core refuses, and the Inbox it
    // answers next no longer holds the run.
    const box = inbox({ approvals: [], review_runs: [failedRun], attention: [] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      throw { kind: "not_found", message: `run ${failedRun.id}` };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("drops a run whose discard was refused once the Inbox has reloaded without it", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [] });
      throw { kind: "not_found", message: `run ${worktreeRun.id}` };
    });
```

**with:**

```tsx

  it("drops a run whose discard was refused once the Inbox has reloaded without it", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      throw { kind: "not_found", message: `run ${worktreeRun.id}` };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  // count as stale, or the run view would show it as it is.
  it("loads a run's changes again when its worktree was discarded while they were loading", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("run_thread", () => [sample.run]);
    backend.on("get_dot", () => sample.dot);
```

**with:**

```tsx
  // count as stale, or the run view would show it as it is.
  it("loads a run's changes again when its worktree was discarded while they were loading", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("run_thread", () => [sample.run]);
    backend.on("get_dot", () => sample.dot);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    backend.on("worktree_diff", () => before.answer);
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [] });
      backend.on("worktree_diff", () => ({ files: [], truncated: false, exists: false }));
      return { ...sample.run, reviewed_at: sample.run.ended_at };
```

**with:**

```tsx
    backend.on("worktree_diff", () => before.answer);
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      backend.on("worktree_diff", () => ({ files: [], truncated: false, exists: false }));
      return { ...sample.run, reviewed_at: sample.run.ended_at };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("loads a run's approvals again when one was decided while they were loading", async () => {
    const box = inbox({ approvals: [approval], review_runs: [] });
    const waiting: Run = {
      ...sample.run,
```

**with:**

```tsx

  it("loads a run's approvals again when one was decided while they were loading", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    const waiting: Run = {
      ...sample.run,
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    backend.on("decide_approval", () => {
      const approved: Approval = { ...sample.approval, status: "approved" };
      box.set({ approvals: [], review_runs: [] });
      backend.on("approvals_for_run", () => [approved]);
      return approved;
```

**with:**

```tsx
    backend.on("decide_approval", () => {
      const approved: Approval = { ...sample.approval, status: "approved" };
      box.set({ approvals: [], review_runs: [], attention: [] });
      backend.on("approvals_for_run", () => [approved]);
      return approved;
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("shows a new approval as soon as the dot asks", async () => {
    const box = inbox({ approvals: [], review_runs: [] });
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(0));
    box.set({ approvals: [approval], review_runs: [] });
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
```

**with:**

```tsx

  it("shows a new approval as soon as the dot asks", async () => {
    const box = inbox({ approvals: [], review_runs: [], attention: [] });
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(0));
    box.set({ approvals: [approval], review_runs: [], attention: [] });
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  /// An Inbox with the one approval, whose decision empties it, and the rules the core offers.
  function waiting(rules: RuleCandidate[] = sample.rules) {
    const box = inbox({ approvals: [approval], review_runs: [] });
    backend.on("approval_rule_candidates", () => rules);
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**with:**

```tsx
  /// An Inbox with the one approval, whose decision empties it, and the rules the core offers.
  function waiting(rules: RuleCandidate[] = sample.rules) {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("approval_rule_candidates", () => rules);
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  };
  const mine = { name: `Approvals asked by ${approval.dot_name}` };
  const empty: Inbox = { approvals: [], review_runs: [] };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));

  it("stand together under their dot, and a single one has no batch buttons", async () => {
    // The core lists approvals in the order asked; a call of another run came in between.
    inbox({ approvals: [approval, elsewhere, second], review_runs: [] });
    renderApp("/inbox");
    const group = await screen.findByRole("group", mine);
```

**with:**

```tsx
  };
  const mine = { name: `Approvals asked by ${approval.dot_name}` };
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));

  it("stand together under their dot, and a single one has no batch buttons", async () => {
    // The core lists approvals in the order asked; a call of another run came in between.
    inbox({ approvals: [approval, elsewhere, second], review_runs: [], attention: [] });
    renderApp("/inbox");
    const group = await screen.findByRole("group", mine);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    ["Deny all (2)", "Deny all", false],
  ])("are answered together by %s, after a confirmation that lists them", async (open, confirm, approved) => {
    const box = inbox({ approvals: [approval, second], review_runs: [] });
    backend.on("decide_run_approvals", () => {
      box.set(empty);
```

**with:**

```tsx
    ["Deny all (2)", "Deny all", false],
  ])("are answered together by %s, after a confirmation that lists them", async (open, confirm, approved) => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_run_approvals", () => {
      box.set(empty);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("stay, with their buttons, when the core refuses the batch", async () => {
    inbox({ approvals: [approval, second], review_runs: [] });
    backend.on("decide_run_approvals", () => {
      throw { kind: "internal", message: "database is locked" };
```

**with:**

```tsx

  it("stay, with their buttons, when the core refuses the batch", async () => {
    inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_run_approvals", () => {
      throw { kind: "internal", message: "database is locked" };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("can be answered together again when the run asks for more after a batch", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [] });
    const later1 = { ...approval, id: "apr-4", tool: "Edit", input: { file_path: "c.txt" } };
    const later2 = { ...approval, id: "apr-5", tool: "Read", input: { file_path: "d.txt" } };
    backend.on("decide_run_approvals", () => {
      // The run went on and asked for two more calls before the Inbox reloaded.
      box.set({ approvals: [later1, later2], review_runs: [] });
      return answered.map((a) => ({ ...a, status: "approved" }));
    });
```

**with:**

```tsx

  it("can be answered together again when the run asks for more after a batch", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    const later1 = { ...approval, id: "apr-4", tool: "Edit", input: { file_path: "c.txt" } };
    const later2 = { ...approval, id: "apr-5", tool: "Read", input: { file_path: "d.txt" } };
    backend.on("decide_run_approvals", () => {
      // The run went on and asked for two more calls before the Inbox reloaded.
      box.set({ approvals: [later1, later2], review_runs: [], attention: [] });
      return answered.map((a) => ({ ...a, status: "approved" }));
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("can still be answered one at a time", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [] });
    backend.on("decide_approval", (args) => {
      box.set({ approvals: [second], review_runs: [] });
      return { ...sample.approval, id: args.id, status: "approved" };
    });
```

**with:**

```tsx

  it("can still be answered one at a time", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_approval", (args) => {
      box.set({ approvals: [second], review_runs: [], attention: [] });
      return { ...sample.approval, id: args.id, status: "approved" };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
      ],
      review_runs: [],
    });
    renderApp("/inbox");
```

**with:**

```tsx
      ],
      review_runs: [],
      attention: [],
    });
    renderApp("/inbox");
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  backend.on("startup_error", () => null);
  backend.on("status", () => quietStatus);
  backend.on("inbox", () => ({ approvals: [], review_runs: [] }));
  // An approval card asks for these when it shows; by default the core offers no rule.
  backend.on("approval_rule_candidates", () => []);
```

**with:**

```ts
  backend.on("startup_error", () => null);
  backend.on("status", () => quietStatus);
  backend.on("inbox", () => ({ approvals: [], review_runs: [], attention: [] }));
  // An approval card asks for these when it shows; by default the core offers no rule.
  backend.on("approval_rule_candidates", () => []);
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test runtime_review_test --test runtime_schedule_test --test store_runs_test 2>&1 | grep -E "^error|could not compile" | sort | uniq -c | sort -rn
cargo test -p dots-core --test store_runs_test 2>&1 | grep -E "^test result|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api src/app src/lib/runtime-events.test.ts src/screens 2>&1 | grep -E "Tests |Test Files|× ")
```

Expected: `runtime_schedule_test` and `runtime_review_test` do not compile: 3 × `` no field `schedule_state` on type `&DotOverview` ``, `` unresolved imports `dots_core::model::AttentionRun`, `dots_core::model::ScheduleView` ``. `store_runs_test` compiles, but cargo runs no test while another target fails to build; alone (the second command) it gives `test result: FAILED. 25 passed; 1 failed`, the failing test being `a_failed_scheduled_run_that_called_no_tool_is_not_for_review` (this line is the prototype's own run, not the separate measurement). The UI: `Test Files  4 failed | 8 passed (12)`, `Tests  8 failed | 220 passed (228)`: "knows every ScheduleStatus the core has", "dismisses a schedule's attention by its dot", "loads the Dots list again when schedules were resumed during its first load", and five tests of `DotsScreen.test.tsx` ("lists each dot with its state, next run and last result", "says why a scheduled dot will not run", "shows the new next run when the tray resumes the schedules", "holds only the switch of the dot that is being switched", "holds only the Run button of the dot that is being started").

- [ ] **Step 3: Implement**

The core and the app first, then the fixture (the next step), then `types.ts`, `commands.ts` and `DotsScreen.tsx`.

**In `crates/dots-core/src/model.rs`, replace:**

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::policy::{Policy, Preset};
use crate::schedule::Schedule;
use crate::{Error, Result};

```

**with:**

```rust
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::policy::{Policy, Preset};
use crate::schedule::{next_slot, Schedule, Until};
use crate::{Error, Result};

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
}

/// One row of the Dots screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
```

**with:**

```rust
}

/// The run that left a schedule needing attention, with its error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionRun {
    pub run_id: String,
    pub error: Option<String>,
}

/// Where a dot's schedule stands, as the Dots screen shows it (Scheduler v2 spec §8): its
/// `schedule_state` row, with the times in local time and what comes next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleView {
    pub status: ScheduleStatus,
    pub fired_count: u32,
    /// The first slot after now, RFC 3339; `None` when nothing more fires: the dot is
    /// disabled, the schedule is not active, or it has no slot or no count left.
    pub next_fire: Option<String>,
    /// When the waiting retry starts, RFC 3339, and the attempt it is; `None` unless a retry
    /// waits on an enabled dot.
    pub retry_at: Option<String>,
    pub retry_attempt: Option<u32>,
    pub attention: Option<AttentionRun>,
}

impl ScheduleView {
    /// The view of `state`, the row of a dot with `schedule`, at `now`, with times in `tz`.
    /// `attention` is the run the row's `attention_run_id` names.
    pub fn new<Tz>(
        state: &ScheduleState,
        schedule: &Schedule,
        enabled: bool,
        attention: Option<AttentionRun>,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> ScheduleView
    where
        Tz: TimeZone + 'static,
        Tz::Offset: std::fmt::Display,
    {
        let parse = |t: &str| DateTime::parse_from_rfc3339(t).ok();
        let counted = match schedule.until {
            Some(Until::Count(n)) => state.fired_count >= n,
            _ => false,
        };
        let live = enabled && state.status == ScheduleStatus::Active;
        // A slot the pass has not handled yet is past: the next fire time is after now.
        let after =
            parse(&state.last_slot_at).map_or(now, |last| now.max(last.with_timezone(&Utc)));
        let next_fire = (live && !counted)
            .then(|| next_slot(schedule, &after, tz).ok().flatten())
            .flatten()
            .map(|t| t.to_rfc3339());
        let retry = state.retry.as_ref().filter(|_| live);
        ScheduleView {
            status: state.status,
            fired_count: state.fired_count,
            next_fire,
            retry_at: retry.map(|r| {
                parse(&r.at).map_or_else(|| r.at.clone(), |at| at.with_timezone(tz).to_rfc3339())
            }),
            retry_attempt: retry.map(|r| r.attempt),
            attention,
        }
    }
}

/// One row of the Dots screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub active_run: Option<Run>,
    pub last_run: Option<Run>,
    /// RFC 3339 local time; `None` without an enabled schedule.
    pub next_fire: Option<String>,
}

```

**with:**

```rust
    pub active_run: Option<Run>,
    pub last_run: Option<Run>,
    /// `None` without a schedule.
    pub schedule_state: Option<ScheduleView>,
}

```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
    /// The newest run of each lineage, if it is finished and not reviewed yet. Only runs the
    /// Inbox can ever show (failed, or with a branch) are returned; `Runtime::inbox` filters further.
    pub async fn unreviewed_thread_ends(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
```

**with:**

```rust
    /// The newest run of each lineage, if it is finished and not reviewed yet. Only runs the
    /// Inbox can ever show (failed, or with a branch) are returned; `Runtime::inbox` filters further.
    /// A failed scheduled run that called no tool is left out (Scheduler v2): it changed
    /// nothing, and its occurrence is retried or ends in "Needs attention".
    pub async fn unreviewed_thread_ends(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
             WHERE r.reviewed_at IS NULL AND r.status IN ('succeeded', 'failed', 'cancelled') \
             AND (r.status = 'failed' OR r.branch IS NOT NULL) \
             AND r.id = (SELECT x.id FROM runs x WHERE x.root_run_id = r.root_run_id \
                         ORDER BY x.queued_at DESC, x.id DESC LIMIT 1) \
```

**with:**

```rust
             WHERE r.reviewed_at IS NULL AND r.status IN ('succeeded', 'failed', 'cancelled') \
             AND (r.status = 'failed' OR r.branch IS NOT NULL) \
             AND NOT (r.status = 'failed' AND r.trigger_kind = 'schedule' \
                      AND NOT EXISTS (SELECT 1 FROM run_events e \
                                      WHERE e.run_id = r.id AND e.kind = 'tool_call')) \
             AND r.id = (SELECT x.id FROM runs x WHERE x.root_run_id = r.root_run_id \
                         ORDER BY x.queued_at DESC, x.id DESC LIMIT 1) \
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use std::time::Duration;

use chrono::{Local, Utc};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
```

**with:**

```rust
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::model::{
    Approval, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun, PendingApproval,
    ReviewRun, Run, RunEventRecord, RunStatus, ScheduleChange, SettingsSaved, Status, TriggerKind,
    WorktreeDiff,
};
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{next_slot, upcoming, Schedule};
use crate::scheduler::{Clock, Scheduler, SystemClock};
use crate::server::{self, ServerState};
```

**with:**

```rust
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::model::{
    Approval, AttentionRun, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun,
    PendingApproval, ReviewRun, Run, RunEventRecord, RunStatus, ScheduleChange, ScheduleView,
    SettingsSaved, Status, TriggerKind, WorktreeDiff,
};
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{upcoming, Schedule};
use crate::scheduler::{Clock, Scheduler, SystemClock};
use crate::server::{self, ServerState};
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    }

    /// Every dot with its active and last run and its next scheduled fire time.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = self.clock.now().with_timezone(&Local);
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
            let next_fire = match (&dot.spec.schedule, dot.spec.enabled) {
                (Some(schedule), true) => next_slot(schedule, &now, &Local)
                    .ok()
                    .flatten()
                    .map(|t| t.to_rfc3339()),
                _ => None,
            };
            rows.push(DotOverview {
                active_run: self.store.latest_run(&dot.id, true).await?,
                last_run: self.store.latest_run(&dot.id, false).await?,
                next_fire,
                dot,
            });
        }
        Ok(rows)
    }

```

**with:**

```rust
    }

    /// Every dot with its active and last run and where its schedule stands.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = self.clock.now();
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
            rows.push(DotOverview {
                active_run: self.store.latest_run(&dot.id, true).await?,
                last_run: self.store.latest_run(&dot.id, false).await?,
                schedule_state: self.schedule_view(&dot, now).await?,
                dot,
            });
        }
        Ok(rows)
    }

    /// The view of the dot's schedule row in local time; `None` without a schedule or a row.
    async fn schedule_view(&self, dot: &Dot, now: DateTime<Utc>) -> Result<Option<ScheduleView>> {
        let Some(schedule) = &dot.spec.schedule else {
            return Ok(None);
        };
        let Some(state) = self.store.get_schedule_state(&dot.id).await? else {
            return Ok(None);
        };
        let attention = match &state.attention_run_id {
            Some(id) => match self.store.get_run(id).await {
                Ok(run) => Some(AttentionRun {
                    run_id: run.id,
                    error: run.error,
                }),
                // Deleted since the row was read: its flag went with it.
                Err(Error::NotFound(_)) => None,
                Err(e) => return Err(e),
            },
            None => None,
        };
        Ok(Some(ScheduleView::new(
            &state,
            schedule,
            dot.spec.enabled,
            attention,
            now,
            &Local,
        )))
    }

```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
}

#[tauri::command]
pub async fn worktree_diff(state: State<'_, AppState>, run_id: String) -> Res<WorktreeDiff> {
```

**with:**

```rust
}

/// Clears the "Needs attention" of the dot's schedule.
#[tauri::command]
pub async fn dismiss_schedule_attention(state: State<'_, AppState>, dot_id: String) -> Res<()> {
    Ok(state.rt()?.dismiss_schedule_attention(&dot_id).await?)
}

#[tauri::command]
pub async fn worktree_diff(state: State<'_, AppState>, run_id: String) -> Res<WorktreeDiff> {
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    inbox,
    inbox_count,
    worktree_diff,
    discard_worktree,
```

**with:**

```rust
    inbox,
    inbox_count,
    dismiss_schedule_attention,
    worktree_diff,
    discard_worktree,
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        "Approval": to(approval()),
        "RuntimeEvent": [
            to(RuntimeEvent::RunUpdated { run: run() }),
            to(RuntimeEvent::RunEvent { event: event() }),
            to(RuntimeEvent::ApprovalRequested { approval: approval() }),
```

**with:**

```rust
        "Approval": to(approval()),
        "RuntimeEvent": [
            // A retry, queued: it names the run it retries.
            to(RuntimeEvent::RunUpdated { run: Run {
                id: "01J0RUN2".into(),
                root_run_id: "01J0RUN2".into(),
                payload: Some(json!({"scheduled_for": "2026-10-09T02:00:00+01:00", "attempt": 2})),
                status: RunStatus::Queued,
                session_id: None,
                workspace_path: None,
                branch: None,
                base_commit: None,
                summary: None,
                tokens_in: 0,
                tokens_out: 0,
                started_at: None,
                ended_at: None,
                retry_of: Some("01J0RUN".into()),
                ..run()
            }}),
            to(RuntimeEvent::RunEvent { event: event() }),
            to(RuntimeEvent::ApprovalRequested { approval: approval() }),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        "Settings": to(&settings),
        "SettingsSaved": to(SettingsSaved { settings: settings.clone(), restart_required: true }),
        "DotOverview": to(DotOverview {
            dot: dot(),
            active_run: None,
            last_run: Some(run()),
            next_fire: Some("2026-10-10T02:00:00+01:00".into()),
        }),
        "Inbox": to(Inbox {
```

**with:**

```rust
        "Settings": to(&settings),
        "SettingsSaved": to(SettingsSaved { settings: settings.clone(), restart_required: true }),
        // Every field of the schedule's view is set, so the UI's schema checks each one.
        "DotOverview": to(DotOverview {
            dot: dot(),
            active_run: None,
            last_run: Some(run()),
            schedule_state: Some(ScheduleView {
                status: ScheduleStatus::Active,
                fired_count: 3,
                next_fire: Some("2026-10-10T02:00:00+01:00".into()),
                retry_at: Some("2026-10-09T13:02:00+01:00".into()),
                retry_attempt: Some(2),
                attention: Some(AttentionRun {
                    run_id: "01J0RUN0".into(),
                    error: Some("model server unreachable".into()),
                }),
            }),
        }),
        "Inbox": to(Inbox {
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            "Preset": Preset::ALL,
            "FileStatus": FileStatus::ALL,
            "CmdErrorKind": CmdErrorKind::ALL,
        },
```

**with:**

```rust
            "Preset": Preset::ALL,
            "FileStatus": FileStatus::ALL,
            "ScheduleStatus": ScheduleStatus::ALL,
            "CmdErrorKind": CmdErrorKind::ALL,
        },
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const Preset = z.enum(["read-only", "sandboxed", "trusted", "custom"]);
export const FileStatus = z.enum(["added", "modified", "deleted", "renamed"]);

export const Rule = z.strictObject({ tool: z.string(), action: Action });
```

**with:**

```ts
export const Preset = z.enum(["read-only", "sandboxed", "trusted", "custom"]);
export const FileStatus = z.enum(["added", "modified", "deleted", "renamed"]);
export const ScheduleStatus = z.enum(["active", "completed", "expired"]);

export const Rule = z.strictObject({ tool: z.string(), action: Action });
```

**In `ui/src/api/types.ts`, replace:**

```ts
});

export const DotOverview = z.strictObject({
  dot: Dot,
  active_run: nullable(Run),
  last_run: nullable(Run),
  next_fire: nullable(z.string()),
});

```

**with:**

```ts
});

// Where a dot's schedule stands (Scheduler v2 spec §8). Times are RFC 3339 local.
// `next_fire` is null when nothing more fires (disabled, not active, nothing left);
// `retry_at` and `retry_attempt` are set only while a retry waits.
export const ScheduleView = z.strictObject({
  status: ScheduleStatus,
  fired_count: z.number().int(),
  next_fire: nullable(z.string()),
  retry_at: nullable(z.string()),
  retry_attempt: nullable(z.number().int()),
  attention: nullable(z.strictObject({ run_id: z.string(), error: nullable(z.string()) })),
});

export const DotOverview = z.strictObject({
  dot: Dot,
  active_run: nullable(Run),
  last_run: nullable(Run),
  // Null without a schedule.
  schedule_state: nullable(ScheduleView),
});

```

**In `ui/src/api/types.ts`, replace:**

```ts
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
  // Newest failure first. Optional until the Inbox screen shows it.
  attention: z.array(NeedsAttention).optional(),
});

```

**with:**

```ts
  approvals: z.array(PendingApproval),
  review_runs: z.array(ReviewRun),
  // Newest failure first.
  attention: z.array(NeedsAttention),
});

```

**In `ui/src/api/types.ts`, replace:**

```ts
export type Preset = z.infer<typeof Preset>;
export type FileStatus = z.infer<typeof FileStatus>;
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
```

**with:**

```ts
export type Preset = z.infer<typeof Preset>;
export type FileStatus = z.infer<typeof FileStatus>;
export type ScheduleStatus = z.infer<typeof ScheduleStatus>;
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type Settings = z.infer<typeof Settings>;
export type SettingsSaved = z.infer<typeof SettingsSaved>;
export type DotOverview = z.infer<typeof DotOverview>;
export type PendingApproval = z.infer<typeof PendingApproval>;
```

**with:**

```ts
export type Settings = z.infer<typeof Settings>;
export type SettingsSaved = z.infer<typeof SettingsSaved>;
export type ScheduleView = z.infer<typeof ScheduleView>;
export type DotOverview = z.infer<typeof DotOverview>;
export type PendingApproval = z.infer<typeof PendingApproval>;
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  inbox: () => call(T.Inbox, "inbox"),
  inboxCount: () => call(z.number().int(), "inbox_count"),

  worktreeDiff: (runId: string) => call(T.WorktreeDiff, "worktree_diff", { runId }),
```

**with:**

```ts
  inbox: () => call(T.Inbox, "inbox"),
  inboxCount: () => call(z.number().int(), "inbox_count"),
  /// Clears the "Needs attention" of the dot's schedule; nothing happens when it is not set.
  /// Rejects with kind `not_found` when the dot has no schedule.
  dismissScheduleAttention: (dotId: string) =>
    call(nothing, "dismiss_schedule_attention", { dotId }),

  worktreeDiff: (runId: string) => call(T.WorktreeDiff, "worktree_diff", { runId }),
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
  if (!row.dot.enabled) return "Disabled";
  if (schedulesPaused) return "Paused";
  return localDateTime(row.next_fire) || "Not scheduled";
}

```

**with:**

```tsx
  if (!row.dot.enabled) return "Disabled";
  if (schedulesPaused) return "Paused";
  return localDateTime(row.schedule_state?.next_fire) || "Not scheduled";
}

```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.

A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
```

**with:**

```markdown
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.
  From Scheduler v2 a failed run with trigger `schedule` that recorded no `tool_call` event
  is not listed: it changed nothing, and its occurrence is retried or ends in "needs
  attention", which the Inbox lists instead.

A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  note)` decides every pending approval of a run, and `decide_approval` takes `allow_rule`,
  which the core accepts only when it is one of those candidates.
- **Runs, dots and approvals are addressed by id.** `open_run_folder(run_id)` resolves the path
  in core; the UI cannot make the backend open, read or delete an arbitrary path. Two values
```

**with:**

```markdown
  note)` decides every pending approval of a run, and `decide_approval` takes `allow_rule`,
  which the core accepts only when it is one of those candidates.
- Scheduler v2 adds `dismiss_schedule_attention(dot_id)`: it clears the dot's "needs
  attention", answers `not_found` when the dot has no schedule row, and does nothing when the
  flag is not set. In `dot_overview` each row's `next_fire` is replaced by `schedule_state`
  (null without a schedule): `{ status, fired_count, next_fire, retry_at, retry_attempt,
  attention: { run_id, error } | null }`, times in RFC 3339 local. `next_fire` is the first
  slot after now, null while the dot is disabled, when the schedule is not `active`, or when
  no slot or count is left; `retry_at` and `retry_attempt` are set while a retry waits.
- **Runs, dots and approvals are addressed by id.** `open_run_folder(run_id)` resolves the path
  in core; the UI cannot make the backend open, read or delete an arbitrary path. Two values
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  holds a sample of `ApprovalUpdated`, every `ApplyState`, and the rule candidates the core
  derives for the `Approval` sample (`RuleCandidate`). From Scheduler v2 it holds a sample of
  every `RuntimeEvent` type, `ScheduleUpdated` among them, and the UI test checks that.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
```

**with:**

```markdown
  holds a sample of `ApprovalUpdated`, every `ApplyState`, and the rule candidates the core
  derives for the `Approval` sample (`RuleCandidate`). From Scheduler v2 it holds a sample of
  every `RuntimeEvent` type, `ScheduleUpdated` among them (the UI test checks that), every
  `ScheduleStatus`, a queued retry (the `RunUpdated` sample, with `retry_of`), and a
  `DotOverview` sample that sets every field of `schedule_state`.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. `Commands` gains `dismiss_schedule_attention`; the `DotOverview` sample's `next_fire` becomes `schedule_state` with every field set; `Enums` gains `ScheduleStatus` (`active`, `completed`, `expired`); the `RunUpdated` sample's run becomes the queued retry `01J0RUN2`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test runtime_review_test --test runtime_schedule_test --test store_runs_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api src/app src/lib/runtime-events.test.ts src/screens 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `runtime_review_test` 5 passed, `runtime_schedule_test` 6 passed, `store_runs_test` 26 passed; `dots-app` 49 passed; every UI file of the run passes.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 411 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 256 passed in 20 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/contract.rs apps/dots-app/src/lib.rs crates/dots-core/src/model.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/runs.rs crates/dots-core/tests/runtime_review_test.rs crates/dots-core/tests/runtime_schedule_test.rs crates/dots-core/tests/store_runs_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/commands.test.ts ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/app/Layout.test.tsx ui/src/lib/runtime-events.test.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/DotsScreen.test.tsx ui/src/screens/DotsScreen.tsx ui/src/screens/InboxScreen.test.tsx ui/src/test/fixtures.ts
git commit -m "feat(app): show where each schedule stands and dismiss its attention"
```

### Task 7: Build a dot's schedule by kind, with catch-up, a stop and the core's objection

The dot editor's Schedule section becomes a builder (spec section 9): a kind, the fields of that kind, "Catch up a missed run", a Stop, and the next runs from the core. The UI holds back only what cannot be sent (an emptied field, a number out of range); whether the schedule would run is the core's answer, and `preview_schedule` now gives it for a schedule with no slot left too, in the same words as a refused save. That objection blocks a save only for a new or changed schedule: a stored `Once` that has passed is kept with a note, so its dot can still be edited. A stored `Every` is previewed from its own anchor, which is kept while its minutes stay. The section moves out of `DotForm` into its own component. The main spec's editor bullet and the `preview_schedule` passage follow.

**Files:**
- Create: `ui/src/screens/dot-editor/ScheduleEditor.test.tsx`, `ui/src/screens/dot-editor/ScheduleEditor.tsx`
- Modify: `crates/dots-core/src/runtime.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/lib/dot-rules.ts`, `ui/src/screens/DotEditorScreen.tsx`, `ui/src/screens/dot-editor/DotForm.tsx`
- Test: `crates/dots-core/tests/runtime_review_test.rs`, `ui/src/lib/dot-rules.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`, `ui/src/screens/dot-editor/ScheduleEditor.test.tsx`, `ui/src/test/fixtures.ts`

**Interfaces:**
- Consumes: Task 2's `T.Schedule`, `commands.previewSchedule`, `keys.schedulePreview`, `scheduleOf`; the contract's `Schedule` samples; `NativeSelect`, `Switch`, `Label`, `Button`, `Input` under `@/components/ui/`, and `Field`.
- Produces:
  - `Runtime::preview_schedule` calls `check_runs_after(now, Local)` after `fill_anchor`: a schedule with no slot left is `Invalid("this schedule would never run")`; it never answers `[]` now. An `Every` sent with its anchor keeps its phase.
  - `@/lib/dot-rules`: `ScheduleKind` (zod enum `none`, `once`, `every`, `daily`, `weekly`, `cron`, and its type); `kindOf(schedule)`; `scheduleOf(schedule)` only trims a cron expression (an emptied one stays a cron schedule with `expr: ""`); `scheduleError(schedule) -> string | undefined` ("Repeat every 1 minute to 7 days (10080 minutes), in whole minutes.", "Pick a time.", "Pick the date and time of the run.", "Write a cron expression.", "Stop after a whole number of runs, 1 or more.", "Pick the date to stop after."); `localDateTimeIn(days, time, now?) -> "YYYY-MM-DDTHH:MM"`; `everyRule(minutes, stored)` (keeps the stored anchor while the minutes equal the stored ones, `null` otherwise); `withKind(current, kind, stored, now?)` (the stored rule back when its kind is picked again; a new rule takes the time from the previous one or 09:00; defaults once tomorrow at that time, weekly `["mon"]`, every 60 minutes, cron `""`; `catch_up` and `until` carried over; Once gets `until: null`). `specErrors` puts `scheduleError` under `errors.schedule`. `cronSchedule` is removed.
  - `ScheduleEditor({ schedule, stored, onChange, error?, preview })` and `type SchedulePreview = { times, pending, note? }` in `@/screens/dot-editor/ScheduleEditor`: "Kind" (`#dot-schedule-kind`: "None", "Once", "Every few minutes or hours", "Daily", "Weekly", "Cron expression"); Once "Date and time"; Every "Every" with "Unit" (minutes or hours; whole hours open in hours); Daily "Time"; Weekly a fieldset "Days" of seven toggles (`aria-pressed`, full day names) and "Time"; Cron "Cron expression" (`#dot-schedule-cron`); a `Switch` "Catch up a missed run" and a fieldset "Stop" ("Never", "After a number of runs" with "Number of runs", "After a date" with "Stop date", default 10 runs or 30 days ahead at 23:59), both hidden for Once; the error or note in an `aria-live` region; "Next runs".
  - `DotEditorScreen`: previews only when `scheduleError` is undefined; the core's objection goes into `errors.schedule` only when the schedule differs from the stored one (`hashKey`), otherwise it is the preview's `note`.
  - Test fixtures: `sample.schedules` (the contract's six samples, in their order).

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/runtime_review_test.rs`, replace:**

```rust
        "{times:?}"
    );
    // Refused like `DotSpec::validate`; empty when no slot is left.
    let no_day = json!({"rule": {"kind": "weekly", "days": [], "time": "09:00"}});
    assert!(matches!(preview(no_day, 3), Err(Error::Invalid(_))));
    let past = json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}});
    assert_eq!(preview(past, 3).unwrap(), Vec::<String>::new());
}

```

**with:**

```rust
        "{times:?}"
    );
    // An every schedule sent back with its anchor keeps its phase.
    let anchored = json!({"rule": {"kind": "every", "minutes": 60,
        "anchor": "2020-01-01T00:20:00Z"}});
    let times = preview(anchored, 2).unwrap();
    assert!(times.iter().all(|t| t.contains(":20:00")), "{times:?}");
    // Refused like `DotSpec::validate`, and like a create or a change of the schedule when no
    // slot is left, so the editor can say why before a save.
    let no_day = json!({"rule": {"kind": "weekly", "days": [], "time": "09:00"}});
    assert!(matches!(preview(no_day, 3), Err(Error::Invalid(_))));
    let never =
        |v| matches!(preview(v, 3), Err(Error::Invalid(m)) if m == "this schedule would never run");
    assert!(never(
        json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}})
    ));
    assert!(never(json!({"rule": {"kind": "daily", "time": "09:00"},
        "until": {"date": "2020-01-01T08:00"}})));
}

```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
import { describe, expect, it } from "vitest";

import { mcpOf, parseMcp, specErrors, specOf } from "@/lib/dot-rules";
import { blankSpec, sample } from "@/test/fixtures";

```

**with:**

```ts
import { describe, expect, it } from "vitest";

import type { Schedule, ScheduleRule } from "@/api/types";
import {
  everyRule,
  localDateTimeIn,
  mcpOf,
  parseMcp,
  scheduleError,
  scheduleOf,
  specErrors,
  specOf,
  withKind,
} from "@/lib/dot-rules";
import { blankSpec, sample } from "@/test/fixtures";

```

**Append to `ui/src/lib/dot-rules.test.ts`:**

```ts

describe("schedule rules", () => {
  const [once, everyAnchored, , daily, weekly, cron] = sample.schedules;
  const of = (rule: ScheduleRule, more: Partial<Schedule> = {}): Schedule => ({
    rule,
    catch_up: true,
    until: null,
    ...more,
  });
  // A Wednesday, 10:30 local time.
  const now = new Date(2026, 9, 14, 10, 30);

  it("takes the contract's samples, every kind", () => {
    expect(sample.schedules.map((s) => s.rule.kind)).toEqual([
      "once",
      "every",
      "every",
      "daily",
      "weekly",
      "cron",
    ]);
  });

  it("finds what the core cannot be sent, and leaves the rest to it", () => {
    for (const s of sample.schedules) expect(scheduleError(s), s.rule.kind).toBeUndefined();
    expect(scheduleError(null)).toBeUndefined();
    const every = (minutes: number) => scheduleError(of({ kind: "every", minutes, anchor: null }));
    for (const bad of [Number.NaN, 0, 1.5, 10081]) {
      expect(every(bad), String(bad)).toMatch(/whole minutes/);
    }
    expect(every(1)).toBeUndefined();
    expect(every(10080)).toBeUndefined();
    expect(scheduleError(of({ kind: "daily", time: "" }))).toBe("Pick a time.");
    expect(scheduleError(of({ kind: "weekly", days: ["mon"], time: "" }))).toBe("Pick a time.");
    expect(scheduleError(of({ kind: "once", at: "" }))).toBe("Pick the date and time of the run.");
    expect(scheduleError(of({ kind: "cron", expr: "  " }))).toBe("Write a cron expression.");
    const until = (u: Schedule["until"]) => scheduleError({ ...daily, until: u });
    for (const count of [Number.NaN, 0, 2.5]) {
      expect(until({ count }), String(count)).toMatch(/whole number of runs/);
    }
    expect(until({ count: 1 })).toBeUndefined();
    expect(until({ date: "" })).toBe("Pick the date to stop after.");
    // The core answers for these, through the preview.
    expect(scheduleError(of({ kind: "weekly", days: [], time: "09:00" }))).toBeUndefined();
    expect(scheduleError(of({ kind: "cron", expr: "every day" }))).toBeUndefined();
    // And the draft's checks name it.
    expect(specErrors({ ...good, schedule: of({ kind: "daily", time: "" }) }, "").schedule).toBe(
      "Pick a time.",
    );
  });

  it("trims a cron expression and keeps every other kind as it is", () => {
    expect(scheduleOf(of({ kind: "cron", expr: " 0 0 2 * * * " }))).toEqual(
      of({ kind: "cron", expr: "0 0 2 * * *" }),
    );
    for (const s of sample.schedules) expect(scheduleOf(s)).toEqual(s);
    // An emptied cron field stays a cron schedule: the editor says what is missing.
    expect(scheduleOf(of({ kind: "cron", expr: " " }))).toEqual(of({ kind: "cron", expr: "" }));
    expect(scheduleOf(null)).toBeNull();
  });

  it("writes a local date and time some days ahead", () => {
    expect(localDateTimeIn(1, "09:00", now)).toBe("2026-10-15T09:00");
    expect(localDateTimeIn(30, "23:59", now)).toBe("2026-11-13T23:59");
  });

  it("starts each kind with catch-up on and no stop, carrying the time over", () => {
    expect(withKind(null, "daily", null, now)).toEqual(of({ kind: "daily", time: "09:00" }));
    expect(withKind(null, "weekly", null, now)).toEqual(
      of({ kind: "weekly", days: ["mon"], time: "09:00" }),
    );
    expect(withKind(null, "every", null, now)).toEqual(
      of({ kind: "every", minutes: 60, anchor: null }),
    );
    expect(withKind(null, "cron", null, now)).toEqual(of({ kind: "cron", expr: "" }));
    expect(withKind(null, "once", null, now)).toEqual(
      of({ kind: "once", at: "2026-10-15T09:00" }),
    );
    expect(withKind(daily, "none", null, now)).toBeNull();
    expect(withKind(daily, "daily", null, now)).toBe(daily);
    // The time, the catch-up and the stop go along.
    const quiet = { ...weekly, catch_up: false, until: { count: 3 } };
    expect(withKind(quiet, "daily", null, now)).toEqual(
      of({ kind: "daily", time: "07:30" }, { catch_up: false, until: { count: 3 } }),
    );
    // A once has no stop, and always catches up.
    expect(withKind(quiet, "once", null, now)).toEqual(
      of({ kind: "once", at: "2026-10-15T07:30" }),
    );
    expect(withKind(once, "daily", null, now)).toEqual(of({ kind: "daily", time: "09:00" }));
  });

  it("gives back the stored rule when its kind is picked again", () => {
    const away = withKind(cron, "daily", cron, now);
    expect(withKind(away, "cron", cron, now)).toEqual(cron);
    const viaOnce = withKind(everyAnchored, "once", everyAnchored, now);
    expect(withKind(viaOnce, "every", everyAnchored, now)).toEqual(everyAnchored);
  });

  it("keeps the stored anchor of an every rule only while its minutes stay", () => {
    expect(everyRule(90, everyAnchored)).toEqual(everyAnchored.rule);
    expect(everyRule(60, everyAnchored)).toEqual({ kind: "every", minutes: 60, anchor: null });
    expect(everyRule(90, daily)).toEqual({ kind: "every", minutes: 90, anchor: null });
    expect(everyRule(90, null)).toEqual({ kind: "every", minutes: 90, anchor: null });
  });
});
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
const path = `/dots/${dot.id}`;

/// What the cron field saves.
const cron = (expr: string) =>
  ({ rule: { kind: "cron", expr }, catch_up: false, until: null }) satisfies Schedule;

describe("the dot editor, new dot", () => {
```

**with:**

```tsx
const path = `/dots/${dot.id}`;

/// A cron schedule: as stored before Scheduler v2 (no catch-up), or new (catch-up on).
const cron = (expr: string, catch_up = false) =>
  ({ rule: { kind: "cron", expr }, catch_up, until: null }) satisfies Schedule;
const never = "this schedule would never run";

describe("the dot editor, new dot", () => {
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  it("previews the next runs of a schedule and shows the core's objection to a bad one", async () => {
    const { user } = renderApp("/dots/new");
    await user.type(await screen.findByLabelText("Cron expression"), "0 0 2 * * *");
    const next = await screen.findByRole("list", { name: "Next runs" });
    expect(within(next).getAllByRole("listitem").map((li) => li.textContent)).toEqual(
```

**with:**

```tsx
  it("previews the next runs of a schedule and shows the core's objection to a bad one", async () => {
    const { user } = renderApp("/dots/new");
    expect(await screen.findByLabelText("Kind")).toHaveValue("none");
    await user.selectOptions(screen.getByLabelText("Kind"), "cron");
    await user.type(screen.getByLabelText("Cron expression"), "0 0 2 * * *");
    const next = await screen.findByRole("list", { name: "Next runs" });
    expect(within(next).getAllByRole("listitem").map((li) => li.textContent)).toEqual(
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    );
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({
      schedule: cron("0 0 2 * * *"),
      count: 3,
    });
```

**with:**

```tsx
    );
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({
      schedule: cron("0 0 2 * * *", true),
      count: 3,
    });
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  });

  it("saves the cron expression as a cron schedule without catch-up", async () => {
    backend.on("create_dot", (args) => ({ ...dot, ...(args.spec as object), id: "dot-new" }));
    backend.on("get_dot", () => ({ ...dot, id: "dot-new", name: "nightly" }));
    const { user, router } = renderApp("/dots/new");
    await user.type(await screen.findByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
    await user.type(screen.getByLabelText("Cron expression"), " 0 0 2 * * * ");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
```

**with:**

```tsx
  });

  /// Fills in what a new dot needs besides its schedule.
  async function fillBasics(user: ReturnType<typeof renderApp>["user"]) {
    await user.type(await screen.findByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
  }
  const basics = { name: "nightly", instructions: "Run the tests.", workdir: "C:/dev/app" };

  it("saves a new weekly schedule with catch-up on and a stop", async () => {
    backend.on("create_dot", (args) => ({ ...dot, ...(args.spec as object), id: "dot-new" }));
    backend.on("get_dot", () => ({ ...dot, id: "dot-new", name: "nightly" }));
    const { user, router } = renderApp("/dots/new");
    await fillBasics(user);
    await user.selectOptions(screen.getByLabelText("Kind"), "weekly");
    await user.click(screen.getByRole("button", { name: "Friday" }));
    const time = screen.getByLabelText("Time");
    await user.clear(time);
    await user.type(time, "07:15");
    await user.click(screen.getByRole("radio", { name: "After a number of runs" }));
    await user.click(screen.getByRole("button", { name: "Create dot" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    const weekly: Schedule = {
      rule: { kind: "weekly", days: ["mon", "fri"], time: "07:15" },
      catch_up: true,
      until: { count: 10 },
    };
    expect(backend.callsTo("create_dot")).toEqual([
      { spec: { ...blankSpec, ...basics, schedule: weekly } },
    ]);
  });

  it("saves a cron expression without its surrounding spaces", async () => {
    backend.on("create_dot", (args) => ({ ...dot, ...(args.spec as object), id: "dot-new" }));
    backend.on("get_dot", () => ({ ...dot, id: "dot-new", name: "nightly" }));
    const { user, router } = renderApp("/dots/new");
    await fillBasics(user);
    await user.selectOptions(screen.getByLabelText("Kind"), "cron");
    await user.type(screen.getByLabelText("Cron expression"), " 0 0 2 * * * ");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      {
        spec: {
          ...blankSpec,
          name: "nightly",
          instructions: "Run the tests.",
          workdir: "C:/dev/app",
          schedule: cron("0 0 2 * * *"),
        },
      },
    ]);
  });

```

**with:**

```tsx
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      { spec: { ...blankSpec, ...basics, schedule: cron("0 0 2 * * *", true) } },
    ]);
  });

  it("says why a once in the past would never run, before a save", async () => {
    backend.on("preview_schedule", () => {
      throw { kind: "invalid", message: never };
    });
    const { user } = renderApp("/dots/new");
    await fillBasics(user);
    await user.selectOptions(screen.getByLabelText("Kind"), "once");
    const at = screen.getByLabelText("Date and time");
    await user.clear(at);
    await user.type(at, "2020-01-01T09:00");
    expect(await screen.findByText(never)).toBeVisible();
    await waitFor(() =>
      expect(backend.callsTo("preview_schedule").at(-1)).toEqual({
        schedule: { rule: { kind: "once", at: "2020-01-01T09:00" }, catch_up: true, until: null },
        count: 3,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Fix the marked fields");
    expect(backend.callsTo("create_dot")).toEqual([]);
  });

  it("does not preview an emptied field and marks it once a save was tried", async () => {
    const { user } = renderApp("/dots/new");
    await fillBasics(user);
    await user.selectOptions(screen.getByLabelText("Kind"), "every");
    expect(await screen.findByRole("list", { name: "Next runs" })).toBeVisible();
    await user.clear(screen.getByLabelText("Every"));
    await waitFor(() => expect(screen.queryByRole("list", { name: "Next runs" })).toBeNull());
    expect(screen.queryByText(/whole minutes/)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByText(/whole minutes/)).toBeVisible();
    expect(backend.callsTo("create_dot")).toEqual([]);
    // Only whole minutes were sent to the core.
    for (const call of backend.callsTo("preview_schedule")) {
      expect(call).toMatchObject({ schedule: { rule: { kind: "every", minutes: 60 } } });
    }
  });

```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  });

  it("clears the schedule when the cron expression is emptied", async () => {
    stored();
    const { user } = renderApp(path);
    await user.clear(await screen.findByLabelText("Cron expression"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
```

**with:**

```tsx
  });

  it("removes the schedule with the kind None, and wants an emptied cron field", async () => {
    stored();
    const { user } = renderApp(path);
    await user.clear(await screen.findByLabelText("Cron expression"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByText("Write a cron expression.")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([]);

    await user.selectOptions(screen.getByLabelText("Kind"), "none");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  });

  it("shows a schedule it cannot edit yet and keeps it on save", async () => {
    const daily: Schedule = {
      rule: { kind: "weekly", days: ["mon", "fri"], time: "09:00" },
      catch_up: true,
      until: { count: 10 },
    };
    const held = stored();
    held.dot = { ...dot, schedule: daily };
    const { user } = renderApp(path);
    expect(await screen.findByText("Weekly on mon, fri at 09:00, at most 10 runs")).toBeVisible();
    expect(screen.queryByLabelText("Cron expression")).toBeNull();
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({ schedule: daily, count: 3 });

    const instructions = screen.getByLabelText("Instructions");
    await user.clear(instructions);
    await user.type(instructions, "Only summarize.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
```

**with:**

```tsx
  });

  it("previews a stored every schedule from its anchor, kept while the minutes stay", async () => {
    const every = sample.schedules[1]!;
    expect(every.rule).toMatchObject({ kind: "every", anchor: "2026-10-09T12:00:00Z" });
    const held = stored();
    held.dot = { ...dot, schedule: every };
    const { user } = renderApp(path);
    expect(await screen.findByLabelText("Every")).toHaveValue(90);
    await waitFor(() =>
      expect(backend.callsTo("preview_schedule").at(-1)).toEqual({ schedule: every, count: 3 }),
    );

    await typeInstructions(user, "Only summarize.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot").at(-1)).toEqual({
      id: dot.id,
      spec: { ...specOf(held.dot), instructions: "Only summarize." },
    });

    // New minutes: the core starts the phase again.
    const minutes = screen.getByLabelText("Every");
    await user.clear(minutes);
    await user.type(minutes, "45");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(2));
    expect(backend.callsTo("update_dot")[1]).toMatchObject({
      spec: { schedule: { ...every, rule: { kind: "every", minutes: 45, anchor: null } } },
    });
  });

  it("keeps a stored once that has passed, and refuses another past date", async () => {
    backend.on("preview_schedule", () => {
      throw { kind: "invalid", message: never };
    });
    const once: Schedule = {
      rule: { kind: "once", at: "2020-01-01T09:00" },
      catch_up: true,
      until: null,
    };
    const held = stored();
    held.dot = { ...dot, schedule: once };
    const { user } = renderApp(path);
    // Unchanged, the core keeps it as it is: what it says is a note, not an error.
    expect(await screen.findByText(never)).toBeVisible();
    await typeInstructions(user, "Only summarize.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    ]);

    // Removed, the cron field is back.
    await user.click(screen.getByRole("button", { name: "Remove schedule" }));
    expect(screen.getByLabelText("Cron expression")).toHaveValue("");
  });

```

**with:**

```tsx
    ]);

    const at = screen.getByLabelText("Date and time");
    await user.clear(at);
    await user.type(at, "2021-01-01T09:00");
    await waitFor(() =>
      expect(backend.callsTo("preview_schedule").at(-1)).toMatchObject({
        schedule: { rule: { at: "2021-01-01T09:00" } },
      }),
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Fix the marked fields");
    expect(backend.callsTo("update_dot")).toHaveLength(1);
  });

```

**Create `ui/src/screens/dot-editor/ScheduleEditor.test.tsx`:**

```tsx
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { Schedule } from "@/api/types";
import { localDateTimeIn } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { sample } from "@/test/fixtures";

const [once, everyAnchored, , daily, weekly, cron] = sample.schedules;
const none: SchedulePreview = { times: [], pending: false };

function setup(
  initial: Schedule | null,
  more: { stored?: Schedule | null; error?: string; preview?: SchedulePreview } = {},
) {
  let latest = initial;
  let changes = 0;
  function Host() {
    const [schedule, setSchedule] = useState(initial);
    latest = schedule;
    return (
      <ScheduleEditor
        schedule={schedule}
        stored={more.stored === undefined ? initial : more.stored}
        onChange={(s) => {
          changes += 1;
          setSchedule(s);
        }}
        error={more.error}
        preview={more.preview ?? none}
      />
    );
  }
  const { unmount } = render(<Host />);
  return { user: userEvent.setup(), schedule: () => latest, changes: () => changes, unmount };
}

const kind = () => screen.getByLabelText("Kind");

describe("the schedule editor", () => {
  it("offers every kind, and no schedule at first", async () => {
    const { user, schedule } = setup(null);
    expect(kind()).toHaveValue("none");
    expect(within(kind()).getAllByRole("option").map((o) => o.getAttribute("value"))).toEqual([
      "none",
      "once",
      "every",
      "daily",
      "weekly",
      "cron",
    ]);
    expect(screen.queryByLabelText("Catch up a missed run")).toBeNull();
    expect(screen.queryByRole("group", { name: "Stop" })).toBeNull();

    await user.selectOptions(kind(), "daily");
    expect(schedule()).toEqual({
      rule: { kind: "daily", time: "09:00" },
      catch_up: true,
      until: null,
    });
    expect(screen.getByLabelText("Time")).toHaveValue("09:00");
    expect(screen.getByLabelText("Catch up a missed run")).toBeChecked();
    expect(screen.getByRole("radio", { name: "Never" })).toBeChecked();

    await user.selectOptions(kind(), "none");
    expect(schedule()).toBeNull();
  });

  it("shows each stored kind as it is and changes nothing by showing it", () => {
    for (const s of sample.schedules) {
      const { changes, unmount } = setup(s);
      expect(kind()).toHaveValue(s.rule.kind);
      expect(changes()).toBe(0);
      unmount();
    }
  });

  it("sets the time and the days of a weekly schedule with toggles", async () => {
    const { user, schedule } = setup(weekly);
    const days = screen.getByRole("group", { name: "Days" });
    const toggles = within(days).getAllByRole("button");
    expect(toggles.map((b) => b.getAttribute("aria-label"))).toEqual([
      "Monday",
      "Tuesday",
      "Wednesday",
      "Thursday",
      "Friday",
      "Saturday",
      "Sunday",
    ]);
    expect(toggles.filter((b) => b.getAttribute("aria-pressed") === "true")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Monday" }));
    await user.click(screen.getByRole("button", { name: "Wednesday" }));
    const wednesday = screen.getByRole("button", { name: "Wednesday" });
    expect(wednesday).toHaveAttribute("aria-pressed", "true");
    expect(schedule()?.rule).toEqual({ kind: "weekly", days: ["wed", "fri"], time: "07:30" });

    // By keyboard too.
    screen.getByRole("button", { name: "Sunday" }).focus();
    await user.keyboard(" ");
    expect(schedule()?.rule).toMatchObject({ days: ["wed", "fri", "sun"] });

    const time = screen.getByLabelText("Time");
    await user.clear(time);
    await user.type(time, "18:45");
    expect(schedule()?.rule).toMatchObject({ time: "18:45" });
    expect(schedule()?.until).toBeNull();
  });

  it("takes minutes or hours, and keeps the anchor while the minutes stay", async () => {
    const { user, schedule } = setup(everyAnchored);
    // 90 minutes are not whole hours.
    expect(screen.getByLabelText("Every")).toHaveValue(90);
    expect(screen.getByLabelText("Unit")).toHaveValue("minutes");

    const every = screen.getByLabelText("Every");
    await user.clear(every);
    await user.type(every, "2");
    await user.selectOptions(screen.getByLabelText("Unit"), "hours");
    expect(schedule()?.rule).toEqual({ kind: "every", minutes: 120, anchor: null });
    expect(screen.getByLabelText("Every")).toHaveValue(2);

    await user.clear(every);
    await user.type(every, "1.5");
    expect(schedule()).toEqual(everyAnchored);

    await user.clear(every);
    expect(schedule()?.rule).toMatchObject({ minutes: Number.NaN });
  });

  it("shows whole hours in hours", () => {
    setup({ ...everyAnchored, rule: { kind: "every", minutes: 180, anchor: null } });
    expect(screen.getByLabelText("Every")).toHaveValue(3);
    expect(screen.getByLabelText("Unit")).toHaveValue("hours");
  });

  it("asks a once schedule for its date only: no catch-up, no stop", async () => {
    const { user, schedule } = setup({ ...daily, catch_up: false });
    await user.selectOptions(kind(), "once");
    expect(schedule()).toEqual({
      rule: { kind: "once", at: localDateTimeIn(1, "09:00") },
      catch_up: true,
      until: null,
    });
    expect(screen.queryByLabelText("Catch up a missed run")).toBeNull();
    expect(screen.queryByRole("group", { name: "Stop" })).toBeNull();

    const at = screen.getByLabelText("Date and time");
    await user.clear(at);
    await user.type(at, "2030-01-02T07:05");
    expect(schedule()?.rule).toEqual({ kind: "once", at: "2030-01-02T07:05" });
  });

  it("stops never, after a number of runs or after a date", async () => {
    const { user, schedule } = setup(daily);
    expect(screen.getByRole("radio", { name: "After a date" })).toBeChecked();
    expect(screen.getByLabelText("Stop date")).toHaveValue("2026-12-31T18:00");

    await user.click(screen.getByRole("radio", { name: "After a number of runs" }));
    expect(schedule()?.until).toEqual({ count: 10 });
    const runs = screen.getByLabelText("Number of runs");
    await user.clear(runs);
    await user.type(runs, "3");
    expect(schedule()?.until).toEqual({ count: 3 });
    expect(screen.queryByLabelText("Stop date")).toBeNull();

    // The stored date comes back.
    await user.click(screen.getByRole("radio", { name: "After a date" }));
    expect(schedule()?.until).toEqual(daily.until);

    await user.click(screen.getByRole("radio", { name: "Never" }));
    expect(schedule()?.until).toBeNull();
    expect(screen.queryByLabelText("Number of runs")).toBeNull();
  });

  it("starts a new date stop a month ahead, at the end of the day", async () => {
    const { user, schedule } = setup(weekly);
    await user.click(screen.getByRole("radio", { name: "After a date" }));
    expect(schedule()?.until).toEqual({ date: localDateTimeIn(30, "23:59") });
    const date = screen.getByLabelText("Stop date");
    await user.clear(date);
    await user.type(date, "2031-05-06T12:00");
    expect(schedule()?.until).toEqual({ date: "2031-05-06T12:00" });
  });

  it("turns catch-up off and on", async () => {
    const { user, schedule } = setup(weekly);
    await user.click(screen.getByLabelText("Catch up a missed run"));
    expect(schedule()?.catch_up).toBe(false);
    expect(schedule()?.rule).toEqual(weekly.rule);
  });

  it("keeps the cron field as it was, and gives the stored rule back", async () => {
    const { user, schedule } = setup(cron);
    expect(screen.getByLabelText("Cron expression")).toHaveValue("0 0 2 * * *");
    // A cron schedule from before Scheduler v2 does not catch up.
    expect(screen.getByLabelText("Catch up a missed run")).not.toBeChecked();
    await user.selectOptions(kind(), "weekly");
    await user.selectOptions(kind(), "cron");
    expect(schedule()).toEqual(cron);
    const field = screen.getByLabelText("Cron expression");
    await user.clear(field);
    await user.type(field, "0 30 9 * * Mon-Fri");
    expect(schedule()?.rule).toEqual({ kind: "cron", expr: "0 30 9 * * Mon-Fri" });
  });

  it("shows the core's objection under the fields, and the next runs", () => {
    const times = ["2026-10-20T09:00:00+02:00"];
    setup(once, { error: "this schedule would never run", preview: { times, pending: false } });
    expect(screen.getByText("this schedule would never run")).toBeVisible();
    const next = screen.getByRole("list", { name: "Next runs" });
    expect(within(next).getByRole("listitem")).toHaveTextContent(localDateTime(times[0]));
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
  startupError: T.StartupError.parse(contract.StartupError),
};

```

**with:**

```ts
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
  startupError: T.StartupError.parse(contract.StartupError),
  /// One schedule of each kind: once; every with an anchor and a count; every without an
  /// anchor; daily until a date; weekly; cron.
  schedules: T.Schedule.array().parse(contract.Schedule),
};

```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test runtime_review_test 2>&1 | grep -E "^test result|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/dot-rules.test.ts src/screens/DotEditorScreen.test.tsx src/screens/dot-editor 2>&1 | grep -E "Tests |Test Files|× ")
```

Expected: `test preview_schedule_lists_the_next_fire_times ... FAILED`, `test result: FAILED. 4 passed; 1 failed`. The UI: `Test Files  3 failed | 1 passed (4)`, `Tests  14 failed | 32 passed (46)`: `ScheduleEditor.test.tsx` fails as a file (the component does not exist), six tests of the "schedule rules" in `dot-rules.test.ts` fail, and eight tests of `DotEditorScreen.test.tsx`, among them "says why a once in the past would never run, before a save" and "keeps a stored once that has passed, and refuses another past date".

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    /// The next `count` (1 to 10) slots of a schedule saved now, RFC 3339 in local time, within
    /// its bounds; empty when it has none. Refuses what `DotSpec::validate` refuses, so the UI
    /// parses no schedule.
    pub fn preview_schedule(schedule: &Schedule, count: usize) -> Result<Vec<String>> {
        schedule.validate()?;
```

**with:**

```rust

    /// The next `count` (1 to 10) slots of a schedule saved now, RFC 3339 in local time, within
    /// its bounds. Refuses what `DotSpec::validate` refuses, and what a create or a change of
    /// the schedule refuses ("this schedule would never run": no slot is left), so the UI
    /// parses no schedule and can say why before a save. An `Every` sent with its anchor keeps
    /// its phase; one without starts now.
    pub fn preview_schedule(schedule: &Schedule, count: usize) -> Result<Vec<String>> {
        schedule.validate()?;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let mut schedule = schedule.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        Ok(upcoming(&schedule, &now, &Local)?
            .take(count.clamp(1, 10))
```

**with:**

```rust
        let mut schedule = schedule.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        schedule.check_runs_after(&now, &Local)?;
        Ok(upcoming(&schedule, &now, &Local)?
            .take(count.clamp(1, 10))
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
import { z } from "zod";

import { LIMITS, type DotSpec, type Schedule } from "@/api/types";

export type SpecErrors = Partial<Record<keyof DotSpec, string>>;
```

**with:**

```ts
import { z } from "zod";

import { LIMITS, type DotSpec, type Schedule, type ScheduleRule } from "@/api/types";

export type SpecErrors = Partial<Record<keyof DotSpec, string>>;
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
  const mcp = mcpOf(spec, mcpText);
  if ("error" in mcp) errors.mcp_servers = mcp.error;
  return errors;
}

/// The schedule the cron field holds once `text` is typed into it: a cron schedule without
/// catch-up or stop, as cron schedules were before Scheduler v2. An empty field is no schedule.
export function cronSchedule(text: string): Schedule | null {
  return text === "" ? null : { rule: { kind: "cron", expr: text }, catch_up: false, until: null };
}

/// The schedule a draft saves and previews: a cron expression without its surrounding spaces,
/// none when only spaces are left. Other kinds as they are.
export function scheduleOf(schedule: Schedule | null): Schedule | null {
  if (schedule?.rule.kind !== "cron") return schedule;
  const expr = schedule.rule.expr.trim();
  return expr === "" ? null : { ...schedule, rule: { kind: "cron", expr } };
}

```

**with:**

```ts
  const mcp = mcpOf(spec, mcpText);
  if ("error" in mcp) errors.mcp_servers = mcp.error;
  const schedule = scheduleError(spec.schedule);
  if (schedule) errors.schedule = schedule;
  return errors;
}

/// The schedule a draft saves and previews: a cron expression without its surrounding spaces.
/// Other kinds as they are.
export function scheduleOf(schedule: Schedule | null): Schedule | null {
  if (schedule?.rule.kind !== "cron") return schedule;
  return { ...schedule, rule: { kind: "cron", expr: schedule.rule.expr.trim() } };
}

/// The schedule editor's kinds: a rule's kind, or no schedule.
export const ScheduleKind = z.enum(["none", "once", "every", "daily", "weekly", "cron"]);
export type ScheduleKind = z.infer<typeof ScheduleKind>;

export const kindOf = (schedule: Schedule | null): ScheduleKind => schedule?.rule.kind ?? "none";

const MAX_EVERY_MINUTES = 7 * 24 * 60;
const everyMinutes = whole(
  1,
  MAX_EVERY_MINUTES,
  "Repeat every 1 minute to 7 days (10080 minutes), in whole minutes.",
);
const runCount = whole(1, 2 ** 32 - 1, "Stop after a whole number of runs, 1 or more.");

/// What is wrong with a schedule that the core cannot even be sent: an emptied field, a
/// number that is not a whole one or out of the core's range. The rest (no day picked, a cron
/// expression that does not parse, a date that leaves no run) the core answers through the
/// preview, so the editor shows its words.
export function scheduleError(schedule: Schedule | null): string | undefined {
  if (schedule === null) return undefined;
  const { rule, until } = schedule;
  switch (rule.kind) {
    case "once":
      if (rule.at === "") return "Pick the date and time of the run.";
      break;
    case "every": {
      const minutes = everyMinutes.safeParse(rule.minutes);
      if (!minutes.success) return minutes.error.issues[0]?.message;
      break;
    }
    case "daily":
    case "weekly":
      if (rule.time === "") return "Pick a time.";
      break;
    case "cron":
      if (rule.expr.trim() === "") return "Write a cron expression.";
      break;
  }
  if (until !== null && "count" in until) {
    const count = runCount.safeParse(until.count);
    if (!count.success) return count.error.issues[0]?.message;
  }
  if (until !== null && "date" in until && until.date === "") {
    return "Pick the date to stop after.";
  }
  return undefined;
}

const pad = (n: number) => String(n).padStart(2, "0");

/// The local date `days` after `now`'s, at `time` (`HH:MM`), as a schedule writes it:
/// `YYYY-MM-DDTHH:MM`.
export function localDateTimeIn(days: number, time: string, now: Date = new Date()): string {
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
  return `${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}T${time}`;
}

/// An every rule of `minutes`. It keeps the stored rule's anchor while the minutes are the
/// stored ones; otherwise it has none, and the core starts the phase when it is saved (spec
/// "Scheduler v2" §2).
export function everyRule(minutes: number, stored: Schedule | null): ScheduleRule {
  const anchor =
    stored?.rule.kind === "every" && stored.rule.minutes === minutes ? stored.rule.anchor : null;
  return { kind: "every", minutes, anchor };
}

function timeOf(rule: ScheduleRule | undefined): string {
  switch (rule?.kind) {
    case "daily":
    case "weekly":
      return rule.time || "09:00";
    case "once":
      return rule.at.slice(11) || "09:00";
    default:
      return "09:00";
  }
}

/// The schedule after `kind` is picked in the editor. Picking the stored schedule's kind again
/// gives its rule back (with an every rule's anchor). A new rule takes the time of the one
/// before, or 09:00: a once is tomorrow, a weekly is on Mondays, an every is hourly, a cron
/// expression is empty. Catch-up (on for a new schedule) and the stop go along; a once has
/// no stop and always catches up, as its checkbox is hidden.
export function withKind(
  current: Schedule | null,
  kind: ScheduleKind,
  stored: Schedule | null,
  now: Date = new Date(),
): Schedule | null {
  if (kind === "none") return null;
  if (current?.rule.kind === kind) return current;
  const fromStored = stored?.rule.kind === kind ? stored : null;
  const time = timeOf(current?.rule);
  const fresh: Record<ScheduleRule["kind"], () => ScheduleRule> = {
    once: () => ({ kind: "once", at: localDateTimeIn(1, time, now) }),
    every: () => everyRule(60, stored),
    daily: () => ({ kind: "daily", time }),
    weekly: () => ({ kind: "weekly", days: ["mon"], time }),
    cron: () => ({ kind: "cron", expr: "" }),
  };
  const rule = fromStored?.rule ?? fresh[kind]();
  if (kind === "once") return { rule, catch_up: fromStored?.catch_up ?? true, until: null };
  const keep = current !== null && current.rule.kind !== "once" ? current : fromStored;
  return { rule, catch_up: keep?.catch_up ?? true, until: keep?.until ?? null };
}

```

**Create `ui/src/screens/dot-editor/ScheduleEditor.tsx`:**

```tsx
// The dot editor's Schedule section (spec "Scheduler v2" §9): a kind, its fields, catch-up,
// the stop, and the next runs as the core computes them. The UI parses no schedule: what is
// wrong with one comes from the core, through the preview, and shows under the fields.
import { useId, useState, type ReactNode } from "react";

import type { Schedule, ScheduleRule, Until, Weekday } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { everyRule, kindOf, localDateTimeIn, ScheduleKind, withKind } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";

/// The core's next runs of the schedule being edited, and what it said about the stored
/// schedule when that is not an error (`note`).
export type SchedulePreview = { times: string[]; pending: boolean; note?: string };

type Rule<K extends ScheduleRule["kind"]> = Extract<ScheduleRule, { kind: K }>;

const KINDS: [ScheduleKind, string][] = [
  ["none", "None"],
  ["once", "Once"],
  ["every", "Every few minutes or hours"],
  ["daily", "Daily"],
  ["weekly", "Weekly"],
  ["cron", "Cron expression"],
];

const DAYS: [Weekday, string][] = [
  ["mon", "Monday"],
  ["tue", "Tuesday"],
  ["wed", "Wednesday"],
  ["thu", "Thursday"],
  ["fri", "Friday"],
  ["sat", "Saturday"],
  ["sun", "Sunday"],
];

/// A fieldset inside the Schedule section, its legend styled like a field's label.
function Group(props: { legend: string; children: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1.5 text-sm leading-none font-medium">{props.legend}</legend>
      {props.children}
    </fieldset>
  );
}

function TimeField(props: { value: string; onChange: (time: string) => void }) {
  return (
    <Field label="Time" htmlFor="dot-schedule-time" hint="In this PC's time zone.">
      <Input
        id="dot-schedule-time"
        type="time"
        className="w-36"
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
      />
    </Field>
  );
}

function OnceFields(props: { rule: Rule<"once">; onChange: (rule: ScheduleRule) => void }) {
  return (
    <Field label="Date and time" htmlFor="dot-schedule-at" hint="In this PC's time zone.">
      <Input
        id="dot-schedule-at"
        type="datetime-local"
        className="w-56"
        value={props.rule.at}
        onChange={(e) => props.onChange({ kind: "once", at: e.target.value })}
      />
    </Field>
  );
}

type Unit = "minutes" | "hours";
const FACTOR: Record<Unit, number> = { minutes: 1, hours: 60 };

/// `n` units in minutes, without the float noise of a decimal number of hours (0.1 h is 6).
function minutesOf(n: number, unit: Unit): number {
  const m = n * FACTOR[unit];
  return Math.abs(m - Math.round(m)) < 1e-9 ? Math.round(m) : m;
}

/// The minutes of an every rule, typed in minutes or in hours. The unit only helps typing: the
/// rule holds minutes. A schedule of whole hours opens in hours.
function EveryFields(props: {
  rule: Rule<"every">;
  stored: Schedule | null;
  onChange: (rule: ScheduleRule) => void;
}) {
  const { minutes } = props.rule;
  const [unit, setUnit] = useState<Unit>(() =>
    minutes >= 60 && minutes % 60 === 0 ? "hours" : "minutes",
  );
  const set = (m: number) => props.onChange(everyRule(m, props.stored));
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor="dot-schedule-every">Every</Label>
      <div className="flex gap-2">
        <Input
          id="dot-schedule-every"
          type="number"
          min={1}
          className="w-28"
          value={Number.isNaN(minutes) ? "" : minutes / FACTOR[unit]}
          onChange={(e) =>
            set(e.target.value === "" ? Number.NaN : minutesOf(Number(e.target.value), unit))
          }
        />
        <NativeSelect
          aria-label="Unit"
          value={unit}
          onChange={(e) => {
            const next: Unit = e.target.value === "hours" ? "hours" : "minutes";
            setUnit(next);
            // The number stays as typed: 2 minutes become 2 hours.
            if (!Number.isNaN(minutes)) set(minutesOf(minutes / FACTOR[unit], next));
          }}
        >
          <option value="minutes">minutes</option>
          <option value="hours">hours</option>
        </NativeSelect>
      </div>
      <p className="text-xs text-muted-foreground">
        Counted in elapsed time from when the schedule is saved; daylight saving does not shift
        it.
      </p>
    </div>
  );
}

function WeeklyFields(props: { rule: Rule<"weekly">; onChange: (rule: ScheduleRule) => void }) {
  const { rule } = props;
  function toggle(day: Weekday) {
    const on = rule.days.includes(day);
    // In week order, whatever order the days were picked in.
    const days = DAYS.map(([d]) => d).filter((d) => (d === day ? !on : rule.days.includes(d)));
    props.onChange({ ...rule, days });
  }
  return (
    <>
      <Group legend="Days">
        <div className="flex flex-wrap gap-1">
          {DAYS.map(([day, name]) => {
            const on = rule.days.includes(day);
            return (
              <Button
                key={day}
                type="button"
                size="sm"
                variant={on ? "default" : "outline"}
                aria-pressed={on}
                aria-label={name}
                onClick={() => toggle(day)}
              >
                {name.slice(0, 3)}
              </Button>
            );
          })}
        </div>
      </Group>
      <TimeField value={rule.time} onChange={(time) => props.onChange({ ...rule, time })} />
    </>
  );
}

function CronField(props: { rule: Rule<"cron">; onChange: (rule: ScheduleRule) => void }) {
  return (
    <Field
      label="Cron expression"
      htmlFor="dot-schedule-cron"
      hint="Six fields: second minute hour day-of-month month day-of-week, and an optional year."
    >
      <Input
        id="dot-schedule-cron"
        className="font-mono"
        placeholder="0 0 2 * * *"
        value={props.rule.expr}
        onChange={(e) => props.onChange({ kind: "cron", expr: e.target.value })}
      />
    </Field>
  );
}

function RuleFields(props: {
  rule: ScheduleRule;
  stored: Schedule | null;
  onChange: (rule: ScheduleRule) => void;
}) {
  const { rule, onChange } = props;
  switch (rule.kind) {
    case "once":
      return <OnceFields rule={rule} onChange={onChange} />;
    case "every":
      return <EveryFields rule={rule} stored={props.stored} onChange={onChange} />;
    case "daily":
      return <TimeField value={rule.time} onChange={(time) => onChange({ ...rule, time })} />;
    case "weekly":
      return <WeeklyFields rule={rule} onChange={onChange} />;
    case "cron":
      return <CronField rule={rule} onChange={onChange} />;
  }
}

/// When the schedule stops: never, after a number of runs, or after a local date and time.
/// Picking a stop the stored schedule has gives its value back.
function StopFields(props: {
  until: Until | null;
  stored: Schedule | null;
  onChange: (until: Until | null) => void;
}) {
  const name = useId();
  const { until, stored } = props;
  const stop = until === null ? "never" : "count" in until ? "count" : "date";
  const storedUntil = stored?.rule.kind === "once" ? null : (stored?.until ?? null);
  const radio = (value: typeof stop, label: string, next: () => Until | null) => (
    <label className="flex items-center gap-2 text-sm">
      <input
        type="radio"
        name={name}
        value={value}
        checked={stop === value}
        onChange={() => props.onChange(next())}
      />
      {label}
    </label>
  );
  return (
    <Group legend="Stop">
      {radio("never", "Never", () => null)}
      {radio("count", "After a number of runs", () =>
        storedUntil && "count" in storedUntil ? storedUntil : { count: 10 },
      )}
      {until !== null && "count" in until && (
        <div className="ml-6">
          <Field label="Number of runs" htmlFor="dot-schedule-count">
            <Input
              id="dot-schedule-count"
              type="number"
              min={1}
              className="w-28"
              value={Number.isNaN(until.count) ? "" : until.count}
              onChange={(e) =>
                props.onChange({
                  count: e.target.value === "" ? Number.NaN : Number(e.target.value),
                })
              }
            />
          </Field>
        </div>
      )}
      {radio("date", "After a date", () =>
        storedUntil && "date" in storedUntil ? storedUntil : { date: localDateTimeIn(30, "23:59") },
      )}
      {until !== null && "date" in until && (
        <div className="ml-6">
          <Field
            label="Stop date"
            htmlFor="dot-schedule-until"
            hint="The last run is at or before this time."
          >
            <Input
              id="dot-schedule-until"
              type="datetime-local"
              className="w-56"
              value={until.date}
              onChange={(e) => props.onChange({ date: e.target.value })}
            />
          </Field>
        </div>
      )}
    </Group>
  );
}

/// The Schedule section's controls. `stored` is the dot's saved schedule (null for a new dot):
/// picking its kind or stop again gives its values back, and an every rule keeps its anchor
/// while its minutes are the stored ones. `error` is what blocks a save.
export function ScheduleEditor(props: {
  schedule: Schedule | null;
  stored: Schedule | null;
  onChange: (schedule: Schedule | null) => void;
  error?: string;
  preview: SchedulePreview;
}) {
  const { schedule, stored, onChange, preview } = props;
  return (
    <div className="flex flex-col gap-3">
      <Field label="Kind" htmlFor="dot-schedule-kind">
        <NativeSelect
          id="dot-schedule-kind"
          value={kindOf(schedule)}
          onChange={(e) => onChange(withKind(schedule, ScheduleKind.parse(e.target.value), stored))}
        >
          {KINDS.map(([kind, label]) => (
            <option key={kind} value={kind}>
              {label}
            </option>
          ))}
        </NativeSelect>
      </Field>
      {schedule && (
        <RuleFields
          rule={schedule.rule}
          stored={stored}
          onChange={(rule) => onChange({ ...schedule, rule })}
        />
      )}
      {schedule && schedule.rule.kind !== "once" && (
        <>
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2">
              <Switch
                id="dot-schedule-catch-up"
                checked={schedule.catch_up}
                onCheckedChange={(catch_up) => onChange({ ...schedule, catch_up })}
              />
              <Label htmlFor="dot-schedule-catch-up">Catch up a missed run</Label>
            </div>
            <p className="text-xs text-muted-foreground">
              When the PC was off or Dots was closed at a run's time, the dot runs once when Dots
              is back, if that is within 24 hours.
            </p>
          </div>
          <StopFields
            until={schedule.until}
            stored={stored}
            onChange={(until) => onChange({ ...schedule, until })}
          />
        </>
      )}
      {/* The core answers while the user types; a screen reader hears it too. */}
      <div aria-live="polite">
        {props.error ? (
          <p className="text-xs text-destructive">{props.error}</p>
        ) : (
          preview.note && <p className="text-xs text-muted-foreground">{preview.note}</p>
        )}
      </div>
      {preview.times.length > 0 && (
        <div className="text-xs text-muted-foreground">
          <span>Next runs:</span>
          <ul aria-label="Next runs" className="mt-1 flex flex-col gap-0.5">
            {preview.times.map((t) => (
              <li key={t}>{localDateTime(t)}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import type { ReactNode } from "react";

import {
  EngineKind,
  WorkspaceMode,
  type Dot,
  type DotSpec,
  type Schedule,
  type ScheduleRule,
} from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
```

**with:**

```tsx
import type { ReactNode } from "react";

import { EngineKind, WorkspaceMode, type Dot, type DotSpec } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { cronSchedule, type SpecErrors } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds: the spec, and the MCP servers as the text being typed.
export type Draft = { spec: DotSpec; mcpText: string };

export type SchedulePreview = { times: string[]; pending: boolean };

const localDate = (dateTime: string) => dateTime.replace("T", " ");

function describeRule(rule: ScheduleRule): string {
  switch (rule.kind) {
    case "once":
      return `Once at ${localDate(rule.at)}`;
    case "every":
      return `Every ${rule.minutes} minutes`;
    case "daily":
      return `Daily at ${rule.time}`;
    case "weekly":
      return `Weekly on ${rule.days.join(", ")} at ${rule.time}`;
    case "cron":
      return `Cron ${rule.expr}`;
  }
}

/// A schedule in words, for the kinds this form cannot edit yet.
function describeSchedule({ rule, until }: Schedule): string {
  const what = describeRule(rule);
  if (until === null) return what;
  return "count" in until
    ? `${what}, at most ${until.count} runs`
    : `${what}, until ${localDate(until.date)}`;
}

function Section(props: { title: string; children: ReactNode }) {
```

**with:**

```tsx
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SpecErrors } from "@/lib/dot-rules";
import { notifyError } from "@/lib/notify";
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds: the spec, and the MCP servers as the text being typed.
export type Draft = { spec: DotSpec; mcpText: string };

function Section(props: { title: string; children: ReactNode }) {
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx

      <Section title="Schedule">
        {spec.schedule && spec.schedule.rule.kind !== "cron" ? (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-3">
              <span className="text-sm">{describeSchedule(spec.schedule)}</span>
              <Button size="sm" variant="outline" onClick={() => set("schedule", null)}>
                Remove schedule
              </Button>
            </div>
            {errors.schedule ? (
              <p className="text-xs text-destructive">{errors.schedule}</p>
            ) : (
              <p className="text-xs text-muted-foreground">
                This editor cannot change this kind of schedule yet. Saving keeps it as it is.
              </p>
            )}
          </div>
        ) : (
          <Field
            label="Cron expression"
            htmlFor="dot-schedule"
            error={errors.schedule}
            hint="Six fields: second minute hour day-of-month month day-of-week. Empty: no schedule."
          >
            <Input
              id="dot-schedule"
              className="font-mono"
              placeholder="0 0 2 * * *"
              value={spec.schedule?.rule.kind === "cron" ? spec.schedule.rule.expr : ""}
              onChange={(e) => set("schedule", cronSchedule(e.target.value))}
            />
          </Field>
        )}
        {props.schedule.times.length > 0 && (
          <div className="text-xs text-muted-foreground">
            <span>Next runs:</span>
            <ul aria-label="Next runs" className="mt-1 flex flex-col gap-0.5">
              {props.schedule.times.map((t) => (
                <li key={t}>{localDateTime(t)}</li>
              ))}
            </ul>
          </div>
        )}
      </Section>

```

**with:**

```tsx

      <Section title="Schedule">
        <ScheduleEditor
          schedule={spec.schedule}
          stored={props.existing?.schedule ?? null}
          onChange={(schedule) => set("schedule", schedule)}
          error={errors.schedule}
          preview={props.schedule}
        />
      </Section>

```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { useState, type ReactNode } from "react";
```

**with:**

```tsx
import { hashKey, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { useState, type ReactNode } from "react";
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { mcpOf, scheduleOf, specErrors, specOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
```

**with:**

```tsx
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { mcpOf, scheduleError, scheduleOf, specErrors, specOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
  const save = useMutation({ mutationFn: props.onSave });

  // The core validates the schedule and computes the preview; the UI parses no schedule.
  const schedule = scheduleOf(useDebounced(draft.spec.schedule, 300));
  const preview = useQuery({
    queryKey: keys.schedulePreview(schedule),
    queryFn: () => (schedule ? commands.previewSchedule(schedule, 3) : []),
    enabled: schedule !== null,
  });

  const errors = specErrors(draft.spec, draft.mcpText);
  if (schedule !== null && preview.isError) errors.schedule = errorMessage(preview.error);

  function submit() {
```

**with:**

```tsx
  const save = useMutation({ mutationFn: props.onSave });

  // The core validates the schedule and computes the preview; the UI parses no schedule. It
  // only holds back one with an emptied field, which the core could not even be sent.
  const schedule = scheduleOf(useDebounced(draft.spec.schedule, 300));
  const sendable = schedule !== null && scheduleError(schedule) === undefined;
  const preview = useQuery({
    queryKey: keys.schedulePreview(schedule),
    queryFn: () => (schedule ? commands.previewSchedule(schedule, 3) : []),
    enabled: sendable,
  });
  // The core refuses a new or changed schedule that would never run, and keeps one saved
  // unchanged (a dot whose once has passed can still be edited). So its objection to the
  // stored schedule is a note, and blocks a save only once the schedule changed.
  const stored = props.existing?.schedule ?? null;
  const changed = hashKey([schedule]) !== hashKey([scheduleOf(stored)]);
  const objection = sendable && preview.isError ? errorMessage(preview.error) : undefined;

  const errors = specErrors(draft.spec, draft.mcpText);
  if (objection && changed) errors.schedule ??= objection;

  function submit() {
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
        draft={draft}
        onChange={setDraft}
        // Until the first save attempt only the schedule's error shows: the core reports it
        // while the user types, and an empty new form should not start out red.
        errors={tried ? errors : errors.schedule ? { schedule: errors.schedule } : {}}
        existing={props.existing}
        endpoints={settings?.local_endpoints ?? []}
        schedule={{
          times: schedule !== null && preview.data ? preview.data : [],
          pending: preview.isFetching,
        }}
      />
```

**with:**

```tsx
        draft={draft}
        onChange={setDraft}
        // Until the first save attempt only the core's objection to the schedule shows: it
        // comes while the user types, and an empty new form should not start out red.
        errors={tried ? errors : objection && changed ? { schedule: objection } : {}}
        existing={props.existing}
        endpoints={settings?.local_endpoints ?? []}
        schedule={{
          times: sendable && preview.data ? preview.data : [],
          pending: preview.isFetching,
          note: changed ? undefined : objection,
        }}
      />
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- **Dots** — list: name, engine/model, state, next scheduled run, last result, enable toggle,
  Run now.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode; cron
  with human-readable preview and next 3 fire times; webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; MCP servers (Claude only);
  use-user-settings toggle.
```

**with:**

```markdown
- **Dots** — list: name, engine/model, state, next scheduled run, last result, enable toggle,
  Run now.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode;
  schedule (from Scheduler v2: a kind picker None / Once / Every / Daily / Weekly / Cron with
  the kind's fields — a date and time; minutes, typed in minutes or hours; a time; seven day
  toggles and a time; the cron text — then "Catch up a missed run" and "Stop" (never, after N
  runs, after a date), both hidden for Once; the next 3 fire times from `preview_schedule`, and
  the core's objection under the fields); webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; MCP servers (Claude only);
  use-user-settings toggle.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   newest first), `inbox_count()` (counts all three), and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, answers `[]` when no slot is left, and
   previews an `every` rule from now; the UI parses no schedule).
7. Review actions on a thread's shared worktree:
   - `worktree_diff(run_id) -> { files: [{ path, status, patch, binary }], truncated, exists }` — `git
```

**with:**

```markdown
   newest first), `inbox_count()` (counts all three), and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, refuses one with no slot left as a create or
   a change does (`invalid`, "this schedule would never run"), and previews an `every` rule
   from the anchor it is sent, or from now without one; the UI parses no schedule).
7. Review actions on a thread's shared worktree:
   - `worktree_diff(run_id) -> { files: [{ path, status, patch, binary }], truncated, exists }` — `git
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - Approvals of the thread show inline.
- The editor validates with zod rules mirroring `DotSpec::validate`; server errors still show
  on save.
- Tests: Vitest + Testing Library + jsdom, with `invoke` and `listen` mocked.

```

**with:**

```markdown
  - Approvals of the thread show inline.
- The editor validates with zod rules mirroring `DotSpec::validate`; server errors still show
  on save. From Scheduler v2 the schedule is checked by the core through `preview_schedule`
  while it is typed; the UI only holds back an emptied field or a number the core cannot
  take. The objection blocks a save when the schedule is new or changed; for the stored
  schedule saved unchanged (a passed Once) it is a note, as the core keeps it.
- Tests: Vitest + Testing Library + jsdom, with `invoke` and `listen` mocked.

```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p dots-core --test runtime_review_test 2>&1 | grep -E "^test result|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/dot-rules.test.ts src/screens/DotEditorScreen.test.tsx src/screens/dot-editor 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `runtime_review_test` 5 passed; the four UI files pass.

- [ ] **Step 5: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 411 passed (the Rust change extends an existing test), `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 278 passed in 21 files, typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/runtime.rs crates/dots-core/tests/runtime_review_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/lib/dot-rules.test.ts ui/src/lib/dot-rules.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/DotEditorScreen.tsx ui/src/screens/dot-editor/DotForm.tsx ui/src/screens/dot-editor/ScheduleEditor.test.tsx ui/src/screens/dot-editor/ScheduleEditor.tsx ui/src/test/fixtures.ts
git commit -m "feat(ui): build a dot's schedule by kind, with catch-up, a stop and the core's objection"
```

### Task 8: Show where each schedule stands, what needs attention, and which slot a run served

The screens show what the core now knows (spec section 9). The Dots list says how an ended schedule ended, flags a dot that needs attention, and shows a waiting retry under the next run. The Inbox gets the group "Needs attention" between the approvals and the runs to review: the dot, the failed run and its error, "Run now" and "Dismiss"; the sidebar's count includes it, as the tray's already does. The run view says under a scheduled run's header which slot it served, that it was caught up, or which retry it is, with a link to the run that failed. The number of attempts comes from the contract's limits, not from the UI. The main spec's UI passages and the contract bullet follow.

**Files:**
- Modify: `apps/dots-app/src/contract.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`, `ui/src/app/Layout.tsx`, `ui/src/components/Transcript.tsx`, `ui/src/lib/format.ts`, `ui/src/screens/DotsScreen.tsx`, `ui/src/screens/InboxScreen.tsx`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `ui/src/app/Layout.test.tsx`, `ui/src/lib/format.test.ts`, `ui/src/screens/DotsScreen.test.tsx`, `ui/src/screens/InboxScreen.test.tsx`, `ui/src/screens/RunScreen.test.tsx`

**Interfaces:**
- Consumes: Task 6's `schedule_state`, `Inbox.attention`, `commands.dismissScheduleAttention`; `commands.runNow`, `onSingleClick`, `notifyError`, `invalidate`, `keys`; `localDateTime`; the contract's queued-retry sample.
- Produces:
  - Contract `Limits.max_schedule_attempts` (= `scheduler::MAX_ATTEMPTS`, 6); UI `LIMITS.max_schedule_attempts`, tied to it by the existing limits test.
  - `localTime(iso)` in `@/lib/format`: the time of day only, with the pass-through rules of `localDateTime`.
  - `Layout`: the Inbox count is `approvals + review_runs + attention`.
  - `DotsScreen`: `nextRun(row, schedulesPaused)` gives "No schedule", "Disabled", then "Completed (missed)" (`completed` with `fired_count == 0`), "Completed", "Expired", and only then "Paused", the next run or "Not scheduled". `retrying(row, schedulesPaused) -> string | null`: "Retrying at <h:mm> (attempt k of 6)", null while the dot is off, schedules are paused or no retry waits. The name cell gets a destructive badge "Needs attention" (its `title` is the error); the retry line stands under the next run.
  - `InboxScreen`: a section "Needs attention" (`aria-labelledby="inbox-attention"`) between "Approvals" and "Runs to review", one item per flag (`aria-label` "<dot> needs attention"): the dot linked to its page, "Scheduled run failed <relative time> · <error>", a link "Open the failed run", "Run now" (starts a run, opens it) and "Dismiss"; both mutations use `onSingleClick`, show a refusal and invalidate `keys.inbox` and `keys.overview`. The Inbox is empty only without attention items too.
  - `Transcript`: under each run's trigger header, for a `schedule` run whose payload parses: "Scheduled for <date and time>", "…, caught up after the PC was off", or for `attempt > 1` "Retry k of 6 of the failed run" (a link to `retry_of`; without it "Retry k of 6"). Nothing for another trigger or a payload it cannot read.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
      expect(within(nav).getByRole("link", { name })).toBeVisible();
    }
    // One pending approval and one run to review.
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*2$/ })).toBeVisible();
  });

```

**with:**

```tsx
      expect(within(nav).getByRole("link", { name })).toBeVisible();
    }
    // One pending approval, one run to review and one schedule that needs attention.
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*3$/ })).toBeVisible();
  });

```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
    backend.on("inbox", () => sample.inbox);
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*2$/ })).toBeVisible();
  });
});
```

**with:**

```tsx
    backend.on("inbox", () => sample.inbox);
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*3$/ })).toBeVisible();
  });
});
```

**In `ui/src/lib/format.test.ts`, replace:**

```ts
import { describe, expect, it } from "vitest";

import { duration, localDateTime, relativeTime } from "./format";

const at = (iso: string) => Date.parse(iso);
```

**with:**

```ts
import { describe, expect, it } from "vitest";

import { duration, localDateTime, localTime, relativeTime } from "./format";

const at = (iso: string) => Date.parse(iso);
```

**In `ui/src/lib/format.test.ts`, replace:**

```ts
    expect(localDateTime("soon")).toBe("soon");
    expect(localDateTime("2026-10-12T10:00:00.000Z")).not.toBe("");
  });
});
```

**with:**

```ts
    expect(localDateTime("soon")).toBe("soon");
    expect(localDateTime("2026-10-12T10:00:00.000Z")).not.toBe("");
    expect(localTime(null)).toBe("");
    expect(localTime("soon")).toBe("soon");
  });

  it("writes the time of day alone, without the date", () => {
    const iso = "2026-10-12T10:04:00.000Z";
    const time = localTime(iso);
    expect(time).not.toBe("");
    expect(localDateTime(iso)).toContain(time);
    expect(localDateTime(iso)).not.toBe(time);
  });
});
```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx

import type { Dot, DotOverview, Run } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { nextRun } from "@/screens/DotsScreen";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const scheduled: DotOverview = sample.overview;
const busy: DotOverview = {
  dot: { ...sample.dot, id: "dot-busy", name: "triage", schedule: null },
```

**with:**

```tsx

import type { Dot, DotOverview, Run } from "@/api/types";
import { localDateTime, localTime } from "@/lib/format";
import { nextRun, retrying } from "@/screens/DotsScreen";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// The contract's sample, whose retry waits and which needs attention.
const troubled: DotOverview = sample.overview;
/// A scheduled dot with nothing waiting.
const scheduled: DotOverview = {
  ...troubled,
  schedule_state: {
    ...troubled.schedule_state!,
    retry_at: null,
    retry_attempt: null,
    attention: null,
  },
};
const busy: DotOverview = {
  dot: { ...sample.dot, id: "dot-busy", name: "triage", schedule: null },
```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
  ...scheduled,
  schedule_state: { ...scheduled.schedule_state!, next_fire },
});
const row = (name: string) => screen.getByRole("link", { name }).closest("tr")!;
```

**with:**

```tsx
  ...scheduled,
  schedule_state: { ...scheduled.schedule_state!, next_fire },
});
/// `scheduled` with the state `status` after `fired_count` slots fired.
const ended = (status: "completed" | "expired", fired_count: number): DotOverview => ({
  ...scheduled,
  schedule_state: { ...scheduled.schedule_state!, status, fired_count, next_fire: null },
});
const row = (name: string) => screen.getByRole("link", { name }).closest("tr")!;
```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
    expect(second.getByText("No schedule")).toBeVisible();
    expect(second.getByText("Never ran")).toBeVisible();
  });

```

**with:**

```tsx
    expect(second.getByText("No schedule")).toBeVisible();
    expect(second.getByText("Never ran")).toBeVisible();
    // Nothing waits for either.
    expect(screen.queryByText("Needs attention")).toBeNull();
    expect(screen.queryByText(/^Retrying at/)).toBeNull();
  });

```

**In `ui/src/screens/DotsScreen.test.tsx`, replace:**

```tsx
    expect(nextRun(firing(null), false)).toBe("Not scheduled");
    expect(nextRun({ ...scheduled, schedule_state: null }, false)).toBe("Not scheduled");
  });

```

**with:**

```tsx
    expect(nextRun(firing(null), false)).toBe("Not scheduled");
    expect(nextRun({ ...scheduled, schedule_state: null }, false)).toBe("Not scheduled");
  });

  it("says that a schedule has ended, and how", () => {
    expect(nextRun(ended("completed", 3), false)).toBe("Completed");
    // A Once whose slot was missed and skipped completed without a run.
    expect(nextRun(ended("completed", 0), false)).toBe("Completed (missed)");
    // A stored cron expression that no longer parses.
    expect(nextRun(ended("expired", 2), false)).toBe("Expired");
    // An ended schedule stays ended while schedules are paused.
    expect(nextRun(ended("completed", 1), true)).toBe("Completed");
    const off = { ...ended("completed", 1), dot: { ...scheduled.dot, enabled: false } };
    expect(nextRun(off, false)).toBe("Disabled");
  });

  it("says when a retry waits, unless nothing can run", () => {
    const at = troubled.schedule_state!.retry_at;
    expect(retrying(troubled, false)).toBe(`Retrying at ${localTime(at)} (attempt 2 of 6)`);
    expect(retrying(scheduled, false)).toBeNull();
    expect(retrying(troubled, true)).toBeNull();
    expect(retrying({ ...troubled, dot: { ...troubled.dot, enabled: false } }, false)).toBeNull();
    expect(retrying({ ...troubled, schedule_state: null }, false)).toBeNull();
  });

  it("flags a dot whose schedule needs attention, and shows its waiting retry", async () => {
    backend.on("dot_overview", () => [troubled, busy]);
    renderApp("/dots");
    await screen.findByRole("link", { name: troubled.dot.name });
    const first = within(row(troubled.dot.name));
    expect(first.getByText("Needs attention")).toBeVisible();
    expect(first.getByText(localDateTime(troubled.schedule_state!.next_fire))).toBeVisible();
    expect(
      first.getByText(
        `Retrying at ${localTime(troubled.schedule_state!.retry_at)} (attempt 2 of 6)`,
      ),
    ).toBeVisible();
    expect(within(row("triage")).queryByText("Needs attention")).toBeNull();
  });

  it("shows how an ended schedule ended in place of the next run", async () => {
    backend.on("dot_overview", () => [ended("completed", 0)]);
    renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });
    expect(within(row(scheduled.dot.name)).getByText("Completed (missed)")).toBeVisible();
  });

```

**Append to `ui/src/screens/InboxScreen.test.tsx`:**

```tsx

describe("a schedule that needs attention", () => {
  const [flagged] = sample.inbox.attention;
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  const only = (attention: Inbox["attention"]): Inbox => ({ ...empty, attention });
  const item = () => screen.findByRole("listitem", { name: `${flagged.dot_name} needs attention` });
  /// Lets what a command's answer set off reach the screen.
  const settle = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));

  it("names the dot, the run that failed and why, and offers Run now and Dismiss", async () => {
    inbox({ approvals: [approval], review_runs: [failedRun], attention: [flagged] });
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "Needs attention" });
    const shown = within(await item());
    expect(within(group).getAllByRole("listitem")).toHaveLength(1);
    expect(shown.getByRole("link", { name: flagged.dot_name })).toHaveAttribute(
      "href",
      `/dots/${flagged.dot_id}`,
    );
    expect(shown.getByRole("link", { name: "Open the failed run" })).toHaveAttribute(
      "href",
      `/runs/${flagged.run_id}`,
    );
    expect(shown.getByText(new RegExp(flagged.error!))).toBeVisible();
    expect(shown.getByRole("button", { name: "Run now" })).toBeVisible();
    expect(shown.getByRole("button", { name: "Dismiss" })).toBeVisible();
    // The group stands between the approvals and the runs to review.
    const groups = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(groups).toEqual(["Approvals", "Needs attention", "Runs to review"]);
  });

  it("is all that waits when nothing else does", async () => {
    inbox(only([{ ...flagged, error: null }]));
    renderApp("/inbox");
    expect(await item()).toBeVisible();
    expect(screen.queryByText(/Nothing waits for you/)).toBeNull();
  });

  it("dismisses the flag, and reloads the Inbox and the Dots list", async () => {
    const box = inbox(only([flagged]));
    backend.on("dismiss_schedule_attention", () => {
      box.set(empty);
      return null;
    });
    const { user, queryClient, listening } = renderApp("/inbox");
    await listening();
    queryClient.setQueryData(keys.overview, [sample.overview]);
    await user.click(within(await item()).getByRole("button", { name: "Dismiss" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("dismiss_schedule_attention")).toEqual([{ dotId: flagged.dot_id }]);
    expect(queryClient.getQueryState(keys.overview)?.isInvalidated).toBe(true);
  });

  it("dismisses one flag when the button is double-clicked and the next item moves up", async () => {
    const next = { ...flagged, dot_id: "dot-other", dot_name: "tidy-notes", run_id: "run-other" };
    const box = inbox(only([flagged, next]));
    backend.on("dismiss_schedule_attention", () => {
      box.set(only([next]));
      return null;
    });
    renderApp("/inbox");
    fireEvent.click(within(await item()).getByRole("button", { name: "Dismiss" }), { detail: 1 });
    await waitFor(() =>
      expect(screen.getAllByRole("listitem", { name: /needs attention/ })).toHaveLength(1),
    );
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }), { detail: 2 });
    await settle();
    expect(backend.callsTo("dismiss_schedule_attention")).toEqual([{ dotId: flagged.dot_id }]);
  });

  it("keeps the flag and says why when the core refuses the dismissal", async () => {
    inbox(only([flagged]));
    backend.on("dismiss_schedule_attention", () => {
      throw { kind: "not_found", message: `schedule of dot ${flagged.dot_id}` };
    });
    const { user } = renderApp("/inbox");
    await user.click(within(await item()).getByRole("button", { name: "Dismiss" }));
    expect(await screen.findByText(`schedule of dot ${flagged.dot_id}`)).toBeVisible();
    expect(await item()).toBeVisible();
  });

  it("starts a run of the dot, opens it and reloads the Inbox", async () => {
    inbox(only([flagged]));
    backend.on("run_now", () => ({ ...sample.run, id: "run-new", status: "queued" }));
    backend.on("run_thread", () => [{ ...sample.run, id: "run-new", status: "queued" }]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    backend.on("approvals_for_run", () => []);
    backend.on("worktree_diff", () => sample.diff);
    const { user, router, listening } = renderApp("/inbox");
    await listening();
    const loads = backend.callsTo("inbox").length;
    await user.click(within(await item()).getByRole("button", { name: "Run now" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/run-new"));
    expect(backend.callsTo("run_now")).toEqual([{ dotId: flagged.dot_id }]);
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(loads));
  });

  it("shows why a run could not start, and keeps the flag", async () => {
    inbox(only([flagged]));
    backend.on("run_now", () => {
      throw { kind: "conflict", message: "the dot is disabled" };
    });
    const { user } = renderApp("/inbox");
    await user.click(within(await item()).getByRole("button", { name: "Run now" }));
    expect(await screen.findByText("the dot is disabled")).toBeVisible();
    expect(await item()).toBeVisible();
  });
});
```

**In `ui/src/screens/RunScreen.test.tsx`, replace:**

```tsx

import type { ApplyState, Approval, Run, RunEventRecord, WorktreeDiff } from "@/api/types";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
```

**with:**

```tsx

import type { ApplyState, Approval, Run, RunEventRecord, WorktreeDiff } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
```

**Append to `ui/src/screens/RunScreen.test.tsx`:**

```tsx

describe("the line under a scheduled run's trigger", () => {
  const slot = "2026-10-09T02:00:00+01:00";
  const scheduled: Run = { ...folderRun, payload: { scheduled_for: slot } };
  const section = () => screen.findByRole("region", { name: /^Scheduled run,/ });

  it("says which slot the run was for", async () => {
    thread([scheduled]);
    renderApp(`/runs/${scheduled.id}`);
    expect(
      within(await section()).getByText(`Scheduled for ${localDateTime(slot)}`),
    ).toBeVisible();
  });

  it("says that a missed slot was caught up", async () => {
    const caughtUp: Run = { ...scheduled, payload: { scheduled_for: slot, caught_up: true } };
    thread([caughtUp]);
    renderApp(`/runs/${caughtUp.id}`);
    expect(
      within(await section()).getByText(
        `Scheduled for ${localDateTime(slot)}, caught up after the PC was off`,
      ),
    ).toBeVisible();
  });

  it("says which attempt a retry is, and links the run that failed", async () => {
    const retry: Run = {
      ...scheduled,
      id: "run-retry",
      retry_of: "run-failed",
      payload: { scheduled_for: slot, attempt: 2 },
    };
    thread([retry]);
    renderApp(`/runs/${retry.id}`);
    const shown = within(await section());
    expect(shown.getByText("Retry 2 of 6 of")).toBeVisible();
    expect(shown.getByRole("link", { name: "the failed run" })).toHaveAttribute(
      "href",
      "/runs/run-failed",
    );
    expect(shown.queryByText(/^Scheduled for/)).toBeNull();
  });

  it("says nothing for a run of another trigger, or a payload it cannot read", async () => {
    const manual: Run = { ...folderRun, id: "run-manual", trigger: "manual", payload: null };
    const odd: Run = { ...scheduled, id: "run-odd", payload: { scheduled_for: 7 } };
    thread([manual, odd]);
    renderApp(`/runs/${odd.id}`);
    await screen.findByRole("region", { name: /^Manual run,/ });
    expect(within(await section()).queryByText(/^Scheduled for|^Retry/)).toBeNull();
    expect(screen.queryByText(/^Scheduled for|^Retry/)).toBeNull();
  });
});
```

- [ ] **Step 2: Run them to verify they fail**

```bash
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/app/Layout.test.tsx src/lib/format.test.ts src/screens/DotsScreen.test.tsx src/screens/InboxScreen.test.tsx src/screens/RunScreen.test.tsx 2>&1 | grep -E "Tests |Test Files|× ")
```

Expected: `Test Files  5 failed (5)`, `Tests  18 failed | 100 passed (118)`: the two count tests of `Layout.test.tsx`, two tests of `format.test.ts` (`localTime` does not exist), four of `DotsScreen.test.tsx`, seven of the "a schedule that needs attention" group in `InboxScreen.test.tsx`, and three of `RunScreen.test.tsx` ("says which slot the run was for", "says that a missed slot was caught up", "says which attempt a retry is, and links the run that failed"). "says nothing for a run of another trigger, or a payload it cannot read" passes before the task: it guards the negative side.

- [ ] **Step 3: Implement**

The contract first, then the fixture (the next step), then the UI files of this step.

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
use dots_core::schedule::Schedule;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS};
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
use dots_core::schedule::Schedule;
use dots_core::scheduler::MAX_ATTEMPTS;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS};
use serde_json::{json, Value};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
/// shared enum and `Limits` the validation limits the editor mirrors: the UI parses live data
/// with its schemas, so a variant or limit it does not know must fail a test, not a screen.
/// `NewDotSpec` is the answer of the `new_dot_spec` command, the blank dot of the UI's tests,
```

**with:**

```rust

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
/// shared enum and `Limits` the validation limits the editor mirrors, with the attempts of a
/// scheduled occurrence that the Dots list and the run view name: the UI parses live data
/// with its schemas, so a variant or limit it does not know must fail a test, not a screen.
/// `NewDotSpec` is the answer of the `new_dot_spec` command, the blank dot of the UI's tests,
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            "max_approval_wait_secs": MAX_APPROVAL_WAIT_SECS,
            "max_concurrent_runs": MAX_CONCURRENT_RUNS,
        },
        "StartupError": to(StartupError {
```

**with:**

```rust
            "max_approval_wait_secs": MAX_APPROVAL_WAIT_SECS,
            "max_concurrent_runs": MAX_CONCURRENT_RUNS,
            "max_schedule_attempts": MAX_ATTEMPTS,
        },
        "StartupError": to(StartupError {
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const StartupError = z.strictObject({ message: z.string(), log_dir: z.string() });

/// Validation limits of the core that the dot editor and Settings mirror. The contract test
/// checks them against the Rust constants.
export const LIMITS = {
  max_timeout_secs: 604800,
  max_approval_wait_secs: 280,
  max_concurrent_runs: 16,
} as const;

```

**with:**

```ts
export const StartupError = z.strictObject({ message: z.string(), log_dir: z.string() });

/// Validation limits of the core that the dot editor and Settings mirror, and the attempts a
/// scheduled occurrence has at most, which the Dots list and the run view name. The contract
/// test checks them against the Rust constants.
export const LIMITS = {
  max_timeout_secs: 604800,
  max_approval_wait_secs: 280,
  max_concurrent_runs: 16,
  max_schedule_attempts: 6,
} as const;

```

**In `ui/src/lib/format.ts`, replace:**

```ts
const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/// An RFC 3339 timestamp in the user's locale and time zone. "" for none; the text itself
```

**with:**

```ts
const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const timeOfDay = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });

/// An RFC 3339 timestamp in the user's locale and time zone. "" for none; the text itself
```

**In `ui/src/lib/format.ts`, replace:**

```ts
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : dateTime.format(t);
}

```

**with:**

```ts
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : dateTime.format(t);
}

/// The time of day of an RFC 3339 timestamp, in the user's locale and time zone, for a time
/// that is close enough not to need its date. "" for none; the text itself when it is not a
/// timestamp.
export function localTime(iso: string | null | undefined): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : timeOfDay.format(t);
}

```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
  useRuntimeEvents();
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  const waiting = inbox ? inbox.approvals.length + inbox.review_runs.length : 0;
  return (
    <div className="flex h-screen bg-background text-sm text-foreground">
```

**with:**

```tsx
  useRuntimeEvents();
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  // The same count as the tray's (`inbox_count`).
  const waiting = inbox
    ? inbox.approvals.length + inbox.review_runs.length + inbox.attention.length
    : 0;
  return (
    <div className="flex h-screen bg-background text-sm text-foreground">
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { StatusBadge } from "@/components/StatusBadge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
```

**with:**

```tsx

import { commands } from "@/api/commands";
import { LIMITS, type DotOverview } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { StatusBadge } from "@/components/StatusBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
  TableRow,
} from "@/components/ui/table";
import { localDateTime, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidateDots, keys, setDotEnabledMutation } from "@/lib/query";
```

**with:**

```tsx
  TableRow,
} from "@/components/ui/table";
import { localDateTime, localTime, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidateDots, keys, setDotEnabledMutation } from "@/lib/query";
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
const RUN_NOW = ["run-now"] as const;

/// When the dot runs next on its schedule, or why it will not.
export function nextRun(row: DotOverview, schedulesPaused: boolean): string {
  if (!row.dot.schedule) return "No schedule";
  if (!row.dot.enabled) return "Disabled";
  if (schedulesPaused) return "Paused";
  return localDateTime(row.schedule_state?.next_fire) || "Not scheduled";
}

```

**with:**

```tsx
const RUN_NOW = ["run-now"] as const;

/// When the dot runs next on its schedule, or why it will not. An ended schedule says how it
/// ended, paused or not: a schedule that completed without firing once had its only slot
/// missed and skipped.
export function nextRun(row: DotOverview, schedulesPaused: boolean): string {
  if (!row.dot.schedule) return "No schedule";
  if (!row.dot.enabled) return "Disabled";
  const state = row.schedule_state;
  if (state?.status === "completed") {
    return state.fired_count === 0 ? "Completed (missed)" : "Completed";
  }
  if (state?.status === "expired") return "Expired";
  if (schedulesPaused) return "Paused";
  return localDateTime(state?.next_fire) || "Not scheduled";
}

/// When the retry of the dot's last occurrence starts, and which attempt it is; null when no
/// retry waits, or when none can start (the dot is off or schedules are paused).
export function retrying(row: DotOverview, schedulesPaused: boolean): string | null {
  const state = row.schedule_state;
  if (!row.dot.enabled || schedulesPaused || !state?.retry_at || state.retry_attempt === null) {
    return null;
  }
  const of = LIMITS.max_schedule_attempts;
  return `Retrying at ${localTime(state.retry_at)} (attempt ${state.retry_attempt} of ${of})`;
}

```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
          {overview.data.map((row) => {
            const { dot, active_run: active, last_run: last } = row;
            return (
              <TableRow key={dot.id}>
                <TableCell>
                  <Link to={`/dots/${dot.id}`} className="font-medium hover:underline">
                    {dot.name}
                  </Link>
                </TableCell>
                <TableCell className="text-muted-foreground">
```

**with:**

```tsx
          {overview.data.map((row) => {
            const { dot, active_run: active, last_run: last } = row;
            const paused = status?.schedules_paused ?? false;
            const attention = row.schedule_state?.attention;
            const retry = retrying(row, paused);
            return (
              <TableRow key={dot.id}>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <Link to={`/dots/${dot.id}`} className="font-medium hover:underline">
                      {dot.name}
                    </Link>
                    {/* The Inbox lists the run that failed, with Run now and Dismiss. */}
                    {attention && (
                      <Badge variant="destructive" title={attention.error ?? undefined}>
                        Needs attention
                      </Badge>
                    )}
                  </div>
                </TableCell>
                <TableCell className="text-muted-foreground">
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
                  )}
                </TableCell>
                <TableCell>{nextRun(row, status?.schedules_paused ?? false)}</TableCell>
                <TableCell>
                  {last ? (
```

**with:**

```tsx
                  )}
                </TableCell>
                <TableCell>
                  <div className="flex flex-col">
                    <span>{nextRun(row, paused)}</span>
                    {retry && <span className="text-xs text-muted-foreground">{retry}</span>}
                  </div>
                </TableCell>
                <TableCell>
                  {last ? (
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { ReviewRun } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**with:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { NeedsAttention, ReviewRun } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { toolPreview } from "@/lib/transcript";

```

**with:**

```tsx
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { toolPreview } from "@/lib/transcript";

```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
        canDiscard={run.worktree_exists}
      />
    </li>
  );
```

**with:**

```tsx
        canDiscard={run.worktree_exists}
      />
    </li>
  );
}

/// A schedule whose last occurrence failed in the end: its retries were used up, or its run
/// had called a tool. The schedule keeps firing; the flag goes when a later run of the dot
/// succeeds or when it is dismissed here.
function AttentionRow({ item }: { item: NeedsAttention }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  // The flag shows in the Inbox and in the Dots list. The core also announces the change
  // (`ScheduleUpdated`); this reload does not wait for it.
  const reload = () => {
    for (const queryKey of [keys.inbox, keys.overview]) invalidate(qc, queryKey);
  };
  const runNow = useMutation({
    mutationFn: () => commands.runNow(item.dot_id),
    onSuccess: (run) => void navigate(`/runs/${run.id}`),
    onError: notifyError,
    onSettled: reload,
  });
  const dismiss = useMutation({
    mutationFn: () => commands.dismissScheduleAttention(item.dot_id),
    onError: notifyError,
    onSettled: reload,
  });
  return (
    <li
      aria-label={`${item.dot_name} needs attention`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <Link to={`/dots/${item.dot_id}`} className="font-medium hover:underline">
          {item.dot_name}
        </Link>
        <span className="truncate text-xs text-muted-foreground">
          Scheduled run failed {relativeTime(item.failed_at)}
          {item.error && ` · ${item.error}`}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/runs/${item.run_id}`}>Open the failed run</Link>
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={runNow.isPending}
        onClick={onSingleClick(() => runNow.mutate())}
      >
        <Play aria-hidden /> Run now
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={dismiss.isPending}
        onClick={onSingleClick(() => dismiss.mutate())}
      >
        Dismiss
      </Button>
    </li>
  );
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
}

/// Everything that waits for the user: tool calls to approve, and finished runs to review.
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
```

**with:**

```tsx
}

/// Everything that waits for the user: tool calls to approve, schedules that need attention,
/// and finished runs to review.
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
    );
  }
  const { approvals, review_runs: runs } = inbox.data;
  if (approvals.length === 0 && runs.length === 0) {
    return (
      <Page title="Inbox">
        <p className="text-muted-foreground">
          Nothing waits for you. Approvals a dot asks for and finished runs to review show up
          here.
        </p>
      </Page>
```

**with:**

```tsx
    );
  }
  const { approvals, review_runs: runs, attention } = inbox.data;
  if (approvals.length === 0 && runs.length === 0 && attention.length === 0) {
    return (
      <Page title="Inbox">
        <p className="text-muted-foreground">
          Nothing waits for you. Approvals a dot asks for, schedules that need attention and
          finished runs to review show up here.
        </p>
      </Page>
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
            <RunApprovalsGroup key={group.runId} group={group} />
          ))}
        </section>
      )}
```

**with:**

```tsx
            <RunApprovalsGroup key={group.runId} group={group} />
          ))}
        </section>
      )}
      {attention.length > 0 && (
        <section aria-labelledby="inbox-attention" className="flex flex-col gap-2">
          <h2 id="inbox-attention" className="font-semibold">
            Needs attention
          </h2>
          <ul className="flex flex-col gap-2">
            {attention.map((item) => (
              <AttentionRow key={item.dot_id} item={item} />
            ))}
          </ul>
        </section>
      )}
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
import type { Run, RunEventRecord } from "@/api/types";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
```

**with:**

```tsx
import type { ReactNode } from "react";
import { Link } from "react-router";
import { z } from "zod";

import { LIMITS, type Run, type RunEventRecord } from "@/api/types";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  const message = (run.payload as Record<string, unknown>).message;
  return typeof message === "string" ? message : null;
}

```

**with:**

```tsx
  const message = (run.payload as Record<string, unknown>).message;
  return typeof message === "string" ? message : null;
}

/// What the scheduler wrote into the payload of a run it started: the slot, whether the slot
/// had been missed, and which attempt of the occurrence the run is (a first one has none).
const SchedulePayload = z.object({
  scheduled_for: z.string(),
  caught_up: z.boolean().optional(),
  attempt: z.number().int().optional(),
});

/// The line under a scheduled run's trigger: the slot it served, or the attempt it retries,
/// with a link to the run that failed. Nothing for another trigger or a payload it cannot read.
function ScheduleLine({ run }: { run: Run }) {
  if (run.trigger !== "schedule") return null;
  const payload = SchedulePayload.safeParse(run.payload);
  if (!payload.success) return null;
  const { scheduled_for: slot, caught_up: caughtUp, attempt } = payload.data;
  let line: ReactNode = `Scheduled for ${localDateTime(slot)}`;
  if (caughtUp) line = `${line}, caught up after the PC was off`;
  if (attempt !== undefined && attempt > 1) {
    const retry = `Retry ${attempt} of ${LIMITS.max_schedule_attempts}`;
    line =
      run.retry_of === null ? (
        retry
      ) : (
        <>
          {retry} of{" "}
          <Link to={`/runs/${run.retry_of}`} className="text-foreground hover:underline">
            the failed run
          </Link>
        </>
      );
  }
  return <p className="mb-2 text-xs text-muted-foreground">{line}</p>;
}

```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
              {many && <StatusBadge status={run.status} />}
            </header>
            <div className="flex flex-col gap-2">
              {message !== null && (
```

**with:**

```tsx
              {many && <StatusBadge status={run.status} />}
            </header>
            <ScheduleLine run={run} />
            <div className="flex flex-col gap-2">
              {message !== null && (
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown

- **Dots** — list: name, engine/model, state, next scheduled run, last result, enable toggle,
  Run now.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode;
  schedule (from Scheduler v2: a kind picker None / Once / Every / Daily / Weekly / Cron with
```

**with:**

```markdown

- **Dots** — list: name, engine/model, state, next scheduled run, last result, enable toggle,
  Run now. From Scheduler v2 the next-run column says how an ended schedule ended
  ("Completed", "Completed (missed)" when it completed without firing, "Expired"), and
  "Retrying at 09:04 (attempt 2 of 6)" under it while a retry waits; a dot whose schedule
  needs attention carries a "Needs attention" badge.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode;
  schedule (from Scheduler v2: a kind picker None / Once / Every / Daily / Weekly / Cron with
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  use-user-settings toggle.
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder.
- **Inbox** — pending approvals (tool, pretty-printed input, approve / deny with note) and
  finished runs awaiting review (definition in §12). Unread count mirrored in the tray tooltip.
  From Approvals v2 an approval card, wherever it shows (Inbox, run view, chat), also offers
  "Approve and always allow": the allow rules the core derived for that call, the narrowest
```

**with:**

```markdown
  use-user-settings toggle.
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder. From Scheduler v2 a
  scheduled run's trigger has a line under it: "Scheduled for <slot>", "…, caught up after
  the PC was off", or "Retry 2 of 6 of the failed run" with a link to that run.
- **Inbox** — pending approvals (tool, pretty-printed input, approve / deny with note),
  schedules that need attention (from Scheduler v2) and finished runs awaiting review
  (definition in §12). Unread count mirrored in the tray tooltip and the sidebar.
  From Approvals v2 an approval card, wherever it shows (Inbox, run view, chat), also offers
  "Approve and always allow": the allow rules the core derived for that call, the narrowest
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review + schedules that need
attention.

### Tauri shell (`apps/dots-app`)
```

**with:**

```markdown
A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review + schedules that need
attention; the sidebar's Inbox badge shows the same count.

The "Needs attention" group stands between the approvals and the runs to review. Each item
names the dot (a link to its editor), when the run failed and its error, a link to the failed
run, "Run now" (starts a `manual` run and opens it) and "Dismiss"
(`dismiss_schedule_attention`). Both reload the Inbox and the Dots list when they answer. A
flag leaves the Inbox when it is dismissed or a later run of the dot succeeds.

### Tauri shell (`apps/dots-app`)
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  every `RuntimeEvent` type, `ScheduleUpdated` among them (the UI test checks that), every
  `ScheduleStatus`, a queued retry (the `RunUpdated` sample, with `retry_of`), and a
  `DotOverview` sample that sets every field of `schedule_state`.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
    restart required ("Restart Dots"), schedules paused.
  - While schedules are paused the Dots list shows "paused" in place of the next fire time.
  - Destructive actions (discard worktree, delete dot, regenerate webhook token) ask first in
    an in-app dialog.
```

**with:**

```markdown
  every `RuntimeEvent` type, `ScheduleUpdated` among them (the UI test checks that), every
  `ScheduleStatus`, a queued retry (the `RunUpdated` sample, with `retry_of`), and a
  `DotOverview` sample that sets every field of `schedule_state`; `Limits` gains
  `max_schedule_attempts` (the core's `MAX_ATTEMPTS`, 6), which the Dots list and the run view
  name.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
    restart required ("Restart Dots"), schedules paused.
  - While schedules are paused the Dots list shows "paused" in place of the next fire time.
    From Scheduler v2 an ended schedule shows "Completed", "Completed (missed)" (status
    `completed` with `fired_count` 0) or "Expired" instead, paused or not. A waiting retry
    shows "Retrying at <time> (attempt k of 6)" under it, unless the dot is off or schedules
    are paused; `schedule_state.attention` shows the "Needs attention" badge by the name.
  - Destructive actions (discard worktree, delete dot, regenerate webhook token) ask first in
    an in-app dialog.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - Tool calls and results are paired by id and collapsible.
  - `message` events are hidden; `raw` events are collapsed.
  - The diff is an in-repo unified-diff component with per-file collapse and a truncation
    notice. The core returns git's patch text untrimmed, so a hunk that ends in a blank context
```

**with:**

```markdown
  - Tool calls and results are paired by id and collapsible.
  - `message` events are hidden; `raw` events are collapsed.
  - From Scheduler v2 a run with trigger `schedule` has a line under its trigger, read from
    its payload (`scheduled_for`, `caught_up`, `attempt`) and `retry_of`: "Scheduled for
    <slot>", "Scheduled for <slot>, caught up after the PC was off", or, for attempt 2 or
    more, "Retry k of 6 of the failed run" with a link to the run in `retry_of`. A payload
    that does not read so shows no line.
  - The diff is an in-repo unified-diff component with per-file collapse and a truncation
    notice. The core returns git's patch text untrimmed, so a hunk that ends in a blank context
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. The fixture's `Limits` gains `"max_schedule_attempts": 6`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/contract.test.ts src/app/Layout.test.tsx src/lib/format.test.ts src/screens/DotsScreen.test.tsx src/screens/InboxScreen.test.tsx src/screens/RunScreen.test.tsx 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `dots-app` 49 passed; the six UI files pass.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 411 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 294 passed in 21 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/app/Layout.test.tsx ui/src/app/Layout.tsx ui/src/components/Transcript.tsx ui/src/lib/format.test.ts ui/src/lib/format.ts ui/src/screens/DotsScreen.test.tsx ui/src/screens/DotsScreen.tsx ui/src/screens/InboxScreen.test.tsx ui/src/screens/InboxScreen.tsx ui/src/screens/RunScreen.test.tsx
git commit -m "feat(ui): show where each schedule stands, what needs attention, and which slot a run served"
```

### Task 9: The smoke test of Scheduler v2

The spec asks for a smoke with LM Studio and a temporary data folder (section 12): a dot every minute runs; Dots closed across a slot and started again runs one catch-up run; with the model server down the attempts are retried and then "Needs attention" appears with one notification; the server back and "Run now" clear the flag; a `Once` dot runs once and shows "Completed". This task writes that as a document with the steps an agent runs from a terminal against a debug build (Part A, each with its result) and the steps for the person who owns the machine (Part B, a checklist). It follows the method of `docs/verification/2026-10-13-approvals-v2-smoke.md`, with that smoke's lesson: the app is started with a `PATH` that holds no `claude` CLI, as in `docs/verification/2026-10-13-plan3-smoke.md`. It was not run while this plan was written.

**Files:**
- Create: `docs/verification/2026-10-14-scheduler-v2-smoke.md`

**Interfaces:**
- Consumes: the app of Tasks 1 to 8; `tools/smoke/webview.mjs` (`eval`, `text`, `goto`, `shot`, `click`, `fill`, `wait`); LM Studio's CLI at `~/.lmstudio/bin/lms.exe` with the model `google/gemma-4-12b-qat` downloaded.
- Produces: the document, with a result under every step of Part A.

Times in this task: a schedule names local minutes, so the steps compute "two minutes from now" with `date`. Do not run A5 to A9 within ten minutes of midnight: a daily time two minutes ahead would then be tomorrow's. The model server is never stopped by this smoke: it may be the person's. A dot that should find the server down points at `http://127.0.0.1:9`, where nothing listens; Dots sees exactly what it sees when the server is stopped, a refused connection before any tool call. "The server back" is that dot pointed at port 1234 again.

- [ ] **Step 1: Write the document's frame**

Create `docs/verification/2026-10-14-scheduler-v2-smoke.md` with these sections, in this order: a title "Scheduler v2: smoke test"; a short head (date, build, branch and commit, model, how many steps passed, where the screenshots are); "What this covers" (section 12 of `2026-10-14-scheduler-v2-design.md`); "How the agent steps were run" (the debug build in the shared target folder, a fresh folder under `mktemp -d` as `DOTS_DATA_DIR`, `WEBVIEW2_USER_DATA_FOLDER` moved as well, the debugging port 9333, a `PATH` without the `claude` CLI, every block run in Git Bash from the repository root and starting with `. "${E:?}"`, no argument that starts with `/`, the installed Dots and the real `claude` CLI never touched, the model server never stopped); "Part A: agent steps"; "Findings of Part A"; "What Part A does not show"; "Part B: person steps". Each step of Part A has a heading, its block, an "Expect:" line and a "Result:" line that is filled in when the block has run.

- [ ] **Step 2: A1, nothing of ours is running; build**

```bash
tasklist | grep -i -E 'fake-claude' && echo "STOP: a fake CLI is running" || echo "no fake CLI running"
powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | ForEach-Object { \$_.ExecutablePath }"
bun install --frozen-lockfile 2>&1 | tail -1
CARGO_TARGET_DIR=C:/Users/aksha/dots/target bun run tauri build --debug --no-bundle 2>&1 | tail -2
node --test tools/smoke/webview.test.mjs 2>&1 | grep -E '^ℹ (tests|pass|fail)'
```

Expect: no fake CLI; the only `dots-app.exe` that may be listed is the installed one under `AppData\Local\Dots`, which is left alone. If one under `target` is listed, stop: it blocks the build and is not ours to stop. The debug build finishes ("Built application at: C:\Users\aksha\dots\target\debug\dots-app.exe") and the helper's 15 tests pass.

- [ ] **Step 3: A2, temporary folders, a clean PATH, the state file**

`SHOTS` is a folder outside the repository, for the screenshots and the state file.

```bash
: "${SHOTS:?set SHOTS to a folder outside the repository}"
mkdir -p "$SHOTS"; E="$SHOTS/env.sh"
SMOKE="$(mktemp -d)"
mkdir "$SMOKE/data" "$SMOKE/work"
printf 'hello\n' > "$SMOKE/work/README.md"
# A PATH without any folder that holds the real claude CLI.
CLEAN_PATH=""; IFS=':' read -ra PARTS <<< "$PATH"
for p in "${PARTS[@]}"; do
  if [ -e "$p/claude" ] || [ -e "$p/claude.exe" ] || [ -e "$p/claude.cmd" ]; then echo "left out of PATH: $p"
  else CLEAN_PATH="$CLEAN_PATH${CLEAN_PATH:+:}$p"; fi
done
{
  echo 'set -u'
  printf 'SHOTS=%q\nSMOKE=%q\nE=%q\nCLEAN_PATH=%q\n' "$SHOTS" "$SMOKE" "$E" "$CLEAN_PATH"
  printf 'APP=%q\nWORK=%q\n' "C:/Users/aksha/dots/target/debug/dots-app.exe" "$(cygpath -m "$SMOKE/work")"
} > "$E"
cat >> "$E" <<'EOF'
W="node tools/smoke/webview.mjs 9333"
LMS="$HOME/.lmstudio/bin/lms.exe"
val() { tr -d '"\n '; }
wait_page() { for _ in $(seq 1 60); do node tools/smoke/webview.mjs "$1" eval 'invoke("startup_error")' > /dev/null 2>&1 && break; sleep 1; done; sleep 1; }
# The process id of the app this smoke started: the one whose program is under target, not the installed one.
app_pid() { powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | Where-Object { \$_.ExecutablePath -like '*\\target\\debug\\*' } | ForEach-Object { \$_.ProcessId }" | tr -d '\r'; }
start_app() {
  ( PATH="$CLEAN_PATH" DOTS_DATA_DIR="$(cygpath -w "$SMOKE/data")" DOTS_CONFIRM=yes \
    WEBVIEW2_USER_DATA_FOLDER="$(cygpath -w "$SMOKE/webview")" \
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
    "$APP" > "$SMOKE/app.out" 2>&1 & )
  wait_page 9333
}
# Waits until a run has left `queued` and `running` and prints its status.
settled() { for _ in $(seq 1 150); do S=$($W eval "(await invoke('get_run', { id: '$1' })).status" | val); [ "$S" != "running" ] && [ "$S" != "queued" ] && break; sleep 2; done; echo "$S"; }
# A local time some minutes ahead: HH:MM.
local_hm() { date -d "+$1 min" +%H:%M; }
# A local dot that answers in one word and calls no tool. <name> <schedule as a JS object> <endpoint>
new_dot() { $W eval "(async () => { const spec = await invoke('new_dot_spec'); return (await invoke('create_dot', { spec: { ...spec, name: '$1', engine: 'local', model: 'google/gemma-4-12b-qat', endpoint_url: '$3', workdir: '$WORK', workspace_mode: 'folder', schedule: $2, instructions: 'Reply with the one word done. Do not call any tool.' } })).id; })()" | val; }
# The runs of a dot, oldest first: trigger, status, payload, retry_of, queued_at.
runs() { $W eval "(await invoke('list_runs', { dotId: '$1', limit: 50 })).reverse().map(r => [r.trigger, r.status, r.payload, r.retry_of, r.queued_at])"; }
# What the overview says of a dot's schedule.
view() { $W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$1').schedule_state"; }
# Counts the schedule_attention events that reach the page from now on (window.__attention).
listen_attention() { $W eval "(async () => { const T = window.__TAURI_INTERNALS__; window.__attention = 0; await T.invoke('plugin:event|listen', { event: 'runtime', target: { kind: 'Any' }, handler: T.transformCallback((e) => { if (e.payload && e.payload.type === 'schedule_attention') window.__attention += 1; }) }); return 'listening'; })()"; }
EOF
. "$E"; ls "$SMOKE"; PATH="$CLEAN_PATH" which claude 2> /dev/null || echo "no claude on the clean PATH"
```

Expect: the folders `data` and `work`; one or more "left out of PATH" lines when the real CLI is installed; `no claude on the clean PATH`.

- [ ] **Step 4: A3, the app starts on the temporary data folder; the model server**

```bash
. "${E:?}"
start_app
$W eval '(await invoke("status")).data_dir'
$W text | head -8
timeout 60 "$LMS" ps < /dev/null 2>&1 | grep -v '^$' | cut -c1-110
timeout 60 "$LMS" server status < /dev/null 2>&1 | tail -1
timeout 60 "$LMS" server start < /dev/null 2>&1 | tail -2
timeout 600 "$LMS" load google/gemma-4-12b-qat -c 8192 -y < /dev/null 2>&1 | tail -2
curl -s -m 3 http://127.0.0.1:9/ > /dev/null && echo "STOP: something answers on port 9" || echo "nothing answers on port 9"
```

Expect: the data folder is the temporary one. The banner "The local server is not running: could not listen on 127.0.0.1:47321 …" is expected: the installed Dots holds that port, and local dots do not need it. No banner names a version of the Claude CLI (the clean `PATH` worked). The first `lms ps` and `server status` show what ran before this smoke: write it into the result, and at the end undo only what this smoke started. Then the server runs on port 1234 with the model loaded, and nothing answers on port 9.

- [ ] **Step 5: A4, a dot every minute runs**

```bash
. "${E:?}"
EVERY=$(new_dot every-dot "{ rule: { kind: 'every', minutes: 1, anchor: null }, catch_up: true, until: null }" http://127.0.0.1:1234); echo "EVERY=$EVERY" >> "$E"
$W eval "(await invoke('get_dot', { id: '$EVERY' })).spec.schedule"; view "$EVERY"
for _ in $(seq 1 90); do N=$($W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 50 })).length" | val); [ "$N" -ge 2 ] && break; sleep 2; done
FIRST=$($W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 50 })).at(-1).id" | val); echo "first: $(settled "$FIRST")"
runs "$EVERY"; view "$EVERY"
$W goto dots; sleep 2; $W text | sed -n '/^Dots$/,$p' | head -20; $W shot "$SHOTS/01-every.png"
$W goto "runs/$FIRST"; sleep 2; $W text | grep -E 'Scheduled'; $W shot "$SHOTS/02-every-run.png"
```

Expect: the stored schedule has an `anchor` the core set (RFC 3339 UTC). The first view is `active`, `fired_count` 0, a `next_fire` less than a minute ahead in local time, no retry, no attention. Within about two minutes two runs exist, trigger `schedule`, payload `{"scheduled_for": …}` one minute apart and without `caught_up`; the first ends `succeeded`. The view counts them. The Dots list shows `every-dot` with its next run and no badge; the run view shows "Scheduled run" and "Scheduled for <date, time>".

- [ ] **Step 6: A5, Dots closed across a slot and started again runs one catch-up run**

```bash
. "${E:?}"
for _ in $(seq 1 60); do S=$($W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 1 }))[0].status" | val); [ "$S" != running ] && [ "$S" != queued ] && break; sleep 2; done
AT=$(local_hm 3); DAILY=$(new_dot daily-dot "{ rule: { kind: 'daily', time: '$AT' }, catch_up: true, until: null }" http://127.0.0.1:1234); echo "DAILY=$DAILY" >> "$E"
PID=$(app_pid); taskkill //PID "$PID" //F; echo "closed at $(date +%H:%M:%S); daily slot at $AT"
until [ "$(date +%H:%M)" \> "$AT" ]; do sleep 5; done; sleep 90
STARTED=$(date -u +%Y-%m-%dT%H:%M:%S); echo "STARTED=$STARTED" >> "$E"; start_app; sleep 10
runs "$DAILY"; view "$DAILY"
$W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 50 })).filter(r => r.queued_at > '$STARTED').reverse().map(r => r.payload)"
CATCH=$($W eval "(await invoke('list_runs', { dotId: '$DAILY', limit: 1 }))[0].id" | val); echo "catch-up: $(settled "$CATCH")"
$W goto "runs/$CATCH"; sleep 2; $W text | grep -E 'Scheduled'; $W shot "$SHOTS/03-caught-up.png"
$W eval "(await invoke('set_dot_enabled', { id: '$EVERY', enabled: false })).spec.enabled"
```

Expect: `daily-dot` has exactly one run, trigger `schedule`, payload `{"scheduled_for": <today at AT, local>, "caught_up": true}`, which ends `succeeded`; its view has `fired_count` 1 and `next_fire` tomorrow at `AT`. The run view says "Scheduled for <date, time>, caught up after the PC was off". `every-dot`'s runs queued since the start are one per slot: the first is for the newest slot before the start (with `caught_up: true` when that slot was 60 s or more late), and none is for an older slot of the minutes Dots was closed. If a run of `every-dot` was running at the moment of the close, it is failed as interrupted at the start: write it down; it is not part of this check. `every-dot` is then disabled, so that it stops running every minute.

- [ ] **Step 7: A6, the model server is down: the first two retries**

```bash
. "${E:?}"
listen_attention
AT=$(local_hm 2); RETRY=$(new_dot retry-dot "{ rule: { kind: 'daily', time: '$AT' }, catch_up: true, until: null }" http://127.0.0.1:9); echo "RETRY=$RETRY" >> "$E"; echo "slot at $AT"
for _ in $(seq 1 120); do A=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.retry_attempt" | val); [ "$A" = 2 ] && break; sleep 2; done
runs "$RETRY"; view "$RETRY"
$W goto dots; sleep 2; $W text | grep -E 'retry-dot|Retrying'; $W shot "$SHOTS/04-retrying.png"
for _ in $(seq 1 120); do A=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.retry_attempt" | val); [ "$A" = 3 ] && break; sleep 2; done
runs "$RETRY"; view "$RETRY"
SECOND=$($W eval "(await invoke('list_runs', { dotId: '$RETRY', limit: 50 })).at(-2).id" | val)
$W goto "runs/$SECOND"; sleep 2; $W text | grep -E 'Retry|Scheduled'; $W shot "$SHOTS/05-retry-run.png"
$W eval "(await invoke('inbox')).review_runs.length"; $W eval 'window.__attention'
```

Expect: `listening`. The slot's run fails within seconds, with an error about the connection to the model server and no tool call. The view then has `retry_attempt` 2 and a `retry_at` one minute after that run ended (local time); the Dots list says "Retrying at <h:mm> (attempt 2 of 6)". The second run has trigger `schedule`, `retry_of` the first run, payload `{"scheduled_for": <the slot>, "attempt": 2}`, and fails the same way; the view then has `retry_attempt` 3, two minutes after it ended. The run view of the second run says "Retry 2 of 6 of the failed run", with the link, and no "Scheduled for". No failed run of `retry-dot` is under "Runs to review" (the count of review runs is 0 unless another dot left one), and `window.__attention` is 0. If `listen_attention` answers an error instead of `listening`, write it down: the count of notifications is then left to B3 and to the test `a_failed_scheduled_run_is_quiet_and_its_attention_notifies`.

- [ ] **Step 8: A7, the attempts are used up: "Needs attention" and one notification**

Attempts 3 to 6 start 2, 4, 8 and 16 minutes after the attempt before them ended: the whole occurrence takes 1 + 2 + 4 + 8 + 16 = 31 minutes from the first failure, plus the few seconds of each run, so about 28 minutes remain after A6. The block waits at most 9 minutes, below the 10-minute limit of one command; run it again until it prints `attention set`.

```bash
. "${E:?}"
for _ in $(seq 1 54); do R=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.attention !== null" | val); [ "$R" = true ] && echo "attention set" && break; sleep 10; done
runs "$RETRY"; view "$RETRY"
$W eval 'window.__attention'
$W eval "(await invoke('inbox')).attention"; $W eval "await invoke('inbox_count')"
$W goto inbox; $W wait "retry-dot needs attention" 30; $W text | sed -n '/^Needs attention$/,$p' | head -12; $W shot "$SHOTS/06-attention.png"
$W goto dots; sleep 2; $W text | grep -E 'retry-dot|Needs attention'; $W shot "$SHOTS/07-badge.png"
```

Expect: six runs of trigger `schedule`, all `failed`, the first without `attempt` and the others with `attempt` 2 to 6, each with `retry_of` the run before it. The view: `status` `active`, `fired_count` 1, `retry_at` and `retry_attempt` null, `attention` with the sixth run's id and its error, `next_fire` tomorrow at the slot's time. `window.__attention` is 1: one notification for the occurrence, none for the attempts. The Inbox answer has one attention item for `retry-dot`, and `inbox_count` counts it. The Inbox shows "Needs attention" with "retry-dot needs attention", "Scheduled run failed … · <error>", "Open the failed run", "Run now" and "Dismiss"; the sidebar's Inbox count includes it. The Dots list shows the badge "Needs attention" beside `retry-dot`.

- [ ] **Step 9: A8, the server back, and "Run now" clears the flag**

```bash
. "${E:?}"
$W eval "(async () => { const d = await invoke('get_dot', { id: '$RETRY' }); return (await invoke('update_dot', { id: '$RETRY', spec: { ...d.spec, endpoint_url: 'http://127.0.0.1:1234' } })).spec.endpoint_url; })()"
$W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.attention !== null"
$W goto inbox; $W wait "retry-dot needs attention" 30; $W click "Run now"
sleep 3; NOW=$($W eval "(await invoke('list_runs', { dotId: '$RETRY', limit: 1 }))[0].id" | val); echo "run now: $(settled "$NOW") at $(date +%H:%M:%S)"
for i in $(seq 1 45); do C=$($W eval "(await invoke('inbox')).attention.length" | val); [ "$C" = 0 ] && echo "cleared after about $((i * 2)) s" && break; sleep 2; done
view "$RETRY"; $W eval 'window.__attention'
$W goto inbox; sleep 2; $W shot "$SHOTS/08-cleared.png"
```

Expect: the dot now points at port 1234, and its flag is still set (`true`): the edit did not change the schedule, so the row was not reset. "Run now" opens a new run, trigger `manual`, which ends `succeeded`. Within 60 s after it ended the Inbox has no attention item, the view's `attention` is null, and `window.__attention` is still 1: clearing the flag does not notify.

- [ ] **Step 10: A9, a `Once` dot runs once and shows "Completed"**

```bash
. "${E:?}"
AT=$(date -d '+2 min' +%Y-%m-%dT%H:%M); ONCE=$(new_dot once-dot "{ rule: { kind: 'once', at: '$AT' }, catch_up: true, until: null }" http://127.0.0.1:1234); echo "ONCE=$ONCE" >> "$E"; echo "once at $AT"
for _ in $(seq 1 120); do S=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$ONCE').schedule_state.status" | val); [ "$S" = completed ] && break; sleep 2; done
sleep 70; runs "$ONCE"; view "$ONCE"
$W goto dots; sleep 2; $W text | grep -E 'once-dot|Completed'; $W shot "$SHOTS/09-once.png"
```

Expect: one run, trigger `schedule`, payload `{"scheduled_for": <AT, local>}`, `succeeded`. The view: `status` `completed`, `fired_count` 1, `next_fire` null. Seventy seconds later there is still one run. The Dots list says "Completed" for `once-dot`.

- [ ] **Step 11: A10, stop and clean up**

```bash
. "${E:?}"
PID=$(app_pid); echo "stopping $PID"; [ -n "$PID" ] && taskkill //PID "$PID" //F
cp "$SMOKE"/data/logs/dots.*.log "$SHOTS/"
[ -d "$SMOKE/data" ] && rm -rf "$SMOKE"; ls -d "$SMOKE" 2>&1 | sed 's#/tmp/tmp\.[A-Za-z0-9]*#$SMOKE#'
```

Then unload the model and stop the server only if step A3 showed that this smoke started them: `timeout 60 "$LMS" unload --all < /dev/null`, `timeout 60 "$LMS" server stop < /dev/null`.

Expect: the one process this smoke started is stopped; the temporary folder is gone; the app's log is kept beside the screenshots. Look at every screenshot and say in the results what it shows.

- [ ] **Step 12: Write Part B and the closing sections**

Part B is a checklist for the person who owns the machine, each line with "Result:" to fill in:

- B1. Sleep and wake: a laptop with a release or debug build, a daily dot at a time a few minutes ahead with "Catch up a missed run" on, and a second one at the same time with it off. Put the PC to sleep before that time and wake it at least two minutes after. Within a minute of the wake the first dot has one run, whose run view says "caught up after the PC was off"; the second has none, and its next run is tomorrow.
- B2. A release build with the real data folder, installed over a Dots that has a cron dot: after the upgrade the editor shows the kind "Cron expression" with the same expression and "Catch up a missed run" off, and the dot's next run is where it was before.
- B3. The notification itself: when a schedule gets "Needs attention" (A7 again, or any real failure), Windows shows one notification "<dot>: scheduled run failed" with the error, and none for the failed attempts before it.
- B4. A DST day. It is not tested here: it would need the PC's clock set to a transition night, which changes the machine for everything else that runs on it, and the next real transition may be months away. The rules are checked by the slot tests of Task 1 in a test zone. If the person's PC goes through a DST change while Dots runs a daily or hourly dot, they can look at the run times the next day: a daily time in the skipped hour runs an hour later that day, one in the repeated hour runs once.

"Findings of Part A" lists every expectation that was not met, with what was seen. "What Part A does not show" names B1 to B4, a schedule that expires (it needs an old cron string a newer crate refuses), a dot deleted or disabled while a retry waits, a retry joined into a queued run, and anything a step could not reach; the tests of Tasks 4 and 5 cover those.

- [ ] **Step 13: Commit**

```bash
git add docs/verification/2026-10-14-scheduler-v2-smoke.md
git commit -m "docs(verification): the smoke of Scheduler v2, agent steps run and person steps listed"
```

## After this plan

- Push `feat/scheduler-v2` and open a pull request against `master`. Ask first how the branch is to be finished.
- Open, and not decided by this plan:
  - ~~The flag of a schedule that has ended is cleared only by "Dismiss".~~ Closed by the final review's fix wave: the pass looks at every flagged row, whatever its status and whether the dot is enabled.
  - A retry joined into a queued manual or webhook run starts counting again at attempt 2 when that run fails before a tool call (Task 5). The total stays bounded, but "attempt k of 6" can then be wrong. A column for the attempt of the watched run would make it exact.
  - ~~The flag is also cleared by a successful manual run, which the Scheduler v2 spec's section 6 does not say.~~ Closed: the spec's section 6 now says so (amended 2026-10-15).
  - ~~`preview_schedule` refuses a schedule with no slot left instead of answering `[]` (Task 7).~~ Closed: the spec's section 8 now says so.
  - ~~A cron schedule with minute or second fields loses the rest of the first pass of the repeated hour on a fall-back night (Task 1).~~ Closed: cron takes an ambiguous wall time once, at its earlier instant, and from the final review only an instant that reads back as the wall time (the end of the overlap and the start of the gap on Windows).
  - ~~A failed scheduled run that called no tool is never listed for review (Task 6).~~ Closed: the scheduler marks reviewed a scheduled run's failure that made no tool call when it judges it, and every other failure, also one it never judged, is listed. A retry that succeeds still leaves no trace in the Inbox; a person who wants to know how often a dot needed retries has only the run list.
  - The runtime writes the dot and then its row, not in one transaction (Task 3); the store's would-never-run check and the `Every` anchor take `Utc::now()`, not the runtime's clock (Task 2). Neither matters with the real clock; both matter for a test that moves the clock and saves a dot.
  - A manual run that a slot joined, or a resume child, still notifies its own failure (Task 5), so one occurrence can notify twice.
- Follow-ups the Scheduler v2 spec left out of scope (its section 1): a time zone per dot, more than one catch-up run, a configurable retry count or delay, a history view of a schedule's occurrences, and schedules for webhook or chat triggers.
