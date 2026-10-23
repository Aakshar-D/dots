# Env vault smoke verification

- Date: 2026-10-22
- Build: debug (`bun run tauri build --debug --no-bundle` from `apps/dots-app`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/env-vault` at commit `26940d1`. Windows 11. The test suites were not run again by this task.
- Part A (agent steps): 11 steps (the brief's 7, with the controller's 8 to 11 added). All passed. Each ran once, in order, on one data folder.
- Part B (the owner): 3 steps, not run, for the owner.
- No secret value is written here. Every value was made up for the run (`dots-smoke-` and random hex), kept in the smoke folder and deleted with it.

## How the agent steps were run

- The app is the debug build, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d`, `DOTS_CONFIRM=yes`, `WEBVIEW2_USER_DATA_FOLDER` moved into the same folder, and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`. The real data folder `%LOCALAPPDATA%\dots` and the installed Dots (`%LOCALAPPDATA%\Dots`) were not touched.
- The `PATH` given to the app leaves out `...\AppData\Roaming\npm`, the only folder with a `claude` launcher. `which claude` found nothing with it. The real `claude` CLI was never run.
- The Claude program is a copy of `fake-claude.exe` (so that a later build is not blocked), saved in Settings as "Program path", then "Restart Dots".
- The window is driven with `node tools/smoke/webview.mjs 9333 click|fill|text|eval|goto|shot`; the Tauri commands are called with `invoke(...)` through `eval` where the step needs data the page does not show.
- The vault keeps `FAKE_CLAUDE_SCRIPT` and `FAKE_CLAUDE_ARGS_OUT` out of the fake CLI's environment (they are not in the base environment), so the smoke gives them to the fake CLI the way a person would: as plain Personal variables, granted to the dot's shell. That is itself a use of the shell grant.
- The MCP servers are `fake-mcp.exe` (its log file lists the `FAKE_MCP_*` variables it started with) and two small Node stdio servers written in the smoke folder (`leaky.mjs`, `leaky2.mjs`), which print their variable to stderr and in a tool result. `fake-mcp` has no way to print a variable, so they stand in for it where a step needs a printed value.
- The fake CLI calls the server's tool through the gateway (`mcp_call` in its script). The dot's policy asks about an MCP tool, so a script approves the run's pending approval.
- The values were looked for with `grep -a -c -F` over `dots.db`, `dots.db-wal` and the log files, and with Python `sqlite3` (read only) over `run_events` and `library_items`.

## Part A: agent steps

### A1. Env tab: a secret and a plain variable

Added `DOTS_SMOKE_TOKEN` as a secret and `LOG_LEVEL` as plain, in Library, Env, through the dialog.

Result: pass. The page showed the secret as "secret / Kept in Credential Manager" with "Replace value" and "Remove", and no value. `env_list` answered `value: null` for it and `debug` for `LOG_LEVEL`. `grep -c` for the value: `dots.db` 0, `dots.db-wal` 0, log files 0. `cmdkey /list:Dots/<install id>*` listed one target, `Dots/<install id>/personal/DOTS_SMOKE_TOKEN`.

### A2. A server that reads the variable, a dot that ticks it

Added `fake-mcp` (stdio, `fake-mcp.exe`, env `FAKE_MCP_TOKEN=${DOTS_SMOKE_TOKEN}`), then made a dot in the editor and ticked the server.

Result: pass. The dot's Variables section showed `fake-mcp`, then "DOTS_SMOKE_TOKEN · granted when you save". After Create dot, the dot's `variables` held `mcp:fake-mcp: [DOTS_SMOKE_TOKEN]`. A run started the server: its log had one `start` record with `FAKE_MCP_TOKEN` equal to the value (1 match by `grep -c`; the log was deleted first, so the match is this run's).

### A3. A second variable, not granted, then Grant

Changed the server to also read `FAKE_MCP_ORG=${DOTS_SMOKE_ORG}`. Ran: the run's first event was a notice "DOTS_SMOKE_ORG: not granted to mcp:fake-mcp in dot 'smoke-dot'", and the server did not start (no log file). In the dot's editor the Variables section said "fake-mcp now also wants DOTS_SMOKE_ORG" with a Grant button. Clicked **Grant**, then Save: `variables` held both names, and the section said "DOTS_SMOKE_ORG · not in Personal". Added `DOTS_SMOKE_ORG` as a secret in Env. Ran again: no notice, and the server's log had a `start` with both variables.

Result: pass.

### A4. The value printed by the server is scrubbed

Replaced the server by `leaky.mjs`, which prints its token on stderr at start and on each call, and returns "the token is <value>" from its tool. Ran, and approved the tool call.

Result: pass. The run's events were `tool_call mcp__fake-mcp__whoami`, `tool_result "the token is [secret:DOTS_SMOKE_TOKEN]"`, `finished`. The log lines were `MCP server stderr: leaky server starting with token [secret:DOTS_SMOKE_TOKEN]` and `called with token [secret:DOTS_SMOKE_TOKEN]`. `grep -c` for the value over `dots.db`, `dots.db-wal` and the log: 0 each.

### A5. A shell grant reaches the fake CLI; an environment variable of Dots does not

The app was killed and started again from the start script, which sets `DOTS_SMOKE_LEAK` (a made-up value) in its environment and leaves `LOG_LEVEL` unset. The dot had `LOG_LEVEL` granted to its shell. Ran, then read `env_names` in `FAKE_CLAUDE_ARGS_OUT`.

Result: pass. `LOG_LEVEL` was listed. `DOTS_SMOKE_LEAK`, `DOTS_SMOKE_TOKEN` and `DOTS_SMOKE_ORG` were not. The 48 names were the base environment, the two granted `FAKE_CLAUDE_*` names, `LOG_LEVEL`, and the CLI's own pass-through names (`ANTHROPIC_*`, `CLAUDE_CODE_*`, `MCP_TIMEOUT`, `MCP_TOOL_TIMEOUT`), which this smoke inherited from the shell it was started in.

### A6. Webhook token

In the dot's Webhook section, clicked **Make a token**.

Result: pass. The token (44 characters) was shown in a read-only field with "Copy it now: you won't see this again." After the dot was opened again the page text did not hold it and said it was shown once. `POST` to the dot's webhook URL: `Authorization: Bearer <token>` gave 202; `?token=<token>` gave 401; no token gave 401. `dots.webhook_token` holds `sha256:...`; `grep -c` for the token in `dots.db` and `dots.db-wal`: 0. The 202 started a run; it was approved and succeeded.

### A8 (I2). A secret-looking value over 2,560 bytes stays, and is scrubbed

Added `big-mcp` through the Tauri command the server form calls, with env `BIG_API_KEY=<3,015 bytes>`, and ticked it in the dot. Ran with `leaky2.mjs` printing the value; then restarted the app twice with `restart_app`.

Result: pass.
- The value stayed in the Library: `library_items.config` holds it (1 row); Personal gained no entry for it. The log says `big-mcp: a value of "BIG_API_KEY" is too long to move to the vault` at each start.
- The run's events show `tool_result "big is [secret:BIG_API_KEY]"`; the log shows `server starting with [secret:BIG_API_KEY]`. `run_events` rows holding the value: 0. Log files holding the value: 0.
- After the second restart a run still scrubbed it the same way (the redactor loads at start).
- Log lines with "vacuum" before the restarts and after each of them: 0. `settings` has no `vault_vacuum_pending` row.

### A9 (I3). Replacing the literal behind `${DOTS_SMOKE_TOKEN}` in the form

In the server form's Environment field, replaced `FAKE_MCP_TOKEN=${DOTS_SMOKE_TOKEN}` by `FAKE_MCP_TOKEN=<new value>` (a new made-up value). The form said "FAKE_MCP_TOKEN looks like a secret. When you save, Dots moves its value to Personal as a secret and the server reads it from there." Saved.

Result: pass. The server's config still reads `${DOTS_SMOKE_TOKEN}`. Env holds `DOTS_SMOKE_ORG`, `DOTS_SMOKE_TOKEN`, `FAKE_CLAUDE_ARGS_OUT`, `FAKE_CLAUDE_SCRIPT` and `LOG_LEVEL`: no `DOTS_SMOKE_TOKEN_2`. The dot's grants did not change. The next run's server received the new value (the server's own record of its variable matched the new value, not the old one). Neither value is in `dots.db`, its WAL or the log.

A plugin re-sync cannot be driven from the UI. It is covered by the Rust test `vault_upgrade_test::a_plugin_resync_with_another_value_keeps_the_name_and_the_grant` (final fix wave report, I3), which was not run again here.

### A10 (I1). A URL under a token-like key stays plain

Added `TOKEN_URL=https://example.invalid/token` to the server's Environment in the form.

Result: pass. The form showed no "looks like a secret" notice for it. After Save the server's config holds `TOKEN_URL` as `https://example.invalid/token`. Env gained nothing, and the dot's grants did not change.

### A11 (I5). A toolchain variable reaches the fake CLI

The start script sets `JAVA_HOME` (the Zulu JDK folder on this machine).

Result: pass. `env_names` in `FAKE_CLAUDE_ARGS_OUT` (A5's run) lists `JAVA_HOME`; it does not list `DOTS_SMOKE_LEAK`.

### A7. This file

Written from the notes above. It is committed alone.

## Cleanup

- The smoke app was stopped by name (`taskkill /IM dots-app.exe`) and `tasklist` showed no `dots-app.exe`, `fake-claude.exe` or `fake-mcp.exe`.
- Credential Manager: listed only `Dots/<install id>*` for this smoke's install id (`01M5JTFF5B06G5H8NKZ2X8BTBE`) and deleted each target by name: 2 entries (`personal/DOTS_SMOKE_TOKEN`, `personal/DOTS_SMOKE_ORG`). The same listing then showed 0. No other entry was listed, read or deleted.
- The smoke folder (data, webview data, the copies of the fake programs, the scripts and the made-up values) was deleted.

## Release note

Log files and run events written before the vault are not rewritten. A secret that a run printed with an older version stays in those old files. Delete the old logs after updating if that matters.

## Part B: for the owner (not run)

1. With the real `claude` CLI, logged in, run a dot. The CLI still runs with the base environment (the subscription login works), and a shell grant is visible to its Bash tool (`echo $LOG_LEVEL`).
2. On a copy of the real data folder (`DOTS_DATA_DIR` pointing at it) whose MCP servers have typed tokens: start Dots. The values move to Personal, the dots that use those servers still run, and the Env tab offers to import from Windows and to delete the backups made before the vault.
3. Record the outcome here.
