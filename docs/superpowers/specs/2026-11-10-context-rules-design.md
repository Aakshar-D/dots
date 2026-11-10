# Context meter and compact/clear rules

- Date: 2026-11-10
- Status: built (plan `docs/superpowers/plans/2026-11-10-context-rules.md`)
- Builds on: the Claude engine (`crates/dots-core/src/engine/claude/`), the runner (`runner.rs`),
  sessions continued with `--resume` (chat replies, check-in answers, follow-ups, workflow
  steps), the fresh start of a moved topic (`2026-11-04-topic-tools-design.md` section 4.4),
  dot memory (`memory.rs`), `2026-10-26-usage-limits-design.md`.

## 1. Intent

A chat topic or a follow-up resumes the same Claude session, so every turn sends the whole
conversation again. A long topic carries 150k tokens into each reply and uses the person's
subscription fast. This feature shows how full a session's context is and keeps it small:
it **compacts** a session (the CLI's own `/compact`) when it is fairly full, and **starts
fresh** (a new session, seeded with a summary) when it is very full.

Success: a topic reaches 62% of its 200k window; the next message first compacts it to 18%
and the meter says so; the run page lists "Compacted 124k → 36k". A follow-up whose session
is at 85% starts a new session told what the old one was doing; the topic still reads as one
conversation.

## 2. What the CLI gives (checked with CLI 2.1.322 on 2026-11-10)

Pinned as fixtures in `crates/dots-core/tests/fixtures/context_turn.jsonl` and
`compact.jsonl` (text redacted).

- Every `assistant` line has `message.usage`. `input_tokens + cache_creation_input_tokens +
  cache_read_input_tokens` is what that turn sent: the context fill. Lines of a `Task` helper
  (`parent_tool_use_id` set) are the helper's own context and are ignored.
- `system/init` has `model` (`claude-haiku-5-5`). A model id with `[1m]` has a 1M window.
- The `result` line has `modelUsage.<model>.contextWindow` (1,000,000 for haiku 5.5 on this
  account). It is the most reliable window, but it comes only at the end of a run.
- **Headless `/compact` works.** `claude -p --resume <id> --output-format stream-json
  --verbose` with `/compact` on stdin compacts the session in place (same session id). It
  writes `system/status` `compacting`, then `system/compact_boundary` with
  `compact_metadata: { trigger: "manual", pre_tokens, post_tokens }`, and a `result` with
  `subtype: success`, `result: ""`, `local_command: "compact"` and zero usage. A small test
  session went 24,315 → 3,520 tokens.
- **`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` works.** With it at 2 the CLI started an automatic
  compaction before the turn (`system/status` `compacting`); that session was too small to
  compact (`compact_result: failed`, `compact_error: too_few_groups`) and the turn went on
  normally. An automatic compaction that succeeds writes the same `compact_boundary` with
  `trigger: "auto"`.

## 3. The meter

- The parser emits `EngineEvent::Context { fill, window }`: `fill` from each main-thread
  assistant line, `window` from `system/init` (only for a `[1m]` model) and from the result's
  `modelUsage` (the largest `contextWindow` it names). It is not stored as a run event; the
  runner writes it to the run (`runs.context_fill`, `runs.context_window`, migration 0044)
  and announces the run.
- The window shown: the run's stored window, else 1,000,000 for a `[1m]` model id, else
  200,000 (`context_rules::window_for`).
- A session's fill is its newest run's fill. A topic's meter is the newest run of the topic
  that has one.
- Shown as "Context 92k / 200k (46%)" on a topic (above the reply box) and on the run page.
  Amber from the compact threshold, red from the clear threshold. A run with no fill (a
  local-model run, a run from before this feature) shows no meter.

## 4. Rules

`ContextRules { enabled, compact_at, clear_at }`: on, 60%, 80% by default. Settings › Runs &
server › Context holds the app-wide rules (`Settings::context_rules`); the dot editor can
override them for one dot (`DotSpec::context_rules`, `dots.context_rules` JSON, `null` =
Settings). Each threshold is 10 to 95%, and clear must be above compact; the core checks this
and the UI shows the rule beside the fields.

The rules apply to a Claude run that continues a session (`session_id` set), before its
engine starts. They never apply to a `resume` run (it must finish the approved calls in the
session as it was), a local-model run, or a topic that a move or merge is already starting
afresh. `pct = fill × 100 / window`, of the session's last fill.

| Run | Rules on, pct ≥ clear | Rules on, pct ≥ compact | Asked by the person |
|---|---|---|---|
| Background (anything that is not `chat`) | start fresh | compact | — |
| Chat | compact, and the topic asks "start fresh?" | compact | Compact now / Start fresh |

- Inside a run, while the rules are on, the CLI gets `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` =
  `compact_at`, so a long run compacts itself at the same share. Its `compact_boundary`
  (`trigger: auto`) is recorded like Dots' own.
- A person's choice (section 6) applies to the topic's next turn whether the rules are on or
  off, and is used up by it.

## 5. Compact and start fresh

**Compact.** The engine runs the CLI once more before the turn, with the run's own arguments,
`--resume <session>` and `/compact` on stdin (`ClaudeEngine::compact`). Its
`compact_boundary` gives the fill before and after; the run's fill becomes the after. If it
fails or writes no boundary, the run gets the notice "Compacting the conversation failed: …;
it goes on as it was." and the turn goes ahead.

**Start fresh.** First the old session is asked for a hand-off, again with the run's own
arguments and `--resume <old>`, with `context_rules::HANDOFF_ASK` (what the conversation is
about, decisions, open work, files and names that matter; at most 300 words). Its answer, cut
to 4,000 characters, is the summary; a failure leaves the summary out, with a notice. Then
the run starts with no session, so its prompt is a first prompt: instructions, global
instructions, **the dot's memories**, recent runs, and a "## Where this conversation left
off" section built by `context_rules::handoff_seed`:

1. the summary,
2. the topic's last 6 messages and replies, each cut to 300 characters
   (`prompt::conversation_so_far`),
3. any extra sections (`SeedSection`, from `Runner::extra_seed_sections`). Nothing adds one
   yet. This is the place for a later "project map".

The run stays in its topic (`root_run_id`, `parent_run_id` unchanged), so the topic reads as
one conversation. The new session id is stored on the run as usual; the record of the start
(section 7) keeps the old session id, the link from new to old.

## 6. In a chat topic

- Above the reply box: the meter, and, when the rules are on, the topic's last fill is at or
  above the clear threshold, and the person has not answered: "Context is 82% full — start
  fresh with a summary?" with **Start fresh** and **Keep going**.
- **Start fresh** (`thread_context_choose(root, fresh)`): the topic's next message starts a
  fresh session (section 5). The line then says "Your next message starts a fresh session with
  a summary."
- **Keep going** (`keep_going`): hides the question until the topic's fill has gone under the
  clear threshold (the runner clears the mark when a turn starts or a compaction ends under it)
  and crosses it again. The next message is compacted as usual.
- The topic menu has **Compact now** and **Start fresh**: both apply to the next message
  (nothing is sent by the menu itself, so no turn is spent with nothing to say).
- State: `context_threads (root_run_id, keep_going, request)`.

## 7. Savings

Each compaction and fresh start is a row of `context_events`: run, dot, topic, `kind`
(`compact`, `clear`), `trigger` (`auto` by the rules or the CLI's own, `manual` by the
person), fill before and after (`after` is unknown for a start until its first turn), the old
session, and the time. A compaction is also stored as an `EngineEvent::Compacted` run event,
so the run's transcript shows it.

The run page lists the run's rows ("Compacted 124k → 36k", "Started fresh at 168k") and the
dot's totals ("This dot: 4 compactions, 1 fresh start").

## 8. Not covered

- The local engine reports no per-turn fill, so it has no meter and no rules.
- No token estimate of what a compaction saved over the rest of a topic; the before and after
  are shown as they are.
- The hand-off and the compaction are not counted in the run's tokens or cost; the CLI's
  result for them is not read for usage.
- Compact now does not compact a session while nothing is sent.
