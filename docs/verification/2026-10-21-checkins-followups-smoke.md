# Check-ins and follow-ups: smoke test

- Date: 2026-10-21
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/checkins-followups` at commit `b8dd878` (Tasks 1 to 13 and the two review fixes). Windows 11 on ARM (`aarch64-pc-windows-msvc`), Bun 1.4.3, LLVM for `ring`.
- Model: `qwen2.5:7b` (Q4_K_M) in Ollama 0.40.7, at `http://127.0.0.1:11434`, CPU only (`size_vram` 0), context 4096.
- Part A (agent steps): 3 steps (A1 to A3, A3 in five parts). A1, A2 and A3a to A3d as expected in what Dots did. A3e (notifications) was not observed: the agent cannot see Windows notifications. 2 findings, neither a Dots defect.
- Part B (person steps): 1 step, open. Nothing in Part B was run; the agent never runs the real `claude` CLI.
- Screenshots: not in the repository; in the scratchpad folder of the run (`.../scratchpad/smoke/shots`): `01-inbox-messages.png`, `02-dots-list.png`.

## What this covers

The check-ins spec (section 11) asks for a smoke with a real `claude` dot that asks, is answered, and schedules a follow-up 5 minutes ahead. Part A covers the same paths with a local dot: a note and a question with choices, the answer from the Inbox continuing the thread, a follow-up that fires on time and resumes the session, the follow-up limits, a disabled dot, paused schedules, and a follow-up missed while Dots was closed. Part B is the real `claude` path.

## How the agent steps were run

- The app is the debug build `target/debug/dots-app.exe`, started with `DOTS_DATA_DIR` and `WEBVIEW2_USER_DATA_FOLDER` set to fresh folders in the run's scratchpad. The real data folder `%LOCALAPPDATA%\dots` was never used.
- The app's `PATH` leaves out `~/.local/bin`, the folder that holds the `claude` CLI, so no run could reach it (the app showed "The claude CLI was not found").
- The window is driven through WebView2's debugging port 9333 with `tools/smoke/webview.mjs` (`eval` with `invoke(...)`, `goto`, `click`, `text`, `shot`), as the earlier smokes did.
- Every dot is a local dot (`engine: 'local'`, `model: 'qwen2.5:7b'`, `endpoint_url: 'http://127.0.0.1:11434'`, `workspace_mode: 'folder'`, `approval_wait_secs: 0`, preset `sandboxed`, the default of `new_dot_spec`), made with `create_dot` and run with `run_now`.
- Helpers (shell functions, each one `eval`): `new_dot <name> <instructions>`, `run_now`, `settled <run>` (waits until the run leaves `queued` and `running`), `calls <run>` (tool calls and results), `said <run>`, `msgs` (`messages_list`), `fups` (`follow_ups_list`).
- Times below are UTC; the follow-up tool answers in local time (+05:30).

## Part A: agent steps

### A1. A local dot notes, asks and is answered

```bash
D1=$(new_dot checker "Call the notify tool with the text Starting. Then call the ask tool with the question Ship it? and the choices yes and no. Then finish your turn.")
R1=$(run_now "$D1"); settled "$R1"; calls "$R1"; msgs
$W eval 'invoke("inbox_count")'; $W goto inbox; $W text
$W click "yes"; R2=$(newest "$D1"); settled "$R2"
```

Expect: the run succeeds with a `Notify` call answered "Sent." and an `Ask` call answered "Your question was sent. Finish your turn now; …"; `messages_list` holds the note and the open question; the Inbox's "Messages" group shows both first and the count includes them. "yes" starts a chat run that continues the thread with `Answer to your question "Ship it?": yes`, and the question closes.

Result: pass. Run `01M5EYND76Q0RRHBRMSXA2MTKE` succeeded: `Notify {"text": "Starting."}` → "Sent."; `Ask {"question": "Ship it?", "choices": ["yes", "no"]}` → the `ASKED` sentence. `messages_list`: a `question` "Ship it?" with choices `["yes","no"]` (open) and a `note` "Starting." (unread). `inbox_count` 2; the Inbox showed "Messages" first: "checker asks · just now", "Ship it?", buttons "yes" and "no", the box "Answer to checker", "Reply", "Dismiss"; then "checker says · just now", "Starting.", "Mark read". Clicking "yes" made run `01M5EYRQJSY486D9XC7AAFYY0S`: trigger `chat`, parent `01M5EYND76Q0RRHBRMSXA2MTKE`, payload message `Answer to your question "Ship it?": yes`; it succeeded ("Changes have been confirmed to be shipped…"). The question then had `answered_at` and `read_at`; `inbox_count` 1 (the note).

### A2. A follow-up 5 minutes ahead fires and continues the thread

```bash
D2=$(new_dot later "Call the follow_up tool with the note Check again. and in set to 5m. Then finish your turn.")
R3=$(run_now "$D2"); settled "$R3"; calls "$R3"; fups; $W goto dots; $W text
```

Expect: the result "Follow-up <id> scheduled for <local time>."; the Dots list says "Next follow-up in 5 min". About 5 minutes later a `follow_up` run continues the thread with "Follow-up you scheduled on <time>: Check again.", resuming the session, and the list is empty.

Result: pass. Run `01M5EYSD2QTV8QGS9WRJPSY9BD`: `FollowUp {"in": "5m", "note": "Check again."}` → "Follow-up 01M5EYSSZTC5G8R3EKJRMCNV53 scheduled for Wed 21 Oct 15:41." (due `10:11:09.402Z`). The Dots list showed "Next follow-up in 5 min". Run `01M5EZ2YZCWS792HBVJKAS1QY4` was queued at 10:11:09: trigger `follow_up`, parent `01M5EYSD2QTV8QGS9WRJPSY9BD`, payload `{"follow_up_id": "01M5EYSSZTC5G8R3EKJRMCNV53", "message": "Follow-up you scheduled on Wed 21 Oct 15:36: Check again."}`; its `session_started` names the first run's session `01M5EYSD2QTV8QGS9WRJPSY9BD`; it succeeded ("Follow-up completed…"). `follow_ups_list` was empty after.

### A3. Limits, a disabled dot, paused schedules, a closed Dots, notifications

**A3a. Limits.** `new_dot soon "Call the follow_up tool once with in set to 1m and note a. Then finish your turn."`

Expect: the call is refused with "A follow-up must be at least 5 minutes away." and the run goes on.

Result: pass. Run `01M5F0P3DHFFWTCJ0R4BQDXF6X`: `FollowUp {"in": "1m", …}` → error "A follow-up must be at least 5 minutes away."; the run succeeded. (The model then made unrelated `Read` and `LS` calls; model behaviour.) A first attempt with five follow-up calls in one run (`01M5EYTMGN5BYBBJ80ZEW3RSFK`) never got a model answer and failed with "timeout after 1800s" (finding 1). The pending limit and parallel calls are covered by `checkins_call_test::parallel_calls_store_one_question_and_three_follow_ups`.

**A3b. A disabled dot.** `offdot` scheduled "disabled check" for 16:15 (`10:45:39Z`); `set_dot_enabled(false)` at 10:41:42; read at 10:46:02.

Expect: the follow-up is skipped and a note of Dots' own says why; no run.

Result: pass. `follow_ups_list` no longer held it; `messages_list` for the dot: a `note` with `run_id` null, "A follow-up was skipped because the dot is off. It was due Wed 21 Oct 16:15: disabled check"; the dot had its one manual run only.

**A3c. Paused schedules.** `paused` scheduled "pause check" for 16:18 (`10:48:15Z`); `set_schedules_paused(true)` at 10:46; read at 10:49:05; `set_schedules_paused(false)`; read 20 s later.

Expect: nothing runs while paused; it fires after the pause ends.

Result: pass. At 10:49:05 the follow-up was still `pending` and the dot had only its manual run. 20 s after resuming: a `follow_up` run with "Follow-up you scheduled on Wed 21 Oct 16:11: pause check", and `follow_ups_list` was empty.

**A3d. A follow-up missed while Dots was closed.** `closed` scheduled "closed check" for 16:25 (`10:55:53Z`); the app was stopped at 10:51:05 and started again at 11:16:05 (20 minutes after the follow-up was due).

Expect: it fires once at start, its message ending "(this is 1 hour later than you asked)".

Result: pass. 15 s after the start: one `follow_up` run, parent `01M5F1B7P5J5W955P9MKQE696C`, message "Follow-up you scheduled on Wed 21 Oct 16:20: closed check (this is 1 hour later than you asked)"; no follow-up left pending.

**A3e. Notifications and the "Notify on dot notes" switch.** Not observed: Windows notifications cannot be read from the terminal. The choice (a question always, a note only while the setting is on) is covered by the app's `events` tests and the switch by `SettingsNotes.test.tsx`.

## Part B: person steps

- [ ] **B1.** With a real `claude` CLI, a Claude dot with the instructions "Ask me whether to ship (choices yes, no), then finish." Expect: `--allowedTools` holds `mcp__dots__notify mcp__dots__ask mcp__dots__follow_up`; the transcript shows "Asked: …" without an approval; answering from the chat's bar resumes the session. Then "Schedule a follow-up in 5m to check the build, then finish": it fires 5 minutes later and resumes the same session.

## Findings

1. The local model is slow on this machine (CPU only, about 40 s for a small tool call; a run with five tool calls in one turn timed out after 30 minutes). Model speed, not Dots: every run that reached a tool call got the expected answer from Dots. A GPU-backed server would make local dots usable for real work here.
2. Ollama's default context is 4096 tokens; a dot's first prompt with its tool schemas uses about 1,500 of them (`usage` of the follow-up run: 1509 in). Longer local runs will need a larger context (`OLLAMA_CONTEXT_LENGTH`).
