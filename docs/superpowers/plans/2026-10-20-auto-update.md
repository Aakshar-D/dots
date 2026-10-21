# Updates for Installed Dots Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A version tag on GitHub builds, signs and publishes a release, and every installed Dots finds it, downloads it in the background and offers it in a strip at the top of its window, installing it only when the person says so, after backing up its database.

**Architecture:** The core gains a consistent database backup (`VACUUM INTO`), a hold on the run queue, the names of the dots with a running run, and a `close` that waits for the database. The app gains `update.rs` (the strip's state, the backup files, the order of an install, all without the network) and `update_service.rs`, which wires them to `tauri-plugin-updater`: a check 20 seconds after start and every 6 hours, five commands for the strip's buttons and an `update` event. The UI shows the strip in `Banners` and the newest backup on the fatal-error page. A GitHub Actions workflow and `tools/release/release.mjs` check the tag against the version fields, build and sign the installer and publish the release with its updater manifest.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio, Tauri 2.12 with `tauri-plugin-updater` 2.14 (new) and `chrono` 0.4 (new to the app; the core has it); Bun 1.3, React 19.3, TypeScript 6.0, TanStack Query 5.104, zod 4.6, Vitest 5.0; Node 24 (`node:test`) for the release script; GitHub Actions on `windows-latest`.

**Spec:** `docs/superpowers/specs/2026-10-20-auto-update-design.md` (commit `ffb6d8c`, the base of this plan, branch `feat/auto-update`). Task 7 brings it in line with what the tasks build; the table "Deviations" lists what changed and why.

**Previous plans:** `docs/superpowers/plans/2026-10-08-dots-core.md` (Plan 1) to `2026-10-19-context-library.md`. This plan touches the restart of Plan 3c (`restart_app`, the restart mark, the `Banners` strip and its `RestartButton`).

**Where the tasks come from:** the spec. Tasks 1 and 2 are the core pieces (the backup; the hold, the working dots and `close`); Task 3 is the app's update logic without the plugin; Task 4 wires it to the plugin, the commands, the event and the contract; Task 5 is the strip and the error page; Task 6 is the release workflow, its script and `docs/releasing.md`; Task 7 is the spec; Task 8 is the smoke.

**How this plan's code was checked:** the code blocks of Tasks 1 to 7 were generated from a throwaway worktree of commit `ffb6d8c`, one commit per task, where they compiled and passed on Windows 11, and the plan was then replayed onto `ffb6d8c` by a script that applied every block and compared the tree with each task commit (all matched; `Cargo.lock` and the contract fixture were taken from the task commit, as cargo and the contract command write them). Before the first task: `cargo test -p dots-core` 661 passed, `cargo test -p dots-app` 66, `bun run --cwd ui test` 399 tests in 25 files. At the last task: `dots-core` 666, `dots-app` 81, the UI 406 in 25 files, `node --test tools/release/release.test.mjs` 5; clippy without warnings, `cargo fmt --check` and `tsc --noEmit` clean. The prototype also checked, outside the plan: the NSIS script of tauri-bundler 2.12 in update mode (`/UPDATE`) deletes only the files it installed and runs a non-recursive `RMDir`, so the data folder, which is the install folder, keeps `dots.db`, `backups`, `worktrees`, `plugins`, `mcp` and `logs` (spec §7); `tauri-plugin-updater` 2.14 checks the signature in `download` and again in `install`. A debug build of the last task started with the plugin under a fresh `DOTS_DATA_DIR` and logged `update checks are off in this build or under DOTS_DATA_DIR`.

## Before you start (the owner)

The public key of the owner's signing key goes into `apps/dots-app/tauri.conf.json` in Task 4. The owner makes the key pair before Task 4 (`docs/releasing.md`, "The signing key", which Task 6 writes; the command is `bun run tauri signer generate -w ~/.tauri/dots-updates.key`) and gives the controller the text of `dots-updates.key.pub`, which is public. The prototype used a throwaway key; its public key is the `pubkey` value in Task 4's block, and Task 4 replaces it with the owner's. **No agent makes, reads or stores the owner's private key.** If the owner's public key is not there when Task 4 starts, Task 4 keeps the throwaway key and the controller lists "replace the updater public key" as an open item in its final message; the first release must not be tagged until it is replaced.

## Global Constraints

- Toolchain: Rust 1.95 stable; Node 24; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/auto-update`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\auto-update`, based on `ffb6d8c`. All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Every cargo command runs with `CARGO_TARGET_DIR=C:/Users/aksha/dots/target` (the shared build folder): `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target` once per shell.
- New dependencies: `tauri-plugin-updater = "2.14"` (Task 4) and `chrono = "0.4"` (Task 3), both in `apps/dots-app/Cargo.toml`. No other Cargo or npm dependency, no migration, no new capability: the webview gets no updater permission and opens no address.
- Values the spec fixes: the endpoint `https://github.com/Aakshar-D/dots/releases/latest/download/latest.json`; the variable `DOTS_UPDATE_ENDPOINT`; the first check 20 seconds after start, then every 6 hours; no check in a debug build or when `DOTS_DATA_DIR` is set (not empty); backups in `<data dir>\backups` named `dots-<old version>-<YYYY-MM-DD-HHMMSS>.db`, the 3 newest kept; at most 10 seconds for the database to close; the NSIS installer in passive mode; tags `v*`, plain `vMAJOR.MINOR.PATCH` a release, any suffix a prerelease; the secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- The version lives in five places, which the release check compares with the tag: `apps/dots-app/tauri.conf.json`, `apps/dots-app/Cargo.toml`, `crates/dots-core/Cargo.toml`, `ui/package.json`, and `Cargo.lock` (`dots-app`, `dots-core`). This plan changes none of them (all stay `0.1.0`).
- No agent installs, uninstalls or runs an installer, runs a release build of Dots, pushes a tag, or touches the installed Dots (`%LOCALAPPDATA%\Dots`) or the real data folder `%LOCALAPPDATA%\dots`. Smokes use a debug build with a fresh `DOTS_DATA_DIR`, where updates are off.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit (the blocks hold `§`, `—` and `…`). The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`, plus `open` from `@tauri-apps/plugin-dialog`. No `fetch`, no other plugin.
- Live data is parsed, not cast. A type the UI parses changes only together with the contract fixture: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then the same test without the variable, then `ui/src/api/types.ts`. The fixture `ui/src/api/__fixtures__/contract.json` is written by that command and never by hand. `contract.rs`'s `samples()` is at the `json!` macro's recursion limit: a new sample is added after the macro (`samples["X"] = ...`).
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched (`--no-fail-fast` for `dots-core`), `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. Test output must be free of warnings. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first. The UI files have no formatter; keep lines within 100 columns, as the blocks do.
- Known on this machine: a `vitest` run started right after an install, a `tsc` run, a cargo build or a large write can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`, or take over a minute. Run it again on its own; do not change the pool settings. Run the UI tests before the typecheck.
- Known flaky under load: `runtime_local_test::live_approval_lets_the_command_run`, `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`, `claude_engine_test::a_finished_run_leaves_no_process_behind`, some `resume_test` tests that make worktrees, and `claude_gateway_test::a_claude_run_reaches_its_servers_through_the_gateway_only` (it checks that a run's tools are gone right after the run's status is written; it failed once in the prototype's full run and passed three times alone; this plan does not touch that path). Rerun the test binary alone before treating one as a failure. Task 2's hold test waits 2.5 seconds against a 3-second run; on a machine so loaded that the first run ends early, rerun it alone.
- Fact-Forcing Gate (a hook of this machine): the first Edit or Write of each file, the first Bash command after a task notification, and `rm`, `git commit --amend` or a rebase are blocked until the facts it asks for are stated (callers, affected API, data shapes, the instruction, or for a destructive command what changes and how to undo it); then retry the identical call. For a large new file, write a one-line stub first.

## Deviations from the spec

The spec at `ffb6d8c` says something else in these places; Task 7 changes the spec to match.

| Spec at `ffb6d8c` | This plan | Why |
|---|---|---|
| §6: a "What's new" link to the release page | A "What's new" button; the command `update_open_notes` opens the page of the version that is ready | The webview opens no address of its own (no opener permission); the core builds the address from the version |
| §6: "Restart when it's done", "Stop it and restart", a **Cancel** link | The same, with "when they're done" and "Stop them" for more than one dot; the waiting note says "Will restart into Dots 0.2.0 when Daily digest finishes. New runs wait until after the update." with a Cancel button | Plural text; the spec asks that a person who starts a run while waiting learns it is queued until after the update |
| §7: "after 'Stop it and restart' and the stopped runs have ended" | "Stop it and restart" begins the install at once, as "Restart Dots" does; the working runs are marked interrupted at the next start | Cancelling them first would end them as cancelled, not as failed ("interrupted") as the dialog says |
| §7 step 1: a time limit the builder picks | 10 seconds (`update::CLOSE_LIMIT`); a database that has not closed in time is not backed up, and Dots restarts into the current version with the reason | A backup taken while a task still writes could be incomplete |
| §4: the builder confirms that NSIS accepts a prerelease version | Confirmed (tauri-bundler 2.12 writes `0.2.0.0` for Windows' four numbers); the example tag is `v0.2.0-rc.1` | Checked in the bundler's source |

The spec leaves to the builder, and the plan settles: the workflow uses its own steps with `gh release create`, not `tauri-apps/tauri-action`; the release check and the manifest are one tested script, `tools/release/release.mjs`; the updater files (`.sig`) are made only in the workflow (`--config '{"bundle":{"createUpdaterArtifacts":true}}'`), so a build on a PC needs no key; the reason a failed install gives is carried to the next start in `update-error.txt` in the data folder.

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. A dot still running when the person clicks the strip. Expected: no install while it runs; the queue is held so a busy schedule cannot keep Dots from restarting; the install begins when the last run ends; Cancel releases the queue. Tests: `runtime_test::a_held_queue_starts_no_run_and_working_dots_name_the_running_ones` (Task 2), `update::tests::restart_installs_at_once_when_nothing_works_and_waits_otherwise` and `a_wait_ends_with_the_last_run_or_with_cancel` (Task 3), `Layout.test.tsx` "waits for the dots that are working, and Cancel stops waiting" (Task 5).
2. A backup that cannot be made (disk full, a database that does not close, a missing file). Expected: nothing is installed, Dots restarts on the version it has and says why. Tests: `update::tests::a_failed_step_restarts_on_this_version_with_the_reason` and `a_backup_without_a_database_fails_and_there_is_no_newest` (Task 3), `store_backup_test::a_backup_never_overwrites_a_file` (Task 1).
3. Backups of several versions over time, and other files in the folder. Expected: the 3 newest by time are kept, whatever the version numbers (`0.10.0` after `0.9.0`), and nothing else is deleted. Test: `update::tests::a_backup_keeps_the_three_newest_and_nothing_else_is_touched` (Task 3).
4. A development build or a smoke run. Expected: it never checks, so it never replaces the installed Dots. Test: `update::tests::checks_are_off_in_debug_builds_and_under_dots_data_dir` (Task 3).
5. A tag that disagrees with the version fields, including `Cargo.lock` on a Windows checkout with CRLF. Expected: the workflow fails before it builds, naming each place. Test: `release.test.mjs` "every field that disagrees with the tag is named" (Task 6).

## File Map

- `crates/dots-core/src/store/mod.rs` (Task 1): `backup_database`.
- `crates/dots-core/src/runner.rs`, `runtime.rs`, `store/runs.rs` (Task 2): the hold on the queue, the working dots, `close`.
- `apps/dots-app/src/update.rs` (new, Task 3): the strip's state, the checks' rules, backups, the error mark, the order of an install.
- `apps/dots-app/src/update_service.rs` (new, Task 4): the plugin, the checks, the watch on runs, the install's steps.
- `apps/dots-app/src/{commands,events,lib,state,contract}.rs`, `apps/dots-app/tauri.conf.json`, `apps/dots-app/Cargo.toml` (Tasks 3 and 4).
- `ui/src/api/{types,commands,events}.ts` (Task 4); `ui/src/app/{Banners,FatalPage,Root}.tsx`, `ui/src/lib/{query,runtime-events}.ts`, `ui/src/test/fixtures.ts` (Task 5).
- `.github/workflows/release.yml`, `tools/release/release.mjs` (new, Task 6); `docs/releasing.md` (new, Task 6).
- Tests: `crates/dots-core/tests/store_backup_test.rs` (new), `runtime_test.rs`; the unit tests in `update.rs` and `state.rs`; `ui/src/api/{contract,events}.test.ts`, `ui/src/app/Layout.test.tsx`; `tools/release/release.test.mjs` (new).
- `docs/superpowers/specs/2026-10-20-auto-update-design.md` (Task 7).

### Task 1: Back up the database with VACUUM INTO

A consistent copy of `dots.db` through a connection of its own, so it works after the store has closed. The database is in WAL mode: a copy of the file alone could miss what the WAL holds.

**Files:**

- Modify: `crates/dots-core/src/store/mod.rs`
- Create: `crates/dots-core/tests/store_backup_test.rs`

**Interfaces:**

- Consumes: nothing new.
- Produces: `dots_core::store::backup_database(db: &Path, dest: &Path) -> Result<()>`: fails when `db` does not exist or `dest` does, and creates neither.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/store_backup_test.rs`:**

```rust
mod common;

use common::{spec, temp_store};
use dots_core::store::{backup_database, Store};

#[tokio::test]
async fn a_backup_holds_what_was_written_also_after_the_store_closed() {
    let (dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("digest")).await.unwrap();
    store.close().await;
    let dest = dir.path().join("backups").join("copy.db");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    backup_database(&dir.path().join("dots.db"), &dest)
        .await
        .unwrap();
    let copy = Store::open(&dest).await.unwrap();
    let dots = copy.list_dots().await.unwrap();
    assert_eq!(dots.len(), 1);
    assert_eq!(dots[0].id, dot.id);
}

#[tokio::test]
async fn a_backup_never_overwrites_a_file() {
    let (dir, store) = temp_store().await;
    store.close().await;
    let dest = dir.path().join("copy.db");
    std::fs::write(&dest, b"keep me").unwrap();
    assert!(backup_database(&dir.path().join("dots.db"), &dest)
        .await
        .is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
}

#[tokio::test]
async fn a_missing_database_is_an_error_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("dots.db");
    let dest = dir.path().join("copy.db");
    assert!(backup_database(&db, &dest).await.is_err());
    assert!(!db.exists());
    assert!(!dest.exists());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test store_backup_test`
Expected: FAIL to compile: no `backup_database` in `dots_core::store`.

- [ ] **Step 3: Add `backup_database`**

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

```

**with:**

```rust
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection};

```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust

pub(crate) fn opt_json(s: Option<String>) -> Result<Option<Value>> {
```

**with:**

```rust

/// Writes a consistent copy of the database `db` to `dest` with `VACUUM INTO`, through a
/// connection of its own, so it works after the store has closed (updates spec §7). The
/// database is in WAL mode: a copy of the file alone could miss what the WAL still holds.
/// Fails when `db` does not exist or `dest` does, and creates neither.
pub async fn backup_database(db: &Path, dest: &Path) -> Result<()> {
    let opts = SqliteConnectOptions::new()
        .filename(db)
        .busy_timeout(Duration::from_secs(5));
    let mut conn = opts.connect().await?;
    let done = sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().into_owned())
        .execute(&mut conn)
        .await;
    conn.close().await?;
    done?;
    Ok(())
}

pub(crate) fn opt_json(s: Option<String>) -> Result<Option<Value>> {
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test store_backup_test`
Expected: PASS: 3 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 664 passed; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/store/mod.rs crates/dots-core/tests/store_backup_test.rs
git commit -m "feat(core): back up the database with VACUUM INTO"
```

### Task 2: Hold the run queue, name the working dots, close the runtime

While an update waits for the runs that are working, no queued run starts, whatever its trigger; new runs are still queued. The dots that block an update are those with a run whose status is `running` (spec §6): queued runs and runs awaiting an approval survive a restart. `close` stops the runtime and waits, with a limit, until the database has closed.

**Files:**

- Modify: `crates/dots-core/src/runner.rs`
- Modify: `crates/dots-core/src/runtime.rs`
- Modify: `crates/dots-core/src/store/runs.rs`
- Modify: `crates/dots-core/tests/runtime_test.rs`

**Interfaces:**

- Consumes: `backup_database` (Task 1), in the test.
- Produces: `Runner::set_held(&self, held: bool)`; `Store::running_dot_ids(&self) -> Result<Vec<String>>`; `Runtime::hold_runs(&self, held: bool)`, `Runtime::working_dots(&self) -> Result<Vec<String>>` (dot names, sorted), `Runtime::close(&self, limit: Duration) -> bool` (true when the database closed in time; afterwards every database call fails).

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/runtime_test.rs`, replace:**

```rust
    panic!("no resume run");
}
```

**with:**

```rust
    panic!("no resume run");
}

#[tokio::test]
async fn a_held_queue_starts_no_run_and_working_dots_name_the_running_ones() {
    let t = start(Some(json!([{"sleep_ms": 3000}])), None).await;
    let busy = t.rt.create_dot(folder_spec(&t.dir, "busy")).await.unwrap();
    let idle = t.rt.create_dot(folder_spec(&t.dir, "idle")).await.unwrap();
    assert!(t.rt.working_dots().await.unwrap().is_empty());
    let first = t.rt.run_now(&busy.id).await.unwrap();
    wait_status(t.rt.store(), &first.id, RunStatus::Running, 15).await;
    t.rt.hold_runs(true);
    // A run asked for while the queue is held is queued, and stays queued.
    let second = t.rt.run_now(&idle.id).await.unwrap();
    assert_eq!(t.rt.working_dots().await.unwrap(), vec!["busy".to_string()]);
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(
        t.rt.get_run(&second.id).await.unwrap().status,
        RunStatus::Queued
    );
    t.rt.hold_runs(false);
    wait_status(t.rt.store(), &second.id, RunStatus::Running, 15).await;
    for run in [&first, &second] {
        t.rt.cancel_run(&run.id).await.unwrap();
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn close_ends_the_runtime_and_leaves_a_database_to_back_up() {
    let t = start(None, None).await;
    let dot = t.rt.create_dot(folder_spec(&t.dir, "kept")).await.unwrap();
    assert!(t.rt.close(std::time::Duration::from_secs(5)).await);
    assert!(t.rt.list_dots().await.is_err());
    let db = t.dir.path().join("data").join("dots.db");
    let dest = t.dir.path().join("copy.db");
    dots_core::store::backup_database(&db, &dest).await.unwrap();
    let copy = Store::open(&dest).await.unwrap();
    assert_eq!(copy.list_dots().await.unwrap()[0].id, dot.id);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test runtime_test`
Expected: FAIL to compile: no `working_dots`, `hold_runs` or `close` on `Runtime`.

- [ ] **Step 3: Add the hold, the working dots and `close`**

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    resume_lock: tokio::sync::Mutex<()>,
}
```

**with:**

```rust
    resume_lock: tokio::sync::Mutex<()>,
    /// While set, the dispatcher starts no queued run (`set_held`).
    held: AtomicBool,
}
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            resume_lock: tokio::sync::Mutex::new(()),
        })
```

**with:**

```rust
            resume_lock: tokio::sync::Mutex::new(()),
            held: AtomicBool::new(false),
        })
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust

    pub async fn dispatch_once(self: &Arc<Self>) -> Result<usize> {
        let mut started = 0;
```

**with:**

```rust

    /// Holds or releases the queue. While held, no queued run starts, whatever its trigger;
    /// new runs are still queued and start once the queue is released (updates spec §6).
    pub fn set_held(&self, held: bool) {
        self.held.store(held, Ordering::SeqCst);
        if !held {
            self.wake.notify_one();
        }
    }

    pub async fn dispatch_once(self: &Arc<Self>) -> Result<usize> {
        if self.held.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let mut started = 0;
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        self.shutdown.cancel();
    }
}
```

**with:**

```rust
        self.shutdown.cancel();
    }

    /// Holds or releases the run queue (`Runner::set_held`): while an update waits for the
    /// runs that are working, no queued run starts (updates spec §6).
    pub fn hold_runs(&self, held: bool) {
        self.runner.set_held(held);
    }

    /// The names of the dots with a run whose status is `running`, sorted: the runs an update
    /// waits for (updates spec §6). Queued runs and runs awaiting an approval survive a
    /// restart, so they are left out. A run is no longer counted once the `RunUpdated` event
    /// of its end is out.
    pub async fn working_dots(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for id in self.store.running_dot_ids().await? {
            names.push(self.store.get_dot(&id).await?.spec.name);
        }
        names.sort();
        Ok(names)
    }

    /// Stops the runtime like `shutdown`, then closes the database and waits until every
    /// connection has closed, at most `limit`. True when it closed in time. From then on every
    /// call that reads or writes the database fails (updates spec §7).
    pub async fn close(&self, limit: Duration) -> bool {
        self.shutdown();
        tokio::time::timeout(limit, self.store.close())
            .await
            .is_ok()
    }
}
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust

    pub async fn has_queued_run(&self, dot_id: &str) -> Result<bool> {
```

**with:**

```rust

    /// The dots with a run whose status is `running`, each once, by id.
    pub async fn running_dot_ids(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT dot_id FROM runs WHERE status = 'running' ORDER BY dot_id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn has_queued_run(&self, dot_id: &str) -> Result<bool> {
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test runtime_test -- held_queue close_ends`
Expected: PASS: 2 tests.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-core` 666 passed, `dots-app` 66; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/runner.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/runs.rs crates/dots-core/tests/runtime_test.rs
git commit -m "feat(core): hold the run queue, name working dots, close the runtime"
```

### Task 3: The update note, backups and the order of an install

`update.rs` holds everything about updates that needs no network and no plugin: what the strip shows (`UpdateNote`) and what its buttons do (`UpdateState`), whether this Dots checks and where, the backup's file name, the backup with its pruning, the reason a failed install leaves for the next start, and the order an install goes in, behind the `InstallSteps` trait so it is tested without installing anything. Its tests are in the module.

**Files:**

- Modify: `apps/dots-app/Cargo.toml`
- Modify: `apps/dots-app/src/lib.rs`
- Create: `apps/dots-app/src/update.rs`
- Regenerate: `Cargo.lock` (cargo adds `chrono` to `dots-app`'s dependencies)

**Interfaces:**

- Consumes: `dots_core::store::{backup_database, DB_FILE, Store}` (Task 1).
- Produces, in `crate::update`: `UpdateNote` (`#[serde(tag = "state", rename_all = "snake_case")]`: `None`, `Ready { version, working: Vec<String>, error: Option<String> }`, `Waiting { version, waiting_for: Vec<String> }`, `Installing { version }`); `Next { Nothing, Install }`; `UpdateState` with `new(error: Option<String>)`, `wants(&self, version: &str) -> bool`, `offer(&mut self, version: &str) -> bool`, `note(&self, working: &[String]) -> UpdateNote`, `ready_version(&self) -> Option<&str>`, `holds_runs(&self) -> bool`, `later(&mut self)`, `restart(&mut self, working: &[String]) -> dots_core::Result<Next>`, `stop_and_restart(&mut self) -> dots_core::Result<Next>`, `runs_changed(&mut self, working: &[String]) -> Next`, `cancel_wait(&mut self)`; `checks_enabled(debug: bool, data_dir_override: Option<OsString>) -> bool`; `endpoint(env_override: Option<OsString>) -> Result<Url, String>`; `release_page(version: &str) -> String`; `backup_file_name(version: &str, at: DateTime<Local>) -> String`; `newest_backup(data_dir: &Path) -> Option<PathBuf>`; `back_up(data_dir: &Path, version: &str, at: DateTime<Local>) -> Result<PathBuf, String>` (async); `leave_error(data_dir: &Path, error: &str)`, `take_error(data_dir: &Path) -> Option<String>`; `trait InstallSteps` (`close_runtime`, `back_up`, `restart_current`, `mark_restart`, `install`) and `install(steps: &impl InstallSteps)` (async); constants `DEFAULT_ENDPOINT`, `ENDPOINT_ENV`, `FIRST_CHECK_AFTER`, `CHECK_EVERY`, `CLOSE_LIMIT`, `BACKUPS_DIR`, `KEEP_BACKUPS`, `ERROR_MARK`.

- [ ] **Step 1: Add the module with its tests**

The tests live in the module (`#[cfg(test)] mod tests`), so this step writes both.

**In `apps/dots-app/Cargo.toml`, replace:**

```toml
dots-core = { path = "../../crates/dots-core" }
serde = { version = "1", features = ["derive"] }
```

**with:**

```toml
dots-core = { path = "../../crates/dots-core" }
chrono = "0.4"
serde = { version = "1", features = ["derive"] }
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
pub mod tray;

```

**with:**

```rust
pub mod tray;
pub mod update;

```

**Create `apps/dots-app/src/update.rs`:**

```rust
//! Updates for installed Dots (updates spec): what the update strip shows and what its
//! buttons do, when Dots checks, the backup taken before an install, and the order an install
//! goes in. Free of the updater plugin, so all of it is tested without the network; the app
//! wires it to `tauri-plugin-updater` (`update_service.rs`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use tauri::Url;

/// Where every installed Dots looks for a new version (spec §4).
pub const DEFAULT_ENDPOINT: &str =
    "https://github.com/Aakshar-D/dots/releases/latest/download/latest.json";
/// Points a release build at another manifest, such as a prerelease's (spec §4).
pub const ENDPOINT_ENV: &str = "DOTS_UPDATE_ENDPOINT";
/// The first check after a start, then one every `CHECK_EVERY` (spec §5).
pub const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How long an install waits for the database to close (spec §7).
pub const CLOSE_LIMIT: Duration = Duration::from_secs(10);
/// The backups folder inside the data folder, and how many backups it keeps (spec §7).
pub const BACKUPS_DIR: &str = "backups";
pub const KEEP_BACKUPS: usize = 3;
/// Why the last install did not happen, left for the next start (spec §7).
pub const ERROR_MARK: &str = "update-error.txt";

/// What the update strip shows (spec §6). The UI gets it from `update_note` and the
/// `update` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateNote {
    /// No update is ready, or the person chose "Later".
    None,
    /// `working`: the dots with a run working now. `error`: why the last install did not
    /// happen, when it did not.
    Ready {
        version: String,
        working: Vec<String>,
        error: Option<String>,
    },
    /// "Restart when it's done": Dots installs once no dot has a run working.
    Waiting {
        version: String,
        waiting_for: Vec<String>,
    },
    /// The install has begun; Dots is about to close.
    Installing { version: String },
}

/// What the app does after a change of `UpdateState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Nothing,
    Install,
}

/// The state behind the strip. `later` lasts until the process ends, so the note is back at
/// the next start.
#[derive(Debug, Default)]
pub struct UpdateState {
    ready: Option<String>,
    later: bool,
    waiting: bool,
    installing: bool,
    error: Option<String>,
}

impl UpdateState {
    /// `error`: what the last start left (`take_error`).
    pub fn new(error: Option<String>) -> Self {
        Self {
            error,
            ..Self::default()
        }
    }

    /// Whether a check that found `version` should download it: not while an install waits or
    /// has begun (the person agreed to the version they saw), and not a second time.
    pub fn wants(&self, version: &str) -> bool {
        !self.waiting && !self.installing && self.ready.as_deref() != Some(version)
    }

    /// A downloaded update, checked against Dots' key. True when the note changed.
    pub fn offer(&mut self, version: &str) -> bool {
        if !self.wants(version) {
            return false;
        }
        self.ready = Some(version.to_string());
        !self.later
    }

    pub fn note(&self, working: &[String]) -> UpdateNote {
        let Some(version) = self.ready.clone() else {
            return UpdateNote::None;
        };
        if self.installing {
            UpdateNote::Installing { version }
        } else if self.waiting {
            UpdateNote::Waiting {
                version,
                waiting_for: working.to_vec(),
            }
        } else if self.later {
            UpdateNote::None
        } else {
            UpdateNote::Ready {
                version,
                working: working.to_vec(),
                error: self.error.clone(),
            }
        }
    }

    /// The version that is ready, whether or not the note shows it.
    pub fn ready_version(&self) -> Option<&str> {
        self.ready.as_deref()
    }

    /// Whether the run queue is held: while an install waits (spec §6).
    pub fn holds_runs(&self) -> bool {
        self.waiting
    }

    pub fn later(&mut self) {
        self.later = true;
    }

    /// "Restart to update" and "Restart when it's done": installs at once when no dot has a
    /// run working, else waits for those runs.
    pub fn restart(&mut self, working: &[String]) -> dots_core::Result<Next> {
        self.check_ready()?;
        if working.is_empty() {
            return Ok(self.begin());
        }
        self.waiting = true;
        Ok(Next::Nothing)
    }

    /// "Stop it and restart": installs whatever is working. Those runs end as failed
    /// ("interrupted") at the next start, as after "Restart Dots".
    pub fn stop_and_restart(&mut self) -> dots_core::Result<Next> {
        self.check_ready()?;
        Ok(self.begin())
    }

    /// The working runs changed: while waiting, the install begins once none is left.
    pub fn runs_changed(&mut self, working: &[String]) -> Next {
        if self.waiting && !self.installing && working.is_empty() {
            self.begin()
        } else {
            Next::Nothing
        }
    }

    /// "Cancel": stop waiting, and let the queue start again.
    pub fn cancel_wait(&mut self) {
        self.waiting = false;
    }

    fn check_ready(&self) -> dots_core::Result<()> {
        match (&self.ready, self.installing) {
            (None, _) => Err(dots_core::Error::Conflict("no update is ready".into())),
            (Some(_), true) => Err(dots_core::Error::Conflict(
                "the update is already being installed".into(),
            )),
            (Some(_), false) => Ok(()),
        }
    }

    fn begin(&mut self) -> Next {
        self.installing = true;
        Next::Install
    }
}

/// Whether this Dots checks for updates: never in a debug build, nor when `DOTS_DATA_DIR`
/// is set (a development or smoke instance; spec §5). An empty value counts as not set.
pub fn checks_enabled(debug: bool, data_dir_override: Option<OsString>) -> bool {
    !debug && data_dir_override.is_none_or(|v| v.is_empty())
}

/// The manifest to check: `DOTS_UPDATE_ENDPOINT` when set and not empty, else the GitHub
/// "latest" one. The error names the variable and the value.
pub fn endpoint(env_override: Option<OsString>) -> Result<Url, String> {
    match env_override.filter(|v| !v.is_empty()) {
        Some(raw) => {
            let text = raw.to_string_lossy();
            Url::parse(&text).map_err(|e| format!("{ENDPOINT_ENV} is not a URL ({text}): {e}"))
        }
        None => Ok(Url::parse(DEFAULT_ENDPOINT).expect("the default endpoint is a URL")),
    }
}

/// A version's release page, which the strip's "What's new" opens.
pub fn release_page(version: &str) -> String {
    format!("https://github.com/Aakshar-D/dots/releases/tag/v{version}")
}

/// `dots-<version>-YYYY-MM-DD-HHMMSS.db` (spec §7).
pub fn backup_file_name(version: &str, at: DateTime<Local>) -> String {
    format!("dots-{version}-{}.db", at.format("%Y-%m-%d-%H%M%S"))
}

/// The time in a backup's name, which orders backups whatever their versions.
fn stamp(name: &str) -> Option<&str> {
    let stem = name.strip_prefix("dots-")?.strip_suffix(".db")?;
    let at = stem.len().checked_sub(17)?;
    let (version, stamp) = stem.split_at(at);
    (version.ends_with('-') && stamp.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .then_some(stamp)
}

/// The backups in `dir`, newest first. Other files are left out.
fn backups(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(stamp) = stamp(name) {
            found.push((stamp.to_string(), path.clone()));
        }
    }
    found.sort_by(|a, b| b.cmp(a));
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

/// The newest backup in the data folder, for the fatal-error page (spec §8).
pub fn newest_backup(data_dir: &Path) -> Option<PathBuf> {
    backups(&data_dir.join(BACKUPS_DIR))
        .ok()?
        .into_iter()
        .next()
}

/// Backs up `dots.db` of `data_dir` into its backups folder under `backup_file_name`, then
/// deletes all but the `KEEP_BACKUPS` newest backups. Returns the new backup's path. A backup
/// that could not be pruned after is logged, not an error.
pub async fn back_up(
    data_dir: &Path,
    version: &str,
    at: DateTime<Local>,
) -> Result<PathBuf, String> {
    let dir = data_dir.join(BACKUPS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(backup_file_name(version, at));
    dots_core::store::backup_database(&data_dir.join(dots_core::store::DB_FILE), &dest)
        .await
        .map_err(|e| e.to_string())?;
    match backups(&dir) {
        Ok(all) => {
            for old in all.iter().skip(KEEP_BACKUPS) {
                if let Err(e) = std::fs::remove_file(old) {
                    tracing::warn!("could not delete the old backup {}: {e}", old.display());
                }
            }
        }
        Err(e) => tracing::warn!("could not list the backups in {}: {e}", dir.display()),
    }
    Ok(dest)
}

/// Leaves `error` for the next start. A failure is logged: the restart goes on.
pub fn leave_error(data_dir: &Path, error: &str) {
    if let Err(e) = std::fs::write(data_dir.join(ERROR_MARK), error) {
        tracing::warn!(
            "could not leave the update error in {}: {e}",
            data_dir.display()
        );
    }
}

/// What the last start left with `leave_error`, removed so it counts for one start.
pub fn take_error(data_dir: &Path) -> Option<String> {
    let mark = data_dir.join(ERROR_MARK);
    let text = std::fs::read_to_string(&mark).ok()?;
    if let Err(e) = std::fs::remove_file(&mark) {
        tracing::warn!("could not remove {}: {e}", mark.display());
    }
    Some(text)
}

/// The steps an install is made of, so the order is tested without installing anything.
/// Only this crate implements it, so the futures need no `Send` bound in the signature.
#[allow(async_fn_in_trait)]
pub trait InstallSteps {
    /// Stops the runtime and waits, at most `CLOSE_LIMIT`, for the database to close.
    async fn close_runtime(&self) -> bool;
    async fn back_up(&self) -> Result<PathBuf, String>;
    /// Restarts into this version with `error` left for the next start. Does not return in
    /// the app.
    fn restart_current(&self, error: &str);
    /// The restart mark, so the new Dots shows its window.
    fn mark_restart(&self);
    /// Starts the installer and ends the process. Returns only when it could not start.
    fn install(&self) -> Result<(), String>;
}

/// An install, in the order of spec §7. Every failure restarts into this version with the
/// reason, which the strip shows after that start.
pub async fn install(steps: &impl InstallSteps) {
    if !steps.close_runtime().await {
        steps.restart_current("Dots could not close its database in time, so no backup was made.");
        return;
    }
    if let Err(e) = steps.back_up().await {
        steps.restart_current(&format!("The backup failed: {e}"));
        return;
    }
    steps.mark_restart();
    if let Err(e) = steps.install() {
        steps.restart_current(&format!("The update could not start: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use chrono::TimeZone;

    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn ready(working: &[&str], error: Option<&str>) -> UpdateNote {
        UpdateNote::Ready {
            version: "0.2.0".into(),
            working: names(working),
            error: error.map(String::from),
        }
    }

    #[test]
    fn there_is_no_note_until_an_update_is_ready_and_later_hides_it() {
        let mut s = UpdateState::new(None);
        assert_eq!(s.note(&[]), UpdateNote::None);
        assert_eq!(s.ready_version(), None);
        assert!(s.wants("0.2.0"));
        assert!(s.offer("0.2.0"));
        assert_eq!(s.note(&[]), ready(&[], None));
        assert!(!s.wants("0.2.0"), "the same version is downloaded once");
        s.later();
        assert_eq!(s.note(&[]), UpdateNote::None);
        assert_eq!(s.ready_version(), Some("0.2.0"), "hidden, still ready");
        // A newer version after "Later" stays hidden too, until the next start.
        assert!(!s.offer("0.3.0"));
        assert_eq!(s.note(&[]), UpdateNote::None);
    }

    #[test]
    fn the_note_names_the_working_dots_and_the_last_error() {
        let mut s = UpdateState::new(Some("The backup failed: disk full".into()));
        s.offer("0.2.0");
        assert_eq!(
            s.note(&names(&["Daily digest"])),
            ready(&["Daily digest"], Some("The backup failed: disk full"))
        );
    }

    #[test]
    fn restart_installs_at_once_when_nothing_works_and_waits_otherwise() {
        let mut s = UpdateState::new(None);
        assert!(s.restart(&[]).is_err(), "nothing is ready");
        s.offer("0.2.0");
        assert_eq!(s.restart(&[]).unwrap(), Next::Install);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Installing {
                version: "0.2.0".into()
            }
        );
        assert!(
            s.restart(&[]).is_err(),
            "an install that has begun is not begun twice"
        );

        let mut s = UpdateState::new(None);
        s.offer("0.2.0");
        let busy = names(&["Daily digest"]);
        assert_eq!(s.restart(&busy).unwrap(), Next::Nothing);
        assert!(s.holds_runs());
        assert_eq!(
            s.note(&busy),
            UpdateNote::Waiting {
                version: "0.2.0".into(),
                waiting_for: busy.clone(),
            }
        );
        assert!(
            !s.wants("0.3.0"),
            "a waiting install keeps the version the person saw"
        );
    }

    #[test]
    fn a_wait_ends_with_the_last_run_or_with_cancel() {
        let mut s = UpdateState::new(None);
        s.offer("0.2.0");
        s.restart(&names(&["a", "b"])).unwrap();
        assert_eq!(s.runs_changed(&names(&["b"])), Next::Nothing);
        assert_eq!(s.runs_changed(&[]), Next::Install);
        assert_eq!(s.runs_changed(&[]), Next::Nothing, "one install");

        let mut s = UpdateState::new(None);
        s.offer("0.2.0");
        s.restart(&names(&["a"])).unwrap();
        s.cancel_wait();
        assert!(!s.holds_runs());
        assert_eq!(s.runs_changed(&[]), Next::Nothing);
        assert_eq!(s.note(&[]), ready(&[], None));
    }

    #[test]
    fn stop_and_restart_installs_whatever_works() {
        let mut s = UpdateState::new(None);
        assert!(s.stop_and_restart().is_err());
        s.offer("0.2.0");
        s.restart(&names(&["a"])).unwrap();
        assert_eq!(s.stop_and_restart().unwrap(), Next::Install);
    }

    #[test]
    fn checks_are_off_in_debug_builds_and_under_dots_data_dir() {
        assert!(checks_enabled(false, None));
        assert!(checks_enabled(false, Some("".into())));
        assert!(!checks_enabled(true, None));
        assert!(!checks_enabled(false, Some("C:/smoke".into())));
    }

    #[test]
    fn the_endpoint_is_github_latest_unless_the_variable_names_another() {
        assert_eq!(endpoint(None).unwrap().as_str(), DEFAULT_ENDPOINT);
        assert_eq!(
            endpoint(Some("".into())).unwrap().as_str(),
            DEFAULT_ENDPOINT
        );
        let rc = "https://github.com/Aakshar-D/dots/releases/download/v0.2.0-rc1/latest.json";
        assert_eq!(endpoint(Some(rc.into())).unwrap().as_str(), rc);
        let err = endpoint(Some("not a url".into())).unwrap_err();
        assert!(
            err.starts_with("DOTS_UPDATE_ENDPOINT is not a URL (not a url)"),
            "{err}"
        );
    }

    fn at(h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 20, h, 5, 9).unwrap()
    }

    #[test]
    fn whats_new_is_the_versions_release_page() {
        assert_eq!(
            release_page("0.2.0"),
            "https://github.com/Aakshar-D/dots/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn a_backup_is_named_after_the_old_version_and_the_time() {
        assert_eq!(
            backup_file_name("0.1.0", at(9)),
            "dots-0.1.0-2026-10-20-090509.db"
        );
        assert_eq!(
            stamp("dots-0.1.0-2026-10-20-090509.db"),
            Some("2026-10-20-090509")
        );
        assert_eq!(stamp("dots-0.1.0.db"), None);
        assert_eq!(stamp("notes.txt"), None);
    }

    #[tokio::test]
    async fn a_backup_keeps_the_three_newest_and_nothing_else_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let store = dots_core::store::Store::open(&dir.path().join("dots.db"))
            .await
            .unwrap();
        store.close().await;
        let backups_dir = dir.path().join(BACKUPS_DIR);
        std::fs::create_dir_all(&backups_dir).unwrap();
        // Older backups of two versions; the version does not decide the order, the time does.
        for (version, hour) in [("0.10.0", 1), ("0.9.0", 2), ("0.10.0", 3)] {
            std::fs::write(backups_dir.join(backup_file_name(version, at(hour))), "").unwrap();
        }
        std::fs::write(backups_dir.join("notes.txt"), "mine").unwrap();
        let made = back_up(dir.path(), "0.1.0", at(9)).await.unwrap();
        assert_eq!(made, backups_dir.join("dots-0.1.0-2026-10-20-090509.db"));
        let mut left: Vec<String> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "dots-0.1.0-2026-10-20-090509.db",
                "dots-0.10.0-2026-10-20-030509.db",
                "dots-0.9.0-2026-10-20-020509.db",
                "notes.txt",
            ]
        );
        assert_eq!(newest_backup(dir.path()), Some(made));
    }

    #[tokio::test]
    async fn a_backup_without_a_database_fails_and_there_is_no_newest() {
        let dir = tempfile::tempdir().unwrap();
        assert!(back_up(dir.path(), "0.1.0", at(9)).await.is_err());
        assert_eq!(newest_backup(dir.path()), None);
    }

    #[test]
    fn an_error_left_for_the_next_start_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(take_error(dir.path()), None);
        leave_error(dir.path(), "The backup failed: disk full");
        assert_eq!(
            take_error(dir.path()).as_deref(),
            Some("The backup failed: disk full")
        );
        assert_eq!(take_error(dir.path()), None);
    }

    /// Records the steps; `fail` names the one that fails.
    struct Steps {
        fail: &'static str,
        done: RefCell<Vec<String>>,
    }

    impl Steps {
        fn new(fail: &'static str) -> Self {
            Self {
                fail,
                done: RefCell::new(Vec::new()),
            }
        }
        fn log(&self, step: &str) -> bool {
            self.done.borrow_mut().push(step.to_string());
            self.fail != step
        }
        fn done(&self) -> Vec<String> {
            self.done.borrow().clone()
        }
    }

    impl InstallSteps for Steps {
        async fn close_runtime(&self) -> bool {
            self.log("close")
        }
        async fn back_up(&self) -> Result<PathBuf, String> {
            self.log("back up")
                .then(|| PathBuf::from("b.db"))
                .ok_or_else(|| "disk full".to_string())
        }
        fn restart_current(&self, error: &str) {
            self.log(&format!("restart: {error}"));
        }
        fn mark_restart(&self) {
            self.log("mark");
        }
        fn install(&self) -> Result<(), String> {
            self.log("install")
                .then_some(())
                .ok_or_else(|| "access denied".to_string())
        }
    }

    #[tokio::test]
    async fn an_install_closes_backs_up_marks_then_installs() {
        let steps = Steps::new("");
        install(&steps).await;
        assert_eq!(steps.done(), ["close", "back up", "mark", "install"]);
    }

    #[tokio::test]
    async fn a_failed_step_restarts_on_this_version_with_the_reason() {
        let steps = Steps::new("close");
        install(&steps).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "restart: Dots could not close its database in time, so no backup was made."
            ]
        );
        let steps = Steps::new("back up");
        install(&steps).await;
        assert_eq!(
            steps.done(),
            ["close", "back up", "restart: The backup failed: disk full"]
        );
        let steps = Steps::new("install");
        install(&steps).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "back up",
                "mark",
                "install",
                "restart: The update could not start: access denied"
            ]
        );
    }
}
```

- [ ] **Step 2: Run the module's tests**

Run: `cargo test -p dots-app --lib update::`
Expected: PASS: 14 tests. (Written before the code, they fail to compile: the module is new.)

- [ ] **Step 3: Run the checks**

Run: `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
Expected: `dots-app` 80 passed; no warnings (`InstallSteps` and `install` are `pub`, so nothing is dead code before Task 4 uses them).

- [ ] **Step 4: Commit**

```bash
git add Cargo.lock apps/dots-app/Cargo.toml apps/dots-app/src/lib.rs apps/dots-app/src/update.rs
git commit -m "feat(app): the update note, backups and the install order"
```

### Task 4: Check for, download and install updates

`update_service.rs` wires Task 3 to `tauri-plugin-updater`: it checks (unless `checks_enabled` says no), downloads a newer version and offers it, keeps the note's working dots current from `RunUpdated` events, begins a waiting install when the last run ends, and runs the install's steps. Five commands answer the strip's buttons with the next note, `update_open_notes` opens the release page, and the `update` event carries each change. The fatal-error page learns the newest backup (`StartupError.backup`). The UI's types, commands and event follow, with the fixture, so the contract tests stay green.

Replace the `pubkey` value in the `tauri.conf.json` block with the owner's public key ("Before you start").

**Files:**

- Modify: `apps/dots-app/Cargo.toml`
- Modify: `apps/dots-app/src/commands.rs`
- Modify: `apps/dots-app/src/contract.rs`
- Modify: `apps/dots-app/src/events.rs`
- Modify: `apps/dots-app/src/lib.rs`
- Modify: `apps/dots-app/src/state.rs`
- Create: `apps/dots-app/src/update_service.rs`
- Modify: `apps/dots-app/tauri.conf.json`
- Modify: `ui/src/api/commands.ts`, `ui/src/api/events.ts`, `ui/src/api/types.ts`
- Modify: `ui/src/api/contract.test.ts`, `ui/src/api/events.test.ts`
- Regenerate: `Cargo.lock`, `ui/src/api/__fixtures__/contract.json`

**Interfaces:**

- Consumes: `Runtime::{hold_runs, working_dots, close, subscribe}` (Task 2); all of `crate::update` (Task 3); `startup::{mark_restart, DATA_DIR_ENV}`.
- Produces: `update_service::Updates::new(data_dir: PathBuf, error: Option<String>)` (Tauri-managed, only when the runtime started), `update_service::{note, act, release_page, spawn}`; commands `update_note`, `update_open_notes`, `update_later`, `update_restart`, `update_stop_and_restart`, `update_cancel_wait` (each but `update_open_notes` answers the `UpdateNote` that follows); `events::UPDATE_EVENT = "update"`; `StartupError.backup: String` (empty when there is none); in the UI, `T.UpdateNote`, `UpdateNote`, `StartupError.backup`, `commands.updateNote`, `updateOpenNotes`, `updateLater`, `updateRestart`, `updateStopAndRestart`, `updateCancelWait`, and `onUpdate(handler)`.

- [ ] **Step 1: Write the failing tests**

The new `state.rs` test is in that file's `tests` module, in Step 3's blocks.

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  StartupError: T.StartupError,
};
```

**with:**

```ts
  StartupError: T.StartupError,
  UpdateNote: T.UpdateNote.array(),
};
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts

  it("mirrors the core's limits", () => {
```

**with:**

```ts

  it("has a sample of every update state", () => {
    const states = T.UpdateNote.options.map((o) => o.shape.state.value);
    expect(fixture.UpdateNote.map((n) => n.state).sort()).toEqual([...states].sort());
  });

  it("mirrors the core's limits", () => {
```

**In `ui/src/api/events.test.ts`, replace:**

```ts

const { onLagged, onNavigate, onRuntimeEvent, onStatus } = await import("./events");

```

**with:**

```ts

const { onLagged, onNavigate, onRuntimeEvent, onStatus, onUpdate } = await import("./events");

```

**In `ui/src/api/events.test.ts`, replace:**

```ts
    await onStatus((s) => seen.push(s.port));
    handlers.get("runtime")!({ payload: fixture.RuntimeEvent[0] });
```

**with:**

```ts
    await onStatus((s) => seen.push(s.port));
    await onUpdate((n) => seen.push(n.state));
    handlers.get("runtime")!({ payload: fixture.RuntimeEvent[0] });
```

**In `ui/src/api/events.test.ts`, replace:**

```ts
    handlers.get("status")!({ payload: fixture.Status });
    expect(seen).toEqual(["run_updated", "lagged", "/inbox", 47321]);
  });
```

**with:**

```ts
    handlers.get("status")!({ payload: fixture.Status });
    handlers.get("update")!({ payload: fixture.UpdateNote[1] });
    expect(seen).toEqual(["run_updated", "lagged", "/inbox", 47321, "ready"]);
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/api` (in `ui`)
Expected: FAIL: `T.UpdateNote` is undefined, `onUpdate` is not exported, and the fixture has no `UpdateNote`.

- [ ] **Step 3: Add the plugin, the service, the commands, the event and the UI's types**

**In `apps/dots-app/Cargo.toml`, replace:**

```toml
tauri-plugin-single-instance = "2.5"
tracing = "0.1"
```

**with:**

```toml
tauri-plugin-single-instance = "2.5"
tauri-plugin-updater = "2.14"
tracing = "0.1"
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
use crate::tray::Tray;

```

**with:**

```rust
use crate::tray::Tray;
use crate::update::{Next, UpdateNote};
use crate::update_service;

```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
    app.restart();
}
```

**with:**

```rust
    app.restart();
}

#[tauri::command]
pub async fn update_note(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::note(&app, &state.rt()?).await
}

/// "What's new": opens the release page of the update that is ready. The page is the core's
/// choice, so the webview cannot open an address of its own.
#[tauri::command]
pub fn update_open_notes(app: AppHandle) -> Res<()> {
    let page = update_service::release_page(&app)?;
    app.opener()
        .open_url(&page, None::<&str>)
        .map_err(|e| CmdError::internal(format!("could not open {page}: {e}")))
}

/// "Later": the note is hidden until Dots next starts.
#[tauri::command]
pub async fn update_later(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::act(&app, &state.rt()?, |s, _| {
        s.later();
        Ok(Next::Nothing)
    })
    .await
}

/// "Restart to update" and "Restart when it's done": installs now when no dot has a run
/// working, else once the last of them has ended; no queued run starts meanwhile.
#[tauri::command]
pub async fn update_restart(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::act(&app, &state.rt()?, |s, working| s.restart(working)).await
}

/// "Stop it and restart": installs now. A run that is working ends as failed ("interrupted")
/// at the next start, as after "Restart Dots"; the UI asks first.
#[tauri::command]
pub async fn update_stop_and_restart(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res<UpdateNote> {
    update_service::act(&app, &state.rt()?, |s, _| s.stop_and_restart()).await
}

/// "Cancel" of a waiting install: the queue starts again.
#[tauri::command]
pub async fn update_cancel_wait(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::act(&app, &state.rt()?, |s, _| {
        s.cancel_wait();
        Ok(Next::Nothing)
    })
    .await
}
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use crate::state::StartupError;

```

**with:**

```rust
use crate::state::StartupError;
use crate::update::UpdateNote;

```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
```

**with:**

```rust

/// One note of each state (`UpdateNote`).
fn update_notes() -> Vec<UpdateNote> {
    vec![
        UpdateNote::None,
        UpdateNote::Ready {
            version: "0.2.0".into(),
            working: vec!["Daily digest".into()],
            error: Some("The backup failed: disk full".into()),
        },
        UpdateNote::Waiting {
            version: "0.2.0".into(),
            waiting_for: vec!["Daily digest".into()],
        },
        UpdateNote::Installing {
            version: "0.2.0".into(),
        },
    ]
}

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            log_dir: "C:/Users/u/AppData/Local/dots/logs".into(),
        }),
```

**with:**

```rust
            log_dir: "C:/Users/u/AppData/Local/dots/logs".into(),
            backup: "C:/Users/u/AppData/Local/dots/backups/dots-0.1.0-2026-10-20-091700.db".into(),
        }),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    samples["ContextDocuments"] = to(context_documents());
    samples
```

**with:**

```rust
    samples["ContextDocuments"] = to(context_documents());
    samples["UpdateNote"] = to(update_notes());
    samples
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
pub const STATUS_EVENT: &str = "status";

```

**with:**

```rust
pub const STATUS_EVENT: &str = "status";
/// The update strip changed (updates spec §6); payload is the `UpdateNote`.
pub const UPDATE_EVENT: &str = "update";

```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
pub mod update;

```

**with:**

```rust
pub mod update;
pub mod update_service;

```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    restart_app,
    dot_overview,
```

**with:**

```rust
    restart_app,
    update_note,
    update_open_notes,
    update_later,
    update_restart,
    update_stop_and_restart,
    update_cancel_wait,
    dot_overview,
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
        events::spawn_forwarder(app.clone(), rt.clone());
    }
```

**with:**

```rust
        events::spawn_forwarder(app.clone(), rt.clone());
        if let Ok(dir) = &data_dir {
            // What a failed install left is read while Dots holds the data folder.
            app.manage(update_service::Updates::new(
                dir.clone(),
                update::take_error(dir),
            ));
            update_service::spawn(app.clone(), rt.clone());
        }
    }
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
```

**with:**

```rust
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
```

**In `apps/dots-app/src/state.rs`, replace:**

```rust
    pub log_dir: String,
}
```

**with:**

```rust
    pub log_dir: String,
    /// The newest backup an update made, empty when there is none (updates spec §8).
    pub backup: String,
}
```

**In `apps/dots-app/src/state.rs`, replace:**

```rust
                .unwrap_or_default(),
        })
```

**with:**

```rust
                .unwrap_or_default(),
            backup: self
                .data_dir
                .as_deref()
                .and_then(crate::update::newest_backup)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        })
```

**In `apps/dots-app/src/state.rs`, replace:**

```rust
                log_dir: "".into(),
            })
```

**with:**

```rust
                log_dir: "".into(),
                backup: "".into(),
            })
```

**In `apps/dots-app/src/state.rs`, replace:**

```rust
                log_dir: "C:/logs".into(),
            })
```

**with:**

```rust
                log_dir: "C:/logs".into(),
                backup: "".into(),
            })
```

**In `apps/dots-app/src/state.rs`, replace:**

```rust
        );
    }
}
```

**with:**

```rust
        );
    }

    #[test]
    fn a_failed_start_names_the_newest_backup_of_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(crate::update::BACKUPS_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let newest = backups.join("dots-0.1.0-2026-10-20-091700.db");
        std::fs::write(backups.join("dots-0.1.0-2026-10-19-091700.db"), "").unwrap();
        std::fs::write(&newest, "").unwrap();
        let state = AppState::new(Err("migration 8 failed".into()), None)
            .with_data_dir(Some(dir.path().to_path_buf()));
        let shown = state.startup_error().unwrap();
        assert_eq!(shown.backup, newest.to_string_lossy());
    }
}
```

**Create `apps/dots-app/src/update_service.rs`:**

```rust
//! Wires `update.rs` to `tauri-plugin-updater`, the runtime and the window: the checks, the
//! downloaded update, the strip's buttons and the install (updates spec §5-§7).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use dots_core::events::RuntimeEvent;
use dots_core::Runtime;
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::broadcast::error::RecvError;

use crate::error::CmdError;
use crate::events::UPDATE_EVENT;
use crate::update::{self, InstallSteps, Next, UpdateNote, UpdateState};

/// A downloaded update whose signature the updater has checked.
struct Downloaded {
    update: Update,
    bytes: Vec<u8>,
}

/// Tauri-managed: what the strip shows, and the update it installs. Managed only when the
/// runtime started.
pub struct Updates {
    state: Mutex<UpdateState>,
    downloaded: Mutex<Option<Downloaded>>,
    data_dir: PathBuf,
}

impl Updates {
    /// `error`: what the last start left (`update::take_error`).
    pub fn new(data_dir: PathBuf, error: Option<String>) -> Self {
        Self {
            state: Mutex::new(UpdateState::new(error)),
            downloaded: Mutex::new(None),
            data_dir,
        }
    }
}

fn updates(app: &AppHandle) -> Result<tauri::State<'_, Updates>, CmdError> {
    app.try_state::<Updates>()
        .ok_or_else(|| CmdError::internal("Dots did not start; see the startup error"))
}

/// The note as it stands, with the dots that have a run working now.
pub async fn note(app: &AppHandle, rt: &Arc<Runtime>) -> Result<UpdateNote, CmdError> {
    let working = rt.working_dots().await?;
    Ok(updates(app)?.state.lock().unwrap().note(&working))
}

/// The release page of the update that is ready ("What's new").
pub fn release_page(app: &AppHandle) -> Result<String, CmdError> {
    let updates = updates(app)?;
    let state = updates.state.lock().unwrap();
    match state.ready_version() {
        Some(version) => Ok(update::release_page(version)),
        None => Err(dots_core::Error::Conflict("no update is ready".into()).into()),
    }
}

/// Applies a button of the strip, holds or releases the run queue to match, announces the
/// note and, when the install begins, starts it. Returns the note.
pub async fn act(
    app: &AppHandle,
    rt: &Arc<Runtime>,
    button: impl FnOnce(&mut UpdateState, &[String]) -> dots_core::Result<Next>,
) -> Result<UpdateNote, CmdError> {
    let working = rt.working_dots().await?;
    let updates = updates(app)?;
    let (next, note) = {
        let mut state = updates.state.lock().unwrap();
        let next = button(&mut state, &working)?;
        rt.hold_runs(state.holds_runs());
        (next, state.note(&working))
    };
    announce(app, &note);
    if next == Next::Install {
        start_install(app, rt);
    }
    Ok(note)
}

fn announce(app: &AppHandle, note: &UpdateNote) {
    if let Err(e) = app.emit(UPDATE_EVENT, note) {
        tracing::warn!("emitting the update note failed: {e}");
    }
}

/// Starts the watch on runs and, unless this Dots does not check (`update::checks_enabled`),
/// the checks.
pub fn spawn(app: AppHandle, rt: Arc<Runtime>) {
    let mut rx = rt.subscribe();
    let watched = (app.clone(), rt.clone());
    tauri::async_runtime::spawn(async move {
        let (app, rt) = watched;
        loop {
            match rx.recv().await {
                Ok(RuntimeEvent::RunUpdated { .. }) | Err(RecvError::Lagged(_)) => {
                    runs_changed(&app, &rt).await
                }
                Ok(_) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
    if !update::checks_enabled(
        cfg!(debug_assertions),
        std::env::var_os(crate::startup::DATA_DIR_ENV),
    ) {
        tracing::info!("update checks are off in this build or under DOTS_DATA_DIR");
        return;
    }
    let url = match update::endpoint(std::env::var_os(update::ENDPOINT_ENV)) {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!("update checks are off: {e}");
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(update::FIRST_CHECK_AFTER).await;
        loop {
            if let Err(e) = check(&app, &rt, &url).await {
                tracing::warn!("checking for an update failed: {e}");
            }
            tokio::time::sleep(update::CHECK_EVERY).await;
        }
    });
}

/// A run changed: the note's list of working dots follows, and a waiting install begins once
/// none is left.
async fn runs_changed(app: &AppHandle, rt: &Arc<Runtime>) {
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    // Fails once an install has closed the database; there is nothing left to show then.
    let Ok(working) = rt.working_dots().await else {
        return;
    };
    let (next, note) = {
        let mut state = updates.state.lock().unwrap();
        (state.runs_changed(&working), state.note(&working))
    };
    if note != UpdateNote::None {
        announce(app, &note);
    }
    if next == Next::Install {
        start_install(app, rt);
    }
}

/// One check: a newer version is downloaded, its signature checked, and offered.
async fn check(app: &AppHandle, rt: &Runtime, url: &Url) -> Result<(), String> {
    let updater = app
        .updater_builder()
        .endpoints(vec![url.clone()])
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let Some(found) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let updates = updates(app).map_err(|e| e.message)?;
    if !updates.state.lock().unwrap().wants(&found.version) {
        return Ok(());
    }
    let bytes = found
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    let version = found.version.clone();
    *updates.downloaded.lock().unwrap() = Some(Downloaded {
        update: found,
        bytes,
    });
    let shown = updates.state.lock().unwrap().offer(&version);
    tracing::info!("Dots {version} is downloaded and ready to install");
    if shown {
        let working = rt.working_dots().await.unwrap_or_default();
        let note = updates.state.lock().unwrap().note(&working);
        announce(app, &note);
    }
    Ok(())
}

fn start_install(app: &AppHandle, rt: &Arc<Runtime>) {
    let Ok(updates) = updates(app) else {
        return;
    };
    let Some(downloaded) = updates.downloaded.lock().unwrap().take() else {
        tracing::error!("an install began without a downloaded update");
        return;
    };
    let steps = AppSteps {
        app: app.clone(),
        rt: rt.clone(),
        data_dir: updates.data_dir.clone(),
        downloaded,
    };
    tauri::async_runtime::spawn(async move { update::install(&steps).await });
}

struct AppSteps {
    app: AppHandle,
    rt: Arc<Runtime>,
    data_dir: PathBuf,
    downloaded: Downloaded,
}

impl InstallSteps for AppSteps {
    async fn close_runtime(&self) -> bool {
        self.rt.close(update::CLOSE_LIMIT).await
    }

    async fn back_up(&self) -> Result<PathBuf, String> {
        let made = update::back_up(&self.data_dir, env!("CARGO_PKG_VERSION"), Local::now()).await?;
        tracing::info!("backed up the database to {}", made.display());
        Ok(made)
    }

    fn restart_current(&self, error: &str) {
        tracing::error!("the update was not installed: {error}");
        update::leave_error(&self.data_dir, error);
        crate::startup::mark_restart(&self.data_dir);
        self.app.restart();
    }

    fn mark_restart(&self) {
        crate::startup::mark_restart(&self.data_dir);
    }

    fn install(&self) -> Result<(), String> {
        tracing::info!("installing Dots {}", self.downloaded.update.version);
        self.downloaded
            .update
            .install(&self.downloaded.bytes)
            .map_err(|e| e.to_string())
    }
}
```

**In `apps/dots-app/tauri.conf.json`, replace:**

```json
    }
  }
```

**with:**

```json
    }
  },
  "plugins": {
    "updater": {
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI2NTAyQ0VBNUM4N0VCOEEKUldTSzY0ZGM2aXhRdHUxQ0RJcGRmRGZTeVlqcUZJZzIzaEQ2NGxYOHNPUlpwL1lWdCtkTXVFY2MK",
      "windows": { "installMode": "passive" }
    }
  }
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  restartApp: () => call(nothing, "restart_app"),

```

**with:**

```ts
  restartApp: () => call(nothing, "restart_app"),
  updateNote: () => call(T.UpdateNote, "update_note"),
  updateOpenNotes: () => call(nothing, "update_open_notes"),
  updateLater: () => call(T.UpdateNote, "update_later"),
  updateRestart: () => call(T.UpdateNote, "update_restart"),
  updateStopAndRestart: () => call(T.UpdateNote, "update_stop_and_restart"),
  updateCancelWait: () => call(T.UpdateNote, "update_cancel_wait"),

```

**In `ui/src/api/events.ts`, replace:**

```ts
import * as T from "./types";
import type { RuntimeEvent, Status } from "./types";

```

**with:**

```ts
import * as T from "./types";
import type { RuntimeEvent, Status, UpdateNote } from "./types";

```

**In `ui/src/api/events.ts`, replace:**

```ts
    if (status) handler(status);
  });
}
```

**with:**

```ts
    if (status) handler(status);
  });
}

/// The update strip changed: an update is ready, a wait began or ended, an install began.
export function onUpdate(handler: (note: UpdateNote) => void): Promise<UnlistenFn> {
  return listen("update", (e) => {
    const note = parse("update", T.UpdateNote, e.payload);
    if (note) handler(note);
  });
}
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const CmdError = z.strictObject({ kind: CmdErrorKind, message: z.string() });
export const StartupError = z.strictObject({ message: z.string(), log_dir: z.string() });

```

**with:**

```ts
export const CmdError = z.strictObject({ kind: CmdErrorKind, message: z.string() });
export const StartupError = z.strictObject({
  message: z.string(),
  log_dir: z.string(),
  backup: z.string(),
});

/// What the update strip shows (apps/dots-app/src/update.rs).
export const UpdateNote = z.discriminatedUnion("state", [
  z.strictObject({ state: z.literal("none") }),
  z.strictObject({
    state: z.literal("ready"),
    version: z.string(),
    working: z.array(z.string()),
    error: nullable(z.string()),
  }),
  z.strictObject({
    state: z.literal("waiting"),
    version: z.string(),
    waiting_for: z.array(z.string()),
  }),
  z.strictObject({ state: z.literal("installing"), version: z.string() }),
]);

```

**In `ui/src/api/types.ts`, replace:**

```ts
export type StartupError = z.infer<typeof StartupError>;
```

**with:**

```ts
export type StartupError = z.infer<typeof StartupError>;
export type UpdateNote = z.infer<typeof UpdateNote>;
```

- [ ] **Step 4: Regenerate the contract fixture**

Run: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then `cargo test -p dots-app contract`.
Expected: both pass; the fixture gains `UpdateNote` (four notes), `StartupError.backup` and the six commands.

- [ ] **Step 5: Run the task's tests**

Run: `cargo test -p dots-app`, then `DEBUG_PRINT_LIMIT=0 bun x vitest run src/api` (in `ui`).
Expected: PASS: `dots-app` 81; the UI's `src/api` 65 tests in 3 files.

- [ ] **Step 6: Run the checks**

Run: `cargo clippy --workspace --all-targets`, `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: the UI 401 in 25 files; no warnings.

- [ ] **Step 7: Commit**

```bash
git add Cargo.lock apps/dots-app/Cargo.toml apps/dots-app/src/commands.rs apps/dots-app/src/contract.rs apps/dots-app/src/events.rs apps/dots-app/src/lib.rs apps/dots-app/src/state.rs apps/dots-app/src/update_service.rs apps/dots-app/tauri.conf.json ui/src/api/__fixtures__/contract.json ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/events.test.ts ui/src/api/events.ts ui/src/api/types.ts
git commit -m "feat(app): check for, download and install updates"
```

### Task 5: The update strip and the backup on the error page

The strip is one more `Banner` (tone `info`) at the end of `Banners`: "Dots 0.2.0 is ready." with "What's new", **Restart to update** and **Later**; with dots working, their names and **Restart when it's done** and **Stop it and restart** (which asks first, in `RestartButton`'s words); while waiting, "Will restart into Dots 0.2.0 when … finishes. New runs wait until after the update." with **Cancel**; then "Installing Dots 0.2.0." The `update` event sets the note in the query cache. The fatal-error page names the newest backup and the releases page.

**Files:**

- Modify: `ui/src/app/Banners.tsx`
- Modify: `ui/src/app/FatalPage.tsx`
- Modify: `ui/src/app/Root.tsx`
- Modify: `ui/src/lib/query.ts`
- Modify: `ui/src/lib/runtime-events.ts`
- Modify: `ui/src/test/fixtures.ts`
- Modify: `ui/src/app/Layout.test.tsx`

**Interfaces:**

- Consumes: `commands.update*`, `onUpdate`, `UpdateNote`, `StartupError.backup` (Task 4).
- Produces: `keys.update = ["update"]`; `FatalPage` takes `backup: string | null`; the test backend answers `update_note` with `{ state: "none" }` by default.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx

import type { Dot, DotOverview, Run, Status } from "@/api/types";
import { localDateTime } from "@/lib/format";
```

**with:**

```tsx

import type { Dot, DotOverview, Run, Status, UpdateNote } from "@/api/types";
import { localDateTime } from "@/lib/format";
```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
    expect(screen.getByText(sample.startupError.log_dir)).toBeVisible();
    expect(screen.queryByRole("navigation")).toBeNull();
```

**with:**

```tsx
    expect(screen.getByText(sample.startupError.log_dir)).toBeVisible();
    // The newest backup an update made, and where the previous installer is.
    expect(screen.getByText(sample.startupError.backup)).toBeVisible();
    expect(screen.getByText("https://github.com/Aakshar-D/dots/releases")).toBeVisible();
    expect(screen.queryByRole("navigation")).toBeNull();
```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
  });
});
```

**with:**

```tsx
  });
});

describe("the update strip", () => {
  const ready = (working: string[], error: string | null = null): UpdateNote => ({
    state: "ready",
    version: "0.2.0",
    working,
    error,
  });
  const waiting: UpdateNote = { state: "waiting", version: "0.2.0", waiting_for: ["triage"] };
  const installing: UpdateNote = { state: "installing", version: "0.2.0" };
  const strip = () => screen.findByText(/Dots 0\.2\.0 is ready/);

  it("shows nothing until an update is ready, then what the core announces", async () => {
    const { listening } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await listening();
    expect(screen.queryByText(/is ready/)).toBeNull();
    act(() => backend.emit("update", ready([], "The backup failed: disk full")));
    expect(await strip()).toBeVisible();
    expect(screen.getByText("The backup failed: disk full")).toBeVisible();
  });

  it("opens what's new and hides on Later", async () => {
    backend.on("update_note", () => ready([]));
    backend.on("update_open_notes", () => null);
    backend.on("update_later", () => ({ state: "none" }));
    const { user } = renderApp("/dots");
    await strip();
    await user.click(screen.getByRole("button", { name: "What's new" }));
    await waitFor(() => expect(backend.callsTo("update_open_notes")).toHaveLength(1));
    await user.click(screen.getByRole("button", { name: "Later" }));
    await waitFor(() => expect(screen.queryByText(/is ready/)).toBeNull());
  });

  it("restarts into the update at once when nothing is working", async () => {
    backend.on("update_note", () => ready([]));
    backend.on("update_restart", () => installing);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("button", { name: "Restart to update" }));
    expect(await screen.findByText(/Installing Dots 0\.2\.0/)).toBeVisible();
    expect(backend.callsTo("update_restart")).toHaveLength(1);
  });

  it("waits for the dots that are working, and Cancel stops waiting", async () => {
    backend.on("update_note", () => ready(["triage"]));
    backend.on("update_restart", () => waiting);
    backend.on("update_cancel_wait", () => ready(["triage"]));
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/triage is still running/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Restart to update" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Restart when it's done" }));
    const waitingFor = /Will restart into Dots 0\.2\.0 when triage finishes/;
    expect(await screen.findByText(waitingFor)).toBeVisible();
    expect(screen.getByText(/New runs wait until after the update/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText(/triage is still running/)).toBeVisible();
    expect(backend.callsTo("update_cancel_wait")).toHaveLength(1);
  });

  it("asks before it stops the runs that are working", async () => {
    backend.on("update_note", () => ready(["triage", "digest"]));
    backend.on("update_stop_and_restart", () => installing);
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/triage, digest are still running/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Stop them and restart" }));
    const asked = await screen.findByRole("alertdialog", { name: "Stop and restart now?" });
    const dialog = within(asked);
    expect(dialog.getByText(/triage, digest have runs that are working/)).toBeVisible();
    expect(dialog.getByText(/ends as failed/)).toBeVisible();
    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("update_stop_and_restart")).toEqual([]);

    await user.click(screen.getByRole("button", { name: "Stop them and restart" }));
    await user.click(await screen.findByRole("button", { name: "Stop and restart" }));
    expect(await screen.findByText(/Installing Dots 0\.2\.0/)).toBeVisible();
    expect(backend.callsTo("update_stop_and_restart")).toHaveLength(1);
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  backend.on("status", () => quietStatus);
  backend.on("inbox", () => ({ approvals: [], review_runs: [], attention: [], tools_changed: [] }));
```

**with:**

```ts
  backend.on("status", () => quietStatus);
  backend.on("update_note", () => ({ state: "none" }));
  backend.on("inbox", () => ({ approvals: [], review_runs: [], attention: [], tools_changed: [] }));
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/app` (in `ui`)
Expected: FAIL: the five tests of "the update strip" find no strip, and the error page shows no backup.

- [ ] **Step 3: Add the strip, the listener and the error page's lines**

**In `ui/src/app/Banners.tsx`, replace:**

```tsx
import { commands } from "@/api/commands";
import type { Status } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**with:**

```tsx
import { commands } from "@/api/commands";
import type { Status, UpdateNote } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**In `ui/src/app/Banners.tsx`, replace:**

```tsx

/// What the user must know on every screen: no local server, no or an untested CLI, settings
/// waiting for a restart, paused schedules.
export function Banners() {
```

**with:**

```tsx

/// "a", "a, b" and so on, for the dots a note names.
function dotNames(names: string[]): string {
  return names.join(", ");
}

/// The update strip (updates spec §6): a version is ready, Dots waits for the runs that are
/// working, or the install has begun. The core decides what it shows; each button answers
/// with the note that follows.
function UpdateBanner() {
  const qc = useQueryClient();
  const note = useQuery({ queryKey: keys.update, queryFn: commands.updateNote }).data;
  const press = useMutation({
    mutationFn: (button: () => Promise<UpdateNote>) => button(),
    onSuccess: (next) => qc.setQueryData(keys.update, next),
    onError: notifyError,
  });
  const whatsNew = useMutation({ mutationFn: commands.updateOpenNotes, onError: notifyError });
  if (!note || note.state === "none") return null;
  if (note.state === "installing") {
    return (
      <Banner tone="info">
        <span className="flex-1">Installing Dots {note.version}. Dots restarts in a moment.</span>
      </Banner>
    );
  }
  const busy = press.isPending;
  if (note.state === "waiting") {
    const many = note.waiting_for.length > 1;
    return (
      <Banner tone="info">
        <span className="flex-1">
          Will restart into Dots {note.version} when {dotNames(note.waiting_for)}{" "}
          {many ? "finish" : "finishes"}. New runs wait until after the update.
        </span>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => press.mutate(commands.updateCancelWait)}
        >
          Cancel
        </Button>
      </Banner>
    );
  }
  const { working } = note;
  const many = working.length > 1;
  return (
    <Banner tone="info">
      <span className="flex-1">
        Dots {note.version} is ready.
        {working.length > 0 &&
          ` ${dotNames(working)} ${many ? "are" : "is"} still running.`}{" "}
        <button type="button" className="underline" onClick={() => whatsNew.mutate()}>
          What's new
        </button>
        {note.error && <span className="block text-red-600 dark:text-red-400">{note.error}</span>}
      </span>
      {working.length === 0 ? (
        <>
          <Button size="sm" disabled={busy} onClick={() => press.mutate(commands.updateRestart)}>
            Restart to update
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => press.mutate(commands.updateLater)}
          >
            Later
          </Button>
        </>
      ) : (
        <>
          <Button size="sm" disabled={busy} onClick={() => press.mutate(commands.updateRestart)}>
            Restart when {many ? "they're" : "it's"} done
          </Button>
          <ConfirmButton
            title="Stop and restart now?"
            description={
              <>
                {dotNames(working)} {many ? "have runs that are" : "has a run that is"} working.
                A run that is working is stopped by the restart and ends as failed
                ("interrupted"). Runs that wait in the queue or for an approval continue after it.
              </>
            }
            confirmLabel="Stop and restart"
            onConfirm={() => press.mutate(commands.updateStopAndRestart)}
            disabled={busy}
          >
            Stop {many ? "them" : "it"} and restart
          </ConfirmButton>
        </>
      )}
    </Banner>
  );
}

/// What the user must know on every screen: no local server, no or an untested CLI, settings
/// waiting for a restart, paused schedules, and an update that is ready.
export function Banners() {
```

**In `ui/src/app/Banners.tsx`, replace:**

```tsx
      )}
    </>
```

**with:**

```tsx
      )}
      <UpdateBanner />
    </>
```

**In `ui/src/app/FatalPage.tsx`, replace:**

```tsx
/// Shown instead of the app when the runtime could not start (spec §8). There is no tray in
```

**with:**

```tsx
/// Where every version's installer is (updates spec §8).
const RELEASES = "https://github.com/Aakshar-D/dots/releases";

/// Shown instead of the app when the runtime could not start (spec §8). There is no tray in
```

**In `ui/src/app/FatalPage.tsx`, replace:**

```tsx
/// that state: closing the window exits Dots.
export function FatalPage(props: { message: string; logDir: string | null }) {
  return (
```

**with:**

```tsx
/// that state: closing the window exits Dots.
export function FatalPage(props: {
  message: string;
  logDir: string | null;
  backup: string | null;
}) {
  return (
```

**In `ui/src/app/FatalPage.tsx`, replace:**

```tsx
      )}
      <p className="text-sm text-muted-foreground">
```

**with:**

```tsx
      )}
      {props.backup && (
        <p className="text-sm text-muted-foreground">
          If this began after an update, the data from before it is backed up in{" "}
          <span className="font-mono text-foreground select-all">{props.backup}</span>. The
          previous version's installer is on{" "}
          <span className="font-mono text-foreground select-all">{RELEASES}</span>.
        </p>
      )}
      <p className="text-sm text-muted-foreground">
```

**In `ui/src/app/Root.tsx`, replace:**

```tsx
  if (startup.isPending) return null;
  if (startup.isError) return <FatalPage message={errorMessage(startup.error)} logDir={null} />;
  if (startup.data) {
    return <FatalPage message={startup.data.message} logDir={startup.data.log_dir} />;
  }
```

**with:**

```tsx
  if (startup.isPending) return null;
  if (startup.isError) {
    return <FatalPage message={errorMessage(startup.error)} logDir={null} backup={null} />;
  }
  if (startup.data) {
    return (
      <FatalPage
        message={startup.data.message}
        logDir={startup.data.log_dir}
        backup={startup.data.backup}
      />
    );
  }
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  status: ["status"] as const,
  settings: ["settings"] as const,
```

**with:**

```ts
  status: ["status"] as const,
  update: ["update"] as const,
  settings: ["settings"] as const,
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts

import { onLagged, onNavigate, onRuntimeEvent, onStatus } from "@/api/events";
import type { RunEventRecord, RuntimeEvent } from "@/api/types";
```

**with:**

```ts

import { onLagged, onNavigate, onRuntimeEvent, onStatus, onUpdate } from "@/api/events";
import type { RunEventRecord, RuntimeEvent } from "@/api/types";
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
/// stream refetches everything, the tray navigates, and a status changed elsewhere is taken.
export function useRuntimeEvents(): void {
```

**with:**

```ts
/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
/// stream refetches everything, the tray navigates, and a status or an update note changed
/// elsewhere is taken.
export function useRuntimeEvents(): void {
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      }),
    ];
```

**with:**

```ts
      }),
      onUpdate((note) => qc.setQueryData(keys.update, note)),
    ];
```

- [ ] **Step 4: Run the task's tests**

Run: `DEBUG_PRINT_LIMIT=0 bun x vitest run src/app` (in `ui`)
Expected: PASS: 26 tests.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`.
Expected: the UI 406 in 25 files.

- [ ] **Step 6: Commit**

```bash
git add ui/src/app/Banners.tsx ui/src/app/FatalPage.tsx ui/src/app/Layout.test.tsx ui/src/app/Root.tsx ui/src/lib/query.ts ui/src/lib/runtime-events.ts ui/src/test/fixtures.ts
git commit -m "feat(ui): the update strip and the backup on the error page"
```

### Task 6: Build, sign and publish a release from a version tag

The workflow runs on `v*` tags on `windows-latest`: it tests the release script and checks the tag against the five version fields, builds the installer with the updater files and signs it with the repo's secrets, writes `latest.json` and publishes the release (a prerelease for a tag with a suffix). `docs/releasing.md` is the owner's guide. Nothing here runs the workflow: pushing a tag is the owner's step (Task 8, Part B).

**Files:**

- Create: `tools/release/release.test.mjs`
- Create: `tools/release/release.mjs`
- Create: `.github/workflows/release.yml`
- Create: `docs/releasing.md`

**Interfaces:**

- Consumes: the version fields; the NSIS output `target/release/bundle/nsis/Dots_<version>_x64-setup.exe` and its `.sig`.
- Produces: `node tools/release/release.mjs check <tag>` (prints `prerelease=true|false`, appends it to `$GITHUB_OUTPUT`; exit 1 naming each disagreeing field) and `manifest <tag> <installer> <out>`; exported `versionFields(root)`, `checkTag(root, tag) -> { version, prerelease, errors }`, `manifest({ version, signature, installer, date })`.

- [ ] **Step 1: Write the failing tests**

**Create `tools/release/release.test.mjs`:**

```js
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { checkTag, manifest } from "./release.mjs";

/// A checkout whose version fields read `version`, except those in `except`.
function checkout(version, except = {}) {
  const root = mkdtempSync(join(tmpdir(), "release-"));
  const v = (where) => except[where] ?? version;
  const write = (path, text) => {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  write("apps/dots-app/tauri.conf.json", JSON.stringify({ version: v("tauri") }));
  write(
    "apps/dots-app/Cargo.toml",
    // CRLF, as a Windows checkout has it; a `version` in a later table is not the package's.
    `[package]\r\nname = "dots-app"\r\nversion = "${v("app")}"\r\n\r\n` +
      `[dependencies]\r\nversion = "9"\r\n`,
  );
  write("crates/dots-core/Cargo.toml", `[package]\nname = "dots-core"\nversion = "${v("core")}"\n`);
  write("ui/package.json", JSON.stringify({ name: "ui", version: v("ui") }));
  write(
    "Cargo.lock",
    `[[package]]\nname = "dots-app"\nversion = "${v("lock-app")}"\n\n` +
      `[[package]]\nname = "dots-core"\nversion = "${v("lock-core")}"\n`,
  );
  return root;
}

test("a plain tag that every field agrees with is a release", () => {
  assert.deepEqual(checkTag(checkout("0.2.0"), "v0.2.0"), {
    version: "0.2.0",
    prerelease: false,
    errors: [],
  });
});

test("a tag with a suffix is a prerelease", () => {
  const checked = checkTag(checkout("0.2.0-rc.1"), "v0.2.0-rc.1");
  assert.equal(checked.prerelease, true);
  assert.deepEqual(checked.errors, []);
});

test("every field that disagrees with the tag is named", () => {
  const checked = checkTag(checkout("0.2.0", { core: "0.1.0", "lock-app": "0.1.0" }), "v0.2.0");
  assert.deepEqual(checked.errors, [
    "crates/dots-core/Cargo.toml has 0.1.0, the tag v0.2.0",
    "Cargo.lock (dots-app) has 0.1.0, the tag v0.2.0",
  ]);
});

test("a tag that is not v<version> is refused", () => {
  for (const tag of ["0.2.0", "v0.2", "vnext", "v0.2.0 "]) {
    assert.equal(checkTag(checkout("0.2.0"), tag).errors.length, 1, tag);
  }
});

test("the manifest points the updater at the release's installer and its signature", () => {
  const body = manifest({
    version: "0.2.0",
    signature: "c2lnbmF0dXJl\n",
    installer: "C:/t/release/bundle/nsis/Dots_0.2.0_x64-setup.exe",
    date: new Date("2026-10-20T09:17:00Z"),
  });
  assert.deepEqual(body, {
    version: "0.2.0",
    notes: "Dots 0.2.0",
    pub_date: "2026-10-20T09:17:00.000Z",
    platforms: {
      "windows-x86_64": {
        signature: "c2lnbmF0dXJl",
        url: "https://github.com/Aakshar-D/dots/releases/download/v0.2.0/Dots_0.2.0_x64-setup.exe",
      },
    },
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `node --test tools/release/release.test.mjs`
Expected: FAIL: cannot find module `./release.mjs`.

- [ ] **Step 3: Add the script, the workflow and the guide**

**Create `.github/workflows/release.yml`:**

```yaml
# Builds, signs and publishes a release when a version tag is pushed (updates spec §4). The
# tag must be v<version> and agree with every version field (tools/release/release.mjs). A
# tag with a suffix, such as v0.2.0-rc.1, is published as a prerelease, which no installed
# Dots takes on its own. The person steps are in docs/releasing.md.
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  release:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - name: Check the tag against the version fields
        id: version
        shell: bash
        run: |
          node --test tools/release/release.test.mjs
          node tools/release/release.mjs check "$GITHUB_REF_NAME"

      - uses: oven-sh/setup-bun@v2
        with:
          bun-version: 1.3.13

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2

      - name: Install the UI's packages
        run: bun install --frozen-lockfile

      # The updater files (the installer's .sig) are made here only: a build on a PC has no
      # key, so `tauri.conf.json` leaves them off.
      - name: Build and sign the installer
        shell: bash
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        run: bun run tauri build --config '{"bundle":{"createUpdaterArtifacts":true}}' -- --locked

      - name: Publish the release
        shell: bash
        env:
          GH_TOKEN: ${{ github.token }}
          PRERELEASE: ${{ steps.version.outputs.prerelease }}
        run: |
          version="${GITHUB_REF_NAME#v}"
          installer="target/release/bundle/nsis/Dots_${version}_x64-setup.exe"
          node tools/release/release.mjs manifest "$GITHUB_REF_NAME" "$installer" latest.json
          flags=()
          if [ "$PRERELEASE" = "true" ]; then flags+=(--prerelease); fi
          gh release create "$GITHUB_REF_NAME" --title "Dots $version" --generate-notes \
            "${flags[@]}" "$installer" "$installer.sig" latest.json
```

**Create `docs/releasing.md`:**

```markdown
# Releasing Dots

Every installed Dots checks GitHub for a new version and offers it in a strip at the top of its window (`docs/superpowers/specs/2026-10-20-auto-update-design.md`). A release is made by GitHub (`.github/workflows/release.yml`) when a version tag is pushed. The steps below are the owner's.

## The signing key

**If the private key or its password is lost, no installed Dots can ever take another update.** Each would need a new install by hand of a build with a new public key. Keep the key file and its password in a password manager, not only on one PC.

Every update is signed with the private key. Every Dots carries the public key (`plugins.updater.pubkey` in `apps/dots-app/tauri.conf.json`) and refuses a download whose signature does not match it.

Once, before the first release:

1. Make the key pair, with a password: `bun run tauri signer generate -w ~/.tauri/dots-updates.key`. It writes `dots-updates.key` (private) and `dots-updates.key.pub` (public).
2. Put the text of `dots-updates.key.pub` in `plugins.updater.pubkey`, through a pull request.
3. In the repo's GitHub settings, under Secrets and variables, then Actions, add two repository secrets: `TAURI_SIGNING_PRIVATE_KEY` (the text of `dots-updates.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (its password).

## A release

1. In a pull request, set the new version in all five places:
   - `apps/dots-app/tauri.conf.json` (`version`)
   - `apps/dots-app/Cargo.toml` and `crates/dots-core/Cargo.toml` (`[package]` `version`)
   - `ui/package.json` (`version`)
   - `Cargo.lock`, which `cargo build` updates after the two `Cargo.toml` changes
2. Check them: `node tools/release/release.mjs check v0.2.0` prints `prerelease=false`, or names each place that disagrees.
3. Merge the pull request. Then tag the merge commit on `master` and push the tag:
   `git tag v0.2.0 origin/master` and `git push origin v0.2.0`.
4. The workflow builds, signs and publishes the release "Dots 0.2.0" with the installer, its `.sig` and `latest.json`. Its notes are GitHub's generated notes; edit them on GitHub if you like. Within six hours every running Dots offers the update.

## Trying a version before it reaches everyone

A tag with a suffix, such as `v0.2.0-rc.1`, is published as a prerelease. GitHub's "latest" skips prereleases, so no installed Dots takes it on its own. Such a tag can sit on a branch: set the five places to `0.2.0-rc.1` on the pull request's branch, tag that commit, and set them to `0.2.0` before merging.

To run the whole update path on your own machine:

1. Publish a first prerelease, for example `v0.2.0-rc.1`, and install it by hand from its release page.
2. Publish a second one, `v0.2.0-rc.2`.
3. Quit Dots, then start the installed Dots with `DOTS_UPDATE_ENDPOINT` set to the second one's manifest, in PowerShell:
   `$env:DOTS_UPDATE_ENDPOINT = "https://github.com/Aakshar-D/dots/releases/download/v0.2.0-rc.2/latest.json"; & "$env:LOCALAPPDATA\Dots\dots-app.exe"`.
4. Within a minute the strip offers `0.2.0-rc.2`. Restart to update; Dots backs up its database to `%LOCALAPPDATA%\dots\backups`, installs, and opens on the new version.

The variable changes only where Dots looks; a download must still carry your signature. A Dots started with `DOTS_DATA_DIR` set never checks.

## The first version with updates

Copies of Dots from before this feature (`0.1.0`) have no updater. Install the first release that has it by hand, once, on each machine. Every later version arrives on its own.

## Going back to an older version

A broken release is fixed by publishing a newer one. Deleting a release on GitHub makes "latest" point at the one before it, but a Dots that has updated stays where it is. To take one machine back:

1. Quit Dots.
2. Download the older version's installer from https://github.com/Aakshar-D/dots/releases and run it.
3. If the newer version changed the database, put the backup back: in `%LOCALAPPDATA%\dots`, rename `dots.db` to keep it, delete `dots.db-wal` and `dots.db-shm` if they are there, and copy the newest file from `backups\` (named `dots-<old version>-<date>-<time>.db`) to `dots.db`.
4. Start Dots. If it cannot start, its error page names the newest backup.
```

**Create `tools/release/release.mjs`:**

```js
// The release workflow's two steps that are Dots' own (updates spec §4), kept out of the
// workflow file so they are tested (`node --test tools/release/release.test.mjs`).
//
//   node tools/release/release.mjs check <tag>
//       Fails unless the tag, without its "v", is the version in every version field.
//       Prints "prerelease=true" or "prerelease=false", and appends the same line to
//       $GITHUB_OUTPUT when it is set.
//   node tools/release/release.mjs manifest <tag> <installer> <out>
//       Writes the updater manifest (latest.json) for the NSIS installer <installer>, whose
//       signature is <installer>.sig, to <out>.

import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

/// Where the release's files are downloaded from.
const DOWNLOADS = "https://github.com/Aakshar-D/dots/releases/download";
/// The updater's name for this platform's entry in the manifest.
const PLATFORM = "windows-x86_64";

/// The version of every field the version lives in, by where it is.
export function versionFields(root) {
  const read = (path) => readFileSync(join(root, path), "utf8").replace(/\r\n/g, "\n");
  const cargo = (path) => read(path).match(/^\[package\][^[]*?^version = "([^"]*)"/m)?.[1];
  const locked = (name) =>
    read("Cargo.lock").match(new RegExp(`^name = "${name}"\\nversion = "([^"]*)"`, "m"))?.[1];
  return {
    "apps/dots-app/tauri.conf.json": JSON.parse(read("apps/dots-app/tauri.conf.json")).version,
    "apps/dots-app/Cargo.toml": cargo("apps/dots-app/Cargo.toml"),
    "crates/dots-core/Cargo.toml": cargo("crates/dots-core/Cargo.toml"),
    "ui/package.json": JSON.parse(read("ui/package.json")).version,
    "Cargo.lock (dots-app)": locked("dots-app"),
    "Cargo.lock (dots-core)": locked("dots-core"),
  };
}

/// The tag's version and whether it is a prerelease, or the reasons the tag cannot be
/// released. Only a plain `vMAJOR.MINOR.PATCH` tag is a release; any other is a prerelease,
/// which GitHub's "latest" skips.
export function checkTag(root, tag) {
  if (!/^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(tag)) {
    return { errors: [`the tag ${tag} is not v<version>, such as v0.2.0 or v0.2.0-rc.1`] };
  }
  const version = tag.slice(1);
  const errors = Object.entries(versionFields(root))
    .filter(([, found]) => found !== version)
    .map(([where, found]) => `${where} has ${found ?? "no version"}, the tag ${tag}`);
  return { version, prerelease: version.includes("-"), errors };
}

/// The updater manifest for one Windows installer.
export function manifest({ version, signature, installer, date }) {
  return {
    version,
    notes: `Dots ${version}`,
    pub_date: date.toISOString(),
    platforms: {
      [PLATFORM]: {
        signature: signature.trim(),
        url: `${DOWNLOADS}/v${version}/${encodeURIComponent(basename(installer))}`,
      },
    },
  };
}

function main([command, tag, installer, out]) {
  const root = fileURLToPath(new URL("../..", import.meta.url));
  const checked = tag ? checkTag(root, tag) : { errors: ["no tag given"] };
  if (checked.errors.length > 0) {
    for (const error of checked.errors) console.error(`release: ${error}`);
    process.exit(1);
  }
  if (command === "check") {
    const line = `prerelease=${checked.prerelease}`;
    console.log(line);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${line}\n`);
  } else if (command === "manifest" && installer && out) {
    const signature = readFileSync(`${installer}.sig`, "utf8");
    const body = manifest({ version: checked.version, signature, installer, date: new Date() });
    writeFileSync(out, `${JSON.stringify(body, null, 2)}\n`);
    console.log(`release: wrote ${out}`);
  } else {
    console.error("usage: release.mjs check <tag> | manifest <tag> <installer> <out>");
    process.exit(2);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main(process.argv.slice(2));
}
```

- [ ] **Step 4: Run the task's tests and the check against this checkout**

Run: `node --test tools/release/release.test.mjs`, then `node tools/release/release.mjs check v0.1.0`, then `node tools/release/release.mjs check v0.2.0`.
Expected: 5 tests pass; `prerelease=false` with exit 0; six lines `release: … has 0.1.0, the tag v0.2.0` with exit 1.

- [ ] **Step 5: Check the workflow file**

Run: `python -c "import yaml; d = yaml.safe_load(open('.github/workflows/release.yml')); print([s.get('name', s.get('uses')) for s in d['jobs']['release']['steps']])"` (or any YAML parser at hand).
Expected: the eight steps, from `actions/checkout@v4` to "Publish the release".

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml docs/releasing.md tools/release/release.mjs tools/release/release.test.mjs
git commit -m "ci: build, sign and publish a release from a version tag"
```

### Task 7: Bring the spec in line

The spec's sections 4, 6 and 7 say what the table "Deviations" says the plan does.

**Files:**

- Modify: `docs/superpowers/specs/2026-10-20-auto-update-design.md`

- [ ] **Step 1: Change the spec**

**In `docs/superpowers/specs/2026-10-20-auto-update-design.md`, replace:**

```markdown

**Prereleases** *(my call)*. A tag that is not a plain `vMAJOR.MINOR.PATCH` (for example `v0.2.0-rc1`) is published as a prerelease, which "latest" skips, so it reaches nobody. The version check is the same for it: the version fields must read `0.2.0-rc1`. A prerelease tag may therefore sit on any branch, for example the version pull request's branch before its version is set to the final `0.2.0`. Only plain version tags belong on `master`. The builder confirms that Tauri's NSIS bundle accepts a prerelease version; if it does not, the prerelease form is changed to one it accepts and the doc (§10) says which.

```

**with:**

```markdown

**Prereleases** *(my call)*. A tag that is not a plain `vMAJOR.MINOR.PATCH` (for example `v0.2.0-rc.1`) is published as a prerelease, which "latest" skips, so it reaches nobody. The version check is the same for it: the version fields must read `0.2.0-rc.1`. A prerelease tag may therefore sit on any branch, for example the version pull request's branch before its version is set to the final `0.2.0`. Only plain version tags belong on `master`. Tauri's NSIS bundle accepts a prerelease version: it keeps `0.2.0-rc.1` as the product version and writes `0.2.0.0` where Windows needs four numbers (tauri-bundler 2.12, `try_add_numeric_build_number`).

```

**In `docs/superpowers/specs/2026-10-20-auto-update-design.md`, replace:**

```markdown

**Nothing running.** Text: "Dots <version> is ready." Then a "What's new" link to the GitHub release page, a **Restart to update** button, and a **Later** button.

```

**with:**

```markdown

**Nothing running.** Text: "Dots <version> is ready." Then a "What's new" button, which opens the version's GitHub release page in the browser (Dots opens it; the webview opens no address of its own), a **Restart to update** button, and a **Later** button.

```

**In `docs/superpowers/specs/2026-10-20-auto-update-design.md`, replace:**

```markdown

- **Restart when it's done.** The note changes to "Will restart when Daily digest finishes." with a **Cancel** link. Dots restarts into the new version as soon as no run is working.
- **Stop it and restart.** Asks once, with the wording `RestartButton` uses today: a run that is working ends as failed ("interrupted"); runs waiting in the queue or for an approval continue after the restart. Then it restarts as above.

```

**with:**

```markdown

- **Restart when it's done** ("when they're done" for more than one dot). The note changes to "Will restart into Dots 0.2.0 when Daily digest finishes. New runs wait until after the update." with a **Cancel** button. Dots restarts into the new version as soon as no run is working.
- **Stop it and restart** ("Stop them" for more than one dot). Asks once, with the wording `RestartButton` uses today: a run that is working ends as failed ("interrupted"); runs waiting in the queue or for an approval continue after the restart. Then the install begins at once, as "Restart Dots" does: the working runs are not cancelled first (that would end them as cancelled, not as the dialog says), and the next start marks them interrupted.

```

**In `docs/superpowers/specs/2026-10-20-auto-update-design.md`, replace:**

```markdown

On Windows, Tauri's `Update::install` starts the NSIS installer and then ends the process (`std::process::exit`). The installer, not `app.restart()`, starts the new Dots, and `RunEvent::Exit` does not fire. So everything Dots must do happens before `install`. When no run is working any more (after the wait, or after "Stop it and restart" and the stopped runs have ended), in this order:

1. Shut the runtime down and **wait** until its tasks have stopped writing to `dots.db`, with a time limit *(the builder picks it)*. `shutdown()` alone does not wait.
2. Back up the database (below).
```

**with:**

```markdown

On Windows, Tauri's `Update::install` starts the NSIS installer and then ends the process (`std::process::exit`). The installer, not `app.restart()`, starts the new Dots, and `RunEvent::Exit` does not fire. So everything Dots must do happens before `install`. When no run is working any more (after the wait), or at once after "Stop it and restart", in this order:

1. Shut the runtime down and **wait** until its tasks have stopped writing to `dots.db`, at most 10 seconds (`Runtime::close`, which closes the database). `shutdown()` alone does not wait. If the database has not closed in time, no backup is made: as in step 3, Dots restarts into the current version, with the reason.
2. Back up the database (below).
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-10-20-auto-update-design.md
git commit -m "docs(spec): what the updates plan settled"
```

### Task 8: Smoke test

Write `docs/verification/2026-10-20-updates-smoke.md` in the form of `docs/verification/2026-10-19-context-library-smoke.md`: date, build, what each part covers, the commands, and each step's result.

**Part A (agent steps), against a debug build** (`bun run tauri build --debug --no-bundle`, into the shared target folder), started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d`, a `PATH` without any `claude` CLI, `WEBVIEW2_USER_DATA_FOLDER` moved, and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`, driven with `node tools/smoke/webview.mjs 9333`. The installed Dots is never stopped or touched; if it holds port 47321 the debug app runs without its server, which these steps do not need.

- A1. The app starts with the updater plugin: the window shows the Dots list, the log of the data folder has `update checks are off in this build or under DOTS_DATA_DIR`, and `invoke('update_note')` answers `{"state":"none"}`. `invoke('update_restart')` answers a `conflict` "no update is ready"; `invoke('update_open_notes')` the same.
- A2. The fatal-error page names a backup: quit, put a file `backups/dots-0.1.0-2026-10-20-091700.db` and a `dots.db` that is not a database (a text file) in a second fresh data folder, start: the page shows the error, the backup's path and `https://github.com/Aakshar-D/dots/releases`.
- A3. The strip in the real window: emit a note from the page through the event bridge (`$W eval "await window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event: 'update', payload: { state: 'ready', version: '0.2.0', working: [], error: null } })"`); the strip shows "Dots 0.2.0 is ready." with its three controls (`text`, `shot`); click **Later**: the core has no update, so it answers `{"state":"none"}` and the strip goes. If the bridge refuses the emit, record the refusal and skip A3: the component tests cover the strip.
- A4. `node tools/release/release.mjs check v0.1.0` passes on the branch; `check v0.2.0` names the six places.

**Part B (person steps)**, listed for the owner, none run by an agent:

- B1. Make the key pair, put the public key in `tauri.conf.json` (if Task 4 kept the throwaway key) and add the two secrets (`docs/releasing.md`).
- B2. Push a prerelease tag from a branch whose five version fields read `0.2.0-rc.1`; the workflow publishes a prerelease with the installer, its `.sig` and `latest.json`; the release check fails a tag that disagrees.
- B3. Install `0.2.0-rc.1` by hand; publish `0.2.0-rc.2`; start the installed Dots with `DOTS_UPDATE_ENDPOINT` set to its manifest; within a minute the strip offers it; with a dot running, "Restart when it's done" waits and a scheduled run stays queued; when the run ends Dots backs up to `%LOCALAPPDATA%\dots\backups`, installs in passive mode and opens on `0.2.0-rc.2` with its window shown, data intact; the backup opens.
- B4. "Stop it and restart" with a dot running: asks, installs, and the run is failed "interrupted" after the start.
- B5. A first plain release, once B1 to B4 pass.

- [ ] **Step 1: Run Part A and write the document**
- [ ] **Step 2: Commit**

```bash
git add docs/verification/2026-10-20-updates-smoke.md
git commit -m "docs(verification): the updates smoke"
```
