# Approvals v2: smoke test

- Date: 2026-10-14
- Build: debug (`bun run tauri build --debug --no-bundle`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/approvals-v2` at commit `0f14cc1` (Tasks 1 to 9). Windows 11. The local model is `google/gemma-4-12b-qat` in LM Studio, loaded with a context of 8192.
- Part A (agent steps): 8 steps, 8 passed, each on its first and only run. 4 findings: one defect that is not explained (finding 1), two differences from what the steps expected (findings 2 and 3), one note on the environment (finding 4).
- Part B (person steps): 4 steps, all open. Nothing in Part B was run.
- Screenshots: not in the repository. They are in the scratchpad folder of the run (the folder `SHOTS` below), 8 pictures and the app's log. Each result names its file and says what the picture shows.

## What this covers

Section 10 of `2026-10-13-approvals-v2-design.md` asks for a manual smoke with a local dot: park a shell call, approve it late, see it run once and the run continue; approve another with a rule and see the next run not ask. Part A runs that against a debug build with a real local model, and adds the group decision of one run's approvals (A7). Part B lists what needs a person at the machine, the real data folder or the real `claude` CLI.

## How the agent steps were run

Part A drives the real app from a terminal.

- The app is the debug build, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d`. The real data folder `%LOCALAPPDATA%\dots` is never used. The webview's own profile is moved too (`WEBVIEW2_USER_DATA_FOLDER`), and its debugging port is 9333.
- `DOTS_CONFIRM=yes` answers the native confirmation in a debug build.
- The window is driven through WebView2's debugging port with `tools/smoke/webview.mjs`: `click` and `fill` act on a control by its accessible name, `eval` calls a command through the Tauri bridge, `text` and `shot` read the page.
- The model server is LM Studio (`~/.lmstudio/bin/lms.exe`), port 1234.
- Every block is run in Git Bash from the repository root. Shell variables do not have to survive from one block to the next: block A2 writes a state file, and every later block starts by reading it (`. "${E:?}"`, where `E` is `$SHOTS/env.sh`).
- Git Bash rewrites an argument that starts with `/` into a Windows path. The blocks use no such argument: `taskkill //PID`, routes without the leading slash.
- Safety: the installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) and the real `claude` CLI are never started or stopped by these blocks. Only the `dots-app.exe` that runs from `target\debug` is stopped, by its process id. See finding 4 for what Dots itself does at start.

## Part A: agent steps

### A1. Nothing of ours is running; build

```bash
tasklist | grep -i -E 'fake-claude' && echo "STOP: a fake CLI is running" || echo "no fake CLI running"
powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | ForEach-Object { \$_.ExecutablePath }"
bun install --frozen-lockfile 2>&1 | tail -1
bun run tauri build --debug --no-bundle 2>&1 | tail -2
node --test tools/smoke/webview.test.mjs 2>&1 | grep -E '^ℹ (tests|pass|fail)'
```

Expect: no fake CLI; the only `dots-app.exe` that may be listed is the installed one under `AppData\Local\Dots`, which is left alone. If one under `target\` is listed, stop: it blocks the build and is not ours to stop. The debug build finishes and the helper's 15 tests pass.

Result: pass. `no fake CLI running`. The one `dots-app.exe` listed was `C:\Users\aksha\AppData\Local\Dots\dots-app.exe`, the installed one, left alone. `bun install`: "Checked 572 installs across 632 packages (no changes)". The build (run with `CARGO_TARGET_DIR=C:/Users/aksha/dots/target`) finished: "Finished `dev` profile … in 2m 51s", "Built application at: C:\Users\aksha\dots\target\debug\dots-app.exe". The helper's tests: `tests 15`, `pass 15`, `fail 0`.

### A2. Temporary folders, a scratch folder for the dot, the state file

`SHOTS` is a folder outside the repository, for the screenshots and the state file.

```bash
: "${SHOTS:?set SHOTS to a folder outside the repository}"
mkdir -p "$SHOTS"; E="$SHOTS/env.sh"
SMOKE="$(mktemp -d)"
mkdir "$SMOKE/data" "$SMOKE/work"
printf 'hello\n' > "$SMOKE/work/README.md"
{
  echo 'set -u'
  printf 'SHOTS=%q\nSMOKE=%q\nE=%q\nAPP=%q\n' "$SHOTS" "$SMOKE" "$E" "$(pwd)/target/debug/dots-app.exe"
  printf 'WORK=%q\n' "$(cygpath -m "$SMOKE/work")"
} > "$E"
cat >> "$E" <<'EOF'
W="node tools/smoke/webview.mjs 9333"
LMS="$HOME/.lmstudio/bin/lms.exe"
val() { tr -d '"\n '; }
wait_page() { for _ in $(seq 1 60); do node tools/smoke/webview.mjs "$1" eval 'invoke("startup_error")' > /dev/null 2>&1 && break; sleep 1; done; sleep 1; }
# The process id of the app this smoke started: the one whose program is under target, not the installed one.
app_pid() { powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | Where-Object { \$_.ExecutablePath -like '*\\target\\debug\\*' } | ForEach-Object { \$_.ProcessId }" | tr -d '\r'; }
start_app() {
  ( DOTS_DATA_DIR="$(cygpath -w "$SMOKE/data")" DOTS_CONFIRM=yes \
    WEBVIEW2_USER_DATA_FOLDER="$(cygpath -w "$SMOKE/webview")" \
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
    "$APP" > "$SMOKE/app.out" 2>&1 & )
  wait_page 9333
}
# Waits until a run has left `running` and prints its status.
settled() { for _ in $(seq 1 150); do S=$($W eval "(await invoke('get_run', { id: '$1' })).status" | val); [ "$S" != "running" ] && [ "$S" != "queued" ] && break; sleep 2; done; echo "$S"; }
EOF
. "$E"; ls "$SMOKE"
```

Expect: the folders `data` and `work`.

Result: pass. `SMOKE` was `/tmp/tmp.nmj0unIsuT` (`%TEMP%\tmp.nmj0unIsuT`); it held `data/` and `work/`. Two deviations from the block as written, both about where the files are: `SHOTS` was `C:/Users/aksha/AppData/Local/Temp/claude/C--Users-aksha-dots/<session>/scratchpad/smoke-approvals-v2`, and `APP` was set to `C:/Users/aksha/dots/target/debug/dots-app.exe` (the shared target folder) instead of `$(pwd)/target/debug/dots-app.exe`.

### A3. The app starts on the temporary data folder; the model server

```bash
. "${E:?}"
start_app
$W eval '(await invoke("status")).data_dir'
timeout 60 "$LMS" ps < /dev/null 2>&1 | grep -v '^$' | cut -c1-110
timeout 60 "$LMS" server start < /dev/null 2>&1 | tail -2
timeout 600 "$LMS" load google/gemma-4-12b-qat -c 8192 -y < /dev/null 2>&1 | tail -2
```

Expect: the data folder is the temporary one. The first `lms ps` shows what was loaded before this smoke: write it into the result, and at the end undo only what this smoke started. Then the server runs on port 1234 with the model loaded.

Result: pass. `data_dir`: `C:\Users\aksha\AppData\Local\Temp\tmp.nmj0unIsuT\data`. Before this smoke `lms ps` said "No models are currently loaded." (it first printed "Waking up LM Studio service..."), and `lms server status` said "The server is running on port 1234." So the server was already up before `server start`, and this smoke started nothing but the model. `server start`: "Success! Server is now running on port 1234". `load`: "(6.66 GiB)", "To use the model in the API/SDK, use the identifier "google/gemma-4-12b-qat"."

### A4. A local dot whose shell call parks at once

The dot keeps the Sandboxed rules, under which a command that is not `git status`, `git diff` or `git add` asks. `approval_wait_secs` is 0, so the call parks without waiting.

```bash
. "${E:?}"
DOT=$($W eval "(async () => { const spec = await invoke('new_dot_spec'); return (await invoke('create_dot', { spec: { ...spec, name: 'late-dot', engine: 'local', model: 'google/gemma-4-12b-qat', endpoint_url: 'http://127.0.0.1:1234', workdir: '$WORK', workspace_mode: 'folder', approval_wait_secs: 0, instructions: 'Run exactly this command with the Bash tool, once: Add-Content -Path applied.log -Value ran . If the tool answers that the call is queued for approval, stop and say so. If you are told that the action was performed, say in one sentence what happened.' } })).id; })()" | val); echo "DOT=$DOT" >> "$E"
RUN=$($W eval "(await invoke('run_now', { dotId: '$DOT' })).id" | val); echo "RUN=$RUN" >> "$E"
echo "parent: $(settled "$RUN")"
$W goto inbox; $W wait "late-dot asked" 300
$W text | sed -n '/^Approvals$/,$p'; $W shot "$SHOTS/01-parked-card.png"
ls "$SMOKE/work"
```

Expect: the run ends `awaiting_approval`. The Inbox shows the card for `late-dot` with `Add-Content -Path applied.log -Value ran`, the sentence "The run stopped waiting and is parked. If you approve, Dots performs exactly this call and then continues the run.", and the buttons "Approve", "Deny" and "Approve and always allow". `applied.log` does not exist. If the model issued a different command, write down what it issued and go on with that command: the step is about what happens to the call that was asked.

Result: pass. The model issued the command asked for: `Add-Content -Path applied.log -Value ran`. `parent: awaiting_approval`. The Inbox text: "late-dot asked just now · Open the run", "Bash", the command, its input as JSON, "The run stopped waiting and is parked. If you approve, Dots performs exactly this call and then continues the run.", "Note for the dot", "Approve", "Deny", "Approve and always allow". `01-parked-card.png` shows the Inbox with the badge 1: one amber card, "Bash" with the command, the JSON input in a dark box, the parked sentence under it, then the note field and the three buttons. The two banners above it are discussed in finding 4. `ls "$SMOKE/work"`: `README.md` only, so `applied.log` does not exist.

### A5. A late approval is performed once by Dots

```bash
. "${E:?}"
$W click "Approve"
for _ in $(seq 1 60); do CHILD=$($W eval "(await invoke('run_thread', { runId: '$RUN' })).at(-1).id" | val); [ "$CHILD" != "$RUN" ] && break; sleep 1; done; echo "CHILD=$CHILD" >> "$E"
echo "child: $(settled "$CHILD")"
cat "$SMOKE/work/applied.log"; wc -l < "$SMOKE/work/applied.log"
$W eval "(await invoke('approvals_for_run', { runId: '$RUN' })).map(a => [a.self_apply, a.apply_state, a.applied_at !== null])"
$W goto "runs/$CHILD"; sleep 2; $W text | sed -n '/^late-dot run$/,$p'; $W shot "$SHOTS/02-performed.png"
```

Expect: a resume run exists and ends `succeeded`. `applied.log` has exactly one line, `ran`. The approval is `[true, "applied", true]`. The run view lists "Approved calls" with "Bash", the command and "Performed by Dots", and the transcript shows the call with its result under the resume run. The model's last message says that the action was performed; it issued no second call (the transcript has one `Bash` call in the resume run, the one Dots made).

Result: pass. A resume run existed and `settled` printed `child: succeeded`. `applied.log`: the line `ran`, 1 line. The approval: `[true, "applied", true]`. `run_thread` of the parent: `[["manual", "succeeded"], ["resume", "succeeded"]]`. The run view text: "late-dot run", "Succeeded", "Open folder", "Took 3m 43s", "Tokens 3091 in, 494 out", the model's sentence "I ran the command to add "ran" to applied.log, which was successfully approved and executed.", then a row "Bash", the command, "Performed by Dots"; then "Manual run … Succeeded" with "Bash … failed" and "The call is queued for approval."; then "Continued after approval … Succeeded" with "Bash … done" and the model's sentence again. One `Bash` call in the resume run, the one Dots made; the model issued no second call. `02-performed.png` shows that: a green "Succeeded" badge and "Open folder", the sentence in a box, a row with `Bash`, the command and "Performed by Dots" at its right, and under it the two parts of the transcript, "failed" in red in the first and "done" in grey in the second. The words "Approved calls" are not on the page as text: they are the accessible name of the list (`ul[aria-label="Approved calls"]`), whose one item reads "Bash / Add-Content -Path applied.log -Value ran / Performed by Dots". "Took 3m 43s" counts from the first queueing, so it includes the time the run was parked (the parent was queued at 10:05 am and the resume run at 10:08 am).

### A6. An approval with a rule, and the next run does not ask

```bash
. "${E:?}"
RUN2=$($W eval "(await invoke('run_now', { dotId: '$DOT' })).id" | val); echo "parent: $(settled "$RUN2")"
$W goto inbox; $W wait "late-dot asked" 300
$W click "Approve and always allow"; $W wait "Always allow for late-dot"
$W text | sed -n '/^Approvals$/,$p'; $W shot "$SHOTS/03-rules.png"
$W click "Approve and save rule"
for _ in $(seq 1 60); do CHILD2=$($W eval "(await invoke('run_thread', { runId: '$RUN2' })).at(-1).id" | val); [ "$CHILD2" != "$RUN2" ] && break; sleep 1; done
echo "child: $(settled "$CHILD2")"
$W eval "(async () => { const p = (await invoke('get_dot', { id: '$DOT' })).policy; return [p.preset, p.rules.at(-1)]; })()"
RUN3=$($W eval "(await invoke('run_now', { dotId: '$DOT' })).id" | val); echo "third: $(settled "$RUN3")"
$W eval "(await invoke('approvals_for_run', { runId: '$RUN3' })).length"
wc -l < "$SMOKE/work/applied.log"
$W goto "dots/$DOT"; sleep 1; $W shot "$SHOTS/04-editor-rule.png"
```

Expect: the list shows three rules, the first chosen: `Bash(Add-Content -Path applied.log -Value ran)` "this exact command", `Bash(Add-Content -Path applied.log -Value:*)`, `Bash(Add-Content -Path:*)`, and the sentence "A deny rule still wins over this rule.". After saving, the policy's preset is `custom` and its last rule is `{"tool": "Bash(Add-Content -Path applied.log -Value ran)", "action": "allow"}`. The third run ends `succeeded` without parking, with 0 approvals, and `applied.log` has three lines. The dot editor shows the rule in its table and the preset "Custom". (A model that words the command differently in the third run is asked again: the rule is exact. Record it; it is the rule working, not a defect.)

Result: pass for the rule and the third run; two things differ from "Expect" (findings 1 and 2). The second run parked (`awaiting_approval`) with the same command. The panel "Always allow for late-dot" listed two rules, not three: `Bash(Add-Content -Path applied.log -Value ran)` "this exact command" (chosen) and `Bash(Add-Content -Path applied.log -Value:*)` "commands that start with Add-Content -Path applied.log -Value", then "A deny rule still wins over this rule.", "Approve and save rule" and "Cancel". The third rule of "Expect", `Bash(Add-Content -Path:*)`, is left out on purpose by `allow_candidates` (finding 2). `03-rules.png` shows the card with its three buttons and, below the note field, a bordered group "Always allow for late-dot" with a selected radio on the exact command, an empty radio on the prefix rule, the deny sentence, a white "Approve and save rule" button and "Cancel". After "Approve and save rule": the resume run `succeeded`; the policy `["custom", {"tool": "Bash(Add-Content -Path applied.log -Value ran)", "action": "allow"}]`. The third run (the model issued the same command) `succeeded` without parking, `approvals_for_run` length 0, and `applied.log` had 3 lines. The dot editor opened by `goto` showed the old policy (finding 1): `04-editor-rule.png` shows only the top of the editor (the name `late-dot`, the instructions, "Enabled", the engine "Local model (OpenAI-compatible endpoint)", the model, the endpoint, "Delete", "Run now" and "Save"), and a second picture scrolled to "Permissions", `04b-editor-permissions.png`, shows the Preset "Sandboxed: may edit inside its folder, asks for the rest" and the 17 rules of that preset, with no `Add-Content` rule. After a reload of the page, `04c-editor-after-reload.png` shows the table ending with `Bash(Add-Content -Path applied.log -Value ran)` set to "allo(w)", the "Add rule" button, "Anything else: ask", and the select of the preset reads `custom`. The cut-off word in the action boxes ("allo", "den") is the cosmetic defect already known from the Plan 3 smoke.

### A7. Two calls of one run, answered together

```bash
. "${E:?}"
TWO=$($W eval "(async () => { const spec = await invoke('new_dot_spec'); return (await invoke('create_dot', { spec: { ...spec, name: 'two-calls', engine: 'local', model: 'google/gemma-4-12b-qat', endpoint_url: 'http://127.0.0.1:1234', workdir: '$WORK', workspace_mode: 'folder', approval_wait_secs: 0, instructions: 'Make two tool calls with the Bash tool, in this order: first Add-Content -Path order.log -Value one , then Add-Content -Path order.log -Value two . If a call is queued for approval, still make the other one, then stop.' } })).id; })()" | val)
RUN4=$($W eval "(await invoke('run_now', { dotId: '$TWO' })).id" | val); echo "parent: $(settled "$RUN4")"
$W eval "(await invoke('approvals_for_run', { runId: '$RUN4' })).length"
$W goto inbox; $W wait "Approve all (2)" 60; $W shot "$SHOTS/05-group.png"
$W click "Approve all (2)"; $W wait "Approve all 2 calls?"; $W shot "$SHOTS/06-confirm.png"
$W click "Approve all"
for _ in $(seq 1 60); do CHILD4=$($W eval "(await invoke('run_thread', { runId: '$RUN4' })).at(-1).id" | val); [ "$CHILD4" != "$RUN4" ] && break; sleep 1; done
echo "child: $(settled "$CHILD4")"; cat "$SMOKE/work/order.log"
$W eval "(await invoke('run_thread', { runId: '$RUN4' })).length"
```

Expect: the run parks with two approvals, and the Inbox shows them in one group with "Approve all (2)" and "Deny all (2)". The dialog names `two-calls` and lists both commands. After the confirmation there is one resume run (the thread has 2 runs), and `order.log` holds `one`, then `two`. If the model made only one call, the group has no batch buttons: record it and repeat the step once; a model that never issues two calls leaves this step to the tests of Task 6 and Task 9, which is then said under "What Part A does not show".

Result: pass, on the first run; the model made both calls. `parent: awaiting_approval`; two approvals, `Add-Content -Path order.log -Value one` and `… -Value two`, in that order. `05-group.png` shows the Inbox with the badge 2: one line "two-calls asked just now · Open the run" with the buttons "Approve all (2)" and "Deny all (2)" at its right, and under it the two amber cards, each with its own "Bash" command, JSON, the parked sentence, note field and its three buttons. After the click, the dialog text: "Approve all 2 calls?", "two-calls may perform every call this run waits for, in the order it asked:", the two lines `Bash Add-Content -Path order.log -Value one` and `… two`, "Cancel", "Approve all". `06-confirm.png` shows that dialog centred on the blurred Inbox. After "Approve all": the resume run `succeeded`; `order.log` held `one` then `two`; the thread had 2 runs; both approvals were `["approved", "applied"]`.

### A8. Stop and clean up

```bash
. "${E:?}"
PID=$(app_pid); echo "stopping $PID"; [ -n "$PID" ] && taskkill //PID "$PID" //F
cp "$SMOKE"/data/logs/dots.*.log "$SHOTS/"
[ -d "$SMOKE/data" ] && rm -rf "$SMOKE"; ls -d "$SMOKE" 2>&1 | sed 's#/tmp/tmp\.[A-Za-z0-9]*#$SMOKE#'
```

Then unload the model and stop the server only if step A3 showed that this smoke started them: `timeout 60 "$LMS" unload --all < /dev/null`, `timeout 60 "$LMS" server stop < /dev/null`.

Expect: the one process this smoke started is stopped; the temporary folder is gone; the app's log is kept beside the screenshots. Look at every screenshot and say in the results what it shows.

Result: pass. `stopping 4876`; the process's path was `C:\Users\aksha\dots\target\debug\dots-app.exe`; `taskkill`: "SUCCESS: The process with PID 4876 has been terminated." The log (`dots.2026-10-14.log`, 3 lines) was copied beside the screenshots; its one error line is the port conflict of finding 4. The temporary folder was deleted (`ls -d`: "cannot access '$SMOKE'"). The only `dots-app.exe` still running afterwards is the installed one (a different process id). The model was loaded by this smoke, so `lms unload --all` ("Unloaded 1 model.") put `lms ps` back to "No models are currently loaded."; the server was already running before A3, so it was not stopped. Every screenshot was looked at, and what each shows is written under its step above.

## Findings of Part A

1. The dot editor can show a policy from before a rule was saved from the Inbox (not explained, a defect). After A6 saved the rule, the dot's policy in the core was `custom` with the rule last (`get_dot`). The editor opened afterwards with `goto "dots/$DOT"` showed the preset "Sandboxed" and 17 rules without it (`04b-editor-permissions.png`); after a reload of the page it showed `custom` and the rule (`04c-editor-after-reload.png`). `ApprovalCard` calls `invalidateDots` after a decision with a rule, so the cache should have been refreshed, and the cause was not looked for (the steps do not change product code). A person who pressed "Save" in the stale editor would write the old policy over the new rule. The next step would be a component test that decides an approval with a rule and then opens the editor for the same dot without a reload, and a run of this step again to see whether it always happens.
2. A6 expected three rule candidates and the Inbox offered two. `allow_candidates` leaves a two-token prefix out when its second token starts with `-` or `/` (here `-Path`), so `Bash(Add-Content -Path:*)` is not offered; its doc comment says so. The expectation in this document was wrong, not the product.
3. The editor's screenshot taken by the step (`04-editor-rule.png`) shows the top of the page, not the rule table, because the page is not scrolled. A step that wants the table in the picture has to scroll to "Permissions" first (as was done for `04b` and `04c`). Not a product defect.
4. Environment, not a product defect. The banner "The local server is not running: could not listen on 127.0.0.1:47321 … (os error 10048). Webhooks are off and Claude dots cannot run." was on every screen, because the installed Dots holds port 47321. The log has the same line. Local dots do not need the port, so no step was affected. The second banner, "Claude CLI 2.1.299 has not been tested with Dots (tested: 2.1.295).", shows that Dots found the real `claude` CLI on this machine's `PATH` and read its version when it started (the Plan 3 smoke started the app with a `PATH` that left the CLI out, and this one did not). No dot of this smoke is a Claude dot and no run used the CLI; the steps did not run it, but Dots reports the version of the CLI it found (`claude.version` 2.1.299), so it must have asked the CLI for it itself. A later run should use the clean `PATH` of the Plan 3 smoke.

## What Part A does not show

- B1 to B4 below: a parked approval after more than four minutes in the installed app, an approved call that is interrupted by closing Dots, and the Claude engine with the real CLI.
- A call that fails when Dots performs it (a non-zero exit, a timeout), a denied call in a group (`Deny all`), and a group of approvals of a run that was cancelled in the meantime. The tests of Tasks 4, 6 and 9 cover them; this run did not drive them.
- A model that words the command differently in the third run of A6 was not seen: the model repeated the exact command every time, so the rule's exactness is shown only by the unit tests.
- The wait of A5 was a few minutes, not a measured four. The brief's limit of a long wait is B1.

## Part B: person steps

Each line needs a person at the machine. Write the result under it.

- B1. With the installed or a release build and the real data folder: a local dot's call that parks, approved from the Inbox after more than four minutes, runs once.

  Result:

- B2. Close Dots from the tray while an approved call runs (a command that sleeps for a minute). Start it again. The approval says "Outcome unknown: Dots stopped while this ran", the run is failed as interrupted, and the command is not run again.

  Result:

- B3. A Claude dot with the real `claude` CLI: a parked approval still resumes the old way (the model repeats the call), and its card reads "Approving continues it in a new run."

  Result:

- B4. A Claude dot: "Approve and always allow" saves the rule, and the next run of the dot does not ask for that command.

  Result:
