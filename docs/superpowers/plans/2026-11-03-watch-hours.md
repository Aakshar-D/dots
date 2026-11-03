# Watch hours — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A watcher can have hours (weekdays + one time range); events outside them are held and the dot runs once with all of them at the opening.

**Architecture:** Pure hour maths in `watchers/hours.rs`; held events in a `watcher_held` table with their attachments staged under a held folder; `Watchers::fire` holds instead of firing when outside (or while an opening poll runs); `Watchers::open` turns everything held into one run; the supervisor waits for openings and stops email/GitHub polling outside hours.

**Tech Stack:** Rust (tokio, sqlx/SQLite, chrono), Tauri 2, React + TypeScript (zod, vitest).

**Spec:** `docs/superpowers/specs/2026-11-03-watch-hours-design.md`

## Global Constraints

- Hours: weekdays (`"mon".."sun"`) + one range `"HH:MM"`–`"HH:MM"`, local time of this computer; `start > end` runs overnight and belongs to the day it starts; `start == end` refused; at least one day; no duplicate days.
- Error copy: `Watch hours need at least one day.`, `Watch hours need a start and an end that differ.` (other malformed input: a sentence in the same style).
- Outside hours an event is held (`Fired::Held`), never dropped, never run. At the opening: one `watcher` run with at most 100 events in full (oldest first) and a count of the rest.
- Opening payload: `{"watcher_id", "watcher_name", "held": [..≤100 payloads], "held_more": n, "held_since": "<oldest held_at>", "attachments": [..]}`.
- Prompt first line: `While you were away (outside this watcher's hours), these came in:`; last line when `held_more > 0`: `…and N more.`
- Release rule: a watcher holding events while inside its hours releases them at once — at the opening, at start, on resume, on turn-on, on update. Turn off / delete drops them.
- `Off` or `Capped` opening keeps the events held (`Capped` pauses as flooded, as today).
- `Fired::Held` counts as consumed for pollers (cursor advances). The opening poll holds what it finds. A manual check outside hours holds. Email/GitHub do not poll outside hours.
- Only the first 100 held events of a watcher keep attachments; beyond that, files are not staged.
- Row copy: `Outside hours · <n> held until <HH:MM>` (weekday before the time when not today, e.g. `until Mon 09:00`); drop `· <n> held` when 0.
- Editor copy: section `Watch hours`, choices `Any time` / `Only at these times`, hint `Ends the next day` when end < start.
- Migration `0028_watch_hours.sql` (0027 exists on this branch).
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH. No AI trailer in commits.
- Tests must use the fake clock and `TestTz` (no wall-clock waits, no same-millisecond row-order dependence, no dispatcher).

## Review Focus

1. **Fri 22:00–06:00 hours and an event on Saturday 03:00**: inside (Friday's range); Sunday 03:00 is outside. Test in Task 1.
2. **Dots restarted overnight with held events, started after the opening**: released once at start, not twice (start + timer). Test in Task 4.
3. **An email with attachments held overnight**: its files reach the opening run's workspace; held folders survive the startup sweep. Test in Task 3 (sweep) and Task 4 (delivery).
4. **A GitHub watcher outside hours with several event kinds**: no poll happens; at the opening, all kinds found go into the one run, and the cursor ends past them. Test in Task 4.
5. **Hours edited from 09–18 to 00–23:59 while holding**: released at once by `update`. Test in Task 4.

---

### Task 1: Hours — type, validation, maths

**Files:**
- Create: `crates/dots-core/src/watchers/hours.rs` (register in `watchers/mod.rs`)
- Modify: `crates/dots-core/src/schedule.rs` (make `resolve` `pub(crate)`, and the `weekdays` / `local_time` serde modules `pub(crate)`)
- Modify: `crates/dots-core/src/model.rs` (`WatchHours`)
- Test: `crates/dots-core/tests/watchers_hours_test.rs` (uses `tests/common/tz.rs` `TestTz`, `at`)

**Interfaces:**
- Produces:
  - `pub struct WatchHours { pub days: Vec<Weekday>, pub start: NaiveTime, pub end: NaiveTime }` (Serialize/Deserialize via `weekdays` and `local_time`; Clone, Debug, PartialEq, Eq).
  - `pub fn check_hours(h: &WatchHours) -> Result<WatchHours>` (`Error::Invalid` with the Global Constraints copy; returns days sorted).
  - `pub fn is_inside<Tz: TimeZone>(h: &WatchHours, at: &DateTime<Tz>) -> bool`
  - `pub fn next_opening<Tz: TimeZone + 'static>(h: &WatchHours, after: &DateTime<Tz>) -> DateTime<Tz>` — first allowed `start` strictly after `after`, resolved with `schedule::resolve`.

- [ ] **Step 1: Write the failing tests**: same-day range (Mon–Fri 09:00–18:00: Mon 08:59 out, 09:00 in, 17:59 in, 18:00 out, Sat 10:00 out); overnight (Fri 22:00–06:00: Fri 23:00 in, Sat 03:00 in, Sun 03:00 out, Fri 21:59 out) (Review Focus 1); `next_opening` from Fri 18:30 for Mon–Fri 09–18 is Mon 09:00; from Mon 08:00 is Mon 09:00; from Mon 09:00 is Tue 09:00; DST gap day (2026-03-29, `TestTz`) with start 02:30 resolves forward; validation: no days, `start == end`, duplicate day → errors with the exact copy; serde round-trip `{"days":["mon","fri"],"start":"09:00","end":"18:00"}`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test watchers_hours_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the test file and the schedule test files. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): watch hours, and when they open`.

### Task 2: Storage — hours column, held table

**Files:**
- Create: `crates/dots-core/migrations/0028_watch_hours.sql` (spec §4.2 verbatim)
- Create: `crates/dots-core/src/store/held.rs` (register in `store/mod.rs`)
- Modify: `crates/dots-core/src/store/watchers.rs` (`COLUMNS` :15, `watcher_from_row` :18, `add_watcher` :66, `update_watcher` :93), `crates/dots-core/src/model.rs` (`Watcher` :1326)
- Modify: every `Watcher { .. }` literal (`apps/dots-app/src/contract.rs` `fn watchers()` :412, tests)
- Test: `crates/dots-core/tests/watchers_store_test.rs`

**Interfaces:**
- Consumes: `WatchHours` (Task 1).
- Produces:
  - `Watcher.hours: Option<WatchHours>`, `Watcher.held_count: u32` (from the store, a subquery count), `Watcher.next_opening: Option<String>` (store leaves `None`; Task 4 fills it).
  - `Store::add_watcher(dot_id, name, config, hours: Option<&WatchHours>)`, `Store::update_watcher(id, name, config, hours: Option<&WatchHours>)` (cursor rule unchanged: kept only if config equal).
  - `pub struct HeldEvent { pub id: String, pub watcher_id: String, pub payload: Value, pub files: Option<Value>, pub held_at: String }`
  - `Store::hold_event(watcher_id, payload: &Value, files: Option<&Value>, at: DateTime<Utc>) -> Result<HeldEvent>`; `Store::held_count(watcher_id) -> Result<u32>`; `Store::held_events(watcher_id) -> Result<Vec<HeldEvent>>` (oldest first: `held_at, rowid`); `Store::take_held(watcher_id) -> Result<Vec<HeldEvent>>` (select + delete in one transaction, oldest first); `Store::drop_held(watcher_id) -> Result<Vec<HeldEvent>>` (same, returned so files can be removed).

- [ ] **Step 1: Write the failing tests**: hours round-trip on add and update; a pre-0028 row reads `hours == None` (migrate to 0027 first, pattern `store_browser_test.rs:76`); `held_count` on `get_watcher`/`list_watchers`; hold three events at t, t+1s, t+2s → `held_events` oldest first; `take_held` returns them and leaves 0; `drop_held` likewise; deleting the watcher cascades the held rows.
- [ ] **Step 2: Run** `cargo test -p dots-core --test watchers_store_test`. Expected: compile errors.
- [ ] **Step 3: Implement**; callers of `add_watcher`/`update_watcher` pass `None` for now (Task 3 threads hours through `Watchers::add/update`).
- [ ] **Step 4: Run** the watcher test files (`watchers_*_test.rs`, `runtime_watchers_test.rs`) and `cargo build -p dots-app --tests`. Expected: PASS / compiles.
- [ ] **Step 5: Commit** `feat(core): a watcher keeps its hours and the events it holds`.

### Task 3: Holding events

**Files:**
- Modify: `crates/dots-core/src/watchers/mod.rs` (`Fired` :276, `fire` :507, `add` :420, `update` :438, a new `opening: Mutex<HashSet<String>>` field), `crates/dots-core/src/watchers/files.rs` (held staging; `sweep` :227 keeps held folders), `crates/dots-core/src/watchers/email.rs` (:323), `crates/dots-core/src/watchers/github.rs` (`Poller::fire` :311)
- Modify: `crates/dots-core/src/runtime.rs` (`watcher_add`/`watcher_update` ~:2123 take `hours`)
- Test: `crates/dots-core/tests/watchers_test.rs`, `watchers_files_test.rs`, `watchers_email_test.rs`, `watchers_github_test.rs`

**Interfaces:**
- Consumes: Task 1 `is_inside`; Task 2 store functions.
- Produces:
  - `Fired::Held`.
  - `Watchers::add(dot_id, name, config, hours: Option<WatchHours>)`, `Watchers::update(id, name, config, hours: Option<WatchHours>)` (validate with `check_hours`).
  - `WatcherFiles::held_dir(watcher_id, held_id) -> PathBuf` = `<root>/<watcher_id>/held/<held_id>/`; `WatcherFiles::stage_held(watcher_id, held_id, files) -> Result<()>`; `WatcherFiles::remove_held(watcher_id, held_id)`.
  - `fire` rule (after the Off checks, before the cap): hold when the watcher has hours and (`!is_inside(hours, now in local time)` or the watcher id is in `opening`). Holding: `hold_event`; stage files with `stage_held` only if `held_count < 100` before this event, and record the staged files' names in `files`; emit `updated`; return `Held`.
  - The local time zone used by `fire` and the supervisor: `Local` in production; tests need `TestTz` — add it to `WatcherDeps` in the smallest shape that lets the harness set `TestTz` *(implementer's call)*.
  - Pollers: email `Fired::Held => {}` like `Run` (cursor advances); GitHub `Ok(Fired::Held) => Ok(())` like `Run`.

- [ ] **Step 1: Write the failing tests**:
  - `an_event_outside_hours_is_held_not_run`: hours Mon–Fri 09–18, fake clock Sat 10:00; `fire` returns `Held`; no run; `held_count == 1`; a `WatcherUpdated` was emitted.
  - `an_event_inside_hours_runs_as_today`.
  - `a_watcher_without_hours_runs_any_time`.
  - `the_101st_held_event_stages_no_files`: hold 100 events with one attachment each, then the 101st: its held row has `files == None` and no held folder.
  - `held_folders_survive_the_sweep` (Review Focus 3): stage a held event's files, run `files.sweep`, the held folder is still there; a held folder whose row is gone is removed.
  - email: an email arriving outside hours (via `poll_now` with the fake mail) is held and the cursor advances past it (a second `poll_now` holds nothing new).
  - GitHub: an issue event outside hours is held and the repo cursor advances.
- [ ] **Step 2: Run** `cargo test -p dots-core --test watchers_test --test watchers_files_test --test watchers_email_test --test watchers_github_test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same plus `runtime_watchers_test`, and the full `dots-core` suite once. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): a watcher holds what comes in outside its hours`.

### Task 4: The opening

**Files:**
- Modify: `crates/dots-core/src/watchers/mod.rs` (new `open`; supervisor `reconcile` :673 keyed by config **and** hours; `start_polls` :744; a timer task for folder watchers' openings; `resume` :469, `set_enabled` :462, `update` :438, `delete` :452; `list` :415 fills `next_opening`; `spawn` :360 runs the release rule at start)
- Modify: `crates/dots-core/src/watchers/files.rs` (move held files into the run's stage dir)
- Modify: `crates/dots-core/src/watchers/describe.rs` (`section` :95)
- Test: `crates/dots-core/tests/watchers_test.rs`, `watchers_email_test.rs`, `watchers_github_test.rs`, `watchers_prompt_test.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `Watchers::open(&self, watcher_id: &str) -> Result<Fired>`: does the same Off checks and the flood cap as `fire` **before** taking held events; with none held returns `Off`; otherwise `take_held`, builds the opening payload (Global Constraints), moves the first 100 events' held files into `stage_dir(watcher_id, run_id)` (renaming each file `<n>-<name>` to avoid clashes) and lists them in the top-level `attachments` so `copy_into` finds them; removes the held folders of events beyond 100; enqueues one `watcher` run; returns `Run`. On `Off`/`Capped` nothing is taken.
  - Release rule calls `open` when the watcher holds events and is inside its hours: in `spawn` once before the first reconcile; after `resume`; after `set_enabled(true)`; after `update` (also when hours become `None`). `set_enabled(false)` and `delete` call `drop_held` and remove held folders.
  - Supervisor: email/GitHub tasks sleep until `next_opening` while outside (no `poll_now`); at the opening they insert the id into `opening`, `poll_now`, remove it, then `open`. A transient-failure `Poll::Next(at)` outside hours is moved to the next opening. Folder watchers with hours get a small task that sleeps until each opening and calls `open`. Tasks restart when hours change.
  - `Watchers::list` / the watcher returned by add/update/resume/set_enabled carry `next_opening` (RFC 3339 local) when the watcher has hours and is outside now.
  - `describe::section` renders an opening payload: first line, each held event as today's single-event lines, `…and N more.`

- [ ] **Step 1: Write the failing tests**:
  - `the_opening_runs_once_with_everything_held`: hold 3 events Sat; advance the fake clock to Mon 09:00 and let the supervisor wake; one run; its payload `held` has 3 payloads oldest first, `held_more == 0`; `held_count == 0` after.
  - `the_opening_carries_100_and_counts_the_rest`: 130 held → `held.len() == 100`, `held_more == 30`.
  - `the_opening_counts_once_against_the_flood_cap`.
  - `an_off_or_capped_opening_keeps_the_events`: Dots paused at the opening → still held; cap full → still held and the watcher paused as flooded.
  - `held_events_are_released_at_start_once` (Review Focus 2): hold, `restart()` at Mon 09:30 → exactly one run, also after the supervisor's timers run.
  - `editing_hours_so_now_is_inside_releases_at_once` (Review Focus 5); `removing_hours_releases_at_once`; `resuming_inside_hours_releases`; `turning_off_drops_held_events_and_their_files`; `deleting_drops`.
  - email: no `poll_now` happens while outside (fake mail sees no request); at the opening the new mail and the earlier held events go into one run.
  - GitHub (Review Focus 4): outside hours no request reaches wiremock; at the opening an issue and a PR found by the opening poll join one run; the cursor ends past both.
  - `an_email_attachment_held_overnight_reaches_the_run` (Review Focus 3): the opening run's `attachments` lists the file and `copy_into` copies it.
  - `next_opening` is set on a watcher outside its hours and `None` inside.
  - prompt (`watchers_prompt_test.rs`): an opening payload renders the first line, the events, and `…and 30 more.`
- [ ] **Step 2: Run** the four test files. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the watcher test files 3 times each, then the full `dots-core` suite once. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): at the opening a watcher's dot runs once with what it held`.

### Task 5: App and contract

**Files:**
- Modify: `apps/dots-app/src/commands.rs` (`watcher_add` :1479, `watcher_update` :1490), `apps/dots-app/src/contract.rs` (`fn watchers()` :412)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Produces: `watcher_add(dot_id, name, config, hours: Option<WatchHours>)`, `watcher_update(id, name, config, hours: Option<WatchHours>)`; contract samples include one watcher with hours, `held_count: 4`, `next_opening` set, and a `WatchHours` sample.

- [ ] **Step 1: Update** commands and samples.
- [ ] **Step 2: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app`. Expected: PASS.
- [ ] **Step 3: Commit** `feat(app): watchers carry their hours`.

### Task 6: UI

**Files:**
- Modify: `ui/src/api/types.ts` (`Watcher` :417, new `WatchHours`), `ui/src/api/commands.ts` (:421-424), `ui/src/screens/dot-editor/WatchersTab.tsx` (`WatcherForm` :164, row status :562-566), `ui/src/lib/watchers.ts` (new `watcherHoursLine(w, now)`)
- Test: `ui/src/screens/dot-editor/WatchersTab.test.tsx`, `ui/src/lib/watchers.test.ts`

**Interfaces:**
- Consumes: Task 5 contract.
- Produces: `commands.watcherAdd(dotId, name, config, hours)`, `commands.watcherUpdate(id, name, config, hours)`; `watcherHoursLine(w: Watcher, now: Date): string` (`""` when `next_opening` is null).

- [ ] **Step 1: Write the failing tests**:
  - `watcherHoursLine`: 4 held, opening today 09:00 → `Outside hours · 4 held until 09:00`; 0 held → `Outside hours · until 09:00`; opening Monday from a Saturday → `Outside hours · 4 held until Mon 09:00`; null → `""`.
  - Editor: defaults to `Any time` (saves `hours: null`); `Only at these times` with Mon–Fri and 09:00–18:00 saves `{days:["mon",...,"fri"],start:"09:00",end:"18:00"}`; end before start shows `Ends the next day`; a core error appears in the dialog's alert.
  - Row: shows `watcherHoursLine` beside today's state line.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement** with the Global Constraints copy; day toggles as accessible buttons with `aria-pressed`.
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): watch hours in the watcher editor and its row`.

### Task 7: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps: `cargo fmt --all -- --check`; `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`; `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times` (twice); `cargo test -p dots-app --locked`; `node --test tools/release/release.test.mjs`; `cd ui; bun run typecheck; bun run test`. Expected: all pass, both core runs.
- [ ] **Step 2: Push** `feat/watch-hours` and open the PR against `master`, noting it is stacked on #46 (merge #46 first).
