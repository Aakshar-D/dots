# Accounts phase 4: workspaces — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every account and Personal has workspaces; dots and teams live in one; workspace roles narrow what a member can do with that workspace's dots; a workspace's `use` rules narrow which entries its dots may use; the switcher picks a workspace; admins create, rename and delete workspaces and set members' workspace roles.

**Architecture:** A local `workspaces` table holds Personal workspaces (edited directly) and a mirror of each account's roster workspaces (rewritten on every accepted list). `dots.workspace_id` and `teams.workspace_id` place things. Permission checks that have a dot (or team) in hand go through new `accounts::{access_dot, require_dot}` which combine the account role with the workspace role. Account workspace changes are roster edits signed and published with the phase 3 machinery (`edit_members`).

**Tech Stack:** Rust (sqlx/SQLite), Tauri commands + contract samples, React + TanStack Query + zod + vitest.

**Spec:** `docs/superpowers/specs/2026-10-28-accounts-members-design.md` (§6.2 workspaces in the roster, §6.3 workspace roles, §8 Workspaces, §9 switcher and Workspaces tab, §13 phase 4).

Base branch: `feat/accounts-workspaces`, from `feat/accounts-invites` (8f3df91).

Rulings in this plan:
- Account workspaces are the roster's `workspaces`; creating, renaming, deleting them and setting a member's workspace roles are roster edits (`edit_members`), so they need an Admin/Owner whose Dots ID is in `admins`, exactly like member edits. Personal workspaces live only in the local table.
- Effective rights on an account dot: account Owner/Admin → their role in every workspace. Account Member → workspace `admin` or `member` gives Member rights, `viewer` gives Viewer rights, no entry gives no access. Account Viewer → Viewer where they have an entry, no access elsewhere. Legacy accounts and Personal: workspaces narrow nothing.
- "Does not see it" (§6.3): a workspace the member has no entry in is hidden from the switcher and its dots refuse everything except the phase 2 cleanup actions (delete, disable).
- Workspace filtering follows where account filtering exists today (Home's dot list, new-dot defaults). Runs and Usage weren't account-filtered in phase 1 and stay unfiltered; teams have no screen.
- A workspace's `use` editing is in `dots-admin` only; the app shows it read-only.
- Workspace ids come from names: lower-case, runs of anything outside `[a-z0-9]` become one `-`, trimmed of `-`, cut to 32, `workspace` if empty, then `-2`, `-3`… on collision; `default` is always taken.

## Global Constraints

- Never stage the root `Cargo.toml`. Never add a Co-Authored-By trailer.
- Every refusal is plain words naming the account (and workspace), what can't be done and who to ask, like phase 2's `"{account}: a {Role} can't {action}. Ask an Admin or Owner."`
- Outside-workspace refusal, exact: `"{account}: you aren't in the {workspace} workspace. Ask an Admin or Owner to add you."`
- Roles are enforced in `dots-core`, never only in the UI.
- Workspace ids match `^[a-z0-9][a-z0-9-]{0,31}$`; `default` always exists and can't be deleted (its name can change). Deleting a workspace moves its dots and teams to Default.
- A team's dots must all be in the team's account and workspace.
- A dot moves between workspaces of the same account only; it needs Member rights in both; `org_id` never changes.
- Workspace `use` narrows the policy's `use`: deny rules combine; a name must pass both allow lists.
- Usage limits stay per Claude subscription (no change).
- Migrations: next is `0023`. Never edit earlier ones.
- UI: zod `strictObject`; Deep Teal tokens and existing components; every disabled control says why. Contract fixture via `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- Gates per task: targeted `cargo test -p dots-core --test <file>`, `cargo clippy --workspace --tests -- -D warnings`, `cargo fmt --all`; UI tasks `cd ui && bun run test && bun run typecheck`. Known unrelated failure: `runtime_review_test::preview_schedule_lists_the_next_fire_times`.

## Review Focus

1. A list arrives without a workspace this PC has dots in: those dots and teams move to Default, nothing is lost, the current scope falls back to Default (Task 2 `vanished_workspace_moves_dots_to_default`).
2. An account Member with workspace `viewer` tries to run or edit a dot there, including a scheduled run: refused / held with the workspace text (Task 3 `workspace_viewer_cannot_run_and_schedules_hold`).
3. A dot in a workspace the person isn't in: everything refused but delete/disable; the switcher never offers it (Task 3 `outside_workspace_refuses_but_cleanup`, Task 8 test).
4. A workspace `use` that allows less than the policy: the dot can't read the entry, and the refusal names the workspace (Task 4 `workspace_use_narrows_policy`).
5. Moving a dot that's in a team, or to another account's workspace: refused in plain words (Task 6 `move_respects_team_and_account`).

---

### Task 1: Workspaces table and placement columns

**Files:**
- Create: `crates/dots-core/migrations/0023_workspaces.sql`, `crates/dots-core/src/store/workspaces.rs` (register in `store/mod.rs`)
- Modify: `crates/dots-core/src/model.rs` (`Dot.workspace_id`, `Team.workspace_id`, `TeamSpec.workspace_id`), `crates/dots-core/src/store/{dots.rs,teams.rs,orgs.rs}`, every `create_dot` caller (pass `"default"`)
- Test: `crates/dots-core/tests/store_workspaces_test.rs`

**Interfaces:**
- Migration:
  ```sql
  CREATE TABLE workspaces (scope TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL,
    use_rules TEXT, created_at TEXT NOT NULL, PRIMARY KEY (scope, id));
  -- a 'default' row for 'personal' and for every joined org (INSERT … SELECT from orgs)
  ALTER TABLE dots ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';
  ALTER TABLE teams ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';
  ```
- Produces:
  - `pub struct WorkspaceRow { pub scope: String, pub id: String, pub name: String, pub use_rules: Option<UseRules>, pub created_at: String }` (`scope` = `"personal"` or an org id).
  - `Store::list_workspaces(scope) -> Result<Vec<WorkspaceRow>>` (Default first, then by name), `Store::workspace(scope, id) -> Result<Option<WorkspaceRow>>`.
  - `Store::add_workspace(&WorkspaceRow)` (Conflict on a taken id), `Store::rename_workspace(scope, id, name)`, `Store::delete_workspace(scope, id)` — `default` → `Error::Invalid("The Default workspace can't be deleted.")`; moves the scope's dots (`org_id` match, `org_id IS NULL` for personal) and teams in it to `default`; one transaction.
  - `Store::mirror_workspaces(org_id, &[Workspace]) -> Result<Vec<String>>` — replaces the org's rows with the roster's (name, `use`), moves dots/teams of vanished workspaces to `default`, returns the vanished ids; one transaction.
  - `Store::remove_org` also deletes the org's workspace rows; `Store::add_org` adds its `default` row.
  - `Store::create_dot(spec, org_id, workspace_id: &str)`; `Store::set_dot_workspace(id, workspace_id)`; `Dot.workspace_id: String`; `Team.workspace_id: String`; `TeamSpec.workspace_id: String` (serde default `"default"`).
  - `pub fn workspace_id_for(name: &str, taken: &[String]) -> String` in `crates/dots-core/src/accounts/workspaces.rs` (create the module here; re-export from `accounts`).
- [ ] **Step 1: Failing tests:** personal default exists after migration; add/list/rename; delete moves dots and teams to default and refuses `default`; `mirror_workspaces` replaces rows and moves dots of a vanished workspace, returning its id; `remove_org` drops the org's rows; `workspace_id_for("Payments Team", &[])` → `"payments-team"`, with `["payments-team"]` taken → `"payments-team-2"`, `"Default"` → `"default-2"`, `"***"` → `"workspace"`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun `vault_orgs_store_test` and the teams/dots store tests.
- [ ] **Step 5: Commit** `feat(accounts): workspaces table, and dots and teams placed in one`

---

### Task 2: Mirror on fetch, current scope and visible workspaces

**Files:**
- Modify: `crates/dots-core/src/accounts/workspaces.rs`, `crates/dots-core/src/vault/org/service.rs` (call `mirror_workspaces` wherever a roster is accepted and recorded), `crates/dots-core/src/accounts/scope.rs`
- Test: `crates/dots-core/tests/accounts_workspaces_test.rs`

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Clone, Debug, PartialEq)] pub struct WorkspaceView { pub scope: String, pub id: String, pub name: String, pub use_rules: Option<UseRules>, pub my_role: Option<WorkspaceRole>, pub can_manage: bool }` — `my_role` is the member's entry (Owner/Admin → `Some(Admin)`; Personal/legacy → `None`); `can_manage` = Personal, or the account access allows `ManageWorkspaces`.
  - `pub async fn visible_workspaces(store: &Store, orgs: &Orgs, scope: &str) -> Result<Vec<WorkspaceView>>` — Personal and legacy accounts: all rows; active accounts: Owner/Admin all, others only those in `MemberRow.workspaces`; waiting/removed: none.
  - `set_current_scope` accepts any workspace in `visible_workspaces` for that account, else the existing `"no workspace '{id}' in this account"`. `current_scope` falls back to `default` when the stored workspace is no longer visible.
- [ ] **Step 1: Failing tests:** `accepted_list_mirrors_workspaces` (roster with `default` + `payments` → rows, `use` kept); `vanished_workspace_moves_dots_to_default` (dot in `payments`, next list drops it → dot in `default`; current scope `{acme, payments}` reads back `{acme, default}`); `member_sees_only_their_workspaces`; `admin_sees_all`; `scope_refuses_hidden_workspace`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun `accounts_membership_test`.
- [ ] **Step 5: Commit** `feat(accounts): mirror account workspaces and pick one as the current scope`

---

### Task 3: Workspace roles in permission checks

**Files:**
- Modify: `crates/dots-core/src/accounts/permissions.rs`, `crates/dots-core/src/runtime.rs` (`require_for_dot`, `require_run`, `require_for_run`, `create_dot`, `update_dot`, cleanup paths), `crates/dots-core/src/runner.rs` (`role_hold`, `require_run`), `crates/dots-core/src/vault/resolve.rs` (`before_run`), `crates/dots-core/src/apps/registry.rs:151`, `crates/dots-core/src/teams.rs`, `crates/dots-core/src/workflows.rs` (team checks use the team's workspace)
- Test: `crates/dots-core/tests/runtime_workspace_roles_test.rs`

**Interfaces:**
- Produces:
  - `Access` gains `OutsideWorkspace { workspace: String }` (the workspace's name): `allows` false for everything; refusal is the Global Constraints outside-workspace text.
  - `pub fn access_in(orgs: Option<&Orgs>, org_id: Option<&str>, workspace_id: &str) -> Access` — the plan's effective-rights rule on top of `access`. Reads the member's workspaces from the `Orgs` service (add an accessor if none exists).
  - `pub fn access_dot(orgs: Option<&Orgs>, dot: &Dot) -> Access`, `pub async fn require_dot(orgs: Option<&Orgs>, dot: &Dot, p: Permission) -> Result<()>`, `pub async fn require_team(orgs: Option<&Orgs>, team: &Team, p: Permission) -> Result<()>`.
  - Every call site that has a dot or team switches to these; `create_dot` checks the target workspace; cleanup (delete/disable) stays allowed for `OutsideWorkspace`, like waiting/removed.
  - `role_hold` uses `access_dot`; a background run held by a workspace role gets `"{account}: your role in the {workspace} workspace doesn't allow running dots"`.
- [ ] **Step 1: Failing tests:** `workspace_viewer_cannot_run_and_schedules_hold`; `workspace_member_can_edit_and_run`; `outside_workspace_refuses_but_cleanup` (run, edit, answer refused with the exact text; delete and disable allowed); `account_admin_is_admin_everywhere`; `account_viewer_stays_viewer_with_workspace_admin_entry`; `legacy_and_personal_unchanged`; `create_dot_in_a_workspace_needs_rights_there`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun the phase 2 role tests.
- [ ] **Step 5: Commit** `feat(accounts): workspace roles narrow what members can do with its dots`

---

### Task 4: Workspace `use` rules

**Files:**
- Modify: `crates/dots-core/src/vault/resolve.rs` (`org_value`, next to `policy.may_use`), `crates/dots-core/src/vault/org/policy.rs`
- Test: `crates/dots-core/tests/vault_workspace_use_test.rs`

**Interfaces:**
- Produces: `pub fn may_use_in(policy: &UseRules, workspace: Option<&UseRules>, name: &str) -> bool` — either deny refuses; allow must match both (no workspace `use` narrows nothing). `org_value` finds the dot's workspace row (`Store::workspace(org_id, &dot.workspace_id)`) and refuses with the existing `DeniedByPolicy` outcome; the text names the workspace: `"the {workspace} workspace doesn't allow {NAME}"`.
- [ ] **Step 1: Failing tests:** `workspace_use_narrows_policy` (policy `*`, workspace allows `STRIPE_*` → `STRIPE_KEY` ok, `API_KEY` refused with the workspace text); `workspace_deny_adds_to_policy_deny`; `no_workspace_use_changes_nothing`; `policy_deny_still_wins`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun the vault resolve tests.
- [ ] **Step 5: Commit** `feat(accounts): a workspace's use rules narrow the account's`

---

### Task 5: Workspace edits on the roster (core and dots-admin)

**Files:**
- Modify: `crates/dots-core/src/accounts/edit.rs`, `crates/dots-core/src/vault/org/members.rs` (permission per edit kind), `crates/dots-admin/src/roster.rs`
- Test: `crates/dots-core/tests/accounts_edit_test.rs`, `crates/dots-core/tests/accounts_members_service_test.rs`, `crates/dots-admin/tests/admin_test.rs`

**Interfaces:**
- Produces:
  - `RosterEdit` and `Change` gain: `AddWorkspace { name: String }` (id via `workspace_id_for`), `RenameWorkspace { id: String, name: String }`, `DeleteWorkspace { id: String }` (refuses `default`; removes the id from every member's `workspaces`), `SetWorkspaces { member_id: String, workspaces: BTreeMap<String, WorkspaceRole> }` (must include `default`; Owner rows locked as in phase 3).
  - Summaries: `"Add the {name} workspace"`, `"Rename {old} to {name}"`, `"Delete the {name} workspace"`, `"Change {member}'s workspaces"`.
  - `edit_members` requires `ManageWorkspaces` for the three workspace kinds and `ManageMembers` for `SetWorkspaces`; signer, seq, draft and publish handling unchanged.
  - dots-admin: `roster workspace add <file> <name> [--allow P]… [--deny P]…`, `roster workspace rename <file> <id> <name>`, `roster workspace remove <file> <id>`, `roster workspace use <file> <id> [--allow P]… [--deny P]…` (no flags clears `use`), `roster set-workspaces <file> <email> <id>=<role>…`. Unsigned file edits, no seq bump, like the other roster commands.
- [ ] **Step 1: Failing tests:** core `add_rename_delete_workspace`, `delete_default_is_refused`, `delete_removes_member_entries`, `set_workspaces_needs_default`, `owner_workspaces_locked`; service `member_cannot_add_workspace`, `admin_adds_workspace_and_it_mirrors` (publish → refresh → `list_workspaces` has it); dots-admin `workspace_add_with_use`, `workspace_remove_default_refused`, `set_workspaces_by_email`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(accounts): add, rename and delete workspaces, and set members' workspace roles`

---

### Task 6: Runtime: placing and moving dots, teams, Personal workspaces, Inbox

**Files:**
- Modify: `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/store/teams.rs` (`check_accounts` also checks workspace), `crates/dots-core/src/teams.rs`, `crates/dots-core/src/model.rs` (`Inbox.team_scope`)
- Test: `crates/dots-core/tests/runtime_workspaces_test.rs`

**Interfaces:**
- Produces:
  - `Runtime::create_dot(&self, spec: DotSpec, org_id: Option<String>, workspace_id: Option<String>)` — `None` → `"default"`; the workspace must exist in the scope.
  - `Runtime::move_dot(&self, dot_id: &str, workspace_id: &str) -> Result<Dot>` — same account only; `EditDots` in source and target (`access_in`); a dot in a team → `Error::Invalid("{dot} is in the team {team}. Take it out of the team first.")`; unknown workspace → `Error::NotFound("No workspace '{id}' in {account}.")`.
  - Teams: create/update require all member dots in the team's account and workspace: `"dot '{dot}' is in another workspace than the team"`. `TeamSpec.workspace_id` is fixed once created, like `org_id`.
  - Personal workspaces: `Runtime::{personal_workspace_add(name) -> Result<WorkspaceView>, personal_workspace_rename(id, name), personal_workspace_delete(id)}`; `Runtime::workspaces(scope) -> Result<Vec<WorkspaceView>>`.
  - `Inbox.team_scope: Vec<TeamScopeIssue { team_id, team_name, dot_id, dot_name, reason: String }>` — teams with a member dot outside the team's account or workspace (existing data); reason in plain words.
- [ ] **Step 1: Failing tests:** `create_dot_in_workspace`; `move_respects_team_and_account`; `move_needs_member_rights_in_both`; `team_members_share_workspace`; `personal_workspaces_add_rename_delete_moves_dots`; `inbox_lists_team_scope_issues`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun teams/workflows tests.
- [ ] **Step 5: Commit** `feat(accounts): create and move dots between workspaces; team workspace rules`

---

### Task 7: App commands and contract

**Files:**
- Modify: `apps/dots-app/src/{commands.rs,lib.rs,contract.rs}`, `ui/src/api/__fixtures__/contract.json`, `ui/src/api/{types.ts,commands.ts,contract.test.ts}`

**Interfaces:**
- Commands: `workspace_list(scope) -> Vec<WorkspaceView>`, `personal_workspace_add(name) -> WorkspaceView`, `personal_workspace_rename(id, name)`, `personal_workspace_delete(id)`, `dot_move(dot_id, workspace_id) -> Dot`; `create_dot` gains `workspace_id: Option<String>`; `member_edit` carries the new `RosterEdit` kinds.
- Samples: `WorkspaceView` (with and without `use_rules`), `Dot` and `Team` with `workspace_id`, `Inbox` with a `team_scope` item, the new `RosterEdit` kinds.
- [ ] **Step 1:** samples first → contract test FAIL. **Step 2:** implement; `UPDATE_CONTRACT=1 cargo test -p dots-app contract`; zod schemas + wrappers; `bun run test && bun run typecheck` PASS.
- [ ] **Step 3: Commit** `feat(accounts): app commands for workspaces`

---

### Task 8: UI — switcher workspace level, filtering, dot placement

**Files:**
- Modify: `ui/src/app/AccountSwitcher.tsx`, `ui/src/lib/scope.ts`, `ui/src/screens/HomeScreen.tsx`, `ui/src/screens/dot-editor/{DotOrg.tsx,DotEditorScreen.tsx}`, the Inbox / Needs attention view
- Test: the matching `*.test.tsx` / `scope.test.ts`

**Behaviour:**
- Switcher: under the selected account, its visible workspaces (`workspaceList(account)`), the current one checked; picking one calls `scopeSet(account, id)`. An account with only Default shows no workspace list. `useSwitchAccount` still lands on `"default"` when switching accounts.
- `inScope(dot, scope, orgs)` also compares `dot.workspace_id` with `scope.workspace`; Home filters by both.
- Dot editor: a Workspace select next to the account picker (the chosen account's visible workspaces; defaults to the current scope's). A new dot is created there. Changing it on an existing dot calls `dotMove` (account stays fixed); refusals shown in plain words.
- Needs attention shows `team_scope` items with their reason.
- [ ] **Step 1: Failing tests:** `switcher lists the account's workspaces and picks one`; `single Default workspace shows no list`; `inScope compares workspace`; `new dot uses the current workspace`; `moving a dot calls dot_move and shows refusals`; `team scope issue appears in Needs attention`.
- [ ] **Step 2–4:** FAIL, implement, PASS; full UI gates.
- [ ] **Step 5: Commit** `feat(accounts): pick a workspace in the switcher and place dots in one`

---

### Task 9: UI — Workspaces tab and members' workspace roles

**Files:**
- Create: `ui/src/screens/accounts/AccountWorkspaces.tsx`, `ui/src/screens/accounts/PersonalWorkspaces.tsx`
- Modify: `ui/src/screens/Accounts.tsx` (third tab **Workspaces** for active and legacy accounts; a Personal card with its workspaces), `ui/src/screens/accounts/AccountMembers.tsx` (per-member **Workspaces** editor)
- Test: `ui/src/screens/accounts/AccountWorkspaces.test.tsx`, `AccountMembers.test.tsx`

**Behaviour:**
- Account Workspaces tab: list (name, id, `use` read-only as allow/deny, a Default badge). With `can_manage` and the members view's `can_sign`: **Add workspace** (name), **Rename**, **Delete** (confirm: "Its dots on each PC move to Default."; Default has no Delete). All go through `memberEdit` with the members view's `seq`, reusing the Members tab's draft / conflict handling. Disabled controls carry the same reasons as the Members tab (`sign_blocker`, role text, waiting-draft text).
- Personal workspaces: same list with Add / Rename / Delete via the personal commands.
- Members tab: per-member **Workspaces** button opening a small editor (a checkbox and role select per workspace; Default always on) that sends `SetWorkspaces`; Owner rows locked with the Owner text.
- [ ] **Step 1: Failing tests:** `admin adds a workspace through a signed edit`; `delete asks to confirm and Default can't be deleted`; `member sees workspaces read-only`; `personal workspaces add rename delete`; `member workspace roles edit sends set_workspaces`; `owner workspaces locked`.
- [ ] **Step 2–4:** FAIL, implement, PASS; full UI gates.
- [ ] **Step 5: Commit** `feat(accounts): Workspaces tab and members' workspace roles`

---

### Task 10: Docs, smoke and release notes

**Files:**
- Modify: `docs/vault-admin.md` (workspaces in the roster, `roster workspace …` and `set-workspaces`, `use` narrowing, the workspace roles table), `docs/accounts.md` (switching workspaces, what a workspace viewer can do, moving dots), `release-notes.md`
- Create: `docs/verification/2026-10-30-accounts-phase4-smoke.md` (two PCs: add a workspace → appears on both; a member without an entry doesn't see it; a viewer can't run; `use` narrowing; delete moves dots to Default; Personal workspaces)
- [ ] **Step 1:** write; check every command, label and quoted text against the code.
- [ ] **Step 2: Commit** `docs(accounts): workspaces; phase 4 smoke`
