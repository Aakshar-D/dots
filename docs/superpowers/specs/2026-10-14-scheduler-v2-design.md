# Scheduler v2

Date: 2026-10-14. Status: approved in conversation, section by section; this document is the written form.

Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md` (§3 data model, §4 run lifecycle, §8 scheduler, §12 UI). Section 11 lists the passages of the main spec this changes.

## 1. Why

Dots runs on a desktop PC that sleeps, restarts and is closed. Its scheduler is cron only (`crates/dots-core/src/scheduler.rs`): it fires one run per tick in the PC's local time, drops a tick when the dot already has a queued run, skips every tick missed while Dots was not running or the PC slept, and never retries a scheduled run that failed. A daily 09:00 dot on a laptop that wakes at 09:05 does not run that day, and a dot whose model server was down at 09:00 fails silently until the next day.

The idea comes from the review of Cloudflare OS (Approvals v2 spec, section 14): its scheduler (`packages/gatekeeper-scheduler/README.md`) has one-shot runs, `count`/`until` bounds, retry with backoff, a "Needs attention" state and written DST rules. Dots takes those ideas and changes two of them for a desktop: a missed slot can be caught up once, and only a run that did nothing yet is retried.

**Goal.** Scheduled dots are reliable on a desktop: they can run once or a bounded number of times, a slot missed while the PC was off runs once when it is back, a run that failed before it did anything is retried, and a schedule whose run failed in the end says so instead of failing silently.

**Decisions taken in the conversation**

| Question | Decision |
|---|---|
| A slot missed while Dots was not running | Per schedule: catch up once (default) or skip. One run for the most recent missed slot, if it is less than 24 h old; never one per missed slot |
| Which failed runs are retried | Only a run that failed before it made any tool call. A retry never repeats side effects |
| Time zone | The PC's local time, with written DST rules. No per-dot zone |
| How a schedule is written | A builder (once, every N minutes, daily, weekly) with cron kept as the advanced kind, plus bounds |
| When retries are used up | The schedule keeps firing and is flagged "Needs attention" |
| Architecture | Schedule state in its own table (approach A), not future runs in the queue, not state derived from run history |

**Out of scope:** a per-dot time zone, more than one catch-up run, a configurable retry count or delay, a history view of a schedule's occurrences, schedules for webhook or chat triggers.

## 2. The schedule

`DotSpec.schedule` changes from `Option<String>` (a cron expression) to `Option<Schedule>`:

```rust
pub struct Schedule {
    pub rule: Rule,
    /// One run for the most recent missed slot when Dots comes back. Default true.
    pub catch_up: bool,
    /// None: no bound. Not allowed with `Rule::Once`.
    pub until: Option<Until>,
}

pub enum Rule {
    /// Once, at a local date and time.
    Once { at: NaiveDateTime },
    /// Every `minutes` (1 to 10080) of elapsed time, phase counted from `anchor`.
    Every { minutes: u32, anchor: DateTime<Utc> },
    /// Every day at a local time.
    Daily { time: NaiveTime },
    /// On the given weekdays (at least one) at a local time.
    Weekly { days: Vec<Weekday>, time: NaiveTime },
    /// A 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression, as today.
    Cron { expr: String },
}

pub enum Until {
    /// Stop after this many occurrences (at least 1).
    Count(u32),
    /// Stop after this local date and time, inclusive.
    Date(NaiveDateTime),
}
```

Serialised as JSON with a `kind` tag: `{"rule": {"kind": "daily", "time": "09:00"}, "catch_up": true, "until": {"count": 10}}`. Times are `HH:MM` (seconds are 0); dates and date-times are local, without an offset (`2026-10-20T09:00`); `anchor` is RFC 3339 UTC.

`anchor` is set by the core when an `Every` rule is saved without one, or when its `minutes` change; the UI never sends it. A restart therefore keeps the phase.

`DotSpec::validate()` refuses: `minutes` outside 1 to 10080; an empty `days`; a time or date that does not parse; a cron expression that does not parse (as today); `Until::Count(0)`; `until` with `Once`. A `Once` at a time in the past, or an `Until::Date` before the schedule's first slot, is refused when the dot is created or the schedule changes (`Invalid`, "this schedule would never run"). A dot loaded from the database is not refused for a past date: its schedule is completed (§4).

## 3. Slots and time

A **slot** is an instant at which the schedule is due. `slots_after(rule, after, tz)` yields the slots strictly after `after`, in order. The functions are generic over chrono's `TimeZone`; the scheduler passes `Local`.

- `Once`: the one instant of `at`, if it is after `after`.
- `Every`: `anchor + k * minutes` for whole `k`, in elapsed time. It does not follow the wall clock across DST: a 60-minute schedule fires at 01:00, 03:00 local on a spring-forward night (the 02:00 hour does not exist) and twice in the repeated hour on fall-back.
- `Daily` and `Weekly`: the wall-clock time on each matching day, with these DST rules:
  - A local time that does not exist (spring-forward gap) moves forward by the length of the gap: 02:30 on a day when clocks go from 02:00 to 03:00 becomes 03:30.
  - A local time that occurs twice (fall-back overlap) fires once, at the earlier instant.
- `Cron`: as the `cron` crate computes it in `Local`. Its behaviour across DST is whatever the crate does; the plan records what that is in a test and in this section's amendment, and does not change it.

All times are the PC's local time zone as the OS reports it. When the zone changes (travel, a settings change), the next pass computes slots in the new zone; `Every` is unaffected.

## 4. State

**Migration 0005**

- Rewrites every `dots.schedule` that holds a cron expression to `{"rule":{"kind":"cron","expr":<the expression>},"catch_up":false,"until":null}`. `catch_up` is `false` for converted schedules so that their behaviour does not change on upgrade; new schedules default to `true`.
- Adds `runs.retry_of TEXT NULL REFERENCES runs(id) ON DELETE SET NULL`: the failed run a retry repeats.
- Creates `schedule_state`:

| Column | Meaning |
|---|---|
| `dot_id` TEXT PK, FK `dots(id)` ON DELETE CASCADE | One row per dot with a schedule |
| `rule_key` TEXT | The schedule's JSON when the row was made; a different value resets the row |
| `status` TEXT | `active`, `completed` (bound reached, `Once` done, or no slot left), `expired` (the rule no longer parses) |
| `last_slot_at` TEXT | The newest slot that was handled (fired, coalesced or skipped), RFC 3339 |
| `fired_count` INTEGER | Slots that started or joined a run; see "Count" below |
| `watch_run_id` TEXT NULL | The run whose outcome decides the current occurrence |
| `retry_slot_at` TEXT NULL, `retry_attempt` INTEGER NULL, `retry_at` TEXT NULL | A pending retry: which slot, which attempt is next (2 to 6), when |
| `attention_run_id` TEXT NULL | Set when an occurrence failed in the end (§6) |
| `updated_at` TEXT | |

The existing, unused `dots.timezone` column stays as it is.

**Lifecycle of a row**

- Created (or reset) when a dot with a schedule is created, when its schedule changes (`rule_key` differs), and when a disabled dot is enabled: `status = active`, `last_slot_at = now`, `fired_count = 0`, no watch, no retry, no attention. A schedule therefore never catches up a slot from before it existed or before it was enabled.
- Deleted when the schedule is removed or the dot is deleted.
- A row for a disabled dot is left alone and not processed.

**Count.** `fired_count` counts slots that started a run or were coalesced into an already queued run (that run serves the slot). A missed slot that was skipped, and a retry, do not count. When `fired_count` reaches `Until::Count`, the status becomes `completed` after the occurrence's outcome is judged (a pending retry of the last occurrence still runs).

**Completed.** The status becomes `completed` when the count is reached, when a `Once` slot has been handled, or when the next slot lies after `Until::Date` (or no next slot exists). A completed schedule fires nothing; editing the schedule resets it.

## 5. The scheduler loop

The scheduler keeps its role and its file; its loop changes.

**A pass.** For every enabled dot whose row is `active`, in one transaction per dot:

1. Find `due`, the newest slot with `last_slot_at < due <= now`.
2. If there is one:
   - **On time** (`now - due < 60 s`): fire it.
   - **Missed** (`now - due >= 60 s`; Dots was closed, the PC slept, the clock jumped): if `catch_up` is on and `now - due < 24 h`, fire it as a catch-up; otherwise skip it. Older missed slots are skipped silently.
   - Set `last_slot_at = due`.
3. If a retry is due (`retry_at <= now`), start it (§6).
4. Judge the watched run if it has ended (§6).
5. Update `status` (§4).

Then sleep until the earliest of: the next slot of every active row, every `retry_at`, and `now + 60 s`. The 60-second cap is there because tokio timers do not follow the wall clock across system sleep or clock changes; every wake re-reads `Local::now()`. `reload()` still wakes the loop at once, and so does a `RunUpdated` event for a watched run.

**Fire.** If the dot already has a `queued` run, the slot is coalesced into it (as today, logged) and that run becomes the watched run. Otherwise a run is enqueued with trigger `schedule` and payload `{"scheduled_for": <slot, RFC 3339>}`, plus `"caught_up": true` for a catch-up. Either way `fired_count` goes up by one, and a pending retry of an older slot is dropped: the newer occurrence serves.

**Pause.** While schedules are paused (the setting `schedules_paused`), no pass runs and no retry starts. On resume, every active row gets `last_slot_at = now` (a deliberate pause never catches up); a retry that came due during the pause starts on the first pass. At a start with schedules paused nothing happens until resume.

**At start.** Recovery (`recover_interrupted`) runs first, as today. The first pass then applies the rules above, so a PC that was off at 09:00 and starts at 09:20 runs the 09:00 slot once if `catch_up` is on.

**Clock.** The scheduler takes a clock (`now()` and a sleep that can be woken), so tests drive time without waiting.

## 6. Outcome, retry and "Needs attention"

**Watching.** Every fire, coalesce and retry sets `watch_run_id`. The row, not an event, remembers which run decides the occurrence, so judging survives a restart and lost bus events.

**Judging the watched run when it has ended:**

| Outcome of the watched run | Result |
|---|---|
| `succeeded` | The occurrence is done. `attention_run_id` is cleared |
| `awaiting_approval` | Not ended: when it resumes, its resume child becomes the watched run |
| `cancelled` | Done: no retry, no attention (the person stopped it) |
| `failed`, and the run has no `tool_call` event | Retryable: schedule the next attempt, or set attention when the attempts are used up |
| `failed`, and the run has a `tool_call` event | Not retryable: set attention |

"No `tool_call` event" covers a model server that cannot be reached, a `claude` CLI that is missing or fails at start, a workspace that could not be prepared, a timeout before any call, and a run marked `interrupted` at start before its first call. A run that did call a tool may have changed files or run commands, and a retry could repeat that.

**Retry.** An occurrence has at most 6 attempts. After a retryable failure of attempt k (k < 6), `retry_attempt = k + 1` and `retry_at = ended_at + delay(k)`, with delays 1, 2, 4, 8 and 16 minutes. When it is due:

- If the dot already has a `queued` run, the retry is coalesced into it and that run is watched.
- Otherwise a run is enqueued with trigger `schedule`, `retry_of = <the failed run>` and payload `{"scheduled_for": <retry_slot_at>, "attempt": k + 1}`.
- A retry does not count toward `Until::Count`.
- A slot that fires while a retry is pending drops the retry.
- The retry fields are cleared when the retry starts or is dropped.

**Needs attention.** Set when an occurrence's attempts are used up or its failure is not retryable: `attention_run_id = <the last failed run>`. The schedule keeps firing. The flag clears when a later scheduled run of the dot succeeds, when the person dismisses it, or when the row is reset.

## 7. Events and notifications

- New `RuntimeEvent::ScheduleUpdated { dot_id }`, published after every change of a `schedule_state` row.
- The notifier (`apps/dots-app/src/events.rs`):
  - A failed run with trigger `schedule` no longer notifies by itself; its occurrence is either retried or ends in attention.
  - Setting `attention_run_id` notifies once: "<dot>: scheduled run failed — <error, first line>".
  - Clearing it does not notify.
  - A succeeded scheduled run notifies as today.
- The UI invalidates the dots, the overview and the inbox on `ScheduleUpdated`.

## 8. Commands and contract

- `DotSpec.schedule` in the contract becomes the `Schedule` object (zod: a discriminated union on `rule.kind`).
- `preview_schedule(schedule: Schedule, n: u32) -> Vec<String>`: the next `n` slots after now as RFC 3339 local times, `[]` when there is none. It validates the schedule like `validate()` (the UI parses no schedule).
- `DotOverview` gains `schedule_state: Option<ScheduleState>`:
  - `status`, `fired_count`, `next_fire` (RFC 3339 local or null; replaces the top-level `next_fire`);
  - `retry_at` and `retry_attempt` (null unless a retry waits);
  - `attention: Option<{ run_id, error }>`.
- New command `dismiss_schedule_attention(dot_id)`: clears the flag; `NotFound` without a row; no error when there is no flag.
- The `Inbox` answer of the `inbox` command gains `attention: Vec<{ dot_id, dot_name, run_id, error, failed_at }>`, newest first, and `inbox_count` counts these items too.
- `Run` stays the same; the UI reads `scheduled_for`, `caught_up` and `attempt` from the payload, and `retry_of` is a new field.
- The HTTP server and the MCP endpoint are unchanged.

## 9. UI

**Dot editor, "Schedule" section.**

- A kind picker: None, Once, Every, Daily, Weekly, Cron.
- The kind's fields:
  - Once: a date and time;
  - Every: a number of minutes, with "hours" as a unit helper;
  - Daily: a time;
  - Weekly: seven day toggles and a time;
  - Cron: the text field and its help, as today.
- "Catch up a missed run" (checkbox, on by default, hidden for Once).
- "Stop": Never, After N runs, After a date (hidden for Once).
- The next three slots, from `preview_schedule`, as today; the core's objection shows under the field.

**Dots list.**

- The next fire time; "completed" for a completed schedule; "paused" while schedules are paused (as today).
- A "Needs attention" badge when the flag is set.
- "Retrying at 09:04 (attempt 2 of 6)" while a retry waits.

**Inbox.** A group "Needs attention": per dot, the failed run (a link to the run view), its error, "Run now" and "Dismiss".

**Run view.** Under the trigger: "Scheduled for 09:00", "Scheduled for 09:00, caught up after the PC was off", or "Retry 2 of 6 of run <link>".

## 10. Errors and edge cases

- A store error during a pass is logged and the pass is retried after 30 seconds (as today).
- A schedule that no longer parses when loaded (only possible for an old cron string that a newer crate refuses) makes the row `expired`, with a log line; the Dots list shows "expired".
- A dot deleted while a retry waits: the row goes with it (`ON DELETE CASCADE`); the retry never starts.
- A dot disabled while a retry waits: nothing is processed while it is disabled; enabling it resets the row.
- The watched run is deleted (its thread discarded): the occurrence counts as done, without attention.
- `Once` that was missed beyond 24 h, or with `catch_up` off: the slot is skipped and the schedule is `completed`, without a run. The Dots list shows "completed (missed)".
- Two passes cannot run at once: the scheduler is one task.

## 11. Changes to the main spec

The plan edits these passages of `2026-10-08-dots-design.md` in the commits that change the behaviour:

- §1 and the scope table: "Schedule (cron)" becomes "Schedule (once, every, daily, weekly, cron)".
- §2 module list: `src/scheduler/` with its loop, slots and state.
- §3 data model: `dots.schedule` holds the `Schedule` JSON; `runs.retry_of`; the `schedule_state` table.
- §4 run lifecycle, step 1 "Trigger": coalescing stays; add catch-up, retry and the watched run.
- §12 UI: the editor's schedule section, the Dots list's states and badge, the Inbox's "Needs attention" group, the run view's trigger line.
- The commands list: `preview_schedule` takes the object; `dismiss_schedule_attention`; `DotOverview.schedule_state`.
- The tray's "Pause all schedules": a resume does not catch up.

## 12. Testing

- **Slots (unit).** Every kind, with a hand-written test `TimeZone` that has a one-hour spring-forward gap and a one-hour fall-back overlap (no new dependency): the moved 02:30, the single fire in the overlap, `Every` in elapsed time, weekly day sets, `Once` before and after `after`, `Until::Date` inclusive. One test records what the `cron` crate does across the gap and the overlap.
- **Validation (unit).** Each refusal in §2; a past `Once` refused on create and change but loaded as completed.
- **Migration.** A database with a cron string upgrades to the object with `catch_up: false`; `schedule_state` is empty until the first pass or save.
- **Scheduler (fake clock).**
  - Fire on time.
  - Catch-up within 24 h and beyond it, with `catch_up` on and off; only the newest missed slot.
  - Coalesce into a queued run.
  - `Once` done; count reached after the last outcome; date passed.
  - Resume after pause skips; a retry due during the pause starts on resume.
  - Reset on schedule change and on enable.
  - Every judging outcome in §6, including a resume child of an approval.
  - Retry delays and the sixth attempt ending in attention; a retry coalesced; a retry dropped by a new slot.
  - Attention cleared by success and by dismiss.
  - Restart in the middle of a retry wait and with a watched run that ended while Dots was closed.
  - A dot deleted while a retry waits.
- **App.**
  - The notifier: a failed scheduled run does not notify; attention notifies once.
  - The contract fixture.
- **UI.**
  - The editor's kinds and their fields, the bounds, the preview and the core's objection.
  - The Dots list's states, badge and retry line.
  - The Inbox's "Needs attention" group with Run now and Dismiss.
  - The run view's trigger line.
- **Smoke** (LM Studio, a temporary data folder, as in earlier plans):
  - A dot every 1 minute runs.
  - Dots closed across a slot and started again runs one catch-up run.
  - With the model server stopped, the attempts retry and then "Needs attention" appears with one notification.
  - Server started again and "Run now", or the next slot, clears the flag.
  - A `Once` dot runs once and shows "completed".
