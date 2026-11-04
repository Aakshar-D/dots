# Setup agent and easy first dot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A person describes a task in plain words, the setup agent proposes one dot on a card, and **Run setup** creates it and starts its first chat; the manual form gets easier (spaces in names, blank folder, no worktree-on-non-repo, folded rules).

**Architecture:** A new core module `setup.rs` builds the prompt, checks Claude's JSON answer and turns a proposal into a `DotSpec`. A `SetupDrafts` runner copies `SkillDrafts` (one-shot `claude`, cancel by id). `Runtime` gains `setup_ask`, `setup_cancel`, `setup_apply`, and `create_dot`/`update_dot` gain the folder and mode rules. Three Tauri commands expose it; one React component `SetupFlow` is used on the empty home and in a New dot dialog.

**Tech Stack:** Rust (dots-core, tokio, serde_json, chrono), Tauri 2 commands, React + TypeScript + zod + TanStack Query, vitest.

**Spec:** `docs/superpowers/specs/2026-11-03-setup-agent-design.md`

## Global Constraints

- Work only in the worktree `C:\dev\dots-setup-agent`, branch `feat/setup-agent`. Cargo's target dir comes from `.cargo/config.toml` (`C:/dev/dots-target`); do not change it.
- Never add a Claude/AI co-author trailer to commits.
- Name rule: 1 to 64 characters after trimming; letters, digits, spaces, `-`, `_`, `.`; no leading or trailing space; no two spaces in a row. Error text: `Use 1 to 64 letters, digits, spaces, '-', '_' or '.'.`
- `folder_name(name)`: lower case, each run of non-ASCII-alphanumeric characters becomes one `-`, trimmed of `-`, cut to 40 characters (then trimmed of `-` again); empty becomes `dot`.
- Default folder: `<new_dot_root>\<folder_name>`, with `new_dot_root` = `%USERPROFILE%\Dots` in the app; `-2`, `-3`, ... appended while the folder exists and is not empty.
- Setup limits: description and change request 1 to 4000 characters; 1 to 3 questions, each at most 300 characters; instructions cut at 4000 characters; time limit 5 minutes (reuse `skill_creator::DRAFT_LIMIT`); model `sonnet` (reuse `skill_creator::DEFAULT_MODEL`).
- Extras are exactly `web` (adds `WebFetch` allow and `WebSearch` allow) and `mcp:<server>` (adds `mcp__<server>__*` allow). Any extra makes `policy.preset` `custom`.
- Copy, verbatim: "What should your first dot do?", placeholder "For example: every weekday at 9, read my Downloads folder and tell me what's new", buttons "Set it up", "Fill in the form instead", "Continue", "Skip, just propose", "Run setup", "Ask for changes", "Start over"; failure "Setup could not finish. Try describing the task again, or fill in the form."; unreadable answer "The setup agent's answer could not be read. Try again."; folder hint "Leave blank and Dots makes a new folder for this dot."; create toast addition "Runs in the folder itself, because it is not a git repository."; update refusal "This folder is not a git repository. Choose 'The folder itself' or pick a git folder."; extras lines "Read websites and search the web without asking" and "Use <server> without asking"; preset sentences: read-only "May read files. Never writes, runs commands, reads websites or uses MCP servers.", sandboxed "May edit files in its folder and run a few safe git commands. Asks before anything else.", trusted "May run commands without asking."; no-schedule text "Only when you talk to it".

## Spec deviations (decided while planning)

- The prompt gives the local UTC offset (`chrono::Local::now().format("%:z")`), not a zone name: core has no time-zone-name crate and the offset is enough to read "at 9".
- **When it runs** on the card embeds the existing `ScheduleEditor` component instead of a sentence plus **Change**: the UI has no schedule-in-words helper, and the editor is already compact.
- The banner gets no separate **Dismiss**: its existing "Mark as tested" already hides it until the version changes, which is what Dismiss would do. It is hidden when the person has no dots.

## Review Focus

- A description in another language or with Windows paths (`C:\Users\x\My Files`): the prompt carries it unchanged and a named folder with spaces is used as-is. Test in Task 3.
- Claude answering with prose around the JSON, or a fenced block: fence is stripped; any text outside one JSON object is "could not be read". Test in Task 3.
- A dot name made only of punctuation or non-ASCII letters ("Résumé bot", "日本"): `folder_name` falls back sensibly (`r-sum-bot`, `dot`) and the name rule rejects only what it should. Test in Task 1.
- Default folder whose `Dots` parent does not exist yet (fresh machine): it is created with `create_dir_all`. Test in Task 2.
- Run setup pressed twice quickly: the second call meets the name clash error, never two dots. Test in Task 4 (`apply_twice_same_name_conflicts`) and the button is disabled while pending (Task 6).

---

### Task 1: Names with spaces and `folder_name`

**Files:**
- Modify: `crates/dots-core/src/model.rs` (`DotSpec::validate`, add `pub fn folder_name`)
- Modify: `crates/dots-core/src/workspace.rs:~324` (branch name)
- Modify: `crates/dots-core/src/store/dots.rs` (`create_dot`, `update_dot`: case-insensitive clash)
- Modify: `ui/src/lib/dot-rules.ts:16`
- Test: `crates/dots-core/tests/dot_name_test.rs` (new), `ui/src/screens/DotEditorScreen.test.tsx:46`

**Interfaces:**
- Produces: `pub fn folder_name(name: &str) -> String` in `dots_core::model`; `Error::Conflict(format!("a dot named {name} already exists"))` (existing text from `map_unique`) for case-insensitive clashes.

- [ ] **Step 1: Write failing core tests** in `tests/dot_name_test.rs`:
  - `validate` accepts `"Downloads digest"`, `"v1.2 bot"`, `"a"`, a 64-char name; rejects `""`, `" lead"`, `"trail "`, `"two  spaces"`, `"slash/no"`, a 65-char name, `"Résumé"` (non-ASCII letter), each with `Error::Invalid` whose text contains `"Use 1 to 64 letters, digits, spaces"`.
  - `folder_name`: `"Downloads digest"` → `"downloads-digest"`; `"  --A__B..c "` → `"a-b-c"`; `"Résumé bot"` → `"r-sum-bot"`; `"日本"` → `"dot"`; a 64-char `"a b ..."` name → at most 40 chars, not ending in `-`.
  - Store: creating `"Foo"` then `"foo"` → `Conflict`; with rows `"Bar"` and `"bar"` inserted directly by SQL, updating `"bar"`'s instructions succeeds; renaming `"bar"` to `"BAR"` → `Conflict`.
  - Workspace: a worktree run of a dot named `"My Bot"` in a temp git repo gets `run.branch == Some("dots/my-bot/<run_id>")` (use the existing worktree test helpers in `tests/workspace_test.rs` as the model).
- [ ] **Step 2:** `cargo test -p dots-core --test dot_name_test` → FAIL (spaces rejected, `folder_name` missing).
- [ ] **Step 3: Implement.** Name rule in `validate` per Global Constraints. `folder_name` per Global Constraints. Branch `format!("dots/{}/{}", folder_name(&dot.spec.name), run_id)`. In the store, before insert (create) and before update when the trimmed name differs from the stored one, `SELECT 1 FROM dots WHERE name = ? COLLATE NOCASE AND id != ?` → `Conflict`. Grep `spec.name` / `dot.name` across `crates/` and `apps/` for any other non-display use (paths, branch names, tool or env names, CLI args) and route it through `folder_name`; leave logs and display text.
- [ ] **Step 4: UI rule.** `dot-rules.ts:16` regex `/^(?! )(?!.*  )(?!.* $)[A-Za-z0-9 ._-]{1,64}$/` with the Global Constraints error text; update the assertion at `DotEditorScreen.test.tsx:46` and add one accepting `"Test dot"`.
- [ ] **Step 5:** `cargo test -p dots-core --test dot_name_test --test workspace_test` and `cd ui && bun run test -- DotEditorScreen` → PASS.
- [ ] **Step 6: Commit** `feat(core): dot names may hold spaces and dots`

### Task 2: Blank folder and git-aware mode at create and update

**Files:**
- Modify: `crates/dots-core/src/runtime.rs` (`Config` gains `pub new_dot_root: PathBuf`; `create_dot`, `update_dot`)
- Modify: `apps/dots-app/src/state.rs` (or wherever `Config::new` is built for the app) to set `new_dot_root` from `%USERPROFILE%\Dots`
- Modify: `ui/src/screens/dot-editor/DotForm.tsx` (folder hint), `ui/src/screens/DotEditorScreen.tsx` (`NewDot` toast)
- Test: `crates/dots-core/tests/dot_folder_test.rs` (new), `ui/src/screens/DotEditorScreen.test.tsx`

**Interfaces:**
- Consumes: `folder_name` (Task 1).
- Produces: `pub async fn default_folder(root: &Path, name: &str) -> Result<PathBuf>` in `runtime.rs` (or a small `crate::setup_folder` helper): makes and returns the first free `<root>\<folder_name>[-N]`. `pub async fn is_git_repo(path: &Path) -> bool` (move the `rev-parse` check from `teams::check_repo`, which then calls it). `Config::new` default for `new_dot_root`: `data_dir.join("new-dots")` (tests never touch the home folder).

- [ ] **Step 1: Write failing tests** in `dot_folder_test.rs`:
  - `blank_workdir_gets_default_folder`: create with `workdir = ""`, name `"Inbox helper"` → stored workdir `<root>/inbox-helper`, folder exists; root not existing beforehand is created.
  - `default_folder_skips_non_empty`: `<root>/inbox-helper` exists with a file → `<root>/inbox-helper-2`; an existing empty `<root>/inbox-helper` is reused.
  - `worktree_on_plain_folder_becomes_folder_at_create`: mode worktree, workdir a temp non-repo → stored mode `folder`.
  - `worktree_on_repo_stays`: temp dir with `git init` → mode stays `worktree`.
  - `update_refuses_worktree_on_plain_folder`: update to worktree on a non-repo → `Error::Invalid` with the Global Constraints refusal text.
  - `update_blank_workdir_refused`: update with `workdir = ""` → `Invalid("workdir must not be empty")`.
- [ ] **Step 2:** `cargo test -p dots-core --test dot_folder_test` → FAIL.
- [ ] **Step 3: Implement** in `Runtime::create_dot` before the store call: blank (trimmed) workdir → `default_folder(&cfg.new_dot_root, &spec.name)`; then if mode is worktree and `!is_git_repo(workdir)` → set folder. In `update_dot`: if mode is worktree and the workdir is not a repo → refusal. If create fails after making a default folder, remove it when still empty.
- [ ] **Step 4: UI.** Folder field hint under `dot-workdir` = Global Constraints hint. `specErrors` no longer flags a blank workdir for a new dot (it still does for an existing one). `NewDot.onSave`: when `spec.workspace_mode === "worktree"` and the returned `dot.spec.workspace_mode === "folder"`, toast `"Dot created. Runs in the folder itself, because it is not a git repository."`, else `"Dot created"`. Tests: blank folder saves; toast text with a fake backend returning folder mode.
- [ ] **Step 5:** run the core test file and `bun run test -- DotEditorScreen` → PASS; `cargo test -p dots-core --test teams_test` still PASS.
- [ ] **Step 6: Commit** `feat(core): a new dot may leave its folder blank, and never gets worktree mode outside git`

### Task 3: `setup.rs`: prompt, answer check, extras

**Files:**
- Create: `crates/dots-core/src/setup.rs`; register `pub mod setup;` in `lib.rs`
- Test: `crates/dots-core/tests/setup_check_test.rs` (new)

**Interfaces:**
- Consumes: `skill_creator::{unwrap_fence, MAX_REQUEST_CHARS}`, `policy::{Policy, Preset, Rule, Action}`, `schedule::Schedule`, `library::Uses`.
- Produces (all `Serialize, Deserialize, Clone, Debug, PartialEq`):

```rust
pub struct SetupRound { pub questions: Vec<String>, pub answers: Vec<String> }
pub struct SetupRequest {
    pub description: String,
    pub rounds: Vec<SetupRound>,          // question rounds before the first proposal
    pub current: Option<SetupProposal>,   // set for "Ask for changes"
    pub change: Option<String>,
    pub skip: bool,                       // "Skip, just propose"
}
pub struct SetupProposal {
    pub name: String, pub instructions: String, pub folder: Option<String>,
    pub schedule: Option<Schedule>, pub preset: Preset, #[serde(default)] pub extras: Vec<String>,
    #[serde(default)] pub uses: Uses, #[serde(default)] pub first_message: String,
    #[serde(default)] pub notes: String,
}
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupAnswer { Questions { questions: Vec<String> }, Proposal { proposal: SetupProposal, new_dot_root: String } }
pub struct SetupLibrary { pub skills: Vec<(String, String)>, pub mcp_servers: Vec<String>, pub plugins: Vec<String>, pub context: Vec<String> }
pub struct SetupContext { pub today: String /* "2026-11-03 (Tuesday)" */, pub utc_offset: String, pub home: String, pub new_dot_root: String, pub library: SetupLibrary, pub dot_names: Vec<String> }

pub const SETUP_INSTRUCTIONS: &str;          // the system prompt, spec §6 and §6.3
pub fn must_propose(req: &SetupRequest) -> bool;   // skip || current.is_some() || rounds.len() >= 2
pub fn setup_prompt(ctx: &SetupContext, req: &SetupRequest) -> Result<String>;
pub fn check_setup_answer(text: &str, ctx: &SetupContext, req: &SetupRequest) -> Result<SetupAnswer>;
pub fn check_proposal(p: SetupProposal, ctx: &SetupContext, rename_on_clash: bool) -> Result<SetupProposal>;
pub fn setup_policy(preset: Preset, extras: &[String]) -> Policy;
```

- [ ] **Step 1: Write failing tests** in `setup_check_test.rs` (a `ctx()` helper with library skill `pdf-tools` and server `gmail`, dot names `["Inbox helper"]`, home `C:\Users\pat`):
  - `prompt_carries_context_and_rounds`: contains `"2026-11-03 (Tuesday)"`, the offset, `C:\Users\pat`, `pdf-tools`, `gmail`, `Inbox helper`, the description verbatim (use `"Résumé der Dateien in C:\Users\pat\My Files"`), each question and answer; no must-propose line with 0 or 1 rounds.
  - `prompt_must_propose`: with 2 rounds, with `skip`, and with `current`+`change` the prompt contains `"Answer with a proposal now"`; the change request and the current proposal JSON are included.
  - `prompt_rejects`: empty description → `Invalid`; 4001-char description or change → `Invalid`; `change` without `current` → `Invalid`.
  - `answer_questions`: `{"questions":["Which folder?"]}` → `Questions`; inside a ```json fence too; 0 or 4 questions, empty or 301-char question → `Other("The setup agent's answer could not be read. Try again.")`.
  - `answer_unreadable`: prose, `"Sure! {...}"`, both keys, neither key → the unreadable error.
  - `questions_when_must_propose`: questions answer with `skip` → `Other("Setup could not finish. Try describing the task again, or fill in the form.")`.
  - `proposal_fixups`: preset `"admin"` → `sandboxed` + a note; uses skill `nope` dropped + note containing `"nope"`; extra `mcp:slack` without `slack` in uses dropped; `mcp:gmail` kept when `gmail` in uses; extra `"root"` dropped; schedule `{"rule":{"kind":"every","minutes":0}}` dropped + note; name `"inbox helper"` → `"inbox helper 2"` when `rename_on_clash`, `Conflict` when not; 5000-char instructions cut to 4000 + note; folder `C:\does\not\exist` → `None` + note; blank `first_message` → description.
  - `policy_extras`: `setup_policy(ReadOnly, &[])` == `Policy::preset(ReadOnly)`; `setup_policy(ReadOnly, &["web"])` has preset `Custom`, resolves `WebFetch` and `WebSearch` to `Allow` and `Bash` to `Deny`; `mcp:gmail` resolves `mcp__gmail__send` to `Allow`.
- [ ] **Step 2:** `cargo test -p dots-core --test setup_check_test` → FAIL (module missing).
- [ ] **Step 3: Implement** `setup.rs`. Parse with `serde_json::from_str::<serde_json::Value>(unwrap_fence(text).trim())`; the top-level must be an object with exactly one of `questions` / `proposal`. Unknown preset: parse `preset` as a string first, map with `Preset`'s `str_enum` parse, `custom` and unknown → `sandboxed`. Must-propose line text: `"Answer with a proposal now. Do not ask questions."`. The system prompt states the rules in spec §6 (job, what a dot is, how to write instructions, JSON only, guessing rules, the three preset sentences from Global Constraints, the two extras and when to add them, the `Schedule` JSON shapes from `schedule.rs`).
- [ ] **Step 4:** `cargo test -p dots-core --test setup_check_test` → PASS.
- [ ] **Step 5: Commit** `feat(core): the setup agent's prompt and the check of its answer`

### Task 4: Runner and `Runtime::setup_ask / setup_cancel / setup_apply`

**Files:**
- Modify: `crates/dots-core/src/setup.rs` (add `SetupDrafts`)
- Modify: `crates/dots-core/src/runtime.rs` (field, constructor next to `skill_drafts` at ~460, methods)
- Test: `crates/dots-core/tests/setup_test.rs` (new; model: `tests/skill_draft_test.rs`)

**Interfaces:**
- Consumes: Task 3 types; `Runtime::create_dot`, `Runtime::chat`, `library().list()`, `list_dots()`; Task 2 `default_folder`, `is_git_repo`.
- Produces:

```rust
pub struct SetupDrafts { /* same fields as SkillDrafts */ }
impl SetupDrafts {
    pub async fn ask(&self, id: &str, ctx: &SetupContext, req: &SetupRequest) -> Result<SetupAnswer>;
    pub fn cancel(&self, id: &str);
}
pub struct SetupApplied { pub dot: Dot, pub run_id: Option<String>, pub run_error: Option<String> }
impl Runtime {
    pub async fn setup_context(&self) -> Result<SetupContext>;
    pub async fn setup_ask(&self, id: &str, req: SetupRequest) -> Result<SetupAnswer>;
    pub fn setup_cancel(&self, id: &str);
    pub async fn setup_apply(&self, p: SetupProposal, org: Option<String>, workspace: Option<String>) -> Result<SetupApplied>;
}
```

- [ ] **Step 1: Write failing tests** in `setup_test.rs` with fake-claude (`FAKE_CLAUDE_SCRIPT`, `FAKE_CLAUDE_ARGS_OUT`, `answering(text)` as in `skill_draft_test.rs`):
  - `ask_returns_questions` and `ask_returns_checked_proposal` (a missing skill dropped with a note).
  - `ask_sends_context`: args-out stdin contains the Library skill name and an existing dot name; args contain `--max-turns 1` and `--tools ""`.
  - `ask_cancel`: a script that sleeps 10 s; `cancel(id)` → error text `"The setup was cancelled."`; a second `ask` with the same id cancels the first.
  - `ask_times_out` with a 200 ms limit → `"Claude took too long to set up the dot."`.
  - `ask_without_claude` (`claude_path: None`) → the skill creator's "claude CLI not found" text.
  - `apply_creates_dot_and_chat`: proposal with `folder: None`, preset read-only, extras `["web"]` → dot exists, workdir `<new_dot_root>/<folder_name>`, mode `folder`, `policy.preset == custom`, a chat run queued whose payload message is `first_message`; `run_id` set.
  - `apply_git_folder_gets_worktree`: folder is a temp `git init` repo → mode worktree.
  - `apply_twice_same_name_conflicts`: second apply → `Conflict`, still one dot, no second folder left behind.
  - `apply_create_fails_removes_folder`: an `org` that does not exist → error, default folder removed.
- [ ] **Step 2:** `cargo test -p dots-core --test setup_test` → FAIL.
- [ ] **Step 3: Implement.** `SetupDrafts::ask` mirrors `SkillDrafts::draft` with `SETUP_INSTRUCTIONS`, `DEFAULT_MODEL`, error texts: cancelled `"The setup was cancelled."`, timed out `"Claude took too long to set up the dot."`, failed `"Claude could not set up the dot: {e}"`; on success `check_setup_answer`. `setup_context`: today/offset from `chrono::Local`, home from `USERPROFILE` (fallback `HOME`, then empty), `new_dot_root` from `Config`, library from `library().list()` skipping `owner_id.is_some()` (skills with their description), dot names from `list_dots()`. `setup_apply`: `check_proposal(p, ctx, false)`; folder `None` → `default_folder`, else must be an existing dir (`Invalid` otherwise); mode by `is_git_repo`; `DotSpec::new(name, instructions, folder)` + mode, schedule, `setup_policy(preset, extras)`, uses; `create_dot` (on error remove a default folder made here if still empty); then `chat(&dot.id, first_message, None)` — its error goes into `run_error`, not the result.
- [ ] **Step 4:** `cargo test -p dots-core --test setup_test` → PASS.
- [ ] **Step 5: Commit** `feat(core): the setup agent asks Claude and creates the proposed dot`

### Task 5: Tauri commands, contract and UI API

**Files:**
- Modify: `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs` (registry), `apps/dots-app/src/contract.rs` (samples)
- Modify: `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/api/__fixtures__/contract.json` (regenerated)

**Interfaces:**
- Produces commands `setup_ask(id: String, request: SetupRequest) -> SetupAnswer`, `setup_cancel(id: String) -> ()`, `setup_apply(proposal: SetupProposal, org_id: Option<String>, workspace_id: Option<String>) -> SetupApplied` (argument names follow `create_dot`'s). UI: `commands.setupAsk(id, request)`, `commands.setupCancel(id)`, `commands.setupApply(proposal, orgId, workspaceId)`; zod `SetupRequest`, `SetupProposal`, `SetupAnswer` (discriminated on `kind`), `SetupApplied`.

- [ ] **Step 1:** Add samples to `contract.rs` `samples()`: `SetupRequest` (with and without `current`), `SetupAnswer` (both kinds), `SetupApplied` (with `run_error`). Register the commands.
- [ ] **Step 2:** `UPDATE_CONTRACT=1 cargo test -p dots-app contract` then `cargo test -p dots-app contract` → PASS.
- [ ] **Step 3:** Add zod types and commands; `cd ui && bun run test -- contract` → PASS; `bun run typecheck` → no errors.
- [ ] **Step 4: Commit** `feat(app): setup commands`

### Task 6: `SetupFlow` on the empty home and in the New dot dialog

**Files:**
- Create: `ui/src/screens/setup/SetupFlow.tsx`, `ui/src/screens/setup/SetupCard.tsx`, `ui/src/screens/setup/NewDotDialog.tsx`
- Modify: `ui/src/screens/home/HomeScreen.tsx` (`EmptyHome`), `ui/src/screens/home/DotsColumn.tsx:~70` (New dot opens the dialog)
- Test: `ui/src/screens/setup/SetupFlow.test.tsx` (new), update `HomeColumn.test.tsx:63,70-72,164`, `HomeRoutes.test.tsx:66`

**Interfaces:**
- Consumes: Task 5 commands; `useCurrentScope` (`ui/src/lib/scope.ts`) with the same org/workspace mapping as `DotEditorScreen.tsx` lines ~48-58; `ScheduleEditor`; the `Dialog` components `CreateSkillDialog` uses; `invalidateDots`.
- Produces: `export function SetupFlow(props: { onDone?: () => void })`; stages `"describe" | "asking" | "questions" | "card"`; setup id `setup-${++opened}` per mount.

- [ ] **Step 1: Write failing tests** in `SetupFlow.test.tsx` (fake backend, model: `LibraryScreen.test.tsx:438-483`):
  - empty home renders "What should your first dot do?", the placeholder, "Set it up" and a "Fill in the form instead" link to `/dots/new`.
  - questions round: `setup_ask` answers questions → each shows with a text box; "Continue" sends `rounds: [{questions, answers}]`; "Skip, just propose" sends `skip: true`.
  - card: shows name, notes, instructions, the preset sentence, the extra line "Read websites and search the web without asking" with a remove button, uses chips; removing `gmail` from uses also removes `mcp:gmail`.
  - "Ask for changes" sends `current` + `change` and replaces the card; an error keeps the old card and shows the error.
  - "Run setup" calls `setup_apply` with the edited proposal and the scope's org/workspace, is disabled while pending, navigates to `/home/<dot id>`; with `run_error` it toasts `"The dot was created but its first run did not start: <error>"`; an apply error stays on the card above the button.
  - unmount while asking calls `setup_cancel`; "Start over" asks to confirm when a card is shown.
  - DotsColumn: "New dot" is a button opening a dialog titled "New dot" holding the flow and the form link.
- [ ] **Step 2:** `bun run test -- SetupFlow HomeColumn HomeRoutes` → FAIL.
- [ ] **Step 3: Implement** the components with existing `Button`, `Input`, `Textarea`, `Field`, `Dialog`; request counter drops stale answers as in `CreateSkillDialog`. Folder row: the path, or `A new folder: <new_dot_root>\<folder name>` (folder name from a TS copy of `folder_name` in `dot-rules.ts`, root from the answer's `new_dot_root`); "Change" uses the dialog plugin's directory picker as the form's Browse does, plus "Use a new folder". When `setup_ask` fails and the status has no `claude.program`, show under the error: "Dots uses your Claude Code CLI. Install it and sign in, then try again." (test it).
- [ ] **Step 4:** tests → PASS; `bun run typecheck` → clean.
- [ ] **Step 5: Commit** `feat(ui): set up a dot by describing it`

### Task 7: Folded permission rules and a quiet first launch

**Files:**
- Modify: `ui/src/screens/dot-editor/PolicyEditor.tsx`, `ui/src/app/Banners.tsx:~322`
- Test: `ui/src/screens/dot-editor/PolicyEditor.test.tsx`, `ui/src/app/Layout.test.tsx:74`

- [ ] **Step 1: Write failing tests:** PolicyEditor with preset `sandboxed` shows the preset select and its one-line sentence, and no rule inputs until "Show rules" is clicked; with preset `custom` the rules are visible without a click. Layout: with `claude.mismatch` and an empty overview, no "has not been tested" text; with one dot, the banner shows (existing assertion).
- [ ] **Step 2:** `bun run test -- PolicyEditor Layout` → FAIL.
- [ ] **Step 3: Implement:** a `<details>`-style disclosure (or the existing collapsible component if one exists) around rules, default rule and shell grants, `open` initially when `policy.preset === "custom"`. Banner: render the mismatch banner only when `overview.data && overview.data.length > 0`.
- [ ] **Step 4:** tests → PASS; full `bun run test` → PASS.
- [ ] **Step 5: Commit** `feat(ui): fold the permission rules and keep a fresh install quiet`

### Task 8: Whole-branch verification

- [ ] **Step 1:** `cargo test -p dots-core` and `cargo test -p dots-app` → all PASS (note any test that also fails on `master` instead of fixing it here).
- [ ] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings` → clean; `cd ui && bun run typecheck && bun run test` → clean.
- [ ] **Step 3:** Run the dev app (`bun tauri dev` from the worktree root) with `DOTS_DATA_DIR` set to a fresh scratch folder, so the installed Dots data is untouched. On the empty home, describe "every weekday at 9, list the newest files in my Downloads folder and tell me about them", answer or skip questions, press Run setup, and confirm: the dot opens, its first run starts and succeeds, the folder rule and preset match the card.
- [ ] **Step 4:** Commit any fixes found; do not clean the cargo target dir (CLAUDE.local.md: only after merge).
