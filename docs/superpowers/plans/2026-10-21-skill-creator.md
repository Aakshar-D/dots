# Skill Creator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the Library's Skills tab, a person describes a skill in words, Claude writes it through the person's own `claude` CLI with no tools, the person edits it or asks for changes, and Dots saves it as a skill in its own folder and in the Library.

**Architecture:** The core gains `skill_creator` (the draft check, the drafting instructions, the request text, the save folder, and `SkillDrafts`, which keeps at most one running draft per dialog) and `engine::claude::once::ask_once`, one request to the CLI outside a dot run that can be cancelled and has a time limit. `LibraryService` gains a skills folder (`<data>/skills`), `create_skill`, and a `remove` that deletes the folder of Dots' own skill first. The app gains four one-line commands; the UI gains a `CreateSkillDialog` opened from the Skills tab.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio and tokio-util; Tauri 2.12; Bun 1.3, React 19.3, TypeScript 6.0, TanStack Query 5.104, zod 4.6, Vitest 5.0. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-21-skill-creator-design.md` (commit `4721df5`, the base of this plan, branch `feat/skill-creator`). Task 5 adds what the prototype settled; the table "Deviations" lists it.

**Previous plans:** `docs/superpowers/plans/2026-10-15-mcp-skills-plugins.md` made the Library and its skills; this plan adds to its `LibraryService`, its Skills tab and its remove confirmation.

**Where the tasks come from:** the spec. Task 1 is saving and removing (§6, §7); Task 2 is drafting (§5); Task 3 is the app's commands and the contract; Task 4 is the dialog (§4); Task 5 is the spec; Task 6 is the smoke (§8).

**How this plan's code was checked:** the code blocks of Tasks 1 to 5 were generated from a throwaway worktree of commit `4721df5`, one commit per task, where they compiled and passed on Windows 11, and the plan was then replayed onto `4721df5` by a script that applied every block and compared the tree with each task commit (all matched; the contract fixture was taken from the task commit, as the contract command writes it). Before the first task: `cargo test -p dots-core` 712 passed, `cargo test -p dots-app` 94, `bun run --cwd ui test` 432 tests in 31 files. At the last task: `dots-core` 737, `dots-app` 94, the UI 443 in 31 files; clippy without warnings, `cargo fmt --check` and `tsc --noEmit` clean. Outside the plan, `claude --help` of the installed CLI lists `--tools` ("" for none), `--strict-mcp-config`, `--setting-sources`, `--system-prompt`, `--max-turns`, `--model` and `--no-session-persistence`; no agent ran the real CLI.

## Global Constraints

- Toolchain: Rust 1.95 stable; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/skill-creator`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\skill-creator`, based on `4721df5`. All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Every cargo command runs with `CARGO_TARGET_DIR=C:/Users/aksha/dots/target` (the shared build folder): `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target` once per shell.
- No new Cargo or npm dependency, no migration, no new Tauri capability or plugin.
- The subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials or calls an Anthropic API. No agent runs the real `claude` CLI (not even with a prompt); tests and smokes use `fake-claude`.
- Values the spec fixes: the skills folder `<data>/skills` (`runtime::SKILLS_DIR`), the drafts folder `<data>/drafts` (`runtime::DRAFTS_DIR`); the draft's command line `-p --output-format stream-json --verbose --model <model> --max-turns 1 --tools "" --strict-mcp-config --setting-sources "" --no-session-persistence --system-prompt <instructions>`, the request on stdin, and no `--mcp-config`, `--plugin-dir`, `--allowedTools` or `--resume`; a 5-minute limit; the default model `sonnet`; the copy "Create skill", "What should the skill do?", "Write it", "Claude is writing the skill…", "Ask for changes", "What should change?", "Save to Library", "Start over", "Discard this draft?", "Its folder in Dots' skills folder is deleted too.", "Could not delete <path>: <reason>. Close anything using it and try again.", "A skill named X is already in the Library.", "X already exists in Dots' skills folder".
- Nothing is overwritten: a save refuses a name the Library has for a skill and a folder that exists. A remove deletes a folder only when it resolves to a folder inside `<data>/skills`.
- No agent installs or runs an installer, runs a release build, or touches the installed Dots (`%LOCALAPPDATA%\Dots`) or the real data folder `%LOCALAPPDATA%\dots`. Smokes use a debug build with a fresh `DOTS_DATA_DIR`.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit (the blocks hold `§`, `—` and `…`). The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`, plus `open` from `@tauri-apps/plugin-dialog`. No `fetch`, no other plugin.
- Live data is parsed, not cast, with strict schemas: a type the UI parses changes only together with the contract fixture and `ui/src/api/types.ts`, in the same commit. The fixture `ui/src/api/__fixtures__/contract.json` is written by `UPDATE_CONTRACT=1 cargo test -p dots-app contract` and never by hand. `contract.rs`'s `samples()` is at the `json!` macro's recursion limit: a new sample is added after the macro (`samples["X"] = ...`).
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`), `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. Test output must be free of warnings. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first. The UI files have no formatter; keep new lines within 100 columns where the file does.
- Known on this machine: a `vitest` run started right after an install, a `tsc` run, a cargo build or a large write can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`, fail every file at once, or fail one form-typing test under load. Run it again on its own; do not change the pool settings. Run the UI tests before the typecheck.
- Known flaky under load: `runtime_local_test::live_approval_lets_the_command_run`, `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`, `claude_engine_test::a_finished_run_leaves_no_process_behind`, some `resume_test` tests that make worktrees, and `claude_gateway_test::a_claude_run_reaches_its_servers_through_the_gateway_only`. Rerun the test binary alone before treating one as a failure.
- Fact-Forcing Gate (a hook of this machine): the first Edit or Write of each file, the first Bash command after a task notification, and `rm`, `git commit --amend` or a rebase are blocked until the facts it asks for are stated (callers, affected API, data shapes, the instruction, or for a destructive command what changes and how to undo it); then retry the identical call. For a large new file, write a one-line stub first.

## Deviations from the spec

The spec at `4721df5` leaves these open or says less; Task 5 records them in a new §9.

| Spec at `4721df5` | This plan | Why |
|---|---|---|
| §6 step 1: `name` passes `check_name` | Also refused: a Windows device name (`con`, `nul`, `com1`…, also with an extension) and a name ending in `.` | `check_name` takes them, but Windows cannot make such a folder, or drops the dot |
| §5, §6: no size limits | An idea or a change at most 4,000 characters; a draft at most 64 KB (`MAX_SKILL_BYTES`) for a change request and for a save | A pasted file must not become a command line or a skill no listing can show |
| §6 step 3: "a new folder beside it" | A hidden folder `.<name>.<id>` in `<data>/skills`; startup removes hidden folders there, and `<data>/drafts` | A crash between the write and the rename leaves only what startup removes |
| §4: Start over returns to Describe with the idea kept | It also clears the draft, so closing afterwards does not ask "Discard this draft?" | The person chose to drop it |
| §8: "App: the commands and the per-dialog cancel" | No app-level tests: the commands are one-line wrappers like the others; the cancel and the errors are tested in the core, the arguments in `commands.test.ts` | Nothing is decided in the app layer |

The spec leaves to the builder, and the plan settles: drafting lives in `skill_creator.rs` (instructions, request text, `SkillDrafts`) and `engine/claude/once.rs` (the CLI call); the dialog id is a string the UI makes (`create-skill-<n>`), sent with `skill_draft` and `skill_draft_cancel`, and the core cancels a dialog's earlier draft when it starts a new one; a cancelled draft fails with "The draft was cancelled.", which the dialog drops because it counts its requests.

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. A draft whose frontmatter the Library cannot read (a description with `: ` not in quotes, a name used twice, a missing `---`). Expected: the dialog says what to change and Save stays off; a save is refused the same way. Tests: `skill_creator_test::a_draft_says_what_to_change_when_it_cannot_be_saved` (Task 1), `LibraryScreen.test.tsx` "says why a draft cannot be saved and keeps Save off" (Task 4).
2. A name Windows cannot use for a folder (`con`, `notes.`). Expected: refused before anything is written. Test: `skill_creator_test::a_name_windows_cannot_use_for_a_folder_is_refused` (Task 1).
3. Removing a skill whose `SKILL.md` is open in an editor that locks it. Expected: nothing is removed, and the error names the folder and says to close it. Test: `skill_creator_test::a_folder_that_cannot_be_deleted_keeps_the_skill` (Task 1, Windows only).
4. Closing the dialog, pressing Cancel, or asking again while Claude writes. Expected: the CLI is ended, and a late answer does not appear. Tests: `skill_draft_test::cancel_ends_the_cli_and_the_draft` and `a_new_draft_of_the_same_dialog_cancels_the_one_before` (Task 2), `LibraryScreen.test.tsx` "cancels a draft being written, goes back with the idea kept, and drops its answer" (Task 4).
5. No CLI, a CLI that is not logged in, or Dots' port taken by another program. Expected: the dialog shows the CLI's words; drafting works without Dots' local server. Tests: `skill_draft_test::without_the_cli_a_draft_says_how_to_get_it`, `an_error_an_exit_or_an_empty_answer_fails_the_draft` and `drafts_work_while_the_local_servers_port_is_taken` (Task 2), `LibraryScreen.test.tsx` "shows a failed draft where it happened" (Task 4).

## File Map

- `crates/dots-core/src/skill_creator.rs` (new, Tasks 1 to 3): the draft check, the save folder, the drafting instructions, the request text, `SkillDrafts`.
- `crates/dots-core/src/engine/claude/once.rs` (new, Task 2): `ask_once`, `once_args`.
- `crates/dots-core/src/library_service.rs` (Task 1): the skills folder, `create_skill`, `remove`, `LibraryEntry::dots_folder`.
- `crates/dots-core/src/runtime.rs`, `lib.rs`, `engine/claude/mod.rs` (Tasks 1, 2): the folders, the sweeps, `Config::draft_limit`, `Runtime::skill_drafts`.
- `apps/dots-app/src/{commands,lib,contract}.rs` (Tasks 1, 3).
- `ui/src/api/{types,commands}.ts` and the fixture (Tasks 1, 3); `ui/src/screens/library/{CreateSkillDialog,SkillsTab,parts}.tsx` (Task 4).
- Tests: `crates/dots-core/tests/skill_creator_test.rs` (new), `skill_draft_test.rs` (new); `ui/src/api/{contract,commands}.test.ts`, `ui/src/screens/library/LibraryScreen.test.tsx`.
- `docs/superpowers/specs/2026-10-21-skill-creator-design.md` (Task 5); `docs/verification/2026-10-21-skill-creator-smoke.md` (Task 6).

### Task 1: Save a draft in Dots' skills folder, and delete it on remove

The draft check (spec §6 step 1), the save (§6), and the remove that takes Dots' own skill's folder with it (§7). The library entry gains `dots_folder`, so the contract fixture and the UI's strict schema change in this task too.

**Files:**

- Create: `crates/dots-core/src/skill_creator.rs`
- Modify: `crates/dots-core/src/lib.rs`, `crates/dots-core/src/library_service.rs`, `crates/dots-core/src/runtime.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Create: `crates/dots-core/tests/skill_creator_test.rs`

**Interfaces:**

- Consumes: `kit::parse_frontmatter`, `library::check_name`, `LibraryService::add`, `Store::find_library_item`, `util::new_id`.
- Produces:
  - `dots_core::skill_creator::{check_draft(text: &str) -> Result<DraftSummary>, DraftSummary { name: String, description: String }, MAX_SKILL_BYTES, sweep_unfinished(root: &Path)}`; crate-private `write_skill_folder`, `is_inside`, `delete_skill_folder`.
  - `LibraryService::with_skills_dir(self, dir: PathBuf) -> LibraryService`, `LibraryService::create_skill(&self, text: &str) -> Result<LibraryItem>`.
  - `LibraryEntry::dots_folder: bool`; `dots_core::runtime::SKILLS_DIR = "skills"`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/skill_creator_test.rs`:**

```rust
//! The skill creator: what a draft must hold, saving it in Dots' skills folder, and removing one
//! of Dots' own skills (skill creator spec §6, §7).

mod common;

use std::path::{Path, PathBuf};

use common::{spec, temp_store};
use dots_core::events::{new_bus, Bus};
use dots_core::library::{LibraryConfig, SkillConfig};
use dots_core::library_service::LibraryService;
use dots_core::skill_creator::{check_draft, DraftSummary, MAX_SKILL_BYTES};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};

const DRAFT: &str = "---\nname: pr-tests\ndescription: \"Checks a PR for missing tests: use it \
                     on a review.\"\n---\n# Missing tests\n\nList the changed functions.\n";

fn service(store: &Store, bus: &Bus, skills: &Path) -> LibraryService {
    LibraryService::new(store.clone(), bus.clone()).with_skills_dir(skills.to_path_buf())
}

fn invalid(text: &str) -> String {
    match check_draft(text) {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn a_draft_with_a_name_a_description_and_instructions_can_be_saved() {
    assert_eq!(
        check_draft(DRAFT).unwrap(),
        DraftSummary {
            name: "pr-tests".into(),
            description: "Checks a PR for missing tests: use it on a review.".into(),
        }
    );
    // A BOM and CRLF line ends, as an editor on Windows may leave them.
    let crlf = format!("\u{feff}{}", DRAFT.replace('\n', "\r\n"));
    assert_eq!(check_draft(&crlf).unwrap().name, "pr-tests");
}

#[test]
fn a_draft_says_what_to_change_when_it_cannot_be_saved() {
    assert!(invalid("name: x\n").contains("must start with a --- line"));
    assert!(invalid("---\nname: x\ndescription: y\nBody").contains("could not be read"));
    let twice = "---\nname: a\nname: b\ndescription: d\n---\nBody\n";
    assert!(invalid(twice).contains("could not be read"));
    let colon = "---\nname: a\ndescription: Checks: things\n---\nBody\n";
    assert!(invalid(colon).contains("could not be read"));
    let quote = "---\nname: a\ndescription: \"open\n---\nBody\n";
    assert!(invalid(quote).contains("could not be read"));
    assert!(invalid("---\ndescription: d\n---\nBody\n").contains("Add a name: line"));
    assert!(invalid("---\nname: a\n---\nBody\n").contains("Add a description: line"));
    assert!(invalid("---\nname: a\ndescription: d\n---\n\n  \n").contains("instructions"));
    assert!(invalid("---\nname: a b\ndescription: d\n---\nBody\n").contains("skill name"));
}

#[test]
fn a_name_windows_cannot_use_for_a_folder_is_refused() {
    for name in ["con", "NUL", "com1.md", "lpt9", "notes."] {
        let text = format!("---\nname: {name}\ndescription: d\n---\nBody\n");
        assert!(invalid(&text).contains("cannot be a folder name"), "{name}");
    }
    let text = "---\nname: console\ndescription: d\n---\nBody\n";
    assert_eq!(check_draft(text).unwrap().name, "console");
}

#[test]
fn a_draft_longer_than_the_limit_is_refused() {
    let long = format!("{DRAFT}{}", "x".repeat(MAX_SKILL_BYTES));
    assert!(invalid(&long).contains("longer than 64 KB"));
}

#[tokio::test]
async fn a_saved_draft_is_a_skill_in_dots_skills_folder() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let item = lib.create_skill(DRAFT).await.unwrap();
    assert_eq!(item.name, "pr-tests");
    let folder = skills.join("pr-tests");
    assert_eq!(
        item.config,
        LibraryConfig::Skill(SkillConfig {
            path: folder.to_string_lossy().into_owned()
        })
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
        DRAFT
    );
    // Only the skill's folder is left: no hidden folder of the write.
    let names: Vec<String> = std::fs::read_dir(&skills)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["pr-tests"]);
    let list = lib.list().await.unwrap();
    assert!(list[0].dots_folder);
    assert_eq!(
        list[0].skill.as_ref().unwrap().description,
        "Checks a PR for missing tests: use it on a review."
    );
}

#[tokio::test]
async fn a_name_or_folder_that_exists_is_refused_and_nothing_is_overwritten() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    lib.create_skill(DRAFT).await.unwrap();
    let again = DRAFT.replace("List the changed", "Name the changed");
    let err = lib.create_skill(&again).await.unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("already in the Library")),
        "{err}"
    );
    // A folder of that name without a Library row: a person's own, or a save cut short.
    std::fs::create_dir_all(skills.join("csv-tools")).unwrap();
    std::fs::write(skills.join("csv-tools").join("notes.txt"), "mine").unwrap();
    let csv = DRAFT.replace("name: pr-tests", "name: csv-tools");
    let err = lib.create_skill(&csv).await.unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("already exists in Dots' skills folder")),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(skills.join("pr-tests").join("SKILL.md")).unwrap(),
        DRAFT
    );
    assert!(!skills.join("csv-tools").join("SKILL.md").exists());
    assert_eq!(store.list_library_items().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_add_leaves_no_folder_behind() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse BEFORE INSERT ON library_items \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    lib.create_skill(DRAFT).await.unwrap_err();
    assert!(!skills.join("pr-tests").exists());
}

#[tokio::test]
async fn without_a_skills_folder_nothing_is_saved() {
    let (_dir, store) = temp_store().await;
    let lib = LibraryService::new(store.clone(), new_bus());
    assert!(lib.create_skill(DRAFT).await.is_err());
    assert!(store.list_library_items().await.unwrap().is_empty());
}

#[tokio::test]
async fn removing_dots_own_skill_deletes_its_folder_and_no_other() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let own = lib.create_skill(DRAFT).await.unwrap();
    let theirs_dir = dir.path().join("mine").join("csv");
    std::fs::create_dir_all(&theirs_dir).unwrap();
    std::fs::write(
        theirs_dir.join("SKILL.md"),
        "---\nname: csv\ndescription: d\n---\nBody\n",
    )
    .unwrap();
    let theirs = lib
        .add(
            "csv",
            LibraryConfig::Skill(SkillConfig {
                path: theirs_dir.to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    let list = lib.list().await.unwrap();
    let flag = |name: &str| {
        list.iter()
            .find(|e| e.item.name == name)
            .unwrap()
            .dots_folder
    };
    assert!(flag("pr-tests") && !flag("csv"));

    lib.remove(&own.id, false).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
    assert!(skills.exists());
    lib.remove(&theirs.id, false).await.unwrap();
    assert!(theirs_dir.join("SKILL.md").exists());
    // The name can be made again.
    lib.create_skill(DRAFT).await.unwrap();
}

#[tokio::test]
async fn a_used_skill_is_not_deleted_until_the_person_confirms() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let own = lib.create_skill(DRAFT).await.unwrap();
    let mut s = spec("reviewer");
    s.uses.skills = vec!["pr-tests".into()];
    store.create_dot(&s).await.unwrap();
    assert!(matches!(
        lib.remove(&own.id, false).await,
        Err(Error::Conflict(_))
    ));
    assert!(skills.join("pr-tests").join("SKILL.md").exists());
    lib.remove(&own.id, true).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_folder_that_cannot_be_deleted_keeps_the_skill() {
    use std::os::windows::fs::OpenOptionsExt;

    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let own = lib.create_skill(DRAFT).await.unwrap();
    // Open with no sharing, as an editor may: Windows refuses to delete it meanwhile.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(skills.join("pr-tests").join("SKILL.md"))
        .unwrap();
    let err = lib.remove(&own.id, false).await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("Could not delete"), "{text}");
    assert!(text.contains("pr-tests"), "{text}");
    assert!(text.contains("Close anything using it"), "{text}");
    assert!(store.get_library_item(&own.id).await.is_ok());
    drop(held);
    lib.remove(&own.id, false).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
}

#[tokio::test]
async fn starting_dots_removes_saves_a_crash_cut_short() {
    let dir = tempfile::tempdir().unwrap();
    let data: PathBuf = dir.path().join("data");
    let skills = data.join("skills");
    std::fs::create_dir_all(skills.join(".pr-tests.01J0X")).unwrap();
    std::fs::create_dir_all(skills.join("csv")).unwrap();
    let mut cfg = Config::new(&data);
    cfg.port = 0;
    let _rt = Runtime::start(cfg).await.unwrap();
    assert!(!skills.join(".pr-tests.01J0X").exists());
    assert!(skills.join("csv").exists());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test skill_creator_test`
Expected: FAIL to compile: no `skill_creator` in `dots_core`.

- [ ] **Step 3: Add the check, the save and the remove**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        problem: None,
    };
```

**with:**

```rust
        problem: None,
        dots_folder: false,
    };
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            problem: Some("SKILL.md not found".into()),
            ..bare(skill, vec![])
```

**with:**

```rust
            problem: Some("SKILL.md not found".into()),
            dots_folder: true,
            ..bare(skill, vec![])
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod settings;
pub mod store;
```

**with:**

```rust
pub mod settings;
pub mod skill_creator;
pub mod store;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    check_name, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
};
```

**with:**

```rust
    check_name, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
    SkillConfig,
};
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
use crate::model::{DotRef, ToolsChanged};
use crate::store::Store;
```

**with:**

```rust
use crate::model::{DotRef, ToolsChanged};
use crate::skill_creator::{self, check_draft};
use crate::store::Store;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    pub problem: Option<String>,
}
```

**with:**

```rust
    pub problem: Option<String>,
    /// A skill whose folder is in Dots' skills folder: removing it deletes the folder too
    /// (skill creator spec §7).
    pub dots_folder: bool,
}
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    bus: Bus,
}
```

**with:**

```rust
    bus: Bus,
    skills_dir: Option<PathBuf>,
}
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    pub fn new(store: Store, bus: Bus) -> LibraryService {
        LibraryService { store, bus }
    }
```

**with:**

```rust
    pub fn new(store: Store, bus: Bus) -> LibraryService {
        LibraryService {
            store,
            bus,
            skills_dir: None,
        }
    }

    /// Where the skill creator saves skills (`<data>/skills`, skill creator spec §6). Without
    /// it, `create_skill` fails and no skill is Dots' own.
    pub fn with_skills_dir(mut self, dir: PathBuf) -> LibraryService {
        self.skills_dir = Some(dir);
        self
    }

    /// Whether the skill folder `path` is in Dots' skills folder.
    fn is_own_folder(skills_dir: Option<&Path>, path: &str) -> bool {
        skills_dir.is_some_and(|root| skill_creator::is_inside(Path::new(path), root))
    }
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
            .collect();
        let read = blocking(move || {
```

**with:**

```rust
            .collect();
        let skills_dir = self.skills_dir.clone();
        let read = blocking(move || {
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                .map(|(kind, path)| {
                    let dir = PathBuf::from(&path);
                    match kind {
                        LibraryKind::Skill => Some(
```

**with:**

```rust
                .map(|(kind, path)| {
                    let own = kind == LibraryKind::Skill
                        && Self::is_own_folder(skills_dir.as_deref(), &path);
                    let dir = PathBuf::from(&path);
                    let read = match kind {
                        LibraryKind::Skill => Some(
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                        LibraryKind::McpServer => None,
                    }
                })
```

**with:**

```rust
                        LibraryKind::McpServer => None,
                    };
                    (read, own)
                })
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        let mut entries = Vec::new();
        for (item, read) in items.iter().zip(read) {
            let used_by = self
```

**with:**

```rust
        let mut entries = Vec::new();
        for (item, (read, dots_folder)) in items.iter().zip(read) {
            let used_by = self
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                problem: None,
            };
```

**with:**

```rust
                problem: None,
                dots_folder,
            };
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        }
        let owned = self.store.owned_library_items(id).await?;
```

**with:**

```rust
        }
        // Dots' own skill takes its folder with it, first: a folder that cannot be deleted
        // keeps the row, so its name is never left taken by a folder (skill creator spec §7).
        if let LibraryConfig::Skill(c) = &item.config {
            let skills_dir = self.skills_dir.clone();
            let dir = PathBuf::from(&c.path);
            blocking(move || {
                if Self::is_own_folder(skills_dir.as_deref(), &dir.to_string_lossy()) {
                    skill_creator::delete_skill_folder(&dir)
                } else {
                    Ok(())
                }
            })
            .await??;
        }
        let owned = self.store.owned_library_items(id).await?;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        Ok(catalog)
    }
```

**with:**

```rust
        Ok(catalog)
    }

    /// Saves a drafted `SKILL.md` as a new skill (`skill_create`, skill creator spec §6): to
    /// `<skills folder>/<name>/SKILL.md`, then into the Library. A name the Library has for a
    /// skill, or a folder of that name, is refused, so nothing is overwritten; a failed add
    /// removes the folder again.
    pub async fn create_skill(&self, text: &str) -> Result<LibraryItem> {
        let summary = check_draft(text)?;
        let Some(root) = self.skills_dir.clone() else {
            return Err(Error::Other("Dots has no skills folder".into()));
        };
        let name = summary.name;
        if self
            .store
            .find_library_item(LibraryKind::Skill, &name)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "A skill named {name} is already in the Library."
            )));
        }
        let (folder_name, body) = (name.clone(), text.to_string());
        let dir = blocking(move || skill_creator::write_skill_folder(&root, &folder_name, &body))
            .await??;
        let config = LibraryConfig::Skill(SkillConfig {
            path: dir.to_string_lossy().into_owned(),
        });
        match self.add(&name, config, None).await {
            Ok(item) => Ok(item),
            Err(e) => {
                let _ = blocking(move || std::fs::remove_dir_all(&dir)).await;
                Err(e)
            }
        }
    }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::{Error, Result};

```

**with:**

```rust
use crate::{Error, Result};

/// The data folder's folder of the skills the skill creator saves (skill creator spec §6).
pub const SKILLS_DIR: &str = "skills";

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        crate::run_tools::sweep_plugin_folders(&cfg.data_dir.join("plugins"));
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
```

**with:**

```rust
        crate::run_tools::sweep_plugin_folders(&cfg.data_dir.join("plugins"));
        // No skill is being saved yet: hidden folders are saves a crash cut short.
        crate::skill_creator::sweep_unfinished(&cfg.data_dir.join(SKILLS_DIR));
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        Ok(Runtime {
            library: LibraryService::new(store.clone(), bus.clone()),
            store,
```

**with:**

```rust
        Ok(Runtime {
            library: LibraryService::new(store.clone(), bus.clone())
                .with_skills_dir(cfg.data_dir.join(SKILLS_DIR)),
            store,
```

**Create `crates/dots-core/src/skill_creator.rs`:**

```rust
//! The skill creator (skill creator spec): what a drafted `SKILL.md` must hold to be saved, and
//! the folder Dots keeps the skills it makes in.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::kit::parse_frontmatter;
use crate::library::{check_name, LibraryKind};
use crate::{Error, Result};

/// The most a `SKILL.md` the creator saves may hold, in bytes.
pub const MAX_SKILL_BYTES: usize = 64 * 1024;

/// What the Library will show for a draft that can be saved (`skill_check`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftSummary {
    pub name: String,
    pub description: String,
}

/// Names Windows keeps for devices, also with an extension: no folder can have them.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

/// A name that is a valid Library name but no folder name on Windows: a device name, or one
/// ending in `.`, which Windows drops.
fn not_a_folder_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    name.ends_with('.') || RESERVED.contains(&stem.as_str())
}

/// Whether `text` can be saved as a skill (skill creator spec §6, step 1): it opens with a
/// frontmatter that the Library can read, with a `name` the Library takes for a skill and that
/// can be a folder name, and a `description`, and it has instructions below. `Invalid` says
/// what to change.
pub fn check_draft(text: &str) -> Result<DraftSummary> {
    if text.len() > MAX_SKILL_BYTES {
        return invalid(format!(
            "The skill is longer than {} KB; shorten it.",
            MAX_SKILL_BYTES / 1024
        ));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.lines().next().is_none_or(|l| l.trim_end() != "---") {
        return invalid(
            "The skill must start with a --- line, then its name and description, then another \
             --- line.",
        );
    }
    let parsed = parse_frontmatter(text);
    if parsed.fields.is_empty() {
        return invalid(
            "The lines between the --- lines could not be read. Check for a missing closing --- \
             line or quote and a name used twice, and put a description that holds \": \" in \
             double quotes.",
        );
    }
    let field = |key: &str| {
        parsed
            .fields
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };
    let Some(name) = field("name") else {
        return invalid("Add a name: line between the --- lines.");
    };
    check_name(LibraryKind::Skill, name)?;
    if not_a_folder_name(name) {
        return invalid(format!(
            "The name {name:?} cannot be a folder name on Windows; pick another."
        ));
    }
    let Some(description) = field("description") else {
        return invalid("Add a description: line between the --- lines.");
    };
    if parsed.body.trim().is_empty() {
        return invalid("Add the skill's instructions below the second --- line.");
    }
    Ok(DraftSummary {
        name: name.to_string(),
        description: description.to_string(),
    })
}

/// Writes `text` to `<root>/<name>/SKILL.md` (spec §6, step 3): into a hidden folder beside it
/// first, then renamed into place, so a failed write leaves no half-written skill. `Conflict`
/// when `<root>/<name>` exists; nothing is overwritten.
pub(crate) fn write_skill_folder(root: &Path, name: &str, text: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    let dir = root.join(name);
    if dir.exists() {
        return Err(Error::Conflict(format!(
            "{name} already exists in Dots' skills folder ({}).",
            dir.display()
        )));
    }
    let tmp = root.join(format!(".{name}.{}", crate::util::new_id()));
    std::fs::create_dir(&tmp)?;
    let written =
        std::fs::write(tmp.join("SKILL.md"), text).and_then(|()| std::fs::rename(&tmp, &dir));
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    Ok(dir)
}

/// True when `dir` is a folder inside `root`, compared after resolving both: a skill there is
/// Dots' own, and removing it deletes its folder (spec §7). False when either cannot be resolved.
pub(crate) fn is_inside(dir: &Path, root: &Path) -> bool {
    match (dir.canonicalize(), root.canonicalize()) {
        (Ok(dir), Ok(root)) => dir != root && dir.starts_with(&root),
        _ => false,
    }
}

/// Deletes the folder of one of Dots' own skills. The error names the folder (spec §7).
pub(crate) fn delete_skill_folder(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Other(format!(
            "Could not delete {}: {e}. Close anything using it and try again.",
            dir.display()
        ))),
    }
}

/// Removes what a save that never finished left in `root`: its hidden folders. For startup,
/// when no save is running.
pub fn sweep_unfinished(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
```

**In `ui/src/api/types.ts`, replace:**

```ts
  problem: nullable(z.string()),
});
```

**with:**

```ts
  problem: nullable(z.string()),
  // A skill whose folder is in Dots' skills folder: removing it deletes the folder too.
  dots_folder: z.boolean(),
});
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: PASS both; the fixture's library entries gain `"dots_folder"` (`true` on the skill `pdf`).

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-core --test skill_creator_test`
Expected: PASS: 12 tests (11 on a system other than Windows).

- [ ] **Step 6: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 724 passed, `dots-app` 94; the UI 432 in 31 files; no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/src/skill_creator.rs crates/dots-core/src/lib.rs crates/dots-core/src/library_service.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/skill_creator_test.rs apps/dots-app/src/contract.rs ui/src/api/types.ts ui/src/api/__fixtures__/contract.json
git commit -m "feat(core): save a drafted skill in Dots' skills folder and delete it on remove"
```

### Task 2: Draft a skill with one request to the claude CLI

`ask_once` runs the CLI once with the spec's command line (§5) and returns the answer's text; `SkillDrafts` adds the drafting instructions, the request text, a folder of its own under `<data>/drafts`, the time limit and the per-dialog cancel.

**Files:**

- Create: `crates/dots-core/src/engine/claude/once.rs`
- Modify: `crates/dots-core/src/engine/claude/mod.rs`, `crates/dots-core/src/skill_creator.rs`, `crates/dots-core/src/runtime.rs`
- Create: `crates/dots-core/tests/skill_draft_test.rs`

**Interfaces:**

- Consumes: `engine::claude::parse::parse_line`, `proc::{Job, hide_window, read_tail}`, the runtime's `claude_program` and `Config::claude_env`, Task 1's `MAX_SKILL_BYTES`.
- Produces:
  - `dots_core::engine::claude::once::{ask_once(req: &Once<'_>, cancel: &CancellationToken) -> Result<String, OnceFailure>, once_args(model: &str, system_prompt: &str) -> Vec<String>, Once, OnceFailure::{Cancelled, TimedOut, Failed(String)}}`.
  - `dots_core::skill_creator::{DraftRequest { idea: String, model: String, current: Option<String>, change: Option<String> }, draft_prompt(&DraftRequest) -> Result<String>, unwrap_fence(&str) -> String, DRAFTING_INSTRUCTIONS, DRAFT_LIMIT, MAX_REQUEST_CHARS, DEFAULT_MODEL, SkillDrafts}` with `SkillDrafts::draft(&self, dialog: &str, req: &DraftRequest) -> Result<String>` and `SkillDrafts::cancel(&self, dialog: &str)`.
  - `Config::draft_limit: Duration`, `Runtime::skill_drafts(&self) -> &SkillDrafts`, `runtime::DRAFTS_DIR = "drafts"`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/skill_draft_test.rs`:**

````rust
//! Drafting a skill with the `claude` CLI (skill creator spec §5), against the fake CLI.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dots_core::skill_creator::{
    draft_prompt, unwrap_fence, DraftRequest, DRAFTING_INSTRUCTIONS, MAX_REQUEST_CHARS,
};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

const DRAFT: &str = "---\nname: pr-tests\ndescription: \"Checks a PR for missing tests.\"\n---\n\
                     # Missing tests\n\nList the changed functions.\n";

struct Rt {
    dir: TempDir,
    rt: Arc<Runtime>,
    args_out: PathBuf,
}

impl Rt {
    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// What the fake CLI recorded: its arguments, stdin and folder.
    fn recorded(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.args_out).unwrap()).unwrap()
    }
}

async fn start_with(script: Value, change: impl FnOnce(&mut Config)) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script_path.to_string_lossy().to_string(),
    ));
    change(&mut cfg);
    let rt = Arc::new(Runtime::start(cfg).await.unwrap());
    Rt { dir, rt, args_out }
}

async fn start(script: Value) -> Rt {
    start_with(script, |_| {}).await
}

/// A script that answers `text` as the result.
fn answering(text: &str) -> Value {
    json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": text,
                  "usage": {"input_tokens": 1, "output_tokens": 1}}},
    ])
}

fn idea(text: &str) -> DraftRequest {
    DraftRequest {
        idea: text.into(),
        model: "haiku".into(),
        current: None,
        change: None,
    }
}

fn message(err: Error) -> String {
    err.to_string()
}

#[tokio::test]
async fn a_draft_asks_the_cli_once_with_no_tools_and_returns_its_answer() {
    let t = start(answering(DRAFT)).await;
    let text =
        t.rt.skill_drafts()
            .draft("d1", &idea("Check a PR for missing tests"))
            .await
            .unwrap();
    assert_eq!(text, DRAFT);
    let rec = t.recorded();
    let args: Vec<String> = rec["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let pair = |name: &str| {
        let i = args
            .iter()
            .position(|a| a == name)
            .unwrap_or_else(|| panic!("{name}"));
        args[i + 1].clone()
    };
    assert_eq!(args[0], "-p");
    assert_eq!(pair("--output-format"), "stream-json");
    assert!(args.contains(&"--verbose".to_string()));
    assert_eq!(pair("--model"), "haiku");
    assert_eq!(pair("--max-turns"), "1");
    assert_eq!(pair("--tools"), "");
    assert!(args.contains(&"--strict-mcp-config".to_string()));
    assert_eq!(pair("--setting-sources"), "");
    assert!(args.contains(&"--no-session-persistence".to_string()));
    assert_eq!(pair("--system-prompt"), DRAFTING_INSTRUCTIONS);
    for absent in [
        "--mcp-config",
        "--plugin-dir",
        "--allowedTools",
        "--disallowedTools",
        "--permission-prompt-tool",
        "--resume",
    ] {
        assert!(!args.contains(&absent.to_string()), "{absent}");
    }
    assert!(rec["mcp_config"].is_null());
    assert!(rec["prompt"]
        .as_str()
        .unwrap()
        .contains("Check a PR for missing tests"));
    // It ran in a folder of its own under the data folder's drafts, gone afterwards.
    let cwd = PathBuf::from(rec["cwd"].as_str().unwrap());
    assert!(
        cwd.parent().unwrap().ends_with("data/drafts"),
        "{}",
        cwd.display()
    );
    assert!(!cwd.exists());
    assert!(t.data().join("drafts").is_dir());
}

#[tokio::test]
async fn a_change_sends_the_idea_the_current_text_and_the_change() {
    let t = start(answering(DRAFT)).await;
    let req = DraftRequest {
        idea: "Check a PR for missing tests".into(),
        model: "sonnet".into(),
        current: Some("---\nname: old\n---\nMy hand edit\n".into()),
        change: Some("Mention snapshot tests".into()),
    };
    t.rt.skill_drafts().draft("d1", &req).await.unwrap();
    let prompt = t.recorded()["prompt"].as_str().unwrap().to_string();
    assert!(prompt.contains("Check a PR for missing tests"));
    assert!(prompt.contains("<<<SKILL.md\n---\nname: old\n---\nMy hand edit\nSKILL.md>>>"));
    assert!(prompt.contains("Change it like this:\n\nMention snapshot tests"));
    assert!(prompt.contains("whole revised SKILL.md"));
}

#[tokio::test]
async fn a_fenced_answer_is_unwrapped() {
    let t = start(answering(&format!("```markdown\n{DRAFT}```"))).await;
    let text = t.rt.skill_drafts().draft("d1", &idea("x")).await.unwrap();
    assert_eq!(text, DRAFT);
}

#[test]
fn only_an_answer_that_is_one_fenced_block_loses_its_fence() {
    assert_eq!(unwrap_fence(&format!("\n\n{DRAFT}\n\n")), DRAFT);
    assert_eq!(unwrap_fence(&format!("```\n{DRAFT}```")), DRAFT);
    assert_eq!(unwrap_fence(&format!("```md\n{DRAFT}\n```\n")), DRAFT);
    // A fence inside the instructions stays, and so does the text around a block.
    let inside = format!("{DRAFT}\n```sh\ncargo test\n```\n");
    assert_eq!(unwrap_fence(&inside), inside.trim().to_string() + "\n");
    let two = "```\na\n```\ntext\n```\nb\n```";
    assert_eq!(unwrap_fence(two), format!("{two}\n"));
}

#[tokio::test]
async fn an_error_an_exit_or_an_empty_answer_fails_the_draft() {
    let failed = json!([
        {"emit": {"type": "result", "subtype": "error_during_execution", "is_error": true,
                  "result": "Invalid API key", "usage": {}}},
    ]);
    let t = start(failed).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("Claude could not write the skill"), "{err}");
    assert!(err.contains("Invalid API key"), "{err}");

    let exited = json!([{"stderr": "Please run /login"}, {"exit": 1}]);
    let t = start(exited).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("exited with code 1"), "{err}");
    assert!(err.contains("Please run /login"), "{err}");

    let t = start(answering("  \n")).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("no text"), "{err}");
}

/// Waits until the fake CLI has recorded its start.
async fn started(t: &Rt) {
    let end = Instant::now() + Duration::from_secs(20);
    while !t.args_out.exists() {
        assert!(Instant::now() < end, "the CLI did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn cancel_ends_the_cli_and_the_draft() {
    let t = start(json!([{"sleep_ms": 30000}, {"emit": {"type": "result",
        "subtype": "success", "is_error": false, "result": DRAFT}}]))
    .await;
    let rt = t.rt.clone();
    let task = tokio::spawn(async move { rt.skill_drafts().draft("d1", &idea("x")).await });
    started(&t).await;
    let begun = Instant::now();
    t.rt.skill_drafts().cancel("d1");
    let err = message(task.await.unwrap().unwrap_err());
    assert!(err.contains("cancelled"), "{err}");
    assert!(begun.elapsed() < Duration::from_secs(10));
    // A cancel with nothing running does nothing.
    t.rt.skill_drafts().cancel("d1");
}

#[tokio::test]
async fn a_draft_that_takes_too_long_is_ended() {
    let t = start_with(json!([{"sleep_ms": 30000}]), |cfg| {
        cfg.draft_limit = Duration::from_millis(500)
    })
    .await;
    let begun = Instant::now();
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("took too long"), "{err}");
    assert!(begun.elapsed() < Duration::from_secs(15));
}

#[tokio::test]
async fn a_new_draft_of_the_same_dialog_cancels_the_one_before() {
    let t = start(json!([{"sleep_ms": 30000}])).await;
    let rt = t.rt.clone();
    let first = tokio::spawn(async move { rt.skill_drafts().draft("d1", &idea("x")).await });
    started(&t).await;
    std::fs::remove_file(&t.args_out).unwrap();
    // Another dialog's draft is left alone.
    let rt = t.rt.clone();
    let other = tokio::spawn(async move { rt.skill_drafts().draft("d2", &idea("y")).await });
    started(&t).await;
    std::fs::remove_file(&t.args_out).unwrap();
    let rt = t.rt.clone();
    let second = tokio::spawn(async move { rt.skill_drafts().draft("d1", &idea("z")).await });
    let err = message(first.await.unwrap().unwrap_err());
    assert!(err.contains("cancelled"), "{err}");
    assert!(!other.is_finished());
    t.rt.skill_drafts().cancel("d1");
    t.rt.skill_drafts().cancel("d2");
    assert!(second.await.unwrap().is_err());
    assert!(other.await.unwrap().is_err());
}

#[tokio::test]
async fn a_request_that_cannot_be_sent_is_refused_before_the_cli_starts() {
    let t = start(answering(DRAFT)).await;
    let drafts = t.rt.skill_drafts();
    let refused = |r: Result<String, Error>| match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    };
    assert!(refused(drafts.draft("d", &idea("  ")).await).contains("Describe the skill"));
    let mut no_model = idea("x");
    no_model.model = " ".into();
    assert!(refused(drafts.draft("d", &no_model).await).contains("Pick a model"));
    let mut lone = idea("x");
    lone.change = Some("shorter".into());
    assert!(refused(drafts.draft("d", &lone).await).contains("what should change"));
    let mut blank = idea("x");
    blank.current = Some(DRAFT.into());
    blank.change = Some(" ".into());
    assert!(refused(drafts.draft("d", &blank).await).contains("what should change"));
    let long = idea(&"x".repeat(MAX_REQUEST_CHARS + 1));
    assert!(refused(drafts.draft("d", &long).await).contains("at most 4000"));
    assert!(!t.args_out.exists());
}

#[test]
fn the_instructions_name_the_rules_a_draft_is_checked_against() {
    for rule in [
        "complete SKILL.md and nothing else",
        "the first line is ---",
        "name: <name>",
        "lowercase letters, digits and hyphens",
        "at most 64 characters",
        "description: \"",
        "double quotes",
        "No lists, no indented lines, no other keys",
        "whole revised file",
    ] {
        assert!(DRAFTING_INSTRUCTIONS.contains(rule), "{rule}");
    }
    let req = idea("Check a PR");
    assert_eq!(
        draft_prompt(&req).unwrap(),
        "Write a skill for this idea:\n\nCheck a PR\n"
    );
}

#[tokio::test]
async fn without_the_cli_a_draft_says_how_to_get_it() {
    let t = start_with(answering(DRAFT), |cfg| {
        cfg.claude_path = Some(PathBuf::from("C:/no/such/claude.exe"))
    })
    .await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("claude CLI not found"), "{err}");
}

#[tokio::test]
async fn drafts_work_while_the_local_servers_port_is_taken() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let t = start_with(answering(DRAFT), |cfg| {
        cfg.port = port;
        cfg.bind_wait = Duration::from_millis(100);
    })
    .await;
    assert!(t.rt.status().await.server_error.is_some());
    let text = t.rt.skill_drafts().draft("d1", &idea("x")).await.unwrap();
    assert_eq!(text, DRAFT);
}

#[tokio::test]
async fn starting_dots_removes_drafts_folders_a_crash_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let left = data.join("drafts").join("01J0LEFT");
    std::fs::create_dir_all(&left).unwrap();
    let mut cfg = Config::new(&data);
    cfg.port = 0;
    let _rt = Runtime::start(cfg).await.unwrap();
    assert!(!left.exists());
    assert!(!Path::new(&data.join("drafts")).exists());
}
````

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test skill_draft_test`
Expected: FAIL to compile: no `draft_prompt`, `DraftRequest` … in `dots_core::skill_creator`, no field `draft_limit` on `Config`, no method `skill_drafts` on `Runtime`.

- [ ] **Step 3: Add the request, the drafts and the runtime's part**

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
pub mod args;
pub mod parse;
```

**with:**

```rust
pub mod args;
pub mod once;
pub mod parse;
```

**Create `crates/dots-core/src/engine/claude/once.rs`:**

```rust
//! One request to the `claude` CLI outside a dot run (skill creator spec §5): no tools, no MCP
//! servers, no settings files and no saved session. The answer's text comes back.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::parse;
use crate::engine::EngineEvent;

const STDERR_TAIL: usize = 4096;

/// A request: the CLI, its environment and folder, and what to ask.
pub struct Once<'a> {
    pub program: &'a Path,
    pub env: &'a [(String, String)],
    pub cwd: &'a Path,
    pub model: &'a str,
    pub system_prompt: &'a str,
    /// Sent on stdin.
    pub prompt: &'a str,
    /// The process is ended after this long.
    pub limit: Duration,
}

/// Why a request gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnceFailure {
    Cancelled,
    TimedOut,
    /// The CLI failed: its error, or how it exited.
    Failed(String),
}

/// The command line of a request: one turn, no built-in tools (`--tools ""`), no MCP servers
/// (`--strict-mcp-config` without `--mcp-config`), none of the person's settings, hooks or
/// `CLAUDE.md` files (`--setting-sources ""`), and the default system prompt replaced.
pub fn once_args(model: &str, system_prompt: &str) -> Vec<String> {
    [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        model,
        "--max-turns",
        "1",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--system-prompt",
        system_prompt,
    ]
    .map(String::from)
    .to_vec()
}

enum End {
    Exited(Option<std::process::ExitStatus>),
    Cancelled,
    TimedOut,
}

/// What the output said: the result's text, the error, and the last assistant text.
#[derive(Default)]
struct Read {
    answer: Option<String>,
    failure: Option<String>,
    last_text: Option<String>,
}

/// Runs the request and returns the answer: the result's text, else the last assistant text.
/// Cancelling `cancel`, or `limit` passing, ends the CLI and everything it started.
pub async fn ask_once(req: &Once<'_>, cancel: &CancellationToken) -> Result<String, OnceFailure> {
    let mut cmd = Command::new(req.program);
    cmd.args(once_args(req.model, req.system_prompt))
        .current_dir(req.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in req.env {
        cmd.env(k, v);
    }
    crate::proc::hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        OnceFailure::Failed(format!("failed to start {}: {e}", req.program.display()))
    })?;
    let job = crate::proc::Job::assign(&child).unwrap_or_else(|e| {
        tracing::warn!("no job object for claude, using kill_tree: {e}");
        crate::proc::Job::fallback(&child)
    });
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let prompt = req.prompt.to_string();
    tokio::spawn(async move {
        if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
            tracing::warn!("writing the request to claude failed: {e}");
        }
        drop(stdin);
    });
    let mut reading = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();
        let mut read = Read::default();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&buf);
            for ev in parse::parse_line(text.trim_end_matches(['\r', '\n'])) {
                match ev {
                    EngineEvent::Finished { summary } => read.answer = Some(summary),
                    EngineEvent::Failed { error } => read.failure = Some(error),
                    EngineEvent::AssistantText { text } => read.last_text = Some(text),
                    _ => {}
                }
            }
        }
        read
    });
    let mut stderr_task = tokio::spawn(crate::proc::read_tail(stderr, STDERR_TAIL));

    let end = tokio::select! {
        s = child.wait() => End::Exited(s.ok()),
        _ = cancel.cancelled() => End::Cancelled,
        _ = tokio::time::sleep(req.limit) => End::TimedOut,
    };
    if !matches!(end, End::Exited(_)) {
        job.terminate().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    // Whatever claude left running goes now: it would hold the pipes read below open.
    job.close();
    let cap = Duration::from_secs(5);
    let read = match tokio::time::timeout(cap, &mut reading).await {
        Ok(Ok(read)) => read,
        _ => {
            reading.abort();
            Read::default()
        }
    };
    let tail = match tokio::time::timeout(cap, &mut stderr_task).await {
        Ok(Ok(t)) => t,
        _ => {
            stderr_task.abort();
            String::new()
        }
    };
    let status = match end {
        End::Cancelled => return Err(OnceFailure::Cancelled),
        End::TimedOut => return Err(OnceFailure::TimedOut),
        End::Exited(status) => status,
    };
    if let Some(error) = read.failure {
        return Err(OnceFailure::Failed(error));
    }
    match (read.answer, read.last_text) {
        (Some(answer), _) if !answer.trim().is_empty() => Ok(answer),
        (Some(_), last) => Ok(last.unwrap_or_default()),
        (None, _) => {
            let code = status
                .and_then(|s| s.code())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Err(OnceFailure::Failed(if tail.trim().is_empty() {
                format!("claude exited with code {code} without a result")
            } else {
                format!("claude exited with code {code}: {}", tail.trim())
            }))
        }
    }
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::settings::Settings;
use crate::store::Store;
```

**with:**

```rust
use crate::settings::Settings;
use crate::skill_creator::SkillDrafts;
use crate::store::Store;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
pub const SKILLS_DIR: &str = "skills";

```

**with:**

```rust
pub const SKILLS_DIR: &str = "skills";

/// The data folder's folder where each skill draft's CLI starts (skill creator spec §5).
pub const DRAFTS_DIR: &str = "drafts";

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub clock: Arc<dyn Clock>,
}
```

**with:**

```rust
    pub clock: Arc<dyn Clock>,
    /// How long a skill draft may take (skill creator spec §5). A test shortens it.
    pub draft_limit: Duration,
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            clock: Arc::new(SystemClock),
        }
```

**with:**

```rust
            clock: Arc::new(SystemClock),
            draft_limit: crate::skill_creator::DRAFT_LIMIT,
        }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    library: LibraryService,
    clock: Arc<dyn Clock>,
```

**with:**

```rust
    library: LibraryService,
    skill_drafts: SkillDrafts,
    clock: Arc<dyn Clock>,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        crate::skill_creator::sweep_unfinished(&cfg.data_dir.join(SKILLS_DIR));
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
```

**with:**

```rust
        crate::skill_creator::sweep_unfinished(&cfg.data_dir.join(SKILLS_DIR));
        // Nor is a draft being written: the drafts' folders are what a crash left behind.
        let _ = std::fs::remove_dir_all(cfg.data_dir.join(DRAFTS_DIR));
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            server_error,
            claude_program,
```

**with:**

```rust
            server_error,
            // Drafts need the CLI, not the local server: they work while its port is taken.
            skill_drafts: SkillDrafts::new(
                claude_program.clone(),
                cfg.claude_env.clone(),
                cfg.data_dir.join(DRAFTS_DIR),
                cfg.draft_limit,
                shutdown.clone(),
            ),
            claude_program,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    /// The dots' notes and questions: what the `message_*` commands call (check-ins spec
```

**with:**

```rust

    /// The skill creator's drafts: what `skill_draft` and `skill_draft_cancel` call (skill
    /// creator spec §5).
    pub fn skill_drafts(&self) -> &SkillDrafts {
        &self.skill_drafts
    }

    /// The dots' notes and questions: what the `message_*` commands call (check-ins spec
```

**In `crates/dots-core/src/skill_creator.rs`, replace:**

```rust
//! The skill creator (skill creator spec): what a drafted `SKILL.md` must hold to be saved, and
//! the folder Dots keeps the skills it makes in.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::kit::parse_frontmatter;
```

**with:**

```rust
//! The skill creator (skill creator spec): drafting a `SKILL.md` with the `claude` CLI, what a
//! draft must hold to be saved, and the folder Dots keeps the skills it makes in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::{ask_once, Once, OnceFailure};
use crate::kit::parse_frontmatter;
```

**In `crates/dots-core/src/skill_creator.rs`, replace:**

```rust
use crate::{Error, Result};

```

**with:**

````rust
use crate::{Error, Result};

/// The system prompt of a draft (spec §5). It names the rules `check_draft` holds a draft to.
pub const DRAFTING_INSTRUCTIONS: &str = "\
You write skills for Claude Code: a SKILL.md file that an assistant reads when a task calls for it.

Answer with the complete SKILL.md and nothing else: no greeting, no explanation, and no code \
fence around it.

The file starts with a frontmatter block:
- the first line is ---
- then a line name: <name>, where the name uses only lowercase letters, digits and hyphens, \
is at most 64 characters, and says what the skill is for
- then a line description: \"<what the skill does and when to use it>\", on one line, in \
double quotes, with a double quote inside written as \\\"
- then a line ---
Use only these top-level key: value lines. No lists, no indented lines, no other keys.

After the frontmatter, write the instructions in Markdown, for an assistant that will follow \
them: what to do, step by step, what to check, and what to hand back. Keep them specific to \
the idea and short enough to read in a minute.

When you are given a current SKILL.md and a change, answer with the whole revised file: \
changed as asked, and otherwise kept as it was.
";

/// How long a draft may take before the CLI is ended (spec §5).
pub const DRAFT_LIMIT: Duration = Duration::from_secs(5 * 60);

/// The most an idea or a change request may hold, in characters.
pub const MAX_REQUEST_CHARS: usize = 4000;

/// The model a draft uses when the dialog names none.
pub const DEFAULT_MODEL: &str = "sonnet";

/// What the dialog asks for (`skill_draft`): a first draft of `idea`, or, with `current` and
/// `change`, the current text changed as asked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DraftRequest {
    pub idea: String,
    pub model: String,
    pub current: Option<String>,
    pub change: Option<String>,
}

/// The request's text, sent on the CLI's stdin. `Invalid` for an empty idea or model, an
/// idea or change over `MAX_REQUEST_CHARS`, a current text over `MAX_SKILL_BYTES`, and a
/// change without the text to change (or the reverse).
pub fn draft_prompt(req: &DraftRequest) -> Result<String> {
    let idea = req.idea.trim();
    if idea.is_empty() {
        return invalid("Describe the skill first.");
    }
    if req.model.trim().is_empty() {
        return invalid("Pick a model.");
    }
    let too_long = |s: &str| s.chars().count() > MAX_REQUEST_CHARS;
    if too_long(idea) {
        return invalid(format!(
            "Describe the skill in at most {MAX_REQUEST_CHARS} characters."
        ));
    }
    match (req.current.as_deref(), req.change.as_deref().map(str::trim)) {
        (None, None) => Ok(format!("Write a skill for this idea:\n\n{idea}\n")),
        (Some(_), Some("")) | (None, Some(_)) => invalid("Say what should change."),
        (Some(_), None) => invalid("Say what should change."),
        (Some(current), Some(change)) => {
            if too_long(change) {
                return invalid(format!(
                    "Say what should change in at most {MAX_REQUEST_CHARS} characters."
                ));
            }
            if current.len() > MAX_SKILL_BYTES {
                return invalid(format!(
                    "The skill is longer than {} KB; shorten it.",
                    MAX_SKILL_BYTES / 1024
                ));
            }
            Ok(format!(
                "The idea the skill was written for:\n\n{idea}\n\n\
                 The current SKILL.md, between the lines <<<SKILL.md and SKILL.md>>>:\n\n\
                 <<<SKILL.md\n{}\nSKILL.md>>>\n\n\
                 Change it like this:\n\n{change}\n\n\
                 Answer with the whole revised SKILL.md.\n",
                current.trim_end()
            ))
        }
    }
}

/// The answer without surrounding blank lines and, when the whole answer is one fenced code
/// block, without the fence. It ends with one newline.
pub fn unwrap_fence(answer: &str) -> String {
    let text = answer.trim();
    let inner = text
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .and_then(|inner| inner.split_once('\n'))
        .filter(|(info, body)| {
            info.trim().chars().all(|c| c.is_ascii_alphanumeric())
                && !body.lines().any(|l| l.trim_start().starts_with("```"))
        })
        .map(|(_, body)| body.trim());
    format!("{}\n", inner.unwrap_or(text))
}

/// The drafts being written, at most one per dialog.
pub struct SkillDrafts {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
    /// Each draft's CLI starts in a new empty folder in here.
    dir: PathBuf,
    limit: Duration,
    shutdown: CancellationToken,
    /// By dialog id: a number for the draft, and what cancels it.
    running: Mutex<HashMap<String, (u64, CancellationToken)>>,
    next: Mutex<u64>,
}

impl SkillDrafts {
    /// `program` is the CLI the runtime found, `None` when there is none.
    pub fn new(
        program: Option<PathBuf>,
        env: Vec<(String, String)>,
        dir: PathBuf,
        limit: Duration,
        shutdown: CancellationToken,
    ) -> SkillDrafts {
        SkillDrafts {
            program,
            env,
            dir,
            limit,
            shutdown,
            running: Mutex::new(HashMap::new()),
            next: Mutex::new(0),
        }
    }

    /// Writes a draft for the dialog `dialog` (`skill_draft`, spec §5) and returns its text. A
    /// draft the same dialog asked for before is cancelled first. Fails with the CLI's error,
    /// when the answer holds no text, when `cancel` is called, or after the time limit.
    pub async fn draft(&self, dialog: &str, req: &DraftRequest) -> Result<String> {
        let prompt = draft_prompt(req)?;
        let Some(program) = self.program.clone() else {
            return Err(Error::Other(
                "claude CLI not found: install Claude Code or set its path in Settings".into(),
            ));
        };
        let token = self.shutdown.child_token();
        let number = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        };
        let previous = self
            .running
            .lock()
            .unwrap()
            .insert(dialog.to_string(), (number, token.clone()));
        if let Some((_, previous)) = previous {
            previous.cancel();
        }
        let cwd = self.dir.join(crate::util::new_id());
        let asked = match std::fs::create_dir_all(&cwd) {
            Ok(()) => {
                let once = Once {
                    program: &program,
                    env: &self.env,
                    cwd: &cwd,
                    model: req.model.trim(),
                    system_prompt: DRAFTING_INSTRUCTIONS,
                    prompt: &prompt,
                    limit: self.limit,
                };
                let asked = ask_once(&once, &token).await;
                let _ = std::fs::remove_dir_all(&cwd);
                asked
            }
            Err(e) => Err(OnceFailure::Failed(format!(
                "could not make {}: {e}",
                cwd.display()
            ))),
        };
        {
            let mut running = self.running.lock().unwrap();
            if running.get(dialog).is_some_and(|(n, _)| *n == number) {
                running.remove(dialog);
            }
        }
        match asked {
            Ok(answer) if answer.trim().is_empty() => Err(Error::Other(
                "Claude answered with no text; try again.".into(),
            )),
            Ok(answer) => Ok(unwrap_fence(&answer)),
            Err(OnceFailure::Cancelled) => Err(Error::Other("The draft was cancelled.".into())),
            Err(OnceFailure::TimedOut) => Err(Error::Other(
                "Claude took too long to write the skill.".into(),
            )),
            Err(OnceFailure::Failed(e)) => Err(Error::Other(format!(
                "Claude could not write the skill: {e}"
            ))),
        }
    }

    /// Cancels the dialog's draft, if one is being written (`skill_draft_cancel`).
    pub fn cancel(&self, dialog: &str) {
        if let Some((_, token)) = self.running.lock().unwrap().remove(dialog) {
            token.cancel();
        }
    }
}

````

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test skill_draft_test`
Expected: PASS: 13 tests (about 10 s: the cancel and time-limit tests end a sleeping fake CLI).

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 737 passed, `dots-app` 94; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/engine/claude/once.rs crates/dots-core/src/engine/claude/mod.rs crates/dots-core/src/skill_creator.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/skill_draft_test.rs
git commit -m "feat(core): draft a skill with one request to the claude CLI"
```

### Task 3: Commands to draft, check and save a skill

Four commands over Tasks 1 and 2, their contract samples, and the UI's schemas and wrappers.

**Files:**

- Modify: `crates/dots-core/src/skill_creator.rs` (`DraftRequest` serializes, for its sample), `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs`, `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts`, `ui/src/api/commands.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `ui/src/api/contract.test.ts`, `ui/src/api/commands.test.ts`

**Interfaces:**

- Consumes: Task 1's `check_draft`, `DraftSummary`, `LibraryService::create_skill`; Task 2's `DraftRequest`, `Runtime::skill_drafts`.
- Produces:
  - Commands `skill_draft(dialog: String, request: DraftRequest) -> String`, `skill_draft_cancel(dialog: String)`, `skill_check(text: String) -> DraftSummary` (an unusable draft is kind `invalid`), `skill_create(text: String) -> LibraryItem`.
  - UI: `T.DraftRequest`, `T.DraftSummary`; `commands.skillDraft(dialog, request)`, `commands.skillDraftCancel(dialog)`, `commands.skillCheck(text)`, `commands.skillCreate(text)`.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/api/commands.test.ts`, replace:**

```ts

  it("refuses a Library answer in a shape it does not know", async () => {
```

**with:**

```ts

  it("drafts, checks and saves a skill with the arguments the core names", async () => {
    const [first, change] = fixture.DraftRequest;
    reply = async () => "---\nname: pr-tests\n---\nBody\n";
    expect(await commands.skillDraft("dialog-1", first!)).toContain("pr-tests");
    await commands.skillDraft("dialog-1", change!);
    reply = async () => null;
    await commands.skillDraftCancel("dialog-1");
    reply = async () => fixture.DraftSummary;
    expect(await commands.skillCheck("text")).toEqual(fixture.DraftSummary);
    reply = async () => fixture.LibraryItem[1];
    expect(await commands.skillCreate("text")).toEqual(fixture.LibraryItem[1]);
    expect(calls).toEqual([
      ["skill_draft", { dialog: "dialog-1", request: first }],
      ["skill_draft", { dialog: "dialog-1", request: change }],
      ["skill_draft_cancel", { dialog: "dialog-1" }],
      ["skill_check", { text: "text" }],
      ["skill_create", { text: "text" }],
    ]);
  });

  it("says what to change in a draft that cannot be saved", async () => {
    reply = async () => {
      throw { kind: "invalid", message: "Add a description: line between the --- lines." };
    };
    const { kind, message } = await failure(commands.skillCheck("---\nname: a\n---\nB\n"));
    expect(kind).toBe("invalid");
    expect(message).toContain("description");
  });

  it("refuses a Library answer in a shape it does not know", async () => {
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  ScannedSkill: T.ScannedSkill.array(),
  InstalledPlugin: T.InstalledPlugin.array(),
```

**with:**

```ts
  ScannedSkill: T.ScannedSkill.array(),
  DraftRequest: T.DraftRequest.array(),
  DraftSummary: T.DraftSummary,
  InstalledPlugin: T.InstalledPlugin.array(),
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/api`
Expected: FAIL: `contract.test.ts` does not load (`T.DraftRequest` is undefined), and 2 tests of `commands.test.ts` fail ("drafts, checks and saves a skill…", "says what to change in a draft…": `commands.skillDraft` and `commands.skillCheck` are not functions); 16 others pass.

- [ ] **Step 3: Add the commands, the samples, the schemas and the wrappers**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use dots_core::settings::Settings;
use dots_core::Runtime;
```

**with:**

```rust
use dots_core::settings::Settings;
use dots_core::skill_creator::{check_draft, DraftRequest, DraftSummary};
use dots_core::Runtime;
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// The first documents of a context collection, with their titles and summaries.
```

**with:**

```rust

/// Drafts a skill with the `claude` CLI for the Create skill dialog `dialog` (skill creator
/// spec §5): no tools, no MCP servers, none of the person's settings. A draft the same dialog
/// asked for before is cancelled.
#[tauri::command]
pub async fn skill_draft(
    state: State<'_, AppState>,
    dialog: String,
    request: DraftRequest,
) -> Res<String> {
    Ok(state.rt()?.skill_drafts().draft(&dialog, &request).await?)
}

/// Ends the dialog's draft, if one is being written; its `skill_draft` then fails.
#[tauri::command]
pub fn skill_draft_cancel(state: State<'_, AppState>, dialog: String) -> Res<()> {
    state.rt()?.skill_drafts().cancel(&dialog);
    Ok(())
}

/// The name and description a draft would be saved with, or, as `invalid`, what to change.
#[tauri::command]
pub fn skill_check(text: String) -> Res<DraftSummary> {
    Ok(check_draft(&text)?)
}

/// Saves a draft as a new skill in Dots' skills folder and the Library (skill creator spec §6).
#[tauri::command]
pub async fn skill_create(state: State<'_, AppState>, text: String) -> Res<LibraryItem> {
    Ok(state.rt()?.library().create_skill(&text).await?)
}

/// The first documents of a context collection, with their titles and summaries.
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS, MAX_GLOBAL_INSTRUCTIONS};
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS, MAX_GLOBAL_INSTRUCTIONS};
use dots_core::skill_creator::{DraftRequest, DraftSummary};
use serde_json::{json, Value};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    samples["UpdateNote"] = to(update_notes());
    samples
```

**with:**

```rust
    samples["UpdateNote"] = to(update_notes());
    samples["DraftRequest"] = to([
        DraftRequest {
            idea: "Check a PR for missing tests".into(),
            model: "sonnet".into(),
            current: None,
            change: None,
        },
        DraftRequest {
            idea: "Check a PR for missing tests".into(),
            model: "sonnet".into(),
            current: Some("---\nname: pr-tests\n---\nList the changed functions.\n".into()),
            change: Some("Mention snapshot tests".into()),
        },
    ]);
    samples["DraftSummary"] = to(DraftSummary {
        name: "pr-tests".into(),
        description: "Checks a PR for missing tests.".into(),
    });
    samples
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    library_scan_skills,
    context_documents,
```

**with:**

```rust
    library_scan_skills,
    skill_draft,
    skill_draft_cancel,
    skill_check,
    skill_create,
    context_documents,
```

**In `crates/dots-core/src/skill_creator.rs`, replace:**

```rust
/// `change`, the current text changed as asked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DraftRequest {
```

**with:**

```rust
/// `change`, the current text changed as asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRequest {
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  DotSpec,
  LibraryConfig,
```

**with:**

```ts
  DotSpec,
  DraftRequest,
  LibraryConfig,
```

**In `ui/src/api/commands.ts`, replace:**

```ts
    call(T.ScannedSkill.array(), "library_scan_skills", { path }),
  contextDocuments: (id: string) => call(T.ContextDocuments, "context_documents", { id }),
```

**with:**

```ts
    call(T.ScannedSkill.array(), "library_scan_skills", { path }),
  /// Drafts a skill with the claude CLI for the dialog `dialog`, cancelling the dialog's
  /// draft before; answers the SKILL.md text.
  skillDraft: (dialog: string, request: DraftRequest) =>
    call(z.string(), "skill_draft", { dialog, request }),
  skillDraftCancel: (dialog: string) => call(nothing, "skill_draft_cancel", { dialog }),
  /// Rejects with kind `invalid`, saying what to change, when the text cannot be saved.
  skillCheck: (text: string) => call(T.DraftSummary, "skill_check", { text }),
  /// Rejects with kind `conflict` when the name is in the Library or its folder exists.
  skillCreate: (text: string) => call(T.LibraryItem, "skill_create", { text }),
  contextDocuments: (id: string) => call(T.ContextDocuments, "context_documents", { id }),
```

**In `ui/src/api/types.ts`, replace:**

```ts
});
export const InstalledPlugin = z.strictObject({
```

**with:**

```ts
});
// What the Create skill dialog asks for: a first draft of `idea`, or `current` changed as
// `change` says.
export const DraftRequest = z.strictObject({
  idea: z.string(),
  model: z.string(),
  current: nullable(z.string()),
  change: nullable(z.string()),
});
// The name and description the Library will show for a draft that can be saved.
export const DraftSummary = z.strictObject({
  name: z.string(),
  description: z.string(),
});
export const InstalledPlugin = z.strictObject({
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type ScannedSkill = z.infer<typeof ScannedSkill>;
export type InstalledPlugin = z.infer<typeof InstalledPlugin>;
```

**with:**

```ts
export type ScannedSkill = z.infer<typeof ScannedSkill>;
export type DraftRequest = z.infer<typeof DraftRequest>;
export type DraftSummary = z.infer<typeof DraftSummary>;
export type InstalledPlugin = z.infer<typeof InstalledPlugin>;
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: PASS both; the fixture gains `DraftRequest`, `DraftSummary` and the four commands in `Commands`.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-app` 94 passed; the UI 436 in 31 files; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/skill_creator.rs apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs apps/dots-app/src/contract.rs ui/src/api/types.ts ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/commands.test.ts ui/src/api/__fixtures__/contract.json
git commit -m "feat(app): commands to draft, check and save a skill"
```

### Task 4: The Create skill dialog

The dialog of spec §4 in a file of its own, its button in the Skills tab, and the line about the folder in the remove confirmation (§7).

**Files:**

- Create: `ui/src/screens/library/CreateSkillDialog.tsx`
- Modify: `ui/src/screens/library/SkillsTab.tsx`, `ui/src/screens/library/parts.tsx`
- Test: `ui/src/screens/library/LibraryScreen.test.tsx`

**Interfaces:**

- Consumes: Task 3's wrappers and types; `LibraryEntry.dots_folder` (Task 1); `later` from `@/test/backend`.
- Produces: `CreateSkillDialog({ onClose }: { onClose: () => void })`.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
import type { LibraryEntry } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
```

**with:**

```tsx
import type { LibraryEntry } from "@/api/types";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
  });
```

**with:**

```tsx
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
  });
});

describe("creating a skill", () => {
  const DRAFT = "---\nname: pr-tests\ndescription: Checks a PR for missing tests.\n---\nList them.\n";
  const summary = { name: "pr-tests", description: "Checks a PR for missing tests." };

  /// Opens the dialog and describes `idea`.
  async function describeIdea(idea = "Check a PR for missing tests") {
    const view = renderApp("/library");
    await view.user.click(await screen.findByRole("tab", { name: "Skills" }));
    await view.user.click(screen.getByRole("button", { name: "Create skill" }));
    const dialog = await screen.findByRole("dialog", { name: "Create skill" });
    expect(within(dialog).getByRole("button", { name: "Write it" })).toBeDisabled();
    await view.user.type(within(dialog).getByLabelText("What should the skill do?"), idea);
    return { ...view, dialog };
  }

  it("writes a draft from an idea, shows what the Library will show, and saves it", async () => {
    backend.on("skill_draft", () => DRAFT);
    backend.on("skill_check", () => summary);
    backend.on("skill_create", () => sample.libraryItems[1]);
    const { user, dialog } = await describeIdea();
    await user.clear(within(dialog).getByLabelText("Model"));
    await user.type(within(dialog).getByLabelText("Model"), "haiku");
    await user.click(within(dialog).getByRole("button", { name: "Write it" }));
    expect(await within(dialog).findByLabelText("SKILL.md")).toHaveValue(DRAFT);
    expect(backend.callsTo("skill_draft")).toEqual([
      {
        dialog: expect.stringMatching(/^create-skill-/),
        request: { idea: "Check a PR for missing tests", model: "haiku", current: null, change: null },
      },
    ]);
    expect(
      await within(dialog).findByText("The Library will show pr-tests: Checks a PR for missing tests."),
    ).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Save to Library" }));
    await waitFor(() => expect(backend.callsTo("skill_create")).toEqual([{ text: DRAFT }]));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("asks for changes with the text as edited, and keeps it when the request fails", async () => {
    backend.on("skill_draft", () => DRAFT);
    backend.on("skill_check", () => summary);
    const { user, dialog } = await describeIdea();
    await user.click(within(dialog).getByRole("button", { name: "Write it" }));
    const text = await within(dialog).findByLabelText("SKILL.md");
    await user.type(text, "Mine.");
    expect(within(dialog).getByRole("button", { name: "Ask for changes" })).toBeDisabled();
    await user.type(within(dialog).getByLabelText("What should change?"), "Name the tests");
    backend.on("skill_draft", () => {
      throw { kind: "internal", message: "Claude could not write the skill: not logged in" };
    });
    await user.click(within(dialog).getByRole("button", { name: "Ask for changes" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("not logged in");
    expect(within(dialog).getByLabelText("SKILL.md")).toHaveValue(`${DRAFT}Mine.`);
    expect(backend.callsTo("skill_draft")[1]).toMatchObject({
      request: {
        idea: "Check a PR for missing tests",
        model: "sonnet",
        current: `${DRAFT}Mine.`,
        change: "Name the tests",
      },
    });
  });

  it("says why a draft cannot be saved and keeps Save off", async () => {
    backend.on("skill_draft", () => "Just some text\n");
    backend.on("skill_check", () => {
      throw { kind: "invalid", message: "The skill must start with a --- line." };
    });
    const { user, dialog } = await describeIdea();
    await user.click(within(dialog).getByRole("button", { name: "Write it" }));
    expect(
      await within(dialog).findByText("The skill must start with a --- line."),
    ).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Save to Library" })).toBeDisabled();
    expect(backend.callsTo("skill_create")).toEqual([]);
  });

  it("cancels a draft being written, goes back with the idea kept, and drops its answer", async () => {
    const pending = later<string>();
    backend.on("skill_draft", () => pending.answer);
    backend.on("skill_draft_cancel", () => null);
    const { user, dialog } = await describeIdea("Sort my notes");
    await user.click(within(dialog).getByRole("button", { name: "Write it" }));
    expect(await within(dialog).findByRole("status")).toHaveTextContent(
      "Claude is writing the skill…",
    );
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    const [{ dialog: id }] = backend.callsTo("skill_draft") as [{ dialog: string }];
    expect(backend.callsTo("skill_draft_cancel")).toEqual([{ dialog: id }]);
    expect(within(dialog).getByLabelText("What should the skill do?")).toHaveValue("Sort my notes");
    await act(async () => pending.give(DRAFT));
    expect(within(dialog).queryByLabelText("SKILL.md")).toBeNull();
  });

  it("shows a failed draft where it happened", async () => {
    backend.on("skill_draft", () => {
      throw { kind: "internal", message: "claude CLI not found: install Claude Code" };
    });
    const { user, dialog } = await describeIdea();
    await user.click(within(dialog).getByRole("button", { name: "Write it" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("claude CLI not found");
    expect(within(dialog).getByRole("button", { name: "Write it" })).toBeEnabled();
  });

  it("asks before it discards an unsaved draft, and shows a refused save", async () => {
    backend.on("skill_draft", () => DRAFT);
    backend.on("skill_check", () => summary);
    backend.on("skill_create", () => {
      throw { kind: "conflict", message: "A skill named pr-tests is already in the Library." };
    });
    const { user, dialog } = await describeIdea();
    await user.click(within(dialog).getByRole("button", { name: "Write it" }));
    await within(dialog).findByLabelText("SKILL.md");
    await within(dialog).findByText(/The Library will show/);
    await user.click(within(dialog).getByRole("button", { name: "Save to Library" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already in the Library");
    await user.keyboard("{Escape}");
    const ask = await screen.findByRole("alertdialog", { name: "Discard this draft?" });
    await user.click(within(ask).getByRole("button", { name: "Keep editing" }));
    expect(screen.getByRole("dialog", { name: "Create skill" })).toBeVisible();
    await user.keyboard("{Escape}");
    const again = await screen.findByRole("alertdialog", { name: "Discard this draft?" });
    await user.click(within(again).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("says that removing a skill Dots made deletes its folder", async () => {
    listing([{ ...skill, dots_folder: true }]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    const row = await screen.findByRole("listitem", { name: "pdf" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    expect(await screen.findByRole("alertdialog")).toHaveTextContent(
      "Its folder in Dots' skills folder is deleted too.",
    );
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test src/screens/library`
Expected: FAIL: the 7 tests of "creating a skill" (no button "Create skill"; no line about the folder).

- [ ] **Step 3: Add the dialog, its button and the remove line**

**Create `ui/src/screens/library/CreateSkillDialog.tsx`:**

```tsx
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { commands } from "@/api/commands";
import type { DraftRequest, DraftSummary } from "@/api/types";
import { Field } from "@/components/Field";
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
import { Textarea } from "@/components/ui/textarea";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// Counts the dialogs opened. The core files a draft under its dialog's id, and a new draft of
/// the same dialog cancels the one before.
let opened = 0;

type Stage = "describe" | "writing" | "draft";
/// Whether the text can be saved: what the Library will show, or why it cannot be.
type Check = { summary: DraftSummary } | { problem: string } | null;

/// "Create skill" (skill creator spec §4): describe the idea, Claude writes the skill through
/// the person's `claude` CLI, the person edits it or asks for changes, then saves it.
export function CreateSkillDialog({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const [dialog] = useState(() => `create-skill-${++opened}`);
  const [stage, setStage] = useState<Stage>("describe");
  // Where Writing goes back to when it fails or is cancelled.
  const [from, setFrom] = useState<"describe" | "draft">("describe");
  const [idea, setIdea] = useState("");
  const [model, setModel] = useState("sonnet");
  const [text, setText] = useState("");
  const [change, setChange] = useState("");
  const [check, setCheck] = useState<Check>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [discarding, setDiscarding] = useState(false);
  // The newest request: the answer to an older or a cancelled one is dropped.
  const asked = useRef(0);
  const writing = useRef(false);
  writing.current = stage === "writing";

  // A dialog that goes away while Claude writes ends the CLI.
  useEffect(
    () => () => {
      if (writing.current) void commands.skillDraftCancel(dialog).catch(() => {});
    },
    [dialog],
  );

  useEffect(() => {
    if (stage !== "draft") return;
    let current = true;
    commands.skillCheck(text).then(
      (summary) => current && setCheck({ summary }),
      (e: unknown) => current && setCheck({ problem: errorMessage(e) }),
    );
    return () => {
      current = false;
    };
  }, [stage, text]);

  async function write(back: "describe" | "draft") {
    const n = ++asked.current;
    const request: DraftRequest =
      back === "draft"
        ? { idea, model, current: text, change }
        : { idea, model, current: null, change: null };
    setFrom(back);
    setError(null);
    setStage("writing");
    try {
      const answer = await commands.skillDraft(dialog, request);
      if (n !== asked.current) return;
      setText(answer);
      setChange("");
      setStage("draft");
    } catch (e) {
      if (n !== asked.current) return;
      setError(errorMessage(e));
      setStage(back);
    }
  }

  function stopWriting() {
    asked.current++;
    void commands.skillDraftCancel(dialog).catch(() => {});
    setStage(from);
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await commands.skillCreate(text);
      invalidate(qc, keys.library);
      onClose();
    } catch (e) {
      setError(errorMessage(e));
      setSaving(false);
    }
  }

  function leave() {
    if (stage === "writing") stopWriting();
    onClose();
  }

  function close() {
    if (text.trim() !== "") setDiscarding(true);
    else leave();
  }

  function startOver() {
    setText("");
    setChange("");
    setCheck(null);
    setError(null);
    setStage("describe");
  }

  const ready = check !== null && "summary" in check;
  return (
    <>
      <Dialog open onOpenChange={(next) => !next && close()}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>Create skill</DialogTitle>
            <DialogDescription>
              Claude writes it through your claude CLI, with no tools. You read it before it is
              saved.
            </DialogDescription>
          </DialogHeader>
          {stage === "describe" && (
            <>
              <Field
                label="What should the skill do?"
                htmlFor="skill-idea"
                hint="A sentence or two is enough."
              >
                <Textarea
                  id="skill-idea"
                  rows={4}
                  value={idea}
                  onChange={(e) => setIdea(e.target.value)}
                />
              </Field>
              <Field label="Model" htmlFor="skill-model">
                <Input
                  id="skill-model"
                  list="skill-models"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                />
                <datalist id="skill-models">
                  <option value="sonnet" />
                  <option value="opus" />
                  <option value="haiku" />
                </datalist>
              </Field>
            </>
          )}
          {stage === "writing" && (
            <p role="status" className="text-muted-foreground">
              Claude is writing the skill…
            </p>
          )}
          {stage === "draft" && (
            <>
              <Field label="SKILL.md" htmlFor="skill-text">
                <Textarea
                  id="skill-text"
                  rows={16}
                  className="font-mono text-xs"
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                />
              </Field>
              {check &&
                ("summary" in check ? (
                  <p role="status" className="text-xs text-muted-foreground">
                    {`The Library will show ${check.summary.name}: ${check.summary.description}`}
                  </p>
                ) : (
                  <p role="status" className="text-xs text-destructive">
                    {check.problem}
                  </p>
                ))}
              <Field label="What should change?" htmlFor="skill-change">
                <Textarea
                  id="skill-change"
                  rows={2}
                  value={change}
                  onChange={(e) => setChange(e.target.value)}
                />
              </Field>
            </>
          )}
          {error && (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            {stage === "describe" && (
              <Button
                type="button"
                disabled={idea.trim() === "" || model.trim() === ""}
                onClick={() => void write("describe")}
              >
                Write it
              </Button>
            )}
            {stage === "writing" && (
              <Button type="button" variant="outline" onClick={stopWriting}>
                Cancel
              </Button>
            )}
            {stage === "draft" && (
              <>
                <Button type="button" variant="outline" onClick={startOver}>
                  Start over
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={change.trim() === ""}
                  onClick={() => void write("draft")}
                >
                  Ask for changes
                </Button>
                <Button type="button" disabled={!ready || saving} onClick={() => void save()}>
                  Save to Library
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={discarding} onOpenChange={setDiscarding}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard this draft?</AlertDialogTitle>
            <AlertDialogDescription>The skill has not been saved.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction onClick={leave}>Discard</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
```

**In `ui/src/screens/library/SkillsTab.tsx`, replace:**

```tsx
import { open } from "@tauri-apps/plugin-dialog";
import { Plus } from "lucide-react";
import { useEffect, useState } from "react";
```

**with:**

```tsx
import { open } from "@tauri-apps/plugin-dialog";
import { Plus, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
```

**In `ui/src/screens/library/SkillsTab.tsx`, replace:**

```tsx

import { isDeclined, ItemRow, plural, RemoveButton, UsedBy, type SkillEntry } from "./parts";
```

**with:**

```tsx

import { CreateSkillDialog } from "./CreateSkillDialog";
import { isDeclined, ItemRow, plural, RemoveButton, UsedBy, type SkillEntry } from "./parts";
```

**In `ui/src/screens/library/SkillsTab.tsx`, replace:**

```tsx
/// The skills tab. "Add skill" picks a folder: one skill, or a folder with several below it.
export function SkillsTab({ entries }: { entries: SkillEntry[] }) {
```

**with:**

```tsx
/// The skills tab. "Add skill" picks a folder: one skill, or a folder with several below it.
/// "Create skill" has Claude write one from an idea.
export function SkillsTab({ entries }: { entries: SkillEntry[] }) {
```

**In `ui/src/screens/library/SkillsTab.tsx`, replace:**

```tsx
  const [scan, setScan] = useState<Scan | null>(null);

```

**with:**

```tsx
  const [scan, setScan] = useState<Scan | null>(null);
  const [creating, setCreating] = useState(false);

```

**In `ui/src/screens/library/SkillsTab.tsx`, replace:**

```tsx
        </p>
        <Button size="sm" onClick={() => void choose()}>
```

**with:**

```tsx
        </p>
        <Button size="sm" variant="outline" onClick={() => setCreating(true)}>
          <Sparkles aria-hidden /> Create skill
        </Button>
        <Button size="sm" onClick={() => void choose()}>
```

**In `ui/src/screens/library/SkillsTab.tsx`, replace:**

```tsx
      )}
      {scan && (
```

**with:**

```tsx
      )}
      {creating && <CreateSkillDialog onClose={() => setCreating(false)} />}
      {scan && (
```

**In `ui/src/screens/library/parts.tsx`, replace:**

```tsx
          {entry.kind === "plugin" && " Its MCP servers go with it."}
        </>
```

**with:**

```tsx
          {entry.kind === "plugin" && " Its MCP servers go with it."}
          {entry.dots_folder && " Its folder in Dots' skills folder is deleted too."}
        </>
```

- [ ] **Step 4: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: the UI 443 in 31 files; `tsc` clean.

- [ ] **Step 5: Commit**

```bash
git add ui/src/screens/library/CreateSkillDialog.tsx ui/src/screens/library/SkillsTab.tsx ui/src/screens/library/parts.tsx ui/src/screens/library/LibraryScreen.test.tsx
git commit -m "feat(ui): a Create skill dialog in the Library's Skills tab"
```

### Task 5: Record what the prototype settled

The spec gains §9, "Changes made while building", with what the table "Deviations" says; "Left to the builder" becomes §10.

**Files:**

- Modify: `docs/superpowers/specs/2026-10-21-skill-creator-design.md`

- [ ] **Step 1: Change the spec**

**In `docs/superpowers/specs/2026-10-21-skill-creator-design.md`, replace:**

```markdown

## 9. Left to the builder

```

**with:**

```markdown

## 9. Changes made while building

The plan's prototype settled these:

- Names: the core module is `skill_creator` (`check_draft`, `DraftRequest`, `DraftSummary`, `SkillDrafts`), the one-off CLI call is `engine::claude::once::ask_once`, and the commands are `skill_draft`, `skill_draft_cancel`, `skill_check` and `skill_create`. The library entry's field is `dots_folder`.
- Limits: an idea or a change request holds at most 4,000 characters, and a draft at most 64 KB, both for a change request and for saving (§5, §6).
- A name that the Library takes but Windows cannot use for a folder is refused at the draft check: a device name (`con`, `nul`, `com1`, also with an extension) or a name ending in `.` (§6, step 1).
- A save writes into a hidden folder `.<name>.<id>` in `<data>/skills` and renames it; startup removes hidden folders left there, and the whole `<data>/drafts` folder (§5, §6).
- **Start over** also clears the draft, so closing the dialog after it does not ask "Discard this draft?" (§4).
- The commands are one-line wrappers, as the others are: the per-dialog cancel and the error texts are tested in the core, the arguments in the UI's command test (§8).

## 10. Left to the builder

```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-10-21-skill-creator-design.md
git commit -m "docs(spec): record what the skill creator's prototype settled"
```

### Task 6: Smoke test

Write `docs/verification/2026-10-21-skill-creator-smoke.md` in the form of `docs/verification/2026-10-13-plan3-smoke.md`: date, build, what each part covers, the commands, and each step's result.

**Part A (agent steps), against a debug build** (`bun run tauri build --debug --no-bundle`, into the shared target folder), started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d`, `DOTS_CONFIRM=yes`, a `PATH` without any folder holding the real `claude`, `WEBVIEW2_USER_DATA_FOLDER` moved, and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`, driven with `node tools/smoke/webview.mjs 9333`. The Claude program is `fake-claude.exe` copied out of `target/` (`cargo build -p dots-core --bin fake-claude`), set as the `claude_path` setting, then Dots restarted; `FAKE_CLAUDE_SCRIPT` names one script file in the smoke folder, which a step overwrites with the answer it wants. The installed Dots is never stopped or touched; stop only the processes these steps started, by their ids.

- A1. Skills tab, **Create skill**, an idea, **Write it**: the script answers a valid `SKILL.md`; the editor shows it and "The Library will show pr-tests: …".
- A2. Type a hand edit, fill "What should change?", **Ask for changes**: the script answers a second text; the editor shows it. Record the fake CLI's recorded arguments (`FAKE_CLAUDE_ARGS_OUT`): they match the Global Constraints' command line, and the request holds the hand edit.
- A3. **Save to Library**: the dialog closes, the row `pr-tests` is listed, and `<DOTS_DATA_DIR>/skills/pr-tests/SKILL.md` holds the text.
- A4. A dot's editor offers `pr-tests` among the skills; tick it and save the dot.
- A5. Remove `pr-tests`: the confirmation names the dot and says "Its folder in Dots' skills folder is deleted too."; after it, the folder is gone and the dot no longer lists it.
- A6. A script that sleeps 30 s: **Write it**, then **Cancel** within a few seconds: the dialog is back on Describe with the idea, and no `fake-claude.exe` is left running (`tasklist`).
- A7. A script whose answer has no frontmatter: the dialog shows "The skill must start with a --- line…" and Save stays off.

**Part B (person steps)**, listed for the owner, none run by an agent:

- B1. With the real `claude` CLI: create a skill from a one-line idea; ask for a change; save it; a Claude dot that uses it reads it on a task that calls for it.
- B2. The draft ignores the person's setup: put a rule in `~/.claude/CLAUDE.md` (for example "answer in French") and see that the draft does not follow it; then remove the rule.
- B3. The draft cannot act: an idea such as "list the files in this folder and write a skill about them" gives a skill text, and no tool call or approval appears anywhere in Dots.

- [ ] **Step 1: Run Part A and write the document**
- [ ] **Step 2: Commit**

```bash
git add docs/verification/2026-10-21-skill-creator-smoke.md
git commit -m "docs(verification): the skill creator smoke"
```
