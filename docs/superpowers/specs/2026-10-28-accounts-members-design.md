# Accounts, members, roles and workspaces: design

## 1. Intent

People on different computers should be able to share a Dots **account**, the way people share a Cloudflare account or a Claude Console organisation: everyone has their own identity, belongs to one or more accounts with a role, and switches between them. A Claude Console-style layer of **workspaces** and **API keys** sits underneath.

The owner's words and answers (2026-10-28):

- "reference Cloudflare user model and Cloudflare workspaces ... so anyone using our app has pre decided notions like we have on the Cloudflare user model".
- Scope, in their order: first Cloudflare's "accounts, members, roles and permissions, and switching between the accounts", then Claude Console's "organisation, workspaces, members and api keys". They asked whether the two should be built one after the other or are separate, and for a better order. My answer, which they accepted: they are one model (Cloudflare Account = Claude Organisation), so they are built as one system in phases (§13).
- People in an account are on **different computers**.
- The owner chose **no Dots server** ("option B"), building on the vault's existing Organisations: join codes, admin-signed policy, a shared source.
- Install stays **open**: "users should have an open experience of installing the dots. Once you install it, there should be something on top of it ... like a login credential". Releases and auto-update keep working as today.
- The full flow was played back (Dots ID on first launch, accounts replace Organisations, a signed member list, the two-code invite, Cloudflare roles, a switcher, then workspaces and API keys, and the stated limit that roles are enforced by each person's Dots). The owner answered "looks right ... build this".

Everything else is marked as my call.

Success: an admin can invite a colleague on another PC, approve them, give them a role, move them between workspaces and remove them, and every Dots in the account follows within one refresh, with no server run by us.

## 2. Goals, non-goals, anti-goals

**Goals**

- A **Dots ID** per install: a key pair plus name and email, made on first use. It is what "you" means in Dots.
- **Accounts**: today's vault Organisations become accounts. **Personal** is an account too, always present, owned by you.
- A signed **member list** (the roster) per account, stored in the account's source beside the policy: members, their Dots IDs, roles and workspaces.
- **Invites** that work with no server: an invite code from the admin, a request code back, an approval.
- **Roles** modelled on Cloudflare's account roles: Owner, Admin, Member, Viewer. Dots enforces them.
- An **account switcher** in the main rail. Lists and creation follow the selected account and workspace.
- **Workspaces** inside every account, each with a Default workspace, and workspace roles, after Claude Console.
- **API keys** per workspace for Dots' local server.
- `dots-admin` commands for the owner's key and roster, and an updated admin guide.

**Non-goals (my call)**

- A hosted Dots service, central sign-in or password login. The owner chose option B.
- **Syncing dots, teams, runs, library items or memories between computers.** Each Dots keeps its own dots. What an account shares is identity, membership, roles, workspaces, policy and secrets. A shared account library is a later project (§13, phase 6).
- Gating the installer download.
- SSO/OIDC, 2FA, and permissions on single resources (Cloudflare's domain-scoped roles). These come later if needed.
- macOS and Linux key storage (the same as the vault).

**Anti-goals (my call)**

- A person who was removed still using the account's secrets after their next refresh.
- A member list that can be edited on someone's own PC and still be believed.
- Joining that forces people to hand-edit JSON or run a command line. The admin may use `dots-admin`; members never need to.
- Breaking today's org setups. An org with no roster keeps working exactly as it does now (§6.6).
- A switcher that hides something that needs you: Needs you and notifications always show every account.

## 3. Facts the design rests on

- Dots is a single-user Windows desktop app. State lives in SQLite (`crates/dots-core/migrations`, latest `0017_usage.sql`). The machine identity is `install_id` (`store/settings.rs:15`). There is no notion of a human user anywhere. `ChatGptAccount {name, email}` (`chatgpt.rs:67`) is display-only.
- Orgs (spec `2026-10-21-env-vault-design.md`, §6) live in `vault/org/`:
  - `JoinCode` (`join.rs:25`) and `OrgPolicy` (`policy.rs:101`). The policy is minisign-signed and verified in `signed.rs:45`, with a rollback check at `policy.rs:234`.
  - Each org has a `Source` with `fetch` only (`sources/mod.rs:44`): OpenBao, Infisical or SOPS.
  - The `Orgs` service is `service.rs:215`. The offline copy is DPAPI-protected (`offline.rs`), and the refresh loop is `timers.rs`.
- Scope today: `dots.org_id` (NULL means Personal), `DotScope` (`vault/resolve.rs:20`), MCP catalogs per scope. Teams, apps, library items, memories, watchers and usage have no scope column.
- The UI shell is `ui/src/app/Layout.tsx`, with a 46px icon rail `<nav aria-label="Main">` and no top bar. Organisations is a section of Settings (`SettingsScreen.tsx:200`, `Organisations.tsx`).
- The local server (`server/mod.rs`) binds `127.0.0.1` and checks only that the peer is loopback, with no Host or Origin check. It uses per-dot webhook tokens and per-run MCP secrets.
- `dots-admin` (`crates/dots-admin`) has `keygen`, `policy new|check|sign|verify` and `join-code`, built on `dots_core::vault::org` types.

## 4. Words

| Dots | Cloudflare | Claude Console |
|---|---|---|
| Dots ID | User | User |
| Account | Account | Organization |
| Member | Member | Member |
| Role (Owner, Admin, Member, Viewer) | Super Administrator, Administrator, member roles, Administrator Read Only | Admin, Developer, User |
| Workspace | (none) | Workspace |
| API key | API token | API key |

In the UI, "Organisations" becomes **Accounts**. In code, the `orgs` table and `vault::org` module keep their names (my call: renaming would touch the whole vault for no behaviour change). New code lives in `crates/dots-core/src/accounts/`.

## 5. Dots ID

- **Made** silently at start-up (store upgrade for this migration) if missing: an Ed25519 key pair in **minisign** format, the same format the policy already uses. `dots-core` gains the pure-Rust `minisign` crate for signing (it already uses `minisign-verify`).
- **Secret key**: Credential Manager, target `Dots/<install_id>/identity`, with no password. It is protected by the Windows login like every other vault secret (my call).
- **Public key, name, email**: `settings` table keys `identity_public_key`, `profile_name`, `profile_email`.
- **Fingerprint**: the existing `key_fingerprint` of the public key, shown as the short ID people compare out loud.
- **Profile** in Settings: name, email, fingerprint, **Copy my Dots ID**. Name and email are required before joining or inviting. The Join flow asks for them inline if they are empty.
- **More than one PC**: each install has its own Dots ID. A member has a list of keys (§6.2). A person adding a second PC goes through the same invite flow with the same email, and approval adds the key to their existing member entry.
- **Lost PC**: the admin removes that key from the member. There is no recovery of a lost secret key.

## 6. Accounts, roster and roles

### 6.1 Trust

- The join code's `policy_key` stays the account's **root key**, held by the owner (made with `dots-admin keygen`).
- The policy is still signed by the root key only. Policy **v2** adds `admins: [<minisign public key>, ...]`: the Dots IDs allowed to sign the roster.
- The **roster** is valid when signed by the root key or by a key in the current policy's `admins`. Making someone an Admin in the roster takes effect for signing only once the owner adds their key to `admins` and re-signs the policy. The Members page says so (my call: keeps a stolen admin PC from promoting others without the owner).
- **The owner signs in Dots with their own Dots ID**, not the root key. The root key stays with `dots-admin`, offline. `dots-admin roster new --owner-key` and `policy new --admin` put the owner's Dots ID in `admins` from the start (my call). Dots never imports the root key.
- A signer check happens **when a roster is accepted**. Removing a key from `admins` later does not invalidate a roster already accepted; only new rosters must be signed by a current admin (my call: a routine policy change must not cut everyone off).

### 6.2 Roster v1

Stored next to the policy as `DOTS_ROSTER` (§6.5). Signed text, minisign, like the policy.

```json
{
  "v": 1,
  "org_id": "acme-eng",
  "issued_at": "2026-10-28T09:00:00Z",
  "seq": 7,
  "workspaces": [
    { "id": "default", "name": "Default" },
    { "id": "payments", "name": "Payments", "use": { "allow": ["STRIPE_*"], "deny": [] } }
  ],
  "members": [
    {
      "id": "01JB...ULID",
      "name": "Asha Rao",
      "email": "asha@acme.com",
      "role": "admin",
      "keys": ["RWQ...minisign public key..."],
      "workspaces": { "default": "member", "payments": "admin" },
      "added_at": "2026-10-28T09:00:00Z"
    }
  ]
}
```

Validation (fails closed, like the policy):

- `org_id` must match the join code.
- `seq` must be greater than the last accepted roster's (`orgs.last_roster_seq`, rollback check). `seq` is used rather than `issued_at`, because rosters are signed on different PCs whose clocks may differ (my call). `issued_at` is for display only.
- **Concurrent edits**: before publishing, Dots fetches the roster again. If its `seq` is no longer the one the edit started from, Dots refuses to publish, reloads, and asks the admin to redo the change on the new list. A new roster's `seq` is always the base `seq` plus 1.
- No unknown top-level fields.
- Roles must be one of `owner|admin|member|viewer`, and workspace roles one of `admin|member|viewer`.
- Every key appears in at most one member.
- Workspace ids match `^[a-z0-9][a-z0-9-]{0,31}$`, and `default` must exist.
- Every workspace a member names must exist.
- At least one `owner`.

### 6.3 Roles and permissions

Account roles, enforced in `dots-core` (`accounts::can(role, ws_role, Permission) -> bool`), never only in the UI:

| Permission | Viewer | Member | Admin | Owner |
|---|---|---|---|---|
| See account dots, runs, teams, entry names on this PC | yes | yes | yes | yes |
| Create, edit, delete account dots and teams | | yes | yes | yes |
| Run account dots (manual, schedule, watcher, webhook) | | yes | yes | yes |
| Answer approvals for account dots | | yes | yes | yes |
| Grant account entries to dots (within policy `use`) | | yes | yes | yes |
| Add MCP servers in the account's scope | | yes | yes | yes |
| Create API keys in a workspace | | yes (own) | yes | yes |
| Invite, approve, change roles, remove members | | | yes | yes |
| Create, rename, delete workspaces | | | yes | yes |
| Sign the roster | | | if in policy `admins` | yes (root key) |
| Change the policy, root key, `admins` | | | | yes (`dots-admin`) |

**What "account dots" means**: dots aren't synced, so the checks apply to every dot on this PC whose `org_id` is that account, whoever created it. A person moved down to Viewer keeps seeing their account dots, read-only. Their schedules and watchers are held (below), and nothing is deleted.

Workspace roles narrow access inside a workspace. A workspace `viewer` gets Viewer rights for that workspace's dots. A workspace `admin` can manage that workspace's API keys on their PC. Changing anyone's role, account or workspace, means signing the roster, so it needs an account Admin or Owner (my call: no delegated signing). Account Owner and Admin are workspace admins everywhere. A member with no entry for a workspace does not see it.

When a person loses a permission (role change or removal), the next refresh applies it. Scheduled and watcher runs of account dots they can no longer run are held with **Needs attention: "Acme: your role no longer allows running dots"** instead of failing silently. Personal is always Owner.

### 6.4 Membership status on this PC

Stored in new columns on `orgs`:

- `membership`: one of `legacy|waiting|active|removed`.
- `my_member_id`, `my_role`, `my_workspaces` (JSON).
- `last_roster_seq`.
- `pending_request` (JSON: the request code and invite id while waiting).

The last accepted roster is kept in the offline copy, beside the policy.

After every fetch:

- **No roster in the source and policy `roster` is `"off"`**: `legacy` (§6.6).
- **Roster valid, this Dots ID in it**: `active`, with role and workspaces updated.
- **Roster valid, this ID not in it, and this PC was `legacy` or `waiting`**: `waiting`. Nothing in the account resolves, no account dots run, and nothing is wiped. The account shows **Request access**, which makes a request code with no invite (§7).
- **Roster valid, this ID not in it, and this PC was `active`**: `removed`. The effect is the same as `Denied` today: wipe the offline copy and in-memory values, disable the account's dots, label the account "Removed from Acme". Only an active member can become removed.
- **Roster missing or invalid when one was accepted before**: keep the last accepted roster and membership, show the error, and expire with the offline copy. This matches how an `Invalid` policy is handled today.
- **Source `Denied` while `waiting`** (SOPS before the admin adds the age recipient): stay `waiting`, not revoked.

**Moving a legacy org to rosters**, in the order the admin guide gives:

1. Members click **Request access** and send their request codes.
2. The owner builds the roster with `dots-admin roster add-request <code>` in phase 2, or Approve in Dots from phase 3.
3. The owner publishes the roster.
4. Only then does the owner switch the policy to `"required"`.

Anyone missed lands in `waiting`, not `removed`.

### 6.5 Where the roster lives, and publishing

The `Source` trait gains:

```rust
/// Signed roster text, or None if the source has none.
// returned in Fetched alongside `policy`
struct Fetched { entries, policy: String, roster: Option<String> }

/// Write the signed roster. Sources that can't write return Unsupported.
async fn publish_roster(&self, creds: &SourceCreds, signed: &str) -> Result<(), PublishError>;
```

| Source | Read | Publish from Dots |
|---|---|---|
| OpenBao | key `DOTS_ROSTER` at `<mount>/data/<path>/roster` | yes, KV v2 write, if the admin's token may write that path |
| Infisical | secret `DOTS_ROSTER` in `<secret path>/policy` | yes, if the admin's machine identity may write |
| SOPS | field `roster` in the file | no. Dots shows **Copy signed roster** and the admin guide covers putting it in with `sops` |

A failed publish (no write right, offline) leaves the change as a **draft** on the admin's PC (table `roster_drafts`) with a clear error and a **Retry**. Exact HTTP calls are the builder's to confirm against each service's current docs.

### 6.6 Old setups

Policy v1 stays valid and means `roster: "off"`: legacy orgs behave exactly as today, and everyone with source access acts as Member. Policy v2 adds:

- `"roster": "off" | "required"`.
- `"admins": [...]`.

Under `required`, a missing or invalid roster leaves the account `waiting` (never assumes access). Old Dots versions reject v2 policies (unknown fields fail closed), so the admin guide says to update every Dots before switching to v2.

## 7. Invites

Two codes, no server (owner approved).

1. **Admin › Invite** (Accounts › Acme › Members): enter email, account role, and workspaces with roles. Dots stores the invite on the admin's PC (table `account_invites`: id, org_id, email, role, workspaces, created_at, expires_at, used_at) and shows an **invite code**. The code is the join code v2 plus `invite: { id, role, expires_at }` (base64url, also a `.dotsorg` file). Default expiry is 7 days (my call). The role in the code is for display only; the roster is the truth.
2. **New person**: install Dots → Accounts › **Join** → paste the code → preview (account name, root-key fingerprint, offered role, expiry) → enter name and email if missing → give their source credential as today (none needed for SOPS) → Dots shows a **request code**:
   `dotsreq_` + base64url of `{ v, org_id, invite_id, key, name, email, age_recipient?, created_at }` plus a minisign signature by the person's Dots ID (proves they hold the key). The account shows **Waiting for approval**, and Dots keeps fetching on its normal refresh.
3. **Admin › Approve request**: paste the code. Dots checks:
   - the signature,
   - the `org_id`,
   - that the invite exists on this PC, is unused and unexpired,
   - that the email matches the invite.

   It then shows name, email, fingerprint and role. On **Approve**, Dots adds the member (or adds the key to the member with that email), marks the invite used, signs the roster with the admin's Dots ID, and publishes. For SOPS it also shows the person's age recipient to add.
4. The new person's next fetch finds their key, and they are `active`.

Invites live only on the PC that made them, so another admin can't approve someone else's invite (my call: avoids publishing invites). Admins can list and revoke their open invites. **Approve without an invite** is allowed for Owner/Admin after a confirm ("This person has no invite from this PC"). It covers lost invite codes and **Request access** codes from legacy members (§6.4), whose `invite_id` is empty and who pick a role at approval.

Other member actions (Admin/Owner, each signs and publishes the roster):

- change role
- change workspaces
- remove a member
- remove one key

Removing someone also shows a reminder to revoke their source credential. The roster stops Dots honouring them, but only the source cuts off a determined person (§11).

## 8. Workspaces

- Every account has a `default` workspace. Personal has workspaces too, kept locally (Personal never has a roster).
- New table `workspaces (scope, id, name, created_at)`, with `scope` either `personal` or an org id. Account workspaces are mirrored from the roster on each fetch. Personal workspaces are edited directly.
- `dots.workspace_id TEXT NOT NULL DEFAULT 'default'`, set at creation. Dots can be moved between workspaces of the same account by someone with Member rights in both (unlike `org_id`, which stays fixed).
- **What gets a scope column** (my call): only `dots` and `teams`. Runs, memories, watchers, usage rows and check-ins take their scope from their dot. Library items, apps and MCP servers stay global, as today. MCP catalogs and confirmed origins stay per account (as now), not per workspace.
- `teams` gains `org_id` and `workspace_id`. A team's dots must all be in its account and workspace. The migration rule: a team whose dots are all in one org takes that org, otherwise Personal; dots outside the team's scope are listed in Needs attention for the person to fix (my call).
- A workspace's optional `use` narrows the policy's `use` for its dots. Deny rules are combined, and allow rules must pass both.
- Usage limits (`limits.rs`, `usage.rs`) stay per Claude subscription, not per workspace (my call: the limit belongs to the person's Claude plan, not the account).

## 9. Switcher and screens

- **Switcher**: the top of the main rail in `Layout.tsx` gets an avatar button (initials from profile name, with a small badge showing the current account's initial). It opens a popover:
  - Personal, then each account with role badge and status (Waiting, Offline, Removed).
  - The workspaces of the selected account.
  - **Join an account** and **Manage accounts**.
- The selection persists in settings (`current_scope`: `{ "account": "personal" | <org_id>, "workspace": <id> }`).
- **What follows the selection**:
  - Home's dot list, the Dots list, Teams, and run lists and Usage (through each run's dot) filter to the current account and workspace. The Library and Apps screens are not filtered.
  - Creating a dot or team puts it there. The editor's org picker (`DotOrg.tsx`) defaults to it and still allows another account the person can create in.
  - Needs you, notifications and the tray always show every account, each item tagged with its account (anti-goal).
- **Settings › Profile**: §5.
- **Settings › Accounts** (replaces Organisations) has a list and **Join**. Each account page has tabs:
  - **Overview**: status, my role, source, refresh, update join code, leave. These are today's org actions.
  - **Members**: list with role, workspaces, keys and fingerprints. Admins also get Invite, open invites, Approve request, role and workspace changes, remove, drafts and Retry publish.
  - **Workspaces**: list. Admins can create, rename and delete. Deleting moves its dots to Default after a confirm.
  - **API keys**: §10.
  - **Entries**: today's org entries view.
- Every disabled control for lack of a role says which role is needed.

## 10. API keys

- Per workspace, on this PC only (not shared through the roster). Claude Console keys are org-wide because it has a server. Here a key unlocks this PC's local server only.
- Format: `dots_sk_` + 32 random bytes base62. It is shown once.
- Stored in table `api_keys (id, scope, workspace_id, name, prefix, hash, created_at, last_used_at, revoked_at)`, where `hash` is SHA-256 of the key and `prefix` is the first 12 characters for display.
- New routes on the existing server, `Authorization: Bearer dots_sk_...`, limited to the key's workspace:
  - `GET /v1/dots`
  - `POST /v1/dots/{id}/trigger` (same body as the webhook)
  - `GET /v1/runs/{id}`

  Each checks that the key is not revoked and that the person still has run permission in that account.
- Hardening at the same time: every server route checks that the `Host` header is `127.0.0.1:<port>` or `localhost:<port>`, against DNS rebinding.
- Existing per-dot webhook tokens and MCP run secrets stay as they are.

## 11. Threat model

Protected against:

- A member editing the roster or policy on their own PC (both are signed).
- An old roster being replayed (rollback check).
- A removed member using account secrets after their next refresh (offline copy wiped).
- A stolen admin PC promoting others to admin without the owner (§6.1).
- Request codes forged for someone else's key (signed by that key).
- Web pages calling the local API (Host check plus Bearer key).

Not protected against (stated in the admin guide):

- A person patching their own Dots to ignore their role. Roles stop mistakes and honest use. The real cut-off is revoking the source credential, the same trade-off as the vault (vault spec §4).
- A removed person's PC that stays offline. It keeps the offline copy until `offline_hours`.
- A stolen PC while its Windows login is unlocked.

## 12. dots-admin and docs

New `dots-admin` commands (my call on names):

- `policy` v2 fields, plus `policy admins add|remove <dots-id>`.
- `roster new --org-id <id> --owner-name --owner-email --owner-key <dots-id>`.
- `roster check|sign|verify <file>` (`sign` takes the root key).
- `roster add-request <file> <request-code> --role <role>`.
- `roster set-role|remove <file> <email>`.
- `request inspect <code>`.
- `join-code ... --invite` is not needed, because invites are made in Dots.

Docs:

- `docs/vault-admin.md` becomes the accounts admin guide, covering:
  - setting up an account with a roster,
  - the owner key,
  - adding admins,
  - the SOPS roster steps,
  - upgrading policy v1 to v2,
  - cutting someone off in each source.
- `docs/accounts.md`: a short guide for members (join, switch, roles).

## 13. Build phases

Each phase ships on its own, with tests, a smoke doc in `docs/verification/` and a release-note line.

1. **Dots ID and switcher**:
   - identity key and Profile,
   - Organisations renamed to Accounts,
   - switcher with Personal plus existing orgs,
   - `current_scope` filtering for dots and home,
   - `teams.org_id`.
2. **Roster and roles**:
   - policy v2 and roster v1 parse, verify and rollback,
   - membership status,
   - `can()` enforced across runtime commands, runs, approvals, grants and MCP scope,
   - held runs,
   - `dots-admin` roster and policy commands.

   Rosters are signed with `dots-admin` only in this phase.
3. **Invites and Members page**:
   - invite code, request code and approve,
   - signing with the admin's Dots ID,
   - `publish_roster` for OpenBao and Infisical, SOPS copy flow, drafts,
   - member edits.
4. **Workspaces**:
   - table and mirror,
   - `dots.workspace_id`, team workspace,
   - workspace roles and `use`,
   - switcher workspace level,
   - Workspaces tab.
5. **API keys**: keys, `/v1` routes, Host check, API keys tab.
6. **Later, separate designs**: a shared account library (dots and teams templates published to the source), audit of membership changes, SSO.

## 14. Testing

- **Core unit tests**:
  - roster parse and validation (each rule in §6.2), signature by root versus admin versus stranger,
  - rollback,
  - request code sign, verify and tamper,
  - invite expiry and single use,
  - the `can()` matrix as a table test,
  - membership transitions for each fetch outcome (§6.4).
- **Source tests** with the existing fakes (`fake_openbao`, `fake_infisical`, SOPS fixtures): roster read, publish success, publish denied leaving a draft.
- **Runtime tests**:
  - a viewer can't create, run or answer for account dots,
  - a removed member's dots are disabled and the offline copy is gone,
  - a held schedule shows Needs attention,
  - legacy orgs are unchanged.
- **Server tests**: API key auth, revoked key, wrong workspace, Host check.
- **UI tests** (vitest, existing style):
  - switcher,
  - filters,
  - Join with invite through to waiting and active,
  - Members actions by role,
  - disabled controls naming the role,
  - API key shown once.
- **Smoke doc per phase**, including a two-install manual test: two data folders on one PC (dev and installed `install_id`s already separate them) sharing a fake OpenBao.

## 15. Left to the builder

- Exact Credential Manager and settings key names beyond those given, and module layout inside `accounts/`.
- The OpenBao and Infisical write calls, checked against current docs.
- Popover and badge visuals within Deep Teal.
- Whether the `/v1` routes return run events or only status (status at minimum).
- Migration numbering (0018 onward) and how `finish_0018` creates the identity.
