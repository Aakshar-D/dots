# Skill creator smoke verification

- Date: 2026-10-21
- Build: debug (`bun run tauri build --debug --no-bundle` from `apps/dots-app`, into the shared target folder `C:/Users/aksha/dots/target`), branch `feat/skill-creator` at commit `e7051d5` (the skill creator with its review fixes: the save retries its rename briefly, a folder another row also uses is kept, the dialog stays open while it saves, the remove confirmation of a Dots-made skill says "Its folder in Dots' skills folder is deleted too."). Windows 11. The test suites were not run again by this task.
- Part A (agent steps): 7 steps. A1, A2, A3, A4, A6, A7 passed. A5 passed on its second attempt only: the first attempt found a stale-screen defect (finding 1). Each step ran once, in order, on one app process (after one restart to set the Claude program).
- Part B (person steps): 3 steps, all open. Nothing in Part B was run.
- Screenshots: not in the repository. They were in a temporary folder (`a1.png`, `a3.png`, `a5.png`, `a5b.png`, `a6.png`, `a7.png`) that this run deleted. Each result says what the text of the page showed.

## How the agent steps were run

The method is that of `docs/verification/2026-10-13-plan3-smoke.md`.

- The app is the debug build, started with `DOTS_DATA_DIR` set to a fresh folder under `mktemp -d` (`data`), `DOTS_CONFIRM=yes`, `WEBVIEW2_USER_DATA_FOLDER` moved into the same temporary folder, and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`. The real data folder `%LOCALAPPDATA%\dots` and the installed Dots (`%LOCALAPPDATA%\Dots`) were not touched.
- The `PATH` given to the app is the shell's `PATH` without any folder that holds `claude`, `claude.exe` or `claude.cmd` (one folder, `…\AppData\Roaming\npm`, was left out). `PATH="$CLEAN_PATH" which claude` found nothing. The real `claude` CLI was never run.
- The Claude program is `fake-claude.exe`, built with `cargo build -p dots-core --bin fake-claude` and copied out of `target/` into the smoke folder, then saved in Settings as "Program path" (confirmation by `DOTS_CONFIRM=yes`), then "Restart Dots". After the restart `status.claude.program` was the copied path and `server_error` null.
- `FAKE_CLAUDE_SCRIPT` names one file, `script.json`, in the smoke folder; a step copies the script it wants over it (`s1.json` first answer, `s2.json` second answer, `s6.json` sleeps 30 s, `s7.json` has no frontmatter). `FAKE_CLAUDE_ARGS_OUT` names `args.json`, which the fake CLI writes with its arguments, working folder and stdin (`prompt`).
- A successful answer is `{"emit": {"type":"result","subtype":"success","is_error":false,"result":"<SKILL.md text>","usage":{}}}`. The text of `s1.json` is a SKILL.md with `name: pr-tests`, `description: "Run the tests before opening a pull request."` and a short body; `s2.json` differs in the description ("…, second version.") and body ("Run the full test suite, then summarize failures by file.").
- The window is driven with `node tools/smoke/webview.mjs 9333 click|fill|wait|text|eval|goto|shot`. Run in Git Bash from the repository root; routes without a leading slash; `taskkill //PID`.
- Before starting: `tasklist | grep -i -E 'dots-app|fake-claude'` found nothing. At the end only the process this run started was stopped, by its id (30660), and `tasklist` showed no `dots-app.exe` and no `fake-claude.exe`. The temporary folders were deleted.

Setup commands, in short:

```bash
export CARGO_TARGET_DIR=C:/Users/aksha/dots/target
( cd apps/dots-app && bun run tauri build --debug --no-bundle )
cargo build -p dots-core --bin fake-claude
SMOKE="$(mktemp -d)"; mkdir "$SMOKE/data" "$SMOKE/webview"
cp "$CARGO_TARGET_DIR/debug/fake-claude.exe" "$SMOKE/"
# CLEAN_PATH: PATH without a folder holding claude / claude.exe / claude.cmd
( PATH="$CLEAN_PATH" FAKE_CLAUDE_SCRIPT="$SMOKE/script.json" FAKE_CLAUDE_ARGS_OUT="$SMOKE/args.json" \
  DOTS_DATA_DIR="$SMOKE/data" DOTS_CONFIRM=yes WEBVIEW2_USER_DATA_FOLDER="$SMOKE/webview" \
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
  "$CARGO_TARGET_DIR/debug/dots-app.exe" & )
W="node tools/smoke/webview.mjs 9333"
$W click "Open Settings"; $W fill "Program path" "$SMOKE/fake-claude.exe"; $W click "Save"
$W click "Restart Dots"
```

## Part A: agent steps

### A1. Create skill, an idea, Write it

```bash
$W goto library; $W click "Skills"; $W click "Create skill"
$W fill "What should the skill do?" "Run the tests before opening a pull request"
$W click "Write it"
```

Expect: the editor shows the script's SKILL.md and "The Library will show pr-tests: …".

Result: pass. The dialog text: "Create skill", "Claude writes it through your claude CLI, with no tools. You read it before it is saved.", "SKILL.md", "The Library will show pr-tests: Run the tests before opening a pull request.", "What should change?", "Start over", "Ask for changes", "Save to Library", "Close". The SKILL.md field held the exact text of `s1.json`. The recorded arguments, from `args.json`:

`-p --output-format stream-json --verbose --model sonnet --max-turns 1 --tools "" --strict-mcp-config --setting-sources "" --no-session-persistence --system-prompt <instructions>`

That is the command line of the Global Constraints, in that order, with the default model `sonnet`; `mcp_config` null, `plugin_dirs` empty, no `--resume`, no `--allowedTools`. The working folder was `<data>\drafts\<ulid>`. The request on stdin: "Write a skill for this idea:\n\nRun the tests before opening a pull request\n".

### A2. A hand edit, "What should change?", Ask for changes

```bash
cp "$SMOKE/s2.json" "$SMOKE/script.json"
$W fill "SKILL.md" $'---\nname: pr-tests\ndescription: "Run the tests before opening a pull request."\n---\n\n# pr-tests\n\nHANDEDIT run tests with bun.\n'
$W fill "What should change?" "Summarize failures by file"
$W click "Ask for changes"
```

Expect: the editor shows the second text; the recorded arguments match the Global Constraints and the request holds the hand edit.

Result: pass. The SKILL.md field held the text of `s2.json` ("…, second version." and "Run the full test suite, then summarize failures by file."), and "What should change?" was emptied. The recorded arguments were again the 16 of A1 (same flags, same `--system-prompt`, same absence of `--mcp-config`, `--plugin-dir`, `--allowedTools`, `--resume`). The request on stdin:

```
The idea the skill was written for:

Run the tests before opening a pull request

The current SKILL.md, between the lines <<<SKILL.md and SKILL.md>>>:

<<<SKILL.md
---
name: pr-tests
description: "Run the tests before opening a pull request."
---

# pr-tests

HANDEDIT run tests with bun.
SKILL.md>>>

Change it like this:

Summarize failures by file

Answer with the whole revised SKILL.md.
```

The hand edit (`HANDEDIT run tests with bun.`) is in the request, not the first answer.

### A3. Save to Library

```bash
$W click "Save to Library"
ls "$SMOKE/data/skills/pr-tests"; cat "$SMOKE/data/skills/pr-tests/SKILL.md"
```

Expect: the dialog closes, the row `pr-tests` is listed, the folder holds the text.

Result: pass. No dialog was left in the page. The Skills tab listed "pr-tests", "Run the tests before opening a pull request, second version.", the path `<data>\skills\pr-tests`, "Not used yet" and "Remove". `<data>/skills/pr-tests/SKILL.md` held the second text exactly (frontmatter with the quoted description, then "# pr-tests" and "Run the full test suite, then summarize failures by file.").

### A4. A dot's editor offers pr-tests; tick it, save the dot

A dot `skill-user` (Claude engine, folder a scratch git repository) was made with `create_dot` over the bridge, then its editor opened (`goto dots/<id>`).

Expect: the section "Skills" lists `pr-tests`; after ticking and "Save", the dot's `uses.skills` is `["pr-tests"]`.

Result: pass. The editor text had "Skills", "pr-tests", "Run the tests before opening a pull request, second version.". The checkbox has the accessible name "pr-tests" but `webview.mjs click` does not click checkboxes ("no control named …": its list is buttons, tabs, switches and links), so the box was ticked with `element.click()` through `eval`; it read `checked: true`. "Save" then gave `get_dot(...).uses` = `{"mcp_servers":[],"skills":["pr-tests"],"plugins":[],"context":[]}`.

### A5. Remove pr-tests

Expect: the confirmation names the dot and says "Its folder in Dots' skills folder is deleted too."; after it the folder is gone and the dot no longer lists the skill.

Result: pass on the second attempt; the first attempt exposed a defect (finding 1).

- First attempt, straight after A4 without leaving the app (Library, Skills, Remove): the row still read "Not used yet" and the confirmation read "Remove pr-tests? No dot uses it. Its folder in Dots' skills folder is deleted too." The dot does use it. The confirmation was cancelled.
- After a page reload (`location.reload()`), the row read "Used by skill-user", and the confirmation read "Remove pr-tests? Used by skill-user. Removing it takes it out of them. Its folder in Dots' skills folder is deleted too." "Remove skill" then: `<data>/skills` was empty, `get_dot(...).uses.skills` was `[]`, and the tab read "No skills yet."

### A6. A script that sleeps 30 s; Cancel

```bash
cp "$SMOKE/s6.json" "$SMOKE/script.json"
$W click "Create skill"; $W fill "What should the skill do?" "A slow idea"; $W click "Write it"; sleep 3
tasklist | grep -i -E 'dots-app|fake-claude'
$W click "Cancel"; sleep 2; tasklist | grep -i -E 'dots-app|fake-claude'
```

Expect: back on Describe with the idea; no `fake-claude.exe` left.

Result: pass. After 3 s: "Claude is writing the skill…", "Cancel", "Close"; `tasklist` showed `dots-app.exe` 30660 and `fake-claude.exe` 9020. After "Cancel": the text was the Describe step ("What should the skill do?", "A sentence or two is enough.", "Model", "Write it", "Close"), the textarea held "A slow idea", and `tasklist` showed only `dots-app.exe`. `<data>/drafts` was empty at the end.

### A7. An answer without frontmatter

```bash
cp "$SMOKE/s7.json" "$SMOKE/script.json"; $W click "Write it"
```

Expect: "The skill must start with a --- line…"; Save stays off.

Result: pass. The dialog showed the editor with "The skill must start with a --- line, then its name and description, then another --- line." and the "Save to Library" button was `disabled: true`. Nothing was written to `<data>/skills`.

## Findings

1. Defect, A5: the Library's "Used by" list goes stale after a dot's skills are changed. Saving a dot calls `invalidateDots`, which refreshes the dot queries (`dots`, `dot`, `overview`, `inbox`, `runs`) but not `keys.library`, whose entries carry `used_by`; the query has `staleTime: Infinity` (`ui/src/lib/query.ts`). A person who ticks a skill in a dot's editor and then opens the Library sees "Not used yet", and the remove confirmation says "No dot uses it" and does not name the dot. A reload shows the right list. The skill creator's own save does invalidate `keys.library`; the gap is in the dot save and in the delete of a dot. Not fixed here.
2. Observation, not a Dots defect: `tools/smoke/webview.mjs click` has no support for checkboxes; A4 used `element.click()`.

## Part B: person steps

None of these was run by an agent.

- B1. With the real `claude` CLI: create a skill from a one-line idea; ask for a change; save it; a Claude dot that uses it reads it on a task that calls for it.
- B2. The draft ignores the person's setup: put a rule in `~/.claude/CLAUDE.md` (for example "answer in French") and see that the draft does not follow it; then remove the rule.
- B3. The draft cannot act: an idea such as "list the files in this folder and write a skill about them" gives a skill text, and no tool call or approval appears anywhere in Dots.
