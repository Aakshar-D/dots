# Releasing Sage

Every installed Sage checks GitHub for a new version and offers it in a strip at the top of its window (`docs/superpowers/specs/2026-10-20-auto-update-design.md`). A release is made by GitHub (`.github/workflows/release.yml`) when a version tag is pushed. The steps below are the owner's.

## Releasing from inside Sage (the Releases page)

Sage publishes itself from its **Releases** page (spec `docs/superpowers/specs/2026-10-27-updates-ui-design.md` §5). Only a Sage whose GitHub token can push to `Aakshar-D/dots` shows it: in Settings › Updates, **Publish versions of Sage (for its developers)…** chooses that token from the vault. It must be a classic token with the `repo` and `workflow` scopes (a fine-grained token only works for the repository's owner). Sage checks it after each save; a refusal says why.

**The first release, once:**

1. In a copy built from the repo (`bun run tauri dev`, or a debug build with `DOTS_DATA_DIR`), choose the token in Settings › Updates. The **Releases** icon appears.
2. On the Releases page, run the setup: make the signing key with a password of at least 12 characters, **Save key file…** somewhere other than this PC and keep the password with it, put both secrets on GitHub, and open the pull request "Use the release signing key". Sage forgets the key and password once they are on GitHub. If GitHub already has a key, the page says so and changes nothing.
3. Merge that pull request.
4. Build master and install it by hand once (`bun run tauri build`, then run the installer from `target/release/bundle/nsis`). That copy has the updater and the real key.
5. In it (or the dev copy), **Publish trial**: Sage puts the version and the notes on a `release/<version>` branch and tags `v<version>-rc.<n>`. GitHub runs every test suite, then builds and signs. Only publishers' Sage take the trial.
6. Edit **What's new**, **Save notes**, then **Promote <version> to everyone**: the trial's exact code with the final version, merged into master and tagged. Every Sage offers it within six hours.

A promote whose build fails offers **Retry**. **Drop trial** withdraws a trial; **Withdraw <version>…** stops a broken release reaching more copies and offers the copies already on it to go back.

The steps below are the same release by hand, for when the page cannot be used.

## The signing key

**If the private key or its password is lost, no installed Sage can ever take another update.** Each would need a new install by hand of a build with a new public key. Keep the key file and its password in a password manager, not only on one PC.

Every update is signed with the private key. Every Sage carries the public key (`plugins.updater.pubkey` in `apps/dots-app/tauri.conf.json`) and refuses a download whose signature does not match it. The workflow uses the key in one step only, "Sign the installer", after the build: the build itself never sees it.

Once, before the first release:

1. Make the key pair, with a password. In PowerShell, from the repo, make the folder first, then the keys:
   `New-Item -ItemType Directory -Force "$env:USERPROFILE\.tauri"` and
   `bun run tauri signer generate -w "$env:USERPROFILE\.tauri\dots-updates.key"`.
   In Git Bash: `mkdir -p "$HOME/.tauri"` and `bun run tauri signer generate -w "$HOME/.tauri/dots-updates.key"`.
   It writes `dots-updates.key` (private) and `dots-updates.key.pub` (public).
2. Put the text of `dots-updates.key.pub` in `plugins.updater.pubkey`, through a pull request. Until then `plugins.updater.pubkey` holds the prototype's throwaway key, and the release check (below) refuses every tag, prereleases too.
3. In the repo's GitHub settings, under Secrets and variables, then Actions, add two repository secrets: `TAURI_SIGNING_PRIVATE_KEY` (the text of `dots-updates.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (its password).

## A release

1. In a pull request, set the new version in all five places:
   - `apps/dots-app/tauri.conf.json` (`version`)
   - `apps/dots-app/Cargo.toml` and `crates/dots-core/Cargo.toml` (`[package]` `version`)
   - `ui/package.json` (`version`)
   - `Cargo.lock`, which `cargo build` updates after the two `Cargo.toml` changes
2. Check them: `node tools/release/release.mjs check v0.2.0` prints `prerelease=false`, or names each place that disagrees.
3. Merge the pull request. Then fetch, tag the merge commit on `master` and push the tag:
   `git fetch origin`, `git tag v0.2.0 origin/master` and `git push origin v0.2.0`. A plain tag that is not on `master` fails the workflow.
4. The workflow builds the installer, signs it in a step of its own, and publishes the release "Dots 0.2.0" with the installer, its `.sig` and `latest.json`. Its notes are GitHub's generated notes; edit them on GitHub if you like. Within six hours every running Sage offers the update.

The same workflow also builds the macOS app on a Mac runner: one universal build for Apple silicon and Intel Macs, published as `Sage_<version>_universal.dmg` beside the installer, with `Dots_<version>_universal.app.tar.gz` and its `.sig` for the updater (`latest.json` then offers the update to `darwin-aarch64` and `darwin-x86_64` too). If the Mac build fails, the release is published without the Mac files, and the Windows copies are not held back. The Mac app is not code-signed or notarized until the repository has the Apple secrets (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, and for notarization `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`); until then, the first time a person opens it they right-click it and choose **Open**, or run `xattr -d com.apple.quarantine /Applications/Dots.app`. To check the Mac build without releasing, run the **Mac check** workflow (`.github/workflows/mac-check.yml`); a pull request that changes the Rust code runs it too.

If the workflow failed after it created the release, delete that release and its tag on GitHub, fix the cause, and tag again.

## Trying a version before it reaches everyone

A tag with a suffix, such as `v0.2.0-rc.1`, is published as a prerelease. GitHub's "latest" skips prereleases, so no installed Sage takes it on its own. Such a tag can sit on a branch: set the five places to `0.2.0-rc.1` on the pull request's branch, tag that commit, and set them to `0.2.0` before merging.

To run the whole update path on your own machine:

1. Publish a first prerelease, for example `v0.2.0-rc.1`, and install it by hand from its release page.
2. Publish a second one, `v0.2.0-rc.2`.
3. Quit Sage, then start the installed Sage with `DOTS_UPDATE_ENDPOINT` set to the second one's manifest, in PowerShell:
   `$env:DOTS_UPDATE_ENDPOINT = "https://github.com/Aakshar-D/dots/releases/download/v0.2.0-rc.2/latest.json"; & "$env:LOCALAPPDATA\Dots\dots-app.exe"`.
4. Within a minute the strip offers `0.2.0-rc.2`. Restart to update; Sage backs up its database to `%LOCALAPPDATA%\dots\backups`, installs, and opens on the new version.

The variable changes only where Sage looks; a download must still carry your signature. A Sage started with `DOTS_DATA_DIR` set never checks.

## The first version with updates

Copies of Sage from before this feature (`0.1.0`) have no updater. Install the first release that has it by hand, once, on each machine. Every later version arrives on its own.

## What each installed Sage offers (Settings › Updates)

From the version with the updates UI (spec `docs/superpowers/specs/2026-10-27-updates-ui-design.md`):

- **Check for updates** checks at once; **What's new** shows the running version's release notes.
- **Install updates by itself when idle** (off by default): with an update downloaded, Sage installs it when no run is unfinished, nothing is typed in the composer, and the window has had no input for 10 minutes or is hidden. The installer runs quietly and the new Sage stays in the tray; the next time the window shows, a strip says "Dots updated itself to <version> at <time>". **Later** on the strip stops it until Sage next starts.
- **Repair Sage…** downloads the running version again and reinstalls it, with a backup first.
- **Go back to <version>…** is offered while the backup the last update made still exists. It reinstalls the version before the update and puts that backup in place of `dots.db`; the database being replaced is kept in `backups\failed-dots-<version>-<time>.db`. Anything done since the update is lost.
- After each update, the new version must start and show its window within 2 minutes. If it does not (or the start fails, or it crashes before then), Sage offers **Go back**, on the fatal page when Sage cannot start, and never offers that version to this PC again. The record of this lives in `%LOCALAPPDATA%\dots\update-state.json`.
- A version its developers withdraw (part 2's Releases page) shows a strip offering to go back.

A copy started with `DOTS_DATA_DIR`, or a development build, says "Updates are off in this copy" and shows none of these.

## Going back to an older version

Sage can do this itself (above). By hand, as a last resort:

A broken release is fixed by publishing a newer one. Deleting a release on GitHub makes "latest" point at the one before it, but a Sage that has updated stays where it is. To take one machine back:

1. Quit Sage.
2. Download the older version's installer from https://github.com/Aakshar-D/dots/releases and run it.
3. If the newer version changed the database, put the backup back: in `%LOCALAPPDATA%\dots`, rename `dots.db` to keep it, delete `dots.db-wal` and `dots.db-shm` if they are there, and copy the newest file from `backups\` (named `dots-<old version>-<date>-<time>.db`) to `dots.db`.
4. Start Sage. If it cannot start, its error page names the newest backup.

A version that cannot start never checks for updates, so a newer release does not reach it: on such a machine, install the fix by hand from the releases page.
