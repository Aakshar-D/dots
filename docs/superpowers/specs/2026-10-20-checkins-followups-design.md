# Check-ins and self-scheduled follow-ups for dots

Date: 2026-10-20. Status: agreed with the person in a question round, section by section; written for their review.
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md`, the MCP/skills spec `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` and the context library spec `docs/superpowers/specs/2026-10-19-context-library-design.md` (section 12 lists the passages it changes). Source of the idea: a comparison with Grok Bot and OpenAI dots, whose agents report back on their own, ask when they need judgement, and schedule their own follow-ups.

## 1. Why

A dot speaks only when a run ends: its summary lands in the Inbox. It cannot tell the person something on the way, cannot ask a question and get an answer, and cannot say "look again tomorrow at 9" unless the person sets a schedule for it.

This spec adds three tools, for both engines:

- **`notify`:** the dot posts a note to the person and keeps working.
- **`ask`:** the dot asks the person a question and ends its turn; the person's answer continues the thread.
- **`follow_up`:** the dot schedules a run of its own that continues the thread at a later time, with a note to itself.

Decisions (agreed with the person):

| Question | Decision |
|---|---|
| What a check-in is | Both: a note that does not wait, and a question that does |
| How a question waits | Park and resume: the run ends; the answer is a new run that resumes the thread's session. Nothing holds a process or a concurrency slot while the person is away, and no CLI timeout applies |
| What a follow-up run is | A continuation of the same thread (session resumed), with the dot's note; a fresh run only when the thread cannot be resumed |
| Whether follow-ups need approval | No: every preset allows them, within hard limits (section 3). A dot's own `FollowUp` rule (ask, deny) still wins |
| Where code goes | New modules; shared files get list entries and match arms only, to stay clear of parallel work on approvals and the scheduler |

Out of scope: per-dot memory; dots messaging each other; a dot cancelling or editing its follow-ups; recurring follow-ups (a schedule does that); a question that blocks a live run; reply by email or phone; changes to approvals or to the scheduler's own logic.

Anti-goals: two runs continuing one thread at once; a follow-up firing while schedules are paused; a dot filling the person's Inbox or calendar without limit.

## 2. The tools

Each tool is served by Dots on both engines, as `context` is: built in on the local engine, a tool of Dots' own `dots` MCP server on the Claude engine (next to `approve` and `context`; `args.rs` adds `mcp__dots__notify`, `mcp__dots__ask` and `mcp__dots__follow_up` to `--allowedTools`). The `dots` server checks each call against the dot's policy under the tool's policy name before it acts, waiting for an approval when a rule asks, as `call_context` does.

| Tool | Policy name | Input | Result |
|---|---|---|---|
| `notify` | `Notify` | `text`: string, 1 to 2,000 characters | `"Sent."` The run continues |
| `ask` | `Ask` | `question`: string, 1 to 2,000 characters; `choices`: optional array of 1 to 5 strings, each 1 to 80 characters | `"Your question was sent. Finish your turn now; the person's answer will arrive as your next message."` |
| `follow_up` | `FollowUp` | `note`: string, 1 to 2,000 characters; exactly one of `at` (RFC 3339 with offset) or `in` (a whole number and a unit `m`, `h` or `d`, e.g. `"90m"`, `"2h"`, `"1d"`) | `"Follow-up <id> scheduled for <local time, e.g. Tue 21 Oct 09:00>."` |

A call that breaks a rule is answered with a tool error (`isError`) saying which rule, in one sentence, so the dot can correct it; the run does not fail. Rules:

- Text lengths as above, counted in characters after trimming; blank text is refused.
- `ask`: one question per run. A second call answers "You already asked a question in this run; finish your turn now."
- `follow_up` (section 3).

The tool descriptions (shared by both engines in `checkins.rs`, as `context::DESCRIPTION` is) say when to use each: `notify` for progress the person would want to know now, sparingly; `ask` only when the dot cannot go on well without the person's judgement; `follow_up` for work that has to wait (a build, a reply, a date).

From the browser spec (`2026-10-21-browser-design.md` section 4), a dot that browses hands over to the person with `ask` when a page needs a login, a code or a CAPTCHA ("Please <do it> in the browser window, then answer done."): the person does it in the dot's visible Edge window and answers, the answer resumes the run, and the login stays in the dot's profile.

## 3. Follow-up limits

Checked in core when the tool is called, in this order, against the clock of the follow-up loop:

1. `due_at` is at least 5 minutes after now.
2. `due_at` is at most 30 days after now.
3. The dot has fewer than 3 follow-ups with status `pending`.

The error names the limit: "A follow-up must be at least 5 minutes away.", "A follow-up must be at most 30 days away.", "This dot already has 3 pending follow-ups; that is the most it may have."

The limits are constants in `followups.rs`, not settings.

## 4. Policy

`Policy::preset` adds `Notify`, `Ask` and `FollowUp` allow rules to every preset (Sandboxed, Trusted, and Custom, which starts from Sandboxed), after `Skill` and `Context`. Stored dots keep their stored policy; the tools are judged by the dot's rules and, with no matching rule, by the default action of the policy. Unlike other tools, whose order is deny > allow > ask, for these three an ask rule wins over an allow rule (deny > ask > allow), so a person who sets one to ask is asked even though the preset allows it. Migration 0008 (section 8) adds the three allow rules to the stored policy of every dot whose preset is not Custom, as the person would get by choosing the preset again.

The three names, like `Context`, are left out of what the Claude CLI is told (`claude_rules`): the CLI has no such tools, and Dots' `dots` server checks each call.

## 5. Asking and replying

1. The dot calls `ask`. Core stores a `question` message (section 7) and publishes `MessagePosted`; the app notifies.
2. The dot ends its turn; the run ends as it would have (usually `succeeded`). The question is open.
3. The person replies: `Runtime::reply(message_id, text)`.
   - The message must be an open question (not answered, not dismissed); otherwise `Conflict` "this question has already been answered" or "this question was dismissed".
   - The thread is the run's `root_run_id` line. Under the thread lock (section 6), its end is read as `chat` reads it.
   - **The thread is quiet** (every run ended) and its end has a session: a new run, trigger `chat`, parent the thread's end, the session, workspace, branch and base commit carried over as `chat` does, payload message:

     ```
     Answer to your question "<question>": <text>
     ```

   - **The thread is busy:** the same `Conflict` sentences `chat` gives ("the dot is still working on this chat; …"); the question stays open.
   - **The end has no session** (it failed before the dot started): a new run with no parent, trigger `chat`, payload message:

     ```
     Earlier you asked the person: "<question>"
     Their answer: <text>
     ```

4. The message gets `answered_at` and `answer_run_id`; `MessageUpdated` is published.

`choices` are offered as buttons; choosing one sends its text as the answer. The person may always type instead.

The person may dismiss a question (`dismissed_at`): it leaves the Inbox unanswered. A dismissed question cannot be answered.

A note needs no answer. It shows until the person marks it read (`read_at`). A question opened in the Inbox or the chat is also marked read, which does not close it.

`Runtime::chat` is not changed except for taking the shared thread lock: a person who types in the chat of a thread with an open question sends an ordinary message; the question stays open in the Inbox until answered or dismissed. The chat screen steers to `reply` (section 9).

## 6. Follow-ups

A loop in `followups.rs`, started by `Runtime::start` next to the scheduler, owns the `follow_ups` table. It uses the scheduler's `Clock` trait (imported; `scheduler.rs` is not changed) so tests move time by hand.

- It sleeps until the earliest `pending` `due_at`, or until woken: a follow-up was added or cancelled, a run ended, a dot was enabled, disabled or deleted, schedules were paused or resumed.
- At each pass, every `pending` row with `due_at <= now` is considered, oldest first:
  - **Schedules paused globally:** left `pending`; it fires after the pause ends.
  - **Dot disabled:** `skipped`; a note is posted (section 7) saying the follow-up was skipped because the dot is off.
  - **The thread is busy** (a run of the `from_run_id`'s thread has not ended): left `pending`; it is considered again when that run ends. No second run on a thread.
  - **The thread is quiet and its end has a session:** under the thread lock, a run is enqueued: trigger `follow_up`, parent the thread's end, the session, workspace, branch and base commit carried over as `chat` does, payload message:

    ```
    Follow-up you scheduled on <created_at, local>: <note>
    ```

    with ` (this is <n> hours late; Dots was not running at the time)` appended when it fires more than 15 minutes after `due_at`.
  - **The end has no session:** a run with no parent, trigger `follow_up`, payload message:

    ```
    Follow-up you scheduled on <created_at, local>: <note>
    What the earlier run reported: <summary of from_run_id, or "nothing">
    ```

  The row becomes `fired` with `fired_run_id`; `FollowUpsUpdated` is published.
- **Missed while Dots was closed:** at start, overdue rows are due and fire once each, by the rules above.
- **A run that cannot be enqueued** (a store error): the row stays `pending`, `attempts` goes up, and the next try is after 1, 5, then 30 minutes (`retry_at`). After the third failure the row is `skipped` and a note tells the person why.
- **Cancel:** the person cancels a `pending` row (`cancelled`). Dots have no cancel tool.
- **Dot deleted:** its rows are deleted with it (foreign key `ON DELETE CASCADE`).

**Thread lock.** Runtime's `chat_lock` moves into an `Arc` shared with the follow-up loop. `chat`, `reply` and the loop each hold it from reading a thread's end until the new run is stored, so two writers never give one run two children.

**Trigger.** `TriggerKind` gains `FollowUp` (`"follow_up"`). A follow-up run is reviewed in the Inbox like a scheduled run.

## 7. Data

`messages`:

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | ULID |
| `dot_id` | TEXT NOT NULL | → `dots(id)` ON DELETE CASCADE |
| `run_id` | TEXT NULL | the run that posted it; NULL for Dots' own notes (a skipped follow-up) |
| `kind` | TEXT NOT NULL | `note` \| `question` |
| `text` | TEXT NOT NULL | |
| `choices` | TEXT NULL | JSON array of strings, questions only |
| `created_at` | TEXT NOT NULL | |
| `read_at` | TEXT NULL | |
| `answered_at` | TEXT NULL | questions only |
| `answer_run_id` | TEXT NULL | questions only |
| `dismissed_at` | TEXT NULL | questions only |

Index on `(dot_id, created_at)`.

`follow_ups`:

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | ULID |
| `dot_id` | TEXT NOT NULL | → `dots(id)` ON DELETE CASCADE |
| `from_run_id` | TEXT NOT NULL | the run that called `follow_up` |
| `due_at` | TEXT NOT NULL | UTC, RFC 3339 |
| `note` | TEXT NOT NULL | |
| `status` | TEXT NOT NULL | `pending` \| `fired` \| `cancelled` \| `skipped` |
| `fired_run_id` | TEXT NULL | |
| `attempts` | INTEGER NOT NULL DEFAULT 0 | |
| `retry_at` | TEXT NULL | |
| `created_at` | TEXT NOT NULL | |

Index on `(status, due_at)`.

Store files: `store/messages.rs`, `store/follow_ups.rs`. Model types `Message`, `MessageKind`, `FollowUp`, `FollowUpStatus` in `model.rs`.

## 8. Upgrade (migration 0008)

`0008_checkins.sql` creates the two tables and indexes. The policy part (section 4) is done in code in `store/upgrade.rs`, as 0006 and 0007 are: for each dot whose preset is not Custom, add the missing `Notify`, `Ask` and `FollowUp` allow rules. The migration number is checked against `master` before the branch is merged and renumbered if taken.

## 9. Screens

- **Inbox:** a "Messages" group first. A question shows the dot, the text, its choice buttons, a reply box and "Dismiss"; a note shows the text and "Mark read". The Inbox and tray counts add open questions and unread notes.
- **Chat:** a thread's check-ins show in it where they were posted, as messages from the dot, between the runs. While the thread has an open question, a bar above the composer reads "<dot> asked: <question>" with "Reply to question"; sending from it calls `reply`.
- **Run:** `notify`, `ask` and `follow_up` calls show as one readable line ("Sent a note: …", "Asked: …", "Scheduled a follow-up for Tue 21 Oct 09:00: …"), not as raw tool input.
- **Dots and the dot editor:** a "Follow-ups" section lists pending follow-ups (local time, note, Cancel). A dot's card shows "next follow-up in 3 h" when it has one.
- **Settings:** "Notify on dot notes", on by default. Questions always notify.

## 10. Commands, events and contract

Commands: `messages_list(dot_id?)`, `message_reply(id, text)`, `message_dismiss(id)`, `message_mark_read(id)`, `follow_ups_list(dot_id?)`, `follow_up_cancel(id)`. Events: `MessagePosted { message }`, `MessageUpdated { message }`, `FollowUpsUpdated { dot_id }`. All in `apps/dots-app/src/contract.rs` and `ui/src/api/__fixtures__/contract.json`. Settings gain `notify_on_notes: bool` (default `true`). The app notifies on `MessagePosted`: always for a question ("<dot> asks: <question>"), for a note when the setting is on.

## 11. Errors and testing

Errors:

- A tool call that cannot be stored answers the dot "Not delivered: <reason>." and is logged; the run goes on.
- `reply` refusals are the `Conflict` sentences of section 5, shown as they are.
- A follow-up that cannot start retries and then becomes a note (section 6).

Tests, written first, in the repo's patterns:

- Store: round trips of both tables; cascade on dot delete.
- Tools: every input rule and limit, on both engines' entry points; one `ask` per run.
- Policy: presets carry the three rules; a `deny` or `ask` rule wins; the upgrade adds rules to non-Custom dots only.
- `reply`: quiet thread, busy thread, no session, answered, dismissed.
- Follow-up loop on a manual clock: fires when due; waits on a busy thread and fires when it ends; disabled dot; paused then resumed; missed while closed, with the late wording; cancelled; three failed starts then a note; deleted dot.
- Race: `chat` and a due follow-up on one thread give one run.
- Engines: the Claude path through the `dots` MCP server with `fake-claude`; the local path with the scripted engine.
- UI (Vitest): the Messages group, choice buttons, the reply bar, the follow-up list and Cancel.
- Smoke: `docs/verification/<date>-checkins-followups-smoke.md`, with a real `claude` dot that asks, is answered, and schedules a follow-up 5 minutes ahead.

## 12. Changes to the other specs

- Main spec §1, triggers: add `follow_up` (a run a dot scheduled for itself).
- Main spec §5, policy: Dots' own tools are `Context`, `Notify`, `Ask`, `FollowUp`, checked by the `dots` server on the Claude engine.
- Main spec §12 notifications: add dot questions and notes.
- MCP/skills spec §4 and context library spec §4: the `dots` MCP server serves `approve`, `context`, `notify`, `ask` and `follow_up`.

## 13. Changes made while building

The plan (`docs/superpowers/plans/2026-10-20-checkins-followups.md`) changed these from the first version of this spec:

- Every preset allows `Notify`, `Ask` and `FollowUp`, Read-only included, as every preset allows `Skill` and `Context`; migration 0008 adds them to every stored policy whose preset is not Custom (section 4).
- A follow-up whose run cannot be stored is tried again after 1, 5 and 30 minutes; the fourth failed start skips it with the note "A follow-up was skipped because its run could not be started (<error>). It was due <time>: <note>". Any error while firing one follow-up (reading its thread, storing its run) counts as a failed start (section 6).
- The late words are ` (this is <n> hours later than you asked)`, `hour` for one hour; the hours are the lateness rounded, at least 1, and they end the message's first line. They name no cause: a pause, a busy thread or a retry also make a follow-up late while Dots runs, so "Dots was not running at the time" would be false (section 6; from the final review).
- A disabled dot's follow-up posts "A follow-up was skipped because the dot is off. It was due <time>: <note>" (section 6).
- The first prompt of a follow-up run that cannot resume its thread has a `## Follow-up` section with the message; a follow-up run's payload also holds `follow_up_id` (section 6).
- `Message` carries `dot_name`, read from the dot (section 7).
- `messages_list` answers every message that waits (open questions, unread notes) and the 200 newest others, newest first; `follow_ups_list` answers the pending follow-ups, soonest first (section 10).
- Dismissing a dismissed question changes nothing; an answered one is refused with "this question has already been answered"; a note cannot be dismissed (section 5).
- A blank `at` or `in` counts as absent (section 2).
- The chat's bar also offers the question's choices; an answer in a thread whose end has no session opens the new chat (section 9).
- Migration 0007's test (`store_dots_test`) builds its pre-0007 policy without the check-in rules too, since 0008 now appends them after `Context`; behaviour is unchanged.
- The `Runner`'s `checkins` field sits after the `held` and `pass` fields that the auto-update work added; behaviour is unchanged.
- `local_tools_test` counts the three check-in tools among those a run gets only on request (`checkins::schemas`), and round-trips their input through `prepare`.
- A run sends at most 10 notes (`checkins::MAX_NOTES`); the eleventh `notify` is answered "This run has sent 10 notes; that is the most it may send." The count and the insert are one statement, so notes sent at once cannot pass it. Dots' own notes do not count (section 2; from the final review, for the anti-goal of section 1).
- For `Notify`, `Ask` and `FollowUp` an ask rule wins over the preset's allow (deny > ask > allow); every other tool keeps deny > allow > ask (section 4; asked for by the person after the smoke).
- An answer's run payload also holds `answer`, the person's trimmed text; the chat shows it in place of the wrapped message (section 5; from the self-QA).
- Each Inbox reply box is labelled "Answer to <dot>: <question>", so two open questions of one dot have distinct boxes (section 9; from the self-QA).
