# Topic tools: rename, archive, move and merge

- Date: 2026-11-04
- Status: approved (owner's decisions given with the task, 2026-11-04)
- Builds on: `2026-10-22-chat-home-design.md` (topics, History, New topic), `2026-11-03-skip-when-busy-design.md` (house style).

## 1. Goal

A person can tidy their dots' topics: give a topic a name, put old topics away, move a message that went to the wrong topic, and join two topics that are really one. Old topics put themselves away after a while.

Today a topic is only a line of runs sharing `runs.root_run_id`; it has no row, no name of its own and cannot be changed. The chat-first home spec left "renaming or merging topics" out; this is that part.

## 2. Words

- **Topic**: as in the chat-first home spec: a root run and the runs that continued it (`root_run_id`). The root's id stays the topic's identity (and its address, `/home/:dotId/:rootRunId`).
- **Topic row**: an optional `topics` row holding what a topic has beyond its runs. A topic with no row is active and untitled; no backfill.
- **Active run**: one that has not ended (`queued`, `running`, `awaiting_approval`; `RunStatus::is_terminal` is false).
- **Moved set** of a run: the run and its descendants in its topic (by `parent_run_id`). In a one-line topic that is the run and everything after it.
- **Last activity** of a topic: the latest `queued_at` or `ended_at` of its runs, or the time it was last unarchived if later.

## 3. Behaviour (decided with the owner)

1. **Rename**: a topic gets a title (trimmed, at most 100 characters; empty clears it). History, the archived list and the move and merge pickers show it instead of the title derived from the first message.
2. **Archive / unarchive**: an archived topic is left out of the conversation, History's list and the pickers, and listed under **Archived** in History, where it can be opened and unarchived. Any run added to an archived topic (a message, an answer, a follow-up; `insert_run`) unarchives it, and so does a move or merge into it.
3. **Auto-archive**: a setting, **archive topics idle for N days** (default 30, 0 = off, at most 3650). A periodic sweep archives the topics whose last activity is older than that, never a dot's current topic (newest chat topic), never its most recently active topic, never one with an active run or with something waiting (a pending follow-up, an open question, a pending app or instructions proposal).
4. **Move a message**: move a run's moved set to another topic of the same dot, or to a new topic. The moved runs get the target's root; the moved run's parent becomes the target's last run (by `queued_at`), or none for a new topic, where the moved run becomes the root. Inside a topic runs are read **along the line** (§4.6), so moved runs come after the target's runs, and a reply continues the moved tail.
5. **Merge**: merge topic A into topic B of the same dot: all of A's runs get B's root, A's root's parent becomes B's last run, A's row goes. B keeps its title and archived state.
6. **Refusals** (nothing changes): a run or topic of another dot; a target that is the source; moving a topic's root (that is a merge); a topic (source or target) with an active run, or whose follow-up decision is being made; a topic with a workflow step's run in it; a topic an app started (`app_topics`), which the app tracks by its root.
7. **Session continuity**: a Claude session (and a local one, rebuilt from `parent_run_id`) remembers the runs it ran, not the topic's runs after a move. So every topic a move or merge changes (the source, the target, a new one) is marked to **restart**: the next run in it that would resume a session (any trigger but `resume`, which must finish an approved call in its session) starts a fresh session instead, and its prompt carries the topic's runs so far (§4.4). The mark stays until a restarted run has used its fresh session (it succeeded or waits for an approval), so a run that fails first, or a crash, leaves it for the next run.

## 4. Core

### 4.1 Migration 0035

```sql
CREATE TABLE topics (
    root_run_id   TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    title         TEXT,
    archived_at   TEXT,
    unarchived_at TEXT,   -- counts as activity for auto-archive
    restart       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX topics_dot ON topics(dot_id);
```

No `last_active_at` column: activity is read from the runs, which are already indexed by root.

### 4.2 Store (`store/topics.rs`)

- `TopicInfo { root_run_id, dot_id, title: Option<String>, archived_at: Option<String> }` (model).
- `list_topics(dot_id)`, `set_topic_title`, `set_topic_archived(root, archived, now)` (unarchive sets `unarchived_at`), upserting the row.
- `move_runs(run_id, to: Option<&str>) -> String` and `merge_topics(from, into)`: check every refusal of §3.6 and write in one transaction; they also move `app_proposals.root_run_id` and `instruction_proposals.root_run_id` of the moved runs; a merge moves A's `grants` to B, and a move takes the source's unused grants along with the line's tail (the work an approved call goes on in). Messages (check-ins) and follow-ups name their run, not its topic, so they follow it unchanged. They set `restart` on every changed topic.
- `take_topic_restart(root) -> bool`; `archive_idle_topics(now, days) -> Vec<(dot_id, root)>`.

### 4.3 Runtime

`list_topics`, `rename_topic`, `set_topic_archived`, `move_run`, `merge_topics`, each under the thread lock `chat` uses, and each announcing **`RuntimeEvent::TopicsChanged { dot_id }`** (serde `topics_changed`). `chat` into an archived topic unarchives it.

### 4.4 Restart (runner)

Before a run's workspace and prompt are made: when the run has a `session_id`, is not a `resume`, and `take_topic_restart(root)` is true, the run's `session_id` is cleared (stored and in hand). The stored session is kept (so the topic can still be replied to if the run fails); only the engine is started without it. Its prompt is then a first prompt, with **`## This conversation so far`** before the message: the newest 20 earlier runs of the topic along its line (after "(N earlier messages omitted)"), the person's message (typed or answered) and the run's summary, cut like the recent-runs list.

### 4.6 Line order

`list_thread` (and the UI's `topicOf`) read a topic's runs along the parent links: each run after its parent, the children of one run by `queued_at`, then by id; a run whose parent is not in the topic starts a line of its own by time. Chosen over a stored position column: it needs no backfill and no write on every insert, and it is right for any topic whose parent links are, which every insert keeps. The Inbox's "thread end" query still uses the newest run by time.

### 4.5 Auto-archive sweep

The scheduler's pass (at least once a minute) runs the sweep at most once an hour with `Settings::auto_archive_days`, and announces `topics_changed` for each dot it changed. It rides on the pass, so it does not run while schedules are paused. *Builder's call:* the scheduler loop was chosen over a new loop because it already wakes every minute with a clock tests can drive.

## 5. App

Commands `list_topics(dotId)`, `rename_topic(rootRunId, title)`, `set_topic_archived(rootRunId, archived)`, `move_run(runId, toRootRunId?)` (returns the root the run is now in), `merge_topics(fromRootRunId, intoRootRunId)`. Contract: `TopicInfo`, the `topics_changed` event, `auto_archive_days` in Settings. No notification.

## 6. UI

- `home-topics.ts`: topics carry their row's title and archived state; `topicTitle` prefers the title; the conversation, History and pickers leave archived topics out; `archivedTopics` lists them newest first.
- **History**: the older topics as today, then an **Archived** section.
- **Topic menu** (⋯ in the dot's header, for the topic on screen: the older topic shown alone, else the current topic): **Rename…**, **Merge into…** (picker of the dot's other topics), **Archive** / **Unarchive**.
- **Move to topic…** beside each message the person sent in the conversation (not on a topic's first message): a picker of the dot's other topics and **New topic**.
- **Settings**: "Archive topics idle for (days)", 0 = never.
- Live: `topics_changed` reloads the dot's runs, threads and topic rows. Refusals are shown as the core words them.

## 7. Edge cases

- A run moved into an older or newer topic is shown at its time; the reply still continues the newest run of the topic by time, as before.
- A topic emptied by a move cannot happen: the root never moves without a merge.
- An archived topic merged into another goes; one merged into an archived topic stays archived.
- Deleting a dot or a topic's root run deletes its row (cascade).

## 8. Testing

Store: rows, rename, archive/unarchive, each refusal, the move to another topic and to a new topic, merge, proposals moved, restart marks, the sweep's exclusions. Runtime: the restarted run has no `--resume` and carries the conversation so far; chat unarchives. Scheduler: the sweep archives an idle topic. App: commands and contract. UI: `home-topics` (titles, archived), History's Archived section, the topic menu, Move to topic, the setting.

## 9. Not in this part

Splitting a run out of the middle of a topic without what follows; undo; moving runs across dots; bulk actions.
