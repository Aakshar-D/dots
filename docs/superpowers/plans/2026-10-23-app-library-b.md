# App Library, Plan B (screens, bridge, review card) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The person uses Dots' apps: an Apps place on the rail lists them and opens each one's screen in a sandboxed frame that talks to Dots only through a checked bridge; a dot's proposal arrives as a review card with the app running for real on a throwaway database, its code or diff and its summary; the dot editor ticks apps; and a dot can copy an installed app into its workspace to change it.

**Architecture:** The core gains `apps::preview` (a proposal's code on a database in memory, as the person, starting no dot; the frame's `ui/` files by version), `apps::kept` (the data a removal kept, listed and offered to a new app of the same name), the Code tab's diff (`screen::diff_files`, through the `similar` crate), `copy_app` on both engines, and the Inbox's waiting reviews. The Tauri app serves the apps' pages through its own protocol `dots-app` (`app_frame.rs`): only a page's own `ui/` files, each with a CSP limited to its folder and with Dots' client (`app_client.js`) put into its `index.html`. The UI frames a page with `sandbox="allow-scripts"`, hands it one `MessagePort` and answers its calls (`lib/app-bridge.ts`, `components/AppFrame.tsx`); `/apps` and `/apps/:name` are new screens, the review card stands in the conversation under the run that sent the app, and the editor's "Tools and skills" gains an Apps checklist.

**Tech Stack:** Rust 1.95, sqlx 0.8, `rusqlite` 0.32, `rquickjs` 0.14, tokio; new: `similar` 2; Tauri 2.12 (`register_asynchronous_uri_scheme_protocol`); Bun 1.3, React 19.3, TypeScript 6.0, zod 4.6, Vitest 5.0; WebView2.

**Spec:** `docs/superpowers/specs/2026-10-23-app-library-design.md` on `master` (plan A's §15 says what the core built). Task 14 adds §17, "Changes made while building (plan B)", which the table "Deviations" lists.

**Plans before and after this one:** A, the core (merged, PR #24); C, the bundled Projects app, whose screen (the approved mockup A) this plan's frame hosts: a full-height page of its own, with its own columns and side pane.

**Where the tasks come from:** spec §15 "Left to plan B" and §6.2 (Tasks 1, 2, 4, 6, 11), §5 "Screen" (Tasks 8, 9, 13), §10 (Tasks 6, 10, 12), §15's dot-side tool (Task 5), plan A's deferred findings (Tasks 3, 4, 10) and the parked Minors the person chose (Tasks 4, 7).

**How this plan's code was checked:** the code blocks of Tasks 1 to 14 were generated from a throwaway worktree of `bd953cc`, one commit per task, where they compiled and passed on Windows 11, and the plan was then replayed onto `bd953cc` by a script that applied every block and compared the tree with each task commit (all 14 tasks matched; the contract fixture and `Cargo.lock` were taken from the task commit, as the commands write them). Before the first task: `cargo test -p dots-core` 1621 passed (ten of them, in `claude_gateway_test`, `resume_test`, `runner_workflow_test` and `runtime_browser_test`, only when run alone: they time out under the full run's load), `cargo test -p dots-app` 124, `bun run --cwd ui test` 748 tests in 71 files. At the last task: `dots-core` 1643, `dots-app` 129, the UI 781 in 77 files; `cargo fmt --check` and `tsc --noEmit` clean; clippy reports only the base's own warning. Task 13's check ran in a debug build driven over WebView2's remote-debugging port, on a temp data folder, with `fake-claude`: a dot proposed the probe app, its card showed the preview, "Add to library" added it, and the probe's page reached only the bridge, the theme and its own files. No agent ran the real `claude` CLI.

## Global Constraints

- Toolchain: Rust 1.95 stable; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/app-library-b`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\app-library-b`, based on `bd953cc` (`origin/master`). All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Cargo runs one chain at a time, with `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target CARGO_INCREMENTAL=0` once per shell and `-j 2`. Before a full run, check the free space on C: (`powershell -NoProfile -Command "(Get-PSDrive C).Free/1GB"`): an empty test summary can mean a full disk. UI tests run after cargo has finished.
- New dependencies: only `similar = "2"` in `crates/dots-core` (Task 2). No migration. No new Tauri plugin or capability; the `dots-app` protocol is registered on the builder (Task 8), and Dots' CSP gains `frame-src http://dots-app.localhost`.
- The subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials or calls an Anthropic API. No agent runs the real `claude` CLI (not even with a prompt); tests and smokes use `fake-claude`. A real-CLI smoke is the person's.
- Values the spec fixes: the frame `sandbox="allow-scripts"` (no same origin, forms or popups); the page's CSP `default-src 'none'; script-src 'unsafe-inline' <app files>; style-src 'unsafe-inline' <app files>; img-src data: <app files>; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'`; `dots.call(tool, input)`, `dots.onChange(cb)`; the actor of a screen call is always the person; the theme is the Deep Teal tokens; a preview's changes vanish when the card closes and it starts no dot; the card's tabs Preview, Code, Summary and its buttons "Add to library" (or "Update"), "Ask for changes", "Discard"; "Sees the names of the dots that use it"; "Nothing" or "Can start dots"; the Apps entry between Inbox and Library; "Changed by you; Dots no longer updates it"; a change keeps the active version's migrations byte for byte; "Ask for changes" at most 2000 characters.
- No agent installs or runs an installer, runs a release build, or touches the installed Dots (`%LOCALAPPDATA%\Dots`) or the real data folder `%LOCALAPPDATA%\dots`. The WebView2 check of Task 13 runs a debug build on a temp `DOTS_DATA_DIR`, with a copy of `fake-claude` outside `target/`.
- A background command that Claude Code stops for low memory is reported, not restarted without the person's word.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit (the blocks hold `§`, `—`, `›`, `●`, `✓`, `✗` and `…`). The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- Live data is parsed, not cast, with strict schemas: a type the UI parses changes only together with the contract fixture and `ui/src/api/types.ts`, in the same commit (Tasks 4, 6, 8). The fixture `ui/src/api/__fixtures__/contract.json` is written by `UPDATE_CONTRACT=1 cargo test -p dots-app contract` and never by hand. `contract.rs`'s `samples()` is at the `json!` macro's recursion limit: a new sample is added after the macro (`samples["X"] = …`).
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`), `cargo clippy -j 2 --workspace --all-targets` (no warning but the one the base already has: `watchers/github.rs`, `nonminimal_bool`), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first. The UI files have no formatter; keep new lines within 100 columns where the file does. A UI run counts only when "Test Files N passed (N)" equals the number of `*.test.ts*` files under `ui/src`; a `[vitest-pool-runner]: Timeout waiting for worker to respond` run is run again, not accepted.
- Known flaky under load (rerun the test binary alone before treating one as a failure): `claude_gateway_test::a_claude_run_reaches_its_servers_through_the_gateway_only`, the worktree tests of `runtime_worktree_test` and `workflow_test` (a run "did not reach AwaitingApproval" under load), `schedule_test::newest_slot_in_a_long_window_does_not_walk_every_slot`, `dots-app startup::tests::a_panic_is_written_to_the_log_file`, `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`.
- Fact-Forcing Gate (a hook of this machine): the first Edit or Write of each file, the first Bash command after a task notification, and `rm`, `git commit --amend` or a rebase are blocked until the facts it asks for are stated; then retry the identical call. For a large new file, write a one-line stub first.

## Deviations from the spec

The spec leaves these open or says otherwise; Task 14 records them in a new §17.

| Spec | This plan | Why |
|---|---|---|
| §5: how `ui/` files reach the frame is the builder's call | The protocol `dots-app` serves `/app/<name>/<version>/<path>` and `/preview/<id>/<path>`, only files below `ui/`, `GET` only; each answer's CSP names the page's own folder, and `index.html` gets the client right after its doctype | A `srcdoc` or `blob:` frame would inherit Dots' CSP (§14); a folder in the path lets the CSP keep a page to its own files |
| §5: the CSP | The spec's list, plus `font-src <app files>`; no `'self'` | An app's fonts are its files; `'self'` in an opaque origin names nothing |
| §5: `dots.call`, `dots.onChange` | Also `dots.onTheme(cb)`; the theme arrives as CSS variables `--dots-<token>` on the root, with `data-theme` | An app matches Dots by using the variables |
| §5: `dots.onChange` fires when a dot changes the data | It fires on every change of the app's data, the screen's own included | `app_data_changed` names only the app |
| §5: one port per frame | A page that loads a second time (it navigated away) is cut off: "The app's screen left its page", with "Load the app again" | A port must not pass to a page Dots did not serve |
| §6.2: the preview's throwaway database | A database in memory per preview, at most 8 open (opening a ninth closes the oldest); its `ctx.dots.list()` gives the dots that use the app now (none for a new app); deciding a proposal closes its previews | The card may be opened many times; a new app has no users yet |
| §6.2: Code shows every file, and a change's diff | A change shows "What it changes" (`DiffView`) above "Every file"; the diff comes from the `similar` crate | No git repository holds an app's versions |
| §10: Remove deletes the data "after a second confirmation" | Kept data gets a `removed.json`, is listed under "Kept data" on `/apps` with "Delete data", and a new app of the same name whose migrations begin with the ones that ran on it, byte for byte, is offered to start on it ("Add to library" with "Start on the data kept …" ticked, the decision `add_with_kept_data`) | The person's choice in the question round |
| §10: a waiting card counts in Needs you | The Inbox also lists "Apps to review", each with "Open" to its card; the rail, the tray and the dot's needs-you count include it | The person's choice |
| §15: a dot-side tool to read an installed app | `copy_app` `{ app, path }` under `ProposeApp`, on both engines: any installed app's active version, as its version folder holds it, into a new or empty folder of the workspace | A dot may change Projects or another dot's app; it never overwrites a dot's own files |
| Plan A deferred: "Started by an app" | The transcript says "Started by <title>", the run card's label is the title, and the app's message stands under the header | The title is in the run's payload |
| Plan A deferred: the bare `App` candidate | "Always allow" offers `App(<app>.<tool>)` and `App(<app>)` | A bare `App` would allow every app's writes |
| Plan A deferred: off-policy dots listed | An app's users (the screen's row, `ctx.dots.list()`, starts) are the dots that tick it, or all for "Every dot", whose policy leaves them one of its tools | A dot that cannot call the app does not use it |
| Parked: call-log results | At most 30 KB of a result is logged, as of an input | The person folded it in |
| Parked: test time | An app's test files run 60 s together at most; a file past it fails without running | The person folded it in |
| Parked: the thread lock across accept | Accepting lets go of the thread lock while the migrations run; the thread is marked as deciding, so no chat, reply, follow-up or second decision continues it ("Dots is still adding the app you accepted in this chat; …") | The person folded it in |
| Parked: `app_topics` pruning | Removing an app deletes its topics; rows of apps no longer installed are pruned at start | The person folded it in |

The spec leaves to the builder, and the plan settles: the component layout of the Apps screen (a row per app with its switches, then "Kept data") and of the card (header, tabs, footer), the protocol and its paths, the frame's 420 px height in the card, and the client's message shapes (`{ type: "call", id, tool, input }`, `{ type: "result", id, ok, result | error }`, `{ type: "theme", mode, tokens }`, `{ type: "changed" }`, `{ type: "dots:connect" }`). Not folded in: the parked `Atomics.wait` test.

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. An app's page that tries to reach past its sandbox: Tauri's IPC, the network, the parent page, another file, storage. Expected: nothing reached; only its own files load. Tests: the WebView2 check of Task 13 (the probe app's results), `app_frame::tests::the_page_may_load_only_its_own_files_and_reach_nothing` (Task 8), `AppFrame.test.tsx` "calls a listed tool as the page asks and refuses one it does not list" (Task 9).
2. A page that navigates away or is reloaded by its own code. Expected: the bridge is cut, and the person loads the app afresh. Test: `AppFrame.test.tsx` "cuts off a page that navigated away, and loads it afresh when asked" (Task 9).
3. Changes made in a preview. Expected: they never reach real data, no dot starts, and they are gone once the card closes. Tests: `apps_preview_test::a_preview_runs_on_the_sample_data_and_its_changes_vanish_when_it_closes`, `a_preview_calls_as_the_person_only_listed_tools_and_cannot_start_dots` (Task 1).
4. Kept data that no longer fits, or was deleted after the card was posted. Expected: not started on, nothing added, the card still waits. Tests: `apps_kept_test::kept_data_that_does_not_fit_is_not_offered_and_the_app_starts_empty`, `kept_data_deleted_after_the_card_was_posted_is_not_started_on` (Task 4).
5. The person writes in the chat while an update they accepted migrates real data. Expected: that chat is refused with a sentence, other chats go on, and the update lands once. Test: `apps_propose_test::while_an_update_migrates_its_thread_waits_and_other_threads_go_on` (Task 7).

## File Map

- Core: `crates/dots-core/src/apps/{preview,kept}.rs` (new, Tasks 1, 4); `src/apps/{mod,registry,screen,propose,run,tests,wake}.rs`; `src/{policy,model,runtime,runner,checkins,followups}.rs`; `src/store/{apps,dots}.rs`; `src/engine/claude/args.rs`; `src/engine/local/{mod,mcp_tools,app_tools}.rs`, `src/engine/local/tools/mod.rs`; `src/server/mcp.rs`; `Cargo.toml`.
- Core tests: `crates/dots-core/tests/apps_{preview,diff,kept,copy}_test.rs` (new); `tests/apps_{propose,screen,policy,calls,tests}_test.rs`, `tests/common/apps.rs`.
- Tauri: `apps/dots-app/src/{app_frame.rs,app_client.js}` (new, Task 8); `src/{lib,commands,contract}.rs`; `tauri.conf.json`.
- UI: `ui/src/lib/app-bridge.ts`, `ui/src/components/{AppFrame,AppReviewCard}.tsx`, `ui/src/screens/apps/{AppsScreen,AppScreen}.tsx` (new); `ui/src/api/{types,commands}.ts`, the fixture, `ui/src/api/contract.test.ts`, `ui/src/lib/{query,runtime-events,format,home-topics}.ts`, `ui/src/app/{Layout,routes}.tsx`, `ui/src/components/Transcript.tsx`, `ui/src/screens/InboxScreen.tsx`, `ui/src/screens/home/TopicRuns.tsx`, `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, `ui/src/test/fixtures.ts`; tests beside them, and the Inbox literals of existing tests.
- Smoke: `tools/smoke/apps/probe/` (new, Task 13).
- Spec: `docs/superpowers/specs/2026-10-23-app-library-design.md` (Task 14).

### Task 1: A review card's preview, and the frame's files

Spec §6.2 "Preview" and §5 "Screen": a proposal's code running for real, as the person, on a database in memory built from its migrations and `sample.sql`, with no `ctx.dots.start`; and the `ui/` files a frame may load, by app version or by preview.

**Files:**

- Create: `crates/dots-core/src/apps/preview.rs`, `crates/dots-core/tests/apps_preview_test.rs`
- Modify: `crates/dots-core/src/apps/{mod,registry,propose}.rs`, `crates/dots-core/tests/common/apps.rs`

**Interfaces:**

- Consumes: plan A's `Apps::{get, load, proposed_dir, propose}`, `call::{call, Call, Actor, DotRef}`, `data::{open_in_memory, migrate, load_sample}`, `source::read_folder`.
- Produces: `apps::preview::MAX_PREVIEWS = 8`; `Apps::preview_open(proposal_id) -> Result<String>` (a decided proposal is `Conflict(DECIDED_ALREADY)`), `Apps::preview_call(preview, tool, input) -> Result<Value>` (`Invalid` with the rule the call broke, `NotFound` once closed), `Apps::preview_close(preview)`, `Apps::preview_file(preview, path) -> Result<Vec<u8>>`, `Apps::ui_file(name, version, path) -> Result<Vec<u8>>` (`NotFound` for another version or a path outside `ui/`); `Apps::drop_proposed` closes the proposal's previews. Inside the crate: `Apps::listed_dots(&item) -> Result<Vec<DotRef>>` and `Apps::call_now() -> String` (Task 3 changes `listed_dots`). Tests: `common::apps::proposing_run(&store, name) -> Run`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/apps_preview_test.rs`:**

```rust
//! The review card's preview and the frame's files (app library spec sections 5 and 6.2): a
//! proposal runs for real on a throwaway database, as the person, starting no dot; a frame gets
//! only the `ui/` files of the version it opened.

mod common;

use common::apps::{expenses, expenses_with, proposing_run, write_app, MANIFEST, UI, UI_SCRIPT};
use dots_core::apps::preview::MAX_PREVIEWS;
use dots_core::apps::propose::{AppProposal, SENT};
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Store, Apps) {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    (dir, store, apps)
}

/// `files` proposed by a new dot `name`.
async fn proposed(
    dir: &tempfile::TempDir,
    store: &Store,
    apps: &Apps,
    name: &str,
    files: &[(&str, String)],
) -> AppProposal {
    let run = proposing_run(store, name).await;
    let folder = dir.path().join(format!("work-{name}"));
    write_app(&folder, files);
    assert_eq!(
        apps.propose(&run.id, folder, "Built it.").await,
        Ok(SENT.to_string())
    );
    store
        .list_app_proposals(Some(&run.dot_id))
        .await
        .unwrap()
        .remove(0)
}

fn count(rows: &serde_json::Value) -> usize {
    rows.as_array().unwrap().len()
}

#[tokio::test]
async fn a_preview_runs_on_the_sample_data_and_its_changes_vanish_when_it_closes() {
    let (dir, store, apps) = fixture().await;
    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let month = json!({ "month": "2026-10" });
    let preview = apps.preview_open(&p.id).await.unwrap();
    let rows = apps
        .preview_call(&preview, "list_expenses", month.clone())
        .await
        .unwrap();
    assert_eq!(count(&rows), 2, "the sample's two rows: {rows}");
    let added = json!({ "item": "Lunch", "amount": 9.5, "on": "2026-10-23" });
    apps.preview_call(&preview, "add_expense", added)
        .await
        .unwrap();
    let rows = apps
        .preview_call(&preview, "list_expenses", month.clone())
        .await
        .unwrap();
    assert_eq!(count(&rows), 3);
    // Nothing reached the Library.
    assert!(apps.list().await.unwrap().is_empty());

    apps.preview_close(&preview);
    let closed = apps
        .preview_call(&preview, "list_expenses", month.clone())
        .await;
    assert!(matches!(closed, Err(Error::NotFound(_))), "{closed:?}");
    let again = apps.preview_open(&p.id).await.unwrap();
    let rows = apps
        .preview_call(&again, "list_expenses", month)
        .await
        .unwrap();
    assert_eq!(
        count(&rows),
        2,
        "a new preview starts from the sample again"
    );
}

#[tokio::test]
async fn a_preview_calls_as_the_person_only_listed_tools_and_cannot_start_dots() {
    let (dir, store, apps) = fixture().await;
    let manifest = MANIFEST.replacen(
        "\"tools\": [",
        "\"permissions\": [\"start_dots\"],\n  \"tools\": [\n    { \"name\": \"who\", \
         \"description\": \"Who calls\", \"input\": { \"type\": \"object\" }, \"writes\": true },",
        1,
    );
    let logic = format!(
        "{}\nexport function who(input, ctx) {{ return {{ actor: ctx.actor.kind, \
         start: typeof ctx.dots.start }}; }}\n",
        common::apps::LOGIC
    );
    let mut files = expenses_with("app.json", Some(&manifest));
    files.retain(|(path, _)| *path != "logic.js");
    files.push(("logic.js", logic));
    let p = proposed(&dir, &store, &apps, "builder", &files).await;
    let preview = apps.preview_open(&p.id).await.unwrap();
    let who = apps.preview_call(&preview, "who", json!({})).await.unwrap();
    assert_eq!(who, json!({ "actor": "person", "start": "undefined" }));
    let unlisted = apps.preview_call(&preview, "on_run_end", json!({})).await;
    assert!(matches!(unlisted, Err(Error::Invalid(_))), "{unlisted:?}");
    let bad = apps
        .preview_call(&preview, "add_expense", json!({ "item": "Lunch" }))
        .await;
    match bad {
        Err(Error::Invalid(m)) => assert!(m.contains("amount"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_frame_gets_only_the_ui_files_of_its_version_or_its_preview() {
    let (dir, store, apps) = fixture().await;
    let src = tempfile::tempdir().unwrap();
    write_app(src.path(), &expenses());
    apps.install(read_folder(src.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let index = apps.ui_file("expenses", 1, "index.html").await.unwrap();
    assert_eq!(String::from_utf8(index).unwrap(), UI);
    let script = apps.ui_file("expenses", 1, "app.js").await.unwrap();
    assert_eq!(String::from_utf8(script).unwrap(), UI_SCRIPT);
    for (version, path) in [
        (2, "index.html"),
        (1, "../logic.js"),
        (1, "logic.js"),
        (1, ""),
    ] {
        let refused = apps.ui_file("expenses", version, path).await;
        assert!(
            matches!(refused, Err(Error::NotFound(_))),
            "{path}: {refused:?}"
        );
    }
    let missing = apps.ui_file("nothing", 1, "index.html").await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");

    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let preview = apps.preview_open(&p.id).await.unwrap();
    let index = apps.preview_file(&preview, "index.html").unwrap();
    assert_eq!(String::from_utf8(index).unwrap(), UI);
    assert!(matches!(
        apps.preview_file(&preview, "app.json"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        apps.preview_file("no-such-preview", "index.html"),
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn previews_are_bounded_and_a_decided_proposal_closes_its_own() {
    let (dir, store, apps) = fixture().await;
    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let mut open = Vec::new();
    for _ in 0..=MAX_PREVIEWS {
        open.push(apps.preview_open(&p.id).await.unwrap());
    }
    assert!(matches!(
        apps.preview_file(&open[0], "index.html"),
        Err(Error::NotFound(_))
    ));
    assert!(apps.preview_file(&open[1], "index.html").is_ok());
    apps.drop_proposed(&p.id).await;
    for id in &open {
        assert!(apps.preview_file(id, "index.html").is_err(), "{id}");
    }
    let gone = apps.preview_open(&p.id).await;
    assert!(gone.is_err(), "{gone:?}");
}
```

**In `crates/dots-core/tests/common/apps.rs`, replace:**

```rust

/// Writes `files` below `dir`.
```

**with:**

```rust

/// A dot `name` and a manual run of it in `store`, for `Apps::propose`.
pub async fn proposing_run(store: &dots_core::store::Store, name: &str) -> dots_core::model::Run {
    let dot = store.create_dot(&super::spec(name), None).await.unwrap();
    let new = dots_core::model::NewRun::new(&dot.id, dots_core::model::TriggerKind::Manual);
    store.create_run(&new).await.unwrap()
}

/// Writes `files` below `dir`.
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_preview_test`
Expected: FAIL to compile: no `apps::preview` in `dots_core`, no `preview_open`.

- [ ] **Step 3: Add the previews and the frame's files**

**In `crates/dots-core/src/apps/mod.rs`, replace:**

```rust
pub mod manifest;
pub mod propose;
```

**with:**

```rust
pub mod manifest;
pub mod preview;
pub mod propose;
```

**Create `crates/dots-core/src/apps/preview.rs`:**

```rust
//! The review card's preview and the frame's files (app library spec sections 5 and 6.2). A
//! preview runs a proposal's code for real, as the person, on a database in memory built from
//! its migrations and `sample.sql`: calls work, starting dots does not, and its changes vanish
//! when it closes. The frame of an installed app gets the files below `ui/` of the version it
//! was opened at, and nothing else.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::Value;

use super::call::{self, Actor, Call, DotRef};
use super::data;
use super::engine::Limits;
use super::propose::{ProposalStatus, DECIDED_ALREADY};
use super::registry::blocking;
use super::source::{self, AppSource};
use super::Apps;
use crate::{Error, Result};

/// The most previews open at once; opening one more closes the oldest.
pub const MAX_PREVIEWS: usize = 8;

/// One open preview.
pub(super) struct Preview {
    id: String,
    proposal_id: String,
    src: Arc<AppSource>,
    /// Taken out while a call runs: one call at a time.
    conn: Mutex<Option<Connection>>,
    /// What `ctx.dots.list()` gives: the dots that use the app now; none for a new app.
    dots: Vec<DotRef>,
}

/// The open previews, oldest first.
#[derive(Clone, Default)]
pub(super) struct Previews(Arc<Mutex<Vec<Arc<Preview>>>>);

impl Previews {
    fn find(&self, id: &str) -> Result<Arc<Preview>> {
        self.0
            .lock()
            .expect("previews")
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("preview {id}")))
    }

    fn add(&self, preview: Preview) {
        let mut open = self.0.lock().expect("previews");
        open.push(Arc::new(preview));
        if open.len() > MAX_PREVIEWS {
            open.remove(0);
        }
    }

    fn close(&self, id: &str) {
        self.0.lock().expect("previews").retain(|p| p.id != id);
    }

    /// Closes the previews of the proposal `proposal_id`: it was decided.
    pub(super) fn close_proposal(&self, proposal_id: &str) {
        self.0
            .lock()
            .expect("previews")
            .retain(|p| p.proposal_id != proposal_id);
    }
}

impl Apps {
    /// Opens a preview of the waiting proposal `proposal_id` (spec section 6.2): its migrations
    /// and `sample.sql` run on a new database in memory. Returns the preview's id. A decided
    /// proposal is a `Conflict`.
    pub async fn preview_open(&self, proposal_id: &str) -> Result<String> {
        let p = self.store.get_app_proposal(proposal_id).await?;
        if p.status != ProposalStatus::Pending {
            return Err(Error::Conflict(DECIDED_ALREADY.into()));
        }
        let dots = match (&p.app_id, self.get(&p.app_name).await) {
            (Some(app_id), Ok((item, _))) if item.id == *app_id => self.listed_dots(&item).await?,
            _ => Vec::new(),
        };
        let dir = self.proposed_dir(&p.id);
        let (src, conn) = blocking(move || -> Result<(AppSource, Connection)> {
            let src = source::read_folder(&dir)?;
            let mut conn = data::open_in_memory()?;
            data::migrate(&mut conn, &src.migrations)?;
            if let Some(sample) = &src.sample {
                data::load_sample(&conn, sample)?;
            }
            Ok((src, conn))
        })
        .await??;
        let id = crate::util::new_id();
        self.previews.add(Preview {
            id: id.clone(),
            proposal_id: p.id,
            src: Arc::new(src),
            conn: Mutex::new(Some(conn)),
            dots,
        });
        Ok(id)
    }

    /// A call from the preview's frame: as the person, only a listed tool, and no dot can be
    /// started. The rule the call breaks is `Invalid`; a closed preview is `NotFound`.
    pub async fn preview_call(&self, preview: &str, tool: &str, input: Value) -> Result<Value> {
        let p = self.previews.find(preview)?;
        let now = self.call_now();
        let function = tool.to_string();
        blocking(move || -> std::result::Result<Value, String> {
            let mut held = p.conn.lock().expect("preview database");
            let conn = held
                .take()
                .ok_or("the preview's database was lost; close the preview and open it again")?;
            let conn = Rc::new(RefCell::new(conn));
            let ran = call::call(
                &conn,
                Call {
                    manifest: &p.src.manifest,
                    logic: &p.src.logic,
                    function: &function,
                    input: &input,
                    actor: Actor::Person,
                    now,
                    dots: p.dots.clone(),
                    start: None,
                    limits: Limits::default(),
                },
            );
            *held = Rc::try_unwrap(conn).ok().map(RefCell::into_inner);
            ran.map(|out| out.result).map_err(|e| e.to_string())
        })
        .await?
        .map_err(Error::Invalid)
    }

    /// Closes the preview: its database and its changes are gone.
    pub fn preview_close(&self, preview: &str) {
        self.previews.close(preview);
    }

    /// The file `path` below `ui/` of the preview's proposal.
    pub fn preview_file(&self, preview: &str, path: &str) -> Result<Vec<u8>> {
        let p = self.previews.find(preview)?;
        ui_file(&p.src, path)
    }

    /// The file `path` below `ui/` of the app `name` at `version`: `NotFound` once another
    /// version is active, so a frame opened at an older one is reloaded, not mixed.
    pub async fn ui_file(&self, name: &str, version: u32, path: &str) -> Result<Vec<u8>> {
        let (item, config) = self.get(name).await?;
        if config.version != version {
            return Err(Error::NotFound(format!(
                "version {version} of the app {name}"
            )));
        }
        let src = self.load(&item, &config).await?;
        ui_file(&src, path)
    }
}

fn ui_file(src: &AppSource, path: &str) -> Result<Vec<u8>> {
    src.ui
        .get(path)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("ui/{path}")))
}
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust

    /// Deletes a decided proposal's files.
    pub async fn drop_proposed(&self, id: &str) {
        let dir = self.proposed_dir(id);
```

**with:**

```rust

    /// Deletes a decided proposal's files, and closes its previews.
    pub async fn drop_proposed(&self, id: &str) {
        self.previews.close_proposal(id);
        let dir = self.proposed_dir(id);
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
    starter: Arc<std::sync::OnceLock<Starter>>,
}
```

**with:**

```rust
    starter: Arc<std::sync::OnceLock<Starter>>,
    /// The review cards' previews (spec section 6.2).
    pub(super) previews: super::preview::Previews,
}
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            starter: Arc::default(),
        }
```

**with:**

```rust
            starter: Arc::default(),
            previews: Default::default(),
        }
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
        }
        let dots: Vec<DotRef> = self
            .store
            .dots_with_app(&item)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            // A dot the person switched off is neither listed nor woken (spec section 7), as a
            // webhook or a follow-up does not run it.
            .filter(|d| d.spec.enabled)
            .map(|d| DotRef {
                id: d.id,
                name: d.spec.name,
            })
            .collect();
        let logged =
```

**with:**

```rust
        }
        let dots = self.listed_dots(&item).await.map_err(|e| e.to_string())?;
        let logged =
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            matches!(actor, Actor::Dots) || src.manifest.tool(tool).is_some_and(|t| t.writes);
        let now = self
            .clock
            .now()
            .with_timezone(&Local)
            .to_rfc3339_opts(SecondsFormat::Secs, false);
        // Read under the lock, so two calls cannot both spend the hour's last start.
```

**with:**

```rust
            matches!(actor, Actor::Dots) || src.manifest.tool(tool).is_some_and(|t| t.writes);
        let now = self.call_now();
        // Read under the lock, so two calls cannot both spend the hour's last start.
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            Err(e) => Err(e.to_string()),
        }
    }

```

**with:**

```rust
            Err(e) => Err(e.to_string()),
        }
    }

    /// What `ctx.dots.list()` gives a call of the app `item`: the dots that use it. A dot the
    /// person switched off is neither listed nor woken (spec section 7), as a webhook or a
    /// follow-up does not run it.
    pub(super) async fn listed_dots(&self, item: &LibraryItem) -> Result<Vec<DotRef>> {
        Ok(self
            .store
            .dots_with_app(item)
            .await?
            .into_iter()
            .filter(|d| d.spec.enabled)
            .map(|d| DotRef {
                id: d.id,
                name: d.spec.name,
            })
            .collect())
    }

    /// A call's `ctx.now`: the clock's time, RFC 3339 with the local offset.
    pub(super) fn call_now(&self) -> String {
        self.clock
            .now()
            .with_timezone(&Local)
            .to_rfc3339_opts(SecondsFormat::Secs, false)
    }

```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_preview_test`
Expected: PASS: 4 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1625 passed; clippy reports only the warning the base already has (`watchers/github.rs`: `nonminimal_bool`).

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/apps/mod.rs crates/dots-core/src/apps/preview.rs crates/dots-core/src/apps/propose.rs crates/dots-core/src/apps/registry.rs crates/dots-core/tests/apps_preview_test.rs crates/dots-core/tests/common/apps.rs
git commit -m "feat(core): a review card's preview and the frame's files"
```

### Task 2: The Code tab's diff

Spec §6.2 "Code": for a change, the diff against the active version, in the shape the existing `DiffView` shows (`WorktreeDiff`); for a new app every file is added.

**Files:**

- Modify: `crates/dots-core/Cargo.toml` (`similar = "2"`), `crates/dots-core/src/apps/screen.rs`
- Create: `crates/dots-core/tests/apps_diff_test.rs`

**Interfaces:**

- Consumes: `model::{FileDiff, FileStatus, WorktreeDiff}`, `AppSource::files`.
- Produces: `apps::screen::diff_files(old, new: &BTreeMap<String, Vec<u8>>) -> WorktreeDiff` (unchanged files left out, by path, three lines of context, binary without a patch); `Apps::proposal_diff(id) -> Result<WorktreeDiff>` (`NotFound` once decided).

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/apps_diff_test.rs`:**

```rust
//! The review card's Code tab (app library spec section 6.2): a change shows as the diff against
//! the app's active version, a new app as every file added.

mod common;

use std::collections::BTreeMap;

use common::apps::{expenses, expenses_with, proposing_run, write_app, LOGIC};
use dots_core::apps::propose::SENT;
use dots_core::apps::screen::diff_files;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::model::FileStatus;
use dots_core::Error;

fn files(list: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    list.iter()
        .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
        .collect()
}

#[test]
fn a_diff_lists_what_was_added_removed_and_changed_and_skips_the_rest() {
    let old = files(&[
        ("a.txt", b"one\ntwo\nthree\n"),
        ("gone.txt", b"bye\n"),
        ("same.txt", b"same\n"),
        ("ui/logo.png", &[0x89, 0x50, 0xff, 0x00]),
    ]);
    let new = files(&[
        ("a.txt", b"one\n2\nthree\n"),
        ("new.txt", b"hello\n"),
        ("same.txt", b"same\n"),
        ("ui/logo.png", &[0x89, 0x50, 0xfe, 0x00]),
    ]);
    let diff = diff_files(&old, &new);
    let listed: Vec<(&str, FileStatus, bool)> = diff
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status, f.binary))
        .collect();
    assert_eq!(
        listed,
        [
            ("a.txt", FileStatus::Modified, false),
            ("gone.txt", FileStatus::Deleted, false),
            ("new.txt", FileStatus::Added, false),
            ("ui/logo.png", FileStatus::Modified, true),
        ]
    );
    let a = &diff.files[0].patch;
    assert!(a.contains("@@ -1,3 +1,3 @@"), "{a}");
    assert!(a.contains("-two\n+2\n"), "{a}");
    assert!(diff.files[1].patch.contains("-bye\n"));
    assert!(diff.files[2].patch.contains("+hello\n"));
    assert_eq!(diff.files[3].patch, "");
    assert!(!diff.truncated);
}

#[tokio::test]
async fn a_changes_code_tab_shows_its_diff_and_a_new_apps_shows_every_file() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let run = proposing_run(&store, "builder").await;
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    assert_eq!(
        apps.propose(&run.id, first.clone(), "New.").await,
        Ok(SENT.into())
    );
    let new_app = store.list_app_proposals(None).await.unwrap().remove(0);
    let diff = apps.proposal_diff(&new_app.id).await.unwrap();
    assert_eq!(diff.files.len(), expenses().len(), "{diff:?}");
    assert!(diff.files.iter().all(|f| f.status == FileStatus::Added));
    apps.drop_proposed(&new_app.id).await;
    let decided = apps.proposal_diff(&new_app.id).await;
    assert!(matches!(decided, Err(Error::NotFound(_))), "{decided:?}");

    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let logic = LOGIC.replace("ORDER BY on_day, id", "ORDER BY on_day DESC, id");
    let next = dir.path().join("next");
    write_app(&next, &expenses_with("logic.js", Some(&logic)));
    let run = proposing_run(&store, "fixer").await;
    assert_eq!(
        apps.propose(&run.id, next, "Newest first.").await,
        Ok(SENT.into())
    );
    let change = store
        .list_app_proposals(Some(&run.dot_id))
        .await
        .unwrap()
        .remove(0);
    let diff = apps.proposal_diff(&change.id).await.unwrap();
    assert_eq!(diff.files.len(), 1, "{diff:?}");
    assert_eq!(diff.files[0].path, "logic.js");
    assert_eq!(diff.files[0].status, FileStatus::Modified);
    assert!(
        diff.files[0].patch.contains("+  return ctx.db.query"),
        "{}",
        diff.files[0].patch
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_diff_test`
Expected: FAIL to compile: no `diff_files` in `apps::screen`.

- [ ] **Step 3: Add the diff**

The first build writes `Cargo.lock` (`similar` 2.7). Do not run `cargo add` as well.

**In `crates/dots-core/Cargo.toml`, replace:**

```toml
sha2 = "0.10"
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "migrate", "macros"] }
```

**with:**

```toml
sha2 = "0.10"
similar = "2"
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "migrate", "macros"] }
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
use crate::library::LibraryItem;
use crate::{Error, Result};
```

**with:**

```rust
use crate::library::LibraryItem;
use crate::model::{FileDiff, FileStatus, WorktreeDiff};
use crate::{Error, Result};
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
        .collect()
}
```

**with:**

```rust
        .collect()
}

/// How a version's files became another's (spec section 6.2, the Code tab): each file that is
/// new, gone or changed, by path, as a unified diff with three lines of context. A file that is
/// not UTF-8 on either side is listed as binary, without a patch.
pub fn diff_files(
    old: &BTreeMap<String, Vec<u8>>,
    new: &BTreeMap<String, Vec<u8>>,
) -> WorktreeDiff {
    let paths: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    let mut files = Vec::new();
    for path in paths {
        let (before, after) = (old.get(path), new.get(path));
        let status = match (before, after) {
            (None, Some(_)) => FileStatus::Added,
            (Some(_), None) => FileStatus::Deleted,
            (Some(a), Some(b)) if a != b => FileStatus::Modified,
            _ => continue,
        };
        let empty = Vec::new();
        let texts = (
            std::str::from_utf8(before.unwrap_or(&empty)),
            std::str::from_utf8(after.unwrap_or(&empty)),
        );
        let (patch, binary) = match texts {
            (Ok(a), Ok(b)) => {
                let patch = similar::TextDiff::from_lines(a, b)
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{path}"), &format!("b/{path}"))
                    .to_string();
                (patch, false)
            }
            _ => (String::new(), true),
        };
        files.push(FileDiff {
            path: path.clone(),
            status,
            patch,
            binary,
        });
    }
    WorktreeDiff {
        files,
        truncated: false,
        exists: true,
    }
}
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust

    /// The newest `limit` logged calls of the app `name` (at most 500), newest first.
```

**with:**

```rust

    /// The Code tab's diff of the waiting proposal `id` (spec section 6.2): against the app's
    /// active version for a change, every file added for a new app. A decided proposal has no
    /// files left: `NotFound`.
    pub async fn proposal_diff(&self, id: &str) -> Result<WorktreeDiff> {
        let p = self.store.get_app_proposal(id).await?;
        let dir = self.proposed_dir(&p.id);
        if !dir.is_dir() {
            return Err(Error::NotFound(format!("the files of app proposal {id}")));
        }
        let next = blocking(move || source::read_folder(&dir)).await??.files();
        let current = match (&p.app_id, self.get(&p.app_name).await) {
            (Some(app_id), Ok((item, config))) if item.id == *app_id => {
                self.load(&item, &config).await?.files()
            }
            _ => BTreeMap::new(),
        };
        Ok(diff_files(&current, &next))
    }

    /// The newest `limit` logged calls of the app `name` (at most 500), newest first.
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_diff_test`
Expected: PASS: 2 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1627 passed; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/dots-core/Cargo.toml crates/dots-core/src/apps/screen.rs crates/dots-core/tests/apps_diff_test.rs
git commit -m "feat(core): the review card's Code tab diff"
```

### Task 3: An app's users, and "Always allow" naming the app

Plan A's deferred findings: a dot whose policy denies every tool of an app is not among its users (the Apps screen's row, `ctx.dots.list()`, and so the dots it may start); and the "Always allow" rules an app call's approval offers name the app, never a bare `App`.

**Files:**

- Modify: `crates/dots-core/src/apps/{run,registry,preview,screen}.rs`, `crates/dots-core/src/policy.rs`
- Test: `crates/dots-core/tests/apps_policy_test.rs`, `crates/dots-core/tests/apps_screen_test.rs`

**Interfaces:**

- Consumes: Task 1's `listed_dots`.
- Produces: `apps::run::offered_tools(dot, manifest) -> Vec<ToolSpec>` (what `RunApps::open` offers); `Apps::users(&item, manifest) -> Result<Vec<Dot>>`; `listed_dots(&item, manifest)` takes the manifest; `policy::allow_candidates("App", input, _)` gives `App(<app>.<tool>)` ("<tool> of the app <app>") then `App(<app>)` ("every tool of the app <app>"), and nothing for a call without its app.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_policy_test.rs`, replace:**

```rust
use dots_core::apps::policy_input;
use dots_core::policy::{Action, Policy, Preset, Rule, APP_TOOLS, PROPOSE_RULE};
use serde_json::json;
```

**with:**

```rust
use dots_core::apps::policy_input;
use dots_core::policy::{allow_candidates, Action, Policy, Preset, Rule, APP_TOOLS, PROPOSE_RULE};
use serde_json::json;
```

**In `crates/dots-core/tests/apps_policy_test.rs`, replace:**

```rust
    p
}
```

**with:**

```rust
    p
}

#[test]
fn an_app_approval_offers_rules_that_name_its_tool_and_its_app() {
    let input = policy_input("expenses", "add_expense", true, &json!({ "item": "Ink" }));
    let offered: Vec<(String, String)> = allow_candidates("App", &input, None)
        .into_iter()
        .map(|c| (c.pattern, c.label))
        .collect();
    assert_eq!(
        offered,
        [
            (
                "App(expenses.add_expense)".to_string(),
                "add_expense of the app expenses".to_string()
            ),
            (
                "App(expenses)".to_string(),
                "every tool of the app expenses".to_string()
            ),
        ]
    );
    // A call without its app offers nothing: never the bare `App`.
    assert!(allow_candidates("App", &json!({ "input": {} }), None).is_empty());
}
```

**In `crates/dots-core/tests/apps_screen_test.rs`, replace:**

```rust
    );
}
```

**with:**

```rust
    );
}

#[tokio::test]
async fn a_dot_whose_rules_deny_the_app_is_not_among_its_users() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let manifest = common::apps::MANIFEST.replacen(
        "\"tools\": [",
        "\"tools\": [\n    { \"name\": \"team\", \"description\": \"The dots\", \
         \"input\": { \"type\": \"object\" } },",
        1,
    );
    let logic = format!(
        "{}\nexport function team(input, ctx) {{ return ctx.dots.list().map(d => d.name); }}\n",
        common::apps::LOGIC
    );
    let mut files = common::apps::expenses_with("app.json", Some(&manifest));
    files.retain(|(path, _)| *path != "logic.js");
    files.push(("logic.js", logic));
    let src = tempfile::tempdir().unwrap();
    write_app(src.path(), &files);
    apps.install(read_folder(src.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    for (name, rule) in [
        ("reader", None),
        ("no_writes", Some("App(expenses.write)")),
        ("denied", Some("App(expenses)")),
    ] {
        let mut s = spec(name);
        s.uses.apps = vec!["expenses".into()];
        if let Some(rule) = rule {
            s.policy.rules.push(dots_core::policy::Rule::new(
                rule,
                dots_core::policy::Action::Deny,
            ));
        }
        store.create_dot(&s, None).await.unwrap();
    }
    let rows = apps.rows().await.unwrap();
    let mut dots = rows[0].dots.clone();
    dots.sort();
    assert_eq!(dots, ["no_writes", "reader"]);
    let mut listed: Vec<String> = serde_json::from_value(
        apps.call("expenses", "team", json!({}), Actor::Person)
            .await
            .unwrap(),
    )
    .unwrap();
    listed.sort();
    assert_eq!(listed, ["no_writes", "reader"]);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_policy_test --test apps_screen_test`
Expected: FAIL: `an_app_approval_offers_rules_that_name_its_tool_and_its_app` (the bare `App` is offered) and `a_dot_whose_rules_deny_the_app_is_not_among_its_users` (`denied` is listed).

- [ ] **Step 3: Leave out denied dots and name the app**

**In `crates/dots-core/src/apps/preview.rs`, replace:**

```rust
        }
        let dots = match (&p.app_id, self.get(&p.app_name).await) {
            (Some(app_id), Ok((item, _))) if item.id == *app_id => self.listed_dots(&item).await?,
            _ => Vec::new(),
        };
        let dir = self.proposed_dir(&p.id);
```

**with:**

```rust
        }
        let dir = self.proposed_dir(&p.id);
```

**In `crates/dots-core/src/apps/preview.rs`, replace:**

```rust
        .await??;
        let id = crate::util::new_id();
```

**with:**

```rust
        .await??;
        let dots = match (&p.app_id, self.get(&p.app_name).await) {
            (Some(app_id), Ok((item, _))) if item.id == *app_id => {
                self.listed_dots(&item, &src.manifest).await?
            }
            _ => Vec::new(),
        };
        let id = crate::util::new_id();
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
        }
        let dots = self.listed_dots(&item).await.map_err(|e| e.to_string())?;
        let logged =
```

**with:**

```rust
        }
        let dots = self
            .listed_dots(&item, &src.manifest)
            .await
            .map_err(|e| e.to_string())?;
        let logged =
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust

    /// What `ctx.dots.list()` gives a call of the app `item`: the dots that use it. A dot the
    /// person switched off is neither listed nor woken (spec section 7), as a webhook or a
    /// follow-up does not run it.
    pub(super) async fn listed_dots(&self, item: &LibraryItem) -> Result<Vec<DotRef>> {
        Ok(self
```

**with:**

```rust

    /// The dots that use the app `item` whose code is `manifest`: those that tick it (or all,
    /// for an "Every dot" app) and whose policy leaves them one of its tools.
    pub(super) async fn users(
        &self,
        item: &LibraryItem,
        manifest: &crate::apps::manifest::Manifest,
    ) -> Result<Vec<crate::model::Dot>> {
        Ok(self
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            .dots_with_app(item)
            .await?
```

**with:**

```rust
            .dots_with_app(item)
            .await?
            .into_iter()
            .filter(|d| !crate::apps::run::offered_tools(d, manifest).is_empty())
            .collect())
    }

    /// What `ctx.dots.list()` gives a call of the app `item`: its users. A dot the person
    /// switched off is neither listed nor woken (spec section 7), as a webhook or a follow-up
    /// does not run it.
    pub(super) async fn listed_dots(
        &self,
        item: &LibraryItem,
        manifest: &crate::apps::manifest::Manifest,
    ) -> Result<Vec<DotRef>> {
        Ok(self
            .users(item, manifest)
            .await?
```

**In `crates/dots-core/src/apps/run.rs`, replace:**

```rust
use super::call::Actor;
use super::manifest::{split_tool_key, tool_key, ToolSpec};
use super::{policy_input, Apps, APP_POLICY};
```

**with:**

```rust
use super::call::Actor;
use super::manifest::{split_tool_key, tool_key, Manifest, ToolSpec};
use super::{policy_input, Apps, APP_POLICY};
```

**In `crates/dots-core/src/apps/run.rs`, replace:**

```rust
    actor: Actor,
}
```

**with:**

```rust
    actor: Actor,
}

/// The tools of the app `manifest` that `dot`'s policy does not deny (spec section 6, and
/// section 15: a denied tool is not offered). A dot left with none does not use the app.
pub fn offered_tools(dot: &Dot, manifest: &Manifest) -> Vec<ToolSpec> {
    manifest
        .tools
        .iter()
        .filter(|t| {
            let input = policy_input(&manifest.name, &t.name, t.writes, &Value::Null);
            dot.spec.policy.resolve(APP_POLICY, &input) != Action::Deny
        })
        .cloned()
        .collect()
}
```

**In `crates/dots-core/src/apps/run.rs`, replace:**

```rust
            };
            let tools: Vec<ToolSpec> = src
                .manifest
                .tools
                .iter()
                .filter(|t| {
                    let input = policy_input(&item.name, &t.name, t.writes, &Value::Null);
                    dot.spec.policy.resolve(APP_POLICY, &input) != Action::Deny
                })
                .cloned()
                .collect();
            if tools.is_empty() {
```

**with:**

```rust
            };
            let tools = offered_tools(dot, &src.manifest);
            if tools.is_empty() {
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
    /// Every installed app with the dots that use it and the error that keeps it out of runs.
    pub async fn rows(&self) -> Result<Vec<AppRow>> {
```

**with:**

```rust
    /// Every installed app with the dots that use it and the error that keeps it out of runs.
    /// A dot whose policy denies every tool of the app does not use it; while the code cannot
    /// be read, every dot that ticks it is named.
    pub async fn rows(&self) -> Result<Vec<AppRow>> {
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
        for (item, config) in self.list().await? {
            let dots = self
                .store
                .dots_with_app(&item)
                .await?
                .into_iter()
                .map(|d| d.spec.name)
                .collect();
            let error = self.load(&item, &config).await.err().map(|e| e.to_string());
            rows.push(AppRow { item, dots, error });
```

**with:**

```rust
        for (item, config) in self.list().await? {
            let (users, error) = match self.load(&item, &config).await {
                Ok(src) => (self.users(&item, &src.manifest).await?, None),
                Err(e) => (self.store.dots_with_app(&item).await?, Some(e.to_string())),
            };
            let dots = users.into_iter().map(|d| d.spec.name).collect();
            rows.push(AppRow { item, dots, error });
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        }
    } else if !tool.contains('*') {
```

**with:**

```rust
        }
    } else if tool == crate::apps::APP_POLICY {
        // An app call offers its own tool and its app (app library spec section 6), never every
        // app: a bare `App` would allow the writes of apps the person has not seen yet.
        let named = |key: &str| input.get(key).and_then(Value::as_str);
        if let (Some(app), Some(name)) = (named("app"), named("tool")) {
            found.push((
                format!("{tool}({app}.{name})"),
                format!("{name} of the app {app}"),
            ));
            found.push((
                format!("{tool}({app})"),
                format!("every tool of the app {app}"),
            ));
        }
    } else if !tool.contains('*') {
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_policy_test --test apps_screen_test --test apps_run_test --test apps_preview_test`
Expected: PASS: 10, 5, 7 and 4 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1629 passed; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/apps/preview.rs crates/dots-core/src/apps/registry.rs crates/dots-core/src/apps/run.rs crates/dots-core/src/apps/screen.rs crates/dots-core/src/policy.rs crates/dots-core/tests/apps_policy_test.rs crates/dots-core/tests/apps_screen_test.rs
git commit -m "fix(core): an app's users leave out dots it denies, and Always allow names the app"
```

### Task 4: The data a removed app kept

Spec §10 (the data goes only after a second confirmation) and the person's choice: kept data is listed, can be deleted, and a new app of the same name whose migrations begin with the ones that ran on it may start on it. Removing an app also forgets its topics (the parked `app_topics` pruning).

**Files:**

- Create: `crates/dots-core/src/apps/kept.rs`, `crates/dots-core/tests/apps_kept_test.rs`
- Modify: `crates/dots-core/src/apps/{mod,registry,propose,wake}.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/store/apps.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts`, the contract fixture

**Interfaces:**

- Consumes: plan A's `Apps::{install, remove, accept, list, lock, app_dir}`, `registry::DATA_FILE`.
- Produces:
  - `apps::kept::{REMOVED_FILE = "removed.json", KeptData { id, name, title, removed_at, bytes }, KeptOffer { id, title, removed_at, bytes, migrations }}`; `Apps::{kept() -> Result<Vec<KeptData>>, delete_kept(id), kept_fit(&src) -> Result<Option<KeptOffer>>, install_kept(src, kept_id) -> Result<LibraryItem>}`; `Apps::install_on(src, origin, kept: Option<&str>)` inside the crate.
  - `ProposalSummary::kept_data: Option<KeptOffer>` (`#[serde(default)]`, so stored summaries still read); `Decision::AddWithKeptData` (`{ "kind": "add_with_kept_data" }`); `Apps::accept(&p, on_kept: bool)`.
  - `Store::{delete_app_topics(app_id), prune_app_topics() -> Result<u64>}`; `Apps::catch_up` prunes first.
  - UI: `T.KeptOffer`, `ProposalSummary.kept_data`, the decision `add_with_kept_data`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/apps_kept_test.rs`:**

```rust
//! The data an app's removal kept (app library spec section 10): listed until the person deletes
//! it, and offered to a new app of the same name whose migrations keep the ones that ran on it.

mod common;

use common::apps::{expenses, expenses_with, proposing_run, write_app, MIGRATION};
use dots_core::apps::call::Actor;
use dots_core::apps::propose::{AppProposal, SENT};
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::model::{NewRun, TriggerKind};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Store, Apps) {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let src = dir.path().join("first");
    write_app(&src, &expenses());
    apps.install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let ink = json!({ "item": "Ink", "amount": 3, "on": "2026-10-20" });
    apps.call("expenses", "add_expense", ink, Actor::Person)
        .await
        .unwrap();
    (dir, store, apps)
}

async fn proposed(
    dir: &tempfile::TempDir,
    store: &Store,
    apps: &Apps,
    files: &[(&str, String)],
) -> AppProposal {
    let n = store.list_dots().await.unwrap().len();
    let run = proposing_run(store, &format!("builder{n}")).await;
    let folder = dir.path().join(format!("work-{n}"));
    write_app(&folder, files);
    assert_eq!(
        apps.propose(&run.id, folder, "Again.").await,
        Ok(SENT.to_string())
    );
    store
        .list_app_proposals(Some(&run.dot_id))
        .await
        .unwrap()
        .remove(0)
}

async fn items(apps: &Apps) -> Vec<String> {
    let month = json!({ "month": "2026-10" });
    let rows = apps
        .call("expenses", "list_expenses", month, Actor::Person)
        .await
        .unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["item"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn removing_an_app_that_keeps_its_data_lists_it_until_it_is_deleted() {
    let (_dir, _store, apps) = fixture().await;
    let (item, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let kept = apps.kept().await.unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].id, item.id);
    assert_eq!(
        (kept[0].name.as_str(), kept[0].title.as_str()),
        ("expenses", "Expenses")
    );
    assert!(kept[0].bytes > 0);
    assert!(apps.data_path(&item.id).exists());
    assert!(!apps.app_dir(&item.id).join("code").exists());

    apps.delete_kept(&item.id).await.unwrap();
    assert!(apps.kept().await.unwrap().is_empty());
    assert!(!apps.app_dir(&item.id).exists());
    let again = apps.delete_kept(&item.id).await;
    assert!(matches!(again, Err(Error::NotFound(_))), "{again:?}");
}

#[tokio::test]
async fn removing_an_app_with_its_data_keeps_nothing() {
    let (_dir, _store, apps) = fixture().await;
    let (item, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, true).await.unwrap();
    assert!(apps.kept().await.unwrap().is_empty());
    assert!(!apps.app_dir(&item.id).exists());
}

#[tokio::test]
async fn a_new_app_that_keeps_the_migrations_is_offered_the_kept_data_and_starts_on_it() {
    let (dir, store, apps) = fixture().await;
    let (old, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let mut files = expenses();
    files.push((
        "migrations/0002_categories.sql",
        "CREATE TABLE categories (name TEXT PRIMARY KEY);\n".to_string(),
    ));
    let p = proposed(&dir, &store, &apps, &files).await;
    let offer = p
        .summary
        .kept_data
        .clone()
        .expect("the kept data is offered");
    assert_eq!(offer.id, old.id);
    assert_eq!(offer.migrations, ["0002_categories.sql"]);

    let item = apps.accept(&p, true).await.unwrap();
    assert_ne!(item.id, old.id);
    assert_eq!(items(&apps).await, ["Ink"], "the kept row is there");
    assert!(apps.kept().await.unwrap().is_empty());
    assert!(!apps.app_dir(&old.id).exists());
}

#[tokio::test]
async fn kept_data_that_does_not_fit_is_not_offered_and_the_app_starts_empty() {
    let (dir, store, apps) = fixture().await;
    apps.remove("expenses", true, false).await.unwrap();
    let changed = MIGRATION.replace("added_by TEXT NOT NULL", "added_by TEXT");
    let files = expenses_with("migrations/0001_expenses.sql", Some(&changed));
    let p = proposed(&dir, &store, &apps, &files).await;
    assert_eq!(p.summary.kept_data, None);
    let refused = apps.accept(&p, true).await;
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    apps.accept(&p, false).await.unwrap();
    assert!(items(&apps).await.is_empty());
    assert_eq!(
        apps.kept().await.unwrap().len(),
        1,
        "the old data is still kept"
    );
}

#[tokio::test]
async fn kept_data_deleted_after_the_card_was_posted_is_not_started_on() {
    let (dir, store, apps) = fixture().await;
    let (old, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let p = proposed(&dir, &store, &apps, &expenses()).await;
    assert!(p.summary.kept_data.is_some());
    apps.delete_kept(&old.id).await.unwrap();
    let refused = apps.accept(&p, true).await;
    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    assert!(apps.list().await.unwrap().is_empty(), "nothing was added");
}

#[tokio::test]
async fn removing_an_app_forgets_its_topics() {
    let (_dir, store, apps) = fixture().await;
    let (item, _) = apps.get("expenses").await.unwrap();
    let dot = store
        .create_dot(&common::spec("helper"), None)
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::App);
    new.payload = Some(json!({
        "message": "Hi", "app": "expenses", "app_title": "Expenses", "tag": "t1"
    }));
    store
        .start_app_run(&item.id, "t1", &new, "2026-10-27T09:00:00Z")
        .await
        .unwrap();
    assert_eq!(store.open_app_topics(&item.id).await.unwrap().len(), 1);
    apps.remove("expenses", true, false).await.unwrap();
    assert!(store.open_app_topics(&item.id).await.unwrap().is_empty());
    assert_eq!(store.prune_app_topics().await.unwrap(), 0);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_kept_test`
Expected: FAIL to compile: no `kept` on `Apps`, `accept` takes one argument, no `prune_app_topics` on `Store`.

- [ ] **Step 3: Keep, list, offer and delete the data**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        update_from: None,
    };
```

**with:**

```rust
        update_from: None,
        kept_data: None,
    };
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    update.summary.migrations = vec!["0002_category.sql".into()];
    samples["AppProposal"] = to([
```

**with:**

```rust
    update.summary.migrations = vec!["0002_category.sql".into()];
    let mut on_kept = proposal("01J0PROP5", ProposalStatus::Pending);
    on_kept.summary.kept_data = Some(dots_core::apps::kept::KeptOffer {
        id: "01J0APP0".into(),
        title: "Expenses".into(),
        removed_at: TS.into(),
        bytes: 24_576,
        migrations: vec!["0002_category.sql".into()],
    });
    samples["AppProposal"] = to([
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        proposal("01J0PROP4", ProposalStatus::Discarded),
    ]);
```

**with:**

```rust
        proposal("01J0PROP4", ProposalStatus::Discarded),
        on_kept,
    ]);
```

**Create `crates/dots-core/src/apps/kept.rs`:**

```rust
//! The data an app's removal kept (app library spec section 10: the data goes only after a
//! second confirmation). Removing an app that keeps its data leaves `<data>/apps/<app id>/` with
//! its `data.db` and a `removed.json` naming the app and the migrations that ran on the data.
//! The Apps screen lists it, and a new app of the same name whose migrations begin with those,
//! byte for byte, may start on it instead of on an empty database.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::registry::{blocking, remove_all, DATA_FILE};
use super::source::{AppSource, Migration};
use super::Apps;
use crate::library::{AppOrigin, LibraryItem};
use crate::{Error, Result};

/// The note a removal that kept the data leaves beside it.
pub const REMOVED_FILE: &str = "removed.json";

/// What `removed.json` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Removed {
    name: String,
    title: String,
    removed_at: String,
    /// The migrations that ran on the data; `None` when the removed code could not be read.
    migrations: Option<Vec<RanMigration>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RanMigration {
    file: String,
    sha256: String,
}

/// Data a removed app kept, as the Apps screen lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptData {
    /// The removed app's id: its folder below `<data>/apps/`.
    pub id: String,
    pub name: String,
    pub title: String,
    pub removed_at: String,
    /// The size of its database files.
    pub bytes: u64,
}

/// Kept data a new app may start on (the review card's offer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptOffer {
    pub id: String,
    pub title: String,
    pub removed_at: String,
    pub bytes: u64,
    /// The new app's migrations that would run on it: those after the ones that ran.
    pub migrations: Vec<String>,
}

fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The files of an app's database, as SQLite leaves them.
fn data_files() -> [String; 3] {
    [
        DATA_FILE.to_string(),
        format!("{DATA_FILE}-wal"),
        format!("{DATA_FILE}-shm"),
    ]
}

/// Writes `removed.json` into `dir`: the app is being removed and its data stays. Blocks.
pub(super) fn note_removed(
    dir: &Path,
    name: &str,
    title: &str,
    src: Option<&AppSource>,
) -> Result<()> {
    let note = Removed {
        name: name.to_string(),
        title: title.to_string(),
        removed_at: crate::util::now(),
        migrations: src.map(|s| {
            s.migrations
                .iter()
                .map(|m| RanMigration {
                    file: m.file.clone(),
                    sha256: sha256(&m.sql),
                })
                .collect()
        }),
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(REMOVED_FILE), serde_json::to_vec_pretty(&note)?)?;
    Ok(())
}

/// Moves the database files from `from` to `to`. Blocks.
pub(super) fn move_data(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for file in data_files() {
        if from.join(&file).exists() {
            std::fs::rename(from.join(&file), to.join(&file))?;
        }
    }
    Ok(())
}

/// Every folder below `root` with a `removed.json`, and the migrations it names. Blocks.
fn read_kept(root: &Path) -> Vec<(KeptData, Option<Vec<RanMigration>>)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(text) = std::fs::read(dir.join(REMOVED_FILE)) else {
            continue;
        };
        let Ok(note) = serde_json::from_slice::<Removed>(&text) else {
            continue;
        };
        let bytes = data_files()
            .iter()
            .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
            .map(|m| m.len())
            .sum();
        let kept = KeptData {
            id: entry.file_name().to_string_lossy().into_owned(),
            name: note.name,
            title: note.title,
            removed_at: note.removed_at,
            bytes,
        };
        found.push((kept, note.migrations));
    }
    found
}

/// Whether the migrations `ran` begin `next`'s, file name and text alike.
fn fits(ran: &[RanMigration], next: &[Migration]) -> bool {
    ran.len() <= next.len()
        && ran
            .iter()
            .zip(next)
            .all(|(r, m)| r.file == m.file && r.sha256 == sha256(&m.sql))
}

impl Apps {
    async fn kept_notes(&self) -> Result<Vec<(KeptData, Option<Vec<RanMigration>>)>> {
        let live: HashSet<String> = self.list().await?.into_iter().map(|(i, _)| i.id).collect();
        let root = self.root().to_path_buf();
        let mut found = blocking(move || read_kept(&root)).await?;
        // A row that is still there is a removal under way, not kept data.
        found.retain(|(k, _)| !live.contains(&k.id));
        found.sort_by(|a, b| b.0.removed_at.cmp(&a.0.removed_at));
        Ok(found)
    }

    /// The data removed apps kept, newest first (spec section 10).
    pub async fn kept(&self) -> Result<Vec<KeptData>> {
        Ok(self
            .kept_notes()
            .await?
            .into_iter()
            .map(|(k, _)| k)
            .collect())
    }

    /// Deletes the kept data `id`; `NotFound` when there is none.
    pub async fn delete_kept(&self, id: &str) -> Result<()> {
        if !self.kept().await?.iter().any(|k| k.id == id) {
            return Err(Error::NotFound(format!("kept data {id}")));
        }
        let lock = self.lock(id);
        let _held = lock.lock().await;
        let dir = self.app_dir(id);
        blocking(move || remove_all(&dir)).await?
    }

    /// The newest kept data of an app named as `src` is whose migrations `src` keeps: what the
    /// review card offers to start the new app on.
    pub async fn kept_fit(&self, src: &AppSource) -> Result<Option<KeptOffer>> {
        for (kept, ran) in self.kept_notes().await? {
            let Some(ran) = ran else {
                continue;
            };
            if kept.name == src.manifest.name && fits(&ran, &src.migrations) {
                return Ok(Some(KeptOffer {
                    migrations: src.migrations[ran.len()..]
                        .iter()
                        .map(|m| m.file.clone())
                        .collect(),
                    id: kept.id,
                    title: kept.title,
                    removed_at: kept.removed_at,
                    bytes: kept.bytes,
                }));
            }
        }
        Ok(None)
    }

    /// Adds `src` as a new app on the kept data `kept_id`, which must still fit it: the data
    /// moves to the new app, and the migrations after those that ran on it run. On a failure the
    /// data goes back where it was.
    pub async fn install_kept(&self, src: AppSource, kept_id: &str) -> Result<LibraryItem> {
        let lock = self.lock(kept_id);
        let _held = lock.lock().await;
        match self.kept_fit(&src).await? {
            Some(offer) if offer.id == kept_id => {}
            _ => {
                return Err(Error::Conflict(format!(
                    "the data kept from the removed {} is gone or no longer fits; add the app \
                     without it",
                    src.manifest.title
                )))
            }
        }
        self.install_on(src, AppOrigin::Proposed, Some(kept_id))
            .await
    }
}
```

**In `crates/dots-core/src/apps/mod.rs`, replace:**

```rust
pub mod engine;
pub mod manifest;
```

**with:**

```rust
pub mod engine;
pub mod kept;
pub mod manifest;
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
    pub update_from: Option<u32>,
}
```

**with:**

```rust
    pub update_from: Option<u32>,
    /// Data a removed app of this name kept that fits this new app's migrations: the card
    /// offers to start on it (spec section 10).
    #[serde(default)]
    pub kept_data: Option<super::kept::KeptOffer>,
}
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
    Add,
    /// "Ask for changes": the text continues the dot's thread.
```

**with:**

```rust
    Add,
    /// "Add to library" on the kept data the card offered.
    AddWithKeptData,
    /// "Ask for changes": the text continues the dot's thread.
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
        let name = src.manifest.name.clone();
        let (base, migrations) = match self.get(&name).await {
```

**with:**

```rust
        let name = src.manifest.name.clone();
        let mut kept_data = None;
        let (base, migrations) = match self.get(&name).await {
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
            }
            Err(Error::NotFound(_)) => (
                None,
                src.migrations.iter().map(|m| m.file.clone()).collect(),
            ),
            Err(e) => return Err(said(e)),
```

**with:**

```rust
            }
            Err(Error::NotFound(_)) => {
                kept_data = self.kept_fit(&src).await.map_err(said)?;
                (
                    None,
                    src.migrations.iter().map(|m| m.file.clone()).collect(),
                )
            }
            Err(e) => return Err(said(e)),
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
            update_from: base.as_ref().map(|(_, v)| *v),
        };
```

**with:**

```rust
            update_from: base.as_ref().map(|(_, v)| *v),
            kept_data,
        };
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
    /// "Add to library" or "Update" for the pending proposal `p`: a new app at version 1, its
    /// switches off, or the next version of the app it changes. A change whose app has moved
    /// on since it was checked is a `Conflict`.
    pub async fn accept(&self, p: &AppProposal) -> Result<LibraryItem> {
        let dir = self.proposed_dir(&p.id);
```

**with:**

```rust
    /// "Add to library" or "Update" for the pending proposal `p`: a new app at version 1, its
    /// switches off, on the kept data the card offered when `on_kept`, or the next version of
    /// the app it changes. A change whose app has moved on since it was checked is a
    /// `Conflict`.
    pub async fn accept(&self, p: &AppProposal, on_kept: bool) -> Result<LibraryItem> {
        let dir = self.proposed_dir(&p.id);
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
        let Some(base) = p.base_version else {
            return self.install(src, AppOrigin::Proposed).await;
        };
```

**with:**

```rust
        let Some(base) = p.base_version else {
            return match (on_kept, &p.summary.kept_data) {
                (false, _) => self.install(src, AppOrigin::Proposed).await,
                (true, Some(kept)) => self.install_kept(src, &kept.id).await,
                (true, None) => Err(Error::Invalid(format!(
                    "no kept data was offered for the app {}",
                    p.app_name
                ))),
            };
        };
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
    pub async fn install(&self, src: AppSource, origin: AppOrigin) -> Result<LibraryItem> {
        let name = src.manifest.name.clone();
```

**with:**

```rust
    pub async fn install(&self, src: AppSource, origin: AppOrigin) -> Result<LibraryItem> {
        self.install_on(src, origin, None).await
    }

    /// `install`, on the data a removal kept in `<data>/apps/<kept>/` when `kept` names it: the
    /// data moves to the new app before the migrations run, and back on a failure; the kept
    /// folder goes once the app is in.
    pub(super) async fn install_on(
        &self,
        src: AppSource,
        origin: AppOrigin,
        kept: Option<&str>,
    ) -> Result<LibraryItem> {
        let name = src.manifest.name.clone();
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
        let app_dir = self.app_dir(&item.id);
        let written = blocking(move || -> Result<()> {
```

**with:**

```rust
        let app_dir = self.app_dir(&item.id);
        let kept_dir = kept.map(|id| self.app_dir(id));
        let written = blocking(move || -> Result<()> {
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            write_version(&code, &src)?;
            let mut conn = data::open(&data_path)?;
            data::migrate(&mut conn, &src.migrations)?;
            Ok(())
        })
```

**with:**

```rust
            write_version(&code, &src)?;
            if let Some(from) = &kept_dir {
                crate::apps::kept::move_data(from, &app_dir)?;
            }
            let migrated = data::open(&data_path)
                .and_then(|mut conn| data::migrate(&mut conn, &src.migrations));
            match (migrated, &kept_dir) {
                (Ok(_), Some(from)) => {
                    let _ = remove_all(from);
                    Ok(())
                }
                (Ok(_), None) => Ok(()),
                (Err(e), Some(from)) => {
                    let _ = crate::apps::kept::move_data(&app_dir, from);
                    let _ = std::fs::remove_dir_all(&app_dir);
                    Err(e)
                }
                (Err(e), None) => {
                    let _ = std::fs::remove_dir_all(&app_dir);
                    Err(e)
                }
            }
        })
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            let _ = self.store.remove_library_item(&item.id).await;
            let _ = blocking(move || std::fs::remove_dir_all(app_dir)).await;
            return Err(e);
```

**with:**

```rust
            let _ = self.store.remove_library_item(&item.id).await;
            return Err(e);
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
    /// tick it, unless `confirm`; its code goes, and its data too when `delete_data` (the second
    /// confirmation). Kept data is found again by nothing: a new app gets a new id.
    pub async fn remove(&self, name: &str, confirm: bool, delete_data: bool) -> Result<()> {
        let (_held, item, _) = self.get_locked(name).await?;
        // "Every dot" gives the app to dots that do not name it: they use it too.
```

**with:**

```rust
    /// tick it, unless `confirm`; its code goes, and its data too when `delete_data` (the second
    /// confirmation). Data that stays gets a `removed.json`, so the Apps screen lists it and a
    /// new app of the name may start on it (`kept`). Its topics go: no run of them can reach
    /// it again.
    pub async fn remove(&self, name: &str, confirm: bool, delete_data: bool) -> Result<()> {
        let (_held, item, config) = self.get_locked(name).await?;
        // "Every dot" gives the app to dots that do not name it: they use it too.
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
        }
        // The files first: when one cannot be deleted the row stays, and removing again retries.
```

**with:**

```rust
        }
        let src = self.load(&item, &config).await.ok();
        // The files first: when one cannot be deleted the row stays, and removing again retries.
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
        let dir = self.app_dir(&item.id);
        blocking(move || -> Result<()> {
            remove_all(&dir.join("code"))?;
```

**with:**

```rust
        let dir = self.app_dir(&item.id);
        let (app, title) = (item.name.clone(), config.title.clone());
        blocking(move || -> Result<()> {
            if !delete_data {
                crate::apps::kept::note_removed(&dir, &app, &title, src.as_deref())?;
            }
            remove_all(&dir.join("code"))?;
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
        .await??;
        self.store.remove_library_item(&item.id).await?;
```

**with:**

```rust
        .await??;
        self.store.delete_app_topics(&item.id).await?;
        self.store.remove_library_item(&item.id).await?;
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
/// Deletes `path` and all under it; one already gone is not an error.
fn remove_all(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
```

**with:**

```rust
/// Deletes `path` and all under it; one already gone is not an error.
pub(super) fn remove_all(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
```

**In `crates/dots-core/src/apps/wake.rs`, replace:**

```rust
    pub async fn catch_up(&self) -> Result<()> {
        for topic in self.store.idle_app_topics().await? {
```

**with:**

```rust
    pub async fn catch_up(&self) -> Result<()> {
        self.store.prune_app_topics().await?;
        for topic in self.store.idle_app_topics().await? {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            Decision::Discard => (ProposalStatus::Discarded, None, DISCARDED.to_string()),
            Decision::Add => {
                let item = self.apps.accept(&p).await?;
                let version = match &item.config {
```

**with:**

```rust
            Decision::Discard => (ProposalStatus::Discarded, None, DISCARDED.to_string()),
            Decision::Add | Decision::AddWithKeptData => {
                let on_kept = decision == Decision::AddWithKeptData;
                let item = self.apps.accept(&p, on_kept).await?;
                let version = match &item.config {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
                        "The person accepted your change to the app {}; it is now version {version}.",
                        p.app_name
```

**with:**

```rust
                        "The person accepted your change to the app {}; it is now version {version}.",
                        p.app_name
                    ),
                    None if on_kept => format!(
                        "The person added the app {} to the Library, on the data kept from the \
                         removed one.",
                        p.app_name
```

**In `crates/dots-core/src/store/apps.rs`, replace:**

```rust

    /// Stores the run an app's start asked for and its topic, in one transaction (app library
```

**with:**

```rust

    /// Forgets the topics of the app `app_id`: it was removed.
    pub async fn delete_app_topics(&self, app_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM app_topics WHERE app_id = ?")
            .bind(app_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Forgets the topics of apps that are no longer installed (removed before their topics
    /// went with them).
    pub async fn prune_app_topics(&self) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM app_topics WHERE app_id NOT IN \
             (SELECT id FROM library_items WHERE kind = 'app')",
        )
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    /// Stores the run an app's start asked for and its topic, in one transaction (app library
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const ProposalStatus = z.enum(["pending", "added", "changes_asked", "discarded"]);
// What a review card's summary shows.
```

**with:**

```ts
export const ProposalStatus = z.enum(["pending", "added", "changes_asked", "discarded"]);
// Data a removed app of the same name kept that fits a new app's migrations: the review card
// offers to start on it.
export const KeptOffer = z.strictObject({
  id: z.string(),
  title: z.string(),
  removed_at: z.string(),
  bytes: z.number(),
  // The new app's migrations that would run on it.
  migrations: z.array(z.string()),
});
// What a review card's summary shows.
```

**In `ui/src/api/types.ts`, replace:**

```ts
  update_from: nullable(z.number()),
});
```

**with:**

```ts
  update_from: nullable(z.number()),
  kept_data: nullable(KeptOffer),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
  z.strictObject({ kind: z.literal("add") }),
  z.strictObject({ kind: z.literal("ask_for_changes"), text: z.string() }),
```

**with:**

```ts
  z.strictObject({ kind: z.literal("add") }),
  // "Add to library" on the kept data the card offered.
  z.strictObject({ kind: z.literal("add_with_kept_data") }),
  z.strictObject({ kind: z.literal("ask_for_changes"), text: z.string() }),
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_kept_test --test apps_propose_test --test apps_wake_test --test apps_registry_test`
Expected: PASS: 6, 8, 7 and 21 tests.

- [ ] **Step 5: Write the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -j 2 -p dots-app contract`, then `bun x vitest run src/api` and `bun run typecheck` in `ui`.
Expected: the fixture gains the proposal `01J0PROP5` with its `kept_data`, and every proposal a `kept_data`; the UI's API tests pass and `tsc` is clean.

- [ ] **Step 6: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo test -j 2 -p dots-app`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: `dots-core` 1635 passed, `dots-app` 124, the UI 748 in 71 files; no new warnings.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/src/apps/kept.rs crates/dots-core/src/apps/mod.rs crates/dots-core/src/apps/propose.rs crates/dots-core/src/apps/registry.rs crates/dots-core/src/apps/wake.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/apps.rs crates/dots-core/tests/apps_kept_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts
git commit -m "feat(core): the data a removed app kept, listed and offered to a new app of its name"
```

### Task 5: `copy_app`

Spec §15, "Left to plan B": a dot-side tool under `ProposeApp` that copies an installed app's active version, as written, into the run's workspace, on both engines, so a dot can change Projects or another dot's app. The approval of a `copy_app` call is carried out as `copy_app`, not `propose_app` (both share the policy name; `local_name_for` and the runner tell them apart by the input's `app`).

**Files:**

- Modify: `crates/dots-core/src/apps/propose.rs`, `crates/dots-core/src/engine/local/{mod,mcp_tools,app_tools}.rs`, `crates/dots-core/src/engine/local/tools/mod.rs`, `crates/dots-core/src/engine/claude/args.rs`, `crates/dots-core/src/runner.rs`, `crates/dots-core/src/server/mcp.rs`
- Create: `crates/dots-core/tests/apps_copy_test.rs`
- Test: `crates/dots-core/tests/{apps_propose_test,claude_cli_test,local_tools_test}.rs`

**Interfaces:**

- Consumes: plan A's `propose_app` wiring on both engines (`tools::confine`, `call_propose`, `PROPOSE_POLICY`).
- Produces: `propose::{COPY_TOOL = "copy_app", COPY_DESCRIPTION, copy_properties(), COPY_REQUIRED = ["app", "path"], copy_mcp_schema()}`; `Apps::copy_to(name, folder, shown) -> Result<String, String>` ("Copied version <v> of the app <name> into <shown>. …"; "there is no app <name>"; "the folder <shown> is not empty: copy the app into a new folder"); the local tool `copy_app` (alias `ProposeApp`, `path` a `PathStr`); `mcp__dots__copy_app` in the Claude run's `--allowedTools`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/apps_copy_test.rs`:**

```rust
//! `copy_app` (app library spec section 15): a dot copies an installed app's active version, as
//! Dots keeps it, into its workspace, so a change of it keeps the migrations that ran.

mod common;

use common::apps::{expenses, proposing_run, write_app};
use dots_core::apps::propose::SENT;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;

#[tokio::test]
async fn a_copy_is_the_active_version_as_kept_and_a_change_made_from_it_passes() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    let item = apps
        .install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let copy = dir.path().join("work").join("expenses");
    let said = apps
        .copy_to("expenses", copy.clone(), "expenses")
        .await
        .unwrap();
    assert!(
        said.starts_with("Copied version 1 of the app expenses into expenses."),
        "{said}"
    );
    let kept = apps.code_dir(&item.id, 1);
    for entry in [
        "app.json",
        "logic.js",
        "ui/index.html",
        "migrations/0001_expenses.sql",
    ] {
        assert_eq!(
            std::fs::read(copy.join(entry)).unwrap(),
            std::fs::read(kept.join(entry)).unwrap(),
            "{entry}"
        );
    }
    std::fs::write(
        copy.join("migrations/0002_categories.sql"),
        "CREATE TABLE categories (name TEXT PRIMARY KEY);\n",
    )
    .unwrap();
    let run = proposing_run(&store, "fixer").await;
    assert_eq!(
        apps.propose(&run.id, copy, "Categories.").await,
        Ok(SENT.to_string())
    );
}

#[tokio::test]
async fn copying_refuses_an_unknown_app_and_a_folder_in_use() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    assert_eq!(
        apps.copy_to("nothing", dir.path().join("a"), "a").await,
        Err("there is no app nothing".to_string())
    );
    let used = dir.path().join("used");
    std::fs::create_dir_all(&used).unwrap();
    std::fs::write(used.join("notes.md"), "mine").unwrap();
    assert_eq!(
        apps.copy_to("expenses", used.clone(), "used").await,
        Err("the folder used is not empty: copy the app into a new folder".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(used.join("notes.md")).unwrap(),
        "mine"
    );
}
```

**In `crates/dots-core/tests/apps_propose_test.rs`, replace:**

```rust
#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_under_its_policy() {
```

**with:**

```rust
#[tokio::test]
async fn a_local_dot_copies_an_installed_app_into_its_workspace() {
    let t = start().await;
    let src = in_workspace(&t, "first", &expenses());
    t.rt.apps()
        .install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let llm = Llm::start(vec![
        llm_call("c1", "copy_app", json!({"app": "expenses", "path": "copy"})),
        llm_text("Copied."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "fixer"), None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    let names: Vec<&str> = reqs[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"copy_app"), "{names:?}");
    assert!(
        tool_result(&reqs[1], "c1").starts_with("Copied version 1 of the app expenses into copy."),
        "{}",
        tool_result(&reqs[1], "c1")
    );
    assert!(t.dir.path().join("copy").join("app.json").is_file());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_copies_an_installed_app_through_dots_own_server() {
    let script = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_call": {"server": "dots", "tool": "copy_app",
                      "arguments": {"app": "expenses", "path": "copy"}}},
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
    let src = in_workspace(&t, "first", &expenses());
    t.rt.apps()
        .install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let mut s = spec("claude");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    let dot = t.rt.create_dot(s, None).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(t.dir.path().join("args.json")).unwrap())
            .unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        args.contains(&"mcp__dots__copy_app".to_string()),
        "{args:?}"
    );
    assert!(t.dir.path().join("copy").join("logic.js").is_file());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_under_its_policy() {
```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
         mcp__dots__notify mcp__dots__ask mcp__dots__follow_up mcp__dots__remember \
         mcp__dots__forget mcp__dots__recall mcp__dots__propose_app \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
```

**with:**

```rust
         mcp__dots__notify mcp__dots__ask mcp__dots__follow_up mcp__dots__remember \
         mcp__dots__forget mcp__dots__recall mcp__dots__propose_app mcp__dots__copy_app \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    // tools and the three memory tools, which an engine with check-ins adds
    // (`checkins::schemas`, `memory::schemas`), `propose_app`, which an engine with apps
    // adds, and `start_workflow`, which a chat run that may start a workflow gets
    // (`team::start_schema`).
```

**with:**

```rust
    // tools and the three memory tools, which an engine with check-ins adds
    // (`checkins::schemas`, `memory::schemas`), `propose_app` and `copy_app`, which an engine
    // with apps adds, and `start_workflow`, which a chat run that may start a workflow gets
    // (`team::start_schema`).
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
        ),
        ("forget", json!({"id": "01HX"})),
```

**with:**

```rust
        ),
        ("copy_app", json!({"app": "expenses", "path": "apps//copy"})),
        ("forget", json!({"id": "01HX"})),
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    assert_eq!(tools_seen, all);
    // The memory tools share `Memory`, which names the first of them (`local_name_for` tells
    // them apart by the call's input, checked above).
    for (name, alias) in tools::TOOLS {
```

**with:**

```rust
    assert_eq!(tools_seen, all);
    // The memory tools share `Memory`, which names the first of them, and `copy_app` shares
    // `ProposeApp` with `propose_app` (`local_name_for` tells them apart by the call's input,
    // checked above).
    for (name, alias) in tools::TOOLS {
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        assert_eq!(tools::alias(name), Some(*alias));
        if *alias != "Memory" || *name == "remember" {
            assert_eq!(tools::local_name(alias), Some(*name));
```

**with:**

```rust
        assert_eq!(tools::alias(name), Some(*alias));
        if (*alias != "Memory" || *name == "remember") && *name != "copy_app" {
            assert_eq!(tools::local_name(alias), Some(*name));
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --no-fail-fast --test apps_copy_test --test apps_propose_test --test claude_cli_test --test local_tools_test`
Expected: FAIL to compile: no `copy_to` on `Apps`.

- [ ] **Step 3: Add the tool on both engines**

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
    assert.deepEqual, assert.throws and call(tool, input, { actor, now }). To change an app, \
    send its whole folder, keeping the migrations that ran and adding new ones after them. Dots \
    checks the folder and runs the tests; a problem comes back as an error for you to fix.";

```

**with:**

```rust
    assert.deepEqual, assert.throws and call(tool, input, { actor, now }). To change an app, \
    send its whole folder, keeping the migrations that ran and adding new ones after them; copy \
    an installed app into your workspace with copy_app first. Dots checks the folder and runs \
    the tests; a problem comes back as an error for you to fix.";

/// The tool that copies an installed app into the run's workspace, under `ProposeApp` (spec
/// section 15): a change must keep the active version's migrations byte for byte, and only the
/// dot that wrote an app still has its folder.
pub const COPY_TOOL: &str = "copy_app";

pub const COPY_DESCRIPTION: &str = "Copy an installed app's active version, as Dots keeps it, \
    into a new folder of your workspace, to change it there and send it back with propose_app. \
    Keep its migrations exactly as they are and add new ones after them.";

/// `copy_app`'s arguments.
pub fn copy_properties() -> Value {
    json!({
        "app": { "type": "string", "description": "The app's name" },
        "path": { "type": "string", "description": "A new folder, relative to your workspace" }
    })
}

pub const COPY_REQUIRED: [&str; 2] = ["app", "path"];

/// `copy_app` as Dots' own `dots` MCP server lists it.
pub fn copy_mcp_schema() -> Value {
    json!({
        "name": COPY_TOOL,
        "description": COPY_DESCRIPTION,
        "inputSchema": {
            "type": "object", "properties": copy_properties(), "required": COPY_REQUIRED
        }
    })
}

```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust

    /// Deletes a decided proposal's files, and closes its previews.
```

**with:**

```rust

    /// `copy_app`: writes the files of the app `name`'s active version, as its version folder
    /// holds them, into `folder` (already confined to the run's workspace), which must be new
    /// or empty. `shown` is the folder as the dot named it. `Ok` is what the dot is told; `Err`
    /// the problem, in a sentence for the dot.
    pub async fn copy_to(
        &self,
        name: &str,
        folder: PathBuf,
        shown: &str,
    ) -> std::result::Result<String, String> {
        let (item, config) = match self.get(name).await {
            Ok(found) => found,
            Err(Error::NotFound(_)) => return Err(format!("there is no app {name}")),
            Err(e) => return Err(said(e)),
        };
        let src = self.load(&item, &config).await.map_err(said)?;
        let refused = format!("the folder {shown} is not empty: copy the app into a new folder");
        blocking(move || -> std::result::Result<(), String> {
            if folder.is_file() {
                return Err(refused);
            }
            if let Ok(mut entries) = std::fs::read_dir(&folder) {
                if entries.next().is_some() {
                    return Err(refused);
                }
            }
            for (path, bytes) in src.files() {
                let file = folder.join(path);
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(file, bytes).map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .await
        .map_err(said)??;
        Ok(format!(
            "Copied version {} of the app {name} into {shown}. Change it there, keep its \
             migrations as they are and add new ones after them, then send the whole folder with \
             propose_app.",
            config.version
        ))
    }

    /// Deletes a decided proposal's files, and closes its previews.
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
    allow.extend(crate::memory::Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
    // And of `propose_app` (app library spec section 6.2), which the dot calls by that name.
    allow.push(format!("mcp__dots__{}", crate::apps::propose::TOOL));
    // And of the run's app tools (app library spec section 6).
```

**with:**

```rust
    allow.extend(crate::memory::Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
    // And of `propose_app` and `copy_app` (app library spec section 6.2), which the dot calls
    // by those names.
    allow.push(format!("mcp__dots__{}", crate::apps::propose::TOOL));
    allow.push(format!("mcp__dots__{}", crate::apps::propose::COPY_TOOL));
    // And of the run's app tools (app library spec section 6).
```

**In `crates/dots-core/src/engine/local/app_tools.rs`, replace:**

```rust
        &propose::REQUIRED,
    )
```

**with:**

```rust
        &propose::REQUIRED,
    )
}

/// The function definition of `copy_app`.
pub(super) fn copy_schema() -> Value {
    use crate::apps::propose;
    tools::tool(
        propose::COPY_TOOL,
        propose::COPY_DESCRIPTION,
        propose::copy_properties(),
        &propose::COPY_REQUIRED,
    )
```

**In `crates/dots-core/src/engine/local/app_tools.rs`, replace:**

```rust

    /// Performs an allowed or approved app call; `checked` is its policy input.
```

**with:**

```rust

    /// `copy_app` after the gate allowed it: `path` was confined to the workspace by `prepare`,
    /// and is resolved again here.
    pub(super) async fn copy_app(&self, input: &Value) -> ToolOutput {
        let Some(apps) = &self.apps else {
            return ToolOutput::err("copy_app is not available in this run");
        };
        let shown = tools::str_arg(input, "path");
        let folder = match tools::confine(&self.ctx.workspace, shown) {
            Ok(folder) => folder,
            Err(e) => return ToolOutput::err(e),
        };
        match apps
            .copy_to(tools::str_arg(input, "app"), folder, shown)
            .await
        {
            Ok(text) => ToolOutput::ok(text),
            Err(text) => ToolOutput::err(text),
        }
    }

    /// Performs an allowed or approved app call; `checked` is its policy input.
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust
        }
        // `propose_app` (app library spec section 6.2), when the engine has apps.
        if self.apps.is_some() {
```

**with:**

```rust
        }
        // `propose_app` and `copy_app` (app library spec section 6.2), when the engine has apps.
        if self.apps.is_some() {
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust
            out.push(super::app_tools::propose_schema());
        }
```

**with:**

```rust
            out.push(super::app_tools::propose_schema());
            out.push(super::app_tools::copy_schema());
        }
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
        }
        if CheckInTool::from_name(call.name).is_some() {
```

**with:**

```rust
        }
        if call.name == crate::apps::propose::COPY_TOOL {
            return self.copy_app(&call.input).await;
        }
        if CheckInTool::from_name(call.name).is_some() {
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// the memory tools (`memory::run`), which share one alias. `propose_app` hands an app folder in
/// the workspace to the person (app library spec section 6.2), offered when the engine has apps.
pub const TOOLS: &[(&str, &str)] = &[
```

**with:**

```rust
/// the memory tools (`memory::run`), which share one alias. `propose_app` hands an app folder in
/// the workspace to the person (app library spec section 6.2), and `copy_app` copies an
/// installed app into the workspace, under the same alias; both are offered when the engine has
/// apps.
pub const TOOLS: &[(&str, &str)] = &[
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    ("propose_app", "ProposeApp"),
    ("start_workflow", "Workflow"),
```

**with:**

```rust
    ("propose_app", "ProposeApp"),
    ("copy_app", "ProposeApp"),
    ("start_workflow", "Workflow"),
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// The local tool behind a policy alias (`Bash` is `shell`): what an approval's `tool` names.
/// The memory tools share `Memory`: `local_name_for` tells them apart by the call's input.
pub fn local_name(alias: &str) -> Option<&'static str> {
```

**with:**

```rust
/// The local tool behind a policy alias (`Bash` is `shell`): what an approval's `tool` names.
/// The memory tools share `Memory`, and `propose_app` and `copy_app` share `ProposeApp`:
/// `local_name_for` tells them apart by the call's input.
pub fn local_name(alias: &str) -> Option<&'static str> {
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
/// The local tool of an approval's call: `local_name`, or for `Memory` the memory tool whose
/// argument the input holds (`memory::Tool::for_input`).
pub fn local_name_for(alias: &str, input: &Value) -> Option<&'static str> {
```

**with:**

```rust
/// The local tool of an approval's call: `local_name`, or for `Memory` the memory tool whose
/// argument the input holds (`memory::Tool::for_input`), or for `ProposeApp` `copy_app` when
/// the input names an app.
pub fn local_name_for(alias: &str, input: &Value) -> Option<&'static str> {
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
        return Some(crate::memory::Tool::for_input(input).name());
    }
```

**with:**

```rust
        return Some(crate::memory::Tool::for_input(input).name());
    }
    if alias == crate::apps::PROPOSE_POLICY && input.get("app").is_some() {
        return Some(crate::apps::propose::COPY_TOOL);
    }
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
        &[("path", PathStr, true), ("note", Str, true)],
    ),
    (
        "start_workflow",
```

**with:**

```rust
        &[("path", PathStr, true), ("note", Str, true)],
    ),
    ("copy_app", &[("app", Str, true), ("path", PathStr, true)]),
    (
        "start_workflow",
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    } else if tool == crate::apps::PROPOSE_POLICY {
        Cow::Owned(match engine {
            EngineKind::Claude => format!("mcp__dots__{}", crate::apps::propose::TOOL),
            EngineKind::Local => crate::apps::propose::TOOL.to_string(),
        })
```

**with:**

```rust
    } else if tool == crate::apps::PROPOSE_POLICY {
        // `copy_app` names an app; `propose_app` does not.
        let name = if input.get("app").is_some() {
            crate::apps::propose::COPY_TOOL
        } else {
            crate::apps::propose::TOOL
        };
        Cow::Owned(match engine {
            EngineKind::Claude => format!("mcp__dots__{name}"),
            EngineKind::Local => name.to_string(),
        })
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
                tools.push(crate::apps::propose::mcp_schema());
            }
```

**with:**

```rust
                tools.push(crate::apps::propose::mcp_schema());
                tools.push(crate::apps::propose::copy_mcp_schema());
            }
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
                (Some(crate::apps::propose::TOOL), _) => call_propose(&s, &run_id, &req).await,
                (_, Some(tool)) => call_check_in(&s, &run_id, &req, tool).await,
```

**with:**

```rust
                (Some(crate::apps::propose::TOOL), _) => call_propose(&s, &run_id, &req).await,
                (Some(crate::apps::propose::COPY_TOOL), _) => call_copy(&s, &run_id, &req).await,
                (_, Some(tool)) => call_check_in(&s, &run_id, &req, tool).await,
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
    Ok(match apps.propose(run_id, folder, note).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
```

**with:**

```rust
    Ok(match apps.propose(run_id, folder, note).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&cap_head(text)),
    })
}

/// Checks a `copy_app` call against the dot's policy under `ProposeApp`, waiting for an
/// approval when a rule asks, and copies the app into the folder, confined to the run's
/// workspace (app library spec section 15). A call that is not performed is a tool result with
/// `isError`.
async fn call_copy(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let Some(apps) = s.runner.apps() else {
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
    if let Value::Object(args) = &mut input {
        args.retain(|key, _| crate::apps::propose::COPY_REQUIRED.contains(&key.as_str()));
    }
    let input = match s
        .hub
        .check(run_id, crate::apps::PROPOSE_POLICY, input)
        .await
    {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let workspace = match s.store.get_run(run_id).await {
        Ok(run) => run.workspace_path,
        Err(e) => return Ok(not_performed(&format!("the run could not be read: {e}"))),
    };
    let Some(workspace) = workspace else {
        return Ok(not_performed("this run has no workspace"));
    };
    let path = input["path"].as_str().unwrap_or_default();
    let folder = match crate::engine::local::tools::confine(std::path::Path::new(&workspace), path)
    {
        Ok(folder) => folder,
        Err(e) => return Ok(not_performed(&e)),
    };
    let app = input["app"].as_str().unwrap_or_default();
    Ok(match apps.copy_to(app, folder, path).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --no-fail-fast --test apps_copy_test --test apps_propose_test --test claude_cli_test --test local_tools_test --test apps_run_test`
Expected: PASS: 2, 10, 11, 39 and 7 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1639 passed; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/apps/propose.rs crates/dots-core/src/engine/claude/args.rs crates/dots-core/src/engine/local/app_tools.rs crates/dots-core/src/engine/local/mcp_tools.rs crates/dots-core/src/engine/local/mod.rs crates/dots-core/src/engine/local/tools/mod.rs crates/dots-core/src/runner.rs crates/dots-core/src/server/mcp.rs crates/dots-core/tests/apps_copy_test.rs crates/dots-core/tests/apps_propose_test.rs crates/dots-core/tests/claude_cli_test.rs crates/dots-core/tests/local_tools_test.rs
git commit -m "feat(core): copy_app copies an installed app into a dot's workspace"
```

### Task 6: A waiting review needs the person

Spec §10 "Needs you" and the person's choice: a waiting card counts like an approval (the Inbox count and the tray, the rail's badge in the needs colour, the dot's needs-you number), and the Inbox lists it under "Apps to review" with "Open" to the card.

**Files:**

- Modify: `crates/dots-core/src/{model,runtime}.rs`, `crates/dots-core/src/store/dots.rs`, `apps/dots-app/src/contract.rs`, the fixture, `ui/src/api/types.ts`, `ui/src/lib/{query,runtime-events}.ts`, `ui/src/app/Layout.tsx`, `ui/src/screens/InboxScreen.tsx`
- Create: `ui/src/screens/InboxAppReviews.test.tsx`
- Test: `crates/dots-core/tests/apps_propose_test.rs`; every test that writes an Inbox (`ui/src/test/fixtures.ts`, `ui/src/app/{Layout,Look}.test.tsx`, `ui/src/lib/runtime-events.test.ts`, `ui/src/screens/{BrowserApproval,DotEditorScreen,InboxHeld,InboxMessages,InboxScreen,InboxWatchers}.test.tsx`, `ui/src/screens/home/{HomeRoutes,OpenConversation}.test.tsx`)

**Interfaces:**

- Produces: `model::PendingAppReview { dot_id, dot_name, proposal }`, `Inbox::app_reviews` (newest first); `Runtime::inbox_count` and `Store::dot_counts` count waiting proposals; UI `T.PendingAppReview`, `Inbox.app_reviews` (a lazy schema: the proposal's is declared later in `types.ts`), `keys.appProposals(dotId | null)`, the event `app_proposal_changed` refreshing the proposals, the Inbox and the overview.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_propose_test.rs`, replace:**

```rust
    assert!(proposals(&t, &dot.id).await.is_empty());
    t.rt.shutdown();
```

**with:**

```rust
    assert!(proposals(&t, &dot.id).await.is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_waiting_card_counts_in_what_needs_the_person() {
    let t = start().await;
    let folder = in_workspace(&t, "expenses", &expenses());
    let llm = Llm::start(dones(2)).await;
    let run = finished_run(&t, &llm, "builder").await;
    let before = t.rt.inbox_count().await.unwrap();
    t.rt.apps()
        .propose(&run.id, folder, "Tracks expenses.")
        .await
        .unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.app_reviews.len(), 1);
    assert_eq!(inbox.app_reviews[0].dot_name, "builder");
    assert_eq!(inbox.app_reviews[0].proposal.note, "Tracks expenses.");
    assert_eq!(t.rt.inbox_count().await.unwrap(), before + 1);
    assert_eq!(t.rt.store().dot_counts(&run.dot_id).await.unwrap().0, 1);

    let p = proposals(&t, &run.dot_id).await.remove(0);
    t.rt.decide_app_proposal(&p.id, Decision::Discard)
        .await
        .unwrap();
    assert!(t.rt.inbox().await.unwrap().app_reviews.is_empty());
    assert_eq!(t.rt.store().dot_counts(&run.dot_id).await.unwrap().0, 0);
    t.rt.shutdown();
```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
    // One pending approval, one run to review, one schedule that needs attention, one MCP
    // server whose tools changed, one paused watcher and the runs held for Claude usage.
    expect(await within(nav).findByRole("link", { name: "Inbox, 6 waiting" })).toBeVisible();
  });
```

**with:**

```tsx
    // One pending approval, one run to review, one schedule that needs attention, one MCP
    // server whose tools changed, one paused watcher, the runs held for Claude usage and one
    // app sent for review.
    expect(await within(nav).findByRole("link", { name: "Inbox, 7 waiting" })).toBeVisible();
  });
```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: "Inbox, 6 waiting" })).toBeVisible();
  });
```

**with:**

```tsx
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: "Inbox, 7 waiting" })).toBeVisible();
  });
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
      watchers: [], held: [], app_reviews: [],
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
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [], watchers: [], held: [], app_reviews: [] };
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
    watchers: [], held: [], app_reviews: [],
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
      watchers: [], held: [], app_reviews: [],
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
      waiting = { approvals: [], review_runs: [], attention: [], tools_changed: [], watchers: [], held: [], app_reviews: [] };
      return null;
```

**Create `ui/src/screens/InboxAppReviews.test.tsx`:**

```tsx
import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox, PendingAppReview } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [review] = sample.inbox.app_reviews as [PendingAppReview];
const only = (app_reviews: PendingAppReview[]): Inbox => ({
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held: [],
  app_reviews,
});

describe("apps sent for review in the Inbox", () => {
  it("lists each with its dot and opens the card in the dot's conversation", async () => {
    backend.on("inbox", () => only([review]));
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Apps to review" });
    const item = within(section).getByRole("listitem", { name: "Expenses to review" });
    expect(item).toHaveTextContent("builder sent the app Expenses for review");
    expect(item).toHaveTextContent(review.proposal.note);
    expect(within(item).getByRole("link", { name: "Open" })).toHaveAttribute(
      "href",
      `/home/${review.dot_id}/${review.proposal.root_run_id}`,
    );
  });

  it("names a change by the app it changes, and counts it in the needs colour", async () => {
    const change: PendingAppReview = {
      ...review,
      proposal: { ...review.proposal, summary: { ...review.proposal.summary, update_from: 2 } },
    };
    backend.on("inbox", () => only([change]));
    renderApp("/inbox");
    const item = await screen.findByRole("listitem", { name: "Expenses to review" });
    expect(item).toHaveTextContent("builder sent a change to Expenses for review");
    const link = screen.getByRole("link", { name: "Inbox, 1 waiting" });
    expect(within(link).getByText("1")).toHaveAttribute("data-tone", "needs");
  });
});
```

**In `ui/src/screens/InboxHeld.test.tsx`, replace:**

```tsx
  held,
});
```

**with:**

```tsx
  held,
  app_reviews: [],
});
```

**In `ui/src/screens/InboxMessages.test.tsx`, replace:**

```tsx
    ).toHaveTextContent("Half the tests pass.");
    // The six items (a paused watcher and the held runs among them) of the sample Inbox and the
    // two messages.
    const nav = screen.getByRole("navigation", { name: "Main" });
```

**with:**

```tsx
    ).toHaveTextContent("Half the tests pass.");
    // The seven items (a paused watcher, the held runs and an app to review among them) of the
    // sample Inbox and the two messages.
    const nav = screen.getByRole("navigation", { name: "Main" });
```

**In `ui/src/screens/InboxMessages.test.tsx`, replace:**

```tsx
    await waitFor(() =>
      expect(within(nav).getByRole("link", { name: /Inbox/ })).toHaveTextContent("8"),
    );
```

**with:**

```tsx
    await waitFor(() =>
      expect(within(nav).getByRole("link", { name: /Inbox/ })).toHaveTextContent("9"),
    );
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
/// An Inbox as a test writes it: without paused watchers or held runs unless it says so.
type InboxSpec = Omit<Inbox, "watchers" | "held"> & Partial<Pick<Inbox, "watchers" | "held">>;

```

**with:**

```tsx
/// An Inbox as a test writes it: without paused watchers or held runs unless it says so.
type InboxSpec = Omit<Inbox, "watchers" | "held" | "app_reviews"> &
  Partial<Pick<Inbox, "watchers" | "held" | "app_reviews">>;

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
  let current: Inbox = { watchers: [], held: [], app_reviews: [], ...initial };
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
      current = { watchers: [], held: [], app_reviews: [], ...next };
    },
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
        held: [],
      };
```

**with:**

```tsx
        held: [],
        app_reviews: [],
      };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    watchers: [],
    held: [],
  };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));
```

**with:**

```tsx
    watchers: [],
    held: [],
    app_reviews: [],
  };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    held: [],
  };
```

**with:**

```tsx
    held: [],
    app_reviews: [],
  };
```

**In `ui/src/screens/InboxWatchers.test.tsx`, replace:**

```tsx
  held: [],
});
```

**with:**

```tsx
  held: [],
  app_reviews: [],
});
```

**In `ui/src/screens/home/HomeRoutes.test.tsx`, replace:**

```tsx
    const rail = await screen.findByRole("navigation", { name: "Main" });
    const inbox = await within(rail).findByRole("link", { name: "Inbox, 6 waiting" });
    expect(inbox).toHaveAttribute("title", "Inbox, 6 waiting");
    expect(within(inbox).getByText("6")).toBeVisible();
    await user.click(within(rail).getByRole("link", { name: "Home" }));
```

**with:**

```tsx
    const rail = await screen.findByRole("navigation", { name: "Main" });
    const inbox = await within(rail).findByRole("link", { name: "Inbox, 7 waiting" });
    expect(inbox).toHaveAttribute("title", "Inbox, 7 waiting");
    expect(within(inbox).getByText("7")).toBeVisible();
    await user.click(within(rail).getByRole("link", { name: "Home" }));
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
      watchers: [], held: [], app_reviews: [],
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
    watchers: [], held: [], app_reviews: [],
  }));
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_propose_test a_waiting_card`
Expected: FAIL to compile: no `app_reviews` on `Inbox`.

- [ ] **Step 3: Count and list the waiting reviews**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            held: held_runs(),
        }),
```

**with:**

```rust
            held: held_runs(),
            // Filled in by `app_samples`, which has a proposal.
            app_reviews: vec![],
        }),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    ]);
    samples["Enums"]["ProposalStatus"] = to([
```

**with:**

```rust
    ]);
    samples["Inbox"]["app_reviews"] = to([PendingAppReview {
        dot_id: "01J0DOT".into(),
        dot_name: "builder".into(),
        proposal: proposal("01J0PROP", ProposalStatus::Pending),
    }]);
    samples["Enums"]["ProposalStatus"] = to([
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub held: Vec<HeldRun>,
}
```

**with:**

```rust
    pub held: Vec<HeldRun>,
    /// Apps dots sent for review that wait for the person, newest first (app library spec
    /// section 10: a waiting card counts like an approval).
    pub app_reviews: Vec<PendingAppReview>,
}

/// An app proposal that waits for the person, as the Inbox lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAppReview {
    pub dot_id: String,
    pub dot_name: String,
    pub proposal: crate::apps::propose::AppProposal,
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
        let app_reviews = self
            .store
            .list_app_proposals(None)
            .await?
            .into_iter()
            .filter(|p| p.status == crate::apps::propose::ProposalStatus::Pending)
            .map(|proposal| crate::model::PendingAppReview {
                dot_name: name(&proposal.dot_id),
                dot_id: proposal.dot_id.clone(),
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
            app_reviews,
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
            // A waiting review card counts like an approval (app library spec section 10).
            + inbox.app_reviews.len()
            + self.store.count_waiting_messages().await?)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
    /// What the dot has for the person (needs-you spec section 2), as `(needs_you, unread)`:
    /// its open questions and pending approvals; and those plus its runs that ended and its
    /// notes posted after its `seen_at` (none without one). `NotFound` for an unknown dot.
    pub async fn dot_counts(&self, id: &str) -> Result<(u32, u32)> {
```

**with:**

```rust
    /// What the dot has for the person (needs-you spec section 2), as `(needs_you, unread)`:
    /// its open questions, pending approvals, paused watchers and app proposals waiting for
    /// review; and those plus its runs that ended and its notes posted after its `seen_at`
    /// (none without one). `NotFound` for an unknown dot.
    pub async fn dot_counts(&self, id: &str) -> Result<(u32, u32)> {
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
             + (SELECT COUNT(*) FROM watchers w WHERE w.dot_id = d.id \
              AND w.paused_reason IS NOT NULL) AS needs_you, \
             CASE WHEN d.seen_at IS NULL THEN 0 ELSE \
```

**with:**

```rust
             + (SELECT COUNT(*) FROM watchers w WHERE w.dot_id = d.id \
              AND w.paused_reason IS NOT NULL) \
             + (SELECT COUNT(*) FROM app_proposals p WHERE p.dot_id = d.id \
              AND p.status = 'pending') AS needs_you, \
             CASE WHEN d.seen_at IS NULL THEN 0 ELSE \
```

**In `ui/src/api/types.ts`, replace:**

```ts
  held: z.array(HeldRun),
});
```

**with:**

```ts
  held: z.array(HeldRun),
  // Apps dots sent for review that wait for the person, newest first. Lazy: the proposal's
  // schema is declared with the apps, below.
  app_reviews: z.array(z.lazy(() => PendingAppReview)),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
});
// The person's answer on a review card.
```

**with:**

```ts
});
// An app proposal that waits for the person, as the Inbox lists it.
export const PendingAppReview = z.strictObject({
  dot_id: z.string(),
  dot_name: z.string(),
  proposal: AppProposal,
});
export type PendingAppReview = z.infer<typeof PendingAppReview>;
// The person's answer on a review card.
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
        // Runs held for Claude usage wait as one line, however many.
        Math.min(inbox.held.length, 1)
      : 0) + (messages ?? []).filter(waits).length;
```

**with:**

```tsx
        // Runs held for Claude usage wait as one line, however many.
        Math.min(inbox.held.length, 1) +
        // A waiting app review counts like an approval.
        inbox.app_reviews.length
      : 0) + (messages ?? []).filter(waits).length;
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
  // (deep-teal spec section 4).
  const needs = (inbox?.approvals.length ?? 0) > 0 || (messages ?? []).some(isOpenQuestion);
  return (
```

**with:**

```tsx
  // (deep-teal spec section 4).
  const needs =
    (inbox?.approvals.length ?? 0) > 0 ||
    (inbox?.app_reviews.length ?? 0) > 0 ||
    (messages ?? []).some(isOpenQuestion);
  return (
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  watchers: (dotId: string) => ["watchers", dotId] as const,
  limits: ["limits"] as const,
```

**with:**

```ts
  watchers: (dotId: string) => ["watchers", dotId] as const,
  /// The apps dots proposed, of one dot or of all (null).
  appProposals: (dotId: string | null) => ["app-proposals", dotId] as const,
  limits: ["limits"] as const,
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      break;
    case "limits_updated":
```

**with:**

```ts
      break;
    case "app_proposal_changed":
      // A review card was posted or decided: the conversation's cards, the Inbox's reviews and
      // the dots' needs-you counts.
      invalidate(qc, ["app-proposals"]);
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.overview);
      break;
    case "limits_updated":
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
  PausedWatcher,
  ReviewRun,
```

**with:**

```tsx
  PausedWatcher,
  PendingAppReview,
  ReviewRun,
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
      />
    </li>
```

**with:**

```tsx
      />
    </li>
  );
}

/// An app a dot sent for review (app library spec section 10). The card, with its preview, waits
/// in the dot's conversation; the row opens it there.
function AppReviewRow({ item }: { item: PendingAppReview }) {
  const { proposal } = item;
  const { title, update_from: from } = proposal.summary;
  const what = from === null ? `the app ${title}` : `a change to ${title}`;
  return (
    <li
      aria-label={`${title} to review`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">
          {item.dot_name} sent {what} for review
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {proposal.note} · {relativeTime(proposal.created_at)}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/home/${item.dot_id}/${proposal.root_run_id}`}>Open</Link>
      </Button>
    </li>
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
    held,
  } = inbox.data;
```

**with:**

```tsx
    held,
    app_reviews: reviews,
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
    reviews.length === 0
  ) {
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
        <EmptyState icon={InboxIcon}>
          Nothing waits for you. Questions and notes from dots, approvals a dot asks for,
          schedules that need attention, paused watchers and finished runs to review show up
          here.
        </EmptyState>
```

**with:**

```tsx
        <EmptyState icon={InboxIcon}>
          Nothing waits for you. Questions and notes from dots, approvals a dot asks for, apps
          sent for review, schedules that need attention, paused watchers and finished runs to
          review show up here.
        </EmptyState>
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
      {reviews.length > 0 && (
        <section aria-labelledby="inbox-app-reviews" className="flex flex-col gap-2">
          <h2 id="inbox-app-reviews" className="font-semibold">
            Apps to review
          </h2>
          <ul className="flex flex-col gap-2">
            {reviews.map((item) => (
              <AppReviewRow key={item.proposal.id} item={item} />
            ))}
          </ul>
        </section>
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_propose_test --test store_dots_test`, then `UPDATE_CONTRACT=1 cargo test -j 2 -p dots-app contract`, then in `ui`: `bun run typecheck` and `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/InboxAppReviews.test.tsx src/app src/screens/home/HomeRoutes.test.tsx src/screens/InboxMessages.test.tsx src/screens/InboxScreen.test.tsx`.
Expected: PASS: 11 and 16 tests; the fixture's Inbox gains one app review; `tsc` clean; the UI files pass, the rail's count now 7 for the sample Inbox.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo test -j 2 -p dots-app`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: `dots-core` 1640 passed, `dots-app` 124, the UI 750 in 72 files; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/src/model.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/dots.rs crates/dots-core/tests/apps_propose_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/app/Layout.test.tsx ui/src/app/Layout.tsx ui/src/app/Look.test.tsx ui/src/lib/query.ts ui/src/lib/runtime-events.test.ts ui/src/lib/runtime-events.ts ui/src/screens/BrowserApproval.test.tsx ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/InboxAppReviews.test.tsx ui/src/screens/InboxHeld.test.tsx ui/src/screens/InboxMessages.test.tsx ui/src/screens/InboxScreen.test.tsx ui/src/screens/InboxScreen.tsx ui/src/screens/InboxWatchers.test.tsx ui/src/screens/home/HomeRoutes.test.tsx ui/src/screens/home/OpenConversation.test.tsx ui/src/test/fixtures.ts
git commit -m "feat: a waiting app review counts in what needs the person and shows in the Inbox"
```

### Task 7: The folded Minors: logged results, all tests' time, accept without the thread lock

The parked Minors the person folded in: the call log keeps at most 30 KB of a result; an app's test files run 60 s together at most; and accepting an app lets go of the thread lock while its migrations run, marking the thread as deciding so that no chat, reply, follow-up or second decision continues it meanwhile.

**Files:**

- Modify: `crates/dots-core/src/apps/{mod,registry,tests}.rs`, `crates/dots-core/src/{followups,checkins,runtime}.rs`
- Test: `crates/dots-core/tests/{apps_calls_test,apps_tests_test,apps_propose_test}.rs`

**Interfaces:**

- Produces: `apps::MAX_LOGGED = 30 * 1024` (input and result); `apps::tests::{TOTAL_TIME_LIMIT = 60 s, run_tests_bounded(src, limit, total) -> TestReport}` (a file past the total fails with "<file> did not run: the app's tests ran past <s> s in all"); `FollowUps::{begin_deciding(root) -> bool, end_deciding(root), deciding(root) -> bool}`; `checkins::DECIDING` ("Dots is still adding the app you accepted in this chat; send your message again in a moment"), the `Conflict` a chat or reply to a deciding thread gets.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_calls_test.rs`, replace:**

```rust
    assert_eq!(c.now, local.to_rfc3339_opts(SecondsFormat::Secs, false));
}
```

**with:**

```rust
    assert_eq!(c.now, local.to_rfc3339_opts(SecondsFormat::Secs, false));
}

#[tokio::test]
async fn a_long_result_is_logged_as_its_size_and_start() {
    let manifest = common::apps::MANIFEST.replacen(
        "\"tools\": [",
        "\"tools\": [\n    { \"name\": \"pad\", \"description\": \"A long answer\", \
         \"input\": { \"type\": \"object\" }, \"writes\": true },",
        1,
    );
    let logic = format!(
        "{}\nexport function pad(input, ctx) {{ return {{ text: \"x\".repeat(40000) }}; }}\n",
        common::apps::LOGIC
    );
    let mut files = common::apps::expenses_with("app.json", Some(&manifest));
    files.retain(|(path, _)| *path != "logic.js");
    files.push(("logic.js", logic));
    let f = fixture(&files).await;
    let out = f
        .apps
        .call("expenses", "pad", json!({}), Actor::Person)
        .await
        .unwrap();
    assert_eq!(
        out["text"].as_str().unwrap().len(),
        40_000,
        "the caller gets it whole"
    );
    let (item, _) = f.apps.get("expenses").await.unwrap();
    let logged = f.store.app_calls(&item.id, 1).await.unwrap().remove(0);
    let result = logged.result.unwrap();
    assert_eq!(result["truncated"], true);
    assert_eq!(result["bytes"], out.to_string().len());
    assert_eq!(
        result["head"].as_str().unwrap().len(),
        dots_core::apps::MAX_LOGGED
    );
}
```

**In `crates/dots-core/tests/apps_propose_test.rs`, replace:**

```rust
#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_under_its_policy() {
```

**with:**

```rust
#[tokio::test]
async fn while_an_update_migrates_its_thread_waits_and_other_threads_go_on() {
    let t = start().await;
    let first = in_workspace(&t, "first", &expenses());
    let apps = t.rt.apps();
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let ink = json!({ "item": "Ink", "amount": 3, "on": "2026-10-20" });
    apps.call(
        "expenses",
        "add_expense",
        ink,
        dots_core::apps::call::Actor::Person,
    )
    .await
    .unwrap();
    // Slow on real data only: the tests' empty databases seed no recursion.
    let slow = "CREATE TABLE pad AS WITH RECURSIVE c(x) AS (SELECT 1 FROM expenses UNION ALL \
                SELECT x + 1 FROM c WHERE x < 4000000) SELECT max(x) AS x FROM c;\n";
    let mut files = expenses();
    files.push(("migrations/0002_pad.sql", slow.to_string()));
    let next = in_workspace(&t, "next", &files);
    let llm = Llm::start(dones(6)).await;
    let fixer = finished_run(&t, &llm, "fixer").await;
    let other = finished_run(&t, &llm, "other").await;
    apps.propose(&fixer.id, next, "Pads.").await.unwrap();
    let p = proposals(&t, &fixer.dot_id).await.remove(0);
    let done = std::sync::atomic::AtomicBool::new(false);
    let decide = async {
        let decided = t.rt.decide_app_proposal(&p.id, Decision::Add).await;
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        decided
    };
    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let same = t.rt.chat(&fixer.dot_id, "And?", Some(&fixer.id)).await;
        let elsewhere = t.rt.chat(&other.dot_id, "Hello", Some(&other.id)).await;
        (
            same,
            elsewhere,
            done.load(std::sync::atomic::Ordering::SeqCst),
        )
    };
    let (decided, (same, elsewhere, finished)) = tokio::join!(decide, meanwhile);
    assert!(
        !finished,
        "the migration outlasted the chats; make it slower"
    );
    match same {
        Err(Error::Conflict(m)) => assert_eq!(m, dots_core::checkins::DECIDING),
        other => panic!("{other:?}"),
    }
    assert!(elsewhere.is_ok(), "{elsewhere:?}");
    let (decided, _) = decided.unwrap();
    assert_eq!(decided.status, ProposalStatus::Added);
    let (_, config) = apps.get("expenses").await.unwrap();
    assert_eq!(config.version, 2);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_under_its_policy() {
```

**In `crates/dots-core/tests/apps_tests_test.rs`, replace:**

```rust
use dots_core::apps::source::{read_folder, AppSource};
use dots_core::apps::tests::{run_tests, run_tests_within, TestReport, NO_TESTS};

```

**with:**

```rust
use dots_core::apps::source::{read_folder, AppSource};
use dots_core::apps::tests::{
    run_tests, run_tests_bounded, run_tests_within, TestReport, NO_TESTS,
};

```

**In `crates/dots-core/tests/apps_tests_test.rs`, replace:**

```rust
#[test]
fn tests_see_no_clock_or_random_numbers() {
```

**with:**

```rust
#[test]
fn the_test_files_together_are_bounded_too() {
    let forever = "test(\"forever\", () => { for (;;) {} });\n".to_string();
    let mut files = expenses_with("tests/expenses.test.js", None);
    for name in ["tests/a.test.js", "tests/b.test.js", "tests/c.test.js"] {
        files.push((name, forever.clone()));
    }
    let report = run_tests_bounded(
        &source(&files),
        Duration::from_millis(300),
        Duration::from_millis(450),
    );
    let last = report.results.last().unwrap();
    assert_eq!(last.file, "tests/c.test.js", "{report:?}");
    assert_eq!(
        last.error.as_deref(),
        Some("tests/c.test.js did not run: the app's tests ran past 0.45 s in all")
    );
    assert!(!report.passed());
}

#[test]
fn tests_see_no_clock_or_random_numbers() {
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --no-fail-fast --test apps_calls_test --test apps_tests_test --test apps_propose_test`
Expected: FAIL to compile: no `MAX_LOGGED`, no `run_tests_bounded`, no `checkins::DECIDING`.

- [ ] **Step 3: Bound and release**

**In `crates/dots-core/src/apps/mod.rs`, replace:**

```rust

pub use registry::{AppCall, Apps, APPS_DIR};
pub use run::{RunApp, RunApps};
```

**with:**

```rust

pub use registry::{AppCall, Apps, APPS_DIR, MAX_LOGGED};
pub use run::{RunApp, RunApps};
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            let (result, error, ids, changes) = match &ran {
                Ok(out) => (Some(out.result.clone()), None, out.ids.clone(), out.changes),
                Err(e) => (None, Some(e.to_string()), Vec::new(), 0),
```

**with:**

```rust
            let (result, error, ids, changes) = match &ran {
                Ok(out) => (
                    Some(log_value(out.result.clone())),
                    None,
                    out.ids.clone(),
                    out.changes,
                ),
                Err(e) => (None, Some(e.to_string()), Vec::new(), 0),
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
                run_id,
                input: logged_input(input),
                now,
```

**with:**

```rust
                run_id,
                input: log_value(input),
                now,
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust

/// The most of a call's input the call log keeps, as JSON text.
const MAX_LOGGED_INPUT: usize = 30 * 1024;

/// A call's input as the call log keeps it: whole, or past `MAX_LOGGED_INPUT` its size and its
/// start.
fn logged_input(input: Value) -> Value {
    let text = input.to_string();
    if text.len() <= MAX_LOGGED_INPUT {
        return input;
```

**with:**

```rust

/// The most of a call's input, and of its result, the call log keeps, as JSON text.
pub const MAX_LOGGED: usize = 30 * 1024;

/// A call's input or result as the call log keeps it: whole, or past `MAX_LOGGED` its size and
/// its start.
fn log_value(input: Value) -> Value {
    let text = input.to_string();
    if text.len() <= MAX_LOGGED {
        return input;
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
    }
    let mut at = MAX_LOGGED_INPUT;
    while !text.is_char_boundary(at) {
```

**with:**

```rust
    }
    let mut at = MAX_LOGGED;
    while !text.is_char_boundary(at) {
```

**In `crates/dots-core/src/apps/tests.rs`, replace:**

```rust
pub const FILE_TIME_LIMIT: Duration = Duration::from_secs(10);
/// The most tests one file may hold.
```

**with:**

```rust
pub const FILE_TIME_LIMIT: Duration = Duration::from_secs(10);
/// The longest an app's test files may run together: a file past it does not run.
pub const TOTAL_TIME_LIMIT: Duration = Duration::from_secs(60);
/// The most tests one file may hold.
```

**In `crates/dots-core/src/apps/tests.rs`, replace:**

```rust
pub fn run_tests(src: &AppSource) -> TestReport {
    run_tests_within(src, FILE_TIME_LIMIT)
}
```

**with:**

```rust
pub fn run_tests(src: &AppSource) -> TestReport {
    run_tests_bounded(src, FILE_TIME_LIMIT, TOTAL_TIME_LIMIT)
}
```

**In `crates/dots-core/src/apps/tests.rs`, replace:**

```rust
pub fn run_tests_within(src: &AppSource, limit: Duration) -> TestReport {
    let mut results = Vec::new();
```

**with:**

```rust
pub fn run_tests_within(src: &AppSource, limit: Duration) -> TestReport {
    run_tests_bounded(src, limit, TOTAL_TIME_LIMIT)
}

/// `run_tests`, each file bounded by `limit` and all of them by `total`: a file starts with
/// what is left of `total`, and once nothing is left the files after it fail without running.
pub fn run_tests_bounded(src: &AppSource, limit: Duration, total: Duration) -> TestReport {
    let started = Instant::now();
    let mut results = Vec::new();
```

**In `crates/dots-core/src/apps/tests.rs`, replace:**

```rust
        let file = format!("{TESTS_DIR}/{name}");
        results.extend(run_file(src, &file, code, limit));
    }
```

**with:**

```rust
        let file = format!("{TESTS_DIR}/{name}");
        let left = total.saturating_sub(started.elapsed());
        if left.is_zero() {
            results.push(TestResult {
                file: file.clone(),
                name: String::new(),
                passed: false,
                error: Some(format!(
                    "{file} did not run: the app's tests ran past {} s in all",
                    total.as_secs_f32()
                )),
            });
            continue;
        }
        results.extend(run_file(src, &file, code, limit.min(left)));
    }
```

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
/// The refusal of a reply to a thread that has not ended, in the words `Runtime::chat` gives.
pub fn thread_busy(status: RunStatus) -> Error {
```

**with:**

```rust
/// The refusal of a reply to a thread that has not ended, in the words `Runtime::chat` gives.
/// What a reply or a chat message is told while an app the person accepted in the thread is
/// being added (`Runtime::decide_app_proposal`).
pub const DECIDING: &str = "Dots is still adding the app you accepted in this chat; send your \
    message again in a moment";

pub fn thread_busy(status: RunStatus) -> Error {
```

**In `crates/dots-core/src/checkins.rs`, replace:**

```rust
        let _one_at_a_time = lock.lock().await;
        let thread = self.store.list_thread(&asked.root_run_id).await?;
```

**with:**

```rust
        let _one_at_a_time = lock.lock().await;
        if self.follow_ups.deciding(&asked.root_run_id) {
            return Err(Error::Conflict(DECIDING.into()));
        }
        let thread = self.store.list_thread(&asked.root_run_id).await?;
```

**In `crates/dots-core/src/followups.rs`, replace:**

```rust
    thread_lock: Arc<Mutex<()>>,
    wake: Notify,
```

**with:**

```rust
    thread_lock: Arc<Mutex<()>>,
    /// The threads (by root run id) whose app decision is being carried out without the
    /// thread lock: nothing continues them meanwhile (`Runtime::decide_app_proposal`).
    deciding: std::sync::Mutex<std::collections::HashSet<String>>,
    wake: Notify,
```

**In `crates/dots-core/src/followups.rs`, replace:**

```rust
            thread_lock: Arc::new(Mutex::new(())),
            wake: Notify::new(),
```

**with:**

```rust
            thread_lock: Arc::new(Mutex::new(())),
            deciding: Default::default(),
            wake: Notify::new(),
```

**In `crates/dots-core/src/followups.rs`, replace:**

```rust
        self.thread_lock.clone()
    }
```

**with:**

```rust
        self.thread_lock.clone()
    }

    /// Marks the thread `root` as deciding, under the thread lock; `false` when it already is.
    pub fn begin_deciding(&self, root: &str) -> bool {
        self.deciding
            .lock()
            .expect("deciding threads")
            .insert(root.to_string())
    }

    pub fn end_deciding(&self, root: &str) {
        self.deciding.lock().expect("deciding threads").remove(root);
    }

    /// Whether the thread `root` waits for an app decision being carried out: a writer that
    /// continues threads leaves it alone until then.
    pub fn deciding(&self, root: &str) -> bool {
        self.deciding
            .lock()
            .expect("deciding threads")
            .contains(root)
    }
```

**In `crates/dots-core/src/followups.rs`, replace:**

```rust
        };
        if thread.iter().any(|r| !r.status.is_terminal()) {
            return Ok(None);
```

**with:**

```rust
        };
        let deciding = from.as_ref().is_some_and(|f| self.deciding(&f.root_run_id));
        if deciding || thread.iter().any(|r| !r.status.is_terminal()) {
            return Ok(None);
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            // text is shown to the user as it is.
            let thread = self.store.list_thread(&named.root_run_id).await?;
```

**with:**

```rust
            // text is shown to the user as it is.
            if self.follow_ups.deciding(&named.root_run_id) {
                return Err(Error::Conflict(crate::checkins::DECIDING.into()));
            }
            let thread = self.store.list_thread(&named.root_run_id).await?;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    /// as an answered question does (`CheckIns::reply`). A thread that has not ended is refused,
    /// as a reply is.
    pub async fn decide_app_proposal(
```

**with:**

```rust
    /// as an answered question does (`CheckIns::reply`). A thread that has not ended is refused,
    /// as a reply is. Adding runs the app's migrations, which may take long on real data: the
    /// thread lock is let go meanwhile, and the thread is marked as deciding, so no chat, reply
    /// or follow-up continues it and no second decision is made in it.
    pub async fn decide_app_proposal(
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        let p = self.store.get_app_proposal(id).await?;
```

**with:**

```rust
        let lock = self.follow_ups.thread_lock();
        let mut one_at_a_time = lock.lock().await;
        let p = self.store.get_app_proposal(id).await?;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let asked = self.store.get_run(&p.run_id).await?;
        let thread = self.store.list_thread(&asked.root_run_id).await?;
```

**with:**

```rust
        let asked = self.store.get_run(&p.run_id).await?;
        if self.follow_ups.deciding(&asked.root_run_id) {
            return Err(Error::Conflict(crate::checkins::DECIDING.into()));
        }
        let thread = self.store.list_thread(&asked.root_run_id).await?;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
                let on_kept = decision == Decision::AddWithKeptData;
                let item = self.apps.accept(&p, on_kept).await?;
                let version = match &item.config {
```

**with:**

```rust
                let on_kept = decision == Decision::AddWithKeptData;
                self.follow_ups.begin_deciding(&asked.root_run_id);
                drop(one_at_a_time);
                let accepted = self.apps.accept(&p, on_kept).await;
                one_at_a_time = lock.lock().await;
                self.follow_ups.end_deciding(&asked.root_run_id);
                let item = accepted?;
                let version = match &item.config {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            .await?;
        self.runner.enqueued(&run);
```

**with:**

```rust
            .await?;
        drop(one_at_a_time);
        self.runner.enqueued(&run);
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --no-fail-fast --test apps_calls_test --test apps_tests_test --test apps_propose_test --test apps_registry_test`
Expected: PASS: 8, 14, 12 and 21 tests. `while_an_update_migrates_its_thread_waits_and_other_threads_go_on` fails with "the migration outlasted the chats; make it slower" only if the machine runs the 4,000,000-step migration in under 150 ms: raise the bound in its migration, not the wait.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1643 passed; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/apps/mod.rs crates/dots-core/src/apps/registry.rs crates/dots-core/src/apps/tests.rs crates/dots-core/src/checkins.rs crates/dots-core/src/followups.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/apps_calls_test.rs crates/dots-core/tests/apps_propose_test.rs crates/dots-core/tests/apps_tests_test.rs
git commit -m "fix(core): bound logged results and all of an app's tests, and accept an app without the thread lock"
```

### Task 8: The `dots-app` protocol, the page's client, and the screen's new commands

Spec §5 "Screen": how `ui/` files reach the frame (the builder's call): Dots' own protocol, serving only a page's own `ui/` files with a CSP limited to its folder, and putting Dots' client at the top of `index.html`. The rows now name the tools the screen may call and what the app asks to reach. The commands the card and the Apps screen need: the diff, the preview, the kept data.

**Files:**

- Create: `apps/dots-app/src/app_frame.rs`, `apps/dots-app/src/app_client.js`
- Modify: `apps/dots-app/src/{lib,commands,contract}.rs`, `apps/dots-app/tauri.conf.json`, `crates/dots-core/src/apps/{screen,propose}.rs`, `ui/src/api/{types,commands}.ts`, `ui/src/api/contract.test.ts`, the fixture
- Test: `crates/dots-core/tests/apps_screen_test.rs` (the rows' tools), `app_frame.rs`'s own tests

**Interfaces:**

- Consumes: Tasks 1, 2, 4.
- Produces:
  - `app_frame::{SCHEME = "dots-app", ORIGIN = "http://dots-app.localhost", CLIENT, Target::{App { name, version, path }, Preview { id, path }}, parse(path) -> Option<Target>, csp(base) -> String, content_type(path), with_client(html) -> Vec<u8>, response(target, found) -> Response<Vec<u8>>, respond(rt, request) -> Response<Vec<u8>>}`; registered with `register_asynchronous_uri_scheme_protocol`. Dots' CSP gains `frame-src http://dots-app.localhost`.
  - The client (`window.dots`, frozen): `call(tool, input = {}) -> Promise`, `onChange(cb) -> unsubscribe`, `onTheme(cb) -> unsubscribe`; it takes one port, from `window.parent`, on `{ type: "dots:connect" }`, queues calls until then, sends `{ type: "call", id, tool, input }`, and reads `{ type: "result", id, ok, result | error }`, `{ type: "theme", mode, tokens }` (CSS variables `--dots-<token>`, `data-theme`, `color-scheme`) and `{ type: "changed" }`.
  - `AppRow::{tools: Vec<ToolSummary>, permissions: Vec<Permission>}`; `propose::tool_summaries(manifest)`.
  - Commands `app_proposal_diff(id) -> WorktreeDiff`, `app_preview_open(id) -> String`, `app_preview_call(preview, tool, input) -> Value`, `app_preview_close(preview)`, `apps_kept_list() -> Vec<KeptData>`, `app_kept_delete(id)`; UI `commands.{appProposalDiff, appPreviewOpen, appPreviewCall, appPreviewClose, appsKeptList, appKeptDelete}`, `T.{ToolSummary, KeptData}`, `AppRow.{tools, permissions}`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_screen_test.rs`, replace:**

```rust
    assert_eq!(rows[0].error, None);

```

**with:**

```rust
    assert_eq!(rows[0].error, None);
    let tools: Vec<(&str, bool)> = rows[0]
        .tools
        .iter()
        .map(|t| (t.name.as_str(), t.writes))
        .collect();
    assert_eq!(
        tools,
        [
            ("add_expense", true),
            ("list_expenses", false),
            ("month_total", false)
        ]
    );

```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  AppRow: T.AppRow.array(),
  AppFile: T.AppFile.array(),
```

**with:**

```ts
  AppRow: T.AppRow.array(),
  KeptData: T.KeptData.array(),
  AppFile: T.AppFile.array(),
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_screen_test`
Expected: FAIL to compile: no `tools` on `AppRow`.

- [ ] **Step 3: Serve the pages and add the commands**

**Create `apps/dots-app/src/app_client.js`:**

```
// Dots' client for an app's screen (app library spec section 5). The `dots-app` protocol puts it
// at the top of the app's index.html. The page talks to Dots only through the MessagePort the
// host sends once it has loaded: `dots.call(tool, input)` answers with the tool's result or
// rejects with its error, `dots.onChange(cb)` hears that the app's data changed, and the
// theme's colours arrive as CSS variables (`--dots-<token>`) on the root element, with
// `data-theme` set to "light" or "dark" (`dots.onTheme(cb)` hears a change).
(() => {
  let port = null;
  let next = 0;
  const waiting = new Map();
  const queued = [];
  const changes = new Set();
  const themes = new Set();
  const root = document.documentElement;
  const each = (set, arg) => {
    for (const cb of set) {
      try {
        cb(arg);
      } catch (e) {
        console.error(e);
      }
    }
  };
  const theme = (msg) => {
    for (const [name, value] of Object.entries(msg.tokens)) {
      root.style.setProperty(`--dots-${name}`, value);
    }
    root.dataset.theme = msg.mode;
    root.style.colorScheme = msg.mode;
    each(themes, msg.mode);
  };
  const receive = (event) => {
    const msg = event.data;
    if (msg === null || typeof msg !== "object") return;
    if (msg.type === "result") {
      const call = waiting.get(msg.id);
      if (call === undefined) return;
      waiting.delete(msg.id);
      if (msg.ok) call.resolve(msg.result);
      else call.reject(new Error(String(msg.error)));
    } else if (msg.type === "theme") {
      theme(msg);
    } else if (msg.type === "changed") {
      each(changes);
    }
  };
  window.addEventListener("message", (event) => {
    // One port, from the window that framed this page.
    if (port !== null || event.source !== window.parent) return;
    if (event.data === null || typeof event.data !== "object") return;
    if (event.data.type !== "dots:connect" || event.ports.length !== 1) return;
    port = event.ports[0];
    port.onmessage = receive;
    for (const msg of queued.splice(0)) port.postMessage(msg);
  });
  const send = (msg) => (port === null ? queued.push(msg) : port.postMessage(msg));
  const dots = Object.freeze({
    call(tool, input = {}) {
      return new Promise((resolve, reject) => {
        next += 1;
        waiting.set(next, { resolve, reject });
        send({ type: "call", id: next, tool: String(tool), input });
      });
    },
    onChange(cb) {
      changes.add(cb);
      return () => changes.delete(cb);
    },
    onTheme(cb) {
      themes.add(cb);
      return () => themes.delete(cb);
    },
  });
  Object.defineProperty(window, "dots", { value: dots, enumerable: true });
})();
```

**Create `apps/dots-app/src/app_frame.rs`:**

```rust
//! The page an app's screen runs in (app library spec section 5, "Screen"). The `dots-app`
//! protocol serves the files below `ui/` of an installed app's version
//! (`/app/<name>/<version>/<path>`) or of a review card's preview (`/preview/<id>/<path>`), and
//! nothing else. Each answer carries a CSP that lets the page load only from its own folder and
//! connect nowhere, and `index.html` gets Dots' client for the bridge (`app_client.js`) at the
//! top. The UI frames the page with `sandbox="allow-scripts"`, so it runs in an opaque origin
//! and never sees Tauri's IPC.

use dots_core::{Error, Runtime};
use tauri::http::{header, Method, Request, Response, StatusCode};

/// The protocol's name.
pub const SCHEME: &str = "dots-app";
/// Where WebView2 serves the protocol on Windows. The UI frames pages from it
/// (`ui/src/lib/app-bridge.ts`), and Dots' own CSP lets it be framed (`tauri.conf.json`).
pub const ORIGIN: &str = "http://dots-app.localhost";
/// The client the page talks to Dots through.
pub const CLIENT: &str = include_str!("app_client.js");

/// What a request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A file of the app `name` at `version`.
    App {
        name: String,
        version: u32,
        path: String,
    },
    /// A file of the preview `id`.
    Preview { id: String, path: String },
}

impl Target {
    /// The path below `ui/`.
    pub fn path(&self) -> &str {
        match self {
            Target::App { path, .. } | Target::Preview { path, .. } => path,
        }
    }

    /// The folder the page may load from: its own `ui/` files.
    pub fn base(&self) -> String {
        match self {
            Target::App { name, version, .. } => format!("{ORIGIN}/app/{name}/{version}/"),
            Target::Preview { id, .. } => format!("{ORIGIN}/preview/{id}/"),
        }
    }
}

/// Decodes `%XX` escapes; `None` for a broken one or bytes that are not UTF-8.
fn decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = segment.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The target of the request path `path`; `None` for anything else. An empty file path is
/// `index.html`. The files are looked up by name in the app's own list, so no path reaches
/// past it; a `..`, `.` or empty segment is refused all the same.
pub fn parse(path: &str) -> Option<Target> {
    let (kind, rest) = path.strip_prefix('/')?.split_once('/')?;
    let (head, file) = match kind {
        "app" => {
            let mut it = rest.splitn(3, '/');
            let name = it.next()?.to_string();
            let version: u32 = it.next()?.parse().ok()?;
            (Some((name, version)), it.next().unwrap_or(""))
        }
        "preview" => (None, rest),
        _ => return None,
    };
    let (id, file) = match head {
        Some(_) => (None, file),
        None => {
            let (id, file) = file.split_once('/').unwrap_or((file, ""));
            (Some(id.to_string()), file)
        }
    };
    let file = if file.is_empty() { "index.html" } else { file };
    let mut segments = Vec::new();
    for segment in file.split('/') {
        let segment = decode(segment)?;
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\') {
            return None;
        }
        segments.push(segment);
    }
    let path = segments.join("/");
    Some(match (head, id) {
        (Some((name, version)), _) => Target::App {
            name,
            version,
            path,
        },
        (None, Some(id)) if !id.is_empty() => Target::Preview { id, path },
        _ => return None,
    })
}

/// The page's CSP (spec section 5): scripts, styles, images and fonts from its own folder (and
/// inline), images also as `data:`; no connection, frame, form, or base URL.
pub fn csp(base: &str) -> String {
    format!(
        "default-src 'none'; script-src 'unsafe-inline' {base}; style-src 'unsafe-inline' {base}; \
         img-src data: {base}; font-src {base}; connect-src 'none'; frame-src 'none'; \
         form-action 'none'; base-uri 'none'"
    )
}

/// The type of the file `path`, by its extension.
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `html` with the client at its top: after a leading `<!doctype …>` (which must stay first, or
/// the page renders in quirks mode), else before everything.
pub fn with_client(html: &[u8]) -> Vec<u8> {
    let script = format!("<script>{CLIENT}</script>");
    let text = String::from_utf8_lossy(html);
    let lead = text.len()
        - text
            .trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n'])
            .len();
    let at = if text[lead..].to_ascii_lowercase().starts_with("<!doctype") {
        text[lead..].find('>').map_or(lead, |end| lead + end + 1)
    } else {
        lead
    };
    let mut out = String::with_capacity(text.len() + script.len());
    out.push_str(&text[..at]);
    out.push_str(&script);
    out.push_str(&text[at..]);
    out.into_bytes()
}

fn plain(status: StatusCode, text: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
        .body(text.as_bytes().to_vec())
        .expect("a plain response")
}

/// The answer for `target` once its file was looked up.
pub fn response(target: &Target, found: dots_core::Result<Vec<u8>>) -> Response<Vec<u8>> {
    let bytes = match found {
        Ok(bytes) => bytes,
        Err(Error::NotFound(_)) => return plain(StatusCode::NOT_FOUND, "not found"),
        Err(e) => return plain(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let path = target.path();
    let body = if path == "index.html" {
        with_client(&bytes)
    } else {
        bytes
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(header::CONTENT_SECURITY_POLICY, csp(&target.base()))
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .header("Referrer-Policy", "no-referrer")
        .body(body)
        .expect("a file response")
}

/// Answers a request of the protocol: only `GET` of a file an app or a preview has.
pub async fn respond(rt: Option<&Runtime>, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() != Method::GET {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "only GET");
    }
    let Some(target) = parse(request.uri().path()) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let Some(rt) = rt else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "Dots did not start");
    };
    let found = match &target {
        Target::App {
            name,
            version,
            path,
        } => rt.apps().ui_file(name, *version, path).await,
        Target::Preview { id, path } => rt.apps().preview_file(id, path),
    };
    response(&target, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, version: u32, path: &str) -> Option<Target> {
        Some(Target::App {
            name: name.into(),
            version,
            path: path.into(),
        })
    }

    #[test]
    fn a_path_names_an_app_version_or_a_preview_and_a_file_below_ui() {
        assert_eq!(
            parse("/app/expenses/3/index.html"),
            app("expenses", 3, "index.html")
        );
        assert_eq!(parse("/app/expenses/3/"), app("expenses", 3, "index.html"));
        assert_eq!(parse("/app/expenses/3"), app("expenses", 3, "index.html"));
        assert_eq!(
            parse("/app/expenses/3/img/my%20logo.png"),
            app("expenses", 3, "img/my logo.png")
        );
        assert_eq!(
            parse("/preview/01J0PREV/app.js"),
            Some(Target::Preview {
                id: "01J0PREV".into(),
                path: "app.js".into()
            })
        );
        for refused in [
            "/",
            "/app/expenses",
            "/app/expenses/x/index.html",
            "/app/expenses/3/../logic.js",
            "/app/expenses/3/%2e%2e/logic.js",
            "/app/expenses/3/a//b.js",
            "/app/expenses/3/a%5cb.js",
            "/app/expenses/3/%zz",
            "/preview/",
            "/other/x",
        ] {
            assert_eq!(parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_page_may_load_only_its_own_files_and_reach_nothing() {
        let target = app("expenses", 3, "index.html").unwrap();
        let policy = csp(&target.base());
        assert!(policy.starts_with("default-src 'none'; "), "{policy}");
        for part in [
            "script-src 'unsafe-inline' http://dots-app.localhost/app/expenses/3/;",
            "style-src 'unsafe-inline' http://dots-app.localhost/app/expenses/3/;",
            "img-src data: http://dots-app.localhost/app/expenses/3/;",
            "connect-src 'none';",
            "frame-src 'none';",
            "form-action 'none';",
            "base-uri 'none'",
        ] {
            assert!(policy.contains(part), "{part} in {policy}");
        }
        assert!(!policy.contains("'self'"), "{policy}");
        assert!(!policy.contains("ipc:"), "{policy}");
    }

    #[test]
    fn index_html_gets_the_client_after_its_doctype() {
        let page = String::from_utf8(with_client(b"<!DOCTYPE html>\n<title>E</title>")).unwrap();
        assert!(
            page.starts_with("<!DOCTYPE html><script>"),
            "{}",
            &page[..40]
        );
        assert!(page.ends_with("</script>\n<title>E</title>"));
        let bare = String::from_utf8(with_client(b"<div id=app></div>")).unwrap();
        assert!(
            bare.starts_with("<script>// Dots' client"),
            "{}",
            &bare[..40]
        );
        assert!(bare.ends_with("</script><div id=app></div>"));
    }

    #[test]
    fn a_file_is_served_with_its_type_and_the_pages_csp() {
        let target = app("expenses", 3, "index.html").unwrap();
        let page = response(&target, Ok(b"<!doctype html><p>hi".to_vec()));
        assert_eq!(page.status(), StatusCode::OK);
        let headers = page.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(
            headers[header::CONTENT_SECURITY_POLICY],
            csp(&target.base()).as_str()
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(String::from_utf8_lossy(page.body()).contains("window, \"dots\""));

        let script = app("expenses", 3, "app.js").unwrap();
        let js = response(&script, Ok(b"dots.call('x')".to_vec()));
        assert_eq!(
            js.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            js.body(),
            b"dots.call('x')",
            "only index.html gets the client"
        );

        let missing = response(&script, Err(Error::NotFound("ui/app.js".into())));
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(content_type("ui/font.WOFF2"), "font/woff2");
        assert_eq!(content_type("data.bin"), "application/octet-stream");
    }

    #[tokio::test]
    async fn only_get_of_a_known_path_is_answered() {
        let post = Request::builder()
            .method(Method::POST)
            .uri("http://dots-app.localhost/app/expenses/1/index.html")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            respond(None, &post).await.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let other = Request::builder()
            .uri("http://dots-app.localhost/secrets")
            .body(Vec::new())
            .unwrap();
        assert_eq!(respond(None, &other).await.status(), StatusCode::NOT_FOUND);
        let early = Request::builder()
            .uri("http://dots-app.localhost/app/expenses/1/index.html")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            respond(None, &early).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

use dots_core::apps::propose::{AppProposal, Decision};
```

**with:**

```rust

use dots_core::apps::kept::KeptData;
use dots_core::apps::propose::{AppProposal, Decision};
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// The person's answer on the card: add (or update), ask for changes, or discard.
```

**with:**

```rust

/// The Code tab's diff: against the active version for a change, every file for a new app.
#[tauri::command]
pub async fn app_proposal_diff(state: State<'_, AppState>, id: String) -> Res<WorktreeDiff> {
    Ok(state.rt()?.apps().proposal_diff(&id).await?)
}

/// Opens the card's preview: the proposal's code on a database built from its migrations and
/// `sample.sql` (app library spec section 6.2). Returns the preview's id, which its frame's
/// address and calls name.
#[tauri::command]
pub async fn app_preview_open(state: State<'_, AppState>, id: String) -> Res<String> {
    Ok(state.rt()?.apps().preview_open(&id).await?)
}

/// A call from the preview's frame, as the person; no dot can be started from it.
#[tauri::command]
pub async fn app_preview_call(
    state: State<'_, AppState>,
    preview: String,
    tool: String,
    input: serde_json::Value,
) -> Res<serde_json::Value> {
    Ok(state
        .rt()?
        .apps()
        .preview_call(&preview, &tool, input)
        .await?)
}

/// Closes the preview: its changes are gone.
#[tauri::command]
pub async fn app_preview_close(state: State<'_, AppState>, preview: String) -> Res<()> {
    state.rt()?.apps().preview_close(&preview);
    Ok(())
}

/// The data removed apps kept, newest first (app library spec section 10).
#[tauri::command]
pub async fn apps_kept_list(state: State<'_, AppState>) -> Res<Vec<KeptData>> {
    Ok(state.rt()?.apps().kept().await?)
}

/// Deletes data a removed app kept.
#[tauri::command]
pub async fn app_kept_delete(state: State<'_, AppState>, id: String) -> Res<()> {
    Ok(state.rt()?.apps().delete_kept(&id).await?)
}

/// The person's answer on the card: add (or update), ask for changes, or discard.
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            error: None,
        },
```

**with:**

```rust
            error: None,
            tools: vec![
                ToolSummary {
                    name: "get_task".into(),
                    description: "A task with its spec, subtasks and recent activity".into(),
                    writes: false,
                },
                ToolSummary {
                    name: "update_task".into(),
                    description: "Change a task".into(),
                    writes: true,
                },
            ],
            permissions: vec![Permission::StartDots],
        },
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            error: Some("invalid input: app.json: EOF while parsing an object".into()),
        },
```

**with:**

```rust
            error: Some("invalid input: app.json: EOF while parsing an object".into()),
            tools: vec![],
            permissions: vec![],
        },
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    ]);
    samples["AppFile"] = to([
```

**with:**

```rust
    ]);
    samples["KeptData"] = to([dots_core::apps::kept::KeptData {
        id: "01J0APP0".into(),
        name: "expenses".into(),
        title: "Expenses".into(),
        removed_at: TS.into(),
        bytes: 24_576,
    }]);
    samples["AppFile"] = to([
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
pub mod autostart;
```

**with:**

```rust
pub mod app_frame;
pub mod autostart;
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    app_proposal_files,
    app_proposal_decide,
    watchers_list,
```

**with:**

```rust
    app_proposal_files,
    app_proposal_diff,
    app_proposal_decide,
    app_preview_open,
    app_preview_call,
    app_preview_close,
    apps_kept_list,
    app_kept_delete,
    watchers_list,
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
```

**with:**

```rust
        .plugin(tauri_plugin_updater::Builder::new().build())
        // The pages of the apps' screens (app library spec section 5).
        .register_asynchronous_uri_scheme_protocol(app_frame::SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let rt = state::runtime_of(app.try_state::<AppState>().as_deref());
                responder.respond(app_frame::respond(rt.as_deref(), &request).await);
            });
        })
        .setup(move |app| {
```

**In `apps/dots-app/tauri.conf.json`, replace:**

```json
    "security": {
      "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost"
    }
```

**with:**

```json
    "security": {
      "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost; frame-src http://dots-app.localhost"
    }
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
    pub writes: bool,
}
```

**with:**

```rust
    pub writes: bool,
}

/// The tools of `manifest` as the card and the Apps screen list them.
pub fn tool_summaries(manifest: &super::manifest::Manifest) -> Vec<ToolSummary> {
    manifest
        .tools
        .iter()
        .map(|t| ToolSummary {
            name: t.name.clone(),
            description: t.description.clone(),
            writes: t.writes,
        })
        .collect()
}
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
            description: src.manifest.description.clone(),
            tools: src
                .manifest
                .tools
                .iter()
                .map(|t| ToolSummary {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    writes: t.writes,
                })
                .collect(),
            permissions: src.manifest.permissions.clone(),
```

**with:**

```rust
            description: src.manifest.description.clone(),
            tools: tool_summaries(&src.manifest),
            permissions: src.manifest.permissions.clone(),
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
    pub error: Option<String>,
}
```

**with:**

```rust
    pub error: Option<String>,
    /// The tools its screen may call (spec section 5): those its active version lists; none
    /// while its code cannot be read.
    pub tools: Vec<super::propose::ToolSummary>,
    /// What its active version asks to reach outside its data ("Can start dots").
    pub permissions: Vec<super::manifest::Permission>,
}
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
        for (item, config) in self.list().await? {
            let (users, error) = match self.load(&item, &config).await {
                Ok(src) => (self.users(&item, &src.manifest).await?, None),
                Err(e) => (self.store.dots_with_app(&item).await?, Some(e.to_string())),
            };
```

**with:**

```rust
        for (item, config) in self.list().await? {
            let (users, error, tools, permissions) = match self.load(&item, &config).await {
                Ok(src) => (
                    self.users(&item, &src.manifest).await?,
                    None,
                    super::propose::tool_summaries(&src.manifest),
                    src.manifest.permissions.clone(),
                ),
                Err(e) => (
                    self.store.dots_with_app(&item).await?,
                    Some(e.to_string()),
                    Vec::new(),
                    Vec::new(),
                ),
            };
```

**In `crates/dots-core/src/apps/screen.rs`, replace:**

```rust
            let dots = users.into_iter().map(|d| d.spec.name).collect();
            rows.push(AppRow { item, dots, error });
        }
```

**with:**

```rust
            let dots = users.into_iter().map(|d| d.spec.name).collect();
            rows.push(AppRow {
                item,
                dots,
                error,
                tools,
                permissions,
            });
        }
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  appProposalFiles: (id: string) => call(T.AppFile.array(), "app_proposal_files", { id }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already
```

**with:**

```ts
  appProposalFiles: (id: string) => call(T.AppFile.array(), "app_proposal_files", { id }),
  /// The Code tab's diff: against the active version for a change, every file for a new app.
  appProposalDiff: (id: string) => call(T.WorktreeDiff, "app_proposal_diff", { id }),
  /// Opens the card's preview on a throwaway database; answers its id. Rejects with kind
  /// `conflict` once the proposal is decided.
  appPreviewOpen: (id: string) => call(z.string(), "app_preview_open", { id }),
  /// A call from the preview's frame, as the person. Rejects with kind `invalid` and the rule
  /// the call broke, `not_found` once the preview is closed.
  appPreviewCall: (preview: string, tool: string, input: unknown) =>
    call(z.unknown(), "app_preview_call", { preview, tool, input }),
  appPreviewClose: (preview: string) => call(nothing, "app_preview_close", { preview }),
  /// The data removed apps kept, newest first.
  appsKeptList: () => call(T.KeptData.array(), "apps_kept_list"),
  appKeptDelete: (id: string) => call(nothing, "app_kept_delete", { id }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already
```

**In `ui/src/api/types.ts`, replace:**

```ts
});
// What a review card's summary shows.
```

**with:**

```ts
});
// A tool of an app, as a review card and the Apps screen list it.
export const ToolSummary = z.strictObject({
  name: z.string(),
  description: z.string(),
  writes: z.boolean(),
});
// What a review card's summary shows.
```

**In `ui/src/api/types.ts`, replace:**

```ts
  description: z.string(),
  tools: z.array(z.strictObject({ name: z.string(), description: z.string(), writes: z.boolean() })),
  // Empty for "Nothing".
```

**with:**

```ts
  description: z.string(),
  tools: z.array(ToolSummary),
  // Empty for "Nothing".
```

**In `ui/src/api/types.ts`, replace:**

```ts
  // Why its code cannot be read: it is left out of runs until it can.
  error: nullable(z.string()),
});
export type AppRow = z.infer<typeof AppRow>;
export type AppFile = z.infer<typeof AppFile>;
export type AppCall = z.infer<typeof AppCall>;
```

**with:**

```ts
  // Why its code cannot be read: it is left out of runs until it can.
  error: nullable(z.string()),
  // The tools its screen may call; none while its code cannot be read.
  tools: z.array(ToolSummary),
  // What it asks to reach outside its data: "start_dots" for "Can start dots".
  permissions: z.array(Permission),
});
export type AppRow = z.infer<typeof AppRow>;
// Data a removed app kept (app library spec section 10).
export const KeptData = z.strictObject({
  id: z.string(),
  name: z.string(),
  title: z.string(),
  removed_at: z.string(),
  bytes: z.number(),
});
export type KeptData = z.infer<typeof KeptData>;
export type AppFile = z.infer<typeof AppFile>;
export type AppCall = z.infer<typeof AppCall>;
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_screen_test`, then `UPDATE_CONTRACT=1 cargo test -j 2 -p dots-app`, then in `ui`: `bun run typecheck` and `DEBUG_PRINT_LIMIT=0 bun x vitest run src/api`.
Expected: PASS: 5 tests; `dots-app` 129 passed (the five of `app_frame`); the fixture gains `KeptData` and the rows' `tools` and `permissions`; `tsc` clean; the UI's API tests pass.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: `dots-core` 1643 passed, the UI 751 in 72 files (the contract test checks the new `KeptData` sample); no new warnings.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/app_client.js apps/dots-app/src/app_frame.rs apps/dots-app/src/commands.rs apps/dots-app/src/contract.rs apps/dots-app/src/lib.rs apps/dots-app/tauri.conf.json crates/dots-core/src/apps/propose.rs crates/dots-core/src/apps/screen.rs crates/dots-core/tests/apps_screen_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts
git commit -m "feat(app): the dots-app protocol serves an app's page with its client, and the preview's commands"
```

### Task 9: The frame and the bridge

Spec §5 "Screen", the host's side: the frame `sandbox="allow-scripts"`, one port on the page's first load, every message checked (a listed tool; the call is the person's), the theme sent, changes told. A page that loads again is cut off. The client of Task 8 is tested here, run in a context of its own.

**Files:**

- Create: `ui/src/lib/app-bridge.ts`, `ui/src/components/AppFrame.tsx`, `ui/src/components/AppFrame.test.tsx`, `ui/src/lib/app-client.test.ts`

**Interfaces:**

- Consumes: Task 8's client and protocol; `lib/tokens.ts` `THEMES`; `lib/theme.ts` `useResolvedTheme`.
- Produces: `app-bridge.ts`: `APP_ORIGIN`, `SANDBOX = "allow-scripts"`, `appSrc(name, version)`, `previewSrc(id)`, `connectFrame(frame: Window, { tools, call }) -> Bridge { theme(mode), changed(), close() }`. `AppFrame({ src, title, tools, call, changes? })`.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/components/AppFrame.test.tsx`:**

```tsx
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AppFrame } from "@/components/AppFrame";
import { appSrc } from "@/lib/app-bridge";

/// Loads the frame and returns the port its page was handed, with what arrives on it.
function load(frame: HTMLIFrameElement) {
  const post = vi.spyOn(frame.contentWindow!, "postMessage");
  fireEvent.load(frame);
  expect(post).toHaveBeenCalledTimes(1);
  const [message, origin, transfer] = post.mock.calls[0] as unknown as [
    unknown,
    string,
    MessagePort[],
  ];
  expect(message).toEqual({ type: "dots:connect" });
  // An opaque origin has no name to send to.
  expect(origin).toBe("*");
  const port = transfer[0]!;
  const got: unknown[] = [];
  port.onmessage = (e: MessageEvent) => got.push(e.data);
  return { port, got };
}

describe("an app's screen in its frame", () => {
  it("frames the page with scripts only and sends the theme on its first load", async () => {
    render(<AppFrame src={appSrc("expenses", 2)} title="Expenses" tools={[]} call={vi.fn()} />);
    const frame = screen.getByTitle<HTMLIFrameElement>("Expenses");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).toHaveAttribute("src", "http://dots-app.localhost/app/expenses/2/index.html");
    const { got } = load(frame);
    await waitFor(() => expect(got).toHaveLength(1));
    expect(got[0]).toMatchObject({ type: "theme", mode: "light" });
    expect((got[0] as { tokens: Record<string, string> }).tokens.primary).toBe("#0c7765");
  });

  it("calls a listed tool as the page asks and refuses one it does not list", async () => {
    const call = vi.fn(async (tool: string) => (tool === "list_expenses" ? [{ item: "Ink" }] : 0));
    render(
      <AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={["list_expenses"]} call={call} />,
    );
    const { port, got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    port.postMessage({ type: "call", id: 1, tool: "list_expenses", input: { month: "2026-10" } });
    port.postMessage({ type: "call", id: 2, tool: "drop_all", input: {} });
    port.postMessage({ type: "call", id: "3", tool: "list_expenses" });
    port.postMessage({ type: "call", id: 4, tool: "list_expenses", input: {}, actor: "dot" });
    await waitFor(() => expect(got).toHaveLength(3));
    expect(call).toHaveBeenCalledTimes(1);
    expect(call).toHaveBeenCalledWith("list_expenses", { month: "2026-10" });
    expect(got).toContainEqual({ type: "result", id: 1, ok: true, result: [{ item: "Ink" }] });
    expect(got).toContainEqual({
      type: "result",
      id: 2,
      ok: false,
      error: "this app has no tool drop_all",
    });
  });

  it("answers a failed call with its error", async () => {
    const call = vi.fn(async () => {
      throw new Error("amount: must be at least 0");
    });
    render(<AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={["add"]} call={call} />);
    const { port, got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    port.postMessage({ type: "call", id: 7, tool: "add", input: { amount: -1 } });
    await waitFor(() =>
      expect(got).toContainEqual({
        type: "result",
        id: 7,
        ok: false,
        error: "amount: must be at least 0",
      }),
    );
  });

  it("tells the page of each change to the app's data", async () => {
    const { rerender } = render(
      <AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} changes={0} />,
    );
    const { got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    rerender(
      <AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} changes={1} />,
    );
    await waitFor(() => expect(got).toContainEqual({ type: "changed" }));
  });

  it("cuts off a page that navigated away, and loads it afresh when asked", async () => {
    const user = userEvent.setup();
    render(<AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} />);
    const first = screen.getByTitle<HTMLIFrameElement>("Expenses");
    load(first);
    act(() => {
      fireEvent.load(first);
    });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "The app's screen left its page, so Dots stopped answering it.",
    );
    expect(screen.queryByTitle("Expenses")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Load the app again" }));
    const again = screen.getByTitle<HTMLIFrameElement>("Expenses");
    load(again);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
```

**Create `ui/src/lib/app-client.test.ts`:**

```ts
// The client Dots puts into an app's page (apps/dots-app/src/app_client.js), run in a context of
// its own with a stand-in for the page's window.
import { runInNewContext } from "node:vm";

import { describe, expect, it } from "vitest";

import client from "../../../apps/dots-app/src/app_client.js?raw";

type Dots = {
  call: (tool: string, input?: unknown) => Promise<unknown>;
  onChange: (cb: () => void) => () => void;
  onTheme: (cb: (mode: string) => void) => () => void;
};

/// A page with the client loaded: what it set on `window`, a way to deliver a `message` event,
/// the window that framed it, and its root element's style.
function page() {
  const listeners: ((event: unknown) => void)[] = [];
  const style = new Map<string, string>();
  const root = {
    style: { setProperty: (name: string, value: string) => style.set(name, value), colorScheme: "" },
    dataset: {} as Record<string, string>,
  };
  const parent = {};
  const window: Record<string, unknown> = {
    parent,
    addEventListener: (type: string, listener: (event: unknown) => void) => {
      if (type === "message") listeners.push(listener);
    },
  };
  runInNewContext(client, { window, document: { documentElement: root }, console });
  const deliver = (event: unknown) => {
    for (const listener of listeners) listener(event);
  };
  return { window, dots: window.dots as Dots, deliver, parent, root, style };
}

/// The next message that arrives on `port`.
function next(port: MessagePort): Promise<unknown> {
  return new Promise((resolve) => {
    port.onmessage = (e: MessageEvent) => resolve(e.data);
  });
}

function connect(p: ReturnType<typeof page>) {
  const { port1, port2 } = new MessageChannel();
  p.deliver({ source: p.parent, data: { type: "dots:connect" }, ports: [port2] });
  return port1;
}

describe("the client in an app's page", () => {
  it("sends calls through the port once it has one, and answers with the result", async () => {
    const p = page();
    const listed = p.dots.call("list_expenses", { month: "2026-10" });
    const port = connect(p);
    const asked = await next(port);
    expect(asked).toEqual({
      type: "call",
      id: 1,
      tool: "list_expenses",
      input: { month: "2026-10" },
    });
    port.postMessage({ type: "result", id: 1, ok: true, result: [{ item: "Ink" }] });
    await expect(listed).resolves.toEqual([{ item: "Ink" }]);

    const refused = p.dots.call("drop_all");
    expect(await next(port)).toEqual({ type: "call", id: 2, tool: "drop_all", input: {} });
    port.postMessage({ type: "result", id: 2, ok: false, error: "this app has no tool drop_all" });
    await expect(refused).rejects.toThrow("this app has no tool drop_all");
    port.close();
  });

  it("takes one port, and only from the window that framed it", async () => {
    const p = page();
    const stranger = new MessageChannel();
    p.deliver({ source: {}, data: { type: "dots:connect" }, ports: [stranger.port2] });
    const pending = p.dots.call("list_expenses");
    const port = connect(p);
    expect(await next(port)).toMatchObject({ type: "call", id: 1 });
    const second = new MessageChannel();
    p.deliver({ source: p.parent, data: { type: "dots:connect" }, ports: [second.port2] });
    second.port1.postMessage({ type: "result", id: 1, ok: true, result: "forged" });
    port.postMessage({ type: "result", id: 1, ok: true, result: "real" });
    await expect(pending).resolves.toBe("real");
    for (const q of [port, stranger.port1, second.port1]) q.close();
  });

  it("puts the theme's colours on the root and tells the page of changes", async () => {
    const p = page();
    const port = connect(p);
    const modes: string[] = [];
    p.dots.onTheme((mode) => modes.push(mode));
    let changes = 0;
    const stop = p.dots.onChange(() => (changes += 1));
    const heard = new Promise<void>((resolve) => p.dots.onChange(() => resolve()));
    port.postMessage({ type: "theme", mode: "dark", tokens: { background: "#0b1514" } });
    port.postMessage({ type: "changed" });
    await heard;
    expect(p.style.get("--dots-background")).toBe("#0b1514");
    expect(p.root.dataset.theme).toBe("dark");
    expect(p.root.style.colorScheme).toBe("dark");
    expect(modes).toEqual(["dark"]);
    expect(changes).toBe(1);
    stop();
    port.close();
  });

  it("cannot be replaced by the page", () => {
    const p = page();
    expect(() => runInNewContext('"use strict"; window.dots = {};', { window: p.window })).toThrow();
    expect(p.window.dots).toBe(p.dots);
    expect(Object.isFrozen(p.dots)).toBe(true);
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/components/AppFrame.test.tsx src/lib/app-client.test.ts` in `ui`.
Expected: `AppFrame.test.tsx` fails to load (no `@/components/AppFrame`); `app-client.test.ts` passes (the client exists since Task 8).

- [ ] **Step 3: Add the bridge and the frame**

**Create `ui/src/components/AppFrame.tsx`:**

```tsx
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { connectFrame, SANDBOX, type Bridge } from "@/lib/app-bridge";
import { useResolvedTheme } from "@/lib/theme";

/// An app's screen in its sandboxed frame (app library spec section 5), filling its box. The
/// page gets its bridge on its first load. A page that loads again has navigated away from what
/// Dots served: it is cut off, and the person can load it afresh. `changes` counts the changes
/// to the app's data; each new count tells the page (`dots.onChange`).
export function AppFrame(props: {
  src: string;
  title: string;
  tools: readonly string[];
  call: (tool: string, input: unknown) => Promise<unknown>;
  changes?: number;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const bridge = useRef<Bridge | null>(null);
  const loads = useRef(0);
  const [left, setLeft] = useState(false);
  const [fresh, setFresh] = useState(0);
  const theme = useResolvedTheme();
  // What the bridge reads at each call, without a new bridge on each render.
  const latest = useRef({ tools: props.tools, call: props.call, theme });
  latest.current = { tools: props.tools, call: props.call, theme };
  useEffect(() => bridge.current?.theme(theme), [theme]);
  useEffect(() => {
    if ((props.changes ?? 0) > 0) bridge.current?.changed();
  }, [props.changes]);
  // Another page (a new version, another preview) is a new frame with its own first load.
  useEffect(() => {
    loads.current = 0;
  }, [props.src]);
  useEffect(
    () => () => {
      bridge.current?.close();
      bridge.current = null;
    },
    [],
  );
  const onLoad = () => {
    loads.current += 1;
    bridge.current?.close();
    bridge.current = null;
    const page = frame.current?.contentWindow;
    if (loads.current > 1 || !page) {
      setLeft(true);
      return;
    }
    bridge.current = connectFrame(page, {
      get tools() {
        return latest.current.tools;
      },
      call: (tool, input) => latest.current.call(tool, input),
    });
    bridge.current.theme(latest.current.theme);
  };
  if (left) {
    return (
      <div
        role="alert"
        className="m-4 flex flex-col items-start gap-2 rounded-md border border-needs-border bg-needs-surface p-3"
      >
        <p>The app's screen left its page, so Dots stopped answering it.</p>
        <Button
          size="sm"
          onClick={() => {
            loads.current = 0;
            setLeft(false);
            setFresh((n) => n + 1);
          }}
        >
          Load the app again
        </Button>
      </div>
    );
  }
  return (
    <iframe
      key={`${fresh}:${props.src}`}
      ref={frame}
      title={props.title}
      src={props.src}
      sandbox={SANDBOX}
      onLoad={onLoad}
      className="min-h-0 w-full flex-1 border-0 bg-background"
    />
  );
}
```

**Create `ui/src/lib/app-bridge.ts`:**

```ts
// The host side of an app's screen (app library spec section 5): the address its frame loads,
// and the bridge through which the page's `dots.call` reaches Dots. The page runs sandboxed
// (`allow-scripts` only, so in an opaque origin) and gets one MessagePort. Every message on it
// is checked here before anything is called: a call names a tool the app lists, and Dots calls
// it as the person (the page cannot say who calls). The page's client is
// apps/dots-app/src/app_client.js.
import { z } from "zod";

import { errorMessage } from "@/lib/notify";
import { THEMES, type ThemeName } from "@/lib/tokens";

/// Where the `dots-app` protocol serves the apps' pages (`ORIGIN` in
/// apps/dots-app/src/app_frame.rs); Dots' own CSP lets it be framed.
export const APP_ORIGIN = "http://dots-app.localhost";
/// The frame's `sandbox`: scripts, and nothing else (no same origin, forms or popups).
export const SANDBOX = "allow-scripts";

/// The page of the app `name` at `version`.
export const appSrc = (name: string, version: number) =>
  `${APP_ORIGIN}/app/${encodeURIComponent(name)}/${version}/index.html`;
/// The page of a review card's preview.
export const previewSrc = (id: string) =>
  `${APP_ORIGIN}/preview/${encodeURIComponent(id)}/index.html`;

/// A call the page asks for. Anything else on the port is ignored.
const Call = z.strictObject({
  type: z.literal("call"),
  id: z.number().int().positive(),
  tool: z.string(),
  input: z.unknown(),
});

export type Bridge = {
  /// Sends the theme's colours, for `--dots-<token>` in the page.
  theme: (mode: ThemeName) => void;
  /// Tells the page that the app's data changed (`dots.onChange`).
  changed: () => void;
  close: () => void;
};

/// Hands the framed page `frame` its port and answers its calls: `tools` are those it may call
/// (read at each call), `call` performs one as the person.
export function connectFrame(
  frame: Window,
  options: {
    tools: readonly string[];
    call: (tool: string, input: unknown) => Promise<unknown>;
  },
): Bridge {
  const channel = new MessageChannel();
  const port = channel.port1;
  let open = true;
  const send = (message: unknown) => {
    if (open) port.postMessage(message);
  };
  port.onmessage = (event: MessageEvent) => {
    const parsed = Call.safeParse(event.data);
    if (!parsed.success) return;
    const { id, tool, input } = parsed.data;
    if (!options.tools.includes(tool)) {
      send({ type: "result", id, ok: false, error: `this app has no tool ${tool}` });
      return;
    }
    options.call(tool, input ?? {}).then(
      (result) => send({ type: "result", id, ok: true, result }),
      (e: unknown) => send({ type: "result", id, ok: false, error: errorMessage(e) }),
    );
  };
  frame.postMessage({ type: "dots:connect" }, "*", [channel.port2]);
  return {
    theme: (mode) => send({ type: "theme", mode, tokens: THEMES[mode] }),
    changed: () => send({ type: "changed" }),
    close: () => {
      open = false;
      port.close();
    },
  };
}
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/components/AppFrame.test.tsx src/lib/app-client.test.ts`, `bun run typecheck` in `ui`.
Expected: PASS: 9 tests in 2 files; `tsc` clean.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: the UI 760 in 74 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/AppFrame.test.tsx ui/src/components/AppFrame.tsx ui/src/lib/app-bridge.ts ui/src/lib/app-client.test.ts
git commit -m "feat(ui): an app's screen in its sandboxed frame, with the bridge"
```

### Task 10: The Apps screen, an app's own screen, and runs named by their app

Spec §10: the Apps entry on the rail between Inbox and Library; `/apps` with each app's title, description, version, "Every dot", "Can start dots" (for an app that asks for it), its users, its error, "Roll back" and "Remove" (with the second confirmation for the data), then "Kept data"; `/apps/:name` with the frame filling the content area, told of each data change. Plan A's deferred finding: "Started by <title>" and the app's message in the transcript.

**Files:**

- Create: `ui/src/screens/apps/{AppsScreen,AppScreen}.tsx`, `ui/src/screens/apps/AppsScreen.test.tsx`, `ui/src/components/Transcript.test.ts`
- Modify: `ui/src/app/{Layout,routes}.tsx`, `ui/src/components/Transcript.tsx`, `ui/src/lib/{format,home-topics,query,runtime-events}.ts`
- Test: `ui/src/lib/format.test.ts`, `ui/src/screens/library/LibraryScreen.test.tsx` (the rail's order)

**Interfaces:**

- Consumes: Tasks 8 and 9.
- Produces: routes `apps`, `apps/:name`; `keys.{apps, appsKept}`; `appDataChanges` (an `EventTarget` firing the app's name on `app_data_changed`); `formatBytes(n)`; `home-topics.{Started, appTitle(run)}`, `triggerLabel` naming the app; `Transcript.runTrigger(run)`; `AppsScreen.appConfig(row)`.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/components/Transcript.test.ts`:**

```ts
import { describe, expect, it } from "vitest";

import { runTrigger } from "@/components/Transcript";
import { triggerLabel } from "@/lib/home-topics";
import { sample } from "@/test/fixtures";

describe("what started a run", () => {
  it("names the app that started it, by its title", () => {
    const run = {
      ...sample.run,
      trigger: "app" as const,
      payload: { message: "Task t1", app: "projects", app_title: "Projects", tag: "t1" },
    };
    expect(runTrigger(run)).toBe("Started by Projects");
    expect(triggerLabel(run)).toBe("Projects");
  });

  it("says an app started it when the run does not name one", () => {
    const run = { ...sample.run, trigger: "app" as const, payload: null };
    expect(runTrigger(run)).toBe("Started by an app");
    expect(triggerLabel(run)).toBe("App");
    expect(runTrigger({ ...sample.run, trigger: "manual" })).toBe("Manual run");
  });
});
```

**In `ui/src/lib/format.test.ts`, replace:**

```ts

import { duration, localDateTime, localTime, relativeTime } from "./format";

```

**with:**

```ts

import { duration, formatBytes, localDateTime, localTime, relativeTime } from "./format";

```

**In `ui/src/lib/format.test.ts`, replace:**

```ts
const at = (iso: string) => Date.parse(iso);

```

**with:**

```ts
const at = (iso: string) => Date.parse(iso);

describe("sizes", () => {
  it("reads in bytes, kilobytes and megabytes", () => {
    expect(formatBytes(240)).toBe("240 B");
    expect(formatBytes(2_458)).toBe("2.4 KB");
    expect(formatBytes(24_576)).toBe("24 KB");
    expect(formatBytes(1_572_864)).toBe("1.5 MB");
    expect(formatBytes(512 * 1024 * 1024)).toBe("512 MB");
  });
});

```

**Create `ui/src/screens/apps/AppsScreen.test.tsx`:**

```tsx
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AppRow, KeptData } from "@/api/types";
import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";
import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";

const [projects, broken] = T.AppRow.array().parse(contract.AppRow) as [AppRow, AppRow];
const [kept] = T.KeptData.array().parse(contract.KeptData) as [KeptData];

/// The Expenses app as a row: used by one dot, version 3, with a version to roll back to.
function expenses(over: Partial<{ every_dot: boolean; previous: number | null; dots: string[] }> = {}): AppRow {
  if (projects.item.kind !== "app") throw new Error("an app row");
  return {
    ...projects,
    item: {
      ...projects.item,
      id: "01J0APP9",
      name: "expenses",
      config: {
        ...projects.item.config,
        title: "Expenses",
        description: "Logs expenses and totals them by month",
        version: 3,
        previous_version: over.previous === undefined ? 2 : over.previous,
        every_dot: over.every_dot ?? false,
      },
    },
    dots: over.dots ?? ["builder"],
    permissions: [],
    tools: [{ name: "list_expenses", description: "List", writes: false }],
  };
}

describe("the Apps screen", () => {
  it("sits on the rail between Inbox and Library", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => []);
    renderApp("/apps");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const names = within(nav)
      .getAllByRole("link")
      .map((link) => link.getAttribute("aria-label"));
    expect(names.indexOf("Apps")).toBe(names.indexOf("Inbox") + 1);
    expect(names.indexOf("Library")).toBe(names.indexOf("Apps") + 1);
    expect(await screen.findByText(/No apps yet/)).toBeVisible();
  });

  it("lists each app with its version, its users, its error and its switches", async () => {
    backend.on("apps_list", () => [projects, broken, expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_set_every_dot", () => projects.item);
    backend.on("app_set_can_start_dots", () => projects.item);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    expect(row).toHaveTextContent("version 3");
    expect(row).toHaveTextContent("Used by builder");
    expect(within(row).getByRole("link", { name: "Expenses" })).toHaveAttribute(
      "href",
      "/apps/expenses",
    );
    // Expenses does not ask to start dots: no switch for it.
    expect(within(row).queryByRole("switch", { name: "Can start dots" })).toBeNull();
    await user.click(within(row).getByRole("switch", { name: "Every dot" }));
    await waitFor(() =>
      expect(backend.callsTo("app_set_every_dot")).toEqual([{ name: "expenses", on: true }]),
    );
    // Projects asks to start dots, and the sample has it switched on: the click switches it off.
    const asking = screen.getAllByRole("listitem")[0]!;
    await user.click(within(asking).getByRole("switch", { name: "Can start dots" }));
    await waitFor(() =>
      expect(backend.callsTo("app_set_can_start_dots")).toEqual([
        { name: projects.item.name, on: false },
      ]),
    );
    const broke = screen.getAllByRole("listitem")[1]!;
    expect(broke).toHaveTextContent("EOF while parsing an object");
  });

  it("rolls back after asking", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_roll_back", () => expenses().item);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    await user.click(within(row).getByRole("button", { name: "Roll back" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Roll back Expenses?" });
    expect(dialog).toHaveTextContent("Version 2 becomes the active one again.");
    await user.click(within(dialog).getByRole("button", { name: "Roll back" }));
    await waitFor(() => expect(backend.callsTo("app_roll_back")).toEqual([{ name: "expenses" }]));
  });

  it("offers no roll back once an update changed the data's schema", async () => {
    backend.on("apps_list", () => [expenses({ previous: null })]);
    backend.on("apps_kept_list", () => []);
    renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    expect(within(row).queryByRole("button", { name: "Roll back" })).toBeNull();
  });

  it("removes an app after asking twice, keeping its data unless told", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_remove", () => null);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const first = await screen.findByRole("alertdialog", { name: "Remove Expenses?" });
    expect(first).toHaveTextContent("Used by builder. Removing it takes it out of them.");
    await user.click(within(first).getByRole("button", { name: "Remove app" }));
    const second = await screen.findByRole("alertdialog", { name: "Delete Expenses's data too?" });
    await user.click(within(second).getByRole("button", { name: "Keep the data" }));
    await waitFor(() =>
      expect(backend.callsTo("app_remove")).toEqual([
        { name: "expenses", confirm: true, deleteData: false },
      ]),
    );

    await user.click(within(row).getByRole("button", { name: "Remove" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Remove app" }),
    );
    await user.click(
      within(await screen.findByRole("alertdialog", { name: "Delete Expenses's data too?" }))
        .getByRole("button", { name: "Delete the data" }),
    );
    await waitFor(() => expect(backend.callsTo("app_remove")).toHaveLength(2));
    expect(backend.callsTo("app_remove")[1]).toEqual({
      name: "expenses",
      confirm: true,
      deleteData: true,
    });
  });

  it("lists the data removed apps kept, and deletes it after asking", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => [kept]);
    backend.on("app_kept_delete", () => null);
    const { user } = renderApp("/apps");
    const section = await screen.findByRole("region", { name: "Kept data" });
    const item = within(section).getByRole("listitem", { name: "Expenses, kept data" });
    expect(item).toHaveTextContent("24 KB");
    await user.click(within(item).getByRole("button", { name: "Delete data" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete data" }),
    );
    await waitFor(() => expect(backend.callsTo("app_kept_delete")).toEqual([{ id: kept.id }]));
  });
});

describe("an app's own screen", () => {
  it("fills the content area with the app's frame and tells it of changes", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("app_call", () => [{ item: "Ink" }]);
    renderApp("/apps/expenses");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses");
    expect(frame).toHaveAttribute("src", "http://dots-app.localhost/app/expenses/3/index.html");
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    const got: unknown[] = [];
    port.onmessage = (e: MessageEvent) => got.push(e.data);
    port.postMessage({ type: "call", id: 1, tool: "list_expenses", input: { month: "2026-10" } });
    await waitFor(() =>
      expect(got).toContainEqual({ type: "result", id: 1, ok: true, result: [{ item: "Ink" }] }),
    );
    expect(backend.callsTo("app_call")).toEqual([
      { name: "expenses", tool: "list_expenses", input: { month: "2026-10" } },
    ]);
    act(() => backend.emit("runtime", { type: "app_data_changed", app: "expenses" }));
    await waitFor(() => expect(got).toContainEqual({ type: "changed" }));
  });

  it("says so for an app it does not have", async () => {
    backend.on("apps_list", () => []);
    renderApp("/apps/nothing");
    expect(await screen.findByText("There is no app nothing.")).toBeVisible();
  });
});
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
describe("the Library screen", () => {
  it("has a place in the rail between Inbox and Settings", async () => {
    renderApp("/library");
```

**with:**

```tsx
describe("the Library screen", () => {
  it("has a place in the rail between Apps and Settings", async () => {
    renderApp("/library");
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
      .map((l) => l.getAttribute("aria-label"));
    expect(labels).toEqual(["Home", "Inbox", "Library", "Settings"]);
    expect(await screen.findByRole("heading", { name: "Library" })).toBeVisible();
```

**with:**

```tsx
      .map((l) => l.getAttribute("aria-label"));
    expect(labels).toEqual(["Home", "Inbox", "Apps", "Library", "Settings"]);
    expect(await screen.findByRole("heading", { name: "Library" })).toBeVisible();
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/apps src/components/Transcript.test.ts src/lib/format.test.ts` in `ui`.
Expected: the three files fail to load (no `@/screens/apps/AppsScreen`, no `runTrigger`, no `formatBytes`).

- [ ] **Step 3: Add the screens**

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
import { cn } from "cn";
import { House, Inbox as InboxIcon, Library as LibraryIcon, Settings as SettingsIcon } from "lucide-react";
import { NavLink, Outlet } from "react-router";
```

**with:**

```tsx
import { cn } from "cn";
import {
  House,
  Inbox as InboxIcon,
  LayoutGrid,
  Library as LibraryIcon,
  Settings as SettingsIcon,
} from "lucide-react";
import { NavLink, Outlet } from "react-router";
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/library", label: "Library", icon: LibraryIcon },
```

**with:**

```tsx
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/apps", label: "Apps", icon: LayoutGrid },
  { to: "/library", label: "Library", icon: LibraryIcon },
```

**In `ui/src/app/routes.tsx`, replace:**

```tsx
import { Root } from "@/app/Root";
import { DotEditorScreen } from "@/screens/DotEditorScreen";
```

**with:**

```tsx
import { Root } from "@/app/Root";
import { AppScreen } from "@/screens/apps/AppScreen";
import { AppsScreen } from "@/screens/apps/AppsScreen";
import { DotEditorScreen } from "@/screens/DotEditorScreen";
```

**In `ui/src/app/routes.tsx`, replace:**

```tsx
      { path: "inbox", element: <InboxScreen /> },
      { path: "chat", element: <Navigate to="/home" replace /> },
```

**with:**

```tsx
      { path: "inbox", element: <InboxScreen /> },
      { path: "apps", element: <AppsScreen /> },
      { path: "apps/:name", element: <AppScreen /> },
      { path: "chat", element: <Navigate to="/home" replace /> },
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
import { StatusBadge } from "@/components/StatusBadge";
import { isActive } from "@/lib/home-topics";
import { localDateTime, localTime } from "@/lib/format";
```

**with:**

```tsx
import { StatusBadge } from "@/components/StatusBadge";
import { appTitle, isActive } from "@/lib/home-topics";
import { localDateTime, localTime } from "@/lib/format";
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  watcher: "Watcher run",
};

/// What the person wrote, for a chat run: an answer to a question as they typed it, not the
```

**with:**

```tsx
  watcher: "Watcher run",
};

/// What a run's header says started it; an app-started run names its app (app library spec
/// section 7).
export function runTrigger(run: Pick<Run, "trigger" | "payload">): string {
  const title = appTitle(run);
  return title === null ? TRIGGER[run.trigger] : `Started by ${title}`;
}

/// What the person wrote, for a chat run: an answer to a question as they typed it, not the
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx

/// The message a follow-up run carries: what the dot scheduled itself to do.
function FollowUpLine({ run }: { run: Run }) {
  if (run.trigger !== "follow_up" || typeof run.payload !== "object" || run.payload === null) {
    return null;
```

**with:**

```tsx

/// The message a follow-up run carries: what the dot scheduled itself to do; and the message an
/// app started a run with.
function FollowUpLine({ run }: { run: Run }) {
  const carries = run.trigger === "follow_up" || run.trigger === "app";
  if (!carries || typeof run.payload !== "object" || run.payload === null) {
    return null;
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  const names = props.runs.map((run) => {
    const name = `${TRIGGER[run.trigger]}, ${localDateTime(run.queued_at)}`;
    const n = (seen.get(name) ?? 0) + 1;
```

**with:**

```tsx
  const names = props.runs.map((run) => {
    const name = `${runTrigger(run)}, ${localDateTime(run.queued_at)}`;
    const n = (seen.get(name) ?? 0) + 1;
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{TRIGGER[run.trigger]}</span>
              <span>{localDateTime(run.queued_at)}</span>
```

**with:**

```tsx
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{runTrigger(run)}</span>
              <span>{localDateTime(run.queued_at)}</span>
```

**In `ui/src/lib/format.ts`, replace:**

```ts
/// when it is not a timestamp.
export function localDateTime(iso: string | null | undefined): string {
```

**with:**

```ts
/// when it is not a timestamp.
/// A size in bytes as a person reads it: "240 B", "2.4 KB", "1.5 MB".
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

export function localDateTime(iso: string | null | undefined): string {
```

**In `ui/src/lib/home-topics.ts`, replace:**

```ts

/// What started a run, in a word or two: "Scheduled", "Webhook", "Run now".
export function triggerLabel(run: Pick<Run, "trigger">): string {
  return TRIGGER[run.trigger];
}
```

**with:**

```ts

/// A run without its payload (a held run, a usage row) names no app.
export type Started = Pick<Run, "trigger"> & Partial<Pick<Run, "payload">>;

/// The title of the app that started `run` (app library spec section 7), or null.
export function appTitle(run: Started): string | null {
  if (run.trigger !== "app" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const title = (run.payload as Record<string, unknown>).app_title;
  return typeof title === "string" && title !== "" ? title : null;
}

/// What started a run, in a word or two: "Scheduled", "Webhook", "Run now", or the app's title.
export function triggerLabel(run: Started): string {
  return appTitle(run) ?? TRIGGER[run.trigger];
}
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  appProposals: (dotId: string | null) => ["app-proposals", dotId] as const,
  limits: ["limits"] as const,
```

**with:**

```ts
  appProposals: (dotId: string | null) => ["app-proposals", dotId] as const,
  /// The installed apps (the Apps screen), and below them the data removed apps kept.
  apps: ["apps"] as const,
  appsKept: ["apps", "kept"] as const,
  limits: ["limits"] as const,
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      // The Library lists, the Inbox group of a changed tool list, and what shows the dots that
      // use the item: removing it takes it out of them, and no dot event says so.
      for (const queryKey of [keys.library, keys.inbox, keys.dots, ["dot"], keys.overview]) {
        invalidate(qc, queryKey);
```

**with:**

```ts
      // The Library lists, the Inbox group of a changed tool list, and what shows the dots that
      // use the item: removing it takes it out of them, and no dot event says so. An app's row,
      // and the data a removed one kept, are on the Apps screen.
      for (const queryKey of [
        keys.library,
        keys.inbox,
        keys.dots,
        ["dot"],
        keys.overview,
        keys.apps,
      ]) {
        invalidate(qc, queryKey);
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
        invalidate(qc, queryKey);
      }
      break;
    case "org_updated":
```

**with:**

```ts
        invalidate(qc, queryKey);
      }
      break;
    case "app_data_changed":
      // The app's open screen refreshes (`dots.onChange`).
      appDataChanges.dispatchEvent(new Event(event.app));
      break;
    case "org_updated":
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts

/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
```

**with:**

```ts

/// Hears that an app's data changed: an event named by the app, for its open screen.
export const appDataChanges = new EventTarget();

/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
```

**Create `ui/src/screens/apps/AppScreen.tsx`:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";

import { commands } from "@/api/commands";
import { AppFrame } from "@/components/AppFrame";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { appSrc } from "@/lib/app-bridge";
import { keys } from "@/lib/query";
import { appDataChanges } from "@/lib/runtime-events";
import { appConfig } from "@/screens/apps/AppsScreen";

/// How many times the app `name`'s data changed since the screen opened.
function useDataChanges(name: string): number {
  const [changes, setChanges] = useState(0);
  useEffect(() => {
    const heard = () => setChanges((n) => n + 1);
    appDataChanges.addEventListener(name, heard);
    return () => appDataChanges.removeEventListener(name, heard);
  }, [name]);
  return changes;
}

/// `/apps/:name` (app library spec section 10): the app's screen filling the content area. Its
/// calls go to the app as the person.
export function AppScreen() {
  const name = useParams().name ?? "";
  const apps = useQuery({ queryKey: keys.apps, queryFn: commands.appsList });
  const changes = useDataChanges(name);
  const row = apps.data?.find((r) => r.item.name === name);
  const config = row ? appConfig(row) : null;
  if (apps.isPending) return <Page title="Apps" />;
  if (apps.isError || !row || !config || row.error) {
    return (
      <Page title={config?.title ?? name}>
        {apps.isError ? (
          <ErrorNote error={apps.error} />
        ) : row?.error ? (
          <ErrorNote error={`The app ${config?.title ?? name} cannot open: ${row.error}`} />
        ) : (
          <p className="text-muted-foreground">There is no app {name}.</p>
        )}
        <Link to="/apps" className="text-sm hover:underline">
          All apps
        </Link>
      </Page>
    );
  }
  return (
    <div className="flex h-full flex-col">
      <AppFrame
        src={appSrc(name, config.version)}
        title={config.title}
        tools={row.tools.map((t) => t.name)}
        call={(tool, input) => commands.appCall(name, tool, input)}
        changes={changes}
      />
    </div>
  );
}
```

**Create `ui/src/screens/apps/AppsScreen.tsx`:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { LayoutGrid } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { AppConfig, AppRow, KeptData } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { formatBytes, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { ItemRow } from "@/screens/library/parts";

/// The app's config, from its Library item.
export function appConfig(row: AppRow): AppConfig | null {
  return row.item.kind === "app" ? row.item.config : null;
}

/// The Apps screen (app library spec section 10): the installed apps, each opening its screen,
/// with its switches, roll back and remove; then the data removed apps kept.
export function AppsScreen() {
  const apps = useQuery({ queryKey: keys.apps, queryFn: commands.appsList });
  const kept = useQuery({ queryKey: keys.appsKept, queryFn: commands.appsKeptList });
  if (apps.isPending) return <Page title="Apps" />;
  if (apps.isError) {
    return (
      <Page title="Apps">
        <ErrorNote error={apps.error} />
      </Page>
    );
  }
  const keptData = kept.data ?? [];
  return (
    <Page title="Apps">
      {apps.data.length === 0 ? (
        <EmptyState icon={LayoutGrid}>
          No apps yet. Ask a dot in chat for one ("an app to track my expenses"); it sends the app
          for your review, and the apps you add show up here.
        </EmptyState>
      ) : (
        <ul className="flex flex-col gap-2">
          {apps.data.map((row) => (
            <AppItem key={row.item.id} row={row} />
          ))}
        </ul>
      )}
      {kept.isError && <ErrorNote error={kept.error} />}
      {keptData.length > 0 && (
        <section aria-labelledby="apps-kept" className="flex flex-col gap-2">
          <h2 id="apps-kept" className="font-semibold">
            Kept data
          </h2>
          <p className="text-xs text-muted-foreground">
            Data of removed apps. A new app of the same name whose first migrations match can
            start on it when you add it.
          </p>
          <ul className="flex flex-col gap-2">
            {keptData.map((data) => (
              <KeptItem key={data.id} data={data} />
            ))}
          </ul>
        </section>
      )}
    </Page>
  );
}

function AppItem({ row }: { row: AppRow }) {
  const qc = useQueryClient();
  const config = appConfig(row);
  const name = row.item.name;
  const changed = () => {
    invalidate(qc, keys.apps);
    invalidate(qc, keys.library);
    invalidateDots(qc);
  };
  const everyDot = useMutation({
    mutationFn: (on: boolean) => commands.appSetEveryDot(name, on),
    onSuccess: changed,
    onError: notifyError,
  });
  const canStart = useMutation({
    mutationFn: (on: boolean) => commands.appSetCanStartDots(name, on),
    onSuccess: changed,
    onError: notifyError,
  });
  const rollBack = useMutation({
    mutationFn: () => commands.appRollBack(name),
    onSuccess: changed,
    onError: notifyError,
  });
  if (config === null) return null;
  const title = config.title;
  const asksToStart = row.permissions.includes("start_dots");
  return (
    <ItemRow
      label={title}
      actions={
        <>
          {config.previous_version !== null && (
            <ConfirmButton
              title={`Roll back ${title}?`}
              description={`Version ${config.previous_version} becomes the active one again. Its data stays as it is: the update after it changed no data schema.`}
              confirmLabel="Roll back"
              disabled={rollBack.isPending}
              onConfirm={() => rollBack.mutate()}
            >
              Roll back
            </ConfirmButton>
          )}
          <RemoveApp row={row} title={title} />
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <Link to={`/apps/${encodeURIComponent(name)}`} className="font-medium hover:underline">
          {title}
        </Link>
        <span className="text-xs text-muted-foreground">version {config.version}</span>
        {config.every_dot && <Badge variant="secondary">Every dot</Badge>}
        {config.origin.kind === "bundled" && !config.changed && (
          <Badge variant="outline">Comes with Dots</Badge>
        )}
      </div>
      <span className="text-xs text-muted-foreground">{config.description}</span>
      {config.changed && (
        <span className="text-xs text-muted-foreground">
          Changed by you; Dots no longer updates it.
        </span>
      )}
      {row.error && <span className="text-xs text-destructive">{row.error}</span>}
      <span className="text-xs text-muted-foreground">
        {row.dots.length > 0 ? `Used by ${row.dots.join(", ")}` : "No dot uses it yet"}
      </span>
      <div className="flex flex-wrap items-center gap-4 pt-1 text-xs">
        <label className="flex items-center gap-2">
          <Switch
            checked={config.every_dot}
            disabled={everyDot.isPending}
            onCheckedChange={(on) => everyDot.mutate(on)}
          />
          Every dot
        </label>
        {asksToStart && (
          <label className="flex items-center gap-2">
            <Switch
              checked={config.can_start_dots}
              disabled={canStart.isPending}
              onCheckedChange={(on) => canStart.mutate(on)}
            />
            Can start dots
          </label>
        )}
      </div>
    </ItemRow>
  );
}

/// "Remove" (app library spec section 10): the usual "dots use it" confirmation, then a second
/// one for the data, which stays unless the person says to delete it too.
function RemoveApp({ row, title }: { row: AppRow; title: string }) {
  const qc = useQueryClient();
  const [step, setStep] = useState<"closed" | "app" | "data">("closed");
  const remove = useMutation({
    mutationFn: (deleteData: boolean) =>
      commands.appRemove(row.item.name, row.dots.length > 0, deleteData),
    onSuccess: () => {
      invalidate(qc, keys.apps);
      invalidate(qc, keys.library);
      invalidateDots(qc);
    },
    onError: notifyError,
    onSettled: () => setStep("closed"),
  });
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setStep("app")}>
        Remove
      </Button>
      <AlertDialog open={step === "app"} onOpenChange={(open) => !open && setStep("closed")}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {title}?</AlertDialogTitle>
            <AlertDialogDescription>
              {row.dots.length > 0
                ? `Used by ${row.dots.join(", ")}. Removing it takes it out of them.`
                : "No dot uses it."}{" "}
              Its code is deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button onClick={() => setStep("data")}>Remove app</Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={step === "data"} onOpenChange={(open) => !open && setStep("closed")}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {title}'s data too?</AlertDialogTitle>
            <AlertDialogDescription>
              Kept data stays on this PC under Kept data, and a new {title} can start on it.
              Deleted data cannot be brought back.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button
              variant="outline"
              disabled={remove.isPending}
              onClick={() => remove.mutate(false)}
            >
              Keep the data
            </Button>
            <AlertDialogAction
              disabled={remove.isPending}
              onClick={(e) => {
                e.preventDefault();
                remove.mutate(true);
              }}
            >
              Delete the data
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function KeptItem({ data }: { data: KeptData }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.appKeptDelete(data.id),
    onSuccess: () => invalidate(qc, keys.appsKept),
    onError: notifyError,
  });
  return (
    <ItemRow
      label={`${data.title}, kept data`}
      actions={
        <ConfirmButton
          title={`Delete the data of ${data.title}?`}
          description="It cannot be brought back, and no new app can start on it."
          confirmLabel="Delete data"
          disabled={remove.isPending}
          onConfirm={() => remove.mutate()}
        >
          Delete data
        </ConfirmButton>
      }
    >
      <span className="font-medium">{data.title}</span>
      <span className="text-xs text-muted-foreground">
        Removed {relativeTime(data.removed_at)} · {formatBytes(data.bytes)}
      </span>
    </ItemRow>
  );
}
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/apps src/components/Transcript.test.ts src/lib src/app src/screens/WatcherRuns.test.tsx`, `bun run typecheck` in `ui`.
Expected: PASS (the Apps screen's 8 tests among them); `tsc` clean.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: the UI 771 in 76 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/app/Layout.tsx ui/src/app/routes.tsx ui/src/components/Transcript.test.ts ui/src/components/Transcript.tsx ui/src/lib/format.test.ts ui/src/lib/format.ts ui/src/lib/home-topics.ts ui/src/lib/query.ts ui/src/lib/runtime-events.ts ui/src/screens/apps/AppScreen.tsx ui/src/screens/apps/AppsScreen.test.tsx ui/src/screens/apps/AppsScreen.tsx ui/src/screens/library/LibraryScreen.test.tsx
git commit -m "feat(ui): the Apps screen, an app's own screen, and app-started runs named by their app"
```

### Task 11: The review card

Spec §6.2 (the person's approved mockup): the card under the run that sent the app, in a chat thread or under a scheduled or manual run's card; Preview (the frame on the preview, opened with the card and closed with it), Code (every file; for a change first what it changes), Summary (description, tools with `(writes)`, "Nothing" or "Can start dots", "Sees the names of the dots that use it", the tests, files and size, the migrations), and "Add to library" / "Update" (with "Start on the data kept …" when offered), "Ask for changes" (at most 2000 characters) and "Discard". A refusal stays on the card.

**Files:**

- Create: `ui/src/components/AppReviewCard.tsx`, `ui/src/components/AppReviewCard.test.tsx`
- Modify: `ui/src/screens/home/{TopicRuns,Timeline}.tsx`, `ui/src/test/fixtures.ts` (a default `app_proposals_list`)

**Interfaces:**

- Consumes: Tasks 4, 6, 8, 9.
- Produces: `AppReviewCard({ proposal, dotName })`, `AppReviews({ runs, dotName })` (the waiting cards of those runs).

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/components/AppReviewCard.test.tsx`:**

```tsx
import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import type { AppProposal } from "@/api/types";
import { AppReviewCard } from "@/components/AppReviewCard";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { chat, dot, home, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const proposals = T.AppProposal.array().parse(contract.AppProposal);
const fresh = proposals.find((p) => p.id === "01J0PROP")!;
const onKept = proposals.find((p) => p.id === "01J0PROP5")!;
const change: AppProposal = {
  ...fresh,
  id: "01J0PROP6",
  app_id: "01J0APP2",
  base_version: 2,
  summary: { ...fresh.summary, update_from: 2, migrations: ["0002_category.sql"] },
};

function show(proposal: AppProposal) {
  const client = createQueryClient();
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <AppReviewCard proposal={proposal} dotName="builder" />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { ...view, user: userEvent.setup() };
}

function previewing() {
  backend.on("app_preview_open", () => "01J0PREV");
  backend.on("app_preview_close", () => null);
}

describe("an app sent for review", () => {
  it("runs the preview on its own database and closes it with the card", async () => {
    previewing();
    backend.on("app_preview_call", () => [{ item: "Coffee" }]);
    const { unmount } = show(fresh);
    expect(screen.getByRole("article", { name: "Review of Expenses" })).toHaveTextContent(
      "New app: Expenses · from ● builder",
    );
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses preview");
    expect(frame).toHaveAttribute("src", "http://dots-app.localhost/preview/01J0PREV/index.html");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(backend.callsTo("app_preview_open")).toEqual([{ id: fresh.id }]);
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    const got: unknown[] = [];
    port.onmessage = (e: MessageEvent) => got.push(e.data);
    port.postMessage({ type: "call", id: 1, tool: "add_expense", input: { item: "Tea" } });
    await waitFor(() => expect(got).toContainEqual(expect.objectContaining({ id: 1, ok: true })));
    expect(backend.callsTo("app_preview_call")).toEqual([
      { preview: "01J0PREV", tool: "add_expense", input: { item: "Tea" } },
    ]);
    unmount();
    await waitFor(() =>
      expect(backend.callsTo("app_preview_close")).toEqual([{ preview: "01J0PREV" }]),
    );
  });

  it("sums up what the app is, what it reaches and its tests", async () => {
    previewing();
    const { user } = show(fresh);
    await user.click(screen.getByRole("tab", { name: "Summary" }));
    const panel = screen.getByRole("tabpanel", { name: "Summary" });
    expect(panel).toHaveTextContent("Logs expenses and totals them by month");
    expect(panel).toHaveTextContent("add_expense (writes)");
    expect(panel).toHaveTextContent("Can start dots");
    expect(panel).toHaveTextContent("1 of 1 test passed");
    expect(panel).toHaveTextContent("7 files · 2.0 KB");
    expect(panel).toHaveTextContent("0001_expenses.sql");
    expect(screen.getByText(/It is added with "Can start dots" off/)).toBeVisible();
  });

  it("shows every file of a new app, and the diff of a change first", async () => {
    previewing();
    backend.on("app_proposal_files", () => [
      { path: "logic.js", text: "export function list() {}", base64: null },
      { path: "ui/logo.png", text: null, base64: "iVBORw0KGgo=" },
    ]);
    backend.on("app_proposal_diff", () => ({
      files: [{ path: "logic.js", status: "modified", patch: "@@ -1 +1 @@\n-a\n+b\n", binary: false }],
      truncated: false,
      exists: true,
    }));
    const { user, unmount } = show(fresh);
    await user.click(screen.getByRole("tab", { name: "Code" }));
    const files = await screen.findByRole("region", { name: "Every file" });
    expect(await within(files).findByText("logic.js")).toBeVisible();
    expect(within(files).getByText("Binary file.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "What it changes" })).toBeNull();
    expect(backend.callsTo("app_proposal_diff")).toEqual([]);
    unmount();

    const again = show(change);
    expect(screen.getByRole("article")).toHaveTextContent("Change to Expenses: version 2 and on");
    await again.user.click(screen.getByRole("tab", { name: "Code" }));
    const changes = await screen.findByRole("region", { name: "What it changes" });
    expect(await within(changes).findByText("+b")).toBeVisible();
    expect(screen.getByRole("button", { name: "Update" })).toBeVisible();
  });

  it("adds the app, on the kept data unless the person unticks it", async () => {
    previewing();
    backend.on("app_proposal_decide", () => onKept);
    const { user } = show(onKept);
    const box = screen.getByRole("checkbox", { name: /Start on the data kept from the removed Expenses/ });
    expect(box).toBeChecked();
    expect(box.parentElement).toHaveTextContent("it then runs 0002_category.sql on it");
    await user.click(screen.getByRole("button", { name: "Add to library" }));
    await waitFor(() =>
      expect(backend.callsTo("app_proposal_decide")).toEqual([
        { id: onKept.id, decision: { kind: "add_with_kept_data" } },
      ]),
    );
    await user.click(box);
    await user.click(screen.getByRole("button", { name: "Add to library" }));
    await waitFor(() => expect(backend.callsTo("app_proposal_decide")).toHaveLength(2));
    expect(backend.callsTo("app_proposal_decide")[1]).toEqual({
      id: onKept.id,
      decision: { kind: "add" },
    });
  });

  it("asks for changes in the person's words, and discards after asking", async () => {
    previewing();
    backend.on("app_proposal_decide", () => fresh);
    const { user } = show(fresh);
    await user.click(screen.getByRole("button", { name: "Ask for changes" }));
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();
    await user.type(screen.getByRole("textbox", { name: "What to change" }), "Add a category.");
    await user.click(send);
    await waitFor(() =>
      expect(backend.callsTo("app_proposal_decide")).toEqual([
        { id: fresh.id, decision: { kind: "ask_for_changes", text: "Add a category." } },
      ]),
    );
    await user.click(screen.getByRole("button", { name: "Discard" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }),
    );
    await waitFor(() => expect(backend.callsTo("app_proposal_decide")).toHaveLength(2));
    expect(backend.callsTo("app_proposal_decide")[1]).toEqual({
      id: fresh.id,
      decision: { kind: "discard" },
    });
  });

  it("keeps waiting with the refusal shown when the decision is refused", async () => {
    previewing();
    backend.on("app_proposal_decide", () => {
      throw {
        kind: "conflict",
        message: "the dot is still working on this chat; wait until it has finished",
      };
    });
    const { user } = show(fresh);
    await user.click(screen.getByRole("button", { name: "Add to library" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("the dot is still working on this chat");
    expect(screen.getByRole("button", { name: "Add to library" })).toBeEnabled();
  });
});

describe("a review card in the conversation", () => {
  it("stands under the run that sent the app, while it waits", async () => {
    previewing();
    const run = chat("c1", "10:00");
    home([run]);
    const waiting = { ...fresh, dot_id: dot.id, run_id: run.id, root_run_id: run.id };
    const decided = { ...waiting, id: "01J0PROP7", status: "added" as const };
    backend.on("app_proposals_list", () => [waiting, decided]);
    renderApp(`/home/${dot.id}`);
    const conversation = await screen.findByRole("region", { name: "Conversation" });
    const cards = await within(conversation).findAllByRole("article", { name: "Review of Expenses" });
    expect(cards).toHaveLength(1);
    expect(backend.callsTo("app_proposals_list")).toContainEqual({ dotId: dot.id });
  });

  it("stands under the card of a scheduled or manual run that sent the app", async () => {
    previewing();
    const run = scheduled("s1", "09:00");
    home([run]);
    const waiting = { ...fresh, dot_id: dot.id, run_id: run.id, root_run_id: run.id };
    backend.on("app_proposals_list", () => [waiting]);
    renderApp(`/home/${dot.id}`);
    const conversation = await screen.findByRole("region", { name: "Conversation" });
    expect(
      await within(conversation).findByRole("article", { name: "Review of Expenses" }),
    ).toBeVisible();
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  backend.on("approval_rule_candidates", () => []);
  backend.on("dot_overview", () => []);
```

**with:**

```ts
  backend.on("approval_rule_candidates", () => []);
  // A conversation asks for the dot's app proposals; by default it has none.
  backend.on("app_proposals_list", () => []);
  backend.on("dot_overview", () => []);
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/components/AppReviewCard.test.tsx` in `ui`.
Expected: the file fails to load (no `@/components/AppReviewCard`).

- [ ] **Step 3: Add the card**

**Create `ui/src/components/AppReviewCard.tsx`:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { AppFile, AppProposal, ProposalDecision, Run } from "@/api/types";
import { AppFrame } from "@/components/AppFrame";
import { ConfirmButton } from "@/components/ConfirmButton";
import { DiffView } from "@/components/DiffView";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { previewSrc } from "@/lib/app-bridge";
import { formatBytes, relativeTime } from "@/lib/format";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { plural } from "@/screens/library/parts";

/// The most characters "Ask for changes" takes (`propose::MAX_NOTE`).
const MAX_ASK = 2000;

/// The cards of the apps `runs` sent for review that still wait, under those runs: a chat
/// thread's runs, or the first run of a run topic (a scheduled or manual run).
export function AppReviews({ runs, dotName }: { runs: Run[]; dotName: string }) {
  const dotId = runs[0]?.dot_id ?? null;
  const proposals = useQuery({
    queryKey: keys.appProposals(dotId),
    queryFn: () => commands.appProposalsList(dotId),
    enabled: dotId !== null,
  });
  if (proposals.isError) return <ErrorNote error={proposals.error} />;
  const waiting = (proposals.data ?? []).filter(
    (p) => p.status === "pending" && runs.some((run) => run.id === p.run_id),
  );
  return waiting.map((proposal) => (
    <AppReviewCard key={proposal.id} proposal={proposal} dotName={dotName} />
  ));
}

/// A dot's app waiting for the person in its conversation (app library spec section 6.2): the
/// app running in its sandbox on sample data (Preview), every file or the diff of a change
/// (Code), what it is and reaches (Summary), and the decision.
export function AppReviewCard({ proposal, dotName }: { proposal: AppProposal; dotName: string }) {
  const { summary } = proposal;
  const update = summary.update_from !== null;
  const heading = update
    ? `Change to ${summary.title}: version ${summary.update_from} and on`
    : `New app: ${summary.title}`;
  return (
    <article
      aria-label={`Review of ${summary.title}`}
      className="flex flex-col overflow-hidden rounded-lg border border-needs-border bg-card"
    >
      <header className="border-b p-3">
        <p className="font-semibold">
          {heading} <span className="font-normal text-muted-foreground">· from ● {dotName}</span>
        </p>
        <p className="text-xs whitespace-pre-wrap text-muted-foreground">{proposal.note}</p>
      </header>
      <Tabs defaultValue="preview" className="gap-0">
        <TabsList className="mx-3 mt-2">
          <TabsTrigger value="preview">Preview</TabsTrigger>
          <TabsTrigger value="code">Code</TabsTrigger>
          <TabsTrigger value="summary">Summary</TabsTrigger>
        </TabsList>
        {/* Kept while another tab shows, so the preview's changes last until the card closes. */}
        <TabsContent value="preview" forceMount className="p-3 data-[state=inactive]:hidden">
          <Preview proposal={proposal} />
        </TabsContent>
        <TabsContent value="code" className="p-3">
          <Code proposal={proposal} />
        </TabsContent>
        <TabsContent value="summary" className="p-3">
          <Summary proposal={proposal} />
        </TabsContent>
      </Tabs>
      <Decide proposal={proposal} />
    </article>
  );
}

/// The proposal's screen on its own throwaway database, opened with the card and closed with it.
function Preview({ proposal }: { proposal: AppProposal }) {
  const [opened, setOpened] = useState<{ id: string } | { error: unknown } | null>(null);
  useEffect(() => {
    let gone = false;
    let id: string | null = null;
    commands.appPreviewOpen(proposal.id).then(
      (open) => {
        if (gone) void commands.appPreviewClose(open).catch(() => {});
        else {
          id = open;
          setOpened({ id: open });
        }
      },
      (error: unknown) => {
        if (!gone) setOpened({ error });
      },
    );
    return () => {
      gone = true;
      if (id !== null) void commands.appPreviewClose(id).catch(() => {});
    };
  }, [proposal.id]);
  if (opened === null) return <p className="text-muted-foreground">Starting the preview…</p>;
  if ("error" in opened) return <ErrorNote error={opened.error} />;
  const preview = opened.id;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex h-[420px] flex-col overflow-hidden rounded-md border-2 border-dashed border-primary/40">
        <AppFrame
          src={previewSrc(preview)}
          title={`${proposal.summary.title} preview`}
          tools={proposal.summary.tools.map((t) => t.name)}
          call={(tool, input) => commands.appPreviewCall(preview, tool, input)}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Running in its sandbox on sample data. Your changes here vanish when the card closes, and
        it cannot start dots.
      </p>
    </div>
  );
}

function FileText({ file }: { file: AppFile }) {
  return (
    <details className="rounded-md border">
      <summary className="cursor-pointer px-2 py-1 font-mono text-xs">{file.path}</summary>
      {file.text === null ? (
        <p className="border-t px-2 py-1 text-xs text-muted-foreground">Binary file.</p>
      ) : (
        <pre className="overflow-x-auto border-t p-2 text-xs">{file.text}</pre>
      )}
    </details>
  );
}

/// Every file; for a change, first what it changes against the active version.
function Code({ proposal }: { proposal: AppProposal }) {
  const update = proposal.summary.update_from !== null;
  const files = useQuery({
    queryKey: [...keys.appProposals(proposal.dot_id), proposal.id, "files"],
    queryFn: () => commands.appProposalFiles(proposal.id),
  });
  const diff = useQuery({
    queryKey: [...keys.appProposals(proposal.dot_id), proposal.id, "diff"],
    queryFn: () => commands.appProposalDiff(proposal.id),
    enabled: update,
  });
  return (
    <div className="flex flex-col gap-3">
      {update && (
        <section aria-label="What it changes" className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold text-muted-foreground uppercase">
            What it changes
          </h3>
          {diff.isError ? (
            <ErrorNote error={diff.error} />
          ) : diff.data ? (
            <DiffView diff={diff.data} />
          ) : (
            <p className="text-muted-foreground">Loading the changes…</p>
          )}
        </section>
      )}
      <section aria-label="Every file" className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase">Every file</h3>
        {files.isError ? (
          <ErrorNote error={files.error} />
        ) : (
          (files.data ?? []).map((file) => <FileText key={file.path} file={file} />)
        )}
      </section>
    </div>
  );
}

function Label({ children }: { children: string }) {
  return <dt className="mt-2 text-xs text-muted-foreground uppercase">{children}</dt>;
}

function Summary({ proposal }: { proposal: AppProposal }) {
  const { summary } = proposal;
  const results = summary.tests.results;
  const passed = results.filter((r) => r.passed).length;
  const update = summary.update_from !== null;
  return (
    <dl className="text-sm">
      <Label>What it does</Label>
      <dd>{summary.description}</dd>
      <Label>Tools dots get</Label>
      <dd>
        <ul className="flex flex-col gap-1">
          {summary.tools.map((tool) => (
            <li key={tool.name}>
              <code className="font-mono text-xs">{tool.name}</code>
              {tool.writes && <span className="text-xs text-needs"> (writes)</span>}{" "}
              <span className="text-xs text-muted-foreground">{tool.description}</span>
            </li>
          ))}
        </ul>
      </dd>
      <Label>Reaches outside its data</Label>
      <dd>
        {summary.permissions.includes("start_dots")
          ? "Can start dots"
          : "Nothing (no files, no network)"}
      </dd>
      <dd className="text-xs text-muted-foreground">Sees the names of the dots that use it.</dd>
      <Label>Tests</Label>
      <dd>
        {passed} of {plural(results.length, "test")} passed
        <ul className="text-xs text-muted-foreground">
          {results.map((r) => (
            <li key={`${r.file}/${r.name}`}>
              {r.passed ? "✓" : "✗"} {r.name || r.file}
            </li>
          ))}
        </ul>
      </dd>
      <Label>Size</Label>
      <dd>
        {plural(summary.files, "file")} · {formatBytes(summary.bytes)}
      </dd>
      <Label>{update ? "Migrations it runs on your data" : "Migrations"}</Label>
      <dd>
        {summary.migrations.length === 0 ? "None" : summary.migrations.join(", ")}
      </dd>
    </dl>
  );
}

/// Add to library (or Update), Ask for changes and Discard. A refusal (the dot's chat is still
/// working, the app moved on) stays on the card, which keeps waiting.
function Decide({ proposal }: { proposal: AppProposal }) {
  const qc = useQueryClient();
  const { summary } = proposal;
  const kept = summary.kept_data;
  const [onKept, setOnKept] = useState(true);
  const [asking, setAsking] = useState(false);
  const [text, setText] = useState("");
  const decide = useMutation({
    mutationFn: (decision: ProposalDecision) => commands.appProposalDecide(proposal.id, decision),
    onSuccess: () => {
      for (const queryKey of [
        ["app-proposals"],
        keys.apps,
        keys.library,
        keys.inbox,
        keys.overview,
        keys.runs(proposal.dot_id),
        keys.thread(proposal.root_run_id),
      ]) {
        invalidate(qc, queryKey);
      }
    },
  });
  const update = summary.update_from !== null;
  const starts = summary.permissions.includes("start_dots");
  const add = () =>
    decide.mutate(kept !== null && onKept ? { kind: "add_with_kept_data" } : { kind: "add" });
  return (
    <footer className="flex flex-col gap-2 border-t p-3">
      {starts && (
        <p className="text-xs text-muted-foreground">
          It asks to start dots. It is added with "Can start dots" off; you can switch it on in{" "}
          <Link to="/apps" className="text-foreground hover:underline">
            Apps
          </Link>
          .
        </p>
      )}
      {kept !== null && (
        <label className="flex items-start gap-2 text-xs">
          <input
            type="checkbox"
            checked={onKept}
            onChange={(e) => setOnKept(e.target.checked)}
          />
          <span>
            Start on the data kept from the removed {kept.title} (removed{" "}
            {relativeTime(kept.removed_at)}, {formatBytes(kept.bytes)})
            {kept.migrations.length > 0 && `; it then runs ${kept.migrations.join(", ")} on it`}
          </span>
        </label>
      )}
      {decide.isError && <ErrorNote error={decide.error} />}
      {asking && (
        <div className="flex flex-col gap-2">
          <Textarea
            aria-label="What to change"
            value={text}
            maxLength={MAX_ASK}
            onChange={(e) => setText(e.target.value)}
            placeholder="Say what the dot should change"
          />
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={decide.isPending || text.trim() === ""}
              onClick={onSingleClick(() =>
                decide.mutate({ kind: "ask_for_changes", text: text.trim() }),
              )}
            >
              Send
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setAsking(false)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={decide.isPending} onClick={onSingleClick(add)}>
          {update ? "Update" : "Add to library"}
        </Button>
        {!asking && (
          <Button
            size="sm"
            variant="outline"
            disabled={decide.isPending}
            onClick={() => setAsking(true)}
          >
            Ask for changes
          </Button>
        )}
        <ConfirmButton
          title={`Discard ${summary.title}?`}
          description="The dot's files for it are deleted here, and the dot hears that you discarded it."
          confirmLabel="Discard"
          disabled={decide.isPending}
          onConfirm={() => decide.mutate({ kind: "discard" })}
        >
          Discard
        </ConfirmButton>
      </div>
    </footer>
  );
}
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx
import type { Message } from "@/api/types";
import { Button } from "@/components/ui/button";
```

**with:**

```tsx
import type { Message } from "@/api/types";
import { AppReviews } from "@/components/AppReviewCard";
import { Button } from "@/components/ui/button";
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx
              />
              {topic.runs.length > 1 && (
```

**with:**

```tsx
              />
              {/* An app the run sent for review waits under its card. */}
              <AppReviews runs={topic.runs.slice(0, 1)} dotName={dotName} />
              {topic.runs.length > 1 && (
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
import { AppliedCalls } from "@/components/AppliedCalls";
import { ApprovalCard } from "@/components/ApprovalCard";
```

**with:**

```tsx
import { AppliedCalls } from "@/components/AppliedCalls";
import { AppReviews } from "@/components/AppReviewCard";
import { ApprovalCard } from "@/components/ApprovalCard";
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
/// steps, check-ins and browser calls (`Transcript`), the calls Dots performed, a card per
/// approval it waits for, and why the last run failed.
export function TopicRuns(props: { runs: Run[]; checkIns: Message[]; dotName: string }) {
```

**with:**

```tsx
/// steps, check-ins and browser calls (`Transcript`), the calls Dots performed, a card per
/// approval it waits for and per app it sent for review, and why the last run failed.
export function TopicRuns(props: { runs: Run[]; checkIns: Message[]; dotName: string }) {
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
      ))}
      {failure && <Failure>{failure}</Failure>}
```

**with:**

```tsx
      ))}
      <AppReviews runs={runs} dotName={dotName} />
      {failure && <Failure>{failure}</Failure>}
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/components/AppReviewCard.test.tsx src/screens/home`, `bun run typecheck` in `ui`.
Expected: PASS (the card's 8 tests among them); `tsc` clean.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: the UI 779 in 77 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/AppReviewCard.test.tsx ui/src/components/AppReviewCard.tsx ui/src/screens/home/Timeline.tsx ui/src/screens/home/TopicRuns.tsx ui/src/test/fixtures.ts
git commit -m "feat(ui): the review card in the dot's conversation"
```

### Task 12: The dot editor's Apps checklist

Spec §6 and §10: "Tools and skills" gains an Apps checklist; an "Every dot" app shows ticked and fixed; with no app, it says where apps come from.

**Files:**

- Modify: `ui/src/screens/dot-editor/ToolsAndSkills.tsx`
- Test: `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`

**Interfaces:**

- Consumes: plan A's `Uses.apps` and the Library's `app` entries.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx

const [github, pluginServer, pdf, pdfTools, company] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [], apps: [] };
```

**with:**

```tsx

const [github, pluginServer, pdf, pdfTools, company, projects] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [], apps: [] };
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, replace:**

```tsx
    expect(uses().context).toEqual(["clients"]);
  });
```

**with:**

```tsx
    expect(uses().context).toEqual(["clients"]);
  });

  it("lists the apps, one every dot uses ticked and fixed", async () => {
    const own = { ...projects!, id: "01J0APP3", name: "expenses" } as LibraryEntry;
    if (own.kind !== "app") throw new Error("the sample is an app");
    own.config = { ...own.config, every_dot: false, description: "Logs expenses" };
    const { user, uses } = setup({ library: [projects!, own] });
    const group = await screen.findByRole("group", { name: "Apps" });
    const every = within(group).getByLabelText("projects");
    expect(every).toBeChecked();
    expect(every).toBeDisabled();
    expect(within(group).getByText(/Projects, tasks and to-dos, shared by you and your dots · every dot/)).toBeVisible();
    const box = within(group).getByLabelText("expenses");
    expect(box).not.toBeChecked();
    await user.click(box);
    expect(uses().apps).toEqual(["expenses"]);
  });

  it("says where apps come from when there is none", async () => {
    setup({ library: [] });
    const group = await screen.findByRole("group", { name: "Apps" });
    expect(group).toHaveTextContent("No apps yet. Ask a dot for one; the apps you add show in Apps.");
    expect(within(group).getByRole("link", { name: "Apps" })).toHaveAttribute("href", "/apps");
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/dot-editor/ToolsAndSkills.test.tsx` in `ui`.
Expected: FAIL: 2 tests (no group "Apps").

- [ ] **Step 3: Add the checklist**

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
  { kind: "context", key: "context", title: "Context" },
];
```

**with:**

```tsx
  { kind: "context", key: "context", title: "Context" },
  { kind: "app", key: "apps", title: "Apps" },
];
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx

/// A context collection every dot reads, ticked or not.
const everyDot = (entry: LibraryEntry) => entry.kind === "context" && entry.config.every_dot;

```

**with:**

```tsx

/// A context collection every dot reads, or an app every dot uses, ticked or not.
const everyDot = (entry: LibraryEntry) =>
  (entry.kind === "context" || entry.kind === "app") && entry.config.every_dot;

```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
              .join(" · ")
          : null;
  const status = entry.catalog?.status;
```

**with:**

```tsx
              .join(" · ")
          : entry.kind === "app"
            ? [entry.config.description, entry.config.every_dot && "every dot"]
                .filter(Boolean)
                .join(" · ")
            : null;
  const status = entry.catalog?.status;
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx

/// The dot editor's "Tools and skills" section: which Library items the dot uses, as four
/// checklists. A plugin's own MCP servers are not listed: they come with the plugin. A context
/// collection for every dot shows ticked and cannot be unticked here.
export function ToolsAndSkills(props: {
```

**with:**

```tsx

/// The dot editor's "Tools and skills" section: which Library items the dot uses, as five
/// checklists. A plugin's own MCP servers are not listed: they come with the plugin. A context
/// collection or an app for every dot shows ticked and cannot be unticked here.
export function ToolsAndSkills(props: {
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
            <legend className="mb-1 text-xs font-medium text-muted-foreground">{title}</legend>
            {items.length === 0 && stale.length === 0 && (
              <p className="text-xs text-muted-foreground">
```

**with:**

```tsx
            <legend className="mb-1 text-xs font-medium text-muted-foreground">{title}</legend>
            {items.length === 0 && stale.length === 0 && kind !== "app" && (
              <p className="text-xs text-muted-foreground">
```

**In `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, replace:**

```tsx
                Nothing here yet. Add some in the <Link to="/library" className="underline">Library</Link>.
              </p>
```

**with:**

```tsx
                Nothing here yet. Add some in the <Link to="/library" className="underline">Library</Link>.
              </p>
            )}
            {items.length === 0 && stale.length === 0 && kind === "app" && (
              <p className="text-xs text-muted-foreground">
                No apps yet. Ask a dot for one; the apps you add show in{" "}
                <Link to="/apps" className="underline">Apps</Link>.
              </p>
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/dot-editor src/screens/DotEditorScreen.test.tsx`, `bun run typecheck` in `ui`.
Expected: PASS; `tsc` clean.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: the UI 781 in 77 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/dot-editor/ToolsAndSkills.test.tsx ui/src/screens/dot-editor/ToolsAndSkills.tsx
git commit -m "feat(ui): the dot editor's Apps checklist"
```

### Task 13: The frame, checked in WebView2

Spec §5: "The builder confirms this with a test that a frame cannot reach `ipc:`." jsdom has no CSP and no Tauri, so the check runs in a debug Dots: a probe app whose page tries what an app's screen must not reach and records each outcome through the bridge. Dots gives the CLI only the variables it keeps, so `fake-claude` also reads its script from `CLAUDE_CODE_FAKE_SCRIPT`.

**Files:**

- Create: `tools/smoke/apps/probe/{app.json,logic.js,migrations/0001_probes.sql,tests/probe.test.js,ui/index.html,ui/own.js}`
- Modify: `crates/dots-core/src/bin/fake-claude.rs`

- [ ] **Step 1: Add the probe app and the script variable**

**Create `tools/smoke/apps/probe/tests/probe.test.js`:**

```
test("a probe's last outcome is kept", () => {
  call("record", { probe: "network", reached: true, detail: "200" });
  call("record", { probe: "network", reached: false, detail: "blocked" });
  assert.deepEqual(call("results", {}), [{ probe: "network", reached: false, detail: "blocked" }]);
});
```

**In `crates/dots-core/src/bin/fake-claude.rs`, replace:**

```rust
//! Test stand-in for `claude -p --output-format stream-json`. Used only by dots-core tests.

```

**with:**

```rust
//! Test stand-in for `claude -p --output-format stream-json`. Used by dots-core tests, and by
//! smokes of a debug Dots: Dots gives the CLI only the variables it keeps (`vault::child_env`),
//! so a smoke names the script in `CLAUDE_CODE_FAKE_SCRIPT`, which it keeps, in place of
//! `FAKE_CLAUDE_SCRIPT`.

```

**In `crates/dots-core/src/bin/fake-claude.rs`, replace:**

```rust
    let session = flag(&args, "--resume").unwrap_or_else(|| "fake-session".to_string());
    let script: Vec<Value> = match std::env::var("FAKE_CLAUDE_SCRIPT") {
        Ok(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
```

**with:**

```rust
    let session = flag(&args, "--resume").unwrap_or_else(|| "fake-session".to_string());
    let named =
        std::env::var("FAKE_CLAUDE_SCRIPT").or_else(|_| std::env::var("CLAUDE_CODE_FAKE_SCRIPT"));
    let script: Vec<Value> = match named {
        Ok(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
```

**Create `tools/smoke/apps/probe/app.json`:**

```json
{
  "name": "probe",
  "title": "Probe",
  "description": "Tries to reach what an app's screen must not, for the frame smoke",
  "tools": [
    {
      "name": "record",
      "description": "Record whether a probe reached what it tried",
      "input": {
        "type": "object",
        "properties": {
          "probe": { "type": "string", "minLength": 1, "maxLength": 40 },
          "reached": { "type": "boolean" },
          "detail": { "type": "string", "maxLength": 200 }
        },
        "required": ["probe", "reached"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "results",
      "description": "What each probe reached, by probe",
      "input": { "type": "object" }
    }
  ]
}
```

**Create `tools/smoke/apps/probe/logic.js`:**

```
export function record({ probe, reached, detail }, ctx) {
  ctx.db.exec(
    "INSERT INTO probes (probe, reached, detail, at) VALUES (?, ?, ?, ?) " +
      "ON CONFLICT (probe) DO UPDATE SET reached = excluded.reached, detail = excluded.detail, at = excluded.at",
    [probe, reached ? 1 : 0, detail ?? "", ctx.now],
  );
  return { probe };
}

export function results(input, ctx) {
  return ctx.db
    .query("SELECT probe, reached, detail FROM probes ORDER BY probe", [])
    .map((r) => ({ probe: r.probe, reached: r.reached === 1, detail: r.detail }));
}
```

**Create `tools/smoke/apps/probe/migrations/0001_probes.sql`:**

```sql
CREATE TABLE probes (probe TEXT PRIMARY KEY, reached INTEGER NOT NULL, detail TEXT NOT NULL,
  at TEXT NOT NULL);
```

**Create `tools/smoke/apps/probe/ui/index.html`:**

```
<!doctype html>
<meta charset="utf-8">
<title>Probe</title>
<p id="out">Probing…</p>
<script>
  // Each probe tries something an app's screen must not reach, and records whether it got
  // there through the bridge. Only "bridge", "theme" and "own_script" should say reached.
  const out = document.getElementById("out");
  const record = (probe, reached, detail) =>
    dots.call("record", { probe, reached, detail: String(detail ?? "").slice(0, 200) });
  const attempt = async (probe, fn) => {
    try {
      await record(probe, true, await fn());
    } catch (e) {
      await record(probe, false, (e && e.message) || e);
    }
  };
  const within = (ms, promise) =>
    Promise.race([promise, new Promise((_, no) => setTimeout(() => no(new Error(`no answer in ${ms} ms`)), ms))]);
  (async () => {
    await record("bridge", true, "dots.call answered");
    await attempt("theme", async () => {
      await within(3000, new Promise((ok) => {
        const value = () => getComputedStyle(document.documentElement).getPropertyValue("--dots-primary");
        if (value()) ok();
        else dots.onTheme(() => ok());
      }));
      return getComputedStyle(document.documentElement).getPropertyValue("--dots-primary");
    });
    await attempt("ipc_fetch", async () => {
      const r = await within(3000, fetch("http://ipc.localhost/apps_list", { method: "POST", body: "{}" }));
      return `HTTP ${r.status}`;
    });
    await attempt("ipc_internals", async () => {
      const internals = window.__TAURI_INTERNALS__;
      if (!internals) throw new Error("no __TAURI_INTERNALS__");
      const answer = await within(3000, internals.invoke("app_set_every_dot", { name: "probe", on: true }));
      return JSON.stringify(answer).slice(0, 100);
    });
    await attempt("network", async () => {
      const r = await within(5000, fetch("https://example.com/"));
      return `HTTP ${r.status}`;
    });
    await attempt("image", () =>
      within(5000, new Promise((ok, no) => {
        const img = new Image();
        img.onload = () => ok("loaded");
        img.onerror = () => no(new Error("blocked"));
        img.src = "https://example.com/favicon.ico";
      })),
    );
    await attempt("parent_dom", async () => String(parent.document.title));
    await attempt("storage", async () => {
      localStorage.setItem("probe", "1");
      return "stored";
    });
    const script = (src) =>
      within(3000, new Promise((ok, no) => {
        const s = document.createElement("script");
        s.onload = () => ok("loaded");
        s.onerror = () => no(new Error("blocked"));
        s.src = src;
        document.head.appendChild(s);
      }));
    // A file of its own folder loads; a script from anywhere else does not.
    await attempt("own_script", () => script("own.js"));
    await attempt("data_script", () => script("data:text/javascript,window.fromData=1"));
    await record("done", true, "all probes ran");
    out.textContent = "Done.";
  })();
</script>
```

**Create `tools/smoke/apps/probe/ui/own.js`:**

```
window.ownLoaded = true;
```

- [ ] **Step 2: Build a debug Dots and the fake CLI**

The installed Dots may be running (`%LOCALAPPDATA%\Dots\dots-app.exe`): it is never stopped. It holds port 47321 and WebView2's shared data folder, so the debug Dots gets its own of both below.

Run: `bun x vite build` in `ui`, then from the repository root `bun run tauri build --debug --no-bundle` and `cargo build -j 2 -p dots-core --bin fake-claude`.
Expected: `target/debug/dots-app.exe` and `target/debug/fake-claude.exe` are built.

- [ ] **Step 3: Start it on a temp data folder**

```bash
S="$(mktemp -d)"; mkdir -p "$S/data" "$S/work" "$S/bin"
cp -r tools/smoke/apps/probe "$S/work/probe"
cp "$CARGO_TARGET_DIR/debug/fake-claude.exe" "$S/bin/fake-claude.exe"
cat > "$S/script.json" <<'JSON'
[
  {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
  {"mcp_call": {"server": "dots", "tool": "propose_app",
                "arguments": {"path": "probe", "note": "The frame probe."}}},
  {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "Sent.",
            "usage": {"input_tokens": 1, "output_tokens": 1}}}
]
JSON
S_WIN="$(cygpath -m "$S")"
DOTS_DATA_DIR="$S_WIN/data" WEBVIEW2_USER_DATA_FOLDER="$S_WIN/webview" \
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
  CLAUDE_CODE_FAKE_SCRIPT="$S_WIN/script.json" DOTS_CONFIRM=yes \
  "$CARGO_TARGET_DIR/debug/dots-app.exe" > "$S/app.log" 2>&1 &
W="node tools/smoke/webview.mjs 9333"
```

Then, once `$W eval "typeof invoke"` answers `"function"`, point it at the fake CLI and another port, and restart it:

```bash
$W eval "(async () => { const s = await invoke('get_settings'); s.claude_path = '$S_WIN/bin/fake-claude.exe'; s.port = 47399; await invoke('update_settings', { settings: s }); return 'saved'; })()"
$W eval "invoke('restart_app')"
```

Expected: "saved"; then "the page closed before it answered" (the restart). Wait until `$W eval "invoke('status').then(s => s.port)"` answers `47399`.

- [ ] **Step 4: Have a dot propose the probe, and add it on its card**

```bash
DOT=$($W eval "(async () => { const spec = await invoke('new_dot_spec'); spec.name = 'prober'; spec.instructions = 'Propose the probe.'; spec.engine = 'claude'; spec.workdir = '$S_WIN/work'; spec.workspace_mode = 'folder'; const dot = await invoke('create_dot', { spec, orgId: null }); await invoke('run_now', { dotId: dot.id }); return dot.id; })()" | tr -d '"')
$W goto "home/$DOT"
$W wait "New app: Probe" 30
$W eval "(() => { const f = document.querySelector('iframe'); return [f.getAttribute('src'), f.getAttribute('sandbox')]; })()"
$W click "Add to library"
```

Expected: the card shows, its preview frame is `http://dots-app.localhost/preview/<id>/index.html` with sandbox `allow-scripts`, and its page reads "Done."; after the click, `$W eval "(async () => (await invoke('apps_list')).map(r => r.item.name))()"` lists `probe`.

- [ ] **Step 5: Open the probe's screen and read what it reached**

```bash
$W goto apps/probe
sleep 15
$W eval "(async () => (await invoke('app_call', { name: 'probe', tool: 'results', input: {} })).map(r => [r.probe, r.reached]))()"
$W eval "(async () => (await invoke('apps_list')).map(r => [r.item.name, r.item.config.every_dot]))()"
```

Expected: `bridge`, `done`, `own_script` and `theme` reached; `data_script`, `image`, `ipc_fetch`, `ipc_internals`, `network`, `parent_dom` and `storage` did not (`ipc_internals` reads "no answer in 3000 ms": WebView2 puts Tauri's scripts into the frame, and the call they send is not carried out); `probe` keeps `every_dot` false. Any other outcome stops the plan: the frame is not isolated.

- [ ] **Step 6: Stop the debug Dots, and commit**

Stop only the process whose path is `…\target\debug\dots-app.exe` (`Get-Process dots-app | Where-Object { $_.Path -like '*\target\debug\dots-app.exe' } | Stop-Process`), never the installed one.

Run: `cargo test -j 2 -p dots-core --test claude_cli_test --test apps_propose_test`, `cargo fmt --check`.
Expected: PASS: 11 and 12 tests.

```bash
git add crates/dots-core/src/bin/fake-claude.rs tools/smoke/apps/probe/app.json tools/smoke/apps/probe/logic.js tools/smoke/apps/probe/migrations/0001_probes.sql tools/smoke/apps/probe/tests/probe.test.js tools/smoke/apps/probe/ui/index.html tools/smoke/apps/probe/ui/own.js
git commit -m "test: a probe app checks in WebView2 what an app's frame can reach"
```

### Task 14: The spec says what plan B built

**Files:**

- Modify: `docs/superpowers/specs/2026-10-23-app-library-design.md`

- [ ] **Step 1: Add §17**

**In `docs/superpowers/specs/2026-10-23-app-library-design.md`, replace:**

```markdown
The script engine if not QuickJS (with the properties of section 5); the JSON Schema subset; how `ui/` files reach the frame; the exact limits; the "Apps" prompt section's wording; the Projects starting message; component layout of the Apps screen and the card beyond the approved mockups; whether the plan is split (my suggestion: A runtime, store, tools and policy; B screens and the review card; C Projects).
```

**with:**

```markdown
The script engine if not QuickJS (with the properties of section 5); the JSON Schema subset; how `ui/` files reach the frame; the exact limits; the "Apps" prompt section's wording; the Projects starting message; component layout of the Apps screen and the card beyond the approved mockups; whether the plan is split (my suggestion: A runtime, store, tools and policy; B screens and the review card; C Projects).

## 17. Changes made while building (plan B)

Plan B built the screens, the frame and its bridge, the review card and the editor's checklist; these are the places where it says more than the sections above, or differs from them.

- **How the frame gets its files.** Dots serves the apps' pages through its own protocol `dots-app` (on Windows `http://dots-app.localhost`): `/app/<name>/<version>/<path>` for an installed app's version and `/preview/<id>/<path>` for a review card's preview, only files below `ui/`, and only to `GET`. Each answer carries the page's CSP with the page's own folder as "<app files>", and `font-src` for that folder too; `index.html` gets Dots' client right after its doctype. Dots' own CSP gains `frame-src http://dots-app.localhost`. A frame opened at a version that is no longer active gets nothing, so the Apps screen opens the new one.
- **The bridge.** The host hands the page one `MessagePort`, on its first load, and answers only `{ type: "call", id, tool, input }` naming a tool the app lists; the page's `dots` has `call`, `onChange` and `onTheme`. The theme arrives as CSS variables `--dots-<token>` on the page's root, with `data-theme` set to `light` or `dark`. `dots.onChange` fires on every change of the app's data, the screen's own included. A page that loads a second time has navigated away from what Dots served: it is cut off ("The app's screen left its page"), and the person loads the app again.
- **The frame cannot reach Tauri's IPC.** Checked in WebView2 with a probe app (`tools/smoke/apps/probe/`), installed through a review card: from its page, `fetch` to Tauri's IPC, the network, an image from the web, the parent page, `localStorage` and a `data:` script are all refused, while its own files and the bridge work. WebView2 puts the scripts Tauri injects into every frame, so `__TAURI_INTERNALS__` and `chrome.webview` exist in the page; a command invoked through them gets no answer and is not carried out (the probe's `app_set_every_dot` left the switch off).
- **The preview** runs on a database in memory built from the migrations and `sample.sql`, as the person, with no `ctx.dots.start`; `ctx.dots.list()` gives the dots that use the app now (none for a new app). At most 8 previews are open; deciding a proposal closes its previews.
- **The Code tab** shows, for a change, "What it changes" against the active version (the existing `DiffView`) above every file; a new app shows every file.
- **Kept data.** Removing an app that keeps its data writes `removed.json` beside it (its name, title, the time and the migrations that ran). The Apps screen lists it under "Kept data", with "Delete data". A new app of the same name whose migrations begin with those, byte for byte, is offered to start on it: the card's "Start on the data kept from the removed <title>" is ticked, "Add to library" then moves the data to the new app and runs the migrations after those. Data deleted, or no longer fitting, by then is refused and the card waits.
- **Needs you.** A waiting card counts on the rail, in the tray and in the dot's needs-you number, and the Inbox lists it under "Apps to review" with "Open" to the card.
- **`copy_app`** (`{ app, path }`, under `ProposeApp`, both engines) copies an installed app's active version, as its version folder holds it, into a new or empty folder of the run's workspace, so a dot can change Projects or another dot's app.
- **An app's users** are the dots that tick it (all of them for "Every dot") whose policy leaves them one of its tools: the Apps screen's row, `ctx.dots.list()` and the dots an app may start. "Always allow" on an app call's approval offers `App(<app>.<tool>)` and `App(<app>)`, never a bare `App`.
- **App-started runs** say "Started by <title>" in the transcript, with the app's message under the header; the run card's label is the app's title.
- **Bounds.** The call log keeps at most 30 KB of a call's result, as of its input. An app's test files run 60 s together at most; a file past that fails without running. Accepting an app lets go of the thread lock while its migrations run and marks the thread as deciding meanwhile: a chat or a reply to that thread is refused ("Dots is still adding the app you accepted in this chat; send your message again in a moment"), a follow-up waits, and other threads go on. Removing an app deletes its topics, and topics of apps no longer installed are pruned at start.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-10-23-app-library-design.md
git commit -m "docs: spec section 17, what plan B built"
```
