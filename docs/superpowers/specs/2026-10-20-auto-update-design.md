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
- The version also appears in `apps/dots-app/Cargo.toml`, `crates/dots-core/Cargo.toml` and `ui/package.json` (all `0.1.0`).
- The repo `Aakshar-D/dots` is public on GitHub. It has no releases and no `.github/workflows`.
- The NSIS install folder `%LOCALAPPDATA%\Dots` is the same folder as the data folder `%LOCALAPPDATA%\dots` (Windows ignores case). The uninstaller keeps the data. The data folder holds `dots.db` (WAL mode), `logs/`, `mcp/`, and can hold `worktrees/` and `plugins/`.
- `restart_app` (`apps/dots-app/src/commands.rs`) shuts the runtime down, marks the restart so the new process shows its window, and calls `app.restart()`. On the next start, `recover_interrupted` marks runs left `running` as failed with error `interrupted`. Runs that are `queued` or awaiting an approval carry on.
- `ui/src/app/Banners.tsx` already shows strips across the top (`Banner`, tones error, warning, info). Its `RestartButton` already asks before a restart while a dot has a run that has not ended, and names those dots.
- A failed start shows a fatal-error page (`apps/dots-app/src/lib.rs`), for example when a migration fails.
- Runs start from four triggers: `schedule`, `webhook`, `manual`, `chat`.

## 4. Publishing a version

The owner decided: GitHub builds and publishes on its own when a version is marked.

1. The owner raises the version in a pull request, in the four places listed in §3, and merges it.
2. The owner marks the version by pushing a tag `v<version>` (for example `v0.2.0`) on the merged `master` commit.
3. A GitHub Actions workflow runs on tags matching `v*`, on a Windows runner. It:
   - fails at once if the tag, without its `v`, differs from any of the four version fields;
   - builds the NSIS installer the way `bun run tauri build` does today;
   - signs the update artifacts with the private key from the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`;
   - publishes (not as a draft) a GitHub Release named after the tag. The release holds the installer, its `.sig` signature and a `latest.json` manifest in the format Tauri's updater reads.
4. Release notes are the release's body. They default to GitHub's generated notes; the owner can edit them on GitHub afterwards.

**Trust.** Updates use Tauri's own updater (`tauri-plugin-updater`). The public key is built into Dots (`plugins.updater.pubkey` in `tauri.conf.json`). Dots refuses a download whose signature does not match that key. The private key and its password exist only in the owner's keeping and in the repo secrets.

**Person steps, not agent steps.** Generating the key pair (`tauri signer generate`), keeping the private key safe, adding the two repo secrets and pushing a tag are done by the owner. If the private key is lost, installed copies can never take another update; they would need a manual install of a build with a new public key. The setup doc (§10) says so in its first lines.

**The endpoint.** Dots reads `https://github.com/Aakshar-D/dots/releases/latest/download/latest.json`. GitHub's "latest" is the newest published, non-prerelease release.

**My call:** a tag that is not a plain `vMAJOR.MINOR.PATCH` (for example `v0.2.0-rc1`) is published as a prerelease, which "latest" skips. This gives the owner a way to test the workflow without reaching anyone.

## 5. Checking and downloading

- Dots checks shortly after start, and then every 6 hours while it runs. *(my call)*
- A newer version found is downloaded in the background and its signature checked. The note appears only once the update is downloaded and ready to install.
- A failed check or download is logged and retried at the next check. The person sees nothing. *(my call)* A signature failure is logged as a warning and never offered.
- No check is made in debug builds or when `DOTS_DATA_DIR` is set. *(my call: these are development and smoke instances; installing would replace the installed Dots, see §2 anti-goals)*
- The check runs in Rust, in `apps/dots-app`. The webview gets no updater permission; the UI learns the update state through Dots' own commands and events, like the rest of `Status`. *(my call: keeps the CSP and capabilities as they are)*

## 6. The note

The owner chose a strip across the top, where Dots' other notes are (mockup A, kept in `.superpowers/brainstorm/`). It uses the existing `Banner` in `ui/src/app/Banners.tsx`, tone `info`.

**Nothing running.** Text: "Dots <version> is ready." Then a "What's new" link to the GitHub release page, a **Restart to update** button, and a **Later** button.

- Restart to update installs the update and starts the new Dots with its window shown.
- Later hides the note until Dots next starts. *(my call)* The download is kept, so the note comes back at the next start without a new download.

**A dot has a run that has not ended** (`queued`, `running` or awaiting approval as `RestartButton` counts them today, but see the rule below). The text names the dots that have a run working, as `RestartButton` does: "Dots 0.2.0 is ready. Daily digest is still running." The owner decided the two choices:

- **Restart when it's done.** The note changes to "Will restart when Daily digest finishes." with a **Cancel** link. Dots restarts into the new version as soon as no run is working.
- **Stop it and restart.** Asks once, with the wording `RestartButton` uses today: a run that is working ends as failed ("interrupted"); runs waiting in the queue or for an approval continue after the restart. Then it restarts as above.

**Which runs block.** Only runs with status `running` make Dots wait. Queued runs and runs awaiting an approval survive a restart already, so they do not block it. *(my call)*

**While waiting.** No queued run is started, whatever its trigger. New schedule, webhook, manual and chat runs are still accepted and queued; they start after the restart. *(my call: otherwise a busy schedule could keep Dots from ever restarting.)* Cancel lifts the hold and the queue starts again. A person who starts a manual run while waiting should see that it is queued until after the update; the builder picks the wording.

**Quitting instead.** If the person quits Dots from the tray while an update is ready, Dots does not install it then. The note shows at the next start. *(my call)*

## 7. Installing

At restart, in this order:

1. Shut the runtime down as `restart_app` does.
2. Back up the database (below). If the backup fails, do not install. Show the note again with an error line naming the reason, and leave the person on the current version.
3. Hand over to Tauri's updater, which runs the NSIS installer without questions (passive or quiet mode) and starts the new Dots. The new Dots shows its window, like after "Restart Dots".

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
- raising the version in its four places, then tagging;
- testing the workflow with a prerelease tag;
- going back to an older version by hand (§8).

## 11. Testing

- The Rust logic that decides what the note shows (no update, ready, waiting, error), when Dots may restart while waiting, the hold on queued runs, the backup and its pruning, and the "no check under `DOTS_DATA_DIR` or in debug builds" rule are tested without the network and without the real updater.
- The UI strip is tested the way `Banners` is today: each state, each button, the confirm on "Stop it and restart", Cancel.
- The workflow is checked with a prerelease tag before the first real release. That step needs the owner's secrets and is the owner's to run.
- Agents never install, uninstall or run an installer, and never touch the installed Dots or `%LOCALAPPDATA%\dots`. Installing a real update on the owner's machine is a person step in the smoke doc.

## 12. Left to the builder

- Names of the commands, events and `Status` fields that carry the update state to the UI.
- How the hold on queued runs is wired into the dispatcher.
- Whether the workflow uses `tauri-apps/tauri-action` or its own steps, as long as §4's checks and outputs hold.
- Whether the version check in the workflow is a script or inline.
- The exact wording beyond what §6 quotes.
