# General workflows: work that is not code

- Date: 2026-11-23
- Status: built (plan 2 of 5 of the workflows refresh), on top of plan 1 (`2026-11-23-lean-workflows-design.md`).
- Builds on: `2026-10-23-teams-workflows-design.md` (teams, steps, hand-offs, checks, deploys, the budget, the triggers).
- A workflow saved before this plan runs as it did. Each change to that is listed in section 10.

## 1. Why

Workflows were built for one job: sols changing code in a git repo, a check, a push. The person wants them to run a business pipeline end to end, set up by someone who is not a developer: write a post, have it approved, publish it on a day at a time; take a client's message, draft a reply, send it. Before this plan none of that could be built:

- A run needed a project, and a project had to be a git repo.
- The only steps were a sol's job, the person's check, a git deploy and a branch. Sending anything out meant a whole sol run.
- Nothing could wait for a time.
- Steps passed each other prose only: a summary.
- A schedule's run missed while Sage was closed was lost.

This plan adds what those pipelines need to the engine and the builder. Plan 3 adds the connectors (Telegram, mail, posting) and the triggers on what arrives; plan 4 the templates and plain-language building; plan 5 running with the window closed.

## 2. A run needs no git, and no project

- **A project is a folder.** It need not be a git repo. The core refuses one that is not a folder (`<path> is not a folder`). In a git repo a run works as before: worktrees, branches, a check that shows diffs, a deploy that ships. In a plain folder each step works in the folder itself, whatever its sol's workspace mode, and hands on no branch.
- **A run may name no project.** It then works in the team's own folder, `<data>/team-files/<team id>`, made when first used. The run's project reads `Files`, and its `project_id` is null. Run now and Test run offer `The team's own folder`; a schedule's project may be blank; a webhook with no project in its body or its trigger uses it; `start_workflow` with no `project` uses it.
- A deploy in a plain folder has no branch to ship: `push`, `pr` and `pr_merge` fail with `Nothing to ship: the check approved no branch.`, as before.
- Two runs in one plain folder at once work in the same files. Nothing stops it.

## 3. Inputs

A workflow's definition gains `inputs`: what a start is asked for besides the task. Each has a `name` (1 to 40 of a-z, 0-9, `-`, `_`), a `label` (what the person is asked), a `kind` (`text`, `long_text`, `date_time`), `required` and a `default`. At most 20.

- A start gives values by name. One it leaves out takes its default; a blank one is left out of the run. Refused: a name the workflow does not ask for; a needed one left blank (`"<label>" is needed to start`); a text over 8,000 characters; a `date_time` that is not `YYYY-MM-DDTHH:MM`.
- Run now and Test run show a field per input. A webhook's body may carry `inputs`, an object; `start_workflow` may too. A schedule starts with the defaults.
- The run keeps them (`workflow_runs.inputs`, migration 0062). Every agent step's prompt lists them under `### Inputs`.

## 4. Words in an action's texts

A step that runs without a model fills its texts from the run. A text holds marks:

| Mark | Filled with |
|---|---|
| `{{task}}` | the run's task |
| `{{input.<name>}}` | that input |
| `{{<step id>.<output>}}` | an output of an agent step before it (section 5) |
| `{{<step id>.summary}}` | that step's summary |
| `{{secret.<NAME>}}` | a Personal variable of this PC |

Saving refuses a mark that names nothing the workflow has, and a `{{` with no `}}`.

## 5. Outputs

An agent step gains `produces`: up to 10 names. Its prompt asks it to end its reply with each in a block of its name (`<post>…</post>`). When the step ends, the core reads the last block of each name from the whole reply and keeps them on the step's hand-off (`outputs`), each cut to 20,000 characters. A reply without one fails the step (`its reply did not end with a block for: "post"`), and the run waits for a retry.

No tool is called and no extra turn is spent: the outputs ride in the reply the step gives anyway.

## 6. Action steps

`kind: "action"`. Sage itself does one thing. No sol runs, so an action spends no run and no tokens.

- **`http`**: one request. `method` (`get`, `post`, `put`, `patch`, `delete`), `url`, `headers`, `body`.
  - The URL starts with `https://` or `http://`, and its host is written out: no mark in it. A step never chooses where a request goes.
  - A value filled into the URL is percent-encoded. A value filled into a JSON body (the body says so in `Content-Type`, or names no type and starts with `{` or `[`) is escaped to sit inside a JSON string; into a form body it is percent-encoded; into any other it goes as it is. A header value that would hold a line break fails the step.
  - No redirect is followed. The step waits 30 seconds for an answer. A 2xx answer succeeds it; anything else fails it with the status and the first 1,000 characters of the answer.
  - A Test run sends nothing: the step's summary says what it would send.
- **`file`**: writes `content` to `path` inside the run's folder, making folders on the way and replacing a file of that name. The path is relative and stays inside the folder, also after its marks are filled. A file may not hold a secret.
- **What is kept** is the step run's summary: `POST https://host/path answered 200.` and the start of the answer, or `Wrote <path> (<n> characters).` The URL is kept without its query.
- **Secrets.** `{{secret.<NAME>}}` is read from the Personal variables when the step runs. It is never stored with the run: wherever a kept text holds the value, in any of the spellings it was sent in, it is replaced by `•••`. Only a team in Personal reads secrets; a team of an account is refused, since its variables are the account's to grant.
- **A check first.** An `http` action needs a check before it on every path, as a deploy does, unless its `unattended` is on (`Runs without my check`). A `file` action needs none.
- **Stopped midway.** An `http` action that Sage stopped during may have gone out. It fails `interrupted` and waits for the person's Retry. A `file` action is written again.

## 7. Wait steps

`kind: "wait"`. The workflow waits for a time, with no sol and no one to ask.

- `minutes`: that long from when the step begins (1 minute to a year).
- `time`: the next `at` (`HH:MM`, this PC's time) after the step begins, on `weekday` when one is set.
- `input`: the date and time one of the run's `date_time` inputs holds.

The time is worked out once, when the step begins, and kept on its step run, so a restart changes nothing. A time that passed while Sage was closed ends the wait on the first pass after. An input left empty fails the step.

The run stays `running`, not `waiting`: it needs nobody, so it is not in the Inbox. Its clock stands still, so a wait counts no minutes of the budget. The loop passes when the soonest wait ends, not a tick later.

## 8. A check shows what goes out

A check now shows the outputs of the step before it, each in a field the person can change. Approve sends what was changed. The step that gave an output keeps the changed text, so every later step uses it, and the check's decision names what was changed (`edited`). An output the step did not give is refused.

## 9. A schedule that catches up

A schedule trigger gains `catch_up` (its schedule's own `catch_up` stays ignored, so a workflow saved before this plan is unchanged). With it on, the newest slot missed while Sage was closed or the PC asleep starts once Sage is open again, when it is under a day late. Older missed slots, and every missed slot with it off, are skipped as before.

## 10. What changes for a workflow saved before

- A project that is a plain folder is no longer refused.
- Run now is no longer off for a team with no project.
- A webhook call with no project starts a run in the team's folder, where it answered 400.
- Nothing else: `inputs`, `produces` and `catch_up` default to none, none and off.

## 11. The UI

- **Add step** offers Action and Wait.
- **An agent step's sheet** has Outputs: each name shown as a later step writes it, `{{draft.post}}`.
- **An action's sheet**: what it does (send a request, save a file), then its fields, `Runs without my check`, and a list of every mark its texts may hold.
- **A wait's sheet**: what it waits for and its fields.
- **The builder** has an Inputs card above Budget.
- **The schedule trigger** offers `The team's own folder` and `Catch up after Sage was closed`.
- **Run now** asks for each input and offers the team's own folder.
- **The check** shows the outputs, editable.
- **The run page** shows what an action did, and until when a wait waits.

## 12. Testing

- **Core:** the marks and how each value is escaped; the definition's rules for inputs, outputs, actions and waits, and their JSON; a run in the team's folder and in a plain-folder project; a start's inputs and the prompt; outputs read from a reply and a missing one; a check's outputs and an edited approval through to the file an action writes; an HTTP action against a local server (the request as sent, the secret kept out of what is stored, a refused answer, a retry, a Test run); a wait through its time, and one whose time passed while the loop was stopped; a schedule that catches up.
- **UI:** the new fields of each sheet and what a save sends; the Inputs card; Run now with inputs and no project; an edited check.
- **Not tested:** a real outside service. The HTTP action was run only against a local test server.

## 13. Known gaps

- An action's answer is not an output: a later step cannot use what a request returned.
- A wait needs Sage open to end on time. With the PC off, it ends when Sage next starts (plan 5).
- A scheduled slot that comes due while a run of the workflow waits at a wait step is skipped, as for any run in flight. A weekly workflow whose run waits for days blocks its own next slot.
- Secrets are read for a Personal team only.
- A schedule starts with the inputs' defaults: it cannot set them.
- The 24-hour limit of catch-up has no test of its own.
- `{{…}}` marks cannot be written literally in a text.
