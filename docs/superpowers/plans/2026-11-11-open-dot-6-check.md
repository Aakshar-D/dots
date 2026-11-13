# Open Dot Step 6: Whole-App Check, Timing, Polish

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the whole chain works together and say how fast it is: a spoken sentence goes in at the microphone end and a spoken, correct reply comes out at the speaker end, through the real session, the real bridge, the real mind and the real runtime, with every row of what Open Dot can do exercised; then one last review of the whole branch and the write-up a person needs to use it.

**Architecture:** One ignored end-to-end test binary in the app crate that assembles the real pieces with only the two ends faked: the microphone is fed speech synthesised by the real speaker engine, the sink records what would be played and the test transcribes it with the real recogniser, and the Claude CLI is the scripted fake. Timings are taken from the events. Nothing opens a real microphone or speaker.

**Tech Stack:** Rust (`apps/dots-app`, `dots-voice`, `dots-core`), the fake Claude CLI, the real speech models at `DOTS_VOICE_MODELS`.

**Spec:** `docs/superpowers/specs/2026-11-11-open-dot-design.md` section 2 (goal 4: about two seconds from the end of the person's sentence to the first spoken word, a target), 3.5 (every row), 7 ("by hand" list), 8 (step 6).

## Global Constraints

- No real microphone, no real speaker, no real Claude CLI in automated tests. The one measurement with the real CLI is a separate, clearly named, ignored test that sends only harmless fixed sentences and prints its cost.
- No audio or transcript on disk or in logs; tests may hold transcripts in memory and print timings.
- Nothing is pushed. Commits: conventional messages, no co-author trailer, staged by path. Never commit `.cargo/config.toml`.
- Gate: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; all suites as in step 5.

## Review Focus

1. The chain as assembled in the test is the chain the app assembles (same factory code path, same bridge, same `MindPort`), not a parallel copy that could drift.
2. A confirm-tier action through the whole chain: spoken "approve it", Dot asks, spoken "okay" gets the clear-yes line, spoken "yes" approves; a spoken "no" approves nothing.
3. Timings are measured from events with the real clock and reported honestly (median and worst of at least five runs), with what was faked said beside them.

---

### Task 1: The whole chain, end to end

**Files:** `apps/dots-app/tests/open_dot_e2e.rs` (new, `#[ignore]`, needs `DOTS_VOICE_MODELS`), small test seams in `apps/dots-app/src/voice/` if the real factory cannot take a fake microphone and sink (prefer a `RealFactory` constructor that takes the two openers, used by the app with the device openers and by the test with fakes; no behaviour change for the app).

- The harness: a temp runtime with the fake CLI (as `crates/dots-core/tests/open_dot_test.rs` does: script file rewritten between turns), dots, a pending approval, a pending question; the real voice service built through the real factory with a fake microphone (frames fed at real-time pace from clips made by `SherpaSpeaker`) and a recording sink (keeps clips; reports idle after each clip's length); voice events collected from the UI channel; a helper `say(text)` that feeds a sentence and `heard_reply()` that waits for the turn to end and returns what the sink was asked to play, transcribed by `SherpaRecognizer`.
- Scenarios, one test each, each asserting the spoken reply (loosely: key words) and the effect in the runtime:
  1. "What needs me?" — the reply counts the approval and the question; a list card event was sent.
  2. "Read that question" then "Tell it yes, use the staging server" — the question's text is spoken; the answer reaches the dot (`reply` made a run); "Answered" is spoken.
  3. "Approve it" — Dot asks with the dot's name and tool; "okay" gets "I need a clear yes for that."; "yes" approves (the approval is decided) and "Approved." is spoken. Separate test: "no" leaves it pending and says "Okay, I won't."
  4. "Ask the research dot to compare the three vendors" — a new topic exists for that dot with that text; "Sent to" plus the name is spoken.
  5. "Run the backup dot now" — asks; "yes"; a run exists.
  6. "Set up a dot that checks my inbox every morning" — the setup agent's question is spoken, an answer is given, the proposal card event arrives, "create it" creates the dot.
  7. "Open the research dot" — an `open` event with that dot.
  8. "Say that again" — the last reply is spoken again. "Never mind" — "Okay." and nothing pending.
  9. A click instead of speech: the service's `pick(index, seq)` with the seq from the `ask` event approves; a pick with an old seq does nothing.
  10. Interrupt: while a long reply is being played, `open()` stops it and the next sentence is heard.
  11. Wake: with the wake phrase on, "Hey Dot, what needs me?" in one breath is answered.
  12. Voice off: `enable(false)` then a fed sentence produces no event and the microphone opener was not called again.
- The fake CLI scripts answer by prompt content (`when_prompt_has`), so each scenario's model reply is the `DO {...}` line a model would give.
- Timings recorded per turn from event times: end of the fed sentence to `heard`; `heard` to the first `saying`; first `saying` to the first clip reaching the sink; printed as a table with median and worst over the scenarios. The fake CLI answers at once, so these are Dots' own overheads.
- [ ] Run: `DOTS_VOICE_MODELS=C:\dev\dots-voice-models cargo test -p dots-app --locked --test open_dot_e2e -- --ignored --test-threads=1`, three times; record pass counts and the timing table.
- [ ] Commit `test(app): the whole of Open Dot, spoken sentence in, spoken reply out`.

### Task 2: Time with the real model (one measurement)

**Files:** `apps/dots-app/tests/open_dot_e2e.rs` (one more ignored test, named `real_cli_timing`, run only by name).

- Same harness, real Claude CLI found as the app finds it, model `haiku`, an empty runtime (no dots, nothing private), five turns of harmless questions ("What needs me?", "What can you do?", "Say hello.", "What time is it?", "Never mind." — the last needs no model). Print per turn: end of sentence to `heard`, `heard` to first `saying`, first `saying` to first clip, and the total from end of sentence to first sound; then median and worst. Print the usage rows recorded (count and cost).
- [ ] Run it once, by name. Put the numbers in the spec's as-built, against the two-second target, with what would move them (the kept CLI process measured in step 2: about 0.5 s).
- [ ] Commit `test(app): how long Open Dot takes with the real model`.

### Task 3: Final review of the whole branch

- [ ] Package `git diff <merge base ad917b31>..HEAD`; review on the most capable model with the ledgers' deferred items and rulings; one fix round; a scoped re-review. (Run by the controller, not by a builder.)

### Task 4: What a person reads

**Files:** `docs/open-dot.md` (new: what Open Dot is, how to turn it on, what you can say with the table from spec 3.5 in plain words, how confirmations work and what counts as a yes, privacy in five lines (on this computer; nothing saved; what is sent to the model; private approvals), the settings, troubleshooting (microphone blocked, hotkey taken, download interrupted, no sound)), the repo's README or docs index if it lists features (add one line and a link), the release notes file if the repo keeps one (find how past features were announced and follow it), spec section 10 "### Step 6" (the end-to-end results, the timings, the final review's outcome, everything still owed in one list), status line: all six steps built; owed: the hand check and CI.
- [ ] Full gate once more.
- [ ] Commit `docs: how to use Open Dot, and step 6 as built`.
