# Updates for Everyone (Part 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Everyone who runs Dots gets Settings › Updates (version, last check, Check for updates, What's new, Install updates by itself when idle, Repair Dots, Go back), a health check after each update that offers Go back and remembers bad versions, and a strip for a withdrawn version.
**Architecture:** The core gains one setting (`auto_install_updates`), a count of unfinished runs, and a small GitHub releases reader (`dots_core::releases`) with the withdrawn marker. The app keeps a record file `update-state.json` beside `dots.db` (what the last install left, pending health check, bad versions), extends `update.rs`'s pure state (bad versions, the idle rule, three install purposes: update, repair, go back) and `update_service.rs`'s wiring (check now, notes, repair and go-back downloads from a release's own `latest.json`, the idle loop, the 2-minute health timer), and manages `Updates` even when the runtime failed so the fatal page can go back. The UI adds the Settings section, a What's new dialog, three strips and Go back on the fatal page, and reports input activity and its first render.
**Tech Stack:** Rust 1.97 (tokio, chrono, serde, reqwest in the core, wiremock in core tests), Tauri 2.12 with tauri-plugin-updater 2.14; React 19.3, @tanstack/react-query 5.104, zod 4.6, radix-ui, Vitest 5.0 with the fake backend; Bun 1.4.3. No new crate or npm package.
**Spec:** docs/superpowers/specs/2026-10-27-updates-ui-design.md (sections 1 to 4 and 6; section 5 is part 2's plan)

## Global Constraints

- Repository root `C:/Users/shrad/OneDrive/Documents/LOQ/Dots/dots`, branch `feat/updates-ui` (at `d50f371` when this plan was written); commands run from the root (Git Bash). Never commit to `master`. `git add` names its files. No `Co-Authored-By` or other AI trailer in any commit message.
- Toolchain: every `cargo` command that builds tests runs with `export PATH="/c/Program Files/LLVM/bin:$PATH"` first; Bun at `~/.bun/bin` (`cd ui && ~/.bun/bin/bun ...`). Never run prettier. Low memory: `cargo` with `-j 4`, suites one at a time in the foreground.
- **Agents never install, uninstall or run an installer, and never touch the installed Dots or `%LOCALAPPDATA%\dots`** (update spec §11). Every install, repair and go-back path is tested behind the `InstallSteps` seam or a fake; smoke runs use a debug build with `DOTS_DATA_DIR` in the scratchpad, where update checks are off.
- Repository and endpoints: `releases::REPO = "Aakshar-D/dots"`, `releases::API = "https://api.github.com"`; a version's own manifest `https://github.com/Aakshar-D/dots/releases/download/v<version>/latest.json`; the withdrawn marker `releases::WITHDRAWN_MARKER = "<!-- dots:withdrawn -->"` (first line of a withdrawn release's body; part 2 writes it).
- Numbers: `update::IDLE_AFTER = 10 min` of no input; `update_record::HEALTH_LIMIT = 120 s`; the record file `update_record::RECORD_FILE = "update-state.json"` in the data dir; `startup::STAY_HIDDEN_MARK = "stay-hidden-after-update"`. The setting `auto_install_updates` defaults to `false`.
- Exact words (UI): Settings section **Updates**; status line "Dots <v> · up to date · checked <t>", "Dots <v> · <x> is ready", "Dots <v> · never checked"; in a copy without checks "Updates are off in this copy" and no other control; buttons **Check for updates**, **What's new**, **Repair Dots…**, **Go back to <v>…**; switch "Install updates by itself when idle" with hint "Off: a strip appears and you choose when to restart."; check outcomes "Up to date", "<v> is downloading…", "<v> is ready", or the error; Repair's confirm "Dots will download version <v> again, back up your data and reinstall it. Runs that are working will be stopped."; Repair unavailable "This version has no published installer to repair from."; Go back's confirm and strip "Dots <v> did not start properly. Go back to <old>? Your data from before the update comes back with it; anything done since the update is lost." (from Settings the first sentence is "Go back to <old>?"), buttons **Go back** / **Not now**; updated-itself strip "Dots updated itself to <v> at <t>." with **What's new** / **Dismiss**; withdrawn strip "The developers withdrew Dots <v>. Go back to <old>?" or, with nothing to go back to, "The developers withdrew Dots <v>. A fixed version will be offered when it is out."
- Names: setting `auto_install_updates`; core `Runtime::unfinished_runs() -> Result<usize>`, `dots_core::releases::{Release, fetch_release, WITHDRAWN_MARKER, REPO, API}`; app module `update_record`; commands `update_status()`, `update_check_now()`, `update_release_notes(version)`, `update_repair()`, `update_go_back()`, `update_dismiss_updated()`, `report_activity(composerHasText)`, `app_rendered()`; query key `keys.updateStatus` (`["update-status"]`). The contract fixture is only ever regenerated (`UPDATE_CONTRACT=1 cargo test -p dots-app contract`).
- How a step changes code: **Create `path`** (tests named with their assertions; code as signatures and duties); **In `path`, change …** (find the passage named; if it moved, apply it where it is now).
- Before each commit: the task's tests, then `cargo test -p dots-core --no-fail-fast` for a core change, `cargo test -p dots-app` for an app change (Tasks 1 to 3 may leave the contract test failing until Task 4 regenerates), `cargo clippy --workspace --all-targets -j 4 -- -D warnings`, `cargo fmt --check`; for a UI change `cd ui && ~/.bun/bin/bun run typecheck && ~/.bun/bin/bun run test`.
- Known failures, not caused here: `preview_schedule_lists_the_next_fire_times` (time zone); under load `approvals_test`, `gateway`, `browser_host`, `runtime_local`, `memory_server` tests (rerun alone first).

## Review Focus

1. **`update-state.json` missing, empty, corrupt, or from a newer Dots with fields this one does not know.** Expected: Dots starts as if nothing is pending, logs a warning, never fails its start over it. Test: `update_record::tests::a_missing_or_broken_record_reads_as_nothing_pending` (Task 2).
2. **The runtime starts but the window never says it rendered (a broken webview).** Expected: after 120 s the version is marked failed; the next start, still pending, offers Go back on the fatal page or strip. Test: `update_record::tests::a_start_that_never_rendered_fails_at_the_limit_and_the_next_start_offers_go_back` (Task 2).
3. **Go back when the update's backup file is gone** (pruned, or deleted by the person). Expected: Go back is not offered; nothing is installed. Test: `update_record::tests::go_back_needs_the_backup_file` (Task 2).
4. **A run starts while an idle install is downloading or about to begin.** Expected: the install does not begin; it waits for the next idle moment. Test: `update_service::tests::an_idle_install_rechecks_the_runs_under_the_turn_before_it_begins` (Task 5).
5. **A check finds a version this machine marked bad.** Expected: it is never downloaded or offered; a newer one is. Test: `update::tests::a_bad_version_is_never_wanted_but_a_newer_one_is` (Task 3).

---

### Task 1: Core: the setting, unfinished runs, and the releases reader

**Files:**
- Modify: `crates/dots-core/src/settings.rs`, `crates/dots-core/src/store/runs.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/lib.rs`
- Create: `crates/dots-core/src/releases.rs`, `crates/dots-core/tests/releases_test.rs`
- Test: the file that tests `Settings` defaults today, `crates/dots-core/tests/store_runs_test.rs`

**Interfaces:**
- Produces: `Settings.auto_install_updates: bool` (serde default `false`); `Store::unfinished_run_count(&self) -> Result<usize>` and `Runtime::unfinished_runs(&self) -> Result<usize>` (runs with status `queued`, `running` or `awaiting_approval`); `releases::Release { version: String, name: String, body: String, url: String, prerelease: bool, withdrawn: bool }` (`body` without the marker line); `releases::fetch_release(api: &str, version: &str) -> Result<Option<Release>>` (GET `{api}/repos/{REPO}/releases/tags/v{version}`, `Accept: application/vnd.github+json`, a `User-Agent`, no token; 404 is `Ok(None)`; `withdrawn` is `prerelease && body starts with WITHDRAWN_MARKER`).

- [ ] **Step 1: Write the failing tests**
  - `auto_install_updates_is_off_by_default_and_reads_from_old_settings` — `Settings::default().auto_install_updates == false`; a settings JSON without the field parses with it `false`.
  - `store_runs_test::unfinished_runs_count_queued_running_and_awaiting_approval` — one run in each of `queued`, `running`, `awaiting_approval`, `succeeded`, `failed`, `cancelled`: count is 3.
  - `releases_test::a_release_reads_its_notes_and_is_not_withdrawn` (wiremock: 200 with `{"tag_name":"v0.3.0","name":"Dots 0.3.0","body":"• New","html_url":"https://github.com/Aakshar-D/dots/releases/tag/v0.3.0","prerelease":false}`) → `Some(Release { version: "0.3.0", withdrawn: false, body: "• New", .. })`, and the request path is `/repos/Aakshar-D/dots/releases/tags/v0.3.0`.
  - `releases_test::a_prerelease_whose_body_starts_with_the_marker_is_withdrawn` → `withdrawn: true`, `body` without the marker line.
  - `releases_test::a_missing_release_is_none_and_a_server_error_is_an_error` (404 → `Ok(None)`; 500 → `Err`).
- [ ] **Step 2: Run them** — `cargo test -p dots-core -j 4 --test releases_test --test store_runs_test` and the settings test. Expected: compile errors (missing field, method, module).
- [ ] **Step 3: Implement** the field (with the others in `Settings` and its `Default`), the store count (one `SELECT COUNT(*)`), the runtime method (delegates), and `releases.rs` with the reqwest client the core already uses for watchers (timeout 15 s). Export `pub mod releases` in `lib.rs`.
- [ ] **Step 4: Run them again.** Expected: PASS; then the core suite, clippy, fmt.
- [ ] **Step 5: Commit** — `git add` the files above; `git commit -m "feat(core): the auto-install setting, unfinished runs and a release reader"`.

### Task 2: App: the update record and the health check

**Files:**
- Create: `apps/dots-app/src/update_record.rs`
- Modify: `apps/dots-app/src/lib.rs` (`mod update_record;`)

**Interfaces:**
- Produces:
  - `UpdateRecord { leaving: Option<String>, installing: Option<String>, backup: Option<PathBuf>, pending: bool, starts: u32, by_itself_at: Option<String>, bad: Vec<String> }` (serde, `#[serde(default)]`; `by_itself_at` RFC 3339 UTC, set only by an idle install).
  - `read(data_dir: &Path) -> UpdateRecord` (missing, empty or unparsable: `UpdateRecord::default()`, with a `tracing::warn!` for the unparsable case); `write(data_dir: &Path, record: &UpdateRecord) -> Result<(), String>` (write `update-state.json.tmp`, then rename over).
  - `enum Health { Fine, Checking, Failed { version: String, back_to: Option<String> } }` (`back_to` is `leaving` when `go_back_target` allows it).
  - `on_start(record: &mut UpdateRecord, current: &str, runtime_ok: bool) -> Health`: not pending → `Fine`; pending but `installing != current` (the install never happened) → clear `pending`, `Fine`; pending and `!runtime_ok` → `Failed`; pending and `starts >= 1` → `Failed` (the last start of this version never passed); else `starts += 1`, `Checking`.
  - `passed(record: &mut UpdateRecord)`: `pending = false`, `starts = 0`.
  - `failed(record: &mut UpdateRecord, version: &str)`: adds `version` to `bad` once; keeps `pending` so the next start still offers Go back.
  - `go_back_target(record: &UpdateRecord) -> Option<(String, PathBuf)>`: `leaving` and `backup` when both are set and the backup file exists.
  - `HEALTH_LIMIT: Duration = 120 s`.

- [ ] **Step 1: Write the failing tests** (in `update_record.rs`, `tempfile` dirs):
  - `a_missing_or_broken_record_reads_as_nothing_pending` — no file, an empty file, `"{not json"`, and `{"pending":false,"future_field":1}` all read with `pending == false`; `on_start(.., "0.3.0", true) == Health::Fine`.
  - `a_record_is_written_whole_and_read_back`.
  - `an_install_that_never_happened_clears_pending` — `installing: "0.3.1"`, `pending: true`, current `0.3.0` → `Fine`, `pending == false`.
  - `the_first_start_of_the_new_version_checks_and_a_pass_clears_it` — current `0.3.1` → `Checking`, `starts == 1`; `passed` → `pending == false`.
  - `a_start_that_failed_to_start_the_runtime_fails` → `Failed { version: "0.3.1", back_to: Some("0.3.0") }` when the backup exists.
  - `a_start_that_never_rendered_fails_at_the_limit_and_the_next_start_offers_go_back` — `on_start` → `Checking`; `failed(.., "0.3.1")` → `bad == ["0.3.1"]`, `pending` still true; a second `on_start` → `Failed { back_to: Some("0.3.0") }`.
  - `go_back_needs_the_backup_file` — backup path set but file absent → `go_back_target == None` and `Failed { back_to: None }`.
- [ ] **Step 2: Run** — `cargo test -p dots-app -j 4 update_record`. Expected: compile error (module missing).
- [ ] **Step 3: Implement** `update_record.rs` to the interface.
- [ ] **Step 4: Run again.** Expected: PASS; clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(app): the update record and the health check after an update"`.

### Task 3: App: bad versions, the idle rule and the three install purposes

**Files:**
- Modify: `apps/dots-app/src/update.rs` (and the callers in `update_service.rs` so it compiles)

**Interfaces:**
- Consumes: `update_record::{UpdateRecord, write}` (Task 2).
- Produces:
  - `UpdateState::new(error: Option<String>, bad: Vec<String>)` (callers updated); `wants` refuses a version in `bad`; `UpdateState::later_pressed(&self) -> bool`.
  - `enum Purpose { Update, Repair, GoBack }` (serde snake_case). `UpdateNote::Waiting` and `UpdateNote::Installing` gain `purpose: Purpose`. New `UpdateNote` variants: `UpdatedItself { version: String, at: String }`, `Unhealthy { version: String, back_to: Option<String> }`, `Withdrawn { version: String, back_to: Option<String> }`. `UpdateState::set_notice(Option<UpdateNote>)` keeps one of the three; `note()` returns, first that applies: Installing, Waiting, Unhealthy, Withdrawn, Ready, UpdatedItself, Failed, None.
  - `UpdateState::begin_other(&mut self, purpose: Purpose, label: &str, working: &[String]) -> Next` — Repair and Go back enter the same wait-or-install path as `restart` (wait while `working` is not empty, else install), with `purpose` carried into the notes.
  - `pub struct IdleInput { pub enabled: bool, pub ready: bool, pub later: bool, pub busy: bool, pub unfinished_runs: usize, pub composer_has_text: bool, pub input_idle: Duration, pub window_hidden: bool }` (`busy`: waiting or installing) and `pub fn idle_install_due(i: &IdleInput) -> bool` = `enabled && ready && !later && !busy && unfinished_runs == 0 && !composer_has_text && (window_hidden || input_idle >= IDLE_AFTER)`.
  - `InstallSteps` gains `fn record(&self, purpose: Purpose, backup: Option<&Path>) -> Result<(), String>` (for Update and Repair: `leaving = current, installing = target, backup, pending = true, starts = 0`, plus `by_itself_at` for an idle install; for GoBack: `pending = false` and the current version added to `bad`), `async fn set_aside_database(&self) -> Result<PathBuf, String>` (moves `dots.db`, `-wal`, `-shm` to `backups/dots-<current>-failed-<stamp>.db` and siblings), `async fn restore_backup(&self, backup: &Path) -> Result<(), String>` (copies the backup to `dots.db`), `fn undo_restore(&self, set_aside: &Path)` (puts the set-aside files back).
  - `pub async fn install(steps: &impl InstallSteps, purpose: Purpose, backup: Option<&Path>)`. Update and Repair: close → back up → record → mark → install. GoBack (with `backup` = the update's backup): close → set aside → restore → record → mark → install. On a failure after set aside: `undo_restore`, then `restart_current` with the reason. The existing Update failure rules stay (discard the backup no install followed).

- [ ] **Step 1: Write the failing tests** (extend `update.rs`'s tests and its `Steps` fake with the new methods, logging each step name):
  - `a_bad_version_is_never_wanted_but_a_newer_one_is` — `UpdateState::new(None, vec!["0.3.1"])`: `!wants("0.3.1")`, `wants("0.3.2")`.
  - `the_idle_install_is_due_only_when_everything_is_quiet` — table over each `IdleInput` field: all quiet and `input_idle = 10 min` → true; 9 min 59 s with the window shown → false; window hidden with 0 s → true; `later` → false; one unfinished run → false; composer text → false; `busy` → false; `!enabled` → false.
  - `later_stops_the_idle_install_until_the_next_start` — after `later()`, `later_pressed()`; a fresh `UpdateState` is not.
  - `repair_and_go_back_wait_for_working_runs_like_restart` — `begin_other(Repair, "0.3.0", ["digest"])` → `Next::Nothing` and `Waiting { purpose: Repair, .. }`; runs end → `Next::Install`, `Installing { purpose: Repair, .. }`.
  - `the_notes_come_in_order` — with Unhealthy and Ready both set, `note()` is Unhealthy; while Installing, Installing.
  - `an_update_records_then_marks_then_installs` — step log `["close","back_up","record","mark","install"]`.
  - `go_back_sets_the_database_aside_restores_the_backup_and_installs` — log `["close","set_aside","restore","record","mark","install"]`.
  - `a_go_back_whose_installer_cannot_start_puts_the_database_back` — install fails → log ends `["undo_restore","restart_current"]` and the reason starts "The update could not start".
- [ ] **Step 2: Run** — `cargo test -p dots-app -j 4 update::tests`. Expected: compile errors.
- [ ] **Step 3: Implement** to the interface; keep `update_service.rs` compiling with `Purpose::Update`, `None` backup and an empty bad list (Task 4 wires the rest).
- [ ] **Step 4: Run again.** Expected: PASS; clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(app): bad versions, the idle rule, repair and go back in the update state"`.

### Task 4: App: commands for status, check now, notes, repair and go back

**Files:**
- Modify: `apps/dots-app/src/update_service.rs`, `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs` (`command_list!`; manage `Updates` whenever the data dir is known), `apps/dots-app/src/contract.rs`, `apps/dots-app/src/state.rs` (`StartupError` gains `go_back: Option<String>`)
- Regenerate: `ui/src/api/__fixtures__/contract.json`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `UpdateStatus { version: String, checks_on: bool, checked_at: Option<String>, ready: Option<String>, last_error: Option<String>, go_back: Option<String>, auto_install: bool }` (`go_back`: `go_back_target`'s version).
  - `enum CheckOutcome { Off, UpToDate, Ready { version: String }, Failed { error: String } }` (serde tag `state`, snake_case).
  - `ReleaseNotes { version: String, body: String, url: String }`.
  - Commands: `update_status() -> UpdateStatus`; `update_check_now() -> CheckOutcome` (runs the same `check` now; a bad version found counts as up to date; every check, scheduled or not, records `checked_at` in memory when it got an answer); the "Publish versions of Dots…" link of spec §4.1 is part 2's; `update_release_notes(version: String) -> Option<ReleaseNotes>`; `update_repair() -> UpdateNote` (fails with "This version has no published installer to repair from." when `fetch_release` finds none); `update_go_back() -> UpdateNote` (works with no runtime: from the fatal page it skips the wait and the close; with no target, fails with "There is nothing to go back to.").
  - The per-version download: `download_version(app, version) -> Result<Downloaded, String>` builds the updater with the endpoint `https://github.com/Aakshar-D/dots/releases/download/v<version>/latest.json` and `.version_comparator(|_, _| true)`, so the same or an older signed version is accepted; the signature check stays the updater's own.
  - `StartupError.go_back`: the version Go back would install when the record offers it, else `None`.
- [ ] **Step 1: Write the failing tests**
  - In `update_service.rs` tests (the existing `Fake` controls, with a fake checker and a fake release reader): `check_now_reports_off_up_to_date_ready_or_the_error` (none, `0.3.1`, the bad `0.3.1` → up to date, an error).
  - `go_back_is_offered_only_with_a_backup_and_works_without_a_runtime` (record with backup → go-back path without `close`; without → the exact error).
  - `repair_needs_a_published_release` (reader `None` → the exact error).
  - In `state.rs`: `the_fatal_page_offers_go_back_when_the_record_does`.
  - Contract: the five commands in the list, with samples of each new type.
- [ ] **Step 2: Run** — `cargo test -p dots-app -j 4`. Expected: compile errors, then contract failures.
- [ ] **Step 3: Implement.** The checker and the release reader sit behind small traits so the tests need no network. The background check stays silent; only `update_check_now` returns its error. `Updates::new(data_dir, take_error, record.bad)` is managed whenever the data dir is known; only `spawn`'s checks need the runtime.
- [ ] **Step 4: Regenerate and run** — `UPDATE_CONTRACT=1 cargo test -p dots-app -j 4 contract`, then `cargo test -p dots-app -j 4`. Expected: PASS; clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(app): update status, check now, notes, repair and go back"`.

### Task 5: App: the idle install, the health timer, withdrawn versions and the stay-hidden mark

**Files:**
- Modify: `apps/dots-app/src/update_service.rs`, `apps/dots-app/src/startup.rs`, `apps/dots-app/src/lib.rs`, `apps/dots-app/src/commands.rs`, `apps/dots-app/src/contract.rs`
- Regenerate: the contract fixture

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces:
  - Commands `report_activity(composer_has_text: bool)` (records `Instant::now()` and the flag in `Updates`), `app_rendered()` (the health check's last condition; idempotent), `update_dismiss_updated()` (clears `by_itself_at` and the notice).
  - The idle loop: every 60 s, under `Updates.turn`, builds `IdleInput` (the setting from the runtime's `Settings`, `unfinished_runs()`, the main window hidden or minimised) and, when `idle_install_due`, rechecks `unfinished_runs()` right before it starts the install with purpose Update, the updater built with `.installer_arg("/S")` (see Step 3), and the stay-hidden mark instead of the restart mark.
  - The health timer: in `setup`, `update_record::on_start(&mut record, CARGO_PKG_VERSION, runtime.is_ok())`, then write the record; `Checking` → a task that waits for `app_rendered` (the runtime is up already), calls `passed` and writes, or at `HEALTH_LIMIT` calls `failed`, writes, and sets the `Unhealthy` notice; `Failed` → the `Unhealthy` notice (or `StartupError.go_back` when the runtime failed). A pass with `by_itself_at` set sets `UpdatedItself`.
  - Withdrawn: each scheduled check also calls `fetch_release(API, CARGO_PKG_VERSION)`; `withdrawn` → the `Withdrawn` notice with `back_to` from `go_back_target`.
  - `startup::STAY_HIDDEN_MARK`, `mark_stay_hidden(data_dir)`, `take_stay_hidden_mark(data_dir) -> bool`; `show_window(failed, hidden, restarted, stay_hidden)`: a failed start always shows; else `stay_hidden` keeps it hidden; else as today.
- [ ] **Step 1: Write the failing tests**
  - `an_idle_install_rechecks_the_runs_under_the_turn_before_it_begins` (fake: unfinished runs 0 when the loop decides, 1 at the recheck → no install).
  - `an_idle_install_is_quiet_and_keeps_the_window_hidden` (the fake records the installer arguments and that `mark_stay_hidden`, not `mark_restart`, was called).
  - `the_health_check_passes_once_the_window_has_rendered_and_fails_at_the_limit` (paused tokio time: render at 30 s → passed and record written; no render → at 120 s `bad` holds the version and the note is `Unhealthy`).
  - `a_withdrawn_running_version_offers_go_back` (fake release reader).
  - In `startup.rs`: `a_stay_hidden_mark_keeps_the_window_hidden_unless_the_start_failed`.
- [ ] **Step 2: Run** — `cargo test -p dots-app -j 4`. Expected: failures as written.
- [ ] **Step 3: Verify the quiet install before relying on it.** Read the NSIS template of the tauri-bundler version in `Cargo.lock` (`~/.cargo/registry/src/*/tauri-bundler-*/src/bundle/windows/nsis/installer.nsi`) and check what it does with both `/P` (from `installMode: passive`) and `/S`. If `/S` does not win, ledger a ruling and use what does. Also check whether the updater passes the old process's arguments on (`current_exe_args` in tauri-plugin-updater's `updater.rs`); the stay-hidden mark is needed either way.
- [ ] **Step 4: Implement**, regenerate the contract, run `cargo test -p dots-app -j 4`. Expected: PASS; clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(app): idle installs, the health timer and withdrawn versions"`.

### Task 6: UI: Settings › Updates and What's new

**Files:**
- Modify: `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/lib/query.ts`, `ui/src/test/fixtures.ts`, `ui/src/screens/SettingsScreen.tsx`, `ui/src/app/Root.tsx` (`app_rendered` once on mount), `ui/src/screens/home/ReplyArea.tsx` (composer emptiness for `report_activity`)
- Create: `ui/src/components/WhatsNewDialog.tsx`, `ui/src/lib/use-activity.ts`, `ui/src/screens/SettingsUpdates.test.tsx`, `ui/src/components/WhatsNewDialog.test.tsx`

**Interfaces:**
- Consumes: the commands and types of Tasks 4–5 (strict zod schemas matching the regenerated fixture).
- Produces: `UpdatesSection` in Settings, after **Claude usage**; `WhatsNewDialog({ version, open, onOpenChange })` (loads `update_release_notes`, renders the body with `components/Markdown.tsx`; links open in the browser as elsewhere; no notes → "No notes for <v>."); `useActivity()` (calls `report_activity` on `keydown`/`pointerdown` at most once per 30 s, and whenever the composer's emptiness changes).
- [ ] **Step 1: Write the failing tests** (`SettingsUpdates.test.tsx`, fake backend):
  - "show the version and when it last checked" → "Dots 0.3.0 · up to date · checked <localTime>".
  - "say updates are off in a copy without checks, and show nothing else" — `checks_on: false` → the text, no **Check for updates**.
  - "check now and say what it found" — `{state: "ready", version: "0.3.1"}` → "0.3.1 is ready"; while pending the button is disabled; a failed outcome shows its error inline.
  - "save the idle switch with the other settings" — toggle and Save → `auto_install_updates: true` in `update_settings`.
  - "repair asks first with the exact words" and "repair says when there is no installer".
  - "go back is offered only when status names a version, and asks first".
  - `WhatsNewDialog.test.tsx`: "shows the release's notes" and "says when a version has no notes".
  - "the window tells the core it rendered, once" (`callsTo("app_rendered")` has length 1 after rerenders).
- [ ] **Step 2: Run** — `cd ui && ~/.bun/bin/bun run test SettingsUpdates WhatsNew`. Expected: FAIL.
- [ ] **Step 3: Implement** with the exact words of the Global Constraints.
- [ ] **Step 4: Run again**, then typecheck and the whole UI suite. Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(ui): Settings › Updates and What's new"`.

### Task 7: UI: the strips and Go back on the fatal page

**Files:**
- Modify: `ui/src/app/Banners.tsx`, `ui/src/app/FatalPage.tsx`, `ui/src/app/Root.tsx` (passes `go_back` to the fatal page)
- Test: the existing banners test file, `ui/src/app/FatalPage.test.tsx`

**Interfaces:**
- Consumes: `UpdateNote` variants and `purpose` (Task 3), `update_go_back`, `update_dismiss_updated` (Tasks 4–5), `WhatsNewDialog` (Task 6).
- [ ] **Step 1: Write the failing tests**
  - "an unhealthy version offers Go back with the exact words" — **Go back** calls `update_go_back`; **Not now** hides it until the next note.
  - "a withdrawn version offers Go back, or says a fix will come when there is nothing to go back to".
  - "after updating itself, says so once with What's new and Dismiss" — Dismiss calls `update_dismiss_updated`.
  - "repair and go back waiting for runs say what they wait to do" — `Waiting { purpose: "repair" }` → "Dots will repair itself when <dots> finish."; `go_back` → "Dots will go back to <v> when <dots> finish." *(wording beyond the spec: the builder's, in the same voice)*.
  - FatalPage: "offers Go back when the start after an update failed" — `go_back: "0.3.0"` → **Go back to 0.3.0** calls `update_go_back`; without it, today's text only.
- [ ] **Step 2: Run** — `cd ui && ~/.bun/bin/bun run test Banners FatalPage`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again**, typecheck, whole UI suite. Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(ui): strips for going back, withdrawn versions and updating itself"`.

### Task 8: Docs, suites and the smoke

**Files:**
- Modify: `docs/releasing.md` (Settings › Updates, Repair, Go back, idle installs; the by-hand going back stays as the fallback), `docs/superpowers/specs/2026-10-27-updates-ui-design.md` (§8 "As built", part 1: deviations, builder's choices, smoke)
- Create: `docs/verification/2026-10-27-updates-part1-smoke.md`
- [ ] **Step 1: Suites** — core, app, UI, clippy, fmt, typecheck. Expected: only the known failures.
- [ ] **Step 2: Smoke** in a debug build with `DOTS_DATA_DIR` in the scratchpad (checks are off there): Settings › Updates says "Updates are off in this copy"; the idle switch saves; a hand-written `update-state.json` in the scratch data dir (`pending: true`, `installing` = the build's version, `starts: 1`, `leaving: "0.0.9"`, a backup file present) makes the next start show the Unhealthy strip. Never press Go back, Repair or any install in the smoke. axe in light and dark on Settings and with the strip; 420 px width.
- [ ] **Step 3: Write** the smoke record, the spec's §8, and the person steps still open: a real update, repair and go back on the person's own machine.
- [ ] **Step 4: Commit** — `git commit -m "docs: updates for everyone as built, and the smoke run"`.
