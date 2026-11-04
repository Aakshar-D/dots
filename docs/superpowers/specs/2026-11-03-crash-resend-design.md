# Resend a person's message after Dots restarts

- Date: 2026-11-03
- Status: approved (owner decided the behaviour, 2026-11-03)
- Part of: Tier 1 of the build list (reliability first).
- Builds on: `2026-11-02-stuck-run-watchdog-design.md`, `2026-10-14-scheduler-v2-design.md` (retries, `retry_of`).

## 1. Goal

A message the person sent is not lost because Dots stopped while the dot was answering it. When Dots starts again, the message is sent again, once more at most three times, and the conversation says so.

What happens today: a chat message, a reply and an answer to a check-in each become a `runs` row (trigger `chat`, the message in `payload`) written before anything is done with it. At start, `Store::recover_interrupted` marks every `running` run `failed` with error `interrupted`, expires its pending approvals and closes half-performed approved calls (`unknown` / `skipped`). Nothing is sent again: the person sees "interrupted" and must type the message again. There is no Retry action for a failed chat run.

Success: after a restart, each interrupted message of the person is queued again, unless doing so could repeat an approved action; the resent run carries the note "Resent after Dots restarted"; a message that keeps getting interrupted stops after 3 resends with a reason; and a failed message can be sent again by hand.

## 2. Words

- **Person's message**: a run with trigger `chat`: a new chat, a reply in a topic, or the run that carries an answer to a check-in (`checkins::continuation`). Run now (`manual`), follow-ups the dot scheduled (`follow_up`), resume runs, and every other trigger are not.
- **Resend**: a new `queued` run that repeats an interrupted run: same trigger, payload, `parent_run_id` and `root_run_id` (so it stays in the same topic), and `retry_of` = the interrupted run.
- **Tries** of a message: the number of `retry_of` links from a run back to the first run of the message.

## 3. Behaviour (decided with the owner)

1. **At start, an interrupted person's message is resent.** In the same transaction that marks it failed / `interrupted`.
2. **Not resent when a call may have run.** If the interrupted run recorded any tool call (a `tool_call` run event), or has an approval that was approved (or whose call is `unknown`), sending the message again could do that action twice. This is the scheduler's rule for a retry (Scheduler v2 §6, `Store::called_a_tool`). No run event says whether a call was read-only, so every call counts, a Read as much as a shell command. It stays failed with error `interrupted`; the person can send it again by hand (§3.6). *Changed after review:* the first version blocked only on approved calls, so a call the policy allowed without asking could run twice.
3. **At most 3 resends per message.** A run whose chain of `retry_of` already has 3 links is not resent; it fails with error `interrupted; stopped resending after 3 tries`.
4. **The note.** The resend's payload gets `"resent": "restart"`. The conversation shows "Resent after Dots restarted" by the resent message. *My call:* a payload key, not a new column: no migration, and the payload already is "what the person sent and how". The prompt ignores the key.
5. **Idempotent.** Marking failed and resending is one transaction: a crash during recovery leaves nothing done, and the next start does it all. A second recovery finds no `running` run and resends nothing.
6. **Send again by hand.** A failed chat run that is the last run of its topic, and works for no workflow step, shows a **Send again** button under its error. It queues the same resend (without the note), `retry_of` = the failed run, under the same checks as a reply: nothing else in the topic is working, and the usage limit is not reached. Sending again by hand counts as a try too.
7. **Other triggers are unchanged.** Scheduled runs keep the scheduler's retries (an interrupted scheduled run that called no tool is retried, Scheduler v2 §6). Watcher, workflow, webhook, API, app, follow-up, manual and resume runs stay failed / `interrupted` as today. A chat run that works for a workflow step (`workflow_step_run_id` set) is left to the workflow too.

## 4. Core

### 4.1 Recovery

`Store::recover_interrupted` keeps its signature (returns the interrupted run ids). After marking them failed, for each one, inside the same transaction:

- skip unless `trigger_kind = 'chat'` and `workflow_step_run_id IS NULL`;
- skip if `EXISTS approvals WHERE run_id = ? AND (status = 'approved' OR apply_state = 'unknown')`, or if the run called a tool (`called_a_tool`, read in the same transaction);
- count tries with a recursive `retry_of` walk; at 3 or more, set `error = 'interrupted; stopped resending after 3 tries'` and skip;
- otherwise insert the resend (`insert_run`). Its session comes from the parent, as `chat` and `continuation` set it (a new chat starts a new session). It works in the interrupted run's folder, branch and base commit when that run had one, else in the parent's. *Why:* the review list shows only the last run of each topic, so a worktree the interrupted run made would otherwise be left where nothing lists it or cleans it up. Since a run is resent only when it called no tool, that worktree is as it was made. `workflow_step_run_id` stays `None`.

The dispatcher's first pass picks the queued resends up.

### 4.2 Send again

`Runtime::resend(run_id) -> Result<Run>`: the run must be a failed `chat` run; permission and limit as `chat`; under `chat_lock`, the topic has no active run and the run is its last; then it enqueues the resend (§4.1 fields, no note). Shared builder: `NewRun::resend_of(run, parent)`. Errors are `Conflict` with text the UI shows as it is.

### 4.3 With delivery tracking and attachments (merged from master)

- Recovery marks an interrupted run that never reached its dot as not delivered (`interrupted`). When Dots resends it, the resend is that message's Retry: the old run is dismissed in the same transaction, so it leaves no Needs you item or notice, and the conversation shows it as a muted "Not delivered" above the resent message. A message that is not resent stays not delivered for the person, with its Retry and Discard. At the limit its reason is `interrupted; stopped resending after 3 tries`.
- The message's files (`chat_attachments`) move to the resend, in the same transaction, as a Retry's do. Send again moves them too (`create_chat_run` with `carry_from`).
- One button per message: a failed message that never got there has the delivery line's **Retry** (and Discard). **Send again** is only for one that got there and failed afterwards; the core refuses Send again for a not-delivered message.



- Command `resend_run(run_id) -> Run`, registered and in the contract.
- `Transcript`: a chat run whose payload has `resent: "restart"` shows a muted line "Resent after Dots restarted" under its bubble.
- `TopicRuns`: under the failure of a failed last run with trigger `chat` and no `workflow_step_run_id`, a **Send again** button calls `commands.resendRun`.

## 6. Not doing

- No migration; no new column.
- No notification for a resend.

## 7. Tests

Store: resent with same payload/parent/root and `retry_of`, in the interrupted run's folder; not resent for other triggers, a step's chat, a run that called a tool, or one with an approved call; limit at 3 with reason; recovery twice resends once. Runtime: `resend` queues a run and refuses a run that is not a failed chat or not the last of its topic. UI: the note, and the button calls the command.
