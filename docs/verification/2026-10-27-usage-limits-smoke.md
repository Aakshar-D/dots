# Usage and limits: smoke test

- Date: 2026-10-27
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/usage-limits` at `db4eb89`. Windows 11 on ARM, Bun 1.4.3, Claude CLI 2.1.310 logged in as the person.
- Data: a fresh sandbox data folder (`.../scratchpad/usmoke/data`); two Claude dots in folder mode, "pinger" and "ticker" (every minute), both told to reply "ok".
- Steps: 6, all as expected; two findings.
- Screenshots: not in the repository; `.../scratchpad/usmoke/shots/`.

## How it was run

The app was started on the data folder above with WebView2's debugging port 9333 and driven with `tools/smoke/webview.mjs`; axe-core was run in the page over the same port.

## Steps

### 1. A real run brings a reading and a cost

Result: pass. Run now of "pinger" succeeded with 24,392 tokens in, 50 out and $0.098 API-eq.; `limits_get` held 5-hour 1% (resets 13:10 UTC) and weekly 18% (resets the next day 04:00 UTC).

### 2. A background run is held above the share

Expect: with the weekly share at 10%, a scheduled run waits and shows in the Inbox.

Result: pass. The next slot of "ticker" stayed queued; the Inbox showed "Held for Claude usage: ticker · Scheduled · starts after Wed 9:30 am" with **Run anyway**; the rail's control read "Claude usage: 5-hour 1%, resets 6:40 pm; weekly 18%, resets Wed 9:30 am; holding 1 runs", both bars in the holding colour; the Inbox link read "Inbox, 1 waiting".

### 3. The person's own run is never held

Result: pass. Run now of "pinger" during the hold started and succeeded.

### 4. Run anyway

Result: pass. Clicking **Run anyway** in the Inbox released the held run (`hold_released`), which started and succeeded.

### 5. Raising the share releases what is held

Result: pass. With a run held again, the weekly share went back to 70%: the run started at once and succeeded; nothing stayed held.

### 6. The Usage page and accessibility

Result: pass. The Usage page showed both windows with their shares and a table of both dots (runs, tokens, run time, API-eq.). axe: no violations on Usage, Inbox, Settings and Home, in dark and in light. At 420 px no screen scrolled sideways.

## Not seen

The tray's tooltip lines and the Windows notices ("Dots is holding background runs…", "Held runs are starting") are not reachable from the page; the app's unit tests cover their text.

## Findings

1. The rail's bar tracks are hard to see at low usage: at 1% the bars are almost invisible against the rail, in both themes.
2. "holding 1 runs": the count is not made singular.
