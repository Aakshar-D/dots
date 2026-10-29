# Accounts phase 3: invites, request codes, in-app signing and the Members page — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An Admin or Owner can invite a colleague from inside Dots, approve their signed request code, change roles, remove members or one key, and Dots signs the member list with the admin's own Dots ID and publishes it to OpenBao or Infisical (SOPS: copy flow), keeping a draft with Retry when publishing fails.

**Architecture:** Pure pieces live in `crates/dots-core/src/accounts/` (request codes, invite offers, roster edits). The `Orgs` service gains a second `impl Orgs` block in `vault/org/members.rs` that fetches fresh, edits, signs, publishes and keeps drafts. Sources gain `publish_roster`. The app exposes commands; the UI gets an Overview | Members tab strip on each account and an invite-aware Join flow.

**Tech Stack:** Rust (sqlx/SQLite, minisign 0.10, minisign-verify, reqwest 0.12, ulid, base64 0.22, axum fakes), Tauri commands + contract samples, React + TanStack Query + zod + vitest.

**Spec:** `docs/superpowers/specs/2026-10-28-accounts-members-design.md` (§5, §6.1, §6.4, §6.5, §7, §9 Members tab, §12, §13 phase 3, §14).

Base branch: `feat/accounts-invites`, created from `feat/accounts-roster` (ad5505e).

Rulings in this plan: changing a member's workspaces (§7) moves to phase 4 with workspace roles; members get `{"default": <role's default>}` here. The Members page is a tab strip on each account card rather than a separate page, so phases 4–5 add Workspaces and API keys tabs to it. Approving an invite marks it used even when the result is a draft, so it can't be approved twice.

## Global Constraints

- Never stage the root `Cargo.toml` (unrelated local change). Never add a Co-Authored-By trailer to commits.
- Every refusal is plain words naming the account, what can't be done and who to ask, like phase 2's `"{account}: a {Role} can't {action}. Ask an Admin or Owner."`
- Roles are enforced in `dots-core`, never only in the UI: every member action calls `accounts::require(orgs, Some(org_id), Permission::ManageMembers)` first.
- Signing in Dots uses this PC's Dots ID (the `identity` secret). Dots never imports the root key (§6.1).
- A list signed in Dots can't add, change or remove an Owner (root-key-only rule, `Roster::check_owners`). Edits touching an Owner are refused before signing with: `"Owners can only be changed with the account owner's key, using dots-admin."`
- Signing needs this PC's Dots ID in the policy's `admins`. Otherwise: `"{account}: your Dots ID isn't one of the account's signing admins yet. Ask the owner to add it with dots-admin policy admins add."`
- A new roster's `seq` is the base `seq` + 1. Before publishing Dots fetches again; if the accepted `seq` is no longer the base: `"{account}: the member list changed (now #{seq}) while you were editing. Check it and make the change again."` (§6.2 concurrent edits).
- Invite default expiry: 7 days. Request code prefix: `dotsreq_`. Invite codes are join codes with `v: 2` and an `invite` object; a code without an invite is still written as `v: 1` so older Dots read it.
- Migrations: next is `0021`. Never edit 0019/0020.
- UI: zod `strictObject` schemas; Deep Teal tokens; every disabled control says which role is needed (phase 2 `notAllowedText`). Contract fixture regenerated with `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- Gates per task: targeted `cargo test -p dots-core --test <file>`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`; UI tasks also `cd ui && npx vitest run && npx tsc --noEmit`. Known unrelated failure: `runtime_review_test::preview_schedule_lists_the_next_fire_times` (timezone).

## Review Focus

1. Two admins editing at once: the second publish must be refused with the "changed while you were editing" text and no draft kept (Task 6 `concurrent_edit_is_refused`).
2. Admin whose Dots ID isn't in `admins`: refused before anything is signed or drafted (Task 6 `signer_not_in_admins_is_refused_before_signing`).
3. Bad request codes: tampered, other org, expired invite, used invite, email mismatch, key already a member — each a distinct plain refusal (Task 1 and Task 6 tests).
4. Publish refused (403 / offline) leaves exactly one draft with the error; Retry after someone else changed the list is refused and the draft dropped (Task 6 `retry_after_list_changed_drops_draft`).
5. An admin trying to add a PC to an Owner, promote to Owner, or remove an Owner in Dots gets the Owner text, nothing signed (Task 4 `owner_entries_are_locked`).

---

### Task 1: Signing with the Dots ID, and request codes

**Files:**
- Modify: `crates/dots-core/src/accounts/identity.rs`
- Create: `crates/dots-core/src/accounts/request.rs`
- Modify: `crates/dots-core/src/accounts/mod.rs` (re-exports)
- Test: `crates/dots-core/tests/accounts_request_test.rs`

**Interfaces:**
- Produces:
  - `pub async fn sign_with_identity(store: &Store, secrets: &dyn SecretStore, text: &str, trusted_comment: &str) -> Result<String>` — minisign signature box (prehashed) by the identity secret key; `Error::Other` if the secret is missing.
  - `pub const REQUEST_PREFIX: &str = "dotsreq_";`
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(deny_unknown_fields)] pub struct AccessRequest { pub v: u32 /* 1 */, pub org_id: String, pub invite_id: Option<String>, pub key: String, pub name: String, pub email: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub age_recipient: Option<String>, pub created_at: String }`
  - `pub async fn make_request(store: &Store, secrets: &dyn SecretStore, org_id: &str, invite_id: Option<&str>, age_recipient: Option<&str>, now: DateTime<Utc>) -> Result<String>` — key/name/email from `identity()`; `Error::Invalid("Add your name and email in Settings › Profile first.")` if either is empty.
  - `pub fn parse_request(code: &str) -> Result<AccessRequest, RequestError>` — strips whitespace, requires the prefix, base64url (padding optional) of `{"request":"<AccessRequest JSON text>","signature":"<minisign box>"}` (`deny_unknown_fields`); verifies with `verify_with_any(text, sig, &[request.key])` before trusting any other field.
  - `pub enum RequestError { NotACode, Unreadable(String), Signature, Version(u32) }`, texts: `"this is not a Dots request code"`, `"the request code is not readable: {0}"`, `"the request code's signature doesn't match its Dots ID"`, `"request code version {0} is not supported"`.

- [ ] **Step 1: Failing tests** in `accounts_request_test.rs` (store/secrets from the existing test helpers, `ensure_identity`, `set_profile`):
  - `request_round_trips`: `make_request(.., "acme-eng", Some("inv1"), None, now)` starts with `dotsreq_`; `parse_request` gives `org_id == "acme-eng"`, `invite_id == Some("inv1")`, `key == identity.public_key`, name/email from the profile.
  - `request_without_invite_has_none`: `invite_id == None`; the request text has `"invite_id":null`.
  - `tampered_request_is_refused`: change `email` in the request text, keep the old signature → `RequestError::Signature`.
  - `request_signed_by_another_key_is_refused`: `key` is a stranger's, signed by this identity → `Signature`.
  - `garbage_is_not_a_request`: `"hello"` → `NotACode`; `"dotsreq_!!"` → `Unreadable(_)`.
  - `needs_a_profile`: no name → `Error::Invalid` with the Profile text.
  - `sign_with_identity_verifies`: `verify_with_any(text, sig, &[public_key])` is `Ok`.
- [ ] **Step 2: Run** `cargo test -p dots-core --test accounts_request_test` — FAIL (unresolved items).
- [ ] **Step 3: Implement.** Load the `SecretKeyBox` from secret `IDENTITY_SECRET`, `into_secret_key(None)`, `minisign::sign(Some(&pk), &sk, Cursor::new(text), Some(trusted_comment), None)`. Trusted comment for requests: `dots request <org_id>`.
- [ ] **Step 4: Run** — PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(accounts): sign with the Dots ID, and signed request codes`

---

### Task 2: Invite offers in join codes

**Files:**
- Modify: `crates/dots-core/src/vault/org/join.rs`, `crates/dots-core/src/vault/org/service.rs` (`JoinPreview.invite`)
- Test: the existing join-code test file (find it with `grep -l parse_join_code crates/dots-core/tests`); create `accounts_invite_code_test.rs` if none fits.

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)] #[serde(deny_unknown_fields)] pub struct InviteOffer { pub id: String, pub role: Role, pub expires_at: String }`
  - `JoinCode` gains `#[serde(default, skip_serializing_if = "Option::is_none")] pub invite: Option<InviteOffer>`.
  - `JOIN_CODE_VERSION` becomes 2. `JoinCode::encode()` writes `v: 1` when `invite` is `None`, `v: 2` when `Some`. `parse_join_code` accepts 1 and 2; `invite` with `v: 1` → `Error::Invalid("a version 1 join code can't carry an invite")`; other versions → the existing unsupported-version error.
  - `JoinPreview` gains `pub invite: Option<InviteOffer>` (`null` when none).
- [ ] **Step 1: Failing tests:** `plain_code_still_encodes_as_v1` (decoded JSON has `v == 1` and no `invite`); `invite_code_round_trips_as_v2`; `v1_with_invite_is_refused`; `v3_is_refused`; `preview_shows_the_offer` (`Orgs::preview_join` on an invite code: `invite.role == Role::Member`, `expires_at` kept).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** the join and `vault_orgs_*` tests — PASS.
- [ ] **Step 5: Commit** `feat(accounts): invite offers in join codes (v2)`

---

### Task 3: Tables for invites, drafts and the pending request

**Files:**
- Create: `crates/dots-core/migrations/0021_invites.sql`, `crates/dots-core/src/store/invites.rs` (register in `store/mod.rs`)
- Modify: `crates/dots-core/src/store/orgs.rs`, `crates/dots-core/src/vault/org/row.rs`
- Test: `crates/dots-core/tests/store_invites_test.rs`

**Interfaces:**
- Migration:
  ```sql
  CREATE TABLE account_invites (id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    email TEXT NOT NULL, role TEXT NOT NULL, workspaces TEXT NOT NULL, created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL, used_at TEXT, revoked_at TEXT);
  CREATE TABLE roster_drafts (org_id TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    base_seq INTEGER NOT NULL, seq INTEGER NOT NULL, signed TEXT NOT NULL, summary TEXT NOT NULL,
    error TEXT NOT NULL, created_at TEXT NOT NULL);
  ALTER TABLE orgs ADD COLUMN pending_request TEXT;
  ```
  If `PRAGMA foreign_keys` is not on for the pool, `Store::remove_org` must delete these rows itself (test covers it either way).
- Produces (`store/invites.rs`):
  - `pub struct InviteRow { pub id: String, pub org_id: String, pub email: String, pub role: Role, pub workspaces: BTreeMap<String, WorkspaceRole>, pub created_at: String, pub expires_at: String, pub used_at: Option<String>, pub revoked_at: Option<String> }`
  - `Store::add_invite(&InviteRow)`, `Store::invite(org_id, id) -> Result<Option<InviteRow>>`, `Store::list_invites(org_id) -> Result<Vec<InviteRow>>` (newest first), `Store::mark_invite_used(org_id, id, at)`, `Store::revoke_invite(org_id, id, at)` (`NotFound` if absent).
  - `pub struct DraftRow { pub org_id: String, pub base_seq: u64, pub seq: u64, pub signed: String, pub summary: String, pub error: String, pub created_at: String }`; `Store::put_draft(&DraftRow)` (replaces), `Store::draft(org_id) -> Result<Option<DraftRow>>`, `Store::delete_draft(org_id)`.
  - `OrgRow.pending_request: Option<PendingRequest>`, `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)] pub struct PendingRequest { pub code: String, pub invite_id: Option<String> }`, stored as JSON; `Store::set_pending_request(org_id, Option<&PendingRequest>)`.
- [ ] **Step 1: Failing tests:** invite add/get/list/used/revoke round trip; `put_draft` replaces the previous one for the org; pending request round trip and cleared with `None`; removing the org removes its invites and draft.
- [ ] **Step 2–4:** FAIL, implement, PASS (also `vault_orgs_store_test`).
- [ ] **Step 5: Commit** `feat(accounts): store invites, roster drafts and the pending request`

---

### Task 4: Roster edits (pure)

**Files:**
- Create: `crates/dots-core/src/accounts/edit.rs`
- Modify: `crates/dots-core/src/accounts/mod.rs`; `crates/dots-admin/src/roster.rs` (use the moved helpers, delete its private copies)
- Test: `crates/dots-core/tests/accounts_edit_test.rs`

**Interfaces:**
- Produces:
  - `pub fn new_member_id(roster: &Roster, key: &str) -> String` — moved unchanged from dots-admin `member_id` (`m-<12 hex of key fingerprint>`, `-N` on collision).
  - `pub fn default_workspace_role(role: Role) -> WorkspaceRole` — moved from dots-admin.
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)] pub enum RosterEdit { Approve { request: String, role: Option<Role>, confirm_no_invite: bool }, SetRole { member_id: String, role: Role }, Remove { member_id: String }, RemoveKey { member_id: String, key: String } }` — the wire form; the service turns `Approve` into `Change::AddKey`.
  - `pub enum Change { AddKey { name: String, email: String, key: String, role: Role, workspaces: BTreeMap<String, WorkspaceRole> }, SetRole { member_id: String, role: Role }, Remove { member_id: String }, RemoveKey { member_id: String, key: String } }`
  - `pub fn apply(roster: &Roster, change: &Change, now: DateTime<Utc>) -> crate::Result<(Roster, String)>` — new roster with `seq + 1`, `issued_at = now` (RFC 3339, seconds, `Z`), plus a one-line summary: `"Add asha@acme.com as Member"`, `"Add a PC for Asha Rao"`, `"Change Asha Rao to Viewer"`, `"Remove Asha Rao"`, `"Remove one PC of Asha Rao"`. The result is serialized and run through `parse_roster` so every §6.2 rule holds.
  - Rules: AddKey whose email is already listed adds the key to that member (role/workspaces unchanged); a key already listed → `Error::Conflict("This Dots ID is already on the member list.")`; any Owner touched (role to/from Owner, removing an Owner, adding/removing an Owner's key) → `Error::NotAllowed(<Owner text>)`; removing a member's last key removes the member; unknown id → `Error::NotFound("No member '{id}' on the list.")`. `dots-admin` keeps allowing Owner changes (it signs with the root key), so it calls a lower-level `apply_unchecked` or passes `allow_owner_changes: bool` — pick one, `apply` itself always refuses.
- [ ] **Step 1: Failing tests:** `add_new_member_bumps_seq`; `add_key_to_existing_email_keeps_role`; `key_already_listed_is_refused`; `owner_entries_are_locked` (promote to Owner, demote an Owner, remove an Owner, add a key to an Owner's email); `remove_last_key_removes_member`; `unknown_member_is_not_found`; `summaries_read_plainly`.
- [ ] **Step 2–4:** FAIL, implement, PASS; `cargo test -p dots-admin` still PASS after the move.
- [ ] **Step 5: Commit** `feat(accounts): roster edits shared by Dots and dots-admin`

---

### Task 5: Publishing the roster from sources

**Files:**
- Modify: `crates/dots-core/src/vault/org/sources/{mod.rs,openbao.rs,infisical.rs,sops.rs}`
- Modify: `crates/dots-core/tests/common/{fake_openbao.rs,fake_infisical.rs}` (write routes)
- Test: `crates/dots-core/tests/vault_publish_roster_test.rs`

**Interfaces:**
- Produces:
  - `pub enum PublishError { Unsupported, Denied(String), Unreachable(String), Invalid(String) }` (thiserror; texts `"this source can't be written from Dots"`, `"the source refused the write: {0}"`, `"the source can't be reached: {0}"`, `"the source answered oddly: {0}"`).
  - `Source` gains `async fn publish_roster(&self, creds: &SourceCreds, signed: &str) -> Result<(), PublishError> { Err(PublishError::Unsupported) }` (default; SOPS keeps it).
  - OpenBao: `POST {address}/v1/{mount}/data/{path}/roster`, body `{"data":{"DOTS_ROSTER": signed}}`, same token/namespace/login handling as reads. 401/403 → Denied naming the path; network/5xx → Unreachable.
  - Infisical: upsert secret `DOTS_ROSTER` in the policy folder: `PATCH /api/v4/secrets/DOTS_ROSTER` with `{projectId, environment, secretPath, secretValue}`; on 404, `POST` the same. Confirm field names against current Infisical API docs; if v4 writes differ, use the documented v3 raw endpoints and say so in the commit. 401/403 → Denied.
  - Fakes: OpenBao KV v2 POST handler storing `data` at the path (with a `deny_writes(mount, path)` switch; requests recorded). Infisical PATCH/POST on `/api/v4/secrets/{name}` (404 on PATCH when missing; `deny_writes()` switch).
- [ ] **Step 1: Failing tests:** `openbao_publish_then_fetch_reads_it`; `openbao_publish_denied_is_denied`; `infisical_publish_creates_then_updates`; `infisical_publish_denied_is_denied`; `sops_publish_is_unsupported`; `unreachable_is_unreachable` (`fail_next(Status(503))`).
- [ ] **Step 2–4:** FAIL, implement, PASS (existing source tests too).
- [ ] **Step 5: Commit** `feat(accounts): publish the member list to OpenBao and Infisical`

---

### Task 6: Orgs member service: invites, requests, edits, drafts

**Files:**
- Create: `crates/dots-core/src/vault/org/members.rs` (second `impl Orgs`; `mod members;` in `vault/org/mod.rs`)
- Modify: `crates/dots-core/src/vault/org/service.rs` (`OrgSummary.request_code`; join with an invite; `pending_request` cleared when `active`; drafts dropped once their `seq` is accepted)
- Test: `crates/dots-core/tests/accounts_members_service_test.rs`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces (`pub async fn` on `Orgs`, errors `crate::Error`):
  - `members(&self, org_id) -> Result<MembersView>`. `MembersView { seq: u64, members: Vec<MemberView>, can_manage: bool, can_sign: bool, sign_blocker: Option<String>, publishes: bool /* false for SOPS */, invites: Vec<InviteView>, draft: Option<DraftView> }`; `MemberView { id, name, email, role: Role, keys: Vec<KeyView { key, fingerprint, is_me: bool, signs: bool /* in policy admins */ }>, workspaces: BTreeMap<String, WorkspaceRole>, added_at, owner_locked: bool }`; `InviteView { id, email, role, created_at, expires_at, state: InviteState }` with `InviteState` `open|used|expired|revoked`; `DraftView { base_seq, seq, summary, error, created_at, signed }`. `Error::NotFound` when the account has no accepted list.
  - `create_invite(&self, org_id, email: &str, role: Role) -> Result<CreatedInvite>`; `CreatedInvite { invite: InviteView, code: String }` — `code` is the join code rebuilt from the `OrgRow` (name, source settings, policy_key) with `invite: Some(InviteOffer { id: <ulid>, role, expires_at: now + 7 days })`. Role Owner → the Owner text. Workspaces stored as `{"default": default_workspace_role(role)}`. Needs `ManageMembers`.
  - `revoke_invite(&self, org_id, invite_id) -> Result<()>`.
  - `request_access(&self, org_id) -> Result<OrgSummary>` — only while `waiting` (else `Error::Invalid`); makes a request with no invite (`age_recipient` from `sops_identity` for SOPS accounts), stores the `PendingRequest`, returns the summary with `request_code`. Called again it returns the same code.
  - `join` with an invite code: after the join, if the account is `waiting`, store a `PendingRequest` with the invite id. `OrgSummary` gains `request_code: Option<String>` (set while `waiting` with a pending request); becoming `active` clears `pending_request`.
  - `inspect_request(&self, org_id, code) -> Result<RequestPreview>`; `RequestPreview { name, email, key, fingerprint, invite: Option<InviteView>, role: Option<Role>, age_recipient: Option<String>, problem: Option<String> }`. `problem`, in plain words: other account (`"This request is for another account ({org_id})."`); invite not from this PC (`"This invite wasn't made on this PC."`); invite used / expired / revoked; email mismatch (`"This request is for {request_email}, but the invite was for {invite_email}."`); Dots ID already on the list. A code that doesn't parse is `Error::Invalid(<RequestError text>)`.
  - `edit_members(&self, org_id, base_seq: u64, edit: RosterEdit) -> Result<EditOutcome>`; `#[serde(tag = "outcome", rename_all = "snake_case")] pub enum EditOutcome { Published { summary: OrgSummary }, Draft { draft: DraftView } }`. Order:
    1. `require(ManageMembers)`.
    2. Signer check: this PC's key in `policy.admins`, else the Global Constraints text; nothing stored.
    3. Refresh now; accepted `seq != base_seq` → `Error::Conflict(<changed text>)`.
    4. Resolve Approve: a `problem` from the `inspect_request` checks → `Error::Invalid(problem)`; no invite needs `confirm_no_invite` and a `role`, else `Error::Invalid("This person has no invite from this PC. Confirm to approve them anyway, and pick a role.")`; the invite's role and workspaces win when present.
    5. `edit::apply`, then `check_owners(false, row.member.owners_hash)` as a guard.
    6. `sign_with_identity` (trusted comment `dots roster <org_id> <seq>`), `roster_envelope`, `publish_roster`.
    7. Ok → refresh; invite marked used; draft deleted; `Published`. `Unsupported` or any `PublishError` → `put_draft` (Unsupported's error text: `"Copy the signed list into the SOPS file (see the admin guide)."`); invite marked used; `Draft`.
  - `retry_draft(&self, org_id) -> Result<EditOutcome>` — refresh; accepted `seq >= draft.seq` → delete draft, `Published`; accepted `seq != draft.base_seq` → delete draft, `Error::Conflict(<changed text>)`; else publish again (same outcomes).
  - `discard_draft(&self, org_id) -> Result<()>`.
- [ ] **Step 1: Failing tests** (fake OpenBao; policy v2 `required` with this PC's Dots ID in `admins`; this PC an Admin on seq 1, signed with the root key in setup):
  - `invite_then_approve_publishes_and_newcomer_becomes_active` — a second service with its own data dir and the same fake joins with the invite code → `waiting`, `request_code` set; the admin's `inspect_request` has no problem and `role == Some(Member)`; `edit_members(.., 1, Approve { .. })` → `Published`, `roster_seq == Some(2)`; the newcomer's `refresh` → `active`, `request_code == None`.
  - `concurrent_edit_is_refused` — publish seq 2 out of band, then `edit_members(.., 1, ..)` → `Conflict` with the changed text; no draft.
  - `signer_not_in_admins_is_refused_before_signing` — `admins: []` → `NotAllowed` text; no draft; the fake saw no write.
  - `member_role_cannot_manage` — this PC a Member → `NotAllowed` (phase 2 text).
  - `expired_used_and_mismatched_invites_are_problems` — advance 8 days → expired; approving twice → used; request with another email → mismatch text.
  - `approve_without_invite_needs_confirm_and_role`.
  - `publish_denied_keeps_a_draft` (fake `deny_writes` on the roster path) → `Draft` with the Denied text; `members().draft` is `Some`.
  - `retry_after_list_changed_drops_draft`; `retry_publishes_when_allowed` (lift the deny, retry → `Published`, draft gone).
  - `sops_edit_is_a_draft_to_copy` — `Draft` whose `signed` verifies against this PC's key; once the test writes it into the SOPS fixture and refreshes, the draft is gone.
  - `request_access_for_waiting_pc_is_stable` — same code twice; its `invite_id` is `None`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun `accounts_membership_test` and the `vault_orgs_*` tests.
- [ ] **Step 5: Commit** `feat(accounts): invites, approvals and member edits signed in Dots, with drafts`

---

### Task 7: dots-admin request commands

**Files:**
- Create: `crates/dots-admin/src/request.rs`; modify `crates/dots-admin/src/roster.rs` and the subcommand dispatch
- Test: `crates/dots-admin/tests/admin_test.rs`

**Interfaces:**
- `dots-admin request inspect <code>` prints org, name, email, fingerprint, invite id (or `none`), age recipient (if any), created_at; on a bad code exits non-zero with the `RequestError` text.
- `dots-admin roster add-request <file> <code> --role <role>` — `parse_request`; `org_id` must equal the file's; adds the member (or the key to the member with that email) via Task 4's helpers with Owner changes allowed; writes the unsigned file (no seq bump; `sign` bumps, as today). Prints the age-recipient reminder for SOPS when present.
- [ ] **Step 1: Failing tests:** `request_inspect_prints_fields`; `request_inspect_refuses_tampered`; `add_request_adds_member_with_role`; `add_request_wrong_org_is_refused`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(dots-admin): request inspect and roster add-request`

---

### Task 8: App commands and contract

**Files:**
- Modify: `crates/dots-core/src/runtime.rs` (thin wrappers next to `join_org`), `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs` (`command_list!`), `apps/dots-app/src/contract.rs`, `ui/src/api/__fixtures__/contract.json` (regenerated), `ui/src/api/commands.ts` (names only, so the command-name check passes)

**Interfaces:**
- Commands (`org_id: String` first, `-> Res<_>`): `member_list -> MembersView`, `invite_create(email, role) -> CreatedInvite`, `invite_revoke(invite_id) -> ()`, `request_inspect(code) -> RequestPreview`, `member_edit(base_seq: u64, edit: RosterEdit) -> EditOutcome`, `roster_draft_retry -> EditOutcome`, `roster_draft_discard -> ()`, `org_request_access -> OrgSummary`.
- Contract samples: `MembersView`, `CreatedInvite`, `RequestPreview`, `EditOutcome` (both variants), `JoinPreview` with an invite, `OrgSummary` with `request_code`.
- [ ] **Step 1:** add the samples; `cargo test -p dots-app contract` FAIL (fixture differs).
- [ ] **Step 2:** implement the commands and wrappers; `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then rerun without it — PASS.
- [ ] **Step 3: Commit** `feat(accounts): app commands for members, invites and drafts`

---

### Task 9: UI — join with an invite, and waiting with a request code

**Files:**
- Modify: `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/screens/Accounts.tsx` (`JoinDialog`, `OrgRow`)
- Create: `ui/src/components/RequestCodeBox.tsx`
- Test: `ui/src/screens/Accounts.test.tsx` (extend); the existing contract test

**Interfaces:**
- Schemas (`strictObject`): `InviteOffer`, `JoinPreview.invite`, `OrgSummary.request_code` (nullable), `MembersView`, `MemberView`, `KeyView`, `InviteView`, `DraftView`, `CreatedInvite`, `RequestPreview`, `EditOutcome` (discriminated on `outcome`), `RosterEdit` (discriminated on `kind`).
- Commands: `memberList`, `inviteCreate`, `inviteRevoke`, `requestInspect`, `memberEdit`, `rosterDraftRetry`, `rosterDraftDiscard`, `orgRequestAccess`.
- `RequestCodeBox({ account, code })`: read-only monospace code, **Copy request code**, line `"Send this to an Admin of {account}. Dots checks again on its normal refresh."`
- JoinDialog preview: `"Invited as {Role} · expires {date}"` when `preview.invite`; an expired offer shows `"This invite has expired. Ask for a new one."` and disables Join. After joining with an invite code the dialog shows `RequestCodeBox` instead of closing.
- `OrgRow` while `waiting`: `RequestCodeBox` when `request_code`, else a **Request access** button calling `orgRequestAccess`.
- [ ] **Step 1: Failing vitest tests:** `join preview shows the offered role and expiry`; `expired invite disables Join`; `after joining with an invite the request code is shown`; `waiting account without a request offers Request access`; `request code copies to the clipboard`.
- [ ] **Step 2–4:** FAIL, implement, PASS; `npx tsc --noEmit` clean.
- [ ] **Step 5: Commit** `feat(accounts): join with an invite and send a request code`

---

### Task 10: UI — Members tab

**Files:**
- Create: `ui/src/screens/accounts/AccountMembers.tsx`, `ui/src/screens/accounts/InviteDialog.tsx`, `ui/src/screens/accounts/ApproveDialog.tsx`
- Modify: `ui/src/screens/Accounts.tsx` (each account card gets tabs **Overview** | **Members**; Overview is today's row content; Members shown for `active` accounts)
- Test: `ui/src/screens/accounts/AccountMembers.test.tsx`

**Behaviour:**
- List: name, email, role badge, workspaces, each key's fingerprint, "This PC" on `is_me`. An Admin none of whose keys `signs` shows: "Can sign the list once the owner adds their Dots ID to the policy's admins." (§6.1)
- With `can_manage && can_sign`:
  - **Invite** (email; role Admin/Member/Viewer). The result shows the code with Copy and **Save as .dotsorg**.
  - Open invites with **Revoke**.
  - **Approve request**: paste a code → `requestInspect` → name, email, fingerprint, role, the age recipient for SOPS, and `problem` in an alert (Approve disabled). No invite → checkbox "This person has no invite from this PC" plus a role picker.
  - Per-member role select. **Remove** asks to confirm with: "Also revoke their access in the source: the list stops Dots honouring them, only the source cuts them off." Per-key **Remove this PC**.
- Owner rows: controls disabled, title = the Owner text. Without `can_manage`: disabled with phase 2 `notAllowedText` (needs Admin or Owner). `can_manage` without `can_sign`: disabled with `sign_blocker`.
- Every edit sends `base_seq = view.seq`; a `conflict` error shows its text and refetches.
- Draft banner when `draft`: summary, error, **Retry**, **Discard**; when `!publishes` also **Copy signed list**.
- [ ] **Step 1: Failing vitest tests:** `viewer sees members but no controls`; `admin invites and sees the code`; `approve shows the request and publishes`; `approve problem is shown and Approve disabled`; `owner rows are locked with the owner text`; `not a signing admin explains why`; `conflict refetches and shows the text`; `draft banner retries and discards`; `sops draft offers copy`.
- [ ] **Step 2–4:** FAIL, implement, PASS; full `npx vitest run` and `npx tsc --noEmit`.
- [ ] **Step 5: Commit** `feat(accounts): Members tab with invites, approvals, role changes and drafts`

---

### Task 11: Docs, smoke and release notes

**Files:**
- Modify: `docs/vault-admin.md` section 10 (inviting and approving in Dots; signing admins; write rights per source, including the ACL write path for `…/roster`; SOPS copy flow; drafts; `request inspect` and `roster add-request`; Owner changes stay in dots-admin), `docs/accounts.md` (join with an invite, request access, waiting for approval), `release-notes.md`
- Create: `docs/verification/2026-10-29-accounts-phase3-smoke.md` (two data folders on one PC sharing a dev OpenBao: invite → join → request → approve → active; role change; remove; publish denied → draft → retry; SOPS copy flow)
- [ ] **Step 1:** write the docs; check every command name and quoted text against the code.
- [ ] **Step 2: Commit** `docs(accounts): inviting, approving and member edits in Dots; phase 3 smoke`
