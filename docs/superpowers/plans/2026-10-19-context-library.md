# Shared Context Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every dot can start from one set of global instructions kept in Settings, and can read shared context collections (folders of text documents the person keeps in the Library) with one read-only tool, `context`, on both engines.

**Architecture:** Settings gains `global_instructions` and each dot `use_global_instructions`; `build_prompt` puts the text at the head of a thread's first prompt. A new core module `context` walks a collection's folder and lists, searches and reads its documents, confined to the folder. The Library gains the kind `context` (`ContextConfig {path, description, every_dot}`) and dots tick collections in `uses.context`. A run's tools resolve its collections; the first prompt lists them; the local engine offers a built-in `context` tool and Dots' own `dots` MCP server offers the same tool to Claude runs, both checked as the policy tool `Context`, which every preset allows. Migration 0007 adds the column and, in a code step after 0006's, the `Context` rule to stored policies. The UI gets a Settings text area, an editor switch and checklist, and a Library tab.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio, axum and reqwest already in use, Tauri 2.12; Bun 1.3, Vite 8.3, React 19.3, TypeScript 6.0 (strict), TanStack Query 5.104, zod 4.6, Vitest 5.0 with Testing Library and jsdom. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-19-context-library-design.md` (commit `b904af4`, the base of this plan, branch `feat/context-library`). It extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md` and the MCP/skills spec `2026-10-15-mcp-skills-plugins-design.md`. Task 8 brings the context library spec and the main spec in line with what the tasks build (context library spec section 13); the table "Deviations" lists what changed and why.

**Previous plans:** `docs/superpowers/plans/2026-10-08-dots-core.md` (Plan 1), `2026-10-09-dots-local-engine.md` (Plan 2), `2026-10-09-dots-app-3a.md`, `2026-10-12-dots-app-3b.md`, `2026-10-13-dots-app-3c.md` (Plan 3), `2026-10-13-approvals-v2.md` and `2026-10-14-approvals-v2-followups.md` (Approvals v2), `2026-10-14-scheduler-v2.md` (Scheduler v2), `2026-10-15-mcp-skills-plugins.md` (MCP servers, skills and plugins).

**Where the tasks come from:** the context library spec, whose idea comes from the review of Cloudflare OS (its `gatekeeper-context` package and admin instance instructions) and Approvals v2 spec section 14 ("a shared context library that many dots read"). Task 1 is the global instructions end to end; Task 2 is the reader of a collection's folder, pure and standalone; Task 3 keeps collections in the Library; Task 4 gives a run its collections, the prompt section, the policy rule, the upgrade and the local tool; Task 5 gives Claude runs the same tool through Dots' own MCP server; Tasks 6 and 7 are the screens; Task 8 is the spec; Task 9 is the smoke.

**How this plan's code was checked:** the code blocks of Tasks 1 to 8 were generated from a throwaway worktree of commit `b904af4` (branch `proto/context-library`), one commit per task, where they compiled and passed on Windows 11, and the plan was then replayed onto `b904af4` by a script that applied every block and compared the tree with each task commit (all matched; the contract fixture was taken from the task commit, as its command writes it). Before the first task: `cargo test -p dots-core` 609 passed, `cargo test -p dots-app` 66, `bun run --cwd ui test` 372 tests in 25 files. At the last task: `dots-core` 646, `dots-app` 66, the UI 381 in 25 files; clippy without warnings, `cargo fmt --check` and `tsc --noEmit` clean.

## Global Constraints

- Toolchain: Rust 1.95 stable; Node 24; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/context-library`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\context-library`, based on `b904af4`. All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Every cargo command runs with `CARGO_TARGET_DIR=C:/Users/aksha/dots/target` (the shared build folder): `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target` once per shell.
- No new dependency (Cargo or npm). One migration, `0007_context.sql` (Task 1; Task 4 adds to it). `sqlx` runtime queries only.
- Global instructions are at most 32 KB (`32 * 1024` bytes, UTF-8). They head only a thread's first prompt, under the fixed headings `## Instructions for every dot` and `## Instructions for this dot`; resumes and chat follow-ups send only their message.
- A context collection's Library name matches `^[a-z0-9][a-z0-9_-]{0,39}$`; its description is at most 300 characters. A document is a file below the folder, at any depth, whose extension without regard to case is one of `md`, `markdown`, `txt`, `rst`, `adoc`, `org`, `json`, `yaml`, `yml`, `toml`, `csv`; names starting with `.` and links are left out. Bounds: 2,000 documents listed and searched per collection, 10 MB per document, 64 MB read per search, 30 KB per tool result, 10 search results with up to 3 lines each, 200 characters per summary or line, 50 collections in a prompt, 200 documents in the Library's "Documents".
- `context` never writes, and reads only below the collection's folder: a path that is absolute, holds `..`, a `:` or a hidden name, or leads out of the folder through a link, is refused.
- The policy tool is `Context`; every preset allows it, after `Skill`. A `Context` rule is never passed to the CLI's `--allowedTools` / `--disallowedTools`. Claude runs reach the tool as `mcp__dots__context` on Dots' own `dots` server, which checks each call with the run's policy; `--allowedTools` gains `mcp__dots__context` for a run with collections.
- How the code blocks are written. A step changes files in three ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
  - **Append to `path`:** the block is added at the end of the file.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit (the blocks hold `—` and `…`). The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`, plus `open` from `@tauri-apps/plugin-dialog`. No `fetch`, no other plugin, no new capability.
- Live data is parsed, not cast. A type the UI parses changes only together with the contract fixture: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then the same test without the variable, then `ui/src/api/types.ts`. The fixture `ui/src/api/__fixtures__/contract.json` is written by that command and never by hand.
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`), `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. Test output must be free of warnings. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first.
- Known on this machine: a `vitest` run started right after an install, a `tsc` run, a cargo build or a large write can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`, or time out in the tests that type into forms (one `DotEditorScreen` form test failed this way once while the prototype was measured). Run it again on its own; do not change the pool settings. Run the UI tests before the typecheck.
- Known flaky under load: `runtime_local_test::live_approval_lets_the_command_run` (it failed once in the measurement, at Task 5, and passed twice on its own), `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`, `claude_engine_test::a_finished_run_leaves_no_process_behind`, some `resume_test` tests that make worktrees. Rerun the test binary alone before treating one as a failure.
- Fact-Forcing Gate (a hook of this machine): the first Edit or Write of each file, and the first Bash command after a task notification, are blocked until the facts it asks for are stated (callers, affected API, data shapes, the instruction); then retry the identical call. For a large new file, write a one-line stub first.

## Deviations from the spec

The spec at `b904af4` says something else in these places; Task 8 changes the spec to match.

| Spec at `b904af4` | This plan | Why |
|---|---|---|
| §4: the local engine lists collections in its system prompt, after the skills | Both engines list them at the end of the first prompt (`build_prompt`), under `## Shared context` | One function for both engines; the local engine sends that prompt as its first user message |
| §4 is silent on CLI lists | `Policy::cli_lists` leaves `Context` rules out | The CLI has no `Context` tool; the `dots` server applies the rule itself |
| §8: the `Context` rule is added by migration 0007 | 0007's SQL lists the dots in `upgrade_0007`; `finish_0007` adds the rule after `finish_0006` | As SQL it ran before `finish_0006` and put `Context` ahead of `Skill` on a database from before 0006 |
| §6: an "Open folder" button; the editor switch shown only when the setting is not blank | No "Open folder" (the row shows the path); the switch is always shown, with a note | No command opens an arbitrary folder and the plan adds no plugin; reading Settings in the editor would add a query to every editor test for little |

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. A path out of a collection: `../x.md`, `sub/../../x.md`, an absolute path, `C:x.md`, `a.md:stream`, `.secret.md`, a junction to another folder, an empty path. Expected: refused, nothing read. Test: `context_test::reading_outside_the_collection_or_a_non_document_is_refused` (Task 2).
2. A dot's own `Context` deny rule on a Claude run: the CLI is told `--allowedTools mcp__dots__context`, so only Dots' server can refuse. Expected: the call returns `isError` and no document text. Tests: `claude_gateway_test::dots_own_server_offers_context_only_with_collections_and_checks_each_call` (Task 5) and, for local runs, `local_context_test::a_dots_own_context_rule_still_decides` (Task 4).
3. A database from before 0006 opened by this version: both code steps run in one open. Expected: rules end `Skill`, then `Context`, as the presets have them, and a rule that names `Context` already is left alone. Tests: `store_library_test::migration_0006_*` and `store_dots_test::migration_0007_lets_every_stored_policy_read_context` (Task 4).
4. A collection with more documents than the bound (2,001). Expected: the first 2,000 in sorted order, a closing line that says how many there are, no unbounded output. Test: `context_test::the_walk_stops_listing_at_the_bound_and_counts_all` (Task 2).
5. Global instructions on a resume or a chat follow-up, and a text of multi-byte characters at the limit. Expected: not repeated after the first prompt; the UI counts bytes as the core does (16,385 × "é" is refused). Tests: `engine_prompt_test::resumes_and_follow_ups_do_not_repeat_the_global_instructions` (Task 1) and `SettingsScreen.test.tsx` "saves the global instructions, up to 32 KB" (Task 6).

## File Map

- `crates/dots-core/src/context.rs` (new, Task 2): walking, describing, listing, searching and reading a collection's documents; the `context` tool's description, arguments and prompt section.
- `crates/dots-core/src/engine/local/tools/context.rs` (new, Task 4): the local engine's `context` tool.
- `crates/dots-core/migrations/0007_context.sql` (new, Task 1; Task 4): `dots.use_global_instructions`, the `upgrade_0007` list.
- `crates/dots-core/src/settings.rs`, `model.rs`, `store/dots.rs`, `prompt.rs`, `runner.rs` (Task 1; `prompt.rs` and `runner.rs` again in Task 4): global instructions.
- `crates/dots-core/src/library.rs`, `library_service.rs`, `store/library.rs` (Task 3; `store/library.rs` again in Task 4): the Library kind `context`.
- `crates/dots-core/src/run_tools.rs`, `policy.rs`, `store/mod.rs`, `store/upgrade.rs`, `engine/local/{mod,mcp_tools}.rs`, `engine/local/tools/mod.rs` (Task 4): a run's collections, the `Context` rule and upgrade, the local tool.
- `crates/dots-core/src/server/mcp.rs`, `server/gateway.rs`, `engine/claude/args.rs` (Task 5): the Claude engine's `context` tool.
- `apps/dots-app/src/{commands,contract,lib}.rs` (Tasks 1 and 3): the `context_documents` command and the contract samples.
- `ui/src/api/{types,commands}.ts`, `ui/src/lib/dot-rules.ts` (Tasks 1 and 3): the UI's types and command.
- `ui/src/screens/SettingsScreen.tsx`, `screens/dot-editor/{DotForm,ToolsAndSkills}.tsx` (Task 6); `screens/library/{ContextTab,LibraryScreen,parts}.tsx` (Tasks 3 and 7): the screens.
- Tests: `crates/dots-core/tests/{context_test,local_context_test}.rs` (new) and the existing test files each task names; the UI's `*.test.tsx` next to their screens, `ui/src/test/fixtures.ts`.
- `docs/superpowers/specs/2026-10-08-dots-design.md`, `2026-10-19-context-library-design.md` (Task 8).

### Task 1: Start every dot's first prompt with the global instructions of Settings

Settings gains the text, each dot the switch (a column of migration 0007), and `build_prompt` the head. The runner reads the setting when it builds each run's prompt, so a change needs no restart. The UI's types follow, with the fixture.

**Files:**

- Modify: `apps/dots-app/src/contract.rs`
- Create: `crates/dots-core/migrations/0007_context.sql`
- Modify: `crates/dots-core/src/model.rs`
- Modify: `crates/dots-core/src/prompt.rs`
- Modify: `crates/dots-core/src/runner.rs`
- Modify: `crates/dots-core/src/settings.rs`
- Modify: `crates/dots-core/src/store/dots.rs`
- Modify: `crates/dots-core/tests/claude_engine_test.rs`
- Modify: `crates/dots-core/tests/engine_prompt_test.rs`
- Modify: `crates/dots-core/tests/settings_test.rs`
- Modify: `crates/dots-core/tests/store_dots_test.rs`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Modify: `ui/src/api/types.ts`
- Modify: `ui/src/lib/dot-rules.ts`

**Interfaces:**

- Consumes: nothing new.
- Produces: `Settings.global_instructions: String` and `settings::MAX_GLOBAL_INSTRUCTIONS: usize = 32 * 1024` (checked in `Settings::validate`: "global_instructions must be at most 32 KB"); `DotSpec.use_global_instructions: bool` (`#[serde(default = "yes")]`, `true` in `DotSpec::new`), stored in `dots.use_global_instructions`; `prompt::build_prompt(dot: &Dot, run: &Run, workspace: &Path, now: DateTime<Local>, global: &str) -> String`; in the UI, `Settings.global_instructions`, `DotSpec.use_global_instructions` and `LIMITS.max_global_instructions = 32768`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_engine_test.rs`, replace:**

```rust
                    "usage": {"input_tokens": 3, "output_tokens": 2}}})
}
```

**with:**

```rust
                    "usage": {"input_tokens": 3, "output_tokens": 2}}})
}

#[tokio::test]
async fn the_runner_starts_the_prompt_with_the_global_instructions_of_settings() {
    let e = e2e(
        json!([init(),
               {"emit": {"type": "assistant", "message": {"content": [{"type": "text", "text": "$PROMPT"}]}}},
               success("fake done")]),
        None,
    )
    .await;
    let mut settings = e.store.get_settings().await.unwrap();
    settings.global_instructions = "Use British spelling.".into();
    e.store.put_settings(&settings).await.unwrap();
    let dot = folder_dot(&e, 280).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 15).await;
    let events = e.store.list_events(&run.id, 0).await.unwrap();
    let text = events
        .iter()
        .find(|ev| ev.kind == "assistant_text")
        .unwrap();
    assert!(text.data["text"].as_str().unwrap().starts_with(
        "## Instructions for every dot\n\nUse British spelling.\n\n\
         ## Instructions for this dot\n\nDo the thing."
    ));
}
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Manual, None, None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws/RUN1"), at());
    assert!(p.starts_with("Do the thing."));
```

**with:**

```rust
    let run = fake_run(TriggerKind::Manual, None, None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws/RUN1"), at(), "");
    assert!(p.starts_with("Do the thing."));
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Webhook, Some(json!({"case": 42})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at());
    assert!(p.contains("```json\n{\n  \"case\": 42\n}\n```"));
```

**with:**

```rust
    let run = fake_run(TriggerKind::Webhook, Some(json!({"case": 42})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "");
    assert!(p.contains("```json\n{\n  \"case\": 42\n}\n```"));
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    assert_eq!(
        build_prompt(&dot, &resume, Path::new("C:/ws"), at()),
        "Approval granted."
```

**with:**

```rust
    assert_eq!(
        build_prompt(&dot, &resume, Path::new("C:/ws"), at(), ""),
        "Approval granted."
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    assert_eq!(
        build_prompt(&dot, &follow, Path::new("C:/ws"), at()),
        "and now?"
    );
}
```

**with:**

```rust
    assert_eq!(
        build_prompt(&dot, &follow, Path::new("C:/ws"), at(), ""),
        "and now?"
    );
}

#[test]
fn a_first_prompt_starts_with_the_global_instructions() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(TriggerKind::Manual, None, None);
    let p = build_prompt(
        &dot,
        &run,
        Path::new("C:/ws"),
        at(),
        "\n  Use British spelling.  \n",
    );
    assert!(p.starts_with(
        "## Instructions for every dot\n\nUse British spelling.\n\n\
         ## Instructions for this dot\n\nDo the thing."
    ));
    let chat = fake_run(TriggerKind::Chat, Some(json!({"message": "hi"})), None);
    let p = build_prompt(&dot, &chat, Path::new("C:/ws"), at(), "Be brief.");
    assert!(p.starts_with("## Instructions for every dot\n\nBe brief.\n\n"));
}

#[test]
fn blank_or_switched_off_global_instructions_leave_the_prompt_as_it_was() {
    let mut dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(TriggerKind::Manual, None, None);
    let plain = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "");
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), " \n\t "),
        plain
    );
    dot.spec.use_global_instructions = false;
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), "Be brief."),
        plain
    );
    assert!(plain.starts_with("Do the thing."));
}

#[test]
fn resumes_and_follow_ups_do_not_repeat_the_global_instructions() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let resume = fake_run(
        TriggerKind::Resume,
        Some(json!({"message": "go"})),
        Some("s"),
    );
    let follow = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "more"})),
        Some("s"),
    );
    for (run, message) in [(resume, "go"), (follow, "more")] {
        assert_eq!(
            build_prompt(&dot, &run, Path::new("C:/ws"), at(), "Be brief."),
            message
        );
    }
}
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Chat, Some(json!({"message": "status?"})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at());
    assert!(p.starts_with("Do the thing."));
```

**with:**

```rust
    let run = fake_run(TriggerKind::Chat, Some(json!({"message": "status?"})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "");
    assert!(p.starts_with("Do the thing."));
```

**In `crates/dots-core/tests/settings_test.rs`, replace:**

```rust
    assert!(!s.schedules_paused);
}
```

**with:**

```rust
    assert!(!s.schedules_paused);
    assert_eq!(s.global_instructions, "");
}
```

**In `crates/dots-core/tests/settings_test.rs`, replace:**

```rust
        schedules_paused: true,
    };
```

**with:**

```rust
        schedules_paused: true,
        global_instructions: "Use British spelling.\nSign off as the team.".into(),
    };
```

**In `crates/dots-core/tests/settings_test.rs`, replace:**

```rust
            "https",
        ),
    ];
    for (mutate, needle) in cases {
```

**with:**

```rust
            "https",
        ),
        (
            |s| s.global_instructions = "x".repeat(settings::MAX_GLOBAL_INSTRUCTIONS + 1),
            "32 KB",
        ),
    ];
    for (mutate, needle) in cases {
```

**In `crates/dots-core/tests/settings_test.rs`, replace:**

```rust
    assert_eq!(store.get_settings().await.unwrap(), Settings::default());
}
```

**with:**

```rust
    assert_eq!(store.get_settings().await.unwrap(), Settings::default());
    // 32 KB exactly is allowed.
    let s = Settings {
        global_instructions: "x".repeat(settings::MAX_GLOBAL_INSTRUCTIONS),
        ..Settings::default()
    };
    store.put_settings(&s).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), s);
}
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    s.max_turns = 7;
    let updated = store.update_dot(&dot.id, &s).await.unwrap();
```

**with:**

```rust
    s.max_turns = 7;
    assert!(s.use_global_instructions);
    s.use_global_instructions = false;
    let updated = store.update_dot(&dot.id, &s).await.unwrap();
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    assert_eq!(updated.spec.max_turns, 7);
    assert_eq!(updated.webhook_token, dot.webhook_token);
```

**with:**

```rust
    assert_eq!(updated.spec.max_turns, 7);
    assert!(!updated.spec.use_global_instructions);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec, s);
    assert_eq!(updated.webhook_token, dot.webhook_token);
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test engine_prompt_test --test settings_test --test store_dots_test --test claude_engine_test`
Expected: FAIL to compile: `build_prompt` takes 4 arguments, `use_global_instructions` and `global_instructions` and `MAX_GLOBAL_INSTRUCTIONS` do not exist.

- [ ] **Step 3: Add the setting, the dot's switch and the prompt's head**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::scheduler::MAX_ATTEMPTS;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS};
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::scheduler::MAX_ATTEMPTS;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS, MAX_GLOBAL_INSTRUCTIONS};
use serde_json::{json, Value};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        local_endpoints: vec!["http://127.0.0.1:1234".into()],
        ..Settings::default()
```

**with:**

```rust
        local_endpoints: vec!["http://127.0.0.1:1234".into()],
        global_instructions: "Use British spelling.".into(),
        ..Settings::default()
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            "max_schedule_attempts": MAX_ATTEMPTS,
        },
```

**with:**

```rust
            "max_schedule_attempts": MAX_ATTEMPTS,
            "max_global_instructions": MAX_GLOBAL_INSTRUCTIONS,
        },
```

**Create `crates/dots-core/migrations/0007_context.sql`:**

```sql
-- The shared context library (spec 2026-10-19-context-library-design.md, sections 2 and 8).

-- A dot's first prompt starts with the global instructions of Settings unless it is switched off.
ALTER TABLE dots ADD COLUMN use_global_instructions INTEGER NOT NULL DEFAULT 1;
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust

pub const MAX_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;
```

**with:**

```rust

fn yes() -> bool {
    true
}

pub const MAX_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub uses: Uses,
    pub use_user_settings: bool,
```

**with:**

```rust
    pub uses: Uses,
    /// The first prompt starts with the global instructions of Settings (context library spec
    /// section 2).
    #[serde(default = "yes")]
    pub use_global_instructions: bool,
    pub use_user_settings: bool,
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
            uses: Uses::default(),
            use_user_settings: false,
```

**with:**

```rust
            uses: Uses::default(),
            use_global_instructions: true,
            use_user_settings: false,
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust

pub fn build_prompt(dot: &Dot, run: &Run, workspace: &Path, now: DateTime<Local>) -> String {
    let message = run
```

**with:**

```rust

/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it; resumes and chat follow-ups send only
/// their message.
pub fn build_prompt(
    dot: &Dot,
    run: &Run,
    workspace: &Path,
    now: DateTime<Local>,
    global: &str,
) -> String {
    let message = run
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    let mut out = String::new();
    out.push_str(dot.spec.instructions.trim());
```

**with:**

```rust
    let mut out = String::new();
    let global = global.trim();
    if dot.spec.use_global_instructions && !global.is_empty() {
        out.push_str("## Instructions for every dot\n\n");
        out.push_str(global);
        out.push_str("\n\n## Instructions for this dot\n\n");
    }
    out.push_str(dot.spec.instructions.trim());
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        };
        let secret = self.register_secret(&run.id);
```

**with:**

```rust
        };
        let global = self.store.get_settings().await?.global_instructions;
        let secret = self.register_secret(&run.id);
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            workspace: prepared.path.clone(),
            prompt: build_prompt(&dot, run, &prepared.path, chrono::Local::now()),
            session_id: run.session_id.clone(),
```

**with:**

```rust
            workspace: prepared.path.clone(),
            prompt: build_prompt(&dot, run, &prepared.path, chrono::Local::now(), &global),
            session_id: run.session_id.clone(),
```

**In `crates/dots-core/src/settings.rs`, replace:**

```rust
pub const MAX_CONCURRENT_RUNS: usize = 16;

```

**with:**

```rust
pub const MAX_CONCURRENT_RUNS: usize = 16;
/// The longest global instructions, in bytes (context library spec section 2).
pub const MAX_GLOBAL_INSTRUCTIONS: usize = 32 * 1024;

```

**In `crates/dots-core/src/settings.rs`, replace:**

```rust
    pub schedules_paused: bool,
}
```

**with:**

```rust
    pub schedules_paused: bool,
    /// The text every dot's first prompt starts with, unless the dot switches it off.
    pub global_instructions: String,
}
```

**In `crates/dots-core/src/settings.rs`, replace:**

```rust
            schedules_paused: false,
        }
```

**with:**

```rust
            schedules_paused: false,
            global_instructions: String::new(),
        }
```

**In `crates/dots-core/src/settings.rs`, replace:**

```rust
            check_endpoint_url(url)?;
        }
```

**with:**

```rust
            check_endpoint_url(url)?;
        }
        if self.global_instructions.len() > MAX_GLOBAL_INSTRUCTIONS {
            return invalid("global_instructions must be at most 32 KB");
        }
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";
```

**with:**

```rust
pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, use_global_instructions, \
    use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            uses: serde_json::from_str(&row.try_get::<String, _>("uses")?)?,
            use_user_settings: row.try_get::<i64, _>("use_user_settings")? != 0,
```

**with:**

```rust
            uses: serde_json::from_str(&row.try_get::<String, _>("uses")?)?,
            use_global_instructions: row.try_get::<i64, _>("use_global_instructions")? != 0,
            use_user_settings: row.try_get::<i64, _>("use_user_settings")? != 0,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, use_user_settings, \
             max_turns, timeout_secs, approval_wait_secs, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
```

**with:**

```rust
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, use_global_instructions, \
             use_user_settings, max_turns, timeout_secs, approval_wait_secs, enabled, \
             created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
```

**with:**

```rust
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, \
             use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, updated_at = ? WHERE id = ? \
```

**with:**

```rust
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, \
             use_global_instructions = ?, use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, updated_at = ? WHERE id = ? \
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_user_settings as i64)
```

**with:**

```rust
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
```

**In `ui/src/api/types.ts`, replace:**

```ts
  uses: Uses,
  use_user_settings: z.boolean(),
```

**with:**

```ts
  uses: Uses,
  use_global_instructions: z.boolean(),
  use_user_settings: z.boolean(),
```

**In `ui/src/api/types.ts`, replace:**

```ts
  schedules_paused: z.boolean(),
});
```

**with:**

```ts
  schedules_paused: z.boolean(),
  global_instructions: z.string(),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
  max_schedule_attempts: 6,
} as const;
```

**with:**

```ts
  max_schedule_attempts: 6,
  max_global_instructions: 32768,
} as const;
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
    uses: dot.uses,
    use_user_settings: dot.use_user_settings,
```

**with:**

```ts
    uses: dot.uses,
    use_global_instructions: dot.use_global_instructions,
    use_user_settings: dot.use_user_settings,
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the fixture changes only where this task's types changed.

**Generated:** `ui/src/api/__fixtures__/contract.json`

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test engine_prompt_test --test settings_test --test store_dots_test --test claude_engine_test`
Expected: PASS.

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 613 passed, `dots-app` 66, the UI 372 in 25 files; no warnings.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/migrations/0007_context.sql crates/dots-core/src/model.rs crates/dots-core/src/prompt.rs crates/dots-core/src/runner.rs crates/dots-core/src/settings.rs crates/dots-core/src/store/dots.rs crates/dots-core/tests/claude_engine_test.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/settings_test.rs crates/dots-core/tests/store_dots_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/lib/dot-rules.ts
git commit -m "feat(core): start every dot's first prompt with the global instructions of Settings"
```

### Task 2: List, search and read the documents of a context collection

A new core module that knows nothing of runs or the Library: given a folder, it walks the documents in sorted order, describes each from its frontmatter or text, and answers the `context` tool's three forms within the spec's bounds. `call` is the entry both engines use from Tasks 4 and 5.

**Files:**

- Create: `crates/dots-core/src/context.rs`
- Modify: `crates/dots-core/src/lib.rs`
- Create: `crates/dots-core/tests/context_test.rs`

**Interfaces:**

- Consumes: `kit::parse_frontmatter(&str) -> Frontmatter { fields, body }` (existing).
- Produces: `dots_core::context` with `EXTENSIONS`, `MAX_DOCUMENTS = 2_000`, `MAX_DOCUMENT_BYTES = 10 MiB`, `MAX_SEARCH_BYTES = 64 MiB`, `MAX_OUTPUT = 30 KiB`, `MAX_PROMPT_COLLECTIONS = 50`, `DESCRIPTION: &str`, `properties() -> Value`; `struct Collection { name: String, dir: PathBuf, description: String, documents: usize }`; `struct Document { path, title, summary }` (`Serialize`); `struct Walk { paths: Vec<String>, total: usize }`; `walk(&Path) -> Result<Walk, String>`; `describe(path: &str, text: &str) -> Document`; `documents(root: &Path, limit: usize) -> Result<(Vec<Document>, usize), String>`; `list(&Path)`, `search(&Path, query: &str)`, `read(&Path, rel: &str, offset: usize)`, `call(&[Collection], &Value)`, all `-> Result<String, String>`; `prompt_section(&[Collection], tool: &str) -> String`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/context_test.rs`:**

```rust
use std::path::Path;

use dots_core::context::{self, Collection, MAX_DOCUMENTS, MAX_OUTPUT};
use serde_json::json;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Creates a directory link at `link` pointing to `target` (a junction on Windows).
fn dir_link(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// A collection named `company` of the folder `root`.
fn company(root: &Path) -> Vec<Collection> {
    vec![Collection {
        name: "company".into(),
        dir: root.to_path_buf(),
        description: "How we work".into(),
        documents: 0,
    }]
}

#[test]
fn the_walk_finds_documents_by_extension_in_sorted_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for rel in [
        "b.md",
        "A.TXT",
        "notes/c.markdown",
        "notes/deep/d.yaml",
        "data.csv",
        "conf.toml",
        "x.json",
        "y.yml",
        "z.rst",
        "w.adoc",
        "v.org",
        "app.exe",
        "logo.png",
        "README",
        ".hidden.md",
        ".git/config.md",
    ] {
        write(root, rel, "text");
    }
    let walk = context::walk(root).unwrap();
    assert_eq!(
        walk.paths,
        [
            "A.TXT",
            "b.md",
            "conf.toml",
            "data.csv",
            "notes/c.markdown",
            "notes/deep/d.yaml",
            "v.org",
            "w.adoc",
            "x.json",
            "y.yml",
            "z.rst",
        ]
    );
    assert_eq!(walk.total, 11);
}

#[test]
fn the_walk_leaves_out_folder_links() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "secret.md", "secret");
    write(dir.path(), "a.md", "a");
    dir_link(outside.path(), &dir.path().join("linked"));
    assert_eq!(context::walk(dir.path()).unwrap().paths, ["a.md"]);
}

#[test]
fn the_walk_stops_listing_at_the_bound_and_counts_all() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..=MAX_DOCUMENTS {
        write(dir.path(), &format!("d{i:05}.md"), "x");
    }
    let walk = context::walk(dir.path()).unwrap();
    assert_eq!(walk.paths.len(), MAX_DOCUMENTS);
    assert_eq!(walk.total, MAX_DOCUMENTS + 1);
    assert_eq!(walk.paths[0], "d00000.md");
    let list = context::list(dir.path()).unwrap();
    assert!(
        list.contains("the collection has 2001 documents; only the first 2000"),
        "{}",
        &list[list.len().saturating_sub(300)..]
    );
}

#[test]
fn a_missing_folder_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = context::walk(&dir.path().join("gone")).unwrap_err();
    assert!(err.starts_with("the folder cannot be read"), "{err}");
}

#[test]
fn titles_and_summaries_come_from_frontmatter_then_text_then_the_name() {
    let fm = context::describe(
        "people/ana.md",
        "---\ntitle: Ana Lima\ndescription: Owns billing\n---\n# Ignored\nBody",
    );
    assert_eq!(
        (fm.title.as_str(), fm.summary.as_str()),
        ("Ana Lima", "Owns billing")
    );
    let heading = context::describe("style.md", "\n# House style\n\n## Tone\nShort sentences.\n");
    assert_eq!(
        (heading.title.as_str(), heading.summary.as_str()),
        ("House style", "Short sentences.")
    );
    let plain = context::describe("notes/on-call.txt", "\n  First line here.  \nSecond");
    assert_eq!(
        (plain.title.as_str(), plain.summary.as_str()),
        ("on-call", "First line here.")
    );
    assert_eq!(plain.path, "notes/on-call.txt");
    // A frontmatter that does not parse gives no fields.
    let broken = context::describe("b.md", "---\ntitle: [unclosed\n---\n# Real\ntext");
    assert_eq!(broken.title, "Real");
    let long = context::describe("l.md", &"é".repeat(300));
    assert_eq!(long.summary.chars().count(), 201);
    assert!(long.summary.ends_with('…'));
}

#[test]
fn the_list_shows_one_line_per_document() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "style.md", "# House style\nShort sentences.");
    write(dir.path(), "empty.txt", "");
    assert_eq!(
        context::list(dir.path()).unwrap(),
        "- empty.txt — empty\n- style.md — House style: Short sentences."
    );
    let none = tempfile::tempdir().unwrap();
    assert_eq!(
        context::list(none.path()).unwrap(),
        "The collection has no documents."
    );
}

#[test]
fn a_long_list_is_capped_and_says_what_it_left_out() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..400 {
        write(
            dir.path(),
            &format!("doc{i:03}.md"),
            &format!("# Title {i}\n{}", "word ".repeat(40)),
        );
    }
    let list = context::list(dir.path()).unwrap();
    assert!(list.len() <= MAX_OUTPUT, "{}", list.len());
    assert!(
        list.contains("more documents are not listed; use query"),
        "{list}"
    );
}

#[test]
fn a_search_needs_every_word_and_ranks_by_hits() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "billing.md",
        "# Billing\nInvoices go out on the 1st.\nAsk Ana about invoices.\nInvoices, invoices!",
    );
    write(
        dir.path(),
        "people/ana.md",
        "# Ana\nAna owns invoices and billing.",
    );
    write(dir.path(), "style.md", "# Style\nNothing about money.");
    let out = context::search(dir.path(), "INVOICES ana").unwrap();
    assert!(
        out.starts_with("2 documents match \"INVOICES ana\":"),
        "{out}"
    );
    let billing = out.find("billing.md — Billing").unwrap();
    let ana = out.find("people/ana.md — Ana").unwrap();
    // billing.md: 4 hits of "invoices" and 1 of "ana". people/ana.md: 1 and 4 (its text, path
    // and title), so 5 against 6.
    assert!(billing < ana, "{out}");
    assert!(
        out.contains("  line 2: Invoices go out on the 1st."),
        "{out}"
    );
    assert!(out.contains("  line 3: Ask Ana about invoices."), "{out}");
    assert!(!out.contains("style.md"), "{out}");
}

#[test]
fn a_word_in_the_path_or_title_matches() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "runbooks/deploy.md", "Steps to ship.");
    let out = context::search(dir.path(), "runbooks").unwrap();
    assert!(out.starts_with("1 documents match"), "{out}");
    assert!(out.contains("runbooks/deploy.md — deploy"), "{out}");
}

#[test]
fn a_search_shows_the_ten_best_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..12 {
        write(dir.path(), &format!("d{i:02}.md"), &"alpha ".repeat(i + 1));
    }
    let out = context::search(dir.path(), "alpha").unwrap();
    assert!(
        out.starts_with("12 documents match \"alpha\"; the 10 best:"),
        "{out}"
    );
    assert!(out.find("d11.md").unwrap() < out.find("d10.md").unwrap());
    assert!(!out.contains("d01.md"), "{out}");
    assert!(!out.contains("d00.md"), "{out}");
}

#[test]
fn a_search_with_no_match_or_no_word_says_so() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", "alpha");
    assert_eq!(
        context::search(dir.path(), "beta").unwrap(),
        "No document matches \"beta\"."
    );
    assert!(context::search(dir.path(), " a b ").is_err());
}

#[test]
fn a_document_is_read_from_an_offset() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes/a.md", "one\ntwo\nthree\n");
    assert_eq!(
        context::read(dir.path(), "notes/a.md", 1).unwrap(),
        "one\ntwo\nthree"
    );
    assert_eq!(
        context::read(dir.path(), "notes/a.md", 2).unwrap(),
        "two\nthree"
    );
    assert_eq!(
        context::read(dir.path(), "./notes/a.md", 3).unwrap(),
        "three"
    );
    let past = context::read(dir.path(), "notes/a.md", 9).unwrap_err();
    assert!(
        past.contains("has 3 lines; offset 9 is past the end"),
        "{past}"
    );
}

#[test]
fn a_long_document_is_cut_with_the_offset_to_go_on() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=2000)
        .map(|i| format!("line number {i:04}\n"))
        .collect();
    write(dir.path(), "long.md", &text);
    let first = context::read(dir.path(), "long.md", 1).unwrap();
    assert!(first.len() <= MAX_OUTPUT);
    let next: usize = first
        .rsplit("continue with offset=")
        .next()
        .unwrap()
        .trim_end_matches(']')
        .parse()
        .unwrap();
    let last_shown = first.lines().rev().nth(1).unwrap();
    assert_eq!(last_shown, format!("line number {:04}", next - 1));
    let rest = context::read(dir.path(), "long.md", next).unwrap();
    assert!(
        rest.starts_with(&format!("line number {next:04}")),
        "{rest}"
    );
    // One line longer than a result: its start, then the next line.
    write(
        dir.path(),
        "wide.md",
        &format!("{}\nafter", "x".repeat(40_000)),
    );
    let wide = context::read(dir.path(), "wide.md", 1).unwrap();
    assert!(wide.len() <= MAX_OUTPUT);
    assert!(
        wide.ends_with("[truncated: continue with offset=2]"),
        "{}",
        &wide[wide.len() - 60..]
    );
    assert_eq!(context::read(dir.path(), "wide.md", 2).unwrap(), "after");
}

#[test]
fn reading_outside_the_collection_or_a_non_document_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("kb");
    write(&root, "a.md", "a");
    write(&root, "tool.exe", "MZ");
    write(&root, ".secret.md", "hidden");
    write(&root, "sub/b.md", "b");
    write(dir.path(), "outside.md", "secret");
    let outside_dir = tempfile::tempdir().unwrap();
    write(outside_dir.path(), "x.md", "secret");
    dir_link(outside_dir.path(), &root.join("linked"));
    let absolute = dir.path().join("outside.md").to_string_lossy().into_owned();
    for rel in [
        "../outside.md",
        "sub/../../outside.md",
        absolute.as_str(),
        "C:outside.md",
        "a.md:stream",
        ".secret.md",
        "linked/x.md",
        "",
    ] {
        let err = context::read(&root, rel, 1).unwrap_err();
        assert!(err.contains("is not inside the collection"), "{rel}: {err}");
    }
    let exe = context::read(&root, "tool.exe", 1).unwrap_err();
    assert_eq!(exe, "tool.exe is not a document of the collection");
    let gone = context::read(&root, "gone.md", 1).unwrap_err();
    assert_eq!(gone, "gone.md was not found in the collection");
    let folder = context::read(&root, "sub", 1).unwrap_err();
    assert!(folder.contains("is a folder"), "{folder}");
}

#[test]
fn a_call_lists_searches_or_reads_by_its_arguments() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "style.md", "# Style\nShort sentences.");
    let c = company(dir.path());
    assert_eq!(
        context::call(&c, &json!({"collection": "company"})).unwrap(),
        "- style.md — Style: Short sentences."
    );
    assert!(
        context::call(&c, &json!({"collection": "company", "query": "short"}))
            .unwrap()
            .starts_with("1 documents match")
    );
    assert_eq!(
        context::call(
            &c,
            &json!({"collection": "company", "path": "style.md", "offset": 2})
        )
        .unwrap(),
        "Short sentences."
    );
    // Empty optional arguments are no arguments.
    assert_eq!(
        context::call(
            &c,
            &json!({"collection": " company ", "query": " ", "path": null})
        )
        .unwrap(),
        "- style.md — Style: Short sentences."
    );
}

#[test]
fn a_call_with_bad_arguments_says_what_is_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let c = company(dir.path());
    let err = |input| context::call(&c, &input).unwrap_err();
    assert_eq!(err(json!({})), "collection is required");
    assert_eq!(
        err(json!({"collection": "clients"})),
        "unknown collection \"clients\". Collections: company"
    );
    assert_eq!(
        context::call(&[], &json!({"collection": "x"})).unwrap_err(),
        "unknown collection \"x\". This run has no collections."
    );
    assert!(
        err(json!({"collection": "company", "query": "a b", "path": "a.md"})).contains("not both")
    );
    for offset in [json!(0), json!(-1), json!(1.5), json!("2")] {
        assert_eq!(
            err(json!({"collection": "company", "path": "a.md", "offset": offset})),
            "offset must be a whole number of at least 1"
        );
    }
    let gone = company(&dir.path().join("gone"));
    assert_eq!(
        context::call(&gone, &json!({"collection": "company"})).unwrap_err(),
        "the folder of collection company is missing"
    );
}

#[test]
fn the_prompt_section_lists_the_collections() {
    assert_eq!(context::prompt_section(&[], "context"), "");
    let mut collections = vec![
        Collection {
            name: "company".into(),
            dir: "C:/kb".into(),
            description: "How we work".into(),
            documents: 42,
        },
        Collection {
            name: "clients".into(),
            dir: "C:/clients".into(),
            description: " ".into(),
            documents: 1,
        },
    ];
    let section = context::prompt_section(&collections, "mcp__dots__context");
    assert_eq!(
        section,
        "Shared context you can read. Call the tool `mcp__dots__context` with a collection \
         name to list its documents, `query` to search it, or `path` to read one. Look here \
         before you guess how the person works.\n\
         - company: How we work (42 documents)\n\
         - clients: (no description) (1 document)"
    );
    for i in 0..60 {
        collections.push(Collection {
            name: format!("c{i}"),
            dir: "C:/x".into(),
            description: String::new(),
            documents: 0,
        });
    }
    let section = context::prompt_section(&collections, "context");
    assert_eq!(section.lines().count(), 1 + context::MAX_PROMPT_COLLECTIONS);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test context_test`
Expected: FAIL to compile: unresolved import `dots_core::context`.

- [ ] **Step 3: Add the module**

**Create `crates/dots-core/src/context.rs`:**

```rust
//! The documents of a context collection (context library spec section 3): text files below a
//! folder the person chose in the Library. The `context` tool lists them, searches them and
//! reads one, the same way on both engines; it never writes. Each call walks the folder
//! afresh, so an edit shows at the next call.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::kit::parse_frontmatter;

/// The extensions of a document, compared without regard to case.
pub const EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "rst", "adoc", "org", "json", "yaml", "yml", "toml", "csv",
];
/// The most documents a collection lists and searches.
pub const MAX_DOCUMENTS: usize = 2_000;
/// A larger document is not read.
pub const MAX_DOCUMENT_BYTES: u64 = 10 * 1024 * 1024;
/// A search stops after reading this much.
pub const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
/// A tool result is capped at this many bytes.
pub const MAX_OUTPUT: usize = 30 * 1024;
/// The most collections the prompt lists.
pub const MAX_PROMPT_COLLECTIONS: usize = 50;
/// The longest summary and matching line shown, in characters.
const MAX_LINE: usize = 200;
const MAX_RESULTS: usize = 10;
const MAX_TERMS: usize = 10;
const LINES_PER_RESULT: usize = 3;
/// How much of a document the list reads for its title and summary.
const HEAD_BYTES: u64 = 8 * 1024;

/// What the tool tells the model it does.
pub const DESCRIPTION: &str = "Read the shared context the person keeps for dots: list a \
    collection's documents, search them with `query`, or read one with `path`.";

/// The arguments of the `context` tool, as JSON Schema properties; only `collection` is
/// required.
pub fn properties() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    json!({
        "collection": s("The collection's name, as the prompt lists it"),
        "query": s("Words to search for; a document must have every word"),
        "path": s("A document to read, as the list or a search names it"),
        "offset": {"type": "integer", "description": "With path: the first line to return, 1-based"}
    })
}

/// A collection of a run: its Library name and folder, with the count of documents found when
/// the run opened its tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub dir: PathBuf,
    pub description: String,
    pub documents: usize,
}

/// A document as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    /// Relative to the folder, with `/`.
    pub path: String,
    pub title: String,
    pub summary: String,
}

/// The documents below a folder: the first `MAX_DOCUMENTS` paths in sorted order, and how many
/// there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk {
    pub paths: Vec<String>,
    pub total: usize,
}

/// Whether `name` (a file name) has a document's extension.
fn is_document_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The documents below `root`. Names starting with `.` and links are left out, files and
/// folders alike. `Err` when `root` cannot be read as a folder.
pub fn walk(root: &Path) -> Result<Walk, String> {
    let mut found = Vec::new();
    let mut folders = vec![(root.to_path_buf(), String::new())];
    let mut first = true;
    while let Some((folder, prefix)) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if first => return Err(format!("the folder cannot be read: {e}")),
            Err(_) => continue,
        };
        first = false;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                folders.push((entry.path(), format!("{rel}/")));
            } else if kind.is_file() && is_document_name(&name) {
                found.push(rel);
            }
        }
    }
    found.sort();
    let total = found.len();
    found.truncate(MAX_DOCUMENTS);
    Ok(Walk {
        paths: found,
        total,
    })
}

/// `text` cut at `max` characters, with `…` when it was cut.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The title and summary of the document at `path` (relative, with `/`) whose text is `text`:
/// the frontmatter's `title`, else the first `# ` heading, else the file name without its
/// extension; the frontmatter's `description`, else the first non-empty line that is not a
/// heading, cut at 200 characters.
pub fn describe(path: &str, text: &str) -> Document {
    let front = parse_frontmatter(text);
    let field = |key: &str| {
        front
            .fields
            .get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let lines = || front.body.lines().map(str::trim);
    let title = field("title")
        .or_else(|| {
            lines()
                .find_map(|l| l.strip_prefix("# "))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
        .unwrap_or_else(|| {
            let name = path.rsplit('/').next().unwrap_or(path);
            Path::new(name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string())
        });
    let summary = field("description")
        .or_else(|| {
            lines()
                .find(|l| !l.is_empty() && !l.starts_with('#') && *l != "---")
                .map(str::to_string)
        })
        .unwrap_or_default();
    Document {
        path: path.to_string(),
        title: clip(&title, MAX_LINE),
        summary: clip(&summary, MAX_LINE),
    }
}

/// The joined path of `rel` (relative, with `/`) below `root`.
fn joined(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// At most `limit` bytes of the file at `path`, with bytes that are not UTF-8 replaced.
fn read_head(path: &Path, limit: u64) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The documents of `root`, described from the first 8 KB of each, with the walk's total.
pub fn documents(root: &Path, limit: usize) -> Result<(Vec<Document>, usize), String> {
    let walk = walk(root)?;
    let docs = walk
        .paths
        .iter()
        .take(limit)
        .map(|rel| {
            let text = read_head(&joined(root, rel), HEAD_BYTES).unwrap_or_default();
            describe(rel, &text)
        })
        .collect();
    Ok((docs, walk.total))
}

/// Adds `line` to `out` unless it would pass `MAX_OUTPUT`; false when it did not fit.
fn push_line(out: &mut String, line: &str) -> bool {
    if out.len() + line.len() + 1 > MAX_OUTPUT {
        return false;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
    true
}

/// The list of the collection's documents, one `path — title: summary` line each.
pub fn list(root: &Path) -> Result<String, String> {
    let (docs, total) = documents(root, MAX_DOCUMENTS)?;
    if docs.is_empty() {
        return Ok("The collection has no documents.".into());
    }
    let mut out = String::new();
    let mut shown = 0;
    for doc in &docs {
        let line = match doc.summary.as_str() {
            "" => format!("- {} — {}", doc.path, doc.title),
            summary => format!("- {} — {}: {summary}", doc.path, doc.title),
        };
        // Room is kept for the closing lines.
        if out.len() + line.len() + 200 > MAX_OUTPUT {
            break;
        }
        push_line(&mut out, &line);
        shown += 1;
    }
    if shown < docs.len() {
        out.push_str(&format!(
            "\n[{} more documents are not listed; use query to search them]",
            docs.len() - shown
        ));
    }
    if total > docs.len() {
        out.push_str(&format!(
            "\n[the collection has {total} documents; only the first {MAX_DOCUMENTS} are \
             listed and searched]"
        ));
    }
    Ok(out)
}

/// The search terms of `query`: lower case, at least 2 characters, each once, at most 10.
fn terms(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| t.chars().count() >= 2 && seen.insert(t.clone()))
        .take(MAX_TERMS)
        .collect()
}

struct Hit {
    doc: Document,
    count: usize,
    lines: Vec<String>,
}

/// The documents that have every word of `query` in their path, title or text, ranked by the
/// count of hits, then by path, with up to 3 matching lines each.
pub fn search(root: &Path, query: &str) -> Result<String, String> {
    let terms = terms(query);
    if terms.is_empty() {
        return Err("query needs a word of at least 2 characters".into());
    }
    let walk = walk(root)?;
    let mut read: u64 = 0;
    let mut stopped = false;
    let mut hits = Vec::new();
    for rel in &walk.paths {
        let path = joined(root, rel);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > MAX_DOCUMENT_BYTES {
            continue;
        }
        if read + meta.len() > MAX_SEARCH_BYTES {
            stopped = true;
            break;
        }
        read += meta.len();
        let Ok(text) = read_head(&path, MAX_DOCUMENT_BYTES) else {
            continue;
        };
        let doc = describe(rel, &text);
        let lower = text.to_lowercase();
        let known = format!("{} {}", rel.to_lowercase(), doc.title.to_lowercase());
        if !terms
            .iter()
            .all(|t| lower.contains(t.as_str()) || known.contains(t.as_str()))
        {
            continue;
        }
        let count = terms
            .iter()
            .map(|t| lower.matches(t.as_str()).count() + known.matches(t.as_str()).count())
            .sum();
        let lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                let l = l.to_lowercase();
                terms.iter().any(|t| l.contains(t.as_str()))
            })
            .take(LINES_PER_RESULT)
            .map(|(i, l)| format!("  line {}: {}", i + 1, clip(l.trim(), MAX_LINE)))
            .collect();
        hits.push(Hit { doc, count, lines });
    }
    hits.sort_by(|a, b| b.count.cmp(&a.count).then(a.doc.path.cmp(&b.doc.path)));
    let mut out = if hits.is_empty() {
        format!("No document matches {query:?}.")
    } else if hits.len() > MAX_RESULTS {
        format!(
            "{} documents match {query:?}; the {MAX_RESULTS} best:",
            hits.len()
        )
    } else {
        format!("{} documents match {query:?}:", hits.len())
    };
    for hit in hits.iter().take(MAX_RESULTS) {
        let mut block = format!("\n{} — {}", hit.doc.path, hit.doc.title);
        for line in &hit.lines {
            block.push('\n');
            block.push_str(line);
        }
        if !push_line(&mut out, &block) {
            break;
        }
    }
    if stopped {
        out.push_str("\n[the search stopped after 64 MB; results may be incomplete]");
    }
    Ok(out)
}

/// The document `rel` of the collection at `root`, from line `offset` (1-based), capped at
/// `MAX_OUTPUT`. It must name a document inside the folder: a path that is absolute, holds
/// `..` or a `:`, leads out of the folder through a link, or is hidden is refused, and so is a
/// file that is not a document.
pub fn read(root: &Path, rel: &str, offset: usize) -> Result<String, String> {
    let outside = || format!("{rel} is not inside the collection");
    let given = Path::new(rel);
    let plain = given.components().all(|c| match c {
        Component::Normal(n) => {
            let n = n.to_string_lossy();
            !n.contains(':') && !n.starts_with('.')
        }
        Component::CurDir => true,
        _ => false,
    });
    if !plain || rel.trim().is_empty() {
        return Err(outside());
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|e| format!("the folder cannot be read: {e}"))?;
    let path = std::fs::canonicalize(canonical_root.join(given))
        .map_err(|_| format!("{rel} was not found in the collection"))?;
    if !path.starts_with(&canonical_root) {
        return Err(outside());
    }
    if path.is_dir() {
        return Err(format!("{rel} is a folder; name a document in it"));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    if !name.as_deref().is_some_and(is_document_name) {
        return Err(format!("{rel} is not a document of the collection"));
    }
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("{rel} could not be read: {e}"))?
        .len();
    if size > MAX_DOCUMENT_BYTES {
        return Err(format!("{rel} is larger than 10 MB"));
    }
    let text = read_head(&path, MAX_DOCUMENT_BYTES)
        .map_err(|e| format!("{rel} could not be read: {e}"))?;
    let lines: Vec<&str> = text.lines().collect();
    if offset > 1 && offset > lines.len() {
        return Err(format!(
            "{rel} has {} lines; offset {offset} is past the end",
            lines.len()
        ));
    }
    let mut out = String::new();
    let mut next = None;
    for (i, line) in lines.iter().enumerate().skip(offset.saturating_sub(1)) {
        // Room is kept for the closing line.
        if out.len() + line.len() + 80 > MAX_OUTPUT {
            if out.is_empty() {
                // One line longer than a result: its start, and the next call goes past it.
                let mut cut = MAX_OUTPUT - 80;
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.push_str(&line[..cut]);
                next = (i + 2 <= lines.len()).then_some(i + 2);
                if next.is_none() {
                    out.push_str("\n[truncated: the rest of this line is not shown]");
                }
            } else {
                next = Some(i + 1);
            }
            break;
        }
        if i + 1 > offset.max(1) {
            out.push('\n');
        }
        out.push_str(line);
    }
    if let Some(next) = next {
        out.push_str(&format!("\n[truncated: continue with offset={next}]"));
    }
    Ok(out)
}

/// Runs a `context` call against `collections`: the list without `query` and `path`, a
/// search with `query`, a document with `path`. Blocking: it reads the folder.
pub fn call(collections: &[Collection], input: &Value) -> Result<String, String> {
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let name = text("collection").ok_or("collection is required")?;
    let Some(collection) = collections.iter().find(|c| c.name == name) else {
        let names: Vec<&str> = collections.iter().map(|c| c.name.as_str()).collect();
        return Err(if names.is_empty() {
            format!("unknown collection {name:?}. This run has no collections.")
        } else {
            format!(
                "unknown collection {name:?}. Collections: {}",
                names.join(", ")
            )
        });
    };
    let offset = match input.get("offset") {
        None | Some(Value::Null) => 1,
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => usize::try_from(n).unwrap_or(usize::MAX),
            _ => return Err("offset must be a whole number of at least 1".into()),
        },
    };
    if !collection.dir.is_dir() {
        return Err(format!("the folder of collection {name} is missing"));
    }
    match (text("query"), text("path")) {
        (Some(_), Some(_)) => {
            Err("give either query (to search) or path (to read a document), not both".into())
        }
        (Some(query), None) => search(&collection.dir, query),
        (None, Some(path)) => read(&collection.dir, path, offset),
        (None, None) => list(&collection.dir),
    }
}

/// The prompt's section that lists `collections` for the tool named `tool`; empty without
/// collections.
pub fn prompt_section(collections: &[Collection], tool: &str) -> String {
    if collections.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "Shared context you can read. Call the tool `{tool}` with a collection name to list its \
         documents, `query` to search it, or `path` to read one. Look here before you guess \
         how the person works."
    );
    for c in collections.iter().take(MAX_PROMPT_COLLECTIONS) {
        let description = match c.description.trim() {
            "" => "(no description)",
            d => d,
        };
        let count = match c.documents {
            1 => "1 document".to_string(),
            n => format!("{n} documents"),
        };
        out.push_str(&format!("\n- {}: {description} ({count})", c.name));
    }
    out
}
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod approvals;
pub mod engine;
```

**with:**

```rust
pub mod approvals;
pub mod context;
pub mod engine;
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test context_test`
Expected: PASS: 17 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 630 passed, `dots-app` 66; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/context.rs crates/dots-core/src/lib.rs crates/dots-core/tests/context_test.rs
git commit -m "feat(core): list, search and read the documents of a context collection"
```

### Task 3: Keep context collections in the Library and describe their documents

The Library gains its fourth kind. A collection needs an existing folder to be added or changed; the list gives its document count or the problem; `context_documents` describes the first 200 documents for the Library screen. Dots tick collections in `uses.context`, which the store checks and cleans like the other kinds. The UI's types, the command wrapper and the fixture follow; the screens come in Tasks 6 and 7.

**Files:**

- Modify: `apps/dots-app/src/commands.rs`
- Modify: `apps/dots-app/src/contract.rs`
- Modify: `apps/dots-app/src/lib.rs`
- Modify: `crates/dots-core/src/library.rs`
- Modify: `crates/dots-core/src/library_service.rs`
- Modify: `crates/dots-core/src/store/library.rs`
- Modify: `crates/dots-core/tests/library_service_test.rs`
- Modify: `crates/dots-core/tests/library_test.rs`
- Modify: `crates/dots-core/tests/run_tools_test.rs`
- Modify: `crates/dots-core/tests/store_library_test.rs`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Modify: `ui/src/api/commands.test.ts`
- Modify: `ui/src/api/commands.ts`
- Modify: `ui/src/api/contract.test.ts`
- Modify: `ui/src/api/types.ts`
- Modify: `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`
- Modify: `ui/src/screens/library/parts.tsx`

**Interfaces:**

- Consumes: Task 2's `context::walk`, `context::documents` and `context::Document`.
- Produces: `LibraryKind::Context` (`"context"`, label "context collection", uses key `context`); `ContextConfig { path: String, description: String, every_dot: bool }` (the last two `#[serde(default)]`); `LibraryConfig::Context`; `Uses.context: Vec<String>`; `library::MAX_COLLECTION_NAME = 40`, `MAX_COLLECTION_DESCRIPTION = 300`; `LibraryEntry.context: Option<ContextDetail { documents: usize }>`; `ContextDocuments { documents: Vec<Document>, total: usize, truncated: bool }`, `library_service::SHOWN_DOCUMENTS = 200`, `LibraryService::context_documents(&self, id: &str) -> Result<ContextDocuments>`; the app command `context_documents(id)`; in the UI, `LibraryKind` with `"context"`, `ContextConfig`, `ContextDetail`, `ContextDocument`, `ContextDocuments`, `Uses.context`, `commands.contextDocuments(id)`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/library_service_test.rs`, replace:**

```rust
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, McpServerConfig, PluginConfig, SkillConfig,
};
```

**with:**

```rust
use dots_core::library::{
    CatalogStatus, ContextConfig, LibraryConfig, LibraryKind, McpServerConfig, PluginConfig,
    SkillConfig,
};
```

**Append to `crates/dots-core/tests/library_service_test.rs`:**

```rust

fn collection(path: &str, every_dot: bool) -> LibraryConfig {
    LibraryConfig::Context(ContextConfig {
        path: path.into(),
        description: "How we work".into(),
        every_dot,
    })
}

#[tokio::test]
async fn a_collection_needs_a_folder_and_lists_its_document_count() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let missing = dir.path().join("nope").to_string_lossy().into_owned();
    let err = lib
        .add("company", collection(&missing, false), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
    let kb = dir.path().join("kb");
    write(
        &kb.join("style.md"),
        "# Style
Short sentences.",
    );
    write(&kb.join("people/ana.md"), "# Ana");
    write(&kb.join("logo.png"), "png");
    let path = kb.to_string_lossy().into_owned();
    let item = lib
        .add("company", collection(&path, false), None)
        .await
        .unwrap();
    let mut s = spec("nightly");
    s.uses.context = vec!["company".into()];
    store.create_dot(&s).await.unwrap();
    let entry = lib
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.item.id == item.id)
        .unwrap();
    assert_eq!(entry.context.unwrap().documents, 2);
    assert_eq!(entry.used_by.len(), 1);
    assert_eq!(entry.problem, None);
    // The folder goes away: the list says so.
    std::fs::remove_dir_all(&kb).unwrap();
    let entry = lib
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.item.id == item.id)
        .unwrap();
    assert_eq!(entry.context, None);
    assert_eq!(entry.problem.as_deref(), Some("folder not found"));
    // An update to a folder that does not exist is refused.
    let err = lib
        .update(&item.id, collection(&missing, true))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
}

#[tokio::test]
async fn removing_a_collection_takes_it_out_of_the_dots_that_tick_it() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let kb = dir.path().join("kb");
    std::fs::create_dir_all(&kb).unwrap();
    let item = lib
        .add("company", collection(&kb.to_string_lossy(), false), None)
        .await
        .unwrap();
    let mut s = spec("nightly");
    s.uses.context = vec!["company".into()];
    s.uses.skills = Vec::new();
    store.create_dot(&s).await.unwrap();
    let mut unknown = spec("audit");
    unknown.uses.context = vec!["clients".into()];
    let err = store.create_dot(&unknown).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("the Library has no context collection named \"clients\""),
        "{err}"
    );
    assert!(matches!(
        lib.remove(&item.id, false).await,
        Err(Error::Conflict(_))
    ));
    lib.remove(&item.id, true).await.unwrap();
    let dots = store.list_dots().await.unwrap();
    assert!(dots[0].spec.uses.context.is_empty());
}

#[tokio::test]
async fn the_documents_of_a_collection_are_described_up_to_200() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let kb = dir.path().join("kb");
    for i in 0..205 {
        write(
            &kb.join(format!("d{i:03}.md")),
            &format!(
                "---
title: Doc {i}
---
About {i}."
            ),
        );
    }
    let item = lib
        .add("company", collection(&kb.to_string_lossy(), false), None)
        .await
        .unwrap();
    let docs = lib.context_documents(&item.id).await.unwrap();
    assert_eq!(docs.documents.len(), 200);
    assert_eq!(docs.total, 205);
    assert!(docs.truncated);
    assert_eq!(docs.documents[0].path, "d000.md");
    assert_eq!(docs.documents[0].title, "Doc 0");
    assert_eq!(docs.documents[0].summary, "About 0.");
    let skill_path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig { path: skill_path }),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        lib.context_documents(&skill.id).await,
        Err(Error::Invalid(_))
    ));
    std::fs::remove_dir_all(&kb).unwrap();
    assert!(matches!(
        lib.context_documents(&item.id).await,
        Err(Error::NotFound(_))
    ));
}
```

**In `crates/dots-core/tests/library_test.rs`, replace:**

```rust
use dots_core::library::{
    check_name, looks_secret, mcp_server_name, plugin_server_name, CatalogStatus, LibraryConfig,
    LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig, SkillConfig, Uses,
};
```

**with:**

```rust
use dots_core::library::{
    check_name, looks_secret, mcp_server_name, plugin_server_name, CatalogStatus, ContextConfig,
    LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
    SkillConfig, Uses,
};
```

**In `crates/dots-core/tests/library_test.rs`, replace:**

```rust
        serde_json::to_value(&uses).unwrap(),
        json!({"mcp_servers": [], "skills": [], "plugins": []})
    );
}
```

**with:**

```rust
        serde_json::to_value(&uses).unwrap(),
        json!({"mcp_servers": [], "skills": [], "plugins": [], "context": []})
    );
    // A dot stored before collections existed reads with none.
    let old: Uses =
        serde_json::from_value(json!({"mcp_servers": ["sf"], "skills": [], "plugins": []}))
            .unwrap();
    assert!(old.context.is_empty());
}
```

**Append to `crates/dots-core/tests/library_test.rs`:**

```rust

#[test]
fn a_context_collection_name_is_short_and_lower_case() {
    for good in ["company", "a", "kb-2", "on_call", "9lives", &"x".repeat(40)] {
        assert!(check_name(LibraryKind::Context, good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "Company",
        "-kb",
        "_kb",
        "kb.notes",
        "kb notes",
        "kb/x",
        &"x".repeat(41),
    ] {
        assert!(invalid(check_name(LibraryKind::Context, bad)), "{bad:?}");
    }
    assert_eq!(LibraryKind::Context.as_str(), "context");
    assert_eq!(LibraryKind::Context.uses_key(), "context");
    assert_eq!(LibraryKind::Context.label(), "context collection");
}

#[test]
fn a_context_config_reads_as_the_spec_writes_it_and_is_checked() {
    let item: LibraryItem = serde_json::from_value(json!({
        "id": "01", "name": "company", "kind": "context",
        "config": {"path": "C:/notes/company"},
        "owner_id": null, "created_at": "t", "updated_at": "t"
    }))
    .unwrap();
    assert_eq!(item.kind(), LibraryKind::Context);
    assert_eq!(
        item.config,
        LibraryConfig::Context(ContextConfig {
            path: "C:/notes/company".into(),
            description: String::new(),
            every_dot: false,
        })
    );
    let with = |path: &str, description: String| {
        LibraryConfig::Context(ContextConfig {
            path: path.into(),
            description,
            every_dot: true,
        })
        .validate()
    };
    assert!(with("C:/kb", "é".repeat(300)).is_ok());
    assert!(invalid(with("  ", String::new())));
    assert!(invalid(with("C:/kb", "é".repeat(301))));
}

#[test]
fn uses_name_each_collection_once() {
    let uses = Uses {
        context: vec!["company".into(), "company".into()],
        ..Uses::default()
    };
    assert!(invalid(uses.validate()));
    let bad = Uses {
        context: vec!["Company".into()],
        ..Uses::default()
    };
    assert!(invalid(bad.validate()));
    assert!(!Uses {
        context: vec!["company".into()],
        ..Uses::default()
    }
    .is_empty());
}
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
        skills: vec![],
        plugins: vec!["forms".into()],
    };
    let (tools, notices) = f.open(&uses, None).await;
```

**with:**

```rust
        skills: vec![],
        plugins: vec!["forms".into()],
        context: vec![],
    };
    let (tools, notices) = f.open(&uses, None).await;
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
        plugins: vec!["forms".into()],
    };
```

**with:**

```rust
        plugins: vec!["forms".into()],
        context: vec![],
    };
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
        plugins: v(plugins),
    }
```

**with:**

```rust
        plugins: v(plugins),
        context: Vec::new(),
    }
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
    expect(await commands.libraryScanSkills("C:/skills")).toEqual(fixture.ScannedSkill);
    reply = async () => fixture.InstalledPlugin;
```

**with:**

```ts
    expect(await commands.libraryScanSkills("C:/skills")).toEqual(fixture.ScannedSkill);
    reply = async () => fixture.ContextDocuments;
    expect(await commands.contextDocuments("i1")).toEqual(fixture.ContextDocuments);
    reply = async () => fixture.InstalledPlugin;
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
      ["library_scan_skills", { path: "C:/skills" }],
      ["installed_claude_plugins", undefined],
```

**with:**

```ts
      ["library_scan_skills", { path: "C:/skills" }],
      ["context_documents", { id: "i1" }],
      ["installed_claude_plugins", undefined],
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  LibraryEntry: T.LibraryEntry.array(),
  McpCatalog: T.McpCatalog,
```

**with:**

```ts
  LibraryEntry: T.LibraryEntry.array(),
  ContextDocuments: T.ContextDocuments,
  McpCatalog: T.McpCatalog,
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
const [github, pluginServer, pdf, pdfTools] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [] };

```

**with:**

```tsx
const [github, pluginServer, pdf, pdfTools] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [] };

```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    await user.click(screen.getByLabelText("pdf"));
    expect(uses()).toEqual({ mcp_servers: ["github"], skills: [], plugins: ["pdf-tools"] });
  });
```

**with:**

```tsx
    await user.click(screen.getByLabelText("pdf"));
    expect(uses()).toEqual({
      mcp_servers: ["github"],
      skills: [],
      plugins: ["pdf-tools"],
      context: [],
    });
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test library_test --test library_service_test --test store_library_test --test run_tools_test`
Expected: FAIL to compile: `ContextConfig`, `LibraryKind::Context` and `Uses.context` do not exist.

- [ ] **Step 3: Add the kind, its store and service parts, the command and the UI types**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use dots_core::library::{LibraryConfig, LibraryItem, McpCatalog, McpServerConfig};
use dots_core::library_service::{EnvVariable, LibraryEntry, McpProbe, ScannedSkill};
use dots_core::model::{
```

**with:**

```rust
use dots_core::library::{LibraryConfig, LibraryItem, McpCatalog, McpServerConfig};
use dots_core::library_service::{
    ContextDocuments, EnvVariable, LibraryEntry, McpProbe, ScannedSkill,
};
use dots_core::model::{
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
}

#[tauri::command]
pub fn installed_claude_plugins(state: State<'_, AppState>) -> Res<Vec<InstalledPlugin>> {
```

**with:**

```rust
}

/// The first documents of a context collection, with their titles and summaries.
#[tauri::command]
pub async fn context_documents(state: State<'_, AppState>, id: String) -> Res<ContextDocuments> {
    Ok(state.rt()?.library().context_documents(&id).await?)
}

#[tauri::command]
pub fn installed_claude_plugins(state: State<'_, AppState>) -> Res<Vec<InstalledPlugin>> {
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust

use dots_core::engine::EngineEvent;
```

**with:**

```rust

use dots_core::context::Document;
use dots_core::engine::EngineEvent;
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig,
    PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::{
    EnvVariable, LibraryEntry, McpProbe, PluginDetail, ScannedSkill, SkillDetail,
};
```

**with:**

```rust
use dots_core::library::{
    CatalogStatus, ContextConfig, LibraryConfig, LibraryItem, LibraryKind, McpCatalog,
    McpServerConfig, PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::{
    ContextDetail, ContextDocuments, EnvVariable, LibraryEntry, McpProbe, PluginDetail,
    ScannedSkill, SkillDetail,
};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        plugins: vec!["pdf-tools".into()],
    };
```

**with:**

```rust
        plugins: vec!["pdf-tools".into()],
        context: vec!["company".into()],
    };
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust

/// A config of every kind and transport, as `library_add` takes them.
```

**with:**

```rust

fn context_config() -> LibraryConfig {
    LibraryConfig::Context(ContextConfig {
        path: "C:/notes/company".into(),
        description: "How we work: style, people, runbooks".into(),
        every_dot: true,
    })
}

/// A config of every kind and transport, as `library_add` takes them.
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        plugin_config(),
    ]
```

**with:**

```rust
        plugin_config(),
        context_config(),
    ]
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        plugin: None,
        problem: None,
```

**with:**

```rust
        plugin: None,
        context: None,
        problem: None,
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    let plugin = library_item("01J0PLG", "pdf-tools", plugin_config());
    vec![
```

**with:**

```rust
    let plugin = library_item("01J0PLG", "pdf-tools", plugin_config());
    let company = library_item("01J0CTX", "company", context_config());
    vec![
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            }),
            ..bare(plugin, users)
        },
    ]
}
```

**with:**

```rust
            }),
            ..bare(plugin, users.clone())
        },
        LibraryEntry {
            context: Some(ContextDetail { documents: 42 }),
            ..bare(company, users)
        },
    ]
}

/// A collection's first documents, more of them left out.
fn context_documents() -> ContextDocuments {
    ContextDocuments {
        documents: vec![Document {
            path: "people/ana.md".into(),
            title: "Ana Lima".into(),
            summary: "Owns billing".into(),
        }],
        total: 42,
        truncated: true,
    }
}
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    };
    json!({
        "DotSpec": to(local_spec()),
```

**with:**

```rust
    };
    let mut samples = json!({
        "DotSpec": to(local_spec()),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        }),
    })
}
```

**with:**

```rust
        }),
    });
    // Past the `json!` macro's recursion limit, so added here.
    samples["ContextDocuments"] = to(context_documents());
    samples
}
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    library_scan_skills,
    installed_claude_plugins,
```

**with:**

```rust
    library_scan_skills,
    context_documents,
    installed_claude_plugins,
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
    Plugin => "plugin",
});
```

**with:**

```rust
    Plugin => "plugin",
    Context => "context",
});
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
            Self::Plugin => "plugin",
        }
```

**with:**

```rust
            Self::Plugin => "plugin",
            Self::Context => "context collection",
        }
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
            Self::Plugin => "plugins",
        }
```

**with:**

```rust
            Self::Plugin => "plugins",
            Self::Context => "context",
        }
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
pub const MAX_ITEM_NAME: usize = 64;

```

**with:**

```rust
pub const MAX_ITEM_NAME: usize = 64;
/// The longest context collection name: the model passes it to the `context` tool.
pub const MAX_COLLECTION_NAME: usize = 40;
/// The longest description of a context collection, in characters.
pub const MAX_COLLECTION_DESCRIPTION: usize = 300;

```

**In `crates/dots-core/src/library.rs`, replace:**

```rust

/// An item's config, with its kind: `{"kind": "mcp_server", "config": {...}}`.
```

**with:**

```rust

/// A folder of documents that dots read with the `context` tool (context library spec
/// section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextConfig {
    pub path: String,
    #[serde(default)]
    pub description: String,
    /// Every dot uses the collection, ticked or not.
    #[serde(default)]
    pub every_dot: bool,
}

/// An item's config, with its kind: `{"kind": "mcp_server", "config": {...}}`.
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
    Plugin(PluginConfig),
}
```

**with:**

```rust
    Plugin(PluginConfig),
    Context(ContextConfig),
}
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
    pub plugins: Vec<String>,
}
```

**with:**

```rust
    pub plugins: Vec<String>,
    /// Context collections; one whose `every_dot` is on is used without being named.
    pub context: Vec<String>,
}
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
    pub fn is_empty(&self) -> bool {
        self.mcp_servers.is_empty() && self.skills.is_empty() && self.plugins.is_empty()
    }
```

**with:**

```rust
    pub fn is_empty(&self) -> bool {
        self.mcp_servers.is_empty()
            && self.skills.is_empty()
            && self.plugins.is_empty()
            && self.context.is_empty()
    }
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
            LibraryKind::Plugin => &self.plugins,
        }
```

**with:**

```rust
            LibraryKind::Plugin => &self.plugins,
            LibraryKind::Context => &self.context,
        }
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
            }
        }
    }
    Ok(())
```

**with:**

```rust
            }
        }
        LibraryKind::Context => {
            let mut chars = name.chars();
            let first_ok = chars
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
            let rest_ok =
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if !first_ok || !rest_ok || name.len() > MAX_COLLECTION_NAME {
                return invalid(format!(
                    "context collection name {name:?}: use 1 to {MAX_COLLECTION_NAME} characters \
                     of a-z, 0-9, '-' or '_', starting with a letter or digit"
                ));
            }
        }
    }
    Ok(())
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
            Self::Plugin(_) => LibraryKind::Plugin,
        }
```

**with:**

```rust
            Self::Plugin(_) => LibraryKind::Plugin,
            Self::Context(_) => LibraryKind::Context,
        }
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
            Self::Plugin(c) if c.path.trim().is_empty() => invalid("a plugin needs its folder"),
            _ => Ok(()),
```

**with:**

```rust
            Self::Plugin(c) if c.path.trim().is_empty() => invalid("a plugin needs its folder"),
            Self::Context(c) if c.path.trim().is_empty() => {
                invalid("a context collection needs its folder")
            }
            Self::Context(c) if c.description.chars().count() > MAX_COLLECTION_DESCRIPTION => {
                invalid(format!(
                    "a context collection's description is at most {MAX_COLLECTION_DESCRIPTION} \
                     characters"
                ))
            }
            _ => Ok(()),
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust

use crate::events::{Bus, RuntimeEvent};
```

**with:**

```rust

use crate::context::{self, Document};
use crate::events::{Bus, RuntimeEvent};
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    pub plugin: Option<PluginDetail>,
    /// Why a skill or plugin folder could not be read ("folder not found").
    pub problem: Option<String>,
}

```

**with:**

```rust
    pub plugin: Option<PluginDetail>,
    /// A context collection read from its folder.
    pub context: Option<ContextDetail>,
    /// Why a skill, plugin or context folder could not be read ("folder not found").
    pub problem: Option<String>,
}

/// What a context collection's folder holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextDetail {
    pub documents: usize,
}

/// The first documents of a collection (`context_documents`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextDocuments {
    pub documents: Vec<Document>,
    /// All documents found, up to `context::MAX_DOCUMENTS` and past it.
    pub total: usize,
    /// More documents than are listed here.
    pub truncated: bool,
}

/// How many documents `context_documents` describes.
pub const SHOWN_DOCUMENTS: usize = 200;

```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        "SKILL.md not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

```

**with:**

```rust
        "SKILL.md not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

fn context_problem(dir: &Path, e: &str) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else {
        e.to_string()
    }
}

```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                LibraryConfig::McpServer(_) => (LibraryKind::McpServer, String::new()),
            })
```

**with:**

```rust
                LibraryConfig::McpServer(_) => (LibraryKind::McpServer, String::new()),
                LibraryConfig::Context(c) => (LibraryKind::Context, c.path.clone()),
            })
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                            kit::read_skill(&dir)
                                .map(|s| (Some(s), None))
                                .map_err(|e| skill_problem(&dir, &e)),
```

**with:**

```rust
                            kit::read_skill(&dir)
                                .map(|s| (Some(s), None, None))
                                .map_err(|e| skill_problem(&dir, &e)),
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                            kit::read_plugin(&dir)
                                .map(|p| (None, Some(p)))
                                .map_err(|e| plugin_problem(&dir, &e)),
                        ),
```

**with:**

```rust
                            kit::read_plugin(&dir)
                                .map(|p| (None, Some(p), None))
                                .map_err(|e| plugin_problem(&dir, &e)),
                        ),
                        LibraryKind::Context => Some(
                            context::walk(&dir)
                                .map(|w| (None, None, Some(w.total)))
                                .map_err(|e| context_problem(&dir, &e)),
                        ),
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                plugin: None,
                problem: None,
```

**with:**

```rust
                plugin: None,
                context: None,
                problem: None,
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
            match read {
                Some(Ok((skill, plugin))) => {
                    entry.skill = skill.map(|s| SkillDetail {
```

**with:**

```rust
            match read {
                Some(Ok((skill, plugin, documents))) => {
                    entry.context = documents.map(|documents| ContextDetail { documents });
                    entry.skill = skill.map(|s| SkillDetail {
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
            }
            LibraryConfig::McpServer(_) => Ok(None),
        }
    }
```

**with:**

```rust
            }
            LibraryConfig::Context(c) => {
                if !Path::new(&c.path).is_dir() {
                    return Err(Error::NotFound(format!(
                        "the folder {} was not found",
                        c.path
                    )));
                }
                Ok(None)
            }
            LibraryConfig::McpServer(_) => Ok(None),
        }
    }

    /// The first `SHOWN_DOCUMENTS` documents of the context collection `id`, each with its
    /// title and summary (`context_documents`).
    pub async fn context_documents(&self, id: &str) -> Result<ContextDocuments> {
        let item = self.store.get_library_item(id).await?;
        let LibraryConfig::Context(config) = &item.config else {
            return Err(Error::Invalid(format!(
                "the {} {:?} is not a context collection",
                item.kind().label(),
                item.name
            )));
        };
        let dir = PathBuf::from(&config.path);
        let (documents, total) = blocking(move || context::documents(&dir, SHOWN_DOCUMENTS))
            .await?
            .map_err(Error::NotFound)?;
        Ok(ContextDocuments {
            truncated: total > documents.len(),
            documents,
            total,
        })
    }
```

**In `crates/dots-core/src/store/library.rs`, replace:**

```rust
const ITEM_COLUMNS: &str = "id, kind, name, config, owner_id, created_at, updated_at";
/// Servers, then skills, then plugins; by name within a kind.
const ITEM_ORDER: &str = "CASE kind WHEN 'mcp_server' THEN 0 WHEN 'skill' THEN 1 ELSE 2 END, name";
const CATALOG_COLUMNS: &str = "server_id, status, accepted_fingerprint, accepted_tools, \
```

**with:**

```rust
const ITEM_COLUMNS: &str = "id, kind, name, config, owner_id, created_at, updated_at";
/// Servers, then skills, then plugins, then context collections; by name within a kind.
const ITEM_ORDER: &str = "CASE kind WHEN 'mcp_server' THEN 0 WHEN 'skill' THEN 1 \
    WHEN 'plugin' THEN 2 ELSE 3 END, name";
const CATALOG_COLUMNS: &str = "server_id, status, accepted_fingerprint, accepted_tools, \
```

**In `crates/dots-core/src/store/library.rs`, replace:**

```rust
        LibraryConfig::Plugin(c) => serde_json::to_string(c)?,
    })
```

**with:**

```rust
        LibraryConfig::Plugin(c) => serde_json::to_string(c)?,
        LibraryConfig::Context(c) => serde_json::to_string(c)?,
    })
```

**In `crates/dots-core/src/store/library.rs`, replace:**

```rust
                LibraryKind::Plugin => &mut uses.plugins,
            };
```

**with:**

```rust
                LibraryKind::Plugin => &mut uses.plugins,
                LibraryKind::Context => &mut uses.context,
            };
```

**In `ui/src/api/commands.ts`, replace:**

```ts
    call(T.ScannedSkill.array(), "library_scan_skills", { path }),
  installedClaudePlugins: () => call(T.InstalledPlugin.array(), "installed_claude_plugins"),
```

**with:**

```ts
    call(T.ScannedSkill.array(), "library_scan_skills", { path }),
  contextDocuments: (id: string) => call(T.ContextDocuments, "context_documents", { id }),
  installedClaudePlugins: () => call(T.InstalledPlugin.array(), "installed_claude_plugins"),
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const ScheduleStatus = z.enum(["active", "completed", "expired"]);
export const LibraryKind = z.enum(["mcp_server", "skill", "plugin"]);
// Where an MCP server's tool list stands: accepted, changed since, last check failed, or never
```

**with:**

```ts
export const ScheduleStatus = z.enum(["active", "completed", "expired"]);
export const LibraryKind = z.enum(["mcp_server", "skill", "plugin", "context"]);
// Where an MCP server's tool list stands: accepted, changed since, last check failed, or never
```

**In `ui/src/api/types.ts`, replace:**

```ts
  plugins: z.array(z.string()),
});
```

**with:**

```ts
  plugins: z.array(z.string()),
  context: z.array(z.string()),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
});
// An item's config with its kind: what `library_add` and `library_update` take.
```

**with:**

```ts
});
// A folder of documents dots read with the `context` tool (context library spec section 3).
export const ContextConfig = z.strictObject({
  path: z.string(),
  description: z.string(),
  // Every dot uses it, ticked or not.
  every_dot: z.boolean(),
});
// An item's config with its kind: what `library_add` and `library_update` take.
```

**In `ui/src/api/types.ts`, replace:**

```ts
  z.strictObject({ kind: z.literal("plugin"), config: PluginConfig }),
]);
```

**with:**

```ts
  z.strictObject({ kind: z.literal("plugin"), config: PluginConfig }),
  z.strictObject({ kind: z.literal("context"), config: ContextConfig }),
]);
```

**In `ui/src/api/types.ts`, replace:**

```ts
    }),
  ]);
```

**with:**

```ts
    }),
    z.strictObject({
      kind: z.literal("context"),
      config: ContextConfig,
      ...libraryItemShape,
      ...extra,
    }),
  ]);
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const SkillDetail = z.strictObject({ description: z.string() });
export const PluginDetail = z.strictObject({
```

**with:**

```ts
export const SkillDetail = z.strictObject({ description: z.string() });
export const ContextDetail = z.strictObject({ documents: z.number().int() });
export const PluginDetail = z.strictObject({
```

**In `ui/src/api/types.ts`, replace:**

```ts
  plugin: nullable(PluginDetail),
  // Why a skill or plugin folder could not be read.
  problem: nullable(z.string()),
});
```

**with:**

```ts
  plugin: nullable(PluginDetail),
  context: nullable(ContextDetail),
  // Why a skill, plugin or context folder could not be read.
  problem: nullable(z.string()),
});
export const ContextDocument = z.strictObject({
  path: z.string(),
  title: z.string(),
  summary: z.string(),
});
// A collection's first documents (`context_documents`); `total` counts them all.
export const ContextDocuments = z.strictObject({
  documents: z.array(ContextDocument),
  total: z.number().int(),
  truncated: z.boolean(),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type LibraryEntry = z.infer<typeof LibraryEntry>;
export type McpProbe = z.infer<typeof McpProbe>;
```

**with:**

```ts
export type LibraryEntry = z.infer<typeof LibraryEntry>;
export type ContextConfig = z.infer<typeof ContextConfig>;
export type ContextDocuments = z.infer<typeof ContextDocuments>;
export type McpProbe = z.infer<typeof McpProbe>;
```

**In `ui/src/screens/library/parts.tsx`, replace:**

```tsx

const KIND_WORD = { mcp_server: "server", skill: "skill", plugin: "plugin" } as const;

```

**with:**

```tsx

const KIND_WORD = {
  mcp_server: "server",
  skill: "skill",
  plugin: "plugin",
  context: "collection",
} as const;

```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the fixture changes only where this task's types changed.

**Generated:** `ui/src/api/__fixtures__/contract.json`

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test library_test --test library_service_test --test store_library_test --test run_tools_test`
Expected: PASS.

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 636 passed, `dots-app` 66, the UI 373 in 25 files; no warnings.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/contract.rs apps/dots-app/src/lib.rs crates/dots-core/src/library.rs crates/dots-core/src/library_service.rs crates/dots-core/src/store/library.rs crates/dots-core/tests/library_service_test.rs crates/dots-core/tests/library_test.rs crates/dots-core/tests/run_tools_test.rs crates/dots-core/tests/store_library_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.test.ts ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/screens/dot-editor/ToolsAndSkills.test.tsx ui/src/screens/library/parts.tsx
git commit -m "feat(core): keep context collections in the Library and describe their documents"
```

### Task 4: Give a run its context collections, list them in the prompt and read them with the local context tool

A run's tools resolve its collections (ticked, or for every dot) and count their documents; a missing folder is left out with a run event. `build_prompt` ends a first prompt with the `## Shared context` section, naming the tool as the engine's model sees it. Every preset allows `Context`; `cli_lists` keeps it from the CLI. Migration 0007's code step adds the rule to stored policies after 0006's, which is why several 0006 tests now expect `Context` after `Skill`. The local engine offers `context` only to a run with collections and runs it through the gate like `skill`.

**Files:**

- Modify: `crates/dots-core/migrations/0007_context.sql`
- Modify: `crates/dots-core/src/engine/local/mcp_tools.rs`
- Modify: `crates/dots-core/src/engine/local/mod.rs`
- Create: `crates/dots-core/src/engine/local/tools/context.rs`
- Modify: `crates/dots-core/src/engine/local/tools/mod.rs`
- Modify: `crates/dots-core/src/policy.rs`
- Modify: `crates/dots-core/src/prompt.rs`
- Modify: `crates/dots-core/src/run_tools.rs`
- Modify: `crates/dots-core/src/runner.rs`
- Modify: `crates/dots-core/src/store/library.rs`
- Modify: `crates/dots-core/src/store/mod.rs`
- Modify: `crates/dots-core/src/store/upgrade.rs`
- Modify: `crates/dots-core/tests/engine_prompt_test.rs`
- Create: `crates/dots-core/tests/local_context_test.rs`
- Modify: `crates/dots-core/tests/local_tools_test.rs`
- Modify: `crates/dots-core/tests/policy_test.rs`
- Modify: `crates/dots-core/tests/store_dots_test.rs`
- Modify: `crates/dots-core/tests/store_library_test.rs`
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**

- Consumes: Task 1's `build_prompt`; Task 2's `context::{Collection, walk, call, prompt_section, properties, DESCRIPTION}`; Task 3's `ContextConfig` and `Uses.context`.
- Produces: `Store::used_collections(&self, uses: &Uses) -> Result<Vec<LibraryItem>>`; `RunTools::contexts(&self) -> &[Collection]`; `prompt::context_tool_name(EngineKind) -> &'static str` (`mcp__dots__context` / `context`); `build_prompt(dot, run, workspace, now, global: &str, contexts: &[Collection]) -> String`; the presets' `Context` allow rule; `Policy::cli_lists` without `Context` rules; `engine::local::tools::context::{schema() -> Value, run(&[Collection], &Value) -> ToolOutput}` and `("context", "Context")` in `tools::TOOLS`; `store::upgrade::finish_0007` and the table `upgrade_0007`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
use common::{fake_dot, fake_run};
use dots_core::engine::scripted::{ScriptedEngine, Step};
```

**with:**

```rust
use common::{fake_dot, fake_run};
use dots_core::context::Collection;
use dots_core::engine::scripted::{ScriptedEngine, Step};
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
};
use dots_core::model::{ApplyState, TriggerKind, WorkspaceMode};
use dots_core::prompt::build_prompt;
use serde_json::{json, Value};
```

**with:**

```rust
};
use dots_core::model::{ApplyState, EngineKind, TriggerKind, WorkspaceMode};
use dots_core::prompt::{build_prompt, context_tool_name};
use serde_json::{json, Value};
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Manual, None, None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws/RUN1"), at(), "");
    assert!(p.starts_with("Do the thing."));
```

**with:**

```rust
    let run = fake_run(TriggerKind::Manual, None, None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws/RUN1"), at(), "", &[]);
    assert!(p.starts_with("Do the thing."));
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Webhook, Some(json!({"case": 42})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "");
    assert!(p.contains("```json\n{\n  \"case\": 42\n}\n```"));
```

**with:**

```rust
    let run = fake_run(TriggerKind::Webhook, Some(json!({"case": 42})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]);
    assert!(p.contains("```json\n{\n  \"case\": 42\n}\n```"));
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    assert_eq!(
        build_prompt(&dot, &resume, Path::new("C:/ws"), at(), ""),
        "Approval granted."
```

**with:**

```rust
    assert_eq!(
        build_prompt(&dot, &resume, Path::new("C:/ws"), at(), "", &[]),
        "Approval granted."
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    assert_eq!(
        build_prompt(&dot, &follow, Path::new("C:/ws"), at(), ""),
        "and now?"
```

**with:**

```rust
    assert_eq!(
        build_prompt(&dot, &follow, Path::new("C:/ws"), at(), "", &[]),
        "and now?"
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
        "\n  Use British spelling.  \n",
    );
```

**with:**

```rust
        "\n  Use British spelling.  \n",
        &[],
    );
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let chat = fake_run(TriggerKind::Chat, Some(json!({"message": "hi"})), None);
    let p = build_prompt(&dot, &chat, Path::new("C:/ws"), at(), "Be brief.");
    assert!(p.starts_with("## Instructions for every dot\n\nBe brief.\n\n"));
```

**with:**

```rust
    let chat = fake_run(TriggerKind::Chat, Some(json!({"message": "hi"})), None);
    let p = build_prompt(&dot, &chat, Path::new("C:/ws"), at(), "Be brief.", &[]);
    assert!(p.starts_with("## Instructions for every dot\n\nBe brief.\n\n"));
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Manual, None, None);
    let plain = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "");
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), " \n\t "),
        plain
```

**with:**

```rust
    let run = fake_run(TriggerKind::Manual, None, None);
    let plain = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]);
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), " \n\t ", &[]),
        plain
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), "Be brief."),
        plain
```

**with:**

```rust
    assert_eq!(
        build_prompt(&dot, &run, Path::new("C:/ws"), at(), "Be brief.", &[]),
        plain
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
        assert_eq!(
            build_prompt(&dot, &run, Path::new("C:/ws"), at(), "Be brief."),
            message
```

**with:**

```rust
        assert_eq!(
            build_prompt(&dot, &run, Path::new("C:/ws"), at(), "Be brief.", &[]),
            message
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
    let run = fake_run(TriggerKind::Chat, Some(json!({"message": "status?"})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "");
    assert!(p.starts_with("Do the thing."));
```

**with:**

```rust
    let run = fake_run(TriggerKind::Chat, Some(json!({"message": "status?"})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]);
    assert!(p.starts_with("Do the thing."));
```

**Append to `crates/dots-core/tests/engine_prompt_test.rs`:**

```rust

fn collections() -> Vec<Collection> {
    vec![Collection {
        name: "company".into(),
        dir: "C:/kb".into(),
        description: "How we work".into(),
        documents: 3,
    }]
}

#[test]
fn a_first_prompt_ends_with_the_collections_under_the_engines_tool_name() {
    let mut dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(TriggerKind::Manual, None, None);
    dot.spec.engine = EngineKind::Claude;
    let claude = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &collections());
    assert!(
        claude.ends_with(
            "what needs human review.\n\n## Shared context\n\nShared context you can read. \
             Call the tool `mcp__dots__context` with a collection name to list its documents, \
             `query` to search it, or `path` to read one. Look here before you guess how the \
             person works.\n- company: How we work (3 documents)\n"
        ),
        "{claude}"
    );
    dot.spec.engine = EngineKind::Local;
    let local = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &collections());
    assert!(local.contains("Call the tool `context` with"), "{local}");
    assert_eq!(context_tool_name(EngineKind::Local), "context");
    let resume = fake_run(
        TriggerKind::Resume,
        Some(json!({"message": "go"})),
        Some("s"),
    );
    assert_eq!(
        build_prompt(&dot, &resume, Path::new("C:/ws"), at(), "", &collections()),
        "go"
    );
    let none = build_prompt(&dot, &run, Path::new("C:/ws"), at(), "", &[]);
    assert!(!none.contains("Shared context"), "{none}");
}
```

**Create `crates/dots-core/tests/local_context_test.rs`:**

```rust
//! Context collections in a local-engine run (context library spec sections 3 to 5): the
//! first prompt lists them, the `context` tool reads them under the policy name `Context`, and
//! a run without collections is not offered the tool.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::tools::{self, context as context_tool};
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::library::{ContextConfig, LibraryConfig};
use dots_core::library_service::LibraryService;
use dots_core::model::{DotSpec, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Setup {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
    lib: LibraryService,
}

async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    std::fs::create_dir(dir.path().join("ws")).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
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
    let lib = LibraryService::new(store.clone(), bus);
    Setup {
        dir,
        store,
        runner,
        lib,
    }
}

impl Setup {
    /// Adds the collection `name` of the folder `folder` below the test's folder.
    async fn collection(&self, name: &str, folder: &str, every_dot: bool) {
        let path = self.dir.path().join(folder);
        std::fs::create_dir_all(&path).unwrap();
        let config = LibraryConfig::Context(ContextConfig {
            path: path.to_string_lossy().into_owned(),
            description: format!("About {name}"),
            every_dot,
        });
        self.lib.add(name, config, None).await.unwrap();
    }

    fn spec(&self, llm: &Llm) -> DotSpec {
        let mut s = spec("reader");
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
async fn a_local_run_lists_its_collections_and_reads_them_with_the_context_tool() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    t.collection("everyone", "all", true).await;
    t.collection("unused", "other", false).await;
    write(
        &t.dir.path().join("kb/style.md"),
        "# House style\nShort sentences.\nNo jargon.",
    );
    write(&t.dir.path().join("all/hello.md"), "# Hello");
    let llm = Llm::start(vec![
        call("c1", "context", json!({"collection": "company"})),
        call(
            "c2",
            "context",
            json!({"collection": "company", "path": "style.md", "offset": 2}),
        ),
        call(
            "c3",
            "context",
            json!({"collection": "company", "query": "jargon"}),
        ),
        text("Read it."),
    ])
    .await;
    let mut s = t.spec(&llm);
    // Every preset allows `Context`, Read-only too.
    s.policy = Policy::preset(Preset::ReadOnly);
    s.uses.context = vec!["company".into()];
    let run = t.run(&s, RunStatus::Succeeded).await;

    let reqs = llm.requests();
    let prompt = messages(&reqs[0])[1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        prompt.ends_with(
            "\n## Shared context\n\nShared context you can read. Call the tool `context` with a \
             collection name to list its documents, `query` to search it, or `path` to read \
             one. Look here before you guess how the person works.\n\
             - company: About company (1 document)\n\
             - everyone: About everyone (1 document)\n"
        ),
        "{prompt}"
    );
    let names = tool_names(&reqs[0]);
    assert!(names.iter().any(|n| n == "context"), "{names:?}");
    assert!(!names.iter().any(|n| n == "skill"), "{names:?}");
    assert_eq!(
        tool_result(&reqs[1], "c1"),
        "- style.md — House style: Short sentences."
    );
    assert_eq!(tool_result(&reqs[2], "c2"), "Short sentences.\nNo jargon.");
    assert!(
        tool_result(&reqs[3], "c3").contains("  line 3: No jargon."),
        "{}",
        tool_result(&reqs[3], "c3")
    );
    let calls: Vec<Value> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_call")
        .map(|ev| ev.data)
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["tool"], "Context");
    assert_eq!(calls[0]["input"], json!({"collection": "company"}));
}

#[tokio::test]
async fn a_run_without_collections_is_not_offered_the_tool() {
    let t = setup().await;
    let llm = Llm::start(vec![text("Nothing to read.")]).await;
    t.run(&t.spec(&llm), RunStatus::Succeeded).await;
    let reqs = llm.requests();
    assert!(!tool_names(&reqs[0]).iter().any(|n| n == "context"));
    let prompt = messages(&reqs[0])[1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!prompt.contains("Shared context"), "{prompt}");
}

#[tokio::test]
async fn a_missing_folder_leaves_the_collection_out_with_a_notice() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    std::fs::remove_dir_all(t.dir.path().join("kb")).unwrap();
    let llm = Llm::start(vec![text("Done.")]).await;
    let mut s = t.spec(&llm);
    s.uses.context = vec!["company".into()];
    let run = t.run(&s, RunStatus::Succeeded).await;
    let notices: Vec<String> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "notice")
        .map(|ev| ev.data["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        notices.contains(&"context company: folder not found".to_string()),
        "{notices:?}"
    );
    assert!(!tool_names(&llm.requests()[0])
        .iter()
        .any(|n| n == "context"));
}

#[tokio::test]
async fn a_dots_own_context_rule_still_decides() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    write(&t.dir.path().join("kb/a.md"), "secret plans");
    let llm = Llm::start(vec![
        call(
            "c1",
            "context",
            json!({"collection": "company", "path": "a.md"}),
        ),
        text("Could not read it."),
    ])
    .await;
    let mut s = t.spec(&llm);
    s.policy = Policy::preset(Preset::Sandboxed);
    s.policy.rules.insert(0, Rule::new("Context", Action::Deny));
    s.uses.context = vec!["company".into()];
    t.run(&s, RunStatus::Succeeded).await;
    let result = tool_result(&llm.requests()[1], "c1");
    assert!(!result.contains("secret plans"), "{result}");
    assert!(result.to_lowercase().contains("denied"), "{result}");
}

#[test]
fn context_is_a_local_tool_with_the_policy_name_context() {
    assert_eq!(tools::alias("context"), Some("Context"));
    assert_eq!(tools::local_name("Context"), Some("context"));
    let ws = tempfile::tempdir().unwrap();
    let prepared = tools::prepare(
        "context",
        &json!({"collection": "kb", "path": "a.md", "offset": 3, "extra": 1}),
        ws.path(),
    )
    .unwrap();
    assert_eq!(
        prepared.input,
        json!({"collection": "kb", "path": "a.md", "offset": 3})
    );
    let err = tools::prepare("context", &json!({"query": "x"}), ws.path()).unwrap_err();
    assert!(err.contains("'collection'"), "{err}");
    let err = tools::prepare(
        "context",
        &json!({"collection": "kb", "offset": "3"}),
        ws.path(),
    )
    .unwrap_err();
    assert!(err.contains("non-negative integer"), "{err}");
    let schema = context_tool::schema();
    assert_eq!(schema["function"]["name"], "context");
    assert_eq!(
        schema["function"]["parameters"]["required"],
        json!(["collection"])
    );
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    assert_eq!(tools::alias("grep"), Some("Grep"));
    // Every local tool but `skill`, which a run gets only with skills (`skill::schema`).
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len() - 1
    );
```

**with:**

```rust
    assert_eq!(tools::alias("grep"), Some("Grep"));
    // Every local tool but `skill` and `context`, which a run gets only with skills
    // (`skill::schema`) and context collections (`context::schema`).
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len() - 2
    );
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        ("skill", json!({"name": "forms:fill", "file": ""})),
    ];
```

**with:**

```rust
        ("skill", json!({"name": "forms:fill", "file": ""})),
        ("context", json!({"collection": "kb", "query": "style"})),
        (
            "context",
            json!({"collection": "kb", "path": "a.md", "offset": 2}),
        ),
    ];
```

**Append to `crates/dots-core/tests/policy_test.rs`:**

```rust

#[test]
fn every_preset_allows_reading_context_and_a_rule_still_wins() {
    for preset in [
        Preset::ReadOnly,
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::Custom,
    ] {
        let p = Policy::preset(preset);
        assert_eq!(
            p.resolve("Context", &json!({"collection": "kb"})),
            Action::Allow,
            "{preset:?}"
        );
        // The CLI has no `Context` tool: the rule stays out of its lists.
        let (allow, deny) = p.cli_lists();
        assert!(!allow.iter().chain(&deny).any(|t| t.starts_with("Context")));
    }
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.insert(0, Rule::new("Context", Action::Deny));
    assert_eq!(p.resolve("Context", &json!({})), Action::Deny);
    assert!(!p.cli_lists().1.iter().any(|t| t == "Context"));
}
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust

/// Migration 0005 on a database that Dots left with a cron string (Scheduler v2 spec §4).
```

**with:**

```rust

/// Migration 0007 on a database Dots left at 0006: every stored policy gets a `Context` allow
/// rule unless a rule names `Context`, and every dot starts with the global instructions.
#[tokio::test]
async fn migration_0007_lets_every_stored_policy_read_context() {
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
        .filter(|m| m.version < 7)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0006 left it: the preset's rules without `Context`.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| r.tool != "Context");
        p
    };
    let mut denies = old(Preset::Custom);
    denies.rules.push(dots_core::policy::Rule::new(
        "Context(kb)",
        dots_core::policy::Action::Deny,
    ));
    for (id, name, policy) in [
        ("D1", "sandboxed", old(Preset::Sandboxed)),
        ("D2", "read-only", old(Preset::ReadOnly)),
        ("D3", "denies", denies.clone()),
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
        let dot = store.get_dot(id).await.unwrap();
        assert_eq!(dot.spec.policy, Policy::preset(preset), "{id}");
        assert!(dot.spec.use_global_instructions);
        assert!(dot.spec.uses.context.is_empty());
    }
    // A rule that names `Context` already is the person's: nothing is added.
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, denies);
}

/// Migration 0005 on a database that Dots left with a cron string (Scheduler v2 spec §4).
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("mcp__sf__query", Action::Allow),
            Rule::new("Skill", Action::Allow)
        ]
```

**with:**

```rust
            Rule::new("mcp__sf__query", Action::Allow),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("mcp__sf-2", Action::Deny),
            Rule::new("Skill", Action::Allow),
        ]
    );
```

**with:**

```rust
            Rule::new("mcp__sf-2", Action::Deny),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("Skill", Action::Allow),
        ]
```

**with:**

```rust
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
        d6.policy.rules,
        [
            Rule::new("mcp__*", Action::Ask),
            Rule::new("Skill", Action::Allow)
        ]
    );
    assert_eq!(d6.uses, Uses::default());
```

**with:**

```rust
        d6.policy.rules,
        [
            Rule::new("mcp__*", Action::Ask),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
    assert_eq!(d6.uses, Uses::default());
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
            Rule::new("mcp__*", Action::Ask),
            Rule::new("Skill", Action::Allow)
        ]
```

**with:**

```rust
            Rule::new("mcp__*", Action::Ask),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
    assert_eq!(b.uses, uses(&["gh-2", "sf-2"], &[], &[]));
    let skill = Rule::new("Skill", Action::Allow);
    assert_eq!(
        a.policy.rules,
        [Rule::new("mcp__gh__*", Action::Deny), skill.clone()]
    );
```

**with:**

```rust
    assert_eq!(b.uses, uses(&["gh-2", "sf-2"], &[], &[]));
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
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
        b.policy.rules,
        [Rule::new("mcp__gh-2__*", Action::Deny), skill]
    );
```

**with:**

```rust
        b.policy.rules,
        [Rule::new("mcp__gh-2__*", Action::Deny), skill, context]
    );
```

**In `crates/dots-core/tests/store_library_test.rs`, replace:**

```rust
    };
    assert_eq!(rules("D1").await, [Rule::new("Skill", Action::Allow)]);
    assert_eq!(rules("D2").await, [Rule::new("Skill", Action::Deny)]);
    assert_eq!(rules("D3").await, [Rule::new("Skill(pdf)", Action::Ask)]);
}
```

**with:**

```rust
    };
    let context = Rule::new("Context", Action::Allow);
    assert_eq!(
        rules("D1").await,
        [Rule::new("Skill", Action::Allow), context.clone()]
    );
    assert_eq!(
        rules("D2").await,
        [Rule::new("Skill", Action::Deny), context.clone()]
    );
    assert_eq!(
        rules("D3").await,
        [Rule::new("Skill(pdf)", Action::Ask), context]
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test local_context_test --test policy_test --test store_dots_test --test store_library_test --test engine_prompt_test --test local_tools_test --test claude_cli_test`
Expected: FAIL to compile: `build_prompt` takes 5 arguments; `context_tool_name` and `engine::local::tools::context` do not exist.

- [ ] **Step 3: Resolve a run's collections, write the prompt section, add the rule, the upgrade and the local tool**

**Append to `crates/dots-core/migrations/0007_context.sql`:**

```sql

-- Every preset allows `Context` (section 3). Each dot's policy gets the rule when the store opens
-- (`store/upgrade.rs`, `finish_0007`), after the code part of 0006 has added `Skill`, so the
-- rules end as the presets' do. The rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0007 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0007 (dot_id) SELECT id FROM dots;
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust
    /// The functions offered this turn: the built-in tools, `skill` when the run has skills,
    /// and the tools the run's servers list now. A list change a server announced is listed
```

**with:**

```rust
    /// The functions offered this turn: the built-in tools, `skill` when the run has skills,
    /// `context` when it has context collections,
    /// and the tools the run's servers list now. A list change a server announced is listed
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust
            out.push(tools::skill::schema());
        }
```

**with:**

```rust
            out.push(tools::skill::schema());
        }
        if !self.ctx.tools.contexts().is_empty() {
            out.push(tools::context::schema());
        }
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust

    /// Runs a built-in call the gate allowed: `skill` reads the run's skills; every other tool
    /// works in the workspace, the walking tools leaving out what the policy denies.
    async fn run_allowed(&self, call: &PreparedCall) -> ToolOutput {
```

**with:**

```rust

    /// Runs a built-in call the gate allowed: `skill` reads the run's skills and `context` its
    /// context collections; every other tool works in the workspace, the walking tools leaving
    /// out what the policy denies.
    async fn run_allowed(&self, call: &PreparedCall) -> ToolOutput {
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
            return tools::skill::run(self.ctx.tools.skills(), &call.input).await;
        }
```

**with:**

```rust
            return tools::skill::run(self.ctx.tools.skills(), &call.input).await;
        }
        if call.name == "context" {
            return tools::context::run(self.ctx.tools.contexts(), &call.input).await;
        }
```

**Create `crates/dots-core/src/engine/local/tools/context.rs`:**

```rust
//! The `context` tool (policy name `Context`) of the local engine: it reads the run's context
//! collections, never the workspace (context library spec section 4). The reading itself is
//! `crate::context`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::context::{self, Collection, DESCRIPTION};

/// The function definition, offered to a run that has context collections.
pub fn schema() -> Value {
    tool(
        "context",
        DESCRIPTION,
        context::properties(),
        &["collection"],
    )
}

/// Runs an allowed `context` call (its input as `prepare` left it) against the run's
/// collections.
pub async fn run(collections: &[Collection], input: &Value) -> ToolOutput {
    let (collections, input) = (collections.to_vec(), input.clone());
    match tokio::task::spawn_blocking(move || context::call(&collections, &input)).await {
        Ok(Ok(text)) => ToolOutput::ok(text),
        Ok(Err(e)) => ToolOutput::err(e),
        Err(e) => ToolOutput::err(format!("reading the collection failed: {e}")),
    }
}
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust

mod fs;
```

**with:**

```rust

pub mod context;
mod fs;
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// `skill` reads the run's skills, not the workspace: the engine runs it (`skill::run`) and
/// offers it only to a run that has skills.
pub const TOOLS: &[(&str, &str)] = &[
```

**with:**

```rust
/// `skill` reads the run's skills, not the workspace: the engine runs it (`skill::run`) and
/// offers it only to a run that has skills. `context` reads the run's context collections in
/// the same way (`context::run`).
pub const TOOLS: &[(&str, &str)] = &[
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    ("skill", "Skill"),
];
```

**with:**

```rust
    ("skill", "Skill"),
    ("context", "Context"),
];
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    ("skill", &[("name", Str, true), ("file", Str, false)]),
];
```

**with:**

```rust
    ("skill", &[("name", Str, true), ("file", Str, false)]),
    (
        "context",
        &[
            ("collection", Str, true),
            ("query", Str, false),
            ("path", Str, false),
            ("offset", Count, false),
        ],
    ),
];
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
```

**with:**

```rust

pub(super) fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust

/// OpenAI `tools` array for every built-in tool but `skill` (`skill::schema`).
pub fn schemas() -> Value {
```

**with:**

```rust

/// OpenAI `tools` array for every built-in tool but `skill` (`skill::schema`) and `context`
/// (`context::schema`).
pub fn schemas() -> Value {
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        };
        // Every preset lets the model read the skills the dot uses.
        rules.push(Rule::new("Skill", Action::Allow));
        Policy {
```

**with:**

```rust
        };
        // Every preset lets the model read the skills and the context collections the dot uses.
        rules.push(Rule::new("Skill", Action::Allow));
        rules.push(Rule::new("Context", Action::Allow));
        Policy {
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust

    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`.
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let pick = |a: Action| -> Vec<String> {
```

**with:**

```rust

    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`. A rule of
    /// `Context` is left out: the CLI has no such tool, and Dots' own `dots` server checks each
    /// call of its `context` tool against these rules itself (context library spec section 4).
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let dots_only = |tool: &str| {
            let tool = tool.trim();
            tool == "Context" || tool.starts_with("Context(")
        };
        let pick = |a: Action| -> Vec<String> {
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
                .iter()
                .filter(|r| r.action == a)
                .map(|r| r.tool.trim().to_string())
```

**with:**

```rust
                .iter()
                .filter(|r| r.action == a && !dots_only(&r.tool))
                .map(|r| r.tool.trim().to_string())
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust

use crate::model::{Dot, Run, TriggerKind};

/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it; resumes and chat follow-ups send only
/// their message.
pub fn build_prompt(
```

**with:**

```rust

use crate::context::{self, Collection};
use crate::model::{Dot, EngineKind, Run, TriggerKind};

/// The name a run's model calls the `context` tool by: Dots' own MCP server's tool on the
/// Claude engine, a built-in tool on the local engine.
pub fn context_tool_name(engine: EngineKind) -> &'static str {
    match engine {
        EngineKind::Claude => "mcp__dots__context",
        EngineKind::Local => "context",
    }
}

/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it, and ends with the run's context
/// collections; resumes and chat follow-ups send only their message.
pub fn build_prompt(
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
    global: &str,
) -> String {
```

**with:**

```rust
    global: &str,
    contexts: &[Collection],
) -> String {
```

**In `crates/dots-core/src/prompt.rs`, replace:**

```rust
        "- End with a short summary: what you did, what changed, and what needs human review.\n",
    );
    out
}
```

**with:**

```rust
        "- End with a short summary: what you did, what changed, and what needs human review.\n",
    );
    let section = context::prompt_section(contexts, context_tool_name(dot.spec.engine));
    if !section.is_empty() {
        out.push_str("\n## Shared context\n\n");
        out.push_str(&section);
        out.push('\n');
    }
    out
}
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
//! - the skills it reads, standalone (`pdf`) and from its plugins (`forms:fill`);
//! - for a Claude-engine run, the plugin folders it passes with `--plugin-dir`.
//!
```

**with:**

```rust
//! - the skills it reads, standalone (`pdf`) and from its plugins (`forms:fill`);
//! - for a Claude-engine run, the plugin folders it passes with `--plugin-dir`;
//! - the context collections it reads (context library spec section 5), ticked or for every
//!   dot, each with the count of its documents. Nothing is held open for them.
//!
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust

use crate::events::Bus;
```

**with:**

```rust

use crate::context::{self, Collection};
use crate::events::Bus;
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    skills: Vec<RunSkill>,
    kit: Option<Arc<RunKit>>,
```

**with:**

```rust
    skills: Vec<RunSkill>,
    contexts: Vec<Collection>,
    kit: Option<Arc<RunKit>>,
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                &self.skills.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
```

**with:**

```rust
                &self.skills.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
            .field(
                "contexts",
                &self.contexts.iter().map(|c| &c.name).collect::<Vec<_>>(),
            )
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        self.skills.iter().find(|s| s.name == name)
    }
```

**with:**

```rust
        self.skills.iter().find(|s| s.name == name)
    }

    /// The context collections, by name.
    pub fn contexts(&self) -> &[Collection] {
        &self.contexts
    }
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                Err(e) => notices.push(format!("the plugin folders could not be made: {e}")),
            }
```

**with:**

```rust
                Err(e) => notices.push(format!("the plugin folders could not be made: {e}")),
            }
        }

        for item in store.used_collections(uses).await? {
            let LibraryConfig::Context(config) = &item.config else {
                continue;
            };
            let dir = PathBuf::from(&config.path);
            let walk_dir = dir.clone();
            let walked = tokio::task::spawn_blocking(move || context::walk(&walk_dir))
                .await
                .map_err(|e| crate::Error::Other(format!("reading a collection failed: {e}")))?;
            match walked {
                Ok(walk) => tools.contexts.push(Collection {
                    name: item.name,
                    dir,
                    description: config.description.clone(),
                    documents: walk.total,
                }),
                Err(_) if !dir.is_dir() => {
                    notices.push(format!("context {}: folder not found", item.name))
                }
                Err(e) => notices.push(format!("context {}: {e}", item.name)),
            }
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            workspace: prepared.path.clone(),
            prompt: build_prompt(&dot, run, &prepared.path, chrono::Local::now(), &global),
            session_id: run.session_id.clone(),
```

**with:**

```rust
            workspace: prepared.path.clone(),
            prompt: build_prompt(
                &dot,
                run,
                &prepared.path,
                chrono::Local::now(),
                &global,
                tools.contexts(),
            ),
            session_id: run.session_id.clone(),
```

**In `crates/dots-core/src/store/library.rs`, replace:**

```rust
        item_from_row(&row)
    }
```

**with:**

```rust
        item_from_row(&row)
    }

    /// The context collections a run of a dot with `uses` reads: those it names and every one
    /// whose `every_dot` is on, by name.
    pub async fn used_collections(&self, uses: &Uses) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE kind = 'context' AND (\
             name IN (SELECT value FROM json_each(?)) \
             OR json_extract(config, '$.every_dot') = 1) ORDER BY name"
        ))
        .bind(serde_json::to_string(&uses.context)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
impl Store {
    /// Opens (creating if needed) the database file and applies migrations, with the part of
    /// migration 0006 that needs code (`upgrade.rs`).
    pub async fn open(path: &Path) -> Result<Store> {
```

**with:**

```rust
impl Store {
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006 and 0007 that need code (`upgrade.rs`).
    pub async fn open(path: &Path) -> Result<Store> {
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
        upgrade::finish_0006(&pool).await?;
        Ok(Store { pool })
```

**with:**

```rust
        upgrade::finish_0006(&pool).await?;
        upgrade::finish_0007(&pool).await?;
        Ok(Store { pool })
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
//! The part of migration 0006 that needs code (MCP/skills spec section 10). The SQL migration
//! moves each dot's old `mcp_servers` JSON into the table `upgrade_0006`; when the store opens,
//! `finish_0006` turns those servers into Library items ticked in the dots' `uses`, brings each
//! dot's policy up to date, and drops the table, in one transaction.

```

**with:**

```rust
//! The parts of migrations 0006 and 0007 that need code. 0006 (MCP/skills spec section 10):
//! the SQL migration moves each dot's old `mcp_servers` JSON into the table `upgrade_0006`;
//! when the store opens, `finish_0006` turns those servers into Library items ticked in the
//! dots' `uses`, brings each dot's policy up to date, and drops the table, in one transaction.
//! 0007 (context library spec section 8): `finish_0007` then gives each dot listed in
//! `upgrade_0007` a `Context` allow rule unless a rule names `Context`, and drops that table.

```

**Append to `crates/dots-core/src/store/upgrade.rs`:**

```rust

/// Gives the policy of each dot that migration 0007 listed a `Context` allow rule at the end,
/// unless a rule names `Context` (`Context` or `Context(...)`), and drops the list.
pub(super) async fn finish_0007(pool: &SqlitePool) -> Result<()> {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0007'",
    )
    .fetch_one(pool)
    .await?;
    if waiting == 0 {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT d.id, d.policy FROM upgrade_0007 u JOIN dots d ON d.id = u.dot_id")
            .fetch_all(&mut *tx)
            .await?;
    for (dot_id, stored) in rows {
        let Some(policy) = with_context(&stored) else {
            tracing::warn!(dot = %dot_id, "migration 0007: the policy could not be read");
            continue;
        };
        sqlx::query("UPDATE dots SET policy = ? WHERE id = ?")
            .bind(policy)
            .bind(&dot_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE upgrade_0007")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The stored policy with a `Context` allow rule at the end when no rule names `Context`;
/// `None` when the stored JSON is not a policy.
fn with_context(stored: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(stored).ok()?;
    let rules = value.get_mut("rules")?.as_array_mut()?;
    let names_context = |rule: &Value| {
        rule["tool"].as_str().is_some_and(|t| {
            let t = t.trim();
            t == "Context" || t.starts_with("Context(")
        })
    };
    if !rules.iter().any(names_context) {
        rules.push(json!({"tool": "Context", "action": Action::Allow.as_str()}));
    }
    // Through `Policy`, so the JSON is written as Dots writes a policy.
    let policy: Policy = serde_json::from_value(value).ok()?;
    serde_json::to_string(&policy).ok()
}
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the fixture changes only where this task's types changed.

**Generated:** `ui/src/api/__fixtures__/contract.json`

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test local_context_test --test policy_test --test store_dots_test --test store_library_test --test engine_prompt_test --test local_tools_test --test claude_cli_test`
Expected: PASS (`local_context_test`: 5 tests).

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 644 passed, `dots-app` 66, the UI 373 in 25 files; no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/migrations/0007_context.sql crates/dots-core/src/engine/local/mcp_tools.rs crates/dots-core/src/engine/local/mod.rs crates/dots-core/src/engine/local/tools/context.rs crates/dots-core/src/engine/local/tools/mod.rs crates/dots-core/src/policy.rs crates/dots-core/src/prompt.rs crates/dots-core/src/run_tools.rs crates/dots-core/src/runner.rs crates/dots-core/src/store/library.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/upgrade.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/local_context_test.rs crates/dots-core/tests/local_tools_test.rs crates/dots-core/tests/policy_test.rs crates/dots-core/tests/store_dots_test.rs crates/dots-core/tests/store_library_test.rs ui/src/api/__fixtures__/contract.json
git commit -m "feat(core): give a run its context collections, list them in the prompt and read them with the local context tool"
```

### Task 5: Serve the context tool from Dots' own MCP server to Claude runs

The `dots` server lists `context` next to `approve` for a run with collections, checks each call with the hub as the tool `Context` (waiting for an approval when a rule asks), and reads the collections with Task 2's `call`. A call that is not performed is a tool result with `isError`, in the gateway's words. The CLI gets `--allowedTools mcp__dots__context`, so it does not ask `approve` first.

**Files:**

- Modify: `crates/dots-core/src/engine/claude/args.rs`
- Modify: `crates/dots-core/src/server/gateway.rs`
- Modify: `crates/dots-core/src/server/mcp.rs`
- Modify: `crates/dots-core/tests/claude_gateway_test.rs`

**Interfaces:**

- Consumes: Task 4's `RunTools::contexts()` and `context::call`; `PermissionGate::check` (existing).
- Produces: the `dots` server's `context` tool (`tools/list`, `tools/call`); `server::gateway::not_performed` is `pub(super)`; `--allowedTools mcp__dots__context` for a run with collections.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
use dots_core::events::{new_bus, Bus};
use dots_core::library::{LibraryConfig, LibraryItem, McpServerConfig, PluginConfig, SkillConfig};
use dots_core::library_service::LibraryService;
```

**with:**

```rust
use dots_core::events::{new_bus, Bus};
use dots_core::library::{
    ContextConfig, LibraryConfig, LibraryItem, McpServerConfig, PluginConfig, SkillConfig,
};
use dots_core::library_service::LibraryService;
```

**Append to `crates/dots-core/tests/claude_gateway_test.rs`:**

```rust

/// Adds the context collection `company`: a folder with `style.md`.
async fn add_company(e: &Env) {
    let kb = e.dir.path().join("kb");
    write_file(&kb.join("style.md"), "# House style\nShort sentences.\n");
    e.lib
        .add(
            "company",
            LibraryConfig::Context(ContextConfig {
                path: kb.to_string_lossy().into_owned(),
                description: "How we work".into(),
                every_dot: false,
            }),
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_claude_run_reads_its_collections_through_dots_own_server() {
    let e = env(
        json!([init(),
               {"mcp_list": "dots"},
               mcp_call("dots", "context", json!({"collection": "company", "path": "style.md"})),
               success()]),
        true,
    )
    .await;
    add_company(&e).await;
    let dot = claude_dot(&e, |s| {
        s.policy = Policy::preset(Preset::ReadOnly);
        s.uses.context = vec!["company".into()];
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let args: Vec<String> = serde_json::from_value(recorded(&e)["args"].clone()).unwrap();
    let allowed = values_of(&args, "--allowedTools");
    assert!(
        allowed.contains(&"mcp__dots__context".to_string()),
        "{args:?}"
    );
    assert!(!allowed.contains(&"Context".to_string()), "{args:?}");
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(listed["tools"], json!(["approve", "context"]));
    let calls = events(&e, &run.id, "tool_call").await;
    assert_eq!(calls[0].data["tool"], "mcp__dots__context");
    let result = &events(&e, &run.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], false, "{result}");
    assert_eq!(result["output"], "# House style\nShort sentences.");
}

#[tokio::test]
async fn dots_own_server_offers_context_only_with_collections_and_checks_each_call() {
    let e = env(json!([]), false).await;
    add_company(&e).await;
    let dot = claude_dot(&e, |s| s.uses.context = vec!["company".into()]).await;
    let bare = claude_dot(&e, |s| s.name = "bare".into()).await;
    let denying = claude_dot(&e, |s| {
        s.name = "denying".into();
        s.uses.context = vec!["company".into()];
        s.policy.rules.insert(0, Rule::new("Context", Action::Deny));
    })
    .await;
    let mut secrets = Vec::new();
    for d in [&dot, &bare, &denying] {
        let run = e
            .store
            .create_run(&NewRun::new(&d.id, TriggerKind::Manual))
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let (tools, _) = RunTools::open(OpenRequest {
            store: &e.store,
            bus: &e.bus,
            run_id: &run.id,
            uses: &d.spec.uses,
            workspace: e.dir.path(),
            plugins_dir: None,
            cancel: &cancel,
        })
        .await
        .unwrap();
        let secret = e.runner.register_secret(&run.id);
        e.runner.register_tools(&run.id, Arc::new(tools));
        secrets.push(secret);
    }
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let names = |v: Value| -> Vec<String> {
        v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let with: Value = rpc(&e, "/mcp", Some(&secrets[0]), list.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(names(with.clone()), ["approve", "context"]);
    assert_eq!(
        with["result"]["tools"][1]["inputSchema"]["required"],
        json!(["collection"])
    );
    let without: Value = rpc(&e, "/mcp", Some(&secrets[1]), list)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(names(without), ["approve"]);

    let context = |args: Value| {
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "context", "arguments": args}})
    };
    let ok: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[0]),
        context(json!({"collection": "company"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(
        ok["result"]["content"][0]["text"],
        "- style.md — House style: Short sentences."
    );
    assert!(ok["result"].get("isError").is_none(), "{ok}");
    // The dot's own rule decides: a `Context` deny rule denies the call.
    let denied: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[2]),
        context(json!({"collection": "company"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert!(
        !denied["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("style.md"),
        "{denied}"
    );
    let unknown: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[0]),
        context(json!({"collection": "x"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(unknown["result"]["isError"], true);
    assert!(
        unknown["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Collections: company"),
        "{unknown}"
    );
    // A run without collections has no `context` tool.
    let none: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[1]),
        context(json!({"collection": "company"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(none["error"]["code"], -32602, "{none}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test claude_gateway_test`
Expected: FAIL: `a_claude_run_reads_its_collections_through_dots_own_server` and `dots_own_server_offers_context_only_with_collections_and_checks_each_call` (the server lists only `approve`).

- [ ] **Step 3: Offer and check the tool on the `dots` server**

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
    );
    if !allow.is_empty() {
```

**with:**

```rust
    );
    // Dots' own `dots` server checks each `context` call against the policy itself (context
    // library spec section 4).
    if !ctx.tools.contexts().is_empty() {
        allow.push("mcp__dots__context".into());
    }
    if !allow.is_empty() {
```

**In `crates/dots-core/src/server/gateway.rs`, replace:**

```rust
/// local engine gives its model.
fn not_performed(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
```

**with:**

```rust
/// local engine gives its model.
pub(super) fn not_performed(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust

use super::webhook::bearer;
use super::ServerState;
use crate::engine::{PermissionGate, PermissionOutcome};
```

**with:**

```rust

use super::gateway::not_performed;
use super::webhook::bearer;
use super::ServerState;
use crate::context;
use crate::engine::{PermissionGate, PermissionOutcome};
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust

pub async fn handle(State(s): State<ServerState>, headers: HeaderMap, body: Bytes) -> Response {
```

**with:**

```rust

/// The `context` tool of a run that has context collections (context library spec section 4):
/// the same arguments as the local engine's.
fn context_tool() -> Value {
    json!({
        "name": "context",
        "description": context::DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": context::properties(),
            "required": ["collection"]
        }
    })
}

pub async fn handle(State(s): State<ServerState>, headers: HeaderMap, body: Bytes) -> Response {
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [approve_tool()] }),
        "tools/call" => match call_approve(&s, &run_id, &req).await {
            Ok(v) => v,
            Err((code, msg)) => return rpc_error(id, code, &msg),
        },
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
```

**with:**

```rust
        "ping" => json!({}),
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
            match answer {
                Ok(v) => v,
                Err((code, msg)) => return rpc_error(id, code, &msg),
            }
        }
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
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

/// The run has context collections open, so `context` is one of its tools.
fn has_contexts(s: &ServerState, run_id: &str) -> bool {
    s.runner
        .run_tools(run_id)
        .is_some_and(|tools| !tools.contexts().is_empty())
}

/// Checks a `context` call against the dot's policy as the tool `Context`, waiting for an
/// approval when a rule asks, and reads the run's collections. A call that is not performed is
/// a tool result with `isError`, as the gateway answers.
async fn call_context(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let Some(tools) = s
        .runner
        .run_tools(run_id)
        .filter(|tools| !tools.contexts().is_empty())
    else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let input = match s.hub.check(run_id, "Context", input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let collections = tools.contexts().to_vec();
    let read = tokio::task::spawn_blocking(move || context::call(&collections, &input)).await;
    Ok(match read {
        Ok(Ok(text)) => json!({ "content": [{ "type": "text", "text": text }] }),
        Ok(Err(e)) => not_performed(&e),
        Err(e) => not_performed(&format!("reading the collection failed: {e}")),
    })
}
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test claude_gateway_test`
Expected: PASS: 9 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 646 passed, `dots-app` 66; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/engine/claude/args.rs crates/dots-core/src/server/gateway.rs crates/dots-core/src/server/mcp.rs crates/dots-core/tests/claude_gateway_test.rs
git commit -m "feat(core): serve the context tool from Dots' own MCP server to Claude runs"
```

### Task 6: Global instructions in Settings, and a dot's context collections and switch in its editor

Settings gets a text area with the byte count and the 32 KB check, counted in UTF-8 as the core counts. The editor gets the switch in Basics and a fourth checklist, Context, where a collection for every dot shows ticked and disabled.

**Files:**

- Modify: `ui/src/screens/DotEditorScreen.test.tsx`
- Modify: `ui/src/screens/SettingsScreen.test.tsx`
- Modify: `ui/src/screens/SettingsScreen.tsx`
- Modify: `ui/src/screens/dot-editor/DotForm.tsx`
- Modify: `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`
- Modify: `ui/src/screens/dot-editor/ToolsAndSkills.tsx`

**Interfaces:**

- Consumes: Task 1's `Settings.global_instructions`, `DotSpec.use_global_instructions`, `LIMITS.max_global_instructions`; Task 3's `LibraryEntry` of kind `context`.
- Produces: `settingsErrors` taking an optional `global_instructions` and `utf8Bytes(text: string): number` in `SettingsScreen.tsx`; the switch "Start with the global instructions"; the checklist "Context" in `ToolsAndSkills`.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
      { id: dot.id, spec: { ...specOf(dot), instructions: "Only summarize." } },
    ]);
```

**with:**

```tsx
      { id: dot.id, spec: { ...specOf(dot), instructions: "Only summarize." } },
    ]);
  });

  it("switches the global instructions off for this dot", async () => {
    stored();
    const { user } = renderApp(path);
    const box = await screen.findByLabelText("Start with the global instructions");
    expect(box).toBeChecked();
    await user.click(box);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(dot), use_global_instructions: false } },
    ]);
```

**In `ui/src/screens/SettingsScreen.test.tsx`, replace:**

```tsx
    expect(await screen.findByRole("button", { name: "Restart Dots" })).toBeVisible();
  });
```

**with:**

```tsx
    expect(await screen.findByRole("button", { name: "Restart Dots" })).toBeVisible();
  });

  it("saves the global instructions, up to 32 KB", async () => {
    saving();
    const { user } = renderApp("/settings");
    const text = await screen.findByLabelText("Global instructions");
    expect(text).toHaveValue(stored.global_instructions);
    await user.clear(text);
    await user.type(text, "Sign off as the team.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, global_instructions: "Sign off as the team." } },
    ]);
    const limit = { port: 1, max_concurrent_runs: 1 };
    expect(settingsErrors({ ...limit, global_instructions: "x".repeat(32768) })).toEqual({});
    // Bytes, as the core counts: "é" is two.
    expect(settingsErrors({ ...limit, global_instructions: "é".repeat(16385) })).toEqual({
      global: "The global instructions are at most 32 KB.",
    });
  });
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx

const [github, pluginServer, pdf, pdfTools] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [] };
```

**with:**

```tsx

const [github, pluginServer, pdf, pdfTools, company] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [] };
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
describe("the editor's Tools and skills section", () => {
  it("lists the Library in three checklists, without the servers a plugin brings", async () => {
    setup();
```

**with:**

```tsx
describe("the editor's Tools and skills section", () => {
  it("lists the Library in four checklists, without the servers a plugin brings", async () => {
    setup();
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    expect(within(screen.getByRole("group", { name: "Plugins" })).getByLabelText("pdf-tools")).not.toBeChecked();
  });
```

**with:**

```tsx
    expect(within(screen.getByRole("group", { name: "Plugins" })).getByLabelText("pdf-tools")).not.toBeChecked();
  });

  it("lists the context collections, one for every dot ticked and fixed", async () => {
    const own = { ...company!, id: "01J0CTX2", name: "clients" } as LibraryEntry;
    if (own.kind !== "context") throw new Error("the sample is a context collection");
    own.config = { ...own.config, every_dot: false, description: "Who they are" };
    const { user, uses } = setup({ library: [company!, own] });
    const group = await screen.findByRole("group", { name: "Context" });
    const every = within(group).getByLabelText("company");
    expect(every).toBeChecked();
    expect(every).toBeDisabled();
    expect(within(group).getByText(/42 documents · every dot/)).toBeVisible();
    const box = within(group).getByLabelText("clients");
    expect(box).not.toBeChecked();
    await user.click(box);
    expect(uses().context).toEqual(["clients"]);
  });
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    setup({ library: [] });
    expect(await screen.findAllByText(/Nothing here yet/)).toHaveLength(3);
    expect(screen.getAllByRole("link", { name: "Library" })[0]).toHaveAttribute("href", "/library");
```

**with:**

```tsx
    setup({ library: [] });
    expect(await screen.findAllByText(/Nothing here yet/)).toHaveLength(4);
    expect(screen.getAllByRole("link", { name: "Library" })[0]).toHaveAttribute("href", "/library");
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/SettingsScreen.test.tsx src/screens/dot-editor/ToolsAndSkills.test.tsx src/screens/DotEditorScreen.test.tsx`
Expected: FAIL: the label "Global instructions", the switch and the group "Context" are not found.

- [ ] **Step 3: Add the text area, the switch and the checklist**

**In `ui/src/screens/SettingsScreen.tsx`, replace:**

```tsx
import { Switch } from "@/components/ui/switch";
import { errorMessage, notifyError } from "@/lib/notify";
```

**with:**

```tsx
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage, notifyError } from "@/lib/notify";
```

**In `ui/src/screens/SettingsScreen.tsx`, replace:**

```tsx

type Errors = { port?: string; max?: string };

/// What is wrong with the numbers, mirroring `Settings::validate` in the core.
export function settingsErrors(draft: Pick<Settings, "port" | "max_concurrent_runs">): Errors {
  const errors: Errors = {};
  const whole = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
```

**with:**

```tsx

type Errors = { port?: string; max?: string; global?: string };

/// The size of `text` in UTF-8 bytes, as the core counts it.
export const utf8Bytes = (text: string) => new TextEncoder().encode(text).length;

/// What is wrong with the numbers and the global instructions, mirroring `Settings::validate`
/// in the core.
export function settingsErrors(
  draft: Pick<Settings, "port" | "max_concurrent_runs"> &
    Partial<Pick<Settings, "global_instructions">>,
): Errors {
  const errors: Errors = {};
  if (utf8Bytes(draft.global_instructions ?? "") > LIMITS.max_global_instructions) {
    errors.global = "The global instructions are at most 32 KB.";
  }
  const whole = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
```

**In `ui/src/screens/SettingsScreen.tsx`, replace:**

```tsx

      <Section title="Local server and runs">
```

**with:**

```tsx

      <Section title="Global instructions">
        <Field
          label="Global instructions"
          htmlFor="settings-global"
          error={tried ? errors.global : undefined}
          hint={`Every dot's first prompt starts with these. A dot can switch them off in its editor. ${utf8Bytes(draft.global_instructions)} of ${LIMITS.max_global_instructions} bytes.`}
        >
          <Textarea
            id="settings-global"
            rows={6}
            value={draft.global_instructions}
            onChange={(e) => set({ global_instructions: e.target.value })}
          />
        </Field>
      </Section>

      <Section title="Local server and runs">
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
        </div>
      </Section>
```

**with:**

```tsx
        </div>
        <div className="flex items-center gap-2">
          <Switch
            id="dot-global-instructions"
            checked={spec.use_global_instructions}
            onCheckedChange={(v) => set("use_global_instructions", v)}
          />
          <Label htmlFor="dot-global-instructions">Start with the global instructions</Label>
        </div>
        <p className="-mt-2 text-xs text-muted-foreground">
          The global instructions are set in Settings. A blank text adds nothing.
        </p>
      </Section>
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
  { kind: "plugin", key: "plugins", title: "Plugins" },
];

```

**with:**

```tsx
  { kind: "plugin", key: "plugins", title: "Plugins" },
  { kind: "context", key: "context", title: "Context" },
];

/// A context collection every dot reads, ticked or not.
const everyDot = (entry: LibraryEntry) => entry.kind === "context" && entry.config.every_dot;

```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
        ? entry.plugin?.description
        : null;
  const status = entry.catalog?.status;
```

**with:**

```tsx
        ? entry.plugin?.description
        : entry.kind === "context"
          ? [
              entry.config.description,
              entry.context && plural(entry.context.documents, "document"),
              entry.config.every_dot && "every dot",
            ]
              .filter(Boolean)
              .join(" · ")
          : null;
  const status = entry.catalog?.status;
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx

/// The dot editor's "Tools and skills" section: which Library items the dot uses, as three
/// checklists. A plugin's own MCP servers are not listed: they come with the plugin.
export function ToolsAndSkills(props: {
```

**with:**

```tsx

/// The dot editor's "Tools and skills" section: which Library items the dot uses, as four
/// checklists. A plugin's own MCP servers are not listed: they come with the plugin. A context
/// collection for every dot shows ticked and cannot be unticked here.
export function ToolsAndSkills(props: {
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
                    aria-label={entry.name}
                    checked={uses[key].includes(entry.name)}
                    onChange={(e) => toggle(key, entry.name, e.target.checked)}
```

**with:**

```tsx
                    aria-label={entry.name}
                    checked={everyDot(entry) || uses[key].includes(entry.name)}
                    disabled={everyDot(entry)}
                    onChange={(e) => toggle(key, entry.name, e.target.checked)}
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/SettingsScreen.test.tsx src/screens/dot-editor/ToolsAndSkills.test.tsx src/screens/DotEditorScreen.test.tsx`
Expected: PASS.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: the UI 376 in 25 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/SettingsScreen.test.tsx ui/src/screens/SettingsScreen.tsx ui/src/screens/dot-editor/DotForm.tsx ui/src/screens/dot-editor/ToolsAndSkills.test.tsx ui/src/screens/dot-editor/ToolsAndSkills.tsx
git commit -m "feat(ui): global instructions in Settings, and a dot's context collections and switch in its editor"
```

### Task 7: A Library tab for context collections, with their documents

The Library's fourth tab: rows with the description, count, folder and users; "Add collection" picks a folder and opens a form with a name made from it; "Edit" changes all but the name; "Documents" lists the first 200 from `context_documents`.

**Files:**

- Create: `ui/src/screens/library/ContextTab.tsx`
- Modify: `ui/src/screens/library/LibraryScreen.test.tsx`
- Modify: `ui/src/screens/library/LibraryScreen.tsx`
- Modify: `ui/src/screens/library/parts.tsx`
- Modify: `ui/src/test/fixtures.ts`

**Interfaces:**

- Consumes: Task 3's `commands.contextDocuments` and types; `RemoveButton`, `ItemRow`, `UsedBy`, `plural` from `parts.tsx`.
- Produces: `ContextTab`, `nameFromFolder(path: string): string` and `validName(name: string): boolean` in `screens/library/ContextTab.tsx`; `ContextEntry` in `parts.tsx`; `sample.contextDocuments` in the UI's test fixtures.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx

const [github, owned, skill, plugin] = sample.library as [
  LibraryEntry,
```

**with:**

```tsx

const [github, owned, skill, plugin, company] = sample.library as [
  LibraryEntry,
  LibraryEntry,
```

**Append to `ui/src/screens/library/LibraryScreen.test.tsx`:**

```tsx

describe("the Library's Context tab", () => {
  it("lists a collection with its description, documents, folder and every-dot badge", async () => {
    listing([company]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    expect(within(row).getByText("How we work: style, people, runbooks")).toBeVisible();
    expect(within(row).getByText("42 documents")).toBeVisible();
    expect(within(row).getByText("every dot")).toBeVisible();
    expect(within(row).getByText("Every dot reads it")).toBeVisible();
    expect(within(row).getByText("C:/notes/company")).toBeVisible();
  });

  it("adds a collection from a folder under a name made from it", async () => {
    listing([]);
    backend.on("dialog_open", () => "C:\\Notes\\Company Wiki");
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    expect(within(dialog).getByText(/Only add folders whose writers you trust/)).toBeVisible();
    expect(within(dialog).getByLabelText("Name")).toHaveValue("company-wiki");
    await user.type(within(dialog).getByLabelText("Description"), "How we work");
    await user.click(within(dialog).getByLabelText("Every dot reads it"));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "company-wiki",
          config: {
            kind: "context",
            config: { path: "C:\\Notes\\Company Wiki", description: "How we work", every_dot: true },
          },
          accept: null,
        },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("checks the name and the description before it asks the core", async () => {
    listing([]);
    backend.on("dialog_open", () => "C:/kb");
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    const name = within(dialog).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Bad Name");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(
      within(dialog).getByText("Use 1 to 40 of a-z, 0-9, _ and -, starting with a letter or digit."),
    ).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("edits a collection: the name stays, the config goes to the core", async () => {
    listing([company]);
    backend.on("library_update", () => company);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit company" });
    expect(within(dialog).getByLabelText("Name")).toBeDisabled();
    await user.click(within(dialog).getByLabelText("Every dot reads it"));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_update")).toEqual([
        {
          id: company.id,
          config: {
            kind: "context",
            config: {
              path: "C:/notes/company",
              description: "How we work: style, people, runbooks",
              every_dot: false,
            },
          },
        },
      ]),
    );
  });

  it("shows a collection's first documents and says how many there are", async () => {
    listing([company]);
    backend.on("context_documents", () => sample.contextDocuments);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Documents" }));
    const dialog = await screen.findByRole("dialog", { name: "Documents of company" });
    expect(await within(dialog).findByText("Ana Lima")).toBeVisible();
    expect(within(dialog).getByText("Owns billing")).toBeVisible();
    expect(within(dialog).getByText("The first 1 of 42 documents.")).toBeVisible();
    expect(backend.callsTo("context_documents")).toEqual([{ id: company.id }]);
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  scannedSkills: T.ScannedSkill.array().parse(contract.ScannedSkill),
  installedPlugins: T.InstalledPlugin.array().parse(contract.InstalledPlugin),
```

**with:**

```ts
  scannedSkills: T.ScannedSkill.array().parse(contract.ScannedSkill),
  contextDocuments: T.ContextDocuments.parse(contract.ContextDocuments),
  installedPlugins: T.InstalledPlugin.array().parse(contract.InstalledPlugin),
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/library`
Expected: FAIL: the tab "Context" is not found.

- [ ] **Step 3: Add the tab**

**Create `ui/src/screens/library/ContextTab.tsx`:**

```tsx
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ItemRow, plural, RemoveButton, UsedBy, type ContextEntry } from "./parts";

/// The longest name and description, as `library::check_name` and `LibraryConfig::validate`
/// take them.
const MAX_NAME = 40;
const MAX_DESCRIPTION = 300;
const NAME_RULE = "Use 1 to 40 of a-z, 0-9, _ and -, starting with a letter or digit.";

export const validName = (name: string) => /^[a-z0-9][a-z0-9_-]{0,39}$/.test(name);

/// A name for a collection from its folder: lower case, other characters as `-`, cut at 40.
export function nameFromFolder(path: string): string {
  const folder = path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
  return folder
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[-_]+/, "")
    .slice(0, MAX_NAME);
}

type Form = { name: string; path: string; description: string; every_dot: boolean };

/// Add a collection (`entry` null) or change one: its folder, description and "Every dot". The
/// name of a stored collection stays.
function CollectionDialog(props: { entry: ContextEntry | null; path: string; onClose: () => void }) {
  const qc = useQueryClient();
  const { entry } = props;
  const [form, setForm] = useState<Form>({
    name: entry?.name ?? nameFromFolder(props.path),
    path: props.path,
    description: entry?.config.description ?? "",
    every_dot: entry?.config.every_dot ?? false,
  });
  const [tried, setTried] = useState(false);
  const [state, setState] = useState<{ busy: boolean; error?: string }>({ busy: false });
  const errors = {
    name: validName(form.name) ? undefined : NAME_RULE,
    description:
      [...form.description].length > MAX_DESCRIPTION
        ? `At most ${MAX_DESCRIPTION} characters.`
        : undefined,
  };
  const set = (change: Partial<Form>) => setForm({ ...form, ...change });

  async function browse() {
    try {
      const picked = await open({ directory: true, title: "Choose the collection's folder" });
      if (typeof picked === "string") set({ path: picked });
    } catch (e) {
      notifyError(e);
    }
  }

  async function save() {
    setTried(true);
    if (errors.name || errors.description) return;
    const config = {
      kind: "context" as const,
      config: { path: form.path, description: form.description.trim(), every_dot: form.every_dot },
    };
    setState({ busy: true });
    try {
      if (entry) await commands.libraryUpdate(entry.id, config);
      else await commands.libraryAdd(form.name, config, null);
      invalidate(qc, keys.library);
      props.onClose();
    } catch (e) {
      setState({ busy: false, error: errorMessage(e) });
    }
  }

  const title = entry ? `Edit ${entry.name}` : "Add context collection";
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Dots read these files as guidance. Only add folders whose writers you trust.
          </DialogDescription>
        </DialogHeader>
        <Field label="Name" htmlFor="context-name" error={tried ? errors.name : undefined}>
          <Input
            id="context-name"
            value={form.name}
            disabled={entry !== null}
            onChange={(e) => set({ name: e.target.value })}
          />
        </Field>
        <Field label="Folder" htmlFor="context-path">
          <div className="flex gap-2">
            <Input id="context-path" readOnly className="font-mono" value={form.path} />
            <Button type="button" variant="outline" onClick={() => void browse()}>
              Choose…
            </Button>
          </div>
        </Field>
        <Field
          label="Description"
          htmlFor="context-description"
          error={tried ? errors.description : undefined}
          hint="Dots see it next to the name, to know when to look here."
        >
          <Textarea
            id="context-description"
            rows={2}
            value={form.description}
            onChange={(e) => set({ description: e.target.value })}
          />
        </Field>
        <div className="flex items-center gap-2">
          <Switch
            id="context-every-dot"
            checked={form.every_dot}
            onCheckedChange={(every_dot) => set({ every_dot })}
          />
          <Label htmlFor="context-every-dot">Every dot reads it</Label>
        </div>
        {state.error && (
          <p role="alert" className="text-xs text-destructive">
            {state.error}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button type="button" disabled={state.busy} onClick={() => void save()}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The first documents of a collection, each with its title and summary.
function DocumentsDialog(props: { entry: ContextEntry; onClose: () => void }) {
  const docs = useQuery({
    queryKey: [...keys.library, "documents", props.entry.id],
    queryFn: () => commands.contextDocuments(props.entry.id),
  });
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{`Documents of ${props.entry.name}`}</DialogTitle>
          <DialogDescription className="font-mono">{props.entry.config.path}</DialogDescription>
        </DialogHeader>
        {docs.isPending && <p className="text-muted-foreground">Reading the folder…</p>}
        {docs.isError && <ErrorNote error={docs.error} />}
        {docs.data && docs.data.documents.length === 0 && (
          <p className="text-muted-foreground">No documents in that folder.</p>
        )}
        {docs.data && docs.data.documents.length > 0 && (
          <ul className="flex flex-col gap-1.5">
            {docs.data.documents.map((doc) => (
              <li key={doc.path} className="flex flex-col rounded-md border px-2 py-1.5">
                <span className="text-sm font-medium">{doc.title}</span>
                {doc.summary && <span className="text-xs text-muted-foreground">{doc.summary}</span>}
                <span className="font-mono text-xs text-muted-foreground">{doc.path}</span>
              </li>
            ))}
          </ul>
        )}
        {docs.data?.truncated && (
          <p role="status" className="text-xs text-muted-foreground">
            {`The first ${docs.data.documents.length} of ${plural(docs.data.total, "document")}.`}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

type Open =
  | { what: "add"; path: string }
  | { what: "edit"; entry: ContextEntry }
  | { what: "documents"; entry: ContextEntry };

/// The context tab: folders of documents that dots read with the `context` tool.
export function ContextTab({ entries }: { entries: ContextEntry[] }) {
  const [opened, setOpened] = useState<Open | null>(null);

  async function add() {
    try {
      const folder = await open({ directory: true, title: "Choose the collection's folder" });
      if (typeof folder === "string") setOpened({ what: "add", path: folder });
    } catch (e) {
      notifyError(e);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Folders of text documents (Markdown, text, JSON, YAML, CSV) that dots list, search and
          read. Dots never change them.
        </p>
        <Button size="sm" onClick={() => void add()}>
          <Plus aria-hidden /> Add collection
        </Button>
      </div>
      {entries.length === 0 ? (
        <p className="text-muted-foreground">No context collections yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => (
            <ItemRow
              key={entry.id}
              label={entry.name}
              actions={
                <>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setOpened({ what: "documents", entry })}
                  >
                    Documents
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setOpened({ what: "edit", entry })}
                  >
                    Edit
                  </Button>
                  <RemoveButton entry={entry} />
                </>
              }
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{entry.name}</span>
                {entry.config.every_dot && <Badge variant="secondary">every dot</Badge>}
                {entry.context && (
                  <span className="text-xs text-muted-foreground">
                    {plural(entry.context.documents, "document")}
                  </span>
                )}
              </div>
              {entry.config.description && (
                <span className="text-xs">{entry.config.description}</span>
              )}
              <span className="truncate font-mono text-xs text-muted-foreground">
                {entry.config.path}
              </span>
              {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
              {entry.config.every_dot ? (
                <span className="text-xs text-muted-foreground">Every dot reads it</span>
              ) : (
                <UsedBy dots={entry.used_by} />
              )}
            </ItemRow>
          ))}
        </ul>
      )}
      {opened?.what === "add" && (
        <CollectionDialog entry={null} path={opened.path} onClose={() => setOpened(null)} />
      )}
      {opened?.what === "edit" && (
        <CollectionDialog
          entry={opened.entry}
          path={opened.entry.config.path}
          onClose={() => setOpened(null)}
        />
      )}
      {opened?.what === "documents" && (
        <DocumentsDialog entry={opened.entry} onClose={() => setOpened(null)} />
      )}
    </div>
  );
}
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx

import { McpServersTab, serverStatus } from "./McpServersTab";
import type { PluginEntry, ServerEntry, SkillEntry } from "./parts";
import { PluginsTab } from "./PluginsTab";
```

**with:**

```tsx

import { ContextTab } from "./ContextTab";
import { McpServersTab, serverStatus } from "./McpServersTab";
import type { ContextEntry, PluginEntry, ServerEntry, SkillEntry } from "./parts";
import { PluginsTab } from "./PluginsTab";
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx

/// The Library: the MCP servers, skills and plugins that dots can use. `?review=<server id>`
/// opens the review of that server's changed tool list (the Inbox links to it).
```

**with:**

```tsx

/// The Library: the MCP servers, skills, plugins and context collections that dots can use. `?review=<server id>`
/// opens the review of that server's changed tool list (the Inbox links to it).
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx
  const plugins = entries.filter((e): e is PluginEntry => e.kind === "plugin");

```

**with:**

```tsx
  const plugins = entries.filter((e): e is PluginEntry => e.kind === "plugin");
  const contexts = entries.filter((e): e is ContextEntry => e.kind === "context");

```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx
          <TabsTrigger value="plugins">Plugins</TabsTrigger>
        </TabsList>
```

**with:**

```tsx
          <TabsTrigger value="plugins">Plugins</TabsTrigger>
          <TabsTrigger value="context">Context</TabsTrigger>
        </TabsList>
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx
        </TabsContent>
      </Tabs>
```

**with:**

```tsx
        </TabsContent>
        <TabsContent value="context">
          <ContextTab entries={contexts} />
        </TabsContent>
      </Tabs>
```

**In `ui/src/screens/library/parts.tsx`, replace:**

```tsx
export type PluginEntry = Extract<LibraryEntry, { kind: "plugin" }>;

```

**with:**

```tsx
export type PluginEntry = Extract<LibraryEntry, { kind: "plugin" }>;
export type ContextEntry = Extract<LibraryEntry, { kind: "context" }>;

```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/library`
Expected: PASS.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: the UI 381 in 25 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/library/ContextTab.tsx ui/src/screens/library/LibraryScreen.test.tsx ui/src/screens/library/LibraryScreen.tsx ui/src/screens/library/parts.tsx ui/src/test/fixtures.ts
git commit -m "feat(ui): a Library tab for context collections, with their documents"
```

### Task 8: Bring the context library and main specs in line with what was built

The spec at `b904af4` is changed where the table "Deviations from the spec" says, with a section 13 that lists the changes; the main spec gets the passages of context library spec section 12.

**Files:**

- Modify: `docs/superpowers/specs/2026-10-08-dots-design.md`
- Modify: `docs/superpowers/specs/2026-10-19-context-library-design.md`

**Interfaces:**

- Consumes: Tasks 1 to 7.
- Produces: the specs.

- [ ] **Step 1: Edit the specs**

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
rule when no rule names `Skill`, and a Trusted policy loses the preset's old `mcp__*` ask rule.

```

**with:**

```markdown
rule when no rule names `Skill`, and a Trusted policy loses the preset's old `mcp__*` ask rule.
From the context library (`2026-10-19-context-library-design.md` §8), migration 0007 adds
`dots.use_global_instructions` (default 1), and its code step (`finish_0007`, after
`finish_0006`) gives every stored policy a `Context` allow rule at the end when no rule names
`Context`. `settings` gains `global_instructions` (default empty, at most 32 KB) and `dots.uses`
gains `context` (a missing key reads as none).

```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   standing rules: work only inside the workspace; when done, end with a summary listing what
   changed and what needs human review.
5. **Execute.** Engine events are appended to `run_events` and broadcast to the UI.
```

**with:**

```markdown
   standing rules: work only inside the workspace; when done, end with a summary listing what
   changed and what needs human review. From the context library: when the global instructions
   of Settings are not blank and the dot uses them, the prompt starts with
   `## Instructions for every dot`, the text, and `## Instructions for this dot`; a run with
   context collections ends the prompt with a `## Shared context` section that lists them and
   names the tool (`context` on local dots, `mcp__dots__context` on Claude dots). Resumes and
   chat follow-ups still send only their message.
5. **Execute.** Engine events are appended to `run_events` and broadcast to the UI.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- Presets:
  - Every preset allows `Skill` (the tool that reads a skill) and sets `mcp_default`: `ask` for
    all but `read-only`, which sets `deny`.
  - `read-only`: allow Read/LS/Glob/Grep; deny Write/Edit/Bash; default deny.
```

**with:**

```markdown
- Presets:
  - Every preset allows `Skill` (the tool that reads a skill) and, from the context library,
    `Context` (the tool that reads the dot's context collections), and sets `mcp_default`:
    `ask` for all but `read-only`, which sets `deny`. A rule of `Context` is not passed to the
    CLI's `--allowedTools` / `--disallowedTools`: the CLI has no such tool, and Dots' own
    server checks each call against it.
  - `read-only`: allow Read/LS/Glob/Grep; deny Write/Edit/Bash; default deny.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
reaches PowerShell on stdin behind a small fixed `-EncodedCommand` bootstrap, so its length is
not bound by the Windows command-line limit), and from MCP/skills `skill` (policy name `Skill`,
offered only to a run with skills). Their
argument names match the Claude tools they alias (`file_path`, `content`, `old_string`,
```

**with:**

```markdown
reaches PowerShell on stdin behind a small fixed `-EncodedCommand` bootstrap, so its length is
not bound by the Windows command-line limit), from MCP/skills `skill` (policy name `Skill`,
offered only to a run with skills), and from the context library `context` (policy name
`Context`, offered only to a run with context collections; arguments `collection`, `query`,
`path`, `offset`; context library spec §3). Their
argument names match the Claude tools they alias (`file_path`, `content`, `old_string`,
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  it, the switch "Run this plugin's hooks"; "Add plugin" picks one of the installed Claude
  Code plugins or a folder. Removing asks first and names the dots. The screen never asks a
  second time for what the core confirms in a native dialog (a program to run, env changes, a
```

**with:**

```markdown
  it, the switch "Run this plugin's hooks"; "Add plugin" picks one of the installed Claude
  Code plugins or a folder. From the context library, Context: name, "every dot" badge,
  document count, description, folder, the dots that tick it; "Add collection" picks a folder
  and asks for name, description and "Every dot reads it"; "Documents" lists the first 200
  documents; "Edit" changes all but the name. Removing asks first and names the dots. The screen never asks a
  second time for what the core confirms in a native dialog (a program to run, env changes, a
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  From MCP/skills a section "Tools and skills" (`screens/dot-editor/ToolsAndSkills.tsx`) edits
  the dot's `uses` as three checklists from the Library (MCP servers, skills, plugins; a
  plugin's own servers are not listed: they come with the plugin). A server whose tools changed
  or that was unreachable says so on its row. A name the Library no longer has stays ticked,
```

**with:**

```markdown
  From MCP/skills a section "Tools and skills" (`screens/dot-editor/ToolsAndSkills.tsx`) edits
  the dot's `uses` as checklists from the Library (MCP servers, skills, plugins and, from the
  context library, Context, where a collection for every dot shows ticked and disabled; a
  plugin's own servers are not listed: they come with the plugin). From the context library,
  Basics has the switch "Start with the global instructions". A server whose tools changed
  or that was unreachable says so on its row. A name the Library no longer has stays ticked,
```

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

```markdown

Date: 2026-10-19. Status: written without a question round (the person said "go"); every decision below is the writer's call unless marked otherwise, for the person to check while reading.
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md` and the MCP/skills spec `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` (section 12 lists the passages it changes). Source of the idea: Approvals v2 spec section 14 ("a shared context library that many dots read") and the review of Cloudflare OS (`packages/gatekeeper-context`, `AdminConfig.instanceInstructions`).
```

**with:**

```markdown

Date: 2026-10-19. Status: written without a question round (the person said "go"); every decision below is the writer's call unless marked otherwise, for the person to check while reading. Brought in line with what the plan's prototype built (section 13).
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md` and the MCP/skills spec `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` (section 12 lists the passages it changes). Source of the idea: Approvals v2 spec section 14 ("a shared context library that many dots read") and the review of Cloudflare OS (`packages/gatekeeper-context`, `AdminConfig.instanceInstructions`).
```

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

````markdown

A run with collections gets, after the standing rules of the first prompt (Claude engine) or in the system prompt after the skills section (local engine):

```
Shared context you can read. Call the tool `context` with a collection name to list its documents, `query` to search it, or `path` to read one. Look here before you guess how the person works.
````

**with:**

````markdown

A run with collections gets, on both engines, a section at the end of its first prompt, after the standing rules (the local engine sends that prompt as its first user message):

```
## Shared context

Shared context you can read. Call the tool `context` with a collection name to list its documents, `query` to search it, or `path` to read one. Look here before you guess how the person works.
````

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

```markdown
- `--allowedTools` gains `mcp__dots__context` for a run with collections, so the CLI does not first ask `approve` (the server checks it instead, as the gateway does for MCP servers).
- The prompt section of this section names the tool `context`; the CLI shows it as `mcp__dots__context`, and the section says so in its first line for the Claude engine: "Call the tool `mcp__dots__context` …".

```

**with:**

```markdown
- `--allowedTools` gains `mcp__dots__context` for a run with collections, so the CLI does not first ask `approve` (the server checks it instead, as the gateway does for MCP servers).
- The prompt section names the tool as the engine's model sees it: `mcp__dots__context` on the Claude engine, `context` on the local engine (`prompt::context_tool_name`).
- A rule of `Context` is left out of `--allowedTools` and `--disallowedTools` (`Policy::cli_lists`): the CLI has no such tool, and the `dots` server checks each call against the rule itself.

```

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

```markdown

- `RunTools::open` resolves the run's collections (section 3), checks each folder exists, and counts its documents (bounded walk). `RunTools` gains `contexts() -> &[RunContext…]` with name, folder, description and count. Nothing is opened or held: each call reads the folder afresh.
- `OpenRequest` is unchanged; the store gives the `every_dot` collections.

```

**with:**

```markdown

- `RunTools::open` resolves the run's collections (section 3) with `Store::used_collections(uses)` (the ticked ones and every `every_dot` one, by name), walks each folder and counts its documents. `RunTools::contexts() -> &[context::Collection]` gives name, folder, description and count. Nothing is opened or held: each call reads the folder afresh.
- `OpenRequest` is unchanged.

```

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

```markdown
- **Settings:** a "Global instructions" text area (with the 32 KB limit shown), saved with the other settings, and a line: "Every dot's first prompt starts with these. A dot can switch them off in its editor."
- **Library:** a fourth tab **Context** (after Plugins): name, description, folder, document count, "Every dot" badge, the dots that tick it. "Add collection" picks a folder, then name (default: the folder name, made valid), description, "Every dot" switch. Each row has "Open folder" and "Documents" (a list of the first 200 documents with title and summary, from a new command). Edit and delete as for other kinds.
- **Dot editor:** in "Tools and skills", a fourth checklist **Context** (an `every_dot` collection shows ticked and disabled, with "every dot"), and a switch "Start with the global instructions" (shown when the setting is not blank; its state is saved either way).
- **Run view:** `context` calls show as other tool calls (`context` on local dots, `dots › context` on Claude dots).
```

**with:**

```markdown
- **Settings:** a "Global instructions" text area (with the 32 KB limit shown), saved with the other settings, and a line: "Every dot's first prompt starts with these. A dot can switch them off in its editor."
- **Library:** a fourth tab **Context** (after Plugins): name, description, folder, document count, "Every dot" badge, the dots that tick it (an `every_dot` collection says "Every dot reads it" instead). "Add collection" picks a folder, then name (default: the folder name, lower case, other characters as `-`), description, "Every dot reads it" switch, with the line "Dots read these files as guidance. Only add folders whose writers you trust." Each row has "Documents" (a list of the first 200 documents with title and summary, from a new command), "Edit" (all but the name) and "Remove". There is no "Open folder": no command opens an arbitrary folder, and the plan adds no plugin; the row shows the path.
- **Dot editor:** in "Tools and skills", a fourth checklist **Context** (an `every_dot` collection shows ticked and disabled, with "every dot"), and in Basics a switch "Start with the global instructions", always shown, with the note that the text is set in Settings and a blank text adds nothing.
- **Run view:** `context` calls show as other tool calls (`context` on local dots, `dots › context` on Claude dots).
```

**In `docs/superpowers/specs/2026-10-19-context-library-design.md`, replace:**

```markdown

- `dots.uses` rows gain `"context": []` lazily: `Uses` reads a missing key as empty, so no SQL rewrite is needed.
- Stored policies gain the `Context` allow rule (section 3), done where 0006 added `Skill` (`store/upgrade.rs` pattern), once, recorded by the migration.
- `use_global_instructions` defaults to `true` through serde; `global_instructions` defaults to empty through `Settings`' `#[serde(default)]`.
```

**with:**

```markdown

- The SQL adds `dots.use_global_instructions INTEGER NOT NULL DEFAULT 1` and lists every dot in a table `upgrade_0007`.
- When the store opens, `upgrade::finish_0007` runs after `finish_0006`: each listed dot's policy gets the `Context` allow rule at the end unless a rule names `Context` (`Context` or `Context(...)`), written through `Policy`, and the table is dropped. Running after `finish_0006` keeps the rules in the presets' order (`Skill`, then `Context`) when a database from before 0006 is opened.
- `dots.uses` rows gain `"context": []` lazily: `Uses` reads a missing key as empty, so no SQL rewrite is needed.
- `use_global_instructions` defaults to `true` through serde; `global_instructions` defaults to empty through `Settings`' `#[serde(default)]`.
```

**Append to `docs/superpowers/specs/2026-10-19-context-library-design.md`:**

```markdown

## 13. Changes made while building

The plan's prototype changed these from the first version of this spec:

- The prompt section is part of the first prompt on both engines, not the local engine's system prompt, so one function (`build_prompt`) writes it for both.
- `Context` rules stay out of the CLI's tool lists (section 4).
- Migration 0007's policy step is code that runs after 0006's (section 8): as SQL it ran before `finish_0006` and put `Context` ahead of `Skill`.
- The Library row has no "Open folder" and the editor's switch is always shown (section 6).
```

- [ ] **Step 2: Run the checks**

Run: `git diff --cached --check` after staging.
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-10-08-dots-design.md docs/superpowers/specs/2026-10-19-context-library-design.md
git commit -m "docs(spec): bring the context library and main specs in line with what was built"
```

### Task 9: The smoke test of the context library

Agent steps with LM Studio (`~/.lmstudio/bin/lms`, model `google/gemma-4-12b-qat`) and the debug app on a temp `DOTS_DATA_DIR`, as in the MCP/skills smoke (`docs/verification/`); person steps where the real `claude` CLI is needed (the agent never runs it). Write the results to `docs/verification/2026-10-19-context-library-smoke.md` and commit it with `docs(verification): the smoke of the context library`.

- [ ] **Step 1: Agent: a local dot reads a collection.** Make a folder with `style.md` (`# House style` and two lines) and `people/ana.md`; add it in the Library's Context tab as `company`; tick it on a local dot whose instructions say "Search the company collection for the house style and quote it". Run now. Expected: the run's first prompt ends with `## Shared context` naming `company (2 documents)`; the transcript shows `Context` calls (a search, then a read); the summary quotes the file.
- [ ] **Step 2: Agent: every dot and the global instructions.** Switch "Every dot reads it" on for `company`, set Settings → Global instructions to "Sign every summary with — Dots", and run a second local dot that ticks nothing. Expected: its first prompt starts with `## Instructions for every dot` and lists `company`; its summary is signed. Switch the dot's "Start with the global instructions" off and run again: the heading is gone.
- [ ] **Step 3: Agent: refusals.** On the first dot add a rule `Context` deny and run: the calls are denied and the run still succeeds. Remove the folder and run: the run event says `context company: folder not found` and the prompt has no section.
- [ ] **Step 4: Person: a Claude dot.** With a real `claude` CLI, tick `company` on a Claude dot and run it with "Read the house style from the shared context and quote it." Expected: the transcript shows `dots › context` calls that succeed without an approval; under a `Context` ask rule the call reaches the Inbox and, approved, the resumed run reads the file.
