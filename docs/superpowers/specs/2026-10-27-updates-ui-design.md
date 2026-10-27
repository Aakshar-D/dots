# Updates in the Dots window: design

Status: built (part 1, section 4, and part 2, section 5). Builds on `2026-10-20-auto-update-design.md` ("the update spec"), which stays in force except where §2 says otherwise.

## 1. Intent

The person asked for "something like updater in the dots ui that sets up updates". Today a release only happens through `git tag` commands and a signing key nobody has set up yet, and a person running Dots can do nothing about updates except wait for the strip. This phase puts both sides in the window:

- **Everyone** who runs Dots can see their version, check now, read what's new, let Dots update itself while idle, repair a broken install, and go back when a new version will not start.
- **The developers** (the person and Aakshar) publish from inside Dots: a one-time guided setup of the signing key, a trial version only they receive, then **Promote** to everyone, and **Withdraw** if a release turns out broken.

The person's decisions, in the order given:

- Who it is for: both sides.
- Who may publish: only people with write access to `Aakshar-D/dots` ("only me and Aakshar").
- Publishing: trial first, then everyone.
- Everyone's side: version and **Check for updates**, **What's new**, auto-update on or off. (A switch for everyone to take trial versions was not chosen.)
- Trials reach publishers' Dots automatically, nobody else's.
- Auto-update on: Dots downloads the new version and restarts itself into it when idle. Off by default on a fresh install.
- Layout: an Updates section in Settings for everyone; a Releases page of its own for publishers (mockup B, kept in `.superpowers/brainstorm/`).
- Signing in for publishing: a GitHub token from the vault, as watchers use.
- Promote ships exactly the trial's code; changes merged after the trial wait for the next trial.
- The one-time setup is guided by Dots.
- Notes: drafted from the merged pull requests, edited by the publisher before Promote.
- From a look at another agent app's code graph (`Graph agents/`, "Linked Agents"): a health check after an update with a memory of bad versions, tests that must pass before a trial is built, Withdraw a release, and Repair Dots.
- The first release: the Releases page works in a copy built from the repo too, and the person also builds and installs Dots by hand once (§5.11).
- "Later" on an update stops the idle install until Dots next starts.
- Build order: two parts, one design. Part 1 is everyone's side and the safety net (§4); part 2 is the Releases page (§5). Each gets its own plan and pull request.

Success: the first release goes out from the Releases page without a terminal; every later version reaches publishers as a trial, then everyone with one click; a version that will not start on someone's machine sends them back to the one that did, with their data, and is never offered to that machine again.

## 2. Goals, non-goals, anti-goals

**Goals**

- Settings › Updates for everyone: version, last check, Check for updates, What's new, Install updates by itself when idle (off by default), Repair Dots.
- After an update, a health check; on failure, Go back (the previous installer plus the database backup), and the failed version is never offered to that machine again.
- A Releases page for publishers: guided key setup, Publish trial, live progress, editable notes, Promote, Drop trial, Withdraw.
- The release workflow runs the test suites before it builds anything.

**Non-goals**

- Usage or adoption counts (how many copies run each version). Dots sends nothing home today; that stays so.
- macOS or Linux builds.
- Publishing from a fork, or to a repository other than `Aakshar-D/dots`.
- Users other than publishers taking trial versions.
- In-app editing of anything but the release notes (no changelog files, no version history editing).

**Anti-goals**

- An update installed that was not signed with the owner's key: still true for trials, repairs and going back.
- A signing key or GitHub secret replaced by the setup. If one exists, the setup stops and says so.
- The private key, its password or a GitHub token written to a log, an event, the database outside the vault, or the UI after the step that needs it.
- Dots restarting itself while the person is using the window or while any run is working, queued to start, or waiting for an approval.
- A data upgrade or a go-back that loses data with no copy to return to.
- A publish action that happens without the publisher pressing its button and confirming.
- Nagging: Check for updates shows its error inline; background check failures stay silent (the update spec's §5 rule holds).

**What this changes in the update spec**

- Anti-goal "Dots restarts on its own … without the person clicking": now allowed only when the person turned **Install updates by itself when idle** on. That switch is the click.
- Non-goal "Release channels": a trial channel exists, for publishers only.
- §8 "Going back is by hand": Dots now goes back by itself when asked (§4.4). The by-hand steps in `docs/releasing.md` stay as the fallback.

## 3. Facts the design rests on

- `apps/dots-app/src/update.rs` (866 lines) checks `https://github.com/Aakshar-D/dots/releases/latest/download/latest.json` after start and every 6 hours, downloads in the background, keeps the state behind the strip (`UpdateState`: ready, later, waiting, installing), and installs by the update spec's §7 order (close the runtime, back up `dots.db` to `backups/`, mark the restart, hand over to the NSIS installer). `DOTS_UPDATE_ENDPOINT` overrides the endpoint; `DOTS_DATA_DIR` or a debug build turns checks off.
- `ui/src/app/Banners.tsx` shows the strip.
- `tauri.conf.json` still holds the prototype's throwaway updater key. `node tools/release/release.mjs check` refuses every tag until it is replaced: "has the prototype's updater key; put the owner's public key in plugins.updater.pubkey first". The repository has no releases, no tags and no Actions secrets yet.
- `.github/workflows/release.yml` runs on `v*` tags on `windows-latest`: checks the tag against the five version fields, checks a plain tag is on `master`, builds, signs in a step of its own, publishes the release with the installer, `.sig` and `latest.json`. It runs no test suite today.
- The five version fields: `apps/dots-app/tauri.conf.json`, `apps/dots-app/Cargo.toml`, `crates/dots-core/Cargo.toml`, `ui/package.json`, and `Cargo.lock` (`dots-app`, `dots-core`).
- The vault holds secrets by name; watchers choose a GitHub token from it with "Add to vault…" (`crates/dots-core/src/watchers/github.rs` talks to the GitHub API with ETags and rate limits).
- The person and Aakshar both have write access to `Aakshar-D/dots`.
- The update spec's testing rule stands: agents never install, uninstall or run an installer, and never touch the installed Dots or `%LOCALAPPDATA%\dots`.

## 4. Part 1: everyone's side and the safety net

### 4.1 Settings › Updates

A section in Settings, after Claude usage:

- "Dots 0.3.0 · up to date · checked 10:42" (or "· 0.3.1 is ready", or "· never checked"). A development build or a `DOTS_DATA_DIR` instance says "Updates are off in this copy" and shows no update controls; it still shows the publishing link below (§5.11).
- **Check for updates.** Checks at once, then shows: "Up to date", "0.3.1 is downloading…", "0.3.1 is ready" (the strip appears as today), or the reason the check failed, inline, until the next check.
- **What's new.** Opens the notes of the version running (and, when one is ready, of that one) in a dialog inside Dots, rendered from the GitHub release's body. Links in the notes open in the browser.
- **Install updates by itself when idle.** A switch, off by default; a setting kept with the others (`Settings`).
- **Repair Dots** (§4.3).
- **Go back to 0.2.0** while the backup made by the last update exists (§4.4).
- For a publisher (§5.1), the section ends with "Publish versions of Dots…" which opens the Releases page; for anyone else, "Publish versions of Dots (for its developers)…" which opens the token chooser (§5.1).

### 4.2 Installing by itself when idle

With the switch on and an update ready, Dots installs it when **idle**, meaning all of:

- no run is `running`, `awaiting_approval` or `queued` (a queued run would start soon), and no chat is being written (the composer is empty of unsent text);
- the window has had no keyboard or pointer input for 10 minutes, or is hidden or minimised *(my call: 10 minutes)*.

It then follows the update spec's §7 order (close, back up, mark, install). The new Dots starts the way the old one was: hidden in the tray if it was hidden, without stealing focus *(my call)*. Next time the window is shown, a strip says once: "Dots updated itself to 0.3.1 at 03:12. What's new" with **What's new** and **Dismiss**.

**Later** on the strip stops the idle install until Dots next starts, as it hides the strip today.

An idle install runs the NSIS installer quietly, with no window *(my call; the build's `installMode` is `currentUser`, so no prompt is needed)*. An install the person clicks keeps today's passive progress window. The installer starts the new Dots without `--hidden`, so Dots writes a mark next to the restart mark that tells the new process to stay in the tray.

If a run starts while it waits to be idle, it waits again. If the backup fails, the update spec's §7 step 3 holds, and the switch does not retry until the next download.

### 4.3 Repair Dots

For an install that misbehaves. **Repair Dots…** asks once: "Dots will download version 0.3.0 again, back up your data and reinstall it. Runs that are working will be stopped." Then it downloads the running version's own installer from its GitHub release, checks its signature like any update, and installs it through the §7 order. Offered only when the running version has a published release; otherwise it says "This version has no published installer to repair from."

*(my call)* Repair waits for working runs like Restart to update does (the update spec's §6 choices: wait, or stop them).

### 4.4 Health check and going back

Before handing over to the installer, Dots writes `<data dir>/update-state.json` *(my call: a file, not the database, because going back replaces the database)*: the version it is leaving, the version it is installing, the backup's path, and `pending: true`.

The new version passes its health check when, within 2 minutes of starting, all of these have happened: its migrations ran, the runtime started, and the window's UI loaded and told the core so (once, on its first render). It then sets `pending: false` and records the version as good.

It fails when any of these happens first:

- the start shows the fatal-error page (a migration failed, the runtime could not start);
- the 2 minutes pass;
- the next start finds `pending: true` still set by a previous start of the same version (it crashed).

On failure the fatal page, or a strip if the window works, says: "Dots 0.3.1 did not start properly. Go back to 0.2.0? Your data from before the update comes back with it; anything done since the update is lost." with **Go back** and **Not now**.

**Go back** downloads the previous version's installer from its GitHub release, checks its signature with the same key, closes the runtime, keeps the current `dots.db` as `backups/dots-<failed version>-failed-<time>.db`, puts the update's backup in place of `dots.db` (removing `dots.db-wal` and `dots.db-shm`), marks the restart, and installs the old version (the updater allows this one downgrade). The failed version goes into `update-state.json` as **bad**: no check offers it to this machine again; a newer version is offered as usual.

**Go back to 0.2.0** in Settings › Updates does the same for a version that started but misbehaves, while the backup exists (the three newest are kept, as today). It asks first, with the same warning about work done since the update.

If the download fails, nothing changes and the reason shows; the by-hand steps in `docs/releasing.md` stay as the last resort.

### 4.5 Withdrawn versions (shared with §5.9)

On each check, Dots also reads its own version's release on GitHub (the public API, no token). If that release is marked withdrawn (§5.9), Settings › Updates and a strip say: "The developers withdrew Dots 0.3.1. Go back to 0.2.0?" with **Go back** (§4.4) and **Not now**. A newer version, when published, is offered as usual.

## 5. Part 2: the Releases page

### 5.1 Who is a publisher

A publisher is a Dots with a GitHub token chosen for publishing whose account has push (write) access to `Aakshar-D/dots`, checked with the GitHub API when chosen and at each start. The token is picked from the vault with the same chooser and **Add to vault…** that watchers use. The repository is fixed; it is not a setting.

The chooser asks for a **classic token with the `repo` and `workflow` scopes** and checks both when it is chosen. *(my call, from the builder check: `Aakshar-D` is a personal account, so only Aakshar's own fine-grained token can reach the repository, and without `workflow` GitHub refuses ref updates that bring in `.github/workflows` changes, which §5.4's branch moves and §6's workflow change make.)* A fine-grained token is accepted only if it has Contents, Pull requests, Secrets and Workflows read and write, and Actions and Metadata read, which in practice means Aakshar's. Secrets write is used by the setup only.

A publisher sees a **Releases** icon in the rail. Nobody else sees it. If the token stops working or loses access, the icon goes, and Settings › Updates says why.

### 5.2 Setting up releases (once)

Releases are **set up** when master's `tauri.conf.json` holds a public key that is not the prototype's and both secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` exist on the repository. Until then the page shows the setup, in this order, each step ticked when done:

1. **Make the signing key.** The publisher types a password twice (at least 12 characters *(my call)*). Dots makes the key pair in memory.
2. **Save a backup.** "If this key and its password are both lost, no installed Dots can ever take another update." **Save key file…** writes the private key file where the publisher chooses. The step completes only when the publisher ticks "I have saved the key file and its password somewhere other than this PC".
3. **Put it in GitHub's secrets.** Dots encrypts both values with the repository's public key for secrets and stores them. It refuses if either secret already exists.
4. **Put the public key into Dots.** Dots opens a pull request that replaces `plugins.updater.pubkey`, titled "Use the release signing key", and links to it. The step completes when that pull request is merged (the page checks master).

After step 3, Dots forgets the private key and password. They exist only in the publisher's backup and in GitHub's secrets.

If the secrets already exist but master still has the prototype key, or the other way round, the page says what is there and what is missing and does nothing: "GitHub already has a signing key. Ask whoever set it up for the public key, or remove both secrets on GitHub to start again." The setup never replaces or deletes a secret.

### 5.3 Ready

- **Live for everyone:** the newest published, not withdrawn release, with its date, or "nothing published yet".
- **Changes on master since it:** the merged pull requests between that release's tag and master, newest first, linked.
- **Version** for the trial: suggested as the next minor version (0.2.0 to 0.3.0), editable, which must be a plain `MAJOR.MINOR.PATCH` above the live one.
- **Publish trial**, which asks once: "Publish 0.3.0-rc.1 from master as it is now? It goes only to publishers' Dots."

### 5.4 Publishing a trial

Dots, through the GitHub API, without a local git:

1. takes master's head commit C;
2. makes a branch `release/0.3.0` at C, and on it one commit that sets the five version fields to `0.3.0-rc.1` (in `Cargo.lock`, only the `version` lines of `dots-app` and `dots-core`);
3. pushes the tag `v0.3.0-rc.1` on that commit.

A later trial of the same version (after a failure, or to include newer changes) is `rc.2` from master's new head, on the same branch name, which is moved.

The release workflow then runs. It now has a **test** job first (core, app and UI suites, typecheck, clippy and fmt) and builds only if it passes. The Releases page shows the run's stages live: waiting, testing, building, signing, published, with the elapsed time and **View on GitHub**. A failure names the stage and links to the failed step; the trial is not published, and **Publish trial** is offered again.

The notes draft (§5.6) is committed with the version change as `release-notes.md` at the repository's root on the release branch; the workflow publishes the release with that file as its body (`--notes-file`) instead of GitHub's generated notes, and puts it in `latest.json`'s `notes` *(my call, from the builder check: the notes are right from the first minute, even if Dots is closed during the run)*. A tag without the file (a release made by hand) keeps today's generated notes.

### 5.5 Trials reach publishers

A publisher's Dots, at each check, also looks for the newest prerelease (with its token) newer than the version running, not withdrawn and not bad on this machine, and offers it through the usual strip and **Install updates by itself when idle**, like any update. Its signature is checked the same way. Settings › Updates says "0.3.0-rc.1 (trial)".

Promoting or dropping a trial ends it: a publisher still on `0.3.0-rc.1` is offered `0.3.0` when it is out, or the next version.

### 5.6 Notes

While a trial is out, the page shows **What's new**: a text box drafted from the merged pull requests since the live version, one line each, the title with its `feat(ui):`-style prefix removed and its number linked. The publisher edits it freely. **Save notes** commits it as `release-notes.md` on the release branch (and edits the trial's release body to match); Promote's version commit carries it, so the release is published with it. *(my call: drafted from titles by plain rules, no model run.)*

### 5.7 Promote

**Promote 0.3.0 to everyone** asks once: "Every Dots will be offered 0.3.0 within six hours. It is exactly the code of 0.3.0-rc.1." Then Dots:

1. adds a commit on `release/0.3.0`, on top of the trial's, setting the five fields to `0.3.0`;
2. merges `release/0.3.0` into master with a merge commit (only the version fields differ from C, so it merges cleanly unless master's version fields changed meanwhile; if it cannot, it stops and says so);
3. tags that version commit `v0.3.0`. Its tree is the trial's code with the final version numbers, and it is on master's history, which the workflow's check accepts;
4. shows the run as in §5.4 (tests and build run again for the release);
5. once published, deletes the branch.

If the run fails, master already reads 0.3.0 and the tag exists. The code is the trial's, whose tests passed, so the page offers **Retry**, which re-runs the same workflow run through the Actions API without a new tag or commit. If it fails again, the page says so and the next trial is 0.3.1; 0.3.0 is skipped (the "above the live one" rule also skips a version whose tag exists) *(my call, from the builder check)*.

If master moved after the trial, the page shows "master has 2 changes newer than this trial; publish a new trial to include them" before Promote, and Promote still ships the trial's code.

### 5.8 Drop trial

**Drop trial** asks once, then marks the prerelease withdrawn (§5.9) and deletes the branch. Publishers' Dots stop being offered it; a publisher already on it is offered the live version back through Go back (§4.5).

### 5.9 Withdraw a release

On the live release: **Withdraw 0.3.1…** asks once: "No more Dots will take 0.3.1. Dots already on it will be offered to go back to 0.3.0." Dots edits the release to a prerelease named "Dots 0.3.1 (withdrawn)" with the word `withdrawn` in a hidden marker at the top of its body. GitHub's "latest" then points at the release before it, so no new copy takes 0.3.1; copies already on it see §4.5. A withdrawn release cannot be promoted again; the fix is a new version.

### 5.10 Safety of publishing

- Every publish action (setup steps 3 and 4, Publish trial, Save notes, Promote, Drop trial, Withdraw) needs its button and a confirmation; none is done by a dot, a schedule, or an agent tool.
- The token is read from the vault for each call and never logged, sent in an event or shown. GitHub's error text is shown with any token-like text removed.
- The repository and the branch and tag names are built by Dots, not typed by the publisher; the version is validated as `MAJOR.MINOR.PATCH`.

### 5.11 The first release

No installed Dots has the Releases page yet: nothing is published, and 0.1.0 has no updater. The person decided both of these:

- **The Releases page works in a copy built from the repo** (`bun run tauri dev`, a debug build, or one started with `DOTS_DATA_DIR`). Such a copy never checks for or installs updates, but a publisher can open Releases from Settings › Updates, choose a token from that copy's own vault, run the setup and publish. The rail icon follows the same publisher rule.
- **The person builds and installs Dots by hand once**, after the setup's pull request has put the real public key on master: a local NSIS build of master, installed by the person. That copy has the updater and the real key, so it takes the first trial and every release after it. `docs/releasing.md` gets these steps. Installing is a person step; agents never run it.

## 6. Testing

- The idle rule, the health check states (pending, good, failed by page, by time, by crash), bad and withdrawn versions, choosing between a release and a trial, and the update-state file are tested in Rust with a fake clock and no network.
- The go-back order (download, check, close, keep the failed database, restore, mark, install) and Repair use the updater behind a seam, as the update spec's tests do; no test installs anything.
- The GitHub calls of the Releases page (publisher check, setup detection, secrets, branch, commit, tag, PR, merge, release edits, workflow progress) are tested against a mock server, as watchers' GitHub tests are. The version-field edit is tested on copies of the five real files.
- The key step is tested with a generated key: a file signed with it verifies against its public key; the private key does not appear in the state the UI reads.
- The UI: Settings › Updates in each state; the Releases page in each stage of §5.2–5.9; every confirmation; the rail icon only for a publisher.
- The workflow's test job is checked with a prerelease tag before the first release.
- Person steps, each only on the person's word: running the setup against the real repository, publishing a real trial, promoting it, and installing it on the person's own machine. Agents never install or touch the installed Dots (update spec §11).

## 7. Left to the builder

- Command, event and setting names.
- How the UI tells the core it has rendered (§4.4), and where the 2-minute timer lives.
- The library that makes the minisign key pair and the one that encrypts GitHub secrets.
- Whether the GitHub calls live in the app crate or the core (watchers' client may be reused).
- How the workflow's stages are read from the Actions API (jobs and steps) and how often they are polled while the page is open *(at most every 10 s)*.
- The exact layout of the Releases page within mockup B, and wording beyond what this document quotes.

## 8. As built, part 1 (everyone's side and the safety net)

Plan: `docs/superpowers/plans/2026-10-27-updates-part1.md`. Smoke: `docs/verification/2026-10-27-updates-part1-smoke.md`.

**Deviations and the builder's choices**

| Where | Built | Why |
|---|---|---|
| §4.4 | A set-aside database is `backups/failed-dots-<v>-<time>.db` | `dots-<v>-…` would count as a backup: pruning keeps three and could delete the update's backup |
| §4.3 | A repair keeps the record's earlier go-back target | Going back after a repair goes to the version before the last update |
| §4.2 | An idle install downloads the update again with the quiet installer (`/S`) | tauri-plugin-updater fixes installer arguments when it checks |
| §4.2 | Whether `/S` beats the config's `/P` was not readable (no bundler sources on the build machine) | At worst a small progress window shows; to confirm on the first real idle install |
| §4.1 | "Checking…" while a check runs | The version is unknown until the check answers |
| §4.4 | The window reports its first render from the app frame, not the fatal page | A fatal page is not a healthy start |
| §4.1 | The strip's Go back has no second confirm; Settings' does | The strip carries the warning sentence |
| Wording | "Repairing Dots. Dots restarts in a moment.", "Going back to Dots <v>. …", "Dots will repair itself when … finishes.", "Dots will go back to <v> when … finishes.", the fatal page's "This start followed an update. Go back to <v>? …" | Beyond what §4 quotes |

**Not seen in the smoke (person steps):** a real update, idle install, repair and go back on an installed Dots; the quiet installer's behaviour; the withdrawn strip against a real withdrawn release.

## 9. As built, part 2 (the Releases page)

Plan: `docs/superpowers/plans/2026-10-27-updates-part2.md`. Smoke: `docs/verification/2026-10-27-updates-part2-smoke.md`.

| Where | Built | Why |
|---|---|---|
| §5.2–5.9 | The flows run through a `Hub` trait; their tests use an in-memory repository, the HTTP client its own mock-server tests | Trees, branches and tags are checked directly |
| §5.2 | `Setup` is `done`, `needed`, or `half_done { secrets: how many of the two, pubkey }` | One secret of two is half done too |
| §5.2 | The key file is saved through a native save dialog the app opens | The window gets no file-system power |
| §5.4 | A second trial of a version keeps notes already on its branch | The publisher's edits survive |
| §5.7 | The page shows a promotion in progress or failed (`PageState.promotion`), with **Retry** | Retry needs something to show |
| §5.7 | The release branch is not deleted after a promotion | Deleting from a read of the page would be a side effect |
| §5.9 | Withdraw's confirm says "…go back to the version before it." | The page does not carry the previous release |
| §5.4 | CI skips `preview_schedule_lists_the_next_fire_times` | It depends on the machine's time zone |

**Person steps, each only on the person's word:** the setup against the real repository; the first trial (which also checks the workflow's new test job); promoting it; a hand install of master after the key PR is merged.
