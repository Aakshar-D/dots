# Open Dot, the voice assistant: design

- Date: 2026-11-11
- Status: steps 1 (ears and voice), 2 (the mind) and 3 (app wiring) are built; steps 4 to 6 are not
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
- "Hey Dot" does not raise a hidden or minimised window (step 5, as built): the reply is spoken, and the tray shows "Dot is listening". Only the hotkey and the tray's "Talk to Dot" bring the window forward.

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
- The linker warning LNK4098 (the speech library's Windows archives use the static C runtime while Rust links the dynamic one) is to be fixed in step 3, when the app first links the crate: build the Windows targets with the static C runtime (`-C target-feature=+crt-static`) so Rust, every `cc`-built dependency and the speech library agree, and the warning goes away. A clean CI run does not show the mismatch is harmless. Done in step 3: the Windows CI jobs (`test`, `release`) set `RUSTFLAGS=-C target-feature=+crt-static` for the whole job, and `tools/voice/README.md` gives the two lines for a local `.cargo/config.toml`. Measured on Windows ARM64 (CI is x64, so evidence there, not proof): without the flag the voice crate's test and example binaries warn LNK4098; with it no binary warns, the voice and app tests pass, and the app starts and stays up.
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
- A firm yes depends on what is asked (`firm_yes(heard, ConfirmKind)`). For every kind: yes, yeah, yep, yup, yes please, yes do it, yes yes, yes go ahead, do it, go ahead, go for it, please do, confirm, absolutely. An approval also takes approve, approve it; a deny also takes deny, deny it, deny that; a run also takes run it; a creation also takes create it, make it; a send also takes send it, send; an answer (and an answer read back) also takes send it, send that. On an approval, deny, deny it and deny that are a no. Another kind's own words are neither yes nor no ("approve" said to "Deny ...?"): the step is dropped and the words go to the model as a new turn. The no words are the same for every kind. "Okay", "ok", "sure", "right" and "correct" are not firm. A weak one is answered "I need a clear yes for that. Say yes, or no." and the step stays; a second weak one drops it ("Okay, I won't.").
- A click on Yes is a yes, a click on No is a no. A spoken position word ("one", "the first") never counts as yes or no on a confirmation. A spoken no ends the step. Anything else drops the step and the words go to the model as a new turn.
- A step waits 60 s (`PENDING_LIMIT`), then is dropped; "say that again" asks it again and starts its 60 s again, at most twice per step, and so does the one unclear-yes retry; whatever was renewed, the step is dropped three waits (180 s) after it was first asked. `pending()` is false for a step past its time, and a click on such a step says "There's nothing waiting on a yes or no.". Going quiet, a newer turn, `lapse` and `reset` all drop it.
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
- Ruling: the confirm-tier yes is a firm set (no "okay", "right", "sure"), so a forgotten confirmation is not passed by an acknowledgement. Cost: the person must say "yes", "do it" or the step's own verb ("approve" passes only an approval question, "deny" only a deny, "run it" a run, "create it" a new dot, "send it" a send or an answer). "Go ahead", "go for it" and "yes go ahead" do not confirm a deny (they may mean "let it go ahead"): they are an unclear yes there, asked once more.
- Ruling: pending steps expire in core 60 s after being asked; a lapse cancels the turn in flight. "Say that again" (at most twice) and the one unclear-yes retry each start the 60 s again, but a step never waits more than three waits (180 s) from when it was first asked. A click after the 10-minute idle forget dropped the step says "There's nothing waiting on a yes or no.", as on an expired one.
- Ruling: the send and answer rule was tightened: the "only question waiting" exception is removed; a name match needs a word unique to that dot and not a stop word; a click on a model-written option is not the person's words. The on-screen dot exception stays.
- Ruling: setup-agent requests made for Open Dot are cap-checked; their usage is recorded (done through `setup_ask_with_usage`; it was to be deferred and written here only if a small change to setup did not allow it).
- Ruling: the kept process is not built (see above).

**Known issues and deferred.**

- Fixed (fix round 2): on a Deny confirmation, "approve" and "approve it" passed as a firm yes (they would have denied), and "deny" counted as a no. The firm words now depend on the kind of step (see the confirmation rules above).
- Also fixed in that round: a repeat renews the wait (at most twice); an expired click says the nothing-waiting line and `pending()` checks the limit; per-kind firm phrases ("create it", "send it", "run it", "yes yes", "yes go ahead"); the `cancel()` doc line; and the tests wait on a notify or move times back, not on sleeps (the "late 300 ms" pattern and the idle tests).
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
- The fix round for the Deny wording above: done (fix round 2).
- Steps 3 to 6 of section 8. Nothing of this is wired into the app yet.

### Step 3: app wiring (2026-11-12)

**What exists now.** The app owns the voice session and joins it to the mind. Everything is in `apps/dots-app/src/voice/` unless named otherwise; with voice off none of it runs (see "Feature off" below). The stage and The Breath are step 4, so in a build of step 3 alone what Open Dot hears and says is audible, and visible only in the Voice tab's status line and the tray.

- `bridge`: a pure state machine between the session and the mind, behind two traits so it is tested with fakes. `SessionPort` (`open`, `close`, `interrupt`, `thinking`, `say`, `say_delta`, `end_of_reply`, `filler`, `answered`, `notice`, `state`) is implemented for `dots_voice::Session`; `MindPort` (`hear`, `pick`, `lapse`, `cancel`, `reset`, `pending`) for the runtime's `Mind`. It keeps one live turn: each `Heard` makes the turn before it stale, and every event is applied under one lock and only for the live turn, so no session call carries a stale turn id. One driver task starts mind turns in the order they were heard, and a turn dropped before the driver takes it never reaches the mind. A click's turn is remembered (`turn -> index`) and reaches the mind as `pick`, never as `hear`. `close` drops what waits (`cancel`, `lapse`, `reset`, then `Cleared`); `stop` drops the live turn and interrupts; `open`, and the session starting to listen, drop a live turn too (talking over it). A turn the mind made nothing of is ended by the bridge, and a mind call that panics ends its turn with "Something went wrong. Try again.". A finished deed's outcome (`MindEvent::Did`, added to core in this step) is spoken on the live turn when its own turn went stale, or said as a notice when the session rests, so it is never lost.
- `service`: owns the session and the download. A `SessionFactory` builds sessions (`RealFactory` follows `examples/session.rs`; tests use a fake), a `Host` gives the settings and the mind (`RuntimeHost`), a `HotkeyPort` registers the shortcut and a `Listener` hears the microphone and readiness changes (the tray). `settle` brings what runs in line with the settings in a loop (build; rebuild on another microphone or speaker; set voice, speed, wake phrase and talk-over on the running session), so a change made during a build is never lost; the factory asks `wanted()` just before `Session::start`, so a build no longer wanted opens no microphone. `start` (at launch) applies the stored settings and resumes a download that ended with Dots; `apply_in_background` applies a settings save without making the save wait for a model load; `shutdown` (exit, tray quit, restart) cancels the download, shuts the session down and waits up to 500 ms for it to let go of its devices.
- `download`: the speech files' download through `dots_voice::install`, progress at most ten times a second (plus the last), every failure as a sentence: cancelled; "The download stopped. It picks up where it left off when you turn Open Dot on again." (stall or network); damaged; no room; busy. One download at a time: a second `voice_enable(true)` while one runs answers with the current status.
- **The eleven commands** (the spec's nine plus `voice_remove` and `voice_view`), thin wrappers in `commands.rs`: `voice_status`, `voice_enable(on)`, `voice_download_cancel`, `voice_remove`, `voice_open`, `voice_close`, `voice_stop`, `voice_pick(index)`, `voice_view(view)`, `voice_preview(voice, speed)` (says "Hi, I'm Dot." in that voice, then puts the saved one back), `voice_devices`. Each answers "Dots did not start; see the startup error" when the runtime did not start. `voice_view` only stores the view (a field over 256 bytes is refused), so it is cheap with voice off.
- **The `voice` event**: one stream, tagged by `kind`: `state`, `level` (skipped while the main window is hidden or minimised; a level that is not a number is sent as 0), `hearing`, `heard`, `saying`, `ask`, `card`, `open`, `cleared`, `woke`, `trouble`, `download`, and `status` (sent whenever the status changes, never twice the same). `VoiceStatus` is `enabled, installed, downloading, ready, state, mic_open, download_bytes, hotkey_error, trouble`. The contract fixture has `VoiceEvent`, `VoiceStatus`, `VoiceDevices` and `View`, each with a zod schema.
- **Settings fields** (`dots_core::Settings`, all off by default): `voice_enabled`, `voice_id` (0 to 9), `voice_speed` (0.8 to 1.4), `voice_hotkey` (default `CmdOrCtrl+Shift+Space`, empty for none; the core checks its shape only), `voice_wake_phrase`, `voice_interrupt_by_talking`, `voice_mic`, `voice_speaker` (ids at most 256 bytes), `voice_notices`. None needs a restart.
- `hotkey`: the global shortcut through `tauri-plugin-global-shortcut` 2.4.1 (pinned "2.4"), registered from Rust only while voice is on and a shortcut is set. A press brings the main window forward and opens Open Dot (while it speaks or thinks that is interrupt-and-listen). The new shortcut is registered before the old one is let go, so a taken one leaves the old one working; one that could not be let go stays tracked and is let go when voice turns off. `parse_hotkey` is the authority on what a shortcut is.
- **Tray**: "Talk to Dot" right after "Open Dots", enabled only while Open Dot is ready; the tooltip ends with "Dot is listening" while the microphone is open. The service tells the tray through a watch, so it never waits on the main thread.
- **Mac microphone line**: `apps/dots-app/Info.plist` with `NSMicrophoneUsageDescription` "Dots listens when you talk to Open Dot. Audio stays on this Mac.", referenced from `tauri.conf.json` (`bundle.macOS.infoPlist`). The web view's content policy is unchanged.
- **Voice tab** (`ui/src/screens/VoiceSection.tsx`, after Phone): the Open Dot switch (outside the draft form; it acts at once), the two lines about the download, a progress bar with Cancel, a status line (Off, Getting ready, Ready, or the trouble sentence, and "Listening" while the microphone is open), ten voices with play buttons, speaking speed, the hotkey field with its error and Clear, "Hey Dot", "Interrupt by just talking", microphone and speaker ("System default" first), which model thinks (Fast, Balanced, Most capable, bound to `open_dot_model`), "Tell me aloud when something needs me", and "Remove speech files" (asks first).
- **`useVoice`** (`ui/src/lib/use-voice.ts`, mounted once in `Layout`): subscribes to `voice`, keeps the status in the query cache and the fast-changing parts (state, levels, words, question, card, download, trouble) in a small external store read by selector, so a level does not redraw the app; navigates on `open`; and reports the view with `voice_view` (account and workspace from the current scope, screen and dot from the route), at most once per change.

**Tests (the closing gate on 2026-11-12, branch `feat/open-dot`, ARM64 Windows, debug build).** `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` are clean. `dots-core` has 274 unit tests passing (127 of them in `open_dot`), and `tests/open_dot_test.rs` 36. The whole core suite (`cargo test -p dots-core --locked --no-fail-fast -- --skip preview_schedule`) ran 3119 tests passing, 0 failing, 2 ignored. A first whole-suite run (without `--no-fail-fast`) stopped at `result_check_test::short_answers_switched_off_dots_and_runners_without_a_checker_are_not_checked` (a run did not reach Succeeded within the 15 s `wait_status` limit); run alone it passed 2 of 3 and failed once the same way, so it is a timing flake, not touched by this step; the full rerun passed it. `dots-voice` has 173 tests passing (7 more ignored without the models). `dots-app` has 280 tests passing; its 76 `voice::` tests passed 5 runs in a row. `node --test tools/release/release.test.mjs` (11) and `node --test tools/voice/sherpa-lib.test.mjs` (10) pass. The UI has a clean type check and 1766 tests in 162 files passing; that run includes step 4's first UI commits (The Breath), which another builder was adding at the same time. The contract test passes without `UPDATE_CONTRACT`.

**How a turn travels.**

1. The hotkey, "Talk to Dot" in the tray, or `voice_open` calls `Service::open`. Voice off, still downloading or building, or files missing answer with a sentence instead ("Open Dot is off. ..." or "Open Dot is still getting ready."); a session that failed or ended is built again first. Otherwise the bridge calls `session.open()`: the microphone opens and the session listens.
2. The ears hear the sentence end by themselves and the session sends `Heard { text, turn }`. The service's one event task hands every session event to the bridge in order (and updates the status from `State`).
3. The bridge makes that turn the live one, puts the session in Thinking, sends `heard` to the UI, and the driver starts the mind turn: `Mind::hear` with the words and the stored view, or `Mind::pick(index)` when the turn came from a click.
4. The mind answers with events on a channel. For the live turn the bridge turns `Filler` into `session.filler`, `Say` into `session.say_delta`, `Ask` into the spoken question plus an `ask` event with the options, `SayEnd` into `session.end_of_reply` (rest, follow up or ask), and forwards `Card`, `Open` and `Cleared`. A stale turn's events are dropped.
5. The voice speaks sentence by sentence as the text arrives. When the sound has finished (plus 250 ms) the session listens again for 6 s for a follow-up, or for the answer to a question, or rests and closes the microphone. If it rests while a step waits for a yes, the bridge calls `lapse`, so a later "yes" approves nothing.

**Feature off, and where it is enforced** (checked by reading on 2026-11-12):

- Microphone: opened only by a running `Session`, through the opener `RealFactory::build` hands it (`CpalMic::open`). A session is built only by `Service::settle`, which builds only when `voice_enabled`, the files are installed, no download runs and Dots is not quitting; `build` asks `wanted()` (the same conditions, the same generation) just before `Session::start`. Turning voice off shuts the session down at once (`apply_now`).
- Models: loaded only in `RealFactory::build` (the `Sherpa*::load` calls), reached only through `settle` as above.
- Hotkey: registered only by `Service::sync_hotkey`, which wants a shortcut only when `voice_enabled` and the shortcut is not empty, and lets go of it otherwise. At startup with voice off it makes no plugin call (the plugin itself is set up and registers nothing).
- Download: started only by `start_download`, from `enable(true)` (the person's switch), `open` (voice on and files missing, not while quitting) and `start` (voice on, files missing, not quitting). `apply` never downloads.
- Not guarded by the setting: `voice_devices` lists the system's microphones and speakers by name (it opens no stream), and `voice_view` stores the view.

**The static C runtime, measured.** On this machine (Windows 11 ARM64, rustc 1.97.1): without `-C target-feature=+crt-static` the voice crate's own test and example binaries warn LNK4098 (`defaultlib 'LIBCMT' conflicts with use of other libs`); with it no binary warns. `dots-app.exe` (debug) was 95,389,696 bytes without and 95,431,680 with (+42 KB); the voice and app tests pass both ways, and the app starts and stays up for 10 s both ways. The Windows CI jobs (`test`, `release`) set `RUSTFLAGS=-C target-feature=+crt-static` for the whole job (the rust-cache key hashes `RUSTFLAGS`, so the first run starts cold), and `tools/voice/README.md` gives the two lines for a local `.cargo/config.toml`. At this gate, without the flag, relinking `dots-app` (which now calls the speech library through the service) showed no LNK4098 locally; the voice crate's own binaries still warn without it, so the flag stays. Windows x64 is unproven until CI runs: its speech archive is also `static-MT`, so the same fix is expected.

**Differences from sections 3.1, 3.2, 3.7 and 5.3, and why.**

- 3.1: the mark in the rail does not come alive yet (step 4). A blocked microphone is reported as the session's own sentence ("The microphone could not be opened ..."), kept in the status until it listens again; there is no separate check for system permission and no "how to allow it" line yet. A cancelled download, or a failed one the person started, turns voice off, so "off until turned on" stays literal; a cut `.part` resumes when it is turned on again. A download that resumes by itself (at start, or on `open`) and fails leaves voice on and says "The download stopped. It picks up where it left off the next time you open Open Dot."; the next `open` or turn-on tries again, so a login with no network yet does not turn voice off. Quitting mid-download keeps voice on, and the next start resumes.
- 3.2: no soft tone on open (moved to step 5). The rail mark is step 4. "Talk to Dot" is disabled until Open Dot is ready. A shortcut with no modifier is refused unless it is F1 to F24 ("Add Ctrl, Alt or Shift to that shortcut, or use a function key."), so a plain key cannot be taken from every app. "That shortcut is already used by another app. Pick a different one." is said only when the plugin says the shortcut is already registered or in use; any other failure says "Dots could not set that shortcut.". The wake phrase is passed to the session as a setting; how it behaves on real audio is step 5.
- 3.7: voices are named "Voice 1" to "Voice 10"; the hotkey is a text field (names joined with +), not a key recorder; "which model thinks" offers Fast, Balanced and Most capable. The Open Dot switch is not part of the settings form: a settings save keeps the stored `voice_enabled`, read in core under the settings lock as it stores (`Runtime::update_settings_keeping_voice`), so a form, or a program-path dialog left open for minutes, cannot undo a turn-on or a failed download's turn-off. "The rail mark shows the open microphone" is step 4; the tray tooltip and the status line show it now. "Tell me aloud when something needs me" is stored only; the notices are step 5.
- 5.3: two more commands than the spec lists (`voice_remove`, `voice_view`), so the files can be removed and the mind sees the screen the person is on. The event carries more kinds than the spec names (`hearing`, `woke`, `trouble`, `cleared`, `status`). A saved device change rebuilds the session in the background and forgets what waited for a yes. The view's topic is known only when the address names it.

**Rulings made while building (the ledger, in order).**

- Ruling: the plan was not shown to the user before execution (standing preference; plan committed 99b24ab1).
- Ruling: the install lock became an OS file lock (`File::try_lock`) held through the blocking work, and `remove()` returns Busy during an install; this removes staleness and check-then-use races. Cost: none known (toolchain 1.95 has it).
- Ruling: a finished deed's outcome must reach the person even when its turn went stale: the mind emits `MindEvent::Did { said }` after a deed, and the bridge speaks it on the live turn when the deed's own turn is stale. Cost: a small core and bridge change, folded into Task 5.
- Ruling: the Rust fix builder ran while the UI-only Task 7 builder worked (disjoint trees; no type changes in that round, so the contract fixture was untouched; commits retried on `index.lock`), to save a serial wait. Cost if wrong: a broken commit to redo.
- Ruling: the step 3 gate waited for the Rust re-review result (fixes first).

**Known issues and deferred.**

- Fixed in the last round (just before this gate): a lapse or reset landing between a turn's last check and its start can no longer begin a dropped turn (the drop count is compared inside `begin`, under its lock); `voice_enabled` is kept from storage at the moment of the write; `open` does nothing while Dots quits; a failed resume at start keeps voice on; a shortcut with no modifier is refused unless it is a function key; a shortcut that could not be let go stays tracked, and "taken" is said only when it is; a missing voice service does not block saving settings.
- Minor (deferred): the claim that the app binary itself needs the static runtime was unproven until the service linked the speech library and CI x64 runs (see the measurement above).
- Minor (deferred, step 4): `topic_root` is null at `/home/:dotId` (the topic is chosen inside the dot pane); an approval opens `/inbox` (there is no per-approval address).
- Minor (deferred, step 4, which reworks the voice hook): the Voice tab's own status fetch can resolve after a status event and overwrite it; it should read the status only through the hook's cache with the same guard.
- Minor (deferred): an outcome can be lost when a `Heard` lands between `Did` and its `Say` (review 4c); the window's visibility is asked per level event (8b); a download task that panics leaves "downloading" set (9); `enable`, `remove` and a download's end read the whole settings and write them back (10), so a settings save landing in between can be overwritten by them.
- The real plugin registration (`PluginHotkey`) is not unit-tested; the tracking and the error sentences are tested through a fake plugin, and the service through a fake port. The hotkey, the tray and the Mac plist have not been tried live (no builder started the app with a window or a microphone).
- The greeting relies on `Session::notice`, which speaks only while Idle; the preview restores the saved voice once the session rests again, or after 5 s.
- `apply_in_background` has no command-level test (it needs Tauri state); its parts are tested in the service. `PartWriter` (the `.part` writes under the install lock) has no test of its own.
- Test-first was followed loosely in Tasks 5 and 6 (tests written with the code; the first runs passed).

**Still owed before this can ship.**

- The hand check on the real app (`docs/open-dot-hand-check.md`), on speakers and on headphones, with the time from the end of a sentence to the first word written here. No person has talked to the wired app yet.
- A CI run on Windows x64 (with the static runtime) and on Mac (the plist, the plugin and the speech library have never been built there).
- The soft tone on open (moved to step 5), the wake phrase and talking over it on real audio, and the spoken notices: step 5.
- Steps 4 to 6 of section 8 (step 4, the stage and The Breath, is under way on this branch).
