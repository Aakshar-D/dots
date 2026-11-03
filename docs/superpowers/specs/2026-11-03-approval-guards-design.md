# Approval guards: private folders, the always-allowed list, harmless tools

- Date: 2026-11-03
- Status: approved (design decisions given by the owner, 2026-11-03)
- Builds on: the per-dot policy (`policy.rs`), "Approve and always allow" (`allow_candidates`, `Runtime::decide_approval`), the Claude engine's argument list (`engine/claude/args.rs`).

## 1. Goal

Three small guards around approvals:

1. A call that touches a private folder (keys, tokens, credentials) is never approved without the person seeing it, whatever rule or preset would allow it.
2. The person can see, in one place, what a dot is always allowed to do, and take any of it back.
3. Harmless bookkeeping tools of the model (its to-do list, its plan mode, starting a sub-agent) never stop a run to ask.

## 2. Words

- **Private place**: a folder or file from the fixed list in §3.1.
- **Touches a private place** (a call): a file tool's path (or a Glob/Grep pattern) names a private place or something inside one; a search or listing (`Grep`, `Glob`, `LS`) starts in a folder that holds one (`~`, `~/.config`, or the run's folder when it is such a folder); or a shell command's text mentions one. Reading one file in the home folder is not a search of it.
- **Plumbing tool**: a tool from the fixed list in §5.

## 3. Private-folder guard

### 3.1 The list (fixed, not editable)

- In the home folder (`~`): `.ssh`, `.aws`, `.gnupg`, `.kube`, `.docker`, `.config/gh`, `.azure`, `.claude/.credentials.json`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`.
- Secret files, wherever they are: `.env`, `.env.<something>` (also `.env.example`), `.env-<something>`, `.envrc`, `<name>.env`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, and SSH keys by name (`id_rsa*`, `id_ed25519*`, `id_ecdsa*`, so also their `.pub` files).
- Windows: `%APPDATA%\Microsoft\Credentials`, `%APPDATA%\Microsoft\Protect` (and the same under `%LOCALAPPDATA%`, which also holds credential files).
- Dots' own vault folder: `<data dir>/vault`.

### 3.2 Matching

- Case-insensitive on every system (fails closed on Linux and macOS, where the folders are lower-case anyway). `/` and `\` are the same.
- `~` and the folder variables (`USERPROFILE`, `HOME`, `APPDATA`, `LOCALAPPDATA`, `DOTS_DATA_DIR`, in their `%NAME%`, `$NAME`, `${NAME}` and `$env:NAME` forms) are expanded from the environment before matching. Other variables are not: `PATH` could name anything.
- The home-folder entries and the Windows entries match as **path parts anywhere in the path**, not only under the real home folder: `C:\Users\other\.ssh\id_rsa`, `/home/me/.aws/credentials`, `../../.ssh/config` and `project/.npmrc` all count. *My call:* this is simpler than resolving every way of naming the home folder, and a project's own `.npmrc` or `.docker/config.json` holds tokens too. The cost is an extra question for such files, never a missed one.
- Quotes, backticks and `^` are dropped before matching (`.ss''h` is `.ssh`); trailing dots and spaces of a part are dropped (`.ssh.` is `.ssh` on Windows). A command word is also read with its backslash escapes removed (`.s\sh`), and every reading of its brace alternations is judged (`.ss{h,}`, `.{ssh,aws}`). `..` is resolved before a place of two parts is matched (`~/.config/x/../gh`), and the path is matched as written too.
- A wildcard part with a letter or digit in it (`*ssh`, `?ssh`, `[.]ssh`, `.env*`, `*.pem`) counts when it could match a private name. A part without one (`*`, `**`, `*.*`) counts only by where it is: when a private place lies below the folder before it and the pattern has `**` or the part matches the place's next part (`~/*/id_rsa`, `~/**/x`). Otherwise every search would ask.
- The vault matches by its absolute path (after `..` is resolved), and a relative file path is also judged joined to the run's folder.
- Best effort for shell commands, as for deny rules today: a command can still build a path at run time. The worktree and the CLI lists stay the real boundary.

### 3.3 Behaviour

1. **Nothing to save.** `allow_candidates` offers no rule for a call that touches a private place, and no candidate whose own pattern names one or covers one (a `dir/**` whose `dir` holds a private place, such as the home folder).
2. **Refused if sent anyway.** `Runtime::decide_approval` and `Store::approve_with_rule` refuse to save an allow rule for such a call, or a rule whose pattern names or covers a private place, with `Invalid("this touches a private folder, so it always asks: no rule can allow it")`. Nothing is decided then (the person can still approve once).
3. **Always asks.** At check time (`Policy::resolve_in`), a call that touches a private place and would be allowed (by an allow rule, by `default: allow`, by any preset including Trusted) becomes **ask**. A deny rule still wins (deny). A one-time approval still works: the one call goes through.
4. **The card says why.** The approval card of such a call shows, where "Approve and always allow" would be: **"This touches a private folder, so it always asks."** A new command `approval_touches_private(id) -> bool` tells the card.

### 3.4 The Claude CLI

The CLI allows a call that matches `--allowedTools` itself, without asking Dots (a preset's bare `Read` would read `~/.ssh/id_rsa`). So every Claude run also gets `--settings {"permissions":{"ask":[...]}}` with ask rules for the private places: for `Read` and `Edit` (the CLI applies `Edit` rules to every editing tool and ignores `Write` ones) `Tool(//**/<place>)` and `Tool(//**/<place>/**)` (and the vault as `<data folder name>/vault`), for `Bash` and `PowerShell` `Tool(*<place>*)`. An ask rule beats an allow rule in the CLI (probed on CLI 2.1.317: `--allowedTools Read` plus `ask: ["Read(//**/.env)"]` sends the call to the permission prompt; without it the file is read). The CLI then asks Dots' `approve` tool, and §3.3 decides. These rules are broad (a command that mentions `.env` in any way goes to Dots); Dots' own matching is the precise one.

The settings go in a file next to the run's MCP config (`mcp-<run>.settings.json`, removed with it), not inline: `claude_path` may name a `.cmd`, and cmd.exe would read the JSON's quotes and `*` itself.

Probed on CLI 2.1.317 (2026-11-03): `//**/<name>` rules catch Windows absolute paths in both `\` and `/` form. An absolute rule must spell the drive as `//c/Users/...`; `//C:/Users/...` matches nothing. A rule on a folder also covers everything below it.

The CLI checks a search only against the folder it starts in, so a `Grep` over `~` would read `~/.ssh` under a preset's bare `Grep`. So `Grep`, `Glob` and `LS` never go in `--allowedTools`, even when the policy allows them (owner's decision, 2026-11-03): every search reaches `approve`, Dots answers an ordinary one with an allow at once (no prompt), and asks only when the search starts in a folder that holds a private place (§3.3). `Read` stays in the allow list: a single file is caught by the ask rules above.

## 4. The "Always allowed" list

In the dot editor's policy section, above the raw rules, a list **"Always allowed"** shows the dot's allow rules in words, each with **Remove**:

- `Bash(git status:*)` → "Bash: commands that start with git status"; `Bash(npm test)` → "Bash: the command npm test".
- `Read(src/**)` → "Read: everything under src/"; `Read(a.txt)` → "Read: a.txt".
- `WebFetch(domain:example.com)` → "WebFetch: example.com and its subdomains".
- `mcp__github__*` → "every tool of the server github"; a bare name → "<Tool>: every call".
- The rule's own text is shown next to the words, in code type.

Dots' own tools (`Skill`, `Context`, the check-in tools, `Memory`, `Workflow`, the app tools, `Instructions`) are left out: every preset allows them, and they never reach the person's machine. An empty list says "Nothing is always allowed."

Remove takes the rule out of the policy through the editor's existing change path (the policy becomes `custom`, as for any rule edit) and is saved with the form, like every other policy edit. No new table, no new command.

## 5. Harmless tools approved automatically

`PLUMBING_TOOLS` in core: `TodoWrite`, `Task`, `Agent`, `ExitPlanMode`, `EnterPlanMode`, `ToolSearch`, `ListMcpResourcesTool`, `BashOutput`, `KillShell`, `KillBash`.

- `Policy::resolve_in` resolves them to **allow** under every preset and default, unless a rule the person wrote decides otherwise: a deny rule (deny) or an ask rule (ask). The default is not consulted for them.
- `build_args` adds them to the CLI's `--allowedTools`, so the CLI never asks, except one a deny or ask rule matches (read as `resolve_in` reads rules, so `Todo*` counts).
- **Not** on the list: `WebSearch` and `WebFetch` (they send data off the machine), `Workflow` (it starts real work) and `ReadMcpResourceTool` (it can read a `file://` resource past the private-folder guard). They keep their rules.
- `Task`/`Agent` start a sub-agent; its own tool calls are checked one by one as usual, so allowing the start allows nothing else.

## 6. Testing

- Core: the matcher (each place, case, `\`, `~` and env vars, quotes, wildcards, `.env.local`, not `.envrc`, not `process.env.X`, the vault by absolute path, a relative path joined to the run's folder); `allow_candidates` offers nothing for such calls and no covering candidate; `resolve_in` turns allow into ask under Trusted, an allow rule and `default: allow`, keeps deny; plumbing tools allowed under every preset, denied by a deny rule; `decide_approval` refuses a rule for such a call and still approves once; `approve_with_rule` refuses; `approval_touches_private`; `build_args` has the plumbing tools and the `--settings` ask rules.
- App: the command; the contract fixture.
- UI: the card's line (shown for a private call, no "always allow" button); the "Always allowed" list (words, Remove calls the change with the rule gone, Dots' own tools left out, empty text).

## 7. Not in this part

- Letting the person edit the private list.
- Refusing allow rules that name a private place when typed by hand in the rules editor (they do nothing: §3.3.3 asks anyway).
- Guarding MCP tools' arguments (a filesystem MCP server's paths).
