# Accounts phase 2: signed member list and roles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each account can carry a signed member list (the roster). Dots works out this PC's membership and role from it, and enforces the Owner/Admin/Member/Viewer permissions in core. The owner manages the roster with `dots-admin`.

**Architecture:**
- Policy v2 adds `roster` and `admins`.
- A new `dots_core::accounts::roster` module parses and verifies a minisign-signed roster in its own envelope.
- Every source also reads an optional `DOTS_ROSTER`.
- The `Orgs` service evaluates membership after every accepted fetch and keeps it in new `orgs` columns plus the offline copy.
- `accounts::permissions::can` decides each action. It is checked at the runtime entry points and at the runner's single start point. Background runs a role no longer allows are held with a reason instead of failing.

**Tech Stack:** Rust (sqlx/SQLite, minisign, minisign-verify), Tauri, React + zod, vitest.

**Spec:** `docs/superpowers/specs/2026-10-28-accounts-members-design.md` (§6.1–§6.4, §6.6, §11, §12, §13 item 2)

## Global Constraints

- Roles are `owner|admin|member|viewer`, shown as Owner, Admin, Member, Viewer. Workspace roles are `admin|member|viewer`.
- Roster v1 fields are `v`, `org_id`, `issued_at`, `seq`, `workspaces`, `members`.
- Member fields are `id`, `name`, `email`, `role`, `keys`, `workspaces`, `added_at`.
- Workspace fields are `id`, `name`, and optional `use {allow, deny}`.
- Unknown fields fail closed at every level.
- Validation rules (spec §6.2, verbatim):
  - `org_id` matches.
  - `seq` is greater than the last accepted.
  - Roles are valid.
  - Every key appears in at most one member.
  - Workspace ids match `^[a-z0-9][a-z0-9-]{0,31}$`, and `default` exists.
  - Every workspace a member names exists.
  - At least one `owner`.
- Trust (spec §6.1): a roster is valid if signed by the join code's root key (`policy_key`) or by a key in the current policy's `admins`. The signer check happens when a roster is accepted. Removing a key from `admins` later never invalidates an already accepted roster.
- Policy v1 still means `roster: "off"`, so legacy orgs behave exactly as today. Policy v2 adds `"roster": "off" | "required"` (default `"off"`) and `"admins": [...]` (default `[]`). A v1 policy containing either field is refused.
- Rosters are read from:
  - OpenBao: key `DOTS_ROSTER` at `<mount>/data/<path>/roster`.
  - Infisical: secret `DOTS_ROSTER` in `<secret path>/policy`.
  - SOPS: top-level field `roster`.
- Membership values are `legacy|waiting|active|removed` (spec §6.4). Only a PC that was `active` can become `removed`. `removed` wipes like `Denied` does today.
- Personal is always Owner. A `legacy` account means Member, and `waiting`/`removed` mean no permissions.
- The permission table is spec §6.3. Workspace roles are parsed and validated but **not enforced until phase 4**.
- The refusal error is `Error::NotAllowed(String)`, formatted as `"{account}: a {Role} can't {action}. Ask an Admin or Owner."`. Waiting uses `"{account}: you aren't on its member list yet. Send your Dots ID to an Admin."`. Removed uses `"{account}: you were removed from it."`.
- Commit messages carry no AI co-author trailer. Never stage the root `Cargo.toml`.

**Rulings carried from the spec into this plan:**
- **Request codes and Request access move to phase 3**, together with invite and request codes. In phase 2 a waiting member uses **Copy my Dots ID** (built in phase 1), and the owner adds them with `dots-admin roster add --key`. `roster add-request` arrives in phase 3.
- **Disabled controls** in phase 2 cover only: the new-dot button and the editor's Save for a Viewer, the Inbox release for role holds, and the approval buttons. The remaining "every disabled control names the role" work comes with the Members page in phase 3.

## Review Focus

- **Upgrade from legacy to roster:** when the owner publishes a v2 policy with `roster: "required"` before this PC is on the list, the PC goes to `waiting`, never `removed`, and nothing is wiped. Tested in Task 4.
- **Bad or missing roster after one was accepted:** the PC keeps the last roster and its membership, and status says why. Expiry follows the offline copy. Tested in Task 4.
- **A background run whose role was taken away:** schedule, watcher, webhook and follow-up runs are held with a reason in the Inbox, not failed. Manual runs are refused with the role message. Tested in Task 6.
- **The identity is not made yet when orgs load** (first start, or the ID was lost): membership is evaluated after the Dots ID exists, and is never treated as `removed` because of a missing key. Tested in Task 4.
- **An old offline copy written by phase 1** (no roster field) still loads. Tested in Task 4.

---

### Task 1: Policy v2 fields

**Files:**
- Modify: `crates/dots-core/src/vault/org/policy.rs` (`POLICY_VERSION`, `FIELDS` :27-38, `OrgPolicy` :100, `RawPolicy` :116, `parse_policy` :154, `summary` :251)
- Test: `crates/dots-core/tests/vault_org_policy_test.rs`

**Interfaces:**
- Produces:
  - `pub const POLICY_VERSION: u32 = 2;` (the newest version), plus `pub const POLICY_VERSIONS: [u32; 2] = [1, 2];`
  - `str_enum!` `pub enum RosterMode { Off, Required }`, serialized `off|required`.
  - `OrgPolicy.roster: RosterMode` and `OrgPolicy.admins: Vec<String>` (minisign base64 public keys).
- Rules:
  - v1 keeps today's `FIELDS` exactly.
  - v2 allows `FIELDS` plus `roster` and `admins`.
  - Each admin must parse with `minisign_verify::PublicKey::from_base64`. A bad one gives `Invalid("admins[<i>] is not a Dots ID")`.
  - `summary()` adds `Member list: required, N admin(s)` when required.
- Check every use of `POLICY_VERSION` (dots-admin `policy new` writes it). `policy new` without `--roster` must keep writing v1 (Task 7), so callers that need "v1" use the literal 1 or a new `POLICY_V1` const.

- [ ] **Step 1: Write the failing tests:**
  - `v1_policy_means_roster_off`
  - `v2_policy_reads_roster_and_admins`
  - `v1_policy_with_roster_is_refused` (the error is the same "unknown field" error as today)
  - `v2_bad_admin_key_refused`
  - `version_3_refused` ("policy version 3 is not supported")
  - The existing v1 tests still pass.
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test vault_org_policy_test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Keep one flat struct with version-gated allowed fields (as described above). Leave every other `policy.field` user unchanged.
- [ ] **Step 4: Run the tests.** Run the test above, plus `cargo test -p dots-core --test vault_orgs_test` and `cargo test -p dots-admin`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): policy v2 with member list mode and admins"`

### Task 2: Roster module (parse, sign envelope, verify)

**Files:**
- Create: `crates/dots-core/src/accounts/roster.rs`
- Modify: `crates/dots-core/src/accounts/mod.rs`, `crates/dots-core/src/vault/org/signed.rs` (expose a shared signature check)
- Test: `crates/dots-core/tests/accounts_roster_test.rs`

**Interfaces:**
- Produces, in `accounts::roster`:
  - `pub const ROSTER_VERSION: u32 = 1;` and `pub const ROSTER_ENVELOPE_VERSION: u32 = 1;`
  - `str_enum!` `pub enum Role { Owner, Admin, Member, Viewer }` with `fn label(&self) -> &'static str` ("Owner"…).
  - `str_enum!` `pub enum WorkspaceRole { Admin, Member, Viewer }`
  - `pub struct Workspace { pub id: String, pub name: String, pub use_rules: Option<UseRules> }`, where the JSON key is `use` and `UseRules` comes from `vault::org`.
  - `pub struct Member { pub id: String, pub name: String, pub email: String, pub role: Role, pub keys: Vec<String>, pub workspaces: BTreeMap<String, WorkspaceRole>, pub added_at: String }`
  - `pub struct Roster { pub org_id: String, pub issued_at: String, pub seq: u64, pub workspaces: Vec<Workspace>, pub members: Vec<Member> }`, all with `Serialize, Deserialize, Clone, Debug, PartialEq`.
  - `pub fn parse_roster(text: &str, org_id: &str) -> Result<Roster, RosterError>`: every rule in Global Constraints except `seq` and trust.
  - `pub fn roster_envelope(roster_text: &str, signature_box: &str) -> String`, which produces `{"v":1,"roster":<text>,"signature":<box>}`.
  - `pub fn verify_roster(signed: &str, root_key: &str, admins: &[String], org_id: &str) -> Result<Accepted, RosterError>`. `pub struct Accepted { pub roster: Roster, pub text: String, pub signer: String }`, where `signer` is the key that verified it.
  - `impl Roster { pub fn check_seq(&self, last: Option<u64>) -> Result<(), RosterError>; pub fn member_with_key(&self, key: &str) -> Option<&Member> }`
  - `pub enum RosterError { Envelope(String), Signature, Invalid(String), WrongOrg { expected: String, found: String }, RolledBack { last: u64, found: u64 } }` with `Display` in plain words, for example `RolledBack` → "this member list (#5) is older than the one already accepted (#7)".
- Signature check: reuse the prehashed minisign check that `signed.rs` uses for the policy, through a `pub(crate) fn verify_with_any(text: &str, signature_box: &str, keys: &[&str]) -> Result<usize, ()>`. It returns the index of the key that verified, and `verify_policy` calls it with one key.

- [ ] **Step 1: Write the failing tests.** Build rosters in the test with `minisign::sign` (as `tests/common/orgs.rs::sign` does for policies):
  - `accepts_roster_signed_by_root`
  - `accepts_roster_signed_by_admin_key`
  - `refuses_roster_signed_by_stranger` (Signature)
  - `refuses_tampered_text`
  - `wrong_org` (WrongOrg)
  - `seq_must_grow`: `check_seq(Some(7))` on 5 and on 7 gives RolledBack, and on 8 is Ok.
  - One test per validation rule, each asserting its exact `Invalid` text:
    - `"the member list needs a 'default' workspace"`
    - `"workspace id 'Bad_Id' must be lowercase letters, digits and dashes, up to 32"`
    - `"member asha@acme.com names workspace 'nope', which isn't on the list"`
    - `"key RWQ… is listed for two members"` (the key cut to 8 characters plus "…")
    - `"the member list needs at least one Owner"`
    - `"unknown field 'x'"` at top level, in a member and in a workspace
  - `member_with_key_finds_member`
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test accounts_roster_test`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `roster.rs` and `verify_with_any`.
- [ ] **Step 4: Run the tests.** Run the test above plus `cargo test -p dots-core --test vault_org_policy_test`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): signed member list (roster v1) parse and verify"`

### Task 3: Sources read the roster

**Files:**
- Modify:
  - `crates/dots-core/src/vault/org/sources/mod.rs` (`Fetched` :52 gains `pub roster: Option<String>`, plus `pub const ROSTER_ENTRY: &str = "DOTS_ROSTER";`)
  - `crates/dots-core/src/vault/org/sources/openbao.rs` (`reads` :136, `read` :199-225)
  - `crates/dots-core/src/vault/org/sources/infisical.rs` (`read` :205-233, `skipped` :381)
  - `crates/dots-core/src/vault/org/sources/sops.rs` (`open` :215-311, `Part` :318)
  - Fakes: `crates/dots-core/tests/common/fake_openbao.rs` (`put_roster`) and `fake_infisical.rs` (`put_roster`), plus `tests/common/orgs.rs` (`roster(org_id, seq, members: Value) -> String` builder and `sign_roster(keys, text)`)
  - Every `Fetched { .. }` literal (`Scripted` source etc.)
- Test: `crates/dots-core/tests/vault_org_openbao_test.rs`, `vault_org_infisical_test.rs`, `vault_org_sops_test.rs`

**Interfaces:**
- Produces: `Fetched.roster`. Each source returns `None` when there is no roster, so a missing roster is never an error. A roster that is present but not text, or empty, is `SourceError::Invalid("DOTS_ROSTER is not text")`.
- OpenBao: a 404 on `<path>/roster` gives `None`, and other errors behave as the policy read does.
- Infisical: `DOTS_ROSTER` is never listed as an entry (`skipped`).
- SOPS: build the roster test file in a temp folder, the way the existing SOPS tests that write files do. Don't regenerate the checked-in fixtures.

- [ ] **Step 1: Write the failing tests.** For each source: `reads_roster_when_present` (`Fetched.roster == Some(signed)`) and `no_roster_is_none`. For Infisical also: `roster_is_not_an_entry`.
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test vault_org_openbao_test --test vault_org_infisical_test --test vault_org_sops_test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Run the same command plus `cargo test -p dots-core --test vault_orgs_test`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): sources read DOTS_ROSTER"`

### Task 4: Membership in the Orgs service

**Files:**
- Create: `crates/dots-core/migrations/0019_roster.sql`
- Modify:
  - `crates/dots-core/src/vault/org/service.rs` (`fetch` :800, `verify` :1273, `refresh_org` :744-794, `load_one` :961-990, `save_copy` :921, `status_text` :1181, `summary_of` :1212, `OrgSummary` :93)
  - `crates/dots-core/src/vault/org/offline.rs` (`OfflineCopy` :236 + `Stored` :257 gain `roster: Option<String>` with `#[serde(default)]`; `FORMAT` stays 1)
  - `crates/dots-core/src/vault/org/row.rs`, `crates/dots-core/src/store/orgs.rs` (`ORG_COLUMNS` :11, plus a new `record_membership`)
  - `crates/dots-core/src/runtime.rs` (move `accounts::ensure_identity` **before** `orgs.load()`, ~:155)
  - `apps/dots-app/src/contract.rs` (`OrgSummary` samples :1393/:1688), `ui/src/api/types.ts` (`orgSummaryShape` :949)
- Test: `crates/dots-core/tests/accounts_membership_test.rs` (harness: `tests/common/orgs.rs` `OrgsHarness`/`Scripted`, with a `MemorySecrets` holding an identity made by `ensure_identity`)

**Interfaces:**
- Consumes: Task 1 (`OrgPolicy.roster`, `.admins`), Task 2 (`verify_roster`, `Roster`, `Role`), Task 3 (`Fetched.roster`), and phase 1 `accounts::identity(&Store)` (`public_key`).
- Produces:
  - `str_enum!` `pub enum Membership { Legacy, Waiting, Active, Removed }` in `accounts`.
  - `OrgSummary` gains:
    - `membership: Membership`
    - `role: Option<Role>`
    - `roster_seq: Option<u64>`
    - `roster_error: Option<String>`
    - `member_count: Option<u32>`
  - `Orgs::access(&self, org_id: &str) -> Option<(Membership, Option<Role>)>`: `None` when the org is not joined.
- The migration:

```sql
-- Accounts phase 2 (spec 2026-10-28-accounts-members-design.md §6.4): this PC's place on each
-- account's signed member list. 'legacy' until the account's policy asks for a list.
ALTER TABLE orgs ADD COLUMN membership TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE orgs ADD COLUMN my_member_id TEXT;
ALTER TABLE orgs ADD COLUMN my_role TEXT;
ALTER TABLE orgs ADD COLUMN my_workspaces TEXT;
ALTER TABLE orgs ADD COLUMN last_roster_seq INTEGER;
```

- **Rules**, evaluated in `refresh_org`'s Accepted arm and at join, after the policy is verified:
  - If the policy is `RosterMode::Off`, membership is `legacy` and the roster is ignored.
  - If it is `Required` and `Fetched.roster` is `Some`:
    - `verify_roster(text, row.policy_key, &policy.admins, org_id)`, then `check_seq(last_roster_seq)`.
    - If it is valid: when this PC's identity key is in it, membership is `active` with the role and workspaces recorded. When it is not, `removed` if the previous membership was `active`, otherwise `waiting`.
  - If it is `Required` and the roster is missing or invalid: when a roster was accepted before, keep the old membership and set `roster_error` (status_text adds "member list not accepted: <error>"). When none was accepted before, membership is `waiting`.
  - `removed` runs the same wipe as the Denied arm (entries, offline copy) and labels the account "Removed from <name>". It stays removed until the person rejoins.
  - `waiting` keeps the entries out of resolution: `before_run` refuses with the waiting message, and grants show unavailable.
  - The accepted roster text is stored in the offline copy. `load_one` re-verifies it against the copy's policy, and if that fails it keeps the membership columns and sets `roster_error`.
  - If there is no identity when evaluated, membership is `waiting`, never `removed`.

- [ ] **Step 1: Write the failing tests:**
  - `v1_policy_is_legacy`
  - `required_and_on_list_is_active_with_role`
  - `required_not_on_list_first_time_is_waiting_nothing_wiped`
  - `active_then_dropped_is_removed_and_wiped`
  - `bad_roster_after_accepted_keeps_membership_and_reports`
  - `older_seq_is_refused_keeps_previous`
  - `admin_signed_roster_accepted_stranger_refused`
  - `old_offline_copy_without_roster_loads`
  - `waiting_dot_run_refused_with_message` (through `Vault::before_run`)
  - `summary_carries_membership_and_role`
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test accounts_membership_test`. Expected: FAIL.
- [ ] **Step 3: Implement**, then update the contract sample and the strict `OrgSummary` zod shape (all five new fields, nullable where they are `Option`), and regenerate with `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- [ ] **Step 4: Run the tests.** Run `cargo test -p dots-core --test accounts_membership_test --test vault_orgs_test --test vault_org_offline_test --test runtime_org_test`, then `cargo test -p dots-app`, then `cd ui && bun run test src/api`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): membership from the signed member list"`

### Task 5: Permissions and the refusals on direct actions

**Files:**
- Create: `crates/dots-core/src/accounts/permissions.rs`
- Modify:
  - `crates/dots-core/src/error.rs` (add `#[error("{0}")] NotAllowed(String)`)
  - `apps/dots-app/src/error.rs` (`CmdErrorKind::NotAllowed`, mapping and message, next to `Invalid` :30/:57/:64), plus the UI's command-error kind list if one exists (`grep -rn "not_found\|\"invalid\"" ui/src/api`)
  - `crates/dots-core/src/runtime.rs`:
    - `create_dot` :981, `update_dot` :998, `delete_dot` :1014, `set_dot_enabled` :1049, `regenerate_webhook_token` :1059
    - `run_now` :1065, `chat` :1073
    - `decide_approval` :1262, `decide_run_approvals` :1300
  - `crates/dots-core/src/library_service.rs` (`add_confirmed_origin` :844, `accept` :859, when the scope is an org)
  - `crates/dots-core/src/vault/resolve.rs` (`before_run` :361: waiting and removed messages, and run permission)
- Test: `crates/dots-core/tests/accounts_permissions_test.rs` (the `can` table), `crates/dots-core/tests/runtime_roles_test.rs` (runtime refusals; harness copied from `runtime_org_test.rs`, with a roster served by `FakeOpenBao::put_roster`)

**Interfaces:**
- Consumes: `Orgs::access` (Task 4).
- Produces:
  - `pub enum Permission { View, EditDots, RunDots, AnswerApprovals, GrantEntries, AddMcp, CreateApiKeys, ManageMembers, ManageWorkspaces, SignRoster }` with `fn action(&self) -> &'static str` ("see this account's dots", "create or change dots", "run dots", "answer approvals", "give secrets to dots", "add MCP servers", "create API keys", "manage members", "manage workspaces", "sign the member list").
  - `pub fn can(role: Role, p: Permission) -> bool`, the spec §6.3 table. `SignRoster` is true for Owner and Admin; whether an admin key is in the policy is checked elsewhere.
  - `pub enum Access { Owner, Role(Role), Legacy, Waiting, Removed }` with `fn allows(&self, p) -> bool`. Personal is `Owner`, and `Legacy` acts as Member.
  - `Runtime::access(&self, org_id: Option<&str>) -> Access`.
  - `Runtime::require(&self, org_id: Option<&str>, p: Permission) -> Result<()>`, which returns `Error::NotAllowed` with the exact texts from Global Constraints. The account name is the org name, and the role is `Role::label()`.
- `update_dot` requires `EditDots`, and additionally `GrantEntries` when the grants differ from the stored ones.
- `run_now` and `chat` require `RunDots` before enqueueing.
- Approvals load run → dot → org, then require `AnswerApprovals`.
- `before_run` refuses a dot whose access does not allow `RunDots`, using the waiting, removed or role text. This is the backstop for every trigger.

- [ ] **Step 1: Write the failing tests:**
  - `can_matrix` covers every (Role, Permission) pair against §6.3, as a table test.
  - In `runtime_roles_test`:
    - `viewer_cannot_create_edit_run_or_answer`: assert each `Error::NotAllowed` text exactly, for example `"Acme Engineering: a Viewer can't create or change dots. Ask an Admin or Owner."`.
    - `member_can_create_and_run`
    - `member_grant_change_needs_grant_permission_ok_for_member`
    - `waiting_run_refused_with_waiting_text`
    - `personal_always_allowed`
    - `legacy_account_acts_as_member`
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test accounts_permissions_test --test runtime_roles_test`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Run the tests above plus `cargo test -p dots-core` (once, before committing) and `cargo test -p dots-app`. Expected: PASS, apart from the known pre-existing `preview_schedule_lists_the_next_fire_times`.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): roles enforced on dots, runs, approvals and MCP"`

### Task 6: Background runs held for a role, with a reason in the Inbox

**Files:**
- Modify:
  - `crates/dots-core/src/runner.rs` (`dispatch_once` :466-470)
  - `crates/dots-core/src/model.rs` (`HeldRun` :565 gains `reason: HoldReason`)
  - `crates/dots-core/src/runtime.rs` (`held_runs` :1719, `inbox` :664, `run_release_hold` :1683)
  - `crates/dots-core/src/limits.rs` (`is_background` :78 reused)
  - `apps/dots-app/src/contract.rs`, `ui/src/api/types.ts` (`HeldRun`)
  - The Inbox held-runs UI (find it with `grep -rln "held" ui/src/screens | grep -i inbox`)
- Test: `crates/dots-core/tests/runtime_roles_test.rs` (add), `ui/src/screens/InboxHeld.test.tsx` (add)

**Interfaces:**
- Produces:
  - `#[serde(tag = "kind", rename_all = "snake_case")] pub enum HoldReason { Usage, Role { account: String, text: String } }`
  - The runner leaves a queued background run (`is_background(trigger)`) in the queue when its dot's access does not allow `RunDots`, and lists it as held with `Role`.
  - Manual and chat runs never reach this point, because Task 5 refuses them.
  - When access returns (role back, or rejoined), the run proceeds on the next dispatch.
  - `run_release_hold` on a role-held run gives `Error::NotAllowed("this run waits for <account> to let you run dots again")`.
- In the UI, a role-held row shows `text` (for example "Acme Engineering: your role no longer allows running dots") and no Release button.

- [ ] **Step 1: Write the failing tests:**
  - `scheduled_run_held_for_role_then_runs_when_role_returns`: demote with a new roster at `seq + 1`, check that the scheduled run is in `inbox().held` with the `Role` reason and not failed, then promote and check that it runs.
  - `release_refused_for_role_hold`
  - UI: a role hold shows its text and no Release button.
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test runtime_roles_test` and `cd ui && bun run test src/screens/InboxHeld.test.tsx`. Expected: FAIL.
- [ ] **Step 3: Implement**, update the contract and zod schema, and regenerate the fixtures.
- [ ] **Step 4: Run the tests.** Run the tests above plus `cargo test -p dots-core --test runner_limits_test --test runtime_limits_test` and `cargo test -p dots-app`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): background runs wait when a role no longer allows them"`

### Task 7: dots-admin roster and policy commands

**Files:**
- Create: `crates/dots-admin/src/roster.rs`
- Modify: `crates/dots-admin/src/lib.rs` (`USAGE` :64, dispatch :91-101), `crates/dots-admin/src/policy.rs` (`new_policy` :31 gains `--roster`, plus `admins add|remove`)
- Test: `crates/dots-admin/tests/admin_test.rs` (same `run_with` style)

**Interfaces:**
- Consumes: `dots_core::accounts::roster::{parse_roster, roster_envelope, verify_roster, Roster, Role}`, `dots_core::vault::org::key_fingerprint`.
- Produces these commands. Every write re-validates with `parse_roster` and prints the fingerprint of each key it adds.
  - `policy new --org-id <id> [--roster]`: with `--roster` it writes a v2 policy with `"roster": "required"` and `"admins": []`. Without it, v1 exactly as today.
  - `policy admins add <policy.json> <dots-id>` and `policy admins remove <policy.json> <dots-id>`: these only edit the file, and the owner signs afterwards with `policy sign`.
  - `roster new --org-id <id> --owner-name <n> --owner-email <e> --owner-key <dots-id> [--out roster.json]`: `seq: 0`, the `default` workspace, and the owner with `default: admin`.
  - `roster add <roster.json> --name <n> --email <e> --key <dots-id> --role <role> [--workspace <id>=<role> ...]`: when the email already exists, it adds the key to that member (a second PC).
  - `roster set-role <roster.json> <email> <role>` and `roster remove <roster.json> <email> [--key <dots-id>]`
  - `roster check <roster.json> --org-id <id>`
  - `roster sign <roster.json> --key policy.key`: sets `seq` to its value plus 1 and `issued_at` to now, signs with the root key (password through `Ask` / `DOTS_ADMIN_PASSWORD`), and prints the signed text to store as `DOTS_ROSTER`.
  - `roster verify <signed> --pub policy.pub [--admin <dots-id> ...] --org-id <id>`
- The `USAGE` text lists the new commands in the existing style.

- [ ] **Step 1: Write the failing tests:**
  - `roster_new_add_sign_verify_round_trip`
  - `roster_sign_bumps_seq`
  - `add_same_email_adds_key`
  - `remove_last_owner_refused` ("the member list needs at least one Owner")
  - `policy_new_roster_is_v2_required`
  - `policy_new_default_stays_v1`
  - `policy_admins_add_remove`
  - `bad_dots_id_refused` ("not a Dots ID")
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-admin`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Run `cargo test -p dots-admin` and `cargo clippy -p dots-admin --tests -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(dots-admin): member list and policy admins commands"`

### Task 8: UI for membership and roles

**Files:**
- Modify:
  - `ui/src/screens/Accounts.tsx` (`OrgRow` :53, `ORG_TONE` :30)
  - `ui/src/app/AccountSwitcher.tsx` (role badge per account)
  - the new-dot button and the editor Save (find them with `grep -rn "New dot\|createDot" ui/src`)
  - the approval buttons in `ui/src/components/ApprovalCard.tsx`
- Create: `ui/src/lib/access.ts`, `ui/src/lib/access.test.ts`
- Test: `ui/src/screens/Accounts.test.tsx`, `ui/src/app/AccountSwitcher.test.tsx`, plus the editor and approval tests (extend the existing ones)

**Interfaces:**
- Consumes: the `OrgSummary` fields from Task 4.
- Produces:
  - `accessOf(org: OrgSummary | undefined, orgId: string | null): { kind: "owner" | "role" | "legacy" | "waiting" | "removed"; role?: Role }`, mirroring core `Access`.
  - `allows(access, p: "edit" | "run" | "answer"): boolean`, mirroring the core table for those three.
  - `notAllowedText(accountName, access, p)`: the same strings as core (Global Constraints).
- Account row:
  - It shows a role badge (Owner, Admin, Member or Viewer).
  - Waiting shows "Waiting to be added to the member list" plus a **Copy my Dots ID** button (reuse the phase 1 copy logic).
  - Removed shows "Removed from <name>".
  - A roster error shows its text.
  - Member count and list number are shown as "N members · list #seq".
- Switcher: the role label next to each account.
- Disabled with a `title` of `notAllowedText(...)`:
  - New dot, when the current account doesn't allow edit.
  - Editor Save, when the dot's account doesn't allow edit.
  - Approval Approve and Deny, when the dot's account doesn't allow answer.

- [ ] **Step 1: Write the failing tests:**
  - `access.test.ts` covers the table and the texts.
  - Accounts: a waiting row shows the text and a Copy button that copies `public_key`. Removed and role badge cases are covered too.
  - Switcher: the role label shows.
  - Viewer: New dot is disabled with the exact title, and editor Save is disabled. Approval buttons are disabled for a Viewer.
- [ ] **Step 2: Run and confirm failure.** Run `cd ui && bun run test src/lib/access.test.ts src/screens/Accounts.test.tsx src/app`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Run `cd ui && bun run test && bun run typecheck`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): member list status, roles and role-disabled controls in the UI"`

### Task 9: Docs, smoke and gates

**Files:**
- Modify:
  - `docs/vault-admin.md`, adding a section "Accounts with a member list" that covers:
    - the root key versus admin keys
    - `roster new/add/sign`
    - storing `DOTS_ROSTER` for each source, at the exact paths from Global Constraints
    - the order of steps for moving a legacy org (publish the roster first, then switch the policy to v2 `required`, and update every Dots before using v2)
    - removing someone, and revoking their source credential as well
    - the threat-model lines from spec §11
  - `docs/accounts.md` (roles in plain words, the waiting and removed states, "Copy my Dots ID and send it to your admin")
  - `release-notes.md` (one line)
- Create: `docs/verification/2026-10-29-accounts-phase2-smoke.md`

- [ ] **Step 1: Write the smoke doc.** It is a manual checklist against the dev app with a fake or real OpenBao, covering:
  1. A v1 org is unchanged (legacy).
  2. Publish policy v2 `required` with a roster without this PC, and the account shows Waiting with Copy my Dots ID.
  3. Add the key with `roster add` and `sign`, refresh, and the account shows Active with the role.
  4. Demote to Viewer: New dot is disabled with the reason, a manual run is refused, and a scheduled run is held in the Inbox with the reason.
  5. Promote back, and the held run runs.
  6. Remove from the roster: the account shows Removed and its secrets are gone.
  7. Publish a roster with a lower `seq`: it is refused, and the previous roster is kept.
- [ ] **Step 2: Run the full gates.** Run `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --tests -- -D warnings`, `cd ui && bun run test && bun run typecheck`. Expected: all PASS, apart from the known pre-existing `runtime_review_test::preview_schedule_lists_the_next_fire_times`. Any other failure is checked against `feat/accounts` before you go on.
- [ ] **Step 3: Commit.** `git commit -m "docs(accounts): member list admin guide, roles for members, phase 2 smoke"`
