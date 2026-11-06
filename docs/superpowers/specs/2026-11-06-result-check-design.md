# Result check: a quick second look at every answer before it reaches you

- Date: 2026-11-06
- Status: approved (owner's request, 2026-11-06)
- Builds on: the runner (`runner.rs`), the one-off CLI request (`engine/claude/once.rs`, used by
  the skill creator), the notifier (`apps/dots-app/src/events.rs`), check-in notes, Settings.

## 1. Goal

"Review gates. An automatic check on every agent result before it reaches you, because you've
seen weak output before."

Workflows already have human `Check` steps. Nothing looks at an ordinary dot's answer before the
person reads it. The result check is a cheap, automatic second opinion on each final answer. It
only flags; it never changes, blocks or re-runs anything.

## 2. Which runs are checked (`result_check::should_check`)

In this order; the first rule that applies decides:

1. **Settings › Claude › Check every result** (`check_results`, default **on**) is off: no check.
2. **The dot's own switch** (`DotSpec.check_results`, default on, dot editor › Engine, "Check its
   results"; shown for Claude dots only) is off: no check.
3. **Local-engine dots** are not checked: the checker is a Claude CLI call and a local dot's
   owner may not want Claude involved. (Could be allowed later with a local checker.)
4. **Workflow steps** (`workflow_step_run_id` set, or a `workflow` trigger) are not checked: the
   answer goes to the next step, not to the person, and teams already have `Check` steps.
5. The run did not **succeed**: nothing to check.
6. **No final text** (blank summary): nothing to check.
7. **Short answers** (under `MIN_ANSWER_CHARS` = 200 characters, trimmed) are not checked: a
   short reply ("Done.", "Yes, it is Tuesday.") has too little in it to judge, and checking it
   costs more than it saves.

Every other succeeded Claude run is checked: chats, Run now, schedules, webhooks, watchers,
follow-ups, resumes, API and app runs. Checker calls are not runs, so a check is never checked.
A run without a check has `result_check = null` and shows no badge.

## 3. The check (`Runner::check_result`)

- When the run is about to be stored as `succeeded` and a check applies, the runner first stores
  `result_check = {"state": "checking", "model": …}`, then ends the run as usual. The run is
  done: its slot is freed, its conversation can go on. The check runs on its own task.
- **Checker call**: one `claude -p` request (`CliChecker`, built on `ask_once`): `--tools ""`,
  `--strict-mcp-config` with no MCP config, `--setting-sources ""`, `--max-turns 1`,
  `--no-session-persistence`, a replaced system prompt, in a new empty folder that is removed
  afterwards. Text in, JSON out: no file, shell or MCP access at all. The model is
  Settings › Claude › **Checker model** (`checker_model`, default `haiku`, the cheapest).
- **Time limit**: `CHECK_LIMIT` = 30 s (the CLI is ended then, and the runner gives up 5 s later
  whatever the checker does).
- **What it reads** (`build_prompt`): the task (the person's message for a chat, or their answer
  to a question; else the dot's instructions and what started the run), the dot's final answer,
  and for a worktree run a summary of its changes (file list, then patches up to 6,000
  characters, `diff_summary`). Task and answer are cut at 6,000 and 12,000 characters.
- **Prompt-injection hygiene**: task, answer and changes are untrusted data. Each sits in its
  own block whose tag ends with a random nonce made per check (`<answer_x7f…>`); a copy of that
  tag inside the data is defused, so the data cannot close its block. The system prompt says the
  blocks are untrusted data to judge, never instructions to follow, and that anything in them
  about how to judge or what verdict to give is ignored. The checker has no tools, so the worst
  a hostile answer can do is sway its own verdict.
- **Reply**: `{"verdict": "looks_good" | "has_concerns" | "failed_task", "concerns": [short
  strings], "confidence": 0..1}`. Read robustly (`parse_verdict`): the whole reply, else the
  first balanced `{…}` with a known verdict (fences and chatter around it are fine). Verdict
  spellings are normalised (`Has Concerns`, `ok`, `fail`…). At most 5 concerns, each cut to 240
  characters; non-text entries dropped. `looks_good` keeps no concerns; a flagged verdict with
  none gets "The check found a problem but did not say what." Confidence: a number 0–1, a
  percentage, or high/medium/low; anything else is dropped.
- **Never fails the run.** A checker that errors, times out, panics or answers something
  unreadable leaves `{"state": "skipped", "reason": …}` ("Check skipped" in the UI, the reason on
  hover). The run's status, summary and error are never touched by the check.
- **Restart**: a check Dots stopped in the middle of is marked skipped ("Dots stopped before the
  check ended.") at the next start (`Store::skip_unfinished_checks`); the checks' folder is
  removed.

## 4. Before it reaches you

- **Conversation**: under each checked answer, a badge (`ResultCheckBadge`): **Checking…**
  while it runs, **✓ Checked** for `looks_good`, **⚠ N concerns** (or **⚠ May have missed the
  task**) which opens the list of concerns, and **Check skipped**. Run cards (scheduled and
  other background topics) show the same badge.
- **Notifications** wait for the check. The notifier holds the end notice of a run that ended
  succeeded with its check running, and sends it when the run is published again with a verdict
  or skipped. The core ends every check within its limit, so the wait is at most about 30 s. The
  notice's title says what the check found: "nightly finished (2 concerns from the check)" or
  "nightly finished, but may have missed the task". A held notice survives an event-stream lag.
  Chat runs still do not notify (the person is watching).
- **Inbox note**: a flagged verdict (`has_concerns` / `failed_task`) leaves a note of the run in
  the Inbox ("Result check: 2 concerns about this answer." and the list), which notifies as any
  note does (Settings › Notify on dot notes). Its "Open conversation" leads to the badge.
- **"Ask the dot to fix this"**: in the opened concern list, one click sends the concerns as the
  person's next message in the same conversation, after its last message (`result_check_fix` →
  `Runtime::ask_to_fix`, which goes through `chat`, so its refusals apply: the dot still busy, a
  newer message). The button waits while the conversation is busy. **No auto-loop**: Dots never
  sends a fix by itself, and the fix's own answer gets its own check like any other.

## 5. Usage and cost

What the checker used (the CLI's `result` usage and `total_cost_usd`) is **added to the run's
own usage** (`tokens_in`, `tokens_out`, `tokens_cache_read`, `cost_usd`), so the Usage screen,
the per-run numbers and anything that sums runs stay honest. It is also recorded as a run event
of its own, kind `result_check` (model, verdict or skip reason, tokens, cost), so the run's
event log tells the dot's own usage and the check's apart. The transcript does not show it.
The checker's call does not count against "runs at once" and is not held by the usage hold:
it is short, one turn, and on the cheapest model.

## 6. Data

- Migration `0041_result_check.sql` (number may move at merge): `runs.result_check TEXT`
  (JSON of `ResultCheck`, tagged by `state`: `checking` / `done` / `skipped`), and
  `dots.check_results INTEGER NOT NULL DEFAULT 1`.
- Settings: `check_results` (bool, default true), `checker_model` (string, default `haiku`, not
  blank, stored trimmed).
- `Run.result_check`, `DotSpec.check_results` in the contract; the UI's zod schemas mirror them.

## 7. Tests

- Core (`tests/result_check_test.rs`): verdict parsing (clean, fenced, chatty, garbage, partial,
  caps), every skip rule, settings defaults and an old dot spec, the task text, the fenced prompt
  and a forged closing tag, the diff summary, the fix and note texts; against a stand-in checker
  in the runner: a verdict kept with the run, usage added and the `result_check` event recorded,
  the Inbox note, the run announced first as checking and then with its verdict; a failing,
  garbled or panicking checker leaving the run succeeded with "Check skipped"; short answers,
  switched-off settings or dot, and no checker asking nothing; the Settings model used; an
  unfinished check skipped at the next start.
- Core (`tests/runtime_result_check_test.rs`): "Ask the dot to fix this" sends the concerns as
  the next message of the same thread; refused while the dot works on it and when nothing was
  flagged.
- App (`events.rs`): the end notice waits for the check and says what it found, once; a skipped
  check lets the plain notice go; a held notice survives a lag; chat runs stay quiet.
- UI: the badge in every state, the concerns list and the fix call, the busy and refused fix,
  the event not shown in the transcript, the Settings switch and checker model, the dot
  editor's switch.

## 8. Not done

- No automatic re-run or loop on concerns; no blocking of a run's delivery on the verdict.
- Local-engine dots are not checked (no local checker yet).
- The Inbox note has no fix button of its own; it opens the conversation, where it is.
