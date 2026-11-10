# Usage tab smoke (2026-11-10)

Branch `feat/usage-tab`. Claude CLI 2.1.322 installed; no Claude run was needed: the scan reads transcripts that already exist.

## Transcript format (field names only)

Checked on this PC's `~/.claude/projects` (24 project folders, 684 `.jsonl` files, 1.3 GB). `CLAUDE_CONFIG_DIR` not set.

- Files: `<project>/<session>.jsonl` (227) and `<project>/<session>/subagents/agent-<id>.jsonl` (457). A subagent's lines carry the parent's `sessionId`; a top-level file's name is its `sessionId`.
- Line types seen: `assistant`, `user`, `attachment`, `api-request`, `api-request-blob`, `last-prompt`, `queue-operation`, `cost-state`, `system` and others.
- Assistant lines always had `timestamp` (24 characters, RFC 3339 UTC with ms), `sessionId`, `cwd`, `requestId`, `message.id`, `message.model`, `message.usage` with `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens`. `entrypoint` is `cli`, `sdk-cli` or `claude-desktop`.
- Duplicates: of 64,791 assistant lines, 32,183 distinct `(message.id, requestId)`. 509 repeats differed only in `output_tokens` (streaming); 226 repeats were in another file (resumed sessions). One `<synthetic>` model line.

## Scan against the real folder

A throwaway example (not committed) ran `ClaudeUsageService` in a release build against the real folder, with a fresh temporary database, so no Dots session ids were known. Read-only on the transcripts.

- First full read: 37 scans of about 32 MiB each, 41.8 s in all; the slowest single scan 2.7 s.
- A rescan right after: 1 file (a live session), 5,225 bytes, 63 ms.
- Summaries: 5 hours 392 messages in 6 sessions (4 ms); week 7,458 in 63 sessions (28 ms); month 22,216 in 226 sessions, 50 project rows (112 ms).
- Dots' share was 0 here: the temporary database had no runs, and no transcript this month had a cwd inside `%LOCALAPPDATA%\dots`. The split is covered by the core tests.

## Not seen

- The page in the running app (the lead's in-app QA): the rail link, the section, the block line, "Reading Claude Code's transcripts…" during the first read.
- The block line needs a 5-hour reading from a real dot run.
