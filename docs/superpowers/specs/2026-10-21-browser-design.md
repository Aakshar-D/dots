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

## 12. Changes to the other specs

- Main spec §1, out of scope: browser use is no longer out of scope (this spec).
- Main spec §5, policy: rule kinds `Browser(<site>)` and `BrowserRisky(<site>)`, decided by Dots' browser gate before the dot's other rules for `browser` tools.
- MCP/skills spec §2: a built-in MCP server, `browser`, is started by Dots for dots with `browser` on; it is not a Library item and its catalog is not fingerprinted.
- Check-ins spec §2: a dot's handover for logins, 2FA and CAPTCHAs uses `ask`.
