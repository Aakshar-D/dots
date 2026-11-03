# Watch hours

- Date: 2026-11-03
- Status: approved (owner reviewed this document, 2026-11-03)
- Part of: Tier 1 of the build list (reliability first), third of seven. The stuck-run watchdog is merged (#45); skip-when-busy is in review (#46).
- Builds on: `2026-10-23-watchers-design.md` (watchers, `Watchers::fire`, the flood cap, polling), `2026-11-03-skip-when-busy-design.md` (migration 0027).

## 1. Goal

A watcher can be told when it may start its dot. Outside those hours its events are held, not lost, and the dot runs once with all of them when the hours open.

What happens today: every event goes through `Watchers::fire` (`crates/dots-core/src/watchers/mod.rs`) and becomes one queued `watcher` run at once, at any hour; email and GitHub watchers poll every minute (`POLL_EVERY`); the only limit is the flood cap (20 runs per watcher per hour).

Success: an email watcher set to Mon–Fri 09:00–18:00 starts nothing at night or at the weekend; at 09:00 Monday the dot runs once with everything that came in; the watcher's row says what it is holding.

## 2. Words

- **Hours** (of a watcher): a set of weekdays and one time range `start`–`end` in this computer's local time. `start < end` is a same-day range; `start > end` runs overnight and belongs to the day it starts (Mon 22:00–06:00 covers Monday 22:00 to Tuesday 06:00). `start == end` is refused.
- **Inside / outside**: whether a moment falls in the watcher's hours. A watcher with no hours is always inside.
- **Opening**: the next moment the hours start (the first `start` on an allowed day after now).
- **Held event**: an event that arrived while outside, kept until the opening.

## 3. Behaviour (decided with the owner)

1. **Hours are per watcher**: optional; none means "any time", as today. Existing watchers have none. *(My call.)*
2. **Shape**: weekdays plus one time range; overnight ranges allowed. Time zone: this computer's.
3. **Outside the hours, events are held**, not dropped and not run.
4. **At the opening, the dot runs once** with everything held: one `watcher` run whose payload carries the held events, framed for the dot as "while you were away". It carries at most **100** events in full; the rest are counted ("and 37 more").
5. **Email and GitHub watchers stop checking outside their hours.** Their first check at the opening picks up everything new since their last check (their cursor), and those events join the held ones in the single opening run. A folder watcher keeps watching and holds what it sees.
6. **Visible, quietly**: the watcher's row shows **"Outside hours · 4 held until 09:00"** (or "Outside hours · until 09:00" with nothing held). No notification, no Needs you item.
7. **Unchanged rules still apply** *(my calls)*: the opening run is **one** fire against the flood cap; it goes through every check `fire` makes today (Dots paused, watcher off or paused, dot disabled); if the dot is busy the run queues, as watcher runs do today (skip-when-busy does not apply to watchers).
8. **Edge cases** *(my calls)*:
   - Held events are stored; they survive a restart.
   - Dots closed at the opening: the opening run starts as soon as Dots next starts (if the watcher is inside its hours then, or its hours have passed the opening).
   - Removing a watcher's hours releases what it holds at once (the opening run fires now).
   - Turning a watcher off, or deleting it, drops what it holds.
   - Changing a watcher's hours keeps what it holds; the new opening applies.
   - **One rule for releasing**: whenever a watcher that holds events is inside its hours, they are released at once (one opening run). This is checked at the opening, at start, when the watcher is resumed or turned back on, and when its hours change. Held events are never left waiting while new events fire one by one.
   - **The opening run meets the flood cap** (20 runs already this hour): its events stay held and the watcher pauses as flooded, as any capped event does; they are released when it is resumed (by the rule above).
   - **"Check now"** (or any manual check) outside the hours checks and holds what it finds.
   - **Attachments of held events**: only the first 100 held events keep their attachments (those are the ones the opening run carries in full); an event held beyond 100 is stored without its files, which are not staged at all. So a flood at night cannot fill the disk.

Owner's calls: hold and run once at the opening; per watcher; days + one range, overnight allowed; this computer's time zone; email/GitHub stop checking outside hours; up to 100 events then a count; visible quietly.

## 4. Core

### 4.1 Hours

- **`WatchHours { days: Vec<Weekday>, start: String /* "HH:MM" */, end: String }`** (serde; weekdays as `"mon".."sun"`), validated: at least one day, no duplicates, `HH:MM` 00:00–23:59, `start != end`. Error text follows the watchers spec's sentence style, e.g. `"Watch hours need at least one day."`, `"Watch hours need a start and an end that differ."`.
- **`Watcher.hours: Option<WatchHours>`**; `Watchers::add` and `update` take it.
- **Pure functions** (`crates/dots-core/src/watchers/hours.rs`), taking a time zone like the schedule code does (`&Local` in the app, a fixed zone in tests):
  - `is_inside(hours: &WatchHours, at: DateTime<Tz>) -> bool`
  - `next_opening(hours: &WatchHours, after: DateTime<Tz>) -> DateTime<Tz>` (the first allowed `start` strictly after `after`)
  - `next_closing(hours: &WatchHours, after: DateTime<Tz>) -> DateTime<Tz>`
  - Daylight-saving gaps and overlaps follow how `schedule.rs` resolves local times *(builder: reuse its helper)*.

### 4.2 Storage

- **Migration 0028** (`0028_watch_hours.sql`; 0027 is taken by skip-when-busy, which merges first):

```sql
ALTER TABLE watchers ADD COLUMN hours TEXT;   -- JSON WatchHours, null = any time
CREATE TABLE watcher_held (
    id         TEXT PRIMARY KEY,
    watcher_id TEXT NOT NULL REFERENCES watchers(id) ON DELETE CASCADE,
    payload    TEXT NOT NULL,           -- JSON: the event's payload as fire() would have used it
    files      TEXT,                    -- JSON: files staged for it (email attachments), if any
    held_at    TEXT NOT NULL            -- RFC 3339 UTC
);
CREATE INDEX watcher_held_watcher ON watcher_held(watcher_id, held_at);
```

- Store functions: hold an event, count and list a watcher's held events (oldest first, `held_at, rowid`), take all of them (delete and return, in one transaction), drop all of them.
- **Attachments**: an email event's files are staged today per run (`files.stage(watcher_id, run_id, files)`). A held event's files are staged once under a held folder keyed by the held row's id and moved to the opening run's staging when it fires; dropping held events removes their folders. *(Builder's call on the exact folder layout; nothing may be lost or leaked.)*

### 4.3 `Watchers::fire`

After today's off/paused/disabled checks and before the flood cap: if the watcher has hours and `now` is outside, **hold** the event (store it, stage its files under the held folder only if fewer than 100 are already held), emit `WatcherUpdated`, and return a new **`Fired::Held`**. Inside its hours, unchanged.

**Pollers and `Held`**: to the email and GitHub pollers (`email.rs`, `github.rs`), `Fired::Held` means the event was taken: the cursor advances past it (like `Run`), so it is not read again. *(My call.)*

**The opening poll holds**: the check an email or GitHub watcher makes at its opening calls a hold path (e.g. `fire` with a "hold" flag, or a separate `hold` entry point), not the plain `fire`, so what it finds joins the held events and goes into the single opening run instead of firing one run each. *(My call; builder picks the shape.)*

### 4.4 The opening run

- **`Watchers::open(watcher_id)`**: takes all held events; if none, does nothing; otherwise builds one payload `{"watcher_id", "watcher_name", "held": [<first 100 payloads, oldest first>], "held_more": <count beyond 100>, "held_since": <oldest held_at>}`, moves the first 100 events' staged files into the run's staging (files of the events beyond 100 are dropped with them), and fires it through the same path as a normal event (flood cap, checks, enqueue). If that fire returns `Off` (watcher off, Dots paused...) or `Capped` (the watcher then pauses as flooded), the held events stay held *(builder: take them only after the checks and the cap pass, or put them back in the same transaction)*.
- **The dot's prompt** for such a run says it was held while the watcher was outside its hours and lists the events *(builder: in `watchers/describe.rs`, where a watcher run's payload becomes prompt text)*: first line **"While you were away (outside this watcher's hours), these came in:"**, then each event as today's single-event description, then **"…and N more."** when `held_more > 0`.
- **When it runs**: the `Watchers` service keeps, per watcher with hours, a timer for its next opening (and, for email/GitHub, uses `next_opening` as the next poll time while outside). At the opening: an email/GitHub watcher polls first (its new events are held, since the poll lands exactly at the opening *(builder: treat events found by the opening poll as held)*), then `open` runs. A folder watcher just runs `open`.
- **Release rule** (§3.8): `open` also runs, when the watcher holds events and is inside its hours, at start, on resume, when it is turned back on, and on `update` (hours changed so that now is inside, or hours removed). **Watcher turned off or deleted**: drop its held events.
- A change of hours must also restart or notify the watcher's poll task (today tasks are keyed by config only). *(Builder's call how.)*

### 4.5 Polling outside hours

Email and GitHub watchers do not poll while outside their hours; their next poll is the opening. The cursor is untouched, so the opening poll reads everything since the last check. A transient-failure backoff that ends outside the hours waits for the opening.

### 4.6 What the UI reads

`Watcher` gains `hours: Option<WatchHours>`, `held_count: u32`, and `next_opening: Option<String>` (RFC 3339 local, set only when the watcher has hours and is outside them now).

## 5. App

- The watcher add/update commands carry `hours`.
- Contract fixture: `WatchHours`, the new `Watcher` fields.
- No notification for held events or the opening (the opening run notifies like any watcher run that ends, as today).

## 6. UI

- **Watcher editor** (`ui/src/screens/dot-editor/WatchersTab.tsx`): a **"Watch hours"** section: *Any time* (default) or *Only at these times* with seven day toggles (Mon–Sun) and two time fields (start, end), with the hint **"Ends the next day"** when end is before start. Errors from the core are shown next to the section.
- **Watcher row**: when `next_opening` is set, **"Outside hours · `<n>` held until `<HH:MM>`"** (with the weekday when the opening is not today, e.g. "until Mon 09:00"); "· `<n>` held" is left out when nothing is held.
- Live: `watcher_updated` refreshes the row (it already does).

## 7. Edge cases

- **A watcher paused by the flood cap or an error**: holds nothing new (fire returns `Off` first, as today); what it already holds stays until it is resumed and its next opening.
- **Dots paused** (tray "Pause schedules and watchers"): as today, nothing fires and nothing is held.
- **Many events at night**: all stored as small JSON rows; the opening run carries 100, counts the rest. Only the first 100 keep attachments on disk (§3.8), so storage stays bounded in practice. *(My call.)*
- **Clock changes and DST**: hours are local wall time; the opening is recomputed after each run and at start.
- **Hours that never open** (impossible after validation: at least one day, start ≠ end).

## 8. Testing

- Hours: `is_inside`, `next_opening`, `next_closing` for same-day and overnight ranges, weekday boundaries (Fri 22:00–06:00 covers Saturday 03:00, not Sunday 03:00), validation errors, DST days.
- Store: hold, count, list oldest first, take (atomic), drop; cascade on watcher delete; the `hours` column round-trips and an old row reads as none.
- Watchers: an event outside hours is held (no run), inside fires as today; the opening run carries the held events, oldest first, 100 in full and the count of the rest, and counts once against the flood cap; held events survive a restart and open at start; removing hours, or changing them so now is inside, opens at once; resuming or turning back on inside the hours opens at once; turning off or deleting drops; an `Off` or `Capped` opening keeps the events (and `Capped` pauses as flooded); a held event advances the email/GitHub cursor; the opening poll's events join the single opening run; a manual check outside hours holds; events held beyond 100 stage no files; email/GitHub do not poll outside hours and poll at the opening; held email attachments reach the opening run and dropped ones are removed.
- Describe: the opening run's prompt text (first line, events, "…and N more.").
- App: the commands carry `hours`; the contract fixture.
- UI: the editor section (any time / days and times, overnight hint, errors); the row line with and without held events, today vs another day.

## 9. Collisions with parallel work

Migration 0028 (0027 is skip-when-busy's; merge #46 first). Shared files: `watchers/mod.rs`, `watchers/describe.rs`, `watchers/files.rs`, `store/watchers.rs`, `model.rs`, core `events.rs` (unchanged event), app `commands.rs` / `contract.rs`, UI `WatchersTab.tsx`, `api/types.ts`. New: `watchers/hours.rs`, `store/held.rs`.

## 10. Not in this part

- Hours for schedules, or an app-wide default.
- Several ranges per day, or different hours per day.
- Per-watcher time zones.
- A setting to drop instead of hold.
