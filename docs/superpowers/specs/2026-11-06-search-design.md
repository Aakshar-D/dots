# Search: one box across chats, results, memory, goals and files

- Date: 2026-11-06
- Status: built (owner's request: "Searchable history. Search across all chats, decisions and files.")
- Builds on: `2026-10-23-dot-memory-design.md` (memory), `2026-11-04-topic-tools-design.md` (topic titles), the Intents spec (goals), the attachments spec (chat files).

## 1. Goal

A person can find something said or decided weeks ago without remembering which dot or topic it was in. Today only the Memory tab has a narrow filter, and History lists old topics without a search. Search is one box, opened from anywhere with **Ctrl+K** (Cmd+K on a Mac) or the search button in the rail, that looks through everything Dots keeps as text for the account and workspace in view.

## 2. What is indexed

Each kind is copied, as its table stores it, into one full-text index (migration 0037: `search_docs` and the FTS5 table `search_fts`).

| Kind | What | Opens |
| --- | --- | --- |
| `chat` | What the person said: a chat run's `message`, or the `answer` it carries | Its topic on the home |
| `reply` | What a dot said: a run's result (`summary`) and each `assistant_text` event it wrote along the way. One hit per run: the best of them | Its topic on the home when the topic started in a chat, else the run's own page |
| `note` | A note or question a dot posted (`messages`) | Its run's topic; a note of Dots' own opens the dot's conversation |
| `memory` | A memory (`memories`) | The dot's Memory tab |
| `intent` | A goal (Intents; the "decisions") | The Intents screen |
| `topic` | The title the person gave a topic | The topic |
| `file` | The name of a file added to a chat, once a message took it | Its topic |

The text is what the tables already hold, so it is already scrubbed of secret values (env vault §10); the index never reads a file from disk.

## 3. Staying in step

Triggers on the source tables keep the copies (`search_docs`) in step in the same write: an insert adds the copy, a change of the text replaces it, a delete removes it. No trigger writes to the FTS table itself: a statement whose triggers reach a virtual table reads the table's settings before it takes the write lock, and inside a transaction that fails at once with "database is locked" when another connection wrote meanwhile (a run queued while another run writes its events). So the copies' changes wait in `search_queue`, and the store applies them, removals first, under `BEGIN IMMEDIATE` before each search and when it opens. A copy added and removed before that never reaches the index. A deleted dot's rows go by cascade and their triggers remove the copies. Runs never change dot, so a copy keeps its dot and run; the topic a run is in, a goal's dot and a goal's account are read when searching, so a move, merge or relink needs no reindex. The FTS table runs with `secure-delete`, so the words of a deleted row leave the index pages too. The migration backfills every row stored before it (a one-time cost at the first start after the update, larger with a long run history).

## 4. Query

- What the person typed is split into words the way the index splits them (runs of letters and digits). Each word is quoted, so nothing typed is query syntax: quotes, `-`, `*`, `OR`, `NEAR`, parentheses and `column:` are only separators or plain words. All words must match. The last word also matches words it starts (`lis` finds Lisbon), unless the text ends after it with a space or a mark. At most 12 words count. A text with no word searches nothing.
- Hits are ranked by bm25, newest first among equals. Each has a snippet of about 16 words with every matched word between U+E000 and U+E001; the UI splits on those marks and renders the parts as text (never as HTML).
- The search covers only the dots and goals of the account and workspace in view, and needs the role's View permission there.
- Optional filters: kinds, one dot, and a limit (default 30, at most 100).

## 5. The box

A dialog labelled **Search**, its input a combobox driving a listbox of hits grouped by kind (Topics, You said, Dots said, Notes and questions, Goals, Memory, Files). It searches 200 ms after the last key. Arrow keys, Home and End move through the hits, Enter or a click opens one and closes the box; Escape closes it. It says what to type when empty, "Searching…" while waiting, "Nothing matches" for no hits, and the core's sentence when a search fails.

## 6. Not indexed, and limits

- Tool calls and tool output, raw engine lines, notices, errors, workflow runs and steps, the Library, settings, dot instructions and names, and the text inside attached files (Dots keeps only their names in the database).
- A run's result and its replies fold into one hit, so a long conversation in one run is one hit, not many.
- A word inside another word is not found (`bon` does not find Lisbon); there is no stemming (`trips` does not find trip) and no fuzzy matching.
- A goal opens the Intents screen, not that goal; a memory opens its dot's Memory tab, not that row.
- Words of a deleted row stay in `search_queue` (to take them out of the index) until the next search or start.
- Snippet marks are private-use characters; a stored text that itself holds them would show a stray highlight.
