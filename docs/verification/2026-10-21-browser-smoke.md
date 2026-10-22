# Browser use: smoke test

- Date: 2026-10-21
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/browser` at commit `90a0fb9` (the browser plan, its review fixes, the private connection to Edge of spec section 14 and its review fixes). Windows 11 on ARM (`aarch64-pc-windows-msvc`), Bun 1.4.3, LLVM for `ring`.
- Browser: Microsoft Edge 155.0.4283.45 (`C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`), started by Dots through `browser-pipe.mjs` under Node.js 24.18; `@playwright/mcp@0.0.83` (Playwright 1.64) from the npx cache.
- Steps 1 and 2 were run earlier the same day on the build before section 14 (Edge on a debugging port), with the local model `qwen2.5:7b` in Ollama 0.40.7. Steps 3 to 7 were run by the agent at the person's request with a real Claude dot (`claude` CLI 2.1.306, model `sonnet`) on the build above.
- Steps: 7. Steps 1 to 7 as expected in what Dots did, except Step 7's password typed by a person, which is open for the person (the agent cannot type into the Edge window). 3 findings, 1 of them a possible Dots defect that did not happen again.
- Screenshots: none taken; the run events quoted below are the record.

## What this covers

Spec section 9 asks for a smoke with real Edge and real Playwright MCP: reading a public page, a first-time site approval, adding to cart on a demo shop with "Place order" asking, a login handover through `ask`, and a download landing in the workspace. Section 14 adds: no dot's Edge listens on a port, no command line holds a door's secret, a second run of the dot is refused while one holds its browser, and quitting Dots closes the dot's Edge.

## How the agent steps were run

- The app is the debug build `target/debug/dots-app.exe`, started with `DOTS_DATA_DIR` and `WEBVIEW2_USER_DATA_FOLDER` set to fresh folders in the run's scratchpad (`.../scratchpad/br2/data`, `.../br2/webview`). The real data folder `%LOCALAPPDATA%\dots` and the person's own Edge profile were never used: every dot's profile is `<DOTS_DATA_DIR>\browser\<dot_id>\profile`.
- For Steps 3 to 7 the app's `PATH` held `~/.local/bin` (the `claude` CLI), as the person asked for a Claude dot.
- The window is driven through WebView2's debugging port 9333 with `tools/smoke/webview.mjs` (`eval` with `invoke(...)`), as the earlier smokes did. Approvals are decided with `decide_approval` (with `allowRule` for the "Always allow" choices), the question answered with `message_reply`.
- The demo shop is a local page served from the scratchpad on `127.0.0.1:47999` (`/` with "Add to cart", "Cart", "Log in", "Download statement"; `/cart` with "Place order", which sets the title to "ordered"; `/login` with Email and Password, setting the cookie `who=me`; `/statement.pdf` as an attachment holding `%PDF-1.4 demo`).
- The Claude dot `shopper` (`01M5FSDKDWA5ZXEENANA97QPA7`): `engine: 'claude'`, `model: 'sonnet'`, `workspace_mode: 'folder'` on `.../br2/work`, `approval_wait_secs: 240`, Browser on, no site; its instructions set per step with `update_dot`, each beginning "Use your browser tools (mcp__browser__*)."
- Checks of section 14 (PowerShell): `Get-CimInstance Win32_Process` for every `msedge.exe` whose command line names the data folder, then `Get-NetTCPConnection -State Listen` owned by any of them; and every process whose command line holds `/cdp/`.

## Steps

### 1. The status, and Edge where Settings says

Expect: the Browser section shows Edge and Node.js found; a wrong Edge path fails a browser call with "Microsoft Edge was not found; browser use needs it (Settings → Edge path)."

Result: pass (earlier build). On this build `browser_status` again answered `{"edge": "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", "npx": "C:\\Program Files\\nodejs\\npx.cmd"}`.

### 2. A local dot reads a public page, after a first-time site approval

Expect: the card "reader wants to open **example.com**"; "Always allow on example.com" saves `Browser(example.com)`; the summary names "Example Domain"; the Edge window closes when the run ends.

Result: pass (earlier build, local model): the card, the rule saved by the click, "Example Domain" in the summary, and Edge closed after the run. The same first-site card and rule on this build are in Step 3.

### 3. Add to cart on the demo shop, and "Place order" asks

Expect: the card for the site 127.0.0.1; then "Add to cart" without a card; then the card "wants to click **"Place order"** on **127.0.0.1**" with "always allow on 127.0.0.1" and "always allow risky actions on 127.0.0.1". Deny: the run is told "Denied by the reviewer." and no order is placed. Run again and approve: "Clicked "Place order"", the title "ordered".

Result: pass, after a first run whose clicks timed out (finding 1).

- Run `01M5FSDKQ5NR9BF3QPXW6HD59P`: the pending approval `mcp__browser__browser_navigate` with `dots_browser` `{"action":"open","site":"127.0.0.1","risky":false}`; its rule candidate `Browser(127.0.0.1)` ("always allow on 127.0.0.1"). At that moment 15 `msedge.exe` processes ran on the dot's profile, **0 listening ports**, and **0 processes with `/cdp/` on their command line**. Approved with the rule: "Dots: Opened 127.0.0.1", "Dots: Read the page" (title "Demo shop"); then both clicks ("Add to cart", `e3`; "Cart", `e4`, sent in parallel) and a retry failed with `TimeoutError: browserBackend.callTool: Timeout 5000ms exceeded. … waiting for element to be visible, enabled and stable`. The run ended saying nothing was ordered.
- Run `01M5FSQJ30PJV4D5YV6RK3E7KY` (the dot's Edge opened with "Open the browser" beforehand): "Clicked "Add to cart"", "Clicked "Cart"", then the pending `browser_click` with `{"action":"click","labels":["Place order"],"page":"127.0.0.1/cart","risky":true}` and the candidates `Browser(127.0.0.1)` and `BrowserRisky(127.0.0.1)`. Denied: the result "Denied by the reviewer."; the summary "the "Place order" click was denied by the reviewer, so no order was placed."
- Run `01M5FSSQ08N72N8Y1C5M10YSS3`: the card again; approved with "always allow risky actions on 127.0.0.1": "Dots: Clicked "Place order"", and the summary "The page title then changed to "ordered"". The dot's policy now holds `Browser(127.0.0.1)` and `BrowserRisky(127.0.0.1)` (Step 7's rule).

### 4. A download lands in the workspace

Instructions: "Open http://127.0.0.1:47999/, take a snapshot and click Download statement, then take a snapshot."

Expect: the workspace's `downloads/statement.pdf` holds `%PDF-1.4 demo`; "Downloaded statement.pdf"; nothing new in the person's Downloads folder.

Result: pass. Run `01M5FSTPXYTEQPP99VNBRGXH6W`: "Dots: Clicked "Download statement"" and "Dots: Downloaded statement.pdf"; `work/downloads/statement.pdf` holds `%PDF-1.4 demo`. Run `01M5FSTQ5F8EYSW1J28SWDMDBJ`, started at the same moment, was `queued` until the first ended (Dots runs one run of a dot at a time), then "Dots: Downloaded statement (2).pdf". The person's Downloads folder holds no `statement` file.

### 5. The login handover asks, and the password is never typed

Instructions: "Open http://127.0.0.1:47999/login, take a snapshot, type me@example.com into Email and the password hunter2 into Password, then click Log in."

Expect: typing into "Email" goes through; the password is not typed by the dot (a `browser_type` into "Password" is refused with "Dots does not type passwords or codes. …"); the dot asks the person, and the question is in the Inbox; the answer resumes the thread.

Result: pass. Run `01M5FSWXW2PM7C33M3DF9D3W6A`: "Dots: Typed into "Email"" (`fill('me@example.com')`), then no `browser_type` into Password: the dot called `mcp__dots__ask` with "I entered me@example.com in the Email field on http://127.0.0.1:47999/login. Please type the password into the Password field in th…", answered with the `ASKED` sentence; `messages_list` held the open question. The reply "done" made run `01M5FSY7HVMRADCEPHE0XMKM1F`, which continued the thread ("You've confirmed the password is entered and Log in is clicked. …"). An earlier run with the instructions without a password (`01M5FSW028DEJJPRJ3GV4ZYW3K`) typed the Email and clicked "Log in" (the demo form needs no password), which set the shop's cookie.

### 6. A window closed, and a profile in use

Expect: (a) after the dot's Edge window is closed, the next browser call starts Edge again on the same profile; (b) with an Edge started by hand on the dot's profile, the call fails with "This dot's browser profile is in use by another Edge window; close it and try again."

Result: pass.

- (a) The dot's Edge (opened with "Open the browser") was ended with `Stop-Process` on its browser process; no `msedge.exe` of the data folder was left. Run `01M5FT25168G1352KG3JVRYZBT` then "Dots: Opened 127.0.0.1/cart" and "Dots: Read the page" (title "Cart", "1 item").
- (b) `msedge.exe --user-data-dir=<the dot's profile> --no-first-run about:blank` by hand; run `01M5FT32P36Z4JS1A402VRG6PS`: both browser calls answered "This dot's browser profile is in use by another Edge window; close it and try again."; the run went on. That Edge was then closed.

### 7. A real Claude dot, the risky rule, a later run still logged in, and Dots quitting

Expect: "Place order" passes after "Always allow risky actions on 127.0.0.1" (the policy gains `BrowserRisky(127.0.0.1)`); a later run of the same dot is still logged in (the shop's `who=me` cookie in the dot's profile); the person types the password in the Edge window and answers "done"; quitting Dots closes the dot's Edge.

Result: pass, except the password typed by a person, which is open.

- The risky rule: Step 3, run `01M5FSSQ08N72N8Y1C5M10YSS3`.
- Still logged in: run `01M5FSYQE528WJCWJCDWQ64G1V` ("use browser_evaluate with the function () => document.cookie"): the call asked as code (`{"action":"run_code","dangerous":true}`) even with `BrowserRisky(127.0.0.1)` in the policy; approved once, it answered `who=me`.
- The handover: the question and the resumed thread are Step 5; typing the password into the window is for a person at the machine.
- Dots quitting: with the dot's Edge opened by "Open the browser" (9 `msedge.exe` processes, 0 listening ports, 0 command lines with `/cdp/`), the app was ended with `taskkill /F`. Within 2 s no `msedge.exe` of the data folder and no `node` running `browser-pipe.mjs` was left.
- A second run while one holds the browser ("This dot's browser is in use by another run; try again when it ends.") was not seen: Dots queues a dot's runs (Step 4). It is covered by `browser_call_test::a_dots_browser_serves_one_run_at_a_time`.

## Findings

1. The first run of the smoke (`01M5FSDKQ5NR9BF3QPXW6HD59P`) could read the shop but every click timed out waiting for the button to be "visible, enabled and stable". It did not happen again in the seven later runs that clicked, among them a first run on a new profile (`01M5FSZMN7G5DV1DXZAY1YD1VR`) and two parallel clicks (`01M5FT0M29DCF7G0P8389SETSG`). To place it, the same clicks were run outside the app: Playwright's own pipe launch; a pass-through relay with `connectOverCDP`; `@playwright/mcp@0.0.83` on that relay, with Node started with and without a window; and real Edge through Dots' own `EdgeHost`, door and relay driven by `@playwright/mcp@0.0.83`. All clicked "Add to cart" (title "added"), and `requestAnimationFrame` fired. The cause is not known; it was the first Edge the app started after a rebuild. If it comes back, the first thing to read is whether the page renders frames (`requestAnimationFrame`) and whether the window is in front.
2. Edge starts at a dot's first browser call, before the gate's card is decided (15 Edge processes ran while the first-site card waited). This is the deferred minor "Edge starts even for calls that will be refused" of the browser plan's review.
3. The Claude dot chose `mcp__dots__ask` for the password without trying `browser_type`, so the refusal sentence itself was not seen in a run. It is covered by the gate's tests (`PASSWORD_REFUSAL`).
