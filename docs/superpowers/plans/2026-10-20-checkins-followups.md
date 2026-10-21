# Check-ins and Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dot can tell the person something while it works (`notify`), ask them a question and park until they answer (`ask`), and schedule a run of its own that continues its thread later (`follow_up`), on both engines, with the answers, notes and follow-ups visible in the Inbox, the chat, the run view, the Dots list and the dot editor.
**Architecture:** Two tables (`messages`, `follow_ups`, migration 0008) with their store files and model types. `checkins.rs` holds the three tools' descriptions and input rules and `CheckIns`, which performs an allowed call and answers a reply. `followups.rs` holds the limits and `FollowUps`, the loop that fires due follow-ups on the scheduler's `Clock`. Runtime's `chat_lock` becomes the `Arc` thread lock that `chat`, `reply` and the loop share. The Claude engine reaches the tools on Dots' own `dots` MCP server (`server/mcp.rs`, checked by the hub under `Notify`, `Ask`, `FollowUp`, as `context` is); the local engine has them built in (`engine/local/tools/checkins.rs`). Every preset allows the three policy names after `Skill` and `Context`; `finish_0008` adds them to stored non-Custom policies. The app gains six commands, three events, a notification and the setting `notify_on_notes`; the UI an Inbox "Messages" group, a chat reply bar, run-view lines, a Follow-ups tab and a Settings switch.
**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio, axum, chrono 0.4.45, Tauri 2.12; Bun 1.3, Vite 8.3, React 19.3, TypeScript 6.0 (strict), TanStack Query 5.104, zod 4.6, Vitest 5.0 with Testing Library and jsdom. No new dependency.
**Spec:** docs/superpowers/specs/2026-10-20-checkins-followups-design.md

## Global Constraints

- Repository root `C:/Users/shrad/OneDrive/Documents/LOQ/Dots/dots`, branch `feat/checkins-followups`; all commands run from the root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .`. No `Co-Authored-By` or other AI trailer in any commit message.
- Toolchain: Rust 1.95 stable; Bun 1.3 (`bun x`, not `bunx`); Windows 11, Git Bash. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first.
- A parallel developer changes this repository (approvals, scheduler). Shared files get list entries, match arms, one-line calls and the `Arc`'d thread lock only; logic goes in the new modules `checkins.rs`, `followups.rs`, `store/messages.rs`, `store/follow_ups.rs`, `engine/local/tools/checkins.rs` and new UI files. `scheduler.rs` is not changed.
- One migration, `0008_checkins.sql` (Task 1; Task 2 appends to it). Before the branch is merged: `git fetch origin && git ls-tree --name-only origin/master crates/dots-core/migrations/`; if `0008_*` is taken, rename the file to the next free number and rename `upgrade_0008` and `finish_0008` with it.
- Text of `notify.text`, `ask.question`, `follow_up.note`: 1 to 2,000 characters, counted with `chars()` after `trim()`; blank is refused; the trimmed text is stored.
- `ask.choices`: optional; an array of 1 to 5 strings, each 1 to 80 characters after `trim()`.
- `follow_up`: exactly one of `at` (RFC 3339 with an offset) or `in` (digits and one of `m`, `h`, `d`, e.g. `90m`, `2h`, `1d`); a blank `at` or `in` counts as absent. Limits, checked in this order against the follow-up loop's clock: at least 5 minutes ahead (`MIN_AHEAD`), at most 30 days ahead (`MAX_AHEAD`), fewer than 3 `pending` follow-ups for the dot (`MAX_PENDING`). Constants in `followups.rs`, not settings.
- One question per run, checked by the insert itself; the 3-pending limit is checked by the insert itself (one SQL statement each), so parallel calls cannot pass them together.
- A follow-up run that cannot be stored: tried again after 1, 5, then 30 minutes (`RETRY_DELAYS`); the fourth failed start skips it and posts a note.
- A follow-up that fires more than 15 minutes (`LATE_AFTER`) after `due_at` appends ` (this is <n> hours late; Dots was not running at the time)` to its message's first line (`hour` when n is 1; n is the lateness rounded to whole hours, at least 1).
- Exact sentences (tool results and errors): `Sent.`; `Your question was sent. Finish your turn now; the person's answer will arrive as your next message.`; `You already asked a question in this run; finish your turn now.`; `Follow-up <id> scheduled for <%a %-d %b %H:%M local>.`; `A follow-up must be at least 5 minutes away.`; `A follow-up must be at most 30 days away.`; `This dot already has 3 pending follow-ups; that is the most it may have.`; `Not delivered: <reason>.`; `this question has already been answered`; `this question was dismissed`; and `chat`'s busy sentences, reused unchanged.
- Run messages: a reply with a session `Answer to your question "<question>": <text>`; without one `Earlier you asked the person: "<question>"\nTheir answer: <text>`; a follow-up `Follow-up you scheduled on <created_at local>: <note>`, and without a session a second line `What the earlier run reported: <summary or "nothing">`.
- Policy names `Notify`, `Ask`, `FollowUp` (`policy::CHECK_IN_TOOLS`): every preset allows them after `Skill` and `Context`; their rules never reach the CLI's `--allowedTools` / `--disallowedTools`; Claude runs always get `--allowedTools mcp__dots__notify mcp__dots__ask mcp__dots__follow_up`.
- One thread lock (`FollowUps::thread_lock`, an `Arc<tokio::sync::Mutex<()>>`) is held by `Runtime::chat`, `CheckIns::reply` and the follow-up loop from reading a thread's end until the new run is stored.
- The follow-up loop uses `scheduler::Clock`, `scheduler::MAX_SLEEP` (60 s) and `scheduler::ERROR_WAIT` (30 s) without changing `scheduler.rs`.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`. Live data is parsed, not cast. The fixture `ui/src/api/__fixtures__/contract.json` is written only by `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- How the code blocks are written. A step changes files in three ways, applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block occurs in the file exactly once, as it stands at that moment; the second takes its place.
  - **Append to `path`:** the block is added at the end of the file.
- A Replace whose passage is not found once (the parallel developer changed it): stop, read the file, and apply the same change to the passage as it is now; never paste over their change.
- Before each commit: the task's own tests, then as the change requires `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt` then `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. Files are LF with a final newline and valid UTF-8 (`git diff --cached --check`).
- Known flaky under load (from the previous plan): `runtime_local_test::live_approval_lets_the_command_run`, `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`, `claude_engine_test::a_finished_run_leaves_no_process_behind`, some `resume_test` tests, and a `vitest` run started right after an install or a build (`Timeout waiting for worker to respond`). Rerun alone before treating one as a failure.

## Review Focus

The inputs and failure modes the spec implies that no task's tests would naturally exercise, most likely to bite a person first, each pinned to the test that covers it:

1. A model's loose follow-up time: `"in": "2 hours"`, `"90"`, `"1.5h"`, `"2H"`, `"-5m"`, a number too large for a duration, `"at"` without an offset or in the past, both or neither given, a blank `at` next to a valid `in`, exactly 5 minutes and exactly 30 days. Expected: one rule sentence each, the edges accepted. Test: `checkins_test::follow_up_times_are_read_and_limited_at_their_edges` (Task 3).
2. Parallel tool calls in one turn (Claude can issue several at once): two `ask` calls together, four `follow_up` calls together. Expected: one question and one `You already asked…`; three follow-ups and one `This dot already has 3 pending…`. Test: `checkins_call_test::parallel_calls_store_one_question_and_three_follow_ups` (Task 4).
3. Two answers to one question at once (a double click on a choice, the Inbox and the chat bar together). Expected: one run; the other answer is refused with a `Conflict` and changes nothing. Tests: `runtime_checkins_test::two_replies_at_once_start_one_run` (Task 5) and `InboxMessages.test.tsx` "answers once on a double click" (Task 10).
4. A chat message and a due follow-up on the same thread at the same moment. Expected: no run gets two children; the follow-up fires either before (the chat is refused) or after the chat's run. Test: `runtime_followups_test::a_chat_and_a_due_follow_up_on_one_thread_give_one_run` (Task 6).
5. Text counted in bytes instead of characters, or untrimmed: 2,000 × `é` and 80 × `🙂` accepted, one more refused, whitespace-only refused, padding trimmed before storing. Test: `checkins_test::text_is_counted_in_characters_after_trimming` and `a_question_offers_one_to_five_choices_of_up_to_80_characters` (Task 3).

---

## Context

It extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md`, the MCP/skills spec `2026-10-15-mcp-skills-plugins-design.md` and the context library spec `2026-10-19-context-library-design.md`. Task 13 brings the spec and the passages its section 12 names in line with what the tasks build; the table "Deviations" lists what changed and why.

**Previous plans:** `docs/superpowers/plans/2026-10-19-context-library.md` (the most recent; this plan copies its `context` tool pattern), and the plans it lists.

**How this plan's code was written:** every Replace block below was copied from the files at commit `aabd37f` (branch `feat/checkins-followups`); the new code follows the idioms of the files it sits next to. The blocks were not compiled in a prototype, so a step that fails to compile is fixed in that step, keeping the step's intent, and the fix is named in the task's commit message body.

## Deviations from the spec

The spec says something else, or nothing, in these places; Task 13 records them in its section 13.

| Spec | This plan | Why |
|---|---|---|
| §4: the rules go to "every preset (Sandboxed, Trusted, and Custom)" | Read-only gets them too | `Policy::preset` gives every preset `Skill` and `Context` the same way; the tools write nothing in the workspace |
| §6: "next try after 1, 5, then 30 minutes … after the third failure the row is skipped" | Failures 1 to 3 retry after 1, 5 and 30 minutes; the fourth failure skips | Otherwise the 30-minute delay is never used |
| §6: "this is <n> hours late" | `hour` when n is 1; n rounded to whole hours, at least 1; appended to the first line | A 16-minute delay must not read "0 hours" |
| §6: "a run that cannot be enqueued" | Any error while firing one row (reading its thread, storing its run) counts as a failed start of that row | One error path; a store error while reading is as transient as one while writing |
| §7: `messages` columns | `Message` also carries `dot_name` (joined from `dots`) | The Inbox and the notification name the dot without another query |
| §10: `messages_list(dot_id?)`, `follow_ups_list(dot_id?)` | `messages_list` answers every waiting message and the 200 newest others, newest first; `follow_ups_list` answers the pending ones, soonest first | Bounded lists; the screens show only these |
| §5: dismiss | Dismissing a dismissed question changes nothing; dismissing an answered one is `Conflict`; a note cannot be dismissed (`Invalid`) | Idempotent for a second window |
| §6: follow-up run without a session | Its first prompt has a `## Follow-up` section with the message; the payload also holds `follow_up_id` | The prompt must say what the run is for |
| §9: the chat bar | It also offers the question's choices; a reply in a thread whose end has no session opens the new chat | Same as the Inbox; the answer starts a new thread then |
| §2: blank `at`/`in` | A blank string counts as absent | The local engine passes empty optional strings through |

## File Map

- `crates/dots-core/migrations/0008_checkins.sql` (new, Task 1; Task 2 appends): the two tables, their indexes, the `upgrade_0008` list.
- `crates/dots-core/src/model.rs` (Task 1): `TriggerKind::FollowUp`, `MessageKind`, `Message`, `FollowUpStatus`, `FollowUp`, the two refusal sentences.
- `crates/dots-core/src/store/messages.rs`, `store/follow_ups.rs` (new, Task 1), `store/mod.rs` (Tasks 1, 2): the rows; one-question and 3-pending checks inside the inserts; answer and fire in one transaction with their run.
- `crates/dots-core/src/policy.rs`, `store/upgrade.rs` (Task 2): `CHECK_IN_TOOLS`, the preset rules, `cli_lists`, `finish_0008`.
- `crates/dots-core/src/checkins.rs` (new, Tasks 3, 4, 5): tool names, descriptions, input rules, `local_label`, `CheckIns::call`, `reply`, `dismiss`, `mark_read`, `list_messages`, `thread_busy`.
- `crates/dots-core/src/followups.rs` (new, Tasks 3, 4, 6): limits, `due_at`, `FollowUps` (schedule, list, cancel, pause, the loop), `late_words`, `follow_up_run`.
- `crates/dots-core/src/lib.rs` (Task 3), `events.rs` (Task 4): module list; `MessagePosted`, `MessageUpdated`, `FollowUpsUpdated`.
- `crates/dots-core/src/runtime.rs` (Tasks 5, 6, 7, 8): fields, `reply`, accessors, `inbox_count`, wake and pause one-liners, the shared lock, wiring.
- `crates/dots-core/src/prompt.rs` (Task 6): the `FollowUp` arms.
- `crates/dots-core/src/runner.rs`, `server/mcp.rs`, `engine/claude/args.rs` (Task 7): the `dots` server's three tools and `--allowedTools`.
- `crates/dots-core/src/engine/local/{mod.rs,mcp_tools.rs,tools/mod.rs}`, `engine/local/tools/checkins.rs` (new) (Task 8): the local tools.
- `crates/dots-core/src/settings.rs` (Task 9): `notify_on_notes`.
- `apps/dots-app/src/{commands.rs,lib.rs,events.rs,contract.rs}` (Task 9; `contract.rs` also regenerated in Tasks 1, 2): commands, notification, samples.
- `ui/src/api/{types.ts,commands.ts}`, `ui/src/lib/{query.ts,runtime-events.ts}`, `ui/src/test/fixtures.ts` (Task 9; `types.ts` also Task 1): the UI's side of the contract.
- `ui/src/lib/messages.ts`, `ui/src/components/MessageCard.tsx` (new), `ui/src/screens/InboxScreen.tsx`, `ui/src/app/Layout.tsx` (Task 10): the Messages group and the count.
- `ui/src/screens/ChatScreen.tsx`, `ui/src/components/Transcript.tsx` (Task 11; `Transcript.tsx` also Tasks 1, 12): check-ins in a chat and the reply bar.
- `ui/src/lib/transcript.ts`, `ui/src/components/FollowUpsList.tsx` (new), `ui/src/screens/{DotsScreen,DotEditorScreen,SettingsScreen}.tsx` (Task 12): run-view lines, follow-ups, the switch.
- Tests (new unless named): `crates/dots-core/tests/{store_checkins_test,policy_checkins_test,checkins_test,checkins_call_test,runtime_checkins_test,followups_test,runtime_followups_test,claude_checkins_test,local_checkins_test}.rs`; existing `store_library_test.rs` (Task 2), `engine_prompt_test.rs` (Task 6), `claude_cli_test.rs` (Task 7), `settings_test.rs` (Task 9); UI `ui/src/api/{commands.test.ts,contract.test.ts}` (existing, Task 9), `ui/src/lib/checkins-events.test.ts`, `ui/src/screens/{InboxMessages,ChatCheckIns,RunCheckIns,FollowUps,SettingsNotes}.test.tsx`.
- Docs: the four specs (Task 13); `docs/verification/2026-10-20-checkins-followups-smoke.md` (Task 14).

### Task 1: Store the dots' messages and follow-ups

Migration 0008 creates the two tables. The model gains the trigger `follow_up`, the message and follow-up types and the two sentences that refuse a closed question. The store reads and writes the rows; the inserts check the one question per run and the pending limit themselves, and answering a question or firing a follow-up stores its run in the same transaction. The UI's trigger enum and the transcript's trigger names follow, with the fixture.

**Files:**

- Create: `crates/dots-core/migrations/0008_checkins.sql`
- Modify: `crates/dots-core/src/model.rs` (the `TriggerKind` list near line 12; types appended at the end)
- Modify: `crates/dots-core/src/store/mod.rs` (the module list near line 9)
- Create: `crates/dots-core/src/store/messages.rs`
- Create: `crates/dots-core/src/store/follow_ups.rs`
- Test: `crates/dots-core/tests/store_checkins_test.rs` (new)
- Modify: `ui/src/api/types.ts` (line 10)
- Modify: `ui/src/components/Transcript.tsx` (the `TRIGGER` map near line 17)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**

- Consumes: `store::runs::insert_run(conn: &mut SqliteConnection, new: &NewRun) -> Result<String>` (exists, `pub(super)`), `util::{new_id, now, timestamp}`, `str_enum!`.
- Produces:
  - `TriggerKind::FollowUp` (`"follow_up"`).
  - `str_enum!(MessageKind { Note => "note", Question => "question" })`, `str_enum!(FollowUpStatus { Pending => "pending", Fired => "fired", Cancelled => "cancelled", Skipped => "skipped" })`.
  - `pub const QUESTION_ANSWERED: &str = "this question has already been answered"`, `pub const QUESTION_DISMISSED: &str = "this question was dismissed"` (in `model`).
  - `pub struct Message { id, dot_id, dot_name, run_id: Option<String>, kind: MessageKind, text, choices: Option<Vec<String>>, created_at, read_at: Option<String>, answered_at: Option<String>, answer_run_id: Option<String>, dismissed_at: Option<String> }` with `fn waits(&self) -> bool` and `fn refusal(&self) -> Option<Error>`.
  - `pub struct FollowUp { id, dot_id, from_run_id, due_at, note, status: FollowUpStatus, fired_run_id: Option<String>, attempts: u32, retry_at: Option<String>, created_at }`.
  - `Store::add_note(&self, dot_id: &str, run_id: Option<&str>, text: &str) -> Result<Message>`; `Store::add_question(&self, dot_id: &str, run_id: &str, text: &str, choices: Option<&[String]>) -> Result<Option<Message>>` (`None`: the run asked already); `Store::get_message(&self, id: &str) -> Result<Message>`; `Store::list_messages(&self, dot_id: Option<&str>) -> Result<Vec<Message>>`; `Store::count_waiting_messages(&self) -> Result<usize>`; `Store::mark_message_read(&self, id: &str) -> Result<Message>`; `Store::dismiss_question(&self, id: &str) -> Result<Message>`; `Store::answer_question(&self, id: &str, new: &NewRun) -> Result<(Message, Run)>`.
  - `Store::add_follow_up(&self, dot_id: &str, from_run_id: &str, due_at: DateTime<Utc>, note: &str, max_pending: usize, now: DateTime<Utc>) -> Result<Option<FollowUp>>` (`None`: at the limit); `Store::get_follow_up(&self, id: &str) -> Result<FollowUp>`; `Store::list_pending_follow_ups(&self, dot_id: Option<&str>) -> Result<Vec<FollowUp>>`; `Store::fire_follow_up(&self, id: &str, new: &NewRun) -> Result<Run>`; `Store::retry_follow_up(&self, id: &str, retry_at: DateTime<Utc>) -> Result<bool>`; `Store::skip_follow_up(&self, id: &str, failed: bool) -> Result<bool>`; `Store::cancel_follow_up(&self, id: &str) -> Result<FollowUp>`.
  - UI: `TriggerKind` gains `"follow_up"`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/store_checkins_test.rs`:**

```rust
//! The `messages` and `follow_ups` rows (check-ins spec section 7): round trips, the one
//! question per run and the pending limit that the inserts check themselves, answering and
//! firing in one transaction with their run, and the rows going with their dot.

mod common;

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::model::{
    Dot, FollowUpStatus, Message, MessageKind, NewRun, Run, RunStatus, TriggerKind,
    QUESTION_ANSWERED, QUESTION_DISMISSED,
};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;

const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// A dot and a queued run of it.
async fn dot_and_run(store: &Store, name: &str) -> (Dot, Run) {
    let dot = store.create_dot(&spec(name)).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    (dot, run)
}

/// The chat run that answers a question of `run`.
fn answer_of(run: &Run) -> NewRun {
    let mut new = NewRun::new(&run.dot_id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "yes"}));
    new.parent_run_id = Some(run.id.clone());
    new.root_run_id = Some(run.root_run_id.clone());
    new
}

/// Lets the clock move on, so rows written next sort after the ones before.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

fn ids(list: Vec<Message>) -> Vec<String> {
    list.into_iter().map(|m| m.id).collect()
}

#[tokio::test]
async fn notes_and_questions_round_trip_and_name_their_dot() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "asker").await;
    let note = store
        .add_note(&dot.id, Some(&run.id), "Half way there.")
        .await
        .unwrap();
    assert_eq!(note.kind, MessageKind::Note);
    assert_eq!(note.dot_name, "asker");
    assert_eq!(note.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(note.choices, None);
    assert!(note.waits());
    tick().await;
    let own = store
        .add_note(&dot.id, None, "A follow-up was skipped.")
        .await
        .unwrap();
    assert_eq!(own.run_id, None);
    tick().await;
    let choices = vec!["yes".to_string(), "no".to_string()];
    let question = store
        .add_question(&dot.id, &run.id, "Ship it?", Some(&choices))
        .await
        .unwrap()
        .expect("the run's first question");
    assert_eq!(question.kind, MessageKind::Question);
    assert_eq!(question.choices, Some(choices));
    assert_eq!(store.get_message(&question.id).await.unwrap(), question);
    // Newest first, of one dot or of all.
    assert_eq!(
        ids(store.list_messages(Some(&dot.id)).await.unwrap()),
        [question.id.clone(), own.id.clone(), note.id.clone()]
    );
    assert_eq!(store.list_messages(None).await.unwrap().len(), 3);
    assert!(store
        .list_messages(Some("no-such-dot"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.count_waiting_messages().await.unwrap(), 3);

    let read = store.mark_message_read(&note.id).await.unwrap();
    assert!(read.read_at.is_some());
    assert!(!read.waits());
    // A second read keeps the first time.
    assert_eq!(
        store.mark_message_read(&note.id).await.unwrap().read_at,
        read.read_at
    );
    // A question that was read still waits for its answer.
    assert!(store.mark_message_read(&question.id).await.unwrap().waits());
    assert_eq!(store.count_waiting_messages().await.unwrap(), 2);
    assert!(matches!(
        store.get_message("nope").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.mark_message_read("nope").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn a_run_asks_one_question_and_two_asks_at_once_store_one() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "asker").await;
    let (a, b) = tokio::join!(
        store.add_question(&dot.id, &run.id, "First?", None),
        store.add_question(&dot.id, &run.id, "Second?", None),
    );
    let stored: Vec<Message> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(stored.len(), 1, "one question per run");
    assert!(store
        .add_question(&dot.id, &run.id, "Third?", None)
        .await
        .unwrap()
        .is_none());
    // A note does not count, and another run asks its own.
    store
        .add_note(&dot.id, Some(&run.id), "a note")
        .await
        .unwrap();
    let other = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store
        .add_question(&dot.id, &other.id, "Again?", None)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn answering_stores_the_run_with_the_answer_and_closes_the_question() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "asker").await;
    let question = store
        .add_question(&dot.id, &run.id, "Ship it?", None)
        .await
        .unwrap()
        .unwrap();
    let (answered, child) = store
        .answer_question(&question.id, &answer_of(&run))
        .await
        .unwrap();
    assert_eq!(child.status, RunStatus::Queued);
    assert_eq!(child.trigger, TriggerKind::Chat);
    assert_eq!(child.parent_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(answered.answer_run_id.as_deref(), Some(child.id.as_str()));
    assert!(answered.answered_at.is_some());
    assert!(!answered.waits());
    assert!(answered.refusal().is_some());

    // A closed question is refused, and no run is stored for it.
    let again = store.answer_question(&question.id, &answer_of(&run)).await;
    assert!(
        matches!(&again, Err(Error::Conflict(m)) if m == QUESTION_ANSWERED),
        "{again:?}"
    );
    let dismissed = store.dismiss_question(&question.id).await;
    assert!(
        matches!(&dismissed, Err(Error::Conflict(m)) if m == QUESTION_ANSWERED),
        "{dismissed:?}"
    );
    assert_eq!(
        store.list_thread(&run.root_run_id).await.unwrap().len(),
        2,
        "only the first answer stored a run"
    );

    let other = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let second = store
        .add_question(&dot.id, &other.id, "Which branch?", None)
        .await
        .unwrap()
        .unwrap();
    assert!(second.refusal().is_none());
    let closed = store.dismiss_question(&second.id).await.unwrap();
    assert!(closed.dismissed_at.is_some());
    assert!(!closed.waits());
    // A second dismiss changes nothing.
    assert_eq!(store.dismiss_question(&second.id).await.unwrap(), closed);
    let late = store.answer_question(&second.id, &answer_of(&other)).await;
    assert!(
        matches!(&late, Err(Error::Conflict(m)) if m == QUESTION_DISMISSED),
        "{late:?}"
    );
    assert_eq!(store.list_thread(&other.root_run_id).await.unwrap().len(), 1);

    let note = store.add_note(&dot.id, None, "a note").await.unwrap();
    assert!(matches!(
        store.dismiss_question(&note.id).await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        store.answer_question(&note.id, &answer_of(&run)).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn follow_ups_round_trip_and_four_at_once_store_the_three_allowed() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "later").await;
    let at = |minutes: i64| t0() + TimeDelta::minutes(minutes);
    let (a, b, c, d) = tokio::join!(
        store.add_follow_up(&dot.id, &run.id, at(30), "third", 3, t0()),
        store.add_follow_up(&dot.id, &run.id, at(10), "first", 3, t0()),
        store.add_follow_up(&dot.id, &run.id, at(20), "second", 3, t0()),
        store.add_follow_up(&dot.id, &run.id, at(40), "fourth", 3, t0()),
    );
    let stored: Vec<_> = [a, b, c, d].into_iter().flat_map(Result::unwrap).collect();
    assert_eq!(stored.len(), 3, "four at once store three");
    let pending = store.list_pending_follow_ups(Some(&dot.id)).await.unwrap();
    assert_eq!(pending.len(), 3);
    assert!(
        pending.windows(2).all(|w| w[0].due_at <= w[1].due_at),
        "soonest first: {pending:#?}"
    );
    let first = pending[0].clone();
    assert_eq!(first.status, FollowUpStatus::Pending);
    assert_eq!(first.from_run_id, run.id);
    assert_eq!(first.created_at, timestamp(t0()));
    assert_eq!(first.attempts, 0);
    assert_eq!(first.retry_at, None);
    assert_eq!(store.get_follow_up(&first.id).await.unwrap(), first);
    assert!(store
        .list_pending_follow_ups(Some("no-such-dot"))
        .await
        .unwrap()
        .is_empty());

    // A failed start is counted; then the row fires, and its run is stored with it.
    assert!(store.retry_follow_up(&first.id, at(11)).await.unwrap());
    let retried = store.get_follow_up(&first.id).await.unwrap();
    assert_eq!(retried.attempts, 1);
    assert_eq!(retried.retry_at, Some(timestamp(at(11))));
    let mut new = NewRun::new(&dot.id, TriggerKind::FollowUp);
    new.payload = Some(json!({"message": "later", "follow_up_id": first.id}));
    let fired = store.fire_follow_up(&first.id, &new).await.unwrap();
    assert_eq!(fired.trigger, TriggerKind::FollowUp);
    assert_eq!(fired.status, RunStatus::Queued);
    let row = store.get_follow_up(&first.id).await.unwrap();
    assert_eq!(row.status, FollowUpStatus::Fired);
    assert_eq!(row.fired_run_id.as_deref(), Some(fired.id.as_str()));
    assert_eq!(row.retry_at, None);
    assert!(matches!(
        store.fire_follow_up(&first.id, &new).await,
        Err(Error::Conflict(_))
    ));
    assert!(!store.retry_follow_up(&first.id, at(12)).await.unwrap());
    // A fired one no longer counts: there is room for one more.
    assert!(store
        .add_follow_up(&dot.id, &run.id, at(50), "fifth", 3, t0())
        .await
        .unwrap()
        .is_some());

    let pending = store.list_pending_follow_ups(Some(&dot.id)).await.unwrap();
    let cancelled = store.cancel_follow_up(&pending[0].id).await.unwrap();
    assert_eq!(cancelled.status, FollowUpStatus::Cancelled);
    assert!(matches!(
        store.cancel_follow_up(&pending[0].id).await,
        Err(Error::Conflict(_))
    ));
    assert!(store.skip_follow_up(&pending[1].id, true).await.unwrap());
    let skipped = store.get_follow_up(&pending[1].id).await.unwrap();
    assert_eq!(skipped.status, FollowUpStatus::Skipped);
    assert_eq!(skipped.attempts, 1);
    assert!(!store.skip_follow_up(&pending[1].id, false).await.unwrap());
    assert_eq!(store.list_pending_follow_ups(None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_deleted_dot_takes_its_messages_and_follow_ups_with_it() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "gone").await;
    let note = store.add_note(&dot.id, Some(&run.id), "bye").await.unwrap();
    let follow_up = store
        .add_follow_up(&dot.id, &run.id, t0() + TimeDelta::hours(1), "later", 3, t0())
        .await
        .unwrap()
        .unwrap();
    store.delete_dot(&dot.id).await.unwrap();
    assert!(matches!(
        store.get_message(&note.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.get_follow_up(&follow_up.id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(store.count_waiting_messages().await.unwrap(), 0);
    assert!(store.list_pending_follow_ups(None).await.unwrap().is_empty());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test store_checkins_test`
Expected: FAIL to compile: `MessageKind`, `FollowUpStatus`, `Message`, `QUESTION_ANSWERED`, `TriggerKind::FollowUp` and the `Store` methods `add_note`, `add_follow_up`, … do not exist.

- [ ] **Step 3: Add the migration, the model types and the store**

**Create `crates/dots-core/migrations/0008_checkins.sql`:**

```sql
-- Check-ins and follow-ups (spec 2026-10-20-checkins-followups-design.md, sections 7 and 8).

-- What dots tell and ask the person. A note of Dots' own (a skipped follow-up) has no run.
CREATE TABLE messages (
    id            TEXT PRIMARY KEY,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT,
    kind          TEXT NOT NULL,
    text          TEXT NOT NULL,
    choices       TEXT,
    created_at    TEXT NOT NULL,
    read_at       TEXT,
    answered_at   TEXT,
    answer_run_id TEXT,
    dismissed_at  TEXT
);
CREATE INDEX messages_dot ON messages(dot_id, created_at);

-- The runs dots scheduled for themselves. The follow-up loop finds the due ones by status.
CREATE TABLE follow_ups (
    id           TEXT PRIMARY KEY,
    dot_id       TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    from_run_id  TEXT NOT NULL,
    due_at       TEXT NOT NULL,
    note         TEXT NOT NULL,
    status       TEXT NOT NULL,
    fired_run_id TEXT,
    attempts     INTEGER NOT NULL DEFAULT 0,
    retry_at     TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX follow_ups_due ON follow_ups(status, due_at);
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    Chat => "chat",
    Resume => "resume",
});
```

**with:**

```rust
    Chat => "chat",
    Resume => "resume",
    FollowUp => "follow_up",
});
```

**Append to `crates/dots-core/src/model.rs`:**

```rust

str_enum!(MessageKind { Note => "note", Question => "question" });
str_enum!(FollowUpStatus {
    Pending => "pending",
    Fired => "fired",
    Cancelled => "cancelled",
    Skipped => "skipped",
});

/// What `reply` answers about a question that was answered already (check-ins spec section 5).
pub const QUESTION_ANSWERED: &str = "this question has already been answered";
/// What `reply` answers about a question the person dismissed.
pub const QUESTION_DISMISSED: &str = "this question was dismissed";

/// A dot's check-in (check-ins spec sections 5 and 7): a note that needs no answer, or a
/// question the person answers or dismisses. Times are RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub dot_id: String,
    /// The dot's name, for the Inbox and the notification.
    pub dot_name: String,
    /// The run that posted it; `None` for a note of Dots' own (a skipped follow-up).
    pub run_id: Option<String>,
    pub kind: MessageKind,
    pub text: String,
    /// The answers a question offers as buttons.
    pub choices: Option<Vec<String>>,
    pub created_at: String,
    pub read_at: Option<String>,
    pub answered_at: Option<String>,
    /// The run the answer started.
    pub answer_run_id: Option<String>,
    pub dismissed_at: Option<String>,
}

impl Message {
    /// It waits for the person: a question neither answered nor dismissed, or a note not read
    /// yet. The Inbox and its count show these.
    pub fn waits(&self) -> bool {
        match self.kind {
            MessageKind::Question => self.answered_at.is_none() && self.dismissed_at.is_none(),
            MessageKind::Note => self.read_at.is_none(),
        }
    }

    /// Why the message cannot be answered: `None` for an open question.
    pub fn refusal(&self) -> Option<Error> {
        match self.kind {
            MessageKind::Note => Some(Error::Invalid("a note cannot be answered".into())),
            MessageKind::Question if self.dismissed_at.is_some() => {
                Some(Error::Conflict(QUESTION_DISMISSED.into()))
            }
            MessageKind::Question if self.answered_at.is_some() => {
                Some(Error::Conflict(QUESTION_ANSWERED.into()))
            }
            MessageKind::Question => None,
        }
    }
}

/// A run a dot scheduled for itself (check-ins spec sections 6 and 7). Times are RFC 3339 UTC
/// (`util::timestamp`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUp {
    pub id: String,
    pub dot_id: String,
    /// The run that called `follow_up`; the follow-up continues its thread.
    pub from_run_id: String,
    pub due_at: String,
    /// What the dot told itself to do then.
    pub note: String,
    pub status: FollowUpStatus,
    pub fired_run_id: Option<String>,
    /// Starts that failed.
    pub attempts: u32,
    /// When a start that failed is tried again.
    pub retry_at: Option<String>,
    pub created_at: String,
}
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod approvals;
mod dots;
mod library;
mod runs;
```

**with:**

```rust
mod approvals;
mod dots;
mod follow_ups;
mod library;
mod messages;
mod runs;
```

**Create `crates/dots-core/src/store/messages.rs`:**

```rust
//! The `messages` rows: the notes and questions dots post (check-ins spec sections 5 and 7).
//! A run asks at most one question; the insert checks it, so two calls at once cannot both
//! ask. Answering stores the answer's run in the same transaction.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{Message, MessageKind, NewRun, Run};
use crate::util::{new_id, now};
use crate::{Error, Result};

/// The messages `list_messages` gives besides those that wait.
const MESSAGES_SHOWN: i64 = 200;

const MESSAGE_COLUMNS: &str = "m.id AS id, m.dot_id AS dot_id, d.name AS dot_name, \
    m.run_id AS run_id, m.kind AS kind, m.text AS text, m.choices AS choices, \
    m.created_at AS created_at, m.read_at AS read_at, m.answered_at AS answered_at, \
    m.answer_run_id AS answer_run_id, m.dismissed_at AS dismissed_at";

/// A message of `m` that waits for the person (`Message::waits`).
const WAITS: &str = "((m.kind = 'question' AND m.answered_at IS NULL \
    AND m.dismissed_at IS NULL) OR (m.kind = 'note' AND m.read_at IS NULL))";

fn message_from_row(row: &SqliteRow) -> Result<Message> {
    Ok(Message {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        dot_name: row.try_get("dot_name")?,
        run_id: row.try_get("run_id")?,
        kind: MessageKind::parse(&row.try_get::<String, _>("kind")?)?,
        text: row.try_get("text")?,
        choices: row
            .try_get::<Option<String>, _>("choices")?
            .map(|s| serde_json::from_str(&s))
            .transpose()?,
        created_at: row.try_get("created_at")?,
        read_at: row.try_get("read_at")?,
        answered_at: row.try_get("answered_at")?,
        answer_run_id: row.try_get("answer_run_id")?,
        dismissed_at: row.try_get("dismissed_at")?,
    })
}

impl Store {
    /// Stores a note of the dot; `run_id` is `None` for a note of Dots' own.
    pub async fn add_note(&self, dot_id: &str, run_id: Option<&str>, text: &str) -> Result<Message> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO messages (id, dot_id, run_id, kind, text, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(run_id)
        .bind(MessageKind::Note.as_str())
        .bind(text)
        .bind(now())
        .execute(&self.pool)
        .await?;
        self.get_message(&id).await
    }

    /// Stores the question of the run `run_id`; `None` when the run asked one already. The
    /// check and the insert are one statement.
    pub async fn add_question(
        &self,
        dot_id: &str,
        run_id: &str,
        text: &str,
        choices: Option<&[String]>,
    ) -> Result<Option<Message>> {
        let id = new_id();
        let choices = match choices {
            Some(list) => Some(serde_json::to_string(list)?),
            None => None,
        };
        let res = sqlx::query(
            "INSERT INTO messages (id, dot_id, run_id, kind, text, choices, created_at) \
             SELECT ?, ?, ?, 'question', ?, ?, ? WHERE NOT EXISTS \
             (SELECT 1 FROM messages WHERE run_id = ? AND kind = 'question')",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(run_id)
        .bind(text)
        .bind(choices)
        .bind(now())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_message(&id).await.map(Some)
    }

    pub async fn get_message(&self, id: &str) -> Result<Message> {
        let row = sqlx::query(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN dots d ON d.id = m.dot_id \
             WHERE m.id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("message {id}")))?;
        message_from_row(&row)
    }

    /// Every message that waits for the person and the `MESSAGES_SHOWN` newest others, of one
    /// dot or of all, newest first.
    pub async fn list_messages(&self, dot_id: Option<&str>) -> Result<Vec<Message>> {
        let rows = sqlx::query(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN dots d ON d.id = m.dot_id \
             WHERE (? IS NULL OR m.dot_id = ?) AND ({WAITS} OR m.id IN \
             (SELECT x.id FROM messages x WHERE ? IS NULL OR x.dot_id = ? \
              ORDER BY x.created_at DESC, x.id DESC LIMIT ?)) \
             ORDER BY m.created_at DESC, m.id DESC"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .bind(dot_id)
        .bind(dot_id)
        .bind(MESSAGES_SHOWN)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(message_from_row).collect()
    }

    /// The messages that wait for the person: open questions and unread notes.
    pub async fn count_waiting_messages(&self) -> Result<usize> {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM messages m WHERE {WAITS}"))
            .fetch_one(&self.pool)
            .await?;
        Ok(n as usize)
    }

    /// Marks a message read; a second read keeps the first time. A question stays open.
    pub async fn mark_message_read(&self, id: &str) -> Result<Message> {
        let res = sqlx::query("UPDATE messages SET read_at = COALESCE(read_at, ?) WHERE id = ?")
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("message {id}")));
        }
        self.get_message(id).await
    }

    /// Dismisses an open question; one dismissed already stays as it is. `Conflict` for an
    /// answered question, `Invalid` for a note.
    pub async fn dismiss_question(&self, id: &str) -> Result<Message> {
        let res = sqlx::query(
            "UPDATE messages SET dismissed_at = ? WHERE id = ? AND kind = 'question' \
             AND answered_at IS NULL AND dismissed_at IS NULL",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        let message = self.get_message(id).await?;
        if res.rows_affected() == 0 && message.dismissed_at.is_none() {
            if message.kind == MessageKind::Note {
                return Err(Error::Invalid("only a question can be dismissed".into()));
            }
            if let Some(refusal) = message.refusal() {
                return Err(refusal);
            }
        }
        Ok(message)
    }

    /// Answers the open question `id` with the run `new`, in one transaction: the run is stored
    /// and the question gets `answered_at` and `answer_run_id`. A closed question is refused
    /// (`Message::refusal`) and nothing is written.
    pub async fn answer_question(&self, id: &str, new: &NewRun) -> Result<(Message, Run)> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE messages SET answered_at = ?, answer_run_id = ? WHERE id = ? \
             AND kind = 'question' AND answered_at IS NULL AND dismissed_at IS NULL",
        )
        .bind(now())
        .bind(&run_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            let message = self.get_message(id).await?;
            return Err(message
                .refusal()
                .unwrap_or_else(|| Error::Conflict(format!("message {id} changed meanwhile"))));
        }
        tx.commit().await?;
        Ok((self.get_message(id).await?, self.get_run(&run_id).await?))
    }
}
```

**Create `crates/dots-core/src/store/follow_ups.rs`:**

```rust
//! The `follow_ups` rows: the runs dots scheduled for themselves (check-ins spec sections 6
//! and 7). The follow-up loop (`followups.rs`) fires a due row with `fire_follow_up`, which
//! stores its run in the same transaction, and retries or skips a row whose run could not be
//! stored. Callers pass the times, so the loop's clock decides every time written here.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{FollowUp, FollowUpStatus, NewRun, Run};
use crate::util::{new_id, timestamp};
use crate::{Error, Result};

const FOLLOW_UP_COLUMNS: &str =
    "id, dot_id, from_run_id, due_at, note, status, fired_run_id, attempts, retry_at, created_at";

fn follow_up_from_row(row: &SqliteRow) -> Result<FollowUp> {
    Ok(FollowUp {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        from_run_id: row.try_get("from_run_id")?,
        due_at: row.try_get("due_at")?,
        note: row.try_get("note")?,
        status: FollowUpStatus::parse(&row.try_get::<String, _>("status")?)?,
        fired_run_id: row.try_get("fired_run_id")?,
        attempts: row.try_get::<i64, _>("attempts")? as u32,
        retry_at: row.try_get("retry_at")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    /// Stores a pending follow-up of `dot_id`, unless the dot has `max_pending` pending ones
    /// already (`None`). The count and the insert are one statement, so two calls at once
    /// cannot pass the limit together.
    pub async fn add_follow_up(
        &self,
        dot_id: &str,
        from_run_id: &str,
        due_at: DateTime<Utc>,
        note: &str,
        max_pending: usize,
        now: DateTime<Utc>,
    ) -> Result<Option<FollowUp>> {
        let id = new_id();
        let res = sqlx::query(
            "INSERT INTO follow_ups (id, dot_id, from_run_id, due_at, note, status, created_at) \
             SELECT ?, ?, ?, ?, ?, 'pending', ? WHERE \
             (SELECT COUNT(*) FROM follow_ups WHERE dot_id = ? AND status = 'pending') < ?",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(from_run_id)
        .bind(timestamp(due_at))
        .bind(note)
        .bind(timestamp(now))
        .bind(dot_id)
        .bind(max_pending as i64)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_follow_up(&id).await.map(Some)
    }

    pub async fn get_follow_up(&self, id: &str) -> Result<FollowUp> {
        let row = sqlx::query(&format!(
            "SELECT {FOLLOW_UP_COLUMNS} FROM follow_ups WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("follow-up {id}")))?;
        follow_up_from_row(&row)
    }

    /// The pending follow-ups, of one dot or of all, soonest first.
    pub async fn list_pending_follow_ups(&self, dot_id: Option<&str>) -> Result<Vec<FollowUp>> {
        let rows = sqlx::query(&format!(
            "SELECT {FOLLOW_UP_COLUMNS} FROM follow_ups WHERE status = 'pending' \
             AND (? IS NULL OR dot_id = ?) ORDER BY due_at, id"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(follow_up_from_row).collect()
    }

    /// Fires the pending follow-up `id`: stores `new` and marks the row `fired` with its run, in
    /// one transaction. `Conflict` when the row is no longer pending; nothing is written then.
    pub async fn fire_follow_up(&self, id: &str, new: &NewRun) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE follow_ups SET status = 'fired', fired_run_id = ?, retry_at = NULL \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(&run_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "follow-up {id} is no longer pending"
            )));
        }
        tx.commit().await?;
        self.get_run(&run_id).await
    }

    /// Counts a failed start of the pending follow-up `id`: `attempts` goes up and the next try
    /// is at `retry_at`. False when the row is no longer pending.
    pub async fn retry_follow_up(&self, id: &str, retry_at: DateTime<Utc>) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE follow_ups SET attempts = attempts + 1, retry_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(timestamp(retry_at))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Skips the pending follow-up `id`; `failed` counts the start that failed last. False when
    /// the row is no longer pending.
    pub async fn skip_follow_up(&self, id: &str, failed: bool) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE follow_ups SET status = 'skipped', retry_at = NULL, \
             attempts = attempts + ? WHERE id = ? AND status = 'pending'",
        )
        .bind(failed as i64)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Cancels the pending follow-up `id` (the person's; dots have no cancel tool). `Conflict`
    /// once it fired, was skipped or was cancelled.
    pub async fn cancel_follow_up(&self, id: &str) -> Result<FollowUp> {
        let res = sqlx::query(
            "UPDATE follow_ups SET status = 'cancelled', retry_at = NULL \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        let row = self.get_follow_up(id).await?;
        if res.rows_affected() == 0 {
            let why = match row.status {
                FollowUpStatus::Fired => "this follow-up has already started its run",
                FollowUpStatus::Skipped => "this follow-up was skipped",
                FollowUpStatus::Cancelled | FollowUpStatus::Pending => {
                    "this follow-up was cancelled already"
                }
            };
            return Err(Error::Conflict(why.into()));
        }
        Ok(row)
    }
}
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const TriggerKind = z.enum(["schedule", "webhook", "manual", "chat", "resume"]);
```

**with:**

```ts
export const TriggerKind = z.enum([
  "schedule",
  "webhook",
  "manual",
  "chat",
  "resume",
  // A run a dot scheduled for itself (check-ins spec section 6).
  "follow_up",
]);
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  chat: "Chat message",
  resume: "Continued after approval",
};
```

**with:**

```tsx
  chat: "Chat message",
  resume: "Continued after approval",
  follow_up: "Follow-up the dot scheduled",
};
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the fixture's `Enums.TriggerKind` gains `follow_up` and nothing else changes.

**Generated:** `ui/src/api/__fixtures__/contract.json`

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test store_checkins_test`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/api/contract.test.ts`
Expected: PASS (`store_checkins_test`: 5 tests); the contract test knows every `TriggerKind`.

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: all pass; no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/migrations/0008_checkins.sql crates/dots-core/src/model.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/messages.rs crates/dots-core/src/store/follow_ups.rs crates/dots-core/tests/store_checkins_test.rs ui/src/api/types.ts ui/src/components/Transcript.tsx ui/src/api/__fixtures__/contract.json
git commit -m "feat(core): store the dots' messages and follow-ups"
```

### Task 2: Allow the check-in tools in every preset and in stored policies

Every preset gains `Notify`, `Ask` and `FollowUp` allow rules after `Skill` and `Context`; their rules stay out of the CLI's lists, because the CLI has no such tools and Dots checks each call itself. Migration 0008 lists the dots, and `finish_0008`, after `finish_0007`, adds the missing rules to every stored policy whose preset is not Custom.

**Files:**

- Modify: `crates/dots-core/src/policy.rs` (`READ_TOOLS` near line 64; `Policy::preset` near line 155; `cli_lists` near line 219)
- Modify: `crates/dots-core/migrations/0008_checkins.sql` (append)
- Modify: `crates/dots-core/src/store/upgrade.rs` (module doc, imports; `finish_0008` appended)
- Modify: `crates/dots-core/src/store/mod.rs` (`Store::open`)
- Test: `crates/dots-core/tests/policy_checkins_test.rs` (new)
- Modify: `crates/dots-core/tests/store_library_test.rs` (three expected rule lists of migration 0006 tests)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**

- Consumes: `Policy::preset`, `Policy::cli_lists`, `upgrade::finish_0007` (exist).
- Produces: `pub const CHECK_IN_TOOLS: [&str; 3] = ["Notify", "Ask", "FollowUp"]` in `policy`; `upgrade::finish_0008(pool: &SqlitePool) -> Result<()>` (`pub(super)`), run by `Store::open` after `finish_0007`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/policy_checkins_test.rs`:**

```rust
//! The check-in tools in the policy (check-ins spec sections 4 and 8): every preset allows
//! `Notify`, `Ask` and `FollowUp` after `Skill` and `Context`, a dot's own rule still decides,
//! the CLI never sees their rules, and migration 0008 gives them to stored policies that are
//! not Custom.

use dots_core::policy::{Action, Policy, Preset, Rule, CHECK_IN_TOOLS};
use dots_core::store::Store;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

#[test]
fn every_preset_allows_the_check_in_tools_after_skill_and_context() {
    assert_eq!(CHECK_IN_TOOLS, ["Notify", "Ask", "FollowUp"]);
    for preset in Preset::ALL {
        let p = Policy::preset(*preset);
        let tail: Vec<&str> = p.rules[p.rules.len() - 5..]
            .iter()
            .map(|r| r.tool.as_str())
            .collect();
        assert_eq!(
            tail,
            ["Skill", "Context", "Notify", "Ask", "FollowUp"],
            "{preset:?}"
        );
        for tool in CHECK_IN_TOOLS {
            assert_eq!(
                p.resolve(tool, &json!({"text": "x"})),
                Action::Allow,
                "{preset:?} {tool}"
            );
        }
        // The CLI has no such tools: their rules stay out of its lists.
        let (allow, deny) = p.cli_lists();
        assert!(
            !allow
                .iter()
                .chain(&deny)
                .any(|t| CHECK_IN_TOOLS.contains(&t.as_str())),
            "{preset:?}"
        );
    }
}

#[test]
fn a_dots_own_check_in_rule_still_decides_and_stays_out_of_the_cli_lists() {
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.insert(0, Rule::new("Notify", Action::Deny));
    assert_eq!(p.resolve("Notify", &json!({"text": "x"})), Action::Deny);
    // deny > allow > ask: an ask rule decides once the preset's allow is gone.
    p.rules.retain(|r| r.tool != "FollowUp");
    p.rules.push(Rule::new("FollowUp", Action::Ask));
    assert_eq!(
        p.resolve("FollowUp", &json!({"note": "x", "in": "1h"})),
        Action::Ask
    );
    p.rules.push(Rule::new("Ask(x)", Action::Deny));
    assert!(p.validate().is_ok());
    let (allow, deny) = p.cli_lists();
    assert!(
        !deny.iter().any(|t| t == "Notify" || t == "Ask(x)"),
        "{deny:?}"
    );
    assert!(!allow.iter().any(|t| t == "Ask"), "{allow:?}");
    // A tool whose name only starts like one stays the CLI's.
    p.rules.push(Rule::new("Asks", Action::Deny));
    assert!(p.cli_lists().1.contains(&"Asks".to_string()));
}

/// Migration 0008 on a database Dots left at 0007: every stored policy whose preset is not
/// Custom gets an allow rule for each check-in tool that no rule names, after `Context`.
#[tokio::test]
async fn migration_0008_lets_every_policy_that_is_not_custom_check_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 8)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0007 left it: the preset's rules without the check-in tools.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| !CHECK_IN_TOOLS.contains(&r.tool.as_str()));
        p
    };
    let mut asks = old(Preset::Trusted);
    asks.rules.push(Rule::new("Ask", Action::Deny));
    let custom = old(Preset::Custom);
    for (id, name, policy) in [
        ("D1", "sandboxed", old(Preset::Sandboxed)),
        ("D2", "read-only", old(Preset::ReadOnly)),
        ("D3", "trusted", asks.clone()),
        ("D4", "custom", custom.clone()),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(serde_json::to_string(&policy).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    for (id, preset) in [("D1", Preset::Sandboxed), ("D2", Preset::ReadOnly)] {
        assert_eq!(
            store.get_dot(id).await.unwrap().spec.policy,
            Policy::preset(preset),
            "{id}"
        );
    }
    // A rule that names a tool already is the person's: only the others are added.
    let mut want = asks;
    want.rules.push(Rule::new("Notify", Action::Allow));
    want.rules.push(Rule::new("FollowUp", Action::Allow));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, want);
    // A Custom policy is the person's own: it is left as it was.
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom);
    store.close().await;
    // Opening again adds nothing twice.
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        store.get_dot("D1").await.unwrap().spec.policy,
        Policy::preset(Preset::Sandboxed)
    );
}
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("mcp__github_com__get", Action::Deny),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
```

**with:**

```rust
            Rule::new("mcp__github_com__get", Action::Deny),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
            // D4 is Trusted: migration 0008 adds the check-in tools.
            Rule::new("Notify", Action::Allow),
            Rule::new("Ask", Action::Allow),
            Rule::new("FollowUp", Action::Allow),
        ]
    );
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
    // Migration 0007 adds `Context` after the `Skill` that 0006 adds.
    let skill = Rule::new("Skill", Action::Allow);
    let context = Rule::new("Context", Action::Allow);
    assert_eq!(
        a.policy.rules,
        [
            Rule::new("mcp__gh__*", Action::Deny),
            skill.clone(),
            context.clone()
        ]
    );
    assert_eq!(
        b.policy.rules,
        [Rule::new("mcp__gh-2__*", Action::Deny), skill, context]
    );
```

**with:**

```rust
    // Migration 0007 adds `Context` after the `Skill` that 0006 adds, and 0008 the check-in
    // tools after it: both dots are Sandboxed.
    let tail = [
        Rule::new("Skill", Action::Allow),
        Rule::new("Context", Action::Allow),
        Rule::new("Notify", Action::Allow),
        Rule::new("Ask", Action::Allow),
        Rule::new("FollowUp", Action::Allow),
    ];
    let with_tail = |first: Rule| [vec![first], tail.to_vec()].concat();
    assert_eq!(
        a.policy.rules,
        with_tail(Rule::new("mcp__gh__*", Action::Deny))
    );
    assert_eq!(
        b.policy.rules,
        with_tail(Rule::new("mcp__gh-2__*", Action::Deny))
    );
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
    let context = Rule::new("Context", Action::Allow);
    assert_eq!(
        rules("D1").await,
        [Rule::new("Skill", Action::Allow), context.clone()]
    );
```

**with:**

```rust
    let context = Rule::new("Context", Action::Allow);
    // D1 is Trusted: migration 0008 adds the check-in tools. D2 and D3 are Custom.
    assert_eq!(
        rules("D1").await,
        [
            Rule::new("Skill", Action::Allow),
            context.clone(),
            Rule::new("Notify", Action::Allow),
            Rule::new("Ask", Action::Allow),
            Rule::new("FollowUp", Action::Allow),
        ]
    );
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test policy_checkins_test --test store_library_test`
Expected: FAIL to compile: `CHECK_IN_TOOLS` does not exist in `dots_core::policy`.

- [ ] **Step 3: Add the rules, keep them from the CLI and upgrade stored policies**

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
const READ_TOOLS: &[&str] = &["Read", "LS", "Glob", "Grep"];
```

**with:**

```rust
/// Dots' own check-in tools by policy name (check-ins spec section 4): every preset allows
/// them, and the CLI never sees their rules.
pub const CHECK_IN_TOOLS: [&str; 3] = ["Notify", "Ask", "FollowUp"];

const READ_TOOLS: &[&str] = &["Read", "LS", "Glob", "Grep"];
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        // Every preset lets the model read the skills and the context collections the dot uses.
        rules.push(Rule::new("Skill", Action::Allow));
        rules.push(Rule::new("Context", Action::Allow));
```

**with:**

```rust
        // Every preset lets the model read the skills and the context collections the dot uses,
        // and check in with the person: notes, a question, follow-ups within their limits.
        rules.push(Rule::new("Skill", Action::Allow));
        rules.push(Rule::new("Context", Action::Allow));
        rules.extend(CHECK_IN_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`. A rule of
    /// `Context` is left out: the CLI has no such tool, and Dots' own `dots` server checks each
    /// call of its `context` tool against these rules itself (context library spec section 4).
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let dots_only = |tool: &str| {
            let tool = tool.trim();
            tool == "Context" || tool.starts_with("Context(")
        };
```

**with:**

```rust
    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`. A rule of
    /// `Context` or of a check-in tool (`CHECK_IN_TOOLS`) is left out: the CLI has no such tool,
    /// and Dots' own `dots` server checks each call of its tools against these rules itself
    /// (context library spec section 4, check-ins spec section 4).
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let dots_only = |tool: &str| {
            let tool = tool.trim();
            ["Context"].iter().chain(CHECK_IN_TOOLS.iter()).any(|name| {
                tool == *name
                    || tool
                        .strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with('('))
            })
        };
```

**Append to `crates/dots-core/migrations/0008_checkins.sql`:**

```sql

-- Every preset allows `Notify`, `Ask` and `FollowUp` (section 4). The policy of each dot whose
-- preset is not Custom gets the missing rules when the store opens (`store/upgrade.rs`,
-- `finish_0008`), after 0007's `Context`. The rows wait here, and the table is dropped once
-- they are done.
CREATE TABLE upgrade_0008 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0008 (dot_id) SELECT id FROM dots;
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
//! 0007 (context library spec section 8): `finish_0007` then gives each dot listed in
//! `upgrade_0007` a `Context` allow rule unless a rule names `Context`, and drops that table.
```

**with:**

```rust
//! 0007 (context library spec section 8): `finish_0007` then gives each dot listed in
//! `upgrade_0007` a `Context` allow rule unless a rule names `Context`, and drops that table.
//! 0008 (check-ins spec section 8): `finish_0008` last gives each dot listed in `upgrade_0008`
//! whose preset is not Custom an allow rule for each check-in tool that no rule names, and
//! drops that table.
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
use crate::policy::{Action, Policy, Preset};
```

**with:**

```rust
use crate::policy::{Action, Policy, Preset, CHECK_IN_TOOLS};
```

**Append to `crates/dots-core/src/store/upgrade.rs`:**

```rust

/// Gives the policy of each dot that migration 0008 listed the check-in tools' allow rules
/// (`with_checkins`), and drops the list.
pub(super) async fn finish_0008(pool: &SqlitePool) -> Result<()> {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0008'",
    )
    .fetch_one(pool)
    .await?;
    if waiting == 0 {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT d.id, d.policy FROM upgrade_0008 u JOIN dots d ON d.id = u.dot_id")
            .fetch_all(&mut *tx)
            .await?;
    for (dot_id, stored) in rows {
        let Some(policy) = with_checkins(&stored) else {
            tracing::warn!(dot = %dot_id, "migration 0008: the policy could not be read");
            continue;
        };
        sqlx::query("UPDATE dots SET policy = ? WHERE id = ?")
            .bind(policy)
            .bind(&dot_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE upgrade_0008")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The stored policy with an allow rule at the end for each check-in tool (`CHECK_IN_TOOLS`, in
/// that order) that no rule names (`Notify` or `Notify(...)`); a Custom policy is left as it
/// is, as the person made it. `None` when the stored JSON is not a policy.
fn with_checkins(stored: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(stored).ok()?;
    let custom = value.get("preset").and_then(Value::as_str) == Some(Preset::Custom.as_str());
    let rules = value.get_mut("rules")?.as_array_mut()?;
    if !custom {
        for tool in CHECK_IN_TOOLS {
            let names_tool = |rule: &Value| {
                rule["tool"].as_str().is_some_and(|t| {
                    let t = t.trim();
                    t == tool
                        || t.strip_prefix(tool)
                            .is_some_and(|rest| rest.starts_with('('))
                })
            };
            if !rules.iter().any(names_tool) {
                rules.push(json!({"tool": tool, "action": Action::Allow.as_str()}));
            }
        }
    }
    // Through `Policy`, so the JSON is written as Dots writes a policy.
    let policy: Policy = serde_json::from_value(value).ok()?;
    serde_json::to_string(&policy).ok()
}
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006 and 0007 that need code (`upgrade.rs`).
```

**with:**

```rust
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006, 0007 and 0008 that need code (`upgrade.rs`).
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
        upgrade::finish_0007(&pool).await?;
```

**with:**

```rust
        upgrade::finish_0007(&pool).await?;
        upgrade::finish_0008(&pool).await?;
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the policies of the `DotSpec`, `Dot`, `DotOverview` and `NewDotSpec` samples end with the three allow rules, and nothing else changes.

**Generated:** `ui/src/api/__fixtures__/contract.json`

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test policy_checkins_test --test store_library_test --test store_dots_test --test policy_test --test claude_cli_test`
Expected: PASS (`policy_checkins_test`: 3 tests). `claude_cli_test::argv_contains_required_flags_in_order` still sees `--allowedTools Read LS Glob Grep Skill`: the new rules never reach the CLI.

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: all pass; no warnings. A UI test that counts a preset's rules counts them from the fixture, so it follows.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/src/policy.rs crates/dots-core/migrations/0008_checkins.sql crates/dots-core/src/store/upgrade.rs crates/dots-core/src/store/mod.rs crates/dots-core/tests/policy_checkins_test.rs crates/dots-core/tests/store_library_test.rs ui/src/api/__fixtures__/contract.json
git commit -m "feat(core): allow the check-in tools in every preset and in stored policies"
```

### Task 3: Describe the three tools and check their input

The tools' names (as each engine shows them), policy names, descriptions and arguments, and the input rules of section 2, in a new module `checkins.rs`; the follow-up limits of section 3 and the time a follow-up is due, in a new module `followups.rs`. Both are pure: nothing is stored yet.

**Files:**

- Create: `crates/dots-core/src/checkins.rs`
- Create: `crates/dots-core/src/followups.rs`
- Modify: `crates/dots-core/src/lib.rs` (module list)
- Test: `crates/dots-core/tests/checkins_test.rs` (new)

**Interfaces:**

- Consumes: `policy::CHECK_IN_TOOLS` (Task 2), `model::EngineKind`.
- Produces, in `checkins`:
  - `pub const MAX_TEXT: usize = 2_000`, `MAX_CHOICES: usize = 5`, `MAX_CHOICE: usize = 80`; `pub const SENT: &str`, `ASKED: &str`, `ASKED_ALREADY: &str` (the exact results).
  - `pub enum Tool { Notify, Ask, FollowUp }` (`Copy`, `Eq`) with `const ALL: [Tool; 3]`, `fn name(self) -> &'static str` (`notify`, `ask`, `follow_up`), `fn policy_name(self) -> &'static str` (`Notify`, `Ask`, `FollowUp`), `fn from_name(&str) -> Option<Tool>`, `fn from_policy_name(&str) -> Option<Tool>`, `fn model_name(self, engine: EngineKind) -> &'static str` (`mcp__dots__<name>` on Claude), `fn description(self) -> &'static str`, `fn properties(self) -> Value`, `fn required(self) -> &'static [&'static str]`, `fn mcp_schema(self) -> Value`.
  - `pub enum When { At(DateTime<Utc>), In(TimeDelta) }`; `pub enum Request { Notify { text: String }, Ask { question: String, choices: Option<Vec<String>> }, FollowUp { note: String, when: When } }`.
  - `pub fn check(tool: Tool, input: &Value) -> std::result::Result<Request, String>`; `pub fn local_label(t: DateTime<Utc>) -> String` (`%a %-d %b %H:%M`, local time).
- Produces, in `followups`: `pub const MIN_AHEAD: TimeDelta` (5 min), `MAX_AHEAD` (30 days), `MAX_PENDING: usize = 3`, `LATE_AFTER` (15 min), `RETRY_DELAYS: [TimeDelta; 3]` (1, 5, 30 min); `pub const TOO_SOON`, `TOO_FAR`, `TOO_MANY: &str`; `pub fn due_at(when: When, now: DateTime<Utc>) -> std::result::Result<DateTime<Utc>, &'static str>`; `pub fn parse_time(s: &str) -> Result<DateTime<Utc>>`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/checkins_test.rs`:**

```rust
//! The check-in tools' input rules and the follow-up limits (check-ins spec sections 2 and 3):
//! the same for both engines, so they are checked here once.

use chrono::{DateTime, Local, TimeDelta, Utc};
use dots_core::checkins::{check, local_label, Request, Tool, When};
use dots_core::followups::{due_at, MAX_AHEAD, MAX_PENDING, MIN_AHEAD};
use dots_core::model::EngineKind;
use dots_core::policy::CHECK_IN_TOOLS;
use serde_json::{json, Value};

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2030-01-07T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn err(tool: Tool, input: Value) -> String {
    check(tool, &input).unwrap_err()
}

/// When a `follow_up` call with `input` is due, or the rule it breaks.
fn when(input: Value) -> Result<When, String> {
    match check(Tool::FollowUp, &input) {
        Ok(Request::FollowUp { when, .. }) => Ok(when),
        Ok(other) => panic!("not a follow-up: {other:?}"),
        Err(e) => Err(e),
    }
}

#[test]
fn the_tools_have_their_names_on_both_engines() {
    let names: Vec<&str> = Tool::ALL.iter().map(|t| t.name()).collect();
    assert_eq!(names, ["notify", "ask", "follow_up"]);
    assert_eq!(Tool::ALL.map(Tool::policy_name), CHECK_IN_TOOLS);
    assert_eq!(Tool::from_name("follow_up"), Some(Tool::FollowUp));
    assert_eq!(Tool::from_name("FollowUp"), None);
    assert_eq!(Tool::from_policy_name("Ask"), Some(Tool::Ask));
    assert_eq!(Tool::from_policy_name("ask"), None);
    assert_eq!(Tool::Ask.model_name(EngineKind::Claude), "mcp__dots__ask");
    assert_eq!(
        Tool::FollowUp.model_name(EngineKind::Claude),
        "mcp__dots__follow_up"
    );
    assert_eq!(Tool::FollowUp.model_name(EngineKind::Local), "follow_up");
    let schema = Tool::FollowUp.mcp_schema();
    assert_eq!(schema["name"], "follow_up");
    assert_eq!(schema["inputSchema"]["required"], json!(["note"]));
    assert_eq!(
        Tool::Ask.mcp_schema()["inputSchema"]["properties"]["choices"]["type"],
        "array"
    );
    for tool in Tool::ALL {
        assert!(!tool.description().is_empty());
    }
}

#[test]
fn text_is_counted_in_characters_after_trimming() {
    let at_limit = "é".repeat(2_000);
    assert_eq!(
        check(Tool::Notify, &json!({"text": format!("  {at_limit}\n")})),
        Ok(Request::Notify {
            text: at_limit.clone()
        })
    );
    assert_eq!(
        err(Tool::Notify, json!({"text": format!("{at_limit}é")})),
        "`text` must be at most 2,000 characters."
    );
    assert_eq!(
        err(Tool::Notify, json!({"text": " \n\t "})),
        "`text` must not be blank."
    );
    for missing in [json!({}), json!({"text": 5}), json!({"text": null})] {
        assert_eq!(
            err(Tool::Notify, missing),
            "`text` must be a string of 1 to 2,000 characters."
        );
    }
    assert_eq!(
        err(Tool::Ask, json!({"question": ""})),
        "`question` must not be blank."
    );
    assert_eq!(
        err(
            Tool::FollowUp,
            json!({"note": "x".repeat(2_001), "in": "1h"})
        ),
        "`note` must be at most 2,000 characters."
    );
}

#[test]
fn a_question_offers_one_to_five_choices_of_up_to_80_characters() {
    let emoji = "🙂".repeat(80);
    assert_eq!(
        check(
            Tool::Ask,
            &json!({"question": " Ship it? ", "choices": [" yes ", emoji.clone()]})
        ),
        Ok(Request::Ask {
            question: "Ship it?".into(),
            choices: Some(vec!["yes".into(), emoji.clone()]),
        })
    );
    assert_eq!(
        check(Tool::Ask, &json!({"question": "Why?", "choices": null})),
        Ok(Request::Ask {
            question: "Why?".into(),
            choices: None,
        })
    );
    let list = "`choices` must be a list of 1 to 5 strings.";
    for choices in [
        json!([]),
        json!(["a", "b", "c", "d", "e", "f"]),
        json!("yes"),
        json!([1]),
    ] {
        assert_eq!(
            err(Tool::Ask, json!({"question": "q", "choices": choices})),
            list
        );
    }
    let each = "Each choice must be 1 to 80 characters.";
    assert_eq!(
        err(Tool::Ask, json!({"question": "q", "choices": ["  "]})),
        each
    );
    assert_eq!(
        err(
            Tool::Ask,
            json!({"question": "q", "choices": [format!("{emoji}🙂")]})
        ),
        each
    );
}

#[test]
fn follow_up_times_are_read_and_limited_at_their_edges() {
    assert_eq!(
        when(json!({"note": "n", "in": "90m"})),
        Ok(When::In(TimeDelta::minutes(90)))
    );
    assert_eq!(
        when(json!({"note": "n", "in": " 2h "})),
        Ok(When::In(TimeDelta::hours(2)))
    );
    // A blank `at` next to `in` counts as absent.
    assert_eq!(
        when(json!({"note": "n", "in": "1d", "at": ""})),
        Ok(When::In(TimeDelta::days(1)))
    );
    assert_eq!(
        when(json!({"note": "n", "at": "2030-01-07T10:00:00+01:00"})),
        Ok(When::At(t0()))
    );
    // A number too large for a duration reads as the longest one, which the limit refuses.
    assert_eq!(
        when(json!({"note": "n", "in": "99999999999999999999d"})),
        Ok(When::In(TimeDelta::MAX))
    );
    let unit = "`in` must be a whole number and a unit m, h or d, such as 90m, 2h or 1d.";
    for bad in ["2 hours", "90", "1.5h", "-5m", "h", "2w", "2H", "1d2h"] {
        assert_eq!(
            when(json!({"note": "n", "in": bad})),
            Err(unit.to_string()),
            "{bad}"
        );
    }
    assert_eq!(when(json!({"note": "n", "in": 90})), Err(unit.to_string()));
    let offset =
        "`at` must be a time in RFC 3339 with an offset, such as 2026-10-21T09:00:00+02:00.";
    for bad in ["2030-01-07T10:00:00", "tomorrow 9am", "2030-01-07"] {
        assert_eq!(
            when(json!({"note": "n", "at": bad})),
            Err(offset.to_string()),
            "{bad}"
        );
    }
    let one = "Give exactly one of `at` or `in`.";
    assert_eq!(when(json!({"note": "n"})), Err(one.to_string()));
    assert_eq!(when(json!({"note": "n", "in": " "})), Err(one.to_string()));
    assert_eq!(
        when(json!({"note": "n", "in": "2h", "at": "2030-01-07T10:00:00Z"})),
        Err(one.to_string())
    );

    // The limits, against the clock of the follow-up loop.
    let soon = "A follow-up must be at least 5 minutes away.";
    let far = "A follow-up must be at most 30 days away.";
    assert_eq!(due_at(When::In(MIN_AHEAD), t0()), Ok(t0() + MIN_AHEAD));
    assert_eq!(due_at(When::In(TimeDelta::minutes(4)), t0()), Err(soon));
    assert_eq!(due_at(When::In(TimeDelta::zero()), t0()), Err(soon));
    assert_eq!(
        due_at(When::At(t0() - TimeDelta::hours(1)), t0()),
        Err(soon)
    );
    assert_eq!(
        due_at(When::At(t0() + TimeDelta::seconds(299)), t0()),
        Err(soon)
    );
    assert_eq!(due_at(When::In(TimeDelta::days(30)), t0()), Ok(t0() + MAX_AHEAD));
    assert_eq!(due_at(When::In(TimeDelta::hours(721)), t0()), Err(far));
    assert_eq!(due_at(When::In(TimeDelta::MAX), t0()), Err(far));
    assert_eq!(MAX_PENDING, 3);
}

#[test]
fn times_are_shown_in_local_time_as_weekday_day_month_and_time() {
    let label = local_label(t0());
    assert_eq!(
        label,
        t0().with_timezone(&Local)
            .format("%a %-d %b %H:%M")
            .to_string()
    );
    assert_eq!(label.split(' ').count(), 4, "{label}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test checkins_test`
Expected: FAIL to compile: unresolved imports `dots_core::checkins` and `dots_core::followups`.

- [ ] **Step 3: Add the two modules**

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod approvals;
pub mod context;
```

**with:**

```rust
pub mod approvals;
pub mod checkins;
pub mod context;
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod events;
pub mod kit;
```

**with:**

```rust
pub mod events;
pub mod followups;
pub mod kit;
```

**Create `crates/dots-core/src/checkins.rs`:**

```rust
//! Check-ins (check-ins spec sections 2, 3 and 5): a dot tells the person something with
//! `notify`, asks them with `ask`, and schedules a run of its own with `follow_up`. The tools
//! are the same on both engines: built in on the local engine, tools of Dots' own `dots` MCP
//! server on the Claude engine. This module holds their names, descriptions and input rules.

use chrono::{DateTime, Local, TimeDelta, Utc};
use serde_json::{json, Value};

use crate::model::EngineKind;

/// The longest note, question or follow-up note, in characters after trimming.
pub const MAX_TEXT: usize = 2_000;
/// The most choices a question offers.
pub const MAX_CHOICES: usize = 5;
/// The longest choice, in characters after trimming.
pub const MAX_CHOICE: usize = 80;

/// What `notify` answers.
pub const SENT: &str = "Sent.";
/// What `ask` answers.
pub const ASKED: &str = "Your question was sent. Finish your turn now; the person's answer will \
    arrive as your next message.";
/// What a second `ask` in one run answers.
pub const ASKED_ALREADY: &str = "You already asked a question in this run; finish your turn now.";

const CHOICES_RULE: &str = "`choices` must be a list of 1 to 5 strings.";
const CHOICE_RULE: &str = "Each choice must be 1 to 80 characters.";
const ONE_OF_RULE: &str = "Give exactly one of `at` or `in`.";
const AT_RULE: &str =
    "`at` must be a time in RFC 3339 with an offset, such as 2026-10-21T09:00:00+02:00.";
const IN_RULE: &str = "`in` must be a whole number and a unit m, h or d, such as 90m, 2h or 1d.";

const NOTIFY_DESCRIPTION: &str = "Post a short note to the person and keep working. Use it \
    sparingly, for progress they would want to know now; your summary reaches them when the \
    run ends.";
const ASK_DESCRIPTION: &str = "Ask the person a question, then finish your turn: their answer \
    arrives as your next message. Ask only when you cannot go on well without their judgement. \
    Offer `choices` when the answer is one of a few. One question per run.";
const FOLLOW_UP_DESCRIPTION: &str = "Schedule a run of your own that continues this \
    conversation later, for work that has to wait: a build, a reply, a date. Give `note`, what \
    to do then, and either `at` (RFC 3339 with an offset) or `in` (a whole number and m, h or \
    d, such as 90m, 2h or 1d). It must be 5 minutes to 30 days away, and a dot has at most 3 \
    waiting.";

/// One of the three check-in tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Notify,
    Ask,
    FollowUp,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Notify, Tool::Ask, Tool::FollowUp];

    /// The name a model calls the tool by on the local engine, and the `dots` server's tool.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Notify => "notify",
            Tool::Ask => "ask",
            Tool::FollowUp => "follow_up",
        }
    }

    /// The name the policy, approvals and grants know the tool by (`policy::CHECK_IN_TOOLS`).
    pub fn policy_name(self) -> &'static str {
        match self {
            Tool::Notify => "Notify",
            Tool::Ask => "Ask",
            Tool::FollowUp => "FollowUp",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn from_policy_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.policy_name() == name)
    }

    /// The name the run's model knows the tool by: Dots' own MCP server's tool on the Claude
    /// engine, a built-in tool on the local engine.
    pub fn model_name(self, engine: EngineKind) -> &'static str {
        match (engine, self) {
            (EngineKind::Claude, Tool::Notify) => "mcp__dots__notify",
            (EngineKind::Claude, Tool::Ask) => "mcp__dots__ask",
            (EngineKind::Claude, Tool::FollowUp) => "mcp__dots__follow_up",
            (EngineKind::Local, tool) => tool.name(),
        }
    }

    /// When to use the tool, shared by both engines.
    pub fn description(self) -> &'static str {
        match self {
            Tool::Notify => NOTIFY_DESCRIPTION,
            Tool::Ask => ASK_DESCRIPTION,
            Tool::FollowUp => FOLLOW_UP_DESCRIPTION,
        }
    }

    /// The tool's arguments, as JSON Schema properties.
    pub fn properties(self) -> Value {
        let s = |d: &str| json!({"type": "string", "description": d});
        match self {
            Tool::Notify => json!({ "text": s("What to tell the person, up to 2,000 characters") }),
            Tool::Ask => json!({
                "question": s("The question, up to 2,000 characters"),
                "choices": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Up to 5 short answers the person can pick, each up to 80 characters"
                }
            }),
            Tool::FollowUp => json!({
                "note": s("What to do when the follow-up runs, up to 2,000 characters"),
                "at": s("When, in RFC 3339 with an offset, e.g. 2026-10-21T09:00:00+02:00"),
                "in": s("How long from now: a whole number and m, h or d, e.g. 90m, 2h, 1d")
            }),
        }
    }

    pub fn required(self) -> &'static [&'static str] {
        match self {
            Tool::Notify => &["text"],
            Tool::Ask => &["question"],
            Tool::FollowUp => &["note"],
        }
    }

    /// The tool as Dots' own `dots` MCP server lists it.
    pub fn mcp_schema(self) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "inputSchema": {
                "type": "object",
                "properties": self.properties(),
                "required": self.required()
            }
        })
    }
}

/// When a follow-up is due, as the call gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    At(DateTime<Utc>),
    In(TimeDelta),
}

/// A call whose input passed the rules (`check`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Notify {
        text: String,
    },
    Ask {
        question: String,
        choices: Option<Vec<String>>,
    },
    FollowUp {
        note: String,
        when: When,
    },
}

/// The input rules of section 2: what a call of `tool` with `input` asks for, or the rule it
/// breaks, in one sentence for the model. The follow-up limits are checked when it is
/// scheduled (`followups::due_at`), against the loop's clock.
pub fn check(tool: Tool, input: &Value) -> std::result::Result<Request, String> {
    match tool {
        Tool::Notify => Ok(Request::Notify {
            text: text(input, "text")?,
        }),
        Tool::Ask => Ok(Request::Ask {
            question: text(input, "question")?,
            choices: choices(input)?,
        }),
        Tool::FollowUp => Ok(Request::FollowUp {
            note: text(input, "note")?,
            when: when(input)?,
        }),
    }
}

/// The string argument `key`, trimmed: 1 to `MAX_TEXT` characters.
fn text(input: &Value, key: &str) -> std::result::Result<String, String> {
    let Some(raw) = input.get(key).and_then(Value::as_str) else {
        return Err(format!("`{key}` must be a string of 1 to 2,000 characters."));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("`{key}` must not be blank."));
    }
    if trimmed.chars().count() > MAX_TEXT {
        return Err(format!("`{key}` must be at most 2,000 characters."));
    }
    Ok(trimmed.to_string())
}

/// The optional `choices`: 1 to `MAX_CHOICES` strings of 1 to `MAX_CHOICE` characters, trimmed.
fn choices(input: &Value) -> std::result::Result<Option<Vec<String>>, String> {
    let list = match input.get("choices") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(list)) if (1..=MAX_CHOICES).contains(&list.len()) => list,
        Some(_) => return Err(CHOICES_RULE.into()),
    };
    let mut out = Vec::new();
    for item in list {
        let Some(choice) = item.as_str().map(str::trim) else {
            return Err(CHOICES_RULE.into());
        };
        if !(1..=MAX_CHOICE).contains(&choice.chars().count()) {
            return Err(CHOICE_RULE.into());
        }
        out.push(choice.to_string());
    }
    Ok(Some(out))
}

/// The one of `at` and `in` that is given; a blank string counts as absent.
fn when(input: &Value) -> std::result::Result<When, String> {
    let given = |key: &str| {
        input
            .get(key)
            .filter(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()))
    };
    match (given("at"), given("in")) {
        (Some(at), None) => at
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|t| When::At(t.with_timezone(&Utc)))
            .ok_or_else(|| AT_RULE.to_string()),
        (None, Some(within)) => within
            .as_str()
            .and_then(parse_in)
            .map(When::In)
            .ok_or_else(|| IN_RULE.to_string()),
        _ => Err(ONE_OF_RULE.into()),
    }
}

/// `in` as a duration: digits, then `m`, `h` or `d`. A number too large for a duration reads as
/// the longest one, which the 30-day limit refuses.
fn parse_in(s: &str) -> Option<TimeDelta> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let digits = &s[..s.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = digits.parse().unwrap_or(i64::MAX);
    let delta = match unit {
        'm' => TimeDelta::try_minutes(n),
        'h' => TimeDelta::try_hours(n),
        'd' => TimeDelta::try_days(n),
        _ => return None,
    };
    Some(delta.unwrap_or(TimeDelta::MAX))
}

/// `t` in local time as a person reads it in a result or a prompt, e.g. `Tue 21 Oct 09:00`.
pub fn local_label(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%a %-d %b %H:%M").to_string()
}
```

**Create `crates/dots-core/src/followups.rs`:**

```rust
//! Follow-ups (check-ins spec sections 3 and 6): runs a dot schedules for itself. A follow-up
//! continues the thread of the run that scheduled it, with the dot's note, once it is due.
//! This module holds the limits, checked when the tool is called, and the follow-up loop.

use chrono::{DateTime, TimeDelta, Utc};

use crate::checkins::When;
use crate::{Error, Result};

/// A follow-up is due at least this long after it is scheduled.
pub const MIN_AHEAD: TimeDelta = TimeDelta::minutes(5);
/// A follow-up is due at most this long after it is scheduled.
pub const MAX_AHEAD: TimeDelta = TimeDelta::days(30);
/// The most pending follow-ups a dot may have.
pub const MAX_PENDING: usize = 3;
/// A follow-up that fires later than this after it was due says how late it is.
pub const LATE_AFTER: TimeDelta = TimeDelta::minutes(15);
/// How long after each failed start the next try waits. The start after the last of them is
/// the last: when it fails too, the follow-up is skipped.
pub const RETRY_DELAYS: [TimeDelta; 3] = [
    TimeDelta::minutes(1),
    TimeDelta::minutes(5),
    TimeDelta::minutes(30),
];

pub const TOO_SOON: &str = "A follow-up must be at least 5 minutes away.";
pub const TOO_FAR: &str = "A follow-up must be at most 30 days away.";
pub const TOO_MANY: &str =
    "This dot already has 3 pending follow-ups; that is the most it may have.";

/// When a follow-up asked for at `now` is due, or the limit it breaks (section 3, in order:
/// `MIN_AHEAD`, then `MAX_AHEAD`; `MAX_PENDING` is checked when it is stored).
pub fn due_at(when: When, now: DateTime<Utc>) -> std::result::Result<DateTime<Utc>, &'static str> {
    let ahead = match when {
        When::At(t) => t - now,
        When::In(d) => d,
    };
    if ahead < MIN_AHEAD {
        return Err(TOO_SOON);
    }
    if ahead > MAX_AHEAD {
        return Err(TOO_FAR);
    }
    Ok(now + ahead)
}

/// A time the store wrote (`util::timestamp`).
pub fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Invalid(format!("stored time {s:?}: {e}")))
}
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test checkins_test`
Expected: PASS: 5 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/lib.rs crates/dots-core/src/checkins.rs crates/dots-core/src/followups.rs crates/dots-core/tests/checkins_test.rs
git commit -m "feat(core): describe the check-in tools and check their input"
```

### Task 4: Perform a check-in call: store, announce and limit

`CheckIns::call` performs an allowed call for both engines: a note or a question is stored and announced (`MessagePosted`), one question per run; a follow-up is scheduled by `FollowUps::schedule` within its limits, in the order of section 3, against the follow-up loop's clock, and announced (`FollowUpsUpdated`). A broken rule is answered in its one sentence; a call that cannot be stored is answered "Not delivered: <reason>." and logged. `FollowUps` here is the handle the loop of Task 6 runs on: the clock, the shared thread lock, the wake and the pause.

**Files:**

- Modify: `crates/dots-core/src/events.rs` (import; three variants after `LibraryUpdated`)
- Modify: `crates/dots-core/src/followups.rs` (imports; `FollowUps` appended)
- Modify: `crates/dots-core/src/checkins.rs` (module doc, imports; `CheckIns` appended)
- Test: `crates/dots-core/tests/checkins_call_test.rs` (new)

**Interfaces:**

- Consumes: Task 1's `Store` methods, Task 3's `check`, `Request`, `When`, `local_label`, `due_at`, `parse_time`, the constants; `scheduler::Clock` (exists, unchanged).
- Produces:
  - `RuntimeEvent::MessagePosted { message: Message }`, `RuntimeEvent::MessageUpdated { message: Message }`, `RuntimeEvent::FollowUpsUpdated { dot_id: String }` (JSON types `message_posted`, `message_updated`, `follow_ups_updated`).
  - `pub struct FollowUps` with `fn new(store: Store, bus: Bus, clock: Arc<dyn Clock>) -> Arc<FollowUps>`, `fn thread_lock(&self) -> Arc<tokio::sync::Mutex<()>>`, `fn wake(&self)`, `fn set_paused(&self, paused: bool)`, `fn is_paused(&self) -> bool`, `async fn schedule(&self, run: &Run, note: &str, when: When) -> Result<FollowUp>` (`Error::Invalid(<limit sentence>)`), `async fn list(&self, dot_id: Option<&str>) -> Result<Vec<FollowUp>>` (pending, soonest first), `async fn cancel(&self, id: &str) -> Result<FollowUp>`.
  - `#[derive(Clone)] pub struct CheckIns` with `fn new(store: Store, bus: Bus, follow_ups: Arc<FollowUps>) -> CheckIns` and `async fn call(&self, run_id: &str, tool: Tool, input: &Value) -> std::result::Result<String, String>`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/checkins_call_test.rs`:**

```rust
//! A check-in call performed (check-ins spec sections 2, 3 and 5): a note and a question are
//! stored and announced, one question per run; a follow-up is scheduled within its limits, in
//! order; calls made at once cannot pass the limits; and a call that cannot be stored is not
//! delivered, without failing the run.

mod common;

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::checkins::{local_label, CheckIns, Tool, ASKED, ASKED_ALREADY, SENT};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::followups::{FollowUps, TOO_FAR, TOO_MANY, TOO_SOON};
use dots_core::model::{FollowUpStatus, MessageKind, NewRun, Run, TriggerKind};
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

struct Setup {
    dir: TempDir,
    store: Store,
    checkins: CheckIns,
    follow_ups: Arc<FollowUps>,
    events: Receiver<RuntimeEvent>,
    /// A queued run of the dot `checker`: the run that calls the tools.
    run: Run,
}

/// Check-ins on a fake clock at `T0`.
async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let follow_ups = FollowUps::new(store.clone(), bus.clone(), FakeClock::new(t0()));
    let checkins = CheckIns::new(store.clone(), bus.clone(), follow_ups.clone());
    let dot = store.create_dot(&spec("checker")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    Setup {
        dir,
        store,
        checkins,
        follow_ups,
        events: bus.subscribe(),
        run,
    }
}

impl Setup {
    async fn call(&self, tool: Tool, input: Value) -> Result<String, String> {
        self.checkins.call(&self.run.id, tool, &input).await
    }
}

#[tokio::test]
async fn a_note_is_stored_and_announced() {
    let mut t = setup().await;
    assert_eq!(
        t.call(Tool::Notify, json!({"text": " Half way. "})).await,
        Ok(SENT.to_string())
    );
    let stored = t.store.list_messages(None).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, MessageKind::Note);
    assert_eq!(stored[0].text, "Half way.");
    assert_eq!(stored[0].run_id.as_deref(), Some(t.run.id.as_str()));
    match t.events.try_recv().unwrap() {
        RuntimeEvent::MessagePosted { message } => assert_eq!(message, stored[0]),
        other => panic!("{other:?}"),
    }
    // A broken rule is answered, and nothing is stored or announced.
    assert_eq!(
        t.call(Tool::Notify, json!({"text": ""})).await,
        Err("`text` must not be blank.".to_string())
    );
    assert_eq!(t.store.list_messages(None).await.unwrap().len(), 1);
    assert!(t.events.try_recv().is_err());
}

#[tokio::test]
async fn a_run_asks_one_question_with_its_choices() {
    let t = setup().await;
    assert_eq!(
        t.call(
            Tool::Ask,
            json!({"question": "Ship it?", "choices": ["yes", "no"]})
        )
        .await,
        Ok(ASKED.to_string())
    );
    assert_eq!(
        t.call(Tool::Ask, json!({"question": "Really?"})).await,
        Err(ASKED_ALREADY.to_string())
    );
    let stored = t.store.list_messages(None).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, MessageKind::Question);
    assert_eq!(stored[0].text, "Ship it?");
    assert_eq!(
        stored[0].choices,
        Some(vec!["yes".to_string(), "no".to_string()])
    );
    // Another run of the dot asks its own.
    let other = t
        .store
        .create_run(&NewRun::new(&t.run.dot_id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(
        t.checkins
            .call(&other.id, Tool::Ask, &json!({"question": "And you?"}))
            .await,
        Ok(ASKED.to_string())
    );
}

#[tokio::test]
async fn a_follow_up_is_scheduled_within_its_limits_in_order() {
    let mut t = setup().await;
    let answer = t
        .call(
            Tool::FollowUp,
            json!({"note": "Check the build.", "in": "2h"}),
        )
        .await
        .unwrap();
    let pending = t.follow_ups.list(Some(&t.run.dot_id)).await.unwrap();
    assert_eq!(pending.len(), 1);
    let due = t0() + TimeDelta::hours(2);
    assert_eq!(pending[0].note, "Check the build.");
    assert_eq!(pending[0].from_run_id, t.run.id);
    assert_eq!(pending[0].due_at, timestamp(due));
    assert_eq!(pending[0].created_at, T0);
    assert_eq!(
        answer,
        format!(
            "Follow-up {} scheduled for {}.",
            pending[0].id,
            local_label(due)
        )
    );
    assert!(matches!(
        t.events.try_recv().unwrap(),
        RuntimeEvent::FollowUpsUpdated { dot_id } if dot_id == t.run.dot_id
    ));
    for later in ["3h", "4h"] {
        assert!(t
            .call(Tool::FollowUp, json!({"note": "n", "in": later}))
            .await
            .is_ok());
    }
    // The limits in order: a call that breaks two of them is told the first.
    assert_eq!(
        t.call(Tool::FollowUp, json!({"note": "n", "in": "1m"})).await,
        Err(TOO_SOON.to_string())
    );
    assert_eq!(
        t.call(Tool::FollowUp, json!({"note": "n", "in": "31d"})).await,
        Err(TOO_FAR.to_string())
    );
    assert_eq!(
        t.call(Tool::FollowUp, json!({"note": "n", "in": "1h"})).await,
        Err(TOO_MANY.to_string())
    );
    // Cancelling one makes room.
    let cancelled = t.follow_ups.cancel(&pending[0].id).await.unwrap();
    assert_eq!(cancelled.status, FollowUpStatus::Cancelled);
    assert!(t
        .call(Tool::FollowUp, json!({"note": "n", "in": "1h"}))
        .await
        .is_ok());
}

/// Review Focus 2: Claude may issue several tool calls in one turn.
#[tokio::test]
async fn parallel_calls_store_one_question_and_three_follow_ups() {
    let t = setup().await;
    let first = json!({"question": "First?"});
    let second = json!({"question": "Second?"});
    let (a, b) = tokio::join!(
        t.checkins.call(&t.run.id, Tool::Ask, &first),
        t.checkins.call(&t.run.id, Tool::Ask, &second),
    );
    let mut asked = [a, b];
    asked.sort();
    assert_eq!(
        asked,
        [Ok(ASKED.to_string()), Err(ASKED_ALREADY.to_string())]
    );

    let inputs: Vec<Value> = ["1h", "2h", "3h", "4h"]
        .iter()
        .map(|later| json!({"note": "n", "in": later}))
        .collect();
    let (a, b, c, d) = tokio::join!(
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[0]),
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[1]),
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[2]),
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[3]),
    );
    let refused: Vec<String> = [a, b, c, d].into_iter().filter_map(Result::err).collect();
    assert_eq!(refused, [TOO_MANY.to_string()]);
    assert_eq!(t.follow_ups.list(None).await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_call_that_cannot_be_stored_is_not_delivered() {
    let t = setup().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        t.dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("ALTER TABLE messages RENAME TO messages_away")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let answer = t
        .call(Tool::Notify, json!({"text": "Half way."}))
        .await
        .unwrap_err();
    assert!(
        answer.starts_with("Not delivered: ") && answer.ends_with('.'),
        "{answer}"
    );
    let unknown = t
        .checkins
        .call("no-such-run", Tool::Ask, &json!({"question": "q"}))
        .await
        .unwrap_err();
    assert_eq!(unknown, "Not delivered: not found: run no-such-run.");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test checkins_call_test`
Expected: FAIL to compile: `CheckIns`, `FollowUps` and `RuntimeEvent::MessagePosted` do not exist.

- [ ] **Step 3: Add the events, the follow-ups handle and `CheckIns`**

**In `crates/dots-core/src/events.rs`, replace:**

```rust
use crate::model::{Approval, Run, RunEventRecord};
```

**with:**

```rust
use crate::model::{Approval, Message, Run, RunEventRecord};
```

**In `crates/dots-core/src/events.rs`, replace:**

```rust
    LibraryUpdated {
        item_id: String,
    },
}
```

**with:**

```rust
    LibraryUpdated {
        item_id: String,
    },
    /// A dot posted a note or asked a question (check-ins spec section 5). The app notifies.
    MessagePosted {
        message: Message,
    },
    /// A message was read, answered or dismissed.
    MessageUpdated {
        message: Message,
    },
    /// A follow-up of the dot was scheduled, fired, cancelled or skipped.
    FollowUpsUpdated {
        dot_id: String,
    },
}
```

**In `crates/dots-core/src/followups.rs`, replace:**

```rust
use chrono::{DateTime, TimeDelta, Utc};

use crate::checkins::When;
use crate::{Error, Result};
```

**with:**

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Mutex, Notify};

use crate::checkins::When;
use crate::events::{Bus, RuntimeEvent};
use crate::model::{FollowUp, Run};
use crate::scheduler::Clock;
use crate::store::Store;
use crate::{Error, Result};
```

**Append to `crates/dots-core/src/followups.rs`:**

```rust

/// The follow-ups of every dot, and the loop that fires them (spec section 6). Held by the
/// runtime, by `CheckIns`, which schedules them, and by the loop.
pub struct FollowUps {
    store: Store,
    bus: Bus,
    clock: Arc<dyn Clock>,
    /// Held from reading a thread's end until the run that continues it is stored.
    /// `Runtime::chat` and `CheckIns::reply` take it too, so no two of them give one run two
    /// children.
    thread_lock: Arc<Mutex<()>>,
    wake: Notify,
    paused: AtomicBool,
}

impl FollowUps {
    pub fn new(store: Store, bus: Bus, clock: Arc<dyn Clock>) -> Arc<FollowUps> {
        Arc::new(FollowUps {
            store,
            bus,
            clock,
            thread_lock: Arc::new(Mutex::new(())),
            wake: Notify::new(),
            paused: AtomicBool::new(false),
        })
    }

    /// The lock of every writer that continues a thread.
    pub fn thread_lock(&self) -> Arc<Mutex<()>> {
        self.thread_lock.clone()
    }

    /// Wakes the loop for a pass: a follow-up was added or cancelled, or a dot changed.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// While schedules are paused no follow-up fires; a due one fires once the pause ends.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        self.wake();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Schedules a follow-up of `run`'s dot that continues `run`'s thread (spec section 3).
    /// The limits are checked in order against the loop's clock; a broken one is `Invalid`
    /// with its sentence.
    pub async fn schedule(&self, run: &Run, note: &str, when: When) -> Result<FollowUp> {
        let now = self.clock.now();
        let due = due_at(when, now).map_err(|limit| Error::Invalid(limit.into()))?;
        let Some(row) = self
            .store
            .add_follow_up(&run.dot_id, &run.id, due, note, MAX_PENDING, now)
            .await?
        else {
            return Err(Error::Invalid(TOO_MANY.into()));
        };
        self.updated(&row.dot_id);
        self.wake();
        Ok(row)
    }

    /// The pending follow-ups, of one dot or of all, soonest first.
    pub async fn list(&self, dot_id: Option<&str>) -> Result<Vec<FollowUp>> {
        self.store.list_pending_follow_ups(dot_id).await
    }

    /// Cancels a pending follow-up (the person's; dots have no cancel tool). `Conflict` once
    /// it fired, was skipped or was cancelled.
    pub async fn cancel(&self, id: &str) -> Result<FollowUp> {
        let row = self.store.cancel_follow_up(id).await?;
        self.updated(&row.dot_id);
        self.wake();
        Ok(row)
    }

    fn updated(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::FollowUpsUpdated {
            dot_id: dot_id.to_string(),
        });
    }
}
```

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
//! server on the Claude engine. This module holds their names, descriptions and input rules.

use chrono::{DateTime, Local, TimeDelta, Utc};
use serde_json::{json, Value};

use crate::model::EngineKind;
```

**with:**

```rust
//! server on the Claude engine. This module holds their names, descriptions and input rules,
//! and `CheckIns`, which performs an allowed call.

use std::sync::Arc;

use chrono::{DateTime, Local, TimeDelta, Utc};
use serde_json::{json, Value};

use crate::events::{Bus, RuntimeEvent};
use crate::followups::{parse_time, FollowUps};
use crate::model::{EngineKind, Message};
use crate::store::Store;
use crate::{Error, Result};
```

**Append to `crates/dots-core/src/checkins.rs`:**

```rust

/// Performs the check-in tools' calls for both engines (spec sections 2 and 3). Cheap to clone.
#[derive(Clone)]
pub struct CheckIns {
    store: Store,
    bus: Bus,
    follow_ups: Arc<FollowUps>,
}

impl CheckIns {
    pub fn new(store: Store, bus: Bus, follow_ups: Arc<FollowUps>) -> Self {
        Self {
            store,
            bus,
            follow_ups,
        }
    }

    /// Performs an allowed call of `tool` by the run `run_id`. `Ok` is the text the model gets;
    /// `Err` a tool error: the input rule or the limit the call breaks, in one sentence, or
    /// "Not delivered: <reason>." when it could not be stored (logged). The run goes on either
    /// way.
    pub async fn call(
        &self,
        run_id: &str,
        tool: Tool,
        input: &Value,
    ) -> std::result::Result<String, String> {
        let request = check(tool, input)?;
        match self.perform(run_id, request).await {
            Ok(text) => Ok(text),
            Err(Error::Invalid(rule)) => Err(rule),
            Err(e) => {
                tracing::warn!(run = %run_id, tool = tool.name(), "check-in not delivered: {e}");
                Err(format!("Not delivered: {e}."))
            }
        }
    }

    async fn perform(&self, run_id: &str, request: Request) -> Result<String> {
        let run = self.store.get_run(run_id).await?;
        match request {
            Request::Notify { text } => {
                let message = self
                    .store
                    .add_note(&run.dot_id, Some(&run.id), &text)
                    .await?;
                self.posted(message);
                Ok(SENT.to_string())
            }
            Request::Ask { question, choices } => {
                let asked = self
                    .store
                    .add_question(&run.dot_id, &run.id, &question, choices.as_deref())
                    .await?;
                match asked {
                    Some(message) => {
                        self.posted(message);
                        Ok(ASKED.to_string())
                    }
                    None => Err(Error::Invalid(ASKED_ALREADY.to_string())),
                }
            }
            Request::FollowUp { note, when } => {
                let row = self.follow_ups.schedule(&run, &note, when).await?;
                let due = parse_time(&row.due_at)?;
                Ok(format!(
                    "Follow-up {} scheduled for {}.",
                    row.id,
                    local_label(due)
                ))
            }
        }
    }

    fn posted(&self, message: Message) {
        let _ = self.bus.send(RuntimeEvent::MessagePosted { message });
    }
}
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test checkins_call_test --test checkins_test --test store_checkins_test`
Expected: PASS (`checkins_call_test`: 5 tests).

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass; no warnings. The app's `Notifier` ignores the new events (`_ => None`) until Task 9.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/events.rs crates/dots-core/src/followups.rs crates/dots-core/src/checkins.rs crates/dots-core/tests/checkins_call_test.rs
git commit -m "feat(core): perform check-in calls within their rules and limits"
```

### Task 5: Reply to a question through the shared thread lock

`CheckIns::reply` answers an open question as section 5 says: under the thread lock, the thread's end is read as `chat` reads it; a quiet end with a session is continued by a chat run, an end without one gets a fresh chat run, and a busy thread is refused with `chat`'s own sentences. The question is closed and its run stored in one transaction, so two answers at once start one run. `CheckIns` also lists, reads and dismisses messages. The runtime makes the follow-ups handle and `CheckIns` at start, shares the thread lock with `chat` (its `chat_lock` becomes that `Arc`), exposes `reply`, `checkins()` and `follow_ups()`, and counts waiting messages in `inbox_count`.

**Files:**

- Modify: `crates/dots-core/src/checkins.rs` (imports; `thread_busy` and a second `impl CheckIns` appended)
- Modify: `crates/dots-core/src/runtime.rs` (imports near line 11 and 17; `Runtime` fields near line 90; `Runtime::start` near lines 100 and 236; accessors after `library()` near line 587; `inbox_count` near line 461)
- Test: `crates/dots-core/tests/runtime_checkins_test.rs` (new)

**Interfaces:**

- Consumes: `Store::answer_question`, `Message::refusal` (Task 1); `FollowUps::new`, `FollowUps::thread_lock`, `CheckIns::new`, `CheckIns::call` (Task 4); `Runner::enqueued(&self, run: &Run)` (exists).
- Produces:
  - `pub fn thread_busy(status: RunStatus) -> Error` in `checkins` (`chat`'s two sentences).
  - `CheckIns::reply(&self, runner: &Runner, id: &str, text: &str) -> Result<Run>`; `CheckIns::list_messages(&self, dot_id: Option<&str>) -> Result<Vec<Message>>`; `CheckIns::mark_read(&self, id: &str) -> Result<Message>`; `CheckIns::dismiss(&self, id: &str) -> Result<Message>` (each change announced as `MessageUpdated`).
  - `Runtime::reply(&self, message_id: &str, text: &str) -> Result<Run>`, `Runtime::checkins(&self) -> &CheckIns`, `Runtime::follow_ups(&self) -> &Arc<FollowUps>`; `Runtime::inbox_count` adds `Store::count_waiting_messages`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/runtime_checkins_test.rs`:**

```rust
//! Asking and replying through the runtime (check-ins spec section 5): a reply continues a
//! quiet thread from its end, starts afresh when the end has no session, is refused while the
//! thread is busy and for a closed question, and two answers at once start one run. Runs are
//! stored but never started (`max_concurrent_runs` is 0), so each test builds its thread.

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::checkins::Tool;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, Message, MessageKind, NewRun, Run, RunStatus, TriggerKind, QUESTION_ANSWERED,
    QUESTION_DISMISSED,
};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

struct Rt {
    _dir: TempDir,
    rt: Runtime,
}

/// A runtime that stores runs and never starts one.
async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { _dir: dir, rt }
}

impl Rt {
    fn store(&self) -> &Store {
        self.rt.store()
    }

    async fn dot(&self, name: &str) -> Dot {
        self.rt.create_dot(spec(name)).await.unwrap()
    }

    /// A manual run of `dot` that ended with `status`, with the session `s1` when `session`.
    async fn ended(&self, dot: &Dot, status: RunStatus, session: bool) -> Run {
        let run = self
            .store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        if session {
            self.store().set_session(&run.id, "s1").await.unwrap();
        }
        self.store()
            .finish_run(&run.id, status, Some("Built it."), None)
            .await
            .unwrap()
    }

    /// A chat run that continues `run`; it stays queued.
    async fn chat_after(&self, run: &Run) -> Run {
        let mut new = NewRun::new(&run.dot_id, TriggerKind::Chat);
        new.payload = Some(json!({"message": "and now?"}));
        new.parent_run_id = Some(run.id.clone());
        new.root_run_id = Some(run.root_run_id.clone());
        self.store().create_run(&new).await.unwrap()
    }

    /// The question `text`, with the choices yes and no, asked in `run`.
    async fn ask(&self, run: &Run, text: &str) -> Message {
        let input = json!({"question": text, "choices": ["yes", "no"]});
        let answer = self.rt.checkins().call(&run.id, Tool::Ask, &input).await;
        assert!(answer.is_ok(), "{answer:?}");
        self.store()
            .list_messages(Some(&run.dot_id))
            .await
            .unwrap()
            .into_iter()
            .find(|m| {
                m.kind == MessageKind::Question && m.run_id.as_deref() == Some(run.id.as_str())
            })
            .unwrap()
    }
}

#[tokio::test]
async fn a_reply_continues_a_quiet_thread_from_its_end() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&run, "Ship it?").await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1, "an open question waits");
    let mut events = t.rt.subscribe();
    let reply = t.rt.reply(&question.id, " yes ").await.unwrap();
    assert_eq!(reply.trigger, TriggerKind::Chat);
    assert_eq!(reply.status, RunStatus::Queued);
    assert_eq!(reply.parent_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(reply.root_run_id, run.root_run_id);
    assert_eq!(reply.session_id.as_deref(), Some("s1"));
    assert_eq!(
        reply.payload,
        Some(json!({"message": "Answer to your question \"Ship it?\": yes"}))
    );
    let answered = t.store().get_message(&question.id).await.unwrap();
    assert_eq!(answered.answer_run_id.as_deref(), Some(reply.id.as_str()));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    let mut seen = Vec::new();
    while let Ok(ev) = events.try_recv() {
        seen.push(ev);
    }
    assert!(
        seen.iter()
            .any(|e| matches!(e, RuntimeEvent::RunUpdated { run } if run.id == reply.id)),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(
            e,
            RuntimeEvent::MessageUpdated { message }
                if message.id == question.id && message.answered_at.is_some()
        )),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_reply_to_a_thread_whose_end_has_no_session_starts_afresh() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let first = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&first, "Which branch?").await;
    // A chat that continued the thread failed before the dot started: no session to resume.
    let failed = t.chat_after(&first).await;
    t.store()
        .finish_run(&failed.id, RunStatus::Failed, None, Some("claude CLI not found"))
        .await
        .unwrap();
    let reply = t.rt.reply(&question.id, "main").await.unwrap();
    assert_eq!(reply.trigger, TriggerKind::Chat);
    assert_eq!(reply.parent_run_id, None);
    assert_eq!(reply.root_run_id, reply.id);
    assert_eq!(reply.session_id, None);
    assert_eq!(
        reply.payload,
        Some(json!({
            "message": "Earlier you asked the person: \"Which branch?\"\nTheir answer: main"
        }))
    );
}

#[tokio::test]
async fn a_reply_to_a_busy_thread_is_refused_with_the_words_of_chat() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&run, "Ship it?").await;
    let busy = t.chat_after(&run).await;
    let refused = t.rt.reply(&question.id, "yes").await;
    assert!(
        matches!(&refused, Err(Error::Conflict(m))
            if m.starts_with("the dot is still working on this chat")),
        "{refused:?}"
    );
    t.store()
        .set_status(&busy.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    let refused = t.rt.reply(&question.id, "yes").await;
    assert!(
        matches!(&refused, Err(Error::Conflict(m))
            if m.starts_with("this chat is waiting for an approval")),
        "{refused:?}"
    );
    // The same words as a chat message sent to that thread now.
    let chat = t.rt.chat(&dot.id, "and now?", Some(&busy.id)).await;
    assert_eq!(
        format!("{:?}", chat.unwrap_err()),
        format!("{:?}", refused.unwrap_err())
    );
    // The question stays open, and no refused reply stored a run.
    assert!(t.store().get_message(&question.id).await.unwrap().waits());
    assert_eq!(t.store().list_thread(&run.root_run_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_closed_question_or_a_note_cannot_be_answered() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let answered = t.ask(&run, "Ship it?").await;
    t.rt.reply(&answered.id, "yes").await.unwrap();
    let again = t.rt.reply(&answered.id, "no").await;
    assert!(
        matches!(&again, Err(Error::Conflict(m)) if m == QUESTION_ANSWERED),
        "{again:?}"
    );

    let other = t.ended(&dot, RunStatus::Succeeded, true).await;
    let dismissed = t.ask(&other, "Which branch?").await;
    let closed = t.rt.checkins().dismiss(&dismissed.id).await.unwrap();
    assert!(closed.dismissed_at.is_some());
    let late = t.rt.reply(&dismissed.id, "main").await;
    assert!(
        matches!(&late, Err(Error::Conflict(m)) if m == QUESTION_DISMISSED),
        "{late:?}"
    );

    let third = t.ended(&dot, RunStatus::Succeeded, true).await;
    let open = t.ask(&third, "Merge it?").await;
    assert!(matches!(
        t.rt.reply(&open.id, "  ").await,
        Err(Error::Invalid(_))
    ));
    t.rt.checkins()
        .call(&third.id, Tool::Notify, &json!({"text": "Half way."}))
        .await
        .unwrap();
    let note = t
        .store()
        .list_messages(Some(&dot.id))
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.kind == MessageKind::Note)
        .unwrap();
    assert!(matches!(
        t.rt.reply(&note.id, "ok").await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        t.rt.reply("no-such-message", "ok").await,
        Err(Error::NotFound(_))
    ));
}

/// Review Focus 3: a double click, or the Inbox and the chat bar together.
#[tokio::test]
async fn two_replies_at_once_start_one_run() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&run, "Ship it?").await;
    let (a, b) = tokio::join!(
        t.rt.reply(&question.id, "yes"),
        t.rt.reply(&question.id, "no")
    );
    assert_eq!(
        [&a, &b].iter().filter(|r| r.is_ok()).count(),
        1,
        "{a:?} {b:?}"
    );
    assert!(
        matches!(a.as_ref().err().or(b.as_ref().err()), Some(Error::Conflict(_))),
        "{a:?} {b:?}"
    );
    assert_eq!(t.store().list_thread(&run.root_run_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn messages_are_read_and_dismissed_and_the_waiting_ones_counted() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    t.rt.checkins()
        .call(&run.id, Tool::Notify, &json!({"text": "Half way."}))
        .await
        .unwrap();
    let question = t.ask(&run, "Ship it?").await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 2);
    let listed = t.rt.checkins().list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(listed.len(), 2);
    let note = listed
        .iter()
        .find(|m| m.kind == MessageKind::Note)
        .unwrap();
    let mut events = t.rt.subscribe();
    let read = t.rt.checkins().mark_read(&note.id).await.unwrap();
    assert!(read.read_at.is_some());
    assert!(matches!(
        events.try_recv().unwrap(),
        RuntimeEvent::MessageUpdated { message } if message.id == note.id
    ));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    // A question that was read still waits for its answer.
    t.rt.checkins().mark_read(&question.id).await.unwrap();
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    t.rt.checkins().dismiss(&question.id).await.unwrap();
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test runtime_checkins_test`
Expected: FAIL to compile: `Runtime::reply` and `Runtime::checkins` do not exist.

- [ ] **Step 3: Answer through `CheckIns` and wire the runtime**

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
use crate::model::{EngineKind, Message};
```

**with:**

```rust
use crate::model::{EngineKind, Message, NewRun, Run, RunStatus, TriggerKind};
use crate::runner::Runner;
```

**Append to `crates/dots-core/src/checkins.rs`:**

```rust

/// The refusal of a reply to a thread that has not ended, in the words `Runtime::chat` gives.
pub fn thread_busy(status: RunStatus) -> Error {
    Error::Conflict(
        match status {
            RunStatus::AwaitingApproval => {
                "this chat is waiting for an approval; answer it, then send your message again"
            }
            _ => {
                "the dot is still working on this chat; wait until it has finished, then send \
                 your message again"
            }
        }
        .into(),
    )
}

/// The person's side (spec section 5): the messages, and the answer to a question.
impl CheckIns {
    /// Every message that waits for the person and the newest others, of one dot or of all,
    /// newest first.
    pub async fn list_messages(&self, dot_id: Option<&str>) -> Result<Vec<Message>> {
        self.store.list_messages(dot_id).await
    }

    /// Marks a message read; a question stays open.
    pub async fn mark_read(&self, id: &str) -> Result<Message> {
        let message = self.store.mark_message_read(id).await?;
        self.updated(message.clone());
        Ok(message)
    }

    /// Dismisses an open question: it leaves the Inbox unanswered, and cannot be answered.
    pub async fn dismiss(&self, id: &str) -> Result<Message> {
        let message = self.store.dismiss_question(id).await?;
        self.updated(message.clone());
        Ok(message)
    }

    /// Answers the open question `id` with `text` (spec section 5). Under the thread lock the
    /// end of the question's thread is read as `Runtime::chat` reads it: a thread that has not
    /// ended is refused with `chat`'s sentences (`thread_busy`); a quiet end with a session is
    /// continued by a chat run; an end without one gets a chat run of its own. The question is
    /// closed and the run stored in one transaction, so a second answer is refused.
    pub async fn reply(&self, runner: &Runner, id: &str, text: &str) -> Result<Run> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("the answer must not be empty".into()));
        }
        let question = self.store.get_message(id).await?;
        if let Some(refusal) = question.refusal() {
            return Err(refusal);
        }
        let Some(asked_in) = &question.run_id else {
            return Err(Error::Invalid(format!(
                "message {id} was not asked by a run"
            )));
        };
        let asked = self.store.get_run(asked_in).await?;
        // Held until the new run is stored, as `Runtime::chat` holds it.
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        let thread = self.store.list_thread(&asked.root_run_id).await?;
        if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
            return Err(thread_busy(active.status));
        }
        let end = thread.last().unwrap_or(&asked);
        let mut new = NewRun::new(&asked.dot_id, TriggerKind::Chat);
        let message = if end.session_id.is_some() {
            new.parent_run_id = Some(end.id.clone());
            new.root_run_id = Some(end.root_run_id.clone());
            new.session_id = end.session_id.clone();
            new.workspace_path = end.workspace_path.clone();
            new.branch = end.branch.clone();
            new.base_commit = end.base_commit.clone();
            format!("Answer to your question \"{}\": {text}", question.text)
        } else {
            format!(
                "Earlier you asked the person: \"{}\"\nTheir answer: {text}",
                question.text
            )
        };
        new.payload = Some(json!({ "message": message }));
        let (answered, run) = self.store.answer_question(id, &new).await?;
        runner.enqueued(&run);
        self.updated(answered);
        Ok(run)
    }

    fn updated(&self, message: Message) {
        let _ = self.bus.send(RuntimeEvent::MessageUpdated { message });
    }
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::approvals::{ApprovalHub, DecideEffect};
```

**with:**

```rust
use crate::approvals::{ApprovalHub, DecideEffect};
use crate::checkins::CheckIns;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::events::{new_bus, Bus, RuntimeEvent};
```

**with:**

```rust
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::followups::FollowUps;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    /// Makes `chat` check a thread and add to it in one step.
    chat_lock: Mutex<()>,
    shutdown: CancellationToken,
}
```

**with:**

```rust
    /// Makes `chat` check a thread and add to it in one step. It is the follow-ups' thread lock
    /// (`FollowUps::thread_lock`), which `reply` and the follow-up loop hold too, so no two of
    /// them give one run two children.
    chat_lock: Arc<Mutex<()>>,
    checkins: CheckIns,
    follow_ups: Arc<FollowUps>,
    shutdown: CancellationToken,
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let bus = new_bus();
        let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
```

**with:**

```rust
        let bus = new_bus();
        let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
        // Check-ins and follow-ups (check-ins spec): made before the engines, which offer the
        // check-in tools, and before the runner, which the follow-up loop starts runs on.
        let follow_ups = FollowUps::new(store.clone(), bus.clone(), cfg.clock.clone());
        let checkins = CheckIns::new(store.clone(), bus.clone(), follow_ups.clone());
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            settings: Mutex::new(settings),
            chat_lock: Mutex::new(()),
            shutdown,
        })
```

**with:**

```rust
            settings: Mutex::new(settings),
            chat_lock: follow_ups.thread_lock(),
            checkins,
            follow_ups,
            shutdown,
        })
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        Ok(inbox.approvals.len()
            + inbox.review_runs.len()
            + inbox.attention.len()
            + inbox.tools_changed.len())
```

**with:**

```rust
        // Open questions and unread notes wait too (check-ins spec section 9).
        Ok(inbox.approvals.len()
            + inbox.review_runs.len()
            + inbox.attention.len()
            + inbox.tools_changed.len()
            + self.store.count_waiting_messages().await?)
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    /// The Library: what the `library_*` commands call (MCP/skills spec section 9).
    pub fn library(&self) -> &LibraryService {
        &self.library
    }
```

**with:**

```rust
    /// The Library: what the `library_*` commands call (MCP/skills spec section 9).
    pub fn library(&self) -> &LibraryService {
        &self.library
    }

    /// The dots' notes and questions: what the `message_*` commands call (check-ins spec
    /// section 10). Answering a question is `reply`.
    pub fn checkins(&self) -> &CheckIns {
        &self.checkins
    }

    /// The follow-ups dots scheduled: what the `follow_up*` commands call.
    pub fn follow_ups(&self) -> &Arc<FollowUps> {
        &self.follow_ups
    }

    /// Answers the open question `message_id` with `text` (check-ins spec section 5): a run
    /// continues the question's thread, or starts afresh when its end has no session.
    pub async fn reply(&self, message_id: &str, text: &str) -> Result<Run> {
        self.checkins.reply(&self.runner, message_id, text).await
    }
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test runtime_checkins_test --test runtime_test`
Expected: PASS (`runtime_checkins_test`: 6 tests); `runtime_test::two_follow_ups_sent_at_once_do_not_fork_the_thread` still passes on the shared lock.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/checkins.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/runtime_checkins_test.rs
git commit -m "feat(core): reply to a dot's question under the shared thread lock"
```

### Task 6: Fire due follow-ups from a loop on the scheduler's clock

The loop of section 6 in `followups.rs`: a pass over the pending follow-ups, oldest due first, then a sleep until the soonest is due or 60 s. A wake (a follow-up added or cancelled, a dot changed, a pause or resume) and the end of any run wake it. While schedules are paused it does nothing; a disabled dot's follow-up is skipped with a note; a busy thread is left for later; a quiet thread is continued under the thread lock by a `follow_up` run, or a fresh one when its end has no session; a failed start is retried after 1, 5 and 30 minutes and then skipped with a note. The runtime starts the loop next to the scheduler and wakes or pauses it where it reloads or pauses the scheduler. The prompt of a follow-up run is its message when it resumes a session, else the dot's first prompt with a `## Follow-up` section.

**Files:**

- Modify: `crates/dots-core/src/followups.rs` (imports; the loop appended)
- Modify: `crates/dots-core/src/runtime.rs` (`Runtime::start` near line 220; `change_settings` near line 313; `update_dot`, `delete_dot`, `set_dot_enabled` near lines 642 to 668)
- Modify: `crates/dots-core/src/prompt.rs` (`build_prompt`)
- Test: `crates/dots-core/tests/followups_test.rs` (new), `crates/dots-core/tests/runtime_followups_test.rs` (new)
- Modify: `crates/dots-core/tests/engine_prompt_test.rs` (append one test)

**Interfaces:**

- Consumes: `Store::{list_pending_follow_ups, fire_follow_up, retry_follow_up, skip_follow_up, add_note, list_thread, get_run, get_dot}`; `FollowUps` fields and `updated` (Task 4); `scheduler::{Clock, MAX_SLEEP, ERROR_WAIT}`; `Runner::enqueued`; `checkins::local_label`.
- Produces: `FollowUps::spawn(self: &Arc<Self>, runner: Arc<Runner>, shutdown: CancellationToken) -> JoinHandle<()>`; `pub fn follow_up_run(row: &FollowUp, from: Option<&Run>, end: Option<&Run>, now: DateTime<Utc>) -> Result<NewRun>`; `pub fn late_words(due: DateTime<Utc>, now: DateTime<Utc>) -> String`; `build_prompt` handles `TriggerKind::FollowUp`. Run payload of a follow-up: `{"message": <text>, "follow_up_id": <id>}`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/followups_test.rs`:**

```rust
//! The follow-up loop on a clock the test moves by hand (check-ins spec section 6): a due
//! follow-up continues its thread; it waits while the thread is busy and while schedules are
//! paused; a disabled dot's is skipped with a note; one missed while Dots was closed fires once
//! and says how late it is; a cancelled one never fires; a run that cannot be stored is tried
//! after 1, 5 and 30 minutes and then skipped with a note; a deleted dot takes its follow-ups.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::checkins::local_label;
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::followups::{late_words, FollowUps, LATE_AFTER};
use dots_core::model::{
    Dot, EngineKind, FollowUp, FollowUpStatus, MessageKind, NewRun, Run, RunStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// The fake clock starts here.
const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// `secs` seconds after `T0`.
fn after(secs: i64) -> DateTime<Utc> {
    t0() + TimeDelta::seconds(secs)
}

/// Runner without a dispatcher: the runs the loop starts stay queued.
fn idle_runner(store: &Store, dir: &TempDir, bus: Bus) -> Arc<Runner> {
    Runner::new(
        store.clone(),
        bus,
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: String::new(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
        },
    )
}

/// The follow-up loop on a fake clock at `T0`, not started yet.
struct Harness {
    dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    bus: Bus,
    runner: Arc<Runner>,
    follow_ups: Arc<FollowUps>,
    shutdown: CancellationToken,
    sleeps: u64,
}

impl Harness {
    async fn new() -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let follow_ups = FollowUps::new(store.clone(), bus.clone(), clock.clone());
        Harness {
            dir,
            store,
            clock,
            bus,
            runner,
            follow_ups,
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
    }

    fn start(&self) {
        self.follow_ups
            .spawn(self.runner.clone(), self.shutdown.clone());
    }

    /// Stops the loop and starts a new one on the same database and clock, as a restart of
    /// Dots does.
    fn restart(&mut self) {
        self.shutdown.cancel();
        self.shutdown = CancellationToken::new();
        self.follow_ups =
            FollowUps::new(self.store.clone(), self.bus.clone(), self.clock.clone());
        self.start();
    }

    /// Waits until the loop sleeps again after its pass, and returns when it means to wake,
    /// in seconds after `T0`.
    async fn settle(&mut self) -> i64 {
        self.sleeps += 1;
        (self.clock.nth_sleep(self.sleeps).await - t0()).num_seconds()
    }

    /// Moves the clock to `after(secs)`, which must reach the loop's wake time, and waits for
    /// the pass that follows.
    async fn at(&mut self, secs: i64) -> i64 {
        self.clock.set(after(secs));
        self.settle().await
    }

    /// A dot and the run that schedules its follow-up: ended `succeeded` with the summary
    /// "Built it.", and with the session `s1` when `session`.
    async fn thread(&self, name: &str, session: bool) -> (Dot, Run) {
        let dot = self.store.create_dot(&spec(name)).await.unwrap();
        let run = self
            .store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        if session {
            self.store.set_session(&run.id, "s1").await.unwrap();
        }
        let run = self
            .store
            .finish_run(&run.id, RunStatus::Succeeded, Some("Built it."), None)
            .await
            .unwrap();
        (dot, run)
    }

    /// A follow-up of `run` with the note "check CI", due `secs` after `T0` and scheduled at
    /// `T0`. Written to the store directly: the wake of `FollowUps::schedule` would add a pass
    /// that the test does not count.
    async fn follow_up(&self, run: &Run, secs: i64) -> FollowUp {
        self.store
            .add_follow_up(&run.dot_id, &run.id, after(secs), "check CI", 3, t0())
            .await
            .unwrap()
            .unwrap()
    }

    async fn row(&self, follow_up: &FollowUp) -> FollowUp {
        self.store.get_follow_up(&follow_up.id).await.unwrap()
    }

    /// The run the follow-up fired.
    async fn fired_run(&self, follow_up: &FollowUp) -> Run {
        let row = self.row(follow_up).await;
        assert_eq!(row.status, FollowUpStatus::Fired, "{row:?}");
        self.store
            .get_run(row.fired_run_id.as_deref().unwrap())
            .await
            .unwrap()
    }

    /// Ends `run` with `status` and the session `s1`, as the runner does, and announces it.
    async fn end(&self, run: &Run, status: RunStatus) {
        self.store.set_session(&run.id, "s1").await.unwrap();
        let ended = self
            .store
            .finish_run(&run.id, status, Some("Waited."), None)
            .await
            .unwrap();
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run: ended });
    }

    /// Runs `sql` behind the store's back.
    async fn exec(&self, sql: &str) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(self.dir.path().join("dots.db"))
            .foreign_keys(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        sqlx::query(sql).execute(&pool).await.unwrap();
        pool.close().await;
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn a_due_follow_up_continues_its_thread_with_the_note() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("watcher", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    assert_eq!(
        h.settle().await,
        60,
        "nothing is due: the loop sleeps 60 s at most"
    );
    assert_eq!(h.at(600).await, 660);
    let run = h.fired_run(&follow_up).await;
    assert_eq!(run.trigger, TriggerKind::FollowUp);
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.parent_run_id.as_deref(), Some(from.id.as_str()));
    assert_eq!(run.root_run_id, from.root_run_id);
    assert_eq!(run.session_id.as_deref(), Some("s1"));
    assert_eq!(
        run.payload,
        Some(json!({
            "message": format!("Follow-up you scheduled on {}: check CI", local_label(t0())),
            "follow_up_id": follow_up.id,
        }))
    );
    assert!(h.follow_ups.list(Some(&dot.id)).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_follow_up_whose_thread_ended_without_a_session_starts_afresh() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("fresh", false).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    h.at(600).await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(run.parent_run_id, None);
    assert_eq!(run.root_run_id, run.id);
    assert_eq!(run.session_id, None);
    assert_eq!(
        run.payload.unwrap()["message"],
        format!(
            "Follow-up you scheduled on {}: check CI\nWhat the earlier run reported: Built it.",
            local_label(t0())
        )
    );
}

#[tokio::test]
async fn a_follow_up_waits_while_its_thread_is_busy_and_fires_when_the_run_ends() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("busy", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    // A chat message continued the thread; its run has not started.
    let mut chat = NewRun::new(&from.dot_id, TriggerKind::Chat);
    chat.parent_run_id = Some(from.id.clone());
    chat.root_run_id = Some(from.root_run_id.clone());
    chat.session_id = Some("s1".into());
    let busy = h.store.create_run(&chat).await.unwrap();
    h.start();
    h.settle().await;
    h.at(600).await;
    assert_eq!(
        h.row(&follow_up).await.status,
        FollowUpStatus::Pending,
        "no second run on a thread"
    );
    h.end(&busy, RunStatus::Succeeded).await;
    h.settle().await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(run.parent_run_id.as_deref(), Some(busy.id.as_str()));
}

#[tokio::test]
async fn a_follow_up_of_a_disabled_dot_is_skipped_with_a_note() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("off", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.store.set_dot_enabled(&dot.id, false).await.unwrap();
    let mut events = h.bus.subscribe();
    h.start();
    h.settle().await;
    h.at(600).await;
    let row = h.row(&follow_up).await;
    assert_eq!(row.status, FollowUpStatus::Skipped);
    assert_eq!(row.fired_run_id, None);
    assert_eq!(h.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 1);
    let notes = h.store.list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, MessageKind::Note);
    assert_eq!(notes[0].run_id, None);
    assert_eq!(
        notes[0].text,
        format!(
            "A follow-up was skipped because the dot is off. It was due {}: check CI",
            local_label(after(600))
        )
    );
    let (mut posted, mut updated) = (false, false);
    while let Ok(ev) = events.try_recv() {
        match ev {
            RuntimeEvent::MessagePosted { .. } => posted = true,
            RuntimeEvent::FollowUpsUpdated { dot_id } if dot_id == dot.id => updated = true,
            _ => {}
        }
    }
    assert!(posted && updated);
}

#[tokio::test]
async fn a_follow_up_waits_while_schedules_are_paused_and_fires_after() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("paused", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    h.follow_ups.set_paused(true);
    h.clock.set(after(900));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.clock.sleeps(), h.sleeps, "no pass while paused");
    assert_eq!(h.row(&follow_up).await.status, FollowUpStatus::Pending);
    h.follow_ups.set_paused(false);
    h.settle().await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(
        run.payload.unwrap()["message"],
        format!("Follow-up you scheduled on {}: check CI", local_label(t0())),
        "five minutes late is not late"
    );
}

#[tokio::test]
async fn a_follow_up_missed_while_dots_was_closed_fires_once_with_the_late_words() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("closed", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.clock.set(after(600 + 2 * 3600));
    h.start();
    h.settle().await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(
        run.payload.unwrap()["message"],
        format!(
            "Follow-up you scheduled on {}: check CI (this is 2 hours late; Dots was not running \
             at the time)",
            local_label(t0())
        )
    );
    h.restart();
    h.settle().await;
    assert_eq!(
        h.store.list_thread(&from.root_run_id).await.unwrap().len(),
        2,
        "it fires once"
    );
}

#[test]
fn a_late_follow_up_says_how_many_hours_late_it_is() {
    let due = t0();
    assert_eq!(late_words(due, due + LATE_AFTER), "");
    assert_eq!(
        late_words(due, due + TimeDelta::minutes(16)),
        " (this is 1 hour late; Dots was not running at the time)"
    );
    assert_eq!(
        late_words(due, due + TimeDelta::minutes(150)),
        " (this is 3 hours late; Dots was not running at the time)"
    );
}

#[tokio::test]
async fn a_cancelled_follow_up_never_fires() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("cancelled", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    let cancelled = h.follow_ups.cancel(&follow_up.id).await.unwrap();
    assert_eq!(cancelled.status, FollowUpStatus::Cancelled);
    assert_eq!(h.settle().await, 60, "the cancel woke the loop");
    h.at(600).await;
    assert_eq!(h.store.list_thread(&from.root_run_id).await.unwrap().len(), 1);
    assert!(matches!(
        h.follow_ups.cancel(&follow_up.id).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn a_run_that_cannot_be_stored_is_tried_after_1_5_and_30_minutes_then_skipped() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("stuck", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.exec("CREATE TRIGGER no_runs BEFORE INSERT ON runs BEGIN SELECT RAISE(FAIL, 'disk full'); END")
        .await;
    h.start();
    assert_eq!(h.settle().await, 60);
    assert_eq!(h.at(600).await, 660, "the first failure: again after 1 minute");
    let row = h.row(&follow_up).await;
    assert_eq!(
        (row.status, row.attempts, row.retry_at),
        (FollowUpStatus::Pending, 1, Some(timestamp(after(660))))
    );
    assert_eq!(h.at(660).await, 720);
    assert_eq!(
        h.row(&follow_up).await.retry_at,
        Some(timestamp(after(960))),
        "the second: again after 5 minutes"
    );
    assert_eq!(h.at(960).await, 1020);
    assert_eq!(
        h.row(&follow_up).await.retry_at,
        Some(timestamp(after(2760))),
        "the third: again after 30 minutes"
    );
    h.at(2760).await;
    let row = h.row(&follow_up).await;
    assert_eq!((row.status, row.attempts), (FollowUpStatus::Skipped, 4));
    let notes = h.store.list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(notes.len(), 1);
    let text = &notes[0].text;
    assert!(
        text.starts_with("A follow-up was skipped because its run could not be started ("),
        "{text}"
    );
    assert!(
        text.ends_with(&format!(
            "It was due {}: check CI",
            local_label(after(600))
        )),
        "{text}"
    );
}

#[tokio::test]
async fn a_deleted_dot_takes_its_follow_ups_with_it() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("gone", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    h.store.delete_dot(&dot.id).await.unwrap();
    assert_eq!(h.at(600).await, 660, "nothing is left to fire");
    assert!(matches!(
        h.store.get_follow_up(&follow_up.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
}
```

**Create `crates/dots-core/tests/runtime_followups_test.rs`:**

```rust
//! A chat message and a due follow-up on one thread at the same moment (check-ins spec section
//! 6, "Thread lock"): whichever comes first, no run of the thread gets two children.

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, wait_status};
use dots_core::checkins::When;
use dots_core::model::{FollowUpStatus, RunStatus, WorkspaceMode};
use dots_core::{Config, Error, Runtime};

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2030-01-07T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// Review Focus 4.
#[tokio::test]
async fn a_chat_and_a_due_follow_up_on_one_thread_give_one_run() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(t0());
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.clock = clock.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    let mut s = spec("racer");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = rt.create_dot(s).await.unwrap();
    let first = rt.run_now(&dot.id).await.unwrap();
    let first = wait_status(rt.store(), &first.id, RunStatus::Succeeded, 15).await;
    let follow_up = rt
        .follow_ups()
        .schedule(&first, "Look again.", When::In(TimeDelta::minutes(10)))
        .await
        .unwrap();

    let (chat, ()) = tokio::join!(rt.chat(&dot.id, "and now?", Some(&first.id)), async {
        clock.set(t0() + TimeDelta::minutes(11))
    });
    match &chat {
        Ok(run) => assert_eq!(run.parent_run_id.as_deref(), Some(first.id.as_str())),
        Err(Error::Conflict(_)) => {}
        Err(e) => panic!("{e}"),
    }
    // Whichever came first, the follow-up fires: at once, or when the chat's run has ended.
    let deadline = Instant::now() + Duration::from_secs(30);
    let thread = loop {
        let row = rt.store().get_follow_up(&follow_up.id).await.unwrap();
        let thread = rt.store().list_thread(&first.root_run_id).await.unwrap();
        if row.status == FollowUpStatus::Fired && thread.iter().all(|r| r.status.is_terminal()) {
            break thread;
        }
        assert!(
            Instant::now() < deadline,
            "the follow-up did not fire: {row:?} {thread:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let parents: Vec<&str> = thread
        .iter()
        .filter_map(|r| r.parent_run_id.as_deref())
        .collect();
    let unique: HashSet<&str> = parents.iter().copied().collect();
    assert_eq!(
        unique.len(),
        parents.len(),
        "a run has two children: {thread:#?}"
    );
    assert_eq!(thread.len(), if chat.is_ok() { 3 } else { 2 });
    rt.shutdown();
}
```

**Append to `crates/dots-core/tests/engine_prompt_test.rs`:**

```rust

#[test]
fn a_follow_up_run_sends_its_message_or_starts_afresh_with_it() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let message = "Follow-up you scheduled on Mon 7 Jan 10:00: check CI";
    let resumed = fake_run(
        TriggerKind::FollowUp,
        Some(json!({"message": message, "follow_up_id": "F1"})),
        Some("s"),
    );
    assert_eq!(
        build_prompt(&dot, &resumed, Path::new("C:/ws"), at(), "Be brief.", &[]),
        message
    );
    let fresh = fake_run(
        TriggerKind::FollowUp,
        Some(json!({
            "message": format!("{message}\nWhat the earlier run reported: nothing"),
            "follow_up_id": "F1"
        })),
        None,
    );
    let p = build_prompt(&dot, &fresh, Path::new("C:/ws"), at(), "", &[]);
    assert!(
        p.starts_with(
            "Do the thing.\n\n---\n\n## Follow-up\n\nFollow-up you scheduled on Mon 7 Jan 10:00: \
             check CI\nWhat the earlier run reported: nothing\n\n## Standing rules\n\n"
        ),
        "{p}"
    );
    assert!(!p.contains("## Trigger"), "{p}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test followups_test --test runtime_followups_test --test engine_prompt_test`
Expected: FAIL to compile: `FollowUps::spawn` and `late_words` do not exist; `engine_prompt_test` fails to compile with them, and `a_follow_up_run_sends_its_message_or_starts_afresh_with_it` fails once it compiles (the prompt has a `## Trigger` section).

- [ ] **Step 3: Add the loop, start it, and give follow-up runs their prompt**

**In `crates/dots-core/src/followups.rs`, replace:**

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Mutex, Notify};

use crate::checkins::When;
use crate::events::{Bus, RuntimeEvent};
use crate::model::{FollowUp, Run};
use crate::scheduler::Clock;
use crate::store::Store;
use crate::{Error, Result};
```

**with:**

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::checkins::{local_label, When};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{FollowUp, NewRun, Run, TriggerKind};
use crate::runner::Runner;
use crate::scheduler::{Clock, ERROR_WAIT, MAX_SLEEP};
use crate::store::Store;
use crate::{Error, Result};
```

**Append to `crates/dots-core/src/followups.rs`:**

```rust

/// The loop (spec section 6).
impl FollowUps {
    /// Runs the loop until `shutdown`: a pass over the pending follow-ups, then a sleep until
    /// the soonest is due or `MAX_SLEEP`, whichever is first. `wake`, a pause or a resume, and
    /// the end of any run wake it at once. The first pass runs at once and fires what came due
    /// while Dots was closed. `runner` announces the runs it starts and wakes the dispatcher.
    pub fn spawn(
        self: &Arc<Self>,
        runner: Arc<Runner>,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let this = self.clone();
        // Subscribed before the first pass, so no end of a run is missed.
        let mut events = self.bus.subscribe();
        tokio::spawn(async move {
            loop {
                let wake = match this.step(&runner).await {
                    Ok(wake) => wake,
                    Err(e) => {
                        tracing::error!("follow-up loop error: {e}");
                        Some(this.clock.now() + ERROR_WAIT)
                    }
                };
                let sleep = async {
                    match wake {
                        Some(at) => this.clock.sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                // A wake that came during the pass is taken before the clock is asked to sleep.
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = this.wake.notified() => {}
                    _ = run_ended(&mut events) => {}
                    _ = sleep => {}
                }
            }
        })
    }

    /// One turn of the loop: `None` while schedules are paused (only a wake moves it on), else
    /// a pass and the time to wake for the next one.
    async fn step(&self, runner: &Runner) -> Result<Option<DateTime<Utc>>> {
        if self.is_paused() {
            return Ok(None);
        }
        let now = self.clock.now();
        let mut wake = now + MAX_SLEEP;
        for row in self.store.list_pending_follow_ups(None).await? {
            let at = parse_time(row.retry_at.as_deref().unwrap_or(&row.due_at))?;
            if at > now {
                wake = wake.min(at);
                continue;
            }
            self.handle(runner, &row, now).await;
        }
        Ok(Some(wake))
    }

    /// One due row: skipped with a note while its dot is off; left for later while its thread
    /// is busy (the end of the run wakes the loop); else fired. A failure while firing it is a
    /// failed start (`failed_start`).
    async fn handle(&self, runner: &Runner, row: &FollowUp, now: DateTime<Utc>) {
        let fired = match self.store.get_dot(&row.dot_id).await {
            // Deleted meanwhile: its rows went with it.
            Err(Error::NotFound(_)) => return,
            Err(e) => Err(e),
            Ok(dot) if !dot.spec.enabled => {
                let text = format!(
                    "A follow-up was skipped because the dot is off. It was due {}: {}",
                    due_label(row),
                    row.note
                );
                if let Err(e) = self.skip(row, &text, false).await {
                    tracing::warn!(follow_up = %row.id, "skipping a follow-up failed: {e}");
                }
                return;
            }
            Ok(_) => self.fire(row, now).await,
        };
        match fired {
            Ok(Some(run)) => {
                tracing::info!(follow_up = %row.id, run = %run.id, "follow-up fired");
                runner.enqueued(&run);
                self.updated(&row.dot_id);
            }
            Ok(None) => {
                tracing::debug!(follow_up = %row.id, "thread busy: the follow-up waits");
            }
            // Cancelled since the pass read it.
            Err(Error::Conflict(_)) => {}
            Err(e) => self.failed_start(row, &e, now).await,
        }
    }

    /// Under the thread lock: `None` while a run of the thread has not ended, else the run that
    /// continues it, stored with the row (`Store::fire_follow_up`).
    async fn fire(&self, row: &FollowUp, now: DateTime<Utc>) -> Result<Option<Run>> {
        let _one_at_a_time = self.thread_lock.lock().await;
        let from = match self.store.get_run(&row.from_run_id).await {
            Ok(run) => Some(run),
            Err(Error::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        let thread = match &from {
            Some(from) => self.store.list_thread(&from.root_run_id).await?,
            None => Vec::new(),
        };
        if thread.iter().any(|r| !r.status.is_terminal()) {
            return Ok(None);
        }
        let new = follow_up_run(row, from.as_ref(), thread.last(), now)?;
        self.store.fire_follow_up(&row.id, &new).await.map(Some)
    }

    /// A start of `row` failed with `e`: it is tried again after the next of `RETRY_DELAYS`;
    /// once they are used up, it is skipped with a note that says why.
    async fn failed_start(&self, row: &FollowUp, e: &Error, now: DateTime<Utc>) {
        tracing::warn!(
            follow_up = %row.id, attempt = row.attempts + 1,
            "the follow-up's run could not be started: {e}"
        );
        let written = match RETRY_DELAYS.get(row.attempts as usize) {
            Some(delay) => match self.store.retry_follow_up(&row.id, now + *delay).await {
                Ok(_) => {
                    self.updated(&row.dot_id);
                    Ok(())
                }
                Err(e) => Err(e),
            },
            None => {
                let text = format!(
                    "A follow-up was skipped because its run could not be started ({e}). It was \
                     due {}: {}",
                    due_label(row),
                    row.note
                );
                self.skip(row, &text, true).await
            }
        };
        if let Err(e) = written {
            tracing::error!(follow_up = %row.id, "recording a failed start failed: {e}");
        }
    }

    /// Skips `row` and tells the person why with a note of Dots' own. `failed`: the start that
    /// failed last is counted.
    async fn skip(&self, row: &FollowUp, text: &str, failed: bool) -> Result<()> {
        if !self.store.skip_follow_up(&row.id, failed).await? {
            return Ok(());
        }
        let message = self.store.add_note(&row.dot_id, None, text).await?;
        let _ = self.bus.send(RuntimeEvent::MessagePosted { message });
        self.updated(&row.dot_id);
        Ok(())
    }
}

/// The run that serves `row` at `now` (spec section 6): it continues the thread's `end` when
/// that has a session, as `chat` does; else it starts afresh and says what the run `from`
/// reported.
pub fn follow_up_run(
    row: &FollowUp,
    from: Option<&Run>,
    end: Option<&Run>,
    now: DateTime<Utc>,
) -> Result<NewRun> {
    let head = format!(
        "Follow-up you scheduled on {}: {}{}",
        local_label(parse_time(&row.created_at)?),
        row.note,
        late_words(parse_time(&row.due_at)?, now)
    );
    let mut new = NewRun::new(&row.dot_id, TriggerKind::FollowUp);
    let message = match end.filter(|end| end.session_id.is_some()) {
        Some(end) => {
            new.parent_run_id = Some(end.id.clone());
            new.root_run_id = Some(end.root_run_id.clone());
            new.session_id = end.session_id.clone();
            new.workspace_path = end.workspace_path.clone();
            new.branch = end.branch.clone();
            new.base_commit = end.base_commit.clone();
            head
        }
        None => {
            let reported = from
                .and_then(|run| run.summary.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("nothing");
            format!("{head}\nWhat the earlier run reported: {reported}")
        }
    };
    new.payload = Some(json!({ "message": message, "follow_up_id": row.id }));
    Ok(new)
}

/// What a follow-up's message adds when it fires more than `LATE_AFTER` after `due`: how many
/// hours late it is, rounded, at least 1. Nothing otherwise.
pub fn late_words(due: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let late = now - due;
    if late <= LATE_AFTER {
        return String::new();
    }
    let hours = ((late.num_minutes() + 30) / 60).max(1);
    let unit = if hours == 1 { "hour" } else { "hours" };
    format!(" (this is {hours} {unit} late; Dots was not running at the time)")
}

/// When `row` was due, as the person reads it.
fn due_label(row: &FollowUp) -> String {
    parse_time(&row.due_at).map_or_else(|_| row.due_at.clone(), local_label)
}

/// Returns when a run has ended (its `RunUpdated`), or when events were lost and one may have.
async fn run_ended(events: &mut Receiver<RuntimeEvent>) {
    loop {
        match events.recv().await {
            Ok(RuntimeEvent::RunUpdated { run }) if run.status.is_terminal() => return,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return,
            Err(RecvError::Closed) => std::future::pending().await,
        }
    }
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
```

**with:**

```rust
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
        // Follow-ups wait while schedules are paused (check-ins spec section 6).
        follow_ups.set_paused(settings.schedules_paused);
        follow_ups.spawn(runner.clone(), shutdown.clone());
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        self.scheduler.set_paused(next.schedules_paused);
        *current = next.clone();
```

**with:**

```rust
        self.scheduler.set_paused(next.schedules_paused);
        self.follow_ups.set_paused(next.schedules_paused);
        *current = next.clone();
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        self.sync_schedule(&dot, dot.spec.enabled && !was_enabled)
            .await?;
        self.scheduler.reload();
        Ok(dot)
```

**with:**

```rust
        self.sync_schedule(&dot, dot.spec.enabled && !was_enabled)
            .await?;
        self.scheduler.reload();
        self.follow_ups.wake();
        Ok(dot)
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        if had_row {
            self.schedule_updated(id);
        }
        self.scheduler.reload();
        Ok(())
```

**with:**

```rust
        if had_row {
            self.schedule_updated(id);
        }
        self.scheduler.reload();
        self.follow_ups.wake();
        Ok(())
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        self.sync_schedule(&dot, enabled && !was_enabled).await?;
        self.scheduler.reload();
        Ok(dot)
```

**with:**

```rust
        self.sync_schedule(&dot, enabled && !was_enabled).await?;
        self.scheduler.reload();
        self.follow_ups.wake();
        Ok(dot)
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it, and ends with the run's context
/// collections; resumes and chat follow-ups send only their message.
```

**with:**

```rust
/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it, and ends with the run's context
/// collections; resumes, chat follow-ups and follow-up runs that resume a session send only
/// their message.
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        TriggerKind::Chat if run.session_id.is_some() => return message.to_string(),
```

**with:**

```rust
        TriggerKind::Chat | TriggerKind::FollowUp if run.session_id.is_some() => {
            return message.to_string()
        }
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    if run.trigger == TriggerKind::Chat {
        out.push_str("## Message from the user\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else {
```

**with:**

```rust
    if run.trigger == TriggerKind::Chat {
        out.push_str("## Message from the user\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else if run.trigger == TriggerKind::FollowUp {
        // A follow-up whose thread could not be resumed (check-ins spec section 6).
        out.push_str("## Follow-up\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else {
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test followups_test --test runtime_followups_test --test engine_prompt_test --test runtime_checkins_test --test runtime_schedule_test`
Expected: PASS (`followups_test`: 10 tests; `runtime_followups_test`: 1). The scheduler's own tests are unchanged and still pass.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/followups.rs crates/dots-core/src/runtime.rs crates/dots-core/src/prompt.rs crates/dots-core/tests/followups_test.rs crates/dots-core/tests/runtime_followups_test.rs crates/dots-core/tests/engine_prompt_test.rs
git commit -m "feat(core): fire due follow-ups from a loop on the scheduler's clock"
```

### Task 7: Serve the check-in tools from Dots' own MCP server to Claude runs

As `context` is served (context library spec section 4): the `dots` server lists `notify`, `ask` and `follow_up` once the runtime has given the runner its `CheckIns`, checks each call with `hub.check(run_id, <policy name>, input)` (waiting for an approval when a rule asks) and performs it with `CheckIns::call`; a call that is not performed is a tool result with `isError`. `--allowedTools` always names the three, so the CLI does not ask `approve` first. After a parked check-in approval, the resume prompt names the tool as the model knows it (`mcp__dots__follow_up`).

**Files:**

- Modify: `crates/dots-core/src/runner.rs` (imports near lines 4 and 14; `model_tool_name` near line 27; `Runner` fields near line 67; `Runner::new` near line 87; `set_checkins` and `checkins` after `run_tools` near line 120)
- Modify: `crates/dots-core/src/server/mcp.rs` (import near line 11; `tools/list` and `tools/call` near line 102; `call_check_in` before `call_approve`)
- Modify: `crates/dots-core/src/engine/claude/args.rs` (imports; `--allowedTools` near line 42)
- Modify: `crates/dots-core/src/runtime.rs` (`Runtime::start`, after `Runner::new`)
- Modify: `crates/dots-core/tests/claude_cli_test.rs` (the expected argv near line 156)
- Test: `crates/dots-core/tests/claude_checkins_test.rs` (new)

**Interfaces:**

- Consumes: `CheckIns::call`, `Tool::{ALL, from_name, from_policy_name, policy_name, model_name, mcp_schema}` (Tasks 3, 4); `ApprovalHub` as `PermissionGate::check`; `gateway::not_performed` (exists, `pub(super)`).
- Produces: `Runner::set_checkins(&self, checkins: CheckIns)`, `Runner::checkins(&self) -> Option<&CheckIns>`; the `dots` server's tools `notify`, `ask`, `follow_up`; `--allowedTools … mcp__dots__notify mcp__dots__ask mcp__dots__follow_up` on every Claude run.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
         --strict-mcp-config --setting-sources  --allowedTools Read LS Glob Grep Skill \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
```

**with:**

```rust
         --strict-mcp-config --setting-sources  --allowedTools Read LS Glob Grep Skill \
         mcp__dots__notify mcp__dots__ask mcp__dots__follow_up \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
```

**Create `crates/dots-core/tests/claude_checkins_test.rs`:**

```rust
//! The check-in tools on the Claude engine (check-ins spec section 2): Dots' own `dots` MCP
//! server lists `notify`, `ask` and `follow_up`, checks each call against the dot's policy
//! under `Notify`, `Ask` and `FollowUp`, and performs it; `--allowedTools` names them, so the
//! CLI does not ask `approve` first. Driven through `fake-claude`.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::{ApprovalHub, DecideEffect};
use dots_core::checkins::{local_label, CheckIns, ASKED, ASKED_ALREADY, SENT};
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::followups::{FollowUps, TOO_FAR, TOO_MANY, TOO_SOON};
use dots_core::model::{
    Dot, DotSpec, EngineKind, MessageKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind,
    WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::SystemClock;
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    base: String,
    args_out: PathBuf,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, the local server, and
/// the check-ins on the PC's clock. Without `dispatch` nothing runs.
async fn env(script: Value, dispatch: bool) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let args_out = dir.path().join("args.json");
    let engine = ClaudeEngine::new(
        PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")),
        dir.path().join("scratch"),
    )
    .with_env("FAKE_CLAUDE_SCRIPT", &script_path.to_string_lossy())
    .with_env("FAKE_CLAUDE_ARGS_OUT", &args_out.to_string_lossy());
    let listener = bind(0).await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: format!("{base}/mcp"),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_secs(2),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    let follow_ups = FollowUps::new(store.clone(), bus.clone(), Arc::new(SystemClock));
    runner.set_checkins(CheckIns::new(store.clone(), bus.clone(), follow_ups));
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner: runner.clone(),
            hub: hub.clone(),
        },
        CancellationToken::new(),
    );
    if dispatch {
        runner.spawn_dispatcher(CancellationToken::new());
    }
    Env {
        dir,
        store,
        hub,
        runner,
        base,
        args_out,
    }
}

/// A Claude dot in the test folder, under the Sandboxed preset unless `edit` changes it.
async fn claude_dot(e: &Env, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec("checker");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 0;
    edit(&mut s);
    e.store.create_dot(&s).await.unwrap()
}

async fn start(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

/// A run of `dot` that the test calls the `dots` server for, and its secret. It never starts.
async fn registered(e: &Env, dot: &Dot) -> (Run, String) {
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let secret = e.runner.register_secret(&run.id);
    (run, secret)
}

/// What `fake-claude` recorded of its last start.
fn recorded(e: &Env) -> Value {
    serde_json::from_str(&std::fs::read_to_string(&e.args_out).unwrap()).unwrap()
}

/// The values after `flag`, up to the next flag.
fn values_of(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .skip_while(|a| *a != flag)
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect()
}

async fn events(e: &Env, run_id: &str, kind: &str) -> Vec<RunEventRecord> {
    e.store
        .list_events(run_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == kind)
        .collect()
}

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

fn success() -> Value {
    json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                    "usage": {"input_tokens": 1, "output_tokens": 1}}})
}

fn mcp_call(server: &str, tool: &str, arguments: Value) -> Value {
    json!({"mcp_call": {"server": server, "tool": tool, "arguments": arguments}})
}

/// The answer of Dots' own server to `body`, for the run of `secret`.
async fn rpc(e: &Env, secret: &str, body: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{}/mcp", e.base))
        .bearer_auth(secret)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Calls the `dots` server's tool `name` for the run of `secret`.
async fn call(e: &Env, secret: &str, name: &str, arguments: Value) -> Value {
    rpc(
        e,
        secret,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": name, "arguments": arguments}}),
    )
    .await
}

fn text(answer: &Value) -> &str {
    answer["result"]["content"][0]["text"].as_str().unwrap()
}

/// The resume run of `parent`, once the runner made it.
async fn resume_child(e: &Env, parent: &Run) -> Run {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let runs = e.store.list_runs(Some(&parent.dot_id), 10).await.unwrap();
        if let Some(r) = runs
            .into_iter()
            .find(|r| r.parent_run_id.as_deref() == Some(parent.id.as_str()))
        {
            return r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no resume run appeared"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_claude_run_checks_in_through_dots_own_server() {
    let e = env(
        json!([init(),
               {"mcp_list": "dots"},
               mcp_call("dots", "notify", json!({"text": "Half way there."})),
               mcp_call("dots", "ask", json!({"question": "Ship it?", "choices": ["yes", "no"]})),
               mcp_call("dots", "ask", json!({"question": "Really?"})),
               mcp_call("dots", "follow_up", json!({"note": "Check the build.", "in": "2h"})),
               mcp_call("dots", "notify", json!({"text": "  "})),
               success()]),
        true,
    )
    .await;
    let dot = claude_dot(&e, |_| {}).await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let args: Vec<String> = serde_json::from_value(recorded(&e)["args"].clone()).unwrap();
    let allowed = values_of(&args, "--allowedTools");
    for tool in ["mcp__dots__notify", "mcp__dots__ask", "mcp__dots__follow_up"] {
        assert!(allowed.contains(&tool.to_string()), "{args:?}");
    }
    for name in ["Notify", "Ask", "FollowUp"] {
        assert!(!allowed.contains(&name.to_string()), "{args:?}");
    }
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(
        listed["tools"],
        json!(["approve", "notify", "ask", "follow_up"])
    );
    let calls: Vec<Value> = events(&e, &run.id, "tool_call")
        .await
        .into_iter()
        .map(|ev| ev.data["tool"].clone())
        .collect();
    assert_eq!(
        calls,
        [
            json!("mcp__dots__notify"),
            json!("mcp__dots__ask"),
            json!("mcp__dots__ask"),
            json!("mcp__dots__follow_up"),
            json!("mcp__dots__notify"),
        ]
    );
    let pending = e.store.list_pending_follow_ups(Some(&dot.id)).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].from_run_id, run.id);
    let due = DateTime::parse_from_rfc3339(&pending[0].due_at)
        .unwrap()
        .with_timezone(&Utc);
    let results: Vec<(String, bool)> = events(&e, &run.id, "tool_result")
        .await
        .into_iter()
        .map(|ev| {
            (
                ev.data["output"].as_str().unwrap().to_string(),
                ev.data["is_error"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            (SENT.to_string(), false),
            (ASKED.to_string(), false),
            (ASKED_ALREADY.to_string(), true),
            (
                format!(
                    "Follow-up {} scheduled for {}.",
                    pending[0].id,
                    local_label(due)
                ),
                false
            ),
            ("`text` must not be blank.".to_string(), true),
        ]
    );
    let messages = e.store.list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(messages.len(), 2);
    let question = messages
        .iter()
        .find(|m| m.kind == MessageKind::Question)
        .unwrap();
    assert_eq!(question.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(
        question.choices,
        Some(vec!["yes".to_string(), "no".to_string()])
    );
}

#[tokio::test]
async fn dots_own_server_checks_each_check_in_against_the_dots_policy() {
    let e = env(json!([]), false).await;
    let plain = claude_dot(&e, |_| {}).await;
    let denying = claude_dot(&e, |s| {
        s.name = "denying".into();
        s.policy.rules.insert(0, Rule::new("Notify", Action::Deny));
    })
    .await;
    let asking = claude_dot(&e, |s| {
        s.name = "asking".into();
        // deny > allow > ask: the preset's bare `FollowUp` allow would win over an ask rule.
        s.policy.rules.retain(|r| r.tool != "FollowUp");
        s.policy.rules.push(Rule::new("FollowUp", Action::Ask));
    })
    .await;

    let (_, plain_secret) = registered(&e, &plain).await;
    let listed = rpc(
        &e,
        &plain_secret,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["approve", "notify", "ask", "follow_up"]);
    assert_eq!(
        listed["result"]["tools"][3]["inputSchema"]["required"],
        json!(["note"])
    );
    let sent = call(&e, &plain_secret, "notify", json!({"text": "Hello."})).await;
    assert_eq!(text(&sent), SENT);
    assert!(sent["result"].get("isError").is_none(), "{sent}");

    // The dot's own rule decides: a `Notify` deny rule denies the call, and nothing is stored.
    let (_, denying_secret) = registered(&e, &denying).await;
    let denied = call(&e, &denying_secret, "notify", json!({"text": "Hello."})).await;
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert!(
        text(&denied).contains("Denied by the policy of dot 'denying'"),
        "{denied}"
    );
    assert!(e
        .store
        .list_messages(Some(&denying.id))
        .await
        .unwrap()
        .is_empty());

    // An ask rule asks: with no wait, the call parks on an approval and nothing is scheduled.
    let (asked_in, asking_secret) = registered(&e, &asking).await;
    let parked = call(
        &e,
        &asking_secret,
        "follow_up",
        json!({"note": "Later.", "in": "1h"}),
    )
    .await;
    assert_eq!(parked["result"]["isError"], true, "{parked}");
    let approvals = e.store.approvals_for_run(&asked_in.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "FollowUp");
    assert!(e
        .store
        .list_pending_follow_ups(Some(&asking.id))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn dots_own_server_answers_a_broken_rule_with_its_sentence() {
    let e = env(json!([]), false).await;
    let dot = claude_dot(&e, |_| {}).await;
    let (_, secret) = registered(&e, &dot).await;
    let refused = |answer: &Value| -> String {
        assert_eq!(answer["result"]["isError"], true, "{answer}");
        text(answer).to_string()
    };
    let far = (Utc::now() + TimeDelta::days(31)).to_rfc3339();
    assert_eq!(
        refused(&call(&e, &secret, "follow_up", json!({"note": "n", "in": "4m"})).await),
        TOO_SOON
    );
    assert_eq!(
        refused(&call(&e, &secret, "follow_up", json!({"note": "n", "at": &far})).await),
        TOO_FAR
    );
    assert_eq!(
        refused(
            &call(
                &e,
                &secret,
                "follow_up",
                json!({"note": "n", "in": "2h", "at": &far})
            )
            .await
        ),
        "Give exactly one of `at` or `in`."
    );
    assert_eq!(
        refused(
            &call(
                &e,
                &secret,
                "ask",
                json!({"question": "q", "choices": ["a", "b", "c", "d", "e", "f"]})
            )
            .await
        ),
        "`choices` must be a list of 1 to 5 strings."
    );
    assert_eq!(
        refused(&call(&e, &secret, "notify", json!({"text": "x".repeat(2_001)})).await),
        "`text` must be at most 2,000 characters."
    );
    for later in ["1h", "2h", "3h"] {
        let ok = call(&e, &secret, "follow_up", json!({"note": "n", "in": later})).await;
        assert!(ok["result"].get("isError").is_none(), "{ok}");
    }
    assert_eq!(
        refused(&call(&e, &secret, "follow_up", json!({"note": "n", "in": "4h"})).await),
        TOO_MANY
    );
    // Arguments that are not an object are a protocol error, as for `context`.
    let bad = rpc(
        &e,
        &secret,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "notify", "arguments": "hi"}}),
    )
    .await;
    assert_eq!(bad["error"]["code"], -32602, "{bad}");
}

#[tokio::test]
async fn a_parked_check_in_resumes_with_the_name_the_model_knows() {
    let e = env(
        json!([init(),
               mcp_call("dots", "follow_up", json!({"note": "Later.", "in": "1h"})),
               success()]),
        true,
    )
    .await;
    let dot = claude_dot(&e, |s| {
        s.policy.rules.retain(|r| r.tool != "FollowUp");
        s.policy.rules.push(Rule::new("FollowUp", Action::Ask));
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::AwaitingApproval, 20).await;
    let approval = e.store.approvals_for_run(&run.id).await.unwrap().remove(0);
    assert_eq!(approval.tool, "FollowUp");
    assert!(matches!(
        e.hub.decide(&approval.id, true, None).await.unwrap(),
        DecideEffect::Parked(_)
    ));
    e.runner.maybe_resume(&run.id).await.unwrap();
    let child = resume_child(&e, &run).await;
    let message = child.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        message.contains("you may now use mcp__dots__follow_up with the same input"),
        "{message}"
    );
    // The resumed run calls again, and the grant lets the call through.
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await;
    assert_eq!(
        e.store
            .list_pending_follow_ups(Some(&dot.id))
            .await
            .unwrap()
            .len(),
        1
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test claude_checkins_test --test claude_cli_test`
Expected: FAIL to compile: `Runner::set_checkins` does not exist; once it does, `argv_contains_required_flags_in_order` fails (`--allowedTools` lacks the three tools).

- [ ] **Step 3: Give the runner its check-ins and serve them**

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
use std::sync::{Arc, Mutex};
```

**with:**

```rust
use std::sync::{Arc, Mutex, OnceLock};
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
use crate::approvals::grant_key;
```

**with:**

```rust
use crate::approvals::grant_key;
use crate::checkins::{CheckIns, Tool as CheckInTool};
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
/// The name the run's model knows a policy tool by: the `Context` tool is `mcp__dots__context`
/// on the Claude engine, `context` on the local one; every other tool keeps its name.
fn model_tool_name(tool: &str, engine: EngineKind) -> &str {
    if tool == "Context" {
        context_tool_name(engine)
    } else {
        tool
    }
}
```

**with:**

```rust
/// The name the run's model knows a policy tool by: the `Context` tool is `mcp__dots__context`
/// on the Claude engine, `context` on the local one, and the check-in tools likewise
/// (`checkins::Tool::model_name`); every other tool keeps its name.
fn model_tool_name(tool: &str, engine: EngineKind) -> &str {
    if tool == "Context" {
        context_tool_name(engine)
    } else if let Some(check_in) = CheckInTool::from_policy_name(tool) {
        check_in.model_name(engine)
    } else {
        tool
    }
}
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    wake: Notify,
    resume_lock: tokio::sync::Mutex<()>,
}
```

**with:**

```rust
    wake: Notify,
    resume_lock: tokio::sync::Mutex<()>,
    /// The check-in tools Dots' own `dots` MCP server serves (check-ins spec section 2), set
    /// once by the runtime before any run starts (`set_checkins`).
    checkins: OnceLock<CheckIns>,
}
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            wake: Notify::new(),
            resume_lock: tokio::sync::Mutex::new(()),
        })
```

**with:**

```rust
            wake: Notify::new(),
            resume_lock: tokio::sync::Mutex::new(()),
            checkins: OnceLock::new(),
        })
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    /// The open tools of a running run (found by its secret, `run_for_secret`).
    pub fn run_tools(&self, run_id: &str) -> Option<Arc<RunTools>> {
        self.tools.lock().unwrap().get(run_id).cloned()
    }
```

**with:**

```rust
    /// The open tools of a running run (found by its secret, `run_for_secret`).
    pub fn run_tools(&self, run_id: &str) -> Option<Arc<RunTools>> {
        self.tools.lock().unwrap().get(run_id).cloned()
    }

    /// Gives Dots' own `dots` MCP server the check-in tools. The runtime calls it once, at
    /// start; a second call changes nothing.
    pub fn set_checkins(&self, checkins: CheckIns) {
        let _ = self.checkins.set(checkins);
    }

    /// The check-in tools, once the runtime has set them.
    pub fn checkins(&self) -> Option<&CheckIns> {
        self.checkins.get()
    }
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
use crate::context;
```

**with:**

```rust
use crate::checkins::Tool;
use crate::context;
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
        "tools/list" => {
            let mut tools = vec![approve_tool()];
            if has_contexts(&s, &run_id) {
                tools.push(context_tool());
            }
            json!({ "tools": tools })
        }
        "tools/call" => {
            let answer = match req.pointer("/params/name").and_then(Value::as_str) {
                Some("context") => call_context(&s, &run_id, &req).await,
                _ => call_approve(&s, &run_id, &req).await,
            };
```

**with:**

```rust
        "tools/list" => {
            let mut tools = vec![approve_tool()];
            if has_contexts(&s, &run_id) {
                tools.push(context_tool());
            }
            // Every run checks in (check-ins spec section 2), once the runtime has given the
            // runner the check-ins.
            if s.runner.checkins().is_some() {
                tools.extend(Tool::ALL.map(Tool::mcp_schema));
            }
            json!({ "tools": tools })
        }
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(Value::as_str);
            let answer = match (name, name.and_then(Tool::from_name)) {
                (Some("context"), _) => call_context(&s, &run_id, &req).await,
                (_, Some(tool)) => call_check_in(&s, &run_id, &req, tool).await,
                _ => call_approve(&s, &run_id, &req).await,
            };
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
async fn call_approve(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
```

**with:**

```rust
/// Checks a call of a check-in tool against the dot's policy under its policy name (`Notify`,
/// `Ask`, `FollowUp`), waiting for an approval when a rule asks, and performs it (check-ins
/// spec section 2). A call that is not performed is a tool result with `isError`, as for
/// `context`.
async fn call_check_in(
    s: &ServerState,
    run_id: &str,
    req: &Value,
    tool: Tool,
) -> Result<Value, (i64, String)> {
    let Some(checkins) = s.runner.checkins() else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let input = match s.hub.check(run_id, tool.policy_name(), input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    Ok(match checkins.call(run_id, tool, &input).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&text),
    })
}

async fn call_approve(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
use crate::engine::RunContext;
```

**with:**

```rust
use crate::checkins::Tool;
use crate::engine::RunContext;
use crate::model::EngineKind;
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
    if !ctx.tools.contexts().is_empty() {
        allow.push("mcp__dots__context".into());
    }
```

**with:**

```rust
    if !ctx.tools.contexts().is_empty() {
        allow.push("mcp__dots__context".into());
    }
    // It checks each call of its check-in tools too (check-ins spec section 2).
    allow.extend(Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
                plugins_dir: cfg.data_dir.join("plugins"),
            },
        );
        // Decisions parked before a restart (decided, but the resume never happened) are
```

**with:**

```rust
                plugins_dir: cfg.data_dir.join("plugins"),
            },
        );
        // Dots' own `dots` MCP server serves the check-in tools to Claude runs.
        runner.set_checkins(checkins.clone());
        // Decisions parked before a restart (decided, but the resume never happened) are
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test claude_checkins_test --test claude_cli_test --test claude_gateway_test --test claude_engine_test --test runtime_test`
Expected: PASS (`claude_checkins_test`: 4 tests). `claude_gateway_test` still sees only `approve` (and `context`) on runners without check-ins.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/runner.rs crates/dots-core/src/server/mcp.rs crates/dots-core/src/engine/claude/args.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/claude_cli_test.rs crates/dots-core/tests/claude_checkins_test.rs
git commit -m "feat(core): serve the check-in tools from Dots' own MCP server to Claude runs"
```

### Task 8: Give the local engine the check-in tools

As `context` is built in (context library spec section 4): the local tool table gains `notify`, `ask` and `follow_up` under their policy names, with their arguments (`choices` is a new argument type, an array the tool checks); `LocalEngine::with_checkins` offers them to every run, and an allowed call runs `CheckIns::call`. A parked check-in approval is performed by the resume run like any other approved local call.

**Files:**

- Create: `crates/dots-core/src/engine/local/tools/checkins.rs`
- Modify: `crates/dots-core/src/engine/local/tools/mod.rs` (module list; `TOOLS`; `Arg`; `ARGS`; the type check in `prepare`; the `schemas` doc)
- Modify: `crates/dots-core/src/engine/local/mod.rs` (imports; `LocalEngine`; `start`; `Session`; `run_allowed`)
- Modify: `crates/dots-core/src/engine/local/mcp_tools.rs` (`tool_schemas` near line 158)
- Modify: `crates/dots-core/src/runtime.rs` (the local engine in `Runtime::start`)
- Test: `crates/dots-core/tests/local_checkins_test.rs` (new)

**Interfaces:**

- Consumes: `CheckIns::call`, `Tool` (Tasks 3, 4); `tools::tool(name, description, properties, required) -> Value` (exists, `pub(super)`).
- Produces: `tools::checkins::schemas() -> Vec<Value>`; `tools::checkins::run(checkins: &CheckIns, run_id: &str, name: &str, input: &Value) -> ToolOutput`; `LocalEngine::with_checkins(self, checkins: CheckIns) -> Self`; `tools::alias("notify") == Some("Notify")`, `alias("ask") == Some("Ask")`, `alias("follow_up") == Some("FollowUp")`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/local_checkins_test.rs`:**

```rust
//! The check-in tools in a local-engine run (check-ins spec section 2): `notify`, `ask` and
//! `follow_up` are built in, checked under their policy names, and do what Dots' own MCP
//! server does for Claude runs.

mod common;
mod llm;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::checkins::{local_label, CheckIns, ASKED, ASKED_ALREADY, SENT};
use dots_core::engine::local::tools;
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::followups::FollowUps;
use dots_core::model::{
    DotSpec, EngineKind, MessageKind, NewRun, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::SystemClock;
use dots_core::store::Store;
use llm::{call, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Setup {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
}

/// A runner with the local engine, which has the check-in tools when `check_ins`.
async fn setup(check_ins: bool) -> Setup {
    let (dir, store) = temp_store().await;
    std::fs::create_dir(dir.path().join("ws")).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let mut engine =
        LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    if check_ins {
        let follow_ups = FollowUps::new(store.clone(), bus.clone(), Arc::new(SystemClock));
        engine = engine.with_checkins(CheckIns::new(store.clone(), bus.clone(), follow_ups));
    }
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus,
        engines,
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Setup { dir, store, runner }
}

impl Setup {
    fn spec(&self, llm: &Llm) -> DotSpec {
        let mut s = spec("checker");
        s.workdir = self.dir.path().join("ws").to_string_lossy().into_owned();
        s.workspace_mode = WorkspaceMode::Folder;
        s.engine = EngineKind::Local;
        s.endpoint_url = Some(llm.endpoint());
        s.model = "test-model".into();
        s
    }

    async fn run(&self, s: &DotSpec, status: RunStatus) -> String {
        let dot = self.store.create_dot(s).await.unwrap();
        let r = self
            .runner
            .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        wait_status(&self.store, &r.id, status, 20).await;
        r.id
    }
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_local_run_checks_in_with_its_built_in_tools() {
    let t = setup(true).await;
    let llm = Llm::start(vec![
        call("c1", "notify", json!({"text": "Half way there."})),
        call(
            "c2",
            "ask",
            json!({"question": "Ship it?", "choices": ["yes", "no"]}),
        ),
        call("c3", "ask", json!({"question": "Really?"})),
        call(
            "c4",
            "follow_up",
            json!({"note": "Check the build.", "in": "2h"}),
        ),
        call("c5", "notify", json!({"text": " "})),
        text("Asked."),
    ])
    .await;
    let run = t.run(&t.spec(&llm), RunStatus::Succeeded).await;

    let reqs = llm.requests();
    let names = tool_names(&reqs[0]);
    for name in ["notify", "ask", "follow_up"] {
        assert!(names.iter().any(|n| n == name), "{names:?}");
    }
    assert_eq!(tool_result(&reqs[1], "c1"), SENT);
    assert_eq!(tool_result(&reqs[2], "c2"), ASKED);
    assert_eq!(tool_result(&reqs[3], "c3"), ASKED_ALREADY);
    let pending = t.store.list_pending_follow_ups(None).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].from_run_id, run);
    let due = DateTime::parse_from_rfc3339(&pending[0].due_at)
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(
        tool_result(&reqs[4], "c4"),
        format!(
            "Follow-up {} scheduled for {}.",
            pending[0].id,
            local_label(due)
        )
    );
    assert_eq!(tool_result(&reqs[5], "c5"), "`text` must not be blank.");
    let messages = t.store.list_messages(None).await.unwrap();
    assert_eq!(messages.len(), 2);
    let question = messages
        .iter()
        .find(|m| m.kind == MessageKind::Question)
        .unwrap();
    assert_eq!(
        question.choices,
        Some(vec!["yes".to_string(), "no".to_string()])
    );
    // The transcript names each call by its policy name, as for every built-in tool.
    let calls: Vec<Value> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_call")
        .map(|ev| ev.data["tool"].clone())
        .collect();
    assert_eq!(
        calls,
        [
            json!("Notify"),
            json!("Ask"),
            json!("Ask"),
            json!("FollowUp"),
            json!("Notify")
        ]
    );
}

#[tokio::test]
async fn a_dots_own_follow_up_rule_still_decides_on_the_local_engine() {
    let t = setup(true).await;
    let llm = Llm::start(vec![
        call("c1", "follow_up", json!({"note": "Later.", "in": "1h"})),
        text("Could not."),
    ])
    .await;
    let mut s = t.spec(&llm);
    s.policy.rules.insert(0, Rule::new("FollowUp", Action::Deny));
    t.run(&s, RunStatus::Succeeded).await;
    let result = tool_result(&llm.requests()[1], "c1");
    assert!(result.to_lowercase().contains("denied"), "{result}");
    assert!(t
        .store
        .list_pending_follow_ups(None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_engine_without_check_ins_offers_none() {
    let t = setup(false).await;
    let llm = Llm::start(vec![text("Nothing to say.")]).await;
    t.run(&t.spec(&llm), RunStatus::Succeeded).await;
    let names = tool_names(&llm.requests()[0]);
    assert!(
        !names
            .iter()
            .any(|n| n == "notify" || n == "ask" || n == "follow_up"),
        "{names:?}"
    );
}

#[test]
fn the_check_in_tools_are_local_tools_with_their_policy_names() {
    assert_eq!(tools::alias("notify"), Some("Notify"));
    assert_eq!(tools::alias("ask"), Some("Ask"));
    assert_eq!(tools::alias("follow_up"), Some("FollowUp"));
    assert_eq!(tools::local_name("FollowUp"), Some("follow_up"));
    let ws = tempfile::tempdir().unwrap();
    let prepared = tools::prepare(
        "ask",
        &json!({"question": "Ship?", "choices": ["y"], "extra": 1}),
        ws.path(),
    )
    .unwrap();
    assert_eq!(
        prepared.input,
        json!({"question": "Ship?", "choices": ["y"]})
    );
    let err = tools::prepare(
        "ask",
        &json!({"question": "Ship?", "choices": "y"}),
        ws.path(),
    )
    .unwrap_err();
    assert!(err.contains("'choices' must be an array"), "{err}");
    let err = tools::prepare("follow_up", &json!({"in": "1h"}), ws.path()).unwrap_err();
    assert!(err.contains("'note'"), "{err}");
    let names: Vec<String> = tools::checkins::schemas()
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["notify", "ask", "follow_up"]);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test local_checkins_test`
Expected: FAIL to compile: `LocalEngine::with_checkins` and `tools::checkins` do not exist.

- [ ] **Step 3: Add the local tools**

**Create `crates/dots-core/src/engine/local/tools/checkins.rs`:**

```rust
//! The check-in tools of the local engine (`notify`, `ask`, `follow_up`; policy names `Notify`,
//! `Ask`, `FollowUp`): built in, as `context` is (check-ins spec section 2). What they do is
//! `crate::checkins`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::checkins::{CheckIns, Tool};

/// The function definitions, offered to every run of an engine that has check-ins.
pub fn schemas() -> Vec<Value> {
    Tool::ALL
        .iter()
        .map(|t| tool(t.name(), t.description(), t.properties(), t.required()))
        .collect()
}

/// Runs an allowed call of the check-in tool `name` (its input as `prepare` left it) for the
/// run `run_id`. A broken rule or a call that could not be stored is an error result; the run
/// goes on.
pub async fn run(checkins: &CheckIns, run_id: &str, name: &str, input: &Value) -> ToolOutput {
    let Some(check_in) = Tool::from_name(name) else {
        return ToolOutput::err(format!("unknown tool '{name}'"));
    };
    match checkins.call(run_id, check_in, input).await {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
pub mod context;
mod fs;
```

**with:**

```rust
pub mod checkins;
pub mod context;
mod fs;
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// offers it only to a run that has skills. `context` reads the run's context collections in
/// the same way (`context::run`).
pub const TOOLS: &[(&str, &str)] = &[
```

**with:**

```rust
/// offers it only to a run that has skills. `context` reads the run's context collections in
/// the same way (`context::run`). The check-in tools tell and ask the person and schedule a
/// follow-up (`checkins::run`), offered to every run of an engine that has check-ins.
pub const TOOLS: &[(&str, &str)] = &[
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    ("skill", "Skill"),
    ("context", "Context"),
];
```

**with:**

```rust
    ("skill", "Skill"),
    ("context", "Context"),
    ("notify", "Notify"),
    ("ask", "Ask"),
    ("follow_up", "FollowUp"),
];
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    Count,
    Bool,
}

use Arg::{Bool, Count, PathStr, Str};
```

**with:**

```rust
    Count,
    Bool,
    /// An array; the tool checks its items.
    List,
}

use Arg::{Bool, Count, List, PathStr, Str};
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
            ("offset", Count, false),
        ],
    ),
];
```

**with:**

```rust
            ("offset", Count, false),
        ],
    ),
    ("notify", &[("text", Str, true)]),
    ("ask", &[("question", Str, true), ("choices", List, false)]),
    (
        "follow_up",
        &[("note", Str, true), ("at", Str, false), ("in", Str, false)],
    ),
];
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
        let ok = match kind {
            Str | PathStr => value.is_string(),
            Count => value.is_u64(),
            Bool => value.is_boolean(),
        };
        if !ok {
            let expected = match kind {
                Str | PathStr => "a string",
                Count => "a non-negative integer",
                Bool => "a boolean",
            };
```

**with:**

```rust
        let ok = match kind {
            Str | PathStr => value.is_string(),
            Count => value.is_u64(),
            Bool => value.is_boolean(),
            List => value.is_array(),
        };
        if !ok {
            let expected = match kind {
                Str | PathStr => "a string",
                Count => "a non-negative integer",
                Bool => "a boolean",
                List => "an array",
            };
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// OpenAI `tools` array for every built-in tool but `skill` (`skill::schema`) and `context`
/// (`context::schema`).
```

**with:**

```rust
/// OpenAI `tools` array for every built-in tool but `skill` (`skill::schema`), `context`
/// (`context::schema`) and the check-in tools (`checkins::schemas`).
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
use crate::engine::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
```

**with:**

```rust
use crate::checkins::{CheckIns, Tool as CheckInTool};
use crate::engine::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
pub struct LocalEngine {
    store: Store,
    gate: Arc<dyn PermissionGate>,
    client: ChatClient,
}
```

**with:**

```rust
pub struct LocalEngine {
    store: Store,
    gate: Arc<dyn PermissionGate>,
    client: ChatClient,
    /// The check-in tools, offered to every run when set (`with_checkins`).
    checkins: Option<CheckIns>,
}
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
        Self {
            store,
            gate,
            client: ChatClient::new(retry_delays),
        }
    }
}
```

**with:**

```rust
        Self {
            store,
            gate,
            client: ChatClient::new(retry_delays),
            checkins: None,
        }
    }

    /// Offers the check-in tools (`notify`, `ask`, `follow_up`) to every run (check-ins spec
    /// section 2).
    pub fn with_checkins(mut self, checkins: CheckIns) -> Self {
        self.checkins = Some(checkins);
        self
    }
}
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
            messages,
            mcp_names: ToolNames::default(),
        };
```

**with:**

```rust
            messages,
            mcp_names: ToolNames::default(),
            checkins: self.checkins.clone(),
        };
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
    messages: Vec<Value>,
    mcp_names: ToolNames,
}
```

**with:**

```rust
    messages: Vec<Value>,
    mcp_names: ToolNames,
    checkins: Option<CheckIns>,
}
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
        if call.name == "context" {
            return tools::context::run(self.ctx.tools.contexts(), &call.input).await;
        }
```

**with:**

```rust
        if call.name == "context" {
            return tools::context::run(self.ctx.tools.contexts(), &call.input).await;
        }
        if CheckInTool::from_name(call.name).is_some() {
            return match &self.checkins {
                Some(checkins) => {
                    tools::checkins::run(checkins, &self.ctx.run_id, call.name, &call.input).await
                }
                None => ToolOutput::err(format!("{} is not available in this run", call.name)),
            };
        }
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust
        if !self.ctx.tools.contexts().is_empty() {
            out.push(tools::context::schema());
        }
```

**with:**

```rust
        if !self.ctx.tools.contexts().is_empty() {
            out.push(tools::context::schema());
        }
        if self.checkins.is_some() {
            out.extend(tools::checkins::schemas());
        }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            Arc::new(LocalEngine::with_retry_delays(
                store.clone(),
                hub.clone(),
                cfg.local_retry_delays.clone(),
            )),
```

**with:**

```rust
            Arc::new(
                LocalEngine::with_retry_delays(
                    store.clone(),
                    hub.clone(),
                    cfg.local_retry_delays.clone(),
                )
                .with_checkins(checkins.clone()),
            ),
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test local_checkins_test --test local_context_test --test local_tools_test --test local_engine_test --test runtime_local_test`
Expected: PASS (`local_checkins_test`: 4 tests). The other local tests build their engines without check-ins and are unchanged.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/engine/local/tools/checkins.rs crates/dots-core/src/engine/local/tools/mod.rs crates/dots-core/src/engine/local/mod.rs crates/dots-core/src/engine/local/mcp_tools.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/local_checkins_test.rs
git commit -m "feat(core): give the local engine the check-in tools"
```

### Task 9: Commands, events, the notification setting and the contract

The app's side of section 10: six commands over `Runtime::reply`, `checkins()` and `follow_ups()`; a notification on `MessagePosted` (a question always, "<dot> asks: <question>"; a note while `notify_on_notes` is on); the setting `notify_on_notes` (default `true`); samples of the new types and events in the contract. The UI's side: the schemas, the six command wrappers, the query keys, the cache updates on the three events, and test defaults.

**Files:**

- Modify: `crates/dots-core/src/settings.rs` (`Settings` and its `Default`)
- Modify: `crates/dots-core/tests/settings_test.rs` (two passages)
- Modify: `apps/dots-app/src/commands.rs` (imports; six commands appended)
- Modify: `apps/dots-app/src/lib.rs` (`command_list!`)
- Modify: `apps/dots-app/src/events.rs` (imports; `Notice`; `decide`; `notice_text`; `notify`; one test)
- Modify: `apps/dots-app/src/contract.rs` (samples)
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Modify: `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/api/commands.test.ts`, `ui/src/api/contract.test.ts`
- Modify: `ui/src/lib/query.ts` (`keys`), `ui/src/lib/runtime-events.ts` (`applyRuntimeEvent`), `ui/src/test/fixtures.ts`
- Test: `ui/src/lib/checkins-events.test.ts` (new)

**Interfaces:**

- Consumes: `Runtime::{reply, checkins, follow_ups}`, `CheckIns::{list_messages, dismiss, mark_read}`, `FollowUps::{list, cancel}`, `RuntimeEvent::{MessagePosted, MessageUpdated, FollowUpsUpdated}`, `Message`, `MessageKind`, `FollowUp`, `FollowUpStatus`.
- Produces:
  - `Settings.notify_on_notes: bool` (default `true`).
  - Tauri commands `messages_list(dot_id: Option<String>) -> Vec<Message>`, `message_reply(id, text) -> Run`, `message_dismiss(id) -> Message`, `message_mark_read(id) -> Message`, `follow_ups_list(dot_id: Option<String>) -> Vec<FollowUp>`, `follow_up_cancel(id) -> FollowUp`.
  - App: `Notice::MessagePosted { message: Box<Message> }`, `pub fn message_notifies(message: &Message, notify_on_notes: bool) -> bool`.
  - UI: `T.MessageKind`, `T.FollowUpStatus`, `T.Message`, `T.FollowUp`, the three `RuntimeEvent` variants, `Settings.notify_on_notes`; `commands.messagesList(dotId: string | null)`, `messageReply(id, text)`, `messageDismiss(id)`, `messageMarkRead(id)`, `followUpsList(dotId: string | null)`, `followUpCancel(id)`; `keys.messages(dotId: string | null)` (`["messages", dotId]`), `keys.followUps(dotId: string | null)` (`["follow-ups", dotId]`); `sample.messages` (an open question with the choices yes and no, an answered one, a dismissed one, a note of Dots' own) and `sample.followUps`; test defaults `messages_list`, `follow_ups_list` answer `[]` and `message_mark_read` answers the message read.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/settings_test.rs`, replace:**

```rust
    assert!(!s.schedules_paused);
    assert_eq!(s.global_instructions, "");
}
```

**with:**

```rust
    assert!(!s.schedules_paused);
    assert_eq!(s.global_instructions, "");
    assert!(s.notify_on_notes);
}
```

**In `crates/dots-core/tests/settings_test.rs`, replace:**

```rust
        global_instructions: "Use British spelling.\nSign off as the team.".into(),
    };
```

**with:**

```rust
        global_instructions: "Use British spelling.\nSign off as the team.".into(),
        notify_on_notes: false,
    };
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
            [false, false, true]
        );
    }
}
```

**with:**

```rust
            [false, false, true]
        );
    }

    fn posted(kind: MessageKind) -> Message {
        Message {
            id: "m1".into(),
            dot_id: "d".into(),
            dot_name: "nightly".into(),
            run_id: Some("r".into()),
            kind,
            text: "\n  Ship the release?\nmore".into(),
            choices: None,
            created_at: "t".into(),
            read_at: None,
            answered_at: None,
            answer_run_id: None,
            dismissed_at: None,
        }
    }

    #[test]
    fn a_question_always_notifies_and_a_note_only_when_switched_on() {
        let mut n = Notifier::default();
        let question = posted(MessageKind::Question);
        let event = RuntimeEvent::MessagePosted {
            message: question.clone(),
        };
        assert_eq!(
            n.decide(&event),
            Some(Notice::MessagePosted {
                message: Box::new(question.clone())
            })
        );
        assert!(message_notifies(&question, false));
        let note = posted(MessageKind::Note);
        assert!(message_notifies(&note, true));
        assert!(!message_notifies(&note, false));
        // An answer or a read changes the Inbox and notifies nothing.
        let updated = RuntimeEvent::MessageUpdated {
            message: note.clone(),
        };
        assert!(n.decide(&updated).is_none());
        assert!(affects_inbox(&updated));
        assert_eq!(
            notice_text(
                &Notice::MessagePosted {
                    message: Box::new(question)
                },
                "nightly"
            ),
            (
                "nightly asks: Ship the release?".into(),
                "Answer it in the Inbox.".into()
            )
        );
        assert_eq!(
            notice_text(
                &Notice::MessagePosted {
                    message: Box::new(note)
                },
                "nightly"
            ),
            ("Note from nightly".into(), "Ship the release?".into())
        );
    }
}
```

**Create `ui/src/lib/checkins-events.test.ts`:**

```ts
import { describe, expect, it } from "vitest";

import { createQueryClient, keys } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { sample } from "@/test/fixtures";

type Client = ReturnType<typeof createQueryClient>;
const stale = (qc: Client, key: readonly unknown[]) => qc.getQueryState(key)?.isInvalidated;

describe("check-in events and the query cache", () => {
  it("marks every list of messages stale when a message is posted or changes", () => {
    for (const type of ["message_posted", "message_updated"] as const) {
      const qc = createQueryClient();
      qc.setQueryData(keys.messages(null), []);
      qc.setQueryData(keys.messages(sample.dot.id), []);
      qc.setQueryData(keys.followUps(null), []);
      applyRuntimeEvent(qc, { type, message: sample.messages[0] });
      expect(stale(qc, keys.messages(null))).toBe(true);
      expect(stale(qc, keys.messages(sample.dot.id))).toBe(true);
      expect(stale(qc, keys.followUps(null))).toBe(false);
    }
  });

  it("marks every list of follow-ups stale when a dot's follow-ups change", () => {
    const qc = createQueryClient();
    qc.setQueryData(keys.followUps(null), []);
    qc.setQueryData(keys.followUps(sample.dot.id), []);
    qc.setQueryData(keys.messages(null), []);
    applyRuntimeEvent(qc, { type: "follow_ups_updated", dot_id: sample.dot.id });
    expect(stale(qc, keys.followUps(null))).toBe(true);
    expect(stale(qc, keys.followUps(sample.dot.id))).toBe(true);
    expect(stale(qc, keys.messages(null))).toBe(false);
  });
});
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
  it("dismisses a schedule's attention by its dot", async () => {
```

**with:**

```ts
  it("sends the check-in commands as the core names them", async () => {
    reply = async () => fixture.Message;
    expect(await commands.messagesList(null)).toEqual(fixture.Message);
    await commands.messagesList("d1");
    reply = async () => fixture.Run;
    await commands.messageReply("m1", "yes");
    reply = async () => fixture.Message[0];
    await commands.messageDismiss("m1");
    await commands.messageMarkRead("m1");
    reply = async () => fixture.FollowUp;
    expect(await commands.followUpsList(null)).toEqual(fixture.FollowUp);
    reply = async () => fixture.FollowUp[0];
    await commands.followUpCancel("f1");
    expect(calls).toEqual([
      ["messages_list", { dotId: null }],
      ["messages_list", { dotId: "d1" }],
      ["message_reply", { id: "m1", text: "yes" }],
      ["message_dismiss", { id: "m1" }],
      ["message_mark_read", { id: "m1" }],
      ["follow_ups_list", { dotId: null }],
      ["follow_up_cancel", { id: "f1" }],
    ]);
  });

  it("dismisses a schedule's attention by its dot", async () => {
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
  StartupError: T.StartupError,
};
```

**with:**

```ts
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
  StartupError: T.StartupError,
  Message: T.Message.array(),
  FollowUp: T.FollowUp.array(),
};
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  LibraryKind: T.LibraryKind,
  CatalogStatus: T.CatalogStatus,
};
```

**with:**

```ts
  LibraryKind: T.LibraryKind,
  CatalogStatus: T.CatalogStatus,
  MessageKind: T.MessageKind,
  FollowUpStatus: T.FollowUpStatus,
};
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test settings_test`, `cargo test -p dots-app events`
Expected: FAIL to compile: `notify_on_notes`, `Notice::MessagePosted`, `message_notifies` and `MessageKind` (in the app) do not exist.

- [ ] **Step 3: Add the setting, the commands, the notification and the samples**

**In `crates/dots-core/src/settings.rs`, replace:**

```rust
    /// The text every dot's first prompt starts with, unless the dot switches it off.
    pub global_instructions: String,
}
```

**with:**

```rust
    /// The text every dot's first prompt starts with, unless the dot switches it off.
    pub global_instructions: String,
    /// A dot's note shows a notification (check-ins spec section 10). A question always does.
    pub notify_on_notes: bool,
}
```

**In `crates/dots-core/src/settings.rs`, replace:**

```rust
            global_instructions: String::new(),
        }
```

**with:**

```rust
            global_instructions: String::new(),
            notify_on_notes: true,
        }
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, Inbox, Run, RunEventRecord, SettingsSaved, Status,
    WorktreeDiff,
};
```

**with:**

```rust
use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, FollowUp, Inbox, Message, Run, RunEventRecord,
    SettingsSaved, Status, WorktreeDiff,
};
```

**Append to `apps/dots-app/src/commands.rs`:**

```rust

/// Every message that waits for the person and the newest others, newest first; of one dot
/// with `dot_id` (check-ins spec section 10).
#[tauri::command]
pub async fn messages_list(
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<Vec<Message>> {
    Ok(state
        .rt()?
        .checkins()
        .list_messages(dot_id.as_deref())
        .await?)
}

/// Answers a dot's question: a run continues the question's thread.
#[tauri::command]
pub async fn message_reply(state: State<'_, AppState>, id: String, text: String) -> Res<Run> {
    Ok(state.rt()?.reply(&id, &text).await?)
}

#[tauri::command]
pub async fn message_dismiss(state: State<'_, AppState>, id: String) -> Res<Message> {
    Ok(state.rt()?.checkins().dismiss(&id).await?)
}

#[tauri::command]
pub async fn message_mark_read(state: State<'_, AppState>, id: String) -> Res<Message> {
    Ok(state.rt()?.checkins().mark_read(&id).await?)
}

/// The pending follow-ups, soonest first; of one dot with `dot_id`.
#[tauri::command]
pub async fn follow_ups_list(
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<Vec<FollowUp>> {
    Ok(state.rt()?.follow_ups().list(dot_id.as_deref()).await?)
}

#[tauri::command]
pub async fn follow_up_cancel(state: State<'_, AppState>, id: String) -> Res<FollowUp> {
    Ok(state.rt()?.follow_ups().cancel(&id).await?)
}
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    context_documents,
    installed_claude_plugins,
    environment_variables,
];
```

**with:**

```rust
    context_documents,
    installed_claude_plugins,
    environment_variables,
    messages_list,
    message_reply,
    message_dismiss,
    message_mark_read,
    follow_ups_list,
    follow_up_cancel,
];
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
use dots_core::model::{Run, RunStatus, TriggerKind};
```

**with:**

```rust
use dots_core::model::{Message, MessageKind, Run, RunStatus, TriggerKind};
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
    /// A schedule needs attention: its occurrence failed in the end (Scheduler v2 spec §7).
    ScheduleFailed {
        dot_id: String,
        error: Option<String>,
    },
}
```

**with:**

```rust
    /// A schedule needs attention: its occurrence failed in the end (Scheduler v2 spec §7).
    ScheduleFailed {
        dot_id: String,
        error: Option<String>,
    },
    /// A dot posted a note or asked a question (check-ins spec section 10). A question always
    /// notifies; a note only while Settings say so (`message_notifies`).
    MessagePosted {
        message: Box<Message>,
    },
}
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
            RuntimeEvent::ScheduleAttention { dot_id, error, .. } => Some(Notice::ScheduleFailed {
                dot_id: dot_id.clone(),
                error: error.clone(),
            }),
            _ => None,
```

**with:**

```rust
            RuntimeEvent::ScheduleAttention { dot_id, error, .. } => Some(Notice::ScheduleFailed {
                dot_id: dot_id.clone(),
                error: error.clone(),
            }),
            RuntimeEvent::MessagePosted { message } => Some(Notice::MessagePosted {
                message: Box::new(message.clone()),
            }),
            _ => None,
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
        Notice::ScheduleFailed { error, .. } => (
            format!("{dot_name}: scheduled run failed"),
            first_line(error.as_deref(), "The run failed."),
        ),
    }
}
```

**with:**

```rust
        Notice::ScheduleFailed { error, .. } => (
            format!("{dot_name}: scheduled run failed"),
            first_line(error.as_deref(), "The run failed."),
        ),
        Notice::MessagePosted { message } => match message.kind {
            MessageKind::Question => (
                format!(
                    "{dot_name} asks: {}",
                    first_line(Some(message.text.as_str()), "a question")
                ),
                "Answer it in the Inbox.".to_string(),
            ),
            MessageKind::Note => (
                format!("Note from {dot_name}"),
                first_line(Some(message.text.as_str()), "A note."),
            ),
        },
    }
}

/// Whether a posted message shows a notification: a question always, a note while the setting
/// `notify_on_notes` is on.
pub fn message_notifies(message: &Message, notify_on_notes: bool) -> bool {
    message.kind == MessageKind::Question || notify_on_notes
}
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
async fn notify(app: &AppHandle, rt: &Runtime, notice: &Notice) {
    let dot_id = match notice {
        Notice::ApprovalNeeded { run_id, .. } => rt.get_run(run_id).await.map(|r| r.dot_id).ok(),
        Notice::RunEnded { run } => Some(run.dot_id.clone()),
        Notice::ScheduleFailed { dot_id, .. } => Some(dot_id.clone()),
    };
```

**with:**

```rust
async fn notify(app: &AppHandle, rt: &Runtime, notice: &Notice) {
    if let Notice::MessagePosted { message } = notice {
        if !message_notifies(message, rt.settings().await.notify_on_notes) {
            return;
        }
    }
    let dot_id = match notice {
        Notice::ApprovalNeeded { run_id, .. } => rt.get_run(run_id).await.map(|r| r.dot_id).ok(),
        Notice::RunEnded { run } => Some(run.dot_id.clone()),
        Notice::ScheduleFailed { dot_id, .. } => Some(dot_id.clone()),
        Notice::MessagePosted { message } => Some(message.dot_id.clone()),
    };
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
fn run() -> Run {
```

**with:**

```rust
/// A question that waits, an answered one, a dismissed one, and a note of Dots' own that was
/// read: every optional field of a message is set in one of them.
fn messages() -> Vec<Message> {
    let open = Message {
        id: "01J0MSG1".into(),
        dot_id: "01J0DOT".into(),
        dot_name: "nightly-tests".into(),
        run_id: Some("01J0RUN".into()),
        kind: MessageKind::Question,
        text: "Ship the release?".into(),
        choices: Some(vec!["yes".into(), "no".into()]),
        created_at: TS.into(),
        read_at: None,
        answered_at: None,
        answer_run_id: None,
        dismissed_at: None,
    };
    vec![
        open.clone(),
        Message {
            id: "01J0MSG2".into(),
            read_at: Some(TS.into()),
            answered_at: Some(TS.into()),
            answer_run_id: Some("01J0RUN3".into()),
            ..open.clone()
        },
        Message {
            id: "01J0MSG3".into(),
            choices: None,
            dismissed_at: Some(TS.into()),
            ..open.clone()
        },
        Message {
            id: "01J0MSG4".into(),
            run_id: None,
            kind: MessageKind::Note,
            text: "A follow-up was skipped because the dot is off. It was due Tue 21 Oct 09:00: \
                   check CI"
                .into(),
            choices: None,
            read_at: Some(TS.into()),
            ..open
        },
    ]
}

/// A pending follow-up that failed to start once, and one that fired.
fn follow_ups() -> Vec<FollowUp> {
    let pending = FollowUp {
        id: "01J0FUP1".into(),
        dot_id: "01J0DOT".into(),
        from_run_id: "01J0RUN".into(),
        due_at: "2026-10-21T08:00:00.000Z".into(),
        note: "Check the build.".into(),
        status: FollowUpStatus::Pending,
        fired_run_id: None,
        attempts: 1,
        retry_at: Some("2026-10-21T08:01:00.000Z".into()),
        created_at: TS.into(),
    };
    vec![
        pending.clone(),
        FollowUp {
            id: "01J0FUP2".into(),
            status: FollowUpStatus::Fired,
            fired_run_id: Some("01J0RUN4".into()),
            attempts: 0,
            retry_at: None,
            ..pending
        },
    ]
}

fn run() -> Run {
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    // Past the `json!` macro's recursion limit, so added here.
    samples["ContextDocuments"] = to(context_documents());
    samples
```

**with:**

```rust
    // Past the `json!` macro's recursion limit, so added here.
    samples["ContextDocuments"] = to(context_documents());
    samples["Message"] = to(messages());
    samples["FollowUp"] = to(follow_ups());
    samples["Enums"]["MessageKind"] = to(MessageKind::ALL);
    samples["Enums"]["FollowUpStatus"] = to(FollowUpStatus::ALL);
    samples["RuntimeEvent"]
        .as_array_mut()
        .expect("the RuntimeEvent samples")
        .extend([
            to(RuntimeEvent::MessagePosted {
                message: messages()[0].clone(),
            }),
            to(RuntimeEvent::MessageUpdated {
                message: messages()[1].clone(),
            }),
            to(RuntimeEvent::FollowUpsUpdated {
                dot_id: "01J0DOT".into(),
            }),
        ]);
    samples
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const RuntimeEvent = z.discriminatedUnion("type", [
```

**with:**

```ts
export const MessageKind = z.enum(["note", "question"]);
export const FollowUpStatus = z.enum(["pending", "fired", "cancelled", "skipped"]);
// A dot's note or question (check-ins spec section 7). Times are RFC 3339 UTC.
export const Message = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  dot_name: z.string(),
  // Null for a note of Dots' own (a skipped follow-up).
  run_id: nullable(z.string()),
  kind: MessageKind,
  text: z.string(),
  choices: nullable(z.array(z.string())),
  created_at: z.string(),
  read_at: nullable(z.string()),
  answered_at: nullable(z.string()),
  // The run the answer started.
  answer_run_id: nullable(z.string()),
  dismissed_at: nullable(z.string()),
});
// A run a dot scheduled for itself (check-ins spec section 6). Times are RFC 3339 UTC.
export const FollowUp = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  from_run_id: z.string(),
  due_at: z.string(),
  note: z.string(),
  status: FollowUpStatus,
  fired_run_id: nullable(z.string()),
  attempts: z.number().int(),
  retry_at: nullable(z.string()),
  created_at: z.string(),
});

export const RuntimeEvent = z.discriminatedUnion("type", [
```

**In `ui/src/api/types.ts`, replace:**

```ts
  z.strictObject({
    type: z.literal("schedule_attention"),
    dot_id: z.string(),
    run_id: z.string(),
    error: nullable(z.string()),
  }),
]);
```

**with:**

```ts
  z.strictObject({
    type: z.literal("schedule_attention"),
    dot_id: z.string(),
    run_id: z.string(),
    error: nullable(z.string()),
  }),
  // A dot posted a note or asked a question.
  z.strictObject({ type: z.literal("message_posted"), message: Message }),
  // A message was read, answered or dismissed.
  z.strictObject({ type: z.literal("message_updated"), message: Message }),
  // A follow-up of the dot was scheduled, fired, cancelled or skipped.
  z.strictObject({ type: z.literal("follow_ups_updated"), dot_id: z.string() }),
]);
```

**In `ui/src/api/types.ts`, replace:**

```ts
  schedules_paused: z.boolean(),
  global_instructions: z.string(),
});
```

**with:**

```ts
  schedules_paused: z.boolean(),
  global_instructions: z.string(),
  // A dot's note notifies; a question always does.
  notify_on_notes: z.boolean(),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type StartupError = z.infer<typeof StartupError>;
```

**with:**

```ts
export type StartupError = z.infer<typeof StartupError>;
export type MessageKind = z.infer<typeof MessageKind>;
export type FollowUpStatus = z.infer<typeof FollowUpStatus>;
export type Message = z.infer<typeof Message>;
export type FollowUp = z.infer<typeof FollowUp>;
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  environmentVariables: (names: string[]) =>
    call(T.EnvVariable.array(), "environment_variables", { names }),
};
```

**with:**

```ts
  environmentVariables: (names: string[]) =>
    call(T.EnvVariable.array(), "environment_variables", { names }),

  /// Every message that waits (open questions, unread notes) and the newest others, newest
  /// first; of one dot with `dotId`.
  messagesList: (dotId: string | null) => call(T.Message.array(), "messages_list", { dotId }),
  /// Answers a dot's question; the run that continues its thread. Rejects with kind `conflict`
  /// when the question is closed or its thread is still busy, in words to show as they are.
  messageReply: (id: string, text: string) => call(T.Run, "message_reply", { id, text }),
  messageDismiss: (id: string) => call(T.Message, "message_dismiss", { id }),
  messageMarkRead: (id: string) => call(T.Message, "message_mark_read", { id }),
  /// The pending follow-ups, soonest first; of one dot with `dotId`.
  followUpsList: (dotId: string | null) =>
    call(T.FollowUp.array(), "follow_ups_list", { dotId }),
  /// Rejects with kind `conflict` once the follow-up fired, was skipped or was cancelled.
  followUpCancel: (id: string) => call(T.FollowUp, "follow_up_cancel", { id }),
};
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  diff: (runId: string) => ["diff", runId] as const,
};
```

**with:**

```ts
  diff: (runId: string) => ["diff", runId] as const,
  messages: (dotId: string | null) => ["messages", dotId] as const,
  followUps: (dotId: string | null) => ["follow-ups", dotId] as const,
};
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
      break;
  }
```

**with:**

```ts
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
      break;
    case "message_posted":
    case "message_updated":
      // The Inbox's Messages group and its count, and the chat the message belongs to. An
      // answer's run comes with its own `run_updated`.
      invalidate(qc, ["messages"]);
      break;
    case "follow_ups_updated":
      // The pending follow-ups in the Dots list and the dot's editor.
      invalidate(qc, ["follow-ups"]);
      break;
  }
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  schedules: T.Schedule.array().parse(contract.Schedule),
};
```

**with:**

```ts
  schedules: T.Schedule.array().parse(contract.Schedule),
  /// An open question with the choices "yes" and "no", an answered one, a dismissed one, and a
  /// read note of Dots' own.
  messages: T.Message.array().parse(contract.Message),
  /// A pending follow-up and a fired one.
  followUps: T.FollowUp.array().parse(contract.FollowUp),
};
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  backend.on("list_runs", () => []);
```

**with:**

```ts
  backend.on("list_runs", () => []);
  // No dot has checked in or scheduled anything. A question on screen is marked read.
  backend.on("messages_list", () => []);
  backend.on("follow_ups_list", () => []);
  backend.on("message_mark_read", (args) => ({
    ...sample.messages[0],
    id: String(args.id),
    read_at: sample.run.queued_at,
  }));
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the fixture gains `Message`, `FollowUp`, `Enums.MessageKind`, `Enums.FollowUpStatus`, three `RuntimeEvent` samples, `Settings.notify_on_notes` and the six names in `Commands`.

**Generated:** `ui/src/api/__fixtures__/contract.json`

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test settings_test`, `cargo test -p dots-app`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/api src/lib/checkins-events.test.ts`
Expected: PASS; `contract.test.ts` parses every new sample and knows every new enum and event type; `commands.test.ts` "invokes exactly the commands the app registers" sees the six new names.

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: all pass; no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/src/settings.rs crates/dots-core/tests/settings_test.rs apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs apps/dots-app/src/events.rs apps/dots-app/src/contract.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/api/commands.ts ui/src/api/commands.test.ts ui/src/api/contract.test.ts ui/src/lib/query.ts ui/src/lib/runtime-events.ts ui/src/test/fixtures.ts ui/src/lib/checkins-events.test.ts
git commit -m "feat(app): commands, events, notifications and contract for check-ins"
```

### Task 10: The Inbox's Messages group and its count

The Inbox gains a "Messages" group, first: a question shows the dot, its text, its choices as buttons, a reply box, "Reply" and "Dismiss"; a note shows its text and "Mark read". A question shown there counts as read (it stays open). The sidebar's Inbox count, like the tray's `inbox_count`, adds the open questions and unread notes.

**Files:**

- Create: `ui/src/lib/messages.ts`
- Create: `ui/src/components/MessageCard.tsx`
- Modify: `ui/src/screens/InboxScreen.tsx` (imports; `InboxScreen` near line 218)
- Modify: `ui/src/app/Layout.tsx` (imports; the count near line 24)
- Test: `ui/src/screens/InboxMessages.test.tsx` (new)

**Interfaces:**

- Consumes: `commands.{messagesList, messageReply, messageDismiss, messageMarkRead}`, `keys.messages`, `invalidate`, `onSingleClick`, `relativeTime`, `notifyError` (Task 9 and existing).
- Produces: `isOpenQuestion(message: Message): boolean` and `waits(message: Message): boolean` in `@/lib/messages`; `<MessageCard message={Message} />` (an `li` named "Question from <dot>" or "Note from <dot>"; the reply box labelled "Answer to <dot>").

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/screens/InboxMessages.test.tsx`:**

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [open, answered, dismissed, dotsNote] = sample.messages;
const note: Message = {
  ...dotsNote,
  id: "n1",
  run_id: "r1",
  read_at: null,
  text: "Half the tests pass.",
};
const asking = `Question from ${open.dot_name}`;

/// The messages the core holds; an answer, a dismiss or a read changes one, as in the core.
function holding(initial: Message[]) {
  let current = initial;
  const change = (id: unknown, fields: Partial<Message>) => {
    current = current.map((m) => (m.id === id ? { ...m, ...fields } : m));
    return current.find((m) => m.id === id);
  };
  backend.on("messages_list", () => current);
  backend.on("message_reply", (args) => {
    change(args.id, { answered_at: sample.run.queued_at, answer_run_id: "run-answer" });
    return { ...sample.run, id: "run-answer", trigger: "chat", status: "queued" };
  });
  backend.on("message_dismiss", (args) => change(args.id, { dismissed_at: sample.run.queued_at }));
  backend.on("message_mark_read", (args) => change(args.id, { read_at: sample.run.queued_at }));
}

describe("the Inbox's messages", () => {
  it("lists the open questions and unread notes first and counts them", async () => {
    holding([open, answered, dismissed, note]);
    backend.on("inbox", () => sample.inbox);
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "Messages" });
    const approvals = screen.getByRole("region", { name: "Approvals" });
    expect(group.compareDocumentPosition(approvals) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(group).getAllByRole("listitem")).toHaveLength(2);
    const question = within(group).getByRole("listitem", { name: asking });
    expect(question).toHaveTextContent(open.text);
    expect(within(question).getByRole("button", { name: "yes" })).toBeVisible();
    expect(within(question).getByRole("button", { name: "no" })).toBeVisible();
    expect(
      within(group).getByRole("listitem", { name: `Note from ${note.dot_name}` }),
    ).toHaveTextContent("Half the tests pass.");
    // The four items of the sample Inbox and the two messages.
    const nav = screen.getByRole("navigation", { name: "Main" });
    await waitFor(() =>
      expect(within(nav).getByRole("link", { name: /Inbox/ })).toHaveTextContent("6"),
    );
    // A question shown here counts as read; it stays open.
    await waitFor(() => expect(backend.callsTo("message_mark_read")).toEqual([{ id: open.id }]));
    expect(within(group).getByRole("listitem", { name: asking })).toBeVisible();
  });

  it("answers a question with one of its choices", async () => {
    holding([open]);
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.click(within(question).getByRole("button", { name: "yes" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([{ id: open.id, text: "yes" }]),
    );
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  it("answers a question with typed text", async () => {
    holding([{ ...open, choices: null }]);
    const { user } = renderApp("/inbox");
    const box = await screen.findByLabelText(`Answer to ${open.dot_name}`);
    expect(screen.getByRole("button", { name: "Reply" })).toBeDisabled();
    await user.type(box, "  Ship it on Friday.  ");
    await user.click(screen.getByRole("button", { name: "Reply" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([
        { id: open.id, text: "Ship it on Friday." },
      ]),
    );
  });

  // Review Focus 3.
  it("answers once on a double click", async () => {
    holding([open]);
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.dblClick(within(question).getByRole("button", { name: "no" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("message_reply")).toEqual([{ id: open.id, text: "no" }]);
  });

  it("dismisses a question and marks a note read", async () => {
    holding([open, note]);
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.click(within(question).getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(backend.callsTo("message_dismiss")).toEqual([{ id: open.id }]));
    const shown = await screen.findByRole("listitem", { name: `Note from ${note.dot_name}` });
    await user.click(within(shown).getByRole("button", { name: "Mark read" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("message_mark_read")).toContainEqual({ id: note.id });
  });

  it("shows a refused answer as the core words it", async () => {
    backend.on("messages_list", () => [open]);
    backend.on("message_reply", () => {
      throw { kind: "conflict", message: "this question has already been answered" };
    });
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.click(within(question).getByRole("button", { name: "yes" }));
    expect(await screen.findByText("this question has already been answered")).toBeVisible();
    expect(within(question).getByRole("button", { name: "yes" })).toBeEnabled();
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/InboxMessages.test.tsx`
Expected: FAIL: the region "Messages" is not found.

- [ ] **Step 3: Add the group and the count**

**Create `ui/src/lib/messages.ts`:**

```ts
import type { Message } from "@/api/types";

/// A question neither answered nor dismissed.
export function isOpenQuestion(message: Message): boolean {
  return (
    message.kind === "question" && message.answered_at === null && message.dismissed_at === null
  );
}

/// A message that waits for the person: an open question, or a note not read yet. The Inbox
/// shows these and counts them, as the core's `inbox_count` does.
export function waits(message: Message): boolean {
  return message.kind === "question" ? isOpenQuestion(message) : message.read_at === null;
}
```

**Create `ui/src/components/MessageCard.tsx`:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Message } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// A dot's question or note in the Inbox (check-ins spec section 9). A question offers its
/// choices, a reply box and Dismiss; a note offers Mark read. A question shown here counts as
/// read, and stays open until it is answered or dismissed.
export function MessageCard({ message }: { message: Message }) {
  const qc = useQueryClient();
  const [text, setText] = useState("");
  // What a change shows up in: the messages, and the thread and runs an answer adds a run to.
  const reload = () => {
    for (const queryKey of [["messages"], ["runs"], ["thread"]]) invalidate(qc, queryKey);
  };
  const reply = useMutation({
    mutationFn: (answer: string) => commands.messageReply(message.id, answer),
    onSuccess: () => {
      setText("");
      toast.success(`Answer sent to ${message.dot_name}`);
    },
    onError: notifyError,
    onSettled: reload,
  });
  const dismiss = useMutation({
    mutationFn: () => commands.messageDismiss(message.id),
    onError: notifyError,
    onSettled: reload,
  });
  const markRead = useMutation({
    mutationFn: () => commands.messageMarkRead(message.id),
    onError: notifyError,
    onSettled: reload,
  });
  const question = message.kind === "question";
  const unreadQuestion = question && message.read_at === null;
  const markReadNow = markRead.mutate;
  useEffect(() => {
    if (unreadQuestion) markReadNow();
  }, [unreadQuestion, markReadNow]);
  // One answer per question: after a success the card waits for the list to drop it.
  const answering = reply.isPending || reply.isSuccess;
  return (
    <li
      aria-label={`${question ? "Question" : "Note"} from ${message.dot_name}`}
      className="flex flex-col gap-2 rounded-lg border p-3"
    >
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <Link to={`/dots/${message.dot_id}`} className="font-medium text-foreground hover:underline">
          {message.dot_name}
        </Link>
        <span>
          {question ? "asks" : "says"} · {relativeTime(message.created_at)}
        </span>
        {message.run_id && (
          <Link to={`/runs/${message.run_id}`} className="hover:underline">
            Open the run
          </Link>
        )}
      </p>
      <p className="whitespace-pre-wrap">{message.text}</p>
      {question ? (
        <>
          {message.choices && message.choices.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {message.choices.map((choice) => (
                <Button
                  key={choice}
                  size="sm"
                  variant="outline"
                  disabled={answering}
                  onClick={onSingleClick(() => reply.mutate(choice))}
                >
                  {choice}
                </Button>
              ))}
            </div>
          )}
          <div className="flex items-end gap-2">
            <label htmlFor={`answer-${message.id}`} className="sr-only">
              Answer to {message.dot_name}
            </label>
            <Textarea
              id={`answer-${message.id}`}
              rows={2}
              placeholder="Type an answer"
              value={text}
              disabled={answering}
              onChange={(e) => setText(e.target.value)}
            />
            <Button
              size="sm"
              disabled={answering || text.trim() === ""}
              onClick={onSingleClick(() => reply.mutate(text.trim()))}
            >
              Reply
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={answering || dismiss.isPending}
              onClick={onSingleClick(() => dismiss.mutate())}
            >
              Dismiss
            </Button>
          </div>
        </>
      ) : (
        <div>
          <Button
            size="sm"
            variant="outline"
            disabled={markRead.isPending}
            onClick={onSingleClick(() => markRead.mutate())}
          >
            Mark read
          </Button>
        </div>
      )}
    </li>
  );
}
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
```

**with:**

```tsx
import { ErrorNote } from "@/components/ErrorNote";
import { MessageCard } from "@/components/MessageCard";
import { Page } from "@/components/Page";
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
```

**with:**

```tsx
import { relativeTime } from "@/lib/format";
import { waits } from "@/lib/messages";
import { notifyError } from "@/lib/notify";
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
  if (inbox.isPending) return <Page title="Inbox" />;
  if (inbox.isError) {
    return (
      <Page title="Inbox">
        <ErrorNote error={inbox.error} />
      </Page>
    );
  }
  const { approvals, review_runs: runs, attention, tools_changed: tools } = inbox.data;
  if (
    approvals.length === 0 &&
    runs.length === 0 &&
    attention.length === 0 &&
    tools.length === 0
  ) {
    return (
      <Page title="Inbox">
        <p className="text-muted-foreground">
          Nothing waits for you. Approvals a dot asks for, schedules that need attention and
          finished runs to review show up here.
        </p>
      </Page>
    );
  }
  return (
    <Page title="Inbox">
      {approvals.length > 0 && (
```

**with:**

```tsx
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
  // The dots' open questions and unread notes come first (check-ins spec section 9).
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  });
  if (inbox.isPending || messages.isPending) return <Page title="Inbox" />;
  if (inbox.isError) {
    return (
      <Page title="Inbox">
        <ErrorNote error={inbox.error} />
      </Page>
    );
  }
  const { approvals, review_runs: runs, attention, tools_changed: tools } = inbox.data;
  const waiting = (messages.data ?? []).filter(waits);
  if (
    !messages.isError &&
    waiting.length === 0 &&
    approvals.length === 0 &&
    runs.length === 0 &&
    attention.length === 0 &&
    tools.length === 0
  ) {
    return (
      <Page title="Inbox">
        <p className="text-muted-foreground">
          Nothing waits for you. Questions and notes from dots, approvals a dot asks for,
          schedules that need attention and finished runs to review show up here.
        </p>
      </Page>
    );
  }
  return (
    <Page title="Inbox">
      {messages.isError && <ErrorNote error={messages.error} />}
      {waiting.length > 0 && (
        <section aria-labelledby="inbox-messages" className="flex flex-col gap-2">
          <h2 id="inbox-messages" className="font-semibold">
            Messages
          </h2>
          <ul className="flex flex-col gap-2">
            {waiting.map((message) => (
              <MessageCard key={message.id} message={message} />
            ))}
          </ul>
        </section>
      )}
      {approvals.length > 0 && (
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
import { keys } from "@/lib/query";
```

**with:**

```tsx
import { waits } from "@/lib/messages";
import { keys } from "@/lib/query";
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  // The same count as the tray's (`inbox_count`).
  const waiting = inbox
    ? inbox.approvals.length +
      inbox.review_runs.length +
      inbox.attention.length +
      inbox.tools_changed.length
    : 0;
```

**with:**

```tsx
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data;
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  }).data;
  // The same count as the tray's (`inbox_count`), the open questions and unread notes included.
  const waiting =
    (inbox
      ? inbox.approvals.length +
        inbox.review_runs.length +
        inbox.attention.length +
        inbox.tools_changed.length
      : 0) + (messages ?? []).filter(waits).length;
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/InboxMessages.test.tsx src/screens/InboxScreen.test.tsx src/app/Layout.test.tsx`
Expected: PASS (`InboxMessages.test.tsx`: 6 tests); the existing Inbox and Layout tests are unchanged (the defaults answer no messages).

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/messages.ts ui/src/components/MessageCard.tsx ui/src/screens/InboxScreen.tsx ui/src/app/Layout.tsx ui/src/screens/InboxMessages.test.tsx
git commit -m "feat(ui): the Inbox's Messages group and its count"
```

### Task 11: Check-ins in a chat, and the reply bar

A thread's notes and questions show in its chat as messages from the dot, after the run that posted each. While the thread has an open question, a bar above the composer reads "<dot> asked: <question>" with the question's choices and "Reply to question"; with that pressed, the composer sends its text as the answer (`message_reply`) instead of a chat message. An answer to a thread whose end has no session starts a new chat, which opens. Opening the chat marks the open question read.

**Files:**

- Modify: `ui/src/components/Transcript.tsx` (types import near line 8; `CheckIn` added before `Transcript`; the `messages` prop and its rendering)
- Modify: `ui/src/screens/ChatScreen.tsx` (imports near line 19; `Conversation`)
- Test: `ui/src/screens/ChatCheckIns.test.tsx` (new)

**Interfaces:**

- Consumes: `commands.{messagesList, messageReply, messageMarkRead}`, `keys.messages`, `isOpenQuestion` (Tasks 9, 10).
- Produces: `Transcript` takes an optional `messages?: Message[]`; each shows after its run as a `group` named "Note from the dot" or "Question from the dot"; the chat's bar is a `region` named "Open question" with the button "Reply to question".

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/screens/ChatCheckIns.test.tsx`:**

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message, Run } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot = sample.dot;
/// A chat run of the thread that starts at run "a".
const run = (id: string, at: string, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  root_run_id: "a",
  parent_run_id: null,
  trigger: "chat",
  branch: null,
  payload: { message: `question ${id}` },
  queued_at: `2026-10-09T${at}:00.000Z`,
  ...over,
});
const [open] = sample.messages; // a question with the choices "yes" and "no"
const asked: Message = { ...open, run_id: "a", created_at: "2026-10-09T10:02:00.000Z" };

/// A dot with one chat thread of `runs`, and its `messages`.
function thread(runs: Run[], messages: Message[]) {
  backend.on("list_dots", () => [dot]);
  backend.on("list_runs", () => runs);
  backend.on("run_thread", () => runs);
  backend.on("list_run_events", () => []);
  backend.on("approvals_for_run", () => []);
  backend.on("messages_list", () => messages);
}

describe("check-ins in a chat", () => {
  it("shows the dot's notes and questions after the run that posted them", async () => {
    const note: Message = {
      ...sample.messages[3],
      id: "n1",
      run_id: "a",
      text: "Half the tests pass.",
      created_at: "2026-10-09T10:01:00.000Z",
    };
    thread([run("a", "10:00"), run("b", "11:00", { parent_run_id: "a" })], [asked, note]);
    renderApp(`/chat/${dot.id}/a`);
    const conversation = await screen.findByRole("region", { name: "Conversation" });
    await within(conversation).findByRole("group", { name: "Note from the dot" });
    const [first, second] = within(conversation).getAllByRole("region", {
      name: /^Chat message,/,
    });
    expect(within(first).getByRole("group", { name: "Note from the dot" })).toHaveTextContent(
      "Half the tests pass.",
    );
    const question = within(first).getByRole("group", { name: "Question from the dot" });
    expect(question).toHaveTextContent(asked.text);
    expect(question).toHaveTextContent("Question · waiting for your answer");
    expect(within(second).queryByRole("group", { name: /from the dot/ })).toBeNull();
  });

  it("replies to the open question from the bar above the composer", async () => {
    thread([run("a", "10:00")], [asked]);
    backend.on("message_reply", (args) =>
      run("c", "12:00", {
        parent_run_id: "a",
        status: "queued",
        payload: { message: `Answer to your question "${asked.text}": ${String(args.text)}` },
      }),
    );
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    expect(bar).toHaveTextContent(`${dot.name} asked: ${asked.text}`);
    await user.click(within(bar).getByRole("button", { name: "Reply to question" }));
    await user.type(screen.getByLabelText("Message"), "Ship it on Friday.{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([
        { id: asked.id, text: "Ship it on Friday." },
      ]),
    );
    expect(backend.callsTo("chat")).toEqual([]);
    // Opening the chat marked the question read.
    expect(backend.callsTo("message_mark_read")).toEqual([{ id: asked.id }]);
  });

  it("answers the open question with one of its choices", async () => {
    thread([run("a", "10:00")], [asked]);
    backend.on("message_reply", () => run("c", "12:00", { parent_run_id: "a", status: "queued" }));
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    await user.click(within(bar).getByRole("button", { name: "yes" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([{ id: asked.id, text: "yes" }]),
    );
  });

  it("shows no bar once the question is answered, and a message goes to the chat", async () => {
    const answered: Message = {
      ...asked,
      read_at: asked.created_at,
      answered_at: asked.created_at,
      answer_run_id: "b",
    };
    thread([run("a", "10:00")], [answered]);
    backend.on("chat", () => run("b", "12:00", { parent_run_id: "a", status: "queued" }));
    const { user } = renderApp(`/chat/${dot.id}/a`);
    expect(await screen.findByText("Question · answered")).toBeVisible();
    expect(screen.queryByRole("region", { name: "Open question" })).toBeNull();
    await user.type(screen.getByLabelText("Message"), "and now?{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "and now?", parentRunId: "a" },
      ]),
    );
    expect(backend.callsTo("message_reply")).toEqual([]);
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/ChatCheckIns.test.tsx`
Expected: FAIL: the groups "Note from the dot" and the region "Open question" are not found.

- [ ] **Step 3: Show the check-ins and add the bar**

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
import { LIMITS, type LibraryEntry, type Run, type RunEventRecord } from "@/api/types";
```

**with:**

```tsx
import {
  LIMITS,
  type LibraryEntry,
  type Message,
  type Run,
  type RunEventRecord,
} from "@/api/types";
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: {
  runs: Run[];
  events: Map<string, RunEventRecord[]>;
```

**with:**

```tsx
/// A note or a question the dot posted during a run, as a message from the dot (check-ins spec
/// section 9).
function CheckIn({ message }: { message: Message }) {
  const question = message.kind === "question";
  const state = !question
    ? null
    : message.answered_at
      ? "answered"
      : message.dismissed_at
        ? "dismissed"
        : "waiting for your answer";
  return (
    <div
      role="group"
      aria-label={question ? "Question from the dot" : "Note from the dot"}
      className="flex max-w-prose flex-col gap-1 self-start rounded-lg bg-muted px-3 py-1.5"
    >
      <p className="whitespace-pre-wrap">{message.text}</p>
      {state && <span className="text-xs text-muted-foreground">Question · {state}</span>}
    </div>
  );
}

/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: {
  runs: Run[];
  events: Map<string, RunEventRecord[]>;
  /// The dot's notes and questions of these runs, oldest first; each shows after its run.
  messages?: Message[];
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
              {items.length === 0 && message === null && run.status === "queued" && (
                <p className="text-muted-foreground">Waiting to start.</p>
              )}
            </div>
```

**with:**

```tsx
              {items.length === 0 && message === null && run.status === "queued" && (
                <p className="text-muted-foreground">Waiting to start.</p>
              )}
              {(props.messages ?? [])
                .filter((checkIn) => checkIn.run_id === run.id)
                .map((checkIn) => (
                  <CheckIn key={checkIn.id} message={checkIn} />
                ))}
            </div>
```

**In `ui/src/screens/ChatScreen.tsx`, replace:**

```tsx
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { eventsQuery, invalidate, keys } from "@/lib/query";
```

**with:**

```tsx
import { relativeTime } from "@/lib/format";
import { isOpenQuestion } from "@/lib/messages";
import { notifyError } from "@/lib/notify";
import { eventsQuery, invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
```

**In `ui/src/screens/ChatScreen.tsx`, replace:**

```tsx
  const { dotId, dotName, rootId } = props;
  const qc = useQueryClient();
```

**with:**

```tsx
  const { dotId, dotName, rootId } = props;
  const qc = useQueryClient();
  const navigate = useNavigate();
```

**In `ui/src/screens/ChatScreen.tsx`, replace:**

```tsx
  const latest = runs.at(-1);
  // A follow-up continues from the thread's last run, so it has to have ended.
  const busy = runs.some(isActive);
```

**with:**

```tsx
  const latest = runs.at(-1);
  // The dot's notes and questions of this thread, oldest first (check-ins spec section 9).
  const messages = useQuery({
    queryKey: keys.messages(dotId),
    queryFn: () => commands.messagesList(dotId),
  });
  const checkIns = (messages.data ?? [])
    .filter((m) => runs.some((run) => run.id === m.run_id))
    .sort((a, b) => a.created_at.localeCompare(b.created_at));
  const question = checkIns.filter(isOpenQuestion).at(-1);
  // "Reply to question" pressed: the composer sends the answer to the open question.
  const [replying, setReplying] = useState(false);
  const answering = replying && question !== undefined;
  // A follow-up continues from the thread's last run, so it has to have ended.
  const busy = runs.some(isActive);
```

**In `ui/src/screens/ChatScreen.tsx`, replace:**

```tsx
  if (thread.isPending) return null;
  if (thread.isError) return <ErrorNote error={thread.error} />;
```

**with:**

```tsx
  // Answers the open question. The core refuses a closed question or a busy thread, in words
  // shown as they are; the thread and the messages are loaded again either way.
  const answer = useMutation({
    mutationFn: (text: string) => commands.messageReply(question!.id, text),
    onSuccess: (run) => {
      setReplying(false);
      invalidate(qc, ["messages"]);
      // A thread whose end has no session is answered in a new chat, which opens.
      if (run.root_run_id !== rootId) void navigate(`/chat/${dotId}/${run.root_run_id}`);
      return reload();
    },
    onError: (error) => {
      notifyError(error);
      invalidate(qc, ["messages"]);
      return reload();
    },
  });
  // Opening the chat marks its open question read; it stays open until it is answered.
  const unread = question !== undefined && question.read_at === null ? question.id : null;
  useEffect(() => {
    if (unread === null) return;
    void commands.messageMarkRead(unread).then(
      () => invalidate(qc, ["messages"]),
      () => {},
    );
  }, [unread, qc]);
  if (thread.isPending) return null;
  if (thread.isError) return <ErrorNote error={thread.error} />;
```

**In `ui/src/screens/ChatScreen.tsx`, replace:**

```tsx
        <Transcript runs={runs} events={eventsByRun} shownError={failure} />
```

**with:**

```tsx
        <Transcript runs={runs} events={eventsByRun} shownError={failure} messages={checkIns} />
```

**In `ui/src/screens/ChatScreen.tsx`, replace:**

```tsx
      <Composer
        disabled={busy || send.isPending || !latest || closed}
        hint={
          busy
            ? "The dot is working…"
            : closed
              ? "This chat cannot be continued"
              : "Reply to continue this chat"
        }
        onSend={(message) => send.mutateAsync(message)}
      />
    </>
  );
}
```

**with:**

```tsx
      {question && (
        <div
          role="region"
          aria-label="Open question"
          className="flex flex-wrap items-center gap-2 border-t px-3 py-2"
        >
          <p className="min-w-0 flex-1 whitespace-pre-wrap">
            {dotName ?? "The dot"} asked: {question.text}
          </p>
          {question.choices?.map((choice) => (
            <Button
              key={choice}
              size="sm"
              variant="outline"
              disabled={busy || answer.isPending}
              onClick={onSingleClick(() => answer.mutate(choice))}
            >
              {choice}
            </Button>
          ))}
          <Button
            size="sm"
            variant={answering ? "default" : "outline"}
            aria-pressed={answering}
            disabled={busy}
            onClick={() => setReplying(!replying)}
          >
            Reply to question
          </Button>
        </div>
      )}
      <Composer
        disabled={
          answering ? busy || answer.isPending : busy || send.isPending || !latest || closed
        }
        hint={
          answering
            ? "Your answer to the question"
            : busy
              ? "The dot is working…"
              : closed
                ? "This chat cannot be continued"
                : "Reply to continue this chat"
        }
        onSend={(message) =>
          answering ? answer.mutateAsync(message) : send.mutateAsync(message)
        }
      />
    </>
  );
}
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/ChatCheckIns.test.tsx src/screens/ChatScreen.test.tsx src/screens/RunScreen.test.tsx`
Expected: PASS (`ChatCheckIns.test.tsx`: 4 tests); the existing chat and run view tests are unchanged.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/Transcript.tsx ui/src/screens/ChatScreen.tsx ui/src/screens/ChatCheckIns.test.tsx
git commit -m "feat(ui): show check-ins in a chat and answer from the reply bar"
```

### Task 12: Run-view lines, the Follow-ups list and the Settings switch

In the run view (and every transcript), a `notify`, `ask` or `follow_up` call shows as one readable line ("Sent a note: …", "Asked: …", "Scheduled a follow-up for Tue 21 Oct 09:00: …"), under either engine's name for it, with its result folded under it; a follow-up run shows its note under its trigger. The dot editor gains a "Follow-ups" tab that lists the dot's pending follow-ups (local time, note, Cancel); the Dots list says when a dot's next follow-up fires ("Next follow-up in 3 h"). Settings gain the switch "Notify on dot notes".

**Files:**

- Modify: `ui/src/lib/transcript.ts` (`TranscriptItem`; `checkInLine` and its helpers; the `tool_call` case)
- Modify: `ui/src/components/Transcript.tsx` (import near line 15; `FollowUpLine` after `ScheduleLine`; the `checkin` case in `Item`; `FollowUpLine` under `ScheduleLine`)
- Create: `ui/src/components/FollowUpsList.tsx`
- Modify: `ui/src/screens/DotEditorScreen.tsx` (import; the tabs near line 213)
- Modify: `ui/src/screens/DotsScreen.tsx` (import; a query; the "Next run" cell)
- Modify: `ui/src/screens/SettingsScreen.tsx` (the General section near line 177)
- Test: `ui/src/screens/RunCheckIns.test.tsx`, `ui/src/screens/FollowUps.test.tsx`, `ui/src/screens/SettingsNotes.test.tsx` (new)

**Interfaces:**

- Consumes: `commands.{followUpsList, followUpCancel}`, `keys.followUps`, `Settings.notify_on_notes` (Task 9); `localDateTime`, `relativeTime`, `invalidate`, `onSingleClick`.
- Produces: `TranscriptItem` gains `{ type: "checkin"; seq; kind: CheckInKind; input; result }`; `export type CheckInKind = "notify" | "ask" | "follow_up"`; `checkInLine(kind: CheckInKind, input: unknown, result: { output: string; isError: boolean } | null): string`; `<FollowUpsList dotId={string} />` (a list named "Follow-ups", items named "Follow-up due <local time>").

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/screens/RunCheckIns.test.tsx`:**

```tsx
import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Run, RunEventRecord } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const run: Run = { ...sample.run, branch: null, base_commit: null };
let seq = 0;
const ev = (data: Record<string, unknown>, runId = run.id): RunEventRecord => ({
  run_id: runId,
  seq: ++seq,
  ts: run.queued_at,
  kind: String(data.kind),
  data,
});

/// Answers the run view's queries for one thread.
function thread(runs: Run[], events: RunEventRecord[] = []) {
  backend.on("run_thread", () => runs);
  backend.on("get_dot", () => sample.dot);
  backend.on("list_run_events", (args) =>
    events.filter((e) => e.run_id === args.runId && e.seq > (args.afterSeq as number)),
  );
  backend.on("approvals_for_run", () => []);
  backend.on("worktree_diff", () => sample.diff);
}

describe("check-ins in the run view", () => {
  it("shows each check-in call as one readable line, under either engine's name", async () => {
    thread(
      [run],
      [
        ev({ kind: "tool_call", id: "c1", tool: "Notify", input: { text: "Half way there." } }),
        ev({ kind: "tool_result", id: "c1", output: "Sent.", is_error: false }),
        ev({
          kind: "tool_call",
          id: "c2",
          tool: "mcp__dots__ask",
          input: { question: "Ship it?", choices: ["yes", "no"] },
        }),
        ev({ kind: "tool_result", id: "c2", output: "Your question was sent.", is_error: false }),
        ev({
          kind: "tool_call",
          id: "c3",
          tool: "FollowUp",
          input: { note: "Check the build.", in: "2h" },
        }),
        ev({
          kind: "tool_result",
          id: "c3",
          output: "Follow-up 01J0FUP1 scheduled for Tue 21 Oct 09:00.",
          is_error: false,
        }),
        ev({
          kind: "tool_call",
          id: "c4",
          tool: "mcp__dots__follow_up",
          input: { note: "Too soon.", in: "1m" },
        }),
        ev({
          kind: "tool_result",
          id: "c4",
          output: "A follow-up must be at least 5 minutes away.",
          is_error: true,
        }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("Sent a note: Half way there.")).toBeVisible();
    expect(screen.getByText("Asked: Ship it?")).toBeVisible();
    expect(
      screen.getByText("Scheduled a follow-up for Tue 21 Oct 09:00: Check the build."),
    ).toBeVisible();
    const refused = screen.getByText("Scheduled a follow-up: Too soon.").closest("details")!;
    expect(within(refused).getByText("failed")).toBeInTheDocument();
    expect(
      within(refused).getByText("A follow-up must be at least 5 minutes away."),
    ).toBeInTheDocument();
    expect(screen.queryByText("mcp__dots__ask")).toBeNull();
    expect(screen.queryByText("FollowUp")).toBeNull();
  });

  it("says that a follow-up run is one the dot scheduled, with its note", async () => {
    const followUp: Run = {
      ...run,
      trigger: "follow_up",
      payload: {
        message: "Follow-up you scheduled on Mon 20 Oct 09:00: Check the build.",
        follow_up_id: "01J0FUP1",
      },
    };
    thread([followUp]);
    renderApp(`/runs/${followUp.id}`);
    expect(await screen.findByText("Follow-up the dot scheduled")).toBeVisible();
    expect(
      screen.getByText("Follow-up you scheduled on Mon 20 Oct 09:00: Check the build."),
    ).toBeVisible();
  });
});
```

**Create `ui/src/screens/FollowUps.test.tsx`:**

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { FollowUp } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [pending] = sample.followUps;

describe("follow-ups", () => {
  it("lists a dot's pending follow-ups in its editor and cancels one", async () => {
    let list: FollowUp[] = [{ ...pending, dot_id: sample.dot.id }];
    backend.on("get_dot", () => sample.dot);
    backend.on("follow_ups_list", (args) => (args.dotId === sample.dot.id ? list : []));
    backend.on("follow_up_cancel", (args) => {
      list = [];
      return { ...pending, id: String(args.id), status: "cancelled" };
    });
    const { user } = renderApp(`/dots/${sample.dot.id}`);
    await user.click(await screen.findByRole("tab", { name: "Follow-ups" }));
    const item = await screen.findByRole("listitem", {
      name: `Follow-up due ${localDateTime(pending.due_at)}`,
    });
    expect(item).toHaveTextContent(pending.note);
    await user.click(within(item).getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(backend.callsTo("follow_up_cancel")).toEqual([{ id: pending.id }]),
    );
    expect(await screen.findByText(/No follow-up waits/)).toBeVisible();
  });

  it("says in the Dots list when a dot's next follow-up fires", async () => {
    backend.on("dot_overview", () => [sample.overview]);
    const soon = new Date(Date.now() + 3 * 3_600_000).toISOString();
    backend.on("follow_ups_list", () => [
      { ...pending, dot_id: sample.overview.dot.id, due_at: soon },
    ]);
    renderApp("/dots");
    expect(await screen.findByText("Next follow-up in 3 h")).toBeVisible();
  });
});
```

**Create `ui/src/screens/SettingsNotes.test.tsx`:**

```tsx
import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Settings } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

describe("the Settings switch for dot notes", () => {
  it("is on by default and is saved with the other settings", async () => {
    let current: Settings = sample.settings;
    backend.on("get_settings", () => current);
    backend.on("update_settings", (args) => {
      current = args.settings as Settings;
      return { settings: current, restart_required: false };
    });
    const { user } = renderApp("/settings");
    const toggle = await screen.findByLabelText("Notify on dot notes");
    expect(toggle).toBeChecked();
    expect(screen.getByText("Questions from dots always notify.")).toBeVisible();
    await user.click(toggle);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...sample.settings, notify_on_notes: false } },
      ]),
    );
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/RunCheckIns.test.tsx src/screens/FollowUps.test.tsx src/screens/SettingsNotes.test.tsx`
Expected: FAIL: the lines "Sent a note: …", the tab "Follow-ups", the text "Next follow-up in 3 h" and the switch "Notify on dot notes" are not found.

- [ ] **Step 3: Add the lines, the list and the switch**

**In `ui/src/lib/transcript.ts`, replace:**

```ts
  | { type: "error"; seq: number; text: string }
```

**with:**

```ts
  /// A call of a check-in tool (`notify`, `ask`, `follow_up`), shown as one line.
  | {
      type: "checkin";
      seq: number;
      kind: CheckInKind;
      input: unknown;
      /// `null` until the result arrives.
      result: { output: string; isError: boolean } | null;
    }
  | { type: "error"; seq: number; text: string }
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
/// One line that says what a tool call is about.
```

**with:**

```ts
export type CheckInKind = "notify" | "ask" | "follow_up";

/// The check-in tools by the names their calls are stored under: the policy name on the local
/// engine, Dots' own MCP server's tool on the Claude engine.
const CHECK_INS = new Map<string, CheckInKind>([
  ["Notify", "notify"],
  ["mcp__dots__notify", "notify"],
  ["Ask", "ask"],
  ["mcp__dots__ask", "ask"],
  ["FollowUp", "follow_up"],
  ["mcp__dots__follow_up", "follow_up"],
]);

/// The trimmed string argument `key` of a call, or "".
function field(input: unknown, key: string): string {
  const value =
    typeof input === "object" && input !== null
      ? (input as Record<string, unknown>)[key]
      : undefined;
  return typeof value === "string" ? value.trim() : "";
}

/// The line a check-in call shows as (check-ins spec section 9). A follow-up names its time
/// once the core's answer has it: the core writes it in local time.
export function checkInLine(
  kind: CheckInKind,
  input: unknown,
  result: { output: string; isError: boolean } | null,
): string {
  switch (kind) {
    case "notify":
      return `Sent a note: ${field(input, "text")}`;
    case "ask":
      return `Asked: ${field(input, "question")}`;
    case "follow_up": {
      const when =
        result && !result.isError ? /scheduled for (.+)\.$/.exec(result.output)?.[1] : undefined;
      const note = field(input, "note");
      return when ? `Scheduled a follow-up for ${when}: ${note}` : `Scheduled a follow-up: ${note}`;
    }
  }
}

/// One line that says what a tool call is about.
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
  const calls = new Map<string, Extract<TranscriptItem, { type: "tool" }>>();
```

**with:**

```ts
  const calls = new Map<string, Extract<TranscriptItem, { type: "tool" | "checkin" }>>();
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
      case "tool_call": {
        const call = {
          type: "tool" as const,
          seq: record.seq,
          tool: event.tool,
          input: event.input,
          result: null,
        };
        calls.set(event.id, call);
        items.push(call);
        break;
      }
```

**with:**

```ts
      case "tool_call": {
        const kind = CHECK_INS.get(event.tool);
        const call =
          kind === undefined
            ? {
                type: "tool" as const,
                seq: record.seq,
                tool: event.tool,
                input: event.input,
                result: null,
              }
            : {
                type: "checkin" as const,
                seq: record.seq,
                kind,
                input: event.input,
                result: null,
              };
        calls.set(event.id, call);
        items.push(call);
        break;
      }
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
import { toolPreview, transcriptItems, type TranscriptItem } from "@/lib/transcript";
```

**with:**

```tsx
import {
  checkInLine,
  toolPreview,
  transcriptItems,
  type TranscriptItem,
} from "@/lib/transcript";
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  return <p className="mb-2 text-xs text-muted-foreground">{line}</p>;
}
```

**with:**

```tsx
  return <p className="mb-2 text-xs text-muted-foreground">{line}</p>;
}

/// The message a follow-up run carries: what the dot scheduled itself to do.
function FollowUpLine({ run }: { run: Run }) {
  if (run.trigger !== "follow_up" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const message = (run.payload as Record<string, unknown>).message;
  if (typeof message !== "string") return null;
  return <p className="mb-2 text-xs whitespace-pre-wrap text-muted-foreground">{message}</p>;
}

/// What became of a call: running, done, failed, or no result in a run that has ended.
function outcome(result: { isError: boolean } | null, active: boolean): string {
  if (result === null) return active ? "running" : "no result";
  return result.isError ? "failed" : "done";
}
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
    case "raw":
      return (
```

**with:**

```tsx
    case "checkin":
      return (
        <details className="rounded-md border px-2 py-1">
          <summary className="flex cursor-pointer items-center gap-2 text-xs">
            <span className="min-w-0 flex-1 truncate">
              {checkInLine(item.kind, item.input, item.result)}
            </span>
            <span className={item.result?.isError ? "text-destructive" : "text-muted-foreground"}>
              {outcome(item.result, active)}
            </span>
          </summary>
          {item.result && (
            <pre
              className={`mt-1 max-h-80 overflow-auto text-xs whitespace-pre-wrap ${
                item.result.isError ? "text-destructive" : "text-muted-foreground"
              }`}
            >
              {item.result.output}
            </pre>
          )}
        </details>
      );
    case "raw":
      return (
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
            <ScheduleLine run={run} />
```

**with:**

```tsx
            <ScheduleLine run={run} />
            <FollowUpLine run={run} />
```

**Create `ui/src/components/FollowUpsList.tsx`:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// A dot's pending follow-ups, soonest first, each with its local time, its note and Cancel
/// (check-ins spec section 9). The dot schedules them itself; only the person cancels one.
export function FollowUpsList({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.followUps(dotId),
    queryFn: () => commands.followUpsList(dotId),
  });
  const cancel = useMutation({
    mutationFn: (id: string) => commands.followUpCancel(id),
    onError: notifyError,
    // The Dots list shows the next one too. The core also announces the change.
    onSettled: () => invalidate(qc, ["follow-ups"]),
  });
  if (list.isPending) return null;
  if (list.isError) return <ErrorNote error={list.error} />;
  if (list.data.length === 0) {
    return (
      <p className="text-muted-foreground">
        No follow-up waits. A dot schedules one with its follow-up tool when its work has to
        wait.
      </p>
    );
  }
  return (
    <ul aria-label="Follow-ups" className="flex flex-col gap-2">
      {list.data.map((followUp) => (
        <li
          key={followUp.id}
          aria-label={`Follow-up due ${localDateTime(followUp.due_at)}`}
          className="flex items-center gap-3 rounded-lg border p-3"
        >
          <div className="flex min-w-0 flex-1 flex-col">
            <span className="font-medium">{localDateTime(followUp.due_at)}</span>
            <span className="truncate text-xs text-muted-foreground">{followUp.note}</span>
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={cancel.isPending}
            onClick={onSingleClick(() => cancel.mutate(followUp.id))}
          >
            Cancel
          </Button>
        </li>
      ))}
    </ul>
  );
}
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
```

**with:**

```tsx
import { ErrorNote } from "@/components/ErrorNote";
import { FollowUpsList } from "@/components/FollowUpsList";
import { Page } from "@/components/Page";
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
              <TabsTrigger value="runs">Runs</TabsTrigger>
            </TabsList>
            <TabsContent value="settings">{form}</TabsContent>
            <TabsContent value="runs">
              <RunsList dotId={dotId} />
            </TabsContent>
```

**with:**

```tsx
              <TabsTrigger value="runs">Runs</TabsTrigger>
              <TabsTrigger value="follow-ups">Follow-ups</TabsTrigger>
            </TabsList>
            <TabsContent value="settings">{form}</TabsContent>
            <TabsContent value="runs">
              <RunsList dotId={dotId} />
            </TabsContent>
            <TabsContent value="follow-ups">
              <FollowUpsList dotId={dotId} />
            </TabsContent>
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
import { LIMITS, type DotOverview } from "@/api/types";
```

**with:**

```tsx
import { LIMITS, type DotOverview, type FollowUp } from "@/api/types";
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
```

**with:**

```tsx
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  // Every dot's pending follow-ups, soonest first: a row says when its dot's next one fires.
  const followUps = useQuery({
    queryKey: keys.followUps(null),
    queryFn: () => commands.followUpsList(null),
  }).data;
  const nextFollowUp = (dotId: string): FollowUp | undefined =>
    followUps?.find((followUp) => followUp.dot_id === dotId);
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
            const retry = retrying(row, paused);
```

**with:**

```tsx
            const retry = retrying(row, paused);
            const followUp = nextFollowUp(dot.id);
```

**In `ui/src/screens/DotsScreen.tsx`, replace:**

```tsx
                    <span>{nextRun(row, paused)}</span>
                    {retry && <span className="text-xs text-muted-foreground">{retry}</span>}
```

**with:**

```tsx
                    <span>{nextRun(row, paused)}</span>
                    {retry && <span className="text-xs text-muted-foreground">{retry}</span>}
                    {followUp && (
                      <span className="text-xs text-muted-foreground">
                        Next follow-up {relativeTime(followUp.due_at)}
                      </span>
                    )}
```

**In `ui/src/screens/SettingsScreen.tsx`, replace:**

```tsx
          <Label htmlFor="settings-autostart">Start Dots when I sign in</Label>
        </div>
```

**with:**

```tsx
          <Label htmlFor="settings-autostart">Start Dots when I sign in</Label>
        </div>
        <div className="flex items-center gap-2">
          <Switch
            id="settings-notify-notes"
            checked={draft.notify_on_notes}
            onCheckedChange={(notify_on_notes) => set({ notify_on_notes })}
          />
          <Label htmlFor="settings-notify-notes">Notify on dot notes</Label>
        </div>
        <p className="text-xs text-muted-foreground">Questions from dots always notify.</p>
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/RunCheckIns.test.tsx src/screens/FollowUps.test.tsx src/screens/SettingsNotes.test.tsx src/screens/RunScreen.test.tsx src/screens/DotsScreen.test.tsx src/screens/DotEditorScreen.test.tsx src/screens/SettingsScreen.test.tsx src/lib/transcript.test.ts`
Expected: PASS (5 new tests); the existing run view, Dots, editor, Settings and transcript tests are unchanged.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/transcript.ts ui/src/components/Transcript.tsx ui/src/components/FollowUpsList.tsx ui/src/screens/DotEditorScreen.tsx ui/src/screens/DotsScreen.tsx ui/src/screens/SettingsScreen.tsx ui/src/screens/RunCheckIns.test.tsx ui/src/screens/FollowUps.test.tsx ui/src/screens/SettingsNotes.test.tsx
git commit -m "feat(ui): check-in lines in the run view, follow-ups in the dot editor and the Dots list, and the notes switch"
```

### Task 13: Bring the specs in line with what was built

The check-ins spec gains section 13, "Changes made while building", from the table "Deviations from the spec" and any fix a task's step recorded in its commit body. The passages its section 12 names are changed in the main spec, the MCP/skills spec and the context library spec.

**Files:**

- Modify: `docs/superpowers/specs/2026-10-20-checkins-followups-design.md` (section 13 appended)
- Modify: `docs/superpowers/specs/2026-10-08-dots-design.md` (§1 table; §3 `runs.trigger`; §5 presets; §12 notifications)
- Modify: `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` (end of §4)
- Modify: `docs/superpowers/specs/2026-10-19-context-library-design.md` (§4, Claude engine)

**Interfaces:**

- Consumes: the Deviations table of this plan; `git log --format=%B feat/checkins-followups` for fixes recorded by Tasks 1 to 12.
- Produces: specs that match the code.

- [ ] **Step 1: Edit the specs**

**Append to `docs/superpowers/specs/2026-10-20-checkins-followups-design.md`:**

```markdown

## 13. Changes made while building

The plan (`docs/superpowers/plans/2026-10-20-checkins-followups.md`) changed these from the first version of this spec:

- Every preset allows `Notify`, `Ask` and `FollowUp`, Read-only included, as every preset allows `Skill` and `Context`; migration 0008 adds them to every stored policy whose preset is not Custom (section 4).
- A follow-up whose run cannot be stored is tried again after 1, 5 and 30 minutes; the fourth failed start skips it with the note "A follow-up was skipped because its run could not be started (<error>). It was due <time>: <note>". Any error while firing one follow-up (reading its thread, storing its run) counts as a failed start (section 6).
- The late words say `hour` for one hour; the hours are the lateness rounded, at least 1, and they end the message's first line (section 6).
- A disabled dot's follow-up posts "A follow-up was skipped because the dot is off. It was due <time>: <note>" (section 6).
- The first prompt of a follow-up run that cannot resume its thread has a `## Follow-up` section with the message; a follow-up run's payload also holds `follow_up_id` (section 6).
- `Message` carries `dot_name`, read from the dot (section 7).
- `messages_list` answers every message that waits (open questions, unread notes) and the 200 newest others, newest first; `follow_ups_list` answers the pending follow-ups, soonest first (section 10).
- Dismissing a dismissed question changes nothing; an answered one is refused with "this question has already been answered"; a note cannot be dismissed (section 5).
- A blank `at` or `in` counts as absent (section 2).
- The chat's bar also offers the question's choices; an answer in a thread whose end has no session opens the new chat (section 9).
```

Then add, under the same heading, one line for each fix that a task's commit body recorded (`git log --format=%B master..feat/checkins-followups`), in the words of the lines above.

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
| Triggers (v1) | Schedule (once, every, daily, weekly, cron; Scheduler v2), webhook, manual (Run now + chat) |
```

**with:**

```markdown
| Triggers (v1) | Schedule (once, every, daily, weekly, cron; Scheduler v2), webhook, manual (Run now + chat), follow-up (a run a dot scheduled for itself; `2026-10-20-checkins-followups-design.md`) |
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- `trigger` TEXT (`schedule` | `webhook` | `manual` | `chat`), `payload` TEXT NULL (JSON)
```

**with:**

```markdown
- `trigger` TEXT (`schedule` | `webhook` | `manual` | `chat` | `resume` | `follow_up`), `payload` TEXT NULL (JSON)
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - Every preset allows `Skill` (the tool that reads a skill) and, from the context library,
    `Context` (the tool that reads the dot's context collections), and sets `mcp_default`:
    `ask` for all but `read-only`, which sets `deny`. A rule of `Context` is not passed to the
    CLI's `--allowedTools` / `--disallowedTools`: the CLI has no such tool, and Dots' own
    server checks each call against it.
```

**with:**

```markdown
  - Every preset allows `Skill` (the tool that reads a skill), from the context library
    `Context` (the tool that reads the dot's context collections), and from the check-ins spec
    `Notify`, `Ask` and `FollowUp` (a dot's note, question and follow-up), and sets
    `mcp_default`: `ask` for all but `read-only`, which sets `deny`. Dots' own tools are
    `Context`, `Notify`, `Ask` and `FollowUp`: their rules are not passed to the CLI's
    `--allowedTools` / `--disallowedTools`, because the CLI has no such tools, and Dots' own
    `dots` server checks each call against them on the Claude engine.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  already announced. From Scheduler v2 a failed run with trigger `schedule` does not notify;
  `ScheduleAttention` does, once: "<dot>: scheduled run failed" with the error's first line.
```

**with:**

```markdown
  already announced. From Scheduler v2 a failed run with trigger `schedule` does not notify;
  `ScheduleAttention` does, once: "<dot>: scheduled run failed" with the error's first line.
  From the check-ins spec, `MessagePosted` notifies: a dot's question always ("<dot> asks:
  <question>"), a note while the setting `notify_on_notes` is on ("Note from <dot>").
```

**In `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md`, replace:**

```markdown
## 5. The gateway
```

**with:**

```markdown
- Dots' own `dots` MCP server serves `approve` and, from the context library and the check-ins spec, `context`, `notify`, `ask` and `follow_up`; it checks each call of its own tools against the dot's policy (`2026-10-20-checkins-followups-design.md` section 2).

## 5. The gateway
```

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

```markdown
- Dots' own MCP server (`server/mcp.rs`, entry `dots`) lists, for a run with collections, a second tool `context` with the same arguments and description, besides `approve`.
```

**with:**

```markdown
- Dots' own MCP server (`server/mcp.rs`, entry `dots`) lists, for a run with collections, a second tool `context` with the same arguments and description, besides `approve`. From the check-ins spec it also lists `notify`, `ask` and `follow_up` for every run (`2026-10-20-checkins-followups-design.md` section 2).
```

- [ ] **Step 2: Run the checks**

Run: `git diff --check`, then `grep -n "follow_up" docs/superpowers/specs/2026-10-08-dots-design.md`
Expected: no whitespace errors; the two new mentions of `follow_up` in the main spec.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-10-20-checkins-followups-design.md docs/superpowers/specs/2026-10-08-dots-design.md docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md docs/superpowers/specs/2026-10-19-context-library-design.md
git commit -m "docs(spec): bring the specs in line with the check-ins as built"
```

### Task 14: The smoke test of check-ins and follow-ups

Spec section 11 asks for a smoke with a real `claude` dot that asks, is answered, and schedules a follow-up 5 minutes ahead. As in `docs/verification/2026-10-19-context-library-smoke.md`, the agent drives a debug build with a local model (Part A) and never runs the real `claude` CLI; Part B lists what a person runs at the machine. The record goes to `docs/verification/2026-10-20-checkins-followups-smoke.md`, in the form of the previous smoke: a header (date, build and commit, model, how many steps passed, where the screenshots are), "What this covers", "How the agent steps were run", then each step with its commands, "Expect:" and "Result:" from what happened, and the findings.

**Files:**

- Create: `docs/verification/2026-10-20-checkins-followups-smoke.md`

**Interfaces:**

- Consumes: a debug build (`bun run tauri build --debug --no-bundle` in the repository root), a fresh `DOTS_DATA_DIR`, `tools/smoke/webview.mjs`, LM Studio with a tool-calling model, as the previous smoke set them up.
- Produces: the smoke record.

- [ ] **Step 1: Agent: a local dot notes, asks and is answered.** A local dot (folder workspace, `approval_wait_secs: 0`, preset `sandboxed`) with the instructions "Call notify with 'Starting.', then ask the person 'Ship it?' with the choices yes and no, then finish." Run it now. Expect: the run succeeds; the transcript shows "Sent a note: Starting." and "Asked: Ship it?"; `messages_list` holds a note and an open question; the Inbox's "Messages" group shows both first, and the sidebar count includes them; a notification "<dot> asks: Ship it?" appears. Click "yes". Expect: a chat run continues the thread with the message `Answer to your question "Ship it?": yes`, and the question leaves the Inbox.

- [ ] **Step 2: Agent: a follow-up 5 minutes ahead fires and continues the thread.** On the same dot, instructions "Call follow_up with the note 'Check again.' and in '5m', then finish." Expect: the result "Follow-up <id> scheduled for <local time>."; the dot editor's "Follow-ups" tab lists it; the Dots list shows "Next follow-up in 5 min". Wait 5 minutes. Expect: a run with the trigger `follow_up` ("Follow-up the dot scheduled") continues the thread with "Follow-up you scheduled on <time>: Check again." and the list is empty.

- [ ] **Step 3: Agent: limits, pause, a disabled dot and a closed Dots.** (a) Ask for `in: '1m'` and for a fourth pending follow-up: the run shows the refused lines with their sentences. (b) Schedule one 5 minutes ahead, pause all schedules, wait past it: nothing runs; resume: it fires. (c) Schedule one, disable the dot, wait past it: it is skipped and a note says why. (d) Schedule one 5 minutes ahead, close Dots, and start it again 20 minutes after the follow-up was due: it fires once at start, and its message ends "(this is 1 hour late; Dots was not running at the time)". (e) Turn "Notify on dot notes" off: a note posts without a notification; a question still notifies.

- [ ] **Step 4: Person: a real Claude dot.** With a real `claude` CLI, a Claude dot with the instructions "Ask me whether to ship (choices yes, no), then finish." Expect: `--allowedTools` holds `mcp__dots__notify mcp__dots__ask mcp__dots__follow_up`; the transcript shows "Asked: …" without an approval; answering from the chat's bar resumes the session. Then "Schedule a follow-up in 5m to check the build, then finish": it fires 5 minutes later and resumes the same session.

- [ ] **Step 5: Write the record and commit**

Write `docs/verification/2026-10-20-checkins-followups-smoke.md` from what happened in Steps 1 to 3 (each step's commands, "Expect:" and "Result:", with run ids and the texts seen), Step 4 listed as open for the person, and the findings (a Dots defect is fixed in its own commit before this one, and named here).

```bash
git add docs/verification/2026-10-20-checkins-followups-smoke.md
git commit -m "docs(verification): the check-ins and follow-ups smoke"
```
