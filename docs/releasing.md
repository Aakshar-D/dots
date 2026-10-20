# Releasing Dots

Every installed Dots checks GitHub for a new version and offers it in a strip at the top of its window (`docs/superpowers/specs/2026-10-20-auto-update-design.md`). A release is made by GitHub (`.github/workflows/release.yml`) when a version tag is pushed. The steps below are the owner's.

## The signing key

**If the private key or its password is lost, no installed Dots can ever take another update.** Each would need a new install by hand of a build with a new public key. Keep the key file and its password in a password manager, not only on one PC.

Every update is signed with the private key. Every Dots carries the public key (`plugins.updater.pubkey` in `apps/dots-app/tauri.conf.json`) and refuses a download whose signature does not match it. The workflow uses the key in one step only, "Sign the installer", after the build: the build itself never sees it.

Once, before the first release:

1. Make the key pair, with a password. In PowerShell, from the repo, make the folder first, then the keys:
   `New-Item -ItemType Directory -Force "$env:USERPROFILE\.tauri"` and
   `bun run tauri signer generate -w "$env:USERPROFILE\.tauri\dots-updates.key"`.
   In Git Bash: `mkdir -p "$HOME/.tauri"` and `bun run tauri signer generate -w "$HOME/.tauri/dots-updates.key"`.
   It writes `dots-updates.key` (private) and `dots-updates.key.pub` (public).
2. Put the text of `dots-updates.key.pub` in `plugins.updater.pubkey`, through a pull request.
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
4. The workflow builds the installer, signs it in a step of its own, and publishes the release "Dots 0.2.0" with the installer, its `.sig` and `latest.json`. Its notes are GitHub's generated notes; edit them on GitHub if you like. Within six hours every running Dots offers the update.

If the workflow failed after it created the release, delete that release and its tag on GitHub, fix the cause, and tag again.

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

A version that cannot start never checks for updates, so a newer release does not reach it: on such a machine, install the fix by hand from the releases page.
