# MCP servers, skills and plugins for dots

Date: 2026-10-15. Status: design approved in conversation, written for review.
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md` (section 13 below lists the passages it changes). Source of the MCP trust idea: the review of Cloudflare OS (`packages/mcp-shared/src/tools.ts`), Approvals v2 spec section 14.

## 1. Why

Local-engine dots (LM Studio and other OpenAI-compatible endpoints) have seven built-in tools and nothing else. Claude-engine dots can use MCP servers, but only through a JSON box per dot, and none of the person's skills or plugins load, because Dots runs the CLI with `--setting-sources ""` and `--strict-mcp-config`.

This spec adds three things a dot can use, for both engines:

- **MCP servers**, with a trust boundary: a tool that declares itself read-only runs without asking, every other tool asks, the dot's own rules still win, and a server whose tool list changed is held until the person accepts the change.
- **Skills**, in Claude Code's format: a folder with a `SKILL.md`.
- **Plugins**, in Claude Code's format: a folder with skills, commands, agents, hooks and MCP servers.

Decisions taken with the person (2026-10-15):

| Question | Decision |
|---|---|
| What "skills" and "plugins" are | Claude Code's formats |
| Where they are added | A Library; each dot ticks which items it uses |
| A server's tool list changed | Hold the server: all its tools ask until the change is accepted |
| `readOnlyHint` | Allowed without asking; the rest asks; the dot's rules win |
| Secrets | From the environment only: Dots stores a variable name, never a value |
| The old per-dot MCP JSON box | Moved into the Library; inline values become variable names |
| A plugin's hooks | Left out unless the person allows them per plugin (Claude engine only) |
| Approach | A gateway: Dots runs every MCP server and checks every call, for both engines |

Out of scope: MCP resources, prompts, sampling and elicitation; OAuth for HTTP servers; servers shared between runs; installing or updating plugins from Dots (that stays in Claude Code); a plugin's LSP servers, output styles, themes, workflows and monitors.

## 2. Library items

A new table holds the Library. Each item has a kind, a name unique within its kind, and a config.

```sql
CREATE TABLE library_items (
  id         TEXT PRIMARY KEY,          -- ULID
  kind       TEXT NOT NULL,             -- 'mcp_server' | 'skill' | 'plugin'
  name       TEXT NOT NULL,
  config     TEXT NOT NULL,             -- JSON, per kind below
  owner_id   TEXT NULL REFERENCES library_items(id) ON DELETE CASCADE,  -- an MCP server of a plugin
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (kind, name)
);
```

### MCP server

```json
{ "transport": "stdio", "command": "npx", "args": ["-y", "some-server"], "cwd": null,
  "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly" } }
{ "transport": "http", "url": "https://example.com/mcp",
  "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" } }
```

- The name is the tool prefix: tools are `mcp__<name>__<tool>`. It matches `^[a-z0-9_-]{1,40}$`, has no `__` and neither starts nor ends with `_` (`mcp__files___x` would read as server `files`); `dots` is reserved.
- `${VAR}` in `env` values, `args`, `cwd`, `url` and `headers` is replaced, when the server starts, by the value of `VAR` in Dots' own environment. The value is never written to the database, an export, a log or a run event. A plain value with no `${…}` is stored as written; the editor warns when a key looks like a secret (`TOKEN`, `KEY`, `SECRET`, `PASSWORD`, `AUTH` in its name, or a header named `Authorization`) and the value has no `${…}`.
- A variable that is not set stops that server from starting (section 6).

### Skill

```json
{ "path": "C:\\Users\\me\\skills\\pdf" }
```

The folder holds `SKILL.md`. Dots reads its YAML frontmatter as Claude Code does: `name` (default: the folder name) and `description` (default: the first non-empty line of the body); other fields are kept for the Claude engine except `allowed-tools` (section 5). A frontmatter that does not parse leaves the skill with no fields, as in Claude Code. The Library item's name is the skill's name.

### Plugin

```json
{ "path": "C:\\Users\\me\\.claude\\plugins\\cache\\mkt\\pdf-tools\\1.2.0",
  "installed_id": "pdf-tools@mkt", "run_hooks": false }
```

- `path` is a plugin folder. `installed_id` is set when the plugin was picked from the person's installed Claude Code plugins (read from `~/.claude/plugins/installed_plugins.json`, or `CLAUDE_CODE_PLUGIN_CACHE_DIR`); its `installPath` may change on an update (section 7).
- Dots reads the plugin's parts: `.claude-plugin/plugin.json` (`name`; default the folder name), `skills/`, `commands/`, `agents/`, `hooks/hooks.json` and `.mcp.json`.
- Each server of `.mcp.json` becomes an MCP server item with `owner_id` set to the plugin and the name `plugin-<plugin>-<server>` (made valid as any server name). It is not `plugin_<plugin>_<server>`, the prefix Claude Code itself gives a plugin's servers: with the person's settings loaded and the same plugin enabled in Claude Code, the CLI may start that server itself, and its tools must not fall under the run's `--allowedTools mcp__<server>__*` or the gateway's pass in `approve`. `${CLAUDE_PLUGIN_ROOT}` in its config is replaced by the plugin's path. These items are refreshed whenever the plugin is read and deleted with it.
- `run_hooks` is off by default. Switching it on asks first: "This plugin's hooks run commands on their own, without approval, and can approve tool calls. Run them?" Hooks run only on Claude-engine dots.

### A dot's uses

`DotSpec` gains:

```json
"uses": { "mcp_servers": ["salesforce"], "skills": ["pdf"], "plugins": ["pdf-tools"] }
```

Names refer to Library items. A plugin's MCP servers come with the plugin; they are not ticked separately. `validate()` refuses a name that is not in the Library. Deleting a Library item that dots use asks first and names those dots; on delete, the name is removed from their `uses`.

## 3. Catalogs and fingerprints

```sql
CREATE TABLE mcp_catalogs (
  server_id            TEXT PRIMARY KEY REFERENCES library_items(id) ON DELETE CASCADE,
  accepted_fingerprint TEXT NULL,
  accepted_tools       TEXT NULL,       -- JSON: the accepted tools/list result
  seen_fingerprint     TEXT NULL,
  seen_tools           TEXT NULL,
  status               TEXT NOT NULL,   -- 'accepted' | 'changed' | 'unreachable' | 'unchecked'
  checked_at           TEXT NULL,
  error                TEXT NULL
);
```

- **Fingerprint:** SHA-256 over the tools sorted by name, each as canonical JSON (keys sorted, no whitespace) of `name`, `description`, `inputSchema` and `annotations`. The server's `title`, `outputSchema` and `_meta` are not part of it.
- **Adding a server** connects to it, lists its tools and shows them, with a "read-only" badge where `annotations.readOnlyHint === true`. Saving accepts that list: `accepted_*` = `seen_*`, status `accepted`. A server that cannot be reached can still be saved, as `unchecked`; its tools ask until a check succeeds and the person accepts the list.
- **Checks** happen at every run that uses the server (section 6) and on "Check now" in the Library. A different fingerprint sets `seen_*` and status `changed`; the server is held (section 4). A failed check sets `unreachable` and `error`, and keeps the accepted list.
- **Accepting** a change copies `seen_*` to `accepted_*` and sets `accepted`. The review shows tools added, removed and changed (a changed tool shows which of description, inputs or annotations changed).
- Every change of a catalog publishes `RuntimeEvent::LibraryUpdated`.

## 4. The check of a call

One function decides every MCP tool call, for both engines. The first match wins:

1. A `deny` rule of the dot's policy matching the tool: **deny**.
2. The server is held (status `changed` or `unchecked`), or the tool is not in the accepted list: **ask**.
3. An `allow` rule matching the tool: **allow**.
4. An `ask` rule matching the tool: **ask**.
5. The accepted list gives the tool `annotations.readOnlyHint: true`: **allow**.
6. Otherwise the policy's `mcp_default` action.

- `Policy` gains `mcp_default: Action`, used in step 6 for MCP tools instead of `default`. The presets set it to `ask` (Sandboxed, Custom, Trusted) and `deny` (Read-only). So under Trusted, whose `default` is `allow`, an MCP tool that is not read-only still asks; under Read-only it is denied.
- The Trusted preset's `mcp__*` ask rule is removed: as a rule it would come before step 5 and make read-only tools ask.
- Rules match `mcp__<server>__<tool>` as today (prefix patterns such as `mcp__sf__*`). The `readOnlyHint` used is the one in the accepted list, never one a server reports later.
- **Ask** goes through the existing approval queue (Approvals v2): the call waits up to `approval_wait_secs`, is then parked, and the run resumes when it is decided. The approval row stores the server's accepted fingerprint (`approvals.mcp_fingerprint`).
- **A decided call carried out later** (the local engine's approved calls on resume) or **retried** (the Claude engine after a one-shot grant) goes through only if the server's accepted fingerprint still equals the stored one; otherwise it is skipped with "the tool changed since it was approved".
- `allow_candidates` offers, for an MCP tool, the rule `mcp__<server>__<tool>` (every call of this tool) and `mcp__<server>__*` (every tool of this server).

- Dots' own `dots` MCP server serves `approve` and, from the context library and the check-ins spec, `context`, `notify`, `ask` and `follow_up`; it checks each call of its own tools against the dot's policy (`2026-10-20-checkins-followups-design.md` section 2).

## 5. The gateway

### MCP client

A new module `crates/dots-core/src/mcp/` holds a client written like the existing server side (`server/mcp.rs`): JSON-RPC 2.0 with `serde_json`, no new dependency.

- **stdio:** the server is a child process with its stdin and stdout piped, a hidden window, and a Windows Job Object (or a process group elsewhere), as `proc.rs` does for the shell tool. Messages are newline-delimited JSON. Its stderr goes to the app log, capped.
- **http:** Streamable HTTP over `reqwest`: each message is a POST with `Accept: application/json, text/event-stream`; a JSON or an SSE response is read; an `Mcp-Session-Id` header is kept and sent back.
- **Methods:** `initialize` (protocol version `2025-06-18`; the server's version is accepted when it is one the client knows), `notifications/initialized`, `tools/list` (following `nextCursor`), `tools/call`. Other requests from the server are answered with a JSON-RPC error; notifications are ignored, except `notifications/tools/list_changed`, which triggers a re-list (section 6).
- **Timeouts:** 30 s for `initialize` and `tools/list`; 300 s for a `tools/call` of the local engine; cancelling the run cancels the call at once.

### Per run

When a run starts, Dots opens a session with every MCP server the dot uses, its plugins' servers included: it starts or connects to the server, lists its tools, updates the catalog and fingerprint (section 3), and keeps the connection for the run. The sessions close when the run ends, and stdio servers are stopped.

### Local engine

The tools of every open session are added to the tools sent to the model, as OpenAI function definitions named `mcp__<server>__<tool>`, with the server's description and `inputSchema`. A name longer than 64 characters, the OpenAI limit, is cut to 55 characters plus `_` and the first 8 hex characters of its SHA-256; the map back is kept per run. A call is checked (section 4), emitted as a `ToolCall` event, sent, and its result reported like a built-in tool's: the `text` parts joined, other parts as `[image omitted]` or `[<type> omitted]`, `isError` as an error result, capped at 30 KB.

### Claude engine

- `--mcp-config` lists, besides the existing `dots` entry, one entry per open session: `{"type": "http", "url": "http://127.0.0.1:<port>/mcp/s/<server>", "headers": {"Authorization": "Bearer <run secret>"}}`. `--strict-mcp-config` stays.
- `--allowedTools mcp__<server>__*` is passed for each of them, so the CLI does not ask Dots' `approve` tool for these calls; the gateway decides instead. `approve` answers allow for a gateway tool; for any other `mcp__*` tool it is asked about (not `mcp__dots__*`) it applies the section 4 check with no trust, so only deny and allow rules and `mcp_default` decide, never `default`.
- The gateway route `POST /mcp/s/<server>` (loopback only, the run secret as today) answers `initialize`, `tools/list` from the run's open session, and `tools/call` by running the check of section 4, waiting for an approval if it asks, and forwarding the call to the server. A denied or parked call returns a tool result with `isError: true` and the same text the local engine gives.
- `MCP_TOOL_TIMEOUT` stays `(approval_wait_secs + 30) × 1000`.

## 6. Errors and edge cases

| Case | What happens |
|---|---|
| A server cannot start: a variable is not set, the command is not found, it exits | It is left out of the run; a run event "salesforce: could not start: SF_TOKEN is not set"; the catalog becomes `unreachable` with the error; the run goes on |
| A server does not answer `initialize` or `tools/list` within 30 s | Same as above, "did not answer" |
| The tool list differs from the accepted one at run start | Status `changed`; the server is held for this run and later ones; the Inbox shows it (section 8) |
| `notifications/tools/list_changed` during a run | The client lists again; a new fingerprint sets `changed` at once, so the next call asks. A call waits for that listing up to 30 s; if it is not done by then, that call asks as if the server were held |
| A server dies during a run | Its tools return "the server stopped"; no restart |
| A call is made to a tool the accepted list does not have | Step 2 of the check: ask |
| A skill or plugin folder is missing, or `SKILL.md` is missing | That item is left out, with a run event ("skill pdf: folder not found") |
| An installed plugin's `installPath` no longer exists | Dots looks up `installed_id` in `installed_plugins.json` again and stores the new path, with `run_hooks` off: the hooks the person allowed were those of the old folder. If they were on, the run gets the event "the plugin <name> moved, so its hooks were switched off; switch them on again in the Library" and runs without them. If there is no new path, the plugin is left out with an event |
| A plugin copy is larger than 50 MB | Left out, with an event |
| A dot uses an item that was deleted | Not possible: delete removes it from `uses` |
| The person changes a server's config | The catalog is reset to `unchecked` and the next check asks to accept. When an http server moves to another origin (scheme, host, port, as the HTTP client reaches them after `${VAR}` in the URL is expanded), the accepted list is dropped too, so the same tools at a new host are not accepted again silently |

## 7. Skills and plugins in each engine

### Claude engine

For each run Dots writes plugin folders under `<data_dir>/plugins/run-<run_id>/` and passes each with `--plugin-dir`:

- `dots-skills/`: a generated plugin (`.claude-plugin/plugin.json` with `"name": "dots-skills"`) whose `skills/` holds a copy of each standalone skill the dot uses. Its skills appear as `dots-skills:<name>`.
- One copy of each plugin the dot uses, without `.mcp.json` (its servers go through the gateway), without `.lsp.json` and the manifest's `lspServers` (Claude Code would start LSP servers itself), without `hooks/` unless `run_hooks` is on, and without `monitors/`. These names are matched as Windows matches them: without regard to case or a trailing `.` or space.
- In every copy (the skills of `dots-skills` too), a file or folder whose name ends in `.` or a space, or that has the same name as another entry of its folder without regard to case, is left out with a log line, on every platform: Windows would read and write another file under such a name (`hooks.` as `hooks`, `SKILL.md.` over the cleaned `SKILL.md`).
- In every markdown file of a copy (skills, commands, agents, in any folder the manifest names), Dots removes the `allowed-tools` field from the frontmatter, so nothing can let itself run a tool without Dots' check; it also removes `hooks` (unless `run_hooks`), `permissionMode` and `mcpServers`. Dots finds the frontmatter where Claude Code does (`---`, white space and a line break, up to the first `---` after it) and follows only one form of it: top-level `key: value` lines with indented continuation lines. A frontmatter in any other form (tags, anchors, aliases, merge keys, flow or explicit mappings, tabs, a lone carriage return, or one that never closes) is replaced by an empty one and the body kept, so nothing after it can become the frontmatter. The result is checked again until it is either empty or in that form without those fields.
- The folders are deleted when the run ends, as the MCP config file is today.
- Dots reads the CLI's `system/init` event: each entry of `plugin_errors` becomes a run event and a log line.
- `--setting-sources` is unchanged: empty by default, `user,project,local` with `use_user_settings`.

### Local engine

- The system prompt gains a section after the standing rules:

  ```
  Skills you can use. Call the tool `skill` with a name to read one before you follow it.
  - pdf: Extract text and tables from PDF files. …
  - pdf-tools:fill-form: …
  ```

  Each line is `name: description`, the description joined with `when_to_use` and cut at 1,536 characters. A plugin's skills are named `plugin:skill`.
- A new built-in tool `skill`, policy name `Skill`, with the arguments `name` (required) and `file` (optional). Without `file` it returns the `SKILL.md` body, with `${CLAUDE_SKILL_DIR}` replaced by the skill's folder, followed by the list of the skill's files. With `file` it returns that file; the path is resolved inside the skill folder and refused if it leaves it. Results are capped at 30 KB.
- `Skill` reads only Dots-chosen folders, so every preset allows it: the presets gain `Skill` **allow**.
- A skill's scripts run only through `shell`, under the normal rules. `allowed-tools` is ignored.
- A plugin on a local dot gives its skills and its MCP servers; its commands, agents and hooks do not apply, and the Library and the dot editor say so.

## 8. Screens

- **Library** (new sidebar item, between Dots and Inbox), with three tabs:
  - **MCP servers:** name, transport and status badge (Accepted, Tools changed, Unreachable, Missing variable, Unchecked), the dots that use it, "Check now". "Add server" opens a form (name; stdio: command, args, working folder, env; http: URL, headers; a hint that `${VAR}` reads Dots' environment and the variables that are not set). "Test connection" lists the tools, each with its description and a "read-only" badge. "Tools changed" opens the review with "Accept".
  - **Skills:** name, description, folder, the dots that use it. "Add skill" picks a folder; picking a folder that holds several skills lists every `SKILL.md` below it to tick.
  - **Plugins:** name, folder, its parts (skills, commands, agents, MCP servers, hooks), the "Run this plugin's hooks" switch (asks first), the dots that use it. "Add plugin" picks a folder or one of the installed Claude Code plugins.
- **Dot editor:** a section "Tools and skills" replaces the MCP JSON box, with three checklists (MCP servers, skills, plugins). For a local dot it notes which plugin parts do not apply. It lists variables that the chosen servers need and that are not set. The section "Claude Code" keeps the switch "Load my Claude Code user settings".
- **Inbox:** a group "Tools changed": the server, the dots that use it, and "Review" (opens the Library review). Its items count in the Inbox badge and the tray.
- **Run view:** MCP calls show as other tool calls, with the server and tool name; run events for items left out and for server problems.

## 9. Commands, events and contract

- Commands: `library_list`, `library_add(kind, name, config)`, `library_update(id, config)`, `library_remove(id)` (refused with the list of dots unless `confirm: true`), `library_probe(config)` (tests a server config without saving; returns the tools or the error), `library_check(id)`, `library_accept(id, fingerprint)` (refused if the seen fingerprint moved meanwhile), `library_scan_skills(path)`, `installed_claude_plugins()`, `environment_variables(names)` (says which of the names are set, never their values).
- Events: `RuntimeEvent::LibraryUpdated { item_id }`; the existing `ToolCall` and run events carry MCP calls and the new run events.
- `Inbox` gains `tools_changed: [{server_id, name, dots}]`; `inbox_count` counts it.
- `DotSpec.uses` replaces `DotSpec.mcp_servers` in the contract; the fixture is regenerated by its command.

## 10. Upgrade (migration 0006)

- Creates `library_items`, `mcp_catalogs` and `approvals.mcp_fingerprint`, and adds the column `dots.uses` (JSON, default `{"mcp_servers":[],"skills":[],"plugins":[]}`).
- Each server in a dot's `mcp_servers` JSON becomes an MCP server item (status `unchecked`), ticked in that dot's `uses`. Two dots with a server of the same name and the same config share one item; different configs get the names `<name>`, `<name>-2`, …, and the dot's rules naming `mcp__<name>__` are rewritten to the new name.
- Inline values are not copied when their key looks like a secret (section 2): an env value becomes `${<KEY>}`, an `Authorization` header becomes `<scheme> ${<SERVER>_TOKEN}` (upper case, `-` as `_`), where the scheme is the old value's first word when the value has more than one and that word reads as a scheme (a letter, then up to 23 letters, digits or `-`: `Basic`, `token`), else `Bearer`. The dot then lists the variables to set (section 8). Other values are copied.
- The column `dots.mcp_servers` is dropped.
- The Trusted preset rule `mcp__*` ask is removed from stored policies whose preset is `trusted`; every stored policy gets `mcp_default: "ask"` (`"deny"` for `read_only` and for a policy whose `default` is `deny`), and a `Skill` allow rule at the end when no rule of it names `Skill` (`Skill` or `Skill(...)`), as every preset has (section 7).

## 11. Testing

- A test binary `fake-mcp` (like `fake-claude`): stdio and http modes; tools scripted by a JSON file (names, descriptions, schemas, `readOnlyHint`, canned results, an error, a delay); a switch that changes its tool list and sends `notifications/tools/list_changed`; it records the calls it receives.
- Core tests: the check of section 4 in every order and preset; the fingerprint (canonical JSON, what is in and out); name shortening; `${VAR}` expansion and a missing variable; the client over both transports (initialize, paging, call, error, timeout, server death); per-run sessions and their cleanup; catalog states and accept; the approval fingerprint guard on resume and on retry; skill parsing (frontmatter, defaults, broken YAML), the `skill` tool and its path containment; plugin reading; the per-run plugin copies (no `.mcp.json`, hooks only with `run_hooks`, no `allowed-tools`); migration 0006 from a 0005 database with inline secrets.
- The Claude engine through `fake-claude`, which reads `--mcp-config` and `--plugin-dir` and calls a gateway tool.
- UI tests: the Library tabs, add and test a server, review and accept, the hooks switch, the editor's "Tools and skills" section, the Inbox group.
- Smoke (agent steps, LM Studio): a local dot with `fake-mcp` (a read-only tool runs, a writing tool asks and is approved), a skill the model reads with `skill`, a plugin with a skill and a server, a changed tool list that holds the server and shows in the Inbox, accept. Person steps: a real Claude-engine run with a real plugin and a real MCP server (the agent never runs the real `claude` CLI).

## 12. Open risks

- `--strict-mcp-config` with `--plugin-dir`: the docs do not say whether strict mode also drops a plugin's `.mcp.json` servers. Dots removes `.mcp.json` from its copies, so it does not depend on the answer.
- Whether `--setting-sources ""` stops the CLI from loading `~/.claude/skills` is not documented. Dots' own skills do not depend on it; the person step of the smoke checks which skills the `system/init` event lists.
- A server's `readOnlyHint` is its own claim. A server that lies is caught only by the person's review when it is added or changed.

## 13. Changes to the main spec

- Section 1, out of scope: remove "MCP servers for local-engine dots".
- Section 3: `dots.mcp_servers` replaced by `dots.uses`; the new tables.
- Section 4, the Claude engine's arguments: the gateway entries, `--allowedTools` for them, `--plugin-dir`.
- Section 5: the check of MCP calls, `mcp_default`, the presets' `Skill` allow and the Trusted change.
- Section 6: the gateway route.
- Section 7: the Library screen, the editor section, the Inbox group.
- Section 12: the dot editor's "Claude Code" section.
