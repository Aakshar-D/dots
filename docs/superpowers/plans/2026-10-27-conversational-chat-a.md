# Conversational Chat, Plan A (answers, recent runs, instruction proposals) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dot's chat becomes a conversation. A message the person types is answered rather than taken as a run of the dot's instructions, the chat knows the dot's recent runs, and when asked to change how it works the dot proposes new instructions that the person approves or rejects on a card.

**Architecture:**
- **Prompt:** `Runtime::chat` marks the person's messages (`"typed": true` in the run's payload). `prompt::build_prompt_full` frames such a thread's first prompt as a conversation, lists the dot's recent runs (`Store::ended_runs_outside`), and names "Runs since your last message" on a resume.
- **Proposals:** a new module `dots_core::instructions` holds them. Its tool `propose_instructions` is served by Dots' own MCP server (Claude engine) and built into the local engine. It is offered only to a run of a topic the person started, under the new policy name `Instructions`, which every preset allows.
- **Deciding:** a proposal is stored in `instruction_proposals` (migration 0018) and replaces the dot's waiting one. `Runtime::decide_instruction_proposal` saves approved instructions through `update_dot`, refusing ones the person edited since, and continues the chat as a decided app proposal does.
- **UI:** the Inbox lists the waiting proposals. The Tauri app gains two commands, and the UI gains a card with a line diff, shown in the conversation and the Inbox.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio; Tauri 2.12; Bun 1.3, React 19.3, TypeScript 6.0, zod 4.6, TanStack Query, Vitest 5.0, Testing Library. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-27-conversational-chat-design.md` (commit `e1145d2`, the base of this plan, branch `feat/conversational-chat`, from `origin/master` `bd953cc`). Task 9 adds its section 9, "As built (plan A)", which the table "Deviations" lists.

**Plans after this one:** B, the conversation's Claude-app-like layout (spec section 5). It restyles the files Task 8 touches (`TopicRuns`, `Timeline`, `DotPane`) and the card.

**Where the tasks come from:** spec section 3 (Tasks 1, 5), section 3.1 (Task 2), section 4 (Tasks 3 to 8), section 7 (each task's tests).

**How this plan's code was checked:** the code blocks of Tasks 1 to 9 were generated from a throwaway worktree of `e1145d2`, one commit per task, where they compiled and passed on Windows 11. The plan was then replayed onto `e1145d2` by a script that applied every block and compared the tree with each task commit. All 9 tasks matched; the contract fixture was taken from the task commit, as the commands write it.

| | Before the first task | At the last task |
|---|---|---|
| `cargo test -p dots-core` | 1745 passed | 1771 |
| `cargo test -p dots-app` | 124 | 124 |
| `bun run --cwd ui test` | 748 tests in 71 files | 759 in 73 |

At the last task, clippy added no warning (one in `watchers/github.rs` was there before), and `cargo fmt --check` and `tsc --noEmit` were clean. No agent ran the real `claude` CLI: the Claude engine's tests run `fake-claude`, and the local engine's run a scripted endpoint.

## Global Constraints

- Toolchain: Rust 1.95 stable; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash). Windows 11, Git Bash.
- Branch `feat/conversational-chat`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\conversational-chat`, based on `e1145d2`. All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Every cargo command runs with `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target-chat CARGO_INCREMENTAL=0`, set once per shell. This machine runs out of memory on a full parallel test run: use `cargo test -j 2 ... -- --test-threads=2` for the full `dots-core` run, and check free space on C: (`df -h /c`) before it.
- One new migration, `0018_instructions.sql` (Task 3): the table `instruction_proposals`, and `upgrade_0018` for `store/upgrade.rs`. No new dependency, Tauri capability or plugin.
- The subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials or calls an Anthropic API. No agent runs the real `claude` CLI (not even with a prompt); tests use `fake-claude`.
- Values the spec and this plan fix:
  - **Payload and framing:** a person's message carries the key `typed: true`. The framing texts are `prompt::CONVERSATION`, `prompt::CHAT_SUMMARY_RULE` and `prompt::NO_INSTRUCTIONS_TOOL`.
  - **Recent runs:** the newest 5, each cut to 300 characters with `…`, under `## Your recent runs` and `## Runs since your last message`.
  - **The tool:** named `propose_instructions` (`mcp__dots__propose_instructions` on the Claude engine). Its policy name is `Instructions`, and every preset ends with an allow rule for it. Instructions are 1 to 20,000 characters and a reason 1 to 500.
  - **Statuses and texts:** `pending`/`approved`/`rejected`/`replaced`. What the dot is told is `instructions::{SENT, NOT_HERE, UNCHANGED, APPROVED, REJECTED}`, and the refusals are `DECIDED_ALREADY` and `STALE`.
- A proposal changes nothing until the person approves it; only a run of a topic the person started may propose; approving never overwrites instructions the person edited since.
- No agent installs or runs an installer, runs a release build, or touches the installed Dots (`%LOCALAPPDATA%\Dots`) or the real data folder `%LOCALAPPDATA%\dots`.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Before each commit, check with:
  - `git diff --cached --check`
  - `git ls-files --eol <files>`
  - `iconv -f UTF-8 -t UTF-8 <file> > /dev/null`

  The blocks hold `·` and `…`. The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- Live data is parsed, not cast, with strict schemas: a type the UI parses changes only together with the contract fixture and `ui/src/api/types.ts`, in the same commit (Tasks 3, 4, 6, 7). The fixture `ui/src/api/__fixtures__/contract.json` is written by `UPDATE_CONTRACT=1 cargo test -p dots-app contract` and never by hand.
- Before each commit, run the task's own tests. Then, as the change requires:
  - `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`)
  - `cargo clippy -p dots-core -p dots-app --tests` (no new warnings)
  - `cargo fmt --check`
  - `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
  - `bun run --cwd ui typecheck`

  `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first. The UI files have no formatter; keep new lines within 100 columns where the file does.
- Known on this machine:
  - A `vitest` run started right after a cargo build can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`; run it again on its own.
  - Known flaky under load: before treating one of these as a failure, rerun its test binary alone.
    - `memory_server_test::dots_own_server_serves_the_memory_tools_for_the_runs_dot`: its "never started" run is dispatched and ends, so the next RPC is refused; already so at `e1145d2`, about one run in three.
    - `browser_host_test::closing_ends_it_over_its_pipe_and_a_closed_window_starts_again`: a 10 s wait for a fake Edge.
- Fact-Forcing Gate (a hook of this machine): these are blocked until the facts it asks for are stated, after which you retry the identical call:
  - the first Edit or Write of each file
  - the first Bash command after a task notification
  - `rm`, `git commit --amend`, or a rebase

  For a large new file, write a one-line stub first.

## Deviations from the spec

The spec at `e1145d2` leaves these open; Task 9 records them in a new section 9.

| Spec at `e1145d2` | This plan | Why |
|---|---|---|
| Section 3: "a person's chat message" | The payload key `typed: true`, set by `Runtime::chat`; no other writer sets it | The runs Dots makes are chat runs too, and must keep the task framing |
| Section 3: a reply to a run topic may need a framing line | None: it resumes the run's session, with "Runs since your last message" when runs ended | The session already holds the run; a framing line would restate it |
| Section 3.1: "the newest 5 ended runs outside this topic" | Succeeded, failed or cancelled, other chat topics included, latest end first; "since" means ended after the parent run was queued | The runs a person asks about include other chats |
| Section 4: a proposal and its card | One waits per dot, and a newer one replaces it (`replaced`); 20,000 and 500 characters; a proposal equal to the instructions is refused; proposals go with their dot | Two waiting cards would race each other's base |
| Section 4: approving | Refused while the topic still works (as a reply is) and when the instructions changed since (`STALE`, the card waits) | The decision continues the chat; an edit in the editor must not be lost |
| Section 4: the card | A line diff of the two texts (`lib/line-diff.ts`) rather than `DiffView`; only waiting proposals are shown; the Inbox group "New instructions" counts in the Inbox, not in the dots' needs-you counts | `DiffView` renders git patches |

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. **An answer Dots writes for the person.** The answer to a run's question, or a decision on an app or instruction proposal, gets the conversation framing. Expected: the task framing stays, so a dot that asked mid-task finishes the task. Tests: `engine_prompt_test::chat_runs_dots_makes_keep_the_task_framing` (Task 1) and `runtime_instructions_test::approving_saves_the_instructions_and_continues_the_chat` (the decision's run is not typed, Task 6).
2. **A run outside the person's chat tries to propose instructions.** This covers a scheduled run, a webhook, a workflow step, or a policy that denies `Instructions`. Expected: the tool is neither offered nor named, and a call answers `NOT_HERE`. Tests:
   - `instructions_test::only_a_topic_the_person_started_may_propose` (Task 4)
   - `instructions_tools_test::a_local_dot_proposes_from_the_person_s_chat_only` and `a_deny_rule_turns_the_tool_off_and_the_chat_is_told_to_send_the_person_to_the_editor` (Task 5)
3. **The person approves after editing the instructions in the editor.** Expected: refused with `STALE`, the edit kept, the card still waiting and rejectable. Tests: `runtime_instructions_test::instructions_edited_since_the_proposal_are_not_overwritten` (Task 6) and `Instructions.test.tsx` "shows a refused decision as the core words it and keeps the card" (Task 8).
4. **The dot proposes twice before the person decides, or while other writes go on.** Expected: the newer proposal replaces the waiting one, only it has a card, and the store never answers "database is locked". Tests:
   - `instructions_test::a_person_s_chat_proposes_and_a_newer_proposal_replaces_the_waiting_one` (Task 4, whose transaction writes first)
   - `instructions_tools_test::a_local_dot_proposes_from_the_person_s_chat_only` (Task 5, which failed one run in two with a read-first transaction)
   - `Instructions.test.tsx` "shows the waiting one in the conversation" (Task 8)
5. **Recent runs with long, multi-line or empty summaries, and failed runs.** Expected: one line each, cut to 300 characters with `…`, the error when there is no summary. Tests: `engine_prompt_test::recent_runs_are_one_line_each` and `store_runs_test::ended_runs_outside_a_topic_latest_end_first` (Task 2).

## File Map

- `crates/dots-core/src/prompt.rs` (Tasks 1, 2, 5), `src/runtime.rs` (Tasks 1, 6), `src/runner.rs` (Tasks 2, 5), `src/store/runs.rs` (Task 2).
- `crates/dots-core/src/instructions.rs` (new, Task 3; Tasks 4 to 6), `src/store/instructions.rs` (new, Task 4), `migrations/0018_instructions.sql` (new, Task 3).
- `crates/dots-core/src/{lib,policy,events,checkins,model,run_tools}.rs`; `src/store/{mod,upgrade}.rs`; `src/server/mcp.rs`; `src/engine/claude/args.rs`; `src/engine/local/{mod,mcp_tools}.rs`, `src/engine/local/tools/{mod,instructions}.rs` (`instructions.rs` new, Task 5).
- `apps/dots-app/src/{commands,lib,contract}.rs`; `ui/src/api/{types,commands}.ts`, the fixture, `ui/src/api/contract.test.ts`; `ui/src/lib/{query,runtime-events,line-diff}.ts`; `ui/src/components/InstructionsCard.tsx` (new); `ui/src/screens/home/{TopicRuns,Timeline,DotPane}.tsx`; `ui/src/screens/InboxScreen.tsx`.
- Tests: `crates/dots-core/tests/{instructions_policy,instructions,instructions_tools,runtime_instructions}_test.rs` (new); `ui/src/lib/line-diff.test.ts` and `ui/src/screens/home/Instructions.test.tsx` (new). Existing tests whose expectations the new rule, tool and Inbox field change.
- `docs/superpowers/specs/2026-10-27-conversational-chat-design.md` (Task 9).

### Task 1: A person's chat message is a conversation

Spec section 3: the first prompt of a thread the person started with a message is framed as a conversation, and only work ends with a summary. Chat runs that Dots makes keep the task framing.

**Files:**

- Modify: `crates/dots-core/src/prompt.rs`, `crates/dots-core/src/runtime.rs`
- Test: `crates/dots-core/tests/engine_prompt_test.rs`, `crates/dots-core/tests/runtime_test.rs`

**Interfaces:**

- Produces: `dots_core::prompt::{TYPED = "typed", CONVERSATION, CHAT_SUMMARY_RULE, is_typed(&Run) -> bool}`; `Runtime::chat` stores `{"message": m, "typed": true}`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
use dots_core::model::{ApplyState, EngineKind, TriggerKind, WorkspaceMode};
use dots_core::prompt::{build_prompt, context_tool_name};
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::model::{ApplyState, EngineKind, TriggerKind, WorkspaceMode};
use dots_core::prompt::{
    build_prompt, context_tool_name, is_typed, CHAT_SUMMARY_RULE, CONVERSATION,
};
use serde_json::{json, Value};
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    assert_eq!(build_prompt(&dot, &resume, ws, at(), "", &[]), "Go on.");
}
```

**with:**

```rust
    assert_eq!(build_prompt(&dot, &resume, ws, at(), "", &[]), "Go on.");
}

#[test]
fn a_person_s_first_chat_message_is_a_conversation() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "why did it skip the coffee?", "typed": true})),
        None,
    );
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]);
    assert!(p.starts_with("Do the thing."));
    let talk = p.find(CONVERSATION).expect("the conversation section");
    let message = p
        .find("## Message from the user\n\nwhy did it skip the coffee?")
        .expect("the message");
    assert!(talk < message);
    assert!(p.contains(CHAT_SUMMARY_RULE));
    assert!(!p.contains("- End with a short summary"));
    assert!(p.contains("do not retry it"));
}

#[test]
fn chat_runs_dots_makes_keep_the_task_framing() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    for payload in [
        json!({"message": "You asked: \"Which?\"\nThe person answered: the red one", "answer": "the red one"}),
        json!({"message": "The person added the app notes to the Library."}),
        json!({"message": "hi", "typed": false}),
    ] {
        let run = fake_run(TriggerKind::Chat, Some(payload), None);
        let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]);
        assert!(!p.contains("## Chat with the person"), "{p}");
        assert!(p.contains(
            "- End with a short summary: what you did, what changed, and what needs human review."
        ));
    }
}

#[test]
fn a_person_s_resumed_chat_message_is_only_the_message() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "and now?", "typed": true})),
        Some("s"),
    );
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]),
        "and now?"
    );
}

#[test]
fn only_a_chat_run_can_be_typed() {
    let typed = Some(json!({"message": "go", "typed": true}));
    assert!(is_typed(&fake_run(TriggerKind::Chat, typed.clone(), None)));
    assert!(!is_typed(&fake_run(TriggerKind::Webhook, typed, None)));
    assert!(!is_typed(&fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "go"})),
        None
    )));
}
```

**In `crates/dots-core/tests/runtime_test.rs`, replace:**

```rust
    assert!(rec["prompt"].as_str().unwrap().contains("build_failed"));
    t.rt.shutdown();
}

```

**with:**

```rust
    assert!(rec["prompt"].as_str().unwrap().contains("build_failed"));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_person_s_chat_message_is_marked_typed() {
    let t = start(None, None).await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "typed"), None)
            .await
            .unwrap();
    let run = t.rt.chat(&dot.id, "why?", None).await.unwrap();
    assert_eq!(run.payload, Some(json!({"message": "why?", "typed": true})));
    assert!(dots_core::prompt::is_typed(&run));
}

```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test engine_prompt_test --test runtime_test`
Expected: FAIL to compile: no `is_typed`, `CHAT_SUMMARY_RULE` or `CONVERSATION` in `dots_core::prompt`.

- [ ] **Step 3: Frame the person's message**

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
use crate::policy::Action;

```

**with:**

```rust
use crate::policy::Action;

/// The key `Runtime::chat` sets in a chat run's payload: the person typed the message in a chat
/// (conversational chat spec section 3). Chat runs Dots makes itself carry no such key.
pub const TYPED: &str = "typed";

/// What a person's chat message's first prompt tells the dot (conversational chat spec section
/// 3): a chat is a conversation, not a run of its instructions.
pub const CONVERSATION: &str = "## Chat with the person\n\n\
    This is a conversation with the person who set you up, not a run of the instructions above.\n\n\
    - Answer questions directly, also ones that have nothing to do with your instructions.\n\
    - Run the process your instructions describe only when the person asks you to run it.\n\
    - When asked to change something in your work, change only that, without redoing the rest.\n\
    - Files a past run wrote are in that run's own copy, not in your workspace. To change them, \
    tell the person to use Reply on that run's card.\n";

/// The standing rule on summaries of a person's chat: only work ends with one.
pub const CHAT_SUMMARY_RULE: &str = "- When you ran your instructions or changed files, end with \
    a short summary: what you did, what changed, and what needs human review. Otherwise your \
    answer is your reply.\n";

/// The run carries a message the person typed in a chat, not one Dots wrote for them.
pub fn is_typed(run: &Run) -> bool {
    run.trigger == TriggerKind::Chat
        && run
            .payload
            .as_ref()
            .and_then(|p| p.get(TYPED))
            .and_then(Value::as_bool)
            == Some(true)
}

```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    out.push_str("\n\n---\n\n");
    if run.trigger == TriggerKind::Chat {
```

**with:**

```rust
    out.push_str("\n\n---\n\n");
    let typed = is_typed(run);
    if typed {
        out.push_str(CONVERSATION);
        out.push('\n');
    }
    if run.trigger == TriggerKind::Chat {
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    );
    out.push_str(
        "- End with a short summary: what you did, what changed, and what needs human review.\n",
    );
    // A dot with a browser is told how to use it (browser spec section 4).
```

**with:**

```rust
    );
    if typed {
        out.push_str(CHAT_SUMMARY_RULE);
    } else {
        out.push_str(
            "- End with a short summary: what you did, what changed, and what needs human review.\n",
        );
    }
    // A dot with a browser is told how to use it (browser spec section 4).
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let mut new = NewRun::new(dot_id, TriggerKind::Chat);
        new.payload = Some(json!({ "message": message }));
        // Held until the new run is stored: of two follow-ups sent at once, the second must
```

**with:**

```rust
        let mut new = NewRun::new(dot_id, TriggerKind::Chat);
        // The person typed it (conversational chat spec section 3).
        new.payload = Some(json!({ "message": message, crate::prompt::TYPED: true }));
        // Held until the new run is stored: of two follow-ups sent at once, the second must
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test engine_prompt_test --test runtime_test`
Expected: PASS: `engine_prompt_test` 19, `runtime_test` 18.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2`, `cargo clippy -p dots-core --tests`, `cargo fmt --check`.
Expected: `dots-core` 1750 passed; no new warning.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/prompt.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/runtime_test.rs
git commit -m "feat(core): a person's chat message is a conversation, not a run of the instructions"
```

### Task 2: The chat names the dot's recent runs

Spec section 3.1: a person's first message lists the dot's newest 5 ended runs outside the topic. A resumed message names the runs that ended since the topic's last run was queued, and only when there are some.

**Files:**

- Modify: `crates/dots-core/src/prompt.rs`, `crates/dots-core/src/runner.rs`, `crates/dots-core/src/store/runs.rs`
- Test: `crates/dots-core/tests/engine_prompt_test.rs`, `crates/dots-core/tests/runtime_test.rs`, `crates/dots-core/tests/store_runs_test.rs`

**Interfaces:**

- Consumes: Task 1's `prompt::is_typed`, `CONVERSATION`.
- Produces:
  - `Store::ended_runs_outside(dot_id, root_run_id, ended_after: Option<&str>, limit: i64) -> Result<Vec<Run>>`
  - `prompt::{PromptParts { contexts, memories, apps, recent: &[Run] }, RECENT_RUNS = 5, RECENT_TEXT = 300, recent_runs(&[Run]) -> String}`
  - the private `Runner::recent_runs(&Run) -> Vec<Run>`

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
use dots_core::model::{ApplyState, EngineKind, TriggerKind, WorkspaceMode};
use dots_core::prompt::{
    build_prompt, context_tool_name, is_typed, CHAT_SUMMARY_RULE, CONVERSATION,
};
```

**with:**

```rust
use dots_core::model::{ApplyState, EngineKind, TriggerKind, WorkspaceMode};
use dots_core::model::{Run, RunStatus};
use dots_core::prompt::{
    build_prompt, build_prompt_full, context_tool_name, is_typed, recent_runs, PromptParts,
    CHAT_SUMMARY_RULE, CONVERSATION, RECENT_TEXT,
};
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
}

#[test]
fn only_a_chat_run_can_be_typed() {
```

**with:**

```rust
}

fn ended(
    trigger: TriggerKind,
    status: RunStatus,
    summary: Option<&str>,
    error: Option<&str>,
) -> Run {
    let mut run = fake_run(trigger, None, None);
    run.status = status;
    run.summary = summary.map(str::to_string);
    run.error = error.map(str::to_string);
    run.ended_at = Some("2026-10-27T08:00:00.000Z".into());
    run
}

fn local(t: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(t)
        .unwrap()
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

#[test]
fn recent_runs_are_one_line_each() {
    let when = local("2026-10-27T08:00:00.000Z");
    let long = "word ".repeat(100);
    let runs = [
        ended(
            TriggerKind::Schedule,
            RunStatus::Succeeded,
            Some("Ordered milk,\n\n  eggs and bread."),
            None,
        ),
        ended(
            TriggerKind::Webhook,
            RunStatus::Failed,
            Some("  "),
            Some("timed out"),
        ),
        ended(TriggerKind::Manual, RunStatus::Cancelled, None, None),
        ended(
            TriggerKind::Watcher,
            RunStatus::Succeeded,
            Some(&long),
            None,
        ),
    ];
    let lines: Vec<String> = recent_runs(&runs).lines().map(str::to_string).collect();
    assert_eq!(
        lines[0],
        format!("- Scheduled · {when} · done: Ordered milk, eggs and bread.")
    );
    assert_eq!(lines[1], format!("- Webhook · {when} · failed: timed out"));
    assert_eq!(lines[2], format!("- Run now · {when} · cancelled"));
    let cut = lines[3].split_once(": ").unwrap().1;
    assert_eq!(cut.chars().count(), RECENT_TEXT + 1);
    assert!(cut.ends_with('…'));
    assert_eq!(lines.len(), 4);
}

#[test]
fn a_person_s_first_chat_message_lists_the_recent_runs_before_the_conversation() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let recent = [ended(
        TriggerKind::Schedule,
        RunStatus::Succeeded,
        Some("Sent 3 emails"),
        None,
    )];
    let parts = PromptParts {
        recent: &recent,
        ..PromptParts::default()
    };
    let typed = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "why?", "typed": true})),
        None,
    );
    let p = build_prompt_full(&dot, &typed, Path::new("C:/ws"), at(), "", parts);
    let list = p
        .find("## Your recent runs\n\n- Scheduled · ")
        .expect("the list");
    assert!(p.contains(" · done: Sent 3 emails\n\n---\n\n"));
    assert!(p.find("Do the thing.").unwrap() < list);
    assert!(list < p.find(CONVERSATION).unwrap());
    // Without runs, no section; a chat run Dots made never gets one.
    let none = build_prompt_full(
        &dot,
        &typed,
        Path::new("C:/ws"),
        at(),
        "",
        PromptParts::default(),
    );
    assert!(!none.contains("## Your recent runs"));
    let made = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "The person added the app x."})),
        None,
    );
    let made = build_prompt_full(&dot, &made, Path::new("C:/ws"), at(), "", parts);
    assert!(!made.contains("## Your recent runs"));
}

#[test]
fn a_person_s_resumed_message_names_the_runs_since_before_it() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let recent = [ended(
        TriggerKind::Schedule,
        RunStatus::Succeeded,
        Some("Sent 3 emails"),
        None,
    )];
    let parts = PromptParts {
        recent: &recent,
        ..PromptParts::default()
    };
    let typed = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "anything new?", "typed": true})),
        Some("s"),
    );
    assert_eq!(
        build_prompt_full(&dot, &typed, Path::new("C:/ws"), at(), "", parts),
        format!(
            "## Runs since your last message\n\n{}\n## Message from the user\n\nanything new?",
            recent_runs(&recent)
        )
    );
    // A resume Dots made stays only its message.
    let made = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "Approved."})),
        Some("s"),
    );
    assert_eq!(
        build_prompt_full(&dot, &made, Path::new("C:/ws"), at(), "", parts),
        "Approved."
    );
}

#[test]
fn only_a_chat_run_can_be_typed() {
```

**In `crates/dots-core/tests/runtime_test.rs`, replace:**

```rust
    assert!(dots_core::prompt::is_typed(&run));
}
```

**with:**

```rust
    assert!(dots_core::prompt::is_typed(&run));
}

#[tokio::test]
async fn a_person_s_chat_names_the_dot_s_runs_outside_the_topic() {
    let t = start(None, None).await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "recent"), None)
            .await
            .unwrap();
    let prompt = || -> String {
        let rec: Value =
            serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
        rec["prompt"].as_str().unwrap().to_string()
    };
    let manual = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &manual.id, RunStatus::Succeeded, 15).await;

    let first = t.rt.chat(&dot.id, "what did you do?", None).await.unwrap();
    let first = wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 15).await;
    let p = prompt();
    let listed = p
        .find("## Your recent runs\n\n- Run now · ")
        .expect("the list");
    assert!(p[listed..].contains(" · done: fake done\n"));
    assert!(listed < p.find("## Chat with the person").unwrap());

    // Nothing ended since: the resume is only the message.
    let second = t.rt.chat(&dot.id, "and?", Some(&first.id)).await.unwrap();
    let second = wait_status(t.rt.store(), &second.id, RunStatus::Succeeded, 15).await;
    assert_eq!(prompt(), "and?");

    // A run that ended since the topic's last run is named before the message.
    let again = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &again.id, RunStatus::Succeeded, 15).await;
    let third =
        t.rt.chat(&dot.id, "anything new?", Some(&second.id))
            .await
            .unwrap();
    wait_status(t.rt.store(), &third.id, RunStatus::Succeeded, 15).await;
    let p = prompt();
    assert!(p.starts_with("## Runs since your last message\n\n- Run now · "));
    assert_eq!(p.matches("- Run now").count(), 1, "{p}");
    assert!(p.ends_with("\n## Message from the user\n\nanything new?"));
}
```

**In `crates/dots-core/tests/store_runs_test.rs`, replace:**

```rust
    assert_eq!(store.list_runs(None, 1).await.unwrap().len(), 1);
}
```

**with:**

```rust
    assert_eq!(store.list_runs(None, 1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn ended_runs_outside_a_topic_latest_end_first() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    let other = store.create_dot(&spec("b"), None).await.unwrap();
    let ended = |trigger, ended_at: &'static str, status| {
        let store = store.clone();
        let dot_id = dot.id.clone();
        async move {
            let run = store
                .create_run(&NewRun::new(&dot_id, trigger))
                .await
                .unwrap();
            store
                .finish_run(&run.id, status, Some("s"), None)
                .await
                .unwrap();
            store
                .set_run_times(&run.id, "2026-10-27T07:00:00.000Z", Some(ended_at))
                .await
                .unwrap();
            run
        }
    };
    let topic = ended(
        TriggerKind::Chat,
        "2026-10-27T09:00:00.000Z",
        RunStatus::Succeeded,
    )
    .await;
    let mut reply = NewRun::new(&dot.id, TriggerKind::Chat);
    reply.parent_run_id = Some(topic.id.clone());
    reply.root_run_id = Some(topic.id.clone());
    store.create_run(&reply).await.unwrap();
    let morning = ended(
        TriggerKind::Schedule,
        "2026-10-27T08:00:00.000Z",
        RunStatus::Succeeded,
    )
    .await;
    let failed = ended(
        TriggerKind::Webhook,
        "2026-10-27T08:30:00.000Z",
        RunStatus::Failed,
    )
    .await;
    let later = ended(
        TriggerKind::Manual,
        "2026-10-27T10:00:00.000Z",
        RunStatus::Cancelled,
    )
    .await;
    // Neither a run that has not ended nor another dot's run is named.
    store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let theirs = store
        .create_run(&NewRun::new(&other.id, TriggerKind::Manual))
        .await
        .unwrap();
    store
        .finish_run(&theirs.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();

    let ids = |runs: Vec<Run>| runs.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(
        ids(store
            .ended_runs_outside(&dot.id, &topic.id, None, 5)
            .await
            .unwrap()),
        vec![later.id.clone(), failed.id.clone(), morning.id.clone()]
    );
    assert_eq!(
        ids(store
            .ended_runs_outside(&dot.id, &topic.id, None, 2)
            .await
            .unwrap()),
        vec![later.id.clone(), failed.id.clone()]
    );
    assert_eq!(
        ids(store
            .ended_runs_outside(&dot.id, &topic.id, Some("2026-10-27T08:30:00.000Z"), 5)
            .await
            .unwrap()),
        vec![later.id]
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test engine_prompt_test --test runtime_test --test store_runs_test`
Expected: FAIL to compile: no `recent_runs`, `PromptParts` or `RECENT_TEXT` in `dots_core::prompt`, and no method `ended_runs_outside` on `Store`.

- [ ] **Step 3: List the runs**

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
/// What a run's prompt names beyond the dot and the run: its context collections, the dot's
/// memories and the run's apps.
#[derive(Default, Clone, Copy)]
```

**with:**

```rust
/// What a run's prompt names beyond the dot and the run: its context collections, the dot's
/// memories, the run's apps and, for a person's chat message, the dot's runs outside its topic
/// (`recent_runs`).
#[derive(Default, Clone, Copy)]
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    pub apps: Option<&'a RunApps>,
}
```

**with:**

```rust
    pub apps: Option<&'a RunApps>,
    pub recent: &'a [Run],
}

/// The most runs a person's chat message names (conversational chat spec section 3.1).
pub const RECENT_RUNS: i64 = 5;
/// The most characters of a run's summary or error it names.
pub const RECENT_TEXT: usize = 300;

/// What the dot's recent runs list calls a trigger.
fn trigger_label(trigger: TriggerKind) -> &'static str {
    match trigger {
        TriggerKind::Schedule => "Scheduled",
        TriggerKind::Webhook => "Webhook",
        TriggerKind::Manual => "Run now",
        TriggerKind::Chat => "Chat",
        TriggerKind::Resume => "Continued after approval",
        TriggerKind::FollowUp => "Follow-up",
        TriggerKind::App => "Started by an app",
        TriggerKind::Workflow => "Workflow step",
        TriggerKind::Watcher => "Watcher",
    }
}

/// One line per run of `runs` (conversational chat spec section 3.1): trigger, local end time,
/// status and the first `RECENT_TEXT` characters of its summary, else of its error, on one
/// line.
pub fn recent_runs(runs: &[Run]) -> String {
    let mut out = String::new();
    for run in runs {
        let when = run
            .ended_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let status = match run.status {
            crate::model::RunStatus::Succeeded => "done",
            crate::model::RunStatus::Failed => "failed",
            crate::model::RunStatus::Cancelled => "cancelled",
            _ => "not ended",
        };
        out.push_str(&format!(
            "- {} · {when} · {status}",
            trigger_label(run.trigger)
        ));
        let text = run
            .summary
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(run.error.as_deref())
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        if !text.is_empty() {
            let mut cut: String = text.chars().take(RECENT_TEXT).collect();
            if cut.len() < text.len() {
                cut.push('…');
            }
            out.push_str(": ");
            out.push_str(&cut);
        }
        out.push('\n');
    }
    out
}
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        memories,
        apps: None,
    };
```

**with:**

```rust
        memories,
        ..PromptParts::default()
    };
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        apps,
    } = parts;
```

**with:**

```rust
        apps,
        recent,
    } = parts;
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        .unwrap_or_default();
    match run.trigger {
```

**with:**

```rust
        .unwrap_or_default();
    // A person's message that resumes its topic names the runs that ended since the topic's
    // last one (conversational chat spec section 3.1).
    if is_typed(run) && run.session_id.is_some() && !recent.is_empty() {
        return format!(
            "## Runs since your last message\n\n{}\n## Message from the user\n\n{message}",
            recent_runs(recent)
        );
    }
    match run.trigger {
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    }
    out.push_str("\n\n---\n\n");
    let typed = is_typed(run);
    if typed {
```

**with:**

```rust
    }
    let typed = is_typed(run);
    if typed && !recent.is_empty() {
        out.push_str("\n\n## Your recent runs\n\n");
        out.push_str(recent_runs(recent).trim_end());
    }
    out.push_str("\n\n---\n\n");
    if typed {
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        cur.trigger == TriggerKind::Chat
    }
```

**with:**

```rust
        cur.trigger == TriggerKind::Chat
    }

    /// The dot's runs a person's chat message names (conversational chat spec section 3.1): for
    /// the first message of a topic, the newest ended runs outside it; for one that resumes the
    /// topic, those that ended after its last run was queued. Nothing for any other run. A list
    /// that cannot be read is logged and left empty; the chat goes on without it.
    async fn recent_runs(&self, run: &Run) -> Vec<Run> {
        if !crate::prompt::is_typed(run) {
            return Vec::new();
        }
        let since = match (&run.session_id, &run.parent_run_id) {
            (None, _) => None,
            (Some(_), Some(parent)) => match self.store.get_run(parent).await {
                Ok(parent) => Some(parent.queued_at),
                Err(e) => {
                    tracing::warn!(run = %run.id, "reading the chat's last run failed: {e}");
                    return Vec::new();
                }
            },
            (Some(_), None) => return Vec::new(),
        };
        match self
            .store
            .ended_runs_outside(
                &run.dot_id,
                &run.root_run_id,
                since.as_deref(),
                crate::prompt::RECENT_RUNS,
            )
            .await
        {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(run = %run.id, "listing the dot's recent runs failed: {e}");
                Vec::new()
            }
        }
    }
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        let memories = self.store.list_memories(&dot.id).await?;
        let secret = self.register_secret(&run.id);
```

**with:**

```rust
        let memories = self.store.list_memories(&dot.id).await?;
        let recent = self.recent_runs(run).await;
        let secret = self.register_secret(&run.id);
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
                apps: tools.apps().map(|apps| apps.as_ref()),
            },
```

**with:**

```rust
                apps: tools.apps().map(|apps| apps.as_ref()),
                recent: &recent,
            },
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
        ))
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
```

**with:**

```rust
        ))
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// The `limit` newest ended runs of `dot_id` outside the lineage `root_run_id`, latest end
    /// first; with `ended_after`, only those that ended after it (conversational chat spec
    /// section 3.1).
    pub async fn ended_runs_outside(
        &self,
        dot_id: &str,
        root_run_id: &str,
        ended_after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? AND root_run_id <> ? \
             AND status IN ('succeeded', 'failed', 'cancelled') AND ended_at IS NOT NULL \
             AND (? IS NULL OR ended_at > ?) \
             ORDER BY ended_at DESC, id DESC LIMIT ?"
        ))
        .bind(dot_id)
        .bind(root_run_id)
        .bind(ended_after)
        .bind(ended_after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test engine_prompt_test --test runtime_test --test store_runs_test`
Expected: PASS: `engine_prompt_test` 22, `runtime_test` 19, `store_runs_test` 28.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2`, `cargo clippy -p dots-core --tests`, `cargo fmt --check`.
Expected: `dots-core` 1755 passed; no new warning.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/prompt.rs crates/dots-core/src/runner.rs crates/dots-core/src/store/runs.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/runtime_test.rs crates/dots-core/tests/store_runs_test.rs
git commit -m "feat(core): a person's chat names the dot's recent runs and the runs since its last message"
```

### Task 3: The `Instructions` policy rule, and migration 0018

Spec section 4, "Policy": every preset allows `Instructions`, and existing dots get the rule (Custom too) through `finish_0018`. The migration also creates the proposals' table that Task 4 uses.

**Files:**

- Create: `crates/dots-core/migrations/0018_instructions.sql`, `crates/dots-core/src/instructions.rs`, `crates/dots-core/tests/instructions_policy_test.rs`
- Modify: `crates/dots-core/src/lib.rs`, `crates/dots-core/src/policy.rs`, `crates/dots-core/src/store/mod.rs`, `crates/dots-core/src/store/upgrade.rs`, `ui/src/api/__fixtures__/contract.json` (regenerated)
- Test: `crates/dots-core/tests/{memory_policy,policy_checkins,store_dots,store_library,workflow_tools}_test.rs`. Their pinned rule tails and stored policies gain `Instructions`, last.

**Interfaces:**

- Produces: `dots_core::instructions::POLICY_NAME = "Instructions"`; `dots_core::policy::{INSTRUCTIONS_TOOL, INSTRUCTIONS_RULE}`; `Policy::preset` ends with `Rule::new("Instructions", Action::Allow)`; the table `instruction_proposals` (columns `id, dot_id, run_id, root_run_id, base, instructions, reason, status, answer_run_id, created_at, decided_at`, one `pending` row per dot).

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/instructions_policy_test.rs`:**

```rust
//! The policy name `Instructions` (conversational chat spec section 4).

mod common;

use dots_core::policy::{Action, Policy, Preset, Rule, INSTRUCTIONS_RULE, INSTRUCTIONS_TOOL};
use serde_json::{json, Value};

#[test]
fn every_preset_allows_proposing_instructions_last() {
    for preset in [
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::ReadOnly,
        Preset::Custom,
    ] {
        let p = Policy::preset(preset);
        assert_eq!(
            p.resolve(INSTRUCTIONS_TOOL, &json!({})),
            Action::Allow,
            "{preset:?}"
        );
        assert_eq!(
            p.rules.last(),
            Some(&Rule::new("Instructions", Action::Allow))
        );
    }
}

#[test]
fn an_ask_or_deny_rule_wins_and_a_rule_names_no_argument() {
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.push(Rule::new("Instructions", Action::Ask));
    assert_eq!(p.resolve("Instructions", &json!({})), Action::Ask);
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.insert(0, Rule::new("Instructions", Action::Deny));
    assert_eq!(p.resolve("Instructions", &json!({})), Action::Deny);
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules
        .push(Rule::new("Instructions(anything)", Action::Allow));
    assert!(p
        .validate()
        .unwrap_err()
        .to_string()
        .contains(INSTRUCTIONS_RULE));
}

#[test]
fn the_cli_never_sees_the_instructions_rule() {
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.push(Rule::new("Instructions", Action::Deny));
    let (allow, deny) = p.cli_lists();
    assert!(!allow
        .iter()
        .chain(&deny)
        .any(|t| t.starts_with("Instructions")));
}

/// Makes `dot_id` look stored before migration 0018: its policy without an `Instructions` rule
/// (or with `keep`), and listed in `upgrade_0018`.
async fn as_before_0018(db: &std::path::Path, dot_id: &str, keep: Option<Value>) {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db.display()))
        .await
        .unwrap();
    let stored: String = sqlx::query_scalar("SELECT policy FROM dots WHERE id = ?")
        .bind(dot_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut policy: Value = serde_json::from_str(&stored).unwrap();
    let rules = policy["rules"].as_array_mut().unwrap();
    rules.retain(|r| r["tool"] != "Instructions");
    if let Some(rule) = keep {
        rules.insert(0, rule);
    }
    sqlx::query("UPDATE dots SET policy = ? WHERE id = ?")
        .bind(policy.to_string())
        .bind(dot_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS upgrade_0018 (dot_id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO upgrade_0018 VALUES (?)")
        .bind(dot_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn stored_dots_get_the_instructions_rule_once_custom_too() {
    let (dir, store) = common::temp_store().await;
    let plain = store.create_dot(&common::spec("old"), None).await.unwrap();
    let mut custom = common::spec("custom");
    custom.policy = Policy::preset(Preset::Custom);
    let custom = store.create_dot(&custom, None).await.unwrap();
    let denied = store
        .create_dot(&common::spec("denied"), None)
        .await
        .unwrap();
    drop(store);
    let db = dir.path().join("dots.db");
    as_before_0018(&db, &plain.id, None).await;
    as_before_0018(&db, &custom.id, None).await;
    as_before_0018(
        &db,
        &denied.id,
        Some(json!({"tool": "Instructions", "action": "deny"})),
    )
    .await;
    let store = dots_core::store::Store::open(&db).await.unwrap();
    let rules = |id: String| {
        let store = store.clone();
        async move {
            store
                .get_dot(&id)
                .await
                .unwrap()
                .spec
                .policy
                .rules
                .into_iter()
                .filter(|r| r.tool == "Instructions")
                .map(|r| r.action)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(rules(plain.id.clone()).await, vec![Action::Allow]);
    assert_eq!(rules(custom.id.clone()).await, vec![Action::Allow]);
    assert_eq!(rules(denied.id.clone()).await, vec![Action::Deny]);
    drop(store);
    let store = dots_core::store::Store::open(&db).await.unwrap();
    let again = store.get_dot(&plain.id).await.unwrap().spec.policy.rules;
    assert_eq!(again.iter().filter(|r| r.tool == "Instructions").count(), 1);
}
```

**In `crates/dots-core/tests/memory_policy_test.rs`, replace:**

```rust
        // `Workflow` follows it (teams spec section 7), then `App` and `ProposeApp` (app library
        // spec section 6).
        let tail: Vec<&str> = p.rules[p.rules.len() - 7..]
            .iter()
```

**with:**

```rust
        // `Workflow` follows it (teams spec section 7), then `App` and `ProposeApp` (app library
        // spec section 6), then `Instructions` (conversational chat spec section 4).
        let tail: Vec<&str> = p.rules[p.rules.len() - 8..]
            .iter()
```

**In `crates/dots-core/tests/memory_policy_test.rs`, replace:**

```rust
                "App",
                "ProposeApp"
            ],
```

**with:**

```rust
                "App",
                "ProposeApp",
                "Instructions"
            ],
```

**In `crates/dots-core/tests/memory_policy_test.rs`, replace:**

```rust
        p.rules.retain(|r| {
            r.tool != "Memory" && r.tool != "Workflow" && !APP_TOOLS.contains(&r.tool.as_str())
        });
```

**with:**

```rust
        p.rules.retain(|r| {
            r.tool != "Memory"
                && r.tool != "Workflow"
                && !APP_TOOLS.contains(&r.tool.as_str())
                && r.tool != "Instructions"
        });
```

**In `crates/dots-core/tests/memory_policy_test.rs`, replace:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, kept);
```

**with:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    kept.rules.push(Rule::new("Instructions", Action::Allow));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, kept);
```

**In `crates/dots-core/tests/memory_policy_test.rs`, replace:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom_then);
```

**with:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    custom_then
        .rules
        .push(Rule::new("Instructions", Action::Allow));
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom_then);
```

**In `crates/dots-core/tests/policy_checkins_test.rs`, replace:**

```rust
        // `Memory` follows them (dot memory spec section 3), then `Workflow` (teams spec section
        // 7), then `App` and `ProposeApp` (app library spec section 6).
        let tail: Vec<&str> = p.rules[p.rules.len() - 9..]
            .iter()
```

**with:**

```rust
        // `Memory` follows them (dot memory spec section 3), then `Workflow` (teams spec section
        // 7), then `App` and `ProposeApp` (app library spec section 6), then `Instructions`.
        let tail: Vec<&str> = p.rules[p.rules.len() - 10..]
            .iter()
```

**In `crates/dots-core/tests/policy_checkins_test.rs`, replace:**

```rust
                "App",
                "ProposeApp"
            ],
```

**with:**

```rust
                "App",
                "ProposeApp",
                "Instructions"
            ],
```

**In `crates/dots-core/tests/policy_checkins_test.rs`, replace:**

```rust
                && !APP_TOOLS.contains(&tool)
        });
```

**with:**

```rust
                && !APP_TOOLS.contains(&tool)
                && tool != "Instructions"
        });
```

**In `crates/dots-core/tests/policy_checkins_test.rs`, replace:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, want);
```

**with:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    want.rules.push(Rule::new("Instructions", Action::Allow));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, want);
```

**In `crates/dots-core/tests/policy_checkins_test.rs`, replace:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom);
```

**with:**

```rust
        .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
    custom.rules.push(Rule::new("Instructions", Action::Allow));
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom);
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
                && !dots_core::policy::APP_TOOLS.contains(&r.tool.as_str())
        });
```

**with:**

```rust
                && !dots_core::policy::APP_TOOLS.contains(&r.tool.as_str())
                && r.tool != "Instructions"
        });
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    );
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, denies);
```

**with:**

```rust
    );
    denies.rules.push(dots_core::policy::Rule::new(
        "Instructions",
        dots_core::policy::Action::Allow,
    ));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, denies);
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
    assert_eq!(d1.policy.mcp_default, Action::Ask);
```

**with:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
    assert_eq!(d1.policy.mcp_default, Action::Ask);
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
    let d4 = dot("D4").await;
```

**with:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
    let d4 = dot("D4").await;
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Workflow", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
    assert_eq!(d4.policy.mcp_default, Action::Ask);
```

**with:**

```rust
            Rule::new("Workflow", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
    assert_eq!(d4.policy.mcp_default, Action::Ask);
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
    assert_eq!(d6.uses, Uses::default());
```

**with:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
    assert_eq!(d6.uses, Uses::default());
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
}
```

**with:**

```rust
            Rule::new("Context", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
}
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
        Rule::new("ProposeApp", Action::Allow),
    ];
```

**with:**

```rust
        Rule::new("ProposeApp", Action::Allow),
        Rule::new("Instructions", Action::Allow),
    ];
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Workflow", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
    assert_eq!(
```

**with:**

```rust
            Rule::new("Workflow", Action::Allow),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
    assert_eq!(
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            context.clone(),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
        ]
    );
    assert_eq!(
```

**with:**

```rust
            context.clone(),
            Rule::new("App", Action::Allow),
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
    );
    assert_eq!(
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("ProposeApp", Action::Allow),
        ]
```

**with:**

```rust
            Rule::new("ProposeApp", Action::Allow),
            Rule::new("Instructions", Action::Allow),
        ]
```

**In `crates/dots-core/tests/workflow_tools_test.rs`, replace:**

```rust
        // The app rules come after it (app library spec section 6).
        let tail: Vec<&str> = p.rules[p.rules.len() - 4..]
            .iter()
```

**with:**

```rust
        // The app rules come after it (app library spec section 6).
        let tail: Vec<&str> = p.rules[p.rules.len() - 5..]
            .iter()
```

**In `crates/dots-core/tests/workflow_tools_test.rs`, replace:**

```rust
            tail,
            ["Memory", "Workflow", "App", "ProposeApp"],
            "{preset:?}"
```

**with:**

```rust
            tail,
            ["Memory", "Workflow", "App", "ProposeApp", "Instructions"],
            "{preset:?}"
```

**In `crates/dots-core/tests/workflow_tools_test.rs`, replace:**

```rust
        let mut p = Policy::preset(preset);
        p.rules
            .retain(|r| r.tool != "Workflow" && !APP_TOOLS.contains(&r.tool.as_str()));
        p
```

**with:**

```rust
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| {
            r.tool != "Workflow"
                && !APP_TOOLS.contains(&r.tool.as_str())
                && r.tool != "Instructions"
        });
        p
```

**In `crates/dots-core/tests/workflow_tools_test.rs`, replace:**

```rust
            .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        p
```

**with:**

```rust
            .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        p.rules.push(Rule::new("Instructions", Action::Allow));
        p
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test instructions_policy_test`
Expected: FAIL to compile: no `INSTRUCTIONS_RULE` or `INSTRUCTIONS_TOOL` in `dots_core::policy`.

Run: `cargo test -p dots-core --no-fail-fast --test memory_policy_test --test policy_checkins_test --test workflow_tools_test --test store_dots_test --test store_library_test`
Expected: FAIL: 10 tests, the presets' tails (`every_preset_allows_memory_after_the_check_in_tools`, `every_preset_allows_the_check_in_tools_after_skill_and_context`, `workflow_follows_memory_asked_by_sandboxed_and_custom_and_the_cli_never_sees_it`) and the migrations' stored policies (`migration_0006_*` 3, `0007`, `0008`, `0012`, `0015`).

- [ ] **Step 3: Add the rule and the migration**

**Create `crates/dots-core/migrations/0018_instructions.sql`:**

```sql
-- Instruction proposals (conversational chat spec section 4): from a person's chat, a dot
-- proposes new instructions for itself; the person approves or rejects them on a card. `base`
-- is the dot's instructions when it proposed, so an approval after the person edited them is
-- refused. One proposal waits per dot: a newer one replaces it.
CREATE TABLE instruction_proposals (
    id            TEXT PRIMARY KEY,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT NOT NULL,
    root_run_id   TEXT NOT NULL,
    base          TEXT NOT NULL,
    instructions  TEXT NOT NULL,
    reason        TEXT NOT NULL,
    status        TEXT NOT NULL,
    answer_run_id TEXT,
    created_at    TEXT NOT NULL,
    decided_at    TEXT
);
CREATE INDEX instruction_proposals_dot ON instruction_proposals(dot_id, created_at);
CREATE UNIQUE INDEX instruction_proposals_pending_dot
    ON instruction_proposals(dot_id) WHERE status = 'pending';

-- The dots whose policy gets the `Instructions` rule its preset has, Custom too
-- (`upgrade::finish_0018`).
CREATE TABLE upgrade_0018 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0018 (dot_id) SELECT id FROM dots;
```

**Create `crates/dots-core/src/instructions.rs`:**

```rust
//! A dot proposes new instructions for itself (conversational chat spec section 4): from a
//! person's chat it calls `propose_instructions`, Dots keeps the proposal and posts a card, and
//! the person's answer continues the chat, as a decided app proposal does.

/// The one name the policy, approvals and grants know `propose_instructions` by. Every preset
/// allows it: nothing changes until the person approves the card.
pub const POLICY_NAME: &str = "Instructions";
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod followups;
pub mod kit;
```

**with:**

```rust
pub mod followups;
pub mod instructions;
pub mod kit;
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust

/// A tool whose ask rule wins over an allow rule: the check-in tools, `Memory`, `Workflow` and
/// the app tools.
fn ask_wins(tool: &str) -> bool {
```

**with:**

```rust

/// Dots' own `propose_instructions` by its policy name (conversational chat spec section 4):
/// treated as `Memory` is. A rule names it alone: `Instructions(...)` is refused.
pub const INSTRUCTIONS_TOOL: &str = crate::instructions::POLICY_NAME;

/// The refusal of a rule `Instructions(...)`.
pub const INSTRUCTIONS_RULE: &str = "an Instructions rule names no argument";

/// A tool whose ask rule wins over an allow rule: the check-in tools, `Memory`, `Workflow`,
/// the app tools and `Instructions`.
fn ask_wins(tool: &str) -> bool {
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        || APP_TOOLS.contains(&tool)
}
```

**with:**

```rust
        || APP_TOOLS.contains(&tool)
        || tool == INSTRUCTIONS_TOOL
}
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        rules.extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        Policy {
```

**with:**

```rust
        rules.extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        // And propose new instructions for itself from a chat: the card is the proposal's gate
        // (conversational chat spec section 4).
        rules.push(Rule::new(INSTRUCTIONS_TOOL, Action::Allow));
        Policy {
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
                WORKFLOW_TOOL,
            ]
```

**with:**

```rust
                WORKFLOW_TOOL,
                INSTRUCTIONS_TOOL,
            ]
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
                    return Err(invalid(WORKFLOW_RULE));
                }
```

**with:**

```rust
                    return Err(invalid(WORKFLOW_RULE));
                }
                if tool == INSTRUCTIONS_TOOL {
                    return Err(invalid(INSTRUCTIONS_RULE));
                }
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
        upgrade::finish_0016(&store.pool).await?;
        Ok(store)
```

**with:**

```rust
        upgrade::finish_0016(&store.pool).await?;
        upgrade::finish_0018(&store.pool).await?;
        Ok(store)
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
//! whose URL reads a variable reaches now. 0016 (app library spec section 6): `finish_0016` does
//! what `finish_0012` does for `App` and `ProposeApp` with `upgrade_0016`, Custom policies too.

```

**with:**

```rust
//! whose URL reads a variable reaches now. 0016 (app library spec section 6): `finish_0016` does
//! what `finish_0012` does for `App` and `ProposeApp` with `upgrade_0016`, Custom policies too,
//! and 0018 (conversational chat spec section 4) `finish_0018` for `Instructions` with
//! `upgrade_0018`.

```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust

/// Gives the policy of each dot listed in `upgrade_<migration>` the rule its preset has for each
```

**with:**

```rust

/// Gives the policy of each dot that migration 0018 listed the `Instructions` allow rule, a
/// Custom policy included, as 0016 did for the app tools (conversational chat spec section 4),
/// and drops the list.
pub(super) async fn finish_0018(pool: &SqlitePool) -> Result<()> {
    give_preset_rules(pool, "0018", &[crate::policy::INSTRUCTIONS_TOOL], true).await
}

/// Gives the policy of each dot listed in `upgrade_<migration>` the rule its preset has for each
```

Then regenerate the contract fixture (the presets' policies are in its dot samples):

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`
Expected: PASS; `ui/src/api/__fixtures__/contract.json` gains an `Instructions` rule in each sample policy.

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test instructions_policy_test --test memory_policy_test --test policy_checkins_test --test workflow_tools_test --test store_dots_test --test store_library_test`
Expected: PASS: 4, 3, 4, 11, 16 and 17 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2`, `cargo test -p dots-app`, `cargo clippy -p dots-core -p dots-app --tests`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: `dots-core` 1759 passed, `dots-app` 124, the UI 748; no new warning.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/migrations/0018_instructions.sql crates/dots-core/src/instructions.rs crates/dots-core/src/lib.rs crates/dots-core/src/policy.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/upgrade.rs crates/dots-core/tests/instructions_policy_test.rs crates/dots-core/tests/memory_policy_test.rs crates/dots-core/tests/policy_checkins_test.rs crates/dots-core/tests/store_dots_test.rs crates/dots-core/tests/store_library_test.rs crates/dots-core/tests/workflow_tools_test.rs ui/src/api/__fixtures__/contract.json
git commit -m "feat(core): the Instructions policy rule every preset allows, and migration 0018 with the proposals table"
```

### Task 4: Instruction proposals, stored and checked

Spec section 4, "A proposal, not an approval": `Proposals::propose` checks the text, refuses a run outside the person's chat, keeps the proposal in place of the dot's waiting one and sends `instruction_proposal_changed`.

**Files:**

- Create: `crates/dots-core/src/store/instructions.rs`, `crates/dots-core/tests/instructions_test.rs`
- Modify: `crates/dots-core/src/instructions.rs`, `crates/dots-core/src/store/mod.rs`, `crates/dots-core/src/events.rs`, `crates/dots-core/src/checkins.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts`, `ui/src/api/__fixtures__/contract.json` (regenerated)

**Interfaces:**

- Consumes: Task 1's `prompt::is_typed`; Task 3's table.
- Produces:
  - In `dots_core::instructions`:
    - constants `TOOL = "propose_instructions"`, `SENT`, `NOT_HERE`, `UNCHANGED`, `DECIDED_ALREADY`, `STALE`, `APPROVED`, `REJECTED`, `MAX_INSTRUCTIONS = 20_000`, `MAX_REASON = 500`, `DESCRIPTION`, `REQUIRED`
    - `properties()` and `mcp_schema()`
    - `InstructionStatus { Pending, Approved, Rejected, Replaced }`
    - `InstructionProposal { id, dot_id, run_id, root_run_id, base, instructions, reason, status, answer_run_id, created_at, decided_at }`
    - `may_propose(&Store, &Run) -> Result<bool>`
    - `Proposals::{new(Store, Bus), propose(run_id, instructions, reason) -> Result<String, String>, changed(id, dot_id)}`
  - `Store::{add_instruction_proposal(&p) -> Result<Vec<String>> (the ids it replaced), get_instruction_proposal, list_instruction_proposals(Option<&str>), decide_instruction_proposal(id, status, &NewRun) -> Result<(InstructionProposal, Run)>}`
  - `RuntimeEvent::InstructionProposalChanged { id, dot_id }`
  - `CheckIns::instructions() -> &Proposals`

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/instructions_test.rs`:**

```rust
//! Instructions a dot proposes for itself (conversational chat spec section 4).

mod common;

use common::{spec, temp_store};
use dots_core::events::RuntimeEvent;
use dots_core::instructions::{
    may_propose, InstructionStatus, Proposals, DECIDED_ALREADY, MAX_INSTRUCTIONS, MAX_REASON,
    NOT_HERE, SENT, UNCHANGED,
};
use dots_core::model::{NewRun, Run, RunStatus, TriggerKind};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;
use tokio::sync::broadcast;

async fn chat(store: &Store, dot_id: &str, typed: bool) -> Run {
    let mut new = NewRun::new(dot_id, TriggerKind::Chat);
    new.payload = Some(if typed {
        json!({"message": "from now on, buy the closest brand", "typed": true})
    } else {
        json!({"message": "The person added the app notes to the Library."})
    });
    store.create_run(&new).await.unwrap()
}

fn proposals(store: &Store) -> (Proposals, broadcast::Receiver<RuntimeEvent>) {
    let (bus, rx) = broadcast::channel(16);
    (Proposals::new(store.clone(), bus), rx)
}

fn changed(rx: &mut broadcast::Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::InstructionProposalChanged { id, .. } = ev {
            ids.push(id);
        }
    }
    ids
}

#[tokio::test]
async fn a_person_s_chat_proposes_and_a_newer_proposal_replaces_the_waiting_one() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let (p, mut rx) = proposals(&store);
    let next = "Do the thing.\nIf an item is out of stock, buy the closest brand.";
    assert_eq!(
        p.propose(&run.id, &format!("  {next}\n"), " Buy the closest brand ")
            .await
            .unwrap(),
        SENT
    );
    let listed = store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let first = &listed[0];
    assert_eq!(first.base, "Do the thing.");
    assert_eq!(first.instructions, next);
    assert_eq!(first.reason, "Buy the closest brand");
    assert_eq!(first.status, InstructionStatus::Pending);
    assert_eq!(first.run_id, run.id);
    assert_eq!(first.root_run_id, run.id);
    assert_eq!(changed(&mut rx), vec![first.id.clone()]);
    // The person did not decide yet and the dot proposes again: the newer one waits instead.
    p.propose(&run.id, "Do the thing twice.", "Twice")
        .await
        .unwrap();
    let listed = store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].instructions, "Do the thing twice.");
    assert_eq!(listed[0].status, InstructionStatus::Pending);
    assert_eq!(listed[1].id, first.id);
    assert_eq!(listed[1].status, InstructionStatus::Replaced);
    assert!(listed[1].decided_at.is_some());
    assert_eq!(
        changed(&mut rx),
        vec![first.id.clone(), listed[0].id.clone()]
    );
    assert_eq!(
        store.list_instruction_proposals(None).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn only_a_topic_the_person_started_may_propose() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let (p, _rx) = proposals(&store);
    let scheduled = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let made = chat(&store, &dot.id, false).await;
    for run in [&scheduled, &made] {
        assert!(!may_propose(&store, run).await.unwrap());
        assert_eq!(
            p.propose(&run.id, "Other.", "Why").await.unwrap_err(),
            NOT_HERE
        );
    }
    // A run that continues the person's topic may, a workflow step's run may not.
    let typed = chat(&store, &dot.id, true).await;
    let mut resume = NewRun::new(&dot.id, TriggerKind::Resume);
    resume.parent_run_id = Some(typed.id.clone());
    resume.root_run_id = Some(typed.id.clone());
    let resume = store.create_run(&resume).await.unwrap();
    assert!(may_propose(&store, &resume).await.unwrap());
    let mut step = typed.clone();
    step.workflow_step_run_id = Some("STEP".into());
    assert!(!may_propose(&store, &step).await.unwrap());
    assert!(store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_text_and_the_reason_are_checked() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let (p, _rx) = proposals(&store);
    let long = "x".repeat(MAX_INSTRUCTIONS + 1);
    let reason = "r".repeat(MAX_REASON + 1);
    for (instructions, why, err) in [
        (
            "  ",
            "Why",
            "the instructions must be 1 to 20000 characters",
        ),
        (
            long.as_str(),
            "Why",
            "the instructions must be 1 to 20000 characters",
        ),
        ("Other.", " ", "the reason must be 1 to 500 characters"),
        (
            "Other.",
            reason.as_str(),
            "the reason must be 1 to 500 characters",
        ),
        (" Do the thing. ", "Why", UNCHANGED),
    ] {
        assert_eq!(
            p.propose(&run.id, instructions, why).await.unwrap_err(),
            err
        );
    }
    assert!(store
        .list_instruction_proposals(None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_proposal_is_decided_once_and_goes_with_its_dot() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let (p, _rx) = proposals(&store);
    p.propose(&run.id, "Other.", "Why").await.unwrap();
    let id = store.list_instruction_proposals(None).await.unwrap()[0]
        .id
        .clone();
    let next = NewRun::new(&dot.id, TriggerKind::Chat);
    let (decided, answer) = store
        .decide_instruction_proposal(&id, InstructionStatus::Approved, &next)
        .await
        .unwrap();
    assert_eq!(decided.status, InstructionStatus::Approved);
    assert_eq!(decided.answer_run_id.as_deref(), Some(answer.id.as_str()));
    assert!(decided.decided_at.is_some());
    let again = store
        .decide_instruction_proposal(&id, InstructionStatus::Rejected, &next)
        .await
        .unwrap_err();
    assert!(matches!(again, Error::Conflict(m) if m == DECIDED_ALREADY));
    assert!(matches!(
        store
            .decide_instruction_proposal("nope", InstructionStatus::Rejected, &next)
            .await
            .unwrap_err(),
        Error::NotFound(_)
    ));
    // The refused decisions stored no run.
    let runs = store.list_runs(Some(&dot.id), 10).await.unwrap();
    assert_eq!(runs.len(), 2);
    for r in &runs {
        store
            .finish_run(&r.id, RunStatus::Succeeded, None, None)
            .await
            .unwrap();
    }
    store.delete_dot_if_idle(&dot.id).await.unwrap();
    assert!(store
        .list_instruction_proposals(None)
        .await
        .unwrap()
        .is_empty());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test instructions_test`
Expected: FAIL to compile: no `may_propose`, `InstructionStatus`, `Proposals` or the constants in `dots_core::instructions`.

- [ ] **Step 3: Store and check the proposals**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            }),
            to(RuntimeEvent::WatcherUpdated {
```

**with:**

```rust
            }),
            to(RuntimeEvent::InstructionProposalChanged {
                id: "01J0INSTRUCTIONS".into(),
                dot_id: "01J0DOT".into(),
            }),
            to(RuntimeEvent::WatcherUpdated {
```

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
    memories: Memories,
    /// Weak: the workflow runs hold the runner, which holds these.
```

**with:**

```rust
    memories: Memories,
    /// The instructions dots propose (conversational chat spec section 4).
    instructions: crate::instructions::Proposals,
    /// Weak: the workflow runs hold the runner, which holds these.
```

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
            memories: Memories::new(store.clone(), bus.clone()),
            store,
```

**with:**

```rust
            memories: Memories::new(store.clone(), bus.clone()),
            instructions: crate::instructions::Proposals::new(store.clone(), bus.clone()),
            store,
```

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
        &self.memories
    }
```

**with:**

```rust
        &self.memories
    }

    /// The instructions dots propose: `propose_instructions` goes through them.
    pub fn instructions(&self) -> &crate::instructions::Proposals {
        &self.instructions
    }
```

**In `crates/dots-core/src/events.rs`, replace:**

```rust
    },
    /// A team, its members or its projects were made, changed or deleted (teams spec
```

**with:**

```rust
    },
    /// A dot proposed instructions, or a proposal was decided or replaced (conversational chat
    /// spec section 4): the card reads it again.
    InstructionProposalChanged {
        id: String,
        dot_id: String,
    },
    /// A team, its members or its projects were made, changed or deleted (teams spec
```

**In `crates/dots-core/src/instructions.rs`, replace:**

```rust

/// The one name the policy, approvals and grants know `propose_instructions` by. Every preset
```

**with:**

```rust

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::{Bus, RuntimeEvent};
use crate::model::Run;
use crate::store::Store;
use crate::{Error, Result};

/// The one name the policy, approvals and grants know `propose_instructions` by. Every preset
```

**In `crates/dots-core/src/instructions.rs`, replace:**

```rust
pub const POLICY_NAME: &str = "Instructions";
```

**with:**

```rust
pub const POLICY_NAME: &str = "Instructions";
/// The tool's name on both engines (`mcp__dots__propose_instructions` on the Claude engine).
pub const TOOL: &str = "propose_instructions";
/// What the dot is told when the card is posted.
pub const SENT: &str = "Sent to the person for review. Finish your turn; their answer arrives \
    as your next message.";
/// What a run outside a person's chat is told.
pub const NOT_HERE: &str = "propose_instructions works only in a chat the person started";
/// What a proposal that changes nothing is told.
pub const UNCHANGED: &str = "these are your instructions as they are: send them with the change \
    made";
/// What a decision on a decided proposal is told.
pub const DECIDED_ALREADY: &str = "these instructions have already been decided";
/// What approving a proposal is told when the person edited the instructions since.
pub const STALE: &str = "the dot's instructions changed after it proposed these; ask it to \
    propose them again";
/// What the dot reads when the person approves or rejects.
pub const APPROVED: &str = "The person approved your new instructions; your next runs use them.";
pub const REJECTED: &str = "The person rejected your new instructions; they stay as they were.";
/// The most characters of proposed instructions and of a reason.
pub const MAX_INSTRUCTIONS: usize = 20_000;
pub const MAX_REASON: usize = 500;

pub const DESCRIPTION: &str = "Propose new instructions for yourself, when the person asks you \
    to change how you work from now on. Send the whole new text (your instructions with the \
    change made) and a one-line reason for the person. They approve or reject it on a card; \
    nothing changes until they approve.";

/// The tool's arguments.
pub fn properties() -> Value {
    json!({
        "instructions": {
            "type": "string",
            "description": "Your whole new instructions, with the change made"
        },
        "reason": { "type": "string", "description": "What changes and why, in one line" }
    })
}

pub const REQUIRED: [&str; 2] = ["instructions", "reason"];

/// The tool as Dots' own `dots` MCP server lists it.
pub fn mcp_schema() -> Value {
    json!({
        "name": TOOL,
        "description": DESCRIPTION,
        "inputSchema": { "type": "object", "properties": properties(), "required": REQUIRED }
    })
}

str_enum!(InstructionStatus {
    Pending => "pending",
    Approved => "approved",
    Rejected => "rejected",
    // A newer proposal of the dot took its place.
    Replaced => "replaced",
});

/// Instructions a dot proposed, and what the person decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstructionProposal {
    pub id: String,
    pub dot_id: String,
    pub run_id: String,
    pub root_run_id: String,
    /// The dot's instructions when it proposed: the card's "before".
    pub base: String,
    pub instructions: String,
    pub reason: String,
    pub status: InstructionStatus,
    /// The run the decision started.
    pub answer_run_id: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// The run may propose instructions: a run of a topic the person started with a message they
/// typed, not a workflow step's.
pub async fn may_propose(store: &Store, run: &Run) -> Result<bool> {
    if run.workflow_step_run_id.is_some() {
        return Ok(false);
    }
    let root = if run.root_run_id == run.id {
        run.clone()
    } else {
        store.get_run(&run.root_run_id).await?
    };
    Ok(crate::prompt::is_typed(&root))
}

/// The proposals, for both engines' `propose_instructions`. Cheap to clone.
#[derive(Clone)]
pub struct Proposals {
    store: Store,
    bus: Bus,
}

impl Proposals {
    pub fn new(store: Store, bus: Bus) -> Self {
        Self { store, bus }
    }

    /// `propose_instructions` for the run `run_id`: checks the text, keeps the proposal in
    /// place of the dot's waiting one and posts the card. `Ok` is what the dot is told; `Err`
    /// the problem, in a sentence for the dot.
    pub async fn propose(
        &self,
        run_id: &str,
        instructions: &str,
        reason: &str,
    ) -> std::result::Result<String, String> {
        let said = |e: Error| match e {
            Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
            other => other.to_string(),
        };
        let instructions = instructions.trim();
        let reason = reason.trim();
        if instructions.is_empty() || instructions.chars().count() > MAX_INSTRUCTIONS {
            return Err(format!(
                "the instructions must be 1 to {MAX_INSTRUCTIONS} characters"
            ));
        }
        if reason.is_empty() || reason.chars().count() > MAX_REASON {
            return Err(format!("the reason must be 1 to {MAX_REASON} characters"));
        }
        let run = self.store.get_run(run_id).await.map_err(said)?;
        if !may_propose(&self.store, &run).await.map_err(said)? {
            return Err(NOT_HERE.to_string());
        }
        let dot = self.store.get_dot(&run.dot_id).await.map_err(said)?;
        let base = dot.spec.instructions.trim().to_string();
        if base == instructions {
            return Err(UNCHANGED.to_string());
        }
        let proposal = InstructionProposal {
            id: crate::util::new_id(),
            dot_id: run.dot_id.clone(),
            run_id: run.id.clone(),
            root_run_id: run.root_run_id.clone(),
            base,
            instructions: instructions.to_string(),
            reason: reason.to_string(),
            status: InstructionStatus::Pending,
            answer_run_id: None,
            created_at: crate::util::now(),
            decided_at: None,
        };
        let replaced = self
            .store
            .add_instruction_proposal(&proposal)
            .await
            .map_err(said)?;
        for id in replaced {
            self.changed(&id, &proposal.dot_id);
        }
        self.changed(&proposal.id, &proposal.dot_id);
        Ok(SENT.to_string())
    }

    pub(crate) fn changed(&self, id: &str, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::InstructionProposalChanged {
            id: id.to_string(),
            dot_id: dot_id.to_string(),
        });
    }
}
```

**Create `crates/dots-core/src/store/instructions.rs`:**

```rust
//! The instructions dots propose for themselves (conversational chat spec section 4).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::instructions::{InstructionProposal, InstructionStatus, DECIDED_ALREADY};
use crate::model::{NewRun, Run};
use crate::{Error, Result};

const COLUMNS: &str = "id, dot_id, run_id, root_run_id, base, instructions, reason, status, \
    answer_run_id, created_at, decided_at";

fn proposal_from_row(row: &SqliteRow) -> Result<InstructionProposal> {
    Ok(InstructionProposal {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        run_id: row.try_get("run_id")?,
        root_run_id: row.try_get("root_run_id")?,
        base: row.try_get("base")?,
        instructions: row.try_get("instructions")?,
        reason: row.try_get("reason")?,
        status: InstructionStatus::parse(&row.try_get::<String, _>("status")?)?,
        answer_run_id: row.try_get("answer_run_id")?,
        created_at: row.try_get("created_at")?,
        decided_at: row.try_get("decided_at")?,
    })
}

impl Store {
    /// Keeps the proposal `p` in place of the dot's waiting one, which becomes `replaced`, in
    /// one transaction. Returns the ids it replaced.
    pub async fn add_instruction_proposal(&self, p: &InstructionProposal) -> Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;
        // A write first: a transaction that read first could not take the write lock while
        // another connection writes, and would fail as "database is locked".
        let replaced: Vec<String> = sqlx::query_scalar(
            "UPDATE instruction_proposals SET status = 'replaced', decided_at = ? \
             WHERE dot_id = ? AND status = 'pending' RETURNING id",
        )
        .bind(crate::util::now())
        .bind(&p.dot_id)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "INSERT INTO instruction_proposals ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&p.id)
        .bind(&p.dot_id)
        .bind(&p.run_id)
        .bind(&p.root_run_id)
        .bind(&p.base)
        .bind(&p.instructions)
        .bind(&p.reason)
        .bind(p.status.as_str())
        .bind(&p.answer_run_id)
        .bind(&p.created_at)
        .bind(&p.decided_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(replaced)
    }

    pub async fn get_instruction_proposal(&self, id: &str) -> Result<InstructionProposal> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM instruction_proposals WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("instruction proposal {id}")))?;
        proposal_from_row(&row)
    }

    /// The proposals of one dot or of all, newest first.
    pub async fn list_instruction_proposals(
        &self,
        dot_id: Option<&str>,
    ) -> Result<Vec<InstructionProposal>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM instruction_proposals WHERE ? IS NULL OR dot_id = ? \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(proposal_from_row).collect()
    }

    /// Decides the pending proposal `id` and stores the run that continues the dot's chat, in
    /// one transaction: a second decision is refused.
    pub async fn decide_instruction_proposal(
        &self,
        id: &str,
        status: InstructionStatus,
        new: &NewRun,
    ) -> Result<(InstructionProposal, Run)> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE instruction_proposals SET status = ?, answer_run_id = ?, decided_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(&run_id)
        .bind(crate::util::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            self.get_instruction_proposal(id).await?;
            return Err(Error::Conflict(DECIDED_ALREADY.to_string()));
        }
        tx.commit().await?;
        Ok((
            self.get_instruction_proposal(id).await?,
            self.get_run(&run_id).await?,
        ))
    }
}
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod follow_ups;
mod library;
```

**with:**

```rust
mod follow_ups;
mod instructions;
mod library;
```

**In `ui/src/api/types.ts`, replace:**

```ts
  z.strictObject({ type: z.literal("app_proposal_changed"), id: z.string(), dot_id: z.string() }),
  // A watcher of the dot was added, changed, paused, resumed, deleted or had an event.
```

**with:**

```ts
  z.strictObject({ type: z.literal("app_proposal_changed"), id: z.string(), dot_id: z.string() }),
  // A dot proposed instructions, or a proposal was decided or replaced: the card reads it again.
  z.strictObject({
    type: z.literal("instruction_proposal_changed"),
    id: z.string(),
    dot_id: z.string(),
  }),
  // A watcher of the dot was added, changed, paused, resumed, deleted or had an event.
```

Then regenerate the contract fixture (the event's sample):

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`
Expected: PASS; the fixture gains the `instruction_proposal_changed` sample.

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test instructions_test`
Expected: PASS: 4 tests. Run it twice: the replacing transaction writes first, so a second connection's writes can't make it fail with "database is locked".

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2`, `cargo test -p dots-app`, `cargo clippy -p dots-core -p dots-app --tests`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 1763 passed, `dots-app` 124, the UI 748; no new warning.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/src/checkins.rs crates/dots-core/src/events.rs crates/dots-core/src/instructions.rs crates/dots-core/src/store/instructions.rs crates/dots-core/src/store/mod.rs crates/dots-core/tests/instructions_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts
git commit -m "feat(core): instruction proposals: the propose check, the store and the changed event"
```

### Task 5: `propose_instructions` on both engines

Spec section 4, "Only for a person's chat message": the runner offers the tool to a run whose topic the person started, unless the policy denies `Instructions`.
- **Claude engine:** Dots' own `dots` MCP server lists and serves it, and the CLI allows `mcp__dots__propose_instructions`.
- **Local engine:** it is built in, as the memory tools are.
- **Prompt:** the chat's first prompt names it, or tells the dot to send the person to the editor.

**Files:**

- Create: `crates/dots-core/src/engine/local/tools/instructions.rs`, `crates/dots-core/tests/instructions_tools_test.rs`
- Modify: `crates/dots-core/src/instructions.rs`, `crates/dots-core/src/run_tools.rs`, `crates/dots-core/src/runner.rs`, `crates/dots-core/src/prompt.rs`, `crates/dots-core/src/server/mcp.rs`, `crates/dots-core/src/engine/claude/args.rs`, `crates/dots-core/src/engine/local/tools/mod.rs`, `crates/dots-core/src/engine/local/mcp_tools.rs`, `crates/dots-core/src/engine/local/mod.rs`
- Test: `crates/dots-core/tests/engine_prompt_test.rs`, `crates/dots-core/tests/local_tools_test.rs`. The local tools gain `propose_instructions`, aliased `Instructions`, which is not in `schemas()`.

**Interfaces:**

- Consumes: Task 4's `may_propose`, `Proposals::propose`, `CheckIns::instructions`, `TOOL`, `REQUIRED`, `mcp_schema`; Task 3's `INSTRUCTIONS_TOOL`.
- Produces:
  - `instructions::model_name(EngineKind) -> &'static str`
  - `RunTools::{instructions() -> bool, with_instructions(bool) -> RunTools}`
  - `PromptParts.instructions_tool: Option<&str>`
  - `prompt::NO_INSTRUCTIONS_TOOL`
  - `engine::local::tools::instructions::{schema, run}`
  - the local alias `("propose_instructions", "Instructions")`

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    build_prompt, build_prompt_full, context_tool_name, is_typed, recent_runs, PromptParts,
    CHAT_SUMMARY_RULE, CONVERSATION, RECENT_TEXT,
};
```

**with:**

```rust
    build_prompt, build_prompt_full, context_tool_name, is_typed, recent_runs, PromptParts,
    CHAT_SUMMARY_RULE, CONVERSATION, NO_INSTRUCTIONS_TOOL, RECENT_TEXT,
};
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
#[test]
fn only_a_chat_run_can_be_typed() {
```

**with:**

```rust
#[test]
fn a_person_s_chat_is_told_how_it_may_change_its_instructions() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let typed = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "do it twice from now on", "typed": true})),
        None,
    );
    let offered = PromptParts {
        instructions_tool: Some("propose_instructions"),
        ..PromptParts::default()
    };
    let p = build_prompt_full(&dot, &typed, Path::new("C:/ws"), at(), "", offered);
    assert!(p.contains(&format!(
        "{CONVERSATION}- When the person asks you to change how you work from now on, propose \
         new instructions with `propose_instructions` instead of only promising to.\n\n"
    )));
    assert!(!p.contains(NO_INSTRUCTIONS_TOOL));
    let p = build_prompt_full(
        &dot,
        &typed,
        Path::new("C:/ws"),
        at(),
        "",
        PromptParts::default(),
    );
    assert!(p.contains(&format!("{CONVERSATION}{NO_INSTRUCTIONS_TOOL}\n")));
    // A chat run Dots made is told neither.
    let made = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "Approved."})),
        None,
    );
    let p = build_prompt_full(&dot, &made, Path::new("C:/ws"), at(), "", offered);
    assert!(!p.contains("propose_instructions") && !p.contains(NO_INSTRUCTIONS_TOOL));
}

#[test]
fn only_a_chat_run_can_be_typed() {
```

**Create `crates/dots-core/tests/instructions_tools_test.rs`:**

```rust
//! `propose_instructions` on both engines (conversational chat spec section 4): offered to and
//! named in a run of a chat the person started, never to another run or under a deny rule.

mod common;
mod llm;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{spec, wait_status};
use dots_core::instructions::{InstructionStatus, SENT};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::policy::{Action, Rule};
use dots_core::prompt::NO_INSTRUCTIONS_TOOL;
use dots_core::{Config, Runtime};
use llm::{call as llm_call, messages, text as llm_text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start_with(edit: impl FnOnce(&mut Config, &Path)) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    edit(&mut cfg, dir.path());
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
    s
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

fn prompt_of(req: &Value) -> String {
    messages(req)
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_local_dot_proposes_from_the_person_s_chat_only() {
    let t = start_with(|_, _| {}).await;
    let llm = Llm::start(vec![
        llm_call(
            "c1",
            "propose_instructions",
            json!({"instructions": "Do the thing twice.", "reason": "Twice"}),
        ),
        llm_text("Sent it to you."),
        llm_text("Done."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "shopper"), None)
            .await
            .unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(tool_names(&reqs[0]).contains(&"propose_instructions".to_string()));
    assert!(prompt_of(&reqs[0])
        .contains("propose new instructions with `propose_instructions` instead"));
    assert_eq!(tool_result(&reqs[1], "c1"), SENT);
    let list =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].instructions, "Do the thing twice.");
    assert_eq!(list[0].status, InstructionStatus::Pending);
    assert_eq!(list[0].run_id, chat.id);

    // A run that is not a chat the person started is neither offered nor told about it.
    let manual = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &manual.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(!tool_names(&reqs[2]).contains(&"propose_instructions".to_string()));
    assert!(!prompt_of(&reqs[2]).contains("propose_instructions"));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_deny_rule_turns_the_tool_off_and_the_chat_is_told_to_send_the_person_to_the_editor() {
    let t = start_with(|_, _| {}).await;
    let llm = Llm::start(vec![llm_text("Edit them in the editor.")]).await;
    let mut s = local_spec(&t, &llm.endpoint(), "denied");
    s.policy
        .rules
        .insert(0, Rule::new("Instructions", Action::Deny));
    let dot = t.rt.create_dot(s, None).await.unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(!tool_names(&reqs[0]).contains(&"propose_instructions".to_string()));
    assert!(prompt_of(&reqs[0]).contains(NO_INSTRUCTIONS_TOOL.trim_end()));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_from_the_person_s_chat() {
    let script = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "propose_instructions",
                      "arguments": {"instructions": "Do the thing twice.", "reason": "Twice"}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    let t = start_with(|cfg, dir| {
        let path = dir.join("script.json");
        std::fs::write(&path, script.to_string()).unwrap();
        cfg.claude_env = vec![
            (
                "FAKE_CLAUDE_SCRIPT".into(),
                path.to_string_lossy().into_owned(),
            ),
            (
                "FAKE_CLAUDE_ARGS_OUT".into(),
                dir.join("args.json").to_string_lossy().into_owned(),
            ),
        ];
    })
    .await;
    let mut s = spec("claude");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    let dot = t.rt.create_dot(s, None).await.unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(t.dir.path().join("args.json")).unwrap())
            .unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        args.contains(&"mcp__dots__propose_instructions".to_string()),
        "{args:?}"
    );
    assert!(recorded["prompt"]
        .as_str()
        .unwrap()
        .contains("with `mcp__dots__propose_instructions` instead"));
    let list =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reason, "Twice");
    t.rt.shutdown();
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    // (`checkins::schemas`, `memory::schemas`), `propose_app`, which an engine with apps
    // adds, and `start_workflow`, which a chat run that may start a workflow gets
    // (`team::start_schema`).
    assert_eq!(
```

**with:**

```rust
    // (`checkins::schemas`, `memory::schemas`), `propose_app`, which an engine with apps
    // adds, `start_workflow`, which a chat run that may start a workflow gets
    // (`team::start_schema`), and `propose_instructions`, which a run of a chat the person
    // started gets (`instructions::schema`).
    assert_eq!(
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len() - 10
    );
```

**with:**

```rust
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len() - 11
    );
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}),
        ),
```

**with:**

```rust
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}),
        ),
        (
            "propose_instructions",
            json!({"instructions": "Do it twice.", "reason": "Twice"}),
        ),
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test engine_prompt_test --test instructions_tools_test`
Expected: FAIL to compile: no `NO_INSTRUCTIONS_TOOL` in `dots_core::prompt`, no field `instructions_tool` in `PromptParts`.

Run: `cargo test -p dots-core --test local_tools_test`
Expected: FAIL: `a_prepared_input_passes_prepare_again_unchanged` (`propose_instructions: unknown tool`) and the schema count test.

- [ ] **Step 3: Offer and serve the tool**

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
                .to_string(),
        );
    }
    if !allow.is_empty() {
```

**with:**

```rust
                .to_string(),
        );
    }
    // And of `propose_instructions`, for a run of a chat the person started (conversational
    // chat spec section 4).
    if ctx.tools.instructions() {
        allow.push(crate::instructions::model_name(EngineKind::Claude).to_string());
    }
    if !allow.is_empty() {
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust
        }
        // The built-in browser's tools (browser spec section 2.2); their names fit as they are,
```

**with:**

```rust
        }
        // A run of a chat the person started (conversational chat spec section 4).
        if self.ctx.tools.instructions() && self.checkins.is_some() {
            out.push(tools::instructions::schema());
        }
        // The built-in browser's tools (browser spec section 2.2); their names fit as they are,
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
        }
        if crate::memory::Tool::from_name(call.name).is_some() {
```

**with:**

```rust
        }
        if call.name == crate::instructions::TOOL {
            return match &self.checkins {
                Some(checkins) if self.ctx.tools.instructions() => {
                    tools::instructions::run(checkins.instructions(), &self.ctx.run_id, &call.input)
                        .await
                }
                _ => ToolOutput::err(format!("{} is not available in this run", call.name)),
            };
        }
        if crate::memory::Tool::from_name(call.name).is_some() {
```

**Create `crates/dots-core/src/engine/local/tools/instructions.rs`:**

```rust
//! `propose_instructions` on the local engine (policy name `Instructions`): built in, as the
//! memory tools are (conversational chat spec section 4). What it does is
//! `crate::instructions::Proposals`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::instructions::{Proposals, DESCRIPTION, REQUIRED, TOOL};

/// The function definition of `propose_instructions`, offered to a run of a chat the person
/// started.
pub fn schema() -> Value {
    tool(
        TOOL,
        DESCRIPTION,
        crate::instructions::properties(),
        &REQUIRED,
    )
}

/// Runs an allowed `propose_instructions` call (its input as `prepare` left it) for the run
/// `run_id`.
pub async fn run(proposals: &Proposals, run_id: &str, input: &Value) -> ToolOutput {
    let arg = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default();
    match proposals
        .propose(run_id, arg("instructions"), arg("reason"))
        .await
    {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
mod fs;
pub mod memory;
```

**with:**

```rust
mod fs;
pub mod instructions;
pub mod memory;
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// the workspace to the person (app library spec section 6.2), offered when the engine has apps.
pub const TOOLS: &[(&str, &str)] = &[
```

**with:**

```rust
/// the workspace to the person (app library spec section 6.2), offered when the engine has apps.
/// `propose_instructions` proposes new instructions for the dot (conversational chat spec
/// section 4), offered to a run of a chat the person started.
pub const TOOLS: &[(&str, &str)] = &[
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    ("start_workflow", "Workflow"),
];
```

**with:**

```rust
    ("start_workflow", "Workflow"),
    ("propose_instructions", "Instructions"),
];
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
            ("task", Str, true),
        ],
    ),
];
```

**with:**

```rust
            ("task", Str, true),
        ],
    ),
    (
        "propose_instructions",
        &[("instructions", Str, true), ("reason", Str, true)],
    ),
];
```

**In `crates/dots-core/src/instructions.rs`, replace:**

```rust
use crate::events::{Bus, RuntimeEvent};
use crate::model::Run;
use crate::store::Store;
```

**with:**

```rust
use crate::events::{Bus, RuntimeEvent};
use crate::model::{EngineKind, Run};
use crate::store::Store;
```

**In `crates/dots-core/src/instructions.rs`, replace:**

```rust
pub const REQUIRED: [&str; 2] = ["instructions", "reason"];

```

**with:**

```rust
pub const REQUIRED: [&str; 2] = ["instructions", "reason"];

/// The name the run's model knows the tool by: Dots' own MCP server's tool on the Claude
/// engine, a built-in tool on the local engine.
pub fn model_name(engine: EngineKind) -> &'static str {
    match engine {
        EngineKind::Claude => "mcp__dots__propose_instructions",
        EngineKind::Local => TOOL,
    }
}

```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    tell the person to use Reply on that run's card.\n";

```

**with:**

```rust
    tell the person to use Reply on that run's card.\n";

/// What a person's chat is told when its dot may not propose instructions.
pub const NO_INSTRUCTIONS_TOOL: &str = "- You cannot change your own instructions: when the \
    person asks you to change how you work from now on, tell them to edit your instructions \
    in the dot's editor.\n";

```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    pub recent: &'a [Run],
}
```

**with:**

```rust
    pub recent: &'a [Run],
    /// The name the run's model calls `propose_instructions` by, when the run has it.
    pub instructions_tool: Option<&'a str>,
}
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        recent,
    } = parts;
```

**with:**

```rust
        recent,
        instructions_tool,
    } = parts;
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        out.push_str(CONVERSATION);
        out.push('\n');
```

**with:**

```rust
        out.push_str(CONVERSATION);
        out.push_str(&match instructions_tool {
            Some(tool) => format!(
                "- When the person asks you to change how you work from now on, propose new \
                 instructions with `{tool}` instead of only promising to.\n"
            ),
            None => NO_INSTRUCTIONS_TOOL.to_string(),
        });
        out.push('\n');
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    startable: Vec<crate::model::StartableWorkflow>,
}
```

**with:**

```rust
    startable: Vec<crate::model::StartableWorkflow>,
    /// The run may propose new instructions for its dot (conversational chat spec section 4).
    instructions: bool,
}
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        &self.startable
    }
```

**with:**

```rust
        &self.startable
    }

    /// The run may propose new instructions with `propose_instructions`.
    pub fn instructions(&self) -> bool {
        self.instructions
    }

    /// The run's tools, with `propose_instructions` when `offered`.
    pub fn with_instructions(mut self, offered: bool) -> RunTools {
        self.instructions = offered;
        self
    }
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        Cow::Borrowed(crate::team_tools::Tool::for_input(input).model_name(engine))
    } else if tool == crate::apps::PROPOSE_POLICY {
```

**with:**

```rust
        Cow::Borrowed(crate::team_tools::Tool::for_input(input).model_name(engine))
    } else if tool == crate::instructions::POLICY_NAME {
        Cow::Borrowed(crate::instructions::model_name(engine))
    } else if tool == crate::apps::PROPOSE_POLICY {
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        };
        let tools = Arc::new(tools);
```

**with:**

```rust
        };
        // A run of a chat the person started may propose new instructions for its dot
        // (conversational chat spec section 4), unless its policy denies `Instructions`
        // outright: the tool is then neither offered nor named.
        let instructions_off = dot
            .spec
            .policy
            .resolve(crate::policy::INSTRUCTIONS_TOOL, &Value::Null)
            == crate::policy::Action::Deny;
        let proposes = !instructions_off
            && self.checkins().is_some()
            && match crate::instructions::may_propose(&self.store, run).await {
                Ok(may) => may,
                Err(e) => {
                    tracing::warn!(run = %run.id, "reading the run's topic failed: {e}");
                    false
                }
            };
        let tools = tools.with_instructions(proposes);
        let tools = Arc::new(tools);
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
                recent: &recent,
            },
```

**with:**

```rust
                recent: &recent,
                instructions_tool: tools
                    .instructions()
                    .then(|| crate::instructions::model_name(dot.spec.engine)),
            },
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
            }
            json!({ "tools": tools })
```

**with:**

```rust
            }
            // A run of a chat the person started (conversational chat spec section 4).
            if has_instructions(&s, &run_id) {
                tools.push(crate::instructions::mcp_schema());
            }
            json!({ "tools": tools })
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
                (Some(crate::apps::propose::TOOL), _) => call_propose(&s, &run_id, &req).await,
                (_, Some(tool)) => call_check_in(&s, &run_id, &req, tool).await,
```

**with:**

```rust
                (Some(crate::apps::propose::TOOL), _) => call_propose(&s, &run_id, &req).await,
                (Some(crate::instructions::TOOL), _) => call_instructions(&s, &run_id, &req).await,
                (_, Some(tool)) => call_check_in(&s, &run_id, &req, tool).await,
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}
```

**with:**

```rust
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

/// The run may propose new instructions, so `propose_instructions` is one of its tools.
fn has_instructions(s: &ServerState, run_id: &str) -> bool {
    s.runner
        .run_tools(run_id)
        .is_some_and(|tools| tools.instructions())
}

/// Checks a `propose_instructions` call against the dot's policy under `Instructions` and
/// keeps the proposal (conversational chat spec section 4). A call that is not performed is a
/// tool result with `isError`, as for the check-in tools.
async fn call_instructions(
    s: &ServerState,
    run_id: &str,
    req: &Value,
) -> Result<Value, (i64, String)> {
    let Some(checkins) = s.runner.checkins().filter(|_| has_instructions(s, run_id)) else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let mut input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    // Only the tool's own arguments, as the local engine's `prepare` keeps.
    if let Value::Object(args) = &mut input {
        args.retain(|key, _| crate::instructions::REQUIRED.contains(&key.as_str()));
    }
    let input = match s
        .hub
        .check(run_id, crate::instructions::POLICY_NAME, input)
        .await
    {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let arg = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default();
    Ok(
        match checkins
            .instructions()
            .propose(run_id, arg("instructions"), arg("reason"))
            .await
        {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(text) => not_performed(&text),
        },
    )
}
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test engine_prompt_test --test instructions_tools_test --test local_tools_test --test instructions_test --test claude_cli_test`
Expected: PASS: `engine_prompt_test` 23, `instructions_tools_test` 3, `local_tools_test` 40, `instructions_test` 4, `claude_cli_test` as before.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2`, `cargo clippy -p dots-core --tests`, `cargo fmt --check`.
Expected: `dots-core` 1767 passed; no new warning.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/engine/claude/args.rs crates/dots-core/src/engine/local/mcp_tools.rs crates/dots-core/src/engine/local/mod.rs crates/dots-core/src/engine/local/tools/instructions.rs crates/dots-core/src/engine/local/tools/mod.rs crates/dots-core/src/instructions.rs crates/dots-core/src/prompt.rs crates/dots-core/src/run_tools.rs crates/dots-core/src/runner.rs crates/dots-core/src/server/mcp.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/instructions_tools_test.rs crates/dots-core/tests/local_tools_test.rs
git commit -m "feat(core): propose_instructions on both engines, offered to and named in a chat the person started"
```

### Task 6: The person decides, and the Inbox lists the waiting proposals

Spec section 4, "On approve", "Stale proposal" and "On reject":
- **Approving:** `Runtime::decide_instruction_proposal` saves the instructions through `update_dot`, unless they changed since the proposal.
- **The chat:** the topic continues with Dots' own words, as for an app proposal.
- **Busy topics:** a topic whose runs have not ended is refused.

The Inbox gains `instructions` and counts them.

**Files:**

- Create: `crates/dots-core/tests/runtime_instructions_test.rs`
- Modify: `crates/dots-core/src/instructions.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runtime.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts`, `ui/src/api/contract.test.ts`, `ui/src/api/__fixtures__/contract.json` (regenerated)
- Test: the UI tests' Inboxes gain `instructions: []`: `ui/src/app/Look.test.tsx`, `ui/src/lib/runtime-events.test.ts`, `ui/src/screens/{BrowserApproval,DotEditorScreen,InboxHeld,InboxScreen,InboxWatchers}.test.tsx`, `ui/src/screens/home/OpenConversation.test.tsx`, `ui/src/test/fixtures.ts`.

**Interfaces:**

- Consumes: Task 4's store functions, `Proposals::changed`, the constants; `checkins::{continuation, thread_busy}`; `Runtime::update_dot`.
- Produces:
  - `instructions::PendingInstructions { proposal, dot_name }`
  - `Inbox.instructions: Vec<PendingInstructions>` (newest first)
  - `Runtime::decide_instruction_proposal(id, approve: bool) -> Result<(InstructionProposal, Run)>`
  - UI: `T.InstructionStatus`, `T.InstructionProposal`, `T.PendingInstructions`, and the type exports `InstructionProposal` and `PendingInstructions`

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/runtime_instructions_test.rs`:**

```rust
//! The person decides instructions a dot proposed (conversational chat spec section 4): the
//! Inbox lists the waiting ones, approving saves them unless they changed since, and the chat
//! continues with what the person decided.

mod common;

use std::path::PathBuf;

use common::{spec, wait_status};
use dots_core::instructions::{InstructionStatus, APPROVED, DECIDED_ALREADY, REJECTED, STALE};
use dots_core::model::{DotSpec, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start(script: Option<Value>) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    if let Some(script) = script {
        let p = dir.path().join("script.json");
        std::fs::write(&p, script.to_string()).unwrap();
        cfg.claude_env
            .push(("FAKE_CLAUDE_SCRIPT".into(), p.to_string_lossy().to_string()));
    }
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

fn folder_spec(t: &Rt, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

/// A dot whose chat ended after it proposed `Do the thing twice.`, and the proposal's id.
async fn proposed(t: &Rt, name: &str) -> (String, Run, String) {
    let dot = t.rt.create_dot(folder_spec(t, name), None).await.unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    let chat = wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 15).await;
    t.rt.checkins()
        .instructions()
        .propose(&chat.id, "Do the thing twice.", "Twice")
        .await
        .unwrap();
    let id =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap()[0]
            .id
            .clone();
    (dot.id, chat, id)
}

fn message(run: &Run) -> &str {
    run.payload.as_ref().unwrap()["message"].as_str().unwrap()
}

#[tokio::test]
async fn approving_saves_the_instructions_and_continues_the_chat() {
    let t = start(None).await;
    let (dot_id, chat, id) = proposed(&t, "shopper").await;
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.instructions.len(), 1);
    assert_eq!(inbox.instructions[0].proposal.id, id);
    assert_eq!(inbox.instructions[0].dot_name, "shopper");
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    let (decided, run) = t.rt.decide_instruction_proposal(&id, true).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Approved);
    assert_eq!(decided.answer_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(
        t.rt.get_dot(&dot_id).await.unwrap().spec.instructions,
        "Do the thing twice."
    );
    // The chat goes on in the dot's session, with Dots' own words: not a message the person
    // typed.
    assert_eq!(run.trigger, TriggerKind::Chat);
    assert_eq!(run.parent_run_id.as_deref(), Some(chat.id.as_str()));
    assert_eq!(run.root_run_id, chat.id);
    assert_eq!(run.session_id, chat.session_id);
    assert_eq!(message(&run), APPROVED);
    assert!(!dots_core::prompt::is_typed(&run));
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    assert!(t.rt.inbox().await.unwrap().instructions.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    let again =
        t.rt.decide_instruction_proposal(&id, false)
            .await
            .unwrap_err();
    assert!(matches!(again, Error::Conflict(m) if m == DECIDED_ALREADY));
}

#[tokio::test]
async fn rejecting_keeps_the_instructions_and_tells_the_dot() {
    let t = start(None).await;
    let (dot_id, _chat, id) = proposed(&t, "shopper").await;
    let (decided, run) = t.rt.decide_instruction_proposal(&id, false).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Rejected);
    assert_eq!(message(&run), REJECTED);
    assert_eq!(
        t.rt.get_dot(&dot_id).await.unwrap().spec.instructions,
        "Do the thing."
    );
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
}

#[tokio::test]
async fn instructions_edited_since_the_proposal_are_not_overwritten() {
    let t = start(None).await;
    let (dot_id, _chat, id) = proposed(&t, "shopper").await;
    let mut edited = t.rt.get_dot(&dot_id).await.unwrap().spec;
    edited.instructions = "Do the other thing.".into();
    t.rt.update_dot(&dot_id, edited).await.unwrap();
    let err =
        t.rt.decide_instruction_proposal(&id, true)
            .await
            .unwrap_err();
    assert!(matches!(err, Error::Conflict(m) if m == STALE));
    assert_eq!(
        t.rt.get_dot(&dot_id).await.unwrap().spec.instructions,
        "Do the other thing."
    );
    let p = t.rt.store().get_instruction_proposal(&id).await.unwrap();
    assert_eq!(p.status, InstructionStatus::Pending);
    // The card still waits: the person may reject it.
    let (decided, _) = t.rt.decide_instruction_proposal(&id, false).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Rejected);
}

#[tokio::test]
async fn a_chat_that_is_still_working_is_not_continued() {
    let t = start(Some(json!([{"sleep_ms": 3000}]))).await;
    let dot =
        t.rt.create_dot(folder_spec(&t, "busy"), None)
            .await
            .unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Running, 15).await;
    t.rt.checkins()
        .instructions()
        .propose(&chat.id, "Do the thing twice.", "Twice")
        .await
        .unwrap();
    let id =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap()[0]
            .id
            .clone();
    let err =
        t.rt.decide_instruction_proposal(&id, true)
            .await
            .unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("still working on this chat")),
        "{err}"
    );
    assert_eq!(
        t.rt.get_dot(&dot.id).await.unwrap().spec.instructions,
        "Do the thing."
    );
    t.rt.shutdown();
}
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  AppProposal: T.AppProposal.array(),
  ToolsChanged: T.ToolsChanged.array(),
```

**with:**

```ts
  AppProposal: T.AppProposal.array(),
  InstructionProposal: T.InstructionProposal.array(),
  ToolsChanged: T.ToolsChanged.array(),
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  ProposalStatus: T.ProposalStatus,
  WatcherKind: T.WatcherKind,
```

**with:**

```ts
  ProposalStatus: T.ProposalStatus,
  InstructionStatus: T.InstructionStatus,
  WatcherKind: T.WatcherKind,
```

**In `ui/src/app/Look.test.tsx`, replace:**

```tsx
      tools_changed: [],
      watchers: [], held: [],
    }));
```

**with:**

```tsx
      tools_changed: [],
      watchers: [], held: [], instructions: [],
    }));
```

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts
describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [], watchers: [], held: [] };
  /// Shows the Inbox as the app's frame does.
```

**with:**

```ts
describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [], watchers: [], held: [], instructions: [] };
  /// Shows the Inbox as the app's frame does.
```

**In `ui/src/screens/BrowserApproval.test.tsx`, replace:**

```tsx
    tools_changed: [],
    watchers: [], held: [],
  }));
```

**with:**

```tsx
    tools_changed: [],
    watchers: [], held: [], instructions: [],
  }));
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
      tools_changed: [],
      watchers: [], held: [],
    };
```

**with:**

```tsx
      tools_changed: [],
      watchers: [], held: [], instructions: [],
    };
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [], attention: [], tools_changed: [], watchers: [], held: [] };
      return null;
```

**with:**

```tsx
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [], attention: [], tools_changed: [], watchers: [], held: [], instructions: [] };
      return null;
```

**In `ui/src/screens/InboxHeld.test.tsx`, replace:**

```tsx
  held,
});
```

**with:**

```tsx
  held,
  instructions: [],
});
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

/// An Inbox as a test writes it: without paused watchers or held runs unless it says so.
type InboxSpec = Omit<Inbox, "watchers" | "held"> & Partial<Pick<Inbox, "watchers" | "held">>;

```

**with:**

```tsx

/// An Inbox as a test writes it: without paused watchers, held runs or proposed instructions
/// unless it says so.
type InboxSpec = Omit<Inbox, "watchers" | "held" | "instructions"> &
  Partial<Pick<Inbox, "watchers" | "held" | "instructions">>;

```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
function inbox(initial: InboxSpec) {
  let current: Inbox = { watchers: [], held: [], ...initial };
  backend.on("inbox", () => current);
```

**with:**

```tsx
function inbox(initial: InboxSpec) {
  let current: Inbox = { watchers: [], held: [], instructions: [], ...initial };
  backend.on("inbox", () => current);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    set: (next: InboxSpec) => {
      current = { watchers: [], held: [], ...next };
    },
```

**with:**

```tsx
    set: (next: InboxSpec) => {
      current = { watchers: [], held: [], instructions: [], ...next };
    },
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
        watchers: [],
        held: [],
      };
```

**with:**

```tsx
        watchers: [],
        held: [], instructions: [],
      };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    tools_changed: [],
    watchers: [],
    held: [],
  };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));
```

**with:**

```tsx
    tools_changed: [],
    watchers: [],
    held: [], instructions: [],
  };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    watchers: [],
    held: [],
  };
```

**with:**

```tsx
    watchers: [],
    held: [], instructions: [],
  };
```

**In `ui/src/screens/InboxWatchers.test.tsx`, replace:**

```tsx
  watchers,
  held: [],
});
```

**with:**

```tsx
  watchers,
  held: [], instructions: [],
});
```

**In `ui/src/screens/home/OpenConversation.test.tsx`, replace:**

```tsx
      tools_changed: [],
      watchers: [], held: [],
    }));
```

**with:**

```tsx
      tools_changed: [],
      watchers: [], held: [], instructions: [],
    }));
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
    tools_changed: [],
    watchers: [], held: [],
  }));
```

**with:**

```ts
    tools_changed: [],
    watchers: [], held: [], instructions: [],
  }));
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test runtime_instructions_test`
Expected: FAIL to compile: no method `decide_instruction_proposal` on `Runtime`, no field `instructions` on `Inbox`.

- [ ] **Step 3: Decide, and list in the Inbox**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            held: held_runs(),
        }),
```

**with:**

```rust
            held: held_runs(),
            instructions: vec![dots_core::instructions::PendingInstructions {
                proposal: instruction_proposal(
                    "01J0INS",
                    dots_core::instructions::InstructionStatus::Pending,
                ),
                dot_name: "shopper".into(),
            }],
        }),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    apps_samples(&mut samples);
    samples["DraftSummary"] = to(DraftSummary {
```

**with:**

```rust
    apps_samples(&mut samples);
    samples["InstructionProposal"] = to(dots_core::instructions::InstructionStatus::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| instruction_proposal(&format!("01J0INS{i}"), *s))
        .collect::<Vec<_>>());
    samples["Enums"]["InstructionStatus"] = to(dots_core::instructions::InstructionStatus::ALL);
    samples["DraftSummary"] = to(DraftSummary {
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    samples
}
```

**with:**

```rust
    samples
}

/// Instructions a dot proposed (conversational chat spec section 4), with `status`.
fn instruction_proposal(
    id: &str,
    status: dots_core::instructions::InstructionStatus,
) -> dots_core::instructions::InstructionProposal {
    use dots_core::instructions::{InstructionProposal, InstructionStatus};
    let decided = status != InstructionStatus::Pending;
    InstructionProposal {
        id: id.into(),
        dot_id: "01J0DOT".into(),
        run_id: "01J0RUN".into(),
        root_run_id: "01J0RUN".into(),
        base: "Order the usual groceries.".into(),
        instructions: "Order the usual groceries.\nIf an item is out of stock, buy the \
                       closest brand and say so in the summary."
            .into(),
        reason: "Buy the closest brand when an item is out of stock.".into(),
        status,
        answer_run_id: (decided && status != InstructionStatus::Replaced)
            .then(|| "01J0RUN2".into()),
        created_at: TS.into(),
        decided_at: decided.then(|| TS.into()),
    }
}
```

**In `crates/dots-core/src/instructions.rs`, replace:**

```rust

/// The run may propose instructions: a run of a topic the person started with a message they
```

**with:**

```rust

/// A proposal that waits for the person, as the Inbox lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInstructions {
    pub proposal: InstructionProposal,
    pub dot_name: String,
}

/// The run may propose instructions: a run of a topic the person started with a message they
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub held: Vec<HeldRun>,
}
```

**with:**

```rust
    pub held: Vec<HeldRun>,
    /// Instructions dots proposed that wait for the person, newest first (conversational chat
    /// spec section 4).
    pub instructions: Vec<crate::instructions::PendingInstructions>,
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            .collect();
        Ok(Inbox {
```

**with:**

```rust
            .collect();
        let instructions = self
            .store
            .list_instruction_proposals(None)
            .await?
            .into_iter()
            .filter(|p| p.status == crate::instructions::InstructionStatus::Pending)
            .map(|proposal| crate::instructions::PendingInstructions {
                dot_name: name(&proposal.dot_id),
                proposal,
            })
            .collect();
        Ok(Inbox {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            held: self.held_runs(&names).await?,
        })
```

**with:**

```rust
            held: self.held_runs(&names).await?,
            instructions,
        })
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            + usize::from(!inbox.held.is_empty())
            + self.store.count_waiting_messages().await?)
```

**with:**

```rust
            + usize::from(!inbox.held.is_empty())
            + inbox.instructions.len()
            + self.store.count_waiting_messages().await?)
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    /// Decides an approval. With `allow_rule`, an approval also saves that allow rule to the
```

**with:**

```rust

    /// The person's answer on a card of instructions a dot proposed (conversational chat spec
    /// section 4). Approving saves them as the editor does (`update_dot`), unless the dot's
    /// instructions changed since it proposed: that is a `Conflict` and the card waits. Then
    /// the proposal is decided and the dot's chat continues with what the person decided, as
    /// for an app proposal (`decide_app_proposal`). A chat that has not ended is refused.
    pub async fn decide_instruction_proposal(
        &self,
        id: &str,
        approve: bool,
    ) -> Result<(crate::instructions::InstructionProposal, Run)> {
        use crate::instructions::{InstructionStatus, APPROVED, DECIDED_ALREADY, REJECTED, STALE};
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        let p = self.store.get_instruction_proposal(id).await?;
        if p.status != InstructionStatus::Pending {
            return Err(Error::Conflict(DECIDED_ALREADY.into()));
        }
        let asked = self.store.get_run(&p.run_id).await?;
        let thread = self.store.list_thread(&asked.root_run_id).await?;
        if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
            return Err(crate::checkins::thread_busy(active.status));
        }
        let end = thread.last().unwrap_or(&asked);
        let (status, said) = if approve {
            let dot = self.store.get_dot(&p.dot_id).await?;
            if dot.spec.instructions.trim() != p.base {
                return Err(Error::Conflict(STALE.into()));
            }
            let mut spec = dot.spec;
            spec.instructions = p.instructions.clone();
            self.update_dot(&p.dot_id, spec).await?;
            (InstructionStatus::Approved, APPROVED)
        } else {
            (InstructionStatus::Rejected, REJECTED)
        };
        let message = if end.session_id.is_some() {
            said.to_string()
        } else {
            format!(
                "Earlier you proposed new instructions: \"{}\"\n{said}",
                p.reason
            )
        };
        let new = crate::checkins::continuation(&asked, end, message, None);
        let (decided, run) = self
            .store
            .decide_instruction_proposal(id, status, &new)
            .await?;
        self.runner.enqueued(&run);
        self.checkins
            .instructions()
            .changed(&decided.id, &decided.dot_id);
        Ok((decided, run))
    }

    /// Decides an approval. With `allow_rule`, an approval also saves that allow rule to the
```

**In `ui/src/api/types.ts`, replace:**

```ts
});
export const Inbox = z.strictObject({
```

**with:**

```ts
});
// Instructions a dot proposed for itself, and what the person decided (conversational chat
// spec section 4). `base` is the dot's instructions when it proposed.
export const InstructionStatus = z.enum(["pending", "approved", "rejected", "replaced"]);
export const InstructionProposal = z.strictObject({
  id: z.string(),
  dot_id: z.string(),
  run_id: z.string(),
  root_run_id: z.string(),
  base: z.string(),
  instructions: z.string(),
  reason: z.string(),
  status: InstructionStatus,
  answer_run_id: nullable(z.string()),
  created_at: z.string(),
  decided_at: nullable(z.string()),
});
export const PendingInstructions = z.strictObject({
  proposal: InstructionProposal,
  dot_name: z.string(),
});

export const Inbox = z.strictObject({
```

**In `ui/src/api/types.ts`, replace:**

```ts
  held: z.array(HeldRun),
});
```

**with:**

```ts
  held: z.array(HeldRun),
  // Instructions dots proposed that wait for the person, newest first.
  instructions: z.array(PendingInstructions),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type Inbox = z.infer<typeof Inbox>;
export type McpServerConfig = z.infer<typeof McpServerConfig>;
```

**with:**

```ts
export type Inbox = z.infer<typeof Inbox>;
export type InstructionProposal = z.infer<typeof InstructionProposal>;
export type PendingInstructions = z.infer<typeof PendingInstructions>;
export type McpServerConfig = z.infer<typeof McpServerConfig>;
```

Then regenerate the contract fixture (the Inbox and proposal samples, the enum):

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`
Expected: PASS.

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test runtime_instructions_test`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/api`
Expected: PASS: 4 tests; the UI contract tests parse the new samples.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2`, `cargo test -p dots-app`, `cargo clippy -p dots-core -p dots-app --tests`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 1771 passed, `dots-app` 124, the UI 750; no new warning.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/src/instructions.rs crates/dots-core/src/model.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/runtime_instructions_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/app/Look.test.tsx ui/src/lib/runtime-events.test.ts ui/src/screens/BrowserApproval.test.tsx ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/InboxHeld.test.tsx ui/src/screens/InboxScreen.test.tsx ui/src/screens/InboxWatchers.test.tsx ui/src/screens/home/OpenConversation.test.tsx ui/src/test/fixtures.ts
git commit -m "feat(core): the person approves or rejects proposed instructions, and the Inbox lists the waiting ones"
```

### Task 7: The app's commands, their wrappers, and the event's refresh

The Tauri app gains `instruction_proposals_list` and `instruction_proposal_decide`. The UI gains their wrappers and the query key `instructionProposals(dotId)`, which `instruction_proposal_changed` marks stale with the Inbox.

**Files:**

- Modify: `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs`, `ui/src/api/commands.ts`, `ui/src/lib/query.ts`, `ui/src/lib/runtime-events.ts`, `ui/src/api/__fixtures__/contract.json` (regenerated: the command list)
- Test: `ui/src/lib/runtime-events.test.ts`

**Interfaces:**

- Consumes: Task 6's `Runtime::decide_instruction_proposal`; Task 4's `Store::list_instruction_proposals`.
- Produces:
  - commands `instruction_proposals_list(dot_id: Option<String>) -> Vec<InstructionProposal>` and `instruction_proposal_decide(id, approve: bool) -> InstructionProposal`
  - `commands.instructionProposalsList(dotId | null)` and `commands.instructionProposalDecide(id, approve)`
  - `keys.instructionProposals(dotId) = ["instruction-proposals", dotId]`

- [ ] **Step 1: Write the failing test**

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts
    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
```

**with:**

```ts
    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("marks the dot's proposed instructions and the inbox stale when a proposal changes", () => {
    const qc = createQueryClient();
    const lists = [keys.instructionProposals(sample.dot.id), keys.inbox];
    const untouched = [keys.instructionProposals("other"), keys.dot(sample.dot.id), keys.dots];
    for (const key of [...lists, ...untouched]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, {
      type: "instruction_proposal_changed",
      id: "01J0INS",
      dot_id: sample.dot.id,
    });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
```

- [ ] **Step 2: Run it to see it fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/lib/runtime-events.test.ts`
Expected: FAIL: 1 test (`keys.instructionProposals is not a function`).

- [ ] **Step 3: Add the commands and the refresh**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// The dot's watchers, oldest first (watchers spec section 8).
```

**with:**

```rust

/// The instructions dots proposed, of one dot or of all, newest first (conversational chat
/// spec section 4).
#[tauri::command]
pub async fn instruction_proposals_list(
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<Vec<dots_core::instructions::InstructionProposal>> {
    Ok(state
        .rt()?
        .store()
        .list_instruction_proposals(dot_id.as_deref())
        .await?)
}

/// The person's answer on the card: approve (save the instructions) or reject.
#[tauri::command]
pub async fn instruction_proposal_decide(
    state: State<'_, AppState>,
    id: String,
    approve: bool,
) -> Res<dots_core::instructions::InstructionProposal> {
    Ok(state
        .rt()?
        .decide_instruction_proposal(&id, approve)
        .await?
        .0)
}

/// The dot's watchers, oldest first (watchers spec section 8).
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    app_proposal_decide,
    watchers_list,
```

**with:**

```rust
    app_proposal_decide,
    instruction_proposals_list,
    instruction_proposal_decide,
    watchers_list,
```

**In `ui/src/api/commands.ts`, replace:**

```ts
    call(T.AppProposal, "app_proposal_decide", { id, decision }),

```

**with:**

```ts
    call(T.AppProposal, "app_proposal_decide", { id, decision }),
  /// The instructions dots proposed, of one dot or of all (null), newest first.
  instructionProposalsList: (dotId: string | null) =>
    call(T.InstructionProposal.array(), "instruction_proposals_list", { dotId }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already,
  /// when the dot's instructions changed since it proposed, and while its chat is working.
  instructionProposalDecide: (id: string, approve: boolean) =>
    call(T.InstructionProposal, "instruction_proposal_decide", { id, approve }),

```

**In `ui/src/lib/query.ts`, replace:**

```ts
  memories: (dotId: string) => ["memories", dotId] as const,
  watchers: (dotId: string) => ["watchers", dotId] as const,
```

**with:**

```ts
  memories: (dotId: string) => ["memories", dotId] as const,
  /// The instructions a dot proposed (conversational chat spec section 4).
  instructionProposals: (dotId: string) => ["instruction-proposals", dotId] as const,
  watchers: (dotId: string) => ["watchers", dotId] as const,
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      break;
    case "watcher_updated":
```

**with:**

```ts
      break;
    case "instruction_proposal_changed":
      // The dot's proposal cards and the Inbox's group of them. Approving changes the dot,
      // which the card that decided reads again itself.
      invalidate(qc, keys.instructionProposals(event.dot_id));
      invalidate(qc, keys.inbox);
      break;
    case "watcher_updated":
```

Then regenerate the contract fixture (its `Commands`):

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`
Expected: PASS; the fixture's `Commands` gains the two names.

- [ ] **Step 4: Run the checks**

Run: `cargo test -p dots-app`, `cargo clippy -p dots-app --tests`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-app` 124 passed (`every_command_in_commands_rs_is_registered_once` among them), the UI 751 (`commands.test.ts` "invokes exactly the commands the app registers" among them); no new warning.

- [ ] **Step 5: Commit**

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.ts ui/src/lib/query.ts ui/src/lib/runtime-events.test.ts ui/src/lib/runtime-events.ts
git commit -m "feat(app): the instruction proposal commands, their UI wrappers and the event that refreshes them"
```

### Task 8: The card, in the conversation and in the Inbox

Spec section 4, "The proposal card": "<dot> wants to change its instructions", the reason, a line diff of before and after, and Approve or Reject. A refusal shows as the core words it, and the card stays.
- **Conversation:** the card stands under the runs of the topic that proposed.
- **Inbox:** a group "New instructions" holds each card with its dot and Open conversation.

**Files:**

- Create: `ui/src/lib/line-diff.ts`, `ui/src/components/InstructionsCard.tsx`, `ui/src/lib/line-diff.test.ts`, `ui/src/screens/home/Instructions.test.tsx`
- Modify: `ui/src/screens/home/TopicRuns.tsx`, `ui/src/screens/home/Timeline.tsx`, `ui/src/screens/home/DotPane.tsx`, `ui/src/screens/InboxScreen.tsx`, `ui/src/test/fixtures.ts` (the default `instruction_proposals_list` answer)

**Interfaces:**

- Consumes: Task 7's commands and key; Task 6's types.
- Produces:
  - `lineDiff(before, after) -> DiffLine[]` (`{ kind: "same" | "added" | "removed", text }`)
  - `InstructionsCard({ proposal, dotName })`, an `article` named "New instructions for <dot>"
  - `TopicRuns` prop `proposals?` and `Timeline` prop `proposals`

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/lib/line-diff.test.ts`:**

```ts
import { describe, expect, it } from "vitest";

import { lineDiff } from "@/lib/line-diff";

describe("a line diff of instructions", () => {
  it("keeps the common lines and marks what was removed and added", () => {
    const before = "Order the usual groceries.\nPay with the saved card.\nSend the receipt.";
    const after =
      "Order the usual groceries.\nIf an item is out of stock, buy the closest brand.\n" +
      "Send the receipt.";
    expect(lineDiff(before, after)).toEqual([
      { kind: "same", text: "Order the usual groceries." },
      { kind: "removed", text: "Pay with the saved card." },
      { kind: "added", text: "If an item is out of stock, buy the closest brand." },
      { kind: "same", text: "Send the receipt." },
    ]);
  });

  it("adds every line to an empty text and removes every line of one emptied", () => {
    expect(lineDiff("", "a\nb")).toEqual([
      { kind: "added", text: "a" },
      { kind: "added", text: "b" },
    ]);
    expect(lineDiff("a", "")).toEqual([{ kind: "removed", text: "a" }]);
    expect(lineDiff("same", "same")).toEqual([{ kind: "same", text: "same" }]);
  });

  it("appends lines added at the end after the common ones", () => {
    expect(lineDiff("Do the thing.", "Do the thing.\nThen report.")).toEqual([
      { kind: "same", text: "Do the thing." },
      { kind: "added", text: "Then report." },
    ]);
  });
});
```

**Create `ui/src/screens/home/Instructions.test.tsx`:**

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { InstructionProposal } from "@/api/types";
import { backend } from "@/test/backend";
import { chat, dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const a = chat("a", "10:00", {
  payload: { message: "buy the closest brand from now on", typed: true },
});

const proposal = (over: Partial<InstructionProposal> = {}): InstructionProposal => ({
  id: "p1",
  dot_id: dot.id,
  run_id: "a",
  root_run_id: "a",
  base: "Order the usual groceries.\nPay with the saved card.",
  instructions:
    "Order the usual groceries.\nIf an item is out of stock, buy the closest brand.\n" +
    "Pay with the saved card.",
  reason: "Buy the closest brand when an item is out of stock.",
  status: "pending",
  answer_run_id: null,
  created_at: "2026-10-09T10:01:00.000Z",
  decided_at: null,
  ...over,
});

const named = `New instructions for ${dot.name}`;
const card = () => screen.findByRole("article", { name: named });

/// The dot's proposals are `list` until the person decides one; then there are none.
function deciding(list: InstructionProposal[]) {
  let now = list;
  backend.on("instruction_proposals_list", () => now);
  backend.on("instruction_proposal_decide", (args) => {
    now = [];
    return proposal({ status: args.approve ? "approved" : "rejected" });
  });
}

describe("instructions a dot proposed", () => {
  it("shows the waiting one in the conversation with its reason and changes", async () => {
    home([a]);
    backend.on("instruction_proposals_list", () => [
      proposal(),
      proposal({ id: "p0", status: "replaced", reason: "An older idea." }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = await screen.findByRole("region", { name: "Conversation" });
    const shown = await within(view).findByRole("article", { name: named });
    expect(shown).toHaveTextContent(`${dot.name} wants to change its instructions`);
    expect(shown).toHaveTextContent("Buy the closest brand when an item is out of stock.");
    const changes = within(shown).getByLabelText("Changes to the instructions");
    const added = [...changes.querySelectorAll('[data-kind="added"]')];
    expect(added.map((l) => l.textContent)).toEqual([
      "+ If an item is out of stock, buy the closest brand.",
    ]);
    expect(changes.querySelectorAll('[data-kind="removed"]')).toHaveLength(0);
    expect(within(view).getAllByRole("article", { name: /^New instructions/ })).toHaveLength(1);
    expect(screen.queryByText("An older idea.")).toBeNull();
    expect(backend.callsTo("instruction_proposals_list")).toContainEqual({ dotId: dot.id });
  });

  it("approves it, and reads the dot and the list again", async () => {
    home([a]);
    deciding([proposal()]);
    const { user } = renderApp(`/home/${dot.id}`);
    const shown = await card();
    const dotsAsked = backend.callsTo("dot_overview").length;
    await user.click(within(shown).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(backend.callsTo("instruction_proposal_decide")).toEqual([
        { id: "p1", approve: true },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("article", { name: named })).toBeNull());
    // Approving changed the dot.
    await waitFor(() =>
      expect(backend.callsTo("dot_overview").length).toBeGreaterThan(dotsAsked),
    );
  });

  it("rejects it", async () => {
    home([a]);
    deciding([proposal()]);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.click(within(await card()).getByRole("button", { name: "Reject" }));
    await waitFor(() =>
      expect(backend.callsTo("instruction_proposal_decide")).toEqual([
        { id: "p1", approve: false },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("article", { name: named })).toBeNull());
  });

  it("shows a refused decision as the core words it and keeps the card", async () => {
    const refusal =
      "the dot's instructions changed after it proposed these; ask it to propose them again";
    home([a]);
    backend.on("instruction_proposals_list", () => [proposal()]);
    backend.on("instruction_proposal_decide", () => {
      throw { kind: "conflict", message: refusal };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const shown = await card();
    await user.click(within(shown).getByRole("button", { name: "Approve" }));
    expect(await screen.findByText(refusal)).toBeVisible();
    expect(within(shown).getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("lists the waiting ones in the Inbox with their dot and conversation", async () => {
    backend.on("inbox", () => ({
      approvals: [],
      review_runs: [],
      attention: [],
      tools_changed: [],
      watchers: [],
      held: [],
      instructions: [{ proposal: proposal(), dot_name: dot.name }],
    }));
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "New instructions" });
    expect(within(group).getByRole("article", { name: named })).toBeVisible();
    expect(within(group).getByRole("link", { name: "Open conversation" })).toHaveAttribute(
      "href",
      `/chat/${dot.id}/a`,
    );
    expect(within(group).getByRole("link", { name: dot.name })).toHaveAttribute(
      "href",
      `/dots/${dot.id}`,
    );
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  }));
  // An approval card asks for these when it shows; by default the core offers no rule.
```

**with:**

```ts
  }));
  // No dot proposed instructions.
  backend.on("instruction_proposals_list", () => []);
  // An approval card asks for these when it shows; by default the core offers no rule.
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/lib/line-diff.test.ts src/screens/home/Instructions.test.tsx`
Expected: FAIL: `line-diff.test.ts` cannot import `@/lib/line-diff`; `Instructions.test.tsx` finds no card.

- [ ] **Step 3: Add the card and its places**

**Create `ui/src/components/InstructionsCard.tsx`:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { InstructionProposal } from "@/api/types";
import { Button } from "@/components/ui/button";
import { lineDiff, type DiffLine } from "@/lib/line-diff";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

const MARK: Record<DiffLine["kind"], string> = { same: "  ", added: "+ ", removed: "- " };
const LINE: Record<DiffLine["kind"], string> = {
  same: "",
  added: "bg-success/10 text-foreground",
  removed: "bg-destructive/10 text-foreground",
};

/// Instructions a dot proposed for itself (conversational chat spec section 4): why, its
/// instructions before and after line by line, and Approve or Reject. Nothing changes until the
/// person approves; the dot reads the answer as its chat's next message.
export function InstructionsCard(props: { proposal: InstructionProposal; dotName: string }) {
  const { proposal, dotName } = props;
  const qc = useQueryClient();
  const decide = useMutation({
    mutationFn: (approve: boolean) => commands.instructionProposalDecide(proposal.id, approve),
    onSuccess: (_answer, approve) => {
      invalidate(qc, keys.instructionProposals(proposal.dot_id));
      invalidate(qc, keys.inbox);
      // The dot's instructions changed, and no runtime event announces a dot change.
      if (approve) invalidateDots(qc);
    },
    onError: notifyError,
  });
  // One decision per card: after a success the card stays until its list has reloaded.
  const decided = decide.isPending || decide.isSuccess;
  const lines = lineDiff(proposal.base, proposal.instructions);
  return (
    <article
      aria-label={`New instructions for ${dotName}`}
      className="flex flex-col gap-2 rounded-xl border border-needs-border bg-needs-surface p-3"
    >
      <header className="flex flex-col gap-0.5">
        <span className="font-semibold text-needs">{dotName} wants to change its instructions</span>
        <span className="text-sm">{proposal.reason}</span>
      </header>
      <pre
        aria-label="Changes to the instructions"
        className="max-h-72 overflow-auto rounded-md bg-background/60 py-1 text-xs whitespace-pre-wrap"
      >
        {lines.map((line, i) => (
          <div key={i} className={`px-2 ${LINE[line.kind]}`} data-kind={line.kind}>
            {MARK[line.kind]}
            {line.text === "" ? " " : line.text}
          </div>
        ))}
      </pre>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={decided} onClick={onSingleClick(() => decide.mutate(true))}>
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={decided}
          onClick={onSingleClick(() => decide.mutate(false))}
        >
          Reject
        </Button>
      </div>
    </article>
  );
}
```

**Create `ui/src/lib/line-diff.ts`:**

```ts
// A line diff of two texts, for the card of instructions a dot proposed (conversational chat
// spec section 4).

export type DiffLine = { kind: "same" | "added" | "removed"; text: string };

/// The lines of `after` against those of `before`: a longest common subsequence of lines is
/// kept, what only `before` has is removed and what only `after` has is added, a removal
/// before the addition that takes its place.
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before === "" ? [] : before.split("\n");
  const b = after === "" ? [] : after.split("\n");
  // keep[i][j]: the longest common subsequence of a[i..] and b[j..].
  const keep = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      keep[i][j] = a[i] === b[j] ? keep[i + 1][j + 1] + 1 : Math.max(keep[i + 1][j], keep[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (keep[i + 1][j] >= keep[i][j + 1]) {
      out.push({ kind: "removed", text: a[i++] });
    } else {
      out.push({ kind: "added", text: b[j++] });
    }
  }
  while (i < a.length) out.push({ kind: "removed", text: a[i++] });
  while (j < b.length) out.push({ kind: "added", text: b[j++] });
  return out;
}
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
  PausedWatcher,
  ReviewRun,
```

**with:**

```tsx
  PausedWatcher,
  PendingInstructions,
  ReviewRun,
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
import { ErrorNote } from "@/components/ErrorNote";
import { MessageCard } from "@/components/MessageCard";
```

**with:**

```tsx
import { ErrorNote } from "@/components/ErrorNote";
import { InstructionsCard } from "@/components/InstructionsCard";
import { MessageCard } from "@/components/MessageCard";
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx

/// Everything that waits for the user: tool calls to approve, schedules that need attention,
```

**with:**

```tsx

/// Instructions a dot proposed, with links to its dot and its conversation.
function InstructionsRow({ item }: { item: PendingInstructions }) {
  const { proposal, dot_name: dotName } = item;
  return (
    <li className="flex flex-col gap-2">
      <p className="text-xs text-muted-foreground">
        <Link to={`/dots/${proposal.dot_id}`} className="text-foreground hover:underline">
          {dotName}
        </Link>{" "}
        proposed {relativeTime(proposal.created_at)} ·{" "}
        {/* The old chat address of the run opens its topic, in the conversation or alone. */}
        <Link to={`/chat/${proposal.dot_id}/${proposal.run_id}`} className="hover:underline">
          Open conversation
        </Link>
      </p>
      <InstructionsCard proposal={proposal} dotName={dotName} />
    </li>
  );
}

/// Everything that waits for the user: tool calls to approve, schedules that need attention,
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
    held,
  } = inbox.data;
```

**with:**

```tsx
    held,
    instructions,
  } = inbox.data;
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
    watchers.length === 0 &&
    held.length === 0
  ) {
```

**with:**

```tsx
    watchers.length === 0 &&
    held.length === 0 &&
    instructions.length === 0
  ) {
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
          ))}
        </section>
```

**with:**

```tsx
          ))}
        </section>
      )}
      {instructions.length > 0 && (
        <section aria-labelledby="inbox-instructions" className="flex flex-col gap-2">
          <h2 id="inbox-instructions" className="font-semibold">
            New instructions
          </h2>
          <ul className="flex flex-col gap-2">
            {instructions.map((item) => (
              <InstructionsRow key={item.proposal.id} item={item} />
            ))}
          </ul>
        </section>
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
    [];
  const held =
```

**with:**

```tsx
    [];
  // The instructions the dot proposed that wait for the person (conversational chat spec
  // section 4), each on a card under the run that proposed it.
  const proposals = (
    useQuery({
      queryKey: keys.instructionProposals(dot.id),
      queryFn: () => commands.instructionProposalsList(dot.id),
    }).data ?? []
  ).filter((p) => p.status === "pending");
  const held =
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
        dotName={dot.name}
        onReply={(topic) => setPick({ kind: "topic", rootId: topic.rootId })}
```

**with:**

```tsx
        dotName={dot.name}
        proposals={proposals}
        onReply={(topic) => setPick({ kind: "topic", rootId: topic.rootId })}
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx

import type { Message } from "@/api/types";
import { Button } from "@/components/ui/button";
```

**with:**

```tsx

import type { InstructionProposal, Message } from "@/api/types";
import { Button } from "@/components/ui/button";
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx
  dotName: string;
  /// Reply on a run card: makes its topic the reply target.
```

**with:**

```tsx
  dotName: string;
  /// The instructions the dot proposed that wait for the person.
  proposals: InstructionProposal[];
  /// Reply on a run card: makes its topic the reply target.
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx
              dotName={dotName}
            />
```

**with:**

```tsx
              dotName={dotName}
              proposals={props.proposals}
            />
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
import { commands } from "@/api/commands";
import type { Message, Run, RunEventRecord } from "@/api/types";
import { AppliedCalls } from "@/components/AppliedCalls";
```

**with:**

```tsx
import { commands } from "@/api/commands";
import type { InstructionProposal, Message, Run, RunEventRecord } from "@/api/types";
import { AppliedCalls } from "@/components/AppliedCalls";
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
import { LoadErrors } from "@/components/ErrorNote";
import { Failure, Transcript } from "@/components/Transcript";
```

**with:**

```tsx
import { LoadErrors } from "@/components/ErrorNote";
import { InstructionsCard } from "@/components/InstructionsCard";
import { Failure, Transcript } from "@/components/Transcript";
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
/// steps, check-ins and browser calls (`Transcript`), the calls Dots performed, a card per
/// approval it waits for, and why the last run failed.
export function TopicRuns(props: { runs: Run[]; checkIns: Message[]; dotName: string }) {
  const { runs, dotName } = props;
```

**with:**

```tsx
/// steps, check-ins and browser calls (`Transcript`), the calls Dots performed, a card per
/// approval it waits for, the instructions the dot proposed that wait for the person, and why
/// the last run failed.
export function TopicRuns(props: {
  runs: Run[];
  checkIns: Message[];
  dotName: string;
  /// The dot's waiting proposals; the topic shows those its runs made.
  proposals?: InstructionProposal[];
}) {
  const { runs, dotName } = props;
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
  const checkIns = props.checkIns.filter((m) => runs.some((run) => run.id === m.run_id));
  return (
```

**with:**

```tsx
  const checkIns = props.checkIns.filter((m) => runs.some((run) => run.id === m.run_id));
  const proposals = (props.proposals ?? []).filter((p) =>
    runs.some((run) => run.id === p.run_id),
  );
  return (
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
      ))}
      {failure && <Failure>{failure}</Failure>}
```

**with:**

```tsx
      ))}
      {proposals.map((proposal) => (
        <InstructionsCard key={proposal.id} proposal={proposal} dotName={dotName} />
      ))}
      {failure && <Failure>{failure}</Failure>}
```

- [ ] **Step 4: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: the UI 759 tests in 73 files pass; `tsc` clean.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/InstructionsCard.tsx ui/src/lib/line-diff.test.ts ui/src/lib/line-diff.ts ui/src/screens/InboxScreen.tsx ui/src/screens/home/DotPane.tsx ui/src/screens/home/Instructions.test.tsx ui/src/screens/home/Timeline.tsx ui/src/screens/home/TopicRuns.tsx ui/src/test/fixtures.ts
git commit -m "feat(ui): the card of proposed instructions, in the conversation and in the Inbox"
```

### Task 9: The spec, as built

The spec's new section 9 records what this plan settled (the table "Deviations").

**Files:**

- Modify: `docs/superpowers/specs/2026-10-27-conversational-chat-design.md`

- [ ] **Step 1: Add section 9**

**In `docs/superpowers/specs/2026-10-27-conversational-chat-design.md`, replace:**

```markdown
- Spacing, radii and sizes within the existing theme tokens.
```

**with:**

```markdown
- Spacing, radii and sizes within the existing theme tokens.

## 9. As built (plan A: sections 3 and 4)

Plan A (`docs/superpowers/plans/2026-10-27-conversational-chat-a.md`) builds sections 3, 3.1 and 4; plan B builds section 5. Where the spec left room:

- **A person's chat message** is a chat run whose payload has `"typed": true`, which `Runtime::chat` sets (`prompt::is_typed`). The runs Dots makes (`checkins::continuation`, the decisions on app and instruction proposals) have no such key and keep the task framing.
- **The conversation framing** is `prompt::CONVERSATION`, after the memories and the recent runs and before "Message from the user", with one line on changing instructions: the tool's name when the run has it, else `prompt::NO_INSTRUCTIONS_TOOL` (edit them in the editor). The summary rule becomes `prompt::CHAT_SUMMARY_RULE`.
- **A reply to a run topic** gets no framing line of its own: it resumes the run's session as before, with "Runs since your last message" when runs ended since.
- **The recent runs** are the newest 5 runs of the dot that ended (succeeded, failed or cancelled) outside the topic, other chat topics included, latest end first, one line each: trigger, local end time, `done`/`failed`/`cancelled`, and the first 300 characters of the summary, else of the error, on one line, cut with `…`. "Since the last message" means ended after the parent run was queued.
- **One proposal waits per dot**: a newer proposal replaces the waiting one (status `replaced`, card gone). The instructions are 1 to 20,000 characters, the reason 1 to 500; a proposal equal to the current instructions is refused. Proposals go with their dot.
- **The tool is offered** to a run whose topic's root is a person's chat message, not to a workflow step's run, and not when the policy denies `Instructions`. The preset rule is `Instructions` allow, last in every preset; `Instructions(...)` is refused, and the CLI never sees the rule.
- **Deciding** needs the topic's runs to have ended (the refusals of a reply), so a card can't be decided while the dot still works on the chat. Approving checks the instructions are still the proposal's `base`, else refuses with `instructions::STALE` and the card waits. The chat goes on with `instructions::APPROVED` or `REJECTED`, in the session when the last run had one.
- **The Inbox** has a "New instructions" group of the waiting proposals, each with its dot and Open conversation, and counts them. The dots' needs-you counts don't count them. The conversation shows only waiting proposals, under the topic's runs.
- **The card's diff** is a line diff of the two texts (`lib/line-diff.ts`), not `DiffView`, which shows git patches.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-10-27-conversational-chat-design.md
git commit -m "docs(spec): conversational chat plan A as built"
```

## After the last task

- The whole run: `cargo test -j 2 -p dots-core --no-fail-fast -- --test-threads=2` (1771), `cargo test -p dots-app` (124), `cargo clippy -p dots-core -p dots-app --tests`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test` (759), `bun run --cwd ui typecheck`.
- The spec's smoke with a real model (section 7) is the person's to run, not an agent's: the subscription rule. It comes after plan B, which changes the screen it looks at. What it checks for plan A:
  - a question answered with no process run
  - a question about a scheduled run answered from its summary
  - an instruction change approved and seen in the editor
  - "run it now" running the process
