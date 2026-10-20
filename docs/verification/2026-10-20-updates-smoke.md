# Updates for installed Dots: smoke test

- Date: 2026-10-20
- Build: debug (`bun run tauri build --debug --no-bundle`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/auto-update` at commit `6d9eebe`. Windows 11.
- Part A (agent steps): 4 steps (A1 to A4), all passed. No Dots defect found.
- Part B (person steps): 5 steps (B1 to B5), open. Nothing in Part B was run.
- Screenshots: not in the repository. They are in the scratchpad folder of the run (`C:/Users/aksha/AppData/Local/Temp/claude/C--Users-aksha-dots/6f64b9c2-ef46-4359-8b32-09a568c6c3bc/scratchpad/updates-smoke`): `01-list.png`, `02-strip.png`, `03-after-later.png`, `04-fatal.png`, and the two app logs.

## What this covers

Installed Dots updates itself: a signed `latest.json` and installer on a GitHub release, a check at start and on a timer, a strip in the window ("Dots X is ready."), a backup of the data before the install, and a fatal-error page that names the backup. Part A covers what runs without a published release: the updater plugin starts, the update commands answer correctly when no update is ready, the fatal-error page, the strip in the real window, and the release check script. Part B lists what needs a person with a signing key and a second release.

## How the agent steps were run

- The app is the debug build, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d` (A1 and A3: `C:\Users\aksha\AppData\Local\Temp\tmp.RGRJbu2OQE\data`; A2: `C:\Users\aksha\AppData\Local\Temp\tmp.GwxHs3kZcH\data`). The webview profile was moved into the same temp folders (`...\wv`), with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`. Every path variable was echoed before use.
- The real data folder `%LOCALAPPDATA%\dots` and the installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) were never used or touched. Before each stop, `Get-Process dots-app` showed one process, `C:\Users\aksha\dots\target\debug\dots-app.exe`, and only that one was stopped.
- `PATH` left out `/c/Users/aksha/AppData/Roaming/npm`, the only folder with a `claude` CLI; `which claude` then found nothing, and the window shows "The claude CLI was not found".
- The window is driven with `node tools/smoke/webview.mjs 9333` (`W` below). Commands that answer an error were wrapped so the error is printed: `t() { $W eval "(async()=>{try{return await invoke($1)}catch(e){return {err:e}}})()"; }`.
- Nothing was installed, released, pushed or tagged; no signing key was used.

## Part A: agent steps

### A1. The app starts with the updater plugin

```bash
$W text
grep -rn "update checks are off" "$DOTS_DATA_DIR/logs"
$W eval "await invoke('update_note')"
t "'update_restart'"; t "'update_open_notes'"
t "'update_stop_and_restart',{working:[]}"
t "'update_later'"
```

Expect: the window shows the Dots list; the log has `update checks are off in this build or under DOTS_DATA_DIR`; `update_note` answers `{"state":"none"}`; `update_restart` and `update_open_notes` answer a `conflict` "no update is ready". Changed after the plan: `update_stop_and_restart` with `{ working: [] }` answers the same conflict; `update_later` has no ready check and answers `{"state":"none"}`.

Result: pass. The window showed the Dots list ("No dots yet.", plus the "claude CLI was not found" banner, expected with no CLI on the PATH). The log line at `logs/dots.2026-10-20.log:4`: `INFO dots_app::update_service: update checks are off in this build or under DOTS_DATA_DIR`. `update_note` answered `{"state":"none"}`. `update_restart`, `update_open_notes` and `update_stop_and_restart` with `{working:[]}` each answered `{"kind":"conflict","message":"no update is ready"}`. `update_later` answered `{"state":"none"}`.

Note: my first attempt at the error cases had a quoting slip in the command string (the page said "Command update_restartupdate_restart not found"); that was the smoke command, not Dots. The rerun with the `t` helper gave the results above.

### A2. The fatal-error page names a backup

```bash
# second fresh data folder
echo dummy > "$T2/data/backups/dots-0.1.0-2026-10-20-091700.db"
echo "this is not a database" > "$T2/data/dots.db"
# start the app with DOTS_DATA_DIR on it
$W text
```

Expect: the page shows the error, the backup's path and `https://github.com/Aakshar-D/dots/releases`.

Result: pass. The page read:

```
Dots could not start

Dots could not read its settings in C:\Users\aksha\AppData\Local\Temp\tmp.GwxHs3kZcH\data: error returned from database: (code: 26) file is not a database

The log folder has the details: C:\Users\aksha\AppData\Local\Temp\tmp.GwxHs3kZcH\data\logs

If this began after an update, the data from before it is backed up in C:\Users\aksha\AppData\Local\Temp\tmp.GwxHs3kZcH\data\backups\dots-0.1.0-2026-10-20-091700.db. The previous version's installer is on https://github.com/Aakshar-D/dots/releases.

Close this window to quit. Dots tries again the next time you start it.
```

### A3. The strip in the real window

```bash
$W eval "await window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event: 'update', payload: { state: 'ready', version: '0.2.0', working: [], error: null } })"
$W text; $W shot 02-strip.png
$W click "Later"; $W text; $W shot 03-after-later.png
```

Expect: the strip shows "Dots 0.2.0 is ready." with three controls; Later makes it go.

Result: pass. The bridge accepted the emit (answered `null`). The strip showed `Dots 0.2.0 is ready.` with `What's new`, `Restart to update` and `Later`. After the click on Later the strip was gone from the text. The answer of the `update_later` call behind that click was not captured separately; A1 recorded that command's answer (`{"state":"none"}`).

### A4. The release check

```bash
node tools/release/release.mjs check v0.1.0
node tools/release/release.mjs check v0.2.0
```

Expect: `v0.1.0` passes; `v0.2.0` names the six places.

Result: pass. `check v0.1.0` printed `prerelease=false`, exit 0. `check v0.2.0` exit 1, six lines, each "has 0.1.0, the tag v0.2.0": `apps/dots-app/tauri.conf.json`, `apps/dots-app/Cargo.toml`, `crates/dots-core/Cargo.toml`, `ui/package.json`, `Cargo.lock (dots-app)`, `Cargo.lock (dots-core)`.

## Part B: person steps (open, none run)

- B1. Make the key pair, put the public key in `tauri.conf.json` (if Task 4 kept the throwaway key) and add the two secrets (`docs/releasing.md`).
- B2. Push a prerelease tag from a branch whose five version fields read `0.2.0-rc.1`; the workflow publishes a prerelease with the installer, its `.sig` and `latest.json` (signing is a step of its own in the workflow); the release check fails a tag that disagrees.
- B3. Install `0.2.0-rc.1` by hand; publish `0.2.0-rc.2`; start the installed Dots with `DOTS_UPDATE_ENDPOINT` set to its manifest; within a minute the strip offers it; with a dot running, "Restart when it's done" waits (and Later is offered too) and a scheduled run stays queued; when the run ends Dots backs up to `%LOCALAPPDATA%\dots\backups`, installs in passive mode and opens on `0.2.0-rc.2` with its window shown, data intact; the backup opens.
- B4. "Stop it and restart" with a dot running: asks, installs (the command sends the dot names the strip showed), and the run is failed "interrupted" after the start.
- B5. A first plain release, once B1 to B4 pass.

## Findings

No Dots defect. The strip here was driven by a synthetic event, not a real ready update, so B3 is the first proof of the full path.
