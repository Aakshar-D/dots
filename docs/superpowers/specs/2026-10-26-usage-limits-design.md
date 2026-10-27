# Usage and limits: dots that leave room for the person's own Claude work

- Date: 2026-10-26
- Status: built (plan `docs/superpowers/plans/2026-10-26-usage-limits.md`); section 12 records how it differs
- Builds on: the Claude engine (`crates/dots-core/src/engine/claude/`), the runner's dispatcher (`runner.rs`, `dispatch_once`, `set_held`), the scheduler's coalescing (`scheduler.rs`, `store/schedules.rs` `start_or_join`), `2026-10-23-watchers-design.md` (watcher runs, the flood cap), `2026-10-22-needs-you-design.md` (the Inbox, notifications, the tray).

## 1. Intent

The person uses one Claude subscription for their own work (Claude Code, claude.ai) and for their dots, which run through the official `claude` CLI logged in as them. The subscription has usage windows: a 5-hour window and a weekly window (and weekly windows per model). Dots running in the background can use up the allowance the person needs for their own work, and today nothing shows it or stops it.

This phase does two things:

1. **Holds background dots back** when the account's Claude usage is high, so the person keeps room for their own work. Their own actions in Dots (chat, Run now) are never held back.
2. **Shows usage**: a live meter of the account's limits, a Usage page per dot, and numbers on each run.

Success: on a busy afternoon the 5-hour window reaches 60%; the nightly-tests schedule and an Invoices watcher event wait instead of starting; the rail bars turn amber and the tray says "Holding 2 runs until 15:10"; the person can still chat with any dot; after 15:10 the two runs start one after another; the Usage page shows nightly-tests used most of the window.

## 2. Goals, non-goals, anti-goals

Goals: read the account's limit windows from the CLI; hold background Claude runs above a share the person sets; start held runs after the reset, one at a time; a live meter (rail and tray); a Usage page; tokens, run time and API-equivalent cost per run; held runs in the Inbox with **Run anyway**.

Non-goals (this phase): budgets per dot or caps in tokens or dollars; real API billing (all Claude use is the subscription); switching a dot to a local model when Claude is held; predicting when a limit will be hit; charts; exporting usage; reading limits by any means other than the CLI's own output (no calls to Anthropic's API, no reading the CLI's credentials: the subscription constraint in `2026-10-08-dots-design.md`).

Anti-goals (a failure even if it works): the person's chat or Run now refused or delayed by the hold; a held run lost or never started; a burst of held runs all starting at the reset and eating the new window; a meter that looks precise when its reading is old; Dots running a request of its own just to read the limits.

## 3. Reading the limits

The `claude` CLI (tested with 2.1.310) writes, in its `stream-json` output, events of type `rate_limit_event`. Each carries the limit status (`allowed`, `allowed_warning`, `rejected`), the window that currently limits (`rateLimitType`: `five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet`, ...), its `utilization` (a fraction, usually 0 to 1), `resetsAt` (Unix seconds), and `unifiedWindows`, every window's own utilization and reset. The exact shape is to be confirmed by the builder against a real run (section 11) and pinned in a fixture of the fake CLI.

- The Claude engine's parser turns each such event into a new `EngineEvent::Limits` with the windows it names. It is not written to the transcript as text. A line the parser cannot read is ignored (logged at debug), never a run failure.
- The runner hands each `Limits` event to a new **limits** service in the core, which keeps the latest reading per window: `window`, `utilization` (0 to 1), `resets_at`, `status`, `read_at`. Kept in a new table (section 7) so the meter survives a restart.
- The reading is account-wide: the person's own Claude use counts. That is the point.
- A window whose `resets_at` has passed is **unknown** until the next reading: it no longer holds runs back, and the meter shows it as "reset, no reading yet". A window never read is unknown too.
- Dots never starts a run, or calls anything, to get a reading. Readings come only from runs that happen anyway.
- The CLI may not emit the event on every run (section 11 confirms how often). Nothing depends on it doing so: a run that ends with no reading leaves the stored reading as it was, a hold lifts at the latest at `resets_at`, and the meter shows the reading's age.

## 4. Holding back

**Settings** (the Settings screen, a new "Claude usage" section): a switch "Hold background dots when Claude usage is high" (on by default); "5-hour window share" (default 60%) and "weekly window share" (default 70%), each 10 to 100% in steps of 5. The per-model weekly windows are shown but never hold runs back.

**When it holds** (the guard): a run is held while all of these are true:

1. its dot's engine is `claude` (local-model runs are never held);
2. its trigger is background: `schedule`, `watcher`, `follow_up` or `webhook`. `chat`, `manual` (Run now) and `resume` (a run continuing after the person's approval) are never held;
3. the switch is on, and the 5-hour window's known utilization is at or above its share, or the weekly window's is at or above its share;
4. it has not been released with **Run anyway**.

**Limit reached** (builder's call, from the review): while the latest status is `rejected` and its window has not reset, the CLI would refuse any Claude run, so:
- chat and Run now are refused at once with "Claude's limit is reached until 15:10" (no run is created);
- a resume after the person's approval stays `queued` and starts after the reset (the approval is not lost);
- background runs are held as above; **Run anyway** does not get past `rejected`.

A background run that started under the share but ends because the CLI reported `rejected` before it made any tool call goes back to `queued` (held), keeping its trigger and payload, so a watcher event is not lost. One that had already made a tool call fails as today, with the note "stopped by Claude's usage limit"; it shows in the Inbox as failed runs do.

**How it holds:** a held run stays `queued`; the dispatcher skips it (as it skips a dot that already has a running run). Nothing new is stored on the run for the hold itself; whether a queued run is held is computed from the guard. Each watcher event stays its own queued run, and the watcher's flood cap (20 per hour, then pause) keeps that bounded.

**Coalescing** (builder's call): today the scheduler folds a slot (or a retry) that fires while the dot has any queued run into that run. With holds lasting hours a schedule would fold into a held watcher run and lose its own identity. This phase narrows it: a schedule slot or retry joins only a queued run whose trigger is `schedule`. So a schedule that fires several times while held still runs once.

**After the reset** (the ramp): when a hold ends (its window reset, or a reading shows it under the share), the limits service enters **draining**, kept in memory. While draining, at most one background Claude run runs at a time, oldest queued first, whether it was queued before or after the hold ended. A reading at or above a share during draining holds the rest again. Draining ends when no background Claude run is queued. A run that ends with no reading (it failed early, was cancelled, or the CLI gave none) still lets the next one start. Draining is not stored: after a restart, while either window is unknown, the same one-at-a-time rule applies, and it lifts with the first reading under both shares.

**Run anyway** (Inbox, run card, run page): releases that one run from the hold and from draining's queue. It is stored on the run (`hold_released`), so it survives a restart.

**Notifications:** one when holding starts ("Dots is holding background runs: Claude 5-hour usage is at 64%"), one when it ends ("Held runs are starting"). Not one per run.

## 5. What it shows

**Run numbers.** Each run already stores `tokens_in` and `tokens_out`. New: `cost_usd`, the CLI's `total_cost_usd` from its result line (null for local runs, and for Claude runs whose CLI did not give one). Run time is `finished_at - started_at`. Run cards and the run page show: "12m 04s · 182k tokens (170k in, 12k out) · $1.32 API-eq." The tooltip on "API-eq." says the subscription does not bill per run, so this is what the run would cost on the API.

**Live meter** (the person's choice C):

- At the bottom of the nav rail, on every screen: two thin vertical bars, 5-hour and weekly, filled to the known utilization. Teal normally, amber while runs are held. Hover or focus shows "Claude 5h 42%, resets 15:10 · week 55%, resets Thu 09:00 · holding 0 runs", and the age of the reading when it is older than 10 minutes ("read 25 min ago"). An unknown window shows an empty bar with a dashed outline. Click opens the Usage page.
- The tray icon's tooltip gains a second line: "Claude 5h 42% · week 55%", and "Holding 3 runs until 15:10" while holding. It keeps its Needs you count.

**Usage page** (route `/usage`, opened from the rail bars; the person's choice A, table first):

- Top: the same two windows, with their shares from Settings and "Holding N runs" (a link to the Inbox).
- A period switch: **5 hours / Week / Month** (the last 5 hours, 7 days, 30 days, by each run's `started_at`).
- A table, one row per dot with a run in the period, sorted by tokens: dot, runs, tokens (in + out), run time, API-eq. $ (a dash for local dots). A deleted dot's runs are deleted with it (`ON DELETE CASCADE`), so they are not counted; the page says so in a line under the table.
- Clicking a row expands that dot's five heaviest runs in the period (by tokens), each a link to the run page.
- These are Dots' own runs only; the meter above is the whole account.

**Inbox:** a new list "Held for Claude usage", one row per held run: dot, trigger, queued at, "starts after 15:10" (or "waiting for the first run after the reset"), and **Run anyway**. Held runs count in Needs you only as one line ("3 runs held"), not each.

## 6. Events and contract

- A new `RuntimeEvent::LimitsUpdated` (the windows, the hold state, the held count), sent on each new reading, on a hold start or end, and when a held run starts or is released.
- New commands: `limits_get` (windows, hold state, held run ids), `usage_summary(period)` (the table and each dot's heaviest runs), `run_release_hold(run_id)`. Settings gain the three fields.
- The app's contract fixture is regenerated (`UPDATE_CONTRACT=1 cargo test -p dots-app contract`).

## 7. Storage

Migration `0015_usage.sql`. On master, 0013 is orgs and 0014 watchers; the other developer's open PRs #22 (teams) and #24 (apps) each claim 0013 and will renumber when they merge. If one merges first and takes 0015, this one moves to the next free number. As built: #22 took 0015 (teams) and #24 0016 (apps) first, so this phase ships `0017_usage.sql`.

- Table `claude_limits`: `window` (primary key, the CLI's name), `utilization` (real), `resets_at`, `status`, `read_at` (ISO-8601 UTC, `util::now()`).
- `runs`: `cost_usd REAL` (nullable), `hold_released INTEGER NOT NULL DEFAULT 0`.
- Index `runs(started_at)` for the Usage page, if none exists.
- Settings: `hold_enabled`, `hold_share_five_hour`, `hold_share_weekly`, stored where the other settings are.

## 8. Collisions with parallel work

Open PRs by the other developer: #22 (teams and workflows) and #24 (apps). Shared files this phase touches: `model.rs` (settings, run fields), `runner.rs` (the dispatcher's guard, the `Limits` event, re-queueing a run ended by `rejected`), `store/schedules.rs` (`start_or_join` joins schedule runs only), `engine/claude/parse.rs`, `engine/mod.rs`, `events.rs`, `runtime.rs`, `store/runs.rs`, the app's `commands.rs`, `lib.rs`, `contract.rs`, `tray.rs`; in the UI `api/{commands,types}.ts`, `app/Layout.tsx` (the rail bars), the router (the `/usage` route), the Inbox screen, Settings, run cards and the run page. New files: `limits.rs` (the service and the guard), `store/limits.rs`, `migrations/0015_usage.sql`, `screens/UsageScreen.tsx`, the meter component and their tests. Before the PR, merge master in and resolve as the watchers phase did.

## 9. Testing

- Parser: the fake CLI emits `rate_limit_event` lines (a fixture copied from a real run, section 11); the engine turns them into `Limits`; a malformed one is ignored.
- Guard: each trigger × engine × reading combination (held or not); the switch off; an unknown window; a reset that has passed; `rejected` holds the person's chat too; **Run anyway** releases one run and survives a restart.
- Dispatcher: held runs stay queued while others start; draining starts them one at a time, oldest first, and goes on when a run ends with no reading; after a restart with unknown windows the same rule applies; a schedule firing three times while held leaves one run, and does not join a queued watcher run; a background run ended by `rejected` before any tool call is queued again with its payload; chat and Run now are refused under `rejected`.
- Store: `cost_usd` from a result line; the usage summary per period, sorted.
- UI (Vitest, fake backend): the rail bars (levels, amber, unknown, stale reading text, keyboard focus shows the numbers, click opens Usage); the Usage page (period switch, sorting, expanding a row, the local dash); the Inbox list and **Run anyway**; Settings fields and their bounds; the run card's numbers.
- App: the tray tooltip's second line; the notifications on hold start and end.
- Smoke (a fresh scratch `DOTS_DATA_DIR`, never the person's real Edge profile): one real Claude run to capture a `rate_limit_event`; then the shares set low (10%) so a schedule is held, **Run anyway**, and the hold lifting when the share is raised.

## 10. Accessibility

The rail bars are one focusable control (a link to `/usage`) whose accessible name reads the numbers ("Claude usage: 5-hour 42%, resets 15:10; weekly 55%, resets Thursday 09:00; holding 0 runs"). Colour is never the only signal: holding also shows in the name and the tooltip. The Usage table is a real table with a caption; the expand control is a button with `aria-expanded`. Same axe and keyboard checks as earlier phases, light and dark, down to 420 px wide.

## 11. Left to the builder

- The exact shape of `rate_limit_event` and its `unifiedWindows` in CLI 2.1.310, confirmed from one real run's output and pinned in the fake CLI's fixture. If the CLI gives only the currently limiting window, the core keeps per-window readings as they arrive.
- Whether `utilization` can exceed 1 (show at most 100%, store as read).
- The tray tooltip's wording within the lines above.
- The visual detail of the rail bars within the mockup (teal, amber, dashed when unknown).

## 12. As built

### 12.1 Where it differs

| Spec | As built | Why |
|---|---|---|
| §4: after a restart, "while either window is unknown" one at a time | Only a hold window that **reset** since its last reading (or draining) forces one at a time; a window **never read** does not | If the CLI named only the limiting window, the weekly one might never be read and runs would go one at a time forever |
| §4: a schedule slot joins "only a queued run whose trigger is `schedule`" | A slot (or a retry) joins any queued run except a `watcher`, `webhook` or `follow_up` one | Joining a queued Run now run is earlier, tested behaviour; the point is that a slot does not vanish into an event's run |
| §5: held runs count "in Needs you only as one line" | The rail's Inbox count and the tray's to-review count add 1 when any run is held; per-dot needs-you counts are unchanged | Held runs are not one dot's doing |
| §4: a share out of bounds is the core's refusal | The Settings screen also checks it, with the core's words, before saving | As the other number fields do |
| §6: an event "on each new reading" | `LimitsUpdated` is sent only when what it carries changed (the dispatcher publishes on every pass; the state at start counts as sent) | Otherwise one event every 2 seconds |
| (silent) a reset that passes | No timer of its own: the dispatcher looks every 2 seconds and the guard reads the clock | A reset releases runs within 2 seconds |

### 12.2 Decided by the builder (section 11)

- `rate_limit_event` in CLI 2.1.310 (captured from a real run, pinned in `crates/dots-core/tests/fixtures/rate_limit_event.json`): `rate_limit_info` has `status`, `resetsAt` (Unix seconds), `rateLimitType`, and `unifiedWindows` with `five_hour` and `seven_day`, each `{utilization, resetsAt, status}`. Every run reports both windows. A line with a status and no window is a `Limits` event with no readings; one without a readable status is ignored. The result line's `total_cost_usd` is the run's API-equivalent cost.
- Utilization is stored as read and shown at most 100%.
- The tray: "Dots: N to review", then "Claude 5h 42% · week 55%" ("–" for an unknown window), then "Holding N runs until 15:10" while holding. The hold notices are titled "Claude usage" and open the Usage page.
- Tokens show one decimal under 10k ("1.5k"). The run page shows the thread's API-eq. cost next to its tokens, and a held run's reason with **Run anyway**.
- Run events stored before this phase have no `cost_usd`; the UI reads them as before.

### 12.3 Smoke

Run on 2026-10-27, debug build of `feat/usage-limits` at `db4eb89`, a fresh data folder, Claude CLI 2.1.310 logged in as the person (`docs/verification/2026-10-27-usage-limits-smoke.md`).

- One real run ("Reply with the single word ok"): 24,392 tokens in, 50 out, $0.098 API-eq.; the readings were 5-hour 1% and weekly 18%, both kept.
- The weekly share set to 10%: the next slot of a one-minute schedule was held; the Inbox listed "ticker · Scheduled · starts after Wed 9:30 am" with **Run anyway**; the rail's bars turned to the holding colour and read "…; holding 1 runs"; the Inbox link counted 1.
- Run now of another dot during the hold started and succeeded; **Run anyway** on the held run started it and it succeeded.
- The share raised back to 70% with a run held: it started at once and succeeded.
- The Usage page listed both dots (runs, tokens, run time, cost). axe: no violations on Usage, Inbox, Settings and Home, dark and light; no sideways scroll at 420 px.
- Not seen: the tray tooltip and the Windows notices (not reachable from the page; covered by the app's unit tests).
- Found: the rail's bar tracks barely show at low usage; "holding 1 runs".
