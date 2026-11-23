# Plain workflows: made by someone who is not a developer

- Date: 2026-11-23
- Status: built (plan 4 of 5 of the workflows refresh), on top of plan 3 (`2026-11-23-connected-workflows-design.md`).

## 1. Why

The person wants to hand Sage to someone who is not technical, or to a company, and have them make the workflows they need. An audit of the UI on 2026-11-23 found that a first workflow took about 16 clicks and some 15 ideas the person had to hold already (team, member, project, branch, send-back, outcome, delegate, budget), with no example to start from, no way to copy one, and no way to hand one to someone else.

Plans 2 and 3 gave workflows the steps real pipelines need. This plan is about starting one.

## 2. Describe it, and Sage writes it

**New workflow** opens a dialog. The person says what they want in their own words, and Sage writes the workflow.

- One call to the `claude` CLI, on the default drafting model, as the skill creator and the setup agent make theirs. The model is told the team's members by name and role, the team's projects, and the shape and rules of a workflow.
- The model names members by name; the core puts their ids in. A name the team does not have becomes the first member, with a note saying so.
- The model writes no key. Where a step needs one it names a variable and says in its notes which key to keep under that name.
- The answer is read into a definition and checked by the same rules a save uses. A draft that breaks one is sent back once with the rule it broke. A second that breaks one is refused with that rule: `The workflow Claude wrote cannot be used: <rule>. Say it another way, or build it by hand.`
- Nothing is saved by drafting. The dialog shows the name, the steps in order with their kinds, and **Before it can run**: what the person still has to do. **Make it** then makes the workflow, turned off, and opens it in the builder.
- A newer request from the same dialog cancels the one before, and closing the dialog cancels it.

## 3. Start from one that is ready

The same dialog lists five workflows to start from. Each is shown before it is made, with what is left to do:

| Workflow | What it does |
|---|---|
| Write a post, approve it, publish it at a time | A sol writes; the person approves; Sage waits for the date and time given, then sends it to a posting service |
| Answer messages on Telegram | A message to the bot starts it; a sol writes the answer; the person approves; Sage sends it to the same chat |
| Write an email, approve it, send it | A sol writes a subject and a body; the person approves; Sage sends it through a mail service |
| A report every Monday morning | On a schedule that catches up; a sol writes; Sage saves the file |
| Change code, check it, open a pull request | The first team's workflow, as it was |

Their steps go to the team's first member. The ones that send use a placeholder address (`https://example.com/your-posting-service`) and a named variable for the key: the person fills in their own service.

**Start with an empty one** makes the one-step workflow New workflow used to make.

## 4. Drafts

A workflow written by Sage, started from a template or brought in from a file may not be finished: the Telegram template listens to no chat yet. So such a workflow is stored as a **draft** (`workflow_create_draft`): turned off whatever it asks for, and kept even while its steps break a rule. It shows that rule as its problem, as a workflow whose member left does, and a save refuses it until it is put right. Its name is checked as any workflow's. A workflow made any other way must still break no rule.

## 5. Copy, and a file to hand over

- **Copy** on a workflow's row makes a draft of it named `<name> copy`.
- **Export** writes the workflow to a file the person picks: `{"sage_workflow": 1, "name", "definition"}`, with each member named by its sol's name, since ids mean nothing on another PC. A file holds no key: steps name variables.
- **Import** brings a file into the team as a draft. Members are found by their sols' names, without case. One the team does not have stays as the file names it, and the builder shows `<name> is not a member of the team`. A name the team already has gets a number. A file that is not a workflow, is over 1 MB, or is of another version is refused: `That file is not a Sage workflow.`

## 6. Testing

- **Core:** the draft's prompt; an answer read into a definition (members by id, ids made for steps, a fenced answer); a member the team lacks; answers that are no workflow or break a rule; a draft kept off with its problem and refused by a save; a workflow written to a file and brought into its own team and into another.
- **UI:** the dialog's three ways in, what it shows before making, a failed draft, the copy, the file out and the file in.
- **Not tested:** the model. No test asks a real model for a workflow, so how good its drafts are is not measured here. The reading of its answer and the second asking are tested with written answers.

## 7. Known gaps

- What is left to do is shown once, in the dialog. The builder does not keep it; it shows only the rule a draft breaks.
- A draft is one call. It cannot ask the person a question back.
- The templates name no real posting or mail service: the person supplies the address and the body their service wants.
- The builder's own wording is unchanged: it still says branch, deploy and send-back where a step uses them.
- A run's cost is still shown per run, in tokens, not per step and not in money.
- An exported file carries the workflow only: not the sols, their instructions or the variables it names.
