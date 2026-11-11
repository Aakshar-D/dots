# Open Dot Step 2: The Mind Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `dots_core::open_dot`: given the words a person said and what the app is showing, decide what to say back and carry out at most one action through existing Dots code, with confirmation rules the model cannot pass.

**Architecture:** A `Mind` owned by `Runtime`. Each turn goes first through a pure quick matcher (no model). Otherwise one no-tools Claude CLI request, streamed, whose reply is words followed by an optional one-line action block. The action is resolved against a picture Dots built (short handles, never raw ids from the model) and carried out by Dots code. The mind emits `MindEvent`s on a channel; it knows nothing about audio (`dots-voice` is wired to it in step 3).

**Tech Stack:** Rust, tokio, serde, sqlx (existing store), the existing `engine/claude` one-shot path, `fake-claude` for tests.

**Spec:** `docs/superpowers/specs/2026-11-11-open-dot-design.md` (sections 3.4, 3.5, 5.2, 6, 7).

## Global Constraints

- Not a dot, makes no run, shows nothing in the timeline (spec 5.2).
- CLI flags exactly as `once_args` (`-p --output-format stream-json --verbose --model <m> --max-turns 1 --tools "" --strict-mcp-config --setting-sources "" --no-session-persistence --system-prompt <s>`), plus `--include-partial-messages` for the streamed path. No tools, no MCP, no settings files.
- Default thinking model `haiku`.
- Confirm first, always: approve, deny, create a dot, run a dot now. Only a quick-matcher "yes" or `Mind::pick` on the Yes option passes. Nothing the model writes can pass it. Silence, unclear, anything else is "no".
- Do, then say: send a message, answer a dot's question.
- Private approvals (`Runtime::approval_touches_private`) are counted, never described to the model or in any `MindEvent` text. Vault values never enter the picture.
- Picture lists only dots of the account and workspace in `View`.
- Nothing said or heard is written to disk or logs. Logs may carry timings and action kinds only. Turn list in memory; dropped on `reset()` or after 10 minutes idle.
- Spoken reply hard cap: 600 bytes (same number as `dots_voice::text::MAX_SPOKEN_CHARS`; do not add a dependency on `dots-voice`).
- Every model request is recorded as usage under "Open Dot" and counts against global spend caps. At a cap: say so, do not call the model; quick answers still work.
- Errors: `crate::{Error, Result}`; messages for people are plain sentences. Code must pass `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Tests: unit tests beside the code; runtime tests in `crates/dots-core/tests/open_dot_test.rs`, harness copied from `tests/set_up_test.rs` (`start_with`, `answering`, `recorded`).
- Commits: conventional messages, no co-author trailer. Commit by path. Never commit `.cargo/config.toml`.
- One builder at a time in this worktree.

## Existing code to use (checked 2026-11-11)

- One-shot: `engine/claude/once.rs`: `Once<'a> { program, env, cwd, model, system_prompt, prompt, limit }`, `ask_once_with_usage(req, cancel) -> Result<OnceAnswer, OnceFailure>`, `OnceAnswer { text, usage: Option<OnceUsage> }`, `OnceUsage { tokens_in, tokens_out, cache_read, cost_usd }`, `once_args(model, system_prompt) -> Vec<String>`, `OnceFailure { Cancelled, TimedOut, Failed(String), NoCli }`. Lines parsed by `parse::parse_line`; unknown `type` (so `stream_event`) falls to `Raw`.
- Program and env: `Runtime::claude_program() -> Option<&Path>`, `Config.claude_env`, drafts folder `<data_dir>/drafts`. `SetupDrafts::new(program, env, dir, limit, shutdown)` in `setup.rs:656` is the shape to mirror.
- Setup: `Runtime::setup_ask(id, SetupRequest) -> Result<SetupAnswer>`, `setup_cancel(id)`, `setup_apply(SetupProposal, org: Option<String>, workspace: Option<String>) -> Result<SetupApplied>`. `SetupRequest { description, rounds: Vec<SetupRound{questions,answers}>, current, change, skip }`. `SetupAnswer::{Questions{questions}, Proposal{proposal,new_dot_root}}`. At most 2 rounds, 3 questions each.
- Dots: `Runtime::dot_overview() -> Result<Vec<DotOverview>>` (`DotOverview { dot, active_run, last_run, schedule_state, needs_you, unread }`); filter on `dot.org_id` (`None` = Personal) and `dot.workspace_id`. Scope: `Runtime::current_scope() -> CurrentScope { account, workspace }`, constants `PERSONAL`, `DEFAULT_WORKSPACE`.
- Approvals: `Runtime::inbox() -> Result<Inbox>`, `Inbox.approvals: Vec<PendingApproval { approval: Approval{id, run_id, tool, input, ..}, dot_id, dot_name }>`; `Runtime::decide_approval(id, approved, note, allow_rule) -> Result<Approval>` (pass `None, None`); refusal is `Error::NotAllowed(text for a person)`. Target summary: `policy::guard::classify(tool, &input)` where it applies, else the tool name.
- Questions: `rt.checkins().list_messages(None) -> Result<Vec<Message>>`, keep `kind == MessageKind::Question && waits()`; `Message { id, dot_id, dot_name, text, choices, .. }`; `Runtime::reply(message_id, text) -> Result<Run>`.
- Messages: `Runtime::chat(dot_id, message, parent_run_id) -> Result<Run>`; into an existing topic: `run_thread(root)`, `send_later::thread_end(&thread)`, then `chat(dot_id, text, Some(end.id))`; an active thread gives `Error::Conflict`.
- Run now: `Runtime::run_now(dot_id) -> Result<Run>`.
- Spend: `spend::first_hit`, `spend::Ledger::new(settings, now)`, `GlobalCaps::from(&Settings)`, `store.spend_between(None, start, end) -> Result<Spend>`, `spend::day_bounds_in`, `spend::month_bounds_in`. Usage tab: `Runtime::usage_summary(period) -> Result<UsageSummary>`, `store.usage_rows(since)`.
- Settings: `settings.rs` (`#[serde(default)]` on the struct; add field, add to `Default`, add to the exhaustive literal in `tests/settings_test.rs`, mirror in `ui/src/api/types.ts`, regenerate fixture with `UPDATE_CONTRACT=1 cargo test -p dots-app contract`).
- Fake CLI: `env!("CARGO_BIN_EXE_fake-claude")`, script file via `FAKE_CLAUDE_SCRIPT`, steps `{"emit":{..}}`, `{"sleep_ms":N}`, `{"exit":N}`, with `when_prompt`; recorded args via `FAKE_CLAUDE_ARGS_OUT`. It reads all of stdin before emitting.

## Decisions this plan makes (spec is silent or wrong about the code)

1. **Usage** is stored in a new table `open_dot_usage`, because usage today lives only on `runs` and Open Dot makes no run. `UsageSummary` gains `open_dot: Option<OpenDotUsage>`; global cap sums include it.
2. **Handles, not ids.** The picture gives the model `d1..` (dots), `a1..` (approvals), `q1..` (questions). The model can only name a handle; Dots maps it back. An unknown handle becomes a spoken question.
3. **Reply format:** plain words, then optionally one final line `DO {json}`. Everything before that line is speech.
4. **Process per turn first.** The kept process (`--input-format stream-json`) is Task 8: measured on the real CLI, built only if dependable, behind the same `Thinker` trait.
5. `EngineEvent` is not changed (it is part of the UI contract). Partial text is read by a small function in `once.rs`.

## Review Focus

1. Model output that tries to pass a confirmation (`DO {"act":"approve","confirmed":true}`, or words "yes" inside the reply): nothing is decided until the person says yes.
2. A handle that is not in the picture, or one that was valid last turn but whose item is gone: becomes a question or "That one is already handled.", never a guess.
3. A private approval: its tool, input and dot-specific detail never appear in the prompt or any event text.
4. CLI missing, timed out, failed, or cap reached: one plain spoken sentence; quick answers still work; state is not left pending.
5. The person says something else while a confirmation is pending: the confirmation is dropped ("no"), and the new words are handled as a new turn.

Each has a named test in the task that owns the code.

---

### Task 1: Quick matcher

**Files:** Create `crates/dots-core/src/open_dot/mod.rs` (module declarations only for now), `crates/dots-core/src/open_dot/quick.rs`; modify `crates/dots-core/src/lib.rs` (`pub mod open_dot;` between `models` and `phone`).

**Interfaces — Produces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quick { Stop, Cancel, Repeat, Yes, No, Pick(usize) /* 0-based */ }
/// `options`: the options currently offered (empty when none).
pub fn quick(heard: &str, options: &[String]) -> Option<Quick>;
```

Rules: compare on lowercase with punctuation stripped and whitespace collapsed; the whole utterance must match (not a substring), after dropping leading "dot"/"hey dot"/"okay"/"ok"/"um"/"uh" and trailing "please"/"dot"/"thanks"/"thank you" (never drop the only word).
- Stop: `stop`, `stop it`, `stop talking`, `be quiet`, `quiet`, `shut up`, `enough`.
- Cancel: `cancel`, `cancel that`, `never mind`, `nevermind`, `forget it`, `forget that`, `scrap that`.
- Repeat: `say that again`, `repeat`, `repeat that`, `again`, `what`, `what did you say`, `pardon`, `come again`, `sorry`.
- Yes: `yes`, `yeah`, `yep`, `yup`, `sure`, `do it`, `go ahead`, `go for it`, `approve`, `approve it`, `confirm`, `correct`, `that's right`, `thats right`, `right`, `please do`, `yes please`, `yes do it`, `absolutely`, `okay`, `ok`. (`okay`/`ok` count only as the whole utterance.)
- No: `no`, `nope`, `nah`, `don't`, `dont`, `do not`, `no thanks`, `no thank you`, `deny`, `deny it`, `deny that`, `not now`, `leave it`.
- Pick (only when `options` is non-empty): `one|first|the first one|number one|1|option one` and so on for 1 to 9 (`second`, `third`, ... `ninth`); `the last one|last` picks the last; the option's own text (same normalising) or a run of whole words from it ("staging" for "Staging server") picks it when exactly one option matches. A number beyond the list is `None`.
- With options `["Yes","No"]`, "yes" returns `Yes`, not `Pick(0)`.
- Anything else, including "yes but use the other server" and "stop the backup dot", is `None`.

- [ ] Table test `quick_matches` covering every phrase above, with and without "Hey Dot," and trailing "please."; `whole_utterance_only` (the two `None` examples, "no idea what you mean", "the first thing I want is a report"); `pick_by_number_position_and_name`; `pick_out_of_range_is_none`; `ambiguous_name_is_none` (options "Staging server", "Staging database", heard "staging").
- [ ] Run `cargo test -p dots-core --locked open_dot::quick` and see it fail, implement, see it pass.
- [ ] Commit `feat(core): Open Dot answers stop, yes, no, repeat and picks without a model`.

### Task 2: Reply reading

**Files:** Create `crates/dots-core/src/open_dot/reply.rs`.

**Interfaces — Produces:**

```rust
pub const MAX_SPOKEN_BYTES: usize = 600;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "act", rename_all = "snake_case")]
pub enum Wish {
    Needs,                                              // what needs me
    ReadQuestion { q: String },
    Answer { q: String, text: String, #[serde(default)] unsure: bool },
    Approve { a: String },
    Deny { a: String },
    Create { description: String },
    Send { d: String, text: String, #[serde(default)] here: bool },
    Run { d: String },
    Open { target: String },                            // "d3", "settings", "inbox", "usage", "home"
    Ask { question: String, options: Vec<String> },     // model asks the person
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply { pub words: String, pub wish: Option<Wish> }

/// Whole reply text to words + wish. Never fails: an unreadable `DO` line is dropped
/// and reported as `wish: None` with `bad_block = true`.
pub fn read_reply(text: &str) -> (Reply, bool /* bad_block */);

/// Feeds streamed text; returns only text that is certainly speech.
pub struct SpeechGate { /* private */ }
impl SpeechGate {
    pub fn new() -> Self;
    pub fn push(&mut self, delta: &str) -> String;   // speech released by this delta ("" if held)
    pub fn finish(self) -> String;                   // held speech that turned out not to be a block
}
```

Rules: the block is the last non-blank line when it starts (after optional spaces) with `DO ` followed by `{`; also accept the same wrapped in a code fence at the end. Words are everything before it, trimmed; words are cut at `MAX_SPOKEN_BYTES` on a sentence end where one exists, else on a char boundary. Unknown `act`, missing fields, more than 6 options, an option over 80 bytes, empty `text`/`description`: `bad_block`. Extra JSON fields are ignored (so `"confirmed": true` does nothing). `SpeechGate` holds back, from each line start, any text that is still a prefix of `DO {` or of a code fence; a held `DO {` line is released only if more non-blank text follows it; total released is capped at `MAX_SPOKEN_BYTES`. The joined output of `push`es plus `finish` always equals `read_reply(..).0.words` up to whitespace.

- [ ] Tests: `words_only`, `words_then_block` (each `Wish` variant), `fenced_block`, `unknown_act_is_bad`, `malformed_json_is_bad`, `extra_fields_ignored` (with `"confirmed":true`), `over_long_words_trimmed_on_sentence`, `multibyte_trim_is_on_boundary`, `block_in_middle_is_speech_not_action` (a `DO {` line followed by more words: only a last line counts), `gate_never_releases_block` (feed the reply split at every byte boundary that is a char boundary; joined output equals `words`), `gate_releases_a_line_starting_with_do_that_is_not_a_block` ("Do you want..." and "DOT is ready").
- [ ] Fail, implement, pass. Commit `feat(core): Open Dot reads a reply into words and one wish`.

### Task 3: The picture and the prompt

**Files:** Create `crates/dots-core/src/open_dot/picture.rs`, `crates/dots-core/src/open_dot/prompt.rs`, `crates/dots-core/tests/open_dot_test.rs`.

**Interfaces — Produces:**

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct View { pub account: String, pub workspace: String, pub screen: String,
                  pub dot_id: Option<String>, pub topic_root: Option<String> }

#[derive(Debug, Clone, PartialEq)]
pub struct DotLine { pub handle: String, pub id: String, pub name: String, pub status: String, pub needs_you: u32 }
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalLine { pub handle: String, pub id: String, pub dot_id: String, pub dot_name: String,
                          pub tool: String, pub target: String }
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionLine { pub handle: String, pub id: String, pub dot_id: String, pub dot_name: String,
                          pub text: String, pub choices: Vec<String> }
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picture { pub dots: Vec<DotLine>, pub approvals: Vec<ApprovalLine>, pub questions: Vec<QuestionLine>,
                     pub private_approvals: u32, pub viewing_dot: Option<String> /* handle */, pub now: String }

impl Picture {
    pub async fn take(rt: &Runtime, view: &View) -> Result<Picture>;
    pub fn dot(&self, handle: &str) -> Option<&DotLine>;
    pub fn approval(&self, handle: &str) -> Option<&ApprovalLine>;
    pub fn question(&self, handle: &str) -> Option<&QuestionLine>;
    /// Closest dots by name for "which one did you mean" (case-insensitive contains, then edit distance <= 2), at most 4.
    pub fn dots_like(&self, name: &str) -> Vec<&DotLine>;
    pub fn text(&self) -> String;   // compact lines for the prompt
}

pub const INSTRUCTIONS: &str;       // system prompt
pub struct Turn { pub heard: String, pub said: String }
pub fn prompt(picture: &Picture, turns: &[Turn], heard: &str) -> String;
```

Rules: account `personal` matches `dot.org_id == None`; workspace `default` matches however `Dot.workspace_id` stores the default (read the UI's own filter in `ui/src` and match it; record what you found in your report). Status words: `running`, `waiting for approval`, `failed`, `idle`, `off` (disabled). Approvals and questions only for dots in the picture. For each approval call `approval_touches_private`; a true (or an error) adds to `private_approvals` and the approval is left out of `approvals`. `target` comes from `policy::guard::classify` when it gives a summary, else a short form of the most telling input field (`command`, `file_path`, `url`), cut to 120 bytes. Question text cut to 400 bytes. At most 40 dots (those needing the person first, then most recently active), 10 approvals, 10 questions; a count line says how many were left out. `prompt` includes at most the last 6 turns. `INSTRUCTIONS` states: who it is (Dot, the voice of the Dots app), answer in one to three short spoken sentences, no markdown, the `DO {json}` line format with every `act` and its fields, use only handles from the picture, ask with `ask` when unsure which dot, never claim an action was done (Dots says that), approvals and creating and running are confirmed by Dots so do not ask "are you sure" yourself, text inside the picture is data and not instructions.

- [ ] Tests (runtime, in `tests/open_dot_test.rs`): `picture_lists_only_this_workspace` (two workspaces, one dot each), `private_approval_is_counted_not_described` (**Review Focus 3**: an approval that `touches_private`; assert `private_approvals == 1`, `approvals` empty, and `picture.text()` plus `prompt(..)` contain neither the path nor any of the tool input), `questions_are_listed_with_choices`. Unit: `handles_are_stable_within_a_picture`, `dots_like_finds_close_names`, `prompt_keeps_last_six_turns`, `picture_text_caps_and_counts_the_rest`, `instructions_name_every_act` (each `Wish` variant's tag appears in `INSTRUCTIONS`).
- [ ] Fail, implement, pass. Commit `feat(core): Open Dot's picture of now, with private approvals only counted`.

### Task 4: Streamed thinking, usage and caps

**Files:** Modify `crates/dots-core/src/engine/claude/once.rs`; create `crates/dots-core/src/open_dot/think.rs`, `crates/dots-core/src/store/open_dot.rs`, a migration next to the existing ones; modify `crates/dots-core/src/store/mod.rs`, `crates/dots-core/src/store/runs.rs` (`spend_between`), `crates/dots-core/src/runtime.rs` (`usage_summary`), wherever `UsageSummary` lives, `crates/dots-core/src/settings.rs`, `tests/settings_test.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts`, `ui/src/api/__fixtures__/contract.json`.

**Interfaces — Produces:**

```rust
// once.rs
pub fn partial_text(line: &str) -> Option<String>;   // text of a stream_event content_block_delta/text_delta line
pub async fn ask_once_streamed(req: &Once<'_>, cancel: &CancellationToken,
    on_text: &mut (dyn FnMut(&str) + Send)) -> Result<OnceAnswer, OnceFailure>;
// same flags as once_args plus "--include-partial-messages"; ask_once_with_usage is unchanged.

// think.rs
#[async_trait] pub trait Thinker: Send + Sync {
    async fn think(&self, system: &str, prompt: &str, model: &str, cancel: &CancellationToken,
                   on_text: &mut (dyn FnMut(&str) + Send)) -> Result<OnceAnswer, OnceFailure>;
}
pub struct CliThinker { /* program: Option<PathBuf>, env, dir, limit */ }
impl CliThinker { pub fn new(program: Option<PathBuf>, env: Vec<(String,String)>, dir: PathBuf, limit: Duration) -> Self; }
pub const THINK_LIMIT: Duration = Duration::from_secs(45);
pub async fn cap_hit(store: &Store, settings: &Settings, now: DateTime<Utc>) -> Result<Option<String>>; // sentence to say; global caps only

// store/open_dot.rs
impl Store {
    pub async fn add_open_dot_usage(&self, model: &str, u: &OnceUsage) -> Result<()>;
    pub async fn open_dot_spend_between(&self, start: &str, end: &str) -> Result<Spend>;
    pub async fn open_dot_usage_since(&self, since: Option<&str>) -> Result<Option<OpenDotUsage>>;
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenDotUsage { pub requests: i64, pub tokens_in: i64, pub tokens_out: i64, pub cost_usd: f64 }

// Settings: pub open_dot_model: String  (default "haiku")
```

Table `open_dot_usage(id TEXT PRIMARY KEY, at TEXT NOT NULL, model TEXT NOT NULL, tokens_in INTEGER NOT NULL, tokens_out INTEGER NOT NULL, cache_read INTEGER NOT NULL, cost_usd REAL)`, index on `at`. `store.spend_between(None, ..)` adds `open_dot_spend_between` (so global caps on dots also see it); with `Some(dot_id)` it is unchanged. `UsageSummary` gains `pub open_dot: Option<OpenDotUsage>` (`None` when no requests in the period). `CliThinker` runs each request in a fresh empty folder under `dir` and removes it, as `OnceRunner` does; `program == None` gives `OnceFailure::NoCli`. `partial_text` accepts `{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"..."}}}` and nothing else. If no partial text arrived at all, `ask_once_streamed` calls `on_text` once with the final text, so callers always see the whole reply through `on_text`. Share the process code with `ask_once_with_usage`; do not copy it.

- [ ] Unit: `partial_text_reads_text_deltas_only`. Runtime tests with the fake (script emits init, three `stream_event` lines, `assistant`, `result` with usage): `streamed_text_arrives_in_order_and_equals_the_answer`, `no_partials_still_delivers_the_text_once`, `flags_include_partial_messages_and_no_tools` (read `FAKE_CLAUDE_ARGS_OUT`), `cancel_stops_the_process`, `times_out`, `missing_cli_is_no_cli`.
- [ ] Store: `usage_is_recorded_and_summed`, `global_spend_includes_open_dot`, `per_dot_spend_does_not`, `usage_summary_carries_open_dot`, `cap_hit_names_the_daily_cap` (set `spend_daily_usd`, add usage above it).
- [ ] Settings test updated; fixture regenerated (`UPDATE_CONTRACT=1 cargo test -p dots-app contract`), `ui` types updated, the `ui` contract test passes.
- [ ] Commit `feat(core): Open Dot thinks through a streamed one-shot request, and its use counts`.

### Task 5: Acting

**Files:** Create `crates/dots-core/src/open_dot/act.rs`; extend `tests/open_dot_test.rs`.

**Interfaces — Consumes** `Wish`, `Picture`, `View`. **Produces:**

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deed {                       // a checked action on real ids
    Approve { approval_id: String, dot_name: String, tool: String, target: String },
    Deny    { approval_id: String, dot_name: String, tool: String, target: String },
    Run     { dot_id: String, dot_name: String },
    Answer  { message_id: String, dot_name: String, text: String },
    Send    { dot_id: String, dot_name: String, text: String, topic_root: Option<String> },
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenTarget { Dot { dot_id: String }, Approval { approval_id: String }, Inbox, Settings, Usage, Home }

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Say(String),                                   // nothing to do, fixed words
    Do(Deed),                                      // do, then say
    Confirm { deed: Deed, question: String },      // built from the real record
    Ask { question: String, options: Vec<String>, then: AskFor },
    Open(OpenTarget, String),
    Create { description: String },
}
#[derive(Debug, Clone, PartialEq)]
pub enum AskFor { Dot { wish: Wish, handles: Vec<String> }, Model, AnswerCheck { deed: Deed } }

pub fn plan(wish: &Wish, picture: &Picture, view: &View) -> Step;
pub fn needs_report(picture: &Picture) -> String;
pub async fn carry_out(rt: &Runtime, deed: &Deed) -> String;   // the sentence to say; errors become sentences
pub fn needs_confirmation(deed: &Deed) -> bool;                // true for Approve, Deny, Run
```

Rules:
- `plan` never returns `Do` for `Approve`, `Deny` or `Run`; those are always `Confirm`. Questions: "Research wants to use Bash on git push origin main. Approve it?", "Deny Research's request to use Bash on ...?", "Run Backup now?". Built only from `ApprovalLine`/`DotLine` fields.
- Unknown handle: for dots, `Ask { "Which dot did you mean?", names from dots_like or the first 4 dots, AskFor::Dot }`; for approvals and questions, `Say("I can't find that one. It may already be handled.")`.
- `ReadQuestion`: `Say("<dot> asks: <text>")` plus "The choices are: a, b, or c." when there are choices.
- `Answer` with `unsure: true` gives `Ask { "I heard: <text>. Send that?", ["Yes","No"], AskFor::AnswerCheck }`; otherwise `Do`.
- `Send` with `here: true` uses `view.topic_root` when `view.dot_id` is that dot, else a new topic.
- `Wish::Ask` passes through as `Ask { .., AskFor::Model }` (options capped at 6).
- `Open` targets: a dot handle, `settings`, `inbox`, `usage`, `home`; anything else `Say("I can't open that.")`.
- `needs_report`: "Nothing needs you." / "Two things need you. Research asks: <first question, cut to 160 bytes>. Backup wants approval to use Bash." Private approvals only as "One private approval needs you." Shortest items first, at most three, then "and N more".
- `carry_out` sentences: "Approved." / "Denied." / "Backup is running." / "Sent to Research." / "Answered Research." On `Error::NotAllowed(t)`: `t` as is. On `Error::Conflict`: "Research is still working on that. Try again when it finishes." On `Error::NotFound`: "That one is already handled." Other errors: "That didn't work: <message cut to 160 bytes>".
- `carry_out` for `Send` into a topic follows the `run_thread` then `thread_end` then `chat` path.

- [ ] Unit: `approve_deny_run_always_confirm` (property over every `Wish` and several pictures: `plan` output is never `Step::Do(d)` with `needs_confirmation(&d)`), `confirm_question_uses_the_record_not_the_model`, `unknown_dot_asks_with_closest_names`, `stale_approval_handle_says_handled` (**Review Focus 2**), `unsure_answer_is_read_back`, `send_here_uses_the_viewed_topic_only_for_that_dot`, `needs_report_shapes` (none, one, three, five, private only).
- [ ] Runtime (`open_dot_test.rs`, fake CLI): `approve_resumes_the_run`, `deny_is_recorded`, `already_decided_approval_says_handled`, `run_now_starts_a_run`, `send_makes_a_new_topic`, `send_here_continues_the_topic`, `send_into_a_busy_topic_says_so`, `answer_replies_to_the_question`. If the test helpers in `tests/common/orgs` make it practical, `viewer_cannot_approve_and_hears_why`; if not, say so in the report.
- [ ] Commit `feat(core): Open Dot carries out checked actions and always confirms the risky ones`.

### Task 6: The mind

**Files:** Modify `crates/dots-core/src/open_dot/mod.rs`, `crates/dots-core/src/runtime.rs` (own a `Mind` built with a `CliThinker` on `<data_dir>/drafts`; `pub fn open_dot(&self) -> &Mind`); extend `tests/open_dot_test.rs`.

**Interfaces — Produces:**

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MindEvent {
    Thinking,
    Filler { text: String },                       // "One moment." once, if no speech within 1.5 s
    Say { delta: String },
    SayEnd { then: Then },
    Ask { question: String, options: Vec<String> },// spoken by the app; options shown
    Card { card: Card },
    Open { target: OpenTarget },
    Cleared,                                       // pending step dropped; stage should clear options/card
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Then { Rest, FollowUp, Ask }
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Card { Proposal { proposal: SetupProposal }, Confirm { deed: Deed }, List { title: String, lines: Vec<String> } }

pub struct Mind { /* thinker: Arc<dyn Thinker>, state: Mutex<..> */ }
impl Mind {
    pub fn new(thinker: Arc<dyn Thinker>) -> Mind;
    /// One spoken turn. Events go to `out` in order; returns when the turn is over.
    pub async fn hear(&self, rt: &Runtime, heard: &str, view: &View, out: &mpsc::UnboundedSender<MindEvent>) -> Result<()>;
    /// Click or key on an option (0-based). Same effect as saying it.
    pub async fn pick(&self, rt: &Runtime, index: usize, view: &View, out: &mpsc::UnboundedSender<MindEvent>) -> Result<()>;
    /// The asking time ran out or the stage closed: pending step is dropped ("no").
    pub fn lapse(&self, out: &mpsc::UnboundedSender<MindEvent>);
    pub fn cancel(&self);     // stops a request in flight; keeps turns
    pub fn reset(&self);      // forgets everything
    pub fn pending(&self) -> bool;
}
pub const IDLE_FORGET: Duration = Duration::from_secs(600);
pub const FILLER_AFTER: Duration = Duration::from_millis(1500);
```

Turn order in `hear`:
1. If idle longer than `IDLE_FORGET`, `reset`.
2. `quick(heard, pending options)`:
   - `Stop`/`Cancel`: drop pending, cancel a request in flight (and `setup_cancel`), `Cleared`, say nothing for Stop, "Okay." for Cancel, `SayEnd{Rest}`.
   - `Repeat`: say the last reply again (and re-`Ask` if a question is pending).
   - Pending `Confirm`: `Yes` runs `carry_out`, says the sentence, `SayEnd{FollowUp}`; `No` says "Okay, I won't." Anything that is not a quick Yes/No **drops the confirmation** (`Cleared`) and continues at step 3 with the new words (**Review Focus 5**).
   - Pending `Ask`: `Pick(i)`, or `Yes`/`No` when the options are Yes/No, resolves it by `AskFor` (`Dot`: re-plan the wish with the chosen dot's handle, confirm tier still applies; `AnswerCheck`: yes sends, no "Okay, not sent."; `Model`: "The person chose: <option>" is the next heard words for the model). Words that are not a pick go to the model, with the question in the turns, and the pending ask is dropped.
   - Pending setup question: the words are the answer to the current question; when the round's answers are complete call `setup_ask` again.
   - `Yes`/`No` with nothing pending: "There's nothing waiting on a yes or no.", `SayEnd{Rest}`.
3. `cap_hit`: if some, say it, `SayEnd{Rest}`, return.
4. `Thinking`; `Picture::take`; `prompt`; `thinker.think` with `settings.open_dot_model`; deltas go through `SpeechGate` to `Say`; a timer emits `Filler` once if nothing was said by `FILLER_AFTER`. Record usage when the answer carries it. On `OnceFailure`: `NoCli` "I can't think right now: the Claude CLI isn't set up. Open Settings to fix it."; `TimedOut` "That took too long. Try again."; `Failed` "Something went wrong reaching Claude."; `Cancelled` says nothing. Always `SayEnd{Rest}` and no pending state (**Review Focus 4**).
5. `read_reply`; `plan` the wish; then by `Step`: `Say` adds the words; `Do` runs `carry_out` and says its sentence; `Confirm` emits `Card::Confirm` and `Ask{question, ["Yes","No"]}`, `SayEnd{Ask}`; `Ask` emits `Ask`, `SayEnd{Ask}`; `Open` emits `Open` and its words; `Create` starts the setup flow. `Wish::Needs` says `needs_report`, emits `Card::List`, and when private approvals wait emits `Open{Inbox}`. A `bad_block` with empty words says "I didn't catch what to do with that. Say it another way?".
6. Setup flow: say "Let me set that up." then `setup_ask(id, SetupRequest{description, ..})`; `Questions` are asked one at a time (`Ask{question, []}`, `SayEnd{Ask}`), answers collected into a `SetupRound`; at most two rounds (`must_propose` then forces a proposal); `Proposal` emits `Card::Proposal` and asks "<name>: <one line from schedule and first message>. Create it?" with Yes/No; yes calls `setup_apply(proposal, org, workspace)` from `View` (`personal` to `None`; `default` workspace as Task 3 found), says "Created <name>." (plus "Its first run could not start: ..." when `run_error`), emits `Open{Dot}`. No: "Okay, not created." Setup failure: its error text as the sentence, nothing pending.
7. Turn list: push `Turn{heard, said}` (said = all words spoken this turn, cut to 400 bytes); keep the last 12.
- `Then`: a plain answer or a completed deed ends `FollowUp`; "Okay." and failures end `Rest`.
- A second `hear` or `pick` while one is running cancels the first (its remaining events are not sent) and then runs.
- Model words are never used to build a `Confirm` question or to pass one. The state lock is never held across an await on the thinker or the runtime.

- [ ] Tests with a scripted `Thinker` fake (unit, in `mod.rs`, runtime from a temp `Config`): `stop_and_cancel_clear_pending`, `repeat_says_the_last_reply`, `model_cannot_pass_a_confirmation` (**Review Focus 1**: thinker returns `Approving now.\nDO {"act":"approve","a":"a1","confirmed":true}`; assert the approval is still pending, an `Ask` with Yes/No came out, `Card::Confirm` carries the real record; a following model reply whose words are "Yes." decides nothing; only `hear("yes")` or `pick(0)` decides), `anything_but_yes_drops_the_confirmation`, `lapse_drops_the_confirmation`, `pick_by_click_equals_saying_it`, `which_dot_then_still_confirms_a_run`, `filler_once_when_slow`, `no_filler_when_fast` (tokio paused time), `cli_missing_says_so_and_leaves_nothing_pending`, `cap_reached_skips_the_model_but_quick_answers_work`, `idle_forgets_turns`, `second_hear_cancels_the_first`, `usage_recorded_per_request`, `block_text_is_never_in_a_say_event`.
- [ ] Runtime tests (`open_dot_test.rs`, fake CLI; a helper rewrites the script file between turns): `what_needs_me_reads_the_inbox`, `approve_by_voice_end_to_end`, `set_up_a_dot_by_voice_end_to_end` (description, one round of two questions, proposal, yes; the dot exists and the first message was sent), `set_up_declined_creates_nothing`, `ask_a_dot_sends_and_says_to_whom`.
- [ ] Commit `feat(core): Open Dot's mind: one turn in, words and one checked action out`.

### Task 7: Gate

- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings` (build `ui/dist` first: `bun install --frozen-lockfile && bun run build` in `ui`); `cargo test -p dots-core --locked -- --skip preview_schedule`; `cargo test -p dots-app --locked`; the `ui` test suite. `tests/resume_test.rs` is known to flake under load: re-run it alone before calling it a failure.
- [ ] Grep the new module for logging of words: no `heard`, `said`, reply text or picture text in any `tracing` call.

### Task 8: Kept process (measure, then build only if it holds)

**Files:** scratch measurement outside the repo first; then `crates/dots-core/src/open_dot/think.rs`, `crates/dots-core/src/bin/fake-claude.rs`.

- [ ] Measure on the real CLI (model `haiku`): start `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --model haiku --tools "" --strict-mcp-config --setting-sources "" --no-session-persistence --system-prompt <s>`, send 10 user messages one after another over stdin, record time to first text delta for each, whether each turn ends with its own `result` line, and what happens when stdin is held open idle for 10 minutes and when the process is killed mid-turn. Compare with 10 fresh processes. Keep prompts free of anything private. Write the numbers into the spec's "As built".
- [ ] Build `KeptThinker` only if: every turn produced a `result`, later turns reach first text at least 0.7 s sooner than fresh, and a kill is detected and recovered by a fresh process within the same turn. Otherwise record "kept process not built" with the numbers and stop.
- [ ] If built: `KeptThinker` implements `Thinker` (the system prompt is fixed at start; the picture and heard words go in each user message; earlier turns are not resent; restarted when the model setting changes, after `IDLE_FORGET`, on `reset`, or on any failure, falling back to `CliThinker` for that turn). `fake-claude` gains a mode (`FAKE_CLAUDE_KEEP=1`): read stdin line by line and run the script once per line. Tests: `kept_process_serves_three_turns`, `dead_process_falls_back_within_the_turn`, `reset_ends_the_process`, `usage_recorded_per_turn`.
- [ ] Commit `feat(core): Open Dot keeps one CLI process for a conversation` or `docs(spec): kept process measured and not built`.

### Task 9: As built

- [ ] Add "### Step 2: the mind" under section 10 of the spec: what was built, the decisions above, anything that differs from sections 3.5 and 5.2 with the reason, the kept-process numbers, deferred minors, what is owed. Update the status line. Commit `docs(spec): Open Dot step 2 as built`.
