# Deep Teal: smoke test

- Date: 2026-10-23
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/deep-teal` at commit `0b2d183` (Tasks 1 to 7 and the final review's fixes). Windows 11 on ARM, Bun 1.4.3.
- Data: the sandbox data folder of the earlier smokes (`.../scratchpad/ch/data`), two local dots with runs, questions, an approval performed by Dots, and badges.
- Steps: 5, all as expected.
- Screenshots: not in the repository; `.../scratchpad/qa/teal/{light,dark}-{home,inbox,library,settings,dots,runs}.png`.

## How it was run

The app was started on the data folder above with WebView2's debugging port 9333 and driven with `tools/smoke/webview.mjs` and a small CDP helper (`scratchpad/qa/cdp.mjs`: console capture, axe-core 4.10.2 with the WCAG 2.0/2.1 A and AA, WCAG 2.2 AA and best-practice rules). The theme was changed through Settings' Appearance control (its radios clicked in the page). The title bar was read through Windows' `DwmGetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`.

## Steps

### 1. The switch changes the page at once

Expect: choosing Light or Dark in Settings → Appearance sets the page's theme and keeps the choice.

Result: pass. Light: `<html>` without `dark`, `color-scheme: light`, `dots.theme` = `light`. Dark: `dark`, `color-scheme: dark`, `dots.theme` = `dark`.

### 2. No accessibility violations in either theme

Expect: axe reports nothing on the home (badges in both tones, a question, run cards), the Inbox, Library, Settings, the dot editor and a run's page, in Light and in Dark.

Result: pass. "no violations" on all 12 (6 screens × 2 themes).

### 3. No console errors

Result: pass. No error, warning or uncaught exception while switching themes and visiting the 12 screen/theme pairs.

### 4. The choice survives a restart, and the title bar follows

Expect: with Dark chosen, after the app is stopped and started again the page opens dark, before anything is clicked; the title bar is dark; switching to Light makes it light.

Result: pass. Before the restart the title bar's dark-mode attribute was 1; after it `<html>` was `dark` with `dots.theme` `dark` and the attribute 1; after choosing Light the attribute was 0. The choice was then set back to System.

### 5. The look

Expect: the Deep Teal palette in both themes; the person's messages in bubbles, the dot's text under a "● name · time" line, tool steps as chips (a call sent for approval in the needs colour), questions in soft cards, run cards with a left rule; the rail's active pill and amber counts; Settings' Appearance control.

Result: pass, from the screenshots of the six screens in both themes (`light-home.png`, `dark-home.png`, `light-settings.png`, `light-runs.png`, …).

## Findings

None about Dots.
