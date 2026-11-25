<!-- dots:notes 0.5.5 -->
- Workflows cost less: each step can have its own model, a lean mode that leaves out what the step does not need, and a list of the earlier steps it reads, so a step no longer gets everything every time (#131)
- A workflow is no longer only for code. A team can work in a plain folder, a run can start with a form you fill in, and a step can use what an earlier step wrote by name, as in `{{draft.title}}` (#132, #134)
- Two new kinds of step: an action, which sends a Telegram message, calls a web address or writes a file without a sol, and a wait, which holds the run for some minutes, until a time of day, or until you answer (#132, #133)
- A workflow can start from a Telegram message, and a schedule can catch up on a run that was missed while the computer was off (#133)
- Describe what you want in a sentence and Sage drafts the workflow for you to check. There are ready-made templates, and a workflow can be saved to a file and brought into another team (#134)
- Workflows that run on a schedule or wait for a message keep the computer awake while they have work to do. Settings turns this off. The Teams page says which workflows are standing by (#135)
- Fixes found in a review of all of the above: a file step can no longer write into a project's git folder, secrets are hidden from what a run shows, and several cases where a run could stall or lose a send-back are closed (#136)
- Sage can be installed with one line in PowerShell or Terminal, which the repository's front page and each release's page now give (#137)
