# Delivery tracking

- Date: 2026-11-03
- Status: approved (owner reviewed this document, 2026-11-03)
- Part of: Tier 1 of the build list, fourth of seven (watchdog merged #45; skip-when-busy #46 and watch hours #47 in review).
- Builds on: `2026-10-08-dots-design.md` (run lifecycle), `2026-10-22-needs-you-design.md` (Needs you, notifications), `2026-10-26-usage-limits-design.md` (the usage hold), `2026-11-02-stuck-run-watchdog-design.md`.

## 1. Goal

The person can see whether each thing sent to a dot actually reached it: delivered, still waiting (and why), or not delivered (with Retry). Today a message becomes a queued run and nothing says whether the dot ever started on it; a run that fails before the engine takes it, or sits in the queue, is only found by looking.

Success: a chat message that never reaches the dot shows "Not delivered: <reason>" with Retry and Discard, appears in Needs you and notifies once; a message waiting in the queue says why after 5 minutes; each dot's header menu shows today's counts.

## 2. Words

- **Sent**: a run was created (any trigger: chat, schedule, watcher, follow-up, API key, manual).
- **Delivered**: the run's engine started on it — `engine.start(...)` returned successfully in the runner. Recorded as `runs.delivered_at`.
- **Waiting**: sent, not delivered, not ended.
- **Overdue**: waiting for more than **5 minutes** since `queued_at`.
- **Not delivered**: the run ended without ever being delivered (it failed or was cancelled before its engine started), or it has waited **30 minutes**. A run held on purpose for the usage limit never becomes not delivered by waiting (§3.4).
- **Wait reason**: why a waiting run has not started: `busy` (the concurrency limit is full), `usage_hold` (held for the usage limit), `updating` (the runner is held for an update), `dot_busy` (another run of the same dot is active: the dispatcher starts one run per dot), `queued` (none of these; just next in line).

## 3. Behaviour (decided with the owner)

1. **Tracked**: every run, any trigger.
2. **Delivered** = the engine started on it.
3. **Overdue at 5 minutes**: a quiet line on the message / run card: **"Waiting `<n>` min · `<reason text>`"** (texts in §6).
4. **Not delivered**:
   - immediately when a run ends `failed` with no `delivered_at`; the reason is the run's error's first line (cancelled runs are not "not delivered", §3.9);
   - at **30 minutes** of waiting, *except* while the reason is `usage_hold` *(my call: the hold is deliberate and can last hours; it shows its reason but does not escalate)*. The run itself is left queued; it is marked not delivered (`undelivered_at`) and still starts if a slot frees up later — then it becomes delivered and the mark clears *(my call)*.
5. **For chat messages** (trigger `chat`), not delivered:
   - the message shows **"Not delivered: `<reason>`"** with **Retry** and **Discard**;
   - it counts in **Needs you** (until retried, discarded, or delivered);
   - **one** Windows notification: title `<dot> didn't get your message`, body `<reason>` (not for the dot being viewed).
6. **For background triggers** *(my call)*: the state shows on the run card and in the counts; no Needs you item and no extra notification, because their existing alerts already cover failure (a schedule's Needs attention, a watcher's pause).
7. **Retry** creates a new run with the same message / payload and trigger (a chat retry posts the same text again in the same conversation topic); the old run is marked dismissed. **Discard** marks it dismissed *(my call)*; both remove it from Needs you; it stays in history.
8. **Daily counts**: each dot's header menu shows **"Today: `<s>` sent · `<d>` delivered · `<u>` not delivered"** for the local day (runs whose `queued_at` falls on today, local time). *(My call: no Settings totals in this part.)*

Owner's calls: delivered = engine started; overdue 5 min; not-delivered mark + Retry/Discard + Needs you + one notification; counts per dot in its header menu; track every trigger; overdue is a heads-up, a problem at 30 min.

### 3.9 Decided after the builder check *(my calls)*

1. **Cancels are not "not delivered".** A run cancelled before delivery — by the person (Cancel), or by Dots (a dot disabled or deleted, `cancel_if_inactive`, Discard itself) — is shown as cancelled, as today. Not delivered comes only from: a run that **failed** before its engine started (including `interrupted` at restart), or **30 minutes of waiting**.
2. **Existing runs at the upgrade**: migration 0030 sets `delivered_at = started_at` for every existing run with a `started_at`, and `dismissed_at = ended_at` for every existing ended run without one, so history never floods Needs you or today's counts.
3. **Retry of a chat message**: dismiss the old run (and cancel it if still queued), then send the same text through the chat path **with the old run's own `parent_run_id`** (the same place in the thread; a first message becomes a new first message). Do this in that order, so the chat path's "the parent is the last run and every run has ended" rules hold. The conversation shows the dismissed one as a muted "Not delivered" line above the new message.
4. **Retry and Discard are for chat messages only.** `retry_delivery` / `discard_delivery` return `Conflict` for other triggers; background runs show their delivery state but no buttons (retrying them could re-fire side effects: watcher events, follow-up chains, workflow steps, API calls).
5. **Counts** count run rows created by a send: every trigger except rows that only continue an earlier run (approval-resume children and usage-hold requeues are the same send, not a new one). `delivered` = rows with `delivered_at`; `not_delivered` = rows with `undelivered_at` and no `delivered_at` (live state), dismissed ones included. A retry is a second send.
6. **Delivered means `engine.start` returned `Ok`** (for Claude: the process started and took the prompt). A process that dies right after counts as delivered; its failure is the run's own result.

## 4. Core

### 4.1 Storage

- **Migration 0030** (`0030_delivery.sql`; 0027 is master's teams_b, 0028/0029 are skip-when-busy's and watch hours'):

```sql
ALTER TABLE runs ADD COLUMN delivered_at TEXT;    -- engine started on it (RFC 3339 UTC)
ALTER TABLE runs ADD COLUMN undelivered_at TEXT;  -- marked not delivered (ended undelivered, or 30 min waiting)
ALTER TABLE runs ADD COLUMN undelivered_reason TEXT;
ALTER TABLE runs ADD COLUMN dismissed_at TEXT;    -- the person retried or discarded it
```

- `Run` gains `delivered_at`, `undelivered_at`, `undelivered_reason`, `dismissed_at` (all `Option<String>`) and a computed **`delivery: Delivery`** (`delivered | waiting | not_delivered`) *(builder: computed in the row mapping or the model, from the columns and status)*.
- Store: `mark_delivered(run_id, at)` (sets `delivered_at`, clears `undelivered_at`/`undelivered_reason`); `mark_undelivered(run_id, reason, at)` (only when not delivered and not dismissed); `dismiss_run(run_id, at)`; `delivery_counts(dot_id, day_start_utc, day_end_utc) -> (sent, delivered, not_delivered)`; ending a run without `delivered_at` (`finish_run_with`, `fail_if_running`, `cancel_if_inactive`, `recover_interrupted`) also sets `undelivered_at` and the reason in the same write.
- **Needs you** (`dot_counts`) adds chat runs with `undelivered_at` set, `dismissed_at` null and `delivered_at` null.

### 4.2 Runner

- After `engine.start(ctx)` returns `Ok`, `mark_delivered` and emit `RunUpdated`. (A resumed run — approval resume, usage-hold requeue — is delivered when its engine starts; a run delivered once stays delivered.)
- *Builder's call:* exact place relative to the existing post-start code (the watchdog's quiet limit write is next to it).

### 4.3 Wait reasons and the 30-minute mark

- **`Runtime::wait_reason(run) -> Option<WaitReason>`** for a queued run: `usage_hold` if the run is held by the usage-limit hold; `updating` if the runner is held; `dot_busy` if another run of the same dot (not of a workflow step) is active in the runner; `busy` if the runner's slots in use are at `max_concurrent_runs`; else `queued`. Exposed on `Run` as `wait_reason: Option<String>` when the run is queued *(builder: computed when runs are read for the UI; it may be momentarily stale)*.
- **A delivery sweeper** (a small task in the runtime, every 30 seconds, using the scheduler's `Clock` so tests can drive it): for each queued, undelivered, not-dismissed run with `queued_at` older than 30 minutes, wait reason not `usage_hold` and not held for a member's role (members spec §6.3), `mark_undelivered(reason = <wait reason text>)` (which marks only a run still `queued`) and emit `RunUpdated`. *(The 5-minute heads-up needs no core state: the UI computes it from `queued_at`.)*

### 4.4 Retry and Discard

- **`Runtime::retry_delivery(run_id) -> Result<Run>`**: `NotFound`; `Conflict` unless the run is not delivered and not dismissed. Creates a new run with the old run's dot, trigger, message/payload (for chat: the same message text in the same topic, through the existing chat path so permission and limit checks apply), dismisses the old one, returns the new run.
- **`Runtime::discard_delivery(run_id) -> Result<Run>`**: same checks; dismisses. If the run is still queued (the 30-minute case), it is also cancelled *(my call: discarding a waiting message should stop it starting later)*.
- Both emit `RunUpdated`.

### 4.5 Events and counts

- `RunUpdated` already carries the run; no new event.
- **`Runtime::delivery_today(dot_id) -> Result<DeliveryCounts { sent, delivered, not_delivered }>`** for the local day.

## 5. App

- Commands `retry_delivery(runId)`, `discard_delivery(runId)`, `delivery_today(dotId)`.
- Notifier: on a `RunUpdated` where a **chat** run newly has `undelivered_at` (and `dismissed_at` null) — once per run — notice title `<dot> didn't get your message`, body the reason's first line, route `/chat/<dot>/<run>`; not for the dot being viewed.
- Contract fixture: the new `Run` fields, `DeliveryCounts`, the commands.

## 6. UI

- **On a chat message / run card** (wherever a run's live state shows in the conversation and on the run view):
  - waiting ≥ 5 min: muted **"Waiting `<n>` min · `<reason>`"**, reason texts: `busy` → "other runs are busy", `usage_hold` → "held for the usage limit", `updating` → "Dots is updating", `dot_busy` → "this dot is busy with another run", `queued` → "next in line";
  - not delivered (and not dismissed): **"Not delivered: `<reason>`"** in the warning colour, with **Retry** and **Discard** buttons; errors from either show next to them;
  - dismissed: a muted "Not delivered" with no buttons.
- **Dot header menu**: a line **"Today: `<s>` sent · `<d>` delivered · `<u>` not delivered"** (refreshes on `run_updated` for that dot).
- Live: `run_updated` already refreshes conversations and the overview.

## 7. Edge cases

- **App restart while queued**: queued runs survive restarts today; their waiting time keeps counting from `queued_at`.
- **App restart while running**: `recover_interrupted` fails the run; if it had no `delivered_at`, it is marked not delivered ("interrupted").
- **Runner held for an update for a long time**: reason `updating`; escalates at 30 minutes like any wait *(my call)*.
- **A run delivered after being marked not delivered** (a slot freed up at minute 31): delivered wins; the mark and the Needs you item clear; the notification already sent is not recalled.
- **Watcher events held by watch hours** are not runs; they are not tracked until the opening run is created.
- **Approval resume runs** are delivered when their engine starts, like any run.

## 8. Testing

- Store: `mark_delivered` / `mark_undelivered` / `dismiss_run`; ending undelivered sets the mark in the same write; delivered wins over an earlier mark; `delivery_counts` for a local day (boundary at local midnight with a fixed zone); Needs you counts undelivered chat runs only.
- Runner: a run is delivered after `engine.start` succeeds; an engine that fails to start leaves it undelivered with the error as reason; a resumed run keeps `delivered_at`.
- Sweeper (fake clock): marks at 30 minutes with the wait reason; never for `usage_hold`; not for delivered or dismissed runs.
- Wait reasons: each of the five.
- Retry/Discard: errors; retry creates a new run with the same text and dismisses the old; discard cancels a still-queued run.
- App: commands; one notice per chat run, none for background runs, none for the viewed dot; contract.
- UI: the waiting line at 5 minutes (fixed now), each reason text; the not-delivered line with Retry/Discard and their errors; dismissed state; the header counts line.

## 9. Collisions with parallel work

Stacked on #47 (watch hours) for the migration order; migration 0030. Shared files: `runner.rs`, `store/runs.rs`, `store/dots.rs`, `model.rs`, `runtime.rs`, app `commands.rs` / `events.rs` / `contract.rs`, UI conversation components, `api/types.ts`. Parallel branches crash-resend and approval-guards also touch `runner.rs` / `runtime.rs` / `commands.rs`; expect mechanical conflicts and a contract regeneration when merging.

## 10. Not in this part

- Settings › Usage totals across dots.
- Automatic retries.
- Tracking held watcher events before their opening.
