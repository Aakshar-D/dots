# Updates for everyone (part 1): smoke test

- Date: 2026-10-27
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/updates-ui` at `9a7dda5`, version 0.1.0. Windows 11 on ARM.
- Data: a fresh sandbox folder under the session scratchpad, started with `DOTS_DATA_DIR` (update checks are off there). Nothing was installed; Go back, Repair and Check for updates were never pressed.
- Steps: 3, all as expected.

## 1. Settings › Updates in a copy without checks

Result: pass. The section says "Updates are off in this copy" and has no button or switch. A fresh start writes `update-state.json` with nothing pending. axe on Settings: no violations, dark and light; at 420 px nothing scrolls sideways.

## 2. A start that failed its health check

A hand-written `update-state.json` (leaving 0.0.9, installing 0.1.0, `pending: true`, `starts: 1`, a backup file present) stood for a previous start of 0.1.0 that never passed.

Result: pass. The strip read "Dots 0.1.0 did not start properly. Go back to 0.0.9? Your data from before the update comes back with it; anything done since the update is lost." with **Go back** and **Not now**; the record then held `bad: ["0.1.0"]`, still pending. At 420 px nothing scrolls sideways. axe on that screen found only `page-has-heading-one` (moderate), which the empty Home page has on master too.

## 3. A first start after an idle install that passes

The record said `pending: true`, `starts: 0`, `by_itself_at` set, and the stay-hidden mark was in the data folder.

Result: pass. The main window stayed hidden (Windows `IsWindowVisible` false); the page rendered, the record became `pending: false`, the mark was taken, and the strip read "Dots updated itself to 0.1.0 at 8:42 am." with **What's new** and **Dismiss**.

## Person steps still open

A real update, an idle install (and whether its installer shows any window), a repair and a go back on the person's own installed Dots, after part 2 can publish a release.
