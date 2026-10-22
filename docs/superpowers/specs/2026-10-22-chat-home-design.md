# Chat-first home (chat-first UI, part A)

- Date: 2026-10-22
- Status: approved in conversation, awaiting review of this document
- Part of: the chat-first UI, split into A (this: the layout), B (what needs me: unread counts, notifications that open the right place, tray links) and C (visual design: brand, light and dark themes, restyled components). B and C get their own specs.
- Builds on: `2026-10-08-dots-design.md` (screens), `2026-10-20-checkins-followups-design.md` (check-ins in chat, sections 9 and 10), `2026-10-21-browser-design.md` (browser cards).

## 1. Goal

Dots opens on a conversation with your dots. Your dots are listed like chat contacts; picking one shows its conversation, where you read what it did, answer what it needs and give it the next job. Approvals, questions, notes, follow-ups, browser actions and the dot's other runs (scheduled, webhook, manual) appear in that conversation, in time order.

What it fixes today: the app opens on a table of dots; chat is a separate screen with a dot dropdown and a list of threads; scheduled runs are never in a conversation; the Dots table, Chat and Inbox each show part of what a dot needs.

Success: a person can do a day's work with their dots from the home alone (start a job, follow it, approve, answer, reply to a scheduled run's result, start a new topic, open older topics), and reaches the editor, run details and Inbox in one click from it.

## 2. Words

- **Topic**: one line of runs, a root run and the runs that continued it (`root_run_id`). Every run already belongs to one. A topic whose root is a chat run is a **chat topic**; one whose root is a scheduled, webhook or manual run is a **run topic**.
- **Current topic** of a dot: its newest chat topic (by the root's `queued_at`). A dot with no chat topic has none.
- **Conversation** of a dot: the current topic and the last 20 run topics whose roots were queued after the current topic's root, in time order; earlier ones are in History. With no current topic, the dot's last 20 run topics.
- **Reply target**: the topic your next message continues (section 5).

## 3. Layout and routes

```
┌──┬───────────────┬────────────────────────────────────────┐
│◉ │ Dots      [+] │ shopper  ● running        ⚙  History  New topic │
│✉³│ ● shopper     │──────────────────────────────────────────│
│▤ │   Placed ord… │ You: order the usual                     │
│⚙ │ ○ reader      │ shopper: Opened shop · Clicked "Add…" ▸  │
│  │   Example Do… │ ┌ wants to click "Place order" ───────┐  │
│  │ ◐ nightly ⏰  │ │ [Allow once] [Always…] [Deny]       │  │
│  │   next 02:00  │ └─────────────────────────────────────┘  │
│  │               │ ▸ Scheduled · 08:00 · ✓ done · Sent 3…   │
│  │               │ Replying to: current topic               │
│  │               │ [ Message shopper…                Send ] │
└──┴───────────────┴────────────────────────────────────────┘
```

- **Icon rail** (replaces the text sidebar of `app/Layout.tsx`): Home, Inbox (with its count, as today), Library, Settings, each with a tooltip and an `aria-label`. The status banners and toasts stay as they are.
- **Routes** (`app/routes.tsx`):
  - `/` → `/home`.
  - `/home`: the dot last opened (kept in `localStorage`, ignored when it no longer exists), else the first dot of the column, else the empty home (section 8).
  - `/home/:dotId`: that dot's conversation.
  - `/home/:dotId/:rootRunId`: an older topic of that dot, alone (section 4.4).
  - Redirects: `/dots` → `/home`; `/chat` → `/home`; `/chat/:dotId` → `/home/:dotId`; `/chat/:dotId/:runId` → `/home/:dotId/<that run's root>` when it is not in the dot's conversation, else `/home/:dotId`. Unknown paths → `/home`.
  - Unchanged: `/dots/new`, `/dots/:dotId` (the editor), `/runs/:runId`, `/inbox`, `/library`, `/settings`.
- **Dots column**: one row per dot from `dot_overview`, newest activity first (the later of the active run's and last run's `queued_at`; a dot with no run sorts by name after the others). A row shows:
  - a status mark: **waiting** (an awaiting-approval run, or an open question of the dot), **running** (a queued or running run), **attention** (its schedule needs attention), **off** (disabled), else **idle**; first match wins, in that order;
  - the name;
  - the last line: the last run's `summary`, else its `error`, else its chat message, cut to one line;
  - the next scheduled fire (`schedule_state.next_fire`) as "next 02:00", or the next follow-up as today's Dots list shows it.
  - **+** opens `/dots/new`. Selecting a row opens `/home/:dotId`. The column is a list with arrow-key navigation and `aria-current` on the open dot.
- **Conversation header**: the dot's name and status mark; **⚙** a menu with Edit (the editor), Run now, and an Enabled switch (what the Dots table offered); **History**, a menu of the dot's topics older than the conversation (title: the first line of the root's message or its trigger and time; newest first; up to 50), each opening `/home/:dotId/:rootRunId`; **New topic** (section 5).

## 4. The conversation

### 4.1 Chat topics

Rendered as the chat renders a thread today (`components/Transcript.tsx`, reused): each run's message as yours, the dot's text, its tool steps folded into one line that expands, check-in items (notes, questions, follow-up lines), browser calls, applied calls, a pending approval card per waiting approval, a failure alert.

### 4.2 Run topics

A run topic is a **run card**: its trigger and time ("Scheduled · 08:00", "Webhook · 09:12", "Run now · 10:40"), its status, the first line of its summary or error, and two buttons, **Details** (`/runs/:runId`) and **Reply** (section 5). While the run waits on an approval or has an open question, the card is open and shows the approval card or the question in it. The runs that continued it (your replies) render under the card as in 4.1.

### 4.3 Order and loading

- Items are ordered by `queued_at` of each topic's root; inside a topic, by run.
- Data, all from existing commands: `list_runs(dotId, 200)` for the topics; `run_thread(root)` for each topic in the conversation; `list_run_events` and `approvals_for_run` for each run when it is rendered expanded; `messages_list(dotId)` for check-ins. A run card that is closed loads no events.
- Live: the existing `runtime` events keep it current, as the chat does today (`lib/runtime-events.ts`): a new run of the dot appears at the end (a new run topic as a card), statuses change in place.
- The view sticks to the bottom while new items arrive, unless the person scrolled up; then a "New activity ↓" button appears.

### 4.4 An older topic

`/home/:dotId/:rootRunId` shows that one topic alone (4.1 or 4.2), with a bar "Older topic · Back to the current conversation". Its composer continues it (section 5).

## 5. Sending

- **Reply target**, shown in a chip above the composer: "Replying to: current topic" or "Replying to: Scheduled · 08:00 ✕".
  - By default: the current topic, whatever ran after it; with only run topics shown, a new chat topic. The default never moves while the person types: a run topic gets a message only through its **Reply** (or when it is the older topic shown alone). The chip is announced politely (`aria-live`).
  - **Reply** on a run card makes that topic the target; ✕ returns to the default.
  - In an older topic (4.4), that topic.
- **Send**: `chat(dotId, message, <the target topic's last run>)`. With no target (a dot with no topic, or after **New topic**), `chat(dotId, message, null)`, which starts a new chat topic; it becomes the current topic.
- **New topic**: the next message starts a new chat topic; the chip says "New topic" with ✕ to undo. Nothing is sent until the person sends.
- **Locked**: while the target topic has a run that has not ended, the composer is disabled and says why ("shopper is working on this…" or "Waiting for your approval above"), as the chat does today. A topic whose last run has no session shows today's "cannot be continued" note and a **New topic** button.
- **Open question**: as today, the bar above the composer with the choices and "Reply to question"; answering uses `message_reply`. Opening the conversation marks its open questions read, as the chat does today.
- **Errors**: a refused or failed send keeps the text in the composer and shows the error's text (the backend's sentences, as today).
- Enter sends, Shift+Enter makes a new line.

## 6. Inbox and the other screens

- **Inbox** stays the list of everything that needs the person across dots. Each message and approval gets **Open conversation**, to `/home/:dotId` (or the older topic's route when the item's run is not in the conversation). Answers in either place update the other live, through the existing events.
- **Chat screen** (`screens/ChatScreen.tsx`, `lib/chat-threads.ts`): removed; its tests move to the home's tests where the behavior carries over (thread building, composer, check-ins in chat, follow-ups).
- **Dots screen** (`screens/DotsScreen.tsx`): removed; the column and the header's ⚙ menu carry what it showed and did (attention, active and last run, enabled, run now, next follow-up). Its tests move likewise.
- **Unchanged**: the run screen, the dot editor, Library, Settings.
- **Message cards** keep their links to the dot and the run, and gain **Open conversation**.

## 7. Collisions with parallel work

Most UI files were written by Aakshar; the person tells him before work starts. New code goes into new files under `ui/src/screens/home/` and `ui/src/lib/` (`home-topics.ts`: building topics and the conversation; `dot-status.ts`: the status mark). Edits to shared files stay small: `app/routes.tsx` (routes, redirects), `app/Layout.tsx` (the rail), `screens/InboxScreen.tsx` and `components/MessageCard.tsx` (the link), and the removal of `ChatScreen.tsx`, `DotsScreen.tsx` and `chat-threads.ts`. `components/Transcript.tsx` and `components/ApprovalCard.tsx` are reused unchanged where possible. No backend, command, event or migration changes: `dot_overview`, `list_runs`, `run_thread`, `list_run_events`, `approvals_for_run`, `messages_list`, `follow_ups_list`, `chat`, `message_reply`, `run_now`, `set_dot_enabled` already exist.

## 8. Edge cases

- **No dots**: the home shows "Create your first dot" with a button to `/dots/new`; the rail still works.
- **A dot deleted while open**: back to `/home`.
- **A dot with only run topics**: the conversation is its last 20 run topics; sending starts a new chat topic unless a card's **Reply** was used.
- **A topic with more than one end** (runs older than the one-line rule): the line of runs is shown as `run_thread` returns it; the reply continues its newest run, and the backend's refusal text is shown if it refuses.
- **Many runs**: the column reads `dot_overview` (one call); a conversation never loads more than the 200 newest runs of the dot; older ones are reached through History.
- **Disabled dot**: its conversation stays readable; sending is allowed (`chat` does not check whether the dot is enabled). The header shows "off".

## 9. Testing

- UI tests (Vitest, Testing Library, the fake backend in `src/test/backend.ts`), one file per feature next to the screens:
  - the column: order, status marks in their priority, last line, next fire, selection and keyboard;
  - topics and the conversation (`home-topics.ts` unit tests): current topic, run topics after it, the 20 run topics without a chat topic, ordering;
  - the timeline: chat topics as transcripts, run cards closed and open (approval, question), continuations under a card;
  - sending: the default target, Reply on a card, ✕, New topic, older topic, locked states, errors kept in the composer;
  - check-ins in the conversation (moved from `ChatCheckIns.test.tsx`), follow-ups (moved from `FollowUps.test.tsx`), browser cards;
  - routes and redirects; the Inbox's Open conversation; the rail's labels and Inbox count; the empty home; a deleted dot.
- `bun run test` and `bun run typecheck` pass; `cargo test -p dots-app` passes (no contract change expected).
- Smoke (`tools/smoke/webview.mjs` with a debug build and a fresh data folder): the home opens on the last dot; a local dot's chat, a scheduled run card and replying to it, an approval answered inline, a question answered from the bar, New topic and History, the redirects from `/chat` and `/dots`, the Inbox link. Screenshots of the home with and without dots.

## 10. Out of scope (B and C)

Unread counts per dot or topic, notifications that open the right conversation, tray links, notification wording (B). Brand colors, light theme and a theme switch, typography, restyled components (C): the new screens use today's components and styles. Renaming or merging topics, search, pinning dots, drag to reorder.

## 11. As built

The plan (`docs/superpowers/plans/2026-10-22-chat-home.md`) built sections 1 to 9 with these choices where the spec left room or said otherwise:

- **Open conversation** (section 6) links to the run's old chat address `/chat/:dotId/:runId`; the redirect of section 3 opens the dot's conversation when the run's topic is in it, else the older topic alone. A note of Dots' own links to `/home/:dotId`.
- **The rail** (section 3): each link's tooltip is its `title`; the Inbox's count is in its name too ("Inbox, 4 waiting").
- **Run now** in the header's ⚙ stays on the home: the run shows up at the end of the conversation as a card, whose Details opens the run.
- **The column's next line** is `schedule_state.next_fire` ("next 02:00") also while schedules are paused, which the banner says; the Dots list's "Paused", "Completed", "Expired" and "Retrying at …" are not carried over.
- **A run card** shows today's status badge; open, it shows the root run's pending approvals and its newest open question, not its notes (the Inbox has them).
- **Questions**: opening a conversation marks every unread open question of its runs read; the bar shows the newest open one.
- **The chip's ✕** shows only after Reply or New topic. A default target that is a run topic reads "Replying to: Scheduled · 08:00".
- **An older topic** is loaded with `run_thread(rootRunId)`, so any run of the topic names it; the conversation and an older topic each open with their own reply target and scroll.
- **Tests** are in `ui/src/screens/home/*.test.tsx` and `ui/src/lib/{home-topics,dot-status}.test.ts`, with the fake dot and runs they share in `ui/src/test/home.ts`.
- **The smoke** adds: no request to the old `/dots` or `/chat` screens is left (each redirects); a dot with no run shows "Ask <dot> something"; History lists the older chat and run topics and opens one with "Older topic · Back to the current conversation".

### After the final review (2026-10-22)

- The default reply target is the current chat topic, not the topic with the newest run: with a schedule, every message had gone to the newest scheduled run, and the target could change while the person typed. A run card gets a message only through **Reply**. Decided by the person.
- The conversation shows at most the last 20 run topics after the current topic; the rest are in History. A dot on a short schedule had loaded and reloaded a thread for each of up to 200 run topics.
- A failed reload of the dots keeps the open conversation, its reply target and the text being typed, and shows the error over the screen; only a first load that failed shows the error alone.
