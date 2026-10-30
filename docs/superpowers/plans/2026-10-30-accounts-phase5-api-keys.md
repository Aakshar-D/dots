# Accounts phase 5: API keys and the Host check — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-workspace API keys on this PC (`dots_sk_…`, shown once, stored as a hash) unlock three `/v1` routes on the local server limited to the key's workspace and the person's current role, and every server route refuses requests whose `Host` isn't this server.

**Architecture:** Keys live in a new `api_keys` table with pure key-making in `crates/dots-core/src/accounts/api_keys.rs`. The `Runtime` creates, lists and revokes keys behind the workspace permission rule; the server gets a `v1` module whose handlers authenticate the Bearer key, then check `require_in` for the key's workspace on every request. A Host-check middleware sits beside the existing loopback check.

**Tech Stack:** Rust (axum 0.8, sqlx/SQLite, getrandom 0.3, sha2), Tauri commands + contract samples, React + TanStack Query + zod + vitest.

**Spec:** `docs/superpowers/specs/2026-10-28-accounts-members-design.md` (§10 API keys, §6.3 "Create API keys in a workspace", §11 threat model, §13 phase 5).

Base branch: `feat/accounts-api-keys`, from `feat/accounts-workspaces` (df98cad).

Rulings in this plan:
- Keys are local to this PC, so "own" keys (§6.3) is moot: creating a workspace's keys needs `CreateApiKeys` in that workspace (`require_in`; Member rights or above there); revoking is always allowed (it only reduces access; any key stored on this PC, NotFound for an unknown id). Personal workspaces always allow it.
- Key text: `dots_sk_` + 43 characters from `[0-9A-Za-z]`, each drawn from `getrandom` bytes by rejection sampling (bytes ≥ 248 dropped), so ≥256 bits and no modulo bias.
- `prefix` = the first 12 characters of the key (`dots_sk_` + 4). `hash` = `util::token_hash(key)` (the existing `sha256:<hex>` format), compared with `ct_eq` like webhook tokens.
- Deleting a workspace or a workspace vanishing from the roster revokes its keys; leaving the account deletes the account's keys. A key whose workspace no longer exists is refused.
- `/v1` errors are JSON `{"error": "<plain words>"}`: 401 for a missing, unknown or revoked key (one text for all, so a caller can't probe which); 403 for a role refusal (the core's text); 404 for a dot or run outside the key's workspace (same text as unknown); 409 for a disabled dot, like the webhook.
- `GET /v1/runs/{id}` returns status, not events (§15 leaves it open; status is the minimum).
- Host check: allowed values are exactly `127.0.0.1:<port>` and `localhost:<port>` (host part case-insensitive) for the server's actual bound port; a missing `Host` is refused. Refusal: 403 with no body for non-`/v1` routes (like the loopback check) and `{"error": "This server only answers requests addressed to 127.0.0.1 or localhost."}` for `/v1`.

## Global Constraints

- Never stage the root `Cargo.toml`. Never add a Co-Authored-By trailer.
- The key is shown once (in the create answer) and never stored or logged in plain text; only `hash` and `prefix` are kept. Error texts never contain a key.
- Every `/v1` request checks: key not revoked, workspace still exists, and the person still has the needed permission in that workspace (`RunDots` to trigger, `View` to list or read runs) — at request time, not at key creation.
- Existing per-dot webhook tokens and MCP run secrets stay as they are (they also pass through the Host check).
- Every refusal is plain words; roles enforced in `dots-core`.
- Migrations: next is `0024`. Never edit earlier ones.
- UI: zod `strictObject`; Deep Teal tokens and existing components; every disabled control says why, using the core's text (never a TS role rule). Contract fixture via `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
- Gates per task: targeted `cargo test -p dots-core --test <file>`, `cargo clippy --workspace --tests -- -D warnings`, `cargo fmt --all`; UI tasks `cd ui && bun run test && bun run typecheck`. Known unrelated failure: `runtime_review_test::preview_schedule_lists_the_next_fire_times`.

## Review Focus

1. A key keeps working after the person's role drops to Viewer, or after they're removed from the account or the workspace: every `/v1` route must refuse at request time (Task 4 `role_change_refuses_existing_key`).
2. A key for workspace A used on a dot or run in workspace B (same account) or another account: 404, never a trigger (Task 4 `key_is_limited_to_its_workspace`).
3. A request with `Host: evil.example:<port>` (DNS rebinding) to any route, including `/dots/{id}/trigger` and `/mcp`: refused; the normal callers (Claude CLI, Playwright websocket, webhook callers) still work (Task 3 tests).
4. A revoked key, a key with one changed character, and a missing/garbled `Authorization` header: all 401 with the same text (Task 4 `bad_keys_are_401`).
5. The workspace is deleted or the account is left after a key was made: the key stops working and the API keys tab doesn't show it as usable (Task 2 `workspace_gone_revokes_keys`).

---

### Task 1: The api_keys table and key making

**Files:**
- Create: `crates/dots-core/migrations/0024_api_keys.sql`, `crates/dots-core/src/accounts/api_keys.rs`, `crates/dots-core/src/store/api_keys.rs` (register both modules)
- Test: `crates/dots-core/tests/store_api_keys_test.rs`

**Interfaces:**
- Migration: `CREATE TABLE api_keys (id TEXT PRIMARY KEY, scope TEXT NOT NULL, workspace_id TEXT NOT NULL, name TEXT NOT NULL, prefix TEXT NOT NULL, hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL, last_used_at TEXT, revoked_at TEXT);` plus an index on `(scope, workspace_id)`.
- Produces:
  - `pub const API_KEY_PREFIX: &str = "dots_sk_";`, `pub fn new_api_key() -> String` (the rulings' algorithm), `pub fn key_prefix(key: &str) -> String` (first 12 chars), `pub fn looks_like_key(text: &str) -> bool` (prefix + exactly 43 base62 chars).
  - `pub struct ApiKeyRow { pub id: String, pub scope: String, pub workspace_id: String, pub name: String, pub prefix: String, pub hash: String, pub created_at: String, pub last_used_at: Option<String>, pub revoked_at: Option<String> }`.
  - `Store::add_api_key(&ApiKeyRow)`, `Store::api_keys(scope) -> Result<Vec<ApiKeyRow>>` (newest first), `Store::api_key(id) -> Result<Option<ApiKeyRow>>`, `Store::api_key_by_secret(key: &str) -> Result<Option<ApiKeyRow>>` (looks up by `token_hash`, confirms with `ct_eq`), `Store::revoke_api_key(id, at)` (NotFound if absent; an already revoked key keeps its first time), `Store::touch_api_key(id, at)`.
- [ ] **Step 1: Failing tests:** `new_keys_have_the_shape` (prefix, length 51, base62 only, 100 keys all different); `key_prefix_is_12_chars`; `looks_like_key` accepts a new key and refuses short / long / other prefix / a `-`; `add_list_revoke_touch_round_trip`; `lookup_by_secret_finds_only_the_exact_key` (one changed character → None); `a_key_for_a_workspace_is_added_only_while_the_workspace_is_there`; the key's text is in no file Dots wrote (checked in `create_shows_the_secret_once_and_lists_without_it`).
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(accounts): api_keys table and key making`

---

### Task 2: Creating, listing and revoking keys in the runtime

**Files:**
- Modify: `crates/dots-core/src/accounts/api_keys.rs` (views), `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/store/{workspaces.rs,orgs.rs}` (revoke/delete when a workspace or account is gone)
- Test: `crates/dots-core/tests/runtime_api_keys_test.rs`

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Clone, Debug, PartialEq)] pub struct ApiKeyView { pub id: String, pub scope: String, pub workspace_id: String, pub workspace_name: String, pub name: String, pub prefix: String, pub created_at: String, pub last_used_at: Option<String>, pub revoked_at: Option<String>, pub refusal: Option<String> }` — `refusal` is the core's text when this PC may not manage keys in that workspace (`require_in(.., CreateApiKeys)`), else None.
  - `#[derive(Serialize, Clone, Debug)] pub struct CreatedApiKey { pub key: ApiKeyView, pub secret: String }`.
  - `Runtime::api_keys(scope: &str) -> Result<Vec<ApiKeyView>>` — keys of the scope's visible workspaces only (`visible_workspaces`).
  - `Runtime::create_api_key(scope: &str, workspace_id: &str, name: &str) -> Result<CreatedApiKey>` — `require_in(CreateApiKeys)`; the workspace must exist (`Error::NotFound("No workspace '{id}' in {account}.")`); name trimmed, 1–64 chars, else `Error::Invalid("Give the key a name (up to 64 characters).")`.
  - `Runtime::revoke_api_key(id: &str) -> Result<ApiKeyView>` — same permission on the key's workspace.
  - `WorkspaceView.refusals` gains `create_api_keys: Option<String>` (core text) so the UI can disable the create form per workspace without a TS role rule.
  - Store hooks: `delete_workspace` and `mirror_workspaces` set `revoked_at` on keys of removed workspaces; `remove_org` deletes the org's keys — each in the existing transaction.
- [ ] **Step 1: Failing tests:** `create_shows_the_secret_once_and_lists_without_it`; `viewer_cannot_create_but_can_revoke` (exact phase 2/4 text; revoking needs no role); `outside_workspace_cannot_create`; `personal_keys_allowed`; `name_rules`; `workspace_gone_revokes_keys` (delete workspace → revoked; roster drops it → revoked; leave account → gone); `list_shows_only_visible_workspaces`.
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun the workspace and store suites.
- [ ] **Step 5: Commit** `feat(accounts): create, list and revoke API keys per workspace`

---

### Task 3: Host check on every server route

**Files:**
- Modify: `crates/dots-core/src/server/mod.rs` (the middleware learns the bound port: `ServerState` gains `port: u16`, or `serve` builds the layer with it), every `ServerState` construction (runtime, tests)
- Test: `crates/dots-core/tests/server_host_test.rs`

**Interfaces:**
- Produces: `async fn host_only(...)` middleware layered next to `loopback_only`; `pub fn host_allowed(host: Option<&str>, port: u16) -> bool` (pure, unit-tested).
- [ ] **Step 1: Failing tests:** `host_allowed` table (`127.0.0.1:P` ✓, `localhost:P` ✓, `LOCALHOST:P` ✓, `127.0.0.1` ✗, `127.0.0.1:other` ✗, `evil.example:P` ✗, `[::1]:P` ✗, missing ✗); `rebinding_host_is_refused_on_every_route` (`/health`, `/dots/{id}/trigger`, `/mcp`, `/mcp/s/x`, `/v1/dots` with `Host: evil.example:P` → 403; `/v1` gets the JSON text); `normal_callers_still_work` (existing webhook and MCP flows with reqwest's default Host; a websocket upgrade to `/cdp/{secret}` with `Host: 127.0.0.1:P` isn't refused by the check).
- [ ] **Step 2–4:** FAIL, implement, PASS; rerun `server_test`, `claude_gateway_test`, `claude_browser_test`, `apps_run_test`, `memory_server_test`.
- [ ] **Step 5: Commit** `feat(server): refuse requests not addressed to this server (DNS rebinding)`

---

### Task 4: The /v1 routes

**Files:**
- Create: `crates/dots-core/src/server/v1.rs`
- Modify: `crates/dots-core/src/server/mod.rs` (routes; body limit like the webhook's)
- Test: `crates/dots-core/tests/server_v1_test.rs`

**Interfaces:**
- Routes: `GET /v1/dots` → `200 {"dots":[{"id","name","enabled","org_id","workspace_id"}]}` (the key's scope and workspace only); `POST /v1/dots/{id}/trigger` → the webhook's body handling (empty/whitespace = no payload; bad JSON = 400 `"body must be JSON: …"`), `202 {"run_id"}`; `GET /v1/runs/{id}` → `200 {"id","dot_id","status","queued_at","started_at","ended_at","summary","error"}`.
- Auth per request: `webhook::bearer`; `looks_like_key`; `Store::api_key_by_secret`; not revoked; workspace row exists → else 401 `"That API key isn't valid. Make a new one in Dots."`. Then `require_in(orgs, org of the scope, workspace, View or RunDots)` → 403 with the core's text. A dot or run outside the key's scope and workspace → 404 `"Not found."` (same as unknown). Disabled dot → 409 `"dot is disabled"`. On success `touch_api_key`. Triggers use a new `TriggerKind::Api` if adding a kind is simple (it must show in run lists as "API"), else `TriggerKind::Webhook` with the reason in the report.
- [ ] **Step 1: Failing tests:** `list_dots_in_the_keys_workspace_only`; `trigger_queues_a_run_and_returns_its_id`; `read_run_status`; `key_is_limited_to_its_workspace` (other workspace / other account → 404, nothing queued); `bad_keys_are_401` (missing header, garbage, one changed char, revoked, workspace deleted — same text); `role_change_refuses_existing_key` (Member → Viewer: trigger 403, list/read 200; removed from the workspace → 403 with the outside text); `disabled_dot_is_409`; `last_used_is_updated`; `bad_json_is_400`.
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(server): /v1 routes for dots and runs with API keys`

---

### Task 5: App commands and contract

**Files:**
- Modify: `apps/dots-app/src/{commands.rs,lib.rs,contract.rs}`, `crates/dots-core/src/runtime.rs` (`api_base_url() -> Result<String>`: `http://127.0.0.1:{port}/v1`, Conflict when the server isn't running, like `webhook_url`), `ui/src/api/{types.ts,commands.ts,contract.test.ts,__fixtures__/contract.json}`

**Interfaces:**
- Commands: `api_key_list(scope) -> Vec<ApiKeyView>`, `api_key_create(scope, workspace_id, name) -> CreatedApiKey`, `api_key_revoke(id) -> ApiKeyView`, `api_base_url() -> String`; `WorkspaceView.refusals.create_api_keys` in the zod schema.
- Samples: `ApiKeyView` (live with `refusal: null`, revoked, and one with a refusal), `CreatedApiKey`, `TriggerKind` with `api` if Task 4 added it.
- [ ] **Step 1:** samples first → contract FAIL. **Step 2:** implement, regenerate, zod + wrappers, `bun run test && bun run typecheck` PASS.
- [ ] **Step 3: Commit** `feat(accounts): app commands for API keys`

---

### Task 6: UI — API keys tab

**Files:**
- Create: `ui/src/screens/accounts/ApiKeys.tsx` (used for an account and for Personal)
- Modify: `ui/src/screens/Accounts.tsx` (fourth tab **API keys** for active and legacy accounts; an API keys part in the Personal section)
- Test: `ui/src/screens/accounts/ApiKeys.test.tsx`

**Behaviour:**
- List: name, prefix + `…`, workspace name, created, last used ("Never" when null), a Revoked badge. **Revoke** (confirm: "Apps using this key stop working at once.") disabled with the key's `refusal` as the reason.
- **Create key**: name + workspace select (visible workspaces; an option is disabled with `refusals.create_api_keys` as the reason). The answer shows the secret once in a read-only box with **Copy**, the line "Copy it now. Dots won't show it again.", the base URL from `api_base_url`, and an example `curl -H "Authorization: Bearer <key>" <base>/dots`. Closing drops the secret from memory (never put in the TanStack Query cache).
- Server not running → the base URL line shows the core's Conflict text.
- Invalidate the key list after create/revoke and on `org_updated`.
- [ ] **Step 1: Failing tests:** `lists_keys_with_prefix_and_last_used`; `create_shows_the_secret_once` (after closing and reopening the secret is gone; the list shows only the prefix); `revoke_asks_to_confirm`; `viewer_can_revoke_but_not_create`; `server_off_shows_the_reason`.
- [ ] **Step 2–4:** FAIL, implement, PASS; full UI gates.
- [ ] **Step 5: Commit** `feat(accounts): API keys tab`

---

### Task 7: Docs, smoke and release notes

**Files:**
- Modify: `docs/accounts.md` (making a key, using it with curl, what it can reach, revoking), `docs/vault-admin.md` (one paragraph: keys are per PC, never shared through the roster; role changes apply at once), `release-notes.md`
- Create: `docs/verification/2026-10-30-accounts-phase5-smoke.md` (make a key → list dots → trigger → read run; wrong workspace 404; Viewer can't trigger; revoke → 401; `curl -H "Host: evil.example:PORT"` → 403; webhooks and a Claude run with MCP still work)
- [ ] **Step 1:** write; check every route, text and label against the code.
- [ ] **Step 2: Commit** `docs(accounts): API keys; phase 5 smoke`
