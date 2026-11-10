# Usage tab: all of the person's Claude use, not only the dots'

- Date: 2026-11-10
- Status: approved for build (plan `docs/superpowers/plans/2026-11-10-usage-tab.md`)
- Builds on: `2026-10-26-usage-limits-design.md` (sections 5 and 12: the limit readings, the Usage page, the rail bars).

## 1. Intent

The Usage page shows the account's two limit windows and Dots' own runs. But the person's own Claude Code sessions burn the same subscription, and the page cannot say how much. Nor can the person find the page: it opens only from the thin meter bars at the bottom of the rail.

This phase does three things:

1. **A Usage entry in the rail**, like Home or Settings. The meter bars stay.
2. **All Claude use** on the Usage page: what Claude Code's own transcripts on this PC say, split into "Dots" and "Your own sessions", per period and per project, with a count for the current 5-hour block.
3. **Cheap to keep up**: the transcripts are read incrementally, and what was read is kept in Dots' database.

Success: the person clicks Usage in the rail; under the dots' table, "All Claude use" says this week was 4,210 messages in 63 sessions, 3,900 of them their own and 310 the dots'; the project table shows `dots` (their repo) first; next to the 5-hour window it says "This block: 212 messages · 1.4M tokens since 10:10". Opening the page again a minute later reads only the bytes appended since.

## 2. Goals, non-goals, anti-goals

Goals: a rail entry; read Claude Code's local transcripts read-only; dedupe streamed and copied messages; tell Dots' messages from the person's own; totals, messages, sessions and a project table per period; a count for the current 5-hour block; an incremental scan with a cap per pass; never block the UI.

Non-goals: charts (a per-day list is left for later); costs in dollars for the person's own sessions (the CLI writes no cost to the transcript, and a price table would go stale); other machines' use; claude.ai chat use; editing or deleting transcripts.

Anti-goals: any call to Anthropic's API; reading the CLI's credentials or settings; writing to `~/.claude`; a message counted twice (once as Dots', once as the person's); the app stalling while a large folder is read; the first scan of a 1 GB folder holding the database for minutes.

## 3. Where the transcripts are

Claude Code writes one JSON line per event to `<config>/projects/<encoded cwd>/<session id>.jsonl`, and a subagent's lines to `<config>/projects/<encoded cwd>/<session id>/subagents/agent-<id>.jsonl`. `<config>` is `CLAUDE_CONFIG_DIR` when set, else `~/.claude` (`USERPROFILE`, then `HOME`).

Checked against this PC's files (CLI 2.1.x, field names only): an assistant line has `type: "assistant"`, `timestamp` (RFC 3339 UTC with milliseconds), `sessionId`, `cwd`, `requestId`, `isSidechain`, `entrypoint` (`cli`, `sdk-cli`, ...), and `message` with `id`, `model` and `usage` (`input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens`, and others not used). A subagent's lines carry the parent's `sessionId`. Other line types (`user`, `attachment`, `api-request`, ...) are skipped.

- **Duplicates.** About 40% of assistant lines repeat an earlier `(message.id, requestId)`: a streamed reply is written once per content block, and a resumed session copies earlier lines into its new file. Usually the usage is identical; sometimes only `output_tokens` grew. A message is kept once per `(message.id, requestId)` across all files, with the largest `output_tokens` seen.
- **Skipped:** a line that does not parse, a line with no `message.usage`, no `message.id` or no `timestamp`, and `model: "<synthetic>"` (the CLI's own error notes, no tokens). A missing `requestId` is stored as empty.

## 4. Dots or the person

Every dot's Claude run writes transcripts too. A message is **Dots'** when either is true:

1. its `sessionId` is a session Dots stored on a run (`runs.session_id`), checked when the page asks, so a session stored after the scan still counts; or
2. its `cwd` is inside Dots' data folder (worktrees, result checks, skill drafts and other one-off CLI calls start there). Checked when the line is read, and kept with the message.

Also, when the line is read, a message whose session is already on a run is marked Dots' and keeps that mark, so a dot deleted later (its runs go with it) does not move its messages to the person.

Everything else is **Your own sessions**. Each message is in exactly one of the two. `entrypoint` is not used: the person can run `claude -p` too.

## 5. Scanning

- **What is remembered per file** (table `claude_transcript_files`): path, size, modification time, the byte offset read up to, and a hash of its first bytes read (at most 4 KiB). A file whose size equals the offset and whose modification time is unchanged is skipped without opening. A file smaller than its offset, or whose first bytes no longer match the hash (rewritten), is read again from the start; the dedupe keeps it from counting twice.
- **Only complete lines**: reading stops before a last line with no newline yet; it is read next time.
- **Only what can matter**: lines without `"assistant"` and `"usage"` are skipped before JSON parsing. Messages older than 35 days are not stored, and a file last modified more than 35 days ago is marked read without opening. Stored messages older than 35 days are deleted at each scan.
- **Cap**: at most 32 MiB of new bytes per scan; files are taken newest first, so the current block is right first. A scan that stops at the cap says so (`more`), and the page asks for the next scan at once.
- **When**: when the Usage page opens, then every minute while it stays open (not again within 30 seconds of the last scan, which answers instead, so a background scan shows up on an open page), and in the background every 10 minutes, the first 2 minutes after start. One scan at a time; a second asker waits for the running one and gets its result.
- **Off the UI thread**: file reading runs on Tokio's blocking pool, database writes in one transaction per file; the commands are async.
- A file that cannot be read is skipped (logged at debug) and tried next scan.
- Read-only: files are opened for reading; nothing under `<config>` is written, moved or locked.

## 6. What it shows

**Rail.** A Usage link (gauge icon) before Settings, with the same look and keyboard handling as the other links. The meter bars stay below the links.

**Top of the Usage page.** Next to the 5-hour window: "This block: 212 messages · 1.4M tokens since 10:10". The block is the 5-hour window the CLI last reported: it started 5 hours before its `resets_at`. Without a known 5-hour reading the line is left out. It counts all messages (Dots' and the person's) since the block started.

**All Claude use** (a section under the dots' table, in each period tab, so the same period switch drives it):

- Three totals: Your own sessions, Dots, All. Each shows messages, sessions, tokens and cache reads.
- **Tokens** are `input + cache creation + output`. Cache reads are shown on their own ("cache reads 48M"): they are by far the largest number and are cheap, and folding them in would hide the rest.
- A project table, heaviest first by tokens: project (the cwd's folder name; the full path on hover and as the cell's description), messages, sessions, tokens, cache reads. Messages with no cwd are one row, "(unknown folder)". All Dots' messages are one row, "Dots (all dots)", so the dots' many worktree folders do not crowd the table. At most 50 project rows.
- "Read 3 min ago from C:\Users\me\.claude\projects" under the table; "Reading Claude Code's transcripts…" while a scan goes on.
- **Empty states:** no `projects` folder: "No Claude Code transcripts on this PC (looked in …)". A folder with nothing in the period: "No Claude Code use in the last 7 days."

## 7. Commands and contract

- `claude_usage(period)` returns `ClaudeUsage`: `period`, `since`, `folder` (the projects folder, or null), `available`, `scanned_at`, `own`, `dots`, `projects`, `block`.
- `claude_usage_scan()` returns `ClaudeScan`: `scanned_at`, `files`, `bytes`, `more`.
- The contract fixture gains both samples.

## 8. Storage

Migration `0043_claude_usage.sql`. The shared brief reserved `0030` for this PR, but master already has `0030_delivery.sql` to `0042_audit.sql`, and sqlx refuses two migrations with one number, so this phase takes the next free one.

- `claude_transcript_files(path PRIMARY KEY, size, mtime_ms, offset, head_hash, scanned_at)`.
- `claude_messages(message_id, request_id, ts, session_id, cwd, model, input_tokens, cache_creation_tokens, cache_read_tokens, output_tokens, dots, PRIMARY KEY (message_id, request_id))`, index on `ts`.
- Index `runs(session_id)` for the Dots check.

## 9. Testing

- Core, with fixture `.jsonl` files in a temporary folder: the parser (fields, skipped lines, synthetic model); dedupe within a file and across files, with the largest `output_tokens`; an append read from the stored offset only; a partial last line read next time; a rewritten (smaller) file; the cap and `more`; old files and old messages left out; Dots' split by stored session, by data folder, and by a session stored after the scan; no double count; the block count; no folder.
- UI (Vitest): the rail's Usage link (name, active state, keyboard); the section's totals and project table, the path on hover, the Dots row; the empty state with no folder; the block line; the scan is asked when the page opens.

## 10. Accessibility

The rail link has the name "Usage" like the others. The section is a labelled region with a heading; the project table is a real table with a caption; the full path is the name cell's `title`. Numbers never rely on colour.
