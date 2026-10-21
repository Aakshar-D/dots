# Env Vault, Plan A: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dots keeps people's variables and secrets in a vault. Secrets are stored in Windows Credential Manager and never shown again. Each dot is granted, per MCP server and for its shell, the variables it may read. Child processes start from a minimal base environment instead of all of Dots' own. Secret values are scrubbed from everything Dots stores or logs. Setups from before the vault are brought in.

**Architecture:** A new core module `vault` holds:

- the secret store: `SecretStore`, implemented by `WindowsCredentials` and `MemorySecrets`
- Personal entries: the `env_entries` table and the `Vault` service
- grants: `DotSpec::variables`, keyed `mcp:<server>` or `shell`
- the resolver that replaces Dots' own environment for `${VAR}`
- the base environment every child starts from (`child_env`)
- a process-wide redactor that the store and the log writer scrub with
- the moving of secret-looking literals into Personal, plus the one-time import and old-backups offers

The app gains nine `env_*` commands. The UI gains the Library's **Env** tab, a dot's **Variables** section, and a webhook token that is shown once.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio; `windows-sys` 0.61.2 (Credential Manager, ACLs); `zeroize` 1 (already in `Cargo.lock`, now a direct dependency); Tauri 2.12; Bun 1.3, React 19.3, TypeScript 6.0, TanStack Query 5.104, zod 4.6, Vitest 5.0.

**Spec:** `docs/superpowers/specs/2026-10-21-env-vault-design.md` at commit `1df8358`, the base of this plan, on branch `feat/env-vault`. This plan is **plan A** of three, as Task 10 records in the spec's new §14:

- **A:** Personal, grants, resolution and the base environment, the Env tab and a dot's Variables, redaction, migration and the two token fixes.
- **B:** orgs, sources, policy, the offline copy, per-scope catalogs, origin pinning, the org picker and `dots-admin`.
- **C:** audit.

**Previous plans:** `docs/superpowers/plans/2026-10-15-mcp-skills-plugins.md` made the Library, the gateway and `${VAR}` from Dots' environment. This plan replaces that source with the vault.

**How this plan's code was checked:** the code blocks of Tasks 1 to 10 were generated from a throwaway worktree of commit `1df8358`, one commit per task (branch `proto/env-vault-a`), where they compiled and passed on Windows 11. A script then replayed the plan onto `1df8358`, applying every block and comparing the tree with each task commit. All matched; `Cargo.lock` and the contract fixture were taken from the task commits, since the plan's commands write them.

| Point | Core | App | UI |
|---|---|---|---|
| Before the first task | 740 passed | 94 | 447 tests in 31 files |
| At the last task | 774 passed | 94 | 462 in 32 files |

At the last task, clippy runs without warnings, and `cargo fmt --check` and `tsc --noEmit` are clean. The Credential Manager and DACL tests ran against the real Windows APIs, under throwaway install ids, and cleaned up after themselves. No agent ran the real `claude` CLI.

## Global Constraints

- Toolchain: Rust 1.95 stable; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/env-vault`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\env-vault`, based on `1df8358`. All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Every cargo command runs with `CARGO_TARGET_DIR=C:/Users/aksha/dots/target` (the shared build folder): `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target` once per shell.
- Dependencies:
  - The only new Cargo dependency is `zeroize = "1"`, already in `Cargo.lock` through other crates.
  - `windows-sys` gains the features `Win32_Foundation`, `Win32_Security_Credentials` (Task 1) and `Win32_Security_Authorization` (Task 7).
  - No npm dependency, no new Tauri capability or plugin.
  - One migration: `0009_vault.sql`, made in Task 2 and extended in Tasks 3 and 7. Dots has no release with it yet.
- The subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials or calls an Anthropic API. The CLI's own variables (`CLAUDE_CONFIG_DIR`, `ANTHROPIC_*`, `CLAUDE_CODE_*`) are passed through unread (Task 4). No agent runs the real `claude` CLI; tests and smokes use `fake-claude`.
- **No secret in clear on disk or in a test's output.** Tests use made-up values. A test that touches Credential Manager uses an install id `test-<ULID>` and deletes what it wrote. `Config::new` keeps secrets in memory, and only the app's `Config::from_settings` uses Credential Manager.
- Values the spec fixes:
  - Variable names: `^[A-Za-z_][A-Za-z0-9_]{0,127}$`, compared without regard to case.
  - Credential Manager targets: `Dots/<install_id>/personal/<NAME>`.
  - Consumers: `mcp:<server>` and `shell`.
  - The run-event texts: "`NAME`: not granted to `mcp:<server>` in dot '`<dot>`'" and "`NAME`: not in Personal (dot '`<dot>`', `<consumer>`)".
  - The redaction mark `[secret:NAME]`; values under 8 bytes are not scrubbed.
  - Webhook tokens stored as `sha256:<hex>`, accepted as `Authorization: Bearer` only.
  - The UI copy that the tests below quote.
- No agent installs or runs an installer, runs a release build, or touches the installed Dots (`%LOCALAPPDATA%\Dots`), the real data folder `%LOCALAPPDATA%\dots`, or the person's own Credential Manager entries. Smokes use a debug build with a fresh `DOTS_DATA_DIR`.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit (the blocks hold `§`, `—` and `…`). The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`. No `fetch`, no other plugin.
- Live data is parsed, not cast, with strict schemas: a type the UI parses changes only together with the contract fixture and `ui/src/api/types.ts`, in the same commit. The fixture `ui/src/api/__fixtures__/contract.json` is written by `UPDATE_CONTRACT=1 cargo test -p dots-app contract` and never by hand. `contract.rs`'s `samples()` is at the `json!` macro's recursion limit: a new sample is added after the macro (`samples["X"] = ...`).
- Before each commit:
  - the task's own tests
  - then, as the change requires: `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`), `cargo clippy --workspace --all-targets` with no warnings, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, and `bun run --cwd ui typecheck`

  Test output must be free of warnings. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first. The UI files have no formatter; keep new lines within 100 columns where the file does.
- Known on this machine: a `vitest` run started right after an install, a `tsc` run, a cargo build or a large write can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`, fail every file at once, or fail one form-typing test under load. Run it again on its own; do not change the pool settings. Run the UI tests before the typecheck.
- Known to be flaky under load (rerun the test binary alone before treating one as a failure):
  - `runtime_local_test::live_approval_lets_the_command_run`
  - `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`
  - `claude_engine_test::a_finished_run_leaves_no_process_behind`
  - some `resume_test` tests that make worktrees
  - `claude_gateway_test::a_claude_run_reaches_its_servers_through_the_gateway_only`
- Fact-Forcing Gate (a hook of this machine): the first Edit or Write of each file, the first Bash command after a task notification, and `rm`, `git commit --amend` or a rebase are blocked until the facts it asks for are stated (callers, affected API, data shapes, the instruction, or for a destructive command what changes and how to undo it); then retry the identical call. For a large new file, write a one-line stub first.

## Deviations from the spec

Task 10 records each of these in the spec's new §14, with the reason.

| Spec at `1df8358` | This plan | Why |
|---|---|---|
| §5.2: secrets in Credential Manager, no size | At most 2,560 bytes; a longer one is refused | Credential Manager's limit per value |
| §10: one `Redactor` per run, and one process-wide | One process-wide redactor of every secret read since start | A superset, and the log writer and the store need it without a run |
| §10: approvals are scrubbed | Their stored input is scrubbed; a call Dots performs after a late decision carries `[secret:NAME]` | The stored text is what is applied; no second copy of a secret is kept |
| §11: a plugin server with a literal is held, with **Move to vault** | Literals move to Personal automatically on save, on every plugin sync, and at start for stored ones (their dots granted the names); no held state, no button | Nothing secret ever reaches the database, and nothing waits on a click |
| §8: library-time callers resolve against Personal | So does `environment_variables`, now "in Personal"; the moved-origin check compares URL text | Dots' own environment is read for nothing `${VAR}` |
| §8: base environment list | Also `PSModulePath`, `HOME`, `ProgramW6432`, `CommonProgramFiles(x86)`, `CommonProgramW6432` | The local engine's PowerShell and Unix-style tools need them |
| §11: webhook token hashed, shown once | A new dot starts without one (**Make a token**) | A token made with the dot could never be shown |
| §9: Env rows show "used by" | Left out | Not needed for Personal alone; with orgs, plan B |

The spec leaves these to the builder, and the plan settles them:

- **The module layout.** `vault/{secrets,entries,service,grants,resolve,child_env,redact,literals,upgrade}.rs`.
- **The upgrade markers.** The `settings` rows `install_id`, `vault_since`, `vault_import_answered` and `vault_backups_answered`, kept outside `Settings`.
- **Where scrubbing happens.** In the store's writers, so every caller is covered: run events, payloads, summaries, errors, approvals, notes and questions.
- **The grant sync.** It lives in `Store::create_dot` and `update_dot` (`synced_grants`), and migration 0009 backfills existing dots through `finish_0009`. The UI shows the same rule ("granted when you save") in the dot's Variables section.

## Review Focus

These are the inputs and conditions most likely to bite a person that the spec implies, each pinned to the tests that cover it:

1. **A server's config gains a `${VAR}` after a dot ticked it**, by an edit or a plugin update. Expected: the dot is not granted it; the run says which variable and which server; the editor offers **Grant**. Tests:
   - `vault_grants_test::ticking_a_server_grants_its_variables_and_a_later_one_waits` (Task 3)
   - `Variables.test.tsx` "asks before it grants what a server already used asks for now" (Task 9)
2. **A token set in Dots' own Windows environment** (an old `GITHUB_TOKEN`). Expected: it reaches neither the CLI, nor an MCP server, nor the shell. Tests:
   - `claude_engine_test::the_cli_gets_the_shell_grants_and_not_the_rest_of_dots_environment`
   - `vault_child_env_test::an_mcp_server_gets_its_own_env_but_not_dots_environment`
   - `local_tools_test::the_shell_gets_its_grants_and_not_the_rest_of_dots_environment`
   - all in Task 4
3. **A server that echoes its token**, in a tool result, in stderr or through the model. Expected: what Dots stores, shows live and logs holds `[secret:NAME]`. Tests (Task 5):
   - `vault_redact_test::what_the_store_keeps_of_a_run_holds_no_secret_it_has_read`
   - `vault_redact_test::the_log_writer_scrubs_each_line`
   - `vault_redact_test::every_common_spelling_of_a_secret_is_scrubbed`
4. **Upgrading a data folder whose servers hold typed tokens.** Expected: the values move to Personal, the dots that use those servers keep working (they are granted the names), and the database file no longer holds the values. Tests (Task 6):
   - `vault_upgrade_test::stored_secrets_move_at_start_and_their_dots_are_granted_the_names`
   - `vault_upgrade_test::the_library_never_stores_a_typed_secret`
5. **Names that differ only in case** (`github_token` and `GITHUB_TOKEN`), as Windows treats them. Expected: one variable; a second add is refused, and a grant in either case resolves it. Tests:
   - `vault_entries_test::a_name_taken_in_any_case_is_refused` (Task 2)
   - `vault_grants_test::a_name_resolves_only_when_granted_and_only_from_personal` (Task 3)

## File Map

- `crates/dots-core/src/vault/` (new), one file per part:
  - `mod.rs`, `secrets.rs` (Task 1)
  - `entries.rs`, `service.rs` (Task 2)
  - `grants.rs`, `resolve.rs` (Task 3)
  - `child_env.rs` (Task 4)
  - `redact.rs` (Task 5)
  - `literals.rs`, `upgrade.rs` (Task 6)
- `crates/dots-core/migrations/0009_vault.sql` (new in Task 2; Tasks 3 and 7).
- `crates/dots-core/src/store/`:
  - `env.rs` (new, Task 2)
  - `settings.rs` (Tasks 1 and 6)
  - `dots.rs` (Tasks 3, 6 and 7)
  - `upgrade.rs` (Tasks 3, 6 and 7)
  - `runs.rs`, `approvals.rs`, `messages.rs` (Task 5)
  - `mod.rs`, `library.rs`
- `crates/dots-core/src/`:
  - `model.rs` (Tasks 3 and 7)
  - `library_service.rs` (Tasks 3 and 6)
  - `run_tools.rs`, `runner.rs`, `runtime.rs` (Tasks 2, 3 and 6)
  - `engine/claude/{mod,once}.rs`, `engine/local/{mod,tools/mod,tools/shell}.rs`, `mcp/stdio.rs`, `bin/fake-claude.rs` (Task 4)
  - `server/webhook.rs`, `proc.rs`, `util.rs` (Task 7)
- `apps/dots-app/src/`:
  - `{commands,lib,contract}.rs` (Tasks 2, 3, 6 and 7)
  - `confirm.rs` (Task 3)
  - `startup.rs` (Task 5)
- `ui/src/api/{types,commands}.ts` and the fixture (Tasks 2, 3, 6 and 7).
- `ui/src/screens/library/{EnvTab,LibraryScreen,ServerForm}.tsx` and `ui/src/lib/query.ts` (Task 8).
- `ui/src/screens/dot-editor/{Variables,DotForm,WebhookPanel}.tsx` (Tasks 7 and 9).
- Tests:
  - new: `crates/dots-core/tests/vault_{secrets,entries,grants,child_env,redact,upgrade}_test.rs` and `ui/src/screens/dot-editor/Variables.test.tsx`
  - changed: `claude_engine_test.rs`, `claude_gateway_test.rs`, `library_service_test.rs`, `local_tools_test.rs`, `proc_test.rs`, `run_tools_test.rs`, `runtime_test.rs`, `server_test.rs`, `store_dots_test.rs`, `tests/common/mod.rs`, `ui/src/api/{contract,commands}.test.ts`, `ui/src/lib/dot-rules.test.ts`, `ui/src/screens/{DotEditorScreen,library/LibraryScreen}.test.tsx` and `ui/src/test/fixtures.ts`
- `docs/superpowers/specs/2026-10-21-env-vault-design.md` (Task 10); `docs/verification/2026-10-21-env-vault-smoke.md` (Task 11).

### Task 1: A secret store in Windows Credential Manager

The store a secret value lives in (spec §5.2): one generic credential per value, its target `Dots/<install_id>/<key>`, and an in-memory store for tests. The data folder gets an install id, made once and kept outside `Settings`.

**Files:**

- Create: `crates/dots-core/src/vault/mod.rs`, `crates/dots-core/src/vault/secrets.rs`
- Modify: `crates/dots-core/Cargo.toml`, `crates/dots-core/src/lib.rs`, `crates/dots-core/src/store/settings.rs`
- Create: `crates/dots-core/tests/vault_secrets_test.rs`

**Interfaces:**

- Consumes: `Store` (its pool), `util::new_id`.
- Produces:
  - `dots_core::vault::{SecretStore, MemorySecrets, MAX_SECRET_BYTES = 2560}` and, on Windows, `WindowsCredentials::new(install_id: &str)`.
  - The `SecretStore` trait has `put(&self, key, value) -> Result<()>`, `get(&self, key) -> Result<Option<Zeroizing<String>>>`, `delete(&self, key) -> Result<()>` (a missing key is fine) and `keys(&self, prefix) -> Result<Vec<String>>` (sorted).
  - `Store::install_id(&self) -> Result<String>`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_secrets_test.rs`:**

```rust
//! Where secret values are kept (env vault spec §5.2).

mod common;

use common::temp_store;
use dots_core::vault::{MemorySecrets, SecretStore, MAX_SECRET_BYTES};
use dots_core::Error;

fn round_trip(store: &dyn SecretStore) {
    store.put("personal/GITHUB_TOKEN", "ghp_one").unwrap();
    store.put("personal/SF_TOKEN", "sf-two").unwrap();
    store.put("other/X", "x").unwrap();
    assert_eq!(
        store
            .get("personal/GITHUB_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("ghp_one")
    );
    store.put("personal/GITHUB_TOKEN", "ghp_three").unwrap();
    assert_eq!(
        store
            .get("personal/GITHUB_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("ghp_three")
    );
    assert_eq!(
        store.keys("personal/").unwrap(),
        vec![
            "personal/GITHUB_TOKEN".to_string(),
            "personal/SF_TOKEN".to_string()
        ]
    );
    store.delete("personal/GITHUB_TOKEN").unwrap();
    store.delete("personal/GITHUB_TOKEN").unwrap();
    assert!(store.get("personal/GITHUB_TOKEN").unwrap().is_none());
    let long = "x".repeat(MAX_SECRET_BYTES + 1);
    assert!(matches!(
        store.put("personal/LONG", &long),
        Err(Error::Invalid(_))
    ));
    store
        .put("personal/FULL", &"y".repeat(MAX_SECRET_BYTES))
        .unwrap();
    for key in store.keys("").unwrap() {
        store.delete(&key).unwrap();
    }
    assert!(store.keys("").unwrap().is_empty());
}

#[test]
fn memory_secrets_keep_replace_list_and_delete_values() {
    round_trip(&MemorySecrets::new());
}

#[cfg(windows)]
#[test]
fn credential_manager_keeps_replace_lists_and_deletes_values() {
    use dots_core::vault::WindowsCredentials;
    let install = format!("test-{}", dots_core::util::new_id());
    round_trip(&WindowsCredentials::new(&install));
}

#[cfg(windows)]
#[test]
fn two_installs_never_see_each_others_secrets() {
    use dots_core::vault::WindowsCredentials;
    let a = WindowsCredentials::new(&format!("test-{}", dots_core::util::new_id()));
    let b = WindowsCredentials::new(&format!("test-{}", dots_core::util::new_id()));
    a.put("personal/SHARED_NAME", "from-a").unwrap();
    assert!(b.get("personal/SHARED_NAME").unwrap().is_none());
    assert!(b.keys("").unwrap().is_empty());
    a.delete("personal/SHARED_NAME").unwrap();
}

#[tokio::test]
async fn the_install_id_is_made_once_and_kept() {
    let (dir, store) = temp_store().await;
    let first = store.install_id().await.unwrap();
    assert_eq!(first.len(), 26);
    assert_eq!(store.install_id().await.unwrap(), first);
    store.close().await;
    let again = dots_core::store::Store::open(&dir.path().join("dots.db"))
        .await
        .unwrap();
    assert_eq!(again.install_id().await.unwrap(), first);
    // The settings the person edits do not carry it.
    let settings = serde_json::to_value(again.get_settings().await.unwrap()).unwrap();
    assert!(settings.get("install_id").is_none());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test vault_secrets_test`
Expected: the build fails: `dots_core::vault` and `Store::install_id` are not defined.

- [ ] **Step 3: Write the code**

**In `crates/dots-core/Cargo.toml`, replace:**

```toml
ulid = "1"

```

**with:**

```toml
ulid = "1"
zeroize = "1"

```

**In `crates/dots-core/Cargo.toml`, replace:**

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Security", "Win32_System_JobObjects", "Win32_System_Threading"] }

```

**with:**

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Foundation", "Win32_Security", "Win32_Security_Credentials", "Win32_System_JobObjects", "Win32_System_Threading"] }

```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod util;
pub mod workspace;
```

**with:**

```rust
pub mod util;
pub mod vault;
pub mod workspace;
```

**In `crates/dots-core/src/store/settings.rs`, replace:**

```rust
impl Store {
    /// The stored settings, with defaults for missing keys. Unknown keys are ignored, and so is
```

**with:**

```rust
impl Store {
    /// This data folder's install id: a ULID made the first time it is asked for and kept in the
    /// `settings` table under `install_id`, outside `Settings` (env vault spec §5.2). It names
    /// the folder's secrets in Credential Manager.
    pub async fn install_id(&self) -> Result<String> {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('install_id', ?)")
            .bind(Value::String(crate::util::new_id()).to_string())
            .execute(&self.pool)
            .await?;
        let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'install_id'")
            .fetch_one(&self.pool)
            .await?;
        match serde_json::from_str::<Value>(&raw)? {
            Value::String(id) => Ok(id),
            _ => Err(crate::Error::Other(
                "the stored install id is not text".into(),
            )),
        }
    }

    /// The stored settings, with defaults for missing keys. Unknown keys are ignored, and so is
```

**Create `crates/dots-core/src/vault/mod.rs`:**

```rust
//! The env vault (env vault spec): the variables and secrets dots are granted, where they are
//! kept, and how a run gets them.

pub mod secrets;

#[cfg(windows)]
pub use secrets::WindowsCredentials;
pub use secrets::{MemorySecrets, SecretStore, MAX_SECRET_BYTES};
```

**Create `crates/dots-core/src/vault/secrets.rs`:**

```rust
//! Where secret values are kept (env vault spec §5.2): Windows Credential Manager, one generic
//! credential per value, its target `Dots/<install_id>/<key>`. The install id keeps a test or
//! dev data folder away from the installed app's secrets. Tests use `MemorySecrets`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::{Error, Result};

/// The longest value Credential Manager holds (`CRED_MAX_CREDENTIAL_BLOB_SIZE`), in bytes.
pub const MAX_SECRET_BYTES: usize = 2560;

/// A store of secret values by key (`personal/<NAME>`). A key is ASCII without `*`.
pub trait SecretStore: Send + Sync {
    /// Stores `value` under `key`, replacing what was there.
    fn put(&self, key: &str, value: &str) -> Result<()>;
    /// The value under `key`, if any.
    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>>;
    /// Deletes the value under `key`; a key with none is not an error.
    fn delete(&self, key: &str) -> Result<()>;
    /// Every key that starts with `prefix`, sorted.
    fn keys(&self, prefix: &str) -> Result<Vec<String>>;
}

fn check(key: &str, value: Option<&str>) -> Result<()> {
    if key.is_empty() || !key.is_ascii() || key.contains('*') {
        return Err(Error::Invalid(format!("not a secret key: {key:?}")));
    }
    if value.is_some_and(|v| v.len() > MAX_SECRET_BYTES) {
        return Err(Error::Invalid(format!(
            "a secret is at most {MAX_SECRET_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Secrets in memory, gone when the process ends. Tests use it; so does `Config::new`.
#[derive(Default)]
pub struct MemorySecrets {
    values: Mutex<BTreeMap<String, Zeroizing<String>>>,
}

impl MemorySecrets {
    pub fn new() -> MemorySecrets {
        MemorySecrets::default()
    }
}

impl SecretStore for MemorySecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        check(key, Some(value))?;
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), Zeroizing::new(value.to_string()));
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        check(key, None)?;
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        check(key, None)?;
        self.values.lock().unwrap().remove(key);
        Ok(())
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

/// Windows Credential Manager, for the signed-in Windows user. Each value is a generic
/// credential that persists on this machine only (`CRED_PERSIST_LOCAL_MACHINE`).
#[cfg(windows)]
pub struct WindowsCredentials {
    /// `Dots/<install_id>/`.
    prefix: String,
}

#[cfg(windows)]
impl WindowsCredentials {
    pub fn new(install_id: &str) -> WindowsCredentials {
        WindowsCredentials {
            prefix: format!("Dots/{install_id}/"),
        }
    }

    fn target(&self, key: &str) -> Vec<u16> {
        wide(&format!("{}{key}", self.prefix))
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn last_error(what: &str) -> Error {
    Error::Other(format!(
        "Credential Manager: {what} failed: {}",
        std::io::Error::last_os_error()
    ))
}

#[cfg(windows)]
impl SecretStore for WindowsCredentials {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };
        check(key, Some(value))?;
        let mut target = self.target(key);
        let mut user = wide("Dots");
        let mut blob = Zeroizing::new(value.as_bytes().to_vec());
        let cred = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer in `cred` points into a buffer that lives until the call ends.
        if unsafe { CredWriteW(&cred, 0) } == 0 {
            return Err(last_error("saving a secret"));
        }
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };
        check(key, None)?;
        let target = self.target(key);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated; `cred` receives a buffer freed below.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(None);
            }
            return Err(Error::Other(format!(
                "Credential Manager: reading a secret failed: {err}"
            )));
        }
        // SAFETY: CredReadW succeeded, so `cred` points to a credential whose blob is
        // `CredentialBlobSize` bytes long; it is copied before the buffer is freed.
        let bytes = unsafe {
            let c = &*cred;
            let bytes = if c.CredentialBlob.is_null() {
                Zeroizing::new(Vec::new())
            } else {
                Zeroizing::new(
                    std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                        .to_vec(),
                )
            };
            CredFree(cred as *const _);
            bytes
        };
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Other("Credential Manager: a secret is not UTF-8".into()))?;
        Ok(Some(Zeroizing::new(text.to_string())))
    }

    fn delete(&self, key: &str) -> Result<()> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        check(key, None)?;
        let target = self.target(key);
        // SAFETY: `target` is NUL-terminated.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                return Err(Error::Other(format!(
                    "Credential Manager: deleting a secret failed: {err}"
                )));
            }
        }
        Ok(())
    }

    fn keys(&self, prefix: &str) -> Result<Vec<String>> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredEnumerateW, CredFree, CREDENTIALW};
        let filter = wide(&format!("{}{prefix}*", self.prefix));
        let mut count = 0u32;
        let mut list: *mut *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `filter` is NUL-terminated; `list` receives `count` pointers, freed below.
        if unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut list) } == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(Vec::new());
            }
            return Err(Error::Other(format!(
                "Credential Manager: listing secrets failed: {err}"
            )));
        }
        let mut keys = Vec::new();
        // SAFETY: CredEnumerateW succeeded: `list` holds `count` credentials, each with a
        // NUL-terminated target name.
        unsafe {
            for i in 0..count as usize {
                let c = &**list.add(i);
                let mut len = 0;
                while *c.TargetName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(c.TargetName, len));
                if let Some(key) = name.strip_prefix(&self.prefix) {
                    keys.push(key.to_string());
                }
            }
            CredFree(list as *const _);
        }
        keys.sort();
        Ok(keys)
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo build -p dots-core` (it adds `zeroize` to `Cargo.lock`), then `cargo test -p dots-core --test vault_secrets_test`
Expected: 4 passed. The two Windows tests write to Credential Manager under `Dots/test-<ULID>/` and delete what they wrote.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`
Expected: no warnings; every test passes.

```bash
git add Cargo.lock crates/dots-core/Cargo.toml crates/dots-core/src/lib.rs crates/dots-core/src/store/settings.rs crates/dots-core/src/vault/mod.rs crates/dots-core/src/vault/secrets.rs crates/dots-core/tests/vault_secrets_test.rs
git commit -m "feat(core): a secret store in Windows Credential Manager"
```

### Task 2: Personal variables, the vault service and the Env commands

Personal entries (spec §5.1 and §5.2): plain values in the new table `env_entries`, a secret's value in the secret store, and listings that never carry a secret's value. The runtime gets a `Vault`, and the app gets `env_list`, `env_add`, `env_set` and `env_remove`.

**Files:**

- Create: `crates/dots-core/migrations/0009_vault.sql`, `crates/dots-core/src/vault/entries.rs`, `crates/dots-core/src/vault/service.rs`, `crates/dots-core/src/store/env.rs`
- Modify: `crates/dots-core/src/vault/mod.rs`, `crates/dots-core/src/vault/secrets.rs`, `crates/dots-core/src/store/mod.rs`, `crates/dots-core/src/runtime.rs`, `apps/dots-app/src/{commands,lib,contract}.rs`, `ui/src/api/{types,commands}.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Tests: `crates/dots-core/tests/vault_entries_test.rs` (new), `ui/src/api/{contract,commands}.test.ts`

**Interfaces:**

- Consumes: Task 1's `SecretStore`, `MemorySecrets`.
- Produces:
  - `vault::entries::{EntryKind { Plain, Secret }, EnvEntry { name, kind, value: Option<String>, created_at, updated_at }, check_variable_name, check_value, MAX_VARIABLE_NAME = 128, MAX_PLAIN_BYTES = 32 * 1024}`.
  - `vault::Vault`, with:
    - `new(Store, Arc<dyn SecretStore>)`
    - `list() -> Result<Vec<EnvEntry>>`
    - `add(name, kind, value) -> Result<EnvEntry>`, which gives `Conflict` "A variable named X is already in Personal." for a name taken in any case
    - `set_value(name, value) -> Result<EnvEntry>`
    - `remove(name) -> Result<()>`
  - `vault::os_secrets(install_id) -> Arc<dyn SecretStore>`.
  - `Config::secrets: Option<Arc<dyn SecretStore>>`: `Config::new` gives memory, `from_settings` gives `None`, which means Credential Manager.
  - `Runtime::vault() -> &Vault`.
  - The `Store` gains `list_env_entries`, `find_env_entry`, `add_env_entry`, `set_env_value` and `remove_env_entry`.
  - The UI gets `T.EntryKind`, `T.EnvEntry`, and `commands.{envList, envAdd, envSet, envRemove}`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_entries_test.rs`:**

```rust
//! Personal variables (env vault spec §5): plain values in the database, secrets in the secret
//! store, and no listing that carries a secret's value.

mod common;

use std::sync::Arc;

use common::temp_store;
use dots_core::vault::{EntryKind, MemorySecrets, SecretStore, Vault, MAX_SECRET_BYTES};
use dots_core::Error;

async fn vault() -> (tempfile::TempDir, Vault, Arc<MemorySecrets>) {
    let (dir, store) = temp_store().await;
    let secrets = Arc::new(MemorySecrets::new());
    (dir, Vault::new(store, secrets.clone()), secrets)
}

#[tokio::test]
async fn a_secret_goes_to_the_secret_store_and_is_never_listed() {
    let (_dir, vault, secrets) = vault().await;
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    let added = vault
        .add("GITHUB_TOKEN", EntryKind::Secret, "ghp_s3cret")
        .await
        .unwrap();
    assert_eq!(added.value, None);
    let listed = vault.list().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|e| (e.name.as_str(), e.value.as_deref()))
            .collect::<Vec<_>>(),
        vec![("GITHUB_TOKEN", None), ("LOG_LEVEL", Some("debug"))]
    );
    assert!(!serde_json::to_string(&listed)
        .unwrap()
        .contains("ghp_s3cret"));
    assert_eq!(
        secrets
            .get("personal/GITHUB_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("ghp_s3cret")
    );
}

#[tokio::test]
async fn a_name_taken_in_any_case_is_refused() {
    let (_dir, vault, _) = vault().await;
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, "a")
        .await
        .unwrap();
    let err = vault
        .add("github_token", EntryKind::Plain, "b")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m == "A variable named github_token is already in Personal."),
        "{err}"
    );
}

#[tokio::test]
async fn names_and_values_a_child_process_cannot_take_are_refused() {
    let (_dir, vault, secrets) = vault().await;
    let long_name = "A".repeat(129);
    for name in [
        "",
        "1TOKEN",
        "MY-TOKEN",
        "MY TOKEN",
        "Ü",
        long_name.as_str(),
    ] {
        assert!(
            matches!(
                vault.add(name, EntryKind::Plain, "x").await,
                Err(Error::Invalid(_))
            ),
            "{name:?}"
        );
    }
    vault
        .add(&"A".repeat(128), EntryKind::Plain, "x")
        .await
        .unwrap();
    vault.add("_PRIVATE", EntryKind::Plain, "").await.unwrap();
    assert!(matches!(
        vault.add("NUL", EntryKind::Plain, "a\0b").await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        vault.add("EMPTY", EntryKind::Secret, "").await,
        Err(Error::Invalid(_))
    ));
    let big = "x".repeat(32 * 1024 + 1);
    assert!(matches!(
        vault.add("BIG", EntryKind::Plain, &big).await,
        Err(Error::Invalid(_))
    ));
    // A secret too long for the store leaves no row behind.
    let long = "x".repeat(MAX_SECRET_BYTES + 1);
    assert!(matches!(
        vault.add("LONG", EntryKind::Secret, &long).await,
        Err(Error::Invalid(_))
    ));
    assert!(!vault.list().await.unwrap().iter().any(|e| e.name == "LONG"));
    assert!(secrets.keys("").unwrap().is_empty());
}

#[tokio::test]
async fn a_value_is_replaced_and_a_removed_secret_leaves_the_store() {
    let (_dir, vault, secrets) = vault().await;
    vault
        .add("SF_TOKEN", EntryKind::Secret, "old")
        .await
        .unwrap();
    vault
        .add("MODE", EntryKind::Plain, "readonly")
        .await
        .unwrap();
    let replaced = vault.set_value("sf_token", "new").await.unwrap();
    assert_eq!((replaced.name.as_str(), replaced.value), ("SF_TOKEN", None));
    assert_eq!(
        secrets
            .get("personal/SF_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("new")
    );
    assert_eq!(
        vault
            .set_value("MODE", "write")
            .await
            .unwrap()
            .value
            .as_deref(),
        Some("write")
    );
    assert!(matches!(
        vault.set_value("SF_TOKEN", "").await,
        Err(Error::Invalid(_))
    ));
    vault.remove("SF_TOKEN").await.unwrap();
    assert!(secrets.get("personal/SF_TOKEN").unwrap().is_none());
    assert!(matches!(
        vault.remove("SF_TOKEN").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        vault.set_value("GONE", "x").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn the_runtime_keeps_a_tests_secrets_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dots_core::Config::new(dir.path().join("data"));
    assert!(cfg.secrets.is_some());
    let settings = dots_core::settings::Settings::default();
    assert!(dots_core::Config::from_settings(dir.path(), &settings)
        .secrets
        .is_none());
    let rt = dots_core::Runtime::start(cfg).await.unwrap();
    rt.vault()
        .add("A_TOKEN", EntryKind::Secret, "x")
        .await
        .unwrap();
    assert_eq!(rt.vault().list().await.unwrap().len(), 1);
    rt.shutdown();
}
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts

  it("refuses a Library answer in a shape it does not know", async () => {
```

**with:**

```ts

  it("sends the Env tab's commands as the core names them, and never takes a secret back", async () => {
    reply = async () => fixture.EnvEntry;
    expect(await commands.envList()).toEqual(fixture.EnvEntry);
    reply = async () => fixture.EnvEntry[0];
    await commands.envAdd("GITHUB_TOKEN", "secret", "ghp_x");
    await commands.envSet("GITHUB_TOKEN", "ghp_y");
    reply = async () => null;
    expect(await commands.envRemove("GITHUB_TOKEN")).toBeNull();
    expect(calls).toEqual([
      ["env_list", undefined],
      ["env_add", { name: "GITHUB_TOKEN", kind: "secret", value: "ghp_x" }],
      ["env_set", { name: "GITHUB_TOKEN", value: "ghp_y" }],
      ["env_remove", { name: "GITHUB_TOKEN" }],
    ]);
    reply = async () => [{ ...fixture.EnvEntry[0], secret: "ghp_x" }];
    expect((await failure(commands.envList())).kind).toBe("internal");
  });

  it("refuses a Library answer in a shape it does not know", async () => {
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  EnvVariable: T.EnvVariable.array(),
  WorktreeDiff: T.WorktreeDiff,
```

**with:**

```ts
  EnvVariable: T.EnvVariable.array(),
  EnvEntry: T.EnvEntry.array(),
  WorktreeDiff: T.WorktreeDiff,
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  FollowUpStatus: T.FollowUpStatus,
};
```

**with:**

```ts
  FollowUpStatus: T.FollowUpStatus,
  EntryKind: T.EntryKind,
};
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test vault_entries_test`
Expected: the build fails: `Vault`, `EntryKind` and `Config::secrets` are not defined.

- [ ] **Step 3: Write the code**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use dots_core::skill_creator::{check_draft, DraftRequest, DraftSummary};
use dots_core::Runtime;
```

**with:**

```rust
use dots_core::skill_creator::{check_draft, DraftRequest, DraftSummary};
use dots_core::vault::{EntryKind, EnvEntry};
use dots_core::Runtime;
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// Every message that waits for the person and the newest others, newest first; of one dot
```

**with:**

```rust

/// The personal variables of the Env tab. A secret's value is never in the answer.
#[tauri::command]
pub async fn env_list(state: State<'_, AppState>) -> Res<Vec<EnvEntry>> {
    Ok(state.rt()?.vault().list().await?)
}

/// Adds a personal variable; a secret's value goes to Credential Manager.
#[tauri::command]
pub async fn env_add(
    state: State<'_, AppState>,
    name: String,
    kind: EntryKind,
    value: String,
) -> Res<EnvEntry> {
    Ok(state.rt()?.vault().add(&name, kind, &value).await?)
}

/// Edits a plain value, or replaces a secret's without showing it.
#[tauri::command]
pub async fn env_set(state: State<'_, AppState>, name: String, value: String) -> Res<EnvEntry> {
    Ok(state.rt()?.vault().set_value(&name, &value).await?)
}

#[tauri::command]
pub async fn env_remove(state: State<'_, AppState>, name: String) -> Res<()> {
    Ok(state.rt()?.vault().remove(&name).await?)
}

/// Every message that waits for the person and the newest others, newest first; of one dot
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::skill_creator::{DraftRequest, DraftSummary};
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::skill_creator::{DraftRequest, DraftSummary};
use dots_core::vault::{EntryKind, EnvEntry};
use serde_json::{json, Value};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    ]);
    samples["DraftSummary"] = to(DraftSummary {
```

**with:**

```rust
    ]);
    samples["EnvEntry"] = to([
        EnvEntry {
            name: "GITHUB_TOKEN".into(),
            kind: EntryKind::Secret,
            value: None,
            created_at: TS.into(),
            updated_at: TS.into(),
        },
        EnvEntry {
            name: "LOG_LEVEL".into(),
            kind: EntryKind::Plain,
            value: Some("debug".into()),
            created_at: TS.into(),
            updated_at: TS.into(),
        },
    ]);
    samples["Enums"]["EntryKind"] = to(EntryKind::ALL);
    samples["DraftSummary"] = to(DraftSummary {
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    environment_variables,
    messages_list,
```

**with:**

```rust
    environment_variables,
    env_list,
    env_add,
    env_set,
    env_remove,
    messages_list,
```

**Create `crates/dots-core/migrations/0009_vault.sql`:**

```sql
-- The env vault (spec 2026-10-21-env-vault-design.md, section 5).

-- Personal variables. A secret's value is in Credential Manager (`vault::secrets`), never here:
-- its `value` is NULL. Names are unique without regard to case, as Windows environment names.
CREATE TABLE env_entries (
    name       TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    kind       TEXT NOT NULL,
    value      TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::store::Store;
use crate::workspace::{Prepared, WorkspaceManager};
```

**with:**

```rust
use crate::store::Store;
use crate::vault::{os_secrets, MemorySecrets, SecretStore, Vault};
use crate::workspace::{Prepared, WorkspaceManager};
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub draft_limit: Duration,
}
```

**with:**

```rust
    pub draft_limit: Duration,
    /// Where secret values are kept (env vault spec §5.2). `None`: the signed-in user's
    /// Credential Manager under this data folder's install id, which is what the app uses.
    /// `Config::new` keeps them in memory, so a test never writes to Credential Manager.
    pub secrets: Option<Arc<dyn SecretStore>>,
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            draft_limit: crate::skill_creator::DRAFT_LIMIT,
        }
```

**with:**

```rust
            draft_limit: crate::skill_creator::DRAFT_LIMIT,
            secrets: Some(Arc::new(MemorySecrets::new())),
        }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        cfg.claude_path = settings.claude_path.as_ref().map(PathBuf::from);
        cfg
```

**with:**

```rust
        cfg.claude_path = settings.claude_path.as_ref().map(PathBuf::from);
        cfg.secrets = None;
        cfg
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    library: LibraryService,
    skill_drafts: SkillDrafts,
```

**with:**

```rust
    library: LibraryService,
    vault: Vault,
    skill_drafts: SkillDrafts,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let settings = store.get_settings().await?;
        let bus = new_bus();
```

**with:**

```rust
        let settings = store.get_settings().await?;
        let secrets = match &cfg.secrets {
            Some(secrets) => secrets.clone(),
            None => os_secrets(&store.install_id().await?),
        };
        let vault = Vault::new(store.clone(), secrets);
        let bus = new_bus();
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
                .with_skills_dir(cfg.data_dir.join(SKILLS_DIR)),
            store,
```

**with:**

```rust
                .with_skills_dir(cfg.data_dir.join(SKILLS_DIR)),
            vault,
            store,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    /// The skill creator's drafts: what `skill_draft` and `skill_draft_cancel` call (skill
```

**with:**

```rust

    /// The personal variables of the Env tab (env vault spec §5).
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// The skill creator's drafts: what `skill_draft` and `skill_draft_cancel` call (skill
```

**Create `crates/dots-core/src/store/env.rs`:**

```rust
//! Personal variables (env vault spec §5.2). A secret's row holds no value.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::util::now;
use crate::vault::entries::{EntryKind, EnvEntry};
use crate::{Error, Result};

const ENTRY_COLUMNS: &str = "name, kind, value, created_at, updated_at";

fn entry_from_row(row: &SqliteRow) -> Result<EnvEntry> {
    Ok(EnvEntry {
        name: row.try_get("name")?,
        kind: EntryKind::parse(&row.try_get::<String, _>("kind")?)?,
        value: row.try_get("value")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn not_found(name: &str) -> Error {
    Error::NotFound(format!("no variable named {name}"))
}

impl Store {
    /// Every personal variable, by name.
    pub async fn list_env_entries(&self) -> Result<Vec<EnvEntry>> {
        let rows = sqlx::query(&format!(
            "SELECT {ENTRY_COLUMNS} FROM env_entries ORDER BY name COLLATE NOCASE"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(entry_from_row).collect()
    }

    /// The variable named `name`, in any case.
    pub async fn find_env_entry(&self, name: &str) -> Result<Option<EnvEntry>> {
        let row = sqlx::query(&format!(
            "SELECT {ENTRY_COLUMNS} FROM env_entries WHERE name = ?"
        ))
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(entry_from_row).transpose()
    }

    /// Adds a variable; `value` is `None` for a secret. A name taken in any case is a `Conflict`.
    pub async fn add_env_entry(
        &self,
        name: &str,
        kind: EntryKind,
        value: Option<&str>,
    ) -> Result<EnvEntry> {
        let ts = now();
        let added = sqlx::query(
            "INSERT INTO env_entries (name, kind, value, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(name)
        .bind(kind.as_str())
        .bind(value)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await;
        if let Err(sqlx::Error::Database(db)) = &added {
            if db.is_unique_violation() {
                return Err(Error::Conflict(format!(
                    "A variable named {name} is already in Personal."
                )));
            }
        }
        added?;
        self.find_env_entry(name)
            .await?
            .ok_or_else(|| not_found(name))
    }

    /// Sets a plain variable's value, or marks a secret's as replaced (`value` `None`).
    pub async fn set_env_value(&self, name: &str, value: Option<&str>) -> Result<EnvEntry> {
        let done = sqlx::query("UPDATE env_entries SET value = ?, updated_at = ? WHERE name = ?")
            .bind(value)
            .bind(now())
            .bind(name)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_found(name));
        }
        self.find_env_entry(name)
            .await?
            .ok_or_else(|| not_found(name))
    }

    pub async fn remove_env_entry(&self, name: &str) -> Result<()> {
        let done = sqlx::query("DELETE FROM env_entries WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_found(name));
        }
        Ok(())
    }
}
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod dots;
mod follow_ups;
```

**with:**

```rust
mod dots;
mod env;
mod follow_ups;
```

**Create `crates/dots-core/src/vault/entries.rs`:**

```rust
//! Personal variables (env vault spec §5.1): plain config, shown and edited, or a secret, which
//! is written once and never shown again.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

str_enum!(EntryKind {
    Plain => "plain",
    Secret => "secret",
});

/// The longest variable name.
pub const MAX_VARIABLE_NAME: usize = 128;
/// The longest plain value, in bytes. A secret's limit is `secrets::MAX_SECRET_BYTES`.
pub const MAX_PLAIN_BYTES: usize = 32 * 1024;

/// A personal variable as the Env tab shows it. A secret's `value` is always `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvEntry {
    pub name: String,
    pub kind: EntryKind,
    pub value: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A variable name: a letter or `_`, then letters, digits or `_`, at most 128 characters.
pub fn check_variable_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !first_ok || !rest_ok || name.len() > MAX_VARIABLE_NAME {
        return Err(Error::Invalid(format!(
            "variable name {name:?}: use 1 to {MAX_VARIABLE_NAME} characters of A-Z, a-z, 0-9 \
             or '_', starting with a letter or '_'"
        )));
    }
    Ok(())
}

/// A value a child process can be given: no NUL, a secret not empty, a plain value at most
/// `MAX_PLAIN_BYTES`.
pub fn check_value(kind: EntryKind, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(Error::Invalid("a value cannot hold a NUL character".into()));
    }
    match kind {
        EntryKind::Secret if value.is_empty() => {
            Err(Error::Invalid("a secret needs a value".into()))
        }
        EntryKind::Plain if value.len() > MAX_PLAIN_BYTES => Err(Error::Invalid(format!(
            "a plain value is at most {} KB",
            MAX_PLAIN_BYTES / 1024
        ))),
        _ => Ok(()),
    }
}
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust

pub mod secrets;

#[cfg(windows)]
```

**with:**

```rust

pub mod entries;
pub mod secrets;
pub mod service;

pub use entries::{EntryKind, EnvEntry};
#[cfg(windows)]
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust
pub use secrets::WindowsCredentials;
pub use secrets::{MemorySecrets, SecretStore, MAX_SECRET_BYTES};
```

**with:**

```rust
pub use secrets::WindowsCredentials;
pub use secrets::{os_secrets, MemorySecrets, SecretStore, MAX_SECRET_BYTES};
pub use service::Vault;
```

**In `crates/dots-core/src/vault/secrets.rs`, replace:**

```rust

/// A store of secret values by key (`personal/<NAME>`). A key is ASCII without `*`.
pub trait SecretStore: Send + Sync {
    /// Stores `value` under `key`, replacing what was there.
```

**with:**

```rust

/// A store of secret values by key (`personal/<NAME>`). A key is ASCII without `*`. Its
/// `Debug` never shows a value.
pub trait SecretStore: Send + Sync + std::fmt::Debug {
    /// Stores `value` under `key`, replacing what was there.
```

**In `crates/dots-core/src/vault/secrets.rs`, replace:**

```rust
    values: Mutex<BTreeMap<String, Zeroizing<String>>>,
}
```

**with:**

```rust
    values: Mutex<BTreeMap<String, Zeroizing<String>>>,
}

impl std::fmt::Debug for MemorySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.values.lock().map(|v| v.len()).unwrap_or(0);
        f.debug_struct("MemorySecrets")
            .field("count", &count)
            .finish()
    }
}
```

**In `crates/dots-core/src/vault/secrets.rs`, replace:**

```rust
#[cfg(windows)]
pub struct WindowsCredentials {
```

**with:**

```rust
#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsCredentials {
```

**In `crates/dots-core/src/vault/secrets.rs`, replace:**

```rust
        wide(&format!("{}{key}", self.prefix))
    }
```

**with:**

```rust
        wide(&format!("{}{key}", self.prefix))
    }
}

/// The secret store of the signed-in user for the data folder `install_id`: Credential Manager
/// on Windows, memory elsewhere (Dots ships on Windows; a keychain is later).
pub fn os_secrets(install_id: &str) -> std::sync::Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        std::sync::Arc::new(WindowsCredentials::new(install_id))
    }
    #[cfg(not(windows))]
    {
        tracing::warn!("no keychain on this system: secrets of {install_id} last until Dots ends");
        std::sync::Arc::new(MemorySecrets::new())
    }
```

**Create `crates/dots-core/src/vault/service.rs`:**

```rust
//! The personal variables of the Env tab (env vault spec §5): a plain value lives in the
//! database, a secret's in the secret store, and a listing never carries a secret's value.

use std::sync::Arc;

use super::entries::{check_value, check_variable_name, EntryKind, EnvEntry};
use super::secrets::SecretStore;
use crate::store::Store;
use crate::Result;

/// The secret store key of the personal variable `name`.
fn personal_key(name: &str) -> String {
    format!("personal/{name}")
}

#[derive(Clone)]
pub struct Vault {
    store: Store,
    secrets: Arc<dyn SecretStore>,
}

impl Vault {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>) -> Vault {
        Vault { store, secrets }
    }

    /// Every personal variable (`env_list`). A secret's `value` is `None`.
    pub async fn list(&self) -> Result<Vec<EnvEntry>> {
        self.store.list_env_entries().await
    }

    /// Adds a personal variable (`env_add`). A secret's value goes to the secret store; if it
    /// cannot be stored there, the row is taken out again.
    pub async fn add(&self, name: &str, kind: EntryKind, value: &str) -> Result<EnvEntry> {
        check_variable_name(name)?;
        check_value(kind, value)?;
        match kind {
            EntryKind::Plain => self.store.add_env_entry(name, kind, Some(value)).await,
            EntryKind::Secret => {
                let entry = self.store.add_env_entry(name, kind, None).await?;
                if let Err(e) = self.secrets.put(&personal_key(&entry.name), value) {
                    self.store.remove_env_entry(&entry.name).await?;
                    return Err(e);
                }
                Ok(entry)
            }
        }
    }

    /// Replaces a personal variable's value (`env_set`): a plain one's is edited, a secret's
    /// replaced without being shown.
    pub async fn set_value(&self, name: &str, value: &str) -> Result<EnvEntry> {
        let entry = self.get(name).await?;
        check_value(entry.kind, value)?;
        match entry.kind {
            EntryKind::Plain => self.store.set_env_value(&entry.name, Some(value)).await,
            EntryKind::Secret => {
                self.secrets.put(&personal_key(&entry.name), value)?;
                self.store.set_env_value(&entry.name, None).await
            }
        }
    }

    /// Removes a personal variable (`env_remove`): a secret's value first, so a failure there
    /// keeps the row to try again.
    pub async fn remove(&self, name: &str) -> Result<()> {
        let entry = self.get(name).await?;
        if entry.kind == EntryKind::Secret {
            self.secrets.delete(&personal_key(&entry.name))?;
        }
        self.store.remove_env_entry(&entry.name).await
    }

    async fn get(&self, name: &str) -> Result<EnvEntry> {
        self.store
            .find_env_entry(name)
            .await?
            .ok_or_else(|| crate::Error::NotFound(format!("no variable named {name}")))
    }
}
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  DraftRequest,
  LibraryConfig,
```

**with:**

```ts
  DraftRequest,
  EntryKind,
  LibraryConfig,
```

**In `ui/src/api/commands.ts`, replace:**

```ts
    call(T.EnvVariable.array(), "environment_variables", { names }),

```

**with:**

```ts
    call(T.EnvVariable.array(), "environment_variables", { names }),
  /// The personal variables; a secret's value is never in the answer.
  envList: () => call(T.EnvEntry.array(), "env_list"),
  /// Rejects with kind `conflict` when the name is taken in any case.
  envAdd: (name: string, kind: EntryKind, value: string) =>
    call(T.EnvEntry, "env_add", { name, kind, value }),
  /// Edits a plain value, or replaces a secret's.
  envSet: (name: string, value: string) => call(T.EnvEntry, "env_set", { name, value }),
  envRemove: (name: string) => call(nothing, "env_remove", { name }),

```

**In `ui/src/api/types.ts`, replace:**

```ts
export const EnvVariable = z.strictObject({ name: z.string(), set: z.boolean() });

```

**with:**

```ts
export const EnvVariable = z.strictObject({ name: z.string(), set: z.boolean() });
export const EntryKind = z.enum(["plain", "secret"]);
// A personal variable of the Env tab. A secret's value never leaves the core: it is null.
export const EnvEntry = z.strictObject({
  name: z.string(),
  kind: EntryKind,
  value: nullable(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
});

```

**In `ui/src/api/types.ts`, replace:**

```ts
export type EnvVariable = z.infer<typeof EnvVariable>;
export type FileDiff = z.infer<typeof FileDiff>;
```

**with:**

```ts
export type EnvVariable = z.infer<typeof EnvVariable>;
export type EntryKind = z.infer<typeof EntryKind>;
export type EnvEntry = z.infer<typeof EnvEntry>;
export type FileDiff = z.infer<typeof FileDiff>;
```

- [ ] **Step 4: Run the tests and write the contract**

Run: `cargo test -p dots-core --test vault_entries_test --test vault_secrets_test`, then `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app` and `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
Expected: 5 and 4 passed; app 94 passed; UI 450 tests in 31 files.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`, `bun run --cwd ui typecheck`
Expected: no warnings; every test passes.

```bash
git add crates/dots-core/migrations/0009_vault.sql crates/dots-core/src/vault crates/dots-core/src/store/env.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/vault_entries_test.rs apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs apps/dots-app/src/contract.rs ui/src/api/types.ts ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/commands.test.ts ui/src/api/__fixtures__/contract.json
git commit -m "feat: personal variables in the vault, with the Env commands"
```

### Task 3: Grants, and `${VAR}` resolved from Personal

A dot's grants (spec §7) and the resolver (spec §8). A run's MCP servers and its shell resolve only what is granted to them, and only from Personal; what does not resolve leaves the server out, with a run event naming the variable. The Library's probe, check, missing variables and `environment_variables` read Personal, and so does the native confirmation's origin. Migration 0009 grants every existing dot the variables of the servers it uses.

**Files:**

- Create: `crates/dots-core/src/vault/grants.rs`, `crates/dots-core/src/vault/resolve.rs`
- Modify:
  - `crates/dots-core/migrations/0009_vault.sql`
  - `crates/dots-core/src/{model,library_service,run_tools,runner,runtime}.rs`
  - `crates/dots-core/src/store/{dots,library,mod,upgrade}.rs`
  - `crates/dots-core/src/vault/{mod,service}.rs`
  - `apps/dots-app/src/{commands,confirm}.rs`
  - `ui/src/api/types.ts`, `ui/src/lib/dot-rules.ts`
- Regenerate: the contract fixture
- Tests: `crates/dots-core/tests/vault_grants_test.rs` (new); `claude_gateway_test.rs`, `library_service_test.rs`, `run_tools_test.rs`

**Interfaces:**

- Consumes: Task 2's `Vault`, `check_variable_name`.
- Produces:
  - `vault::{Grants(BTreeMap<String, Vec<String>>), Consumer { Mcp(String), Shell }, SHELL}`.
  - `Grants` has `names(&Consumer)`, `allows(&Consumer, name)`, `validate()` and `synced(had: &[String], using: &[(String, Vec<String>)]) -> Grants`.
  - `Consumer` has `key()` and `parse(key)`.
  - `vault::{Resolved, Missing { name, consumer, why }, Unresolved { NotGranted, NotDefined }}`, with `Missing::text(dot) -> String`.
  - `Vault` gains:
    - `resolve(&Grants, &Consumer, &[String]) -> Result<Result<Resolved, Missing>>`
    - `personal_values(&[String]) -> Result<Resolved>`
    - `in_memory(Store)`
    - `pub(crate) personal_value`
  - `DotSpec::variables: Grants`.
  - `Store::synced_grants(before: Option<&Uses>, &DotSpec)` (crate).
  - `OpenRequest` gains `vault`, `grants` and `dot_name`.
  - `RunTools::shell_env() -> &[(String, Zeroizing<String>)]`.
  - `Runner::set_vault(Vault)`.
  - `LibraryService::with_vault(Vault)`; `environment_variables` becomes `async` and returns `Result`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
use dots_core::store::Store;
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::store::Store;
use dots_core::vault::{Grants, Vault};
use serde_json::{json, Value};
```

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
```

**with:**

```rust
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
```

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));

```

**with:**

```rust
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));

```

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    assert!(tools.server("plugin-forms-files").is_some());
```

**with:**

```rust
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    assert!(tools.server("plugin-forms-files").is_some());
```

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
            cancel: &cancel,
        })
```

**with:**

```rust
            cancel: &cancel,
            vault: &Vault::in_memory(e.store.clone()),
            grants: &Grants::default(),
            dot_name: "gateway",
        })
```

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
```

**with:**

```rust
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
```

**In `crates/dots-core/tests/claude_gateway_test.rs`, replace:**

```rust
        cancel: &cancel,
    })
```

**with:**

```rust
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        grants: &Grants::default(),
        dot_name: "gateway",
    })
```

**In `crates/dots-core/tests/library_service_test.rs`, replace:**

```rust
use dots_core::store::Store;
use dots_core::Error;
```

**with:**

```rust
use dots_core::store::Store;
use dots_core::vault::{EntryKind, Grants, Vault};
use dots_core::Error;
```

**In `crates/dots-core/tests/library_service_test.rs`, replace:**

```rust
#[tokio::test]
async fn environment_variables_say_which_are_set_and_never_their_values() {
    let (_dir, store) = temp_store().await;
```

**with:**

```rust
#[tokio::test]
async fn environment_variables_say_which_personal_has_and_never_their_values() {
    let (_dir, store) = temp_store().await;
```

**In `crates/dots-core/tests/library_service_test.rs`, replace:**

```rust
    let bus = new_bus();
    let lib = service(&store, &bus);
    std::env::set_var("DOTS_T5_SET_VAR", "secret-value");
    let vars = lib.environment_variables(&[
        "DOTS_T5_SET_VAR".to_string(),
        "DOTS_T5_NEVER_SET_ENV".to_string(),
    ]);
    let v = serde_json::to_value(&vars).unwrap();
```

**with:**

```rust
    let bus = new_bus();
    let vault = Vault::in_memory(store.clone());
    vault
        .add("DOTS_T5_SET_VAR", EntryKind::Secret, "secret-value")
        .await
        .unwrap();
    // Dots' own environment no longer counts (env vault spec §8).
    std::env::set_var("DOTS_T5_NEVER_SET_ENV", "from-the-environment");
    let lib = service(&store, &bus).with_vault(vault);
    let vars = lib
        .environment_variables(&[
            "DOTS_T5_SET_VAR".to_string(),
            "DOTS_T5_NEVER_SET_ENV".to_string(),
        ])
        .await
        .unwrap();
    let v = serde_json::to_value(&vars).unwrap();
```

**In `crates/dots-core/tests/library_service_test.rs`, replace:**

```rust
                plugins_dir: None,
                cancel: &CancellationToken::new(),
            })
            .await
```

**with:**

```rust
                plugins_dir: None,
                cancel: &CancellationToken::new(),
                vault: &Vault::in_memory(store.clone()),
                grants: &Grants::default(),
                dot_name: "forms",
            })
            .await
```

**In `crates/dots-core/tests/library_service_test.rs`, replace:**

```rust
                cancel: &CancellationToken::new(),
            })
```

**with:**

```rust
                cancel: &CancellationToken::new(),
                vault: &Vault::in_memory(store.clone()),
                grants: &Grants::default(),
                dot_name: "forms",
            })
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, PluginConfig, SkillConfig, Uses,
};
```

**with:**

```rust
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, McpServerConfig, PluginConfig, SkillConfig, Uses,
};
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
use dots_core::store::Store;
use serde_json::json;
```

**with:**

```rust
use dots_core::store::Store;
use dots_core::vault::{Grants, Vault};
use serde_json::json;
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
            cancel: &CancellationToken::new(),
        })
```

**with:**

```rust
            cancel: &CancellationToken::new(),
            vault: &Vault::in_memory(self.store.clone()),
            grants: &Grants::default(),
            dot_name: "tools",
        })
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
    f.accepted_server("good", &good).await;
    let bad = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let bad_cfg = LibraryConfig::McpServer(
        bad.stdio_with(vec!["${DOTS_T5_NEVER_SET_RUN}".into()], Default::default()),
    );
    let bad_id = f.lib.add("bad", bad_cfg, None).await.unwrap().id;
```

**with:**

```rust
    f.accepted_server("good", &good).await;
    // A program that is not there: a variable that does not resolve is the dot's grants'
    // business (`vault_grants_test`), not the server's.
    let bad_cfg = LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "dots-t5-no-such-program".into(),
        args: Vec::new(),
        cwd: None,
        env: Default::default(),
    });
    let bad_id = f.lib.add("bad", bad_cfg, None).await.unwrap().id;
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
        notices,
        ["bad: could not start: DOTS_T5_NEVER_SET_RUN is not set"]
    );
```

**with:**

```rust
        notices,
        ["bad: could not start: dots-t5-no-such-program was not found"]
    );
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
        catalog.error.as_deref(),
        Some("could not start: DOTS_T5_NEVER_SET_RUN is not set")
    );
```

**with:**

```rust
        catalog.error.as_deref(),
        Some("could not start: dots-t5-no-such-program was not found")
    );
```

**In `crates/dots-core/tests/run_tools_test.rs`, replace:**

```rust
        cancel: &cancel,
    })
```

**with:**

```rust
        cancel: &cancel,
        vault: &Vault::in_memory(f.store.clone()),
        grants: &Grants::default(),
        dot_name: "tools",
    })
```

**Create `crates/dots-core/tests/vault_grants_test.rs`:**

```rust
//! What a dot is granted and how its variables resolve (env vault spec §7, §8): a server the
//! dot ticks is granted the variables of its config, a name resolves only when it is granted
//! and only from Personal, and what does not resolve leaves the server out with the reason.

mod common;

use std::collections::BTreeMap;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store};
use dots_core::events::new_bus;
use dots_core::library::{LibraryConfig, McpServerConfig, PluginConfig, Uses};
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::store::Store;
use dots_core::vault::{Consumer, EntryKind, Grants, Missing, Unresolved, Vault};
use dots_core::Error;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tokio_util::sync::CancellationToken;

fn grants(pairs: &[(&str, &[&str])]) -> Grants {
    Grants(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect(),
    )
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn server_with(env: &[(&str, &str)]) -> LibraryConfig {
    LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    })
}

#[test]
fn a_consumer_is_a_server_or_the_shell() {
    assert_eq!(Consumer::Mcp("github".into()).key(), "mcp:github");
    assert_eq!(Consumer::Shell.key(), "shell");
    assert_eq!(
        Consumer::parse("mcp:github").unwrap(),
        Consumer::Mcp("github".into())
    );
    assert_eq!(Consumer::parse("shell").unwrap(), Consumer::Shell);
    for bad in ["github", "mcp:", "mcp:Git Hub", "mcp:dots", "Shell"] {
        assert!(
            matches!(Consumer::parse(bad), Err(Error::Invalid(_))),
            "{bad}"
        );
    }
}

#[test]
fn grants_name_consumers_and_variables_once() {
    grants(&[("mcp:github", &["GITHUB_TOKEN"]), ("shell", &["LOG_LEVEL"])])
        .validate()
        .unwrap();
    for bad in [
        grants(&[("github", &["A"])]),
        grants(&[("shell", &["1A"])]),
        grants(&[("shell", &["A", "a"])]),
    ] {
        assert!(matches!(bad.validate(), Err(Error::Invalid(_))), "{bad:?}");
    }
    let g = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);
    assert!(g.allows(&Consumer::Mcp("github".into()), "github_token"));
    assert!(!g.allows(&Consumer::Mcp("other".into()), "GITHUB_TOKEN"));
    assert!(!g.allows(&Consumer::Shell, "GITHUB_TOKEN"));
}

#[test]
fn a_save_grants_a_newly_used_server_its_variables_and_drops_an_unused_one() {
    let given = grants(&[
        ("shell", &["LOG_LEVEL"]),
        ("mcp:kept", &["A"]),
        ("mcp:gone", &["B"]),
    ]);
    let using = vec![
        ("kept".to_string(), names(&["A", "NEW_IN_CONFIG"])),
        ("fresh".to_string(), names(&["C", "D"])),
    ];
    let synced = given.synced(&names(&["kept", "gone"]), &using);
    assert_eq!(
        synced,
        grants(&[
            ("shell", &["LOG_LEVEL"]),
            // Still used: kept as given. A variable its config gained is not granted.
            ("mcp:kept", &["A"]),
            // Newly used: granted what its config reads.
            ("mcp:fresh", &["C", "D"]),
        ])
    );
}

#[tokio::test]
async fn ticking_a_server_grants_its_variables_and_a_later_one_waits() {
    let (_dir, store) = temp_store().await;
    let gh = store
        .add_library_item(
            "github",
            &server_with(&[("TOKEN", "${GITHUB_TOKEN}")]),
            None,
        )
        .await
        .unwrap();
    store
        .add_library_item("plain", &server_with(&[("MODE", "readonly")]), None)
        .await
        .unwrap();
    let mut s = spec("triage");
    s.uses.mcp_servers = names(&["github", "plain"]);
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(
        dot.spec.variables,
        grants(&[("mcp:github", &["GITHUB_TOKEN"])])
    );

    // The server's config gains a variable: the dot is not granted it.
    store
        .update_library_item(
            &gh.id,
            &server_with(&[("TOKEN", "${GITHUB_TOKEN}"), ("ORG", "${GITHUB_ORG}")]),
        )
        .await
        .unwrap();
    let saved = store.update_dot(&dot.id, &dot.spec).await.unwrap();
    assert_eq!(
        saved.spec.variables,
        grants(&[("mcp:github", &["GITHUB_TOKEN"])])
    );

    // Granting it by hand keeps it; unticking the server drops its grants.
    let mut granted = saved.spec.clone();
    granted.variables = grants(&[
        ("mcp:github", &["GITHUB_TOKEN", "GITHUB_ORG"]),
        ("shell", &["LOG_LEVEL"]),
    ]);
    let saved = store.update_dot(&dot.id, &granted).await.unwrap();
    assert_eq!(saved.spec.variables, granted.variables);
    let mut unticked = saved.spec.clone();
    unticked.uses.mcp_servers = names(&["plain"]);
    let saved = store.update_dot(&dot.id, &unticked).await.unwrap();
    assert_eq!(saved.spec.variables, grants(&[("shell", &["LOG_LEVEL"])]));

    // A grant for something that is not a consumer is refused.
    let mut bad = saved.spec.clone();
    bad.variables = grants(&[("github", &["GITHUB_TOKEN"])]);
    assert!(matches!(
        store.update_dot(&dot.id, &bad).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn ticking_a_plugin_grants_the_variables_of_its_servers() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let root = dir.path().join("forms");
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"forms"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join(".mcp.json"),
        json!({"mcpServers": {"db": {"command": "node", "env": {"T": "${DB_TOKEN}"}}}}).to_string(),
    )
    .unwrap();
    let lib = dots_core::library_service::LibraryService::new(store.clone(), bus);
    lib.add(
        "forms",
        LibraryConfig::Plugin(PluginConfig {
            path: root.to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        }),
        None,
    )
    .await
    .unwrap();
    let mut s = spec("filler");
    s.uses.plugins = names(&["forms"]);
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(
        dot.spec.variables,
        grants(&[("mcp:plugin-forms-db", &["DB_TOKEN"])])
    );
}

#[tokio::test]
async fn a_name_resolves_only_when_granted_and_only_from_personal() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, "ghp_value")
        .await
        .unwrap();
    std::env::set_var("DOTS_VAULT_ONLY_IN_ENV", "from-the-environment");
    let github = Consumer::Mcp("github".into());
    let g = grants(&[("mcp:github", &["github_token", "DOTS_VAULT_ONLY_IN_ENV"])]);

    let ok = vault
        .resolve(&g, &github, &names(&["GITHUB_TOKEN"]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.get("GITHUB_TOKEN").as_deref(), Some("ghp_value"));

    let missing = vault
        .resolve(
            &g,
            &github,
            &names(&["GITHUB_TOKEN", "DOTS_VAULT_ONLY_IN_ENV"]),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.why, Unresolved::NotDefined);
    assert_eq!(
        missing.text("triage"),
        "DOTS_VAULT_ONLY_IN_ENV: not in Personal (dot 'triage', mcp:github)"
    );

    let refused: Missing = vault
        .resolve(&g, &Consumer::Shell, &names(&["GITHUB_TOKEN"]))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        refused.text("triage"),
        "GITHUB_TOKEN: not granted to shell in dot 'triage'"
    );
}

async fn open(
    store: &Store,
    vault: &Vault,
    uses: &Uses,
    grants: &Grants,
    dir: &std::path::Path,
) -> (RunTools, Vec<String>) {
    RunTools::open(OpenRequest {
        store,
        bus: &new_bus(),
        run_id: "01RUN",
        uses,
        workspace: dir,
        plugins_dir: None,
        cancel: &CancellationToken::new(),
        vault,
        grants,
        dot_name: "triage",
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_run_starts_a_server_with_its_granted_values_and_leaves_out_one_without() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    vault
        .add("GH_TOKEN", EntryKind::Secret, "ghp_run")
        .await
        .unwrap();
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    let fake = FakeMcp::new(json!({"tools": [tool("search", true)]}));
    let config = fake.stdio_with(
        Vec::new(),
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "${GH_TOKEN}".to_string())]),
    );
    let item = store
        .add_library_item("gh", &LibraryConfig::McpServer(config), None)
        .await
        .unwrap();
    let uses = Uses {
        mcp_servers: names(&["gh"]),
        ..Default::default()
    };

    // Not granted: left out, said why, and the Library's catalog is not marked.
    let none = grants(&[("shell", &["LOG_LEVEL", "NOT_THERE"])]);
    let (tools, notices) = open(&store, &vault, &uses, &none, dir.path()).await;
    assert!(tools.servers().is_empty());
    assert!(fake.log().is_empty());
    assert!(notices.contains(&"GH_TOKEN: not granted to mcp:gh in dot 'triage'".to_string()));
    assert!(notices.contains(&"NOT_THERE: not in Personal (dot 'triage', shell)".to_string()));
    let shell: Vec<(&str, &str)> = tools
        .shell_env()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(shell, [("LOG_LEVEL", "debug")]);
    let catalog = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(catalog.error, None);
    tools.close().await;

    // Granted: the server starts with the value.
    let granted = grants(&[("mcp:gh", &["GH_TOKEN"])]);
    let (tools, notices) = open(&store, &vault, &uses, &granted, dir.path()).await;
    assert_eq!(tools.servers().len(), 1, "{notices:?}");
    let start = fake
        .log()
        .into_iter()
        .find_map(|r| r.get("start").cloned())
        .unwrap();
    assert_eq!(start["env"]["FAKE_MCP_TOKEN"], "ghp_run");
    tools.close().await;
}

/// Migration 0009 on a database Dots left at 0008: each dot is granted the variables of the
/// servers it uses.
#[tokio::test]
async fn migration_0009_grants_each_dot_the_variables_of_its_servers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 9)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, updated_at) \
         VALUES ('S1', 'mcp_server', 'github', ?, NULL, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(
        json!({"transport": "http", "url": "https://api.github.com/mcp",
               "headers": {"Authorization": "Bearer ${GITHUB_TOKEN}"}})
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    let policy = serde_json::to_string(&dots_core::policy::Policy::preset(
        dots_core::policy::Preset::Sandboxed,
    ))
    .unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
         webhook_token, policy, uses, max_turns, timeout_secs, approval_wait_secs, \
         created_at, updated_at) VALUES ('D1', 'triage', 'Do it.', 'claude', 'sonnet', \
         'C:/tmp/repo', 'worktree', 't', ?, ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(&policy)
    .bind(json!({"mcp_servers": ["github"]}).to_string())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let want = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);
    assert_eq!(store.get_dot("D1").await.unwrap().spec.variables, want);
    store.close().await;
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.get_dot("D1").await.unwrap().spec.variables, want);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test vault_grants_test`
Expected: the build fails: `vault::Grants`, `Consumer` and `DotSpec::variables` are not defined.

- [ ] **Step 3: Write the code**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// Which of the names Dots' environment sets. Never a value.
#[tauri::command]
pub fn environment_variables(
    state: State<'_, AppState>,
```

**with:**

```rust

/// Which of the names Personal has. Never a value.
#[tauri::command]
pub async fn environment_variables(
    state: State<'_, AppState>,
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
) -> Res<Vec<EnvVariable>> {
    Ok(state.rt()?.library().environment_variables(&names))
}
```

**with:**

```rust
) -> Res<Vec<EnvVariable>> {
    Ok(state.rt()?.library().environment_variables(&names).await?)
}
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
) -> Option<String> {
    library_confirmation_in(stored, next, plugin_starts, &process_env)
}

/// Dots' own environment, where the server's `${VAR}` is read when it starts.
fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}
```

**with:**

```rust
) -> Option<String> {
    library_confirmation_in(stored, next, plugin_starts, &|_| None)
}
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
/// name `${VAR}`, in the plugin folder `config` names, for the question
/// of `library_confirmation`. Only when the plugin is new (`stored` is `None`) or its path
/// changed; a folder that cannot be read gives none, and the core then refuses it.
fn plugin_starts(stored: Option<&LibraryConfig>, config: &LibraryConfig) -> Vec<String> {
    let LibraryConfig::Plugin(next) = config else {
        return Vec::new();
    };
    if matches!(stored, Some(LibraryConfig::Plugin(old)) if old.path == next.path) {
        return Vec::new();
    }
    let Ok(info) = dots_core::kit::read_plugin(std::path::Path::new(&next.path)) else {
        return Vec::new();
```

**with:**

```rust
/// name `${VAR}`, in the plugin folder `config` names, for the question
/// of `library_confirmation`, read from `info` (`plugin_info`), each origin with `${VAR}` read
/// through `env`.
fn plugin_starts(
    info: Option<&dots_core::kit::PluginInfo>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let Some(info) = info else {
        return Vec::new();
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
                        variables.join(", "),
                        origin_text(url, &process_env)
                    )
```

**with:**

```rust
                        variables.join(", "),
                        origin_text(url, env)
                    )
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust

/// Asks `library_confirmation`'s question, if there is one. A refusal is `cancelled`, and the
/// caller then starts and stores nothing.
```

**with:**

```rust

/// The plugin folder `config` names, read for the question of `library_confirmation`: only
/// when the plugin is new (`stored` is `None`) or its path changed. A folder that cannot be
/// read gives none, and the core then refuses it.
fn plugin_info(
    stored: Option<&LibraryConfig>,
    config: &LibraryConfig,
) -> Option<dots_core::kit::PluginInfo> {
    let LibraryConfig::Plugin(next) = config else {
        return None;
    };
    if matches!(stored, Some(LibraryConfig::Plugin(old)) if old.path == next.path) {
        return None;
    }
    dots_core::kit::read_plugin(std::path::Path::new(&next.path)).ok()
}

/// Asks `library_confirmation`'s question, if there is one, with each origin computed from
/// Personal's values, where the server's `${VAR}` resolve. A refusal is `cancelled`, and the
/// caller then starts and stores nothing.
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
async fn confirm_library<F, Fut>(
    stored: Option<&LibraryConfig>,
```

**with:**

```rust
async fn confirm_library<F, Fut>(
    rt: &Runtime,
    stored: Option<&LibraryConfig>,
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
{
    if let Some(question) = library_confirmation(stored, next, &plugin_starts(stored, next)) {
        if !ask(question).await {
```

**with:**

```rust
{
    let info = plugin_info(stored, next);
    let mut names = Vec::new();
    for config in [stored, Some(next)].into_iter().flatten() {
        if let LibraryConfig::McpServer(server) = config {
            names.extend(server.variables());
        }
    }
    for server in info.iter().flat_map(|i| &i.servers) {
        names.extend(server.config.variables());
    }
    let personal = rt.vault().personal_values(&names).await?;
    let env = |k: &str| personal.get(k);
    let starts = plugin_starts(info.as_ref(), &env);
    if let Some(question) = library_confirmation_in(stored, next, &starts, &env) {
        if !ask(question).await {
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
    next.validate()?;
    confirm_library(None, &next, ask).await?;
    let LibraryConfig::McpServer(config) = next else {
```

**with:**

```rust
    next.validate()?;
    confirm_library(rt, None, &next, ask).await?;
    let LibraryConfig::McpServer(config) = next else {
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
    }
    confirm_library(None, &config, ask).await?;
    Ok(rt.library().add(name, config, accept).await?)
```

**with:**

```rust
    }
    confirm_library(rt, None, &config, ask).await?;
    Ok(rt.library().add(name, config, accept).await?)
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
        }
        confirm_library(Some(&stored.config), &config, ask).await?;
    }
```

**with:**

```rust
        }
        confirm_library(rt, Some(&stored.config), &config, ask).await?;
    }
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
    use super::*;
    use crate::error::CmdErrorKind;
```

**with:**

```rust
    use super::*;

    /// `plugin_starts` of the plugin `config` names, with no variable set.
    fn starts_of(stored: Option<&LibraryConfig>, config: &LibraryConfig) -> Vec<String> {
        plugin_starts(plugin_info(stored, config).as_ref(), &|_| None)
    }
    use crate::error::CmdErrorKind;
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
        // New: the stdio server's command line; the HTTP one starts nothing.
        assert_eq!(plugin_starts(None, &config), [r#""node" "a.js""#]);
        // Same path: not read, not asked again.
        assert!(plugin_starts(Some(&config), &config).is_empty());
        // Another path: read again.
```

**with:**

```rust
        // New: the stdio server's command line; the HTTP one starts nothing.
        assert_eq!(starts_of(None, &config), [r#""node" "a.js""#]);
        // Same path: not read, not asked again.
        assert!(starts_of(Some(&config), &config).is_empty());
        // Another path: read again.
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
        let elsewhere = at(std::path::Path::new("C:/no/such/plugin"));
        assert_eq!(
            plugin_starts(Some(&elsewhere), &config),
            [r#""node" "a.js""#]
        );
        // Unreadable: nothing here; the core refuses the folder.
        assert!(plugin_starts(None, &elsewhere).is_empty());
        // Not a plugin.
        assert!(plugin_starts(None, &stdio("x", &[], None)).is_empty());
    }
```

**with:**

```rust
        let elsewhere = at(std::path::Path::new("C:/no/such/plugin"));
        assert_eq!(starts_of(Some(&elsewhere), &config), [r#""node" "a.js""#]);
        // Unreadable: nothing here; the core refuses the folder.
        assert!(starts_of(None, &elsewhere).is_empty());
        // Not a plugin.
        assert!(starts_of(None, &stdio("x", &[], None)).is_empty());
    }
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
        });
        let starts = plugin_starts(None, &config);
        assert_eq!(starts, [r#"the values of TOKEN to "https://h.example:9""#]);
```

**with:**

```rust
        });
        let starts = starts_of(None, &config);
        assert_eq!(starts, [r#"the values of TOKEN to "https://h.example:9""#]);
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
        .unwrap();
        let none = plugin_starts(None, &config);
        assert!(none.is_empty());
```

**with:**

```rust
        .unwrap();
        let none = starts_of(None, &config);
        assert!(none.is_empty());
```

**In `crates/dots-core/migrations/0009_vault.sql`, replace:**

```sql
);
```

**with:**

```sql
);

-- What each dot's consumers may resolve (section 7): `{"mcp:<server>": [names], "shell": [...]}`.
ALTER TABLE dots ADD COLUMN variables TEXT NOT NULL DEFAULT '{}';

-- Each dot is granted the variables of the servers it uses when the store opens
-- (`store/upgrade.rs`, `finish_0009`), as ticking them would. The rows wait here, and the table
-- is dropped once they are done.
CREATE TABLE upgrade_0009 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0009 (dot_id) SELECT id FROM dots;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
use crate::store::Store;
use crate::{Error, Result};
```

**with:**

```rust
use crate::store::Store;
use crate::vault::{Resolved, Vault};
use crate::{Error, Result};
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust

/// Dots' own environment, where `${VAR}` is read.
pub(crate) fn process_env(key: &str) -> Option<String> {
```

**with:**

```rust

/// Dots' own environment, where Claude Code's plugin list is looked up (`kit::installed`).
/// `${VAR}` is never read here: it resolves from the vault (env vault spec §8).
pub(crate) fn process_env(key: &str) -> Option<String> {
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust

/// Whether a variable is set in Dots' environment. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
```

**with:**

```rust

/// Whether Personal has a variable. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    config: &McpServerConfig,
    options: SessionOptions,
```

**with:**

```rust
    config: &McpServerConfig,
    env: &Resolved,
    options: SessionOptions,
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
) -> std::result::Result<(McpSession, Vec<McpTool>), McpError> {
    let session = McpSession::open(name, config, &process_env, options).await?;
    match session.list_tools().await {
```

**with:**

```rust
) -> std::result::Result<(McpSession, Vec<McpTool>), McpError> {
    let session = McpSession::open(name, config, &|k| env.get(k), options).await?;
    match session.list_tools().await {
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    skills_dir: Option<PathBuf>,
}
```

**with:**

```rust
    skills_dir: Option<PathBuf>,
    /// Where a probe, a check and the missing variables read `${VAR}`: Personal.
    vault: Vault,
}
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        LibraryService {
            store,
```

**with:**

```rust
        LibraryService {
            vault: Vault::in_memory(store.clone()),
            store,
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
            skills_dir: None,
        }
    }

```

**with:**

```rust
            skills_dir: None,
        }
    }

    /// The vault whose Personal variables `${VAR}` reads (env vault spec §8).
    pub fn with_vault(mut self, vault: Vault) -> LibraryService {
        self.vault = vault;
        self
    }

```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
            if let LibraryConfig::McpServer(config) = &item.config {
                entry.missing_variables = config
                    .variables()
                    .into_iter()
                    .filter(|v| process_env(v).is_none())
                    .collect();
```

**with:**

```rust
            if let LibraryConfig::McpServer(config) = &item.config {
                let variables = config.variables();
                let personal = self.vault.personal_values(&variables).await?;
                entry.missing_variables = variables
                    .into_iter()
                    .filter(|v| !personal.contains(v))
                    .collect();
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    pub async fn probe(&self, config: &McpServerConfig) -> Result<McpProbe> {
        let (session, tools) = open_and_list("probe", config, SessionOptions::default()).await?;
        session.close().await;
```

**with:**

```rust
    pub async fn probe(&self, config: &McpServerConfig) -> Result<McpProbe> {
        let env = self.vault.personal_values(&config.variables()).await?;
        let (session, tools) =
            open_and_list("probe", config, &env, SessionOptions::default()).await?;
        session.close().await;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust

    /// Which of `names` Dots' environment sets (`environment_variables`).
    pub fn environment_variables(&self, names: &[String]) -> Vec<EnvVariable> {
        names
            .iter()
```

**with:**

```rust

    /// Which of `names` Personal has (`environment_variables`).
    pub async fn environment_variables(&self, names: &[String]) -> Result<Vec<EnvVariable>> {
        let personal = self.vault.personal_values(names).await?;
        Ok(names
            .iter()
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
                name: name.clone(),
                set: process_env(name).is_some(),
            })
            .collect()
    }
```

**with:**

```rust
                name: name.clone(),
                set: personal.contains(name),
            })
            .collect())
    }
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        };
        match open_and_list(&item.name, config, SessionOptions::default()).await {
            Ok((session, tools)) => {
```

**with:**

```rust
        };
        let env = self.vault.personal_values(&config.variables()).await?;
        match open_and_list(&item.name, config, &env, SessionOptions::default()).await {
            Ok((session, tools)) => {
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
use crate::schedule::{next_slot, Schedule, Until};
use crate::{Error, Result};
```

**with:**

```rust
use crate::schedule::{next_slot, Schedule, Until};
use crate::vault::Grants;
use crate::{Error, Result};
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub uses: Uses,
    /// The first prompt starts with the global instructions of Settings (context library spec
```

**with:**

```rust
    pub uses: Uses,
    /// The variables each of its consumers may resolve (env vault spec §7).
    #[serde(default)]
    pub variables: Grants,
    /// The first prompt starts with the global instructions of Settings (context library spec
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
            uses: Uses::default(),
            use_global_instructions: true,
```

**with:**

```rust
            uses: Uses::default(),
            variables: Grants::default(),
            use_global_instructions: true,
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
        self.uses.validate()?;
        Ok(())
```

**with:**

```rust
        self.uses.validate()?;
        self.variables.validate()?;
        Ok(())
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
use tokio_util::sync::CancellationToken;

```

**with:**

```rust
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
use crate::library::{CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, Uses};
use crate::library_service::{process_env, record_check, refresh_plugin, skill_problem};
use crate::mcp::{McpError, McpSession, McpTool, SessionOptions};
```

**with:**

```rust
use crate::library::{CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, Uses};
use crate::library_service::{record_check, refresh_plugin, skill_problem};
use crate::mcp::{McpError, McpSession, McpTool, SessionOptions};
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
use crate::store::Store;
use crate::Result;
```

**with:**

```rust
use crate::store::Store;
use crate::vault::{Consumer, Grants, Resolved, Vault};
use crate::Result;
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    pub cancel: &'a CancellationToken,
}
```

**with:**

```rust
    pub cancel: &'a CancellationToken,
    /// Where the dot's variables resolve, its grants, and its name for the run events
    /// (env vault spec §8).
    pub vault: &'a Vault,
    pub grants: &'a Grants,
    pub dot_name: &'a str,
}
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    watchers: Mutex<Vec<JoinHandle<()>>>,
}
```

**with:**

```rust
    watchers: Mutex<Vec<JoinHandle<()>>>,
    /// The values the dot grants its shell (env vault spec §7), for the engine to set.
    shell_env: Vec<(String, Zeroizing<String>)>,
}
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust

    /// A Claude-engine run's folders for `--plugin-dir`; empty for a local-engine run.
```

**with:**

```rust

    /// The variables granted to the dot's shell, with their values: the `claude` CLI's
    /// environment, or the local engine's shell tool's.
    pub fn shell_env(&self) -> &[(String, Zeroizing<String>)] {
        &self.shell_env
    }

    /// A Claude-engine run's folders for `--plugin-dir`; empty for a local-engine run.
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            cancel,
        } = req;
```

**with:**

```rust
            cancel,
            vault,
            grants,
            dot_name,
        } = req;
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust

        let items: Vec<LibraryItem> = store
            .used_mcp_servers(uses)
```

**with:**

```rust

        let used: Vec<LibraryItem> = store
            .used_mcp_servers(uses)
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            .collect();
        let Some(opened) = open_sessions(&items, workspace, cancel).await else {
```

**with:**

```rust
            .collect();
        // A server whose variables do not all resolve for it is left out, with the reason; its
        // catalog is the Library's and does not record a dot's grants.
        let mut items = Vec::new();
        for item in used {
            let LibraryConfig::McpServer(config) = &item.config else {
                continue;
            };
            let consumer = Consumer::Mcp(item.name.clone());
            match vault
                .resolve(grants, &consumer, &config.variables())
                .await?
            {
                Ok(env) => items.push((item, env)),
                Err(missing) => notices.push(missing.text(dot_name)),
            }
        }
        let shell_env = shell_env(vault, grants, dot_name, &mut notices).await?;
        let Some(opened) = open_sessions(&items, workspace, cancel).await else {
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        let mut tools = RunTools::default();
        for (item, outcome) in items.into_iter().zip(opened) {
            match outcome {
```

**with:**

```rust
        let mut tools = RunTools::default();
        tools.shell_env = shell_env;
        for ((item, _), outcome) in items.into_iter().zip(opened) {
            match outcome {
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust

/// Opens a session with each server at once and lists its tools; one outcome per server, in
/// order. `None` when `cancel` fires first: what was opened is closed.
async fn open_sessions(
    items: &[LibraryItem],
    workspace: &Path,
```

**with:**

```rust

/// The values granted to the dot's shell; one that does not resolve is left out, with a run
/// event saying why.
async fn shell_env(
    vault: &Vault,
    grants: &Grants,
    dot_name: &str,
    notices: &mut Vec<String>,
) -> Result<Vec<(String, Zeroizing<String>)>> {
    let mut out = Vec::new();
    for name in grants.names(&Consumer::Shell) {
        let names = std::slice::from_ref(name);
        match vault.resolve(grants, &Consumer::Shell, names).await? {
            Ok(env) => {
                if let Some(value) = env.get(name) {
                    out.push((name.clone(), Zeroizing::new(value)));
                }
            }
            Err(missing) => notices.push(missing.text(dot_name)),
        }
    }
    Ok(out)
}

/// Opens a session with each server at once, its `${VAR}` read from its resolved values, and
/// lists its tools; one outcome per server, in order. `None` when `cancel` fires first: what
/// was opened is closed.
async fn open_sessions(
    items: &[(LibraryItem, Resolved)],
    workspace: &Path,
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    let mut set = JoinSet::new();
    for (i, item) in items.iter().enumerate() {
        let LibraryConfig::McpServer(config) = item.config.clone() else {
```

**with:**

```rust
    let mut set = JoinSet::new();
    for (i, (item, env)) in items.iter().enumerate() {
        let LibraryConfig::McpServer(config) = item.config.clone() else {
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        };
        let name = item.name.clone();
```

**with:**

```rust
        };
        let env = env.clone();
        let name = item.name.clone();
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            let open = async {
                let session = McpSession::open(&name, &config, &process_env, options).await?;
                let changes = session.list_changed();
```

**with:**

```rust
            let open = async {
                let session = McpSession::open(&name, &config, &|k| env.get(k), options).await?;
                let changes = session.list_changed();
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
use crate::util::{ct_eq, new_token};
use crate::workspace::{Prepared, WorkspaceManager};
```

**with:**

```rust
use crate::util::{ct_eq, new_token};
use crate::vault::Vault;
use crate::workspace::{Prepared, WorkspaceManager};
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    checkins: OnceLock<CheckIns>,
}
```

**with:**

```rust
    checkins: OnceLock<CheckIns>,
    /// Where runs resolve their dots' variables, set once by the runtime (`set_vault`). A
    /// runner without one resolves from an empty vault in memory.
    vault: OnceLock<Vault>,
}
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            checkins: OnceLock::new(),
        })
    }
```

**with:**

```rust
            checkins: OnceLock::new(),
            vault: OnceLock::new(),
        })
    }

    /// The vault runs resolve their dots' variables from (env vault spec §8). Set once, before
    /// any run starts.
    pub fn set_vault(&self, vault: Vault) {
        let _ = self.vault.set(vault);
    }
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        };
        // The run's MCP sessions, skills and plugin folders (MCP/skills spec section 5).
        let (tools, notices) = RunTools::open(OpenRequest {
```

**with:**

```rust
        };
        // The run's MCP sessions, skills and plugin folders (MCP/skills spec section 5), each
        // server with the variables its dot grants it (env vault spec §8).
        let vault = self
            .vault
            .get()
            .cloned()
            .unwrap_or_else(|| Vault::in_memory(self.store.clone()));
        let (tools, notices) = RunTools::open(OpenRequest {
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            cancel: &cancel,
        })
```

**with:**

```rust
            cancel: &cancel,
            vault: &vault,
            grants: &dot.spec.variables,
            dot_name: &dot.spec.name,
        })
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        runner.set_checkins(checkins.clone());
        // Decisions parked before a restart (decided, but the resume never happened) are
```

**with:**

```rust
        runner.set_checkins(checkins.clone());
        runner.set_vault(vault.clone());
        // Decisions parked before a restart (decided, but the resume never happened) are
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            library: LibraryService::new(store.clone(), bus.clone())
                .with_skills_dir(cfg.data_dir.join(SKILLS_DIR)),
            vault,
```

**with:**

```rust
            library: LibraryService::new(store.clone(), bus.clone())
                .with_skills_dir(cfg.data_dir.join(SKILLS_DIR))
                .with_vault(vault.clone()),
            vault,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use super::Store;
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
```

**with:**

```rust
use super::Store;
use crate::library::{LibraryConfig, Uses};
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use crate::util::{new_id, new_token, now};
use crate::{Error, Result};
```

**with:**

```rust
use crate::util::{new_id, new_token, now};
use crate::vault::Grants;
use crate::{Error, Result};
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, use_global_instructions, \
    use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";
```

**with:**

```rust
pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, variables, \
    use_global_instructions, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            uses: serde_json::from_str(&row.try_get::<String, _>("uses")?)?,
            use_global_instructions: row.try_get::<i64, _>("use_global_instructions")? != 0,
```

**with:**

```rust
            uses: serde_json::from_str(&row.try_get::<String, _>("uses")?)?,
            variables: serde_json::from_str(&row.try_get::<String, _>("variables")?)?,
            use_global_instructions: row.try_get::<i64, _>("use_global_instructions")? != 0,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
impl Store {
    /// Creates a dot. Every item its `uses` names must be in the Library (`Invalid`).
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        let spec = &prepared(spec, None)?;
        let id = new_id();
```

**with:**

```rust
impl Store {
    /// `spec`'s grants as a save stores them (`Grants::synced`): a server newly used, alone or
    /// with its plugin, is granted the variables of its config; `before` is what the dot used.
    pub(crate) async fn synced_grants(
        &self,
        before: Option<&Uses>,
        spec: &DotSpec,
    ) -> Result<Grants> {
        let had: Vec<String> = match before {
            Some(uses) => self
                .used_mcp_servers(uses)
                .await?
                .into_iter()
                .map(|i| i.name)
                .collect(),
            None => Vec::new(),
        };
        let using: Vec<(String, Vec<String>)> = self
            .used_mcp_servers(&spec.uses)
            .await?
            .into_iter()
            .filter_map(|i| match i.config {
                LibraryConfig::McpServer(c) => Some((i.name, c.variables())),
                _ => None,
            })
            .collect();
        Ok(spec.variables.synced(&had, &using))
    }

    /// Creates a dot. Every item its `uses` names must be in the Library (`Invalid`). The
    /// servers it uses are granted their variables.
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        let mut spec = prepared(spec, None)?;
        spec.variables = self.synced_grants(None, &spec).await?;
        let spec = &spec;
        let id = new_id();
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, use_global_instructions, \
             use_user_settings, max_turns, timeout_secs, approval_wait_secs, enabled, \
             created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
```

**with:**

```rust
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, variables, \
             use_global_instructions, use_user_settings, max_turns, timeout_secs, \
             approval_wait_secs, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
```

**with:**

```rust
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(serde_json::to_string(&spec.variables)?)
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
    /// schedule is prepared against the stored one (`prepared`). Every item its `uses` names
    /// must be in the Library (`Invalid`).
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        let previous = self.get_dot(id).await?.spec.schedule;
        let spec = &prepared(spec, previous.as_ref())?;
        let mut tx = self.pool.begin().await?;
```

**with:**

```rust
    /// schedule is prepared against the stored one (`prepared`). Every item its `uses` names
    /// must be in the Library (`Invalid`). A server it uses now and did not before is granted
    /// its variables; one it no longer uses loses its grants.
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        let stored = self.get_dot(id).await?.spec;
        let mut spec = prepared(spec, stored.schedule.as_ref())?;
        spec.variables = self.synced_grants(Some(&stored.uses), &spec).await?;
        let spec = &spec;
        let mut tx = self.pool.begin().await?;
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, \
             use_global_instructions = ?, use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
```

**with:**

```rust
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, variables = ?, \
             use_global_instructions = ?, use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_global_instructions as i64)
```

**with:**

```rust
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(serde_json::to_string(&spec.variables)?)
        .bind(spec.use_global_instructions as i64)
```

**In `crates/dots-core/src/store/library.rs`, replace:**

```rust
        if config.kind() == LibraryKind::McpServer {
            if moved_origin(&old.config, config, &|k| std::env::var(k).ok()) {
                sqlx::query(
```

**with:**

```rust
        if config.kind() == LibraryKind::McpServer {
            // Without values: a URL whose text changed counts as moved (`origin_changed`).
            if moved_origin(&old.config, config, &|_| None) {
                sqlx::query(
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006, 0007 and 0008 that need code (`upgrade.rs`).
    pub async fn open(path: &Path) -> Result<Store> {
```

**with:**

```rust
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006 to 0009 that need code (`upgrade.rs`).
    pub async fn open(path: &Path) -> Result<Store> {
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
        upgrade::finish_0008(&pool).await?;
        Ok(Store { pool })
    }
```

**with:**

```rust
        upgrade::finish_0008(&pool).await?;
        let store = Store { pool };
        upgrade::finish_0009(&store).await?;
        Ok(store)
    }
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
        }
    }
    // Through `Policy`, so the JSON is written as Dots writes a policy.
    let policy: Policy = serde_json::from_value(value).ok()?;
    serde_json::to_string(&policy).ok()
}
```

**with:**

```rust
        }
    }
    // Through `Policy`, so the JSON is written as Dots writes a policy.
    let policy: Policy = serde_json::from_value(value).ok()?;
    serde_json::to_string(&policy).ok()
}

/// Grants each dot listed in `upgrade_0009` the variables of the servers it uses, as ticking
/// them would (env vault spec §11 step 2), then drops the table. After 0006, so the servers it
/// made are there.
pub(super) async fn finish_0009(store: &super::Store) -> Result<()> {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0009'",
    )
    .fetch_one(&store.pool)
    .await?;
    if waiting == 0 {
        return Ok(());
    }
    let ids: Vec<String> = sqlx::query_scalar("SELECT dot_id FROM upgrade_0009")
        .fetch_all(&store.pool)
        .await?;
    let mut tx = store.pool.begin().await?;
    for id in ids {
        let spec = store.get_dot(&id).await?.spec;
        let grants = store.synced_grants(None, &spec).await?;
        sqlx::query("UPDATE dots SET variables = ? WHERE id = ?")
            .bind(serde_json::to_string(&grants)?)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE upgrade_0009")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
```

**Create `crates/dots-core/src/vault/grants.rs`:**

```rust
//! What a dot is granted (env vault spec §7): for each consumer, the variable names it may
//! resolve. A consumer is one Library MCP server (`mcp:<server>`) or the dot's shell (`shell`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::entries::check_variable_name;
use crate::library::{check_name, LibraryKind};
use crate::{Error, Result};

/// The key of the shell consumer.
pub const SHELL: &str = "shell";
const MCP_PREFIX: &str = "mcp:";

/// Who a variable is resolved for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consumer {
    /// The Library MCP server of this name, started for the dot's run.
    Mcp(String),
    /// The dot's agent process: the `claude` CLI and its Bash tool, or the local engine's
    /// shell tool.
    Shell,
}

impl Consumer {
    /// `mcp:<server>` or `shell`, as `Grants` and messages name it.
    pub fn key(&self) -> String {
        match self {
            Self::Mcp(server) => format!("{MCP_PREFIX}{server}"),
            Self::Shell => SHELL.to_string(),
        }
    }

    pub fn parse(key: &str) -> Result<Consumer> {
        if key == SHELL {
            return Ok(Self::Shell);
        }
        match key.strip_prefix(MCP_PREFIX) {
            Some(server) => {
                check_name(LibraryKind::McpServer, server)?;
                Ok(Self::Mcp(server.to_string()))
            }
            None => Err(Error::Invalid(format!(
                "a grant is for 'shell' or 'mcp:<server>', not {key:?}"
            ))),
        }
    }
}

/// A dot's grants: consumer key to variable names, in the order they were granted. A fixed
/// list: a server that later asks for a new `${VAR}` does not get it until it is granted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grants(pub BTreeMap<String, Vec<String>>);

fn contains(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

impl Grants {
    /// The names granted to `consumer`.
    pub fn names(&self, consumer: &Consumer) -> &[String] {
        self.0
            .get(&consumer.key())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Whether `name` is granted to `consumer`, in any case.
    pub fn allows(&self, consumer: &Consumer, name: &str) -> bool {
        contains(self.names(consumer), name)
    }

    /// Each key is a consumer, each name a variable name granted once.
    pub fn validate(&self) -> Result<()> {
        for (key, names) in &self.0 {
            Consumer::parse(key)?;
            let mut seen: Vec<String> = Vec::new();
            for name in names {
                check_variable_name(name)?;
                if contains(&seen, name) {
                    return Err(Error::Invalid(format!("{key} is granted {name} twice")));
                }
                seen.push(name.clone());
            }
        }
        Ok(())
    }

    /// The grants as a save stores them (spec §7): the shell's as given; a server the dot uses
    /// keeps the names given for it, and a server it did not use before (`had` lists those it
    /// did) is also granted every variable its config reads now (`using`: each server with
    /// its variables); a server it no longer uses loses its grants.
    pub fn synced(&self, had: &[String], using: &[(String, Vec<String>)]) -> Grants {
        let mut out = BTreeMap::new();
        if let Some(shell) = self.0.get(SHELL) {
            if !shell.is_empty() {
                out.insert(SHELL.to_string(), shell.clone());
            }
        }
        for (server, variables) in using {
            let key = Consumer::Mcp(server.clone()).key();
            let mut names = self.0.get(&key).cloned().unwrap_or_default();
            if !had.contains(server) {
                for variable in variables {
                    if !contains(&names, variable) {
                        names.push(variable.clone());
                    }
                }
            }
            if !names.is_empty() {
                out.insert(key, names);
            }
        }
        Grants(out)
    }
}
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust
pub mod entries;
pub mod secrets;
```

**with:**

```rust
pub mod entries;
pub mod grants;
pub mod resolve;
pub mod secrets;
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust
pub use entries::{EntryKind, EnvEntry};
#[cfg(windows)]
```

**with:**

```rust
pub use entries::{EntryKind, EnvEntry};
pub use grants::{Consumer, Grants};
pub use resolve::{Missing, Resolved, Unresolved};
#[cfg(windows)]
```

**Create `crates/dots-core/src/vault/resolve.rs`:**

```rust
//! Resolving a dot's variables (env vault spec §8): a name resolves only when it is granted to
//! the consumer, and only from Personal. What does not resolve says which variable and why.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

use super::grants::{Consumer, Grants};
use super::service::Vault;
use crate::Result;

/// Why a variable did not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    NotGranted,
    NotDefined,
}

/// A variable that did not resolve for a consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub name: String,
    pub consumer: Consumer,
    pub why: Unresolved,
}

impl Missing {
    /// The words of a run event, for the dot `dot`.
    pub fn text(&self, dot: &str) -> String {
        let consumer = self.consumer.key();
        match self.why {
            Unresolved::NotGranted => {
                format!("{}: not granted to {consumer} in dot '{dot}'", self.name)
            }
            Unresolved::NotDefined => {
                format!("{}: not in Personal (dot '{dot}', {consumer})", self.name)
            }
        }
    }
}

/// Resolved values by the name they were asked for.
#[derive(Debug, Clone, Default)]
pub struct Resolved(BTreeMap<String, Zeroizing<String>>);

impl Resolved {
    /// The value of `name`, for `McpServerConfig::expanded`.
    pub fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).map(|v| v.to_string())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Each name with its value.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl Vault {
    /// Resolves `names` for `consumer` under `grants`: all of them, or the first that does not
    /// resolve.
    pub async fn resolve(
        &self,
        grants: &Grants,
        consumer: &Consumer,
        names: &[String],
    ) -> Result<std::result::Result<Resolved, Missing>> {
        let mut out = BTreeMap::new();
        for name in names {
            let missing = |why| Missing {
                name: name.clone(),
                consumer: consumer.clone(),
                why,
            };
            if !grants.allows(consumer, name) {
                return Ok(Err(missing(Unresolved::NotGranted)));
            }
            match self.personal_value(name).await? {
                Some(value) => {
                    out.insert(name.clone(), value);
                }
                None => return Ok(Err(missing(Unresolved::NotDefined))),
            }
        }
        Ok(Ok(Resolved(out)))
    }

    /// The personal values of those of `names` that Personal has, as if all were granted: for
    /// what the Library does outside a run (a probe, a check, a confirmation's origin).
    pub async fn personal_values(&self, names: &[String]) -> Result<Resolved> {
        let mut out = BTreeMap::new();
        for name in names {
            if let Some(value) = self.personal_value(name).await? {
                out.insert(name.clone(), value);
            }
        }
        Ok(Resolved(out))
    }
}
```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust
use std::sync::Arc;

```

**with:**

```rust
use std::sync::Arc;

use zeroize::Zeroizing;

```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust
        Vault { store, secrets }
    }
```

**with:**

```rust
        Vault { store, secrets }
    }

    /// A vault whose secrets live in memory: for a Library or a runner made without the
    /// runtime's (tests).
    pub fn in_memory(store: Store) -> Vault {
        Vault::new(store, Arc::new(super::MemorySecrets::new()))
    }
```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust

    async fn get(&self, name: &str) -> Result<EnvEntry> {
```

**with:**

```rust

    /// The value of the personal variable `name`, in any case, if there is one.
    pub(crate) async fn personal_value(&self, name: &str) -> Result<Option<Zeroizing<String>>> {
        let Some(entry) = self.store.find_env_entry(name).await? else {
            return Ok(None);
        };
        match entry.kind {
            EntryKind::Plain => Ok(entry.value.map(Zeroizing::new)),
            EntryKind::Secret => self.secrets.get(&personal_key(&entry.name)),
        }
    }

    async fn get(&self, name: &str) -> Result<EnvEntry> {
```

**In `ui/src/api/types.ts`, replace:**

```ts
  uses: Uses,
  use_global_instructions: z.boolean(),
```

**with:**

```ts
  uses: Uses,
  // Consumer (`mcp:<server>` or `shell`) to the variable names it may resolve.
  variables: z.record(z.string(), z.array(z.string())),
  use_global_instructions: z.boolean(),
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
    uses: dot.uses,
    use_global_instructions: dot.use_global_instructions,
```

**with:**

```ts
    uses: dot.uses,
    variables: dot.variables,
    use_global_instructions: dot.use_global_instructions,
```

- [ ] **Step 4: Run the tests and write the contract**

Run: `cargo test -p dots-core --test vault_grants_test --test run_tools_test --test library_service_test --test claude_gateway_test`, then `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app` and `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
Expected: all pass; `vault_grants_test` has 8. App 94; UI 450.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`, `bun run --cwd ui typecheck`
Expected: no warnings; core 757 passed.

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/confirm.rs crates/dots-core/migrations/0009_vault.sql crates/dots-core/src/library_service.rs crates/dots-core/src/model.rs crates/dots-core/src/run_tools.rs crates/dots-core/src/runner.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/dots.rs crates/dots-core/src/store/library.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/upgrade.rs crates/dots-core/src/vault crates/dots-core/tests/claude_gateway_test.rs crates/dots-core/tests/library_service_test.rs crates/dots-core/tests/run_tools_test.rs crates/dots-core/tests/vault_grants_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/lib/dot-rules.ts
git commit -m 'feat(core): dots are granted variables, and ${VAR} resolves from Personal'
```

### Task 4: Child processes start from a base environment

Every child process starts from a base environment (spec §8): `env_clear`, then the base, then what it is given. That covers the `claude` CLI (the version probe, runs and skill drafts), every stdio MCP server, and the local engine's shell. The CLI and the shell get the dot's shell grants. `fake-claude` records the names of its environment, and the values of names that start with `DOTS_TEST_`.

**Files:**

- Create: `crates/dots-core/src/vault/child_env.rs`
- Modify: `crates/dots-core/src/vault/mod.rs`, `crates/dots-core/src/engine/claude/{mod,once}.rs`, `crates/dots-core/src/engine/local/{mod,tools/mod,tools/shell}.rs`, `crates/dots-core/src/mcp/stdio.rs`, `crates/dots-core/src/bin/fake-claude.rs`
- Tests: `crates/dots-core/tests/vault_child_env_test.rs` (new); `claude_engine_test.rs`, `local_tools_test.rs`

**Interfaces:**

- Consumes: Task 3's `RunTools::shell_env`.
- Produces:
  - `vault::child_env::{BASE_ENV, CLI_ENV, CLI_PREFIXES, Child { Cli, Other }, base_env(Child), start_clean(&mut Command, Child)}`.
  - `engine::local::tools::run_with` gains a fifth argument, `shell_env: &[(String, Zeroizing<String>)]`; `run` passes `&[]`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_engine_test.rs`, replace:**

```rust
use dots_core::store::Store;
use serde_json::{json, Value};
```

**with:**

```rust
use dots_core::store::Store;
use dots_core::vault::{EntryKind, Grants, Vault};
use serde_json::{json, Value};
```

**In `crates/dots-core/tests/claude_engine_test.rs`, replace:**

```rust
    assert!(failed.error.unwrap().contains("failed to start"));
}
```

**with:**

```rust
    assert!(failed.error.unwrap().contains("failed to start"));
}

/// The CLI starts with the base of Dots' environment and the dot's shell grants, nothing else
/// (env vault spec §8).
#[tokio::test]
async fn the_cli_gets_the_shell_grants_and_not_the_rest_of_dots_environment() {
    std::env::set_var("DOTS_TEST_CLI_LEAK", "from-dots");
    let e = e2e(json!([init(), success("fake done")]), None).await;
    let vault = Vault::in_memory(e.store.clone());
    vault
        .add("DOTS_TEST_CLI_GRANTED", EntryKind::Secret, "granted-value")
        .await
        .unwrap();
    vault
        .add("DOTS_TEST_CLI_UNGRANTED", EntryKind::Plain, "not-for-it")
        .await
        .unwrap();
    e.runner.set_vault(vault);
    let mut s = spec("cli-env");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.variables = Grants(
        [(
            "shell".to_string(),
            vec!["DOTS_TEST_CLI_GRANTED".to_string()],
        )]
        .into(),
    );
    let dot = e.store.create_dot(&s).await.unwrap();
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 15).await;
    let args = recorded_args(&e);
    assert_eq!(
        args["env"],
        json!({"DOTS_TEST_CLI_GRANTED": "granted-value"})
    );
    let names: Vec<String> = serde_json::from_value(args["env_names"].clone()).unwrap();
    let upper: Vec<String> = names.iter().map(|n| n.to_ascii_uppercase()).collect();
    assert!(upper.contains(&"PATH".to_string()));
    assert!(upper.contains(&"MCP_TOOL_TIMEOUT".to_string()));
    assert!(upper.contains(&"FAKE_CLAUDE_SCRIPT".to_string()));
    assert!(!upper.contains(&"DOTS_TEST_CLI_LEAK".to_string()));
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    let filter = tools::PathFilter::from_policy(policy, call.alias, ws);
    tools::run_with(&call, ws, &CancellationToken::new(), &filter).await
}
```

**with:**

```rust
    let filter = tools::PathFilter::from_policy(policy, call.alias, ws);
    tools::run_with(&call, ws, &CancellationToken::new(), &filter, &[]).await
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    assert_eq!(out.output.trim(), "done: héllo ✓");
}
```

**with:**

```rust
    assert_eq!(out.output.trim(), "done: héllo ✓");
}

/// The shell gets the base of Dots' environment and what the dot grants it (env vault spec §8).
#[tokio::test]
async fn the_shell_gets_its_grants_and_not_the_rest_of_dots_environment() {
    std::env::set_var("DOTS_TEST_SHELL_LEAK", "from-dots");
    let dir = tempfile::tempdir().unwrap();
    let script = if cfg!(windows) {
        "Write-Output \"granted=$env:DOTS_TEST_SHELL_GRANTED leak=$env:DOTS_TEST_SHELL_LEAK\""
    } else {
        "echo \"granted=$DOTS_TEST_SHELL_GRANTED leak=$DOTS_TEST_SHELL_LEAK\""
    };
    let args = json!({ "command": script });
    let call = tools::prepare("shell", &args, dir.path()).unwrap();
    let env = [(
        "DOTS_TEST_SHELL_GRANTED".to_string(),
        zeroize::Zeroizing::new("yes".to_string()),
    )];
    let out = tools::run_with(
        &call,
        dir.path(),
        &CancellationToken::new(),
        &Default::default(),
        &env,
    )
    .await;
    assert!(!out.is_error, "{out:?}");
    assert!(out.output.contains("granted=yes leak="), "{out:?}");
    assert!(!out.output.contains("from-dots"), "{out:?}");
}
```

**Create `crates/dots-core/tests/vault_child_env_test.rs`:**

```rust
//! What a child process starts with (env vault spec §8): the base of Dots' environment, never
//! the rest, then what it is given.

mod common;

use std::collections::BTreeMap;

use common::fake_mcp::{tool, FakeMcp};
use dots_core::mcp::{McpSession, SessionOptions};
use dots_core::vault::child_env::{base_env, Child, BASE_ENV};
use serde_json::json;

fn names(child: Child) -> Vec<String> {
    base_env(child)
        .into_iter()
        .map(|(k, _)| k.to_string_lossy().to_ascii_uppercase())
        .collect()
}

#[test]
fn the_base_keeps_what_a_program_needs_and_drops_the_rest() {
    std::env::set_var("DOTS_TEST_BASE_LEAK", "from-dots");
    std::env::set_var("ANTHROPIC_DOTS_TEST_BASE", "for-the-cli");
    std::env::set_var("https_proxy", "http://proxy.example:8080");
    let other = names(Child::Other);
    assert!(other.contains(&"PATH".to_string()));
    assert!(other.contains(&"HTTPS_PROXY".to_string()));
    assert!(!other.contains(&"DOTS_TEST_BASE_LEAK".to_string()));
    assert!(!other.contains(&"ANTHROPIC_DOTS_TEST_BASE".to_string()));
    let cli = names(Child::Cli);
    assert!(cli.contains(&"ANTHROPIC_DOTS_TEST_BASE".to_string()));
    assert!(!cli.contains(&"DOTS_TEST_BASE_LEAK".to_string()));
    // The list itself: what the spec names, and nothing that holds a secret by its name.
    for name in [
        "SystemRoot",
        "PATH",
        "TEMP",
        "USERPROFILE",
        "APPDATA",
        "NO_PROXY",
    ] {
        assert!(BASE_ENV.contains(&name), "{name}");
    }
    assert!(!BASE_ENV.iter().any(|n| dots_core::library::looks_secret(n)));
}

#[tokio::test]
async fn an_mcp_server_gets_its_own_env_but_not_dots_environment() {
    std::env::set_var("FAKE_MCP_DOTS_TEST_LEAK", "from-dots");
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let config = fake.stdio_with(
        Vec::new(),
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "given".to_string())]),
    );
    let session = McpSession::open("srv", &config, &|_| None, SessionOptions::default())
        .await
        .unwrap();
    session.close().await;
    let start = fake
        .log()
        .into_iter()
        .find_map(|r| r.get("start").cloned())
        .unwrap();
    assert_eq!(start["env"], json!({"FAKE_MCP_TOKEN": "given"}));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test vault_child_env_test --test claude_engine_test --test local_tools_test`
Expected: the build fails: `vault::child_env` is not defined, and `run_with` takes four arguments.

- [ ] **Step 3: Write the code**

**In `crates/dots-core/src/bin/fake-claude.rs`, replace:**

```rust
            "mcp_tool_timeout": std::env::var("MCP_TOOL_TIMEOUT").ok(),
            "mcp_config": mcp,
```

**with:**

```rust
            "mcp_tool_timeout": std::env::var("MCP_TOOL_TIMEOUT").ok(),
            // Every variable the CLI was started with, by name, and the values of those whose
            // name starts with `DOTS_TEST_`.
            "env_names": std::env::vars().map(|(k, _)| k).collect::<Vec<_>>(),
            "env": std::env::vars()
                .filter(|(k, _)| k.starts_with("DOTS_TEST_"))
                .collect::<std::collections::BTreeMap<_, _>>(),
            "mcp_config": mcp,
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
use crate::engine::{Engine, EngineEvent, RunContext};
use crate::{Error, Result};
```

**with:**

```rust
use crate::engine::{Engine, EngineEvent, RunContext};
use crate::vault::child_env::{start_clean, Child};
use crate::{Error, Result};
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
    let mut cmd = Command::new(program);
    cmd.arg("--version")
```

**with:**

```rust
    let mut cmd = Command::new(program);
    start_clean(&mut cmd, Child::Cli);
    cmd.arg("--version")
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
        let mut cmd = Command::new(&self.program);
        cmd.args(args::build_args(&ctx, &mcp_path))
```

**with:**

```rust
        let mut cmd = Command::new(&self.program);
        // Dots' environment stays out; the dot's shell grants go in (env vault spec §8).
        start_clean(&mut cmd, Child::Cli);
        cmd.envs(
            ctx.tools
                .shell_env()
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        cmd.args(args::build_args(&ctx, &mcp_path))
```

**In `crates/dots-core/src/engine/claude/once.rs`, replace:**

```rust
    let mut cmd = Command::new(req.program);
    cmd.args(once_args(req.model, req.system_prompt))
```

**with:**

```rust
    let mut cmd = Command::new(req.program);
    crate::vault::child_env::start_clean(&mut cmd, crate::vault::child_env::Child::Cli);
    cmd.args(once_args(req.model, req.system_prompt))
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
            PathFilter::from_policy(&self.ctx.dot.spec.policy, call.alias, &self.ctx.workspace);
        tools::run_with(call, &self.ctx.workspace, &self.ctx.cancel, &filter).await
    }
```

**with:**

```rust
            PathFilter::from_policy(&self.ctx.dot.spec.policy, call.alias, &self.ctx.workspace);
        tools::run_with(
            call,
            &self.ctx.workspace,
            &self.ctx.cancel,
            &filter,
            self.ctx.tools.shell_env(),
        )
        .await
    }
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
use tokio_util::sync::CancellationToken;

```

**with:**

```rust
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust

/// Runs an allowed call with nothing hidden. Every failure becomes an error result for the model.
pub async fn run(call: &PreparedCall, workspace: &Path, cancel: &CancellationToken) -> ToolOutput {
    run_with(call, workspace, cancel, &PathFilter::default()).await
}
```

**with:**

```rust

/// Runs an allowed call with nothing hidden and nothing granted to the shell. Every failure
/// becomes an error result for the model.
pub async fn run(call: &PreparedCall, workspace: &Path, cancel: &CancellationToken) -> ToolOutput {
    run_with(call, workspace, cancel, &PathFilter::default(), &[]).await
}
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust

/// Runs an allowed call; the walking tools leave out what `filter` hides.
pub async fn run_with(
```

**with:**

```rust

/// Runs an allowed call; the walking tools leave out what `filter` hides, and the shell gets
/// `shell_env`, the values the dot grants it.
pub async fn run_with(
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    filter: &PathFilter,
) -> ToolOutput {
```

**with:**

```rust
    filter: &PathFilter,
    shell_env: &[(String, Zeroizing<String>)],
) -> ToolOutput {
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
        "grep" => search::grep(&root, input, filter).await,
        "shell" => shell::shell(&root, input, cancel).await,
        other => ToolOutput::err(format!("unknown tool '{other}'")),
```

**with:**

```rust
        "grep" => search::grep(&root, input, filter).await,
        "shell" => shell::shell(&root, input, cancel, shell_env).await,
        other => ToolOutput::err(format!("unknown tool '{other}'")),
```

**In `crates/dots-core/src/engine/local/tools/shell.rs`, replace:**

```rust
use tokio_util::sync::CancellationToken;

```

**with:**

```rust
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

```

**In `crates/dots-core/src/engine/local/tools/shell.rs`, replace:**

```rust

pub(super) async fn shell(root: &Path, input: &Value, cancel: &CancellationToken) -> ToolOutput {
    let script = str_arg(input, "command");
```

**with:**

```rust

/// Runs the call's script in `root`, with the base of Dots' environment and `env`, the values
/// the dot grants its shell (env vault spec §8).
pub(super) async fn shell(
    root: &Path,
    input: &Value,
    cancel: &CancellationToken,
    env: &[(String, Zeroizing<String>)],
) -> ToolOutput {
    let script = str_arg(input, "command");
```

**In `crates/dots-core/src/engine/local/tools/shell.rs`, replace:**

```rust
    let (mut cmd, stdin) = command_for(script);
    cmd.current_dir(root)
```

**with:**

```rust
    let (mut cmd, stdin) = command_for(script);
    crate::vault::child_env::start_clean(&mut cmd, crate::vault::child_env::Child::Other);
    cmd.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    cmd.current_dir(root)
```

**In `crates/dots-core/src/mcp/stdio.rs`, replace:**

```rust
        let mut cmd = Command::new(&program);
        cmd.args(launch.args)
```

**with:**

```rust
        let mut cmd = Command::new(&program);
        // Only the base of Dots' environment, then the server's own (env vault spec §8).
        crate::vault::child_env::start_clean(&mut cmd, crate::vault::child_env::Child::Other);
        cmd.args(launch.args)
```

**Create `crates/dots-core/src/vault/child_env.rs`:**

```rust
//! What a child process starts with (env vault spec §8): not Dots' whole environment, only a
//! base it needs to run, and then what it is given. So a secret set for Dots never reaches the
//! `claude` CLI, an MCP server or the shell, and `printenv` shows nothing that was not granted.

use std::ffi::OsString;

use tokio::process::Command;

/// The variables every child keeps from Dots' environment, compared without regard to case.
pub const BASE_ENV: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATH",
    "PATHEXT",
    "PSModulePath",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "LANG",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
];

/// What the `claude` CLI also keeps: its own configuration and login, passed through unread
/// (the subscription rule).
pub const CLI_ENV: &[&str] = &["CLAUDE_CONFIG_DIR"];
/// Prefixes of the `claude` CLI's own variables.
pub const CLI_PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_CODE_"];

/// Which kind of child the environment is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Child {
    /// The `claude` CLI: the base and its own variables.
    Cli,
    /// Anything else Dots starts for a run: an MCP server, the local engine's shell.
    Other,
}

fn kept(name: &str, child: Child) -> bool {
    let upper = name.to_ascii_uppercase();
    BASE_ENV.iter().any(|k| k.eq_ignore_ascii_case(name))
        || (child == Child::Cli
            && (CLI_ENV.iter().any(|k| k.eq_ignore_ascii_case(name))
                || CLI_PREFIXES.iter().any(|p| upper.starts_with(p))))
}

/// The variables of Dots' environment that a `child` keeps.
pub fn base_env(child: Child) -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(k, _)| k.to_str().is_some_and(|k| kept(k, child)))
        .collect()
}

/// Clears `cmd`'s environment down to the base of `child`. What the caller sets after this
/// (the granted values, Dots' own settings) is added on top.
pub fn start_clean(cmd: &mut Command, child: Child) {
    cmd.env_clear().envs(base_env(child));
}
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust

pub mod entries;
```

**with:**

```rust

pub mod child_env;
pub mod entries;
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p dots-core --test vault_child_env_test --test claude_engine_test --test local_tools_test`
Expected: 2, 10 and 39 passed.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`
Expected: no warnings; core 761 passed.

```bash
git add crates/dots-core/src/bin/fake-claude.rs crates/dots-core/src/engine/claude/mod.rs crates/dots-core/src/engine/claude/once.rs crates/dots-core/src/engine/local/mod.rs crates/dots-core/src/engine/local/tools/mod.rs crates/dots-core/src/engine/local/tools/shell.rs crates/dots-core/src/mcp/stdio.rs crates/dots-core/src/vault/child_env.rs crates/dots-core/src/vault/mod.rs crates/dots-core/tests/claude_engine_test.rs crates/dots-core/tests/local_tools_test.rs crates/dots-core/tests/vault_child_env_test.rs
git commit -m "feat(core): child processes start from a base environment and their grants"
```

### Task 5: Secret values scrubbed from what Dots keeps and logs

The redactor (spec §10): every secret the vault reads, adds or replaces is added to a process-wide redactor. The store scrubs with it as it writes run events (and so the live view), run payloads, summaries and errors, approval inputs, and the text of notes and questions. The app's log writer scrubs every line.

**Files:**

- Create: `crates/dots-core/src/vault/redact.rs`
- Modify: `crates/dots-core/src/vault/{mod,service}.rs`, `crates/dots-core/src/store/{runs,approvals,messages}.rs`, `apps/dots-app/src/startup.rs`
- Test: `crates/dots-core/tests/vault_redact_test.rs` (new)

**Interfaces:**

- Consumes: Task 2's `Vault`; Task 3's `personal_value`.
- Produces: `vault::redact::{Redactor, global(), spellings(value), RedactWriter::new(W), MIN_REDACT_BYTES = 8}`. A `Redactor` has `add(name, value)`, `scrub(&str) -> Cow<str>` and `scrub_json(&Value) -> Value`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_redact_test.rs`:**

```rust
//! Scrubbing secret values from what Dots keeps (env vault spec §10).

mod common;

use std::io::Write;

use common::{spec, temp_store};
use dots_core::model::{NewRun, RunStatus, TriggerKind};
use dots_core::vault::redact::{global, spellings, RedactWriter, Redactor};
use dots_core::vault::{EntryKind, Vault};
use serde_json::json;

#[test]
fn every_common_spelling_of_a_secret_is_scrubbed() {
    let r = Redactor::default();
    r.add("API_KEY", "p@ss word/+=\"x");
    let text = format!(
        "raw {} url {} plus {} json {} b64 {}",
        "p@ss word/+=\"x",
        "p%40ss%20word%2F%2B%3D%22x",
        "p%40ss+word%2F%2B%3D%22x",
        "p@ss word/+=\\\"x",
        "cEBzcyB3b3JkLys9Ing=",
    );
    let out = r.scrub(&text);
    assert!(!out.contains("p@ss"), "{out}");
    assert!(!out.contains("p%40ss"), "{out}");
    assert!(!out.contains("cEBzcyB3b3JkLys9Ing"), "{out}");
    assert_eq!(out.matches("[secret:API_KEY]").count(), 5, "{out}");
    assert!(spellings("abc").contains(&"YWJj".to_string()));
}

#[test]
fn a_short_value_is_left_alone_and_a_longer_secret_wins() {
    let r = Redactor::default();
    r.add("SHORT", "1234567");
    r.add("OUTER", "token-abcdefgh-1234");
    r.add("INNER", "abcdefgh");
    assert_eq!(r.scrub("1234567").as_ref(), "1234567");
    assert_eq!(
        r.scrub("x token-abcdefgh-1234 y").as_ref(),
        "x [secret:OUTER] y"
    );
    assert_eq!(r.scrub("abcdefgh").as_ref(), "[secret:INNER]");
    assert_eq!(
        r.scrub_json(&json!({"a": ["abcdefgh", 1], "abcdefgh": null})),
        json!({"a": ["[secret:INNER]", 1], "[secret:INNER]": null})
    );
}

#[tokio::test]
async fn what_the_store_keeps_of_a_run_holds_no_secret_it_has_read() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    vault
        .add(
            "DOTS_REDACT_TOKEN",
            EntryKind::Secret,
            "ghp_redact_test_value_42",
        )
        .await
        .unwrap();
    let secret = "ghp_redact_test_value_42";
    let dot = store.create_dot(&spec("redact")).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Webhook);
    new.payload = Some(json!({"echo": format!("token={secret}")}));
    let run = store.create_run(&new).await.unwrap();
    assert_eq!(
        run.payload,
        Some(json!({"echo": "token=[secret:DOTS_REDACT_TOKEN]"}))
    );
    let ev = store
        .append_event(
            &run.id,
            "tool_result",
            &json!({"output": format!("Bearer {secret}")}),
        )
        .await
        .unwrap();
    assert_eq!(ev.data["output"], "Bearer [secret:DOTS_REDACT_TOKEN]");
    let stored = store.list_events(&run.id, 0).await.unwrap();
    assert!(!serde_json::to_string(&stored).unwrap().contains(secret));
    let approval = store
        .create_approval(
            &run.id,
            "Bash",
            &json!({"command": format!("curl -H 'x: {secret}'")}),
        )
        .await
        .unwrap();
    assert!(!approval.input.to_string().contains(secret));
    let finished = store
        .finish_run(
            &run.id,
            RunStatus::Failed,
            Some(secret),
            Some(&format!("failed: {secret}")),
        )
        .await
        .unwrap();
    assert!(!format!("{:?} {:?}", finished.summary, finished.error).contains(secret));
    let note = store
        .add_note(&dot.id, Some(&run.id), secret)
        .await
        .unwrap();
    assert_eq!(note.text, "[secret:DOTS_REDACT_TOKEN]");
}

#[test]
fn the_log_writer_scrubs_each_line() {
    global().add("DOTS_REDACT_LOG", "log-secret-value-77");
    let mut out = Vec::new();
    {
        let mut w = RedactWriter::new(&mut out);
        w.write_all(b"INFO MCP server stderr: auth log-secret-value-77 ok\n")
            .unwrap();
    }
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "INFO MCP server stderr: auth [secret:DOTS_REDACT_LOG] ok\n"
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test vault_redact_test`
Expected: the build fails: `vault::redact` is not defined.

- [ ] **Step 3: Write the code**

**In `apps/dots-app/src/startup.rs`, replace:**

```rust
    let appender = appender(log_dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
```

**with:**

```rust
    let appender = appender(log_dir)?;
    // Every line is scrubbed of the secrets Dots has read (env vault spec §10).
    let (writer, guard) =
        tracing_appender::non_blocking(dots_core::vault::redact::RedactWriter::new(appender));
    tracing_subscriber::fmt()
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
use crate::util::{json_hash, new_id, now};
use crate::{Error, Result};
```

**with:**

```rust
use crate::util::{json_hash, new_id, now};
use crate::vault::redact::global;
use crate::{Error, Result};
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
        .bind(tool)
        .bind(input.to_string())
        .bind(json_hash(input))
```

**with:**

```rust
        .bind(tool)
        .bind(global().scrub_json(input).to_string())
        .bind(json_hash(input))
```

**In `crates/dots-core/src/store/messages.rs`, replace:**

```rust
use crate::util::{new_id, now};
use crate::{Error, Result};
```

**with:**

```rust
use crate::util::{new_id, now};
use crate::vault::redact::global;
use crate::{Error, Result};
```

**In `crates/dots-core/src/store/messages.rs`, replace:**

```rust
        .bind(MessageKind::Note.as_str())
        .bind(text)
        .bind(now())
```

**with:**

```rust
        .bind(MessageKind::Note.as_str())
        .bind(global().scrub(text).as_ref())
        .bind(now())
```

**In `crates/dots-core/src/store/messages.rs`, replace:**

```rust
        .bind(run_id)
        .bind(text)
        .bind(now())
```

**with:**

```rust
        .bind(run_id)
        .bind(global().scrub(text).as_ref())
        .bind(now())
```

**In `crates/dots-core/src/store/messages.rs`, replace:**

```rust
        .bind(run_id)
        .bind(text)
        .bind(choices)
        .bind(now())
```

**with:**

```rust
        .bind(run_id)
        .bind(global().scrub(text).as_ref())
        .bind(choices.map(|c| global().scrub(&c).into_owned()))
        .bind(now())
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
use crate::util::{new_id, now};
use crate::{Error, Result};
```

**with:**

```rust
use crate::util::{new_id, now};
use crate::vault::redact::global;
use crate::{Error, Result};
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
    .bind(new.trigger.as_str())
    .bind(new.payload.as_ref().map(|v| v.to_string()))
    .bind(RunStatus::Queued.as_str())
```

**with:**

```rust
    .bind(new.trigger.as_str())
    .bind(
        new.payload
            .as_ref()
            .map(|v| global().scrub_json(v).to_string()),
    )
    .bind(RunStatus::Queued.as_str())
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
        .bind(status.as_str())
        .bind(summary)
        .bind(error)
        .bind(now())
```

**with:**

```rust
        .bind(status.as_str())
        .bind(summary.map(|s| global().scrub(s).into_owned()))
        .bind(error.map(|e| global().scrub(e).into_owned()))
        .bind(now())
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
        let ts = now();
        let seq: i64 = sqlx::query_scalar(
```

**with:**

```rust
        let ts = now();
        // What the run event shows, the live view included: no secret value (env vault §10).
        let data = &global().scrub_json(data);
        let seq: i64 = sqlx::query_scalar(
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust
pub mod grants;
pub mod resolve;
```

**with:**

```rust
pub mod grants;
pub mod redact;
pub mod resolve;
```

**Create `crates/dots-core/src/vault/redact.rs`:**

```rust
//! Scrubbing secret values from what Dots keeps (env vault spec §10): run events, approvals, a
//! run's payload, summary and error, the messages of dots, and the log. Every secret Dots reads
//! is added here; a text that holds one, in any common spelling, gets `[secret:NAME]` in its
//! place. A backstop: resolution keeps values out of places that never need them.

use std::borrow::Cow;
use std::io::Write;
use std::sync::{LazyLock, RwLock};

use base64::Engine as _;
use serde_json::Value;

/// Values shorter than this are not scrubbed: too likely to match ordinary text.
pub const MIN_REDACT_BYTES: usize = 8;

/// The secret values to scrub, each spelling with the name it stands for, longest first.
#[derive(Default)]
pub struct Redactor {
    spellings: RwLock<Vec<(String, String)>>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.spellings.read().map(|s| s.len()).unwrap_or(0);
        f.debug_struct("Redactor")
            .field("spellings", &count)
            .finish()
    }
}

static GLOBAL: LazyLock<Redactor> = LazyLock::new(Redactor::default);

/// The process's redactor: the vault adds every secret it reads, and the store and the log
/// scrub with it.
pub fn global() -> &'static Redactor {
    &GLOBAL
}

/// `value` percent-encoded, every byte but `A-Za-z0-9-._~`; `plus` writes a space as `+`.
fn url_encoded(value: &str, plus: bool, lower: bool) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b' ' if plus => out.push('+'),
            _ if lower => out.push_str(&format!("%{b:02x}")),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The spellings of `value` that text may carry: as it is, URL-encoded (`%20` and `+` for a
/// space, upper and lower case hex), inside a JSON string, and in standard and URL-safe base64.
pub fn spellings(value: &str) -> Vec<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut out = vec![
        value.to_string(),
        url_encoded(value, false, false),
        url_encoded(value, true, false),
        url_encoded(value, false, true),
        json[1..json.len().saturating_sub(1).max(1)].to_string(),
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
    ];
    out.retain(|s| !s.is_empty());
    out.sort();
    out.dedup();
    out
}

impl Redactor {
    /// Scrubs `value` as the secret `name` from now on. A value shorter than
    /// `MIN_REDACT_BYTES` is left alone.
    pub fn add(&self, name: &str, value: &str) {
        if value.len() < MIN_REDACT_BYTES {
            return;
        }
        let mut all = self.spellings.write().unwrap();
        for spelling in spellings(value) {
            if !all.iter().any(|(s, _)| *s == spelling) {
                all.push((spelling, name.to_string()));
            }
        }
        all.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    }

    /// `text` with every spelling of every secret replaced by `[secret:NAME]`, longest first.
    pub fn scrub<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let all = self.spellings.read().unwrap();
        if !all.iter().any(|(s, _)| text.contains(s.as_str())) {
            return Cow::Borrowed(text);
        }
        let mut out = text.to_string();
        for (spelling, name) in all.iter() {
            if out.contains(spelling.as_str()) {
                out = out.replace(spelling.as_str(), &format!("[secret:{name}]"));
            }
        }
        Cow::Owned(out)
    }

    /// `value` with every string in it scrubbed, object keys included.
    pub fn scrub_json(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.scrub(s).into_owned()),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.scrub_json(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (self.scrub(k).into_owned(), self.scrub_json(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// A writer that scrubs each write with the process's redactor: the log's. Each write is one
/// line of the logger, so a value is never split across two.
pub struct RedactWriter<W> {
    inner: W,
}

impl<W> RedactWriter<W> {
    pub fn new(inner: W) -> RedactWriter<W> {
        RedactWriter { inner }
    }
}

impl<W: Write> Write for RedactWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match std::str::from_utf8(buf) {
            Ok(text) => self.inner.write_all(global().scrub(text).as_bytes())?,
            Err(_) => self
                .inner
                .write_all(global().scrub(&String::from_utf8_lossy(buf)).as_bytes())?,
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust
                }
                Ok(entry)
```

**with:**

```rust
                }
                super::redact::global().add(&entry.name, value);
                Ok(entry)
```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust
                self.secrets.put(&personal_key(&entry.name), value)?;
                self.store.set_env_value(&entry.name, None).await
```

**with:**

```rust
                self.secrets.put(&personal_key(&entry.name), value)?;
                super::redact::global().add(&entry.name, value);
                self.store.set_env_value(&entry.name, None).await
```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust
            EntryKind::Plain => Ok(entry.value.map(Zeroizing::new)),
            EntryKind::Secret => self.secrets.get(&personal_key(&entry.name)),
        }
```

**with:**

```rust
            EntryKind::Plain => Ok(entry.value.map(Zeroizing::new)),
            EntryKind::Secret => {
                let value = self.secrets.get(&personal_key(&entry.name))?;
                if let Some(value) = &value {
                    super::redact::global().add(&entry.name, value);
                }
                Ok(value)
            }
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p dots-core --test vault_redact_test`
Expected: 4 passed.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`
Expected: no warnings; core 765, app 94 passed.

```bash
git add apps/dots-app/src/startup.rs crates/dots-core/src/store/approvals.rs crates/dots-core/src/store/messages.rs crates/dots-core/src/store/runs.rs crates/dots-core/src/vault/mod.rs crates/dots-core/src/vault/redact.rs crates/dots-core/src/vault/service.rs crates/dots-core/tests/vault_redact_test.rs
git commit -m "feat: secret values are scrubbed from what Dots keeps and logs"
```

### Task 6: Typed secrets move into Personal; the import and old-backups offers

Spec §11:

- A secret-looking value with no `${…}` moves into Personal when a server is added or changed and each time a plugin's servers are synced, and the config reads `${NAME}` instead.
- At start, values stored by a version before the vault move the same way. The dots that use such a server are granted the names, and the database is vacuumed.
- The Env tab's one-time offers get their core and commands: import what Windows sets, and delete the backups made before the vault.

**Files:**

- Create: `crates/dots-core/src/vault/literals.rs`, `crates/dots-core/src/vault/upgrade.rs`
- Modify: `crates/dots-core/src/vault/{mod,service}.rs`, `crates/dots-core/src/store/{dots,mod,settings,upgrade}.rs`, `crates/dots-core/src/{library_service,run_tools,runtime}.rs`, `apps/dots-app/src/{commands,lib}.rs`, `ui/src/api/commands.ts`
- Regenerate: the contract fixture (the command list)
- Tests: `crates/dots-core/tests/vault_upgrade_test.rs` (new), `ui/src/api/commands.test.ts`

**Interfaces:**

- Consumes: Tasks 2 and 3 (`Vault::add`, `personal_value`, `Store::dots_using`, `set_dot_variables`).
- Produces:
  - `vault::literals::has_literals(server, &McpServerConfig)`.
  - `Vault` gains:
    - `move_literals(server, &McpServerConfig) -> Result<(McpServerConfig, Vec<String>)>`
    - `move_stored_literals() -> Result<usize>`
    - `mark_vault_since()`
    - `import_candidates()`, `import(&[String])` and `dismiss_import()`
    - `old_backups(data_dir)` and `answer_old_backups(data_dir, delete)`
  - `vault::upgrade::{VAULT_SINCE, IMPORT_ANSWERED, BACKUPS_ANSWERED, BACKUPS_DIR}`.
  - The `Store` gains `marker`, `set_marker`, `pub(crate) set_dot_variables` and `vacuum`.
  - `sync_plugin_servers` and `refresh_plugin` take `&Vault`.
  - `Runtime` gains `old_backups()` and `answer_old_backups(delete)`.
  - The commands `env_import_candidates`, `env_import`, `env_import_dismiss`, `env_old_backups` and `env_old_backups_answer`, with their `commands.ts` wrappers.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/vault_upgrade_test.rs`:**

```rust
//! Secret values typed into the Library move into Personal, and a setup from before the vault
//! is brought in (env vault spec §11).

mod common;

use std::path::Path;
use std::time::{Duration, SystemTime};

use common::{spec, temp_store};
use dots_core::events::new_bus;
use dots_core::library::{LibraryConfig, LibraryKind, McpServerConfig, PluginConfig};
use dots_core::library_service::LibraryService;
use dots_core::store::Store;
use dots_core::vault::{EntryKind, Vault};
use dots_core::Error;
use serde_json::json;

fn stdio(env: &[(&str, &str)]) -> McpServerConfig {
    McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn http(headers: &[(&str, &str)]) -> McpServerConfig {
    McpServerConfig::Http {
        url: "https://api.example.com/mcp".into(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

/// Whether the database file, with its WAL, holds `text`.
fn db_holds(dir: &Path, text: &str) -> bool {
    ["dots.db", "dots.db-wal"].iter().any(|f| {
        std::fs::read(dir.join(f))
            .map(|b| b.windows(text.len()).any(|w| w == text.as_bytes()))
            .unwrap_or(false)
    })
}

/// An old backup: a `.db` file in `<dir>/backups` an hour older than now.
fn old_backup(dir: &Path) -> std::path::PathBuf {
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let old = backups.join("dots-0.1.0-2026-10-01-090000.db");
    std::fs::write(&old, b"old").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    old
}

#[tokio::test]
async fn a_secret_looking_value_moves_into_personal_under_its_name() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let (config, moved) = vault
        .move_literals(
            "sf",
            &stdio(&[
                ("SF_TOKEN", "sf-literal-1"),
                ("MODE", "readonly"),
                ("API_KEY", "${MY_KEY}"),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(moved, ["SF_TOKEN"]);
    assert_eq!(
        config,
        stdio(&[
            ("SF_TOKEN", "${SF_TOKEN}"),
            ("MODE", "readonly"),
            ("API_KEY", "${MY_KEY}")
        ])
    );
    let (config, moved) = vault
        .move_literals(
            "git-hub",
            &http(&[
                ("Authorization", "token ghp_literal_2"),
                ("X-Api-Key", "key-literal-3"),
                ("Accept", "application/json"),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(moved, ["GIT_HUB_AUTHORIZATION", "GIT_HUB_X_API_KEY"]);
    assert_eq!(
        config,
        http(&[
            ("Authorization", "token ${GIT_HUB_AUTHORIZATION}"),
            ("X-Api-Key", "${GIT_HUB_X_API_KEY}"),
            ("Accept", "application/json"),
        ])
    );
    let values = vault
        .personal_values(&["SF_TOKEN".into(), "GIT_HUB_AUTHORIZATION".into()])
        .await
        .unwrap();
    assert_eq!(values.get("SF_TOKEN").as_deref(), Some("sf-literal-1"));
    assert_eq!(
        values.get("GIT_HUB_AUTHORIZATION").as_deref(),
        Some("ghp_literal_2")
    );
    let listed = vault.list().await.unwrap();
    assert!(listed
        .iter()
        .all(|e| e.kind == EntryKind::Secret && e.value.is_none()));
}

#[tokio::test]
async fn the_same_value_reuses_its_name_and_another_value_gets_a_new_one() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    vault
        .add("SF_TOKEN", EntryKind::Secret, "first")
        .await
        .unwrap();
    let (_, same) = vault
        .move_literals("a", &stdio(&[("SF_TOKEN", "first")]))
        .await
        .unwrap();
    assert_eq!(same, ["SF_TOKEN"]);
    let (config, other) = vault
        .move_literals("b", &stdio(&[("SF_TOKEN", "second")]))
        .await
        .unwrap();
    assert_eq!(other, ["SF_TOKEN_2"]);
    assert_eq!(config, stdio(&[("SF_TOKEN", "${SF_TOKEN_2}")]));
}

#[tokio::test]
async fn the_library_never_stores_a_typed_secret() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    let lib = LibraryService::new(store.clone(), new_bus()).with_vault(vault.clone());
    let item = lib
        .add(
            "sf",
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "typed-secret-value-4")])),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        item.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    let updated = lib
        .update(
            &item.id,
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "typed-secret-value-5")])),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN_2}")]))
    );
    assert!(!db_holds(dir.path(), "typed-secret-value"));
}

#[tokio::test]
async fn a_plugins_written_secret_moves_into_personal_as_it_is_synced() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    let lib = LibraryService::new(store.clone(), new_bus()).with_vault(vault.clone());
    let root = dir.path().join("forms");
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"forms"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join(".mcp.json"),
        json!({"mcpServers": {"db": {"command": "node", "env": {"DB_TOKEN": "plugin-secret-6"}}}})
            .to_string(),
    )
    .unwrap();
    lib.add(
        "forms",
        LibraryConfig::Plugin(PluginConfig {
            path: root.to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        }),
        None,
    )
    .await
    .unwrap();
    let server = store
        .find_library_item(LibraryKind::McpServer, "plugin-forms-db")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        server.config,
        LibraryConfig::McpServer(stdio(&[("DB_TOKEN", "${DB_TOKEN}")]))
    );
    assert!(!db_holds(dir.path(), "plugin-secret-6"));
    let mut s = spec("filler");
    s.uses.plugins = vec!["forms".into()];
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec.variables.0["mcp:plugin-forms-db"], ["DB_TOKEN"]);
}

#[tokio::test]
async fn stored_secrets_move_at_start_and_their_dots_are_granted_the_names() {
    let (dir, store) = temp_store().await;
    // As a version before the vault stored them.
    let item = store
        .add_library_item(
            "sf",
            &LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "stored-secret-value-7")])),
            None,
        )
        .await
        .unwrap();
    let mut s = spec("salesforce");
    s.uses.mcp_servers = vec!["sf".into()];
    let dot = store.create_dot(&s).await.unwrap();
    assert!(dot.spec.variables.0.is_empty());
    let vault = Vault::in_memory(store.clone());
    assert_eq!(vault.move_stored_literals().await.unwrap(), 1);
    assert_eq!(
        store.get_library_item(&item.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.variables.0["mcp:sf"],
        ["SF_TOKEN"]
    );
    assert!(!db_holds(dir.path(), "stored-secret-value-7"));
    // Nothing is left to move.
    assert_eq!(vault.move_stored_literals().await.unwrap(), 0);
}

#[tokio::test]
async fn the_import_offers_what_windows_sets_once() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    std::env::set_var("DOTS_IMPORT_TEST_TOKEN", "from-windows");
    store
        .add_library_item(
            "imp",
            &LibraryConfig::McpServer(stdio(&[
                ("A", "${DOTS_IMPORT_TEST_TOKEN}"),
                ("B", "${DOTS_IMPORT_TEST_NEVER_SET}"),
            ])),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        vault.import_candidates().await.unwrap(),
        ["DOTS_IMPORT_TEST_TOKEN"]
    );
    assert!(matches!(
        vault.import(&["PATH".to_string()]).await,
        Err(Error::Invalid(_))
    ));
    assert_eq!(
        vault
            .import(&["DOTS_IMPORT_TEST_TOKEN".to_string()])
            .await
            .unwrap(),
        ["DOTS_IMPORT_TEST_TOKEN"]
    );
    let values = vault
        .personal_values(&["DOTS_IMPORT_TEST_TOKEN".into()])
        .await
        .unwrap();
    assert_eq!(
        values.get("DOTS_IMPORT_TEST_TOKEN").as_deref(),
        Some("from-windows")
    );
    assert!(vault.import_candidates().await.unwrap().is_empty());
}

#[tokio::test]
async fn backups_made_before_the_vault_are_offered_once_and_deleted_on_yes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    let old = old_backup(dir.path());
    let vault = Vault::in_memory(store);
    vault.mark_vault_since().await.unwrap();
    let new = dir
        .path()
        .join("backups")
        .join("dots-0.1.0-2026-10-30-090000.db");
    std::fs::write(&new, b"new").unwrap();
    std::fs::write(dir.path().join("backups").join("notes.txt"), b"x").unwrap();
    assert_eq!(
        vault.old_backups(dir.path()).await.unwrap(),
        ["dots-0.1.0-2026-10-01-090000.db"]
    );
    vault.answer_old_backups(dir.path(), true).await.unwrap();
    assert!(!old.exists() && new.exists());
    assert!(vault.old_backups(dir.path()).await.unwrap().is_empty());
}

#[tokio::test]
async fn keeping_the_old_backups_answers_the_offer_too() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    let old = old_backup(dir.path());
    let vault = Vault::in_memory(store);
    vault.mark_vault_since().await.unwrap();
    vault.answer_old_backups(dir.path(), false).await.unwrap();
    assert!(old.exists());
    assert!(vault.old_backups(dir.path()).await.unwrap().is_empty());
}
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts

  it("refuses a Library answer in a shape it does not know", async () => {
```

**with:**

```ts

  it("sends the vault's one-time offers as the core names them", async () => {
    reply = async () => ["GITHUB_TOKEN"];
    expect(await commands.envImportCandidates()).toEqual(["GITHUB_TOKEN"]);
    expect(await commands.envImport(["GITHUB_TOKEN"])).toEqual(["GITHUB_TOKEN"]);
    reply = async () => ["dots-0.1.0-2026-10-20-091700.db"];
    expect(await commands.envOldBackups()).toEqual(["dots-0.1.0-2026-10-20-091700.db"]);
    reply = async () => null;
    await commands.envImportDismiss();
    await commands.envOldBackupsAnswer(true);
    expect(calls.slice(-5)).toEqual([
      ["env_import_candidates", undefined],
      ["env_import", { names: ["GITHUB_TOKEN"] }],
      ["env_old_backups", undefined],
      ["env_import_dismiss", undefined],
      ["env_old_backups_answer", { delete: true }],
    ]);
  });

  it("refuses a Library answer in a shape it does not know", async () => {
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test vault_upgrade_test`
Expected: the build fails: `Vault::move_literals` and `import_candidates` are not defined.

- [ ] **Step 3: Write the code**

**In `apps/dots-app/src/commands.rs`, replace:**

```rust

/// Every message that waits for the person and the newest others, newest first; of one dot
```

**with:**

```rust

/// The names Library servers read that Dots' Windows environment sets and Personal lacks,
/// until the offer is answered. Never a value.
#[tauri::command]
pub async fn env_import_candidates(state: State<'_, AppState>) -> Res<Vec<String>> {
    Ok(state.rt()?.vault().import_candidates().await?)
}

/// Copies the names from Dots' Windows environment into Personal as secrets; answers the offer.
#[tauri::command]
pub async fn env_import(state: State<'_, AppState>, names: Vec<String>) -> Res<Vec<String>> {
    Ok(state.rt()?.vault().import(&names).await?)
}

#[tauri::command]
pub async fn env_import_dismiss(state: State<'_, AppState>) -> Res<()> {
    Ok(state.rt()?.vault().dismiss_import().await?)
}

/// The update backups made before the vault, by file name, until the offer is answered.
#[tauri::command]
pub async fn env_old_backups(state: State<'_, AppState>) -> Res<Vec<String>> {
    Ok(state.rt()?.old_backups().await?)
}

/// Deletes them when `delete`; answers the offer either way.
#[tauri::command]
pub async fn env_old_backups_answer(state: State<'_, AppState>, delete: bool) -> Res<()> {
    Ok(state.rt()?.answer_old_backups(delete).await?)
}

/// Every message that waits for the person and the newest others, newest first; of one dot
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    env_remove,
    messages_list,
```

**with:**

```rust
    env_remove,
    env_import_candidates,
    env_import,
    env_import_dismiss,
    env_old_backups,
    env_old_backups_answer,
    messages_list,
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    bus: &Bus,
    plugin: &LibraryItem,
```

**with:**

```rust
    bus: &Bus,
    vault: &Vault,
    plugin: &LibraryItem,
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    for (name, config) in wanted {
        let config = LibraryConfig::McpServer(config);
        match existing.iter().find(|i| i.name == name) {
```

**with:**

```rust
    for (name, config) in wanted {
        // A secret written into the plugin's `.mcp.json` moves into Personal; the Library keeps
        // `${NAME}` (env vault spec §11). It stays in the plugin's own folder.
        let config = LibraryConfig::McpServer(vault.move_literals(&name, &config).await?.0);
        match existing.iter().find(|i| i.name == name) {
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    bus: &Bus,
    item: &LibraryItem,
```

**with:**

```rust
    bus: &Bus,
    vault: &Vault,
    item: &LibraryItem,
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
    let mut warnings = info.warnings.clone();
    match sync_plugin_servers(store, bus, item, &info).await {
        Ok(more) => warnings.extend(more),
```

**with:**

```rust
    let mut warnings = info.warnings.clone();
    match sync_plugin_servers(store, bus, vault, item, &info).await {
        Ok(more) => warnings.extend(more),
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        let plugin = self.read_folder(&config).await?;
        let item = self.store.add_library_item(name, &config, None).await?;
```

**with:**

```rust
        let plugin = self.read_folder(&config).await?;
        let config = self.without_literals(name, config).await?;
        let item = self.store.add_library_item(name, &config, None).await?;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        if let Some(info) = plugin {
            for warning in sync_plugin_servers(&self.store, &self.bus, &item, &info).await? {
                tracing::warn!("plugin {name}: {warning}");
```

**with:**

```rust
        if let Some(info) = plugin {
            let synced = sync_plugin_servers(&self.store, &self.bus, &self.vault, &item, &info);
            for warning in synced.await? {
                tracing::warn!("plugin {name}: {warning}");
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        let plugin = self.read_folder(&config).await?;
        let item = self.store.update_library_item(id, &config).await?;
```

**with:**

```rust
        let plugin = self.read_folder(&config).await?;
        let config = self.without_literals(&old.name, config).await?;
        let item = self.store.update_library_item(id, &config).await?;
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        if let Some(info) = plugin {
            for warning in sync_plugin_servers(&self.store, &self.bus, &item, &info).await? {
                tracing::warn!("plugin {}: {warning}", item.name);
```

**with:**

```rust
        if let Some(info) = plugin {
            let synced = sync_plugin_servers(&self.store, &self.bus, &self.vault, &item, &info);
            for warning in synced.await? {
                tracing::warn!("plugin {}: {warning}", item.name);
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust

    fn refuse_owned(&self, item: &LibraryItem, verb: &str) -> Result<()> {
```

**with:**

```rust

    /// An MCP server's config with its secret-looking values moved into Personal (env vault
    /// spec §11): the Library never stores them.
    async fn without_literals(&self, name: &str, config: LibraryConfig) -> Result<LibraryConfig> {
        match config {
            LibraryConfig::McpServer(server) => Ok(LibraryConfig::McpServer(
                self.vault.move_literals(name, &server).await?.0,
            )),
            other => Ok(other),
        }
    }

    fn refuse_owned(&self, item: &LibraryItem, verb: &str) -> Result<()> {
```

**In `crates/dots-core/src/library_service.rs`, replace:**

```rust
        store.remove_library_item(&item.id).await.unwrap();
        let out = refresh_plugin(&store, &bus, &item).await.unwrap();
        assert_eq!(out.unwrap_err(), GONE);
```

**with:**

```rust
        store.remove_library_item(&item.id).await.unwrap();
        let vault = Vault::in_memory(store.clone());
        let out = refresh_plugin(&store, &bus, &vault, &item).await.unwrap();
        assert_eq!(out.unwrap_err(), GONE);
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            };
            match refresh_plugin(store, bus, &item).await? {
                Ok(read) => {
```

**with:**

```rust
            };
            match refresh_plugin(store, bus, vault, &item).await? {
                Ok(read) => {
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        let vault = Vault::new(store.clone(), secrets);
        let bus = new_bus();
```

**with:**

```rust
        let vault = Vault::new(store.clone(), secrets);
        // A setup from before the vault: its stored secret values move into Personal (env
        // vault spec §11). A failure is logged and the app starts anyway.
        vault.mark_vault_since().await?;
        match vault.move_stored_literals().await {
            Ok(0) => {}
            Ok(n) => tracing::info!("moved the secret values of {n} MCP server(s) to the vault"),
            Err(e) => tracing::error!("moving stored secret values to the vault failed: {e}"),
        }
        let bus = new_bus();
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    /// The skill creator's drafts: what `skill_draft` and `skill_draft_cancel` call (skill
```

**with:**

```rust

    /// The update backups made before this data folder first ran with the vault, until the
    /// offer is answered (env vault spec §11).
    pub async fn old_backups(&self) -> Result<Vec<String>> {
        self.vault.old_backups(&self.data_dir).await
    }

    /// Deletes the old backups when `delete`, and answers the offer.
    pub async fn answer_old_backups(&self, delete: bool) -> Result<()> {
        self.vault.answer_old_backups(&self.data_dir, delete).await
    }

    /// The skill creator's drafts: what `skill_draft` and `skill_draft_cancel` call (skill
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
```

**with:**

```rust

    /// Sets a dot's grants as they are, for a change Dots makes itself (env vault spec §11).
    pub(crate) async fn set_dot_variables(&self, id: &str, grants: &Grants) -> Result<()> {
        sqlx::query("UPDATE dots SET variables = ? WHERE id = ?")
            .bind(serde_json::to_string(grants)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod upgrade;

```

**with:**

```rust
mod upgrade;

pub(crate) use upgrade::{auth_scheme, var_name};

```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust

    /// Closes every connection of this store.
```

**with:**

```rust

    /// Rewrites the database so freed pages lose what they held, and empties the WAL.
    pub(crate) async fn vacuum(&self) -> Result<()> {
        sqlx::query("VACUUM").execute(&self.pool).await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Closes every connection of this store.
```

**In `crates/dots-core/src/store/settings.rs`, replace:**

```rust
        }
    }
```

**with:**

```rust
        }
    }

    /// A marker Dots keeps in the `settings` table outside `Settings` (`key` is none of its
    /// fields), as JSON.
    pub async fn marker(&self, key: &str) -> Result<Option<Value>> {
        let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(raw.map(|r| serde_json::from_str(&r)).transpose()?)
    }

    /// Sets a marker; with `keep` an existing one is left as it is.
    pub async fn set_marker(&self, key: &str, value: &Value, keep: bool) -> Result<()> {
        let sql = if keep {
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)"
        } else {
            "INSERT INTO settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value"
        };
        sqlx::query(sql)
            .bind(key)
            .bind(value.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
/// An upper-case variable name made from `s`: `-` and other characters become `_`.
fn var_name(s: &str) -> String {
    s.chars()
```

**with:**

```rust
/// An upper-case variable name made from `s`: `-` and other characters become `_`.
pub(crate) fn var_name(s: &str) -> String {
    s.chars()
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
/// `Bearer`. A first word that does not read as one may be part of the secret: it is not kept.
fn auth_scheme(value: &str) -> &str {
    let mut words = value.split_whitespace();
```

**with:**

```rust
/// `Bearer`. A first word that does not read as one may be part of the secret: it is not kept.
pub(crate) fn auth_scheme(value: &str) -> &str {
    let mut words = value.split_whitespace();
```

**Create `crates/dots-core/src/vault/literals.rs`:**

```rust
//! Secret values typed into an MCP server's config (env vault spec §11): a secret-looking env
//! value or header with no `${…}` moves into Personal and the config reads `${NAME}` instead,
//! so the value is never stored in the Library. An env value goes under its key
//! (`SF_TOKEN`), an `Authorization` header under `<SERVER>_AUTHORIZATION` with its scheme
//! kept, another header under `<SERVER>_<HEADER>`.

use super::entries::{check_variable_name, EntryKind};
use super::secrets::MAX_SECRET_BYTES;
use super::service::Vault;
use crate::library::{looks_secret, McpServerConfig};
use crate::store::{auth_scheme, var_name};
use crate::Result;

/// Where a value sits in a config.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Env(String),
    Header(String),
}

/// A secret-looking value of a config: where it is, the name it moves under, the value, and
/// the text around `${NAME}` (an `Authorization` header keeps its scheme).
#[derive(Debug, Clone)]
struct Found {
    place: Place,
    name: String,
    value: String,
    prefix: String,
}

fn hidden(key: &str, value: &str) -> bool {
    looks_secret(key) && !value.contains("${") && !value.trim().is_empty()
}

/// The secret-looking values of the server `server`'s config.
fn found(server: &str, config: &McpServerConfig) -> Vec<Found> {
    let mut out = Vec::new();
    match config {
        McpServerConfig::Stdio { env, .. } => {
            for (key, value) in env {
                if hidden(key, value) {
                    out.push(Found {
                        place: Place::Env(key.clone()),
                        name: var_name(key),
                        value: value.clone(),
                        prefix: String::new(),
                    });
                }
            }
        }
        McpServerConfig::Http { headers, .. } => {
            for (key, value) in headers {
                if !hidden(key, value) {
                    continue;
                }
                let (prefix, secret) = if key.eq_ignore_ascii_case("authorization") {
                    let scheme = auth_scheme(value);
                    let secret = match value.trim_start().strip_prefix(scheme) {
                        Some(rest) if value.split_whitespace().count() > 1 => rest.trim(),
                        _ => value.trim(),
                    };
                    (format!("{scheme} "), secret.to_string())
                } else {
                    (String::new(), value.clone())
                };
                out.push(Found {
                    place: Place::Header(key.clone()),
                    name: format!("{}_{}", var_name(server), var_name(key)),
                    value: secret,
                    prefix,
                });
            }
        }
    }
    out
}

/// Whether `config` holds a secret-looking value that is not `${…}`.
pub fn has_literals(server: &str, config: &McpServerConfig) -> bool {
    !found(server, config).is_empty()
}

impl Vault {
    /// `config` with each secret-looking value moved into Personal as a secret, and the names
    /// it moved under. A name Personal has with the same value is reused; one it has with
    /// another value gets `_2`, `_3`, … A value too long for the secret store stays where it is.
    pub async fn move_literals(
        &self,
        server: &str,
        config: &McpServerConfig,
    ) -> Result<(McpServerConfig, Vec<String>)> {
        let mut config = config.clone();
        let mut moved = Vec::new();
        for f in found(server, &config) {
            if f.value.len() > MAX_SECRET_BYTES || check_variable_name(&f.name).is_err() {
                tracing::warn!(
                    "{server}: a value of {:?} is too long to move to the vault",
                    f.name
                );
                continue;
            }
            let mut name = f.name.clone();
            for n in 2.. {
                match self.personal_value(&name).await? {
                    None => {
                        self.add(&name, EntryKind::Secret, &f.value).await?;
                        break;
                    }
                    Some(v) if *v == f.value => break,
                    Some(_) => name = format!("{}_{n}", f.name),
                }
            }
            let text = format!("{}${{{name}}}", f.prefix);
            match (&mut config, &f.place) {
                (McpServerConfig::Stdio { env, .. }, Place::Env(key)) => {
                    env.insert(key.clone(), text);
                }
                (McpServerConfig::Http { headers, .. }, Place::Header(key)) => {
                    headers.insert(key.clone(), text);
                }
                _ => unreachable!("a place of this config's transport"),
            }
            moved.push(name);
        }
        Ok((config, moved))
    }
}
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust
pub mod grants;
pub mod redact;
```

**with:**

```rust
pub mod grants;
pub mod literals;
pub mod redact;
```

**In `crates/dots-core/src/vault/mod.rs`, replace:**

```rust
pub mod service;

```

**with:**

```rust
pub mod service;
pub mod upgrade;

```

**In `crates/dots-core/src/vault/service.rs`, replace:**

```rust
        Vault { store, secrets }
    }
```

**with:**

```rust
        Vault { store, secrets }
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }
```

**Create `crates/dots-core/src/vault/upgrade.rs`:**

```rust
//! Bringing a setup from before the vault in (env vault spec §11): secret values stored in the
//! Library move into Personal, the dots that use their servers are granted the new names, and
//! the Env tab offers, once each, to import variables from Dots' Windows environment and to
//! delete update backups made before the vault.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::entries::EntryKind;
use super::grants::Consumer;
use super::literals::has_literals;
use super::service::Vault;
use crate::library::LibraryConfig;
use crate::library_service::process_env;
use crate::{Error, Result};

/// When this data folder first ran with the vault; backups older than that may hold values.
pub const VAULT_SINCE: &str = "vault_since";
/// The import offer was answered.
pub const IMPORT_ANSWERED: &str = "vault_import_answered";
/// The old backups offer was answered.
pub const BACKUPS_ANSWERED: &str = "vault_backups_answered";
/// The data folder's backups folder (the app's `update::BACKUPS_DIR`).
pub const BACKUPS_DIR: &str = "backups";

impl Vault {
    /// Moves every secret value stored in a Library server's config into Personal, and
    /// grants the dots that use the server the names it moved under. The database is vacuumed
    /// after a move, so its freed pages lose the old values. Returns how many servers changed.
    pub async fn move_stored_literals(&self) -> Result<usize> {
        let store = self.store();
        let mut changed = 0;
        for item in store.list_library_items().await? {
            let LibraryConfig::McpServer(config) = &item.config else {
                continue;
            };
            if !has_literals(&item.name, config) {
                continue;
            }
            let (moved_config, names) = self.move_literals(&item.name, config).await?;
            if names.is_empty() {
                continue;
            }
            store
                .update_library_item(&item.id, &LibraryConfig::McpServer(moved_config))
                .await?;
            let key = Consumer::Mcp(item.name.clone()).key();
            for dot in store.dots_using(&item).await? {
                let mut grants = dot.spec.variables.clone();
                let list = grants.0.entry(key.clone()).or_default();
                for name in &names {
                    if !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                        list.push(name.clone());
                    }
                }
                store.set_dot_variables(&dot.id, &grants).await?;
            }
            changed += 1;
        }
        if changed > 0 {
            store.vacuum().await?;
        }
        Ok(changed)
    }

    /// Records when this data folder first ran with the vault, once.
    pub async fn mark_vault_since(&self) -> Result<()> {
        self.store()
            .set_marker(VAULT_SINCE, &json!(crate::util::now()), true)
            .await
    }

    /// The `${VAR}` names Library servers read that Dots' Windows environment sets and Personal
    /// does not have (`env_import_candidates`), until the offer is answered.
    pub async fn import_candidates(&self) -> Result<Vec<String>> {
        let store = self.store();
        if store.marker(IMPORT_ANSWERED).await?.is_some() {
            return Ok(Vec::new());
        }
        let mut names: Vec<String> = Vec::new();
        for item in store.list_library_items().await? {
            if let LibraryConfig::McpServer(config) = &item.config {
                for name in config.variables() {
                    let new = !names.iter().any(|n| n.eq_ignore_ascii_case(&name));
                    if new && process_env(&name).is_some() {
                        names.push(name);
                    }
                }
            }
        }
        let personal = self.personal_values(&names).await?;
        names.retain(|n| !personal.contains(n));
        names.sort();
        Ok(names)
    }

    /// Copies `names` from Dots' Windows environment into Personal as secrets and answers the
    /// offer (`env_import`). Only names `import_candidates` offers are taken.
    pub async fn import(&self, names: &[String]) -> Result<Vec<String>> {
        let offered = self.import_candidates().await?;
        let mut imported = Vec::new();
        for name in names {
            if !offered.contains(name) {
                return Err(Error::Invalid(format!("{name} is not offered for import")));
            }
            if let Some(value) = process_env(name) {
                self.add(name, EntryKind::Secret, &value).await?;
                imported.push(name.clone());
            }
        }
        self.answer(IMPORT_ANSWERED).await?;
        Ok(imported)
    }

    /// Answers the import offer without importing (`env_import_dismiss`).
    pub async fn dismiss_import(&self) -> Result<()> {
        self.answer(IMPORT_ANSWERED).await
    }

    /// The update backups in `data_dir` made before this data folder first ran with the vault
    /// (`env_old_backups`), by file name, until the offer is answered.
    pub async fn old_backups(&self, data_dir: &Path) -> Result<Vec<String>> {
        Ok(self
            .old_backup_paths(data_dir)
            .await?
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect())
    }

    /// Deletes the old backups when `delete`, and answers the offer either way
    /// (`env_old_backups_answer`).
    pub async fn answer_old_backups(&self, data_dir: &Path, delete: bool) -> Result<()> {
        if delete {
            for path in self.old_backup_paths(data_dir).await? {
                std::fs::remove_file(&path).map_err(|e| {
                    Error::Other(format!("could not delete {}: {e}", path.display()))
                })?;
            }
        }
        self.answer(BACKUPS_ANSWERED).await
    }

    async fn old_backup_paths(&self, data_dir: &Path) -> Result<Vec<PathBuf>> {
        let store = self.store();
        if store.marker(BACKUPS_ANSWERED).await?.is_some() {
            return Ok(Vec::new());
        }
        let Some(Value::String(since)) = store.marker(VAULT_SINCE).await? else {
            return Ok(Vec::new());
        };
        let Ok(since) = chrono::DateTime::parse_from_rfc3339(&since) else {
            return Ok(Vec::new());
        };
        let Ok(entries) = std::fs::read_dir(data_dir.join(BACKUPS_DIR)) else {
            return Ok(Vec::new());
        };
        let mut out: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "db"))
            .filter(|e| {
                e.metadata().and_then(|m| m.modified()).is_ok_and(|t| {
                    chrono::DateTime::<chrono::Utc>::from(t) < since.with_timezone(&chrono::Utc)
                })
            })
            .map(|e| e.path())
            .collect();
        out.sort();
        Ok(out)
    }

    async fn answer(&self, key: &str) -> Result<()> {
        self.store()
            .set_marker(key, &json!(crate::util::now()), false)
            .await
    }
}
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  envRemove: (name: string) => call(nothing, "env_remove", { name }),

```

**with:**

```ts
  envRemove: (name: string) => call(nothing, "env_remove", { name }),
  /// Names Library servers read that Windows sets and Personal lacks; never a value. Empty once
  /// the offer is answered.
  envImportCandidates: () => call(z.string().array(), "env_import_candidates"),
  /// Copies them into Personal as secrets and answers the offer; answers the names imported.
  envImport: (names: string[]) => call(z.string().array(), "env_import", { names }),
  envImportDismiss: () => call(nothing, "env_import_dismiss"),
  /// Update backups made before the vault, by file name; empty once the offer is answered.
  envOldBackups: () => call(z.string().array(), "env_old_backups"),
  envOldBackupsAnswer: (deleteThem: boolean) =>
    call(nothing, "env_old_backups_answer", { delete: deleteThem }),

```

- [ ] **Step 4: Run the tests and write the contract**

Run: `cargo test -p dots-core --test vault_upgrade_test`, then `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
Expected: 8 passed; UI 451.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `bun run --cwd ui typecheck`
Expected: no warnings; core 773, app 94 passed.

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/lib.rs crates/dots-core/src/library_service.rs crates/dots-core/src/run_tools.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/dots.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/settings.rs crates/dots-core/src/store/upgrade.rs crates/dots-core/src/vault/literals.rs crates/dots-core/src/vault/mod.rs crates/dots-core/src/vault/service.rs crates/dots-core/src/vault/upgrade.rs crates/dots-core/tests/vault_upgrade_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/commands.test.ts ui/src/api/commands.ts
git commit -m "feat: secret values typed into the Library move into Personal; import and old backups offers"
```

### Task 7: Webhook tokens kept hashed and shown once; the gateway file owner-only

Spec §11:

- **Webhook tokens.** A dot's token is stored as `sha256:<hex>`, and `finish_0009` hashes the ones stored before. A new dot has none. A token is shown once, when it is made, and is accepted only as `Authorization: Bearer`, so `?token=` is refused. `Dot` carries `has_webhook_token` in place of the token.
- **The gateway file.** On Windows, `write_private` gives the per-run file a DACL with only the signed-in user, before its secret is written.

**Files:**

- Modify:
  - `crates/dots-core/Cargo.toml`, `crates/dots-core/migrations/0009_vault.sql`
  - `crates/dots-core/src/{model,proc,util}.rs`, `crates/dots-core/src/engine/claude/mod.rs`, `crates/dots-core/src/server/webhook.rs`, `crates/dots-core/src/store/{dots,upgrade}.rs`
  - `apps/dots-app/src/contract.rs`
  - `ui/src/api/types.ts`, `ui/src/screens/dot-editor/WebhookPanel.tsx`
- Regenerate: the contract fixture
- Tests: `crates/dots-core/tests/{common/mod,proc_test,runtime_test,server_test,store_dots_test,vault_grants_test}.rs`, `ui/src/lib/dot-rules.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`

**Interfaces:**

- Consumes: Task 3's `finish_0009`.
- Produces:
  - `util::token_hash(&str) -> String`.
  - `Store::check_webhook_token(id, token) -> Result<bool>`.
  - `Dot::has_webhook_token: bool`, replacing `webhook_token`.
  - `proc::owner_only(&Path) -> io::Result<()>` (Windows).
  - The UI's `T.Dot` gets `has_webhook_token`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/common/mod.rs`, replace:**

```rust
        id: format!("dot-{name}"),
        webhook_token: "t".into(),
        created_at: "2026-10-08T00:00:00.000Z".into(),
```

**with:**

```rust
        id: format!("dot-{name}"),
        has_webhook_token: true,
        created_at: "2026-10-08T00:00:00.000Z".into(),
```

**In `crates/dots-core/tests/proc_test.rs`, replace:**

```rust
    #[cfg(windows)]
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after terminate"
    );
}
```

**with:**

```rust
    #[cfg(windows)]
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after terminate"
    );
}

/// The gateway file's DACL: one entry, the signed-in user's, nothing inherited (env vault spec
/// §11).
#[cfg(windows)]
#[test]
fn an_owner_only_file_has_one_entry_for_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-run.json");
    std::fs::write(&path, "{}").unwrap();
    dots_core::proc::owner_only(&path).unwrap();
    let out = std::process::Command::new("icacls")
        .arg(&path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let user = std::env::var("USERNAME").unwrap();
    let entries: Vec<&str> = text
        .lines()
        .filter(|l| l.contains(":(") && !l.contains("Successfully"))
        .collect();
    assert_eq!(entries.len(), 1, "{text}");
    assert!(
        entries[0].contains(&user) && entries[0].contains("(F)"),
        "{text}"
    );
    assert!(!text.contains("(I)"), "{text}");
}
```

**In `crates/dots-core/tests/runtime_test.rs`, replace:**

```rust
    let url = t.rt.webhook_url(&dot).unwrap();
    assert!(url.starts_with(&format!("http://127.0.0.1:{}/dots/", t.rt.port())));
```

**with:**

```rust
    let url = t.rt.webhook_url(&dot).unwrap();
    let token = t.rt.regenerate_webhook_token(&dot.id).await.unwrap();
    assert!(url.starts_with(&format!("http://127.0.0.1:{}/dots/", t.rt.port())));
```

**In `crates/dots-core/tests/runtime_test.rs`, replace:**

```rust
        .post(&url)
        .bearer_auth(&dot.webhook_token)
        .json(&json!({"event": "build_failed"}))
```

**with:**

```rust
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({"event": "build_failed"}))
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
}

#[tokio::test]
async fn health() {
```

**with:**

```rust
}

/// A dot with a webhook token, and the token.
async fn hooked(srv: &Srv, name: &str) -> (Dot, String) {
    let d = dot(srv, name, 5).await;
    let token = srv.store.regenerate_webhook_token(&d.id).await.unwrap();
    (d, token)
}

#[tokio::test]
async fn health() {
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
async fn webhook_accepts_valid_requests() {
    let srv = start().await;
    let d = dot(&srv, "hook", 5).await;
    let url = format!("{}/dots/{}/trigger", srv.base, d.id);
    let r = srv
```

**with:**

```rust
async fn webhook_accepts_valid_requests() {
    let srv = start().await;
    let (d, token) = hooked(&srv, "hook").await;
    let url = format!("{}/dots/{}/trigger", srv.base, d.id);
    let r = srv
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
        .post(&url)
        .bearer_auth(&d.webhook_token)
        .json(&json!({"case": 7}))
```

**with:**

```rust
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({"case": 7}))
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
        .http
        .post(format!("{url}?token={}", d.webhook_token))
        .send()
```

**with:**

```rust
        .http
        .post(&url)
        .bearer_auth(&token)
        .send()
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
        None
    );
}

```

**with:**

```rust
        None
    );
    // A token in the URL is refused: it would end up in proxy logs (env vault spec §11).
    let r = srv
        .http
        .post(format!("{url}?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
    let srv = start().await;
    let d = dot(&srv, "hook", 5).await;
    let url = format!("{}/dots/{}/trigger", srv.base, d.id);
```

**with:**

```rust
    let srv = start().await;
    let fresh = dot(&srv, "fresh", 5).await;
    let (d, token) = hooked(&srv, "hook").await;
    let url = format!("{}/dots/{}/trigger", srv.base, d.id);
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
    assert_eq!(post(srv.http.post(&url).bearer_auth("wrong")).await, 401);
    assert_eq!(
```

**with:**

```rust
    assert_eq!(post(srv.http.post(&url).bearer_auth("wrong")).await, 401);
    // A dot without a token takes none.
    let fresh_url = format!("{}/dots/{}/trigger", srv.base, fresh.id);
    assert_eq!(post(srv.http.post(&fresh_url).bearer_auth("")).await, 401);
    assert_eq!(
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
    assert_eq!(
        post(
            srv.http
                .post(&url)
                .bearer_auth(&d.webhook_token)
                .body("not json")
        )
        .await,
        400
```

**with:**

```rust
    assert_eq!(
        post(srv.http.post(&url).bearer_auth(&token).body("not json")).await,
        400
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
                .post(&url)
                .bearer_auth(&d.webhook_token)
                .body(format!("\"{big}\""))
```

**with:**

```rust
                .post(&url)
                .bearer_auth(&token)
                .body(format!("\"{big}\""))
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
    srv.store.set_dot_enabled(&d.id, false).await.unwrap();
    assert_eq!(
        post(srv.http.post(&url).bearer_auth(&d.webhook_token)).await,
        409
    );
    assert!(srv
```

**with:**

```rust
    srv.store.set_dot_enabled(&d.id, false).await.unwrap();
    assert_eq!(post(srv.http.post(&url).bearer_auth(&token)).await, 409);
    assert!(srv
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    assert_eq!(dot.id.len(), 26);
    assert_eq!(dot.webhook_token.len(), 43);
    assert_eq!(dot.created_at, dot.updated_at);
```

**with:**

```rust
    assert_eq!(dot.id.len(), 26);
    // No webhook token until one is made (env vault spec §11).
    assert!(!dot.has_webhook_token);
    assert_eq!(dot.created_at, dot.updated_at);
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec, s);
    assert_eq!(updated.webhook_token, dot.webhook_token);
    assert!(updated.updated_at > dot.updated_at);
```

**with:**

```rust
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec, s);
    assert_eq!(updated.has_webhook_token, dot.has_webhook_token);
    assert!(updated.updated_at > dot.updated_at);
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    assert!(!disabled.spec.enabled);
    let token = store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_ne!(token, dot.webhook_token);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().webhook_token, token);
    store.delete_dot(&dot.id).await.unwrap();
```

**with:**

```rust
    assert!(!disabled.spec.enabled);
    assert!(!store.check_webhook_token(&dot.id, "").await.unwrap());
    let first = store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_eq!(first.len(), 43);
    assert!(store.get_dot(&dot.id).await.unwrap().has_webhook_token);
    assert!(store.check_webhook_token(&dot.id, &first).await.unwrap());
    let token = store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_ne!(token, first);
    assert!(!store.check_webhook_token(&dot.id, &first).await.unwrap());
    assert!(store.check_webhook_token(&dot.id, &token).await.unwrap());
    // Only its hash is stored.
    let stored: String = sqlx::query_scalar("SELECT webhook_token FROM dots WHERE id = ?")
        .bind(&dot.id)
        .fetch_one(
            &sqlx::SqlitePool::connect_with(
                sqlx::sqlite::SqliteConnectOptions::new().filename(_d.path().join("dots.db")),
            )
            .await
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(stored.starts_with("sha256:") && !stored.contains(&token));
    store.delete_dot(&dot.id).await.unwrap();
```

**In `crates/dots-core/tests/vault_grants_test.rs`, replace:**

```rust
    let want = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);
    assert_eq!(store.get_dot("D1").await.unwrap().spec.variables, want);
    store.close().await;
```

**with:**

```rust
    let want = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);
    let dot = store.get_dot("D1").await.unwrap();
    assert_eq!(dot.spec.variables, want);
    // Its webhook token is kept as a hash, and still works.
    assert!(dot.has_webhook_token);
    assert!(store.check_webhook_token("D1", "t").await.unwrap());
    store.close().await;
```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
    expect(spec).not.toHaveProperty("id");
    expect(spec).not.toHaveProperty("webhook_token");
    expect(spec.name).toBe(sample.dot.name);
```

**with:**

```ts
    expect(spec).not.toHaveProperty("id");
    expect(spec).not.toHaveProperty("has_webhook_token");
    expect(spec.name).toBe(sample.dot.name);
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx

  it("shows the webhook, copies it, and makes a new token after a confirmation", async () => {
    const held = stored();
```

**with:**

```tsx

  it("shows the webhook, and a new token once, after a confirmation", async () => {
    const held = stored();
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, webhook_token: "tok-new" };
      return "tok-new";
```

**with:**

```tsx
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, has_webhook_token: true };
      return "tok-new";
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    await waitFor(() => expect(screen.getByLabelText("Webhook URL")).toHaveValue(url));
    expect(screen.getByLabelText("Token")).toHaveValue(dot.webhook_token);

```

**with:**

```tsx
    await waitFor(() => expect(screen.getByLabelText("Webhook URL")).toHaveValue(url));
    // The token is kept as a hash: it is not shown again.
    expect(screen.getByText(/This dot has a token\. It was shown once/)).toBeVisible();
    expect(screen.queryByLabelText("Token")).toBeNull();

```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    await waitFor(() => expect(screen.getByLabelText("Token")).toHaveValue("tok-new"));
    expect(backend.callsTo("regenerate_webhook_token")).toEqual([{ id: dot.id }]);
  });
```

**with:**

```tsx
    await waitFor(() => expect(screen.getByLabelText("Token")).toHaveValue("tok-new"));
    expect(screen.getByText("Copy it now: you won't see this again.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Copy the token" }));
    expect(await navigator.clipboard.readText()).toBe("tok-new");
    expect(backend.callsTo("regenerate_webhook_token")).toEqual([{ id: dot.id }]);
  });

  it("makes a dot's first token without asking", async () => {
    const held = stored();
    held.dot = { ...held.dot, has_webhook_token: false };
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, has_webhook_token: true };
      return "tok-first";
    });
    const { user } = renderApp(path);
    expect(await screen.findByText(/no token yet/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Make a token" }));
    await waitFor(() => expect(screen.getByLabelText("Token")).toHaveValue("tok-first"));
  });
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, name: "renamed", instructions: "Stored.", webhook_token: "tok-new" };
      return "tok-new";
```

**with:**

```tsx
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, name: "renamed", instructions: "Stored.", has_webhook_token: true };
      return "tok-new";
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, webhook_token: "tok-new" };
      backend.on("get_dot", () => {
```

**with:**

```tsx
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, has_webhook_token: true };
      backend.on("get_dot", () => {
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test store_dots_test --test server_test --test proc_test`
Expected: the build fails: `Dot::has_webhook_token`, `Store::check_webhook_token` and `proc::owner_only` are not defined.

- [ ] **Step 3: Write the code**

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        id: "01J0DOT".into(),
        webhook_token: "tok_abc".into(),
        created_at: TS.into(),
```

**with:**

```rust
        id: "01J0DOT".into(),
        has_webhook_token: true,
        created_at: TS.into(),
```

**In `crates/dots-core/Cargo.toml`, replace:**

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Foundation", "Win32_Security", "Win32_Security_Credentials", "Win32_System_JobObjects", "Win32_System_Threading"] }

```

**with:**

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Foundation", "Win32_Security", "Win32_Security_Authorization", "Win32_Security_Credentials", "Win32_System_JobObjects", "Win32_System_Threading"] }

```

**In `crates/dots-core/migrations/0009_vault.sql`, replace:**

```sql

-- Each dot is granted the variables of the servers it uses when the store opens
```

**with:**

```sql

-- Webhook tokens are kept as their hash (`sha256:<hex>`, section 11); `finish_0009` hashes the
-- stored ones. An empty token is none.

-- Each dot is granted the variables of the servers it uses when the store opens
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust

/// Writes a file readable only by the owner on unix (it holds a bearer secret).
async fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
```

**with:**

```rust

/// Writes a file readable only by the owner (it holds a bearer secret): mode 0600 on unix; on
/// Windows its DACL is set to the owner alone before the secret is written.
async fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
    let mut f = opts.open(path).await?;
    f.write_all(data).await?;
```

**with:**

```rust
    let mut f = opts.open(path).await?;
    #[cfg(windows)]
    crate::proc::owner_only(path)?;
    f.write_all(data).await?;
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub id: String,
    pub webhook_token: String,
    pub created_at: String,
```

**with:**

```rust
    pub id: String,
    /// Whether the dot has a webhook token. The token is shown once, when it is made; Dots
    /// keeps only its hash (env vault spec §11).
    pub has_webhook_token: bool,
    pub created_at: String,
```

**In `crates/dots-core/src/proc.rs`, replace:**

```rust
    String::from_utf8_lossy(&buf).to_string()
}
```

**with:**

```rust
    String::from_utf8_lossy(&buf).to_string()
}

/// Gives the file at `path` a DACL with one entry, full access for the signed-in user, and no
/// inherited ones (env vault spec §11: the gateway file holds a run's bearer secret).
#[cfg(windows)]
pub fn owner_only(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, GENERIC_ALL, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SET_ACCESS, SE_FILE_OBJECT,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let fail = |code: u32| std::io::Error::from_raw_os_error(code as i32);
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of this process; `token` receives a handle closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buf = vec![0u8; 256];
    let mut len = 0u32;
    // SAFETY: `buf` is `buf.len()` bytes; a larger need is reported in `len` and retried.
    let mut ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut len,
        )
    };
    if ok == 0 && len as usize > buf.len() {
        buf = vec![0u8; len as usize];
        // SAFETY: as above, with the size asked for.
        ok = unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len)
        };
    }
    let err = std::io::Error::last_os_error();
    // SAFETY: `token` came from OpenProcessToken.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err(err);
    }
    // SAFETY: GetTokenInformation filled `buf` with a TOKEN_USER whose SID points into `buf`.
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid.cast(),
            ..Default::default()
        },
    };
    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: one entry, no old ACL; `acl` receives a LocalAlloc'd ACL freed below.
    let code = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
    if code != 0 {
        return Err(fail(code));
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is NUL-terminated and `acl` valid until it is freed after the call.
    let code = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    // SAFETY: `acl` came from SetEntriesInAclW.
    unsafe { LocalFree(acl.cast()) };
    if code != 0 {
        return Err(fail(code));
    }
    Ok(())
}
```

**In `crates/dots-core/src/server/webhook.rs`, replace:**

```rust
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
```

**with:**

```rust
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::AUTHORIZATION;
```

**In `crates/dots-core/src/server/webhook.rs`, replace:**

```rust
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
```

**with:**

```rust
use axum::Json;
use serde_json::{json, Value};
```

**In `crates/dots-core/src/server/webhook.rs`, replace:**

```rust
use crate::model::{NewRun, TriggerKind};
use crate::util::ct_eq;
use crate::Error;

#[derive(Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

```

**with:**

```rust
use crate::model::{NewRun, TriggerKind};
use crate::Error;

```

**In `crates/dots-core/src/server/webhook.rs`, replace:**

```rust
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    headers: HeaderMap,
```

**with:**

```rust
    Path(id): Path<String>,
    headers: HeaderMap,
```

**In `crates/dots-core/src/server/webhook.rs`, replace:**

```rust
    };
    let token = bearer(&headers).or(q.token);
    if !token
        .as_deref()
        .is_some_and(|t| ct_eq(t, &dot.webhook_token))
    {
        return error(StatusCode::UNAUTHORIZED, "invalid token");
```

**with:**

```rust
    };
    // A bearer header only: a token in the URL ends up in proxy logs (env vault spec §11).
    let valid = match bearer(&headers) {
        Some(token) => match s.store.check_webhook_token(&dot.id, &token).await {
            Ok(valid) => valid,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        },
        None => false,
    };
    if !valid {
        return error(StatusCode::UNAUTHORIZED, "invalid token");
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use crate::schedule::Schedule;
use crate::util::{new_id, new_token, now};
use crate::vault::Grants;
```

**with:**

```rust
use crate::schedule::Schedule;
use crate::util::{ct_eq, new_id, new_token, now, token_hash};
use crate::vault::Grants;
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        id: row.try_get("id")?,
        webhook_token: row.try_get("webhook_token")?,
        created_at: row.try_get("created_at")?,
```

**with:**

```rust
        id: row.try_get("id")?,
        has_webhook_token: !row.try_get::<String, _>("webhook_token")?.is_empty(),
        created_at: row.try_get("created_at")?,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(schedule_json(spec)?)
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
```

**with:**

```rust
        .bind(schedule_json(spec)?)
        // No token until one is made, which shows it once.
        .bind("")
        .bind(serde_json::to_string(&spec.policy)?)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust

    pub async fn regenerate_webhook_token(&self, id: &str) -> Result<String> {
```

**with:**

```rust

    /// Makes the dot a new webhook token and returns it: the only time it is shown. Only its
    /// hash is kept.
    /// Whether `token` is the dot's webhook token. A dot without one takes none.
    pub async fn check_webhook_token(&self, id: &str, token: &str) -> Result<bool> {
        let stored: Option<String> =
            sqlx::query_scalar("SELECT webhook_token FROM dots WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let stored = stored.ok_or_else(|| Error::NotFound(format!("dot {id}")))?;
        Ok(!stored.is_empty() && ct_eq(&token_hash(token), &stored))
    }

    pub async fn regenerate_webhook_token(&self, id: &str) -> Result<String> {
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        let res = sqlx::query("UPDATE dots SET webhook_token = ?, updated_at = ? WHERE id = ?")
            .bind(&token)
            .bind(now())
```

**with:**

```rust
        let res = sqlx::query("UPDATE dots SET webhook_token = ?, updated_at = ? WHERE id = ?")
            .bind(token_hash(&token))
            .bind(now())
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
/// Grants each dot listed in `upgrade_0009` the variables of the servers it uses, as ticking
/// them would (env vault spec §11 step 2), then drops the table. After 0006, so the servers it
/// made are there.
```

**with:**

```rust
/// Grants each dot listed in `upgrade_0009` the variables of the servers it uses, as ticking
/// them would (env vault spec §11 step 2), hashes its webhook token, then drops the table. After 0006, so the servers it
/// made are there.
```

**In `crates/dots-core/src/store/upgrade.rs`, replace:**

```rust
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE upgrade_0009")
```

**with:**

```rust
            .execute(&mut *tx)
            .await?;
        // The token in clear becomes its hash: it keeps working, and is never shown again.
        let token: String = sqlx::query_scalar("SELECT webhook_token FROM dots WHERE id = ?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
        if !token.is_empty() && !token.starts_with("sha256:") {
            sqlx::query("UPDATE dots SET webhook_token = ? WHERE id = ?")
                .bind(crate::util::token_hash(&token))
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("DROP TABLE upgrade_0009")
```

**In `crates/dots-core/src/util.rs`, replace:**

```rust

/// Constant-time string comparison for secrets.
```

**with:**

```rust

/// How a webhook token is stored (env vault spec §11): `sha256:` and the hex SHA-256 of it.
pub fn token_hash(token: &str) -> String {
    let hex: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// Constant-time string comparison for secrets.
```

**In `ui/src/api/types.ts`, replace:**

```ts
  id: z.string(),
  webhook_token: z.string(),
  created_at: z.string(),
```

**with:**

```ts
  id: z.string(),
  // The token itself is shown once, when it is made.
  has_webhook_token: z.boolean(),
  created_at: z.string(),
```

**In `ui/src/screens/dot-editor/WebhookPanel.tsx`, replace:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy } from "lucide-react";
```

**with:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Copy } from "lucide-react";
```

**In `ui/src/screens/dot-editor/WebhookPanel.tsx`, replace:**

```tsx

/// How another program starts this dot: its trigger URL and bearer token.
export function WebhookPanel({ dot }: { dot: Dot }) {
```

**with:**

```tsx

/// How another program starts this dot: its trigger URL and bearer token. Dots keeps only the
/// token's hash, so a token is shown once, when it is made.
export function WebhookPanel({ dot }: { dot: Dot }) {
```

**In `ui/src/screens/dot-editor/WebhookPanel.tsx`, replace:**

```tsx
  const qc = useQueryClient();
  const url = useQuery({
```

**with:**

```tsx
  const qc = useQueryClient();
  const [shown, setShown] = useState<string | null>(null);
  const url = useQuery({
```

**In `ui/src/screens/dot-editor/WebhookPanel.tsx`, replace:**

```tsx
    onSuccess: (token) => {
      invalidateDots(qc);
      qc.setQueryData(keys.dot(dot.id), { ...dot, webhook_token: token });
      toast.success("New token. The old one no longer works.");
    },
```

**with:**

```tsx
    onSuccess: (token) => {
      setShown(token);
      invalidateDots(qc);
      qc.setQueryData(keys.dot(dot.id), { ...dot, has_webhook_token: true });
      toast.success(
        dot.has_webhook_token ? "New token. The old one no longer works." : "Token made.",
      );
    },
```

**In `ui/src/screens/dot-editor/WebhookPanel.tsx`, replace:**

```tsx
      </Field>
      <Field label="Token" htmlFor="webhook-token">
        <div className="flex gap-2">
          <Input id="webhook-token" readOnly className="font-mono" value={dot.webhook_token} />
          <CopyButton what="token" text={dot.webhook_token} />
          <ConfirmButton
```

**with:**

```tsx
      </Field>
      {shown !== null ? (
        <Field
          label="Token"
          htmlFor="webhook-token"
          hint="Copy it now: you won't see this again."
        >
          <div className="flex gap-2">
            <Input id="webhook-token" readOnly className="font-mono" value={shown} />
            <CopyButton what="token" text={shown} />
          </div>
        </Field>
      ) : (
        <p className="text-sm text-muted-foreground">
          {dot.has_webhook_token
            ? "This dot has a token. It was shown once, when it was made."
            : "This dot has no token yet, so its webhook refuses every request."}
        </p>
      )}
      <div>
        {dot.has_webhook_token ? (
          <ConfirmButton
```

**In `ui/src/screens/dot-editor/WebhookPanel.tsx`, replace:**

```tsx
          </ConfirmButton>
        </div>
      </Field>
    </div>
```

**with:**

```tsx
          </ConfirmButton>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => regenerate.mutate()}
            disabled={regenerate.isPending}
          >
            Make a token
          </Button>
        )}
      </div>
    </div>
```

- [ ] **Step 4: Run the tests and write the contract**

Run: `cargo test -p dots-core --test proc_test --test vault_grants_test --test server_test --test store_dots_test --test runtime_test`, then `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
Expected: all pass; UI 452.

- [ ] **Step 5: Check and commit**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `bun run --cwd ui typecheck`
Expected: no warnings; core 774, app 94 passed.

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/Cargo.toml crates/dots-core/migrations/0009_vault.sql crates/dots-core/src/engine/claude/mod.rs crates/dots-core/src/model.rs crates/dots-core/src/proc.rs crates/dots-core/src/server/webhook.rs crates/dots-core/src/store/dots.rs crates/dots-core/src/store/upgrade.rs crates/dots-core/src/util.rs crates/dots-core/tests/common/mod.rs crates/dots-core/tests/proc_test.rs crates/dots-core/tests/runtime_test.rs crates/dots-core/tests/server_test.rs crates/dots-core/tests/store_dots_test.rs crates/dots-core/tests/vault_grants_test.rs ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/lib/dot-rules.test.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/dot-editor/WebhookPanel.tsx
git commit -m "fix: webhook tokens are kept hashed and shown once; the gateway file is owner-only"
```

### Task 8: The Library's Env tab

The Env tab (spec §9):

- **Personal**, with each row's name, kind, and value (plain entries only).
- **Add variable**: a secret goes through a password field and is on by default.
- **Edit** a plain value, or **Replace value** for a secret, without showing it.
- **Remove**, after asking.
- The two one-time offers.

The server form now says variables come from Personal and that a typed secret moves there when the server is saved.

**Files:**

- Create: `ui/src/screens/library/EnvTab.tsx`
- Modify: `ui/src/screens/library/{LibraryScreen,ServerForm}.tsx`, `ui/src/lib/query.ts`
- Tests: `ui/src/screens/library/LibraryScreen.test.tsx`, `ui/src/test/fixtures.ts`

**Interfaces:**

- Consumes: Tasks 2 and 6's `commands.env*`.
- Produces:
  - `EnvTab`, and `validVariableName(name)` in `EnvTab.tsx`.
  - `keys.env`, `keys.envImport` and `keys.envBackups`.
  - The default test handlers `env_list`, `env_import_candidates` and `env_old_backups`, each answering `[]`.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx

  it("names the variables that Dots' environment does not set, and warns about a pasted secret", async () => {
    backend.on("environment_variables", (args) =>
```

**with:**

```tsx

  it("names the variables that Personal does not have, and says a pasted secret moves there", async () => {
    backend.on("environment_variables", (args) =>
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
    );
    expect(await within(dialog).findByText("Not set in Dots' environment: NOT_SET")).toBeVisible();
    expect(within(dialog).getByText(/API_TOKEN looks like a secret.*\$\{VAR\}/)).toBeVisible();
    expect(within(dialog).getByText(/reads the variable from Dots' environment/)).toBeVisible();
  });
```

**with:**

```tsx
    );
    expect(
      await within(dialog).findByText("Not in Personal: NOT_SET. Add it in the Env tab."),
    ).toBeVisible();
    expect(within(dialog).getByText(/API_TOKEN looks like a secret.*moves its value to Personal/)).toBeVisible();
    expect(within(dialog).getByText(/reads the variable from Personal/)).toBeVisible();
  });
```

**In `ui/src/screens/library/LibraryScreen.test.tsx`, replace:**

```tsx
    expect(name).not.toMatch(/[-_]$/);
  });
});
```

**with:**

```tsx
    expect(name).not.toMatch(/[-_]$/);
  });
});

describe("the Library's Env tab", () => {
  const secret = {
    name: "GITHUB_TOKEN",
    kind: "secret" as const,
    value: null,
    created_at: "2026-10-21T12:00:00.000Z",
    updated_at: "2026-10-21T12:00:00.000Z",
  };
  const plain = { ...secret, name: "LOG_LEVEL", kind: "plain" as const, value: "debug" };

  async function openTab(entries: unknown[] = [secret, plain]) {
    backend.on("env_list", () => entries);
    const rendered = renderApp("/library");
    await rendered.user.click(await screen.findByRole("tab", { name: "Env" }));
    return rendered;
  }

  it("lists the personal variables and never a secret's value", async () => {
    await openTab();
    const row = await screen.findByRole("listitem", { name: "GITHUB_TOKEN" });
    expect(within(row).getByText("secret")).toBeVisible();
    expect(within(row).getByText("Kept in Credential Manager")).toBeVisible();
    expect(within(row).getByRole("button", { name: "Replace value" })).toBeVisible();
    const config = screen.getByRole("listitem", { name: "LOG_LEVEL" });
    expect(within(config).getByText("debug")).toBeVisible();
    expect(within(config).getByRole("button", { name: "Edit" })).toBeVisible();
  });

  it("adds a secret through a password field, and checks the name first", async () => {
    backend.on("env_add", () => secret);
    const { user } = await openTab([]);
    expect(await screen.findByText("No variables yet.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Add variable" }));
    const dialog = await screen.findByRole("dialog", { name: "Add variable" });
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "password");
    await user.type(within(dialog).getByLabelText("Name"), "1BAD");
    await user.type(within(dialog).getByLabelText("Value"), "ghp_x");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText(/Use a letter or _/)).toBeVisible();
    expect(backend.callsTo("env_add")).toEqual([]);
    await user.clear(within(dialog).getByLabelText("Name"));
    await user.type(within(dialog).getByLabelText("Name"), "GITHUB_TOKEN");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(backend.callsTo("env_add")).toEqual([
      { name: "GITHUB_TOKEN", kind: "secret", value: "ghp_x" },
    ]);
  });

  it("adds plain config as text, and shows a refusal in the dialog", async () => {
    backend.on("env_add", () => {
      throw { kind: "conflict", message: "A variable named log_level is already in Personal." };
    });
    const { user } = await openTab();
    await user.click(await screen.findByRole("button", { name: "Add variable" }));
    const dialog = await screen.findByRole("dialog", { name: "Add variable" });
    await user.click(within(dialog).getByLabelText("Secret"));
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "text");
    await user.type(within(dialog).getByLabelText("Name"), "log_level");
    await user.type(within(dialog).getByLabelText("Value"), "info");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByText(/already in Personal/)).toBeVisible();
    expect(backend.callsTo("env_add")).toEqual([{ name: "log_level", kind: "plain", value: "info" }]);
  });

  it("replaces a secret's value without showing either, and edits plain config", async () => {
    backend.on("env_set", () => secret);
    const { user } = await openTab();
    const row = await screen.findByRole("listitem", { name: "GITHUB_TOKEN" });
    await user.click(within(row).getByRole("button", { name: "Replace value" }));
    const dialog = await screen.findByRole("dialog", { name: "Replace GITHUB_TOKEN" });
    expect(within(dialog).getByLabelText("Value")).toHaveValue("");
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "password");
    await user.type(within(dialog).getByLabelText("Value"), "ghp_new");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const config = screen.getByRole("listitem", { name: "LOG_LEVEL" });
    await user.click(within(config).getByRole("button", { name: "Edit" }));
    const edit = await screen.findByRole("dialog", { name: "Edit LOG_LEVEL" });
    expect(within(edit).getByLabelText("Value")).toHaveValue("debug");
    await user.clear(within(edit).getByLabelText("Value"));
    await user.type(within(edit).getByLabelText("Value"), "warn");
    await user.click(within(edit).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(backend.callsTo("env_set")).toEqual([
      { name: "GITHUB_TOKEN", value: "ghp_new" },
      { name: "LOG_LEVEL", value: "warn" },
    ]);
  });

  it("removes a variable after asking", async () => {
    backend.on("env_remove", () => null);
    const { user } = await openTab();
    const row = await screen.findByRole("listitem", { name: "GITHUB_TOKEN" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    await user.click(await screen.findByRole("button", { name: "Remove variable" }));
    await waitFor(() => expect(backend.callsTo("env_remove")).toEqual([{ name: "GITHUB_TOKEN" }]));
  });

  it("offers once to import what Windows sets, and to delete old backups", async () => {
    let offered = ["GITHUB_TOKEN"];
    backend.on("env_import_candidates", () => offered);
    backend.on("env_import", () => {
      offered = [];
      return ["GITHUB_TOKEN"];
    });
    let files = ["dots-0.1.0-2026-10-20-091700.db"];
    backend.on("env_old_backups", () => files);
    backend.on("env_old_backups_answer", () => {
      files = [];
      return null;
    });
    const { user } = await openTab([]);
    const offer = await screen.findByRole("region", { name: "Import" });
    expect(within(offer).getByText("GITHUB_TOKEN")).toBeVisible();
    await user.click(within(offer).getByRole("button", { name: "Import" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Import" })).toBeNull());
    expect(backend.callsTo("env_import")).toEqual([{ names: ["GITHUB_TOKEN"] }]);
    const backups = screen.getByRole("region", { name: "Old backups" });
    expect(within(backups).getByText("dots-0.1.0-2026-10-20-091700.db")).toBeVisible();
    await user.click(within(backups).getByRole("button", { name: "Keep them" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Old backups" })).toBeNull());
    expect(backend.callsTo("env_old_backups_answer")).toEqual([{ delete: false }]);
  });

  it("dismisses the import offer for good", async () => {
    let offered = ["SF_TOKEN"];
    backend.on("env_import_candidates", () => offered);
    backend.on("env_import_dismiss", () => {
      offered = [];
      return null;
    });
    const { user } = await openTab([]);
    const offer = await screen.findByRole("region", { name: "Import" });
    await user.click(within(offer).getByRole("button", { name: "Don't import" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Import" })).toBeNull());
    expect(backend.callsTo("env_import_dismiss")).toEqual([{}]);
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  );
  backend.on("new_dot_spec", () => blankSpec);
```

**with:**

```ts
  );
  // Personal is empty, and the vault has no one-time offer to make.
  backend.on("env_list", () => []);
  backend.on("env_import_candidates", () => []);
  backend.on("env_old_backups", () => []);
  backend.on("new_dot_spec", () => blankSpec);
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/library/LibraryScreen.test.tsx` in `ui`
Expected: the Env tab's 7 tests fail (no tab "Env"), and so does the server form's "names the variables that Personal does not have" test.

- [ ] **Step 3: Write the code**

**In `ui/src/lib/query.ts`, replace:**

```ts
  environmentVariables: (names: string[]) => ["environment-variables", names] as const,
  dots: ["dots"] as const,
```

**with:**

```ts
  environmentVariables: (names: string[]) => ["environment-variables", names] as const,
  env: ["env"] as const,
  envImport: ["env-import"] as const,
  envBackups: ["env-backups"] as const,
  dots: ["dots"] as const,
```

**Create `ui/src/screens/library/EnvTab.tsx`:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { EnvEntry } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
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
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ItemRow } from "./parts";

/// A variable name as `vault::entries::check_variable_name` takes it.
export const validVariableName = (name: string) => /^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(name);
const NAME_RULE = "Use a letter or _, then letters, digits or _ (at most 128).";

/// Add a variable (`entry` null), or edit a plain one's value or replace a secret's. A secret's
/// value is typed into a password field and never shown, before or after it is saved.
function VariableDialog(props: { entry: EnvEntry | null; onClose: () => void }) {
  const qc = useQueryClient();
  const { entry } = props;
  const [name, setName] = useState(entry?.name ?? "");
  const [secret, setSecret] = useState(entry ? entry.kind === "secret" : true);
  const [value, setValue] = useState(entry?.kind === "plain" ? (entry.value ?? "") : "");
  const [tried, setTried] = useState(false);
  const [state, setState] = useState<{ busy: boolean; error?: string }>({ busy: false });
  const errors = {
    name: entry || validVariableName(name) ? undefined : NAME_RULE,
    value: secret && value === "" ? "A secret needs a value." : undefined,
  };

  async function save() {
    setTried(true);
    if (errors.name || errors.value) return;
    setState({ busy: true });
    try {
      if (entry) await commands.envSet(entry.name, value);
      else await commands.envAdd(name, secret ? "secret" : "plain", value);
      invalidate(qc, keys.env);
      invalidate(qc, keys.library);
      props.onClose();
    } catch (e) {
      setState({ busy: false, error: errorMessage(e) });
    }
  }

  const title = !entry
    ? "Add variable"
    : entry.kind === "secret"
      ? `Replace ${entry.name}`
      : `Edit ${entry.name}`;
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {entry?.kind === "secret"
              ? "The new value takes the old one's place. Neither is shown."
              : "A dot can read it only where it is granted it."}
          </DialogDescription>
        </DialogHeader>
        {!entry && (
          <>
            <Field label="Name" htmlFor="env-name" error={tried ? errors.name : undefined}>
              <Input
                id="env-name"
                className="font-mono"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <div className="flex items-center gap-2">
              <Switch id="env-secret" checked={secret} onCheckedChange={setSecret} />
              <Label htmlFor="env-secret">Secret</Label>
              <span className="text-xs text-muted-foreground">
                {secret
                  ? "Kept in Windows Credential Manager and never shown again."
                  : "Plain config: shown here and editable."}
              </span>
            </div>
          </>
        )}
        <Field label="Value" htmlFor="env-value" error={tried ? errors.value : undefined}>
          <Input
            id="env-value"
            type={secret ? "password" : "text"}
            autoComplete="off"
            className="font-mono"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
        </Field>
        {state.error && <p className="text-sm text-destructive">{state.error}</p>}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button type="button" disabled={state.busy} onClick={() => void save()}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function RemoveVariable({ entry }: { entry: EnvEntry }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.envRemove(entry.name),
    onSuccess: () => {
      invalidate(qc, keys.env);
      invalidate(qc, keys.library);
    },
    onError: notifyError,
  });
  return (
    <ConfirmButton
      title={`Remove ${entry.name}?`}
      description="A dot that is granted it can no longer read it."
      confirmLabel="Remove variable"
      disabled={remove.isPending}
      onConfirm={() => remove.mutate()}
    >
      Remove
    </ConfirmButton>
  );
}

/// The one-time offer to copy the variables Library servers read from Dots' Windows
/// environment, which Dots no longer reads.
function ImportOffer({ names }: { names: string[] }) {
  const qc = useQueryClient();
  const answer = useMutation({
    mutationFn: (take: boolean) => (take ? commands.envImport(names) : commands.envImportDismiss()),
    onSuccess: () => {
      invalidate(qc, keys.env);
      invalidate(qc, keys.envImport);
      invalidate(qc, keys.library);
    },
    onError: notifyError,
  });
  return (
    <div role="region" aria-label="Import" className="flex flex-col gap-2 rounded-lg border p-3">
      <p className="text-sm">
        Dots no longer reads its own Windows environment. Library servers use these variables,
        which Windows sets: <span className="font-mono">{names.join(", ")}</span>. Import them
        into Personal as secrets?
      </p>
      <div className="flex gap-2">
        <Button type="button" size="sm" disabled={answer.isPending} onClick={() => answer.mutate(true)}>
          Import
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={answer.isPending}
          onClick={() => answer.mutate(false)}
        >
          Don't import
        </Button>
      </div>
    </div>
  );
}

/// The one-time offer to delete the update backups made before the vault: they may hold keys
/// in plain text. Dots never deletes them on its own.
function BackupsOffer({ files }: { files: string[] }) {
  const qc = useQueryClient();
  const answer = useMutation({
    mutationFn: (remove: boolean) => commands.envOldBackupsAnswer(remove),
    onSuccess: () => invalidate(qc, keys.envBackups),
    onError: notifyError,
  });
  return (
    <div role="region" aria-label="Old backups" className="flex flex-col gap-2 rounded-lg border p-3">
      <p className="text-sm">
        These update backups were made before the vault and may hold keys in plain text:{" "}
        <span className="font-mono">{files.join(", ")}</span>. Delete them?
      </p>
      <div className="flex gap-2">
        <Button
          type="button"
          size="sm"
          variant="destructive"
          disabled={answer.isPending}
          onClick={() => answer.mutate(true)}
        >
          Delete backups
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={answer.isPending}
          onClick={() => answer.mutate(false)}
        >
          Keep them
        </Button>
      </div>
    </div>
  );
}

/// The Library's Env tab (env vault spec §9): the personal variables a dot can be granted. A
/// secret's value is never shown.
export function EnvTab() {
  const env = useQuery({ queryKey: keys.env, queryFn: commands.envList });
  const imports = useQuery({ queryKey: keys.envImport, queryFn: commands.envImportCandidates });
  const backups = useQuery({ queryKey: keys.envBackups, queryFn: commands.envOldBackups });
  const [editing, setEditing] = useState<EnvEntry | null | undefined>(undefined);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-4">
        <p className="text-sm text-muted-foreground">
          Variables your dots can be granted. A server's <code>{"${VAR}"}</code> reads them. A
          secret is kept in Windows Credential Manager and never shown again.
        </p>
        <Button type="button" onClick={() => setEditing(null)}>
          <Plus aria-hidden /> Add variable
        </Button>
      </div>
      {imports.data && imports.data.length > 0 && <ImportOffer names={imports.data} />}
      {backups.data && backups.data.length > 0 && <BackupsOffer files={backups.data} />}
      <section aria-label="Personal" className="flex flex-col gap-2">
        <h2 className="text-sm font-medium">Personal</h2>
        {env.isError && <ErrorNote error={env.error} />}
        {env.data?.length === 0 && (
          <p className="text-sm text-muted-foreground">No variables yet.</p>
        )}
        <ul className="flex flex-col gap-2">
          {(env.data ?? []).map((entry) => (
            <ItemRow
              key={entry.name}
              label={entry.name}
              actions={
                <>
                  <Button type="button" variant="outline" size="sm" onClick={() => setEditing(entry)}>
                    {entry.kind === "secret" ? "Replace value" : "Edit"}
                  </Button>
                  <RemoveVariable entry={entry} />
                </>
              }
            >
              <div className="flex items-center gap-2">
                <span className="font-mono text-sm font-medium">{entry.name}</span>
                <Badge variant="secondary">{entry.kind}</Badge>
              </div>
              {entry.kind === "secret" ? (
                <span className="text-xs text-muted-foreground">Kept in Credential Manager</span>
              ) : (
                <span className="truncate font-mono text-xs text-muted-foreground">{entry.value}</span>
              )}
            </ItemRow>
          ))}
        </ul>
      </section>
      {editing !== undefined && (
        <VariableDialog entry={editing} onClose={() => setEditing(undefined)} />
      )}
    </div>
  );
}
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx
import { ContextTab } from "./ContextTab";
import { McpServersTab, serverStatus } from "./McpServersTab";
```

**with:**

```tsx
import { ContextTab } from "./ContextTab";
import { EnvTab } from "./EnvTab";
import { McpServersTab, serverStatus } from "./McpServersTab";
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx

/// The Library: the MCP servers, skills, plugins and context collections that dots can use. `?review=<server id>`
/// opens the review of that server's changed tool list (the Inbox links to it).
```

**with:**

```tsx

/// The Library: the MCP servers, skills, plugins, context collections and variables that dots
/// can use. `?review=<server id>`
/// opens the review of that server's changed tool list (the Inbox links to it).
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx
          <TabsTrigger value="context">Context</TabsTrigger>
        </TabsList>
```

**with:**

```tsx
          <TabsTrigger value="context">Context</TabsTrigger>
          <TabsTrigger value="env">Env</TabsTrigger>
        </TabsList>
```

**In `ui/src/screens/library/LibraryScreen.tsx`, replace:**

```tsx
        </TabsContent>
      </Tabs>
```

**with:**

```tsx
        </TabsContent>
        <TabsContent value="env">
          <EnvTab />
        </TabsContent>
      </Tabs>
```

**In `ui/src/screens/library/ServerForm.tsx`, replace:**

```tsx
            Write <code>{"${VAR}"}</code> where a value should come from a variable: {stdio
              ? "Dots reads the variable from Dots' environment when the server starts, and never stores its value."
              : "Dots reads the variable from Dots' environment and sends its value to the server. It asks first when the host is new, and never stores the value."}
          </p>
```

**with:**

```tsx
            Write <code>{"${VAR}"}</code> where a value should come from a variable: {stdio
              ? "Dots reads the variable from Personal (the Env tab) when the server starts, for the dots granted it."
              : "Dots reads the variable from Personal (the Env tab) and sends its value to the server, for the dots granted it. It asks first when the host is new."}
          </p>
```

**In `ui/src/screens/library/ServerForm.tsx`, replace:**

```tsx
            <p className="text-xs text-amber-300">
              Not set in Dots' environment: {notSet.join(", ")}
            </p>
```

**with:**

```tsx
            <p className="text-xs text-amber-300">
              Not in Personal: {notSet.join(", ")}. Add it in the Env tab.
            </p>
```

**In `ui/src/screens/library/ServerForm.tsx`, replace:**

```tsx
            <p key={key} className="text-xs text-amber-300">
              {key} looks like a secret. Write {"${VAR}"} instead of the value, so that the value
              is not stored.
            </p>
```

**with:**

```tsx
            <p key={key} className="text-xs text-amber-300">
              {key} looks like a secret. When you save, Dots moves its value to Personal as a
              secret and the server reads it from there.
            </p>
```

- [ ] **Step 4: Run the tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/library/LibraryScreen.test.tsx` in `ui`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
Expected: 79 passed; UI 459 in 31 files.

- [ ] **Step 5: Check and commit**

Run: `bun run --cwd ui typecheck`
Expected: clean.

```bash
git add ui/src/lib/query.ts ui/src/screens/library/EnvTab.tsx ui/src/screens/library/LibraryScreen.test.tsx ui/src/screens/library/LibraryScreen.tsx ui/src/screens/library/ServerForm.tsx ui/src/test/fixtures.ts
git commit -m "feat(ui): the Library's Env tab"
```

### Task 9: A dot's Variables

The dot editor's **Variables** section (spec §9), which shows, for each server the dot uses:

- the variables it is granted
- the ones a newly ticked server will be granted when the dot is saved
- the ones a server it already used asks for now, with **Grant**

It also lists the shell's grants, with the warning and a way to add and remove them.

**Files:**

- Create: `ui/src/screens/dot-editor/Variables.tsx`
- Modify: `ui/src/screens/dot-editor/DotForm.tsx`
- Test: `ui/src/screens/dot-editor/Variables.test.tsx` (new)

**Interfaces:**

- Consumes: Task 3's `DotSpec.variables`; Task 8's `validVariableName`, `keys.env`; `variablesIn` from `screens/library/text.ts`.
- Produces: `Variables({ uses, variables, before, onChange })`.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/screens/dot-editor/Variables.test.tsx`:**

```tsx
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { Uses } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { Variables } from "@/screens/dot-editor/Variables";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

type Grants = Record<string, string[]>;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [] };
const both: Uses = { ...none, mcp_servers: ["github"], plugins: ["pdf-tools"] };
const token = {
  name: "GITHUB_TOKEN",
  kind: "secret" as const,
  value: null,
  created_at: "2026-10-21T12:00:00.000Z",
  updated_at: "2026-10-21T12:00:00.000Z",
};

function setup(opts: { uses: Uses; before: Uses | null; variables?: Grants }) {
  backend.on("library_list", () => sample.library);
  backend.on("env_list", () => [token]);
  let latest = opts.variables ?? {};
  function Host() {
    const [variables, setVariables] = useState<Grants>(opts.variables ?? {});
    latest = variables;
    return (
      <Variables uses={opts.uses} variables={variables} before={opts.before} onChange={setVariables} />
    );
  }
  render(
    <QueryClientProvider client={createQueryClient()}>
      <Host />
    </QueryClientProvider>,
  );
  return { user: userEvent.setup(), variables: () => latest };
}

describe("the editor's Variables section", () => {
  it("says a newly ticked server is granted its variables when the dot is saved", async () => {
    setup({ uses: both, before: null });
    const github = await screen.findByRole("group", { name: "github" });
    expect(within(github).getByText("GITHUB_TOKEN · granted when you save")).toBeVisible();
    // A plugin's server is the plugin's: ticking the plugin uses it.
    const render = screen.getByRole("group", { name: "plugin-pdf-tools-render" });
    expect(within(render).getByText("EXAMPLE_TOKEN · granted when you save")).toBeVisible();
  });

  it("asks before it grants what a server already used asks for now", async () => {
    const { user, variables } = setup({
      uses: both,
      before: both,
      variables: { "mcp:github": ["GITHUB_TOKEN"] },
    });
    const github = await screen.findByRole("group", { name: "github" });
    expect(within(github).getByText("GITHUB_TOKEN")).toBeVisible();
    const render = screen.getByRole("group", { name: "plugin-pdf-tools-render" });
    expect(within(render).getByText(/now also wants/)).toHaveTextContent(
      "plugin-pdf-tools-render now also wants EXAMPLE_TOKEN",
    );
    await user.click(within(render).getByRole("button", { name: "Grant" }));
    expect(variables()).toEqual({
      "mcp:github": ["GITHUB_TOKEN"],
      "mcp:plugin-pdf-tools-render": ["EXAMPLE_TOKEN"],
    });
    expect(within(render).getByText(/not in Personal/)).toBeVisible();
  });

  it("grants a variable to the shell with a warning, and takes it back", async () => {
    const { user, variables } = setup({ uses: none, before: none });
    const shell = await screen.findByRole("group", { name: "Shell" });
    expect(within(shell).getByText(/could send them anywhere its tools reach/)).toBeVisible();
    await user.type(within(shell).getByLabelText("Variable for the shell"), "1BAD");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(within(shell).getByText(/Use a letter or _/)).toBeVisible();
    expect(variables()).toEqual({});
    await user.clear(within(shell).getByLabelText("Variable for the shell"));
    await user.type(within(shell).getByLabelText("Variable for the shell"), "GITHUB_TOKEN");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(variables()).toEqual({ shell: ["GITHUB_TOKEN"] });
    await user.click(
      within(shell).getByRole("button", { name: "Stop granting GITHUB_TOKEN to the shell" }),
    );
    expect(variables()).toEqual({ shell: [] });
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/dot-editor/Variables.test.tsx` in `ui`
Expected: the file fails to load: `@/screens/dot-editor/Variables` does not exist.

- [ ] **Step 3: Write the code**

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";
```

**with:**

```tsx
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { Variables } from "@/screens/dot-editor/Variables";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx

      <Section title="Limits">
```

**with:**

```tsx

      <Section title="Variables">
        <Variables
          uses={spec.uses}
          variables={spec.variables}
          before={props.existing?.uses ?? null}
          onChange={(variables) => set("variables", variables)}
        />
      </Section>

      <Section title="Limits">
```

**Create `ui/src/screens/dot-editor/Variables.tsx`:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { LibraryEntry, Uses } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { keys } from "@/lib/query";
import { validVariableName } from "@/screens/library/EnvTab";
import { variablesIn } from "@/screens/library/text";

type Grants = Record<string, string[]>;
type Server = Extract<LibraryEntry, { kind: "mcp_server" }>;

const SHELL = "shell";
const has = (names: string[], name: string) =>
  names.some((n) => n.toLowerCase() === name.toLowerCase());

/// The Library servers a dot with `uses` starts: those it ticks, and those of its plugins.
function usedServers(library: LibraryEntry[], uses: Uses | null): Server[] {
  if (!uses) return [];
  const plugins = new Set(
    library.filter((e) => e.kind === "plugin" && uses.plugins.includes(e.name)).map((e) => e.id),
  );
  return library.filter(
    (e): e is Server =>
      e.kind === "mcp_server" &&
      (e.owner_id === null ? uses.mcp_servers.includes(e.name) : plugins.has(e.owner_id)),
  );
}

/// The `${VAR}` names a server's config reads.
function wants(server: Server): string[] {
  const c = server.config;
  return c.transport === "stdio"
    ? variablesIn([...Object.values(c.env), ...c.args, c.cwd ?? ""])
    : variablesIn([c.url, ...Object.values(c.headers)]);
}

/// A dot's grants (env vault spec §7): what each server it uses may read, and what its shell
/// gets. A server the dot did not use before (`before` is the stored dot's `uses`) is granted
/// its variables when the dot is saved; a variable a server asks for later waits for Grant.
export function Variables(props: {
  uses: Uses;
  variables: Grants;
  before: Uses | null;
  onChange: (variables: Grants) => void;
}) {
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const env = useQuery({ queryKey: keys.env, queryFn: commands.envList });
  const [shellName, setShellName] = useState("");
  const [tried, setTried] = useState(false);
  if (library.isError) return <ErrorNote error={library.error} />;
  const entries = library.data ?? [];
  const personal = (env.data ?? []).map((e) => e.name);
  const had = usedServers(entries, props.before).map((s) => s.name);
  const shell = props.variables[SHELL] ?? [];
  const grant = (key: string, name: string) =>
    props.onChange({ ...props.variables, [key]: [...(props.variables[key] ?? []), name] });
  const missing = (name: string) =>
    env.data && !has(personal, name) ? " · not in Personal" : "";

  function addShell() {
    setTried(true);
    if (!validVariableName(shellName) || has(shell, shellName)) return;
    grant(SHELL, shellName);
    setShellName("");
    setTried(false);
  }

  return (
    <div className="flex flex-col gap-3">
      {usedServers(entries, props.uses).map((server) => {
        const key = `mcp:${server.name}`;
        const granted = props.variables[key] ?? [];
        const fresh = !had.includes(server.name);
        const names = wants(server);
        if (names.length === 0 && granted.length === 0) return null;
        return (
          <div key={server.id} role="group" aria-label={server.name} className="flex flex-col gap-1">
            <span className="font-mono text-sm font-medium">{server.name}</span>
            {[...new Set([...granted, ...names])].map((name) =>
              has(granted, name) ? (
                <span key={name} className="font-mono text-xs">
                  {name}
                  <span className="text-muted-foreground">{missing(name)}</span>
                </span>
              ) : fresh ? (
                <span key={name} className="font-mono text-xs text-muted-foreground">
                  {name} · granted when you save
                </span>
              ) : (
                <div key={name} className="flex items-center gap-2">
                  <span className="text-xs text-amber-700 dark:text-amber-400">
                    {server.name} now also wants <code>{name}</code>
                  </span>
                  <Button type="button" variant="outline" size="sm" onClick={() => grant(key, name)}>
                    Grant
                  </Button>
                </div>
              ),
            )}
          </div>
        );
      })}
      <div role="group" aria-label="Shell" className="flex flex-col gap-1">
        <span className="text-sm font-medium">Shell</span>
        <p className="text-xs text-muted-foreground">
          Set in the agent's own environment, where its shell commands run. The agent can read
          these values and could send them anywhere its tools reach.
        </p>
        {shell.map((name) => (
          <div key={name} className="flex items-center gap-2">
            <span className="font-mono text-xs">
              {name}
              <span className="text-muted-foreground">{missing(name)}</span>
            </span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={`Stop granting ${name} to the shell`}
              onClick={() =>
                props.onChange({ ...props.variables, [SHELL]: shell.filter((n) => n !== name) })
              }
            >
              Remove
            </Button>
          </div>
        ))}
        <div className="flex gap-2">
          <Input
            aria-label="Variable for the shell"
            className="w-64 font-mono"
            value={shellName}
            onChange={(e) => setShellName(e.target.value)}
          />
          <Button type="button" variant="outline" size="sm" onClick={addShell}>
            Grant to the shell
          </Button>
        </div>
        {tried && !validVariableName(shellName) && (
          <p className="text-xs text-destructive">
            Use a letter or _, then letters, digits or _ (at most 128).
          </p>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/dot-editor/Variables.test.tsx` in `ui`, then `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`
Expected: 3 passed; UI 462 in 32 files.

- [ ] **Step 5: Check and commit**

Run: `bun run --cwd ui typecheck`
Expected: clean.

```bash
git add ui/src/screens/dot-editor/DotForm.tsx ui/src/screens/dot-editor/Variables.test.tsx ui/src/screens/dot-editor/Variables.tsx
git commit -m "feat(ui): a dot's Variables: what its servers and shell are granted"
```

### Task 10: The spec records what plan A settled

This adds the spec's §14 (the split into plans A, B and C, and every deviation in the table above), and renumbers "Left to the builder" to §15.

- [ ] **Step 1: Write the change**

**In `docs/superpowers/specs/2026-10-21-env-vault-design.md`, replace:**

```markdown

## 14. Left to the builder

```

**with:**

```markdown

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
- **Redaction uses one redactor for the whole process.** It holds every secret Dots has read since it started, not one list per run. The store scrubs as it writes: run events (so the live view too), approval inputs, run payloads, summaries and errors, and the text of notes and questions. The log file is scrubbed line by line (§10).
- **An approval keeps the scrubbed input.** When Dots itself performs an approved call (the local engine after a late decision), it performs it with `[secret:NAME]` where a secret value was. The input hash is still the one of the call as asked (§10).
- **Secret-looking values move to Personal automatically**, with no "held" state and no button:
  - when a server is added or changed
  - each time a plugin's servers are synced from its `.mcp.json`
  - at every start, for values a version before the vault stored; the dots that use such a server are granted the names

  A name Personal has with the same value is reused; another value gets `_2`, `_3`, … A value over 2,560 bytes stays where it is and is logged. The server form says this happens instead of offering "Move to vault" (§9, §11).
- **The import offer** lists the `${VAR}` names that Library servers read, that Dots' Windows environment sets, and that Personal lacks. They are imported as secrets. Importing or dismissing answers the offer for good: the `settings` row `vault_import_answered` (§11 step 3).
- **"Made before this release"** means older than the `settings` row `vault_since`, written the first time the data folder runs with the vault. Deleting or keeping the backups answers the offer for good: `vault_backups_answered` (§11 step 4).
- **A new dot has no webhook token.** Its webhook refuses every request until **Make a token**. A token is stored as `sha256:<hex>` and shown once; tokens from before are hashed when the store opens (§11).
- **The gateway file's DACL** is set before its secret is written: one entry, full access for the signed-in user, nothing inherited (§11).
- **Not in plan A:**
  - the Env tab's "used by" (§9)
  - org sections, the org picker and the Organisations page (plan B)
  - audit records (plan C)

## 15. Left to the builder

```

- [ ] **Step 2: Check and commit**

Run: `iconv -f UTF-8 -t UTF-8 docs/superpowers/specs/2026-10-21-env-vault-design.md > /dev/null`, `git diff --check`
Expected: no output.

```bash
git add docs/superpowers/specs/2026-10-21-env-vault-design.md
git commit -m "docs(spec): record what the env vault plan A settled"
```

### Task 11: The smoke

Use a debug build of the app, the fake CLI and a fresh data folder: `DOTS_DATA_DIR` set to a new temp folder. Use the webview driver from `tools/smoke/webview.mjs` (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`). The debug build writes to Credential Manager under the fresh folder's install id. Afterwards, delete those entries with `cmdkey /list:Dots/<install_id>*` and `cmdkey /delete:<target>`, by their target only.

- [ ] **Step 1: Part A (agent)**
  1. Start the app and open Library → Env. Add `DOTS_SMOKE_TOKEN` as a secret and `LOG_LEVEL` as plain. The secret's row shows no value, and `dots.db` holds no value for it: search the file for the value with `grep -c`.
  2. Add an MCP server (`fake-mcp`) whose env reads `${DOTS_SMOKE_TOKEN}`, and a dot that ticks it. The dot's Variables section shows the variable "granted when you save". Save, run the dot, and confirm the server started with the value (the fake server's log).
  3. Change the server to read a second variable, `${DOTS_SMOKE_ORG}`. Run again: the run's events say "DOTS_SMOKE_ORG: not granted to mcp:… in dot '…'". Click **Grant**, save, add the variable in Env, and run again: the server starts.
  4. Make the fake server print the token in a tool result and on stderr. The run's events and the log file hold `[secret:DOTS_SMOKE_TOKEN]`, never the value.
  5. Set `DOTS_SMOKE_LEAK` in the environment the app starts with. A shell grant of `LOG_LEVEL` reaches the fake CLI (`FAKE_CLAUDE_ARGS_OUT` lists it), and `DOTS_SMOKE_LEAK` does not.
  6. In the dot's Webhook section, click **Make a token**. It is shown once. A `curl -H "Authorization: Bearer <token>"` gives 202; `?token=` gives 401.
  7. Write down each step's outcome in `docs/verification/2026-10-21-env-vault-smoke.md`. Commit it (`docs(verification): the env vault smoke`).
- [ ] **Step 2: Part B (the owner)**
  1. With the real `claude` CLI, logged in, run a dot. The CLI still runs with the base environment (the subscription login works), and a shell grant is visible to its Bash tool (`echo $LOG_LEVEL`).
  2. On a copy of the real data folder (`DOTS_DATA_DIR` pointing at it) whose MCP servers have typed tokens: start Dots. The values move to Personal, the dots that use those servers still run, and the Env tab offers to import from Windows and to delete the backups made before the vault.
  3. Record the outcome in the same smoke file.
