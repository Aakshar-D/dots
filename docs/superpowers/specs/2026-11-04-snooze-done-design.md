# Snooze and Done on the Inbox

- Date: 2026-11-04
- Status: draft for the owner's review (the behaviour in §3 is the owner's, decided 2026-11-04)
- Part of: Tier 1 of the build list, item 1 (first of three: Snooze/Done, then Send later, then Attachments). Branch `feat/snooze-done`, stacked on `feat/delivery-tracking`.
- Builds on: `2026-10-22-needs-you-design.md` (Needs you, counts, notifications), `2026-11-03-delivery-tracking-design.md` (not-delivered messages, the sweeper pattern), the check-ins spec (questions and notes).

## 1. Goal

The person can put an Inbox item away until later, or mark it done, without answering it. Today an item stays in the Inbox, and in every count, until it is dealt with in its own way (answered, decided, resumed, reviewed). Some things are for later, and some are dealt with somewhere else.

Success: every Inbox card has **Snooze** (approvals) or **Snooze** and **Done** (everything else). A snoozed item leaves the Inbox and every count, and it comes back at its time, also after a restart, with a notification and a "Back from snooze" tag at the top. A done item leaves the same way and sits in a "Done" list for 7 days with Undo.

## 2. Words

- **Item**: one thing the Inbox lists. It has a **key**: a kind and an id (§4.1).
- **Snoozed**: hidden until a time (`snoozed_until`).
- **Back**: an item that came back from snooze. It shows at the top with the tag until it leaves or is marked again.
- **Done**: hidden by the person, with no time (`done_at`).
- **Hidden**: snoozed or done.
- **Leaves on its own**: the item is no longer in the Inbox for its own reasons (a question was answered, an approval decided, a watcher resumed, a run reviewed).

## 3. Behaviour (decided with the owner)

1. **Every Inbox item** can be snoozed: questions, notes, approvals, runs to review, not-delivered messages, schedules that need attention, paused watchers, runs held for usage, tools changed, apps to review, new instructions, team scope problems.
2. **Done** is on every item **except approvals**. An approval is decided, not marked done.
3. **Presets**: In 1 hour · This evening (6 PM) · Tomorrow morning (9 AM) · Next week (Monday 9 AM) · Pick a date and time. Local time. They live in one UI helper, `ui/src/lib/later.ts`, which Send later reuses.
4. **A snoozed item** is hidden from the Inbox and from every count: each dot's needs-you and unread, the Home rail's unread total, the Inbox rail count, and the tray (Inbox count, tooltip and Needs you). It shows in a collapsed **Snoozed** section, with when it comes back and **Unsnooze**.
5. **Coming back**: a notification **"Back from snooze: `<item>`"**. The item returns at the **top** of the Inbox with a **"Back from snooze"** tag, and the counts include it again. Snoozes are stored in the database, so they survive restarts. A sweep on the `Clock` wakes items whose time passed, also those that passed while Dots was closed.
6. **Done** hides the item from the Inbox and every count. It moves to a collapsed **Done** section at the bottom, with **Undo**. Items done more than 7 days ago drop off that list.
7. **An item that leaves on its own** just leaves: from the Inbox, the Snoozed list or the Done list. No notification.
8. **Snoozing an approval** does not change how the approval waits or parks. It only hides it. The run still waits, the approval card in the conversation still shows, and deciding it there takes it away.

### 3.9 Builder's calls

1. **A passed preset is hidden, not rolled.** "This evening (6 PM)" is not offered once it is 6 PM or later. "Tomorrow morning" is always 9 AM of the next calendar day, and "Next week" is the next Monday after today (on a Monday, the following Monday). Rolling "This evening" to tomorrow would show a label that is not true.
2. **Unsnooze and Undo put the item back as it was**, with no tag and no notification.
3. **A back item keeps its tag** until it leaves on its own, or is snoozed or marked done again. It is not timed.
4. **The 7 days are for the Done list only.** A done item that has not left on its own stays hidden after 7 days, and it is no longer listed. (Otherwise an unread note marked done would come back after a week.) Its mark is deleted when it leaves on its own.
5. **Done does not change the item.** A note marked done is not marked read, a question marked done stays open in the conversation, and a run to review marked done is not marked reviewed. Done is about the Inbox only.
6. **Approvals are snoozed per run**, as the Inbox groups them: the group's **Snooze** snoozes each approval it shows. A call the run asks for later is a new item and shows.
7. **The Inbox gains a "Not delivered" section.** It has none today: a not-delivered chat message counts in its dot's needs-you but is listed only as a failed run to review, and the 30-minute ones are not listed at all. The section lists chat runs that are not delivered and not dismissed, with the existing delivery line (Retry, Discard). Those runs are left out of "Runs to review", so nothing is listed twice.
8. **Wake timing**: the sweep runs once when Dots starts and then every 30 seconds. An item comes back at the first sweep at or after its time, so up to 30 seconds late. It stays hidden until then, so the notification and the item arrive together.
9. **One notification per sweep.** One item gives "Back from snooze: `<title>`". Several items woken in the same sweep (typically after a restart) give one notification, "Back from snooze: `<n>` items". It is shown even while the person is reading that dot's conversation, because they asked to be reminded.
10. **Snooze limits**: the time must be in the future and at most 365 days ahead.
11. **The Inbox rail count** is computed by one shared UI function, and it now counts team-scope problems as the core's `inbox_count` already does.

## 4. Core

### 4.1 Item keys

`InboxKind` and the id for each kind. The id names the item **as it is now**, so a mark never hides a later item with the same subject:

| kind | id | title (notification, Snoozed and Done lists) |
|---|---|---|
| `message` | message id (questions and notes) | `<dot> asks: <text>` / `<dot> says: <text>` |
| `approval` | approval id | `<dot> asks to use <tool>` |
| `review` | run id | `<dot>: run to review` |
| `not_delivered` | run id | `<dot> didn't get your message` |
| `attention` | the failed run's id (`attention_run_id`) | `<dot>'s schedule needs attention` |
| `watcher` | `<watcher id>@<paused_at>` | `<dot>'s watcher <name> paused` |
| `held` | run id | `<dot>: run waiting to run` |
| `tools` | `<server id>/<scope>` | `<server name> tools changed` |
| `instructions` | proposal id | `<dot> proposed new instructions` |
| `app_review` | proposal id | `<dot> sent <app title> for review` |
| `team_scope` | `<team id>/<dot id>` | `<dot> in team <team> needs attention` |

A title uses the first line of any text and is cut to 80 characters with `…`. Titles are worked out from the item when they are read. They are not stored.

### 4.2 Storage

- **Migration 0031** (`0031_inbox_marks.sql`; 0029 is delivery tracking, 0030 is taken by topic-tools):

```sql
CREATE TABLE inbox_marks (
    kind TEXT NOT NULL,
    item_id TEXT NOT NULL,
    snoozed_until TEXT,  -- RFC 3339 UTC; hidden until the sweep wakes it
    done_at TEXT,        -- marked done
    back_at TEXT,        -- woke from snooze: the "Back from snooze" tag
    marked_at TEXT NOT NULL,
    PRIMARY KEY (kind, item_id),
    CHECK ((snoozed_until IS NOT NULL) + (done_at IS NOT NULL) + (back_at IS NOT NULL) = 1)
);
CREATE INDEX inbox_marks_due ON inbox_marks (snoozed_until) WHERE snoozed_until IS NOT NULL;
```

- A mark is always in exactly one state. An item with no mark is shown normally.
- **Counts.** A mark hides its item when `snoozed_until` or `done_at` is set. `dot_counts` leaves hidden items out of every term that is an Inbox item: questions (`message`), pending approvals (`approval`), paused watchers (`watcher`), pending app proposals (`app_review`), undelivered chat runs (`not_delivered`). In unread, it also leaves out notes (`message`) and ended runs hidden as `review` or `not_delivered`. Quiet runs are not Inbox items and are unchanged. `count_waiting_messages` leaves out hidden messages.

### 4.3 The Inbox

- `Runtime::inbox()` builds the full list as today, plus `not_delivered` (§3.9.7) and the waiting messages (read for keys only). It then reads the marks and returns:
  - each section **without hidden items** (back items stay in their sections);
  - `back`: marks of back items still listed, newest `back_at` first;
  - `snoozed`: marks of snoozed items still listed, soonest `snoozed_until` first;
  - `done`: marks of done items still listed with `done_at` in the last 7 days, newest first.
  - Each mark comes with the item's `title` and `dot_id` (`None` for `tools`).
- The questions and notes stay in `messages_list`, as today. The UI leaves out the hidden ones using `snoozed` and `done` (§6).
- `inbox_count()` counts the shown sections, `not_delivered` included, and the waiting messages that are not hidden.

### 4.4 Snooze, Done and their undo

- **`inbox_snooze(items, until)`**: `Invalid` when `items` is empty, or `until` is not after now or more than 365 days ahead ("Pick a time in the future."). `NotFound("That is no longer in the Inbox.")` when any item is not in the full Inbox. Otherwise, in one transaction, every item gets `snoozed_until = until` (its done or back state cleared).
- **`inbox_done(item)`**: `Conflict("An approval is decided, not marked done.")` for `approval`; `NotFound` as above. Otherwise `done_at = now`.
- **`inbox_unsnooze(item)`**: deletes a snoozed mark; `NotFound("That is not snoozed.")` otherwise.
- **`inbox_undo_done(item)`**: deletes a done mark; `NotFound("That is not done.")` otherwise.
- Each one sends **`RuntimeEvent::InboxUpdated`**.

### 4.5 The wake sweep

- **`Runtime::wake_snoozed()`**, one pass. With no marks it does nothing. Otherwise it builds the full Inbox's keys once, deletes every mark whose item has left (§3.7), then wakes each snoozed mark with `snoozed_until <= now`: it sets `back_at = now` and clears `snoozed_until`. If anything changed, it sends `InboxUpdated`. If anything woke, it also sends **`RuntimeEvent::SnoozeEnded { items }`** with the woken marks and their titles.
- **`Runtime::spawn_snooze_waker(self: &Arc<Self>)`** runs one pass at once and then one every 30 seconds of the runtime's `Clock`, until shutdown. The app starts it right after the event forwarder subscribes, so the first pass's notification is heard. It holds the runtime weakly. *(Builder's call: the pass needs the whole Inbox, which only the runtime builds, so it cannot be spawned inside `Runtime::start` as the delivery sweeper is.)*

## 5. App

- Commands `inbox_snooze(items, until)`, `inbox_unsnooze(item)`, `inbox_done(item)`, `inbox_undo_done(item)`. `items` and `item` are `InboxKey { kind, id }`, and `until` is RFC 3339.
- `lib.rs` setup calls `rt.spawn_snooze_waker()` after `events::spawn_forwarder`.
- Notifier: `SnoozeEnded` gives `Notice::BackFromSnooze { items }`, with the texts in §3.9.9. Body for one item: `It is at the top of your Inbox.`; for several, their titles joined with `; ` and cut to 120 characters. The route is `/inbox`. It has no dot, so it is never quiet.
- The tray already refreshes on every event that is not a run event, so `InboxUpdated` refreshes it.
- Contract fixture: `Inbox`'s new fields, `InboxMark`, `InboxKey`, `Enums.InboxKind`, the two events, the four commands, and **`InboxKeys`**: the keys the core gives the sample Inbox and sample message, so the UI's own key functions are checked against the core's.

## 6. UI

- **Card actions** (right end of every row or card header): **Snooze** (a menu button with a clock icon) and **Done** (check icon). An approvals group has **Snooze** only, in its header line.
- **Snooze menu**: `In 1 hour` · `This evening (6 PM)` · `Tomorrow morning (9 AM)` · `Next week (Monday 9 AM)` · `Pick a date and time…`. The last one opens a dialog titled **Snooze until**, with a `Date and time` field (`datetime-local`), and **Snooze** / **Cancel** buttons. A time not in the future shows `Pick a time in the future.` and Snooze stays off.
- **Back from snooze**: the top section, heading **Back from snooze**, lists the back items with their own cards, newest first. Each card has a small **Back from snooze** tag.
- **Not delivered**: a section after Approvals. Each row shows the dot's name, the message's first line, the delivery line (`Not delivered: <reason>`, Retry, Discard), and **Open** (`/chat/<dot>/<run>`).
- **Snoozed** (collapsed, at the bottom): a toggle **Snoozed (`<n>`)**. Each row shows `<title> · Back <when>` and **Unsnooze**. `<when>` is the time alone for today (`6:00 PM`), the weekday and time within 6 days (`Mon 9:00 AM`), else the local date and time.
- **Done** (collapsed, last): a toggle **Done (`<n>`)**. Each row shows `<title> · Done <relative time>` and **Undo**.
- Both toggles are closed at first and stay as the person left them while the screen is open. Neither one shows when it is empty.
- **Empty state**: when nothing is shown, the existing empty state with its copy, which now mentions "messages that did not reach a dot". The Snoozed and Done toggles still show below it when they have items.
- **Counts**: the rail's Inbox count comes from one function (`inboxWaiting`), which leaves out hidden messages and adds `not_delivered` and `team_scope`. The Home count and the dot badges come from the core's counts.
- **Live**: `inbox_updated` refreshes the Inbox and the overview (the unread total is under it). `snooze_ended` refreshes nothing itself, because an `inbox_updated` goes with it. `message_posted` and `message_updated` already refresh the messages.

## 7. Edge cases

- **Dots closed past a snooze time**: the first pass at start wakes it and notifies once. The next pass finds nothing.
- **An item leaves while snoozed** (the question is answered in the conversation): it leaves the Snoozed list at once, because lists only show items still in the Inbox. Its mark is deleted at the next pass, and it never notifies.
- **A watcher resumed and paused again**: new `paused_at`, new key, shown. The same goes for a schedule that fails again (new run id).
- **Snoozed twice**: the later call wins (an upsert).
- **Snoozing a back item**: allowed. It loses its tag.
- **Runs held for usage** count as one Inbox line, however many there are. Snoozing every held run takes the line out of the count. Snoozing some of them leaves it in.
- **A dot deleted**: its items leave, and their marks go at the next pass.
- **Clock moved backward**: a snooze simply wakes later. Nothing is lost.

## 8. Testing

- Store: snooze, done, unsnooze and undo-done states and errors; the check constraint; `wake_due` wakes only marks due by the given time; `prune_marks` keeps only the keys given; `dot_counts` and `count_waiting_messages` leave out hidden items, kind by kind; a back mark counts again.
- Runtime: each section filtered; `back`, `snoozed` and `done` lists and their order; Done list drop-off at 7 days with the item still hidden (`FakeClock`); approval Done refused; validation errors; an item that left is not listed; the Not delivered section and its removal from Runs to review; `inbox_count`.
- Wake sweep (`FakeClock`): wakes at its time and not before; one `SnoozeEnded` per pass; nothing for an item that left; a restart wakes what came due while closed.
- App: commands; the notice texts and route; one notice for several items; the notice is not quiet for the viewed dot; contract and `InboxKeys`.
- UI: `later.ts` presets at fixed local times (6 PM boundary, Monday, the last day of a month); `inboxKeys` against the fixture; the Snooze menu and dialog; Done and Undo; Unsnooze; the Back section first, with the tag; the rail count with a snoozed message; approvals without Done.

## 9. Collisions with parallel work

Stacked on delivery tracking (`feat/delivery-tracking`). It uses migration 0031, and 0030 is topic-tools on a parallel branch: sqlx applies a lower version merged later. Send later (`feat/send-later`, 0032) is stacked on this branch and reuses `later.ts` and `LaterMenu`. Shared files: `runtime.rs` (`inbox`), `store/dots.rs` (`dot_counts`), `store/messages.rs`, `model.rs`, `events.rs` (core and app), `contract.rs`, `InboxScreen.tsx`, `Layout.tsx`, `api/types.ts`, `runtime-events.ts`.

## 10. Not in this part

- Snooze from a notification or from the tray.
- Snooze or Done in a dot's conversation (only in the Inbox).
- Custom presets, or setting the evening and morning hours.
- Marking a note read, or a run reviewed, when it is marked done.
