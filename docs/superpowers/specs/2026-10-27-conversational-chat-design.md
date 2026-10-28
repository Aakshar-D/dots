# Conversational chat (a Claude-app-like conversation)

- Date: 2026-10-27
- Status: approved in conversation, builder check done, awaiting review of this document
- Builds on: `2026-10-22-chat-home-design.md` (the home, its column, topics, reply target, composer), `2026-10-08-dots-design.md` (prompts, policy, approvals), the dot memory spec (the `Memory` tool, the pattern the new tool follows).

## 1. Goal

Talking to a dot should feel like talking to Claude. Today, any chat message is appended to the dot's instructions as its task: a question ("why did it skip the coffee?") makes the dot run its whole process again and end with a run summary, instead of answering.

After this change, in one conversation the person can:

1. **Ask anything**, about the dot's work or about something unrelated, and get an answer. The dot does not run its process to answer, though it may read files or look at what it did when that helps.
2. **Ask for a change to the work** ("fix the typo in that report", "redo step 3"): the dot makes that change only, not the whole process.
3. **Ask for a change to the dot itself** ("from now on, also email me the summary"): the dot proposes new instructions, shown as a diff the person approves or rejects.
4. **Run the process** when they ask for it ("run it now", "do the usual").

The dot decides which of these a message is from its wording. There is no mode switch or button.

The conversation screen also takes the Claude app's **layout** (section 5). Colors stay Dots' own (the deep-teal theme, light and dark); restyling the palette is out of scope.

Success: the person asks a dot a question in its chat and gets an answer with no process run; asks for an instruction change and sees it as a diff they approve, after which the editor shows the new instructions; and says "run it" and the process runs as today.

## 2. Non-goals and anti-goals

- **Non-goals:** a Claude-like color palette or typography; replies that type out word by word (Dots has no text-delta events, so streaming would be backend work); changing scheduled, webhook, watcher, app, workflow or follow-up runs; editing a dot's schedule, tools, policy or engine from chat (only its instructions).
- **Anti-goals (failure even if it works):**
  - A dot changes its own instructions without the person's approval.
  - A scheduled or other unattended run changes the dot's instructions.
  - A question still produces a full run of the process.
  - The new layout loses something the home does today (approvals, questions, run cards, reply target, History, New topic, locked composer, errors kept in the composer).

## 3. The chat prompt

In `crates/dots-core/src/prompt.rs`, `build_prompt_full`, for a **person's chat message** that starts a thread (a `TriggerKind::Chat` run with no `session_id`, whose message the person typed in a chat topic). Chat runs Dots makes itself (an answer to a run's question, `checkins::continuation`; a decision on an app proposal, `decide_app_proposal`; a reply on a run topic that cannot resume) keep today's task framing, so a dot that asked something mid-task still finishes it. *My call.* For a person's chat message:

- The dot's instructions are framed as **what the dot does when asked to run**, not as the task of this message. They stay under the global instructions and with the memories as today.
- The message section tells the model: this is a conversation with the person who owns the dot. Answer questions directly, including ones unrelated to your instructions. Run your process only when the person asks for it. Make a requested change to your work without redoing the rest. When the person asks you to change how you work from now on, propose new instructions with the instructions tool (section 4) instead of only promising to.
- The standing rule "End with a short summary: what you did, what changed, and what needs human review." applies only when the dot ran its process or changed files. For a plain answer, the answer is the reply. The other standing rules (workspace directory, queued approvals) stay.
- Chat runs that resume a session keep sending only their message, as today, except for the "Runs since" section (3.1); the framing of the thread's first prompt carries over.
- Every other trigger's prompt is unchanged.

### 3.1 The dot's recent runs

A chat topic runs in its own session and copy of the files, so on its own it cannot see what the dot's scheduled and other runs did. Decided by the person: the chat gets a short list of the dot's recent runs.

- **What:** a `## Your recent runs` section in a person's chat message's first prompt, after the instructions and memories. It lists the dot's newest 5 ended runs outside this topic, newest first. For each: trigger and time ("Scheduled · 2026-10-27 08:00"), status, and the summary's first 300 characters, or else the error's. *Count and length are my call.* No run, no section.
- **Runs since:** a chat topic lives for days while schedules keep running. So a person's message that resumes a session is prefixed with a `## Runs since your last message` section when runs of the dot ended after the topic's previous run was queued (same format, at most 5). Without such runs, the resume sends only the message, as today. *My call.*
- **Fixes to a past run's output:** the framing tells the dot that a past run's files are in that run's own copy. When asked to change them, it says to use **Reply** on that run's card, and doesn't edit its own copy. *My call.*
- **Not included:** a tool to open a past run's full events or files. The list is the only view of past runs from the main chat; Reply gives the full run.

The exact wording is the builder's, tested against both engines (Claude and local) with a question, a work change, an instruction change and "run it now".

**Replies to a run topic** (the **Reply** on a run card) resume the scheduled run's session, whose first prompt was the full task. The builder's call is whether that resume needs a short framing line too; the behaviour in section 1 must hold for them.

## 4. The instructions tool

A new Dots tool that lets a dot propose new instructions for itself, modeled on the memory tools (`memory.rs`, `server/mcp.rs` on the Claude engine, `engine/local/tools/` on the local engine) for its wiring, and on `ProposeApp` (`apps/propose.rs`) for how the proposal is stored and decided.

- **A proposal, not an approval** *(my call)*. An ordinary approval parks after `approval_wait_secs`, locks the topic, and on decision starts a `Resume` run whose fixed prompt ("Continue the task, then end with your summary.") would make the dot run its process. So, like `ProposeApp`: the tool stores a proposal (a new table, its migration and contract types) and answers at once ("Sent for review. Finish your turn."). The run ends normally, and the topic stays open for more messages.
- **Name and input:** one tool (for example `propose_instructions`) with the full new instructions text and a one-line reason. One policy name (for example `Instructions`) in `policy.rs`, treated like `Memory`: a rule names it alone, and an ask rule wins over an allow rule.
- **Policy** *(my call)*: every preset gets an `Instructions` rule that allows the proposal to be made, ReadOnly included, since nothing changes until the person decides. An upgrade step in `store/upgrade.rs` adds the rule to existing dots, as `finish_0012`/`0015`/`0016` did for earlier tools. An explicit `Instructions` deny rule turns the tool off.
- **Only for a person's chat message:** offered only to the runs that section 3 frames as conversation, and to their continuations in that topic. Other runs don't see the tool, so an unattended run cannot propose. If it is called from any other run, it answers "not performed".
- **The proposal card** (inline in the conversation and in the Inbox) shows "<dot> wants to change its instructions", the reason, and a before/after diff of the instructions (`components/DiffView.tsx`), with Approve and Reject. It is not part of "Approve and always allow" or approve-all.
- **On approve:** the dot's instructions become the proposed text, through the same path the editor saves by (`update_dot`), so validation stays consistent; an open editor refreshes. The decision continues the topic as a new chat message, as app proposals do ("The person approved your new instructions."), so the dot can confirm in a sentence. Runs already going keep their prompt; later runs get the new instructions.
- **Stale proposal:** if the dot's instructions changed between the proposal and the decision (the person edited them in the editor), approving is refused with a sentence saying so.
- **On reject:** the topic continues with "The person declined your new instructions." and the dot carries on.

## 5. The conversation layout

Layout A from the conversation: the existing home (`ui/src/screens/home/`), restyled. Same routes, data, live updates and behaviour as the chat-home spec; only presentation changes.

```
┌──┬──────────────┬──────────────────────────────────────────────┐
│◉ │ + New dot    │ shopper ▾                History · New topic │
│✉³│ Dots         │                                              │
│▤ │ ● shopper    │      ▸ Scheduled · 08:00 · ✓ done · Ordered… │
│⚙ │   Placed or… │                  ┌─────────────────────────┐ │
│  │ ○ reader     │                  │ why did it skip coffee? │ │
│  │   Summaris…  │                  └─────────────────────────┘ │
│  │ ◐ nightly    │      The coffee you usually buy was out of   │
│  │   next 02:00 │      stock, so I left it out…                │
│  │              │      ▸ Read instructions · Proposed an edit  │
│  │              │      ┌ shopper wants to change its instr… ┐  │
│  │              │      │ + If an item is out of stock, …    │  │
│  │              │      │ [Approve] [Reject]                 │  │
│  │              │      └────────────────────────────────────┘  │
│  │              │      Replying to: current topic              │
│  │              │      ╭────────────────────────────────────╮  │
│  │              │      │ Reply to shopper…                  │  │
│  │              │      │                                (↑) │  │
│  │              │      ╰────────────────────────────────────╯  │
└──┴──────────────┴──────────────────────────────────────────────┘
```

- **Icon rail:** unchanged (Home, Inbox with count, Library, Settings).
- **Dots column:** stays the list of dots, styled like Claude's chat list: quiet rows, the open dot highlighted by a background, not a border; status mark, name and last line as today; **+ New dot** at the top.
- **Header:** slim, no bar look. The dot's name with ▾ opens the menu that ⚙ opens today (Edit, Run now, Enabled); the status mark stays next to the name; History and New topic on the right.
- **Reading column:** messages, cards and composer share one centered column with a max width of about 720px; extra width is empty space on both sides.
- **Your messages:** rounded bubbles aligned right, max about 75% of the column.
- **The dot's replies:** plain text with markdown (`components/Markdown.tsx`), full column width, with no bubble and no name label.
- **Tool steps:** one muted line per group of consecutive steps ("▸ Read 2 files · Ran a command") that expands on click into today's per-step chips (`ToolChip` in `Transcript.tsx`). Today each step is its own chip, so the one-line summary is new.
- **Run cards, approvals, questions, check-ins, browser calls, failures:** stay inline as cards within the column, restyled to rounded, light-bordered cards matching the new look. The instruction-change card is section 4's.
- **Composer:** a rounded box pinned to the bottom of the column; the textarea grows with its text up to a few lines, then scrolls; a round send button inside the box at the bottom right; the "Replying to" chip and the open-question bar sit just above the box. Enter sends, Shift+Enter new line, locked and error states as today.
- **Empty conversation:** the dot's name and "Ask <dot> something" centered in the column, with the composer under it, as Claude's new-chat screen.
- **Themes:** everything uses the existing theme tokens, so light and dark both work; no new colors.
- **Small windows:** below the column's width, the column fills the space with the usual side padding.

`components/Transcript.tsx` is shared with the run screen. Restyling for the conversation must not change the run screen's look, so the conversation either passes a variant to it or wraps it, the builder's call.

## 6. Constraints

- Both engines (Claude, local) get the new prompt and the instructions tool.
- No change to the routes, the reply-target rules, the topics or the data the home loads.
- Existing tests keep passing, updated only where they assert on markup or styles this changes.
- Prompt tests in `crates/dots-core` that assert on the chat prompt text are updated to the new framing; prompts of other triggers must not change (their tests stay as they are).

## 7. Testing

- **Prompt** (Rust unit tests next to `prompt.rs`): a person's chat message's first prompt has the conversation framing and not the unconditional summary rule; Dots-made chat runs keep the task framing; the recent-runs section lists the newest 5 ended runs outside the topic, cut to 300 characters, and is absent with none; a resumed message gets "Runs since your last message" only when runs ended since, else only its message; every other trigger's prompt is byte-for-byte as before.
- **Instructions tool** (Rust): offered only to a person's chat runs on both engines; called from another run it answers "not performed"; it stores a proposal and answers at once, and the topic stays open; approve saves through `update_dot` and continues the topic; a stale proposal is refused; reject continues the topic with the decline; an explicit deny rule turns it off; the upgrade step adds the rule to existing dots of every preset.
- **UI** (Vitest, Testing Library, the fake backend): the instruction-change card shows the diff and the reason and calls approve or reject; the conversation renders your messages as right-aligned bubbles and the dot's text without a label; the header menu holds Edit, Run now and Enabled; the empty conversation; the composer's send button, locked and error states (the existing sending tests carry over).
- `bun run test`, `bun run typecheck`, `cargo test -p dots-core`, `cargo test -p dots-app` pass.
- **Smoke** (`tools/smoke/webview.mjs`, debug build, fresh data folder) with a real model: ask a question and see an answer with no process run; after a scheduled or manual run, ask about it in the main chat and get an answer from its summary; ask an unrelated question and get an answer; ask for an instruction change, approve it, and see it in the editor; say "run it now" and see the process run. Screenshots of the conversation in light and dark, with and without messages.

## 8. Left to the builder

- The exact prompt wording (section 3), and whether a reply to a run topic needs a framing line.
- The tool's and policy's final names.
- How Transcript gets its conversation look without changing the run screen.
- Spacing, radii and sizes within the existing theme tokens.

## 9. As built (plan A: sections 3 and 4)

Plan A (`docs/superpowers/plans/2026-10-27-conversational-chat-a.md`) builds sections 3, 3.1 and 4; plan B builds section 5. Where the spec left room:

- **A person's chat message** is a chat run whose payload has `"typed": true`, which `Runtime::chat` sets (`prompt::is_typed`). The runs Dots makes (`checkins::continuation`, the decisions on app and instruction proposals) have no such key and keep the task framing.
- **The conversation framing** is `prompt::CONVERSATION`, after the memories and the recent runs and before "Message from the user", with one line on changing instructions: the tool's name when the run has it, else `prompt::NO_INSTRUCTIONS_TOOL` (edit them in the editor). The summary rule becomes `prompt::CHAT_SUMMARY_RULE`.
- **A reply to a run topic** gets no framing line of its own: it resumes the run's session as before, with "Runs since your last message" when runs ended since.
- **The recent runs** are the newest 5 runs of the dot that ended (succeeded, failed or cancelled) outside the topic, other chat topics included, latest end first, one line each: trigger, local end time, `done`/`failed`/`cancelled`, and the first 300 characters of the summary, else of the error, on one line, cut with `…`. "Since the last message" means ended after the parent run was queued.
- **One proposal waits per dot**: a newer proposal replaces the waiting one (status `replaced`, card gone). The instructions are 1 to 20,000 characters, the reason 1 to 500; a proposal equal to the current instructions is refused. Proposals go with their dot.
- **The tool is offered** to a run whose topic's root is a person's chat message, not to a workflow step's run, and not when the policy denies `Instructions`. The preset rule is `Instructions` allow, last in every preset; `Instructions(...)` is refused, and the CLI never sees the rule.
- **Deciding** needs the topic's runs to have ended (the refusals of a reply), so a card can't be decided while the dot still works on the chat. Approving checks the instructions are still the proposal's `base`, else refuses with `instructions::STALE` and the card waits. The chat goes on with `instructions::APPROVED` or `REJECTED`, in the session when the last run had one.
- **The Inbox** has a "New instructions" group of the waiting proposals, each with its dot and Open conversation, and counts them, in the tray's count and in the rail's Inbox count alike. They don't take the needs colour (a proposal is not an approval), and the dots' needs-you counts don't count them. The conversation shows only waiting proposals, under the topic's runs.
- **The card's diff** is a line diff of the two texts (`lib/line-diff.ts`), not `DiffView`, which shows git patches.
