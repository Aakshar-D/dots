# Stuck-run watchdog

- Date: 2026-11-02
- Status: approved (owner reviewed this document, 2026-11-02)
- Part of: Tier 1 of the build list (reliability first). This is the first of seven, each its own design and pull request: watchdog, skip a schedule when the dot is busy, watch hours, delivery tracking, snooze and done, send later, attachments.
- Builds on: `2026-10-08-dots-design.md` (run lifecycle, §4 timeouts), `2026-10-22-needs-you-design.md` (Needs you, notifications, tray).

## 1. Goal

A run that hangs is caught and stopped, and the person is told, instead of sitting silently until its time limit.

What exists today: each dot has a time limit (`DotSpec.timeout_secs`, default 1800 s). The runner (`runner.rs`, the `select!` loop after `engine.start`) cancels a run that passes it and ends it `failed` with `timeout after Ns`. Nothing notices a run that has stopped producing anything: a hung shell command, a stuck MCP server or a model call that never returns all wait out the full limit (up to 7 days if the dot's limit is raised), and the person sees "running".

Success: a run with no output for the quiet limit shows up in Needs you with a notification. The person can stop it or keep waiting. If nobody answers, Dots stops it at twice the limit and says why.

## 2. Words

- **Output**: any `EngineEvent` the runner receives for the run (text, a tool call, a tool result, limits, finished, failed: every event that reaches `self.record`).
- **Quiet**: time since the run's last output, or since its engine started if it has produced none.
- **Quiet limit**: how long quiet may last before the warning. App default 10 minutes (Settings); each dot may set its own or turn the watchdog off.
- **Waiting on the person**: the run has a pending approval (`store.has_pending_for_run`). That time is not quiet.

## 3. Behaviour (decided with the owner)

1. **Warn at the quiet limit.** When a run has been quiet for its quiet limit, it is marked **quiet** and:
   - it counts in its dot's **needs you** (so in the column badge, the Home count and the tray's Needs you submenu);
   - a card on the run in the conversation (and the run view) says **"Quiet for 10 min"**, with **Stop** and **Keep waiting**;
   - a Windows notification is sent (§5.2), unless the person is viewing that dot (the needs-you quiet rule).
2. **Long shell commands count as quiet.** A tool call that is still running (a build, a test suite) gives no output until it ends; that time counts. A hung command is what this catches. A dot that runs long builds raises its own limit.
3. **Keep waiting** clears the warning and restarts the quiet clock: the next warning comes after another full quiet limit of no output. The run's time limit still applies.
4. **Stop** cancels the run the same way the Cancel button does; it ends `cancelled`.
5. **Auto-stop at twice the limit.** A run still quiet at twice its quiet limit (10 minutes after the warning by default), with no Keep waiting, is stopped by Dots: cancelled through the same path as the time limit (the run's job object is terminated after the cancel grace), and ended `failed` with the error **`stopped: no output for 20 min`** (the minutes being twice the quiet limit, written in minutes).
6. **Any output clears it.** The quiet mark goes, the card and the needs-you count with it, and the clock restarts.
7. **Approvals pause it.** While the run is waiting on the person, no warning and no auto-stop. The clock restarts when the wait ends (the decision lands or the approval is parked).

Owner's calls: warn then stop; 10 minutes; long commands count; a per-dot setting in the dot editor; auto-stop 10 minutes after the warning; Keep waiting restarts the clock; Needs you plus a notification; a scheduled run the watchdog stopped is reported at once and not retried (§4.5).

## 4. Core

### 4.1 Settings and the dot

- **`Settings.quiet_limit_secs: u64`**, default `600`. Validated `60..=86400`.
- **`DotSpec.quiet_limit_secs: Option<u64>`**, `#[serde(default)]` (a spec written before reads as `None`). `None`: the app default. `Some(0)`: watchdog off for this dot. Otherwise validated `60..=86400`. *Builder's call:* where the field sits in the struct; it must round-trip through the existing dot save and export paths.
- The effective limit is read when the run's engine starts; a change applies to the next run.

### 4.2 The runner

The watchdog lives in the runner's existing event loop, next to the time limit. A third timer in the same `select!`:

- `quiet_at = last_output + limit` (warn), then `stop_at = quiet_since + limit` (auto-stop). `last_output` starts at `engine.start` and moves on every event received.
- When the warn timer fires: if the run is waiting on the person, restart the clock from now and do nothing else. Otherwise mark the run quiet (§4.3) and arm the auto-stop timer.
- When the auto-stop timer fires: if waiting on the person, clear the mark and restart the clock. Otherwise cancel exactly as the time limit does (`cancel.cancel()`, then the grace), and remember it was the watchdog so the ending reads `stopped: no output for N min` instead of `timeout after Ns`. If the time limit and the watchdog both fired, the first one to fire names the error.
- On any event while quiet: clear the mark (§4.3), restart the clock.
- **Keep waiting** reaches the loop the way a cancel does: the runner keeps, per running run, a way to tell its loop (*builder's call*: a `Notify` or a channel kept alongside the run's cancel token in the runner's registry). On it: clear the mark, restart the clock.
- Watchdog off (`Some(0)`): no timer.
- The approval check is a store read only when a timer fires, never per event.

### 4.3 The quiet mark

- **Migration 0026** (`0026_run_quiet.sql`), three columns:
  - `ALTER TABLE dots ADD COLUMN quiet_limit_secs INTEGER` (null: app default; 0: off). `DotSpec` is stored as columns of `dots`, so §4.1's field needs one.
  - `ALTER TABLE runs ADD COLUMN quiet_since TEXT` (RFC 3339, null when not quiet). A run that ends has it cleared in the same write as its ending. At app start, runs left `running` already become `failed` (`interrupted`); their mark is cleared with them.
  - `ALTER TABLE runs ADD COLUMN quiet_limit_secs INTEGER`: the effective limit the run started with (null: watchdog off), written when its engine starts. *My call:* stored so the UI's card shows the right minutes even if the dot or the app default changed mid-run.
  - `ALTER TABLE runs ADD COLUMN quiet_stopped INTEGER NOT NULL DEFAULT 0`: 1 when the watchdog ended the run (§3.5), written with its ending. The scheduler (§4.5) reads it instead of matching the error text.
- `Run` gains `quiet_since: Option<String>`, `quiet_limit_secs: Option<u64>` and `quiet_stopped: bool`.
- Setting or clearing the mark emits a runtime event **`run_quiet { run_id, dot_id, quiet_since | null, quiet_limit_secs }`** so the UI, the tray and the notifier refresh.
- **Needs you** (needs-you spec §2) becomes: open questions + pending approvals + **quiet runs**. `DotOverview.needs_you` counts them; so `unread` does too.

### 4.4 Runtime

- **`Runtime::keep_waiting(run_id)`**: `NotFound` for an unknown run; `Conflict` if the run is not running or not quiet. Otherwise tells the runner (§4.2).
- Stop uses the existing cancel (whatever the Cancel button calls today).

### 4.5 A scheduled run the watchdog stopped (decided with the owner)

Today a failed scheduled run is retried with no notice, and the person hears only when retries run out (`ScheduleAttention`; app `events.rs` skips `RunEnded` for failed scheduled runs). A run that hangs would hang again on each retry.

So a scheduled run that ends with `quiet_stopped` is **not retried**: the scheduler treats it as if its retries were used up, and the schedule goes straight to **Needs attention** with the run's error. The existing `ScheduleAttention` notice then fires at once: "`<dot>`'s schedule needs attention" / `stopped: no output for 20 min`. Later slots of the schedule fire as normal. *Builder's call:* where in `scheduler.rs` the watched run's `Failed` branch checks the flag.

## 5. App

### 5.1 Commands

- `keep_waiting(runId)`: thin wrapper of §4.4.
- The contract fixture gains `Run.quiet_since`, `DotSpec.quiet_limit_secs`, `Settings.quiet_limit_secs` and the `run_quiet` event.

### 5.2 Notification (`events.rs`)

On `run_quiet` with a time set (not on clearing), any trigger, chat runs included, except for the dot being viewed:

| Notice | Title | Body |
|---|---|---|
| Run gone quiet | `<dot> has gone quiet` | `No output for <n> min. Open it to stop it or keep waiting.` |

The click opens `/chat/<dot>/<run>` (the existing route map).

When the watchdog stops a run, the person is told at once, with no new notice type:

- **Scheduled run**: the schedule's Needs attention notice (§4.5).
- **Watcher, follow-up, manual and other background runs**: the existing "`<dot>` failed" notice, whose body is the error's first line, `stopped: no output for 20 min`.
- **Chat run**: *my call:* it also gets that "`<dot>` failed" notice (today a chat run that ends does not notify), because the person may have walked away from it. Not for the dot being viewed.

### 5.3 Tray

Counts quiet runs through `needs_you`; nothing else changes.

## 6. UI

- **The quiet card**: on a running run with `quiet_since`, in the conversation where the run's live progress shows and on the run view, in the waiting colour: "Quiet for `<n>` min" (counted live from the last output), the line "Stops by itself in `<m>` min" (until auto-stop), and two buttons, **Stop** (the existing cancel) and **Keep waiting** (`keep_waiting`). A failed call shows its error next to the buttons. *Builder's call:* exact placement, following how pending approval cards sit in the conversation today.
- **Dot editor**: under the time limit, **"Quiet limit"**: a choice of *App default (10 min)*, *Off*, or a number of minutes (1 to 1440). Its help line: "If this dot shows no output for this long, Dots asks you; at twice this long it stops the run."
- **Settings** (in the group with the concurrent-runs setting): **"Quiet limit for every dot"**, minutes, 1 to 1440, default 10.
- Live: `run_quiet` refreshes the run, the conversation and the overview.

## 7. Edge cases

- **Waiting on a live approval for a long time**: never quiet (§3.7). A parked run is not running, so it has no clock.
- **Quiet limit above the time limit**: allowed; the time limit simply fires first. *My call:* the dot editor shows a hint "Longer than the time limit; the time limit will end the run first." No error.
- **PC asleep**: *builder to confirm* whether tokio's `Instant` advances across Windows sleep. If it does, the first warning after resume is cleared by the run's next output; acceptable either way.
- **Keep waiting raced by output**: both clear the mark; the clock restarts once. Keep waiting on a run that just ended: `Conflict`, the UI refreshes.
- **Stop raced by auto-stop**: the cancel path is already idempotent; the person's Stop ends it `cancelled`, the watchdog's ends it `failed`, whichever reaches the loop first.
- **Local engine runs**: same rules; any `EngineEvent` counts as output.
- **Team and workflow parent runs**: each run has its own clock. Checked: no run blocks on a child (`start_workflow` does not wait; resume runs start after their parent ended), so nothing extra.

## 8. Testing

- Core (runner, with the fake engine and short limits): a quiet run is marked at the limit and the event is emitted; output clears the mark; Keep waiting clears it and the next warning comes a full limit later; auto-stop at twice the limit ends the run `failed` with `stopped: no output for N min`; a pending approval pauses both timers; watchdog off sets no mark; the time limit firing first keeps `timeout after Ns`; the mark is cleared when the run ends any way and at start for interrupted runs; `quiet_stopped` is set only by the watchdog; `needs_you` counts quiet runs; `keep_waiting` errors (unknown, not running, not quiet).
- Scheduler: a scheduled run stopped by the watchdog is not retried and its schedule needs attention at once; one that failed any other way still retries; later slots still fire.
- Settings and spec: defaults, validation bounds, an old spec without the field reads as `None`.
- App: the gone-quiet notice's wording and route; none for the viewed dot; a watchdog-stopped chat run notifies as failed, other chat runs still do not; the contract fixture.
- UI: the card (minutes, both buttons, error); the dot editor's three choices and hint; the Settings field.

## 9. Collisions with parallel work

Small additions to shared files: core `runner.rs` (the loop), `scheduler.rs` (no retry after a watchdog stop), `store/dots.rs` (the dot's column), `model.rs`, `settings.rs`, `store/runs.rs`, `store/dots.rs` (the count), `runtime.rs`, core `events.rs`, a migration (0026: tell the parallel developer it is taken); app `events.rs`, `commands.rs`, `lib.rs`, `contract.rs`; UI `DotForm.tsx`, the settings screen, the conversation's run card, `api/types.ts`, `commands.ts`. New UI code in a new file (`ui/src/screens/home/QuietCard.tsx`).

## 10. Not in this part

- A "still alive" signal from the engine itself (the agents-app's heartbeat): Dots' engines already stream events, so output stands in for it.
- Retrying a run the watchdog stopped: never automatic (§4.5); the person reruns it.
- Skipping a schedule while the dot is busy: the next design.
