# Env vault: design

## 1. Intent

Dots is going to be deployed across the owner's organisations. Today any dot can read every variable in Dots' environment, and some credentials sit in plain text. This design adds an env variable library with a vault for credentials, so that secrets are stored encrypted, reach only the dot and the tool they were granted to, stay out of history and logs, and stay under an organisation admin's control.

The owner's words and answers (2026-10-21):

- "add a env variabel library with a vault for credentails".
- Why: "this is required for security as this system will be deployed in our org".
- Reference: "you can refer to the company os". That is the open-source Cloudflare OS at `C:\Users\aksha\cloudflare-os` (§3.2).
- Who holds keys: "hybrid version where admin can still has control". People add their own keys, and admins hand out shared keys and keep control.
- Admin control covers both cases offered: cutting off someone who leaves, and rotating a leaked key once for everyone ("yes").
- Services: "configure so that i may add any of the services as we have multipel orgs".
- Multiple orgs means one Dots belongs to several orgs at once, and they must never mix ("b"). Every dot belongs to exactly one org (or Personal), chosen when the dot is created and fixed after ("yes").
- First services: "anything that open source and free". The starting set was played back and confirmed ("yes"): OpenBao (also covers HashiCorp Vault), Infisical, and a SOPS + age encrypted file. Personal keys go in Windows Credential Manager.
- Offline: "keep running on an encrypted local copy of the last secrets they fetched".
- Audit: "admin needs to see which dot used which secret", read in "a built-in dashboard". The dashboard needs a central server, so the work was split into two projects with the owner's "yes" (§2). This document is project 1.
- Asked what would make the vault a failure even if it worked, the owner answered "all good non of these" to the three examples offered. The anti-goals below are my call.
- The full proposal (Env tab, Orgs page, per-dot Variables, the run-time rules, migration, the two extra fixes) was played back. The owner answered "ok".

Everything else is marked as my call.

Success: Dots can be handed to a security team that asks "where do our keys live, who can use them, and how do we cut someone off?", and every answer is something Dots enforces rather than a convention people have to follow.

## 2. Goals, non-goals, anti-goals

**Goals**

- An **Env** tab in the Library for plain variables and secrets, grouped by Personal and by org.
- **Personal** entries stored on this PC: secrets in Windows Credential Manager, plain values in the database.
- **Orgs**: a person joins any number of orgs. Each org has one **source** (OpenBao/Vault, Infisical, or a SOPS + age file) and a **signed policy** written by its admin.
- **Sources are adapters** behind one trait, so adding Conjur, Passbolt or another service later touches nothing else.
- **Grants**: a dot gets a variable only when it is granted to that dot for a named consumer, such as one MCP server or the shell. Nothing is granted by default.
- **Isolation**: a dot resolves only its own org's entries, plus personal entries if the org's policy allows them. An Org A value never resolves in an Org B dot, even when the names are the same.
- **No inherited environment**: child processes (the `claude` CLI, MCP servers, the local engine's shell tool) start from a minimal base environment plus their grants.
- **Redaction**: secret values are scrubbed from run events, approvals, webhook payloads, logs and the live UI stream, in every common spelling.
- **Offline copy**: each org's last fetched secrets are kept encrypted with DPAPI and used until the policy's expiry.
- **Audit records**: every secret use is recorded (never the value), queued locally, and sent to the org's audit endpoint in a documented format.
- **Migration** of today's setups, and two related fixes: the webhook token and the gateway file (§11).
- **Admin tooling**: a `dots-admin` command-line helper and an admin guide (§12). The owner chose to include this in project 1 ("yes").

**Non-goals (this project)**

- The admin server and dashboard (project 2). Project 1 only fixes the audit record format and the endpoint protocol that project 2 will receive.
- Writing to an org source from Dots. Org entries are read-only in Dots; admins manage them in their service.
- OAuth "connect account" flows, like Cloudflare OS gatekeepers. Dots injects values; it does not run per-vendor OAuth.
- Proxying API calls so that even the consumer never sees the value. A granted MCP server or shell does receive the value (§8).
- macOS and Linux keychains. Dots ships on Windows; the store is behind a trait, so they can be added later.
- Auth for the local engine's endpoint, and secrets in skills, plugins' own files or context folders.
- Protecting against malware running as the user, or against the user deliberately extracting their own granted secrets (§4).

**Anti-goals (my call)**

- Any screen, command, export, log or run event that shows a stored secret value after it is saved.
- A secret readable by a dot, consumer or org it was not granted to.
- A secret written in plain text anywhere on disk: database, backups, `.mcp.json` copies, the gateway file, the offline copy, the audit queue.
- A run failing for a variable reason (missing, expired, revoked, policy) without saying which variable, which org and why.
- So much friction that people paste keys into MCP configs anyway. The import and the MCP server picker (§9) exist to prevent this.

## 3. Facts the design rests on

### 3.1 Dots today

- `${VAR}` in an MCP server's `env`, `args`, `cwd`, `url` and `headers` is expanded when the server starts (`crates/dots-core/src/mcp/vars.rs`, `McpServerConfig::expanded(&dyn Fn(&str) -> Option<String>)`, `variables()`). An unset variable gives "X is not set".
- The only resolver is `library_service::process_env` (`std::env::var`), used at `run_tools.rs:527` (runs), `library_service.rs:147` (probe/check), `:475` (missing variables), `:717` (`environment_variables`), and its copy at `apps/dots-app/src/confirm.rs:98`.
- Dots runs every MCP server itself (the gateway, MCP spec 2026-10-15). Stdio servers are spawned at `mcp/stdio.rs:55-56` with `.envs(launch.env)` on top of the inherited environment. HTTP servers get expanded headers at `mcp/mod.rs:184`.
- The `claude` CLI is spawned at `engine/claude/mod.rs:91-104` with `MCP_TOOL_TIMEOUT` plus test-only `claude_env`. The same applies to `once.rs:89` (skill drafts) and the version probe at `mod.rs:24-33`. The local engine's shell tool spawns at `engine/local/tools/shell.rs:156`. **No spawn calls `env_clear`**, so every child inherits Dots' full environment.
- Literal values in `env` and `headers` are stored as written in `library_items.config` (`migrations/0006_library.sql`), and copied verbatim from plugin `.mcp.json` (`kit/plugin.rs:147,155`). `looks_secret` (`library.rs:485`) warns about them; `store/upgrade.rs:161-233` (`finish_0006`) once turned inline values into `${VAR}`.
- `dots.webhook_token` is plain text (`0001_init.sql:12`), shown and copyable in `WebhookPanel.tsx`, and accepted as `Bearer` or `?token=` (`server/webhook.rs:20-46`).
- The per-run gateway file `data_dir/mcp/mcp-<run>.json` holds a bearer token; `write_private` (`engine/claude/mod.rs:70`) sets `0600` on Unix only.
- Unredacted text is stored in `run_events.data`, `approvals.input` and `runs.payload` (`0001_init.sql`), and MCP stderr is logged at info level (`mcp/stdio.rs:222`).
- Update backups are `VACUUM INTO` copies under `data_dir/backups`.
- There are no crypto or keyring dependencies. `windows-sys 0.61.2` has `Win32_Security`, `Win32_System_JobObjects` and `Win32_System_Threading`. `minisign-verify` is already in `Cargo.lock` through the updater.
- A Library kind is modelled end-to-end through `LibraryKind`/`LibraryConfig` (`library.rs`), `store/library.rs`, `LibraryService`, the Tauri commands in `apps/dots-app/src/commands.rs` (registered in `lib.rs`), `ui/src/api/{commands,types}.ts`, a tab under `ui/src/screens/library/`, the per-dot picker `ui/src/screens/dot-editor/ToolsAndSkills.tsx`, and the contract samples (`apps/dots-app/src/contract.rs` → `ui/src/api/__fixtures__/contract.json`).
- Subscription rule: Dots spawns only the official `claude` CLI and never reads, copies or sends its credentials.

### 3.2 What the company OS does, and what Dots takes from it

Cloudflare OS has no general vault. Each credential lives inside the gatekeeper that uses it, agents get handles rather than values, and admins disable integrations through one checkpoint (`packages/workshop-shared/src/gatekeeper.ts`, `packages/workshop-backend/src/admin-config.ts`, `AGENTS.md:16,45-48`). Dots takes from it:

- **The value stays in the component that uses it.** In Dots that is the Rust process that resolves and injects at spawn time. The agent gets a name, never the value, unless the shell is granted (§8).
- **Write-only secrets.** Listings return metadata only (`user.ts:528-547`).
- **Status per entry**: connected, expired, revoked (`gatekeepers.tsx`).
- **One checkpoint** where org policy is applied (`getGatekeeperClassFor`); in Dots this is the resolver (§8).
- **Redact every encoding before truncating** (`packages/mcp-shared/src/util.ts:17-58`).
- **Never log secrets, headers or bodies** (`AGENTS.md:119-120`).

Unlike Cloudflare OS, Dots needs its own encryption at rest, because it runs on laptops rather than in managed storage.

## 4. Threat model

**Protected against:**

- Secrets in plain text on disk: the database, update backups, config copies, logs.
- A dot, or the agent inside it, reading a secret that wasn't granted to it, including through `printenv` in the shell.
- One org's secrets reaching another org's dot, or another org's admin.
- Secrets copied into run history, approvals, the live UI or logs by a tool or server that echoes them.
- Another Windows account on the same PC. Credential Manager and DPAPI are per user.
- A copied disk without the user's Windows login. DPAPI ties the data to it.
- A user editing org policy locally. The policy is signed by the admin's key (§6).

**Not protected against (stated in the admin guide):**

- Malware or tools running as the same Windows user. They can call DPAPI and Credential Manager too.
- The user deliberately extracting a secret granted to them, for example by patching Dots or attaching a debugger. Policy enforcement stops mistakes and honest use, not a determined insider with admin rights on their own laptop. Revocation at the source and the source's own audit log are the admin's controls there.
- A granted consumer misusing the value: an MCP server sending it elsewhere, or a shell-granted agent `curl`ing it out. Approvals still gate tool calls. Shell grants are opt-in, can be forbidden by policy (§6), and carry a warning (§9).

## 5. Entries, scopes and storage

### 5.1 Entries

An entry has a **scope** (Personal, or one org), a **name** (`^[A-Za-z_][A-Za-z0-9_]{0,127}$`, the existing `is_var_name` with a length cap), and a **kind**:

- **Plain**: config such as `LOG_LEVEL`. The value is shown and editable.
- **Secret**: write-only. After saving, Dots shows only the name, scope, source, status and when it was last updated. Personal secrets can be replaced or deleted, never revealed. There is no "reveal" button.

Names are unique within a scope. The same name may exist in several scopes; resolution picks one (§8).

### 5.2 Where things are stored (my call)

| What | Where |
|---|---|
| Personal plain values | DB table `env_entries` (scope `personal`, kind `plain`, `value`) |
| Personal secret values | Credential Manager, generic credential, target `Dots/<install_id>/personal/<NAME>`. The DB row has no value. `install_id` is a random ULID made at first start and kept in settings, so a dev build or test data folder never touches the installed app's secrets (my call). |
| Org entries | Not stored as rows. Read from the source and held in memory, plus the offline copy. |
| Org source credentials (token, password, client secret, age identity) | Credential Manager, target `Dots/<install_id>/org/<org_id>/source` |
| Org offline copy (values plus the signed policy) | `data_dir/vault/<org_id>.bin`, encrypted with DPAPI (`CryptProtectData`, user scope, entropy = the org id) |
| Org settings (name, source kind, address and paths, admin public key) | DB table `orgs`. No secrets. |
| Grants | `dots.variables` JSON column (§7) |
| Dot's org | `dots.org_id`, NULL meaning Personal, set at creation and never updated |
| Audit queue | DB table `audit_outbox` (§10). Holds names, never values. |

`windows-sys` gains `Win32_Security_Credentials` and `Win32_Security_Cryptography`. Values are held in memory in a zeroizing type (the `zeroize` crate, already transitive) and dropped after spawn. The exact type is the builder's call.

Storage sits behind a `SecretStore` trait (put, get, delete, list targets under a prefix) with a Windows implementation and an in-memory one for tests.

## 6. Orgs, sources and policy

### 6.1 Joining an org

An admin gives each person a **join code** (my call): a base64url JSON blob, also accepted as a `.dotsorg` file, holding:

```json
{ "v": 1, "org_id": "acme-eng", "name": "Acme Engineering",
  "source": { "kind": "openbao", "address": "https://bao.acme.internal", "mount": "secret", "path": "dots" },
  "policy_key": "RWQ...minisign public key..." }
```

The join code holds no secrets. In **Settings → Organisations → Join**, the person pastes the code, sees the org name and the key fingerprint, then gives their own credential for the source:

- **OpenBao / HashiCorp Vault** (KV v2): a token, or username and password against a `userpass` or `ldap` mount named in the join code. Dots logs in through `auth/<mount>/login/<user>` and renews the token while it is renewable. Secrets come from `<mount>/data/<path>/env`, as one key per variable. OIDC browser login is later.
- **Infisical** (self-hosted or cloud): a Universal Auth machine identity (client id and client secret) that the admin creates per person. The join code gives the site URL, project id, environment slug and secret path.
- **SOPS + age**: Dots generates an age identity for this org, stores it in Credential Manager and shows its public key for the person to send to the admin. The join code gives a file location: a local path, synced by git or IT, or an HTTPS URL. Only SOPS files in **JSON format** are supported (my call). Dots decrypts natively with the `age` and `aes-gcm` crates and checks the SOPS MAC. The file holds `{ "env": { NAME: value, ... }, "policy": "<signed policy text>" }`. Revoking someone means the admin re-encrypts without their recipient, so it takes effect on the next fetch.

Dots then fetches once. Joining succeeds only when the fetch works and the policy's signature checks out against `policy_key`. The `org_id` must be new on this PC.

### 6.2 The source trait (my call)

```rust
#[async_trait]
trait Source: Send + Sync {
    /// Log in if needed and read every entry plus the signed policy text.
    async fn fetch(&self, creds: &SourceCreds) -> Result<Fetched, SourceError>;
}
struct Fetched { entries: BTreeMap<String, Zeroizing<String>>, policy: String /* minisign-signed */ }
enum SourceError { Unreachable(String), Denied(String), Invalid(String) }
```

- `Denied` (401/403, a failed MAC or failed age decryption) means **revoked**. Dots wipes that org's offline copy and in-memory values, and marks the org *access revoked*.
- `Unreachable` means **offline**. Dots keeps using the offline copy until it expires.
- `Invalid` (bad data, bad signature, wrong `org_id`) means **policy invalid**. Dots keeps the last good copy until it expires, and shows the error.

The exact HTTP calls are the builder's to confirm against each service's current API docs.

### 6.3 Policy

The admin writes a JSON policy, signs it with `minisign` (the same open-source tool and format as Dots' updates), and stores the signed text in the source, beside the entries rather than among them (my call). For OpenBao that is key `DOTS_POLICY` at `<mount>/data/<path>/policy`. For Infisical it is secret `DOTS_POLICY` in the sub-folder `<secret path>/policy`. For SOPS it is the `policy` field. `DOTS_POLICY` is never an entry, so it can't be granted. The audit token is an ordinary org entry. Dots verifies the policy with `minisign-verify` against the join code's key. Version 1:

```json
{
  "v": 1,
  "org_id": "acme-eng",
  "issued_at": "2026-10-21T09:00:00Z",
  "plain": ["LOG_LEVEL", "API_BASE_URL"],
  "use": { "allow": ["*"], "deny": ["PROD_*"] },
  "personal": "fallback",
  "shell_secrets": false,
  "refresh_minutes": 15,
  "offline_hours": 24,
  "audit": { "endpoint": "https://audit.acme.internal/dots", "token_entry": "DOTS_AUDIT_TOKEN" }
}
```

- `plain`: org entries shown with their values. Every other org entry is a secret.
- `use`: which org entry names dots may be granted. Globs use `*` only; deny wins.
- `personal`: `"deny"` means only org entries resolve, `"fallback"` means a personal entry resolves when the org has no entry of that name, and `"override"` means a personal entry wins. The default is `"deny"` (my call). An org entry that personal entries can't shadow shows a lock badge.
- `shell_secrets`: whether a secret may be granted to the shell consumer (§7). The default is `false`.
- `refresh_minutes` (default 15, minimum 1) and `offline_hours` (default 24, `0` means no offline copy).
- `audit`: optional. If set, `token_entry` names an org entry sent as `Authorization: Bearer`.
- A policy with `org_id` different from the join code is invalid. So is one with `issued_at` older than the last accepted policy (no rollback), or unknown fields at the top level (my call: fail closed).

### 6.4 Refresh and lifecycle

- Dots fetches each org at startup, every `refresh_minutes`, when the person clicks **Refresh**, and before a run starts if the copy is older than `refresh_minutes`.
- **Rotation**: the new value is used from the next fetch. A run already started keeps the value it was spawned with.
- **Expiry**: once `offline_hours` have passed since the last good fetch, the offline copy is deleted and runs that need the org's entries fail with "Acme Engineering: secrets expired offline; reconnect to refresh".
- **Leaving an org** (**Remove** in Settings) wipes the offline copy, the Credential Manager targets and the `orgs` row. The org's dots remain but are disabled and labelled "org removed". The person can delete them.
- **Changing an org's settings** (my call): the admin hands out a new join code for the same `org_id`. In Settings it goes through **Update** on that org, not **Join**. Dots shows what changed (source, address, key fingerprint), and a changed `policy_key` gets a plain warning. The person confirms, Dots fetches with the new settings, and only replaces the old ones if that fetch and signature check succeed. Dots and grants are untouched.
- **Re-joining a removed org**: `dots.org_id` is plain text with no foreign key, so joining the same `org_id` again re-attaches and re-enables those dots (my call).
- **No rollback**: the last accepted `issued_at` lives in the `orgs` row, not just the offline copy, so expiry doesn't reset it. Remove deletes it with the row.

## 7. Grants

A grant gives a dot one variable for one **consumer**:

- `mcp:<server>`: the variable may be expanded in that Library MCP server's config when this dot starts it.
- `shell`: the variable is set in the environment of the dot's agent process. For the Claude engine, that's the `claude` CLI process and so its Bash tool. For the local engine, it's the shell tool.

`dots.variables` (my call):

```json
{ "mcp:github": ["GITHUB_TOKEN"], "shell": ["LOG_LEVEL"] }
```

- Ticking an MCP server for a dot (`ToolsAndSkills.tsx`) grants that server's `${VAR}` names for `mcp:<server>` at the same time, with the names listed beside the tick. Unticking removes those grants. This is how existing setups keep working with no extra step (my call).
- Ticking a **plugin** does the same for every MCP server the plugin brings (`store.used_mcp_servers`), listed per server under the plugin's tick (my call).
- **Grants are a fixed list of names, stored**, not worked out from the server's current config (my call). When a config later gains a `${VAR}`, through an edit or a plugin update synced from disk, the dots that use it are not granted the new name. Their editor shows "github now also wants GITHUB_TOKEN" with **Grant**, and a run fails with `NotGranted` naming it until then. A plugin update can't quietly reach a new secret.
- Shell grants are made by hand in the dot editor's **Variables** section.
- A grant names a variable, not a scope. The scope comes from the dot's org at resolution time (§8).

## 8. Resolution and spawning

One function replaces `process_env` at every call site in §3.1:

```rust
fn resolve(dot: &DotScope, consumer: &Consumer, name: &str) -> Resolution
enum Resolution { Value(Zeroizing<String>), Missing { why: MissingWhy } }
// MissingWhy: NotGranted, NotDefined, DeniedByPolicy, ShellSecretsForbidden,
//             OrgOffline, OrgExpired, OrgRevoked, PolicyInvalid, OrgRemoved
```

Order, for a dot in org O (my call):

1. Not granted to this consumer: `NotGranted`.
2. If O is not Personal: if the org's status blocks use, return its reason. If the name is denied by `use`: `DeniedByPolicy`.
3. Look up O's entry and the personal entry, then apply `personal`.
4. If the value is a secret and the consumer is `shell` while `shell_secrets` is false: `ShellSecretsForbidden`.
5. Personal dots see personal entries only.

Library-time callers with no dot (`missing_variables`, `environment_variables`, `confirm.rs` origin text) resolve against **Personal**. They keep returning names and booleans only. `missing_variables` becomes per scope in the UI.

**Check and catalogs are per scope** (my call). A server's tool list can differ by credential, so `mcp_catalogs` gains a `scope` column (`personal` or an `org_id`) in its key. The accepted fingerprint and held status are kept per (server, scope). A run uses the catalog of its dot's scope. **Check** in the Library gets a "Check as" picker (Personal and each joined org) and resolves every variable of the server in that scope as if all were granted to it. Catalog failures stored on the Library item name only the scope and variable, never a dot.

**HTTP origin pinning** (my call). The host can move when a variable sits in the scheme, host or port of a `url`, and an org entry may hold a different value than the Personal one used at confirm time. So at spawn Dots compares the expanded URL's origin with the origins confirmed for that server. If the origin isn't confirmed, the server doesn't start: "github: https://x.example is not a confirmed origin; open the server in the Library to confirm it". Confirmed origins are stored on the Library item per scope. Confirming happens in the server form, with the same native dialog as today.

A failed resolution stops that consumer from starting, as "X is not set" does today, but the error names the variable, the org and the reason ("GITHUB_TOKEN: not granted to mcp:github in dot 'Triage'"). It is recorded as a run event and an audit record.

**Base environment** for every child (my call): the inherited values of `SystemRoot`, `SystemDrive`, `windir`, `ComSpec`, `PATH`, `PATHEXT`, `TEMP`, `TMP`, `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`, `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`, `CommonProgramFiles`, `NUMBER_OF_PROCESSORS`, `PROCESSOR_ARCHITECTURE`, `OS`, `USERNAME`, `USERDOMAIN`, `COMPUTERNAME`, `LANG`, and the proxy and CA variables (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY` in any case, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`). Then `env_clear()`, the base, the consumer's grants and Dots' own fixed variables (`MCP_TOOL_TIMEOUT`, `claude_env`). The `claude` CLI process also gets its own configuration variables inherited if set (`CLAUDE_CONFIG_DIR`, `ANTHROPIC_*`, `CLAUDE_CODE_*`): Dots passes them through without reading them, so the CLI's login keeps working (subscription rule). The Part B smoke confirms that a logged-in CLI still runs with this base. The list lives in one constant, and tests pin it.

## 9. Screens

**Library → Env tab** (mirrors `ContextTab.tsx`):

- Sections: **Personal**, then one per org, each with its status line ("Connected · refreshed 3 min ago", "Offline · copy valid until 14:20", "Access revoked", "Policy invalid: signature does not match").
- Each row shows the name, Plain or Secret, the value (plain only), the source (This PC, OpenBao, Infisical or SOPS file), status, a lock badge, and "used by" (the dots that grant it).
- **Add variable** (Personal only) opens a dialog with name, kind and value. A secret's value uses a password field with no reveal toggle. Secret rows offer **Replace value** and **Delete**; plain rows offer **Edit** and **Delete**. Org rows have no actions and point to "Managed by Acme Engineering".
- **Import from Windows environment** (§11) is offered here too.

**Settings → Organisations**: list, **Join**, **Refresh**, **Remove**, the source type, last fetch, the key fingerprint and the policy summary in plain words.

**Dot editor**:

- The **org picker** appears only when creating a dot, listing Personal and every joined org (default Personal). After creation it shows as read-only text.
- A **Variables** section lists every grant by consumer with its resolution status. It adds shell grants, warning "The agent can read this value and could send it anywhere its tools reach." It hides secrets for shell when `shell_secrets` is false.

**MCP server form** (`ServerForm.tsx`): "Not set in Dots' environment" becomes "Not in Personal". A pasted secret-looking literal now offers **Move to vault**. That saves the value as a personal secret named after the key and replaces it with `${NAME}`.

## 10. Redaction and audit

**Redaction** (my call):

- Each run builds a `Redactor` from every secret value resolved for it.
- A process-wide one covers all values currently held for logging that isn't tied to a run.
- Values shorter than 8 characters are not used for scrubbing, to avoid false hits, but are still recorded as used.
- Every spelling is replaced by `[secret:NAME]`: raw, URL-encoded (both `%20` and `+`), JSON-string-escaped, and standard and URL-safe base64 of the raw value. Matching runs before any truncation.
- It applies to `run_events.data`, `approvals.input`, `runs.payload`, events sent to the UI, the `tracing` output, and MCP stderr before it is logged.
- Redaction is a backstop, not the main defence. §8 keeps values out of places that never needed them.

**Audit records**: one per resolution attempt, plus one per fetch outcome per org. JSON, one object per line:

```json
{ "v": 1, "id": "01J...", "seq": 412, "prev": "sha256:...", "at": "2026-10-21T09:12:03Z",
  "org_id": "acme-eng", "user": { "windows": "ACME\\jdoe", "machine": "LT-0423", "source_identity": "jdoe" },
  "dot": { "id": "...", "name": "Triage" }, "run_id": "...",
  "event": "resolved", "name": "GITHUB_TOKEN", "consumer": "mcp:github", "source": "openbao", "reason": null }
```

- `event` is one of `resolved`, `denied` (with `reason`, a `MissingWhy`), `fetched`, `offline`, `revoked`, `expired` or `policy_invalid`.
- `seq` counts up per org per install, and `prev` is the SHA-256 of the previous record's canonical JSON. A gap or a broken chain is visible to the receiver. Signing records with a per-device key needs device registration, so it waits for project 2 (my call; this changes the earlier playback, which said "signed").
- Records with a dot are only written for dots in an org. Personal dots get no audit (my call).
- The queue is `audit_outbox` in the DB. It holds no values, so it isn't encrypted (my call; this also changes the earlier playback). The DB is still under the user's profile.
- Sending: `POST <endpoint>` with `Content-Type: application/x-ndjson`, batches of up to 500 records, `Authorization: Bearer <token_entry>`. A 2xx response deletes the batch; anything else retries with backoff. The queue keeps the newest 100,000 records per org and records the drop as an `expired` event. That bound is my call.
- The format and endpoint go in `docs/vault-audit-format.md`, which is what project 2 implements.
- People see their own records in a **History** list on the org's settings page.

## 11. Migration and related fixes

On the first start after upgrade (in `store/upgrade.rs`, after a new migration `0011_vault.sql`):

1. **Literal secrets** in `library_items.config` (`looks_secret` keys or `Authorization` headers with no `${…}`) move into personal secrets named after the key (`SF_TOKEN`, or `<SERVER>_AUTHORIZATION` for a header). They become `${NAME}`, then the DB is VACUUMed, as `finish_0006` did.
   - **Plugin servers** are re-synced from the plugin's `.mcp.json` on every run, so a rewrite of the stored config wouldn't stick. Instead, each plugin server item gets an `overrides` map (key → `${NAME}`). `sync_plugin_servers` applies it after reading the file and before storing, so the literal never reaches the DB (my call).
   - A plugin server with a secret-looking literal and no override is **held** with "move this value to the vault" and a **Move to vault** button that makes the override.
   - The plaintext stays in the plugin's own folder, which Dots does not own. The Env tab says so and names the file.
2. **Grants are created** for every MCP server each existing dot uses (§7). Every existing dot becomes Personal.
3. **Import offer**: the Env tab shows a one-time banner listing the `${VAR}` names used by Library items that are set in Dots' Windows environment, with **Import** (copied into personal secrets) or **Dismiss**. Dots stops reading its own environment for `${VAR}` after this release, so without import those servers report the variable missing.
4. **Old backups**: backups made before this release may hold the old plaintext values. The Env tab offers **Delete old backups** once, naming the files. Dots never deletes them on its own.

**Webhook token** (my call):

- Stored as a SHA-256 hash.
- Shown once, when generated or regenerated, with a copy button and "You won't see this again".
- `?token=` is no longer accepted, because query strings end up in proxy logs. Dots has no tagged release yet, so nothing in the field breaks.
- Existing tokens are hashed by the migration, and the panel offers **Regenerate**.

**Gateway file**: on Windows, `write_private` sets a DACL granting only the current user. Profile folders are already per user, so this is defence in depth.

## 12. Admin tooling

**`dots-admin`** is a new binary crate, `crates/dots-admin`. It uses the same policy and join-code types as `dots-core` (one module, so the two can't drift) and the pure-Rust `minisign` crate for signing. It is built with `cargo build -p dots-admin --release` and is not part of the NSIS installer. Attaching it to releases is left to the release work (my call). Commands (my call on names):

- `dots-admin keygen --out <dir>`: a minisign key pair for the org (`policy.key`, `policy.pub`), with the secret key password-protected.
- `dots-admin policy new --org-id <id>`: writes a policy file with every field at its default and an `issued_at` of now.
- `dots-admin policy check <policy.json>`: the same validation Dots applies (§6.3), with errors in plain words.
- `dots-admin policy sign <policy.json> --key policy.key`: checks the policy, sets `issued_at` to now, and writes the signed text ready to store as `DOTS_POLICY`.
- `dots-admin policy verify <signed> --pub policy.pub`: what a Dots client would accept.
- `dots-admin join-code --org-id <id> --name <name> --source <openbao|infisical|sops> [source flags] --pub policy.pub [--file out.dotsorg]`: prints the join code, or writes the `.dotsorg` file.
- `dots-admin audit verify <records.ndjson>`: checks `seq` and the `prev` hash chain for one org and install, and reports gaps.

**Admin guide** `docs/vault-admin.md`:

- the threat model (§4)
- setting up each source with its exact paths: OpenBao KV v2 mount and policy for read-only access per person; an Infisical project, environment, folder layout and one machine identity per person; the SOPS JSON layout and how to add or remove age recipients
- making and signing the policy, and handing out join codes
- rotating a key, cutting someone off in each source, rotating the policy signing key (the **Update** flow in §6.4)
- the audit endpoint (pointing to `docs/vault-audit-format.md`)

## 13. Testing

- **Unit**: the resolver order (§8) as a table test, including same-name entries across two orgs, `personal` modes, `shell_secrets`, every status, and the rule that a Personal dot never sees an org value. Also: policy verification (good, bad signature, wrong org, rollback, unknown field), redaction of every spelling, the audit chain, and the base environment constant.
- **Adapters**: a fake OpenBao and a fake Infisical as `axum` test servers (login, read, 403, timeout). SOPS fixture files checked in, made with the real `sops` and `age` tools and a test-only identity. How they are produced is the builder's call, but the steps go in a README next to them.
- **Spawn**: a test MCP server and the fake CLI print their environment. The tests assert only base, granted and fixed variables arrive, and that a variable set in the test process environment does not.
- **Store**: an in-memory `SecretStore` in tests. One Windows-only test round-trips Credential Manager and DPAPI under a unique target prefix and cleans up.
- **UI and contract**: Env tab, Organisations page, dot editor Variables and org picker, through the existing contract samples.
- **`dots-admin`**: a policy signed by `dots-admin` is accepted by the Dots verifier, and a join code it writes is parsed by Dots. These run as one round-trip test. `audit verify` is checked against a chain with a gap.
- **Isolation extras**: catalogs and confirmed origins per scope; a plugin update adding a `${VAR}` does not grant it; a plugin override survives a re-sync.
- **Part B smoke (the owner's)**:
  - A real logged-in `claude` CLI runs with the base environment.
  - A real OpenBao dev server and a real SOPS file join, rotate and revoke.
  - An offline expiry.
  - The audit endpoint receives records (any HTTP sink, such as a small local script).

## 14. Changes made while building (plan A)

The plan is split into three: **A** Personal (§5), grants (§7), resolution and the base environment (§8), the Env tab and the dot's Variables (§9), redaction (§10), migration and the two fixes (§11); **B** orgs, sources, policy, the offline copy, per-scope catalogs and origin pinning, the org picker and `dots-admin` (§6, §8, §9, §12); **C** audit (§10). Plan A's prototype settled these:

- **Names.** The core module is `vault`:
  - `secrets` (`SecretStore`, `WindowsCredentials`, `MemorySecrets`)
  - `entries` (`EntryKind`, `EnvEntry`)
  - `service` (`Vault`)
  - `grants` (`Grants`, `Consumer`)
  - `resolve` (`Resolved`, `Missing`, `Unresolved`)
  - `child_env` (`BASE_ENV`, `start_clean`)
  - `redact` (`Redactor`, `RedactWriter`)
  - `literals`
  - `upgrade`

  The commands are `env_list`, `env_add`, `env_set`, `env_remove`, `env_import_candidates`, `env_import`, `env_import_dismiss`, `env_old_backups` and `env_old_backups_answer`. A dot's grants are `DotSpec::variables`, and the Env tab is "Env".
- **Secrets are at most 2,560 bytes**, Credential Manager's limit for one value (`CRED_MAX_CREDENTIAL_BLOB_SIZE`). A longer secret is refused (§5.2).
- **Names compare without regard to case**, as Windows environment names do: `github_token` and `GITHUB_TOKEN` are one variable (§5.1).
- **`Config::new` keeps secrets in memory.** Only the app's `Config::from_settings` uses Credential Manager, under `Dots/<install_id>/`, so a test never writes there (§5.2).
- **Library-time reads resolve from Personal.** A probe, a check, the "missing variables" of a row and `environment_variables` read Personal values; `environment_variables` now answers whether Personal has a name. Dots no longer reads its own Windows environment for `${VAR}` anywhere (§8).
- **The moved-origin check compares URL text.** When a server's URL changes, the accepted tool list is reset by comparing the text, since values are no longer read for it. A URL whose text changed counts as moved (§8; origin pinning per scope is plan B).
- **What a variable that does not resolve says.** The server is left out of the run with a run event "`NAME`: not granted to `mcp:<server>` in dot '`<dot>`'" or "`NAME`: not in Personal (dot '`<dot>`', `<consumer>`)". The server's Library catalog is not marked: the failure is the dot's (§8).
- **The base environment also keeps** `PSModulePath` (the local engine's PowerShell), `HOME`, `ProgramW6432`, `CommonProgramFiles(x86)` and `CommonProgramW6432` (§8).
- **Redaction uses one redactor for the whole process.** It holds every Personal secret, each read once when the vault opens at start, and every secret Dots reads after that; not one list per run. A secret that Credential Manager fails to read at start is logged by name and skipped. The store scrubs as it writes: run events (so the live view too), approval inputs, run payloads, summaries and errors, and the text of notes and questions. The log file is scrubbed line by line (§10).
- **An approval keeps the scrubbed input.** When Dots itself performs an approved call (the local engine after a late decision), it performs it with `[secret:NAME]` where a secret value was. The input hash is still the one of the call as asked (§10).
- **Secret-looking values move to Personal automatically**, with no "held" state and no button:
  - when a server is added or changed
  - each time a plugin's servers are synced from its `.mcp.json`
  - at every start, for values a version before the vault stored; the dots that use such a server are granted the names

  A name Personal has with the same value is reused; another value gets `_2`, `_3`, … A value over 2,560 bytes stays where it is, is logged and is scrubbed (see below). The server form says this happens instead of offering "Move to vault" (§9, §11).

  A key looks secret when one of its words, split on `_`, `-` and `.` and at a lower-case to upper-case boundary (`accessToken`, `OAuthToken`; a run of capitals is one word, so `APIKey` is `APIKEY`), in any case, is `TOKEN`, `KEY`, `SECRET`, `PASSWORD`, `PASSWD`, `PWD`, `AUTH`, `AUTHORIZATION`, `CREDENTIAL`, `CREDENTIALS` or `APIKEY`. A key whose last word is `URL`, `URI`, `HOST`, `ENDPOINT`, `ID`, `NAME`, `TYPE`, `PATH`, `FILE`, `DIR`, `HEADER`, `REGION`, `PORT`, `ISSUER`, `REALM`, `SCOPE` or `MODE` never does (`TOKEN_URL`, `KEY_ID`, `API_KEY_HEADER`). A filesystem path does not move. An `http(s)` URL with no user or password and no query (`?`) does not move either, but only when every secret word of its key is `AUTH`, `AUTHORIZATION`, `CREDENTIAL` or `CREDENTIALS` (`AUTH_SERVER=https://auth.example.invalid/realms/x`); under a key with `TOKEN`, `KEY`, `SECRET`, `PASSWORD`, `PASSWD`, `PWD` or `APIKEY` the URL moves, since a webhook URL is the secret (`SLACK_TOKEN`, `WEBHOOK_SECRET`). The UI's `looksSecret` and the server form's notice follow the same rule (§9, §11).

  A server already stored keeps its names:
  - A plugin's re-sync whose file has another value for a key the stored server reads as `${NAME}` keeps `${NAME}`. Personal wins: its value stays, no `NAME_2` is made, and the difference is logged by name only. So the dots that use the server stay granted.
  - A form edit that types a new value into a key that read `${NAME}` replaces NAME's value in Personal, when no other Library item reads `${NAME}`. Otherwise the value gets a name of its own, `NAME_2`, `NAME_3`, …
- **The import offer** lists the `${VAR}` names that Library servers read, that Dots' Windows environment sets, and that Personal lacks. They are imported as secrets. Importing or dismissing answers the offer for good: the `settings` row `vault_import_answered` (§11 step 3). A value over 2,560 bytes, or one the vault refuses or cannot store, is skipped and logged by name; the others are still imported, and the Env tab names what was not.
- **"Made before this release"** means older than the `settings` row `vault_since`, written the first time the data folder runs with the vault. Deleting or keeping the backups answers the offer for good: `vault_backups_answered` (§11 step 4).
- **A new dot has no webhook token.** Its webhook refuses every request until **Make a token**. A token is stored as `sha256:<hex>` and shown once; tokens from before are hashed when the store opens (§11).
- **The gateway file's DACL** is set before its secret is written: one entry, full access for the signed-in user, nothing inherited (§11).
- **A shell variable that does not resolve is left out, and the run starts.** A server whose variable does not resolve is left out of the run. The shell instead runs without that variable, and the run event names it (§8).
- **The claude CLI's own variables are never set from Personal.** The CLI's environment passes through, unread:
  - every name that starts with `ANTHROPIC_` or `CLAUDE_CODE_`
  - `CLI_ENV`: `CLAUDE_CONFIG_DIR`, `DISABLE_TELEMETRY`, `DISABLE_AUTOUPDATER`, `DISABLE_ERROR_REPORTING`, `MCP_TIMEOUT`, `BASH_DEFAULT_TIMEOUT_MS`, `BASH_MAX_TIMEOUT_MS` and `MAX_MCP_OUTPUT_TOKENS`

  A shell grant of one of these names is skipped, with the run notice "`NAME`: the claude CLI's own variable, not set from Personal". So is a shell grant, for the CLI only, of a name that would point it at another endpoint or load code into it: `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, `NODE_EXTRA_CA_CERTS` and `NODE_OPTIONS`, in any case, with the same notice. The CLI keeps what Dots' environment passed through for them; other children may be granted them. Bedrock's and Vertex's `AWS_*` and `GOOGLE_*` names are not passed through (§8).
- **The base environment also keeps** `SSL_CERT_DIR`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `ALL_PROXY`, `ALLUSERSPROFILE` and `PUBLIC`, and where toolchains are, so an agent's build still finds them: `JAVA_HOME`, `GOROOT`, `GOPATH`, `CARGO_HOME`, `RUSTUP_HOME`, `DOTNET_ROOT`, `PNPM_HOME`, `NVM_HOME`, `NVM_SYMLINK`, `VIRTUAL_ENV`, `CONDA_PREFIX` and `ANDROID_HOME` (§8).
- **Redaction covers more spellings and more places** (§10):
  - A value is matched raw, URL-encoded, JSON-escaped and base64-encoded. Base64 is matched at all three byte alignments, so a secret inside a longer encoded value is caught too. Values under 8 bytes are not matched.
  - Text is scrubbed before it is cut to a size limit, so no piece of a secret survives at the cut. This holds for the context library's tool too: a document's head, a summary, a matching line and an echoed argument.
  - Also scrubbed: an MCP server's catalog error, the reviewer notes of approvals and follow-ups, the panic line's message, and a command's error message to the UI. If the redactor is busy when a panic is written, the message is withheld: the line says `(message withheld: redactor busy)`.
  - An "Always allow" grant and a parked approval are keyed by the scrubbed input. Every Personal secret is in the redactor from the start, so they still match after Dots restarts.
- **A value that cannot move to Personal is scrubbed.** A value over 2,560 bytes, or one the vault refuses (one holding a NUL), stays in the Library config, but Dots scrubs it from everything it writes and logs its name (§11).
- **A key that does not make a variable name gets a derived one.** A digit at the start gets a leading `_`, and the name is cut to 128 characters. A suffix `_2`, `_3`, … keeps the name within 128 (§11).
- **A one-word `Authorization` value** becomes `${NAME}`, with no scheme added (§11).
- **The move at start survives a failing server.** Only a server with a value that can move is moved; a value that cannot is scrubbed and never makes a start vacuum. The `settings` row `vault_vacuum_pending` is set just before the first config is really rewritten, and when tokens from before are hashed. Every start vacuums while it is set. A server that fails is logged by name and tried again at the next start, and the others still move. `vault_since` is written once the move has run, even when a server failed (§11).
- **The import offer never lists the claude CLI's own names or the base environment's names**, and import refuses them. Their values are never read (§11 step 3).
- **Deleting the old backups asks first.** A backup that cannot be deleted is logged and named in the Env tab, by its file name. The answer is then not recorded, so the offer stays with what is left (§11 step 4).
- The migration is 0011, after master's 0009 (browser) and 0010 (seen).
- **Not in plan A:**
  - the Env tab's "used by" (§9)
  - org sections, the org picker and the Organisations page (plan B)
  - audit records (plan C)

## 15. Left to the builder

- Exact service API calls and crate versions (verify against current docs), the zeroizing type, and the order of the plan's tasks. A natural split into stages:
  1. personal vault plus grants, base environment, redaction and migration
  2. orgs, policy, adapters, offline copy and `dots-admin`
  3. audit
- The UI layout details within the screens in §9, following the existing Library and dot editor patterns.
- Error wording beyond the examples here, keeping the rule that every failure names the variable, the org and the reason.
