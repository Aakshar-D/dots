# Open Dot, the voice assistant: design

- Date: 2026-11-11
- Status: builder check done; written for the person's review
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

(Filled in when the work is done.)
