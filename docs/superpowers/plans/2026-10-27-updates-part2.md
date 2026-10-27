# Releases Page (Updates Part 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publishers (a vault GitHub token with push access to `Aakshar-D/dots`) get a Releases page that sets up the signing key once, publishes a trial only publishers' Dots take, shows the build live, drafts editable notes, promotes the trial's exact code to everyone, drops a trial, and withdraws a release; the release workflow runs the test suites first.
**Architecture:** A new core module `dots_core::publish` holds a small GitHub REST client (`publish::github`, reqwest, base URL injectable for wiremock), the key and secret crypto (`publish::keys`: minisign key pairs in Tauri's encoding, libsodium sealed boxes for GitHub secrets), the pure version and notes logic (`publish::versions`), and the flows that combine them (`publish::flow`: setup, trial, notes, promote, retry, drop, withdraw, page state). The app wraps the flows as commands, keeps the publisher token's vault name in `Settings`, checks publisher access at start, adds the trial channel to the update checks, and holds a made-but-unsent key only in memory. The UI adds `/releases` (rail icon for publishers only), the token chooser in Settings › Updates, and the setup, ready, trial and live views of mockup B. `.github/workflows/release.yml` gains a test job and publishes with `release-notes.md` when the tag has one.
**Tech Stack:** Rust 1.97 (reqwest 0.12, serde, base64 0.22, minisign 0.10 — already in `Cargo.lock` via dots-admin — and one new crate, `crypto_box` 0.9 with feature `seal`), Tauri 2.12 with tauri-plugin-updater 2.14 and tauri-plugin-dialog; React 19.3, react-query 5.104, zod 4.6, Vitest 5.0 with the fake backend; GitHub Actions; Node for `tools/release/release.mjs`.
**Spec:** docs/superpowers/specs/2026-10-27-updates-ui-design.md (section 5, the parts of 1–3 and 6 it relies on, and §8 for what part 1 built)

## Global Constraints

- Branch `feat/updates-ui`, after part 1 and its final-review fixes. Commands from the repo root (Git Bash). `git add` names its files. No AI trailer in commits. `cargo` with `export PATH="/c/Program Files/LLVM/bin:$PATH"` and `-j 4`; Bun at `~/.bun/bin`; never prettier; suites one at a time.
- **No real GitHub call, ever, from a test or the smoke.** Every flow is tested against wiremock; the smoke uses a token that has no access, or none. Publishing to the real repository (setup, trial, promote, withdraw) is a person step, each only on the person's word. Agents never install anything (update spec §11).
- **The token and the private key never appear in a log, an event, an error shown to the UI, the database outside the vault, or a test snapshot.** GitHub error text passes through `publish::scrub(text, secrets)` before it leaves the core. The made key lives only in the app's memory until step 3 sends it, then is dropped.
- Repository fixed: `releases::REPO = "Aakshar-D/dots"`; API `releases::API`. Branch `release/<version>`; tags `v<version>-rc.<n>` and `v<version>`; file `release-notes.md` at the repository root; secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; the prototype key is `PROTOTYPE_PUBKEY` from `tools/release/release.mjs` (copied as `publish::keys::PROTOTYPE_PUBKEY`); the withdrawn marker `releases::WITHDRAWN_MARKER`.
- Key encoding as `tauri signer generate` writes it: the private key is base64 (standard) of minisign's secret-key box text, encrypted with the publisher's password (minisign scrypt); the public key in `plugins.updater.pubkey` is base64 of the public-key box text. Password at least 12 characters.
- Token: a classic token whose `X-OAuth-Scopes` header lists both `repo` and `workflow`, or a fine-grained token (no such header); either way `GET /repos/{REPO}` must say `permissions.push: true`.
- The five version fields (spec §3); in `Cargo.lock` only the `version` line right after `name = "dots-app"` and `name = "dots-core"`.
- Exact words (UI): rail icon and page title **Releases**; setup "Releases aren't set up yet." with steps **Make the signing key**, **Save a backup** ("If this key and its password are both lost, no installed Dots can ever take another update."; **Save key file…**; tick "I have saved the key file and its password somewhere other than this PC"), **Put it in GitHub's secrets**, **Put the public key into Dots** (PR title "Use the release signing key"); half set up: "GitHub already has a signing key. Ask whoever set it up for the public key, or remove both secrets on GitHub to start again."; ready: "Live for everyone:" (or "nothing published yet"), "Changes on master since it:", **Version**, **Publish trial** with confirm "Publish <v>-rc.<n> from master as it is now? It goes only to publishers' Dots."; stages waiting, testing, building, signing, published, failed, with **View on GitHub**; "master has <n> changes newer than this trial; publish a new trial to include them" ("1 change" for one); **What's new**, **Save notes**; **Promote <v> to everyone** with confirm "Every Dots will be offered <v> within six hours. It is exactly the code of <v>-rc.<n>."; **Retry**; **Drop trial**; **Withdraw <v>…** with confirm "No more Dots will take <v>. Dots already on it will be offered to go back to <previous>."; token chooser hint "A classic token with the repo and workflow scopes."; Settings › Updates link "Publish versions of Dots…" (publisher) or "Publish versions of Dots (for its developers)…" (anyone else).
- Polling: the page reads the workflow run at most every 10 s while it is open and a run is in progress.
- Known failures, not caused here: as in part 1's plan.

## Review Focus

1. **A secret already exists on GitHub while master still has the prototype key (or the other way round).** Expected: the setup does nothing and says the half-set-up sentence; no secret is ever replaced. Test: `publish_flow_test::a_half_set_up_repository_is_left_alone` (Task 4).
2. **Promote when master moved, or when its version fields changed meanwhile.** Expected: Promote still ships the trial's tree; if the merge into master conflicts, nothing is tagged and the page says why. Test: `publish_flow_test::promote_ships_the_trial_tree_and_stops_on_a_conflicting_merge` (Task 4).
3. **A GitHub error whose text contains the token (or the key).** Expected: the text shown has it replaced by `[hidden]`. Test: `publish_github_test::errors_never_carry_the_token` (Task 1).
4. **The token lacks `workflow` scope, or its account has no push access.** Expected: Dots is not a publisher; the chooser says which is missing; the rail icon does not show. Test: `publish_github_test::a_token_without_workflow_or_push_is_not_a_publisher` (Task 1).
5. **A trial of a version whose tag already exists (a failed promote), or a version not above the live one.** Expected: refused with a message; the next free version is suggested. Test: `publish_versions_test::a_version_must_be_new_and_above_the_live_one` (Task 3).

---

### Task 1: Core: the GitHub client for publishing

**Files:**
- Create: `crates/dots-core/src/publish/mod.rs`, `crates/dots-core/src/publish/github.rs`, `crates/dots-core/tests/publish_github_test.rs`
- Modify: `crates/dots-core/src/lib.rs` (`pub mod publish;`)

**Interfaces:**
- Produces:
  - `publish::scrub(text: &str, secrets: &[&str]) -> String` (each non-empty secret replaced by `[hidden]`).
  - `GitHub::new(api: &str, token: &str) -> Result<GitHub>`. Every call sends `Authorization: Bearer <token>`, `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, a User-Agent; a non-2xx answer is `Err(Error::Other(scrub("GitHub answered <status>: <message>", [token])))`.
  - `access(&self) -> Result<Access>`; `Access { login: String, push: bool, scopes: Option<Vec<String>> }` (`GET /user` for login and `X-OAuth-Scopes`; `GET /repos/{REPO}` for `permissions.push`); `Access::publisher_problem(&self) -> Option<&'static str>`: `None` when publishing works, else "This token cannot push to Aakshar-D/dots." or "This token needs the repo and workflow scopes.".
  - Secrets: `secret_exists(&self, name) -> Result<bool>`; `secrets_key(&self) -> Result<(String /*key_id*/, String /*base64 key*/)>`; `put_secret(&self, name, key_id, encrypted_b64) -> Result<()>` — first checks `secret_exists` and returns `Err` "<name> already exists on GitHub." when it does.
  - Contents and git data: `file(&self, path, reference) -> Result<Option<String>>` (decoded text, `None` on 404); `head(&self, branch) -> Result<String>`; `commit_files(&self, parent: &str, files: &[(String, String)], message: &str) -> Result<String>` (blobs, a tree on the parent's tree, a commit; its sha); `set_branch(&self, branch, sha, force: bool) -> Result<()>`; `delete_branch(&self, branch) -> Result<()>`; `tag(&self, tag, sha) -> Result<()>`; `tag_exists(&self, tag) -> Result<bool>`; `merge(&self, base, head, message) -> Result<MergeOutcome>` (`POST /repos/{REPO}/merges`: 201 `Merged(sha)`, 204 `AlreadyMerged`, 409 `Conflict`); `open_pull(&self, head, base, title, body) -> Result<(u64, String)>`; `pull_merged(&self, number) -> Result<bool>`.
  - History: `merged_pulls_since(&self, base: Option<&str>, head: &str) -> Result<Vec<MergedPull>>` (`MergedPull { number: u64, title: String }`, newest first; from `GET compare/{base}...{head}` merge commit messages "Merge pull request #N from …\n\n<title>"; with no base, the 30 newest merged PRs into master); `commits_ahead(&self, base_sha, branch) -> Result<u32>`.
  - Releases and runs: `releases(&self) -> Result<Vec<ReleaseInfo>>` (`ReleaseInfo { id, tag, name, body, prerelease, draft, url, published_at }`, newest first); `edit_release(&self, id, name: Option<&str>, body: Option<&str>, prerelease: Option<bool>) -> Result<()>`; `run_for_tag(&self, tag) -> Result<Option<RunInfo>>` (`RunInfo { id, status, conclusion, url, jobs: Vec<JobInfo> }`, `JobInfo { name, status, conclusion, steps: Vec<(name, status, conclusion)> }`); `rerun(&self, run_id) -> Result<()>`.
- [ ] **Step 1: Write the failing tests** (`publish_github_test.rs`, wiremock; each asserts the method, path and headers it relies on and the parsed result):
  - `a_publisher_token_reads_its_login_scopes_and_push` → `publisher_problem() == None`.
  - `a_token_without_workflow_or_push_is_not_a_publisher` — scopes `repo` only → "This token needs the repo and workflow scopes."; push false → "This token cannot push to Aakshar-D/dots."; no scopes header with push true → `None`.
  - `errors_never_carry_the_token` — a 401 whose message echoes the token → `[hidden]`, never the token.
  - `put_secret_refuses_an_existing_secret` (no PUT received).
  - `commit_files_builds_blobs_a_tree_and_a_commit_on_the_parent` (`base_tree`, `parents == [parent]`).
  - `merge_reports_merged_already_merged_and_conflict`.
  - `merged_pulls_since_reads_merge_commits_newest_first`.
  - `a_run_for_a_tag_reads_its_jobs_and_steps`.
- [ ] **Step 2: Run** — `cargo test -p dots-core -j 4 --test publish_github_test`. Expected: compile errors.
- [ ] **Step 3: Implement** `publish/mod.rs` (each later module declared in its task) and `github.rs`. The contents API returns base64 with newlines; remove them before decoding.
- [ ] **Step 4: Run again.** Expected: PASS; core suite, clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(core): a GitHub client for publishing Dots"`.

### Task 2: Core: the signing key and GitHub's sealed secrets

**Files:**
- Create: `crates/dots-core/src/publish/keys.rs`, `crates/dots-core/tests/publish_keys_test.rs`
- Modify: `crates/dots-core/Cargo.toml` (`minisign = "0.10"`, `crypto_box = { version = "0.9", features = ["seal"] }`), `crates/dots-core/src/publish/mod.rs`

**Interfaces:**
- Produces: `MadeKey { pub private_b64: String, pub public_b64: String }` (Debug prints neither); `make_key(password: &str) -> Result<MadeKey>` (fewer than 12 characters: "The password needs at least 12 characters."); `seal(public_key_b64: &str, value: &str) -> Result<String>` (libsodium `crypto_box_seal`, base64 out); `PROTOTYPE_PUBKEY: &str`; `pubkey_in_conf(tauri_conf_json: &str) -> Option<String>`; `with_pubkey(tauri_conf_json: &str, public_b64: &str) -> Result<String>` (only `plugins.updater.pubkey`'s value changes).
- [ ] **Step 1: Write the failing tests**
  - `a_made_key_signs_what_its_public_key_verifies` (sign with the decoded secret key and password, verify with the decoded public key, using `minisign`).
  - `a_short_password_is_refused` (11 characters → the exact error).
  - `a_sealed_secret_opens_with_the_matching_secret_key` (a `crypto_box::SecretKey` made in the test opens `seal(its public b64, "value")` to "value").
  - `the_pubkey_is_read_and_replaced_without_touching_the_rest` (on a copy of the real `apps/dots-app/tauri.conf.json`: `pubkey_in_conf == Some(PROTOTYPE_PUBKEY)`; `with_pubkey` changes one line).
  - `debug_shows_no_key`.
- [ ] **Step 2: Run** — `cargo test -p dots-core -j 4 --test publish_keys_test`. Expected: compile errors.
- [ ] **Step 3: Implement**, following `crates/dots-admin/src/keys.rs` for minisign's box text; base64 the box text with the standard engine, as `tauri signer generate` does.
- [ ] **Step 4: Run again.** Expected: PASS; core suite, clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(core): the release signing key and sealed GitHub secrets"`.

### Task 3: Core: versions and the notes draft

**Files:**
- Create: `crates/dots-core/src/publish/versions.rs`, `crates/dots-core/tests/publish_versions_test.rs`
- Modify: `crates/dots-core/src/publish/mod.rs`

**Interfaces:**
- Produces: `VERSION_FILES: [&str; 5]`; `set_version(path: &str, text: &str, version: &str) -> Result<String>` (every other byte kept); `parse(v: &str) -> Option<(u64, u64, u64)>` (plain only); `next_minor(live: Option<&str>) -> String` (`None` → "0.2.0"); `check_new(version: &str, live: Option<&str>, tags: &[String]) -> Result<(), String>` ("A version is MAJOR.MINOR.PATCH, for example 0.3.0.", "<v> is not above the live <live>.", "<v> was published before; use <next>." when `v<v>` exists); `next_rc(version: &str, tags: &[String]) -> u32`; `draft_notes(pulls: &[MergedPull]) -> String` (`- <title> (#<n>)` per pull, a leading `type(scope): ` or `type: ` removed, first letter upper-cased).
- [ ] **Step 1: Write the failing tests**
  - `the_five_fields_are_set_and_nothing_else_moves` (copies of the real five files; `Cargo.lock` changes exactly two lines).
  - `a_version_must_be_new_and_above_the_live_one` (`0.3`, `0.3.0-rc.1` → format error; `0.2.0` with live `0.2.0` → not above; `0.3.0` with tag `v0.3.0` → "0.3.0 was published before; use 0.4.0."; `0.3.0` with live `0.2.0` → ok).
  - `the_next_trial_number_follows_the_tags`.
  - `notes_are_drafted_from_pull_titles`.
  - `the_next_minor_follows_the_live_version`.
- [ ] **Step 2: Run** — `cargo test -p dots-core -j 4 --test publish_versions_test`. Expected: compile errors.
- [ ] **Step 3: Implement** with plain text edits.
- [ ] **Step 4: Run again.** Expected: PASS; core suite, clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(core): release versions and the notes draft"`.

### Task 4: Core: the publishing flows

**Files:**
- Create: `crates/dots-core/src/publish/flow.rs`, `crates/dots-core/tests/publish_flow_test.rs`
- Modify: `crates/dots-core/src/publish/mod.rs`

**Interfaces:**
- Consumes: Tasks 1–3; `releases::WITHDRAWN_MARKER`.
- Produces:
  - `Setup { Done, Needed { secrets: bool, pubkey: bool }, HalfDone { secrets: bool, pubkey: bool } }`; `setup_state(gh) -> Result<Setup>` (secrets: both exist; pubkey: master's conf holds a non-prototype key).
  - `put_key_in_secrets(gh, key: &MadeKey, password: &str) -> Result<()>` (only when `Needed`); `open_pubkey_pull(gh, public_b64) -> Result<(u64, String)>` (branch `release-key` from master with `with_pubkey`, PR "Use the release signing key"; refused when master already has a non-prototype key).
  - `PageState { setup, live: Option<ReleaseInfo>, trial: Option<Trial>, changes: Vec<MergedPull>, suggested: String }`, `Trial { version, rc, tag, release: Option<ReleaseInfo>, run: Option<RunInfo>, newer_on_master: u32, notes: String }`; `page_state(gh) -> Result<PageState>` (live: newest non-prerelease, non-draft, not withdrawn; trial: the newest `v*-rc.*` tag whose `release/<v>` branch exists and whose version has no plain tag; notes: `release-notes.md` on the branch, else the draft).
  - `publish_trial(gh, version) -> Result<Trial>` (`check_new`; master's head C; `release/<v>` set to C (force); one commit setting the five fields to `<v>-rc.<n>` and writing `release-notes.md` with the draft; tag `v<v>-rc.<n>` on it).
  - `save_notes(gh, version, notes) -> Result<()>` (a commit changing only `release-notes.md`; the trial's release body edited to match when it exists).
  - `promote(gh, version) -> Result<()>` (a commit setting the fields to `<v>`; `merge(master, release/<v>)`: `Conflict` → `Err` "master could not take <v>: its version fields changed. Publish a new trial." and nothing tagged; else tag `v<v>` on the version commit); `retry(gh, tag)`; `drop_trial(gh, version)` and `withdraw(gh, release)` (release made a prerelease named "Dots <v> (withdrawn)", body prefixed with `WITHDRAWN_MARKER` and a new line; drop also deletes the branch).
  - `Stage { Waiting, Testing, Building, Signing, Published, Failed { step: String } }`; `stage_of(run, release) -> Stage`.
- [ ] **Step 1: Write the failing tests** (wiremock with an in-test repository helper that answers each call and records what it was sent):
  - `setup_is_needed_done_or_half_done`.
  - `a_half_set_up_repository_is_left_alone` (no PUT received).
  - `a_trial_commits_the_versions_and_notes_on_its_branch_and_tags_it`.
  - `a_second_trial_takes_the_next_number_from_masters_new_head`.
  - `saving_notes_changes_only_the_notes_file_and_the_release_body`.
  - `promote_ships_the_trial_tree_and_stops_on_a_conflicting_merge`.
  - `drop_and_withdraw_mark_the_release_withdrawn`.
  - `the_stage_follows_the_run_and_the_release` (table).
- [ ] **Step 2: Run** — `cargo test -p dots-core -j 4 --test publish_flow_test`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again.** Expected: PASS; core suite, clippy, fmt.
- [ ] **Step 5: Commit** — `git commit -m "feat(core): publishing flows: setup, trial, notes, promote, drop and withdraw"`.

### Task 5: The release workflow tests first and takes the notes file

**Files:**
- Modify: `.github/workflows/release.yml`, `tools/release/release.mjs`, `tools/release/release.test.mjs`

**Interfaces:**
- Produces: a job `test` on `windows-latest` (the same pinned checkout, Bun, Rust and cache actions as `release`; then `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p dots-core`, `cargo test -p dots-app`, `cd ui && bun install --frozen-lockfile && bun run typecheck && bun run test`); `release` gets `needs: test`. Publishing uses `--notes-file release-notes.md` when the file exists at the tag, else `--generate-notes`. `release.mjs manifest <tag> <installer> <out> [notes-file]` puts the notes in `latest.json`'s `notes` when given.
- [ ] **Step 1: Write the failing test** in `release.test.mjs`: "the manifest carries the notes file when one is given" (and "Dots <v>" without it).
- [ ] **Step 2: Run** — `node --test tools/release/release.test.mjs`. Expected: FAIL.
- [ ] **Step 3: Implement.** If `preview_schedule_lists_the_next_fire_times` depends on the machine's time zone (it fails locally), run the core tests in the job with `-- --skip preview_schedule_lists_the_next_fire_times` and ledger it.
- [ ] **Step 4: Run** the node tests, then `node tools/release/release.mjs check v0.1.0` (still refuses the prototype key). Expected: PASS and the same refusal.
- [ ] **Step 5: Commit** — `git commit -m "ci: the release workflow runs the tests first and publishes the notes file"`.

### Task 6: App: publisher, trial channel and the commands

**Files:**
- Modify: `crates/dots-core/src/settings.rs` (`publish_token: Option<String>`, a vault entry's name, default `None`), `apps/dots-app/src/update_service.rs`, `apps/dots-app/src/commands.rs`, `apps/dots-app/src/lib.rs`, `apps/dots-app/src/contract.rs`
- Create: `apps/dots-app/src/publishing.rs`
- Regenerate: the contract fixture

**Interfaces:**
- Consumes: Tasks 1–4; `Runtime::vault().personal_value(name)`.
- Produces:
  - `publishing::Publisher` (Tauri-managed): `PublisherStatus { token: Option<String>, state: PublisherState }`, `PublisherState { None, Checking, Publisher { login }, Refused { reason } }` (serde tag `state`), checked at start and when the setting changes; `pending_key: Mutex<Option<MadeKey>>`.
  - Commands (each but the first two refuse with "Dots is not set up to publish." unless the state is `Publisher`): `publisher_status`, `publisher_check`, `releases_page() -> PageState`, `release_make_key(password) -> String` (public b64; keeps the key in `pending_key`), `release_save_key(path)` (writes `private_b64` where the save dialog chose), `release_put_secrets(password)`, `release_open_key_pull() -> (u64, String)`, `release_publish_trial(version) -> Trial`, `release_save_notes(version, notes)`, `release_promote(version)`, `release_retry(tag)`, `release_drop_trial(version)`, `release_withdraw(version)`. Each reads the token from the vault for the call.
  - Trial channel in `update_service::check_once`: a publisher also lists releases and, when the newest prerelease is not withdrawn, not bad and newer than both the running version and the latest release, checks that release's own `latest.json` instead (`trial_endpoint(releases, running, latest, bad) -> Option<Url>`).
  - `UpdateStatus.publisher: bool`.
- [ ] **Step 1: Write the failing tests**
  - `publishing::tests::the_publisher_state_follows_the_token` (fake access: none → `None`; push and scopes → `Publisher`; missing workflow → `Refused` with the core's sentence).
  - `publishing::tests::a_made_key_is_kept_until_it_is_in_the_secrets_and_then_dropped`.
  - `update_service::tests::a_publisher_takes_the_newest_good_trial` (newer prerelease → its manifest; withdrawn, bad or older → none; not a publisher → none).
  - Contract: the new commands; samples of `PublisherStatus`, `PageState` (setup needed, a trial building, a live release), `Trial`.
- [ ] **Step 2: Run** — `cargo test -p dots-app -j 4`. Expected: compile errors, then contract.
- [ ] **Step 3: Implement**, then regenerate the contract.
- [ ] **Step 4: Run** app tests, the core settings test, clippy, fmt. Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(app): publishers, the trial channel and the release commands"`.

### Task 7: UI: the Releases page

**Files:**
- Create: `ui/src/screens/ReleasesScreen.tsx`, `ui/src/screens/ReleasesScreen.test.tsx`, `ui/src/components/VaultChooser.tsx` (moved out of `WatchersTab.tsx`, which imports it)
- Modify: `ui/src/api/types.ts`, `ui/src/api/commands.ts`, `ui/src/api/contract.test.ts`, `ui/src/lib/query.ts`, `ui/src/test/fixtures.ts`, `ui/src/app/routes.tsx` (`/releases`), `ui/src/app/Layout.tsx` (rail icon only for a publisher), `ui/src/screens/UpdatesSection.tsx` (the publishing link, the token chooser, its hint and its refusal reason)

**Interfaces:**
- Consumes: Task 6's commands and types.
- [ ] **Step 1: Write the failing tests** (`ReleasesScreen.test.tsx`, fake backend):
  - "shows the rail icon only to a publisher".
  - "walks through the setup in order and ticks each step" (12-character password; **Save key file…** calls the save dialog and `release_save_key`; the tick enables step 3; step 4 opens the PR and links it).
  - "says a half set-up repository is left alone" (exact sentence; no step buttons).
  - "publishes a trial after asking with the exact words" and "refuses a version the core refuses".
  - "shows the build's stage and links to the run".
  - "says how many changes master has past the trial" ("1 change", "2 changes").
  - "saves the notes and promotes after asking" (exact confirm).
  - "offers Retry after a failed promote, and Drop trial and Withdraw ask first".
  - In `SettingsUpdates.test.tsx`: "a non-publisher sees the developers' link, which opens the token chooser" and "a publisher's link opens the Releases page".
- [ ] **Step 2: Run** — `cd ui && ~/.bun/bin/bun run test ReleasesScreen SettingsUpdates`. Expected: FAIL.
- [ ] **Step 3: Implement** (mockup B; poll `releases_page` every 10 s while a run is in progress).
- [ ] **Step 4: Run again**, typecheck, whole UI suite. Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(ui): the Releases page"`.

### Task 8: Docs, suites and the smoke

**Files:**
- Modify: `docs/releasing.md` (the Releases page first; the by-hand steps kept as the fallback; §5.11's first-release steps), `docs/superpowers/specs/2026-10-27-updates-ui-design.md` (§9 "As built, part 2")
- Create: `docs/verification/2026-10-27-updates-part2-smoke.md`
- [ ] **Step 1: Suites** — core, app, UI, node release tests, clippy, fmt, typecheck. Expected: only the known failures.
- [ ] **Step 2: Smoke** in a debug build with `DOTS_DATA_DIR` in the scratchpad: no token → Settings › Updates shows the developers' link and no rail icon; a vault entry with a made-up token → the check is refused with the core's sentence and no rail icon. Nothing is sent to the real repository with a real token. axe light and dark on Settings and on `/releases` through the dev server with the fake backend's states; 420 px.
- [ ] **Step 3: Write** the smoke record, the spec's §9, and the person steps in order: (1) a classic token with `repo` + `workflow` in a dev copy; (2) run the setup; (3) merge the key PR; (4) build master and install it by hand; (5) publish the first trial; (6) promote it.
- [ ] **Step 4: Commit** — `git commit -m "docs: the Releases page as built, the first-release steps and the smoke"`.
