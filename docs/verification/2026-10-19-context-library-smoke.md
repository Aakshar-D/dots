# Context library: smoke test

- Date: 2026-10-19
- Build: debug (`bun run tauri build --debug --no-bundle`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/context-library` at commit `5da5be3` (Tasks 1 to 8, with their fix rounds). Windows 11. The local model is `google/gemma-4-12b-qat` in LM Studio, loaded with a context of 8192.
- Part A (agent steps): 3 steps (A1 to A3), all as expected in what Dots did. The local model strayed in two runs (a tool call that was not asked for); that is model behaviour, not a defect. 4 findings, none a Dots defect.
- Part B (person steps): 1 step with two parts (B1, B2), open. Nothing in Part B was run; the agent never runs the real `claude` CLI.
- Screenshots: not in the repository. They are in the scratchpad folder of the run (`C:/Users/aksha/AppData/Local/Temp/claude/C--Users-aksha-dots/6f64b9c2-ef46-4359-8b32-09a568c6c3bc/scratchpad/ctx`): `01-library-context.png`, `02-settings.png`, `03-denied-run.png`, `04-missing-folder.png`, the app's log, and the first prompt of A1 (`prompt-R1.txt`).

## What this covers

The context library spec asks for a smoke with LM Studio: a local dot that lists, searches and reads a shared folder with the `Context` tool; a collection that every dot reads without being ticked, with the global instructions of Settings at the head of every first prompt; and the refusals (a `Context` deny rule, a folder that is gone). Part A runs that against a debug build with a real local model. Part B lists what needs a person at the machine, because the agent never runs the real `claude` CLI: a Claude-engine dot that reads the collection through the `dots` MCP server, first without an approval, then under an ask rule.

## How the agent steps were run

Part A drives the real app from a terminal, as the earlier smokes did (`2026-10-15-mcp-skills-plugins-smoke.md`).

- The app is the debug build in the shared target folder, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d` (`C:\Users\aksha\AppData\Local\Temp\tmp.45wBazorI0\data`). The real data folder `%LOCALAPPDATA%\dots` and the installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) are never used or touched; the installed Dots was not running. The webview's own profile is moved too (`WEBVIEW2_USER_DATA_FOLDER`); its debugging port is 9333. Every path variable was echoed before use.
- The app is started with a `PATH` without any folder that holds a `claude` CLI (`/c/Users/aksha/AppData/Roaming/npm` was left out), so no run can reach the real CLI.
- The window is driven through WebView2's debugging port with `tools/smoke/webview.mjs`: `eval` calls a command through the Tauri bridge (`invoke(...)`); `goto`, `click`, `text` and `shot` drive and read the page. Library items, dots and Settings were changed with `invoke` (`library_add`, `library_update`, `create_dot`, `update_dot`, `update_settings`); the Library's Context tab and Settings were read with `text` and `shot`.
- The model server is LM Studio (`~/.lmstudio/bin/lms.exe`), port 1234. Before the smoke the server ran and no model was loaded. The smoke loaded the model (`lms load google/gemma-4-12b-qat -c 8192 -y`) and unloaded it at the end; the server was left as it was found.
- The folder `company` holds `style.md` (`# House style`, `Use short sentences.`, `Every summary ends with a full stop.`) and `people/ana.md` (`# Ana`, `Ana leads the design team.`). The dots are local dots (`engine: 'local'`, `workspace_mode: 'folder'`, `approval_wait_secs: 0`, preset `sandboxed`) with a work folder that holds a `README.md`.
- The first prompt of a run is the second `message` event of `list_run_events` (the first is the system message).

Helpers used in the commands below (shell functions of the run's state file, each wrapping one `$W eval`, with `W="node tools/smoke/webview.mjs 9333"`): `new_dot <name> <uses> <instructions>`, `run_now <dot>`, `settled <run>` (waits until the run has left `queued` and `running`), `prompt <run>`, `calls <run>` (tool calls, results and notices), `said <run>`.

## Part A: agent steps

### A1. A local dot reads a collection

```bash
$W eval "(await invoke('library_add', { name: 'company', config: { kind: 'context', config: { path: '$COMPANY', description: 'House style and people', every_dot: false } }, accept: null }))"
$W eval "await invoke('context_documents', { id: '$(item_id company)' })"
D1=$(new_dot style-dot "{ mcp_servers: [], skills: [], plugins: [], context: ['company'] }" "Search the company collection for the house style and quote it")
R1=$(run_now "$D1"); settled "$R1"; prompt "$R1"; calls "$R1"; said "$R1"
```

Expect: the first prompt ends with `## Shared context` naming `company (2 documents)`; the transcript shows `Context` calls (a list or search, then a read); the summary quotes the file.

Result: pass. `library_add` answered the item `company` (kind `context`, `every_dot` false). `context_documents`: 2 documents, `people/ana.md` ("Ana", "Ana leads the design team.") and `style.md` ("House style", "Use short sentences."), `complete` true. Run `01M5BBA54VXK9KT769KJ3P7XFK` ended `succeeded`. Its first prompt ended with:

```
## Standing rules

- Work only inside this directory: C:/Users/aksha/AppData/Local/Temp/tmp.45wBazorI0/work
- If a tool call is queued for human approval, do not retry it; finish the remaining work.
- End with a short summary: what you did, what changed, and what needs human review.

## Shared context

Shared context you can read. Call the tool `context` with a collection name to list its documents, `query` to search it, or `path` to read one. Look here before you guess how the person works.
- company: House style and people (2 documents)
```

The `Context` calls in the transcript: `Context {"collection":"company"}` with the result (not an error) "- people/ana.md — Ana: Ana leads the design team.\n- style.md — House style: Use short sentences."; then `Context {"collection":"company","path":"style.md"}` with the result (not an error) "# House style\nUse short sentences.\nEvery summary ends with a full stop.". The summary: "The house style is:\n\n# House style\nUse short sentences.\nEvery summary ends with a full stop." The model listed and then read, where the brief said "a search, then a read"; the list call is the same tool. No approval was made.

`01-library-context.png` (taken in A2) shows the Library's Context tab: the row `company` with "every dot", "2 documents", the description, the folder, "Every dot reads it", Documents, Edit and Remove.

### A2. Every dot, and the global instructions

```bash
CID=$(item_id company)
$W eval "(await invoke('library_update', { id: '$CID', config: { kind: 'context', config: { path: '$COMPANY', description: 'House style and people', every_dot: true } } })).config"
$W eval "(async () => { const s = await invoke('get_settings'); const r = await invoke('update_settings', { settings: { ...s, global_instructions: 'Sign every summary with — Dots' } }); return r.settings.global_instructions; })()"
D2=$(new_dot signer-dot "{ mcp_servers: [], skills: [], plugins: [], context: [] }" "Say hello to the user in one sentence, then give your summary. Do not use tools unless you need them.")
R2=$(run_now "$D2"); settled "$R2"; prompt "$R2"; said "$R2"
$W eval "(async () => { const d = await invoke('get_dot', { id: '$D2' }); return (await invoke('update_dot', { id: '$D2', spec: { ...d, use_global_instructions: false } })).use_global_instructions; })()"
R3=$(run_now "$D2"); settled "$R3"; prompt "$R3" | head -12; prompt "$R3" | grep -c "Instructions for every dot"
```

Expect: the first prompt of the second dot, which ticks nothing, starts with `## Instructions for every dot` and lists `company`; its summary is signed. With "Start with the global instructions" off, the heading is gone.

Result: pass. The update answered `every_dot: true`; the setting was saved as "Sign every summary with — Dots". Run `01M5BBCRYBXQ0JGFVKKQK64SWH` of `signer-dot` (uses nothing) ended `succeeded`. Its first prompt began:

```
## Instructions for every dot

Sign every summary with — Dots

## Instructions for this dot

Say hello to the user in one sentence, then give your summary. Do not use tools unless you need them.

---

## Trigger
```

and ended with the same `## Shared context` section as in A1, listing `- company: House style and people (2 documents)` (the collection is read though the dot ticks nothing). The model called `LS` once (its own choice) and its summary ended "— Dots": "Hello! I have explored the workspace and found a `README.md` file. ... — Dots". With `use_global_instructions` off (the update answered `false`), run `01M5BBFA0TP3DAMNDS02EMK486` ended `succeeded`; its first prompt began "Say hello to the user in one sentence, ..." with no heading, and the count of "Instructions for every dot" in it is `0`. The `## Shared context` section is still there (the collection is separate from the global instructions). The summary was not signed.

`02-settings.png` shows Settings with the "Global instructions" field ("Every dot's first prompt starts with these. A dot can switch them off in its editor. 32 of 32768 bytes.").

### A3. Refusals

```bash
# the deny rule
$W eval "(async () => { const d = await invoke('get_dot', { id: '$D1' }); const u = await invoke('update_dot', { id: '$D1', spec: { ...d, policy: { ...d.policy, rules: [...d.policy.rules, { tool: 'Context', action: 'deny' }] } } }); return u.policy.rules.filter(r => r.tool.startsWith('Context')); })()"
R4=$(run_now "$D1"); settled "$R4"; calls "$R4"
# the same, after the instructions were narrowed to the context tool
R5=$(run_now "$D1"); settled "$R5"; calls "$R5"
# the deny rule removed, the folder gone
mv "$SMOKE/company" "$SMOKE/company-removed"
R6=$(run_now "$D1"); settled "$R6"; calls "$R6"; prompt "$R6" | grep -c "Shared context"
```

Expect: with the rule `Context` deny added beside the preset's `Context` allow, the calls are denied (deny outranks allow) and the run still succeeds. With the folder removed, the run has the notice `context company: folder not found` and the prompt has no `## Shared context` section.

Result: pass, after a second attempt of the first part (model behaviour).

Deny, attempt 1: the rules after the update were `Context allow` (the preset's) and `Context deny`. Run `01M5BBHT6CPDVHGXJ2S1DWY64N` called `Context` three times (`{"collection":"company"}`, `{"collection":"company","query":"house style"}`, `{"collection":"company"}`); each result was an error: "Denied by the policy of dot 'style-dot': Context is not permitted." The model then used other tools it was not asked to use (`LS`, `Read README.md`, `Bash {"command":"dir"}`); the `Bash` call was queued for approval, so the run ended `awaiting_approval` (screenshot `03-denied-run.png`: "Needs approval", the summary "The request to search the company collection for the house style was denied by the policy of dot 'style-dot', and I cannot access the context. — Dots", and the Bash card). The approval was denied by hand and the run then ended `succeeded`. This is the model straying, not a Dots defect: the denial of `Context` was right. Attempt 2: the dot's instructions were narrowed ("Use only the context tool. If it is denied, do not use any other tool: just say so and finish."); run `01M5BBNH7GYHV9V6SFCGHEH4VC` made one call, `Context {"collection":"company"}`, denied with the same text (an error result); no approval; the run ended `succeeded` with the summary "Denied by the policy of dot 'style-dot': Context is not permitted. — Dots".

Folder gone: the deny rule was removed again (`Context allow` only) and the folder `company` was moved to `company-removed`. Run `01M5BBQ5C14J8MB7FB2E5GZGQF`: its first event is the notice `context company: folder not found`; its first prompt has no `## Shared context` section (the count of "Shared context" in it is `0`; it still begins with the global instructions and the dot's instructions). The model had no `context` tool and called `Bash {"command":"context"}` (model behaviour); that call was queued, so the run ended `awaiting_approval`; the approval was denied by hand.

In the Library, after the folder went, `library_list` answers `problem: "folder not found"` for `company`, and `context_documents` answers `not_found` ("the folder cannot be read: The system cannot find the path specified. (os error 3)"). The Context tab showed "folder not found" on the row after the page was loaded again; `04-missing-folder.png` was taken before that reload and still shows "2 documents" (finding 2).

## Findings of Part A

1. The `Context` tool works end to end on a local dot: list, read (with the `collection` and `path` arguments the model chose itself), the document text as the result, no approval under the preset's `Context` allow rule.
2. The Library's Context tab did not notice that the folder was gone until the page was loaded again: in the same page session the row still read "2 documents" after the folder had been moved away. After a reload it shows "folder not found". The folder changes outside Dots, so nothing tells the open page; the run itself reports the problem with its notice. A display note, not a defect of the run.
3. Model behaviour, not Dots: `gemma-4-12b-qat` uses other tools when `Context` is denied or missing (`LS`, `Read`, `Bash`), which parked two runs at `awaiting_approval` (A3). Narrowing the instructions made the denial run end `succeeded`. On Dots' side the denied calls are error results the model can read, and they do not fail the run.
4. The notice `context company: folder not found` is a run event shown at the start of the run; the prompt leaves out the whole section when no collection can be read, as the brief expects.

No Dots defect found.

## What Part A does not show

- The Claude engine: the `dots` MCP server's context tool, its policy under the `Context` rule, and the approval flow with a resume (Part B).
- The Library's editor flows through the window (Add collection, Edit, the document list, the dot editor's tick boxes and the "Start with the global instructions" switch): the commands behind them were called with `invoke`; only the Context tab and Settings were read through the page.
- A collection with many documents, a large file, or a folder with files that are not text (the unit tests cover the limits).
- A policy rule that names one collection (`Context(company)`).

## Part B: person steps

These steps need the real `claude` CLI, which the agent never runs. Run them in the debug build or the installed Dots, on a Claude dot in a throw-away workspace.

Before: the Library has a Context collection `company`, a folder with a `style.md` that holds a few lines (for example `# House style` and two lines) and a `people/ana.md`. The Library lists it with a document count and no problem.

### B1. A Claude dot reads the collection without an approval

1. In the Library, open the Context tab and check that `company` is there (add it with "Add collection" if not). Leave "Every dot reads it" off.
2. Create or open a Claude dot (engine Claude). In its editor tick `company` among the context collections. Save.
3. Run it with the message (as the dot's instructions, or in a chat): `Read the house style from the shared context and quote it.`
4. Expect: the transcript shows `dots › context` calls (a list or a read of `style.md`) that succeed. No approval reaches the Inbox: the preset allows `Context`. The summary quotes the house style.
5. Record: the run id, the calls and their results as the run page shows them, the summary, and whether anything reached the Inbox.

### B2. Under a `Context` ask rule the call reaches the Inbox and, approved, the resumed run reads the file

The preset's `Context` allow rule outranks a `Context` ask rule (policy precedence is deny, then allow, then ask), so an added ask rule alone changes nothing. Replace the allow rule.

1. Open the dot's policy. Remove the rule `Context` allow (the preset's) and add the rule `Context` ask. Save.
2. Run the dot again with `Read the house style from the shared context and quote it.`
3. Expect: the run stops at the first `dots › context` call and ends `awaiting_approval`. The Inbox shows an approval for `mcp__dots__context` with the collection (and the path) in its arguments.
4. Approve the call in the Inbox.
5. Expect: Dots performs exactly that call and a resume run starts. Its resume message names `mcp__dots__context`. The resumed run reads the file (`style.md`) and quotes it in its summary. The approval is `approved` with `apply_state` `applied`.
6. Record: both run ids, the approval's id and arguments, the resume message, and the summary.

Things to look for as failures: a `dots › context` call that asks although the preset allows it (B1); a call that is allowed under the ask rule because the allow rule was left in place (B2); an approval that does not name `mcp__dots__context`; a resume run that does not read the file.
