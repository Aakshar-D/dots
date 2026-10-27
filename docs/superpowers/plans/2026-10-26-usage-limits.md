# Usage and Limits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Background Claude runs wait while the account's 5-hour or weekly Claude usage is at or above a share the person sets, start one at a time after the reset, and the person sees the limits (rail bars, tray), a Usage page per dot, and tokens, run time and API-equivalent cost on each run.
**Architecture:** The Claude engine's parser turns the CLI's `rate_limit_event` lines into `EngineEvent::Limits` and reads `total_cost_usd` into `EngineEvent::Usage`. A new core service `limits::Limits` (`crates/dots-core/src/limits.rs`) keeps the latest reading per window (table `claude_limits`, migration 0015), the hold shares from `Settings`, and the in-memory draining flag; it answers the dispatcher's one question, `admit`, and publishes `RuntimeEvent::LimitsUpdated`. The runner asks `admit` before claiming a queued run, feeds it each `Limits` event, and puts back a background run that the CLI's `rejected` ended before any tool call. The runtime refuses chat and Run now under `rejected`, adds the Inbox's held list, the usage summary and **Run anyway**; the app wraps three commands, extends the tray tooltip and notifies on hold start and end; the UI adds the rail bars, `/usage`, the Inbox list, run numbers and a Settings section.
**Tech Stack:** Rust 1.97 (sqlx SQLite, tokio, chrono), Tauri 2.12; React 19.3, @tanstack/react-query 5.104, zod 4.6, radix-ui, lucide-react, Vitest 5.0 with Testing Library and the fake backend (`ui/src/test/backend.ts`); Bun 1.4.3. No new crate or npm package.
**Spec:** docs/superpowers/specs/2026-10-26-usage-limits-design.md (sections 1 to 10; section 11 is decided below)

## Global Constraints

- Repository root `C:/Users/shrad/OneDrive/Documents/LOQ/Dots/dots`, branch `feat/usage-limits` (at `4129cdb` when this plan was written); commands run from the root (Git Bash). Never commit to `master`. `git add` names its files: never `git add -A` or `git add .`. No `Co-Authored-By` line or any other AI trailer in any commit message.
- Toolchain: Rust 1.97 stable; Bun 1.4.3 at `~/.bun/bin` (`cd ui && ~/.bun/bin/bun ...`). Every `cargo` command that builds `dots-core` tests or `dots-app` runs with `export PATH="/c/Program Files/LLVM/bin:$PATH"` first. Never run `bun x prettier` (the repo has no prettier; it reformats whole files).
- Migration `crates/dots-core/migrations/0015_usage.sql` (0013 orgs, 0014 watchers). Table `claude_limits`: `window TEXT PRIMARY KEY`, `utilization REAL NOT NULL`, `resets_at TEXT NOT NULL`, `status TEXT NOT NULL`, `read_at TEXT NOT NULL`. `ALTER TABLE runs ADD COLUMN cost_usd REAL`; `ALTER TABLE runs ADD COLUMN hold_released INTEGER NOT NULL DEFAULT 0`; `CREATE INDEX IF NOT EXISTS runs_started ON runs(started_at)`. Times are the core's `util::now()` form (RFC 3339 UTC).
- Numbers: settings `hold_enabled` default `true`, `hold_share_five_hour` default `60`, `hold_share_weekly` default `70` (percent, `u8`, 10 to 100 in steps of 5); `limits::STALE_AFTER = 10 min` (the meter shows the reading's age past it); Usage periods `five_hours` = 5 h, `week` = 7 days, `month` = 30 days back from now, by `started_at`; `limits::HEAVIEST = 5` runs per dot.
- Windows: the CLI names (`five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet`, …) are stored as read. Only `five_hour` and `seven_day` hold runs back (`limits::FIVE_HOUR`, `limits::WEEKLY`). A window is **known** when it has a row whose `resets_at` is after now; **reset** when it has a row whose `resets_at` is past; **never read** when it has no row.
- Background triggers: `schedule`, `watcher`, `follow_up`, `webhook` (`limits::is_background(TriggerKind) -> bool`). `chat`, `manual`, `resume` are the person's. Only `EngineKind::Claude` runs are ever held.
- Exact words, core (in `limits.rs`): `SHARE_RULE` "A share is 10 to 100%, in steps of 5."; `limit_reached(until)` "Claude's limit is reached until <until>." (`until` is `resets_at` in local time, `%H:%M` when it is today, else `%a %H:%M`); `STOPPED_BY_LIMIT` "stopped by Claude's usage limit"; notices `hold_started(window_label, pct)` "Dots is holding background runs: Claude <5-hour|weekly> usage is at <pct>%" and `HOLD_ENDED` "Held runs are starting"; `NOT_HELD` "This run is not held." (Run anyway on a run that is not a queued, held background Claude run).
- Exact words, UI: the rail control's accessible name "Claude usage: 5-hour <p>%, resets <t>; weekly <p>%, resets <t>; holding <n> runs" (an unknown window reads "5-hour unknown"); its tooltip adds "read <n> min ago" past 10 minutes; tray second line "Claude 5h <p>% · week <p>%" and, while holding, a third "Holding <n> runs until <t>"; page title **Usage**, period switch **5 hours** / **Week** / **Month**, table caption "Dots' runs in the last 5 hours|7 days|30 days", columns Dot, Runs, Tokens, Run time, API-eq.; under the table "A deleted dot's runs are deleted with it and not counted."; empty "No runs in the last 5 hours|7 days|30 days."; Inbox section **Held for Claude usage**, row text "starts after <t>" or "waiting for the first run after the reset", button **Run anyway**; run numbers "<duration> · <tokens> tokens (<in> in, <out> out) · $<x.xx> API-eq." (no cost part for a run without one; no duration part for a run that has not ended); on a held run "Held: Claude <5-hour|weekly> at <p>%, starts after <t>"; the "API-eq." tooltip "Your subscription does not bill per run: this is what the run would cost on the API."; Settings section **Claude usage**, switch "Hold background dots when Claude usage is high", fields "5-hour window share" and "Weekly window share" (%). Formats: tokens `950`, `12k`, `182k`, `2.1M`; duration `45s`, `12m 04s`, `3h 12m`; cost `$1.32`. Colours: the theme's tokens only (`ui/src/lib/look-guard.test.ts`): normal bars `bg-primary`, holding bars `bg-needs`.
- Names: `EngineEvent::Limits { status: String, windows: Vec<LimitReading> }`; `EngineEvent::Usage` gains `cost_usd: Option<f64>`; `RuntimeEvent::LimitsUpdated { limits: LimitsState, change: Option<HoldChange> }` (`{"type": "limits_updated", ...}`); commands `limits_get()`, `usage_summary(period)`, `run_release_hold(runId)`; query keys `keys.limits` (`["limits"]`) and `keys.usage(period)` (`["usage", period]`); `Inbox.held`. The contract fixture `ui/src/api/__fixtures__/contract.json` is only ever regenerated (`UPDATE_CONTRACT=1 cargo test -p dots-app contract`, Task 6). Tasks 1 to 5 run `cargo clippy --workspace --all-targets` but not `cargo test -p dots-app` (its contract test fails until Task 6).
- How a step changes code: **Create `path`** (tests are named with their assertions; code is given as signatures and what it must do); **In `path`, change …** (find the passage named; if it moved, apply the same change where it is now). Files are LF with a final newline.
- Before each commit: the task's tests, then `cargo test -p dots-core --no-fail-fast`, `cargo clippy --workspace --all-targets` (no new warning), `cargo fmt --check` for a Rust change; from Task 6 also `cargo test -p dots-app`; for a UI change `cd ui && ~/.bun/bin/bun run typecheck` and `cd ui && ~/.bun/bin/bun run test`.
- Known failures, not caused here: `runtime_review_test::preview_schedule_lists_the_next_fire_times` (time zone); under load `approvals_test::*`, `runtime_local_test::live_approval_lets_the_command_run`, `browser_call_test::a_live_approval_on_a_page_that_moved_is_not_performed`, some `resume_test` tests (rerun alone first); clippy's one warning `this 'if' has identical blocks` in `chatgpt.rs`.

## Review Focus

1. **The CLI names only the limiting window, so `seven_day` is never read.** If "never read" forced one-at-a-time, background runs would go one at a time forever. Expected: only a **reset** window (or draining) forces one-at-a-time; a window never read does not. Test: `limits_test::a_window_never_read_does_not_keep_runs_one_at_a_time` (Task 3).
2. **Utilization above 1, or a reading whose `resetsAt` is already past.** Expected: stored as read; the meter shows at most 100%; a reading already past counts as reset. Tests: `limits_test::a_reading_already_reset_counts_as_reset`, `LimitsMeter.test.tsx` "shows at most 100%" (Tasks 3, 7).
3. **The person raises a share or turns holding off while runs are held.** Expected: held runs start at once, one at a time, no restart, and the hold-ended notice fires once. Test: `runtime_limits_test::raising_a_share_releases_held_runs_one_at_a_time` (Task 5).
4. **A dot switched to the local engine while its run is held.** Expected: the run is no longer held and starts. Test: `runtime_limits_test::a_held_run_of_a_dot_switched_to_local_starts` (Task 5).
5. **A `rejected` reading arrives while a chat run is already running.** Expected: that run ends as the CLI says (not re-queued, it is the person's); the next chat is refused with the limit text. Test: `runner_limits_test::a_running_chat_is_not_touched_by_a_rejected_reading` (Task 4).

---

## Context

The runner (`crates/dots-core/src/runner.rs`) starts queued runs in `dispatch_once`, oldest first (`Store::dispatchable_runs`), at most `max_concurrent` and one per dot; `active` holds a run from before `claim_run` to its end; `wake` (a `Notify`) re-runs dispatch. `record()` stores each `EngineEvent` (`Usage` adds tokens via `Store::add_usage`). `execute_inner` reads events until the engine ends, picks the status (`Failed` when the engine sent `Failed`) and calls `Store::finish_run`. The Claude engine's `engine/claude/parse.rs` turns each `stream-json` line into events (`result_events` reads the final `result` line). The fake CLI (`src/bin/fake-claude.rs`) plays scripted steps; a step `{"emit": <json>}` prints that line, which is how tests feed `rate_limit_event` and `total_cost_usd`. The scheduler queues through `store/schedules.rs` `start_or_join`, which today joins the dot's oldest queued run of any trigger. Background services follow the scheduler's pattern (`Arc`, a spawned loop, the `Clock`; tests use `tests/common/clock.rs`'s `FakeClock`). Settings live in `settings.rs` (`#[serde(default)]`, `validate`), saved by `Runtime::update_settings`. Run setters on the runner use a `OnceLock` (`set_acting`).

Task 1 storage and settings; Task 2 parsing; Task 3 the limits service; Task 4 the runner and scheduler; Task 5 the runtime; Task 6 the app and the UI's data layer; Task 7 the rail bars; Task 8 the Usage page; Task 9 Inbox, run numbers, Settings, chat refusal; Task 10 spec as built, smoke, suites.

## Deviations from the spec

Task 10 records these in the spec's section 12.

| Spec | This plan | Why |
|---|---|---|
| §4: after a restart, "while either window is unknown" one at a time | Only a window that **reset** since its last reading (or draining) forces one at a time; a window **never read** does not | If the CLI names only the limiting window, `seven_day` may never be read, and runs would go one at a time forever (Review Focus 1). After a restart, stored readings whose reset passed still force it |
| §5: "Held runs count in Needs you only as one line" | The rail's Inbox count adds 1 when any run is held; per-dot Needs you counts are unchanged | Held runs are not one dot's doing |

## Section 11 decided

- `rate_limit_event`'s shape: Task 2 Step 1 captures one real line and pins it in a fixture; the parser reads `rate_limit_info.status`, `rateLimitType`, `utilization`, `resetsAt`, and `unifiedWindows` when present (each entry its own window). If the real line differs, the parser follows the real line and Task 10 records it.
- Utilization above 1: stored as read; displayed `min(100, round(u*100))`.
- Tray wording and bar details: as in Global Constraints.

---

### Task 1: Storage, run cost and the hold settings

**Files:**
- Create: `crates/dots-core/migrations/0015_usage.sql`, `crates/dots-core/src/store/limits.rs`, `crates/dots-core/tests/limits_store_test.rs`
- Modify: `crates/dots-core/src/store/mod.rs` (one module), `crates/dots-core/src/store/runs.rs` (`RUN_COLUMNS`, `run_from_row`, `add_usage`, `requeue_run`, `set_hold_released`), `crates/dots-core/src/model.rs` (`Run` two fields), `crates/dots-core/src/settings.rs` (three fields, `validate`), `crates/dots-core/src/runner.rs` (`add_usage` call passes `None` until Task 2)

**Interfaces:**
- Produces: `Run { cost_usd: Option<f64>, hold_released: bool, .. }`; `store::limits::LimitReading { window: String, utilization: f64, resets_at: String, status: String, read_at: String }` (`Serialize, Deserialize, Clone, PartialEq, Debug`; Task 3 re-exports it as `limits::LimitReading`); `Store::save_limit_readings(&self, readings: &[LimitReading]) -> Result<()>` (upsert by window); `Store::limit_readings(&self) -> Result<Vec<LimitReading>>` (by window name); `Store::add_usage(&self, id: &str, tokens_in: i64, tokens_out: i64, cost_usd: Option<f64>) -> Result<()>` (a cost added to null becomes the cost, to a value the sum); `Store::set_hold_released(&self, id: &str) -> Result<Run>`; `Store::requeue_run(&self, id: &str) -> Result<Run>` (`running` to `queued`: clears `started_at`, `session_id`, `workspace_path`, `branch`, `base_commit`, `error`, `summary`; keeps trigger and payload; `Error::Conflict` if not running); `Settings { hold_enabled: bool, hold_share_five_hour: u8, hold_share_weekly: u8, .. }`; `settings::SHARE_RULE`.

- [ ] **Step 1: Write the failing tests** in `crates/dots-core/tests/limits_store_test.rs` (the `common` helpers as `watchers_store_test.rs` uses them):
  - `readings_are_kept_per_window_and_replaced`: save `five_hour` 0.42, then 0.64, and `seven_day` 0.55; `limit_readings()` has two rows, `five_hour` at 0.64.
  - `cost_adds_up_and_stays_null_without_one`: `add_usage(id, 10, 2, None)` leaves `cost_usd == None`; then `Some(0.5)` and `Some(0.25)` give `Some(0.75)`; tokens are 30 and 6 after the three calls.
  - `requeue_puts_a_running_run_back_with_its_payload`: a `Watcher` run with payload `{"a":1}`, claimed, `set_session` done; `requeue_run` gives `status == Queued`, `started_at == None`, `session_id == None`, `workspace_path == None`, the same payload and trigger; a second `requeue_run` is `Err(Error::Conflict(_))`.
  - `hold_released_is_stored`: `set_hold_released(id)`; `get_run(id).hold_released == true`.
  - In `settings.rs`'s test module, `hold_shares_default_and_bounds`: defaults `true`, 60, 70; `validate` refuses 5, 105 and 62 with "A share is 10 to 100%, in steps of 5." and accepts 10 and 100; settings JSON without the fields loads the defaults.
- [ ] **Step 2: Run them** — `cargo test -p dots-core --test limits_store_test` and `cargo test -p dots-core --lib settings`. Expected: compile errors (no such fields or methods).
- [ ] **Step 3: Implement** to the interfaces; the migration as in Global Constraints.
- [ ] **Step 4: Run them.** Expected: PASS. Then the suites.
- [ ] **Step 5: Commit** — `git add` the files above; message `feat(core): usage storage, run cost and the hold settings`.

### Task 2: The CLI's limits and cost are read

**Files:**
- Create: `crates/dots-core/tests/fixtures/rate_limit_event.json`, `crates/dots-core/tests/claude_limits_test.rs`
- Modify: `crates/dots-core/src/engine/mod.rs` (`Usage.cost_usd`, a `Limits` variant with `kind() == "limits"`), `crates/dots-core/src/engine/claude/parse.rs`, `crates/dots-core/src/engine/local/mod.rs` and `client.rs` (`cost_usd: None`), `crates/dots-core/src/runner.rs` (`record` passes `cost_usd`)

**Interfaces:**
- Consumes: `LimitReading` (Task 1).
- Produces: `EngineEvent::Usage { tokens_in: u64, tokens_out: u64, cost_usd: Option<f64> }`; `EngineEvent::Limits { status: String, windows: Vec<LimitReading> }` (`read_at` = `util::now()`; `resets_at` from Unix seconds to RFC 3339 UTC).

- [ ] **Step 1: Capture one real line.** In a scratch folder (never the person's data dir), run the person's logged-in CLI once: `claude -p "Reply with ok" --output-format stream-json --verbose > rl.jsonl`; copy the first line whose `type` is `rate_limit_event` into `tests/fixtures/rate_limit_event.json` unchanged, and note in the ledger whether it has `unifiedWindows`. If no such line appears, write the fixture from spec section 3's field names and ledger that the shape is unconfirmed (Task 10 re-checks).
- [ ] **Step 2: Write the failing tests** in `claude_limits_test.rs` (single lines through the parser, as `parse.rs`'s own tests do, and one fake-CLI run with `{"emit": ...}` steps):
  - `the_rate_limit_line_becomes_a_limits_event`: the fixture gives one `Limits` with its `status` and a `five_hour` reading (and `seven_day` if the fixture has it) with its utilization and `resets_at` as RFC 3339.
  - `a_malformed_rate_limit_line_is_ignored`: `{"type":"rate_limit_event","rate_limit_info":"x"}` gives no event and no `Failed`.
  - `the_result_line_carries_the_cost`: a result line with `"total_cost_usd": 1.32` gives `Usage { cost_usd: Some(1.32), .. }`; without it `None`; after a fake-CLI run, `get_run(id).cost_usd == Some(1.32)`.
  - `a_limits_event_is_not_transcript_text`: the run's stored events have one `limits` event and no `assistant_text` or `raw` holding the line.
- [ ] **Step 3: Run them.** Expected: FAIL.
- [ ] **Step 4: Implement**; a line the parser cannot read logs at debug and yields nothing.
- [ ] **Step 5: Run them.** Expected: PASS; then the suites.
- [ ] **Step 6: Commit** — `feat(core): the Claude CLI's limit readings and run cost are read`.

### Task 3: The limits service: readings, the guard and draining

**Files:**
- Create: `crates/dots-core/src/limits.rs`, `crates/dots-core/tests/limits_test.rs`
- Modify: `crates/dots-core/src/lib.rs` (one module), `crates/dots-core/src/events.rs` (one variant)

**Interfaces:**
- Consumes: `Store::save_limit_readings`, `Store::limit_readings`, `SHARE_RULE` (Task 1).
- Produces, in `limits.rs`:
  - `pub use crate::store::limits::LimitReading; pub use crate::settings::SHARE_RULE;` the constants and words of Global Constraints; `pub fn is_background(t: TriggerKind) -> bool`.
  - `#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)] pub struct HoldShares { pub enabled: bool, pub five_hour: u8, pub weekly: u8 }`, `impl From<&Settings>`.
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub struct LimitsState { pub windows: Vec<LimitReading>, pub five_hour: Option<LimitReading>, pub weekly: Option<LimitReading>, pub shares: HoldShares, pub holding: bool, pub draining: bool, pub rejected_until: Option<String>, pub held_run_ids: Vec<String>, pub hold_until: Option<String> }` (`windows` every stored reading; `five_hour`/`weekly` the **known** ones only; `hold_until` the `resets_at` of the window that holds; `held_run_ids` oldest first).
  - `#[serde(tag = "kind", rename_all = "snake_case")] pub enum HoldChange { Started { window: String, utilization: f64 }, Ended }`.
  - `pub struct QueuedRun { pub id: String, pub trigger: TriggerKind, pub engine: EngineKind, pub hold_released: bool, pub queued_at: String }`; `pub enum Admit { Go, Held, Rejected }`.
  - `pub struct Limits`; `pub async fn load(store: Store, bus: Bus, clock: Arc<dyn Clock>, shares: HoldShares) -> Result<Arc<Self>>`; `pub async fn record(&self, status: &str, readings: Vec<LimitReading>) -> Result<()>` (saves; `rejected` sets `rejected_until` to the limiting window's `resets_at`); `pub fn set_shares(&self, shares: HoldShares)`; `pub fn admit(&self, run: &QueuedRun, running_background_claude: usize) -> Admit`; `pub fn state(&self, queued: &[QueuedRun]) -> LimitsState`; `pub fn publish(&self, queued: &[QueuedRun])` (sends `LimitsUpdated`, with `change` when `holding` flipped since the last publish; `Started → Ended` while background Claude runs are queued sets `draining`; no background Claude run queued clears it); `pub fn rejected_until(&self) -> Option<String>`; `pub fn next_change(&self) -> Option<DateTime<Utc>>` (the earliest future `resets_at` of a hold window or of `rejected`).
- `admit` (spec §4): `engine != Claude` → `Go`. `rejected_until` in the future → `Rejected`, whatever the trigger and `hold_released`. Not background → `Go`. `hold_released` → `Go`. `shares.enabled` and a **known** hold window at or above its share → `Held`. Otherwise `Held` when `running_background_claude >= 1` and (`draining` or a hold window is **reset**), else `Go`. `holding` in the state is true when any queued run gets `Held`.
- `events.rs`: `RuntimeEvent::LimitsUpdated { limits: LimitsState, change: Option<HoldChange> }`.

- [ ] **Step 1: Write the failing tests** in `limits_test.rs` (`FakeClock`, an in-memory store, `QueuedRun`s by hand):
  - `local_runs_are_never_held`; `the_persons_runs_are_never_held` (chat, manual, resume at 0.95 → `Go`);
  - `background_runs_are_held_at_the_share` (five_hour 0.60 → schedule `Held`; 0.59 → `Go`; weekly 0.70 → `Held`, 0.69 → `Go`);
  - `the_switch_off_holds_nothing`; `run_anyway_goes` (`hold_released` at 0.9 → `Go`);
  - `rejected_stops_every_claude_run_until_it_resets` (chat → `Rejected`, a released run → `Rejected`; with the clock past `resets_at` → `Go`);
  - `a_reading_already_reset_counts_as_reset` (0.9, `resets_at` 1 min ago: `Go` with no background run running, `Held` with one);
  - `a_window_never_read_does_not_keep_runs_one_at_a_time` (only `five_hour` at 0.1; one background running → `Go`);
  - `draining_lets_one_at_a_time_until_the_queue_is_empty` (held at 0.7; a 0.2 reading; `publish` gives `Ended`; with one running → `Held`, with none → `Go`; after `publish(&[])` `draining` is false);
  - `holding_start_and_end_are_published_once` (two readings above the share give one `Started { window: "five_hour", utilization: 0.64 }`; a reading below gives one `Ended`);
  - `readings_survive_a_reload` (a new `Limits::load` on the same store has the reading).
- [ ] **Step 2: Run** `cargo test -p dots-core --test limits_test`. Expected: compile errors.
- [ ] **Step 3: Implement** to the interfaces; state behind a `std::sync::Mutex`.
- [ ] **Step 4: Run them.** Expected: PASS; then the suites.
- [ ] **Step 5: Commit** — `feat(core): the limits service decides which background runs wait`.

### Task 4: The runner holds, drains and puts back; schedules join schedule runs only

**Files:**
- Create: `crates/dots-core/tests/runner_limits_test.rs`
- Modify: `crates/dots-core/src/runner.rs` (`set_limits`, `nudge`, `dispatch_once`, `record`, `execute_inner`'s end), `crates/dots-core/src/store/runs.rs` (`dispatchable_runs` returns each run's dot engine too, as `Vec<(Run, EngineKind)>`, callers updated), `crates/dots-core/src/store/schedules.rs` (`start_or_join`), the scheduler tests that pinned joining any queued run

**Interfaces:**
- Consumes: `Limits::{admit, record, publish}`, `QueuedRun`, `Admit`, `STOPPED_BY_LIMIT` (Task 3); `Store::requeue_run` (Task 1).
- Produces: `Runner::set_limits(&self, limits: Arc<Limits>)` (a `OnceLock`; without it nothing is held); `Runner::nudge(&self)` (wakes the dispatcher); `Runner::queued_runs(&self) -> Result<Vec<QueuedRun>>` (for `publish`); `Active` gains `background_claude: bool`.

- [ ] **Step 1: Write the failing tests** in `runner_limits_test.rs` (fake CLI, the `common` harness as `runtime_watchers_test.rs`):
  - `held_runs_stay_queued_while_others_start` (five_hour at 0.7: a schedule run stays `queued`; a manual run of another dot starts);
  - `held_runs_start_one_at_a_time_oldest_first` (three watcher runs of three dots held, `max_concurrent` 2; a 0.1 reading recorded; only the oldest runs; when it ends the next starts);
  - `a_run_that_ends_without_a_reading_lets_the_next_start` (as above, the fake CLI emits no `rate_limit_event`);
  - `a_background_run_rejected_before_a_tool_call_is_queued_again` (the fake CLI emits a `rejected` line then a failed result: the run is `queued` with its payload; `rejected_until()` is set; it does not start until the clock passes it);
  - `a_background_run_rejected_after_a_tool_call_fails_with_the_note` (a tool call first: `failed`, error starting "stopped by Claude's usage limit");
  - `a_running_chat_is_not_touched_by_a_rejected_reading` (Review Focus 5: ends `failed` with the CLI's text, not re-queued);
  - scheduler tests: `a_schedule_slot_does_not_join_a_queued_watcher_run` (two runs queued) and `a_schedule_slot_joins_a_queued_schedule_run` (one).
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.** `dispatch_once`: claim only on `Admit::Go`. `record`: `Limits` → `limits.record`, `publish(queued_runs)`, `nudge`. `execute_inner`: track `saw_tool_call` and `saw_rejected` (a `Limits` event with status `rejected`); background, failed, `saw_rejected`, no tool call → workspace cleanup as for a failure, `requeue_run`, emit the run, no `finish_run`; background, `saw_rejected` after a tool call → `Failed` with `"{STOPPED_BY_LIMIT}: {cli error}"`. At every run end, `publish` and `nudge`. `start_or_join`: add `AND trigger_kind = 'schedule'`.
- [ ] **Step 4: Run them**, then the suites. Expected: PASS (each scheduler test changed for the narrower join is named in the ledger).
- [ ] **Step 5: Commit** — `feat(core): the runner holds background Claude runs and starts them one at a time after the reset`.

### Task 5: The runtime: refusal under the limit, Run anyway, the held list and the usage summary

**Files:**
- Create: `crates/dots-core/tests/runtime_limits_test.rs`
- Modify: `crates/dots-core/src/runtime.rs` (load `Limits` at start, `runner.set_limits`, a task that sleeps until `next_change()` then publishes and nudges, woken by each `LimitsUpdated`; `set_shares` and a nudge in `update_settings`; `run_now` and `chat`; `inbox`; one `impl` block appended), `crates/dots-core/src/model.rs` (`Inbox.held`, the usage types), `crates/dots-core/src/store/runs.rs` (the summary query)

**Interfaces:**
- Produces: `Runtime::limits_get(&self) -> Result<LimitsState>`; `Runtime::usage_summary(&self, period: UsagePeriod) -> Result<UsageSummary>`; `Runtime::run_release_hold(&self, run_id: &str) -> Result<Run>` (`Error::Invalid(NOT_HELD)` unless `queued`, background, Claude and not released; then publishes and nudges); `Inbox.held: Vec<HeldRun>`; `str_enum!(UsagePeriod { FiveHours => "five_hours", Week => "week", Month => "month" })`; `UsageSummary { period: UsagePeriod, since: String, rows: Vec<UsageRow> }`; `UsageRow { dot_id: String, dot_name: String, engine: EngineKind, runs: u32, tokens_in: i64, tokens_out: i64, run_secs: i64, cost_usd: Option<f64>, heaviest: Vec<UsageRun> }` (by `tokens_in + tokens_out` descending, then name); `UsageRun { run_id: String, trigger: TriggerKind, started_at: String, tokens_in: i64, tokens_out: i64, run_secs: Option<i64>, cost_usd: Option<f64> }` (the dot's `HEAVIEST` by tokens); `HeldRun { run_id: String, dot_id: String, dot_name: String, trigger: TriggerKind, queued_at: String, starts_after: Option<String> }` (`starts_after` = `hold_until`; `None` while draining). `run_secs` computed in Rust from `started_at`/`ended_at`; a run without `ended_at` adds nothing.

- [ ] **Step 1: Write the failing tests** in `runtime_limits_test.rs`:
  - `chat_and_run_now_are_refused_under_the_limit` (a `rejected` reading resetting at 15:10 local today: both `Err(Error::Invalid(m))` with `m == "Claude's limit is reached until 15:10."`; no run is created; a local dot's chat goes through);
  - `run_anyway_starts_one_held_run` (two held schedule runs; `run_release_hold(first)` starts it, the other stays queued; on a chat run `Invalid("This run is not held.")`);
  - `the_inbox_lists_held_runs_with_their_reset` (`starts_after` is the `five_hour` `resets_at`);
  - `raising_a_share_releases_held_runs_one_at_a_time` (Review Focus 3: held at 0.62 with share 60; `update_settings` to 65; one starts, the next after it ends; one `Ended` published);
  - `a_held_run_of_a_dot_switched_to_local_starts` (Review Focus 4);
  - `a_reset_passing_releases_without_a_new_reading` (`FakeClock` past `resets_at`: the first held run starts with no event recorded);
  - `the_usage_summary_sums_per_dot_in_the_period` (runs started now−1 h, now−3 days, now−20 days: `five_hours` counts one, `week` two, `month` three; tokens, `run_secs` and cost summed; a local dot's `cost_usd` is `None`; `heaviest` ordered, at most 5).
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** to the interfaces.
- [ ] **Step 4: Run them**, then the suites. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(core): Run anyway, held runs in the Inbox, the usage summary and the limit's refusal`.

### Task 6: The app's commands, tray and notices; the contract; the UI's data layer

**Files:**
- Modify: `apps/dots-app/src/commands.rs` (three commands, appended), `apps/dots-app/src/lib.rs` (three names), `apps/dots-app/src/contract.rs` (samples: `LimitsState` holding and not, both `HoldChange`s, `UsageSummary`, `HeldRun`, `Run` with `cost_usd`, the event), `apps/dots-app/src/events.rs` (a notice on `change`), `apps/dots-app/src/tray.rs` (`tooltip(count: usize, limits: Option<&LimitsState>) -> String`, refreshed on `LimitsUpdated`), `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/api/contract.test.ts`, `ui/src/test/fixtures.ts`, `ui/src/lib/query.ts`, `ui/src/lib/runtime-events.ts`, `ui/src/api/__fixtures__/contract.json` (regenerated)

**Interfaces:**
- Produces (UI): strict zod `LimitReading`, `HoldShares`, `LimitsState`, `HoldChange`, `UsagePeriod`, `UsageSummary`, `UsageRow`, `UsageRun`, `HeldRun`; `Run.cost_usd: number | null`, `Run.hold_released: boolean`; `Inbox.held`; `commands.limitsGet()`, `commands.usageSummary(period)`, `commands.runReleaseHold(runId)`; `keys.limits`, `keys.usage(period)`; on `limits_updated` `setQueryData(keys.limits, e.limits)` and invalidate `keys.inbox` and `["usage"]`; fixtures `sample.limits` (holding) and `sample.usage`.
- Tray: the lines of Global Constraints joined with `\n`; the first line unchanged.

- [ ] **Step 1: Write the failing tests:** app `tray::tests::tooltip_shows_the_limits_and_holding` ("Dots: 2 to review\nClaude 5h 42% · week 55%"; holding adds "\nHolding 3 runs until 15:10"; an unknown window reads "5h –"); app events test `a_hold_start_and_end_notify_once_each` (texts per Global Constraints, `/usage` as the target); UI `runtime-events.test.ts` "limits_updated writes the limits"; `commands.test.ts` for the three commands' arguments.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement**, then `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- [ ] **Step 4: Run** `cargo test -p dots-app`, the UI typecheck and tests. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(app): limits commands, the tray's usage lines and the hold notices`.

### Task 7: The rail bars

**Files:**
- Create: `ui/src/components/LimitsMeter.tsx`, `ui/src/components/LimitsMeter.test.tsx`, `ui/src/lib/usage-format.ts`, `ui/src/lib/usage-format.test.ts`, `ui/src/screens/UsageScreen.tsx` (a `Page` titled **Usage**, filled in Task 8)
- Modify: `ui/src/app/Layout.tsx` (the meter at the rail's bottom, `mt-auto`, after the links), `ui/src/app/routes.tsx` (`{ path: "usage", element: <UsageScreen /> }`)

**Interfaces:**
- Produces: `formatTokens(n: number): string`, `formatDuration(secs: number): string`, `formatCost(usd: number): string`, `runSeconds(run: Pick<Run, "started_at" | "ended_at">): number | null`, `pct(u: number): number` (`min(100, round(u*100))`), `resetTime(iso: string): string` (`HH:MM` today, else `ddd HH:MM`); `<LimitsMeter />`.

- [ ] **Step 1: Write the failing tests:** `usage-format.test.ts` (950 → "950", 12_400 → "12k", 182_000 → "182k", 2_100_000 → "2.1M"; 45 → "45s", 724 → "12m 04s", 11_520 → "3h 12m"; 1.3249 → "$1.32"; `pct(1.4)` → 100). `LimitsMeter.test.tsx`: two bars whose fill follows the readings; the link's accessible name is the Global Constraints sentence; while holding both bars carry `data-holding` and the name says "holding 3 runs"; an unknown window renders a dashed empty bar and reads "unknown"; a reading 25 minutes old adds "read 25 min ago" to the title; "shows at most 100%"; clicking goes to `/usage`; Tab reaches it.
- [ ] **Step 2: Run** `cd ui && ~/.bun/bin/bun run test -- LimitsMeter usage-format`. Expected: FAIL.
- [ ] **Step 3: Implement.** One `Link` to `/usage` around both bars, as wide as the rail's links, with `title` and `aria-label`.
- [ ] **Step 4: Run them**, then typecheck and the UI suite. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(ui): the rail's Claude usage bars`.

### Task 8: The Usage page

**Files:**
- Create: `ui/src/screens/UsageScreen.test.tsx`
- Modify: `ui/src/screens/UsageScreen.tsx`

- [ ] **Step 1: Write the failing tests** in `UsageScreen.test.tsx` (fake backend, `sample.usage`):
  - opens on **5 hours** and asks `usage_summary` with `five_hours`; **Week** and **Month** ask `week` and `month`, and the caption follows;
  - rows in the order the core sent, with formatted columns; a local dot's API-eq. cell is "–";
  - a row's button (`aria-expanded`) shows its heaviest runs, each a link to `/runs/<id>`;
  - the top shows both windows with their shares and "Holding 2 runs" linking to `/inbox`;
  - the deleted-dots note shows; an empty period says "No runs in the last 5 hours.";
  - the table has a caption and column headers.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement** with `Page`, the tabs pattern the editor uses for the period, a `<table>`.
- [ ] **Step 4: Run them**, then typecheck and the UI suite. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(ui): the Usage page`.

### Task 9: Held runs in the Inbox, run numbers, Settings, the chat refusal

**Files:**
- Create: `ui/src/screens/InboxHeld.test.tsx`, `ui/src/components/RunNumbers.tsx`, `ui/src/components/RunNumbers.test.tsx`, `ui/src/screens/SettingsUsage.test.tsx`
- Modify: `ui/src/screens/InboxScreen.tsx` (section), `ui/src/app/Layout.tsx` (the Inbox count adds 1 when `inbox.held` is not empty), `ui/src/screens/home/RunCard.tsx` and `ui/src/screens/RunScreen.tsx` (`<RunNumbers />`; on a held run the held line and **Run anyway**), `ui/src/screens/SettingsScreen.tsx` (section)

- [ ] **Step 1: Write the failing tests:**
  - `InboxHeld.test.tsx`: the section lists each held run (dot, trigger label, "starts after 15:10" or "waiting for the first run after the reset"); **Run anyway** calls `run_release_hold` with the id; the rail's Inbox count counts the held list as one.
  - `RunNumbers.test.tsx`: the run numbers sentence for an ended Claude run; no cost part without `cost_usd`; no duration part while running; the "API-eq." tooltip text; a held run's card shows the held line and **Run anyway**.
  - `SettingsUsage.test.tsx`: the switch and both share fields with their defaults; 62 shows the core's "A share is 10 to 100%, in steps of 5."; saving sends the three fields.
  - In the home tests: a chat refused under the limit shows "Claude's limit is reached until 15:10." where the composer shows its errors.
- [ ] **Step 2: Run them.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them**, then typecheck and the UI suite. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(ui): held runs in the Inbox, run numbers and the Claude usage settings`.

### Task 10: The spec as built, the smoke run, and the whole suites

**Files:**
- Modify: `docs/superpowers/specs/2026-10-26-usage-limits-design.md` (section 12 "As built": the deviations table above, section 11's choices, the smoke record); a smoke note under `docs/verification/` as earlier phases

- [ ] **Step 1: Whole suites:** `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, UI typecheck and tests. Expected: only the known failures.
- [ ] **Step 2: Smoke** in a fresh scratch `DOTS_DATA_DIR` (never the person's real Edge profile): one real Claude run gives a reading in the rail bars and the tray; set the 5-hour share to 10%; a schedule due now is held and listed in the Inbox; **Run anyway** starts it; raise the share to 100%: the next held run starts and "Held runs are starting" shows once; the Usage page lists the dot. Record whether `unifiedWindows` came. Then axe (light and dark) on Home, `/usage`, the Inbox and Settings, keyboard through the rail bars and the period switch, and the 420 px width with no sideways scroll (spec section 10).
- [ ] **Step 3: Write section 12** and the smoke note.
- [ ] **Step 4: Commit** — `docs: usage and limits as built, and the smoke run`.
