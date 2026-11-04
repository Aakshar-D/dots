# Send later

- Date: 2026-11-04
- Status: draft for the owner's review (the owner's decisions of 2026-11-04 are in §3; the rest are marked *builder's call*)
- Part of: Tier 1 of the build list, item 2. Second of three stacked branches: Snooze/Done (`feat/snooze-done`, migration 0031) → **Send later** (`feat/send-later`, migration **0032**) → Attachments (`feat/attachments`, migration 0033).
- Builds on: `2026-10-27-conversational-chat-design.md` (the chat path and its parent rule), `2026-11-03-delivery-tracking-design.md` (what happens to a run once sent), `2026-10-26-usage-limits-design.md` (the Claude limit), and the presets helper from snooze/done (`ui/src/lib/later.ts`).

## 1. Goal

The person can write a message to a dot now and have Dots send it later: "tomorrow morning", "Monday 9 AM", or a time they pick. Until it goes, the message waits as a card in the conversation with a live countdown, and can be sent now, edited or cancelled. It is sent even if Dots was closed at that time: as soon as Dots runs again, marked as sent late.

Success: a message scheduled for "In 1 hour" shows "Sends in 1h" under it, goes to the topic it was written in an hour later as an ordinary chat message (with delivery tracking), and the dot's header menu said "1 scheduled" until then. With Dots closed over the due time, it goes when Dots starts and shows "Sent late (was due 9:00 AM)".

## 2. Words

- **Scheduled message**: a row in `scheduled_messages`: the text, the dot, the topic it was written in, and the time it is due.
- **Due**: its `due_at` has passed.
- **Sent**: Dots sent it through the chat path and a chat run exists. From then on it is that run; this feature is done with it.
- **Waiting** (a due message that has not gone yet): its topic is busy, waits for an approval, or Claude's usage limit is reached. Dots tries again on its next pass.
- **Late**: sent more than **1 minute** after `due_at` *(builder's call)*.
- **Pending**: scheduled, sending or failed; what the cards and the "N scheduled" count show.

## 3. Behaviour (decided with the owner)

1. **Composer**: a small arrow menu beside Send with the presets of the presets helper from snooze/done (`ui/src/lib/later.ts`): In 1 hour · This evening (6 PM) · Tomorrow morning (9 AM) · Monday 9 AM · Pick a date and time. The helper decides each preset's time and what happens to a preset already past; this feature does not redefine them.
2. **Before it goes**: a card in the conversation with the text, a live countdown "Sends in 2h 14m", and **Send now · Edit · Cancel**. The dot's header menu shows **"N scheduled"**.
3. **Dots closed or asleep at the due time**: it sends as soon as Dots runs again, and the message shows **"Sent late (was due 9:00 AM)"**.
4. **Where it goes**: to the topic that was the reply target when it was scheduled, as a reply after whatever is latest there at send time (the chat path's parent rule). A message scheduled with "New topic" starts a new topic.
5. **Once sent** it is a normal chat run: delivery tracking applies (Waiting / Not delivered / Retry / Discard, Needs you, today's counts).
6. **Survives restarts**: stored in the database, sent by a sweep on the `Clock`.

### 3.1 Builder's calls

1. **The topic is gone or cannot be continued** at send time (its root run no longer exists, its last run ended before the dot had a session, or it is a workflow step's thread that ended): the message is sent as a **new topic**, and says so under the bubble (§6). It is never dropped. *(builder's call)*
2. **The topic is busy** at the due time (a run of it is queued, running or awaiting approval, or a follow-up is deciding): the message **waits** and goes as soon as the topic is free, on the next pass. Two messages due for the same topic go in due order, one per free turn, so each continues from the dot's answer to the one before. *(builder's call)*
3. **Claude's usage limit reached** at the due time (`refuse_at_the_limit` would refuse it): the message waits until the limit resets, then goes (late). The usage **hold** for background runs does not apply: a chat run is the person's own. *(builder's call)*
4. **Refused for good** at send time (the dot's org is no longer joined, the person's role no longer lets them run it): the message becomes **Not sent** with the reason, keeps its card (Send now · Edit · Cancel), and Dots notifies once. It is not a Needs you item. *(builder's call)*
5. **Skip when busy** does not apply: it is for a schedule's repeating slots. A scheduled message is never skipped. If the dot is busy with *another* topic, the new run queues as any chat run does (delivery tracking's `dot_busy` wait reason). *(builder's call)*
6. **Pausing schedules** (tray / Settings) does not pause scheduled messages: they are the person's messages, not the dot's schedule. *(builder's call)*
7. **The composer's locks still apply**: the arrow menu is usable exactly when Send is (not while the target topic is busy, waiting for approval, closed, or while answering an open question; the menu is hidden while answering). *(builder's call: keeps one rule for the composer; scheduling while the dot works is §10.)*
8. **Times**: a scheduled time must be at least 1 minute ahead and at most 365 days ahead. *(builder's call)*
9. **Cancel** is immediate, with no confirm and no undo; the row is kept as `cancelled`. **Edit** changes the text and the time in place. *(builder's call)*
10. **No notice when it is sent**; the dot's answer notifies as any chat run's does. *(builder's call)*
11. **Where the cards stand**: at the end of the conversation, after every topic, in due order; each names the topic it goes to. Viewing an older topic on its own shows only the cards for that topic. *(builder's call)*

## 4. Core

### 4.1 Storage

**Migration 0032** (`0032_send_later.sql`):

```sql
CREATE TABLE scheduled_messages (
    id          TEXT PRIMARY KEY,
    dot_id      TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    root_run_id TEXT,              -- the topic it was written in; NULL = a new topic
    text        TEXT NOT NULL,
    due_at      TEXT NOT NULL,     -- RFC 3339 UTC
    status      TEXT NOT NULL,     -- scheduled | sending | sent | failed | cancelled
    waiting     TEXT,              -- a due message not sent yet: busy | approval | usage_limit
    error       TEXT,              -- failed: why
    run_id      TEXT REFERENCES runs(id) ON DELETE SET NULL,  -- sent: the chat run
    sent_at     TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX scheduled_messages_due ON scheduled_messages(status, due_at);
CREATE INDEX scheduled_messages_dot ON scheduled_messages(dot_id, status);
```

`root_run_id` has no foreign key: a missing root is a case the sender handles (§3.1.1). Deleting a dot deletes its rows.

**Lifecycle**:

```
scheduled ──(sender or Send now claims it)──▶ sending ──▶ sent
    ▲  │                                         │
    │  └──(Cancel)──▶ cancelled                  ├──▶ scheduled + waiting   (busy / approval / usage_limit, or a lost race)
    │                                            └──▶ failed (error)
    └──(Edit)── failed ──(Send now)──▶ sending;  failed ──(Cancel)──▶ cancelled
```

Every change is one conditional `UPDATE … WHERE status IN (…)`, so the sender, Send now, Edit and Cancel never both win (§4.4). The sender's writes to a `scheduled` row (the claim, a waiting reason, `failed`) also need it to be due at that moment, so a pass that read a row before an Edit moved it later changes nothing.

### 4.2 Sending one message

`Runtime::send_scheduled(msg)`, by the sender for a due message or by Send now:

1. **The dot may still run it**: `require_run` on the dot. Refused → `failed` with the refusal's text (§3.1.4).
2. **Its target**, read from the thread of `root_run_id` (no lock):
   - no root, or the root row is gone → **new topic** (with the "gone" note when there was a root);
   - a run of the thread is not ended, or `follow_ups.deciding(root)` → **wait** (`approval` when one awaits approval, else `busy`);
   - the thread's end (the chat path's rule: the last run that is not dismissed-undelivered) has no `session_id`, or `in_ended_step_thread` → **new topic** with the note;
   - else **reply** to that end.
3. **The Claude limit**: `limits.rejected_until()` set for a Claude dot → **wait** (`usage_limit`).
4. **Claim**: `scheduled → sending`. Lost (edited, cancelled or claimed meanwhile) → nothing more.
5. **Send** through the chat path with the payload note (§4.3). The chat path takes its lock and checks again; a `Conflict` there means the thread moved since step 2 → back to `scheduled` with `waiting = busy`, tried on the next pass. Its refusal at Claude's limit (`refuse_at_the_limit`: the limit was reached since step 3) → back to `scheduled` with `waiting = usage_limit`. Any other error → `failed`, with its own reason, even while the limit is reached.
6. **Done**: `sent`, `run_id`, `sent_at`, `waiting`/`error` cleared.

### 4.3 The chat path and the payload note

- `Runtime::chat` becomes a wrapper over **`chat_with(dot_id, message, parent_run_id, extra: ChatExtra)`**. `ChatExtra` holds `send_later: Option<SentLater>` and is merged into the run's payload: `{ "message", "typed": true, "send_later": { "id", "due_at", "late", "new_topic" } }`. The prompt reads only `message`, so the dot sees exactly what the person wrote.
- `late = sent_at − due_at > 1 minute`. `new_topic` is true when step 2 fell back to a new topic for a message that had a topic.
- **The extension point for attachments** (`feat/attachments`, migration 0033): `ChatExtra` is where the files of a message travel to the chat path, and `Runtime::chat_extra_for(&ScheduledMessage) -> ChatExtra` is the one place a scheduled message's extras are built. Attachments adds its staged files there (keyed by the scheduled message's `id`), and frees them in `Runtime::release_staged(id)`, which this part calls (as a no-op) on Cancel. Nothing about attachments is designed here.

### 4.4 Send now, Edit, Cancel and their races

- **`schedule_message(dot_id, text, root_run_id, due_at)`**: text not empty; `due_at` RFC 3339, 1 minute to 365 days after the clock's now (`Invalid` with the §6 texts); `require_run`; a `root_run_id` must be a run of this dot. Wakes the sender.
- **`send_scheduled_now(id)`**: the row must be `scheduled` or `failed` (`Conflict` otherwise, §6 texts). Sets `due_at = now`, clears `error`, then §4.2 at once. Returns the row: `sent`, or still `scheduled` with `waiting` (it goes when the topic is free), or `failed`. From then on the press is the message's due time: sent at once it is never late; if it has to wait more than a minute for its topic, it is "Sent late (was due `<the time Send now was pressed>`)", which is when the person asked for it. The time it was first scheduled for is not kept.
- **`edit_scheduled_message(id, text, due_at)`**: same checks as scheduling; only from `scheduled` or `failed`; sets `status = scheduled`, clears `waiting` and `error`. Wakes the sender.
- **`cancel_scheduled_message(id)`**: only from `scheduled` or `failed` → `cancelled`; calls `release_staged(id)`.
- **Races**: the conditional updates decide. The sender claimed it first → Edit and Cancel answer `Conflict("This message is being sent now.")` or, once sent, `Conflict("This message has already been sent.")`; the UI shows that and the card turns into the sent message. Cancel or Edit first → the sender's claim finds nothing and moves on. Two Send nows → one sends, the other gets `Conflict`, whether it lost at the reset or at the claim.
- **`scheduled_messages(dot_id)`**: the pending rows (`scheduled`, `sending`, `failed`), ordered `due_at, id`.
- The changes need the `RunDots` permission on the dot (`require_run` for schedule and Send now; `require_for_dot(RunDots)` for Edit and Cancel). The list is a read and needs only `View`, as the conversation does: a Viewer sees the cards (and cannot use their buttons).

### 4.5 The sender, restarts, and sleep

- **The sender** is a loop on the runtime's `Clock`: one pass sends every row with `status = scheduled` and `due_at <= now` (in `due_at, id` order); then it sleeps until the earliest future `due_at` or **30 seconds**, whichever is sooner, or until woken by a schedule or an edit. The 30-second cap retries waiting messages and catches up after the PC sleeps. *(builder's call)*
- It needs the whole runtime (the chat path's checks and lock), so it is started with **`Runtime::spawn_send_later(self: &Arc<Self>) -> JoinHandle<()>`**, which the app calls right after it puts the runtime in an `Arc` (`startup.rs`), and stops on the runtime's shutdown token. *(builder's call: `Runtime::start` returns a plain `Runtime`)*
- **At start** (`Runtime::start`, after `recover_interrupted`): a row left in `sending` by a crash is `sent` if a run's payload has `send_later.id` equal to its id (that run becomes its `run_id`), else back to `scheduled`. So a crash never sends twice and never loses a message.
- **Each pass** first does the first half of that: a row `sending` whose run is stored is `sent` (the write after the chat path failed, e.g. the database was busy), so its card does not stay "Sending…" until the next start. A row with no run yet is left alone: it may be on its way through the chat path.
- The first pass runs at once: everything that came due while Dots was closed goes then, marked late.
- **Events**: `RuntimeEvent::ScheduledMessageUpdated { message: ScheduledMessage }` on every change of a row (scheduled, edited, waiting changed, sent, failed, cancelled). A pass that finds the same waiting reason again sends nothing. The new run's own `RunUpdated` goes as usual.

## 5. App

- Commands `schedule_message(dotId, text, rootRunId, dueAt)`, `scheduled_messages(dotId)`, `send_scheduled_now(id)`, `edit_scheduled_message(id, text, dueAt)`, `cancel_scheduled_message(id)`.
- Startup: `runtime.spawn_send_later()` after `Arc::new(runtime)`.
- Notifier: on a `ScheduledMessageUpdated` whose row newly has `status = failed`, once per row id and failure: title **`<dot> didn't get your scheduled message`**, body the error's first line, route `/home/<dot>`; not for the dot being viewed.
- Contract fixture: `ScheduledMessage`, its enums, the event and the commands.

## 6. UI

**Composer** (`Composer.tsx`): beside Send, a small chevron button (`aria-label` and title **"Send later"**) that opens a menu titled **"Send later"**:
- one item per preset from the presets helper, its label on the left and its time on the right, muted (`localWeekdayTime`, e.g. "Thu 9:00 AM"); a preset less than a minute away (6 PM at 5:59:30) is not offered, as the core would refuse it;
- **"Pick a date and time…"** opens a small dialog **"Send later"** with a `datetime-local` field labelled **"Date and time"** and buttons **"Schedule"** and **"Cancel"**.
- Choosing schedules the text in the box for the reply target; on success the box is cleared, as after Send. If the box was locked meanwhile (the topic turned busy while the dialog was open) or emptied, nothing is scheduled and the dialog stays open with **"You can't schedule a message right now. Try again once you can send one."** or **"Type a message to schedule."**
- Refusals (shown in the dialog, or as the usual error notice from the menu): **"Pick a time at least a minute from now."**, **"Pick a time within the next year."**

**Card** (`ScheduledCard`, at the end of the conversation, §3.1.11), `role="region"`, `aria-label` **"Scheduled message"**:
- top line, muted, with a clock icon: **"Scheduled for `<when>` · `<target>`"**; `<when>` = `localWeekdayTime` within 7 days, else `localDateTime`; `<target>` = **"current topic"**, **"new topic"**, or **"`<topic title>`"** (`topicTitle`), or **"an older topic"** when its root is not among those shown;
- the text, in the person's bubble style with a dashed border;
- the status line, live (re-rendered each second with `useNow`):
  - before due: **"Sends in `<d>`d `<h>`h"** / **"Sends in `<h>`h `<m>`m"** / **"Sends in `<m>`m"**. Whole minutes are rounded up; the two largest non-zero units are shown ("Sends in 2h 14m", "Sends in 1h", "Sends in 3d 4h", "Sends in 1m");
  - due, not waiting, or `sending`: **"Sending…"**;
  - `waiting = busy`: **"Due · waiting for `<dot>` to finish this topic"**; `approval`: **"Due · waiting for your approval in this topic"**; `usage_limit`: **"Due · waiting for the usage limit to reset"**;
  - `failed`: **"Not sent: `<error>`"** in the warning colour (`text-needs`).
- buttons **"Send now"**, **"Edit"**, **"Cancel"** (none while `sending`).
- **Edit** turns the card into a form: a text box labelled **"Scheduled message"**, a `datetime-local` field labelled **"Send at"** (filled with the message's time, or an hour from now for a message that is due or less than two minutes away, whose own time would be refused), buttons **"Save"** and **"Discard changes"**. Errors from any button show under them (`role="alert"`), e.g. **"This message has already been sent."**, **"This message is being sent now."**, **"This message was cancelled."**

**The sent message** (`Transcript.tsx`, under the person's bubble of a chat run whose payload has `send_later`), muted:
- late: **"Sent late (was due `<time>`)"**, `<time>` = `localDayTime(due_at, queued_at)` (e.g. "Sent late (was due 9:00 AM)");
- on time: **"Sent as scheduled"**;
- `new_topic`: also **"Started a new topic: the one it was written in can't be continued."**

**Dot header menu** (`DotMenu.tsx`): a label **"`<n>` scheduled"** after the Today line, shown when n > 0 (n = pending rows).

## 7. Edge cases

- **Dots closed over the due time**: sent on start, late, in due order. Several due for one topic: the first now, each next after the dot has answered the one before.
- **Asleep PC**: the 30-second cap sends within about 30 seconds of waking.
- **Crash between claim and send**: §4.5 recovery.
- **The person edits while it is being sent**: their Save gets "This message is being sent now." / "This message has already been sent."; nothing they typed is sent.
- **The topic was continued by the person meanwhile**: the scheduled message goes after the newest message there (the parent is read at send time).
- **The person switched to a new topic**: it still goes to the topic it was written in; the card says which.
- **The dot is deleted**: its scheduled messages go with it.
- **The dot is disabled**: it is still sent (the chat path does not refuse a disabled dot); the run waits as any chat run of a disabled dot does.
- **Retry of a sent message that was not delivered** (delivery tracking): the retry is a plain chat message and has no "Sent late" line.
- **Time zone / DST change before it goes**: `due_at` is an instant; the card shows it in the new local time.

## 8. Testing

- Store: insert and read; pending list order; due list; conditional claim / edit / cancel (each only from its states); recovery of `sending` with and without a run carrying its id.
- Send (fake clock, `hold_runs(true)`): reply to the topic's end; a reply after a message the person typed meanwhile; new topic for no root, a missing root, a root of another dot, an end without a session and an ended workflow step's thread (with `new_topic`); waits for a busy topic and for approval, then sends; waits for the Claude limit, also when the chat path refuses at it after the target was read, while another refusal at the limit fails; failed for an org removed; `late` true at 2 minutes, false at 30 seconds; two due for one topic go in order; a pass settles a `sending` row whose run is stored.
- Races: Edit and Cancel after the claim → `Conflict`; the sender after Cancel sends nothing; a stale pass neither sets a waiting reason on nor fails a row an Edit moved later; two Send nows, one after the other and at once, send once.
- Validation: past, under a minute, over 365 days, empty text, a root of another dot.
- Loop: `spawn_send_later` sends at the due time on the fake clock; a due message waiting on a busy topic is sent by the loop alone once the topic is free (the 30-second cap); after a restart its first pass sends what came due, once.
- Roles: a Viewer reads the list and is refused schedule, Send now, Edit and Cancel.
- App: commands; one failed notice per row; none for the viewed dot; contract.
- UI: the countdown texts at fixed `now`; the composer menu calls `schedule_message` with the preset's ISO time and the reply target's root; the custom dialog's refusals; card buttons and their errors; the edit form; "Sent late (was due …)", "Sent as scheduled", the new-topic note; the menu's "N scheduled".

## 9. Collisions with parallel work

Stacked on `feat/snooze-done` (migration 0031, which creates `ui/src/lib/later.ts`); migration 0032; attachments (0033) builds on §4.3's extension point. Shared files: `runtime.rs` (`chat`), `model.rs`, `events.rs` (core and app), `contract.rs`, `startup.rs`, `Composer.tsx`, `ReplyArea.tsx`, `Timeline.tsx`, `Transcript.tsx`, `DotMenu.tsx`, `api/types.ts`, `api/commands.ts`, `runtime-events.ts`. Snooze also touches `DotMenu`-adjacent counts and `events.rs` notices; expect mechanical conflicts and a contract regeneration.

## 10. Not in this part

- Scheduling while the dot is working on the topic (the composer stays locked then, §3.1.7).
- Repeating messages (that is a schedule).
- A list of all scheduled messages across dots, and an undo for Cancel.
- Scheduled answers to an open question.
