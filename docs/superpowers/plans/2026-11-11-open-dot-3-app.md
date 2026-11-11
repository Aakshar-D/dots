# Open Dot Step 3: App Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Dots app owns a voice session and the mind, joins them, and exposes them: commands, one `voice` event stream, settings, model download with progress, a global hotkey, a tray item, and a Voice tab in Settings. After this step a person can turn Open Dot on, download the speech files, press the hotkey and talk; the stage and The Breath (step 4) are not built yet, so what it hears and says is only audible and visible in the Voice tab's status line.

**Architecture:** A new app module `voice` with three parts: a pure `bridge` (a state machine that turns `SessionEvent`s and `MindEvent`s into calls on a session port, a mind port and UI events; tested with fakes), a `service` that builds the real `dots_voice::Session` from installed models and runs the bridge on the Tauri async runtime, and a `download` part (one install at a time, progress events, cancel). A managed `VoiceHandle` is what commands talk to. No audio passes through the web view.

**Tech Stack:** Rust (Tauri 2.12, `dots-voice`, `dots-core::open_dot`, `tauri-plugin-global-shortcut`, `tokio-util`), React + TypeScript + zod + TanStack Query + vitest.

**Spec:** `docs/superpowers/specs/2026-11-11-open-dot-design.md` (sections 3.1, 3.2, 3.6, 3.7, 5.3, 6, 7, and section 10 "what is owed").

**Recon (file:line facts for everything below):** `.superpowers/sdd/2026-11-11-open-dot-3-app/recon.md` (git-ignored; read the section your task names).

## Global Constraints

- Off until turned on: with `voice_enabled == false` no microphone is opened, no model is loaded, no hotkey is registered, and startup time does not change.
- No audio or transcript on disk or in logs. Logs may say that a turn happened and timings.
- Speech files live in `<data dir>/voice/models`; downloaded on first enable; verified by `dots_voice::models::install`.
- Every command is a plain `#[tauri::command]` `pub fn`/`pub async fn` in `apps/dots-app/src/commands.rs`, added to `command_list!` in `lib.rs`, with a wrapper in `ui/src/api/commands.ts`; new types in `ui/src/api/types.ts`; samples in `contract.rs`; fixture regenerated with `UPDATE_CONTRACT=1 cargo test -p dots-app contract`; every new fixture key gets a schema in `ui/src/api/contract.test.ts`.
- Settings fields get defaults that keep everything off; `Settings` in `types.ts` is a `z.strictObject`, so Rust and TS must change together, plus `tests/settings_test.rs` and `ui/src/test/fixtures.ts`.
- Any command the UI calls at mount (the rail, `Layout`) needs a default answer in `installDefaults()` (`ui/src/test/fixtures.ts`).
- A click on an option reaches the mind as `Mind::pick(index)`, never as spoken words. Spoken words reach it only as `Mind::hear`.
- The default hotkey is `Ctrl+Shift+Space` (`Cmd+Shift+Space` on a Mac). A failed registration is reported in Settings and the app carries on.
- Whenever the microphone is open the tray tooltip says "Dot is listening".
- Windows builds link the C runtime statically (see Task 2); the LNK4098 warning must be gone.
- Gate for every task: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; the tests the task names; for UI changes `bun run typecheck` and `bun run test` in `ui`.
- Commits: conventional messages, no co-author trailer, staged by path. Never commit `.cargo/config.toml`. One builder at a time in this worktree.
- Hand checks that open the microphone are the user's to run; builders never run them.

## Decisions this plan makes

1. **Settings fields** (all in `dots_core::Settings`): `voice_enabled: bool = false`, `voice_id: u8 = 0` (0 to 9), `voice_speed: f32 = 1.0` (0.8 to 1.4), `voice_hotkey: String = "CmdOrCtrl+Shift+Space"` (empty string means no hotkey), `voice_wake_phrase: bool = false`, `voice_interrupt_by_talking: bool = false`, `voice_mic: Option<String> = None`, `voice_speaker: Option<String> = None`, `voice_notices: bool = false`. `open_dot_model` already exists.
2. **Commands** (spec's nine plus two): `voice_status`, `voice_enable`, `voice_download_cancel`, `voice_open`, `voice_close`, `voice_stop`, `voice_pick`, `voice_preview`, `voice_devices`, `voice_remove` (delete the downloaded files), `voice_view` (the UI reports what it is showing, for the mind's picture).
3. **One event name, `voice`**, payload `VoiceEvent`, tagged by `kind`. Levels are forwarded at most 30 a second and only while the main window is visible.
4. **The session is built in the background** when `voice_enabled` and the files are installed (at startup and after enabling). Changing microphone or speaker rebuilds it; changing voice, speed, wake phrase or interrupt-by-talking is applied to the running session.
5. **Static C runtime on Windows** is set through `RUSTFLAGS` in the Windows CI jobs and documented for local builds; the user's local `.cargo/config.toml` is not tracked and is not edited by builders without saying so in the report.
6. **Wake phrase, talk-over interrupt and spoken notices** are passed through as settings here; their behaviour on real audio and the notices listener are step 5.

## Review Focus

1. Feature off: nothing voice-related runs (no model load, no mic, no hotkey, no session thread). A test asserts the service is not built when `voice_enabled` is false.
2. A click on Yes while the same answer is being spoken: exactly one `Mind::pick` or one `Mind::hear` acts; the later `Heard` wins and the earlier turn's events are dropped.
3. The stage is closed or the asking time runs out while a confirmation is pending: the mind's pending step is dropped (`lapse`), and a later "yes" approves nothing.
4. Download interrupted (cancel, network stall, app quit): the next enable resumes; two enables at once start one download.
5. Hotkey taken by another app, or microphone blocked by the system: a plain sentence in Settings; the app keeps working; no panic.

---

### Task 1: Carried fixes in `dots-voice`

Small items left by the step 1 review. **Files:** `crates/dots-voice/src/session.rs`, `voice.rs`, `models.rs`, tests beside them.

- [ ] In Thinking, after `voice.stop()` for a failed speaker, drain `voice_events` so two queued fillers give one `Trouble`. Test `one_trouble_for_two_fillers`.
- [ ] `voice.rs`: a refused clip is reported as "The speaker stopped working." only when `sink.failed()`; a full queue drops the clip quietly. Test.
- [ ] One `Trouble` per turn when no speaker can be opened (filler then reply). Test.
- [ ] `open_mic_now` resets `ears`, `short_ears` and `in_speech` as well as the wake spotter. Test with talk-over on.
- [ ] `models::install` holds a lock file (`<root>/.installing`, removed on exit, stale after 60 s without progress) so two installs into one folder cannot run at once; the second returns a new `ModelError::Busy` ("a download is already running"). Test.
- [ ] `Device`, `Progress` and `SessionSettings` stay non-serde (the app has its own DTOs).
- [ ] `cargo test -p dots-voice --locked`; session tests 10 times in a row. Commit `fix(voice): one trouble per failure, a full queue is not a broken speaker, and one install at a time`.

### Task 2: The app links the voice crate, with a static C runtime on Windows

**Files:** `apps/dots-app/Cargo.toml`, `Cargo.lock`, `.github/workflows/release.yml`, `tools/voice/README.md`, spec section 10.

- [ ] Add `dots-voice = { path = "../../crates/dots-voice" }`, `tokio-util = "0.7"`, `tauri-plugin-global-shortcut = "2"` (the newest 2.x that resolves with Tauri 2.12; record the version) to `apps/dots-app/Cargo.toml`; add `"rt"` and `"time"` to the app's tokio features if the service needs them. `cargo build -p dots-app` locally with a dummy use (`const _: fn() = || { let _ = dots_voice::Manifest::pinned; };` in a new `apps/dots-app/src/voice/mod.rs` declared from `lib.rs`).
- [ ] Measure before and after on this machine (Windows ARM64): build `dots-app` without the flag and record whether LNK4098 appears; then with `RUSTFLAGS=-C target-feature=+crt-static` in a **separate target dir** (`CARGO_TARGET_DIR=C:/dev/dots-target-static`, so the main cache is not rebuilt), record whether LNK4098 is gone, the binary size, and that `cargo test -p dots-app --locked` and `cargo test -p dots-voice --locked` pass with the flag. Start the built `dots-app.exe` with `DOTS_DATA_DIR` pointing at a temp folder for 10 seconds and confirm it stays up (no window interaction; kill it after). Delete `C:/dev/dots-target-static` afterwards and report the space freed.
- [ ] If the flag removes the warning and tests pass: add `env: RUSTFLAGS: "-C target-feature=+crt-static"` to the Windows jobs (`test` and `release`) in `release.yml` (job-level `env`, so clippy, tests and `tauri build` all use it), and a "Local Windows builds" note in `tools/voice/README.md` giving the two lines for a local `.cargo/config.toml` (`[target.'cfg(all(windows, target_env = "msvc"))'] rustflags = ["-C", "target-feature=+crt-static"]`). If it does not: stop, record the numbers, and report BLOCKED with what you saw.
- [ ] Check `Swatinem/rust-cache` picks up the env change (it hashes `RUSTFLAGS`); note it in the report.
- [ ] Commit `build(app): the app links the voice crate, and Windows builds link the C runtime statically`.

### Task 3: Settings fields

**Files:** `crates/dots-core/src/settings.rs`, `crates/dots-core/tests/settings_test.rs`, `ui/src/api/types.ts`, `ui/src/test/fixtures.ts`, `ui/src/api/__fixtures__/contract.json` (regenerated), any UI place that builds a full `Settings`.

- [ ] Add the nine fields of Decision 1 with their defaults. `validate()`: `voice_id <= 9` ("Pick one of the ten voices."), `0.8 <= voice_speed <= 1.4` and finite ("Speaking speed must be between 0.8 and 1.4."), `voice_hotkey` empty or parses as a shortcut (validate shape only here: non-empty tokens joined by `+`, at most 4, last token a key; the real parse happens in the app), device ids at most 256 bytes.
- [ ] `restart_required` unchanged (none of these need a restart).
- [ ] Tests: defaults keep everything off; round trip; each bound rejected with its sentence; an old settings table loads with defaults.
- [ ] zod: `voice_id: z.number().int()`, `voice_speed: z.number()`, `voice_mic: nullable(z.string())`, etc. Fixture regenerated; UI contract test and typecheck pass.
- [ ] Commit `feat(core): voice settings, all off by default`.

### Task 4: The bridge

**Files:** Create `apps/dots-app/src/voice/bridge.rs`, `apps/dots-app/src/voice/events.rs`; modify `apps/dots-app/src/voice/mod.rs`.

**Interfaces — Consumes:** `dots_voice::{SessionEvent, State, After}`, `dots_core::open_dot::{MindEvent, Then, Card, View}`, `dots_core::open_dot::act::OpenTarget` (read their current definitions; the mind's event order is documented on `MindEvent`). **Produces:**

```rust
// events.rs — what the UI receives on the "voice" event
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
    State { state: State, mic_open: bool },
    Level { mic: f32, voice: f32 },
    Hearing { text: String },
    Heard { text: String },
    Saying { text: String },
    Ask { question: String, options: Vec<String> },
    Card { card: Card },
    Open { target: OpenTarget },
    Cleared,                                   // options and card should go away
    Woke,
    Trouble { message: String },
    Download { done_bytes: u64, total_bytes: u64 },
    Status { status: VoiceStatus },            // sent whenever the status changes
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceStatus {
    pub enabled: bool, pub installed: bool, pub downloading: bool,
    pub ready: bool,                            // session built and running
    pub state: State, pub mic_open: bool,
    pub download_bytes: u64,                    // Manifest::pinned().total_bytes()
    pub hotkey_error: Option<String>, pub trouble: Option<String>,
}

// bridge.rs
pub trait SessionPort: Send + Sync {            // implemented for dots_voice::Session and a test fake
    fn open(&self); fn close(&self); fn interrupt(&self); fn thinking(&self);
    fn say(&self, turn: u64, text: &str, then: After);
    fn say_delta(&self, turn: u64, delta: &str);
    fn end_of_reply(&self, turn: u64, then: After);
    fn filler(&self, turn: u64, text: &str);
    fn answered(&self, text: &str) -> Option<u64>;
    fn state(&self) -> State;
}
#[async_trait] pub trait MindPort: Send + Sync { // implemented over Arc<Runtime> and a test fake
    async fn hear(&self, heard: &str, view: &View, out: &UnboundedSender<MindEvent>);
    async fn pick(&self, index: usize, view: &View, out: &UnboundedSender<MindEvent>);
    fn lapse(&self, out: &UnboundedSender<MindEvent>);
    fn cancel(&self); fn reset(&self); fn pending(&self) -> bool;
}
pub struct Bridge { /* session: Arc<dyn SessionPort>, mind: Arc<dyn MindPort>, ui: UnboundedSender<VoiceEvent>, state */ }
impl Bridge {
    pub fn new(session: Arc<dyn SessionPort>, mind: Arc<dyn MindPort>, ui: UnboundedSender<VoiceEvent>) -> Arc<Bridge>;
    pub fn on_session(self: &Arc<Self>, ev: SessionEvent);        // called for every session event, in order
    pub fn set_view(&self, view: View);
    pub fn open(&self);                                           // hotkey, rail, tray, voice_open
    pub fn close(&self);                                          // Esc / voice_close
    pub fn stop(&self);                                           // voice_stop: stop speech, listen
    pub fn pick(self: &Arc<Self>, index: usize);                  // click on option `index`
}
```

Behaviour:
- `Heard { text, turn }`: `session.thinking()`; send `VoiceEvent::Heard`; start a task for that turn: if the turn was created by `pick` (remembered index), call `mind.pick(index, ..)`, else `mind.hear(text, ..)`. A newer `Heard` makes the older turn stale: its `MindEvent`s are dropped (the session also drops text for a stale turn, by turn id).
- `MindEvent` for the live turn: `Thinking` (nothing; the session is already Thinking), `Filler{text}` to `session.filler(turn, text)`, `Say{delta}` to `session.say_delta(turn, delta)`, `Ask{question, options}`: remember the options, send `VoiceEvent::Ask`, and speak the question with `session.say_delta(turn, question)`; `SayEnd{then}` to `session.end_of_reply(turn, map(then))` with `Then::Rest/FollowUp/Ask` to `After::Rest/FollowUp/Ask`; `Card`, `Open`, `Cleared` forwarded (`Cleared` also forgets the options).
- `pick(index)`: only when options are showing and `index` is in range: `session.answered(option_text)`; remember `turn -> index` from its return value. Out of range or no options: ignored.
- `State { state: Idle, .. }` arriving while `mind.pending()` (asking time ran out, or closed): `mind.lapse(..)`.
- `close()`: `session.close()`, `mind.cancel()`, `mind.lapse(..)`, `mind.reset()`; send `Cleared`. `stop()`: `mind.cancel()`, `session.interrupt()`. `open()`: `session.open()`.
- `State`, `Level`, `Hearing`, `Saying`, `Woke`, `Trouble` forwarded as `VoiceEvent`s; `Woke` also means `open()` semantics are already applied by the session.
- No words in logs.

- [ ] Tests with fake ports (`#[tokio::test]`): `heard_goes_to_hear_and_reply_is_spoken_in_order`, `click_goes_to_pick_not_hear` (**Review Focus 2 and the click rule**), `a_newer_heard_drops_the_older_turns_events`, `ask_speaks_the_question_and_shows_options`, `idle_while_pending_lapses` (**Review Focus 3**), `close_resets_the_mind_and_clears`, `stop_cancels_and_listens`, `pick_out_of_range_is_ignored`, `filler_goes_to_filler`, `then_maps_to_after`, `card_and_open_are_forwarded`.
- [ ] Commit `feat(app): the bridge between the voice session and Open Dot's mind`.

### Task 5: The service, the download, and the commands

**Files:** Create `apps/dots-app/src/voice/service.rs`, `apps/dots-app/src/voice/download.rs`; modify `voice/mod.rs`, `lib.rs` (manage the handle in `setup` before `AppState`, shut down on `RunEvent::Exit`), `commands.rs`, `tray.rs` (quit handler shuts the session down), `contract.rs`, `error.rs` if a `ModelError` map helps; UI: `ui/src/api/commands.ts`, `types.ts`, `events.ts`, `contract.test.ts`, `ui/src/test/fixtures.ts`, fixture.

**Interfaces — Produces:**

```rust
pub struct VoiceHandle { /* Option<Arc<Service>>; None when the runtime did not start */ }
impl VoiceHandle { pub fn get(&self) -> Result<Arc<Service>, CmdError>; }
pub fn manage(app: &AppHandle, rt: Option<Arc<Runtime>>, data_dir: Option<PathBuf>);   // like phone::manage

impl Service {
    pub async fn status(&self) -> VoiceStatus;
    pub async fn enable(&self, on: bool) -> Result<VoiceStatus, CmdError>;  // saves the setting; on: download if needed, then build
    pub fn download_cancel(&self);
    pub async fn remove(&self) -> Result<VoiceStatus, CmdError>;           // turns off, stops the session, deletes the files
    pub fn open(&self); pub fn close(&self); pub fn stop(&self); pub fn pick(&self, index: usize);
    pub fn set_view(&self, view: View);
    pub async fn preview(&self, voice: u8, speed: f32) -> Result<(), CmdError>;  // speaks "Hi, I'm Dot." in that voice, then restores
    pub fn devices(&self) -> VoiceDevices;                                  // { microphones: Vec<VoiceDevice{id,name,default}>, speakers: .. }
    pub async fn apply(&self, settings: &Settings);                         // called from update_settings
    pub fn shutdown(&self);
}
```

Commands (thin wrappers): `voice_status() -> Res<VoiceStatus>`, `voice_enable(on: bool) -> Res<VoiceStatus>`, `voice_download_cancel() -> Res<()>`, `voice_remove() -> Res<VoiceStatus>`, `voice_open() -> Res<()>`, `voice_close() -> Res<()>`, `voice_stop() -> Res<()>`, `voice_pick(index: u32) -> Res<()>`, `voice_view(view: View) -> Res<()>`, `voice_preview(voice: u8, speed: f32) -> Res<()>`, `voice_devices() -> Res<VoiceDevices>`.

Rules:
- Building the session follows `crates/dots-voice/examples/session.rs`, on `spawn_blocking` (model loads and device opens block). Opener closures capture the chosen device ids. A build failure becomes `VoiceStatus.trouble` in a plain sentence ("The microphone could not be opened. Check that Dots is allowed to use it in system settings.") and is retried on the next `open`.
- Not built when `voice_enabled` is false or files are not installed (**Review Focus 1**). `voice_open` while not ready: emits `Trouble { "Open Dot is off. Turn it on in Settings, under Voice." }` or "still getting ready".
- Download: one at a time (**Review Focus 4**; a second `enable(true)` while downloading returns the current status); progress to `VoiceEvent::Download` at most 10 a second; `ModelError::Cancelled` to `CmdError::cancelled`, `Stalled`/`Network` to a sentence that says it will resume next time, `WrongHash` to "The download was damaged. Try again.", `Disk` to "There is not enough room, or the folder cannot be written." After a successful first install the session is built and says "Hi, I'm Dot." once.
- `apply(settings)`: voice/speed to `set_voice`; wake phrase/interrupt to `session.settings(..)`; mic/speaker changed: rebuild; `voice_enabled` turned off: shut the session down and release the microphone.
- Events: one task forwards `VoiceEvent`s with `app.emit("voice", &ev)`; `Level` is skipped while the main window is hidden; every status change also emits `VoiceEvent::Status`.
- `shutdown()` on exit, tray quit and `restart_app`: `session.shutdown()` then wait up to 500 ms for the event channel to close, so the device is released.
- The mind port calls `rt.open_dot().hear(&rt, ..)`; errors from `hear`/`pick` become `Trouble { "Something went wrong. Try again." }` plus `end_of_reply(turn, Rest)`.
- Contract: samples for `VoiceEvent` (one of each kind), `VoiceStatus`, `VoiceDevices`, `View`; zod schemas (`z.discriminatedUnion("kind", ..)`), `onVoice(handler)` in `ui/src/api/events.ts`; wrappers in `commands.ts`; `installDefaults()` answers `voice_status` with an all-off status and `voice_devices` with empty lists.

- [ ] Tests (Rust, no real audio: the service takes a `SessionFactory` trait so tests give it a fake session): `off_builds_nothing` (**Review Focus 1**), `enable_downloads_then_builds_and_greets` (local axum server serving a tiny manifest; the service takes the manifest as a parameter), `two_enables_one_download`, `cancel_leaves_a_resumable_part_and_status_off`, `remove_turns_off_and_deletes`, `device_change_rebuilds`, `voice_change_does_not_rebuild`, `build_failure_is_a_sentence_and_retried_on_open`, `shutdown_releases` ; `every_command_in_commands_rs_is_registered_once` and the contract test pass; UI `commands.test.ts`, `contract.test.ts`, `events.test.ts` (an `onVoice` case).
- [ ] Commit `feat(app): the voice service, speech file download, and voice commands`.

### Task 6: Hotkey, tray, Mac microphone line

**Files:** `apps/dots-app/src/voice/hotkey.rs` (new), `lib.rs` (plugin registration), `tray.rs`, `tauri.conf.json`, `voice/service.rs`.

- [ ] `tauri_plugin_global_shortcut` registered in the builder; the shortcut itself is registered from Rust only when `voice_enabled` and `voice_hotkey` is non-empty, re-registered when the setting changes, unregistered when voice is turned off. Pressing it: `crate::show_main(app)` then `service.open()` (while Speaking or Thinking the session treats `open` as interrupt-and-listen, which is spec 3.6). A parse or registration failure sets `VoiceStatus.hotkey_error` ("That shortcut is already used by another app. Pick a different one." / "That is not a shortcut Dots understands.") and nothing else changes (**Review Focus 5**).
- [ ] `pub fn parse_hotkey(s: &str) -> Result<Shortcut, String>` with tests: the default parses; `CmdOrCtrl` maps per platform; empty is "no hotkey"; nonsense gives the sentence.
- [ ] Tray: a "Talk to Dot" item after "Open" (id `talk`), enabled only when voice is ready; `on_menu` case calls `show_main` then `service.open()`. `Tray` gains `listening: AtomicBool`; `tooltip(count, limits, now, listening)` appends the line "Dot is listening" when true; `Tray::set_listening(bool)` is called by the service on every `State { mic_open }` change and rebuilds the tooltip from the last count and limits. Update the tooltip unit tests and add `tooltip_says_dot_is_listening`.
- [ ] `tauri.conf.json`: `bundle.macOS.infoPlist` (or the form Tauri 2.12 accepts; check its schema in `apps/dots-app/gen/schemas`) with `NSMicrophoneUsageDescription`: "Dots listens when you talk to Open Dot. Audio stays on this Mac."
- [ ] Tests: hotkey parse table; tooltip tests; a service test that turning voice off unregisters (through a `HotkeyPort` fake).
- [ ] Commit `feat(app): a global hotkey and a tray item start Open Dot, and the tray says when it listens`.

### Task 7: Voice tab in Settings, and the UI's voice hook

**Files:** Create `ui/src/screens/VoiceSection.tsx`, `ui/src/screens/VoiceSection.test.tsx`, `ui/src/lib/use-voice.ts`, `ui/src/lib/use-voice.test.ts`; modify `ui/src/screens/SettingsScreen.tsx` (`TABS`: `{ id: "voice", label: "Voice" }` after `phone`; `content.voice`), `ui/src/lib/query.ts` (`keys.voice`, `keys.voiceDevices`), `ui/src/lib/runtime-events.ts` or `Layout.tsx` (mount the hook once), `ui/src/test/fixtures.ts`.

- [ ] `useVoice()`: subscribes once with `onVoice`; keeps `status` in the query cache (`keys.voice`, seeded by `commands.voiceStatus()`), and exposes the latest `state`, `micOpen`, `level`, `hearing`, `saying`, `ask`, `card`, `download`, `trouble` from a small external store (no re-render of the whole app on `Level`; use `useSyncExternalStore` with selectors). On `Open { target }` it navigates (`dot` to `/dots/<id>` or the app's real dot route, `approval` and `inbox` to the inbox route, `settings`, `usage`, `home`; read `ui/src/app/routes.tsx`). It reports the view with `commands.voiceView(...)` whenever account, workspace, route, viewed dot or open topic changes (find where `set_viewing` is called and where the current scope lives, and reuse them).
- [ ] `VoiceSection` (copy the shape of `PhoneSection.tsx`): 
  - "Open Dot" switch. Off and not installed: two lines, "Open Dot lets you talk to Dots and hear it answer. Turning it on downloads about 260 MB of speech files once; they stay on this computer." Turning on calls `voiceEnable(true)`; while downloading shows a progress bar with MB done of total and a Cancel button; errors as `notifyError`.
  - A status line: Ready / Getting ready / Off / the trouble sentence; and, while the microphone is open, "Listening".
  - Voice: ten choices ("Voice 1" to "Voice 10") each with a play button (`voicePreview(i, speed)`); speaking speed slider 0.8 to 1.4 step 0.05.
  - Hotkey: a text field showing the shortcut, with the hotkey error under it when there is one, and a "Clear" button.
  - "Hey Dot" switch with the note: "The microphone stays on while Dots is running so it can hear the phrase. Audio never leaves this computer."
  - "Interrupt by just talking" switch with the note "Best with headphones."
  - Microphone and speaker selects (from `voiceDevices()`, first entry "System default").
  - "Which model thinks" select bound to `open_dot_model` (haiku "Fast", sonnet "Balanced", opus "Most capable"; use the model ids the existing model pickers in Settings use).
  - "Tell me aloud when something needs me" switch.
  - "Remove speech files" (`ConfirmButton`; `voiceRemove()`), shown when installed.
  - Settings fields go through the existing draft/save form (`set({...})`); enable, cancel, preview and remove act at once.
- [ ] Tests (vitest, `backend.on(...)` and `backend.emit("voice", ...)`): off state explains the download; turning on calls `voice_enable` and shows progress from `download` events; cancel calls `voice_download_cancel`; ready state shows controls; preview calls `voice_preview` with voice and speed; hotkey error is shown; changing the microphone marks the form unsaved and saves through `update_settings`; remove asks first; `useVoice` navigates on `open` and reports the view.
- [ ] `bun run typecheck`, `bun run test`. Commit `feat(ui): a Voice tab turns Open Dot on, downloads its speech files, and sets how it sounds`.

### Task 8: Gate, hand-check script, as built

- [ ] Full gate: fmt, workspace clippy, `cargo test -p dots-core --locked -- --skip preview_schedule`, `cargo test -p dots-voice --locked`, `cargo test -p dots-app --locked`, node tests in `tools/voice` and `tools/release`, `bun run typecheck`, `bun run test`. `tests/resume_test.rs` flakes under load: re-run alone before calling it a failure.
- [ ] `docs/open-dot-hand-check.md`: the steps the user runs on the real app after this step (turn on, download, hotkey, say "what needs me", cancel a download and resume, change voice, unplug the headset mid-reply, hotkey taken), with what should happen at each.
- [ ] Spec section 10: "### Step 3: app wiring" (what was built, the decisions above, the static-runtime numbers, deferred minors, what is owed). Status line updated. Commit `docs(spec): Open Dot step 3 as built`.
