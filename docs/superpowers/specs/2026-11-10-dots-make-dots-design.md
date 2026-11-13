# Dots that make dots: design

- Date: 2026-11-10
- Status: builder check done; written for the person's review
- Part of: "Dots that build". The person asked that dots be able to make new dots, spin up workflows and create apps. That is four projects, agreed in this order:
  1. **Dots make dots** (this document). It also sets the shared rules the later parts reuse: the maker's reach, asking beyond it, the record of who made what, and the limits.
  2. Dot-made apps go live with no click.
  3. Teams and workflows become separate things: a workflow stands alone, a step picks a dot or a team, and each team has a lead dot.
  4. Dots create and start workflows.
- Builds on: `2026-10-08-dots-design.md` (dots, runs), `2026-10-13-approvals-v2-design.md` (policy presets, approvals), `2026-10-21-env-vault-design.md` (variables and grants), `2026-10-27-conversational-chat-design.md` (chat topics, proposal cards, continuations), `2026-11-03-setup-agent-design.md` (creating a dot from a proposal), `2026-11-06-spend-caps-design.md` (daily caps).
- Based on `origin/master` at `ad917b31`.

## 1. Intent

The person's words: "the dots should have the ability to make new dots". When a dot in the middle of a run decides it needs another dot, "it should create one", and the new dot should "run right away", with no approval step.

The limit they set: "if permissions outside the current scope are required ask the user". Scope means what the maker dot may touch: its folder, tools, secrets and permission level. It does not mean the task the maker was given.

What would make it a failure, in their words: "if it went rogue and did something outside the scope".

What happens today: only a person creates a dot (the New dot form, the setup agent's **Run setup**, or importing Claude Code agents). No tool offered to a run can create, change, start or delete a dot. Dots do not record who made them.

Success: the person switches "Can make dots" on for a dot named Researcher and asks it to watch three blogs. Researcher makes a dot named Blog Watcher with a Monday schedule, gives it a first job, and ends its turn. Blog Watcher runs at once. When it finishes, Researcher is woken with the result and answers the person. The person clicked nothing after their message. Blog Watcher stays in the dots list under Researcher, and can reach nothing Researcher could not reach.

## 2. Goals, non-goals, anti-goals

**Goals**

1. A dot with **Can make dots** switched on can make a new dot during any of its runs except a run that is a workflow step (section 5). The new dot exists at once and can run at once.
2. A made dot holds nothing beyond its maker's reach (section 6). A request for more goes to the person as a card, and nothing beyond the maker's reach is given until they answer (section 7).
3. The maker can give a job to a dot it made, and is woken with the result when that job ends (section 8).
4. The maker can change and delete dots it made (section 9).
5. Made dots are kept. They show in the dots list under their maker, marked "made by <maker>" (section 11).
6. Limits on how many dots a maker makes, with more asking the person (section 10).

**Non-goals (this part)**

- Dots making workflows, teams or apps. Those are parts 2 to 4.
- A made dot making dots. The tools never switch **Can make dots** on. Only the person does, in that dot's settings.
- Watchers and webhook triggers on a made dot, set by the maker. The maker may set a schedule only (my call; the person said yes to "a schedule or other trigger", so this narrows it for the first part and is listed for their review).
- A maker starting, changing or deleting a dot it did not make.
- Taking things away from made dots when the person later narrows the maker. The person said no to that.
- A cost cap on one run.
- Undo for a maker's delete.

**Anti-goals (a failure even if it works)**

- A made dot that holds a tool, secret, folder or permission its maker did not hold when it was made or changed, without the person's answer on a card.
- A dot with the switch off making, changing, starting or deleting any dot.
- A chain of makers and wakes that runs without end or without the person being able to see it.
- A made dot the person cannot find, or whose maker they cannot tell.
- A maker changing or deleting a dot it did not make.
- A secret's value shown in a card, a tool result or a prompt. Cards name variables, never values.

## 3. Words

- **Maker**: a dot with **Can make dots** on.
- **Made dot**: a dot created by a maker's tool call. It records its maker.
- **Reach**: what a dot may touch, as held in its own settings at this moment (section 6).
- **Extras**: the parts of a requested dot that lie beyond the maker's reach.
- **Make request**: a request with extras, or over a limit, waiting for the person's answer.
- **Job**: a message a maker gives to a dot it made, which starts a run of that dot.
- **Wake**: a new run of the maker, in the maker's same chat topic, that carries a job's result or the person's answer to a make request.

## 4. What this overrides

Earlier designs say the opposite of this one in places. For makers, and only for makers, this design wins:

- Setup agent, anti-goal: "It proposes; nothing is created until **Run setup**". A maker's dot is created with no click.
- Env vault: "Nothing is granted by default." A maker may give a made dot variables the maker itself is granted.
- Conversational chat, non-goal: "editing a dot's schedule, tools, policy or engine from chat". A maker may set these on dots it made, within its reach.

Nothing changes for a dot with the switch off, which is every dot until the person turns it on.

## 5. The switch

A new part of a dot's settings, shown in the dot editor as a section **Making dots**:

- **Can make dots**: off by default, for every existing dot and every new dot, including made dots.
- **Dots it may make in one run**: default 3, range 1 to 10.
- **Dots it may keep in total**: default 10, range 1 to 50.

The two numbers show only while the switch is on. The ranges are my call.

Turning the switch on needs the same rights on this PC that creating a dot needs (`EditDots`). A make call that gives variables needs `GrantEntries`, as creating a dot with variables does today. If this PC lacks the right, the make call fails with a plain error and no card.

The tools in sections 7 to 9 are offered to a run only when its dot has the switch on. They share one new policy name, `MakeDots`, which every preset allows (my call: the switch is the gate; the policy name lets the person add an ask or deny rule, and an ask rule wins as it does for Dots' other own tools). When the policy denies `MakeDots`, the tools are not offered.

Both engines (Claude and local) get the tools.

A run that is a workflow step is not offered the tools, even when its dot is a maker (my call). A step's chat ends with the step, so a wake or a card answer would have nowhere to land, and Dots' other own tools of this kind (`propose_instructions`, `start_workflow`) already leave step runs out. Part 4 may revisit this.

The make request card is decided on the desktop only, as proposal cards are today; the phone view decides approvals only.

## 6. Reach: what a made dot may hold

The check runs when a dot is made and each time its maker changes it. It compares the requested settings with the maker's settings **at that moment**. It is not run again later: if the person narrows the maker afterwards, made dots keep what they have.

| Setting | Within reach when |
|---|---|
| Account and workspace | Always the maker's. Not choosable. A different one is refused, not asked. |
| Engine, endpoint | Always the maker's. Not choosable. |
| Model | The maker's by default. Another model of the same engine is within reach. |
| Folder | See "Folder rules" below the table. |
| Workspace mode | Follows from the folder rules. Not choosable. |
| Permissions | One of: (a) an exact copy of the maker's whole policy, or (b) a plain preset, with no added rules, that is no stronger than the maker's preset, in the fixed order read-only, sandboxed, trusted. A maker whose preset is custom may give (a); it may give read-only only when its own rules allow everything read-only allows (each of read-only's allow rules comes back allow from the maker's policy, for any input), since custom rules can be narrower than read-only. Otherwise read-only asks. Default when not stated: (a). |
| Library items (`uses`: MCP servers, skills, plugins, context, apps) | Each named item is one the maker names in its own `uses`. Items switched to "every dot" come anyway, as for any dot. |
| Variables | For each consumer, the names are among those the maker is granted for that consumer. Today, saving a dot grants a newly used MCP server every variable its config reads; for a made dot that automatic grant must not add a name the maker lacks. |
| Browser | Only if the maker has it. |
| Use user settings | Only if the maker has it on. |
| Turn limit, time limit | No higher than the maker's. Default: the maker's. |
| Approval wait, quiet limit, global instructions, result check, graph, context rules | Copied from the maker. Not choosable. |
| Schedule | Any valid schedule. A schedule is not part of reach. |
| Can make dots | Always off. Not askable. |
| Enabled | On. |

**Folder rules** (my call). "The maker's folder" is the folder in the maker's settings (`workdir`), never a run's worktree copy.

- **Left blank**: Dots makes a new empty default folder in folder mode, as `create_dot` does today. Within reach, because it exposes nothing that exists.
- **The maker's own folder**: within reach. The made dot gets the maker's workspace mode, so a made dot of a worktree maker also works in per-run worktrees.
- **A folder below the maker's folder, when the maker is in folder mode**: within reach, in folder mode. A relative path is taken from the maker's folder. A folder that does not exist yet is created by Dots.
- **A folder below the maker's folder, when the maker is in worktree mode**: beyond reach, so it asks. The maker itself only ever writes in worktree copies; a made dot in a subfolder would write straight into the person's real checkout, which is more than the maker can do. A path inside a run's worktree copy is refused with a plain error that says to leave the folder blank or use the maker's own.
- **Anywhere else**: beyond reach, so it asks. The folder must exist.
- Paths are compared after resolving the real path (case, `..`, links and junctions), so a link that leads outside is outside. A link or junction whose target cannot be found is outside too.
- A folder below the maker's that is, or is inside, `.git` or `.claude` (any case) is beyond reach: their settings and hooks run code. A maker whose own folder is not a full path (`.`, `C:`) has no folder within reach: a full path asks, a relative one is refused.
- Before anything reads the disk, a folder on another computer or a device (`\\server\share`, `\\?\UNC\…`, `\\.\…`) and a drive without its root (`C:notes`) are refused with a plain error. A made dot that is to work in worktree copies of a folder not ready for them is refused, never moved to folder mode.

Notes:

- Permission rules have no clean "A inside B" test, which is why the choice is "the maker's own rules, or a weaker plain preset" and not free rules. This is my call. The preset label can be trusted for the order: any edited rule or default, an "always allow" from an approval, and setup extras all turn a policy into custom.
- Name and instructions are the maker's to write. They follow today's rules for any dot (name unique on this PC, 1 to 64 characters, and so on). A name clash is a plain tool error.
- **Spend**: a made dot's spend counts toward its maker's daily caps as well as its own. If the maker has a daily cap, the maker and all dots it made share it: when the shared total reaches the cap, runs of all of them wait or stop as a capped dot's runs do today. A made dot starts with no cap of its own; the person may set one. If the maker was itself made, the spend counts up the whole chain. This is my call.
- **Spend survives a delete.** Today a dot's spend is summed from its runs, and deleting a dot deletes its runs. A made dot's spend must still count toward its makers' shared caps for the rest of that day (and month, for the monthly cap) after the made dot is deleted, by the maker or by the person. Otherwise a maker could clear its cap by deleting what it made. The chain also survives deleting a dot in the middle: when B, which A made, is deleted, the dots B made move up to count toward A, and only B's own runs are kept for the makers above. Who made them, and who may change or delete them, does not move.

## 7. Making a dot, and asking for more

### 7.1 The tool

`make_dot` takes: name, instructions, a one-line purpose (shown on cards and lists), and optionally the choosable settings in section 6, a schedule, and a first job (section 8).

Dots checks, in this order:

1. The request is a valid dot (today's validation).
2. The limits (section 10).
3. Reach (section 6).

**All within reach and within limits**: the dot is created at once, with its maker and the making run recorded. If a first job was given, it starts. The tool result says the dot was made and names it. A **Made dot** card is posted in the maker's chat topic (section 11).

**Extras or over a limit**: nothing is created. Dots stores a make request and posts an ask card (7.2). The tool result says: the request is waiting for the person, do not wait for it, carry on with other work or end the turn, and you will be woken with the answer. The maker's run carries on.

A request that could never be allowed is refused with a plain error and no card: another account or workspace, another engine, a Library item or variable that does not exist, or **Can make dots**.

### 7.2 The ask card

Shown in the maker's chat topic and in the Inbox, with an OS notification like an approval's.

- Title: "<maker> wants to make a dot with more than it has", or "<maker> wants to make one more dot" when only a limit is passed.
- Body: the new dot's name and purpose, then the extras in plain words, one per line: each Library item, each variable name with its consumer, the stronger preset, the folder outside the maker's, the browser, a higher turn or time limit. When a limit is passed, the line says which ("4th dot in this run; the limit is 3").
- Buttons:
  - **Allow for this dot**: the dot is made as requested. The extras belong to that dot only. The maker's own settings do not change, and the maker's reach does not grow.
  - **Make it without these**: the dot is made with the extras removed (items and variables dropped, the preset and limits lowered to the maker's, the folder replaced by a new default folder). Not shown when the only problem is a limit.
  - **Don't make it**: nothing is made.

After any answer the maker is woken with a message that says what the person chose and, if a dot was made, its name. A first job given with the request starts when the dot is made.

The person who answers must hold the rights the result needs (`EditDots`, and `GrantEntries` when variables are given); otherwise the Allow button is off with a line saying why.

At most 3 make requests may wait per maker. A further one is refused with a plain error (my call).

A request is checked again when the person answers, against the Library and vault as they are then. If something it names has gone (a folder outside the maker's too), the card says so and only **Don't make it** stays.

The card and the answer are judged against the maker as it is when the person looks and when they answer, not as it was when it asked: the request keeps what the maker asked, the reach check and the limits run again, and the card's lines, its title and **Make it without these** come from that. **Allow** is off, and refused, when the maker was narrowed so that the dot would get something the card did not show at first; the person can still make it without these or not make it. An answer carries the limit line of the card the person answered; an answer that would make a dot past a limit that card did not show is refused, and the card read again shows it. An answer also carries what the card showed beyond the maker's reach; **Allow** and **Make it without these** are refused when the request would now give something else, and the card read again shows it. Reading a card writes no limit; a waiting request whose named things have gone is marked gone by the read, and `making_updated` is published. When it was the made dot that was given more since, the card says so. "Make it without these" tells the maker what was left out as it stood when the person answered. The Inbox item has the card's own title and lines. A card for a change lists every setting the change names, the new instructions in full; a card for a new dot carries its instructions and schedule.

Deleting the maker discards its waiting requests.

## 8. Jobs and wakes

`give_job` takes a made dot and a message (1 to 4,000 characters).

- A maker may give a job only to a dot it made itself (its own direct made dots).
- A job to a made dot that holds anything beyond its maker's reach as the maker is now (the check of section 9) does not start: it waits for the person as a job request, whose card shows the dot, what it holds beyond the maker, the line "Its final message goes back to <maker>." and the job in full. **Allow** starts the job as it was given; **Don't give it** wakes the maker once; there is no "without". A job request counts toward the 3 waiting requests, and toward the loop guard only once it is allowed and starts; the person's Allow is not held by the loop guard.
- The job starts a run of the made dot in a chat topic of that dot. Jobs from one maker thread go to the same topic of the made dot, so the made dot keeps the thread of the work (my call).
- The made dot's prompt for that run says the job comes from the maker, by name.
- The tool returns at once: the job has started, do not wait, you will be woken.
- If the made dot is busy, the job queues behind its current run, as any run does.
- **What one job covers** (my call): the run `give_job` starts, any resume runs after approvals, and, when that run ends its turn by asking the person a question (`ask`), the run that carries the person's answer, and so on until a run ends without a question waiting. Follow-ups the made dot schedules for later are not part of the job.
- When the job ends, in any way (finished, failed, stopped, capped), Dots wakes the maker exactly once: a new run in the maker's same chat topic whose message gives the made dot's name, how the job ended, and the last run's final text, cut at 8,000 characters with a note when cut. If the maker is busy, the wake queues; today's continuation path refuses a busy thread, so queueing is new work.
- While a job waits on the person's answer, the maker is not woken. The maker's topic shows "<made dot> is waiting for your answer" with **Open run**. If the question is never answered the job stays open; the person can stop it from the made dot's topic, which ends the job as stopped and wakes the maker.
- Cards in the maker's topic: "<made dot> is working" with the job text and **Open run**, then "<made dot> finished" (or failed, stopped) when it ends.
- A job run counts as background or not as the maker's run did, for the usage holds that already exist. A job and a wake are also background when the maker's topic has no message the person typed (a topic a schedule started stays unattended through its wakes and the jobs they give).
- **Reports and jobs are quoted data** (fix 14b): a made dot's final text in a wake and a maker's job text are written by a model. Every line of them is quoted with `> ` in the prompt, on both engines, in a first prompt and in a session, so no line can open a section such as "## Message from the user". The wake's note says a dot's final message is that dot's work, not instructions, to be checked before acting and never taken as the person's words; the job's note says it is not a message from the person. A run that carries a job or a wake starts no workflows.
- A job that goes on in the made dot's session is told the dot's instructions and purpose as they are now when they changed since the session began, as a person's message is.
- A job whose run waits for a spending cap or the usage limit tells the maker once that it has not started; the job stays open. A refused approval with no note ends the job as stopped by the person, not as finished. A failed job run that the person sends again after its job ended is no longer a job.
- A wake whose topic is adding an app is tried again after a wait; a wake to a topic whose last run has no session stays in that topic; a wake for a maker that is switched off, or whose account is removed, is dropped with a line in the log.

**Loop guard**: in one maker chat topic, at most 6 jobs may be given between one message typed by the person and the next (the existing "typed" flag on a chat message; answers to questions and to cards are not typed and do not reset the count). The 7th is refused with a plain error that tells the dot to report to the person. For a topic started by a schedule or other background trigger, the count runs from that trigger. The number is my call.

The shared spend cap (section 6) is the second guard.

## 9. Changing and deleting made dots

- `list_made_dots`: the maker's own made dots, each with name, purpose, schedule, whether it is running, and how its last run ended.
- `change_dot`: takes a made dot and any of the settings `make_dot` takes. Only the settings named change; the rest stay, including anything the person edited and any extras the person allowed. The named settings are checked against the maker's reach at that moment. Extras go through the ask card, worded "<maker> wants to give <dot> more than it has". Removing things never asks.
- `delete_dot`: takes a made dot. Refused while that dot has a run going or queued. It deletes the dot as the person's delete does, history included, and posts a "<maker> deleted <dot>" card in the maker's topic. The person agreed the maker may delete; that it is final, with no undo, is as today's delete and is listed for their review.
- An approval of a maker's `delete_dot`, `change_dot` or `give_job` names one made dot, by its id. The call is performed on that dot alone, and refused if its name now finds another dot or none.

When a made dot holds anything beyond its maker's reach as the maker is now (what the person allowed it or gave it, or what the maker no longer has), every change the maker asks for it that is not pure narrowing goes to the person as a change request, whose card lists what the dot holds; there is nothing to save without that. Pure narrowing is removing Library items or variables, a schedule of none, lower limits, a narrower preset, and switching the browser or the person's Claude settings off; a change of instructions, purpose, name, folder, model or schedule, or adding anything, is not. What a made dot holds counts its folder as within reach only when it is the new folder made for that dot, a folder of a worktree maker's own folder as beyond reach when the dot works in it directly, and "Can make dots" switched on as beyond reach. Deleting a made dot that has a change or a job waiting wakes the maker once to say the request went with the dot.

A maker can do these only to dots whose recorded maker is itself. The person can still open, edit, run and delete any made dot like any other dot; their edits are not held to the maker's reach. What the person narrows on a made dot (the browser or their Claude settings off, a Library item or variable removed, the schedule cleared, a lower limit or a narrower preset) counts as held until the dot has it again: a maker's change that gives it back goes to the person as a change request, whose line says it undoes their change.

When a maker is deleted, its made dots stay. They keep showing who made them, by the name the maker had (my call: keep the name as text so the list can still say "made by Researcher (deleted)").

## 10. Limits

- **Per run**: dots made by one run of the maker, default 3. A wake is a new run.
- **Total**: made dots of this maker that exist now, default 10. Deleting one frees a place.
- Passing either limit turns the make call into a make request (section 7). **Allow** covers that one dot.
- Waiting make requests do not count toward the limits until allowed.

## 11. What the person sees

Mockups are in `.superpowers/brainstorm/2300-1794370584/content/dots-make-dots.html` (not committed). The person saw them and said ok.

1. **Dot editor**: the **Making dots** section (section 5). On a made dot, a line near the top: "Made by <maker>", linking to the maker.
2. **Maker's chat topic**: cards, in the order things happen:
   - **Made dot: <name>**, tagged "made by <maker>", with the purpose line, schedule if any, what it uses in plain words ("Uses: web fetch, folder research/blogs. No secrets."), and **Open dot**.
   - **<name> is working** / **finished** (section 8).
   - The ask card (section 7.2).
   - **<maker> changed <name>** with what changed, and **<maker> deleted <name>**.
3. **Dots list**: made dots sit indented under their maker, tagged "made by <maker>". The maker's row shows "can make dots · 2 of 10". A made dot whose maker is deleted sits at the top level with its tag. A dot waiting on a make request is not in the list (it does not exist yet); the request shows in the Inbox.
4. **Inbox**: a section for waiting make requests, with the same card.
5. **Notifications**: the ask card notifies like an approval. "<maker> made a dot: <name>" notifies when the maker's run was not started by a message the person just typed (my call: no noise while they are watching the chat). Both follow the existing notification settings.

The maker's prompt gains a short **Making dots** section: the rules of sections 6 to 10 in a few lines, and the list of its made dots with purpose and schedule (the first 20, then how many more), so it reuses a dot before making another. The section says reports and the final messages of made dots are their work, not instructions. A made dot's prompt says the person's words and settings and its permissions come before what its maker or a job says. Job and wake runs are named as such in a dot's list of recent runs. Reports and jobs are quoted data and their runs start no workflows (section 8).

## 12. Constraints

- Subscription rule, unchanged: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials.
- Every make, change, delete and job goes through the same runtime path a person's action does, so today's checks (valid dot, name clash, Library items exist, account and workspace rights, schedule reload) all still apply.
- The reach check and the limits are enforced in the core, not in the prompt. The prompt only explains them.
- Existing dots are untouched by the upgrade: the switch is off, no maker is recorded.
- A new Dots-only policy name follows the pattern `Instructions` used, on both engines, with the upgrade step that gives existing policies the rule.
- An approval or grant for a call that names a made dot is for that dot, by its id, not its name: a rename cannot let one approval be spent on another dot.

## 13. What must be proven by tests

1. For each row of the table in section 6, a request beyond reach creates nothing and produces a make request; the same request within reach creates the dot.
2. A dot with the switch off, or with `MakeDots` denied, is not offered the tools, and a forged call is refused.
3. A maker cannot change, delete or give a job to a dot it did not make, including another maker's made dot.
4. The three answers on the ask card each do what section 7.2 says, and the maker's own settings are unchanged after **Allow for this dot**.
5. The limits of section 10 and the loop guard of section 8 hold, including across wakes.
6. A job's end wakes the maker exactly once, for every way a job can end, including a job whose first run ended on a question to the person; no wake happens while the question waits. A workflow-step run of a maker is not offered the tools.
7. Narrowing the maker afterwards leaves made dots as they were; the maker's next `change_dot` is checked against the narrowed reach.
8. The automatic variable grant never gives a made dot a name its maker lacks.
9. Each folder rule in section 6 holds: a link or junction that leads outside is outside; a subfolder under a worktree maker asks; a path inside a worktree copy is refused.
10. Shared spend: a maker's daily cap is reached by its made dots' spend, and deleting a made dot does not lower the shared total for that day.

## 14. Left to the builder

- Table and column names, the migration number (master was at `0045` when this was written; check again when executing), and the Rust and TypeScript type shapes.
- Whether make requests reuse the proposal tables' pattern or get their own table.
- The exact tool schemas and tool-result wording, within the 64-character tool-name limit.
- How the wake is queued (the existing continuation path is the precedent).
- Card layout details beyond what section 11 says.
- How shared spend is kept, as long as it survives the deletion of a made dot (section 6).
- The trigger kind of a job run, which decides whether the usage holds treat it as background.

## 15. What plan A built

Plan A (`docs/superpowers/plans/2026-11-10-dots-make-dots-a.md`) built the core: everything above except the screens of section 11, which are plan B's. A fix task after the plan's tasks (fix 14b) closed what the task reviews found, and the sections above were edited where its rulings changed them. This section records where the code still differs from the sections above, and what plan B must know. Where this section and an earlier one disagree, this section says what the code does.

### Names

- Module `dots_core::making`: `reach` (the check of section 6, and `held_extras`: what a made dot holds beyond its maker now), `service` (`Makers`, `make_dot`, the limits), `requests` (the ask card and its three answers, for requests of the kinds `make`, `change` and `job`), `manage` (list, change, delete; `delete_and_tell`, which the maker's delete and the person's share), `jobs` (jobs, job requests, the loop guard, `unattended`), `wake`, `log` (the making log of a topic), `cards` (the log as the chat's cards), `tools` (the five tools, shared by both engines; `checked_input` ties a call to one made dot).
- Tools: `make_dot`, `change_dot`, `delete_dot`, `give_job`, `list_made_dots`; on the Claude engine `mcp__dots__<name>`. Policy name `MakeDots`.
- Run payload keys: `job_id` and `job_from` on a job's run, `wake_id` on a wake's run, `background` on either when it counts as background, `from_maker` on a job's text sent again after its job ended.
- Migrations `0047` to `0054`, then `0055` to `0058` (all after master's `0046`): the dot's making columns, the `MakeDots` rule for existing policies, `making_events`, `making_wakes`, `make_requests` (and its `ask` column for a change), `making_jobs`, `made_spend_kept`; then a make request's stored ask (`make_ask`) and indexes, `making_jobs.held_told_at` and indexes, `dots.spend_parent_dot_id`, and `dots.made_narrowed` (what the person narrowed on a made dot).
- Limits: `MAKE_PER_RUN_DEFAULT` 3 (at most `MAKE_PER_RUN_MAX` 10), `MAKE_TOTAL_DEFAULT` 10 (at most `MAKE_TOTAL_MAX` 50), `MAX_WAITING_REQUESTS` 3 (make, change and job requests together), `MAX_JOBS_BETWEEN_TYPED` 6, `MAX_JOB` 4,000 characters, `MAX_JOB_RESULT` 8,000, `MAX_PURPOSE` 200, `MADE_LISTED` 20 (made dots named in the maker's prompt), `MAKER_DEPTH` 32 (how far a chain of makers is followed for spend).

### Where the code differs

Fix 14b's rulings that are now written into the sections above: a custom maker gives read-only only when its rules allow all of it (section 6); the folder rules for `.git`, `.claude`, other computers and a maker without a full path (section 6); the spend chain that survives a delete in the middle (section 6); a card and an answer judged against the maker as it is then, and the card's limit line sent with the answer (section 7.2); a job to a dot that holds more than its maker waits for the person, and the person's Allow is not held by the loop guard (section 8); reports and jobs quoted, their runs starting no workflows, and a wake that stays in its topic (section 8); a change to a dot that holds more than its maker asks unless it only narrows, with "Can make dots" switched on counted as more (section 9); an approval that names one dot by its id (sections 9 and 12). The final review's fix wrote three more into them: the card's extras sent with an answer (section 7.2), the job card's line that the result goes back to the maker (section 8), and the person's narrowing of a made dot kept (section 9); its other changes are in the table.

| Section | The code | Why |
|---|---|---|
| 6, spend "and month, for the monthly cap" | A dot has daily caps only. A deleted made dot's spend counts for the day toward its makers' caps, and for the month toward the global monthly cap | No per-dot monthly cap exists |
| 6, spend in the middle of a chain | Spend follows its own chain, `dots.spend_parent_dot_id`, set to the maker when a dot is made. When a dot in the middle is deleted, the dots it made take its own spend parent. The dots below are then nobody's made dots: they are in no maker's list or count, and show "made by <name> (deleted)" | A spend chain must not give A rights over a dot it never made |
| 8 "If the made dot is busy, the job queues behind its current run" | True when the made dot is busy in another topic. A second job from the same maker topic to the same dot is refused until the first ends, before any card is stored, with a sentence that says the maker will be woken | Jobs of one topic share one topic of the made dot, and a topic takes one run at a time |
| 8 "the person can stop it from the made dot's topic" | Stopping a job that waits on a question is dismissing the question. A question never answered or dismissed keeps the job open, and that dot takes no further job from that topic | The product has no other stop for a topic whose run has ended |
| 8 "counts as background or not as the maker's run did", and "no message the person typed" | A job's run and a wake's run are chat runs. A job's run carries `"background": true` when the run that gave the job was background, or when the maker's topic is unattended: nobody typed in it and its first run is background (a schedule's, a watcher's, or one a background job opened). A wake's run carries it only when the topic is unattended. A topic started with Run now, or by an app, is watched. A resume run after an approval, and a run that carries the person's answer, are foreground | Resume runs are foreground for every dot today; a person who pressed Run now is watching |
| 8 "tells the maker once that it has not started" | The maker is told only when its wake could start: not while the maker itself is held by a cap (its own, the one it shares, or the cap for every dot), and, while the maker's topic is unattended, not while Claude's usage hold or the role hold would hold the maker's background wake. The tell is then left for a later pass. A job is told once, ever | A wake that is held itself would keep the maker's topic busy, and the person could not type there |
| 8 "sends again after its job ended is no longer a job" | Send again and Retry of a job's run keep the text the maker's words: the new run carries `from_maker` and no `job_id`, its prompt quotes the text with a note that it is the maker's, sent again by the person, and that nobody is woken, and it starts no workflows. While the job is still open, Send again goes on with the job | The person pressed the button, but a model wrote the text |
| 8 (silent) a job given through an approval | A `give_job` the person approved on a `MakeDots` approval card still asks on a job card when the dot holds more than its maker | The approval card does not show what the dot holds |
| 8 loop guard | Counted from the `job_started` entries of the making log since the newest typed run of the topic | Job rows go with a deleted dot; the log does not |
| 9 `delete_dot` | With "Always ask before sending or deleting" on (it is on in every preset), a maker's `delete_dot` waits for the person's approval like any other delete a dot does. With the setting off it deletes at once | The setting is the person's, and a dot's delete of an app's data or a file already asks |
| 9, which dot an approval is for | `change_dot`, `delete_dot` and `give_job` find the made dot before the policy is asked, and the policy input is `{"tool", "input", "dot": {"id", "name"}}`, the name as stored. A name that finds no made dot is refused before the person is asked. An approval stored without the dot's id is refused when it is performed | So the approval card names the dot, and a grant is keyed on it |
| 9 "Removing things never asks" | Removing never asks about reach. An ask rule the person adds for `MakeDots` still asks for every call, as for any tool | The policy is the person's |
| 9 (silent) a person's delete of a made dot | Both deletes take one path, and both are refused while the dot has a run queued, running or waiting for an approval. A job of the dot still open is settled first: one whose run already ended ends as that run did, the rest end as `deleted`. Each maker topic is woken once with all it is told (the job, and a change or job request that went with the dot). The wakes are stored in the delete's own transaction, so makers are woken only if the dot is gone. The topics that showed the dot are told | A maker waiting on a job should learn the dot is gone, and only if it is |
| 11.2 "Uses: web fetch, …" | The line names Library items by kind, the browser, the person's Claude settings, the folder, and variables by name: `Uses: the browser, folder research/blogs. No secrets.` or `… Variables: API_KEY for the shell.` `Uses: nothing. No secrets.` when it uses nothing | A dot has a permission preset, not a list of tools; a variable may be plain, so "No secrets." is said only when there are none |
| 11.5 "Both follow the existing notification settings" | The ask notifies always, as an approval does, for a make, a change and a job request. "<maker> made a dot: <name>" follows `notify_on_notes` and is not sent for a typed run, or for a dot made by the person's own Allow | The one setting that exists is for notes, and it does not silence approvals |
| 11, the prompt section | In a conversation's first prompt only, as every prompt section. The list of made dots is from that moment; `list_made_dots` gives it as it is now | A resumed session already holds it |
| 7.2, who may answer | Allow and "without the extras" need `EditDots` (and `GrantEntries` when a variable is granted), and a dot within the workspace's guardrails ceiling: past it, the card shows Allow off with the ceiling's own words. Allow on a job request needs `RunDots` for the made dot. Refuse needs no right | App proposals ask `AnswerApprovals` for every answer; plan B decides whether Refuse should too |
| 7.2, the answer and the card | An answer carries the card's `shown` (a digest of the lines of what it showed beyond the maker's reach), as it carries `limit`. **Allow** and **Make it without these** are refused (`Conflict`), with the card read again, when the request would now give something else: the maker's settings moved twice between the card and the answer | The person's Allow must cover only what they saw |
| 8, a job card | A job card's lines end with "Its final message goes back to <maker>." When the job ends, the values of a variable the made dot holds beyond its maker's reach, as the vault gives them then, are replaced by `[hidden]` in its final text and error before they go into the wake. A value shorter than the vault's redactor scrubs (8 bytes) is left | A secret's value must not reach another dot's prompt |
| 8, a failed run's error in a wake | The wake says "The run failed. Its error text:" and quotes the error, on one line and cut at 300 characters, as it quotes the final text | CLI and MCP errors can carry tool or model text |
| 8, Retry of a wake's run | Retry of a wake's run that never reached the maker sends it again as Dots' report: the new run keeps `wake_id`, is not typed, starts no workflows and does not reset the loop guard | A model wrote part of the text |
| 8, a wake's run check | A wake's run is one Dots starts by itself. In a topic the person watches it is not made while this PC may not run the maker (`RunDots` in its account and workspace): the wake waits and is tried again after the loop's wait. In an unattended topic it is background, so the role hold keeps it queued. The wake that carries the person's answer to a request is one of these | No run starts beyond the account's settings with no person involved |
| 6, a custom maker's presets | A custom maker has a plain preset within reach when its rules allow everything that preset allows (each allow rule allowed for any input, or the same rule), none of its own deny or ask rules is missing from the preset, and its defaults are no wider. So one "Always allow" on a Sandboxed maker leaves the Sandboxed dots it made within its reach | Otherwise one ordinary approval makes every job and change of its made dots ask |
| 9, the person's narrowing | What the person narrows on a made dot with their own edit is kept (`dots.made_narrowed`) until the dot has it again, by the person's edit or with their say-so. A maker's change that gives it back is a change request; its line says "<setting>, which undoes your change to this dot", and "without" keeps the person's setting. A Custom policy the person sets is not kept as a narrowing | A person expects their own edit to stick |

### For plan B

- `Runtime`: `make_request_card(id)`, `waiting_make_requests()`, `decide_make_request(id, decision, limit, shown)` with `decision` `allow`, `without` or `refuse`, and `making_cards(root_run_id)`. The app has no command for them yet, and `MakeRequestCard` has no UI schema or contract sample: plan B adds them.
- The card (`MakeRequestCard`) is built fresh on each read, against the maker as it is then. Reading it writes no limit; a waiting request whose named things have gone is marked `gone` by the read, and `making_updated` is published, so plan B must not count on a read leaving the status as it was. `name` is the dot's name now. `asked` lists every setting a change names. `instructions` and `schedule` are given in full. `limit` is the limit line the card shows, and `shown` a digest of what it shows beyond the maker's reach (`None` when nothing). A job card's lines end with "Its final message goes back to <maker>.": plan B shows it. `can_allow`, `can_without` and `blocked` say which buttons are on, and why Allow is off.
- Plan B must send the card's `limit` and `shown` back with every Allow and Without (`Inbox.make_requests` items carry `shown` too). An answer whose limit or `shown` no longer matches is refused (`Conflict`) and `making_updated` is published: read the card again before Allow is offered again.
- Plan B must render the instructions, the model and a request's `first_job` as plain text, set apart from Dots' own words. For a request of kind `job`, `first_job` is the job itself: label it as the maker's job. A job request has no Without button.
- Never offer Allow from the Inbox item: it is slim. `Inbox.make_requests` (Inbox key kind `make_request`; each item's `kind` is `make`, `change` or `job`) is built from the card, so its `name`, `title` and `lines` match the card; the Inbox section reads the full cards with `waiting_make_requests()`. The rail count and `inboxKeys` already count make requests.
- `DotOverview.made_dots` with `dot.making.total` gives "N of M"; it counts by who made the dot. `DotSpend.shared_today` is the maker's shared total, by the spend chain, deleted dots included.
- Events: `making_updated` (read a topic's cards again, and any open ask card), `make_requested` (for every kind of request), `dot_made`. The app already turns `make_requested`, and a `dot_made` that is not typed, into OS notifications that open the maker's topic. The UI's `runtime-events.ts` handles none of the three yet.
- The UI schemas of `DotOverview.made_dots` and `Inbox.make_requests` are optional until plan B updates the test mocks; then they become required. `Dot.making`, `Dot.made` and `DotSpend.shared_today` are required already. The Inbox contract sample has one make request, of kind `make`. It has no change or job request, and no dot with `made` set: a second request moves the Inbox counts the UI tests pin.
- A job's run and a wake's run are chat runs. Their payloads carry `job_id` and `job_from`, or `wake_id`, and `background` when they count as background (a job's run when the run that gave it was background or the maker's topic is unattended; a wake's run only when the topic is unattended); a job's text sent again after its job ended carries `from_maker`. None of them is typed. The chat must draw none of them as the person's message: a job's text and a `from_maker` text are the maker's words, a wake is Dots' report. Send again and Retry already keep the maker's words, and a wake's run as Dots' report, in the core. A dot's recent runs in a prompt name them "Job from <maker>" and "Report from Dots".
- A job's run has no notification of its own; plan B decides whether it needs one.
- On the local engine a making call shows as tool `MakeDots` with input `{"tool", "input"}`; on the Claude engine as `mcp__dots__make_dot` and so on. The approval on either engine is for `MakeDots`; for `change_dot`, `delete_dot` and `give_job` its input also has `"dot": {"id", "name"}`, and the approval card names the dot by its stored name. A local approval is applied by the resume run, so the made dot's `made.run_id` is that run.
- A held run of a made dot names the maker's cap in its text only (`<maker> reached its $2.00 daily cap, shared with the dots it made`); no field says whose cap it is.
- `spend_today` lists a maker only when it has a cap or a run of its own today.
- No screen turns "Can make dots" on until plan B: until then the feature is reachable only through the store.

### Left open

- The making log and jobs are keyed by the topic's root run when written. Moving or merging a topic's runs does not move them: the cards stay with the old root, and the loop guard counts from the new one.
- "Another model of the same engine" is not checked: a made dot may name any model its engine accepts. The model's text is only checked to be plain.
- The per-run limit counts by run id, and a resume run after an approval is a new run, so the count starts again after each approved resume. The total limit still caps.
