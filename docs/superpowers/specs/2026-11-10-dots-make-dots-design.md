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
- **Spend survives a delete.** Today a dot's spend is summed from its runs, and deleting a dot deletes its runs. A made dot's spend must still count toward its makers' shared caps for the rest of that day (and month, for the monthly cap) after the made dot is deleted, by the maker or by the person. Otherwise a maker could clear its cap by deleting what it made.

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

The card and the answer are judged against the maker as it is when the person looks and when they answer, not as it was when it asked: the request keeps what the maker asked, the reach check and the limits run again, and the card's lines, its title and **Make it without these** come from that. **Allow** is off, and refused, when the maker was narrowed so that the dot would get something the card did not show at first; the person can still make it without these or not make it. An answer carries the limit line of the card the person answered; an answer that would make a dot past a limit that card did not show is refused, and the card read again shows it. Reading a card writes nothing. When it was the made dot that was given more since, the card says so. "Make it without these" tells the maker what was left out as it stood when the person answered. The Inbox item has the card's own title and lines. A card for a change lists every setting the change names, the new instructions in full; a card for a new dot carries its instructions and schedule.

Deleting the maker discards its waiting requests.

## 8. Jobs and wakes

`give_job` takes a made dot and a message (1 to 4,000 characters).

- A maker may give a job only to a dot it made itself (its own direct made dots).
- A job to a made dot that holds anything beyond its maker's reach as the maker is now (the check of section 9) does not start: it waits for the person as a job request, whose card shows the dot, what it holds beyond the maker and the job in full. **Allow** starts the job as it was given; **Don't give it** wakes the maker once; there is no "without". A job request counts toward the 3 waiting requests, and toward the loop guard only once it is allowed and starts; the person's Allow is not held by the loop guard.
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

When a made dot holds anything beyond its maker's reach as the maker is now (what the person allowed it or gave it, or what the maker no longer has), every change the maker asks for it that is not pure narrowing goes to the person as a change request, whose card lists what the dot holds; there is nothing to save without that. Pure narrowing is removing Library items or variables, a schedule of none, lower limits, a narrower preset, and switching the browser or the person's Claude settings off; a change of instructions, purpose, name, folder, model or schedule, or adding anything, is not. What a made dot holds counts its folder as within reach only when it is the new folder made for that dot, a folder of a worktree maker's own folder as beyond reach when the dot works in it directly, and "Can make dots" switched on as beyond reach. Deleting a made dot that has a change or a job waiting wakes the maker once to say the request went with the dot.

A maker can do these only to dots whose recorded maker is itself. The person can still open, edit, run and delete any made dot like any other dot; their edits are not held to the maker's reach.

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
