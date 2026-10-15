# Scheduler v2: smoke test

- Date: 2026-10-15
- Build: debug (`bun run tauri build --debug --no-bundle`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/scheduler-v2` at commit `19ce683` (Tasks 1 to 8, with their fix rounds). Windows 11, local time UTC-4. The local model is `google/gemma-4-12b-qat` in LM Studio, loaded with a context of 8192.
- Part A (agent steps): 10 steps, 10 as expected in what Dots did. Four blocks of the plan had to be corrected on the way (a command shape or a page text that the block assumed wrongly, findings 1 and 2); the blocks below are the corrected ones, as they were run. 4 findings in all: two defects of the smoke's own blocks, one coalesced slot that the steps did not foresee, one note on the environment.
- Part B (person steps): 4 steps, all open. Nothing in Part B was run.
- Screenshots: not in the repository. They are in the scratchpad folder of the run (the folder `SHOTS` below), 9 pictures and the app's log. Each result names its file and says what the picture shows.

## What this covers

Section 12 of `2026-10-14-scheduler-v2-design.md` asks for a smoke with LM Studio and a temporary data folder: a dot every minute runs; Dots closed across a slot and started again runs one catch-up run; with the model server down the attempts are retried and then "Needs attention" appears with one notification; the server back and "Run now" clear the flag; a `Once` dot runs once and shows "Completed". Part A runs that against a debug build with a real local model. Part B lists what needs a person at the machine, the real data folder or a real notification.

## How the agent steps were run

Part A drives the real app from a terminal.

- The app is the debug build in the shared target folder, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d`. The real data folder `%LOCALAPPDATA%\dots` is never used. The webview's own profile is moved too (`WEBVIEW2_USER_DATA_FOLDER`), and its debugging port is 9333.
- The app is started with a `PATH` that holds no `claude` CLI (block A2 leaves out every folder that has one), so no run can reach the real CLI.
- `DOTS_CONFIRM=yes` answers the native confirmation in a debug build.
- The window is driven through WebView2's debugging port with `tools/smoke/webview.mjs`: `click` and `fill` act on a control by its accessible name, `eval` calls a command through the Tauri bridge, `text` and `shot` read the page.
- The model server is LM Studio (`~/.lmstudio/bin/lms.exe`), port 1234. It is never stopped by a step. A dot that should find it down points at `http://127.0.0.1:9`, where nothing listens: Dots sees a refused connection before any tool call, as when the server is stopped.
- A schedule names local minutes, so the steps compute "two minutes from now" with `date`. A5 to A9 are not run within ten minutes of midnight.
- Every block is run in Git Bash from the repository root. Shell variables do not have to survive from one block to the next: block A2 writes a state file, and every later block starts by reading it (`. "${E:?}"`, where `E` is `$SHOTS/env.sh`).
- Git Bash rewrites an argument that starts with `/` into a Windows path. The blocks use no such argument: `taskkill //PID`, routes without the leading slash.
- Safety: the installed Dots (`%LOCALAPPDATA%\Dots\dots-app.exe`) and the real `claude` CLI are never started or stopped by these blocks. Only the `dots-app.exe` that runs from `target\debug` is stopped, by its process id.

## Part A: agent steps

### A1. Nothing of ours is running; build

```bash
tasklist | grep -i -E 'fake-claude' && echo "STOP: a fake CLI is running" || echo "no fake CLI running"
powershell -NoProfile -Command "Get-CimInstance Win32_Process -Filter \"Name='dots-app.exe'\" | ForEach-Object { \$_.ExecutablePath }"
bun install --frozen-lockfile 2>&1 | tail -1
CARGO_TARGET_DIR=C:/Users/aksha/dots/target bun run tauri build --debug --no-bundle 2>&1 | tail -2
node --test tools/smoke/webview.test.mjs 2>&1 | grep -E '^ℹ (tests|pass|fail)'
```

Expect: no fake CLI; the only `dots-app.exe` that may be listed is the installed one under `AppData\Local\Dots`, which is left alone. If one under `target` is listed, stop: it blocks the build and is not ours to stop. The debug build finishes ("Built application at: C:\Users\aksha\dots\target\debug\dots-app.exe") and the helper's 15 tests pass.

Result: pass. `no fake CLI running`. The one `dots-app.exe` listed was `C:\Users\aksha\AppData\Local\Dots\dots-app.exe`, the installed one, left alone. `bun install`: "Checked 572 installs across 632 packages (no changes)". The build finished: "Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 06s", "Built application at: C:\Users\aksha\dots\target\debug\dots-app.exe". The helper's tests: `tests 15`, `pass 15`, `fail 0`.

### A2. Temporary folders, a clean PATH, the state file

`SHOTS` is a folder outside the repository, for the screenshots and the state file. Set it before this block.

```bash
: "${SHOTS:?set SHOTS to a folder outside the repository}"
mkdir -p "$SHOTS"; E="$SHOTS/env.sh"
SMOKE="$(mktemp -d)"
mkdir "$SMOKE/data" "$SMOKE/work"
printf 'hello\n' > "$SMOKE/work/README.md"
# A PATH without any folder that holds the real claude CLI.
CLEAN_PATH=""; IFS=':' read -ra PARTS <<< "$PATH"
for p in "${PARTS[@]}"; do
  if [ -e "$p/claude" ] || [ -e "$p/claude.exe" ] || [ -e "$p/claude.cmd" ]; then echo "left out of PATH: $p"
  else CLEAN_PATH="$CLEAN_PATH${CLEAN_PATH:+:}$p"; fi
done
{
  echo 'set -u'
  printf 'SHOTS=%q\nSMOKE=%q\nE=%q\nCLEAN_PATH=%q\n' "$SHOTS" "$SMOKE" "$E" "$CLEAN_PATH"
  printf 'APP=%q\nWORK=%q\n' "C:/Users/aksha/dots/target/debug/dots-app.exe" "$(cygpath -m "$SMOKE/work")"
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
# A local time some minutes ahead: HH:MM.
local_hm() { date -d "+$1 min" +%H:%M; }
# A local dot that answers in one word and calls no tool. <name> <schedule as a JS object> <endpoint>
new_dot() { $W eval "(async () => { const spec = await invoke('new_dot_spec'); return (await invoke('create_dot', { spec: { ...spec, name: '$1', engine: 'local', model: 'google/gemma-4-12b-qat', endpoint_url: '$3', workdir: '$WORK', workspace_mode: 'folder', schedule: $2, instructions: 'Reply with the one word done. Do not call any tool.' } })).id; })()" | val; }
# The runs of a dot, oldest first: trigger, status, payload, retry_of, queued_at.
runs() { $W eval "(await invoke('list_runs', { dotId: '$1', limit: 50 })).reverse().map(r => [r.trigger, r.status, r.payload, r.retry_of, r.queued_at])"; }
# What the overview says of a dot's schedule.
view() { $W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$1').schedule_state"; }
# Counts the schedule_attention events that reach the page from now on (window.__attention).
listen_attention() { $W eval "(async () => { const T = window.__TAURI_INTERNALS__; window.__attention = 0; await T.invoke('plugin:event|listen', { event: 'runtime', target: { kind: 'Any' }, handler: T.transformCallback((e) => { if (e.payload && e.payload.type === 'schedule_attention') window.__attention += 1; }) }); return 'listening'; })()"; }
EOF
. "$E"; ls "$SMOKE"; PATH="$CLEAN_PATH" which claude 2> /dev/null || echo "no claude on the clean PATH"
```

Expect: the folders `data` and `work`; one or more "left out of PATH" lines when the real CLI is installed; `no claude on the clean PATH`.

Result: pass. `SHOTS` was `C:/Users/aksha/AppData/Local/Temp/claude/C--Users-aksha-dots/5f801379-3be9-46b5-8f0d-fa7a43eda6aa/scratchpad/sched-smoke`. Two "left out of PATH" lines, both `/c/Users/aksha/AppData/Roaming/npm` (the folder with the real `claude` shim, listed twice in the PATH). `ls` showed `data/` and `work/`. `no claude on the clean PATH`.

### A3. The app starts on the temporary data folder; the model server

```bash
. "${E:?}"
start_app
$W eval '(await invoke("status")).data_dir'
$W text | head -8
timeout 60 "$LMS" ps < /dev/null 2>&1 | grep -v '^$' | cut -c1-110
timeout 60 "$LMS" server status < /dev/null 2>&1 | tail -1
timeout 60 "$LMS" server start < /dev/null 2>&1 | tail -2
timeout 600 "$LMS" load google/gemma-4-12b-qat -c 8192 -y < /dev/null 2>&1 | tail -2
curl -s -m 3 http://127.0.0.1:9/ > /dev/null && echo "STOP: something answers on port 9" || echo "nothing answers on port 9"
```

Expect: the data folder is the temporary one. The banner "The local server is not running: could not listen on 127.0.0.1:47321 …" is expected: the installed Dots holds that port, and local dots do not need it. No banner names a version of the Claude CLI (the clean `PATH` worked). The first `lms ps` and `server status` show what ran before this smoke: write it into the result, and at the end undo only what this smoke started. Then the server runs on port 1234 with the model loaded, and nothing answers on port 9.

Result: pass. The data folder was `C:\Users\aksha\AppData\Local\Temp\tmp.sJygpUtbMz\data`. The page showed the sidebar (Dots, Inbox, Chat, Settings) and two banners: the port-47321 one ("could not listen on 127.0.0.1:47321: Only one usage of each socket address … (os error 10048). Webhooks are off and Claude dots cannot run.") and "The claude CLI was not found. Claude dots cannot run until it is installed or its path is set." The second is what the clean `PATH` should give: it names no version. What ran before: `lms ps` printed "Waking up LM Studio service..." and "No models are currently loaded."; `server status` then answered "The server is running on port 1234." (the wake-up of the service by `lms ps` may be what started it; the dispatch of this task saw nothing on port 1234 beforehand). `server start` answered "Success! Server is now running on port 1234"; `load` finished (6.66 GiB, identifier "google/gemma-4-12b-qat"); `nothing answers on port 9`. This smoke is therefore treated as having started the server and loaded the model, and A10 undoes both.

### A4. A dot every minute runs

The block as run differs from the plan's in one line: `get_dot` answers the dot itself, so the stored schedule is `.schedule`, not `.spec.schedule` (finding 1).

```bash
. "${E:?}"
EVERY=$(new_dot every-dot "{ rule: { kind: 'every', minutes: 1, anchor: null }, catch_up: true, until: null }" http://127.0.0.1:1234); echo "EVERY=$EVERY" >> "$E"
$W eval "(await invoke('get_dot', { id: '$EVERY' })).schedule"; view "$EVERY"
for _ in $(seq 1 90); do N=$($W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 50 })).length" | val); [ "$N" -ge 2 ] && break; sleep 2; done
FIRST=$($W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 50 })).at(-1).id" | val); echo "first: $(settled "$FIRST")"
runs "$EVERY"; view "$EVERY"
$W goto dots; sleep 2; $W text | sed -n '/^Dots$/,$p' | head -20; $W shot "$SHOTS/01-every.png"
$W goto "runs/$FIRST"; sleep 2; $W text | grep -E 'Scheduled'; $W shot "$SHOTS/02-every-run.png"
```

Expect: the stored schedule has an `anchor` the core set (RFC 3339 UTC). The first view is `active`, `fired_count` 0, a `next_fire` less than a minute ahead in local time, no retry, no attention. Within about two minutes two runs exist, trigger `schedule`, payload `{"scheduled_for": …}` one minute apart and without `caught_up`; the first ends `succeeded`. The view counts them. The Dots list shows `every-dot` with its next run and no badge; the run view shows "Scheduled run" and "Scheduled for <date, time>".

Result: pass. The block's first `get_dot` line threw as in the plan (`.spec` is undefined); with `.schedule` the stored schedule is `{"rule": {"kind": "every", "minutes": 1, "anchor": "2026-10-15T14:16:42.992Z"}, "catch_up": true, "until": null}`. First view: `active`, `fired_count` 0, `next_fire` `2026-10-15T10:17:42.992-04:00` (at 10:16:42 local), `retry_at`, `retry_attempt` and `attention` null. The first run ended `succeeded`. Two runs, trigger `schedule`, no `caught_up`, payloads `scheduled_for` `10:17:42.992-04:00` and `10:18:42.992-04:00`, queued at `14:17:43.007Z` and `14:18:43.010Z` (one minute apart). The view then said `fired_count` 3, not 2: a third slot (10:19:42) had fired by then and was coalesced into the second run, which was still queued or running (see finding 3). Screenshot `01-every.png`: the Dots list with one row, `every-dot`, "local · google/gemma-4-12b-qat", state "Running", next run "15 Oct 2026, 10:20 am", last result "Running 1 min ago", the toggle on, a "Run now" button, no badge; above the list the two banners of A3. Screenshot `02-every-run.png`: "every-dot run", "Succeeded", the line "Scheduled run 15 Oct 2026, 10:17 am" and "Scheduled for 15 Oct 2026, 10:17 am", the model's answer "done", "Took 2m 14s", "Tokens 946 in, 319 out" (the model thinks before it answers, so one run takes longer than a slot).

### A5. Dots closed across a slot and started again runs one catch-up run

Two lines differ from the plan's block: an `echo` of the last status of `every-dot` was added after the first wait, and the last command reads `.enabled`, not `.spec.enabled` (finding 1).

```bash
. "${E:?}"
for _ in $(seq 1 60); do S=$($W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 1 }))[0].status" | val); [ "$S" != running ] && [ "$S" != queued ] && break; sleep 2; done
echo "every last status: $S at $(date +%H:%M:%S)"
AT=$(local_hm 3); DAILY=$(new_dot daily-dot "{ rule: { kind: 'daily', time: '$AT' }, catch_up: true, until: null }" http://127.0.0.1:1234); echo "DAILY=$DAILY" >> "$E"
PID=$(app_pid); taskkill //PID "$PID" //F; echo "closed at $(date +%H:%M:%S); daily slot at $AT"
until [ "$(date +%H:%M)" \> "$AT" ]; do sleep 5; done; sleep 90
STARTED=$(date -u +%Y-%m-%dT%H:%M:%S); echo "STARTED=$STARTED" >> "$E"; echo "started at $(date +%H:%M:%S)"; start_app; sleep 10
runs "$DAILY"; view "$DAILY"
$W eval "(await invoke('list_runs', { dotId: '$EVERY', limit: 50 })).filter(r => r.queued_at > '$STARTED').reverse().map(r => [r.status, r.payload, r.queued_at])"
CATCH=$($W eval "(await invoke('list_runs', { dotId: '$DAILY', limit: 1 }))[0].id" | val); echo "catch-up: $(settled "$CATCH")"
$W goto "runs/$CATCH"; sleep 2; $W text | grep -E 'Scheduled'; $W shot "$SHOTS/03-caught-up.png"
$W eval "(await invoke('set_dot_enabled', { id: '$EVERY', enabled: false })).enabled"
```

Expect: `daily-dot` has exactly one run, trigger `schedule`, payload `{"scheduled_for": <today at AT, local>, "caught_up": true}`, which ends `succeeded`; its view has `fired_count` 1 and `next_fire` tomorrow at `AT`. The run view says "Scheduled for <date, time>, caught up after the PC was off". `every-dot`'s runs queued since the start are one per slot: the first is for the newest slot before the start (with `caught_up: true` when that slot was 60 s or more late), and none is for an older slot of the minutes Dots was closed. If a run of `every-dot` was running at the moment of the close, it is failed as interrupted at the start: write it down; it is not part of this check. `every-dot` is then disabled, so that it stops running every minute.

Result: pass. `every-dot` had settled (`succeeded`) before the close; no run was running at the close, so nothing was interrupted. Closed at 10:20:34 (process 27964, the one under `target`), daily slot 10:23, started again at 10:25:32. `daily-dot` has exactly one run: trigger `schedule`, `{"caught_up": true, "scheduled_for": "2026-10-15T10:23:00-04:00"}`, queued `14:25:35.072Z`, `succeeded`; its view: `active`, `fired_count` 1, `next_fire` `2026-10-16T10:23:00-04:00`, no retry, no attention. The run view reads "Scheduled run 15 Oct 2026, 10:25 am" and "Scheduled for 15 Oct 2026, 10:23 am, caught up after the PC was off". `every-dot` had, since the start, a run for the slot 10:24:42.992 (queued 14:25:35.045Z, no `caught_up`: that slot was 53 s late, under 60 s) and one for the slot 10:25:42.992 (queued 14:25:42.996Z); none for the slots 10:21:42 to 10:23:42 of the minutes Dots was closed. The list of all runs of `every-dot` afterwards: 10:17:42, 10:18:42, 10:24:42, 10:25:42, and the slot 10:19:42 has no run of its own (finding 3). `set_dot_enabled` answered `false`. Screenshot `03-caught-up.png`: "daily-dot run", "Succeeded", "Queued 15 Oct 2026, 10:25 am", "Took 30s", "Tokens 951 in, 6 out", the answer "done", and the lines "Scheduled run 15 Oct 2026, 10:25 am" and "Scheduled for 15 Oct 2026, 10:23 am, caught up after the PC was off".

### A6. The model server is down: the first two retries

```bash
. "${E:?}"
listen_attention
AT=$(local_hm 2); RETRY=$(new_dot retry-dot "{ rule: { kind: 'daily', time: '$AT' }, catch_up: true, until: null }" http://127.0.0.1:9); echo "RETRY=$RETRY" >> "$E"; echo "slot at $AT"
for _ in $(seq 1 120); do A=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.retry_attempt" | val); [ "$A" = 2 ] && break; sleep 2; done
runs "$RETRY"; view "$RETRY"
$W goto dots; sleep 2; $W text | grep -E 'retry-dot|Retrying'; $W shot "$SHOTS/04-retrying.png"
for _ in $(seq 1 120); do A=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.retry_attempt" | val); [ "$A" = 3 ] && break; sleep 2; done
runs "$RETRY"; view "$RETRY"
SECOND=$($W eval "(await invoke('list_runs', { dotId: '$RETRY', limit: 50 })).at(-2).id" | val)
$W goto "runs/$SECOND"; sleep 2; $W text | grep -E 'Retry|Scheduled'; $W shot "$SHOTS/05-retry-run.png"
$W eval "(await invoke('inbox')).review_runs.length"; $W eval 'window.__attention'
```

Expect: `listening`. The slot's run fails within seconds, with an error about the connection to the model server and no tool call. The view then has `retry_attempt` 2 and a `retry_at` one minute after that run ended (local time); the Dots list says "Retrying at <h:mm> (attempt 2 of 6)". The second run has trigger `schedule`, `retry_of` the first run, payload `{"scheduled_for": <the slot>, "attempt": 2}`, and fails the same way; the view then has `retry_attempt` 3, two minutes after it ended. The run view of the second run says "Retry 2 of 6 of the failed run", with the link, and no "Scheduled for". No failed run of `retry-dot` is under "Runs to review" (the count of review runs is 0 unless another dot left one), and `window.__attention` is 0. If `listen_attention` answers an error instead of `listening`, write it down: the count of notifications is then left to B3 and to the test `a_failed_scheduled_run_is_quiet_and_its_attention_notifies`.

Result: pass. `listening`. Slot 10:28. The first run (trigger `schedule`, payload `{"scheduled_for": "2026-10-15T10:28:00-04:00"}`, queued 14:28:00.013Z) failed after 9 s with "local endpoint http://127.0.0.1:9/v1/chat/completions unreachable: error sending request for url (http://127.0.0.1:9/v1/chat/completions) (gave up after 3 attempts)", no tokens. The view: `fired_count` 1, `retry_attempt` 2, `retry_at` `10:29:09.141-04:00`, `next_fire` `2026-10-16T10:28:00-04:00`, attention null. The second run: trigger `schedule`, `retry_of` the first run's id, payload `{"attempt": 2, "scheduled_for": "…10:28:00-04:00"}`, queued 14:29:09.155Z, failed the same way after 9 s; the view then had `retry_attempt` 3, `retry_at` `10:31:18.300-04:00` (two minutes after it ended at 10:29:18). The run view of the second run reads "Scheduled run 15 Oct 2026, 10:29 am" and "Retry 2 of 6 of the failed run"; there is no "Scheduled for". `review_runs` count 0, `window.__attention` 0. Screenshot `04-retrying.png`: the Dots list with `daily-dot` (next run 16 Oct 10:23 am, Succeeded), `every-dot` (Disabled, Succeeded) and `retry-dot` (state Idle, next run "16 Oct 2026, 10:28 am" with the second line "Retrying at 10:29 am (attempt 2 of 6)", last result "Failed just now"). Screenshot `05-retry-run.png`: "retry-dot run", the badge "Failed", the buttons "Open folder" and "Mark reviewed", "Queued 15 Oct 2026, 10:29 am", "Took 9s", "Tokens 0 in, 0 out", the error in red, then "Scheduled run 15 Oct 2026, 10:29 am" and "Retry 2 of 6 of the failed run", where "the failed run" is a link in the picture.

### A7. The attempts are used up: "Needs attention" and one notification

Attempts 3 to 6 start 2, 4, 8 and 16 minutes after the attempt before them ended: the whole occurrence takes 1 + 2 + 4 + 8 + 16 = 31 minutes from the first failure, plus the few seconds of each run, so about 28 minutes remain after A6. The block waits at most 9 minutes, below the 10-minute limit of one command; run it again until it prints `attention set`. The block as run waits for the text "Needs attention" on the Inbox page, not "retry-dot needs attention" (finding 2).

```bash
. "${E:?}"
for _ in $(seq 1 54); do R=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.attention !== null" | val); [ "$R" = true ] && echo "attention set" && break; sleep 10; done
runs "$RETRY"; view "$RETRY"
$W eval 'window.__attention'
$W eval "(await invoke('inbox')).attention"; $W eval "await invoke('inbox_count')"
$W goto inbox; $W wait "Needs attention" 30; $W text | sed -n '/^Needs attention$/,$p' | head -12; $W shot "$SHOTS/06-attention.png"
$W goto dots; sleep 2; $W text | grep -E 'retry-dot|Needs attention'; $W shot "$SHOTS/07-badge.png"
```

Expect: six runs of trigger `schedule`, all `failed`, the first without `attempt` and the others with `attempt` 2 to 6, each with `retry_of` the run before it. The view: `status` `active`, `fired_count` 1, `retry_at` and `retry_attempt` null, `attention` with the sixth run's id and its error, `next_fire` tomorrow at the slot's time. `window.__attention` is 1: one notification for the occurrence, none for the attempts. The Inbox answer has one attention item for `retry-dot`, and `inbox_count` counts it. The Inbox shows "Needs attention" with "retry-dot", "Scheduled run failed … · <error>", "Open the failed run", "Run now" and "Dismiss"; the sidebar's Inbox count includes it. The Dots list shows the badge "Needs attention" beside `retry-dot`.

Result: pass. Four runs of the wait loop (the occurrence ended at 10:59:54, 31 minutes after the first failure); the run starts were 10:28:00, 10:29:09, 10:31:18, 10:35:27, 10:43:36 and 10:59:45 (gaps of 1, 2, 4, 8 and 16 minutes after each failure). Six runs, trigger `schedule`, all `failed`, payloads with no `attempt` for the first and `attempt` 2 to 6 for the others, every `retry_of` the run before it. The view: `active`, `fired_count` 1, `retry_at` null, `retry_attempt` null, `next_fire` `2026-10-16T10:28:00-04:00`, `attention` `{"run_id": "01M50173ESKR54C75A6BE8TA95", "error": "local endpoint … (gave up after 3 attempts)"}`, and that run id is the sixth run's. `window.__attention` 1. The inbox answer's `attention` has one item (`dot_name` `retry-dot`, that run id, `failed_at` `2026-10-15T14:59:54.920Z`); `inbox_count` 1. The app's log has exactly one line for the end: "scheduled run failed in the end: the schedule needs attention dot=retry-dot run=01M50173ESKR54C75A6BE8TA95" at 14:59:54, and five lines "scheduled run failed before any tool call: retry … attempt=2…6" before it. Screenshot `06-attention.png`: the sidebar's Inbox has the count 1; the page "Inbox", the heading "Needs attention", a card "retry-dot" with "Scheduled run failed just now · local endpoint http://127.0.0.1:9/v1/chat/completions unreachable: error sending r…" and the buttons "Open the failed run", "Run now" and "Dismiss". Screenshot `07-badge.png`: the Dots list with the red badge "Needs attention" beside `retry-dot`, its last result "Failed just now", next run 16 Oct 10:28 am; the Inbox count 1 in the sidebar. Deviation: the page text does not hold "retry-dot needs attention" as one phrase (the card's title is "retry-dot" under the heading "Needs attention"), so the plan's `wait` with that phrase timed out after 30 s ("the page did not show … within 30 s"); the page itself was right (finding 2).

### A8. The server back, and "Run now" clears the flag

The block as run reads the dot itself (`{ ...d, endpoint_url }`, not `d.spec`) and waits for "Needs attention" (findings 1 and 2).

```bash
. "${E:?}"
$W eval "(async () => { const d = await invoke('get_dot', { id: '$RETRY' }); return (await invoke('update_dot', { id: '$RETRY', spec: { ...d, endpoint_url: 'http://127.0.0.1:1234' } })).endpoint_url; })()"
$W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$RETRY').schedule_state.attention !== null"
$W goto inbox; $W wait "Needs attention" 30; $W click "Run now"
sleep 3; NOW=$($W eval "(await invoke('list_runs', { dotId: '$RETRY', limit: 1 }))[0].id" | val); echo "run now: $(settled "$NOW") at $(date +%H:%M:%S)"
$W eval "(await invoke('get_run', { id: '$NOW' })).trigger"
for i in $(seq 1 45); do C=$($W eval "(await invoke('inbox')).attention.length" | val); [ "$C" = 0 ] && echo "cleared after about $((i * 2)) s" && break; sleep 2; done
view "$RETRY"; $W eval 'window.__attention'
$W goto inbox; sleep 2; $W shot "$SHOTS/08-cleared.png"
```

Expect: the dot now points at port 1234, and its flag is still set (`true`): the edit did not change the schedule, so the row was not reset. "Run now" opens a new run, trigger `manual`, which ends `succeeded`. Within 60 s after it ended the Inbox has no attention item, the view's `attention` is null, and `window.__attention` is still 1: clearing the flag does not notify.

Result: pass. `update_dot` answered `http://127.0.0.1:1234`; the flag was still set (`true`). `clicked "Run now"`; the new run's trigger is `manual` and it ended `succeeded` (at 11:01:30). The Inbox had no attention item after about 38 s; the view: `active`, `fired_count` 1, `next_fire` `2026-10-16T10:28:00-04:00`, `attention` null; `window.__attention` 1. Screenshot `08-cleared.png`: the page "Inbox" with the sentence "Nothing waits for you. Approvals a dot asks for, schedules that need attention and finished runs to review show up here." and no count beside Inbox in the sidebar.

### A9. A `Once` dot runs once and shows "Completed"

```bash
. "${E:?}"
AT=$(date -d '+2 min' +%Y-%m-%dT%H:%M); ONCE=$(new_dot once-dot "{ rule: { kind: 'once', at: '$AT' }, catch_up: true, until: null }" http://127.0.0.1:1234); echo "ONCE=$ONCE" >> "$E"; echo "once at $AT"
for _ in $(seq 1 120); do S=$($W eval "(await invoke('dot_overview')).find(o => o.dot.id === '$ONCE').schedule_state.status" | val); [ "$S" = completed ] && break; sleep 2; done
sleep 70; runs "$ONCE"; view "$ONCE"
$W goto dots; sleep 2; $W text | grep -E 'once-dot|Completed'; $W shot "$SHOTS/09-once.png"
```

Expect: one run, trigger `schedule`, payload `{"scheduled_for": <AT, local>}`, `succeeded`. The view: `status` `completed`, `fired_count` 1, `next_fire` null. Seventy seconds later there is still one run. The Dots list says "Completed" for `once-dot`.

Result: pass. Once at `2026-10-15T11:04`. After the 70 s: one run, trigger `schedule`, `{"scheduled_for": "2026-10-15T11:04:00-04:00"}`, queued 15:04:00.014Z, `succeeded`; the view `completed`, `fired_count` 1, `next_fire` null, no retry, no attention. Screenshot `09-once.png`: the Dots list with four rows: `daily-dot` (next run 16 Oct 10:23 am, Succeeded 39 min ago), `every-dot` (Disabled, Succeeded 39 min ago), `once-dot` (next-run column "Completed", Succeeded 1 min ago) and `retry-dot` (next run 16 Oct 10:28 am, Succeeded 4 min ago, no badge); no badge on any row.

### A10. Stop and clean up

```bash
. "${E:?}"
PID=$(app_pid); echo "stopping $PID"; [ -n "$PID" ] && taskkill //PID "$PID" //F
cp "$SMOKE"/data/logs/dots.*.log "$SHOTS/"
[ -d "$SMOKE/data" ] && rm -rf "$SMOKE"; ls -d "$SMOKE" 2>&1 | sed 's#/tmp/tmp\.[A-Za-z0-9]*#$SMOKE#'
```

Then unload the model and stop the server only if step A3 showed that this smoke started them: `timeout 60 "$LMS" unload --all < /dev/null`, `timeout 60 "$LMS" server stop < /dev/null`.

Expect: the one process this smoke started is stopped; the temporary folder is gone; the app's log is kept beside the screenshots. Look at every screenshot and say in the results what it shows.

Result: pass. Process 24084 (the app under `target\debug`) stopped; the temporary folder is gone (`ls: cannot access '$SMOKE': No such file or directory`); the log `dots.2026-10-15.log` is in the screenshot folder beside the nine pictures. Because A3 showed no model loaded and the server not answering before the smoke, the model was unloaded ("Unloaded 1 model.") and the server stopped ("Stopped the server on port 1234."); `lms ps` then listed no model. The only `dots-app.exe` left running is the installed one under `AppData\Local\Dots`, untouched. The screenshots are described under each step above.

## Findings of Part A

1. The blocks of A4, A5 and A8 read `get_dot`'s answer as `{ spec: … }`. It is the dot itself, with the spec's fields at the top level (`id`, `name`, `schedule`, `enabled`, `endpoint_url`, …), and the same for `set_dot_enabled` and `update_dot`. The plan's block threw "Cannot read properties of undefined (reading 'schedule')" at A4; at A5 `.spec.enabled` and at A8 `d.spec` would have failed the same way. The blocks above are corrected: `.schedule`, `.enabled`, `{ ...d, endpoint_url }`, `.endpoint_url`. `update_dot` accepts the dot's extra fields (`id`, `created_at`, …) in `spec` without complaint. Not a defect of the product.
2. The plan's `wait "retry-dot needs attention"` (A7 and A8) does not match what the Inbox page shows: the card's title is "retry-dot" under the heading "Needs attention", and the page text never holds the two as one phrase. The wait timed out after 30 s at A7 and the block went on; the screenshot and the text show the page as designed. The blocks above wait for "Needs attention". Not a defect of the product.
3. A4: the runs of a model that thinks (about 320 output tokens for the answer "done") last more than a minute, so a schedule `Every 1 minute` had a run still active at the next slot. The slot of 10:19:42 was "coalesced into the queued run" (log line at 14:19:43), so it has no run of its own, and `fired_count` counts it (3 where the plan expected 2). The plan's A4 expectation of "two runs" and the count of the view are both right for a dot whose run is shorter than a slot; the steps did not foresee this case. As designed.
4. The environment: `lms ps` woke LM Studio's service and the server then reported itself running on port 1234 before `server start` ran; the model was not loaded. The smoke loaded the model and stopped both at A10, and the person's LM Studio had nothing loaded before and has nothing loaded now. The two Task fixes named in the dispatch that the steps do not reach (a cron slot in the fall-back overlap, the status write that skips a reset row) are not exercised here; the fix that a slot firing during a pending retry drops the retry could not show either: `retry-dot` is daily, so its six runs are as the plan expects, and `every-dot` never failed, so no attempt appeared on it.

## What Part A does not show

- B1 to B4 below: a real sleep and wake, an upgrade over a Dots that has a cron dot, the notification Windows shows, and a DST day.
- A schedule that expires: it needs an old cron string that a newer crate refuses.
- A dot deleted or disabled while a retry waits.
- A retry joined into a queued run.
- The notification itself. A7 counts the `schedule_attention` events that reach the page (1, once for the occurrence and none for the attempts); that Windows shows a toast for it is B3.
- The fall-back overlap of a cron slot, the reset-row guard of the pass, and a slot that fires during a pending retry (Task 1 and Task 5 fixes): the unit tests of those tasks cover them, not this smoke.

The tests of Tasks 4 and 5 cover the rest.

## Part B: person steps

These need the person at the machine, the real data folder, or a real notification. Each line has a "Result:" to fill in. None of them was run.

### B1. Sleep and wake

A laptop with a release or debug build. Create a daily dot at a time a few minutes ahead with "Catch up a missed run" on, and a second one at the same time with it off. Put the PC to sleep before that time and wake it at least two minutes after.

Expect: within a minute of the wake the first dot has one run, whose run view says "caught up after the PC was off". The second dot has none, and its next run is tomorrow.

Result:

### B2. Upgrade over a Dots that has a cron dot

A release build with the real data folder, installed over a Dots that has a cron dot.

Expect: after the upgrade the editor shows the kind "Cron expression" with the same expression and "Catch up a missed run" off, and the dot's next run is where it was before.

Result:

### B3. The notification itself

When a schedule gets "Needs attention" (A7 again, or any real failure), look at what Windows shows.

Expect: one notification "<dot>: scheduled run failed" with the error, and none for the failed attempts before it.

Result:

### B4. A DST day

Not tested here: it would need the PC's clock set to a transition night, which changes the machine for everything else that runs on it, and the next real transition may be months away. The rules are checked by the slot tests of Task 1 in a test zone. If the person's PC goes through a DST change while Dots runs a daily or hourly dot, they can look at the run times the next day.

Expect: a daily time in the skipped hour runs an hour later that day; one in the repeated hour runs once.

Result:
