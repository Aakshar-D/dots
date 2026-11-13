# Open Dot step 1 (ears and voice) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a new crate `dots-voice` that hears, speaks, spots the wake phrase and runs the listen/think/speak state machine, all on the person's machine, with nothing in the app using it yet.

**Architecture:** every engine (speak, hear, find speech, wake phrase, microphone, speaker) sits behind a small trait. Pure logic (text cleaning, sentence splitting, end-of-sentence detection, the speaking queue, the session state machine) is written against those traits and tested with fakes. Real engines wrap the `sherpa-onnx` crate and `cpal`. Models are downloaded on demand from a pinned manifest and verified by SHA-256.

**Tech Stack:** Rust (workspace toolchain 1.95), `sherpa-onnx = "=2.0.1"` (default static linking), `cpal = "0.18"`, `tokio`, `reqwest` (same features as `dots-core`), `sha2`, `bzip2`, `tar`, `thiserror = "2"`, `serde`, `tracing`. Dev: `tempfile`, `axum = "0.8"`.

**Spec:** `docs/superpowers/specs/2026-11-11-open-dot-design.md` (sections 5.1, 6, 7).

## Global Constraints

- No audio sample and no heard or spoken text is written to disk or to logs. Logs may carry state names and durations only.
- Nothing is loaded from a model file whose SHA-256 does not match the manifest.
- English only.
- The crate must build on `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`, `x86_64-apple-darwin`. Local builds on this machine need `C:\Program Files\LLVM\bin` on `PATH`.
- `cargo fmt --all --check` and `cargo clippy -p dots-voice --all-targets --locked -- -D warnings` pass after every task.
- Tests must not need a microphone, a speaker, the network, or the models. Tests that need the real models are `#[ignore]` and read the folder from `DOTS_VOICE_MODELS`.
- Commit messages: `feat|fix|test|docs(voice): <plain sentence>`. No co-author trailer.
- Work in `C:\dev\dots-open-dot` on branch `feat/open-dot`. Never commit `.cargo/config.toml`.
- Do not run `cargo clean`.

## Review Focus

1. The person stops talking mid-word, or coughs: a clip shorter than 0.25 s of speech must not become a turn.
2. `stop()` while a sentence is being synthesised: no sound from that sentence may play afterwards.
3. A download is cut at any byte: the next start resumes or restarts and ends with verified files; a half-unpacked model folder is never reported as installed.
4. The reply text is empty, only markdown, or only a code block: the speaker says nothing or "(code is on screen)" and the session still returns to idle.
5. The microphone disappears (unplugged) while listening: the session reports `Trouble` and goes to `Idle`; it does not hang or panic.

---

### Task 1: Crate and spoken-text helpers

**Files:**
- Create: `crates/dots-voice/Cargo.toml`, `crates/dots-voice/src/lib.rs`, `crates/dots-voice/src/text.rs`
- Modify: `Cargo.toml` (workspace `members`: add `"crates/dots-voice"`; leave `default-members` alone)

**Interfaces:**
- Produces:
  - `pub fn speakable(text: &str) -> String`
  - `pub struct SentenceStream` with `pub fn new() -> Self`, `pub fn push(&mut self, delta: &str) -> Vec<String>`, `pub fn finish(&mut self) -> Vec<String>`
  - `pub const MAX_SPOKEN_CHARS: usize = 600`
  - `pub fn trim_spoken(text: &str) -> String`

- [ ] **Step 1: Write the failing tests** in `text.rs` (`#[cfg(test)] mod tests`):

```rust
#[test]
fn markdown_links_code_and_emoji_are_not_spoken() {
    assert_eq!(speakable("**Done.** See [the report](https://x.test/r)."), "Done. See the report.");
    assert_eq!(speakable("Run `cargo test` now"), "Run cargo test now");
    assert_eq!(speakable("Here:\n```rust\nfn main() {}\n```\nThat is all."), "Here: (code is on screen) That is all.");
    assert_eq!(speakable("- one\n- two"), "one. two.");
    assert_eq!(speakable("Great 🎉 news"), "Great news");
    assert_eq!(speakable("# Title\nBody"), "Title. Body.");
    assert_eq!(speakable("https://example.test/a/b"), "(link is on screen)");
    assert_eq!(speakable("   "), "");
}

#[test]
fn sentences_come_out_as_soon_as_they_end() {
    let mut s = SentenceStream::new();
    assert!(s.push("Sent to Re").is_empty());
    assert_eq!(s.push("search. It will"), vec!["Sent to Research."]);
    assert_eq!(s.push(" start at 8.30 a.m. tomorrow! Ok"), vec!["It will start at 8.30 a.m. tomorrow!"]);
    assert_eq!(s.finish(), vec!["Ok"]);
    assert!(s.finish().is_empty());
}

#[test]
fn a_long_reply_is_cut_at_a_sentence_end() {
    let long = "This is a sentence. ".repeat(60);
    let cut = trim_spoken(&long);
    assert!(cut.len() <= MAX_SPOKEN_CHARS);
    assert!(cut.ends_with('.'));
}
```

- [ ] **Step 2: Run** `cargo test -p dots-voice text` — expected: fails to compile (functions missing).
- [ ] **Step 3: Implement.** `speakable`: strip fenced code to `(code is on screen)`, bare URLs to `(link is on screen)`, markdown link to its label, inline code/bold/italic/heading markers removed, list items and lines joined as sentences each ending in `.` unless they already end in `.!?:`, characters outside letters, digits, common punctuation and spaces dropped, whitespace collapsed. `SentenceStream`: a sentence ends at `.`, `!` or `?` followed by whitespace, except when the `.` follows a single letter (`a.m.`, `e.g.`) or sits between digits.
- [ ] **Step 4: Run** `cargo test -p dots-voice text` — expected: 3 passed.
- [ ] **Step 5: Commit** `feat(voice): a new crate, with text made fit to be spoken`.

### Task 2: Model manifest and downloader

**Files:**
- Create: `crates/dots-voice/src/models.rs`
- Modify: `crates/dots-voice/src/lib.rs`

**Interfaces:**
- Produces:

```rust
pub struct ModelFile { pub name: &'static str, pub url: &'static str, pub bytes: u64, pub sha256: &'static str, pub unpack: bool }
pub struct Manifest { pub files: Vec<ModelFile> }
impl Manifest { pub fn pinned() -> Self; pub fn total_bytes(&self) -> u64 }
pub struct ModelPaths { pub root: PathBuf }   // accessors below
impl ModelPaths { pub fn speak_dir(&self) -> PathBuf; pub fn hear_dir(&self) -> PathBuf; pub fn wake_dir(&self) -> PathBuf; pub fn vad_file(&self) -> PathBuf }
pub fn installed(root: &Path, manifest: &Manifest) -> bool
pub struct Progress { pub done_bytes: u64, pub total_bytes: u64, pub file: String }
#[derive(thiserror::Error, Debug)] pub enum ModelError { Cancelled, Network(String), WrongHash { file: String }, Disk(String) }
pub async fn install(root: &Path, manifest: &Manifest, on_progress: impl Fn(Progress) + Send + Sync, cancel: tokio_util::sync::CancellationToken) -> Result<(), ModelError>
pub fn remove(root: &Path) -> std::io::Result<()>
```

- `Manifest::pinned()` values (base `https://github.com/k2-fsa/sherpa-onnx/releases/download/`):

| name | url (after base) | bytes | sha256 | unpack |
|---|---|---|---|---|
| `speak` | `tts-models/sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2` | 128774318 | `82fa96f91c4ef8abaae3a14a3f4153facf88bed821d1f7331cec2700f432c427` | true |
| `hear` | `asr-models/sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2` | 111266225 | `43232c1d13013d37317163baec3135bd771a186a4356f28c889bab453bb0e891` | true |
| `wake` | `kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2` | 17626723 | `f170013b4716e41b62b9bfd809687c207cef798ef9bc6534d524e17af9b6561a` | true |
| `vad` | `asr-models/silero_vad.onnx` | 643854 | `9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6` | false |

- Layout under `root`: `<name>/` for unpacked archives (the archive's single top folder is flattened away), `vad/silero_vad.onnx`, and `<name>.ok` written last, holding the sha256. `installed` is true only when every `.ok` exists and matches the manifest. Downloads go to `<name>.part` and resume with an HTTP `Range` request; the hash is checked over the whole file before unpacking; unpacking goes to `<name>.tmp/` and is renamed into place.

- [ ] **Step 1: Write the failing tests** (`models.rs` tests; serve bytes from an in-test `axum` server on `127.0.0.1:0` that honours `Range`; build a small `.tar.bz2` in the test with one top folder and two files):
  - `a_good_download_installs_and_reports_progress`: `installed` false before, true after; last `Progress.done_bytes == total_bytes`; unpacked file contents equal the originals; no `.part` or `.tmp` left.
  - `a_wrong_hash_installs_nothing`: returns `ModelError::WrongHash`, `installed` false, no folder for that name.
  - `a_cut_download_resumes`: server drops the connection after half the bytes on the first request; a second `install` succeeds and the server saw a `Range` header.
  - `cancel_stops_and_leaves_no_install`: token cancelled from the first progress callback → `ModelError::Cancelled`, `installed` false.
  - `an_archive_that_escapes_its_folder_is_refused`: a tar entry named `../evil` → `ModelError::Disk`, nothing written outside `root`.
  - `the_pinned_manifest_is_whole`: 4 files, every sha256 is 64 hex characters, `total_bytes() == 258311120`.
- [ ] **Step 2: Run** `cargo test -p dots-voice models` — expected: compile failure.
- [ ] **Step 3: Implement** with `reqwest` streaming, `sha2::Sha256`, `bzip2::read::BzDecoder` + `tar::Archive` (unpack inside `spawn_blocking`; reject any entry whose path is absolute or contains `..`).
- [ ] **Step 4: Run** `cargo test -p dots-voice models` — expected: 6 passed.
- [ ] **Step 5: Commit** `feat(voice): speech models are fetched once, checked by hash, and resumable`.

### Task 3: Engine traits and the real engines

**Files:**
- Create: `crates/dots-voice/src/engine.rs`, `crates/dots-voice/src/sherpa.rs`, `crates/dots-voice/tests/real_engines.rs`
- Modify: `crates/dots-voice/src/lib.rs`

**Interfaces:**
- Consumes: `ModelPaths` (Task 2).
- Produces (`engine.rs`):

```rust
pub struct Clip { pub samples: Vec<f32>, pub rate: u32 }               // mono, -1.0..=1.0
pub trait Speaker: Send + Sync { fn speak(&self, sentence: &str, voice: u8, speed: f32) -> Result<Clip, EngineError>; fn voices(&self) -> u8; }
pub trait Recognizer: Send + Sync { fn transcribe(&self, clip: &Clip) -> Result<String, EngineError>; }
pub trait SpeechDetector: Send { fn is_speech(&mut self, frame16k: &[f32]) -> bool; fn reset(&mut self); }   // frame = 512 samples at 16 kHz
pub trait WakeSpotter: Send { fn heard(&mut self, frame16k: &[f32]) -> Option<WakeWord>; fn reset(&mut self); }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum WakeWord { HeyDot, DotStop }
#[derive(thiserror::Error, Debug)] pub enum EngineError { #[error("speech models are not installed")] NotInstalled, #[error("{0}")] Failed(String) }
pub const FRAME: usize = 512;
pub const EAR_RATE: u32 = 16_000;
pub fn resample(clip: &Clip, rate: u32) -> Clip;
```

- Produces (`sherpa.rs`): `SherpaSpeaker::load(&ModelPaths, threads: i32)`, `SherpaRecognizer::load(&ModelPaths)`, `SherpaDetector::load(&ModelPaths)`, `SherpaWake::load(&ModelPaths)`, each `-> Result<Self, EngineError>`.
- Pinned settings (from the 2026-11-11 measurements):
  - Speaker: `OfflineTtsSupertonicModelConfig` with files `duration_predictor.int8.onnx`, `text_encoder.int8.onnx`, `vector_estimator.int8.onnx`, `vocoder.int8.onnx`, `tts.json`, `unicode_indexer.bin`, `voice.bin`; `GenerationConfig { sid: voice, speed, num_steps: 5, extra: {"lang": "en"} }`; output 44 100 Hz; 10 voices.
  - Recognizer: `OfflineMoonshineModelConfig { encoder: encoder_model.ort, merged_decoder: decoder_model_merged.ort }`, `tokens.txt`, 2 threads.
  - Detector: `SileroVadModelConfig { threshold: 0.5, min_silence_duration: 0.1, min_speech_duration: 0.1, window_size: 512, .. }` at 16 kHz; `is_speech` feeds one frame and returns `detected()`.
  - Wake: `KeywordSpotterConfig` over `encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx`, `decoder-epoch-12-avg-2-chunk-16-left-64.onnx`, `joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx`, `tokens.txt`, 1 thread, `keywords_buf = "▁HE Y ▁DO T :1.5 #0.2 @HEY_DOT\n▁DO T ▁S TO P :1.5 #0.2 @DOT_STOP"`. Before using the second line, check each of its tokens is in `tokens.txt`; if one is missing, respell it from tokens that are there and note the spelling in the commit body.

- [ ] **Step 1: Write the failing tests.**
  - Unit (`engine.rs`): `resample_keeps_duration` (1 s of 44 100 Hz → 16 000 samples ±1; a 440 Hz sine keeps peak within 5%).
  - Unit (`sherpa.rs`): `loading_without_models_says_not_installed` (empty temp dir → `EngineError::NotInstalled` for all four).
  - Ignored integration (`tests/real_engines.rs`, each `#[ignore]`, models from `DOTS_VOICE_MODELS`):
    - `it_hears_what_it_says`: speak "I created a dot called Morning Brief." with voices 0 and 5, transcribe, lower-cased result contains "morning brief".
    - `hey_dot_is_caught_and_near_misses_are_not`: speak "Hey Dot, what needs me?" → `Some(HeyDot)`; "Hey Dad, I connected the dots on that report." → `None`; "Dot, stop." → `Some(DotStop)`; over voices 0, 3, 7.
    - `silence_is_not_speech_and_speech_is`: 1 s of zeros → no frame is speech; a spoken clip → at least 10 speech frames.
    - `a_short_sentence_is_ready_fast`: "Okay, I'm on it." synthesised in under 1500 ms after one warm-up call.
- [ ] **Step 2: Run** `cargo test -p dots-voice` — expected: compile failure.
- [ ] **Step 3: Implement** the traits, `resample` (linear interpolation is enough), and the four wrappers. The sherpa handles are not `Sync`; wrap each in a `Mutex`.
- [ ] **Step 4: Run** `cargo test -p dots-voice` — expected: all non-ignored pass. Then, with the models folder from the scratchpad copied into the manifest layout (or installed with a one-off `install` call), run `DOTS_VOICE_MODELS=<folder> cargo test -p dots-voice --test real_engines -- --ignored` — expected: 4 passed. Record the timings in the commit body.
- [ ] **Step 5: Commit** `feat(voice): speaking, hearing, speech detection and the wake phrase behind small traits`.

### Task 4: Ears (when a sentence starts and ends)

**Files:**
- Create: `crates/dots-voice/src/ears.rs`
- Modify: `crates/dots-voice/src/lib.rs`

**Interfaces:**
- Consumes: `SpeechDetector`, `Recognizer`, `Clip`, `FRAME`, `EAR_RATE`.
- Produces:

```rust
pub struct EarSettings { pub end_quiet_ms: u32, pub min_speech_ms: u32, pub pre_roll_ms: u32, pub partial_every_ms: u32, pub max_clip_ms: u32 }
impl Default for EarSettings { /* 700, 250, 300, 400, 30_000 */ }
impl EarSettings { pub fn short_answer() -> Self /* end_quiet_ms 450, rest default */ }
#[derive(Debug, PartialEq)] pub enum EarEvent { Started, Partial(String), Final(String), Dropped }
pub struct Ears { /* private */ }
impl Ears {
    pub fn new(detector: Box<dyn SpeechDetector>, recognizer: Arc<dyn Recognizer>, settings: EarSettings) -> Self;
    pub fn push(&mut self, frame16k: &[f32]) -> Vec<EarEvent>;   // exactly FRAME samples
    pub fn reset(&mut self);
}
pub fn level(frame: &[f32]) -> f32;   // 0.0..=1.0, RMS mapped so normal speech sits around 0.3-0.7
```

- Behaviour: quiet frames are kept in a ring of `pre_roll_ms`. The first speech frame emits `Started` and begins a clip that starts with the pre-roll. While in a clip, every `partial_every_ms` of new audio emits `Partial(text)` if the text changed. After `end_quiet_ms` of non-speech: if the clip holds less than `min_speech_ms` of speech frames emit `Dropped`, else `Final(text)`; a `Final` with empty text is emitted as `Dropped`. At `max_clip_ms` the clip is ended as if quiet had come. `push` transcribes on the calling thread (the session calls it from its own worker).

- [ ] **Step 1: Write the failing tests** with a scripted fake detector (a `Vec<bool>` per frame) and a fake recognizer that returns `"<n> frames"` for a clip of n frames:
  - `quiet_makes_no_events`
  - `speech_then_quiet_makes_started_partials_final` (asserts order, and that the final clip length = pre-roll frames + speech frames + trailing quiet frames up to the end)
  - `a_cough_is_dropped` (5 speech frames = 160 ms → `Started` then `Dropped`)
  - `a_pause_shorter_than_the_end_quiet_does_not_end_the_sentence` (600 ms gap inside speech → one `Final`)
  - `short_answer_settings_end_sooner` (500 ms gap ends it)
  - `a_very_long_clip_is_ended_at_the_cap`
  - `an_empty_transcript_is_dropped`
  - `level_of_silence_is_zero_and_of_full_scale_is_one`
- [ ] **Step 2: Run** `cargo test -p dots-voice ears` — expected: compile failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p dots-voice ears` — expected: 8 passed.
- [ ] **Step 5: Commit** `feat(voice): it knows when you start and stop talking`.

### Task 5: The speaking queue

**Files:**
- Create: `crates/dots-voice/src/voice.rs`
- Modify: `crates/dots-voice/src/lib.rs`

**Interfaces:**
- Consumes: `Speaker`, `Clip`, `speakable`, `SentenceStream`, `trim_spoken`.
- Produces:

```rust
pub trait Sink: Send + Sync {
    fn play(&self, clip: Clip);            // queues; returns at once
    fn stop(&self);                        // drops everything queued and playing, at once
    fn idle(&self) -> bool;                // true when nothing is queued or playing
    fn level(&self) -> f32;                // 0.0..=1.0 of what is sounding now
}
#[derive(Clone, Debug, PartialEq)] pub enum VoiceEvent { Saying(String), Finished, Stopped, Trouble(String) }
pub struct Voice { /* private */ }
impl Voice {
    pub fn new(speaker: Arc<dyn Speaker>, sink: Arc<dyn Sink>, events: tokio::sync::mpsc::UnboundedSender<VoiceEvent>) -> Self;
    pub fn set_voice(&self, voice: u8, speed: f32);          // speed clamped to 0.8..=1.4
    pub fn say(&self, text: &str);                           // whole reply
    pub fn say_delta(&self, delta: &str);                    // streamed reply
    pub fn end_of_reply(&self);                              // flushes the last partial sentence
    pub fn stop(&self);
    pub fn speaking(&self) -> bool;
}
```

- Behaviour: one worker thread owns the order. Each sentence is `speakable`-cleaned (empty results are skipped), `Saying(sentence)` is sent when its clip is handed to the sink, `Finished` once after `end_of_reply` (or `say`) when the sink goes idle. `stop` bumps a generation counter: any clip synthesised for an older generation is thrown away, the sink is stopped, `Stopped` is sent once. A speaker error sends `Trouble` and the reply ends. Total spoken text per reply is capped with `trim_spoken`.

- [ ] **Step 1: Write the failing tests** with a fake speaker (records sentences, optional per-call delay through a barrier) and a fake sink (records clips, `idle` settable):
  - `a_reply_is_spoken_sentence_by_sentence_in_order`
  - `streamed_text_starts_speaking_before_the_reply_ends` (first `Saying` arrives after the delta that completes sentence one, before `end_of_reply`)
  - `stop_during_synthesis_plays_nothing_from_that_sentence` (speaker blocked on a barrier; `stop`; release → sink got no clip; one `Stopped`)
  - `a_new_reply_after_stop_is_spoken`
  - `only_markdown_or_empty_text_says_nothing_and_finishes`
  - `a_code_only_reply_says_code_is_on_screen`
  - `a_speaker_error_reports_trouble_and_ends`
  - `speed_is_clamped`
- [ ] **Step 2: Run** `cargo test -p dots-voice voice` — expected: compile failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p dots-voice voice` — expected: 8 passed.
- [ ] **Step 5: Commit** `feat(voice): replies are spoken a sentence at a time and stop at once`.

### Task 6: Microphone and speaker

**Files:**
- Create: `crates/dots-voice/src/audio.rs`, `crates/dots-voice/examples/talk.rs`
- Modify: `crates/dots-voice/src/lib.rs`

**Interfaces:**
- Consumes: `Sink`, `Clip`, `FRAME`, `EAR_RATE`, `resample`.
- Produces:

```rust
pub struct Device { pub id: String, pub name: String, pub default: bool }
pub fn microphones() -> Vec<Device>;
pub fn speakers() -> Vec<Device>;
pub trait Mic: Send { fn frames(&mut self) -> std::sync::mpsc::Receiver<MicFrame>; }   // taken once
pub enum MicFrame { Audio(Vec<f32>), Lost(String) }                                    // Audio = FRAME samples, 16 kHz mono
pub struct CpalMic; impl CpalMic { pub fn open(device_id: Option<&str>) -> Result<Self, EngineError>; }   // dropping it closes the microphone
pub struct CpalSink; impl CpalSink { pub fn open(device_id: Option<&str>) -> Result<Self, EngineError>; }
impl Sink for CpalSink { /* ... */ }
pub fn frames_from(samples: &[f32], rate: u32, channels: u16, carry: &mut Vec<f32>) -> Vec<Vec<f32>>;   // pure: downmix, resample to 16 kHz, cut into FRAME pieces, remainder kept in carry
```

- Behaviour: `CpalMic` converts whatever the device gives (any rate, any channel count, i16 or f32) with `frames_from`; a stream error sends `MicFrame::Lost` once and ends. `CpalSink` resamples each clip to the device rate, plays clips back to back, applies a 5 ms fade at `stop` so there is no click, and tracks `level` from the samples just played. `cpal::Stream` is not `Send` on every platform: keep each stream on a thread the struct owns.
- `examples/talk.rs`: loads models from `DOTS_VOICE_MODELS`, says "I'm listening.", then repeats back each sentence it hears ("You said: ..."), printing nothing of what was heard; Ctrl+C ends. This is the hand check for this task.

- [ ] **Step 1: Write the failing tests** for `frames_from`: `stereo_48k_becomes_mono_16k_frames` (48 000 stereo samples of a constant 0.5/−0.5 pair → mono 0.0; 1 s gives 31 frames with 128 samples carried), `the_remainder_is_carried_to_the_next_call` (two calls give the same frames as one call with the joined input), `already_16k_mono_is_only_cut`.
- [ ] **Step 2: Run** `cargo test -p dots-voice audio` — expected: compile failure.
- [ ] **Step 3: Implement** `frames_from`, the device lists, `CpalMic`, `CpalSink`, and the example.
- [ ] **Step 4: Run** `cargo test -p dots-voice audio` — expected: 3 passed. Then `cargo build -p dots-voice --example talk` — expected: builds. Do not run the example in an automated step (it opens the microphone); say in the task report that the hand check is owed.
- [ ] **Step 5: Commit** `feat(voice): the microphone and the speaker`.

### Task 7: The session state machine

**Files:**
- Create: `crates/dots-voice/src/session.rs`
- Modify: `crates/dots-voice/src/lib.rs`

**Interfaces:**
- Consumes: `Ears`, `EarSettings`, `EarEvent`, `level`, `Voice`, `VoiceEvent`, `Sink`, `Mic`, `MicFrame`, `WakeSpotter`, `WakeWord`.
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum State { Off, Idle, Listening, Thinking, Speaking, Asking }
#[derive(Clone, Debug, PartialEq, serde::Serialize)] #[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    State { state: State, mic_open: bool },
    Level { mic: f32, voice: f32 },                 // at most 30 a second, only while Listening/Asking/Speaking
    Hearing { text: String },                       // live words
    Heard { text: String },                         // a finished sentence from the person: the app sends it to the mind
    Saying { text: String },
    Woke,                                           // "Hey Dot" while Idle
    Trouble { message: String },
}
pub struct SessionSettings { pub wake_phrase: bool, pub interrupt_by_talking: bool, pub follow_up_secs: u32 /* 6 */, pub reopen_after_ms: u32 /* 250 */ }
pub struct Parts { pub ears: Ears, pub short_ears: Ears, pub voice: Voice, pub sink: Arc<dyn Sink>, pub wake: Option<Box<dyn WakeSpotter>>, pub open_mic: Box<dyn Fn() -> Result<Box<dyn Mic>, EngineError> + Send> }
pub struct Session { /* a handle; cheap to clone */ }
impl Session {
    pub fn start(parts: Parts, settings: SessionSettings, events: tokio::sync::mpsc::UnboundedSender<SessionEvent>) -> Self;
    pub fn open(&self);                              // Idle -> Listening
    pub fn close(&self);                             // any -> Idle; stops speech
    pub fn interrupt(&self);                         // Speaking/Thinking -> Listening; stops speech
    pub fn thinking(&self);                          // the app took a Heard and is asking the mind
    pub fn say(&self, text: &str, then: After);      // whole reply
    pub fn say_delta(&self, delta: &str);
    pub fn end_of_reply(&self, then: After);
    pub fn settings(&self, settings: SessionSettings);
    pub fn shutdown(&self);                          // -> Off; closes the microphone
    pub fn state(&self) -> State;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum After { Rest, FollowUp, Ask }   // Ask = wait for an answer with short-answer ears, no time limit of its own
```

- Rules (each is a test):
  1. `Idle`: the microphone is closed unless `wake_phrase` is on; with it on, frames go only to the wake spotter; `HeyDot` emits `Woke` and moves to `Listening`.
  2. `Listening`: frames go to the ears; `Partial` → `Hearing`; `Final` → `Heard` and the state becomes `Thinking`; `Dropped` keeps listening. With no speech for `follow_up_secs` after a `FollowUp` reply (or 8 s after `open`), go to `Idle`.
  3. `Thinking`: the microphone stays open only for the wake spotter (`DotStop`/`HeyDot` → `interrupt`).
  4. `Speaking`: frames go only to the wake spotter (`HeyDot` or `DotStop` → `interrupt`), unless `interrupt_by_talking` is on, in which case an ears `Started` also interrupts. No frame taken while speaking is ever given to the recogniser when `interrupt_by_talking` is off.
  5. When speech finishes (`VoiceEvent::Finished` and the sink idle), wait `reopen_after_ms`, clear the ears, then: `Rest` → `Idle`; `FollowUp` → `Listening`; `Ask` → `Asking`.
  6. `Asking` is `Listening` with the short-answer ears and a 20 s limit before `Idle`.
  7. `close`, `interrupt`, `shutdown` always stop speech first.
  8. `MicFrame::Lost` or a failed `open_mic` → `Trouble` then `Idle`.
  9. Every state change emits `State` with a true `mic_open` exactly when a microphone stream exists.
- The timers take time from an injectable clock (`trait Clock { fn now(&self) -> Instant }`, a `Session::start_with_clock` for tests) so tests do not sleep.

- [ ] **Step 1: Write the failing tests**, one per rule above, with fakes for every part (scripted mic frames, scripted detector, fake recogniser, fake speaker, fake sink, fake wake spotter, fake clock). Add: `words_spoken_by_dot_are_never_transcribed` (rule 4: the fake recogniser panics if called while `Speaking` with `interrupt_by_talking` off), `mic_is_closed_in_idle_without_the_wake_phrase` (the `open_mic` call count stays 0 until `open`), and `the_mic_unplugged_while_listening_reports_trouble_and_rests` (Review Focus 5).
- [ ] **Step 2: Run** `cargo test -p dots-voice session` — expected: compile failure.
- [ ] **Step 3: Implement** as one worker thread that selects over a command channel, the microphone frames, the voice events and a tick.
- [ ] **Step 4: Run** `cargo test -p dots-voice` — expected: every non-ignored test passes; then `cargo clippy -p dots-voice --all-targets --locked -- -D warnings` and `cargo fmt --all --check` — expected: clean.
- [ ] **Step 5: Commit** `feat(voice): the listen, think, speak session`.

### Task 8: CI and the hand check

**Files:**
- Modify: `.github/workflows/release.yml` (test job: add `cargo test -p dots-voice --locked` after the `dots-core` test line), `.github/workflows/mac-check.yml` (same line; and make its path filter include `crates/dots-voice/**` if it filters by path)
- Modify: `docs/superpowers/specs/2026-11-11-open-dot-design.md` section 10 (As built: step 1 notes)

- [ ] **Step 1:** Add the CI lines.
- [ ] **Step 2:** Run the full local gate: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test -p dots-voice --locked`, `cargo test -p dots-core --locked -- --skip preview_schedule`. Expected: all pass. (The clippy run covers `dots-app`, which needs the built UI: run `bun install --frozen-lockfile` and `bun run build` in `ui/` first if `ui/dist` is missing.)
- [ ] **Step 3:** Write into "As built": the crate layout, the pinned model table, the real-engine timings from Task 3, anything that differed from this plan, and "hand check owed: `cargo run -p dots-voice --example talk` on speakers and headphones".
- [ ] **Step 4: Commit** `docs(spec): Open Dot step 1 as built, and CI tests the voice crate`.
