# Updates for installed Dots: design

## 1. Intent

Today a new Dots reaches a machine only by hand: build the NSIS installer from `master`, quit Dots, back up `%LOCALAPPDATA%\dots`, run the installer with `/S`, start Dots again. This feature replaces that with a release that the owner starts by marking a version, and that every installed Dots, on any machine, picks up by itself.

The owner's words:

- Scope: "for all dots", meaning every installed copy, on any machine.
- Publishing: "GitHub building and publishing it on its own when you mark a new version".
- Receiving: "a note in the app asking them to restart into the new version".
- A run still in progress: "wait till run is done or give user the option to close the runs".
- The note's place: a strip across the top of the window (mockup option A), not a card in the sidebar.

Success means that after a version is marked on GitHub, every installed Dots shows the note within a few hours, and one click puts it on the new version with its data intact. Nobody has to build, download or run an installer.

## 2. Goals, non-goals, anti-goals

**Goals**

- A version tag on GitHub builds, signs and publishes a release with no step on the owner's PC.
- Every installed Dots finds the release, downloads it in the background and shows the note.
- The person chooses when to restart. Runs in progress are waited for or stopped, never cut off without being asked.
- The database is backed up before the new version opens it.

**Non-goals**

- macOS and Linux builds. Only the Windows NSIS build is made and tested today.
- A Windows code-signing certificate. Updates are protected by Dots' own update signature (§4). Windows SmartScreen still warns on a person's first install.
- Release channels (beta, stable), staged rollouts, or holding back a version for some machines.
- Going back to an older version from inside Dots (§8 says what a person can do instead).
- Updating the `claude` CLI, local models or plugins. Only Dots itself.

**Anti-goals** (these would make it a failure even if it works)

- Dots restarts on its own, or stops a working run, without the person clicking.
- An update that did not come from the owner's key is installed.
- A data upgrade loses data with no copy to go back to.
- Dots nags: repeated pop-ups, an error banner whenever GitHub cannot be reached, or the note returning minutes after "Later".
- A development build or a smoke run (`DOTS_DATA_DIR` set) replaces the installed Dots or touches its data.

## 3. Facts the design rests on

- The Tauri app is `apps/dots-app`. `tauri.conf.json` has `"version": "0.1.0"`, `"identifier": "dev.dots.app"`, bundle target `nsis` with `installMode: "currentUser"`. It has no updater plugin.
- The version also appears in `apps/dots-app/Cargo.toml`, `crates/dots-core/Cargo.toml` and `ui/package.json` (all `0.1.0`), and `Cargo.lock` records it for `dots-app` and `dots-core`. "The version fields" below means these five places.
- The repo `Aakshar-D/dots` is public on GitHub. It has no releases and no `.github/workflows`.
- The NSIS install folder `%LOCALAPPDATA%\Dots` is the same folder as the data folder `%LOCALAPPDATA%\dots` (Windows ignores case). A reinstall over it with `/S` kept the data on 2026-10-20 (the new build applied its migrations to the existing `dots.db`); nothing in the repo shows what the updater's install mode does to that folder (§7). The data folder holds `dots.db` (WAL mode), `logs/`, `mcp/`, and can hold `worktrees/` and `plugins/`.
- `restart_app` (`apps/dots-app/src/commands.rs`) shuts the runtime down, marks the restart so the new process shows its window, and calls `app.restart()`. `Runtime::shutdown()` only cancels a token; it does not wait for the runtime's tasks to stop writing. On the next start, `recover_interrupted` marks runs left `running` as failed with error `interrupted`. Runs that are `queued` or awaiting an approval carry on.
- `ui/src/app/Banners.tsx` already shows strips across the top (`Banner`, tones error, warning, info). Its `RestartButton` already asks before a restart while a dot has a run that has not ended, and names those dots.
- A failed start shows a fatal-error page (`apps/dots-app/src/lib.rs`), for example when a migration fails.
- Runs start from four triggers: `schedule`, `webhook`, `manual`, `chat`.

## 4. Publishing a version

The owner decided: GitHub builds and publishes on its own when a version is marked.

1. The owner raises the version in a pull request, in the version fields listed in §3, and merges it.
2. The owner marks the version by pushing a tag `v<version>` (for example `v0.2.0`) on the merged `master` commit.
3. A GitHub Actions workflow runs on tags matching `v*`, on a Windows runner. It:
   - fails at once if the tag, without its `v`, differs from any of the version fields;
   - builds the NSIS installer the way `bun run tauri build` does today;
   - signs the update artifacts with the private key from the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`;
   - publishes (not as a draft) a GitHub Release named after the tag. The release holds the installer, its `.sig` signature and a `latest.json` manifest in the format Tauri's updater reads.
4. Release notes are the release's body. They default to GitHub's generated notes; the owner can edit them on GitHub afterwards.

**Trust.** Updates use Tauri's own updater (`tauri-plugin-updater`). The public key is built into Dots (`plugins.updater.pubkey` in `tauri.conf.json`). Dots refuses a download whose signature does not match that key. The private key and its password exist only in the owner's keeping and in the repo secrets.

**Person steps, not agent steps.** Generating the key pair (`tauri signer generate`), keeping the private key safe, adding the two repo secrets and pushing a tag are done by the owner. If the private key is lost, installed copies can never take another update; they would need a manual install of a build with a new public key. The setup doc (§10) says so in its first lines.

**The endpoint.** Dots reads `https://github.com/Aakshar-D/dots/releases/latest/download/latest.json`. GitHub's "latest" is the newest published, non-prerelease release.

**Prereleases** *(my call)*. A tag that is not a plain `vMAJOR.MINOR.PATCH` (for example `v0.2.0-rc.1`) is published as a prerelease, which "latest" skips, so it reaches nobody. The version check is the same for it: the version fields must read `0.2.0-rc.1`. A prerelease tag may therefore sit on any branch, for example the version pull request's branch before its version is set to the final `0.2.0`. Only plain version tags belong on `master`. Tauri's NSIS bundle accepts a prerelease version: it keeps `0.2.0-rc.1` as the product version and writes `0.2.0.0` where Windows needs four numbers (tauri-bundler 2.12, `try_add_numeric_build_number`).

**Testing an update before it reaches everyone** *(my call)*. A release build reads the environment variable `DOTS_UPDATE_ENDPOINT`. When it is set, Dots checks that URL instead of the GitHub "latest" one, for example a prerelease's `latest.json`. The signature check is unchanged, so this can only point Dots at a build signed with the owner's key. This lets the owner run the whole path (check, download, note, backup, install, restart) on their own machine with two prereleases before a plain tag reaches every machine. It is never set by Dots itself, and `DOTS_DATA_DIR` still turns checks off (§5).

## 5. Checking and downloading

- Dots checks shortly after start, and then every 6 hours while it runs. *(my call)*
- A newer version found is downloaded in the background and its signature checked. The note appears only once the update is downloaded and ready to install.
- A failed check or download is logged and retried at the next check. The person sees nothing. *(my call)* A signature failure is logged as a warning and never offered.
- No check is made in debug builds or when `DOTS_DATA_DIR` is set. *(my call: these are development and smoke instances; installing would replace the installed Dots, see §2 anti-goals)*
- The check runs in Rust, in `apps/dots-app`. The webview gets no updater permission; the UI learns the update state through Dots' own commands and events, like the rest of `Status`. *(my call: keeps the CSP and capabilities as they are)*

## 6. The note

The owner chose a strip across the top, where Dots' other notes are (mockup A, kept in `.superpowers/brainstorm/`). It uses the existing `Banner` in `ui/src/app/Banners.tsx`, tone `info`.

**Nothing running.** Text: "Dots <version> is ready." Then a "What's new" button, which opens the version's GitHub release page in the browser (Dots opens it; the webview opens no address of its own), a **Restart to update** button, and a **Later** button.

- Restart to update installs the update and starts the new Dots with its window shown.
- Later hides the note until Dots next starts. *(my call)* The next start checks and downloads again before the note returns. Tauri's updater keeps a download only in memory and checks its signature when it downloads; keeping the bytes on disk would need Dots to check the signature again itself, which is not worth it for a file of about 6 MB.

**A dot has a run working.** The text names the dots that have a run with status `running`: "Dots 0.2.0 is ready. Daily digest is still running." This needs its own query: `dot_overview`, which `RestartButton` uses, gives each dot's newest run that is queued, running or awaiting approval, and cannot tell those apart. The owner decided the two choices:

- **Restart when it's done** ("when they're done" for more than one dot). The note changes to "Will restart into Dots 0.2.0 when Daily digest finishes. New runs wait until after the update." with a **Cancel** button. Dots restarts into the new version as soon as no run is working.
- **Stop it and restart** ("Stop them" for more than one dot). Asks once, with the wording `RestartButton` uses today: a run that is working ends as failed ("interrupted"); runs waiting in the queue or for an approval continue after the restart. Then the install begins at once, as "Restart Dots" does: the working runs are not cancelled first (that would end them as cancelled, not as the dialog says), and the next start marks them interrupted.

**Which runs block.** Only runs with status `running` make Dots wait. Queued runs and runs awaiting an approval survive a restart already, so they do not block it. *(my call)*

**While waiting.** No queued run is started, whatever its trigger. New schedule, webhook, manual and chat runs are still accepted and queued; they start after the restart. *(my call: otherwise a busy schedule could keep Dots from ever restarting.)* Cancel lifts the hold and the queue starts again. A person who starts a manual run while waiting should see that it is queued until after the update; the builder picks the wording.

**Quitting instead.** If the person quits Dots from the tray while an update is ready, Dots does not install it then. The note shows after the next start's check. *(my call)*

## 7. Installing

On Windows, Tauri's `Update::install` starts the NSIS installer and then ends the process (`std::process::exit`). The installer, not `app.restart()`, starts the new Dots, and `RunEvent::Exit` does not fire. So everything Dots must do happens before `install`. When no run is working any more (after the wait), or at once after "Stop it and restart", in this order:

1. Shut the runtime down and **wait** until its tasks have stopped writing to `dots.db`, at most 10 seconds (`Runtime::close`, which closes the database). `shutdown()` alone does not wait. If the database has not closed in time, no backup is made: as in step 3, Dots restarts into the current version, with the reason.
2. Back up the database (below).
3. If the backup fails, do not install. Restart into the current version with `app.restart()`, as "Restart Dots" does; no run is working by then, so this costs nothing. The new process shows the note again with an error line naming the reason. *(my call: a shut-down runtime cannot be started again in the same process.)*
4. Write the restart mark (`startup::mark_restart`) so the new Dots shows its window, like after "Restart Dots".
5. Hand over to `Update::install`, with the NSIS installer in passive or quiet mode so it asks nothing and starts the new Dots.

**The data folder is the install folder** (§3). Before this ships, the builder checks the tauri-bundler NSIS script of the version in use, in the mode the updater runs it, and confirms it leaves `dots.db`, `backups/`, `worktrees/`, `plugins/`, `mcp/` and `logs/` in place. If it does not, that is a stop: the plan says what it found before any further task.

**Backup.** *(my call on the details; the owner agreed to backing up)*

- A consistent copy of `dots.db` is made with SQLite (`VACUUM INTO` or the backup API), not a file copy. The database is in WAL mode, so a plain copy of `dots.db` alone could miss recent writes.
- It goes to `<data dir>\backups\dots-<old version>-<YYYY-MM-DD-HHMMSS>.db`.
- Only the database is backed up. `worktrees/`, `plugins/`, `mcp/` and `logs/` are not: no migration changes them, and worktrees can be large.
- The three newest backups are kept; older ones are deleted after a successful backup.

## 8. When a new version is broken

The owner was asked and left this open. *(my call)*

- If the new version fails to start (for example a migration fails), the existing fatal-error page adds two lines: where the newest backup is, and a link to the GitHub releases page, where the previous installer still is.
- Going back is by hand: install the previous release's installer, then put the backup in place of `dots.db`. The setup doc (§10) lists the steps.
- A broken release is fixed forward by publishing a newer version. Deleting a release on GitHub makes "latest" point at the one before it, but machines that already updated stay on the broken version.

## 9. The first updating version

Installed copies of `0.1.0` have no updater. The first version with this feature reaches each machine once by hand, from its GitHub release. Every later version arrives by itself.

## 10. Documentation

A short doc, `docs/releasing.md` *(my call on the name)*:

- the warning that a lost private key ends updates for every installed copy;
- generating the key pair and adding the two secrets;
- raising the version in its five places, then tagging;
- testing with prerelease tags: the workflow first, then the whole update path on the owner's machine with `DOTS_UPDATE_ENDPOINT` (§4);
- going back to an older version by hand (§8).

## 11. Testing

- The Rust logic that decides what the note shows (no update, ready, waiting, error), when Dots may restart while waiting, the hold on queued runs, the backup and its pruning, and the "no check under `DOTS_DATA_DIR` or in debug builds" rule are tested without the network and without the real updater.
- The UI strip is tested the way `Banners` is today: each state, each button, the confirm on "Stop it and restart", Cancel.
- The order in §7 (wait, backup, restart on a failed backup, mark, install) is tested with the updater behind a seam, so no test installs anything.
- The workflow, and then the update path with `DOTS_UPDATE_ENDPOINT`, are checked with prerelease tags before the first plain release. These steps need the owner's secrets and machine and are the owner's to run.
- Agents never install, uninstall or run an installer, and never touch the installed Dots or `%LOCALAPPDATA%\dots`. Installing a real update on the owner's machine is a person step in the smoke doc.

## 12. Left to the builder

- Names of the commands, events and `Status` fields that carry the update state to the UI.
- How the hold on queued runs is wired into the dispatcher.
- Whether the workflow uses `tauri-apps/tauri-action` or its own steps, as long as §4's checks and outputs hold.
- Whether the version check in the workflow is a script or inline.
- The exact wording beyond what §6 quotes.
