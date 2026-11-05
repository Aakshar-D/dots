# Teams and workflows: agent orchestration with a visual builder

- Date: 2026-10-23
- Status: agreed with the person in a question round (layouts picked in the visual companion), written for their review
- Builds on: `2026-10-08-dots-design.md` (runs, workspaces, the runner), `2026-10-13-approvals-v2-design.md` (parking and resuming), `2026-10-20-checkins-followups-design.md` (Dots' own tools, park and resume on `ask`), `2026-10-22-chat-home-design.md` (topics), `2026-10-22-needs-you-design.md` (needs you, notifications), `2026-10-22-deep-teal-design.md` (the look)
- Integrates later with: the app library (being designed in a parallel session; its first app is an Asana-like project manager). Section 12 says where.

## 1. Why

A dot works alone. It cannot hand work to another dot, and the check-ins spec put "dots messaging each other" out of scope. The person wants to organize dots into **teams** and run **automated workflows** across them, drawn in a **visual builder**.

The first team, in the person's words: a **Dots management team**. Its members are a **user helper** (the person's personal assistant, which they chat with), a **task manager** and a **dev agent**. One run of it:

1. The person tells the user helper about a bug or a feature, in chat.
2. The user helper turns it into a task and starts the team's workflow.
3. The task manager decides how to split the work and starts one or more dev agents.
4. The dev agents do the work, each in its own worktree.
5. The person checks the result.
6. On approval, the work is deployed. What "deployed" means depends on the outcome and the project: the dev work can be on Dots itself or on a separate app the person wants to push.

**Success:** the person builds that team and workflow in the builder, chats one request to the user helper, watches the run from the chat and from a run page, approves once, and gets a pushed branch with an open PR (or a ready branch, if the workflow says so), without touching anything else.

**Anti-goals, from the person:** agents burning through the Claude subscription's usage; a team stuck in a loop sending work back and forth. Either one makes the feature a failure even if everything else works.

## 2. Decisions

| Question | Decision | Whose |
|---|---|---|
| Who decides the hand-offs | Both. Drawn steps are the backbone, and a step can let its agent start other members at runtime, inside limits set in the builder | the person |
| Where requests come from | The user helper dot, in the person's chat with it | the person |
| Deployment | Configurable per workflow: stop at a ready branch, or push (and open or merge a PR). Both must be possible | the person |
| Builder layout | B: a top-to-bottom step list (like GitHub Actions or Zapier), not a free canvas and not lanes | the person |
| Watching a run | Both: a run page (the builder's list coming alive, budget on top) and a live card in the chat with the user helper | the person |
| Guards | A budget per workflow run and a cap on send-backs; at a limit the workflow pauses and asks the person | proposed, accepted |
| Projects | The team keeps its own project list (name and repo) for now; the PM app replaces it later | the person ("continue, integrate later") |
| Team members | Existing dots, each keeping its own instructions, engine, tools and policy | my call |
| Dev agent copies | Runs started by a workflow are exempt from "one running run per dot"; each gets its own worktree | my call |
| A deploy needs a check | The builder refuses a deploy step that no check step comes before on every path | my call |
| Deploy is done by Dots, not an agent | `git push` and `gh` are run by the core after the check, so the push is exactly what the person approved | my call |
| A send-back | Picks up where the step left off: its agent's session is resumed with the note, on the same branch (section 5.4) | the person |
| Who commits | Dots commits what an agent step leaves uncommitted when its run ends, with hooks off (section 6.1) | my call |
| What the token budget counts | New input plus output; cache reads are shown, not counted (section 9) | my call |
| Which branches a check shows and a deploy pushes | The latest agent step's branches before the check (section 6.2) | my call |
| The clock while waiting on the person | Paused (section 9) | my call |
| Test run | A real run whose deploy stops at a ready branch (section 5.1) | my call |

## 3. Words

- **Team**: a named group of dots. A dot can be in several teams.
- **Member**: a dot in a team, with a **role** (free text, e.g. "task manager") shown in the builder and given to agents in their prompt.
- **Project**: an entry on the team's project list: a name and a local git repo. A workflow run works on exactly one project.
- **Workflow**: a named, ordered list of steps belonging to one team, with a trigger and a budget. A team can have several workflows.
- **Step**: one entry of the list (section 5).
- **Workflow run**: one execution of a workflow. It has a task text, a project, a status, its spend against the budget, and the step runs it made.
- **Step run**: one step's execution inside a workflow run. An agent step's run is backed by one or more ordinary dot runs.
- **Hand-off**: what a finished step passes to the next one: its summary, the branches its runs produced, and its outcome label (section 6).
- **Send-back**: a jump to an earlier step, from a check (the person) or from an agent step that allows it. Counted against the cap.

## 4. Data model

New tables, in one new migration. The number is fixed when the plan is written: master has 0012 (memories), and the app library may take the next one.

**teams**: `id` TEXT PK (ULID), `name` TEXT UNIQUE, `description` TEXT, `created_at`, `updated_at`.

**team_members**: `team_id` FK (ON DELETE CASCADE), `dot_id` FK (ON DELETE CASCADE), `role` TEXT; PK (`team_id`, `dot_id`).

**team_projects**: `id` TEXT PK, `team_id` FK (ON DELETE CASCADE), `name` TEXT (unique within the team), `repo_path` TEXT (must be a git repo when saved), `default_branch` TEXT NULL (null: found as worktrees find it today), `remote` TEXT (default `origin`), `created_at`, `updated_at`.

**workflows**: `id` TEXT PK, `team_id` FK (ON DELETE CASCADE), `name` TEXT (unique within the team), `definition` TEXT (JSON: trigger, steps, budget; section 5), `enabled` INTEGER, `webhook_token` TEXT, `created_at`, `updated_at`. The definition is validated as a whole on save (section 5.8).

**workflow_runs**: `id` TEXT PK, `workflow_id` FK (ON DELETE SET NULL), `team_id`, `definition` TEXT (a copy of the workflow's definition at start: editing a workflow never changes a run in flight), `project_id`, `task` TEXT, `trigger` TEXT (`chat` | `manual` | `schedule` | `webhook`), `origin_run_id` TEXT NULL (the chat run that started it, FK `runs`, ON DELETE SET NULL), `status` (`running` | `waiting` | `succeeded` | `failed` | `cancelled`), `waiting_on` TEXT NULL (`check` | `budget` | `send_backs`), `current_step` TEXT NULL, `test` INTEGER (0/1, section 5.1), `spent_runs`, `spent_tokens` (as section 9 counts them), `active_ms` (section 9), `send_backs` INTEGER, `extra_budget` TEXT (JSON: what "Add budget" added), `error` TEXT NULL, `started_at`, `ended_at`.

**workflow_step_runs**: `id` TEXT PK, `workflow_run_id` FK (ON DELETE CASCADE), `step_id` TEXT (the step's id in the definition), `attempt` INTEGER (1, then +1 for each send-back that reaches the step again), `status` (`running` | `waiting` | `succeeded` | `failed` | `cancelled` | `skipped`), `handoff` TEXT NULL (JSON, section 6), `decision` TEXT NULL (a check's `approved` or `sent_back`, with the note), `deploy_result` TEXT NULL (JSON: pushed branches, PR URLs, errors), `started_at`, `ended_at`.

**runs** gains `workflow_step_run_id` TEXT NULL (FK, ON DELETE SET NULL). `runs.trigger` gains the value `workflow`. A run's resume children (approvals, `ask`) inherit the column, so they count against the same budget.

Deleting a dot that is a member of a team removes it from the team. A workflow whose steps name that dot is marked invalid (section 5.8) and cannot start until fixed. Deleting a team asks first when it has workflow runs in flight, then cancels them.

## 5. Steps

The definition's `steps` is a list. Each step has an `id` (stable, made by the builder), a `kind`, a `name` shown in the list, and its own fields. Steps run in list order unless a branch or a send-back says otherwise.

### 5.1 Trigger (exactly one, always first)

- `chat`: a member starts the workflow with the `start_workflow` tool (section 7) during a chat with the person. The trigger names which members may do it (the user helper, in the first team). The workflow run's `origin_run_id` is that chat run.
- `manual`: "Run now" on the team page or the builder, or "Test run" in the builder. The person types the task and picks the project. Both run the saved definition (the builder saves first). A **Test run** is a real run in every way except that every deploy step acts as `branch`: nothing is pushed. It is flagged (`workflow_runs.test`) and labelled "Test" on the run page and the team page. The workflow's enabled switch arms the `chat`, `schedule` and `webhook` triggers and Run now; a Test run starts while the workflow is off, so a new workflow can be tried before it is armed (plan C). Every other start of an off workflow is refused with `workflow <name> is turned off`, and a workflow with a problem refuses a Test run too.
- `schedule`: Dots' schedule rules (Scheduler v2), with a fixed task text and project. My call: no catch-up and no retries. A slot that comes due while an earlier run of the same workflow is still in flight (waiting on a check included) is skipped, with a log line. Workflow schedules keep their own state, keyed by workflow, not the dot-keyed `schedule_state`. "Pause schedules" pauses them too.
- `webhook`: `POST` to the workflow's URL, `http://127.0.0.1:<port>/workflows/<id>/trigger`, with its token in an `Authorization: Bearer <token>` header. The body's `task` and `project` (by name) are used, falling back to fixed values set on the trigger. The token is shown once, when it is made, as a dot's is. At most 3 runs of one workflow may be in flight, waiting included, whatever started them (`MAX_WEBHOOK_IN_FLIGHT`, the person's decision, 2026-11-04): past that the webhook answers `429` with `too many runs in flight` and starts nothing. The cap is checked after the token (401) and the 409s (a workflow turned off or with a problem), before the body is read. Only webhook starts are capped.

### 5.2 Agent step

A member does one job. Fields: `member` (a dot of the team), `job` (instructions for this step, added to the dot's own), `can_send_back_to` (optional: earlier step ids this agent may send work back to), `outcomes` (optional: a list of labels the agent may report, used by a later branch).

Dots queues one run of the member dot with trigger `workflow`. Its prompt is the dot's usual prompt plus a `## Workflow` section: the team and the member's role, the task, the project, this step's job, the hand-offs of the steps before it (section 6), and which workflow tools it has (section 7). The step ends when that run (and any resume children) ends: it succeeds with the run's summary as its hand-off; a failed or cancelled run fails the step.

### 5.3 Agent-decides step (delegate)

An agent step whose member may start other members. Extra fields: `may_start` (member dot ids), `at_once` (default 3), `total` (default 6).

The member's run gets the `start_member` tool. Each call queues a run of the named member with the given sub-task, in its own worktree (section 8). The member then finishes its turn. When every member it started has ended, Dots resumes the delegating run with their results (each one's summary, branch and status), as it resumes after an `ask`. The delegating agent can start more (within `total`) or finish. Its final summary, together with every started run's branch, is the step's hand-off.

In the first team, the task manager's step is this kind with `may_start: [dev agent]`.

### 5.4 Check (the person)

The workflow pauses (`waiting`, `waiting_on: check`) and the check appears in **Needs you** (section 10). Fields: `name`, `show` (what to put in front of the person: the hand-off summary, and the diff of each branch from the project's default branch), `send_back_to` (the step a send-back goes to; default: the step before).

The person **approves** (the workflow goes on) or **sends back** with a note. They can also **stop** the workflow run.

**What the check shows is read from the core** (plan C): the step, the hand-off summary, and each shown branch (section 6.2) with its tip commit, or `null` for a branch that is gone. Reading it changes nothing. **Approve names the tips the page showed.** Under the loop's lock the core reads the shown branches again; if the set differs, or any tip differs from what git says now, it refuses with `The branches changed since the check was shown: <branch>. Look at the check again.` (several branches joined by `, `) and records nothing. The page then reads the check again. The recorded approval is the tips the page showed, which equal git's.

**View diff** shows a branch's committed changes from the project's default branch: `<merge-base(default, commit)>..<commit>` in the project repo, capped as a worktree's diff is. The default branch is found as deploys find it: the project's `default_branch`, else the remote's `HEAD`, else the branch the repo is on. Only a branch the waiting check shows (at its tip now), or a branch and commit an approval or a deploy result recorded, may be diffed. Any other branch or commit is refused, so the page cannot make Dots read an arbitrary ref.

**A send-back picks up where the target step left off** (the person's choice). The target step gets a new step run (`attempt` + 1). It is not a fresh start: its agent's last run is resumed, as an answer to `ask` is, with a message carrying the note ("The person sent this back: <note>"). It keeps the same session and works on the same worktree and branch, recreated if needed as the main spec's resumes do. For a delegate step, the delegating run is resumed. It decides whether to start members again, which are new runs counted against `total` and the budget. The steps after the target, up to the sender, then run again in order, each also resumed from its last run with a message naming the send-back and the new hand-off. A step whose last run cannot be resumed (its session gone) starts fresh from the target's branch, and the run page says so. The same applies to an agent's `send_back`.

### 5.5 Deploy

Done by Dots itself after a check, never by an agent. Fields: `mode`, and per mode:

| Mode | What Dots does, for each branch in the hand-off |
|---|---|
| `branch` | Nothing more: the branches are kept and listed as ready, with "Open folder" and "View diff" |
| `push` | `git push <remote> <branch>` from the project repo |
| `pr` | `push`, then `gh pr create` against the default branch, title from the task, body from the hand-off summary |
| `pr_merge` | `pr`, then `gh pr merge` with `merge_method` (`merge`, `squash` or `rebase`, default `squash`) |
| `member` | Starts a member as an agent step with a deploy job, e.g. "tag a release" (for apps whose deploy is more than a push). Its run's policy applies as usual |

A `push`, `pr` or `pr_merge` deploy whose check approved no branch fails with `Nothing to ship: the check approved no branch.`, before any scan or `gh` call, and the workflow waits as for any failed step (the person's decision, 2026-11-04). A `branch` deploy with no branch still succeeds, listing none. So does a Test run's deploy, which acts as `branch` (section 5.1). A `member` deploy with no approved branch starts from its usual start.

Git and `gh` run as the person, with their existing credentials. Dots never reads or stores those credentials. A missing `gh`, a rejected push or a failed merge fails the deploy step with the tool's message. Nothing already pushed is undone. The workflow run then fails, and the person can retry the deploy step from the run page.

### 5.6 Branch on outcome

Picks one of several sub-lists by the outcome label the previous agent step reported (`report_outcome`, section 7), or by the project. Fields: `on` (`outcome` | `project`), `paths` (label or project name to a list of steps), `otherwise` (a list of steps, maybe empty). The sub-lists show indented under the branch in the builder. After a path ends, the workflow goes on after the branch.

This is how "deployment depends on the outcome" is drawn. For example, branch on project: Dots goes to `pr`, the other app goes to `member` (a release step). Or branch on outcome: `fixed` goes to a check and deploy, `needs_info` goes to a step where the user helper asks the person.

### 5.7 Budget (on the workflow, not a step)

`max_runs` (default 10), `max_tokens` (default 1,500,000), `max_minutes` (default 120), `max_send_backs` (default 3). Every dot run with this workflow run's `workflow_step_run_id` counts, resume children included.

### 5.8 Validation on save

Refused, with a message naming the step: no trigger, or more than one; a step naming a dot that is not a member; a `may_start` naming a non-member; a deploy not preceded by a check on every path; a send-back target that is not earlier in the list; a branch on outcome whose labels no earlier agent step declares; an `at_once` above `total`; budget values of 0. A workflow that became invalid (a member deleted) shows the reason in the builder and refuses to start.

## 6. Hand-offs

A step's hand-off is JSON: `summary` (text), `branches` (list of `{run_id, branch, worktree_path, base_commit}`), `outcome` (label or null), and for a delegate step `members` (list of `{dot, sub_task, status, summary, branch}`).

The next agent step's prompt gets the hand-offs of every earlier step of this attempt, newest last, each under its step name. Summaries are cut to 4,000 characters each, with the full text one click away on the run page.

**Which branch a later agent step works on:** if the hand-offs before it carry exactly one branch, its worktree starts from that branch's tip on a new branch (`dots/<dot>/<run_id>`). Otherwise it starts from the project's default branch, and the branches are listed in its prompt. Merging several branches into one is the delegating agent's job when it wants one result. This is my call: Dots does not merge automatically.

### 6.1 Commits

Sandboxed dots ask before `git commit`, because commit hooks run code (`policy.rs`). If agents had to commit, every commit would be an approval, and work left uncommitted would be lost between steps (the next step starts from a branch tip) and never pushed. So, my call: **when a workflow run's dot run ends in worktree mode, Dots commits what it left uncommitted.** It runs `git add -A` and a commit with hooks off (`core.hooksPath` pointed at an empty folder in Dots' data, and `--no-verify`), as the person's git identity from their config. The message is `<step name>: <first line of the run's summary>`. Nothing is committed when the worktree is clean. A dot that commits itself (its policy allows it) is unaffected. This happens before the hand-off is built, so the hand-off's branches always hold all the work. In folder mode nothing is committed.

### 6.2 Which branches a check shows and a deploy pushes

My call: **the branches of the latest agent step before the check**, on the path taken. Earlier steps' branches are ancestors or stale. For a delegate step whose delegating run's own branch has commits, that branch first, then every member branch whose tip is not an ancestor of the lead's tip (`git merge-base --is-ancestor`), in the order the members started: the members' work the lead did not merge is not hidden (the person's decision, 2026-11-04). A member branch that is gone is left out; one git cannot judge is shown. Without lead commits, each member's branch. The rule is applied when the step's hand-off is built, so it also decides where the next agent step starts (section 6): a lead that left a member unmerged hands on two or more branches, and the next step starts from the project's default branch with them listed. The deploy acts on exactly the branches its check showed and approved, recorded with the decision (section 5.4). A later commit to any of them (there should be none) refuses the deploy with `The branch changed after it was approved.`

## 7. Workflow tools

Served like `notify`, `ask` and `follow_up`: built in on the local engine, tools of Dots' own `dots` MCP server on the Claude engine (added to `--allowedTools`). Each is checked against the dot's policy under its policy name. Each is given only to the runs that may use it.

| Tool | Policy name | Given to | Input | Result |
|---|---|---|---|---|
| `start_workflow` | `Workflow` | a chat run of a member that the trigger lets start it | `workflow` (name), `project` (name), `task` (1 to 8,000 characters) | "Workflow <name> started on <project>." The chat card appears (section 10) |
| `start_member` | `Workflow` | the run of a delegate step | `member` (dot name), `task` (1 to 8,000 characters) | "Started <member>." Refused with a reason past `at_once`, `total` or the budget, or for a member not in `may_start` |
| `report_outcome` | `Workflow` | runs of agent steps that declare `outcomes` | `outcome` (one of the declared labels) | "Outcome recorded." The last call wins |
| `send_back` | `Workflow` | runs of agent steps with `can_send_back_to` | `to_step` (step name), `note` | "Sent back to <step>." The run should then finish its turn; the send-back happens when it ends |

Every new and stored policy preset gets a `Workflow` rule (a code step after the migration, as `finish_0007` did for `Context`): Sandboxed and Custom ask, so each `start_workflow` call is an approval before the workflow starts, and ReadOnly and Trusted allow (the person's decision, 2026-10-24). A stored Sandboxed policy gets `ask`; a stored Custom policy gets nothing, as before. Under `ask`, `start_workflow` and `start_member` ask. `report_outcome` and `send_back` do not (the person's decision, 2026-11-04): an `ask` for them, from a rule or the policy's default, counts as allow. A policy cannot tell the person's own `Workflow: ask` from the one the preset gave (a rule carries no origin, and Custom starts from the Sandboxed rules), so the person's own `ask` counts as allow for these two as well. A dot's own `Workflow: deny`, or a default of deny, refuses all four tools. A rule of `ask` makes each `start_workflow` or `start_member` call an approval as usual, and the tool is still offered to the chat.

A refused call is a tool error naming the rule, in one sentence, and the run goes on, as with `follow_up`.

## 8. Runs and the runner

- **Workspace.** A run with `workflow_step_run_id` uses the workflow run's project repo in place of its dot's `workdir`. The dot's `workspace_mode` still applies: `worktree` (the expected setting for dev agents) makes a worktree of the project repo. `folder` works in the project repo itself, the person's own checkout, so the builder shows a warning on a member in folder mode (in the step's settings sheet, not on the card: `<name> works in the folder itself: in a workflow it changes the project's own checkout, and leaves no branch to check or deploy.`).
- **One running run per dot.** The rule stays for ordinary runs. Workflow runs are exempt: two copies of the dev agent run at once, each in its own worktree. A workflow run counts as one dot "busy" for the rest of the app (the column's status mark).
- **Concurrency.** `max_concurrent_runs` still caps every run at once, team or not. Started members beyond it queue. The builder's `at_once` is a ceiling, not a promise. The team page shows the current setting and links to Settings.
- **Order.** Queued workflow runs are dispatched FIFO with the rest. My call: no priority.
- **Restart.** The rules of the main spec apply to each dot run. A workflow run whose current step's runs were interrupted fails that step, and the workflow run goes to `waiting` with the error, so the person can retry the step or stop. A delegate run parked waiting for its members is resumed when they end, as now after a restart.
- **Cancellation.** Stopping a workflow run cancels its queued and running dot runs and closes its waiting check. Cancelling one dot run of a workflow fails its step.

## 9. Guards

- **Spend** is counted as dot runs start, as usage events arrive, and on the clock.
- **Tokens**, my call: new input plus output. The Claude engine's `Usage` today folds cache creation and cache reads into `tokens_in` (`engine/claude/parse.rs`). A Claude Code session re-reads its cached context every turn, so counting cache reads would stop almost every workflow at the 1,500,000 default. `EngineEvent::Usage` therefore gains a `cache_read` count, kept separately on the run (a new `runs.tokens_cache_read` column; `tokens_in` keeps its meaning so existing screens are unchanged). The budget counts `tokens_in - tokens_cache_read + tokens_out`. The run page shows cache reads beside the meter, not in it.
- **The clock**, my call: it counts only while the workflow run is not waiting on the person. It is paused while waiting on a check, a limit, an `ask` or an approval of any of its runs. So the workflow run keeps `active_ms`, updated as it enters and leaves waiting, not computed from `started_at`.
- **At a limit:** no new dot run of this workflow run is started (`start_member` and the next step are refused or held). Runs already running finish. The workflow run then goes to `waiting` with `waiting_on: budget` (or `send_backs`). It appears in Needs you with what was spent, the limit hit, and two actions: **Add budget** (raise that limit for this run by its default amount, once per click) or **Stop**.
- **Send-backs:** the `max_send_backs`-th + 1 send-back, from the person or an agent, is not performed. The workflow waits with `waiting_on: send_backs`, showing the note that would have been sent; the person can allow it once more or stop.
- **No agent tool changes a budget or a limit.** Only the person does, in the app.

## 10. UI

### 10.1 Teams in the rail

A **Teams** link in the rail, after Library. Routes: `/teams` (list), `/teams/:teamId` (team page), `/teams/:teamId/workflows/:workflowId` (builder), `/workflow-runs/:runId` (run page).

**Team page:** name and description; **Members** (add an existing dot, set its role, remove); **Projects** (name, repo folder picker, remote, default branch); **Workflows** (name, trigger, enabled switch, last run, "New workflow"); **Recent runs** (status, task, project, spend).

As built (plan C): the rail link is `Teams` (lucide `UsersRound`), after Library and before a publisher's Releases and Settings. The team page opens with a **Waiting** list while any of the team's runs waits on the person (the Inbox's rows, section 10.5), then Members, Projects, Workflows and Recent runs. The member picker offers only dots of the team's account and workspace. A workflow's row shows its trigger, `<n> run(s) in flight`, its problem, its last run, and the enabled switch, which saves at once. **New workflow** makes one turned off, with one agent step done by the first member, and opens it in the builder. **Run now** asks for the task and the project in a dialog and opens the new run's page; it is disabled while the workflow is off, has a problem, or the team has no project. Recent runs are the newest 20 (status, task, workflow, project, spent, started). **Delete team** asks first and says that runs in flight are stopped first.

### 10.2 The builder (layout B)

The chosen mockup: `.superpowers/brainstorm/3795-1792776045/content/builder-layout.html`, option B.

- A header with the workflow name, **Test run**, **Save**, and the enabled switch.
- The steps as cards in one column joined by arrows, numbered. The trigger card first, with an accent border. Delegate steps have a dashed accent border and say what they may start ("may start Dev agent · 3 at once · 6 total"). Checks use the needs-you colours. A branch shows its paths as indented sub-columns, each labelled.
- **＋ Add step** between any two cards and at the end, offering the kinds of section 5. Cards move up and down with buttons and the keyboard (a drag handle too, but not only drag). My call: no free placement.
- Clicking a card opens its settings in a side sheet (shadcn `Sheet`): the fields of its kind.
- A **Budget** card at the bottom: the four limits.
- Validation errors (section 5.8) show on the card they name, and Save stays disabled until they are fixed.
- No canvas or graph library: cards, lists and the existing shadcn kit.

As built (plan C):

- **The draft** lives in the page until **Save**, the enabled switch included (the team page's switch saves at once). Under the header, `1 run in flight uses the previous version` or `<n> runs in flight use the previous version` while `runs_in_flight` is above 0.
- **Live validation** sends the draft to the core 300 ms after an edit (`workflow_check_spec`), which answers with the first refusal a save would give and the step it names (`WorkflowProblem { step_id, message }`, by the same rules as section 5.8). The problem shows on that card, else on the header, and Save stays off. It does not see a name another workflow has, or a schedule that would never run: Save refuses those, and the refusal shows on the header.
- **Moves** stay within the step's own list: `Move <name> up` and `Move <name> down`, or Alt+ArrowUp and Alt+ArrowDown on a focused card. There is no drag handle, and no move into or out of a branch's path. A step is deleted at once (it is a draft); a branch that holds steps asks first.
- **Step ids** are `<kind>-<n>`, the smallest n no step of the whole tree has, made when the step is added.
- **The sheets.** Each step's sheet holds its kind's fields. The trigger card opens its own sheet: the kind, a chat's starters, a schedule (the dot editor's schedule editor, without None and without catch-up), its task and project, or a webhook's URL, its token (shown once, as a dot's), and the fallbacks. Outcomes can be added and removed but not renamed, since a rename would break a branch's label; a branch's path labels can be renamed.
- **Test run** saves a changed draft first, then asks for the task and the project. It works while the workflow is off (section 5.1).
- **Delete workflow** sits in the header and asks first. Its runs stay on the team page as runs of a deleted workflow.

### 10.3 The run page (option A of the second mockup)

`.superpowers/brainstorm/3795-1792776045/content/run-view.html`, option A.

- A header with the workflow, the task's first line, the project, and **Stop**.
- Budget meters: runs, tokens, minutes, send-backs, each "spent / limit".
- The definition's steps as in the builder, each with its status (waiting, running with the existing pulse, done, failed, skipped) and its attempt count when above 1. A delegate card opens to show one small card per started member (sub-task, status, commits). A card links to its dot runs' existing run screens.
- A waiting check shows its hand-off summary, **View diff** for each branch (the existing diff view), a note box, **Send back**, **Approve** and **Stop**.
- A finished deploy shows what it did: branches pushed, PR links, or the ready branches with **Open folder**.
- A failed step shows its error and **Retry step** (a new attempt, counted against the budget).

As built (plan C):

- **One Stop**, in the header, which also links the team and, for a run a chat started, `Open the chat`. A deleted workflow's run is titled `A deleted workflow`.
- **Meters** show cache reads beside the tokens meter (`· <n> cache reads`). A reached limit's bar takes the needs colour.
- **Links to dot runs** come from the core (`workflow_run_dot_runs`, section 11): each step card lists `Run of <dot>` with its status, one link per thread, and a delegate's members each link to their own run (`Open run`). A branch's paths show as columns, marked `Taken` or `Not taken` once it decided.
- **At a limit:** **Add budget** takes no amount. It raises each limit reached by its default, once per click, and the panel says what it will add. A send-back held at the cap shows its note and **Allow one more send-back**.
- **View diff** opens the diff in a dialog (section 5.4). A deploy's result lists each branch as `Ready`, `Pushed` or `Not pushed`, its pull request (a link only for an `https://` URL), whether it merges once its checks pass, and, for a ready branch, **Open folder** and **View diff**.

### 10.4 The chat card (option B of the second mockup)

When a chat run calls `start_workflow`, its conversation shows a **workflow card** after that turn, in the topic of the chat run, live:

- the team, the spend ("4 runs · 182k tokens");
- one line of step marks (✓ done, ● waiting on you, spinner running, ○ not reached);
- while a check waits: **View diff**, **Send back…**, **Approve**; at a limit: **Add budget**, **Stop**;
- always **Open run**.

The card is part of the transcript, so reopening the conversation later shows its last state. My call: when the workflow run ends, the user helper is **not** woken up automatically. The card shows the result, and the person can ask the helper about it.

As built (plan C): the card is shown in the home conversation only, inside the section of the run that started it, after that run's words and check-ins; the run screen shows none. One query reads the cards of every run the conversation shows (`workflow_runs_for_origins`), and `workflow_run_updated` keeps them live. The step line follows the path a branch took, and marks a failed step ✕ and a stopped or skipped one –. A waiting check shows the run page's whole check panel (the summary, each branch with View diff, the note, Send back, and Approve with the tips it showed). A failed step shows its error and **Retry step**. **Stop** shows once while the run is in flight. The card's actions act on the workflow only: they do not wake the chat's dot. Delegate members are not on the card; the run page shows them.

### 10.5 Needs you and notifications

- A workflow run waiting on the person (a check, the budget, the send-back cap, or a failed step: plan A's `step_failed`) is listed in the Inbox under **Workflows** and counts in the Inbox badge, whatever started it (the person's decision, 2026-11-04). A run a chat started also counts in **needs you** for the dot whose chat started it; once that chat run is deleted, it counts in the Inbox only. Either way it is listed on the team page, under **Waiting**.
- An Inbox row shows the task's first line, the team (a link to its page), the workflow (`A deleted workflow` once it is gone), ` · at <step>`, when it started and what it waits on (`Needs your check`, `Hit its budget`, `Hit its send-back limit`, `A step failed`), and **Open the run**. The section stands after Apps to review and before New instructions.
- Notifications are sent when a run's state changes, once per change (a wait on a new step run is a change):

  | When | Title | Body |
  |---|---|---|
  | A check waits | `<team> needs your check` | `<step name>: <task's first line>` |
  | The budget is reached | `<team> hit its budget` | the limit, e.g. `runs, 10 of 10` |
  | The send-back cap holds one | `<team> hit its send-back limit` | `send-backs, <n> of <max>` |
  | A step failed | `<team> failed` | the run's error, `<step name>: <error>` |
  | The run failed, without waiting on a failed step | `<team> failed` | the task's first line |
  | The run succeeded | `<team> finished` | the task's first line |

  A Test run's title ends with ` (test)`. A cancelled run is not told: the person stopped it. A click opens `/workflow-runs/<id>`. A run a chat started is quiet while the person reads that dot's conversation, where its card is live.
- **Step runs are quiet** (the person's decision, 2026-11-04): a dot run of a workflow step gets no "finished" or "failed" notice and is not listed in Runs to review. Its questions and approvals notify and count as before.

## 11. Commands and events

Thin Tauri commands over new `Runtime` methods, in a new core module (`teams.rs`, with `store/teams.rs` and `store/workflows.rs`). Teams, members and projects: create, update, delete, list. Workflows: create, update (validates), delete, list, get. Workflow runs: start (manual), list (by team, by status), get (with step runs and hand-offs), decide check (approve, send back with note), add budget, allow send-back, retry step, stop.

Runtime events, forwarded to the UI as today: `team_updated`, `workflow_updated`, `workflow_run_updated` (status, current step, spend, waiting state; the run page, chat card, Needs you and Inbox refresh from it). The contract fixture gains the new types and events.

As built (plan C), the commands are: `teams_list`, `team_get`, `team_create`, `team_update`, `team_delete` (stops the team's runs in flight first), `project_add`, `project_update`, `project_delete`, `open_project_folder` (a project's repo, or a ready branch's worktree, found by id, never a path from the UI), `workflows_list`, `workflow_get`, `workflow_create`, `workflow_update`, `workflow_delete`, `workflow_check_spec` (the builder's live validation), `workflow_webhook_url` (refused while the local server is not running, as a dot's webhook URL is), `regenerate_workflow_token`, `workflow_runs_list` (by team, workflow, statuses and a limit, newest first), `workflow_runs_for_origins` (the runs the given chat runs started, oldest first: the chat card), `workflow_run_get`, `workflow_run_dot_runs` (every dot run of the run's step runs, oldest first, each with its thread's root, its dot's name and its status), `workflow_run_start` (Run now and Test run, trigger `manual`), `workflow_check`, `workflow_branch_diff`, `workflow_approve` (with the tips the check showed), `workflow_send_back`, `workflow_add_budget`, `workflow_allow_send_back`, `workflow_retry_step` and `workflow_stop`. Stop needs the `RunDots` permission, as Add budget and Retry step do. A `Workflow` also says how many of its runs are in flight (`runs_in_flight`) and which started last (`last_run`).

Events, as built: `team_updated` is also sent for each team of a dot that is renamed or deleted. `workflow_updated` is sent for each workflow of a team whose set of members changes (its problem may change), of each team of a deleted dot, and for each workflow a team's deletion removes. No event is sent for a workflow when one of its runs starts or ends: the UI reads its workflows again on `workflow_run_updated`. A `run_updated` for a dot run of a step marks the run pages' dot-run links stale.

## 12. The app library, later

The app library (parallel session) gives Dots an Asana-like project manager that agents read specs from and write progress to. When both exist:

- The team's **projects** come from the PM app's projects, each recording its repo. The `team_projects` table becomes a fallback or goes away.
- `start_workflow` takes a PM task in place of free text. The workflow run links to it, and each step's progress is written to it.
- The task manager reads and updates tasks through the PM app's tools, like any other app tool.

Nothing in this spec should block that. Keep project lookup behind one function (`project for a workflow run`) and the task text behind one field, so the swap touches those two places.

## 13. Edge cases

- **Editing a workflow** while runs are in flight: the runs keep their copied definition. The builder says "N runs in flight use the previous version."
- **A member's dot is disabled:** its steps queue no runs. The step fails with "<dot> is disabled", and the workflow waits so the person can enable it and retry.
- **The project repo is gone or not a repo** at start: `start_workflow` is refused with the reason. If it happens mid-run, the step fails.
- **Two workflow runs on one project:** allowed. Each run's worktrees are separate. Two deploys may push different branches to the same remote.
- **A chat run that called `start_workflow` is deleted:** the workflow run goes on. It stays in the Inbox and no longer counts in a dot's needs you (section 10.5).
- **Schedules paused:** scheduled workflow triggers do not fire. Running workflow runs go on.
- **A dev agent asks the person (`ask`)** inside a workflow: it works as today, and the workflow's step waits on that run. The question shows in the dev agent's conversation and in Needs you as usual.

## 14. Testing

- **Core:** definition validation (every rule in 5.8); hand-off building and the branch-choice rule (section 6); each step kind's state changes; delegate limits (`at_once`, `total`, `may_start`); resume of the delegating run when its members end; send-back attempts and the cap; every budget limit and its waiting state; add budget and allow send-back; the workspace override and the one-per-dot exemption; restart in the middle of each step kind; cancel cascades; the `Workflow` policy rule added by the upgrade step. Deploy against a local bare repo as the remote (push), and a fake `gh` on PATH (PR, merge, failures), with no network.
- **Engines:** the four tools on the fake Claude CLI and on the scripted local engine, including refusals.
- **App:** commands and contract; notification wording and routes.
- **UI:** the builder (add, move, edit, delete steps; branches; validation messages; save), the run page (each status; the check actions; meters), the chat card (each state), the Teams screens, Needs you counts.
- **Smoke** (debug build, temp `DOTS_DATA_DIR`, fake Claude CLI): build the Dots management team and its workflow in the UI against a scratch repo with a local bare remote; chat a request to the user helper; watch the card; approve; see the branch pushed to the bare remote. Then a budget stop, and a send-back from the check. A real-CLI run is the person's to do.

## 15. Out of scope

Integration with the app library (section 12, later); a free canvas or lanes; members on other machines; teams calling other teams; parallel paths other than a delegate step's members; automatic merging of several branches; recurring triggers inside a workflow; per-member budgets; an agent editing workflows; templates or sharing of workflows.

## 16. Left to the builder

- Module boundaries inside the core, beyond the names in section 11.
- The JSON shape of the definition, as long as it carries section 5's fields and stays valid through renames of dots (store dot ids, show names).
- How spend is aggregated (queried from `runs` or kept on the workflow run as events arrive), as long as the limits act before a new run starts.
- The exact prompt wording of the `## Workflow` section and the tool descriptions, in the style of `checkins.rs`.
- Visual detail within Deep Teal and the chosen mockups.

## 17. What plans B and C built

### 17.1 Plan B

Plan B (`docs/superpowers/plans/2026-11-02-teams-b.md`) built the core of sections 5.1 (`schedule`, `webhook`), 5.3 to 5.6, 6.2, 7 and 9: delegate steps and `start_member`, send-backs and their cap, `report_outcome` and `send_back`, deploys, branch steps, and the two triggers, with plan A's parked findings. Its "Deviations from the spec" table holds, except where this list says otherwise. The plan's text was not changed while it ran, so its code blocks for Task 7 still show the first versions of the scan, the gh calls and the refusal.

- **What a deploy refuses** is every object the push would send, not the files changed since the branch's base. The scan lists `git rev-list --objects <commit> --not <ids>`, where the ids are what `git ls-remote` reports for the push URL (not the local tracking refs, which an agent can write), with `GIT_NO_REPLACE_OBJECTS=1`, and reads each object's name (its full path) and size. So a secret added and then deleted on the branch is refused, and a first push to an empty remote scans the whole history. A size that cannot be read refuses. The refusal is `Not pushed: <branch> has commits that hold files that look like secrets or are over 10 MB: <files>. Rewrite the branch so none of the commits it would push holds them, then retry the step.`
- **The pull request a `pr` deploy reuses** is found with `gh pr list --head=<branch> --base=<base> --state=open --json=url,headRefName,baseRefName,isCrossRepository`, parsed in Dots: only an open PR of that head into that base, not from a fork. `gh pr create` takes `--head=`, `--base=`, `--title=` and `--body-file=-`, with the body on stdin. A remote URL's user and password are taken out of what is shown.
- **A `member` deploy** starts from the approved commit, not the branch's tip, and checks every approved tip first, as the other modes do. Run again after a send-back, it starts from the approved commit again. Its prompt shows `#### Approved` with `Branches: <branch> (commit <sha>)`. With several approved branches it starts from its usual start, and the prompt lists each branch at its commit. The base is looked up only for `pr` and `pr_merge`.
- **A failed deploy keeps what it did:** the failed step run's `deploy_result` lists each branch, pushed or not, its PR and its auto-merge, and the error (`error`, absent on success). A deploy that failed before it could read the approval records no branches; an `interrupted` one records no result.
- **What a check shows** is the latest earlier hand-off; when that one has no branch (a step that changed nothing), it shows the branches of the newest earlier hand-off that has some, and a deploy after it ships them. A step's start branch uses the same rule.
- **A step with outcomes, run again after a send-back,** is told so: ``Report the outcome again with `<tool>`, one of: "a", "b": the earlier one no longer counts.`` follows the send-back or rerun message.
- **`send_back` to a name several targets share** (compared without case) is refused: `"<to>" names more than one step you can send work back to: "a", "b". Nothing was recorded.`
- **A step run cut off by a restart** before the workflow run moved to it (`Running`, no runs) is used again by the next pass, not left running beside a new attempt: agent steps, branches and deploys. A check's own step run is not.
- **A Sandboxed step asks** before `report_outcome`, `send_back` and `start_member`, all under `Workflow`, as the plan's open row said. Plan C changed this: `report_outcome` and `send_back` no longer ask (section 7).
- **Cancelling one member of a delegate step** reports it to the lead with its status, and the step goes on (section 5.3 read as the rule for members; section 8's "fails its step" as the step's own run).

**Open questions for the person**, answered on 2026-11-04 and built by plan C (17.2): whether `report_outcome` and `send_back` should not ask under Sandboxed; whether a lead's own commits should no longer hide its members' branches at a check (6.2: an ancestor rule); whether the webhook should cap the runs in flight per workflow (an answer 429).

### 17.2 Plan C

Plan C (`docs/superpowers/plans/2026-11-04-teams-c.md`) built sections 10 and 11: the Teams screens, the builder and its sheets, the run page, the chat card, the Inbox's Workflows section and the notifications, over new app commands, with the person's answers to plan B's open questions and three more of their decisions (a deploy with no branch fails, the Inbox lists every waiting run, step runs are quiet), all of 2026-11-04. The sections above now say what it built where it differs from or adds to their first text; in short:

- **Workflow tools under Sandboxed and Custom** (section 7): `start_workflow` and `start_member` ask; `report_outcome` and `send_back` do not. The check is in `Policy::resolve_in`, so both engines get it from one place, and the tool is told apart by its input as approvals tell it. A deviation: the person's own `Workflow: ask`, or a default of ask, counts as allow for those two, since a policy cannot tell it from the preset's. A deny still refuses all four.
- **The ancestor rule** (section 6.2) is applied when the hand-off is built, so the check, the approval, the deploy and the next step's start branch all read one list.
- **A deploy with no branch** (section 5.5) fails a `push`, `pr` or `pr_merge` with `Nothing to ship: the check approved no branch.`; the run's error is `<step name>: Nothing to ship: the check approved no branch.` and it waits on `step_failed`.
- **A check is read before it is approved** (section 5.4): `Workflows::check` gives `CheckView { step_id, summary, branches: [ShownBranch { branch, run_id, commit }] }`, and `approve_check(id, note, tips)` refuses tips that are not git's now. A branch's diff is from the project's default branch (`Workflows::branch_diff`, `workspace::range_diff`).
- **The webhook's cap** (section 5.1) is 3 runs in flight per workflow, answered `429` `too many runs in flight`.
- **A Test run starts while its workflow is off** (section 5.1, the controller's call after the builder was built), so a new workflow, which is made off, can be tried before it is armed. Only a `manual` start with `test` passes; a `test` flag on another trigger does not.
- **Waiting runs** (section 10.5) are listed in the Inbox and counted in its badge (core `inbox_count` and the rail), and in needs you for the dot whose chat started them. Step runs send no "finished" or "failed" notice and are left out of Runs to review.
- **A step's dot runs** come from one store query (`Workflows::dot_runs`, `StepDotRun { step_run_id, run_id, root_run_id, dot_id, dot_name, status, started_at }`), not from the newest dot runs.
- **Workflow notices** are decided on `workflow_run_updated` only, by the run's status, its wait and the current step's newest step run, so a repeat is quiet. A run first seen waiting or ended is told, since the core publishes a workflow run only when it moves or the person acts on it.

### 17.3 Known gaps

- A rerun message names the latest hand-off's branches, so a later step that kept its branch gets no merge line when the step just before it changed nothing (plan B).
- A delegate step whose second attempt changed nothing still hides its first attempt's member branches, so with no earlier branches the check shows none. A `push`, `pr` or `pr_merge` deploy after it now fails with `Nothing to ship: the check approved no branch.` instead of succeeding having pushed nothing, but the branches stay hidden.
- A `member` deploy refused by its tip check records no `deploy_result` (plan B).
- The Claude-engine test of tools a step was not offered sees an empty result for the server's `unknown tool` error: fake-claude drops RPC errors (plan B).
- The webhook cap's lock is one for the whole process, so it serialises the webhook starts of every workflow, not of one. Manual, chat and schedule starts are not capped and can add a run between the count and the start. A `429` has no `Retry-After`.
- The person's own `Workflow: ask` cannot make `report_outcome` or `send_back` ask (section 7).
- Live validation does not see a name another workflow has, or a schedule that would never run; only Save refuses them.
- The rail's Inbox badge keeps its muted tone for waiting workflow runs, a check included; only approvals, app reviews and open questions give it the needs tone.
- An Inbox row of a waiting workflow run has no "Open the chat" link (the item carries the origin dot, not the origin run); the run page has one.
- The notifier keeps every workflow run that has not ended and the newest 256 ended ones (`ENDED_WORKFLOW_RUNS_KEPT`); past that the oldest ended run is dropped, so a run published again after 256 newer runs ended is told again.
- A dot's `fresh` count (new since seen, part of its unread mark) still counts its ended step runs; only Runs to review and the notices leave them out.
- The chat card shows no delegate members and no links to a step's dot runs; the run page has both. On the run page, an approved check's card still lists the delegate members of the hand-off it showed.
- The builder has no drag handle and no move into or out of a branch's path, and nothing guards leaving it with an unsaved draft. A schedule switched inside the schedule editor from `once` to another kind can carry `catch_up: true`, which the core ignores. Field hints are not linked to their controls (`aria-describedby`). A disabled button's title, which says why Run now or Test run is off, does not show on hover.
- A branch's diff is read whole from git before it is capped, as a worktree's diff is.
- `workflow_runs` has no index on `origin_run_id` or `workflow_id`.
- Under load, multi-step tests in `workflows_test` can hit their 60 s deadline; they pass alone.
