# Shared context library for dots

Date: 2026-10-19. Status: written without a question round (the person said "go"); every decision below is the writer's call unless marked otherwise, for the person to check while reading.
Extends the main spec `docs/superpowers/specs/2026-10-08-dots-design.md` and the MCP/skills spec `docs/superpowers/specs/2026-10-15-mcp-skills-plugins-design.md` (section 12 lists the passages it changes). Source of the idea: Approvals v2 spec section 14 ("a shared context library that many dots read") and the review of Cloudflare OS (`packages/gatekeeper-context`, `AdminConfig.instanceInstructions`).

## 1. Why

Every dot knows only its own `instructions` and the thread of its own runs. What several dots should all know (how the person names branches, who the customers are, the house style of a summary, a runbook) is copied into each dot's instructions by hand, and drifts.

This spec adds two things, for both engines:

- **Global instructions:** one text, set in Settings, that every dot's first prompt starts with. A dot can switch it off.
- **Context collections:** a Library item that names a folder of text documents. A dot that uses a collection sees it listed in its prompt and reads it with one read-only tool, `context`: list the documents, search them, read one.

Decisions (writer's calls):

| Question | Decision |
|---|---|
| Where a collection's documents live | A folder on disk the person picks; Dots never copies or edits it. The person edits the files with their own editor, and a git checkout works as a collection |
| How a dot reads them | One tool `context` (policy name `Context`), served by Dots on both engines: built in on the local engine, a tool of Dots' own `dots` MCP server on the Claude engine |
| Whether reading asks | No: every preset allows `Context`, as it allows `Skill`. A dot's own `Context` rule (ask, deny) still wins |
| How a dot gets a collection | Ticked in the dot's `uses`, like other Library items, or by the collection's "Every dot" switch |
| Search | A plain text search over the files at the time of the call; no index, no embeddings |
| Writing to a collection | Not possible through `context`. A dot that may write files writes only where its policy lets it, as today |
| Global instructions on resumes and chat follow-ups | Only the first prompt of a thread carries them, as it carries the dot's instructions today |

Out of scope: dots writing to or learning into a collection (memory), syncing a collection from git, embeddings or a search index, binary documents (PDF, Word), per-document permissions, a document editor in Dots, global instructions per engine or per preset.

Anti-goals: a dot's reading a collection ever needs approval under a preset; a collection's folder is modified by Dots; a document outside the collection's folder can be read through `context`.

## 2. Global instructions

- `Settings` gains `global_instructions: String`, default empty, at most 32 KB, checked by `Settings::validate`. Saving it needs no restart: it is read when each run's prompt is built.
- `DotSpec` gains `use_global_instructions: bool`, default `true` (`#[serde(default = …)]`, so stored dots get `true`).
- `build_prompt` (`prompt.rs`): when the setting is not blank and the dot uses it, the prompt starts with

  ```
  ## Instructions for every dot

  <global instructions, trimmed>

  ## Instructions for this dot

  <dot instructions, trimmed>
  ```

  Otherwise the prompt starts with the dot's instructions as today. Resume and chat follow-up prompts are unchanged (only the message).
- The heading text is fixed so a reader of a transcript can tell the two apart.

## 3. Context collections

### Library item

A new Library kind `context`:

```json
{ "path": "C:\\Users\\me\\notes\\company", "description": "How we work: style, people, runbooks", "every_dot": false }
```

- The name matches `^[a-z0-9][a-z0-9_-]{0,39}$` (a new arm of `library::check_name`); it is what the model passes as `collection`, so it is kept short and lower case.
- `path` is an existing folder, stored as given. `description` is at most 300 characters, may be empty. `every_dot` makes every dot use the collection without ticking it.
- `Uses` gains `context: Vec<String>` (`#[serde(default)]`). `validate()` and the store's "the Library has it" check cover it as other kinds. Deleting a collection that dots tick asks first and removes it from their `uses`, as for other kinds.
- A run's collections are the ticked ones plus every `every_dot` collection, each once, sorted by name.

### Documents

- A document is a file below the folder (any depth) whose extension, without regard to case, is one of `md`, `markdown`, `txt`, `rst`, `adoc`, `org`, `json`, `yaml`, `yml`, `toml`, `csv`.
- Left out: names starting with `.` (so `.git`), links (files and folders), and anything above the bounds: 2,000 documents per collection, 10 MB per document (the existing `MAX_READ_BYTES`), 64 MB read per search.
- The walk visits at most 100,000 entries, and the count of a larger folder is reported as a lower bound; a path whose resolved form has a hidden name below the folder (through a short name or a link) is refused; arguments echoed in a result are cut to 200 characters.
- A document's path is relative to the folder with `/`. Its title is the frontmatter `title`, else the first `# ` heading, else the file name without extension. Its summary is the frontmatter `description`, else the first non-empty body line that is not a heading, cut at 200 characters. Frontmatter is read with the existing `kit::parse_frontmatter`; one that does not parse gives no fields. Non-UTF-8 bytes are replaced.
- Folder walking is done in `spawn_blocking`, in a sorted order (paths compared as strings), so the same folder lists the same way every time.

### The `context` tool

One tool, the same for both engines:

| Argument | Meaning |
|---|---|
| `collection` (required) | A collection of the run, by name |
| `query` (optional) | Search the collection |
| `path` (optional) | Read this document |
| `offset` (optional, integer ≥ 1) | With `path`: the first line to return |

- **Neither `query` nor `path`:** the list of documents, one line each, `path — title: summary`, sorted by path; capped at 30 KB with a closing line saying how many were left out and to use `query`.
- **`query`:** case-insensitive search. The query is split on white space into terms (at most 10, each at least 2 characters). A document matches when it has every term, in its path, title or text. Results are ranked by the count of term hits, then by path; at most 10 documents, each with its path, title and up to 3 matching lines (`line N: text`, each cut at 200 characters). No match says so and lists nothing.
- **`path`:** the document's text from `offset` (default 1), capped at 30 KB. A cut result ends with `[truncated: continue with offset=<next line>]`. The path is resolved as the `skill` tool resolves `file`: no absolute path, no `..`, no drive or stream `:`, canonicalized and required to stay inside the canonical folder; it must be a document (extension list above) and not hidden.
- Both `query` and `path` given: an error that names the two forms.
- An unknown collection: an error listing the run's collections.
- A missing folder at call time: an error "the folder of collection <name> is missing".
- Every result is a normal tool result, so it shows in the run view and is stored in `run_events` as other calls are.

### Policy

- Policy name `Context`. Every preset gains a `Context` **allow** rule next to `Skill`: it reads only folders the person chose in the Library.
- The upgrade (migration 0007) adds a `Context` allow rule at the end of every stored policy that has no rule naming `Context` (`Context` or `Context(...)`).
- The check is the normal one (`PermissionGate::check`, tool `Context`), so a dot's `Context` ask or deny rule works as for any built-in tool, approvals included.

## 4. The two engines

### Prompt section (both engines)

A run with collections gets, after the standing rules of the first prompt (Claude engine) or in the system prompt after the skills section (local engine):

```
Shared context you can read. Call the tool `context` with a collection name to list its documents, `query` to search it, or `path` to read one. Look here before you guess how the person works.
- company: How we work: style, people, runbooks (42 documents)
- clients: (no description) (7 documents)
```

At most 50 collections are listed; the line count is computed when the run opens its tools. A collection whose folder is missing at that time is left out with a run event ("context company: folder not found").

### Local engine

- `context` is a built-in tool (`engine/local/tools/context.rs`), offered only to a run that has collections, like `skill`. Its arguments are registered in the tool argument table; policy alias `Context`.
- The engine runs an allowed call against the run's collections, as it runs `skill`.

### Claude engine

- Dots' own MCP server (`server/mcp.rs`, entry `dots`) lists, for a run with collections, a second tool `context` with the same arguments and description, besides `approve`.
- Its `tools/call` checks the call with `hub.check(run_id, "Context", input)` (waiting for an approval if a rule asks), then runs the same reader as the local engine and returns its text as a tool result; a denied call returns `isError: true` with the message, as the gateway does.
- `--allowedTools` gains `mcp__dots__context` for a run with collections, so the CLI does not first ask `approve` (the server checks it instead, as the gateway does for MCP servers).
- The prompt section of this section names the tool `context`; the CLI shows it as `mcp__dots__context`, and the section says so in its first line for the Claude engine: "Call the tool `mcp__dots__context` …".

## 5. Run tools

- `RunTools::open` resolves the run's collections (section 3), checks each folder exists, and counts its documents (bounded walk). `RunTools` gains `contexts() -> &[RunContext…]` with name, folder, description and count. Nothing is opened or held: each call reads the folder afresh.
- `OpenRequest` is unchanged; the store gives the `every_dot` collections.

## 6. Screens

- **Settings:** a "Global instructions" text area (with the 32 KB limit shown), saved with the other settings, and a line: "Every dot's first prompt starts with these. A dot can switch them off in its editor."
- **Library:** a fourth tab **Context** (after Plugins): name, description, folder, document count, "Every dot" badge, the dots that tick it. "Add collection" picks a folder, then name (default: the folder name, made valid), description, "Every dot" switch. Each row has "Open folder" and "Documents" (a list of the first 200 documents with title and summary, from a new command). Edit and delete as for other kinds.
- **Dot editor:** in "Tools and skills", a fourth checklist **Context** (an `every_dot` collection shows ticked and disabled, with "every dot"), and a switch "Start with the global instructions" (shown when the setting is not blank; its state is saved either way).
- **Run view:** `context` calls show as other tool calls (`context` on local dots, `dots › context` on Claude dots).

## 7. Commands, events and contract

- `library_add` / `library_update` / `library_remove` / `library_list` accept the new kind; `LibraryItem` JSON gains `kind: "context"` with its config.
- New: `context_documents(id)` returns `{ documents: [{path, title, summary}], total, truncated }` for the Library's "Documents" view (first 200).
- `Settings` gains `global_instructions`; `DotSpec` gains `use_global_instructions`; `Uses` gains `context`. The contract fixture is regenerated by its command.
- `RuntimeEvent::LibraryUpdated` is sent on collection changes as for other kinds.

## 8. Upgrade (migration 0007)

- `dots.uses` rows gain `"context": []` lazily: `Uses` reads a missing key as empty, so no SQL rewrite is needed.
- Stored policies gain the `Context` allow rule (section 3), done where 0006 added `Skill` (`store/upgrade.rs` pattern), once, recorded by the migration.
- `use_global_instructions` defaults to `true` through serde; `global_instructions` defaults to empty through `Settings`' `#[serde(default)]`.

## 9. Errors and edge cases

| Case | What happens |
|---|---|
| The folder is missing when the run starts | Collection left out of the run, run event "context <name>: folder not found" |
| The folder is missing at a call | Tool error, the run goes on |
| A document is changed during a run | The next call reads the new text: there is no cache |
| More than 2,000 documents | The first 2,000 in sorted order are listed and searched; the list says how many were left out |
| A search would read more than 64 MB | It stops at the bound and says the results may be incomplete |
| A path with `..`, an absolute path, a link out of the folder | Refused: "<path> is not inside the collection" |
| A non-document file (`.exe`, `.png`) named by `path` | Refused: "<path> is not a document of the collection" |
| Global instructions longer than 32 KB | Settings refuses to save, naming the limit |
| A dot's `Context` rule is `deny` | Calls are denied with the normal message; the tool is still offered so the model learns why |

## 10. Testing

- Core: the prompt with and without global instructions, switched off per dot, resume and chat follow-up unchanged; `Settings` limit; `Uses.context` validate and the Library check; `every_dot` merge; document walk (extensions, hidden, links, bounds, sorted order, title and summary rules, broken frontmatter); list, search (all terms, ranking, line snippets, caps), read (offset, truncation, containment: `..`, absolute, `C:`, stream `:`, link out, non-document); the `Context` preset rule and migration 0007 on a 0006 database; the local engine offering `context` only with collections and running it; Dots' `dots` MCP server listing `context` only for a run with collections, checking it through the hub (allow, deny, ask), and `--allowedTools mcp__dots__context`.
- The Claude engine through `fake-claude`, scripted to call `mcp__dots__context`.
- UI: the Context tab (add, documents, every dot, delete with users), the editor checklist and switch, the Settings text area.
- Smoke (agent steps, LM Studio): a local dot with a collection searches and reads a document; global instructions appear in the first prompt of a run's transcript. Person step: a real Claude-engine run that calls `mcp__dots__context`.

## 11. Open risks

- Plain search finds words, not meaning; a collection of many documents with different words for one thing needs the model to try several queries. An index can come later without changing the tool.
- Shared documents are trusted text from the person, but a collection that is a git checkout others push to is someone else's text in every dot that reads it. The Library's add form says: "Dots read these files as guidance. Only add folders whose writers you trust."
- The Claude engine sees the tool as `mcp__dots__context`; whether a model trained on Claude Code prefers its own `Read` on a path it guesses is a matter of the prompt section, checked in the person step.

## 12. Changes to the main and MCP/skills specs

- Main spec section 3: `dots` spec gains `use_global_instructions`; `settings` gains `global_instructions`.
- Main spec section 4, prompt assembly: the global instructions head (section 2).
- Main spec section 5 and MCP/skills spec section 7: every preset's `Context` allow.
- MCP/skills spec section 2: the Library kind `context` and `uses.context`; section 5: the `dots` server's `context` tool and `--allowedTools mcp__dots__context`; section 8: the Context tab and checklist.
