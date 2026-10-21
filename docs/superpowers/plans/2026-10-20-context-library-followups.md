# Context library: follow-ups

What the execution of `2026-10-19-context-library.md` left open (PR #9, merged as `8c055dc`). Every item comes from a task review, the smoke or the whole-branch review, and is still open in `8c055dc`; findings fixed during the execution are left out. The final reviewer judged that all of them can wait. They are a starting list for the next plan, not decisions.

## Decisions to take

- **Arguments on Dots' own MCP server.** The `dots` server passes a call's raw arguments to the policy and to `context::call` (`crates/dots-core/src/server/mcp.rs`). The local engine checks arguments against the tool's table first. An argument table for the server would make both engines refuse the same malformed calls the same way.
- **Collections added after a thread started.** Chat follow-ups and resumes keep the prompt of the thread's first run, so they never learn of a collection added or ticked later. Options: a notice in the follow-up prompt, or a fresh "Shared context" section for each follow-up.
- **An ask rule next to the preset's allow.** Policy precedence is deny, then allow, then ask, so a `Context` or `Context(x)` ask rule changes nothing while the preset's `Context` allow is there. This is the existing design for every built-in tool. The policy editor could say so when a person adds an ask rule that an allow rule outranks.
- **A rule for one collection on the approval card.** `policy::allow_candidates` offers no "allow this collection" candidate (`Context(<name>)`) for a `Context` call.

## Reading documents (`crates/dots-core/src/context.rs`)

- UTF-16LE text files, which Windows PowerShell 5.1 `Out-File` writes by default, are read as noise. A byte-order mark check would decode them.
- `read` depends on `canonicalize`, which fails on some virtual drives.
- Between `canonicalize` and the open in `read` the file can be swapped. `search` and `documents` re-resolve `root` joined with the relative path without the confinement check. NTFS hard links cannot be told apart from ordinary files; the spec should say so.
- A frontmatter block scalar with newlines, or a Unix file name with `\n`, can forge list lines or closing lines in the tool output. Replace control characters with spaces.
- A line longer than 30 KB loses its tail with no notice (minified JSON, for example).
- The 64 MB budget of `search` counts `metadata().len()`, not the bytes read.
- `search` can go about 130 bytes past 30 KB with its closing notes; `cap_head` then cuts them off. Reserve a margin, as `list` does.
- When the walk is cut short, `search` looks through the first 2,000 documents in the order the OS walks them, not in sorted order (accepted by a ruling, open to change).
- When the walk is cut short and found no documents, `list` says "at least 0 documents; only the first 2000 found are listed".
- Names that are not valid UTF-8 are made lossy: they are listed but cannot be read. Skip them.
- `MAX_DOCUMENT_BYTES` duplicates `engine::local::tools::MAX_READ_BYTES`; the spec says to reuse it.
- `walk_bounded` and `list_bounded` are public without `#[doc(hidden)]`.
- A dot reaches every collection with the tool, but the prompt lists at most 50 of them, with no notice of the rest.

## Library and collections

- Relative, drive-relative and root-relative folder paths are accepted (`crates/dots-core/src/library.rs`). Refuse a path that is not absolute; consider refusing folders inside Dots' data folder or its worktrees.
- Two collections can point at one folder through case or trailing-separator variants.
- A description or path is not checked for control or bidi characters (no `check_plain`). The prompt now collapses a description's whitespace, but other control characters and the tool output are not covered.
- `library_list` walks every collection one after another on each list (`library_service.rs`).
- `context_documents` maps every walk error to `NotFound`, also "access denied".
- On add, `read_folder` runs before `validate`, so a blank path gives `NotFound`, not `Invalid`.
- The Library's Context tab keeps showing the old document count for a folder moved outside Dots until the page is reloaded. No event refreshes `library_list` when a folder changes (seen in the smoke).

## Runs and the prompt

- The runner reads Settings for every run, also for resumes and follow-ups that do not use the global instructions. A failed settings read fails such a run.
- At run start the collections are walked one after another and the walk ignores cancel. This also happens on resumes and follow-ups, which never show the count (`run_tools.rs`).
- A dot whose policy cannot be parsed never gets the `Context` allow: `finish_0007` logs it and drops the table. The spec should say so.

## Claude engine and the `dots` server

- The server's context call has no time limit, unlike the CLI's `MCP_TOOL_TIMEOUT`. A slow share or a OneDrive folder can end in a transport timeout.
- `server/mcp.rs` parses the arguments before it looks up the run's collections, so a run without collections gets an argument error instead of "no collections".
- A policy deny `mcp__*` or `mcp__dots__*` reaches `--disallowedTools` and silently switches off `mcp__dots__context`. The spec should say that `Context` rules govern the tool.
- `server/mcp.rs` imports `engine::local::tools::cap_head`, so the server depends on a local-engine module. Move `cap_head` to a shared module.

## Policy

- Prefix patterns such as `Context*` or `Cont*` still reach the CLI lists (`policy.rs`, `cli_lists`).
- When a rule is saved with an approval (`store/approvals.rs`), nothing checks that `validate` runs on it. No `Context` candidate exists today, so nothing reaches it yet.

## UI

- Settings: the byte-count hint for the global instructions disappears when the field shows its over-limit error (`SettingsScreen.tsx`, `Field.tsx`). `SettingsScreen.tsx` keeps `?? ""` and `Partial` only for old callers (plan-mandated).
- Dot editor: the note under "Start with the global instructions" is not tied to the switch by `aria-describedby` (`DotForm.tsx`). The disabled tick box of an every-dot collection has no description link (`ToolsAndSkills.tsx`).
- Library, Context tab (`ui/src/screens/library/ContextTab.tsx`):
  - An every-dot row says "Every dot reads it" and hides the dots that tick it.
  - The add and edit dialog is not a form, so Enter does not submit it.
  - The path is truncated with no `title`.
  - Open Edit and Documents dialogs keep the entry as it was when they were opened.
  - The Documents dialog says "No documents in that folder" when a walk cut short found none.
  - `nameFromFolder` gives odd or empty names for drive roots and non-ASCII folder names; the person must type a name.
  - The folder-picker code is written twice, and the "at least " text twice (row and dialog); an `atLeast(complete)` helper would serve both.
- `LibraryScreen.tsx` has a doc comment longer than 100 columns.
- zod: `z.number().int()` for the counts could be `.nonnegative()`.

## Tests

- No test reads an old `DotSpec` JSON without `use_global_instructions`, or a dot row from before 0007, as `true`.
- No test of the 10 MB refusal or skip, the 64 MB stop of `search`, or a Unix file symlink.
- No test of the description limit through the service's add, of collections after plugins in `ITEM_ORDER`, or of an `every_dot` update publishing `LibraryUpdated`.
- No test of a local `Context` ask rule, then its approval, then the resume.
- No test of `ContextDetail.complete` coming from a real bounded walk through `library_service` (the service cannot reach the bound).
- UI: no test that pasting 16,385 × "é" into the global instructions blocks Save and does not call `update_settings`; no test of the byte-count hint.
- UI: no test of a stale name in `uses.context`, or of a name both ticked and every-dot.
- UI: the trimmed-description test checks only that `library_add` was called, not the value saved.
- New test strings use raw line breaks instead of `\n` (`context.rs` unit test, `context_test.rs`, `claude_gateway_test.rs`).

## Smaller

- `store/dots.rs`: the `UPDATE` string runs to about 120 columns, and the `DOT_COLUMNS` line break is ragged.
- `apps/dots-app/src/contract.rs` adds the `ContextDocuments` sample after the `json!` macro, which is at its recursion limit; `#![recursion_limit]` is the other way.
- Spec §4: the prompt example shows no "at least N documents" case.
- Data folders from smoke runs of Task 1-3 builds of the branch fail sqlx's checksum, because `0007_context.sql` changed after Task 1. Delete them.

## Smoke

The agent ran Part A of `docs/verification/2026-10-19-context-library-smoke.md` (A1-A3, no Dots defect). Part B, the person steps with the real `claude` CLI, is not run by an agent: B1 (a Claude dot reads a collection without an approval) and B2 (under a `Context` ask rule the call reaches the Inbox, and the resumed run reads the file). Part A also leaves out:

- the Library's editor flows through the window (Add collection, Edit, the document list, the dot editor's tick boxes and the switch);
- a collection with many documents, a large file, or files that are not text;
- a rule that names one collection (`Context(company)`).

The local model used in the smoke (`gemma-4-12b-qat`) made `Bash` calls nobody asked for when `Context` was denied, which parked two runs. The next smoke should allow for retries.
