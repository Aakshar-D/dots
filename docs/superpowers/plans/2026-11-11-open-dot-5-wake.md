# Open Dot Step 5: Wake Phrase, Interrupting, Spoken Notices

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The last behaviours of Open Dot: a soft tone when it starts listening, "Hey Dot" and "Dot, stop" working through the whole app, interrupting by just talking, and Dot telling the person aloud when something needs them.

**Architecture:** Most of the machinery exists (the session already has the wake spotter, the stop phrase and talk-over; the settings already exist and are passed through). This step adds: a tone made in `dots-voice` and played through the same sink as speech; a small pure `notices` decider in the app that listens to the runtime's events and speaks one short line through `Session::notice`; the wiring and tests that prove the wake path end to end with fakes; and the real-audio checks written into the hand-check document.

**Tech Stack:** Rust (`dots-voice`, `apps/dots-app`), a little TypeScript for one setting.

**Spec:** `docs/superpowers/specs/2026-11-11-open-dot-design.md` sections 3.2 ("A soft tone plays and the stage opens"), 3.6 (interrupting), 3.7 (settings), 5.2 ("Tell me aloud when something needs me": one short line, never more than one every 30 seconds, never while the person is speaking), and the "private things" rule.

## Global Constraints

- Wake phrase and "Interrupt by just talking" stay off by default. With the wake phrase on, the microphone is open while Dots runs, the tray says "Dot is listening", and audio never leaves the computer.
- Spoken notices are off by default (`voice_notices`). At most one every 30 seconds. Never while the person is speaking or Dot is mid-turn (only when the session is Idle). Never describe a private approval: "One private approval needs you."
- A notice is one short sentence built by Dots code from the record (dot name, kind of thing), never from model text, and never contains a tool input, a path, or a dot's question text.
- No audio or transcript on disk or in logs.
- No echo cancellation: on speakers, interrupting by voice is best effort; the key and the click always work.
- Gate per task: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; the tests the task names; UI `bun run typecheck` and `bun run test` when UI changes.
- Commits: conventional messages, no co-author trailer, staged by path. Never commit `.cargo/config.toml`. Builders never run anything that opens the microphone or plays sound; real-audio checks are the user's.

## Review Focus

1. A burst of events (ten approvals in a second): exactly one notice, then silence for 30 seconds, then one more that counts what is waiting.
2. A private approval: the spoken line names nothing about it.
3. The person is mid-sentence or Dot is speaking when something arrives: nothing is spoken then; it is spoken (once, counted) when the session next rests, if it is still waiting and 30 seconds have passed since the last notice.
4. Notices off, or voice off or not ready: nothing is spoken and nothing is queued.
5. The tone never plays on a follow-up listen or after an interruption, only when listening starts from rest by hotkey, click, tray or wake phrase; and it is never heard by the ears (the deaf tail applies).

---

### Task 1: The tone

**Files:** `crates/dots-voice/src/tone.rs` (new), `crates/dots-voice/src/session.rs`, `crates/dots-voice/src/lib.rs`, tests beside them; `crates/dots-core/src/settings.rs` (`voice_tone: bool = true`), `tests/settings_test.rs`, `ui/src/api/types.ts`, fixture; `apps/dots-app/src/voice/service.rs` (pass it through); `ui/src/screens/VoiceSection.tsx` (a switch "Play a tone when Dot starts listening").

- `pub fn listen_tone(rate: u32) -> Clip`: two soft sine notes rising a fourth (for example 660 Hz then 880 Hz), 70 ms each with a 15 ms gap, raised-cosine fade in and out of 12 ms, peak 0.2 of full scale; no clicks (first and last samples zero); total under 200 ms. Pure; tests on length, peak, zero ends, and that energy sits at the two notes (a simple Goertzel or correlation check).
- `SessionSettings` gains `tone: bool` (default true). The session plays the tone through the sink when it moves from Idle to Listening because of `open()` or the wake phrase (`Woke`), then opens the ears only after the tone has finished plus the existing `reopen_after_ms` (the tone must not be heard as speech). Not on follow-up listening, not after `interrupt()`, not when `answered()` is used, not when the sink has failed (listening starts at once).
- Tests with the fake clock and fake sink: `tone_on_open_then_ears_after_it`, `no_tone_on_follow_up`, `no_tone_after_interrupt`, `tone_off_opens_ears_at_once`, `woke_plays_the_tone`, `failed_sink_skips_the_tone`.
- Setting plumbing, default on; the Voice tab switch; fixture regenerated.
- Commit `feat(voice): a soft tone when Dot starts listening`.

### Task 2: Wake phrase and stop phrase through the app

**Files:** `apps/dots-app/src/voice/{service,bridge}.rs` tests; `crates/dots-voice/tests/real_engines.rs`.

- Service tests with the fake session: turning `voice_wake_phrase` on applies `SessionSettings.wake_phrase` without a rebuild and the status shows `mic_open` once the session reports it; turning it off closes the microphone when resting; `voice_interrupt_by_talking` likewise.
- Bridge tests: `Woke` while the stage was closed clears the closed flag and the next `Heard` starts a turn; `Woke` during a live turn (Dot speaking, "Hey Dot" said over it) drops the turn like `open()` does; a `State { Listening }` after "Dot, stop" drops the live turn and nothing more is spoken for it.
- A wake does not raise a hidden window (the reply is spoken; the tray shows listening). Document this in the spec's as-built as a call made.
- Real engines (`#[ignore]`, need `DOTS_VOICE_MODELS`): drive the real `Session` with fake microphone frames synthesised by the real speaker engine: "Hey Dot." then a sentence gives `Woke` then `Heard` with that sentence (not including the wake phrase); while a long reply is being spoken (fake sink), "Dot, stop." fed to the microphone stops it; with `interrupt_by_talking` on, any sentence stops it; with it off, a sentence that is not the stop phrase does not. Record hit counts over 10 voices.
- Commit `test(voice): the wake phrase, the stop phrase and talk-over through the session and the app`.

### Task 3: Spoken notices

**Files:** `apps/dots-app/src/voice/notices.rs` (new), `apps/dots-app/src/voice/{mod,service}.rs`, `apps/dots-app/src/lib.rs` (start the listener).

**Interfaces — Produces:**

```rust
pub enum Needs { Approval { dot_name: String, private: bool }, Question { dot_name: String } }
pub struct Notices { /* last_spoken: Option<Instant>, waiting: Vec<Needs> */ }
impl Notices {
    pub fn new() -> Self;
    /// Something new needs the person. Returns the sentence to speak now, if any.
    pub fn arrived(&mut self, need: Needs, resting: bool, now: Instant) -> Option<String>;
    /// The session came to rest. Returns the sentence to speak now, if something is still waiting.
    pub fn rested(&mut self, still_waiting: usize, private_waiting: usize, now: Instant) -> Option<String>;
    pub fn clear(&mut self);                                   // notices turned off, or voice off
}
pub const NOTICE_GAP: Duration = Duration::from_secs(30);
```

- Sentences: one approval "Research needs your approval."; one question "Research has a question for you."; one private approval "One private approval needs you."; several "Three things need you." (count of everything waiting, from the inbox count, not from the burst).
- The listener subscribes to the runtime's events (`rt.subscribe()`, as `events::spawn_forwarder` does): `ApprovalRequested` (private decided with `rt.approval_touches_private`; an error counts as private) and `MessagePosted` with a question. It ignores things for the dot the person is viewing (the same `quiet(dot_id, rt.viewing())` rule desktop notifications use) and anything while `voice_notices` is off or the service is not ready. It speaks through the service (`Session::notice`, which only speaks while Idle); `resting` is the session being Idle with the stage closed.
- When the session returns to Idle after a turn, the service asks `rested(..)` with the current inbox counts.
- Tests (pure, with a fake clock): the five Review Focus items, plus `the_viewed_dot_is_not_announced`, `a_question_and_an_approval_together_are_counted`, `lagged_events_recount_from_the_inbox`. Service test with fakes: a notice reaches `notice()` only when ready, Idle and on.
- Commit `feat(app): Dot says aloud when something needs you, at most once every half minute`.

### Task 4: Gate, hand check, as built

- [ ] Full gate (fmt, workspace clippy, core, voice, app, node tools, UI typecheck and tests).
- [ ] `docs/open-dot-hand-check.md`: add steps for the tone, "Hey Dot" from across the room and with music playing, "Dot, stop" on speakers and on headphones, interrupt by talking on headphones, wake while the window is hidden, notices (one approval; a burst; a private one; while speaking), and a line to write down how often the spoken interrupt works on speakers (spec 3.6 asks for the number).
- [ ] Spec section 10: "### Step 5" (what was built, calls made, deferred, owed). Status line updated. Commit `docs(spec): Open Dot step 5 as built`.
