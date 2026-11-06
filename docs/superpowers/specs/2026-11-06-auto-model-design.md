# Auto model: a dot picks Haiku, Sonnet or Opus for each task

- Date: 2026-11-06
- Status: built (owner's request, 2026-11-06)
- Builds on: the model switcher (2026-11-05), the runner, Settings › Claude.

## 1. Goal

"Model choice. Route cheap tasks to smaller models and hard ones to bigger ones, to cut cost."

Today a dot has one model, picked by hand. **Auto** is a new choice for Claude dots: the core
picks a model for each run from what the run is, and says why.

- Auto is listed first wherever a Claude model is picked: the dot editor, the switch above a
  conversation and Settings › Claude › Default model (so new dots can start on Auto). It is the
  model id `auto`.
- Local-engine dots do not get Auto: a local endpoint lists its own models. A local dot saved
  with `auto` is refused, and changing a dot from Claude to local drops it.
- Pickers of one fixed model leave Auto out: Auto's own sizes in Settings and the skill
  creator's model (a skill draft is no run of a dot; with Auto as the default it starts from
  Sonnet, and the core drafts with Sonnet if it is sent `auto`).

## 2. Routing (`dots_core::auto_model::route`)

Per run, in the runner, just before the engine starts. Fixed rules, no extra model call: the
same run always gets the same pick, and the reason is a few words.

**Signals** read from the run and its dot:

- **trigger**: schedule, watcher and webhook runs are background checks.
- **task text**: the run's message (the person's chat message, an app's or a follow-up's text, a
  workflow step's section); without one, the dot's instructions.
- **length** of that text: over 1,500 characters is *medium*, over 6,000 is *big*.
- **attachments** (the payload's `attachments`): one or more is *medium*, three or more *big*.
- **hard words** (whole words): build, implement, refactor, debug, design, architect,
  analyse/analyze/analysis, plan, investigate, migrate, optimise/optimize, rewrite, code,
  coding, program, algorithm, bug, diagnose, research, prove, strategy, audit, review.
- **light words**: summarise/summarize/summary, tldr, check, remind, list, translate,
  rephrase, reword, notify, ping, status, count, hi, hello, hey, thanks, weather, define,
  spell, lookup.
- **policy**: the dot *can act* when its policy does not deny all of Edit, Write, Bash and
  PowerShell.
- **workspace**: worktree mode means code work.
- **thread**: the tier Auto picked for the run's parent (the run it resumes or continues).

**Rules**, in order:

1. A message that says "use opus", "use sonnet" or "use haiku" gets that size ("you asked for
   Opus"). Only a message counts, not the dot's instructions.
2. A resume keeps its parent's tier ("continues the run it resumes").
3. Hard words: **Large** when the task is big ("long, hard task: design"), or is code work (a
   worktree dot) with two hard words or a medium task ("code work: refactor, debug"); otherwise
   **Standard** ("hard task: refactor").
4. Big without hard words: **Standard** ("long task", "many attachments").
5. Light words in a task that is not medium: **Small** ("light task: summarise"), also for a dot
   that can act.
6. Not medium, and the dot cannot act: **Small** for a background run ("background check") or a
   chat message of at most 280 characters ("short question").
7. Everything else: **Standard** ("has attachments", "long message", "can edit files or run
   commands", "general task").

So a dot that can edit files or run commands gets Standard at least, unless the task reads as
light; a hard word wins over a light one.

**Threads.** A conversation keeps the tier it had. It moves up only for rule 1 or when rules 3
and 4 pick a larger tier from the task itself ("escalated: hard task: refactor"); it never moves
down on its own ("kept from earlier in this conversation"), except when the person asks
(rule 1).

## 3. Sizes and models (Settings › Claude › Auto model)

`Settings.auto_models`: `small` = `haiku`, `standard` = `sonnet`, `large` = `opus` by default,
the curated aliases of `models::CLAUDE_MODELS`; Custom… takes any id `--model` takes. Each must
name a model, without spaces, and not be `auto`. Settings stored before this read the defaults.

## 4. Fallback

When an Auto run fails because the CLI could not use the model (its error says the model is
unknown, not available or not allowed for the account, or overloaded: "There's an issue with
the selected model", `not_found_error` about a model, `overloaded_error`, "… is not available
with … plan" and the like — `auto_model::model_unavailable`), the runner starts it **once** more
with the next size down (Large → Standard, Standard → Small, Small → Standard), only when:

- the failed attempt made no tool call (nothing done twice), and the run is not being stopped;
- the next size names a different model;
- the first attempt ends within 15 s (its channel is read to its end, so its process and
  temporary files are gone before the second attempt reuses the run id).

Usage limits and rate limits are not model errors: the usage hold handles them. The failed
attempt's error is not recorded; a notice is: "Opus could not be used (…). Trying again with
Sonnet." A dot with its own model is never retried.

## 5. What is recorded and shown

Migration `0038_run_model.sql` adds to `runs`: `model` (the model the engine was started
with, for every run), `model_tier`, `model_reason` (Auto runs only) and `model_fallback_from`
(the model tried first). They are on `Run` and in the UI's schema.

- Under the dot's reply, in the conversation and on a run card: "Auto · Sonnet — code work:
  refactor", with "· fallback" and a tooltip after a fallback.
- Run details: a **Model** entry, the label above or the model alone for a dot with its own,
  and "(Opus could not be used)" after a fallback.

## 6. Limits

- Word lists are English and whole-word; a hard task worded without them gets Standard (or
  Small, for a read-only dot's short question). The person can say "use opus".
- The pick is made once per run; a long run does not change model midway.
- Error matching follows the CLI's wording as of 2.1.x; new wording falls through to an
  ordinary failure, never to a retry.
- Cost per size is not measured here; the Usage page's API-equivalent cost shows the effect.
