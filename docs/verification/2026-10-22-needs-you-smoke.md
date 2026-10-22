# What needs me: smoke test

- Date: 2026-10-22
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/needs-you` at commit `19ac646` (Tasks 1 to 7 and the final review's fix). Windows 11 on ARM, Bun 1.4.3, LLVM for `ring`.
- Data: the data folder of the chat-first home's smoke (`.../scratchpad/ch/data`), made before migration 0010, with its two local dots `helper` (`01M5HVXCYST6A9WKGZA0TNN8KV`) and `asker` (`01M5HVXD7CVJ7YMT52XRT20FPY`, which asks "Ship it?"), model `qwen2.5:7b` in Ollama.
- Steps: 6 by the agent, as expected; 2 open for a person at the machine (a click on a notification, the tray's Needs you).
- Screenshot: not in the repository; `.../scratchpad/ch/shots/b01-badges.png`.

## How it was run

The app was started on the data folder above with `PATH` without the `claude` CLI and WebView2's debugging port 9333, and driven with `tools/smoke/webview.mjs`. Notifications were read from the toast history Windows keeps: a debug build under `target\debug` shows its toasts under PowerShell's app id (the notification plugin sets the app's own id only for an installed build), whose history `Windows.UI.Notifications.ToastNotificationManager.History.GetHistory('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe')` lists. The window was hidden to the tray by sending it `WM_CLOSE`, as the close button does.

## Steps

### 1. An upgrade shows no wall of old results

Expect: after migration 0010 every dot's `seen_at` is null and only what needs the person counts; the dot open in the focused window is marked seen.

Result: pass. `dot_overview`: `asker` `seen_at` `2026-10-22T15:51:25.630Z` (open on start, the window focused), needs you 0, unread 0; `helper` `seen_at` null, 0, 0. `unread_total` 0. The rail's links: "Home", "Inbox", "Library", "Settings".

### 2. What happens on a dot not viewed counts; on the viewed one, only what needs you

Expect: a run of `helper` (not viewed) that ends is new for it; a run of `asker` (viewed) that ends is not new, and its question needs the person.

Result: pass. With `asker` open and `helper` marked seen, Run now on both (`01M5J4Z57YK87J721ZRPXQ9R8K`, `01M5J4Z5J6SV3X365M7Y1TPHA3`). After both ended: `asker` needs you 1, unread 1 (its question, not its run); `helper` needs you 0, unread 1 (its run). The rail: "Home, 2 unread", "Inbox, 1 waiting". The column: `asker` with a badge "1" in the waiting colour, `helper` with a muted "1", each named "1 unread" (`b01-badges.png`).

### 3. Opening a dot clears what is new

Expect: opening `helper`'s conversation marks it seen.

Result: pass. After opening `/home/<helper>`: `asker` 1, 1; `helper` 0, 0; the rail "Home, 1 unread"; `helper`'s row has no badge.

### 4. No notification for the dot being viewed

Expect: of the runs in step 2, `helper`'s notifies, `asker`'s does not.

Result: pass. The toast history gained "helper finished · I'm ready to assist! What's the next step?" (the new wording, no "Inbox"), and nothing for `asker`'s run or its question.

### 5. Hidden to the tray, notifications come back

Expect: once the window is hidden (the review's fix: the app ends the viewing on losing the focus and on hiding), a run of the dot that was open notifies.

Result: pass. `WM_CLOSE` hid the window (`IsWindowVisible` false). Run now on `asker` (`01M5J55FQ2AW0EFNX65G54QF99`): the history gained "asker asks · Ship it?" and "asker finished · I have asked the user if they want to ship the changes. …", in the new wording.

### 6. The wording

Expect: titles and bodies as spec section 4.2's table, no "Inbox" instruction.

Result: pass for the notices seen: "<dot> asks" with the question, "<dot> finished" with the summary's first line. The approval and schedule wordings are covered by the app's tests (`notice_text`).

### 7. Open for a person: a click on a notification

A click on a toast while it is on screen should bring Dots forward on the conversation it is about (`/chat/<dot>/<run>` for a run, `/home/<dot>` for a question). The agent cannot click a toast; the handler and the routes are covered by `notify_routes` and `events` tests.

### 8. Open for a person: the tray's Needs you

Right-click the tray icon with `asker` waiting: "Needs you ▸ asker (1)" opens its conversation; with nothing waiting, "Nothing waiting". The agent cannot open the tray menu; the labels, order and limit are covered by `tray` tests.

## Findings

None about Dots. Notifications of a debug build show under PowerShell's name and icon (the plugin's choice for builds under `target`); an installed build shows them as Dots.

## Self-QA after the smoke (commit `17782e3`)

- **Found:** a minimized window could keep the focus (Windows left it the foreground window), so neither the window's focus event nor the webview's `blur` or `visibilitychange` fired: `asker`, open when minimized, stayed viewed; a run of it gave no toast and did not count as new.
- **Fixed** in `apps/dots-app/src/viewing.rs`: the app keeps the dot the webview reports and tells the core it is viewed only while the main window is visible, not minimized and focused, checking again on focus changes, resizes (minimize, restore), hiding and showing; a dot that comes back into view is marked seen.
- **Checked again:** with `asker` open, Dots minimized (`IsIconic` true): Run now on `asker` gave two toasts ("asker asks · Ship it?", "asker finished · …") and `asker`'s unread went from 4 to 5 (4 needing you, 1 new). Restored to the front: `asker` was marked seen, unread 4.
- **Also checked:** focusing the window marks the open dot seen (unread 2 → 1); answering a question from the bar updates the dot's badge and the Home total at once (2 → 1); no console errors on any route; axe-core 4.10.2 (WCAG 2.2 A/AA and best practices): no violations on the home with both badge tones and on the Inbox; no overflow at 800 and 1440 px.
