# What needs me (chat-first UI, part B)

- Date: 2026-10-22
- Status: approved in conversation, awaiting review of this document
- Part of: the chat-first UI. A (the chat-first home, `2026-10-22-chat-home-design.md`) is merged; this is B; C (visual design) follows.
- Builds on: `2026-10-22-chat-home-design.md`, `2026-10-20-checkins-followups-design.md` (notifications, section 10), `2026-10-08-dots-design.md` (tray).

## 1. Goal

A person sees at a glance which dots need them and what is new, and a notification takes them straight to the conversation it is about.

What it fixes today: nothing tracks what the person has seen of a dot; the column shows a status mark but no count; a notification says "waiting in the Inbox" and a click on it does nothing; the tray only opens the Inbox; a dot whose conversation the person is reading still pops a notification.

Success: the column says how many things each dot has for the person; the Home rail icon says the total; a click on a notification brings Dots forward on the right conversation; the tray lists the dots waiting for the person; nothing pops up for the dot being read.

## 2. Words

- **Needs you** (of a dot): its open questions (posted, not answered or dismissed) plus its pending approvals.
- **Seen** (of a dot): the time the person last looked at its conversation, `seen_at`, kept by the core; none for a dot never looked at since this part shipped.
- **New** (of a dot): its runs that ended (succeeded, failed or cancelled; any trigger, chat included) after `seen_at`, plus its notes posted after `seen_at`. With no `seen_at`, nothing is new.
- **Unread** (of a dot): needs you + new.
- **Viewing**: the dot whose conversation is open on the home in the focused Dots window, or none (another screen, the window not focused, hidden in the tray, minimized).

## 3. Core

- **Migration 0010** (`0010_seen.sql`): `ALTER TABLE dots ADD COLUMN seen_at TEXT` (RFC 3339, null at first). The parallel developer is told 0010 is taken.
- **`DotOverview`** (returned by `dot_overview`) gains `needs_you: u32` and `unread: u32`, computed as section 2 says from the store. `Dot` gains `seen_at: Option<String>`.
- **`Runtime::mark_dot_seen(dot_id)`** sets `seen_at` to now and emits a `dot_seen { dot_id }` runtime event (so other windows and the tray refresh). Unknown dot: `NotFound`.
- **Viewing** is held by the app, not stored: `Runtime::set_viewing(Option<dot_id>)` keeps it in memory; `Runtime::viewing()` reads it. While a dot is viewed, a run of it that ends and a note it posts mark it seen (the core calls `mark_dot_seen` itself), so its count stays at needs-you only.

## 4. App

### 4.1 Commands

- `mark_dot_seen(dotId)` and `set_viewing(dotId | null)`, thin wrappers of section 3. The contract fixture gains `DotOverview.needs_you`, `DotOverview.unread`, `Dot.seen_at` and the `dot_seen` event.

### 4.2 Notifications (`events.rs`)

- **Which events notify**: unchanged (an approval request, a non-chat run that ends or parks, a schedule that needs attention, a question, a note while `notify_on_notes` is on), except: **nothing for the dot being viewed**.
- **Wording** (title, body), replacing every "Inbox" instruction:

| Notice | Title | Body |
|---|---|---|
| Question | `<dot> asks` | the question's first line |
| Approval request | `<dot> needs your approval` | `<tool>: <one-line preview of the call>` (the tool alone when there is no preview) |
| Run parked on an approval | `<dot> is waiting for your approval` | the run's summary's first line, else "A call waits for your decision." |
| Run succeeded | `<dot> finished` | the summary's first line, else "The run succeeded." |
| Run failed | `<dot> failed` | the error's first line, else "The run failed." |
| Schedule needs attention | `<dot>'s schedule needs attention` | the error's first line, else "Its run failed." |
| Note | `<dot> says` | the note's first line |

- **Clicking**: each notification gets an id; the app keeps the route of the last 100 (`id → route`). At start the app registers one handler with the notification plugin (`Notification::on_action`). A click (`tap`) on a notification: the main window is shown, unminimized and focused, and the app emits the existing `navigate` event with its route. Routes: `/chat/<dot>/<run>` for a notice about a run or an approval (the home's redirect opens its topic: the conversation or the older topic), `/home/<dot>` for a question, a note or a schedule. An id it no longer knows: the window is shown, without a route.
- **Limit**: Windows reports a click only while the toast is on screen; a click in the Action Center opens Dots where it was. The handler being refused at start is logged; notifications still show.

### 4.3 Tray (`tray.rs`)

- The menu: **Open Dots**; **Needs you ▸** a submenu with one item per dot whose needs-you is above 0, `"<dot> (<n>)"`, most waiting first, then by name, at most 10, each showing the window on `/home/<dot>`; with none, one disabled item "Nothing waiting"; **Inbox (n)** as today; **Pause schedules**; **Quit**. The submenu is rebuilt with each count refresh (the tray already refreshes on every event that may change the Inbox, and now on `dot_seen`). The tooltip stays "Dots: n to review".

## 5. UI

- **Column** (`DotsColumn`): a count badge on a row whose `unread` is above 0, after the name; named for assistive technology ("3 unread"), in the waiting colour when `needs_you` is above 0, else muted. Rows sort: needs you above 0 first, then the home's order (newest activity first, then by name).
- **Rail**: the Home link shows the sum of `unread` (the Inbox count's style; "99+" above 99); its name says it ("Home, 4 unread").
- **Viewing and seen** (a hook in `DotPane`): when the conversation of a dot is open on the home and the window has the focus, the UI calls `set_viewing(dot)` and `mark_dot_seen(dot)`; on leaving it (another dot, another screen, the window losing the focus or being hidden), `set_viewing(null)`. A failed call is retried at the next change, with no message. An older topic counts as viewing its dot.
- Live: `dot_seen` refreshes the overview (and so the badges); the existing events already refresh it for runs, approvals and messages.

## 6. Collisions with parallel work

Shared files get small additions: in the core, a migration, `store/dots.rs` (the column and the overview's counts), `model.rs` (fields), `runtime.rs` (two methods, the viewing state), `events.rs` of the core (one event); in the app, `events.rs` (the notifier's wording, the quiet rule, the click routes), `tray.rs` (the submenu), `commands.rs` and `lib.rs` (two commands, the handler at start), `contract.rs`. In the UI, `DotsColumn.tsx`, `Layout.tsx` (the Home count), `DotPane.tsx` (the hook), `api/types.ts` and `commands.ts`. New code goes into new files where it can (`ui/src/screens/home/use-viewing.ts`, `apps/dots-app/src/notify_routes.rs`).

## 7. Edge cases

- **A deleted dot** drops out of the counts, the tray and the viewing state; its routes in the notification map lead to `/home`, which opens another dot.
- **After the migration** `seen_at` is null for every dot: only needs-you counts, so no wall of old results.
- **Many dots waiting**: the tray lists 10; the Inbox has the rest.
- **Two windows** (not today's app, one window): the last `set_viewing` wins.
- **Clock changes**: `seen_at` and `ended_at` are both the core's UTC clock.

## 8. Testing

- Core: the counts (needs you, new runs after seen, notes after seen, none before a first seen, cancelled and chat runs counted, deleted dots gone); `mark_dot_seen` (sets, emits, unknown dot); viewing marks seen as runs end and notes arrive.
- App: the notifier's quiet rule for the viewed dot; the wording of every notice; the route of every notice; the id → route map keeps the last 100; the tray's submenu labels, order, limit and "Nothing waiting".
- UI: badges and their names, order with needs-you first, the rail total and "99+", viewing and seen calls on open, switch, blur and route change, the retry after a failure.
- Smoke (debug build, fresh data folder): a real notification clicked opens the conversation; the tray's Needs you opens a dot; no notification while viewing that dot; badges clear on opening a dot. Screenshots where Windows allows.

## 9. Out of scope

Per-topic unread counts; marking unread by hand; an approve button on a notification (a decision stays in the app, where the call is shown whole); badges on the taskbar icon; sounds; the visual design (C).
