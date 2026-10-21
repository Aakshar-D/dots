# Skill creator: design

## 1. Intent

Today a skill reaches the Library only as a folder that already holds a `SKILL.md`: the person writes the file somewhere, then "Add skill" points the Library at that folder. The owner wants to make a skill from an idea instead.

The owner's words and answers:

- "add a skill creator in library".
- Starting point: an "idea you describe in words".
- The flow played back and confirmed ("yes"): in the Library's Skills tab, click **Create skill**, describe the idea in a sentence or two, Claude (through the person's own `claude` CLI) writes the skill, the person reads the draft, edits it or asks for changes, and saves it. It then sits in the Library like any other skill and a dot can pick it in its editor.
- The owner then said "go" to the rest. Every other choice below is marked as my call.

Success: a person goes from a one-line idea to a skill a dot can use without leaving Dots, opening a text editor or choosing a folder.

## 2. Goals, non-goals, anti-goals

**Goals**

- Create a skill from a description, in the Skills tab.
- Fix a draft both ways: ask Claude for changes, and edit the text by hand (my call: the question about which went unanswered, and both are cheap).
- The saved skill is an ordinary Library skill: listed, picked by dots, removed like the others.

**Non-goals**

- Editing a skill after it is saved. The folder can be edited by hand; an "Edit" button is a later feature.
- Files beside `SKILL.md` (scripts, templates, references). The creator writes one file.
- Drafting with the local engine. Only the `claude` CLI drafts.
- Testing the skill on a sample task before saving.
- Streaming the draft into the editor as it is written. The dialog shows that Claude is writing, then the whole draft.

**Anti-goals**

- Dots reads, copies or sends the `claude` CLI's credentials, or calls an Anthropic API itself. It only spawns the official CLI (the subscription rule).
- The drafting run can act on the machine: read or write files, run commands, reach MCP servers or the network through tools.
- Saving overwrites a folder or a skill the person already has.
- A draft that the Library cannot read is saved, so the Skills tab shows an error row right after "Save".

## 3. Facts the design rests on

- A Library skill is `LibraryConfig::Skill(SkillConfig { path })`, a path to a folder holding `SKILL.md`. `LibraryService::add` (`crates/dots-core/src/library_service.rs`) reads the folder with `kit::read_skill` before it stores the row. A plain skill gets no confirmation dialog.
- `kit::read_skill` takes `name` from the frontmatter (else the folder name), `description` (else the first body line) and `when_to_use`. `check_name` (`library.rs`) allows a first ASCII letter or digit, then `A-Za-z0-9._-`, at most 64 characters.
- `kit::parse_frontmatter` (`kit/frontmatter.rs`) accepts top-level `key: value`, quoted values and `>`/`|` block scalars. Indented lines after a plain value join that value; after a bare `key:` an indented block or `- ` items are skipped as a list or map. Every field comes back empty when the block is broken: an indented line or `- ` item where a key should start (the first line, or after a quoted value), a duplicate key, a plain value containing `: ` or ending in `:`, a value starting with `@` or a backtick, or an unclosed quote. An unclosed `---` block gives no fields.
- `run_dirs.rs` strips `allowed-tools` when a skill is copied for a run, so a drafted `allowed-tools` grants nothing.
- `LibraryService::remove` deletes rows only, never files.
- `LibraryService` has no data folder today; the runtime holds it (`cfg.data_dir`).
- The CLI is found once, at startup, by `engine::claude::resolve_claude(Settings.claude_path)` (`runtime.rs`); if it is missing the runtime reports "claude CLI not found: install Claude Code or set its path in Settings". Runs pass the prompt on stdin, read `--output-format stream-json --verbose` with `parse::parse_line`, and cancel through a `CancellationToken` that ends the process tree (`proc.rs`). `cli_version` is the only one-off call today.
- The installed CLI offers `--tools ""` (no built-in tools), `--strict-mcp-config` (no MCP servers unless `--mcp-config`), `--setting-sources`, `--system-prompt`, `--max-turns`, `--model` and `--no-session-persistence`.
- The data folder is `%LOCALAPPDATA%\dots`, or `DOTS_DATA_DIR`. It already holds `logs`, `mcp`, `worktrees`, `plugins` and `backups`.
- There is no default model setting. A dot's model defaults to `"sonnet"`; the dot editor offers a `claude-models` datalist.
- Tests point Dots at `crates/dots-core/src/bin/fake-claude.rs` through `claude_path`; it replays `FAKE_CLAUDE_SCRIPT` and records its arguments to `FAKE_CLAUDE_ARGS_OUT`.

## 4. The dialog

In the Skills tab, a **Create skill** button sits next to "Add skill". It opens a dialog with three states.

1. **Describe.** A text box, "What should the skill do?", and a model field (the dot editor's `claude-models` list, default `sonnet`; my call). **Write it** is enabled once the box has text.
2. **Writing.** "Claude is writing the skill…" with **Cancel**. Cancel, or closing the dialog, ends the CLI process.
3. **Draft.**
   - The whole `SKILL.md` in a monospace text box, editable.
   - Under it, the name and description the Library will show, read from the text as it changes, or the reason the text cannot be saved (§6).
   - A smaller box, "What should change?", with **Ask for changes**. It sends the idea, the current text (hand edits included) and the request, and returns to Writing. A failed or cancelled request keeps the current text.
   - **Save to Library**, enabled while the text can be saved.
   - **Start over** returns to Describe with the idea kept.

A failed draft (the CLI is missing or not logged in, it exits with an error, or it answers with no text) shows the error in the dialog and stays in the state it came from. Closing the dialog with an unsaved draft asks "Discard this draft?" (my call).

## 5. Drafting

A new core function drafts once and returns text. It reuses the run machinery's pieces, not a dot run: no run row, no Runs entry, no policy, no approvals.

- **Command line** (my calls): `-p --output-format stream-json --verbose --model <model> --max-turns 1 --tools "" --strict-mcp-config --setting-sources "" --no-session-persistence --system-prompt <the drafting instructions>`. No `--mcp-config`, `--plugin-dir`, `--allowedTools` or `--resume`. The request goes on stdin. The `Config.claude_env` pairs are set, as for runs.
- **The CLI** is the one the runtime found at startup. A missing CLI fails the draft with the runtime's "claude CLI not found" message. Drafting does not need Dots' local server, so it works even when the server's port was taken.
- **Keeping the person's setup out** (my call): `--setting-sources ""` keeps the person's settings, hooks and `CLAUDE.md` files out of the draft, and `--system-prompt` replaces the default system prompt. The owner's Part B smoke checks this with the real CLI (§8). If the CLI still loads the person's `CLAUDE.md`, that is a follow-up, not a reason to change `CLAUDE_CONFIG_DIR`, which would also hide the CLI's login.
- **Folder:** the process starts in a new empty folder `<data>/drafts/<id>`, removed when the call ends. Startup removes leftovers.
- **Answer:** the final assistant text. If it is wrapped in a single fenced code block, the fence is stripped.
- **Limits:** 5 minutes, then the process is ended and the call fails "Claude took too long to write the skill" (my call). Only one draft at a time per dialog; the app keeps at most one running draft per dialog id and cancels it when the dialog closes.
- **Drafting instructions** tell Claude to answer with the complete `SKILL.md` and nothing else:
  - frontmatter first, between `---` lines, with `name` (lowercase letters, digits and hyphens, at most 64 characters, describing the skill) and `description` (one line saying what the skill does and when to use it, in double quotes);
  - only top-level `key: value` lines: no lists, no indentation;
  - then the instructions in Markdown, written for an assistant that will follow them;
  - for a change request: the current file and the request are given, and the answer is the whole revised file.
  The exact wording is left to the builder; it lives in one constant in core and is covered by a test that it names these rules.

## 6. Saving

**Save to Library** sends the text to a new core call, which:

1. parses the frontmatter: it must have a non-empty `name` and `description`, and `name` must pass `check_name` for a skill;
2. refuses a name already in the Library ("A skill named X is already in the Library") or a folder `<data>/skills/<name>` that already exists ("X already exists in Dots' skills folder");
3. writes `<data>/skills/<name>/SKILL.md`: first into a new folder beside it, then renamed into place, so a failed save leaves no half-written folder;
4. adds it with `LibraryService::add` as a skill with that path. If the add fails, the folder is removed.

The dialog shows the same checks live (§4) using the same parse, exposed as a small "check draft" call, so Save is only enabled when step 1 would pass. Steps 2–4 can still fail at save time; the error shows in the dialog and the draft stays.

## 7. Removing

A skill whose folder is inside `<data>/skills` (compared after resolving both paths) is Dots' own. Removing it from the Library also deletes that folder, and the remove confirmation says so ("Its folder in Dots' skills folder is deleted too."). Other skills keep today's behaviour: only the Library row goes. (My call: otherwise a removed skill's name can never be created again, and Dots' folder fills with orphans.)

- **What marks it** (my call): the path alone, no new column. A folder a person puts in `<data>/skills` by hand and adds with "Add skill" counts as Dots' own too; the confirmation tells them before anything is deleted. The library entry the UI receives gets a field saying the folder is Dots' own, so the confirmation can show the line.
- **Order** (my call): the folder is deleted first, then the row. If the folder cannot be deleted (on Windows, a file in it is open elsewhere), the remove fails, the row stays, and the error names the folder: "Could not delete <path>: <reason>. Close anything using it and try again." So a row never goes while its folder stays behind.

## 8. Testing

- Core: drafting with `fake-claude` (arguments recorded and checked against §5, the answer returned, a fenced answer unwrapped, an exit with an error, no text, cancel ends the process, the timeout with a shortened limit in the test); the draft check (missing or bad `name`, missing `description`, a broken block from §3 such as a duplicate key or an unclosed quote, no `---` block); saving (folder written, Library row added, a name or folder that exists refused, the folder removed when the add fails); removing a Dots-made skill deletes its folder, removing another skill does not, a folder that cannot be deleted keeps the row.
- fake-claude may need to answer a plain text turn without `--mcp-config`.
- App: the commands and the per-dialog cancel.
- UI: the three states, Ask for changes sending the edited text, Save enabled only for a draft that passes, errors shown in the dialog, the discard question, the Create skill button.
- Smoke (agents: `fake-claude`, temp `DOTS_DATA_DIR`): create, ask for a change, save, see the row, pick it in a dot, remove it. Part B for the owner: the same with the real `claude` CLI, plus a check that the draft ignores the person's `~/.claude/CLAUDE.md` (for example, a rule there to write in a set style is not followed).

## 9. Left to the builder

- The wording of the drafting instructions and of the dialog's copy, within §4–§5.
- How the dialog id and the cancel are carried between UI and app (an id passed with the draft call and a cancel command, as for follow-ups, is expected).
- Whether drafting lives in `engine::claude` or a new module beside it.
