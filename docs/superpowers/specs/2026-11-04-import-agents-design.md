# Import Claude Code agents as dots

- Date: 2026-11-04
- Status: approved (design decided by the owner in the request, 2026-11-04; builder's calls marked)
- Builds on: `2026-10-14` MCP/skills spec (kit, installed plugins), the dot editor's `create_dot`.

## 1. Goal

A person who already has Claude Code subagents (markdown files with a YAML frontmatter) can turn them into dots in a few clicks, instead of copying each one's prompt, model and tools into the New dot form by hand.

Success: from the Dots column, "Import from Claude Code" lists every agent found on this PC, grouped by where it lives, with what each would become; the person ticks some and imports them; each becomes a dot with the agent's prompt, model and a policy built from its tools.

## 2. Words

- **Agent file**: a `.md` file with a frontmatter of `name`, `description`, `tools` (a comma string or a YAML list), `model` (`sonnet`, `opus`, `haiku`, `inherit` or a full id), maybe `color` and others; the body is the system prompt.
- **Source**: where an agent file was found. `user` (`~/.claude/agents/**`), `project` (`<workdir>/.claude/agents/**` for the workdir of each existing dot), `plugin` (`agents/**` of each installed Claude Code plugin, labelled with the plugin's name).

## 3. Behaviour

1. **Scan** (`scan_claude_agents`) reads every source and returns one entry per file: source kind, plugin name or project folder, path, the dot name it would get, description, model, the listed tools, warnings, `already_imported`, and an `error` when the file cannot be imported. A file that cannot be read or has no prompt is listed with its reason, never an error for the whole scan.
2. **Import** (`import_claude_agents(paths, orgId, workspaceId)`) scans again, accepts only paths that fresh scan returned without an error, re-reads each file (never trusts what the UI shows), and creates one dot per path through `Runtime::create_dot`. It returns the dots made and, per path, why one was not.
3. **Mapping** to `DotSpec`:
   - `name`: the frontmatter `name`, else the file stem, made a valid dot name (*builder's call, forced by `DotSpec::validate`*: dot names are 1-64 of `A-Za-z0-9-_`, so other characters become `-`, runs of `-` collapse, an empty result is `agent`).
   - `description`: Dots has no description field, so it is shown in the preview only and nothing is added to the prompt.
   - `instructions`: the body, trimmed. An empty body is an error ("has no instructions").
   - `engine`: claude. `model`: `sonnet`/`opus`/`haiku` as they are (Dots uses these aliases); `inherit` or none gives the `DotSpec::new` default; any other value is kept as a full model id.
   - `workdir`: a project agent gets its project folder and the workspace mode of the dot that has that folder. Any other agent gets `<data dir>/agents/<dot name>` (made at import) in folder mode (*builder's call*: Dots has no default workdir and worktree mode needs a git repository).
   - Everything else: the `DotSpec::new` defaults.
4. **Policy**:
   - No `tools` listed: the Sandboxed preset.
   - Tools listed: the Custom preset with default ask, without the preset's own allow rules for file and command tools (`Read`, `LS`, `Glob`, `Grep`, `Write(./**)`, `Edit(./**)`, `Bash(git …)`), keeping its deny rules and Dots' own tools. Each listed known tool gets an allow rule by its bare name, except the risky ones (`Bash`, `PowerShell`, `Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `WebFetch`), which get an ask rule. Unknown names (and `mcp__…` tools, whose servers the dot does not have) are ignored and named in the warnings.
   - Known tools: `Read`, `LS`, `Glob`, `Grep`, `WebSearch`, `TodoWrite`, `Task`, `Agent`, `NotebookRead`, `Skill`, `BashOutput`, `KillShell`, `ExitPlanMode` (allowed) and the risky list.
5. **Already imported**: a dot whose name equals the agent's dot name, ignoring case. Importing it anyway gives a new name with `-2`, `-3`, … (*builder's call*: `" (2)"` is not a valid dot name). Names are also kept apart within one import.
6. **Frontmatter lists**: the kit's reader learns simple lists (block `- item` and flow `[a, b]`) in a new `Frontmatter::lists` map and a `Frontmatter::list(key)` accessor that also splits a comma string. `fields` is unchanged for every existing caller.

## 4. Core

- `kit/frontmatter.rs`: `lists`, `list()`.
- `kit/agent.rs` (new): `read_agent(path)`, `dot_name(raw)`, `agent_model(raw)`, `agent_policy(tools)`, `agent_files(dir)` (recursive `.md`, at most 6 levels, files over 256 KiB refused).
- `claude_agents.rs` (new): `ScannedAgent`, `AgentSourceKind`, `ImportResult`, `scan(...)` over given folders (pure, testable), and the spec mapping.
- `Config::claude_home` (`None` in `Config::new`, the home folder in `Config::from_settings`), so tests never read the real `~/.claude`.
- `Runtime::scan_claude_agents()`, `Runtime::import_claude_agents(paths, org_id, workspace_id)`.

## 5. App

Commands `scan_claude_agents` and `import_claude_agents`; registered in `command_list!`; contract samples `ScannedAgent` and `AgentImport`, added by hand.

## 6. UI

"Import from Claude Code" under New dot in the Dots column (hidden while making dots is refused), opening a dialog: agents grouped by source (User, each project folder, each plugin), a checkbox per importable agent (already imported ones unchecked and marked "Already a dot"), its description, model, tools and warnings, a file that cannot be imported shown with its reason and no checkbox. Import makes them in the account and workspace in view, then lists what was made and what failed, and a toast says how many.

## 7. Not in this part

- Keeping an imported dot in step with its file later.
- `disallowedTools`, hooks, `color`, MCP servers of an agent.
