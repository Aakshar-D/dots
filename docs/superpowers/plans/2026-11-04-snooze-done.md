# Snooze and Done on the Inbox — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every Inbox item can be snoozed until a preset or picked time, and every item except an approval can be marked done. Hidden items leave the Inbox and every count. A snoozed item comes back at its time, also after a restart, with a notification and a "Back from snooze" tag at the top. Done items stay listed for 7 days with Undo.

**Architecture:** One new table, `inbox_marks` (migration 0031), keyed by an item's kind and id. A new core module `inbox` holds the key rules and titles. `Runtime::inbox` builds the full Inbox, then leaves out hidden items and adds `back`, `snoozed` and `done` lists and a new `not_delivered` section. The store's count queries skip hidden items. A wake pass on the runtime's `Clock` prunes marks of items that left and wakes due snoozes. The app starts it and turns its event into one notification. The UI gets a shared presets helper (`later.ts`), its own copy of the key rules (checked against the core through the contract fixture), a Snooze menu, and the new Inbox sections.

**Tech Stack:** Rust (tokio, sqlx/SQLite, chrono), Tauri 2, React + TypeScript (zod, vitest, TanStack Query, radix-ui DropdownMenu and Dialog).

**Spec:** `docs/superpowers/specs/2026-11-04-snooze-done-design.md` (§3.9 holds the builder's calls).

## Global Constraints

- Branch `feat/snooze-done` from `feat/delivery-tracking`. Migration `0031_inbox_marks.sql`: the `CREATE TABLE` and `CREATE INDEX` from spec §4.2, verbatim.
- `InboxKind` values: `message` | `approval` | `review` | `not_delivered` | `attention` | `watcher` | `held` | `tools` | `instructions` | `app_review` | `team_scope`. The ids are as in spec §4.1: watcher `<watcher_id>@<paused_at>`, tools `<server_id>/<scope>`, team_scope `<team_id>/<dot_id>`, attention the failed run's id, and the item's own id for the rest.
- Titles (spec §4.1): first line of any text, then the whole title is cut to 80 chars with `…`.
- Hidden = a mark with `snoozed_until` or `done_at` set. A back mark (`back_at`) is shown and counted.
- Done list = done marks with `done_at >= now - 7 days`. Older done marks stay and keep hiding.
- Wake pass: once at start, then every **30 s** of the `Clock`. Snooze `until`: after now and at most **365 days** ahead.
- Error texts, verbatim: `Pick a time in the future.` (`Invalid`), `That is no longer in the Inbox.` (`NotFound`), `An approval is decided, not marked done.` (`Conflict`), `That is not snoozed.` / `That is not done.` (`NotFound`).
- Notification: one item → title `Back from snooze: <title>`, body `It is at the top of your Inbox.`. Several items → title `Back from snooze: <n> items`, body the titles joined with `; ` and cut to 120 chars. Route `/inbox`. Never quiet.
- UI copy, verbatim: buttons `Snooze`, `Done`, `Unsnooze`, `Undo`; menu `In 1 hour`, `This evening (6 PM)`, `Tomorrow morning (9 AM)`, `Next week (Monday 9 AM)`, `Pick a date and time…`; dialog title `Snooze until`, field `Date and time`, buttons `Snooze` and `Cancel`, error `Pick a time in the future.`; headings `Back from snooze`, `Not delivered`; tag `Back from snooze`; toggles `Snoozed (<n>)` and `Done (<n>)`; rows `<title> · Back <when>` and `<title> · Done <relative>`.
- Presets (local time): +60 min; today 18:00, left out when now ≥ 18:00; next calendar day 09:00; the next Monday after today at 09:00 (a Monday gives the following Monday).
- Rust builds on this machine need `C:\Program Files\LLVM\bin` on PATH. Never commit `CLAUDE.local.md` or `.cargo/config.toml`. No AI trailer in commits.
- Tests must not depend on same-millisecond row order (give ids or times explicitly, and assert by key). They must not depend on a dispatcher claiming runs: call `rt.hold_runs(true)` before creating runs. They must not read before an async writer finishes (await the command, or wait for the bus event). They must not depend on the real clock: use `FakeClock` in core, and fixed `Date`s built with the local constructor (`new Date(2026, 10, 4, 18, 30)`) in UI.

## Review Focus

1. **An item that left on its own while snoozed, whose time then passes** (a question answered in the conversation): no notification, not in any list, and its mark is gone after the pass. A watcher resumed and paused again is not hidden by its old mark. Tests `a_snoozed_item_that_left_never_comes_back` (Task 3) and `a_watcher_paused_again_is_shown` (Task 2).
2. **Dots closed past a snooze time, then started**: the first pass wakes the item at once and sends one `SnoozeEnded`. The next pass sends none. Test `snoozes_due_while_closed_wake_at_start` (Task 3).
3. **Counts for every source while hidden, and again once back**: a dot's needs-you and unread (a hidden note, and a hidden run to review that ended after `seen_at`), `inbox_count`, and the rail. Tests `dot_counts_leave_out_hidden_items` (Task 1), `inbox_count_leaves_out_hidden_items` (Task 2), `the_rail_count_leaves_out_a_snoozed_message` (Task 6).
4. **Presets at their edges**: at 18:30 "This evening" is left out; at 17:59 it is 1 minute ahead; on Monday "Next week" is 7 days ahead; on 31 Dec "Tomorrow morning" is 1 Jan. Test `laterPresets_at_the_edges` (Task 5).
5. **An approval, which is never done but can be snoozed**: Done is refused in core even when the UI is bypassed. A snoozed approval still parks the run, and deciding it in the conversation takes it out of the Snoozed list. Tests `an_approval_can_be_snoozed_but_not_done` and `deciding_a_snoozed_approval_takes_it_away` (Task 2), `approvals_have_no_done` (Task 6).

---

### Task 1: Storage — the marks table, its writes, and counts that skip hidden items

**Files:**
- Create: `crates/dots-core/migrations/0031_inbox_marks.sql`
- Create: `crates/dots-core/src/inbox.rs` (register `pub mod inbox;` in `lib.rs`)
- Create: `crates/dots-core/src/store/inbox_marks.rs` (register in `store/mod.rs`)
- Modify: `crates/dots-core/src/store/dots.rs` (`dot_counts` :338)
- Modify: `crates/dots-core/src/store/messages.rs` (`count_waiting_messages` :166; new `waiting_messages` next to `list_messages` :147)
- Modify: `crates/dots-core/src/store/runs.rs` (new `undelivered_chat_runs` next to `unreviewed_thread_ends` :322)
- Test: `crates/dots-core/tests/store_inbox_marks_test.rs` (new)

**Interfaces:**
- Produces (in `inbox.rs`):
  - `str_enum!(InboxKind { Message => "message", Approval => "approval", Review => "review", NotDelivered => "not_delivered", Attention => "attention", Watcher => "watcher", Held => "held", Tools => "tools", Instructions => "instructions", AppReview => "app_review", TeamScope => "team_scope" })`.
  - `#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)] pub struct InboxKey { pub kind: InboxKind, pub id: String }`.
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct InboxMark { pub kind: InboxKind, pub id: String, pub title: String, pub dot_id: Option<String>, pub snoozed_until: Option<String>, pub done_at: Option<String>, pub back_at: Option<String> }`. The store fills `title` with `""` and `dot_id` with `None`, and Task 2 fills them.
  - `pub fn hidden_sql(kind: InboxKind, id_expr: &str) -> String`: `NOT EXISTS (SELECT 1 FROM inbox_marks k WHERE k.kind = '<kind>' AND k.item_id = <id_expr> AND (k.snoozed_until IS NOT NULL OR k.done_at IS NOT NULL))`. Use it in every count below.
- Produces (store):
  - `Store::snooze_marks(&self, keys: &[InboxKey], until: &str, now: &str) -> Result<()>`: one transaction, an upsert per key that sets `snoozed_until` and clears `done_at` and `back_at`.
  - `Store::done_mark(&self, key: &InboxKey, now: &str) -> Result<()>`: upsert `done_at`, clearing the other two.
  - `Store::delete_mark_if(&self, key: &InboxKey, state: MarkState) -> Result<bool>`, with `pub enum MarkState { Snoozed, Done }`. It deletes only a mark in that state.
  - `Store::list_marks(&self) -> Result<Vec<InboxMark>>`; `Store::count_marks(&self) -> Result<usize>`.
  - `Store::prune_marks(&self, keep: &HashSet<InboxKey>) -> Result<usize>`: deletes marks whose key is not in `keep`. It reads the keys first, then deletes by key, in one transaction.
  - `Store::wake_due(&self, now: &str) -> Result<Vec<InboxKey>>`: `UPDATE … SET back_at = ?, snoozed_until = NULL WHERE snoozed_until <= ? RETURNING kind, item_id`.
  - `Store::waiting_messages(&self) -> Result<Vec<Message>>`: the `WAITS` rows only, ordered as `list_messages`.
  - `Store::undelivered_chat_runs(&self) -> Result<Vec<Run>>`: `trigger_kind = 'chat'`, `undelivered_at` set, `dismissed_at` and `delivered_at` null, ordered `undelivered_at DESC, id DESC`.
  - `dot_counts`: needs-you terms use `hidden_sql` with `message` (`m.id`), `approval` (`a.id`), `watcher` (`w.id || '@' || w.paused_at`), `app_review` (`p.id`) and `not_delivered` (`u.id`). The unread terms use `message` for notes, and for ended runs both `review` (`r.id`) and `not_delivered` (`r.id`). The quiet-run term is unchanged.
  - `count_waiting_messages`: `WAITS AND <hidden_sql(message, m.id)>`.

- [ ] **Step 1: Write the failing tests** (store only, explicit ids and times, no runner):
  - `snooze_then_done_then_undo`: `snooze_marks` gives one mark with `snoozed_until` only. `done_mark` on it gives `done_at` only. `delete_mark_if(Snoozed)` returns `false`, and `delete_mark_if(Done)` returns `true` and leaves no mark.
  - `a_mark_is_in_one_state`: a raw `INSERT` with both `snoozed_until` and `done_at` fails on the check constraint.
  - `wake_due_wakes_only_what_is_due`: two snoozes until `T+1m` and `T+2m`. `wake_due(T+1m)` returns the first only, which then has `back_at == T+1m` and no `snoozed_until`. A second call returns nothing.
  - `prune_marks_keeps_only_given_keys`: three marks, keep two → returns 1.
  - `dot_counts_leave_out_hidden_items` (Review Focus 3): a dot with an open question, a pending approval, a paused watcher, a pending app proposal and an undelivered chat run. `seen_at` is set, then a note and a failed run end after it. Record the counts first. Snoozing each needs-you item drops needs-you by 1 and unread by 1. Done on the note and on the failed run (as `review`) drops unread by 1 each. `wake_due` past the snooze restores the counts. Marking a watcher key with an old `paused_at` does not hide the same watcher paused again.
  - `waiting_messages_are_the_ones_that_wait` and `count_waiting_messages_leaves_out_hidden_ones`.
  - `undelivered_chat_runs_lists_only_live_marks`: dismissed, delivered and background runs are left out.
  - `migration_0031_creates_the_table`: migrate to `< 31` (pattern `store_quiet_test.rs:49`), open the `Store`, `count_marks() == 0`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test store_inbox_marks_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p dots-core --test store_inbox_marks_test --test store_dots_test --test store_delivery_test --test checkins_test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): an Inbox item can be marked snoozed or done, and counts skip it`.

### Task 2: The Inbox — keys, titles, filtered sections, Not delivered, snooze and done

**Files:**
- Modify: `crates/dots-core/src/inbox.rs` (keys and titles)
- Modify: `crates/dots-core/src/model.rs` (`Inbox` :660; new `UndeliveredMessage`)
- Modify: `crates/dots-core/src/runtime.rs` (`inbox` :742 split into `full_inbox` + filter; `inbox_count` :809; new methods after `dismiss_schedule_attention`)
- Modify: `crates/dots-core/src/events.rs` (`RuntimeEvent` :10: `InboxUpdated`, `SnoozeEnded`)
- Modify: `apps/dots-app/src/contract.rs` `Inbox` sample :1183 (new fields empty, so it still builds)
- Test: `crates/dots-core/tests/runtime_inbox_marks_test.rs` (new)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct UndeliveredMessage { #[serde(flatten)] pub run: Run, pub dot_name: String }`.
  - `Inbox` gains `#[serde(default)]` fields: `not_delivered: Vec<UndeliveredMessage>`, `back: Vec<InboxMark>`, `snoozed: Vec<InboxMark>`, `done: Vec<InboxMark>`.
  - `pub struct InboxEntry { pub key: InboxKey, pub dot_id: Option<String>, pub title: String }` and `pub fn entries(inbox: &Inbox, messages: &[Message]) -> Vec<InboxEntry>` (`inbox.rs`): one entry per item in the order of the sections, using spec §4.1's ids and titles. `pub fn title_cut(text: &str) -> String` (first line, 80 chars, `…`).
  - `RuntimeEvent::InboxUpdated` (no fields) and `RuntimeEvent::SnoozeEnded { items: Vec<InboxMark> }`.
  - `Runtime::full_inbox(&self) -> Result<(Inbox, Vec<Message>)>` (private): today's body plus `not_delivered` from `undelivered_chat_runs`, `review_runs` without those run ids, and `waiting_messages`.
  - `Runtime::inbox(&self) -> Result<Inbox>`: drops hidden items from every section. `back`, `snoozed` and `done` hold marks whose key is in `entries(..)`, with `title` and `dot_id` from the entry, in spec §4.3's order. The 7-day cut is from `self.clock.now()`.
  - `Runtime::inbox_count`: today's sum over the filtered sections, plus `not_delivered.len()`, plus `store.count_waiting_messages()` (now without hidden ones).
  - `Runtime::inbox_snooze(&self, items: &[InboxKey], until: DateTime<Utc>) -> Result<()>`, `inbox_done(&self, item: &InboxKey) -> Result<()>`, `inbox_unsnooze(&self, item: &InboxKey) -> Result<()>`, `inbox_undo_done(&self, item: &InboxKey) -> Result<()>`. They use the checks and texts in Global Constraints (presence = the key is in `entries(full_inbox)`), and each sends `InboxUpdated` after the write.

- [ ] **Step 1: Write the failing tests** (`Runtime::start` with `cfg.clock = FakeClock::new(t0)` and `hold_runs(true)`; items built through the store: `add_question`, `add_note`, approvals with `create_approval`, a paused watcher, a failed run, an undelivered chat run):
  - `every_kind_has_its_key_and_title`: `entries` over a built Inbox gives the §4.1 keys. Titles: `courier asks: Ship it?`, `courier says: Half the tests pass.`, `courier asks to use Bash`, `courier didn't get your message`. An 81-char line is cut to 80 with `…`.
  - `a_snoozed_item_leaves_its_section_and_shows_in_snoozed`: one per kind that has a section (approval, review, attention, watcher, instructions, app_review, not_delivered). The `snoozed` list holds marks with titles, soonest first.
  - `done_items_show_for_seven_days`: done at `t0`; at `t0 + 7d - 1ms` it is listed; at `t0 + 7d + 1ms` it is not listed and is still hidden.
  - `an_approval_can_be_snoozed_but_not_done` (Review Focus 5): `inbox_done(approval)` → `Conflict("An approval is decided, not marked done.")`. `inbox_snooze` works, and the run stays `awaiting_approval`.
  - `deciding_a_snoozed_approval_takes_it_away` (Review Focus 5): snooze, then `rt.decide_approval(..)` → absent from `snoozed`.
  - `snooze_checks_its_input`: empty items → `Invalid`; `until == now` and `until = now + 366d` → `Invalid("Pick a time in the future.")`; an unknown key → `NotFound("That is no longer in the Inbox.")`, and nothing is written for the other keys of the same call. `inbox_unsnooze` with no snooze → `NotFound("That is not snoozed.")`. `inbox_undo_done` with no done → `NotFound("That is not done.")`.
  - `unsnooze_and_undo_put_it_back_without_a_tag`: the item is back in its section, and `back` is empty.
  - `a_watcher_paused_again_is_shown` (Review Focus 1): snooze a paused watcher, resume it, pause it again (new `paused_at`) → listed, not hidden.
  - `not_delivered_has_its_own_section`: a failed undelivered chat run is in `not_delivered` and not in `review_runs`. After `discard_delivery` it is in neither.
  - `inbox_count_leaves_out_hidden_items` (Review Focus 3): count before, after snoozing a question, after done on a review run, and after snoozing every held run (the held line goes) or only one (it stays).
  - `each_change_sends_inbox_updated`: subscribe, then snooze → one `InboxUpdated`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test runtime_inbox_marks_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p dots-core --test runtime_inbox_marks_test --test runtime_workspaces_test --test runtime_delivery_test --test runtime_checkins_test` and `cargo build -p dots-app --tests`. Expected: PASS / compiles.
- [ ] **Step 5: Commit** `feat(core): snooze or mark done any Inbox item, and list not-delivered messages`.

### Task 3: The wake sweep

**Files:**
- Modify: `crates/dots-core/src/runtime.rs` (new `wake_snoozed`, `spawn_snooze_waker`), `crates/dots-core/src/inbox.rs` (`pub const WAKE_EVERY: TimeDelta = TimeDelta::seconds(30); pub const DONE_LISTED_FOR: TimeDelta = TimeDelta::days(7);`)
- Test: `crates/dots-core/tests/inbox_waker_test.rs` (new)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces:
  - `Runtime::wake_snoozed(&self) -> Result<Vec<InboxMark>>`: `count_marks() == 0` → `Ok(vec![])`. Otherwise `full_inbox` → `entries` → `prune_marks(keys)` → `wake_due(timestamp(clock.now()))`. If anything was pruned or woken, send `InboxUpdated`. If anything woke, send `SnoozeEnded { items }` with each woken mark's title and `dot_id`. Return the woken marks.
  - `Runtime::spawn_snooze_waker(self: &Arc<Self>) -> JoinHandle<()>`: holds a `Weak<Runtime>`. It runs a pass at once, then loops `select! { shutdown.cancelled(), clock.sleep_until(clock.now() + WAKE_EVERY) }` and passes again. It stops when the upgrade fails. Errors are logged with `tracing::warn!("waking snoozed items failed: {e}")`, and the loop goes on.

- [ ] **Step 1: Write the failing tests** (`FakeClock::new(t0)`, `Arc<Runtime>`):
  - `a_snooze_wakes_at_its_time_and_not_before`: snooze a question until `t0 + 1h`. A pass at `t0 + 59m` returns nothing, and the item is in `snoozed`. Then `clock.set(t0 + 1h)`: the pass returns one mark titled `courier asks: Ship it?`, `inbox().back` has it, the question is in the counts again, and the bus saw one `InboxUpdated` then one `SnoozeEnded`.
  - `several_wake_in_one_event`: three due → one `SnoozeEnded` with 3 items.
  - `a_snoozed_item_that_left_never_comes_back` (Review Focus 1): snooze, answer the question with `rt.reply(message_id, "yes")`, `clock.set` past the time → the pass returns nothing, no `SnoozeEnded`, `count_marks() == 0`.
  - `snoozes_due_while_closed_wake_at_start` (Review Focus 2): snooze until `t0 + 1h`, shut down and drop. Start a new runtime on the same data dir with a `FakeClock` at `t0 + 3h`. Subscribe, `spawn_snooze_waker`, and receive `SnoozeEnded` within 5 s (bus `recv` with a timeout). Then `clock.nth_sleep(1)`, `clock.set(t0 + 3h + 30s)`, and a pass sends no second `SnoozeEnded`.
  - `the_waker_runs_on_the_clock`: spawn; `clock.nth_sleep(1)`; snooze until `t0 + 10s`; `clock.set(t0 + 30s)` → `SnoozeEnded` arrives.
  - `no_marks_no_work`: with no marks a pass sends nothing.
- [ ] **Step 2: Run** `cargo test -p dots-core --test inbox_waker_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the new file twice, then the full `dots-core` suite once (`cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): snoozed items come back at their time, also after a restart`.

### Task 4: App — commands, the waker, the notice, the contract

**Files:**
- Modify: `apps/dots-app/src/commands.rs` (next to `inbox` :709), `apps/dots-app/src/lib.rs` (`command_list!` after `inbox_count` :115; setup after `events::spawn_forwarder` :292)
- Modify: `apps/dots-app/src/events.rs` (`Notice` :31, `Notifier::decide`, `notice_text`, `notice_route`, `notify` :375, tests)
- Modify: `apps/dots-app/src/contract.rs` (`Inbox` sample :1183, events list ~:1359, `Enums` :1293)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Consumes: Tasks 2–3.
- Produces:
  - Commands `inbox_snooze(items: Vec<InboxKey>, until: String) -> Res<()>` (parse RFC 3339, else `Invalid("Pick a time in the future.")`), `inbox_unsnooze(item: InboxKey) -> Res<()>`, `inbox_done(item: InboxKey) -> Res<()>`, `inbox_undo_done(item: InboxKey) -> Res<()>`.
  - Setup: `rt.spawn_snooze_waker();` right after `events::spawn_forwarder(app.clone(), rt.clone())`.
  - `Notice::BackFromSnooze { items: Vec<InboxMark> }`. `decide`: `SnoozeEnded { items }` with items → this notice. `notice_text` gives the Global Constraints texts. `notice_route` gives `/inbox`. In `notify` its dot is `None`, and the route match treats it like `Hold` (`notice_route(notice, "")`).
  - Contract: `samples["Inbox"]` gains one `not_delivered` (the `RunNotDelivered` run, `dot_name: "nightly-tests"`), one `back`, one `snoozed` (`snoozed_until: TS`) and one `done` mark (titles from `entries`). `samples["InboxMark"]`, `samples["InboxKey"]`, `Enums.InboxKind = InboxKind::ALL`, events `InboxUpdated` and `SnoozeEnded`. `samples["InboxKeys"] = inbox::entries(&inbox_sample, &[message_sample]).keys`, from the same `Inbox` and `Message` samples the fixture holds.

- [ ] **Step 1: Write the failing tests** (events.rs `mod tests`):
  - `back_from_snooze_texts`: one item → `("Back from snooze: courier asks: Ship it?", "It is at the top of your Inbox.")`. Three items → `("Back from snooze: 3 items", "<t1>; <t2>; <t3>")`, cut at 120 chars.
  - `back_from_snooze_opens_the_inbox`: route `/inbox`.
  - `snooze_ended_notifies_once_per_event`: `decide(SnoozeEnded{..})` → `Some(BackFromSnooze)`. `decide(InboxUpdated)` → `None`.
- [ ] **Step 2: Run** `cargo test -p dots-app events`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands, the setup line, the notice and the samples.
- [ ] **Step 4: Regenerate and run**: `$env:UPDATE_CONTRACT=1; cargo test -p dots-app contract; Remove-Item Env:UPDATE_CONTRACT; cargo test -p dots-app`. Expected: PASS. The UI contract test fails until Task 5 updates `types.ts`. That is expected here.
- [ ] **Step 5: Commit** `feat(app): snooze and done commands, and a notice when an item is back`.

### Task 5: UI — types, presets, item keys, the Snooze menu

**Files:**
- Modify: `ui/src/api/types.ts` (`Inbox` :687 new fields; `InboxKind`, `InboxKey`, `InboxMark`, `UndeliveredMessage`; `RuntimeEvent` :487 `inbox_updated`, `snooze_ended`), `ui/src/api/commands.ts` (after `inboxCount` :200), `ui/src/test/fixtures.ts` (default `inbox` :123: empty `not_delivered`, `back`, `snoozed`, `done`)
- Create: `ui/src/lib/later.ts`, `ui/src/lib/inbox.ts`, `ui/src/components/LaterMenu.tsx`, `ui/src/components/InboxActions.tsx`
- Test: `ui/src/lib/later.test.ts`, `ui/src/lib/inbox.test.ts`, `ui/src/components/InboxActions.test.tsx`

**Interfaces:**
- Consumes: Task 4 contract.
- Produces:
  - `later.ts`: `export type LaterId = "hour" | "evening" | "morning" | "next_week"`; `export interface LaterPreset { id: LaterId; label: string; at: Date }`; `export function laterPresets(now: Date): LaterPreset[]` (Global Constraints rules and snooze labels, in menu order, `evening` left out from 18:00); `export function isLater(at: Date, now: Date): boolean` (strictly after now and ≤ 365 days); `export function fromPicker(value: string): Date | null` (a `datetime-local` value read as local time); `export function toPicker(d: Date): string`; `export function backText(iso: string, now: number): string` (`Back ` + `localWeekdayTime` within 6 days, else `localDateTime`). Send later reuses all of these and maps `next_week` to its own label.
  - `inbox.ts`: `keyString(k: InboxKey): string` (`kind:id`); one key function per kind, mirroring core: `messageKey(m)`, `approvalKey(a)`, `reviewKey(r)`, `notDeliveredKey(r)`, `attentionKey(a)` (id `a.run_id`), `watcherKey(w)` (`${watcher_id}@${at}`), `heldKey(h)`, `toolsKey(t)` (`${server_id}/${scope}`), `instructionsKey(i)`, `appReviewKey(r)`, `teamScopeKey(t)`; `inboxKeys(inbox: Inbox, messages: Message[]): InboxKey[]` in the core's section order; `hiddenKeys(inbox): Set<string>` (snoozed ∪ done); `backKeys(inbox): Set<string>`; `inboxWaiting(inbox: Inbox | undefined, messages: Message[] | undefined): number` (the sections' lengths, `held` as at most 1, `team_scope` and `not_delivered` included, plus `messages.filter(waits)` minus hidden ones).
  - `<LaterMenu label="Snooze" onPick={(at: Date) => void} disabled? />`: a radix `DropdownMenu` with the presets from `laterPresets(new Date())` (read when it opens), and `Pick a date and time…`, which opens a `Dialog` (`Snooze until`, `Date and time`, `Snooze`/`Cancel`, the error when `!isLater`).
  - `<InboxActions items={InboxKey[]} done={boolean} />`: `LaterMenu` → `commands.inboxSnooze(items, at.toISOString())`; when `done`, a `Done` button → `commands.inboxDone(items[0])`. Errors go to `notifyError`. On settle it invalidates `keys.inbox` and `keys.overview`.
  - Commands: `inboxSnooze(items, until)`, `inboxUnsnooze(item)`, `inboxDone(item)`, `inboxUndoDone(item)` (all `z.null()`).

- [ ] **Step 1: Write the failing tests.**
  - `laterPresets_at_the_edges` (Review Focus 4). Wed 4 Nov 2026 10:00 → 4 presets: 11:00 today, 18:00 today, 5 Nov 09:00, Mon 9 Nov 09:00, with the Global Constraints labels. 17:59 → `evening` is 18:00 (1 min ahead). 18:00 and 18:30 → no `evening`. Mon 9 Nov 08:00 → `next_week` is Mon 16 Nov 09:00. Thu 31 Dec 2026 22:00 → `morning` is 1 Jan 2027 09:00. Assert with local getters (`getDate()`, `getHours()`).
  - `isLater`: now → false; now + 1 min → true; now + 366 d → false. `fromPicker("2026-11-05T09:00")` has local hours 9. `fromPicker("")` → `null`.
  - `inboxKeys_match_the_core`: `inboxKeys(T.Inbox.parse(contract.Inbox), [T.Message.parse(contract.Message)])` deep-equals `contract.InboxKeys`.
  - `inboxWaiting`: the fixture Inbox with one waiting message → the expected sum. Putting that message's key in `snoozed` lowers it by 1. A `done` key lowers it too.
  - `InboxActions`: `Snooze` → `Tomorrow morning (9 AM)` calls `inbox_snooze` with `{ items, until }`, where `until` is tomorrow 09:00 local as ISO. `Pick a date and time…` with a past value shows `Pick a time in the future.`, and the dialog's `Snooze` is disabled. `Done` calls `inbox_done` with `{ item }`. With `done={false}` there is no `Done` button. A rejected call shows the toast.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS, the contract test included.
- [ ] **Step 5: Commit** `feat(ui): later presets, Inbox item keys and the Snooze menu`.

### Task 6: UI — the Inbox screen, the rail count, live updates

**Files:**
- Modify: `ui/src/screens/InboxScreen.tsx` (every row: actions; new sections), `ui/src/components/MessageCard.tsx` (optional `actions?: ReactNode` prop shown in its action row), `ui/src/app/Layout.tsx` (:46–69 → `inboxWaiting`), `ui/src/lib/runtime-events.ts` (`inbox_updated`, `snooze_ended`; `message_posted`/`message_updated` unchanged)
- Create: `ui/src/screens/inbox/MarkedList.tsx` (the Snoozed and Done toggles), `ui/src/screens/inbox/NotDeliveredRow.tsx`
- Test: `ui/src/screens/InboxSnooze.test.tsx` (new), `ui/src/lib/runtime-events.test.ts`, `ui/src/app/Look.test.tsx` or `Layout.test.tsx` (rail count)

**Interfaces:**
- Consumes: Task 5.
- Produces:
  - `InboxScreen`: Messages are `messages.filter(waits)` minus `hiddenKeys(inbox)`. Each row or card gets `<InboxActions items={[key]} done />`. An approvals group gets `<InboxActions items={group.approvals.map(approvalKey)} done={false} />` in its header. Back items (by `backKeys`) are taken out of their sections and rendered first, under `Back from snooze`, in `inbox.back` order, each with `<Badge>Back from snooze</Badge>`. Back approvals are grouped with `byRun`. Section order after that: Messages, Approvals, Not delivered, then today's order. `MarkedList` comes last, Snoozed then Done.
  - `NotDeliveredRow({ item: UndeliveredMessage })`: the dot name, `oneLine(chatText(run))`, `<DeliveryLine run={item} />`, `Open` → `/chat/${dot_id}/${id}`.
  - `MarkedList({ title: "Snoozed" | "Done", marks: InboxMark[] })`: a button with `aria-expanded`, text `Snoozed (n)` / `Done (n)`, closed at first (`useState`). Rows `${title} · ${backText(snoozed_until, now)}` with `Unsnooze`, or `${title} · Done ${relativeTime(done_at)}` with `Undo`. Hidden when `marks` is empty.
  - The empty state shows when nothing is shown (hidden items do not count). It uses the spec §6 copy, and `MarkedList`s render below it.
  - `applyRuntimeEvent`: `inbox_updated` → invalidate `keys.inbox` and `keys.overview`. `snooze_ended` → nothing.

- [ ] **Step 1: Write the failing tests** (`InboxSnooze.test.tsx`, the backend stubs keep marks the way the core does):
  - `snoozing_a_question_moves_it_to_snoozed`: `Snooze` → `In 1 hour` on a question card. After the stubbed `inbox` returns it in `snoozed`, the Messages group is gone and `Snoozed (1)` shows. Expanding it shows `courier asks: Ship it? · Back <time>`. `Unsnooze` calls `inbox_unsnooze`.
  - `done_moves_it_to_done_with_undo`: `Done` on a paused watcher → `Done (1)` → `Undo` calls `inbox_undo_done` with the watcher key `w1@<at>`.
  - `approvals_have_no_done` (Review Focus 5): the approvals group shows `Snooze` and no `Done`, and its snooze sends every approval id of the group.
  - `back_items_come_first_with_their_tag`: an Inbox with a review run in `back` → the first region is `Back from snooze`, holding that row with the tag. The row is not repeated under `Runs to review`.
  - `not_delivered_section_lists_the_message`: the row shows `Not delivered: claude CLI not found`, `Retry` and `Discard`.
  - `everything_hidden_shows_the_empty_state_and_the_lists`.
  - `the_rail_count_leaves_out_a_snoozed_message` (Review Focus 3): rail `Inbox, 1 waiting` → with that message's key in `snoozed` → `Inbox` (no count).
  - runtime-events: `inbox_updated` invalidates `["inbox"]` and `["overview"]`.
- [ ] **Step 2: Run** `cd ui; bun run test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Existing Inbox tests (`InboxScreen`, `InboxMessages`, `InboxHeld`, `InboxWatchers`, `InboxAppReviews`) must pass unchanged, apart from new buttons in role queries that now match more than one element. Fix those by naming the button more exactly, not by removing the actions.
- [ ] **Step 4: Run** `cd ui; bun run typecheck; bun run test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): snooze and done on every Inbox item, with Back from snooze, Snoozed and Done`.

### Task 7: Whole-branch check and pull request

- [ ] **Step 1: Run** CI's steps: `cargo fmt --all -- --check`; `cargo clippy -p dots-core -p dots-app --all-targets -- -D warnings`; `cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule_lists_the_next_fire_times` (twice); `cargo test -p dots-app --locked`; `node --test tools/release/release.test.mjs`; `cd ui; bun run typecheck; bun run test`. Expected: all pass, in both core runs.
- [ ] **Step 2: Commit the spec and plan** to `docs/superpowers/specs/2026-11-04-snooze-done-design.md` and `docs/superpowers/plans/2026-11-04-snooze-done.md` (`docs: snooze and done spec and plan`).
- [ ] **Step 3: Push** `feat/snooze-done` and open the PR with base `feat/delivery-tracking`. In the body, say that it is stacked on delivery tracking, that it adds migration 0031 (0030 is topic-tools), that Send later stacks on it and reuses `later.ts` and `LaterMenu`, and list the builder's calls in spec §3.9.
