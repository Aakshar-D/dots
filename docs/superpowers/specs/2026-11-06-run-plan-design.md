# Live plan and timeline: see each step a dot plans, who owns it and where it is

- Date: 2026-11-06
- Status: approved (owner's request, 2026-11-06)
- Builds on: the conversation (`Transcript`, conversational chat spec section 5), run events
  (`run_events`, `EngineEvent`), the workflow card's step marks (teams spec section 10.4), the
  approval guards' plumbing tools (`PLUMBING_TOOLS`).

## 1. Goal

"A visible plan and timeline. Show each task's steps, who owns each one and its status live,
instead of chat updates."

Team workflows have a step view; an ordinary chat or scheduled run has none. The person has to
read the dot's words and the tool-step lines to know where it is.

## 2. Behaviour

1. **Where.** Each run in a conversation (and on the run screen) gets one card, above the dot's
   reply, under the person's message: the *plan card*. It uses the workflow card's language: a
   bordered card, ✓ done, a spinner for the step being worked on, ○ to do, ✕ failed, ● waiting
   on the person, – stopped.
2. **Plan.** When the dot keeps a to-do list (the `claude` CLI's `TodoWrite`), the card shows its
   newest state: each step in order, done ones muted, the one being worked on in its "-ing"
   words (`activeForm`) with "Now", and "3 of 7 done" with a thin progress bar.
3. **Owner.** The card is named by the dot ("Plan of Scout"). A helper the dot started with the
   CLI's `Task`/`Agent` tool keeps its own plan, shown nested under the dot's in a dashed box
   named "Helper: <what it was asked>". Its steps never overwrite the dot's plan.
4. **Timeline.** Under the plan, behind a "Timeline · N steps" toggle: what the run did, oldest
   first, each line with its start time, mark, words and duration. Consecutive tool calls of the
   same owner make one line in the transcript's own words ("Read 4 files · Ran a command"); the
   dot's words between calls start a new line. A question or note to the person, a helper, a
   team member or workflow started, and the browser each get a line of their own ("Asked you:
   Which branch?", "Handed to a helper: Check the logs", "Handed to Ada: Write the docs",
   "Started the workflow Release"). Each plan step gets a "Done: <step>" line when it is marked
   done. A helper's lines carry its name. A line opens to its calls and results.
5. **Live.** The card reads the run's events, which the runtime bus already keeps current, so
   it moves as each event arrives. Progress changes are announced politely to screen readers.
6. **Compact.** Open while the run is active; once it ends, one line: "Plan · 7 of 7 done ·
   Took 4m 12s". A step left in progress in an ended run reads "Stopped", one never started
   "Not started". Toggling by hand wins over this.
7. **No plan.** A run without a `TodoWrite` list (the local engine, a quick answer) shows a
   "Timeline" card instead when it handed work on or made at least three calls over two or more
   lines. It starts closed, as one line ("Timeline · 5 steps · Took 1m 05s"), since the
   transcript already shows those steps. A quick answer shows no card. Whatever is closed is
   not in the page at all, so the transcript's lines are never there twice.
8. **Accessible.** Every state is in words (a visually hidden ": Done", the step's title), not
   colour alone, and marks differ in shape. The spinner and bar stop moving under reduced
   motion. Toggles are buttons with `aria-expanded`.

## 3. Plan state (core and UI)

- `EngineEvent::ToolCall` gains `parent: Option<String>`: the stream's `parent_tool_use_id`, the
  `Task`/`Agent` call whose helper made the call. It is left out of the stored JSON when `None`,
  and an event stored before it reads as `None`. The local engine never sets it.
- `ui/src/lib/run-plan.ts` reads the events in `seq` order (duplicates once), so events that
  arrive out of order still give the newest state. Per owner, the newest `TodoWrite` call wins.
  A call whose input has no `todos` array, or whose result is an error, changes nothing; an empty
  list clears the owner's plan. An item with no words is left out; a status other than
  `pending`, `in_progress`, `completed` reads as `pending`; a missing `activeForm` falls back to
  the step's words. A `parent` the run does not know counts as the dot's own.

## 4. Transcript words

The tool-step line names `TodoWrite` "Updated the plan" and `Task`/`Agent` "Started a helper"
instead of "Used TodoWrite". Nothing else in the transcript changes: the card sits above it, the
timeline is closed by default when there is a plan, so the reply is not repeated.

## 5. Asking for a plan

`TodoWrite` is a plumbing tool: every preset already allows it, and only a deny or ask rule the
person wrote changes that (approval guards spec section 5). It writes nothing to disk. No policy
change is needed.

A Claude-engine dot whose policy allows `TodoWrite` gets one more standing rule in its first
prompt (`prompt::PLAN_RULE`): keep a short to-do list for a task of several steps and update it
as each step starts and ends, and skip it for a quick answer. Local-engine dots and a policy
that denies it get no such rule.

## 6. Not in this change

- A team member's own plan inside a delegate step's card: a member's run is a thread of its
  own, with its own plan card on its run page. The delegating run's timeline names the member
  it handed to.
- A live ticking duration on a running line: the working line already counts the run's time.
