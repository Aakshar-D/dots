# Open Dot, the voice assistant: design

- Date: 2026-11-11
- Status: steps 1 (ears and voice) and 2 (the mind) are built; steps 3 to 6 are not
- Builds on: `2026-10-08-dots-design.md` (dots, runs), `2026-11-03-setup-agent-design.md` (describe a task, get a dot), `2026-10-22-needs-you-design.md` (approvals and questions), `2026-10-27-conversational-chat-design.md` (chat topics), `2026-11-04-topic-tools-design.md` (topics), `2026-10-22-deep-teal-design.md` (colours).

## 1. Intent

The person's words: "Personal agent like open jarvis on dots ... one stop solution ... u can talk to it and make things work crazy good." "A person who has no interest in doing anything when using this app ... can use it to create dots, create threads for anything and everything that they want to do, and it's all voice commanded and there is someone who talks back and tells you exactly what it needs from you." "It should have an animation, a cool animation on top of it that is not copied. That should be our brand value, which is like a breathing animation ... when it communicates back to me, it should feel good, premium." "I command it to do something, it rewards back to me with options." "Hooked with my Claude CLI or my Codex CLI ... when I hit it, it does the thing for me."

Open Dot is one voice you talk to that runs the rest of Dots for you. You say what you want; it answers aloud, asks for what it is missing, offers choices, and does the work through the dots you already have or new ones it sets up.

### What was learned from OpenJarvis

OpenJarvis (Apache-2.0) was read from source on 2026-11-11. No code is copied. Its voice side is a small add-on: click to record, the whole clip is transcribed after you stop, the whole reply is synthesised before any sound, the microphone is off while it answers, there is no wake word, no interrupting, and no visual beyond a red pulsing button.

Ideas taken:

- Speech runs on the person's machine first.
- One player for the whole app, so two replies never talk over each other.
- Text is cleaned before it is spoken (no markdown, links, code or emoji read aloud).
- Actions are sorted by risk; risky ones are confirmed, and a failed check refuses instead of running.
- A disabled microphone always says why.

Weaknesses Open Dot must not share: waiting for the whole reply before any sound; no way to interrupt; spoken requests that still need a button press; confirmations that print raw arguments; approvals found by polling.

## 2. Goals, non-goals, anti-goals

Goals:

1. Hands-free: start by hotkey, by clicking the mark, from the tray, or by saying "Hey Dot". Speak; it replies aloud. No typing needed for any supported action.
2. It does real work in Dots: sets up a new dot from a spoken description, starts a topic with a dot, sends a message to a dot, tells you what needs you, reads out and answers a dot's question, approves or denies an approval, runs a dot now, opens a screen, and reports what is going on.
3. It asks back. When it needs a choice it speaks a short question and shows up to four options; you answer by voice, click, or number key.
4. It feels fast: The Breath reacts the instant you stop talking (a hard requirement), and it aims to start speaking about two seconds later on a normal reply (a target, measured and written into "As built"; quick answers such as yes, no and stop are spoken in under half a second).
5. You can stop or interrupt it at any time.
6. The look is our own: the three dots of the Dots mark, breathing.
7. Speech never leaves the machine. No new accounts or keys: the thinking uses the Claude CLI sign-in that dots already use.
8. Works on Windows and Mac release builds.

Non-goals (this project):

- Languages other than English. (The voice supports 31; the on-device recogniser's other languages are non-commercial. A later spec.)
- Controlling other apps or the operating system.
- A Codex CLI engine. Dots has no Codex engine today; Open Dot uses whichever engines Dots has, so it gains Codex when Dots does.
- Cloud speech services.
- Thinking with a local endpoint. Dots has no one-shot local request and no "default engine" setting; without the Claude CLI, Open Dot says what is missing and stays off. (Later spec.)
- Echo cancellation (see 3.6).
- A phone version.
- A floating always-on-top mini window. The stage lives in the main window; the hotkey brings the window forward. (Later spec.)

Anti-goals (these would make it a failure even if it works):

- It approves, creates, or runs something the person did not clearly agree to.
- The microphone is open without a visible sign, or audio is stored or sent anywhere.
- It reads long text, code, or links aloud.
- It looks like any other assistant's orb.
- It talks over itself, or keeps talking after "stop".

## 3. What the person sees and hears

### 3.1 First time

Settings gains a **Voice** tab. Turning on "Open Dot" explains in two lines what will be downloaded (about 260 MB, once, kept on this computer) and shows progress. When it finishes, the mark in the rail comes alive and a short spoken hello plays in the chosen voice. If the microphone is blocked by the system, the tab says so and how to allow it.

### 3.2 Starting it

- Click the Open Dot mark at the bottom of the rail.
- Global hotkey, default `Ctrl+Shift+Space` (`Cmd+Shift+Space` on a Mac), changeable. If the window is hidden it comes forward.
- Tray menu: "Talk to Dot".
- Saying "Hey Dot", only if the person turned that on (off by default).

A soft tone plays and the stage opens.

### 3.3 The stage

A panel that rises from the bottom centre of the window, above whatever screen is showing. It holds:

- **The Breath** (section 4).
- **Live words**: what it hears, appearing as you speak; then what it says, one line at a time. Small, quiet type. Nothing else scrolls.
- **Options**, when it asks: up to four pills, numbered 1 to 4.
- **A card**, when it helps to see something: the proposed dot (name, what it will do, schedule), the approval being decided (dot, action, target), or a short list (what needs you).
- A close control and a "why is the mic off" line when it is.

`Esc` closes the stage and stops any speech. Closing never cancels work already started in a dot.

### 3.4 A turn

1. Listening: you speak. It hears the end of your sentence by itself (no button).
2. Thinking: The Breath changes at once; if thinking takes more than about 1.5 seconds it says a short filler line once ("One moment.").
3. Speaking: it answers in one to three short sentences. Anything longer goes on the card and it says the gist.
4. It then either listens again (when it asked something, or for 6 seconds after an answer so you can follow up without the wake phrase) or goes quiet. The microphone opens for this only once its own sound has fully finished playing plus a quarter of a second, so it never hears the tail of its own reply.

### 3.5 What it can do

| You say something like | It does | Asks first? |
|---|---|---|
| "What needs me?" / "What's going on?" | Reads the count, then the first items, shortest first | No |
| "Read that question" / "Tell it yes, use the staging server" | Reads a dot's question; sends your answer | Sends after repeating your answer back only if it was unsure of the words |
| "Approve it" / "Deny that" | Decides an approval | Always: it says which dot, what action, on what, then waits for a clear yes |
| "Set up a dot that checks my inbox every morning" | Runs the setup agent, asks its follow-ups aloud, shows the proposal | Always, on the proposal |
| "Ask the research dot to compare the three vendors" | Sends a message as a new topic, or into the topic you are looking at if you say "here" | No, but it says what it sent and to whom |
| "Run the backup dot now" | Starts a run | Yes |
| "Open the research dot" / "Show settings" | Navigates | No |
| "Stop" / "Never mind" / "Cancel" | Stops speaking and drops the pending step | No |
| "Say that again" | Repeats its last reply | No |

If it does not know which dot you mean it asks, with the closest names as options. If you ask for something it cannot do, it says so in one sentence and offers the nearest thing it can do.

### 3.6 Interrupting

- `Esc`, the hotkey, or clicking The Breath stops speech at once and listens.
- Saying "Hey Dot" or "Dot, stop" while it speaks stops it. On headphones this is dependable. On speakers it is best effort, because there is no echo cancellation: its own voice can mask yours. The key and the click always work, and that is the bar for release; how well the spoken interrupt does on speakers is measured by hand and written into "As built".
- Setting "Interrupt by just talking" (off by default; meant for headphones): any speech stops it. It is off by default because on speakers its own voice would trigger it.

### 3.7 Settings (Voice tab)

On/off and download state; voice (ten to choose from, each with a play button); speaking speed; hotkey; "Hey Dot" on/off with a plain note that the microphone stays on while Dots is running and that audio stays on this computer; "Interrupt by just talking"; microphone and speaker choice; which model thinks (default: the small one); "Tell me aloud when something needs me" (off by default); remove downloaded voice files.

Whenever the microphone is open, the rail mark shows it, and the tray icon tooltip says "Dot is listening".

## 4. The Breath

Our own mark, not an orb. It is the three dots of the Dots logo (yellow `#f2c94c`, blue `#56ccf2`, green `#6fcf97`) drawn as soft bodies that can touch and melt into each other, on the deep teal surface.

| State | What it does |
|---|---|
| Resting | The three dots sit in the logo's arrangement and breathe: a slow swell and settle, about one breath every 4.5 seconds, slightly out of step with each other |
| Listening | They lean apart and open up; each one swells with your voice, the yellow one most. Silence brings them back to a shallow breath |
| Thinking | They circle one another slowly and stretch toward each other, touching and parting |
| Speaking | They melt into one shape that pulses with its own voice, the three colours still visible as currents inside it; between sentences it loosens |
| Asking | They part into as many lobes as there are options and hold |
| Done | One quick settle back to the logo arrangement |
| Trouble | They dim and sit still; no shaking, no red flashing |

Rules: motion follows the real sound level, never a canned loop while sound is present; every change between states is a continuous morph (no cuts) of 250 to 400 ms; with "reduce motion" on, the dots hold still and only brightness changes. The same mark, small and resting, is the rail button; it breathes there only while the microphone is open.

Built with SVG and a blur-and-threshold filter driven by a few numbers updated each frame. No animation library is added.

## 5. How it is built

Three parts, each testable alone.

### 5.1 `crates/dots-voice` (new): ears and voice

Native, on-device, through the `sherpa-onnx` crate (Apache-2.0, statically linked; ready builds exist for Windows x64, Windows ARM64 and Mac universal). Measured on the development machine (Windows ARM64) on 2026-11-11:

| Job | Model | Licence | Result |
|---|---|---|---|
| Speak | Supertonic 3 (int8), 44.1 kHz, 10 voices | MIT | 0.37 s for a short sentence; about 6 times faster than real time; no skipped words |
| Hear | Moonshine base English (quantised) | MIT | a 5 s sentence in about 0.1 s |
| Find speech | Silero VAD | MIT | (bundled with the library's examples; tiny) |
| Wake phrase | Zipformer keyword spotter, 3.3 M | Apache-2.0 | "Hey Dot": 30 of 30 synthetic tests right, near-misses ignored, about 3% of one core |

Rejected by measurement: Kokoro (skips any word missing from its word list, for example "created"); Pocket TTS (non-commercial); speech in the web view (voice 3 to 5 times slower than real time without a capable graphics card).

Pieces:

- `models`: a manifest of files (URL, size, SHA-256, unpacked layout) and a downloader that resumes, verifies every file, unpacks to `<data dir>/voice/models`, and reports progress. Nothing is loaded unless its hash matches.
- `audio`: microphone in and speaker out through `cpal`; resampling to what each model wants; a level meter (about 30 readings a second) for both directions.
- `ears`: speech detection, a short pre-roll so the first word is not clipped, end-of-sentence detection (about 0.7 s of quiet, shorter after a yes/no question), live partial words by re-reading the growing clip a few times a second, and the final words.
- `voice`: turns text into sentences, cleans them for speech, synthesises sentence by sentence and plays them back-to-back so sound starts after the first sentence; one queue for the whole app; stop is immediate.
- `wake`: the keyword spotter on the open microphone, for "Hey Dot", and "Dot stop" while speaking.
- `session`: the state machine that ties these together: `Off, Idle, Listening, Thinking, Speaking, Asking`, with the events the app needs. Engines sit behind traits so the state machine is tested with fakes and recorded clips.

While it speaks, the microphone clip is not sent to the recogniser unless "Interrupt by just talking" is on; only the keyword spotter listens.

### 5.2 `dots-core::open_dot` (new): the mind

Kept outside runs, like the setup agent: it is not a dot, makes no run, and shows nothing in the timeline. What it does appears where it always would (a new dot, a message in a topic, a decided approval).

One turn:

1. **Quick answers, no model**: a small matcher handles stop, cancel, repeat, yes, no, and picking an option by number, position ("the second one") or name. These must feel instant and must not depend on a model being reachable.
2. **Otherwise one model request** to the Claude CLI, with no tools, no MCP servers and no settings files, exactly as the one-shot path (`engine/claude/once.rs`) does it. It is given: how to behave, the list of actions, a compact picture of now (dots with status, what needs the person with ids, the screen and topic being viewed, the time), and the last few turns. It answers with the words to say, then an optional block naming one action or one question with options.
3. **The action is checked and carried out by Dots code, not by the model**: names are matched to real dots, ids must be ones from the picture it was given, and anything that does not check out becomes a spoken question instead of an action.
4. **The result is spoken** from fixed, short sentences where possible ("Sent to Research."), so most turns need one model request.

Confirmation rules, enforced in code:

- No confirmation: reading, reporting, opening a screen, repeating.
- Do, then say: sending a message, answering a dot's question.
- Confirm first, always: approving or denying, creating a dot, running a dot now. The confirmation is built by Dots from the real record (dot name, action, target), not from the model's wording. Only the person can pass it: a clear spoken "yes" caught by the quick matcher, or a click or key press on the Yes option. Nothing the model writes can pass it. Silence, an unclear reply, or anything else is "no".
- An approval the person may not decide (managed policy, wrong account) is refused with the reason.

Creating a dot reuses `setup_ask` and `setup_apply` unchanged: its follow-up questions are spoken one at a time (at most two rounds, as today), the proposal is shown on the card and summarised aloud, and a yes applies it.

Thinking model: the small one by default (`haiku`), changeable. Spoken replies are capped in length by instruction and by a hard trim.

Speaking sooner. Measured on 2026-11-11 with the small model: a fresh CLI process gives its first words about 2.3 s after it is started, of which about 1 s is the process starting. So:

- The request asks for partial output (`--include-partial-messages`; a new `stream_event` case in the parser), and the first full sentence is spoken while the rest arrives.
- One CLI process is started when the stage opens and kept for the conversation (`--input-format stream-json`), so later turns skip the start-up second, and the conversation's earlier turns do not have to be resent. It is ended when the stage closes or after 10 minutes idle.
- If the kept process cannot be made dependable, the fallback is one process per turn with the last turns resent; everything else is unchanged. The plan measures both before building on either.

Which workspace: Open Dot acts in the account and workspace the app is showing, the same ones the screen passes to `setup_apply`. The picture lists that workspace's dots only.

Private things: the picture and the spoken words follow the rule the app already has. An approval the runtime marks as touching private data (`approval_touches_private`) is counted but not described to the model or read aloud: it says "one private approval needs you" and opens it on screen. Vault values never enter the picture.

Usage: every model request Open Dot makes is recorded like any other send and shows in the Usage tab under "Open Dot", and counts against spend caps. At a cap it says so and stops asking the model; quick answers still work.

"Tell me aloud when something needs me" listens to the existing runtime events (approval requested, question posted) and speaks one short line, never more than one every 30 seconds, never while the person is speaking.

Nothing spoken or heard is saved. The turn list lives in memory and is dropped when the stage closes or after 10 minutes idle.

### 5.3 App and UI

- `apps/dots-app`: owns the voice session; new commands (`voice_status`, `voice_enable`, `voice_download_cancel`, `voice_open`, `voice_close`, `voice_stop`, `voice_pick`, `voice_preview`, `voice_devices`) and one new event stream (`voice`: state, levels, heard words, said words, options, card, download progress); the global hotkey (`tauri-plugin-global-shortcut`); the tray item; the Mac microphone usage line. The content policy of the web view does not change: no audio passes through it.
- `ui`: `OpenDotStage`, `Breath`, `VoiceSection` (settings), the rail mark, and a small hook that follows the `voice` events. Settings fields are added to `Settings` with defaults that keep everything off.

## 6. Constraints

- Follow the repo's patterns: commands registered once and added to the contract fixture; settings added with defaults; conventional commit messages; tests beside the code.
- Speech models are downloaded on first enable, not shipped in the installer.
- The build fetches the speech library's ready-made binaries; CI already allows network in builds. The version is pinned.
- No audio or transcript is written to disk or logs. Logs may say that a turn happened and how long each step took.
- The feature is off until turned on. With it off, no microphone is opened, no model is loaded, and startup time does not change.
- Hotkey registration can fail (taken by another app): say so in Settings and carry on without it.

## 7. Testing

- `dots-voice`: state machine with fake engines (every state and interruption path); text cleaning and sentence splitting; manifest verification (good, wrong hash, partial download, resume); a recorded-clip test for the real engines that runs only when the models are present locally.
- `open_dot`: quick matcher table tests; reply parsing (good, malformed, over-long, unknown action, unknown id); confirmation rules (an approval can never pass without the matcher's yes, whatever the model says); each action against the real runtime with the fake Claude binary; the setup flow by voice end to end with fakes.
- `dots-app`: contract fixture, command wiring.
- `ui`: stage states, options by key and click, settings tab, reduced motion, the Breath's numbers for each state (not pixels).
- By hand, on the real app: a full spoken session through every row of 3.5, on speakers and on headphones, with the timings in goal 4 measured and written into "As built".
- Mac: the mac-check workflow must pass; a spoken session on a Mac is owed before a Mac release.

## 8. Order of work

1. Ears and voice crate, with the downloader.
2. The mind, with fakes.
3. App wiring, settings, hotkey, tray.
4. The stage and The Breath.
5. Wake phrase, interrupting, spoken notices.
6. Whole-app check by voice, timing, and polish.

One branch (`feat/open-dot`), one pull request per step where a step stands alone, QA before merge.

## 9. Calls made without asking (the person can overturn any)

- The name "Open Dot", the wake phrase "Hey Dot", and the default hotkey.
- On-device speech only, English only, in this project.
- The wake phrase and "Interrupt by just talking" start off.
- It lives in the main window, not a floating window.
- Nothing said is saved.
- The small model thinks by default.
- Speech files come from the speech library's public release pages, pinned by hash. Hosting our own copy is a later choice.
- Codex is out of this project because Dots has no Codex engine.
- It thinks with the Claude CLI only; local endpoints are a later spec.
- No echo cancellation: on speakers, interrupting by voice is best effort and the key always works.
- A click or key on Yes counts as agreeing, the same as saying yes.
- Private approvals are counted but never described aloud or sent to the model.
- Its model requests show in Usage and count against spend caps.
- It works in the workspace the app is showing.

## 10. As built

### Step 1: ears and voice (2026-11-11)

**What exists now.** The crate `crates/dots-voice` is a workspace member. It has no Tauri or core dependency; the app will own a `Session`. Its modules:

- `text`: turns a model reply into speakable sentences (strips markdown, code fences, links and list markers; cuts at sentence ends; caps what is spoken).
- `models`: the pinned manifest, the resumable downloader, the SHA-256 check and the unpacker (files are verified when installed; loading checks only that they are there).
- `engine`: the traits (`Speaker`, `Recognizer`, `SpeechDetector`, `WakeSpotter`), `Clip`, and resampling.
- `sherpa`: the real engines behind those traits, through `sherpa-onnx` 2.0.1.
- `ears`: speech detection with a pre-roll, end-of-sentence detection, live partial words and the final words.
- `voice`: one speech queue; synthesises sentence by sentence; stop is immediate.
- `audio`: microphone and speaker through `cpal`, with a bounded playback queue and level readings.
- `session`: the state machine (`Off, Idle, Listening, Thinking, Speaking, Asking`) and its events. Its calls are `open`, `close`, `interrupt`, `thinking`, `say(turn, text, then)`, `say_delta(turn, delta)`, `end_of_reply(turn, then)`, `answered(text)`, `filler(turn, text)`, `notice`, `settings`, `set_voice`, `state`, `turn` and `shutdown`; the person's words, spoken or clicked, arrive as `Heard { text, turn }`.
- `examples/talk.rs`: a command-line program that hears one sentence and says it back, using the ears and the voice directly.
- `examples/session.rs`: the hand check for the whole `Session` on the real microphone and speaker: it prints every session event, says back what it heard, and takes typed commands (open, close, interrupt, wake phrase, talking over, a clicked answer, a question).

The crate has 163 unit tests, all passing, and 7 tests that use the real models and are ignored unless `DOTS_VOICE_MODELS` is set (all 7 pass here).

**The speech models** are pinned by size and SHA-256 and come from the speech library's public release pages. Total download: 258 MB (258,311,120 bytes). They install under `<data dir>/voice/models`; in development `DOTS_VOICE_MODELS` points at a folder (here `C:\dev\dots-voice-models`).

| Folder | Model | Size | Licence |
|---|---|---|---|
| `speak` | Supertonic 3 (int8), 10 voices | 128.8 MB | MIT |
| `hear` | Moonshine base English (quantised) | 111.3 MB | MIT |
| `wake` | Zipformer keyword spotter (gigaspeech, 3.3 M) | 17.6 MB | Apache-2.0 |
| `vad` | Silero VAD | 0.6 MB | MIT |

**Measured on the development machine (Windows ARM64, debug build, synthetic voices).** The six real-engine tests pass (about 70 to 110 s for the set, depending on load). A short sentence ("Okay, I'm on it.") is ready to play in 672 ms in the last run (980 and 1374 ms in two earlier runs; the test's limit is 1500 ms, and a release build will be faster). The wake phrase "Hey Dot, what needs me?" was caught 10 of 10 times, one per voice. "Dot, stop." was caught 10 of 10 times, with the keyword spelled `▁DO T ▁ST O P` because the model's token list has no `TO`. Two near-miss sentences ("Hey Dad, I connected the dots on that report." and "Please stop the music before the dog gets a hot donut.") caused no wake, 0 false hits in 20.

**What differed from the design or the plan.**

- `min_speech` is judged on the longest unbroken run of speech, not the total, because two coughs could otherwise add up to a turn and cost a wasted model request.
- A reply to a turn the person interrupted is never spoken: every reply call carries a turn id, and the voice drops late text of a stopped reply (a latch) until that reply is closed, because a model stream cannot be halted at once and the queue has to swallow its tail.
- With the wake phrase off, the microphone is closed in Idle, Thinking and Speaking (unless "Interrupt by just talking" is on), because the design forbids an open microphone without a visible sign and there is nothing to listen for.
- Safety limits were added so the assistant cannot stay stuck: 120 s in Thinking (then "No reply came back." and Idle), 60 s in Speaking without word from the voice or new reply text, and 20 s for a question (Asking, then Idle). Listening rests after its quiet limit (8 s after `open`, `follow_up_secs` after a reply); noise no longer restarts that limit, but a clip in progress pauses it, so sound the detector keeps calling speech can hold Listening open.
- The speech library's own build script downloads a prebuilt native library with no hash check, so CI and release builds use `tools/voice` instead: it fetches the pinned archive per target, checks size and SHA-256, and sets `SHERPA_ONNX_ARCHIVE_DIR`. `release.yml` and `mac-check.yml` call it and run `cargo test -p dots-voice --locked`. Local builds still use the library's own download.
- The playback queue is bounded to 60 s of sound, and the output callback never waits on the queue, because a stuck callback would stop the sound.
- The session gained `Parts.voice_events`, `MicOpener`, `SystemClock`, `notice`, `turn` and `set_voice`, because the plan gave it no way to hear the voice, to be tested against a clock, to speak a notice, or to change voice. It also drops late text for a cut turn itself and stops the voice at once.
- Choices the plan left open: "Dot, stop" while Listening or Asking closes the session; `open` while Thinking interrupts; a voice error while Speaking returns to Idle; the wake phrase defaults to off.
- A reply ends (`Finished`) only after it is closed, every sentence has been handed over and the speaker is idle, so a gap between sentences cannot end it.

**Changed in the final fix round (2026-11-11), after the whole-branch review.**

- A speaker that fails is no longer silent. `Sink::play` returns whether the clip was taken and `Sink::failed` says the speaker stopped working; the voice then ends the reply with `Trouble("The speaker stopped working.")` instead of `Finished`, the session reports it and rests, and the next thing spoken opens a fresh speaker through `Parts.open_sink` (`Voice::set_sink`). If none opens, that is reported too and the session rests.
- A click or key press on an option is a turn: `answered(text)` stops listening, asking or speaking, sends `Heard { text, turn }` with a new turn id and moves to Thinking, from any state but Off (where it returns `None`). Spoken and clicked turns share one counter, so ids never repeat.
- `filler(turn, text)` speaks a short line ("One moment.") while Thinking for the current turn. Thinking's 120 s limit keeps running, the line is not the reply, the reply is spoken after it, and a filler for any other turn is dropped. The 60 s speaking limit is now re-armed by every accepted `say`, `say_delta` and `end_of_reply` and by every voice event, so a long streamed reply is not cut while text or sound is still arriving.
- Downloads: the connection must open within 15 s, and a download with no data for 30 s ends with `ModelError::Stalled`, keeping its `.part` to resume. The final renames are tried up to 5 times on Windows (100 ms pause, doubling) because an antivirus scan can hold a fresh folder, and a checked `.part` is never deleted when putting it in place fails: the next try puts it in place without downloading it again. A plain file is copied, not moved, out of its `.part`.
- A lost microphone (this reverses the earlier rulings): with the wake phrase on, it is tried again every 5 s through `MicOpener`, quietly after the first report, so "Hey Dot" comes back after sleep or a headset change. Lost while Thinking or Speaking, it does not stop the reply; once speech is over it is tried at once, and the loss is reported once only if it still will not open then.
- `examples/session.rs` drives the real `Session` (see above), and a new ignored real-engine test feeds "Yes.", "No." and "Stop." in all ten voices through the real `Ears` with the short-answer settings.
- Measured: through the real detector, those one-word answers ran 6 to 19 frames of 32 ms (192 to 608 ms) as their longest unbroken run of speech; in 180 clips, 3 ran 7 frames or fewer, so with `min_speech_ms` at 250 (8 frames) about one curt answer in 60 was dropped as a cough. `short_answer()` now uses 160 ms (5 frames): one frame below the shortest run seen, and a 60 ms click (2 frames) is still dropped. About one synthetic clip in 200 held no speech the detector could find at all; no setting helps that, and the test makes such a clip again.
- `shutdown` still returns at once; its doc comment now says the event channel closing is the sign the worker has ended.

**Known and not fixed.** All were found in review and judged small. The first group should be fixed before shipping.

- Downloads (`models`): a `.part` is deleted before the marker is written and a crash between re-downloads is not handled; there is no fsync before the marker; the `206 Content-Range` is not checked (a wrong offset surfaces as `WrongHash`) and a `416` errors instead of restarting; two installs on one root are not guarded; there are no tests for a stale `.tmp`, a stale marker with a changed hash, or cancel mid-stream.
- Text (`text`): a one-line "```x```" opens a fence and swallows the rest; text with no `. ! ?` is not cut at newlines, so the first speech waits; "2024. Great year" loses "2024." as a list marker; an indented "  1. x" can still leave a lone "1."; mixed `~~~` and "```" fences toggle state; the length cap counts bytes.
- Engines (`sherpa`): `get_result` allocates for each ready frame; `heard` and `is_speech` do not `debug_assert` the frame length; the 1500 ms "short sentence" test can flake in a debug build on a loaded machine; the doc comment on the hit-rate test is misleading; the voice clamp is undocumented; the downsample tail is untested; the linker warning LNK4098 (see below) is accepted for now.
- Ears (`ears`): the slow-partial backoff only skips every other interval; the partial cadence and unchanged-text suppression are untested; the wrong-size-frame test is skipped in debug; a reset after `Started` emits no `Dropped`, so the session must clear its own state on reset.
- Voice (`voice`): `sink.idle()` is called under the state lock; a barrier waits without a timeout; the closed-receiver test is weak; there is a gap between queued replies; `say()` clears the latch and appends in two lock scopes, and no doc line says `say()` is the way out of the latch.
- Audio (`audio`): the microphone callback allocates on each call; the sample-format test tests `cpal`, not our code; the example can hear its own last syllables because idle comes before the device tail ends (the session's `reopen_after_ms` covers this in the app); playback reserves 60 s of `f32` up front (11.5 MB at 48 kHz, 46 MB at 192 kHz) and should grow lazily.
- Session (`session`): `shutdown()` does not join the worker, so the microphone is released asynchronously (the event channel closing is the sign it has ended); `thinking()` is not tied to a turn (harmless).

**Still owed before this can ship.**

- The hand check: `cargo run -p dots-voice --example session` (and `talk`) with `DOTS_VOICE_MODELS` set, on speakers and on headphones, including a curt "yes" after a question, unplugging the headset mid-answer and with the wake phrase on. No person has heard this yet; the real-engine tests use synthetic voices.
- The linker warning LNK4098 (the speech library's Windows archives use the static C runtime while Rust links the dynamic one) is to be fixed in step 3, when the app first links the crate: build the Windows targets with the static C runtime (`-C target-feature=+crt-static`) so Rust, every `cc`-built dependency and the speech library agree, and the warning goes away. A clean CI run does not show the mismatch is harmless.
- Locally built installers link an unverified download of the speech library (its own build script fetches it with no hash check); CI and release builds verify it through `tools/voice`. Build installers in CI, or set `SHERPA_ONNX_ARCHIVE_DIR` to the tool's verified archive before building one locally.
- A first CI run on Windows x64 and on Mac, where the crate has never been built.
- Steps 2 to 6 of section 8. None is started, and nothing of this is wired into the app yet.

### Step 2: the mind (2026-11-12)

**What exists now.** The module `crates/dots-core/src/open_dot/` is part of `dots-core`. It makes no run and no dot; it takes the person's words, one turn at a time, and gives back events for the stage to say and show. Nothing is wired to the app, the voice crate or the UI yet (step 3). Its files:

- `quick`: the no-model matcher for stop, cancel, repeat, yes, no, and picking an option by number, position or name. It also has `firm_yes` (the strict yes a confirmation needs) and the list of common words that can never name a dot.
- `reply`: reads a model reply into the words to say and at most one wish (ten kinds: needs, read a question, answer, approve, deny, create, send, run, open, ask). It also has `SpeechGate`, which lets words stream out while holding back the `DO {...}` line, so the block is never spoken. Spoken words are cut at 600 bytes on a sentence end.
- `picture`: the picture of now. It lists the workspace's dots (at most 40), the approvals (10) and the questions (10), with the screen and topic being viewed, and gives each a handle (`d1`, `a1`, `q1`). Private approvals are counted, not listed. Targets are cleaned, scrubbed of vault values and cut at 120 bytes.
- `prompt`: the instructions (with three example replies) and the prompt: the picture between a header and "End of picture.", the last 6 turns, and the person's words.
- `think`: the `Thinker` trait and `CliThinker`, which asks the Claude CLI for one streamed answer in a fresh folder that it removes afterwards (default model `haiku`, 45 s limit), and `cap_hit`, which checks the global spending caps. It logs only how long a request took and how it ended.
- `act`: turns a wish into a checked step (say, do, ask, confirm, create), builds every confirmation question from the real record, and carries out deeds through the runtime (`decide_approval`, `run_now`, `reply`, `chat`). Its spoken sentences are mostly fixed.
- `mod`: `Mind`, which holds what waits between turns (a confirmation, a question with options, a setup in progress, a proposal), the turn in flight, and the events (`Thinking, Filler, Say, SayEnd, Ask, Card, Open, Cleared`). Calls: `hear`, `pick` (a click), `lapse`, `cancel`, `reset`, `pending`.
- `tests`: a scripted thinker and the mind's tests, run against a real runtime built from a temporary config (the integration tests are in `crates/dots-core/tests/open_dot_test.rs`).

Also changed outside the module: a store table `open_dot_usage` (migration `0046_open_dot_usage.sql`: id, at, model, tokens in, tokens out, cache read, cost) with `store/open_dot.rs`; a setting `open_dot_model` (default `haiku`, must not be empty); `UsageSummary` gains `open_dot` (the app's contract fixture was regenerated); `Runtime` owns the `Mind` (`Runtime::open_dot()`); the CLI one-shot path in `once.rs` can stream partial text and report usage (its command line and limits are as before); `setup_ask_with_usage` and `SETUP_MODEL` let the mind count a setup round; and the fake CLI test binary can answer by what a prompt holds.

**Tests (the closing gate on 2026-11-12, branch `feat/open-dot`, ARM64 Windows, debug build).** `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` are clean. `dots-core` has 260 unit tests, all passing (113 of them in `open_dot`; the rest were there before). `tests/open_dot_test.rs` has 36 tests; they passed in 8 runs in a row on a quiet machine (12 to 18 s each, 36 of 36 every time), so the 15 s `wait_status` timeouts seen twice in 15 runs while the machine was loaded did not come back. The whole core suite (`cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule`) ran 3100 tests passing and 1 failing: `context_rules_test::the_store_keeps_fills_requests_and_records` (left 36000, right 30000 at line 429), a test from an earlier project that this branch did not touch; run alone it passed 3 of 3, so it is a flake whose cause was not found. A first whole-suite run failed `claude_engine_test::a_finished_run_leaves_no_process_behind` while the gate's own voice and app tests were running beside it; the rerun alone passed. `dots-voice` has 163 tests passing (7 more ignored without the models), `dots-app` 202, `node --test tools/release/release.test.mjs` 11 and `node --test tools/voice/sherpa-lib.test.mjs` 10, and the UI has a clean type check and 1708 tests passing. The `ui/dist` build was already present. Not run: `preview_schedule` tests (skipped as the gate says).

**How a turn goes.**

1. The words arrive (`hear`) or an option is clicked (`pick`). A newer turn cancels the one in flight. After 10 minutes idle, the turns and what waits are forgotten. A waiting step older than 60 s, or made in another account or workspace, is dropped first.
2. Stop, cancel, repeat, yes, no and picks are answered without a model.
3. Otherwise the global spending caps are checked. At a cap it says so and asks no model.
4. Dots takes the picture, builds the prompt and asks the model through the CLI with no tools. The first sentences stream out through the gate. If nothing has been said after 1.5 s it says "One moment." once. The request is recorded in `open_dot_usage` when the CLI reports usage.
5. The reply is read into words and at most one wish. Dots code checks the wish against the picture: a handle must be one the model was given, and anything else becomes a spoken question.
6. The step is said, shown, asked or done. Every turn ends with exactly one `SayEnd`, even when it fails.

**The confirmation rules as built.**

- Always asked first: approving, denying, creating a dot and running a dot now. The question is built from the record (dot, tool, target), never from the model's words. A long or cut command is not spoken at all: it says "a long command", opens the approval on screen, and asks. A private approval cannot be decided by voice; it is opened on screen.
- A firm yes is one of: yes, yeah, yep, yup, yes please, yes do it, do it, go ahead, go for it, please do, approve, approve it, confirm, absolutely. "Okay", "ok", "sure", "right" and "correct" are not firm. A weak one is answered "I need a clear yes for that. Say yes, or no." and the step stays; a second weak one drops it ("Okay, I won't.").
- A click on Yes is a yes, a click on No is a no. A spoken position word ("one", "the first") never counts as yes or no on a confirmation. A spoken no ends the step. Anything else drops the step and the words go to the model as a new turn.
- A step waits 60 s (`PENDING_LIMIT`), then is dropped. Going quiet, a newer turn, `lapse` and `reset` all drop it.
- At the moment of yes, Dots takes a fresh picture and checks that the record is still there (approval still pending and not private, dot still in view, question still waiting). If not, it says "That one is already handled." or "I can't find that dot anymore." and does nothing. A second yes or click for a step a yes already took does nothing. A cancelled or replaced turn never carries out a deed.
- Send and answer go straight through (said after) only when the person named the dot in their own words (a word of its name, three letters or more, not a common word, and in no other dot's name), or chose it from Dots' own "Which dot?" options, or it is the dot on screen, or (answering) the question was read aloud in the turn before. Otherwise Dots asks "Send to X: ...?" or "Answer X with: ...?" first. A click on an option the model wrote names nothing. If the person was unsure of the words, it reads them back ("I heard: ... Send that?") whatever the target. Text over 400 bytes is refused as too long to send by voice.
- A question the model asks (with options) is answered back to the model; it can never pass a confirmation.

**What the model is and is not given.**

- It gets handles, never ids. It can only name a handle; Dots maps it back and refuses one it was not shown.
- Private approvals are only counted ("one private approval needs you"); their tool and target are not in the picture, are not said, and open on screen.
- Targets are cleaned (hidden characters out, one line, 120 bytes), vault secrets are replaced (`[secret:NAME]`, or `[secret]` when split), URL credentials, queries and fragments are removed, and private paths and secrets are looked for in the raw, cleaned and squeezed forms of a command and in any field of any tool (`command`, `file_path`, `url`).
- Long commands are not spoken (see above). Questions from dots are given as the Inbox shows them (cut at 400 bytes). Snoozed and done questions are neither listed nor counted.
- Errors said aloud lose paths, ids and vault values and are cut at 160 bytes. Markdown marks are never said.
- No tools, no MCP servers, no settings files: the request runs as the one-shot path does, in a folder of its own. Nothing heard, said or pictured is logged.

**Differences from sections 3.5 and 5.2, and why.**

- Usage is a new table `open_dot_usage`, not runs: Open Dot makes no run and usage lived only on runs. A dot's own caps never hold Open Dot; the caps for every dot together do.
- The model gets handles (`d1`, `a1`, `q1`), not ids, so it cannot name a record it was not shown or invent one.
- Reply format: plain words, then at most one final line `DO {json}`. Everything before it is speech. A malformed block is not run; it says "I didn't catch what to do with that. Say it another way?".
- Send and answer are sometimes confirmed, against "do, then say" in 3.5: a prompt-injected question or dot name could otherwise make Dots send messages in the person's name. They go straight only when the person's words name the target, it was chosen, or it is on screen (see above). Overturnable.
- "Okay", "sure", "right" and "correct" are not a yes for a confirmation (they are for a question the model asked), so that an acknowledgement, or noise heard as a word, cannot approve, run or create. The cost is that the person has to say "yes" or "do it".
- Spoken position words never choose Yes or No on a confirmation; only a click does.
- Section 5.2 says a confirmation is a yes "caught by the quick matcher". It is the strict matcher `firm_yes`, plus a click.
- The creation question says only the name and when it runs; the first message, instructions, folder and access are on the card.
- Spend totals now include Open Dot: the day, month and cap totals add `open_dot_usage`, so the per-dot rows in the Usage tab no longer add up to the total.
- Every deed a yes passes is re-checked against a fresh picture at the moment of yes (not in the spec).
- Pending steps expire after 60 s, and a lapse cancels the turn in flight (not in the spec).
- Setup-agent requests made for Open Dot are cap-checked and their usage is recorded.
- The model's own question text is sent as the Inbox shows it, written by the dot's own model: parity with the app. A question that quotes private data would reach the Open Dot request.
- The picture cuts to 40 dots, 10 approvals and 10 questions inside `take`, so a handle the model never saw cannot resolve.
- A refused send says why: "still working" only when the thread is active (the runtime's own constant); any other conflict is said as the runtime's own sentence; a missing dot says "I can't find that dot anymore."
- The kept process was not built (below), so each turn starts a fresh CLI process and sends the last turns again.

**The kept process: measured, not built.** Measured on 2026-11-12 with `claude` 2.1.322 and `haiku` (`scratchpad\kept-process\results.md`; ten fresh runs, ten kept turns):

- First text, fresh process: 1594 ms median (the system line alone takes about 480 ms). Result line: 1828 ms.
- First text, kept process, turns 2 to 10: 1114 ms median. The gap is 480 ms, under the plan's 0.7 s bar. (Paired by prompt it was 683 ms, but the fresh answers were longer; the median result gap was 710 ms.)
- Every kept turn ended with its own result line (10 of 10, also two sent back to back and one after idle), with per-turn usage and memory of earlier turns.
- A kill was noticed within about 25 ms of the kill returning (about 275 to 307 ms including `taskkill`). It survived 10 minutes idle and answered in 1034 ms. A crash with no kill was not tested.
- About 250 MB resident (claude plus conhost), growing about 0.5 MB a turn.
- Ruling: not built, because the gap is under the bar and fresh is already 1.6 s to the first words. About half a second a turn is the cost if that is wrong. To be looked at again in step 6 (timing polish), with real prompts.

**Rulings made while building (the ledger, in order).**

- Ruling: the plan was not shown to the user before execution (standing preference to build with stated defaults; plan committed 3e0dd788). Cost if wrong: rework of step 2.
- Ruling: tasks 1 and 2 went to one builder in one dispatch (two commits), reviewed together, both pure with no shared state with the runtime. Cost if wrong: one larger review.
- Ruling: `Picture` must cap in `take()`, so a handle the model never saw cannot resolve (ids must be ones from the picture it was given). Cost if wrong: none, stricter.
- Ruling: question text is sent to the model as the Inbox shows it (written by the dot's own model), for parity with the app. Cost if wrong: a question quoting private data reaches the Open Dot model request.
- Ruling: a send conflict says "still working" only when the thread is active; other conflicts speak the runtime's own sentence; not found on run or send says "I can't find that dot anymore." The brief's blanket mapping misled.
- Ruling: send and answer run without a yes only when the person's words name the target dot, or it is on screen, or (answer) the question was just read or is the only one; otherwise Dots asks "Send to X: ...?". The spec says "do, then say", but a prompt-injected question could otherwise send messages in the person's name. Cost if wrong: one extra yes on some sends. Overturnable. (Tightened below.)
- Ruling: every pending deed is re-checked against a fresh picture at the moment of yes; a view change drops it.
- Ruling: spoken position words never count as yes or no on a confirmation; only a click maps an index to yes or no.
- Ruling: the confirm-tier yes is a firm set (no "okay", "right", "sure"), so a forgotten confirmation is not passed by an acknowledgement. Cost: the person must say "yes", "do it" or "approve".
- Ruling: pending steps expire in core 60 s after being asked; a lapse cancels the turn in flight.
- Ruling: the send and answer rule was tightened: the "only question waiting" exception is removed; a name match needs a word unique to that dot and not a stop word; a click on a model-written option is not the person's words. The on-screen dot exception stays.
- Ruling: setup-agent requests made for Open Dot are cap-checked; their usage is recorded (done through `setup_ask_with_usage`; it was to be deferred and written here only if a small change to setup did not allow it).
- Ruling: the kept process is not built (see above).

**Known issues and deferred.**

- Important, found in the last re-review and not fixed: on a Deny confirmation, "approve" and "approve it" pass as a firm yes (they would deny), and "deny" counts as a no. The fix round to dispatch: per-kind firm words (on Deny, "approve" is anything else and "deny" and "deny it" are firm; on Approve, "deny" is a no).
- Also from that re-review, not fixed: renew the wait on a repeat (at most twice); an expired click should say the nothing-waiting line and `pending()` should check the limit; per-kind firm phrases ("create it", "send it", "run it", "yes yes", "yes go ahead"); a doc line; and tests that wait on a notify, not on sleeps (the "late 300 ms" pattern and the idle test).
- Minor (deferred): `SpeechGate` text grows after the cap; there is no `debug_assert` on the released boundary.
- Minor (deferred): label the "today" spend total in the UI as including Open Dot (the per-dot rows no longer sum to it).
- Minor (deferred): the usage of cancelled and timed-out requests is not counted (M-16): `OnceFailure` carries no usage, and counting it changes the CLI runner's contract.
- Minor (deferred): underscores are not stripped from speech (a word may be cut between deltas).
- Minor (deferred): three fixes have no test: a cancelled setup round ends and loses its answers (M-11), a cap read error fails closed with "Something went wrong. Try again." (M-13), and a setup round says "One moment." once when slow (M-17). By reading only.
- A secret spoken from an error as `[secret:NAME]` is said as written; setup-agent errors other than the CLI failure form are only cleaned by `plain_error`; the yes-check uses the capped picture, so a record that moved past a cap reads as handled or gone; a run conflict still says "still working on that" for any run conflict.
- The picture calls `Runtime::inbox()` each turn (it builds the whole Inbox) and reads the store twice per in-view approval; fine at voice-turn rates. A dot whose only waiting item is a question shows "idle, needs you 1". There is no test with an org dot.
- A stray fence line before an unclosed `DO {` line would be spoken as "backtick backtick backtick" (rare).
- `open_dot_test` timed out under load in the builder's runs (2 of 15, `wait_status` 15 s at `tests/common/mod.rs:142`); on a quiet machine it passed 8 of 8 (see the Tests paragraph above).

**Still owed before this can ship.**

- The hand check by voice, once the app is wired (step 3): confirmations by voice and by click, a weak "okay", a send that asks and one that goes straight, a cap, and the filler line.
- A CI run on Windows x64 and on Mac (`cargo test -p dots-core` and the rest; nothing of step 2 has run there).
- Label the spend total in the UI as including Open Dot (step 3 or 4).
- The fix round for the Deny wording above.
- Steps 3 to 6 of section 8. Nothing of this is wired into the app yet.
