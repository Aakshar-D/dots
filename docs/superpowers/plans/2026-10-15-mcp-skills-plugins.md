# MCP Servers, Skills and Plugins Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dot of either engine can use MCP servers, skills and plugins that the person adds once to a Library, and every MCP tool call passes one check: a tool that declares itself read-only runs without asking, every other tool asks, the dot's own rules still win, and a server whose tool list changed is held until the person accepts the change.

**Architecture:** The Library is a table of items (MCP servers, skills, plugins) with a catalog per server that keeps the tool list the person accepted and its fingerprint; a dot names the items it uses in `DotSpec.uses`, and migration 0006 moves the old per-dot MCP JSON into it. When a run starts, the core opens a "run tools" object: an MCP session per used server through its own client (stdio or Streamable HTTP), the run's skills, and for a Claude run per-run copies of the plugins, stripped of what could get around the check. The local engine offers the sessions' tools and a `skill` tool to the model; the Claude engine reaches the same sessions only through a gateway route of Dots' local server, so that one function, `Policy::resolve_mcp`, decides every call for both engines.

**Tech Stack:** Rust 1.95, sqlx 0.8 on SQLite, tokio, axum and reqwest already in use, Tauri 2.12; Bun 1.3, Vite 8.3, React 19.3, TypeScript 6.0 (strict), TanStack Query 5.104, zod 4.6, Vitest 5.0 with Testing Library and jsdom. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` (commit `5f756c8`, the base of this plan, branch `feat/mcp-skills`). It extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md`; each task edits the main spec's passages (MCP spec section 13) in the commit that changes the behaviour. Where this plan does something else than the MCP spec, the table "Deliberate deviations from the spec" below says what and why; the MCP spec itself is not edited by the tasks.

**Previous plans:** `docs/superpowers/plans/2026-10-08-dots-core.md` (Plan 1), `2026-10-09-dots-local-engine.md` (Plan 2), `2026-10-09-dots-app-3a.md`, `2026-10-12-dots-app-3b.md`, `2026-10-13-dots-app-3c.md` (Plan 3), `2026-10-13-approvals-v2.md` and `2026-10-14-approvals-v2-followups.md` (Approvals v2), `2026-10-14-scheduler-v2.md` (Scheduler v2).

**Where the tasks come from:** the MCP spec, whose trust idea comes from the review of Cloudflare OS (Approvals v2 spec, section 14). Task 1 is the pure check of a call; Tasks 2 to 4 are the data and the parts that stand alone (the Library and migration 0006, the MCP client with its test server `fake-mcp`, skills and plugins on disk); Task 5 joins them in the runtime; Tasks 6 and 7 give them to the local engine and the Claude engine; Task 8 is the app's commands and the contract; Tasks 9 and 10 the UI; Task 11 the smoke test.

**How this plan's code was checked:** the code blocks of Tasks 1 to 10 were generated from a throwaway worktree of commit `5f756c8`, one commit per task, where they compiled and passed on Windows 11. Before the first task the counts were: `cargo test -p dots-core` 426 passed, `cargo test -p dots-app` 49, `bun run --cwd ui test` 295 tests in 21 files. At the last task: `dots-core` 578 passed with 0 failures, `dots-app` 57, the UI 359 tests in 25 files, `cargo clippy --workspace --all-targets` reported no warnings, `cargo fmt --check` and `tsc --noEmit` were clean. Every task's own state was measured the same way, in a separate checkout of that task's commit: the counts in each task's "Run everything" step come from there. Each "verify they fail" step was measured by putting that task's test files onto the commit before it; the decisive lines quoted there are from that measurement. The plan was then replayed: its code blocks were applied by a script to a fresh checkout of `5f756c8`, and after every task the tree was compared with the prototype's commit (409 edits over 10 tasks). Not checked: Task 11 (the smoke test was not run; its agent steps are written from the spec's section 11 and the smokes of Approvals v2 and Scheduler v2), a build on Linux or macOS, and any run of the real `claude` CLI: what the CLI does with the gateway, with `--plugin-dir` and with `mcp__<server>__*` in `--allowedTools` is checked only by the person steps of Task 11.

## Global Constraints

- Toolchain: Rust 1.95 stable; Node 24; Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash); WebView2 runtime. Windows 11, Git Bash.
- Branch `feat/mcp-skills`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\mcp-skills`, based on `5f756c8`. All commands run from the repository root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- Every cargo command runs with `CARGO_TARGET_DIR=C:/Users/aksha/dots/target` (the shared build folder): `export CARGO_TARGET_DIR=C:/Users/aksha/dots/target` once per shell.
- No new dependency (Cargo or npm). One migration, `0006_library.sql` (Task 2; Task 7 adds one line to it). `sqlx` runtime queries only. Timestamps the core writes come from `util::now()`.
- An MCP server's Library name matches `^[a-z0-9_-]{1,40}$`, holds no `__`, and is not `dots`; its tools are `mcp__<name>__<tool>`. A skill's or a plugin's name is 1 to 64 characters of `A-Za-z0-9._-` and starts with a letter or a digit (Task 2).
- `${VAR}` in an MCP server's `env` values, `args`, `cwd`, `url` and `headers` is replaced, when the server starts, from Dots' own environment. A value is never written to the database, a log, a run event, an error text or the CLI's MCP config file (from Task 7). A variable that is not set stops that server from starting.
- Timeouts of the MCP client: 30 s for `initialize` and for `tools/list` with all its pages; 300 s for a local-engine `tools/call`; a cancel of the run cancels a call at once. A Claude run's gateway call gets what is left of `MCP_TOOL_TIMEOUT` (`(approval_wait_secs + 30) × 1000` ms).
- A tool name sent to a local model is at most 64 characters: a longer one is its first 55 characters, `_` and the first 8 hex characters of its SHA-256, and the map back is kept per run. Tool results and `skill` results are capped at 30 KB. A per-run plugin copy is capped at 50 MB per skill and per plugin.
- How the code blocks are written. A step changes files in three ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
  - **Append to `path`:** the block is added at the end of the file.
- A step that says "Replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Check with `git diff --cached --check`, `git ls-files --eol <files>` and `iconv -f UTF-8 -t UTF-8 <file> > /dev/null` before each commit. The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- The UI reaches the backend only through `ui/src/api/commands.ts` and `ui/src/api/events.ts`, plus `open` from `@tauri-apps/plugin-dialog`. No `fetch`, no other plugin, no new capability.
- Live data is parsed, not cast. A type the UI parses changes only together with the contract fixture: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`, then the same test without the variable, then `ui/src/api/types.ts`. The fixture `ui/src/api/__fixtures__/contract.json` is written by that command and never by hand.
- A command the web UI names and the core runs (an MCP server's command line, a plugin's servers, a plugin's hooks) is confirmed in a native dialog from Rust before anything is run or stored (Task 8), the way `claude_path` is. An in-app question uses `@/components/ConfirmButton`.
- Before each commit: the task's own tests, then, as the change requires, `cargo test` of the crates touched, `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt --check`, `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test`, `bun run --cwd ui typecheck`. Test output must be free of warnings. `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first.
- Known on this machine: a `vitest` run started right after an install, a `tsc` run, a cargo build or a large write can fail with `[vitest-pool-runner]: Timeout waiting for worker to respond`, or show 5 s or 30 s timeouts in the tests that type into forms. Run it again; do not change the pool settings. Run the UI tests before the typecheck.
- Known flaky under load on this machine: `local_tools_test::shell_stops_at_its_timeout_and_on_cancel` (a 1 s timing test), `runtime_local_test::live_approval_lets_the_command_run`, `claude_engine_test::a_finished_run_leaves_no_process_behind` (it failed once in the measurement, at Task 4, and passed at every other commit), some `resume_test` tests that make worktrees, and one form test of `DotEditorScreen.test.tsx`. If one fails, run it alone; if it passes, note it and go on.
- Tests never reach the network: a test that needs an MCP server over HTTP runs `fake-mcp --http`, which listens on 127.0.0.1 (Task 3). The tests of a Claude run use `fake-claude`.
- Subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials. An agent never runs the real `claude` CLI; Task 11's steps with the real CLI are the person's.
- A running `dots-app.exe`, `fake-claude.exe` or `fake-mcp.exe` under `target/` blocks cargo builds. An installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) may be running on this machine; it is the user's, it does not block builds, and nobody stops it. A smoke run uses a temporary `DOTS_DATA_DIR`, never the real `%LOCALAPPDATA%\dots`, and stops only the processes it started.
- The Fact-Forcing Gate hook blocks the first Edit or Write of each file and the first Bash command of a session. State the facts it asks for and repeat the identical call. For a large new file, send a one-line stub first.
- Commit messages: Conventional Commits; no AI co-author or attribution trailers.

### Changes a user will notice

- The policy editor's Trusted preset reads "asks before git push and MCP writes": under Trusted an MCP tool that declares itself read-only runs without asking, and any other MCP tool asks (Task 1).
- On upgrade, the MCP servers of each dot's old JSON box move into the Library, ticked in that dot's uses; a secret-looking value becomes a variable name such as `${GITHUB_TOKEN}`, which must then be set in Dots' environment. The dot editor's MCP JSON box is gone (Task 2); the checklists come in Task 10.
- A run says in its transcript what was left out and why: a server that could not start ("salesforce: could not start: SF_TOKEN is not set"), a missing skill folder, a held server, a plugin error from the Claude CLI (Tasks 5 and 7).
- A local-engine dot gets its MCP servers' tools and a `skill` tool, and its system prompt lists its skills (Task 6). A Claude-engine dot reaches its MCP servers through Dots, gets its skills and plugins through `--plugin-dir`, and loads no plugin hooks unless they were switched on (Task 7).
- Adding or changing an MCP server's command line, adding a plugin that starts servers, and switching a plugin's hooks on ask first in a Windows dialog that names what will run (Task 8).
- A new "Library" screen between Dots and Inbox, with tabs for MCP servers, skills and plugins: add, test a connection, check now, review a changed tool list and accept it, remove (asking first and naming the dots), scan a folder for skills, add an installed Claude Code plugin, switch a plugin's hooks (Task 9).
- The dot editor's section "Tools and skills" ticks the Library items a dot uses; the Inbox has a group "Tools changed" with a link to the review, counted in the badge and the tray; the run view names an MCP call "<server> › <tool>" and shows run notices as notes (Tasks 5 and 10).

### Deliberate deviations from the spec

| Spec | This plan | Why |
|---|---|---|
| §4 adds `mcp_default` to `Policy`; §8 lists no policy change in the UI | Task 1 also changes `ui/src/api/types.ts` (the zod `Policy` gains `mcp_default`), the contract fixture, and the Trusted label in `PolicyEditor.tsx`. There is no editor field for `mcp_default`; the editor keeps the stored value. A policy stored before the field reads as `ask`, also a Read-only one, until Task 2's upgrade sets `deny` | `Policy` crosses the contract as a strict object: without the field the UI would refuse every policy |
| §7 "the presets gain `Skill` allow" | The rule also reaches a Claude run's `--allowedTools` (`--allowedTools Read LS Glob Grep Skill` in `claude_cli_test`), since the CLI lists follow the policy's allow rules (Task 1) | The allow rule is a plain tool name; the Claude engine has its own `Skill` tool that reads only the skills Dots passed |
| §2 server name `^[a-z0-9_-]{1,40}$` | Also refuses `__` (Task 2) | `mcp__<server>__<tool>` is split at the first `__` after `mcp__`, as Claude Code does; a server name with `__` would be read wrongly |
| §2, §10 generated names (`plugin_<plugin>_<server>`, `<name>-2`) | `library::mcp_server_name` makes every generated name valid: lower case, other characters as `_`, no `__`, no leading or trailing `_`, `dots` becomes `dots-server`, an empty name `server`, and a name over 40 characters keeps 31 characters, `-` and 8 hex characters of its SHA-256 (Task 2) | A plugin's server name is the plugin's name and the server's, which may be long or hold any character |
| §2 skill and plugin names: no rule | 1 to 64 characters of `A-Za-z0-9._-`, the first a letter or a digit (Task 2) | The name is the folder name of the per-run copy (Task 4) |
| §10 migration 0006 | In two parts: the SQL creates the tables and keeps each dot's old JSON in a table `upgrade_0006`, and `store/upgrade.rs` (`finish_0006`, called by `Store::open`) converts it in one transaction, then drops the table (Task 2) | sqlx runs SQL migrations only, and the conversion (shared items, `-2` names, rule rewrites, secret detection) needs code. The table stays until the commit, so a crash repeats the conversion |
| §10 conversion details | A server without `type` is `http` with a `url`, else `stdio`; `sse` is read as `http`; a server Dots cannot read is left out with a warning and its rules are untouched; "the same config" is compared after the conversion; a secret-looking header other than `Authorization` becomes `${<HEADER>}`; an env key keeps its case in `${<KEY>}`; `mcp_default` is added only when missing; the stored preset string is `read-only` (Task 2) | The Library has only stdio and HTTP; the rest follows the spec's rules where it is silent |
| §2 `${VAR}` "in `env` values, `args`, `cwd`, `url` and `headers`" | Not in `command` (Tasks 2 and 3) | The spec's list; a variable command line would also escape the native confirmation of Task 8 |
| §5 Claude engine: gateway entries | From Task 2 to Task 6 the Claude engine's `--mcp-config` lists the dot's used servers directly, with `${VAR}` expanded into the per-run private config file (deleted after the run), as the old JSON box did; in Tasks 5 and 6 a Claude run therefore starts each stdio server twice. Task 7 replaces the entries with the gateway's | Claude dots keep working between the tasks |
| §5 "other requests from the server are answered with a JSON-RPC error" | `ping` is answered with `{}`; every other request gets -32601 (Task 3) | MCP requires an answer to `ping` |
| §5 HTTP: list changes "in the POST's response" | Also read from the GET event stream after `initialize` (left alone on 405, opened again when the server ends it); `DELETE` with the session on close (Task 3) | Streamable HTTP servers announce their own changes on the GET stream |
| §5 stdio | A bare command is looked up in `PATH` with `PATHEXT` on Windows (`npx` finds `npx.cmd`); the default folder is the run's workspace (`SessionOptions.cwd`); on close stdin is closed and the server gets 2 s before its Job is terminated; stderr goes only to the log, capped at 64 KB per server, and never into an error text (Task 3) | Rust's `Command` only adds `.exe`; Claude Code starts servers in its working folder; MCP's stdio shutdown; a server may print a secret |
| §5 timeouts | The 30 s of `tools/list` covers all its pages; a tool without `inputSchema` gets `{"type": "object"}`; an HTTP status error reads "the server answered HTTP <status>" (Task 3) | One limit as the spec gives it; a model needs a valid function schema |
| §7 the per-run copies remove `allowed-tools` | Also: `allowed-tools` from agent files; the `hooks` frontmatter field of skills, commands and agents unless the plugin runs hooks (a standalone skill never keeps it); `permissionMode` and `mcpServers` of agents always; the copied `plugin.json` loses `mcpServers`, `monitors` and, unless `run_hooks`, `hooks`. `.git` folders and symbolic links are not copied; the 50 MB cap counts only what is copied (Task 4) | Each of these lets a plugin start a server, run a command or approve a call without Dots' check |
| §7 skills appear as `dots-skills:<name>`; "a plugin's skills are named `plugin:skill`" | The copy's manifest `name` is the plugin's Library name, so both engines name a plugin's skill `<library name>:<skill>` (Task 4) | One name for the same skill in both engines and in the Library |
| §2 plugin parts | An inline `mcpServers` object in `plugin.json` is read after `.mcp.json`; `sse` and `streamable-http` are read as `http` (Task 4) | Claude Code reads both places |
| §2 `CLAUDE_CODE_PLUGIN_CACHE_DIR` | Read as the folder that holds `installed_plugins.json` (Task 4). Unverified: the person steps of Task 11 check it | The real CLI cannot be run by the agent |
| §6 "The person changes a server's config: … the next check asks to accept" | A check whose fingerprint equals the accepted one sets `accepted` again, also after a config change reset the catalog to `unchecked`. A server that was never accepted and is then checked becomes `changed`, so that it shows in the Inbox (Task 5) | The trust is in the tool definitions; identical definitions mean identical trust. A change of the command line is confirmed natively (Task 8) |
| §9 `library_add(kind, name, config)` | `LibraryService::add(name, config, accept: Option<&str>)`: the kind is in `LibraryConfig`; with `accept` (the probe's fingerprint) the server is checked and the list accepted only if it has that fingerprint; without it, the server is saved `unchecked` and not started (Tasks 5 and 8) | "Saving accepts that list" must accept the list the person saw, not a newer one |
| §6 run events "salesforce: could not start: …" | A new engine event, `EngineEvent::Notice { text }` (kind `notice`), recorded before the engine's first event (Task 5) | The runner records run events only as engine events; there was no kind for a note |
| §7 "For each run Dots writes plugin folders" | Only for a Claude-engine run; a local run reads its skills from the Library's folders (Task 5) | Only the CLI reads `--plugin-dir` |
| §7 "The folders are deleted when the run ends" | Also: `Runtime::start` deletes every `<data_dir>/plugins/run-*` folder a crash left behind (Task 5) | No run is running at start |
| §6 a list change during a run | The hold is made by `RunTools` for both engines; it never clears during the run; it writes no run event (the catalog changes and `LibraryUpdated` is published) (Task 5) | The watcher has no access to the runner's recorder |
| §5 "a name longer than 64 characters" is shortened | A name with a character a function name cannot hold (outside `A-Za-z0-9_-`) is shortened the same way, the character as `_` (Task 6) | An OpenAI endpoint refuses the whole request for such a name |
| §7 the `skill` tool; the prompt section "after the standing rules" | `skill` is offered only to a run that has skills; the skills section goes into the system message, so a resumed or chat run keeps the list of its first run while the tool reads the current run's skills (Task 6) | The standing rules are in the runner's prompt, which the local engine does not build |
| §6 "a new fingerprint sets `changed` at once, so the next call asks" | The engine waits for `RunServer::settled()` (every announced change listed and checked, at most 30 s) before each MCP check and before each turn's schemas (Task 6) | The watcher lists again asynchronously; without the wait, the call right after the change was allowed (seen in the prototype) |
| Not in the spec | `PermissionGate::check_mcp` has a default that denies; a result without any content part shows its `structuredContent`, else "(no output)"; Dots does not check MCP arguments against the schema (Task 6) | A gate that forgets MCP is safe; the input the person approves is exactly what is sent |
| §4 "the approval row stores the server's accepted fingerprint" | `grants.mcp_fingerprint` too, in migration 0006 (one `ALTER`, Task 7); a database that ran Tasks 2 to 6's version of 0006 fails sqlx's checksum | The one-shot grant of the Claude engine's retry has no link to its approval; 0006 is unreleased |
| §4 the retry after a one-shot grant | The grant is taken first, before the deny rules, as for built-in tools; a grant whose fingerprint differs is used up and the call refused with "<tool>: not performed, because the tool changed since it was approved." (Task 7) | One-shot means one use; a later identical call goes through the check again and asks anew |
| §5 the CLI does not ask `approve` for gateway tools | If the CLI asks `mcp__dots__approve` about a tool of the run's open servers, `approve` answers allow, and the gateway checks the call (Task 7) | One question per call even if the CLI ignores `mcp__<server>__*` in `--allowedTools` |
| §7 `plugin_errors` | Read shape-tolerantly: an object's `plugin`/`name`/`source` and `error`/`message`, a string as the error, anything else as JSON (Task 7). The gateway does not push `tools/list_changed` to the CLI | The CLI's init entries are not documented; the person steps check a real one |
| §9 `library_probe(config)` | Takes an `McpServerConfig` (Task 8) | Only servers are probed |
| §2 "Switching `run_hooks` on asks first"; no question for a program | The question is a native dialog from Rust (`confirm::library_confirmation`), also for a stdio server whose command, args, cwd or env keys change, and for a new plugin, or a new plugin path, that starts stdio servers; the Library screen asks no second time (Tasks 8 and 9) | The web UI names a program the core runs; the same rule as `claude_path` |
| §9 `LibraryUpdated` | Not an OS notification; the UI marks the Library, the Inbox, the dots and the overview stale (Task 8) | Removing an item edits dots' `uses` with no dot event |
| §8 Library screen | A plugin added from a folder is named after the folder (editable); the hooks switch shows only when the plugin has hooks or they are on; "Missing variable" is a badge beside the status; no rename and no skill edit; a plugin's own server has "Check now" but no Edit or Remove (Task 9) | No command reads a manifest name; the core has no rename; an owned server follows its plugin |
| §8 Run view: "with the server and tool name" | "<server> › <tool>", and a plugin's server "<plugin> › <tool>"; only in the run view (approval cards keep `mcp__<server>__<tool>`); the editor's missing variables come from the Library list (Task 10) | The spec asks for the names in the run view |
| §11 the smoke | Task 11 holds the steps; the prototype did not run them | The smoke needs LM Studio, a built app, and the person for the real CLI |

## Review Focus

1. **A writing MCP tool runs without asking.** The six steps must apply in order for both engines: a deny rule first, a held server or an unlisted tool asks even when it is read-only or allowed, the `readOnlyHint` used is the accepted list's and never the server's latest, Trusted asks and Read-only denies a writing tool, and a Claude run reaches its servers only through the gateway. → Task 1 `a_held_server_asks_even_for_an_allowed_or_read_only_tool`, `an_ask_rule_beats_the_read_only_flag`, `trusted_asks_for_a_writing_mcp_tool_and_allows_a_read_only_one`, `read_only_denies_a_writing_mcp_tool_and_allows_a_read_only_one`; Task 6 `a_read_only_tool_runs_and_a_writing_tool_asks_with_the_accepted_fingerprint`, `the_dots_rules_and_mcp_default_come_before_and_after_the_read_only_hint`; Task 7 `a_claude_run_reaches_its_servers_through_the_gateway_only`, `the_gateway_answers_a_running_runs_open_servers_and_checks_each_call`.
2. **A person's servers, rules or secrets go wrong on upgrade.** Migration 0006 must move every readable server of every dot into the Library, share equal configs, rename clashes and the rules that name them, turn secret-looking values into variable names, and set `mcp_default` without overwriting a stored one; a variable's value must never reach a run event, an error or the CLI's config file. → Task 2 `migration_0006_moves_each_dots_servers_into_the_library`, `migration_0006_keeps_a_stored_mcp_default`, `secret_looking_keys`; Task 3 `an_http_session_with_json_answers_keeps_its_session_id` (a refused header is not shown), `an_http_server_that_is_not_there_cannot_start` (the expanded URL is not shown), `errors_say_what_happened_without_the_server_name`; Task 5 `environment_variables_say_which_are_set_and_never_their_values`; Task 7 `mcp_config_lists_dots_and_a_gateway_entry_per_open_server`.
3. **A changed tool list that is not held, or an approval carried out after the tool changed.** A list that differs at run start or changes mid-run must hold the server before the next call; accept must take only the list that was shown; an approval or a grant must not be used once the accepted list moved. Look at the deviation that re-accepts an equal fingerprint after a config change. → Task 5 `a_list_that_changed_since_it_was_accepted_holds_the_server`, `a_list_change_during_the_run_holds_the_server_at_once` (in `run_tools_test.rs`), `adding_with_a_fingerprint_that_moved_leaves_the_list_to_review`, `check_records_a_changed_list_and_accept_guards_the_fingerprint`; Task 6 `a_list_change_during_the_run_holds_the_server_at_once` (in `local_mcp_test.rs`), `an_approved_call_is_skipped_when_the_tool_changed_since_it_was_approved`; Task 7 `an_mcp_grant_allows_only_under_the_fingerprint_it_was_approved_with`, `a_grant_does_not_hold_once_the_servers_accepted_list_changed`; Task 9 "forgets the test when the config changes, so a save accepts nothing it did not show", "says so when the list moved since it was shown".
4. **Server processes left running, or a run that hangs on a server.** Every session must close at the run's end, on a cancel while servers are still opening and when the session is dropped; a dead server must answer "the server stopped" at once; a slow call must time out or be cancelled; plugin folders must not pile up. → Task 3 `close_stops_the_server_and_so_does_dropping_the_session`, `a_server_that_dies_stops_its_session`, `a_slow_call_times_out_or_is_cancelled_and_the_server_is_told`, `a_server_that_exits_or_is_silent_at_initialize_cannot_start`, `a_server_that_writes_much_to_stderr_is_not_blocked`; Task 5 `a_cancelled_run_stops_opening_its_servers`, `a_server_that_cannot_start_is_left_out_with_a_run_event`, `starting_dots_removes_plugin_folders_a_crash_left_behind`, `a_run_gets_its_tools_records_what_was_left_out_and_closes_them_at_the_end`; Task 7 `a_claude_run_reaches_its_servers_through_the_gateway_only` (the server is gone after the run).
5. **A skill or a plugin that gets around the check.** A copy must keep no `.mcp.json`, no inline `mcpServers`, no `monitors/`, no hooks unless switched on, no `allowed-tools`, and no agent `permissionMode`; the `skill` tool must read nothing outside the skill's folder; a plugin that starts servers or runs hooks must be confirmed natively. → Task 4 `a_plugin_copy_has_no_servers_monitors_or_hooks_and_no_allowed_tools`, `skills_go_into_a_generated_dots_skills_plugin_without_allowed_tools`, `frontmatter_hooks_are_removed_from_a_plugin_unless_it_runs_hooks`, `agents_lose_permission_mode_and_mcp_servers_even_with_hooks_on`, `allowed_tools_is_removed_from_a_frontmatter_dots_reads_as_broken`, `a_symbolic_link_is_not_followed` (Unix only); Task 6 `a_file_is_read_only_inside_the_skill_folder`, `a_link_out_of_the_skill_folder_is_refused` (Unix only: on Windows no test follows a link out of a copied or a read folder); Task 8 `a_plugin_that_starts_servers_asks_with_their_command_lines_and_the_hooks_in_one_dialog`, `hooks_ask_only_when_they_go_from_off_to_on`, `a_change_of_env_asks_and_names_the_keys_but_not_the_values`. Not covered by a test: a plugin's `.mcp.json` that changes on disk after the plugin was added is synced at run start without a question (see "After this plan").

## File Map

```
crates/dots-core/
  migrations/0006_library.sql          library_items, mcp_catalogs, approvals.mcp_fingerprint,
                                       dots.uses, upgrade_0006, drop dots.mcp_servers (Task 2);
                                       grants.mcp_fingerprint (Task 7)
  src/policy.rs                        mcp_default, McpTrust, resolve_mcp, Skill allow, candidates (1)
  src/library.rs                       LibraryKind, configs, LibraryItem, Uses, McpCatalog, names (2)
  src/store/library.rs                 Library CRUD, catalogs, dots_using, used_mcp_servers (2);
                                       record_catalog_seen/failure, owned items, changed servers (5)
  src/store/upgrade.rs                 finish_0006: the conversion of the old per-dot servers (2)
  src/store/{mod,dots}.rs              finish_0006 at open; the uses column, check_uses (2)
  src/model.rs                         DotSpec.uses (2); DotRef, ToolsChanged, Inbox.tools_changed (5);
                                       Approval.mcp_fingerprint (6)
  src/mcp/{mod,rpc,stdio,http,tools,vars}.rs  the MCP client, McpTool, fingerprint, ${VAR} (3)
  src/bin/fake-mcp.rs                  the test MCP server, stdio and http (3)
  src/kit/{mod,frontmatter,skill,plugin,installed,run_dirs}.rs  skills and plugins on disk,
                                       installed plugins, per-run plugin folders (4)
  src/library_service.rs               the Library commands, LibraryEntry and the other views (5)
  src/run_tools.rs                     RunTools, RunServer, the list-change watcher, sweep (5);
                                       RunServer::settled (6)
  src/events.rs                        RuntimeEvent::LibraryUpdated (5)
  src/engine/mod.rs                    RunContext.mcp_servers (2, removed in 7); RunContext.tools,
                                       EngineEvent::Notice (5); PermissionGate::check_mcp (6)
  src/runner.rs, src/runtime.rs        used servers (2); RunTools at run start and end,
                                       plugins_dir, Runtime::library, the inbox (5);
                                       register_tools, run_tools, run_cancel (7)
  src/kit/installed.rs                 InstalledPlugin is Serialize (5)
  src/approvals.rs                     ApprovalHub::check_mcp (6); the grant and TOOL_CHANGED (7)
  src/store/approvals.rs               create_mcp_approval (6); create_mcp_grant, take_mcp_grant (7)
  src/store/runs.rs                    resume_parked keeps a grant's fingerprint (7)
  src/engine/local/mcp_tools.rs        model_name, format_result, the MCP side of the loop (6)
  src/engine/local/tools/{mod,skill}.rs  the skill tool and the prompt section (6)
  src/engine/local/mod.rs              schemas per turn, MCP calls, apply_approved guard (6)
  src/server/gateway.rs                POST /mcp/s/{server} (7)
  src/server/{mod,mcp}.rs              the route; shared JSON-RPC helpers, approve passthrough (7)
  src/engine/claude/{args,mod,parse}.rs  interim mcp_config (2); gateway entries, --allowedTools,
                                       --plugin-dir, plugin_errors as notices (7)
  src/bin/fake-claude.rs               records --mcp-config and --plugin-dir; calls a gateway tool (7)
  src/lib.rs                           pub mod library (2), mcp (3), kit (4),
                                       library_service and run_tools (5)
  tests/common/{mod,fake_mcp}.rs       FakeMcp helpers (3), FakeMcp::rewrite (5)
  tests/policy_mcp_test.rs (1), library_test.rs, store_library_test.rs (2), mcp_test.rs,
  mcp_client_test.rs (3), kit_test.rs, kit_run_test.rs (4), library_service_test.rs,
  run_tools_test.rs (5), local_mcp_test.rs, local_skill_test.rs (6), claude_gateway_test.rs (7);
  existing tests follow the changed types                                                (1-7)
apps/dots-app/
  src/contract.rs                      uses (2), notice and tools_changed (5), mcp_fingerprint (6),
                                       the Library samples, LibraryUpdated, enums (8)
  src/events.rs                        Approval literals (6); a Library update does not notify (8)
  src/commands.rs, src/lib.rs          the ten Library commands (8)
  src/confirm.rs                       library_confirmation and the confirmed wrappers (8)
ui/src/
  api/__fixtures__/contract.json       regenerated                                (Tasks 1, 2, 5, 6, 8)
  api/types.ts                         Policy.mcp_default (1), Uses (2), notice, ToolsChanged (5),
                                       Approval.mcp_fingerprint (6), the Library types (8)
  api/commands.ts, lib/query.ts, lib/runtime-events.ts  the Library commands, keys, library_updated (8)
  lib/dot-rules.ts, screens/dot-editor/DotForm.tsx, screens/DotEditorScreen.tsx
                                       no MCP JSON box, uses kept (2); "Tools and skills" (10)
  screens/dot-editor/PolicyEditor.tsx  the Trusted label (1)
  lib/transcript.ts, components/Transcript.tsx  notice (5), note items, MCP call names (10)
  app/Layout.tsx                       the badge counts tools_changed (5); the Library link (9)
  app/routes.tsx, screens/library/*    the Library screen (9)
  lib/mcp-tool.ts                      parseMcpTool, mcpToolLabel (10)
  screens/dot-editor/ToolsAndSkills.tsx  the three checklists (10)
  screens/InboxScreen.tsx              the group "Tools changed" (10)
  test/fixtures.ts, api/*.test.ts, lib/*.test.ts, app/Layout.test.tsx, screens/**/*.test.tsx (1-10)
docs/
  superpowers/specs/2026-10-08-dots-design.md   the passages this plan changes               (1-10)
  verification/2026-10-15-mcp-skills-plugins-smoke.md  the smoke of this plan (new)             (11)
```

---

### Task 1: Check MCP calls through the policy, with an `mcp_default` action

Every MCP tool call of both engines will be decided by one pure function, and this task writes it first, with nothing calling it yet. `Policy::resolve_mcp` applies the six steps of spec section 4 in order: a deny rule, a held server or an unlisted tool (ask), an allow rule, an ask rule, the accepted `readOnlyHint` (allow), and the new `Policy.mcp_default`. The Library's knowledge of the server comes in as `McpTrust`, so the function needs no store. The presets change as spec sections 4 and 7 say: `mcp_default` is `ask`, and `deny` for Read-only; the Trusted preset loses its `mcp__*` ask rule, which as a rule would make read-only tools ask; every preset allows `Skill`. `allow_candidates` offers the rule for the whole server beside the tool's own. `Policy` crosses the contract as a strict object, so the UI type, the fixture and the Trusted label follow in the same commit. The main spec's section 5 follows.

**Files:**
- Create: `crates/dots-core/tests/policy_mcp_test.rs`
- Modify: `crates/dots-core/src/policy.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`, `ui/src/screens/dot-editor/PolicyEditor.tsx`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/claude_cli_test.rs`, `crates/dots-core/tests/local_tools_test.rs`, `crates/dots-core/tests/policy_mcp_test.rs`, `crates/dots-core/tests/policy_test.rs`, `ui/src/screens/dot-editor/PolicyEditor.test.tsx`

**Interfaces:**
- Consumes: `Policy`, `Rule`, `Action`, `Preset`, `Policy::preset`, `allow_candidates` and `cli_lists` (Plan 1 and Approvals v2).
- Produces, in `dots_core::policy`:
  - `Policy.mcp_default: Action`, with `#[serde(default = "default_mcp_action")]`: a stored policy without it reads as `ask`.
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct McpTrust { pub held: bool, pub read_only: bool }`. `held` is true when the server's status is not `accepted` or the tool is not in its accepted list (spec step 2 in one flag); `read_only` is the accepted list's `annotations.readOnlyHint`.
  - `Policy::resolve_mcp(&self, tool: &str, input: &Value, trust: McpTrust) -> Action`, `tool` being `mcp__<server>__<tool>`: a deny rule, `held` (ask), an allow rule, an ask rule, `read_only` (allow), `mcp_default`. Rules match without a workspace. MCP tools never fall to `default`.
  - `Policy::preset(..)`: every preset's rules end with `Skill` allow, so `cli_lists().0` holds `Skill`; `mcp_default` is `ask` for Sandboxed, Custom and Trusted and `deny` for Read-only; Trusted has no `mcp__*` rule.
  - `allow_candidates("mcp__<server>__<tool>")`: the tool itself, then `mcp__<server>__*` labelled "every tool of the server <server>"; the server name ends at the first `__`. A name without a server and a tool offers only itself.
  - UI: `T.Policy` gains `mcp_default: Action`; the Trusted label reads "Trusted: may do anything, asks before git push and MCP writes".

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
            "-p --output-format stream-json --verbose --model haiku --max-turns 12 \
         --permission-prompt-tool mcp__dots__approve --mcp-config C:/data/mcp-RUN1.json \
         --strict-mcp-config --setting-sources  --allowedTools Read LS Glob Grep \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
        ),
```

**with:**

```rust
            "-p --output-format stream-json --verbose --model haiku --max-turns 12 \
         --permission-prompt-tool mcp__dots__approve --mcp-config C:/data/mcp-RUN1.json \
         --strict-mcp-config --setting-sources  --allowedTools Read LS Glob Grep Skill \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
        ),
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        rules: vec![Rule::new("Read(secret/**)", Action::Deny)],
        default: Action::Allow,
    };

```

**with:**

```rust
        rules: vec![Rule::new("Read(secret/**)", Action::Deny)],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };

```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        rules: all,
        default,
    }
}
```

**with:**

```rust
        rules: all,
        default,
        mcp_default: Action::Ask,
    }
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
            rules: vec![Rule::new(&format!("{alias}(secret/**)"), Action::Deny)],
            default: Action::Allow,
        };
        assert_eq!(
```

**with:**

```rust
            rules: vec![Rule::new(&format!("{alias}(secret/**)"), Action::Deny)],
            default: Action::Allow,
            mcp_default: Action::Ask,
        };
        assert_eq!(
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        rules: vec![Rule::new("Read(secret/**)", Action::Deny)],
        default: Action::Allow,
    };
    // The workspace root, named or left out, is walked as before: `secret` is the hidden entry.
```

**with:**

```rust
        rules: vec![Rule::new("Read(secret/**)", Action::Deny)],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };
    // The workspace root, named or left out, is walked as before: `secret` is the hidden entry.
```

**Create `crates/dots-core/tests/policy_mcp_test.rs`:**

```rust
//! The check of an MCP call (MCP spec section 4): `Policy::resolve_mcp`, `mcp_default`, the
//! presets' changes and the rules offered for an MCP tool.

use dots_core::policy::{allow_candidates, Action, McpTrust, Policy, Preset, Rule};
use serde_json::json;

const TRUSTED_SERVER: McpTrust = McpTrust {
    held: false,
    read_only: false,
};
const READ_ONLY_TOOL: McpTrust = McpTrust {
    held: false,
    read_only: true,
};
const HELD_SERVER: McpTrust = McpTrust {
    held: true,
    read_only: false,
};
const HELD_READ_ONLY_TOOL: McpTrust = McpTrust {
    held: true,
    read_only: true,
};

fn policy(rules: Vec<Rule>, mcp_default: Action) -> Policy {
    Policy {
        preset: Preset::Custom,
        rules,
        default: Action::Allow,
        mcp_default,
    }
}

fn resolve(p: &Policy, tool: &str, trust: McpTrust) -> Action {
    p.resolve_mcp(tool, &json!({}), trust)
}

const TOOL: &str = "mcp__sf__query";

#[test]
fn a_deny_rule_wins_over_everything() {
    let p = policy(
        vec![
            Rule::new("mcp__sf__*", Action::Deny),
            Rule::new(TOOL, Action::Allow),
        ],
        Action::Allow,
    );
    for trust in [TRUSTED_SERVER, READ_ONLY_TOOL, HELD_SERVER] {
        assert_eq!(resolve(&p, TOOL, trust), Action::Deny);
    }
}

#[test]
fn a_held_server_asks_even_for_an_allowed_or_read_only_tool() {
    let p = policy(vec![Rule::new(TOOL, Action::Allow)], Action::Allow);
    assert_eq!(resolve(&p, TOOL, HELD_SERVER), Action::Ask);
    assert_eq!(resolve(&p, TOOL, HELD_READ_ONLY_TOOL), Action::Ask);
    // Held beats the default too, and a deny default does not turn the ask into a deny.
    let denying = policy(vec![], Action::Deny);
    assert_eq!(resolve(&denying, TOOL, HELD_SERVER), Action::Ask);
}

#[test]
fn an_allow_rule_beats_an_ask_rule_and_the_default() {
    let p = policy(
        vec![
            Rule::new("mcp__sf__*", Action::Ask),
            Rule::new(TOOL, Action::Allow),
        ],
        Action::Deny,
    );
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Allow);
    assert_eq!(resolve(&p, "mcp__sf__update", TRUSTED_SERVER), Action::Ask);
}

#[test]
fn an_ask_rule_beats_the_read_only_flag() {
    let p = policy(vec![Rule::new("mcp__sf__*", Action::Ask)], Action::Allow);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Ask);
}

#[test]
fn a_read_only_tool_is_allowed_whatever_the_default() {
    for default in [Action::Allow, Action::Ask, Action::Deny] {
        let p = policy(vec![], default);
        assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
    }
}

#[test]
fn any_other_tool_gets_mcp_default_not_default() {
    for mcp_default in [Action::Allow, Action::Ask, Action::Deny] {
        let p = policy(vec![], mcp_default);
        assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), mcp_default);
        // The plain check still uses `default`.
        assert_eq!(p.resolve(TOOL, &json!({})), Action::Allow);
    }
}

#[test]
fn the_presets_set_mcp_default() {
    for (preset, expected) in [
        (Preset::Sandboxed, Action::Ask),
        (Preset::Custom, Action::Ask),
        (Preset::Trusted, Action::Ask),
        (Preset::ReadOnly, Action::Deny),
    ] {
        assert_eq!(Policy::preset(preset).mcp_default, expected, "{preset:?}");
    }
}

#[test]
fn trusted_asks_for_a_writing_mcp_tool_and_allows_a_read_only_one() {
    let p = Policy::preset(Preset::Trusted);
    assert!(!p.rules.iter().any(|r| r.tool == "mcp__*"));
    assert_eq!(p.default, Action::Allow);
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Ask);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
    assert_eq!(resolve(&p, TOOL, HELD_SERVER), Action::Ask);
}

#[test]
fn read_only_denies_a_writing_mcp_tool_and_allows_a_read_only_one() {
    let p = Policy::preset(Preset::ReadOnly);
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Deny);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
}

#[test]
fn sandboxed_asks_for_a_writing_mcp_tool() {
    let p = Policy::preset(Preset::Sandboxed);
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Ask);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
}

#[test]
fn every_preset_allows_the_skill_tool() {
    for preset in [
        Preset::ReadOnly,
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::Custom,
    ] {
        let p = Policy::preset(preset);
        assert!(p.cli_lists().0.contains(&"Skill".to_string()), "{preset:?}");
        assert_eq!(p.resolve("Skill", &json!({"name": "pdf"})), Action::Allow);
    }
}

#[test]
fn a_stored_policy_without_mcp_default_reads_as_ask() {
    let p: Policy = serde_json::from_value(json!({
        "preset": "custom",
        "rules": [{"tool": "Read", "action": "allow"}],
        "default": "allow",
    }))
    .unwrap();
    assert_eq!(p.mcp_default, Action::Ask);
    let back = serde_json::to_value(&p).unwrap();
    assert_eq!(back["mcp_default"], "ask");
}

#[test]
fn an_mcp_tool_offers_the_tool_and_its_server() {
    let found = allow_candidates("mcp__github__create_issue", &json!({"title": "x"}), None);
    let patterns: Vec<&str> = found.iter().map(|c| c.pattern.as_str()).collect();
    assert_eq!(
        patterns,
        vec!["mcp__github__create_issue", "mcp__github__*"]
    );
    assert_eq!(found[0].label, "every call of this tool");
    assert_eq!(found[1].label, "every tool of the server github");
    // Each one is a valid rule that allows the call.
    for c in &found {
        let p = policy(vec![Rule::new(&c.pattern, Action::Allow)], Action::Ask);
        p.validate().unwrap();
        assert_eq!(
            p.resolve_mcp("mcp__github__create_issue", &json!({}), TRUSTED_SERVER),
            Action::Allow
        );
    }
}

#[test]
fn a_name_without_a_server_and_a_tool_offers_only_itself() {
    for tool in ["mcp__github", "mcp__github__", "mcp____x"] {
        let found = allow_candidates(tool, &json!({}), None);
        let patterns: Vec<&str> = found.iter().map(|c| c.pattern.as_str()).collect();
        assert_eq!(patterns, vec![tool]);
    }
    assert!(allow_candidates("mcp__*", &json!({}), None).is_empty());
}
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
use dots_core::policy::{allow_candidates, Action, Policy, Preset, Rule, RuleCandidate};
use serde_json::json;

```

**with:**

```rust
use dots_core::policy::{allow_candidates, Action, McpTrust, Policy, Preset, Rule, RuleCandidate};
use serde_json::json;

```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust

#[test]
fn trusted_allows_by_default_but_asks_for_push_and_mcp() {
    let p = Policy::preset(Preset::Trusted);
    assert_eq!(p.resolve("Bash", &bash("cargo test")), Action::Allow);
```

**with:**

```rust

#[test]
fn trusted_allows_by_default_but_asks_for_push_and_for_a_writing_mcp_tool() {
    let p = Policy::preset(Preset::Trusted);
    assert_eq!(p.resolve("Bash", &bash("cargo test")), Action::Allow);
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
        Action::Ask
    );
    assert_eq!(
        p.resolve("mcp__salesforce__update", &json!({})),
        Action::Ask
    );
```

**with:**

```rust
        Action::Ask
    );
    let writes = McpTrust {
        held: false,
        read_only: false,
    };
    assert_eq!(
        p.resolve_mcp("mcp__salesforce__update", &json!({}), writes),
        Action::Ask
    );
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
        ],
        default: Action::Allow,
    };
    assert_eq!(p.resolve("Bash", &bash("rm -i x")), Action::Deny);
```

**with:**

```rust
        ],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };
    assert_eq!(p.resolve("Bash", &bash("rm -i x")), Action::Deny);
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
        rules: vec![Rule::new("Bash(rm:*)", Action::Deny)],
        default: Action::Allow,
    };
    assert_eq!(p.resolve("Bash", &bash("ls; rm -rf x")), Action::Deny);
```

**with:**

```rust
        rules: vec![Rule::new("Bash(rm:*)", Action::Deny)],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };
    assert_eq!(p.resolve("Bash", &bash("ls; rm -rf x")), Action::Deny);
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
        ],
        default: Action::Allow,
    };
    assert_eq!(p.resolve("mcp__sf__query", &json!({})), Action::Deny);
```

**with:**

```rust
        ],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };
    assert_eq!(p.resolve("mcp__sf__query", &json!({})), Action::Deny);
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
            rules: vec![Rule::new(bad, Action::Allow)],
            default: Action::Ask,
        };
        assert!(p.validate().is_err(), "{bad:?} should be invalid");
```

**with:**

```rust
            rules: vec![Rule::new(bad, Action::Allow)],
            default: Action::Ask,
            mcp_default: Action::Ask,
        };
        assert!(p.validate().is_err(), "{bad:?} should be invalid");
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
        rules,
        default,
    }
}
```

**with:**

```rust
        rules,
        default,
        mcp_default: Action::Ask,
    }
}
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
    assert_eq!(
        allow_candidates("mcp__github__create_issue", &json!({"title": "x"}), ws()),
        vec![RuleCandidate {
            pattern: "mcp__github__create_issue".into(),
            label: "every call of this tool".into(),
        }]
    );
    assert_eq!(
```

**with:**

```rust
    assert_eq!(
        allow_candidates("mcp__github__create_issue", &json!({"title": "x"}), ws()),
        vec![
            RuleCandidate {
                pattern: "mcp__github__create_issue".into(),
                label: "every call of this tool".into(),
            },
            RuleCandidate {
                pattern: "mcp__github__*".into(),
                label: "every tool of the server github".into(),
            },
        ]
    );
    assert_eq!(
```

**In `crates/dots-core/tests/policy_test.rs`, replace:**

```rust
        }
    }
    assert_eq!(seen, 17);
}

```

**with:**

```rust
        }
    }
    assert_eq!(seen, 18);
}

```

**In `ui/src/screens/dot-editor/PolicyEditor.test.tsx`, replace:**

```tsx
  ],
  default: "ask",
};
const readOnly: Policy = {
```

**with:**

```tsx
  ],
  default: "ask",
  mcp_default: "ask",
};
const readOnly: Policy = {
```

**In `ui/src/screens/dot-editor/PolicyEditor.test.tsx`, replace:**

```tsx
  rules: [{ tool: "Read", action: "allow" }],
  default: "deny",
};

```

**with:**

```tsx
  rules: [{ tool: "Read", action: "allow" }],
  default: "deny",
  mcp_default: "deny",
};

```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test claude_cli_test --test local_tools_test --test policy_mcp_test --test policy_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/dot-editor/PolicyEditor.test.tsx 2>&1 | grep -E "Tests |Test Files|× |FAIL ")
```

Expected: three test targets do not compile. The decisive lines: 10 × `` error[E0560]: struct `Policy` has no field named `mcp_default` ``, 2 × `` error[E0609]: no field `mcp_default` on type `Policy` ``, `` no method named `resolve_mcp` found for struct `Policy` ``, 2 × `` error[E0432]: unresolved import `dots_core::policy::McpTrust` ``, then `could not compile` for `policy_test` (7 errors), `policy_mcp_test` (6) and `local_tools_test` (4). `claude_cli_test` compiles, but cargo runs no test while other targets fail to build. The UI: `Test Files  1 failed (1)`, `Tests  1 failed | 2 passed (3)`; the failing test is "replaces the rules with a chosen preset's" (its preset sample has `mcp_default`, which the strict schema refuses).

- [ ] **Step 3: Implement**

The core first, then the UI files. `types.ts` is checked against the contract fixture, which the next step regenerates.

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
    pub rules: Vec<Rule>,
    pub default: Action,
}

```

**with:**

```rust
    pub rules: Vec<Rule>,
    pub default: Action,
    /// What an MCP tool gets when no rule and no read-only flag decides (`resolve_mcp`, step 6).
    /// A policy stored before it existed reads as `ask`.
    #[serde(default = "default_mcp_action")]
    pub mcp_default: Action,
}

fn default_mcp_action() -> Action {
    Action::Ask
}

/// What the Library knows about the server and the tool of an MCP call, for `resolve_mcp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpTrust {
    /// The server is held (its status is not `accepted`) or the tool is not in its accepted list.
    pub held: bool,
    /// The accepted list gives the tool `annotations.readOnlyHint: true`.
    pub read_only: bool,
}

```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
    pub fn preset(preset: Preset) -> Policy {
        let mut rules: Vec<Rule> = Vec::new();
        let default = match preset {
            Preset::ReadOnly => {
```

**with:**

```rust
    pub fn preset(preset: Preset) -> Policy {
        let mut rules: Vec<Rule> = Vec::new();
        let mut mcp_default = Action::Ask;
        let default = match preset {
            Preset::ReadOnly => {
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
                    rules.push(Rule::new(t, Action::Deny));
                }
                Action::Deny
            }
```

**with:**

```rust
                    rules.push(Rule::new(t, Action::Deny));
                }
                mcp_default = Action::Deny;
                Action::Deny
            }
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
                rules.push(Rule::new("Bash(git push:*)", Action::Ask));
                rules.push(Rule::new("PowerShell(git push:*)", Action::Ask));
                rules.push(Rule::new("mcp__*", Action::Ask));
                Action::Allow
            }
        };
        Policy {
            preset,
            rules,
            default,
        }
    }
```

**with:**

```rust
                rules.push(Rule::new("Bash(git push:*)", Action::Ask));
                rules.push(Rule::new("PowerShell(git push:*)", Action::Ask));
                Action::Allow
            }
        };
        // Every preset lets the model read the skills the dot uses.
        rules.push(Rule::new("Skill", Action::Allow));
        Policy {
            preset,
            rules,
            default,
            mcp_default,
        }
    }
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        }
        self.default
    }

```

**with:**

```rust
        }
        self.default
    }

    /// The check of an MCP tool call (`tool` is `mcp__<server>__<tool>`). The first match wins:
    /// a deny rule; a held server or an unlisted tool (ask); an allow rule; an ask rule; the
    /// accepted `readOnlyHint` (allow); `mcp_default`. MCP tools never fall to `default`.
    pub fn resolve_mcp(&self, tool: &str, input: &Value, trust: McpTrust) -> Action {
        if self.rule_matches(Action::Deny, tool, input, None) {
            return Action::Deny;
        }
        if trust.held {
            return Action::Ask;
        }
        if self.rule_matches(Action::Allow, tool, input, None) {
            return Action::Allow;
        }
        if self.rule_matches(Action::Ask, tool, input, None) {
            return Action::Ask;
        }
        if trust.read_only {
            return Action::Allow;
        }
        self.mcp_default
    }

```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
///   not at all for a UNC, device or drive-relative path.
/// - `WebFetch`: the host.
/// - Any other tool: its name.
///
```

**with:**

```rust
///   not at all for a UNC, device or drive-relative path.
/// - `WebFetch`: the host.
/// - An MCP tool (`mcp__<server>__<tool>`): its name, and `mcp__<server>__*` for the server.
/// - Any other tool: its name.
///
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
        // A name that ends in `*` would be read as a prefix of other tools' names.
        found.push((tool.to_string(), "every call of this tool".into()));
    }
    let mut out: Vec<RuleCandidate> = Vec::new();
```

**with:**

```rust
        // A name that ends in `*` would be read as a prefix of other tools' names.
        found.push((tool.to_string(), "every call of this tool".into()));
        // An MCP tool also offers its whole server. The server name ends at the first `__`.
        let server = tool
            .strip_prefix("mcp__")
            .and_then(|rest| rest.split_once("__"))
            .filter(|(server, name)| !server.is_empty() && !name.is_empty());
        if let Some((server, _)) = server {
            found.push((
                format!("mcp__{server}__*"),
                format!("every tool of the server {server}"),
            ));
        }
    }
    let mut out: Vec<RuleCandidate> = Vec::new();
```

**In `crates/dots-core/src/policy.rs`, replace:**

```rust
            rules: vec![Rule::new(&pattern, Action::Allow)],
            default: Action::Ask,
        };
        if rule.validate().is_ok()
```

**with:**

```rust
            rules: vec![Rule::new(&pattern, Action::Allow)],
            default: Action::Ask,
            mcp_default: Action::Ask,
        };
        if rule.validate().is_ok()
```

**In `ui/src/api/types.ts`, replace:**

```ts
  rules: z.array(Rule),
  default: Action,
});

```

**with:**

```ts
  rules: z.array(Rule),
  default: Action,
  mcp_default: Action,
});

```

**In `ui/src/screens/dot-editor/PolicyEditor.tsx`, replace:**

```tsx
  "read-only": "Read-only: may read, never writes or runs commands",
  sandboxed: "Sandboxed: may edit inside its folder, asks for the rest",
  trusted: "Trusted: may do anything, asks before git push and MCP tools",
  custom: "Custom: the rules below",
};
```

**with:**

```tsx
  "read-only": "Read-only: may read, never writes or runs commands",
  sandboxed: "Sandboxed: may edit inside its folder, asks for the rest",
  trusted: "Trusted: may do anything, asks before git push and MCP writes",
  custom: "Custom: the rules below",
};
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

````markdown
    { "tool": "WebFetch",           "action": "deny" }
  ],
  "default": "ask"
}
```

- Tool patterns use Claude Code permission-rule syntax so they pass through unchanged to
  `--allowedTools` / `--disallowedTools`. The local engine matches the same syntax against its
````

**with:**

````markdown
    { "tool": "WebFetch",           "action": "deny" }
  ],
  "default": "ask",
  "mcp_default": "ask"
}
```

- `mcp_default` is the action for an MCP tool that no rule and no read-only flag decides (see
  `2026-10-15-mcp-skills-plugins-design.md` section 4). A policy without it reads as `ask`.
- Tool patterns use Claude Code permission-rule syntax so they pass through unchanged to
  `--allowedTools` / `--disallowedTools`. The local engine matches the same syntax against its
````

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
  `policy::allow_candidates` derives from that very call is accepted, so the caller cannot
  widen the policy at will; the policy's preset becomes `custom`. The pending approvals of one
  run can also be decided together (`decide_run_approvals`): in the order they were asked,
  with one resume, and without a rule. See `2026-10-13-approvals-v2-design.md` §4 and §5.
- Presets:
  - `read-only`: allow Read/LS/Glob/Grep; deny Write/Edit/Bash; default deny.
  - `sandboxed` (default): allow Read/LS/Glob/Grep, `Write(./**)`, `Edit(./**)`,
```

**with:**

```markdown
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
  `policy::allow_candidates` derives from that very call is accepted, so the caller cannot
  widen the policy at will; the policy's preset becomes `custom`. For an MCP tool the candidates
  are `mcp__<server>__<tool>` and `mcp__<server>__*`. The pending approvals of one
  run can also be decided together (`decide_run_approvals`): in the order they were asked,
  with one resume, and without a rule. See `2026-10-13-approvals-v2-design.md` §4 and §5.
- Presets:
  - Every preset allows `Skill` (the tool that reads a skill) and sets `mcp_default`: `ask` for
    all but `read-only`, which sets `deny`.
  - `read-only`: allow Read/LS/Glob/Grep; deny Write/Edit/Bash; default deny.
  - `sandboxed` (default): allow Read/LS/Glob/Grep, `Write(./**)`, `Edit(./**)`,
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    A rule that names a directory alone (`Read(**/secret)`) does not cover what is below it;
    `Read(secret/**)` does.
  - `trusted`: allow all; ask for `Bash(git push:*)` and `mcp__*`; default allow.

### Claude engine
```

**with:**

```markdown
    A rule that names a directory alone (`Read(**/secret)`) does not cover what is below it;
    `Read(secret/**)` does.
  - `trusted`: allow all; ask for `Bash(git push:*)`; default allow. An MCP tool that is not
    read-only still asks, through `mcp_default`; there is no `mcp__*` rule, which would make
    read-only tools ask too.

### Claude engine
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: the first run fails with "contract.json is stale"; the second and third pass. Every policy in the fixture (`Dot`, `DotOverview`, `DotSpec`, `NewDotSpec`) gains `"mcp_default": "ask"`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test claude_cli_test --test local_tools_test --test policy_mcp_test --test policy_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/dot-editor/PolicyEditor.test.tsx 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `claude_cli_test` 10 passed, `local_tools_test` 38, `policy_mcp_test` 14, `policy_test` 51 (on Windows; one test of each of the first two is for other systems); `dots-app` 49 passed; the UI file passes.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 440 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 295 passed in 21 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/src/policy.rs crates/dots-core/tests/claude_cli_test.rs crates/dots-core/tests/local_tools_test.rs crates/dots-core/tests/policy_mcp_test.rs crates/dots-core/tests/policy_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/screens/dot-editor/PolicyEditor.test.tsx ui/src/screens/dot-editor/PolicyEditor.tsx
git commit -m "feat(core): check MCP calls through the policy with an mcp_default action"
```

### Task 2: Keep MCP servers, skills and plugins in a Library that dots use

This task holds the whole schema of spec sections 2, 3 and 10 and the types that read it. Migration 0006 creates `library_items` and `mcp_catalogs`, adds `approvals.mcp_fingerprint` (used from Task 6) and `dots.uses`, and drops `dots.mcp_servers`; because the conversion of each dot's old JSON needs code, the SQL keeps that JSON in a table `upgrade_0006`, and `Store::open` converts it once, in one transaction, then drops the table: servers become Library items with an unchecked catalog, equal configs share one item, a clash gets a `-2` name and the dot's rules follow it, secret-looking values become `${VAR}`, and stored policies get `mcp_default`. The store gets the Library's reads and writes and refuses a dot that uses an item the Library does not have. `DotSpec.uses` replaces `mcp_servers` end to end. So that Claude dots keep working until the gateway (Task 7), the Claude engine's `--mcp-config` lists the used servers directly, expanded from the environment, as the old JSON box did. The editor's MCP JSON box goes; a dot's `uses` is kept on save, and the checklists come in Task 10. The main spec's sections 3, 4 and 7 follow.

**Files:**
- Create: `crates/dots-core/migrations/0006_library.sql`, `crates/dots-core/src/library.rs`, `crates/dots-core/src/store/library.rs`, `crates/dots-core/src/store/upgrade.rs`, `crates/dots-core/tests/library_test.rs`, `crates/dots-core/tests/store_library_test.rs`
- Modify: `apps/dots-app/src/contract.rs`, `crates/dots-core/src/engine/claude/args.rs`, `crates/dots-core/src/engine/claude/mod.rs`, `crates/dots-core/src/engine/mod.rs`, `crates/dots-core/src/lib.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runner.rs`, `crates/dots-core/src/store/dots.rs`, `crates/dots-core/src/store/mod.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`, `ui/src/lib/dot-rules.ts`, `ui/src/screens/DotEditorScreen.tsx`, `ui/src/screens/dot-editor/DotForm.tsx`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/claude_cli_test.rs`, `crates/dots-core/tests/engine_prompt_test.rs`, `crates/dots-core/tests/library_test.rs`, `crates/dots-core/tests/local_engine_test.rs`, `crates/dots-core/tests/runner_test.rs`, `crates/dots-core/tests/store_dots_test.rs`, `crates/dots-core/tests/store_library_test.rs`, `ui/src/lib/dot-rules.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`

**Interfaces:**
- Consumes: Task 1's `mcp_default`; `str_enum!`, `util::now()`, `Error::Invalid`, `Conflict` and `NotFound`; the store's `migrate!` and its dot columns.
- Produces:
  - Migration `0006_library.sql`: `library_items (id, kind, name, config, owner_id REFERENCES library_items(id) ON DELETE CASCADE, created_at, updated_at, UNIQUE (kind, name))` with an index on `owner_id`; `mcp_catalogs (server_id PRIMARY KEY REFERENCES library_items(id) ON DELETE CASCADE, accepted_fingerprint, accepted_tools, seen_fingerprint, seen_tools, status NOT NULL, checked_at, error)`; `approvals.mcp_fingerprint TEXT`; `dots.uses TEXT NOT NULL DEFAULT '{"mcp_servers":[],"skills":[],"plugins":[]}'`; `upgrade_0006 (dot_id, mcp_servers)` filled from `dots`; `dots.mcp_servers` dropped.
  - `store::upgrade::finish_0006(pool: &SqlitePool) -> Result<()>` (`pub(super)`), called by `Store::open` after the migrations: when `upgrade_0006` exists, it converts every dot in one transaction and drops the table.
  - In `dots_core::library`: `LibraryKind` (`str_enum!`: `mcp_server`, `skill`, `plugin`; `label()`, `uses_key()`); `CatalogStatus` (`accepted`, `changed`, `unreachable`, `unchecked`); `MAX_SERVER_NAME = 40`, `MAX_ITEM_NAME = 64`; `McpServerConfig` (tag `transport`): `Stdio { command: String, args: Vec<String>, cwd: Option<String>, env: BTreeMap<String, String> }` and `Http { url: String, headers: BTreeMap<String, String> }`; `SkillConfig { path: String }`; `PluginConfig { path: String, installed_id: Option<String>, run_hooks: bool }`; `LibraryConfig` (tag `kind`, content `config`): `McpServer`, `Skill`, `Plugin`, with `kind()` and `validate()`; `LibraryItem { id, name, #[serde(flatten)] config: LibraryConfig, owner_id: Option<String>, created_at, updated_at }` with `kind()`; `Uses { mcp_servers, skills, plugins: Vec<String> }` (`#[serde(default)]`; `is_empty()`, `names(kind) -> &[String]`, `validate()`); `McpCatalog { server_id, status, accepted_fingerprint, accepted_tools: Option<Value>, seen_fingerprint, seen_tools: Option<Value>, checked_at, error }` with `McpCatalog::unchecked(server_id)`.
  - `check_name(kind: LibraryKind, name: &str) -> Result<()>`, `looks_secret(key: &str) -> bool`, `mcp_server_name(raw: &str) -> String` (always a valid server name), `plugin_server_name(plugin: &str, server: &str) -> String`.
  - `McpServerConfig::variables(&self) -> Vec<String>` (sorted, unique) and `McpServerConfig::expanded(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<McpServerConfig>` (`Invalid("SF_TOKEN is not set")` for a missing one). Task 3 moves both to `mcp/vars.rs` unchanged.
  - `Store`: `add_library_item(&self, name: &str, config: &LibraryConfig, owner_id: Option<&str>) -> Result<LibraryItem>` (a new server gets an `unchecked` catalog in the same transaction; `owner_id` must be a plugin and the item a server; a taken name is `Conflict`); `get_library_item(&self, id: &str) -> Result<LibraryItem>`; `find_library_item(&self, kind: LibraryKind, name: &str) -> Result<Option<LibraryItem>>`; `list_library_items(&self) -> Result<Vec<LibraryItem>>`; `update_library_item(&self, id: &str, config: &LibraryConfig) -> Result<LibraryItem>` (keeps kind and name; a changed server config sets the catalog to `unchecked` and keeps `accepted_*`); `remove_library_item(&self, id: &str) -> Result<()>` (takes the name out of every dot's `uses`; the cascade removes a plugin's servers and the catalogs); `dots_using(&self, item: &LibraryItem) -> Result<Vec<Dot>>`; `used_mcp_servers(&self, uses: &Uses) -> Result<Vec<LibraryItem>>` (the ticked servers and those of the ticked plugins); `get_catalog`, `list_catalogs`, `put_catalog`; `accept_catalog(&self, server_id: &str, fingerprint: &str) -> Result<McpCatalog>` (`Conflict` when the seen fingerprint moved).
  - `DotSpec.uses: Uses` replaces `mcp_servers`; `create_dot` and `update_dot` write the dot and then check `uses` in one transaction (`Invalid`, "the Library has no MCP server named …").
  - Interim until Task 7: `RunContext.mcp_servers: Vec<LibraryItem>`, filled by the runner from `used_mcp_servers`; `engine::claude::args::mcp_config(ctx: &RunContext, env: &dyn Fn(&str) -> Option<String>) -> Value` lists them, expanded, beside `dots` (a server with a missing variable is left out with a warning).
  - Contract: the `DotSpec` sample's `uses` is `{"mcp_servers": ["github"], "skills": ["pdf"], "plugins": ["pdf-tools"]}`. UI: `T.Uses`, `DotSpec.uses`; in `@/lib/dot-rules`, `parseMcp` and `mcpOf` are gone, `specErrors(spec)` takes one argument and `specOf` copies `uses`; `DotForm`'s `Draft` is `{ spec: DotSpec }`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::{EngineEvent, RunContext};
use dots_core::model::WorkspaceMode;
use dots_core::policy::{Policy, Preset};
use serde_json::json;
use tokio_util::sync::CancellationToken;

```

**with:**

```rust
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::{EngineEvent, RunContext};
use dots_core::library::{LibraryConfig, LibraryItem};
use dots_core::model::WorkspaceMode;
use dots_core::policy::{Policy, Preset};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
    dot.spec.max_turns = 12;
    dot.spec.policy = Policy::preset(Preset::ReadOnly);
    dot.spec.mcp_servers = Some(json!({"sf": {"type": "stdio", "command": "sf-mcp"}}));
    RunContext {
        run_id: "RUN1".into(),
```

**with:**

```rust
    dot.spec.max_turns = 12;
    dot.spec.policy = Policy::preset(Preset::ReadOnly);
    dot.spec.uses.mcp_servers = vec!["sf".into(), "gh".into()];
    let item = |name: &str, config: Value| LibraryItem {
        id: format!("ID-{name}"),
        name: name.into(),
        config: LibraryConfig::McpServer(serde_json::from_value(config).unwrap()),
        owner_id: None,
        created_at: "t".into(),
        updated_at: "t".into(),
    };
    RunContext {
        run_id: "RUN1".into(),
```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
        cancel: CancellationToken::new(),
        approved_calls: Vec::new(),
    }
}
```

**with:**

```rust
        cancel: CancellationToken::new(),
        approved_calls: Vec::new(),
        mcp_servers: vec![
            item(
                "sf",
                json!({"transport": "stdio", "command": "sf-mcp", "args": ["--org", "${SF_ORG}"],
                       "env": {"SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly"}}),
            ),
            item(
                "gh",
                json!({"transport": "http", "url": "https://gh.example.com/mcp",
                       "headers": {"Authorization": "Bearer ${GH_TOKEN}"}}),
            ),
            item(
                "lost",
                json!({"transport": "stdio", "command": "lost", "env": {"K": "${NOT_SET}"}}),
            ),
        ],
    }
}

fn test_env(name: &str) -> Option<String> {
    match name {
        "SF_ORG" => Some("acme".into()),
        "SF_TOKEN" => Some("sf-secret".into()),
        "GH_TOKEN" => Some("gh-secret".into()),
        _ => None,
    }
}
```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust

#[test]
fn mcp_config_merges_dot_servers_and_adds_dots() {
    let cfg = mcp_config(&ctx(None));
    assert_eq!(cfg["mcpServers"]["sf"]["command"], "sf-mcp");
    assert_eq!(cfg["mcpServers"]["dots"]["type"], "http");
    assert_eq!(
```

**with:**

```rust

#[test]
fn mcp_config_lists_the_library_servers_with_their_variables_and_adds_dots() {
    let cfg = mcp_config(&ctx(None), &test_env);
    assert_eq!(
        cfg["mcpServers"]["sf"],
        json!({"type": "stdio", "command": "sf-mcp", "args": ["--org", "acme"],
               "env": {"MODE": "readonly", "SF_TOKEN": "sf-secret"}})
    );
    assert_eq!(
        cfg["mcpServers"]["gh"],
        json!({"type": "http", "url": "https://gh.example.com/mcp",
               "headers": {"Authorization": "Bearer gh-secret"}})
    );
    // A server whose variable is not set is left out.
    assert!(cfg["mcpServers"].get("lost").is_none(), "{cfg}");
    assert_eq!(cfg["mcpServers"].as_object().unwrap().len(), 3);
    assert_eq!(cfg["mcpServers"]["dots"]["type"], "http");
    assert_eq!(
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
        cancel,
        approved_calls: Vec::new(),
    }
}
```

**with:**

```rust
        cancel,
        approved_calls: Vec::new(),
        mcp_servers: Vec::new(),
    }
}
```

**Create `crates/dots-core/tests/library_test.rs`:**

```rust
//! The Library's model: item names, configs, `${VAR}` references and a dot's uses (MCP/skills
//! spec sections 2 and 10).

use std::collections::BTreeMap;

use dots_core::library::{
    check_name, looks_secret, mcp_server_name, plugin_server_name, CatalogStatus, LibraryConfig,
    LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig, SkillConfig, Uses,
};
use dots_core::Error;
use serde_json::json;

fn invalid(r: dots_core::Result<()>) -> bool {
    matches!(r, Err(Error::Invalid(_)))
}

fn stdio(command: &str) -> McpServerConfig {
    McpServerConfig::Stdio {
        command: command.into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
    }
}

#[test]
fn an_mcp_server_name_is_its_tool_prefix() {
    for good in ["sf", "my-server_2", "a", &"x".repeat(40)] {
        assert!(check_name(LibraryKind::McpServer, good).is_ok(), "{good}");
    }
    // `__` would make `mcp__<server>__<tool>` ambiguous; `dots` is Dots' own server.
    for bad in ["", "Sf", "a.b", "a b", "a__b", "dots", &"x".repeat(41)] {
        assert!(invalid(check_name(LibraryKind::McpServer, bad)), "{bad:?}");
    }
}

#[test]
fn a_skill_or_plugin_name_is_a_folder_name() {
    for kind in [LibraryKind::Skill, LibraryKind::Plugin] {
        for good in ["pdf", "pdf-tools", "Fill_Form.2", &"x".repeat(64)] {
            assert!(check_name(kind, good).is_ok(), "{good}");
        }
        for bad in [
            "",
            ".",
            "..",
            "-x",
            "a/b",
            "a\\b",
            "a:b",
            "a b",
            &"x".repeat(65),
        ] {
            assert!(invalid(check_name(kind, bad)), "{bad:?}");
        }
    }
}

#[test]
fn a_generated_server_name_is_always_a_valid_one() {
    assert_eq!(mcp_server_name("GitHub"), "github");
    assert_eq!(mcp_server_name("my.server"), "my_server");
    assert_eq!(mcp_server_name("a__b"), "a_b");
    assert_eq!(mcp_server_name("a. b"), "a_b");
    assert_eq!(mcp_server_name("_x_"), "x");
    assert_eq!(mcp_server_name("..."), "server");
    assert_eq!(mcp_server_name("dots"), "dots-server");
    assert_eq!(
        plugin_server_name("pdf-tools", "files"),
        "plugin_pdf-tools_files"
    );
    assert_eq!(plugin_server_name("my__plug", "_x"), "plugin_my_plug_x");
    let long = mcp_server_name(&"Long.Name".repeat(10));
    assert_eq!(long.len(), 40);
    assert_ne!(long, mcp_server_name(&"Long.Name".repeat(11)));
    for raw in [
        "GitHub",
        "a__b",
        "...",
        "dots",
        &"Long.Name".repeat(10),
        "é_ü__x",
    ] {
        let name = mcp_server_name(raw);
        assert!(
            check_name(LibraryKind::McpServer, &name).is_ok(),
            "{raw} -> {name}"
        );
    }
}

#[test]
fn secret_looking_keys() {
    for key in [
        "SF_TOKEN",
        "api_key",
        "CLIENT_SECRET",
        "Password",
        "Authorization",
        "X-Auth",
    ] {
        assert!(looks_secret(key), "{key}");
    }
    for key in ["MODE", "PATH", "Accept", "region"] {
        assert!(!looks_secret(key), "{key}");
    }
}

#[test]
fn server_configs_read_as_the_spec_writes_them() {
    let s: McpServerConfig = serde_json::from_value(json!({
        "transport": "stdio", "command": "npx", "args": ["-y", "some-server"], "cwd": null,
        "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly" }
    }))
    .unwrap();
    let McpServerConfig::Stdio {
        command, args, env, ..
    } = &s
    else {
        panic!("{s:?}")
    };
    assert_eq!(command, "npx");
    assert_eq!(args, &["-y", "some-server"]);
    assert_eq!(env["MODE"], "readonly");
    let h: McpServerConfig = serde_json::from_value(json!({
        "transport": "http", "url": "https://example.com/mcp",
        "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" }
    }))
    .unwrap();
    assert!(matches!(h, McpServerConfig::Http { .. }));
    // Optional parts may be left out.
    let bare: McpServerConfig =
        serde_json::from_value(json!({"transport": "stdio", "command": "x"})).unwrap();
    assert_eq!(bare, stdio("x"));
    let round = serde_json::to_value(&s).unwrap();
    assert_eq!(serde_json::from_value::<McpServerConfig>(round).unwrap(), s);
}

#[test]
fn an_item_carries_its_kind_next_to_its_config() {
    let item = LibraryItem {
        id: "01J0LIB".into(),
        name: "pdf".into(),
        config: LibraryConfig::Skill(SkillConfig {
            path: "C:/skills/pdf".into(),
        }),
        owner_id: None,
        created_at: "t".into(),
        updated_at: "t".into(),
    };
    let v = serde_json::to_value(&item).unwrap();
    assert_eq!(
        v,
        json!({"id": "01J0LIB", "name": "pdf", "kind": "skill",
               "config": {"path": "C:/skills/pdf"}, "owner_id": null,
               "created_at": "t", "updated_at": "t"})
    );
    assert_eq!(serde_json::from_value::<LibraryItem>(v).unwrap(), item);
    assert_eq!(item.kind(), LibraryKind::Skill);
    let plugin: LibraryConfig = serde_json::from_value(
        json!({"kind": "plugin", "config": {"path": "C:/p", "installed_id": null}}),
    )
    .unwrap();
    assert_eq!(
        plugin,
        LibraryConfig::Plugin(PluginConfig {
            path: "C:/p".into(),
            installed_id: None,
            run_hooks: false,
        })
    );
}

#[test]
fn configs_are_checked() {
    assert!(LibraryConfig::McpServer(stdio("npx")).validate().is_ok());
    assert!(invalid(LibraryConfig::McpServer(stdio(" ")).validate()));
    let http = |url: &str| {
        LibraryConfig::McpServer(McpServerConfig::Http {
            url: url.into(),
            headers: BTreeMap::new(),
        })
    };
    assert!(http("https://example.com/mcp").validate().is_ok());
    assert!(http("http://127.0.0.1:9000/mcp").validate().is_ok());
    assert!(invalid(http("example.com/mcp").validate()));
    assert!(invalid(
        LibraryConfig::Skill(SkillConfig { path: "".into() }).validate()
    ));
    assert!(invalid(
        LibraryConfig::Plugin(PluginConfig {
            path: " ".into(),
            installed_id: None,
            run_hooks: false
        })
        .validate()
    ));
}

#[test]
fn variables_are_found_and_expanded_from_the_environment() {
    let cfg: McpServerConfig = serde_json::from_value(json!({
        "transport": "stdio", "command": "${TOOLS}/srv", "args": ["--org", "${ORG}", "${ORG}"],
        "cwd": "${HOME_DIR}", "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly", "X": "${" }
    }))
    .unwrap();
    assert_eq!(
        cfg.variables(),
        ["HOME_DIR", "ORG", "SF_TOKEN"].map(String::from)
    );
    let env = |k: &str| match k {
        "TOOLS" => Some("C:/t".to_string()),
        "ORG" => Some("acme".to_string()),
        "HOME_DIR" => Some("C:/h".to_string()),
        "SF_TOKEN" => Some("s3cret".to_string()),
        _ => None,
    };
    let McpServerConfig::Stdio {
        command,
        args,
        cwd,
        env: vars,
    } = cfg.expanded(&env).unwrap()
    else {
        panic!()
    };
    // The spec reads variables in env values, args, cwd, url and headers: not in the command.
    assert_eq!(command, "${TOOLS}/srv");
    assert_eq!(args, ["--org", "acme", "acme"]);
    assert_eq!(cwd.as_deref(), Some("C:/h"));
    assert_eq!(vars["SF_TOKEN"], "s3cret");
    assert_eq!(vars["MODE"], "readonly");
    assert_eq!(vars["X"], "${", "an unclosed reference stays as written");

    let http: McpServerConfig = serde_json::from_value(json!({
        "transport": "http", "url": "https://${HOST}/mcp",
        "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" }
    }))
    .unwrap();
    assert_eq!(
        http.variables(),
        ["EXAMPLE_TOKEN", "HOST"].map(String::from)
    );
    let err = http
        .expanded(&|k| (k == "HOST").then(|| "h".into()))
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "EXAMPLE_TOKEN is not set"),
        "{err:?}"
    );
}

#[test]
fn uses_default_to_nothing() {
    let uses: Uses = serde_json::from_value(json!({})).unwrap();
    assert_eq!(uses, Uses::default());
    assert!(uses.is_empty());
    assert_eq!(
        serde_json::to_value(&uses).unwrap(),
        json!({"mcp_servers": [], "skills": [], "plugins": []})
    );
}

#[test]
fn a_new_catalog_is_unchecked() {
    let c = McpCatalog::unchecked("01J0SRV");
    assert_eq!(c.server_id, "01J0SRV");
    assert_eq!(c.status, CatalogStatus::Unchecked);
    assert_eq!(c.accepted_fingerprint, None);
    assert_eq!(c.seen_tools, None);
    assert_eq!(
        CatalogStatus::parse("changed").unwrap(),
        CatalogStatus::Changed
    );
}
```

**In `crates/dots-core/tests/local_engine_test.rs`, replace:**

```rust
        cancel,
        approved_calls: approvals,
    };
    let mut rx = engine.start(ctx).await.unwrap();
```

**with:**

```rust
        cancel,
        approved_calls: approvals,
        mcp_servers: Vec::new(),
    };
    let mut rx = engine.start(ctx).await.unwrap();
```

**Append to `crates/dots-core/tests/runner_test.rs`:**

```rust

/// Keeps the context of each run it is started with, and finishes it.
#[derive(Default)]
struct CapturingEngine(std::sync::Mutex<Vec<dots_core::engine::RunContext>>);

#[async_trait::async_trait]
impl Engine for CapturingEngine {
    async fn start(
        &self,
        ctx: dots_core::engine::RunContext,
    ) -> dots_core::Result<tokio::sync::mpsc::Receiver<EngineEvent>> {
        self.0.lock().unwrap().push(ctx);
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(EngineEvent::Finished {
            summary: "done".into(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

#[tokio::test]
async fn a_run_is_given_the_library_servers_its_dot_uses() {
    use dots_core::library::{LibraryConfig, PluginConfig};
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(CapturingEngine::default());
    let runner = runner_with(&store, &bus, &dir, engine.clone(), 1);
    runner.spawn_dispatcher(CancellationToken::new());
    let server = |command: &str| {
        LibraryConfig::McpServer(
            serde_json::from_value(json!({"transport": "stdio", "command": command})).unwrap(),
        )
    };
    let plugin = store
        .add_library_item(
            "tools",
            &LibraryConfig::Plugin(PluginConfig {
                path: "C:/p".into(),
                installed_id: None,
                run_hooks: false,
            }),
            None,
        )
        .await
        .unwrap();
    let owned = store
        .add_library_item("plugin_tools_files", &server("f"), Some(&plugin.id))
        .await
        .unwrap();
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .add_library_item("gh", &server("gh-mcp"), None)
        .await
        .unwrap();
    let mut s = spec("uses");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.uses.mcp_servers = vec!["sf".into()];
    s.uses.plugins = vec!["tools".into()];
    let dot = store.create_dot(&s).await.unwrap();
    let id = manual(&runner, &dot).await;
    wait_status(&store, &id, RunStatus::Succeeded, 5).await;
    let started = engine.0.lock().unwrap();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].mcp_servers, vec![owned, sf]);
}
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
use chrono::{Local, TimeDelta, TimeZone, Utc};
use common::{spec, temp_store};
use dots_core::model::{Dot, EngineKind};
use dots_core::policy::{Policy, Preset};
```

**with:**

```rust
use chrono::{Local, TimeDelta, TimeZone, Utc};
use common::{spec, temp_store};
use dots_core::library::LibraryConfig;
use dots_core::model::{Dot, EngineKind};
use dots_core::policy::{Policy, Preset};
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
    let mut s = spec("triage");
    s.schedule = Some(Schedule::cron("0 0 8 * * Mon-Fri"));
    s.mcp_servers = Some(json!({"sf": {"type": "stdio", "command": "sf-mcp"}}));
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec, s);
```

**with:**

```rust
    let mut s = spec("triage");
    s.schedule = Some(Schedule::cron("0 0 8 * * Mon-Fri"));
    let sf = LibraryConfig::McpServer(
        serde_json::from_value(json!({"transport": "stdio", "command": "sf-mcp"})).unwrap(),
    );
    store.add_library_item("sf", &sf, None).await.unwrap();
    s.uses.mcp_servers = vec!["sf".into()];
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec, s);
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
        "until": {"count": 0}
    })));
    let mut reserved_mcp = spec("mcp");
    reserved_mcp.mcp_servers = Some(json!({"dots": {}}));
    let mut zero_turns = spec("zero");
    zero_turns.max_turns = 0;
```

**with:**

```rust
        "until": {"count": 0}
    })));
    let mut unknown_server = spec("mcp");
    unknown_server.uses.mcp_servers = vec!["sf".into()];
    let mut empty_skill = spec("skill");
    empty_skill.uses.skills = vec!["".into()];
    let mut zero_turns = spec("zero");
    zero_turns.max_turns = 0;
```

**In `crates/dots-core/tests/store_dots_test.rs`, replace:**

```rust
        once_bounded,
        count_zero,
        reserved_mcp,
        zero_turns,
    ] {
```

**with:**

```rust
        once_bounded,
        count_zero,
        unknown_server,
        empty_skill,
        zero_turns,
    ] {
```

**Create `crates/dots-core/tests/store_library_test.rs`:**

```rust
//! The Library in the store: items, catalogs and the dots that use them (MCP/skills spec
//! sections 2, 3 and 10), and migration 0006.

mod common;

use std::collections::BTreeMap;

use common::{spec, temp_store};
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
    SkillConfig, Uses,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

fn server(command: &str) -> LibraryConfig {
    LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: command.into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
    })
}

fn skill(path: &str) -> LibraryConfig {
    LibraryConfig::Skill(SkillConfig { path: path.into() })
}

fn plugin(path: &str) -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: path.into(),
        installed_id: None,
        run_hooks: false,
    })
}

fn uses(mcp: &[&str], skills: &[&str], plugins: &[&str]) -> Uses {
    let v = |names: &[&str]| names.iter().map(|s| s.to_string()).collect();
    Uses {
        mcp_servers: v(mcp),
        skills: v(skills),
        plugins: v(plugins),
    }
}

#[tokio::test]
async fn items_are_added_read_and_listed() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    assert_eq!(sf.id.len(), 26);
    assert_eq!(sf.kind(), LibraryKind::McpServer);
    assert_eq!(sf.config, server("sf-mcp"));
    assert_eq!(sf.created_at, sf.updated_at);
    assert_eq!(store.get_library_item(&sf.id).await.unwrap(), sf);
    // The same name may be used once per kind.
    let pdf_skill = store
        .add_library_item("pdf", &skill("C:/skills/pdf"), None)
        .await
        .unwrap();
    let pdf_plugin = store
        .add_library_item("pdf", &plugin("C:/plugins/pdf"), None)
        .await
        .unwrap();
    let err = store
        .add_library_item("sf", &server("other"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(
        store
            .find_library_item(LibraryKind::Skill, "pdf")
            .await
            .unwrap(),
        Some(pdf_skill.clone())
    );
    assert_eq!(
        store
            .find_library_item(LibraryKind::McpServer, "pdf")
            .await
            .unwrap(),
        None
    );
    // By kind (servers, skills, plugins), then by name.
    let all = store.list_library_items().await.unwrap();
    assert_eq!(all, vec![sf, pdf_skill, pdf_plugin]);
    let err = store.get_library_item("nope").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn bad_names_and_configs_are_refused() {
    let (_d, store) = temp_store().await;
    for (name, config) in [
        ("a__b", server("x")),
        ("dots", server("x")),
        ("Upper", server("x")),
        ("sf", server(" ")),
        ("../x", skill("C:/s")),
        ("p", plugin("")),
    ] {
        let err = store
            .add_library_item(name, &config, None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
    }
    // Only an MCP server belongs to an item, and only to a plugin.
    let sk = store
        .add_library_item("pdf", &skill("C:/s"), None)
        .await
        .unwrap();
    let err = store
        .add_library_item("plugin_pdf_x", &server("x"), Some(&sk.id))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let pl = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    let err = store
        .add_library_item("other", &skill("C:/o"), Some(&pl.id))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert!(store
        .add_library_item("plugin_tools_x", &server("x"), Some(&pl.id))
        .await
        .is_ok());
}

#[tokio::test]
async fn a_new_server_has_an_unchecked_catalog() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    assert_eq!(
        store.get_catalog(&sf.id).await.unwrap(),
        McpCatalog::unchecked(&sf.id)
    );
    let sk = store
        .add_library_item("pdf", &skill("C:/s"), None)
        .await
        .unwrap();
    let err = store.get_catalog(&sk.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert_eq!(store.list_catalogs().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_catalog_is_written_and_accepted_only_at_the_seen_fingerprint() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    let tools = json!([{"name": "query", "annotations": {"readOnlyHint": true}}]);
    let seen = McpCatalog {
        seen_fingerprint: Some("f1".into()),
        seen_tools: Some(tools.clone()),
        status: CatalogStatus::Changed,
        checked_at: Some("2026-10-15T00:00:00.000Z".into()),
        ..McpCatalog::unchecked(&sf.id)
    };
    store.put_catalog(&seen).await.unwrap();
    assert_eq!(store.get_catalog(&sf.id).await.unwrap(), seen);

    let err = store.accept_catalog(&sf.id, "f0").await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    let accepted = store.accept_catalog(&sf.id, "f1").await.unwrap();
    assert_eq!(accepted.status, CatalogStatus::Accepted);
    assert_eq!(accepted.accepted_fingerprint.as_deref(), Some("f1"));
    assert_eq!(accepted.accepted_tools, Some(tools));
    let err = store.accept_catalog("nope", "f1").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn changing_a_server_config_resets_its_catalog() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .put_catalog(&McpCatalog {
            seen_fingerprint: Some("f1".into()),
            seen_tools: Some(json!([])),
            status: CatalogStatus::Changed,
            ..McpCatalog::unchecked(&sf.id)
        })
        .await
        .unwrap();
    store.accept_catalog(&sf.id, "f1").await.unwrap();

    // The same config: nothing to check again.
    store
        .update_library_item(&sf.id, &server("sf-mcp"))
        .await
        .unwrap();
    assert_eq!(
        store.get_catalog(&sf.id).await.unwrap().status,
        CatalogStatus::Accepted
    );

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let changed = store
        .update_library_item(&sf.id, &server("sf-mcp-2"))
        .await
        .unwrap();
    assert_eq!(changed.config, server("sf-mcp-2"));
    assert!(changed.updated_at > sf.updated_at);
    let catalog = store.get_catalog(&sf.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unchecked);
    assert_eq!(catalog.seen_fingerprint, None);
    // The list accepted before stays, for the review of the next check.
    assert_eq!(catalog.accepted_fingerprint.as_deref(), Some("f1"));

    let err = store
        .update_library_item(&sf.id, &skill("C:/s"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let err = store
        .update_library_item(&sf.id, &server(""))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let err = store
        .update_library_item("nope", &server("x"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn a_dot_uses_only_what_the_library_has() {
    let (_d, store) = temp_store().await;
    let tools = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    store
        .add_library_item("plugin_tools_files", &server("f"), Some(&tools.id))
        .await
        .unwrap();
    store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .add_library_item("pdf", &skill("C:/s"), None)
        .await
        .unwrap();

    let mut s = spec("triage");
    s.uses = uses(&["sf"], &["pdf"], &["tools"]);
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec.uses, s.uses);

    for (name, bad) in [
        ("srv", uses(&["nope"], &[], &[])),
        ("skl", uses(&[], &["sf"], &[])),
        ("plg", uses(&[], &[], &["pdf"])),
        // A plugin's server comes with the plugin; it is not ticked on its own.
        ("own", uses(&["plugin_tools_files"], &[], &[])),
        ("dup", uses(&["sf", "sf"], &[], &[])),
    ] {
        let mut s = spec(name);
        s.uses = bad;
        let err = store.create_dot(&s).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
        let mut changed = dot.spec.clone();
        changed.uses = s.uses.clone();
        let err = store.update_dot(&dot.id, &changed).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
    }
    let names: Vec<String> = store
        .list_dots()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.spec.name)
        .collect();
    assert_eq!(names, ["triage"]);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec.uses, s.uses);
}

#[tokio::test]
async fn a_run_gets_the_ticked_servers_and_those_of_the_ticked_plugins() {
    let (_d, store) = temp_store().await;
    let tools = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    let files = store
        .add_library_item("plugin_tools_files", &server("f"), Some(&tools.id))
        .await
        .unwrap();
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .add_library_item("gh", &server("gh-mcp"), None)
        .await
        .unwrap();
    let got = store
        .used_mcp_servers(&uses(&["sf"], &[], &["tools"]))
        .await
        .unwrap();
    assert_eq!(got, vec![files.clone(), sf.clone()]);
    assert_eq!(
        store
            .used_mcp_servers(&uses(&["sf"], &[], &[]))
            .await
            .unwrap(),
        vec![sf]
    );
    assert!(store
        .used_mcp_servers(&Uses::default())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn removing_an_item_unticks_it_and_takes_a_plugins_servers_along() {
    let (_d, store) = temp_store().await;
    let tools = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    let files = store
        .add_library_item("plugin_tools_files", &server("f"), Some(&tools.id))
        .await
        .unwrap();
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    let mut a = spec("a");
    a.uses = uses(&["sf"], &[], &["tools"]);
    let a = store.create_dot(&a).await.unwrap();
    let mut b = spec("b");
    b.uses = uses(&["sf"], &[], &[]);
    let b = store.create_dot(&b).await.unwrap();
    let c = store.create_dot(&spec("c")).await.unwrap();

    let names = |dots: Vec<dots_core::model::Dot>| -> Vec<String> {
        dots.into_iter().map(|d| d.spec.name).collect()
    };
    assert_eq!(names(store.dots_using(&sf).await.unwrap()), ["a", "b"]);
    assert_eq!(names(store.dots_using(&tools).await.unwrap()), ["a"]);
    // A plugin's server is used by the dots that use the plugin.
    assert_eq!(names(store.dots_using(&files).await.unwrap()), ["a"]);

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store.remove_library_item(&sf.id).await.unwrap();
    let a2 = store.get_dot(&a.id).await.unwrap();
    assert_eq!(a2.spec.uses, uses(&[], &[], &["tools"]));
    assert!(a2.updated_at > a.updated_at);
    assert_eq!(
        store.get_dot(&b.id).await.unwrap().spec.uses,
        Uses::default()
    );
    assert_eq!(store.get_dot(&c.id).await.unwrap(), c);
    let err = store.get_catalog(&sf.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");

    store.remove_library_item(&tools.id).await.unwrap();
    assert_eq!(
        store.get_dot(&a.id).await.unwrap().spec.uses,
        Uses::default()
    );
    let err = store.get_library_item(&files.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert!(store.list_catalogs().await.unwrap().is_empty());
    let err = store.remove_library_item(&tools.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

/// A database as Dots left it at migration 0005, with the dots of `rows`: id, name, policy
/// JSON and the old `mcp_servers` JSON.
async fn database_0005(path: &std::path::Path, rows: &[(&str, &str, Value, Option<Value>)]) {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version <= 5)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    for (i, (id, name, policy, mcp)) in rows.iter().enumerate() {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, mcp_servers, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', 't', ?, ?, 40, 1800, 240, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(policy.to_string())
        .bind(mcp.as_ref().map(|v| v.to_string()))
        .bind(format!("2026-10-01T00:00:0{i}.000Z"))
        .bind(format!("2026-10-01T00:00:0{i}.000Z"))
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
}

/// A policy as stored before `mcp_default` existed.
fn old_policy(preset: &str, rules: Value, default: &str) -> Value {
    json!({"preset": preset, "rules": rules, "default": default})
}

/// Migration 0006 on a database with per-dot MCP JSON (spec section 10).
#[tokio::test]
async fn migration_0006_moves_each_dots_servers_into_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let sandboxed = || {
        old_policy(
            "sandboxed",
            json!([{"tool": "Read", "action": "allow"}]),
            "ask",
        )
    };
    let sf = json!({"type": "stdio", "command": "sf-mcp", "args": ["--org", "acme"],
                    "env": {"SF_TOKEN": "abc", "MODE": "readonly", "API_KEY": "${MY_KEY}"}});
    database_0005(
        &path,
        &[
            // D1 and D2 have the same server: one item.
            (
                "D1",
                "one",
                old_policy(
                    "custom",
                    json!([{"tool": "mcp__sf__query", "action": "allow"}]),
                    "ask",
                ),
                Some(json!({ "sf": sf })),
            ),
            ("D2", "two", sandboxed(), Some(json!({ "sf": sf }))),
            // D3's `sf` differs: it becomes `sf-2`, and its rules follow.
            (
                "D3",
                "three",
                old_policy(
                    "custom",
                    json!([
                        {"tool": "mcp__sf__*", "action": "ask"},
                        {"tool": "mcp__sfx__run", "action": "deny"},
                        {"tool": "mcp__sf", "action": "deny"}
                    ]),
                    "ask",
                ),
                Some(json!({"sf": {"command": "sf-mcp", "args": ["--org", "other"]}})),
            ),
            // An http server without `type`, under a name that is not a valid one now, and its
            // Authorization header; a Trusted policy with the old `mcp__*` ask rule.
            (
                "D4",
                "four",
                old_policy(
                    "trusted",
                    json!([
                        {"tool": "Bash(git push:*)", "action": "ask"},
                        {"tool": "mcp__*", "action": "ask"},
                        {"tool": "mcp__GitHub.com__list", "action": "allow"}
                    ]),
                    "allow",
                ),
                Some(json!({
                    "GitHub.com": {"url": "https://api.example.com/mcp",
                                   "headers": {"Authorization": "Bearer xyz", "Accept": "a",
                                               "X-Api-Key": "k"}},
                    "broken": 3
                })),
            ),
            (
                "D5",
                "five",
                old_policy(
                    "read-only",
                    json!([{"tool": "Read", "action": "allow"}]),
                    "deny",
                ),
                None,
            ),
            // A custom policy may hold `mcp__*` ask on purpose: only Trusted's is the preset's.
            (
                "D6",
                "six",
                old_policy(
                    "custom",
                    json!([{"tool": "mcp__*", "action": "ask"}]),
                    "ask",
                ),
                Some(json!({})),
            ),
        ],
    )
    .await;

    let store = Store::open(&path).await.unwrap();
    let items = store.list_library_items().await.unwrap();
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["github_com", "sf", "sf-2"]);
    for item in &items {
        assert_eq!(item.owner_id, None);
        assert_eq!(
            store.get_catalog(&item.id).await.unwrap(),
            McpCatalog::unchecked(&item.id)
        );
    }
    let config = |name: &str| -> Value {
        let item = items.iter().find(|i| i.name == name).unwrap();
        serde_json::to_value(&item.config).unwrap()
    };
    // A secret-looking value becomes a variable; others, and references, are copied.
    assert_eq!(
        config("sf"),
        json!({"kind": "mcp_server", "config": {
            "transport": "stdio", "command": "sf-mcp", "args": ["--org", "acme"], "cwd": null,
            "env": {"API_KEY": "${MY_KEY}", "MODE": "readonly", "SF_TOKEN": "${SF_TOKEN}"}}})
    );
    assert_eq!(config("sf-2")["config"]["args"], json!(["--org", "other"]));
    assert_eq!(
        config("github_com"),
        json!({"kind": "mcp_server", "config": {
            "transport": "http", "url": "https://api.example.com/mcp",
            "headers": {"Accept": "a", "Authorization": "Bearer ${GITHUB_COM_TOKEN}",
                        "X-Api-Key": "${X_API_KEY}"}}})
    );

    let dot = |id: &str| {
        let store = store.clone();
        let id = id.to_string();
        async move { store.get_dot(&id).await.unwrap().spec }
    };
    let d1 = dot("D1").await;
    assert_eq!(d1.uses, uses(&["sf"], &[], &[]));
    assert_eq!(
        d1.policy.rules,
        [Rule::new("mcp__sf__query", Action::Allow)]
    );
    assert_eq!(d1.policy.mcp_default, Action::Ask);
    assert_eq!(dot("D2").await.uses, uses(&["sf"], &[], &[]));
    let d3 = dot("D3").await;
    assert_eq!(d3.uses, uses(&["sf-2"], &[], &[]));
    assert_eq!(
        d3.policy.rules,
        [
            Rule::new("mcp__sf-2__*", Action::Ask),
            Rule::new("mcp__sfx__run", Action::Deny),
            Rule::new("mcp__sf-2", Action::Deny),
        ]
    );
    let d4 = dot("D4").await;
    assert_eq!(d4.uses, uses(&["github_com"], &[], &[]));
    assert_eq!(d4.policy.preset, Preset::Trusted);
    assert_eq!(
        d4.policy.rules,
        [
            Rule::new("Bash(git push:*)", Action::Ask),
            Rule::new("mcp__github_com__list", Action::Allow),
        ]
    );
    assert_eq!(d4.policy.mcp_default, Action::Ask);
    let d5 = dot("D5").await;
    assert_eq!(d5.uses, Uses::default());
    assert_eq!(d5.policy.mcp_default, Action::Deny);
    let d6 = dot("D6").await;
    assert_eq!(d6.policy.rules, [Rule::new("mcp__*", Action::Ask)]);
    assert_eq!(d6.uses, Uses::default());
    store.close().await;

    // The policy is written with `mcp_default`; the old column and the waiting rows are gone;
    // approvals have the fingerprint column.
    let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let raw: String = sqlx::query_scalar("SELECT policy FROM dots WHERE id = 'D5'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let stored: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["mcp_default"], "deny");
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('dots')")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(columns.contains(&"uses".to_string()));
    assert!(!columns.contains(&"mcp_servers".to_string()));
    let approvals: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('approvals')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(approvals.contains(&"mcp_fingerprint".to_string()));
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0006'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(waiting, 0);
    pool.close().await;

    // Opening again converts nothing twice.
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.list_library_items().await.unwrap().len(), 3);
    assert_eq!(
        store.get_dot("D6").await.unwrap().spec.policy.rules,
        [Rule::new("mcp__*", Action::Ask)]
    );
}

/// A policy stored with `mcp_default` (written by a build that already had it) keeps its own.
#[tokio::test]
async fn migration_0006_keeps_a_stored_mcp_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let mut policy = Policy::preset(Preset::ReadOnly);
    policy.mcp_default = Action::Allow;
    database_0005(
        &path,
        &[("D1", "one", serde_json::to_value(&policy).unwrap(), None)],
    )
    .await;
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.get_dot("D1").await.unwrap().spec.policy, policy);
}
```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
  everyRule,
  localDateTimeIn,
  mcpOf,
  parseMcp,
  scheduleError,
  scheduleOf,
```

**with:**

```ts
  everyRule,
  localDateTimeIn,
  scheduleError,
  scheduleOf,
```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
describe("dot editor rules", () => {
  it("accepts a complete dot", () => {
    expect(specErrors(good, "")).toEqual({});
    expect(specErrors(specOf(sample.dot), JSON.stringify(sample.dot.mcp_servers))).toEqual({});
  });

  it("names what a new dot still lacks", () => {
    expect(Object.keys(specErrors(blankSpec, "")).sort()).toEqual([
      "instructions",
      "name",
```

**with:**

```ts
describe("dot editor rules", () => {
  it("accepts a complete dot", () => {
    expect(specErrors(good)).toEqual({});
    expect(specErrors(specOf(sample.dot))).toEqual({});
  });

  it("names what a new dot still lacks", () => {
    expect(Object.keys(specErrors(blankSpec)).sort()).toEqual([
      "instructions",
      "name",
```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
  it("checks the name the way the core does", () => {
    for (const name of ["", "has space", "ünï", "x".repeat(65)]) {
      expect(specErrors({ ...good, name }, "").name, name).toBeDefined();
    }
    for (const name of ["a", "nightly-tests_2", "x".repeat(64)]) {
      expect(specErrors({ ...good, name }, "").name, name).toBeUndefined();
    }
  });
```

**with:**

```ts
  it("checks the name the way the core does", () => {
    for (const name of ["", "has space", "ünï", "x".repeat(65)]) {
      expect(specErrors({ ...good, name }).name, name).toBeDefined();
    }
    for (const name of ["a", "nightly-tests_2", "x".repeat(64)]) {
      expect(specErrors({ ...good, name }).name, name).toBeUndefined();
    }
  });
```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
  it("wants an http endpoint for a local dot only", () => {
    const local = { ...good, engine: "local" as const };
    expect(specErrors({ ...local, endpoint_url: null }, "").endpoint_url).toMatch(/needs its/);
    expect(specErrors({ ...local, endpoint_url: "https://x" }, "").endpoint_url).toMatch(/https/);
    expect(specErrors({ ...local, endpoint_url: "http://" }, "").endpoint_url).toBeDefined();
    expect(specErrors({ ...local, endpoint_url: " http://127.0.0.1:1234 " }, "")).toEqual({});
    expect(specErrors({ ...good, endpoint_url: null }, "")).toEqual({});
  });

  it("keeps the limits inside the core's bounds", () => {
    const bad = { max_turns: 0, timeout_secs: 604801, approval_wait_secs: 281 };
    expect(Object.keys(specErrors({ ...good, ...bad }, "")).sort()).toEqual(Object.keys(bad).sort());
    const edge = { max_turns: 1, timeout_secs: 604800, approval_wait_secs: 280 };
    expect(specErrors({ ...good, ...edge }, "")).toEqual({});
    // An emptied number field.
    expect(specErrors({ ...good, max_turns: Number.NaN }, "").max_turns).toMatch(/whole number/);
    expect(specErrors({ ...good, timeout_secs: 1.5 }, "").timeout_secs).toBeDefined();
  });

  it("refuses a rule without a pattern", () => {
    const policy = { ...good.policy, rules: [{ tool: " ", action: "ask" as const }] };
    expect(specErrors({ ...good, policy }, "").policy).toBe("Every rule needs a tool pattern.");
  });

  it("reads the MCP servers field", () => {
    expect(parseMcp("  ")).toEqual({ value: null });
    expect(parseMcp('{"gh": {"type": "http"}}')).toEqual({ value: { gh: { type: "http" } } });
    expect(parseMcp("{nope")).toEqual({ error: "This is not valid JSON." });
    expect(parseMcp("[1]")).toHaveProperty("error");
    expect(parseMcp('"x"')).toHaveProperty("error");
    expect(parseMcp('{"dots": {}}')).toEqual({ error: "The server name 'dots' is reserved." });
    expect(specErrors(good, "{nope").mcp_servers).toBe("This is not valid JSON.");
  });

  it("checks and saves the MCP servers of a Claude dot only", () => {
    const local = { ...good, engine: "local" as const, endpoint_url: "http://127.0.0.1:1234" };
    // A local dot's form hides the field: what it still holds is neither checked nor saved.
    expect(specErrors(local, "{nope")).toEqual({});
    expect(mcpOf(local, '{"gh": {}}')).toEqual({ value: null });
    expect(mcpOf(local, "{nope")).toEqual({ value: null });
    expect(mcpOf(good, '{"gh": {}}')).toEqual({ value: { gh: {} } });
    expect(mcpOf(good, "{nope")).toEqual({ error: "This is not valid JSON." });
  });

  it("takes the editable fields of a stored dot", () => {
    const spec = specOf(sample.dot);
    expect(spec).not.toHaveProperty("id");
    expect(spec).not.toHaveProperty("webhook_token");
    expect(spec.name).toBe(sample.dot.name);
  });
});
```

**with:**

```ts
  it("wants an http endpoint for a local dot only", () => {
    const local = { ...good, engine: "local" as const };
    expect(specErrors({ ...local, endpoint_url: null }).endpoint_url).toMatch(/needs its/);
    expect(specErrors({ ...local, endpoint_url: "https://x" }).endpoint_url).toMatch(/https/);
    expect(specErrors({ ...local, endpoint_url: "http://" }).endpoint_url).toBeDefined();
    expect(specErrors({ ...local, endpoint_url: " http://127.0.0.1:1234 " })).toEqual({});
    expect(specErrors({ ...good, endpoint_url: null })).toEqual({});
  });

  it("keeps the limits inside the core's bounds", () => {
    const bad = { max_turns: 0, timeout_secs: 604801, approval_wait_secs: 281 };
    expect(Object.keys(specErrors({ ...good, ...bad })).sort()).toEqual(Object.keys(bad).sort());
    const edge = { max_turns: 1, timeout_secs: 604800, approval_wait_secs: 280 };
    expect(specErrors({ ...good, ...edge })).toEqual({});
    // An emptied number field.
    expect(specErrors({ ...good, max_turns: Number.NaN }).max_turns).toMatch(/whole number/);
    expect(specErrors({ ...good, timeout_secs: 1.5 }).timeout_secs).toBeDefined();
  });

  it("refuses a rule without a pattern", () => {
    const policy = { ...good.policy, rules: [{ tool: " ", action: "ask" as const }] };
    expect(specErrors({ ...good, policy }).policy).toBe("Every rule needs a tool pattern.");
  });

  it("takes the editable fields of a stored dot, with the Library items it uses", () => {
    const spec = specOf(sample.dot);
    expect(spec).not.toHaveProperty("id");
    expect(spec).not.toHaveProperty("webhook_token");
    expect(spec.name).toBe(sample.dot.name);
    expect(sample.dot.uses.mcp_servers).not.toEqual([]);
    expect(spec.uses).toEqual(sample.dot.uses);
  });
});
```

**In `ui/src/lib/dot-rules.test.ts`, replace:**

```ts
    expect(scheduleError(of({ kind: "cron", expr: "every day" }))).toBeUndefined();
    // And the draft's checks name it.
    expect(specErrors({ ...good, schedule: of({ kind: "daily", time: "" }) }, "").schedule).toBe(
      "Pick a time.",
    );
```

**with:**

```ts
    expect(scheduleError(of({ kind: "cron", expr: "every day" }))).toBeUndefined();
    // And the draft's checks name it.
    expect(specErrors({ ...good, schedule: of({ kind: "daily", time: "" }) }).schedule).toBe(
      "Pick a time.",
    );
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  it("asks a local dot for its endpoint and hides what only Claude dots have", async () => {
    const { user } = renderApp("/dots/new");
    expect(await screen.findByLabelText("MCP servers")).toBeVisible();
    expect(screen.queryByLabelText("Endpoint")).toBeNull();

    await user.selectOptions(screen.getByLabelText("Engine"), "local");
    expect(screen.queryByLabelText("MCP servers")).toBeNull();
    expect(screen.queryByLabelText("Load my Claude Code user settings")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Create dot" }));
```

**with:**

```tsx
  it("asks a local dot for its endpoint and hides what only Claude dots have", async () => {
    const { user } = renderApp("/dots/new");
    expect(await screen.findByLabelText("Load my Claude Code user settings")).toBeVisible();
    expect(screen.queryByLabelText("Endpoint")).toBeNull();

    await user.selectOptions(screen.getByLabelText("Engine"), "local");
    expect(screen.queryByLabelText("Load my Claude Code user settings")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Create dot" }));
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  });

  it("refuses MCP servers that are not a JSON object", async () => {
    const { user } = renderApp("/dots/new");
    // user-event reads "{{" as one typed "{".
    await user.type(await screen.findByLabelText("MCP servers"), "{{nope");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByText("This is not valid JSON.")).toBeVisible();
  });
});
```

**with:**

```tsx
  });

  it("has no MCP JSON box: MCP servers come from the Library", async () => {
    renderApp("/dots/new");
    await screen.findByLabelText("Name");
    expect(screen.queryByLabelText("MCP servers")).toBeNull();
  });
});
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    expect(dot.schedule?.rule.kind).toBe("cron");
    expect(screen.getByLabelText("Cron expression")).toHaveValue(cron("0 0 2 * * *").rule.expr);
    expect(screen.getByLabelText("MCP servers")).toHaveValue(
      JSON.stringify(dot.mcp_servers, null, 2),
    );

    const instructions = screen.getByLabelText("Instructions");
    await user.clear(instructions);
```

**with:**

```tsx
    expect(dot.schedule?.rule.kind).toBe("cron");
    expect(screen.getByLabelText("Cron expression")).toHaveValue(cron("0 0 2 * * *").rule.expr);
    const instructions = screen.getByLabelText("Instructions");
    await user.clear(instructions);
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  const endpoint = "http://127.0.0.1:1234";

  it("saves a dot switched to the local engine without its MCP servers", async () => {
    stored();
    const { user } = renderApp(path);
```

**with:**

```tsx
  const endpoint = "http://127.0.0.1:1234";

  it("keeps the Library items a dot uses when it is switched to the local engine", async () => {
    stored();
    const { user } = renderApp(path);
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    // The dot was stored with MCP servers, which only the Claude engine has.
    expect(dot.mcp_servers).not.toBeNull();
    expect(backend.callsTo("update_dot")).toEqual([
      {
        id: dot.id,
        spec: { ...specOf(dot), engine: "local", endpoint_url: endpoint, mcp_servers: null },
      },
    ]);
  });

  it("does not hold a local dot back for MCP text its form no longer shows", async () => {
    stored();
    const { user } = renderApp(path);
    // user-event reads "{{" as one typed "{". Added to the stored servers, it is not JSON.
    await user.type(await screen.findByLabelText("MCP servers"), "{{nope");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Fix the marked fields");
    expect(screen.getByText("This is not valid JSON.")).toBeVisible();

    await user.selectOptions(screen.getByLabelText("Engine"), "local");
    await user.type(screen.getByLabelText("Endpoint"), endpoint);
    expect(screen.queryByRole("alert")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toHaveLength(1);
    expect(backend.callsTo("update_dot")[0]).toMatchObject({
      spec: { engine: "local", mcp_servers: null },
    });
  });

```

**with:**

```tsx
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    // Both engines use MCP servers, skills and plugins.
    expect(dot.uses.mcp_servers).not.toEqual([]);
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(dot), engine: "local", endpoint_url: endpoint } },
    ]);
    expect(backend.callsTo("update_dot")[0]).toMatchObject({ spec: { uses: dot.uses } });
  });

```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test claude_cli_test --test engine_prompt_test --test library_test --test local_engine_test --test runner_test --test store_dots_test --test store_library_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/dot-rules.test.ts src/screens/DotEditorScreen.test.tsx 2>&1 | grep -E "Tests |Test Files|× |FAIL ")
```

Expected: the seven Rust test targets do not compile. The decisive lines: 30 × `` no method named `add_library_item` found for struct `Store` ``, 25 × `` no field `uses` on type `DotSpec` ``, the same for `get_catalog`, `update_library_item`, `accept_catalog`, `used_mcp_servers`, `remove_library_item`, `list_library_items`, `get_library_item`, `dots_using`, `put_catalog`, `list_catalogs` and `find_library_item`, 5 × `` error[E0432]: unresolved import `dots_core::library` ``, `` struct `RunContext` has no field named `mcp_servers` ``, then `could not compile` for `store_library_test` (82 errors), `runner_test` (8), `store_dots_test` (5), `claude_cli_test` (4), `local_engine_test`, `library_test` and `engine_prompt_test` (1 each). The UI: `Test Files  2 failed (2)`, `Tests  10 failed | 30 passed (40)`: seven tests of the dot rules (their samples have `uses`), one of the schedule rules, and in `DotEditorScreen.test.tsx` "has no MCP JSON box: MCP servers come from the Library" and "keeps the Library items a dot uses when it is switched to the local engine".

- [ ] **Step 3: Implement**

The core first, then the app's contract, then the UI files. `types.ts` is checked against the contract fixture, which the next step regenerates.

**Create `crates/dots-core/migrations/0006_library.sql`:**

```sql
-- MCP servers, skills and plugins (spec 2026-10-15-mcp-skills-plugins-design.md, sections 2, 3
-- and 10).

-- The Library. An MCP server of a plugin names the plugin in `owner_id` and goes with it.
CREATE TABLE library_items (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL,
    name       TEXT NOT NULL,
    config     TEXT NOT NULL,
    owner_id   TEXT REFERENCES library_items(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (kind, name)
);
CREATE INDEX library_items_owner ON library_items(owner_id);

-- Each MCP server's tools: the list the person accepted and the one last seen.
CREATE TABLE mcp_catalogs (
    server_id            TEXT PRIMARY KEY REFERENCES library_items(id) ON DELETE CASCADE,
    accepted_fingerprint TEXT,
    accepted_tools       TEXT,
    seen_fingerprint     TEXT,
    seen_tools           TEXT,
    status               TEXT NOT NULL,
    checked_at           TEXT,
    error                TEXT
);

-- The server's accepted fingerprint when an MCP call was asked; NULL for other tools.
ALTER TABLE approvals ADD COLUMN mcp_fingerprint TEXT;

-- The Library items each dot uses, by name.
ALTER TABLE dots ADD COLUMN uses TEXT NOT NULL
    DEFAULT '{"mcp_servers":[],"skills":[],"plugins":[]}';

-- Each dot's old per-dot servers and its policy are converted when the store opens
-- (`store/upgrade.rs`): names, shared items and the rules that name a server need code. The
-- rows wait here, and the table is dropped once they are converted.
CREATE TABLE upgrade_0006 (
    dot_id      TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE,
    mcp_servers TEXT
);
INSERT INTO upgrade_0006 (dot_id, mcp_servers) SELECT id, mcp_servers FROM dots;

ALTER TABLE dots DROP COLUMN mcp_servers;
```

**Create `crates/dots-core/src/library.rs`:**

```rust
//! The Library: MCP servers, skills and plugins that dots use (MCP/skills spec sections 2 and 3).
//! Each item has a kind, a name unique within its kind, and a config. A dot ticks items by
//! name in its `uses`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

str_enum!(LibraryKind {
    McpServer => "mcp_server",
    Skill => "skill",
    Plugin => "plugin",
});
// Where a server's tool list stands (spec section 3): the person accepted the list last seen;
// the list changed since; the last check failed; or no list was accepted yet.
str_enum!(CatalogStatus {
    Accepted => "accepted",
    Changed => "changed",
    Unreachable => "unreachable",
    Unchecked => "unchecked",
});

impl LibraryKind {
    /// The kind in a message: "MCP server", "skill", "plugin".
    pub fn label(&self) -> &'static str {
        match self {
            Self::McpServer => "MCP server",
            Self::Skill => "skill",
            Self::Plugin => "plugin",
        }
    }

    /// The list of `Uses` that names items of this kind.
    pub fn uses_key(&self) -> &'static str {
        match self {
            Self::McpServer => "mcp_servers",
            Self::Skill => "skills",
            Self::Plugin => "plugins",
        }
    }
}

/// The longest MCP server name: the tool prefix `mcp__<name>__` must leave room for the tool.
pub const MAX_SERVER_NAME: usize = 40;
/// The longest skill or plugin name.
pub const MAX_ITEM_NAME: usize = 64;

/// How Dots starts or reaches an MCP server. `${VAR}` in `env` values, `args`, `cwd`, `url` and
/// `headers` reads Dots' environment when the server starts (`expanded`); the stored config
/// keeps the reference, never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// A folder that holds a `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillConfig {
    pub path: String,
}

/// A Claude Code plugin folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginConfig {
    pub path: String,
    /// `<plugin>@<marketplace>` when picked from the installed Claude Code plugins: its folder
    /// is looked up again when it moves on an update.
    #[serde(default)]
    pub installed_id: Option<String>,
    /// The plugin's hooks run on Claude-engine dots. Off until the person allows it.
    #[serde(default)]
    pub run_hooks: bool,
}

/// An item's config, with its kind: `{"kind": "mcp_server", "config": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum LibraryConfig {
    McpServer(McpServerConfig),
    Skill(SkillConfig),
    Plugin(PluginConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub config: LibraryConfig,
    /// The plugin an MCP server comes from; it goes with the plugin.
    pub owner_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl LibraryItem {
    pub fn kind(&self) -> LibraryKind {
        self.config.kind()
    }
}

/// The Library items a dot uses, by name. A plugin's MCP servers come with the plugin and are
/// not named here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Uses {
    pub mcp_servers: Vec<String>,
    pub skills: Vec<String>,
    pub plugins: Vec<String>,
}

impl Uses {
    pub fn is_empty(&self) -> bool {
        self.mcp_servers.is_empty() && self.skills.is_empty() && self.plugins.is_empty()
    }

    /// The names of the items of `kind`.
    pub fn names(&self, kind: LibraryKind) -> &[String] {
        match kind {
            LibraryKind::McpServer => &self.mcp_servers,
            LibraryKind::Skill => &self.skills,
            LibraryKind::Plugin => &self.plugins,
        }
    }

    /// Each name is a valid one for its kind, and named once. Whether the Library has it is
    /// the store's check.
    pub fn validate(&self) -> Result<()> {
        for kind in LibraryKind::ALL {
            let mut seen = BTreeSet::new();
            for name in self.names(*kind) {
                check_name(*kind, name)?;
                if !seen.insert(name) {
                    return Err(Error::Invalid(format!(
                        "uses names the {} {name:?} twice",
                        kind.label()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// What Dots knows of an MCP server's tools (spec section 3). The tool lists are `tools/list`
/// results as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpCatalog {
    pub server_id: String,
    pub status: CatalogStatus,
    pub accepted_fingerprint: Option<String>,
    pub accepted_tools: Option<Value>,
    pub seen_fingerprint: Option<String>,
    pub seen_tools: Option<Value>,
    pub checked_at: Option<String>,
    pub error: Option<String>,
}

impl McpCatalog {
    /// The catalog of a server that was never checked: nothing accepted, nothing seen.
    pub fn unchecked(server_id: &str) -> McpCatalog {
        McpCatalog {
            server_id: server_id.to_string(),
            status: CatalogStatus::Unchecked,
            accepted_fingerprint: None,
            accepted_tools: None,
            seen_fingerprint: None,
            seen_tools: None,
            checked_at: None,
            error: None,
        }
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

/// An item name: an MCP server's is its tool prefix (`^[a-z0-9_-]{1,40}$`, no `__`, not
/// `dots`); a skill's or plugin's becomes a folder name (1-64 of `A-Za-z0-9._-`, starting with
/// a letter or digit).
pub fn check_name(kind: LibraryKind, name: &str) -> Result<()> {
    match kind {
        LibraryKind::McpServer => {
            let chars_ok = name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if name.is_empty() || name.len() > MAX_SERVER_NAME || !chars_ok {
                return invalid(format!(
                    "MCP server name {name:?}: use 1 to {MAX_SERVER_NAME} characters of a-z, \
                     0-9, '-' or '_'"
                ));
            }
            // `mcp__<server>__<tool>` is split at the first `__` after `mcp__`.
            if name.contains("__") {
                return invalid(format!("MCP server name {name:?}: '__' is not allowed"));
            }
            if name == "dots" {
                return invalid("the MCP server name 'dots' is reserved");
            }
        }
        LibraryKind::Skill | LibraryKind::Plugin => {
            let mut chars = name.chars();
            let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
            let rest_ok =
                chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
            if !first_ok || !rest_ok || name.len() > MAX_ITEM_NAME {
                return invalid(format!(
                    "{} name {name:?}: use 1 to {MAX_ITEM_NAME} characters of A-Z, a-z, 0-9, \
                     '.', '-' or '_', starting with a letter or digit",
                    kind.label()
                ));
            }
        }
    }
    Ok(())
}

impl LibraryConfig {
    pub fn kind(&self) -> LibraryKind {
        match self {
            Self::McpServer(_) => LibraryKind::McpServer,
            Self::Skill(_) => LibraryKind::Skill,
            Self::Plugin(_) => LibraryKind::Plugin,
        }
    }

    /// The config is complete: a command or an http(s) URL, a folder.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::McpServer(McpServerConfig::Stdio { command, .. })
                if command.trim().is_empty() =>
            {
                invalid("an MCP server needs the command that starts it")
            }
            Self::McpServer(McpServerConfig::Http { url, .. })
                if !(url.starts_with("http://") || url.starts_with("https://")) =>
            {
                invalid(format!(
                    "MCP server URL must start with http:// or https://: {url}"
                ))
            }
            Self::Skill(c) if c.path.trim().is_empty() => invalid("a skill needs its folder"),
            Self::Plugin(c) if c.path.trim().is_empty() => invalid("a plugin needs its folder"),
            _ => Ok(()),
        }
    }
}

/// Whether a key names a secret: `TOKEN`, `KEY`, `SECRET`, `PASSWORD` or `AUTH` in it, in any
/// case (so also the header `Authorization`). Such a value is better given as `${VAR}`.
pub fn looks_secret(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    ["TOKEN", "KEY", "SECRET", "PASSWORD", "AUTH"]
        .iter()
        .any(|word| key.contains(word))
}

/// A valid MCP server name made from `raw`: lower case; any other character than a-z, 0-9 and
/// `-` becomes `_`, runs of `_` become one, and `_` is trimmed from the ends; `dots` becomes
/// `dots-server`, nothing becomes `server`; past 40 characters it is cut to 31 plus `-` and the
/// first 8 hex characters of the SHA-256 of `raw`.
pub fn mcp_server_name(raw: &str) -> String {
    let mut name = String::new();
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            c
        } else {
            '_'
        };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    let mut name = match name.trim_matches('_') {
        "" => "server".to_string(),
        "dots" => "dots-server".to_string(),
        trimmed => trimmed.to_string(),
    };
    if name.len() > MAX_SERVER_NAME {
        let hash: String = Sha256::digest(raw.as_bytes())
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect();
        // ASCII only, so any byte index is a character boundary.
        name = format!("{}-{hash}", &name[..MAX_SERVER_NAME - 9]);
    }
    name
}

/// The name of the MCP server `server` of the plugin `plugin`: `plugin_<plugin>_<server>`, the
/// tool prefix Claude Code gives it, made valid by `mcp_server_name`.
pub fn plugin_server_name(plugin: &str, server: &str) -> String {
    mcp_server_name(&format!("plugin_{plugin}_{server}"))
}

enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

fn is_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `s` cut into text and `${VAR}` references. A `${` that is not closed, or whose name is not a
/// variable name, is text.
fn pieces(s: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) if is_var_name(&after[..end]) => {
                out.push(Piece::Text(&rest[..start]));
                out.push(Piece::Var(&after[..end]));
                rest = &after[end + 1..];
            }
            _ => {
                out.push(Piece::Text(&rest[..start + 2]));
                rest = after;
            }
        }
    }
    out.push(Piece::Text(rest));
    out
}

fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::new();
    for piece in pieces(s) {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Var(name) => match env(name) {
                Some(value) => out.push_str(&value),
                None => return invalid(format!("{name} is not set")),
            },
        }
    }
    Ok(out)
}

impl McpServerConfig {
    /// The strings that may hold `${VAR}`, in the order they are expanded.
    fn expandable(&self) -> Vec<&String> {
        match self {
            Self::Stdio { args, cwd, env, .. } => {
                env.values().chain(args.iter()).chain(cwd.iter()).collect()
            }
            Self::Http { url, headers } => std::iter::once(url).chain(headers.values()).collect(),
        }
    }

    /// The variables the config reads, sorted, each once.
    pub fn variables(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for s in self.expandable() {
            for piece in pieces(s) {
                if let Piece::Var(name) = piece {
                    names.insert(name.to_string());
                }
            }
        }
        names.into_iter().collect()
    }

    /// The config with each `${VAR}` replaced by `env(VAR)`. A variable that `env` does not
    /// have is an error naming it: "SF_TOKEN is not set".
    pub fn expanded(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<McpServerConfig> {
        let map = |m: &BTreeMap<String, String>| -> Result<BTreeMap<String, String>> {
            m.iter()
                .map(|(k, v)| Ok((k.clone(), expand(v, env)?)))
                .collect()
        };
        Ok(match self {
            Self::Stdio {
                command,
                args,
                cwd,
                env: vars,
            } => {
                let vars = map(vars)?;
                Self::Stdio {
                    command: command.clone(),
                    args: args.iter().map(|a| expand(a, env)).collect::<Result<_>>()?,
                    cwd: cwd.as_deref().map(|c| expand(c, env)).transpose()?,
                    env: vars,
                }
            }
            Self::Http { url, headers } => {
                let url = expand(url, env)?;
                Self::Http {
                    url,
                    headers: map(headers)?,
                }
            }
        })
    }
}
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod error;
pub mod events;
pub mod model;
pub mod policy;
```

**with:**

```rust
pub mod error;
pub mod events;
pub mod library;
pub mod model;
pub mod policy;
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
use serde_json::Value;

use crate::policy::{Policy, Preset};
use crate::schedule::{next_slot, Schedule, Until};
```

**with:**

```rust
use serde_json::Value;

use crate::library::Uses;
use crate::policy::{Policy, Preset};
use crate::schedule::{next_slot, Schedule, Until};
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    pub schedule: Option<Schedule>,
    pub policy: Policy,
    pub mcp_servers: Option<Value>,
    pub use_user_settings: bool,
    pub max_turns: u32,
```

**with:**

```rust
    pub schedule: Option<Schedule>,
    pub policy: Policy,
    /// The Library items the dot uses (MCP/skills spec section 2).
    #[serde(default)]
    pub uses: Uses,
    pub use_user_settings: bool,
    pub max_turns: u32,
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
            schedule: None,
            policy: Policy::preset(Preset::Sandboxed),
            mcp_servers: None,
            use_user_settings: false,
            max_turns: 40,
```

**with:**

```rust
            schedule: None,
            policy: Policy::preset(Preset::Sandboxed),
            uses: Uses::default(),
            use_user_settings: false,
            max_turns: 40,
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
        }
        self.policy.validate()?;
        if let Some(servers) = &self.mcp_servers {
            let Some(obj) = servers.as_object() else {
                return invalid("mcp_servers must be a JSON object of name -> server config");
            };
            if obj.contains_key("dots") {
                return invalid("mcp server name 'dots' is reserved");
            }
        }
        Ok(())
    }
```

**with:**

```rust
        }
        self.policy.validate()?;
        // Whether the Library has each item is checked by the store when the dot is saved.
        self.uses.validate()?;
        Ok(())
    }
```

**Create `crates/dots-core/src/store/library.rs`:**

```rust
//! The Library's items and MCP catalogs (MCP/skills spec sections 2 and 3).

use serde_json::{json, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use super::dots::{dot_from_row, DOT_COLUMNS};
use super::{opt_json, Store};
use crate::library::{
    check_name, CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, Uses,
};
use crate::model::Dot;
use crate::util::{new_id, now};
use crate::{Error, Result};

const ITEM_COLUMNS: &str = "id, kind, name, config, owner_id, created_at, updated_at";
/// Servers, then skills, then plugins; by name within a kind.
const ITEM_ORDER: &str = "CASE kind WHEN 'mcp_server' THEN 0 WHEN 'skill' THEN 1 ELSE 2 END, name";
const CATALOG_COLUMNS: &str = "server_id, status, accepted_fingerprint, accepted_tools, \
    seen_fingerprint, seen_tools, checked_at, error";

/// The `config` column: the config of the item's kind, without the kind.
fn config_json(config: &LibraryConfig) -> Result<String> {
    Ok(match config {
        LibraryConfig::McpServer(c) => serde_json::to_string(c)?,
        LibraryConfig::Skill(c) => serde_json::to_string(c)?,
        LibraryConfig::Plugin(c) => serde_json::to_string(c)?,
    })
}

fn item_from_row(row: &SqliteRow) -> Result<LibraryItem> {
    let kind = LibraryKind::parse(&row.try_get::<String, _>("kind")?)?;
    let config: Value = serde_json::from_str(&row.try_get::<String, _>("config")?)?;
    Ok(LibraryItem {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        config: serde_json::from_value(json!({ "kind": kind.as_str(), "config": config }))?,
        owner_id: row.try_get("owner_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn catalog_from_row(row: &SqliteRow) -> Result<McpCatalog> {
    Ok(McpCatalog {
        server_id: row.try_get("server_id")?,
        status: CatalogStatus::parse(&row.try_get::<String, _>("status")?)?,
        accepted_fingerprint: row.try_get("accepted_fingerprint")?,
        accepted_tools: opt_json(row.try_get("accepted_tools")?)?,
        seen_fingerprint: row.try_get("seen_fingerprint")?,
        seen_tools: opt_json(row.try_get("seen_tools")?)?,
        checked_at: row.try_get("checked_at")?,
        error: row.try_get("error")?,
    })
}

/// Refuses a name of `uses` that the Library does not have: an MCP server, skill or plugin of
/// that name. A plugin's MCP server is not one: it comes with the plugin. Run inside the
/// transaction that writes the dot, after its write, so that no removal of an item slips in
/// between.
pub(super) async fn check_uses(conn: &mut SqliteConnection, uses: &Uses) -> Result<()> {
    for kind in LibraryKind::ALL {
        let missing: Option<String> = sqlx::query_scalar(
            "SELECT value FROM json_each(?) WHERE value NOT IN \
             (SELECT name FROM library_items WHERE kind = ? AND owner_id IS NULL) LIMIT 1",
        )
        .bind(serde_json::to_string(uses.names(*kind))?)
        .bind(kind.as_str())
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(name) = missing {
            return Err(Error::Invalid(format!(
                "the Library has no {} named {name:?}",
                kind.label()
            )));
        }
    }
    Ok(())
}

impl Store {
    /// Adds an item. `owner_id` names the plugin an MCP server comes from. A new MCP server
    /// gets an `unchecked` catalog. A name its kind already has is a `Conflict`.
    pub async fn add_library_item(
        &self,
        name: &str,
        config: &LibraryConfig,
        owner_id: Option<&str>,
    ) -> Result<LibraryItem> {
        let kind = config.kind();
        check_name(kind, name)?;
        config.validate()?;
        if let Some(owner) = owner_id {
            let owner = self.get_library_item(owner).await?;
            if kind != LibraryKind::McpServer || owner.kind() != LibraryKind::Plugin {
                return Err(Error::Invalid(
                    "only an MCP server belongs to another item, and only to a plugin".into(),
                ));
            }
        }
        let id = new_id();
        let ts = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, \
             updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kind.as_str())
        .bind(name)
        .bind(config_json(config)?)
        .bind(owner_id)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => Error::Conflict(format!(
                "the Library already has a {} named {name:?}",
                kind.label()
            )),
            _ => e.into(),
        })?;
        if kind == LibraryKind::McpServer {
            sqlx::query("INSERT INTO mcp_catalogs (server_id, status) VALUES (?, ?)")
                .bind(&id)
                .bind(CatalogStatus::Unchecked.as_str())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_library_item(&id).await
    }

    pub async fn get_library_item(&self, id: &str) -> Result<LibraryItem> {
        let row = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("library item {id}")))?;
        item_from_row(&row)
    }

    pub async fn find_library_item(
        &self,
        kind: LibraryKind,
        name: &str,
    ) -> Result<Option<LibraryItem>> {
        sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE kind = ? AND name = ?"
        ))
        .bind(kind.as_str())
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(item_from_row)
        .transpose()
    }

    /// Every item: MCP servers, then skills, then plugins, each by name.
    pub async fn list_library_items(&self) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items ORDER BY {ITEM_ORDER}"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// Changes an item's config; its kind and name stay. A changed MCP server config resets
    /// the server's catalog to `unchecked` (spec section 6): what it lists must be checked and
    /// accepted again. The accepted list stays for that review.
    pub async fn update_library_item(
        &self,
        id: &str,
        config: &LibraryConfig,
    ) -> Result<LibraryItem> {
        config.validate()?;
        let old = self.get_library_item(id).await?;
        if old.kind() != config.kind() {
            return Err(Error::Invalid(format!(
                "the {} {:?} cannot become a {}",
                old.kind().label(),
                old.name,
                config.kind().label()
            )));
        }
        if old.config == *config {
            return Ok(old);
        }
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("UPDATE library_items SET config = ?, updated_at = ? WHERE id = ?")
            .bind(config_json(config)?)
            .bind(now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("library item {id}")));
        }
        if config.kind() == LibraryKind::McpServer {
            sqlx::query(
                "UPDATE mcp_catalogs SET status = ?, seen_fingerprint = NULL, seen_tools = NULL, \
                 checked_at = NULL, error = NULL WHERE server_id = ?",
            )
            .bind(CatalogStatus::Unchecked.as_str())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.get_library_item(id).await
    }

    /// Deletes an item, a plugin's MCP servers and every catalog with it, and removes its name
    /// from the `uses` of the dots that name it, in one transaction. Asking first is the
    /// caller's (`dots_using`).
    pub async fn remove_library_item(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        // The delete goes first: it takes the write lock before the dots are read.
        let removed: Option<(String, String)> =
            sqlx::query_as("DELETE FROM library_items WHERE id = ? RETURNING kind, name")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((kind, name)) = removed else {
            return Err(Error::NotFound(format!("library item {id}")));
        };
        let kind = LibraryKind::parse(&kind)?;
        let dots: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, uses FROM dots WHERE EXISTS \
             (SELECT 1 FROM json_each(dots.uses, ?) WHERE value = ?)",
        )
        .bind(format!("$.{}", kind.uses_key()))
        .bind(&name)
        .fetch_all(&mut *tx)
        .await?;
        let ts = now();
        for (dot_id, stored) in dots {
            let mut uses: Uses = serde_json::from_str(&stored)?;
            let list = match kind {
                LibraryKind::McpServer => &mut uses.mcp_servers,
                LibraryKind::Skill => &mut uses.skills,
                LibraryKind::Plugin => &mut uses.plugins,
            };
            list.retain(|n| *n != name);
            sqlx::query("UPDATE dots SET uses = ?, updated_at = ? WHERE id = ?")
                .bind(serde_json::to_string(&uses)?)
                .bind(&ts)
                .bind(&dot_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The dots that use `item`, by name. A plugin's MCP server is used by the dots that use
    /// the plugin.
    pub async fn dots_using(&self, item: &LibraryItem) -> Result<Vec<Dot>> {
        let (kind, name) = match &item.owner_id {
            Some(owner) => {
                let owner = self.get_library_item(owner).await?;
                (owner.kind(), owner.name)
            }
            None => (item.kind(), item.name.clone()),
        };
        let rows = sqlx::query(&format!(
            "SELECT {DOT_COLUMNS} FROM dots WHERE EXISTS \
             (SELECT 1 FROM json_each(dots.uses, ?) WHERE value = ?) ORDER BY name"
        ))
        .bind(format!("$.{}", kind.uses_key()))
        .bind(name)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(dot_from_row).collect()
    }

    /// The MCP servers a run of a dot with `uses` opens: those it names and those of the
    /// plugins it names, by name.
    pub async fn used_mcp_servers(&self, uses: &Uses) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE kind = 'mcp_server' AND (\
             (owner_id IS NULL AND name IN (SELECT value FROM json_each(?))) \
             OR owner_id IN (SELECT id FROM library_items WHERE kind = 'plugin' \
             AND name IN (SELECT value FROM json_each(?)))) ORDER BY name"
        ))
        .bind(serde_json::to_string(&uses.mcp_servers)?)
        .bind(serde_json::to_string(&uses.plugins)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    pub async fn get_catalog(&self, server_id: &str) -> Result<McpCatalog> {
        let row = sqlx::query(&format!(
            "SELECT {CATALOG_COLUMNS} FROM mcp_catalogs WHERE server_id = ?"
        ))
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("catalog of MCP server {server_id}")))?;
        catalog_from_row(&row)
    }

    pub async fn list_catalogs(&self) -> Result<Vec<McpCatalog>> {
        let rows = sqlx::query(&format!(
            "SELECT {CATALOG_COLUMNS} FROM mcp_catalogs ORDER BY server_id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(catalog_from_row).collect()
    }

    /// Writes a server's catalog as it is given.
    pub async fn put_catalog(&self, catalog: &McpCatalog) -> Result<()> {
        let tools = |t: &Option<Value>| t.as_ref().map(Value::to_string);
        sqlx::query(&format!(
            "INSERT INTO mcp_catalogs ({CATALOG_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (server_id) DO UPDATE SET status = excluded.status, \
             accepted_fingerprint = excluded.accepted_fingerprint, \
             accepted_tools = excluded.accepted_tools, \
             seen_fingerprint = excluded.seen_fingerprint, seen_tools = excluded.seen_tools, \
             checked_at = excluded.checked_at, error = excluded.error"
        ))
        .bind(&catalog.server_id)
        .bind(catalog.status.as_str())
        .bind(&catalog.accepted_fingerprint)
        .bind(tools(&catalog.accepted_tools))
        .bind(&catalog.seen_fingerprint)
        .bind(tools(&catalog.seen_tools))
        .bind(&catalog.checked_at)
        .bind(&catalog.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Accepts the tool list last seen, if its fingerprint is still `fingerprint`: the seen
    /// list becomes the accepted one and the status `accepted`. A list seen since is a
    /// `Conflict`: the person accepts only what was shown.
    pub async fn accept_catalog(&self, server_id: &str, fingerprint: &str) -> Result<McpCatalog> {
        let res = sqlx::query(
            "UPDATE mcp_catalogs SET accepted_fingerprint = seen_fingerprint, \
             accepted_tools = seen_tools, status = ? WHERE server_id = ? AND seen_fingerprint = ?",
        )
        .bind(CatalogStatus::Accepted.as_str())
        .bind(server_id)
        .bind(fingerprint)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            self.get_catalog(server_id).await?;
            return Err(Error::Conflict(
                "the server's tool list changed since it was shown; review it again".into(),
            ));
        }
        self.get_catalog(server_id).await
    }
}
```

**Create `crates/dots-core/src/store/upgrade.rs`:**

```rust
//! The part of migration 0006 that needs code (MCP/skills spec section 10). The SQL migration
//! moves each dot's old `mcp_servers` JSON into the table `upgrade_0006`; when the store opens,
//! `finish_0006` turns those servers into Library items ticked in the dots' `uses`, brings each
//! dot's policy up to date, and drops the table, in one transaction.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::library::{
    check_name, looks_secret, mcp_server_name, CatalogStatus, LibraryConfig, LibraryKind,
    McpServerConfig, Uses, MAX_SERVER_NAME,
};
use crate::policy::{Action, Policy, Preset};
use crate::util::{new_id, now};
use crate::Result;

/// Converts what migration 0006 left waiting, if anything.
pub(super) async fn finish_0006(pool: &SqlitePool) -> Result<()> {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0006'",
    )
    .fetch_one(pool)
    .await?;
    if waiting == 0 {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT u.dot_id, u.mcp_servers, d.policy FROM upgrade_0006 u \
         JOIN dots d ON d.id = u.dot_id ORDER BY d.created_at, d.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    // The items in the order they are made: the oldest dot's servers keep their names.
    let mut items: Vec<(String, McpServerConfig)> = Vec::new();
    let mut dots: Vec<(String, Uses, String)> = Vec::new();
    for (dot_id, servers, policy) in rows {
        let servers = servers
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut uses = Uses::default();
        let mut renames: Vec<(String, String)> = Vec::new();
        for (old, raw) in &servers {
            let Some(name) = place(old, raw, &mut items) else {
                tracing::warn!(
                    dot = %dot_id,
                    "migration 0006: the MCP server {old:?} could not be read and was left out"
                );
                continue;
            };
            if !uses.mcp_servers.contains(&name) {
                uses.mcp_servers.push(name.clone());
            }
            renames.push((old.clone(), name));
        }
        let policy = converted_policy(&policy, &renames).unwrap_or_else(|| {
            tracing::warn!(dot = %dot_id, "migration 0006: the policy could not be read");
            policy
        });
        dots.push((dot_id, uses, policy));
    }
    let ts = now();
    for (name, config) in &items {
        let id = new_id();
        sqlx::query(
            "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, \
             updated_at) VALUES (?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(&id)
        .bind(LibraryKind::McpServer.as_str())
        .bind(name)
        .bind(serde_json::to_string(config)?)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO mcp_catalogs (server_id, status) VALUES (?, ?)")
            .bind(&id)
            .bind(CatalogStatus::Unchecked.as_str())
            .execute(&mut *tx)
            .await?;
    }
    for (dot_id, uses, policy) in &dots {
        sqlx::query("UPDATE dots SET uses = ?, policy = ? WHERE id = ?")
            .bind(serde_json::to_string(uses)?)
            .bind(policy)
            .bind(dot_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE upgrade_0006")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The item that the old server `old` with config `raw` becomes: one of `items` with the same
/// name and config, or a new one added to them. A name that is taken by another config gets
/// `-2`, `-3`, ... `None` when `raw` is not a server config Dots can read.
fn place(old: &str, raw: &Value, items: &mut Vec<(String, McpServerConfig)>) -> Option<String> {
    let base = mcp_server_name(old);
    converted(&base, raw)?;
    for n in 1.. {
        let name = if n == 1 {
            base.clone()
        } else {
            let suffix = format!("-{n}");
            let keep = base.len().min(MAX_SERVER_NAME - suffix.len());
            format!("{}{suffix}", &base[..keep])
        };
        if check_name(LibraryKind::McpServer, &name).is_err() {
            continue;
        }
        // The config depends on the name: an Authorization header reads `<NAME>_TOKEN`.
        let config = converted(&name, raw)?;
        match items.iter().find(|(taken, _)| *taken == name) {
            Some((_, existing)) if *existing == config => return Some(name),
            Some(_) => continue,
            None => {
                items.push((name.clone(), config));
                return Some(name);
            }
        }
    }
    unreachable!("some suffix is free")
}

/// An upper-case variable name made from `s`: `-` and other characters become `_`.
fn var_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// A server of a Claude Code `mcpServers` block as a Library config for the server `name`. A
/// secret-looking value that is not a `${VAR}` reference is not copied: an env value becomes
/// `${<KEY>}`, an `Authorization` header `Bearer ${<NAME>_TOKEN}`, another header
/// `${<HEADER>}`. An `sse` server is read as `http`.
fn converted(name: &str, raw: &Value) -> Option<McpServerConfig> {
    let obj = raw.as_object()?;
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let strings = |key: &str| -> BTreeMap<String, String> {
        obj.get(key)
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), text(v))).collect())
            .unwrap_or_default()
    };
    let hidden = |key: &str, value: &str| looks_secret(key) && !value.contains("${");
    let default_type = if obj.contains_key("url") {
        "http"
    } else {
        "stdio"
    };
    let config = match obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(default_type)
    {
        "stdio" => McpServerConfig::Stdio {
            command: obj.get("command")?.as_str()?.to_string(),
            args: obj
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(text).collect())
                .unwrap_or_default(),
            cwd: obj.get("cwd").and_then(Value::as_str).map(String::from),
            env: strings("env")
                .into_iter()
                .map(|(k, v)| {
                    let v = if hidden(&k, &v) {
                        format!(
                            "${{{}}}",
                            k.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
                        )
                    } else {
                        v
                    };
                    (k, v)
                })
                .collect(),
        },
        "http" | "sse" => McpServerConfig::Http {
            url: obj.get("url")?.as_str()?.to_string(),
            headers: strings("headers")
                .into_iter()
                .map(|(k, v)| {
                    let v = if !hidden(&k, &v) {
                        v
                    } else if k.eq_ignore_ascii_case("authorization") {
                        format!("Bearer ${{{}_TOKEN}}", var_name(name))
                    } else {
                        format!("${{{}}}", var_name(&k))
                    };
                    (k, v)
                })
                .collect(),
        },
        _ => return None,
    };
    LibraryConfig::McpServer(config.clone())
        .validate()
        .ok()
        .map(|_| config)
}

/// `tool` with the server renamed, when it names one of `renames` (`mcp__<old>` or
/// `mcp__<old>__...`); the longest old name wins.
fn renamed(tool: &str, renames: &[(String, String)]) -> Option<String> {
    renames
        .iter()
        .filter(|(old, new)| old != new)
        .filter_map(|(old, new)| {
            let rest = tool.strip_prefix(&format!("mcp__{old}"))?;
            (rest.is_empty() || rest.starts_with("__"))
                .then(|| (old.len(), format!("mcp__{new}{rest}")))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, tool)| tool)
}

/// The stored policy brought up to date: the Trusted preset's old `mcp__*` ask rule removed,
/// rules that name a renamed server renamed, and `mcp_default` set when it is missing (`deny`
/// for Read-only, else `ask`). `None` when the stored JSON is not a policy.
fn converted_policy(stored: &str, renames: &[(String, String)]) -> Option<String> {
    let mut value: Value = serde_json::from_str(stored).ok()?;
    let obj = value.as_object_mut()?;
    let preset = obj.get("preset").and_then(Value::as_str).map(String::from);
    if let Some(Value::Array(rules)) = obj.get_mut("rules") {
        if preset.as_deref() == Some(Preset::Trusted.as_str()) {
            rules.retain(|r| !(r["tool"] == "mcp__*" && r["action"] == Action::Ask.as_str()));
        }
        for rule in rules.iter_mut() {
            if let Some(tool) = rule["tool"].as_str().and_then(|t| renamed(t, renames)) {
                rule["tool"] = Value::String(tool);
            }
        }
    }
    if !obj.contains_key("mcp_default") {
        let action = if preset.as_deref() == Some(Preset::ReadOnly.as_str()) {
            Action::Deny
        } else {
            Action::Ask
        };
        obj.insert("mcp_default".into(), json!(action.as_str()));
    }
    // Through `Policy`, so the JSON is written as Dots writes a policy.
    let policy: Policy = serde_json::from_value(value).ok()?;
    serde_json::to_string(&policy).ok()
}
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
mod approvals;
mod dots;
mod runs;
mod schedules;
mod settings;

/// The database file inside the data directory.
```

**with:**

```rust
mod approvals;
mod dots;
mod library;
mod runs;
mod schedules;
mod settings;
mod upgrade;

/// The database file inside the data directory.
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust

impl Store {
    /// Opens (creating if needed) the database file and applies migrations.
    pub async fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
```

**with:**

```rust

impl Store {
    /// Opens (creating if needed) the database file and applies migrations, with the part of
    /// migration 0006 that needs code (`upgrade.rs`).
    pub async fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
```

**In `crates/dots-core/src/store/mod.rs`, replace:**

```rust
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Store { pool })
    }
```

**with:**

```rust
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        upgrade::finish_0006(&pool).await?;
        Ok(Store { pool })
    }
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use sqlx::Row;

use super::{opt_json, Store};
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
use crate::schedule::Schedule;
```

**with:**

```rust
use sqlx::Row;

use super::library::check_uses;
use super::Store;
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
use crate::schedule::Schedule;
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
use crate::{Error, Result};

const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, workdir, \
    workspace_mode, schedule, webhook_token, policy, mcp_servers, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";

fn dot_from_row(row: &SqliteRow) -> Result<Dot> {
    Ok(Dot {
        id: row.try_get("id")?,
```

**with:**

```rust
use crate::{Error, Result};

pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";

pub(super) fn dot_from_row(row: &SqliteRow) -> Result<Dot> {
    Ok(Dot {
        id: row.try_get("id")?,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
                .transpose()?,
            policy: serde_json::from_str(&row.try_get::<String, _>("policy")?)?,
            mcp_servers: opt_json(row.try_get("mcp_servers")?)?,
            use_user_settings: row.try_get::<i64, _>("use_user_settings")? != 0,
            max_turns: row.try_get::<i64, _>("max_turns")? as u32,
```

**with:**

```rust
                .transpose()?,
            policy: serde_json::from_str(&row.try_get::<String, _>("policy")?)?,
            uses: serde_json::from_str(&row.try_get::<String, _>("uses")?)?,
            use_user_settings: row.try_get::<i64, _>("use_user_settings")? != 0,
            max_turns: row.try_get::<i64, _>("max_turns")? as u32,
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust

impl Store {
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        let spec = &prepared(spec, None)?;
        let id = new_id();
        let ts = now();
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, mcp_servers, use_user_settings, \
             max_turns, timeout_secs, approval_wait_secs, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
```

**with:**

```rust

impl Store {
    /// Creates a dot. Every item its `uses` names must be in the Library (`Invalid`).
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        let spec = &prepared(spec, None)?;
        let id = new_id();
        let ts = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, use_user_settings, \
             max_turns, timeout_secs, approval_wait_secs, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(spec.mcp_servers.as_ref().map(|v| v.to_string()))
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
```

**with:**

```rust
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        self.get_dot(&id).await
    }
```

**with:**

```rust
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        check_uses(&mut tx, &spec.uses).await?;
        tx.commit().await?;
        self.get_dot(&id).await
    }
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
    /// queued, running or awaiting runs (their workspaces were derived from the old values);
    /// otherwise nothing changes and `Error::Conflict` is returned. The check is atomic. The
    /// schedule is prepared against the stored one (`prepared`).
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        let previous = self.get_dot(id).await?.spec.schedule;
        let spec = &prepared(spec, previous.as_ref())?;
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, mcp_servers = ?, \
             use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, updated_at = ? WHERE id = ? \
```

**with:**

```rust
    /// queued, running or awaiting runs (their workspaces were derived from the old values);
    /// otherwise nothing changes and `Error::Conflict` is returned. The check is atomic. The
    /// schedule is prepared against the stored one (`prepared`). Every item its `uses` names
    /// must be in the Library (`Invalid`).
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        let previous = self.get_dot(id).await?.spec.schedule;
        let spec = &prepared(spec, previous.as_ref())?;
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, \
             use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, updated_at = ? WHERE id = ? \
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(schedule_json(spec)?)
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(spec.mcp_servers.as_ref().map(|v| v.to_string()))
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
```

**with:**

```rust
        .bind(schedule_json(spec)?)
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
        .bind(spec.workspace_mode.as_str())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        if res.rows_affected() == 0 {
            self.get_dot(id).await?;
            return Err(Error::Conflict(
```

**with:**

```rust
        .bind(spec.workspace_mode.as_str())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            self.get_dot(id).await?;
            return Err(Error::Conflict(
```

**In `crates/dots-core/src/store/dots.rs`, replace:**

```rust
            ));
        }
        self.get_dot(id).await
    }
```

**with:**

```rust
            ));
        }
        check_uses(&mut tx, &spec.uses).await?;
        tx.commit().await?;
        self.get_dot(id).await
    }
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
use tokio_util::sync::CancellationToken;

use crate::model::{ApplyState, Approval, Dot};
use crate::Result;
```

**with:**

```rust
use tokio_util::sync::CancellationToken;

use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::Result;
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
    /// `applies_approved_calls` gets any.
    pub approved_calls: Vec<Approval>,
}

```

**with:**

```rust
    /// `applies_approved_calls` gets any.
    pub approved_calls: Vec<Approval>,
    /// The Library's MCP servers the dot uses, its plugins' included, as stored: `${VAR}` not
    /// expanded (`Store::used_mcp_servers`).
    pub mcp_servers: Vec<LibraryItem>,
}

```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            _ => Vec::new(),
        };
        let secret = self.register_secret(&run.id);
        let ctx = RunContext {
```

**with:**

```rust
            _ => Vec::new(),
        };
        let mcp_servers = self.store.used_mcp_servers(&dot.spec.uses).await?;
        let secret = self.register_secret(&run.id);
        let ctx = RunContext {
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            cancel: cancel.clone(),
            approved_calls,
        };
        let mut rx = engine.start(ctx).await?;
```

**with:**

```rust
            cancel: cancel.clone(),
            approved_calls,
            mcp_servers,
        };
        let mut rx = engine.start(ctx).await?;
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust

use crate::engine::RunContext;

pub fn build_args(ctx: &RunContext, mcp_config_path: &Path) -> Vec<String> {
```

**with:**

```rust

use crate::engine::RunContext;
use crate::library::{LibraryConfig, McpServerConfig};

pub fn build_args(ctx: &RunContext, mcp_config_path: &Path) -> Vec<String> {
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
}

pub fn mcp_config(ctx: &RunContext) -> Value {
    let mut servers = serde_json::Map::new();
    if let Some(Value::Object(extra)) = &ctx.dot.spec.mcp_servers {
        for (name, cfg) in extra {
            servers.insert(name.clone(), cfg.clone());
        }
    }
```

**with:**

```rust
}

/// The run's `--mcp-config`: the Library's MCP servers the dot uses, with `${VAR}` read from
/// `env`, and Dots' own `dots` server. A server whose variable is not set is left out.
pub fn mcp_config(ctx: &RunContext, env: &dyn Fn(&str) -> Option<String>) -> Value {
    let mut servers = serde_json::Map::new();
    for item in &ctx.mcp_servers {
        let LibraryConfig::McpServer(config) = &item.config else {
            continue;
        };
        match config.expanded(env) {
            Ok(config) => {
                servers.insert(item.name.clone(), claude_entry(&config));
            }
            Err(e) => {
                tracing::warn!(run = %ctx.run_id, "MCP server {} left out: {e}", item.name)
            }
        }
    }
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
    );
    json!({ "mcpServers": servers })
}

```

**with:**

```rust
    );
    json!({ "mcpServers": servers })
}

/// A server as an entry of Claude Code's `mcpServers`.
fn claude_entry(config: &McpServerConfig) -> Value {
    match config {
        McpServerConfig::Stdio {
            command,
            args,
            cwd,
            env,
        } => {
            let mut entry =
                json!({ "type": "stdio", "command": command, "args": args, "env": env });
            if let Some(cwd) = cwd {
                entry["cwd"] = json!(cwd);
            }
            entry
        }
        McpServerConfig::Http { url, headers } => {
            json!({ "type": "http", "url": url, "headers": headers })
        }
    }
}

```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
        write_private(
            &mcp_path,
            &serde_json::to_vec_pretty(&args::mcp_config(&ctx))?,
        )
        .await?;
```

**with:**

```rust
        write_private(
            &mcp_path,
            &serde_json::to_vec_pretty(&args::mcp_config(&ctx, &|k| std::env::var(k).ok()))?,
        )
        .await?;
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::engine::EngineEvent;
use dots_core::events::RuntimeEvent;
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
```

**with:**

```rust
use dots_core::engine::EngineEvent;
use dots_core::events::RuntimeEvent;
use dots_core::library::Uses;
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
    );
    s.schedule = Some(Schedule::cron("0 0 2 * * *"));
    s.mcp_servers = Some(json!({"github": {"type": "http", "url": "http://127.0.0.1:9000/mcp"}}));
    s
}
```

**with:**

```rust
    );
    s.schedule = Some(Schedule::cron("0 0 2 * * *"));
    s.uses = Uses {
        mcp_servers: vec!["github".into()],
        skills: vec!["pdf".into()],
        plugins: vec!["pdf-tools".into()],
    };
    s
}
```

**In `ui/src/api/types.ts`, replace:**

```ts
});

const dotSpecShape = {
  name: z.string(),
```

**with:**

```ts
});

/// The Library items a dot uses, by name.
export const Uses = z.strictObject({
  mcp_servers: z.array(z.string()),
  skills: z.array(z.string()),
  plugins: z.array(z.string()),
});

const dotSpecShape = {
  name: z.string(),
```

**In `ui/src/api/types.ts`, replace:**

```ts
  schedule: nullable(Schedule),
  policy: Policy,
  mcp_servers: nullable(z.record(z.string(), z.unknown())),
  use_user_settings: z.boolean(),
  max_turns: z.number().int(),
```

**with:**

```ts
  schedule: nullable(Schedule),
  policy: Policy,
  uses: Uses,
  use_user_settings: z.boolean(),
  max_turns: z.number().int(),
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type Until = z.infer<typeof Until>;
export type Schedule = z.infer<typeof Schedule>;
export type DotSpec = z.infer<typeof DotSpec>;
export type Dot = z.infer<typeof Dot>;
```

**with:**

```ts
export type Until = z.infer<typeof Until>;
export type Schedule = z.infer<typeof Schedule>;
export type Uses = z.infer<typeof Uses>;
export type DotSpec = z.infer<typeof DotSpec>;
export type Dot = z.infer<typeof Dot>;
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
});

/// The MCP servers a text field holds. An empty field is "none".
export function parseMcp(
  text: string,
): { value: Record<string, unknown> | null } | { error: string } {
  if (text.trim() === "") return { value: null };
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { error: "This is not valid JSON." };
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { error: "Write a JSON object: server name, then its configuration." };
  }
  if ("dots" in parsed) return { error: "The server name 'dots' is reserved." };
  return { value: parsed as Record<string, unknown> };
}

/// The MCP servers a draft saves. Only a Claude dot has them: a local dot's form hides the
/// field, so what it still holds is neither checked nor saved.
export function mcpOf(spec: DotSpec, mcpText: string): ReturnType<typeof parseMcp> {
  return spec.engine === "claude" ? parseMcp(mcpText) : { value: null };
}

/// What is wrong with a draft, by field. Empty when it can be saved.
export function specErrors(spec: DotSpec, mcpText: string): SpecErrors {
  const errors: SpecErrors = {};
  const fields = Fields.safeParse(spec);
```

**with:**

```ts
});

/// What is wrong with a draft, by field. Empty when it can be saved.
export function specErrors(spec: DotSpec): SpecErrors {
  const errors: SpecErrors = {};
  const fields = Fields.safeParse(spec);
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
    errors.policy = "Every rule needs a tool pattern.";
  }
  const mcp = mcpOf(spec, mcpText);
  if ("error" in mcp) errors.mcp_servers = mcp.error;
  const schedule = scheduleError(spec.schedule);
  if (schedule) errors.schedule = schedule;
```

**with:**

```ts
    errors.policy = "Every rule needs a tool pattern.";
  }
  const schedule = scheduleError(spec.schedule);
  if (schedule) errors.schedule = schedule;
```

**In `ui/src/lib/dot-rules.ts`, replace:**

```ts
    schedule: dot.schedule,
    policy: dot.policy,
    mcp_servers: dot.mcp_servers,
    use_user_settings: dot.use_user_settings,
    max_turns: dot.max_turns,
```

**with:**

```ts
    schedule: dot.schedule,
    policy: dot.policy,
    uses: dot.uses,
    use_user_settings: dot.use_user_settings,
    max_turns: dot.max_turns,
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds: the spec, and the MCP servers as the text being typed.
export type Draft = { spec: DotSpec; mcpText: string };

function Section(props: { title: string; children: ReactNode }) {
```

**with:**

```tsx
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds. The Library items the dot uses (`spec.uses`) are kept as they are.
export type Draft = { spec: DotSpec };

function Section(props: { title: string; children: ReactNode }) {
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
      {spec.engine === "claude" && (
        <Section title="Claude Code">
          <Field
            label="MCP servers"
            htmlFor="dot-mcp"
            error={errors.mcp_servers}
            hint='A JSON object, as in a Claude Code "mcpServers" block. Empty: none.'
          >
            <Textarea
              id="dot-mcp"
              rows={5}
              className="font-mono"
              value={draft.mcpText}
              onChange={(e) => props.onChange({ ...draft, mcpText: e.target.value })}
            />
          </Field>
          <div className="flex items-center gap-2">
            <Switch
```

**with:**

```tsx
      {spec.engine === "claude" && (
        <Section title="Claude Code">
          <div className="flex items-center gap-2">
            <Switch
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { mcpOf, scheduleError, scheduleOf, specErrors, specOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
```

**with:**

```tsx
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { scheduleError, scheduleOf, specErrors, specOf } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
import { DotForm, type Draft } from "@/screens/dot-editor/DotForm";
import { RunsList } from "@/screens/dot-editor/RunsList";

function draftOf(spec: DotSpec): Draft {
  return {
    spec,
    mcpText: spec.mcp_servers ? JSON.stringify(spec.mcp_servers, null, 2) : "",
  };
}

/// The form with its Save button. `onSave` stores the spec; its rejection is shown above the
```

**with:**

```tsx
import { DotForm, type Draft } from "@/screens/dot-editor/DotForm";
import { RunsList } from "@/screens/dot-editor/RunsList";

/// The form with its Save button. `onSave` stores the spec; its rejection is shown above the
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
  children?: (form: ReactNode) => ReactNode;
}) {
  const [draft, setDraft] = useState(() => draftOf(props.initial));
  const [tried, setTried] = useState(false);
  const settings = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data;
```

**with:**

```tsx
  children?: (form: ReactNode) => ReactNode;
}) {
  const [draft, setDraft] = useState<Draft>(() => ({ spec: props.initial }));
  const [tried, setTried] = useState(false);
  const settings = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data;
```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
  const objection = sendable && preview.isError ? errorMessage(preview.error) : undefined;

  const errors = specErrors(draft.spec, draft.mcpText);
  if (objection && changed) errors.schedule ??= objection;

```

**with:**

```tsx
  const objection = sendable && preview.isError ? errorMessage(preview.error) : undefined;

  const errors = specErrors(draft.spec);
  if (objection && changed) errors.schedule ??= objection;

```

**In `ui/src/screens/DotEditorScreen.tsx`, replace:**

```tsx
    setTried(true);
    if (Object.keys(errors).length > 0) return;
    const mcp = mcpOf(draft.spec, draft.mcpText);
    save.mutate({
      ...draft.spec,
      schedule: scheduleOf(draft.spec.schedule),
      endpoint_url: draft.spec.endpoint_url?.trim() || null,
      mcp_servers: "value" in mcp ? mcp.value : null,
    });
  }
```

**with:**

```tsx
    setTried(true);
    if (Object.keys(errors).length > 0) return;
    save.mutate({
      ...draft.spec,
      schedule: scheduleOf(draft.spec.schedule),
      endpoint_url: draft.spec.endpoint_url?.trim() || null,
    });
  }
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  `timezone` TEXT (default local, unused: schedules use the PC's local time)
- `webhook_token` TEXT (random 32 bytes, base64url)
- `policy` TEXT (JSON, see §5), `mcp_servers` TEXT NULL (JSON, Claude only)
- `use_user_settings` INTEGER (0/1, default 0)
- `max_turns` INTEGER (default 40), `timeout_secs` INTEGER (default 1800, max 604800),
```

**with:**

```markdown
  `timezone` TEXT (default local, unused: schedules use the PC's local time)
- `webhook_token` TEXT (random 32 bytes, base64url)
- `policy` TEXT (JSON, see §5)
- `uses` TEXT (JSON, migration 0006): the Library items the dot uses, by name,
  `{"mcp_servers": [...], "skills": [...], "plugins": [...]}`; both engines. Saving a dot refuses
  a name the Library does not have (a plugin's MCP server is not named: it comes with the
  plugin); removing a Library item removes its name here. It replaces `mcp_servers` (the old
  per-dot JSON, Claude only), which 0006 drops.
- `use_user_settings` INTEGER (0/1, default 0)
- `max_turns` INTEGER (default 40), `timeout_secs` INTEGER (default 1800, max 604800),
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  `failed` | `unknown` | `skipped`), `applied_at` TEXT NULL (migration 0004; see
  `2026-10-13-approvals-v2-design.md` §3.4)

**schedule_state** (migration 0005; Scheduler v2 spec §4) — one row per dot with a schedule:
```

**with:**

```markdown
  `failed` | `unknown` | `skipped`), `applied_at` TEXT NULL (migration 0004; see
  `2026-10-13-approvals-v2-design.md` §3.4)
- `mcp_fingerprint` TEXT NULL (migration 0006): for an MCP call, the server's accepted
  fingerprint when it was asked (`2026-10-15-mcp-skills-plugins-design.md` §4)

**library_items** (migration 0006; MCP/skills spec §2) — the Library: `id` PK (ULID), `kind`
(`mcp_server` | `skill` | `plugin`), `name` (UNIQUE with `kind`; an MCP server's matches
`^[a-z0-9_-]{1,40}$`, without `__`, not `dots`; a skill's or plugin's is 1-64 of `A-Za-z0-9._-`
starting with a letter or digit), `config` (JSON of the kind: an MCP server's
`{"transport": "stdio", "command", "args", "cwd", "env"}` or `{"transport": "http", "url",
"headers"}` with `${VAR}` references, a skill's `{"path"}`, a plugin's `{"path", "installed_id",
"run_hooks"}`), `owner_id` (FK `library_items`, ON DELETE CASCADE: the plugin an MCP server comes
from), `created_at`, `updated_at`.

**mcp_catalogs** (migration 0006; MCP/skills spec §3) — one row per MCP server: `server_id` PK
(FK `library_items`, ON DELETE CASCADE), `accepted_fingerprint`, `accepted_tools`,
`seen_fingerprint`, `seen_tools` (JSON tool lists), `status` (`accepted` | `changed` |
`unreachable` | `unchecked`; a new server and a server whose config changed are `unchecked`),
`checked_at`, `error`.

**schedule_state** (migration 0005; Scheduler v2 spec §4) — one row per dot with a schedule:
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
columns; `0005` converts each cron string in `dots.schedule` to
`{"rule":{"kind":"cron","expr":...},"catch_up":false,"until":null}` (no catch-up, so an upgraded
schedule behaves as before), adds `runs.retry_of` and creates `schedule_state`.

All timestamps are RFC 3339 UTC strings.
```

**with:**

```markdown
columns; `0005` converts each cron string in `dots.schedule` to
`{"rule":{"kind":"cron","expr":...},"catch_up":false,"until":null}` (no catch-up, so an upgraded
schedule behaves as before), adds `runs.retry_of` and creates `schedule_state`; `0006` creates
`library_items` and `mcp_catalogs`, adds `approvals.mcp_fingerprint` and `dots.uses`, and drops
`dots.mcp_servers`. Its conversion needs code, so the SQL keeps each dot's old servers in a
table `upgrade_0006` and the store finishes it when it opens, in one transaction, then drops
that table: each old server becomes an MCP server item (`unchecked`) ticked in the dot's `uses`,
under a valid name made from the old one; dots with a server of the same name and config share
one item, another config gets `<name>-2`, `-3`, ...; the dot's rules naming `mcp__<old>` are
renamed; a secret-looking inline value becomes a variable (an env value `${<KEY>}`, an
`Authorization` header `Bearer ${<NAME>_TOKEN}`, another header `${<HEADER>}`); a server Dots
cannot read is left out with a log line. Every stored policy without `mcp_default` gets it
(`deny` for Read-only, else `ask`), and a Trusted policy loses the preset's old `mcp__*` ask rule.

All timestamps are RFC 3339 UTC strings.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
`(approval_wait_secs + 30) * 1000` on a best-effort basis; CLI 2.1.295 ignores it for the
permission-prompt tool (observed hard cap of about 300 s; CLI 2.1.295 observed 2026-10-09). The per-run MCP config contains the dots MCP server
(`http://127.0.0.1:<port>/mcp`, header `Authorization: Bearer <per-run secret>`) plus the dot's
own `mcp_servers`. An empty `--setting-sources ""` keeps every settings file out of headless
runs: the user's global hooks (GateGuard, caveman, memory hooks) and a repo's committed
`.claude/settings.json`, which could otherwise widen the CLI's own allow list beyond the dot
```

**with:**

```markdown
`(approval_wait_secs + 30) * 1000` on a best-effort basis; CLI 2.1.295 ignores it for the
permission-prompt tool (observed hard cap of about 300 s; CLI 2.1.295 observed 2026-10-09). The per-run MCP config contains the dots MCP server
(`http://127.0.0.1:<port>/mcp`, header `Authorization: Bearer <per-run secret>`) plus the Library
MCP servers the dot uses, its plugins' included, with `${VAR}` read from Dots' environment (a
server whose variable is not set is left out). An empty `--setting-sources ""` keeps every settings file out of headless
runs: the user's global hooks (GateGuard, caveman, memory hooks) and a repo's committed
`.claude/settings.json`, which could otherwise widen the CLI's own allow list beyond the dot
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  runs, after a date), both hidden for Once; the next 3 fire times from `preview_schedule`, and
  the core's objection under the fields); webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; MCP servers (Claude only);
  use-user-settings toggle.
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder. From Scheduler v2 a
```

**with:**

```markdown
  runs, after a date), both hidden for Once; the next 3 fire times from `preview_schedule`, and
  the core's objection under the fields); webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; use-user-settings toggle (Claude only).
  The dot's `uses` (Library items) is kept as stored when the form is saved.
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder. From Scheduler v2 a
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. In every spec of the fixture `mcp_servers` becomes `"uses": {"mcp_servers": ["github"], "plugins": ["pdf-tools"], "skills": ["pdf"]}`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test claude_cli_test --test engine_prompt_test --test library_test --test local_engine_test --test runner_test --test store_dots_test --test store_library_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/dot-rules.test.ts src/screens/DotEditorScreen.test.tsx src/api 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `claude_cli_test` 10 passed, `engine_prompt_test` 9, `library_test` 10, `local_engine_test` 16, `runner_test` 13, `store_dots_test` 13, `store_library_test` 10 (among them `migration_0006_moves_each_dots_servers_into_the_library`, which builds a 0005 database with inline secrets); `dots-app` 49 passed; every UI file of the run passes.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 461 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 292 passed in 21 files (three tests of the MCP JSON box were removed), typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/migrations/0006_library.sql crates/dots-core/src/engine/claude/args.rs crates/dots-core/src/engine/claude/mod.rs crates/dots-core/src/engine/mod.rs crates/dots-core/src/lib.rs crates/dots-core/src/library.rs crates/dots-core/src/model.rs crates/dots-core/src/runner.rs crates/dots-core/src/store/dots.rs crates/dots-core/src/store/library.rs crates/dots-core/src/store/mod.rs crates/dots-core/src/store/upgrade.rs crates/dots-core/tests/claude_cli_test.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/library_test.rs crates/dots-core/tests/local_engine_test.rs crates/dots-core/tests/runner_test.rs crates/dots-core/tests/store_dots_test.rs crates/dots-core/tests/store_library_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/lib/dot-rules.test.ts ui/src/lib/dot-rules.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/DotEditorScreen.tsx ui/src/screens/dot-editor/DotForm.tsx
git commit -m "feat(core): keep MCP servers, skills and plugins in a Library that dots use"
```

### Task 3: An MCP client over stdio and Streamable HTTP, with a `fake-mcp` test server

Dots becomes an MCP client (spec section 5), written like its server side: JSON-RPC 2.0 with `serde_json`, no new dependency. A session starts a stdio server as a child process with a hidden window and a Job object, or talks Streamable HTTP over `reqwest` (JSON or event-stream answers, `Mcp-Session-Id` kept); it initializes, lists the tools following `nextCursor`, calls a tool with a time limit and the run's cancel token, surfaces `notifications/tools/list_changed` to its caller, and fails every request at once with "the server stopped" when the server dies. The fingerprint of spec section 3 is computed here, and the `${VAR}` code of Task 2 moves into the module. The error texts never name the server and never show a variable's value, a URL or a header, so that a run event can be `"<server>: <error>"`. A test binary, `fake-mcp`, plays a scripted server in both modes: tools from a JSON file with read-only flags, canned results, errors and delays, a switch that changes the list and notifies, and a log of what it received. Nothing in the runtime uses the client yet. The main spec's module tree and its testing section follow.

**Files:**
- Create: `crates/dots-core/src/bin/fake-mcp.rs`, `crates/dots-core/src/mcp/http.rs`, `crates/dots-core/src/mcp/mod.rs`, `crates/dots-core/src/mcp/rpc.rs`, `crates/dots-core/src/mcp/stdio.rs`, `crates/dots-core/src/mcp/tools.rs`, `crates/dots-core/src/mcp/vars.rs`, `crates/dots-core/tests/common/fake_mcp.rs`, `crates/dots-core/tests/mcp_client_test.rs`, `crates/dots-core/tests/mcp_test.rs`
- Modify: `crates/dots-core/src/lib.rs`, `crates/dots-core/src/library.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`
- Test: `crates/dots-core/tests/common/fake_mcp.rs`, `crates/dots-core/tests/common/mod.rs`, `crates/dots-core/tests/library_test.rs`, `crates/dots-core/tests/mcp_client_test.rs`, `crates/dots-core/tests/mcp_test.rs`

**Interfaces:**
- Consumes: Task 2's `McpServerConfig`; `proc::Job` and the hidden-window spawn of `proc.rs`; `util::json_hash`; `reqwest`, `tokio`, `tokio_util::sync::CancellationToken`; `axum` in `fake-mcp`.
- Produces, in `dots_core::mcp`:
  - `PROTOCOL_VERSION = "2025-06-18"`, `KNOWN_PROTOCOL_VERSIONS = ["2025-06-18", "2025-03-26", "2024-11-05"]`, `ANSWER_TIMEOUT` (30 s), `CALL_TIMEOUT` (300 s), `CLOSE_GRACE` (2 s).
  - `SessionOptions { cwd: Option<PathBuf>, answer_timeout: Duration, close_grace: Duration }` (`Default`); `cwd` is the stdio folder when the config has none.
  - `McpError`: `Start(String)` ("could not start: SF_TOKEN is not set"), `Timeout` ("did not answer"), `Stopped` ("the server stopped"), `Cancelled`, `Server { code: i64, message: String }` ("<message> (error <code>)"), `Protocol(String)` (as is, e.g. "the server answered HTTP 401 Unauthorized"); `From<McpError> for Error` gives `Error::Other`.
  - `McpTool { name, title: Option<String>, description: Option<String>, input_schema: Value, annotations: Option<Value> }` (camelCase JSON; a missing `inputSchema` is `{"type": "object"}`; other fields dropped); `McpTool::read_only(&self) -> bool` (`annotations.readOnlyHint == true`).
  - `fingerprint(tools: &[McpTool]) -> String`: `util::json_hash` of the tools sorted by name, each with `name`, `description`, `inputSchema` and `annotations` (the absent ones left out).
  - `resolve_program(command: &str, path: Option<&OsStr>, pathext: Option<&OsStr>) -> PathBuf`.
  - `McpSession` (`Send + Sync`, shared as `Arc`): `open(name: &str, config: &McpServerConfig, env: &(dyn Fn(&str) -> Option<String> + Sync), options: SessionOptions) -> Result<McpSession, McpError>`; `name()`, `protocol_version()`, `server_info()`; `list_tools(&self) -> Result<Vec<McpTool>, McpError>`; `call_tool(&self, tool: &str, arguments: Value, limit: Duration, cancel: &CancellationToken) -> Result<Value, McpError>` (the result object as the server sent it; `isError: true` is `Ok`; a timeout or a cancel sends `notifications/cancelled` and leaves the session usable); `list_changed(&self) -> watch::Receiver<u64>` (counts the notifications; the caller lists again); `is_stopped(&self) -> bool`; `close(&self)` (idempotent; dropping a session stops it too).
  - `McpServerConfig::variables` and `expanded` now live in `mcp/vars.rs`, unchanged; their test moved to `mcp_test.rs`.
- Test server: `fake-mcp <script.json> [--http]`; the script fields are documented at the top of `src/bin/fake-mcp.rs` (`tools` with `result`, `error`, `delay_ms`, `exit`, `switch`, `ask_client`; `after_switch`; `page_size`; `protocol_version`; `init_delay_ms`; `exit_at_initialize`; `log`; `alive`; `stderr_bytes`; `ignore_eof`; `sse`; `require_header`; `get_stream`). Over HTTP it listens on 127.0.0.1 and prints its URL first.
- Test helpers in `tests/common/fake_mcp.rs`: `tool(name, read_only)`, `FakeMcp::new(script)`, `.stdio()`, `.stdio_with(args, env)`, `.serve_http().await -> FakeHttp { child, url }`, `FakeHttp::config(headers)`, `.log()`, `.received(method)`, `.wait_for(limit, pred)`, `.alive()`, `.gone_within(limit)`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/common/fake_mcp.rs`:**

```rust
//! The test MCP server `fake-mcp` (`src/bin/fake-mcp.rs`): a script file in a temp folder, its
//! call log, and the server over stdio or Streamable HTTP.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use dots_core::library::McpServerConfig;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// A tool as `fake-mcp` lists it. Add `result`, `error`, `delay_ms`, `exit`, `switch` or
/// `ask_client` to script what a call does.
pub fn tool(name: &str, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": format!("The {name} tool."),
        "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
        "annotations": {"readOnlyHint": read_only}
    })
}

/// One `fake-mcp` script. Its log and lock files are kept next to it.
pub struct FakeMcp {
    dir: TempDir,
}

impl FakeMcp {
    /// Writes `script` (see `fake-mcp.rs` for its fields) with `log` and `alive` set.
    pub fn new(mut script: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        script["log"] = json!(dir.path().join("log.jsonl"));
        script["alive"] = json!(dir.path().join("alive"));
        std::fs::write(dir.path().join("script.json"), script.to_string()).unwrap();
        Self { dir }
    }

    pub fn script(&self) -> PathBuf {
        self.dir.path().join("script.json")
    }

    /// A stdio config that starts `fake-mcp` on this script.
    pub fn stdio(&self) -> McpServerConfig {
        self.stdio_with(Vec::new(), BTreeMap::new())
    }

    /// As `stdio`, with more arguments after the script and an environment.
    pub fn stdio_with(&self, args: Vec<String>, env: BTreeMap<String, String>) -> McpServerConfig {
        let mut all = vec![self.script().to_string_lossy().to_string()];
        all.extend(args);
        McpServerConfig::Stdio {
            command: env!("CARGO_BIN_EXE_fake-mcp").to_string(),
            args: all,
            cwd: None,
            env,
        }
    }

    /// Starts `fake-mcp --http` on this script; it listens on 127.0.0.1 until dropped.
    pub async fn serve_http(&self) -> FakeHttp {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-mcp"));
        cmd.arg(self.script())
            .arg("--http")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        dots_core::proc::hide_window(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let url = tokio::time::timeout(Duration::from_secs(20), lines.next_line())
            .await
            .expect("fake-mcp did not print its URL")
            .unwrap()
            .expect("fake-mcp ended before it printed its URL");
        FakeHttp { child, url }
    }

    /// Every record of the call log: `{"start": ...}`, `{"msg": ...}` (with `http`, `session`,
    /// `protocol` and `auth` over http), `{"http": "GET"|"DELETE", ...}`.
    pub fn log(&self) -> Vec<Value> {
        match std::fs::read_to_string(self.dir.path().join("log.jsonl")) {
            Ok(text) => text
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The messages received with `method`, in order.
    pub fn received(&self, method: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter(|r| r["msg"]["method"] == method)
            .map(|r| r["msg"].clone())
            .collect()
    }

    /// Waits until the log has a record for which `pred` holds.
    pub async fn wait_for(&self, limit: Duration, pred: impl Fn(&Value) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if self.log().iter().any(&pred) {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Whether a `fake-mcp` process of this script runs: it holds a lock on `alive`.
    pub fn alive(&self) -> bool {
        match std::fs::File::open(self.dir.path().join("alive")) {
            Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            Err(_) => false,
        }
    }

    /// Whether the process ends within `limit`.
    pub async fn gone_within(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while self.alive() {
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    }
}

/// `fake-mcp` in http mode. Killed when dropped.
pub struct FakeHttp {
    pub child: Child,
    /// `http://127.0.0.1:<port>/mcp`
    pub url: String,
}

impl FakeHttp {
    pub fn config(&self, headers: BTreeMap<String, String>) -> McpServerConfig {
        McpServerConfig::Http {
            url: self.url.clone(),
            headers,
        }
    }
}
```

**In `crates/dots-core/tests/common/mod.rs`, replace:**

```rust

pub mod clock;
pub mod tz;

```

**with:**

```rust

pub mod clock;
pub mod fake_mcp;
pub mod tz;

```

**In `crates/dots-core/tests/library_test.rs`, replace:**

```rust
//! The Library's model: item names, configs, `${VAR}` references and a dot's uses (MCP/skills
//! spec sections 2 and 10).

use std::collections::BTreeMap;
```

**with:**

```rust
//! The Library's model: item names, configs and a dot's uses (MCP/skills spec sections 2 and
//! 10). `${VAR}` references are tested with the MCP client (`mcp_test.rs`).

use std::collections::BTreeMap;
```

**In `crates/dots-core/tests/library_test.rs`, replace:**

```rust

#[test]
fn variables_are_found_and_expanded_from_the_environment() {
    let cfg: McpServerConfig = serde_json::from_value(json!({
        "transport": "stdio", "command": "${TOOLS}/srv", "args": ["--org", "${ORG}", "${ORG}"],
        "cwd": "${HOME_DIR}", "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly", "X": "${" }
    }))
    .unwrap();
    assert_eq!(
        cfg.variables(),
        ["HOME_DIR", "ORG", "SF_TOKEN"].map(String::from)
    );
    let env = |k: &str| match k {
        "TOOLS" => Some("C:/t".to_string()),
        "ORG" => Some("acme".to_string()),
        "HOME_DIR" => Some("C:/h".to_string()),
        "SF_TOKEN" => Some("s3cret".to_string()),
        _ => None,
    };
    let McpServerConfig::Stdio {
        command,
        args,
        cwd,
        env: vars,
    } = cfg.expanded(&env).unwrap()
    else {
        panic!()
    };
    // The spec reads variables in env values, args, cwd, url and headers: not in the command.
    assert_eq!(command, "${TOOLS}/srv");
    assert_eq!(args, ["--org", "acme", "acme"]);
    assert_eq!(cwd.as_deref(), Some("C:/h"));
    assert_eq!(vars["SF_TOKEN"], "s3cret");
    assert_eq!(vars["MODE"], "readonly");
    assert_eq!(vars["X"], "${", "an unclosed reference stays as written");

    let http: McpServerConfig = serde_json::from_value(json!({
        "transport": "http", "url": "https://${HOST}/mcp",
        "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" }
    }))
    .unwrap();
    assert_eq!(
        http.variables(),
        ["EXAMPLE_TOKEN", "HOST"].map(String::from)
    );
    let err = http
        .expanded(&|k| (k == "HOST").then(|| "h".into()))
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "EXAMPLE_TOKEN is not set"),
        "{err:?}"
    );
}

#[test]
fn uses_default_to_nothing() {
    let uses: Uses = serde_json::from_value(json!({})).unwrap();
```

**with:**

```rust

#[test]
fn uses_default_to_nothing() {
    let uses: Uses = serde_json::from_value(json!({})).unwrap();
```

**Create `crates/dots-core/tests/mcp_client_test.rs`:**

```rust
//! The MCP client against `fake-mcp`, over stdio and Streamable HTTP (MCP/skills spec sections 5
//! and 6).

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use common::fake_mcp::{tool, FakeMcp};
use dots_core::library::McpServerConfig;
use dots_core::mcp::{
    fingerprint, McpError, McpSession, SessionOptions, CALL_TIMEOUT, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(10);

fn no_env(_: &str) -> Option<String> {
    None
}

fn names(tools: &[dots_core::mcp::McpTool]) -> Vec<&str> {
    tools.iter().map(|t| t.name.as_str()).collect()
}

async fn open(fake: &FakeMcp) -> McpSession {
    McpSession::open("fake", &fake.stdio(), &no_env, SessionOptions::default())
        .await
        .unwrap()
}

async fn call(session: &McpSession, tool: &str, args: Value) -> Result<Value, McpError> {
    session
        .call_tool(tool, args, CALL_TIMEOUT, &CancellationToken::new())
        .await
}

fn same_dir(a: &str, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}

#[tokio::test]
async fn a_stdio_session_initializes_lists_every_page_and_calls() {
    let fake = FakeMcp::new(json!({
        "page_size": 2,
        "tools": [tool("a", true), tool("b", false), tool("c", false)]
    }));
    let work = tempfile::tempdir().unwrap();
    let env = |k: &str| match k {
        "FAKE_TOKEN" => Some("s3cret".to_string()),
        "FAKE_ARG" => Some("on".to_string()),
        _ => None,
    };
    let config = fake.stdio_with(
        vec!["--flag=${FAKE_ARG}".into()],
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "${FAKE_TOKEN}".to_string())]),
    );
    let options = SessionOptions {
        cwd: Some(work.path().to_path_buf()),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &config, &env, options)
        .await
        .unwrap();
    assert_eq!(session.name(), "fake");
    assert_eq!(session.protocol_version(), PROTOCOL_VERSION);
    assert_eq!(session.server_info()["name"], "fake-mcp");

    let tools = session.list_tools().await.unwrap();
    assert_eq!(names(&tools), ["a", "b", "c"]);
    assert!(tools[0].read_only() && !tools[1].read_only());

    let result = call(&session, "b", json!({"q": "x"})).await.unwrap();
    assert_eq!(result["content"][0]["text"], r#"b: {"q":"x"}"#);
    assert_eq!(result.get("isError"), None);

    // The server was started with the variables expanded, in the default folder.
    let start = fake.log()[0]["start"].clone();
    assert_eq!(start["env"]["FAKE_MCP_TOKEN"], "s3cret");
    assert_eq!(start["args"][1], "--flag=on");
    assert!(same_dir(start["cwd"].as_str().unwrap(), work.path()));

    let init = &fake.received("initialize")[0];
    assert_eq!(init["params"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(init["params"]["clientInfo"]["name"], "dots");
    let methods: Vec<Value> = fake
        .log()
        .iter()
        .filter_map(|r| r["msg"].get("method").cloned())
        .collect();
    assert_eq!(
        methods,
        json!([
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/list",
            "tools/call"
        ])
        .as_array()
        .unwrap()
        .clone()
    );
    let lists = fake.received("tools/list");
    assert_eq!(lists[1]["params"]["cursor"], "2");
    session.close().await;
}

/// A run shares its sessions between tasks: the engine's calls and the gateway's.
#[tokio::test]
async fn a_session_is_shared_between_tasks_and_answers_calls_in_any_order() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(500);
    let fake = FakeMcp::new(json!({"tools": [slow, tool("fast", true)]}));
    let config = fake.stdio();
    let session = tokio::spawn(async move {
        McpSession::open("fake", &config, &no_env, SessionOptions::default()).await
    })
    .await
    .unwrap()
    .unwrap();
    let session = std::sync::Arc::new(session);
    let first = {
        let session = session.clone();
        tokio::spawn(async move { call(&session, "slow", json!({"n": 1})).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = call(&session, "fast", json!({"n": 2})).await.unwrap();
    assert!(
        !first.is_finished(),
        "the fast call did not wait for the slow one"
    );
    assert_eq!(second["content"][0]["text"], r#"fast: {"n":2}"#);
    let first = first.await.unwrap().unwrap();
    assert_eq!(first["content"][0]["text"], r#"slow: {"n":1}"#);
    session.close().await;
}

#[tokio::test]
async fn the_configs_own_folder_wins_over_the_default() {
    let fake = FakeMcp::new(json!({"tools": []}));
    let own = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let McpServerConfig::Stdio {
        command, args, env, ..
    } = fake.stdio()
    else {
        unreachable!()
    };
    let config = McpServerConfig::Stdio {
        command,
        args,
        env,
        cwd: Some(own.path().to_string_lossy().to_string()),
    };
    let options = SessionOptions {
        cwd: Some(other.path().to_path_buf()),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &config, &no_env, options)
        .await
        .unwrap();
    let start = fake.log()[0]["start"].clone();
    assert!(same_dir(start["cwd"].as_str().unwrap(), own.path()));
    session.close().await;
}

#[tokio::test]
async fn a_missing_variable_or_command_stops_the_server_from_starting() {
    let fake = FakeMcp::new(json!({"tools": []}));
    let config = fake.stdio_with(
        Vec::new(),
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "${FAKE_TOKEN}".to_string())]),
    );
    let err = McpSession::open("fake", &config, &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "could not start: FAKE_TOKEN is not set");
    assert!(fake.log().is_empty(), "nothing was started");

    let missing = McpServerConfig::Stdio {
        command: "dots-no-such-command-xyz".into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
    };
    let err = McpSession::open("fake", &missing, &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, McpError::Start(m) if m.contains("dots-no-such-command-xyz")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_server_that_exits_or_is_silent_at_initialize_cannot_start() {
    let fake = FakeMcp::new(json!({"tools": [], "exit_at_initialize": true}));
    let err = McpSession::open("fake", &fake.stdio(), &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(&err, McpError::Start(_)), "{err:?}");

    let fake = FakeMcp::new(json!({"tools": [], "init_delay_ms": 10_000}));
    let options = SessionOptions {
        answer_timeout: Duration::from_millis(300),
        ..SessionOptions::default()
    };
    let begun = Instant::now();
    let err = McpSession::open("fake", &fake.stdio(), &no_env, options)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, McpError::Timeout), "{err:?}");
    assert_eq!(err.to_string(), "did not answer");
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert!(
        fake.gone_within(WAIT).await,
        "a server that did not start is stopped"
    );
}

#[tokio::test]
async fn the_protocol_version_must_be_one_dots_knows() {
    let fake = FakeMcp::new(json!({"tools": [], "protocol_version": "2025-03-26"}));
    let session = open(&fake).await;
    assert_eq!(session.protocol_version(), "2025-03-26");
    session.close().await;

    let fake = FakeMcp::new(json!({"tools": [], "protocol_version": "1999-01-01"}));
    let err = McpSession::open("fake", &fake.stdio(), &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, McpError::Protocol(m) if m.contains("1999-01-01")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_tool_error_and_an_error_result_come_back() {
    let mut failing = tool("fail", false);
    failing["error"] = json!({"code": -32000, "message": "it broke"});
    let mut is_error = tool("soft", false);
    is_error["result"] = json!({"content": [{"type": "text", "text": "no"}], "isError": true});
    let fake = FakeMcp::new(json!({"tools": [failing, is_error]}));
    let session = open(&fake).await;

    let err = call(&session, "fail", json!({})).await.unwrap_err();
    assert!(
        matches!(&err, McpError::Server { code: -32000, message } if message == "it broke"),
        "{err:?}"
    );
    let err = call(&session, "nope", json!({})).await.unwrap_err();
    assert!(
        matches!(err, McpError::Server { code: -32602, .. }),
        "{err:?}"
    );
    let result = call(&session, "soft", json!({})).await.unwrap();
    assert_eq!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "no");
    session.close().await;
}

#[tokio::test]
async fn a_slow_call_times_out_or_is_cancelled_and_the_server_is_told() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(10_000);
    let fake = FakeMcp::new(json!({"tools": [slow, tool("fast", true)]}));
    let session = open(&fake).await;

    let begun = Instant::now();
    let err = session
        .call_tool(
            "slow",
            json!({}),
            Duration::from_millis(300),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Timeout), "{err:?}");
    assert!(begun.elapsed() < Duration::from_secs(5));

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let begun = Instant::now();
    let err = session
        .call_tool("slow", json!({}), CALL_TIMEOUT, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Cancelled), "{err:?}");
    assert!(begun.elapsed() < Duration::from_secs(5));

    // The server hears of both, and the session goes on.
    assert!(
        fake.wait_for(WAIT, |_| fake_cancelled_count(&fake) == 2)
            .await
    );
    let calls = fake.received("tools/call");
    let cancelled = fake.received("notifications/cancelled");
    assert_eq!(cancelled[0]["params"]["requestId"], calls[0]["id"]);
    assert_eq!(cancelled[1]["params"]["requestId"], calls[1]["id"]);
    assert!(call(&session, "fast", json!({})).await.is_ok());
    session.close().await;
}

fn fake_cancelled_count(fake: &FakeMcp) -> usize {
    fake.received("notifications/cancelled").len()
}

#[tokio::test]
async fn a_list_change_is_surfaced_and_the_new_list_read() {
    let mut switch = tool("switch", false);
    switch["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "tools": [tool("a", true), switch],
        "after_switch": [tool("a", false), tool("z", true)]
    }));
    let session = open(&fake).await;
    let mut changed = session.list_changed();
    let before = session.list_tools().await.unwrap();
    assert_eq!(*changed.borrow(), 0);

    call(&session, "switch", json!({})).await.unwrap();
    tokio::time::timeout(WAIT, changed.changed())
        .await
        .expect("no list change seen")
        .unwrap();
    assert_eq!(*changed.borrow_and_update(), 1);
    let after = session.list_tools().await.unwrap();
    assert_eq!(names(&after), ["a", "z"]);
    assert_ne!(fingerprint(&before), fingerprint(&after));
    session.close().await;
}

#[tokio::test]
async fn requests_from_the_server_are_refused_except_ping() {
    let mut roots = tool("roots", false);
    roots["ask_client"] = json!("roots/list");
    let mut ping = tool("ping", false);
    ping["ask_client"] = json!("ping");
    let fake = FakeMcp::new(json!({"tools": [roots, ping]}));
    let session = open(&fake).await;
    call(&session, "roots", json!({})).await.unwrap();
    call(&session, "ping", json!({})).await.unwrap();
    // `fake-mcp` gives its requests the id `s-<method>`; the answers are in its log.
    let answer = |id: &str| {
        fake.log()
            .into_iter()
            .find(|r| r["msg"]["id"] == id && r["msg"].get("method").is_none())
            .map(|r| r["msg"].clone())
    };
    assert!(fake.wait_for(WAIT, |r| r["msg"]["id"] == "s-ping").await);
    assert!(
        fake.wait_for(WAIT, |r| r["msg"]["id"] == "s-roots/list")
            .await
    );
    assert_eq!(answer("s-roots/list").unwrap()["error"]["code"], -32601);
    assert_eq!(answer("s-ping").unwrap()["result"], json!({}));
    session.close().await;
}

#[tokio::test]
async fn a_server_that_dies_stops_its_session() {
    let mut crash = tool("crash", false);
    crash["exit"] = json!(true);
    let fake = FakeMcp::new(json!({"tools": [crash, tool("a", true)]}));
    let session = open(&fake).await;
    let err = call(&session, "crash", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert_eq!(err.to_string(), "the server stopped");
    assert!(session.is_stopped());
    let begun = Instant::now();
    let err = call(&session, "a", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert!(
        begun.elapsed() < Duration::from_secs(1),
        "no wait, no restart"
    );
    assert!(matches!(session.list_tools().await, Err(McpError::Stopped)));
    session.close().await;
}

#[tokio::test]
async fn close_stops_the_server_and_so_does_dropping_the_session() {
    let fake = FakeMcp::new(json!({"tools": []}));
    let session = open(&fake).await;
    assert!(fake.alive());
    session.close().await;
    assert!(!fake.alive(), "close waits until the server is gone");
    assert!(session.is_stopped());
    session.close().await; // twice is fine

    // A server that does not end when its input closes is killed after the grace time.
    let fake = FakeMcp::new(json!({"tools": [], "ignore_eof": true}));
    let options = SessionOptions {
        close_grace: Duration::from_millis(200),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &fake.stdio(), &no_env, options)
        .await
        .unwrap();
    let begun = Instant::now();
    session.close().await;
    assert!(!fake.alive());
    assert!(begun.elapsed() < Duration::from_secs(5));

    let fake = FakeMcp::new(json!({"tools": [], "ignore_eof": true}));
    let session = open(&fake).await;
    drop(session);
    assert!(
        fake.gone_within(WAIT).await,
        "dropping the session stops it"
    );
}

#[tokio::test]
async fn a_server_that_writes_much_to_stderr_is_not_blocked() {
    let fake = FakeMcp::new(json!({"tools": [tool("a", true)], "stderr_bytes": 1_000_000}));
    let options = SessionOptions {
        answer_timeout: Duration::from_secs(20),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &fake.stdio(), &no_env, options)
        .await
        .unwrap();
    assert_eq!(names(&session.list_tools().await.unwrap()), ["a"]);
    session.close().await;
}

// --- Streamable HTTP ---

fn bearer(var: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("Authorization".to_string(), format!("Bearer ${{{var}}}"))])
}

fn token_env(k: &str) -> Option<String> {
    (k == "FAKE_TOKEN").then(|| "s3cret".to_string())
}

#[tokio::test]
async fn an_http_session_with_json_answers_keeps_its_session_id() {
    let fake = FakeMcp::new(json!({
        "page_size": 1,
        "tools": [tool("a", true), tool("b", false)],
        "require_header": {"authorization": "Bearer s3cret"}
    }));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(bearer("FAKE_TOKEN")),
        &token_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(session.protocol_version(), PROTOCOL_VERSION);
    assert_eq!(names(&session.list_tools().await.unwrap()), ["a", "b"]);
    let result = call(&session, "a", json!({"q": 1})).await.unwrap();
    assert_eq!(result["content"][0]["text"], r#"a: {"q":1}"#);
    session.close().await;

    let log = fake.log();
    let posts: Vec<&Value> = log.iter().filter(|r| r["http"] == "POST").collect();
    assert_eq!(posts[0]["msg"]["method"], "initialize");
    assert_eq!(posts[0]["session"], Value::Null);
    let id = posts[1]["session"]
        .as_str()
        .expect("a session id")
        .to_string();
    for p in &posts[1..] {
        assert_eq!(p["session"], id.as_str(), "{p}");
        assert_eq!(p["protocol"], PROTOCOL_VERSION, "{p}");
    }
    assert!(posts.iter().all(|p| p["auth"] == "Bearer s3cret"));
    let delete = log.iter().find(|r| r["http"] == "DELETE").expect("DELETE");
    assert_eq!(delete["session"], id.as_str());

    // A wrong header is refused, and the error does not show it.
    let err = McpSession::open(
        "web",
        &http.config(bearer("FAKE_TOKEN")),
        &|_: &str| Some("wrong".to_string()),
        SessionOptions::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(err.to_string().contains("401"), "{err}");
    assert!(!err.to_string().contains("wrong"), "{err}");
}

#[tokio::test]
async fn an_http_session_reads_event_streams_with_a_list_change_in_them() {
    let mut switch = tool("switch", false);
    switch["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "sse": true,
        "tools": [tool("a", true), switch],
        "after_switch": [tool("b", false)]
    }));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(names(&session.list_tools().await.unwrap()).len(), 2);
    let changed = session.list_changed();
    call(&session, "switch", json!({})).await.unwrap();
    assert_eq!(
        *changed.borrow(),
        1,
        "the notification came before the answer"
    );
    assert_eq!(names(&session.list_tools().await.unwrap()), ["b"]);

    // A request in the stream is answered with a POST of its own.
    let fake2 = FakeMcp::new(json!({"sse": true, "tools": [roots_tool()]}));
    let http2 = fake2.serve_http().await;
    let session2 = McpSession::open(
        "web2",
        &http2.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    call(&session2, "roots", json!({})).await.unwrap();
    assert!(
        fake2
            .wait_for(WAIT, |r| r["msg"]["id"] == "s-roots/list"
                && r["msg"]["error"]["code"] == -32601)
            .await
    );
    session.close().await;
    session2.close().await;
}

fn roots_tool() -> Value {
    let mut roots = tool("roots", false);
    roots["ask_client"] = json!("roots/list");
    roots
}

#[tokio::test]
async fn an_http_list_change_on_the_get_stream_is_surfaced() {
    let mut switch = tool("switch", false);
    switch["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "get_stream": true,
        "tools": [switch],
        "after_switch": [tool("b", false)]
    }));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    let mut changed = session.list_changed();
    assert!(
        fake.wait_for(WAIT, |r| r["http"] == "GET").await,
        "the client listens on the GET stream"
    );
    call(&session, "switch", json!({})).await.unwrap();
    tokio::time::timeout(WAIT, changed.changed())
        .await
        .expect("no list change seen")
        .unwrap();
    assert_eq!(names(&session.list_tools().await.unwrap()), ["b"]);
    session.close().await;
}

#[tokio::test]
async fn an_http_call_can_be_cancelled_and_a_dead_server_stops_the_session() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(10_000);
    let mut crash = tool("crash", false);
    crash["exit"] = json!(true);
    let fake = FakeMcp::new(json!({"tools": [slow, crash, tool("a", true)]}));
    let mut http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();

    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = session
        .call_tool("slow", json!({}), CALL_TIMEOUT, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Cancelled), "{err:?}");
    assert!(
        fake.wait_for(WAIT, |r| r["msg"]["method"] == "notifications/cancelled")
            .await
    );

    // The call that kills it fails; once the server is gone, the session is stopped.
    assert!(call(&session, "crash", json!({})).await.is_err());
    let _ = http.child.wait().await;
    let err = call(&session, "a", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert!(session.is_stopped());
    session.close().await;
}

#[tokio::test]
async fn an_http_server_that_is_not_there_cannot_start() {
    // A port that was just free.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = McpServerConfig::Http {
        url: format!("http://127.0.0.1:{port}/mcp?key=${{FAKE_TOKEN}}"),
        headers: BTreeMap::new(),
    };
    let err = McpSession::open("web", &config, &token_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(&err, McpError::Start(_)), "{err:?}");
    assert!(
        !err.to_string().contains("s3cret"),
        "the URL is not shown: {err}"
    );
}
```

**Create `crates/dots-core/tests/mcp_test.rs`:**

```rust
use dots_core::library::McpServerConfig;
use dots_core::mcp::{fingerprint, McpError, McpTool};
use dots_core::util::json_hash;
use dots_core::Error;
use serde_json::{json, Value};

fn tool(v: Value) -> McpTool {
    serde_json::from_value(v).unwrap()
}

#[test]
fn variables_are_found_and_expanded_from_the_environment() {
    let cfg: McpServerConfig = serde_json::from_value(json!({
        "transport": "stdio", "command": "${TOOLS}/srv", "args": ["--org", "${ORG}", "${ORG}"],
        "cwd": "${HOME_DIR}", "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly", "X": "${" }
    }))
    .unwrap();
    assert_eq!(
        cfg.variables(),
        ["HOME_DIR", "ORG", "SF_TOKEN"].map(String::from)
    );
    let env = |k: &str| match k {
        "TOOLS" => Some("C:/t".to_string()),
        "ORG" => Some("acme".to_string()),
        "HOME_DIR" => Some("C:/h".to_string()),
        "SF_TOKEN" => Some("s3cret".to_string()),
        _ => None,
    };
    let McpServerConfig::Stdio {
        command,
        args,
        cwd,
        env: vars,
    } = cfg.expanded(&env).unwrap()
    else {
        panic!()
    };
    // The spec reads variables in env values, args, cwd, url and headers: not in the command.
    assert_eq!(command, "${TOOLS}/srv");
    assert_eq!(args, ["--org", "acme", "acme"]);
    assert_eq!(cwd.as_deref(), Some("C:/h"));
    assert_eq!(vars["SF_TOKEN"], "s3cret");
    assert_eq!(vars["MODE"], "readonly");
    assert_eq!(vars["X"], "${", "an unclosed reference stays as written");

    let http: McpServerConfig = serde_json::from_value(json!({
        "transport": "http", "url": "https://${HOST}/mcp",
        "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" }
    }))
    .unwrap();
    assert_eq!(
        http.variables(),
        ["EXAMPLE_TOKEN", "HOST"].map(String::from)
    );
    let err = http
        .expanded(&|k| (k == "HOST").then(|| "h".into()))
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "EXAMPLE_TOKEN is not set"),
        "{err:?}"
    );
}

#[test]
fn a_tool_keeps_what_dots_shows_and_drops_the_rest() {
    let t = tool(json!({
        "name": "search", "title": "Search", "description": "Finds things.",
        "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
        "outputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true},
        "_meta": {"x": 1}
    }));
    assert_eq!(t.name, "search");
    assert_eq!(t.title.as_deref(), Some("Search"));
    assert_eq!(t.description.as_deref(), Some("Finds things."));
    assert!(t.read_only());
    assert_eq!(
        serde_json::to_value(&t).unwrap(),
        json!({
            "name": "search", "title": "Search", "description": "Finds things.",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
            "annotations": {"readOnlyHint": true}
        })
    );

    // A tool without a schema takes no arguments; only `readOnlyHint: true` is read-only.
    let bare = tool(json!({"name": "ping"}));
    assert_eq!(bare.input_schema, json!({"type": "object"}));
    assert!(!bare.read_only());
    for hint in [json!(false), json!("true"), json!(1), Value::Null] {
        let t = tool(json!({"name": "w", "annotations": {"readOnlyHint": hint}}));
        assert!(!t.read_only(), "{hint}");
    }
}

#[test]
fn the_fingerprint_hashes_the_sorted_tools_in_canonical_json() {
    let b = json!({"name": "b", "description": "B.", "inputSchema": {"type": "object"},
                   "annotations": {"readOnlyHint": true}, "title": "Bee",
                   "outputSchema": {"type": "string"}, "_meta": {"v": 1}});
    let a = json!({"name": "a", "inputSchema": {"required": [], "type": "object"}});
    let tools = vec![tool(b.clone()), tool(a.clone())];
    // Sorted by name; `title`, `outputSchema` and `_meta` are not part of it; a field the
    // server did not send is left out.
    assert_eq!(
        fingerprint(&tools),
        json_hash(&json!([
            {"name": "a", "inputSchema": {"type": "object", "required": []}},
            {"name": "b", "description": "B.", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true}}
        ]))
    );
    assert_eq!(fingerprint(&tools).len(), 64);
    assert_eq!(
        fingerprint(&tools),
        fingerprint(&[tool(a.clone()), tool(b.clone())]),
        "the order of the list does not matter"
    );

    let base = fingerprint(&tools);
    let with = |key: &str, value: Value| {
        let mut changed = b.clone();
        changed[key] = value;
        fingerprint(&[tool(a.clone()), tool(changed)])
    };
    assert_eq!(with("title", json!("Other")), base);
    assert_eq!(with("outputSchema", json!({})), base);
    assert_eq!(with("_meta", json!({"v": 2})), base);
    assert_ne!(with("description", json!("Changed.")), base);
    assert_ne!(with("inputSchema", json!({"type": "object", "x": 1})), base);
    assert_ne!(with("annotations", json!({"readOnlyHint": false})), base);
    assert_ne!(fingerprint(&[tool(a.clone())]), base, "a removed tool");
    assert_ne!(fingerprint(&[]), base);
}

#[test]
fn errors_say_what_happened_without_the_server_name() {
    let cases = [
        (
            McpError::Start("SF_TOKEN is not set".into()),
            "could not start: SF_TOKEN is not set",
        ),
        (McpError::Timeout, "did not answer"),
        (McpError::Stopped, "the server stopped"),
        (McpError::Cancelled, "cancelled"),
        (
            McpError::Server {
                code: -32602,
                message: "unknown tool: x".into(),
            },
            "unknown tool: x (error -32602)",
        ),
        (
            McpError::Protocol("the server answered HTTP 500".into()),
            "the server answered HTTP 500",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
        assert!(matches!(Error::from(err), Error::Other(m) if m == text));
    }
}

#[cfg(windows)]
#[test]
fn a_bare_command_is_found_on_the_path_with_its_extension() {
    use dots_core::mcp::resolve_program;
    use std::ffi::OsString;
    use std::path::PathBuf;

    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("npx.cmd"), "@echo off\r\n").unwrap();
    std::fs::write(other.path().join("npx.exe"), "").unwrap();
    let path = std::env::join_paths([dir.path(), other.path()]).unwrap();
    let pathext = OsString::from(".COM;.EXE;.BAT;.CMD");

    // The first folder with any match wins, as cmd.exe finds a program.
    assert_eq!(
        resolve_program("npx", Some(&path), Some(&pathext)),
        dir.path().join("npx.cmd")
    );
    // A command with a folder or an extension, or one not found, is left as written.
    for command in ["node.exe", "C:/tools/x", "missing"] {
        assert_eq!(
            resolve_program(command, Some(&path), Some(&pathext)),
            PathBuf::from(command)
        );
    }
    // Without PATHEXT the usual list is used.
    assert_eq!(
        resolve_program("npx", Some(&path), None),
        dir.path().join("npx.cmd")
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test library_test --test mcp_client_test --test mcp_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
```

Expected: the two new test targets do not compile: 3 × `` error[E0432]: unresolved import `dots_core::mcp` ``, `` error[E0433]: cannot find `mcp` in `dots_core` ``, 2 × ``error: environment variable `CARGO_BIN_EXE_fake-mcp` not defined at compile time``, then ``could not compile `dots-core` (test "mcp_test") due to 2 previous errors`` and `(test "mcp_client_test") due to 4 previous errors`. `library_test` (one test moved out) compiles but is not run while the others fail to build.

- [ ] **Step 3: Implement**

**In `crates/dots-core/src/library.rs`, replace:**

```rust

/// How Dots starts or reaches an MCP server. `${VAR}` in `env` values, `args`, `cwd`, `url` and
/// `headers` reads Dots' environment when the server starts (`expanded`); the stored config
/// keeps the reference, never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
```

**with:**

```rust

/// How Dots starts or reaches an MCP server. `${VAR}` in `env` values, `args`, `cwd`, `url` and
/// `headers` reads Dots' environment when the server starts (`expanded`, in `mcp::vars`); the
/// stored config keeps the reference, never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
```

**In `crates/dots-core/src/library.rs`, replace:**

```rust
    mcp_server_name(&format!("plugin_{plugin}_{server}"))
}

enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

fn is_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `s` cut into text and `${VAR}` references. A `${` that is not closed, or whose name is not a
/// variable name, is text.
fn pieces(s: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) if is_var_name(&after[..end]) => {
                out.push(Piece::Text(&rest[..start]));
                out.push(Piece::Var(&after[..end]));
                rest = &after[end + 1..];
            }
            _ => {
                out.push(Piece::Text(&rest[..start + 2]));
                rest = after;
            }
        }
    }
    out.push(Piece::Text(rest));
    out
}

fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::new();
    for piece in pieces(s) {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Var(name) => match env(name) {
                Some(value) => out.push_str(&value),
                None => return invalid(format!("{name} is not set")),
            },
        }
    }
    Ok(out)
}

impl McpServerConfig {
    /// The strings that may hold `${VAR}`, in the order they are expanded.
    fn expandable(&self) -> Vec<&String> {
        match self {
            Self::Stdio { args, cwd, env, .. } => {
                env.values().chain(args.iter()).chain(cwd.iter()).collect()
            }
            Self::Http { url, headers } => std::iter::once(url).chain(headers.values()).collect(),
        }
    }

    /// The variables the config reads, sorted, each once.
    pub fn variables(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for s in self.expandable() {
            for piece in pieces(s) {
                if let Piece::Var(name) = piece {
                    names.insert(name.to_string());
                }
            }
        }
        names.into_iter().collect()
    }

    /// The config with each `${VAR}` replaced by `env(VAR)`. A variable that `env` does not
    /// have is an error naming it: "SF_TOKEN is not set".
    pub fn expanded(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<McpServerConfig> {
        let map = |m: &BTreeMap<String, String>| -> Result<BTreeMap<String, String>> {
            m.iter()
                .map(|(k, v)| Ok((k.clone(), expand(v, env)?)))
                .collect()
        };
        Ok(match self {
            Self::Stdio {
                command,
                args,
                cwd,
                env: vars,
            } => {
                let vars = map(vars)?;
                Self::Stdio {
                    command: command.clone(),
                    args: args.iter().map(|a| expand(a, env)).collect::<Result<_>>()?,
                    cwd: cwd.as_deref().map(|c| expand(c, env)).transpose()?,
                    env: vars,
                }
            }
            Self::Http { url, headers } => {
                let url = expand(url, env)?;
                Self::Http {
                    url,
                    headers: map(headers)?,
                }
            }
        })
    }
}
```

**with:**

```rust
    mcp_server_name(&format!("plugin_{plugin}_{server}"))
}
```

**Create `crates/dots-core/src/mcp/vars.rs`:**

```rust
//! `${VAR}` in an MCP server's config (MCP/skills spec section 2): the value of `VAR` in Dots'
//! own environment, read when the server starts. A variable that is not set stops the server
//! from starting.

use std::collections::{BTreeMap, BTreeSet};

use crate::library::McpServerConfig;
use crate::{Error, Result};

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

fn is_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `s` cut into text and `${VAR}` references. A `${` that is not closed, or whose name is not a
/// variable name, is text.
fn pieces(s: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) if is_var_name(&after[..end]) => {
                out.push(Piece::Text(&rest[..start]));
                out.push(Piece::Var(&after[..end]));
                rest = &after[end + 1..];
            }
            _ => {
                out.push(Piece::Text(&rest[..start + 2]));
                rest = after;
            }
        }
    }
    out.push(Piece::Text(rest));
    out
}

fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::new();
    for piece in pieces(s) {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Var(name) => match env(name) {
                Some(value) => out.push_str(&value),
                None => return invalid(format!("{name} is not set")),
            },
        }
    }
    Ok(out)
}

impl McpServerConfig {
    /// The strings that may hold `${VAR}`, in the order they are expanded.
    fn expandable(&self) -> Vec<&String> {
        match self {
            Self::Stdio { args, cwd, env, .. } => {
                env.values().chain(args.iter()).chain(cwd.iter()).collect()
            }
            Self::Http { url, headers } => std::iter::once(url).chain(headers.values()).collect(),
        }
    }

    /// The variables the config reads, sorted, each once.
    pub fn variables(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for s in self.expandable() {
            for piece in pieces(s) {
                if let Piece::Var(name) = piece {
                    names.insert(name.to_string());
                }
            }
        }
        names.into_iter().collect()
    }

    /// The config with each `${VAR}` replaced by `env(VAR)`. A variable that `env` does not
    /// have is an error naming it: "SF_TOKEN is not set".
    pub fn expanded(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<McpServerConfig> {
        let map = |m: &BTreeMap<String, String>| -> Result<BTreeMap<String, String>> {
            m.iter()
                .map(|(k, v)| Ok((k.clone(), expand(v, env)?)))
                .collect()
        };
        Ok(match self {
            Self::Stdio {
                command,
                args,
                cwd,
                env: vars,
            } => {
                let vars = map(vars)?;
                Self::Stdio {
                    command: command.clone(),
                    args: args.iter().map(|a| expand(a, env)).collect::<Result<_>>()?,
                    cwd: cwd.as_deref().map(|c| expand(c, env)).transpose()?,
                    env: vars,
                }
            }
            Self::Http { url, headers } => {
                let url = expand(url, env)?;
                Self::Http {
                    url,
                    headers: map(headers)?,
                }
            }
        })
    }
}
```

**Create `crates/dots-core/src/mcp/tools.rs`:**

```rust
//! A server's tools as Dots keeps them, and their fingerprint (MCP/skills spec section 3).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::util::json_hash;

/// One tool of a `tools/list` result: what Dots shows, checks and sends to a model. The
/// server's `outputSchema`, `_meta` and other fields are not kept. A catalog's tool list is a
/// JSON array of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A JSON Schema of the arguments; a tool that sends none takes no arguments.
    #[serde(default = "no_arguments")]
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

fn no_arguments() -> Value {
    json!({ "type": "object" })
}

impl McpTool {
    /// The tool says it changes nothing: `annotations.readOnlyHint` is `true`.
    pub fn read_only(&self) -> bool {
        self.annotations
            .as_ref()
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// SHA-256 (hex) of the tools sorted by name, as one canonical JSON array (keys sorted, no
/// whitespace) of `{name, description, inputSchema, annotations}` per tool; a field the server
/// did not send is left out. `title`, `outputSchema` and `_meta` are not part of it.
pub fn fingerprint(tools: &[McpTool]) -> String {
    let mut sorted: Vec<&McpTool> = tools.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let parts = sorted
        .into_iter()
        .map(|t| {
            let mut part = Map::new();
            part.insert("name".into(), json!(t.name));
            if let Some(description) = &t.description {
                part.insert("description".into(), json!(description));
            }
            part.insert("inputSchema".into(), t.input_schema.clone());
            if let Some(annotations) = &t.annotations {
                part.insert("annotations".into(), annotations.clone());
            }
            Value::Object(part)
        })
        .collect();
    json_hash(&Value::Array(parts))
}
```

**Create `crates/dots-core/src/mcp/rpc.rs`:**

```rust
//! JSON-RPC 2.0 between a session and its server: messages out, and the `Peer` that takes what
//! the server sends, whichever transport carries it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use tokio::sync::{oneshot, watch};

use super::McpError;

type Answer = Result<Value, McpError>;

pub(super) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub(super) fn notification(method: &str, params: Option<Value>) -> Value {
    match params {
        Some(params) => json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        None => json!({ "jsonrpc": "2.0", "method": method }),
    }
}

/// Whether `message` is the answer (a result or an error) to the request `id`.
pub(super) fn is_answer_to(message: &Value, id: u64) -> bool {
    message.get("method").is_none() && message.get("id").and_then(Value::as_u64) == Some(id)
}

/// The requests waiting for an answer, and what the server has said outside of answers.
pub(super) struct Peer {
    name: String,
    next_id: AtomicU64,
    /// `None` once the server has stopped: no request waits any more.
    pending: Mutex<Option<HashMap<u64, oneshot::Sender<Answer>>>>,
    list_changed: watch::Sender<u64>,
}

impl Peer {
    pub(super) fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(Some(HashMap::new())),
            list_changed: watch::Sender::new(0),
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// A new request id and where its answer arrives. Taken before the request is sent, so an
    /// answer cannot come first. Once the server has stopped, the answer is `Stopped` at once.
    pub(super) fn expect(&self) -> (u64, oneshot::Receiver<Answer>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        match self.pending.lock().unwrap().as_mut() {
            Some(pending) => {
                pending.insert(id, tx);
            }
            None => {
                let _ = tx.send(Err(McpError::Stopped));
            }
        }
        (id, rx)
    }

    /// Drops the request `id`: it timed out or was cancelled, and a late answer is ignored.
    pub(super) fn forget(&self, id: u64) {
        if let Some(pending) = self.pending.lock().unwrap().as_mut() {
            pending.remove(&id);
        }
    }

    /// The server has stopped: every waiting request fails with `Stopped`, and so does every
    /// later one.
    pub(super) fn stop(&self) {
        if let Some(pending) = self.pending.lock().unwrap().take() {
            for (_, tx) in pending {
                let _ = tx.send(Err(McpError::Stopped));
            }
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.pending.lock().unwrap().is_none()
    }

    pub(super) fn list_changed(&self) -> watch::Receiver<u64> {
        self.list_changed.subscribe()
    }

    /// Takes one message from the server. An answer goes to the request waiting for it;
    /// `notifications/tools/list_changed` is counted; other notifications are ignored. A request
    /// is refused with a JSON-RPC error, except `ping`: the reply to send back is returned.
    pub(super) fn receive(&self, message: Value) -> Option<Value> {
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id");
        match (method, id) {
            (Some("ping"), Some(id)) => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
            (Some(method), Some(id)) => Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("Dots does not support {method}") }
            })),
            (Some("notifications/tools/list_changed"), None) => {
                self.list_changed.send_modify(|n| *n += 1);
                None
            }
            (Some(_), None) => None,
            (None, Some(id)) => {
                let waiting = id.as_u64().and_then(|id| {
                    self.pending
                        .lock()
                        .unwrap()
                        .as_mut()
                        .and_then(|p| p.remove(&id))
                });
                match waiting {
                    Some(tx) => {
                        let _ = tx.send(answer(&message));
                    }
                    None => {
                        tracing::debug!(server = %self.name, "an answer nobody waits for: {id}")
                    }
                }
                None
            }
            (None, None) => {
                tracing::debug!(server = %self.name, "a message that is not JSON-RPC");
                None
            }
        }
    }
}

fn answer(message: &Value) -> Answer {
    match message.get("error") {
        Some(error) => Err(McpError::Server {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message")
                .to_string(),
        }),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    }
}
```

**Create `crates/dots-core/src/mcp/stdio.rs`:**

```rust
//! The stdio transport: the server is a child process, one JSON-RPC message per line on its
//! stdin and stdout. It runs with no console window and in a Job Object (`proc::Job`), so
//! what it starts goes with it. Its stderr goes to the app log, capped.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Stdio as Piped;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::rpc::Peer;
use super::McpError;
use crate::proc::Job;

/// The stderr bytes of one server that reach the log; the rest is read and dropped.
const STDERR_LOG_CAP: usize = 64 * 1024;

/// What `Stdio::start` runs, with every `${VAR}` already expanded.
pub(super) struct Launch<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<PathBuf>,
    pub env: &'a BTreeMap<String, String>,
    pub grace: Duration,
}

pub(super) struct Stdio {
    outbox: mpsc::UnboundedSender<String>,
    /// Fired by `close`, or by dropping this: the server's input closes, and it is stopped.
    shutdown: CancellationToken,
    _on_drop: DropGuard,
    /// The task that owns the process; it ends once the process has gone.
    monitor: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Stdio {
    pub(super) fn start(peer: Arc<Peer>, launch: Launch<'_>) -> Result<Stdio, McpError> {
        let program = resolve_program(
            launch.command,
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("PATHEXT").as_deref(),
        );
        let mut cmd = Command::new(&program);
        cmd.args(launch.args)
            .envs(launch.env)
            .stdin(Piped::piped())
            .stdout(Piped::piped())
            .stderr(Piped::piped())
            .kill_on_drop(true);
        if let Some(dir) = &launch.cwd {
            cmd.current_dir(dir);
        }
        crate::proc::hide_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            McpError::Start(match e.kind() {
                std::io::ErrorKind::NotFound => format!("{} was not found", launch.command),
                _ => format!("{}: {e}", launch.command),
            })
        })?;
        // Before the server reads anything: what it starts ahead of this is outside the job.
        let job = Job::assign(&child).unwrap_or_else(|e| {
            tracing::warn!(server = %peer.name(), "no job object for the MCP server, using kill_tree: {e}");
            Job::fallback(&child)
        });
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let shutdown = CancellationToken::new();
        let (outbox, inbox) = mpsc::unbounded_channel();
        tokio::spawn(write(stdin, inbox, peer.clone(), shutdown.clone()));
        tokio::spawn(read(stdout, peer.clone(), outbox.clone()));
        tokio::spawn(log_stderr(stderr, peer.name().to_string()));
        let monitor = tokio::spawn(monitor(child, job, peer, shutdown.clone(), launch.grace));
        Ok(Stdio {
            outbox,
            _on_drop: shutdown.clone().drop_guard(),
            shutdown,
            monitor: tokio::sync::Mutex::new(Some(monitor)),
        })
    }

    pub(super) fn send(&self, message: &Value) -> Result<(), McpError> {
        self.outbox
            .send(message.to_string())
            .map_err(|_| McpError::Stopped)
    }

    pub(super) async fn close(&self) {
        self.shutdown.cancel();
        if let Some(monitor) = self.monitor.lock().await.take() {
            let _ = monitor.await;
        }
    }
}

/// Writes each message as one line. On shutdown it drops stdin, which ends the server's input.
async fn write(
    mut stdin: ChildStdin,
    mut inbox: mpsc::UnboundedReceiver<String>,
    peer: Arc<Peer>,
    shutdown: CancellationToken,
) {
    loop {
        let line = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            line = inbox.recv() => match line {
                Some(line) => line,
                None => return,
            },
        };
        let written = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        };
        if let Err(e) = written.await {
            tracing::warn!(server = %peer.name(), "writing to the MCP server failed: {e}");
            peer.stop();
            return;
        }
    }
}

/// Gives each line to the peer, and sends back its replies to requests. The end of stdout is
/// the end of the server.
async fn read(stdout: ChildStdout, peer: Arc<Peer>, outbox: mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = buf.trim_ascii();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(message) => {
                if let Some(reply) = peer.receive(message) {
                    let _ = outbox.send(reply.to_string());
                }
            }
            Err(_) => tracing::debug!(server = %peer.name(), "a line that is not JSON on stdout"),
        }
    }
    peer.stop();
}

/// Logs stderr line by line, up to `STDERR_LOG_CAP` bytes, and drains the rest so the server
/// never blocks on a full pipe.
async fn log_stderr(stderr: ChildStderr, name: String) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    let mut logged = 0;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) if logged < STDERR_LOG_CAP => {
                logged += n;
                let text = String::from_utf8_lossy(&buf);
                tracing::info!(server = %name, "MCP server stderr: {}", text.trim_end());
                if logged >= STDERR_LOG_CAP {
                    tracing::warn!(server = %name, "the MCP server writes more to stderr; the rest is not logged");
                }
            }
            Ok(_) => {}
        }
    }
}

/// Owns the process until it has gone. On shutdown the server has `grace` to exit by itself
/// once its input is closed (the writer closes it on the same signal), then the job is
/// terminated. Either way the job is closed after, so nothing the server started stays. A
/// server that exits by itself is stopped by the reader, at the end of its stdout, so that
/// answers it wrote before it exited still arrive.
async fn monitor(
    mut child: Child,
    job: Job,
    peer: Arc<Peer>,
    shutdown: CancellationToken,
    grace: Duration,
) {
    let closing = tokio::select! {
        status = child.wait() => {
            tracing::warn!(server = %peer.name(), "the MCP server exited: {status:?}");
            false
        }
        _ = shutdown.cancelled() => true,
    };
    if closing && tokio::time::timeout(grace, child.wait()).await.is_err() {
        job.terminate().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    job.close();
    if closing {
        peer.stop();
    }
}

/// The program to start for `command`. On Windows a bare name (no folder, no extension) is
/// looked up as cmd.exe does: in each folder of `path`, with each extension of `pathext`
/// (default `.COM;.EXE;.BAT;.CMD`), so `npx` finds `npx.cmd`. Otherwise, and when nothing is
/// found, the command as written.
pub fn resolve_program(command: &str, path: Option<&OsStr>, pathext: Option<&OsStr>) -> PathBuf {
    #[cfg(windows)]
    {
        let bare = std::path::Path::new(command);
        if bare.components().count() == 1 && bare.extension().is_none() {
            let exts = pathext
                .and_then(OsStr::to_str)
                .unwrap_or(".COM;.EXE;.BAT;.CMD")
                .to_ascii_lowercase();
            for dir in path.map(std::env::split_paths).into_iter().flatten() {
                for ext in exts.split(';').filter(|e| !e.is_empty()) {
                    let candidate = dir.join(format!("{command}{ext}"));
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (path, pathext);
    PathBuf::from(command)
}
```

**Create `crates/dots-core/src/mcp/http.rs`:**

```rust
//! The Streamable HTTP transport: each message is a POST, answered with JSON or with an event
//! stream (SSE); the `Mcp-Session-Id` the server gives at `initialize` goes with every later
//! request, and so does `MCP-Protocol-Version`. A GET stream, when the server offers one,
//! brings what the server says on its own, such as `notifications/tools/list_changed`.
//!
//! Errors never show the URL or the headers: they can hold expanded `${VAR}` values.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde_json::Value;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::rpc::{self, Peer};
use super::{McpError, NOTE_TIMEOUT};

const SESSION_HEADER: &str = "Mcp-Session-Id";
const VERSION_HEADER: &str = "MCP-Protocol-Version";
/// The wait before the GET stream is opened again after the server ended it.
const RELISTEN: Duration = Duration::from_millis(500);

pub(super) struct Http {
    inner: Arc<Inner>,
    /// Fired by `close`, or by dropping this: the GET stream ends.
    listening: CancellationToken,
    _on_drop: DropGuard,
}

struct Inner {
    peer: Arc<Peer>,
    client: reqwest::Client,
    url: String,
    headers: BTreeMap<String, String>,
    session_id: Mutex<Option<String>>,
    /// Set once `initialize` has been answered.
    version: Mutex<Option<String>>,
}

impl Http {
    pub(super) fn new(peer: Arc<Peer>, url: String, headers: BTreeMap<String, String>) -> Self {
        let listening = CancellationToken::new();
        Self {
            inner: Arc::new(Inner {
                peer,
                client: reqwest::Client::new(),
                url,
                headers,
                session_id: Mutex::new(None),
                version: Mutex::new(None),
            }),
            _on_drop: listening.clone().drop_guard(),
            listening,
        }
    }

    pub(super) async fn send(
        &self,
        message: &Value,
        awaiting: Option<u64>,
    ) -> Result<(), McpError> {
        self.inner.send(message, awaiting).await
    }

    /// `initialize` was answered with `version`: it is sent with every later request, a
    /// failure to connect from now on means that the server stopped, and the GET stream opens.
    pub(super) fn ready(&self, version: &str) {
        *self.inner.version.lock().unwrap() = Some(version.to_string());
        tokio::spawn(listen(self.inner.clone(), self.listening.clone()));
    }

    /// Ends the GET stream and deletes the session at the server, if it has one.
    pub(super) async fn close(&self) {
        self.listening.cancel();
        if self.inner.peer.is_stopped() || self.inner.session_id().is_none() {
            return;
        }
        let delete = self
            .inner
            .with_headers(self.inner.client.delete(&self.inner.url));
        let _ = tokio::time::timeout(NOTE_TIMEOUT, delete.send()).await;
    }
}

impl Inner {
    fn session_id(&self) -> Option<String> {
        self.session_id.lock().unwrap().clone()
    }

    fn is_ready(&self) -> bool {
        self.version.lock().unwrap().is_some()
    }

    fn with_headers(&self, mut request: RequestBuilder) -> RequestBuilder {
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(id) = self.session_id() {
            request = request.header(SESSION_HEADER, id);
        }
        if let Some(version) = self.version.lock().unwrap().clone() {
            request = request.header(VERSION_HEADER, version);
        }
        request
    }

    fn post(&self, message: &Value) -> RequestBuilder {
        let request = self
            .client
            .post(&self.url)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .body(message.to_string());
        self.with_headers(request)
    }

    /// POSTs `message` and gives the peer every message of the response, up to the answer to
    /// `awaiting`.
    async fn send(&self, message: &Value, awaiting: Option<u64>) -> Result<(), McpError> {
        let response = self
            .post(message)
            .send()
            .await
            .map_err(|e| self.failed(e))?;
        if let Some(id) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            self.session_id
                .lock()
                .unwrap()
                .get_or_insert_with(|| id.to_string());
        }
        let status = response.status();
        // A session the server no longer knows (MCP: 404) is over.
        if status == StatusCode::NOT_FOUND && self.is_ready() && self.session_id().is_some() {
            self.peer.stop();
            return Err(McpError::Stopped);
        }
        if !status.is_success() {
            return Err(McpError::Protocol(format!(
                "the server answered HTTP {status}"
            )));
        }
        if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
            return Ok(());
        }
        let kind = content_type(&response);
        if kind.starts_with("text/event-stream") {
            self.read_stream(response, awaiting).await
        } else if kind.starts_with("application/json") {
            let body = response.bytes().await.map_err(|e| self.failed(e))?;
            if body.trim_ascii().is_empty() {
                return Ok(());
            }
            let messages = match serde_json::from_slice(&body) {
                Ok(Value::Array(messages)) => messages,
                Ok(message) => vec![message],
                Err(e) => {
                    return Err(McpError::Protocol(format!(
                        "the server's answer is not JSON: {e}"
                    )))
                }
            };
            for message in messages {
                self.receive(message).await;
            }
            Ok(())
        } else if awaiting.is_none() {
            Ok(())
        } else {
            Err(McpError::Protocol(format!(
                "the server answered with {kind:?}, not JSON or an event stream"
            )))
        }
    }

    /// Reads an event stream until it ends or brings the answer to `awaiting`.
    async fn read_stream(
        &self,
        mut response: Response,
        awaiting: Option<u64>,
    ) -> Result<(), McpError> {
        let mut events = SseParser::default();
        loop {
            let Some(chunk) = response.chunk().await.map_err(|e| self.failed(e))? else {
                return Ok(());
            };
            for data in events.push(&chunk) {
                let Ok(message) = serde_json::from_str::<Value>(&data) else {
                    tracing::debug!(server = %self.peer.name(), "an event that is not JSON");
                    continue;
                };
                let done = awaiting.is_some_and(|id| rpc::is_answer_to(&message, id));
                self.receive(message).await;
                if done {
                    return Ok(());
                }
            }
        }
    }

    /// Gives `message` to the peer and POSTs its reply, if any.
    async fn receive(&self, message: Value) {
        let Some(reply) = self.peer.receive(message) else {
            return;
        };
        if let Err(e) = self.post(&reply).send().await {
            tracing::debug!(server = %self.peer.name(), "replying to the server failed: {}", describe(e));
        }
    }

    /// What a failed request means: before `initialize` was answered the server cannot be
    /// reached; after, a refused connection means that it stopped; anything else broke this
    /// request only.
    fn failed(&self, e: reqwest::Error) -> McpError {
        let connect = e.is_connect();
        let text = describe(e);
        if !self.is_ready() {
            McpError::Start(format!("could not reach the server: {text}"))
        } else if connect {
            self.peer.stop();
            McpError::Stopped
        } else {
            McpError::Protocol(format!("the connection failed: {text}"))
        }
    }
}

/// Listens on the GET stream until `stop` fires. A server without one (405, or anything that
/// is not an event stream) is left alone; a stream the server ends is opened again.
async fn listen(inner: Arc<Inner>, stop: CancellationToken) {
    loop {
        let request = inner.with_headers(
            inner
                .client
                .get(&inner.url)
                .header(ACCEPT, "text/event-stream"),
        );
        let response = tokio::select! {
            _ = stop.cancelled() => return,
            response = request.send() => response,
        };
        let response = match response {
            Ok(r)
                if r.status().is_success() && content_type(&r).starts_with("text/event-stream") =>
            {
                r
            }
            Ok(r) => {
                tracing::debug!(server = %inner.peer.name(), "no GET stream: HTTP {}", r.status());
                return;
            }
            Err(e) => {
                tracing::debug!(server = %inner.peer.name(), "no GET stream: {}", describe(e));
                return;
            }
        };
        let ended = tokio::select! {
            _ = stop.cancelled() => return,
            ended = inner.read_stream(response, None) => ended,
        };
        if ended.is_err() || inner.peer.is_stopped() {
            return;
        }
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(RELISTEN) => {}
        }
    }
}

fn content_type(response: &Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The error and its causes, without the URL.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Cuts an event stream into the `data` of its events. `event`, `id`, `retry` and comments
/// are not needed.
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
    data: Option<String>,
}

impl SseParser {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                events.extend(self.data.take());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                match &mut self.data {
                    Some(data) => {
                        data.push('\n');
                        data.push_str(rest);
                    }
                    None => self.data = Some(rest.to_string()),
                }
            }
        }
        events
    }
}
```

**Create `crates/dots-core/src/mcp/mod.rs`:**

```rust
//! The MCP client (MCP/skills spec section 5): a session with one Library MCP server, over stdio
//! or Streamable HTTP. JSON-RPC 2.0 written by hand with `serde_json`, as the server side
//! (`server/mcp.rs`) is.
//!
//! A run opens one `McpSession` per server it uses (`open`: start or connect, `initialize`,
//! `notifications/initialized`), lists the tools (`list_tools`, every page), calls them
//! (`call_tool`, with a time limit and the run's cancel token), watches `list_changed` for the
//! server's `notifications/tools/list_changed`, and closes the session when it ends (`close`,
//! which stops a stdio server). A session is not restarted: once its server has stopped, every
//! request fails at once with `McpError::Stopped`.

mod http;
mod rpc;
mod stdio;
mod tools;
mod vars;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::library::McpServerConfig;
use crate::Error;

pub use stdio::resolve_program;
pub use tools::{fingerprint, McpTool};

/// The protocol version Dots asks for.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// The versions Dots accepts when a server answers with another one.
pub const KNOWN_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// The limit for `initialize` and for `tools/list` with all its pages.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
/// The limit for a `tools/call` of the local engine.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a stdio server has to exit by itself once its input is closed, before it is killed.
pub const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// The limit for a notification and for the closing `DELETE` of an http session.
const NOTE_TIMEOUT: Duration = Duration::from_secs(5);
/// `tools/list` pages read at most: a server that pages forever is not an MCP server.
const MAX_PAGES: usize = 100;

/// How a session starts and ends.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// The folder a stdio server starts in when its config names none. `None`: Dots' own.
    pub cwd: Option<PathBuf>,
    /// The limit for `initialize` and `tools/list`.
    pub answer_timeout: Duration,
    /// How long `close` waits for a stdio server to exit by itself.
    pub close_grace: Duration,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            cwd: None,
            answer_timeout: ANSWER_TIMEOUT,
            close_grace: CLOSE_GRACE,
        }
    }
}

/// Why a request to an MCP server failed. The text never names the server (the caller adds
/// it: "salesforce: could not start: SF_TOKEN is not set") and never holds an expanded
/// `${VAR}` value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    /// The server could not be started or reached: a variable not set, a command not found, a
    /// process that exited before it answered, a refused connection.
    #[error("could not start: {0}")]
    Start(String),
    /// No answer within the time limit.
    #[error("did not answer")]
    Timeout,
    /// The server is gone: its process exited, or its http endpoint stopped taking requests.
    #[error("the server stopped")]
    Stopped,
    /// The caller's cancel token fired first.
    #[error("cancelled")]
    Cancelled,
    /// The server answered with a JSON-RPC error.
    #[error("{message} (error {code})")]
    Server { code: i64, message: String },
    /// The answer is not what MCP says: an HTTP error status, no JSON, an unknown protocol
    /// version, a broken connection.
    #[error("{0}")]
    Protocol(String),
}

impl From<McpError> for Error {
    fn from(e: McpError) -> Self {
        Error::Other(e.to_string())
    }
}

enum Transport {
    Stdio(stdio::Stdio),
    Http(http::Http),
}

impl Transport {
    /// Sends one message. Over http this is the whole exchange: what the server sends back
    /// (an answer to `awaiting`, notifications, requests) goes to the peer before it returns.
    async fn send(&self, message: &Value, awaiting: Option<u64>) -> Result<(), McpError> {
        match self {
            Self::Stdio(t) => t.send(message),
            Self::Http(t) => t.send(message, awaiting).await,
        }
    }

    async fn close(&self) {
        match self {
            Self::Stdio(t) => t.close().await,
            Self::Http(t) => t.close().await,
        }
    }
}

/// An open session with one MCP server. Its methods take `&self`, so a run can share it (in an
/// `Arc`) between the engine and the gateway.
pub struct McpSession {
    name: String,
    peer: Arc<rpc::Peer>,
    transport: Transport,
    protocol_version: String,
    server_info: Value,
    options: SessionOptions,
}

impl McpSession {
    /// Starts or connects to the server `name` with `config`, its `${VAR}` read through `env`,
    /// and initializes the session. A server that does not start, answer or speak a known
    /// protocol version is stopped again before the error is returned.
    pub async fn open(
        name: &str,
        config: &McpServerConfig,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
        options: SessionOptions,
    ) -> Result<McpSession, McpError> {
        let config = config.expanded(env).map_err(|e| {
            McpError::Start(match e {
                Error::Invalid(message) => message,
                other => other.to_string(),
            })
        })?;
        let peer = Arc::new(rpc::Peer::new(name));
        let transport = match config {
            McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
            } => Transport::Stdio(stdio::Stdio::start(
                peer.clone(),
                stdio::Launch {
                    command: &command,
                    args: &args,
                    cwd: cwd.map(PathBuf::from).or_else(|| options.cwd.clone()),
                    env: &env,
                    grace: options.close_grace,
                },
            )?),
            McpServerConfig::Http { url, headers } => {
                Transport::Http(http::Http::new(peer.clone(), url, headers))
            }
        };
        let mut session = McpSession {
            name: name.to_string(),
            peer,
            transport,
            protocol_version: String::new(),
            server_info: Value::Null,
            options,
        };
        match session.initialize().await {
            Ok(()) => Ok(session),
            Err(e) => {
                session.close().await;
                Err(e)
            }
        }
    }

    async fn initialize(&mut self) -> Result<(), McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "dots", "version": env!("CARGO_PKG_VERSION") }
        });
        let limit = self.options.answer_timeout;
        let result = match self.request("initialize", params, limit, None).await {
            Ok(result) => result,
            Err(McpError::Stopped) => {
                return Err(McpError::Start(
                    "the server exited before it answered".into(),
                ))
            }
            Err(e) => return Err(e),
        };
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !KNOWN_PROTOCOL_VERSIONS.contains(&version) {
            return Err(McpError::Protocol(format!(
                "the server speaks MCP version {version:?}, which Dots does not know"
            )));
        }
        self.protocol_version = version.to_string();
        self.server_info = result.get("serverInfo").cloned().unwrap_or(Value::Null);
        if let Transport::Http(http) = &self.transport {
            http.ready(version);
        }
        self.notify("notifications/initialized", None).await
    }

    /// The server's name as the Library has it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The protocol version agreed at `initialize`.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// The server's `serverInfo` (`name`, `version`), as it sent it.
    pub fn server_info(&self) -> &Value {
        &self.server_info
    }

    /// The tools, every page (`nextCursor`), within the answer time limit for all pages.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let deadline = tokio::time::Instant::now() + self.options.answer_timeout;
        let mut tools = Vec::new();
        let mut cursors = HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let page = self.request("tools/list", params, left, None).await?;
            let list = page
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::Protocol("tools/list gave no tools".into()))?;
            for tool in list {
                tools.push(serde_json::from_value(tool.clone()).map_err(|e| {
                    McpError::Protocol(format!("tools/list gave a tool Dots cannot read: {e}"))
                })?);
            }
            match page.get("nextCursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => {
                    if !cursors.insert(next.to_string()) {
                        return Err(McpError::Protocol("tools/list repeats a page".into()));
                    }
                    cursor = Some(next.to_string());
                }
                _ => return Ok(tools),
            }
        }
        Err(McpError::Protocol(format!(
            "tools/list has more than {MAX_PAGES} pages"
        )))
    }

    /// Calls `tool` and returns the `tools/call` result as the server sent it (`content`,
    /// `isError`, ...). A result with `isError: true` is a result, not an error; a JSON-RPC
    /// error is `McpError::Server`. Fails with `Timeout` after `limit` and with `Cancelled` as
    /// soon as `cancel` fires; the server is told with `notifications/cancelled`.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Value,
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        let params = json!({ "name": tool, "arguments": arguments });
        let result = self
            .request("tools/call", params, limit, Some(cancel))
            .await?;
        if !result.is_object() {
            return Err(McpError::Protocol(
                "tools/call gave a result that is not an object".into(),
            ));
        }
        Ok(result)
    }

    /// Counts the server's `notifications/tools/list_changed`: `changed()` wakes on each one.
    /// The session does not list again by itself; the caller does.
    pub fn list_changed(&self) -> watch::Receiver<u64> {
        self.peer.list_changed()
    }

    /// The server has stopped, or the session was closed.
    pub fn is_stopped(&self) -> bool {
        self.peer.is_stopped()
    }

    /// Ends the session: an http session is deleted at the server; a stdio server's input is
    /// closed, and it is killed with everything it started if it has not exited after the
    /// grace time. Returns once it is gone. Closing twice does nothing more.
    pub async fn close(&self) {
        self.transport.close().await;
        self.peer.stop();
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let message = rpc::notification(method, params);
        match tokio::time::timeout(NOTE_TIMEOUT, self.transport.send(&message, None)).await {
            Ok(sent) => sent,
            Err(_) => Err(McpError::Timeout),
        }
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        limit: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, McpError> {
        if self.peer.is_stopped() {
            return Err(McpError::Stopped);
        }
        let (id, answer) = self.peer.expect();
        let message = rpc::request(id, method, params);
        let exchange = async {
            self.transport.send(&message, Some(id)).await?;
            answer.await.unwrap_or(Err(McpError::Stopped))
        };
        let never = CancellationToken::new();
        let cancel = cancel.unwrap_or(&never);
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(McpError::Cancelled),
            r = tokio::time::timeout(limit, exchange) => r.unwrap_or(Err(McpError::Timeout)),
        };
        if outcome.is_err() {
            self.peer.forget(id);
        }
        // `initialize` is never cancelled (MCP); a server that did not answer it is stopped.
        if matches!(outcome, Err(McpError::Timeout | McpError::Cancelled)) && method != "initialize"
        {
            let reason = match outcome {
                Err(McpError::Timeout) => "timed out",
                _ => "cancelled",
            };
            let params = json!({ "requestId": id, "reason": reason });
            if let Err(e) = self.notify("notifications/cancelled", Some(params)).await {
                tracing::debug!(server = %self.name, "telling the server of a cancel failed: {e}");
            }
        }
        outcome
    }
}
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod events;
pub mod library;
pub mod model;
pub mod policy;
```

**with:**

```rust
pub mod events;
pub mod library;
pub mod mcp;
pub mod model;
pub mod policy;
```

**Create `crates/dots-core/src/bin/fake-mcp.rs`:**

```rust
//! Test MCP server for dots-core tests: `fake-mcp <script.json> [--http] [args...]`.
//!
//! Over stdio (the default) it reads one JSON-RPC message per line on stdin. With `--http` it
//! listens on 127.0.0.1 (Streamable HTTP at `/mcp`) and prints its URL as the first line of
//! stdout. Other arguments are only recorded.
//!
//! The script is a JSON object:
//! - `tools`: the tools it lists. Besides the tool's own fields, a tool can have `result` (the
//!   `tools/call` result; default: one text part `<name>: <arguments>`), `error` (`{code,
//!   message}`: a JSON-RPC error instead), `delay_ms`, `exit` (the process exits without an
//!   answer), `switch` (the list becomes `after_switch`, and `notifications/tools/list_changed`
//!   is sent before the answer and on GET streams) and `ask_client` (a request with this method
//!   and the id `s-<method>` is sent to the client before the answer).
//! - `after_switch`: the tools after a `switch`.
//! - `page_size`: tools per `tools/list` page (default: all in one).
//! - `protocol_version`: the version `initialize` answers (default: the client's).
//! - `init_delay_ms`, `exit_at_initialize`: a slow or dying `initialize`.
//! - `log`: a file that gets one JSON line per record: `{"start": {pid, args, cwd, env}}` (env:
//!   the variables whose names start with `FAKE_MCP_`), `{"msg": <message received>}` (over
//!   http with `"http": "POST"` and the `session`, `protocol` and `auth` headers), and
//!   `{"http": "GET" | "DELETE", "session": ...}`.
//! - `alive`: a file locked for as long as the process runs.
//! - `stderr_bytes`: written to stderr at start, before anything is read.
//! - `ignore_eof`: over stdio, keep running for 60 s after stdin ends instead of exiting.
//! - `sse`: over http, answer every POST with an event stream (default: JSON, unless there are
//!   messages before the answer).
//! - `require_header`: over http, `{name: value}` every POST must carry (401 otherwise).
//! - `get_stream`: over http, a GET opens an event stream that sends `list_changed` at the
//!   next `switch` and then ends (default: 405).

use std::collections::HashSet;
use std::fs::File;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Notify;

/// Tool fields that script a call; they are not listed.
const CONTROL: [&str; 6] = [
    "result",
    "error",
    "delay_ms",
    "exit",
    "switch",
    "ask_client",
];

struct Fake {
    script: Value,
    tools: Mutex<Vec<Value>>,
    log: Option<Mutex<File>>,
    switched: Notify,
    sessions: Mutex<HashSet<String>>,
    next_session: AtomicU64,
}

/// What the server sends for one request: messages first, then the answer.
struct Reply {
    before: Vec<Value>,
    answer: Value,
}

fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn list_changed() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
}

fn listed(tool: &Value) -> Value {
    let mut tool = tool.as_object().cloned().unwrap_or_default();
    for key in CONTROL {
        tool.remove(key);
    }
    Value::Object(tool)
}

impl Fake {
    fn record(&self, record: Value) {
        if let Some(log) = &self.log {
            let mut file = log.lock().unwrap();
            writeln!(file, "{record}").unwrap();
            file.flush().unwrap();
        }
    }

    fn flag(&self, name: &str) -> bool {
        self.script[name] == true
    }

    async fn sleep(ms: Option<u64>) {
        if let Some(ms) = ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }

    /// The reply to a request; `None` for a notification or an answer from the client.
    async fn handle(&self, msg: &Value) -> Option<Reply> {
        let id = msg.get("id")?;
        let method = msg.get("method").and_then(Value::as_str)?;
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let mut before = Vec::new();
        let answer = match method {
            "initialize" => {
                Self::sleep(self.script["init_delay_ms"].as_u64()).await;
                if self.flag("exit_at_initialize") {
                    std::process::exit(3);
                }
                let version = self
                    .script
                    .get("protocol_version")
                    .cloned()
                    .unwrap_or_else(|| params["protocolVersion"].clone());
                result(
                    id,
                    json!({"protocolVersion": version,
                           "capabilities": {"tools": {"listChanged": true}},
                           "serverInfo": {"name": "fake-mcp", "version": "0"}}),
                )
            }
            "ping" => result(id, json!({})),
            "tools/list" => {
                let tools: Vec<Value> = self.tools.lock().unwrap().iter().map(listed).collect();
                let start: usize = params["cursor"]
                    .as_str()
                    .map(|c| c.parse().unwrap())
                    .unwrap_or(0);
                let size = match self.script["page_size"].as_u64() {
                    Some(n) if n > 0 => n as usize,
                    _ => tools.len().max(1),
                };
                let end = (start + size).min(tools.len());
                let mut page = json!({"tools": tools[start.min(end)..end]});
                if end < tools.len() {
                    page["nextCursor"] = json!(end.to_string());
                }
                result(id, page)
            }
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let tool = self
                    .tools
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name)
                    .cloned();
                let Some(tool) = tool else {
                    return Some(Reply {
                        before,
                        answer: error(id, -32602, &format!("unknown tool: {name}")),
                    });
                };
                Self::sleep(tool["delay_ms"].as_u64()).await;
                if tool["exit"] == true {
                    std::process::exit(3);
                }
                if let Some(asked) = tool["ask_client"].as_str() {
                    before.push(json!({"jsonrpc": "2.0", "id": format!("s-{asked}"),
                                       "method": asked, "params": {}}));
                }
                if tool["switch"] == true {
                    let after = self.script["after_switch"].as_array().cloned();
                    *self.tools.lock().unwrap() = after.unwrap_or_default();
                    before.push(list_changed());
                    self.switched.notify_waiters();
                }
                if let Some(e) = tool.get("error") {
                    error(
                        id,
                        e["code"].as_i64().unwrap_or(-32000),
                        e["message"].as_str().unwrap_or("error"),
                    )
                } else {
                    let default = json!({"content": [{"type": "text",
                        "text": format!("{name}: {}", params["arguments"])}]});
                    result(id, tool.get("result").cloned().unwrap_or(default))
                }
            }
            other => error(id, -32601, &format!("method not found: {other}")),
        };
        Some(Reply { before, answer })
    }
}

async fn stdio(fake: Arc<Fake>) {
    let out = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        fake.record(json!({"msg": msg}));
        let (fake, out) = (fake.clone(), out.clone());
        // Each request on its own: a slow call does not hold up a cancel or another call.
        tokio::spawn(async move {
            if let Some(reply) = fake.handle(&msg).await {
                let mut out = out.lock().await;
                for m in reply.before.iter().chain([&reply.answer]) {
                    out.write_all(format!("{m}\n").as_bytes()).await.unwrap();
                }
                out.flush().await.unwrap();
            }
        });
    }
    if fake.flag("ignore_eof") {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn header(headers: &HeaderMap, name: &str) -> Value {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map_or(Value::Null, |v| json!(v))
}

fn event_stream(messages: &[&Value]) -> Response {
    let body: String = messages
        .iter()
        .map(|m| format!("event: message\ndata: {m}\n\n"))
        .collect();
    ([(CONTENT_TYPE, "text/event-stream")], body).into_response()
}

/// `Ok(())` when the request names a session this server gave.
fn check_session(fake: &Fake, headers: &HeaderMap) -> Result<(), StatusCode> {
    match header(headers, "mcp-session-id").as_str() {
        None => Err(StatusCode::BAD_REQUEST),
        Some(id) if !fake.sessions.lock().unwrap().contains(id) => Err(StatusCode::NOT_FOUND),
        Some(_) => Ok(()),
    }
}

async fn on_post(State(fake): State<Arc<Fake>>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    fake.record(json!({"http": "POST", "msg": msg,
        "session": header(&headers, "mcp-session-id"),
        "protocol": header(&headers, "mcp-protocol-version"),
        "auth": header(&headers, "authorization")}));
    if let Some(required) = fake.script["require_header"].as_object() {
        for (name, value) in required {
            if header(&headers, name) != *value {
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
    }
    let initialize = msg["method"] == "initialize";
    if !initialize {
        if let Err(status) = check_session(&fake, &headers) {
            return status.into_response();
        }
    }
    let Some(reply) = fake.handle(&msg).await else {
        return StatusCode::ACCEPTED.into_response();
    };
    let mut response = if fake.flag("sse") || !reply.before.is_empty() {
        let all: Vec<&Value> = reply.before.iter().chain([&reply.answer]).collect();
        event_stream(&all)
    } else {
        Json(reply.answer).into_response()
    };
    if initialize {
        let n = fake.next_session.fetch_add(1, Ordering::Relaxed);
        let id = format!("fake-session-{n}");
        fake.sessions.lock().unwrap().insert(id.clone());
        response
            .headers_mut()
            .insert("mcp-session-id", id.parse().unwrap());
    }
    response
}

async fn on_get(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    if !fake.flag("get_stream") {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if let Err(status) = check_session(&fake, &headers) {
        return status.into_response();
    }
    // Waiting before the record: a switch the test makes once it sees the GET is not missed.
    let switched = fake.switched.notified();
    tokio::pin!(switched);
    switched.as_mut().enable();
    fake.record(json!({"http": "GET", "session": header(&headers, "mcp-session-id")}));
    switched.await;
    event_stream(&[&list_changed()])
}

async fn on_delete(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> StatusCode {
    let session = header(&headers, "mcp-session-id");
    fake.record(json!({"http": "DELETE", "session": session}));
    if let Some(id) = session.as_str() {
        fake.sessions.lock().unwrap().remove(id);
    }
    StatusCode::OK
}

async fn http(fake: Arc<Fake>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!("http://{}/mcp", listener.local_addr().unwrap());
    std::io::stdout().flush().unwrap();
    let app = Router::new()
        .route("/mcp", post(on_post).get(on_get).delete(on_delete))
        .with_state(fake);
    axum::serve(listener, app).await.unwrap();
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let script: Value = serde_json::from_str(&std::fs::read_to_string(&args[0]).unwrap()).unwrap();
    let _alive = script["alive"].as_str().map(|path| {
        let file = File::create(path).unwrap();
        file.lock().unwrap();
        file
    });
    let log = script["log"].as_str().map(|path| {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        Mutex::new(file)
    });
    let fake = Arc::new(Fake {
        tools: Mutex::new(script["tools"].as_array().cloned().unwrap_or_default()),
        script,
        log,
        switched: Notify::new(),
        sessions: Mutex::new(HashSet::new()),
        next_session: AtomicU64::new(1),
    });
    let env: Map<String, Value> = std::env::vars()
        .filter(|(k, _)| k.starts_with("FAKE_MCP_"))
        .map(|(k, v)| (k, json!(v)))
        .collect();
    fake.record(json!({"start": {
        "pid": std::process::id(),
        "args": args,
        "cwd": std::env::current_dir().unwrap(),
        "env": env,
    }}));
    if let Some(n) = fake.script["stderr_bytes"].as_u64() {
        let line = format!("{}\n", "x".repeat(99));
        let mut stderr = std::io::stderr();
        for _ in 0..n.div_ceil(100) {
            stderr.write_all(line.as_bytes()).unwrap();
        }
    }
    if args.iter().any(|a| a == "--http") {
        http(fake).await;
    } else {
        stdio(fake).await;
    }
}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
                             complete), sleeping on a clock that tests move by hand
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
```

**with:**

```markdown
                             complete), sleeping on a clock that tests move by hand
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
    src/mcp/                 MCP client: one session per Library MCP server for a run (a stdio
                             child process in a Job Object, or Streamable HTTP), `${VAR}`
                             expansion, tool-list fingerprints (MCP/skills spec §5)
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    src/policy/              allow/ask/deny resolution, one-shot grants
    src/workspace/           git worktree or plain folder per run
  crates/fake-claude/        test binary that mimics `claude -p` stream-json output
  apps/dots-app/             Tauri 2 shell: starts runtime, tray, autostart, commands, events
```

**with:**

```markdown
    src/policy/              allow/ask/deny resolution, one-shot grants
    src/workspace/           git worktree or plain folder per run
    src/bin/fake-mcp.rs      test MCP server (stdio and http) scripted by a JSON file
  crates/fake-claude/        test binary that mimics `claude -p` stream-json output
  apps/dots-app/             Tauri 2 shell: starts runtime, tray, autostart, commands, events
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  the MCP `approve` endpoint at scripted points. Drives end-to-end tests of runner + approval
  flow (approve in window, deny, timeout → park → resume) with no network or subscription use.
- **Local engine:** `wiremock` OpenAI-compatible server returning scripted tool calls; verifies
  the loop, policy gate, and park/resume conversation rebuild.
```

**with:**

```markdown
  the MCP `approve` endpoint at scripted points. Drives end-to-end tests of runner + approval
  flow (approve in window, deny, timeout → park → resume) with no network or subscription use.
- **fake-mcp:** a scripted MCP server over stdio or Streamable HTTP (tools, read-only hints,
  results, errors, delays, a list change with `notifications/tools/list_changed`, a call log);
  drives the MCP client's tests with no network.
- **Local engine:** `wiremock` OpenAI-compatible server returning scripted tool calls; verifies
  the loop, policy gate, and park/resume conversation rebuild.
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p dots-core --test library_test --test mcp_client_test --test mcp_test 2>&1 | grep -E "^test result|FAILED"
```

Expected: `library_test` 9 passed, `mcp_client_test` 18, `mcp_test` 5. The client tests start real `fake-mcp` processes and take about 2.5 s.

- [ ] **Step 5: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

Expected: `dots-core` 483 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent. The UI is unchanged (292 in 21 files).

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/bin/fake-mcp.rs crates/dots-core/src/lib.rs crates/dots-core/src/library.rs crates/dots-core/src/mcp/http.rs crates/dots-core/src/mcp/mod.rs crates/dots-core/src/mcp/rpc.rs crates/dots-core/src/mcp/stdio.rs crates/dots-core/src/mcp/tools.rs crates/dots-core/src/mcp/vars.rs crates/dots-core/tests/common/fake_mcp.rs crates/dots-core/tests/common/mod.rs crates/dots-core/tests/library_test.rs crates/dots-core/tests/mcp_client_test.rs crates/dots-core/tests/mcp_test.rs docs/superpowers/specs/2026-10-08-dots-design.md
git commit -m "feat(core): add an MCP client over stdio and Streamable HTTP, with a fake-mcp test server"
```

### Task 4: Read skills and plugins from disk and prepare a run's plugin folders

Skills and plugins are folders in Claude Code's formats (spec section 2), and this task reads them without a YAML dependency: a small frontmatter reader takes top-level `key: value` lines, quoted values and block scalars, ignores lists and maps, and gives no fields at all for a frontmatter Claude Code would not parse. A skill's name and description default as the spec says; a folder can be scanned for the skills under it; a plugin's name, skills, commands, agents, hooks and servers are read, `.mcp.json` with `${CLAUDE_PLUGIN_ROOT}` filled in; the person's installed Claude Code plugins come from `installed_plugins.json`. For a Claude run, `prepare_run_kit` writes the folders of spec section 7 under `<plugins_dir>/run-<run_id>/`: a generated `dots-skills` plugin with the standalone skills, and a copy of each plugin without its servers, its monitors, its hooks unless they are switched on, and every frontmatter field that would let a skill, a command or an agent run a tool or start a server without Dots' check. A copy over 50 MB, or a missing folder, is left out with a reason. Nothing in the runtime calls the module yet. The main spec's module tree follows.

**Files:**
- Create: `crates/dots-core/src/kit/frontmatter.rs`, `crates/dots-core/src/kit/installed.rs`, `crates/dots-core/src/kit/mod.rs`, `crates/dots-core/src/kit/plugin.rs`, `crates/dots-core/src/kit/run_dirs.rs`, `crates/dots-core/src/kit/skill.rs`, `crates/dots-core/tests/kit_run_test.rs`, `crates/dots-core/tests/kit_test.rs`
- Modify: `crates/dots-core/src/lib.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`
- Test: `crates/dots-core/tests/kit_run_test.rs`, `crates/dots-core/tests/kit_test.rs`

**Interfaces:**
- Consumes: Task 2's `LibraryKind`, `McpServerConfig`, `check_name` and `plugin_server_name`.
- Produces, in `dots_core::kit`:
  - `Frontmatter { fields: BTreeMap<String, String>, body: String }`; `parse_frontmatter(text: &str) -> Frontmatter`; `remove_field(text: &str, key: &str) -> String` (works on lines, so it also cleans a frontmatter Dots reads as broken).
  - `LISTING_MAX_CHARS = 1536`; `SkillInfo { name, description: String, when_to_use: Option<String>, dir: PathBuf, fields: BTreeMap<String, String> }` with `listing(&self) -> String` (the description and `when_to_use` on one line, cut at 1,536 characters); `read_skill(dir: &Path) -> Result<SkillInfo>` (`NotFound` "folder not found: …" or "SKILL.md not found in …"); `scan_skills(root: &Path) -> Result<Vec<SkillInfo>>` (the root's own skill, else its subfolders', else those under `root/skills`).
  - `PluginServer { name: String, config: McpServerConfig }`; `PluginInfo { name, root: PathBuf, version, description: Option<String>, skills: Vec<SkillInfo>, single_skill: bool, commands: Vec<String>, agents: Vec<String>, has_hooks, has_monitors, has_bin: bool, servers: Vec<PluginServer>, warnings: Vec<String> }`; `PluginInfo::qualified_skills(&self, library_name: &str) -> Vec<(String, &SkillInfo)>` (`<library name>:<skill>`); `PluginInfo::server_items(&self, plugin: &str) -> Vec<(String, McpServerConfig)>` (named by `plugin_server_name`); `read_plugin(root: &Path) -> Result<PluginInfo>` (a bad manifest or `.mcp.json` is a warning, not an error).
  - `InstalledPlugin { id, name, marketplace: Option<String>, scope: Option<String>, install_path: PathBuf, version: Option<String>, exists: bool }`; `plugins_root(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf>` (`CLAUDE_CODE_PLUGIN_CACHE_DIR`, else `<USERPROFILE or HOME>/.claude/plugins`); `read_installed(root: &Path) -> Vec<InstalledPlugin>`; `installed_plugins(env) -> Vec<InstalledPlugin>`; `find_installed(env, id: &str) -> Option<InstalledPlugin>`. Unknown shapes are tolerated; a missing file gives none.
  - `MAX_COPY_BYTES = 50 * 1024 * 1024`; `KitSkill { name: String, dir: PathBuf }`; `KitPlugin { name: String, dir: PathBuf, run_hooks: bool }`; `LeftOut { kind: LibraryKind, name: String, reason: String }` (reasons: "folder not found", "SKILL.md not found", "larger than 50 MB", "could not be read: …", "could not be copied: …"); `RunKit { plugin_dirs: Vec<PathBuf>, left_out: Vec<LeftOut> }` with `cleanup(&self)` (idempotent).
  - `prepare_run_kit(plugins_dir: &Path, run_id: &str, skills: &[KitSkill], plugins: &[KitPlugin]) -> Result<RunKit>`: blocking (call it from `spawn_blocking`); `dots-skills` first, then the plugins in the given order; the copy's `.claude-plugin/plugin.json` is named after the Library item; nothing is created without a skill or a plugin.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/kit_run_test.rs`:**

```rust
//! The per-run plugin folders (MCP/skills spec section 7).

use std::fs;
use std::path::Path;

use dots_core::kit::{prepare_run_kit, KitPlugin, KitSkill, MAX_COPY_BYTES};
use dots_core::library::LibraryKind;
use serde_json::Value;
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn skill_in(dir: &Path, name: &str) -> KitSkill {
    let path = dir.join("src-skills").join(name);
    write(
        &path.join("SKILL.md"),
        "---\nname: pdf\nallowed-tools: Bash(rm *)\ndescription: d\n---\nbody\n",
    );
    write(&path.join("scripts").join("run.py"), "print(1)\n");
    KitSkill {
        name: name.into(),
        dir: path,
    }
}

fn plugin_in(dir: &Path, name: &str, run_hooks: bool) -> KitPlugin {
    let root = dir.join("src-plugins").join(name);
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"pdf-tools","version":"1","mcpServers":{"x":{"command":"x"}},"hooks":{"PreToolUse":[]},"monitors":[]}"#,
    );
    write(&root.join(".mcp.json"), r#"{"db":{"command":"db"}}"#);
    write(&root.join("hooks").join("hooks.json"), "{}");
    write(&root.join("monitors").join("m.json"), "{}");
    write(
        &root.join("skills").join("fill").join("SKILL.md"),
        "---\nallowed-tools:\n  - Bash\ndescription: fill\n---\nfill\n",
    );
    write(
        &root.join("commands").join("go.md"),
        "---\nallowed-tools: Bash(curl *)\n---\nGo\n",
    );
    write(
        &root.join("commands").join("sub").join("deep.md"),
        "---\nallowed-tools: Read\n---\nDeep\n",
    );
    write(
        &root.join("agents").join("helper.md"),
        "---\ntools: Read\n---\nHelp\n",
    );
    write(&root.join("bin").join("tool.cmd"), "echo\n");
    write(&root.join(".git").join("config"), "x");
    KitPlugin {
        name: name.into(),
        dir: root,
        run_hooks,
    }
}

#[test]
fn nothing_to_include_creates_nothing() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let kit = prepare_run_kit(&plugins, "r1", &[], &[]).unwrap();
    assert!(kit.plugin_dirs.is_empty() && kit.left_out.is_empty());
    assert!(!plugins.exists());
    kit.cleanup();
}

#[test]
fn skills_go_into_a_generated_dots_skills_plugin_without_allowed_tools() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let skill = skill_in(tmp.path(), "pdf");
    let kit = prepare_run_kit(&plugins, "r1", &[skill], &[]).unwrap();
    let dir = plugins.join("run-r1").join("dots-skills");
    assert_eq!(kit.plugin_dirs, std::slice::from_ref(&dir));
    let manifest: Value =
        serde_json::from_str(&read(&dir.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert_eq!(manifest["name"], "dots-skills");
    let copy = dir.join("skills").join("pdf");
    assert_eq!(
        read(&copy.join("SKILL.md")),
        "---\nname: pdf\ndescription: d\n---\nbody\n"
    );
    assert_eq!(read(&copy.join("scripts").join("run.py")), "print(1)\n");
    // The original is untouched.
    assert!(
        read(&tmp.path().join("src-skills").join("pdf").join("SKILL.md")).contains("allowed-tools")
    );
}

#[test]
fn a_plugin_copy_has_no_servers_monitors_or_hooks_and_no_allowed_tools() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "pdf-tools", false);
    let kit = prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let copy = plugins.join("run-r1").join("pdf-tools");
    assert_eq!(kit.plugin_dirs, std::slice::from_ref(&copy));
    assert!(!copy.join(".mcp.json").exists());
    assert!(!copy.join("monitors").exists());
    assert!(!copy.join("hooks").exists());
    assert!(!copy.join(".git").exists());
    assert!(copy.join("bin").join("tool.cmd").exists());
    assert!(copy.join("agents").join("helper.md").exists());
    assert_eq!(
        read(&copy.join("skills").join("fill").join("SKILL.md")),
        "---\ndescription: fill\n---\nfill\n"
    );
    assert_eq!(read(&copy.join("commands").join("go.md")), "---\n---\nGo\n");
    assert_eq!(
        read(&copy.join("commands").join("sub").join("deep.md")),
        "---\n---\nDeep\n"
    );
    let manifest: Value =
        serde_json::from_str(&read(&copy.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert_eq!(manifest["name"], "pdf-tools");
    assert_eq!(manifest["version"], "1");
    for gone in ["mcpServers", "hooks", "monitors"] {
        assert!(manifest.get(gone).is_none(), "{gone}");
    }
}

#[test]
fn run_hooks_keeps_the_hooks_and_nothing_else_changes() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "pdf-tools", true);
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let copy = plugins.join("run-r1").join("pdf-tools");
    assert!(copy.join("hooks").join("hooks.json").exists());
    assert!(!copy.join(".mcp.json").exists());
    assert!(!copy.join("monitors").exists());
    let manifest: Value =
        serde_json::from_str(&read(&copy.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert!(manifest.get("hooks").is_some());
    assert!(manifest.get("mcpServers").is_none() && manifest.get("monitors").is_none());
}

#[test]
fn a_nested_mcp_json_or_hooks_folder_is_not_special() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "p", false);
    write(
        &plugin.dir.join("skills").join("fill").join(".mcp.json"),
        "{}",
    );
    write(
        &plugin
            .dir
            .join("skills")
            .join("fill")
            .join("hooks")
            .join("h.txt"),
        "h",
    );
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let fill = plugins.join("run-r1").join("p").join("skills").join("fill");
    assert!(fill.join(".mcp.json").exists());
    assert!(fill.join("hooks").join("h.txt").exists());
}

#[test]
fn dots_skills_comes_first_and_plugins_keep_their_order() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let kit = prepare_run_kit(
        &plugins,
        "r1",
        &[skill_in(tmp.path(), "pdf")],
        &[
            plugin_in(tmp.path(), "zeta", false),
            plugin_in(tmp.path(), "alpha", false),
        ],
    )
    .unwrap();
    let names: Vec<_> = kit
        .plugin_dirs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["dots-skills", "zeta", "alpha"]);
}

#[test]
fn what_is_missing_is_left_out_with_a_reason_and_the_rest_goes_in() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    fs::create_dir_all(tmp.path().join("no-skill-file")).unwrap();
    let skills = [
        KitSkill {
            name: "gone".into(),
            dir: tmp.path().join("nope"),
        },
        KitSkill {
            name: "bare".into(),
            dir: tmp.path().join("no-skill-file"),
        },
        skill_in(tmp.path(), "pdf"),
    ];
    let plugin_list = [
        KitPlugin {
            name: "lost".into(),
            dir: tmp.path().join("nope"),
            run_hooks: false,
        },
        plugin_in(tmp.path(), "ok", false),
    ];
    let kit = prepare_run_kit(&plugins, "r1", &skills, &plugin_list).unwrap();
    let left: Vec<_> = kit
        .left_out
        .iter()
        .map(|l| (l.kind, l.name.as_str(), l.reason.as_str()))
        .collect();
    assert_eq!(
        left,
        [
            (LibraryKind::Skill, "gone", "folder not found"),
            (LibraryKind::Skill, "bare", "SKILL.md not found"),
            (LibraryKind::Plugin, "lost", "folder not found"),
        ]
    );
    assert_eq!(kit.plugin_dirs.len(), 2);
    assert!(!plugins.join("run-r1").join("lost").exists());
}

#[test]
fn only_skills_that_are_left_out_make_no_dots_skills_plugin() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let skills = [KitSkill {
        name: "gone".into(),
        dir: tmp.path().join("nope"),
    }];
    let kit = prepare_run_kit(&plugins, "r1", &skills, &[]).unwrap();
    assert!(kit.plugin_dirs.is_empty());
    assert!(!plugins.join("run-r1").join("dots-skills").exists());
    kit.cleanup();
    assert!(!plugins.join("run-r1").exists());
}

#[test]
fn a_copy_over_the_cap_is_left_out_whole() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let big = plugin_in(tmp.path(), "big", false);
    let file = fs::File::create(big.dir.join("blob.bin")).unwrap();
    file.set_len(MAX_COPY_BYTES + 1).unwrap();
    drop(file);
    // Excluded parts do not count: the same blob under `monitors/` is fine.
    let fine = plugin_in(tmp.path(), "fine", false);
    let file = fs::File::create(fine.dir.join("monitors").join("blob.bin")).unwrap();
    file.set_len(MAX_COPY_BYTES + 1).unwrap();
    drop(file);
    let kit = prepare_run_kit(&plugins, "r1", &[], &[big, fine]).unwrap();
    assert_eq!(kit.left_out.len(), 1);
    assert_eq!(kit.left_out[0].name, "big");
    assert_eq!(kit.left_out[0].reason, "larger than 50 MB");
    assert!(!plugins.join("run-r1").join("big").exists());
    assert_eq!(kit.plugin_dirs.len(), 1);
}

#[test]
fn cleanup_deletes_the_run_folder_and_only_that() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let other = prepare_run_kit(&plugins, "r2", &[skill_in(tmp.path(), "a")], &[]).unwrap();
    let kit = prepare_run_kit(&plugins, "r1", &[skill_in(tmp.path(), "b")], &[]).unwrap();
    kit.cleanup();
    kit.cleanup();
    assert!(!plugins.join("run-r1").exists());
    assert!(plugins.join("run-r2").exists());
    other.cleanup();
}

#[test]
fn an_old_folder_of_the_same_run_is_replaced() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    write(&plugins.join("run-r1").join("stale").join("x"), "x");
    prepare_run_kit(&plugins, "r1", &[skill_in(tmp.path(), "a")], &[]).unwrap();
    assert!(!plugins.join("run-r1").join("stale").exists());
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_not_followed() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "p", false);
    let secret = tmp.path().join("secret.txt");
    write(&secret, "secret");
    std::os::unix::fs::symlink(&secret, plugin.dir.join("link.txt")).unwrap();
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    assert!(!plugins.join("run-r1").join("p").join("link.txt").exists());
}

fn hooky_plugin(dir: &Path, name: &str, run_hooks: bool) -> KitPlugin {
    let plugin = plugin_in(dir, name, run_hooks);
    let hooks = "hooks:\n  PreToolUse:\n    - command: evil\n";
    write(
        &plugin.dir.join("skills").join("fill").join("SKILL.md"),
        &format!("---\ndescription: fill\n{hooks}---\nfill\n"),
    );
    write(
        &plugin.dir.join("commands").join("go.md"),
        &format!("---\ndescription: go\n{hooks}---\nGo\n"),
    );
    write(
        &plugin.dir.join("agents").join("helper.md"),
        &format!(
            "---\nname: helper\ntools: Read\npermissionMode: bypassPermissions\nmcpServers:\n  x:\n    command: x\n{hooks}---\nHelp\n"
        ),
    );
    plugin
}

#[test]
fn frontmatter_hooks_are_removed_from_a_plugin_unless_it_runs_hooks() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    prepare_run_kit(
        &plugins,
        "off",
        &[],
        &[hooky_plugin(tmp.path(), "p", false)],
    )
    .unwrap();
    prepare_run_kit(&plugins, "on", &[], &[hooky_plugin(tmp.path(), "p", true)]).unwrap();
    let files = [
        ["skills", "fill", "SKILL.md"],
        ["commands", "go.md", ""],
        ["agents", "helper.md", ""],
    ];
    for file in files {
        let rel: std::path::PathBuf = file.iter().filter(|p| !p.is_empty()).collect();
        let off = read(&plugins.join("run-off").join("p").join(&rel));
        let on = read(&plugins.join("run-on").join("p").join(&rel));
        assert!(
            !off.contains("hooks:") && !off.contains("evil"),
            "{rel:?}: {off}"
        );
        assert!(
            on.contains("hooks:") && on.contains("evil"),
            "{rel:?}: {on}"
        );
    }
}

#[test]
fn agents_lose_permission_mode_and_mcp_servers_even_with_hooks_on() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    prepare_run_kit(&plugins, "on", &[], &[hooky_plugin(tmp.path(), "p", true)]).unwrap();
    let agent = read(
        &plugins
            .join("run-on")
            .join("p")
            .join("agents")
            .join("helper.md"),
    );
    assert!(
        !agent.contains("permissionMode") && !agent.contains("mcpServers"),
        "{agent}"
    );
    assert!(
        agent.contains("name: helper") && agent.contains("tools: Read"),
        "{agent}"
    );
    assert!(agent.ends_with("---\nHelp\n"));
}

#[test]
fn a_standalone_skill_never_keeps_hooks() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let skill = skill_in(tmp.path(), "pdf");
    write(
        &skill.dir.join("SKILL.md"),
        "---\ndescription: d\nhooks:\n  Stop:\n    - command: evil\n---\nbody\n",
    );
    prepare_run_kit(&plugins, "r1", &[skill], &[]).unwrap();
    let copy = read(
        &plugins
            .join("run-r1")
            .join("dots-skills")
            .join("skills")
            .join("pdf")
            .join("SKILL.md"),
    );
    assert_eq!(copy, "---\ndescription: d\n---\nbody\n");
}

#[test]
fn the_copied_manifest_is_named_after_the_library_item() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    // The manifest says pdf-tools; the Library item is called docs.
    let named = plugin_in(tmp.path(), "docs", false);
    // No manifest at all.
    let bare = KitPlugin {
        name: "bare".into(),
        dir: tmp.path().join("src-plugins").join("bare-src"),
        run_hooks: false,
    };
    write(&bare.dir.join("skills").join("s").join("SKILL.md"), "s\n");
    // A manifest that is not JSON.
    let broken = KitPlugin {
        name: "broken".into(),
        dir: tmp.path().join("src-plugins").join("broken-src"),
        run_hooks: false,
    };
    write(
        &broken.dir.join(".claude-plugin").join("plugin.json"),
        "{nope",
    );
    prepare_run_kit(&plugins, "r1", &[], &[named, bare, broken]).unwrap();
    for name in ["docs", "bare", "broken"] {
        let path = plugins
            .join("run-r1")
            .join(name)
            .join(".claude-plugin")
            .join("plugin.json");
        let manifest: Value = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(manifest["name"], name);
    }
    let docs = plugins
        .join("run-r1")
        .join("docs")
        .join(".claude-plugin")
        .join("plugin.json");
    assert_eq!(
        serde_json::from_str::<Value>(&read(&docs)).unwrap()["version"],
        "1"
    );
}
```

**Create `crates/dots-core/tests/kit_test.rs`:**

```rust
//! Skills and plugins on disk: the frontmatter reader, skill and plugin reading, the installed
//! plugins (MCP/skills spec sections 2 and 7).

use std::fs;
use std::path::Path;

use dots_core::kit::{
    find_installed, installed_plugins, parse_frontmatter, plugins_root, read_installed,
    read_plugin, read_skill, remove_field, scan_skills, LISTING_MAX_CHARS,
};
use dots_core::library::McpServerConfig;
use dots_core::Error;
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn field(text: &str, key: &str) -> Option<String> {
    parse_frontmatter(text).fields.get(key).cloned()
}

// ---- frontmatter ----

#[test]
fn plain_and_quoted_values_are_read() {
    let text = "---\nname: pdf\ndescription: \"Extract: text, \\\"tables\\\"\"\nwhen_to_use: 'it''s for PDFs'\nmodel: sonnet # a comment\n---\n# Body\n";
    let fm = parse_frontmatter(text);
    assert_eq!(fm.fields["name"], "pdf");
    assert_eq!(fm.fields["description"], "Extract: text, \"tables\"");
    assert_eq!(fm.fields["when_to_use"], "it's for PDFs");
    assert_eq!(fm.fields["model"], "sonnet");
    assert_eq!(fm.body, "# Body");
}

#[test]
fn block_scalars_fold_or_keep_their_lines() {
    let text = "---\ndescription: >\n  First line\n  second line\n\n  new paragraph\nwhen_to_use: |-\n  keep\n  lines\nname: x\n---\n";
    assert_eq!(
        field(text, "description").unwrap(),
        "First line second line\nnew paragraph"
    );
    assert_eq!(field(text, "when_to_use").unwrap(), "keep\nlines");
    assert_eq!(field(text, "name").unwrap(), "x");
}

#[test]
fn a_plain_value_goes_on_over_indented_lines() {
    let text = "---\ndescription: one\n  two\n  three\nname: x\n---\n";
    assert_eq!(field(text, "description").unwrap(), "one two three");
}

#[test]
fn lists_and_maps_are_ignored_and_the_rest_is_kept() {
    let text = "---\nname: pdf\nallowed-tools:\n  - Bash(git *)\n  - Read\ntools: [a, b,\n  c]\nmeta:\n  nested: yes\nlist2:\n- a\n- b\ndescription: after\n---\n";
    let fm = parse_frontmatter(text);
    assert_eq!(fm.fields.len(), 2, "{:?}", fm.fields);
    assert_eq!(fm.fields["name"], "pdf");
    assert_eq!(fm.fields["description"], "after");
}

#[test]
fn allowed_tools_as_a_string_is_a_field() {
    let text = "---\nallowed-tools: Bash(git *) Read\n---\n";
    assert_eq!(field(text, "allowed-tools").unwrap(), "Bash(git *) Read");
}

#[test]
fn a_broken_frontmatter_gives_no_fields() {
    for text in [
        // `: ` inside a plain value
        "---\nname: pdf\ndescription: Use when: you read a PDF\n---\nbody\n",
        // the same key twice
        "---\nname: a\nname: b\n---\n",
        // an unclosed quote
        "---\nname: \"pdf\ndescription: x\n---\n",
        // a line that is no key
        "---\nname: pdf\njust text\n---\n",
        // a list at the top
        "---\n- a\n- b\n---\n",
    ] {
        let fm = parse_frontmatter(text);
        assert!(fm.fields.is_empty(), "{text:?} gave {:?}", fm.fields);
    }
}

#[test]
fn a_file_without_a_closed_frontmatter_has_none() {
    let fm = parse_frontmatter("# Title\nname: not-a-field\n");
    assert!(fm.fields.is_empty());
    assert_eq!(fm.body, "# Title\nname: not-a-field\n");
    let open = parse_frontmatter("---\nname: x\nno end\n");
    assert!(open.fields.is_empty());
}

#[test]
fn crlf_and_a_byte_order_mark_are_fine() {
    let fm = parse_frontmatter("\u{feff}---\r\nname: pdf\r\ndescription: d\r\n---\r\nbody\r\n");
    assert_eq!(fm.fields["name"], "pdf");
    assert_eq!(fm.fields["description"], "d");
}

#[test]
fn allowed_tools_is_removed_in_every_form() {
    let scalar = "---\nname: a\nallowed-tools: Bash(rm *)\ndescription: d\n---\nbody\n";
    assert_eq!(
        remove_field(scalar, "allowed-tools"),
        "---\nname: a\ndescription: d\n---\nbody\n"
    );
    let list =
        "---\nname: a\nallowed-tools:\n  - Bash\n  - Read\n\n- Write\ndescription: d\n---\nbody\n";
    assert_eq!(
        remove_field(list, "allowed-tools"),
        "---\nname: a\ndescription: d\n---\nbody\n"
    );
    let block = "---\nallowed-tools: >\n  Bash\n  Read\n---\nbody allowed-tools: x\n";
    assert_eq!(
        remove_field(block, "allowed-tools"),
        "---\n---\nbody allowed-tools: x\n"
    );
    let quoted = "---\r\n\"allowed-tools\": Bash\r\nname: a\r\n---\r\n";
    assert_eq!(
        remove_field(quoted, "allowed-tools"),
        "---\r\nname: a\r\n---\r\n"
    );
}

#[test]
fn allowed_tools_is_removed_from_a_frontmatter_dots_reads_as_broken() {
    // Claude Code's YAML may accept what the hand reader refuses; the line-based removal still
    // takes the field out.
    let text = "---\ndescription: Use when: x\nallowed-tools: Bash\n---\nbody\n";
    assert!(parse_frontmatter(text).fields.is_empty());
    assert_eq!(
        remove_field(text, "allowed-tools"),
        "---\ndescription: Use when: x\n---\nbody\n"
    );
}

#[test]
fn removal_leaves_text_without_a_closed_frontmatter_alone() {
    for text in [
        "allowed-tools: Bash\nbody\n",
        "---\nallowed-tools: Bash\nno end\n",
        "",
    ] {
        assert_eq!(remove_field(text, "allowed-tools"), text);
    }
}

// ---- skills ----

#[test]
fn a_skill_takes_its_name_from_the_folder_and_its_description_from_the_body() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("pdf");
    write(&skill.join("SKILL.md"), "\n# Reading PDFs\n\nMore.\n");
    let info = read_skill(&skill).unwrap();
    assert_eq!(info.name, "pdf");
    assert_eq!(info.description, "# Reading PDFs");
    assert_eq!(info.when_to_use, None);
    assert_eq!(info.dir, skill);
}

#[test]
fn a_skill_with_broken_frontmatter_still_loads_with_defaults() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("pdf");
    write(
        &skill.join("SKILL.md"),
        "---\nname: other\ndescription: Use when: x\n---\nFirst body line\n",
    );
    let info = read_skill(&skill).unwrap();
    assert_eq!(info.name, "pdf");
    assert_eq!(info.description, "First body line");
    assert!(info.fields.is_empty());
}

#[test]
fn a_skill_reads_its_fields() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("folder");
    write(
        &skill.join("SKILL.md"),
        "---\nname: pdf\ndescription: Extract text\nwhen_to_use: When a PDF is attached\nallowed-tools: Bash\n---\nbody\n",
    );
    let info = read_skill(&skill).unwrap();
    assert_eq!(info.name, "pdf");
    assert_eq!(info.when_to_use.as_deref(), Some("When a PDF is attached"));
    assert_eq!(info.fields["allowed-tools"], "Bash");
    assert_eq!(info.listing(), "Extract text When a PDF is attached");
}

#[test]
fn the_listing_is_one_line_and_cut_at_the_limit() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("s");
    let long = "é".repeat(2000);
    write(
        &skill.join("SKILL.md"),
        &format!("---\ndescription: >\n  a\n  b\nwhen_to_use: {long}\n---\n"),
    );
    let listing = read_skill(&skill).unwrap().listing();
    assert!(listing.starts_with("a b é"));
    assert_eq!(listing.chars().count(), LISTING_MAX_CHARS);
}

#[test]
fn a_missing_folder_or_skill_file_is_not_found() {
    let dir = tempdir().unwrap();
    let missing = read_skill(&dir.path().join("nope")).unwrap_err();
    assert!(matches!(&missing, Error::NotFound(m) if m.contains("folder not found")));
    fs::create_dir_all(dir.path().join("empty")).unwrap();
    let no_file = read_skill(&dir.path().join("empty")).unwrap_err();
    assert!(matches!(&no_file, Error::NotFound(m) if m.contains("SKILL.md not found")));
}

#[test]
fn a_scan_finds_the_skills_under_a_folder() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(&root.join("b").join("SKILL.md"), "---\nname: bee\n---\n");
    write(&root.join("a").join("SKILL.md"), "A skill\n");
    write(&root.join(".hidden").join("SKILL.md"), "x\n");
    write(&root.join("c").join("README.md"), "no skill\n");
    let names: Vec<_> = scan_skills(root)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["a", "bee"]);
}

#[test]
fn a_scan_of_a_skill_folder_gives_that_skill() {
    let dir = tempdir().unwrap();
    write(&dir.path().join("SKILL.md"), "One\n");
    write(&dir.path().join("sub").join("SKILL.md"), "Two\n");
    assert_eq!(scan_skills(dir.path()).unwrap().len(), 1);
}

#[test]
fn a_scan_falls_back_to_a_skills_folder_and_reports_a_missing_one() {
    let dir = tempdir().unwrap();
    write(&dir.path().join("skills").join("x").join("SKILL.md"), "X\n");
    let found = scan_skills(dir.path()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "x");
    assert!(matches!(
        scan_skills(&dir.path().join("nope")),
        Err(Error::NotFound(_))
    ));
}

// ---- plugins ----

#[test]
fn a_plugin_without_a_manifest_is_named_after_its_folder() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("pdf-tools");
    write(
        &root.join("skills").join("fill").join("SKILL.md"),
        "Fill forms\n",
    );
    write(
        &root.join("skills").join("empty").join("notes.txt"),
        "no skill\n",
    );
    write(&root.join("commands").join("run.md"), "Run\n");
    write(&root.join("agents").join("helper.md"), "Help\n");
    write(&root.join("hooks").join("hooks.json"), "{}");
    fs::create_dir_all(root.join("monitors")).unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    let info = read_plugin(&root).unwrap();
    assert_eq!(info.name, "pdf-tools");
    assert_eq!(info.skills.len(), 1);
    assert_eq!(info.skills[0].name, "fill");
    assert!(!info.single_skill);
    assert_eq!(info.commands, ["run"]);
    assert_eq!(info.agents, ["helper"]);
    assert!(info.has_hooks && info.has_monitors && info.has_bin);
    assert!(info.servers.is_empty() && info.warnings.is_empty());
}

#[test]
fn the_manifest_gives_the_name_and_a_bad_one_a_warning() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("folder");
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"pdf-tools","version":"1.2.0","description":"PDF helpers"}"#,
    );
    let info = read_plugin(&root).unwrap();
    assert_eq!(info.name, "pdf-tools");
    assert_eq!(info.version.as_deref(), Some("1.2.0"));
    assert_eq!(info.description.as_deref(), Some("PDF helpers"));

    write(
        &root.join(".claude-plugin").join("plugin.json"),
        "{not json",
    );
    let broken = read_plugin(&root).unwrap();
    assert_eq!(broken.name, "folder");
    assert_eq!(broken.warnings.len(), 1);
    assert!(broken.warnings[0].contains("plugin.json"));
}

#[test]
fn a_root_skill_without_a_skills_folder_is_a_single_skill_plugin() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("solo");
    write(&root.join("SKILL.md"), "---\nname: solo-skill\n---\nbody\n");
    let info = read_plugin(&root).unwrap();
    assert!(info.single_skill);
    assert_eq!(info.skills[0].name, "solo-skill");

    fs::create_dir_all(root.join("skills")).unwrap();
    let with_folder = read_plugin(&root).unwrap();
    assert!(!with_folder.single_skill);
    assert!(with_folder.skills.is_empty());
}

#[test]
fn mcp_json_servers_are_read_with_the_plugin_root_filled_in() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(
        &root.join(".mcp.json"),
        r#"{"mcpServers":{
            "db": {"command":"${CLAUDE_PLUGIN_ROOT}/bin/db","args":["--root","${CLAUDE_PLUGIN_ROOT}"],
                   "env":{"TOKEN":"${DB_TOKEN}"}},
            "api": {"type":"http","url":"https://example.com/mcp","headers":{"X":"1"}},
            "old": {"type":"sse","url":"https://example.com/sse"},
            "odd": {"type":"carrier-pigeon","url":"x"},
            "empty": {}
        }}"#,
    );
    let info = read_plugin(&root).unwrap();
    let root_text = root.to_string_lossy().into_owned();
    let names: Vec<_> = info.servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["api", "db", "old"]);
    let db = &info.servers[1].config;
    match db {
        McpServerConfig::Stdio {
            command, args, env, ..
        } => {
            assert_eq!(command, &format!("{root_text}/bin/db"));
            assert_eq!(args, &vec!["--root".to_string(), root_text.clone()]);
            // Only the plugin root is replaced; the other variable stays for the server's start.
            assert_eq!(env["TOKEN"], "${DB_TOKEN}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&info.servers[2].config, McpServerConfig::Http { url, .. } if url == "https://example.com/sse")
    );
    assert_eq!(info.warnings.len(), 2, "{:?}", info.warnings);
    assert!(info
        .warnings
        .iter()
        .any(|w| w.contains("odd") && w.contains("carrier-pigeon")));
    assert!(info.warnings.iter().any(|w| w.contains("empty")));
}

#[test]
fn mcp_json_may_be_the_servers_object_itself() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root.join(".mcp.json"), r#"{"db":{"command":"db-server"}}"#);
    let info = read_plugin(&root).unwrap();
    assert_eq!(info.servers.len(), 1);
    assert_eq!(info.servers[0].name, "db");
}

#[test]
fn servers_in_the_manifest_are_read_after_mcp_json() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root.join(".mcp.json"), r#"{"a":{"command":"from-file"}}"#);
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"mcpServers":{"a":{"command":"from-manifest"},"b":{"command":"b"}}}"#,
    );
    let info = read_plugin(&root).unwrap();
    let a = info.servers.iter().find(|s| s.name == "a").unwrap();
    assert!(matches!(&a.config, McpServerConfig::Stdio { command, .. } if command == "from-file"));
    assert_eq!(info.servers.len(), 2);
}

#[test]
fn a_broken_mcp_json_is_a_warning_and_no_servers() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root.join(".mcp.json"), "[1,2");
    let info = read_plugin(&root).unwrap();
    assert!(info.servers.is_empty());
    assert!(info.warnings[0].contains(".mcp.json"));
}

#[test]
fn server_items_get_the_plugin_prefix() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(
        &root.join(".mcp.json"),
        r#"{"My Server":{"command":"a"},"my_server":{"command":"a2"},"my-server":{"command":"b"},"db":{"command":"c"}}"#,
    );
    let items = read_plugin(&root).unwrap().server_items("pdf-tools");
    let names: Vec<_> = items.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        // `My Server` and `my_server` come out the same: the first (by key) is kept.
        [
            "plugin_pdf-tools_my_server",
            "plugin_pdf-tools_db",
            "plugin_pdf-tools_my-server"
        ]
    );
    for (name, _) in &items {
        assert!(
            dots_core::library::check_name(dots_core::library::LibraryKind::McpServer, name)
                .is_ok()
        );
    }
}

#[test]
fn a_plugin_folder_that_is_missing_is_not_found() {
    let dir = tempdir().unwrap();
    assert!(matches!(
        read_plugin(&dir.path().join("nope")),
        Err(Error::NotFound(_))
    ));
}

// ---- installed plugins ----

fn env_of<'a>(vars: &'a [(&'a str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone())
}

#[test]
fn installed_plugins_are_read_from_a_map_of_record_lists() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("cache").join("a");
    fs::create_dir_all(&a).unwrap();
    write(
        &dir.path().join("installed_plugins.json"),
        &format!(
            r#"{{"version":2,"plugins":{{
                "zeta@mkt":[{{"scope":"project","installPath":"{p}/gone","version":"0.1"}},
                            {{"scope":"user","installPath":"{a}","version":"1.2.0"}}],
                "alpha@other-mkt":{{"scope":"user","installPath":"{p}/alpha"}},
                "broken@mkt":{{"scope":"user"}},
                "number@mkt":5
            }}}}"#,
            p = dir.path().to_string_lossy().replace('\\', "/"),
            a = a.to_string_lossy().replace('\\', "/"),
        ),
    );
    let found = read_installed(dir.path());
    let ids: Vec<_> = found.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["alpha@other-mkt", "zeta@mkt"]);
    assert_eq!(found[1].name, "zeta");
    assert_eq!(found[1].marketplace.as_deref(), Some("mkt"));
    assert_eq!(found[1].scope.as_deref(), Some("user"));
    assert_eq!(found[1].version.as_deref(), Some("1.2.0"));
    assert!(found[1].exists);
    assert!(!found[0].exists);
}

#[test]
fn installed_plugins_tolerate_a_top_level_map_and_junk() {
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("installed_plugins.json"),
        r#"{"version":1,"x@m":{"installPath":"C:/nowhere"}}"#,
    );
    let found = read_installed(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "x@m");

    for junk in ["", "not json", "[1,2]", "null", r#"{"plugins":[1]}"#] {
        write(&dir.path().join("installed_plugins.json"), junk);
        assert!(read_installed(dir.path()).is_empty(), "{junk:?}");
    }
    fs::remove_file(dir.path().join("installed_plugins.json")).unwrap();
    assert!(read_installed(dir.path()).is_empty());
}

#[test]
fn the_cache_dir_variable_moves_the_root() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    write(
        &dir.path().join("installed_plugins.json"),
        r#"{"plugins":{"p@m":{"installPath":"C:/x"}}}"#,
    );
    let vars = [
        ("CLAUDE_CODE_PLUGIN_CACHE_DIR", root.clone()),
        ("USERPROFILE", "C:/home".into()),
    ];
    let env = env_of(&vars);
    assert_eq!(plugins_root(&env).unwrap(), Path::new(&root));
    assert_eq!(installed_plugins(&env).len(), 1);
    assert!(find_installed(&env, "p@m").is_some());
    assert!(find_installed(&env, "q@m").is_none());

    let home = [("USERPROFILE", "C:/home".to_string())];
    assert_eq!(
        plugins_root(&env_of(&home)).unwrap(),
        Path::new("C:/home").join(".claude").join("plugins")
    );
    let blank = [
        ("CLAUDE_CODE_PLUGIN_CACHE_DIR", "  ".to_string()),
        ("HOME", "/h".into()),
    ];
    assert_eq!(
        plugins_root(&env_of(&blank)).unwrap(),
        Path::new("/h").join(".claude").join("plugins")
    );
    assert!(plugins_root(&env_of(&[])).is_none());
    assert!(installed_plugins(&env_of(&[])).is_empty());
}

#[test]
fn a_plugins_skills_are_named_after_its_library_item() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("folder");
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"manifest-name"}"#,
    );
    write(
        &root.join("skills").join("fill").join("SKILL.md"),
        "Fill
",
    );
    write(
        &root.join("skills").join("x").join("SKILL.md"),
        "---
name: extract
---
",
    );
    let info = read_plugin(&root).unwrap();
    let names: Vec<String> = info
        .qualified_skills("docs")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["docs:fill", "docs:extract"]);
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test kit_run_test --test kit_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
```

Expected: both test targets do not compile: 2 × `` error[E0432]: unresolved import `dots_core::kit` ``, and two type errors that follow from it (`` can't compare `Vec<String>` with `&Vec<String>` `` and `` can't compare `String` with `&String` ``), then ``could not compile `dots-core` (test "kit_test") due to 3 previous errors`` and `(test "kit_run_test") due to 1 previous error`.

- [ ] **Step 3: Implement**

**Create `crates/dots-core/src/kit/frontmatter.rs`:**

```rust
//! The YAML frontmatter of a `SKILL.md` or a command file, read by hand: top-level `key: value`
//! pairs, quoted values, `>` and `|` block scalars. Lists, nested maps and flow collections are
//! skipped. As in Claude Code, a frontmatter that does not parse gives no fields at all.

use std::collections::BTreeMap;

/// A file split into its frontmatter fields and its body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    /// Top-level keys whose value is a scalar. Empty when there is no frontmatter or it is broken.
    pub fields: BTreeMap<String, String>,
    /// What follows the frontmatter; the whole text when there is none.
    pub body: String,
}

fn is_open(line: &str) -> bool {
    line.trim_end() == "---"
}

fn is_close(line: &str) -> bool {
    matches!(line.trim_end(), "---" | "...")
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// Splits `text` into its frontmatter and its body.
pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().is_none_or(|l| !is_open(l)) {
        return Frontmatter {
            fields: BTreeMap::new(),
            body: text.to_string(),
        };
    }
    let Some(close) = (1..lines.len()).find(|&i| is_close(lines[i])) else {
        // Never closed: no frontmatter, as in Claude Code.
        return Frontmatter {
            fields: BTreeMap::new(),
            body: lines[1..].join("\n"),
        };
    };
    Frontmatter {
        fields: parse_map(&lines[1..close]).unwrap_or_default(),
        body: lines[close + 1..].join("\n"),
    }
}

/// The key of a top-level line and the text after its colon. `None` when the line is not
/// `key: ...` (a plain key ends at the first `:` followed by a space or the end of the line).
fn split_key(line: &str) -> Option<(String, &str)> {
    if let Some(quote) = line.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let rest = &line[1..];
        let end = rest.find(quote)?;
        let after = rest[end + 1..].trim_start();
        let after = after.strip_prefix(':')?;
        return Some((rest[..end].to_string(), after));
    }
    let mut at = 0;
    while let Some(i) = line[at..].find(':') {
        let colon = at + i;
        let after = &line[colon + 1..];
        if after.is_empty() || after.starts_with([' ', '\t']) {
            let key = line[..colon].trim();
            return (!key.is_empty()).then(|| (key.to_string(), after));
        }
        at = colon + 1;
    }
    None
}

/// `s` without a trailing ` # comment`.
fn without_comment(s: &str) -> &str {
    let mut prev_space = true;
    for (i, c) in s.char_indices() {
        if c == '#' && prev_space {
            return s[..i].trim_end();
        }
        prev_space = c == ' ' || c == '\t';
    }
    s.trim_end()
}

/// A plain scalar may not hold `: ` or end with `:` (YAML refuses it).
fn plain_ok(s: &str) -> bool {
    !(s.contains(": ") || s.contains(":\t") || s.ends_with(':'))
}

fn parse_map(lines: &[&str]) -> Option<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    let mut seen = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if is_blank(line) || is_comment(line) {
            continue;
        }
        if indent(line) > 0 || line.starts_with("- ") || line == "-" {
            return None;
        }
        let (key, rest) = split_key(line)?;
        if seen.contains(&key) {
            return None;
        }
        seen.push(key.clone());
        let rest = rest.trim();
        // The lines after this one that belong to the value.
        let mut end = i;
        let mut value = None;
        match rest.chars().next() {
            None | Some('#') => {
                // A list or a nested map follows, or nothing.
                while end < lines.len()
                    && (is_blank(lines[end])
                        || indent(lines[end]) > 0
                        || lines[end].starts_with("- ")
                        || lines[end] == "-")
                {
                    end += 1;
                }
            }
            Some('|') | Some('>') => {
                while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
                    end += 1;
                }
                value = Some(block_scalar(rest, &lines[i..end]));
            }
            Some('[') | Some('{') => {
                let mut depth = bracket_depth(rest);
                while depth > 0 && end < lines.len() {
                    depth += bracket_depth(lines[end]);
                    end += 1;
                }
            }
            Some(q @ ('"' | '\'')) => {
                let (text, used) = quoted(q, rest, &lines[i..])?;
                end = i + used;
                value = Some(text);
            }
            Some('@') | Some('`') => return None,
            Some('&') | Some('*') | Some('!') => {
                while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
                    end += 1;
                }
            }
            Some(_) => {
                let mut text = without_comment(rest).to_string();
                if !plain_ok(&text) {
                    return None;
                }
                // Indented lines continue a plain scalar.
                while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
                    let more = lines[end].trim();
                    end += 1;
                    if more.is_empty() || more.starts_with('#') {
                        continue;
                    }
                    let more = without_comment(more);
                    if !plain_ok(more) {
                        return None;
                    }
                    text.push(' ');
                    text.push_str(more);
                }
                value = Some(text);
            }
        }
        if let Some(v) = value {
            fields.insert(key, v);
        }
        i = end;
    }
    Some(fields)
}

fn bracket_depth(s: &str) -> i32 {
    s.chars()
        .map(|c| match c {
            '[' | '{' => 1,
            ']' | '}' => -1,
            _ => 0,
        })
        .sum()
}

/// A `|` or `>` scalar. `>` joins lines with a space and a blank line becomes a line break.
fn block_scalar(header: &str, lines: &[&str]) -> String {
    let folded = header.starts_with('>');
    let base = lines
        .iter()
        .filter(|l| !is_blank(l))
        .map(|l| indent(l))
        .next()
        .unwrap_or(0);
    let mut out = String::new();
    let mut pending_breaks = 0;
    for line in lines {
        if is_blank(line) {
            pending_breaks += 1;
            continue;
        }
        let text = line.get(base.min(line.len())..).unwrap_or("").trim_end();
        if !out.is_empty() {
            if folded {
                out.push_str(&"\n".repeat(pending_breaks));
                if pending_breaks == 0 {
                    out.push(' ');
                }
            } else {
                out.push_str(&"\n".repeat(pending_breaks + 1));
            }
        }
        pending_breaks = 0;
        out.push_str(text);
    }
    out
}

/// A quoted scalar that starts in `first` and may go on in `more`. Returns the text and the
/// number of lines of `more` it used. `None` when the quote is not closed, or text follows it.
fn quoted(quote: char, first: &str, more: &[&str]) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut used = 0;
    let mut line = &first[1..];
    loop {
        let mut chars = line.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if quote == '"' && c == '\\' {
                match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    ' ' => out.push(' '),
                    'u' => {
                        let hex: String =
                            (0..4).filter_map(|_| chars.next().map(|p| p.1)).collect();
                        out.push(
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)?,
                        );
                    }
                    _ => return None,
                }
            } else if c == quote {
                if quote == '\'' && chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                let tail = line[i + c.len_utf8()..].trim();
                return (tail.is_empty() || tail.starts_with('#')).then_some((out, used));
            } else {
                out.push(c);
            }
        }
        // The quote is still open: the next line continues it, joined by a space.
        let next = more.get(used)?;
        used += 1;
        if is_blank(next) {
            out.push('\n');
            line = "";
        } else {
            if !out.ends_with('\n') {
                out.push(' ');
            }
            line = next.trim_start();
        }
    }
}

/// `text` with the top-level field `key` taken out of its frontmatter, whatever its value looks
/// like: a scalar, a block scalar, or a list. It works on the lines, not on the parsed fields, so
/// that a frontmatter Dots reads as broken but Claude Code reads is cleaned too. A text without
/// a closed frontmatter is returned as it is.
pub fn remove_field(text: &str, key: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let opens = lines
        .first()
        .is_some_and(|l| is_open(l.strip_prefix('\u{feff}').unwrap_or(l)));
    let close = (1..lines.len()).find(|&i| is_close(lines[i]));
    let (true, Some(close)) = (opens, close) else {
        return text.to_string();
    };
    let mut out = String::new();
    out.push_str(lines[0]);
    let mut skipping = false;
    let mut blanks: Vec<&str> = Vec::new();
    for line in &lines[1..close] {
        if skipping {
            if is_blank(line) {
                blanks.push(line);
                continue;
            }
            if indent(line) > 0 || line.starts_with("- ") || line.trim_end() == "-" {
                blanks.clear();
                continue;
            }
            skipping = false;
            blanks.drain(..).for_each(|b| out.push_str(b));
        }
        let plain = line.trim_end_matches(['\r', '\n']);
        if indent(plain) == 0
            && !is_comment(plain)
            && split_key(plain).is_some_and(|(k, _)| k == key)
        {
            skipping = true;
            continue;
        }
        out.push_str(line);
    }
    blanks.drain(..).for_each(|b| out.push_str(b));
    for line in &lines[close..] {
        out.push_str(line);
    }
    out
}
```

**Create `crates/dots-core/src/kit/skill.rs`:**

```rust
//! A skill folder: a `SKILL.md` and the files beside it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;
use crate::{Error, Result};

/// Claude Code cuts a skill's `description` joined with `when_to_use` at this many characters in
/// its listing; the local engine's skills section does the same.
pub const LISTING_MAX_CHARS: usize = 1536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInfo {
    /// The frontmatter `name`, else the folder name. It may not be a valid Library name: the
    /// caller checks it with `check_name`.
    pub name: String,
    /// The frontmatter `description`, else the first non-empty line of the body (empty if none).
    pub description: String,
    pub when_to_use: Option<String>,
    pub dir: PathBuf,
    /// All scalar frontmatter fields (`allowed-tools` as a string included).
    pub fields: BTreeMap<String, String>,
}

impl SkillInfo {
    /// `description` and `when_to_use` joined, on one line and cut at `LISTING_MAX_CHARS`.
    pub fn listing(&self) -> String {
        let joined = match &self.when_to_use {
            Some(w) if !w.trim().is_empty() => format!("{} {}", self.description, w),
            _ => self.description.clone(),
        };
        let one_line = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        one_line.chars().take(LISTING_MAX_CHARS).collect()
    }
}

fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Reads `<dir>/SKILL.md`. `NotFound` when the folder or the file is missing.
pub fn read_skill(dir: &Path) -> Result<SkillInfo> {
    if !dir.is_dir() {
        return Err(Error::NotFound(format!(
            "folder not found: {}",
            dir.display()
        )));
    }
    let file = dir.join("SKILL.md");
    let bytes = fs::read(&file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Error::NotFound(format!("SKILL.md not found in {}", dir.display()))
        }
        _ => Error::Io(e),
    })?;
    let parsed = parse_frontmatter(&String::from_utf8_lossy(&bytes));
    let field = |key: &str| {
        parsed
            .fields
            .get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let description = field("description")
        .or_else(|| {
            parsed
                .body
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(String::from)
        })
        .unwrap_or_default();
    Ok(SkillInfo {
        name: field("name").unwrap_or_else(|| folder_name(dir)),
        description,
        when_to_use: field("when_to_use"),
        dir: dir.to_path_buf(),
        fields: parsed.fields,
    })
}

fn skills_in_children(root: &Path) -> Vec<SkillInfo> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.join("SKILL.md").is_file())
        .collect();
    dirs.sort();
    dirs.iter().filter_map(|d| read_skill(d).ok()).collect()
}

/// The skills under `root` (`library_scan_skills`): `root` itself when it holds a `SKILL.md`,
/// else its subfolders that do, else those of `root/skills`. By folder, so by path. `NotFound`
/// when `root` is not a folder.
pub fn scan_skills(root: &Path) -> Result<Vec<SkillInfo>> {
    if !root.is_dir() {
        return Err(Error::NotFound(format!(
            "folder not found: {}",
            root.display()
        )));
    }
    if root.join("SKILL.md").is_file() {
        return Ok(vec![read_skill(root)?]);
    }
    let found = skills_in_children(root);
    if found.is_empty() {
        return Ok(skills_in_children(&root.join("skills")));
    }
    Ok(found)
}
```

**Create `crates/dots-core/src/kit/plugin.rs`:**

```rust
//! A Claude Code plugin folder, read for the Library (MCP/skills spec section 2).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::skill::{read_skill, SkillInfo};
use crate::library::{plugin_server_name, LibraryConfig, McpServerConfig};
use crate::{Error, Result};

/// An MCP server of the plugin, as `.mcp.json` names it, with `${CLAUDE_PLUGIN_ROOT}` replaced
/// by the plugin's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginServer {
    pub name: String,
    pub config: McpServerConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    /// The manifest `name`, else the folder name. The caller checks it with `check_name`.
    pub name: String,
    pub root: PathBuf,
    pub version: Option<String>,
    pub description: Option<String>,
    /// `skills/<name>/SKILL.md`, by folder; or the plugin's own `SKILL.md` for a single-skill
    /// plugin. A skill in `skills/` that cannot be read is left out with a warning.
    pub skills: Vec<SkillInfo>,
    /// The root holds `SKILL.md` and there is no `skills/` folder.
    pub single_skill: bool,
    /// File names without `.md`, from `commands/` and `agents/`.
    pub commands: Vec<String>,
    pub agents: Vec<String>,
    /// `hooks/hooks.json` exists, or the manifest has `hooks`.
    pub has_hooks: bool,
    pub has_monitors: bool,
    pub has_bin: bool,
    pub servers: Vec<PluginServer>,
    /// What could not be read, in words for the person (an unreadable server, a bad manifest).
    pub warnings: Vec<String>,
}

impl PluginInfo {
    /// The plugin's skills with their qualified names, `<library name>:<skill name>`, the names
    /// Claude Code gives them in a run (the copy's manifest is named `library_name`) and the
    /// local engine's `skill` tool and prompt use.
    pub fn qualified_skills(&self, library_name: &str) -> Vec<(String, &SkillInfo)> {
        self.skills
            .iter()
            .map(|s| (format!("{library_name}:{}", s.name), s))
            .collect()
    }

    /// The servers as Library items would hold them: the name `plugin_<plugin>_<server>` (made
    /// valid) with the config. `plugin` is the plugin's Library name. A server whose name comes
    /// out the same as an earlier one's is left out.
    pub fn server_items(&self, plugin: &str) -> Vec<(String, McpServerConfig)> {
        let mut items: Vec<(String, McpServerConfig)> = Vec::new();
        for server in &self.servers {
            let name = plugin_server_name(plugin, &server.name);
            if !items.iter().any(|(n, _)| *n == name) {
                items.push((name, server.config.clone()));
            }
        }
        items
    }
}

fn read_json(path: &Path) -> std::result::Result<Option<Value>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map(Some)
        .map_err(|e| e.to_string())
}

fn names_of_md(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".md").map(String::from)
        })
        .collect();
    names.sort();
    names
}

/// `value` with `${CLAUDE_PLUGIN_ROOT}` replaced in every string.
fn with_root(value: &Value, root: &str) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("${CLAUDE_PLUGIN_ROOT}", root)),
        Value::Array(a) => Value::Array(a.iter().map(|v| with_root(v, root)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), with_root(v, root)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// One entry of an `mcpServers` object as a Library config: `type` is `stdio`, `http`, `sse` or
/// `streamable-http`; without it a `url` means http and a `command` stdio.
fn server_config(entry: &Value) -> std::result::Result<McpServerConfig, String> {
    let obj = entry.as_object().ok_or("not an object")?;
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let strings = |key: &str| -> BTreeMap<String, String> {
        obj.get(key)
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), text(v))).collect())
            .unwrap_or_default()
    };
    let default_type = if obj.contains_key("url") {
        "http"
    } else {
        "stdio"
    };
    let config = match obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(default_type)
    {
        "stdio" => McpServerConfig::Stdio {
            command: obj
                .get("command")
                .and_then(Value::as_str)
                .ok_or("no command")?
                .to_string(),
            args: obj
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(text).collect())
                .unwrap_or_default(),
            cwd: obj.get("cwd").and_then(Value::as_str).map(String::from),
            env: strings("env"),
        },
        "http" | "sse" | "streamable-http" => McpServerConfig::Http {
            url: obj
                .get("url")
                .and_then(Value::as_str)
                .ok_or("no url")?
                .to_string(),
            headers: strings("headers"),
        },
        other => return Err(format!("transport {other:?} is not supported")),
    };
    LibraryConfig::McpServer(config.clone())
        .validate()
        .map_err(|e| e.to_string())?;
    Ok(config)
}

/// The servers of an `.mcp.json` value: `{"mcpServers": {...}}` or the servers object itself.
fn servers_object(value: &Value) -> Option<&Map<String, Value>> {
    let obj = value.as_object()?;
    match obj.get("mcpServers") {
        Some(Value::Object(inner)) => Some(inner),
        _ => Some(obj),
    }
}

/// Adds the servers of an `.mcp.json` value, with `${CLAUDE_PLUGIN_ROOT}` replaced. A server
/// already listed (by name) is kept as it is.
fn add_servers(
    servers: &mut Vec<PluginServer>,
    warnings: &mut Vec<String>,
    root: &str,
    origin: &str,
    value: &Value,
) {
    let Some(map) = servers_object(value) else {
        warnings.push(format!("{origin}: not an object"));
        return;
    };
    for (server, entry) in map {
        match server_config(&with_root(entry, root)) {
            Ok(config) if !servers.iter().any(|s| s.name == *server) => {
                servers.push(PluginServer {
                    name: server.clone(),
                    config,
                })
            }
            Ok(_) => {}
            Err(e) => warnings.push(format!("MCP server {server} ({origin}): {e}")),
        }
    }
}

/// Reads the plugin folder `root`. `NotFound` when it is not a folder. Parts that cannot be
/// read are left out and named in `warnings`.
pub fn read_plugin(root: &Path) -> Result<PluginInfo> {
    if !root.is_dir() {
        return Err(Error::NotFound(format!(
            "folder not found: {}",
            root.display()
        )));
    }
    let mut warnings = Vec::new();
    let folder = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let manifest = match read_json(&root.join(".claude-plugin").join("plugin.json")) {
        Ok(m) => m.filter(Value::is_object),
        Err(e) => {
            warnings.push(format!("plugin.json could not be read: {e}"));
            None
        }
    };
    let manifest_text = |key: &str| {
        manifest
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let name = manifest_text("name").unwrap_or(folder);

    let skills_dir = root.join("skills");
    let single_skill = root.join("SKILL.md").is_file() && !skills_dir.exists();
    let mut skills = Vec::new();
    if single_skill {
        match read_skill(root) {
            Ok(skill) => skills.push(skill),
            Err(e) => warnings.push(format!("SKILL.md could not be read: {e}")),
        }
    } else if let Ok(entries) = fs::read_dir(&skills_dir) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| p.join("SKILL.md").is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            match read_skill(&dir) {
                Ok(skill) => skills.push(skill),
                Err(e) => warnings.push(format!("skill {}: {e}", dir.display())),
            }
        }
    }

    let root_text = root.to_string_lossy();
    let mut servers: Vec<PluginServer> = Vec::new();
    match read_json(&root.join(".mcp.json")) {
        Ok(Some(value)) => {
            add_servers(&mut servers, &mut warnings, &root_text, ".mcp.json", &value)
        }
        Ok(None) => {}
        Err(e) => warnings.push(format!(".mcp.json could not be read: {e}")),
    }
    match manifest.as_ref().and_then(|m| m.get("mcpServers")) {
        Some(value @ Value::Object(_)) => add_servers(
            &mut servers,
            &mut warnings,
            &root_text,
            "plugin.json",
            value,
        ),
        Some(_) => warnings.push("plugin.json: mcpServers given as a path is not read".into()),
        None => {}
    }

    Ok(PluginInfo {
        name,
        version: manifest_text("version"),
        description: manifest_text("description"),
        skills,
        single_skill,
        commands: names_of_md(&root.join("commands")),
        agents: names_of_md(&root.join("agents")),
        has_hooks: root.join("hooks").join("hooks.json").is_file()
            || manifest.as_ref().is_some_and(|m| m.get("hooks").is_some()),
        has_monitors: root.join("monitors").exists(),
        has_bin: root.join("bin").exists(),
        servers,
        root: root.to_path_buf(),
        warnings,
    })
}
```

**Create `crates/dots-core/src/kit/installed.rs`:**

```rust
//! The plugins Claude Code has installed, from `installed_plugins.json`. The file's schema is
//! not documented, so this reads what it can and ignores the rest.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// `<name>@<marketplace>`
    pub id: String,
    pub name: String,
    pub marketplace: Option<String>,
    pub scope: Option<String>,
    pub install_path: PathBuf,
    pub version: Option<String>,
    /// `install_path` is a folder now (an update may have moved it).
    pub exists: bool,
}

/// The folder that holds `installed_plugins.json`: `CLAUDE_CODE_PLUGIN_CACHE_DIR` when set,
/// else `~/.claude/plugins` (the home folder from `USERPROFILE`, then `HOME`).
pub fn plugins_root(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    if let Some(dir) = set("CLAUDE_CODE_PLUGIN_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = set("USERPROFILE").or_else(|| set("HOME"))?;
    Some(PathBuf::from(home).join(".claude").join("plugins"))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// One plugin id's record, or the list of its records (one per scope). The user's scope wins.
fn plugin_from(id: &str, value: &Value) -> Option<InstalledPlugin> {
    let records: Vec<&Value> = match value {
        Value::Array(list) => list.iter().collect(),
        other => vec![other],
    };
    let readable: Vec<&Value> = records
        .into_iter()
        .filter(|r| text(r, "installPath").is_some())
        .collect();
    let record = readable
        .iter()
        .find(|r| r.get("scope").and_then(Value::as_str) == Some("user"))
        .or_else(|| readable.first())?;
    let install_path = PathBuf::from(text(record, "installPath")?);
    let (name, marketplace) = match id.split_once('@') {
        Some((n, m)) => (n.to_string(), Some(m.to_string())),
        None => (id.to_string(), None),
    };
    Some(InstalledPlugin {
        id: id.to_string(),
        name,
        marketplace,
        scope: text(record, "scope"),
        exists: install_path.is_dir(),
        install_path,
        version: text(record, "version"),
    })
}

/// The plugins listed in `<root>/installed_plugins.json`, by id. A missing or unreadable file
/// gives none. The records are in `plugins` (the shape seen so far) or, failing that, at the top
/// level.
pub fn read_installed(root: &Path) -> Vec<InstalledPlugin> {
    let Ok(raw) = fs::read_to_string(root.join("installed_plugins.json")) else {
        return Vec::new();
    };
    let Ok(Value::Object(top)) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let map = match top.get("plugins") {
        Some(Value::Object(plugins)) => plugins,
        _ => &top,
    };
    let mut found: Vec<InstalledPlugin> = map
        .iter()
        .filter_map(|(id, value)| plugin_from(id, value))
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// The installed plugins, from the folder `plugins_root` names.
pub fn installed_plugins(env: &dyn Fn(&str) -> Option<String>) -> Vec<InstalledPlugin> {
    plugins_root(env).map_or_else(Vec::new, |root| read_installed(&root))
}

/// The installed plugin with this id, if the file still lists it.
pub fn find_installed(env: &dyn Fn(&str) -> Option<String>, id: &str) -> Option<InstalledPlugin> {
    installed_plugins(env).into_iter().find(|p| p.id == id)
}
```

**Create `crates/dots-core/src/kit/run_dirs.rs`:**

```rust
//! The plugin folders of one Claude-engine run (MCP/skills spec section 7): copies under
//! `<plugins_dir>/run-<run_id>/`, each passed to the CLI with `--plugin-dir`, deleted when the
//! run ends.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::frontmatter::remove_field;
use crate::library::LibraryKind;
use crate::{Error, Result};

/// A copy larger than this is left out.
pub const MAX_COPY_BYTES: u64 = 50 * 1024 * 1024;

/// A standalone skill the dot uses: its Library name and its folder.
#[derive(Debug, Clone)]
pub struct KitSkill {
    pub name: String,
    pub dir: PathBuf,
}

/// A plugin the dot uses: its Library name, its folder, and whether its hooks run.
#[derive(Debug, Clone)]
pub struct KitPlugin {
    pub name: String,
    pub dir: PathBuf,
    pub run_hooks: bool,
}

/// An item that did not get into the run, and why (for a run event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub kind: LibraryKind,
    pub name: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct RunKit {
    root: PathBuf,
    /// The folders for `--plugin-dir`: `dots-skills` first (if any skill went in), then each
    /// plugin, in the order given.
    pub plugin_dirs: Vec<PathBuf>,
    pub left_out: Vec<LeftOut>,
}

impl RunKit {
    /// Deletes the run's folder. Safe to call twice.
    pub fn cleanup(&self) {
        if let Err(e) = fs::remove_dir_all(&self.root) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("could not delete {}: {e}", self.root.display());
            }
        }
    }
}

/// The files of `src` to copy, as paths relative to it, and their total size. `None` when the
/// total goes past `MAX_COPY_BYTES`. Symbolic links are not followed or copied, and `.git` is
/// left out; `skip` names more paths to leave out.
fn collect(src: &Path, skip: &dyn Fn(&Path) -> bool) -> std::io::Result<Option<Vec<PathBuf>>> {
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in fs::read_dir(src.join(&rel))? {
            let entry = entry?;
            let path = rel.join(entry.file_name());
            if entry.file_name() == ".git" || skip(&path) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                total += entry.metadata()?.len();
                if total > MAX_COPY_BYTES {
                    return Ok(None);
                }
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(Some(files))
}

/// The cleaned text of a copied file. A `SKILL.md`, a command file and an agent file lose
/// `allowed-tools`, and `hooks` unless `run_hooks`; an agent file also loses `permissionMode`
/// and `mcpServers`, always (an agent must not change Dots' permission handling or bring
/// servers past the gateway).
fn cleaned(rel: &Path, bytes: Vec<u8>, run_hooks: bool) -> Vec<u8> {
    let is_md = rel.extension().is_some_and(|e| e == "md");
    let is_skill = rel.file_name().is_some_and(|n| n == "SKILL.md");
    let is_command = is_md && rel.starts_with("commands");
    let is_agent = is_md && rel.starts_with("agents");
    if !(is_skill || is_command || is_agent) {
        return bytes;
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    text = remove_field(&text, "allowed-tools");
    if !run_hooks {
        text = remove_field(&text, "hooks");
    }
    if is_agent {
        text = remove_field(&text, "permissionMode");
        text = remove_field(&text, "mcpServers");
    }
    text.into_bytes()
}

/// Writes the copy's manifest: the original (when it is a JSON object) without `mcpServers`,
/// `monitors` and, unless `run_hooks`, `hooks`; `name` set to the plugin's Library name, so
/// Claude Code calls its skills `<name>:<skill>`. A plugin with no manifest, or an unreadable
/// one, gets `{"name": ...}`.
fn write_manifest(dest: &Path, name: &str, run_hooks: bool) -> std::io::Result<()> {
    let path = dest.join(".claude-plugin").join("plugin.json");
    let mut manifest = match fs::read(&path).map(|b| serde_json::from_slice::<Value>(&b)) {
        Ok(Ok(Value::Object(m))) => m,
        _ => serde_json::Map::new(),
    };
    manifest.remove("mcpServers");
    manifest.remove("monitors");
    if !run_hooks {
        manifest.remove("hooks");
    }
    manifest.insert("name".into(), Value::String(name.into()));
    fs::create_dir_all(path.parent().expect("has a parent"))?;
    fs::write(
        path,
        serde_json::to_vec_pretty(&Value::Object(manifest)).expect("a JSON object"),
    )
}

/// Copies the files `files` of `src` into `dest`, cleaning those that need it.
fn copy_files(src: &Path, dest: &Path, files: &[PathBuf], run_hooks: bool) -> std::io::Result<()> {
    for rel in files {
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, cleaned(rel, fs::read(src.join(rel))?, run_hooks))?;
    }
    Ok(())
}

/// Copies one folder, or says why not. `Err(reason)` leaves nothing behind.
fn copy_folder(
    src: &Path,
    dest: &Path,
    run_hooks: bool,
    skip: &dyn Fn(&Path) -> bool,
) -> std::result::Result<(), String> {
    if !src.is_dir() {
        return Err("folder not found".into());
    }
    let files = match collect(src, skip) {
        Ok(Some(files)) => files,
        Ok(None) => return Err("larger than 50 MB".into()),
        Err(e) => return Err(format!("could not be read: {e}")),
    };
    copy_files(src, dest, &files, run_hooks).map_err(|e| {
        let _ = fs::remove_dir_all(dest);
        format!("could not be copied: {e}")
    })
}

/// Writes the run's plugin folders under `<plugins_dir>/run-<run_id>/` (an old folder of that
/// name is replaced):
///
/// - `dots-skills/`: a generated plugin whose `skills/<name>/` holds a copy of each skill;
/// - `<name>/`: a copy of each plugin, its manifest named `<name>`, without `.mcp.json` (its servers go through the gateway),
///   without `monitors/`, and without `hooks/` unless `run_hooks`.
///
/// Every copied `SKILL.md`, command and agent file loses `allowed-tools`, `hooks` (unless the
/// plugin's `run_hooks`) and, for an agent, `permissionMode` and `mcpServers`. A skill or plugin that is
/// missing, has no `SKILL.md` (a skill), or is over 50 MB is listed in `left_out`. Nothing is
/// created when nothing goes in. This blocks: call it from `spawn_blocking`.
pub fn prepare_run_kit(
    plugins_dir: &Path,
    run_id: &str,
    skills: &[KitSkill],
    plugins: &[KitPlugin],
) -> Result<RunKit> {
    let root = plugins_dir.join(format!("run-{run_id}"));
    let mut kit = RunKit {
        root: root.clone(),
        plugin_dirs: Vec::new(),
        left_out: Vec::new(),
    };
    if skills.is_empty() && plugins.is_empty() {
        return Ok(kit);
    }
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;

    let skills_plugin = root.join("dots-skills");
    let mut copied_skills = 0;
    for skill in skills {
        let mut leave = |reason: String| {
            kit.left_out.push(LeftOut {
                kind: LibraryKind::Skill,
                name: skill.name.clone(),
                reason,
            })
        };
        if skill.dir.is_dir() && !skill.dir.join("SKILL.md").is_file() {
            leave("SKILL.md not found".into());
            continue;
        }
        let dest = skills_plugin.join("skills").join(&skill.name);
        match copy_folder(&skill.dir, &dest, false, &|_| false) {
            Ok(()) => copied_skills += 1,
            Err(reason) => leave(reason),
        }
    }
    if copied_skills > 0 {
        let manifest = json!({
            "name": "dots-skills",
            "description": "The skills this Dots run uses",
            "version": "0.0.0",
        });
        let dir = skills_plugin.join(".claude-plugin");
        let write = fs::create_dir_all(&dir).and_then(|_| {
            fs::write(
                dir.join("plugin.json"),
                serde_json::to_vec_pretty(&manifest).expect("a literal"),
            )
        });
        if let Err(e) = write {
            kit.cleanup();
            return Err(Error::Io(e));
        }
        kit.plugin_dirs.push(skills_plugin);
    }

    for plugin in plugins {
        let dest = root.join(&plugin.name);
        let run_hooks = plugin.run_hooks;
        let skip = move |rel: &Path| {
            let first = rel.components().next().map(|c| c.as_os_str().to_owned());
            let top = rel.components().count() == 1;
            match first.as_deref().and_then(|f| f.to_str()) {
                Some(".mcp.json") => top,
                Some("monitors") => true,
                Some("hooks") => !run_hooks,
                _ => false,
            }
        };
        let copied = copy_folder(&plugin.dir, &dest, run_hooks, &skip).and_then(|()| {
            write_manifest(&dest, &plugin.name, run_hooks).map_err(|e| {
                let _ = fs::remove_dir_all(&dest);
                format!("could not be copied: {e}")
            })
        });
        match copied {
            Ok(()) => kit.plugin_dirs.push(dest),
            Err(reason) => kit.left_out.push(LeftOut {
                kind: LibraryKind::Plugin,
                name: plugin.name.clone(),
                reason,
            }),
        }
    }
    Ok(kit)
}
```

**Create `crates/dots-core/src/kit/mod.rs`:**

```rust
//! Skills and plugins on disk (MCP/skills spec sections 2 and 7): reading a `SKILL.md`, listing
//! the skills under a folder, reading a plugin folder, the plugins Claude Code has installed,
//! and the per-run plugin folders the Claude engine passes with `--plugin-dir`. It reads and
//! copies folders only; the Library, the runs and the engines call it.

mod frontmatter;
mod installed;
mod plugin;
mod run_dirs;
mod skill;

pub use frontmatter::{parse_frontmatter, remove_field, Frontmatter};
pub use installed::{
    find_installed, installed_plugins, plugins_root, read_installed, InstalledPlugin,
};
pub use plugin::{read_plugin, PluginInfo, PluginServer};
pub use run_dirs::{prepare_run_kit, KitPlugin, KitSkill, LeftOut, RunKit, MAX_COPY_BYTES};
pub use skill::{read_skill, scan_skills, SkillInfo, LISTING_MAX_CHARS};
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod error;
pub mod events;
pub mod library;
pub mod mcp;
```

**with:**

```rust
pub mod error;
pub mod events;
pub mod kit;
pub mod library;
pub mod mcp;
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
                             child process in a Job Object, or Streamable HTTP), `${VAR}`
                             expansion, tool-list fingerprints (MCP/skills spec §5)
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
```

**with:**

```markdown
                             child process in a Job Object, or Streamable HTTP), `${VAR}`
                             expansion, tool-list fingerprints (MCP/skills spec §5)
    src/kit/                 skills and plugins on disk: `SKILL.md` frontmatter, skill and plugin
                             reading, Claude Code's installed plugins, and a Claude-engine run's
                             plugin folders (copies without `.mcp.json`, `monitors/`, hooks
                             unless allowed, or `allowed-tools`; MCP/skills spec §7)
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p dots-core --test kit_run_test --test kit_test 2>&1 | grep -E "^test result|FAILED"
```

Expected: `kit_run_test` 15 passed (on Windows; one more test is for Unix), `kit_test` 32 passed.

- [ ] **Step 5: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

Expected: `dots-core` 530 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent. The UI is unchanged (292 in 21 files). In the measurement of this commit `claude_engine_test::a_finished_run_leaves_no_process_behind` failed once (529 passed, 1 failed) and passed at every other commit: it is a known flaky test; run it alone.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/kit/frontmatter.rs crates/dots-core/src/kit/installed.rs crates/dots-core/src/kit/mod.rs crates/dots-core/src/kit/plugin.rs crates/dots-core/src/kit/run_dirs.rs crates/dots-core/src/kit/skill.rs crates/dots-core/src/lib.rs crates/dots-core/tests/kit_run_test.rs crates/dots-core/tests/kit_test.rs docs/superpowers/specs/2026-10-08-dots-design.md
git commit -m "feat(core): read skills and plugins from disk and prepare a run's plugin folders"
```

### Task 5: Open a run's MCP sessions, skills and plugin folders, and serve the Library at runtime

The runtime joins Tasks 2 to 4. A `LibraryService`, reached through `Runtime::library()`, holds the commands of spec section 9: list, add (a server is accepted only under the fingerprint the person was shown), update, remove (refused while dots use the item unless confirmed), probe, check, accept, scan skills, installed plugins and environment variables; a plugin's own server items are kept in line with its folder whenever it is read; every catalog change publishes `RuntimeEvent::LibraryUpdated`. At run start the runner opens a `RunTools` for the dot: it refreshes the plugins (finding a moved installed plugin again), opens a session with every used server at once in the run's workspace, records each check, holds a server whose list differs from the accepted one, reads the skills, and for a Claude run prepares the plugin folders; what is left out becomes a run event of a new kind, `notice`. While the run lasts, a watcher per session lists again on each list change and holds the server at once; at the end the sessions close and the folders go, and at start Dots deletes the folders a crash left behind. The Inbox gains `tools_changed`, counted by `inbox_count`, the sidebar badge and the tray. The engines receive the tools but ignore them until Tasks 6 and 7. The main spec's module tree, its Inbox passages and its read methods follow.

**Files:**
- Create: `crates/dots-core/src/library_service.rs`, `crates/dots-core/src/run_tools.rs`, `crates/dots-core/tests/library_service_test.rs`, `crates/dots-core/tests/run_tools_test.rs`
- Modify: `apps/dots-app/src/contract.rs`, `crates/dots-core/src/engine/mod.rs`, `crates/dots-core/src/events.rs`, `crates/dots-core/src/kit/installed.rs`, `crates/dots-core/src/lib.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/runner.rs`, `crates/dots-core/src/runtime.rs`, `crates/dots-core/src/store/library.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`, `ui/src/app/Layout.tsx`, `ui/src/lib/transcript.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/claude_cli_test.rs`, `crates/dots-core/tests/claude_engine_test.rs`, `crates/dots-core/tests/common/fake_mcp.rs`, `crates/dots-core/tests/engine_prompt_test.rs`, `crates/dots-core/tests/library_service_test.rs`, `crates/dots-core/tests/local_engine_test.rs`, `crates/dots-core/tests/resume_test.rs`, `crates/dots-core/tests/run_tools_test.rs`, `crates/dots-core/tests/runner_test.rs`, `crates/dots-core/tests/scheduler_test.rs`, `crates/dots-core/tests/server_test.rs`, `ui/src/app/Layout.test.tsx`, `ui/src/lib/runtime-events.test.ts`, `ui/src/lib/transcript.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`, `ui/src/screens/InboxScreen.test.tsx`, `ui/src/test/fixtures.ts`

**Interfaces:**
- Consumes: Task 2's store methods and types, Task 3's `McpSession`, `fingerprint` and `ANSWER_TIMEOUT`, Task 4's `read_skill`, `read_plugin`, `scan_skills`, `installed_plugins`, `find_installed` and `prepare_run_kit`, Task 1's `McpTrust`; the runner's recorder and its cancel token.
- Produces:
  - `RuntimeEvent::LibraryUpdated { item_id: String }` (JSON `{"type": "library_updated", "item_id": …}`). `EngineEvent::Notice { text: String }` (kind `notice`). `RunContext.tools: Arc<RunTools>` (Task 2's `mcp_servers` stays until Task 7).
  - `DotRef { id, name }`; `ToolsChanged { server_id, name, dots: Vec<DotRef> }`; `Inbox.tools_changed: Vec<ToolsChanged>` (by server name); `inbox_count` counts it.
  - `Store`: `record_catalog_seen(&self, server_id: &str, fingerprint: &str, tools: &Value) -> Result<McpCatalog>` (status `accepted` when the fingerprint equals the accepted one, else `changed`; one `UPDATE … RETURNING`, so a check never undoes a concurrent accept); `record_catalog_failure(&self, server_id: &str, error: &str) -> Result<McpCatalog>` (`unreachable`, the lists kept); `owned_library_items(&self, owner_id: &str) -> Result<Vec<LibraryItem>>`; `changed_mcp_servers(&self) -> Result<Vec<LibraryItem>>`.
  - `LibraryService::new(store: Store, bus: Bus)` and `Runtime::library(&self) -> &LibraryService`, with `list(&self) -> Result<Vec<LibraryEntry>>`; `add(&self, name: &str, config: LibraryConfig, accept: Option<&str>) -> Result<LibraryItem>`; `update(&self, id: &str, config: LibraryConfig) -> Result<LibraryItem>`; `remove(&self, id: &str, confirm: bool) -> Result<()>` (`Conflict("the skill \"pdf\" is used by audit, nightly; removing it takes it out of them")` without `confirm`); `probe(&self, config: &McpServerConfig) -> Result<McpProbe>`; `check(&self, id: &str) -> Result<McpCatalog>`; `accept(&self, id: &str, fingerprint: &str) -> Result<McpCatalog>`; `scan_skills(&self, path: &str) -> Result<Vec<ScannedSkill>>`; `installed_plugins(&self) -> Vec<InstalledPlugin>`; `environment_variables(&self, names: &[String]) -> Vec<EnvVariable>`; `tools_changed(&self) -> Result<Vec<ToolsChanged>>`. An owned server cannot be changed or removed on its own (`Invalid`); a skill or plugin folder must be readable to be added (`NotFound`).
  - Views (`Serialize`): `LibraryEntry { #[serde(flatten)] item: LibraryItem, used_by: Vec<DotRef>, catalog: Option<McpCatalog>, missing_variables: Vec<String>, skill: Option<SkillDetail>, plugin: Option<PluginDetail>, problem: Option<String> }`; `SkillDetail { description }`; `PluginDetail { version, description: Option<String>, skills: Vec<String> /* "<plugin>:<skill>" */, commands, agents, servers: Vec<String>, has_hooks: bool, warnings: Vec<String> }`; `McpProbe { tools: Vec<McpTool>, fingerprint: String }`; `ScannedSkill { name, description, path, name_error: Option<String> }`; `EnvVariable { name, set: bool }`. `InstalledPlugin` becomes `Serialize`.
  - In `dots_core::run_tools`: `OpenRequest<'a> { store, bus, run_id, uses: &Uses, workspace: &Path, plugins_dir: Option<&Path>, cancel: &CancellationToken }`; `RunTools` (`Default` is nothing open) with `open(req: OpenRequest<'_>) -> Result<(RunTools, Vec<String>)>` (the second part is the notices), `servers() -> &[RunServer]`, `server(name)`, `resolve<'a>(&self, qualified: &'a str) -> Option<(&RunServer, &'a str)>`, `trust(server, tool) -> Option<McpTrust>`, `skills() -> &[RunSkill]`, `skill(name)`, `plugin_dirs() -> &[PathBuf]`, `close(&self)` (idempotent); `RunServer` with `item_id()`, `name()`, `session() -> &Arc<McpSession>`, `tools() -> Vec<McpTool>` (the latest list), `held() -> bool` (sticky for the run), `accepted_fingerprint() -> Option<String>` (as of run start), `trust(&self, tool: &str) -> McpTrust`; `RunSkill { name: String, info: SkillInfo, plugin: Option<String> }`; `CloseOnDrop(pub Arc<RunTools>)`; `sweep_plugin_folders(plugins_dir: &Path)`.
  - `RunnerConfig.plugins_dir: PathBuf` (`<data_dir>/plugins`). The runner opens the tools after the workspace is ready (plugin folders only for a Claude dot), records the notices before the engine starts, and closes the tools before the run's status is written.
  - Notices: `<server>: <McpError>`, `<server>: its tool list changed since it was accepted; its tools ask until the change is accepted in the Library`, `… has not been accepted yet …`, `skill <name>: folder not found`, `plugin <name>: <reason or warning>`.
  - Contract: an `EngineEvent` sample `Notice` and an `Inbox.tools_changed` sample. UI: `RunEventData` `notice`, `T.DotRef`, `T.ToolsChanged`, `Inbox.tools_changed`; the transcript shows a notice as a raw item labelled "Note"; the sidebar badge adds `tools_changed.length`. Test helper: `FakeMcp::rewrite(script)`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
            ),
        ],
    }
}
```

**with:**

```rust
            ),
        ],
        tools: Default::default(),
    }
}
```

**In `crates/dots-core/tests/claude_engine_test.rs`, replace:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_secs(2),
        },
    );
```

**with:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_secs(2),
            plugins_dir: dir.path().join("plugins"),
        },
    );
```

**In `crates/dots-core/tests/common/fake_mcp.rs`, replace:**

```rust
        std::fs::write(dir.path().join("script.json"), script.to_string()).unwrap();
        Self { dir }
    }

```

**with:**

```rust
        std::fs::write(dir.path().join("script.json"), script.to_string()).unwrap();
        Self { dir }
    }

    /// Replaces the script, keeping its log and lock files: a server started after this lists
    /// what `script` says.
    pub fn rewrite(&self, mut script: Value) {
        script["log"] = json!(self.dir.path().join("log.jsonl"));
        script["alive"] = json!(self.dir.path().join("alive"));
        std::fs::write(self.script(), script.to_string()).unwrap();
    }

```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
        approved_calls: Vec::new(),
        mcp_servers: Vec::new(),
    }
}
```

**with:**

```rust
        approved_calls: Vec::new(),
        mcp_servers: Vec::new(),
        tools: Default::default(),
    }
}
```

**Create `crates/dots-core/tests/library_service_test.rs`:**

```rust
//! The Library's runtime service (MCP/skills spec section 9): add, update and remove with the
//! confirm rule, probe, check and accept with the fingerprint guard, a plugin's own MCP
//! servers kept in line, scans, installed plugins, variables, and `LibraryUpdated`.

mod common;

use std::path::Path;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, McpServerConfig, PluginConfig, SkillConfig,
};
use dots_core::library_service::LibraryService;
use dots_core::mcp::fingerprint;
use dots_core::store::Store;
use dots_core::Error;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast;

fn service(store: &Store, bus: &Bus) -> LibraryService {
    LibraryService::new(store.clone(), bus.clone())
}

fn server(fake: &FakeMcp) -> LibraryConfig {
    LibraryConfig::McpServer(fake.stdio())
}

/// The item ids of the `LibraryUpdated` events waiting on `rx`.
fn updated(rx: &mut broadcast::Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::LibraryUpdated { item_id } = ev {
            ids.push(item_id);
        }
    }
    ids
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn skill_dir(root: &Path, name: &str, description: &str) -> String {
    let dir = root.join(name);
    write(
        &dir.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: {description}\n---\nBody.\n"),
    );
    dir.to_string_lossy().into_owned()
}

/// A plugin folder named `name` with one skill and the MCP servers `servers` (name to script).
fn plugin_dir(root: &Path, name: &str, servers: &[(&str, &FakeMcp)]) -> String {
    let dir = root.join(name);
    write(
        &dir.join(".claude-plugin").join("plugin.json"),
        &json!({"name": name, "version": "1.0.0"}).to_string(),
    );
    write(
        &dir.join("skills").join("fill").join("SKILL.md"),
        "---\nname: fill\ndescription: Fills forms.\n---\nBody.\n",
    );
    let mut map = serde_json::Map::new();
    for (server, fake) in servers {
        map.insert(
            server.to_string(),
            json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [fake.script()]}),
        );
    }
    write(
        &dir.join(".mcp.json"),
        &json!({ "mcpServers": map }).to_string(),
    );
    dir.to_string_lossy().into_owned()
}

fn plugin(path: &str) -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: path.into(),
        installed_id: None,
        run_hooks: false,
    })
}

#[tokio::test]
async fn probe_lists_the_tools_with_their_fingerprint_and_starts_nothing_that_stays() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    let names: Vec<&str> = probe.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["read", "write"]);
    assert_eq!(probe.fingerprint, fingerprint(&probe.tools));
    assert!(fake.gone_within(std::time::Duration::from_secs(10)).await);
    assert!(store.list_library_items().await.unwrap().is_empty());

    let missing = McpServerConfig::Stdio {
        command: env!("CARGO_BIN_EXE_fake-mcp").into(),
        args: vec!["${DOTS_T5_NEVER_SET_PROBE}".into()],
        cwd: None,
        env: Default::default(),
    };
    let err = lib.probe(&missing).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "could not start: DOTS_T5_NEVER_SET_PROBE is not set"
    );
}

#[tokio::test]
async fn adding_a_server_with_the_probed_fingerprint_accepts_that_list() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let mut rx = bus.subscribe();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    let item = lib
        .add("files", server(&fake), Some(&probe.fingerprint))
        .await
        .unwrap();
    let catalog = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Accepted);
    assert_eq!(
        catalog.accepted_fingerprint.as_deref(),
        Some(probe.fingerprint.as_str())
    );
    assert!(catalog.checked_at.is_some());
    assert!(updated(&mut rx).contains(&item.id));
}

#[tokio::test]
async fn a_server_added_without_a_fingerprint_stays_unchecked_and_is_not_started() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let item = lib.add("files", server(&fake), None).await.unwrap();
    assert_eq!(
        store.get_catalog(&item.id).await.unwrap().status,
        CatalogStatus::Unchecked
    );
    assert!(fake.received("initialize").is_empty());
}

#[tokio::test]
async fn adding_with_a_fingerprint_that_moved_leaves_the_list_to_review() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let item = lib
        .add("files", server(&fake), Some("not-what-it-lists"))
        .await
        .unwrap();
    let catalog = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Changed);
    assert_eq!(catalog.accepted_fingerprint, None);
    assert!(catalog.seen_fingerprint.is_some());

    let skill = LibraryConfig::Skill(SkillConfig {
        path: "C:/x".into(),
    });
    let err = lib.add("pdf", skill, Some("fp")).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
}

#[tokio::test]
async fn check_records_a_changed_list_and_accept_guards_the_fingerprint() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let first = lib.probe(&fake.stdio()).await.unwrap();
    let item = lib
        .add("files", server(&fake), Some(&first.fingerprint))
        .await
        .unwrap();

    // The same list: still accepted.
    let same = lib.check(&item.id).await.unwrap();
    assert_eq!(same.status, CatalogStatus::Accepted);

    // The server now lists another tool.
    fake.rewrite(json!({"tools": [tool("read", true), tool("drop", false)]}));
    let mut rx = bus.subscribe();
    let changed = lib.check(&item.id).await.unwrap();
    assert_eq!(changed.status, CatalogStatus::Changed);
    assert_eq!(
        changed.accepted_fingerprint.as_deref(),
        Some(first.fingerprint.as_str())
    );
    let seen = changed.seen_fingerprint.clone().unwrap();
    assert_ne!(seen, first.fingerprint);
    assert_eq!(updated(&mut rx), vec![item.id.clone()]);

    let err = lib.accept(&item.id, &first.fingerprint).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err}");
    let accepted = lib.accept(&item.id, &seen).await.unwrap();
    assert_eq!(accepted.status, CatalogStatus::Accepted);
    assert_eq!(accepted.accepted_fingerprint, Some(seen));
    assert_eq!(updated(&mut rx), vec![item.id.clone()]);
}

#[tokio::test]
async fn a_failed_check_is_unreachable_and_keeps_the_accepted_list() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    let item = lib
        .add("files", server(&fake), Some(&probe.fingerprint))
        .await
        .unwrap();
    let broken = LibraryConfig::McpServer(fake.stdio_with(
        vec!["${DOTS_T5_NEVER_SET_CHECK}".into()],
        Default::default(),
    ));
    lib.update(&item.id, broken).await.unwrap();
    assert_eq!(
        store.get_catalog(&item.id).await.unwrap().status,
        CatalogStatus::Unchecked
    );
    let catalog = lib.check(&item.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unreachable);
    assert_eq!(
        catalog.error.as_deref(),
        Some("could not start: DOTS_T5_NEVER_SET_CHECK is not set")
    );
    assert_eq!(
        catalog.accepted_fingerprint.as_deref(),
        Some(probe.fingerprint.as_str())
    );
}

#[tokio::test]
async fn only_an_mcp_server_is_checked_or_accepted() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add("pdf", LibraryConfig::Skill(SkillConfig { path }), None)
        .await
        .unwrap();
    assert!(matches!(
        lib.check(&skill.id).await.unwrap_err(),
        Error::Invalid(_)
    ));
    assert!(matches!(
        lib.accept(&skill.id, "fp").await.unwrap_err(),
        Error::Invalid(_)
    ));
}

#[tokio::test]
async fn removing_an_item_that_dots_use_asks_first_and_names_them() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add("pdf", LibraryConfig::Skill(SkillConfig { path }), None)
        .await
        .unwrap();
    for name in ["nightly", "audit"] {
        let mut s = spec(name);
        s.uses.skills = vec!["pdf".into()];
        store.create_dot(&s).await.unwrap();
    }
    let err = lib.remove(&skill.id, false).await.unwrap_err();
    match &err {
        Error::Conflict(text) => {
            assert!(text.contains("audit") && text.contains("nightly"), "{text}")
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert!(store.get_library_item(&skill.id).await.is_ok());

    let mut rx = bus.subscribe();
    lib.remove(&skill.id, true).await.unwrap();
    assert!(store.get_library_item(&skill.id).await.is_err());
    for dot in store.list_dots().await.unwrap() {
        assert!(dot.spec.uses.skills.is_empty());
    }
    assert_eq!(updated(&mut rx), vec![skill.id.clone()]);

    // An item no dot uses goes without asking.
    let other = skill_dir(dir.path(), "csv", "Reads CSV.");
    let csv = lib
        .add(
            "csv",
            LibraryConfig::Skill(SkillConfig { path: other }),
            None,
        )
        .await
        .unwrap();
    lib.remove(&csv.id, false).await.unwrap();
}

#[tokio::test]
async fn a_skill_or_plugin_must_be_readable_to_be_added() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let missing = dir.path().join("nope").to_string_lossy().into_owned();
    let err = lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig {
                path: missing.clone(),
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
    let err = lib.add("tools", plugin(&missing), None).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
    assert!(store.list_library_items().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_plugins_mcp_servers_are_kept_in_line_with_its_folder() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let a = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let b = FakeMcp::new(json!({"tools": [tool("b", true)]}));
    let path = plugin_dir(dir.path(), "forms", &[("alpha", &a), ("beta", &b)]);
    let item = lib.add("forms", plugin(&path), None).await.unwrap();
    let owned = |items: Vec<dots_core::library::LibraryItem>| -> Vec<String> {
        items
            .into_iter()
            .filter(|i| i.owner_id.as_deref() == Some(item.id.as_str()))
            .map(|i| i.name)
            .collect()
    };
    assert_eq!(
        owned(store.list_library_items().await.unwrap()),
        ["plugin_forms_alpha", "plugin_forms_beta"]
    );
    let beta = store
        .find_library_item(LibraryKind::McpServer, "plugin_forms_beta")
        .await
        .unwrap()
        .unwrap();
    let alpha = store
        .find_library_item(LibraryKind::McpServer, "plugin_forms_alpha")
        .await
        .unwrap()
        .unwrap();
    // Accept beta's list, so that a change of its config shows as a reset.
    let probe = lib.probe(&b.stdio()).await.unwrap();
    lib.check(&beta.id).await.unwrap();
    lib.accept(&beta.id, &probe.fingerprint).await.unwrap();

    // The folder changes: alpha goes, beta gets an argument, gamma comes.
    let c = FakeMcp::new(json!({"tools": [tool("c", true)]}));
    let mut servers = serde_json::Map::new();
    servers.insert(
        "beta".into(),
        json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [b.script(), "--more"]}),
    );
    servers.insert(
        "gamma".into(),
        json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [c.script()]}),
    );
    write(
        &Path::new(&path).join(".mcp.json"),
        &json!({ "mcpServers": servers }).to_string(),
    );
    let mut rx = bus.subscribe();
    let switched = LibraryConfig::Plugin(PluginConfig {
        path: path.clone(),
        installed_id: None,
        run_hooks: true,
    });
    lib.update(&item.id, switched).await.unwrap();
    assert_eq!(
        owned(store.list_library_items().await.unwrap()),
        ["plugin_forms_beta", "plugin_forms_gamma"]
    );
    assert!(store.get_library_item(&alpha.id).await.is_err());
    let beta_now = store.get_library_item(&beta.id).await.unwrap();
    assert_eq!(beta_now.id, beta.id, "a server that stays keeps its item");
    assert_eq!(
        store.get_catalog(&beta.id).await.unwrap().status,
        CatalogStatus::Unchecked
    );
    let ids = updated(&mut rx);
    assert!(ids.contains(&item.id) && ids.contains(&alpha.id) && ids.contains(&beta.id));

    // A plugin's server is changed through its plugin only.
    let err = lib.update(&beta.id, server(&b)).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    let err = lib.remove(&beta.id, true).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");

    // Removing the plugin removes its servers.
    lib.remove(&item.id, false).await.unwrap();
    assert!(owned(store.list_library_items().await.unwrap()).is_empty());
}

#[tokio::test]
async fn the_list_says_who_uses_an_item_and_what_it_holds() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let needs = LibraryConfig::McpServer(fake.stdio_with(
        Vec::new(),
        [("TOKEN".to_string(), "${DOTS_T5_NEVER_SET_LIST}".to_string())].into(),
    ));
    lib.add("files", needs, None).await.unwrap();
    let skill_path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig {
                path: skill_path.clone(),
            }),
            None,
        )
        .await
        .unwrap();
    let pfake = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let ppath = plugin_dir(dir.path(), "forms", &[("alpha", &pfake)]);
    lib.add("forms", plugin(&ppath), None).await.unwrap();
    let mut s = spec("nightly");
    s.uses.mcp_servers = vec!["files".into()];
    s.uses.plugins = vec!["forms".into()];
    let dot = store.create_dot(&s).await.unwrap();
    // The skill's folder goes away after it was added.
    std::fs::remove_dir_all(&skill_path).unwrap();

    let list = lib.list().await.unwrap();
    let entry = |name: &str| list.iter().find(|e| e.item.name == name).unwrap();
    let files = entry("files");
    assert_eq!(files.used_by.len(), 1);
    assert_eq!(files.used_by[0].id, dot.id);
    assert_eq!(files.used_by[0].name, "nightly");
    assert_eq!(files.missing_variables, ["DOTS_T5_NEVER_SET_LIST"]);
    assert_eq!(
        files.catalog.as_ref().unwrap().status,
        CatalogStatus::Unchecked
    );
    let owned = entry("plugin_forms_alpha");
    assert_eq!(owned.used_by[0].name, "nightly", "used through its plugin");
    let forms = entry("forms");
    let parts = forms.plugin.as_ref().unwrap();
    assert_eq!(parts.skills, ["forms:fill"]);
    assert_eq!(parts.servers, ["plugin_forms_alpha"]);
    assert!(!parts.has_hooks);
    assert_eq!(parts.version.as_deref(), Some("1.0.0"));
    assert!(forms.problem.is_none());
    let pdf = entry("pdf");
    assert_eq!(pdf.item.id, skill.id);
    assert!(pdf.used_by.is_empty());
    assert_eq!(pdf.problem.as_deref(), Some("folder not found"));
    assert!(pdf.skill.is_none());
    // Serialized flat, as the UI reads it.
    let v: Value = serde_json::to_value(files).unwrap();
    assert_eq!(v["kind"], "mcp_server");
    assert_eq!(v["name"], "files");
    assert_eq!(v["catalog"]["status"], "unchecked");
}

#[tokio::test]
async fn a_skill_entry_carries_its_description() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    lib.add("pdf", LibraryConfig::Skill(SkillConfig { path }), None)
        .await
        .unwrap();
    let list = lib.list().await.unwrap();
    assert_eq!(list[0].skill.as_ref().unwrap().description, "Reads PDFs.");
}

#[tokio::test]
async fn scanning_a_folder_lists_its_skills_and_flags_bad_names() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    skill_dir(dir.path(), "pdf", "Reads PDFs.");
    write(
        &dir.path().join("odd").join("SKILL.md"),
        "---\nname: \"has space\"\ndescription: Odd.\n---\n",
    );
    let found = lib
        .scan_skills(&dir.path().to_string_lossy())
        .await
        .unwrap();
    let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["has space", "pdf"]);
    assert!(found[0].name_error.is_some());
    assert_eq!(found[1].name_error, None);
    assert_eq!(found[1].description, "Reads PDFs.");
    assert!(found[1].path.ends_with("pdf"));
}

#[tokio::test]
async fn environment_variables_say_which_are_set_and_never_their_values() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    std::env::set_var("DOTS_T5_SET_VAR", "secret-value");
    let vars = lib.environment_variables(&[
        "DOTS_T5_SET_VAR".to_string(),
        "DOTS_T5_NEVER_SET_ENV".to_string(),
    ]);
    let v = serde_json::to_value(&vars).unwrap();
    assert_eq!(
        v,
        json!([{"name": "DOTS_T5_SET_VAR", "set": true},
               {"name": "DOTS_T5_NEVER_SET_ENV", "set": false}])
    );
    assert!(!v.to_string().contains("secret-value"));
}

#[tokio::test]
async fn installed_plugins_are_read_from_the_cache_folder() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let install = dir.path().join("cache").join("forms");
    std::fs::create_dir_all(&install).unwrap();
    write(
        &dir.path().join("installed_plugins.json"),
        &json!({"plugins": {"forms@mkt": [{"scope": "user", "installPath": install,
                                           "version": "1.0.0"}]}})
        .to_string(),
    );
    std::env::set_var("CLAUDE_CODE_PLUGIN_CACHE_DIR", dir.path());
    let found = lib.installed_plugins();
    std::env::remove_var("CLAUDE_CODE_PLUGIN_CACHE_DIR");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "forms@mkt");
    assert!(found[0].exists);
    let v = serde_json::to_value(&found[0]).unwrap();
    assert_eq!(v["install_path"], json!(install));
}

async fn runtime(dir: &TempDir) -> dots_core::Runtime {
    let mut cfg = dots_core::Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(env!("CARGO_BIN_EXE_fake-claude").into());
    dots_core::Runtime::start(cfg).await.unwrap()
}

#[tokio::test]
async fn the_inbox_lists_servers_whose_tools_changed_and_counts_them() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let item = rt
        .library()
        .add("files", server(&fake), Some("an-old-list"))
        .await
        .unwrap();
    let mut s = spec("nightly");
    s.uses.mcp_servers = vec!["files".into()];
    rt.create_dot(s).await.unwrap();
    let inbox = rt.inbox().await.unwrap();
    assert_eq!(inbox.tools_changed.len(), 1);
    let changed = &inbox.tools_changed[0];
    assert_eq!(changed.server_id, item.id);
    assert_eq!(changed.name, "files");
    assert_eq!(changed.dots.len(), 1);
    assert_eq!(changed.dots[0].name, "nightly");
    assert_eq!(rt.inbox_count().await.unwrap(), 1);

    let seen = rt
        .store()
        .get_catalog(&item.id)
        .await
        .unwrap()
        .seen_fingerprint
        .unwrap();
    rt.library().accept(&item.id, &seen).await.unwrap();
    assert!(rt.inbox().await.unwrap().tools_changed.is_empty());
    assert_eq!(rt.inbox_count().await.unwrap(), 0);
    rt.shutdown();
}

#[tokio::test]
async fn starting_dots_removes_plugin_folders_a_crash_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let left = dir.path().join("data").join("plugins").join("run-01OLD");
    write(&left.join("dots-skills").join("x.txt"), "x");
    let keep = dir.path().join("data").join("plugins").join("other");
    std::fs::create_dir_all(&keep).unwrap();
    let rt = runtime(&dir).await;
    assert!(!left.exists());
    assert!(keep.exists());
    rt.shutdown();
}
```

**In `crates/dots-core/tests/local_engine_test.rs`, replace:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
        },
    );
```

**with:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
            plugins_dir: dir.path().join("plugins"),
        },
    );
```

**In `crates/dots-core/tests/local_engine_test.rs`, replace:**

```rust
        approved_calls: approvals,
        mcp_servers: Vec::new(),
    };
    let mut rx = engine.start(ctx).await.unwrap();
```

**with:**

```rust
        approved_calls: approvals,
        mcp_servers: Vec::new(),
        tools: Default::default(),
    };
    let mut rx = engine.start(ctx).await.unwrap();
```

**In `crates/dots-core/tests/resume_test.rs`, replace:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
        },
    );
```

**with:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
        },
    );
```

**Create `crates/dots-core/tests/run_tools_test.rs`:**

```rust
//! A run's tools (MCP/skills spec sections 5 to 7): the MCP sessions of the servers a dot uses,
//! its plugins' servers included, checked against their catalogs and held when their list
//! changed; the skills it reads; the plugin folders of a Claude-engine run; the items left out;
//! and the close at run end.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::fake_mcp::{tool, FakeMcp};
use common::temp_store;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::LibraryService;
use dots_core::policy::McpTrust;
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::store::Store;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn skill_dir(root: &Path, name: &str) -> String {
    let dir = root.join(name);
    write(
        &dir.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: The {name} skill.\n---\nBody.\n"),
    );
    dir.to_string_lossy().into_owned()
}

fn plugin_dir(root: &Path, name: &str, servers: &[(&str, &FakeMcp)]) -> String {
    let dir = root.join(name);
    write(
        &dir.join(".claude-plugin").join("plugin.json"),
        &json!({ "name": name }).to_string(),
    );
    write(
        &dir.join("skills").join("fill").join("SKILL.md"),
        "---\nname: fill\ndescription: Fills forms.\n---\nBody.\n",
    );
    let mut map = serde_json::Map::new();
    for (server, fake) in servers {
        map.insert(
            server.to_string(),
            json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [fake.script()]}),
        );
    }
    write(
        &dir.join(".mcp.json"),
        &json!({ "mcpServers": map }).to_string(),
    );
    dir.to_string_lossy().into_owned()
}

fn plugin(path: &str, installed_id: Option<&str>) -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: path.into(),
        installed_id: installed_id.map(str::to_string),
        run_hooks: false,
    })
}

struct Fixture {
    dir: TempDir,
    store: Store,
    bus: Bus,
    lib: LibraryService,
}

async fn fixture() -> Fixture {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = LibraryService::new(store.clone(), bus.clone());
    Fixture {
        dir,
        store,
        bus,
        lib,
    }
}

impl Fixture {
    /// Adds the server `name` on `fake`'s script with its current list accepted.
    async fn accepted_server(&self, name: &str, fake: &FakeMcp) -> String {
        let probe = self.lib.probe(&fake.stdio()).await.unwrap();
        self.lib
            .add(
                name,
                LibraryConfig::McpServer(fake.stdio()),
                Some(&probe.fingerprint),
            )
            .await
            .unwrap()
            .id
    }

    async fn open(&self, uses: &Uses, plugins_dir: Option<&Path>) -> (RunTools, Vec<String>) {
        RunTools::open(OpenRequest {
            store: &self.store,
            bus: &self.bus,
            run_id: "01RUN",
            uses,
            workspace: self.dir.path(),
            plugins_dir,
            cancel: &CancellationToken::new(),
        })
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_run_opens_its_servers_and_its_plugins_servers() {
    let f = fixture().await;
    let files = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let files_id = f.accepted_server("files", &files).await;
    let alpha = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let path = plugin_dir(f.dir.path(), "forms", &[("alpha", &alpha)]);
    f.lib.add("forms", plugin(&path, None), None).await.unwrap();
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        skills: vec![],
        plugins: vec!["forms".into()],
    };
    let (tools, notices) = f.open(&uses, None).await;

    let names: Vec<&str> = tools.servers().iter().map(|s| s.name()).collect();
    assert_eq!(names, ["files", "plugin_forms_alpha"]);
    let srv = tools.server("files").unwrap();
    assert_eq!(srv.item_id(), files_id);
    assert!(!srv.held());
    let listed: Vec<String> = srv.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(listed, ["read", "write"]);
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: false,
            read_only: true
        }
    );
    assert_eq!(
        tools.trust("files", "write"),
        Some(McpTrust {
            held: false,
            read_only: false
        })
    );
    // A tool the accepted list does not have asks (spec section 4, step 2).
    assert!(srv.trust("other").held);
    assert_eq!(tools.trust("nope", "read"), None);
    assert_eq!(
        srv.accepted_fingerprint(),
        f.store
            .get_catalog(&files_id)
            .await
            .unwrap()
            .accepted_fingerprint
    );

    // The plugin's server was never accepted: it is held, and the run says so.
    let owned = tools.server("plugin_forms_alpha").unwrap();
    assert!(owned.held());
    assert!(owned.trust("a").held);
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(
        notices[0].starts_with("plugin_forms_alpha: "),
        "{notices:?}"
    );

    // The session works.
    let result = srv
        .session()
        .call_tool(
            "read",
            json!({"q": "x"}),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("read"));

    // The local engine reads skills from the Library folders; no plugin folders are made.
    let skills: Vec<&str> = tools.skills().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skills, ["forms:fill"]);
    assert_eq!(tools.skills()[0].plugin.as_deref(), Some("forms"));
    assert!(tools.plugin_dirs().is_empty());

    tools.close().await;
    assert!(files.gone_within(Duration::from_secs(10)).await);
    assert!(alpha.gone_within(Duration::from_secs(10)).await);
    tools.close().await;
}

#[tokio::test]
async fn a_list_that_changed_since_it_was_accepted_holds_the_server() {
    let f = fixture().await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let id = f.accepted_server("files", &fake).await;
    fake.rewrite(json!({"tools": [tool("read", true), tool("drop", false)]}));
    let mut rx = f.bus.subscribe();
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    let srv = tools.server("files").unwrap();
    assert!(srv.held());
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: true,
            read_only: true
        }
    );
    let catalog = f.store.get_catalog(&id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Changed);
    assert_eq!(notices.len(), 1);
    assert!(notices[0].starts_with("files: "), "{notices:?}");
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::LibraryUpdated { item_id } = ev {
            ids.push(item_id);
        }
    }
    assert_eq!(ids, vec![id]);
    tools.close().await;
}

#[tokio::test]
async fn a_server_that_cannot_start_is_left_out_with_a_run_event() {
    let f = fixture().await;
    let good = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    f.accepted_server("good", &good).await;
    let bad = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let bad_cfg = LibraryConfig::McpServer(
        bad.stdio_with(vec!["${DOTS_T5_NEVER_SET_RUN}".into()], Default::default()),
    );
    let bad_id = f.lib.add("bad", bad_cfg, None).await.unwrap().id;
    let uses = Uses {
        mcp_servers: vec!["bad".into(), "good".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    let names: Vec<&str> = tools.servers().iter().map(|s| s.name()).collect();
    assert_eq!(names, ["good"]);
    assert_eq!(
        notices,
        ["bad: could not start: DOTS_T5_NEVER_SET_RUN is not set"]
    );
    let catalog = f.store.get_catalog(&bad_id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unreachable);
    assert_eq!(
        catalog.error.as_deref(),
        Some("could not start: DOTS_T5_NEVER_SET_RUN is not set")
    );
    tools.close().await;
}

#[tokio::test]
async fn a_claude_run_gets_plugin_folders_that_go_at_close() {
    let f = fixture().await;
    let pdf = skill_dir(f.dir.path(), "pdf");
    let csv = skill_dir(f.dir.path(), "csv");
    f.lib
        .add("pdf", LibraryConfig::Skill(SkillConfig { path: pdf }), None)
        .await
        .unwrap();
    f.lib
        .add(
            "csv",
            LibraryConfig::Skill(SkillConfig { path: csv.clone() }),
            None,
        )
        .await
        .unwrap();
    std::fs::remove_dir_all(&csv).unwrap();
    let path = plugin_dir(f.dir.path(), "forms", &[]);
    f.lib.add("forms", plugin(&path, None), None).await.unwrap();
    let uses = Uses {
        mcp_servers: vec![],
        skills: vec!["csv".into(), "pdf".into()],
        plugins: vec!["forms".into()],
    };
    let plugins_dir = f.dir.path().join("plugins");
    let (tools, notices) = f.open(&uses, Some(&plugins_dir)).await;
    assert_eq!(notices, ["skill csv: folder not found"]);
    let run_root = plugins_dir.join("run-01RUN");
    assert_eq!(
        tools.plugin_dirs(),
        [run_root.join("dots-skills"), run_root.join("forms")]
    );
    assert!(run_root.join("dots-skills/skills/pdf/SKILL.md").is_file());
    let skills: Vec<&str> = tools.skills().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skills, ["pdf", "forms:fill"]);
    assert_eq!(tools.skill("pdf").unwrap().plugin, None);
    tools.close().await;
    assert!(!run_root.exists());
}

#[tokio::test]
async fn a_list_change_during_the_run_holds_the_server_at_once() {
    let f = fixture().await;
    let mut flip = tool("flip", false);
    flip["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "tools": [tool("read", true), flip],
        "after_switch": [tool("read", true), tool("new", true)],
    }));
    let id = f.accepted_server("files", &fake).await;
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    assert!(notices.is_empty(), "{notices:?}");
    let srv = tools.server("files").unwrap();
    assert!(!srv.held());
    srv.session()
        .call_tool(
            "flip",
            json!({}),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !srv.held() {
        assert!(Instant::now() < deadline, "the server was not held");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let listed: Vec<String> = srv.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(listed, ["read", "new"]);
    // `read` keeps the accepted list's read-only hint; `new` is not in it.
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: true,
            read_only: true
        }
    );
    assert_eq!(
        f.store.get_catalog(&id).await.unwrap().status,
        CatalogStatus::Changed
    );
    tools.close().await;
}

#[tokio::test]
async fn an_installed_plugin_that_moved_is_found_again() {
    let f = fixture().await;
    let old = plugin_dir(f.dir.path(), "forms-1.0", &[]);
    f.lib
        .add("forms", plugin(&old, Some("forms@mkt")), None)
        .await
        .unwrap();
    std::fs::remove_dir_all(&old).unwrap();
    let new = plugin_dir(f.dir.path(), "forms-1.1", &[]);
    let cache = f.dir.path().join("cache");
    write(
        &cache.join("installed_plugins.json"),
        &json!({"plugins": {"forms@mkt": [{"scope": "user", "installPath": new}]}}).to_string(),
    );
    std::env::set_var("CLAUDE_CODE_PLUGIN_CACHE_DIR", &cache);
    let uses = Uses {
        plugins: vec!["forms".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(tools.skills().len(), 1);
    let item = f
        .store
        .find_library_item(LibraryKind::Plugin, "forms")
        .await
        .unwrap()
        .unwrap();
    match item.config {
        LibraryConfig::Plugin(c) => assert_eq!(c.path, new),
        other => panic!("{other:?}"),
    }
    tools.close().await;

    // Gone from Claude Code too: left out.
    std::fs::remove_dir_all(&new).unwrap();
    let (tools, notices) = f.open(&uses, None).await;
    std::env::remove_var("CLAUDE_CODE_PLUGIN_CACHE_DIR");
    assert_eq!(notices, ["plugin forms: folder not found"]);
    assert!(tools.skills().is_empty());
    tools.close().await;
}

#[tokio::test]
async fn a_cancelled_run_stops_opening_its_servers() {
    let f = fixture().await;
    let slow = FakeMcp::new(json!({"tools": [tool("read", true)], "init_delay_ms": 20000}));
    f.lib
        .add("slow", LibraryConfig::McpServer(slow.stdio()), None)
        .await
        .unwrap();
    let uses = Uses {
        mcp_servers: vec!["slow".into()],
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let (tools, _) = RunTools::open(OpenRequest {
        store: &f.store,
        bus: &f.bus,
        run_id: "01RUN",
        uses: &uses,
        workspace: f.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(tools.servers().is_empty());
    assert!(slow.gone_within(Duration::from_secs(10)).await);
}
```

**In `crates/dots-core/tests/runner_test.rs`, replace:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
        },
    )
```

**with:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
        },
    )
```

**Append to `crates/dots-core/tests/runner_test.rs`:**

```rust

#[tokio::test]
async fn a_run_gets_its_tools_records_what_was_left_out_and_closes_them_at_the_end() {
    use common::fake_mcp::{tool, FakeMcp};
    use dots_core::library::{LibraryConfig, SkillConfig};
    use dots_core::library_service::LibraryService;
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(CapturingEngine::default());
    let runner = runner_with(&store, &bus, &dir, engine.clone(), 1);
    runner.spawn_dispatcher(CancellationToken::new());
    let lib = LibraryService::new(store.clone(), bus.clone());
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    lib.add(
        "files",
        LibraryConfig::McpServer(fake.stdio()),
        Some(&probe.fingerprint),
    )
    .await
    .unwrap();
    let gone = dir.path().join("gone");
    std::fs::create_dir_all(&gone).unwrap();
    std::fs::write(gone.join("SKILL.md"), "---\nname: gone\n---\nBody.\n").unwrap();
    lib.add(
        "gone",
        LibraryConfig::Skill(SkillConfig {
            path: gone.to_string_lossy().into_owned(),
        }),
        None,
    )
    .await
    .unwrap();
    std::fs::remove_dir_all(&gone).unwrap();
    let mut s = spec("tools");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.uses.mcp_servers = vec!["files".into()];
    s.uses.skills = vec!["gone".into()];
    let dot = store.create_dot(&s).await.unwrap();
    let id = manual(&runner, &dot).await;
    wait_status(&store, &id, RunStatus::Succeeded, 15).await;
    {
        let started = engine.0.lock().unwrap();
        let tools = &started[0].tools;
        let srv = tools
            .server("files")
            .expect("the engine got the open session");
        assert!(!srv.held());
        assert!(srv.session().is_stopped(), "closed when the run ended");
    }
    assert!(fake.gone_within(Duration::from_secs(10)).await);
    let events = store.list_events(&id, 0).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["notice", "finished"]);
    assert_eq!(
        events[0].data,
        json!({"kind": "notice", "text": "skill gone: folder not found"})
    );
}
```

**In `crates/dots-core/tests/scheduler_test.rs`, replace:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
        },
    )
```

**with:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
        },
    )
```

**In `crates/dots-core/tests/server_test.rs`, replace:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
        },
    );
```

**with:**

```rust
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
        },
    );
```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
      expect(within(nav).getByRole("link", { name })).toBeVisible();
    }
    // One pending approval, one run to review and one schedule that needs attention.
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*3$/ })).toBeVisible();
  });

```

**with:**

```tsx
      expect(within(nav).getByRole("link", { name })).toBeVisible();
    }
    // One pending approval, one run to review, one schedule that needs attention and one MCP
    // server whose tools changed.
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*4$/ })).toBeVisible();
  });

```

**In `ui/src/app/Layout.test.tsx`, replace:**

```tsx
    backend.on("inbox", () => sample.inbox);
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*3$/ })).toBeVisible();
  });
});
```

**with:**

```tsx
    backend.on("inbox", () => sample.inbox);
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*4$/ })).toBeVisible();
  });
});
```

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts

describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  /// Shows the Inbox as the app's frame does.
  const watchInbox = (qc: Client) =>
```

**with:**

```ts

describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [] };
  /// Shows the Inbox as the app's frame does.
  const watchInbox = (qc: Client) =>
```

**In `ui/src/lib/transcript.test.ts`, replace:**

```ts
  });

  it("keeps what it cannot read as raw instead of dropping it", () => {
    const items = transcriptItems([
```

**with:**

```ts
  });

  it("shows a notice about the run as a note", () => {
    const items = transcriptItems([ev({ kind: "notice", text: "skill pdf: folder not found" })]);
    expect(items).toEqual([
      { type: "raw", seq: expect.any(Number), label: "Note", text: "skill pdf: folder not found" },
    ]);
  });

  it("keeps what it cannot read as raw instead of dropping it", () => {
    const items = transcriptItems([
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  it("drops the deleted dot's runs from the Inbox and its count", async () => {
    stored();
    let waiting: Inbox = { approvals: [], review_runs: sample.inbox.review_runs, attention: [] };
    backend.on("inbox", () => waiting);
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [], attention: [] };
      return null;
    });
```

**with:**

```tsx
  it("drops the deleted dot's runs from the Inbox and its count", async () => {
    stored();
    let waiting: Inbox = {
      approvals: [],
      review_runs: sample.inbox.review_runs,
      attention: [],
      tools_changed: [],
    };
    backend.on("inbox", () => waiting);
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [], attention: [], tools_changed: [] };
      return null;
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
describe("the Inbox", () => {
  it("lists what waits: approvals with their dot, and runs to review", async () => {
    inbox({ approvals: [approval], review_runs: [worktreeRun, failedRun], attention: [] });
    renderApp("/inbox");
    const approvals = await screen.findByRole("region", { name: "Approvals" });
```

**with:**

```tsx
describe("the Inbox", () => {
  it("lists what waits: approvals with their dot, and runs to review", async () => {
    inbox({
      approvals: [approval],
      review_runs: [worktreeRun, failedRun],
      attention: [],
      tools_changed: [],
    });
    renderApp("/inbox");
    const approvals = await screen.findByRole("region", { name: "Approvals" });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
      review_runs: [{ ...failedRun, branch: "dots/gone" }],
      attention: [],
    });
    renderApp("/inbox");
```

**with:**

```tsx
      review_runs: [{ ...failedRun, branch: "dots/gone" }],
      attention: [],
      tools_changed: [],
    });
    renderApp("/inbox");
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("approves a tool call and drops it from the list", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**with:**

```tsx

  it("approves a tool call and drops it from the list", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [], tools_changed: [] });
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("keeps both buttons of a decided approval off until the list has reloaded", async () => {
    inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("decide_approval", () => ({ ...sample.approval, status: "approved" }));
    const { user } = renderApp("/inbox");
```

**with:**

```tsx

  it("keeps both buttons of a decided approval off until the list has reloaded", async () => {
    inbox({ approvals: [approval], review_runs: [], attention: [], tools_changed: [] });
    backend.on("decide_approval", () => ({ ...sample.approval, status: "approved" }));
    const { user } = renderApp("/inbox");
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    backend.on("inbox", async () => {
      await held;
      return { approvals: [], review_runs: [], attention: [] };
    });

```

**with:**

```tsx
    backend.on("inbox", async () => {
      await held;
      return { approvals: [], review_runs: [], attention: [], tools_changed: [] };
    });

```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    async (button) => {
      const next = { ...approval, id: "apr-next", tool: "Write", input: { file_path: "a.txt" } };
      const box = inbox({ approvals: [approval, next], review_runs: [], attention: [] });
      backend.on("decide_approval", (args) => {
        box.set({ approvals: [next], review_runs: [], attention: [] });
        return { ...sample.approval, id: args.id, status: "approved" };
      });
```

**with:**

```tsx
    async (button) => {
      const next = { ...approval, id: "apr-next", tool: "Write", input: { file_path: "a.txt" } };
      const box = inbox({
        approvals: [approval, next],
        review_runs: [],
        attention: [],
        tools_changed: [],
      });
      backend.on("decide_approval", (args) => {
        box.set({ approvals: [next], review_runs: [], attention: [], tools_changed: [] });
        return { ...sample.approval, id: args.id, status: "approved" };
      });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("marks a run as reviewed and drops it from the list", async () => {
    const box = inbox({ approvals: [], review_runs: [failedRun], attention: [] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.run, id: failedRun.id, reviewed_at: sample.run.ended_at };
    });
```

**with:**

```tsx

  it("marks a run as reviewed and drops it from the list", async () => {
    const box = inbox({
      approvals: [],
      review_runs: [failedRun],
      attention: [],
      tools_changed: [],
    });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      return { ...sample.run, id: failedRun.id, reviewed_at: sample.run.ended_at };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  it("marks one run as reviewed when the button is double-clicked and the next row moves up", async () => {
    const next = { ...failedRun, id: "run-next" };
    const box = inbox({ approvals: [], review_runs: [failedRun, next], attention: [] });
    backend.on("mark_reviewed", (args) => {
      box.set({ approvals: [], review_runs: [next], attention: [] });
      return { ...sample.run, id: args.runId, reviewed_at: sample.run.ended_at };
    });
```

**with:**

```tsx
  it("marks one run as reviewed when the button is double-clicked and the next row moves up", async () => {
    const next = { ...failedRun, id: "run-next" };
    const box = inbox({
      approvals: [],
      review_runs: [failedRun, next],
      attention: [],
      tools_changed: [],
    });
    backend.on("mark_reviewed", (args) => {
      box.set({ approvals: [], review_runs: [next], attention: [], tools_changed: [] });
      return { ...sample.run, id: args.runId, reviewed_at: sample.run.ended_at };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("discards a worktree only after a confirmation", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.run, reviewed_at: sample.run.ended_at };
    });
```

**with:**

```tsx

  it("discards a worktree only after a confirmation", async () => {
    const box = inbox({
      approvals: [],
      review_runs: [worktreeRun],
      attention: [],
      tools_changed: [],
    });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      return { ...sample.run, reviewed_at: sample.run.ended_at };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("shows a refused action and keeps the run in the list", async () => {
    inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("mark_reviewed", () => {
      throw { kind: "conflict", message: "the thread is still running" };
```

**with:**

```tsx

  it("shows a refused action and keeps the run in the list", async () => {
    inbox({ approvals: [], review_runs: [worktreeRun], attention: [], tools_changed: [] });
    backend.on("mark_reviewed", () => {
      throw { kind: "conflict", message: "the thread is still running" };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    // The run is gone (its dot was deleted elsewhere): the core refuses, and the Inbox it
    // answers next no longer holds the run.
    const box = inbox({ approvals: [], review_runs: [failedRun], attention: [] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      throw { kind: "not_found", message: `run ${failedRun.id}` };
    });
```

**with:**

```tsx
    // The run is gone (its dot was deleted elsewhere): the core refuses, and the Inbox it
    // answers next no longer holds the run.
    const box = inbox({
      approvals: [],
      review_runs: [failedRun],
      attention: [],
      tools_changed: [],
    });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      throw { kind: "not_found", message: `run ${failedRun.id}` };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("drops a run whose discard was refused once the Inbox has reloaded without it", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      throw { kind: "not_found", message: `run ${worktreeRun.id}` };
    });
```

**with:**

```tsx

  it("drops a run whose discard was refused once the Inbox has reloaded without it", async () => {
    const box = inbox({
      approvals: [],
      review_runs: [worktreeRun],
      attention: [],
      tools_changed: [],
    });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      throw { kind: "not_found", message: `run ${worktreeRun.id}` };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  // count as stale, or the run view would show it as it is.
  it("loads a run's changes again when its worktree was discarded while they were loading", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("run_thread", () => [sample.run]);
    backend.on("get_dot", () => sample.dot);
```

**with:**

```tsx
  // count as stale, or the run view would show it as it is.
  it("loads a run's changes again when its worktree was discarded while they were loading", async () => {
    const box = inbox({
      approvals: [],
      review_runs: [worktreeRun],
      attention: [],
      tools_changed: [],
    });
    backend.on("run_thread", () => [sample.run]);
    backend.on("get_dot", () => sample.dot);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    backend.on("worktree_diff", () => before.answer);
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      backend.on("worktree_diff", () => ({ files: [], truncated: false, exists: false }));
      return { ...sample.run, reviewed_at: sample.run.ended_at };
```

**with:**

```tsx
    backend.on("worktree_diff", () => before.answer);
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      backend.on("worktree_diff", () => ({ files: [], truncated: false, exists: false }));
      return { ...sample.run, reviewed_at: sample.run.ended_at };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("loads a run's approvals again when one was decided while they were loading", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    const waiting: Run = {
      ...sample.run,
```

**with:**

```tsx

  it("loads a run's approvals again when one was decided while they were loading", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [], tools_changed: [] });
    const waiting: Run = {
      ...sample.run,
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    backend.on("decide_approval", () => {
      const approved: Approval = { ...sample.approval, status: "approved" };
      box.set({ approvals: [], review_runs: [], attention: [] });
      backend.on("approvals_for_run", () => [approved]);
      return approved;
```

**with:**

```tsx
    backend.on("decide_approval", () => {
      const approved: Approval = { ...sample.approval, status: "approved" };
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      backend.on("approvals_for_run", () => [approved]);
      return approved;
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("shows a new approval as soon as the dot asks", async () => {
    const box = inbox({ approvals: [], review_runs: [], attention: [] });
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(0));
    box.set({ approvals: [approval], review_runs: [], attention: [] });
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
```

**with:**

```tsx

  it("shows a new approval as soon as the dot asks", async () => {
    const box = inbox({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(0));
    box.set({ approvals: [approval], review_runs: [], attention: [], tools_changed: [] });
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  /// An Inbox with the one approval, whose decision empties it, and the rules the core offers.
  function waiting(rules: RuleCandidate[] = sample.rules) {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("approval_rule_candidates", () => rules);
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**with:**

```tsx
  /// An Inbox with the one approval, whose decision empties it, and the rules the core offers.
  function waiting(rules: RuleCandidate[] = sample.rules) {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [], tools_changed: [] });
    backend.on("approval_rule_candidates", () => rules);
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [], tools_changed: [] });
      return { ...sample.approval, status: "approved" };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
  };
  const mine = { name: `Approvals asked by ${approval.dot_name}` };
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));

  it("stand together under their dot, and a single one has no batch buttons", async () => {
    // The core lists approvals in the order asked; a call of another run came in between.
    inbox({ approvals: [approval, elsewhere, second], review_runs: [], attention: [] });
    renderApp("/inbox");
    const group = await screen.findByRole("group", mine);
```

**with:**

```tsx
  };
  const mine = { name: `Approvals asked by ${approval.dot_name}` };
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [] };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));

  it("stand together under their dot, and a single one has no batch buttons", async () => {
    // The core lists approvals in the order asked; a call of another run came in between.
    inbox({
      approvals: [approval, elsewhere, second],
      review_runs: [],
      attention: [],
      tools_changed: [],
    });
    renderApp("/inbox");
    const group = await screen.findByRole("group", mine);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
    ["Deny all (2)", "Deny all", false],
  ])("are answered together by %s, after a confirmation that lists them", async (open, confirm, approved) => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_run_approvals", () => {
      box.set(empty);
```

**with:**

```tsx
    ["Deny all (2)", "Deny all", false],
  ])("are answered together by %s, after a confirmation that lists them", async (open, confirm, approved) => {
    const box = inbox({
      approvals: [approval, second],
      review_runs: [],
      attention: [],
      tools_changed: [],
    });
    backend.on("decide_run_approvals", () => {
      box.set(empty);
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("stay, with their buttons, when the core refuses the batch", async () => {
    inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_run_approvals", () => {
      throw { kind: "internal", message: "database is locked" };
```

**with:**

```tsx

  it("stay, with their buttons, when the core refuses the batch", async () => {
    inbox({ approvals: [approval, second], review_runs: [], attention: [], tools_changed: [] });
    backend.on("decide_run_approvals", () => {
      throw { kind: "internal", message: "database is locked" };
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("can be answered together again when the run asks for more after a batch", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    const later1 = { ...approval, id: "apr-4", tool: "Edit", input: { file_path: "c.txt" } };
    const later2 = { ...approval, id: "apr-5", tool: "Read", input: { file_path: "d.txt" } };
    backend.on("decide_run_approvals", () => {
      // The run went on and asked for two more calls before the Inbox reloaded.
      box.set({ approvals: [later1, later2], review_runs: [], attention: [] });
      return answered.map((a) => ({ ...a, status: "approved" }));
    });
```

**with:**

```tsx

  it("can be answered together again when the run asks for more after a batch", async () => {
    const box = inbox({
      approvals: [approval, second],
      review_runs: [],
      attention: [],
      tools_changed: [],
    });
    const later1 = { ...approval, id: "apr-4", tool: "Edit", input: { file_path: "c.txt" } };
    const later2 = { ...approval, id: "apr-5", tool: "Read", input: { file_path: "d.txt" } };
    backend.on("decide_run_approvals", () => {
      // The run went on and asked for two more calls before the Inbox reloaded.
      box.set({ approvals: [later1, later2], review_runs: [], attention: [], tools_changed: [] });
      return answered.map((a) => ({ ...a, status: "approved" }));
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("can still be answered one at a time", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_approval", (args) => {
      box.set({ approvals: [second], review_runs: [], attention: [] });
      return { ...sample.approval, id: args.id, status: "approved" };
    });
```

**with:**

```tsx

  it("can still be answered one at a time", async () => {
    const box = inbox({
      approvals: [approval, second],
      review_runs: [],
      attention: [],
      tools_changed: [],
    });
    backend.on("decide_approval", (args) => {
      box.set({ approvals: [second], review_runs: [], attention: [], tools_changed: [] });
      return { ...sample.approval, id: args.id, status: "approved" };
    });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
      review_runs: [],
      attention: [],
    });
    renderApp("/inbox");
```

**with:**

```tsx
      review_runs: [],
      attention: [],
      tools_changed: [],
    });
    renderApp("/inbox");
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx
describe("a schedule that needs attention", () => {
  const [flagged] = sample.inbox.attention;
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  const only = (attention: Inbox["attention"]): Inbox => ({ ...empty, attention });
  const item = () => screen.findByRole("listitem", { name: `${flagged.dot_name} needs attention` });
```

**with:**

```tsx
describe("a schedule that needs attention", () => {
  const [flagged] = sample.inbox.attention;
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [] };
  const only = (attention: Inbox["attention"]): Inbox => ({ ...empty, attention });
  const item = () => screen.findByRole("listitem", { name: `${flagged.dot_name} needs attention` });
```

**In `ui/src/screens/InboxScreen.test.tsx`, replace:**

```tsx

  it("names the dot, the run that failed and why, and offers Run now and Dismiss", async () => {
    inbox({ approvals: [approval], review_runs: [failedRun], attention: [flagged] });
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "Needs attention" });
```

**with:**

```tsx

  it("names the dot, the run that failed and why, and offers Run now and Dismiss", async () => {
    inbox({
      approvals: [approval],
      review_runs: [failedRun],
      attention: [flagged],
      tools_changed: [],
    });
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "Needs attention" });
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  backend.on("startup_error", () => null);
  backend.on("status", () => quietStatus);
  backend.on("inbox", () => ({ approvals: [], review_runs: [], attention: [] }));
  // An approval card asks for these when it shows; by default the core offers no rule.
  backend.on("approval_rule_candidates", () => []);
```

**with:**

```ts
  backend.on("startup_error", () => null);
  backend.on("status", () => quietStatus);
  backend.on("inbox", () => ({ approvals: [], review_runs: [], attention: [], tools_changed: [] }));
  // An approval card asks for these when it shows; by default the core offers no rule.
  backend.on("approval_rule_candidates", () => []);
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test claude_cli_test --test claude_engine_test --test engine_prompt_test --test library_service_test --test local_engine_test --test resume_test --test run_tools_test --test runner_test --test scheduler_test --test server_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/app/Layout.test.tsx src/lib/runtime-events.test.ts src/lib/transcript.test.ts src/screens/DotEditorScreen.test.tsx src/screens/InboxScreen.test.tsx 2>&1 | grep -E "Tests |Test Files|× |FAIL ")
```

Expected: the ten Rust test targets do not compile. The decisive lines: 6 × `` struct `RunnerConfig` has no field named `plugins_dir` ``, 3 × `` no field `tools_changed` on type `Inbox` ``, 3 × `` struct `RunContext` has no field named `tools` ``, 3 × `` unresolved import `dots_core::library_service` ``, 2 × `` no variant named `LibraryUpdated` found for enum `RuntimeEvent` ``, 2 × `` no method named `library` found for struct `dots_core::Runtime` ``, `` unresolved import `dots_core::run_tools` ``, then `could not compile` for each of the ten. The UI: `Test Files  4 failed | 1 passed (5)`, `Tests  39 failed | 69 passed (108)`: "shows a notice about the run as a note" in `transcript.test.ts`, and every test that loads the Inbox (the sidebar's count, the Inbox screen), because the fixtures' Inbox now carries `tools_changed`, which the strict schema of the commit before refuses; those fail on their 5 s timeouts.

- [ ] **Step 3: Implement**

The core first, then the app's contract, then the UI files. `types.ts` is checked against the contract fixture, which the next step regenerates.

**In `crates/dots-core/src/events.rs`, replace:**

```rust
        error: Option<String>,
    },
}

```

**with:**

```rust
        error: Option<String>,
    },
    /// A Library item was added, changed or removed, or an MCP server's catalog changed (a
    /// check, an accept, a list that changed during a run). `item_id` names the item; the
    /// Library and the Inbox read it again.
    LibraryUpdated {
        item_id: String,
    },
}

```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
}

/// Pending approvals, finished runs awaiting review and schedules that need attention (spec
/// §12 Inbox; Scheduler v2 spec §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inbox {
```

**with:**

```rust
}

/// A dot named in a list: its id and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotRef {
    pub id: String,
    pub name: String,
}

/// An MCP server whose tool list changed since it was accepted, or was never accepted: its
/// tools ask until the person reviews and accepts the list (MCP/skills spec section 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsChanged {
    pub server_id: String,
    pub name: String,
    /// The dots that use it (through its plugin, for a plugin's server), by name.
    pub dots: Vec<DotRef>,
}

/// Pending approvals, finished runs awaiting review, schedules that need attention and MCP
/// servers whose tools changed (spec §12 Inbox; Scheduler v2 spec §8; MCP/skills spec §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inbox {
```

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    /// Newest failure first.
    pub attention: Vec<NeedsAttention>,
}

```

**with:**

```rust
    /// Newest failure first.
    pub attention: Vec<NeedsAttention>,
    /// By server name.
    pub tools_changed: Vec<ToolsChanged>,
}

```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
use std::path::PathBuf;

use async_trait::async_trait;
```

**with:**

```rust
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::Result;

```

**with:**

```rust
use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::run_tools::RunTools;
use crate::Result;

```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
        error: String,
    },
}

```

**with:**

```rust
        error: String,
    },
    /// Something the person should know about the run that is not the model's doing, such as
    /// a Library item left out of it ("skill pdf: folder not found") or an MCP server's
    /// problem ("salesforce: could not start: SF_TOKEN is not set").
    Notice {
        text: String,
    },
}

```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
            Self::Finished { .. } => "finished",
            Self::Failed { .. } => "failed",
        }
    }
```

**with:**

```rust
            Self::Finished { .. } => "finished",
            Self::Failed { .. } => "failed",
            Self::Notice { .. } => "notice",
        }
    }
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
    /// expanded (`Store::used_mcp_servers`).
    pub mcp_servers: Vec<LibraryItem>,
}

```

**with:**

```rust
    /// expanded (`Store::used_mcp_servers`).
    pub mcp_servers: Vec<LibraryItem>,
    /// The run's MCP sessions, skills and plugin folders, opened by the runner before the
    /// engine starts and closed when the run ends (`RunTools`).
    pub tools: Arc<RunTools>,
}

```

**In `crates/dots-core/src/store/library.rs`, replace:**

```rust
    }

    /// Accepts the tool list last seen, if its fingerprint is still `fingerprint`: the seen
    /// list becomes the accepted one and the status `accepted`. A list seen since is a
```

**with:**

```rust
    }

    /// Records a check that listed the server's tools (spec section 3): they become the seen
    /// list, and the status is `accepted` when their fingerprint is the accepted one, else
    /// `changed`. One statement, so a check never undoes an accept that lands meanwhile.
    pub async fn record_catalog_seen(
        &self,
        server_id: &str,
        fingerprint: &str,
        tools: &Value,
    ) -> Result<McpCatalog> {
        let row = sqlx::query(&format!(
            "UPDATE mcp_catalogs SET seen_fingerprint = ?, seen_tools = ?, checked_at = ?, \
             error = NULL, status = CASE WHEN accepted_fingerprint = ? THEN ? ELSE ? END \
             WHERE server_id = ? RETURNING {CATALOG_COLUMNS}"
        ))
        .bind(fingerprint)
        .bind(tools.to_string())
        .bind(now())
        .bind(fingerprint)
        .bind(CatalogStatus::Accepted.as_str())
        .bind(CatalogStatus::Changed.as_str())
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("catalog of MCP server {server_id}")))?;
        catalog_from_row(&row)
    }

    /// Records a check that failed: status `unreachable` with the error. The accepted and the
    /// seen lists stay.
    pub async fn record_catalog_failure(&self, server_id: &str, error: &str) -> Result<McpCatalog> {
        let row = sqlx::query(&format!(
            "UPDATE mcp_catalogs SET status = ?, error = ?, checked_at = ? WHERE server_id = ? \
             RETURNING {CATALOG_COLUMNS}"
        ))
        .bind(CatalogStatus::Unreachable.as_str())
        .bind(error)
        .bind(now())
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("catalog of MCP server {server_id}")))?;
        catalog_from_row(&row)
    }

    /// The MCP servers that belong to the plugin `owner_id`, by name.
    pub async fn owned_library_items(&self, owner_id: &str) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE owner_id = ? ORDER BY name"
        ))
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// The MCP servers whose catalog is `changed`: their tool list waits to be reviewed.
    pub async fn changed_mcp_servers(&self) -> Result<Vec<LibraryItem>> {
        let rows = sqlx::query(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items WHERE id IN \
             (SELECT server_id FROM mcp_catalogs WHERE status = ?) ORDER BY name"
        ))
        .bind(CatalogStatus::Changed.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(item_from_row).collect()
    }

    /// Accepts the tool list last seen, if its fingerprint is still `fingerprint`: the seen
    /// list becomes the accepted one and the status `accepted`. A list seen since is a
```

**In `crates/dots-core/src/kit/installed.rs`, replace:**

```rust
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// `<name>@<marketplace>`
```

**with:**

```rust
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledPlugin {
    /// `<name>@<marketplace>`
```

**Create `crates/dots-core/src/library_service.rs`:**

```rust
//! The Library at runtime (MCP/skills spec sections 3 and 9): what the app's `library_*`
//! commands do. It adds, changes and removes items (asking first when dots use one), tests an
//! MCP server config, checks a server and accepts its list with the fingerprint guard, keeps a
//! plugin's own MCP server items in line with its folder, scans a folder for skills, lists the
//! installed Claude Code plugins, and says which variables are set. Every change of an item or
//! a catalog is published as `RuntimeEvent::LibraryUpdated`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::events::{Bus, RuntimeEvent};
use crate::kit::{self, InstalledPlugin, PluginInfo};
use crate::library::{
    check_name, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
};
use crate::mcp::{fingerprint, McpError, McpSession, McpTool, SessionOptions};
use crate::model::{DotRef, ToolsChanged};
use crate::store::Store;
use crate::{Error, Result};

/// Dots' own environment, where `${VAR}` is read.
pub(crate) fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// One Library item as the Library screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryEntry {
    #[serde(flatten)]
    pub item: LibraryItem,
    /// The dots that use it; for a plugin's MCP server, the dots that use the plugin.
    pub used_by: Vec<DotRef>,
    /// An MCP server's catalog.
    pub catalog: Option<McpCatalog>,
    /// The `${VAR}` of an MCP server's config that Dots' environment does not set.
    pub missing_variables: Vec<String>,
    /// A skill read from its folder.
    pub skill: Option<SkillDetail>,
    /// A plugin read from its folder.
    pub plugin: Option<PluginDetail>,
    /// Why a skill or plugin folder could not be read ("folder not found").
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDetail {
    pub description: String,
}

/// A plugin's parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginDetail {
    pub version: Option<String>,
    pub description: Option<String>,
    /// Qualified: `<plugin>:<skill>`.
    pub skills: Vec<String>,
    pub commands: Vec<String>,
    pub agents: Vec<String>,
    /// The names of its MCP server items.
    pub servers: Vec<String>,
    pub has_hooks: bool,
    /// What could not be read in it.
    pub warnings: Vec<String>,
}

/// What a server config lists (`library_probe`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct McpProbe {
    pub tools: Vec<McpTool>,
    pub fingerprint: String,
}

/// A skill found by `library_scan_skills`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScannedSkill {
    pub name: String,
    pub description: String,
    pub path: String,
    /// Why `name` cannot be a Library name; the person picks another.
    pub name_error: Option<String>,
}

/// Whether a variable is set in Dots' environment. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvVariable {
    pub name: String,
    pub set: bool,
}

pub(crate) fn publish(bus: &Bus, item_id: &str) {
    let _ = bus.send(RuntimeEvent::LibraryUpdated {
        item_id: item_id.to_string(),
    });
}

/// Opens a session with the server and lists its tools. The session is closed again when the
/// listing fails.
pub(crate) async fn open_and_list(
    name: &str,
    config: &McpServerConfig,
    options: SessionOptions,
) -> std::result::Result<(McpSession, Vec<McpTool>), McpError> {
    let session = McpSession::open(name, config, &process_env, options).await?;
    match session.list_tools().await {
        Ok(tools) => Ok((session, tools)),
        Err(e) => {
            session.close().await;
            Err(e)
        }
    }
}

/// Writes the outcome of a check to the server's catalog and publishes the change.
pub(crate) async fn record_check(
    store: &Store,
    bus: &Bus,
    server_id: &str,
    outcome: std::result::Result<&[McpTool], &McpError>,
) -> Result<McpCatalog> {
    let catalog = match outcome {
        Ok(tools) => {
            store
                .record_catalog_seen(
                    server_id,
                    &fingerprint(tools),
                    &serde_json::to_value(tools)?,
                )
                .await?
        }
        Err(e) => {
            store
                .record_catalog_failure(server_id, &e.to_string())
                .await?
        }
    };
    publish(bus, server_id);
    Ok(catalog)
}

/// Why the skill folder `dir` cannot be used, in the words of a run event.
pub(crate) fn skill_problem(dir: &Path, e: &Error) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else if !dir.join("SKILL.md").is_file() {
        "SKILL.md not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

fn plugin_problem(dir: &Path, e: &Error) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("reading the folder failed: {e}")))
}

fn plugin_config(item: &LibraryItem) -> Result<&PluginConfig> {
    match &item.config {
        LibraryConfig::Plugin(c) => Ok(c),
        _ => Err(Error::Invalid(format!("{:?} is not a plugin", item.name))),
    }
}

/// Brings the plugin's MCP server items in line with what its folder lists (spec section 2):
/// new servers are added, changed ones updated (their catalog goes back to `unchecked`),
/// servers no longer listed removed. A server whose item name the Library already has for a
/// server of its own is left out, with a warning.
pub(crate) async fn sync_plugin_servers(
    store: &Store,
    bus: &Bus,
    plugin: &LibraryItem,
    info: &PluginInfo,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let wanted = info.server_items(&plugin.name);
    let existing = store.owned_library_items(&plugin.id).await?;
    for old in &existing {
        if !wanted.iter().any(|(name, _)| *name == old.name) {
            store.remove_library_item(&old.id).await?;
            publish(bus, &old.id);
        }
    }
    for (name, config) in wanted {
        let config = LibraryConfig::McpServer(config);
        match existing.iter().find(|i| i.name == name) {
            Some(old) if old.config == config => {}
            Some(old) => {
                store.update_library_item(&old.id, &config).await?;
                publish(bus, &old.id);
            }
            None => match store
                .add_library_item(&name, &config, Some(&plugin.id))
                .await
            {
                Ok(item) => publish(bus, &item.id),
                Err(Error::Conflict(_)) => warnings.push(format!(
                    "its MCP server {name} is left out: the Library already has an MCP server \
                     of that name"
                )),
                Err(e) => return Err(e),
            },
        }
    }
    Ok(warnings)
}

/// Reads the plugin's folder again, as a run does at its start (spec section 6). An installed
/// plugin whose folder is gone is looked up in Claude Code's list again, and its new folder is
/// stored. Its MCP server items are brought in line. `Ok(Err(reason))` when the folder cannot
/// be used: "folder not found" or "could not be read: ...".
pub(crate) async fn refresh_plugin(
    store: &Store,
    bus: &Bus,
    item: &LibraryItem,
) -> Result<std::result::Result<(PluginInfo, Vec<String>), String>> {
    let mut config = plugin_config(item)?.clone();
    if !Path::new(&config.path).is_dir() {
        let moved = config.installed_id.clone().and_then(|id| {
            kit::find_installed(&process_env, &id)
                .filter(|p| p.exists)
                .map(|p| p.install_path)
        });
        if let Some(path) = moved {
            config.path = path.to_string_lossy().into_owned();
            store
                .update_library_item(&item.id, &LibraryConfig::Plugin(config.clone()))
                .await?;
            publish(bus, &item.id);
        }
    }
    let root = PathBuf::from(&config.path);
    let read = blocking(move || {
        let info = kit::read_plugin(&root);
        info.map_err(|e| plugin_problem(&root, &e))
    })
    .await?;
    let info = match read {
        Ok(info) => info,
        Err(reason) => return Ok(Err(reason)),
    };
    let mut warnings = info.warnings.clone();
    warnings.extend(sync_plugin_servers(store, bus, item, &info).await?);
    Ok(Ok((info, warnings)))
}

pub struct LibraryService {
    store: Store,
    bus: Bus,
}

impl LibraryService {
    pub fn new(store: Store, bus: Bus) -> LibraryService {
        LibraryService { store, bus }
    }

    /// Every item, with the dots that use it, a server's catalog and missing variables, and
    /// what a skill's or plugin's folder holds (`library_list`).
    pub async fn list(&self) -> Result<Vec<LibraryEntry>> {
        let items = self.store.list_library_items().await?;
        let mut catalogs: HashMap<String, McpCatalog> = self
            .store
            .list_catalogs()
            .await?
            .into_iter()
            .map(|c| (c.server_id.clone(), c))
            .collect();
        let folders: Vec<(LibraryKind, String)> = items
            .iter()
            .map(|i| match &i.config {
                LibraryConfig::Skill(c) => (LibraryKind::Skill, c.path.clone()),
                LibraryConfig::Plugin(c) => (LibraryKind::Plugin, c.path.clone()),
                LibraryConfig::McpServer(_) => (LibraryKind::McpServer, String::new()),
            })
            .collect();
        let read = blocking(move || {
            folders
                .into_iter()
                .map(|(kind, path)| {
                    let dir = PathBuf::from(&path);
                    match kind {
                        LibraryKind::Skill => Some(
                            kit::read_skill(&dir)
                                .map(|s| (Some(s), None))
                                .map_err(|e| skill_problem(&dir, &e)),
                        ),
                        LibraryKind::Plugin => Some(
                            kit::read_plugin(&dir)
                                .map(|p| (None, Some(p)))
                                .map_err(|e| plugin_problem(&dir, &e)),
                        ),
                        LibraryKind::McpServer => None,
                    }
                })
                .collect::<Vec<_>>()
        })
        .await?;
        let mut entries = Vec::new();
        for (item, read) in items.iter().zip(read) {
            let used_by = self
                .store
                .dots_using(item)
                .await?
                .into_iter()
                .map(|d| DotRef {
                    id: d.id,
                    name: d.spec.name,
                })
                .collect();
            let mut entry = LibraryEntry {
                item: item.clone(),
                used_by,
                catalog: catalogs.remove(&item.id),
                missing_variables: Vec::new(),
                skill: None,
                plugin: None,
                problem: None,
            };
            if let LibraryConfig::McpServer(config) = &item.config {
                entry.missing_variables = config
                    .variables()
                    .into_iter()
                    .filter(|v| process_env(v).is_none())
                    .collect();
            }
            match read {
                Some(Ok((skill, plugin))) => {
                    entry.skill = skill.map(|s| SkillDetail {
                        description: s.description,
                    });
                    entry.plugin = plugin.map(|p| PluginDetail {
                        skills: p
                            .qualified_skills(&item.name)
                            .into_iter()
                            .map(|(name, _)| name)
                            .collect(),
                        servers: items
                            .iter()
                            .filter(|i| i.owner_id.as_deref() == Some(item.id.as_str()))
                            .map(|i| i.name.clone())
                            .collect(),
                        version: p.version,
                        description: p.description,
                        commands: p.commands,
                        agents: p.agents,
                        has_hooks: p.has_hooks,
                        warnings: p.warnings,
                    });
                }
                Some(Err(problem)) => entry.problem = Some(problem),
                None => {}
            }
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Adds an item (`library_add`). A skill or plugin folder must be readable. A plugin's MCP
    /// servers become items of their own. An MCP server is not started unless `accept` is
    /// given: then it is checked, and the list it gives is accepted if its fingerprint is
    /// `accept`, the one the person was shown by `probe`. Otherwise the list waits for review
    /// (status `changed`), or the server stays `unchecked`.
    pub async fn add(
        &self,
        name: &str,
        config: LibraryConfig,
        accept: Option<&str>,
    ) -> Result<LibraryItem> {
        if accept.is_some() && config.kind() != LibraryKind::McpServer {
            return Err(Error::Invalid(
                "only an MCP server has a tool list to accept".into(),
            ));
        }
        let plugin = self.read_folder(&config).await?;
        let item = self.store.add_library_item(name, &config, None).await?;
        publish(&self.bus, &item.id);
        if let Some(info) = plugin {
            for warning in sync_plugin_servers(&self.store, &self.bus, &item, &info).await? {
                tracing::warn!("plugin {name}: {warning}");
            }
        }
        if let Some(fp) = accept {
            let catalog = self.check_item(&item).await?;
            if catalog.seen_fingerprint.as_deref() == Some(fp) {
                self.store.accept_catalog(&item.id, fp).await?;
                publish(&self.bus, &item.id);
            }
        }
        Ok(item)
    }

    /// Changes an item's config (`library_update`). A changed MCP server goes back to
    /// `unchecked`. A plugin's folder is read again and its MCP servers brought in line. A
    /// plugin's own MCP server is changed through its plugin only.
    pub async fn update(&self, id: &str, config: LibraryConfig) -> Result<LibraryItem> {
        let old = self.store.get_library_item(id).await?;
        self.refuse_owned(&old, "change")?;
        let plugin = self.read_folder(&config).await?;
        let item = self.store.update_library_item(id, &config).await?;
        if item.config != old.config {
            publish(&self.bus, id);
        }
        if let Some(info) = plugin {
            for warning in sync_plugin_servers(&self.store, &self.bus, &item, &info).await? {
                tracing::warn!("plugin {}: {warning}", item.name);
            }
        }
        Ok(item)
    }

    /// Removes an item (`library_remove`). When dots use it, it is refused (`Conflict`, naming
    /// them) unless `confirm`; then its name is taken out of their `uses`. A plugin's MCP
    /// servers go with it; a plugin's server cannot be removed on its own.
    pub async fn remove(&self, id: &str, confirm: bool) -> Result<()> {
        let item = self.store.get_library_item(id).await?;
        self.refuse_owned(&item, "remove")?;
        let users = self.store.dots_using(&item).await?;
        if !users.is_empty() && !confirm {
            let names: Vec<String> = users.into_iter().map(|d| d.spec.name).collect();
            return Err(Error::Conflict(format!(
                "the {} {:?} is used by {}; removing it takes it out of them",
                item.kind().label(),
                item.name,
                names.join(", ")
            )));
        }
        let owned = self.store.owned_library_items(id).await?;
        self.store.remove_library_item(id).await?;
        publish(&self.bus, id);
        for server in owned {
            publish(&self.bus, &server.id);
        }
        Ok(())
    }

    /// Starts or reaches the server of `config` and lists its tools, without saving anything
    /// (`library_probe`). The error is the text the person sees ("could not start: SF_TOKEN is
    /// not set").
    pub async fn probe(&self, config: &McpServerConfig) -> Result<McpProbe> {
        let (session, tools) = open_and_list("probe", config, SessionOptions::default()).await?;
        session.close().await;
        Ok(McpProbe {
            fingerprint: fingerprint(&tools),
            tools,
        })
    }

    /// Checks the MCP server `id` now (`library_check`, "Check now"): its list is recorded as
    /// seen, and the status is `accepted`, `changed` or `unreachable`.
    pub async fn check(&self, id: &str) -> Result<McpCatalog> {
        let item = self.server(id).await?;
        self.check_item(&item).await
    }

    /// Accepts the list last seen of the MCP server `id`, if its fingerprint is still
    /// `fingerprint` (`library_accept`); a newer list is a `Conflict`.
    pub async fn accept(&self, id: &str, fingerprint: &str) -> Result<McpCatalog> {
        self.server(id).await?;
        let catalog = self.store.accept_catalog(id, fingerprint).await?;
        publish(&self.bus, id);
        Ok(catalog)
    }

    /// The skills under `path` (`library_scan_skills`).
    pub async fn scan_skills(&self, path: &str) -> Result<Vec<ScannedSkill>> {
        let root = PathBuf::from(path);
        let found = blocking(move || kit::scan_skills(&root)).await??;
        Ok(found
            .into_iter()
            .map(|s| ScannedSkill {
                name_error: check_name(LibraryKind::Skill, &s.name)
                    .err()
                    .map(|e| match e {
                        Error::Invalid(text) => text,
                        other => other.to_string(),
                    }),
                path: s.dir.to_string_lossy().into_owned(),
                name: s.name,
                description: s.description,
            })
            .collect())
    }

    /// The plugins Claude Code has installed (`installed_claude_plugins`).
    pub fn installed_plugins(&self) -> Vec<InstalledPlugin> {
        kit::installed_plugins(&process_env)
    }

    /// Which of `names` Dots' environment sets (`environment_variables`).
    pub fn environment_variables(&self, names: &[String]) -> Vec<EnvVariable> {
        names
            .iter()
            .map(|name| EnvVariable {
                name: name.clone(),
                set: process_env(name).is_some(),
            })
            .collect()
    }

    /// The MCP servers whose tool list waits for review, with the dots that use them (the
    /// Inbox's "Tools changed").
    pub async fn tools_changed(&self) -> Result<Vec<ToolsChanged>> {
        let mut out = Vec::new();
        for item in self.store.changed_mcp_servers().await? {
            let dots = self
                .store
                .dots_using(&item)
                .await?
                .into_iter()
                .map(|d| DotRef {
                    id: d.id,
                    name: d.spec.name,
                })
                .collect();
            out.push(ToolsChanged {
                server_id: item.id,
                name: item.name,
                dots,
            });
        }
        Ok(out)
    }

    async fn server(&self, id: &str) -> Result<LibraryItem> {
        let item = self.store.get_library_item(id).await?;
        if item.kind() != LibraryKind::McpServer {
            return Err(Error::Invalid(format!(
                "{:?} is a {}, not an MCP server",
                item.name,
                item.kind().label()
            )));
        }
        Ok(item)
    }

    async fn check_item(&self, item: &LibraryItem) -> Result<McpCatalog> {
        let LibraryConfig::McpServer(config) = &item.config else {
            return Err(Error::Invalid(format!(
                "{:?} is not an MCP server",
                item.name
            )));
        };
        match open_and_list(&item.name, config, SessionOptions::default()).await {
            Ok((session, tools)) => {
                session.close().await;
                record_check(&self.store, &self.bus, &item.id, Ok(&tools)).await
            }
            Err(e) => record_check(&self.store, &self.bus, &item.id, Err(&e)).await,
        }
    }

    fn refuse_owned(&self, item: &LibraryItem, verb: &str) -> Result<()> {
        if item.owner_id.is_some() {
            return Err(Error::Invalid(format!(
                "the MCP server {:?} comes with a plugin; {verb} the plugin instead",
                item.name
            )));
        }
        Ok(())
    }

    /// Reads a skill's or plugin's folder before it is stored: `NotFound` when it is missing.
    /// A plugin's reading is returned, for its MCP servers.
    async fn read_folder(&self, config: &LibraryConfig) -> Result<Option<PluginInfo>> {
        match config {
            LibraryConfig::Skill(c) => {
                let dir = PathBuf::from(&c.path);
                blocking(move || kit::read_skill(&dir)).await??;
                Ok(None)
            }
            LibraryConfig::Plugin(c) => {
                let dir = PathBuf::from(&c.path);
                Ok(Some(blocking(move || kit::read_plugin(&dir)).await??))
            }
            LibraryConfig::McpServer(_) => Ok(None),
        }
    }
}
```

**Create `crates/dots-core/src/run_tools.rs`:**

```rust
//! A run's tools (MCP/skills spec sections 5 to 7), opened by the runner before the engine
//! starts and closed when the run ends:
//!
//! - an MCP session with every server the dot uses, its plugins' servers included, each
//!   checked against its catalog at the start: a list that is not the accepted one holds the
//!   server for the run, and so does a `notifications/tools/list_changed` that brings one;
//! - the skills it reads, standalone (`pdf`) and from its plugins (`forms:fill`);
//! - for a Claude-engine run, the plugin folders it passes with `--plugin-dir`.
//!
//! What cannot be used is left out, and `open` says so in run events: a server that cannot
//! start, a folder that is missing.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::events::Bus;
use crate::kit::{self, KitPlugin, KitSkill, PluginInfo, RunKit, SkillInfo};
use crate::library::{CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, Uses};
use crate::library_service::{process_env, record_check, refresh_plugin, skill_problem};
use crate::mcp::{McpError, McpSession, McpTool, SessionOptions};
use crate::policy::McpTrust;
use crate::store::Store;
use crate::Result;

/// What `RunTools::open` needs.
pub struct OpenRequest<'a> {
    pub store: &'a Store,
    pub bus: &'a Bus,
    pub run_id: &'a str,
    pub uses: &'a Uses,
    /// The run's folder: a stdio server without a `cwd` of its own starts there.
    pub workspace: &'a Path,
    /// Where a Claude-engine run's plugin folders go (`<data_dir>/plugins`); `None` makes none.
    pub plugins_dir: Option<&'a Path>,
    /// The run's cancel token: opening stops when it fires.
    pub cancel: &'a CancellationToken,
}

/// A skill of the run, with the name the model calls it by.
#[derive(Debug, Clone)]
pub struct RunSkill {
    /// The Library name for a standalone skill (`pdf`); `<plugin>:<skill>` for a plugin's.
    pub name: String,
    pub info: SkillInfo,
    /// The plugin's Library name, for a plugin's skill.
    pub plugin: Option<String>,
}

#[derive(Debug)]
struct Live {
    /// The list the server gave last.
    tools: Vec<McpTool>,
    /// Every call asks, for the rest of the run (spec section 4, step 2).
    held: bool,
}

/// An open MCP session of the run.
pub struct RunServer {
    item_id: String,
    name: String,
    session: Arc<McpSession>,
    /// The accepted list's tools, each with its `readOnlyHint`.
    accepted: HashMap<String, bool>,
    accepted_fingerprint: Option<String>,
    live: Arc<Mutex<Live>>,
}

impl RunServer {
    /// The server's Library item.
    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    /// The tool prefix: tools are `mcp__<name>__<tool>`.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn session(&self) -> &Arc<McpSession> {
        &self.session
    }

    /// The tools the server lists now (the list of the run's start, or the one a list change
    /// brought), in its order.
    pub fn tools(&self) -> Vec<McpTool> {
        self.live.lock().unwrap().tools.clone()
    }

    /// Whether every call of this server asks: its list was not the accepted one at the start
    /// of the run, or changed during it.
    pub fn held(&self) -> bool {
        self.live.lock().unwrap().held
    }

    /// The accepted list's fingerprint when the run started; an approval stores it
    /// (`approvals.mcp_fingerprint`).
    pub fn accepted_fingerprint(&self) -> Option<String> {
        self.accepted_fingerprint.clone()
    }

    /// What the check of a call of `tool` knows of the server (`Policy::resolve_mcp`): held
    /// when the server is held or the accepted list does not have the tool; read-only as the
    /// accepted list says, never as the server says now.
    pub fn trust(&self, tool: &str) -> McpTrust {
        let read_only = self.accepted.get(tool).copied();
        McpTrust {
            held: self.held() || read_only.is_none(),
            read_only: read_only.unwrap_or(false),
        }
    }
}

/// The run's MCP sessions, skills and plugin folders. `RunTools::default()` has none.
#[derive(Default)]
pub struct RunTools {
    servers: Vec<RunServer>,
    skills: Vec<RunSkill>,
    kit: Option<Arc<RunKit>>,
    stop: CancellationToken,
    watchers: Mutex<Vec<JoinHandle<()>>>,
}

impl fmt::Debug for RunTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunTools")
            .field(
                "servers",
                &self.servers.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
            .field(
                "skills",
                &self.skills.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
            .field("plugin_dirs", &self.plugin_dirs())
            .finish()
    }
}

impl RunTools {
    /// The open sessions, by server name.
    pub fn servers(&self) -> &[RunServer] {
        &self.servers
    }

    pub fn server(&self, name: &str) -> Option<&RunServer> {
        self.servers.iter().find(|s| s.name == name)
    }

    /// The server and tool of `mcp__<server>__<tool>`, when the run has that server open. A
    /// server name never holds `__`, so the name ends at the first one.
    pub fn resolve<'a>(&self, qualified: &'a str) -> Option<(&RunServer, &'a str)> {
        let (server, tool) = qualified.strip_prefix("mcp__")?.split_once("__")?;
        Some((self.server(server)?, tool))
    }

    /// `RunServer::trust` of `tool` of the open server `server`; `None` without that server.
    pub fn trust(&self, server: &str, tool: &str) -> Option<McpTrust> {
        self.server(server).map(|s| s.trust(tool))
    }

    /// The skills: the standalone ones in the dot's order, then each plugin's.
    pub fn skills(&self) -> &[RunSkill] {
        &self.skills
    }

    pub fn skill(&self, name: &str) -> Option<&RunSkill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// A Claude-engine run's folders for `--plugin-dir`; empty for a local-engine run.
    pub fn plugin_dirs(&self) -> &[PathBuf] {
        match &self.kit {
            Some(kit) => &kit.plugin_dirs,
            None => &[],
        }
    }

    /// Opens the run's tools for a dot with `uses`. Returns them with the run events to record:
    /// items left out and servers held. A cancel while opening closes what was opened and
    /// returns no tools. Only a database failure is an error.
    pub async fn open(req: OpenRequest<'_>) -> Result<(RunTools, Vec<String>)> {
        let OpenRequest {
            store,
            bus,
            run_id,
            uses,
            workspace,
            plugins_dir,
            cancel,
        } = req;
        let mut notices = Vec::new();

        // Plugins first: reading one brings its MCP server items in line.
        let mut plugins: Vec<(LibraryItem, PluginInfo)> = Vec::new();
        let mut left_out = HashSet::new();
        for name in &uses.plugins {
            let Some(item) = store.find_library_item(LibraryKind::Plugin, name).await? else {
                continue;
            };
            match refresh_plugin(store, bus, &item).await? {
                Ok((info, warnings)) => {
                    notices.extend(warnings.iter().map(|w| format!("plugin {name}: {w}")));
                    plugins.push((item, info));
                }
                Err(reason) => {
                    notices.push(format!("plugin {name}: {reason}"));
                    left_out.insert(item.id);
                }
            }
        }

        let items: Vec<LibraryItem> = store
            .used_mcp_servers(uses)
            .await?
            .into_iter()
            .filter(|s| s.owner_id.as_ref().is_none_or(|o| !left_out.contains(o)))
            .collect();
        let Some(opened) = open_sessions(&items, workspace, cancel).await else {
            return Ok((RunTools::default(), Vec::new()));
        };
        let mut tools = RunTools::default();
        for (item, outcome) in items.into_iter().zip(opened) {
            match outcome {
                Ok((session, listed, changes)) => {
                    let catalog = record_check(store, bus, &item.id, Ok(&listed)).await?;
                    if let Some(notice) = held_notice(&item.name, &catalog) {
                        notices.push(notice);
                    }
                    let server = RunServer {
                        accepted: accepted_tools(&catalog),
                        accepted_fingerprint: catalog.accepted_fingerprint.clone(),
                        live: Arc::new(Mutex::new(Live {
                            tools: listed,
                            held: catalog.status != CatalogStatus::Accepted,
                        })),
                        session: Arc::new(session),
                        item_id: item.id,
                        name: item.name,
                    };
                    tools.watch(&server, changes, store, bus);
                    tools.servers.push(server);
                }
                Err(e) => {
                    record_check(store, bus, &item.id, Err(&e)).await?;
                    notices.push(format!("{}: {e}", item.name));
                }
            }
        }

        let mut kit_skills = Vec::new();
        for name in &uses.skills {
            let Some(item) = store.find_library_item(LibraryKind::Skill, name).await? else {
                continue;
            };
            let LibraryConfig::Skill(config) = &item.config else {
                continue;
            };
            let dir = PathBuf::from(&config.path);
            let read_dir = dir.clone();
            let read = tokio::task::spawn_blocking(move || kit::read_skill(&read_dir))
                .await
                .map_err(|e| crate::Error::Other(format!("reading a skill failed: {e}")))?;
            match read {
                Ok(info) => {
                    kit_skills.push(KitSkill {
                        name: item.name.clone(),
                        dir,
                    });
                    tools.skills.push(RunSkill {
                        name: item.name,
                        info,
                        plugin: None,
                    });
                }
                Err(e) => notices.push(format!("skill {name}: {}", skill_problem(&dir, &e))),
            }
        }
        for (item, info) in &plugins {
            for (name, skill) in info.qualified_skills(&item.name) {
                tools.skills.push(RunSkill {
                    name,
                    info: skill.clone(),
                    plugin: Some(item.name.clone()),
                });
            }
        }

        if let Some(dir) = plugins_dir {
            let kit_plugins: Vec<KitPlugin> = plugins
                .iter()
                .map(|(item, info)| KitPlugin {
                    name: item.name.clone(),
                    dir: info.root.clone(),
                    run_hooks: matches!(&item.config, LibraryConfig::Plugin(c) if c.run_hooks),
                })
                .collect();
            let (dir, run_id) = (dir.to_path_buf(), run_id.to_string());
            let made = tokio::task::spawn_blocking(move || {
                kit::prepare_run_kit(&dir, &run_id, &kit_skills, &kit_plugins)
            })
            .await
            .map_err(|e| crate::Error::Other(format!("making plugin folders failed: {e}")))?;
            match made {
                Ok(kit) => {
                    notices.extend(
                        kit.left_out
                            .iter()
                            .map(|l| format!("{} {}: {}", l.kind.label(), l.name, l.reason)),
                    );
                    tools.kit = Some(Arc::new(kit));
                }
                Err(e) => notices.push(format!("the plugin folders could not be made: {e}")),
            }
        }
        Ok((tools, notices))
    }

    /// Follows the server's `notifications/tools/list_changed` for the run: each one lists the
    /// tools again and records them in the catalog; a list that is not the accepted one holds
    /// the server at once (spec section 6).
    fn watch(
        &self,
        server: &RunServer,
        mut changes: watch::Receiver<u64>,
        store: &Store,
        bus: &Bus,
    ) {
        let session = server.session.clone();
        let live = server.live.clone();
        let (store, bus) = (store.clone(), bus.clone());
        let (id, name) = (server.item_id.clone(), server.name.clone());
        let stop = self.stop.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    changed = changes.changed() => if changed.is_err() { return },
                }
                let listed = match session.list_tools().await {
                    Ok(listed) => listed,
                    Err(e) => {
                        tracing::warn!("{name}: listing its changed tools failed: {e}");
                        if session.is_stopped() {
                            return;
                        }
                        continue;
                    }
                };
                let accepted = match record_check(&store, &bus, &id, Ok(&listed)).await {
                    Ok(catalog) => catalog.status == CatalogStatus::Accepted,
                    Err(e) => {
                        tracing::warn!("{name}: recording its changed tools failed: {e}");
                        false
                    }
                };
                let mut live = live.lock().unwrap();
                live.tools = listed;
                live.held |= !accepted;
            }
        });
        self.watchers.lock().unwrap().push(handle);
    }

    /// Ends the run's tools: stops following list changes, closes every session (stdio servers
    /// are stopped) and deletes the plugin folders. Calling it twice does nothing more.
    pub async fn close(&self) {
        self.stop.cancel();
        for handle in std::mem::take(&mut *self.watchers.lock().unwrap()) {
            handle.abort();
        }
        let mut closing = JoinSet::new();
        for server in &self.servers {
            let session = server.session.clone();
            closing.spawn(async move { session.close().await });
        }
        while closing.join_next().await.is_some() {}
        if let Some(kit) = self.kit.clone() {
            let _ = tokio::task::spawn_blocking(move || kit.cleanup()).await;
        }
    }
}

impl Drop for RunTools {
    /// A run that ends without `close` (an error, a panic): the sessions stop as they are
    /// dropped; the watchers and the plugin folders go here.
    fn drop(&mut self) {
        self.stop.cancel();
        for handle in self.watchers.get_mut().unwrap().drain(..) {
            handle.abort();
        }
        if let Some(kit) = &self.kit {
            kit.cleanup();
        }
    }
}

/// Closes the run's tools when it is dropped, for a run that ends early through an error: the
/// close runs on its own task. The runner still closes them itself at the end of a run.
pub struct CloseOnDrop(pub Arc<RunTools>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let tools = self.0.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { tools.close().await });
        }
    }
}

type Opened = std::result::Result<(McpSession, Vec<McpTool>, watch::Receiver<u64>), McpError>;

/// Opens a session with each server at once and lists its tools; one outcome per server, in
/// order. `None` when `cancel` fires first: what was opened is closed.
async fn open_sessions(
    items: &[LibraryItem],
    workspace: &Path,
    cancel: &CancellationToken,
) -> Option<Vec<Opened>> {
    let mut set = JoinSet::new();
    for (i, item) in items.iter().enumerate() {
        let LibraryConfig::McpServer(config) = item.config.clone() else {
            continue;
        };
        let name = item.name.clone();
        let options = SessionOptions {
            cwd: Some(workspace.to_path_buf()),
            ..Default::default()
        };
        let cancel = cancel.clone();
        set.spawn(async move {
            let open = async {
                let session = McpSession::open(&name, &config, &process_env, options).await?;
                let changes = session.list_changed();
                match session.list_tools().await {
                    Ok(listed) => Ok((session, listed, changes)),
                    Err(e) => {
                        session.close().await;
                        Err(e)
                    }
                }
            };
            tokio::select! {
                opened = open => (i, Some(opened)),
                _ = cancel.cancelled() => (i, None),
            }
        });
    }
    let mut outcomes: Vec<Option<Opened>> = (0..items.len()).map(|_| None).collect();
    let mut cancelled = false;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((i, Some(opened))) => outcomes[i] = Some(opened),
            Ok((_, None)) => cancelled = true,
            Err(e) => tracing::error!("opening an MCP server failed: {e}"),
        }
    }
    if cancelled || cancel.is_cancelled() {
        for (session, _, _) in outcomes.into_iter().flatten().flatten() {
            session.close().await;
        }
        return None;
    }
    Some(
        outcomes
            .into_iter()
            .map(|o| {
                o.unwrap_or_else(|| Err(McpError::Start("the server could not be opened".into())))
            })
            .collect(),
    )
}

/// The accepted list's tools with their read-only hint.
fn accepted_tools(catalog: &McpCatalog) -> HashMap<String, bool> {
    catalog
        .accepted_tools
        .clone()
        .and_then(|v| serde_json::from_value::<Vec<McpTool>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.name.clone(), t.read_only()))
        .collect()
}

fn held_notice(name: &str, catalog: &McpCatalog) -> Option<String> {
    if catalog.status == CatalogStatus::Accepted {
        return None;
    }
    Some(match catalog.accepted_fingerprint {
        Some(_) => format!(
            "{name}: its tool list changed since it was accepted; its tools ask until the \
             change is accepted in the Library"
        ),
        None => format!(
            "{name}: its tool list has not been accepted yet; its tools ask until it is \
             accepted in the Library"
        ),
    })
}

/// Deletes the plugin folders of runs (`run-*` in `plugins_dir`) that a crash left behind.
/// Call it when no run is running.
pub fn sweep_plugin_folders(plugins_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_run = entry.file_name().to_string_lossy().starts_with("run-")
            && entry.file_type().is_ok_and(|t| t.is_dir());
        if is_run {
            if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!("could not delete {}: {e}", entry.path().display());
            }
        }
    }
}
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod kit;
pub mod library;
pub mod mcp;
pub mod model;
```

**with:**

```rust
pub mod kit;
pub mod library;
pub mod library_service;
pub mod mcp;
pub mod model;
```

**In `crates/dots-core/src/lib.rs`, replace:**

```rust
pub mod proc;
pub mod prompt;
pub mod runner;
pub mod runtime;
```

**with:**

```rust
pub mod proc;
pub mod prompt;
pub mod run_tools;
pub mod runner;
pub mod runtime;
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
};
use crate::prompt::build_prompt;
use crate::store::Store;
use crate::util::{ct_eq, new_token};
```

**with:**

```rust
};
use crate::prompt::build_prompt;
use crate::run_tools::{CloseOnDrop, OpenRequest, RunTools};
use crate::store::Store;
use crate::util::{ct_eq, new_token};
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    pub worktrees_dir: PathBuf,
    pub cancel_grace: Duration,
}

```

**with:**

```rust
    pub worktrees_dir: PathBuf,
    pub cancel_grace: Duration,
    /// Where a Claude-engine run's plugin folders go (`<data_dir>/plugins`).
    pub plugins_dir: PathBuf,
}

```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            }
        };
        if cancel.is_cancelled() {
            if let Err(e) = self.workspaces.cleanup_if_unchanged(&dot, &prepared).await {
                tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
            }
            return self.finish_cancelled_before_start(&run.id).await;
        }
        // A resume run on an engine that performs approved calls itself is handed the calls
```

**with:**

```rust
            }
        };
        // The run's MCP sessions, skills and plugin folders (MCP/skills spec section 5).
        let (tools, notices) = RunTools::open(OpenRequest {
            store: &self.store,
            bus: &self.bus,
            run_id: &run.id,
            uses: &dot.spec.uses,
            workspace: &prepared.path,
            plugins_dir: (dot.spec.engine == EngineKind::Claude)
                .then_some(self.cfg.plugins_dir.as_path()),
            cancel: &cancel,
        })
        .await?;
        let tools = Arc::new(tools);
        let _closer = CloseOnDrop(tools.clone());
        if cancel.is_cancelled() {
            tools.close().await;
            if let Err(e) = self.workspaces.cleanup_if_unchanged(&dot, &prepared).await {
                tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
            }
            return self.finish_cancelled_before_start(&run.id).await;
        }
        for text in notices {
            self.record(&run.id, &EngineEvent::Notice { text }).await?;
        }
        // A resume run on an engine that performs approved calls itself is handed the calls
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            approved_calls,
            mcp_servers,
        };
        let mut rx = engine.start(ctx).await?;
```

**with:**

```rust
            approved_calls,
            mcp_servers,
            tools: tools.clone(),
        };
        let mut rx = engine.start(ctx).await?;
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            }
        }

        let user_cancelled = by_user.load(Ordering::SeqCst);
```

**with:**

```rust
            }
        }
        tools.close().await;

        let user_cancelled = by_user.load(Ordering::SeqCst);
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
use crate::engine::{Engine, UnavailableEngine};
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::model::{
    Approval, AttentionRun, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun,
```

**with:**

```rust
use crate::engine::{Engine, UnavailableEngine};
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::library_service::LibraryService;
use crate::model::{
    Approval, AttentionRun, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    clock: Arc<dyn Clock>,
    port: u16,
```

**with:**

```rust
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    library: LibraryService,
    clock: Arc<dyn Clock>,
    port: u16,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
        // migration 0005 gets one now, and a row a failed change left stale is reset.
```

**with:**

```rust
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }
        // No run is running yet: plugin folders of runs are what a crash left behind.
        crate::run_tools::sweep_plugin_folders(&cfg.data_dir.join("plugins"));
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
        // migration 0005 gets one now, and a row a failed change left stale is reset.
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
                worktrees_dir: cfg.data_dir.join("worktrees"),
                cancel_grace: Duration::from_secs(10),
            },
        );
```

**with:**

```rust
                worktrees_dir: cfg.data_dir.join("worktrees"),
                cancel_grace: Duration::from_secs(10),
                plugins_dir: cfg.data_dir.join("plugins"),
            },
        );
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
        scheduler.spawn(shutdown.clone());
        Ok(Runtime {
            store,
            bus,
```

**with:**

```rust
        scheduler.spawn(shutdown.clone());
        Ok(Runtime {
            library: LibraryService::new(store.clone(), bus.clone()),
            store,
            bus,
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust

    /// Pending approvals, the finished, unreviewed thread ends that failed or kept a worktree
    /// with changes, and the schedules that need attention.
    pub async fn inbox(&self) -> Result<Inbox> {
        let mut names = std::collections::HashMap::new();
```

**with:**

```rust

    /// Pending approvals, the finished, unreviewed thread ends that failed or kept a worktree
    /// with changes, the schedules that need attention, and the MCP servers whose tool list
    /// waits for review.
    pub async fn inbox(&self) -> Result<Inbox> {
        let mut names = std::collections::HashMap::new();
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            review_runs,
            attention: self.store.list_schedule_attention().await?,
        })
    }
```

**with:**

```rust
            review_runs,
            attention: self.store.list_schedule_attention().await?,
            tools_changed: self.library.tools_changed().await?,
        })
    }
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub async fn inbox_count(&self) -> Result<usize> {
        let inbox = self.inbox().await?;
        Ok(inbox.approvals.len() + inbox.review_runs.len() + inbox.attention.len())
    }

```

**with:**

```rust
    pub async fn inbox_count(&self) -> Result<usize> {
        let inbox = self.inbox().await?;
        Ok(inbox.approvals.len()
            + inbox.review_runs.len()
            + inbox.attention.len()
            + inbox.tools_changed.len())
    }

```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
    pub fn store(&self) -> &Store {
        &self.store
    }

```

**with:**

```rust
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The Library: what the `library_*` commands call (MCP/skills spec section 9).
    pub fn library(&self) -> &LibraryService {
        &self.library
    }

```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
/// part of its type, so a new `EngineEvent` variant does not compile until it has a name
/// here and a sample in `engine_events`.
fn engine_event_sample<'a>(samples: &'a [EngineEvent; 9], e: &EngineEvent) -> &'a EngineEvent {
    let [started, text, call, result, usage, raw, message, finished, failed] = samples;
    match e {
        EngineEvent::SessionStarted { .. } => started,
```

**with:**

```rust
/// part of its type, so a new `EngineEvent` variant does not compile until it has a name
/// here and a sample in `engine_events`.
fn engine_event_sample<'a>(samples: &'a [EngineEvent; 10], e: &EngineEvent) -> &'a EngineEvent {
    let [started, text, call, result, usage, raw, message, finished, failed, notice] = samples;
    match e {
        EngineEvent::SessionStarted { .. } => started,
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        EngineEvent::Finished { .. } => finished,
        EngineEvent::Failed { .. } => failed,
    }
}
```

**with:**

```rust
        EngineEvent::Finished { .. } => finished,
        EngineEvent::Failed { .. } => failed,
        EngineEvent::Notice { .. } => notice,
    }
}
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        EngineEvent::Failed {
            error: "timeout".into(),
        },
    ];
```

**with:**

```rust
        EngineEvent::Failed {
            error: "timeout".into(),
        },
        EngineEvent::Notice {
            text: "skill pdf: folder not found".into(),
        },
    ];
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
                failed_at: TS.into(),
            }],
        }),
        "WorktreeDiff": to(WorktreeDiff {
```

**with:**

```rust
                failed_at: TS.into(),
            }],
            tools_changed: vec![ToolsChanged {
                server_id: "01J0SRV".into(),
                name: "github".into(),
                dots: vec![DotRef {
                    id: "01J0DOT".into(),
                    name: "nightly-tests".into(),
                }],
            }],
        }),
        "WorktreeDiff": to(WorktreeDiff {
```

**In `ui/src/api/types.ts`, replace:**

```ts
  z.strictObject({ kind: z.literal("finished"), summary: z.string() }),
  z.strictObject({ kind: z.literal("failed"), error: z.string() }),
]);

```

**with:**

```ts
  z.strictObject({ kind: z.literal("finished"), summary: z.string() }),
  z.strictObject({ kind: z.literal("failed"), error: z.string() }),
  // Something about the run that is not the model's doing: an item left out, a server problem.
  z.strictObject({ kind: z.literal("notice"), text: z.string() }),
]);

```

**In `ui/src/api/types.ts`, replace:**

```ts
  failed_at: z.string(),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
```

**with:**

```ts
  failed_at: z.string(),
});
export const DotRef = z.strictObject({ id: z.string(), name: z.string() });
// An MCP server whose tool list waits for review; its tools ask until it is accepted.
export const ToolsChanged = z.strictObject({
  server_id: z.string(),
  name: z.string(),
  dots: z.array(DotRef),
});
export const Inbox = z.strictObject({
  approvals: z.array(PendingApproval),
```

**In `ui/src/api/types.ts`, replace:**

```ts
  // Newest failure first.
  attention: z.array(NeedsAttention),
});

```

**with:**

```ts
  // Newest failure first.
  attention: z.array(NeedsAttention),
  // By server name.
  tools_changed: z.array(ToolsChanged),
});

```

**In `ui/src/api/types.ts`, replace:**

```ts
export type ReviewRun = z.infer<typeof ReviewRun>;
export type NeedsAttention = z.infer<typeof NeedsAttention>;
export type Inbox = z.infer<typeof Inbox>;
export type FileDiff = z.infer<typeof FileDiff>;
```

**with:**

```ts
export type ReviewRun = z.infer<typeof ReviewRun>;
export type NeedsAttention = z.infer<typeof NeedsAttention>;
export type DotRef = z.infer<typeof DotRef>;
export type ToolsChanged = z.infer<typeof ToolsChanged>;
export type Inbox = z.infer<typeof Inbox>;
export type FileDiff = z.infer<typeof FileDiff>;
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
        items.push({ type: "raw", seq: record.seq, label: "Unparsed output", text: event.line });
        break;
      case "session_started":
      case "usage":
```

**with:**

```ts
        items.push({ type: "raw", seq: record.seq, label: "Unparsed output", text: event.line });
        break;
      case "notice":
        items.push({ type: "raw", seq: record.seq, label: "Note", text: event.text });
        break;
      case "session_started":
      case "usage":
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
  // The same count as the tray's (`inbox_count`).
  const waiting = inbox
    ? inbox.approvals.length + inbox.review_runs.length + inbox.attention.length
    : 0;
  return (
```

**with:**

```tsx
  // The same count as the tray's (`inbox_count`).
  const waiting = inbox
    ? inbox.approvals.length +
      inbox.review_runs.length +
      inbox.attention.length +
      inbox.tools_changed.length
    : 0;
  return (
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
                             plugin folders (copies without `.mcp.json`, `monitors/`, hooks
                             unless allowed, or `allowed-tools`; MCP/skills spec §7)
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
```

**with:**

```markdown
                             plugin folders (copies without `.mcp.json`, `monitors/`, hooks
                             unless allowed, or `allowed-tools`; MCP/skills spec §7)
    src/library_service.rs   the Library at runtime: add, change, remove (asking first when dots
                             use an item), probe, check and accept a server's tool list, a
                             plugin's own MCP server items, `LibraryUpdated` (MCP/skills spec §9)
    src/run_tools.rs         a run's tools, opened before its engine starts and closed when it
                             ends: MCP sessions checked against their catalogs (held when the
                             list changed), skills, plugin folders, items left out as run events
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

````markdown
    Finished { summary: String },
    Failed { error: String },
}
```
````

**with:**

````markdown
    Finished { summary: String },
    Failed { error: String },
    // From MCP/skills: something about the run that is not the model's doing, recorded by
    // the runner (a Library item left out, an MCP server that could not start or is held).
    Notice { text: String },
}
```
````

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  for a retry (`retry_of`).
- **Inbox** — pending approvals (tool, pretty-printed input, approve / deny with note),
  schedules that need attention (from Scheduler v2) and finished runs awaiting review
  (definition in §12). Unread count mirrored in the tray tooltip and the sidebar.
  From Approvals v2 an approval card, wherever it shows (Inbox, run view, chat), also offers
  "Approve and always allow": the allow rules the core derived for that call, the narrowest
```

**with:**

```markdown
  for a retry (`retry_of`).
- **Inbox** — pending approvals (tool, pretty-printed input, approve / deny with note),
  schedules that need attention (from Scheduler v2), finished runs awaiting review
  (definition in §12) and, from MCP/skills, "Tools changed": the MCP servers whose tool list
  changed since it was accepted, or was never accepted, with the dots that use them and
  "Review" (MCP/skills spec §8). Unread count mirrored in the tray tooltip and the sidebar.
  From Approvals v2 an approval card, wherever it shows (Inbox, run view, chat), also offers
  "Approve and always allow": the allow rules the core derived for that call, the narrowest
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
   `inbox() -> { approvals, review_runs, attention }` (from Scheduler v2, `attention`
   lists the schedules that need attention: `dot_id, dot_name, run_id, error, failed_at`,
   newest first), `inbox_count()` (counts all three), and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, refuses one with no slot left as a create or
```

**with:**

```markdown
   `inbox() -> { approvals, review_runs, attention }` (from Scheduler v2, `attention`
   lists the schedules that need attention: `dot_id, dot_name, run_id, error, failed_at`,
   newest first; from MCP/skills, `tools_changed` lists the MCP servers whose catalog is
   `changed`: `server_id, name, dots: [{id, name}]`, by name), `inbox_count()` (counts all
   four), and
   `preview_schedule(schedule, n) -> next n slots` (from Scheduler v2 it takes the schedule
   object, validates it like `DotSpec::validate`, refuses one with no slot left as a create or
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- pending approvals;
- from Scheduler v2, schedules that need attention (an occurrence failed in the end);
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.
```

**with:**

```markdown
- pending approvals;
- from Scheduler v2, schedules that need attention (an occurrence failed in the end);
- from MCP/skills, MCP servers whose tools changed: the catalog's status is `changed` (a check
  found a list that is not the accepted one, or a server's first list waits to be accepted).
  The server leaves the Inbox when its list is accepted;
- runs awaiting review: the latest run of each lineage that is terminal, has `reviewed_at`
  NULL, and either kept its worktree (worktree mode, worktree still on disk) or `failed`.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review + schedules that need
attention; the sidebar's Inbox badge shows the same count.

The "Needs attention" group stands between the approvals and the runs to review. Each item
```

**with:**

```markdown
A run leaves the Inbox through "Discard worktree" or "Mark reviewed". Folder-mode successes only
notify. Unread count = pending approvals + runs awaiting review + schedules that need
attention + MCP servers whose tools changed; the sidebar's Inbox badge shows the same count.

The "Needs attention" group stands between the approvals and the runs to review. Each item
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. `RunEventData` gains the `notice` sample (10 samples), and the `Inbox` sample gains a `tools_changed` item.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test claude_cli_test --test claude_engine_test --test engine_prompt_test --test library_service_test --test local_engine_test --test resume_test --test run_tools_test --test runner_test --test scheduler_test --test server_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/app/Layout.test.tsx src/lib/runtime-events.test.ts src/lib/transcript.test.ts src/screens/DotEditorScreen.test.tsx src/screens/InboxScreen.test.tsx src/api 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `claude_cli_test` 10 passed, `claude_engine_test` 8, `engine_prompt_test` 9, `library_service_test` 17, `local_engine_test` 16, `resume_test` 18, `run_tools_test` 7, `runner_test` 14 (the new `a_run_gets_its_tools_records_what_was_left_out_and_closes_them_at_the_end` among them), `scheduler_test` 34, `server_test` 8; `dots-app` 49 passed; every UI file of the run passes.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 555 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 293 passed in 21 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs crates/dots-core/src/engine/mod.rs crates/dots-core/src/events.rs crates/dots-core/src/kit/installed.rs crates/dots-core/src/lib.rs crates/dots-core/src/library_service.rs crates/dots-core/src/model.rs crates/dots-core/src/run_tools.rs crates/dots-core/src/runner.rs crates/dots-core/src/runtime.rs crates/dots-core/src/store/library.rs crates/dots-core/tests/claude_cli_test.rs crates/dots-core/tests/claude_engine_test.rs crates/dots-core/tests/common/fake_mcp.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/library_service_test.rs crates/dots-core/tests/local_engine_test.rs crates/dots-core/tests/resume_test.rs crates/dots-core/tests/run_tools_test.rs crates/dots-core/tests/runner_test.rs crates/dots-core/tests/scheduler_test.rs crates/dots-core/tests/server_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/types.ts ui/src/app/Layout.test.tsx ui/src/app/Layout.tsx ui/src/lib/runtime-events.test.ts ui/src/lib/transcript.test.ts ui/src/lib/transcript.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/InboxScreen.test.tsx ui/src/test/fixtures.ts
git commit -m "feat(core): open a run's MCP sessions, skills and plugin folders, and serve the Library at runtime"
```

### Task 6: Give local-engine runs their MCP tools and a `skill` tool

A local-engine run now uses what `RunTools` opened (spec sections 5 and 7). Each turn the model is offered the seven file tools, `skill` when the run has skills, and every open server's current tools as OpenAI functions named `mcp__<server>__<tool>`, shortened to 64 characters with a per-run map back. A call is shown as a `ToolCall` with its full name, checked by `ApprovalHub::check_mcp` (the six steps of Task 1 with the server's trust), and, if allowed, sent; its result is reported like a built-in tool's: text parts joined, other parts named, `isError` as an error, 30 KB at most. An ask creates an approval that carries the server's accepted fingerprint, and on resume an approved call is skipped when that fingerprint moved. Before each check and each turn the engine waits until every announced list change is listed and checked, so that a change holds the server before the next call. The `skill` tool reads a skill's `SKILL.md` and list of files, or one file inside the skill's folder, and the system message lists the skills. The main spec's sections 2, 4 and 5 follow, and its section 1 no longer lists MCP servers for local-engine dots as out of scope.

**Files:**
- Create: `crates/dots-core/src/engine/local/mcp_tools.rs`, `crates/dots-core/src/engine/local/tools/skill.rs`, `crates/dots-core/tests/local_mcp_test.rs`, `crates/dots-core/tests/local_skill_test.rs`
- Modify: `apps/dots-app/src/contract.rs`, `apps/dots-app/src/events.rs`, `crates/dots-core/src/approvals.rs`, `crates/dots-core/src/engine/local/mod.rs`, `crates/dots-core/src/engine/local/tools/mod.rs`, `crates/dots-core/src/engine/mod.rs`, `crates/dots-core/src/model.rs`, `crates/dots-core/src/run_tools.rs`, `crates/dots-core/src/store/approvals.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/types.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `crates/dots-core/tests/local_mcp_test.rs`, `crates/dots-core/tests/local_skill_test.rs`, `crates/dots-core/tests/local_tools_test.rs`

**Interfaces:**
- Consumes: Task 1's `resolve_mcp` and `McpTrust`, Task 3's `call_tool`, `CALL_TIMEOUT` and `McpError`, Task 5's `RunTools`, `RunServer` and `RunSkill`; the local engine's `Session`, `ToolOutput`, `cap_head`, `MAX_OUTPUT` (30 KB), `MAX_ENTRIES` and `MAX_READ_BYTES`; the `ApprovalHub` and `apply_approved` of Approvals v2.
- Produces:
  - `PermissionGate::check_mcp(&self, run_id: &str, tool: &str, input: Value, trust: McpTrust, fingerprint: Option<String>) -> Result<PermissionOutcome>`; the default implementation denies ("<tool> cannot be checked, so it is not permitted."). `ApprovalHub` implements it with `resolve_mcp`: allow, deny with the same text as `check`, or ask with an approval row that stores `fingerprint`. It takes no grant (Task 7 adds that).
  - `Approval.mcp_fingerprint: Option<String>` (JSON `"mcp_fingerprint": string | null`); `Store::create_mcp_approval(&self, run_id: &str, tool: &str, input: &Value, self_apply: bool, mcp_fingerprint: Option<&str>) -> Result<Approval>`; `create_approval_with` delegates to it with `None`.
  - `RunServer::settled(&self)` (async): waits, at most `mcp::ANSWER_TIMEOUT`, until every announced list change is listed and checked.
  - In `dots_core::engine::local::mcp_tools`: `MAX_NAME = 64`; `model_name(qualified: &str) -> String` (unchanged when it fits `^[A-Za-z0-9_-]{1,64}$`, else 55 characters with others as `_`, `_`, and 8 hex characters of its SHA-256); `format_result(result: &Value) -> ToolOutput` (text parts joined with a newline, `[<type> omitted]`, `structuredContent` without any part, else "(no output)", `isError` as an error, capped at 30 KB).
  - `tools::TOOLS` gains `("skill", "Skill")`; `tools::schemas()` stays the seven file tools. In `tools::skill`: `schema() -> Value` (function `skill`, `name` required, `file` optional); `prompt_section(skills: &[RunSkill]) -> String` (empty without skills; "Skills you can use. Call the tool `skill` with a name to read one before you follow it." and a line `- <name>: <listing>` per skill); `run(skills: &[RunSkill], input: &Value) -> ToolOutput` (the body with `${CLAUDE_SKILL_DIR}` replaced and the list of files; or the named file, refused when it is absolute, holds `..` or `:`, or resolves outside the folder).
  - Approved MCP calls on resume: an approval whose server is open and whose accepted fingerprint differs is skipped with "- Approval #<id>, <tool> <input>: not performed, because the tool changed since it was approved.", and the calls after it are given up; a server that is not open gives a failed outcome.
  - Contract and UI: `mcp_fingerprint: None` in the app's `Approval` literals; `approvalShape.mcp_fingerprint: nullable(z.string())`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/local_mcp_test.rs`:**

```rust
//! The local engine's MCP tools (MCP/skills spec sections 4 and 5): the tools of the run's open
//! sessions in the schemas, each call checked by `Policy::resolve_mcp` through the approval
//! hub, results reported as text, a server held when its list changes during the run, and an
//! approved call performed on resume only while the server's accepted list is the one it was
//! approved under.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::mcp_tools::{format_result, model_name, MAX_NAME};
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus};
use dots_core::library::{CatalogStatus, LibraryConfig, McpServerConfig};
use dots_core::library_service::LibraryService;
use dots_core::model::{
    ApplyState, Approval, Dot, EngineKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind,
    WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use llm::{call, calls, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    _dir: TempDir,
    ws: PathBuf,
    store: Store,
    bus: Bus,
    runner: Arc<Runner>,
}

async fn env(dispatch: bool) -> Env {
    let (dir, store) = temp_store().await;
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine =
        LocalEngine::with_retry_delays(store.clone(), hub.clone(), vec![Duration::ZERO; 2]);
    hub.set_self_apply([EngineKind::Local]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    if dispatch {
        runner.spawn_dispatcher(CancellationToken::new());
    }
    Env {
        _dir: dir,
        ws,
        store,
        bus,
        runner,
    }
}

/// Adds `config` to the Library as `name`, its tool list accepted as it is now. Returns the
/// accepted fingerprint.
async fn add_server(e: &Env, name: &str, config: McpServerConfig) -> String {
    let lib = LibraryService::new(e.store.clone(), e.bus.clone());
    let probe = lib.probe(&config).await.unwrap();
    lib.add(
        name,
        LibraryConfig::McpServer(config),
        Some(&probe.fingerprint),
    )
    .await
    .unwrap();
    probe.fingerprint
}

/// A local dot that uses the Library servers `servers`, under `policy`. Asking parks at once.
async fn local_dot(e: &Env, llm: &Llm, policy: Policy, servers: &[&str]) -> Dot {
    let mut s = spec("loco");
    s.workdir = e.ws.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "test-model".into();
    s.policy = policy;
    s.approval_wait_secs = 0;
    s.uses.mcp_servers = servers.iter().map(|n| n.to_string()).collect();
    e.store.create_dot(&s).await.unwrap()
}

async fn run(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

async fn events(e: &Env, run_id: &str, kind: &str) -> Vec<RunEventRecord> {
    e.store
        .list_events(run_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == kind)
        .collect()
}

/// The function definitions of a recorded request, by name.
fn schemas(req: &Value) -> HashMap<String, Value> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["function"]["name"].as_str().unwrap().to_string(),
                t.clone(),
            )
        })
        .collect()
}

/// The tools `fake` was called with, in order.
fn called(fake: &FakeMcp) -> Vec<String> {
    fake.received("tools/call")
        .iter()
        .map(|m| m["params"]["name"].as_str().unwrap().to_string())
        .collect()
}

fn hash8(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_name_longer_than_64_characters_is_cut_and_hashed() {
    assert_eq!(MAX_NAME, 64);
    assert_eq!(model_name("mcp__files__look"), "mcp__files__look");
    let exact = format!("mcp__{}__{}", "s".repeat(40), "t".repeat(17));
    assert_eq!(exact.len(), 64);
    assert_eq!(model_name(&exact), exact);

    let long = format!("mcp__{}__{}", "s".repeat(40), "t".repeat(30));
    let short = model_name(&long);
    assert_eq!(short.len(), 64);
    assert_eq!(short, format!("{}_{}", &long[..55], hash8(&long)));
    // Two long names that share their first 55 characters stay apart.
    let other = format!("mcp__{}__{}", "s".repeat(40), "t".repeat(31));
    assert_ne!(model_name(&other), short);

    // A character a function name cannot hold is replaced, and the hash keeps the name apart.
    let dotted = "mcp__files__do.it";
    assert_eq!(
        model_name(dotted),
        format!("mcp__files__do_it_{}", hash8(dotted))
    );
}

#[test]
fn a_result_is_its_text_parts_with_the_other_parts_named() {
    let out = format_result(&json!({"content": [
        {"type": "text", "text": "one"},
        {"type": "image", "data": "aGk=", "mimeType": "image/png"},
        {"type": "text", "text": "two"},
        {"type": "resource_link", "uri": "file:///x", "name": "x"},
        {"type": "audio", "data": "aGk=", "mimeType": "audio/wav"}
    ]}));
    assert_eq!(
        out.output,
        "one\n[image omitted]\ntwo\n[resource_link omitted]\n[audio omitted]"
    );
    assert!(!out.is_error);

    let failed = format_result(&json!({
        "content": [{"type": "text", "text": "bad input"}], "isError": true
    }));
    assert_eq!(
        (failed.output.as_str(), failed.is_error),
        ("bad input", true)
    );

    // No content: the structured result stands in; with neither, the model is told so.
    let structured = format_result(&json!({"content": [], "structuredContent": {"n": 1}}));
    assert_eq!(structured.output, r#"{"n":1}"#);
    assert_eq!(format_result(&json!({})).output, "(no output)");

    let big = format_result(&json!({"content": [{"type": "text", "text": "x".repeat(40_000)}]}));
    assert!(big.output.len() < 31 * 1024, "{}", big.output.len());
    assert!(big.output.contains("[truncated:"));
}

#[tokio::test]
async fn a_read_only_tool_runs_and_a_writing_tool_asks_with_the_accepted_fingerprint() {
    let e = env(true).await;
    let fake = FakeMcp::new(json!({"tools": [tool("look", true), tool("change", false)]}));
    let fingerprint = add_server(&e, "files", fake.stdio()).await;
    let llm = Llm::start(vec![
        calls(&[
            ("c1", "mcp__files__look", json!({"q": "a"})),
            ("c2", "mcp__files__change", json!({"q": "b"})),
        ]),
        text("Waiting for approval."),
    ])
    .await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Sandboxed), &["files"]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::AwaitingApproval, 20).await;

    let reqs = llm.requests();
    let tools = schemas(&reqs[0]);
    assert_eq!(tools.len(), 9, "{:?}", tools.keys());
    assert_eq!(
        tools["mcp__files__look"]["function"],
        json!({
            "name": "mcp__files__look",
            "description": "The look tool.",
            "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
        })
    );
    assert_eq!(tool_result(&reqs[1], "c1"), r#"look: {"q":"a"}"#);
    assert!(tool_result(&reqs[1], "c2").contains("Queued for human approval"));
    assert_eq!(called(&fake), ["look"]);

    let approvals = e.store.approvals_for_run(&r.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "mcp__files__change");
    assert_eq!(approvals[0].input, json!({"q": "b"}));
    assert_eq!(approvals[0].mcp_fingerprint.as_deref(), Some(&*fingerprint));
    assert!(approvals[0].self_apply);

    let shown: Vec<Value> = events(&e, &r.id, "tool_call")
        .await
        .iter()
        .map(|ev| ev.data["tool"].clone())
        .collect();
    assert_eq!(shown, ["mcp__files__look", "mcp__files__change"]);
    assert!(fake.gone_within(Duration::from_secs(10)).await);
}

#[tokio::test]
async fn the_dots_rules_and_mcp_default_come_before_and_after_the_read_only_hint() {
    let e = env(true).await;
    let fake = FakeMcp::new(json!({"tools": [
        tool("look", true), tool("peek", true), tool("change", false)
    ]}));
    add_server(&e, "files", fake.stdio()).await;
    let llm = Llm::start(vec![
        calls(&[
            ("c1", "mcp__files__look", json!({})),
            ("c2", "mcp__files__peek", json!({})),
            ("c3", "mcp__files__change", json!({})),
        ]),
        text("Done."),
    ])
    .await;
    // Read-only: `mcp_default` is deny. A deny rule beats a read-only hint.
    let mut policy = Policy::preset(Preset::ReadOnly);
    policy
        .rules
        .push(Rule::new("mcp__files__peek", Action::Deny));
    let dot = local_dot(&e, &llm, policy, &["files"]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 20).await;
    let reqs = llm.requests();
    assert_eq!(tool_result(&reqs[1], "c1"), "look: {}");
    for id in ["c2", "c3"] {
        let out = tool_result(&reqs[1], id);
        assert!(out.contains("Denied by the policy of dot 'loco'"), "{out}");
    }
    assert_eq!(called(&fake), ["look"]);
    assert!(e.store.approvals_for_run(&r.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn results_errors_and_a_dead_server_reach_the_model() {
    let e = env(true).await;
    let mut big = tool("big", true);
    big["result"] = json!({"content": [{"type": "text", "text": "y".repeat(40_000)}]});
    let mut mixed = tool("mixed", true);
    mixed["result"] = json!({"content": [
        {"type": "text", "text": "one"},
        {"type": "image", "data": "aGk=", "mimeType": "image/png"}
    ]});
    let mut fails = tool("fails", true);
    fails["result"] = json!({"content": [{"type": "text", "text": "bad input"}], "isError": true});
    let mut broken = tool("broken", true);
    broken["error"] = json!({"code": -32000, "message": "boom"});
    let mut dies = tool("dies", true);
    dies["exit"] = json!(true);
    let fake = FakeMcp::new(json!({"tools": [big, mixed, fails, broken, dies]}));
    add_server(&e, "files", fake.stdio()).await;
    let llm = Llm::start(vec![
        calls(&[
            ("big", "mcp__files__big", json!({})),
            ("mixed", "mcp__files__mixed", json!({})),
            ("fails", "mcp__files__fails", json!({})),
            ("broken", "mcp__files__broken", json!({})),
            ("dies", "mcp__files__dies", json!({})),
        ]),
        call("after", "mcp__files__mixed", json!({})),
        text("Done."),
    ])
    .await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Sandboxed), &["files"]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 20).await;
    let reqs = llm.requests();
    let big = tool_result(&reqs[1], "big");
    assert!(
        big.starts_with("yyyy") && big.contains("[truncated:"),
        "{}",
        big.len()
    );
    assert_eq!(tool_result(&reqs[1], "mixed"), "one\n[image omitted]");
    assert_eq!(tool_result(&reqs[1], "fails"), "bad input");
    assert_eq!(tool_result(&reqs[1], "broken"), "boom (error -32000)");
    assert_eq!(tool_result(&reqs[1], "dies"), "the server stopped");
    assert_eq!(tool_result(&reqs[2], "after"), "the server stopped");

    let errors: Vec<(String, bool)> = events(&e, &r.id, "tool_result")
        .await
        .iter()
        .map(|ev| {
            (
                ev.data["id"].as_str().unwrap().to_string(),
                ev.data["is_error"].as_bool().unwrap(),
            )
        })
        .collect();
    let expected = [
        ("big", false),
        ("mixed", false),
        ("fails", true),
        ("broken", true),
        ("dies", true),
        ("after", true),
    ];
    assert_eq!(
        errors,
        expected.map(|(id, err)| (id.to_string(), err)).to_vec()
    );
}

#[tokio::test]
async fn a_long_tool_name_is_shortened_for_the_model_and_mapped_back() {
    let e = env(true).await;
    let long_tool = "t".repeat(30);
    let fake = FakeMcp::new(json!({"tools": [tool(&long_tool, true)]}));
    let server = "s".repeat(40);
    add_server(&e, &server, fake.stdio()).await;
    let qualified = format!("mcp__{server}__{long_tool}");
    let short = model_name(&qualified);
    assert_eq!(short.len(), 64);
    let llm = Llm::start(vec![call("c1", &short, json!({"q": "z"})), text("Done.")]).await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Sandboxed), &[&server]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 20).await;
    let reqs = llm.requests();
    assert!(schemas(&reqs[0]).contains_key(&short));
    assert_eq!(
        tool_result(&reqs[1], "c1"),
        format!(r#"{long_tool}: {{"q":"z"}}"#)
    );
    assert_eq!(called(&fake), [long_tool]);
    // The run view and the policy see the real name.
    assert_eq!(
        events(&e, &r.id, "tool_call").await[0].data["tool"],
        qualified
    );
}

#[tokio::test]
async fn a_list_change_during_the_run_holds_the_server_at_once() {
    let e = env(true).await;
    let mut look = tool("look", true);
    look["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "tools": [look],
        "after_switch": [tool("look", true), tool("extra", true)]
    }));
    add_server(&e, "files", fake.stdio()).await;
    let llm = Llm::start(vec![
        call("c1", "mcp__files__look", json!({"q": "1"})),
        call("c2", "mcp__files__look", json!({"q": "2"})),
        text("Waiting."),
    ])
    .await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Sandboxed), &["files"]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::AwaitingApproval, 20).await;
    let reqs = llm.requests();
    assert_eq!(tool_result(&reqs[1], "c1"), r#"look: {"q":"1"}"#);
    // The next request already offers the new list, and the read-only tool now asks.
    assert!(schemas(&reqs[1]).contains_key("mcp__files__extra"));
    assert!(tool_result(&reqs[2], "c2").contains("Queued for human approval"));
    assert_eq!(called(&fake), ["look"]);
    let approvals = e.store.approvals_for_run(&r.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "mcp__files__look");
    let lib = LibraryService::new(e.store.clone(), e.bus.clone());
    let entry = lib
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.item.name == "files")
        .unwrap();
    assert_eq!(entry.catalog.unwrap().status, CatalogStatus::Changed);
}

#[tokio::test]
async fn a_call_to_a_server_the_run_does_not_have_is_an_error() {
    let e = env(true).await;
    let llm = Llm::start(vec![
        call("c1", "mcp__nowhere__look", json!({})),
        text("Done."),
    ])
    .await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Trusted), &[]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 20).await;
    let out = tool_result(&llm.requests()[1], "c1");
    assert!(out.contains("no MCP server named 'nowhere'"), "{out}");
    assert!(e.store.approvals_for_run(&r.id).await.unwrap().is_empty());
}

// ---- Approved calls performed on resume ----

const CONTINUE: &str = "Continue the task, then end with your summary.";

/// A parked parent run of `dot` whose approved MCP calls `(tool, input, fingerprint)` are
/// queued for Dots, and the resume run that is to perform them. Nothing has started it yet.
async fn queued_resume(
    e: &Env,
    dot: &Dot,
    calls: &[(&str, Value, Option<&str>)],
) -> (Run, Vec<Approval>) {
    let parent = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (tool, input, fingerprint) in calls {
        let a = e
            .store
            .create_mcp_approval(&parent.id, tool, input, true, *fingerprint)
            .await
            .unwrap();
        e.store
            .decide_approval_with_parked(&a.id, true, None, true)
            .await
            .unwrap();
        ids.push(a.id);
    }
    e.store
        .set_status(&parent.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Resume);
    new.payload = Some(json!({ "message": CONTINUE }));
    new.parent_run_id = Some(parent.id.clone());
    new.root_run_id = Some(parent.root_run_id.clone());
    let child = e
        .store
        .resume_parked(&parent.id, &[], &ids, &ids, Some(&new))
        .await
        .unwrap()
        .expect("resume run");
    let approvals = e.store.queued_applies(&parent.id).await.unwrap();
    assert_eq!(approvals.len(), calls.len());
    (child, approvals)
}

async fn perform(e: &Env, child: &Run) -> Run {
    assert_eq!(e.runner.dispatch_once().await.unwrap(), 1);
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await
}

async fn apply_state(e: &Env, approval: &Approval) -> Option<ApplyState> {
    e.store
        .get_approval(&approval.id)
        .await
        .unwrap()
        .apply_state
}

/// The user message of the resume run's first request.
fn report(llm: &Llm) -> String {
    let sent = messages(&llm.requests()[0]);
    sent[1]["content"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn an_approved_call_is_performed_while_the_accepted_list_is_unchanged() {
    let e = env(false).await;
    let fake = FakeMcp::new(json!({"tools": [tool("change", false)]}));
    let fingerprint = add_server(&e, "files", fake.stdio()).await;
    let llm = Llm::start(vec![text("All done.")]).await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Sandboxed), &["files"]).await;
    let input = json!({"q": "b"});
    let (child, approvals) = queued_resume(
        &e,
        &dot,
        &[("mcp__files__change", input.clone(), Some(&fingerprint))],
    )
    .await;
    perform(&e, &child).await;
    assert_eq!(called(&fake), ["change"]);
    assert_eq!(
        apply_state(&e, &approvals[0]).await,
        Some(ApplyState::Applied)
    );
    let told = report(&llm);
    let line = format!(
        "- Approval #{}, mcp__files__change {input}: done.\nchange: {input}",
        approvals[0].id
    );
    assert!(told.contains(&line), "{told}");
    let shown = &events(&e, &child.id, "tool_call").await[0];
    assert_eq!(shown.data["tool"], "mcp__files__change");
}

#[tokio::test]
async fn an_approved_call_is_skipped_when_the_tool_changed_since_it_was_approved() {
    let e = env(false).await;
    let fake = FakeMcp::new(json!({"tools": [tool("change", false)]}));
    add_server(&e, "files", fake.stdio()).await;
    std::fs::write(e.ws.join("README.md"), "hello\n").unwrap();
    let llm = Llm::start(vec![text("All done.")]).await;
    let dot = local_dot(&e, &llm, Policy::preset(Preset::Sandboxed), &["files"]).await;
    let (child, approvals) = queued_resume(
        &e,
        &dot,
        &[
            (
                "mcp__files__change",
                json!({"q": "b"}),
                Some("an older list"),
            ),
            ("Read", json!({"file_path": "README.md"}), None),
        ],
    )
    .await;
    perform(&e, &child).await;
    assert!(called(&fake).is_empty());
    for a in &approvals {
        assert_eq!(apply_state(&e, a).await, Some(ApplyState::Skipped));
    }
    let told = report(&llm);
    assert!(
        told.contains(&format!(
            "- Approval #{}, mcp__files__change {}: not performed, because the tool changed \
             since it was approved.",
            approvals[0].id, approvals[0].input
        )),
        "{told}"
    );
    // The call after it is given up, as after any call that did not succeed.
    assert!(
        told.contains(&format!(
            "- Approval #{}, Read {}: not performed, because an earlier approved action did \
             not complete.",
            approvals[1].id, approvals[1].input
        )),
        "{told}"
    );
    assert!(events(&e, &child.id, "tool_call").await.is_empty());
}
```

**Create `crates/dots-core/tests/local_skill_test.rs`:**

```rust
//! The local engine's `skill` tool and the system prompt's skills section (MCP/skills spec
//! section 7): a skill's body with `${CLAUDE_SKILL_DIR}` replaced and its files listed, a file
//! of the skill read only inside its folder, results capped at 30 KB.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::tools::{self, skill, MAX_OUTPUT};
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::kit;
use dots_core::library::{LibraryConfig, PluginConfig, SkillConfig};
use dots_core::library_service::LibraryService;
use dots_core::model::{EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::{Policy, Preset};
use dots_core::run_tools::RunSkill;
use dots_core::runner::{Runner, RunnerConfig};
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A skill folder `pdf` with a body that names its folder, two files and a `.git` folder.
fn pdf_skill(root: &Path) -> PathBuf {
    let dir = root.join("pdf");
    write(
        &dir.join("SKILL.md"),
        "---\nname: pdf\ndescription: Read PDFs.\nwhen_to_use: When a PDF is named.\n---\n\
         # PDF\n\nRun ${CLAUDE_SKILL_DIR}/scripts/x.py on the file.\n",
    );
    write(&dir.join("scripts/x.py"), "print('x')\n");
    write(&dir.join("ref.md"), "Reference.\n");
    write(&dir.join(".git/HEAD"), "ref: refs/heads/main\n");
    write(&root.join("outside.txt"), "secret\n");
    dir
}

fn skills(dir: &Path) -> Vec<RunSkill> {
    vec![RunSkill {
        name: "pdf".into(),
        info: kit::read_skill(dir).unwrap(),
        plugin: None,
    }]
}

async fn ask(skills: &[RunSkill], input: Value) -> tools::ToolOutput {
    skill::run(skills, &input).await
}

#[tokio::test]
async fn without_a_file_the_body_comes_with_the_skills_files() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    let out = ask(&skills(&dir), json!({"name": "pdf"})).await;
    assert!(!out.is_error, "{}", out.output);
    assert_eq!(
        out.output,
        format!(
            "# PDF\n\nRun {}/scripts/x.py on the file.\n\n\
             Files in this skill (read one with `file`):\n- ref.md\n- scripts/x.py",
            dir.display()
        )
    );
    // An empty `file` is no file.
    let blank = ask(&skills(&dir), json!({"name": "pdf", "file": " "})).await;
    assert_eq!(blank, out);
}

#[tokio::test]
async fn a_file_is_read_only_inside_the_skill_folder() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    let s = skills(&dir);
    let read = ask(&s, json!({"name": "pdf", "file": "scripts/x.py"})).await;
    assert_eq!(
        (read.output.as_str(), read.is_error),
        ("print('x')\n", false)
    );
    let raw = ask(&s, json!({"name": "pdf", "file": "SKILL.md"})).await;
    assert!(
        raw.output.contains("${CLAUDE_SKILL_DIR}"),
        "the file as it is"
    );

    let outside = root.path().join("outside.txt");
    for file in [
        "../outside.txt".to_string(),
        "scripts/../../outside.txt".to_string(),
        outside.to_string_lossy().into_owned(),
    ] {
        let out = ask(&s, json!({"name": "pdf", "file": file})).await;
        assert!(out.is_error, "{file}: {}", out.output);
        assert!(!out.output.contains("secret"), "{file}");
        assert!(
            out.output.contains("inside the skill folder"),
            "{file}: {}",
            out.output
        );
    }
    let missing = ask(&s, json!({"name": "pdf", "file": "nope.md"})).await;
    assert!(
        missing.is_error && missing.output.contains("nope.md"),
        "{}",
        missing.output
    );
    let folder = ask(&s, json!({"name": "pdf", "file": "scripts"})).await;
    assert!(
        folder.is_error && folder.output.contains("is a folder"),
        "{}",
        folder.output
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_out_of_the_skill_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    std::os::unix::fs::symlink(root.path().join("outside.txt"), dir.join("link.txt")).unwrap();
    let out = ask(&skills(&dir), json!({"name": "pdf", "file": "link.txt"})).await;
    assert!(
        out.is_error && !out.output.contains("secret"),
        "{}",
        out.output
    );
}

#[tokio::test]
async fn an_unknown_skill_names_the_skills_there_are() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    let out = ask(&skills(&dir), json!({"name": "word"})).await;
    assert!(out.is_error);
    assert_eq!(out.output, "unknown skill 'word'. Skills: pdf");
    let none = ask(&[], json!({"name": "word"})).await;
    assert_eq!(none.output, "unknown skill 'word'. This run has no skills.");
}

#[tokio::test]
async fn results_are_capped_at_30_kb() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    write(&dir.join("big.md"), &"z".repeat(40_000));
    let out = ask(&skills(&dir), json!({"name": "pdf", "file": "big.md"})).await;
    assert!(!out.is_error);
    assert!(out.output.len() < MAX_OUTPUT + 100, "{}", out.output.len());
    assert!(out.output.contains("[truncated:"));
}

#[tokio::test]
async fn the_prompt_section_lists_each_skill_on_one_line() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    assert_eq!(skill::prompt_section(&[]), "");
    let mut list = skills(&dir);
    list.push(RunSkill {
        name: "kit:fill".into(),
        plugin: Some("kit".into()),
        ..list[0].clone()
    });
    assert_eq!(
        skill::prompt_section(&list),
        "Skills you can use. Call the tool `skill` with a name to read one before you follow \
         it.\n- pdf: Read PDFs. When a PDF is named.\n- kit:fill: Read PDFs. When a PDF is named."
    );
}

// ---- In a run ----

#[tokio::test]
async fn a_local_run_lists_its_skills_and_reads_one_with_the_skill_tool() {
    let (dir, store) = temp_store().await;
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());

    let lib = LibraryService::new(store.clone(), bus);
    let skills_root: TempDir = tempfile::tempdir().unwrap();
    let pdf = pdf_skill(skills_root.path());
    let plugin = skills_root.path().join("kit");
    write(
        &plugin.join(".claude-plugin/plugin.json"),
        r#"{"name": "kit"}"#,
    );
    write(
        &plugin.join("skills/fill/SKILL.md"),
        "---\nname: fill\ndescription: Fill forms.\n---\nAsk for the form.\n",
    );
    let path = |p: &Path| p.to_string_lossy().into_owned();
    lib.add(
        "pdf",
        LibraryConfig::Skill(SkillConfig { path: path(&pdf) }),
        None,
    )
    .await
    .unwrap();
    lib.add(
        "kit",
        LibraryConfig::Plugin(PluginConfig {
            path: path(&plugin),
            installed_id: None,
            run_hooks: false,
        }),
        None,
    )
    .await
    .unwrap();

    let llm = Llm::start(vec![
        call("c1", "skill", json!({"name": "kit:fill"})),
        text("Read it."),
    ])
    .await;
    let mut s = spec("skilled");
    s.workdir = path(&ws);
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "test-model".into();
    // Every preset allows `Skill`, Read-only too.
    s.policy = Policy::preset(Preset::ReadOnly);
    s.uses.skills = vec!["pdf".into()];
    s.uses.plugins = vec!["kit".into()];
    let dot = store.create_dot(&s).await.unwrap();
    let r = runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&store, &r.id, RunStatus::Succeeded, 20).await;

    let reqs = llm.requests();
    let system = messages(&reqs[0])[0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        system.ends_with(
            "\n\nSkills you can use. Call the tool `skill` with a name to read one before you \
             follow it.\n- pdf: Read PDFs. When a PDF is named.\n- kit:fill: Fill forms."
        ),
        "{system}"
    );
    let names: Vec<&str> = reqs[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"skill"));
    assert_eq!(tool_result(&reqs[1], "c1"), "Ask for the form.");
    let shown = store
        .list_events(&r.id, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|ev| ev.kind == "tool_call")
        .unwrap();
    assert_eq!(shown.data["tool"], "Skill");
    assert_eq!(shown.data["input"], json!({"name": "kit:fill"}));
}

#[test]
fn skill_is_a_local_tool_with_the_policy_name_skill() {
    assert_eq!(tools::alias("skill"), Some("Skill"));
    assert_eq!(tools::local_name("Skill"), Some("skill"));
    let ws = tempfile::tempdir().unwrap();
    let prepared = tools::prepare(
        "skill",
        &json!({"name": "pdf", "file": "a.md", "extra": 1}),
        ws.path(),
    )
    .unwrap();
    assert_eq!(prepared.input, json!({"name": "pdf", "file": "a.md"}));
    let err = tools::prepare("skill", &json!({}), ws.path()).unwrap_err();
    assert!(err.contains("'name'"), "{err}");
    assert_eq!(
        skill::schema()["function"]["parameters"]["required"],
        json!(["name"])
    );
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
    assert!(err.contains(".."), "{err}");
    assert_eq!(tools::alias("grep"), Some("Grep"));
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len()
    );
}
```

**with:**

```rust
    assert!(err.contains(".."), "{err}");
    assert_eq!(tools::alias("grep"), Some("Grep"));
    // Every local tool but `skill`, which a run gets only with skills (`skill::schema`).
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len() - 1
    );
}
```

**In `crates/dots-core/tests/local_tools_test.rs`, replace:**

```rust
        ("shell", json!({"command": "git status", "timeout": 30})),
        ("shell", json!("{\"command\": \"git log -1\"}")),
    ];
    let mut tools_seen = std::collections::BTreeSet::new();
```

**with:**

```rust
        ("shell", json!({"command": "git status", "timeout": 30})),
        ("shell", json!("{\"command\": \"git log -1\"}")),
        ("skill", json!({"name": "pdf", "file": "ref/a.md"})),
        ("skill", json!({"name": "forms:fill", "file": ""})),
    ];
    let mut tools_seen = std::collections::BTreeSet::new();
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test local_mcp_test --test local_skill_test --test local_tools_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
```

Expected: two test targets do not compile: `` error[E0609]: no field `mcp_fingerprint` on type `Approval` ``, `` no method named `create_mcp_approval` found for struct `Store` ``, `` unresolved import `dots_core::engine::local::tools::skill` ``, `` unresolved import `dots_core::engine::local::mcp_tools` ``, then ``could not compile `dots-core` (test "local_mcp_test") due to 3 previous errors`` and `(test "local_skill_test") due to 1 previous error`. `local_tools_test` compiles but is not run while the others fail to build.

- [ ] **Step 3: Implement**

The core first, then the app's literals, then `types.ts`, which is checked against the contract fixture that the next step regenerates.

**In `crates/dots-core/src/model.rs`, replace:**

```rust
    /// When the call ended `applied` or `failed`.
    pub applied_at: Option<String>,
}
```

**with:**

```rust
    /// When the call ended `applied` or `failed`.
    pub applied_at: Option<String>,
    /// For an MCP tool (`mcp__<server>__<tool>`): the server's accepted fingerprint when the
    /// call asked, `None` when it had no accepted list. A decided call goes through later only
    /// while the server's accepted fingerprint is still this one (MCP/skills spec section 4).
    pub mcp_fingerprint: Option<String>,
}
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust

const APPROVAL_COLUMNS: &str = "id, run_id, tool, input, input_hash, status, note, parked, \
    resolved, created_at, decided_at, self_apply, apply_state, applied_at";

fn approval_from_row(row: &SqliteRow) -> Result<Approval> {
```

**with:**

```rust

const APPROVAL_COLUMNS: &str = "id, run_id, tool, input, input_hash, status, note, parked, \
    resolved, created_at, decided_at, self_apply, apply_state, applied_at, mcp_fingerprint";

fn approval_from_row(row: &SqliteRow) -> Result<Approval> {
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
            .transpose()?,
        applied_at: row.try_get("applied_at")?,
    })
}
```

**with:**

```rust
            .transpose()?,
        applied_at: row.try_get("applied_at")?,
        mcp_fingerprint: row.try_get("mcp_fingerprint")?,
    })
}
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
        self_apply: bool,
    ) -> Result<Approval> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO approvals (id, run_id, tool, input, input_hash, status, created_at, \
             self_apply) VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&id)
```

**with:**

```rust
        self_apply: bool,
    ) -> Result<Approval> {
        self.create_mcp_approval(run_id, tool, input, self_apply, None)
            .await
    }

    /// Like `create_approval_with`, for a call of an MCP tool: `mcp_fingerprint` is the
    /// server's accepted fingerprint as the call asks (`None` when it has no accepted list).
    pub async fn create_mcp_approval(
        &self,
        run_id: &str,
        tool: &str,
        input: &Value,
        self_apply: bool,
        mcp_fingerprint: Option<&str>,
    ) -> Result<Approval> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO approvals (id, run_id, tool, input, input_hash, status, created_at, \
             self_apply, mcp_fingerprint) VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
        )
        .bind(&id)
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
        .bind(now())
        .bind(i64::from(self_apply))
        .execute(&self.pool)
        .await?;
```

**with:**

```rust
        .bind(now())
        .bind(i64::from(self_apply))
        .bind(mcp_fingerprint)
        .execute(&self.pool)
        .await?;
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::run_tools::RunTools;
use crate::Result;
```

**with:**

```rust
use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::policy::McpTrust;
use crate::run_tools::RunTools;
use crate::Result;
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
    async fn check(&self, run_id: &str, tool: &str, input: Value) -> Result<PermissionOutcome>;

    /// Claims an approved call that waits for Dots to perform it. False means the call is not
    /// the caller's to perform, and it must not be run. A gate without approvals claims nothing.
```

**with:**

```rust
    async fn check(&self, run_id: &str, tool: &str, input: Value) -> Result<PermissionOutcome>;

    /// Checks a call of the MCP tool `tool` (`mcp__<server>__<tool>`) with what the run knows of
    /// its server (`Policy::resolve_mcp`). An approval it asks for stores `fingerprint`, the
    /// server's accepted one. A gate that does not check MCP calls denies them.
    async fn check_mcp(
        &self,
        _run_id: &str,
        tool: &str,
        _input: Value,
        _trust: McpTrust,
        _fingerprint: Option<String>,
    ) -> Result<PermissionOutcome> {
        Ok(PermissionOutcome::Deny {
            message: format!("{tool} cannot be checked, so it is not permitted."),
        })
    }

    /// Claims an approved call that waits for Dots to perform it. False means the call is not
    /// the caller's to perform, and it must not be run. A gate without approvals claims nothing.
```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
use crate::engine::{PermissionGate, PermissionOutcome};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{ApplyState, Approval, ApprovalStatus, EngineKind};
use crate::policy::{Action, Rule};
use crate::store::Store;
use crate::util::json_hash;
```

**with:**

```rust
use crate::engine::{PermissionGate, PermissionOutcome};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{ApplyState, Approval, ApprovalStatus, Dot, EngineKind};
use crate::policy::{Action, McpTrust, Rule};
use crate::store::Store;
use crate::util::json_hash;
```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
        wait: Duration,
        self_apply: bool,
    ) -> Result<PermissionOutcome> {
        let approval = self
            .store
            .create_approval_with(run_id, tool, &input, self_apply)
            .await?;
        let (tx, mut rx) = oneshot::channel();
```

**with:**

```rust
        wait: Duration,
        self_apply: bool,
        mcp_fingerprint: Option<&str>,
    ) -> Result<PermissionOutcome> {
        let approval = self
            .store
            .create_mcp_approval(run_id, tool, &input, self_apply, mcp_fingerprint)
            .await?;
        let (tx, mut rx) = oneshot::channel();
```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
        })
    }
}

```

**with:**

```rust
        })
    }

    /// Carries out what the policy resolved for a call of `dot`'s run.
    async fn act(
        &self,
        action: Action,
        run_id: &str,
        dot: &Dot,
        tool: &str,
        input: Value,
        mcp_fingerprint: Option<&str>,
    ) -> Result<PermissionOutcome> {
        match action {
            Action::Allow => Ok(PermissionOutcome::Allow { input }),
            Action::Deny => Ok(PermissionOutcome::Deny {
                message: format!(
                    "Denied by the policy of dot '{}': {tool} is not permitted.",
                    dot.spec.name
                ),
            }),
            Action::Ask => {
                let wait = Duration::from_secs(dot.spec.approval_wait_secs);
                let self_apply = self.self_apply.lock().unwrap().contains(&dot.spec.engine);
                self.ask(run_id, tool, input, wait, self_apply, mcp_fingerprint)
                    .await
            }
        }
    }
}

```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
        }
        let workspace = run.workspace_path.as_deref().map(Path::new);
        match dot.spec.policy.resolve_in(tool, &input, workspace) {
            Action::Allow => Ok(PermissionOutcome::Allow { input }),
            Action::Deny => Ok(PermissionOutcome::Deny {
                message: format!(
                    "Denied by the policy of dot '{}': {tool} is not permitted.",
                    dot.spec.name
                ),
            }),
            Action::Ask => {
                let wait = Duration::from_secs(dot.spec.approval_wait_secs);
                let self_apply = self.self_apply.lock().unwrap().contains(&dot.spec.engine);
                self.ask(run_id, tool, input, wait, self_apply).await
            }
        }
    }

```

**with:**

```rust
        }
        let workspace = run.workspace_path.as_deref().map(Path::new);
        let action = dot.spec.policy.resolve_in(tool, &input, workspace);
        self.act(action, run_id, &dot, tool, input, None).await
    }

    /// No one-shot grant is taken here: the local engine never gets one, and a grant for an
    /// MCP call must also match the server's fingerprint (the Claude engine's gateway).
    async fn check_mcp(
        &self,
        run_id: &str,
        tool: &str,
        input: Value,
        trust: McpTrust,
        fingerprint: Option<String>,
    ) -> Result<PermissionOutcome> {
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        let action = dot.spec.policy.resolve_mcp(tool, &input, trust);
        self.act(action, run_id, &dot, tool, input, fingerprint.as_deref())
            .await
    }

```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
    accepted_fingerprint: Option<String>,
    live: Arc<Mutex<Live>>,
}

```

**with:**

```rust
    accepted_fingerprint: Option<String>,
    live: Arc<Mutex<Live>>,
    /// The count of list changes the watcher has listed and checked (`McpSession::list_changed`
    /// counts the ones announced).
    handled: watch::Receiver<u64>,
}

```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
            read_only: read_only.unwrap_or(false),
        }
    }
}
```

**with:**

```rust
            read_only: read_only.unwrap_or(false),
        }
    }

    /// Waits until every list change the server announced so far has been listed and checked,
    /// so `tools` and `trust` are current: a change announced with a call's result holds the
    /// server before the next call is checked. Waits at most `mcp::ANSWER_TIMEOUT`, and not at
    /// all once the run's tools are closed.
    pub async fn settled(&self) {
        let announced = *self.session.list_changed().borrow();
        let mut handled = self.handled.clone();
        let _ = tokio::time::timeout(
            crate::mcp::ANSWER_TIMEOUT,
            handled.wait_for(|n| *n >= announced),
        )
        .await;
    }
}
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                        notices.push(notice);
                    }
                    let server = RunServer {
                        accepted: accepted_tools(&catalog),
```

**with:**

```rust
                        notices.push(notice);
                    }
                    let (handled_tx, handled) = watch::channel(0);
                    let server = RunServer {
                        accepted: accepted_tools(&catalog),
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                        item_id: item.id,
                        name: item.name,
                    };
                    tools.watch(&server, changes, store, bus);
                    tools.servers.push(server);
                }
```

**with:**

```rust
                        item_id: item.id,
                        name: item.name,
                        handled,
                    };
                    tools.watch(&server, changes, handled_tx, store, bus);
                    tools.servers.push(server);
                }
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
        server: &RunServer,
        mut changes: watch::Receiver<u64>,
        store: &Store,
        bus: &Bus,
```

**with:**

```rust
        server: &RunServer,
        mut changes: watch::Receiver<u64>,
        handled: watch::Sender<u64>,
        store: &Store,
        bus: &Bus,
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                    changed = changes.changed() => if changed.is_err() { return },
                }
                let listed = match session.list_tools().await {
                    Ok(listed) => listed,
```

**with:**

```rust
                    changed = changes.changed() => if changed.is_err() { return },
                }
                let announced = *changes.borrow_and_update();
                let listed = match session.list_tools().await {
                    Ok(listed) => listed,
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                            return;
                        }
                        continue;
                    }
```

**with:**

```rust
                            return;
                        }
                        handled.send_replace(announced);
                        continue;
                    }
```

**In `crates/dots-core/src/run_tools.rs`, replace:**

```rust
                    }
                };
                let mut live = live.lock().unwrap();
                live.tools = listed;
                live.held |= !accepted;
            }
        });
```

**with:**

```rust
                    }
                };
                {
                    let mut live = live.lock().unwrap();
                    live.tools = listed;
                    live.held |= !accepted;
                }
                handled.send_replace(announced);
            }
        });
```

**Create `crates/dots-core/src/engine/local/tools/skill.rs`:**

```rust
//! The `skill` tool (policy name `Skill`) and the system prompt's list of skills (MCP/skills
//! spec section 7). A skill is read from the folder the Library names, never from the
//! workspace: without `file` the tool returns the `SKILL.md` body, `${CLAUDE_SKILL_DIR}`
//! replaced by the skill's folder, and the list of the skill's files; with `file` it returns
//! that file, which must lie inside the skill's folder. A skill's `allowed-tools` is ignored:
//! its scripts run only through `shell`, under the dot's policy.

use std::path::{Component, Path};

use serde_json::{json, Value};

use super::{cap_head, str_arg, tool, ToolOutput, MAX_ENTRIES, MAX_READ_BYTES};
use crate::kit::parse_frontmatter;
use crate::run_tools::RunSkill;

/// The first line of the system prompt's skills section.
const PROMPT_HEAD: &str =
    "Skills you can use. Call the tool `skill` with a name to read one before you follow it.";

/// The function definition, offered to a run that has skills.
pub fn schema() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    tool(
        "skill",
        "Read a skill: its instructions and the list of its files, or one of its files.",
        json!({
            "name": s("The skill's name, as the system prompt lists it"),
            "file": s("A file of the skill, relative to its folder; default: its instructions")
        }),
        &["name"],
    )
}

/// The system prompt's section that lists `skills`, one `name: description` line each; empty
/// without skills.
pub fn prompt_section(skills: &[RunSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = PROMPT_HEAD.to_string();
    for skill in skills {
        out.push_str(&format!("\n- {}: {}", skill.name, skill.info.listing()));
    }
    out
}

/// Runs an allowed `skill` call (its input as `prepare` left it) against the run's skills.
pub async fn run(skills: &[RunSkill], input: &Value) -> ToolOutput {
    let name = str_arg(input, "name");
    let Some(skill) = skills.iter().find(|s| s.name == name) else {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        return ToolOutput::err(if names.is_empty() {
            format!("unknown skill '{name}'. This run has no skills.")
        } else {
            format!("unknown skill '{name}'. Skills: {}", names.join(", "))
        });
    };
    let dir = skill.info.dir.clone();
    // An empty `file` asks for the instructions, as no `file` does.
    let file = input
        .get("file")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string);
    let read = tokio::task::spawn_blocking(move || match file {
        Some(file) => read_file(&dir, &file),
        None => instructions(&dir),
    })
    .await;
    match read {
        Ok(Ok(text)) => ToolOutput::ok(cap_head(text)),
        Ok(Err(e)) => ToolOutput::err(e),
        Err(e) => ToolOutput::err(format!("reading the skill failed: {e}")),
    }
}

/// The `SKILL.md` body with the folder put in, then the skill's other files.
fn instructions(dir: &Path) -> Result<String, String> {
    let text = read_text(&dir.join("SKILL.md"), "SKILL.md")?;
    let body = parse_frontmatter(&text)
        .body
        .replace("${CLAUDE_SKILL_DIR}", &dir.to_string_lossy());
    let mut out = body.trim().to_string();
    let (files, more) = list_files(dir);
    if !files.is_empty() {
        out.push_str("\n\nFiles in this skill (read one with `file`):");
        for file in files {
            out.push_str(&format!("\n- {file}"));
        }
        if more {
            out.push_str(&format!("\n(only the first {MAX_ENTRIES} are listed)"));
        }
    }
    Ok(out)
}

/// The skill's files below `dir`, relative with `/`, sorted, without `SKILL.md`, hidden
/// entries (`.git`) and links; true when there were more than `MAX_ENTRIES`.
fn list_files(dir: &Path) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut folders = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                folders.push((entry.path(), format!("{rel}/")));
            } else if rel != "SKILL.md" {
                files.push(rel);
            }
        }
    }
    files.sort();
    let more = files.len() > MAX_ENTRIES;
    files.truncate(MAX_ENTRIES);
    (files, more)
}

/// The file `file` of the skill in `dir`. It must name a file inside the folder: a path that
/// is absolute, holds `..`, or leads out of the folder through a link is refused.
fn read_file(dir: &Path, file: &str) -> Result<String, String> {
    let outside = || format!("{file} is not inside the skill folder");
    let given = Path::new(file);
    let plain = given.components().all(|c| match c {
        Component::Normal(n) => !n.to_string_lossy().contains(':'),
        Component::CurDir => true,
        _ => false,
    });
    if !plain {
        return Err(outside());
    }
    let root =
        std::fs::canonicalize(dir).map_err(|e| format!("the skill folder cannot be read: {e}"))?;
    let path = std::fs::canonicalize(root.join(given))
        .map_err(|_| format!("{file} was not found in the skill folder"))?;
    if !path.starts_with(&root) {
        return Err(outside());
    }
    if path.is_dir() {
        return Err(format!("{file} is a folder; name a file in it"));
    }
    read_text(&path, file)
}

/// A text file, refused above `MAX_READ_BYTES`; bytes that are not UTF-8 are replaced.
fn read_text(path: &Path, shown: &str) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{shown} could not be read: {e}"))?
        .len();
    if size > MAX_READ_BYTES {
        return Err(format!("{shown} is larger than 10 MB"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{shown} could not be read: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
mod search;
mod shell;

/// Tool output sent back to the model is capped at this many bytes.
```

**with:**

```rust
mod search;
mod shell;
pub mod skill;

/// Tool output sent back to the model is capped at this many bytes.
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust

/// (local tool name, policy alias). The policy, grants and approvals only see the alias.
pub const TOOLS: &[(&str, &str)] = &[
    ("read_file", "Read"),
```

**with:**

```rust

/// (local tool name, policy alias). The policy, grants and approvals only see the alias.
/// `skill` reads the run's skills, not the workspace: the engine runs it (`skill::run`) and
/// offers it only to a run that has skills.
pub const TOOLS: &[(&str, &str)] = &[
    ("read_file", "Read"),
```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    ("grep", "Grep"),
    ("shell", "Bash"),
];

```

**with:**

```rust
    ("grep", "Grep"),
    ("shell", "Bash"),
    ("skill", "Skill"),
];

```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
        &[("command", Str, true), ("timeout", Count, false)],
    ),
];

```

**with:**

```rust
        &[("command", Str, true), ("timeout", Count, false)],
    ),
    ("skill", &[("name", Str, true), ("file", Str, false)]),
];

```

**In `crates/dots-core/src/engine/local/tools/mod.rs`, replace:**

```rust
    output. Default timeout 120 seconds, maximum 600.";

/// OpenAI `tools` array for every built-in tool.
pub fn schemas() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
```

**with:**

```rust
    output. Default timeout 120 seconds, maximum 600.";

/// OpenAI `tools` array for every built-in tool but `skill` (`skill::schema`).
pub fn schemas() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
```

**Create `crates/dots-core/src/engine/local/mcp_tools.rs`:**

```rust
//! The local engine's MCP tools (MCP/skills spec section 5). The tools of the run's open
//! sessions are offered as functions named `mcp__<server>__<tool>`, a name too long for a
//! function (or holding a character one cannot have) shortened by `model_name` and mapped back
//! for the run. A call is checked by `PermissionGate::check_mcp` with what the run knows of
//! the server (`RunServer::trust`), sent to the server, and its result reported as text.
//!
//! An approved call that the resume run performs goes through only while the server's accepted
//! fingerprint is the one stored with the approval (spec section 4).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::client::ToolCallRequest;
use super::tools::{self, cap_head, ToolOutput};
use super::Session;
use crate::engine::{EngineEvent, PermissionOutcome};
use crate::mcp::CALL_TIMEOUT;
use crate::model::Approval;
use crate::run_tools::RunServer;

/// The longest function name an OpenAI-compatible endpoint takes.
pub const MAX_NAME: usize = 64;
/// How much of a name that is too long is kept, before `_` and 8 hex characters.
const KEPT: usize = 55;

/// What the model is told about an approved call whose server changed its accepted list.
pub(super) const CHANGED: &str = "the tool changed since it was approved";

/// The function name of the MCP tool `qualified` (`mcp__<server>__<tool>`): the name itself
/// when it fits `^[A-Za-z0-9_-]{1,64}$`; otherwise its first 55 characters (each other
/// character as `_`), `_` and the first 8 hex characters of the SHA-256 of the whole name.
pub fn model_name(qualified: &str) -> String {
    let fits = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if qualified.len() <= MAX_NAME && qualified.chars().all(fits) {
        return qualified.to_string();
    }
    let kept: String = qualified
        .chars()
        .map(|c| if fits(c) { c } else { '_' })
        .take(KEPT)
        .collect();
    let hash: String = Sha256::digest(qualified.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{kept}_{hash}")
}

/// A `tools/call` result as the model gets it: the `text` parts joined by new lines, any other
/// part as `[<type> omitted]`, capped at 30 KB. With no part, the `structuredContent` stands
/// in. `isError` makes it an error result.
pub fn format_result(result: &Value) -> ToolOutput {
    let parts: Vec<String> = result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .map(|part| match part["type"].as_str() {
                    Some("text") => part["text"].as_str().unwrap_or_default().to_string(),
                    Some(kind) => format!("[{kind} omitted]"),
                    None => "[content omitted]".to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let text = match result.get("structuredContent") {
        _ if !parts.is_empty() => parts.join("\n"),
        Some(structured) if !structured.is_null() => structured.to_string(),
        _ => "(no output)".to_string(),
    };
    if result["isError"] == true {
        ToolOutput::err(cap_head(text))
    } else {
        ToolOutput::ok(cap_head(text))
    }
}

/// The run's map from function names back to MCP tools. It only grows: a name the model saw
/// on an earlier turn still finds its tool.
#[derive(Default)]
pub(super) struct ToolNames(HashMap<String, String>);

impl ToolNames {
    /// The MCP tool a model's call names: a mapped name, or a full `mcp__` name as it is.
    fn qualified(&self, name: &str) -> Option<String> {
        self.0
            .get(name)
            .cloned()
            .or_else(|| name.starts_with("mcp__").then(|| name.to_string()))
    }
}

/// The arguments of an MCP call: a JSON object, sent as one or as a JSON string.
fn arguments(raw: &Value) -> Result<Value, String> {
    match raw {
        Value::Object(_) => Ok(raw.clone()),
        Value::Null => Ok(json!({})),
        Value::String(s) if s.trim().is_empty() => Ok(json!({})),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(object @ Value::Object(_)) => Ok(object),
            Ok(_) => Err("the arguments must be a JSON object".into()),
            Err(e) => Err(format!(
                "the arguments are not valid JSON ({e}); send a JSON object"
            )),
        },
        _ => Err("the arguments must be a JSON object".into()),
    }
}

/// The error for a call of `qualified` when the run has no such server open.
fn not_open(qualified: &str) -> String {
    match qualified
        .strip_prefix("mcp__")
        .and_then(|r| r.split_once("__"))
    {
        Some((server, _)) => {
            format!("{qualified}: no MCP server named '{server}' is open in this run")
        }
        None => format!("unknown tool '{qualified}'"),
    }
}

/// Sends a call the check allowed to its server. A cancelled call reads "cancelled"; the
/// caller sees the run's cancel.
async fn send(
    server: &RunServer,
    tool: &str,
    input: Value,
    cancel: &CancellationToken,
) -> ToolOutput {
    match server
        .session()
        .call_tool(tool, input, CALL_TIMEOUT, cancel)
        .await
    {
        Ok(result) => format_result(&result),
        Err(e) => ToolOutput::err(e.to_string()),
    }
}

impl Session {
    /// The functions offered this turn: the built-in tools, `skill` when the run has skills,
    /// and the tools the run's servers list now. A list change a server announced is listed
    /// first, so a new tool is offered on the next turn.
    pub(super) async fn tool_schemas(&mut self) -> Vec<Value> {
        let mut out = match tools::schemas() {
            Value::Array(builtin) => builtin,
            _ => Vec::new(),
        };
        if !self.ctx.tools.skills().is_empty() {
            out.push(tools::skill::schema());
        }
        let mut given = HashSet::new();
        for server in self.ctx.tools.servers() {
            server.settled().await;
            for tool in server.tools() {
                let qualified = format!("mcp__{}__{}", server.name(), tool.name);
                let name = model_name(&qualified);
                // A tool the server lists twice is offered once.
                if !given.insert(name.clone()) {
                    continue;
                }
                self.mcp_names.0.insert(name.clone(), qualified);
                let description = tool.description.or(tool.title).unwrap_or_default();
                out.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": tool.input_schema
                    }
                }));
            }
        }
        out
    }

    /// The MCP tool a model's call names, if it names one.
    pub(super) fn mcp_tool(&self, name: &str) -> Option<String> {
        self.mcp_names.qualified(name)
    }

    /// Shows, checks and performs a model's call of the MCP tool `qualified`. `None` when the
    /// run must stop (cancelled, or the runner stopped listening).
    pub(super) async fn mcp_output(
        &self,
        call: &ToolCallRequest,
        qualified: &str,
    ) -> Option<ToolOutput> {
        let parsed = arguments(&call.arguments);
        let shown = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool: qualified.to_string(),
            input: parsed.clone().unwrap_or_else(|_| call.arguments.clone()),
        };
        if !self.emit(shown).await {
            return None;
        }
        let input = match parsed {
            Ok(input) => input,
            Err(e) => return Some(ToolOutput::err(format!("{qualified}: {e}"))),
        };
        let Some((server, tool)) = self.ctx.tools.resolve(qualified) else {
            return Some(ToolOutput::err(not_open(qualified)));
        };
        // A list change announced with an earlier result holds the server before this check.
        server.settled().await;
        let checked = tokio::select! {
            r = self.gate.check_mcp(
                &self.ctx.run_id,
                qualified,
                input,
                server.trust(tool),
                server.accepted_fingerprint(),
            ) => r,
            _ = self.ctx.cancel.cancelled() => return None,
        };
        Some(match checked {
            Ok(PermissionOutcome::Allow { input }) => {
                send(server, tool, input, &self.ctx.cancel).await
            }
            Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
            Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
        })
    }

    /// Whether `approval` is an MCP call whose server's accepted list is no longer the one it
    /// was approved under: it must not be performed.
    pub(super) fn changed_since(&self, approval: &Approval) -> bool {
        self.ctx
            .tools
            .resolve(&approval.tool)
            .is_some_and(|(server, _)| server.accepted_fingerprint() != approval.mcp_fingerprint)
    }

    /// Performs an approved call of the MCP tool `qualified` exactly as it was approved.
    pub(super) async fn perform_mcp(&self, qualified: &str, input: &Value) -> ToolOutput {
        match self.ctx.tools.resolve(qualified) {
            Some((server, tool)) => send(server, tool, input.clone(), &self.ctx.cancel).await,
            None => ToolOutput::err(not_open(qualified)),
        }
    }
}
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust

use self::client::{ChatClient, ToolCallRequest, DEFAULT_RETRY_DELAYS};
use self::tools::{PathFilter, PreparedCall, ToolOutput};

pub mod client;
pub mod history;
pub mod tools;

```

**with:**

```rust

use self::client::{ChatClient, ToolCallRequest, DEFAULT_RETRY_DELAYS};
use self::mcp_tools::{ToolNames, CHANGED};
use self::tools::{PathFilter, PreparedCall, ToolOutput};

pub mod client;
pub mod history;
pub mod mcp_tools;
pub mod tools;

```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
            tx,
            messages,
        };
        tokio::spawn(session.run());
```

**with:**

```rust
            tx,
            messages,
            mcp_names: ToolNames::default(),
        };
        tokio::spawn(session.run());
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
    tx: mpsc::Sender<EngineEvent>,
    messages: Vec<Value>,
}

```

**with:**

```rust
    tx: mpsc::Sender<EngineEvent>,
    messages: Vec<Value>,
    mcp_names: ToolNames,
}

```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
            return;
        }
        if self.messages.is_empty()
            && !self
                .push(json!({"role": "system", "content": SYSTEM_PROMPT}))
                .await
        {
```

**with:**

```rust
            return;
        }
        let skills = tools::skill::prompt_section(self.ctx.tools.skills());
        let system = match skills.as_str() {
            "" => SYSTEM_PROMPT.to_string(),
            skills => format!("{SYSTEM_PROMPT}\n\n{skills}"),
        };
        if self.messages.is_empty()
            && !self
                .push(json!({"role": "system", "content": system}))
                .await
        {
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
            return;
        }
        let tools = tools::schemas();
        let max_turns = self.ctx.dot.spec.max_turns;
        for turn in 1..=max_turns {
            let body = json!({
                "model": self.ctx.dot.spec.model,
```

**with:**

```rust
            return;
        }
        let max_turns = self.ctx.dot.spec.max_turns;
        for turn in 1..=max_turns {
            let tools = self.tool_schemas().await;
            let body = json!({
                "model": self.ctx.dot.spec.model,
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
                continue;
            }
            match self.gate.claim_apply(&approval.id).await {
                Ok(true) => {}
```

**with:**

```rust
                continue;
            }
            if self.changed_since(approval) {
                match self.gate.skip_apply(&approval.id).await {
                    Ok(true) => lines.push(format!("- {what}: not performed, because {CHANGED}.")),
                    Ok(false) => continue,
                    Err(e) => tracing::warn!(approval = %approval.id, "skipping failed: {e}"),
                }
                stopped = true;
                continue;
            }
            match self.gate.claim_apply(&approval.id).await {
                Ok(true) => {}
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
                return None;
            }
            let prepared = tools::local_name(&approval.tool)
                .ok_or_else(|| format!("unknown tool '{}'", approval.tool))
                .and_then(|name| tools::prepare(name, &approval.input, &self.ctx.workspace));
            let output = match prepared {
                // Nothing ran. The refusal is the call's outcome.
                Err(e) => ToolOutput::err(e),
                Ok(call) => {
                    let filter = PathFilter::from_policy(
                        &self.ctx.dot.spec.policy,
                        call.alias,
                        &self.ctx.workspace,
                    );
                    tools::run_with(&call, &self.ctx.workspace, &self.ctx.cancel, &filter).await
                }
            };
```

**with:**

```rust
                return None;
            }
            let output = if approval.tool.starts_with("mcp__") {
                self.perform_mcp(&approval.tool, &approval.input).await
            } else {
                let prepared = tools::local_name(&approval.tool)
                    .ok_or_else(|| format!("unknown tool '{}'", approval.tool))
                    .and_then(|name| tools::prepare(name, &approval.input, &self.ctx.workspace));
                match prepared {
                    // Nothing ran. The refusal is the call's outcome.
                    Err(e) => ToolOutput::err(e),
                    Ok(call) => self.run_allowed(&call).await,
                }
            };
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
    }

    /// Checks, gates and runs one tool call, then records its result.
    async fn handle(&mut self, call: &ToolCallRequest) -> bool {
        if self.ctx.cancel.is_cancelled() {
            return false;
        }
        let prepared = tools::prepare(&call.name, &call.arguments, &self.ctx.workspace);
        let (tool, input) = match &prepared {
```

**with:**

```rust
    }

    /// Runs a built-in call the gate allowed: `skill` reads the run's skills; every other tool
    /// works in the workspace, the walking tools leaving out what the policy denies.
    async fn run_allowed(&self, call: &PreparedCall) -> ToolOutput {
        if call.name == "skill" {
            return tools::skill::run(self.ctx.tools.skills(), &call.input).await;
        }
        let filter =
            PathFilter::from_policy(&self.ctx.dot.spec.policy, call.alias, &self.ctx.workspace);
        tools::run_with(call, &self.ctx.workspace, &self.ctx.cancel, &filter).await
    }

    /// Shows, checks and runs a model's call of a built-in tool. `None` when the run must stop.
    async fn builtin_output(&self, call: &ToolCallRequest) -> Option<ToolOutput> {
        let prepared = tools::prepare(&call.name, &call.arguments, &self.ctx.workspace);
        let (tool, input) = match &prepared {
```

**In `crates/dots-core/src/engine/local/mod.rs`, replace:**

```rust
        };
        if !self.emit(shown).await {
            return false;
        }
        let output = match prepared {
            Err(e) => ToolOutput::err(e),
            Ok(p) => {
                let checked = tokio::select! {
                    r = self.gate.check(&self.ctx.run_id, p.alias, p.input.clone()) => r,
                    _ = self.ctx.cancel.cancelled() => return false,
                };
                match checked {
                    Ok(PermissionOutcome::Allow { input }) => {
                        let filter = PathFilter::from_policy(
                            &self.ctx.dot.spec.policy,
                            p.alias,
                            &self.ctx.workspace,
                        );
                        let allowed = PreparedCall { input, ..p };
                        tools::run_with(&allowed, &self.ctx.workspace, &self.ctx.cancel, &filter)
                            .await
                    }
                    Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
                    Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
                }
            }
        };
        // A cancel that stopped the tool leaves the call unanswered; `history::repair` fills it.
```

**with:**

```rust
        };
        if !self.emit(shown).await {
            return None;
        }
        let p = match prepared {
            Err(e) => return Some(ToolOutput::err(e)),
            Ok(p) => p,
        };
        let checked = tokio::select! {
            r = self.gate.check(&self.ctx.run_id, p.alias, p.input.clone()) => r,
            _ = self.ctx.cancel.cancelled() => return None,
        };
        Some(match checked {
            Ok(PermissionOutcome::Allow { input }) => {
                self.run_allowed(&PreparedCall { input, ..p }).await
            }
            Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
            Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
        })
    }

    /// Checks, gates and runs one tool call, then records its result.
    async fn handle(&mut self, call: &ToolCallRequest) -> bool {
        if self.ctx.cancel.is_cancelled() {
            return false;
        }
        let output = match self.mcp_tool(&call.name) {
            Some(qualified) => self.mcp_output(call, &qualified).await,
            None => self.builtin_output(call).await,
        };
        let Some(output) = output else {
            return false;
        };
        // A cancel that stopped the tool leaves the call unanswered; `history::repair` fills it.
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        apply_state: None,
        applied_at: None,
    }
}
```

**with:**

```rust
        apply_state: None,
        applied_at: None,
        mcp_fingerprint: None,
    }
}
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust
                apply_state: None,
                applied_at: None,
            },
        }
```

**with:**

```rust
                apply_state: None,
                applied_at: None,
                mcp_fingerprint: None,
            },
        }
```

**In `ui/src/api/types.ts`, replace:**

```ts
  apply_state: nullable(ApplyState),
  applied_at: nullable(z.string()),
};
export const Approval = z.strictObject(approvalShape);
```

**with:**

```ts
  apply_state: nullable(ApplyState),
  applied_at: nullable(z.string()),
  // For an MCP call: the server's accepted fingerprint when it asked.
  mcp_fingerprint: nullable(z.string()),
};
export const Approval = z.strictObject(approvalShape);
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown

Watchers (polling GitHub/SF/folders), connecting remote machines, multi-user, cloud hosting,
MCP servers for local-engine dots, a CLI client, a separate daemon process.

### Constraint: subscription use
```

**with:**

```markdown

Watchers (polling GitHub/SF/folders), connecting remote machines, multi-user, cloud hosting,
a CLI client, a separate daemon process.

### Constraint: subscription use
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    src/engine/mod.rs        Engine trait + EngineEvent
    src/engine/claude.rs     drives `claude -p` with stream-json I/O
    src/engine/local.rs      own tool-use loop over OpenAI-compatible chat completions
    src/tools/               built-in tools for the local engine
    src/policy/              allow/ask/deny resolution, one-shot grants
```

**with:**

```markdown
    src/engine/mod.rs        Engine trait + EngineEvent
    src/engine/claude.rs     drives `claude -p` with stream-json I/O
    src/engine/local.rs      own tool-use loop over OpenAI-compatible chat completions, with
                             the run's MCP tools and the `skill` tool (MCP/skills spec §5, §7)
    src/tools/               built-in tools for the local engine
    src/policy/              allow/ask/deny resolution, one-shot grants
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
local engine) the call is queued and the child run performs it before anything else; on every
other engine a one-shot grant is written and the child run's prompt asks the model to issue
the call again. See `2026-10-13-approvals-v2-design.md` §3.

### Steps
```

**with:**

```markdown
local engine) the call is queued and the child run performs it before anything else; on every
other engine a one-shot grant is written and the child run's prompt asks the model to issue
the call again. See `2026-10-13-approvals-v2-design.md` §3. From MCP/skills an approved call of
an MCP tool is performed only while the server's accepted fingerprint is the one stored with the
approval (`approvals.mcp_fingerprint`); otherwise it is skipped with "the tool changed since it
was approved" (MCP/skills spec §4).

### Steps
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  `allow` rules → `ask` rules → `default`. From Approvals v2 a grant is written only for a
  dot whose engine does not perform approved calls itself, which is the Claude engine.
- From Approvals v2 an approval can save an allow rule to the dot's policy, in one transaction
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
```

**with:**

```markdown
  `allow` rules → `ask` rules → `default`. From Approvals v2 a grant is written only for a
  dot whose engine does not perform approved calls itself, which is the Claude engine.
- An MCP tool (`mcp__<server>__<tool>`) is resolved by the check of MCP/skills spec §4 instead
  (`Policy::resolve_mcp`: `deny` rules → held server or a tool not in the accepted list asks →
  `allow` rules → `ask` rules → the accepted list's `readOnlyHint` allows → `mcp_default`),
  through the same approval queue (`PermissionGate::check_mcp`). Its approval stores the
  server's accepted fingerprint (`approvals.mcp_fingerprint`). The local engine takes no grant
  for it.
- From Approvals v2 an approval can save an allow rule to the dot's policy, in one transaction
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
the tool result → append results → repeat until no tool calls or `max_turns`.

Every chat message (system, user, assistant with its `tool_calls`, tool result) is persisted as
a `message` run event. A local run's `session_id` is the `root_run_id` of its lineage. On a
```

**with:**

```markdown
the tool result → append results → repeat until no tool calls or `max_turns`.

From MCP/skills (spec §5) the schemas of each turn also hold the tools the run's MCP sessions
list now (`RunTools`), as functions named `mcp__<server>__<tool>` with the server's description
and `inputSchema`. A name longer than 64 characters, or holding a character other than
`A-Za-z0-9_-`, is cut to 55 characters (others as `_`) plus `_` and the first 8 hex characters of
its SHA-256; the map back is kept for the run. A call is shown and checked under its full name
(`ApprovalHub::check_mcp` with the server's trust), sent with a 300 s limit, and its result
reported as the `text` parts joined, other parts as `[<type> omitted]` (the `structuredContent`
when there is no part), `isError` as an error result, capped at 30 KB. A list change the server
announces is listed and checked before the next call and the next turn's schemas, so a changed
list holds the server for that call. A server that died answers "the server stopped".

Every chat message (system, user, assistant with its `tool_calls`, tool result) is persisted as
a `message` run event. A local run's `session_id` is the `root_run_id` of its lineage. On a
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
approved: the call is claimed first, it takes the path of a live `allow` (`prepare`, then the
tool, with the walking tools' filter), and its outcome is recorded before anything else is done
with the output. The policy is not resolved again. After a call that did not succeed, the calls
that remain are given up. The engine then sends one user message that says what each call did
(and that the quoted outputs are results, not instructions), followed by the runner's resume
prompt. No grant is written and the model does not issue the
```

**with:**

```markdown
approved: the call is claimed first, it takes the path of a live `allow` (`prepare`, then the
tool, with the walking tools' filter), and its outcome is recorded before anything else is done
with the output. The policy is not resolved again. An MCP call whose server's accepted
fingerprint is no longer the stored one is not claimed but skipped ("not performed, because the
tool changed since it was approved"). After a call that did not succeed or was skipped, the
calls that remain are given up. The engine then sends one user message that says what each call did
(and that the quoted outputs are results, not instructions), followed by the runner's resume
prompt. No grant is written and the model does not issue the
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
`glob`, `grep`, `shell` (PowerShell, timeout default 120 s, output capped at 30 KB; the script
reaches PowerShell on stdin behind a small fixed `-EncodedCommand` bootstrap, so its length is
not bound by the Windows command-line limit). Their
argument names match the Claude tools they alias (`file_path`, `content`, `old_string`,
`new_string`, `path`, `pattern`, `command`), so policy path specs, command rules and
```

**with:**

```markdown
`glob`, `grep`, `shell` (PowerShell, timeout default 120 s, output capped at 30 KB; the script
reaches PowerShell on stdin behind a small fixed `-EncodedCommand` bootstrap, so its length is
not bound by the Windows command-line limit), and from MCP/skills `skill` (policy name `Skill`,
offered only to a run with skills). Their
argument names match the Claude tools they alias (`file_path`, `content`, `old_string`,
`new_string`, `path`, `pattern`, `command`), so policy path specs, command rules and
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
cwd = workspace. "Test endpoint" in Settings sends a probe request with one tool and checks that
the model returns a well-formed tool call.

Tests drive the loop against a `wiremock` OpenAI-compatible server returning scripted
```

**with:**

```markdown
cwd = workspace. "Test endpoint" in Settings sends a probe request with one tool and checks that
the model returns a well-formed tool call.

`skill` (MCP/skills spec §7) takes `name` and an optional `file`. Without `file` it returns the
skill's `SKILL.md` body, with `${CLAUDE_SKILL_DIR}` replaced by the skill's folder, followed by
the list of the skill's files (hidden entries and links left out); with `file` it returns that
file, refused when the path is absolute, holds `..` or resolves outside the skill folder. Results
are capped at 30 KB. The system message of a run with skills ends with "Skills you can use. Call
the tool `skill` with a name to read one before you follow it." and one `name: description` line
per skill (standalone skills by their Library name, a plugin's as `<plugin>:<skill>`).

Tests drive the loop against a `wiremock` OpenAI-compatible server returning scripted
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. Every approval in the fixture (the `Approval` sample, the Inbox's approval, and the approvals of the `approval_requested`, `approval_decided` and `approval_updated` events) gains `"mcp_fingerprint": null`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-core --test local_mcp_test --test local_skill_test --test local_tools_test 2>&1 | grep -E "^test result|FAILED"
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `local_mcp_test` 10 passed, `local_skill_test` 7 (on Windows; `a_link_out_of_the_skill_folder_is_refused` is for Unix), `local_tools_test` 38; `dots-app` 49 passed; the UI files under `src/api` pass.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 572 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent, UI 293 passed in 21 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/contract.rs apps/dots-app/src/events.rs crates/dots-core/src/approvals.rs crates/dots-core/src/engine/local/mcp_tools.rs crates/dots-core/src/engine/local/mod.rs crates/dots-core/src/engine/local/tools/mod.rs crates/dots-core/src/engine/local/tools/skill.rs crates/dots-core/src/engine/mod.rs crates/dots-core/src/model.rs crates/dots-core/src/run_tools.rs crates/dots-core/src/store/approvals.rs crates/dots-core/tests/local_mcp_test.rs crates/dots-core/tests/local_skill_test.rs crates/dots-core/tests/local_tools_test.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/types.ts
git commit -m "feat(core): give local-engine runs their MCP tools and a skill tool"
```

### Task 7: Reach a Claude run's MCP servers through a checking gateway, and pass its plugin folders

A Claude run now reaches its MCP servers only through Dots (spec section 5). Dots' local server gets the route `POST /mcp/s/{server}`, behind the loopback check and the run's secret: it answers `initialize` and `tools/list` from the run's open session and, for `tools/call`, waits until the server is settled, runs `check_mcp`, waits for an approval when it asks, and forwards the call; a refusal is a tool result with `isError` and the same text the local engine gives. The CLI's `--mcp-config` lists `dots` and one gateway entry per open session, so no secret is written into the file any more; `--allowedTools` gets `mcp__<server>__*` for each, and `--plugin-dir` each folder of Task 5. The runner keeps each run's tools by run id for the gateway. The one-shot grant of a retried call (Approvals v2) now carries the approval's fingerprint and holds only while the accepted list is the same. `plugin_errors` of the CLI's `system/init` become notices. `fake-claude` records the config and the plugin folders, and can list and call a gateway tool. Task 2's interim direct entries and `RunContext.mcp_servers` go. The main spec's sections 2 to 6 and 9 follow.

**Files:**
- Create: `crates/dots-core/src/server/gateway.rs`, `crates/dots-core/tests/claude_gateway_test.rs`
- Modify: `crates/dots-core/migrations/0006_library.sql`, `crates/dots-core/src/approvals.rs`, `crates/dots-core/src/bin/fake-claude.rs`, `crates/dots-core/src/engine/claude/args.rs`, `crates/dots-core/src/engine/claude/mod.rs`, `crates/dots-core/src/engine/claude/parse.rs`, `crates/dots-core/src/engine/local/mcp_tools.rs`, `crates/dots-core/src/engine/mod.rs`, `crates/dots-core/src/runner.rs`, `crates/dots-core/src/server/mcp.rs`, `crates/dots-core/src/server/mod.rs`, `crates/dots-core/src/store/approvals.rs`, `crates/dots-core/src/store/runs.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`
- Test: `crates/dots-core/tests/approvals_test.rs`, `crates/dots-core/tests/claude_cli_test.rs`, `crates/dots-core/tests/claude_gateway_test.rs`, `crates/dots-core/tests/engine_prompt_test.rs`, `crates/dots-core/tests/local_engine_test.rs`, `crates/dots-core/tests/resume_test.rs`, `crates/dots-core/tests/runner_test.rs`, `crates/dots-core/tests/store_runs_test.rs`

**Interfaces:**
- Consumes: Task 5's `RunTools` and `RunServer`, Task 6's `check_mcp`, `settled()` and `create_mcp_approval`; the server's `loopback_only`, `run_for_secret` and `MAX_MCP_BODY`; `MCP_TOOL_TIMEOUT`; Approvals v2's grants and `resume_parked`.
- Produces:
  - Migration `0006_library.sql` gains `ALTER TABLE grants ADD COLUMN mcp_fingerprint TEXT;`.
  - `Store::create_mcp_grant(&self, root_run_id: &str, tool: &str, input_hash: &str, mcp_fingerprint: Option<&str>) -> Result<()>` (`create_grant` delegates with `None`); `Store::take_mcp_grant(&self, root_run_id: &str, tool: &str, input_hash: &str) -> Result<Option<Option<String>>>` (`None`: no grant; `Some(fp)`: used up, approved under `fp`); `Store::resume_parked(&self, parent_id: &str, grants: &[(String, String, Option<String>)], resolved_ids: &[String], queued_ids: &[String], child: Option<&NewRun>) -> Result<Option<Run>>` (tool, grant key, fingerprint).
  - `approvals::TOOL_CHANGED = "the tool changed since it was approved"` (the local engine's `CHANGED` re-exports it). `ApprovalHub::check_mcp` first takes a matching grant: it allows when the grant's fingerprint equals the given one, else denies "<tool>: not performed, because the tool changed since it was approved.".
  - `Runner::register_tools(&self, run_id: &str, tools: Arc<RunTools>)`, `Runner::run_tools(&self, run_id: &str) -> Option<Arc<RunTools>>` (until the run is forgotten), `Runner::run_cancel(&self, run_id: &str) -> Option<CancellationToken>`. `RunContext.mcp_servers` is removed.
  - `server::gateway::handle(State(s): State<ServerState>, Path(server): Path<String>, headers: HeaderMap, body: Bytes) -> Response` on `POST /mcp/s/{server}` (GET is 405): 401 without the run's secret; 404 for a server the run does not have open; `initialize` (serverInfo name = the server), `ping`, `tools/list` (the session's current tools, after `settled()`), `tools/call` (arguments must be an object, else -32602; the call gets what is left of `MCP_TOOL_TIMEOUT` minus 2 s and the run's cancel token); another method is -32601.
  - In `server/mcp.rs`, shared `pub(super)` helpers `rpc_error(id: Value, code: i64, message: &str) -> Response`, `read_request(body: &[u8]) -> Result<(Value, Value), Box<Response>>`, `initialize_result(req: &Value, name: &str) -> Value`; `approve` answers allow for a tool of the run's open sessions.
  - `engine::claude::args::mcp_config(ctx: &RunContext) -> Value` (`dots` plus one gateway entry per open session) and `gateway_entry(mcp_url: &str, secret: &str, server: &str) -> Value` (`{"type": "http", "url": "<mcp_url>/s/<server>", "headers": {"Authorization": "Bearer <secret>"}}`); `build_args` adds `mcp__<server>__*` to `--allowedTools` and `--plugin-dir <dir>` per plugin folder (after `--disallowedTools`, before `--resume`).
  - `parse.rs`: `system/init` gives `SessionStarted` and one `Notice` per `plugin_errors` entry: "plugin <name>: <error>", "plugin error: <error>" or "plugin error: <json>".
  - `fake-claude`: its record (`FAKE_CLAUDE_ARGS_OUT`) gains `mcp_config` and `plugin_dirs: [{path, files}]`; script steps `{"mcp_list": "<server>"}` and `{"mcp_call": {"server", "tool", "arguments"}}`.

- [ ] **Step 1: Write the failing tests**

**Append to `crates/dots-core/tests/approvals_test.rs`:**

```rust

/// A Claude-engine retry of an approved MCP call takes its one-shot grant only under the
/// server's accepted fingerprint it was approved with (MCP/skills spec section 4).
#[tokio::test]
async fn an_mcp_grant_allows_only_under_the_fingerprint_it_was_approved_with() {
    use dots_core::policy::McpTrust;
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    let tool = "mcp__files__write";
    let input = json!({"path": "a.txt"});
    let writes = McpTrust {
        held: false,
        read_only: false,
    };
    let check = |fingerprint: &str| {
        hub.check_mcp(
            &run.id,
            tool,
            input.clone(),
            writes,
            Some(fingerprint.to_string()),
        )
    };
    store
        .create_mcp_grant(
            &run.root_run_id,
            tool,
            &grant_key(tool, &input),
            Some("fp1"),
        )
        .await
        .unwrap();
    assert_eq!(
        check("fp1").await.unwrap(),
        PermissionOutcome::Allow {
            input: input.clone()
        }
    );
    // Used up: the next call asks, and asking parks at once.
    assert!(matches!(
        check("fp1").await.unwrap(),
        PermissionOutcome::Deny { .. }
    ));
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);

    store
        .create_mcp_grant(
            &run.root_run_id,
            tool,
            &grant_key(tool, &input),
            Some("fp1"),
        )
        .await
        .unwrap();
    assert_eq!(
        check("fp2").await.unwrap(),
        PermissionOutcome::Deny {
            message: "mcp__files__write: not performed, because the tool changed since it was \
                      approved."
                .into()
        }
    );
    // That grant is used up too, and no approval was asked for.
    assert!(!store
        .take_grant(&run.root_run_id, tool, &grant_key(tool, &input))
        .await
        .unwrap());
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);
}
```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust

use common::fake_dot;
use dots_core::engine::claude::args::{build_args, mcp_config, resolve_claude, resolve_in_dirs};
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::{EngineEvent, RunContext};
use dots_core::library::{LibraryConfig, LibraryItem};
use dots_core::model::WorkspaceMode;
use dots_core::policy::{Policy, Preset};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

```

**with:**

```rust

use common::fake_dot;
use dots_core::engine::claude::args::{
    build_args, gateway_entry, mcp_config, resolve_claude, resolve_in_dirs,
};
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::{EngineEvent, RunContext};
use dots_core::model::WorkspaceMode;
use dots_core::policy::{Policy, Preset};
use serde_json::json;
use tokio_util::sync::CancellationToken;

```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
    dot.spec.max_turns = 12;
    dot.spec.policy = Policy::preset(Preset::ReadOnly);
    dot.spec.uses.mcp_servers = vec!["sf".into(), "gh".into()];
    let item = |name: &str, config: Value| LibraryItem {
        id: format!("ID-{name}"),
        name: name.into(),
        config: LibraryConfig::McpServer(serde_json::from_value(config).unwrap()),
        owner_id: None,
        created_at: "t".into(),
        updated_at: "t".into(),
    };
    RunContext {
        run_id: "RUN1".into(),
```

**with:**

```rust
    dot.spec.max_turns = 12;
    dot.spec.policy = Policy::preset(Preset::ReadOnly);
    RunContext {
        run_id: "RUN1".into(),
```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust
        cancel: CancellationToken::new(),
        approved_calls: Vec::new(),
        mcp_servers: vec![
            item(
                "sf",
                json!({"transport": "stdio", "command": "sf-mcp", "args": ["--org", "${SF_ORG}"],
                       "env": {"SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly"}}),
            ),
            item(
                "gh",
                json!({"transport": "http", "url": "https://gh.example.com/mcp",
                       "headers": {"Authorization": "Bearer ${GH_TOKEN}"}}),
            ),
            item(
                "lost",
                json!({"transport": "stdio", "command": "lost", "env": {"K": "${NOT_SET}"}}),
            ),
        ],
        tools: Default::default(),
    }
}

fn test_env(name: &str) -> Option<String> {
    match name {
        "SF_ORG" => Some("acme".into()),
        "SF_TOKEN" => Some("sf-secret".into()),
        "GH_TOKEN" => Some("gh-secret".into()),
        _ => None,
    }
}
```

**with:**

```rust
        cancel: CancellationToken::new(),
        approved_calls: Vec::new(),
        tools: Default::default(),
    }
}
```

**In `crates/dots-core/tests/claude_cli_test.rs`, replace:**

```rust

#[test]
fn mcp_config_lists_the_library_servers_with_their_variables_and_adds_dots() {
    let cfg = mcp_config(&ctx(None), &test_env);
    assert_eq!(
        cfg["mcpServers"]["sf"],
        json!({"type": "stdio", "command": "sf-mcp", "args": ["--org", "acme"],
               "env": {"MODE": "readonly", "SF_TOKEN": "sf-secret"}})
    );
    assert_eq!(
        cfg["mcpServers"]["gh"],
        json!({"type": "http", "url": "https://gh.example.com/mcp",
               "headers": {"Authorization": "Bearer gh-secret"}})
    );
    // A server whose variable is not set is left out.
    assert!(cfg["mcpServers"].get("lost").is_none(), "{cfg}");
    assert_eq!(cfg["mcpServers"].as_object().unwrap().len(), 3);
    assert_eq!(cfg["mcpServers"]["dots"]["type"], "http");
    assert_eq!(
        cfg["mcpServers"]["dots"]["url"],
        "http://127.0.0.1:47321/mcp"
    );
    assert_eq!(
        cfg["mcpServers"]["dots"]["headers"]["Authorization"],
        "Bearer SECRET"
    );
}

```

**with:**

```rust

#[test]
fn mcp_config_lists_dots_and_a_gateway_entry_per_open_server() {
    // No open session: Dots' own server only.
    assert_eq!(
        mcp_config(&ctx(None)),
        json!({"mcpServers": {"dots": {"type": "http", "url": "http://127.0.0.1:47321/mcp",
               "headers": {"Authorization": "Bearer SECRET"}}}})
    );
    // An open session is reached through the gateway with the run's secret, never directly.
    assert_eq!(
        gateway_entry("http://127.0.0.1:47321/mcp", "SECRET", "sf"),
        json!({"type": "http", "url": "http://127.0.0.1:47321/mcp/s/sf",
               "headers": {"Authorization": "Bearer SECRET"}})
    );
}

#[test]
fn init_plugin_errors_become_notices() {
    let line = json!({"type": "system", "subtype": "init", "session_id": "s1", "plugin_errors": [
        {"plugin": "forms", "error": "manifest is not valid JSON"},
        {"message": "no plugin.json"},
        "dots-skills: skills/pdf has no SKILL.md",
        {"code": 7}
    ]})
    .to_string();
    let notice = |text: &str| EngineEvent::Notice { text: text.into() };
    assert_eq!(
        parse_line(&line),
        vec![
            EngineEvent::SessionStarted {
                session_id: "s1".into()
            },
            notice("plugin forms: manifest is not valid JSON"),
            notice("plugin error: no plugin.json"),
            notice("plugin error: dots-skills: skills/pdf has no SKILL.md"),
            notice(r#"plugin error: {"code":7}"#),
        ]
    );
    // An init without errors is the session alone, as before.
    assert_eq!(parse_line(INIT).len(), 1);
}

```

**Create `crates/dots-core/tests/claude_gateway_test.rs`:**

```rust
//! The Claude engine's MCP servers, skills and plugins (MCP/skills spec sections 5 and 7): the
//! gateway `POST /mcp/s/<server>` that checks each call, and the run's `--mcp-config`,
//! `--allowedTools` and `--plugin-dir`, driven through `fake-claude` and `fake-mcp`.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::{parked_message, ApprovalHub, DecideEffect};
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus};
use dots_core::library::{LibraryConfig, LibraryItem, McpServerConfig, PluginConfig, SkillConfig};
use dots_core::library_service::LibraryService;
use dots_core::model::{
    Dot, DotSpec, EngineKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    bus: Bus,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    lib: LibraryService,
    base: String,
    args_out: PathBuf,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, and the local server.
/// Without `dispatch` nothing runs: a test opens a run's tools itself.
async fn env(script: Value, dispatch: bool) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let args_out = dir.path().join("args.json");
    let engine = ClaudeEngine::new(
        PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")),
        dir.path().join("scratch"),
    )
    .with_env("FAKE_CLAUDE_SCRIPT", &script_path.to_string_lossy())
    .with_env("FAKE_CLAUDE_ARGS_OUT", &args_out.to_string_lossy());
    let listener = bind(0).await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: format!("{base}/mcp"),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_secs(2),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner: runner.clone(),
            hub: hub.clone(),
        },
        CancellationToken::new(),
    );
    if dispatch {
        runner.spawn_dispatcher(CancellationToken::new());
    }
    Env {
        lib: LibraryService::new(store.clone(), bus.clone()),
        dir,
        store,
        bus,
        hub,
        runner,
        base,
        args_out,
    }
}

/// Adds `config` as the MCP server `name`, its tool list accepted as it is now.
async fn add_server(e: &Env, name: &str, config: McpServerConfig) -> (LibraryItem, String) {
    let probe = e.lib.probe(&config).await.unwrap();
    let item = e
        .lib
        .add(
            name,
            LibraryConfig::McpServer(config),
            Some(&probe.fingerprint),
        )
        .await
        .unwrap();
    (item, probe.fingerprint)
}

/// A Claude dot in the test folder, under the Sandboxed preset unless `edit` changes it.
async fn claude_dot(e: &Env, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec("gw");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 0;
    edit(&mut s);
    e.store.create_dot(&s).await.unwrap()
}

async fn start(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

/// What `fake-claude` recorded of its last start: `args`, `mcp_config`, `plugin_dirs`, ...
fn recorded(e: &Env) -> Value {
    serde_json::from_str(&std::fs::read_to_string(&e.args_out).unwrap()).unwrap()
}

/// The values after `flag`, up to the next flag.
fn values_of(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .skip_while(|a| *a != flag)
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect()
}

async fn events(e: &Env, run_id: &str, kind: &str) -> Vec<RunEventRecord> {
    e.store
        .list_events(run_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == kind)
        .collect()
}

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

fn success() -> Value {
    json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                    "usage": {"input_tokens": 1, "output_tokens": 1}}})
}

fn mcp_call(server: &str, tool: &str, arguments: Value) -> Value {
    json!({"mcp_call": {"server": server, "tool": tool, "arguments": arguments}})
}

/// The resume run of `parent`, once the runner made it.
async fn resume_child(e: &Env, parent: &Run) -> Run {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let runs = e.store.list_runs(Some(&parent.dot_id), 10).await.unwrap();
        if let Some(r) = runs
            .into_iter()
            .find(|r| r.parent_run_id.as_deref() == Some(parent.id.as_str()))
        {
            return r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no resume run appeared"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_claude_run_reaches_its_servers_through_the_gateway_only() {
    let e = env(
        json!([init(),
               {"mcp_list": "files"},
               mcp_call("files", "read", json!({"q": "a"})),
               success()]),
        true,
    )
    .await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    add_server(&e, "files", fake.stdio()).await;
    let starts = || {
        fake.log()
            .iter()
            .filter(|r| r.get("start").is_some())
            .count()
    };
    let before = starts();
    let dot = claude_dot(&e, |s| s.uses.mcp_servers = vec!["files".into()]).await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let rec = recorded(&e);
    let args: Vec<String> = serde_json::from_value(rec["args"].clone()).unwrap();
    assert!(
        values_of(&args, "--allowedTools").contains(&"mcp__files__*".to_string()),
        "{args:?}"
    );
    let servers = &rec["mcp_config"]["mcpServers"];
    assert_eq!(servers.as_object().unwrap().len(), 2, "{servers}");
    assert_eq!(
        servers["files"],
        json!({"type": "http", "url": format!("{}/mcp/s/files", e.base),
               "headers": {"Authorization": servers["dots"]["headers"]["Authorization"]}})
    );
    // Only the run's own session started the server: the CLI reached it through Dots.
    assert_eq!(starts(), before + 1);

    // The CLI saw the session's tools, and its call reached the server.
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(listed["tools"], json!(["read", "write"]));
    let calls = events(&e, &run.id, "tool_call").await;
    assert_eq!(calls[0].data["tool"], "mcp__files__read");
    let result = &events(&e, &run.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], false);
    assert!(
        result["output"].as_str().unwrap().starts_with("read:"),
        "{result}"
    );
    let received = fake.received("tools/call");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0]["params"]["arguments"], json!({"q": "a"}));
    // The run's tools are gone with it.
    assert!(e.runner.run_tools(&run.id).is_none());
    assert!(fake.gone_within(Duration::from_secs(10)).await);
}

/// Runs a dot whose script writes through `files`: the call asks and parks. Returns the parked
/// run and its approval's id.
async fn parked_write(e: &Env, fake: &FakeMcp) -> (Run, String) {
    let dot = claude_dot(e, |s| s.uses.mcp_servers = vec!["files".into()]).await;
    let parent = start(e, &dot).await;
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 20).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approval.tool, "mcp__files__write");
    assert_eq!(approval.input, json!({"q": "w"}));
    let result = &events(e, &parent.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], true);
    assert_eq!(result["output"], parked_message(&approval.id));
    assert!(fake.received("tools/call").is_empty(), "asked, not sent");
    (parent, approval.id)
}

#[tokio::test]
async fn a_writing_call_asks_and_its_retry_goes_through_with_the_grant() {
    let e = env(
        json!([
            init(),
            mcp_call("files", "write", json!({"q": "w"})),
            success()
        ]),
        true,
    )
    .await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let (_, fingerprint) = add_server(&e, "files", fake.stdio()).await;
    let (parent, approval) = parked_write(&e, &fake).await;
    assert_eq!(
        e.store
            .get_approval(&approval)
            .await
            .unwrap()
            .mcp_fingerprint,
        Some(fingerprint)
    );
    let DecideEffect::Parked(_) = e.hub.decide(&approval, true, None).await.unwrap() else {
        panic!("expected parked")
    };
    e.runner.maybe_resume(&parent.id).await.unwrap();
    let child = resume_child(&e, &parent).await;
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await;
    let result = &events(&e, &child.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], false, "{result}");
    assert!(result["output"].as_str().unwrap().starts_with("write:"));
    assert_eq!(fake.received("tools/call").len(), 1);
}

#[tokio::test]
async fn a_grant_does_not_hold_once_the_servers_accepted_list_changed() {
    let e = env(
        json!([
            init(),
            mcp_call("files", "write", json!({"q": "w"})),
            success()
        ]),
        true,
    )
    .await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let (item, _) = add_server(&e, "files", fake.stdio()).await;
    let (parent, approval) = parked_write(&e, &fake).await;
    // The person accepts a new list before the retry: `write` now says something else.
    let mut write = tool("write", false);
    write["description"] = json!("Writes, and also deletes.");
    fake.rewrite(json!({"tools": [tool("read", true), write]}));
    let seen = e.lib.check(&item.id).await.unwrap();
    e.lib
        .accept(&item.id, seen.seen_fingerprint.as_deref().unwrap())
        .await
        .unwrap();
    e.hub.decide(&approval, true, None).await.unwrap();
    e.runner.maybe_resume(&parent.id).await.unwrap();
    let child = resume_child(&e, &parent).await;
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await;
    let result = &events(&e, &child.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], true);
    assert_eq!(
        result["output"],
        "mcp__files__write: not performed, because the tool changed since it was approved."
    );
    assert!(fake.received("tools/call").is_empty());
    // Nothing was asked again.
    assert!(e
        .store
        .approvals_for_run(&child.id)
        .await
        .unwrap()
        .is_empty());
}

fn write_file(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[tokio::test]
async fn a_claude_run_gets_its_plugin_folders_and_reports_plugin_errors() {
    let e = env(
        json!([{"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION",
                         "plugin_errors": [{"plugin": "forms", "error": "agents/x.md: bad"}]}},
               success()]),
        true,
    )
    .await;
    let skill = e.dir.path().join("lib").join("pdf");
    write_file(
        &skill.join("SKILL.md"),
        "---\nname: pdf\ndescription: Read PDFs.\nallowed-tools: Bash\n---\nBody.\n",
    );
    let fake = FakeMcp::new(json!({"tools": [tool("fill", false)]}));
    let plugin = e.dir.path().join("lib").join("forms");
    write_file(
        &plugin.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "forms"}"#,
    );
    write_file(
        &plugin.join("skills").join("fill").join("SKILL.md"),
        "---\ndescription: Fill forms.\n---\nFill.\n",
    );
    let McpServerConfig::Stdio { command, args, .. } = fake.stdio() else {
        unreachable!()
    };
    write_file(
        &plugin.join(".mcp.json"),
        &json!({"mcpServers": {"files": {"command": command, "args": args}}}).to_string(),
    );
    e.lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig {
                path: skill.to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    e.lib
        .add(
            "forms",
            LibraryConfig::Plugin(PluginConfig {
                path: plugin.to_string_lossy().into_owned(),
                installed_id: None,
                run_hooks: false,
            }),
            None,
        )
        .await
        .unwrap();
    let dot = claude_dot(&e, |s| {
        s.uses.skills = vec!["pdf".into()];
        s.uses.plugins = vec!["forms".into()];
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let rec = recorded(&e);
    let args: Vec<String> = serde_json::from_value(rec["args"].clone()).unwrap();
    let dirs: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--plugin-dir")
        .map(|w| w[1].clone())
        .collect();
    let seen: Vec<String> = rec["plugin_dirs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(dirs, seen);
    assert_eq!(dirs.len(), 2);
    assert!(dirs[0].ends_with("dots-skills"), "{dirs:?}");
    assert!(dirs[1].ends_with("forms"), "{dirs:?}");
    assert_eq!(
        rec["plugin_dirs"][0]["files"],
        json!([".claude-plugin/plugin.json", "skills/pdf/SKILL.md"])
    );
    // The plugin's copy has no `.mcp.json`: its server goes through the gateway.
    assert_eq!(
        rec["plugin_dirs"][1]["files"],
        json!([".claude-plugin/plugin.json", "skills/fill/SKILL.md"])
    );
    let servers = &rec["mcp_config"]["mcpServers"];
    assert_eq!(
        servers["plugin_forms_files"]["url"],
        format!("{}/mcp/s/plugin_forms_files", e.base)
    );
    assert!(values_of(&args, "--allowedTools").contains(&"mcp__plugin_forms_files__*".into()));
    // The folders go with the run.
    assert!(dirs.iter().all(|d| !Path::new(d).exists()));

    let notices: Vec<Value> = events(&e, &run.id, "notice")
        .await
        .into_iter()
        .map(|ev| ev.data["text"].clone())
        .collect();
    assert!(
        notices.contains(&json!("plugin forms: agents/x.md: bad")),
        "{notices:?}"
    );
}

async fn rpc(e: &Env, path: &str, secret: Option<&str>, body: Value) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{}{path}", e.base))
        .json(&body);
    if let Some(s) = secret {
        req = req.bearer_auth(s);
    }
    req.send().await.unwrap()
}

async fn call(e: &Env, secret: &str, tool: &str, arguments: Value) -> Value {
    rpc(
        e,
        "/mcp/s/files",
        Some(secret),
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
               "params": {"name": tool, "arguments": arguments}}),
    )
    .await
    .json()
    .await
    .unwrap()
}

#[tokio::test]
async fn the_gateway_answers_a_running_runs_open_servers_and_checks_each_call() {
    let e = env(json!([]), false).await;
    let fake = FakeMcp::new(json!({"tools": [
        tool("read", true), tool("write", false), tool("drop", false)]}));
    add_server(&e, "files", fake.stdio()).await;
    let dot = claude_dot(&e, |s| {
        s.uses.mcp_servers = vec!["files".into()];
        s.policy.rules.push(Rule {
            tool: "mcp__files__drop".into(),
            action: Action::Deny,
        });
    })
    .await;
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let (tools, notices) = RunTools::open(OpenRequest {
        store: &e.store,
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    let tools = Arc::new(tools);
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, tools.clone());
    let s = Some(secret.as_str());

    // The run's secret, and a server the run has open.
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    assert_eq!(
        rpc(&e, "/mcp/s/files", None, ping.clone()).await.status(),
        401
    );
    assert_eq!(
        rpc(&e, "/mcp/s/files", Some("made-up"), ping.clone())
            .await
            .status(),
        401
    );
    assert_eq!(rpc(&e, "/mcp/s/other", s, ping.clone()).await.status(), 404);
    let other = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let no_tools = e.runner.register_secret(&other.id);
    assert_eq!(
        rpc(&e, "/mcp/s/files", Some(&no_tools), ping.clone())
            .await
            .status(),
        404
    );
    let get = reqwest::Client::new()
        .get(format!("{}/mcp/s/files", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 405);

    let init: Value = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                          "clientInfo": {"name": "claude-code", "version": "x"}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "files");
    let note = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(note.status(), 202);
    let list: Value = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["read", "write", "drop"]);
    assert_eq!(
        list["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );

    // A read-only tool runs; the server's result comes back as it is.
    let read = call(&e, &secret, "read", json!({"q": "a"})).await;
    assert_eq!(read["result"]["content"][0]["type"], "text");
    assert!(read["result"].get("isError").is_none(), "{read}");
    // A rule of the dot denies; nothing reaches the server.
    let drop = call(&e, &secret, "drop", json!({})).await;
    assert_eq!(
        drop["result"],
        json!({"content": [{"type": "text",
               "text": "Denied by the policy of dot 'gw': mcp__files__drop is not permitted."}],
               "isError": true})
    );
    // A writing tool asks, and asking parks at once.
    let write = call(&e, &secret, "write", json!({"q": "w"})).await;
    let approval = e.store.approvals_for_run(&run.id).await.unwrap().remove(0);
    assert_eq!(approval.tool, "mcp__files__write");
    assert_eq!(
        approval.mcp_fingerprint,
        tools.server("files").unwrap().accepted_fingerprint()
    );
    assert_eq!(write["result"]["isError"], true);
    assert_eq!(
        write["result"]["content"][0]["text"],
        parked_message(&approval.id)
    );
    let received: Vec<Value> = fake
        .received("tools/call")
        .into_iter()
        .map(|m| m["params"]["name"].clone())
        .collect();
    assert_eq!(received, [json!("read")]);

    let bad = call(&e, &secret, "read", json!("not an object")).await;
    assert_eq!(bad["error"]["code"], -32602);
    let unknown: Value = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);

    // The CLI's `approve` tool leaves a gateway tool to the gateway, which checks every call.
    let approve: Value = rpc(
        &e,
        "/mcp",
        s,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "approve", "arguments": {
                   "tool_name": "mcp__files__write", "input": {"q": "x"}}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    let decision: Value =
        serde_json::from_str(approve["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        decision,
        json!({"behavior": "allow", "updatedInput": {"q": "x"}})
    );
    assert_eq!(e.store.approvals_for_run(&run.id).await.unwrap().len(), 1);

    tools.close().await;
}
```

**In `crates/dots-core/tests/engine_prompt_test.rs`, replace:**

```rust
        cancel,
        approved_calls: Vec::new(),
        mcp_servers: Vec::new(),
        tools: Default::default(),
    }
```

**with:**

```rust
        cancel,
        approved_calls: Vec::new(),
        tools: Default::default(),
    }
```

**In `crates/dots-core/tests/local_engine_test.rs`, replace:**

```rust
        cancel,
        approved_calls: approvals,
        mcp_servers: Vec::new(),
        tools: Default::default(),
    };
```

**with:**

```rust
        cancel,
        approved_calls: approvals,
        tools: Default::default(),
    };
```

**In `crates/dots-core/tests/resume_test.rs`, replace:**

```rust
        .resume_parked(
            &parent.id,
            &[("Bash".into(), "k".into())],
            std::slice::from_ref(&a.id),
            std::slice::from_ref(&a.id),
```

**with:**

```rust
        .resume_parked(
            &parent.id,
            &[("Bash".into(), "k".into(), None)],
            std::slice::from_ref(&a.id),
            std::slice::from_ref(&a.id),
```

**In `crates/dots-core/tests/runner_test.rs`, replace:**

```rust

#[tokio::test]
async fn a_run_is_given_the_library_servers_its_dot_uses() {
    use dots_core::library::{LibraryConfig, PluginConfig};
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(CapturingEngine::default());
    let runner = runner_with(&store, &bus, &dir, engine.clone(), 1);
    runner.spawn_dispatcher(CancellationToken::new());
    let server = |command: &str| {
        LibraryConfig::McpServer(
            serde_json::from_value(json!({"transport": "stdio", "command": command})).unwrap(),
        )
    };
    let plugin = store
        .add_library_item(
            "tools",
            &LibraryConfig::Plugin(PluginConfig {
                path: "C:/p".into(),
                installed_id: None,
                run_hooks: false,
            }),
            None,
        )
        .await
        .unwrap();
    let owned = store
        .add_library_item("plugin_tools_files", &server("f"), Some(&plugin.id))
        .await
        .unwrap();
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .add_library_item("gh", &server("gh-mcp"), None)
        .await
        .unwrap();
    let mut s = spec("uses");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.uses.mcp_servers = vec!["sf".into()];
    s.uses.plugins = vec!["tools".into()];
    let dot = store.create_dot(&s).await.unwrap();
    let id = manual(&runner, &dot).await;
    wait_status(&store, &id, RunStatus::Succeeded, 5).await;
    let started = engine.0.lock().unwrap();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].mcp_servers, vec![owned, sf]);
}

#[tokio::test]
async fn a_run_gets_its_tools_records_what_was_left_out_and_closes_them_at_the_end() {
    use common::fake_mcp::{tool, FakeMcp};
```

**with:**

```rust

#[tokio::test]
async fn a_run_gets_its_tools_records_what_was_left_out_and_closes_them_at_the_end() {
    use common::fake_mcp::{tool, FakeMcp};
```

**In `crates/dots-core/tests/store_runs_test.rs`, replace:**

```rust
            .resume_parked(
                &parent.id,
                &[("Bash".into(), "k".into())],
                &all,
                &[own.id.clone(), bad.clone()],
```

**with:**

```rust
            .resume_parked(
                &parent.id,
                &[("Bash".into(), "k".into(), None)],
                &all,
                &[own.id.clone(), bad.clone()],
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p dots-core --test approvals_test --test claude_cli_test --test claude_gateway_test --test engine_prompt_test --test local_engine_test --test resume_test --test runner_test --test store_runs_test --no-fail-fast 2>&1 | grep -E "^error(\[E[0-9]+\])?:|could not compile|^test result|^test .* FAILED" | sort | uniq -c | sort -rn
```

Expected: the eight test targets do not compile. The decisive lines: 3 × `` error[E0063]: missing field `mcp_servers` in initializer of `RunContext` `` (the tests no longer set the interim field), 2 × `` no method named `create_mcp_grant` found for struct `Store` ``, 2 × `error[E0308]: mismatched types` (the grant tuples of `resume_parked`), `` no method named `run_tools` found for struct `Arc<Runner>` ``, `` no method named `register_tools` found for struct `Arc<Runner>` ``, `` unresolved import `dots_core::engine::claude::args::gateway_entry` ``, `error[E0061]: this function takes 2 arguments but 1 argument was supplied` (`mcp_config(ctx)`), then `could not compile` for each of the eight.

- [ ] **Step 3: Implement**

**In `crates/dots-core/migrations/0006_library.sql`, replace:**

```sql
ALTER TABLE approvals ADD COLUMN mcp_fingerprint TEXT;

-- The Library items each dot uses, by name.
ALTER TABLE dots ADD COLUMN uses TEXT NOT NULL
```

**with:**

```sql
ALTER TABLE approvals ADD COLUMN mcp_fingerprint TEXT;

-- A one-shot grant of an approved MCP call holds only under this fingerprint (the approval's).
ALTER TABLE grants ADD COLUMN mcp_fingerprint TEXT;

-- The Library items each dot uses, by name.
ALTER TABLE dots ADD COLUMN uses TEXT NOT NULL
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
        input_hash: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO grants (id, root_run_id, tool, input_hash, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(new_id())
```

**with:**

```rust
        input_hash: &str,
    ) -> Result<()> {
        self.create_mcp_grant(root_run_id, tool, input_hash, None)
            .await
    }

    /// A one-shot grant of an approved MCP call, which holds only while the server's accepted
    /// fingerprint is `mcp_fingerprint` (the approval's).
    pub async fn create_mcp_grant(
        &self,
        root_run_id: &str,
        tool: &str,
        input_hash: &str,
        mcp_fingerprint: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO grants (id, root_run_id, tool, input_hash, mcp_fingerprint, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
        .bind(tool)
        .bind(input_hash)
        .bind(now())
        .execute(&self.pool)
```

**with:**

```rust
        .bind(tool)
        .bind(input_hash)
        .bind(mcp_fingerprint)
        .bind(now())
        .execute(&self.pool)
```

**In `crates/dots-core/src/store/approvals.rs`, replace:**

```rust
        Ok(res.rows_affected() == 1)
    }
}
```

**with:**

```rust
        Ok(res.rows_affected() == 1)
    }

    /// Consumes one unused grant of an MCP call. `None` when there is none; otherwise the
    /// fingerprint it was approved under, which the caller compares with the server's.
    pub async fn take_mcp_grant(
        &self,
        root_run_id: &str,
        tool: &str,
        input_hash: &str,
    ) -> Result<Option<Option<String>>> {
        let taken: Option<(Option<String>,)> = sqlx::query_as(
            "UPDATE grants SET used = 1 WHERE id = (SELECT id FROM grants WHERE root_run_id = ? \
             AND tool = ? AND input_hash = ? AND used = 0 LIMIT 1) RETURNING mcp_fingerprint",
        )
        .bind(root_run_id)
        .bind(tool)
        .bind(input_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(taken.map(|(fingerprint,)| fingerprint))
    }
}
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust

    /// Atomically turns the decided parked approvals of `parent_id` into a resume:
    /// inserts the one-shot `grants` (`(tool, grant_key)`, scoped to the parent's
    /// `root_run_id`, read inside the transaction), marks `resolved_ids` resolved, queues
    /// `queued_ids` (approved calls that Dots performs itself in the child run, which get no
    /// grant), inserts the optional `child` run, and closes the parent as `succeeded`.
```

**with:**

```rust

    /// Atomically turns the decided parked approvals of `parent_id` into a resume:
    /// inserts the one-shot `grants` (`(tool, grant_key, mcp_fingerprint)`, scoped to the
    /// parent's `root_run_id`, read inside the transaction), marks `resolved_ids` resolved, queues
    /// `queued_ids` (approved calls that Dots performs itself in the child run, which get no
    /// grant), inserts the optional `child` run, and closes the parent as `succeeded`.
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
        &self,
        parent_id: &str,
        grants: &[(String, String)],
        resolved_ids: &[String],
        queued_ids: &[String],
```

**with:**

```rust
        &self,
        parent_id: &str,
        grants: &[(String, String, Option<String>)],
        resolved_ids: &[String],
        queued_ids: &[String],
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
            .fetch_one(&mut *tx)
            .await?;
        for (tool, key) in grants {
            sqlx::query(
                "INSERT INTO grants (id, root_run_id, tool, input_hash, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(new_id())
```

**with:**

```rust
            .fetch_one(&mut *tx)
            .await?;
        for (tool, key, mcp_fingerprint) in grants {
            sqlx::query(
                "INSERT INTO grants (id, root_run_id, tool, input_hash, mcp_fingerprint, \
                 created_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
```

**In `crates/dots-core/src/store/runs.rs`, replace:**

```rust
            .bind(tool)
            .bind(key)
            .bind(now())
            .execute(&mut *tx)
```

**with:**

```rust
            .bind(tool)
            .bind(key)
            .bind(mcp_fingerprint)
            .bind(now())
            .execute(&mut *tx)
```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
    Parked(Approval),
}

pub fn parked_message(id: &str) -> String {
```

**with:**

```rust
    Parked(Approval),
}

/// Why an approved MCP call is not performed after its server's accepted tool list changed.
pub const TOOL_CHANGED: &str = "the tool changed since it was approved";

pub fn parked_message(id: &str) -> String {
```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
    }

    /// No one-shot grant is taken here: the local engine never gets one, and a grant for an
    /// MCP call must also match the server's fingerprint (the Claude engine's gateway).
    async fn check_mcp(
        &self,
```

**with:**

```rust
    }

    /// A one-shot grant (the Claude engine's retry of a call approved after it parked; the local
    /// engine never gets one) is used up first, and allows the call only while the server's
    /// accepted fingerprint is still the one the call was approved under (spec section 4).
    async fn check_mcp(
        &self,
```

**In `crates/dots-core/src/approvals.rs`, replace:**

```rust
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        let action = dot.spec.policy.resolve_mcp(tool, &input, trust);
        self.act(action, run_id, &dot, tool, input, fingerprint.as_deref())
```

**with:**

```rust
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        let key = grant_key(tool, &input);
        if let Some(granted) = self
            .store
            .take_mcp_grant(&run.root_run_id, tool, &key)
            .await?
        {
            return Ok(if granted == fingerprint {
                PermissionOutcome::Allow { input }
            } else {
                PermissionOutcome::Deny {
                    message: format!("{tool}: not performed, because {TOOL_CHANGED}."),
                }
            });
        }
        let action = dot.spec.policy.resolve_mcp(tool, &input, trust);
        self.act(action, run_id, &dot, tool, input, fingerprint.as_deref())
```

**In `crates/dots-core/src/engine/local/mcp_tools.rs`, replace:**

```rust

/// What the model is told about an approved call whose server changed its accepted list.
pub(super) const CHANGED: &str = "the tool changed since it was approved";

/// The function name of the MCP tool `qualified` (`mcp__<server>__<tool>`): the name itself
```

**with:**

```rust

/// What the model is told about an approved call whose server changed its accepted list.
pub(super) use crate::approvals::TOOL_CHANGED as CHANGED;

/// The function name of the MCP tool `qualified` (`mcp__<server>__<tool>`): the name itself
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
use tokio_util::sync::CancellationToken;

use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::policy::McpTrust;
```

**with:**

```rust
use tokio_util::sync::CancellationToken;

use crate::model::{ApplyState, Approval, Dot};
use crate::policy::McpTrust;
```

**In `crates/dots-core/src/engine/mod.rs`, replace:**

```rust
    /// `applies_approved_calls` gets any.
    pub approved_calls: Vec<Approval>,
    /// The Library's MCP servers the dot uses, its plugins' included, as stored: `${VAR}` not
    /// expanded (`Store::used_mcp_servers`).
    pub mcp_servers: Vec<LibraryItem>,
    /// The run's MCP sessions, skills and plugin folders, opened by the runner before the
    /// engine starts and closed when the run ends (`RunTools`).
    pub tools: Arc<RunTools>,
}
```

**with:**

```rust
    /// `applies_approved_calls` gets any.
    pub approved_calls: Vec<Approval>,
    /// The run's MCP sessions, skills and plugin folders, opened by the runner before the
    /// engine starts and closed when the run ends (`RunTools`). The local engine calls the
    /// sessions itself; the Claude CLI reaches them through the gateway.
    pub tools: Arc<RunTools>,
}
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
    active: Mutex<HashMap<String, Active>>,
    secrets: Mutex<HashMap<String, String>>,
    wake: Notify,
    resume_lock: tokio::sync::Mutex<()>,
```

**with:**

```rust
    active: Mutex<HashMap<String, Active>>,
    secrets: Mutex<HashMap<String, String>>,
    /// Each running run's open tools, by run id, for the MCP gateway.
    tools: Mutex<HashMap<String, Arc<RunTools>>>,
    wake: Notify,
    resume_lock: tokio::sync::Mutex<()>,
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            active: Mutex::new(HashMap::new()),
            secrets: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            resume_lock: tokio::sync::Mutex::new(()),
```

**with:**

```rust
            active: Mutex::new(HashMap::new()),
            secrets: Mutex::new(HashMap::new()),
            tools: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            resume_lock: tokio::sync::Mutex::new(()),
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            .find(|(s, _)| ct_eq(s, secret))
            .map(|(_, run)| run.clone())
    }

```

**with:**

```rust
            .find(|(s, _)| ct_eq(s, secret))
            .map(|(_, run)| run.clone())
    }

    /// Makes the run's open tools reachable through the MCP gateway (`POST /mcp/s/<server>`,
    /// spec section 5) until the run ends.
    pub fn register_tools(&self, run_id: &str, tools: Arc<RunTools>) {
        self.tools.lock().unwrap().insert(run_id.to_string(), tools);
    }

    /// The open tools of a running run (found by its secret, `run_for_secret`).
    pub fn run_tools(&self, run_id: &str) -> Option<Arc<RunTools>> {
        self.tools.lock().unwrap().get(run_id).cloned()
    }

    /// The cancel token of a run the runner holds: a call the gateway sends for the run stops
    /// when it fires.
    pub fn run_cancel(&self, run_id: &str) -> Option<CancellationToken> {
        self.active
            .lock()
            .unwrap()
            .get(run_id)
            .map(|a| a.cancel.clone())
    }

    /// Forgets what the MCP endpoints know of a run that no longer runs.
    fn forget_run(&self, run_id: &str) {
        self.secrets.lock().unwrap().retain(|_, r| r != run_id);
        self.tools.lock().unwrap().remove(run_id);
    }

```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
        cancel.cancel();
        self.active.lock().unwrap().remove(run_id);
        self.secrets.lock().unwrap().retain(|_, r| r != run_id);
        match self
            .store
```

**with:**

```rust
        cancel.cancel();
        self.active.lock().unwrap().remove(run_id);
        self.forget_run(run_id);
        match self
            .store
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            }
        }
        self.secrets.lock().unwrap().retain(|_, r| r != &run_id);
        self.close_applies(&run_id).await;
        // A cancel that arrived after the final status was computed (the run parked) is
```

**with:**

```rust
            }
        }
        self.forget_run(&run_id);
        self.close_applies(&run_id).await;
        // A cancel that arrived after the final status was computed (the run parked) is
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
                }
                (ApprovalStatus::Approved, _) => {
                    grants.push((a.tool.clone(), grant_key(&a.tool, &a.input)));
                    lines.push(format!(
                        "- Approval #{} granted: you may now use {} with the same input. Perform that action now.",
```

**with:**

```rust
                }
                (ApprovalStatus::Approved, _) => {
                    grants.push((
                        a.tool.clone(),
                        grant_key(&a.tool, &a.input),
                        a.mcp_fingerprint.clone(),
                    ));
                    lines.push(format!(
                        "- Approval #{} granted: you may now use {} with the same input. Perform that action now.",
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            _ => Vec::new(),
        };
        let mcp_servers = self.store.used_mcp_servers(&dot.spec.uses).await?;
        let secret = self.register_secret(&run.id);
        let ctx = RunContext {
            run_id: run.id.clone(),
```

**with:**

```rust
            _ => Vec::new(),
        };
        let secret = self.register_secret(&run.id);
        self.register_tools(&run.id, tools.clone());
        let ctx = RunContext {
            run_id: run.id.clone(),
```

**In `crates/dots-core/src/runner.rs`, replace:**

```rust
            cancel: cancel.clone(),
            approved_calls,
            mcp_servers,
            tools: tools.clone(),
        };
```

**with:**

```rust
            cancel: cancel.clone(),
            approved_calls,
            tools: tools.clone(),
        };
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
const DEFAULT_PROTOCOL: &str = "2025-06-18";

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
        .into_response()
}

```

**with:**

```rust
const DEFAULT_PROTOCOL: &str = "2025-06-18";

pub(super) fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
        .into_response()
}

/// The JSON-RPC request in `body` and its id. `Err` is the answer when there is nothing to
/// answer with a result: `202` for a notification, an error for a body that is not one request.
pub(super) fn read_request(body: &[u8]) -> Result<(Value, Value), Box<Response>> {
    let req: Value = serde_json::from_slice(body)
        .map_err(|e| Box::new(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))))?;
    if req.is_array() {
        return Err(Box::new(rpc_error(
            Value::Null,
            -32600,
            "batch requests are not supported",
        )));
    }
    match req.get("id").cloned() {
        Some(id) => Ok((id, req)),
        None => Err(Box::new(StatusCode::ACCEPTED.into_response())),
    }
}

/// The `initialize` result of a tools-only server named `name`, in the client's protocol
/// version.
pub(super) fn initialize_result(req: &Value, name: &str) -> Value {
    json!({
        "protocolVersion": req
            .pointer("/params/protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL),
        "capabilities": { "tools": {} },
        "serverInfo": { "name": name, "version": env!("CARGO_PKG_VERSION") }
    })
}

```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
    };
    if req.is_array() {
        return rpc_error(Value::Null, -32600, "batch requests are not supported");
    }
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let method = req
```

**with:**

```rust
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let (id, req) = match read_request(&body) {
        Ok(read) => read,
        Err(answer) => return *answer,
    };
    let method = req
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
        .unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": req
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "dots", "version": env!("CARGO_PKG_VERSION") }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [approve_tool()] }),
```

**with:**

```rust
        .unwrap_or_default();
    let result = match method {
        "initialize" => initialize_result(&req, "dots"),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [approve_tool()] }),
```

**In `crates/dots-core/src/server/mcp.rs`, replace:**

```rust
        .ok_or((-32602, "arguments.tool_name is required".to_string()))?;
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let decision = match s.hub.check(run_id, tool, input).await {
        Ok(PermissionOutcome::Allow { input }) => {
```

**with:**

```rust
        .ok_or((-32602, "arguments.tool_name is required".to_string()))?;
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    // A tool of the run's open sessions goes through the gateway, which checks every call
    // (`gateway.rs`): checking it here as well would ask twice.
    let gateway = s
        .runner
        .run_tools(run_id)
        .is_some_and(|tools| tools.resolve(tool).is_some());
    if gateway {
        let allow = json!({ "behavior": "allow", "updatedInput": input });
        return Ok(json!({ "content": [{ "type": "text", "text": allow.to_string() }] }));
    }
    let decision = match s.hub.check(run_id, tool, input).await {
        Ok(PermissionOutcome::Allow { input }) => {
```

**Create `crates/dots-core/src/server/gateway.rs`:**

```rust
//! The MCP gateway of Claude-engine runs (MCP/skills spec section 5): `POST /mcp/s/<server>`
//! is one MCP server for the CLI, backed by the run's open session with that server. The run
//! is found by its secret, as for `/mcp`. Each `tools/call` goes through the same check as a
//! local-engine call (`PermissionGate::check_mcp`), waits for an approval when it asks, and is
//! then sent to the server; a call that is not performed is a tool result with `isError`.

use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::mcp::{initialize_result, read_request, rpc_error};
use super::webhook::bearer;
use super::ServerState;
use crate::engine::{PermissionGate, PermissionOutcome};
use crate::mcp::CALL_TIMEOUT;
use crate::run_tools::RunServer;

/// How long before the CLI gives up on a call (`MCP_TOOL_TIMEOUT`) the gateway stops waiting
/// for the server, so its answer still reaches the CLI.
const ANSWER_MARGIN: Duration = Duration::from_secs(2);

pub async fn handle(
    State(s): State<ServerState>,
    Path(server): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(run_id) = bearer(&headers).and_then(|t| s.runner.run_for_secret(&t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(tools) = s.runner.run_tools(&run_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(open) = tools.server(&server) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (id, req) = match read_request(&body) {
        Ok(read) => read,
        Err(answer) => return *answer,
    };
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => initialize_result(&req, open.name()),
        "ping" => json!({}),
        "tools/list" => {
            // A list change the server announced is listed first.
            open.settled().await;
            json!({ "tools": open.tools() })
        }
        "tools/call" => match call(&s, &run_id, open, &req).await {
            Ok(v) => v,
            Err((code, msg)) => return rpc_error(id, code, &msg),
        },
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

/// A tool result that says why the call was not performed (or how it failed), in the words the
/// local engine gives its model.
fn not_performed(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// Checks the call of `req`, waits for its approval if it asks, and sends it to the server.
/// The server's result is returned as it came.
async fn call(
    s: &ServerState,
    run_id: &str,
    server: &RunServer,
    req: &Value,
) -> Result<Value, (i64, String)> {
    let started = Instant::now();
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let tool = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "params.name is required".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let qualified = format!("mcp__{}__{tool}", server.name());
    // A list change announced with an earlier result holds the server before this check.
    server.settled().await;
    let checked = s
        .hub
        .check_mcp(
            run_id,
            &qualified,
            input,
            server.trust(tool),
            server.accepted_fingerprint(),
        )
        .await;
    let input = match checked {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let limit = cli_limit(s, run_id)
        .await
        .saturating_sub(started.elapsed())
        .max(Duration::from_secs(1));
    let cancel = s.runner.run_cancel(run_id).unwrap_or_default();
    Ok(
        match server
            .session()
            .call_tool(tool, input, limit, &cancel)
            .await
        {
            Ok(result) => result,
            Err(e) => not_performed(&e.to_string()),
        },
    )
}

/// How long the CLI waits for a call of this run: `MCP_TOOL_TIMEOUT`, which is the dot's
/// `approval_wait_secs` + 30 s, less the margin; the local engine's limit when the dot cannot
/// be read.
async fn cli_limit(s: &ServerState, run_id: &str) -> Duration {
    let wait = match s.store.get_run(run_id).await {
        Ok(run) => s
            .store
            .get_dot(&run.dot_id)
            .await
            .map(|dot| dot.spec.approval_wait_secs),
        Err(e) => Err(e),
    };
    match wait {
        Ok(wait) => Duration::from_secs(wait + 30).saturating_sub(ANSWER_MARGIN),
        Err(_) => CALL_TIMEOUT,
    }
}
```

**In `crates/dots-core/src/server/mod.rs`, replace:**

```rust
use crate::Result;

pub mod mcp;
pub mod webhook;
```

**with:**

```rust
use crate::Result;

pub mod gateway;
pub mod mcp;
pub mod webhook;
```

**In `crates/dots-core/src/server/mod.rs`, replace:**

```rust
            "/mcp",
            post(mcp::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
```

**with:**

```rust
            "/mcp",
            post(mcp::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
        )
        // The gateway to a run's MCP server: Dots checks each call (MCP/skills spec §5).
        .route(
            "/mcp/s/{server}",
            post(gateway::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust

use crate::engine::RunContext;
use crate::library::{LibraryConfig, McpServerConfig};

pub fn build_args(ctx: &RunContext, mcp_config_path: &Path) -> Vec<String> {
```

**with:**

```rust

use crate::engine::RunContext;

pub fn build_args(ctx: &RunContext, mcp_config_path: &Path) -> Vec<String> {
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
        String::new()
    });
    let (allow, deny) = spec.policy.cli_lists();
    if !allow.is_empty() {
        a.push("--allowedTools".into());
```

**with:**

```rust
        String::new()
    });
    let (mut allow, deny) = spec.policy.cli_lists();
    // The gateway checks every call of a run's MCP server (spec section 4), so the CLI does not
    // ask the `approve` tool for them first.
    allow.extend(
        ctx.tools
            .servers()
            .iter()
            .map(|s| format!("mcp__{}__*", s.name())),
    );
    if !allow.is_empty() {
        a.push("--allowedTools".into());
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
        a.extend(deny);
    }
    if let Some(session) = &ctx.session_id {
        a.push("--resume".into());
```

**with:**

```rust
        a.extend(deny);
    }
    for dir in ctx.tools.plugin_dirs() {
        a.push("--plugin-dir".into());
        a.push(dir.to_string_lossy().to_string());
    }
    if let Some(session) = &ctx.session_id {
        a.push("--resume".into());
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
}

/// The run's `--mcp-config`: the Library's MCP servers the dot uses, with `${VAR}` read from
/// `env`, and Dots' own `dots` server. A server whose variable is not set is left out.
pub fn mcp_config(ctx: &RunContext, env: &dyn Fn(&str) -> Option<String>) -> Value {
    let mut servers = serde_json::Map::new();
    for item in &ctx.mcp_servers {
        let LibraryConfig::McpServer(config) = &item.config else {
            continue;
        };
        match config.expanded(env) {
            Ok(config) => {
                servers.insert(item.name.clone(), claude_entry(&config));
            }
            Err(e) => {
                tracing::warn!(run = %ctx.run_id, "MCP server {} left out: {e}", item.name)
            }
        }
    }
    servers.insert(
```

**with:**

```rust
}

/// The run's `--mcp-config`: Dots' own `dots` server and, for each MCP session the run has open,
/// an entry of the same name that reaches it through the gateway. The CLI never starts or
/// reaches a server itself, and no `${VAR}` value is written to the file.
pub fn mcp_config(ctx: &RunContext) -> Value {
    let mut servers = serde_json::Map::new();
    for server in ctx.tools.servers() {
        servers.insert(
            server.name().to_string(),
            gateway_entry(&ctx.mcp_url, &ctx.mcp_secret, server.name()),
        );
    }
    servers.insert(
```

**In `crates/dots-core/src/engine/claude/args.rs`, replace:**

```rust
}

/// A server as an entry of Claude Code's `mcpServers`.
fn claude_entry(config: &McpServerConfig) -> Value {
    match config {
        McpServerConfig::Stdio {
            command,
            args,
            cwd,
            env,
        } => {
            let mut entry =
                json!({ "type": "stdio", "command": command, "args": args, "env": env });
            if let Some(cwd) = cwd {
                entry["cwd"] = json!(cwd);
            }
            entry
        }
        McpServerConfig::Http { url, headers } => {
            json!({ "type": "http", "url": url, "headers": headers })
        }
    }
}

```

**with:**

```rust
}

/// The `mcpServers` entry of the run's open session `server`: the gateway route
/// `<mcp_url>/s/<server>` with the run's secret (spec section 5).
pub fn gateway_entry(mcp_url: &str, secret: &str, server: &str) -> Value {
    json!({
        "type": "http",
        "url": format!("{}/s/{server}", mcp_url.trim_end_matches('/')),
        "headers": { "Authorization": format!("Bearer {secret}") },
    })
}

```

**In `crates/dots-core/src/engine/claude/parse.rs`, replace:**

```rust
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            match v.get("session_id").and_then(Value::as_str) {
                Some(s) => vec![EngineEvent::SessionStarted {
                    session_id: s.to_string(),
                }],
                None => raw(),
            }
        }
        Some("assistant") => blocks(&v).iter().filter_map(assistant_block).collect(),
```

**with:**

```rust
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            let mut out: Vec<EngineEvent> = v
                .get("session_id")
                .and_then(Value::as_str)
                .map(|s| EngineEvent::SessionStarted {
                    session_id: s.to_string(),
                })
                .into_iter()
                .collect();
            out.extend(plugin_errors(&v));
            if out.is_empty() {
                return raw();
            }
            out
        }
        Some("assistant") => blocks(&v).iter().filter_map(assistant_block).collect(),
```

**In `crates/dots-core/src/engine/claude/parse.rs`, replace:**

```rust
        _ => raw(),
    }
}

```

**with:**

```rust
        _ => raw(),
    }
}

/// The plugins the CLI could not load, from `system/init`'s `plugin_errors` (MCP/skills spec
/// section 7), each as a notice. Its entries' shape is not documented, so an object is read for
/// a plugin name (`plugin`, `name` or `source`) and an error (`error` or `message`), a string
/// is the error, and anything else is shown as JSON.
fn plugin_errors(init: &Value) -> Vec<EngineEvent> {
    let Some(errors) = init.get("plugin_errors").and_then(Value::as_array) else {
        return Vec::new();
    };
    let field = |e: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| e.get(*k).and_then(Value::as_str))
            .map(str::to_string)
    };
    errors
        .iter()
        .map(|e| {
            let error = match e {
                Value::String(s) => Some(s.clone()),
                _ => field(e, &["error", "message"]),
            };
            let text = match (field(e, &["plugin", "name", "source"]), error) {
                (Some(plugin), Some(error)) => format!("plugin {plugin}: {error}"),
                (None, Some(error)) => format!("plugin error: {error}"),
                _ => format!("plugin error: {e}"),
            };
            EngineEvent::Notice { text }
        })
        .collect()
}

```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
        write_private(
            &mcp_path,
            &serde_json::to_vec_pretty(&args::mcp_config(&ctx, &|k| std::env::var(k).ok()))?,
        )
        .await?;
```

**with:**

```rust
        write_private(
            &mcp_path,
            &serde_json::to_vec_pretty(&args::mcp_config(&ctx))?,
        )
        .await?;
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
            let saw_terminal = Arc::new(AtomicBool::new(false));
            let flag = saw_terminal.clone();
            let mut stdout_task = tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
```

**with:**

```rust
            let saw_terminal = Arc::new(AtomicBool::new(false));
            let flag = saw_terminal.clone();
            let run_id = ctx.run_id.clone();
            let mut stdout_task = tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
```

**In `crates/dots-core/src/engine/claude/mod.rs`, replace:**

```rust
                    let line = text.trim_end_matches(['\r', '\n']);
                    for ev in parse::parse_line(line) {
                        let terminal = matches!(
                            ev,
```

**with:**

```rust
                    let line = text.trim_end_matches(['\r', '\n']);
                    for ev in parse::parse_line(line) {
                        // A plugin the CLI could not load (`system/init`'s `plugin_errors`).
                        if let EngineEvent::Notice { text } = &ev {
                            tracing::warn!(run = %run_id, "claude: {text}");
                        }
                        let terminal = matches!(
                            ev,
```

**In `crates/dots-core/src/bin/fake-claude.rs`, replace:**

```rust
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).ok();
    if let Ok(out) = std::env::var("FAKE_CLAUDE_ARGS_OUT") {
        let record = json!({
            "args": args,
```

**with:**

```rust
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).ok();
    let mcp: Option<Value> = flag(&args, "--mcp-config")
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap());
    if let Ok(out) = std::env::var("FAKE_CLAUDE_ARGS_OUT") {
        // What the CLI would load: the config and the plugin folders are gone after the run.
        let plugin_dirs: Vec<Value> = args
            .windows(2)
            .filter(|w| w[0] == "--plugin-dir")
            .map(|w| json!({"path": w[1], "files": files_below(Path::new(&w[1]))}))
            .collect();
        let record = json!({
            "args": args,
```

**In `crates/dots-core/src/bin/fake-claude.rs`, replace:**

```rust
            "cwd": std::env::current_dir().unwrap().to_string_lossy(),
            "mcp_tool_timeout": std::env::var("MCP_TOOL_TIMEOUT").ok(),
        });
        std::fs::write(out, record.to_string()).unwrap();
    }
    let session = flag(&args, "--resume").unwrap_or_else(|| "fake-session".to_string());
    let mcp: Option<Value> = flag(&args, "--mcp-config")
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap());
    let script: Vec<Value> = match std::env::var("FAKE_CLAUDE_SCRIPT") {
        Ok(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
```

**with:**

```rust
            "cwd": std::env::current_dir().unwrap().to_string_lossy(),
            "mcp_tool_timeout": std::env::var("MCP_TOOL_TIMEOUT").ok(),
            "mcp_config": mcp,
            "plugin_dirs": plugin_dirs,
        });
        std::fs::write(out, record.to_string()).unwrap();
    }
    let session = flag(&args, "--resume").unwrap_or_else(|| "fake-session".to_string());
    let script: Vec<Value> = match std::env::var("FAKE_CLAUDE_SCRIPT") {
        Ok(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
```

**In `crates/dots-core/src/bin/fake-claude.rs`, replace:**

```rust
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        }
    }
}
```

**with:**

```rust
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        } else if let Some(server) = step.get("mcp_list").and_then(Value::as_str) {
            let mcp = mcp.as_ref().expect("--mcp-config is required");
            let listed = gateway(mcp, server, "tools/list", json!({})).await;
            let names: Vec<Value> = listed["result"]["tools"]
                .as_array()
                .expect("a tool list")
                .iter()
                .map(|t| t["name"].clone())
                .collect();
            let line = json!({"type": "fake_mcp_tools", "server": server, "tools": names});
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        } else if let Some(call) = step.get("mcp_call") {
            let mcp = mcp.as_ref().expect("--mcp-config is required");
            let server = call["server"].as_str().expect("mcp_call.server");
            let tool = call["tool"].as_str().expect("mcp_call.tool");
            let id = format!("toolu_{i}");
            let use_line = json!({"type": "assistant", "message": {"content": [{
                "type": "tool_use", "id": id, "name": format!("mcp__{server}__{tool}"),
                "input": call["arguments"]}]}});
            writeln!(stdout, "{use_line}").unwrap();
            stdout.flush().unwrap();
            let params = json!({"name": tool, "arguments": call["arguments"]});
            let answer = gateway(mcp, server, "tools/call", params).await;
            let result = &answer["result"];
            let text: Vec<&str> = result["content"]
                .as_array()
                .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect())
                .unwrap_or_default();
            let line = json!({"type": "user", "message": {"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": id, "content": text.join("\n"),
                "is_error": result["isError"].as_bool().unwrap_or(false)}]}});
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        }
    }
}

/// Every file below `dir`, as paths relative to it with `/`, sorted.
fn files_below(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let rel = path.strip_prefix(dir).unwrap();
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push(parts.join("/"));
            }
        }
    }
    out.sort();
    out
}

/// Sends `method` to the `--mcp-config` entry `server`, after `initialize`, as the CLI reaches
/// an MCP server over http. Returns the JSON-RPC answer.
async fn gateway(mcp: &Value, server: &str, method: &str, params: Value) -> Value {
    let entry = &mcp["mcpServers"][server];
    let url = entry["url"].as_str().expect("an http entry");
    let auth = entry["headers"]["Authorization"]
        .as_str()
        .expect("an auth header");
    let client = reqwest::Client::new();
    let post = |body: Value| {
        client
            .post(url)
            .header("Authorization", auth)
            .header("Accept", "application/json, text/event-stream")
            .json(&body)
            .send()
    };
    post(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "fake-claude", "version": "0"}}}))
    .await
    .expect("initialize");
    post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await
        .expect("initialized");
    post(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .await
        .expect(method)
        .json()
        .await
        .expect("a JSON answer")
}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    src/scheduler.rs         the loop: a pass over the schedule rows (fire, catch up, skip,
                             complete), sleeping on a clock that tests move by hand
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint
    src/mcp/                 MCP client: one session per Library MCP server for a run (a stdio
                             child process in a Job Object, or Streamable HTTP), `${VAR}`
```

**with:**

```markdown
    src/scheduler.rs         the loop: a pass over the schedule rows (fire, catch up, skip,
                             complete), sleeping on a clock that tests move by hand
    src/server/              axum on 127.0.0.1:<port>: webhooks + MCP permission endpoint + the
                             MCP gateway of Claude runs (`/mcp/s/<server>`, MCP/skills spec §5)
    src/mcp/                 MCP client: one session per Library MCP server for a run (a stdio
                             child process in a Job Object, or Streamable HTTP), `${VAR}`
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
    src/engine/claude.rs     drives `claude -p` with stream-json I/O
    src/engine/local.rs      own tool-use loop over OpenAI-compatible chat completions, with
                             the run's MCP tools and the `skill` tool (MCP/skills spec §5, §7)
```

**with:**

```markdown
    src/runner/              queue, concurrency, timeouts, cancellation, restart recovery
    src/engine/mod.rs        Engine trait + EngineEvent
    src/engine/claude.rs     drives `claude -p` with stream-json I/O; the run's MCP servers
                             through the gateway, its skills and plugins with `--plugin-dir`
    src/engine/local.rs      own tool-use loop over OpenAI-compatible chat completions, with
                             the run's MCP tools and the `skill` tool (MCP/skills spec §5, §7)
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
`{"rule":{"kind":"cron","expr":...},"catch_up":false,"until":null}` (no catch-up, so an upgraded
schedule behaves as before), adds `runs.retry_of` and creates `schedule_state`; `0006` creates
`library_items` and `mcp_catalogs`, adds `approvals.mcp_fingerprint` and `dots.uses`, and drops
`dots.mcp_servers`. Its conversion needs code, so the SQL keeps each dot's old servers in a
table `upgrade_0006` and the store finishes it when it opens, in one transaction, then drops
```

**with:**

```markdown
`{"rule":{"kind":"cron","expr":...},"catch_up":false,"until":null}` (no catch-up, so an upgraded
schedule behaves as before), adds `runs.retry_of` and creates `schedule_state`; `0006` creates
`library_items` and `mcp_catalogs`, adds `approvals.mcp_fingerprint`, `grants.mcp_fingerprint`
and `dots.uses`, and drops
`dots.mcp_servers`. Its conversion needs code, so the SQL keeps each dot's old servers in a
table `upgrade_0006` and the store finishes it when it opens, in one transaction, then drops
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
an MCP tool is performed only while the server's accepted fingerprint is the one stored with the
approval (`approvals.mcp_fingerprint`); otherwise it is skipped with "the tool changed since it
was approved" (MCP/skills spec §4).

### Steps
```

**with:**

```markdown
an MCP tool is performed only while the server's accepted fingerprint is the one stored with the
approval (`approvals.mcp_fingerprint`); otherwise it is skipped with "the tool changed since it
was approved" (MCP/skills spec §4). On the Claude engine the grant carries that fingerprint
(`grants.mcp_fingerprint`), and the retried call is checked against it (§5).

### Steps
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  `allow` rules → `ask` rules → the accepted list's `readOnlyHint` allows → `mcp_default`),
  through the same approval queue (`PermissionGate::check_mcp`). Its approval stores the
  server's accepted fingerprint (`approvals.mcp_fingerprint`). The local engine takes no grant
  for it.
- From Approvals v2 an approval can save an allow rule to the dot's policy, in one transaction
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
```

**with:**

```markdown
  `allow` rules → `ask` rules → the accepted list's `readOnlyHint` allows → `mcp_default`),
  through the same approval queue (`PermissionGate::check_mcp`). Its approval stores the
  server's accepted fingerprint (`approvals.mcp_fingerprint`). The local engine never gets a
  grant for it. The Claude engine's retry does: `check_mcp` uses up a grant of the call first
  and allows it only while the server's accepted fingerprint is the one the grant carries
  (`grants.mcp_fingerprint`, the approval's); otherwise the call is not performed, with "the tool
  changed since it was approved".
- From Approvals v2 an approval can save an allow rule to the dot's policy, in one transaction
  with the decision (`decide_approval` with `allow_rule`). Only a pattern that
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

````markdown
claude -p --output-format stream-json --input-format stream-json --verbose
  --model <model> --max-turns <n>
  --allowedTools <allow patterns> --disallowedTools <deny patterns>
  --permission-prompt-tool mcp__dots__approve
  --mcp-config <per-run temp json> --strict-mcp-config
  --setting-sources ""                 ("user,project,local" when use_user_settings = 1)
  [--resume <session_id>]
```
````

**with:**

````markdown
claude -p --output-format stream-json --input-format stream-json --verbose
  --model <model> --max-turns <n>
  --allowedTools <allow patterns> [mcp__<server>__* ...] --disallowedTools <deny patterns>
  --permission-prompt-tool mcp__dots__approve
  --mcp-config <per-run temp json> --strict-mcp-config
  --setting-sources ""                 ("user,project,local" when use_user_settings = 1)
  [--plugin-dir <run plugin folder> ...]
  [--resume <session_id>]
```
````

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
`(approval_wait_secs + 30) * 1000` on a best-effort basis; CLI 2.1.295 ignores it for the
permission-prompt tool (observed hard cap of about 300 s; CLI 2.1.295 observed 2026-10-09). The per-run MCP config contains the dots MCP server
(`http://127.0.0.1:<port>/mcp`, header `Authorization: Bearer <per-run secret>`) plus the Library
MCP servers the dot uses, its plugins' included, with `${VAR}` read from Dots' environment (a
server whose variable is not set is left out). An empty `--setting-sources ""` keeps every settings file out of headless
runs: the user's global hooks (GateGuard, caveman, memory hooks) and a repo's committed
`.claude/settings.json`, which could otherwise widen the CLI's own allow list beyond the dot
```

**with:**

```markdown
`(approval_wait_secs + 30) * 1000` on a best-effort basis; CLI 2.1.295 ignores it for the
permission-prompt tool (observed hard cap of about 300 s; CLI 2.1.295 observed 2026-10-09). The per-run MCP config contains the dots MCP server
(`http://127.0.0.1:<port>/mcp`, header `Authorization: Bearer <per-run secret>`) plus, from
MCP/skills (spec §5), one entry per MCP session the run has open (`RunTools`: the Library servers
the dot uses, its plugins' included), named as the server and pointing at the gateway
(`http://127.0.0.1:<port>/mcp/s/<server>`, the same secret). The CLI never starts or reaches a
server itself, and no `${VAR}` value is written to the file. `--allowedTools` gets
`mcp__<server>__*` for each of them, so the CLI does not ask `approve` for these calls: the
gateway checks each one (§6), and the `approve` tool answers allow for a gateway tool in case it
is asked. Each of the run's plugin folders (MCP/skills spec §7: `dots-skills` with the dot's
skills, then a copy of each plugin) is passed with `--plugin-dir`; each entry of the
`system/init` event's `plugin_errors` becomes a `notice` run event and a log line. An empty `--setting-sources ""` keeps every settings file out of headless
runs: the user's global hooks (GateGuard, caveman, memory hooks) and a repo's committed
`.claude/settings.json`, which could otherwise widen the CLI's own allow list beyond the dot
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
- `POST /mcp` — MCP streamable-HTTP endpoint served with `rmcp`, exposing the single tool
  `approve`. Requires the per-run secret; the secret identifies the run.
- `GET /health` — `200`.

```

**with:**

```markdown
- `POST /mcp` — MCP streamable-HTTP endpoint served with `rmcp`, exposing the single tool
  `approve`. Requires the per-run secret; the secret identifies the run.
- `POST /mcp/s/{server}` — from MCP/skills (spec §5), the gateway to the run's open MCP session
  `server` (`404` when the run has none of that name). Requires the per-run secret. It answers
  `initialize`, `tools/list` (the session's current list) and `tools/call`: the call is checked
  by `check_mcp` with what the run knows of the server, waits for an approval when it asks, and
  is then sent to the server, whose result is returned as it came. A denied, parked or failed
  call is a tool result with `isError: true` and the text the local engine gives; the call's
  limit is what is left of `MCP_TOOL_TIMEOUT`.
- `GET /health` — `200`.

```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  the MCP `approve` endpoint at scripted points. Drives end-to-end tests of runner + approval
  flow (approve in window, deny, timeout → park → resume) with no network or subscription use.
- **fake-mcp:** a scripted MCP server over stdio or Streamable HTTP (tools, read-only hints,
  results, errors, delays, a list change with `notifications/tools/list_changed`, a call log);
```

**with:**

```markdown
  the MCP `approve` endpoint at scripted points. Drives end-to-end tests of runner + approval
  flow (approve in window, deny, timeout → park → resume) with no network or subscription use.
  From MCP/skills it also records the `--mcp-config` it was given and the files of each
  `--plugin-dir`, and lists or calls a gateway tool at scripted points.
- **fake-mcp:** a scripted MCP server over stdio or Streamable HTTP (tools, read-only hints,
  results, errors, delays, a list change with `notifications/tools/list_changed`, a call log);
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p dots-core --test approvals_test --test claude_cli_test --test claude_gateway_test --test engine_prompt_test --test local_engine_test --test resume_test --test runner_test --test store_runs_test 2>&1 | grep -E "^test result|FAILED"
```

Expected: `approvals_test` 16 passed, `claude_cli_test` 11 (on Windows), `claude_gateway_test` 5, `engine_prompt_test` 9, `local_engine_test` 16, `resume_test` 18, `runner_test` 13 (the interim test of Task 2 is removed), `store_runs_test` 27. `claude_gateway_test` runs `fake-claude` against the gateway and `fake-mcp`; one of its tests checks that `fake-mcp` starts exactly once for the run.

- [ ] **Step 5: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

Expected: `dots-core` 578 passed, `dots-app` 49 passed, no clippy line, `cargo fmt --check` silent. The UI and the contract are unchanged (293 in 21 files).

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/migrations/0006_library.sql crates/dots-core/src/approvals.rs crates/dots-core/src/bin/fake-claude.rs crates/dots-core/src/engine/claude/args.rs crates/dots-core/src/engine/claude/mod.rs crates/dots-core/src/engine/claude/parse.rs crates/dots-core/src/engine/local/mcp_tools.rs crates/dots-core/src/engine/mod.rs crates/dots-core/src/runner.rs crates/dots-core/src/server/gateway.rs crates/dots-core/src/server/mcp.rs crates/dots-core/src/server/mod.rs crates/dots-core/src/store/approvals.rs crates/dots-core/src/store/runs.rs crates/dots-core/tests/approvals_test.rs crates/dots-core/tests/claude_cli_test.rs crates/dots-core/tests/claude_gateway_test.rs crates/dots-core/tests/engine_prompt_test.rs crates/dots-core/tests/local_engine_test.rs crates/dots-core/tests/resume_test.rs crates/dots-core/tests/runner_test.rs crates/dots-core/tests/store_runs_test.rs docs/superpowers/specs/2026-10-08-dots-design.md
git commit -m "feat(core): reach a Claude run's MCP servers through a checking gateway and pass its plugin folders"
```

### Task 8: Serve the Library through ten commands, and tell the UI when it changes

The app exposes the Library (spec section 9) as ten commands over `Runtime::library()`. Three of them make Dots run or store a program that the web UI names: probing a stdio server, adding a Library item and changing one. They ask first in a native dialog, as the `claude_path` setting does: a stdio server whose command, arguments, folder or environment keys are new or changed ("Dots will run: … Run it?", keys without values), a plugin that is new or moved and starts stdio servers ("This plugin starts: …"), and a plugin whose hooks go from off to on, in one dialog with its servers when both apply. A refusal answers `cancelled` before the core is called. `LibraryUpdated` reaches the UI through the existing event bridge; it does not notify, and the UI marks the Library, the Inbox, the dots and the overview stale. The contract gets a sample of every new shape, and the UI its zod types, commands and query keys. The main spec's section 12 follows.

**Files:**
- Modify: `apps/dots-app/src/commands.rs`, `apps/dots-app/src/confirm.rs`, `apps/dots-app/src/contract.rs`, `apps/dots-app/src/events.rs`, `apps/dots-app/src/lib.rs`, `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/api/commands.ts`, `ui/src/api/types.ts`, `ui/src/lib/query.ts`, `ui/src/lib/runtime-events.ts`
- Regenerate: `ui/src/api/__fixtures__/contract.json`
- Test: `ui/src/api/commands.test.ts`, `ui/src/api/contract.test.ts`, `ui/src/lib/runtime-events.test.ts`

**Interfaces:**
- Consumes: Task 5's `LibraryService` and its views, Task 4's `read_plugin`; the app's `AppState`, `Res`, `CmdError::cancelled`, `confirm::ask` (which takes the answer from `DOTS_CONFIRM=yes` in debug builds only), `command_list!` and the contract's samples.
- Produces:
  - Commands (`Res<T>`; the UI sends camelCase arguments): `library_list() -> Vec<LibraryEntry>`; `library_add(name: String, config: LibraryConfig, accept: Option<String>) -> LibraryItem`; `library_update(id: String, config: LibraryConfig) -> LibraryItem`; `library_remove(id: String, confirm: bool) -> ()`; `library_probe(config: McpServerConfig) -> McpProbe`; `library_check(id: String) -> McpCatalog`; `library_accept(id: String, fingerprint: String) -> McpCatalog`; `library_scan_skills(path: String) -> Vec<ScannedSkill>`; `installed_claude_plugins() -> Vec<InstalledPlugin>` and `environment_variables(names: Vec<String>) -> Vec<EnvVariable>` (both synchronous). The add, update and probe commands take the `AppHandle` for the dialog.
  - In `confirm.rs`: `pub fn library_confirmation(stored: Option<&LibraryConfig>, next: &LibraryConfig, plugin_starts: &[String]) -> Option<String>` (pure: the question, or `None`); `probe_confirmed(rt: &Runtime, config: McpServerConfig, ask: F) -> Result<McpProbe, CmdError>`, `add_confirmed(rt, name: &str, config: LibraryConfig, accept: Option<&str>, ask: F) -> Result<LibraryItem, CmdError>`, `update_confirmed(rt, id: &str, config: LibraryConfig, ask: F) -> Result<LibraryItem, CmdError>` with `F: FnOnce(String) -> Fut, Fut: Future<Output = bool>`; a refusal is `CmdError::cancelled("declined: nothing was run or saved")`.
  - Contract: samples `LibraryConfig` (stdio, http, skill, plugin), `McpServerConfig`, `LibraryItem`, `LibraryEntry` (a changed server with a missing variable, a plugin's server, a skill with a problem, a plugin with every part), `McpCatalog`, `McpProbe`, `ScannedSkill`, `InstalledPlugin`, `EnvVariable`; `RuntimeEvent` `LibraryUpdated`; `Enums.LibraryKind`, `Enums.CatalogStatus`; `Commands` gains the ten names.
  - UI `@/api/types`: `LibraryKind`, `CatalogStatus`, `McpServerConfig`, `SkillConfig`, `PluginConfig`, `LibraryConfig`, `LibraryItem` and `LibraryEntry` (one strict object per kind, `kind` and `config` beside the item's fields), `McpTool` (not strict: the server's own object), `McpCatalog`, `SkillDetail`, `PluginDetail`, `McpProbe`, `ScannedSkill`, `InstalledPlugin`, `EnvVariable`, and the `library_updated` runtime event.
  - UI `@/api/commands`: `libraryList()`, `libraryAdd(name, config, accept: string | null)`, `libraryUpdate(id, config)`, `libraryRemove(id, confirm)`, `libraryProbe(config)`, `libraryCheck(id)`, `libraryAccept(id, fingerprint)`, `libraryScanSkills(path)`, `installedClaudePlugins()`, `environmentVariables(names)`. `@/lib/query` keys `library`, `installedPlugins`, `environmentVariables(names)`. `applyRuntimeEvent` invalidates `keys.library`, `keys.inbox`, `keys.dots`, `["dot"]` and `keys.overview` on `library_updated`.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/api/commands.test.ts`, replace:**

```ts

import fixture from "./__fixtures__/contract.json";

// A plain stub, not vi.fn: the mock's result tracking can report a rejection it saw as
```

**with:**

```ts

import fixture from "./__fixtures__/contract.json";
import * as T from "./types";

// A plain stub, not vi.fn: the mock's result tracking can report a rejection it saw as
```

**In `ui/src/api/commands.test.ts`, replace:**

```ts
    expect(await commands.dismissScheduleAttention("d1")).toBeNull();
    expect(calls).toEqual([["dismiss_schedule_attention", { dotId: "d1" }]]);
  });

```

**with:**

```ts
    expect(await commands.dismissScheduleAttention("d1")).toBeNull();
    expect(calls).toEqual([["dismiss_schedule_attention", { dotId: "d1" }]]);
  });

  it("sends the Library's arguments as the core names them", async () => {
    const config = T.LibraryConfig.parse(fixture.LibraryConfig[0]);
    const server = T.McpServerConfig.parse(fixture.McpServerConfig[0]);
    reply = async () => fixture.LibraryEntry;
    expect(await commands.libraryList()).toEqual(fixture.LibraryEntry);
    reply = async () => fixture.LibraryItem[0];
    await commands.libraryAdd("github", config, "fp1");
    await commands.libraryAdd("github", config, null);
    await commands.libraryUpdate("i1", config);
    reply = async () => null;
    expect(await commands.libraryRemove("i1", true)).toBeNull();
    reply = async () => fixture.McpProbe;
    expect(await commands.libraryProbe(server)).toEqual(fixture.McpProbe);
    reply = async () => fixture.McpCatalog;
    expect(await commands.libraryCheck("i1")).toEqual(fixture.McpCatalog);
    expect(await commands.libraryAccept("i1", "fp2")).toEqual(fixture.McpCatalog);
    reply = async () => fixture.ScannedSkill;
    expect(await commands.libraryScanSkills("C:/skills")).toEqual(fixture.ScannedSkill);
    reply = async () => fixture.InstalledPlugin;
    expect(await commands.installedClaudePlugins()).toEqual(fixture.InstalledPlugin);
    reply = async () => fixture.EnvVariable;
    expect(await commands.environmentVariables(["GITHUB_TOKEN"])).toEqual(fixture.EnvVariable);
    expect(calls).toEqual([
      ["library_list", undefined],
      ["library_add", { name: "github", config, accept: "fp1" }],
      ["library_add", { name: "github", config, accept: null }],
      ["library_update", { id: "i1", config }],
      ["library_remove", { id: "i1", confirm: true }],
      ["library_probe", { config: server }],
      ["library_check", { id: "i1" }],
      ["library_accept", { id: "i1", fingerprint: "fp2" }],
      ["library_scan_skills", { path: "C:/skills" }],
      ["installed_claude_plugins", undefined],
      ["environment_variables", { names: ["GITHUB_TOKEN"] }],
    ]);
  });

  it("refuses to remove a used Library item without a yes, with the dots named", async () => {
    reply = async () => {
      throw {
        kind: "conflict",
        message: 'the skill "pdf" is used by audit; removing it takes it out of them',
      };
    };
    const { kind, message } = await failure(commands.libraryRemove("i1", false));
    expect(kind).toBe("conflict");
    expect(message).toContain("audit");
  });

  it("refuses a Library answer in a shape it does not know", async () => {
    reply = async () => [{ ...fixture.LibraryEntry[0], extra: 1 }];
    expect((await failure(commands.libraryList())).kind).toBe("internal");
    reply = async () => fixture.EnvVariable.map((v) => ({ ...v, value: "secret" }));
    expect((await failure(commands.environmentVariables(["A"]))).kind).toBe("internal");
  });

```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  DotOverview: T.DotOverview,
  Inbox: T.Inbox,
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
```

**with:**

```ts
  DotOverview: T.DotOverview,
  Inbox: T.Inbox,
  LibraryConfig: T.LibraryConfig.array(),
  McpServerConfig: T.McpServerConfig.array(),
  LibraryItem: T.LibraryItem.array(),
  LibraryEntry: T.LibraryEntry.array(),
  McpCatalog: T.McpCatalog,
  McpProbe: T.McpProbe,
  ScannedSkill: T.ScannedSkill.array(),
  InstalledPlugin: T.InstalledPlugin.array(),
  EnvVariable: T.EnvVariable.array(),
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  ScheduleStatus: T.ScheduleStatus,
  CmdErrorKind: T.CmdErrorKind,
};

```

**with:**

```ts
  ScheduleStatus: T.ScheduleStatus,
  CmdErrorKind: T.CmdErrorKind,
  LibraryKind: T.LibraryKind,
  CatalogStatus: T.CatalogStatus,
};

```

**In `ui/src/api/contract.test.ts`, replace:**

```ts
  });

  it("has a sample of every schedule kind and bound", () => {
    const kinds = T.ScheduleRule.options.map((o) => o.shape.kind.value);
```

**with:**

```ts
  });

  it("has a sample of every Library kind and server transport", () => {
    const kinds = T.LibraryConfig.options.map((o) => o.shape.kind.value);
    expect([...new Set(fixture.LibraryConfig.map((c) => c.kind))].sort()).toEqual(
      [...kinds].sort(),
    );
    expect([...new Set(fixture.LibraryEntry.map((e) => e.kind))].sort()).toEqual(
      [...kinds].sort(),
    );
    const transports = T.McpServerConfig.options.map((o) => o.shape.transport.value);
    expect(fixture.McpServerConfig.map((c) => c.transport).sort()).toEqual(
      [...transports].sort(),
    );
  });

  it("has a catalog sample of every status", () => {
    const statuses = fixture.LibraryEntry.map((e) => e.catalog?.status);
    expect(statuses).toEqual(expect.arrayContaining(["accepted", "changed"]));
  });

  it("has a sample of every schedule kind and bound", () => {
    const kinds = T.ScheduleRule.options.map((o) => o.shape.kind.value);
```

**In `ui/src/lib/runtime-events.test.ts`, replace:**

```ts

    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
```

**with:**

```ts

    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("marks the Library, the inbox and what shows a dot's uses stale on a Library update", () => {
    const qc = createQueryClient();
    // Removing an item takes it out of the dots that use it; a changed tool list is an Inbox
    // group.
    const lists = [keys.library, keys.inbox, keys.dots, keys.dot(sample.dot.id), keys.overview];
    const untouched = [keys.settings, keys.runs(sample.dot.id), keys.installedPlugins];
    for (const key of [...lists, ...untouched]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, { type: "library_updated", item_id: "01J0SRV" });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
```

- [ ] **Step 2: Run them to verify they fail**

```bash
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/commands.test.ts src/api/contract.test.ts src/lib/runtime-events.test.ts 2>&1 | grep -E "Tests |Test Files|× |FAIL ")
```

Expected: `Test Files  3 failed (3)`, `Tests  4 failed | 29 passed (33)`: `contract.test.ts` fails as a file (`T.LibraryConfig` does not exist), and "sends the Library's arguments as the core names them", "refuses to remove a used Library item without a yes, with the dots named", "refuses a Library answer in a shape it does not know" and "marks the Library, the inbox and what shows a dot's uses stale on a Library update" fail. The app's own tests (`confirm.rs`, `events.rs`) are in the files they test and come with the next step.

- [ ] **Step 3: Implement**

The app first, then the UI files. `types.ts` is checked against the contract fixture, which the next step regenerates.

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
use std::future::Future;

use dots_core::model::SettingsSaved;
use dots_core::settings::Settings;
```

**with:**

```rust
use std::future::Future;

use dots_core::library::{LibraryConfig, LibraryItem, McpServerConfig};
use dots_core::library_service::McpProbe;
use dots_core::model::SettingsSaved;
use dots_core::settings::Settings;
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
    }
    Ok(rt.update_settings(settings).await?)
}

```

**with:**

```rust
    }
    Ok(rt.update_settings(settings).await?)
}

/// The question to ask before Dots runs or stores what a Library `config` names, or `None`
/// when none is needed. `stored` is the item's config now (`None` for a new item or a probe).
/// - A stdio MCP server whose `command`, `args`, `cwd` or `env` differ from the stored ones (a
///   new server differs): Dots would start that program. `env` is compared as stored, the
///   `${VAR}` text, and the question names its keys, never values.
/// - A plugin that starts stdio servers: `plugin_starts` holds their command lines, which the
///   caller read from the plugin's `.mcp.json` when the plugin is new or its path changed (and
///   left empty otherwise). It is combined with
/// - a plugin whose `run_hooks` goes from off to on (a new plugin counts as off): its hooks run
///   commands without approval.
///
/// Anything else, a `headers` change included, needs none. This function reads no disk.
pub fn library_confirmation(
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    plugin_starts: &[String],
) -> Option<String> {
    match next {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command,
            args,
            cwd,
            env,
        }) => {
            if let Some(LibraryConfig::McpServer(McpServerConfig::Stdio {
                command: c,
                args: a,
                cwd: w,
                env: e,
            })) = stored
            {
                if (c, a, w, e) == (command, args, cwd, env) {
                    return None;
                }
            }
            let place = cwd
                .as_deref()
                .map(|d| format!(" in {d}"))
                .unwrap_or_default();
            let keys = if env.is_empty() {
                String::new()
            } else {
                let keys: Vec<&str> = env.keys().map(String::as_str).collect();
                format!(" with env {}", keys.join(", "))
            };
            Some(format!(
                "Dots will run: {}{place}{keys}. Run it?\n\nAllow this only for a program you trust.",
                command_line(command, args)
            ))
        }
        LibraryConfig::Plugin(p) => {
            let was_on = matches!(stored, Some(LibraryConfig::Plugin(old)) if old.run_hooks);
            let hooks = p.run_hooks && !was_on;
            match (plugin_starts.is_empty(), hooks) {
                (true, false) => None,
                (true, true) => Some(
                    "This plugin's hooks run commands on their own, without approval, and can \
                     approve tool calls. Run them?"
                        .to_string(),
                ),
                (false, hooks) => {
                    let mut text = format!("This plugin starts: {}.", plugin_starts.join("; "));
                    if hooks {
                        text.push_str(
                            " Its hooks also run commands on their own, without approval, and \
                             can approve tool calls.",
                        );
                    }
                    text.push_str(" Add it?");
                    Some(text)
                }
            }
        }
        _ => None,
    }
}

fn command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command lines of the stdio servers in the plugin folder `config` names, for the question
/// of `library_confirmation`. Only when the plugin is new (`stored` is `None`) or its path
/// changed; a folder that cannot be read gives none, and the core then refuses it.
fn plugin_starts(stored: Option<&LibraryConfig>, config: &LibraryConfig) -> Vec<String> {
    let LibraryConfig::Plugin(next) = config else {
        return Vec::new();
    };
    if matches!(stored, Some(LibraryConfig::Plugin(old)) if old.path == next.path) {
        return Vec::new();
    }
    let Ok(info) = dots_core::kit::read_plugin(std::path::Path::new(&next.path)) else {
        return Vec::new();
    };
    info.servers
        .iter()
        .filter_map(|s| match &s.config {
            McpServerConfig::Stdio { command, args, .. } => Some(command_line(command, args)),
            McpServerConfig::Http { .. } => None,
        })
        .collect()
}

/// Asks `library_confirmation`'s question, if there is one. A refusal is `cancelled`, and the
/// caller then starts and stores nothing.
async fn confirm_library<F, Fut>(
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    ask: F,
) -> Result<(), CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    if let Some(question) = library_confirmation(stored, next, &plugin_starts(stored, next)) {
        if !ask(question).await {
            return Err(CmdError::cancelled("declined: nothing was run or saved"));
        }
    }
    Ok(())
}

/// `library_probe`, after the user's yes when the config is a stdio server.
pub async fn probe_confirmed<F, Fut>(
    rt: &Runtime,
    config: McpServerConfig,
    ask: F,
) -> Result<McpProbe, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let next = LibraryConfig::McpServer(config);
    confirm_library(None, &next, ask).await?;
    let LibraryConfig::McpServer(config) = next else {
        unreachable!("wrapped above")
    };
    Ok(rt.library().probe(&config).await?)
}

/// `library_add`, after the user's yes when it adds a stdio server or a plugin with hooks on.
pub async fn add_confirmed<F, Fut>(
    rt: &Runtime,
    name: &str,
    config: LibraryConfig,
    accept: Option<&str>,
    ask: F,
) -> Result<LibraryItem, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    confirm_library(None, &config, ask).await?;
    Ok(rt.library().add(name, config, accept).await?)
}

/// `library_update`, after the user's yes when the stored config does not already hold what is
/// asked for: another stdio command line, or hooks switched on.
pub async fn update_confirmed<F, Fut>(
    rt: &Runtime,
    id: &str,
    config: LibraryConfig,
    ask: F,
) -> Result<LibraryItem, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let stored = rt
        .library()
        .list()
        .await?
        .into_iter()
        .find(|e| e.item.id == id)
        .map(|e| e.item.config);
    // An id the Library does not have is the core's `not_found`, before any question.
    if stored.is_some() {
        confirm_library(stored.as_ref(), &config, ask).await?;
    }
    Ok(rt.library().update(id, config).await?)
}

```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust
    }

    #[test]
    fn the_message_names_the_program() {
```

**with:**

```rust
    }

    fn stdio(command: &str, args: &[&str], cwd: Option<&str>) -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: cwd.map(String::from),
            env: Default::default(),
        })
    }

    fn plugin(run_hooks: bool) -> LibraryConfig {
        LibraryConfig::Plugin(dots_core::library::PluginConfig {
            path: "C:/p".into(),
            installed_id: None,
            run_hooks,
        })
    }

    fn http() -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Http {
            url: "http://127.0.0.1:1/mcp".into(),
            headers: Default::default(),
        })
    }

    #[test]
    fn a_stdio_command_line_needs_a_yes_unless_it_is_the_stored_one() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let next = stdio("npx", &["-y", "srv"], Some("C:/d"));
        let asked = q(None, &next).unwrap();
        assert!(asked.starts_with("Dots will run: npx -y srv in C:/d. Run it?"));
        // Without a folder the question names none.
        assert!(q(None, &stdio("srv", &[], None))
            .unwrap()
            .starts_with("Dots will run: srv. Run it?"));
        // The same command line, args, folder and env: no question.
        assert_eq!(q(Some(&next), &next), None);
        // Any difference in the command, an argument or the folder asks.
        for other in [
            stdio("node", &["-y", "srv"], Some("C:/d")),
            stdio("npx", &["-y", "evil"], Some("C:/d")),
            stdio("npx", &["-y", "srv"], Some("C:/e")),
            stdio("npx", &["-y", "srv"], None),
        ] {
            assert!(q(Some(&next), &other).is_some(), "{other:?}");
        }
        // Stored as another kind or transport: new for this purpose.
        assert!(q(Some(&http()), &next).is_some());
        assert!(q(Some(&plugin(false)), &next).is_some());
    }

    fn stdio_env(env: &[(&str, &str)]) -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: "npx".into(),
            args: vec!["srv".into()],
            cwd: None,
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        })
    }

    #[test]
    fn a_change_of_env_asks_and_names_the_keys_but_not_the_values() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let stored = stdio_env(&[("A", "${A}")]);
        assert_eq!(q(Some(&stored), &stdio_env(&[("A", "${A}")])), None);
        // A value (the stored text), a key added, a key removed.
        for next in [
            stdio_env(&[("A", "${B}")]),
            stdio_env(&[("A", "${A}"), ("NODE_OPTIONS", "--require x")]),
            stdio_env(&[]),
        ] {
            assert!(q(Some(&stored), &next).is_some(), "{next:?}");
        }
        let asked = q(
            Some(&stored),
            &stdio_env(&[("PATH", "C:/evil"), ("NODE_OPTIONS", "--require x")]),
        )
        .unwrap();
        assert!(
            asked.starts_with("Dots will run: npx srv with env NODE_OPTIONS, PATH. Run it?"),
            "{asked}"
        );
        assert!(!asked.contains("evil") && !asked.contains("--require"));
        // With a folder the keys follow it.
        let LibraryConfig::McpServer(McpServerConfig::Stdio {
            command, args, env, ..
        }) = stdio_env(&[("A", "${A}")])
        else {
            unreachable!()
        };
        let in_dir = LibraryConfig::McpServer(McpServerConfig::Stdio {
            command,
            args,
            cwd: Some("C:/d".into()),
            env,
        });
        assert!(q(None, &in_dir)
            .unwrap()
            .starts_with("Dots will run: npx srv in C:/d with env A. Run it?"));
    }

    #[test]
    fn a_plugin_that_starts_servers_asks_with_their_command_lines_and_the_hooks_in_one_dialog() {
        let starts = vec!["node a.js --x".to_string(), "uvx b".to_string()];
        let q = library_confirmation;
        // Servers only.
        assert_eq!(
            q(None, &plugin(false), &starts).unwrap(),
            "This plugin starts: node a.js --x; uvx b. Add it?"
        );
        // Servers and hooks: one question.
        assert_eq!(
            q(None, &plugin(true), &starts).unwrap(),
            "This plugin starts: node a.js --x; uvx b. Its hooks also run commands on their own, \
             without approval, and can approve tool calls. Add it?"
        );
        // A plugin with hooks already on, and servers to start, asks about the servers only.
        assert_eq!(
            q(Some(&plugin(true)), &plugin(true), &starts).unwrap(),
            "This plugin starts: node a.js --x; uvx b. Add it?"
        );
        // Nothing to start and nothing switched on: no question.
        assert_eq!(q(None, &plugin(false), &[]), None);
        assert_eq!(q(Some(&plugin(true)), &plugin(true), &[]), None);
    }

    #[test]
    fn the_plugin_servers_are_read_for_a_new_plugin_or_a_new_path_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"a":{"command":"node","args":["a.js"]},"b":{"type":"http","url":"http://127.0.0.1:1/mcp"}}}"#,
        )
        .unwrap();
        let at = |path: &std::path::Path| {
            LibraryConfig::Plugin(dots_core::library::PluginConfig {
                path: path.to_string_lossy().into_owned(),
                installed_id: None,
                run_hooks: false,
            })
        };
        let config = at(dir.path());
        // New: the stdio server's command line; the HTTP one starts nothing.
        assert_eq!(plugin_starts(None, &config), ["node a.js"]);
        // Same path: not read, not asked again.
        assert!(plugin_starts(Some(&config), &config).is_empty());
        // Another path: read again.
        let elsewhere = at(std::path::Path::new("C:/no/such/plugin"));
        assert_eq!(plugin_starts(Some(&elsewhere), &config), ["node a.js"]);
        // Unreadable: nothing here; the core refuses the folder.
        assert!(plugin_starts(None, &elsewhere).is_empty());
        // Not a plugin.
        assert!(plugin_starts(None, &stdio("x", &[], None)).is_empty());
    }

    #[test]
    fn a_http_server_and_a_skill_need_no_question() {
        let skill = LibraryConfig::Skill(dots_core::library::SkillConfig {
            path: "C:/s".into(),
        });
        assert_eq!(library_confirmation(None, &http(), &[]), None);
        assert_eq!(library_confirmation(None, &skill, &[]), None);
    }

    #[test]
    fn hooks_ask_only_when_they_go_from_off_to_on() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let asked = q(Some(&plugin(false)), &plugin(true)).unwrap();
        assert_eq!(
            asked,
            "This plugin's hooks run commands on their own, without approval, and can approve \
             tool calls. Run them?"
        );
        // A new plugin with hooks on switches them on from nothing.
        assert!(q(None, &plugin(true)).is_some());
        assert_eq!(q(Some(&plugin(true)), &plugin(true)), None);
        assert_eq!(q(Some(&plugin(true)), &plugin(false)), None);
        assert_eq!(q(Some(&plugin(false)), &plugin(false)), None);
        assert_eq!(q(None, &plugin(false)), None);
    }

    #[test]
    fn the_message_names_the_program() {
```

**In `apps/dots-app/src/confirm.rs`, replace:**

```rust

    const PROGRAM: &str = "C:/tools/claude.exe";

    /// A runtime on a temporary data folder: its server on a free port, and no Claude CLI, so
```

**with:**

```rust

    const PROGRAM: &str = "C:/tools/claude.exe";

    #[tokio::test]
    async fn a_declined_server_is_neither_probed_nor_stored_and_an_allowed_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        // A program that does not exist, and no accept: nothing is ever started.
        let config = stdio("dots-no-such-mcp-server", &["a"], None);
        let LibraryConfig::McpServer(server) = config.clone() else {
            unreachable!()
        };
        let mut asked = Vec::new();
        let err = probe_confirmed(&rt, server, |q| {
            asked.push(q);
            std::future::ready(false)
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert_eq!(asked.len(), 1);
        assert!(asked[0].starts_with("Dots will run: dots-no-such-mcp-server a. Run it?"));

        let no = |_| std::future::ready(false);
        let yes = |_| std::future::ready(true);
        let err = add_confirmed(&rt, "srv", config.clone(), None, no)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(rt.library().list().await.unwrap().is_empty());

        let item = add_confirmed(&rt, "srv", config.clone(), None, yes)
            .await
            .unwrap();

        // The same command line again: no question (a refusal would stop it). Another one
        // asks, and a refusal leaves the stored config as it was.
        update_confirmed(&rt, &item.id, config.clone(), no)
            .await
            .unwrap();
        let other = stdio("dots-no-such-mcp-server", &["b"], None);
        let err = update_confirmed(&rt, &item.id, other.clone(), no)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        let stored = rt.library().list().await.unwrap().remove(0).item.config;
        assert_eq!(stored, config);
        update_confirmed(&rt, &item.id, other.clone(), yes)
            .await
            .unwrap();
        let stored = rt.library().list().await.unwrap().remove(0).item.config;
        assert_eq!(stored, other);

        // A server over HTTP is not a program to run.
        add_confirmed(&rt, "web", http(), None, no).await.unwrap();
        rt.shutdown();
    }

    /// A runtime on a temporary data folder: its server on a free port, and no Claude CLI, so
```

**In `apps/dots-app/src/commands.rs`, replace:**

```rust
//! (`DotSpec::validate`, then the runner requires an existing folder) and the `claude_path`
//! setting (`Settings::validate`; the runtime runs that program as the Claude CLI from the
//! next start). The UI is first-party code under a `default-src 'self'` policy.

use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, Inbox, Run, RunEventRecord, SettingsSaved, Status,
```

**with:**

```rust
//! (`DotSpec::validate`, then the runner requires an existing folder) and the `claude_path`
//! setting (`Settings::validate`; the runtime runs that program as the Claude CLI from the
//! next start). The Library commands take more: a skill or plugin folder, a folder to scan for
//! skills, and the command of an MCP server, which `library_probe` starts and a dot's runs
//! start. Like `claude_path`, a new command line or env (and a plugin's servers and hooks) is confirmed in
//! a native dialog from Rust before it is run or stored (`confirm.rs`). The UI is first-party
//! code under a `default-src 'self'` policy.

use dots_core::kit::InstalledPlugin;
use dots_core::library::{LibraryConfig, LibraryItem, McpCatalog, McpServerConfig};
use dots_core::library_service::{EnvVariable, LibraryEntry, McpProbe, ScannedSkill};
use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, Inbox, Run, RunEventRecord, SettingsSaved, Status,
```

**Append to `apps/dots-app/src/commands.rs`:**

```rust

/// Every Library item with its users, catalog and, for a skill or plugin, what its folder holds.
#[tauri::command]
pub async fn library_list(state: State<'_, AppState>) -> Res<Vec<LibraryEntry>> {
    Ok(state.rt()?.library().list().await?)
}

/// `accept`: the fingerprint `library_probe` answered. The tool list the server gives now is
/// accepted only when it has that fingerprint; without it, a server is saved unchecked.
#[tauri::command]
pub async fn library_add(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    config: LibraryConfig,
    accept: Option<String>,
) -> Res<LibraryItem> {
    let rt = state.rt()?;
    confirm::add_confirmed(&rt, &name, config, accept.as_deref(), |message| {
        confirm::ask(&app, "Run this program?", message)
    })
    .await
}

#[tauri::command]
pub async fn library_update(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    config: LibraryConfig,
) -> Res<LibraryItem> {
    let rt = state.rt()?;
    confirm::update_confirmed(&rt, &id, config, |message| {
        confirm::ask(&app, "Run this program?", message)
    })
    .await
}

/// Refused with kind `conflict`, naming the dots, while dots use the item and `confirm` is not set.
#[tauri::command]
pub async fn library_remove(state: State<'_, AppState>, id: String, confirm: bool) -> Res<()> {
    Ok(state.rt()?.library().remove(&id, confirm).await?)
}

/// Starts the server, lists its tools and stops it again; nothing is saved. A stdio server needs the
/// person's yes in a native dialog first (`confirm::library_confirmation`), as do `library_add`
/// and `library_update` for a stdio command line or env they do not already hold, for the
/// stdio servers of a new plugin (or a new plugin path) and for plugin hooks switched on; a refusal answers `cancelled` and nothing is started or stored.
#[tauri::command]
pub async fn library_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    config: McpServerConfig,
) -> Res<McpProbe> {
    let rt = state.rt()?;
    confirm::probe_confirmed(&rt, config, |message| {
        confirm::ask(&app, "Run this program?", message)
    })
    .await
}

#[tauri::command]
pub async fn library_check(state: State<'_, AppState>, id: String) -> Res<McpCatalog> {
    Ok(state.rt()?.library().check(&id).await?)
}

/// Refused with kind `conflict` when the fingerprint is no longer the one last seen.
#[tauri::command]
pub async fn library_accept(
    state: State<'_, AppState>,
    id: String,
    fingerprint: String,
) -> Res<McpCatalog> {
    Ok(state.rt()?.library().accept(&id, &fingerprint).await?)
}

#[tauri::command]
pub async fn library_scan_skills(
    state: State<'_, AppState>,
    path: String,
) -> Res<Vec<ScannedSkill>> {
    Ok(state.rt()?.library().scan_skills(&path).await?)
}

#[tauri::command]
pub fn installed_claude_plugins(state: State<'_, AppState>) -> Res<Vec<InstalledPlugin>> {
    Ok(state.rt()?.library().installed_plugins())
}

/// Which of the names Dots' environment sets. Never a value.
#[tauri::command]
pub fn environment_variables(
    state: State<'_, AppState>,
    names: Vec<String>,
) -> Res<Vec<EnvVariable>> {
    Ok(state.rt()?.library().environment_variables(&names))
}
```

**In `apps/dots-app/src/lib.rs`, replace:**

```rust
    mark_reviewed,
    open_run_folder,
];

```

**with:**

```rust
    mark_reviewed,
    open_run_folder,
    library_list,
    library_add,
    library_update,
    library_remove,
    library_probe,
    library_check,
    library_accept,
    library_scan_skills,
    installed_claude_plugins,
    environment_variables,
];

```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
use dots_core::engine::EngineEvent;
use dots_core::events::RuntimeEvent;
use dots_core::library::Uses;
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
```

**with:**

```rust
use dots_core::engine::EngineEvent;
use dots_core::events::RuntimeEvent;
use dots_core::kit::InstalledPlugin;
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig,
    PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::{
    EnvVariable, LibraryEntry, McpProbe, PluginDetail, ScannedSkill, SkillDetail,
};
use dots_core::mcp::McpTool;
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
        spec: spec(),
    }
}

```

**with:**

```rust
        spec: spec(),
    }
}

fn stdio_server() -> McpServerConfig {
    McpServerConfig::Stdio {
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
        cwd: None,
        env: [("GITHUB_TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    }
}

fn http_server() -> McpServerConfig {
    McpServerConfig::Http {
        url: "https://mcp.example.com/mcp".into(),
        headers: [(
            "Authorization".to_string(),
            "Bearer ${EXAMPLE_TOKEN}".to_string(),
        )]
        .into(),
    }
}

fn skill_config() -> LibraryConfig {
    LibraryConfig::Skill(SkillConfig {
        path: "C:/skills/pdf".into(),
    })
}

fn plugin_config() -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: "C:/plugins/pdf-tools".into(),
        installed_id: Some("pdf-tools@market".into()),
        run_hooks: false,
    })
}

/// A config of every kind and transport, as `library_add` takes them.
fn library_configs() -> Vec<LibraryConfig> {
    vec![
        LibraryConfig::McpServer(stdio_server()),
        LibraryConfig::McpServer(http_server()),
        skill_config(),
        plugin_config(),
    ]
}

fn library_item(id: &str, name: &str, config: LibraryConfig) -> LibraryItem {
    LibraryItem {
        id: id.into(),
        name: name.into(),
        config,
        owner_id: None,
        created_at: TS.into(),
        updated_at: TS.into(),
    }
}

fn tool() -> McpTool {
    McpTool {
        name: "search_issues".into(),
        title: Some("Search issues".into()),
        description: Some("Search the issues of a repository.".into()),
        input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        annotations: Some(json!({"readOnlyHint": true})),
    }
}

fn catalog(status: CatalogStatus) -> McpCatalog {
    let tools = to([tool()]);
    McpCatalog {
        server_id: "01J0SRV".into(),
        status,
        accepted_fingerprint: Some("fp-accepted".into()),
        accepted_tools: Some(tools.clone()),
        seen_fingerprint: Some("fp-seen".into()),
        seen_tools: Some(tools),
        checked_at: Some(TS.into()),
        error: None,
    }
}

/// An entry of every kind: a server whose list changed, a plugin's own server, a skill that
/// could not be read, and a plugin with every part.
fn library_entries() -> Vec<LibraryEntry> {
    let users = vec![DotRef {
        id: "01J0DOT".into(),
        name: "nightly-tests".into(),
    }];
    let bare = |item: LibraryItem, used_by: Vec<DotRef>| LibraryEntry {
        item,
        used_by,
        catalog: None,
        missing_variables: vec![],
        skill: None,
        plugin: None,
        problem: None,
    };
    let github = library_item(
        "01J0SRV",
        "github",
        LibraryConfig::McpServer(stdio_server()),
    );
    let mut owned = library_item(
        "01J0SRV2",
        "plugin_pdf-tools_render",
        LibraryConfig::McpServer(http_server()),
    );
    owned.owner_id = Some("01J0PLG".into());
    let skill = library_item("01J0SKL", "pdf", skill_config());
    let plugin = library_item("01J0PLG", "pdf-tools", plugin_config());
    vec![
        LibraryEntry {
            catalog: Some(catalog(CatalogStatus::Changed)),
            missing_variables: vec!["GITHUB_TOKEN".into()],
            ..bare(github, users.clone())
        },
        LibraryEntry {
            catalog: Some(catalog(CatalogStatus::Accepted)),
            ..bare(owned, users.clone())
        },
        LibraryEntry {
            skill: Some(SkillDetail {
                description: "Fill and read PDF forms.".into(),
            }),
            problem: Some("SKILL.md not found".into()),
            ..bare(skill, vec![])
        },
        LibraryEntry {
            plugin: Some(PluginDetail {
                version: Some("1.2.0".into()),
                description: Some("PDF tools.".into()),
                skills: vec!["pdf-tools:merge".into()],
                commands: vec!["split".into()],
                agents: vec!["reviewer".into()],
                servers: vec!["plugin_pdf-tools_render".into()],
                has_hooks: true,
                warnings: vec!["agents/broken.md: could not be read".into()],
            }),
            ..bare(plugin, users)
        },
    ]
}

```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            }}),
            to(RuntimeEvent::ScheduleUpdated { dot_id: "01J0DOT".into() }),
            to(RuntimeEvent::ScheduleAttention {
                dot_id: "01J0DOT".into(),
```

**with:**

```rust
            }}),
            to(RuntimeEvent::ScheduleUpdated { dot_id: "01J0DOT".into() }),
            to(RuntimeEvent::LibraryUpdated { item_id: "01J0SRV".into() }),
            to(RuntimeEvent::ScheduleAttention {
                dot_id: "01J0DOT".into(),
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            }],
        }),
        "WorktreeDiff": to(WorktreeDiff {
            files: vec![
```

**with:**

```rust
            }],
        }),
        // What `library_add` takes and the Library commands answer: a config of every kind and
        // transport, and entries of every kind (a plugin's own server among them).
        "LibraryConfig": to(library_configs()),
        "McpServerConfig": to([stdio_server(), http_server()]),
        "LibraryItem": to([
            library_item("01J0SRV", "github", LibraryConfig::McpServer(stdio_server())),
            library_item("01J0SKL", "pdf", skill_config()),
        ]),
        "LibraryEntry": to(library_entries()),
        "McpCatalog": to(catalog(CatalogStatus::Changed)),
        "McpProbe": to(McpProbe { tools: vec![tool()], fingerprint: "fp-seen".into() }),
        "ScannedSkill": to([
            ScannedSkill {
                name: "pdf".into(),
                description: "Fill and read PDF forms.".into(),
                path: "C:/skills/pdf".into(),
                name_error: None,
            },
            ScannedSkill {
                name: "bad name".into(),
                description: String::new(),
                path: "C:/skills/bad".into(),
                name_error: Some("skill name \"bad name\": use A-Z, a-z, 0-9".into()),
            },
        ]),
        "InstalledPlugin": to([InstalledPlugin {
            id: "pdf-tools@market".into(),
            name: "pdf-tools".into(),
            marketplace: Some("market".into()),
            scope: Some("user".into()),
            install_path: "C:/Users/u/.claude/plugins/cache/market/pdf-tools/1.2.0".into(),
            version: Some("1.2.0".into()),
            exists: true,
        }]),
        "EnvVariable": to([
            EnvVariable { name: "GITHUB_TOKEN".into(), set: false },
            EnvVariable { name: "PATH".into(), set: true },
        ]),
        "WorktreeDiff": to(WorktreeDiff {
            files: vec![
```

**In `apps/dots-app/src/contract.rs`, replace:**

```rust
            "ScheduleStatus": ScheduleStatus::ALL,
            "CmdErrorKind": CmdErrorKind::ALL,
        },
        "Limits": {
```

**with:**

```rust
            "ScheduleStatus": ScheduleStatus::ALL,
            "CmdErrorKind": CmdErrorKind::ALL,
            "LibraryKind": LibraryKind::ALL,
            "CatalogStatus": CatalogStatus::ALL,
        },
        "Limits": {
```

**In `apps/dots-app/src/events.rs`, replace:**

```rust

    #[test]
    fn a_failed_scheduled_run_is_quiet_and_its_attention_notifies() {
        let mut n = Notifier::default();
```

**with:**

```rust

    #[test]
    fn a_library_update_does_not_notify_but_may_change_the_inbox() {
        let mut n = Notifier::default();
        let ev = RuntimeEvent::LibraryUpdated {
            item_id: "i".into(),
        };
        assert_eq!(n.decide(&ev), None);
        // A server whose tool list changed waits in the Inbox, and the tray counts it.
        assert!(affects_inbox(&ev));
    }

    #[test]
    fn a_failed_scheduled_run_is_quiet_and_its_attention_notifies() {
        let mut n = Notifier::default();
```

**In `ui/src/api/types.ts`, replace:**

```ts
export const FileStatus = z.enum(["added", "modified", "deleted", "renamed"]);
export const ScheduleStatus = z.enum(["active", "completed", "expired"]);

export const Rule = z.strictObject({ tool: z.string(), action: Action });
```

**with:**

```ts
export const FileStatus = z.enum(["added", "modified", "deleted", "renamed"]);
export const ScheduleStatus = z.enum(["active", "completed", "expired"]);
export const LibraryKind = z.enum(["mcp_server", "skill", "plugin"]);
// Where an MCP server's tool list stands: accepted, changed since, last check failed, or never
// accepted.
export const CatalogStatus = z.enum(["accepted", "changed", "unreachable", "unchecked"]);

export const Rule = z.strictObject({ tool: z.string(), action: Action });
```

**In `ui/src/api/types.ts`, replace:**

```ts
  // The dot's schedule row was made, changed or deleted.
  z.strictObject({ type: z.literal("schedule_updated"), dot_id: z.string() }),
  // The dot's schedule needs attention now (sent once, with a `schedule_updated`).
  z.strictObject({
```

**with:**

```ts
  // The dot's schedule row was made, changed or deleted.
  z.strictObject({ type: z.literal("schedule_updated"), dot_id: z.string() }),
  // A Library item was added, changed, removed or checked (an MCP server's catalog included).
  z.strictObject({ type: z.literal("library_updated"), item_id: z.string() }),
  // The dot's schedule needs attention now (sent once, with a `schedule_updated`).
  z.strictObject({
```

**In `ui/src/api/types.ts`, replace:**

```ts
});

export const FileDiff = z.strictObject({
  path: z.string(),
```

**with:**

```ts
});

// The Library (MCP/skills spec section 2 and 9). `${VAR}` in a server's config stays a
// reference: the core reads the environment when the server starts.
export const McpServerConfig = z.discriminatedUnion("transport", [
  z.strictObject({
    transport: z.literal("stdio"),
    command: z.string(),
    args: z.array(z.string()),
    cwd: nullable(z.string()),
    env: z.record(z.string(), z.string()),
  }),
  z.strictObject({
    transport: z.literal("http"),
    url: z.string(),
    headers: z.record(z.string(), z.string()),
  }),
]);
export const SkillConfig = z.strictObject({ path: z.string() });
export const PluginConfig = z.strictObject({
  path: z.string(),
  // `<plugin>@<marketplace>` when picked from the installed Claude Code plugins.
  installed_id: nullable(z.string()),
  run_hooks: z.boolean(),
});
// An item's config with its kind: what `library_add` and `library_update` take.
export const LibraryConfig = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("mcp_server"), config: McpServerConfig }),
  z.strictObject({ kind: z.literal("skill"), config: SkillConfig }),
  z.strictObject({ kind: z.literal("plugin"), config: PluginConfig }),
]);
const libraryItemShape = {
  id: z.string(),
  name: z.string(),
  // The plugin an MCP server comes from; it goes with the plugin.
  owner_id: nullable(z.string()),
  created_at: z.string(),
  updated_at: z.string(),
};
// One strict object per kind: the item's fields and its config sit side by side in the JSON.
const libraryItemOf = <E extends z.ZodRawShape>(extra: E) =>
  z.discriminatedUnion("kind", [
    z.strictObject({
      kind: z.literal("mcp_server"),
      config: McpServerConfig,
      ...libraryItemShape,
      ...extra,
    }),
    z.strictObject({
      kind: z.literal("skill"),
      config: SkillConfig,
      ...libraryItemShape,
      ...extra,
    }),
    z.strictObject({
      kind: z.literal("plugin"),
      config: PluginConfig,
      ...libraryItemShape,
      ...extra,
    }),
  ]);
export const LibraryItem = libraryItemOf({});
// A tool of `tools/list` as the server sent it (camelCase, optional fields left out).
export const McpTool = z.object({
  name: z.string(),
  title: z.string().optional(),
  description: z.string().optional(),
  inputSchema: z.unknown(),
  annotations: z.unknown().optional(),
});
export const McpCatalog = z.strictObject({
  server_id: z.string(),
  status: CatalogStatus,
  accepted_fingerprint: nullable(z.string()),
  // `tools/list` results (arrays of `McpTool`), kept as JSON.
  accepted_tools: nullable(z.unknown()),
  seen_fingerprint: nullable(z.string()),
  seen_tools: nullable(z.unknown()),
  checked_at: nullable(z.string()),
  error: nullable(z.string()),
});
export const SkillDetail = z.strictObject({ description: z.string() });
export const PluginDetail = z.strictObject({
  version: nullable(z.string()),
  description: nullable(z.string()),
  // Qualified: `<plugin>:<skill>`.
  skills: z.array(z.string()),
  commands: z.array(z.string()),
  agents: z.array(z.string()),
  // The names of its MCP server items.
  servers: z.array(z.string()),
  has_hooks: z.boolean(),
  warnings: z.array(z.string()),
});
export const LibraryEntry = libraryItemOf({
  // The dots that use it; for a plugin's server, the dots that use the plugin.
  used_by: z.array(DotRef),
  catalog: nullable(McpCatalog),
  // The `${VAR}` of a server's config that the environment of Dots does not set.
  missing_variables: z.array(z.string()),
  skill: nullable(SkillDetail),
  plugin: nullable(PluginDetail),
  // Why a skill or plugin folder could not be read.
  problem: nullable(z.string()),
});
export const McpProbe = z.strictObject({ tools: z.array(McpTool), fingerprint: z.string() });
export const ScannedSkill = z.strictObject({
  name: z.string(),
  description: z.string(),
  path: z.string(),
  // Why `name` cannot be a Library name.
  name_error: nullable(z.string()),
});
export const InstalledPlugin = z.strictObject({
  id: z.string(),
  name: z.string(),
  marketplace: nullable(z.string()),
  scope: nullable(z.string()),
  install_path: z.string(),
  version: nullable(z.string()),
  exists: z.boolean(),
});
// Whether the environment of Dots sets a variable. Never its value.
export const EnvVariable = z.strictObject({ name: z.string(), set: z.boolean() });

export const FileDiff = z.strictObject({
  path: z.string(),
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type FileStatus = z.infer<typeof FileStatus>;
export type ScheduleStatus = z.infer<typeof ScheduleStatus>;
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
```

**with:**

```ts
export type FileStatus = z.infer<typeof FileStatus>;
export type ScheduleStatus = z.infer<typeof ScheduleStatus>;
export type LibraryKind = z.infer<typeof LibraryKind>;
export type CatalogStatus = z.infer<typeof CatalogStatus>;
export type Rule = z.infer<typeof Rule>;
export type Policy = z.infer<typeof Policy>;
```

**In `ui/src/api/types.ts`, replace:**

```ts
export type ToolsChanged = z.infer<typeof ToolsChanged>;
export type Inbox = z.infer<typeof Inbox>;
export type FileDiff = z.infer<typeof FileDiff>;
export type WorktreeDiff = z.infer<typeof WorktreeDiff>;
```

**with:**

```ts
export type ToolsChanged = z.infer<typeof ToolsChanged>;
export type Inbox = z.infer<typeof Inbox>;
export type McpServerConfig = z.infer<typeof McpServerConfig>;
export type SkillConfig = z.infer<typeof SkillConfig>;
export type PluginConfig = z.infer<typeof PluginConfig>;
export type LibraryConfig = z.infer<typeof LibraryConfig>;
export type LibraryItem = z.infer<typeof LibraryItem>;
export type McpTool = z.infer<typeof McpTool>;
export type McpCatalog = z.infer<typeof McpCatalog>;
export type SkillDetail = z.infer<typeof SkillDetail>;
export type PluginDetail = z.infer<typeof PluginDetail>;
export type LibraryEntry = z.infer<typeof LibraryEntry>;
export type McpProbe = z.infer<typeof McpProbe>;
export type ScannedSkill = z.infer<typeof ScannedSkill>;
export type InstalledPlugin = z.infer<typeof InstalledPlugin>;
export type EnvVariable = z.infer<typeof EnvVariable>;
export type FileDiff = z.infer<typeof FileDiff>;
export type WorktreeDiff = z.infer<typeof WorktreeDiff>;
```

**In `ui/src/api/commands.ts`, replace:**

```ts

import * as T from "./types";
import type { CmdErrorKind, DotSpec, Preset, Schedule, Settings } from "./types";

/// A command failure. `kind` mirrors the Rust `CmdError`.
```

**with:**

```ts

import * as T from "./types";
import type {
  CmdErrorKind,
  DotSpec,
  LibraryConfig,
  McpServerConfig,
  Preset,
  Schedule,
  Settings,
} from "./types";

/// A command failure. `kind` mirrors the Rust `CmdError`.
```

**In `ui/src/api/commands.ts`, replace:**

```ts
  markReviewed: (runId: string) => call(T.Run, "mark_reviewed", { runId }),
  openRunFolder: (runId: string) => call(nothing, "open_run_folder", { runId }),
};
```

**with:**

```ts
  markReviewed: (runId: string) => call(T.Run, "mark_reviewed", { runId }),
  openRunFolder: (runId: string) => call(nothing, "open_run_folder", { runId }),

  libraryList: () => call(T.LibraryEntry.array(), "library_list"),
  /// `accept`: the fingerprint `libraryProbe` answered. The list the server gives now is accepted
  /// only when it has that fingerprint; with `null` a server is saved unchecked and not started.
  libraryAdd: (name: string, config: LibraryConfig, accept: string | null) =>
    call(T.LibraryItem, "library_add", { name, config, accept }),
  libraryUpdate: (id: string, config: LibraryConfig) =>
    call(T.LibraryItem, "library_update", { id, config }),
  /// Rejects with kind `conflict`, naming the dots, while dots use the item and `confirm` is
  /// false. With `confirm` the item also leaves those dots.
  libraryRemove: (id: string, confirm: boolean) =>
    call(nothing, "library_remove", { id, confirm }),
  /// Starts the server, lists its tools and stops it; nothing is saved.
  libraryProbe: (config: McpServerConfig) => call(T.McpProbe, "library_probe", { config }),
  libraryCheck: (id: string) => call(T.McpCatalog, "library_check", { id }),
  /// Rejects with kind `conflict` when the list moved since `fingerprint` was seen.
  libraryAccept: (id: string, fingerprint: string) =>
    call(T.McpCatalog, "library_accept", { id, fingerprint }),
  libraryScanSkills: (path: string) =>
    call(T.ScannedSkill.array(), "library_scan_skills", { path }),
  installedClaudePlugins: () => call(T.InstalledPlugin.array(), "installed_claude_plugins"),
  environmentVariables: (names: string[]) =>
    call(T.EnvVariable.array(), "environment_variables", { names }),
};
```

**In `ui/src/lib/query.ts`, replace:**

```ts
  overview: ["overview"] as const,
  inbox: ["inbox"] as const,
  dots: ["dots"] as const,
  dot: (id: string) => ["dot", id] as const,
```

**with:**

```ts
  overview: ["overview"] as const,
  inbox: ["inbox"] as const,
  library: ["library"] as const,
  installedPlugins: ["installed-plugins"] as const,
  environmentVariables: (names: string[]) => ["environment-variables", names] as const,
  dots: ["dots"] as const,
  dot: (id: string) => ["dot", id] as const,
```

**In `ui/src/lib/runtime-events.ts`, replace:**

```ts
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
```

**with:**

```ts
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
    case "library_updated":
      // The Library lists, the Inbox group of a changed tool list, and what shows the dots that
      // use the item: removing it takes it out of them, and no dot event says so.
      for (const queryKey of [keys.library, keys.inbox, keys.dots, ["dot"], keys.overview]) {
        invalidate(qc, queryKey);
      }
      break;
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  slot after now, null while the dot is disabled, when the schedule is not `active`, or when
  no slot or count is left; `retry_at` and `retry_attempt` are set while a retry waits.
- **Runs, dots and approvals are addressed by id.** `open_run_folder(run_id)` resolves the path
  in core; the UI cannot make the backend open, read or delete an arbitrary path. Two values
```

**with:**

```markdown
  slot after now, null while the dot is disabled, when the schedule is not `active`, or when
  no slot or count is left; `retry_at` and `retry_attempt` are set while a retry waits.
- The MCP/skills feature adds ten commands, each a one-line wrapper over `Runtime::library()`:
  `library_list`, `library_add(name, config, accept)`, `library_update(id, config)`,
  `library_remove(id, confirm)`, `library_probe(config)`, `library_check(id)`,
  `library_accept(id, fingerprint)`, `library_scan_skills(path)`, `installed_claude_plugins()`
  and `environment_variables(names)`. `config` carries its kind (`{kind, config}`); `accept` is
  the fingerprint `library_probe` answered. `library_remove` answers `conflict`, naming the
  dots, while dots use the item and `confirm` is false. A command the web UI names is
  confirmed in a native dialog from Rust, like `claude_path`, before Dots runs or stores it
  (the same `DOTS_CONFIRM=yes` seam in debug builds): `library_probe` with a stdio config, and
  `library_add` / `library_update` whose stdio `command`, `args`, `cwd` or `env` (as stored,
  the `${VAR}` text) differ from the stored ones (a new server differs) ask "Dots will run:
  <command> <args> in <cwd> with env KEY1, KEY2. Run it?" (keys only, never values); a
  `library_add` of a plugin, or a `library_update` that changes its path, whose `.mcp.json` has
  stdio servers asks "This plugin starts: <cmd> <args>; <cmd> <args>. Add it?"; and turning a
  plugin's `run_hooks` on asks "This plugin's hooks run commands on their own, without
  approval, and can approve tool calls. Run them?". When both apply to a plugin, one dialog
  holds both. A refusal answers `cancelled` and nothing is started or stored. A skill or plugin folder is
  only read.
- **Runs, dots and approvals are addressed by id.** `open_run_folder(run_id)` resolves the path
  in core; the UI cannot make the backend open, read or delete an arbitrary path. Two values
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    attention now. The core sends it once, when the flag goes from clear to set, together
    with a `ScheduleUpdated`. Only the notifier uses it.
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
```

**with:**

```markdown
    attention now. The core sends it once, when the flag goes from clear to set, together
    with a `ScheduleUpdated`. Only the notifier uses it.
  - And an eighth, `LibraryUpdated { item_id }`: a Library item was added, changed, removed or
    checked. It never notifies; it updates the tray's Inbox count (a server whose tool list
    changed is an Inbox group).
- Notifications fire on `ApprovalRequested`, and on `RunUpdated` into `succeeded`, `failed` or
  `awaiting_approval`, once per (run, status), and only for a run this process saw queued or
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
    call: that run's own `RunUpdated` does not reach those approvals. `ScheduleUpdated`
    invalidates `dots`, `overview` and `inbox`; `ScheduleAttention` invalidates nothing (the
    `ScheduleUpdated` sent with it does).
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
```

**with:**

```markdown
    call: that run's own `RunUpdated` does not reach those approvals. `ScheduleUpdated`
    invalidates `dots`, `overview` and `inbox`; `ScheduleAttention` invalidates nothing (the
    `ScheduleUpdated` sent with it does). `LibraryUpdated` invalidates `library`, `inbox`,
    `dots`, every `dot` and `overview` (removing an item takes it out of the dots that use it).
  - From Plan 3c an invalidation that arrives while its query is being fetched is kept (the
    query is marked stale again when that fetch has settled), and the app refetches everything
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  `DotOverview` sample that sets every field of `schedule_state`; `Limits` gains
  `max_schedule_attempts` (the core's `MAX_ATTEMPTS`, 6), which the Dots list and the run view
  name.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
```

**with:**

```markdown
  `DotOverview` sample that sets every field of `schedule_state`; `Limits` gains
  `max_schedule_attempts` (the core's `MAX_ATTEMPTS`, 6), which the Dots list and the run view
  name. From the MCP/skills feature it also holds the Library samples (a config of every kind
  and transport, `LibraryEntry` of every kind, a catalog, a probe, scanned skills, installed
  plugins, environment variables), a `LibraryUpdated` event, and every `LibraryKind` and
  `CatalogStatus`.
- Shell around the screens:
  - Banners: `server_error` (links to Settings), CLI version mismatch ("Mark as tested"),
```

- [ ] **Step 4: See the fixture test fail, then regenerate the fixture**

```bash
cargo test -p dots-app contract 2>&1 | grep -E "^test result|stale"
UPDATE_CONTRACT=1 cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
cargo test -p dots-app contract 2>&1 | grep -E "^test result: ok. 1 passed"
git diff --stat -- ui/src/api/__fixtures__/contract.json
```

Expected: stale, then two passes. `Commands` grows from 38 to 48 names; the fixture gains `EnvVariable`, `InstalledPlugin`, `LibraryConfig`, `LibraryEntry`, `LibraryItem`, `McpCatalog`, `McpProbe`, `McpServerConfig` and `ScannedSkill`, the enums `CatalogStatus` and `LibraryKind`, and the `library_updated` event.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p dots-app 2>&1 | grep -E "^test result: ok. [1-9]|FAILED"
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/api/commands.test.ts src/api/contract.test.ts src/lib/runtime-events.test.ts 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: `dots-app` 57 passed (among them `confirm::tests::a_plugin_that_starts_servers_asks_with_their_command_lines_and_the_hooks_in_one_dialog`, `a_declined_server_is_neither_probed_nor_stored_and_an_allowed_one_is` and `events::tests::a_library_update_does_not_notify_but_may_change_the_inbox`); the three UI files pass.

- [ ] **Step 6: Run everything**

```bash
cargo test -p dots-core 2>&1 | grep -E "^test result|FAILED|^error"
cargo test -p dots-app 2>&1 | grep -E "^test result|FAILED|^error"
cargo clippy --workspace --all-targets 2>&1 | grep -E "^warning|^error"
cargo fmt --check
```

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: `dots-core` 578 passed, `dots-app` 57 passed, no clippy line, `cargo fmt --check` silent, UI 310 passed in 21 files, typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add apps/dots-app/src/commands.rs apps/dots-app/src/confirm.rs apps/dots-app/src/contract.rs apps/dots-app/src/events.rs apps/dots-app/src/lib.rs docs/superpowers/specs/2026-10-08-dots-design.md ui/src/api/__fixtures__/contract.json ui/src/api/commands.test.ts ui/src/api/commands.ts ui/src/api/contract.test.ts ui/src/api/types.ts ui/src/lib/query.ts ui/src/lib/runtime-events.test.ts ui/src/lib/runtime-events.ts
git commit -m "feat(app): serve the Library through ten commands and tell the UI when it changes"
```

### Task 9: The Library screen for MCP servers, skills and plugins

The person gets the Library (spec section 8): a sidebar entry between Dots and Inbox and a screen with three tabs. MCP servers lists each server with its transport, a status badge, a "Missing variable" badge, its command line or URL, its last error, the plugin it comes with and the dots that use it, with Check now, Edit, Remove and, for a changed list, Review. The server form takes a stdio command line or an HTTP URL with headers, says that `${VAR}` reads Dots' environment and which variables are not set, warns about a pasted secret, tests the connection and saves accepting exactly the list it showed; any edit of the form forgets the test. The review shows the tools added, removed and changed (which of description, inputs or annotations), and accepts the list that was shown. Skills are added from a folder, with every skill below it to tick; plugins from a folder or from the installed Claude Code plugins, with their parts and the hooks switch. A refusal in the core's native dialog is a calm note, not an error. The main spec's section 7 and the sidebar list follow.

**Files:**
- Create: `ui/src/screens/library/LibraryScreen.test.tsx`, `ui/src/screens/library/LibraryScreen.tsx`, `ui/src/screens/library/McpServersTab.tsx`, `ui/src/screens/library/PluginsTab.tsx`, `ui/src/screens/library/ServerForm.tsx`, `ui/src/screens/library/SkillsTab.tsx`, `ui/src/screens/library/ToolsReview.tsx`, `ui/src/screens/library/parts.tsx`, `ui/src/screens/library/text.ts`, `ui/src/screens/library/tools.test.ts`, `ui/src/screens/library/tools.ts`
- Modify: `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/app/Layout.tsx`, `ui/src/app/routes.tsx`
- Test: `ui/src/screens/library/LibraryScreen.test.tsx`, `ui/src/screens/library/tools.test.ts`, `ui/src/test/fixtures.ts`

**Interfaces:**
- Consumes: Task 8's commands, types and keys; `ConfirmButton`, `invalidate`, `invalidateDots`, `CommandError`, `open` from `@tauri-apps/plugin-dialog`, and the components under `@/components/ui/`.
- Produces:
  - Route `library` and the sidebar link "Library" between Dots and Inbox. `LibraryScreen()`: the tabs "MCP servers", "Skills" and "Plugins"; `?review=<server id>` opens the review of a server whose status is `changed` (Task 10's Inbox links to it).
  - `McpServersTab(props: { entries: ServerEntry[]; plugins: PluginEntry[]; onReview: (id: string) => void })` and `serverStatus(entry: ServerEntry): CatalogStatus`; the status badges Accepted, Tools changed, Unreachable, Unchecked, and a separate "Missing variable: A, B" badge.
  - `ServerForm(props: { entry: ServerEntry | null; onClose: () => void })` and `readDraft(draft: Draft, checkName: boolean): { config: McpServerConfig; errors: Errors } | { config: null; errors: Errors }`. Save is `libraryAdd(name, config, probe?.fingerprint ?? null)` for a new server and `libraryUpdate(id, config)` for an edit (the name cannot change).
  - `ToolsReview(props: { entry: LibraryEntry; onClose: () => void })`: "Tools changed: <name>", Added, Removed, Changed with "description changed", "inputs changed", "annotations changed"; Accept calls `libraryAccept(id, catalog.seen_fingerprint)`, and a `conflict` is shown in the dialog with the list reloaded.
  - `SkillsTab({ entries }: { entries: SkillEntry[] })` ("Add skill", "Add skills" with ticks, "Choose another folder"); `PluginsTab({ entries }: { entries: PluginEntry[] })` (parts, warnings, the hooks switch through `libraryUpdate`, "Add plugin" from the installed plugins or a folder).
  - In `parts.tsx`: `isDeclined(e: unknown): boolean` (`CommandError` of kind `cancelled`), `ServerEntry`, `SkillEntry`, `PluginEntry`, `plural`, `ItemRow`, `UsedBy`, `ToolList`, `RemoveButton` (asks first, naming the dots, then `libraryRemove(id, used_by.length > 0)`).
  - `tools.ts`: `parseTools(value: unknown): McpTool[]`, `isReadOnly(tool: McpTool): boolean`, `type ToolPart`, `type ToolsDiff = { added; removed; changed: { tool; parts: ToolPart[] }[] }`, `diffTools(accepted: McpTool[], seen: McpTool[]): ToolsDiff` (a tool's `title` is not compared). `text.ts`: `parseLines(text, separator: "=" | ":"): Lines`, `joinLines`, `variablesIn(texts: string[]): string[]`, `looksSecret(key: string): boolean`.
  - Test fixtures: `sample.library`, `libraryItems`, `scannedSkills`, `installedPlugins`, `probe`; `installDefaults` answers `library_list` and `installed_claude_plugins` with `[]` and `environment_variables` with every name set.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/screens/library/LibraryScreen.test.tsx`:**

```tsx
import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { LibraryEntry } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [github, owned, skill, plugin] = sample.library as [
  LibraryEntry,
  LibraryEntry,
  LibraryEntry,
  LibraryEntry,
];
const cancelled = { kind: "cancelled", message: "declined: nothing was run or saved" };

const tool = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  description: `${name} tool`,
  inputSchema: { type: "object" },
  ...extra,
});

/// A server whose list changed: `seen` has one tool more, one less and one changed.
function changedServer(): LibraryEntry {
  const catalog = {
    ...github.catalog!,
    status: "changed" as const,
    accepted_tools: [tool("keep"), tool("gone"), tool("edit")],
    seen_fingerprint: "fp-seen",
    seen_tools: [
      tool("keep"),
      tool("edit", { description: "other words" }),
      tool("fresh", { annotations: { readOnlyHint: true } }),
    ],
  };
  return { ...github, catalog };
}

function listing(entries: LibraryEntry[]) {
  backend.on("library_list", () => entries);
}

describe("the Library screen", () => {
  it("has a place in the sidebar between Dots and Inbox", async () => {
    renderApp("/library");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const labels = within(nav)
      .getAllByRole("link")
      .map((l) => l.textContent?.replace(/\d+$/, ""));
    expect(labels.slice(0, 3)).toEqual(["Dots", "Library", "Inbox"]);
    expect(await screen.findByRole("heading", { name: "Library" })).toBeVisible();
  });

  it("lists the servers with their status, their dots and the variables they miss", async () => {
    listing([github, owned, plugin]);
    renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    expect(within(row).getByText("Tools changed")).toBeVisible();
    expect(within(row).getByText("stdio")).toBeVisible();
    expect(within(row).getByText("Missing variable: GITHUB_TOKEN")).toBeVisible();
    expect(within(row).getByText("Used by nightly-tests")).toBeVisible();
    const fromPlugin = screen.getByRole("listitem", { name: "plugin_pdf-tools_render" });
    expect(within(fromPlugin).getByText("Accepted")).toBeVisible();
    expect(within(fromPlugin).getByText("http")).toBeVisible();
    expect(within(fromPlugin).getByText("Comes with plugin pdf-tools")).toBeVisible();
    // A plugin's server changes with its plugin only.
    expect(within(fromPlugin).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(fromPlugin).queryByRole("button", { name: "Remove" })).toBeNull();
  });

  it("shows an unreachable server's error and an unchecked server", async () => {
    const down = {
      ...github,
      missing_variables: [],
      catalog: {
        ...github.catalog!,
        status: "unreachable" as const,
        error: "could not start: nope",
      },
    };
    const fresh = { ...github, id: "x", name: "fresh", missing_variables: [], catalog: null };
    listing([down, fresh]);
    renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    expect(within(row).getByText("Unreachable")).toBeVisible();
    expect(within(row).getByText("could not start: nope")).toBeVisible();
    expect(
      within(screen.getByRole("listitem", { name: "fresh" })).getByText("Unchecked"),
    ).toBeVisible();
  });

  it("checks a server now", async () => {
    listing([github]);
    backend.on("library_check", () => github.catalog);
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Check now" }));
    await waitFor(() => expect(backend.callsTo("library_check")).toEqual([{ id: github.id }]));
  });

  it("reloads the list when the core says the Library changed", async () => {
    listing([]);
    renderApp("/library");
    expect(await screen.findByText("No MCP servers yet.")).toBeVisible();
    listing([github]);
    act(() => backend.emit("runtime", { type: "library_updated", item_id: github.id }));
    expect(await screen.findByRole("listitem", { name: "github" })).toBeVisible();
  });

  it("shows a failed load", async () => {
    backend.on("library_list", () => {
      throw { kind: "internal", message: "no database" };
    });
    renderApp("/library");
    expect(await screen.findByRole("alert")).toHaveTextContent("no database");
  });
});

describe("adding a server", () => {
  async function open() {
    const view = renderApp("/library");
    await view.user.click(await screen.findByRole("button", { name: "Add server" }));
    return { ...view, dialog: await screen.findByRole("dialog", { name: "Add MCP server" }) };
  }

  it("tests the connection, shows the tools, and saves accepting what was shown", async () => {
    backend.on("library_probe", () => ({
      fingerprint: "fp-1",
      tools: [tool("read_it", { annotations: { readOnlyHint: true } }), tool("write_it")],
    }));
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    await user.type(within(dialog).getByLabelText("Command"), "npx");
    await user.type(within(dialog).getByLabelText("Arguments"), "-y{Enter}some-server");
    await user.type(within(dialog).getByLabelText("Environment"), "MODE=readonly");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));

    const config = {
      kind: "mcp_server",
      config: {
        transport: "stdio",
        command: "npx",
        args: ["-y", "some-server"],
        cwd: null,
        env: { MODE: "readonly" },
      },
    };
    expect(await within(dialog).findByText("read_it")).toBeVisible();
    expect(within(dialog).getByText("write_it tool")).toBeVisible();
    expect(within(dialog).getAllByText("read-only")).toHaveLength(1);
    expect(backend.callsTo("library_probe")).toEqual([{ config: config.config }]);

    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([{ name: "files", config, accept: "fp-1" }]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("forgets the test when the config changes, so a save accepts nothing it did not show", async () => {
    backend.on("library_probe", () => ({ fingerprint: "fp-1", tools: [tool("a")] }));
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    await user.type(within(dialog).getByLabelText("Command"), "npx");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));
    expect(await within(dialog).findByText("a tool")).toBeVisible();
    await user.type(within(dialog).getByLabelText("Command"), "x");
    expect(within(dialog).queryByText("a tool")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("library_add")[0]).toMatchObject({ accept: null }));
  });

  it("saves a server that could not be reached, unchecked", async () => {
    backend.on("library_probe", () => {
      throw { kind: "internal", message: "could not start: SF_TOKEN is not set" };
    });
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user, dialog } = await open();
    await user.selectOptions(within(dialog).getByLabelText("Transport"), "http");
    await user.type(within(dialog).getByLabelText("Name"), "sf");
    await user.type(within(dialog).getByLabelText("URL"), "https://example.com/mcp");
    await user.type(within(dialog).getByLabelText("Headers"), "Authorization: Bearer ${{T}");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));
    expect(await within(dialog).findByText("could not start: SF_TOKEN is not set")).toBeVisible();
    expect(within(dialog).getByText(/saved without its tools/i)).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "sf",
          config: {
            kind: "mcp_server",
            config: {
              transport: "http",
              url: "https://example.com/mcp",
              headers: { Authorization: "Bearer ${T}" },
            },
          },
          accept: null,
        },
      ]),
    );
  });

  it("names the variables that Dots' environment does not set, and warns about a pasted secret", async () => {
    backend.on("environment_variables", (args) =>
      (args.names as string[]).map((name) => ({ name, set: name !== "NOT_SET" })),
    );
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Command"), "run");
    await user.type(
      within(dialog).getByLabelText("Environment"),
      "A=${{NOT_SET}{Enter}B=${{IS_SET}{Enter}API_TOKEN=abc123",
    );
    expect(await within(dialog).findByText("Not set in Dots' environment: NOT_SET")).toBeVisible();
    expect(within(dialog).getByText(/API_TOKEN looks like a secret.*\$\{VAR\}/)).toBeVisible();
    expect(within(dialog).getByText(/reads the variable from Dots' environment/)).toBeVisible();
  });

  it("checks the form before it asks the core", async () => {
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "Bad Name");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(
      within(dialog).getByText('Use 1 to 40 of a-z, 0-9, _ and -. "dots" is reserved.'),
    ).toBeVisible();
    expect(within(dialog).getByText("Enter the command to run.")).toBeVisible();
    await user.type(within(dialog).getByLabelText("Environment"), "no-equals");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("Line 1 needs the form KEY=VALUE.")).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("takes a refusal in the native dialog calmly, and a core error where it happened", async () => {
    backend.on("library_probe", () => {
      throw cancelled;
    });
    backend.on("library_add", () => {
      throw { kind: "conflict", message: 'the mcp server "files" already exists' };
    });
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    await user.type(within(dialog).getByLabelText("Command"), "npx");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));
    expect(await within(dialog).findByText(/You declined to run it/)).toBeVisible();
    expect(screen.queryByText(/declined: nothing/)).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
    expect(screen.getByRole("dialog", { name: "Add MCP server" })).toBeVisible();
  });

  it("edits a server: the name stays, the config goes to the core, a refusal keeps the form", async () => {
    listing([github]);
    backend.on("library_update", () => {
      throw cancelled;
    });
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit github" });
    expect(within(dialog).getByLabelText("Name")).toBeDisabled();
    expect(within(dialog).getByLabelText("Command")).toHaveValue("npx");
    expect(within(dialog).getByLabelText("Arguments")).toHaveValue(
      "-y\n@modelcontextprotocol/server-github",
    );
    expect(within(dialog).getByLabelText("Environment")).toHaveValue(
      "GITHUB_TOKEN=${GITHUB_TOKEN}",
    );
    await user.type(within(dialog).getByLabelText("Working folder"), "C:\\work");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_update")).toEqual([
        {
          id: github.id,
          config: {
            kind: "mcp_server",
            config: { ...github.config, cwd: "C:\\work" },
          },
        },
      ]),
    );
    expect(await within(dialog).findByText(/You declined the change/)).toBeVisible();
    expect(screen.getByRole("dialog", { name: "Edit github" })).toBeVisible();
  });
});

describe("the review of a changed tool list", () => {
  it("shows what was added, removed and changed, and accepts the list that was shown", async () => {
    listing([changedServer()]);
    backend.on("library_accept", () => ({ ...github.catalog!, status: "accepted" }));
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    expect(within(dialog).getByText("Added")).toBeVisible();
    expect(within(dialog).getByText("fresh")).toBeVisible();
    expect(within(dialog).getByText("read-only")).toBeVisible();
    expect(within(dialog).getByText("Removed")).toBeVisible();
    expect(within(dialog).getByText("gone")).toBeVisible();
    expect(within(dialog).getByText("Changed")).toBeVisible();
    expect(within(dialog).getByText("edit")).toBeVisible();
    expect(within(dialog).getByText("description changed")).toBeVisible();
    expect(within(dialog).queryByText("keep")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Accept" }));
    await waitFor(() =>
      expect(backend.callsTo("library_accept")).toEqual([
        { id: github.id, fingerprint: "fp-seen" },
      ]),
    );
  });

  it("says so when the list moved since it was shown", async () => {
    listing([changedServer()]);
    backend.on("library_accept", () => {
      throw { kind: "conflict", message: "the tool list changed again" };
    });
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    await user.click(within(dialog).getByRole("button", { name: "Accept" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("changed again");
  });

  it("opens from the Inbox's link and for a server that was never accepted", async () => {
    const changed = changedServer();
    listing([{ ...changed, catalog: { ...changed.catalog!, accepted_tools: null } }]);
    renderApp(`/library?review=${github.id}`);
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    expect(within(dialog).getByText("fresh")).toBeVisible();
    expect(within(dialog).getByText("keep")).toBeVisible();
  });
});

describe("removing an item", () => {
  it("asks first and names the dots, then removes it from them", async () => {
    listing([github]);
    backend.on("library_remove", () => null);
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("nightly-tests");
    expect(backend.callsTo("library_remove")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Remove server" }));
    await waitFor(() =>
      expect(backend.callsTo("library_remove")).toEqual([{ id: github.id, confirm: true }]),
    );
  });

  it("does not claim dots use an item no dot uses", async () => {
    listing([{ ...github, used_by: [] }]);
    backend.on("library_remove", () => null);
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("No dot uses it.");
    await user.click(within(dialog).getByRole("button", { name: "Remove server" }));
    await waitFor(() =>
      expect(backend.callsTo("library_remove")).toEqual([{ id: github.id, confirm: false }]),
    );
  });
});

describe("skills", () => {
  it("lists them with their description, folder, users and problem", async () => {
    listing([github, skill]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    const row = await screen.findByRole("listitem", { name: "pdf" });
    expect(within(row).getByText("Fill and read PDF forms.")).toBeVisible();
    expect(within(row).getByText("C:/skills/pdf")).toBeVisible();
    expect(within(row).getByText("SKILL.md not found")).toBeVisible();
    expect(screen.queryByRole("listitem", { name: "github" })).toBeNull();
  });

  it("scans a folder and adds the skills that were ticked", async () => {
    backend.on("dialog_open", () => "C:/skills");
    backend.on("library_scan_skills", () => sample.scannedSkills);
    backend.on("library_add", () => sample.libraryItems[1]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    await user.click(screen.getByRole("button", { name: "Add skill" }));
    const dialog = await screen.findByRole("dialog", { name: "Add skills" });
    expect(await within(dialog).findByLabelText("pdf")).toBeChecked();
    // A name the Library cannot take is shown with the reason and cannot be ticked.
    expect(within(dialog).getByLabelText("bad name")).toBeDisabled();
    expect(within(dialog).getByText(/use A-Z, a-z, 0-9/)).toBeVisible();
    expect(backend.callsTo("dialog_open")[0]).toMatchObject({ directory: true });
    expect(backend.callsTo("library_scan_skills")).toEqual([{ path: "C:/skills" }]);
    await user.click(within(dialog).getByRole("button", { name: "Add 1 skill" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        { name: "pdf", config: { kind: "skill", config: { path: "C:/skills/pdf" } }, accept: null },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("says when a folder holds no skill, and keeps the dialog when an add fails", async () => {
    backend.on("dialog_open", () => "C:/empty");
    backend.on("library_scan_skills", () => []);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    await user.click(screen.getByRole("button", { name: "Add skill" }));
    const dialog = await screen.findByRole("dialog", { name: "Add skills" });
    expect(await within(dialog).findByText("No SKILL.md found in that folder.")).toBeVisible();

    backend.on("library_scan_skills", () => sample.scannedSkills);
    backend.on("library_add", () => {
      throw { kind: "conflict", message: 'the skill "pdf" already exists' };
    });
    await user.click(within(dialog).getByRole("button", { name: "Choose another folder" }));
    await user.click(await within(dialog).findByRole("button", { name: "Add 1 skill" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
  });
});

describe("plugins", () => {
  it("lists a plugin with its parts, folder and users", async () => {
    listing([plugin, owned]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    const row = await screen.findByRole("listitem", { name: "pdf-tools" });
    expect(within(row).getByText("C:/plugins/pdf-tools")).toBeVisible();
    expect(within(row).getByText("1 skill")).toBeVisible();
    expect(within(row).getByText("1 command")).toBeVisible();
    expect(within(row).getByText("1 agent")).toBeVisible();
    expect(within(row).getByText("1 MCP server")).toBeVisible();
    expect(within(row).getByText("hooks")).toBeVisible();
    expect(within(row).getByText("Used by nightly-tests")).toBeVisible();
    expect(within(row).getByText("agents/broken.md: could not be read")).toBeVisible();
    expect(within(row).getByLabelText("Run this plugin's hooks")).not.toBeChecked();
  });

  it("switches the hooks on through the core, which asks, without a second question here", async () => {
    listing([plugin]);
    backend.on("library_update", () => sample.libraryItems[1]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    const row = await screen.findByRole("listitem", { name: "pdf-tools" });
    await user.click(within(row).getByLabelText("Run this plugin's hooks"));
    await waitFor(() =>
      expect(backend.callsTo("library_update")).toEqual([
        {
          id: plugin.id,
          config: { kind: "plugin", config: { ...plugin.config, run_hooks: true } },
        },
      ]),
    );
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("keeps the hooks off and says so calmly when the person declines", async () => {
    listing([plugin]);
    backend.on("library_update", () => {
      throw cancelled;
    });
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    const row = await screen.findByRole("listitem", { name: "pdf-tools" });
    await user.click(within(row).getByLabelText("Run this plugin's hooks"));
    expect(await within(row).findByText(/You declined: the hooks stay off/)).toBeVisible();
    expect(within(row).getByLabelText("Run this plugin's hooks")).not.toBeChecked();
    expect(screen.queryByText(/declined: nothing/)).toBeNull();
  });

  it("adds an installed Claude Code plugin", async () => {
    listing([]);
    backend.on("installed_claude_plugins", () => [
      ...sample.installedPlugins,
      { ...sample.installedPlugins[0]!, id: "gone@market", name: "gone", exists: false },
    ]);
    backend.on("library_add", () => sample.libraryItems[1]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    await user.click(screen.getByRole("button", { name: "Add plugin" }));
    const dialog = await screen.findByRole("dialog", { name: "Add plugin" });
    const installed = await within(dialog).findByRole("listitem", { name: "pdf-tools@market" });
    const gone = within(dialog).getByRole("listitem", { name: "gone@market" });
    expect(gone).toHaveTextContent("folder not found");
    expect(within(gone).getByRole("button", { name: "Add" })).toBeDisabled();
    await user.click(within(installed).getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "pdf-tools",
          config: {
            kind: "plugin",
            config: {
              path: sample.installedPlugins[0]!.install_path,
              installed_id: "pdf-tools@market",
              run_hooks: false,
            },
          },
          accept: null,
        },
      ]),
    );
  });

  it("adds a plugin from a folder under the folder's name, which can be changed", async () => {
    listing([]);
    backend.on("dialog_open", () => "C:\\plugins\\pdf-tools");
    backend.on("library_add", () => {
      throw cancelled;
    });
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    await user.click(screen.getByRole("button", { name: "Add plugin" }));
    const dialog = await screen.findByRole("dialog", { name: "Add plugin" });
    await user.click(within(dialog).getByRole("button", { name: "Choose a folder" }));
    const name = await within(dialog).findByLabelText("Name");
    expect(name).toHaveValue("pdf-tools");
    await user.clear(name);
    await user.type(name, "pdfs");
    await user.click(within(dialog).getByRole("button", { name: "Add from folder" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "pdfs",
          config: {
            kind: "plugin",
            config: { path: "C:\\plugins\\pdf-tools", installed_id: null, run_hooks: false },
          },
          accept: null,
        },
      ]),
    );
    // The core asks about the servers the plugin starts; a refusal is not an error.
    expect(await within(dialog).findByText(/You declined: the plugin was not added/)).toBeVisible();
  });
});
```

**Create `ui/src/screens/library/tools.test.ts`:**

```ts
import { describe, expect, it } from "vitest";

import { sample } from "@/test/fixtures";

import { diffTools, isReadOnly, parseTools } from "./tools";
import { looksSecret, parseLines, variablesIn } from "./text";

const tool = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  description: `${name} tool`,
  inputSchema: { type: "object" },
  ...extra,
});

describe("a tool list", () => {
  it("is read from the JSON the core keeps, and is empty when it is not a list of tools", () => {
    expect(parseTools([tool("a"), tool("b")]).map((t) => t.name)).toEqual(["a", "b"]);
    expect(parseTools(null)).toEqual([]);
    expect(parseTools({ tools: [] })).toEqual([]);
    expect(parseTools([{ nope: 1 }])).toEqual([]);
    const seen = sample.library[0]!.catalog!.seen_tools;
    expect(parseTools(seen)[0]).toMatchObject({ name: "search_issues" });
  });

  it("calls a tool read-only only when the server says so with a true hint", () => {
    expect(isReadOnly(tool("a", { annotations: { readOnlyHint: true } }))).toBe(true);
    expect(isReadOnly(tool("a", { annotations: { readOnlyHint: "yes" } }))).toBe(false);
    expect(isReadOnly(tool("a", { annotations: null }))).toBe(false);
    expect(isReadOnly(tool("a"))).toBe(false);
  });

  it("tells what was added, removed and changed, and which part of a changed tool", () => {
    const accepted = parseTools([
      tool("kept"),
      tool("gone"),
      tool("words"),
      tool("shape", { inputSchema: { type: "object", properties: { a: {} } } }),
      tool("hint", { annotations: { readOnlyHint: true } }),
    ]);
    const seen = parseTools([
      tool("kept", { title: "only the title differs" }),
      tool("words", { description: "other words" }),
      // The same schema with its keys in another order is the same schema.
      tool("shape", { inputSchema: { properties: { a: {} }, type: "object" } }),
      tool("hint", { annotations: { readOnlyHint: false } }),
      tool("new"),
    ]);
    const diff = diffTools(accepted, seen);
    expect(diff.added.map((t) => t.name)).toEqual(["new"]);
    expect(diff.removed.map((t) => t.name)).toEqual(["gone"]);
    expect(diff.changed.map((c) => [c.tool.name, c.parts])).toEqual([
      ["words", ["description"]],
      ["hint", ["annotations"]],
    ]);
    const inputs = diffTools(
      parseTools([tool("t")]),
      parseTools([tool("t", { inputSchema: { type: "string" } })]),
    );
    expect(inputs.changed.map((c) => c.parts)).toEqual([["inputs"]]);
  });
});

describe("the text of the server form", () => {
  it("reads one KEY=VALUE or Name: value per line, and says which line is wrong", () => {
    expect(parseLines("A=1\n\n B = x=y \n", "=")).toEqual({
      ok: true,
      entries: { A: "1", B: "x=y" },
    });
    expect(parseLines("Authorization: Bearer ${T}", ":")).toEqual({
      ok: true,
      entries: { Authorization: "Bearer ${T}" },
    });
    expect(parseLines("fine=1\nbroken", "=")).toEqual({ ok: false, line: 2 });
    expect(parseLines("=empty", "=")).toEqual({ ok: false, line: 1 });
  });

  it("finds the variables a config reads, once each", () => {
    expect(variablesIn(["${A}/x", "no", "${B}${A}", "$C", "${1X}"])).toEqual(["A", "B"]);
  });

  it("flags a key that looks like a secret", () => {
    for (const key of ["API_TOKEN", "secret", "MyKey", "PASSWORD", "Authorization"]) {
      expect(looksSecret(key)).toBe(true);
    }
    expect(looksSecret("MODE")).toBe(false);
  });
});
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  settings: T.Settings.parse(contract.Settings),
  overview: T.DotOverview.parse(contract.DotOverview),
  inbox: T.Inbox.parse(contract.Inbox),
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
```

**with:**

```ts
  settings: T.Settings.parse(contract.Settings),
  overview: T.DotOverview.parse(contract.DotOverview),
  /// A stdio server with a changed tool list and a missing variable; a plugin's HTTP server; a
  /// skill with a problem; a plugin with every part.
  library: T.LibraryEntry.array().parse(contract.LibraryEntry),
  /// The same items without the extras of a list entry: what `library_add` answers.
  libraryItems: T.LibraryItem.array().parse(contract.LibraryItem),
  scannedSkills: T.ScannedSkill.array().parse(contract.ScannedSkill),
  installedPlugins: T.InstalledPlugin.array().parse(contract.InstalledPlugin),
  probe: T.McpProbe.parse(contract.McpProbe),
  inbox: T.Inbox.parse(contract.Inbox),
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
```

**In `ui/src/test/fixtures.ts`, replace:**

```ts
  backend.on("list_dots", () => []);
  backend.on("get_settings", () => sample.settings);
  backend.on("new_dot_spec", () => blankSpec);
  backend.on("list_runs", () => []);
```

**with:**

```ts
  backend.on("list_dots", () => []);
  backend.on("get_settings", () => sample.settings);
  backend.on("library_list", () => []);
  backend.on("installed_claude_plugins", () => []);
  backend.on("environment_variables", (args) =>
    (args.names as string[]).map((name) => ({ name, set: true })),
  );
  backend.on("new_dot_spec", () => blankSpec);
  backend.on("list_runs", () => []);
```

- [ ] **Step 2: Run them to verify they fail**

```bash
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/library 2>&1 | grep -E "Tests |Test Files|× |FAIL ")
```

Expected: `Test Files  2 failed (2)`, `Tests  26 failed (26)`: `tools.test.ts` fails as a file (`./tools` does not exist), and every test of `LibraryScreen.test.tsx` fails, from "has a place in the sidebar between Dots and Inbox" and "lists the servers with their status, their dots and the variables they miss" to "adds a plugin from a folder under the folder's name, which can be changed".

- [ ] **Step 3: Implement**

**In `ui/src/app/routes.tsx`, replace:**

```tsx
import { DotsScreen } from "@/screens/DotsScreen";
import { InboxScreen } from "@/screens/InboxScreen";
import { RunScreen } from "@/screens/RunScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
```

**with:**

```tsx
import { DotsScreen } from "@/screens/DotsScreen";
import { InboxScreen } from "@/screens/InboxScreen";
import { LibraryScreen } from "@/screens/library/LibraryScreen";
import { RunScreen } from "@/screens/RunScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
```

**In `ui/src/app/routes.tsx`, replace:**

```tsx
      { path: "dots/:dotId", element: <DotEditorScreen /> },
      { path: "runs/:runId", element: <RunScreen /> },
      { path: "inbox", element: <InboxScreen /> },
      { path: "chat", element: <ChatScreen /> },
```

**with:**

```tsx
      { path: "dots/:dotId", element: <DotEditorScreen /> },
      { path: "runs/:runId", element: <RunScreen /> },
      { path: "library", element: <LibraryScreen /> },
      { path: "inbox", element: <InboxScreen /> },
      { path: "chat", element: <ChatScreen /> },
```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Bot, Inbox as InboxIcon, MessageSquare, Settings as SettingsIcon } from "lucide-react";
import { NavLink, Outlet } from "react-router";

```

**with:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Bot, Inbox as InboxIcon, Library as LibraryIcon, MessageSquare, Settings as SettingsIcon } from "lucide-react";
import { NavLink, Outlet } from "react-router";

```

**In `ui/src/app/Layout.tsx`, replace:**

```tsx
const LINKS = [
  { to: "/dots", label: "Dots", icon: Bot },
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/chat", label: "Chat", icon: MessageSquare },
```

**with:**

```tsx
const LINKS = [
  { to: "/dots", label: "Dots", icon: Bot },
  { to: "/library", label: "Library", icon: LibraryIcon },
  { to: "/inbox", label: "Inbox", icon: InboxIcon },
  { to: "/chat", label: "Chat", icon: MessageSquare },
```

**Create `ui/src/screens/library/text.ts`:**

```ts
/// One `KEY=VALUE` (or `Name: value`) per line, as the server form takes env and headers.
/// Blank lines are skipped. `line` of a failure counts from 1, blank lines included.
export type Lines = { ok: true; entries: Record<string, string> } | { ok: false; line: number };

export function parseLines(text: string, separator: "=" | ":"): Lines {
  const entries: Record<string, string> = {};
  const lines = text.split(/\r?\n/);
  for (const [index, raw] of lines.entries()) {
    if (raw.trim() === "") continue;
    const at = raw.indexOf(separator);
    const key = at < 0 ? "" : raw.slice(0, at).trim();
    if (key === "") return { ok: false, line: index + 1 };
    entries[key] = raw.slice(at + 1).trim();
  }
  return { ok: true, entries };
}

/// The inverse of `parseLines`.
export function joinLines(entries: Record<string, string>, separator: "=" | ":"): string {
  const glue = separator === "=" ? "=" : ": ";
  return Object.entries(entries)
    .map(([k, v]) => `${k}${glue}${v}`)
    .join("\n");
}

/// The names of the `${VAR}` references in `texts`, once each, in the order they appear.
export function variablesIn(texts: string[]): string[] {
  const names = new Set<string>();
  for (const text of texts) {
    for (const match of text.matchAll(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g)) names.add(match[1]!);
  }
  return [...names];
}

/// Whether a key (an env name or a header name) looks like it holds a secret (spec section 2).
export function looksSecret(key: string): boolean {
  return /token|key|secret|password|auth/i.test(key);
}
```

**Create `ui/src/screens/library/tools.ts`:**

```ts
import { McpTool } from "@/api/types";

/// The tools in a tool list the core keeps as JSON (`accepted_tools`, `seen_tools`). Anything
/// that is not a list of tools reads as an empty list.
export function parseTools(value: unknown): McpTool[] {
  const parsed = McpTool.array().safeParse(value);
  return parsed.success ? parsed.data : [];
}

/// Whether the server calls the tool read-only. Its own claim, and only a true hint counts.
export function isReadOnly(tool: McpTool): boolean {
  const annotations = tool.annotations;
  return (
    typeof annotations === "object" &&
    annotations !== null &&
    (annotations as Record<string, unknown>).readOnlyHint === true
  );
}

/// JSON with the keys of every object sorted: two values that mean the same text the same.
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object" && value !== null) {
    const entries = Object.entries(value)
      .filter(([, v]) => v !== undefined)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

export type ToolPart = "description" | "inputs" | "annotations";

export type ToolsDiff = {
  added: McpTool[];
  removed: McpTool[];
  changed: { tool: McpTool; parts: ToolPart[] }[];
};

/// What differs between the accepted tools and the ones last seen. Only what the fingerprint
/// covers counts (spec section 3): a tool's `title` is not compared.
export function diffTools(accepted: McpTool[], seen: McpTool[]): ToolsDiff {
  const before = new Map(accepted.map((t) => [t.name, t]));
  const after = new Map(seen.map((t) => [t.name, t]));
  const changed: ToolsDiff["changed"] = [];
  for (const tool of seen) {
    const old = before.get(tool.name);
    if (!old) continue;
    const parts: ToolPart[] = [];
    if ((old.description ?? "") !== (tool.description ?? "")) parts.push("description");
    if (canonical(old.inputSchema) !== canonical(tool.inputSchema)) parts.push("inputs");
    if (canonical(old.annotations ?? null) !== canonical(tool.annotations ?? null)) {
      parts.push("annotations");
    }
    if (parts.length > 0) changed.push({ tool, parts });
  }
  return {
    added: seen.filter((t) => !before.has(t.name)),
    removed: accepted.filter((t) => !after.has(t.name)),
    changed,
  };
}
```

**Create `ui/src/screens/library/parts.tsx`:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { CommandError, commands } from "@/api/commands";
import type { LibraryEntry, McpTool } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Badge } from "@/components/ui/badge";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";

import { isReadOnly } from "./tools";

/// Whether an error is the person's own "no" to a native confirmation (a `cancelled` command).
/// The Library screen never asks a second time: the core's dialog is the question.
export function isDeclined(e: unknown): boolean {
  return e instanceof CommandError && e.kind === "cancelled";
}

export type ServerEntry = Extract<LibraryEntry, { kind: "mcp_server" }>;
export type SkillEntry = Extract<LibraryEntry, { kind: "skill" }>;
export type PluginEntry = Extract<LibraryEntry, { kind: "plugin" }>;

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/// One item of a list: `children` on the left, the actions on the right.
export function ItemRow(props: { label: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <li aria-label={props.label} className="flex items-start gap-3 rounded-lg border p-3">
      <div className="flex min-w-0 flex-1 flex-col gap-1">{props.children}</div>
      <div className="flex shrink-0 items-center gap-2">{props.actions}</div>
    </li>
  );
}

/// The dots that use an item.
export function UsedBy({ dots }: { dots: LibraryEntry["used_by"] }) {
  if (dots.length === 0) return <span className="text-xs text-muted-foreground">Not used yet</span>;
  return (
    <span className="text-xs text-muted-foreground">
      Used by {dots.map((d) => d.name).join(", ")}
    </span>
  );
}

/// An MCP server's tools, each with its description and a "read-only" badge where the server
/// says so.
export function ToolList({ tools }: { tools: McpTool[] }) {
  if (tools.length === 0) {
    return <p className="text-xs text-muted-foreground">The server lists no tools.</p>;
  }
  return (
    <ul className="flex flex-col gap-1.5">
      {tools.map((tool) => (
        <li key={tool.name} className="flex flex-col gap-0.5 rounded-md border px-2 py-1.5">
          <div className="flex items-center gap-2">
            <span className="font-mono text-xs font-medium">{tool.name}</span>
            {isReadOnly(tool) && <Badge variant="secondary">read-only</Badge>}
          </div>
          {tool.description && <span className="text-xs text-muted-foreground">{tool.description}</span>}
        </li>
      ))}
    </ul>
  );
}

const KIND_WORD = { mcp_server: "server", skill: "skill", plugin: "plugin" } as const;

/// "Remove": asks first and names the dots that use the item, then removes it and takes it out
/// of them (`confirm` is true only when dots use it).
export function RemoveButton({ entry }: { entry: LibraryEntry }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.libraryRemove(entry.id, entry.used_by.length > 0),
    onSuccess: () => {
      invalidate(qc, keys.library);
      invalidateDots(qc);
    },
    onError: notifyError,
  });
  const word = KIND_WORD[entry.kind];
  const users = entry.used_by.map((d) => d.name).join(", ");
  return (
    <ConfirmButton
      title={`Remove ${entry.name}?`}
      description={
        <>
          {entry.used_by.length > 0
            ? `Used by ${users}. Removing it takes it out of them.`
            : "No dot uses it."}
          {entry.kind === "plugin" && " Its MCP servers go with it."}
        </>
      }
      confirmLabel={`Remove ${word}`}
      disabled={remove.isPending}
      onConfirm={() => remove.mutate()}
    >
      Remove
    </ConfirmButton>
  );
}
```

**Create `ui/src/screens/library/ToolsReview.tsx`:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { commands } from "@/api/commands";
import type { LibraryEntry } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ToolList } from "./parts";
import { diffTools, parseTools } from "./tools";

function Group(props: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">{props.title}</h3>
      {props.children}
    </section>
  );
}

/// The review of a server whose tool list differs from the accepted one: the tools added,
/// removed and changed (and which part of a changed tool), and "Accept". Accepting names the
/// list that is shown; the core refuses it if the list moved meanwhile.
export function ToolsReview(props: { entry: LibraryEntry; onClose: () => void }) {
  const { entry } = props;
  const qc = useQueryClient();
  const catalog = entry.catalog;
  const diff = diffTools(parseTools(catalog?.accepted_tools), parseTools(catalog?.seen_tools));
  const accept = useMutation({
    mutationFn: () => commands.libraryAccept(entry.id, catalog?.seen_fingerprint ?? ""),
    onSuccess: () => {
      invalidate(qc, keys.library);
      props.onClose();
    },
    // The list moved since it was loaded: load the newer one.
    onError: () => invalidate(qc, keys.library),
  });
  const never = catalog?.accepted_fingerprint == null;

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{`Tools changed: ${entry.name}`}</DialogTitle>
          <DialogDescription>
            {never
              ? "This server's tools were never accepted."
              : "The server lists different tools than the ones you accepted."}{" "}
            Until you accept the new list, runs that use the server ask before every call to it.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          {diff.added.length > 0 && (
            <Group title="Added">
              <ToolList tools={diff.added} />
            </Group>
          )}
          {diff.removed.length > 0 && (
            <Group title="Removed">
              <ToolList tools={diff.removed} />
            </Group>
          )}
          {diff.changed.length > 0 && (
            <Group title="Changed">
              <ul className="flex flex-col gap-1.5">
                {diff.changed.map(({ tool, parts }) => (
                  <li key={tool.name} className="flex flex-col gap-1 rounded-md border px-2 py-1.5">
                    <span className="font-mono text-xs font-medium">{tool.name}</span>
                    <span className="flex flex-wrap gap-1">
                      {parts.map((part) => (
                        <Badge key={part} variant="secondary">{`${part} changed`}</Badge>
                      ))}
                    </span>
                  </li>
                ))}
              </ul>
            </Group>
          )}
          {accept.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(accept.error)}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Close
          </Button>
          <Button type="button" disabled={accept.isPending} onClick={() => accept.mutate()}>
            Accept
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

**Create `ui/src/screens/library/ServerForm.tsx`:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { McpServerConfig } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { isDeclined, ToolList, type ServerEntry } from "./parts";
import { joinLines, looksSecret, parseLines, variablesIn } from "./text";
import { parseTools } from "./tools";

type Draft = {
  name: string;
  transport: "stdio" | "http";
  command: string;
  args: string;
  cwd: string;
  env: string;
  url: string;
  headers: string;
};

type Errors = Partial<Record<"name" | "command" | "args" | "env" | "url" | "headers", string>>;

const NAME_ERROR = 'Use 1 to 40 of a-z, 0-9, _ and -. "dots" is reserved.';

function draftOf(entry: ServerEntry | null): Draft {
  const blank: Draft = {
    name: "",
    transport: "stdio",
    command: "",
    args: "",
    cwd: "",
    env: "",
    url: "",
    headers: "",
  };
  if (!entry) return blank;
  const config = entry.config;
  return config.transport === "stdio"
    ? {
        ...blank,
        name: entry.name,
        command: config.command,
        args: config.args.join("\n"),
        cwd: config.cwd ?? "",
        env: joinLines(config.env, "="),
      }
    : {
        ...blank,
        name: entry.name,
        transport: "http",
        url: config.url,
        headers: joinLines(config.headers, ":"),
      };
}

/// The config the form describes, or what is wrong with it. `name` is checked only when asked:
/// a test of the connection does not need one.
export function readDraft(
  draft: Draft,
  checkName: boolean,
): { config: McpServerConfig; errors: Errors } | { config: null; errors: Errors } {
  const errors: Errors = {};
  if (checkName && (draft.name === "dots" || !/^[a-z0-9_-]{1,40}$/.test(draft.name))) {
    errors.name = NAME_ERROR;
  }
  if (draft.transport === "stdio") {
    if (draft.command.trim() === "") errors.command = "Enter the command to run.";
    const env = parseLines(draft.env, "=");
    if (!env.ok) errors.env = `Line ${env.line} needs the form KEY=VALUE.`;
    if (Object.keys(errors).length > 0 || !env.ok) return { config: null, errors };
    return {
      errors,
      config: {
        transport: "stdio",
        command: draft.command.trim(),
        args: draft.args.split(/\r?\n/).filter((a) => a.trim() !== ""),
        cwd: draft.cwd.trim() === "" ? null : draft.cwd.trim(),
        env: env.entries,
      },
    };
  }
  if (!/^https?:\/\/\S+$/.test(draft.url.trim())) errors.url = "The URL starts with http:// or https://.";
  const headers = parseLines(draft.headers, ":");
  if (!headers.ok) errors.headers = `Line ${headers.line} needs the form Name: value.`;
  if (Object.keys(errors).length > 0 || !headers.ok) return { config: null, errors };
  return { errors, config: { transport: "http", url: draft.url.trim(), headers: headers.entries } };
}

/// The variables the draft reads with `${VAR}`.
function variablesOf(draft: Draft): string[] {
  return variablesIn(
    draft.transport === "stdio"
      ? [draft.command, draft.args, draft.cwd, draft.env]
      : [draft.url, draft.headers],
  );
}

/// The keys of env or headers that look like secrets and hold a plain value.
function secretKeys(draft: Draft): string[] {
  const parsed =
    draft.transport === "stdio" ? parseLines(draft.env, "=") : parseLines(draft.headers, ":");
  if (!parsed.ok) return [];
  return Object.entries(parsed.entries)
    .filter(([key, value]) => looksSecret(key) && !value.includes("${"))
    .map(([key]) => key);
}

/// The form to add an MCP server (`entry` is null) or to change one. Adding tests the
/// connection first when the person asks; saving then accepts the list that was shown.
export function ServerForm(props: { entry: ServerEntry | null; onClose: () => void }) {
  const { entry } = props;
  const qc = useQueryClient();
  const [draft, setDraft] = useState(() => draftOf(entry));
  const [tried, setTried] = useState(false);
  const set = (change: Partial<Draft>) => {
    setDraft({ ...draft, ...change });
    // The list that was shown belongs to the config that was tested.
    probe.reset();
  };

  const probe = useMutation({ mutationFn: commands.libraryProbe });
  const save = useMutation({
    mutationFn: (v: { config: McpServerConfig; accept: string | null }) => {
      const config = { kind: "mcp_server" as const, config: v.config };
      return entry
        ? commands.libraryUpdate(entry.id, config)
        : commands.libraryAdd(draft.name, config, v.accept);
    },
    onSuccess: () => {
      invalidate(qc, keys.library);
      props.onClose();
    },
  });

  const variables = variablesOf(draft);
  const env = useQuery({
    queryKey: keys.environmentVariables(variables),
    queryFn: () => commands.environmentVariables(variables),
    enabled: variables.length > 0,
  });
  const notSet = (env.data ?? []).filter((v) => !v.set).map((v) => v.name);

  const { config, errors } = readDraft(draft, !entry);
  const shown = tried ? errors : {};
  function test() {
    setTried(true);
    const tested = readDraft(draft, false);
    if (tested.config) probe.mutate(tested.config);
  }
  function submit() {
    setTried(true);
    if (!config) return;
    save.mutate({ config, accept: probe.data?.fingerprint ?? null });
  }
  function browse() {
    open({ directory: true, title: "Choose the folder the server runs in" }).then((picked) => {
      if (typeof picked === "string") set({ cwd: picked });
    }, notifyError);
  }

  const stdio = draft.transport === "stdio";
  const tools = probe.data ? parseTools(probe.data.tools) : [];
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{entry ? `Edit ${entry.name}` : "Add MCP server"}</DialogTitle>
          <DialogDescription>
            A server gives dots tools named <code>mcp__name__tool</code>. Dots starts it for each
            run that uses it.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3">
          <Field label="Name" htmlFor="server-name" error={shown.name}>
            <Input
              id="server-name"
              value={draft.name}
              disabled={entry !== null}
              onChange={(e) => set({ name: e.target.value })}
            />
          </Field>
          <Field label="Transport" htmlFor="server-transport">
            <NativeSelect
              id="server-transport"
              value={draft.transport}
              onChange={(e) => set({ transport: e.target.value as Draft["transport"] })}
            >
              <option value="stdio">stdio (a program Dots starts)</option>
              <option value="http">http (a server Dots connects to)</option>
            </NativeSelect>
          </Field>
          <p className="text-xs text-muted-foreground">
            Write <code>{"${VAR}"}</code> where a value should come from a variable: Dots reads the
            variable from Dots' environment when the server starts, and never stores its value.
          </p>
          {stdio ? (
            <>
              <Field label="Command" htmlFor="server-command" error={shown.command}>
                <Input
                  id="server-command"
                  className="font-mono"
                  value={draft.command}
                  onChange={(e) => set({ command: e.target.value })}
                />
              </Field>
              <Field label="Arguments" htmlFor="server-args" hint="One argument per line.">
                <Textarea
                  id="server-args"
                  rows={3}
                  className="font-mono"
                  value={draft.args}
                  onChange={(e) => set({ args: e.target.value })}
                />
              </Field>
              <Field
                label="Working folder"
                htmlFor="server-cwd"
                hint="Empty: the folder the run works in."
              >
                <div className="flex gap-2">
                  <Input
                    id="server-cwd"
                    className="font-mono"
                    value={draft.cwd}
                    onChange={(e) => set({ cwd: e.target.value })}
                  />
                  <Button type="button" variant="outline" onClick={browse}>
                    Browse
                  </Button>
                </div>
              </Field>
              <Field
                label="Environment"
                htmlFor="server-env"
                error={shown.env}
                hint="One KEY=VALUE per line."
              >
                <Textarea
                  id="server-env"
                  rows={3}
                  className="font-mono"
                  value={draft.env}
                  onChange={(e) => set({ env: e.target.value })}
                />
              </Field>
            </>
          ) : (
            <>
              <Field label="URL" htmlFor="server-url" error={shown.url}>
                <Input
                  id="server-url"
                  className="font-mono"
                  placeholder="https://example.com/mcp"
                  value={draft.url}
                  onChange={(e) => set({ url: e.target.value })}
                />
              </Field>
              <Field
                label="Headers"
                htmlFor="server-headers"
                error={shown.headers}
                hint="One Name: value per line."
              >
                <Textarea
                  id="server-headers"
                  rows={3}
                  className="font-mono"
                  value={draft.headers}
                  onChange={(e) => set({ headers: e.target.value })}
                />
              </Field>
            </>
          )}
          {notSet.length > 0 && (
            <p className="text-xs text-amber-300">
              Not set in Dots' environment: {notSet.join(", ")}
            </p>
          )}
          {secretKeys(draft).map((key) => (
            <p key={key} className="text-xs text-amber-300">
              {key} looks like a secret. Write {"${VAR}"} instead of the value, so that the value
              is not stored.
            </p>
          ))}

          <div className="flex flex-col gap-2 rounded-md border p-3">
            <div className="flex items-center gap-3">
              <p className="flex-1 text-xs text-muted-foreground">
                {entry
                  ? "After a change, use Check now to list the tools again."
                  : "Test lists the tools the server offers. Saving then accepts that list."}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={probe.isPending}
                onClick={test}
              >
                {probe.isPending ? "Testing…" : "Test connection"}
              </Button>
            </div>
            {probe.isSuccess && (
              <>
                <p role="status" className="text-xs">
                  {tools.length === 1 ? "1 tool" : `${tools.length} tools`}
                  {entry ? "." : ". Saving accepts this list."}
                </p>
                <ToolList tools={tools} />
              </>
            )}
            {probe.isError &&
              (isDeclined(probe.error) ? (
                <p role="status" className="text-xs text-muted-foreground">
                  You declined to run it. Nothing was started.
                </p>
              ) : (
                <>
                  <p role="alert" className="text-xs text-destructive">
                    {errorMessage(probe.error)}
                  </p>
                  {!entry && (
                    <p className="text-xs text-muted-foreground">
                      You can still save: the server is saved without its tools, and its tools ask
                      until a check succeeds and you accept the list.
                    </p>
                  )}
                </>
              ))}
          </div>

          {save.isError &&
            (isDeclined(save.error) ? (
              <p role="status" className="text-xs text-muted-foreground">
                {entry
                  ? "You declined the change. Nothing was saved."
                  : "You declined to run it. Nothing was saved."}
              </p>
            ) : (
              <p role="alert" className="text-xs text-destructive">
                {errorMessage(save.error)}
              </p>
            ))}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button type="button" disabled={save.isPending} onClick={submit}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

**Create `ui/src/screens/library/McpServersTab.tsx`:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { CatalogStatus } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ItemRow, RemoveButton, UsedBy, type PluginEntry, type ServerEntry } from "./parts";
import { ServerForm } from "./ServerForm";

const STATUS: Record<CatalogStatus, { label: string; className: string }> = {
  accepted: { label: "Accepted", className: "bg-emerald-500/20 text-emerald-300" },
  changed: { label: "Tools changed", className: "bg-amber-500/20 text-amber-300" },
  unreachable: { label: "Unreachable", className: "bg-red-500/20 text-red-300" },
  unchecked: { label: "Unchecked", className: "bg-muted text-muted-foreground" },
};

/// An MCP server's status: its catalog's, or "Unchecked" when it has none.
export function serverStatus(entry: ServerEntry): CatalogStatus {
  return entry.catalog?.status ?? "unchecked";
}

function ServerRow(props: {
  entry: ServerEntry;
  owner: string | undefined;
  onEdit: () => void;
  onReview: () => void;
}) {
  const { entry } = props;
  const qc = useQueryClient();
  const check = useMutation({
    mutationFn: () => commands.libraryCheck(entry.id),
    onSuccess: () => invalidate(qc, keys.library),
    onError: notifyError,
  });
  const status = serverStatus(entry);
  const { label, className } = STATUS[status];
  const config = entry.config;
  return (
    <ItemRow
      label={entry.name}
      actions={
        <>
          {status === "changed" && (
            <Button size="sm" onClick={props.onReview}>
              Review
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={check.isPending}
            onClick={() => check.mutate()}
          >
            {check.isPending ? "Checking…" : "Check now"}
          </Button>
          {!props.owner && (
            <>
              <Button size="sm" variant="outline" onClick={props.onEdit}>
                Edit
              </Button>
              <RemoveButton entry={entry} />
            </>
          )}
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{entry.name}</span>
        <Badge variant="outline">{config.transport}</Badge>
        <Badge variant="secondary" className={className}>
          {label}
        </Badge>
        {entry.missing_variables.length > 0 && (
          <Badge variant="secondary" className="bg-amber-500/20 text-amber-300">
            {`Missing variable: ${entry.missing_variables.join(", ")}`}
          </Badge>
        )}
      </div>
      <span className="truncate font-mono text-xs text-muted-foreground">
        {config.transport === "stdio" ? [config.command, ...config.args].join(" ") : config.url}
      </span>
      {status === "unreachable" && entry.catalog?.error && (
        <span className="text-xs text-destructive">{entry.catalog.error}</span>
      )}
      {props.owner ? (
        <span className="text-xs text-muted-foreground">{`Comes with plugin ${props.owner}`}</span>
      ) : null}
      <UsedBy dots={entry.used_by} />
    </ItemRow>
  );
}

/// The MCP servers tab. `plugins` names the plugin a server comes with.
export function McpServersTab(props: {
  entries: ServerEntry[];
  plugins: PluginEntry[];
  onReview: (id: string) => void;
}) {
  const [form, setForm] = useState<"new" | ServerEntry | null>(null);
  const owners = new Map(props.plugins.map((p) => [p.id, p.name]));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Tools that dots can call. Each server's tool list is accepted by you and checked at
          every run.
        </p>
        <Button size="sm" onClick={() => setForm("new")}>
          <Plus aria-hidden /> Add server
        </Button>
      </div>
      {props.entries.length === 0 ? (
        <p className="text-muted-foreground">No MCP servers yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {props.entries.map((entry) => (
            <ServerRow
              key={entry.id}
              entry={entry}
              owner={entry.owner_id ? (owners.get(entry.owner_id) ?? "a plugin") : undefined}
              onEdit={() => setForm(entry)}
              onReview={() => props.onReview(entry.id)}
            />
          ))}
        </ul>
      )}
      {form && (
        <ServerForm
          key={form === "new" ? "new" : form.id}
          entry={form === "new" ? null : form}
          onClose={() => setForm(null)}
        />
      )}
    </div>
  );
}
```

**Create `ui/src/screens/library/SkillsTab.tsx`:**

```tsx
import { useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus } from "lucide-react";
import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { ScannedSkill } from "@/api/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { isDeclined, ItemRow, plural, RemoveButton, UsedBy, type SkillEntry } from "./parts";

type Scan = { folder: string; skills: ScannedSkill[] };

/// The skills found under a folder, to tick. A name the Library cannot take, or has already, is
/// shown with the reason and cannot be ticked.
function AddSkillsDialog(props: {
  scan: Scan;
  existing: Set<string>;
  onChoose: () => void;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { skills } = props.scan;
  const problem = (s: ScannedSkill) =>
    s.name_error ?? (props.existing.has(s.name) ? "Already in the Library." : null);
  const [unticked, setUnticked] = useState<Set<string>>(new Set());
  const chosen = skills.filter((s) => !problem(s) && !unticked.has(s.path));
  const [state, setState] = useState<{ busy: boolean; error?: string; declined?: boolean }>({
    busy: false,
  });

  // Another folder: what happened with the last one no longer applies.
  useEffect(() => setState({ busy: false }), [props.scan]);

  async function add() {
    setState({ busy: true });
    try {
      for (const skill of chosen) {
        await commands.libraryAdd(
          skill.name,
          { kind: "skill", config: { path: skill.path } },
          null,
        );
        // What was added stays added, whatever comes next.
        invalidate(qc, keys.library);
      }
      props.onClose();
    } catch (e) {
      setState(isDeclined(e) ? { busy: false, declined: true } : { busy: false, error: errorMessage(e) });
    }
  }

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add skills</DialogTitle>
          <DialogDescription className="font-mono">{props.scan.folder}</DialogDescription>
        </DialogHeader>
        {skills.length === 0 ? (
          <p className="text-muted-foreground">No SKILL.md found in that folder.</p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {skills.map((skill) => {
              const why = problem(skill);
              return (
                <li key={skill.path} className="flex items-start gap-2 rounded-md border p-2">
                  <input
                    type="checkbox"
                    className="mt-1"
                    aria-label={skill.name}
                    disabled={why !== null}
                    checked={why === null && !unticked.has(skill.path)}
                    onChange={(e) => {
                      const next = new Set(unticked);
                      if (e.target.checked) next.delete(skill.path);
                      else next.add(skill.path);
                      setUnticked(next);
                    }}
                  />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium">{skill.name}</span>
                    {skill.description && (
                      <span className="text-xs text-muted-foreground">{skill.description}</span>
                    )}
                    <span className="truncate font-mono text-xs text-muted-foreground">
                      {skill.path}
                    </span>
                    {why && <span className="text-xs text-destructive">{why}</span>}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        {state.error && (
          <p role="alert" className="text-xs text-destructive">
            {state.error}
          </p>
        )}
        {state.declined && (
          <p role="status" className="text-xs text-muted-foreground">
            You declined. Nothing more was added.
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onChoose}>
            Choose another folder
          </Button>
          {chosen.length > 0 && (
            <Button type="button" disabled={state.busy} onClick={() => void add()}>
              {`Add ${plural(chosen.length, "skill")}`}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The skills tab. "Add skill" picks a folder: one skill, or a folder with several below it.
export function SkillsTab({ entries }: { entries: SkillEntry[] }) {
  const [scan, setScan] = useState<Scan | null>(null);

  async function choose() {
    try {
      const folder = await open({ directory: true, title: "Choose a skill or a folder of skills" });
      if (typeof folder !== "string") return;
      setScan({ folder, skills: await commands.libraryScanSkills(folder) });
    } catch (e) {
      notifyError(e);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Folders with a SKILL.md. A dot that uses a skill can read it when the task calls for it.
        </p>
        <Button size="sm" onClick={() => void choose()}>
          <Plus aria-hidden /> Add skill
        </Button>
      </div>
      {entries.length === 0 ? (
        <p className="text-muted-foreground">No skills yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => (
            <ItemRow key={entry.id} label={entry.name} actions={<RemoveButton entry={entry} />}>
              <span className="font-medium">{entry.name}</span>
              {entry.skill?.description && (
                <span className="text-xs">{entry.skill.description}</span>
              )}
              <span className="truncate font-mono text-xs text-muted-foreground">
                {entry.config.path}
              </span>
              {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
              <UsedBy dots={entry.used_by} />
            </ItemRow>
          ))}
        </ul>
      )}
      {scan && (
        <AddSkillsDialog
          scan={scan}
          existing={new Set(entries.map((e) => e.name))}
          onChoose={() => void choose()}
          onClose={() => setScan(null)}
        />
      )}
    </div>
  );
}
```

**Create `ui/src/screens/library/PluginsTab.tsx`:**

```tsx
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { isDeclined, ItemRow, plural, RemoveButton, UsedBy, type PluginEntry } from "./parts";

const folderName = (path: string) =>
  path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";

/// A plugin and its parts, with the switch for its hooks. The core asks the person in a system
/// dialog before it lets the hooks run, so the switch asks nothing itself.
function PluginRow({ entry }: { entry: PluginEntry }) {
  const qc = useQueryClient();
  const hooks = useMutation({
    mutationFn: (run_hooks: boolean) =>
      commands.libraryUpdate(entry.id, {
        kind: "plugin",
        config: { ...entry.config, run_hooks },
      }),
    onSuccess: () => invalidate(qc, keys.library),
    onError: (e) => {
      if (!isDeclined(e)) notifyError(e);
    },
  });
  const detail = entry.plugin;
  const parts = detail
    ? [
        detail.skills.length > 0 && plural(detail.skills.length, "skill"),
        detail.commands.length > 0 && plural(detail.commands.length, "command"),
        detail.agents.length > 0 && plural(detail.agents.length, "agent"),
        detail.servers.length > 0 && plural(detail.servers.length, "MCP server"),
        detail.has_hooks && "hooks",
      ].filter((p): p is string => p !== false)
    : [];
  const switchId = `hooks-${entry.id}`;
  return (
    <ItemRow label={entry.name} actions={<RemoveButton entry={entry} />}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{entry.name}</span>
        {detail?.version && <span className="text-xs text-muted-foreground">{detail.version}</span>}
        {parts.map((part) => (
          <Badge key={part} variant="secondary">
            {part}
          </Badge>
        ))}
      </div>
      {detail?.description && <span className="text-xs">{detail.description}</span>}
      <span className="truncate font-mono text-xs text-muted-foreground">{entry.config.path}</span>
      {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
      {detail?.warnings.map((w) => (
        <span key={w} className="text-xs text-amber-300">
          {w}
        </span>
      ))}
      {(detail?.has_hooks || entry.config.run_hooks) && (
        <div className="flex items-center gap-2 pt-1">
          <Switch
            id={switchId}
            checked={entry.config.run_hooks}
            disabled={hooks.isPending}
            onCheckedChange={(on) => hooks.mutate(on)}
          />
          <Label htmlFor={switchId}>Run this plugin's hooks</Label>
        </div>
      )}
      {hooks.isError && isDeclined(hooks.error) && (
        <span role="status" className="text-xs text-muted-foreground">
          You declined: the hooks stay off.
        </span>
      )}
      <span className="text-xs text-muted-foreground">
        Commands, agents and hooks apply to Claude-engine dots only.
      </span>
      <UsedBy dots={entry.used_by} />
    </ItemRow>
  );
}

function AddPluginDialog(props: { existing: PluginEntry[]; onClose: () => void }) {
  const qc = useQueryClient();
  const installed = useQuery({
    queryKey: keys.installedPlugins,
    queryFn: commands.installedClaudePlugins,
  });
  const [folder, setFolder] = useState<{ path: string; name: string } | null>(null);
  const add = useMutation({
    mutationFn: (v: { name: string; path: string; installed_id: string | null }) =>
      commands.libraryAdd(
        v.name,
        { kind: "plugin", config: { path: v.path, installed_id: v.installed_id, run_hooks: false } },
        null,
      ),
    onSuccess: () => {
      invalidate(qc, keys.library);
      props.onClose();
    },
  });
  function choose() {
    open({ directory: true, title: "Choose a plugin folder" }).then((path) => {
      if (typeof path === "string") setFolder({ path, name: folderName(path) });
    }, notifyError);
  }
  const have = new Set(props.existing.flatMap((p) => [p.name, p.config.installed_id ?? ""]));
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add plugin</DialogTitle>
          <DialogDescription>
            A plugin brings skills and MCP servers to every dot that uses it. Its commands, agents
            and hooks work in Claude-engine dots only.
          </DialogDescription>
        </DialogHeader>
        <section className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold uppercase text-muted-foreground">
            Installed in Claude Code
          </h3>
          {installed.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(installed.error)}
            </p>
          )}
          {installed.data?.length === 0 && (
            <p className="text-xs text-muted-foreground">No installed plugins found.</p>
          )}
          <ul className="flex flex-col gap-1.5">
            {installed.data?.map((p) => (
              <li
                key={p.id}
                aria-label={p.id}
                className="flex items-center gap-3 rounded-md border p-2"
              >
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="font-medium">
                    {p.name}
                    {p.version && (
                      <span className="ml-2 text-xs text-muted-foreground">{p.version}</span>
                    )}
                  </span>
                  <span className="truncate font-mono text-xs text-muted-foreground">
                    {p.install_path}
                  </span>
                  {!p.exists && <span className="text-xs text-destructive">folder not found</span>}
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!p.exists || add.isPending || have.has(p.id) || have.has(p.name)}
                  onClick={() => add.mutate({ name: p.name, path: p.install_path, installed_id: p.id })}
                >
                  {have.has(p.id) || have.has(p.name) ? "Added" : "Add"}
                </Button>
              </li>
            ))}
          </ul>
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold uppercase text-muted-foreground">From a folder</h3>
          {folder ? (
            <>
              <span className="truncate font-mono text-xs">{folder.path}</span>
              <Field
                label="Name"
                htmlFor="plugin-name"
                hint="The Library name. Its MCP servers are named plugin_<name>_<server>."
              >
                <Input
                  id="plugin-name"
                  value={folder.name}
                  onChange={(e) => setFolder({ ...folder, name: e.target.value })}
                />
              </Field>
              <div>
                <Button
                  size="sm"
                  disabled={add.isPending || folder.name.trim() === ""}
                  onClick={() =>
                    add.mutate({ name: folder.name.trim(), path: folder.path, installed_id: null })
                  }
                >
                  Add from folder
                </Button>
              </div>
            </>
          ) : null}
          <div>
            <Button size="sm" variant="outline" onClick={choose}>
              {folder ? "Choose another folder" : "Choose a folder"}
            </Button>
          </div>
        </section>
        {add.isError &&
          (isDeclined(add.error) ? (
            <p role="status" className="text-xs text-muted-foreground">
              You declined: the plugin was not added.
            </p>
          ) : (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(add.error)}
            </p>
          ))}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The plugins tab.
export function PluginsTab({ entries }: { entries: PluginEntry[] }) {
  const [adding, setAdding] = useState(false);
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Plugin folders: Claude Code plugins you installed, or any folder with the same layout.
        </p>
        <Button size="sm" onClick={() => setAdding(true)}>
          <Plus aria-hidden /> Add plugin
        </Button>
      </div>
      {entries.length === 0 ? (
        <p className="text-muted-foreground">No plugins yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => (
            <PluginRow key={entry.id} entry={entry} />
          ))}
        </ul>
      )}
      {adding && <AddPluginDialog existing={entries} onClose={() => setAdding(false)} />}
    </div>
  );
}
```

**Create `ui/src/screens/library/LibraryScreen.tsx`:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useSearchParams } from "react-router";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { keys } from "@/lib/query";

import { McpServersTab, serverStatus } from "./McpServersTab";
import type { PluginEntry, ServerEntry, SkillEntry } from "./parts";
import { PluginsTab } from "./PluginsTab";
import { SkillsTab } from "./SkillsTab";
import { ToolsReview } from "./ToolsReview";

/// The Library: the MCP servers, skills and plugins that dots can use. `?review=<server id>`
/// opens the review of that server's changed tool list (the Inbox links to it).
export function LibraryScreen() {
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const [params, setParams] = useSearchParams();
  const [tab, setTab] = useState("servers");

  if (library.isPending) return <Page title="Library" />;
  if (library.isError) {
    return (
      <Page title="Library">
        <ErrorNote error={library.error} />
      </Page>
    );
  }
  const entries = library.data;
  const servers = entries.filter((e): e is ServerEntry => e.kind === "mcp_server");
  const skills = entries.filter((e): e is SkillEntry => e.kind === "skill");
  const plugins = entries.filter((e): e is PluginEntry => e.kind === "plugin");

  const reviewId = params.get("review");
  const reviewing = servers.find((s) => s.id === reviewId && serverStatus(s) === "changed");
  const review = (id: string | null) =>
    setParams(id ? { review: id } : {}, { replace: true });

  return (
    <Page title="Library">
      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="servers">MCP servers</TabsTrigger>
          <TabsTrigger value="skills">Skills</TabsTrigger>
          <TabsTrigger value="plugins">Plugins</TabsTrigger>
        </TabsList>
        <TabsContent value="servers">
          <McpServersTab entries={servers} plugins={plugins} onReview={review} />
        </TabsContent>
        <TabsContent value="skills">
          <SkillsTab entries={skills} />
        </TabsContent>
        <TabsContent value="plugins">
          <PluginsTab entries={plugins} />
        </TabsContent>
      </Tabs>
      {reviewing && <ToolsReview entry={reviewing} onClose={() => review(null)} />}
    </Page>
  );
}
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  "Retrying at 09:04 (attempt 2 of 6)" under it while a retry waits; a dot whose schedule
  needs attention carries a "Needs attention" badge.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode;
  schedule (from Scheduler v2: a kind picker None / Once / Every / Daily / Weekly / Cron with
```

**with:**

```markdown
  "Retrying at 09:04 (attempt 2 of 6)" under it while a retry waits; a dot whose schedule
  needs attention carries a "Needs attention" badge.
- **Library** — (MCP/skills spec §8) between Dots and Inbox, with three tabs. MCP servers:
  name, transport, a status badge (Accepted, Tools changed, Unreachable, Unchecked) and, apart
  from it, the `${VAR}` the environment of Dots does not set; the dots that use it; "Check
  now", "Edit", "Remove"; "Review" for a changed tool list (also reached by
  `/library?review=<server id>`, which the Inbox links to) showing the tools added, removed and
  changed, with "Accept". "Add server" is a form (name; stdio: command, arguments, working
  folder, env; http: URL, headers) with the `${VAR}` hint, the variables that are not set, a
  warning for a pasted secret and "Test connection", which lists the tools with a "read-only"
  badge; saving after a test accepts the list that was shown, and a server that could not be
  tested is saved unchecked. A plugin's own servers are listed with their plugin and have no
  Edit or Remove. Skills: name, description, folder, dots; "Add skill" picks a folder, scans it
  and lists every `SKILL.md` below it to tick. Plugins: name, folder, parts, the dots that use
  it, the switch "Run this plugin's hooks"; "Add plugin" picks one of the installed Claude
  Code plugins or a folder. Removing asks first and names the dots. The screen never asks a
  second time for what the core confirms in a native dialog (a program to run, env changes, a
  plugin's servers and hooks); a refusal (`cancelled`) shows as a calm note, not as an error.
- **Dot editor** — name; instructions (markdown); engine, model, endpoint; workdir + mode;
  schedule (from Scheduler v2: a kind picker None / Once / Every / Daily / Weekly / Cron with
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  - Bun as package manager; `@tauri-apps/cli` as a dev dependency.
- Look: one dark theme (no light mode), shadcn/ui's neutral palette, compact desktop-tool
  density, a left sidebar for Dots, Inbox (with its count), Chat and Settings.
- Data layer:
  - `ui/src/api/types.ts` (strict zod schemas mirroring the Rust JSON), `commands.ts` (typed
```

**with:**

```markdown
  - Bun as package manager; `@tauri-apps/cli` as a dev dependency.
- Look: one dark theme (no light mode), shadcn/ui's neutral palette, compact desktop-tool
  density, a left sidebar for Dots, Library, Inbox (with its count), Chat and Settings.
- Data layer:
  - `ui/src/api/types.ts` (strict zod schemas mirroring the Rust JSON), `commands.ts` (typed
```

- [ ] **Step 4: Run the tests**

```bash
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/screens/library src/app 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: both Library files and the `Layout` tests pass.

- [ ] **Step 5: Run everything**

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: UI 342 passed in 23 files, typecheck clean. No Rust file changes: `dots-core` 578 and `dots-app` 57 as after Task 8.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-10-08-dots-design.md ui/src/app/Layout.tsx ui/src/app/routes.tsx ui/src/screens/library/LibraryScreen.test.tsx ui/src/screens/library/LibraryScreen.tsx ui/src/screens/library/McpServersTab.tsx ui/src/screens/library/PluginsTab.tsx ui/src/screens/library/ServerForm.tsx ui/src/screens/library/SkillsTab.tsx ui/src/screens/library/ToolsReview.tsx ui/src/screens/library/parts.tsx ui/src/screens/library/text.ts ui/src/screens/library/tools.test.ts ui/src/screens/library/tools.ts ui/src/test/fixtures.ts
git commit -m "feat(ui): add the Library screen for MCP servers, skills and plugins"
```

### Task 10: Choose a dot's tools and skills, list changed tools in the Inbox, name MCP calls in the run view

The last screens of spec section 8. The dot editor gets the section "Tools and skills" between Permissions and Limits: three checklists of the Library's own items (a plugin's servers come with the plugin and are not listed), a note on a server whose tools changed or that was unreachable, a name the Library no longer has kept ticked and marked so that it can be unticked, for a local dot the plugin parts that do not apply, and the variables the ticked servers need that Dots' environment does not set. The Inbox gets the group "Tools changed": the server, the dots that use it, and "Review", which opens the Library's review. The run view names an MCP call "<server> › <tool>", a plugin's server by its plugin, and shows a run notice as a note. The main spec's section 7 follows.

**Files:**
- Create: `ui/src/lib/mcp-tool.test.ts`, `ui/src/lib/mcp-tool.ts`, `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`, `ui/src/screens/dot-editor/ToolsAndSkills.tsx`
- Modify: `docs/superpowers/specs/2026-10-08-dots-design.md`, `ui/src/components/Transcript.tsx`, `ui/src/lib/transcript.ts`, `ui/src/screens/InboxScreen.tsx`, `ui/src/screens/dot-editor/DotForm.tsx`
- Test: `ui/src/lib/mcp-tool.test.ts`, `ui/src/lib/transcript.test.ts`, `ui/src/screens/DotEditorScreen.test.tsx`, `ui/src/screens/InboxScreen.test.tsx`, `ui/src/screens/RunScreen.test.tsx`, `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`

**Interfaces:**
- Consumes: Task 8's `commands.libraryList` and `keys.library`, Task 5's `Inbox.tools_changed` and `notice` events, Task 9's route `library?review=<server id>`; `LibraryEntry.missing_variables`, `used_by` and `plugin`.
- Produces:
  - `@/lib/mcp-tool`: `parseMcpTool(name: string): { server: string; tool: string } | null`; `mcpToolLabel(name: string, owners: ReadonlyMap<string, string>): string` ("salesforce › query"; a plugin's server, found in `owners`, is "pdf-tools › page"; another name is returned as it is).
  - `TranscriptItem` gains `{ type: "note"; seq: number; text: string }`; a `notice` event maps to it (it was a raw "Note" item). `Transcript` shows a note as `<p role="note">` and a tool call's name through `mcpToolLabel`, with the full name in `title`; the owners come from the Library list query.
  - `ToolsAndSkills(props: { uses: Uses; engine: EngineKind; onChange: (uses: Uses) => void })`: fieldsets "MCP servers", "Skills", "Plugins", a checkbox per item named after it; "Nothing here yet. Add some in the Library." with a link; for a local dot "On a local dot, <p> gives its skills and MCP servers only. …"; a `role="status"` line "Not set in Dots' environment: …". `DotForm` places it between Permissions and Limits and edits `spec.uses`.
  - `InboxScreen`: a section "Tools changed", one row per server with "Used by a, b" or "No dot uses it yet." and a link "Review" to `/library?review=<server_id>`; the Inbox is empty only when `tools_changed` is empty too.

- [ ] **Step 1: Write the failing tests**

**Create `ui/src/lib/mcp-tool.test.ts`:**

```ts
import { describe, expect, it } from "vitest";

import { mcpToolLabel, parseMcpTool } from "@/lib/mcp-tool";

describe("MCP tool names", () => {
  it("splits mcp__<server>__<tool>", () => {
    expect(parseMcpTool("mcp__salesforce__query")).toEqual({ server: "salesforce", tool: "query" });
    // a tool name may hold a double underscore
    expect(parseMcpTool("mcp__github__list__issues")).toEqual({ server: "github", tool: "list__issues" });
  });

  it("leaves other names alone", () => {
    for (const name of ["Bash", "mcp__", "mcp__x", "mcp____t", "mcp__x__", "skill"]) {
      expect(parseMcpTool(name)).toBeNull();
    }
  });

  it("shows server and tool, or the plugin that owns the server", () => {
    const owners = new Map([["plugin_pdf-tools_render", "pdf-tools"]]);
    expect(mcpToolLabel("mcp__salesforce__query", owners)).toBe("salesforce › query");
    expect(mcpToolLabel("mcp__plugin_pdf-tools_render__page", owners)).toBe("pdf-tools › page");
    expect(mcpToolLabel("Bash", owners)).toBe("Bash");
  });
});
```

**In `ui/src/lib/transcript.test.ts`, replace:**

```ts
    const items = transcriptItems([ev({ kind: "notice", text: "skill pdf: folder not found" })]);
    expect(items).toEqual([
      { type: "raw", seq: expect.any(Number), label: "Note", text: "skill pdf: folder not found" },
    ]);
  });
```

**with:**

```ts
    const items = transcriptItems([ev({ kind: "notice", text: "skill pdf: folder not found" })]);
    expect(items).toEqual([
      { type: "note", seq: expect.any(Number), text: "skill pdf: folder not found" },
    ]);
  });
```

**In `ui/src/screens/DotEditorScreen.test.tsx`, replace:**

```tsx
  });

  it("deletes the dot only after a confirmation", async () => {
    stored();
```

**with:**

```tsx
  });

  it("shows what the dot uses in Tools and skills and saves what was changed there", async () => {
    stored();
    backend.on("library_list", () => sample.library);
    const { user } = renderApp(path);
    const section = await screen.findByRole("group", { name: "Tools and skills" });
    const ticked = (name: string) => within(section).findByLabelText(name);
    for (const name of [...dot.uses.mcp_servers, ...dot.uses.skills, ...dot.uses.plugins]) {
      expect(await ticked(name)).toBeChecked();
    }
    await user.click(await ticked("pdf"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")[0]).toMatchObject({
      spec: { uses: { ...dot.uses, skills: dot.uses.skills.filter((n) => n !== "pdf") } },
    });
  });

  it("deletes the dot only after a confirmation", async () => {
    stored();
```

**Append to `ui/src/screens/InboxScreen.test.tsx`:**

```tsx

describe("the Inbox, tools changed", () => {
  const [changed] = sample.inbox.tools_changed;

  it("lists a server whose tools changed with the dots that use it and a link to its review", async () => {
    inbox({ approvals: [], review_runs: [], attention: [], tools_changed: [changed!] });
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "Tools changed" });
    const row = within(group).getByRole("listitem", { name: `${changed!.name} tools changed` });
    expect(row).toHaveTextContent(changed!.dots.map((d) => d.name).join(", "));
    expect(within(row).getByRole("link", { name: "Review" })).toHaveAttribute(
      "href",
      `/library?review=${changed!.server_id}`,
    );
    expect(screen.queryByText(/Nothing waits for you/)).toBeNull();
  });

  it("says when no dot uses the server", async () => {
    inbox({
      approvals: [],
      review_runs: [],
      attention: [],
      tools_changed: [{ ...changed!, dots: [] }],
    });
    renderApp("/inbox");
    expect(await screen.findByText("No dot uses it yet.")).toBeVisible();
  });

  it("opens the review in the Library", async () => {
    inbox({ approvals: [], review_runs: [], attention: [], tools_changed: [changed!] });
    backend.on("library_list", () => sample.library);
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("link", { name: "Review" }));
    expect(await screen.findByRole("dialog", { name: /Tools changed: github/ })).toBeVisible();
  });
});
```

**In `ui/src/screens/RunScreen.test.tsx`, replace:**

```tsx
    expect(screen.queryByText("hidden")).toBeNull();
    expect(screen.getByText("Unparsed output").closest("details")).not.toHaveAttribute("open");
  });

```

**with:**

```tsx
    expect(screen.queryByText("hidden")).toBeNull();
    expect(screen.getByText("Unparsed output").closest("details")).not.toHaveAttribute("open");
  });

  it("names an MCP call by its server and tool, a plugin's server by its plugin", async () => {
    backend.on("library_list", () => sample.library);
    const [, pluginServer] = sample.library;
    thread(
      [run],
      [
        ev({ kind: "tool_call", id: "m1", tool: "mcp__github__search_issues", input: { query: "bug" } }),
        ev({ kind: "tool_result", id: "m1", output: "3 issues", is_error: false }),
        ev({ kind: "tool_call", id: "m2", tool: `mcp__${pluginServer!.name}__page`, input: { n: 1 } }),
        ev({ kind: "tool_call", id: "m3", tool: "mcp__unknown__ping", input: {} }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("github › search_issues")).toBeVisible();
    expect(await screen.findByText("pdf-tools › page")).toBeVisible();
    expect(screen.getByText("unknown › ping")).toBeVisible();
    expect(screen.queryByText("mcp__github__search_issues")).toBeNull();
  });

  it("reads a notice as a note, not as a raw item", async () => {
    thread([run], [ev({ kind: "notice", text: "MCP server github: its tools changed, they ask first" })]);
    renderApp(`/runs/${run.id}`);
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("MCP server github: its tools changed, they ask first");
    expect(screen.queryByText("Note")?.closest("details")).toBeFalsy();
  });

```

**Create `ui/src/screens/dot-editor/ToolsAndSkills.test.tsx`:**

```tsx
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import type { EngineKind, LibraryEntry, Uses } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

const [github, pluginServer, pdf, pdfTools] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [] };

function setup(opts: { uses?: Uses; engine?: EngineKind; library?: LibraryEntry[] } = {}) {
  backend.on("library_list", () => opts.library ?? sample.library);
  let latest = opts.uses ?? none;
  function Host() {
    const [uses, setUses] = useState(opts.uses ?? none);
    latest = uses;
    return <ToolsAndSkills uses={uses} engine={opts.engine ?? "claude"} onChange={setUses} />;
  }
  render(
    <QueryClientProvider client={createQueryClient()}>
      <MemoryRouter>
        <Host />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { user: userEvent.setup(), uses: () => latest };
}

describe("the editor's Tools and skills section", () => {
  it("lists the Library in three checklists, without the servers a plugin brings", async () => {
    setup();
    const servers = await screen.findByRole("group", { name: "MCP servers" });
    expect(within(servers).getAllByRole("checkbox").map((c) => c.getAttribute("aria-label"))).toEqual([
      github!.name,
    ]);
    expect(within(screen.getByRole("group", { name: "Skills" })).getByLabelText("pdf")).not.toBeChecked();
    expect(within(screen.getByRole("group", { name: "Plugins" })).getByLabelText("pdf-tools")).not.toBeChecked();
  });

  it("ticks and unticks, and hands back the names", async () => {
    const { user, uses } = setup({ uses: { ...none, skills: ["pdf"] } });
    expect(await screen.findByLabelText("pdf")).toBeChecked();
    await user.click(screen.getByLabelText(github!.name));
    await user.click(screen.getByLabelText("pdf-tools"));
    await user.click(screen.getByLabelText("pdf"));
    expect(uses()).toEqual({ mcp_servers: ["github"], skills: [], plugins: ["pdf-tools"] });
  });

  it("keeps a name the Library no longer has, so that it can be unticked", async () => {
    const { user, uses } = setup({ uses: { ...none, skills: ["gone"] } });
    const box = await screen.findByLabelText("gone");
    expect(box).toBeChecked();
    expect(screen.getByText("Not in the Library any more.")).toBeVisible();
    await user.click(box);
    expect(uses().skills).toEqual([]);
  });

  it("says where the Library is empty", async () => {
    setup({ library: [] });
    expect(await screen.findAllByText(/Nothing here yet/)).toHaveLength(3);
    expect(screen.getAllByRole("link", { name: "Library" })[0]).toHaveAttribute("href", "/library");
  });

  it("lists the variables the chosen servers and plugins need and the environment lacks", async () => {
    const needy = { ...pluginServer!, missing_variables: ["PDF_KEY"] } as LibraryEntry;
    const { user } = setup({ library: [github!, needy, pdf!, pdfTools!] });
    await screen.findByLabelText("github");
    expect(screen.queryByText(/Not set in Dots' environment/)).toBeNull();

    // github needs a variable only once it is ticked
    await user.click(screen.getByLabelText("github"));
    expect(screen.getByRole("status")).toHaveTextContent("Not set in Dots' environment: " + github!.missing_variables.join(", "));
    // a plugin's own server counts when its plugin is ticked
    await user.click(screen.getByLabelText("pdf-tools"));
    expect(screen.getByRole("status")).toHaveTextContent("PDF_KEY");
  });

  it("warns about a server whose tools wait for a review", async () => {
    setup();
    const row = (await screen.findByLabelText(github!.name)).closest("li")!;
    expect(row).toHaveTextContent("Tools changed");
  });

  it("tells a local dot which parts of a ticked plugin do not apply", async () => {
    const { user } = setup({ engine: "local" });
    await screen.findByLabelText("pdf-tools");
    expect(screen.queryByText(/do not apply/)).toBeNull();
    await user.click(screen.getByLabelText("pdf-tools"));
    expect(screen.getByText(/1 command, 1 agent and its hooks do not apply/)).toBeVisible();
  });

  it("says nothing of the kind to a Claude dot", async () => {
    setup({ engine: "claude", uses: { ...none, plugins: ["pdf-tools"] } });
    await screen.findByLabelText("pdf-tools");
    expect(screen.queryByText(/do not apply/)).toBeNull();
  });
});
```

- [ ] **Step 2: Run them to verify they fail**

```bash
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/mcp-tool.test.ts src/lib/transcript.test.ts src/screens/DotEditorScreen.test.tsx src/screens/InboxScreen.test.tsx src/screens/RunScreen.test.tsx src/screens/dot-editor/ToolsAndSkills.test.tsx 2>&1 | grep -E "Tests |Test Files|× |FAIL ")
```

Expected: `Test Files  6 failed (6)`, `Tests  7 failed | 106 passed (113)`: `mcp-tool.test.ts` and `ToolsAndSkills.test.tsx` fail as files (their modules do not exist), and "shows a notice about the run as a note", "names an MCP call by its server and tool, a plugin's server by its plugin", "reads a notice as a note, not as a raw item", the three tests of "the Inbox, tools changed" and "shows what the dot uses in Tools and skills and saves what was changed there" fail. A first run right after a build may report `Timeout waiting for worker to respond` and no tests: run it again.

- [ ] **Step 3: Implement**

**Create `ui/src/lib/mcp-tool.ts`:**

```ts
// MCP tool names as the engines report them: `mcp__<server>__<tool>`.

/// The server and tool of an MCP tool name, or null for any other name. A server name never
/// holds `__`, so the first `__` after the prefix ends it; the tool may hold more.
export function parseMcpTool(name: string): { server: string; tool: string } | null {
  const match = /^mcp__(.+?)__(.+)$/.exec(name);
  return match ? { server: match[1]!, tool: match[2]! } : null;
}

/// "salesforce › query" for `mcp__salesforce__query`. A server that comes with a plugin
/// (`owners`: server name to plugin name) shows as its plugin. Another name stays as it is.
export function mcpToolLabel(name: string, owners: ReadonlyMap<string, string>): string {
  const parsed = parseMcpTool(name);
  if (!parsed) return name;
  return `${owners.get(parsed.server) ?? parsed.server} › ${parsed.tool}`;
}
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
    }
  | { type: "error"; seq: number; text: string }
  /// Something the transcript does not render as conversation; shown collapsed.
  | { type: "raw"; seq: number; label: string; text: string };
```

**with:**

```ts
    }
  | { type: "error"; seq: number; text: string }
  /// What Dots says about the run itself: an item left out, a server held.
  | { type: "note"; seq: number; text: string }
  /// Something the transcript does not render as conversation; shown collapsed.
  | { type: "raw"; seq: number; label: string; text: string };
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
        break;
      case "notice":
        items.push({ type: "raw", seq: record.seq, label: "Note", text: event.text });
        break;
      case "session_started":
```

**with:**

```ts
        break;
      case "notice":
        items.push({ type: "note", seq: record.seq, text: event.text });
        break;
      case "session_started":
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
import type { ReactNode } from "react";
import { Link } from "react-router";
import { z } from "zod";

import { LIMITS, type Run, type RunEventRecord } from "@/api/types";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
import { isActive } from "@/lib/chat-threads";
import { localDateTime } from "@/lib/format";
import { toolPreview, transcriptItems, type TranscriptItem } from "@/lib/transcript";

```

**with:**

```tsx
import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Info } from "lucide-react";
import { Link } from "react-router";
import { z } from "zod";

import { commands } from "@/api/commands";
import { LIMITS, type LibraryEntry, type Run, type RunEventRecord } from "@/api/types";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
import { isActive } from "@/lib/chat-threads";
import { localDateTime } from "@/lib/format";
import { mcpToolLabel } from "@/lib/mcp-tool";
import { keys } from "@/lib/query";
import { toolPreview, transcriptItems, type TranscriptItem } from "@/lib/transcript";

```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
/// `active`: the item's run is still going. Only then can a tool call without a result be one
/// that runs; in a run that has ended, the result never came.
function Item({ item, active }: { item: TranscriptItem; active: boolean }) {
  switch (item.type) {
    case "text":
```

**with:**

```tsx
/// `active`: the item's run is still going. Only then can a tool call without a result be one
/// that runs; in a run that has ended, the result never came.
function Item(props: {
  item: TranscriptItem;
  active: boolean;
  /// MCP server name to the plugin it comes with.
  owners: ReadonlyMap<string, string>;
}) {
  const { item, active, owners } = props;
  switch (item.type) {
    case "text":
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
      return (
        <p role="alert" className="rounded-md border border-destructive/50 p-2 text-destructive">
          {item.text}
        </p>
```

**with:**

```tsx
      return (
        <p role="alert" className="rounded-md border border-destructive/50 p-2 text-destructive">
          {item.text}
        </p>
      );
    case "note":
      return (
        <p role="note" className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          {item.text}
        </p>
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
        <details className="rounded-md border px-2 py-1">
          <summary className="flex cursor-pointer items-center gap-2 text-xs">
            <span className="font-semibold">{item.tool}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
              {toolPreview(item.input)}
```

**with:**

```tsx
        <details className="rounded-md border px-2 py-1">
          <summary className="flex cursor-pointer items-center gap-2 text-xs">
            <span className="font-semibold" title={item.tool}>
              {mcpToolLabel(item.tool, owners)}
            </span>
            <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
              {toolPreview(item.input)}
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
}

/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: {
```

**with:**

```tsx
}

/// Server name to plugin name, for the servers that come with a plugin.
function pluginOwners(entries: LibraryEntry[]): Map<string, string> {
  const names = new Map(entries.filter((e) => e.kind === "plugin").map((e) => [e.id, e.name]));
  const owners = new Map<string, string>();
  for (const entry of entries) {
    const plugin = entry.owner_id === null ? undefined : names.get(entry.owner_id);
    if (entry.kind === "mcp_server" && plugin) owners.set(entry.name, plugin);
  }
  return owners;
}

/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: {
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
}) {
  const many = props.runs.length > 1;
  return (
    <div className="flex flex-col gap-4">
```

**with:**

```tsx
}) {
  const many = props.runs.length > 1;
  // A plugin's MCP server shows as its plugin; the Library tells which server comes from which.
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const owners = pluginOwners(library.data ?? []);
  return (
    <div className="flex flex-col gap-4">
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
              )}
              {items.map((item) => (
                <Item key={item.seq} item={item} active={isActive(run)} />
              ))}
              {items.length === 0 && message === null && run.status === "queued" && (
```

**with:**

```tsx
              )}
              {items.map((item) => (
                <Item key={item.seq} item={item} active={isActive(run)} owners={owners} />
              ))}
              {items.length === 0 && message === null && run.status === "queued" && (
```

**Create `ui/src/screens/dot-editor/ToolsAndSkills.tsx`:**

```tsx
import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { EngineKind, LibraryEntry, LibraryKind, Uses } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { keys } from "@/lib/query";
import { plural } from "@/screens/library/parts";

const GROUPS: { kind: LibraryKind; key: keyof Uses; title: string }[] = [
  { kind: "mcp_server", key: "mcp_servers", title: "MCP servers" },
  { kind: "skill", key: "skills", title: "Skills" },
  { kind: "plugin", key: "plugins", title: "Plugins" },
];

/// The parts of a plugin that a local dot does not use, as a phrase ("1 command, 2 agents and
/// its hooks"), or null when it has none.
function unusedParts(entry: LibraryEntry): string | null {
  const plugin = entry.plugin;
  if (!plugin) return null;
  const parts = [
    plugin.commands.length > 0 && plural(plugin.commands.length, "command"),
    plugin.agents.length > 0 && plural(plugin.agents.length, "agent"),
    plugin.has_hooks && "its hooks",
  ].filter((part): part is string => part !== false);
  if (parts.length < 2) return parts[0] ?? null;
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/// What a row says besides the name.
function Detail({ entry }: { entry: LibraryEntry }) {
  const text =
    entry.kind === "skill"
      ? entry.skill?.description
      : entry.kind === "plugin"
        ? entry.plugin?.description
        : null;
  const status = entry.catalog?.status;
  return (
    <>
      {text && <span className="text-xs text-muted-foreground">{text}</span>}
      {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
      {status === "changed" && (
        <span className="text-xs text-amber-700 dark:text-amber-400">
          Tools changed: they ask first until you review them.
        </span>
      )}
      {status === "unreachable" && (
        <span className="text-xs text-amber-700 dark:text-amber-400">
          Unreachable at the last check.
        </span>
      )}
    </>
  );
}

/// The dot editor's "Tools and skills" section: which Library items the dot uses, as three
/// checklists. A plugin's own MCP servers are not listed: they come with the plugin.
export function ToolsAndSkills(props: {
  uses: Uses;
  engine: EngineKind;
  onChange: (uses: Uses) => void;
}) {
  const { uses } = props;
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  if (library.isPending) return <p className="text-muted-foreground">Loading the Library…</p>;
  if (library.isError) return <ErrorNote error={library.error} />;
  const entries = library.data;

  function toggle(key: keyof Uses, name: string, on: boolean) {
    const rest = uses[key].filter((n) => n !== name);
    props.onChange({ ...uses, [key]: on ? [...rest, name] : rest });
  }

  // The variables the dot's servers need, its plugins' servers included.
  const usedPlugins = entries.filter((e) => e.kind === "plugin" && uses.plugins.includes(e.name));
  const missing = [
    ...new Set(
      entries
        .filter(
          (e) =>
            e.kind === "mcp_server" &&
            (e.owner_id === null
              ? uses.mcp_servers.includes(e.name)
              : usedPlugins.some((p) => p.id === e.owner_id)),
        )
        .flatMap((e) => e.missing_variables),
    ),
  ];
  const notes =
    props.engine === "local"
      ? usedPlugins.flatMap((p) => {
          const parts = unusedParts(p);
          return parts
            ? [
                `On a local dot, ${p.name} gives its skills and MCP servers only. Its ${parts} do not apply.`,
              ]
            : [];
        })
      : [];

  return (
    <div className="flex flex-col gap-4">
      {GROUPS.map(({ kind, key, title }) => {
        const items = entries.filter((e) => e.kind === kind && e.owner_id === null);
        const known = new Set(items.map((e) => e.name));
        const stale = uses[key].filter((name) => !known.has(name));
        return (
          <fieldset key={kind} aria-label={title} className="flex flex-col gap-1.5">
            <legend className="mb-1 text-xs font-medium text-muted-foreground">{title}</legend>
            {items.length === 0 && stale.length === 0 && (
              <p className="text-xs text-muted-foreground">
                Nothing here yet. Add some in the <Link to="/library" className="underline">Library</Link>.
              </p>
            )}
            <ul className="flex flex-col gap-1.5">
              {items.map((entry) => (
                <li key={entry.id} className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    className="mt-1"
                    aria-label={entry.name}
                    checked={uses[key].includes(entry.name)}
                    onChange={(e) => toggle(key, entry.name, e.target.checked)}
                  />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium" aria-hidden>
                      {entry.name}
                    </span>
                    <Detail entry={entry} />
                  </div>
                </li>
              ))}
              {stale.map((name) => (
                <li key={name} className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    className="mt-1"
                    aria-label={name}
                    checked
                    onChange={() => toggle(key, name, false)}
                  />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium" aria-hidden>
                      {name}
                    </span>
                    <span className="text-xs text-destructive">Not in the Library any more.</span>
                  </div>
                </li>
              ))}
            </ul>
          </fieldset>
        );
      })}
      {notes.map((note) => (
        <p key={note} className="text-xs text-muted-foreground">
          {note}
        </p>
      ))}
      {missing.length > 0 && (
        <p role="status" className="text-xs text-amber-700 dark:text-amber-400">
          Not set in Dots' environment: {missing.join(", ")}. A server that needs one cannot start
          until it is set.
        </p>
      )}
    </div>
  );
}
```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

```

**with:**

```tsx
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

```

**In `ui/src/screens/dot-editor/DotForm.tsx`, replace:**

```tsx
      </Section>

      <Section title="Limits">
        <NumberField
```

**with:**

```tsx
      </Section>

      <Section title="Tools and skills">
        <ToolsAndSkills
          uses={spec.uses}
          engine={spec.engine}
          onChange={(uses) => set("uses", uses)}
        />
      </Section>

      <Section title="Limits">
        <NumberField
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx

import { commands } from "@/api/commands";
import type { NeedsAttention, ReviewRun } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**with:**

```tsx

import { commands } from "@/api/commands";
import type { NeedsAttention, ReviewRun, ToolsChanged } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
      >
        Dismiss
      </Button>
    </li>
```

**with:**

```tsx
      >
        Dismiss
      </Button>
    </li>
  );
}

/// An MCP server whose tool list changed since it was accepted: its tools ask until a review.
function ToolsChangedRow({ item }: { item: ToolsChanged }) {
  return (
    <li
      aria-label={`${item.name} tools changed`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">{item.name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {item.dots.length === 0
            ? "No dot uses it yet."
            : `Used by ${item.dots.map((dot) => dot.name).join(", ")}`}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/library?review=${encodeURIComponent(item.server_id)}`}>Review</Link>
      </Button>
    </li>
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
    );
  }
  const { approvals, review_runs: runs, attention } = inbox.data;
  if (approvals.length === 0 && runs.length === 0 && attention.length === 0) {
    return (
      <Page title="Inbox">
```

**with:**

```tsx
    );
  }
  const { approvals, review_runs: runs, attention, tools_changed: tools } = inbox.data;
  if (
    approvals.length === 0 &&
    runs.length === 0 &&
    attention.length === 0 &&
    tools.length === 0
  ) {
    return (
      <Page title="Inbox">
```

**In `ui/src/screens/InboxScreen.tsx`, replace:**

```tsx
        </section>
      )}
      {runs.length > 0 && (
        <section aria-labelledby="inbox-runs" className="flex flex-col gap-2">
```

**with:**

```tsx
        </section>
      )}
      {tools.length > 0 && (
        <section aria-labelledby="inbox-tools" className="flex flex-col gap-2">
          <h2 id="inbox-tools" className="font-semibold">
            Tools changed
          </h2>
          <ul className="flex flex-col gap-2">
            {tools.map((item) => (
              <ToolsChangedRow key={item.server_id} item={item} />
            ))}
          </ul>
        </section>
      )}
      {runs.length > 0 && (
        <section aria-labelledby="inbox-runs" className="flex flex-col gap-2">
```

**In `docs/superpowers/specs/2026-10-08-dots-design.md`, replace:**

```markdown
  the core's objection under the fields); webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; use-user-settings toggle (Claude only).
  The dot's `uses` (Library items) is kept as stored when the form is saved.
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder. From Scheduler v2 a
  scheduled run's trigger has a line under it: "Scheduled for <slot>", "…, caught up after
  the PC was off", "Scheduled for <slot> · attempt 3 of 6" for a slot's run that carries the
```

**with:**

```markdown
  the core's objection under the fields); webhook URL + token (copy, regenerate);
  policy editor (preset picker + rule table); limits; use-user-settings toggle (Claude only).
  From MCP/skills a section "Tools and skills" (`screens/dot-editor/ToolsAndSkills.tsx`) edits
  the dot's `uses` as three checklists from the Library (MCP servers, skills, plugins; a
  plugin's own servers are not listed: they come with the plugin). A server whose tools changed
  or that was unreachable says so on its row. A name the Library no longer has stays ticked,
  marked, so that it can be unticked. For a local dot, a ticked plugin that has commands,
  agents or hooks says that they do not apply (only its skills and servers do). A note lists
  the `${VAR}` that the ticked servers, and the servers of ticked plugins, need and the
  environment of Dots does not set (the Library list's `missing_variables`).
- **Run view** — live transcript (assistant text, collapsible tool calls/results), worktree diff,
  summary, status, tokens; cancel, discard worktree, open folder. From MCP/skills an MCP call
  shows its server and tool ("salesforce › query" for `mcp__salesforce__query`; a server that
  comes with a plugin shows as the plugin, found through the Library list), and a `notice`
  run event reads as a note line, not as a collapsed raw item. From Scheduler v2 a
  scheduled run's trigger has a line under it: "Scheduled for <slot>", "…, caught up after
  the PC was off", "Scheduled for <slot> · attempt 3 of 6" for a slot's run that carries the
```

- [ ] **Step 4: Run the tests**

```bash
(cd ui && DEBUG_PRINT_LIMIT=0 bun x vitest run src/lib/mcp-tool.test.ts src/lib/transcript.test.ts src/screens/DotEditorScreen.test.tsx src/screens/InboxScreen.test.tsx src/screens/RunScreen.test.tsx src/screens/dot-editor/ToolsAndSkills.test.tsx 2>&1 | grep -E "Tests |Test Files|FAIL")
```

Expected: the six files pass.

- [ ] **Step 5: Run everything**

```bash
DEBUG_PRINT_LIMIT=0 bun run --cwd ui test
bun run --cwd ui typecheck
```

Expected: UI 359 passed in 25 files, typecheck clean. No Rust file changes: `dots-core` 578 and `dots-app` 57 as after Task 8.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-10-08-dots-design.md ui/src/components/Transcript.tsx ui/src/lib/mcp-tool.test.ts ui/src/lib/mcp-tool.ts ui/src/lib/transcript.test.ts ui/src/lib/transcript.ts ui/src/screens/DotEditorScreen.test.tsx ui/src/screens/InboxScreen.test.tsx ui/src/screens/InboxScreen.tsx ui/src/screens/RunScreen.test.tsx ui/src/screens/dot-editor/DotForm.tsx ui/src/screens/dot-editor/ToolsAndSkills.test.tsx ui/src/screens/dot-editor/ToolsAndSkills.tsx
git commit -m "feat(ui): choose a dot's tools and skills, list changed tools in the Inbox, name MCP calls in the run view"
```

### Task 11: The smoke test of MCP servers, skills and plugins

The spec asks for a smoke with LM Studio (section 11): a local dot with `fake-mcp`, where a read-only tool runs and a writing tool asks and is approved; a skill the model reads with `skill`; a plugin with a skill and a server; a changed tool list that holds the server and shows in the Inbox, then accepted. Its person steps run a real Claude-engine dot with a real plugin and an MCP server, because the agent never runs the real `claude` CLI. This task writes that as a document with the steps an agent runs from a terminal against a debug build (Part A, each with its result) and the steps for the person who owns the machine (Part B, a checklist). It follows the method of `docs/verification/2026-10-14-scheduler-v2-smoke.md`, including its first finding: `get_dot`, `create_dot` and `update_dot` answer the dot itself, with the spec's fields at the top level. It was not run while this plan was written.

**Files:**
- Create: `docs/verification/2026-10-15-mcp-skills-plugins-smoke.md`

**Interfaces:**
- Consumes: the app of Tasks 1 to 10; `fake-mcp` (Task 3), built into the shared target folder; `tools/smoke/webview.mjs` (`eval`, `text`, `goto`, `shot`, `click`, `fill`, `wait`); LM Studio's CLI at `~/.lmstudio/bin/lms.exe` with the model `google/gemma-4-12b-qat` downloaded.
- Produces: the document, with a result under every step of Part A.

What the steps rely on. A local dot's model gets the MCP tools under their full names (`mcp__facts__lookup`), the `skill` tool, and the skills `smoke-facts` and `smokeplug:greet` (a plugin's skill is named after its Library item). `approval_wait_secs: 0` parks a run at its first ask at once, and a decision starts the resume run. `fake-mcp` reads its script when it starts, so each run's server lists the file as it stands; a call of a tool scripted with `switch` changes the running server's list to `after_switch` and announces it. The model may not follow the instructions word for word: if it skips or reorders a call, run the step once more and write down what it did.

- [ ] **Step 1: Write the document's frame**

Create `docs/verification/2026-10-15-mcp-skills-plugins-smoke.md` with these sections, in this order: a title "MCP servers, skills and plugins: smoke test"; a short head (date, build, branch and commit, model, how many steps passed, where the screenshots are); "What this covers" (section 11 of `2026-10-15-mcp-skills-plugins-design.md`); "How the agent steps were run" (the debug build in the shared target folder, a fresh folder under `mktemp -d` as `DOTS_DATA_DIR`, `WEBVIEW2_USER_DATA_FOLDER` moved as well, the debugging port 9333, a `PATH` without the `claude` CLI, `DOTS_CONFIRM=yes` for the native dialogs, every block run in Git Bash from the repository root and starting with `. "${E:?}"`, no argument that starts with `/`, the installed Dots and the real `claude` CLI never touched, the model server never stopped); "Part A: agent steps"; "Findings of Part A"; "What Part A does not show"; "Part B: person steps". Each step of Part A has a heading, its block, an "Expect:" line and a "Result:" line that is filled in when the block has run.

- [ ] **Step 2: A1, nothing of ours is running; build the app and `fake-mcp`**

```bash
tasklist | grep -i -E 'fake-claude|fake-mcp' && echo "STOP: a fake CLI or server is running" || echo "no fake process running"
powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | ForEach-Object { \$_.ExecutablePath }"
bun install --frozen-lockfile 2>&1 | tail -1
CARGO_TARGET_DIR=C:/Users/aksha/dots/target bun run tauri build --debug --no-bundle 2>&1 | tail -2
CARGO_TARGET_DIR=C:/Users/aksha/dots/target cargo build -p dots-core --bin fake-mcp 2>&1 | tail -1
ls -la C:/Users/aksha/dots/target/debug/fake-mcp.exe
node --test tools/smoke/webview.test.mjs 2>&1 | grep -E '^ℹ (tests|pass|fail)'
```

Expect: no fake process; the only `dots-app.exe` that may be listed is the installed one under `AppData\Local\Dots`, which is left alone. If one under `target` is listed, stop: it blocks the build and is not ours to stop. The debug build finishes ("Built application at: C:\Users\aksha\dots\target\debug\dots-app.exe"), `fake-mcp.exe` exists, and the helper's tests pass.

- [ ] **Step 3: A2, temporary folders, a clean PATH, the state file**

`SHOTS` is a folder outside the repository, for the screenshots, the state file and the smoke's kit (A3), which Part B uses too.

```bash
: "${SHOTS:?set SHOTS to a folder outside the repository}"
mkdir -p "$SHOTS"; E="$SHOTS/env.sh"
SMOKE="$(mktemp -d)"
mkdir "$SMOKE/data" "$SMOKE/work"
printf 'hello\n' > "$SMOKE/work/README.md"
KIT="$SHOTS/kit"; rm -rf "$KIT"
mkdir -p "$KIT/logs" "$KIT/cli" "$KIT/skills/smoke-facts" "$KIT/plugins/smokeplug/.claude-plugin" "$KIT/plugins/smokeplug/skills/greet" "$KIT/plugins/smokeplug/commands"
# A PATH without any folder that holds the real claude CLI.
CLEAN_PATH=""; IFS=':' read -ra PARTS <<< "$PATH"
for p in "${PARTS[@]}"; do
  if [ -e "$p/claude" ] || [ -e "$p/claude.exe" ] || [ -e "$p/claude.cmd" ]; then echo "left out of PATH: $p"
  else CLEAN_PATH="$CLEAN_PATH${CLEAN_PATH:+:}$p"; fi
done
{
  echo 'set -u'
  printf 'SHOTS=%q\nSMOKE=%q\nE=%q\nCLEAN_PATH=%q\nKIT=%q\n' "$SHOTS" "$SMOKE" "$E" "$CLEAN_PATH" "$KIT"
  printf 'APP=%q\nWORK=%q\nKITM=%q\n' "C:/Users/aksha/dots/target/debug/dots-app.exe" "$(cygpath -m "$SMOKE/work")" "$(cygpath -m "$KIT")"
  printf 'FAKE=%q\n' "C:/Users/aksha/dots/target/debug/fake-mcp.exe"
} > "$E"
cat >> "$E" <<'EOF'
W="node tools/smoke/webview.mjs 9333"
LMS="$HOME/.lmstudio/bin/lms.exe"
val() { tr -d '"\n '; }
wait_page() { for _ in $(seq 1 60); do node tools/smoke/webview.mjs "$1" eval 'invoke("startup_error")' > /dev/null 2>&1 && break; sleep 1; done; sleep 1; }
# The process id of the app this smoke started: the one whose program is under target, not the installed one.
app_pid() { powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | Where-Object { \$_.ExecutablePath -like '*\\target\\debug\\*' } | ForEach-Object { \$_.ProcessId }" | tr -d '\r'; }
start_app() {
  ( PATH="$CLEAN_PATH" DOTS_DATA_DIR="$(cygpath -w "$SMOKE/data")" DOTS_CONFIRM=yes \
    WEBVIEW2_USER_DATA_FOLDER="$(cygpath -w "$SMOKE/webview")" \
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
    "$APP" > "$SMOKE/app.out" 2>&1 & )
  wait_page 9333
}
# Waits until a run has left `queued` and `running` and prints its status.
settled() { for _ in $(seq 1 150); do S=$($W eval "(await invoke('get_run', { id: '$1' })).status" | val); [ "$S" != "running" ] && [ "$S" != "queued" ] && break; sleep 2; done; echo "$S"; }
# The id and the catalog of a Library item, by name.
item_id() { $W eval "(await invoke('library_list')).find(e => e.name === '$1').id" | val; }
catalog() { $W eval "(await invoke('library_list')).find(e => e.name === '$1').catalog"; }
# A local dot that parks at its first ask. <name> <uses as a JS object> <instructions without quotes>
new_dot() { $W eval "(async () => { const spec = await invoke('new_dot_spec'); return (await invoke('create_dot', { spec: { ...spec, name: '$1', engine: 'local', model: 'google/gemma-4-12b-qat', endpoint_url: 'http://127.0.0.1:1234', workdir: '$WORK', workspace_mode: 'folder', approval_wait_secs: 0, uses: $2, instructions: '$3' } })).id; })()" | val; }
# Gives a dot new instructions (without quotes).
instruct() { $W eval "(async () => { const d = await invoke('get_dot', { id: '$1' }); return (await invoke('update_dot', { id: '$1', spec: { ...d, instructions: '$2' } })).instructions.length; })()"; }
# Starts a run of a dot and prints its id; the newest run of a dot.
run_now() { $W eval "(await invoke('run_now', { dotId: '$1' })).id" | val; }
newest() { $W eval "(await invoke('list_runs', { dotId: '$1', limit: 1 }))[0].id" | val; }
# The tool calls, results and notices of a run, in order.
calls() { $W eval "(await invoke('list_run_events', { runId: '$1', afterSeq: 0 })).filter(e => ['tool_call', 'tool_result', 'notice'].includes(e.kind)).map(e => e.kind === 'tool_call' ? ['call', e.data.tool, e.data.input] : e.kind === 'tool_result' ? ['result', e.data.is_error, e.data.output.slice(0, 160)] : ['notice', e.data.text])"; }
# What the model wrote in a run.
said() { $W eval "(await invoke('list_run_events', { runId: '$1', afterSeq: 0 })).filter(e => e.kind === 'assistant_text').map(e => e.data.text).join(' ').slice(0, 400)"; }
# The approvals of a run: id, tool, status, apply state, fingerprint.
approvals() { $W eval "(await invoke('approvals_for_run', { runId: '$1' })).map(a => [a.id, a.tool, a.status, a.apply_state, a.mcp_fingerprint])"; }
approve_first() { $W eval "(async () => { const a = (await invoke('approvals_for_run', { runId: '$1' }))[0]; return (await invoke('decide_approval', { id: a.id, approved: true, note: null, allowRule: null })).status; })()"; }
# How many tools/call messages a fake-mcp log holds for one tool. <log name> <tool>
served() { grep '"tools/call"' "$KIT/logs/$1.log" 2> /dev/null | grep -c -E "\"name\": ?\"$2\""; }
EOF
. "$E"; ls "$SMOKE"; PATH="$CLEAN_PATH" which claude 2> /dev/null || echo "no claude on the clean PATH"
```

Expect: the folders `data` and `work`; one or more "left out of PATH" lines when the real CLI is installed; `no claude on the clean PATH`.

- [ ] **Step 4: A3, the kit: two scripts of one server, a skill, a plugin, the CLI's config**

The server `facts` has a read-only tool `lookup`, a writing tool `save`, and a read-only tool `refresh` whose call switches the list: it adds a writing tool `erase`. `facts-a.json` is the list before the switch, `facts-b.json` the list after it, written from the same text, so that both give the same fingerprint as the switched server. The server runs from `facts.json`, a copy of one of them. The plugin `smokeplug` has a skill, a command and a server `notes` with one read-only tool.

```bash
. "${E:?}"
B_TOOLS='[
  {"name": "lookup", "description": "Look up a fact about a topic.", "inputSchema": {"type": "object", "properties": {"topic": {"type": "string"}}, "required": ["topic"]}, "annotations": {"readOnlyHint": true}, "result": {"content": [{"type": "text", "text": "The code word for dots is heron."}]}},
  {"name": "save", "description": "Save a note.", "inputSchema": {"type": "object", "properties": {"note": {"type": "string"}}, "required": ["note"]}},
  {"name": "refresh", "description": "Reload the facts.", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}},
  {"name": "erase", "description": "Erase every saved note.", "inputSchema": {"type": "object", "properties": {}}}
]'
cat > "$KIT/facts-a.json" <<EOF
{
  "log": "$KITM/logs/facts.log",
  "tools": [
    {"name": "lookup", "description": "Look up a fact about a topic.", "inputSchema": {"type": "object", "properties": {"topic": {"type": "string"}}, "required": ["topic"]}, "annotations": {"readOnlyHint": true}, "result": {"content": [{"type": "text", "text": "The code word for dots is heron."}]}},
    {"name": "save", "description": "Save a note.", "inputSchema": {"type": "object", "properties": {"note": {"type": "string"}}, "required": ["note"]}},
    {"name": "refresh", "description": "Reload the facts.", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}, "switch": true}
  ],
  "after_switch": $B_TOOLS
}
EOF
printf '{"log": "%s/logs/facts.log", "tools": %s}\n' "$KITM" "$B_TOOLS" > "$KIT/facts-b.json"
cp "$KIT/facts-a.json" "$KIT/facts.json"
cat > "$KIT/skills/smoke-facts/SKILL.md" <<'EOF'
---
name: smoke-facts
description: Facts for the smoke test of Dots. Read it when you are asked for the smoke code word.
---
The smoke code word is kestrel.

More is in notes.md in this skill's folder.
EOF
printf 'Nothing more to know.\n' > "$KIT/skills/smoke-facts/notes.md"
printf '{"name": "smoke-plugin", "version": "0.1.0", "description": "A plugin for the smoke test of Dots."}\n' > "$KIT/plugins/smokeplug/.claude-plugin/plugin.json"
cat > "$KIT/plugins/smokeplug/skills/greet/SKILL.md" <<'EOF'
---
name: greet
description: How to greet in the smoke test.
allowed-tools: Bash
---
Greet with the words: hello from the plugin.
EOF
cat > "$KIT/plugins/smokeplug/commands/hi.md" <<'EOF'
---
description: Say hi.
allowed-tools: Bash
---
Say hi.
EOF
cat > "$KIT/plugins/smokeplug/.mcp.json" <<EOF
{"mcpServers": {"notes": {"command": "$FAKE", "args": ["\${CLAUDE_PLUGIN_ROOT}/notes.json"]}}}
EOF
cat > "$KIT/plugins/smokeplug/notes.json" <<EOF
{"log": "$KITM/logs/notes.log", "tools": [{"name": "peek", "description": "Show the newest note.", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}, "result": {"content": [{"type": "text", "text": "The newest note says: plugin servers work."}]}}]}
EOF
cat > "$KIT/cli/facts.json" <<EOF
{"mcpServers": {"facts": {"type": "stdio", "command": "$FAKE", "args": ["$KITM/facts.json"]}}}
EOF
for f in facts-a.json facts-b.json plugins/smokeplug/.mcp.json plugins/smokeplug/notes.json cli/facts.json; do
  node -e "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8'))" "$KITM/$f" && echo "valid: $f"
done
grep -c CLAUDE_PLUGIN_ROOT "$KIT/plugins/smokeplug/.mcp.json"
```

Expect: five `valid:` lines, and `1`: the `.mcp.json` keeps `${CLAUDE_PLUGIN_ROOT}` for Dots to fill in.

- [ ] **Step 5: A4, the app starts on the temporary data folder; the model server**

```bash
. "${E:?}"
start_app
$W eval '(await invoke("status")).data_dir'
$W text | head -8
timeout 60 "$LMS" ps < /dev/null 2>&1 | grep -v '^$' | cut -c1-110
timeout 60 "$LMS" server status < /dev/null 2>&1 | tail -1
timeout 60 "$LMS" server start < /dev/null 2>&1 | tail -2
timeout 600 "$LMS" load google/gemma-4-12b-qat -c 8192 -y < /dev/null 2>&1 | tail -2
```

Expect: the data folder is the temporary one. The banner "The local server is not running: could not listen on 127.0.0.1:47321 …" is expected: the installed Dots holds that port, and local dots do not need it. "The claude CLI was not found" is expected too (the clean `PATH` worked). The sidebar lists Dots, Library, Inbox. The first `lms ps` and `server status` show what ran before this smoke: write it into the result, and at the end undo only what this smoke started. Then the server runs on port 1234 with the model loaded.

- [ ] **Step 6: A5, the Library: a server tested and added, a skill, a plugin and its server**

```bash
. "${E:?}"
CFG="{ transport: 'stdio', command: '$FAKE', args: ['$KITM/facts.json'], cwd: null, env: {} }"
$W eval "(async () => { const config = $CFG; const p = await invoke('library_probe', { config }); const item = await invoke('library_add', { name: 'facts', config: { kind: 'mcp_server', config }, accept: p.fingerprint }); return [p.tools.map(t => [t.name, t.annotations?.readOnlyHint === true]), p.fingerprint, item.name]; })()"
catalog facts | head -c 300; echo
$W eval "(await invoke('library_scan_skills', { path: '$KITM/skills' })).map(s => [s.name, s.description, s.name_error])"
$W eval "(await invoke('library_add', { name: 'smoke-facts', config: { kind: 'skill', config: { path: '$KITM/skills/smoke-facts' } }, accept: null })).name"
$W eval "(await invoke('library_add', { name: 'smokeplug', config: { kind: 'plugin', config: { path: '$KITM/plugins/smokeplug', installed_id: null, run_hooks: false } }, accept: null })).name"
FACTS=$(item_id facts); NOTES=$(item_id plugin_smokeplug_notes); printf 'FACTS=%s\nNOTES=%s\n' "$FACTS" "$NOTES" >> "$E"
$W eval "(async () => { const c = await invoke('library_check', { id: '$NOTES' }); const a = await invoke('library_accept', { id: '$NOTES', fingerprint: c.seen_fingerprint }); return [c.status, a.status]; })()"
$W eval "(await invoke('library_list')).map(e => [e.kind, e.name, e.catalog?.status ?? null, e.owner_id !== null, e.plugin ? [e.plugin.skills, e.plugin.commands, e.plugin.servers] : null, e.problem])"
$W eval "(await invoke('inbox')).tools_changed.length"
sleep 3; tasklist | grep -c -i fake-mcp
$W goto library; sleep 2; $W text | sed -n '/^Library$/,$p' | head -30; $W shot "$SHOTS/01-library-servers.png"
$W click "Plugins"; sleep 1; $W shot "$SHOTS/02-library-plugins.png"
```

Expect: the probe lists `lookup` (true), `save` (false) and `refresh` (true) with a fingerprint, and the add answers `facts`. The catalog of `facts` is `accepted`, its `accepted_fingerprint` equal to `seen_fingerprint` and to the probe's. The scan lists `smoke-facts` with its description and no name error. The skill and the plugin are added (the plugin's native question, "This plugin starts: …", is answered by `DOTS_CONFIRM=yes`). The plugin's server is named `plugin_smokeplug_notes`; its check answers `changed` (a server never accepted shows for review) and the accept `accepted`. The list: the server `facts` accepted, `plugin_smokeplug_notes` accepted and owned, the skill, and the plugin with the skill `smokeplug:greet`, the command `hi` and the server `plugin_smokeplug_notes`, no problem. `tools_changed` is 0. No `fake-mcp` process is left (the count is 0): the probe and the checks stop the servers they start. The screenshots show the MCP servers tab (two rows, "Accepted", "Comes with plugin smokeplug" on the second) and the Plugins tab (the parts, no hooks switch: the plugin has no hooks).

- [ ] **Step 7: A6, a read-only tool runs, a writing tool asks, and the approved call is carried out**

```bash
. "${E:?}"
MCP=$(new_dot mcp-dot "{ mcp_servers: ['facts'], skills: [], plugins: [] }" "Use your tools, one call at a time. First call mcp__facts__lookup with the topic dots. Then call mcp__facts__save with the note set to the code word that lookup gave you. If a call is queued for approval, stop and say so."); echo "MCP=$MCP" >> "$E"
R1=$(run_now "$MCP"); echo "R1=$R1" >> "$E"; echo "run: $(settled "$R1")"
calls "$R1"; approvals "$R1"
$W eval "(await invoke('library_list')).find(e => e.name === 'facts').catalog.accepted_fingerprint"
echo "served: lookup $(served facts lookup), save $(served facts save)"
$W goto inbox; sleep 2; $W text | sed -n '/^Inbox$/,$p' | head -16; $W shot "$SHOTS/03-ask.png"
approve_first "$R1"
sleep 3; R2=$(newest "$MCP"); echo "resume: $R2 $(settled "$R2")"
approvals "$R1"; calls "$R2"
echo "served: lookup $(served facts lookup), save $(served facts save)"
$W goto "runs/$R1"; sleep 2; $W text | grep -E 'facts' | head -6; $W shot "$SHOTS/04-run.png"
```

Expect: the first run ends `awaiting_approval`. Its calls: `mcp__facts__lookup` with `{"topic": "dots"}`, a result that is not an error, "The code word for dots is heron.", then `mcp__facts__save` with `{"note": "heron"}` (or the model's wording), which is not carried out: its result, if there is one, says that the call waits for approval. One approval: `mcp__facts__save`, `pending`, its `mcp_fingerprint` equal to the catalog's `accepted_fingerprint`. The server has served `lookup` once and `save` not at all. The Inbox shows the approval of `mcp-dot`. After the approval: the approval is `approved` with `apply_state` `applied`, the resume run ends `succeeded`, and the server has served `save` once. The run view names the calls "facts › lookup" and "facts › save".

- [ ] **Step 8: A7, skills and a plugin's server in a local run**

```bash
. "${E:?}"
KITDOT=$(new_dot kit-dot "{ mcp_servers: [], skills: ['smoke-facts'], plugins: ['smokeplug'] }" "Use your tools, one call at a time. First call the tool skill with the name smoke-facts. Then call the tool skill with the name smokeplug:greet. Then call mcp__plugin_smokeplug_notes__peek. Then answer in three lines: the smoke code word, the greeting, and the newest note."); echo "KITDOT=$KITDOT" >> "$E"
R3=$(run_now "$KITDOT"); echo "run: $(settled "$R3")"
calls "$R3"; approvals "$R3"; said "$R3"
echo "served: peek $(served notes peek)"
$W goto "runs/$R3"; sleep 2; $W shot "$SHOTS/05-skills.png"
$W goto "dots/$KITDOT"; sleep 2; $W text | sed -n '/^Tools and skills$/,/^Limits$/p'; $W shot "$SHOTS/06-editor.png"
```

Expect: the run ends `succeeded` with no approval. Its calls: `Skill` with `{"name": "smoke-facts"}`, whose result holds "The smoke code word is kestrel." and the list of the skill's files with `notes.md`; `Skill` with `{"name": "smokeplug:greet"}`, whose result holds "Greet with the words: hello from the plugin."; `mcp__plugin_smokeplug_notes__peek`, whose result is "The newest note says: plugin servers work.". The answer names kestrel. The plugin's server has served `peek` once. The run view names the last call "smokeplug › peek". The editor's "Tools and skills" has `smoke-facts` and `smokeplug` ticked, and the note that on a local dot `smokeplug` gives its skills and MCP servers only and its command does not apply.

- [ ] **Step 9: A8, the tool list changes during a run: the server is held and the Inbox shows it**

```bash
. "${E:?}"
instruct "$MCP" "Use your tools, one call at a time. First call mcp__facts__refresh. Then call mcp__facts__lookup with the topic dots. If a call is queued for approval, stop and say so."
R4=$(run_now "$MCP"); echo "R4=$R4" >> "$E"; echo "run: $(settled "$R4")"
calls "$R4"; approvals "$R4"
catalog facts | head -c 500; echo
$W eval "(await invoke('inbox')).tools_changed"; $W eval "await invoke('inbox_count')"
$W goto inbox; $W wait "Tools changed" 30; $W text | sed -n '/^Tools changed$/,$p' | head -8; $W shot "$SHOTS/07-tools-changed.png"
```

Expect: the run ends `awaiting_approval`. Its calls: `mcp__facts__refresh`, allowed, with the result "refresh: {}"; then `mcp__facts__lookup`, which now asks although it is read-only, because the server is held (its result, if there is one, says that the call waits for approval): one approval, `mcp__facts__lookup`, `pending`, with the fingerprint accepted before the change. The catalog of `facts` is `changed`, with a `seen_fingerprint` that differs from `accepted_fingerprint` and a `seen_tools` that has `erase`. The Inbox lists `facts` under `tools_changed` with the dot `mcp-dot`, and `inbox_count` counts it beside the approval. The Inbox page shows the group "Tools changed" with `facts`, "Used by mcp-dot" and "Review"; the sidebar count includes it.

- [ ] **Step 10: A9, the change is accepted in the Library; the approval given before it is not carried out**

```bash
. "${E:?}"
cp "$KIT/facts-b.json" "$KIT/facts.json"
$W goto "library?review=$FACTS"; $W wait "Tools changed: facts" 30; $W text | sed -n '/^Tools changed: facts$/,$p' | head -16; $W shot "$SHOTS/08-review.png"
$W click "Accept"; sleep 2
catalog facts | head -c 300; echo
$W eval "(await invoke('inbox')).tools_changed.length"
approve_first "$R4"
sleep 3; R5=$(newest "$MCP"); echo "resume: $R5 $(settled "$R5")"
approvals "$R4"; calls "$R5"; said "$R5"
```

Expect: the server now starts with the switched list, so a new run will not see a change again. The review "Tools changed: facts" lists `erase` under Added, and nothing removed or changed. After "Accept" the catalog is `accepted`, with `accepted_fingerprint` the fingerprint that was `seen_fingerprint`, and the Inbox has no `tools_changed` item. The approval of the held `lookup` is then approved, but the resume run does not carry it out: its `apply_state` is `skipped`, and the resume run's report to the model says that it was "not performed, because the tool changed since it was approved". The resume run ends (the model may call `lookup` again: it is now allowed without asking).

- [ ] **Step 11: A10, the run proceeds**

```bash
. "${E:?}"
R6=$(run_now "$MCP"); echo "run: $(settled "$R6")"
calls "$R6"; approvals "$R6"; said "$R6"
echo "served: lookup $(served facts lookup), refresh $(served facts refresh)"
$W goto "runs/$R6"; sleep 2; $W shot "$SHOTS/09-proceeds.png"
```

Expect: the run ends `succeeded` with no approval: `refresh` and `lookup` run without asking (the accepted list is the server's list, and both are read-only); `lookup` answers "The code word for dots is heron."; no notice says that the server is held.

- [ ] **Step 12: A11, stop and clean up**

```bash
. "${E:?}"
PID=$(app_pid); echo "stopping $PID"; [ -n "$PID" ] && taskkill //PID "$PID" //F
sleep 3; tasklist | grep -c -i fake-mcp
cp "$SMOKE"/data/logs/dots.*.log "$SHOTS/"
ls "$SMOKE/data/plugins" 2> /dev/null | grep -c '^run-'
[ -d "$SMOKE/data" ] && rm -rf "$SMOKE"; ls -d "$SMOKE" 2>&1 | sed 's#/tmp/tmp\.[A-Za-z0-9]*#$SMOKE#'
```

Then unload the model and stop the server only if step A4 showed that this smoke started them: `timeout 60 "$LMS" unload --all < /dev/null`, `timeout 60 "$LMS" server stop < /dev/null`.

Expect: the one process this smoke started is stopped; no `fake-mcp` process is left (0); no `run-*` folder is left under the data folder's `plugins` (0: local runs make none); the temporary folder is gone; the app's log is kept beside the screenshots, and the kit stays in `$SHOTS/kit` for Part B. Look at every screenshot and say in the results what it shows.

- [ ] **Step 13: Write Part B and the closing sections**

Part B is a checklist for the person who owns the machine, each line with "Result:" to fill in. The person runs the real `claude` CLI; the agent does not. In Git Bash, `. "$SHOTS/env.sh"` gives `KIT`, `KITM` and `FAKE`; each block runs in a fresh empty folder (`cd "$(mktemp -d)"`).

- B1. The CLI takes `mcp__<server>__*` in `--allowedTools`. With `fake-mcp` as a plain stdio server, a writing tool must run without a permission question:

  ```bash
  claude -p "Call the tool mcp__facts__save with the note test. Then say done." --output-format stream-json --verbose --strict-mcp-config --mcp-config "$KITM/cli/facts.json" --allowedTools "mcp__facts__*" --setting-sources "" > save.jsonl
  grep -o '"type":"tool_result"[^}]*' save.jsonl | head -3; grep '"tools/call"' "$KIT/logs/facts.log" | grep -c '"save"'
  ```

  Expected: a `tool_result` with "save: …" and no permission denial, and the count of `save` calls in the log one higher than before. A denial means the pattern is not honoured: Dots' `approve` then answers allow for the run's own servers and the gateway still asks once (Task 7), but the spec's section 5 should say so.
- B2. The CLI takes `--plugin-dir` more than once; which skills `--setting-sources ""` loads; the shape of `plugin_errors`. Make a broken plugin first: `mkdir -p "$KIT/plugins/broken/.claude-plugin" && printf '{ not json\n' > "$KIT/plugins/broken/.claude-plugin/plugin.json"`.

  ```bash
  claude -p "Say ok." --output-format stream-json --verbose --setting-sources "" --plugin-dir "$KITM/plugins/smokeplug" --plugin-dir "$KITM/plugins/broken" | head -1 > init.json
  cat init.json
  ```

  Record from this `system/init` line: the skills it lists (expected: the plugin's `greet`, under the manifest name `smoke-plugin`, since this is the original folder and not Dots' copy); whether any skill of `~/.claude/skills` is listed (the spec's open risk, section 12); and the exact `plugin_errors` entry of `broken`, to compare with what `parse.rs` reads (`plugin`, `name` or `source`, and `error` or `message`).
- B3. A Claude-engine dot through Dots' gateway. Start the debug build from Git Bash with the real `PATH`, a temporary data folder and real dialogs: `DOTS_DATA_DIR="$(cygpath -w "$(mktemp -d)")" C:/Users/aksha/dots/target/debug/dots-app.exe &`. If the banner says that port 47321 is taken (the installed Dots holds it), set the port to 47399 in Settings, save and "Restart Dots". In the Library add `facts` (command `$FAKE`, argument `$KITM/facts.json`; "Test connection", answer the Windows dialog, "Save"), the skill folder `$KITM/skills/smoke-facts` and the plugin folder `$KITM/plugins/smokeplug` (answer the dialog that names its server). Create a Claude dot in an empty folder that uses `facts`, `smoke-facts` and `smokeplug`, with the instructions "Call mcp__facts__lookup with the topic dots, then mcp__facts__save with the code word it gives. Then use the skill dots-skills:smoke-facts and the skill smokeplug:greet, and answer with the smoke code word and the greeting.", and press "Run now". Expected: the run view shows "facts › lookup" with "The code word for dots is heron." (the CLI accepted the gateway's plain JSON answers); `save` asks once in the Inbox (one approval, `mcp__facts__save`, not two); after "Approve" the resume run's retry goes through and `$KIT/logs/facts.log` has one more `save` call; the answer names kestrel and "hello from the plugin"; no "plugin …" note in the run view. If the CLI reports that the server `facts` failed, record its message from the run's raw events: the gateway's answers were refused.
- B4. `CLAUDE_CODE_PLUGIN_CACHE_DIR`. Find in Claude Code's documentation (or `claude plugin --help`) which folder the variable names, and whether `installed_plugins.json` is in that folder or in the folder above it. Dots reads it as the folder that holds `installed_plugins.json` (`kit::plugins_root`). In the B3 instance, "Add plugin" lists the installed Claude Code plugins: compare them with `claude plugin list`.

"Findings of Part A" lists every expectation that was not met, with what was seen. "What Part A does not show" names B1 to B4; an HTTP server (the tests of Task 3 cover it with `fake-mcp --http`); a server that cannot start or a variable that is not set (Task 5's `a_server_that_cannot_start_is_left_out_with_a_run_event`); a plugin's hooks; an installed plugin that moved; the upgrade of a database with per-dot MCP JSON (Task 2's `migration_0006_moves_each_dots_servers_into_the_library`); and anything a step could not reach.

- [ ] **Step 14: Commit**

```bash
git add docs/verification/2026-10-15-mcp-skills-plugins-smoke.md
git commit -m "docs(verification): the smoke of MCP servers, skills and plugins, agent steps run and person steps listed"
```

## After this plan

- Push `feat/mcp-skills` and open a pull request against `master`. Ask first how the branch is to be finished.
- Fold the results of Part B back: whether the CLI honours `mcp__<server>__*` in `--allowedTools` (else the `approve` passthrough is what keeps one question per call, and the spec's section 5 should say so); whether it takes `--plugin-dir` more than once; the real shape of a `plugin_errors` entry (and `parse.rs` if it differs); whether its Streamable HTTP client takes the gateway's plain JSON answers; whether `--setting-sources ""` loads `~/.claude/skills` (spec section 12); and which folder `CLAUDE_CODE_PLUGIN_CACHE_DIR` names (`kit::plugins_root` if it is not the folder of `installed_plugins.json`).
- Open, and not decided by this plan:
  - A plugin's `.mcp.json` that changes on disk after the plugin was added is not confirmed again. The plugin is read at every run start, its own servers are synced and started under the trust given when it was added (Task 8 asks only for a new plugin or a new path). A change of an owned server's config resets its catalog to `unchecked`, so its tools ask, but the new command line runs without a question.
  - A check whose fingerprint equals the accepted one accepts again after a config change (Task 5), where the spec's section 6 says that the next check asks. The command line itself is confirmed natively (Task 8). The spec should say which it means.
  - A server held in the middle of a run writes no run event (Task 5): the Library and the Inbox show it, the run view does not. A run event would need the watcher to reach the runner's recorder.
  - The gateway does not push `tools/list_changed` to the CLI (Task 7): a tool added during a Claude run is unknown to the model, though callable by name, and it asks.
  - The Library form's name check (`readDraft`) does not refuse `__` in a server name; the core does, so such a save fails with the core's message instead of the form's (Task 9).
  - Approval cards in the Inbox and the chat still show `mcp__<server>__<tool>`; only the run view shows "<server> › <tool>" (Task 10).
  - A resumed or chat follow-up run of a local dot keeps the skills section of its first run's system message, while the `skill` tool reads the current skills (Task 6).
  - A server that lists the same tool name twice is offered to the model once, the first (Task 6).
  - The frontmatter reader is a heuristic for "YAML that does not parse" (Task 4); a mismatch with Claude Code changes only what the local engine shows, never what the copies strip.
  - The MCP spec should take this plan's name rules (no `__` in a server name; skill and plugin names), the two-part migration, `EngineEvent::Notice` and `grants.mcp_fingerprint`.
- Follow-ups the MCP spec left out of scope (its section 1): MCP resources, prompts, sampling and elicitation; OAuth for HTTP servers; servers shared between runs; installing or updating plugins from Dots; a plugin's LSP servers, output styles, themes, workflows and monitors.
