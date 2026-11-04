# Send later — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The person can schedule a chat message to a dot ("Tomorrow morning", "Monday 9 AM", a picked time). It waits as a card with a live countdown and Send now / Edit / Cancel, and goes to the topic it was written in as an ordinary chat run, even after Dots was closed over the due time ("Sent late (was due 9:00 AM)").

**Architecture:** One new table `scheduled_messages` (migration 0032) with a small state machine, where every change is a conditional `UPDATE`. `Runtime::chat` becomes a wrapper over `chat_with(.., ChatExtra)`, which writes a `send_later` note into the run's payload. `Runtime::send_scheduled` picks the target at send time: reply to the topic's end, wait while the topic is busy or the Claude limit is reached, or start a new topic when the old one cannot be continued. A loop on the runtime's `Clock` (`spawn_send_later`, started by the app once the runtime is in an `Arc`) sends what is due. The app adds five commands, one event and one notice. The UI adds an arrow menu beside Send (the presets helper from snooze/done), a card per pending message at the end of the conversation, a line under a sent message, and "N scheduled" in the dot menu.

**Tech Stack:** Rust (tokio, sqlx/SQLite, chrono), Tauri 2, React + TypeScript (zod, vitest, TanStack Query, radix-ui).

**Spec:** `docs/superpowers/specs/2026-11-04-send-later-design.md`

## Global Constraints

- Branch `feat/send-later`, stacked on `feat/snooze-done`. Migration **`0032_send_later.sql`**: the `CREATE TABLE` and two indexes from spec §4.1, verbatim.
- Statuses: `scheduled` | `sending` | `sent` | `failed` | `cancelled`. Waiting reasons: `busy` | `approval` | `usage_limit`. Pending = `scheduled`, `sending`, `failed`.
- Allowed changes (each one conditional `UPDATE`): claim `scheduled → sending`; `sending → sent`; `sending → scheduled` + `waiting` (lost race); `scheduled|sending → failed`; Edit `scheduled|failed → scheduled` (clears `waiting`, `error`); Cancel `scheduled|failed → cancelled`; Send now `scheduled|failed → scheduled` with `due_at = now`, then a send.
- Times: `due_at` from 1 minute to 365 days after `clock.now()`. Late = `sent_at − due_at > 60 s`. The sender wakes at the earliest future `due_at` or after 30 s, whichever is sooner.
- Run payload of a sent message: `{ "message": <text>, "typed": true, "send_later": { "id", "due_at", "late", "new_topic" } }`.
- Core texts (verbatim): `Pick a time at least a minute from now.` · `Pick a time within the next year.` · `This message is being sent now.` · `This message has already been sent.` · `This message was cancelled.` · empty text → `message must not be empty` (the chat path's).
- UI copy (verbatim): `Send later` (button label and menu title, dialog title) · `Pick a date and time…` · `Date and time` · `Schedule` · `Cancel` · `Scheduled for <when> · <target>` with target `current topic` / `new topic` / `<topic title>` / `an older topic` · `Sends in 2h 14m` (two largest non-zero of d/h/m, minutes rounded up) · `Sending…` · `Due · waiting for <dot> to finish this topic` · `Due · waiting for your approval in this topic` · `Due · waiting for the usage limit to reset` · `Not sent: <error>` · `Send now` · `Edit` · `Cancel` · `Scheduled message` · `Send at` · `Save` · `Discard changes` · `Sent late (was due <time>)` · `Sent as scheduled` · `Started a new topic: the one it was written in can't be continued.` · `<n> scheduled`.
- Notice: title `<dot> didn't get your scheduled message`, body = the error's first line, route `/home/<dot>`. Once per row and failure; never for the dot being viewed.
- Presets come from the presets helper from snooze/done (`ui/src/lib/later.ts`). This plan writes against `laterPresets(now: Date): { label: string; at: Date }[]`; adapt to the actual export names that the snooze plan landed, and do not redefine its rules.
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH. Never commit `CLAUDE.local.md` or `.cargo/config.toml`. No AI trailer in commits.
- Tests must not depend on same-millisecond ULID order (assert by id, set times explicitly), on a dispatcher claiming queued runs (`rt.hold_runs(true)` before creating runs), on a read racing an async writer (await the command, or wait for the event), or on the real clock (`FakeClock` in `Config.clock`; UI tests pass a fixed `now`).

## Review Focus

1. **Dots was closed over two messages due for the same topic.** On start, the first goes at once (late). The second waits (`busy`) until the dot has answered the first, then replies to that answer. Neither is lost and neither is sent twice. Test `two_due_for_one_topic_go_in_order` in Task 2.
2. **Dots crashed after the claim, before or after the run was stored.** On restart, a `sending` row whose run exists (payload `send_later.id`) becomes `sent` with that run; one without becomes `scheduled` and is sent once. Tests `recovery_finds_the_run_a_crash_left` in Task 1 and `a_crash_while_sending_never_sends_twice` in Task 2.
3. **Edit or Cancel pressed just as the sender takes the message.** The loser gets the exact Conflict text, nothing the person typed is sent, and the card turns into the sent message. Tests `edit_and_cancel_lose_to_a_claim` and `the_sender_skips_a_cancelled_message` in Task 2, and `an edit that loses shows why` in Task 5.
4. **The person typed into the topic after scheduling, and the dot is still answering at the due time.** The message waits (`busy`), then replies to the newest run, not to the run that was last when it was scheduled. A `Conflict` from the chat path (the thread moved after the check) means wait, not fail. Tests `a_busy_topic_waits_then_replies_to_its_newest_run` and `a_lost_race_in_the_chat_path_waits` in Task 2.
5. **The topic cannot be continued at send time** (its last run never got a session, or its root is gone). The message starts a new topic with `new_topic: true`. It is never stuck failing, and the bubble says so. Tests `a_topic_that_cannot_continue_gets_a_new_topic` in Task 2 and the new-topic line in Task 5.

---

### Task 1: Storage — table, model, state changes, recovery

**Files:**
- Create: `crates/dots-core/migrations/0032_send_later.sql`
- Create: `crates/dots-core/src/store/scheduled_messages.rs` (register in `store/mod.rs` next to `mod skips;` :28)
- Modify: `crates/dots-core/src/model.rs` (next to `ScheduleSkip` :448)
- Test: `crates/dots-core/tests/store_send_later_test.rs` (new)

**Interfaces:**
- Produces:
  - `str_enum!(ScheduledStatus { Scheduled => "scheduled", Sending => "sending", Sent => "sent", Failed => "failed", Cancelled => "cancelled" })`
  - `str_enum!(ScheduledWait { Busy => "busy", Approval => "approval", UsageLimit => "usage_limit" })`
  - `pub struct ScheduledMessage { pub id: String, pub dot_id: String, pub root_run_id: Option<String>, pub text: String, pub due_at: String, pub status: ScheduledStatus, pub waiting: Option<ScheduledWait>, pub error: Option<String>, pub run_id: Option<String>, pub sent_at: Option<String>, pub created_at: String, pub updated_at: String }` (Serialize, Deserialize, Clone, Debug, PartialEq).
  - `pub struct SentLater { pub id: String, pub due_at: String, pub late: bool, pub new_topic: bool }` (Serialize, Deserialize, Clone, Debug, PartialEq): the payload note.
  - Store, all `-> Result<..>`, `now: DateTime<Utc>` written with `util::timestamp`:
    - `insert_scheduled_message(dot_id: &str, root_run_id: Option<&str>, text: &str, due_at: &str, now) -> ScheduledMessage` (status `scheduled`, id `new_id()`)
    - `get_scheduled_message(id: &str) -> ScheduledMessage` (`NotFound("scheduled message {id}")`)
    - `pending_scheduled_messages(dot_id: &str) -> Vec<ScheduledMessage>` (`scheduled|sending|failed`, `ORDER BY due_at, id`)
    - `due_scheduled_messages(now: &str) -> Vec<ScheduledMessage>` (`scheduled`, `due_at <= ?`, `ORDER BY due_at, id`)
    - `next_scheduled_due(after: &str) -> Option<String>` (min `due_at` of `scheduled` rows with `due_at > ?`)
    - `claim_scheduled_message(id, now) -> Option<ScheduledMessage>` (`scheduled → sending`)
    - `set_scheduled_waiting(id, wait: ScheduledWait, now) -> Option<ScheduledMessage>`: `WHERE id = ? AND (status = 'sending' OR (status = 'scheduled' AND waiting IS NOT ?))`; sets `status = 'scheduled'` and `waiting`. `None` when nothing changed (the sender then sends no event).
    - `mark_scheduled_sent(id, run_id: &str, now) -> Option<ScheduledMessage>` (from `sending`; sets `sent_at = now`, clears `waiting`, `error`)
    - `mark_scheduled_failed(id, error: &str, now) -> Option<ScheduledMessage>` (from `scheduled|sending`; clears `waiting`)
    - `edit_scheduled_message(id, text: &str, due_at: &str, now) -> Option<ScheduledMessage>` (from `scheduled|failed`)
    - `cancel_scheduled_message(id, now) -> Option<ScheduledMessage>` (from `scheduled|failed`)
    - `reset_for_send_now(id, now) -> Option<ScheduledMessage>` (from `scheduled|failed`: `status = scheduled`, `due_at = now`, `error = NULL`)
    - `recover_sending_messages(now) -> Vec<ScheduledMessage>`: for each `sending` row, a run with `json_extract(payload, '$.send_later.id') = id` → `sent` with that `run_id` and the run's `queued_at` as `sent_at`; otherwise → `scheduled`. Returns the changed rows.

- [ ] **Step 1: Write the failing tests** (store only; a dot from `fake_dot`; runs from `create_run`; fixed instants `T0 = 2026-11-04T09:00:00.000Z`).
  - `a_scheduled_message_is_stored_and_listed`: insert two with due `T0+2h` and `T0+1h`. `pending_scheduled_messages` returns them in due order, `status == Scheduled`, `waiting == None`, `root_run_id` as given (one `None`).
  - `due_and_next_follow_due_at`: due at `T0-1m`, `T0`, `T0+5m`. `due_scheduled_messages(T0)` returns the first two in order. `next_scheduled_due(T0) == Some(T0+5m)`. A `cancelled` or `sent` row is never due.
  - `a_claim_happens_once`: the first `claim_scheduled_message` gives `Sending`; the second gives `None`. Then `edit_scheduled_message` and `cancel_scheduled_message` both give `None`.
  - `edit_and_cancel_only_from_scheduled_or_failed`: Edit on `scheduled` sets the text and `due_at` and keeps `Scheduled`. Edit on `failed` gives `Scheduled` with `error == None`. Cancel on `failed` gives `Cancelled`. On `sent`, both give `None`.
  - `waiting_changes_only_when_it_differs`: `set_scheduled_waiting(Busy)` on `scheduled` gives `Some`, then `None` the second time, then `Some` for `Approval`. On `sending` it always gives `Some` with `Scheduled`.
  - `sent_and_failed_record_their_outcome`: `mark_scheduled_sent` sets `run_id`, `sent_at` and clears `waiting`. `mark_scheduled_failed(id, "org removed", ..)` sets `error`.
  - `reset_for_send_now_moves_due_at_to_now`: on a `failed` row, `due_at == T0`, `error == None`, `Scheduled`.
  - `recovery_finds_the_run_a_crash_left` (Review Focus 2): two rows claimed. A chat run whose payload has `send_later.id` = the first row's id. `recover_sending_messages` makes the first `Sent` with `run_id` = that run, and the second `Scheduled`.
  - `rows_go_with_their_dot`: `delete_dot` removes them.
- [ ] **Step 2: Run** `cargo test -p dots-core --test store_send_later_test`. Expected: compile errors.
- [ ] **Step 3: Implement** the migration, the model types and the store module (row mapper and a `COLUMNS` const as in `store/skips.rs`).
- [ ] **Step 4: Run** `cargo test -p dots-core --test store_send_later_test --test store_runs_test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): scheduled messages are stored with their state`.

### Task 2: Core — the chat extra, sending one message, the commands, the sender loop

**Files:**
- Create: `crates/dots-core/src/send_later.rs` (register `pub mod send_later;` in `lib.rs`; pure parts and constants)
- Modify: `crates/dots-core/src/runtime.rs`: `chat` :1407 (becomes a wrapper; the live-end rule at :1460 moves to `send_later::thread_end`); `Runtime` struct :111 (a `send_later_wake: Arc<Notify>` field); `start` after `recover_interrupted` :231 (recovery) and the struct literal :460–490; new methods after `delivery_today` :1587
- Modify: `crates/dots-core/src/events.rs` (new variant next to `ScheduleSkipped` :42)
- Test: `crates/dots-core/tests/runtime_send_later_test.rs` (new; `Config.clock = FakeClock::new(T0)`, `claude_path` = fake-claude, `rt.hold_runs(true)` before any run, as `runtime_delivery_test.rs` :26 does)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - In `send_later.rs`: `pub const LATE_AFTER: TimeDelta = TimeDelta::seconds(60)`; `pub const MIN_AHEAD: TimeDelta = TimeDelta::minutes(1)`; `pub const MAX_AHEAD: TimeDelta = TimeDelta::days(365)`; `pub const SEND_EVERY: TimeDelta = TimeDelta::seconds(30)`.
  - `pub fn check_due(due_at: &str, now: DateTime<Utc>) -> Result<String>`: parses RFC 3339 (`Invalid("due_at is not a time")`). Under `now + MIN_AHEAD` gives `Invalid("Pick a time at least a minute from now.")`. Over `now + MAX_AHEAD` gives `Invalid("Pick a time within the next year.")`. Returns the instant as `timestamp(..)`.
  - `pub fn thread_end(thread: &[Run]) -> Option<&Run>`: the chat path's rule, `thread.iter().rfind(|r| r.dismissed_at.is_none() || r.delivered_at.is_some())`. `chat` uses it too, with unchanged behaviour.
  - `pub fn is_late(due_at: &str, sent_at: DateTime<Utc>) -> bool` (`> LATE_AFTER`).
  - `pub fn not_changeable(status: ScheduledStatus) -> Error`: `Sending` gives `Conflict("This message is being sent now.")`, `Sent` gives `Conflict("This message has already been sent.")`, and `Cancelled` gives `Conflict("This message was cancelled.")`.
  - `pub enum SendTarget { Reply { parent_run_id: String }, New { fallback: bool }, Wait(ScheduledWait) }`.
  - `#[derive(Default, Clone, Debug)] pub struct ChatExtra { pub send_later: Option<SentLater> }`. **The attachments extension point.** Attachments adds its fields here, and fills them in `chat_extra_for`.
  - `RuntimeEvent::ScheduledMessageUpdated { message: ScheduledMessage }`.
  - Runtime:
    - `pub async fn chat_with(&self, dot_id: &str, message: &str, parent_run_id: Option<&str>, extra: ChatExtra) -> Result<Run>`. Same checks and lock as today's `chat`. It adds `"send_later": extra.send_later` to the payload when `Some`. `chat(..)` = `chat_with(.., ChatExtra::default())`.
    - `pub async fn schedule_message(&self, dot_id: &str, text: &str, root_run_id: Option<&str>, due_at: &str) -> Result<ScheduledMessage>`: empty text gives `Invalid("message must not be empty")`. Then `get_dot`, `require_run`, `check_due(due_at, self.clock.now())`. A root must be a run of this dot (`Invalid("run {id} belongs to a different dot")`). Then insert, emit, and `send_later_wake.notify_one()`.
    - `pub async fn scheduled_messages(&self, dot_id: &str) -> Result<Vec<ScheduledMessage>>` (`require_for_dot(RunDots)`; pending rows).
    - `pub async fn edit_scheduled_message(&self, id: &str, text: &str, due_at: &str) -> Result<ScheduledMessage>`: the same checks, then the store's edit. `None` gives `not_changeable(current.status)`. Emit, wake.
    - `pub async fn cancel_scheduled_message(&self, id: &str) -> Result<ScheduledMessage>`: store cancel. `None` gives `not_changeable`. Then `self.release_staged(id).await` and emit.
    - `pub async fn send_scheduled_now(&self, id: &str) -> Result<ScheduledMessage>`: `require_run`, then `reset_for_send_now`. `None` gives `not_changeable`. Then `send_scheduled(row)`. If the sender claimed it in between, return the row as it now stands.
    - `pub async fn send_due(&self) -> Result<Vec<ScheduledMessage>>`: one pass over `due_scheduled_messages(now)` in order, `send_scheduled` each. An error on one row is logged and the pass goes on. Returns the rows that changed.
    - `async fn send_scheduled(&self, msg: ScheduledMessage) -> Result<ScheduledMessage>`: spec §4.2 steps 1–6. `require_run` refused gives `mark_scheduled_failed(first line of the error)`. A `Wait` target gives `set_scheduled_waiting`. Then claim (`None` → return the current row). Then `chat_with(dot, text, parent, self.chat_extra_for(&msg, now, fallback))`. `Err(Conflict)` → `set_scheduled_waiting(Busy)`. Other `Err` → `mark_scheduled_failed`. `Ok(run)` → `mark_scheduled_sent(run.id)`. Emit `ScheduledMessageUpdated` for every `Some` row.
    - `#[doc(hidden)] pub async fn send_scheduled_to(&self, msg: ScheduledMessage, target: SendTarget) -> Result<ScheduledMessage>`: steps 4–6 of spec §4.2 for a target already chosen. `send_scheduled` calls it, and tests use it to pass a stale target.
    - `async fn send_target(&self, msg: &ScheduledMessage) -> Result<SendTarget>`. Order:
      1. No root → `New { fallback: false }`.
      2. Root row missing (`NotFound`) → `New { fallback: true }`.
      3. A non-terminal run in the thread → `Wait(Approval)` if one is `AwaitingApproval`, else `Wait(Busy)`. `follow_ups.deciding(root)` → `Wait(Busy)`.
      4. `thread_end` is `None`, or has no `session_id`, or `store.in_ended_step_thread(end)` → `New { fallback: true }`.
      5. Else `Reply { parent_run_id: end.id }`.
      6. After the target: a Claude dot with `limits.rejected_until().is_some()` → `Wait(UsageLimit)` (except over a `Wait` already found).
    - `fn chat_extra_for(&self, msg: &ScheduledMessage, sent_at: DateTime<Utc>, new_topic: bool) -> ChatExtra` (`SentLater { id, due_at, late: is_late(..), new_topic }`).
    - `async fn release_staged(&self, _id: &str)`: a no-op, kept for attachments.
    - `pub fn spawn_send_later(self: &Arc<Self>) -> JoinHandle<()>`. Loop: `send_due()` (log errors). Then `wake = min(next_scheduled_due(now) or none, now + SEND_EVERY)`. Then `select! { biased; shutdown.cancelled() => break, send_later_wake.notified() => {}, clock.sleep_until(wake) => {} }`. The first pass runs at once.
    - In `start`: `store.recover_sending_messages(now)` right after `recover_interrupted`.

- [ ] **Step 1: Write the failing tests.** Fix the clock at `T0`. Build threads with `rt.chat`, then `store` `claim_run` / `finish_run(Succeeded)` and a `session_id`, as `runtime_delivery_test.rs` does. Move time with `clock.set(..)`, then call `rt.send_due()`.
  - `check_due_refuses_past_near_and_far` (unit, `send_later.rs` tests): `T0+30s` gives the minute text. `T0+366d` gives the year text. `"soon"` gives `due_at is not a time`. `T0+1h` gives `Ok("2026-11-04T10:00:00.000Z")`.
  - `scheduling_validates_and_lists`: empty text → `Invalid`; a root of another dot → `Invalid`; a good one is listed by `scheduled_messages` and emits `ScheduledMessageUpdated`.
  - `a_due_message_replies_to_its_topic`: topic P (ended, session `s1`). Schedule for `T0+1h` with root P, `clock.set(T0+1h)`, `send_due`. The row is `Sent` with `run_id = R`. R is `Chat` with `parent_run_id == Some(P)` and `payload.message` = the text, and `payload.send_later == { id, due_at, late: false, new_topic: false }`.
  - `a_message_with_no_topic_starts_one`: root `None` gives R with `parent_run_id == None` and `new_topic: false`.
  - `a_busy_topic_waits_then_replies_to_its_newest_run` (Review Focus 4): P, then the person's Q (parent P), still queued at the due time. `send_due` gives `waiting == Some(Busy)` and status `Scheduled`, with no new run. A second `send_due` emits no event. End Q with a session, `clock.set(+30s)`, `send_due`: R's parent is Q.
  - `an_approval_in_the_topic_waits_with_its_reason`: a run of the thread `AwaitingApproval` (store `set_status`) gives `Approval`.
  - `a_lost_race_in_the_chat_path_waits` (Review Focus 4): P ended with a session. Call `send_scheduled_to(msg, SendTarget::Reply { parent_run_id: P })` after a run Q was added to P's thread, so the target is stale and the chat path answers `Conflict`. The row is `Scheduled` with `Busy`, not `Failed`, and no run was created.
  - `a_topic_that_cannot_continue_gets_a_new_topic` (Review Focus 5): P failed before a session (`session_id == None`) gives R with no parent and `new_topic: true`. A root id that does not exist (insert the row directly with a made-up root) gives the same.
  - `the_usage_limit_waits`: `limits` rejected until `T0+2h` (the `runtime_limits_test.rs` way), due `T0+1h` gives `UsageLimit`. After `clock.set(T0+2h+1m)` it is sent with `late: true`.
  - `a_refused_dot_fails_with_its_reason`: the dot's org removed (as in the `require_run` tests) gives `Failed` with `error` = the refusal's first line, and no run.
  - `late_is_more_than_a_minute`: sent at due +30 s gives `late: false`; at due +2 min gives `late: true`.
  - `two_due_for_one_topic_go_in_order` (Review Focus 1): two messages to P due `T0+1h` and `T0+1h+1m`, clock at `T0+3h`. The first `send_due` sends A (parent P, `late: true`) and leaves B `Busy`. End A's run with a session. The next `send_due` sends B with parent = A's run.
  - `edit_and_cancel_lose_to_a_claim` (Review Focus 3): `store.claim_scheduled_message`, then `edit_scheduled_message` gives `Conflict("This message is being sent now.")`, `cancel_scheduled_message` gives the same, and after `mark_scheduled_sent` both give `Conflict("This message has already been sent.")`.
  - `the_sender_skips_a_cancelled_message`: cancel, pass the due time, `send_due` → no run.
  - `send_now_sends_at_once_and_is_not_late`: a message due tomorrow gives `Sent`, `late: false`, and the row's `due_at == T0`. A second `send_scheduled_now` gives `Conflict("This message has already been sent.")`. Send now on a `failed` row of a dot that may run again gives `Sent`.
  - `a_crash_while_sending_never_sends_twice` (Review Focus 2): start a runtime on a data dir with one `sending` row and no run, then a second runtime start on the same dir. The row is `Scheduled` and, once due, sends one run. The same with the run already present: `Sent` and no second run.
  - `the_sender_sends_on_the_clock`: `Arc::new(rt)`, `spawn_send_later()`, `clock.nth_sleep(1)`. Its deadline is `min(due, T0+30s)`. `clock.set(due)`, then a `ScheduledMessageUpdated` with `Sent` arrives (bus `recv` with a 5 s timeout).
- [ ] **Step 2: Run** `cargo test -p dots-core --test runtime_send_later_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the new file twice, then `runtime_test`, `runtime_delivery_test`, `runtime_followups_test`, `checkins_test` (the `chat` wrapper and `thread_end`), then the whole `dots-core` suite once. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): send a message later, to the topic it was written in`.

### Task 3: App — commands, start-up, the notice, the contract

**Files:**
- Modify: `apps/dots-app/src/commands.rs` (next to `delivery_today` :626), `apps/dots-app/src/lib.rs` (`command_list!` next to `delivery_today` :104)
- Modify: `apps/dots-app/src/startup.rs` :310–313 (`spawn_send_later` on the `Arc`)
- Modify: `apps/dots-app/src/events.rs` (`Notice` :31, `Notifier`/`decide`, `notice_text`, `notice_route` ~:317, tests)
- Modify: `apps/dots-app/src/contract.rs` (sample next to `schedule_skip()` :519, `samples[..]` ~:1328, the `RuntimeEvent` list ~:1368, `Enums`)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Consumes: Task 2.
- Produces:
  - Commands (camelCase args from the UI): `schedule_message(dot_id: String, text: String, root_run_id: Option<String>, due_at: String) -> Res<ScheduledMessage>`, `scheduled_messages(dot_id: String) -> Res<Vec<ScheduledMessage>>`, `send_scheduled_now(id: String) -> Res<ScheduledMessage>`, `edit_scheduled_message(id: String, text: String, due_at: String) -> Res<ScheduledMessage>`, `cancel_scheduled_message(id: String) -> Res<ScheduledMessage>`.
  - `Notice::ScheduledNotSent { message: Box<ScheduledMessage> }`. Its text is `(format!("{dot_name} didn't get your scheduled message"), first_line(message.error.as_deref(), "It could not be sent."))`. Its route is `/home/{dot_id}`, and its dot is `message.dot_id` (so `quiet()` covers the viewed dot).
  - `Notifier` gains `not_sent_told: HashSet<(String, String)>` (row id, `updated_at`), which `forget()` does not clear. It notifies when `status == Failed` and the insert is new. A row that fails again after an Edit has a new `updated_at`, so it notifies again.
  - Contract: `samples["ScheduledMessage"]` (scheduled, root `01J0RUN`, due `2026-11-05T09:00:00.000Z`, `waiting: busy`). Also `samples["ScheduledMessageFailed"]` (failed, error). `Enums.ScheduledStatus`, `Enums.ScheduledWait`, and the event sample. `samples["RunSentLate"]`: a chat run whose payload carries `send_later` with `late: true` (fixture for the UI's zod parse).

- [ ] **Step 1: Write the failing tests** (events.rs `mod tests`):
  - `a_failed_scheduled_message_notifies_once`: a `ScheduledMessageUpdated` with `Failed` gives one `ScheduledNotSent`; the same row again gives `None`; the same row with a later `updated_at` notifies again.
  - `scheduled_messages_otherwise_stay_quiet`: `Scheduled`, `Sent`, `Cancelled`, and waiting rows give `None`.
  - Extend `notification_texts` with `("nightly didn't get your scheduled message", "<org removed text>")`, and `a_notice_opens_the_conversation_it_is_about` with `/home/d`. Extend `nothing_notifies_about_the_dot_being_viewed` with this notice.
- [ ] **Step 2: Run** `cargo test -p dots-app events`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands, the start-up call, the notice and the samples.
- [ ] **Step 4: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app`. Expected: PASS. The UI contract test fails until Task 4 updates `types.ts` and `commands.ts`.
- [ ] **Step 5: Commit** `feat(app): commands for scheduled messages, and a notice when one cannot be sent`.

### Task 4: UI — API, helpers, and the Send later menu in the composer

**Files:**
- Modify: `ui/src/api/types.ts` (new `ScheduledStatus`, `ScheduledWait`, `ScheduledMessage` near `ScheduleSkip` :478; event `scheduled_message_updated` in the event union ~:504)
- Modify: `ui/src/api/commands.ts` (next to `deliveryToday` :178), `ui/src/lib/query.ts` (`keys` :23), `ui/src/lib/runtime-events.ts` (next to `schedule_skipped` :68)
- Create: `ui/src/lib/send-later.ts`, `ui/src/screens/home/SendLaterMenu.tsx`
- Modify: `ui/src/screens/home/Composer.tsx` (an optional `onSchedule` prop; the menu beside Send), `ui/src/screens/home/ReplyArea.tsx` (the `schedule` mutation; `onSchedule` passed unless `answering`)
- Test: `ui/src/lib/send-later.test.ts`, `ui/src/screens/home/SendLater.test.tsx` (new), `ui/src/lib/runtime-events.test.ts`

**Interfaces:**
- Consumes: Task 3 contract; `laterPresets` from the presets helper from snooze/done (`ui/src/lib/later.ts`). The snooze plan provides it; adapt the name.
- Produces:
  - `commands.scheduleMessage(dotId: string, text: string, rootRunId: string | null, dueAt: string)`, `commands.scheduledMessages(dotId)`, `commands.sendScheduledNow(id)`, `commands.editScheduledMessage(id, text, dueAt)`, `commands.cancelScheduledMessage(id)`.
  - `keys.scheduledMessages(dotId) = ["scheduled-messages", dotId]`. On `scheduled_message_updated`, invalidate `keys.scheduledMessages(event.message.dot_id)`. A `sent` row also invalidates `["runs"]` and `["thread"]`.
  - In `send-later.ts`:
    - `sendsIn(dueAt: string, now: number): string`: `Sends in …` or `Sending…`. Minutes are `Math.ceil(ms / 60_000)`, split into d/h/m, and the two largest non-zero units are shown.
    - `waitingText(waiting: ScheduledWait, dotName: string): string`.
    - `SentLaterNote = z.object({ id: z.string(), due_at: z.string(), late: z.boolean(), new_topic: z.boolean() })`.
    - `sentLaterLines(run: Run): string[]`: `[]` for a non-chat run or a payload without a valid `send_later`. Otherwise `Sent late (was due ${localDayTime(due_at, Date.parse(run.queued_at))})` or `Sent as scheduled`, plus the new-topic line when `new_topic`.
    - `toLocalInput(iso: string): string` and `fromLocalInput(value: string): string` (`datetime-local` ↔ ISO, in local time).
  - `<SendLaterMenu disabled: boolean; onPick: (at: Date) => Promise<unknown> />`: the chevron button (`aria-label="Send later"`). Its menu lists `laterPresets(new Date())` with each time right-aligned (`localWeekdayTime(at.toISOString())`), then `Pick a date and time…`, which opens a `Dialog` titled `Send later` with the `Date and time` field and `Schedule` / `Cancel`. Errors from `onPick` show in the dialog (`role="alert"`, `errorMessage`).
  - `Composer`: `onSchedule?: (message: string, at: Date) => Promise<unknown>`. The menu shows only when it is given, is disabled exactly when Send is, and clears the text on success (as `send()` does).
  - `ReplyArea`: `schedule = useMutation({ mutationFn: ({ text, at }) => commands.scheduleMessage(dotId, text, target?.rootId ?? null, at.toISOString()) })`. On success it calls `onPick({ kind: "default" })` and invalidates `keys.scheduledMessages(dotId)`; on error `notifyError`. `target` is the existing reply target (undefined for a new topic or a topic with nothing to continue).

- [ ] **Step 1: Write the failing tests.**
  - `sendsIn`, with `now` fixed and `dueAt = new Date(now + ms).toISOString()`: 2 h 13 min 10 s gives `Sends in 2h 14m`; exactly 60 min gives `Sends in 1h`; 3 d 4 h gives `Sends in 3d 4h`; 20 s gives `Sends in 1m`; 0 and −5 s give `Sending…`.
  - `waitingText`: the three texts for dot `nightly`.
  - `sentLaterLines`: late gives `Sent late (was due <localDayTime>)`, computed with the same function in the test, so no locale is assumed. On time gives `Sent as scheduled`. `new_topic` adds the line. A schedule run or a plain chat run gives `[]`.
  - `SendLater.test.tsx`, with a backend stub and a fake `later.ts` preset list via `vi.mock` (one `In 1 hour` at a fixed `Date`):
    - `the arrow menu schedules the text for the reply target`: type `Ship it`, open `Send later`, choose `In 1 hour`. `schedule_message` is called with `{ dotId, text: "Ship it", rootRunId: <current topic root>, dueAt: <the preset's ISO> }`, and the box is cleared.
    - `a new topic schedules without a root`: pick New topic, and `rootRunId: null`.
    - `the custom time is sent as an instant`: fill `Date and time` with `toLocalInput(iso)`, press `Schedule`, and `dueAt` is that ISO string.
    - `a refused time shows its reason in the dialog`: the stub rejects with `Pick a time at least a minute from now.`, the alert shows it, and the text stays.
    - `the menu is locked when Send is` (a busy topic), and `hidden while answering a question`.
  - `runtime-events`: `scheduled_message_updated` invalidates `["scheduled-messages", dotId]`; a `sent` row also invalidates `["runs"]`.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS, the contract test included.
- [ ] **Step 5: Commit** `feat(ui): a Send later menu beside Send`.

### Task 5: UI — the scheduled card, the sent line, and "N scheduled"

**Files:**
- Create: `ui/src/screens/home/ScheduledCard.tsx` (`ScheduledCard` and `ScheduledCards`)
- Modify: `ui/src/screens/home/Timeline.tsx` (new prop `scheduledFor?: { dotId: string; root?: string }`, cards after the entries), `ui/src/screens/home/DotPane.tsx` :79 (pass `{ dotId: dot.id, root: rootRunId }`)
- Modify: `ui/src/components/Transcript.tsx` (under the person's bubble, next to `<DeliveryLine run={run} />` ~:474)
- Modify: `ui/src/screens/home/DotMenu.tsx` (a label after the Today line)
- Test: `ui/src/screens/home/ScheduledCard.test.tsx` (new), `ui/src/screens/home/HomeMenu.test.tsx`, `ui/src/screens/home/Conversation.test.tsx`

**Interfaces:**
- Consumes: Task 4 (`sendsIn`, `waitingText`, `sentLaterLines`, `toLocalInput`, `fromLocalInput`, commands, keys); `topicTitle` (`home-topics.ts` :145); `useNow`; `localWeekdayTime` / `localDateTime`.
- Produces:
  - `ScheduledCards({ dotId, dotName, topics, root? })`: queries `keys.scheduledMessages(dotId)`. With `root`, it keeps only rows whose `root_run_id === root`. It renders one `ScheduledCard` per row, in the order the core gives. Load errors use `ErrorNote`.
  - `scheduledTarget(msg, topics, currentRoot?: string): string`. It gives `new topic` for no root, `current topic` when the root is the current chat topic's, `topicTitle(t)` for another topic shown, else `an older topic`.
  - `ScheduledCard({ message, dotName, target })`, `role="region"`, `aria-label="Scheduled message"`. It shows the top line, the dashed bubble, and the status line. The status line re-renders with `useNow(message.status === "scheduled")`: `failed` → `Not sent: …` (`text-needs`); `sending` → `Sending…`; `waiting` → `waitingText`; else `sendsIn`. Buttons `Send now` / `Edit` / `Cancel` are hidden while `sending`. Edit shows the form (`Scheduled message`, `Send at` = `toLocalInput(due_at)`, `Save`, `Discard changes`). Each mutation, on settle, invalidates `keys.scheduledMessages(dotId)`, `["runs"]` and `["thread"]`. Each press clears the others' errors. An error shows as `role="alert"` with `errorMessage`, and the form stays open.
  - `Transcript`: `sentLaterLines(run).map(line => <p className="text-xs text-muted-foreground self-end">…</p>)` under the bubble.
  - `DotMenu`: `useQuery(keys.scheduledMessages(dot.id))`, enabled while open. When `length > 0`, a `DropdownMenu.Label` reads `${n} scheduled`.

- [ ] **Step 1: Write the failing tests** (fixed `now` via `vi.setSystemTime`; times as ISO offsets from it).
  - `a scheduled card counts down`: due at now + 2h 13m 10s shows `Sends in 2h 14m`, the top line `Scheduled for <localWeekdayTime(due)> · current topic`, the text, and the three buttons. After `vi.advanceTimersByTime(2h 13m)` it shows `Sends in 1m`.
  - `the card names its target`: `new topic`, `topicTitle` of an older topic shown, `an older topic`.
  - `due states`: `waiting: busy` gives `Due · waiting for nightly to finish this topic`; `approval` and `usage_limit` give their texts; `sending` gives `Sending…` and no buttons; `failed` gives `Not sent: org removed` in `text-needs`.
  - `Send now, Edit and Cancel call the core`: `send_scheduled_now { id }`, `cancel_scheduled_message { id }`, and Edit → change the text → `Save` gives `edit_scheduled_message { id, text, dueAt }`, where `dueAt` is `fromLocalInput` of the field.
  - `an edit that loses shows why` (Review Focus 3): the stub rejects `edit_scheduled_message` with `This message has already been sent.`. The alert shows it and the form stays open. A later `scheduled_messages` without the row removes the card.
  - `an older topic shows only its own cards`: `root` set gives only the matching rows.
  - Conversation: a chat run with `send_later.late` shows `Sent late (was due …)` under its bubble. One with `new_topic` shows `Started a new topic: the one it was written in can't be continued.`.
  - Menu (`HomeMenu.test.tsx`): with two pending rows stubbed, opening the dot menu shows `2 scheduled`; with none, no such line.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement** with the Global Constraints copy. Use the dashed bubble `border border-dashed bg-bubble/60`, and the warning token `text-needs` as `DeliveryLine` does.
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): a scheduled message waits as a card with a countdown, and the dot menu counts them`.

### Task 6: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps: `cargo fmt --all -- --check`; `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`; `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times` (twice); `cargo test -p dots-app --locked`; `node --test tools/release/release.test.mjs`; `cd ui; bun run typecheck; bun run test`. Expected: all pass, in both core runs.
- [ ] **Step 2: Check by hand** in the running debug app (`cargo tauri dev` or however the earlier plans ran it): schedule "In 1 hour", see the card and `1 scheduled`, then Send now. Schedule a custom time 2 minutes ahead, close Dots for 3 minutes, start it, and see `Sent late (was due …)`.
- [ ] **Step 3: Push** `feat/send-later` and open the PR with base `feat/snooze-done`. In the body, say it is stacked on the snooze/done PR, that it adds migration 0032, that attachments (0033) builds on `ChatExtra` / `chat_extra_for` / `release_staged`, and list the spec's builder's calls (§3.1).
- [ ] **Step 4: After the merge only**, follow `CLAUDE.local.md`'s standing rule to clean the Rust build output.
