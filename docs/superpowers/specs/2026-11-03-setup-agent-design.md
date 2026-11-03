# Setup agent and easy first dot: design

- Date: 2026-11-03
- Status: builder check done; written for the person's review
- Part of: "Dots ready from the first minute". The person compared Dots with the core-build list of their other app ("linked agents") and asked to build what Dots lacks, starting with what saves a new person the most time. This document is that first part. Later parts get their own specs, in this order: thread tools (archive, move, split a topic); a Files and links panel and usage per topic; importing a dot and retrying failed background runs.
- Builds on: `2026-10-08-dots-design.md` (dots, runs, workspaces), `2026-10-21-skill-creator-design.md` (one-shot Claude drafts, the dialog pattern), `2026-10-27-conversational-chat-design.md` (chat topics), `2026-10-14-scheduler-v2-design.md` (schedules), `2026-10-13-approvals-v2-design.md` (policy presets).

## 1. Intent

The person's words: "as a new user, I don't want to waste much of my time setting up a dot", "make it as easy and as simple for the user to use dots right away", and "I want an agent up and running when we start the app ... it should be set up automatically once he hits run setup on the app and it sets up the dots for him for a particular task that he is starting the thread for."

What happens today (observed on 2026-11-03 creating the first dot of a fresh install):

- The home shows one paragraph and "Create your first dot", which opens a long form.
- The name "Test dot" is rejected: names allow only letters, digits, `-` and `_`.
- A folder is required, and the person must create one first.
- The default mode, "Git worktree per run", fails at the first run when the folder is not a git repository. Nothing warns at save.
- The Permissions section shows a table of about 25 rules.
- A yellow banner says the Claude CLI version "has not been tested with Dots".

Success: on a fresh install, a person types one or two sentences about a task ("every morning, summarise the new files in my Downloads folder"), answers at most two short questions, presses **Run setup**, and within a minute is in a conversation with a working dot whose first run is going. They never see the long form, a folder picker, a git error or a rule table unless they ask for them.

## 2. Goals, non-goals, anti-goals

**Goals**

1. A **setup agent**: describe a task in plain words, get one proposed dot, adjust it if wanted, press **Run setup**; Dots creates the dot (and its folder) and starts its first chat topic with the task.
2. The setup agent is the first thing on an empty home, and the default path behind **New dot**. The form stays one click away.
3. **Easier form**: a name may contain spaces; the folder may be left blank (Dots makes one); a folder that is not a git repository is never paired with worktree mode; the rule table is folded away behind the preset.
4. No warning banner greets a new person.

**Non-goals (this part)**

- The setup agent changing an existing dot (the chat's instruction proposals already do that).
- The setup agent creating teams, workflows, watchers, Library items (skills, MCP servers, plugins) or vault variables. It only picks from what the Library already has.
- Importing a dot from a file (a later part).
- A guided tour of the app.
- Local-model support for the setup agent itself: it uses the person's `claude` CLI, like the skill creator.

**Anti-goals (a failure even if it works)**

- A dot that cannot run: created with a folder that does not exist, worktree mode on a non-repository, or a name that breaks a run.
- The setup agent getting more power than the person approved. It proposes; nothing is created until **Run setup**, and the proposed permissions are shown in plain words on the card.
- A setup that loops on questions. It asks at most two rounds, then proposes.
- Existing dots, names or folders changed by this work.

## 3. Words

- **Setup**: one conversation with the setup agent, from the first description to **Run setup** or discard.
- **Proposal**: the dot the setup agent suggests, shown as the **setup card**.
- **Default folder**: `%USERPROFILE%\Dots\<folder name>`, where the folder name comes from the dot's name (section 7).

## 4. Where the person meets it

1. **Empty home.** When the current account scope has no dots, the home's right side shows the setup composer instead of today's paragraph:
   - Heading: "What should your first dot do?"
   - A text box (placeholder: "For example: every weekday at 9, read my Downloads folder and tell me what's new").
   - A **Set it up** button.
   - Below, a quiet link: "Fill in the form instead", which goes to `/dots/new`.
2. **New dot.** The **New dot** button in the home column opens the same composer in a dialog (title "New dot"), with the same "Fill in the form instead" link. Any existing route to `/dots/new` keeps working and shows the form.
3. The composer, the questions and the card are one component used in both places (my call).

## 5. The flow

1. The person types a description (1 to 4000 characters, trimmed) and presses **Set it up**.
2. The view shows "Setting up..." with a **Cancel** button. Dots runs the setup agent (section 6).
3. The setup agent answers in one of two ways:
   - **Questions**: one to three short questions. The view shows each with a text box, and a **Continue** button that sends the answers. There is also **Skip, just propose**, which sends no answers and tells the agent to propose now.
   - **Proposal**: the view shows the setup card (section 8).
4. After two rounds of questions the next request tells the agent it must propose (section 6). Dots also treats any third questions answer as a failure: "Setup could not finish. Try describing the task again, or fill in the form."
   - Rounds count only before the first proposal (my call).
   - **Skip, just propose** and **Ask for changes** always carry the must-propose line, and a questions answer to either is a failure with the same message; the card (for Ask for changes) stays as it was.
5. On the card the person can:
   - edit any field in place;
   - **Ask for changes**: a text box; Dots sends the current proposal plus the request and replaces the card with the answer;
   - **Start over**: back to an empty composer (confirm if a proposal is on screen);
   - **Run setup** (primary).
6. **Run setup** creates the dot (section 9). On success Dots navigates to the dot's conversation on the home, where the first run is already queued. On failure the card stays, with the error above the button.
7. Leaving the view (closing the dialog, navigating away) while the agent is working cancels that request, like the skill creator.

## 6. The setup agent

It follows the skill creator exactly (`skill_creator.rs`, `engine/claude/once.rs`): a one-shot `claude` call with no tools, run in a new empty folder under `data/drafts`, at most one request per setup id (a newer request cancels the older), a time limit of 5 minutes, failures reported as Cancelled, TimedOut or Failed.

**What it is told** (system prompt, a constant like `DRAFTING_INSTRUCTIONS`):

- Its job: turn a person's description of a recurring or one-off task into one Dots dot, asking only what it truly cannot guess, at most two rounds.
- What a dot is: a Claude Code agent with instructions it follows on every run, a folder it works in, an optional schedule, permissions, and Library items it may use.
- How to write the instructions: second person, numbered steps where there are steps, what to produce and where, what to tell the person at the end, what never to do. Plain Markdown, at most 4000 characters.
- The answer format: exactly one JSON object, nothing else (section 6.1).
- Guessing rules: prefer a sensible default and say so in `notes` over asking. Ask only about things that change what the dot does (which folder or website, what time, what to do with results). Never ask about model, limits or permissions.
- Permission presets in plain words, and to pick the least power that does the job: `read-only` (may read files, never writes, runs commands, reads websites or uses MCP servers), `sandboxed` (may edit files in its folder and run a few safe git commands; asks before anything else, including websites and MCP tools), `trusted` (may run commands without asking). Never `trusted` unless the person asked for it in their words. A task that saves files needs at least `sandboxed`.
- **Extras** (section 6.3): the few named permissions it may add on top of the preset so the dot runs unattended, and only when the task needs them.
- The schedule shapes it may use (the `Schedule` JSON in section 6.1), and that no schedule means the dot runs only when the person talks to it.

**What it is given** (the prompt on stdin):

- Today's date, the local time zone name and the weekday.
- The person's home folder path, so it can recognise "my Downloads folder" and write `C:\Users\<name>\Downloads`.
- The Library: every skill (name and description), MCP server (name), plugin (name) and context item (name) the person's Library has, excluding plugin-owned servers (same filter as `ToolsAndSkills.tsx`). Empty sections are left out.
- The names of the person's existing dots, so it avoids a clash.
- The description, then each round of questions and answers so far, then (for **Ask for changes**) the current proposal as JSON and the change request.
- On the third request of a setup (after two rounds of questions), a line saying it must answer with a proposal now.

### 6.1 The answer

Exactly one JSON object, optionally inside a code fence (strip it with the skill creator's `unwrap_fence`):

```json
{ "questions": ["Which folder should it read?", "What time each day?"] }
```

or

```json
{
  "proposal": {
    "name": "Downloads digest",
    "instructions": "1. List the files in ...",
    "folder": "C:\\Users\\pat\\Downloads",
    "schedule": { "rule": { "kind": "weekly", "days": ["mon","tue","wed","thu","fri"], "time": "09:00" } },
    "preset": "read-only",
    "extras": ["web"],
    "uses": { "skills": [], "mcp_servers": [], "plugins": [], "context": [] },
    "first_message": "Run it now so I can see the first digest.",
    "notes": "I assumed weekdays at 9:00. Change the schedule if you want weekends too."
  }
}
```

- `folder`: an existing folder the person named, or `null` for a new default folder.
- `schedule`: `null` or the core's `Schedule` JSON (`schedule.rs`). `catch_up` and `until` may be left out (core defaults apply).
- `preset`: one of `read-only`, `sandboxed`, `trusted`.
- `extras`: zero or more of the extras in section 6.3; may be left out.
- `uses`: names only; any group may be left out.
- `first_message`: what Dots sends as the first chat message after creating the dot. If missing, Dots uses the person's original description.
- `notes`: shown on the card under the title; may be empty.

### 6.2 Checking the answer (core, `check_setup_answer`)

The core turns the text into either `Questions(Vec<String>)` or a `SetupProposal` and never trusts it further than this:

- Not JSON, both keys, or neither key: Failed "The setup agent's answer could not be read. Try again."
- `questions`: 1 to 3 non-empty strings, each at most 300 characters; otherwise Failed as above.
- `proposal`: unknown `preset` becomes `sandboxed`, with a line added to notes. Items in `uses` that the Library does not have are dropped, each named in notes ("Left out skill X: it is not in your Library"). A schedule that fails the core's schedule validation is dropped, with a note. A name that clashes with an existing dot gets " 2", " 3", ... appended. Instructions over 4000 characters are cut at 4000 with a note. `folder` that does not exist on disk becomes `null` (a default folder) with a note.
- `extras`: unknown extras are dropped with a note; `mcp:<server>` whose server is not in the proposal's `uses.mcp_servers` (after dropping missing Library items) is dropped with a note; duplicates are removed.
- The checked proposal is what the card shows. Edits on the card are checked again by the same rules when the person presses **Run setup** (section 9), not on every keystroke.

### 6.3 Extras (the person's decision)

The person chose "preset plus extras": the setup agent picks a preset and may add a few named extras so the dot can do its task without stopping for approvals, and the card lists each extra in plain words with a remove button. The set is closed; nothing else can be added:

| Extra | Rules added on top of the preset | On the card |
|---|---|---|
| `web` | `WebFetch` allow, `WebSearch` allow | "Read websites and search the web without asking" |
| `mcp:<server>` | `mcp__<server>__*` allow | "Use <server> without asking" |

- Extras only add allow rules. They never remove a preset's deny rule: the preset's explicit denies (for example read-only's `Bash`, `Write`) still win, as `Policy::resolve` already orders deny before allow.
- A proposal with no extras keeps its pure preset (`Policy::preset(p)`, `preset` field `p`). With any extra, the dot's policy is `Policy::preset(p)` plus the extra rules, and its `preset` field becomes `custom` so the editor shows the real rules (the "Show rules" disclosure in section 10 opens by itself for `custom`). My call: labelling it with the original preset would show rules that do not match the label.
- The prompt tells the agent: add `web` only when the task reads websites; add `mcp:<server>` only for a server it also puts in `uses` and the task needs.

## 7. Names with spaces, and the default folder

- **Name rule** (core `DotSpec::validate` and the UI copy in `ui/src/lib/dot-rules.ts`): 1 to 64 characters after trimming; letters, digits, spaces, `-`, `_` and `.`; no leading or trailing space; no two spaces in a row. Existing names all pass this rule, so nothing migrates.
- The name is still unique across dots (today's case-sensitive UNIQUE constraint stays). New names are also checked ignoring case (my call): "Downloads digest" and "downloads digest" clash. The check is in the store, and runs only when a dot is created or its name changes, so existing dots whose names differ only in case keep saving. No migration, no new index.
- **Folder name** of a dot name (one function in core, `folder_name(name)`, my call): lower case, every run of characters other than ASCII letters and digits becomes one `-`, trimmed of `-`, cut to 40 characters; empty becomes `dot`. "Downloads digest" becomes `downloads-digest`.
- **Every place a dot name goes into something other than display text must use `folder_name`**, starting with the worktree branch `dots/{name}/{run_id}` in `workspace.rs` (`prepare_from`). The builder must grep for every other such use (paths, branch names, MCP or tool names, environment variable names, CLI arguments) and convert them. Log fields and display strings stay as they are.
- Existing worktrees keep working: runs already store their branch (`run.branch`), and `check_owned` in `workspace.rs` only needs a non-empty middle segment, so a run made before the change is cleaned up by its stored branch.

## 8. The setup card

One card, in this order:

1. Title: the dot's name (editable text field).
2. `notes`, if any, as a muted paragraph.
3. **What it does**: the instructions in a text area (about 10 rows, grows).
4. **When it runs**: the schedule in words, using the existing schedule summary used elsewhere in the UI (or "Only when you talk to it" for none), with **Change** opening the existing schedule fields from the form.
5. **Where it works**: the folder path, or "A new folder: `C:\Users\<name>\Dots\downloads-digest`" for the default; **Change** opens a folder picker (the dialog plugin the form's Browse uses) and a "Use a new folder" choice.
6. **What it may do**: the preset in one plain sentence (the three sentences in section 6), with a select to change it; under it, each extra as one line in plain words (section 6.3) with a remove button. Removing an MCP server from **Uses** also removes its extra.
7. **Uses**: the Library items as chips, each removable; nothing shown when empty.
8. Buttons: **Run setup** (primary), **Ask for changes**, **Start over**.

The model, limits, engine, browser and variables are not on the card; the dot gets the form's defaults (section 9) and the person can change them later in the editor.

## 9. Run setup (core, one command)

`setup_apply(proposal, org, workspace)` in the core, behind a new Tauri command, does in order:

1. Check the proposal again with section 6.2's rules (except the name clash: a clash here is an error, "A dot named X already exists", so the person sees it instead of a silent rename).
2. **Folder**: if `null`, make the default folder; if it already exists and is not empty, add `-2`, `-3`, ... until a new name is found. If a path is given, it must exist and be a folder.
3. **Mode**: worktree if the folder is inside a git work tree, else folder (the same `git rev-parse --is-inside-work-tree` check `workspace.rs` uses).
4. Build a `DotSpec` from `DotSpec::new` defaults (section 10), then name, instructions, folder, mode, schedule, the policy (`Policy::preset(preset)` plus extras, section 6.3), uses.
5. `Runtime::create_dot(spec, org, workspace)` as the form does.
6. `Runtime::chat(dot_id, first_message, None)`: the dot's first chat topic, a typed message.
7. Return the dot id and the run id.

If step 5 fails, a folder made in step 2 is removed again (only if still empty). If step 6 fails, the dot stays created and the command returns the dot id with the error; the UI opens the dot's conversation and shows a toast "The dot was created but its first run did not start: <error>".

The org and workspace are the ones a new dot from the form would get today (the selected account and workspace in the switcher).

## 10. Easier form

- **Name**: the new rule (section 7). The error text becomes "Use 1 to 64 letters, digits, spaces, '-', '_' or '.'."
- **Folder** may be blank. The hint under it says "Leave blank and Dots makes a new folder for this dot." `DotSpec::validate` still requires a workdir; `Runtime::create_dot` fills a blank one with the default folder (section 9 step 2) before validating, so both paths share one rule. Updating an existing dot never fills it: a blank folder on update stays an error.
- **Mode default**: the new spec's mode stays `worktree` in the core, but at create time, if the folder is not inside a git work tree and the mode is worktree, create switches it to folder and the UI says so in the "Dot created" toast ("Runs in the folder itself, because it is not a git repository."). On **update**, worktree mode on a non-repository is refused at save with "This folder is not a git repository. Choose 'The folder itself' or pick a git folder.", so it never fails only at run time. (My call: switching silently on update would surprise someone who just changed the mode on purpose.)
- **Permissions**: the preset select stays visible with its one-line description. The rule table, the default rule and the shell grants are inside a closed "Show rules" disclosure. It opens by itself when the preset is `custom`.
- The other sections (Tools and skills, Variables, Browser, Limits) are unchanged.

## 11. The untested-CLI banner

The banner stays for people who have dots, because it guards against a changed CLI output. It is not shown when the person has no dots (my call: a fresh install should open on the setup composer, not a warning). It also gets a **Dismiss** that hides it until the version changes again (same effect as "Mark as tested", stored the same way). The Settings entry stays.

## 12. Errors and edge cases

- `claude` not installed or not signed in: the setup agent's request fails; the view shows the failure text and a line "Dots uses your Claude Code CLI. Install it and sign in, then try again.", plus the "Fill in the form instead" link. Detecting this reuses whatever status the app already has for the CLI (the banner's status); the builder decides the exact check.
- Claude usage held (`usage-limits` hold): the setup agent is a person's own action, like chat; it is never held.
- Two setups at once (dialog open while the empty-home composer exists): each has its own setup id; they do not cancel each other.
- The home scope changes while the dialog is open: the dot goes to the scope selected when **Run setup** is pressed.

## 13. Testing

- Core: a `setup_test.rs` in the style of `skill_draft_test.rs`, with fake-claude answering scripted JSON: questions; a proposal; bad JSON; unknown preset; extras (`web` rules added, `mcp:` without its server dropped, unknown dropped, preset field becomes `custom`, read-only denies still win); questions answering a skip or a change request; missing Library items; schedule that fails validation; name clash; non-existent folder; the third request carrying the must-propose line; cancel. `setup_apply`: default folder made and named from the dot name, `-2` suffix, git folder gets worktree and plain folder gets folder mode, dot created with the preset and uses, first chat run queued with `first_message`, folder removed when create fails.
- Core: name rule (spaces, dots, case-insensitive clash), `folder_name`, worktree branch built with `folder_name`, blank workdir filled at create and refused at update, worktree on non-repository switched at create and refused at update.
- App: the new commands in the contract fixture (`UPDATE_CONTRACT=1`).
- UI (vitest with the fake backend): empty home shows the composer; questions round and skip; card edits; Ask for changes; Run setup navigates to the dot; failure keeps the card; New dot dialog and "Fill in the form instead"; name with spaces accepted in the form; Permissions folded; banner hidden with no dots and dismissible.

## 14. Left to the builder

- Module and command names, the Rust types, and whether the setup agent shares `SkillDrafts` code or copies the pattern.
- The exact system prompt wording, inside the rules of section 6.
- The visual details of the composer and card, using the existing components and the deep-teal theme.
- How the "Show rules" disclosure looks.
