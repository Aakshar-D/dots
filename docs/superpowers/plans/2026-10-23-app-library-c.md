# App Library, Plan C (Projects, bundled) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Projects ships with Dots: an Asana-like app in the app format, with its own data, tools, tests and list screen, installed for every dot on first start and updated with each release while it is still the bundled one; the person assigns a task to a dot, the dot starts on it, reports in its Activity and leaves it In review; and the person can undo changes, open a dot's run from a task, answer a one-time prompt to let Projects start dots, and install Projects again after removing it.

**Architecture:** Projects is a folder in `crates/dots-core/apps/projects/` (`app.json`, `logic.js`, `migrations/`, `sample.sql`, `tests/`, `ui/`) built into Dots by `apps::bundled`, which reads it with the same checks as a dot's proposal (`source::read_files`, split from `read_folder`). `Apps::sync_bundled` installs it at start (once, for every dot, noted in `<data>/apps/bundled.json`) and updates it while it is bundled and unchanged; `Apps::install_bundled` and `bundled_offers` give the Apps screen "Install Projects", on kept data when it fits. Its logic writes an Activity row for every change and starts the assigned dot through `ctx.dots.start`; `run_ended` closes the loop. Its screen is plain modules in its `ui/` (`projects.js`, `markdown.js`), tested in Vitest with a fake `dots`. The host gains `dots.openRun` (checked by `Apps::run_link`), the one-time prompt (`AppConfig.start_asked`, `Apps::answer_start_prompt`) and the "Comes with Dots" section.

**Tech Stack:** Rust 1.95, sqlx 0.8, `rusqlite` 0.32, `rquickjs` 0.14, tokio; Tauri 2.12; Bun 1.3, React 19.3, TypeScript 6.0, zod 4.6, Vitest 5.0; WebView2. Projects' own code is plain JavaScript (ES modules) and SQL. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-23-app-library-design.md` on `master`: section 8 (Projects), with sections 1, 2, 4, 5 and 7 for what an app is and how it starts dots, section 12's Projects line, and §15 "Left to plan C"; §15 and §17 say what plans A and B built. Task 10 adds §18, "Changes made while building (plan C)", which the table "Deviations" lists.

**Plans before this one:** A, the core (merged, PR #24, `df24ced`); B, the screens, the frame and bridge, the review card and `copy_app` (merged, PR #32, `c2e5397`).

**Where the tasks come from:** spec §8 "Data", "Tools", "Undo" and the Done rule (Tasks 1, 2), §8 "Assigning to a dot" and §7 (Task 2), §8 "Bundled and changed copies" (Tasks 3, 4, 7), §8's one-time prompt (Tasks 4, 7), §8 "Screen" and mockup A (Tasks 5, 6), §12's core tests that load the bundled app (Tasks 1, 3), the person's choices in the question round (Tasks 4, 6, 7) and plan B's deferred findings the person folded in (Tasks 7, 8).

**How this plan's code was checked:** the code blocks of Tasks 1 to 10 were generated from a throwaway worktree of `c2e5397`, one commit per task, where they compiled and passed on Windows 11, and the plan was then replayed onto `c2e5397` by a script that applied every block and compared the tree with each task commit (all 10 tasks matched; the contract fixture was taken from the task commit, as the command writes it). Before the first task: `cargo test -p dots-core` 1687 passed, `cargo test -p dots-app` 174, `bun run --cwd ui test` 827 tests in 82 files. At the last task: `dots-core` 1701, `dots-app` 174, the UI 862 in 85 files; `cargo fmt --check` and `tsc --noEmit` clean; clippy reports no warning. Task 9's check ran in a debug build beside the installed Dots, driven over WebView2's remote-debugging port, on a temp data folder, with `fake-claude`: Projects was installed at first start, the prompt switched "Can start dots" on, `tools/smoke/projects.mjs` walked the screen, a fake-claude dot given a task read it, commented and left it In review with its summary, "Open the run" opened its conversation, and `tools/smoke/frame-leaks.mjs` found nothing from Projects' frame. No agent ran the real `claude` CLI.

## Global Constraints

- Toolchain: Rust 1.95 stable; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); Node 22 or later for the smoke scripts; WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/app-library-c`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\app-library-c`, based on `c2e5397` (`origin/master`). All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours, nor is the stray `C\uf03a/` folder of the main checkout).
- Cargo runs one chain at a time, with `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target CARGO_INCREMENTAL=0` once per shell and `-j 2`. The conversational-chat worktrees build into `target-chat`; do not start a second chain on `target`. Before a full run, check the free space on C: (`powershell -NoProfile -Command "(Get-PSDrive C).Free/1GB"`): an empty test summary can mean a full disk. UI tests run after cargo has finished.
- No new dependency, no migration of Dots' own database, no new Tauri plugin or capability. New: `crates/dots-core/apps/projects/` (Projects' files, built into Dots with `include_bytes!`), a `.gitattributes` line keeping that folder LF, `Config.bundled_apps`, `AppConfig.start_asked`, four commands (`apps_bundled_offers`, `app_install_bundled`, `app_answer_start_prompt`, `app_run_link`), and `"allowJs": true` in `ui/tsconfig.json` (the UI tests import Projects' screen modules).
- The subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials or calls an Anthropic API. No agent runs the real `claude` CLI (not even with a prompt); tests and smokes use `fake-claude`. A real-CLI smoke is the person's.
- Values the spec fixes: Projects' name `projects`, title "Projects", description "Projects, tasks and to-dos, shared by you and your dots", `permissions: ["start_dots"]`, `on_run_end: "run_ended"`; the tables `projects`, `sections`, `tasks`, `subtasks`, `activity` with the columns of §8; the statuses To do, In progress, In review, Done; "Only the person marks a task Done; set In review"; `"me"` resolves to the calling dot; the left column My tasks, Assigned to dots, the projects, + New project; the columns Task, Assignee, Due and Status; the side pane's title, assignee, due date, status, Spec (Markdown), subtasks and Activity with a comment box; "● <dot> started" with the run linked; In review when a dot's run ends unless Done; "Changed by you; Dots no longer updates it"; "Install Projects"; Projects is on for every dot; "Can start dots" off until the person switches it on.
- No agent installs or runs an installer, runs a release build, or touches the installed Dots (`%LOCALAPPDATA%\Dots`) or the real data folder `%LOCALAPPDATA%\dots`. Task 9's check runs a debug build on a temp `DOTS_DATA_DIR` and its own `WEBVIEW2_USER_DATA_FOLDER`, with a copy of `fake-claude` outside `target/`, and stops only the process under `target\debug`.
- A background command that Claude Code stops for low memory is reported, not restarted without the person's word.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit (the blocks hold `§`, `—`, `›`, `●`, `✓`, `☐`, `☑`, `×` and `…`). The working copies may be CRLF (`core.autocrlf`); the index must hold LF. Projects' folder must be LF in the working copy too: it is built into Dots byte for byte, and `apps_bundled_test::projects_holds_every_file_of_its_folder_and_only_lf_text` fails on a carriage return.
- Projects' files are listed in `PROJECTS_FILES` (`crates/dots-core/src/apps/bundled.rs`), sorted by path; a file added to the folder is added there in the same step.
- Live data is parsed, not cast, with strict schemas: a type the UI parses changes only together with the contract fixture and `ui/src/api/types.ts`, in the same commit (Task 4). The fixture `ui/src/api/__fixtures__/contract.json` is written by `UPDATE_CONTRACT=1 cargo test -p dots-app contract` and never by hand. `contract.rs`'s `samples()` is at the `json!` macro's recursion limit: a new sample is added after the macro (`samples["X"] = …`).
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`), `cargo clippy -j 2 --workspace --all-targets` (no warning: the base has none), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first. The UI files and Projects' screen have no formatter; keep new lines within 100 columns where the file does. A UI run counts only when "Test Files N passed (N)" equals the number of `*.test.ts*` files under `ui/src`; a `[vitest-pool-runner]: Timeout waiting for worker to respond` run is run again, not accepted.
- Known flaky under load (rerun the test binary alone before treating one as a failure): `claude_gateway_test::a_claude_run_reaches_its_servers_through_the_gateway_only`, the worktree tests of `runtime_worktree_test` and `workflow_test` (a run "did not reach AwaitingApproval" under load), `schedule_test::newest_slot_in_a_long_window_does_not_walk_every_slot`, `dots-app startup::tests::a_panic_is_written_to_the_log_file`, `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`, and the timeouts of `resume_test`, `runner_workflow_test` and `runtime_browser_test` under the full run's load.
- Fact-Forcing Gate (a hook of this machine): the first Edit or Write of each file, the first Bash command after a task notification, and `rm`, `git commit --amend` or a rebase are blocked until the facts it asks for are stated; then retry the identical call. For a large new file, write a one-line stub first. Long text through a Bash heredoc can fail to parse: use the Write tool, or the step applier `.superpowers/tools/plan-gen/apply_step.py` of the main checkout.

## Deviations from the spec

The spec leaves these open or says otherwise; Task 10 records them in a new §18.

| Spec | This plan | Why |
|---|---|---|
| §8: Projects "bundled with Dots" (how is the builder's) | Its files live in `crates/dots-core/apps/projects/`, built into Dots with `include_bytes!` (`apps::bundled`), read with the checks a proposal gets (`source::read_files`); `.gitattributes` keeps the folder LF and a test checks the built-in list against the folder | The person's choice; the same bytes in tests, debug and release, with no resource lookup |
| §8: installed on first start; removing it lasts | `Apps::sync_bundled` at start installs it once, for every dot, and notes it in `<data>/apps/bundled.json`; a removed Projects is not put back, and an app a dot made under the name is left alone. `Config::new` leaves it out (`Config.bundled_apps`), `Config::from_settings` (the app) has it | A note in the apps folder needs no migration; the test runtimes' prompts and tool lists stay as they are |
| §8: a release updates it "through its migrations" while it is bundled | Updated when this release's files differ from the digest of the files Dots last applied (kept in `bundled.json`), whatever the release string; the new version must keep the migrations that ran, byte for byte, or it is refused and logged. A roll back leaves the digest, so the update is not applied again (changed in execution: the release-string rule let the next release reapply it) | The rule a dot's change follows (§15) keeps old data safe; a roll back must last |
| §8: "Install Projects … keeps the old data if its schema matches" | "Comes with Dots" on the Apps screen; kept data fits by §17's rule (its migrations begin the bundled ones, byte for byte); the dialog offers it ticked, and unticked starts fresh, leaving it listed | The person's choice |
| §8: "a one-time prompt" (where is the builder's) | A banner above `/apps/projects`: "Let Projects start dots?" with "Switch on" and "Not now"; either answer, or the Apps screen's switch, sets `AppConfig.start_asked`; installing it again asks again | The person's choice |
| §8: "● builder started" with the run linked | The page's `dots.openRun(run_id)`; the core opens only a run of a topic this app started (`Apps::run_link`), and a preview's frame cannot open runs | The person's choice: the frame cannot navigate Dots |
| §8 "Undo": Undo on a dot's changes | The person undoes any change, their own included; a dot undoes only dots' changes, never back to Done; an undo starts no dot | The person's choice |
| §8 "Data": the columns | Statuses stored as `to_do`, `in_progress`, `in_review`, `done`; a change row's field is `title`, `spec`, `status`, `assignee`, `due_on`, `section` or `subtask:<id>`; a task's creation has a row (`created`, no undo); run rows `started`, `not_started`, `ended` | Values a dot writes as they are stored, and one Undo rule for every field |
| §8 "Tools" | Also `rename_section`, `move_task`, `list_dots`; `set_subtask` renames too; `list_tasks` takes `view` (`my_tasks`, `assigned_to_dots`) and `limit` (100, at most 1000) and says `more`; `get_task` takes `activity` (20) | The screen's needs; a dot's result is bounded at 30 KB |
| §8 "Assigning to a dot" | No start when a dot takes a task itself or it is Done; a refused start is an Activity row and the task waits; the same dot again while its topic is open is not a second start; a run's end moves the task In review only while it is still that dot's | A start must not fail the person's assignment, and a dot's run must not move a task given to someone else |
| §8: reordering, dragging | Dragging in a project's list, onto a task (before it) or a section (its end); reordering within a section writes no Activity row | Order is not a change of the task |
| §8: the Spec is Markdown | A renderer of Projects' own (`ui/markdown.js`): everything escaped first; headings, lists with checkboxes, quotes, code, bold, italic; a link shows its text | The person's choice; the frame loads nothing but Projects' files |
| Plan B deferred: more than 8 open cards evict each other's previews | The preview used least recently closes; a card whose preview closed says so with "Start the preview again" | The person folded it in |
| Plan B deferred: `frame-leaks.mjs` has no timeout | A missing target or one that does not answer within 10 s exits 2 ("did not run") | The person folded it in |
| Plan B deferred: `/preview/<id>` without a final slash still serves `index.html` | Refused, as `/app/<name>/<version>` is; an empty id too | The person folded it in |
| Plan B deferred: untested UI branches | Tests of AppScreen on an app whose code cannot be read, another app's data event, and a theme change after the page loaded | The person folded it in |

The spec leaves to the builder, and the plan settles: the starting message ("You are assigned the Projects task "<title>" (id <id>). Read it first with the Projects tool get_task …"), the screen's layout beyond mockup A, `sample.sql`'s rows, and the prompt's wording.

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. A dot sets a task Done, or undoes its way back to Done. Expected: refused with "Only the person marks a task Done; set In review", nothing changes. Tests: `tasks.test.js` "a dot cannot mark a task Done, and nothing changes" (Task 1), `undo.test.js` "a dot's undo cannot bring back Done" (Task 2).
2. A task's spec holding HTML or a script (pasted, or written by a dot that read a hostile page). Expected: shown as text; nothing runs, nothing loads. Tests: `markdown.test.ts` "escapes HTML, so a spec cannot add elements or handlers of its own", `projects-pane.test.ts` "shows the task's fields, and its spec as Markdown with its HTML as text" (Task 6), and `projects.mjs`'s "spec HTML stays text" in WebView2 (Task 9).
3. A page asks Dots to open a run it did not start (another dot's run, a made-up id), or a preview asks at all. Expected: refused; nothing opens. Tests: `apps_projects_test::a_dot_given_a_task_reads_it_reports_and_leaves_it_in_review` (Task 4's assertions on `run_link`), `AppFrame.test.tsx` "opens a run the page asks for, and refuses where the screen cannot open runs" (Task 7).
4. A Dots release whose Projects changes a migration that already ran on the person's data. Expected: the update is refused and logged; the active version and the data stay. Test: `apps_bundled_test::a_bundled_release_that_changes_a_migration_that_ran_is_refused` (Task 3).
5. The person assigns a task when Projects cannot start the dot (the hour's 20 starts used, schedules paused). Expected: the assignment stands, Activity says why the dot did not start, and the task waits for the dot to find it. Test: `assign.test.js` "a start the rules refuse is written in Activity, and the task waits" (Task 2).

## File Map

- Projects: `crates/dots-core/apps/projects/{README.md,app.json,logic.js,sample.sql}`, `migrations/0001_projects.sql`, `tests/{tasks,undo,assign}.test.js`, `ui/{index.html,main.js,projects.js,markdown.js,projects.css}` (new, Tasks 1, 2, 5, 6); `.gitattributes`.
- Core: `crates/dots-core/src/apps/bundled.rs` (new, Tasks 1 to 6); `src/apps/{mod,source,registry,kept,propose,wake,preview}.rs`, `src/{library,runtime}.rs`.
- Core tests: `crates/dots-core/tests/apps_{bundled,projects}_test.rs` (new); `tests/apps_{registry,preview}_test.rs`.
- Tauri: `apps/dots-app/src/{commands,contract,lib,app_frame}.rs`, `src/app_client.js`.
- UI: `ui/src/apps/{projects-fake.ts,projects-screen.test.ts,projects-pane.test.ts,markdown.test.ts}` (new); `ui/src/api/{types,commands}.ts`, the fixture, `ui/src/api/contract.test.ts`, `ui/src/lib/{app-bridge,query}.ts`, `ui/src/components/{AppFrame,AppReviewCard}.tsx`, `ui/src/screens/apps/{AppScreen,AppsScreen}.tsx`, tests beside them, `ui/tsconfig.json`.
- Smoke: `tools/smoke/projects.mjs` (new, Task 9), `tools/smoke/frame-leaks.mjs`.
- Spec: `docs/superpowers/specs/2026-10-23-app-library-design.md` (Task 10).

### Task 1: Projects comes with Dots: its data, tasks and their Activity

Spec §8 "Data", "Tools", the statuses and the Done rule, and §4: Projects in the app format, built into Dots and read with the checks a proposal gets, with its own tests (statuses, the Done rule, activity rows). `read_folder` is split so an app can be read from files held in memory.

**Files:**

- Create: `crates/dots-core/src/apps/bundled.rs`, `crates/dots-core/apps/projects/{README.md,app.json,logic.js,sample.sql,migrations/0001_projects.sql,tests/tasks.test.js,ui/index.html}`, `crates/dots-core/tests/apps_bundled_test.rs`
- Modify: `crates/dots-core/src/apps/{mod,source}.rs`, `.gitattributes`

**Interfaces:**

- Consumes: plan A's `source::{read_folder, AppSource}` (its checks), `tests::run_tests`, `data::{open_in_memory, migrate, load_sample, query}`.
- Produces: `source::read_files(&BTreeMap<String, Vec<u8>>) -> Result<AppSource>` (the same checks and bounds); `apps::bundled::{PROJECTS = "projects", projects_files() -> BTreeMap<String, Vec<u8>>, projects() -> Result<AppSource>, release() -> String}` (the release is `CARGO_PKG_VERSION`). Projects' tools: `list_projects`, `create_project`, `create_section`, `rename_section`, `list_tasks`, `get_task`, `create_task`, `update_task`, `add_subtask`, `set_subtask`, `comment`, `list_dots`; a task as `{ id, title, project_id, section_id, status, assignee: null | { kind: "person" } | { kind: "dot", id, name }, due_on, position, created_at, updated_at, completed_at }`; an Activity row as `{ id, at, actor: { kind, dot_id, name }, kind, field, before, after, text, run_id, undone_by }`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/apps/projects/tests/tasks.test.js`:**

```
// Projects' tasks: projects and sections, to-dos, statuses and the Done rule, assignees, due
// dates, subtasks, comments, and the Activity row every change writes.

const builder = { kind: "dot", id: "d1", name: "builder", run_id: "r1" };
const dots = [
  { id: "d1", name: "builder" },
  { id: "d2", name: "writer" },
];
const as = (actor) => ({ actor, dots });

function project() {
  const p = call("create_project", { name: "Website" });
  const s = call("create_section", { project_id: p.id, name: "To do" });
  return { p, s };
}

test("a project lists its sections, and its tasks in order", () => {
  const { p, s } = project();
  const later = call("create_section", { project_id: p.id, name: "Later" });
  call("create_task", { title: "Second", section_id: later.id });
  call("create_task", { title: "First", section_id: s.id });
  call("create_task", { title: "Also first", section_id: s.id });
  assert.deepEqual(
    call("list_projects", {}).map((x) => [x.name, x.sections.map((y) => y.name)]),
    [["Website", ["To do", "Later"]]],
  );
  const listed = call("list_tasks", { project_id: p.id });
  assert.deepEqual(
    listed.tasks.map((t) => t.title),
    ["First", "Also first", "Second"],
  );
  assert.equal(listed.more, false);
  assert.equal(listed.tasks[0].project_id, p.id);
});

test("a section is renamed, and a blank name is refused", () => {
  const { s } = project();
  assert.equal(call("rename_section", { id: s.id, name: " Doing " }).name, "Doing");
  assert.throws(() => call("rename_section", { id: s.id, name: "  " }), "cannot be empty");
  assert.throws(() => call("rename_section", { id: "nope", name: "X" }), "There is no section nope");
});

test("a to-do has no project and shows in my tasks with the person's tasks", () => {
  const { s } = project();
  const todo = call("create_task", { title: "Call the landlord" });
  const mine = call("create_task", { title: "Pricing copy", section_id: s.id, assignee: "person" });
  call("create_task", { title: "Signup form", section_id: s.id, assignee: "d1" }, { dots });
  assert.equal(todo.project_id, null);
  assert.deepEqual(
    call("list_tasks", { view: "my_tasks" }).tasks.map((t) => t.id),
    [todo.id, mine.id],
  );
  assert.deepEqual(
    call("list_tasks", { view: "assigned_to_dots" }, { dots }).tasks.map((t) => t.title),
    ["Signup form"],
  );
});

test("each change writes an Activity row with its actor and the values before and after", () => {
  const { s } = project();
  const t = call("create_task", { title: "Signup form", section_id: s.id });
  const out = call("update_task", { id: t.id, status: "in_progress", title: "Signup" }, as(builder));
  assert.equal(out.changes.length, 2);
  assert.equal(out.task.status, "in_progress");
  const rows = call("get_task", { id: t.id }).activity;
  assert.deepEqual(
    rows.map((r) => [r.kind, r.field, r.before, r.after, r.actor.kind, r.actor.name]),
    [
      ["change", "created", null, { title: "Signup form" }, "person", "You"],
      ["change", "title", "Signup form", "Signup", "dot", "builder"],
      ["change", "status", "to_do", "in_progress", "dot", "builder"],
    ],
  );
  assert.equal(rows[1].actor.dot_id, "d1");
  assert.equal(rows[1].at, "2026-01-01T09:00:00+00:00");
});

test("a field set to the value it holds writes no row", () => {
  const t = call("create_task", { title: "Same" });
  assert.equal(call("update_task", { id: t.id, title: "Same", status: "to_do" }).changes.length, 0);
  assert.equal(call("get_task", { id: t.id }).activity.length, 1);
});

test("a dot cannot mark a task Done, and nothing changes", () => {
  const t = call("create_task", { title: "Ship it" });
  assert.throws(
    () => call("update_task", { id: t.id, status: "done", title: "Shipped" }, as(builder)),
    "Only the person marks a task Done; set In review",
  );
  assert.throws(
    () => call("create_task", { title: "Done already", status: "done" }, as(builder)),
    "Only the person marks a task Done",
  );
  const now = call("get_task", { id: t.id });
  assert.equal(now.status, "to_do");
  assert.equal(now.title, "Ship it");
  call("update_task", { id: t.id, status: "in_review" }, as(builder));
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
});

test("the person marks Done, which sets completed_at; leaving Done clears it", () => {
  const t = call("create_task", { title: "Ship it" });
  call("update_task", { id: t.id, status: "done" }, { now: "2026-03-02T10:00:00+01:00" });
  assert.equal(call("get_task", { id: t.id }).completed_at, "2026-03-02T10:00:00+01:00");
  call("update_task", { id: t.id, status: "in_progress" });
  assert.equal(call("get_task", { id: t.id }).completed_at, null);
});

test("assignee: me is the calling dot, a dot is named by id or name, others are refused", () => {
  const t = call("create_task", { title: "Signup form" });
  call("update_task", { id: t.id, assignee: "me" }, as(builder));
  assert.deepEqual(call("get_task", { id: t.id }, { dots }).assignee, {
    kind: "dot",
    id: "d1",
    name: "builder",
  });
  call("update_task", { id: t.id, assignee: "writer" }, { dots });
  assert.equal(call("get_task", { id: t.id }, { dots }).assignee.id, "d2");
  assert.throws(
    () => call("update_task", { id: t.id, assignee: "ghost" }, { dots }),
    "No dot ghost uses Projects",
  );
  call("update_task", { id: t.id, assignee: "me" });
  assert.deepEqual(call("get_task", { id: t.id }).assignee, { kind: "person" });
  const rows = call("get_task", { id: t.id }).activity.filter((r) => r.field === "assignee");
  assert.deepEqual(rows[0].after, { kind: "dot", id: "d1", name: "builder" });
  assert.deepEqual(rows[1].before, { kind: "dot", id: "d1", name: "builder" });
});

test("the assignee of a dot that no longer uses Projects shows without a name", () => {
  const t = call("create_task", { title: "Signup form", assignee: "d1" }, { dots });
  assert.deepEqual(call("get_task", { id: t.id }, { dots: [] }).assignee, {
    kind: "dot",
    id: "d1",
    name: null,
  });
  assert.equal(call("list_tasks", { assignee: "d1" }, { dots: [] }).tasks.length, 1);
});

test("a due date is a real date or null", () => {
  const t = call("create_task", { title: "Rent", due_on: "2028-02-29" });
  assert.equal(t.due_on, "2028-02-29");
  for (const bad of ["2027-02-29", "2026-13-01", "tomorrow", "2026-1-2"]) {
    assert.throws(() => call("update_task", { id: t.id, due_on: bad }), "is not a date");
  }
  call("update_task", { id: t.id, due_on: null });
  assert.equal(call("get_task", { id: t.id }).due_on, null);
});

test("list_tasks filters by assignee, status and due date, and says when there are more", () => {
  const a = call("create_task", { title: "A", assignee: "person", due_on: "2026-10-01" });
  const b = call("create_task", { title: "B", assignee: "d1", due_on: "2026-11-01" }, { dots });
  call("create_task", { title: "C", status: "in_review" });
  assert.deepEqual(call("list_tasks", { assignee: "person" }).tasks.map((t) => t.id), [a.id]);
  assert.deepEqual(call("list_tasks", { assignee: "me" }, as(builder)).tasks.map((t) => t.id), [b.id]);
  assert.deepEqual(call("list_tasks", { assignee: "builder" }, { dots }).tasks.map((t) => t.id), [b.id]);
  assert.deepEqual(call("list_tasks", { status: "in_review" }).tasks.map((t) => t.title), ["C"]);
  assert.deepEqual(call("list_tasks", { due_before: "2026-10-15" }).tasks.map((t) => t.id), [a.id]);
  const limited = call("list_tasks", { limit: 2 });
  assert.equal(limited.tasks.length, 2);
  assert.equal(limited.more, true);
});

test("a task moved to a section of another project changes project", () => {
  const { s } = project();
  const other = call("create_project", { name: "Home" });
  const boxes = call("create_section", { project_id: other.id, name: "Boxes" });
  const t = call("create_task", { title: "Pack", section_id: s.id });
  call("update_task", { id: t.id, section_id: boxes.id });
  const moved = call("get_task", { id: t.id });
  assert.equal(moved.project.name, "Home");
  assert.equal(moved.section.name, "Boxes");
  const row = moved.activity[moved.activity.length - 1];
  assert.equal(row.field, "section");
  assert.equal(row.before.project, "Website");
  assert.equal(row.after.section, "Boxes");
  call("update_task", { id: t.id, section_id: null });
  assert.equal(call("get_task", { id: t.id }).project.name, "Home");
  assert.throws(() => call("create_task", { title: "X", project_id: other.id, section_id: s.id }), "is not in the project");
});

test("subtasks are added, checked and renamed, each with a row", () => {
  const t = call("create_task", { title: "Signup form" });
  const markup = call("add_subtask", { task_id: t.id, title: "Form markup" });
  call("add_subtask", { task_id: t.id, title: "Validation" });
  call("set_subtask", { id: markup.id, done: true }, as(builder));
  call("set_subtask", { id: markup.id, title: "Markup" });
  const got = call("get_task", { id: t.id });
  assert.deepEqual(got.subtasks.map((s) => [s.title, s.done]), [["Markup", true], ["Validation", false]]);
  assert.deepEqual(
    got.activity.slice(1).map((r) => [r.field, r.before, r.after]),
    [
      [`subtask:${markup.id}`, null, { title: "Form markup", done: false }],
      [`subtask:${got.subtasks[1].id}`, null, { title: "Validation", done: false }],
      [`subtask:${markup.id}`, { title: "Form markup", done: false }, { title: "Form markup", done: true }],
      [`subtask:${markup.id}`, { title: "Form markup", done: true }, { title: "Markup", done: true }],
    ],
  );
  assert.equal(call("list_tasks", {}).tasks[0].subtasks.done, 1);
});

test("a comment is an Activity row; get_task gives the last 20 rows, oldest first", () => {
  const t = call("create_task", { title: "Talk" });
  for (let i = 1; i <= 25; i++) call("comment", { task_id: t.id, text: `note ${i}` }, as(builder));
  const got = call("get_task", { id: t.id });
  assert.equal(got.activity.length, 20);
  assert.equal(got.activity_total, 26);
  assert.equal(got.activity[0].text, "note 6");
  assert.equal(got.activity[19].text, "note 25");
  assert.equal(got.activity[19].kind, "comment");
  assert.equal(got.activity[19].actor.name, "builder");
  assert.equal(call("get_task", { id: t.id, activity: 500 }).activity.length, 26);
  assert.throws(() => call("comment", { task_id: t.id, text: " " }), "cannot be empty");
});

test("list_dots gives the dots that use Projects", () => {
  assert.deepEqual(call("list_dots", {}, { dots }), dots);
});
```

**Create `crates/dots-core/tests/apps_bundled_test.rs`:**

```rust
//! The apps that come with Dots (app library spec section 8): Projects is built into Dots from
//! its folder, passes the checks a dot's proposal gets, and its own tests pass.

use std::collections::BTreeMap;
use std::path::Path;

use dots_core::apps::bundled::{projects, projects_files, PROJECTS};
use dots_core::apps::data;
use dots_core::apps::source::read_files;
use dots_core::apps::tests::run_tests;

/// Every file below `dir`, by its path relative to `root` with `/`.
fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&path).unwrap());
        }
    }
}

#[test]
fn projects_holds_every_file_of_its_folder_and_only_lf_text() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("apps")
        .join(PROJECTS);
    let mut on_disk = BTreeMap::new();
    walk(&root, &root, &mut on_disk);
    let built_in = projects_files();
    assert_eq!(
        built_in.keys().collect::<Vec<_>>(),
        on_disk.keys().collect::<Vec<_>>(),
        "list each file of apps/projects in bundled::PROJECTS_FILES"
    );
    for (path, bytes) in &built_in {
        // A CRLF checkout would change the migrations' bytes, and with them what a change to
        // Projects must keep (.gitattributes keeps the folder LF).
        assert!(!bytes.contains(&b'\r'), "{path} holds a carriage return");
        assert!(std::str::from_utf8(bytes).is_ok(), "{path} is not UTF-8");
    }
}

#[test]
fn projects_passes_the_checks_a_proposal_gets_and_its_own_tests() {
    let src = projects().unwrap();
    assert_eq!(src.manifest.name, PROJECTS);
    assert_eq!(src.manifest.title, "Projects");
    assert_eq!(read_files(&projects_files()).unwrap(), src);
    let report = run_tests(&src);
    assert!(report.passed(), "{}", report.problem().unwrap_or_default());
    assert!(report.results.len() >= 15, "{} tests", report.results.len());
}

#[test]
fn projects_migrations_run_and_its_sample_loads() {
    let src = projects().unwrap();
    let mut conn = data::open_in_memory().unwrap();
    assert_eq!(
        data::migrate(&mut conn, &src.migrations).unwrap(),
        src.migrations.len()
    );
    data::load_sample(&conn, src.sample.as_deref().unwrap()).unwrap();
    let tasks = data::query(&conn, "SELECT COUNT(*) AS n FROM tasks", &[]).unwrap();
    assert_eq!(tasks[0]["n"], 4);
}

#[test]
fn an_app_read_from_memory_is_bounded_like_a_folder() {
    let mut files = projects_files();
    files.insert("ui/big.bin".into(), vec![0; 2 * 1024 * 1024]);
    let err = read_files(&files).unwrap_err().to_string();
    assert!(err.contains("more than 2 MB"), "{err}");
    let mut files = projects_files();
    files.insert("notes.txt".into(), b"hi".to_vec());
    let err = read_files(&files).unwrap_err().to_string();
    assert!(err.contains("notes.txt: an app holds"), "{err}");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test`
Expected: FAIL to compile: no `bundled` in `dots_core::apps`, no `read_files` in `source`.

- [ ] **Step 3: Add Projects and read it from memory**

The `.gitattributes` line comes first, so the new files are checked out LF.

**In `.gitattributes`, replace:**

```
*.sql text eol=lf
```

**with:**

```
*.sql text eol=lf
# The apps that come with Dots are built into it byte for byte: their text is LF everywhere.
crates/dots-core/apps/** text eol=lf
```

**Create `crates/dots-core/apps/projects/README.md`:**

```markdown
# Projects

Projects, tasks and to-dos, shared by the person and their dots (app library spec section 8).
It comes with Dots, which updates it with each release until the person accepts a dot's change to
it; from then on it is the person's.

- `migrations/`: the data. A change keeps these files byte for byte and adds new ones after them.
- `logic.js`: the tools. Every change to a task or a subtask writes an Activity row with the
  values before and after; a dot cannot mark a task Done.
- `tests/`: run with every proposed change.
```

**Create `crates/dots-core/apps/projects/app.json`:**

```json
{
  "name": "projects",
  "title": "Projects",
  "description": "Projects, tasks and to-dos, shared by you and your dots",
  "tools": [
    {
      "name": "list_projects",
      "description": "Every project with its sections, in order",
      "input": { "type": "object", "properties": {}, "additionalProperties": false }
    },
    {
      "name": "create_project",
      "description": "Add a project",
      "input": {
        "type": "object",
        "properties": { "name": { "type": "string", "minLength": 1, "maxLength": 100 } },
        "required": ["name"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "create_section",
      "description": "Add a section at the end of a project",
      "input": {
        "type": "object",
        "properties": {
          "project_id": { "type": "string" },
          "name": { "type": "string", "minLength": 1, "maxLength": 100 }
        },
        "required": ["project_id", "name"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "rename_section",
      "description": "Rename a section",
      "input": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "name": { "type": "string", "minLength": 1, "maxLength": 100 }
        },
        "required": ["id", "name"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "list_tasks",
      "description": "Tasks without their spec, filtered. assignee: \"me\" (you), \"person\", or a dot's id or name. view: \"my_tasks\" (the person's tasks and unassigned to-dos) or \"assigned_to_dots\". due_before: YYYY-MM-DD. At most limit tasks (100 by default); more says whether there are others.",
      "input": {
        "type": "object",
        "properties": {
          "project_id": { "type": "string" },
          "assignee": { "type": "string" },
          "status": { "type": "string", "enum": ["to_do", "in_progress", "in_review", "done"] },
          "due_before": { "type": "string" },
          "view": { "type": "string", "enum": ["my_tasks", "assigned_to_dots"] },
          "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
        },
        "additionalProperties": false
      }
    },
    {
      "name": "get_task",
      "description": "A task with its spec, subtasks and recent activity (the last 20 rows, or the last activity rows)",
      "input": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "activity": { "type": "integer", "minimum": 1, "maximum": 500 }
        },
        "required": ["id"],
        "additionalProperties": false
      }
    },
    {
      "name": "create_task",
      "description": "Add a task: in a project's section, in a project, or with neither as a to-do. assignee: \"me\" (you), \"person\", a dot's id or name, or null. due_on: YYYY-MM-DD or null. A dot cannot create a task as done.",
      "input": {
        "type": "object",
        "properties": {
          "title": { "type": "string", "minLength": 1, "maxLength": 200 },
          "project_id": { "type": ["string", "null"] },
          "section_id": { "type": ["string", "null"] },
          "spec": { "type": "string", "maxLength": 20000 },
          "assignee": { "type": ["string", "null"] },
          "due_on": { "type": ["string", "null"] },
          "status": { "type": "string", "enum": ["to_do", "in_progress", "in_review", "done"] }
        },
        "required": ["title"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "update_task",
      "description": "Change a task's fields; each change shows in its Activity. status: to_do, in_progress, in_review or done (only the person marks a task done: set in_review when your work is finished). assignee: \"me\" (you), \"person\", a dot's id or name, or null. due_on: YYYY-MM-DD or null. section_id: a section of any project (the task moves to that project), or null.",
      "input": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "title": { "type": "string", "minLength": 1, "maxLength": 200 },
          "spec": { "type": "string", "maxLength": 20000 },
          "status": { "type": "string", "enum": ["to_do", "in_progress", "in_review", "done"] },
          "assignee": { "type": ["string", "null"] },
          "due_on": { "type": ["string", "null"] },
          "section_id": { "type": ["string", "null"] }
        },
        "required": ["id"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "add_subtask",
      "description": "Add a subtask at the end of a task's checklist",
      "input": {
        "type": "object",
        "properties": {
          "task_id": { "type": "string" },
          "title": { "type": "string", "minLength": 1, "maxLength": 200 }
        },
        "required": ["task_id", "title"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "set_subtask",
      "description": "Check or uncheck a subtask, or rename it",
      "input": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "done": { "type": "boolean" },
          "title": { "type": "string", "minLength": 1, "maxLength": 200 }
        },
        "required": ["id"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "comment",
      "description": "Add a comment to a task's Activity: progress, a question, what you found",
      "input": {
        "type": "object",
        "properties": {
          "task_id": { "type": "string" },
          "text": { "type": "string", "minLength": 1, "maxLength": 4000 }
        },
        "required": ["task_id", "text"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "list_dots",
      "description": "The dots a task can be assigned to: those that use Projects",
      "input": { "type": "object", "properties": {}, "additionalProperties": false }
    }
  ]
}
```

**Create `crates/dots-core/apps/projects/logic.js`:**

```
// Projects (app library spec section 8): projects, sections, tasks and to-dos, shared by the
// person and their dots. Every change to a task or a subtask writes an Activity row with the
// values before and after, in the same call. A dot may not mark a task Done.

const ONLY_PERSON_DONE = "Only the person marks a task Done; set In review";
/// The Activity rows get_task gives when it is not told how many.
const RECENT_ACTIVITY = 20;
/// The tasks list_tasks gives when it is not told how many.
const LIST_LIMIT = 100;

const fail = (message) => {
  throw new Error(message);
};
const one = (ctx, sql, params) => ctx.db.query(sql, params)[0] ?? null;
const exec = (ctx, sql, params) => ctx.db.exec(sql, params);
const json = (value) => (value === undefined ? null : JSON.stringify(value));
const parse = (text) => (text === null ? null : JSON.parse(text));

function trimmed(text, what) {
  const t = text.trim();
  if (t === "") fail(`The ${what} cannot be empty`);
  return t;
}

/// Who calls, as an Activity row names them.
function who(ctx) {
  const a = ctx.actor;
  if (a.kind === "dot") return { kind: "dot", dot_id: a.id, name: a.name };
  return { kind: "person", dot_id: null, name: "You" };
}

const isDot = (ctx) => ctx.actor.kind === "dot";

function projectRow(ctx, id) {
  return one(ctx, "SELECT * FROM projects WHERE id = ?", [id]) ?? fail(`There is no project ${id}`);
}

function sectionRow(ctx, id) {
  return one(ctx, "SELECT * FROM sections WHERE id = ?", [id]) ?? fail(`There is no section ${id}`);
}

function taskRow(ctx, id) {
  return one(ctx, "SELECT * FROM tasks WHERE id = ?", [id]) ?? fail(`There is no task ${id}`);
}

function subtaskRow(ctx, id) {
  return one(ctx, "SELECT * FROM subtasks WHERE id = ?", [id]) ?? fail(`There is no subtask ${id}`);
}

/// The next position at the end of a task list: the project's section, or the to-dos.
function nextTaskPosition(ctx, projectId, sectionId) {
  return one(
    ctx,
    "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM tasks WHERE project_id IS ? AND section_id IS ?",
    [projectId, sectionId],
  ).p;
}

/// The dot `ref` names, by id or by name, among those that use Projects.
function dotNamed(ctx, ref) {
  const dots = ctx.dots.list();
  return dots.find((d) => d.id === ref) ?? dots.find((d) => d.name === ref) ?? null;
}

/// The assignee an input names: null, the person, or a dot that uses Projects. "me" is the
/// caller.
function assigneeOf(ctx, ref) {
  if (ref === null) return null;
  if (ref === "person") return { kind: "person" };
  if (ref === "me") return isDot(ctx) ? { kind: "dot", id: ctx.actor.id } : { kind: "person" };
  const dot = dotNamed(ctx, ref);
  if (dot === null) fail(`No dot ${ref} uses Projects; list_dots names those that do`);
  return { kind: "dot", id: dot.id };
}

const DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// A due date: null or YYYY-MM-DD.
function dueOf(value) {
  if (value === null) return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (m !== null) {
    const [year, month, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
    const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
    const last = month === 2 && leap ? 29 : DAYS[month - 1];
    if (month >= 1 && month <= 12 && day >= 1 && day <= last) return value;
  }
  return fail(`The due date ${value} is not a date: write YYYY-MM-DD`);
}

/// Where a task stands: its project and section, or null for a to-do.
function placeOf(task) {
  if (task.project_id === null) return null;
  return { project_id: task.project_id, section_id: task.section_id };
}

/// The value of a task's field, as a change row holds it (without the names shown with it).
function valueOf(ctx, task, field) {
  switch (field) {
    case "title":
    case "spec":
    case "status":
    case "due_on":
      return task[field];
    case "assignee":
      if (task.assignee_kind === null) return null;
      if (task.assignee_kind === "person") return { kind: "person" };
      return { kind: "dot", id: task.assignee_dot_id };
    case "section":
      return placeOf(task);
    default: {
      const id = field.slice("subtask:".length);
      const s = one(ctx, "SELECT title, done FROM subtasks WHERE id = ? AND task_id = ?", [
        id,
        task.id,
      ]);
      return s === null ? null : { title: s.title, done: s.done === 1 };
    }
  }
}

/// `value` without the names a change row shows with it, so two values compare by what they are.
function bare(value) {
  if (value === null || typeof value !== "object") return value;
  const out = {};
  for (const key of Object.keys(value).sort()) {
    if (key !== "name" && key !== "project" && key !== "section") out[key] = value[key];
  }
  return out;
}

const same = (a, b) => JSON.stringify(bare(a)) === JSON.stringify(bare(b));

/// `value` with the names the Activity shows: a dot's, a project's and a section's.
function named(ctx, field, value) {
  if (value === null) return null;
  if (field === "assignee" && value.kind === "dot") {
    const dot = dotNamed(ctx, value.id);
    const name = dot?.name ?? (isDot(ctx) && ctx.actor.id === value.id ? ctx.actor.name : null);
    return { ...value, name };
  }
  if (field === "section") {
    const project = one(ctx, "SELECT name FROM projects WHERE id = ?", [value.project_id]);
    const section =
      value.section_id === null
        ? null
        : one(ctx, "SELECT name FROM sections WHERE id = ?", [value.section_id]);
    return { ...value, project: project?.name ?? null, section: section?.name ?? null };
  }
  return value;
}

/// Writes `value` into a task's field.
function apply(ctx, task, field, value) {
  const now = ctx.now;
  switch (field) {
    case "title":
    case "spec":
    case "due_on":
      exec(ctx, `UPDATE tasks SET ${field} = ?, updated_at = ? WHERE id = ?`, [value, now, task.id]);
      return;
    case "status":
      exec(ctx, "UPDATE tasks SET status = ?, completed_at = ?, updated_at = ? WHERE id = ?", [
        value,
        value === "done" ? now : null,
        now,
        task.id,
      ]);
      return;
    case "assignee":
      exec(ctx, "UPDATE tasks SET assignee_kind = ?, assignee_dot_id = ?, updated_at = ? WHERE id = ?", [
        value?.kind ?? null,
        value?.kind === "dot" ? value.id : null,
        now,
        task.id,
      ]);
      return;
    case "section": {
      const project = value?.project_id ?? null;
      const section = value?.section_id ?? null;
      exec(
        ctx,
        "UPDATE tasks SET project_id = ?, section_id = ?, position = ?, updated_at = ? WHERE id = ?",
        [project, section, nextTaskPosition(ctx, project, section), now, task.id],
      );
      return;
    }
    default: {
      const id = field.slice("subtask:".length);
      if (value === null) {
        exec(ctx, "DELETE FROM subtasks WHERE id = ?", [id]);
      } else if (one(ctx, "SELECT id FROM subtasks WHERE id = ?", [id]) !== null) {
        exec(ctx, "UPDATE subtasks SET title = ?, done = ? WHERE id = ?", [
          value.title,
          value.done ? 1 : 0,
          id,
        ]);
      } else {
        const position = one(
          ctx,
          "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM subtasks WHERE task_id = ?",
          [task.id],
        ).p;
        exec(ctx, "INSERT INTO subtasks (id, task_id, title, done, position) VALUES (?, ?, ?, ?, ?)", [
          id,
          task.id,
          value.title,
          value.done ? 1 : 0,
          position,
        ]);
      }
      exec(ctx, "UPDATE tasks SET updated_at = ? WHERE id = ?", [now, task.id]);
    }
  }
}

/// Writes an Activity row of the task `taskId` by `actor`; returns its id.
function record(ctx, taskId, actor, row) {
  const id = ctx.id();
  exec(
    ctx,
    "INSERT INTO activity (id, task_id, at, actor_kind, actor_dot_id, actor_name, kind, field, " +
      "before_json, after_json, text, run_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    [
      id,
      taskId,
      ctx.now,
      actor.kind,
      actor.dot_id,
      actor.name,
      row.kind,
      row.field ?? null,
      json(row.before),
      json(row.after),
      row.text ?? null,
      row.run_id ?? null,
    ],
  );
  return id;
}

/// Sets a task's field to `after` as `actor`, with its Activity row. Returns the row's id, or
/// null when the field holds `after` already.
function change(ctx, taskId, field, after, actor) {
  const task = taskRow(ctx, taskId);
  const before = valueOf(ctx, task, field);
  if (same(before, after)) return null;
  const shownBefore = named(ctx, field, before);
  apply(ctx, task, field, after);
  return record(ctx, taskId, actor, {
    kind: "change",
    field,
    before: shownBefore,
    after: named(ctx, field, after),
  });
}

function assigneeOut(task, dots) {
  if (task.assignee_kind === null) return null;
  if (task.assignee_kind === "person") return { kind: "person" };
  const dot = dots.find((d) => d.id === task.assignee_dot_id);
  return { kind: "dot", id: task.assignee_dot_id, name: dot?.name ?? null };
}

function taskOut(task, dots) {
  return {
    id: task.id,
    title: task.title,
    project_id: task.project_id,
    section_id: task.section_id,
    status: task.status,
    assignee: assigneeOut(task, dots),
    due_on: task.due_on,
    position: task.position,
    created_at: task.created_at,
    updated_at: task.updated_at,
    completed_at: task.completed_at,
  };
}

function activityOut(r) {
  return {
    id: r.id,
    at: r.at,
    actor: { kind: r.actor_kind, dot_id: r.actor_dot_id, name: r.actor_name },
    kind: r.kind,
    field: r.field,
    before: parse(r.before_json),
    after: parse(r.after_json),
    text: r.text,
    run_id: r.run_id,
    undone_by: r.undone_by,
  };
}

export function list_projects(_, ctx) {
  const sections = ctx.db.query(
    "SELECT id, project_id, name, position FROM sections ORDER BY position, rowid",
    [],
  );
  return ctx.db
    .query(
      "SELECT id, name, position FROM projects WHERE archived_at IS NULL ORDER BY position, rowid",
      [],
    )
    .map((p) => ({
      ...p,
      sections: sections
        .filter((s) => s.project_id === p.id)
        .map(({ id, name, position }) => ({ id, name, position })),
    }));
}

export function create_project({ name }, ctx) {
  const id = ctx.id();
  const position = one(ctx, "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM projects", []).p;
  const shown = trimmed(name, "project's name");
  exec(ctx, "INSERT INTO projects (id, name, position) VALUES (?, ?, ?)", [id, shown, position]);
  return { id, name: shown, position, sections: [] };
}

export function create_section({ project_id, name }, ctx) {
  projectRow(ctx, project_id);
  const id = ctx.id();
  const position = one(
    ctx,
    "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM sections WHERE project_id = ?",
    [project_id],
  ).p;
  const shown = trimmed(name, "section's name");
  exec(ctx, "INSERT INTO sections (id, project_id, name, position) VALUES (?, ?, ?, ?)", [
    id,
    project_id,
    shown,
    position,
  ]);
  return { id, name: shown, position };
}

export function rename_section({ id, name }, ctx) {
  sectionRow(ctx, id);
  const shown = trimmed(name, "section's name");
  exec(ctx, "UPDATE sections SET name = ? WHERE id = ?", [shown, id]);
  return { id, name: shown };
}

export function list_tasks(input, ctx) {
  const where = ["(t.project_id IS NULL OR p.archived_at IS NULL)"];
  const params = [];
  if (input.project_id !== undefined) {
    where.push("t.project_id = ?");
    params.push(input.project_id);
  }
  if (input.assignee !== undefined) {
    if (input.assignee === "person" || (input.assignee === "me" && !isDot(ctx))) {
      where.push("t.assignee_kind = 'person'");
    } else {
      // A dot that no longer uses Projects is still found by its id.
      const dot = input.assignee === "me" ? { id: ctx.actor.id } : dotNamed(ctx, input.assignee);
      where.push("t.assignee_kind = 'dot' AND t.assignee_dot_id = ?");
      params.push(dot?.id ?? input.assignee);
    }
  }
  if (input.status !== undefined) {
    where.push("t.status = ?");
    params.push(input.status);
  }
  if (input.due_before !== undefined) {
    where.push("t.due_on IS NOT NULL AND t.due_on < ?");
    params.push(dueOf(input.due_before));
  }
  if (input.view === "my_tasks") {
    where.push("(t.assignee_kind = 'person' OR (t.project_id IS NULL AND t.assignee_kind IS NULL))");
  } else if (input.view === "assigned_to_dots") {
    where.push("t.assignee_kind = 'dot'");
  }
  const limit = input.limit ?? LIST_LIMIT;
  const rows = ctx.db.query(
    "SELECT t.*, " +
      "(SELECT COUNT(*) FROM subtasks WHERE task_id = t.id) AS subtasks_total, " +
      "(SELECT COUNT(*) FROM subtasks WHERE task_id = t.id AND done = 1) AS subtasks_done " +
      "FROM tasks t LEFT JOIN projects p ON p.id = t.project_id " +
      "LEFT JOIN sections s ON s.id = t.section_id " +
      `WHERE ${where.join(" AND ")} ` +
      "ORDER BY t.project_id IS NOT NULL, p.position, s.position IS NOT NULL, s.position, " +
      "t.position, t.rowid LIMIT ?",
    [...params, limit + 1],
  );
  const dots = ctx.dots.list();
  return {
    tasks: rows.slice(0, limit).map((t) => ({
      ...taskOut(t, dots),
      subtasks: { done: t.subtasks_done, total: t.subtasks_total },
    })),
    more: rows.length > limit,
  };
}

export function get_task({ id, activity }, ctx) {
  const task = taskRow(ctx, id);
  const project =
    task.project_id === null
      ? null
      : one(ctx, "SELECT id, name FROM projects WHERE id = ?", [task.project_id]);
  const section =
    task.section_id === null
      ? null
      : one(ctx, "SELECT id, name FROM sections WHERE id = ?", [task.section_id]);
  const subtasks = ctx.db
    .query("SELECT id, title, done FROM subtasks WHERE task_id = ? ORDER BY position, rowid", [id])
    .map((s) => ({ id: s.id, title: s.title, done: s.done === 1 }));
  const rows = ctx.db.query(
    "SELECT * FROM (SELECT rowid AS seq, * FROM activity WHERE task_id = ? ORDER BY rowid DESC " +
      "LIMIT ?) ORDER BY seq",
    [id, activity ?? RECENT_ACTIVITY],
  );
  const total = one(ctx, "SELECT COUNT(*) AS n FROM activity WHERE task_id = ?", [id]).n;
  return {
    ...taskOut(task, ctx.dots.list()),
    spec: task.spec,
    project,
    section,
    subtasks,
    activity: rows.map(activityOut),
    activity_total: total,
  };
}

export function create_task(input, ctx) {
  const actor = who(ctx);
  const title = trimmed(input.title, "task's title");
  let project = input.project_id ?? null;
  const section = input.section_id ?? null;
  if (section !== null) {
    const s = sectionRow(ctx, section);
    if (project !== null && project !== s.project_id) {
      fail(`The section ${section} is not in the project ${project}`);
    }
    project = s.project_id;
  } else if (project !== null) {
    projectRow(ctx, project);
  }
  const status = input.status ?? "to_do";
  if (status === "done" && isDot(ctx)) fail(ONLY_PERSON_DONE);
  const assignee = input.assignee === undefined ? null : assigneeOf(ctx, input.assignee);
  const due = input.due_on === undefined ? null : dueOf(input.due_on);
  const id = ctx.id();
  exec(
    ctx,
    "INSERT INTO tasks (id, project_id, section_id, title, spec, assignee_kind, assignee_dot_id, " +
      "due_on, status, position, created_at, updated_at, completed_at) " +
      "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    [
      id,
      project,
      section,
      title,
      input.spec ?? "",
      assignee?.kind ?? null,
      assignee?.kind === "dot" ? assignee.id : null,
      due,
      status,
      nextTaskPosition(ctx, project, section),
      ctx.now,
      ctx.now,
      status === "done" ? ctx.now : null,
    ],
  );
  record(ctx, id, actor, { kind: "change", field: "created", before: null, after: { title } });
  return taskOut(taskRow(ctx, id), ctx.dots.list());
}

export function update_task(input, ctx) {
  const actor = who(ctx);
  const task = taskRow(ctx, input.id);
  const wanted = [];
  if (input.title !== undefined) wanted.push(["title", trimmed(input.title, "task's title")]);
  if (input.spec !== undefined) wanted.push(["spec", input.spec]);
  if (input.status !== undefined) {
    if (input.status === "done" && isDot(ctx)) fail(ONLY_PERSON_DONE);
    wanted.push(["status", input.status]);
  }
  if (input.assignee !== undefined) wanted.push(["assignee", assigneeOf(ctx, input.assignee)]);
  if (input.due_on !== undefined) wanted.push(["due_on", dueOf(input.due_on)]);
  if (input.section_id !== undefined) {
    const place =
      input.section_id === null
        ? { project_id: task.project_id, section_id: null }
        : { project_id: sectionRow(ctx, input.section_id).project_id, section_id: input.section_id };
    wanted.push(["section", place.project_id === null ? null : place]);
  }
  const changes = [];
  for (const [field, value] of wanted) {
    const row = change(ctx, task.id, field, value, actor);
    if (row !== null) changes.push(row);
  }
  return { task: taskOut(taskRow(ctx, task.id), ctx.dots.list()), changes };
}

export function add_subtask({ task_id, title }, ctx) {
  taskRow(ctx, task_id);
  const id = ctx.id();
  const shown = trimmed(title, "subtask's title");
  change(ctx, task_id, `subtask:${id}`, { title: shown, done: false }, who(ctx));
  return { id, title: shown, done: false };
}

export function set_subtask({ id, done, title }, ctx) {
  const s = subtaskRow(ctx, id);
  const after = {
    title: title === undefined ? s.title : trimmed(title, "subtask's title"),
    done: done ?? s.done === 1,
  };
  change(ctx, s.task_id, `subtask:${id}`, after, who(ctx));
  return { id, ...after };
}

export function comment({ task_id, text }, ctx) {
  taskRow(ctx, task_id);
  const id = record(ctx, task_id, who(ctx), { kind: "comment", text: trimmed(text, "comment") });
  return { id };
}

export function list_dots(_, ctx) {
  return ctx.dots.list();
}
```

**Create `crates/dots-core/apps/projects/migrations/0001_projects.sql`:**

```sql
-- Projects' data (app library spec section 8). Ids are ctx.id() values; times are ctx.now
-- (RFC 3339); a due date is YYYY-MM-DD.
CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  archived_at TEXT
);

CREATE TABLE sections (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects (id),
  name TEXT NOT NULL,
  position INTEGER NOT NULL
);

-- A task with no project is a to-do.
CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  project_id TEXT REFERENCES projects (id),
  section_id TEXT REFERENCES sections (id),
  title TEXT NOT NULL,
  spec TEXT NOT NULL DEFAULT '',
  assignee_kind TEXT CHECK (assignee_kind IN ('person', 'dot')),
  assignee_dot_id TEXT,
  due_on TEXT,
  status TEXT NOT NULL CHECK (status IN ('to_do', 'in_progress', 'in_review', 'done')),
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);

CREATE TABLE subtasks (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks (id),
  title TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL
);

-- Every change to a task or a subtask, every comment and every run of a dot on a task, oldest
-- first by rowid. A change's field is title, spec, status, assignee, due_on, section or
-- subtask:<id>, with the values before and after as JSON; undone_by names the change that undid
-- it. A run row's field is started, not_started or ended.
CREATE TABLE activity (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks (id),
  at TEXT NOT NULL,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('person', 'dot')),
  actor_dot_id TEXT,
  actor_name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('change', 'comment', 'run')),
  field TEXT,
  before_json TEXT,
  after_json TEXT,
  text TEXT,
  run_id TEXT,
  undone_by TEXT
);

CREATE INDEX tasks_by_project ON tasks (project_id, section_id, position);
CREATE INDEX subtasks_by_task ON subtasks (task_id, position);
CREATE INDEX activity_by_task ON activity (task_id);
```

**Create `crates/dots-core/apps/projects/sample.sql`:**

```sql
-- The review card's preview of a change to Projects runs on these rows.
INSERT INTO projects (id, name, position) VALUES ('p1', 'Website relaunch', 1);
INSERT INTO projects (id, name, position) VALUES ('p2', 'Home move', 2);
INSERT INTO sections (id, project_id, name, position) VALUES ('s1', 'p1', 'To do', 1);
INSERT INTO sections (id, project_id, name, position) VALUES ('s2', 'p1', 'Done', 2);
INSERT INTO sections (id, project_id, name, position) VALUES ('s3', 'p2', 'Boxes', 1);
INSERT INTO tasks (id, project_id, section_id, title, spec, assignee_kind, assignee_dot_id, due_on,
  status, position, created_at, updated_at, completed_at)
VALUES
  ('t1', 'p1', 's1', 'Write pricing page copy', 'Three plans; keep it short.', 'person', NULL,
   '2026-10-28', 'to_do', 1, '2026-10-20T09:00:00+00:00', '2026-10-20T09:00:00+00:00', NULL),
  ('t2', 'p1', 's1', 'Build signup form',
   'Email and password, inline errors, posts to `/api/signup`.', NULL, NULL, '2026-10-25',
   'in_progress', 2, '2026-10-20T09:05:00+00:00', '2026-10-21T10:00:00+00:00', NULL),
  ('t3', 'p1', 's2', 'Pick hosting', '', 'person', NULL, '2026-10-20', 'done', 1,
   '2026-10-18T09:00:00+00:00', '2026-10-20T16:00:00+00:00', '2026-10-20T16:00:00+00:00'),
  ('t4', NULL, NULL, 'Call the landlord', '', NULL, NULL, NULL, 'to_do', 1,
   '2026-10-21T08:00:00+00:00', '2026-10-21T08:00:00+00:00', NULL);
INSERT INTO subtasks (id, task_id, title, done, position) VALUES
  ('u1', 't2', 'Form markup', 1, 1),
  ('u2', 't2', 'Validation', 0, 2),
  ('u3', 't2', 'Tests', 0, 3);
INSERT INTO activity (id, task_id, at, actor_kind, actor_dot_id, actor_name, kind, field,
  before_json, after_json, text, run_id)
VALUES
  ('a1', 't2', '2026-10-20T09:05:00+00:00', 'person', NULL, 'You', 'change', 'created', NULL,
   '{"title":"Build signup form"}', NULL, NULL),
  ('a2', 't2', '2026-10-21T10:00:00+00:00', 'person', NULL, 'You', 'change', 'subtask:u1',
   '{"title":"Form markup","done":false}', '{"title":"Form markup","done":true}', NULL, NULL),
  ('a3', 't2', '2026-10-21T10:05:00+00:00', 'person', NULL, 'You', 'comment', NULL, NULL, NULL,
   'Match the pricing page style.', NULL);
```

**Create `crates/dots-core/apps/projects/ui/index.html`:**

```
<!doctype html>
<meta charset="utf-8">
<title>Projects</title>
<p>Projects</p>
```

**Create `crates/dots-core/src/apps/bundled.rs`:**

```rust
//! The apps that come with Dots (app library spec section 8): Projects, whose files are built into
//! Dots from `crates/dots-core/apps/projects/`. A file added to that folder is listed in
//! `PROJECTS_FILES` too; a test checks that the two agree.

use std::collections::BTreeMap;

use super::source::{self, AppSource};
use crate::Result;

/// The name of the Projects app.
pub const PROJECTS: &str = "projects";

/// Projects' files, by their path in the app folder.
const PROJECTS_FILES: &[(&str, &[u8])] = &[
    ("README.md", include_bytes!("../../apps/projects/README.md")),
    ("app.json", include_bytes!("../../apps/projects/app.json")),
    ("logic.js", include_bytes!("../../apps/projects/logic.js")),
    (
        "migrations/0001_projects.sql",
        include_bytes!("../../apps/projects/migrations/0001_projects.sql"),
    ),
    (
        "sample.sql",
        include_bytes!("../../apps/projects/sample.sql"),
    ),
    (
        "tests/tasks.test.js",
        include_bytes!("../../apps/projects/tests/tasks.test.js"),
    ),
    (
        "ui/index.html",
        include_bytes!("../../apps/projects/ui/index.html"),
    ),
];

/// The Dots release the bundled apps come with: `AppOrigin::Bundled`'s `release`.
pub fn release() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Projects' files as Dots holds them.
pub fn projects_files() -> BTreeMap<String, Vec<u8>> {
    PROJECTS_FILES
        .iter()
        .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
        .collect()
}

/// Projects, read and checked as a proposal's folder is (`source::read_files`). Blocks.
pub fn projects() -> Result<AppSource> {
    source::read_files(&projects_files())
}
```

**In `crates/dots-core/src/apps/mod.rs`, replace:**

```rust

pub mod call;
```

**with:**

```rust

pub mod bundled;
pub mod call;
```

**In `crates/dots-core/src/apps/source.rs`, replace:**

```rust
pub fn read_folder(dir: &Path) -> Result<AppSource> {
    let files = walk(dir)?;
    let text = |path: &str| -> Result<String> {
```

**with:**

```rust
pub fn read_folder(dir: &Path) -> Result<AppSource> {
    read_files(&walk(dir)?)
}

/// `read_folder` for an app's files held in memory, by their path relative to the app folder
/// with `/` (the apps that come with Dots, `apps::bundled`). The same checks, the bounds
/// included. Blocks.
pub fn read_files(files: &BTreeMap<String, Vec<u8>>) -> Result<AppSource> {
    let total: u64 = files.values().map(|b| b.len() as u64).sum();
    if total > MAX_APP_BYTES {
        return invalid(format!(
            "the app's files hold more than {} MB",
            MAX_APP_BYTES / (1024 * 1024)
        ));
    }
    if files.len() > MAX_APP_FILES {
        return invalid(format!("the app holds more than {MAX_APP_FILES} files"));
    }
    let text = |path: &str| -> Result<String> {
```

**In `crates/dots-core/src/apps/source.rs`, replace:**

```rust
    let mut tests = Vec::new();
    for (path, bytes) in &files {
        if let Some(rest) = path.strip_prefix("ui/") {
```

**with:**

```rust
    let mut tests = Vec::new();
    for (path, bytes) in files {
        if let Some(rest) = path.strip_prefix("ui/") {
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test --test apps_source_test --test apps_tests_test`
Expected: PASS: 4, 10 and 14 tests. `projects_passes_the_checks_a_proposal_gets_and_its_own_tests` runs Projects' 15 tests of `tests/tasks.test.js`; a failure names the test and its error.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1691 passed; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add .gitattributes crates/dots-core/apps/projects crates/dots-core/src/apps/bundled.rs crates/dots-core/src/apps/mod.rs crates/dots-core/src/apps/source.rs crates/dots-core/tests/apps_bundled_test.rs
git ls-files --eol crates/dots-core/apps/projects
git commit -m "feat(core): Projects comes with Dots: its data, tasks and their Activity"
```

Expected: every Projects file `i/lf w/lf attr/text eol=lf`.

### Task 2: Undo, moving tasks, and starting the dot a task is given to

Spec §8 "Undo", "Assigning to a dot" and §7: `undo` with the rules the person chose, `move_task` for dragging, the start of an assigned dot with its message, In progress and the "started" row, and `run_ended` posting the summary or error and moving the task In review. Projects now asks for `start_dots`.

**Files:**

- Create: `crates/dots-core/apps/projects/tests/{undo,assign}.test.js`
- Modify: `crates/dots-core/apps/projects/{app.json,logic.js}`, `crates/dots-core/src/apps/bundled.rs`

**Interfaces:**

- Consumes: Task 1's `change`, `record`, `valueOf`, `same` in `logic.js`; plan A's `ctx.dots.start(dot_id, message, tag)` (only for a tool that writes, with the switch on; it throws a rule's sentence) and the test API's `started()` and `{ kind: "dots" }` actor.
- Produces: the tools `move_task({ id, section_id?, before_id? })`, `undo({ activity_id }) -> { undone, by }`; `on_run_end: "run_ended"`; `permissions: ["start_dots"]`. The start's tag is the task id; its message names the task and its id, `get_task`, `comment`, `set_subtask`, `update_task` and `in_review`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/apps/projects/tests/assign.test.js`:**

```
// Assigning a task to a dot (app library spec section 8): with "Can start dots" on, Projects
// starts the dot on the task, the task goes In progress and Activity says the dot started; when
// the run ends, Activity gets its summary or error and the task goes In review unless it is Done.

const dots = [
  { id: "d1", name: "builder" },
  { id: "d2", name: "writer" },
];
const builder = { kind: "dot", id: "d1", name: "builder", run_id: "r1" };
const on = { dots, can_start: true };
const ended = (e, list = dots) => call("run_ended", e, { actor: { kind: "dots" }, dots: list });
const end = (task, run, status, more) => ({
  run_id: run,
  root_run_id: run,
  tag: task.id,
  dot_id: "d1",
  status,
  summary: null,
  error: null,
  ...more,
});

const rows = (task) => call("get_task", { id: task.id, activity: 500 }, { dots }).activity;

test("assigning a task to a dot starts it on the task, and the task goes In progress", () => {
  const t = call("create_task", { title: "Signup form", spec: "Email and password" });
  call("update_task", { id: t.id, assignee: "builder" }, on);
  const [start] = started();
  assert.equal(start.dot_id, "d1");
  assert.equal(start.tag, t.id);
  assert.ok(start.message.includes(`"Signup form" (id ${t.id})`), start.message);
  assert.ok(start.message.includes("get_task"), start.message);
  assert.ok(start.message.includes("in_review"), start.message);
  assert.equal(call("get_task", { id: t.id }).status, "in_progress");
  const run = rows(t).find((r) => r.kind === "run");
  assert.deepEqual(
    [run.field, run.run_id, run.actor.kind, run.actor.dot_id, run.actor.name],
    ["started", start.run_id, "dot", "d1", "builder"],
  );
});

test("a task created for a dot starts it too", () => {
  const t = call("create_task", { title: "Docs", assignee: "writer" }, on);
  assert.deepEqual(
    started().map((s) => [s.dot_id, s.tag]),
    [["d2", t.id]],
  );
});

test("with the switch off the task waits, and the dot finds it with list_tasks", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, { dots });
  assert.deepEqual(started(), []);
  assert.equal(call("get_task", { id: t.id }).status, "to_do");
  const mine = call("list_tasks", { assignee: "me" }, { actor: builder, dots });
  assert.deepEqual(
    mine.tasks.map((x) => x.id),
    [t.id],
  );
});

test("a dot that takes a task itself, or a Done task, starts nothing", () => {
  const t = call("create_task", { title: "Mine" });
  call("update_task", { id: t.id, assignee: "me" }, { actor: builder, ...on });
  const done = call("create_task", { title: "Old", status: "done" });
  call("update_task", { id: done.id, assignee: "builder" }, on);
  assert.deepEqual(started(), []);
});

test("assigning the same dot again while it works writes no second start", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  call("update_task", { id: t.id, assignee: "person" }, on);
  call("update_task", { id: t.id, assignee: "builder" }, on);
  assert.equal(started().length, 1);
  assert.equal(rows(t).filter((r) => r.field === "started").length, 1);
});

test("a start the rules refuse is written in Activity, and the task waits", () => {
  for (let i = 0; i < 20; i++) call("create_task", { title: `Task ${i}`, assignee: "builder" }, on);
  const t = call("create_task", { title: "One too many", assignee: "builder" }, on);
  assert.equal(started().length, 20);
  assert.equal(call("get_task", { id: t.id }).status, "to_do");
  const row = rows(t).find((r) => r.kind === "run");
  assert.equal(row.field, "not_started");
  assert.ok(row.text.includes("as many dots as the hour allows"), row.text);
});

test("a run's end posts its summary and moves the task In review", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  const [start] = started();
  ended(end(t, start.run_id, "succeeded", { summary: "Form built; tests pass." }));
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
  const [row, review] = rows(t).slice(-2);
  assert.deepEqual(
    [row.kind, row.field, row.after, row.text, row.run_id, row.actor.name],
    ["run", "ended", "succeeded", "Form built; tests pass.", start.run_id, "builder"],
  );
  assert.deepEqual(
    [review.field, review.before, review.after, review.actor.name],
    ["status", "in_progress", "in_review", "builder"],
  );
});

test("a failed run posts its error, and still leaves the task In review", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  const [start] = started();
  ended({ ...end(t, start.run_id, "failed", { error: "The model stopped" }), run_id: "r9" });
  const row = rows(t).find((r) => r.field === "ended");
  assert.deepEqual([row.after, row.text, row.run_id], ["failed", "The model stopped", "r9"]);
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
});

test("a run's end leaves a Done task Done, and a task given to someone else as it is", () => {
  const done = call("create_task", { title: "A", assignee: "builder" }, on);
  call("update_task", { id: done.id, status: "done" });
  const moved = call("create_task", { title: "B", assignee: "builder" }, on);
  call("update_task", { id: moved.id, assignee: "person" });
  const [first, second] = started();
  ended(end(done, first.run_id, "succeeded", { summary: "Done." }));
  ended(end(moved, second.run_id, "succeeded", { summary: "Done." }));
  assert.equal(call("get_task", { id: done.id }).status, "done");
  assert.equal(call("get_task", { id: moved.id }).status, "in_progress");
  assert.equal(rows(moved).filter((r) => r.field === "ended").length, 1);
});

test("a run whose task is gone is ignored; one whose dot was removed keeps its name", () => {
  const gone = { id: "gone" };
  assert.equal(ended(end(gone, "r1", "succeeded", { summary: "x" })), null);
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  const [start] = started();
  ended(end(t, start.run_id, "succeeded", { summary: "Done." }), []);
  assert.equal(rows(t).find((r) => r.field === "ended").actor.name, "builder");
});
```

**Create `crates/dots-core/apps/projects/tests/undo.test.js`:**

```
// Projects' undo (app library spec section 8, "Undo"): a change is put back only while its field
// still holds the change's value; the undo is itself a change; the person undoes anyone's
// change, a dot only a dot's. And moving a task between sections and within one.

const builder = { kind: "dot", id: "d1", name: "builder", run_id: "r1" };
const writer = { kind: "dot", id: "d2", name: "writer", run_id: "r2" };
const dots = [
  { id: "d1", name: "builder" },
  { id: "d2", name: "writer" },
];
const as = (actor) => ({ actor, dots });

const last = (task) => {
  const rows = call("get_task", { id: task.id, activity: 500 }).activity;
  return rows[rows.length - 1];
};

test("the person undoes a dot's change, and the undo is a change row of its own", () => {
  const t = call("create_task", { title: "Signup form" });
  call("update_task", { id: t.id, title: "Signup" }, as(builder));
  const change = last(t);
  const out = call("undo", { activity_id: change.id });
  assert.equal(call("get_task", { id: t.id }).title, "Signup form");
  const undo = last(t);
  assert.equal(out.by, undo.id);
  assert.deepEqual(
    [undo.field, undo.before, undo.after, undo.actor.kind],
    ["title", "Signup", "Signup form", "person"],
  );
  const rows = call("get_task", { id: t.id }).activity;
  assert.equal(rows.find((r) => r.id === change.id).undone_by, undo.id);
  assert.throws(() => call("undo", { activity_id: change.id }), "undone already");
});

test("undo refuses once the field changed since, naming the later change", () => {
  const t = call("create_task", { title: "Signup form" });
  call("update_task", { id: t.id, status: "in_progress" }, as(builder));
  const first = last(t);
  call(
    "update_task",
    { id: t.id, status: "in_review" },
    { actor: writer, dots, now: "2026-01-02T10:00:00+00:00" },
  );
  assert.throws(
    () => call("undo", { activity_id: first.id }),
    "The status was changed since by writer at 2026-01-02T10:00:00+00:00; undo that change first",
  );
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
  // Once the later change is undone, the field holds the first change's value again.
  call("undo", { activity_id: last(t).id });
  call("undo", { activity_id: first.id });
  assert.equal(call("get_task", { id: t.id }).status, "to_do");
});

test("the person undoes their own change; a dot undoes a dot's but not the person's", () => {
  const t = call("create_task", { title: "Rent" });
  call("update_task", { id: t.id, due_on: "2026-11-01" });
  const mine = last(t);
  assert.throws(
    () => call("undo", { activity_id: mine.id }, as(builder)),
    "Only the person undoes the person's changes",
  );
  call("update_task", { id: t.id, title: "Pay rent" }, as(writer));
  call("undo", { activity_id: last(t).id }, as(builder));
  assert.equal(call("get_task", { id: t.id }).title, "Rent");
  call("undo", { activity_id: mine.id });
  assert.equal(call("get_task", { id: t.id }).due_on, null);
});

test("comments, runs and a task's creation have no undo", () => {
  const t = call("create_task", { title: "Talk" });
  const created = last(t);
  call("comment", { task_id: t.id, text: "hello" });
  for (const row of [created, last(t)]) {
    assert.throws(() => call("undo", { activity_id: row.id }), "Only a change is undone");
  }
  assert.throws(() => call("undo", { activity_id: "nope" }), "There is no Activity row nope");
});

test("a dot's undo cannot bring back Done", () => {
  const t = call("create_task", { title: "Ship" });
  call("update_task", { id: t.id, status: "done" });
  call("update_task", { id: t.id, status: "in_progress" }, as(builder));
  assert.throws(
    () => call("undo", { activity_id: last(t).id }, as(builder)),
    "Only the person marks a task Done",
  );
  call("undo", { activity_id: last(t).id });
  assert.equal(call("get_task", { id: t.id }).status, "done");
});

test("undoing a subtask's addition removes it, and undoing that brings it back", () => {
  const t = call("create_task", { title: "Signup form" });
  const s = call("add_subtask", { task_id: t.id, title: "Tests" }, as(builder));
  call("undo", { activity_id: last(t).id });
  assert.equal(call("get_task", { id: t.id }).subtasks.length, 0);
  call("undo", { activity_id: last(t).id });
  assert.deepEqual(call("get_task", { id: t.id }).subtasks, [
    { id: s.id, title: "Tests", done: false },
  ]);
});

test("undoing an assignment puts the assignee back and starts no dot", () => {
  const t = call("create_task", { title: "Signup form", assignee: "person" });
  call("update_task", { id: t.id, assignee: "builder" }, { dots });
  call("update_task", { id: t.id, assignee: "person" }, { dots });
  call("undo", { activity_id: last(t).id }, { dots, can_start: true });
  assert.equal(call("get_task", { id: t.id }, { dots }).assignee.name, "builder");
  assert.deepEqual(started(), []);
});

test("a task dragged to another section moves there, before the task it was dropped on", () => {
  const p = call("create_project", { name: "Website" });
  const todo = call("create_section", { project_id: p.id, name: "To do" });
  const done = call("create_section", { project_id: p.id, name: "Done" });
  const a = call("create_task", { title: "A", section_id: todo.id });
  const b = call("create_task", { title: "B", section_id: done.id });
  const c = call("create_task", { title: "C", section_id: done.id });
  call("move_task", { id: a.id, section_id: done.id, before_id: c.id });
  const titles = () => call("list_tasks", { project_id: p.id }).tasks.map((t) => t.title);
  assert.deepEqual(titles(), ["B", "A", "C"]);
  assert.equal(last(a).field, "section");
  assert.equal(last(a).after.section, "Done");
  // Within a section the order changes and Activity does not.
  const rows = call("get_task", { id: c.id }).activity.length;
  call("move_task", { id: c.id, before_id: b.id });
  assert.deepEqual(titles(), ["C", "B", "A"]);
  assert.equal(call("get_task", { id: c.id }).activity.length, rows);
  assert.throws(() => call("move_task", { id: c.id, before_id: "nope" }), "is not in the list");
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test`
Expected: FAIL: `projects_holds_every_file_of_its_folder_and_only_lf_text` (the new test files are not in `PROJECTS_FILES` yet).

- [ ] **Step 3: Add undo, moving and assignment**

**In `crates/dots-core/apps/projects/app.json`, replace:**

```json
      "input": { "type": "object", "properties": {}, "additionalProperties": false }
    }
  ]
}
```

**with:**

```json
      "input": { "type": "object", "properties": {}, "additionalProperties": false }
    },
    {
      "name": "move_task",
      "description": "Move a task to another section (section_id, or null for none) and before another task of that list (before_id, or null for the end)",
      "input": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "section_id": { "type": ["string", "null"] },
          "before_id": { "type": ["string", "null"] }
        },
        "required": ["id"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "undo",
      "description": "Undo a change in a task's Activity, while its field still holds what the change set. A dot undoes only dots' changes.",
      "input": {
        "type": "object",
        "properties": { "activity_id": { "type": "string" } },
        "required": ["activity_id"],
        "additionalProperties": false
      },
      "writes": true
    }
  ],
  "permissions": ["start_dots"],
  "on_run_end": "run_ended"
}
```

**In `crates/dots-core/apps/projects/logic.js`, replace:**

```

function assigneeOut(task, dots) {
```

**with:**

```

/// What a dot is told when Projects starts it on a task (spec section 8, "Assigning to a dot").
function startMessage(task) {
  return (
    `You are assigned the Projects task "${task.title}" (id ${task.id}). Read it first with the ` +
    "Projects tool get_task: its spec and subtasks say what to do. As you work, post progress " +
    "with comment and check off subtasks with set_subtask. When you have finished, set its " +
    "status to in_review with update_task; only the person marks a task Done."
  );
}

/// Starts the dot a task was just given to, when Projects may start dots: the task goes In
/// progress and its Activity says the dot started, with the run. A dot that takes a task itself
/// is at work already, and a Done task waits. A start the rules refuse is written in Activity,
/// and the task waits for the dot to find it.
function startAssigned(ctx, taskId, actor) {
  const task = taskRow(ctx, taskId);
  if (task.assignee_kind !== "dot" || task.status === "done") return;
  if (isDot(ctx) && ctx.actor.id === task.assignee_dot_id) return;
  if (typeof ctx.dots.start !== "function") return;
  const dot = dotNamed(ctx, task.assignee_dot_id);
  const runner = { kind: "dot", dot_id: task.assignee_dot_id, name: dot?.name ?? "removed dot" };
  let run;
  try {
    run = ctx.dots.start(task.assignee_dot_id, startMessage(task), task.id);
  } catch (e) {
    record(ctx, taskId, runner, { kind: "run", field: "not_started", text: String(e.message ?? e) });
    return;
  }
  // The dot still works on the task from an earlier start: that run is the one.
  const known = one(
    ctx,
    "SELECT id FROM activity WHERE task_id = ? AND kind = 'run' AND field = 'started' AND run_id = ?",
    [taskId, run],
  );
  if (known !== null) return;
  change(ctx, taskId, "status", "in_progress", actor);
  record(ctx, taskId, runner, { kind: "run", field: "started", run_id: run });
}

const FIELD_NAMES = {
  title: "The title",
  spec: "The spec",
  status: "The status",
  assignee: "The assignee",
  due_on: "The due date",
  section: "The section",
};

function assigneeOut(task, dots) {
```

**In `crates/dots-core/apps/projects/logic.js`, replace:**

```
  record(ctx, id, actor, { kind: "change", field: "created", before: null, after: { title } });
  return taskOut(taskRow(ctx, id), ctx.dots.list());
```

**with:**

```
  record(ctx, id, actor, { kind: "change", field: "created", before: null, after: { title } });
  startAssigned(ctx, id, actor);
  return taskOut(taskRow(ctx, id), ctx.dots.list());
```

**In `crates/dots-core/apps/projects/logic.js`, replace:**

```
    if (row !== null) changes.push(row);
  }
```

**with:**

```
    if (row !== null) changes.push(row);
    if (row !== null && field === "assignee") startAssigned(ctx, task.id, actor);
  }
```

**In `crates/dots-core/apps/projects/logic.js`, replace:**

```
  return ctx.dots.list();
}
```

**with:**

```
  return ctx.dots.list();
}

export function move_task({ id, section_id, before_id }, ctx) {
  let task = taskRow(ctx, id);
  if (section_id !== undefined) {
    const place =
      section_id === null
        ? placeOf({ project_id: task.project_id, section_id: null })
        : { project_id: sectionRow(ctx, section_id).project_id, section_id };
    change(ctx, id, "section", place, who(ctx));
    task = taskRow(ctx, id);
  }
  // The order within a list is not a change of the task: Activity does not show it.
  const ids = ctx.db
    .query(
      "SELECT id FROM tasks WHERE project_id IS ? AND section_id IS ? AND id != ? " +
        "ORDER BY position, rowid",
      [task.project_id, task.section_id, id],
    )
    .map((r) => r.id);
  const at = before_id === undefined || before_id === null ? ids.length : ids.indexOf(before_id);
  if (at < 0) fail(`The task ${before_id} is not in the list the task moves to`);
  ids.splice(at, 0, id);
  ids.forEach((each, i) =>
    exec(ctx, "UPDATE tasks SET position = ? WHERE id = ? AND position != ?", [i + 1, each, i + 1]),
  );
  return taskOut(taskRow(ctx, id), ctx.dots.list());
}

export function undo({ activity_id }, ctx) {
  const row =
    one(ctx, "SELECT rowid AS seq, * FROM activity WHERE id = ?", [activity_id]) ??
    fail(`There is no Activity row ${activity_id}`);
  if (row.kind !== "change" || row.field === "created") {
    fail("Only a change is undone: not a comment, a run or a task's creation");
  }
  if (row.undone_by !== null) fail("This change was undone already");
  if (isDot(ctx) && row.actor_kind !== "dot") fail("Only the person undoes the person's changes");
  const task = taskRow(ctx, row.task_id);
  const before = parse(row.before_json);
  if (!same(valueOf(ctx, task, row.field), parse(row.after_json))) {
    const what = FIELD_NAMES[row.field] ?? "The subtask";
    const later = one(
      ctx,
      "SELECT actor_name, at FROM activity WHERE task_id = ? AND field = ? AND kind = 'change' " +
        "AND rowid > ? ORDER BY rowid DESC LIMIT 1",
      [row.task_id, row.field, row.seq],
    );
    fail(
      later === null
        ? `${what} no longer holds what this change set`
        : `${what} was changed since by ${later.actor_name} at ${later.at}; undo that change first`,
    );
  }
  if (row.field === "status" && before === "done" && isDot(ctx)) fail(ONLY_PERSON_DONE);
  // An undo puts the value back and starts no dot, even when it gives the task back to one.
  const by = change(ctx, row.task_id, row.field, before, who(ctx));
  exec(ctx, "UPDATE activity SET undone_by = ? WHERE id = ?", [by, row.id]);
  return { undone: row.id, by };
}

/// A run Projects started has ended (spec section 7): its summary or error goes to the task's
/// Activity, and the task goes In review unless it is Done or now someone else's.
export function run_ended(e, ctx) {
  const task = one(ctx, "SELECT * FROM tasks WHERE id = ?", [e.tag]);
  if (task === null) return null;
  const started = one(
    ctx,
    "SELECT actor_name FROM activity WHERE task_id = ? AND field = 'started' AND run_id = ?",
    [task.id, e.root_run_id],
  );
  const name = dotNamed(ctx, e.dot_id)?.name ?? started?.actor_name ?? "removed dot";
  const runner = { kind: "dot", dot_id: e.dot_id, name };
  const text = e.status === "succeeded" ? (e.summary ?? "") : (e.error ?? e.summary ?? "");
  record(ctx, task.id, runner, {
    kind: "run",
    field: "ended",
    after: e.status,
    text,
    run_id: e.run_id,
  });
  if (task.assignee_kind === "dot" && task.assignee_dot_id === e.dot_id && task.status !== "done") {
    change(ctx, task.id, "status", "in_review", runner);
  }
  return null;
}
```

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
    (
        "tests/tasks.test.js",
```

**with:**

```rust
    (
        "tests/assign.test.js",
        include_bytes!("../../apps/projects/tests/assign.test.js"),
    ),
    (
        "tests/tasks.test.js",
```

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
        include_bytes!("../../apps/projects/tests/tasks.test.js"),
    ),
```

**with:**

```rust
        include_bytes!("../../apps/projects/tests/tasks.test.js"),
    ),
    (
        "tests/undo.test.js",
        include_bytes!("../../apps/projects/tests/undo.test.js"),
    ),
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test`
Expected: PASS: 4 tests; Projects' own 33 tests pass inside `projects_passes_the_checks_a_proposal_gets_and_its_own_tests`.

- [ ] **Step 5: Run the checks**

Run: `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: no new warnings. (`dots-core` stays at 1691: Projects' tests run inside one core test.)

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/apps/projects crates/dots-core/src/apps/bundled.rs
git commit -m "feat(core): Projects undoes changes, moves tasks and starts the dot a task is given to"
```

### Task 3: Installed once, updated while bundled, installed again on request

Spec §8 "Bundled and changed copies" and §6 (Projects is on for every dot): `Apps::sync_bundled` at start, the update of a bundled, unchanged Projects with a release, removal that lasts, and `install_bundled` with `bundled_offers` for "Install Projects", on kept data that fits. A test runtime asks for it with `Config.bundled_apps`; one test runs the whole assignment with a local dot.

**Files:**

- Create: `crates/dots-core/tests/apps_projects_test.rs`
- Modify: `crates/dots-core/src/apps/{bundled,registry,kept,propose}.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/tests/apps_bundled_test.rs`

**Interfaces:**

- Consumes: Task 1's `bundled::{projects, release, PROJECTS}`; plan A's `Apps::{install, update_checked, load, get, roll_back, remove}`; plan B's `Apps::{kept_fit, install_kept, kept}` and `kept::KeptOffer`; `propose::check_migrations_kept`.
- Produces: `bundled::{BUNDLED_FILE = "bundled.json", BundledOffer { name, title, description, kept: Option<KeptOffer> }}`; `Apps::sync_bundled() -> Result<()>`, `Apps::bundled_offers() -> Result<Vec<BundledOffer>>`, `Apps::install_bundled(name, kept: Option<&str>) -> Result<LibraryItem>` (`NotFound` for a name that does not come with Dots, `Conflict` while it is installed); inside the crate `Apps::update_as(name, src, by_proposal, expect, origin: Option<AppOrigin>)` and `Apps::install_kept_as(src, kept_id, origin)`; an app installed with `AppOrigin::Bundled` has "Every dot" on; `Config.bundled_apps: bool` (false in `Config::new`, true in `from_settings`).

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_bundled_test.rs`, replace:**

```rust
//! The apps that come with Dots (app library spec section 8): Projects is built into Dots from
//! its folder, passes the checks a dot's proposal gets, and its own tests pass.

```

**with:**

```rust
//! The apps that come with Dots (app library spec section 8): Projects is built into Dots from
//! its folder, passes the checks a dot's proposal gets, and its own tests pass. Dots installs it
//! once, for every dot, updates it with a release while it is the bundled one, and leaves it out
//! once the person removed it; "Install Projects" brings it back, on its kept data when it fits.

mod common;

```

**In `crates/dots-core/tests/apps_bundled_test.rs`, replace:**

```rust

use dots_core::apps::bundled::{projects, projects_files, PROJECTS};
use dots_core::apps::data;
```

**with:**

```rust

use dots_core::apps::bundled::{projects, projects_files, release, PROJECTS};
use dots_core::apps::call::Actor;
use dots_core::apps::data;
```

**In `crates/dots-core/tests/apps_bundled_test.rs`, replace:**

```rust
use dots_core::apps::tests::run_tests;

```

**with:**

```rust
use dots_core::apps::tests::run_tests;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::{AppConfig, AppOrigin};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Store, Apps) {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    (dir, store, apps)
}

async fn config(apps: &Apps) -> AppConfig {
    apps.get(PROJECTS).await.unwrap().1
}

/// Projects as an earlier release shipped it: the same migrations, another README.
async fn install_earlier(apps: &Apps, files: BTreeMap<String, Vec<u8>>) {
    let src = read_files(&files).unwrap();
    let origin = AppOrigin::Bundled {
        release: "0.0.1".into(),
    };
    apps.install(src, origin).await.unwrap();
}

fn earlier_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = projects_files();
    files.insert("README.md".into(), b"# Projects, as it was\n".to_vec());
    files
}

async fn create_project(apps: &Apps, name: &str) {
    apps.call(
        PROJECTS,
        "create_project",
        json!({ "name": name }),
        Actor::Person,
    )
    .await
    .unwrap();
}

async fn project_names(apps: &Apps) -> Vec<String> {
    let listed = apps
        .call(PROJECTS, "list_projects", json!({}), Actor::Person)
        .await
        .unwrap();
    listed
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

```

**In `crates/dots-core/tests/apps_bundled_test.rs`, replace:**

```rust
    assert!(err.contains("notes.txt: an app holds"), "{err}");
}
```

**with:**

```rust
    assert!(err.contains("notes.txt: an app holds"), "{err}");
}

#[tokio::test]
async fn projects_is_installed_once_for_every_dot_and_removing_it_lasts() {
    let (_dir, _store, apps) = fixture().await;
    apps.sync_bundled().await.unwrap();
    let first = config(&apps).await;
    assert_eq!(first.origin, AppOrigin::Bundled { release: release() });
    assert!(first.every_dot, "Projects is on for every dot");
    assert!(!first.can_start_dots, "starting dots waits for the person");
    assert_eq!(first.version, 1);
    assert!(apps.bundled_offers().await.unwrap().is_empty());

    apps.sync_bundled().await.unwrap();
    assert_eq!(config(&apps).await.version, 1, "nothing new to install");

    apps.remove(PROJECTS, true, true).await.unwrap();
    apps.sync_bundled().await.unwrap();
    assert!(matches!(apps.get(PROJECTS).await, Err(Error::NotFound(_))));
    let offers = apps.bundled_offers().await.unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(
        (
            offers[0].name.as_str(),
            offers[0].title.as_str(),
            &offers[0].kept
        ),
        (PROJECTS, "Projects", &None)
    );
}

#[tokio::test]
async fn a_new_release_updates_the_bundled_projects_and_keeps_its_data() {
    let (_dir, _store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    create_project(&apps, "Home").await;
    apps.sync_bundled().await.unwrap();
    let now = config(&apps).await;
    assert_eq!(now.version, 2);
    assert_eq!(now.origin, AppOrigin::Bundled { release: release() });
    assert!(!now.changed);
    let (item, _) = apps.get(PROJECTS).await.unwrap();
    let active = apps.load(&item, &now).await.unwrap();
    assert_eq!(active.files(), projects().unwrap().files());
    assert_eq!(project_names(&apps).await, ["Home"]);
}

#[tokio::test]
async fn a_bundled_release_that_changes_a_migration_that_ran_is_refused() {
    let (_dir, _store, apps) = fixture().await;
    let mut files = earlier_files();
    let sql = String::from_utf8(files["migrations/0001_projects.sql"].clone()).unwrap();
    files.insert(
        "migrations/0001_projects.sql".into(),
        format!("{sql}\nCREATE TABLE old_notes (id TEXT);\n").into_bytes(),
    );
    install_earlier(&apps, files).await;
    let err = apps.sync_bundled().await.unwrap_err().to_string();
    assert!(
        err.contains("the bundled Projects cannot update: migrations/0001_projects.sql differs"),
        "{err}"
    );
    assert_eq!(config(&apps).await.version, 1);
}

#[tokio::test]
async fn projects_the_person_changed_or_rolled_back_is_left_as_it_is() {
    let (_dir, _store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    let mut theirs = earlier_files();
    theirs.insert("README.md".into(), b"# My Projects\n".to_vec());
    apps.update(PROJECTS, read_files(&theirs).unwrap(), true)
        .await
        .unwrap();
    apps.sync_bundled().await.unwrap();
    let now = config(&apps).await;
    assert!(now.changed);
    assert_eq!(now.version, 2, "Dots no longer updates it");

    // A bundled update the person rolled back stays rolled back.
    let (_dir, _store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    apps.sync_bundled().await.unwrap();
    apps.roll_back(PROJECTS).await.unwrap();
    apps.sync_bundled().await.unwrap();
    assert_eq!(config(&apps).await.version, 1);
}

#[tokio::test]
async fn an_app_a_dot_made_named_projects_is_left_alone() {
    let (_dir, store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    let (item, mut config) = apps.get(PROJECTS).await.unwrap();
    config.origin = AppOrigin::Proposed;
    store
        .update_library_item(&item.id, &dots_core::library::LibraryConfig::App(config))
        .await
        .unwrap();
    apps.sync_bundled().await.unwrap();
    let now = apps.get(PROJECTS).await.unwrap().1;
    assert_eq!((now.version, now.origin), (1, AppOrigin::Proposed));
}

#[tokio::test]
async fn install_projects_starts_on_the_kept_data_or_fresh() {
    let (_dir, _store, apps) = fixture().await;
    apps.sync_bundled().await.unwrap();
    create_project(&apps, "Home").await;
    apps.remove(PROJECTS, true, false).await.unwrap();

    let offer = apps.bundled_offers().await.unwrap().remove(0);
    let kept = offer.kept.expect("the kept data fits");
    assert!(kept.migrations.is_empty(), "no migration runs on it");
    apps.install_bundled(PROJECTS, Some(&kept.id))
        .await
        .unwrap();
    assert_eq!(project_names(&apps).await, ["Home"]);
    assert_eq!(
        config(&apps).await.origin,
        AppOrigin::Bundled { release: release() }
    );
    assert!(apps.kept().await.unwrap().is_empty());

    // Fresh, when the person unticks the kept data: it stays listed.
    apps.remove(PROJECTS, true, false).await.unwrap();
    apps.install_bundled(PROJECTS, None).await.unwrap();
    assert!(project_names(&apps).await.is_empty());
    assert_eq!(apps.kept().await.unwrap().len(), 1);
    let again = apps.install_bundled(PROJECTS, None).await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
    let other = apps.install_bundled("expenses", None).await;
    assert!(matches!(other, Err(Error::NotFound(_))), "{other:?}");
}
```

**Create `crates/dots-core/tests/apps_projects_test.rs`:**

```rust
//! Projects in a running Dots (app library spec sections 8 and 12): Dots installs it at start;
//! the person gives a task to a local dot with "Can start dots" on; the dot reads the task,
//! checks a subtask, comments and sets it In review; Activity shows its start, with the run, and
//! its summary.

mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{spec, wait_status};
use dots_core::apps::bundled::PROJECTS;
use dots_core::apps::call::Actor;
use dots_core::model::{EngineKind, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use llm::{call as llm_call, messages, text as llm_text, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;

async fn start(bundled: bool) -> (TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    cfg.bundled_apps = bundled;
    let rt = Runtime::start(cfg).await.unwrap();
    (dir, rt)
}

async fn person(rt: &Runtime, tool: &str, input: Value) -> Value {
    rt.apps()
        .call(PROJECTS, tool, input, Actor::Person)
        .await
        .unwrap()
}

#[tokio::test]
async fn dots_installs_projects_at_start_and_a_test_runtime_asks_for_it() {
    let (_dir, rt) = start(false).await;
    assert!(matches!(
        rt.apps().get(PROJECTS).await,
        Err(Error::NotFound(_))
    ));
    rt.shutdown();
    let (_dir, rt) = start(true).await;
    let (_, config) = rt.apps().get(PROJECTS).await.unwrap();
    assert!(config.every_dot);
    rt.shutdown();
}

#[tokio::test]
async fn a_dot_given_a_task_reads_it_reports_and_leaves_it_in_review() {
    let (dir, rt) = start(true).await;
    let task = person(
        &rt,
        "create_task",
        json!({ "title": "Signup form", "spec": "Email and password" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let sub = person(
        &rt,
        "add_subtask",
        json!({ "task_id": task, "title": "Form markup" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let llm = Llm::start(vec![
        llm_call("c1", "projects__get_task", json!({ "id": task })),
        llm_call(
            "c2",
            "projects__set_subtask",
            json!({ "id": sub, "done": true }),
        ),
        llm_call(
            "c3",
            "projects__comment",
            json!({ "task_id": task, "text": "Markup done." }),
        ),
        llm_call(
            "c4",
            "projects__update_task",
            json!({ "id": task, "status": "in_review" }),
        ),
        llm_text("Built the form."),
    ])
    .await;
    let mut s = spec("builder");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{}/v1", llm.endpoint()));
    s.model = "test-model".into();
    let builder = rt.create_dot(s, None).await.unwrap();
    rt.apps().set_can_start_dots(PROJECTS, true).await.unwrap();

    person(
        &rt,
        "update_task",
        json!({ "id": task, "assignee": "builder" }),
    )
    .await;
    let got = person(&rt, "get_task", json!({ "id": task })).await;
    assert_eq!(got["status"], "in_progress");
    let started = got["activity"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["field"] == "started")
        .cloned()
        .unwrap();
    let run_id = started["run_id"].as_str().unwrap().to_string();
    let run = rt.store().get_run(&run_id).await.unwrap();
    assert_eq!(
        (run.trigger, run.dot_id.as_str()),
        (TriggerKind::App, builder.id.as_str())
    );
    wait_status(rt.store(), &run_id, RunStatus::Succeeded, 30).await;
    let prompt = messages(&llm.requests()[0])
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        prompt.contains("You are assigned the Projects task \"Signup form\""),
        "{prompt}"
    );

    // The app hears the run's end once nothing in its topic waits.
    let mut rows = Vec::new();
    for _ in 0..200 {
        let got = person(&rt, "get_task", json!({ "id": task, "activity": 100 })).await;
        rows = got["activity"].as_array().unwrap().clone();
        if rows.iter().any(|r| r["field"] == "ended") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let seen: Vec<(String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r["kind"].as_str().unwrap().to_string(),
                r["field"].as_str().unwrap_or_default().to_string(),
                r["actor"]["name"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let sub_field = format!("subtask:{sub}");
    assert_eq!(
        seen.iter()
            .map(|(k, f, a)| (k.as_str(), f.as_str(), a.as_str()))
            .collect::<Vec<_>>(),
        [
            ("change", "created", "You"),
            ("change", sub_field.as_str(), "You"),
            ("change", "assignee", "You"),
            ("change", "status", "You"),
            ("run", "started", "builder"),
            ("change", sub_field.as_str(), "builder"),
            ("comment", "", "builder"),
            ("change", "status", "builder"),
            ("run", "ended", "builder"),
        ]
    );
    let ended = rows.last().unwrap();
    assert_eq!(ended["text"], "Built the form.");
    assert_eq!(ended["run_id"], run_id.as_str());
    let got = person(&rt, "get_task", json!({ "id": task })).await;
    assert_eq!(got["status"], "in_review");
    assert_eq!(got["subtasks"][0]["done"], true);
    rt.shutdown();
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test --test apps_projects_test`
Expected: FAIL to compile: no `sync_bundled` on `Apps`, no `bundled_apps` in `Config`.

- [ ] **Step 3: Install, update and install again**

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
//! `PROJECTS_FILES` too; a test checks that the two agree.

```

**with:**

```rust
//! `PROJECTS_FILES` too; a test checks that the two agree.
//!
//! Dots installs Projects the first time it starts with it, for every dot, and updates it with
//! each release while it is still the bundled one. Once the person accepts a dot's change to it,
//! it is the person's and Dots leaves it alone. Once the person removes it, Dots does not put it
//! back: `<data>/apps/bundled.json` names the bundled apps Dots has installed, and the Apps
//! screen offers to install one again.

```

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
use std::collections::BTreeMap;

use super::source::{self, AppSource};
use crate::Result;

```

**with:**

```rust
use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::kept::KeptOffer;
use super::propose::check_migrations_kept;
use super::registry::blocking;
use super::source::{self, AppSource};
use super::Apps;
use crate::library::{AppConfig, AppOrigin, LibraryItem};
use crate::{Error, Result};

/// The note of the bundled apps Dots has installed, below `<data>/apps/`.
pub const BUNDLED_FILE: &str = "bundled.json";

```

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
    source::read_files(&projects_files())
}
```

**with:**

```rust
    source::read_files(&projects_files())
}

/// A bundled app that is not installed, as the Apps screen offers it ("Install Projects"),
/// with the data a removal kept when the app's migrations fit it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundledOffer {
    pub name: String,
    pub title: String,
    pub description: String,
    pub kept: Option<KeptOffer>,
}

/// What `bundled.json` holds.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Installed {
    installed: Vec<String>,
}

/// The bundled apps Dots has installed. Blocks.
fn read_installed(root: &Path) -> Installed {
    std::fs::read(root.join(BUNDLED_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Notes that Dots has installed the bundled app `name`. Blocks.
fn note_installed(root: &Path, name: &str) -> Result<()> {
    let mut note = read_installed(root);
    if !note.installed.iter().any(|n| n == name) {
        note.installed.push(name.to_string());
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join(BUNDLED_FILE), serde_json::to_vec_pretty(&note)?)?;
    }
    Ok(())
}

impl Apps {
    /// Brings the bundled Projects in line with this Dots release, at start: installed the first
    /// time, updated while it is the bundled one and the person has not changed it, and left
    /// out once the person removed it. An app of the name that a dot made is left alone.
    pub async fn sync_bundled(&self) -> Result<()> {
        let src = blocking(projects).await??;
        let root = self.root().to_path_buf();
        let note = {
            let root = root.clone();
            blocking(move || read_installed(&root)).await?
        };
        match self.get(PROJECTS).await {
            Ok((item, config)) => self.update_bundled(&item, &config, src).await?,
            Err(Error::NotFound(_)) if note.installed.iter().any(|n| n == PROJECTS) => {
                return Ok(())
            }
            Err(Error::NotFound(_)) => {
                self.install(src, AppOrigin::Bundled { release: release() })
                    .await?;
            }
            Err(e) => return Err(e),
        }
        blocking(move || note_installed(&root, PROJECTS)).await?
    }

    /// Updates the installed bundled app `item` to `src`, this release's, while it is bundled,
    /// unchanged by the person and from another release. The new version keeps the migrations
    /// that ran, as a dot's change must; one that does not is refused and nothing changes.
    async fn update_bundled(
        &self,
        item: &LibraryItem,
        config: &AppConfig,
        src: AppSource,
    ) -> Result<()> {
        let AppOrigin::Bundled { release: had } = &config.origin else {
            return Ok(());
        };
        // A roll back keeps the release, so the version it left is not brought back.
        if config.changed || *had == release() {
            return Ok(());
        }
        let active = self.load(item, config).await?;
        if active.files() == src.files() {
            return Ok(());
        }
        check_migrations_kept(&active, &src, config.version).map_err(|e| {
            Error::Invalid(format!("the bundled {} cannot update: {e}", config.title))
        })?;
        self.update_as(
            &item.name,
            src,
            false,
            Some((&item.id, config.version)),
            Some(AppOrigin::Bundled { release: release() }),
        )
        .await?;
        Ok(())
    }

    /// The bundled apps that are not installed: what the Apps screen offers to install.
    pub async fn bundled_offers(&self) -> Result<Vec<BundledOffer>> {
        match self.get(PROJECTS).await {
            Ok(_) => return Ok(Vec::new()),
            Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        let src = blocking(projects).await??;
        Ok(vec![BundledOffer {
            name: src.manifest.name.clone(),
            title: src.manifest.title.clone(),
            description: src.manifest.description.clone(),
            kept: self.kept_fit(&src).await?,
        }])
    }

    /// Installs the bundled app `name` again ("Install Projects"): on the kept data `kept` when
    /// it names data that still fits it, else with fresh data.
    pub async fn install_bundled(&self, name: &str, kept: Option<&str>) -> Result<LibraryItem> {
        if name != PROJECTS {
            return Err(Error::NotFound(format!(
                "no app named {name} comes with Dots"
            )));
        }
        let src = blocking(projects).await??;
        let origin = AppOrigin::Bundled { release: release() };
        let item = match kept {
            Some(id) => self.install_kept_as(src, id, origin).await?,
            None => self.install(src, origin).await?,
        };
        let root = self.root().to_path_buf();
        blocking(move || note_installed(&root, PROJECTS)).await??;
        Ok(item)
    }
}
```

**In `crates/dots-core/src/apps/kept.rs`, replace:**

```rust
    pub async fn install_kept(&self, src: AppSource, kept_id: &str) -> Result<LibraryItem> {
        let lock = self.lock(kept_id);
```

**with:**

```rust
    pub async fn install_kept(&self, src: AppSource, kept_id: &str) -> Result<LibraryItem> {
        self.install_kept_as(src, kept_id, AppOrigin::Proposed)
            .await
    }

    /// `install_kept` for an app from `origin` (a bundled app, `apps::bundled`).
    pub(super) async fn install_kept_as(
        &self,
        src: AppSource,
        kept_id: &str,
        origin: AppOrigin,
    ) -> Result<LibraryItem> {
        let lock = self.lock(kept_id);
```

**In `crates/dots-core/src/apps/kept.rs`, replace:**

```rust
        }
        self.install_on(src, AppOrigin::Proposed, Some(kept_id))
            .await
    }
```

**with:**

```rust
        }
        self.install_on(src, origin, Some(kept_id)).await
    }
```

**In `crates/dots-core/src/apps/propose.rs`, replace:**

```rust
/// has run them).
fn check_migrations_kept(
    current: &AppSource,
```

**with:**

```rust
/// has run them).
pub(super) fn check_migrations_kept(
    current: &AppSource,
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            last_version: 1,
            every_dot: false,
            can_start_dots: false,
```

**with:**

```rust
            last_version: 1,
            // An app that comes with Dots is for every dot (spec section 6: Projects is).
            every_dot: matches!(origin, AppOrigin::Bundled { .. }),
            can_start_dots: false,
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
    ) -> Result<LibraryItem> {
        if src.manifest.name != name {
```

**with:**

```rust
    ) -> Result<LibraryItem> {
        self.update_as(name, src, by_proposal, expect, None).await
    }

    /// `update_checked` that also records where the new version came from, when `origin` names
    /// it (a Dots release's bundled app, `apps::bundled`).
    pub(super) async fn update_as(
        &self,
        name: &str,
        src: AppSource,
        by_proposal: bool,
        expect: Option<(&str, u32)>,
        origin: Option<AppOrigin>,
    ) -> Result<LibraryItem> {
        if src.manifest.name != name {
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
                || (by_proposal && matches!(old.origin, AppOrigin::Bundled { .. })),
            ..old.clone()
```

**with:**

```rust
                || (by_proposal && matches!(old.origin, AppOrigin::Bundled { .. })),
            origin: origin.unwrap_or_else(|| old.origin.clone()),
            ..old.clone()
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub watcher_sources: crate::watchers::Sources,
}
```

**with:**

```rust
    pub watcher_sources: crate::watchers::Sources,
    /// Install and update the apps that come with Dots at start (app library spec section 8).
    /// `Config::new` leaves them out, so a test starts with no app unless it asks for them;
    /// the app's configuration has them.
    pub bundled_apps: bool,
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            watcher_sources: crate::watchers::Sources::default(),
        }
```

**with:**

```rust
            watcher_sources: crate::watchers::Sources::default(),
            bundled_apps: false,
        }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        cfg.secrets = None;
        cfg
```

**with:**

```rust
        cfg.secrets = None;
        cfg.bundled_apps = true;
        cfg
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        .with_clock(cfg.clock.clone());
        let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
```

**with:**

```rust
        .with_clock(cfg.clock.clone());
        // Projects comes with Dots (spec section 8). One that cannot be installed or updated
        // leaves Dots running without the change; the next start tries again.
        if cfg.bundled_apps {
            if let Err(e) = apps.sync_bundled().await {
                tracing::error!(
                    "the apps that come with Dots could not be brought up to date: {e}"
                );
            }
        }
        let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test --test apps_projects_test --test apps_registry_test --test apps_kept_test`
Expected: PASS: 10, 2, 21 and 9 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo test -j 2 -p dots-app`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 1699 passed, `dots-app` 174; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/apps/bundled.rs crates/dots-core/src/apps/kept.rs crates/dots-core/src/apps/propose.rs crates/dots-core/src/apps/registry.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/apps_bundled_test.rs crates/dots-core/tests/apps_projects_test.rs
git commit -m "feat(core): Dots installs Projects once, updates it while bundled, and installs it again on request"
```

### Task 4: Commands for the start prompt, a run's link and Install Projects

Spec §8's one-time prompt and "● builder started" with the run linked, and "Install Projects": the row's `start_asked`, `Apps::answer_start_prompt`, `Apps::run_link` (only a run of a topic the app started), and the four Tauri commands with their contract samples and UI schemas.

**Files:**

- Modify: `crates/dots-core/src/{library.rs,apps/registry.rs,apps/wake.rs}`, `apps/dots-app/src/{commands,contract,lib}.rs`, `ui/src/api/{types,commands}.ts`, `ui/src/api/contract.test.ts`, `ui/src/api/__fixtures__/contract.json` (written by the command)
- Test: `crates/dots-core/tests/apps_{bundled,projects,registry}_test.rs`

**Interfaces:**

- Consumes: Task 3's `BundledOffer`, `bundled_offers`, `install_bundled`; plan A's `store.app_topic(root_run_id)`, `Apps::set_can_start_dots`.
- Produces: `AppConfig.start_asked: bool` (serde default false); `Apps::answer_start_prompt(name, on) -> Result<LibraryItem>` ("Switch on" sets the switch; either answer sets `start_asked`); `set_can_start_dots` also sets `start_asked`; `wake::RunLink { dot_id, root_run_id }`, `Apps::run_link(name, run_id) -> Result<RunLink>` (`NotFound`: "the app <name> started no run <id>"); commands `apps_bundled_offers`, `app_install_bundled(name, keptId)`, `app_answer_start_prompt(name, on)`, `app_run_link(name, runId)`; UI `T.BundledOffer`, `T.RunLink`, `commands.{appsBundledOffers, appInstallBundled, appAnswerStartPrompt, appRunLink}`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_bundled_test.rs`, replace:**

```rust
    assert!(matches!(other, Err(Error::NotFound(_))), "{other:?}");
}
```

**with:**

```rust
    assert!(matches!(other, Err(Error::NotFound(_))), "{other:?}");
}

#[tokio::test]
async fn the_prompt_to_start_dots_shows_until_the_person_answers_or_sets_the_switch() {
    let (_dir, _store, apps) = fixture().await;
    apps.sync_bundled().await.unwrap();
    assert!(!config(&apps).await.start_asked);
    apps.answer_start_prompt(PROJECTS, false).await.unwrap();
    let now = config(&apps).await;
    assert!(
        now.start_asked && !now.can_start_dots,
        "Not now leaves it off"
    );

    // A fresh install asks again; "Switch on" turns it on.
    apps.remove(PROJECTS, true, true).await.unwrap();
    apps.install_bundled(PROJECTS, None).await.unwrap();
    assert!(!config(&apps).await.start_asked);
    apps.answer_start_prompt(PROJECTS, true).await.unwrap();
    let now = config(&apps).await;
    assert!(now.start_asked && now.can_start_dots);

    // The switch on the Apps screen is an answer too.
    apps.remove(PROJECTS, true, true).await.unwrap();
    apps.install_bundled(PROJECTS, None).await.unwrap();
    apps.set_can_start_dots(PROJECTS, false).await.unwrap();
    assert!(config(&apps).await.start_asked);
}
```

**In `crates/dots-core/tests/apps_projects_test.rs`, replace:**

```rust
    assert_eq!(got["subtasks"][0]["done"], true);
    rt.shutdown();
```

**with:**

```rust
    assert_eq!(got["subtasks"][0]["done"], true);

    // The screen opens the run it started, and no other.
    let link = rt.apps().run_link(PROJECTS, &run_id).await.unwrap();
    assert_eq!(
        (link.dot_id.as_str(), link.root_run_id.as_str()),
        (builder.id.as_str(), run_id.as_str())
    );
    let other = rt.run_now(&builder.id).await.unwrap();
    for (app, run) in [
        (PROJECTS, other.id.as_str()),
        (PROJECTS, "nope"),
        ("expenses", run_id.as_str()),
    ] {
        let refused = rt.apps().run_link(app, run).await;
        assert!(
            matches!(refused, Err(Error::NotFound(_))),
            "{app} {run}: {refused:?}"
        );
    }
    rt.shutdown();
```

**In `crates/dots-core/tests/apps_registry_test.rs`, replace:**

```rust
            changed: false,
        }
```

**with:**

```rust
            changed: false,
            start_asked: false,
        }
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  KeptData: T.KeptData.array(),
  AppFile: T.AppFile.array(),
```

**with:**

```ts
  KeptData: T.KeptData.array(),
  BundledOffer: T.BundledOffer.array(),
  RunLink: T.RunLink,
  AppFile: T.AppFile.array(),
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test --test apps_projects_test --test apps_registry_test`
Expected: FAIL to compile: no `answer_start_prompt` or `run_link` on `Apps`, no `start_asked` in `AppConfig`.

- [ ] **Step 3: Add the prompt's state, the run link and the commands**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

use dots_core::apps::kept::KeptData;
```

**with:**

```rust

use dots_core::apps::bundled::BundledOffer;
use dots_core::apps::kept::KeptData;
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use dots_core::apps::screen::{AppFile, AppRow};
use dots_core::apps::AppCall;
```

**with:**

```rust
use dots_core::apps::screen::{AppFile, AppRow};
use dots_core::apps::wake::RunLink;
use dots_core::apps::AppCall;
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// The person's answer on the card: add (or update), ask for changes, or discard.
```

**with:**

```rust

/// The apps that come with Dots and are not installed, with the kept data each may start on
/// (app library spec section 8, "Install Projects").
#[tauri::command]
pub async fn apps_bundled_offers(state: State<'_, AppState>) -> Res<Vec<BundledOffer>> {
    Ok(state.rt()?.apps().bundled_offers().await?)
}

/// Installs an app that comes with Dots again: on the kept data `kept_id`, or fresh.
#[tauri::command]
pub async fn app_install_bundled(
    state: State<'_, AppState>,
    name: String,
    kept_id: Option<String>,
) -> Res<LibraryItem> {
    Ok(state
        .rt()?
        .apps()
        .install_bundled(&name, kept_id.as_deref())
        .await?)
}

/// The person's answer to the one-time prompt to let the app start dots: "Switch on" or
/// "Not now" (app library spec section 8).
#[tauri::command]
pub async fn app_answer_start_prompt(
    state: State<'_, AppState>,
    name: String,
    on: bool,
) -> Res<LibraryItem> {
    Ok(state.rt()?.apps().answer_start_prompt(&name, on).await?)
}

/// Where the app's screen opens a run it started (`dots.openRun`); refused for any other run.
#[tauri::command]
pub async fn app_run_link(
    state: State<'_, AppState>,
    name: String,
    run_id: String,
) -> Res<RunLink> {
    Ok(state.rt()?.apps().run_link(&name, &run_id).await?)
}

/// The person's answer on the card: add (or update), ask for changes, or discard.
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        changed: false,
    })
```

**with:**

```rust
        changed: false,
        start_asked: true,
    })
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    }]);
    samples["AppFile"] = to([
```

**with:**

```rust
    }]);
    samples["BundledOffer"] = to([
        dots_core::apps::bundled::BundledOffer {
            name: "projects".into(),
            title: "Projects".into(),
            description: "Projects, tasks and to-dos, shared by you and your dots".into(),
            kept: Some(dots_core::apps::kept::KeptOffer {
                id: "01J0APP0".into(),
                title: "Projects".into(),
                removed_at: TS.into(),
                bytes: 24_576,
                migrations: vec!["0002_labels.sql".into()],
            }),
        },
        dots_core::apps::bundled::BundledOffer {
            name: "projects".into(),
            title: "Projects".into(),
            description: "Projects, tasks and to-dos, shared by you and your dots".into(),
            kept: None,
        },
    ]);
    samples["RunLink"] = to(dots_core::apps::wake::RunLink {
        dot_id: "01J0DOT".into(),
        root_run_id: "01J0RUN".into(),
    });
    samples["AppFile"] = to([
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    app_kept_delete,
    watchers_list,
```

**with:**

```rust
    app_kept_delete,
    apps_bundled_offers,
    app_install_bundled,
    app_answer_start_prompt,
    app_run_link,
    watchers_list,
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
            changed: false,
        };
```

**with:**

```rust
            changed: false,
            start_asked: false,
        };
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
    /// Switches "Can start dots" (spec section 7): on only for an app whose code asks for it.
    pub async fn set_can_start_dots(&self, name: &str, on: bool) -> Result<LibraryItem> {
```

**with:**

```rust
    /// Switches "Can start dots" (spec section 7): on only for an app whose code asks for it.
    /// The person has then made their choice: the one-time prompt no longer shows.
    pub async fn set_can_start_dots(&self, name: &str, on: bool) -> Result<LibraryItem> {
```

**In `crates/dots-core/src/apps/registry.rs`, replace:**

```rust
                can_start_dots: on,
                ..config
```

**with:**

```rust
                can_start_dots: on,
                start_asked: true,
                ..config
            },
        )
        .await
    }

    /// The person's answer to the one-time prompt to let the app `name` start dots (spec
    /// section 8): "Switch on" (`on`) turns the switch on, "Not now" leaves it off; either way
    /// the prompt no longer shows.
    pub async fn answer_start_prompt(&self, name: &str, on: bool) -> Result<LibraryItem> {
        if on {
            return self.set_can_start_dots(name, true).await;
        }
        let (_held, item, config) = self.get_locked(name).await?;
        self.store_config(
            &item,
            AppConfig {
                start_asked: true,
                ..config
```

**In `crates/dots-core/src/apps/wake.rs`, replace:**

```rust

impl Apps {
    /// Tells the app that started `run`'s topic that the run ended, when nothing in the topic
```

**with:**

```rust

/// Where the person finds a run an app started: the dot's conversation at the topic's root.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunLink {
    pub dot_id: String,
    pub root_run_id: String,
}

impl Apps {
    /// The conversation of `run_id`, a run of a topic the app `name` started, for the app's
    /// screen to open (`dots.openRun`). Any other run is `NotFound`: an app's screen opens only
    /// the runs it started.
    pub async fn run_link(&self, name: &str, run_id: &str) -> Result<RunLink> {
        let (item, _) = self.get(name).await?;
        let not_found = || Error::NotFound(format!("the app {name} started no run {run_id}"));
        let run = match self.store.get_run(run_id).await {
            Ok(run) => run,
            Err(Error::NotFound(_)) => return Err(not_found()),
            Err(e) => return Err(e),
        };
        match self.store.app_topic(&run.root_run_id).await? {
            Some(topic) if topic.app_id == item.id => Ok(RunLink {
                dot_id: run.dot_id,
                root_run_id: run.root_run_id,
            }),
            _ => Err(not_found()),
        }
    }

    /// Tells the app that started `run`'s topic that the run ended, when nothing in the topic
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
    pub changed: bool,
}
```

**with:**

```rust
    pub changed: bool,
    /// The person has answered the one-time prompt to let a bundled app start dots, or set the
    /// switch themselves (spec section 8): the prompt no longer shows.
    #[serde(default)]
    pub start_asked: bool,
}
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  appKeptDelete: (id: string) => call(nothing, "app_kept_delete", { id }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already
```

**with:**

```ts
  appKeptDelete: (id: string) => call(nothing, "app_kept_delete", { id }),
  /// The apps that come with Dots and are not installed ("Install Projects").
  appsBundledOffers: () => call(T.BundledOffer.array(), "apps_bundled_offers"),
  /// Installs an app that comes with Dots again: on the kept data `keptId`, or fresh (null).
  appInstallBundled: (name: string, keptId: string | null) =>
    call(T.LibraryItem, "app_install_bundled", { name, keptId }),
  /// The one-time prompt to let the app start dots: "Switch on" (true) or "Not now" (false).
  appAnswerStartPrompt: (name: string, on: boolean) =>
    call(T.LibraryItem, "app_answer_start_prompt", { name, on }),
  /// Where a run the app started is; rejects for a run it did not start.
  appRunLink: (name: string, runId: string) => call(T.RunLink, "app_run_link", { name, runId }),
  /// The person's answer on the card. Rejects with kind `conflict` when it was decided already
```

**In `ui/src/api/types.ts`, replace:**

```ts
  changed: z.boolean(),
});
```

**with:**

```ts
  changed: z.boolean(),
  // The person answered the one-time prompt to let a bundled app start dots, or set the switch.
  start_asked: z.boolean(),
});
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type KeptData = z.infer<typeof KeptData>;
export type AppFile = z.infer<typeof AppFile>;
```

**with:**

```ts
export type KeptData = z.infer<typeof KeptData>;
// An app that comes with Dots and is not installed: "Install Projects", with the kept data it
// may start on (app library spec section 8).
export const BundledOffer = z.strictObject({
  name: z.string(),
  title: z.string(),
  description: z.string(),
  kept: nullable(KeptOffer),
});
export type BundledOffer = z.infer<typeof BundledOffer>;
// Where a run an app started is: the dot's conversation at the topic's root.
export const RunLink = z.strictObject({
  dot_id: z.string(),
  root_run_id: z.string(),
});
export type RunLink = z.infer<typeof RunLink>;
export type AppFile = z.infer<typeof AppFile>;
```

Then write the fixture: `UPDATE_CONTRACT=1 cargo test -j 2 -p dots-app contract`.

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -j 2 -p dots-core --test apps_bundled_test --test apps_projects_test --test apps_registry_test --test apps_wake_test`, then `cargo test -j 2 -p dots-app`, then `DEBUG_PRINT_LIMIT=0 bun x vitest run src/api src/screens/apps` in `ui`.
Expected: PASS: 11, 2, 21 and 7 tests; `dots-app` 174; the UI 8 files, 138 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 1700; the UI 829 tests in 82 files; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/contract.rs apps/dots-app/src/lib.rs crates/dots-core/src/apps/registry.rs crates/dots-core/src/apps/wake.rs crates/dots-core/src/library.rs crates/dots-core/tests/apps_bundled_test.rs crates/dots-core/tests/apps_projects_test.rs crates/dots-core/tests/apps_registry_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts
git commit -m "feat: commands for the start prompt, a run's link and Install Projects"
```

### Task 5: Projects' list screen

Spec §8 "Screen" and mockup A: the left column (My tasks, Assigned to dots, the projects, + New project), the list grouped by section with Task, Assignee, Due and Status, tasks and sections added inline, sections renamed, tasks dragged between sections, all through `dots.call`, refreshed by `dots.onChange`. The UI tests import the screen's modules (`allowJs`).

**Files:**

- Create: `crates/dots-core/apps/projects/ui/{main.js,projects.js,projects.css}`, `ui/src/apps/{projects-fake.ts,projects-screen.test.ts}`
- Modify: `crates/dots-core/apps/projects/ui/index.html`, `crates/dots-core/src/apps/bundled.rs`, `ui/tsconfig.json`

**Interfaces:**

- Consumes: Tasks 1 and 2's tools; plan B's page client (`dots.call`, `dots.onChange`, the `--dots-<token>` variables).
- Produces: `projects.js`: `mount(root, dots) -> { ready: Promise, reload }`, `h(tag, attrs, ...children)`, `shortDate(day)`, `assigneePill(assignee)`, `STATUS_LABELS`. The screen's accessible names the tests use: navigation "Projects"; buttons "My tasks", "Assigned to dots", "● <project>", the section's name (rename), "Mark <title> done" / "Mark <title> not done"; text boxes "New project", "New section", "Add a task to <section, To-dos or project>", "Rename <section>"; regions named by section or project; rows named by task title. `projects-fake.ts`: `fakeDots(handlers)`, `sample(more)`, `show(fake)`, `task(over)` and the sample's `website`, `copy`, `signup`, `hosting`, `landlord`.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/apps/projects-fake.ts`:**

```ts
// A fake `dots` for Projects' screen in jsdom, and the sample's data: what the screen's tests
// call its tools against.
import userEvent from "@testing-library/user-event";
import { afterEach } from "vitest";

import { mount } from "../../../crates/dots-core/apps/projects/ui/projects.js";

export type Input = Record<string, unknown>;
export type Handler = (input: Input) => unknown;

/// A `dots` client whose calls `handlers` answer, with every call kept.
export function fakeDots(handlers: Record<string, Handler>) {
  const changes = new Set<() => void>();
  const calls: [string, Input][] = [];
  const opened: string[] = [];
  const dots = {
    call: async (tool: string, input: Input) => {
      calls.push([tool, input]);
      const handler = handlers[tool];
      if (!handler) throw new Error(`the fake has no ${tool}`);
      return handler(input);
    },
    onChange: (cb: () => void) => {
      changes.add(cb);
      return () => changes.delete(cb);
    },
    onTheme: () => () => {},
    openRun: async (runId: string) => {
      opened.push(runId);
    },
  };
  return {
    dots,
    opened,
    callsTo: (tool: string) => calls.filter(([t]) => t === tool).map(([, input]) => input),
    change: () => changes.forEach((cb) => cb()),
  };
}

export const task = (over: Input) => ({
  id: "t1",
  title: "Write pricing page copy",
  project_id: "p1",
  section_id: "s1",
  status: "to_do",
  assignee: { kind: "person" },
  due_on: "2026-10-28",
  position: 1,
  created_at: "2026-10-20T09:00:00+00:00",
  updated_at: "2026-10-20T09:00:00+00:00",
  completed_at: null,
  subtasks: { done: 0, total: 0 },
  ...over,
});

export const website = {
  id: "p1",
  name: "Website relaunch",
  position: 1,
  sections: [
    { id: "s1", name: "To do", position: 1 },
    { id: "s2", name: "Done", position: 2 },
  ],
};
export const copy = task({});
export const signup = task({
  id: "t2",
  title: "Build signup form",
  assignee: { kind: "dot", id: "d1", name: "builder" },
  due_on: "2026-10-25",
  status: "in_progress",
  position: 2,
});
export const hosting = task({ id: "t3", title: "Pick hosting", section_id: "s2", status: "done" });
export const landlord = task({
  id: "t4",
  title: "Call the landlord",
  project_id: null,
  section_id: null,
  assignee: null,
  due_on: null,
});

/// Projects with the sample's data, as the handlers' defaults.
export function sample(more: Record<string, Handler> = {}) {
  const projects = [website];
  return fakeDots({
    list_projects: () => projects,
    list_dots: () => [{ id: "d1", name: "builder" }],
    list_tasks: (input) => {
      if (input.project_id === "p1") return { tasks: [copy, signup, hosting], more: false };
      if (input.view === "my_tasks") return { tasks: [landlord, copy, hosting], more: false };
      if (input.view === "assigned_to_dots") return { tasks: [signup], more: false };
      return { tasks: [], more: false };
    },
    create_project: (input) => {
      const p = { id: "p2", name: String(input.name), position: 2, sections: [] };
      projects.push(p);
      return p;
    },
    create_task: () => task({ id: "t9" }),
    create_section: () => ({ id: "s9", name: "Launch", position: 3 }),
    rename_section: (input) => input,
    move_task: () => copy,
    update_task: () => ({ task: copy, changes: [] }),
    ...more,
  });
}

let root: HTMLElement | null = null;

export async function show(fake: ReturnType<typeof fakeDots>) {
  root = document.createElement("div");
  document.body.append(root);
  await mount(root, fake.dots).ready;
  return userEvent.setup();
}

afterEach(() => {
  root?.remove();
  root = null;
});
```

**Create `ui/src/apps/projects-screen.test.ts`:**

```ts
// Projects' screen (app library spec section 8, mockup A), run in jsdom against a fake `dots`:
// the left column, the list grouped by section, inline adds, sections and dragging.
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { copy, landlord, sample, show, signup, task } from "./projects-fake";

const region = (name: string) => screen.getByRole("region", { name });
const row = (name: string) => screen.getByRole("row", { name });

describe("Projects' screen", () => {
  it("opens on My tasks, with the projects and New project in the left column", async () => {
    const fake = sample();
    await show(fake);
    const nav = screen.getByRole("navigation", { name: "Projects" });
    expect(within(nav).getByRole("button", { name: "My tasks" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(within(nav).getByRole("button", { name: "Assigned to dots" })).toBeVisible();
    expect(within(nav).getByRole("button", { name: "● Website relaunch" })).toBeVisible();
    expect(within(nav).getByRole("textbox", { name: "New project" })).toBeVisible();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("My tasks");
    // The to-dos first, then each project's tasks.
    expect(within(region("To-dos")).getByRole("row", { name: "Call the landlord" })).toBeVisible();
    expect(within(region("Website relaunch")).getAllByRole("row")).toHaveLength(2);
    expect(fake.callsTo("list_tasks")).toEqual([{ view: "my_tasks", limit: 1000 }]);
  });

  it("shows a project's tasks by section with Task, Assignee, Due and Status", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Website relaunch");
    const head = screen.getAllByRole("row")[0]!;
    expect(head).toHaveTextContent("TaskAssigneeDueStatus");
    const todo = region("To do");
    expect(within(todo).getAllByRole("row").map((r) => r.getAttribute("aria-label"))).toEqual([
      "Write pricing page copy",
      "Build signup form",
    ]);
    expect(row("Build signup form")).toHaveTextContent("● builder");
    expect(row("Build signup form")).toHaveTextContent("Oct 25");
    expect(row("Build signup form")).toHaveTextContent("In progress");
    expect(row("Write pricing page copy")).toHaveTextContent("You");
    expect(within(region("Done")).getByRole("button", { name: "Mark Pick hosting not done" }))
      .toHaveTextContent("✓");
  });

  it("shows the tasks assigned to dots, and a removed dot as such", async () => {
    const gone = task({ id: "t5", title: "Old", assignee: { kind: "dot", id: "dx", name: null } });
    const fake = sample({ list_tasks: () => ({ tasks: [signup, gone], more: false }) });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "Assigned to dots" }));
    expect(fake.callsTo("list_tasks").at(-1)).toEqual({ view: "assigned_to_dots", limit: 1000 });
    expect(row("Old")).toHaveTextContent("● removed dot");
  });

  it("adds a task inline to its section and keeps the cursor there", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    await user.type(screen.getByRole("textbox", { name: "Add a task to To do" }), "Tests{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("create_task")).toEqual([
        { title: "Tests", project_id: "p1", section_id: "s1" },
      ]),
    );
    await waitFor(() =>
      expect(document.activeElement).toHaveAccessibleName("Add a task to To do"),
    );
    // A to-do from My tasks has no project.
    await user.click(screen.getByRole("button", { name: "My tasks" }));
    await user.type(screen.getByRole("textbox", { name: "Add a task to To-dos" }), "Rent{Enter}");
    await waitFor(() => expect(fake.callsTo("create_task").at(-1)).toEqual({ title: "Rent" }));
  });

  it("adds a section, renames one on Enter and leaves it as it was on Escape", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    await user.type(screen.getByRole("textbox", { name: "New section" }), "Launch{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("create_section")).toEqual([{ project_id: "p1", name: "Launch" }]),
    );
    await user.click(screen.getByRole("button", { name: "To do" }));
    const rename = screen.getByRole("textbox", { name: "Rename To do" });
    await user.clear(rename);
    await user.type(rename, "Backlog{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("rename_section")).toEqual([{ id: "s1", name: "Backlog" }]),
    );
    await user.click(screen.getByRole("button", { name: "Done" }));
    await user.type(screen.getByRole("textbox", { name: "Rename Done" }), "x{Escape}");
    expect(fake.callsTo("rename_section")).toHaveLength(1);
  });

  it("creates a project and opens it", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.type(screen.getByRole("textbox", { name: "New project" }), "Home{Enter}");
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Home"),
    );
    expect(fake.callsTo("create_project")).toEqual([{ name: "Home" }]);
    expect(screen.getByRole("button", { name: "● Home" })).toHaveAttribute("aria-current", "page");
  });

  it("moves a task dropped on another section's task before it, or to a section's end", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    fireEvent.dragStart(row("Write pricing page copy"));
    fireEvent.drop(row("Pick hosting"));
    await waitFor(() =>
      expect(fake.callsTo("move_task")).toEqual([{ id: "t1", section_id: "s2", before_id: "t3" }]),
    );
    fireEvent.dragStart(row("Pick hosting"));
    fireEvent.drop(region("To do"));
    await waitFor(() =>
      expect(fake.callsTo("move_task").at(-1)).toEqual({
        id: "t3",
        section_id: "s1",
        before_id: null,
      }),
    );
  });

  it("marks a task Done with its circle, and shows a refusal above the list", async () => {
    let refuse = false;
    const fake = sample({
      update_task: () => {
        if (refuse) throw new Error("There is no task t1");
        return { task: copy, changes: [] };
      },
    });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "Mark Write pricing page copy done" }));
    await waitFor(() =>
      expect(fake.callsTo("update_task")).toEqual([{ id: "t1", status: "done" }]),
    );
    refuse = true;
    await user.click(screen.getByRole("button", { name: "Mark Pick hosting not done" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("There is no task t1");
  });

  it("follows the app's data when it changes", async () => {
    const tasks = [landlord];
    const fake = sample({ list_tasks: () => ({ tasks, more: false }) });
    await show(fake);
    tasks.push(task({ id: "t8", title: "Pack boxes", project_id: null, section_id: null }));
    fake.change();
    expect(await screen.findByRole("row", { name: "Pack boxes" })).toBeVisible();
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/apps` in `ui`.
Expected: FAIL: `crates/dots-core/apps/projects/ui/projects.js` cannot be resolved.

- [ ] **Step 3: Write the screen**

**In `crates/dots-core/apps/projects/ui/index.html`, replace:**

```
<title>Projects</title>
<p>Projects</p>
```

**with:**

```
<title>Projects</title>
<link rel="stylesheet" href="projects.css">
<div id="app"></div>
<script type="module" src="main.js"></script>
```

**Create `crates/dots-core/apps/projects/ui/main.js`:**

```
// Projects' page: the screen in `#app`, talking to Dots through the client Dots puts in the page.
import { mount } from "./projects.js";

mount(document.getElementById("app"), window.dots);
```

**Create `crates/dots-core/apps/projects/ui/projects.css`:**

```
/* Projects' screen, in Dots' colours: Dots sets the --dots-<token> variables on the root. */
:root {
  color-scheme: light dark;
  font: 13px/1.4 system-ui, "Segoe UI", sans-serif;
  color: var(--dots-foreground, #11201f);
  background: var(--dots-background, #f7fbfb);
}
* { box-sizing: border-box; }
body { margin: 0; }
button, input, select, textarea { font: inherit; color: inherit; }
button { cursor: pointer; background: none; border: 0; padding: 0; text-align: left; }
button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible {
  outline: 2px solid var(--dots-ring, #0e9780);
  outline-offset: 1px;
}
input, select, textarea {
  background: var(--dots-card, #fff);
  border: 1px solid var(--dots-input, #d3e3e1);
  border-radius: 6px;
  padding: 4px 6px;
}

.projects { display: flex; height: 100vh; overflow: hidden; }

.side {
  width: 200px;
  flex: none;
  padding: 12px 8px;
  overflow-y: auto;
  background: var(--dots-sidebar, #eef5f4);
  border-right: 1px solid var(--dots-sidebar-border, #d3e3e1);
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.side h2 {
  margin: 12px 6px 4px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--dots-muted-foreground, #4f6867);
}
.nav { padding: 4px 6px; border-radius: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.nav:hover { background: var(--dots-sidebar-accent, #dcefec); }
.nav.on { background: var(--dots-sidebar-accent, #dcefec); font-weight: 600; }
.side .add { margin-top: 8px; }

.list { flex: 1; min-width: 0; overflow-y: auto; padding: 16px 20px; }
.list h1 { margin: 0 0 12px; font-size: 16px; }
.group { margin-bottom: 14px; }
.group h3 { margin: 10px 0 4px; font-size: 13px; }
.section:hover { text-decoration: underline; }
.row {
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr) 120px 70px 96px;
  gap: 8px;
  align-items: center;
  padding: 5px 4px;
  border-bottom: 1px solid var(--dots-border, #d3e3e1);
}
.row[draggable="true"] { cursor: grab; }
.row.open { background: var(--dots-accent, #e3eceb); }
.row.head {
  font-size: 11px;
  text-transform: uppercase;
  color: var(--dots-muted-foreground, #4f6867);
}
.row .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.check { width: 20px; text-align: center; color: var(--dots-muted-foreground, #4f6867); }
.check.done { color: var(--dots-success, #0c7765); }
.cell { overflow: hidden; white-space: nowrap; }
.pill {
  display: inline-block;
  font-size: 11px;
  padding: 1px 7px;
  border-radius: 10px;
  background: var(--dots-secondary, #e3eceb);
  color: var(--dots-secondary-foreground, #1d3332);
}
.pill.dot, .pill.in_progress { background: var(--dots-primary, #0c7765); color: var(--dots-primary-foreground, #fff); }
.pill.in_review { background: var(--dots-needs-surface, #fff8e5); color: var(--dots-needs, #8f6000); }
.pill.done { background: var(--dots-muted, #e9f2f1); color: var(--dots-muted-foreground, #4f6867); }
.add { width: 100%; margin-top: 4px; border-style: dashed; background: transparent; }
.muted { color: var(--dots-muted-foreground, #4f6867); }
.error { color: var(--dots-destructive, #b23838); }
```

**Create `crates/dots-core/apps/projects/ui/projects.js`:**

```
// Projects' screen (app library spec section 8, mockup A, list view only): the left column (My
// tasks, Assigned to dots, the projects, + New project) and the list, grouped by section, with
// the columns Task, Assignee, Due and Status. Tasks and sections are added inline, sections
// renamed, and tasks dragged between sections. The page talks to Dots only through `dots`, the
// client Dots puts in it: every change is a call of a Projects tool, made as the person, and
// `dots.onChange` brings the screen up to date after any change, a dot's included.

export const STATUS_LABELS = {
  to_do: "To do",
  in_progress: "In progress",
  in_review: "In review",
  done: "Done",
};
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// The most tasks one list shows.
const LIST_LIMIT = 1000;

/// An element: `attrs` are attributes, with `on<event>` for listeners and `text` for its text;
/// a null or false attribute or child is left out.
export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "text") el.textContent = value;
    else if (key === "value") el.value = value;
    else el.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : String(child));
  }
  return el;
}

/// A due date as the list shows it: "Oct 28".
export function shortDate(day) {
  if (!day) return "";
  const [, month, date] = day.split("-").map(Number);
  return `${MONTHS[month - 1]} ${date}`;
}

/// Who a task is assigned to, as a pill.
export function assigneePill(assignee) {
  if (assignee === null) return "";
  if (assignee.kind === "person") return h("span", { class: "pill" }, "You");
  return h("span", { class: "pill dot" }, `● ${assignee.name ?? "removed dot"}`);
}

/// An input that does `action(text)` on Enter and forgets its text on Escape.
function entry(attrs, action) {
  return h("input", {
    ...attrs,
    onkeydown: (e) => {
      if (e.key === "Enter" && e.target.value.trim() !== "") {
        e.preventDefault();
        action(e.target.value.trim());
      } else if (e.key === "Escape") {
        e.target.value = "";
        e.target.blur();
      }
    },
  });
}

/// Shows Projects in `root`, talking to Dots through `dots`. Returns `ready`, which settles once
/// the first load has drawn, and `reload`.
export function mount(root, dots) {
  const state = {
    view: { kind: "my_tasks" },
    projects: [],
    dots: [],
    tasks: [],
    more: false,
    error: null,
    renaming: null,
    dragging: null,
    // The input to focus again after the next draw (an add row keeps the cursor).
    focus: null,
  };
  let loads = 0;

  async function load() {
    const mine = ++loads;
    try {
      const [projects, users] = await Promise.all([
        dots.call("list_projects", {}),
        dots.call("list_dots", {}),
      ]);
      if (state.view.kind === "project" && !projects.some((p) => p.id === state.view.id)) {
        state.view = { kind: "my_tasks" };
      }
      const listed = await dots.call("list_tasks", filter(state.view));
      if (mine !== loads) return;
      Object.assign(state, { projects, dots: users, tasks: listed.tasks, more: listed.more });
    } catch (e) {
      if (mine !== loads) return;
      state.error = String(e?.message ?? e);
    }
    draw();
  }

  /// Runs a change, then loads again; a refusal shows above the list.
  async function act(change) {
    state.error = null;
    try {
      await change();
    } catch (e) {
      state.error = String(e?.message ?? e);
    }
    await load();
  }

  function filter(view) {
    if (view.kind === "project") return { project_id: view.id, limit: LIST_LIMIT };
    return { view: view.kind, limit: LIST_LIMIT };
  }

  function show(view) {
    state.view = view;
    state.renaming = null;
    return load();
  }

  function draw() {
    root.replaceChildren(h("div", { class: "projects" }, side(), list()));
    if (state.focus !== null) {
      root.querySelector(`[data-focus="${state.focus}"]`)?.focus();
      state.focus = null;
    }
  }

  function side() {
    const item = (label, view, on) =>
      h("button", { class: on ? "nav on" : "nav", "aria-current": on ? "page" : null, onclick: () => show(view) }, label);
    const v = state.view;
    return h(
      "nav",
      { class: "side", "aria-label": "Projects" },
      item("My tasks", { kind: "my_tasks" }, v.kind === "my_tasks"),
      item("Assigned to dots", { kind: "assigned_to_dots" }, v.kind === "assigned_to_dots"),
      h("h2", { text: "Projects" }),
      state.projects.map((p) =>
        item(`● ${p.name}`, { kind: "project", id: p.id }, v.kind === "project" && v.id === p.id),
      ),
      entry({ class: "add", placeholder: "+ New project", "aria-label": "New project", "data-focus": "project" }, (name) =>
        act(async () => {
          const p = await dots.call("create_project", { name });
          state.view = { kind: "project", id: p.id };
        }),
      ),
    );
  }

  /// The list's groups: a project's sections (and its tasks in no section), or, in My tasks and
  /// Assigned to dots, one group per project with the to-dos first.
  function groups() {
    const v = state.view;
    if (v.kind === "project") {
      const project = state.projects.find((p) => p.id === v.id);
      const loose = state.tasks.filter((t) => t.section_id === null);
      const out = project.sections.map((s) => ({
        key: s.id,
        section: s,
        tasks: state.tasks.filter((t) => t.section_id === s.id),
        add: { project_id: v.id, section_id: s.id },
      }));
      if (loose.length > 0 || out.length === 0) {
        out.unshift({ key: "none", section: null, tasks: loose, add: { project_id: v.id } });
      }
      return out;
    }
    const todos = state.tasks.filter((t) => t.project_id === null);
    const out = [];
    if (v.kind === "my_tasks" || todos.length > 0) {
      out.push({ key: "todos", title: "To-dos", tasks: todos, add: v.kind === "my_tasks" ? {} : null });
    }
    for (const p of state.projects) {
      const tasks = state.tasks.filter((t) => t.project_id === p.id);
      if (tasks.length > 0) out.push({ key: p.id, title: p.name, tasks, add: null });
    }
    return out;
  }

  function title() {
    const v = state.view;
    if (v.kind === "my_tasks") return "My tasks";
    if (v.kind === "assigned_to_dots") return "Assigned to dots";
    return state.projects.find((p) => p.id === v.id)?.name ?? "";
  }

  function list() {
    const inProject = state.view.kind === "project";
    return h(
      "main",
      { class: "list" },
      h("h1", { text: title() }),
      state.error && h("p", { class: "error", role: "alert", text: state.error }),
      h(
        "div",
        { class: "row head", role: "row" },
        h("span"),
        h("span", { text: "Task" }),
        h("span", { text: "Assignee" }),
        h("span", { text: "Due" }),
        h("span", { text: "Status" }),
      ),
      groups().map(group),
      state.more && h("p", { class: "muted", text: `Showing the first ${LIST_LIMIT} tasks.` }),
      inProject &&
        entry({ class: "add", placeholder: "+ Add section", "aria-label": "New section", "data-focus": "section" }, (name) =>
          act(() => {
            state.focus = "section";
            return dots.call("create_section", { project_id: state.view.id, name });
          }),
        ),
    );
  }

  function heading(g) {
    if (!g.section) {
      return g.title ? h("h3", { text: g.title }) : null;
    }
    if (state.renaming === g.section.id) {
      return h(
        "h3",
        {},
        entry(
          { class: "rename", value: g.section.name, "aria-label": `Rename ${g.section.name}`, "data-focus": "rename", onblur: () => { state.renaming = null; draw(); } },
          (name) => {
            state.renaming = null;
            return act(() => dots.call("rename_section", { id: g.section.id, name }));
          },
        ),
      );
    }
    return h(
      "h3",
      {},
      h("button", {
        class: "section",
        title: "Rename the section",
        text: g.section.name,
        onclick: () => {
          state.renaming = g.section.id;
          state.focus = "rename";
          draw();
        },
      }),
    );
  }

  function group(g) {
    const inProject = state.view.kind === "project";
    const sectionId = g.section ? g.section.id : null;
    return h(
      "section",
      {
        class: "group",
        "aria-label": g.section ? g.section.name : (g.title ?? "No section"),
        ondragover: inProject ? (e) => e.preventDefault() : null,
        ondrop: inProject ? (e) => drop(e, sectionId, null) : null,
      },
      heading(g),
      g.tasks.map((t) => row(t, sectionId)),
      g.tasks.length === 0 && !g.add && h("p", { class: "muted", text: "No tasks here yet." }),
      g.add &&
        entry(
          {
            class: "add",
            placeholder: "+ Add task",
            "aria-label": `Add a task to ${g.section ? g.section.name : (g.title ?? title())}`,
            "data-focus": `add-${g.key}`,
          },
          (text) =>
            act(() => {
              state.focus = `add-${g.key}`;
              return dots.call("create_task", { title: text, ...g.add });
            }),
        ),
    );
  }

  /// A task dropped on the section `sectionId`, before the task `beforeId` (or at the end).
  function drop(e, sectionId, beforeId) {
    e.preventDefault();
    e.stopPropagation();
    const id = state.dragging;
    state.dragging = null;
    if (id === null || id === beforeId) return;
    act(() => dots.call("move_task", { id, section_id: sectionId, before_id: beforeId }));
  }

  function row(task, sectionId) {
    const done = task.status === "done";
    const inProject = state.view.kind === "project";
    return h(
      "div",
      {
        class: "row",
        role: "row",
        "aria-label": task.title,
        "data-task": task.id,
        draggable: inProject ? "true" : null,
        ondragstart: inProject
          ? (e) => {
              state.dragging = task.id;
              e.dataTransfer?.setData("text/plain", task.title);
            }
          : null,
        ondragover: inProject ? (e) => e.preventDefault() : null,
        ondrop: inProject ? (e) => drop(e, sectionId, task.id) : null,
      },
      h("button", {
        class: done ? "check done" : "check",
        "aria-label": done ? `Mark ${task.title} not done` : `Mark ${task.title} done`,
        text: done ? "✓" : "○",
        onclick: () => act(() => dots.call("update_task", { id: task.id, status: done ? "to_do" : "done" })),
      }),
      h("span", { class: "title", text: task.title }),
      h("span", { class: "cell" }, assigneePill(task.assignee)),
      h("span", { class: "cell", text: shortDate(task.due_on) }),
      h("span", { class: "cell" }, h("span", { class: `pill ${task.status}`, text: STATUS_LABELS[task.status] })),
    );
  }

  dots.onChange(() => {
    load();
  });
  return { ready: load(), reload: load };
}
```

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
        include_bytes!("../../apps/projects/ui/index.html"),
    ),
```

**with:**

```rust
        include_bytes!("../../apps/projects/ui/index.html"),
    ),
    (
        "ui/main.js",
        include_bytes!("../../apps/projects/ui/main.js"),
    ),
    (
        "ui/projects.css",
        include_bytes!("../../apps/projects/ui/projects.css"),
    ),
    (
        "ui/projects.js",
        include_bytes!("../../apps/projects/ui/projects.js"),
    ),
```

**In `ui/tsconfig.json`, replace:**

```json
    "resolveJsonModule": true,
    "isolatedModules": true,
```

**with:**

```json
    "resolveJsonModule": true,
    "allowJs": true,
    "isolatedModules": true,
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/apps` in `ui`, then `cargo test -j 2 -p dots-core --test apps_bundled_test`.
Expected: PASS: 1 file, 9 tests; 4 core tests.

- [ ] **Step 5: Run the checks**

Run: `bun run --cwd ui typecheck`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: clean; the UI 838 tests in 83 files.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/apps/projects/ui crates/dots-core/src/apps/bundled.rs ui/src/apps ui/tsconfig.json
git commit -m "feat(projects): the list screen: left column, sections, inline adds and dragging"
```

### Task 6: The side pane

Spec §8 "Screen": clicking a task opens the side pane with its title, assignee (the person or a dot that uses Projects; a removed dot as such), due date, status, Spec in Markdown (escaped first), subtasks, and Activity (newest last) with Undo on every change still in force, "Open the run" on a dot's run rows, and the comment box.

**Files:**

- Create: `crates/dots-core/apps/projects/ui/markdown.js`, `ui/src/apps/{markdown,projects-pane}.test.ts`
- Modify: `crates/dots-core/apps/projects/ui/{projects.js,projects.css}`, `crates/dots-core/src/apps/bundled.rs`

**Interfaces:**

- Consumes: Task 5's `mount`, `h`, `entry`, `shortDate`, `STATUS_LABELS`, `projects-fake.ts`; the tools `get_task` (with `activity: 200`), `update_task`, `add_subtask`, `set_subtask`, `undo`, `comment`; `dots.openRun(run_id)` (the page client's, Task 7).
- Produces: `markdown.js`: `markdown(text) -> string` (HTML), `escape(text)`; `projects.js`: `describe(row) -> string` (what an Activity row says after its actor). The pane's names: complementary "Task"; text box "Title"; combo boxes "Assignee" and "Status"; field "Due"; buttons "Close the task", "Edit the spec", "Save", "Cancel", "Undo", "Open the run", "Comment"; text boxes "Spec", "New subtask", "Comment"; list "Activity".

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/apps/markdown.test.ts`:**

````ts
// Projects' Markdown for a task's spec: everything escaped first, then a small set of markup.
import { describe, expect, it } from "vitest";

import { markdown } from "../../../crates/dots-core/apps/projects/ui/markdown.js";

describe("a spec's Markdown", () => {
  it("escapes HTML, so a spec cannot add elements or handlers of its own", () => {
    const html = markdown('<img src=x onerror="alert(1)"> & <script>bad()</script>');
    expect(html).toBe(
      "<p>&lt;img src=x onerror=&quot;alert(1)&quot;&gt; &amp; &lt;script&gt;bad()&lt;/script&gt;</p>",
    );
    const box = document.createElement("div");
    box.innerHTML = markdown("**<b onclick=x>hi</b>** [a](javascript:alert`1`) `<i>`");
    expect(box.querySelector("b, i, a, script, img")).toBeNull();
    expect(box.textContent).toBe("<b onclick=x>hi</b> a <i>");
  });

  it("writes headings, paragraphs, quotes and code", () => {
    expect(markdown("# Goal\nShip it\nsoon\n\n> wait\n> here")).toBe(
      "<h4>Goal</h4>\n<p>Ship it<br>soon</p>\n<blockquote>wait<br>here</blockquote>",
    );
    expect(markdown("```\nlet a = 1 < 2;\n**not bold**\n```")).toBe(
      "<pre><code>let a = 1 &lt; 2;\n**not bold**</code></pre>",
    );
    expect(markdown("Use `**x**` and **bold**, *it* and _it_ in snake_case_name")).toBe(
      "<p>Use <code>**x**</code> and <strong>bold</strong>, <em>it</em> and <em>it</em> in " +
        "snake_case_name</p>",
    );
  });

  it("writes lists, numbered lists and checkboxes", () => {
    expect(markdown("- one\n- [x] two\n- [ ] three\n\n1. first\n2. second")).toBe(
      '<ul><li>one</li><li class="task">☑ two</li><li class="task">☐ three</li></ul>\n' +
        "<ol><li>first</li><li>second</li></ol>",
    );
  });

  it("shows a link's text with its address as a title", () => {
    expect(markdown("See [the docs](https://example.com/a?b=1&c=2)")).toBe(
      '<p>See <span class="link" title="https://example.com/a?b=1&amp;c=2">the docs</span></p>',
    );
  });
});
````

**Create `ui/src/apps/projects-pane.test.ts`:**

```ts
// Projects' side pane (app library spec section 8): a task's fields, its spec in Markdown, its
// subtasks and its Activity, with Undo, a dot's runs and the comment box.
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { type Input, sample, show, signup } from "./projects-fake";

const row = (over: Input) => ({
  id: "a1",
  at: "2026-10-21T09:12:00+00:00",
  actor: { kind: "person", dot_id: null, name: "You" },
  kind: "change",
  field: null,
  before: null,
  after: null,
  text: null,
  run_id: null,
  undone_by: null,
  ...over,
});
const builder = { kind: "dot", dot_id: "d1", name: "builder" };

const detail = (over: Input = {}) => ({
  ...signup,
  spec: "## Form\nEmail and **password**.\n\n<img src=x onerror=alert(1)>",
  project: { id: "p1", name: "Website relaunch" },
  section: { id: "s1", name: "To do" },
  subtasks: [
    { id: "u1", title: "Form markup", done: true },
    { id: "u2", title: "Validation", done: false },
  ],
  activity: [
    row({ field: "created", after: { title: "Build signup form" } }),
    row({ id: "a2", field: "assignee", after: { kind: "dot", id: "d1", name: "builder" } }),
    row({ id: "a3", actor: builder, kind: "run", field: "started", run_id: "r1" }),
    row({
      id: "a4",
      actor: builder,
      field: "subtask:u1",
      before: { title: "Form markup", done: false },
      after: { title: "Form markup", done: true },
      at: "2026-10-21T09:40:00+00:00",
    }),
    row({ id: "a5", actor: builder, kind: "comment", text: "Markup done." }),
    row({ id: "a6", actor: builder, field: "title", before: "Signup", after: "Build signup form", undone_by: "a7" }),
    row({ id: "a8", actor: builder, kind: "run", field: "ended", after: "succeeded", text: "Form built.", run_id: "r1" }),
  ],
  activity_total: 7,
  ...over,
});

async function opened(more: Parameters<typeof sample>[0] = {}) {
  const fake = sample({ get_task: () => detail(), ...more });
  const user = await show(fake);
  await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
  await user.click(screen.getByRole("button", { name: "Build signup form" }));
  const pane = await screen.findByRole("complementary", { name: "Task" });
  return { fake, user, pane };
}

const item = (pane: HTMLElement, text: RegExp) =>
  within(within(pane).getByRole("list", { name: "Activity" }))
    .getAllByRole("listitem")
    .find((li) => text.test(li.textContent ?? ""))!;

describe("Projects' side pane", () => {
  it("shows the task's fields, and its spec as Markdown with its HTML as text", async () => {
    const { fake, pane } = await opened();
    expect(fake.callsTo("get_task")).toEqual([{ id: "t2", activity: 200 }]);
    expect(within(pane).getByRole("textbox", { name: "Title" })).toHaveValue("Build signup form");
    expect(within(pane).getByRole("combobox", { name: "Assignee" })).toHaveValue("d1");
    expect(within(pane).getByLabelText("Due")).toHaveValue("2026-10-25");
    expect(within(pane).getByRole("combobox", { name: "Status" })).toHaveValue("in_progress");
    expect(within(pane).getByRole("heading", { name: "Form" })).toBeVisible();
    expect(pane.querySelector("strong")).toHaveTextContent("password");
    expect(pane.querySelector(".markdown img")).toBeNull();
    expect(pane).toHaveTextContent("<img src=x onerror=alert(1)>");
    expect(screen.getByRole("row", { name: "Build signup form" })).toHaveClass("open");
  });

  it("changes the title, the assignee, the due date and the status", async () => {
    const { fake, user, pane } = await opened();
    const title = within(pane).getByRole("textbox", { name: "Title" });
    await user.clear(title);
    await user.type(title, "Signup{Tab}");
    await user.selectOptions(within(pane).getByRole("combobox", { name: "Assignee" }), "person");
    await user.selectOptions(within(pane).getByRole("combobox", { name: "Status" }), "in_review");
    await waitFor(() =>
      expect(fake.callsTo("update_task")).toEqual([
        { id: "t2", title: "Signup" },
        { id: "t2", assignee: "person" },
        { id: "t2", status: "in_review" },
      ]),
    );
    await user.clear(within(screen.getByRole("complementary", { name: "Task" })).getByLabelText("Due"));
    await waitFor(() =>
      expect(fake.callsTo("update_task").at(-1)).toEqual({ id: "t2", due_on: null }),
    );
  });

  it("edits the spec as text and saves it", async () => {
    const { fake, user, pane } = await opened();
    await user.click(within(pane).getByRole("button", { name: "Edit the spec" }));
    // Each change draws the pane again: find it anew.
    const box = screen.getByRole("textbox", { name: "Spec" });
    await user.clear(box);
    await user.type(box, "Just email");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(fake.callsTo("update_task")).toEqual([{ id: "t2", spec: "Just email" }]),
    );
  });

  it("checks a subtask and adds one", async () => {
    const { fake, user, pane } = await opened({ set_subtask: () => ({}), add_subtask: () => ({}) });
    expect(within(pane).getByRole("checkbox", { name: "Form markup" })).toBeChecked();
    await user.click(within(pane).getByRole("checkbox", { name: "Validation" }));
    await waitFor(() =>
      expect(fake.callsTo("set_subtask")).toEqual([{ id: "u2", done: true }]),
    );
    await user.type(
      within(screen.getByRole("complementary", { name: "Task" })).getByRole("textbox", {
        name: "New subtask",
      }),
      "Tests{Enter}",
    );
    await waitFor(() =>
      expect(fake.callsTo("add_subtask")).toEqual([{ task_id: "t2", title: "Tests" }]),
    );
  });

  it("shows the Activity newest last, with Undo on each change still in force", async () => {
    let refuse = false;
    const { fake, user, pane } = await opened({
      undo: () => {
        if (refuse) throw new Error("The subtask was changed since by You; undo that change first");
        return { undone: "a4", by: "a9" };
      },
    });
    const texts = within(within(pane).getByRole("list", { name: "Activity" }))
      .getAllByRole("listitem")
      .map((li) => li.textContent);
    expect(texts).toEqual([
      "You created the task · 09:12",
      "You assigned it to builder · 09:12Undo",
      "● builder started · 09:12Open the run",
      '● builder checked "Form markup" · 09:40Undo',
      "● builder: Markup done. · 09:12",
      '● builder renamed it to "Build signup form" · 09:12 (undone)',
      "● builder finished: Form built. · 09:12Open the run",
    ]);
    await user.click(within(item(pane, /checked/)).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.callsTo("undo")).toEqual([{ activity_id: "a4" }]));
    refuse = true;
    const again = screen.getByRole("complementary", { name: "Task" });
    await user.click(within(item(again, /checked/)).getByRole("button", { name: "Undo" }));
    expect(await within(screen.getByRole("complementary", { name: "Task" })).findByRole("alert"))
      .toHaveTextContent("The subtask was changed since by You; undo that change first");
  });

  it("opens a dot's run in its conversation, and says so when it cannot", async () => {
    const { fake, user, pane } = await opened();
    await user.click(within(item(pane, /started/)).getByRole("button", { name: "Open the run" }));
    expect(fake.opened).toEqual(["r1"]);
    fake.dots.openRun = async () => {
      throw new Error("the app projects started no run r1");
    };
    await user.click(within(item(pane, /finished/)).getByRole("button", { name: "Open the run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("started no run r1");
  });

  it("posts a comment", async () => {
    const { fake, user, pane } = await opened({ comment: () => ({ id: "a9" }) });
    await user.type(within(pane).getByRole("textbox", { name: "Comment" }), "Looks good");
    await user.click(within(pane).getByRole("button", { name: "Comment" }));
    await waitFor(() =>
      expect(fake.callsTo("comment")).toEqual([{ task_id: "t2", text: "Looks good" }]),
    );
  });

  it("closes, and closes by itself when the task is gone", async () => {
    let gone = false;
    const { fake, user, pane } = await opened({
      get_task: () => {
        if (gone) throw new Error("There is no task t2");
        return detail();
      },
    });
    await user.click(within(pane).getByRole("button", { name: "Close the task" }));
    expect(screen.queryByRole("complementary", { name: "Task" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Build signup form" }));
    await screen.findByRole("complementary", { name: "Task" });
    gone = true;
    fake.change();
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Task" })).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows a removed dot as the assignee it was", async () => {
    const removed = { kind: "dot", id: "dx", name: null };
    await opened({ get_task: () => detail({ assignee: removed }) });
    const select = screen.getByRole("combobox", { name: "Assignee" });
    expect(select).toHaveValue("dx");
    expect(within(select).getByRole("option", { name: "● removed dot" })).toBeVisible();
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/apps` in `ui`.
Expected: FAIL: `ui/markdown.js` cannot be resolved; the pane's tests find no button named "Build signup form".

- [ ] **Step 3: Add the side pane**

**Create `crates/dots-core/apps/projects/ui/markdown.js`:**

````
// A task's spec in Markdown, as HTML for the side pane. Every character of the text is escaped
// first, so the only markup in the result is what this file writes: headings, paragraphs, lists
// with checkboxes, quotes, code, bold and italic. A link shows its text, with the address as a
// title: the page cannot open it (its frame opens no popups and navigates nowhere).

const ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export const escape = (text) => text.replace(/[&<>"']/g, (c) => ESCAPES[c]);

/// One line's inline markup, on escaped text.
function inline(text) {
  const codes = [];
  // Code is taken out first, so nothing inside it is read as markup.
  let out = text.replace(/`([^`]+)`/g, (_, code) => {
    codes.push(code);
    return `\u0000${codes.length - 1}\u0000`;
  });
  out = out
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<span class="link" title="$2">$1</span>')
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_]+)__/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
    .replace(/(^|[^\w_])_([^_\s][^_]*)_(?!\w)/g, "$1<em>$2</em>");
  return out.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${codes[Number(i)]}</code>`);
}

const LIST_ITEM = /^\s*([-*+]|\d+[.)])\s+(.*)$/;
const CHECKBOX = /^\[([ xX])\]\s+(.*)$/;

/// `text` (Markdown) as HTML.
export function markdown(text) {
  // NUL marks the code taken out of a line; the text cannot hold one.
  const lines = escape(text.replace(/\u0000/g, "").replace(/\r\n?/g, "\n")).split("\n");
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      i += 1;
      continue;
    }
    if (line.startsWith("```")) {
      const code = [];
      i += 1;
      while (i < lines.length && !lines[i].startsWith("```")) code.push(lines[i++]);
      i += 1;
      out.push(`<pre><code>${code.join("\n")}</code></pre>`);
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = Math.min(heading[1].length + 3, 6);
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      i += 1;
      continue;
    }
    if (line.startsWith("&gt;")) {
      const quote = [];
      while (i < lines.length && lines[i].startsWith("&gt;")) {
        quote.push(lines[i++].replace(/^&gt;\s?/, ""));
      }
      out.push(`<blockquote>${quote.map(inline).join("<br>")}</blockquote>`);
      continue;
    }
    const first = LIST_ITEM.exec(line);
    if (first) {
      const ordered = /\d/.test(first[1]);
      const items = [];
      while (i < lines.length && LIST_ITEM.test(lines[i])) {
        const item = LIST_ITEM.exec(lines[i++])[2];
        const box = CHECKBOX.exec(item);
        items.push(
          box
            ? `<li class="task">${box[1] === " " ? "☐" : "☑"} ${inline(box[2])}</li>`
            : `<li>${inline(item)}</li>`,
        );
      }
      const tag = ordered ? "ol" : "ul";
      out.push(`<${tag}>${items.join("")}</${tag}>`);
      continue;
    }
    const para = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("```") &&
      !/^#{1,6}\s/.test(lines[i]) &&
      !lines[i].startsWith("&gt;") &&
      !LIST_ITEM.test(lines[i])
    ) {
      para.push(inline(lines[i++]));
    }
    out.push(`<p>${para.join("<br>")}</p>`);
  }
  return out.join("\n");
}
````

**In `crates/dots-core/apps/projects/ui/projects.css`, replace:**

```
.error { color: var(--dots-destructive, #b23838); }
```

**with:**

```
.error { color: var(--dots-destructive, #b23838); }
.row .title { text-align: left; }
.row .title:hover { text-decoration: underline; }

.pane {
  width: 340px;
  flex: none;
  overflow-y: auto;
  padding: 12px 14px;
  border-left: 1px solid var(--dots-border, #d3e3e1);
  background: var(--dots-card, #fff);
  position: relative;
}
.pane .close { position: absolute; top: 8px; right: 10px; font-size: 18px; }
.pane-title { width: calc(100% - 24px); font-size: 15px; font-weight: 600; border-color: transparent; }
.fields { display: grid; grid-template-columns: 70px 1fr; gap: 6px; align-items: center; margin: 10px 0; }
.fields label { color: var(--dots-muted-foreground, #4f6867); }
.pane h4 { margin: 14px 0 4px; font-size: 12px; text-transform: uppercase; color: var(--dots-muted-foreground, #4f6867); }
.spec-edit, .activity textarea { width: 100%; resize: vertical; }
.markdown { overflow-wrap: anywhere; }
.markdown p, .markdown ul, .markdown ol, .markdown pre, .markdown blockquote { margin: 4px 0; }
.markdown pre, .markdown code { font-family: ui-monospace, Consolas, monospace; font-size: 12px; background: var(--dots-muted, #e9f2f1); border-radius: 4px; }
.markdown pre { padding: 6px 8px; overflow-x: auto; }
.markdown blockquote { padding-left: 8px; border-left: 3px solid var(--dots-border, #d3e3e1); }
.markdown li.task { list-style: none; margin-left: -14px; }
.markdown .link { text-decoration: underline dotted; }
.subtasks { list-style: none; padding: 0; margin: 0; }
.activity ol { list-style: none; padding: 0; margin: 0 0 8px; }
.activity li { margin: 6px 0; }
.activity li.undone { opacity: 0.6; }
.who { font-weight: 600; }
.link-button { color: var(--dots-primary, #0c7765); margin-left: 6px; text-decoration: underline; }
.buttons { display: flex; gap: 8px; margin-top: 6px; }
.primary { background: var(--dots-primary, #0c7765); color: var(--dots-primary-foreground, #fff); border-radius: 6px; padding: 3px 10px; margin-top: 4px; }
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
// the columns Task, Assignee, Due and Status. Tasks and sections are added inline, sections
// renamed, and tasks dragged between sections. The page talks to Dots only through `dots`, the
// client Dots puts in it: every change is a call of a Projects tool, made as the person, and
// `dots.onChange` brings the screen up to date after any change, a dot's included.

```

**with:**

```
// the columns Task, Assignee, Due and Status. Tasks and sections are added inline, sections
// renamed, and tasks dragged between sections. A task opens in the side pane: its fields, its
// spec in Markdown, its subtasks and its Activity, where a change has Undo and a dot's run opens
// in its conversation. The page talks to Dots only through `dots`, the client Dots puts in it:
// every change is a call of a Projects tool, made as the person, and `dots.onChange` brings the
// screen up to date after any change, a dot's included.

import { markdown } from "./markdown.js";

```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
const LIST_LIMIT = 1000;

```

**with:**

```
const LIST_LIMIT = 1000;
/// The most Activity rows the side pane shows.
const PANE_ACTIVITY = 200;

```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```

/// An input that does `action(text)` on Enter and forgets its text on Escape.
```

**with:**

```

/// Who did an Activity row's change: "You" or "● <dot>".
const actorName = (actor) => (actor.kind === "person" ? "You" : `● ${actor.name}`);

/// An Activity row's time: "09:40".
const clock = (at) => at.slice(11, 16);

/// What an Activity row says, after the actor's name.
export function describe(row) {
  const { field, before, after } = row;
  if (row.kind === "comment") return `: ${row.text}`;
  if (row.kind === "run") {
    if (field === "started") return " started";
    if (field === "not_started") return ` could not start: ${row.text}`;
    const said = row.text ? `: ${row.text}` : "";
    if (after === "succeeded") return ` finished${said}`;
    return `'s run ${after === "cancelled" ? "was cancelled" : "failed"}${said}`;
  }
  switch (field) {
    case "created":
      return " created the task";
    case "title":
      return ` renamed it to "${after}"`;
    case "spec":
      return " changed the spec";
    case "status":
      return ` set the status to ${STATUS_LABELS[after]}`;
    case "assignee":
      if (after === null) return " unassigned it";
      return ` assigned it to ${after.kind === "person" ? "you" : (after.name ?? "a removed dot")}`;
    case "due_on":
      return after === null ? " removed the due date" : ` set the due date to ${shortDate(after)}`;
    case "section":
      if (after === null) return " made it a to-do";
      return ` moved it to ${after.project ?? "a project"}${after.section ? ` › ${after.section}` : ""}`;
    default:
      if (before === null) return ` added the subtask "${after.title}"`;
      if (after === null) return ` removed the subtask "${before.title}"`;
      if (before.title !== after.title) {
        return ` renamed the subtask "${before.title}" to "${after.title}"`;
      }
      return ` ${after.done ? "checked" : "unchecked"} "${after.title}"`;
  }
}

/// An input that does `action(text)` on Enter and forgets its text on Escape.
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
    focus: null,
  };
```

**with:**

```
    focus: null,
    // The task open in the side pane: its id, what get_task gave, whether its spec is being
    // edited, and a refusal of a change made there.
    open: null,
    task: null,
    editingSpec: false,
    paneError: null,
  };
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
      const listed = await dots.call("list_tasks", filter(state.view));
      if (mine !== loads) return;
      Object.assign(state, { projects, dots: users, tasks: listed.tasks, more: listed.more });
    } catch (e) {
```

**with:**

```
      const listed = await dots.call("list_tasks", filter(state.view));
      const task = state.open === null ? null : await openTask(state.open);
      if (mine !== loads) return;
      Object.assign(state, { projects, dots: users, tasks: listed.tasks, more: listed.more, task });
      // A task deleted meanwhile closes its pane.
      if (task === null) state.open = null;
    } catch (e) {
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
    draw();
  }
```

**with:**

```
    draw();
  }

  /// The task `id` for the side pane, or null when it is gone.
  async function openTask(id) {
    try {
      return await dots.call("get_task", { id, activity: PANE_ACTIVITY });
    } catch (e) {
      if (String(e?.message ?? e).startsWith("There is no task")) return null;
      throw e;
    }
  }

  function select(id) {
    state.open = id;
    state.editingSpec = false;
    state.paneError = null;
    return load();
  }

  /// A change made in the side pane, then a load; a refusal shows in the pane.
  async function paneAct(change) {
    state.paneError = null;
    try {
      await change();
    } catch (e) {
      state.paneError = String(e?.message ?? e);
    }
    await load();
  }
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
  function draw() {
    root.replaceChildren(h("div", { class: "projects" }, side(), list()));
    if (state.focus !== null) {
```

**with:**

```
  function draw() {
    root.replaceChildren(
      h("div", { class: "projects" }, side(), list(), state.task && pane(state.task)),
    );
    if (state.focus !== null) {
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
      {
        class: "row",
        role: "row",
```

**with:**

```
      {
        class: state.open === task.id ? "row open" : "row",
        role: "row",
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
      }),
      h("span", { class: "title", text: task.title }),
      h("span", { class: "cell" }, assigneePill(task.assignee)),
```

**with:**

```
      }),
      h("button", { class: "title", text: task.title, onclick: () => select(task.id) }),
      h("span", { class: "cell" }, assigneePill(task.assignee)),
```

**In `crates/dots-core/apps/projects/ui/projects.js`, replace:**

```
      h("span", { class: "cell" }, h("span", { class: `pill ${task.status}`, text: STATUS_LABELS[task.status] })),
    );
```

**with:**

```
      h("span", { class: "cell" }, h("span", { class: `pill ${task.status}`, text: STATUS_LABELS[task.status] })),
    );
  }

  /// The side pane of the open task `t`.
  function pane(t) {
    const update = (fields) => paneAct(() => dots.call("update_task", { id: t.id, ...fields }));
    const listed = t.assignee?.kind === "dot" && state.dots.some((d) => d.id === t.assignee.id);
    const assignee = t.assignee === null ? "" : t.assignee.kind === "person" ? "person" : t.assignee.id;
    const pick = (value) => (value === assignee ? true : null);
    return h(
      "aside",
      { class: "pane", "aria-label": "Task" },
      h("button", {
        class: "close",
        "aria-label": "Close the task",
        text: "×",
        onclick: () => {
          state.open = null;
          state.task = null;
          draw();
        },
      }),
      h("input", {
        class: "pane-title",
        "aria-label": "Title",
        value: t.title,
        onchange: (e) => e.target.value.trim() !== "" && update({ title: e.target.value.trim() }),
      }),
      state.paneError && h("p", { class: "error", role: "alert", text: state.paneError }),
      h(
        "div",
        { class: "fields" },
        h("label", { for: "pane-assignee", text: "Assignee" }),
        h(
          "select",
          { id: "pane-assignee", onchange: (e) => update({ assignee: e.target.value || null }) },
          h("option", { value: "", selected: pick(""), text: "No one" }),
          h("option", { value: "person", selected: pick("person"), text: "You" }),
          state.dots.map((d) => h("option", { value: d.id, selected: pick(d.id), text: `● ${d.name}` })),
          t.assignee?.kind === "dot" &&
            !listed &&
            h("option", {
              value: t.assignee.id,
              selected: true,
              text: `● ${t.assignee.name ?? "removed dot"}`,
            }),
        ),
        h("label", { for: "pane-due", text: "Due" }),
        h("input", {
          id: "pane-due",
          type: "date",
          value: t.due_on ?? "",
          onchange: (e) => update({ due_on: e.target.value || null }),
        }),
        h("label", { for: "pane-status", text: "Status" }),
        h(
          "select",
          { id: "pane-status", onchange: (e) => update({ status: e.target.value }) },
          Object.entries(STATUS_LABELS).map(([value, text]) =>
            h("option", { value, selected: value === t.status ? true : null, text }),
          ),
        ),
      ),
      spec(t, update),
      h("h4", { text: "Subtasks" }),
      h(
        "ul",
        { class: "subtasks" },
        t.subtasks.map((s) =>
          h(
            "li",
            {},
            h(
              "label",
              {},
              h("input", {
                type: "checkbox",
                checked: s.done ? true : null,
                onchange: (e) =>
                  paneAct(() => dots.call("set_subtask", { id: s.id, done: e.target.checked })),
              }),
              ` ${s.title}`,
            ),
          ),
        ),
      ),
      entry(
        { class: "add", placeholder: "+ Add subtask", "aria-label": "New subtask", "data-focus": "subtask" },
        (title) =>
          paneAct(() => {
            state.focus = "subtask";
            return dots.call("add_subtask", { task_id: t.id, title });
          }),
      ),
      activity(t),
    );
  }

  /// The task's spec: shown as Markdown, or edited as text.
  function spec(t, update) {
    if (state.editingSpec) {
      const box = h("textarea", { class: "spec-edit", "aria-label": "Spec", rows: 10, value: t.spec });
      return h(
        "div",
        { class: "spec" },
        h("h4", { text: "Spec" }),
        box,
        h(
          "div",
          { class: "buttons" },
          h("button", {
            class: "primary",
            text: "Save",
            onclick: () => {
              state.editingSpec = false;
              update({ spec: box.value });
            },
          }),
          h("button", {
            text: "Cancel",
            onclick: () => {
              state.editingSpec = false;
              draw();
            },
          }),
        ),
      );
    }
    const shown = h("div", { class: "markdown" });
    if (t.spec.trim() === "") shown.append(h("p", { class: "muted", text: "No spec yet." }));
    // `markdown` escapes every character of the spec before it adds markup of its own.
    else shown.innerHTML = markdown(t.spec);
    return h(
      "div",
      { class: "spec" },
      h(
        "h4",
        {},
        "Spec ",
        h("button", {
          class: "link-button",
          "aria-label": "Edit the spec",
          text: "Edit",
          onclick: () => {
            state.editingSpec = true;
            draw();
          },
        }),
      ),
      shown,
    );
  }

  /// The task's Activity, newest last, with Undo on each change still in force, a dot's runs
  /// opening in its conversation, and the comment box.
  function activity(t) {
    const box = h("textarea", { "aria-label": "Comment", rows: 2, placeholder: "Comment…" });
    return h(
      "div",
      { class: "activity" },
      h("h4", { text: "Activity" }),
      t.activity_total > t.activity.length &&
        h("p", { class: "muted", text: `The last ${t.activity.length} of ${t.activity_total}.` }),
      h(
        "ol",
        { "aria-label": "Activity" },
        t.activity.map((r) =>
          h(
            "li",
            { class: r.undone_by ? "undone" : null },
            h("span", { class: "who", text: actorName(r.actor) }),
            describe(r),
            h("span", { class: "muted", text: ` · ${clock(r.at)}` }),
            r.undone_by && h("span", { class: "muted", text: " (undone)" }),
            r.kind === "change" &&
              r.field !== "created" &&
              !r.undone_by &&
              h("button", {
                class: "link-button",
                text: "Undo",
                onclick: () => paneAct(() => dots.call("undo", { activity_id: r.id })),
              }),
            r.kind === "run" &&
              r.run_id &&
              h("button", {
                class: "link-button",
                text: "Open the run",
                onclick: () =>
                  Promise.resolve()
                    .then(() => dots.openRun(r.run_id))
                    .catch((e) => {
                      state.paneError = String(e?.message ?? e);
                      draw();
                    }),
              }),
          ),
        ),
      ),
      box,
      h("button", {
        class: "primary",
        text: "Comment",
        onclick: () =>
          box.value.trim() !== "" &&
          paneAct(() => dots.call("comment", { task_id: t.id, text: box.value.trim() })),
      }),
    );
```

**In `crates/dots-core/src/apps/bundled.rs`, replace:**

```rust
        include_bytes!("../../apps/projects/ui/main.js"),
    ),
```

**with:**

```rust
        include_bytes!("../../apps/projects/ui/main.js"),
    ),
    (
        "ui/markdown.js",
        include_bytes!("../../apps/projects/ui/markdown.js"),
    ),
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/apps` in `ui`, then `cargo test -j 2 -p dots-core --test apps_bundled_test`.
Expected: PASS: 3 files, 22 tests; 4 core tests.

- [ ] **Step 5: Run the checks**

Run: `bun run --cwd ui typecheck`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: clean; the UI 851 tests in 85 files.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/apps/projects/ui crates/dots-core/src/apps/bundled.rs ui/src/apps
git commit -m "feat(projects): the side pane: fields, the spec in Markdown, subtasks and Activity"
```

### Task 7: The host: a run's link, the prompt, Install Projects

The page's client gains `dots.openRun`, the bridge answers `{ type: "open_run", id, run_id }` (refused where the frame has no `openRun`, as a preview's has not), `/apps/:name` opens the run's conversation through `app_run_link` and shows the one-time prompt, and the Apps screen lists "Comes with Dots" with "Install Projects". Plan B's untested branches get their tests: AppScreen on an app whose code cannot be read, another app's data event, a theme change after load.

**Files:**

- Modify: `apps/dots-app/src/app_client.js`, `ui/src/lib/{app-bridge,query}.ts`, `ui/src/components/AppFrame.tsx`, `ui/src/screens/apps/{AppScreen,AppsScreen}.tsx`
- Test: `ui/src/lib/app-client.test.ts`, `ui/src/components/AppFrame.test.tsx`, `ui/src/screens/apps/AppsScreen.test.tsx`

**Interfaces:**

- Consumes: Task 4's commands and `AppConfig.start_asked`; plan B's `connectFrame`, `AppFrame`, `appConfig`.
- Produces: the page's `dots.openRun(runId) -> Promise<null>`; `connectFrame(frame, { tools, call, openRun? })`; `AppFrame`'s `openRun?: (runId) => Promise<void>`; `asksToStart(row, config)` in `AppScreen.tsx`; `keys.appsBundled = ["apps", "bundled"]`. Names: region "Let <title> start dots?" with buttons "Switch on" and "Not now"; region "Comes with Dots" with "Install <title>", whose dialog has the checkbox "Start on the data kept from the removed <title> …" and "Install".

- [ ] **Step 1: Write the failing tests**

**In `ui/src/components/AppFrame.test.tsx`, replace:**

```tsx
import { appSrc } from "@/lib/app-bridge";

```

**with:**

```tsx
import { appSrc } from "@/lib/app-bridge";
import { applyTheme } from "@/lib/theme";

```

**In `ui/src/components/AppFrame.test.tsx`, replace:**

```tsx

  it("tells the page of each change to the app's data", async () => {
```

**with:**

```tsx

  it("opens a run the page asks for, and refuses where the screen cannot open runs", async () => {
    const openRun = vi.fn(async () => {});
    const { unmount } = render(
      <AppFrame
        src={appSrc("projects", 1)}
        title="Projects"
        tools={[]}
        call={vi.fn()}
        openRun={openRun}
      />,
    );
    const { port, got } = load(screen.getByTitle<HTMLIFrameElement>("Projects"));
    port.postMessage({ type: "open_run", id: 1, run_id: "r1" });
    await waitFor(() =>
      expect(got).toContainEqual({ type: "result", id: 1, ok: true, result: null }),
    );
    expect(openRun).toHaveBeenCalledWith("r1");
    unmount();
    render(<AppFrame src={appSrc("projects", 1)} title="Preview" tools={[]} call={vi.fn()} />);
    const preview = load(screen.getByTitle<HTMLIFrameElement>("Preview"));
    preview.port.postMessage({ type: "open_run", id: 2, run_id: "r1" });
    await waitFor(() =>
      expect(preview.got).toContainEqual({
        type: "result",
        id: 2,
        ok: false,
        error: "this screen cannot open runs",
      }),
    );
  });

  it("sends the theme again when it changes after the page loaded", async () => {
    render(<AppFrame src={appSrc("expenses", 1)} title="Expenses" tools={[]} call={vi.fn()} />);
    const { got } = load(screen.getByTitle<HTMLIFrameElement>("Expenses"));
    await waitFor(() => expect(got).toHaveLength(1));
    act(() => {
      applyTheme("dark");
    });
    try {
      await waitFor(() => expect(got.at(-1)).toMatchObject({ type: "theme", mode: "dark" }));
    } finally {
      act(() => {
        applyTheme("light");
      });
    }
  });

  it("tells the page of each change to the app's data", async () => {
```

**In `ui/src/lib/app-client.test.ts`, replace:**

```ts
  call: (tool: string, input?: unknown) => Promise<unknown>;
  onChange: (cb: () => void) => () => void;
```

**with:**

```ts
  call: (tool: string, input?: unknown) => Promise<unknown>;
  openRun: (runId: string) => Promise<unknown>;
  onChange: (cb: () => void) => () => void;
```

**In `ui/src/lib/app-client.test.ts`, replace:**

```ts

  it("takes one port, and only from the window that framed it", async () => {
```

**with:**

```ts

  it("asks the host to open a run, and settles with its answer", async () => {
    const p = page();
    const port = connect(p);
    const opened = p.dots.openRun("r1");
    expect(await next(port)).toEqual({ type: "open_run", id: 1, run_id: "r1" });
    port.postMessage({ type: "result", id: 1, ok: true, result: null });
    await expect(opened).resolves.toBeNull();
    const refused = p.dots.openRun("r2");
    await next(port);
    port.postMessage({ type: "result", id: 2, ok: false, error: "the app started no run r2" });
    await expect(refused).rejects.toThrow("the app started no run r2");
  });

  it("takes one port, and only from the window that framed it", async () => {
```

**In `ui/src/screens/apps/AppsScreen.test.tsx`, replace:**

```tsx

import type { AppRow, KeptData } from "@/api/types";
import { backend } from "@/test/backend";
```

**with:**

```tsx

import type { AppRow, BundledOffer, KeptData } from "@/api/types";
import { backend } from "@/test/backend";
```

**In `ui/src/screens/apps/AppsScreen.test.tsx`, replace:**

```tsx
const [kept] = T.KeptData.array().parse(contract.KeptData) as [KeptData];

```

**with:**

```tsx
const [kept] = T.KeptData.array().parse(contract.KeptData) as [KeptData];
const [onKept, fresh] = T.BundledOffer.array().parse(contract.BundledOffer) as [
  BundledOffer,
  BundledOffer,
];

/// Projects as Dots installs it: bundled, asking to start dots, the switch off and the prompt
/// not answered unless `asked`.
function bundled(asked = false): AppRow {
  if (projects.item.kind !== "app") throw new Error("an app row");
  return {
    ...projects,
    item: {
      ...projects.item,
      config: { ...projects.item.config, can_start_dots: false, start_asked: asked },
    },
  };
}

```

**In `ui/src/screens/apps/AppsScreen.test.tsx`, replace:**

```tsx

describe("an app's own screen", () => {
```

**with:**

```tsx

describe("Projects, which comes with Dots", () => {
  it("offers Install Projects on the kept data, which the person may untick", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => [kept]);
    backend.on("apps_bundled_offers", () => [onKept]);
    backend.on("app_install_bundled", () => projects.item);
    const { user } = renderApp("/apps");
    const section = await screen.findByRole("region", { name: "Comes with Dots" });
    await user.click(within(section).getByRole("button", { name: "Install Projects" }));
    const dialog = await screen.findByRole("alertdialog");
    const box = within(dialog).getByRole("checkbox", { name: /Start on the data kept/ });
    expect(box).toBeChecked();
    expect(dialog).toHaveTextContent("it then runs 0002_labels.sql on it");
    await user.click(within(dialog).getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(backend.callsTo("app_install_bundled")).toEqual([
        { name: "projects", keptId: onKept.kept!.id },
      ]),
    );
    await user.click(within(section).getByRole("button", { name: "Install Projects" }));
    const again = await screen.findByRole("alertdialog");
    await user.click(within(again).getByRole("checkbox", { name: /Start on the data kept/ }));
    await user.click(within(again).getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(backend.callsTo("app_install_bundled").at(-1)).toEqual({
        name: "projects",
        keptId: null,
      }),
    );
  });

  it("installs fresh when no kept data fits", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => []);
    backend.on("apps_bundled_offers", () => [fresh]);
    backend.on("app_install_bundled", () => projects.item);
    const { user } = renderApp("/apps");
    await user.click(await screen.findByRole("button", { name: "Install Projects" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).queryByRole("checkbox")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(backend.callsTo("app_install_bundled")).toEqual([{ name: "projects", keptId: null }]),
    );
  });

  it("asks once above its screen to let it start dots", async () => {
    let row = bundled();
    backend.on("apps_list", () => [row]);
    backend.on("app_answer_start_prompt", () => {
      row = bundled(true);
      return row.item;
    });
    const { user } = renderApp("/apps/projects");
    const prompt = await screen.findByRole("region", { name: "Let Projects start dots?" });
    await user.click(within(prompt).getByRole("button", { name: "Not now" }));
    await waitFor(() =>
      expect(backend.callsTo("app_answer_start_prompt")).toEqual([{ name: "projects", on: false }]),
    );
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Let Projects start dots?" })).toBeNull(),
    );
  });

  it("switches starting dots on from the prompt", async () => {
    backend.on("apps_list", () => [bundled()]);
    backend.on("app_answer_start_prompt", () => projects.item);
    const { user } = renderApp("/apps/projects");
    const prompt = await screen.findByRole("region", { name: "Let Projects start dots?" });
    await user.click(within(prompt).getByRole("button", { name: "Switch on" }));
    await waitFor(() =>
      expect(backend.callsTo("app_answer_start_prompt")).toEqual([{ name: "projects", on: true }]),
    );
  });

  it("opens a run the page asks for in the dot's conversation", async () => {
    backend.on("apps_list", () => [bundled(true)]);
    backend.on("app_run_link", () => ({ dot_id: "d1", root_run_id: "r1" }));
    const { router } = renderApp("/apps/projects");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Projects");
    expect(screen.queryByRole("region", { name: "Let Projects start dots?" })).toBeNull();
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    port.postMessage({ type: "open_run", id: 1, run_id: "r9" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/d1/r1"));
    expect(backend.callsTo("app_run_link")).toEqual([{ name: "projects", runId: "r9" }]);
  });
});

describe("an app's own screen", () => {
```

**In `ui/src/screens/apps/AppsScreen.test.tsx`, replace:**

```tsx

  it("says so for an app it does not have", async () => {
```

**with:**

```tsx

  it("does not tell the frame of another app's data changes", async () => {
    backend.on("apps_list", () => [expenses()]);
    renderApp("/apps/expenses");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses");
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    const got: unknown[] = [];
    port.onmessage = (e: MessageEvent) => got.push(e.data);
    act(() => backend.emit("runtime", { type: "app_data_changed", app: "projects" }));
    act(() => backend.emit("runtime", { type: "app_data_changed", app: "expenses" }));
    await waitFor(() => expect(got).toContainEqual({ type: "changed" }));
    expect(got.filter((m) => (m as { type: string }).type === "changed")).toHaveLength(1);
  });

  it("shows why an app whose code cannot be read does not open", async () => {
    backend.on("apps_list", () => [broken]);
    renderApp(`/apps/${broken.item.name}`);
    expect(
      await screen.findByText(/cannot open: invalid input: app.json: EOF while parsing an object/),
    ).toBeVisible();
    expect(document.querySelector("iframe")).toBeNull();
  });

  it("says so for an app it does not have", async () => {
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/app-client.test.ts src/components/AppFrame.test.tsx src/screens/apps` in `ui`.
Expected: FAIL: `dots.openRun` is not a function; no answer to `open_run`; no region "Comes with Dots" or "Let Projects start dots?". The three tests of plan B's untested branches pass already.

- [ ] **Step 3: Add the run link, the prompt and Install Projects**

**In `apps/dots-app/src/app_client.js`, replace:**

```
// `data-theme` set to "light" or "dark" (`dots.onTheme(cb)` hears a change).
(() => {
```

**with:**

```
// `data-theme` set to "light" or "dark" (`dots.onTheme(cb)` hears a change).
// `dots.openRun(runId)` opens, in Dots, the conversation of a run the app started; it rejects
// for any other run.
(() => {
```

**In `apps/dots-app/src/app_client.js`, replace:**

```
  const send = (msg) => (port === null ? queued.push(msg) : deliver(msg));
  const dots = Object.freeze({
```

**with:**

```
  const send = (msg) => (port === null ? queued.push(msg) : deliver(msg));
  // A request the host answers with a result of the same id.
  const ask = (msg) =>
    new Promise((resolve, reject) => {
      next += 1;
      waiting.set(next, { resolve, reject });
      send({ ...msg, id: next });
    });
  const dots = Object.freeze({
```

**In `apps/dots-app/src/app_client.js`, replace:**

```
    call(tool, input = {}) {
      return new Promise((resolve, reject) => {
        next += 1;
        waiting.set(next, { resolve, reject });
        send({ type: "call", id: next, tool: String(tool), input });
      });
    },
```

**with:**

```
    call(tool, input = {}) {
      return ask({ type: "call", tool: String(tool), input });
    },
    openRun(runId) {
      return ask({ type: "open_run", run_id: String(runId) });
    },
```

**In `ui/src/components/AppFrame.tsx`, replace:**

```tsx
/// Dots served: it is cut off, and the person can load it afresh. `changes` counts the changes
/// to the app's data; each new count tells the page (`dots.onChange`).
export function AppFrame(props: {
```

**with:**

```tsx
/// Dots served: it is cut off, and the person can load it afresh. `changes` counts the changes
/// to the app's data; each new count tells the page (`dots.onChange`). `openRun` opens a run the
/// app started (`dots.openRun`); without it the page's asks are refused.
export function AppFrame(props: {
```

**In `ui/src/components/AppFrame.tsx`, replace:**

```tsx
  call: (tool: string, input: unknown) => Promise<unknown>;
  changes?: number;
```

**with:**

```tsx
  call: (tool: string, input: unknown) => Promise<unknown>;
  openRun?: (runId: string) => Promise<void>;
  changes?: number;
```

**In `ui/src/components/AppFrame.tsx`, replace:**

```tsx
  // What the bridge reads at each call, without a new bridge on each render.
  const latest = useRef({ tools: props.tools, call: props.call, theme });
  latest.current = { tools: props.tools, call: props.call, theme };
  useEffect(() => bridge.current?.theme(theme), [theme]);
```

**with:**

```tsx
  // What the bridge reads at each call, without a new bridge on each render.
  const latest = useRef({ tools: props.tools, call: props.call, openRun: props.openRun, theme });
  latest.current = { tools: props.tools, call: props.call, openRun: props.openRun, theme };
  useEffect(() => bridge.current?.theme(theme), [theme]);
```

**In `ui/src/components/AppFrame.tsx`, replace:**

```tsx
      call: (tool, input) => latest.current.call(tool, input),
    });
```

**with:**

```tsx
      call: (tool, input) => latest.current.call(tool, input),
      openRun: (runId) => {
        const open = latest.current.openRun;
        return open ? open(runId) : Promise.reject(new Error("this screen cannot open runs"));
      },
    });
```

**In `ui/src/lib/app-bridge.ts`, replace:**

```ts
// is checked here before anything is called: a call names a tool the app lists, and Dots calls
// it as the person (the page cannot say who calls). The page's client is
// apps/dots-app/src/app_client.js.
import { z } from "zod";
```

**with:**

```ts
// is checked here before anything is called: a call names a tool the app lists, and Dots calls
// it as the person (the page cannot say who calls); a run the page asks to open is one the app
// started, which the core checks. The page's client is apps/dots-app/src/app_client.js.
import { z } from "zod";
```

**In `ui/src/lib/app-bridge.ts`, replace:**

```ts
});

```

**with:**

```ts
});
/// The page asks to open a run in its conversation (`dots.openRun`).
const OpenRun = z.strictObject({
  type: z.literal("open_run"),
  id: z.number().int().positive(),
  run_id: z.string(),
});

```

**In `ui/src/lib/app-bridge.ts`, replace:**

```ts
/// Hands the framed page `frame` its port and answers its calls: `tools` are those it may call
/// (read at each call), `call` performs one as the person.
export function connectFrame(
```

**with:**

```ts
/// Hands the framed page `frame` its port and answers its calls: `tools` are those it may call
/// (read at each call), `call` performs one as the person, and `openRun` opens a run the app
/// started (a page without it, a preview's, is refused).
export function connectFrame(
```

**In `ui/src/lib/app-bridge.ts`, replace:**

```ts
    call: (tool: string, input: unknown) => Promise<unknown>;
  },
```

**with:**

```ts
    call: (tool: string, input: unknown) => Promise<unknown>;
    openRun?: (runId: string) => Promise<void>;
  },
```

**In `ui/src/lib/app-bridge.ts`, replace:**

```ts
  };
  port.onmessage = (event: MessageEvent) => {
    const parsed = Call.safeParse(event.data);
```

**with:**

```ts
  };
  /// Answers the request `id` with what `work` gives, or its error.
  const answer = (id: number, work: () => Promise<unknown> | unknown) =>
    // Work that throws at once, or answers without a promise, is answered like any failure.
    Promise.resolve()
      .then(work)
      .then(
        (result) => send({ type: "result", id, ok: true, result }),
        (e: unknown) => send({ type: "result", id, ok: false, error: errorMessage(e) }),
      );
  port.onmessage = (event: MessageEvent) => {
    const open = OpenRun.safeParse(event.data);
    if (open.success) {
      const { id, run_id } = open.data;
      answer(id, async () => {
        if (!options.openRun) throw new Error("this screen cannot open runs");
        await options.openRun(run_id);
        return null;
      });
      return;
    }
    const parsed = Call.safeParse(event.data);
```

**In `ui/src/lib/app-bridge.ts`, replace:**

```ts
    }
    // A call that throws at once, or answers without a promise, is answered like any failure.
    Promise.resolve()
      .then(() => options.call(tool, input ?? {}))
      .then(
        (result) => send({ type: "result", id, ok: true, result }),
        (e: unknown) => send({ type: "result", id, ok: false, error: errorMessage(e) }),
      );
  };
```

**with:**

```ts
    }
    answer(id, () => options.call(tool, input ?? {}));
  };
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  appsKept: ["apps", "kept"] as const,
  limits: ["limits"] as const,
```

**with:**

```ts
  appsKept: ["apps", "kept"] as const,
  /// The apps that come with Dots and are not installed ("Install Projects").
  appsBundled: ["apps", "bundled"] as const,
  limits: ["limits"] as const,
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";

```

**with:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
import { commands } from "@/api/commands";
import { AppFrame } from "@/components/AppFrame";
```

**with:**

```tsx
import { commands } from "@/api/commands";
import type { AppConfig, AppRow } from "@/api/types";
import { AppFrame } from "@/components/AppFrame";
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
import { Page } from "@/components/Page";
import { appSrc } from "@/lib/app-bridge";
import { keys } from "@/lib/query";
import { appDataChanges } from "@/lib/runtime-events";
```

**with:**

```tsx
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { appSrc } from "@/lib/app-bridge";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { appDataChanges } from "@/lib/runtime-events";
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx

/// `/apps/:name` (app library spec section 10): the app's screen filling the content area. Its
/// calls go to the app as the person.
export function AppScreen() {
```

**with:**

```tsx

/// Whether to ask the person, once, to let a bundled app start dots (app library spec section 8):
/// it asks for the grant, the switch is off, and the person has not answered or set it.
export const asksToStart = (row: AppRow, config: AppConfig) =>
  config.origin.kind === "bundled" &&
  row.permissions.includes("start_dots") &&
  !config.can_start_dots &&
  !config.start_asked;

/// The one-time prompt above a bundled app's screen: "Switch on" or "Not now".
function StartPrompt({ name, title }: { name: string; title: string }) {
  const qc = useQueryClient();
  const answer = useMutation({
    mutationFn: (on: boolean) => commands.appAnswerStartPrompt(name, on),
    onSuccess: () => invalidate(qc, keys.apps),
    onError: notifyError,
  });
  return (
    <div
      role="region"
      aria-label={`Let ${title} start dots?`}
      className="flex flex-wrap items-center gap-3 border-b border-needs-border bg-needs-surface px-4 py-2 text-sm"
    >
      <p className="flex-1">
        Let {title} start dots? When you assign a task to a dot, {title} starts it on the task.
        You can change this on the Apps screen.
      </p>
      <Button size="sm" disabled={answer.isPending} onClick={() => answer.mutate(true)}>
        Switch on
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={answer.isPending}
        onClick={() => answer.mutate(false)}
      >
        Not now
      </Button>
    </div>
  );
}

/// `/apps/:name` (app library spec section 10): the app's screen filling the content area. Its
/// calls go to the app as the person, and a run it started opens in the dot's conversation.
export function AppScreen() {
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
  const name = useParams().name ?? "";
  const apps = useQuery({ queryKey: keys.apps, queryFn: commands.appsList });
```

**with:**

```tsx
  const name = useParams().name ?? "";
  const navigate = useNavigate();
  const apps = useQuery({ queryKey: keys.apps, queryFn: commands.appsList });
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
  }
  return (
```

**with:**

```tsx
  }
  const openRun = async (runId: string) => {
    const link = await commands.appRunLink(name, runId);
    navigate(`/home/${encodeURIComponent(link.dot_id)}/${encodeURIComponent(link.root_run_id)}`);
  };
  return (
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
    <div className="flex h-full flex-col">
      <AppFrame
```

**with:**

```tsx
    <div className="flex h-full flex-col">
      {asksToStart(row, config) && <StartPrompt name={name} title={config.title} />}
      <AppFrame
```

**In `ui/src/screens/apps/AppScreen.tsx`, replace:**

```tsx
        call={(tool, input) => commands.appCall(name, tool, input)}
        changes={changes}
```

**with:**

```tsx
        call={(tool, input) => commands.appCall(name, tool, input)}
        openRun={openRun}
        changes={changes}
```

**In `ui/src/screens/apps/AppsScreen.tsx`, replace:**

```tsx
import { commands } from "@/api/commands";
import type { AppConfig, AppRow, KeptData } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**with:**

```tsx
import { commands } from "@/api/commands";
import type { AppConfig, AppRow, BundledOffer, KeptData } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**In `ui/src/screens/apps/AppsScreen.tsx`, replace:**

```tsx
/// The Apps screen (app library spec section 10): the installed apps, each opening its screen,
/// with its switches, roll back and remove; then the data removed apps kept.
export function AppsScreen() {
```

**with:**

```tsx
/// The Apps screen (app library spec section 10): the installed apps, each opening its screen,
/// with its switches, roll back and remove; the apps that come with Dots and were removed, to
/// install again; then the data removed apps kept.
export function AppsScreen() {
```

**In `ui/src/screens/apps/AppsScreen.tsx`, replace:**

```tsx
  const kept = useQuery({ queryKey: keys.appsKept, queryFn: commands.appsKeptList });
  if (apps.isPending) return <Page title="Apps" />;
```

**with:**

```tsx
  const kept = useQuery({ queryKey: keys.appsKept, queryFn: commands.appsKeptList });
  const bundled = useQuery({ queryKey: keys.appsBundled, queryFn: commands.appsBundledOffers });
  if (apps.isPending) return <Page title="Apps" />;
```

**In `ui/src/screens/apps/AppsScreen.tsx`, replace:**

```tsx
        </ul>
      )}
```

**with:**

```tsx
        </ul>
      )}
      {bundled.isError && <ErrorNote error={bundled.error} />}
      {(bundled.data ?? []).length > 0 && (
        <section aria-labelledby="apps-bundled" className="flex flex-col gap-2">
          <h2 id="apps-bundled" className="font-semibold">
            Comes with Dots
          </h2>
          <ul className="flex flex-col gap-2">
            {bundled.data!.map((offer) => (
              <BundledItem key={offer.name} offer={offer} />
            ))}
          </ul>
        </section>
      )}
```

**In `ui/src/screens/apps/AppsScreen.tsx`, replace:**

```tsx

function KeptItem({ data }: { data: KeptData }) {
```

**with:**

```tsx

/// An app that comes with Dots and was removed: "Install <title>", on the data its removal kept
/// when that fits (ticked, and the person may untick it to start fresh).
function BundledItem({ offer }: { offer: BundledOffer }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [onKept, setOnKept] = useState(true);
  const install = useMutation({
    mutationFn: () =>
      commands.appInstallBundled(offer.name, onKept ? (offer.kept?.id ?? null) : null),
    onSuccess: () => {
      invalidate(qc, keys.apps);
      invalidate(qc, keys.library);
      invalidateDots(qc);
    },
    onError: notifyError,
    onSettled: () => setOpen(false),
  });
  const kept = offer.kept;
  return (
    <ItemRow
      label={offer.title}
      actions={
        <Button size="sm" onClick={() => setOpen(true)}>
          Install {offer.title}
        </Button>
      }
    >
      <span className="font-medium">{offer.title}</span>
      <span className="text-xs text-muted-foreground">{offer.description}</span>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Install {offer.title}?</AlertDialogTitle>
            <AlertDialogDescription>
              {offer.title} comes with Dots, for every dot. Dots updates it with each release
              until you accept a dot's change to it.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {kept && (
            <label className="flex items-start gap-2 text-sm">
              <input
                type="checkbox"
                className="mt-1"
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
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={install.isPending}
              onClick={(e) => {
                e.preventDefault();
                install.mutate();
              }}
            >
              Install
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ItemRow>
  );
}

function KeptItem({ data }: { data: KeptData }) {
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/app-client.test.ts src/components/AppFrame.test.tsx src/screens/apps` in `ui`, then `cargo test -j 2 -p dots-app app_frame`.
Expected: PASS: 3 files, 28 tests; 8 Rust tests.

- [ ] **Step 5: Run the checks**

Run: `bun run --cwd ui typecheck`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`.
Expected: clean; the UI 861 tests in 85 files.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/app_client.js ui/src/lib/app-bridge.ts ui/src/lib/app-client.test.ts ui/src/lib/query.ts ui/src/components/AppFrame.tsx ui/src/components/AppFrame.test.tsx ui/src/screens/apps/AppScreen.tsx ui/src/screens/apps/AppsScreen.tsx ui/src/screens/apps/AppsScreen.test.tsx
git commit -m "feat(ui): an app's screen opens its runs; Projects asks once to start dots and installs again"
```

### Task 8: Plan B's deferred findings

The three the person folded in besides the UI tests of Task 7: a preview in use stays open (the one used least recently closes) and a card whose preview closed says so and starts it again; `/preview/<id>` needs its final slash and an id; `frame-leaks.mjs` stops waiting after 10 s.

**Files:**

- Modify: `crates/dots-core/src/apps/preview.rs`, `apps/dots-app/src/app_frame.rs`, `ui/src/components/AppReviewCard.tsx`, `tools/smoke/frame-leaks.mjs`
- Test: `crates/dots-core/tests/apps_preview_test.rs`, `ui/src/components/AppReviewCard.test.tsx` (and `app_frame.rs`'s own tests)

**Interfaces:**

- Consumes: plan B's `Previews`, `MAX_PREVIEWS = 8`, `app_frame::parse`, the card's `Preview`.
- Produces: `Previews::find` makes the preview found the most recently used; `parse("/preview/<id>")` and `parse("/preview//x")` are `None`; the card's alert "Dots closed this preview to open another card's …" with "Start the preview again".

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/apps_preview_test.rs`, replace:**

```rust
    assert!(gone.is_err(), "{gone:?}");
}
```

**with:**

```rust
    assert!(gone.is_err(), "{gone:?}");
}

#[tokio::test]
async fn opening_one_preview_too_many_closes_the_one_used_least_recently() {
    let (dir, store, apps) = fixture().await;
    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let mut open = Vec::new();
    for _ in 0..MAX_PREVIEWS {
        open.push(apps.preview_open(&p.id).await.unwrap());
    }
    // The first card is still in use: a call and a file keep its preview.
    apps.preview_call(&open[0], "list_expenses", json!({ "month": "2026-10" }))
        .await
        .unwrap();
    apps.preview_file(&open[1], "index.html").unwrap();
    let newest = apps.preview_open(&p.id).await.unwrap();
    assert!(apps.preview_file(&open[0], "index.html").is_ok());
    assert!(apps.preview_file(&open[1], "index.html").is_ok());
    assert!(matches!(
        apps.preview_file(&open[2], "index.html"),
        Err(Error::NotFound(_))
    ));
    assert!(apps.preview_file(&newest, "index.html").is_ok());
}
```

**In `ui/src/components/AppReviewCard.test.tsx`, replace:**

```tsx
      expect(backend.callsTo("app_preview_close")).toEqual([{ preview: "01J0PREV" }]),
    );
```

**with:**

```tsx
      expect(backend.callsTo("app_preview_close")).toEqual([{ preview: "01J0PREV" }]),
    );
  });

  it("says so when Dots closed the preview for another card's, and starts it again", async () => {
    let opened = 0;
    backend.on("app_preview_open", () => `01J0PREV${(opened += 1)}`);
    backend.on("app_preview_close", () => null);
    backend.on("app_preview_call", () => {
      throw { kind: "not_found", message: "preview 01J0PREV1" };
    });
    const { user } = show(fresh);
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses preview");
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    port.postMessage({ type: "call", id: 1, tool: "add_expense", input: { item: "Tea" } });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Dots closed this preview to open another card's",
    );
    await user.click(screen.getByRole("button", { name: "Start the preview again" }));
    expect(await screen.findByTitle("Expenses preview")).toHaveAttribute(
      "src",
      "http://dots-app.localhost/preview/01J0PREV2/index.html",
    );
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -j 2 -p dots-core --test apps_preview_test`, `cargo test -j 2 -p dots-app app_frame`, and `DEBUG_PRINT_LIMIT=0 bun x vitest run src/components/AppReviewCard.test.tsx` in `ui`.
Expected: FAIL: `opening_one_preview_too_many_closes_the_one_used_least_recently` (the first preview is gone); `a_path_names_an_app_version_or_a_preview_and_a_file_below_ui` (`/preview/01J0PREV` parses); the card shows no alert.

- [ ] **Step 3: Fix the three**

**In `apps/dots-app/src/app_frame.rs`, replace:**

```rust
        None => {
            let (id, file) = file.split_once('/').unwrap_or((file, ""));
            (Some(id.to_string()), file)
```

**with:**

```rust
        None => {
            // As for an app version, a slash must follow the preview's id: `/preview/<id>` has
            // no folder of its own.
            let (id, file) = file.split_once('/')?;
            if id.is_empty() {
                return None;
            }
            (Some(id.to_string()), file)
```

**In `apps/dots-app/src/app_frame.rs`, replace:**

```rust
        );
        for refused in [
```

**with:**

```rust
        );
        assert_eq!(
            parse("/preview/01J0PREV/"),
            Some(Target::Preview {
                id: "01J0PREV".into(),
                path: "index.html".into()
            })
        );
        for refused in [
```

**In `apps/dots-app/src/app_frame.rs`, replace:**

```rust
            "/preview/",
            "/other/x",
```

**with:**

```rust
            "/preview/",
            "/preview/01J0PREV",
            "/preview//index.html",
            "/other/x",
```

**In `crates/dots-core/src/apps/preview.rs`, replace:**

```rust

/// The most previews open at once; opening one more closes the oldest.
pub const MAX_PREVIEWS: usize = 8;
```

**with:**

```rust

/// The most previews open at once; opening one more closes the one used least recently.
pub const MAX_PREVIEWS: usize = 8;
```

**In `crates/dots-core/src/apps/preview.rs`, replace:**

```rust

/// The open previews, oldest first.
#[derive(Clone, Default)]
```

**with:**

```rust

/// The open previews, the one used least recently first.
#[derive(Clone, Default)]
```

**In `crates/dots-core/src/apps/preview.rs`, replace:**

```rust
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
```

**with:**

```rust
impl Previews {
    /// The open preview `id`, which becomes the one used most recently: a card the person is
    /// using keeps its preview while others open.
    fn find(&self, id: &str) -> Result<Arc<Preview>> {
        let mut open = self.0.lock().expect("previews");
        let at = open
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| Error::NotFound(format!("preview {id}")))?;
        let preview = open.remove(at);
        open.push(preview.clone());
        Ok(preview)
    }
```

**In `tools/smoke/frame-leaks.mjs`, replace:**

```js
// them with browser arguments (`browser_args` in apps/dots-app/src/app_frame.rs). Each probe
// waits 6 s. Exit 1 when any probe got a hit, 2 when a probe did not run, 0 otherwise.
//
```

**with:**

```js
// them with browser arguments (`browser_args` in apps/dots-app/src/app_frame.rs). Each probe
// waits 6 s. A target that is missing or does not answer within 10 s counts as a probe that did
// not run. Exit 1 when any probe got a hit, 2 when a probe did not run, 0 otherwise.
//
```

**In `tools/smoke/frame-leaks.mjs`, replace:**

```js
];
const t = (await (await fetch("http://127.0.0.1:9333/json/list")).json()).find((x) => x.type === type);
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const evaluate = (expression) => new Promise((r) => {
  const me = ++id;
```

**with:**

```js
];
const ANSWER_MS = 10000;
/// `promise`, or `fallback` once ANSWER_MS have passed.
const within = (promise, fallback) =>
  Promise.race([promise, new Promise((r) => setTimeout(() => r(fallback), ANSWER_MS))]);
const t = (await (await fetch("http://127.0.0.1:9333/json/list")).json()).find((x) => x.type === type);
if (!t) {
  console.log(`no ${type} target on port 9333`);
  process.exit(2);
}
const ws = new WebSocket(t.webSocketDebuggerUrl);
if ((await within(new Promise((r) => ws.addEventListener("open", () => r("open"))), "late")) !== "open") {
  console.log(`${type}: DevTools did not answer in ${ANSWER_MS / 1000} s`);
  process.exit(2);
}
let id = 0;
const evaluate = (expression) => within(new Promise((r) => {
  const me = ++id;
```

**In `tools/smoke/frame-leaks.mjs`, replace:**

```js
  ws.send(JSON.stringify({ id: me, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
});
const out = [];
```

**with:**

```js
  ws.send(JSON.stringify({ id: me, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
}), `no answer in ${ANSWER_MS / 1000} s`);
const out = [];
```

**In `ui/src/components/AppReviewCard.tsx`, replace:**

```tsx

import { commands } from "@/api/commands";
import type { AppFile, AppProposal, ProposalDecision, Run } from "@/api/types";
```

**with:**

```tsx

import { CommandError, commands } from "@/api/commands";
import type { AppFile, AppProposal, ProposalDecision, Run } from "@/api/types";
```

**In `ui/src/components/AppReviewCard.tsx`, replace:**

```tsx
  const [changes, setChanges] = useState(0);
  useEffect(() => {
```

**with:**

```tsx
  const [changes, setChanges] = useState(0);
  // Dots keeps 8 previews open and closed this one for another card's: the person starts it
  // again (`fresh` counts the starts).
  const [closed, setClosed] = useState(false);
  const [fresh, setFresh] = useState(0);
  useEffect(() => {
```

**In `ui/src/components/AppReviewCard.tsx`, replace:**

```tsx
    };
  }, [proposal.id]);
  if (opened === null) return <p className="text-muted-foreground">Starting the preview…</p>;
```

**with:**

```tsx
    };
  }, [proposal.id, fresh]);
  if (closed) {
    return (
      <div role="alert" className="flex flex-col items-start gap-2 text-sm">
        <p>
          Dots closed this preview to open another card's: it keeps 8 open at a time. Your changes
          in it are gone.
        </p>
        <Button
          size="sm"
          onClick={() => {
            setClosed(false);
            setOpened(null);
            setChanges(0);
            setFresh((n) => n + 1);
          }}
        >
          Start the preview again
        </Button>
      </div>
    );
  }
  if (opened === null) return <p className="text-muted-foreground">Starting the preview…</p>;
```

**In `ui/src/components/AppReviewCard.tsx`, replace:**

```tsx
          call={(tool, input) =>
            commands.appPreviewCall(preview, tool, input).then((result) => {
              if (proposal.summary.tools.some((t) => t.name === tool && t.writes)) {
                setChanges((n) => n + 1);
              }
              return result;
            })
          }
```

**with:**

```tsx
          call={(tool, input) =>
            commands.appPreviewCall(preview, tool, input).then(
              (result) => {
                if (proposal.summary.tools.some((t) => t.name === tool && t.writes)) {
                  setChanges((n) => n + 1);
                }
                return result;
              },
              (error: unknown) => {
                if (error instanceof CommandError && error.kind === "not_found") setClosed(true);
                throw error;
              },
            )
          }
```

- [ ] **Step 4: Run the task's tests**

Run: the three commands of Step 2, and `node --check tools/smoke/frame-leaks.mjs`.
Expected: PASS: 5 core tests, 8 Rust tests, 11 UI tests; the script parses.

- [ ] **Step 5: Run the checks**

Run: `cargo test -j 2 -p dots-core --no-fail-fast`, `cargo test -j 2 -p dots-app`, `cargo clippy -j 2 --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: `dots-core` 1701, `dots-app` 174, the UI 862 tests in 85 files; no new warnings.

- [ ] **Step 6: Commit**

```bash
git add apps/dots-app/src/app_frame.rs crates/dots-core/src/apps/preview.rs crates/dots-core/tests/apps_preview_test.rs tools/smoke/frame-leaks.mjs ui/src/components/AppReviewCard.test.tsx ui/src/components/AppReviewCard.tsx
git commit -m "fix: a preview in use stays open, a preview's page needs its folder, the leak check stops waiting"
```

### Task 9: Projects, checked in WebView2

A real debug Dots, beside the installed one: Projects installed at first start, the prompt, the screen walked in its sandboxed frame (`tools/smoke/projects.mjs`), a fake-claude dot given a task, "Open the run", and the leak check on Projects' frame. Any failure stops the plan.

**Files:**

- Create: `tools/smoke/projects.mjs`

- [ ] **Step 1: Add the walk**

**Create `tools/smoke/projects.mjs`:**

```js
// Projects' screen, walked in its sandboxed frame as a person would (app library spec section 8):
// a project, a section, a task added inline, its side pane, its spec in Markdown, a subtask,
// its Activity and an Undo, with Dots' theme on the page. Prints one line per check and exits 1
// when any failed, 2 when the frame is missing or does not answer, 0 otherwise.
//
// Needs Node 22 or later (the global WebSocket). Start a debug Dots with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS holding `--remote-debugging-port=<port>` plus the
// release `browser_args(false)` (apps/dots-app/src/app_frame.rs), open `apps/projects`, then:
//
//   node tools/smoke/projects.mjs <port>                     the walk
//   node tools/smoke/projects.mjs <port> open-run "<task>"   opens the task, assigned to a dot,
//                                                            from Assigned to dots, and clicks
//                                                            its first "Open the run"
const port = Number(process.argv[2]);
const [mode, task] = [process.argv[3] ?? "walk", process.argv[4] ?? ""];
const ANSWER_MS = 30000;

const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const frame = targets.find((t) => t.type === "iframe" && t.url.includes("/app/projects/"));
if (!frame) {
  console.log("no Projects frame: open apps/projects first");
  process.exit(2);
}
const ws = new WebSocket(frame.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));

/// Runs `expression` (an async function's body) in the frame and gives what it returns.
function run(expression) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ error: `no answer in ${ANSWER_MS / 1000} s` }), ANSWER_MS);
    ws.addEventListener("message", function on(m) {
      const d = JSON.parse(m.data);
      if (d.id !== 1) return;
      clearTimeout(timer);
      ws.removeEventListener("message", on);
      resolve(d.result?.result?.value ?? { error: d.result?.exceptionDetails?.exception?.description });
    });
    ws.send(
      JSON.stringify({
        id: 1,
        method: "Runtime.evaluate",
        params: { expression: `(async () => { ${expression} })()`, awaitPromise: true, returnByValue: true },
      }),
    );
  });
}

// The walk, in the page: each check is [name, passed, detail].
const walk = `
  const checks = [];
  const check = (name, ok, detail = "") => checks.push([name, Boolean(ok), String(detail)]);
  const $ = (sel) => document.querySelector(sel);
  const until = async (find, ms = 5000) => {
    for (let t = 0; t < ms; t += 100) {
      const got = find();
      if (got) return got;
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  };
  const enter = (sel, text) => {
    const input = $(sel);
    input.value = text;
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  };
  const button = (text) => [...document.querySelectorAll("button")].find((b) => b.textContent === text);
  const name = "Smoke " + Math.floor(performance.now());

  check("theme", getComputedStyle(document.documentElement).getPropertyValue("--dots-primary"),
    document.documentElement.dataset.theme);
  check("left column", await until(() => button("My tasks") && button("Assigned to dots")));
  enter('[aria-label="New project"]', name);
  check("new project opens", await until(() => $("h1")?.textContent === name), $("h1")?.textContent);
  enter('[aria-label="New section"]', "To do");
  check("section added", await until(() => $('[aria-label="Add a task to To do"]')));
  enter('[aria-label="Add a task to To do"]', "Write the spec");
  const row = await until(() => $('[role="row"][aria-label="Write the spec"]'));
  check("task added inline", row);
  check("the cursor stays in the add row",
    await until(() => document.activeElement?.getAttribute("aria-label") === "Add a task to To do"));
  row?.querySelector(".title").click();
  const pane = await until(() => $('aside[aria-label="Task"]'));
  check("side pane", pane);
  button("Edit")?.click();
  const box = await until(() => $('textarea[aria-label="Spec"]'));
  box.value = "## Goal\\nShip **it**\\n\\n- [x] one\\n\\n<img src=x onerror=alert(1)>";
  button("Save").click();
  check("spec as Markdown", await until(() => $(".markdown h5") && $(".markdown strong")),
    $(".markdown")?.innerHTML);
  check("spec HTML stays text", !$(".markdown img"));
  enter('[aria-label="New subtask"]', "Tests");
  check("subtask added", await until(() => [...document.querySelectorAll(".subtasks li")]
    .some((li) => li.textContent.includes("Tests"))));
  const items = () => [...document.querySelectorAll('ol[aria-label="Activity"] li')];
  check("activity rows", await until(() => items().length >= 3), items().map((li) => li.textContent).join(" | "));
  const spec = items().find((li) => li.textContent.includes("changed the spec"));
  [...(spec?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "Undo")?.click();
  check("undo", await until(() => items().some((li) => li.textContent.includes("(undone)"))),
    items().map((li) => li.textContent).join(" | "));
  check("undo put the spec back", await until(() => $(".markdown")?.textContent.includes("No spec yet")));
  return checks;
`;

// Opens a run from the task's Activity: Dots then shows the dot's conversation.
const openRun = `
  const checks = [];
  const check = (name, ok, detail = "") => checks.push([name, Boolean(ok), String(detail)]);
  const until = async (find, ms = 5000) => {
    for (let t = 0; t < ms; t += 100) {
      const got = find();
      if (got) return got;
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  };
  const button = (text, root = document) =>
    [...root.querySelectorAll("button")].find((b) => b.textContent === text);
  button("Assigned to dots").click();
  const title = await until(() => button(${JSON.stringify(task)}));
  check("the task is assigned to a dot", title);
  title?.click();
  const open = await until(() => button("Open the run"));
  check("its Activity links the run", open);
  open?.click();
  return checks;
`;

const result = await run(mode === "open-run" ? openRun : walk);
ws.close();
if (!Array.isArray(result)) {
  console.log("the walk did not finish:", result?.error ?? result);
  process.exit(2);
}
let failed = false;
for (const [name, ok, detail] of result) {
  if (!ok) failed = true;
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `: ${detail.slice(0, 160)}` : ""}`);
}
setTimeout(() => process.exit(failed ? 1 : 0), 100);
```

- [ ] **Step 2: Build a debug Dots and the fake CLI**

The installed Dots may be running (`%LOCALAPPDATA%\Dots\dots-app.exe`): it is never stopped. It holds port 47321 and WebView2's shared data folder, so the debug Dots gets its own of both below.

Run: `bun x vite build` in `ui`, then from the repository root `bun run tauri build --debug --no-bundle` and `cargo build -j 2 -p dots-core --bin fake-claude`.
Expected: `target/debug/dots-app.exe` and `target/debug/fake-claude.exe` are built.

- [ ] **Step 3: Start it on a temp data folder**

`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` replaces the app's own WebView2 arguments, so it repeats `browser_args(false)` of `apps/dots-app/src/app_frame.rs` after the debugging port.

```bash
S="$(cygpath -m "$(mktemp -d)")"; mkdir -p "$S/data" "$S/work" "$S/bin"
cp "$CARGO_TARGET_DIR/debug/fake-claude.exe" "$S/bin/fake-claude.exe"
echo '[]' > "$S/script.json"
ARGS='--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --webrtc-ip-handling-policy=disable_non_proxied_udp --host-resolver-rules="MAP * ~NOTFOUND"'
DOTS_DATA_DIR="$S/data" WEBVIEW2_USER_DATA_FOLDER="$S/webview" \
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333 $ARGS" \
  CLAUDE_CODE_FAKE_SCRIPT="$S/script.json" DOTS_CONFIRM=yes \
  "$CARGO_TARGET_DIR/debug/dots-app.exe" > "$S/app.log" 2>&1 &
W="node tools/smoke/webview.mjs 9333"
```

Once `$W eval "typeof invoke"` answers `"function"`, point it at the fake CLI and another port, and restart it:

```bash
$W eval "(async () => { const s = await invoke('get_settings'); s.claude_path = '$S/bin/fake-claude.exe'; s.port = 47399; await invoke('update_settings', { settings: s }); return 'saved'; })()"
$W eval "invoke('restart_app')"
```

Expected: "saved"; then "the page closed before it answered" (the restart). Wait until `$W eval "invoke('status').then(s => s.port)"` answers `47399`. Then:

```bash
$W eval "(async () => (await invoke('apps_list')).map(r => [r.item.name, r.item.config.origin, r.item.config.every_dot, r.item.config.can_start_dots, r.item.config.start_asked, r.error]))()"
```

Expected: `projects`, `{ "kind": "bundled", "release": "0.1.0" }`, `true`, `false`, `false`, `null`.

- [ ] **Step 4: Answer the prompt and walk the screen**

```bash
$W goto apps/projects
$W wait "Let Projects start dots?" 20
$W click "Switch on"
$W eval "(async () => (await invoke('apps_list')).map(r => [r.item.config.can_start_dots, r.item.config.start_asked]))()"
node tools/smoke/projects.mjs 9333
```

Expected: `[true, true]`; every line of the walk `ok` (theme, left column, new project opens, section added, task added inline, the cursor stays in the add row, side pane, spec as Markdown, spec HTML stays text, subtask added, activity rows, undo, undo put the spec back), exit 0.

- [ ] **Step 5: Give a task to a fake-claude dot, and open its run**

```bash
TASK=$($W eval "(async () => (await invoke('app_call', { name: 'projects', tool: 'create_task', input: { title: 'Signup form', spec: 'Email and password.' } })).id)()" | tr -d '"')
cat > "$S/script.json" <<JSON
[
  {"emit": {"type": "system", "subtype": "init", "session_id": "\$SESSION"}},
  {"mcp_call": {"server": "dots", "tool": "projects__get_task", "arguments": {"id": "$TASK"}}},
  {"mcp_call": {"server": "dots", "tool": "projects__comment", "arguments": {"task_id": "$TASK", "text": "Form built."}}},
  {"mcp_call": {"server": "dots", "tool": "projects__update_task", "arguments": {"id": "$TASK", "status": "in_review"}}},
  {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "Built the signup form.", "usage": {"input_tokens": 1, "output_tokens": 1}}}
]
JSON
$W eval "(async () => { const spec = await invoke('new_dot_spec'); spec.name = 'builder'; spec.instructions = 'Build what Projects gives you.'; spec.engine = 'claude'; spec.workdir = '$S/work'; spec.workspace_mode = 'folder'; return (await invoke('create_dot', { spec, orgId: null })).id; })()"
$W eval "(async () => (await invoke('app_call', { name: 'projects', tool: 'update_task', input: { id: '$TASK', assignee: 'builder' } })).task.status)()"
sleep 12
$W eval "(async () => { const t = await invoke('app_call', { name: 'projects', tool: 'get_task', input: { id: '$TASK' } }); return [t.status, t.activity.map(r => [r.kind, r.field, r.actor.name, r.text])]; })()"
$W goto apps/projects
node tools/smoke/projects.mjs 9333 open-run "Signup form"
$W text | head -12
```

Expected: `"in_progress"` at the assignment; then `in_review` with the rows `created`, `assignee`, `status` (You), `run started` (builder), `comment` "Form built." (builder), `status` (builder), `run ended` "Built the signup form." (builder); the open-run checks `ok`; the visible text is builder's conversation with its "Projects" topic and "Built the signup form.".

- [ ] **Step 6: Check Projects' frame for leaks, stop the debug Dots, and commit**

```bash
$W goto apps/projects
node tools/smoke/frame-leaks.mjs iframe 47500
node tools/smoke/frame-leaks.mjs worker 47500
```

Expected: the six probes `ran, 0 … hit(s)`, exit 0; then "no worker target on port 9333", exit 2.

Stop only the debug build, by its path: `powershell -NoProfile -Command "Get-Process dots-app | Where-Object { \$_.Path -like '*\target\debug\dots-app.exe' } | Stop-Process"`. Then:

```bash
git add tools/smoke/projects.mjs
git commit -m "test(smoke): walk Projects' screen in its frame and open a dot's run from it"
```

### Task 10: The spec says what plan C built

**Files:**

- Modify: `docs/superpowers/specs/2026-10-23-app-library-design.md`

- [ ] **Step 1: Add §18 and point §15's last line at it**

**In `docs/superpowers/specs/2026-10-23-app-library-design.md`, replace:**

```markdown
- **The migration is 0016.** The env vault's orgs (0013), the watchers (0014) and the teams (0015) reached `master` first, so the app library's tables, its `upgrade_0016` list and `finish_0016` come after them. None of the four touches a table another does, and the app rules come after the `Workflow` rule that 0015 adds, in the presets as in a migrated policy.
- **Left to plan B:** the Apps screen and its frame, the bridge, the review card with its preview (calls on a throwaway database built from the migrations and `sample.sql`) and diff, the editor's Apps checklist, the needs-you count for waiting cards, and how `ui/` files reach the frame. Plan B also gives dots a way to read an installed app's files (a tool, under `ProposeApp`, that copies the active version, as written, into the run's workspace): a change must keep the active version's migrations byte for byte, and no dot but the one whose workspace still holds its folder can see them, so without it a dot cannot change Projects or another dot's app. The core gives the screen its commands: `apps_list`, `app_call` (as the person), `app_files`, `app_calls`, the switches, roll back, remove, `app_proposals_list`, `app_proposal_files` and `app_proposal_decide`, and the event `app_proposal_changed`. **Left to plan C:** Projects, bundled.

```

**with:**

```markdown
- **The migration is 0016.** The env vault's orgs (0013), the watchers (0014) and the teams (0015) reached `master` first, so the app library's tables, its `upgrade_0016` list and `finish_0016` come after them. None of the four touches a table another does, and the app rules come after the `Workflow` rule that 0015 adds, in the presets as in a migrated policy.
- **Left to plan B:** the Apps screen and its frame, the bridge, the review card with its preview (calls on a throwaway database built from the migrations and `sample.sql`) and diff, the editor's Apps checklist, the needs-you count for waiting cards, and how `ui/` files reach the frame. Plan B also gives dots a way to read an installed app's files (a tool, under `ProposeApp`, that copies the active version, as written, into the run's workspace): a change must keep the active version's migrations byte for byte, and no dot but the one whose workspace still holds its folder can see them, so without it a dot cannot change Projects or another dot's app. The core gives the screen its commands: `apps_list`, `app_call` (as the person), `app_files`, `app_calls`, the switches, roll back, remove, `app_proposals_list`, `app_proposal_files` and `app_proposal_decide`, and the event `app_proposal_changed`. **Left to plan C:** Projects, bundled (built by plan C: §18).

```

**In `docs/superpowers/specs/2026-10-23-app-library-design.md`, replace:**

```markdown
- **Bounds.** The call log keeps at most 30 KB of a call's result, as of its input. An app's test files run 60 s together at most; a file past that fails without running. Accepting an app lets go of the thread lock while its migrations run and marks the thread as deciding meanwhile: a chat or a reply to that thread is refused ("Dots is still adding the app you accepted in this chat; send your message again in a moment"), a follow-up waits, and other threads go on. Removing an app deletes its topics, and topics of apps no longer installed are pruned at start.
```

**with:**

```markdown
- **Bounds.** The call log keeps at most 30 KB of a call's result, as of its input. An app's test files run 60 s together at most; a file past that fails without running. Accepting an app lets go of the thread lock while its migrations run and marks the thread as deciding meanwhile: a chat or a reply to that thread is refused ("Dots is still adding the app you accepted in this chat; send your message again in a moment"), a follow-up waits, and other threads go on. Removing an app deletes its topics, and topics of apps no longer installed are pruned at start.

## 18. Changes made while building (plan C)

Plan C built Projects, bundled; these are the places where it says more than the sections above, or differs from them.

- **How Projects ships.** Its files live in `crates/dots-core/apps/projects/` and are built into Dots (`apps::bundled`), read and checked as a proposal's folder is. They are LF everywhere (`.gitattributes`), so the migrations' bytes are the same in every build; a test checks that the built-in list names every file of the folder. Its tests run with Dots' own (`apps_bundled_test`).
- **Installed once, for every dot.** Dots installs Projects the first time it starts with it, with "Every dot" on and "Can start dots" off, and notes it in `<data>/apps/bundled.json`. A Projects the person removed is not put back; an app a dot made under the name `projects` is left alone. A test runtime installs it only when its configuration asks (`Config.bundled_apps`); the app always does.
- **Updated while bundled.** At start, a bundled Projects the person has not changed is updated when this release's files differ from the active version's and the release differs from the one recorded on its row. The new version must keep the migrations that ran, byte for byte, as a dot's change must; one that does not is refused, logged, and changes nothing. A roll back keeps the release, so the update it left is not applied again; a build of the same release (a developer's) does not update it.
- **"Install Projects".** The Apps screen lists Projects under "Comes with Dots" while it is not installed. The kept data of a removed Projects fits by the rule of §17's kept data (the migrations that ran on it begin the bundled ones, byte for byte); when it fits, the dialog offers "Start on the data kept from the removed Projects", ticked, and the person may untick it to start fresh, which leaves the kept data listed.
- **The one-time prompt.** Above the screen of a bundled app that asks for `start_dots` while the switch is off, a banner asks "Let Projects start dots?" with "Switch on" and "Not now". Either answer, or setting the switch on the Apps screen, ends it (the row's `start_asked`); installing Projects again asks again.
- **A run's link.** The page's `dots` gains `openRun(run_id)`. Dots opens the dot's conversation at the run's topic only when the run belongs to a topic this app started (`app_run_link`); any other run, and every run asked from a preview, is refused.
- **Projects' data and tools.** Statuses are `to_do`, `in_progress`, `in_review` and `done`. Besides the tools of §8 there are `rename_section`, `move_task` (a section, and the task to stand before), `list_dots` (the dots a task can be given to) and `undo`; `set_subtask` also renames. `list_tasks` takes `view: "my_tasks"` (the person's tasks and unassigned to-dos) or `"assigned_to_dots"`, returns at most `limit` tasks (100 unless told; 1000 at most) and says whether there are `more`; `get_task` takes how many Activity rows to give (20 unless told). A change row's field is `title`, `spec`, `status`, `assignee`, `due_on`, `section` or `subtask:<id>`, a task's creation is a row of its own, and a run row says `started`, `not_started` or `ended`. Reordering within a section is not a change of the task and writes no row.
- **Undo** (the person's choice): the person undoes any change, theirs included; a dot undoes only dots' changes. A change is undone only while its field holds what it set, an undone change is not undone twice, a dot's undo never brings back Done, and an undo starts no dot even when it gives the task back to one. A task's creation, comments and runs have no undo.
- **Assigning to a dot.** No dot starts when a dot takes a task itself or the task is Done. A start the rules refuse (the hour's 20, paused schedules) is written in Activity as `not_started`, and the task waits. Giving the task to the same dot again while its topic is open writes no second start. When a run ends, Activity gets its summary or its error; the task goes In review only while it is still that dot's and not Done, whatever the run's status.
- **The screen** is `ui/index.html`, `main.js`, `projects.js`, `markdown.js` and `projects.css`, in Dots' colours through the `--dots-<token>` variables, tested in Vitest from `ui/src/apps/`. Markdown is a small renderer of Projects' own: every character is escaped first, then headings, paragraphs, lists with checkboxes, quotes, code, bold and italic; a link shows its text, with the address as a title. Dragging works in a project's list, onto a task (before it) or onto a section (at its end).
- **Plan B's deferred findings, folded in** (the person's choice): a preview in use stays open, and opening a ninth closes the one used least recently; a card whose preview was closed says so and starts it again. `/preview/<id>` without a final slash, or with an empty id, is refused, as an app version's is. `tools/smoke/frame-leaks.mjs` stops after 10 s when its target is missing or does not answer.
- **Checked in WebView2** (`tools/smoke/projects.mjs`): on a debug build, Projects installed at first start, the prompt switched starting dots on, the screen's walk passed, a fake-claude dot given a task read it, commented and left it In review with its summary, and "Open the run" opened its conversation; the leak check found nothing from Projects' frame.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-10-23-app-library-design.md
git commit -m "docs: the spec says what plan C built"
```
