# Lean workflows: a step uses only what it needs

- Date: 2026-11-23
- Status: built (plan 1 of 5 of the workflows refresh). The person asked for the five plans to be built in order, each as its own pull request.
- Builds on: `2026-10-23-teams-workflows-design.md` (teams, workflows, steps, hand-offs, the budget), the auto model spec (sizes of model), `2026-11-19-memory-design.md` (memories in a prompt, the capture step).
- Changes nothing a workflow saved before it does, but for section 4's shorter older summaries: every option's default is the step as it ran.

## 1. Why

The person finds workflows sluggish and costly, and wants them to do more for fewer tokens. An audit of the engine on 2026-11-23 found where a run's tokens and time go:

- Every agent step is a full run of its member, with everything the member has: its memories, its project's map, and the summary of every step before it (up to 4,000 characters each), whatever the step's job is.
- The model is the member's. A step that only tidies a text runs on the same model as the step that wrote the code. With Auto, the summaries of earlier steps are read as the task, so their words ("refactor", "debug") push a simple step to a larger model.
- After each step run the memory capture step asks a model what to remember, and its tokens count against the workflow's budget.
- The loop that moves runs along passes on every message and approval in the app, and on each pass writes the clock of every run in flight, the ones that wait for the person included.
- The UI answers each `workflow_run_updated` by asking the core again for the run, its check, its diffs, its steps' sol runs, every list of runs, the team's workflows, the Inbox and the overview, although the event carries the whole run. A run announces itself each time its spend changes, so a working step makes the screens ask for all of that over and over.

This plan removes that waste and gives each step three options. It adds no new kind of step: plans 2 to 5 do.

**Not measured here:** token counts before and after. Nothing in this plan was run against the real Claude CLI. The savings are what the prompt no longer holds, and which model a step runs on. A real-CLI comparison is the person's to do.

## 2. A step's options

An agent step gains `options`, three fields. JSON: `"options": {"model": null, "lean": false, "reads": null}` beside the step's other fields. A step saved without `options` reads as one with these defaults, and is written back with them.

| Option | Values | Default | What it does |
|---|---|---|---|
| `model` | `small`, `standard`, `large`, or null | null | The size of model the step's run starts with (section 3). Null: the member's own model |
| `lean` | true or false | false | The prompt has no memories and no project map, and the run saves no memory (section 5) |
| `reads` | a list of step ids, or null | null | Which earlier steps' summaries the prompt carries (section 4) |

A `member` deploy has no options: its job is set on the deploy, and it runs as its member does.

**Validation** (teams spec section 5.8 gains one rule): each id in `reads` must be an agent step that comes before the step in run order, the steps inside an earlier branch's paths included. Otherwise: `step "<name>": it can read only an agent step before it, not "<id>"`.

## 3. The size of model

- A step with `model` set runs on the settings' Auto model of that size (`auto_models.small`, `.standard`, `.large`), whether its member is set to Auto or to a model of its own. The run records the size and the reason `set on the workflow step`, shown where Auto's reason is shown.
- Only a member on the Claude engine has sizes. A member on the local engine keeps its own model, and the option does nothing.
- A run that goes on in the thread of a sized step (a send-back, an answered question, a delegate's results) keeps the size, so a session never changes model between turns.
- When the CLI cannot use the model, the run falls back once to another size, as Auto does.
- A step without `model`, of a member set to Auto, is sized by the workflow's task and the step's job only (`/workflow/route` in the run's payload), not by the summaries of earlier steps. A run queued before this plan has no route and is sized by its whole section, as before.

## 4. Which earlier steps a step reads

The `## Workflow` section's `### Earlier steps`:

- `reads` null: every earlier step's summary, as before, but only the newest one whole (up to 4,000 characters). Each older one is cut to 1,000 characters, ending ` [cut]` (`MAX_OLDER_SUMMARY`). The whole text is on the run page, and the work itself is on the branch.
- `reads` a list: the summaries of those steps, each whole, and no other. An empty list: no `### Earlier steps` at all.

Nothing else reads `reads`: the branch a step starts from, what a check shows and what a deploy ships are found as before.

This is the one change to a workflow saved before this plan: its third and later steps get shorter summaries of the steps two or more back.

## 5. A lean prompt

A lean step's first prompt has the global instructions, the member's instructions, the `## Workflow` section, the standing rules, and what the member's tools need said (browser, apps, shared context). It does not have:

- the member's memories and its project's;
- the project map.

Its run is not captured: no model is asked what to remember, so no capture tokens count against the budget. The member's tools are unchanged; a lean step can still search its memories or the graph with the tools for that.

## 6. The loop

- A pass no longer writes anything for a run that waits for the person (a check, a limit, a failed step). Such a run counts no time, and each of the person's actions that lets it go on (approve, send back, retry, add budget, allow a send-back) sets its clock first, as before, so the wait is not counted afterwards either.
- A pass is woken by a message or an approval only when its run works for a workflow step. A message or an approval of any other run of the app wakes none. A run that cannot be read counts as a step's: a pass too many costs little.
- The 30 s tick, the wake on a step run's change and the wake on a start are unchanged.

## 7. The UI

**The builder.** An agent step's settings sheet gains, under the delegate fields:

- **Model size**: `The member's own model`, `Small: quick and cheap, for simple steps`, `Standard: most work`, `Large: the hardest work, and the most usage`. Hint: `For a member on Claude. The sizes are the Auto models in Settings.`
- **Lean prompt** (a switch), with what it leaves out.
- **Pick the earlier steps it reads** (a switch). Off: `It gets every earlier step's summary: the one just before it whole, older ones shortened.` On: a **Reads** list of the agent steps earlier in run order, none ticked at first.

A new step has the defaults.

**Events.** A `workflow_run_updated` carries the whole run, so the run page and every loaded list that holds the run are given it, with no question asked. The UI asks again only when the run moved: its status, what it waits on, its current step or a step run's status changed, or the run is new to this window. Then it asks for the run's check, its steps' sol runs, the lists of runs, the team's workflows, the workflow, the Inbox and the overview. A branch's diff is never asked again: it is of one commit. A question about the run or a list that is in flight when the event arrives is asked again once it settles, since its answer may be older than the event.

## 8. Dropped from the first sketch

- **One session across steps of the same member.** A resumed session reads its whole earlier transcript again, at cache-read price while the cache is warm and at full price once it has gone cold. A fresh lean prompt with a summary is smaller than either. Send-backs still resume, as the teams spec says: there the earlier work is what the step needs.
- **A larger default for runs at once** (Settings, 2). It is the person's setting, and it caps every run of the app, not only workflows.

## 9. Testing

- **Core:** the options' JSON and their defaults for an older step; the `reads` rule; the summaries a step gets with `reads` null, a list and an empty list; a lean step's prompt; the model and reason of a sized step's run; a sized step on a non-Auto and an Auto member, in a thread, and on the local engine; the route text Auto reads; no time counted, and none after the approval, for a run that waited an hour of passes.
- **UI:** the three fields in the sheet and what a save sends; the agent steps a step may read; the cache after an event for a run that moved, one that only spent more, and one not seen before.
- Not tested: anything against the real Claude CLI.

## 10. Known gaps

- A lean step still gets the tool definitions of every MCP server its member uses. Leaving servers out per step needs the run's tool set to follow the step, not the member: plan 2 does that for steps that need no model at all, and a per-step tool list is not built.
- The budget is still checked only before a run starts, so a running step can go past it.
- A `member` deploy has no options.
- The step card does not show a step's options; only its sheet does.
- No test covers a lean step's skipped capture or its missing project map: the tests' runner has no capture model and no graph. Both are one guard each in `runner.rs`.
