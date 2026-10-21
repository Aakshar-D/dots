# Browser use for dots

Date: 2026-10-21. Status: agreed with the person in a question round, section by section; written for their review.
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md`, the MCP/skills spec `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` and the check-ins spec `docs/superpowers/specs/2026-10-20-checkins-followups-design.md` (section 12 lists the passages it changes). Source of the idea: Grok Bot and OpenAI dots, whose agents work websites that have no API or connector.

## 1. Why

A dot reads and writes files, runs commands, and calls services that have an MCP server. It cannot use an ordinary website: a portal, a shop, a form, a job board, a bank. The person wants dots that read public sites, work inside their logged-in accounts, fill and submit forms, download files, and do any browser task the person asks and authorizes.

This spec gives a dot a real browser: Microsoft Edge in a visible window, with the dot's own profile, driven by Microsoft's Playwright MCP server, behind a gate in Dots that knows the page's address and each button's real label.

Decisions (agreed with the person):

| Question | Decision |
|---|---|
| What browser dots do | Read public sites, work in logged-in accounts, fill and submit forms, download files; any task the person asks and authorizes |
| Whether the person sees the browser | Always: the window is visible whenever a dot browses |
| How actions are authorized | Sites and risky clicks: reading is free; acting on a site needs that site approved; a risky action (pay, buy, send, delete, submit, …) asks even on approved sites unless risky actions are allowed for that site |
| How the browser is driven | Hybrid: Dots launches Edge and owns its window and profile; Playwright MCP (`@playwright/mcp`, pinned) drives it over the debugging protocol; Dots' gate checks every call using the page address it reads itself and the labels in Playwright's own snapshots |
| Profiles | One persistent profile per dot; logins stay between runs |
| Passwords | Dots never types them: a dot hands over to the person with `ask`; typing into a password field is refused |

Out of scope: a hidden (headless) mode; Chrome, Firefox or other browsers; more than one profile per dot; teach-by-demonstration; a password manager; cloud browsers; Playwright MCP's opt-in capabilities (`--caps`: vision, pdf, devtools, network, storage, config, testing).

Anti-goals: a dot types a password or reads cookies or storage; a risky action is performed without the person's approval because the model described the button differently or named it by a selector; a browser action approved late is performed on a page that has moved on; one dot sees another dot's logins.

## 2. The parts

### 2.1 Browser host (`browser/host.rs`)

- On a dot's first browser call, Dots starts Microsoft Edge (`msedge.exe`), visible, with:
  - `--user-data-dir=<data_dir>/browser/<dot_id>/profile`;
  - `--remote-debugging-address=127.0.0.1 --remote-debugging-port=<a free port>`;
  - `--no-first-run --no-default-browser-check`.
- Edge is found at `Settings.edge_path` when set, else at `%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe`, else at `%ProgramFiles%\Microsoft\Edge\Application\msedge.exe`.
- One Edge per dot, shared by the dot's runs: a later run of the same dot reuses it. Edge is closed when the last run of the dot that used it ends. An Edge the person opened with "Open the browser" (section 7) is the same managed Edge: runs reuse it, and it stays open until the person closes it.
- Dots keeps its own connection to Edge's debugging endpoint, used only to read the address of the page a call acts on (section 3.2). Dots never clicks or types through it.
- The host is behind a trait (`BrowserHost`) so tests use a fake browser.

### 2.2 Browser server

- For each run of a dot with `browser: true`, at its first browser call, Dots starts:

  ```
  npx -y @playwright/mcp@0.0.83 --cdp-endpoint http://127.0.0.1:<port> --output-dir <run workspace>/downloads
  ```

  with no `--caps`. The pinned version is a constant (`browser::PLAYWRIGHT_MCP`).
- It is a built-in MCP server named `browser`, not a Library item. It is started, called and stopped by the MCP client and per-run sessions that serve Library servers (`mcp/`, `run_tools.rs`): on the Claude engine its tools are `mcp__browser__<tool>`, reached through the gateway route `/mcp/s/browser`; on the local engine they are bridged as Library tools are (`engine/local/mcp_tools.rs`).
- Its catalog is not fingerprinted or held as a Library server's is: its version is pinned, and the gate (section 3) decides every call.
- It stops when the run ends.

### 2.3 Browser gate (`browser/gate.rs`)

Every call of a `browser` tool is decided by the gate before the dot's policy (section 3). The gate is pure code over the call, the run's snapshot map (section 3.3) and the page address the host reports.

### 2.4 The dot setting

`DotSpec.browser: bool`, default `false`. Only a dot with `browser: true` gets the `browser` server. The dot's first prompt gains a `## Browser` section (section 4).

## 3. The gate

### 3.1 Kinds of call

| Kind | Tools of `@playwright/mcp@0.0.83` (default capabilities) | Rule |
|---|---|---|
| Read | `browser_snapshot`, `browser_take_screenshot`, `browser_wait_for`, `browser_navigate_back`, `browser_console_messages`, `browser_find`, `browser_resize`, `browser_emulate_media`, `browser_close`, `browser_tabs` (except `new` with a URL) | allowed |
| Go to a site | `browser_navigate`, `browser_tabs` `new` with a URL | allowed when the target site is approved (`Browser(<site>)`); otherwise asks |
| Act | `browser_click`, `browser_type`, `browser_fill_form`, `browser_select_option`, `browser_press_key`, `browser_hover`, `browser_drag`, `browser_drop` | allowed when the current page's site is approved; otherwise asks; risky (section 3.4) asks unless `BrowserRisky(<site>)` |
| Always risky | `browser_handle_dialog` (accepting a confirm dialog) | asks unless `BrowserRisky(<site>)` |
| Dangerous | `browser_evaluate`, `browser_run_code_unsafe`, `browser_file_upload`, `browser_network_request`, `browser_network_requests`, and any tool not named in this table | always asks; no rule allows them |

A `deny` rule (`Browser(<site>)` deny) refuses whatever the kind.

### 3.2 Sites

- A site is the host of a URL, lower-cased, without a port; `www.` is dropped.
- A rule `Browser(shop.com)` matches `shop.com` and every subdomain (`app.shop.com`). `Browser(*)` matches every site.
- The site of a `browser_navigate` call is the site of its `url`.
- The site of an act is the site of the page it acts on: the address of the run's current tab, which the host reads over its own connection at the time of the call. A page whose address has no host (`about:blank`, `file:`, `data:`) is no site: an act there asks.

### 3.3 Labels from Playwright's own snapshots

- Every result of a `browser` tool passes through Dots. When a result holds a page snapshot (Playwright writes it as an indented list of lines such as `- button "Pay now" [ref=e12]`), Dots reads each line's role, name and `ref` into the run's snapshot map, replacing the map of an earlier snapshot of the same tab.
- When an act names its target (`target` or `ref`), the gate looks up that reference in the map: its role and name are what the gate judges. The model's own `element` description is not used.
- A target that is not a reference in the map (a reference from an old snapshot, a guessed one, or a selector such as `text=Pay`) is judged risky.
- A snapshot saved to a file (`filename`) is not read; acts on its references are then risky.

### 3.4 Risky

An act is risky when its target's name or role contains, as a whole word and ignoring case, one of: `pay`, `payment`, `buy`, `purchase`, `checkout`, `order`, `send`, `delete`, `remove`, `submit`, `transfer`, `confirm`, `publish`, `post`, `sign`, `subscribe`, `unsubscribe`, `cancel`, `withdraw`, `donate`, `book`, `reserve`, `place` (the list is `browser::RISKY_WORDS`). `browser_handle_dialog` is always risky. `browser_press_key` with `Enter` is not risky on its own (search boxes); this is a known limit (section 11).

### 3.5 Passwords

`browser_type` and `browser_fill_form` into a target whose name contains `password`, `passcode`, `pin`, `one-time`, `otp`, `verification code` or `security code` (whole words, ignoring case), or whose role is a password field, are refused with: "Dots does not type passwords or codes. Ask the person to enter it in the browser window, with your ask tool." No rule allows it.

### 3.6 Asking

- An ask is an ordinary approval (Approvals v2): the run waits up to `approval_wait_secs`, the Inbox and the chat show the card.
- The card says what and where: "wants to open **shop.com/cart**", "wants to click **"Pay now"** on **shop.com**", "wants to type into **"Search"** on **shop.com**". It offers: allow once; always allow on `<site>` (`Browser(<site>)`); for a risky act also always allow risky actions on `<site>` (`Browser(<site>)` and `BrowserRisky(<site>)`); deny.
- **A browser approval counts only while its run waits.** An approval decided after the run parked is not performed later: its apply state is `skipped`, and the resumed run is told "Your browser action was approved after the page may have changed. Take a snapshot and do it again if it is still right." (Rules chosen with it are saved.) This differs on purpose from the late approvals of local dots in Approvals v2.

## 4. The run

- The dot's first prompt gains, when `browser` is on:

  ```
  ## Browser

  You have a browser (tools named browser_*). It is a visible Microsoft Edge window with your
  own profile; logins you were given stay there. Take a snapshot before you act, and act on the
  references in it. Opening a site you have not used before, and actions such as paying,
  sending, deleting or submitting, wait for the person's approval. Never type a password or a
  code: when a page needs a login, 2FA or a CAPTCHA, call ask with "Please <do it> in the browser
  window, then answer done." Files you download land in downloads/ in your workspace.
  ```

- Downloads land in `<run workspace>/downloads`; the run view lists them (section 7).
- The person's handover: the window is always visible, so the person logs in, enters the code or solves the CAPTCHA there, then answers the dot's question; the answer resumes the run (check-ins spec section 5), and the login stays in the dot's profile.

## 5. Errors

| Situation | What happens |
|---|---|
| Edge not found | The browser call fails: "Microsoft Edge was not found; browser use needs it (Settings → Edge path)." The run goes on |
| `npx` not found, or the package cannot be fetched | The call fails with the reason ("Browser use needs Node.js (npx)" or the fetch error). The run goes on |
| The person closes the Edge window during a run | The next browser call starts Edge again on the same profile and reconnects the server; the dot sees a fresh page |
| The Playwright server exits | As any MCP server of a run: the call fails, the next call starts it again |
| The profile is in use by an Edge started outside Dots | The call fails: "This dot's browser profile is in use by another Edge window; close it and try again." |
| The page address cannot be read for an act | The act is treated as on no site: it asks |

## 6. Data

- Migration `0009_browser.sql`: `ALTER TABLE dots ADD COLUMN browser INTEGER NOT NULL DEFAULT 0`. The number is checked against `master` before merging and renumbered if taken.
- `Settings.edge_path: Option<String>`, default `None`.
- Profiles: `<data_dir>/browser/<dot_id>/profile`, removed when the dot is deleted.
- Policy rules `Browser(<site>)` and `BrowserRisky(<site>)`, with `allow` or `deny`; validated as `Context(<name>)` rules are (a site: a host name, or `*`). They never reach the CLI's `--allowedTools` / `--disallowedTools` (`policy::cli_lists`). Presets add none.

## 7. Screens

- **Dot editor:** a "Browser" switch: "Opens a visible Microsoft Edge window with this dot's own profile." Under it, "Sites": the approved sites, each with "risky actions allowed" when set, and Remove; and "Open the browser", which opens the dot's Edge so the person can log in ahead of time.
- **Approval card:** the wording of section 3.6.
- **Run view:** `mcp__browser__*` calls as one line each: "Opened shop.com/cart", "Clicked "Add to cart"", "Typed into "Search"", "Took a screenshot" (shown as an image), "Downloaded statement.pdf".
- **Settings:** a status line ("Microsoft Edge found" or not; "Node.js found" or not) and the field "Edge path".

## 8. Commands, events and contract

Commands: `browser_open(dot_id)` (opens the dot's Edge for the person), `browser_status()` (`{ edge: Option<String>, npx: Option<String> }`). `DotSpec.browser`, `Settings.edge_path` and the two rule kinds go through the existing dot and settings commands. All in `apps/dots-app/src/contract.rs` and `ui/src/api/__fixtures__/contract.json`.

## 9. Testing

Tests are written first, in the repo's patterns.

- Gate, pure: every tool's kind (and an unknown tool as dangerous); site parsing and matching (`www.`, ports, subdomains, `*`, `about:blank`); reading snapshot lines (nested items, quotes in names, several tabs, unknown references, selectors); risky words as whole words in many phrasings; refused password typing; deny over allow.
- Gateway: a fake Playwright server (an extension of `fake-mcp`) returns snapshots and receives calls. An approved site passes; a new site asks; "Pay now" asks on an approved site and passes with `BrowserRisky`; a selector target asks; a late approval is `skipped` and not performed; password typing is refused; `browser_evaluate` asks.
- Host: behind `BrowserHost`; a fake browser (a test binary answering enough of the debugging protocol to report the current tab's address) covers starting, reuse by a second run of the dot, closing after the last run, and a closed window.
- Policy: the two rule kinds validate, stay out of `cli_lists`, and no preset carries them.
- UI (Vitest): the editor switch and Sites list, the approval card wording, the run view lines.
- Smoke, `docs/verification/<date>-browser-smoke.md`, with real Edge and real Playwright MCP, a local dot and a Claude dot: read a public page; a first-time site approval; add to cart on a demo shop with "Place order" asking; a login handover through `ask`; a download landing in the workspace.

## 10. Avoiding clashes

Branch `feat/browser`. New code in `crates/dots-core/src/browser/` and new UI files. Shared files get small edits only: one call to the gate in `server/gateway.rs` and in `engine/local/mcp_tools.rs`; the built-in server in `run_tools.rs`; rule validation and `cli_lists` in `policy.rs`; the `browser` field in `model.rs` and the store; the prompt section in `prompt.rs`; the editor and Settings screens.

## 11. Open risks

- **Playwright MCP's output format.** The gate reads its snapshot lines; a change of format in a later version would leave references unknown, so acts would ask (safe, but noisy). The version is pinned; a version bump comes with a gate test on its output.
- **`Enter` submits forms.** `browser_press_key` with `Enter` is an act, not a risky one, so on an approved site a dot can submit a focused form with it. A person who wants that to ask sets the site to ask (`Browser(<site>)` not allowed).
- **Labels can mislead.** A button whose name says nothing risky ("Continue") can still place an order. Site approval is the main boundary; the risky words are a second one.
- **Visible windows.** A dot's Edge window appears while the person works; the person chose this.
- **The door's secret.** Since section 14 Edge has no port: Dots holds its only connection, and a run's Playwright comes in through a door on Dots' server whose secret lets one connection in. The secret is in the Playwright server's environment, not on its command line, but a program running as the person can read another process's memory, take the secret and come in before Playwright does (Playwright's call then fails). On a computer where such programs run, a dot whose policy lets it run commands should still not be given the browser of a dot whose logins matter.
- **Dots' server down.** Playwright reaches Edge through Dots' local server: when the server could not start (its port taken), browser calls fail on both engines.
- **Keys that activate.** `Tab` then `Enter` or `Space` can activate a focused risky button without a click the gate judges; site approval is the boundary for it.

## 12. Changes to the other specs

- Main spec §1, out of scope: browser use is no longer out of scope (this spec).
- Main spec §5, policy: rule kinds `Browser(<site>)` and `BrowserRisky(<site>)`, decided by Dots' browser gate before the dot's other rules for `browser` tools.
- MCP/skills spec §2: a built-in MCP server, `browser`, is started by Dots for dots with `browser` on; it is not a Library item and its catalog is not fingerprinted.
- Check-ins spec §2: a dot's handover for logins, 2FA and CAPTCHAs uses `ask`.

## 13. Changes made while building

The plan (`docs/superpowers/plans/2026-10-21-browser.md`) changed these from the first version of this spec, after measuring Edge 155 and `@playwright/mcp@0.0.83` on this PC:

- Edge also starts with `--disable-sync`, `--disable-backgrounding-occluded-windows`, `--disable-renderer-backgrounding`, `--disable-background-timer-throttling` and `--disable-features=CalculateNativeWinOcclusion`, and opens `about:blank`. A fresh profile signed in to the person's Microsoft account and synced their extensions into the dot's profile; behind other windows, every click timed out (section 2.1).
- Edge's `msedge.exe` hands over to the browser and exits at once, so Dots follows a dot's Edge by its port, kept in `<data_dir>/browser/<dot_id>/port` (a Dots that starts again finds an open Edge), and closes it with `PUT /json/close/<id>` for each page. A profile is in use when its `lockfile` cannot be opened for writing (section 2.1). Section 14 replaced the port and its file: Dots holds Edge's pipe and closes Edge with `Browser.close`.
- The Playwright server also gets `--snapshot-mode none`, so acts do not write snapshot files into `downloads/`; the model takes snapshots with `browser_snapshot`, which still answers inline. Its tool list is the captured one (`browser/tools.json`), served without starting the server; its `list_changed` is not followed (section 2.2).
- Acts name their element with `target`, a snapshot reference or a selector; `ref` does not exist in 0.0.83. A password field shows as `textbox "Password"`, so the name decides (section 3.5).
- The page an act lands on is the one Playwright reported as current last (its `### Page` section or `(current)` tab), counted only while Edge's `/json/list` has exactly that address; otherwise the act is on no site and asks. `/json/list` is ordered by what was last in front, not by Playwright's tab. Only Playwright's own result sections move the current page: console messages, found text and the results of code cannot (section 3.2). Since section 14 the addresses are Edge's `Target.getTargets` (type `page`), read over its pipe.
- `browser_drop` with `paths` is dangerous, as an upload; `browser_handle_dialog` with `accept: false` is a read (section 3.1).
- "Always allow risky actions on <site>" saves `BrowserRisky(<site>)` alone, and an allow `BrowserRisky(<site>)` also approves the site. For browser calls a deny rule wins, then an ask rule (a `Browser(<site>)` ask keeps that site asking under `Browser(*)`), then an allow rule. A deny rule on the tool's own name (`mcp__browser__*`) refuses it too (sections 3.6 and 6).
- A live approval whose page changed while it waited is not performed either: "The page changed while your browser action waited for approval, so it was not performed. Take a snapshot and do it again if it is still right." (section 3.6).
- The card's data (`action`, `site`, `page`, `labels`, `risky`, `dangerous`) is stored in the approval's input under `dots_browser`; a key of that name in the model's input is dropped first. A card offers no rule for a page with no site or a dangerous call (section 3.6).
- Edge saves downloads to `<data_dir>/browser/<dot_id>/downloads` (written into the profile's `Default/Preferences`); Dots moves each finished file into the run's `downloads/` after each browser call and when the run ends. Over CDP, Playwright's output folder does not catch downloads (section 4).
- A performed call's result starts with `Dots: <line>` (`Opened shop.com/cart`, `Clicked "Add to cart"`, `Typed into "Search"`, `Took a screenshot`, `Downloaded statement.pdf`); the run view shows those lines. A screenshot is read back with the command `browser_screenshot(run_id, name)`, only under the name Playwright gives one (sections 7 and 8).
- A new `Settings.edge_path` is confirmed in a native dialog ("Change the Edge program?"), as a new Claude program is (section 6).
- A package that cannot be fetched fails with "The browser could not start: <MCP error>"; npm's own text goes to the app log (section 5).
- A Library MCP server the dot uses under the name `browser` keeps it; the built-in browser is then left out of the run, with a notice (section 2.2).
- The fake Playwright server of the tests is `fake-mcp` with a script; `fake-browser` stands in for Edge (section 9).
- After the final review: every form field that can take text (all but `checkbox` and `radio`) is checked for a password, since Playwright fills a `slider` as a `textbox`; nothing is learned from an error result or from text after `### Modal state`, where a page's dialog message is written unescaped; every snapshot or find replaces the page's references and one saved to a file forgets them (Playwright numbers references per document); the element an act last landed on is remembered, and while it is a password or code field only `Tab`, `Shift+Tab`, `Enter`, `Escape` and the arrow keys are pressed (section 3.5).
- `browser_run_code_unsafe` is refused, not asked: it runs a program in Playwright's own process, outside the page and the workspace (section 3.1).
- The Playwright server starts in `<data_dir>/browser/playwright`, not in the workspace, where a dot could plant a `node_modules` or an `.npmrc` for `npx` (section 2.2).
- A browser approval is one whose input carries Dots' card (`dots_browser`); the card is dropped from every Library server's input, so a Library server named `browser` cannot show a browser card, and its late approvals are not skipped (section 3.6).
- On the Claude engine a call must finish by the time the CLI stops waiting; the time left is reckoned just before the call is performed, and a call with none left is not performed (section 3.6).
- The approval card lists what the call sends: the full address, the text typed, the fields filled, files and data, and code, each cut at 300 characters (section 7).

## 14. A private connection to Edge

Agreed with the person after the final review (finding 7: the debugging port had no password, so any program on the computer, a dot's shell tool included, could drive a dot's logged-in Edge around the gate). This section replaces the port of section 2.1 and section 13's port file.

- **No port on Edge.** Dots starts Edge through a small Node helper (`browser-pipe.mjs`, written by Dots into `<data_dir>/browser/`), run as `node browser-pipe.mjs <msedge.exe> <edge args…>`. The helper starts Edge with `--remote-debugging-pipe` (and `--edge-skip-compat-layer-relaunch`, without which `msedge.exe` hands over to another process and the pipe is lost) on its file descriptors 3 and 4, and relays them to its own stdin and stdout: messages are JSON, each ended by a NUL byte. Edge opens no listening port. When the helper's stdin closes (Dots stopped), the helper closes Edge.
- **Dots' own reads go over the pipe**: the pages of a dot's Edge are `Target.getTargets` (type `page`), and closing it is `Browser.close`. `/json/list` and `/json/close` are not used.
- **One door for Playwright.** Dots' local server gains a WebSocket route, `/cdp/<secret>`. Each run's browser gets a fresh random secret (128 bits, as a run secret is made) when its Playwright server starts, and the server is started with `--cdp-endpoint ws://127.0.0.1:<dots port>/cdp/<secret>`. The route accepts **one** connection per secret, the first; a second connection, a wrong or a spent secret is refused (HTTP 404) and logged. Dots relays between that connection and the dot's Edge pipe, renumbering message ids so that its own requests and Playwright's do not collide; events go to Playwright.
- **One run at a time per dot.** A dot's browser serves one run at a time: a browser call of a second run while another run of the dot holds the browser fails with "This dot's browser is in use by another run; try again when it ends." The browser is held from a run's first browser call until it ends, as section 2.1 says.
- **Edge lives as long as Dots.** No port file; a Dots that starts again starts a new Edge on the dot's profile. "Open the browser" opens the managed Edge the same way.
- **Errors.** Node not found: the call fails with `NO_NODE`, as for the Playwright server. The helper exits or Edge closes: the next browser call starts both again (the person closing the window, section 5).
- **Testing.** A fake Edge that speaks the pipe protocol (the `fake-browser` test binary, reworked) covers: pages read over the pipe, close, one connection per secret, a wrong secret refused, a second run of a dot refused while the first holds the browser, and ids of Dots' own requests and Playwright's kept apart. The smoke adds: no `msedge.exe` of a dot listens on any TCP port (`Get-NetTCPConnection -State Listen`).

The plan (`docs/superpowers/plans/2026-10-21-browser-pipe.md`) built this section with these changes, after measuring Edge 155, Node.js 24 and `@playwright/mcp@0.0.83` (Playwright 1.64) on this PC:

- The Playwright server finds its door in the variable `PLAYWRIGHT_MCP_CDP_ENDPOINT`, which 0.0.83 reads as `--cdp-endpoint`, not on its command line: any program can list command lines (`Get-CimInstance Win32_Process`).
- When a Playwright goes, Dots detaches the sessions Edge attached for it and turns Edge's auto-attach off; a new Playwright on the same Edge lets the one before go first. Measured: otherwise the next Playwright found no page, since the pipe is one connection for the whole life of Edge.
- A secret is made for each start of a Playwright server. A request with the right secret that is not a WebSocket spends the door and gets the upgrade's own rejection. A Playwright whose connection ended cannot come in again: the run's next browser call starts a new server with a new door.
- Edge closes when the run that holds it ends, unless the person opened it, and when Dots stops: the helper's stdin ends, it sends `Browser.close` and ends Edge after 5 s. The helper puts `--remote-debugging-pipe --edge-skip-compat-layer-relaunch` before Dots' switches.
- A message may be up to 256 MB either way; a longer one from Edge is dropped and logged.
- Node.js missing fails the call with the sentence of `NO_NODE` before anything starts.
- `fake-browser` speaks the pipe on its stdin and stdout, or on its descriptors 3 and 4 under the real helper (the helper's test and the runtime's tests need `node`); its test controls are on an HTTP port of its own that Dots never uses. The pipe's own logic is tested with the test playing Edge in memory.
- The smoke adds: no `msedge.exe` whose command line names a dot's profile holds a listening port (`Get-NetTCPConnection -State Listen -OwningProcess <pid>` for each); no Playwright command line holds `/cdp/`; a browser call of a second run of the dot during a run fails with "This dot's browser is in use by another run; try again when it ends."; quitting Dots closes the dot's Edge.
