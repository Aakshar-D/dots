# Context rules plan

**Spec:** `docs/superpowers/specs/2026-11-10-context-rules-design.md`.

**Branch:** `feat/context-rules`. Migration `0044_context_rules.sql` (master's last is 0042; the
three parallel branches take 0043, 0044 and 0045 in merge order).

## Tasks

1. Parser: `EngineEvent::Context` and `EngineEvent::Compacted` from assistant usage,
   `system/init`, the result's `modelUsage` and `compact_boundary`. Tests on the pinned
   fixtures.
2. `context_rules.rs`: `ContextRules` (defaults, bounds), `window_for`, `decide` (background,
   chat, choices, off), `handoff_seed` with `SeedSection`. Unit tests.
3. Store: migration, `runs.context_fill/window`, `dots.context_rules`, `context_threads`,
   `context_events`; `Settings::context_rules`. Store tests.
4. Engine: `Engine::compact` and `Engine::handoff_summary` (default none), the Claude engine's
   one-off CLI pass, `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`; the scripted engine's stand-ins.
5. Runner: record the fill, decide before the engine starts, compact or start fresh, record
   the rows. Runner tests with the scripted engine (background, chat, resume untouched,
   keep going, failures).
6. Runtime and app: `thread_context`, `thread_context_choose`, `run_context_report`; Tauri
   commands; contract samples.
7. UI: types, commands, `ContextMeter`, the topic's question, topic menu items, run page,
   Settings and dot editor fields. Vitest.
8. Gates, smoke with the real CLI, PR.
