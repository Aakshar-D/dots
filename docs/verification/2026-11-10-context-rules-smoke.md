# Context rules smoke (2026-11-10)

## What the agent ran with the real CLI

`claude --version` 2.1.322, model `haiku`, in a scratch folder, tiny prompts. No app was
started. The output of A1 and A2 is pinned, with text redacted, in
`crates/dots-core/tests/fixtures/context_turn.jsonl` and `compact.jsonl`.

### A1. A turn's fill and the window

`claude -p --output-format stream-json --verbose --model haiku --setting-sources ""` with
"Say hi in one word." on stdin.

- The `assistant` line's `message.usage`: input 2, cache creation 4,767, cache read 19,541,
  so the fill is 24,310. Every block of one message repeats the same usage.
- `system/init` names `model: claude-haiku-5-5`, no window.
- The `result` line's `modelUsage.claude-haiku-5-5.contextWindow` is 1,000,000.

### A2. Headless `/compact`

`/compact` on stdin, `--resume <session>`, the same flags.

- Lines: `system/status` `compacting`, a `rate_limit_event`, `system/status` with
  `compact_result: success`, a second `system/init` (same session id), `system/compact_boundary`
  with `compact_metadata: {trigger: manual, pre_tokens: 24315, post_tokens: 3520}`, two
  replayed `user` lines (the summary and the command's output), then `result` with
  `subtype: success`, `result: ""`, `local_command: compact`, zero usage.
- In a Git Bash shell an argument `/compact` is turned into a Windows path by MSYS; Dots sends
  the prompt on stdin from Rust, which is not affected. On stdin it works.

### A3. The run's own flags

The flags a Dots run uses, with an MCP config whose `dots` server does not answer:
`--max-turns 40 --permission-prompt-tool mcp__dots__approve --mcp-config <file>
--strict-mcp-config --setting-sources "" --settings <file> --resume <session>`.

- `/compact`: boundary 24,529 → 3,543, `result` success. The dead MCP server did not stop it.
- Then `context_rules::HANDOFF_ASK` with `--max-turns 1` in the compacted session: `result`
  success with a plain-text note, which named the code word the first turn had asked to
  remember. The turn's fill was 27,044.

### A4. `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`

`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=2` and a small prompt in a session at about 5.6% of its 1M
window: before the turn the CLI wrote `system/status` `compacting`, then `compact_result:
failed`, `compact_error: too_few_groups` (the session was too small to compact), and the turn
went on normally. So the CLI reads the variable; a successful automatic compaction writes the
same boundary with `trigger: auto` (parsed; not seen live, it needs a long session).

## What the lead should check in the app

1. A Claude dot, a chat topic. After the first reply the reply box has "Context 25k / 1M (3%)"
   (the window comes from the CLI; haiku and sonnet 5.5 report 1M on this account).
2. Settings › Runs & server › Context: set compact at 10%, start fresh at 20%. Save.
3. Send two more messages. The second is compacted first: the run page lists "Compacted …",
   the transcript has "Conversation compacted: …", and the meter drops.
4. Topic menu › Start fresh, then send a message. The run starts a new session (a notice
   "Started a fresh session with a summary: …"), the reply knows what the topic was about, and
   the topic still reads as one conversation.
5. Set start fresh lower than compact in Settings and in a dot's editor (Context › Use its own
   context rules): the rule shows under the fields and Save is refused.
6. A follow-up or watcher run of a full session starts fresh by itself; a chat shows "Context
   is N% full — start fresh with a summary?" with Start fresh and Keep going.
7. Approve a parked approval in a full session: the resume run is neither compacted nor
   started fresh.
