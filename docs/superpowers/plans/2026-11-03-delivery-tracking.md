# Delivery tracking — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every run says whether it reached its dot: delivered (the engine started on it), waiting (and why), or not delivered. A chat message that never got there offers Retry and Discard, counts in Needs you and notifies once. Each dot's header menu shows today's counts.

**Architecture:** Four new `runs` columns (migration 0030). `Run` gains a computed `delivery` and a `wait_reason` that is filled when runs are read for the UI. The store writes the not-delivered mark in the same statement that ends a failed run. The runner marks a run delivered right after `engine.start` returns `Ok`. A small `delivery` module holds the wait-reason rule, the local-day bounds and a 30-second sweeper on the scheduler's `Clock`. The runtime adds chat-only Retry and Discard, plus today's counts. The app adds three commands and one notice. The UI shows one delivery line per run, and a counts line in the dot menu.

**Tech Stack:** Rust (tokio, sqlx/SQLite, chrono), Tauri 2, React + TypeScript (zod, vitest, TanStack Query).

**Spec:** `docs/superpowers/specs/2026-11-03-delivery-tracking-design.md` (§3.9 overrides §4.1 where they differ).

## Global Constraints

- Delivered = `engine.start(ctx)` returned `Ok` in the runner. Recorded once in `runs.delivered_at` and never cleared, not even by a usage-hold `requeue_run`.
- Overdue (UI only) at **5 minutes** since `queued_at`. Not delivered at **30 minutes** of waiting (sweeper), never while the wait reason is `usage_hold`. The run stays queued, and if it starts later it becomes delivered and the mark clears.
- Not delivered at the end only when the run ends **`failed`** with no `delivered_at`. The reason is the first line of its error (`interrupted` for `recover_interrupted`). A **cancel** never marks a run not delivered (§3.9.1).
- Migration `0030_delivery.sql`. Use the four `ALTER TABLE` lines from spec §4.1 verbatim, then the backfill `UPDATE runs SET delivered_at = started_at WHERE started_at IS NOT NULL;` and then `UPDATE runs SET dismissed_at = ended_at WHERE ended_at IS NOT NULL AND delivered_at IS NULL;`.
- `delivery` values: `delivered` | `waiting` | `not_delivered`. Wait reasons: `busy` | `usage_hold` | `updating` | `dot_busy` | `queued`.
- Wait-reason precedence: `usage_hold`, then `updating`, then `dot_busy` (another run of the same dot is running or awaiting approval — the dispatcher starts one run per dot), then `busy` (the store's running count ≥ `max_concurrent`), then `queued`.
- Reason texts (UI line and the sweeper's `undelivered_reason`, verbatim): `busy` → `other runs are busy`, `usage_hold` → `held for the usage limit`, `updating` → `Dots is updating`, `dot_busy` → `this dot is busy with another run`, `queued` → `next in line`.
- UI copy: `Waiting <n> min · <reason text>` (muted, n = whole minutes since `queued_at`, shown from n ≥ 5); `Not delivered: <reason>` (warning colour) with buttons `Retry` and `Discard`; dismissed: muted `Not delivered`, no buttons; menu line `Today: <s> sent · <d> delivered · <u> not delivered`.
- Notice: title `<dot> didn't get your message`, body = the reason's first line, route `/chat/<dot>/<run>`. Chat runs only. Once per run. Never for the dot being viewed.
- Retry and Discard are for `chat` runs only (`Conflict` for other triggers). Both return `Conflict` unless the run has `undelivered_at`, no `delivered_at` and no `dismissed_at`. `NotFound` for an unknown run.
- Counts are for the local day of `queued_at`, over rows with `trigger_kind <> 'resume'`: sent = all such rows, delivered = those with `delivered_at`, not_delivered = those with `undelivered_at` and no `delivered_at` (dismissed ones included).
- Needs you also counts chat runs with `undelivered_at` set, `dismissed_at` null and `delivered_at` null.
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH. No AI trailer in commits.
- Tests must not depend on same-millisecond row order (set `queued_at` explicitly with the hidden helper, or assert by id). They must not depend on a dispatcher claiming queued runs: call `rt.hold_runs(true)` before creating runs, or build a `Runner` without `spawn_dispatcher`. They must not depend on exact timing, locale or time zone: use `FakeClock`, `TestTz`, and relative timestamps in UI tests.

## Review Focus

1. **Retrying a reply in the middle of a conversation**: today `chat` refuses any parent that is not the thread's last run. The dismissed run would still be last, so Retry would always fail with "newer message". After Retry, the new run's `parent_run_id` must equal the old run's, and a later typed message must continue from the new run. Test `retrying_a_reply_continues_the_same_thread` in Task 3, and the UI `lastRun` test in Task 5.
2. **A message stuck behind an update hold for 31 minutes, then released**: the sweeper marks it with `Dots is updating` and the person is notified once. When it starts, `delivered_at` is set, the mark clears, it leaves Needs you, and no second notice is sent. Tests `the_sweeper_marks_a_message_waiting_30_minutes` in Task 2, `delivery_clears_an_earlier_mark` in Task 1, and `a_chat_run_marked_twice_notifies_once` in Task 4.
3. **Dots restarted while a chat run was being prepared (claimed, engine not started)**: `recover_interrupted` marks it not delivered (`interrupted`) and it shows in Needs you. A run that was delivered and then interrupted does not. Test `an_interrupted_run_is_not_delivered_only_if_it_never_started` in Task 1.
4. **Retry pressed twice, or a Retry that the chat path refuses**: at most one new run is created. A refused Retry leaves the old message not delivered and not dismissed, so the person can try again. Tests `retrying_twice_sends_once` and `a_refused_retry_leaves_the_message_retryable` in Task 3.
5. **A background run held for the usage limit for hours, then requeued by Claude's `rejected`**: it is never marked by waiting. After the requeue it keeps `delivered_at`, and it counts as one send, not two. Tests `the_sweeper_never_marks_a_usage_hold` in Task 2 and `a_requeue_is_not_a_second_send` in Task 1.

---

### Task 1: Storage — columns, delivery, end-of-run marks, counts, Needs you

**Files:**
- Create: `crates/dots-core/migrations/0030_delivery.sql` (Global Constraints)
- Modify: `crates/dots-core/src/model.rs` (`Run` :237; new `Delivery`, `DeliveryCounts`)
- Modify: `crates/dots-core/src/store/runs.rs` (`RUN_COLUMNS` :11, `run_from_row` :17, `finish_run_with` :612, `fail_if_running` :636, `cancel_if_inactive` :654, `recover_interrupted` :683; new functions next to `set_run_times` :494)
- Modify: `crates/dots-core/src/store/dots.rs` (`dot_counts` :338, one more `+ (SELECT COUNT(*) ...)` term in `needs_you`)
- Modify: every `Run { .. }` literal: `crates/dots-core/tests/common/mod.rs` `fake_run` :84, `apps/dots-app/src/contract.rs` `fn run()` :530, `apps/dots-app/src/events.rs` test `fn run()` :396
- Test: `crates/dots-core/tests/store_delivery_test.rs` (new)

**Interfaces:**
- Produces:
  - `str_enum!(Delivery { Delivered => "delivered", Waiting => "waiting", NotDelivered => "not_delivered" })`, plus `Delivery::of(status: RunStatus, delivered_at: Option<&str>, undelivered_at: Option<&str>) -> Option<Delivery>`. The rule: `delivered_at` → `Delivered`; else `undelivered_at` → `NotDelivered`; else not terminal → `Waiting`; else `None` (ended without either mark: cancelled before start, or history from before 0030).
  - New `Run` fields: `delivered_at`, `undelivered_at`, `undelivered_reason`, `dismissed_at` (all `Option<String>`, `#[serde(default)]`); `delivery: Option<Delivery>` (computed in `run_from_row`, `#[serde(default)]`); `wait_reason: Option<WaitReason>` (`#[serde(default)]`; the store always leaves it `None`, Task 2 fills it). Declare `WaitReason` in this task as `str_enum!(WaitReason { Busy => "busy", UsageHold => "usage_hold", Updating => "updating", DotBusy => "dot_busy", Queued => "queued" })` so every `Run` literal changes only once.
  - `pub struct DeliveryCounts { pub sent: u32, pub delivered: u32, pub not_delivered: u32 }` (Serialize, Deserialize, Clone, Debug, PartialEq, Eq).
  - `Store::mark_delivered(&self, id: &str, at: DateTime<Utc>) -> Result<Option<Run>>`: sets `delivered_at` only where it is null, and clears `undelivered_at`/`undelivered_reason` in the same statement. Returns `None` when it was already delivered (nothing changed).
  - `Store::mark_undelivered(&self, id: &str, reason: &str, at: DateTime<Utc>) -> Result<Option<Run>>`: only where `delivered_at`, `undelivered_at` and `dismissed_at` are all null. Returns `None` otherwise.
  - `Store::dismiss_run(&self, id: &str, at: DateTime<Utc>) -> Result<Option<Run>>`: only where `dismissed_at IS NULL`. Returns `None` otherwise (the guard against a double Retry).
  - `Store::undismiss_run(&self, id: &str) -> Result<Run>`: clears `dismissed_at` (used by a refused Retry, Task 3).
  - `Store::overdue_undelivered(&self, queued_before: &str) -> Result<Vec<Run>>`: status `queued`, all of `delivered_at`/`undelivered_at`/`dismissed_at` null, `queued_at <= ?`, ordered `queued_at, id`.
  - `Store::delivery_counts(&self, dot_id: &str, day_start: &str, day_end: &str) -> Result<DeliveryCounts>`: half-open interval `[day_start, day_end)` on `queued_at`, using the Global Constraints rule.
  - `#[doc(hidden)] Store::set_run_queued_at(&self, id: &str, at: &str) -> Result<()>` (for tests, like `set_run_times`).
  - End-of-run writes (same statement, no second query). `finish_run_with`: when `status == Failed` and `delivered_at IS NULL`, set `undelivered_at = COALESCE(undelivered_at, now)` and `undelivered_reason` = the first non-empty line of the scrubbed error (bound from Rust; if there is no error, `failed`). `fail_if_running`: the same with its `error`. `recover_interrupted`: the same with `interrupted`. Cancel paths (`finish_run_with` with `Cancelled`, `cancel_if_inactive`) never set `undelivered_at`. When `undelivered_at` is already set and the person had not dismissed it, they set `dismissed_at = now` (ruling: a cancelled message leaves Needs you).

- [ ] **Step 1: Write the failing tests** in `store_delivery_test.rs`. Use the store only, with no runner. Create runs with `create_run`, move them with `claim_run` / `finish_run` / `fail_if_running`, and give each run an explicit `set_run_queued_at`.
  - `a_new_run_is_waiting`: `delivery == Some(Waiting)`, all four new fields `None`, `wait_reason == None`.
  - `mark_delivered_sets_once`: the first call returns `Some`, `delivery == Some(Delivered)`; the second call returns `None` and `delivered_at` is unchanged.
  - `delivery_clears_an_earlier_mark` (Review Focus 2): `mark_undelivered(id, "Dots is updating", t)` gives `NotDelivered` with reason `Dots is updating`; then `mark_delivered` gives `Delivered`, `undelivered_at == None`, `undelivered_reason == None`.
  - `mark_undelivered_skips_delivered_and_dismissed_runs`: both cases return `None`.
  - `a_failed_run_never_delivered_is_marked_in_the_same_write`: claim, then `finish_run(Failed, None, Some("claude CLI not found\nsecond line"))` gives `undelivered_reason == Some("claude CLI not found")` and `undelivered_at` set. A delivered run that fails stays `Delivered`. `fail_if_running(id, "internal error: engine task panicked")` gives that reason.
  - `a_cancelled_run_is_not_marked`: cancel a queued run with `cancel_if_inactive`, and a claimed one with `finish_run(Cancelled, ..)`. Both have `undelivered_at == None` and `delivery == None`. A run marked first and then cancelled keeps its mark and gets `dismissed_at` set.
  - `an_interrupted_run_is_not_delivered_only_if_it_never_started` (Review Focus 3): two claimed runs, one with `mark_delivered`. `recover_interrupted` gives the undelivered one reason `interrupted`; the delivered one stays `Delivered`.
  - `dismiss_run_happens_once` and `undismiss_run_clears_it`.
  - `overdue_undelivered_lists_only_queued_unmarked_runs_queued_before_the_cut`.
  - `needs_you_counts_undelivered_chat_runs_only`: a failed undelivered chat run raises `dot_counts(dot).0` by 1. A watcher run that failed undelivered does not. After `dismiss_run` it drops back. A run marked and then delivered does not count.
  - `delivery_counts_cover_the_day_and_leave_out_resumes`, for the day `[2026-11-03T00:00:00.000Z, 2026-11-04T00:00:00.000Z)`: two chat runs (one delivered, one marked and dismissed), one schedule run queued at `2026-11-03T23:59:59.999Z`, one run at `2026-11-04T00:00:00.000Z` (outside), and one `resume` run (left out). Result: `{ sent: 3, delivered: 1, not_delivered: 1 }`.
  - `a_requeue_is_not_a_second_send` (Review Focus 5): claim, `mark_delivered`, `requeue_run`. Then `delivered_at` is still set, `delivery == Some(Delivered)`, and `sent == 1`.
  - `migration_0030_backfills_history`: migrate to versions `< 30` (pattern `store_quiet_test.rs:49`). Insert one ended run with `started_at`, and one ended run without it. Open the `Store`. The first has `delivered_at == started_at` and `dismissed_at == None`. The second has `delivered_at == None`, `dismissed_at == ended_at` and `delivery == None`. `dot_counts(..).0 == 0`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test store_delivery_test`. Expected: compile errors (no columns, no fields, no functions).
- [ ] **Step 3: Implement.** Add the migration, the model types, the store functions and the extra `needs_you` term. Add the new columns to `RUN_COLUMNS` and fill the `Run` fields in `run_from_row`. Update the three `Run` literals: `delivered_at` set and `delivery: Some(Delivered)` in the contract sample, everything else `None`.
- [ ] **Step 4: Run** `cargo test -p dots-core --test store_delivery_test --test store_runs_test --test store_quiet_test --test runtime_seen_test --test store_dots_test` and `cargo build -p dots-app --tests`. Expected: PASS / compiles.
- [ ] **Step 5: Commit** `feat(core): a run records whether it reached its dot`.

### Task 2: Runner, wait reasons and the 30-minute sweeper

**Files:**
- Create: `crates/dots-core/src/delivery.rs` (register in `lib.rs`)
- Modify: `crates/dots-core/src/runner.rs`: after `engine.start(ctx).await?` (:1356), before the deadline is set; new `pub` methods next to `set_held` (:604) and `running_background_claude` (:304)
- Modify: `crates/dots-core/src/store/runs.rs` (new `running_count`)
- Modify: `crates/dots-core/src/runtime.rs`: `list_runs` :672, `get_run` :676, `run_thread` :681 fill `wait_reason`; `start` spawns the sweeper after `runner.spawn_dispatcher` (:390); new `Runtime::wait_reason`
- Test: `crates/dots-core/tests/runner_delivery_test.rs` (new; a `Runner` with no dispatcher for the wait reasons, and with one for delivery), `crates/dots-core/tests/delivery_sweeper_test.rs` (new; `FakeClock`)

**Interfaces:**
- Consumes: Task 1 (`mark_delivered`, `mark_undelivered`, `overdue_undelivered`, `set_run_queued_at`, `WaitReason`).
- Produces:
  - `impl WaitReason { pub fn text(self) -> &'static str }`, which returns the Global Constraints texts.
  - `pub struct WaitFacts { pub usage_held: bool, pub runner_held: bool, pub dot_has_active_run: bool, pub running: usize, pub max_concurrent: usize }` and `pub fn wait_reason(f: &WaitFacts) -> WaitReason` (in `delivery.rs`, the precedence from Global Constraints).
  - `Store::running_count(&self) -> Result<usize>` (`status = 'running'`).
  - `Runner::is_held(&self) -> bool`; `Runner::max_concurrent(&self) -> usize`; `Runner::usage_held_ids(&self) -> Result<HashSet<String>>`. The last one is the `held_run_ids` of `limits.state(&self.queued_runs().await?, self.running_background_claude())`, and empty without limits.
  - `Runner::fill_wait_reasons(&self, runs: &mut [Run]) -> Result<()>`: sets `wait_reason` for each `queued` run whose `delivery == Some(Waiting)`, and `None` for every other run. It reads the facts once per call (held ids, running count, each dot's `enabled` with a small cache).
  - `Runtime::wait_reason(&self, run: &Run) -> Result<Option<WaitReason>>` (delegates to the runner).
  - In `execute_inner`, right after `engine.start` returns `Ok`: `if let Some(row) = store.mark_delivered(&run.id, Utc::now()).await? { self.emit_run(&row) }`. A store error here propagates like the `set_run_quiet_limit` write above it.
  - `pub struct DeliverySweeper { store: Store, runner: Arc<Runner>, bus: Bus, clock: Arc<dyn Clock> }`, with `new(..)`, `pub async fn sweep(&self) -> Result<Vec<Run>>` and `pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()>`. `sweep`: cut = `timestamp(clock.now() - 30 min)`; for each `overdue_undelivered(cut)` run, compute its reason with `fill_wait_reasons`, skip `UsageHold`, call `mark_undelivered(id, reason.text(), clock.now())`, and send `RunUpdated` for each row returned. Loop: `select! { shutdown, clock.sleep_until(clock.now() + 30 s) }` and then `sweep`. Errors are logged and the loop goes on.

- [ ] **Step 1: Write the failing tests.**
  - `wait_reason_follows_its_precedence`: a table over `WaitFacts`. All false/low gives `Queued`. `running == max` gives `Busy`. `dot_has_active_run` together with `running == max` gives `DotBusy`. `runner_held` with everything else gives `Updating`. `usage_held` with everything else gives `UsageHold`. The `text()` of each equals the Global Constraints copy.
  - Runner with no dispatcher, one queued chat run each, checked with `fill_wait_reasons`. `next_in_line` → `Queued`. `the_runner_held` (`set_held(true)`) → `Updating`. `the_dot_busy`: another run of the SAME dot moved to running with `store.claim_run` (max_concurrent high) → `DotBusy`. `busy`: `max_concurrent = 1`, a run of ANOTHER dot moved to running → `Busy`. A disabled dot alone gives `Queued` (no `dot_off` reason). `usage_hold`: `Limits` with the `runner_limits_test.rs` shares, `record` 0.7, a `Schedule` run → `UsageHold`. A running run, and a delivered requeued run, have `wait_reason == None`.
  - Runner with a dispatcher, `ScriptedEngine` finishing. `a_run_is_delivered_when_its_engine_starts`: `delivered_at` is set before the run ends, and a `RunUpdated` with `delivery == Some(Delivered)` and `status == Running` is seen on the bus. `an_engine_that_cannot_start_leaves_it_undelivered`: an engine whose `start` returns `Err(Error::Other("no start"))` (not `unavailable`) gives `Failed`, `undelivered_reason == Some("no start")`, `delivered_at == None`. `an_unavailable_engine_marks_it_too`: `UnavailableEngine("claude CLI not found: install Claude Code")` gives that reason.
  - `a_resumed_run_keeps_delivered_at`: a run delivered and requeued (store `requeue_run`, as the usage hold does) that starts again keeps its first `delivered_at`.
  - Sweeper (`FakeClock::new(t0)`, runner without dispatcher, `set_held(true)`):
    - `the_sweeper_marks_a_message_waiting_30_minutes` (Review Focus 2): a chat run with `queued_at = t0 - 31 min` and one at `t0 - 29 min`. `sweep()` returns one run with `undelivered_reason == Some("Dots is updating")`. A second `sweep()` returns none.
    - `the_sweeper_never_marks_a_usage_hold` (Review Focus 5): a held schedule run queued at `t0 - 5 h` is left unmarked.
    - `the_sweeper_skips_delivered_and_dismissed_runs`.
    - `the_sweeper_runs_on_the_clock`: `spawn`, `clock.nth_sleep(1)`, `clock.set(t0 + 30 s)`, and a `RunUpdated` for the overdue run arrives (bus `recv` with a 5 s timeout).
- [ ] **Step 2: Run** `cargo test -p dots-core --test runner_delivery_test --test delivery_sweeper_test`. Expected: compile errors.
- [ ] **Step 3: Implement.** The runtime spawns `DeliverySweeper` with `cfg.clock`, `shutdown.clone()`. `list_runs` / `get_run` / `run_thread` call `fill_wait_reasons` on the rows they return.
- [ ] **Step 4: Run** the two new files, then `runner_test`, `runner_limits_test`, `runner_quiet_test`, `runtime_limits_test`, `runtime_schedule_test`, `runtime_followups_test` (runtimes on a `FakeClock` now also run the sweeper; a test that moves the fake clock more than 30 min past a real `queued_at` may now see an extra `RunUpdated`; fix the test's expectation, not the sweeper). Then run the full `dots-core` suite once. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): a run is delivered when its engine starts, and one waiting 30 minutes is marked`.

### Task 3: Retry, Discard, today's counts

**Files:**
- Modify: `crates/dots-core/src/runtime.rs`: `chat` :1368 (the parent rule at :1418), new methods after `cancel_run` :1446
- Modify: `crates/dots-core/src/runner.rs`: a public wrapper over `cancel_inactive` (:1623)
- Modify: `crates/dots-core/src/delivery.rs` (`local_day_bounds`)
- Test: `crates/dots-core/tests/runtime_delivery_test.rs` (new; `Runtime::start` with `hold_runs(true)` before any run is created), `crates/dots-core/tests/delivery_sweeper_test.rs` (day bounds)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces:
  - `chat` parent rule: the thread's last run is now the last run **without `dismissed_at`** (fall back to none if every run is dismissed, which refuses with today's "newer message" text). The "every run has ended" check is unchanged. Algorithm: `let live = thread.iter().filter(|r| r.dismissed_at.is_none()).last()`, then `live.filter(|r| r.id == pid)`.
  - `Runner::cancel_if_queued(&self, run_id: &str) -> Result<Option<Run>>`: the existing `cancel_inactive` path (expires approvals, releases attachments, emits). `None` when the run was not queued (for example, just claimed).
  - `Runtime::retry_delivery(&self, run_id: &str) -> Result<Run>`:
    1. `get_run` (`NotFound`).
    2. `Conflict` unless the trigger is `Chat` and the run has `undelivered_at`, no `delivered_at`, no `dismissed_at`.
    3. If queued: `cancel_if_queued`. Its write also dismisses (Task 1's cancel rule). `None` → `Conflict("this message has just reached the dot")`. This also guards against a second Retry.
    4. Otherwise: `dismiss_run`. `None` → `Conflict` (already retried).
    5. Text = `payload.answer` if it is a string, else `payload.message` (what the bubble showed). If neither, `undismiss_run` and `Invalid`.
    6. `self.chat(&run.dot_id, &text, run.parent_run_id.as_deref())`. On `Err(e)`: `undismiss_run`, emit `RunUpdated` for the old run, and return `e`.
    7. Emit `RunUpdated` for the old (dismissed) run and return the new run.
  - `Runtime::discard_delivery(&self, run_id: &str) -> Result<Run>`: steps 1–4 as for Retry (a queued run is cancelled, which dismisses it; any other run is dismissed). Then emit `RunUpdated` and return the dismissed run.
  - `pub fn local_day_bounds<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (DateTime<Utc>, DateTime<Utc>)` (in `delivery.rs`): local midnight of `now`'s local date and of the next date. Resolve each with `.earliest()`; if that falls in a gap, use the first valid instant after it. Both are returned in UTC.
  - `Runtime::delivery_today(&self, dot_id: &str) -> Result<DeliveryCounts>`: `get_dot` (`NotFound`), bounds from `local_day_bounds(self.clock.now(), &Local)`, formatted with `util::timestamp`, then `store.delivery_counts`.

- [ ] **Step 1: Write the failing tests** (`runtime_delivery_test.rs`). Build the starting states with the store: `create_run`, `claim_run`, then `finish_run(Failed, None, Some("claude CLI not found"))`. Use explicit `set_run_queued_at` values 1 s apart.
  - `retry_resends_a_first_message_as_a_new_first_message`: the old run has no parent. The new run is `Chat`, `payload.message == "Ship it"`, `parent_run_id == None`, `status == Queued`. The old run has `dismissed_at` set and `dot_counts(..).0 == 0`.
  - `retrying_a_reply_continues_the_same_thread` (Review Focus 1): P (chat, succeeded, `session_id = "s1"`), then old (chat, parent P, root P, failed undelivered). Retry gives `new.parent_run_id == Some(P)`, `new.root_run_id == P`, `new.session_id == Some("s1")`. Next, finish the new run as succeeded with a session; `rt.chat(dot, "next", Some(new.id))` succeeds.
  - `retrying_twice_sends_once` (Review Focus 4): the second `retry_delivery` returns `Conflict`, and `list_runs` holds exactly one new chat run.
  - `a_refused_retry_leaves_the_message_retryable` (Review Focus 4): add a queued run to P's thread so the chat path refuses. `retry_delivery` returns `Conflict`, the old run has `dismissed_at == None`, and Needs you is still 1.
  - `retry_and_discard_refuse_what_they_do_not_apply_to`: unknown id → `NotFound`; a `Schedule` run marked not delivered → `Conflict`; a delivered chat run → `Conflict`; a waiting chat run with no mark → `Conflict`; an already dismissed run → `Conflict`.
  - `discard_dismisses_and_cancels_a_waiting_message`: a queued chat run marked by `mark_undelivered`. `discard_delivery` gives `status == Cancelled`, `dismissed_at` set, and `undelivered_at` still set (the muted line).
  - `delivery_today_counts_the_dot`: with the system clock, the runs have `queued_at` = now minus 1 s and minus 2 s: one delivered and one marked. Expect `{ sent: 2, delivered: 1, not_delivered: 1 }`. Unknown dot → `NotFound`.
  - `local_day_bounds_follow_local_midnight` (in `delivery_sweeper_test.rs`, `TestTz` UTC+1): now `2026-11-03T22:59:59Z` gives `[2026-11-02T23:00Z, 2026-11-03T23:00Z)`; now `2026-11-03T23:00:00Z` gives the next day. On 2026-03-29 (spring forward) the day is 23 h long.
- [ ] **Step 2: Run** `cargo test -p dots-core --test runtime_delivery_test --test delivery_sweeper_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the two files twice, then `runtime_test`, `runtime_followups_test`, `followups_test`, `checkins_test` (the `chat` parent rule) and the full `dots-core` suite once. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): retry or discard a message that did not get there, and count today's sends`.

### Task 4: App — commands, the notice, the contract

**Files:**
- Modify: `apps/dots-app/src/commands.rs` (next to `keep_waiting` :606), `apps/dots-app/src/lib.rs` (`command_list!`, after `keep_waiting` ~:101)
- Modify: `apps/dots-app/src/events.rs` (`Notice` :31, `Notifier` :83 / `decide` :98 / `forget` :93, `notice_text` :215, `notice_route` :288, `notify` :346, tests)
- Modify: `apps/dots-app/src/contract.rs` (samples ~:1290, `Enums` ~:1255)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Consumes: Task 3 runtime methods; `Run` fields from Task 1.
- Produces:
  - Commands `retry_delivery(run_id: String) -> Res<Run>`, `discard_delivery(run_id: String) -> Res<Run>`, `delivery_today(dot_id: String) -> Res<DeliveryCounts>`.
  - `Notice::NotDelivered { run: Box<Run> }`. Its text is (`format!("{dot_name} didn't get your message")`, `first_line(run.undelivered_reason.as_deref(), "It did not reach the dot.")`) and its route `/chat/{dot_id}/{run.id}`. In `notify`, its dot is `run.dot_id`, so the existing `quiet()` keeps it silent for the viewed dot.
  - `Notifier` gains `undelivered_told: HashSet<String>`, which `forget()` does **not** clear. In the `RunUpdated` arm, before the ended-run logic, compute `newly = run.trigger == Chat && run.undelivered_at.is_some() && run.dismissed_at.is_none() && run.delivered_at.is_none() && (was_working || run.status == Queued) && undelivered_told.insert(run.id)`. If `newly`, return `NotDelivered`. (`was_working` covers a run seen queued or running that then failed. `status == Queued` covers the sweeper's mark, also after a restart. A row seen for the first time in a final state, such as `mark_reviewed`, stays quiet.)
  - Contract: `samples["Run"]` unchanged (now delivered). Also `samples["RunNotDelivered"]` (chat, failed, `undelivered_at`, reason `claude CLI not found`), `samples["RunWaiting"]` (chat, queued, `wait_reason: busy`), `samples["DeliveryCounts"] = { sent: 5, delivered: 4, not_delivered: 1 }`, `Enums.Delivery`, `Enums.WaitReason`. The three commands come through `COMMANDS`.

- [ ] **Step 1: Write the failing tests** (events.rs `mod tests`).
  - `a_chat_message_not_delivered_notifies_once`: chat run queued, then running, then failed with the mark → one `NotDelivered`. The same row again → `None`.
  - `a_chat_run_marked_twice_notifies_once` (Review Focus 2): queued with the mark (sweeper) → notifies. Then running with `delivered_at` → nothing. Then `forget()`, then the same marked row again → nothing.
  - `a_background_run_not_delivered_does_not_notify_as_such`: a `Watcher` run failing undelivered gives today's `RunEnded`, never `NotDelivered`. A `Schedule` run gives `None`, as today.
  - `a_dismissed_or_first_seen_ended_run_stays_quiet`.
  - `notification_texts` / `a_notice_opens_the_conversation_it_is_about`: extend with `("nightly didn't get your message", "claude CLI not found")` and `/chat/d/r`. `nothing_notifies_about_the_dot_being_viewed` already covers `quiet()`; add the `NotDelivered` route line there.
- [ ] **Step 2: Run** `cargo test -p dots-app events`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands, the notice and the samples.
- [ ] **Step 4: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app`. Expected: PASS. The UI contract test fails until Task 5 updates `types.ts` and `commands.ts`; that is expected here.
- [ ] **Step 5: Commit** `feat(app): retry, discard and today's counts, and a notice when a message does not get there`.

### Task 5: UI — delivery lines, Retry/Discard, the menu's counts

**Files:**
- Modify: `ui/src/api/types.ts` (`runShape` ~:145–182: the six new fields; new `Delivery`, `WaitReason`, `DeliveryCounts`), `ui/src/api/commands.ts` (next to `keepWaiting` :174: `retryDelivery`, `discardDelivery`, `deliveryToday`)
- Create: `ui/src/lib/delivery.ts` (`WAIT_REASON_TEXT`, `deliveryLine(run, now)`), `ui/src/components/DeliveryLine.tsx`
- Modify: `ui/src/components/Transcript.tsx` (under the person's bubble :463–473, and the conversation `Cancelled` marker :503: hidden when `undelivered_at` is set), `ui/src/screens/home/RunCard.tsx` (under `HeldNote`), `ui/src/screens/home/DotMenu.tsx` (the counts line), `ui/src/lib/home-topics.ts` (`lastRun` :85), `ui/src/lib/query.ts` (`keys.deliveryToday`)
- Test: `ui/src/lib/delivery.test.ts`, `ui/src/components/DeliveryLine.test.tsx`, `ui/src/screens/home/HomeMenu.test.tsx` (menu line), `ui/src/lib/home-topics.test.ts`

**Interfaces:**
- Consumes: Task 4 contract.
- Produces:
  - `deliveryLine(run: Run, now: number): { kind: "waiting" | "not_delivered" | "dismissed"; text: string } | null`. Its cases:
    - `not_delivered` and not dismissed → `Not delivered: <undelivered_reason ?? "">`.
    - Dismissed → `Not delivered`.
    - `waiting`, `queued`, `wait_reason` set and ≥ 5 whole minutes since `queued_at` → `Waiting <n> min · <WAIT_REASON_TEXT[reason]>`.
    - Otherwise → `null`.
  - `<DeliveryLine run={run} />`: renders the line. A `waiting` line re-renders with `useNow`. For `not_delivered` + `trigger === "chat"` it shows `Retry` and `Discard` buttons. On settle they invalidate `["runs"]`, `["thread"]` and `keys.overview`. Each press resets the other's error. The error shows as `role="alert"` with `errorMessage`, as `QuietCard` does. Background runs get no buttons.
  - `keys.deliveryToday(dotId) = ["runs", dotId, "delivery-today"]`. This sits under `["runs"]`, so the existing `run_updated` handler refreshes it; no runtime-events change is needed.
  - `DotMenu`: a non-interactive `DropdownMenu.Label` after `Enabled` showing `Today: ${s} sent · ${d} delivered · ${u} not delivered`. The query is enabled while the menu is open.
  - `lastRun(topic)`: the last run without `dismissed_at`, else the last run (so a reply after a Retry or Discard continues the live line, matching Task 3's core rule).

- [ ] **Step 1: Write the failing tests.**
  - `deliveryLine`, with a fixed `now` and `queued_at` = `now - k*60_000` as ISO strings (no locale or time zone involved): 4 min `busy` → `null`; 5 min for each of the five reasons → `Waiting 5 min · other runs are busy` / `… held for the usage limit` / `… Dots is updating` / `… this dot is busy with another run` / `… next in line`; 12 min `queued` → `Waiting 12 min · next in line`; marked → `Not delivered: claude CLI not found`; dismissed → `Not delivered`; delivered → `null`.
  - `DeliveryLine`: a chat not-delivered run shows the text and the buttons `Retry` and `Discard`. `Retry` calls `retry_delivery` with `{ runId }`. A rejected `discard_delivery` shows its message in the alert. A watcher run that was not delivered shows the text with no buttons. A dismissed run shows muted `Not delivered` with no buttons.
  - Conversation: a dismissed chat run followed by its retry shows `Not delivered` above the new bubble, and no `Cancelled`.
  - `lastRun` skips a dismissed last run.
  - Menu: opening the dot menu shows `Today: 5 sent · 4 delivered · 1 not delivered` (backend stub `delivery_today`). A `run_updated` for the dot refetches it.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement** with the Global Constraints copy. Use the warning colour for not delivered, the token `QuietCard` uses (`text-needs`).
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS, the contract test included.
- [ ] **Step 5: Commit** `feat(ui): each message says whether it got there, with Retry and Discard, and today's counts in the dot menu`.

### Task 6: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps: `cargo fmt --all -- --check`; `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`; `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times` (twice); `cargo test -p dots-app --locked`; `node --test tools/release/release.test.mjs`; `cd ui; bun run typecheck; bun run test`. Expected: all pass, in both core runs.
- [ ] **Step 2: Push** `feat/delivery-tracking` and open the PR with base `feat/watch-hours`. Note in the body that it is stacked on #47 (and #47 on #46), that it adds migration 0030, and the rulings this plan made (Task 1 cancel/dismiss, Task 3 `chat` parent rule and Retry text, `delivery: Option`).

## Rulings (controller, after drafting)

- Ruling: wait reason `dot_off` replaced by `dot_busy` ("this dot is busy with another run") — the dispatcher starts runs of a disabled dot anyway, so "turned off" would be untrue; a run waiting behind its own dot's run was otherwise shown as "next in line" — cost if wrong: a disabled dot's queued run shows a different reason.
- Ruling: spec §2/§6 reason list changes accordingly (`dot_off` → `dot_busy`).
