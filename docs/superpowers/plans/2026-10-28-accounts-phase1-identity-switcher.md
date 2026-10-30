# Accounts phase 1: Dots ID and switcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every install has a Dots ID (key pair, name, email) and a Profile. Organisations are shown as Accounts. A switcher in the main rail picks Personal or a joined account, and Home and new dots follow it. Teams carry their account.

**Architecture:**
- A new `dots_core::accounts` module holds the identity (`identity.rs`) and the current selection (`scope.rs`). Both are stored as `settings` markers, outside `Settings`, with the secret key in the existing `SecretStore`.
- The runtime makes the identity at start and exposes four calls, which Tauri commands wrap.
- The UI adds a Profile section, renames Organisations to Accounts, adds `AccountSwitcher` at the top of the rail, and filters Home client-side by `dot.org_id`.
- Migration `0018_accounts.sql` adds `teams.org_id`.

**Tech Stack:** Rust (sqlx/SQLite, `minisign` 0.10 already a dependency), Tauri commands, React + TanStack Query + zod, vitest.

**Spec:** `docs/superpowers/specs/2026-10-28-accounts-members-design.md` (§5, §9, §13 phase 1)

## Global Constraints

- Secret key in the `SecretStore` under key `identity` (Credential Manager target `Dots/<install_id>/identity`), minisign format, **no password**.
- Settings markers: `identity_public_key`, `profile_name`, `profile_email`, `current_scope`.
- The fingerprint is `vault::org::key_fingerprint(public_key)`, never a new scheme.
- The workspace is always `"default"` in phase 1. Any other value is `Error::Invalid`.
- `current_scope.account` is `"personal"` or a joined org id.
- UI copy says **Account(s)**, never "Organisation(s)". Code names (`orgs`, `vault::org`, `org_id`) stay.
- Needs you, Inbox, the rail counts and notifications are **never** filtered by the switcher.
- Every new type the UI receives gets a sample in `apps/dots-app/src/contract.rs`. Regenerate with `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- Commit messages carry no AI co-author trailer.

## Review Focus

- **The selected account is left or removed:** `current_scope()` falls back to Personal and Home is not blank. Tested in Task 2 and Task 5.
- **Credential Manager lost the secret key but `identity_public_key` remains** (a reinstall or cleared vault): Dots makes a new pair and replaces the public key, with no start-up failure. Tested in Task 1.
- **Profile input:** blank or whitespace name, an email without `@`, or text over the limits is rejected with a plain message, and surrounding spaces are trimmed. Tested in Task 1 and Task 4.
- **Something needs you in an account that isn't selected:** the switcher shows its waiting count, and the Inbox still lists it. Tested in Task 5.
- **Existing teams with dots from several orgs** migrate to Personal. Creating or updating a team with a dot from another account is refused with a message naming the dot. Tested in Task 3.

---

### Task 1: Dots ID in core

**Files:**
- Create: `crates/dots-core/src/accounts/mod.rs`, `crates/dots-core/src/accounts/identity.rs`
- Modify: `crates/dots-core/src/lib.rs` (add `pub mod accounts;`), `crates/dots-core/src/runtime.rs` (call `ensure_identity` after `Vault::open` at ~:167, and add methods)
- Test: `crates/dots-core/tests/accounts_identity_test.rs`

**Interfaces:**
- Produces:
  - `pub struct Identity { pub public_key: String, pub fingerprint: String, pub name: String, pub email: String }`, with `Serialize, Deserialize, Clone, Debug, PartialEq`. `name` and `email` are `""` until set.
  - `pub async fn ensure_identity(store: &Store, secrets: &dyn SecretStore) -> Result<Identity>`. It makes the key pair when the marker or the secret is missing, and returns the current identity.
  - `pub async fn identity(store: &Store) -> Result<Identity>`.
  - `pub async fn set_profile(store: &Store, name: &str, email: &str) -> Result<Identity>`.
  - `pub const IDENTITY_SECRET: &str = "identity";`
  - `pub const MAX_PROFILE_NAME: usize = 80;` and `pub const MAX_EMAIL: usize = 254;`
  - `Runtime::identity(&self) -> Result<Identity>` and `Runtime::set_profile(&self, name: &str, email: &str) -> Result<Identity>`.

- [ ] **Step 1: Write the failing tests** in `accounts_identity_test.rs`. They use `Store::open` on a tempdir and `MemorySecrets`.

```rust
#[tokio::test] async fn makes_identity_once() {
    // first ensure: public_key non-empty, fingerprint == key_fingerprint(&public_key), name/email ""
    // secrets.get("identity") is Some and parses with minisign::SecretKeyBox::from_string
    // second ensure returns the same public_key
}
#[tokio::test] async fn new_pair_when_secret_lost() {
    // ensure; secrets.delete("identity"); ensure again -> different public_key, secret present again
}
#[tokio::test] async fn profile_trims_and_validates() {
    // set_profile("  Asha Rao ", " asha@acme.com ") -> name "Asha Rao", email "asha@acme.com"
    // "" or "   " name -> Err(Error::Invalid(m)), m == "a name is needed"
    // "asha" email   -> m == "an email address needs an @"
    // 81-char name   -> m == "a name must be at most 80 characters"
    // 255-char email -> m == "an email address must be at most 254 characters"
    // identity(&store) after a set returns the stored values
}
```

- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test accounts_identity_test`. Expected: FAIL to compile (`accounts` is not found).

- [ ] **Step 3: Implement `identity.rs`.**
  - Use `minisign::KeyPair::generate_unencrypted_keypair()`.
  - Store `sk.to_box(Some("dots identity secret key"))?.into_string()` in the `SecretStore`.
  - Store `pk.to_base64()` in marker `identity_public_key` (`Store::set_marker`, `keep: false`).
  - The email check is: contains `@`, with a non-empty part on each side.

- [ ] **Step 4: Wire the runtime.** In `Runtime::start`, after `Vault::open`, call `accounts::ensure_identity(&store, secrets.as_ref())`. A failure is logged with `tracing::warn!` and does not stop start-up (the same pattern as `move_stored_literals`). Add the two `Runtime` methods.

- [ ] **Step 5: Run and confirm the tests pass.** Run `cargo test -p dots-core --test accounts_identity_test`. Expected: PASS. Also run `cargo test -p dots-core --test runtime_test` and expect PASS (start-up is unaffected).

- [ ] **Step 6: Commit.** `git commit -m "feat(accounts): Dots ID key pair and profile in core"`

### Task 2: Current scope in core

**Files:**
- Create: `crates/dots-core/src/accounts/scope.rs`
- Modify: `crates/dots-core/src/accounts/mod.rs`, `crates/dots-core/src/runtime.rs`
- Test: `crates/dots-core/tests/accounts_scope_test.rs`

**Interfaces:**
- Consumes: `Orgs::summaries()` (the joined orgs, as `org_list` uses them).
- Produces:
  - `pub struct CurrentScope { pub account: String, pub workspace: String }`, with `Serialize, Deserialize, Clone, Debug, PartialEq`.
  - `pub const PERSONAL: &str = "personal";` and `pub const DEFAULT_WORKSPACE: &str = "default";`
  - `Runtime::current_scope(&self) -> Result<CurrentScope>`. It reads marker `current_scope`. A missing marker, unreadable JSON, or an `account` that is no longer a joined org returns `{ account: "personal", workspace: "default" }`, without rewriting the marker.
  - `Runtime::set_current_scope(&self, account: &str, workspace: &str) -> Result<CurrentScope>`. It refuses an `account` that is not `personal` or a joined org with `Error::Invalid("no account '<id>' on this PC")`, and a workspace other than `default` with `Error::Invalid("no workspace '<ws>' in this account")`.

- [ ] **Step 1: Write the failing tests** with the runtime harness pattern of `tests/runtime_org_test.rs` (`FakeOpenBao`, `common::orgs` helpers):
  - `default_is_personal`
  - `set_and_read_back_joined_org`
  - `unknown_account_refused` (asserts the exact message)
  - `other_workspace_refused`
  - `falls_back_after_org_removed`: join, set, `orgs().remove`, then `current_scope().account == "personal"`.
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test accounts_scope_test`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `scope.rs` and the runtime methods.
- [ ] **Step 4: Run and confirm the tests pass.** Run the same command. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): current account selection in core"`

### Task 3: Teams carry their account

**Files:**
- Create: `crates/dots-core/migrations/0018_accounts.sql`
- Modify: `crates/dots-core/src/model.rs:798-889` (`TeamSpec`, `Team`), `crates/dots-core/src/store/teams.rs` (`create_team`, `update_team`, row mapping in `get_team`/`list_teams`), `apps/dots-app/src/contract.rs` (Team sample) and `ui/src/api/types.ts` if a Team schema exists there
- Test: `crates/dots-core/tests/teams_test.rs` (add cases)

**Interfaces:**
- Produces:
  - `TeamSpec.org_id: Option<String>` with `#[serde(default)]`.
  - `Team.org_id: Option<String>`.
  - `pub const TEAM_ORG_BACKFILL_SQL: &str` in `store/teams.rs`, `#[doc(hidden)]`: the same `UPDATE` as the migration, so the integration test can run it again. A comment says it must match `0018_accounts.sql`.

The migration:

```sql
-- Accounts phase 1 (spec 2026-10-28-accounts-members-design.md §8): a team's account. NULL is
-- Personal. A team whose dots are all in one org takes that org; any other team is Personal.
ALTER TABLE teams ADD COLUMN org_id TEXT;
UPDATE teams SET org_id = (
    SELECT MIN(d.org_id) FROM team_members m JOIN dots d ON d.id = m.dot_id WHERE m.team_id = teams.id
) WHERE (
    SELECT COUNT(DISTINCT IFNULL(d.org_id, '')) FROM team_members m JOIN dots d ON d.id = m.dot_id
    WHERE m.team_id = teams.id
) = 1;
```

- [ ] **Step 1: Write the failing tests:**
  - `team_takes_spec_org`: create with `org_id: Some(ACME)` and one Acme dot, then `team.org_id == Some(ACME)`.
  - `team_refuses_dot_of_other_account`: a Personal dot in an Acme team gives `Error::Invalid("dot '<name>' is in another account than the team")`. The same holds for `update_team`.
  - `backfill_assigns_single_org_teams`: build two Personal teams, then use raw SQL to set their dots' `org_id` (one team all Acme, one mixed) and `teams.org_id = NULL`. Run `TEAM_ORG_BACKFILL_SQL` on the pool (the test reaches it through a `#[doc(hidden)] pub fn Store::pool_for_tests()` if none exists, or through an existing test hook). Then the all-Acme team has Acme and the mixed team stays NULL.
- [ ] **Step 2: Run and confirm failure.** Run `cargo test -p dots-core --test teams_test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Write the migration. In `create_team`, look up each member dot's `org_id` inside the transaction and compare it with `spec.org_id`. `update_team` keeps the team's stored `org_id` (a team's account is fixed, like a dot's), ignores `spec.org_id`, and checks members against the stored one.
- [ ] **Step 4: Run the tests.** Run `cargo test -p dots-core --test teams_test`, then `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): teams belong to an account"`

### Task 4: Tauri commands, contract, Profile section and the Accounts rename

**Files:**
- Modify:
  - `apps/dots-app/src/commands.rs` (four commands next to `org_list` at :854)
  - `apps/dots-app/src/lib.rs` (register them next to `org_list` at :131)
  - `apps/dots-app/src/contract.rs` (`Identity`, `CurrentScope` samples)
  - `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/lib/query.ts` (keys `identity`, `scope`)
  - `ui/src/screens/SettingsScreen.tsx:200` (render `<ProfileSection />` above accounts)
  - Rename `ui/src/screens/Organisations.tsx` to `Accounts.tsx` (and its test), plus every UI string "Organisation(s)" or "Its org" shown to people, found with `grep -rn "Organisation\|Its org" ui/src` (`HomeScreen.tsx:381`, `lib/dot-org.ts` `orgRemovedText`, the `dot-editor/DotOrg.tsx` label)
- Create: `ui/src/screens/ProfileSection.tsx`, `ui/src/screens/ProfileSection.test.tsx`
- Test: `ui/src/api/commands.test.ts`, `ui/src/api/contract.test.ts` (fixture-driven)

**Interfaces:**
- Consumes: Task 1 and Task 2 runtime methods.
- Produces:
  - Tauri commands `identity_get() -> Identity`, `profile_set(name: String, email: String) -> Identity`, `scope_get() -> CurrentScope` and `scope_set(account: String, workspace: String) -> CurrentScope`.
  - UI `commands.identityGet()`, `commands.profileSet(name, email)`, `commands.scopeGet()`, `commands.scopeSet(account, workspace)`.
  - zod `T.Identity` and `T.CurrentScope`.
  - Query keys `keys.identity` and `keys.scope`. The UI invalidates `keys.scope` itself after `scopeSet`, so no runtime event is needed.

- [ ] **Step 1: Write the failing tests:**
  - `commands.test.ts`: each new command calls the right name with the right args and parses the fixture.
  - `ProfileSection.test.tsx`:
    - The heading is "Profile".
    - It shows the fingerprint and a **Copy my Dots ID** button that copies `public_key`.
    - Saving calls `profileSet("Asha Rao", "asha@acme.com")`.
    - A core error message is shown under the form.
  - `Accounts.test.tsx`: the existing tests with the legend and region named "Accounts".
- [ ] **Step 2: Run and confirm failure.** Run `cd ui && bun run test src/api src/screens/ProfileSection.test.tsx src/screens/Accounts.test.tsx`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands and contract samples, regenerate the fixtures (`UPDATE_CONTRACT=1 cargo test -p dots-app contract`), then build the UI. `ProfileSection` follows the fieldset and legend style of `Accounts.tsx` and uses `Field`, `Input` and `Button` from `components`.
- [ ] **Step 4: Run all tests.** Run `cargo test -p dots-app && cd ui && bun run test`. Expected: PASS, with no test still looking for "Organisations".
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): Profile in Settings, Organisations shown as Accounts"`

### Task 5: Account switcher and scoped Home

**Files:**
- Create: `ui/src/app/AccountSwitcher.tsx`, `ui/src/app/AccountSwitcher.test.tsx`, `ui/src/lib/scope.ts`, `ui/src/lib/scope.test.ts`
- Modify:
  - `ui/src/app/Layout.tsx:72` (switcher first in the rail)
  - `ui/src/screens/home/HomeScreen.tsx:33-34` (filter rows)
  - the new-dot default in `ui/src/screens/dot-editor/DotOrg.tsx` / `DotEditorScreen.tsx` (a new dot starts in the current account; an existing dot is unchanged)

**Interfaces:**
- Consumes: `commands.scopeGet/scopeSet`, `commands.identityGet`, `commands.orgList`, and `commands.dotOverview` (`DotOverview.dot.org_id`, `.needs_you`).
- Produces:
  - `useCurrentScope(): CurrentScope | undefined`.
  - `inScope(dot: { org_id: string | null }, scope: CurrentScope): boolean`. Personal matches `org_id === null`.
  - `waitingByAccount(rows: DotOverview[]): Map<string, number>`, keyed by `"personal"` or the org id, summing `needs_you`.

The switcher's behaviour:
- A 36px round button showing the profile initials (up to 2 from `name`; "?" when blank). It has a small badge with the first letter of the current account, and `aria-label="Account: <name>"`.
- Clicking opens a `role="menu"` list anchored to the rail's right edge:
  - **Personal**, then each joined org in `orgList` order. Each item shows its name, a check on the current one, its status text when it is not plain active (reuse the org status labels in `Accounts.tsx`), and a needs-colour count from `waitingByAccount` when greater than 0.
  - A separator, then **Join an account** and **Manage accounts**. Both navigate to `/settings#accounts`.
- Choosing an item calls `scopeSet(id, "default")`, invalidates `keys.scope`, and closes the menu. Escape and clicking outside also close it.

- [ ] **Step 1: Write the failing tests:**
  - `scope.test.ts`: `inScope` for Personal and org dots, and `waitingByAccount` sums.
  - `AccountSwitcher.test.tsx`:
    - The initials render.
    - The menu lists Personal plus fixture orgs.
    - Picking Acme calls `scopeSet("acme-eng", "default")`.
    - A non-current account with `needs_you` 2 shows "2".
    - Escape closes the menu.
  - Home tests (extend the existing ones): with scope Acme, only Acme dots are listed. With scope Personal, only `org_id === null` dots are listed. While the scope is loading, every dot is listed.
  - `Layout.test.tsx`: the Inbox count still counts approvals from dots of a non-current account.
- [ ] **Step 2: Run and confirm failure.** Run `cd ui && bun run test src/app src/lib/scope.test.ts src/screens/home`. Expected: FAIL.
- [ ] **Step 3: Implement** the switcher, the hook and the Home filter.
- [ ] **Step 4: Run all tests.** Run `cd ui && bun run test`, plus the UI's typecheck script from `ui/package.json`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(accounts): account switcher in the rail, Home follows it"`

### Task 6: Docs and smoke

**Files:**
- Create: `docs/accounts.md` (for members: your Dots ID and Profile, switching accounts, joining. Members and roles are marked "coming next"), `docs/verification/2026-10-28-accounts-phase1-smoke.md`
- Modify: `release-notes.md` (one line in the existing style)

- [ ] **Step 1: Write the smoke doc** with the following manual checks against the dev app:
  1. The first start shows a fingerprint in Settings › Profile.
  2. The fingerprint survives a restart.
  3. Saving a name changes the rail initials.
  4. Joining a test org adds it to the switcher.
  5. Switching filters Home.
  6. A new dot defaults to the selected account.
  7. Leaving the selected account returns to Personal.
  8. An approval in a non-selected account still shows in the Inbox and on the switcher.
- [ ] **Step 2: Run the full gates.** Run `cargo test --workspace && cargo clippy --workspace -- -D warnings && cd ui && bun run test`. Expected: all PASS.
- [ ] **Step 3: Commit.** `git commit -m "docs(accounts): member guide and phase 1 smoke"`
