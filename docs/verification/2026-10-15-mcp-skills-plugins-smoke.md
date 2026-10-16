# MCP servers, skills and plugins: smoke test

- Date: 2026-10-16
- Build: debug (`bun run tauri build --debug --no-bundle`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/mcp-skills` at commit `89eb313` (Tasks 1 to 10, with their fix rounds). Windows 11. The local model is `google/gemma-4-12b-qat` in LM Studio, loaded with a context of 8192.
- Names changed after the run: a plugin's MCP server item is now named `plugin-<plugin>-<server>`, no longer `plugin_<plugin>_<server>` (a fix of the final review: the old name is Claude Code's own prefix for a plugin's server). This document says `plugin-smokeplug-notes` and `mcp__plugin-smokeplug-notes__peek` where the run saw `plugin_smokeplug_notes` and `mcp__plugin_smokeplug_notes__peek`; Part A was not run again.
- Part A (agent steps): 11 steps (A1 to A11), 11 as expected in what Dots did. Two checks inside them could not be made as written (findings 1 and 2). 8 findings in all, none a defect that stops the feature: the rest are display notes, differences in the environment and a known open item.
- Part B (person steps): 4 steps, all open. Nothing in Part B was run; the agent never runs the real `claude` CLI.
- Screenshots: not in the repository. They are in the scratchpad folder of the run (the folder `SHOTS` below: `C:/Users/aksha/AppData/Local/Temp/claude/C--Users-aksha-dots/5f801379-3be9-46b5-8f0d-fa7a43eda6aa/scratchpad/mcp/smoke-shots`), 9 pictures, the app's log `dots.2026-10-16.log`, and the kit for Part B in `SHOTS/kit`. Each result says what the picture shows.

## What this covers

Section 11 of `2026-10-15-mcp-skills-plugins-design.md` asks for a smoke with LM Studio: a local dot with `fake-mcp`, where a read-only tool runs and a writing tool asks and is approved; a skill the model reads with `skill`; a plugin with a skill and a server; a changed tool list that holds the server and shows in the Inbox, then accepted. Part A runs that against a debug build with a real local model. Part B lists what needs a person at the machine, because the agent never runs the real `claude` CLI: a Claude-engine dot with a real plugin and an MCP server.

Review fixes made since the plan was written change some expectations of the plan's blocks. The ones this smoke touches: server names may not start or end with `_` or contain `__` (the plugin's server is `plugin-smokeplug-notes`, which is allowed); a server whose tool list changed and cannot be listed again is held; a server's catalog loses its accepted list when an http url moves to another origin; `mcp__*` tools of a Claude run are decided by the MCP check (`mcp_default`); plugin copies for Claude runs are cleaned, and local runs make none (counted in A11).

## How the agent steps were run

Part A drives the real app from a terminal.

- The app is the debug build in the shared target folder, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d`. The real data folder `%LOCALAPPDATA%\dots` is never used. The webview's own profile is moved too (`WEBVIEW2_USER_DATA_FOLDER`), and its debugging port is 9333.
- The app is started with a `PATH` that holds no `claude` CLI (block A2 leaves out every folder that has one; here `/c/Users/aksha/AppData/Roaming/npm`), so no run can reach the real CLI.
- `DOTS_CONFIRM=yes` answers the native confirmations in a debug build (the plugin's "This plugin starts: ..." question, the command line of a server).
- The window is driven through WebView2's debugging port with `tools/smoke/webview.mjs`: `click` and `fill` act on a control by its accessible name, `eval` calls a command through the Tauri bridge, `text` and `shot` read the page.
- The model server is LM Studio (`~/.lmstudio/bin/lms.exe`), port 1234. It is never stopped by a step. A4 records what ran before the smoke; A11 undoes only what the smoke started.
- Every block is run in Git Bash from the repository root. Block A2 writes a state file (`$SHOTS/env.sh`) and every later block starts by reading it (`. "${E:?}"`). In this run each block was saved as a script and run with `E` and `SHOTS` exported; the blocks below are what the scripts held.
- Git Bash rewrites an argument that starts with `/` into a Windows path. The blocks use no such argument: `taskkill //PID`, routes without the leading slash.
- Safety: the installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) and the real `claude` CLI are never started or stopped by these blocks. Only the `dots-app.exe` that runs from `target\debug` is stopped, by its process id. The installed Dots was not running at any point of this run.

## Part A: agent steps

### A1. Nothing of ours is running; build the app and `fake-mcp`

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

Result: pass. `no fake process running`. No `dots-app.exe` was listed at all (the installed one was not running). `bun install`: "Checked 572 installs across 632 packages (no changes)". The build finished: "Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 59s", "Built application at: C:\Users\aksha\dots\target\debug\dots-app.exe". `fake-mcp` built ("Finished `dev` profile ... in 26.78s"); `fake-mcp.exe` exists (3359744 bytes). The helper's tests: `tests 15`, `pass 15`, `fail 0`.

### A2. Temporary folders, a clean PATH, the state file

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
app_pid() { powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | Where-Object { \$_.ExecutablePath -like '*\target\debug\*' } | ForEach-Object { \$_.ProcessId }" | tr -d '\r'; }
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

Result: pass. Output: `left out of PATH: /c/Users/aksha/AppData/Roaming/npm` (twice, the folder is twice on the PATH), `data`, `work`, `no claude on the clean PATH`.

### A3. The kit: two scripts of one server, a skill, a plugin, the CLI's config

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

Result: pass. `valid: facts-a.json`, `valid: facts-b.json`, `valid: plugins/smokeplug/.mcp.json`, `valid: plugins/smokeplug/notes.json`, `valid: cli/facts.json`, and `1`.

### A4. The app starts on the temporary data folder; the model server

Before this block, what ran was recorded with `lms ps` and `lms server status` (a separate call, not part of the block): the first call printed "Waking up LM Studio service...", then "No models are currently loaded." and "The server is running on port 1234."

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

Expect: the data folder is the temporary one. The banner "The local server is not running: could not listen on 127.0.0.1:47321 ..." is expected: the installed Dots holds that port, and local dots do not need it. "The claude CLI was not found" is expected too (the clean `PATH` worked). The sidebar lists Dots, Library, Inbox. The first `lms ps` and `server status` show what ran before this smoke: write it into the result, and at the end undo only what this smoke started. Then the server runs on port 1234 with the model loaded.

Result: pass, with one difference (finding 3). `status.data_dir` was `C:\Users\aksha\AppData\Local\Temp\tmp.51M0I5qrU9\data`. The page text: sidebar "Dots, Library, Inbox, Chat, Settings", banner "The claude CLI was not found. Claude dots cannot run until it is installed or its path is set." with "Open Settings". There was no local-server banner. Before the smoke: the model server already ran on port 1234 and no model was loaded. In the block, `server status` said "The server is running on port 1234."; `server start` said "Success! Server is now running on port 1234"; the model loaded ("(6.66 GiB)", "To use the model in the API/SDK, use the identifier "google/gemma-4-12b-qat"").

### A5. The Library: a server tested and added, a skill, a plugin and its server

```bash
. "${E:?}"
CFG="{ transport: 'stdio', command: '$FAKE', args: ['$KITM/facts.json'], cwd: null, env: {} }"
$W eval "(async () => { const config = $CFG; const p = await invoke('library_probe', { config }); const item = await invoke('library_add', { name: 'facts', config: { kind: 'mcp_server', config }, accept: p.fingerprint }); return [p.tools.map(t => [t.name, t.annotations?.readOnlyHint === true]), p.fingerprint, item.name]; })()"
catalog facts | head -c 300; echo
$W eval "(await invoke('library_scan_skills', { path: '$KITM/skills' })).map(s => [s.name, s.description, s.name_error])"
$W eval "(await invoke('library_add', { name: 'smoke-facts', config: { kind: 'skill', config: { path: '$KITM/skills/smoke-facts' } }, accept: null })).name"
$W eval "(await invoke('library_add', { name: 'smokeplug', config: { kind: 'plugin', config: { path: '$KITM/plugins/smokeplug', installed_id: null, run_hooks: false } }, accept: null })).name"
FACTS=$(item_id facts); NOTES=$(item_id plugin-smokeplug-notes); printf 'FACTS=%s\nNOTES=%s\n' "$FACTS" "$NOTES" >> "$E"
$W eval "(async () => { const c = await invoke('library_check', { id: '$NOTES' }); const a = await invoke('library_accept', { id: '$NOTES', fingerprint: c.seen_fingerprint }); return [c.status, a.status]; })()"
$W eval "(await invoke('library_list')).map(e => [e.kind, e.name, e.catalog?.status ?? null, e.owner_id !== null, e.plugin ? [e.plugin.skills, e.plugin.commands, e.plugin.servers] : null, e.problem])"
$W eval "(await invoke('inbox')).tools_changed.length"
sleep 3; tasklist | grep -c -i fake-mcp
$W goto library; sleep 2; $W text | sed -n '/^Library$/,$p' | head -30; $W shot "$SHOTS/01-library-servers.png"
$W click "Plugins"; sleep 1; $W shot "$SHOTS/02-library-plugins.png"
```

Expect: the probe lists `lookup` (true), `save` (false) and `refresh` (true) with a fingerprint, and the add answers `facts`. The catalog of `facts` is `accepted`, its `accepted_fingerprint` equal to `seen_fingerprint` and to the probe's. The scan lists `smoke-facts` with its description and no name error. The skill and the plugin are added (the plugin's native question, "This plugin starts: ...", is answered by `DOTS_CONFIRM=yes`). The plugin's server is named `plugin-smokeplug-notes`; its check answers `changed` (a server never accepted shows for review) and the accept `accepted`. The list: the server `facts` accepted, `plugin-smokeplug-notes` accepted and owned, the skill, and the plugin with the skill `smokeplug:greet`, the command `hi` and the server `plugin-smokeplug-notes`, no problem. `tools_changed` is 0. No `fake-mcp` process is left (the count is 0): the probe and the checks stop the servers they start. The screenshots show the MCP servers tab (two rows, "Accepted", "Comes with plugin smokeplug" on the second) and the Plugins tab (the parts, no hooks switch: the plugin has no hooks).

Result: pass, with a gap in what was recorded (finding 1). The first lines of the block's output (the probe, the adds, the scan, the check and accept of the plugin's server) were cut off by a `tail` in this run and were not kept. They were checked again afterwards: a new probe of the `facts.json` config gave `[["lookup",true],["save",false],["refresh",true]]` and the fingerprint `276f403c...5f7c`; the catalog of `facts` is `accepted` with `accepted_fingerprint` `276f403c...5f7c`, equal to `seen_fingerprint`; the scan lists `smoke-facts` with the description "Facts for the smoke test of Dots. Read it when you are asked for the smoke code word." and name error null. Kept from the block: the list holds `facts` (accepted, not owned), `plugin-smokeplug-notes` (accepted, owned), the skill `smoke-facts`, and the plugin `smokeplug` with skills `["smokeplug:greet"]`, commands `["hi"]`, servers `["plugin-smokeplug-notes"]` and `problem` null; `tools_changed` 0; the `fake-mcp` count 0. The status `changed` of the plugin server's first check was not seen: a check run afterwards answered `accepted` with `seen_fingerprint` equal to `accepted_fingerprint`.

`01-library-servers.png` shows the MCP servers tab: two rows, `facts` (stdio, "Accepted", "Not used yet", buttons Check now, Edit, Remove) and `plugin-smokeplug-notes` (stdio, "Accepted", "Comes with plugin smokeplug", "Not used yet", only "Check now"). `02-library-plugins.png` shows the Plugins tab: `smokeplug` 0.1.0 with "1 skill", "1 command", "1 MCP server", the description, the folder, "Commands, agents and hooks apply to Claude-engine dots only.", "Not used yet" and Remove; no hooks switch. The red banner "The claude CLI was not found" is on both.

### A6. A read-only tool runs, a writing tool asks, and the approved call is carried out

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

Result: pass. The first run ended `awaiting_approval`. Calls: `mcp__facts__lookup` `{"topic":"dots"}` with result (not an error) "The code word for dots is heron."; `mcp__facts__save` `{"note":"heron"}` with result (flagged as an error) "Queued for human approval #01M53A0GNRJT2A2M5XXG51AVSJ. Do not retry this action. Finish any other work and end with your summary." One approval, `mcp__facts__save`, `pending`, `mcp_fingerprint` `276f403c...5f7c`, equal to the catalog's `accepted_fingerprint`. Served: lookup 1, save 0. The Inbox page showed "Approvals", "mcp-dot asked just now", `mcp__facts__save {"note":"heron"}`. After `approve_first`: `"approved"`; the resume run ended `succeeded`; the approval is `approved`, `applied`; the resume run's one call is `mcp__facts__save {"note":"heron"}` with result `save: {"note":"heron"}` (not an error); served: lookup 1, save 1. The run page text names "facts › lookup" and "facts › save". The model did not stray.

`03-ask.png`: the Inbox with the group "Approvals", one card "mcp-dot asked just now", the title `mcp__facts__save {"note":"heron"}`, the arguments as JSON, the line "The run stopped waiting and is parked. If you approve, Dots performs exactly this call and then continues the run.", a note field and the buttons Approve, Deny, Approve and always allow; the sidebar's Inbox count is 1. `04-run.png`: the run "mcp-dot run", "Succeeded", the summary of the resume run; a "Performed by Dots" row for `mcp__facts__save`; "Manual run" with rows "facts › lookup ... done" and "facts › save ... failed" (finding 4); "Continued after approval" with "facts › save ... done".

### A7. Skills and a plugin's server in a local run

```bash
. "${E:?}"
KITDOT=$(new_dot kit-dot "{ mcp_servers: [], skills: ['smoke-facts'], plugins: ['smokeplug'] }" "Use your tools, one call at a time. First call the tool skill with the name smoke-facts. Then call the tool skill with the name smokeplug:greet. Then call mcp__plugin-smokeplug-notes__peek. Then answer in three lines: the smoke code word, the greeting, and the newest note."); echo "KITDOT=$KITDOT" >> "$E"
R3=$(run_now "$KITDOT"); echo "run: $(settled "$R3")"
calls "$R3"; approvals "$R3"; said "$R3"
echo "served: peek $(served notes peek)"
$W goto "runs/$R3"; sleep 2; $W shot "$SHOTS/05-skills.png"
$W goto "dots/$KITDOT"; sleep 2; $W text | sed -n '/^Tools and skills$/,/^Limits$/p'; $W shot "$SHOTS/06-editor.png"
```

Expect: the run ends `succeeded` with no approval. Its calls: `Skill` with `{"name": "smoke-facts"}`, whose result holds "The smoke code word is kestrel." and the list of the skill's files with `notes.md`; `Skill` with `{"name": "smokeplug:greet"}`, whose result holds "Greet with the words: hello from the plugin."; `mcp__plugin-smokeplug-notes__peek`, whose result is "The newest note says: plugin servers work.". The answer names kestrel. The plugin's server has served `peek` once. The run view names the last call "smokeplug › peek". The editor's "Tools and skills" has `smoke-facts` and `smokeplug` ticked, and the note that on a local dot `smokeplug` gives its skills and MCP servers only and its command does not apply.

Result: pass, with one picture gap (finding 2). The run ended `succeeded`, no approvals. Calls: `Skill` `{"name":"smoke-facts"}` with "The smoke code word is kestrel.\n\nMore is in notes.md in this skill's folder.\n\nFiles in this skill (read one with `file`):\n- notes.md"; `Skill` `{"name":"smokeplug:greet"}` with "Greet with the words: hello from the plugin."; `mcp__plugin-smokeplug-notes__peek` `{}` with "The newest note says: plugin servers work.". The model wrote "kestrel / hello from the plugin. / plugin servers work." `peek` served once. The editor text: "Tools and skills", "MCP servers facts", "Skills smoke-facts Facts for the smoke test of Dots. ...", "Plugins smokeplug A plugin for the smoke test of Dots. On a local dot, smokeplug gives its skills and MCP servers only. Its 1 command do not apply." (the page text lists every item, ticked or not; see finding 2).

`05-skills.png`: "kit-dot run", "Succeeded", "Took 1m 53s", the answer, rows "Skill {"name":"smoke-facts"} done", "Skill {"name":"smokeplug:greet"} done", "smokeplug › peek {} done", and the answer text. `06-editor.png`: the top of the dot's editor (name, instructions, engine "Local model", model, endpoint, workspace folder); the "Tools and skills" section is below the fold and is not in the picture.

### A8. The tool list changes during a run: the server is held and the Inbox shows it

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

Result: pass. The run ended `awaiting_approval`. Calls: `mcp__facts__refresh` `{}` with "refresh: {}" (not an error); `mcp__facts__lookup` `{"topic":"dots"}` with the queued text "Queued for human approval #01M53A8Y7NMA8NAHQ7BNQ7N67F. ...". One approval, `mcp__facts__lookup`, `pending`, `mcp_fingerprint` `276f403c...5f7c` (the one accepted before the change). The catalog of `facts`: `changed`; read again with a second call: `seen_fingerprint` differs from `accepted_fingerprint`, `seen_tools` `["lookup","save","refresh","erase"]`. `inbox().tools_changed`: `[{"name":"facts","dots":[{"name":"mcp-dot", ...}]}]`; `inbox_count` 2. The page text: "Tools changed", "facts", "Used by mcp-dot", "Review".

`07-tools-changed.png`: the Inbox with the sidebar count 2; the "Approvals" card for `mcp__facts__lookup {"topic":"dots"}`; below it the group "Tools changed" with one card "facts / Used by mcp-dot" and a "Review" button.

### A9. The change is accepted in the Library; the approval given before it is not carried out

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

Result: pass. The review dialog read "Tools changed: facts", "The server lists different tools than the ones you accepted. Until you accept the new list, runs that use the server ask before every call to it.", ADDED `erase` "Erase every saved note."; no removed or changed section. After "Accept": `status` `accepted`, `accepted_fingerprint` `60e1a654...1cc9` (the old one was `276f403c...5f7c`); `tools_changed` count 0. `approve_first`: `"approved"`. The resume run ended `succeeded`; the approval: `mcp__facts__lookup`, `approved`, `apply_state` `skipped`. The first event of the resume run is the message "Dots performed your approved actions. ... Approval #01M53A8Y7NMA8NAHQ7BNQ7N67F, mcp__facts__lookup {"topic":"dots"}: not performed, becaus[e the tool changed since it was approv...]" (read from the event list, cut at 300 characters). The model then called `mcp__facts__lookup` again; the call was allowed without asking and gave "The code word for dots is heron."

`08-review.png`: the review dialog over the blurred Library, where the `facts` row carries a yellow "Tools changed" badge; buttons Close and Accept. (The picture was taken before "Accept" was pressed.)

### A10. The run proceeds

```bash
. "${E:?}"
R6=$(run_now "$MCP"); echo "run: $(settled "$R6")"
calls "$R6"; approvals "$R6"; said "$R6"
echo "served: lookup $(served facts lookup), refresh $(served facts refresh)"
$W goto "runs/$R6"; sleep 2; $W shot "$SHOTS/09-proceeds.png"
```

Expect: the run ends `succeeded` with no approval: `refresh` and `lookup` run without asking (the accepted list is the server's list, and both are read-only); `lookup` answers "The code word for dots is heron."; no notice says that the server is held.

Result: pass. The run ended `succeeded`; no approvals. Calls: `mcp__facts__refresh` `{}` "refresh: {}"; `mcp__facts__lookup` `{"topic":"dots"}` "The code word for dots is heron." No notice event in the list. The model wrote "The code word for dots is heron." Served: lookup 3 (the runs of A6, A9 and A10; the parked call of A8 was never served), refresh 2 (A8 and A10).

`09-proceeds.png`: "mcp-dot run", "Succeeded", "Took 47s", rows "facts › refresh {} done" and "facts › lookup {"topic":"dots"} done", the answer "The code word for dots is heron.".

### A11. Stop and clean up

```bash
. "${E:?}"
PID=$(app_pid); echo "stopping $PID"; [ -n "$PID" ] && taskkill //PID "$PID" //F
sleep 3; tasklist | grep -c -i fake-mcp
cp "$SMOKE"/data/logs/dots.*.log "$SHOTS/"
ls "$SMOKE/data/plugins" 2> /dev/null | grep -c '^run-'
[ -d "$SMOKE/data" ] && rm -rf "$SMOKE"; ls -d "$SMOKE" 2>&1 | sed 's#/tmp/tmp\.[A-Za-z0-9]*#$SMOKE#'
```

Then the model was unloaded, because step A4 showed that this smoke loaded it: `timeout 60 "$LMS" unload --all < /dev/null`. The server was already running before the smoke, so it was not stopped.

Expect: the one process this smoke started is stopped; no `fake-mcp` process is left (0); no `run-*` folder is left under the data folder's `plugins` (0: local runs make none); the temporary folder is gone; the app's log is kept beside the screenshots, and the kit stays in `$SHOTS/kit` for Part B. Look at every screenshot and say in the results what it shows.

Result: pass. `stopping 12264`, "SUCCESS: The process with PID 12264 has been terminated." `fake-mcp` count 0; `run-*` folders 0; the temporary folder is gone (`ls: cannot access '$SMOKE'`). No `dots-app.exe` is running now. `lms unload --all`: "Unloaded 1 model."; `lms ps`: "No models are currently loaded."; `lms server status`: "The server is running on port 1234." (as before the smoke). The log `dots.2026-10-16.log` is in the screenshots folder with `01-library-servers.png` to `09-proceeds.png` and `kit/`. What each picture shows is under its step.

## Findings of Part A

1. A5: the first lines of the block's output (probe, adds, scan, and the check and accept of the plugin's server) were lost to a `tail` in this run. The probe, the catalog and the scan were checked again and match the expectations. The expectation that the first check of `plugin-smokeplug-notes` answers `changed` was not seen: the plugin's server was already `accepted` when the list was read, and a check run later answered `accepted`. Whether the add of the plugin or the block's own check accepted it first is not known from this run.
2. A7: the editor's ticks (`smoke-facts` and `smokeplug` ticked, `facts` not) were not seen. The page text lists every item and does not tell ticked from unticked, and the screenshot shows only the top of the page. The run shows that the dot used the skill and the plugin and not `facts`. The note about commands on a local dot was seen in the text.
3. A4: the banner "The local server is not running: could not listen on 127.0.0.1:47321" did not appear, because the installed Dots was not running at all and the port was free. The first `lms` call printed "Waking up LM Studio service..."; the server already ran on port 1234 and no model was loaded, so only the model was undone.
4. A6: in the run view, the first run's parked call `facts › save` shows "failed" (its tool result is flagged as an error: the "Queued for human approval" text), beside "Continued after approval" where the same call shows "done". A person may read "failed" as a fault; the approval is the cause.
5. A6, A8: the cards in the Inbox name `mcp__facts__save` and `mcp__facts__lookup`; only the run view shows "facts › save" (a known open item of the plan, Task 10).
6. A9: the text "not performed, because the tool changed since it was approved" was read only up to "approv", because the smoke's listing cuts the message at 300 characters. The apply state `skipped` and the model's second call confirm the case.
7. The model followed the instructions exactly in every run; no step was run twice. Each local run took about one to two minutes (A6 2m 01s, A7 1m 53s, A10 47s).
8. The review fixes made after the plan was written changed nothing a step of Part A relied on: the plugin's server name `plugin-smokeplug-notes` is valid under the new name rules, `DOTS_CONFIRM=yes` answered the plugin's native question, and the held server and the skipped approval behaved as the fixes describe.

## What Part A does not show

- B1 to B4 below: the real `claude` CLI, its `--allowedTools` with `mcp__<server>__*`, `--plugin-dir` more than once, `plugin_errors`, the gateway's answers to the CLI's HTTP client, `--setting-sources ""` and `~/.claude/skills`, and `CLAUDE_CODE_PLUGIN_CACHE_DIR`.
- An HTTP server (the tests of Task 3 cover it with `fake-mcp --http`), including the native question before `${VAR}` values go to a new host and the catalog that loses its accepted list when the url moves to another origin.
- A server that cannot start or a variable that is not set (Task 5's `a_server_that_cannot_start_is_left_out_with_a_run_event`), and a server held because its changed list cannot be listed again.
- A plugin's hooks, and an installed plugin that moved (the native question asked again with hooks on).
- The plugin copies of Claude runs (names matched without case, `.md` files cleaned, `.lsp.json` dropped): local runs make none (A11 counted 0 `run-*` folders).
- The MCP check for non-gateway `mcp__*` tools of a Claude run (`mcp_default`).
- The upgrade of a database with per-dot MCP JSON (Task 2's `migration_0006_moves_each_dots_servers_into_the_library`).
- The Library forms (add, edit and remove by clicking) and the "Add plugin" list of installed plugins: Part A called the same commands through the bridge.
- The ticks of the dot editor (finding 2).

## Part B: person steps

Part B is a checklist for the person who owns the machine. The person runs the real `claude` CLI; the agent did not. In Git Bash, `. "$SHOTS/env.sh"` gives `KIT`, `KITM` and `FAKE` (set `SHOTS` to the folder named at the top first); each block runs in a fresh empty folder (`cd "$(mktemp -d)"`). The kit (`facts.json`, the skill, the plugin) is in `$SHOTS/kit`. A9 left `facts.json` as the switched list; for B1 and B3 either list is fine, and `cp "$KIT/facts-a.json" "$KIT/facts.json"` restores the first.

- [ ] **B1.** The CLI takes `mcp__<server>__*` in `--allowedTools`. With `fake-mcp` as a plain stdio server, a writing tool must run without a permission question:

  ```bash
  claude -p "Call the tool mcp__facts__save with the note test. Then say done." --output-format stream-json --verbose --strict-mcp-config --mcp-config "$KITM/cli/facts.json" --allowedTools "mcp__facts__*" --setting-sources "" > save.jsonl
  grep -o '"type":"tool_result"[^}]*' save.jsonl | head -3; grep '"tools/call"' "$KIT/logs/facts.log" | grep -c '"save"'
  ```

  Expected: a `tool_result` with "save: ..." and no permission denial, and the count of `save` calls in the log one higher than before (A6 left it at 1; the log is `$KIT/logs/facts.log`). A denial means the pattern is not honoured: Dots' `approve` then answers allow for the run's own servers and the gateway still asks once (Task 7), but the spec's section 5 should say so.

  Result:

- [ ] **B2.** The CLI takes `--plugin-dir` more than once; which skills `--setting-sources ""` loads; the shape of `plugin_errors`. Make a broken plugin first: `mkdir -p "$KIT/plugins/broken/.claude-plugin" && printf '{ not json\n' > "$KIT/plugins/broken/.claude-plugin/plugin.json"`.

  ```bash
  claude -p "Say ok." --output-format stream-json --verbose --setting-sources "" --plugin-dir "$KITM/plugins/smokeplug" --plugin-dir "$KITM/plugins/broken" | head -1 > init.json
  cat init.json
  ```

  Record from this `system/init` line: the skills it lists (expected: the plugin's `greet`, under the manifest name `smoke-plugin`, since this is the original folder and not Dots' copy); whether any skill of `~/.claude/skills` is listed (the spec's open risk, section 12); and the exact `plugin_errors` entry of `broken`, to compare with what `parse.rs` reads (`plugin`, `name` or `source`, and `error` or `message`).

  Result:

- [ ] **B3.** A Claude-engine dot through Dots' gateway. Start the debug build from Git Bash with the real `PATH`, a temporary data folder and real dialogs: `DOTS_DATA_DIR="$(cygpath -w "$(mktemp -d)")" C:/Users/aksha/dots/target/debug/dots-app.exe &`. If the banner says that port 47321 is taken (the installed Dots holds it), set the port to 47399 in Settings, save and "Restart Dots". In the Library add `facts` (command `$FAKE`, argument `$KITM/facts.json`; "Test connection", answer the Windows dialog, "Save"), the skill folder `$KITM/skills/smoke-facts` and the plugin folder `$KITM/plugins/smokeplug` (answer the dialog that names its server). Create a Claude dot in an empty folder that uses `facts`, `smoke-facts` and `smokeplug`, with the instructions "Call mcp__facts__lookup with the topic dots, then mcp__facts__save with the code word it gives. Then use the skill dots-skills:smoke-facts and the skill smokeplug:greet, and answer with the smoke code word and the greeting.", and press "Run now".

  Expected: the run view shows "facts › lookup" with "The code word for dots is heron." (the CLI accepted the gateway's plain JSON answers); `save` asks once in the Inbox (one approval, `mcp__facts__save`, not two); after "Approve" the resume run's retry goes through and `$KIT/logs/facts.log` has one more `save` call; the answer names kestrel and "hello from the plugin"; no "plugin ..." note in the run view. If the CLI reports that the server `facts` failed, record its message from the run's raw events: the gateway's answers were refused.

  Result:

- [ ] **B4.** `CLAUDE_CODE_PLUGIN_CACHE_DIR`. Find in Claude Code's documentation (or `claude plugin --help`) which folder the variable names, and whether `installed_plugins.json` is in that folder or in the folder above it. Dots reads it as the folder that holds `installed_plugins.json` (`kit::plugins_root`). In the B3 instance, "Add plugin" lists the installed Claude Code plugins: compare them with `claude plugin list`.

  Result:
