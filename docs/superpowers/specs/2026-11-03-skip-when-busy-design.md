# Skip a schedule when the dot is busy

- Date: 2026-11-03
- Status: approved (owner reviewed this document, 2026-11-03)
- Part of: Tier 1 of the build list (reliability first), second of seven. The first, the stuck-run watchdog, is merged (#45).
- Builds on: `2026-10-14-scheduler-v2-design.md` (slots, catch-up, retries, bounds), `2026-11-02-stuck-run-watchdog-design.md`.

## 1. Goal

A repeating schedule does not pile runs up behind a dot that is still working. A slot that comes due while the dot is busy is skipped, with a reason the person can see, and the next slot runs as normal.

What happens today: `Scheduler::handle` fires a due slot through `Store::record_slot`, whose `start_or_join` joins the dot's queued run if there is one and otherwise inserts a new queued run. A dot that is running (or paused on an approval) therefore gets another run queued behind it, which starts the moment the current one ends: an hourly dot whose run takes 70 minutes ends up running back to back forever.

Success: a busy dot's due slot leaves no new run, the dot's conversation and its schedule panel say it was skipped and why, and nothing else about schedules changes.

## 2. Words

- **Busy** (a dot): it has a run whose status is `running` or `awaiting_approval`, any trigger (a chat reply in progress counts). A run that is only `queued` does not make the dot busy.
- **Skip** (a slot): the slot is handled (its `last_slot_at` moves on, as for any handled slot) but starts no run and joins none; a skip record is kept.

## 3. Behaviour (decided with the owner)

1. **A repeating schedule's due slot, dot busy: skip it.** No run is created or joined. The next slot is computed as usual.
2. **Dot only has a queued run:** unchanged; the slot joins it (coalesced), as today.
3. **One-time schedules (`Rule::Once`) never skip.** They behave as today: the slot fires and its run queues behind the current one, starting when the dot is free.
4. **Retries never skip.** A due retry of an earlier slot's failed run starts as today (it queues behind the current run). *My call:* a retry belongs to a slot that already counted; skipping it would drop that slot without a word.
5. **Bounds:** a skipped slot does not count toward `Until::Count(n)`. This is already how `fired_count` works (it counts slots that started or joined a run); it must stay that way. `Until::Date` still ends the schedule on its date.
6. **Missed slots and catch-up:** a slot that is skipped as missed (the existing late rules) stays exactly as today. Busy is checked only for a slot that would otherwise fire.
7. **Telling the person:** no notification and no Needs you item. Each skip is recorded and shown in the dot's conversation and in its schedule panel (§6).
8. **Watchers:** unchanged; their events still queue.
9. **Run now:** never blocked.

Owner's calls: skip with a reason; busy = running or waiting on the person, chat included; queued-only still joins; history only; one-time schedules wait; skips do not use up a run limit; schedules only, not watchers.

## 4. Core

### 4.1 Busy check

`Store::dot_is_busy(&self, dot_id: &str) -> Result<bool>`: `EXISTS (SELECT 1 FROM runs WHERE dot_id = ? AND status IN ('running', 'awaiting_approval'))`.

### 4.2 Recording a skip

- **Migration 0027** (`0027_schedule_skips.sql`):

```sql
CREATE TABLE schedule_skips (
    id         TEXT PRIMARY KEY,
    dot_id     TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    slot_at    TEXT NOT NULL,          -- the slot's time, RFC 3339 UTC
    reason     TEXT NOT NULL,          -- 'busy' (the only reason for now)
    run_id     TEXT,                   -- the run that made the dot busy, if known
    created_at TEXT NOT NULL
);
CREATE INDEX schedule_skips_dot ON schedule_skips(dot_id, slot_at);
```

  `run_id` is a plain column, not a foreign key, so deleting old runs never fails on it.
- **`Store::record_slot`** gains the busy case in the same transaction as the slot write, so the busy check and the slot are one step: when `fire` is `Some(new)`, the schedule is not `Once`, and the dot is busy, it writes `last_slot_at` like a skipped slot, inserts a `schedule_skips` row with the busy run's id, keeps only the newest 50 skip rows for that dot (deletes older ones), and returns a new **`SlotStart::Busy { skip: ScheduleSkip }`**. *Builder's call:* whether the "not `Once`" decision is passed in by the scheduler (it knows the rule) or read in the store; the busy query must run inside the transaction.
- **`ScheduleSkip`** (model): `{ id, dot_id, slot_at, reason, run_id: Option<String>, created_at }`.
- A pending retry is left as it is on a busy skip (only a fired slot drops it today).
- The slot does not count: `fired_count` is not incremented.

### 4.3 Scheduler

`Scheduler::handle` treats `SlotStart::Busy` like `SlotStart::Skipped` for timing (`last = due`, no run to watch), logs it at info level, and emits **`RuntimeEvent::ScheduleSkipped { dot_id, skip }`** (serde `schedule_skipped`). It also emits the existing `ScheduleUpdated { dot_id }` so the schedule panel refreshes.

### 4.4 Reading skips

- **`Runtime::schedule_skips(dot_id, limit) -> Result<Vec<ScheduleSkip>>`**, newest first (at most 50).
- **`ScheduleView`** (what the schedule panel reads, `Runtime::schedule_view`) gains `last_skipped_at: Option<String>` and `skipped_since_run: u32` (skips with `slot_at` after the dot's newest scheduled run's `queued_at`, or all kept skips if it has none).

## 5. App

- Command `schedule_skips(dotId, limit?)` wrapping §4.4.
- Contract fixture: `ScheduleSkip`, the `schedule_skipped` event, the two `ScheduleView` fields.
- No notification for `schedule_skipped` (the notifier ignores it). The tray refreshes on it like on any event; counts do not change.

## 6. UI

- **Conversation:** the dot's conversation shows each kept skip as a quiet, muted line at its time among the other entries: **"Schedule skipped at `<local time>`: still working on the previous run"** (with the date when it is not today). *Builder's call:* where in the timeline these lines are merged (by time with topics and notes) and how they look, following how notes are shown; they must not count as unread (they are not messages).
- **Schedule panel** (`ScheduleEditor` / wherever `ScheduleView` is shown): when `last_skipped_at` is set, a line **"Last skipped `<local time>` · `<n>` skipped since the last run"** (`n` = `skipped_since_run`; the "· n skipped" part is left out when it is 0).
- Live: `schedule_skipped` refreshes the dot's skips and its schedule view.

## 7. Edge cases

- **Two slots due in one pass** (the existing catch-up code handles at most one due slot per pass): each due slot is judged on its own; unchanged.
- **The busy run ends between the check and the insert:** impossible to race inside the transaction for the slot write; a run that ends a moment later simply means the next slot fires.
- **A dot deleted:** its skips go with it (`ON DELETE CASCADE`).
- **Schedules paused** (`schedules_paused`): no slots are handled, so no skips are recorded; unchanged.
- **A dot disabled:** as today (its schedule does not fire); no skips.
- **Clock and time zone:** `slot_at` is UTC; the UI shows local time.

## 8. Testing

- Store: `dot_is_busy` for running, awaiting approval, queued only (false), ended (false), another dot's run (false); `record_slot` returns `Busy` and writes the skip and `last_slot_at` without a run or a `fired_count` change; joins a queued run when the dot is not busy; keeps only 50 skips; skips go with the dot.
- Scheduler: a repeating slot while busy is skipped and announced, and the next slot fires once the dot is free; a chat run in progress makes it busy; a run awaiting approval makes it busy; a `Once` schedule still fires and queues while busy; a due retry still starts while busy; a `Count(n)` schedule still runs `n` times with skips in between; a pending retry survives a busy skip; a missed slot is still handled by the late rules, not recorded as busy.
- Runtime: `schedule_skips` newest first and limited; `ScheduleView.last_skipped_at` and `skipped_since_run`.
- App: the command; the contract fixture; no notice for `schedule_skipped`.
- UI: the conversation line (wording, local time, date when not today, not counted as unread); the panel line with and without a count; refresh on the event.

## 9. Collisions with parallel work

Small additions to shared files: core `scheduler.rs`, `store/schedules.rs`, `store/runs.rs` (busy query) or a new `store/skips.rs`, `model.rs`, core `events.rs`, `runtime.rs`, migration 0027 (tell the parallel developer it is taken); app `commands.rs`, `lib.rs`, `contract.rs`; UI `api/types.ts`, `commands.ts`, `lib/runtime-events.ts`, the home timeline, `ScheduleEditor.tsx`. New UI code in a new file (`ui/src/screens/home/SkipLine.tsx`).

## 10. Not in this part

- Skipping watcher events while busy (watchers keep queuing; watch hours is the next design).
- A per-dot choice between skip and queue.
- Notifying after a streak of skips.
