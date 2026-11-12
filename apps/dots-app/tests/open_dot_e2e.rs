//! The whole of Open Dot, end to end: a spoken sentence goes in at the microphone and a spoken
//! reply comes out at the speaker, through the real voice service (built by the real factory
//! on the real models), the real session, the real bridge, the real mind and the real runtime.
//! Only the two ends are fakes: the microphone is fed speech made by the real speaker engine,
//! frame by frame at a microphone's pace, and the speaker is a recorder whose clips are
//! transcribed by the real recogniser. The Claude CLI is the scripted fake (`fake-claude` of
//! `dots-core`), whose script is written before each turn with the reply a model would give.
//! Nothing here opens a real audio device: the service gets the test's own openers.
//!
//! Ignored by default; they need the speech models and the fake CLI built first:
//!
//! ```text
//! cargo build -p dots-core --bin fake-claude
//! DOTS_VOICE_MODELS=C:\dev\dots-voice-models cargo test -p dots-app --locked --test open_dot_e2e -- --ignored --test-threads=1 --nocapture --skip real_cli_timing
//! ```
//!
//! (With `--release`, build `fake-claude` with `--release` too; `DOTS_FAKE_CLAUDE` names it
//! anywhere else.) Each test prints its turns' timings; the last, `zz_timings`, prints them
//! all with the median and the worst. The person speaks in voice 0 throughout: it is the voice
//! in which every sentence and short answer here was heard right (checked over the ten
//! voices: "Approve it." was heard right in 7 of 10, "Create it." in 8 of 10, every other
//! sentence in 10 of 10 or 9 of 10, and voice 0 heard all of them). The listening tone is off,
//! as in the session tests of `dots-voice`, so the recorder holds only Dot's words; the wake
//! test turns it on, as the app has it. "End of the sentence" is the last loud sample fed, not
//! the end of the speaker's padding of silence after it.
//!
//! `real_cli_timing` uses the real Claude CLI and spends a few cents of the person's own
//! sign-in: run it only by name.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dots_app::testing::{self, OpenMic, OpenSink, Service, VoiceEvent};
use dots_core::model::{ApprovalStatus, Dot, DotSpec, Message, Run, RunStatus, WorkspaceMode};
use dots_core::open_dot::act::OpenTarget;
use dots_core::open_dot::picture::{Picture, View};
use dots_core::open_dot::Card;
use dots_core::{Config, Runtime};
use dots_voice::{
    resample, Clip, EarEvent, EarSettings, Ears, Mic, MicFrame, ModelPaths, Recognizer,
    SherpaDetector, SherpaRecognizer, SherpaSpeaker, Sink, Speaker, State, EAR_RATE, FRAME,
};
use serde_json::{json, Value};
use tempfile::TempDir;

// ---- the two ends ---------------------------------------------------------------------------

/// The newest microphone stream's sender: frames sent while it is closed are lost, as on a
/// real one.
type MicSlot = Arc<Mutex<Option<Sender<MicFrame>>>>;

struct FakeMic(Option<Receiver<MicFrame>>);

impl Mic for FakeMic {
    fn frames(&mut self) -> Receiver<MicFrame> {
        self.0.take().unwrap_or_else(|| channel().1)
    }
}

/// A speaker that keeps every clip it is asked to play, and is busy for as long as each would
/// sound.
#[derive(Default)]
struct Recorder {
    clips: Mutex<Vec<(Instant, Clip)>>,
    busy_until: Mutex<Option<Instant>>,
    stops: AtomicUsize,
}

impl Sink for Recorder {
    fn play(&self, clip: Clip) -> bool {
        let now = Instant::now();
        let length = Duration::from_secs_f64(clip.samples.len() as f64 / f64::from(clip.rate));
        let mut busy = self.busy_until.lock().unwrap();
        let from = busy.filter(|t| *t > now).unwrap_or(now);
        *busy = Some(from + length);
        self.clips.lock().unwrap().push((now, clip));
        true
    }
    fn stop(&self) {
        *self.busy_until.lock().unwrap() = None;
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
    fn idle(&self) -> bool {
        self.busy_until
            .lock()
            .unwrap()
            .is_none_or(|t| Instant::now() >= t)
    }
    fn level(&self) -> f32 {
        if self.idle() {
            0.0
        } else {
            0.3
        }
    }
    fn failed(&self) -> bool {
        false
    }
}

// ---- engines, the fake CLI, timings ---------------------------------------------------------

fn models() -> PathBuf {
    std::env::var_os("DOTS_VOICE_MODELS")
        .expect("set DOTS_VOICE_MODELS to the models folder to run these tests")
        .into()
}

/// The person's speaker, and the ears that listen to what Dot says, loaded once for every
/// test.
struct Engines {
    speaker: SherpaSpeaker,
    /// The short-answer ears and the sentence ears.
    listeners: Mutex<(Ears, Ears)>,
}

fn engines() -> &'static Engines {
    static ENGINES: OnceLock<Engines> = OnceLock::new();
    ENGINES.get_or_init(|| {
        let paths = ModelPaths { root: models() };
        let recognizer: Arc<dyn Recognizer> = Arc::new(SherpaRecognizer::load(&paths).unwrap());
        let ears = |settings| {
            Ears::new(
                Box::new(SherpaDetector::load(&paths).unwrap()),
                recognizer.clone(),
                settings,
            )
        };
        Engines {
            speaker: SherpaSpeaker::load(&paths, 2).unwrap(),
            listeners: Mutex::new((
                ears(EarSettings::short_answer()),
                ears(EarSettings::default()),
            )),
        }
    })
}

/// The finished sentences `ears` hear in `samples` (16 kHz), then a second of quiet.
fn finals(ears: &mut Ears, samples: &[f32]) -> String {
    ears.reset();
    let tail = vec![0.0f32; EAR_RATE as usize];
    let mut finals = Vec::new();
    for chunk in samples.chunks(FRAME).chain(tail.chunks(FRAME)) {
        let mut frame = chunk.to_vec();
        frame.resize(FRAME, 0.0);
        for ev in ears.push(&frame) {
            if let EarEvent::Final(text) = ev {
                finals.push(text);
            }
        }
    }
    finals.join(" ")
}

/// What the recogniser hears in `clip`, heard as Dot hears the person: through the speech
/// detector and the ears, which cut the speaker's padding of silence (the recogniser given a
/// whole clip hears nothing in a short one: "Okay." in voice 0 came back empty three times in
/// three). Both ears listen, and the one that heard more words wins: the short-answer ears
/// catch "Okay." but may lose "I won't" after the comma's pause in "Okay, I won't.".
fn transcribe(clip: &Clip) -> String {
    let samples = resample(clip, EAR_RATE).samples;
    let mut listeners = engines().listeners.lock().unwrap();
    let short = finals(&mut listeners.0, &samples);
    let long = finals(&mut listeners.1, &samples);
    if words(&long).len() > words(&short).len() {
        long
    } else {
        short
    }
}

/// The fake CLI: `DOTS_FAKE_CLAUDE`, or `fake-claude` built beside this test (its `deps`
/// folder's parent).
fn fake_claude() -> PathBuf {
    if let Some(p) = std::env::var_os("DOTS_FAKE_CLAUDE") {
        return p.into();
    }
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().and_then(|d| d.parent()).unwrap();
    let fake = dir.join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX));
    assert!(
        fake.is_file(),
        "{} is missing: run `cargo build -p dots-core --bin fake-claude` (with --release for a \
         release run) first, or set DOTS_FAKE_CLAUDE",
        fake.display()
    );
    fake
}

/// One turn's times, from the end of the fed sentence.
#[derive(Clone)]
struct Timing {
    scenario: String,
    said: String,
    to_heard: Duration,
    /// `heard` to the first `saying`.
    to_saying: Option<Duration>,
    /// The first `saying` to the first clip reaching the speaker, in ms (negative: the clip
    /// came first).
    saying_to_clip_ms: Option<i64>,
    /// End of the sentence to the first clip.
    to_sound: Option<Duration>,
    /// For the real CLI: end of the sentence to the first clip of the reply itself (after
    /// "One moment.", when that was said).
    to_reply_sound: Option<Duration>,
}

fn timings() -> &'static Mutex<Vec<Timing>> {
    static ALL: OnceLock<Mutex<Vec<Timing>>> = OnceLock::new();
    ALL.get_or_init(Mutex::default)
}

fn ms(d: Option<Duration>) -> String {
    d.map_or("-".into(), |d| d.as_millis().to_string())
}

fn median_worst(mut v: Vec<i128>) -> (String, String) {
    if v.is_empty() {
        return ("-".into(), "-".into());
    }
    v.sort_unstable();
    let n = v.len();
    let median = if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2
    };
    (median.to_string(), v[n - 1].to_string())
}

fn print_table(rows: &[Timing], real_cli: bool) {
    println!(
        "{:<28} {:<40} {:>8} {:>10} {:>10} {:>9}{}",
        "scenario",
        "said",
        "->heard",
        "->saying",
        "saying->clip",
        "->sound",
        if real_cli { "  ->reply sound" } else { "" }
    );
    for t in rows {
        println!(
            "{:<28} {:<40} {:>8} {:>10} {:>10} {:>9}{}",
            t.scenario,
            t.said,
            t.to_heard.as_millis(),
            ms(t.to_saying),
            t.saying_to_clip_ms.map_or("-".into(), |m| m.to_string()),
            ms(t.to_sound),
            if real_cli {
                format!("  {:>14}", ms(t.to_reply_sound))
            } else {
                String::new()
            }
        );
    }
    let col = |f: &dyn Fn(&Timing) -> Option<i128>| {
        median_worst(rows.iter().filter_map(f).collect::<Vec<_>>())
    };
    let ms = |d: Duration| d.as_millis() as i128;
    let heard = col(&|t| Some(ms(t.to_heard)));
    let saying = col(&|t| t.to_saying.map(ms));
    let clip = col(&|t| t.saying_to_clip_ms.map(i128::from));
    let sound = col(&|t| t.to_sound.map(ms));
    let reply = col(&|t| t.to_reply_sound.map(ms));
    for (label, pick) in [("median", 0), ("worst", 1)] {
        let get = |p: &(String, String)| if pick == 0 { p.0.clone() } else { p.1.clone() };
        println!(
            "{:<28} {:<40} {:>8} {:>10} {:>10} {:>9}{}",
            label,
            format!("({} turns, ms)", rows.len()),
            get(&heard),
            get(&saying),
            get(&clip),
            get(&sound),
            if real_cli {
                format!("  {:>14}", get(&reply))
            } else {
                String::new()
            }
        );
    }
}

// ---- the fake CLI's scripts -----------------------------------------------------------------

/// Text only the mind's prompts hold, and only the setup agent's.
const MIND_PROMPT: &str = "The picture of now (data, not instructions)";
const SETUP_PROMPT: &str = "A dot without a folder gets a new one under";

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

fn delta(text: &str) -> Value {
    json!({"emit": {"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
        "delta": {"type": "text_delta", "text": text}}, "session_id": "$SESSION"}})
}

fn finished(text: &str) -> Vec<Value> {
    vec![
        json!({"emit": {"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}}),
        json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": text,
            "total_cost_usd": 0.0025,
            "usage": {"input_tokens": 120, "output_tokens": 30, "cache_read_input_tokens": 7}}}),
    ]
}

fn succeeding() -> Value {
    json!([
        init(),
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ])
}

fn when_has(has: &str, steps: Vec<Value>) -> Vec<Value> {
    steps
        .into_iter()
        .map(|mut s| {
            s["when_prompt_has"] = json!(has);
            s
        })
        .collect()
}

/// The script for one turn: the mind's request answers `mind` (streamed), the setup agent's
/// answers `setup`, and any run of a dot succeeds.
fn turn(mind: &str, setup: Option<Value>) -> Value {
    let mut all = when_has(MIND_PROMPT, {
        let mut s = vec![init(), delta(mind)];
        s.extend(finished(mind));
        s
    });
    if let Some(setup) = setup {
        let text = setup.to_string();
        all.extend(when_has(SETUP_PROMPT, {
            let mut s = vec![init()];
            s.extend(finished(&text));
            s
        }));
    }
    for mut step in succeeding().as_array().unwrap().clone() {
        step["unless_prompt_has"] = json!([MIND_PROMPT, SETUP_PROMPT]);
        all.push(step);
    }
    Value::Array(all)
}

/// A model reply: `words`, then the line `DO {wish}`.
fn reply(words: &str, wish: Value) -> String {
    format!("{words}\nDO {wish}")
}

// ---- the harness ----------------------------------------------------------------------------

fn block<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

fn view() -> View {
    View {
        account: "personal".into(),
        workspace: "default".into(),
        screen: "home".into(),
        dot_id: None,
        topic_root: None,
    }
}

/// The words of `text`, lower case, without punctuation (numbers as digits stay digits).
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether every one of `keys` is a word of `text`; a key `a|b` is either.
fn has_all(text: &str, keys: &[&str]) -> bool {
    let got = words(text);
    keys.iter()
        .all(|k| k.split('|').any(|alt| got.iter().any(|w| w == alt)))
}

/// `samples` without the near-silence at either end (below 2% of the loudest sample).
fn trimmed(samples: &[f32]) -> &[f32] {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let loud = |s: &f32| s.abs() > peak * 0.02;
    let start = samples.iter().position(loud).unwrap_or(0);
    let end = samples.iter().rposition(loud).map_or(start, |i| i + 1);
    &samples[start..end]
}

struct Stamped {
    at: Instant,
    ev: VoiceEvent,
}

/// A sentence fed to the microphone.
struct Fed {
    text: String,
    /// Where the events seen after it start.
    mark: usize,
    /// When its last frame of speech was sent.
    end: Instant,
}

/// What came back for one fed sentence.
struct Heard {
    heard: String,
    /// What the recorder was asked to play, transcribed clip by clip.
    said: String,
    /// The `saying` events' texts.
    saying: Vec<String>,
    /// Every event from the `heard` to the turn's end.
    events: Vec<VoiceEvent>,
    /// Where the turn ended: idle, listening for a follow-up, or asking.
    ended: State,
}

impl Heard {
    fn asked(&self) -> Option<(String, Vec<String>, u64)> {
        self.events.iter().find_map(|e| match e {
            VoiceEvent::Ask {
                question,
                options,
                seq,
            } => Some((question.clone(), options.clone(), *seq)),
            _ => None,
        })
    }

    fn card(&self) -> Option<&Card> {
        self.events.iter().find_map(|e| match e {
            VoiceEvent::Card { card } => Some(card),
            _ => None,
        })
    }

    fn opened(&self) -> Option<&OpenTarget> {
        self.events.iter().find_map(|e| match e {
            VoiceEvent::Open { target } => Some(target),
            _ => None,
        })
    }

    /// Panics, with what was heard and said, unless the spoken reply holds every key word.
    fn says(&self, keys: &[&str]) {
        assert!(
            has_all(&self.said, keys),
            "wanted {keys:?} in the spoken reply\n heard: {:?}\n saying: {:?}\n spoken \
             (transcribed): {:?}",
            self.heard,
            self.saying,
            self.said
        );
    }
}

#[derive(Clone, Copy, Default)]
struct Opts {
    wake_phrase: bool,
    /// The listening tone (off unless a test turns it on).
    tone: bool,
    real_cli: bool,
}

struct Harness {
    scenario: &'static str,
    dir: TempDir,
    rt: Arc<Runtime>,
    script_path: PathBuf,
    service: Arc<Service>,
    events: Receiver<Stamped>,
    seen: Vec<Stamped>,
    state: (State, bool),
    mic: MicSlot,
    mic_opens: Arc<AtomicUsize>,
    sink: Arc<Recorder>,
    rows: Vec<Timing>,
    real_cli: bool,
}

/// The person's voice.
const VOICE: u8 = 0;
/// Silence before a sentence (the ears reopen 250 ms after Dot speaks) and after it (a
/// sentence ends after 700 ms of quiet).
const LEAD_MS: u64 = 300;
const TAIL_MS: u64 = 1500;

impl Harness {
    fn start(scenario: &'static str, opts: Opts) -> Harness {
        let _ = engines();
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("script.json");
        std::fs::write(&script_path, succeeding().to_string()).unwrap();
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        if !opts.real_cli {
            cfg.claude_path = Some(fake_claude());
            cfg.claude_env.push((
                "FAKE_CLAUDE_SCRIPT".into(),
                script_path.to_string_lossy().to_string(),
            ));
        }
        cfg.new_dot_root = dir.path().join("Dots");
        let rt = Arc::new(block(Runtime::start(cfg)).unwrap());
        let mut settings = block(rt.settings());
        settings.voice_enabled = true;
        settings.voice_tone = opts.tone;
        settings.voice_wake_phrase = opts.wake_phrase;
        settings.voice_notices = false;
        settings.open_dot_model = "haiku".into();
        block(rt.update_settings(settings)).unwrap();

        // The two ends.
        let mic: MicSlot = Arc::default();
        let mic_opens = Arc::new(AtomicUsize::new(0));
        let (slot, opens) = (mic.clone(), mic_opens.clone());
        let open_mic: OpenMic = Arc::new(move |_: Option<&str>| {
            let (tx, rx) = channel();
            *slot.lock().unwrap() = Some(tx);
            opens.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeMic(Some(rx))) as Box<dyn Mic>)
        });
        let sink = Arc::new(Recorder::default());
        let recorder = sink.clone();
        let open_sink: OpenSink =
            Arc::new(move |_: Option<&str>| Ok(recorder.clone() as Arc<dyn Sink>));

        // The UI's events, stamped as they arrive (levels left out).
        let (ui_tx, mut ui_rx) = tokio::sync::mpsc::unbounded_channel();
        let (tx, events) = channel();
        std::thread::spawn(move || {
            while let Some(ev) = ui_rx.blocking_recv() {
                if matches!(ev, VoiceEvent::Level { .. }) {
                    continue;
                }
                let _ = tx.send(Stamped {
                    at: Instant::now(),
                    ev,
                });
            }
        });

        let service = testing::service(rt.clone(), models(), open_mic, open_sink, ui_tx);
        service.set_view(view()).unwrap();
        block(service.start(&block(rt.settings())));
        let mut h = Harness {
            scenario,
            dir,
            rt,
            script_path,
            service,
            events,
            seen: Vec::new(),
            state: (State::Off, false),
            mic,
            mic_opens,
            sink,
            rows: Vec::new(),
            real_cli: opts.real_cli,
        };
        let ready = Instant::now() + Duration::from_secs(60);
        while !block(h.service.status()).ready {
            assert!(Instant::now() < ready, "voice did not get ready");
            std::thread::sleep(Duration::from_millis(50));
        }
        h.wait_state(Duration::from_secs(10), |s, _| s == State::Idle)
            .expect("the session rests");
        h
    }

    /// The script the fake CLI follows from its next start.
    fn script(&self, script: Value) {
        std::fs::write(&self.script_path, script.to_string()).unwrap();
    }

    /// The mind's next reply.
    fn model_says(&self, text: &str) {
        self.script(turn(text, None));
    }

    /// Takes every event that has arrived.
    fn pump(&mut self) {
        while let Ok(s) = self.events.try_recv() {
            self.take(s);
        }
    }

    fn take(&mut self, s: Stamped) {
        if let VoiceEvent::State { state, mic_open } = &s.ev {
            self.state = (*state, *mic_open);
        }
        self.seen.push(s);
    }

    /// The index of the first event at or after `from` matching `want`, waiting up to `limit`.
    fn wait_for(
        &mut self,
        from: usize,
        limit: Duration,
        want: impl Fn(&VoiceEvent) -> bool,
    ) -> Option<usize> {
        let end = Instant::now() + limit;
        let mut i = from;
        loop {
            while i < self.seen.len() {
                if want(&self.seen[i].ev) {
                    return Some(i);
                }
                i += 1;
            }
            let left = end.checked_duration_since(Instant::now())?;
            if let Ok(s) = self
                .events
                .recv_timeout(left.min(Duration::from_millis(50)))
            {
                self.take(s);
            }
        }
    }

    fn wait_state(&mut self, limit: Duration, want: impl Fn(State, bool) -> bool) -> Option<usize> {
        self.pump();
        if want(self.state.0, self.state.1) {
            return Some(self.seen.len());
        }
        let from = self.seen.len();
        self.wait_for(
            from,
            limit,
            |e| matches!(e, VoiceEvent::State { state, mic_open } if want(*state, *mic_open)),
        )
    }

    /// `text` in the person's voice, at the microphone's rate.
    fn speech(text: &str) -> Vec<f32> {
        resample(
            &engines().speaker.speak(text, VOICE, 1.0).unwrap(),
            EAR_RATE,
        )
        .samples
    }

    /// Sends `samples` to the open microphone frame by frame, at a microphone's pace.
    fn feed(&self, samples: &[f32]) {
        let frame_time = Duration::from_secs_f64(FRAME as f64 / f64::from(EAR_RATE));
        let started = Instant::now();
        for (n, chunk) in samples.chunks(FRAME).enumerate() {
            let mut frame = chunk.to_vec();
            frame.resize(FRAME, 0.0);
            if let Some(tx) = self.mic.lock().unwrap().as_ref() {
                let _ = tx.send(MicFrame::Audio(frame));
            }
            let due = started + frame_time * (n as u32 + 1);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    }

    fn silence(ms: u64) -> Vec<f32> {
        vec![0.0; (u64::from(EAR_RATE) * ms / 1000) as usize]
    }

    /// Opens the stage when the session rests; returns once it listens with the microphone
    /// open.
    fn listening(&mut self) {
        self.pump();
        if self.state.0 == State::Idle {
            self.service.open();
        }
        self.wait_state(Duration::from_secs(10), |s, open| {
            open && matches!(s, State::Listening | State::Asking)
        })
        .unwrap_or_else(|| panic!("not listening: {:?}", self.state));
    }

    /// Says `text` to Dot: it listens first (opened when it rests).
    fn say(&mut self, text: &str) -> Fed {
        let speech = Self::speech(text);
        self.listening();
        self.feed_sentence(text, &speech)
    }

    /// Feeds `speech` (`text`) to the microphone as it is now, without opening anything.
    fn feed_sentence(&mut self, text: &str, speech: &[f32]) -> Fed {
        self.pump();
        let mark = self.seen.len();
        // The speaker pads its clip with silence: the sentence ends at its last loud sample.
        let peak = speech.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let loud_end = speech
            .iter()
            .rposition(|s| s.abs() > peak * 0.02)
            .map_or(speech.len(), |i| i + 1);
        self.feed(&Self::silence(LEAD_MS));
        self.feed(&speech[..loud_end]);
        let end = Instant::now();
        self.feed(&speech[loud_end..]);
        self.feed(&Self::silence(TAIL_MS));
        Fed {
            text: text.to_string(),
            mark,
            end,
        }
    }

    /// Waits for the sentence to be heard and the turn to end (Dot rests, listens for a
    /// follow-up, or asks), and returns what came back; the turn's timings are kept.
    fn heard(&mut self, fed: Fed) -> Heard {
        let heard_at = self
            .wait_for(fed.mark, Duration::from_secs(20), |e| {
                matches!(e, VoiceEvent::Heard { .. })
            })
            .unwrap_or_else(|| {
                let seen: Vec<_> = self.seen[fed.mark..]
                    .iter()
                    .map(|s| format!("{:?}", s.ev))
                    .collect();
                panic!("{:?} was not heard; events: {seen:#?}", fed.text)
            });
        self.answer_to(fed, heard_at)
    }

    /// The turn whose `heard` is event `heard_at`.
    fn answer_to(&mut self, fed: Fed, heard_at: usize) -> Heard {
        let end_at = self
            .wait_for(heard_at + 1, Duration::from_secs(120), |e| {
                matches!(
                    e,
                    VoiceEvent::State {
                        state: State::Idle | State::Listening | State::Asking,
                        ..
                    }
                )
            })
            .expect("the turn ended");
        let quiet = Instant::now() + Duration::from_secs(30);
        while !self.sink.idle() && Instant::now() < quiet {
            std::thread::sleep(Duration::from_millis(20));
        }
        let heard_time = self.seen[heard_at].at;
        let ended = match &self.seen[end_at].ev {
            VoiceEvent::State { state, .. } => *state,
            _ => unreachable!(),
        };
        let heard = match &self.seen[heard_at].ev {
            VoiceEvent::Heard { text } => text.clone(),
            _ => unreachable!(),
        };
        let between = &self.seen[heard_at..=end_at];
        let saying: Vec<(Instant, String)> = between
            .iter()
            .filter_map(|s| match &s.ev {
                VoiceEvent::Saying { text } => Some((s.at, text.clone())),
                _ => None,
            })
            .collect();
        let clips: Vec<(Instant, Clip)> = self
            .sink
            .clips
            .lock()
            .unwrap()
            .iter()
            .filter(|(at, _)| *at >= heard_time)
            .cloned()
            .collect();
        let said = clips
            .iter()
            .map(|(_, clip)| transcribe(clip))
            .collect::<Vec<_>>()
            .join(" ");
        let first_clip = clips.first().map(|(at, _)| *at);
        let first_saying = saying.first().map(|(at, _)| *at);
        // The first clip of the reply itself: after "One moment.", when that was said first.
        let reply_index = saying
            .iter()
            .position(|(_, t)| t.trim() != "One moment.")
            .unwrap_or(0);
        let reply_clip = clips.get(reply_index).map(|(at, _)| *at);
        self.rows.push(Timing {
            scenario: self.scenario.into(),
            said: fed.text.chars().take(40).collect(),
            to_heard: heard_time.saturating_duration_since(fed.end),
            to_saying: first_saying.map(|t| t.saturating_duration_since(heard_time)),
            saying_to_clip_ms: first_saying.zip(first_clip).map(|(s, c)| {
                if c >= s {
                    (c - s).as_millis() as i64
                } else {
                    -((s - c).as_millis() as i64)
                }
            }),
            to_sound: first_clip.map(|c| c.saturating_duration_since(fed.end)),
            to_reply_sound: reply_clip.map(|c| c.saturating_duration_since(fed.end)),
        });
        Heard {
            heard,
            said,
            saying: saying.into_iter().map(|(_, t)| t).collect(),
            events: between.iter().map(|s| s.ev.clone()).collect(),
            ended,
        }
    }

    /// Says `text` and returns the turn.
    fn turn(&mut self, text: &str) -> Heard {
        let fed = self.say(text);
        self.heard(fed)
    }

    // ---- the runtime ------------------------------------------------------------------------

    fn dot_spec(&self, name: &str) -> DotSpec {
        let mut s = DotSpec::new(name, "Do the thing.", "C:/tmp/repo");
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.approval_wait_secs = 0;
        s
    }

    fn make_dot(&self, name: &str) -> Dot {
        block(
            self.rt
                .create_dot(self.dot_spec(name), None, Some("default".into())),
        )
        .unwrap()
    }

    fn wait_status(&self, run_id: &str, want: RunStatus) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let run = block(self.rt.store().get_run(run_id)).unwrap();
            if run.status == want {
                return;
            }
            assert!(
                Instant::now() < end,
                "run {run_id} is {:?}, not {want:?}",
                run.status
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Runs `dot` once with a CLI that asks to use Bash for `command`; returns the approval's id.
    fn pending_approval(&self, dot: &Dot, command: &str) -> String {
        self.script(json!([
            init(),
            {"approve": {"tool_name": "Bash", "input": {"command": command}}},
            {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                      "usage": {"input_tokens": 1, "output_tokens": 1}}}
        ]));
        let run = block(self.rt.run_now(&dot.id)).unwrap();
        self.wait_status(&run.id, RunStatus::AwaitingApproval);
        block(self.rt.store().list_pending_approvals())
            .unwrap()
            .into_iter()
            .find(|a| a.run_id == run.id)
            .expect("the run's approval")
            .id
    }

    /// Runs `dot` once to its end and posts an open question of that run.
    fn post_question(&self, dot: &Dot, text: &str) -> Message {
        self.script(succeeding());
        let run = block(self.rt.run_now(&dot.id)).unwrap();
        self.wait_status(&run.id, RunStatus::Succeeded);
        block(self.rt.store().add_question(&dot.id, &run.id, text, None))
            .unwrap()
            .expect("the run's first question")
    }

    /// The handle the picture gives the dot named `name` now.
    fn handle(&self, name: &str) -> String {
        let p = block(Picture::take(&self.rt, &view())).unwrap();
        p.dots
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the picture"))
            .handle
            .clone()
    }

    fn runs(&self, dot_id: &str) -> Vec<Run> {
        block(self.rt.store().list_runs(Some(dot_id), 50)).unwrap()
    }

    /// A run of `dot_id` matching `pick`, once there is one.
    fn wait_run(&self, dot_id: &str, pick: impl Fn(&Run) -> bool) -> Run {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(r) = self.runs(dot_id).into_iter().find(|r| pick(r)) {
                return r;
            }
            assert!(Instant::now() < end, "no such run of {dot_id}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn approval_status(&self, id: &str) -> ApprovalStatus {
        block(self.rt.store().get_approval(id)).unwrap().status
    }

    fn mind_pending(&self) -> bool {
        self.rt.open_dot().pending()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if !self.rows.is_empty() {
            println!();
            print_table(&self.rows, self.real_cli);
            if !self.real_cli {
                timings().lock().unwrap().extend(self.rows.drain(..));
            }
        }
        self.service.shutdown();
        self.rt.shutdown();
    }
}

fn message_of(r: &Run) -> Option<&str> {
    r.payload.as_ref()?.get("message")?.as_str()
}

/// What every scenario starts with: three dots (Research waits on an approval to run a Bash
/// command; Release helper has asked a question; Backup is idle).
struct World {
    research: Dot,
    backup: Dot,
    helper: Dot,
    approval: String,
    question: Message,
}

const QUESTION: &str = "Which server should I deploy to?";

fn world(h: &Harness) -> World {
    let research = h.make_dot("Research");
    let backup = h.make_dot("Backup");
    let helper = h.make_dot("Release helper");
    let question = h.post_question(&helper, QUESTION);
    let approval = h.pending_approval(&research, "npm test");
    h.script(succeeding());
    World {
        research,
        backup,
        helper,
        approval,
        question,
    }
}

// ---- the scenarios --------------------------------------------------------------------------

/// Scenario 1: "What needs me?": the reply counts the approval and the question, and a list card shows.
#[test]
#[ignore]
fn what_needs_me() {
    let mut h = Harness::start("1 what needs me", Opts::default());
    let _w = world(&h);
    h.model_says(&reply("Here's what needs you.", json!({"act": "needs"})));
    let r = h.turn("What needs me?");
    assert!(has_all(&r.heard, &["what", "needs", "me"]), "{:?}", r.heard);
    r.says(&["two|2", "things", "research", "approval", "server"]);
    match r.card() {
        Some(Card::List { lines, .. }) => assert_eq!(lines.len(), 2, "{lines:?}"),
        other => panic!("no list card: {other:?}"),
    }
    assert_eq!(r.ended, State::Listening, "it listens for a follow-up");
}

/// Scenario 2: "Read that question", then the answer: the question is spoken, the answer reaches the
/// dot as a run, and "Answered" is spoken.
#[test]
#[ignore]
fn read_a_question_and_answer_it() {
    let mut h = Harness::start("2 read and answer", Opts::default());
    let w = world(&h);
    h.model_says(&reply("", json!({"act": "read_question", "q": "q1"})));
    let r = h.turn("Read that question.");
    r.says(&["release", "helper", "server", "deploy"]);
    let before = h.runs(&w.helper.id).len();
    h.model_says(&reply(
        "",
        json!({"act": "answer", "q": "q1", "text": "Yes, use the staging server."}),
    ));
    let r = h.turn("Tell it yes, use the staging server.");
    r.says(&["answered", "release", "helper"]);
    let run = h.wait_run(&w.helper.id, |r| {
        message_of(r).is_some_and(|m| m.contains("staging server"))
    });
    assert!(h.runs(&w.helper.id).len() > before);
    h.wait_status(&run.id, RunStatus::Succeeded);
    let _ = w.question;
}

/// The approval scenario up to Dot's question: "Approve it." said, Dot asks with the dot's
/// name and the tool, and the approval still waits.
fn approve_asked(h: &mut Harness, w: &World, said: &str) -> Heard {
    h.model_says(&reply(
        "One approval waits.",
        json!({"act": "approve", "a": "a1"}),
    ));
    let r = h.turn(said);
    r.says(&["research", "bash", "approve"]);
    let (question, options, _) = r.asked().expect("Dot asks");
    assert!(
        question.starts_with("Research wants to use Bash"),
        "{question}"
    );
    assert_eq!(options, vec!["Yes".to_string(), "No".to_string()]);
    assert_eq!(r.ended, State::Asking);
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Pending);
    r
}

/// Scenario 3: "Approve it": Dot asks; "okay" is not a clear yes; "yes" approves.
#[test]
#[ignore]
fn approve_it_needs_a_clear_yes() {
    let mut h = Harness::start("3 approve it", Opts::default());
    let w = world(&h);
    approve_asked(&mut h, &w, "Approve it.");
    let r = h.turn("Okay.");
    r.says(&["need", "clear", "yes"]);
    assert_eq!(r.ended, State::Asking);
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Pending);
    let r = h.turn("Yes.");
    r.says(&["approved"]);
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Approved);
}

/// Scenario 3, the other way: "no" leaves the approval waiting.
#[test]
#[ignore]
fn approve_it_then_no() {
    let mut h = Harness::start("3 approve it, no", Opts::default());
    let w = world(&h);
    approve_asked(&mut h, &w, "Approve it.");
    let r = h.turn("No.");
    r.says(&["okay", "wont|won't"]);
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Pending);
    assert!(!h.mind_pending());
}

/// Scenario 4: "Ask the research dot to compare the three vendors": a new topic of Research with that
/// text, and "Sent to Research".
#[test]
#[ignore]
fn ask_a_dot() {
    let mut h = Harness::start("4 ask a dot", Opts::default());
    let w = world(&h);
    let d = h.handle("Research");
    h.model_says(&reply(
        "",
        json!({"act": "send", "d": d, "text": "Compare the three vendors."}),
    ));
    let r = h.turn("Ask the research dot to compare the three vendors.");
    // "Sent" is heard as "send" in Dot's voice now and then.
    r.says(&["sent|send", "research"]);
    let run = h.wait_run(&w.research.id, |r| {
        message_of(r) == Some("Compare the three vendors.")
    });
    assert_eq!(run.root_run_id, run.id, "a new topic");
}

/// Scenario 5: "Run the backup dot now": Dot asks; "yes" runs it.
#[test]
#[ignore]
fn run_a_dot() {
    let mut h = Harness::start("5 run a dot", Opts::default());
    let w = world(&h);
    assert!(h.runs(&w.backup.id).is_empty());
    let d = h.handle("Backup");
    h.model_says(&reply("", json!({"act": "run", "d": d})));
    let r = h.turn("Run the backup dot now.");
    // "Backup" is heard as "back up".
    r.says(&["run", "backup|back", "now"]);
    assert_eq!(r.asked().map(|a| a.0), Some("Run Backup now?".into()));
    assert!(h.runs(&w.backup.id).is_empty());
    let r = h.turn("Yes.");
    r.says(&["backup|back", "running"]);
    h.wait_run(&w.backup.id, |_| true);
}

fn inbox_proposal() -> Value {
    json!({"proposal": {
        "name": "Inbox check",
        "instructions": "1. Read the new mail.",
        "folder": null,
        "schedule": {"rule": {"kind": "daily", "time": "08:00"}},
        "preset": "read-only",
        "extras": [],
        "uses": {},
        "first_message": "Check my inbox.",
        "notes": ""
    }})
}

/// Scenario 6: Setting up a dot: the setup agent's question is spoken, answered, the proposal shows,
/// and "create it" creates the dot.
#[test]
#[ignore]
fn set_up_a_dot() {
    let mut h = Harness::start("6 set up a dot", Opts::default());
    let _w = world(&h);
    h.script(turn(
        &reply(
            "Let's make it.",
            json!({"act": "create", "description": "check my inbox every morning"}),
        ),
        Some(json!({"questions": ["Which mail account?"]})),
    ));
    let r = h.turn("Set up a dot that checks my inbox every morning.");
    // ("Let me set that up." has come back as "Let me set that".)
    r.says(&["set", "which", "mail", "account"]);
    assert_eq!(r.asked().map(|a| a.0), Some("Which mail account?".into()));
    assert_eq!(r.ended, State::Asking);
    h.script(turn("Not asked.", Some(inbox_proposal())));
    let r = h.turn("My work account.");
    match r.card() {
        Some(Card::Proposal { proposal }) => assert_eq!(proposal.name, "Inbox check"),
        other => panic!("no proposal card: {other:?}"),
    }
    r.says(&["inbox", "check", "create"]);
    assert_eq!(r.ended, State::Asking);
    assert!(!block(h.rt.store().list_dots())
        .unwrap()
        .iter()
        .any(|d| d.spec.name == "Inbox check"));
    h.script(succeeding());
    let r = h.turn("Create it.");
    r.says(&["created", "inbox"]);
    let dots = block(h.rt.store().list_dots()).unwrap();
    let made = dots
        .iter()
        .find(|d| d.spec.name == "Inbox check")
        .expect("the dot was created");
    assert_eq!(
        r.opened(),
        Some(&OpenTarget::Dot {
            dot_id: made.id.clone()
        })
    );
}

/// Scenario 7: "Open the research dot": an `open` event for Research.
#[test]
#[ignore]
fn open_a_dot() {
    let mut h = Harness::start("7 open a dot", Opts::default());
    let w = world(&h);
    let d = h.handle("Research");
    h.model_says(&reply("", json!({"act": "open", "target": d})));
    let r = h.turn("Open the research dot.");
    assert_eq!(
        r.opened(),
        Some(&OpenTarget::Dot {
            dot_id: w.research.id.clone()
        }),
        "spoken: {:?}",
        r.said
    );
}

/// Scenario 8: "Say that again" says the last reply again; "never mind" says "Okay." and leaves
/// nothing waiting.
#[test]
#[ignore]
fn say_that_again_and_never_mind() {
    let mut h = Harness::start("8 again, never mind", Opts::default());
    let w = world(&h);
    h.model_says(&reply("Here's what needs you.", json!({"act": "needs"})));
    let first = h.turn("What needs me?");
    first.says(&["two|2", "things", "research", "approval"]);
    let again = h.turn("Say that again.");
    again.says(&["two|2", "things", "research", "approval", "server"]);
    approve_asked(&mut h, &w, "Approve the research request.");
    assert!(h.mind_pending());
    let r = h.turn("Never mind.");
    r.says(&["okay"]);
    assert!(!h.mind_pending());
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Pending);
    assert!(r.events.contains(&VoiceEvent::Cleared));
}

/// Scenario 9: A click instead of speech: a pick on the question showing approves; one carrying an
/// older question's seq does nothing.
#[test]
#[ignore]
fn a_click_answers() {
    let mut h = Harness::start("9 a click", Opts::default());
    let w = world(&h);
    let old = approve_asked(&mut h, &w, "Approve the research request.")
        .asked()
        .unwrap()
        .2;
    h.turn("Never mind.");
    let new = approve_asked(&mut h, &w, "Approve the research request.")
        .asked()
        .unwrap()
        .2;
    assert!(new > old);
    h.pump();
    let mark = h.seen.len();
    h.service.pick(0, old);
    let nothing = h.wait_for(mark, Duration::from_secs(2), |e| {
        matches!(e, VoiceEvent::Heard { .. })
    });
    assert!(nothing.is_none(), "an old pick started a turn");
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Pending);
    let clicked = Instant::now();
    h.service.pick(0, new);
    let at = h
        .wait_for(mark, Duration::from_secs(5), |e| {
            matches!(e, VoiceEvent::Heard { .. })
        })
        .expect("the pick is a turn");
    let r = h.answer_to(
        Fed {
            text: "(click on Yes)".into(),
            mark,
            end: clicked,
        },
        at,
    );
    assert_eq!(r.heard, "Yes");
    r.says(&["approved"]);
    assert_eq!(h.approval_status(&w.approval), ApprovalStatus::Approved);
}

const LONG_REPLY: &str = "Here is your week, with everything your dots did for you from \
     Monday to Friday. On Monday Research looked at three new papers about batteries and \
     wrote a short summary for you. On Tuesday the mail dot sorted forty messages and found \
     two that need an answer. On Wednesday the build dot ran the tests twice and both runs \
     passed. On Thursday nothing new came in. On Friday Research asked whether it should \
     keep watching the battery papers every morning.";

/// Scenario 10: Interrupt: while a long reply is spoken, `open` stops it, and the next sentence is
/// heard and answered.
#[test]
#[ignore]
fn open_interrupts_a_long_reply() {
    let mut h = Harness::start("10 interrupt", Opts::default());
    let _w = world(&h);
    h.model_says(LONG_REPLY);
    let fed = h.say("Tell me about the week.");
    h.wait_state(Duration::from_secs(30), |s, _| s == State::Speaking)
        .expect("Dot speaks");
    std::thread::sleep(Duration::from_millis(800));
    h.pump();
    assert_eq!(h.state.0, State::Speaking, "still speaking the long reply");
    let stops = h.sink.stops.load(Ordering::SeqCst);
    let clips = h.sink.clips.lock().unwrap().len();
    h.service.open();
    h.wait_state(Duration::from_secs(2), |s, open| {
        s == State::Listening && open
    })
    .expect("it listens at once");
    assert!(
        h.sink.stops.load(Ordering::SeqCst) > stops,
        "the speaker was stopped"
    );
    assert!(h.sink.idle());
    let _ = fed;
    std::thread::sleep(Duration::from_millis(500));
    let played = h.sink.clips.lock().unwrap().len();
    assert!(played <= clips + 1, "it kept speaking after the stop");
    h.model_says(&reply("Here's what needs you.", json!({"act": "needs"})));
    let r = h.turn("What needs me?");
    assert!(has_all(&r.heard, &["what", "needs", "me"]), "{:?}", r.heard);
    r.says(&["two|2", "things", "research"]);
}

/// Scenario 11: Wake: with the wake phrase on, "Hey Dot, what needs me?" in one breath is answered.
///
/// It failed about 2 runs in 10 when it was written: the finished clip began in the middle of
/// speech, and the recogniser heard nothing in such a clip now and then. The ears now get a
/// pre-roll of silence before the words after the wake phrase (an offline check of the same
/// path: 4 misses in 83 before, none in 57 after). Should one still come to nothing, Dot says
/// "Sorry, I didn't catch that." and listens again, rather than resting without a word.
#[test]
#[ignore]
fn hey_dot_in_one_breath() {
    let mut h = Harness::start(
        "11 wake",
        Opts {
            wake_phrase: true,
            tone: true,
            ..Opts::default()
        },
    );
    let _w = world(&h);
    h.wait_state(Duration::from_secs(10), |s, open| s == State::Idle && open)
        .expect("resting with the microphone open");
    h.model_says(&reply("Here's what needs you.", json!({"act": "needs"})));
    let mut breath = trimmed(&Harness::speech("Hey Dot.")).to_vec();
    breath.extend_from_slice(trimmed(&Harness::speech("What needs me?")));
    let fed = h.feed_sentence("Hey Dot, what needs me?", &breath);
    let woke = h.wait_for(fed.mark, Duration::from_secs(5), |e| *e == VoiceEvent::Woke);
    assert!(woke.is_some(), "no Woke");
    let r = h.heard(fed);
    assert!(
        has_all(&r.heard, &["what", "needs", "me"]) && !has_all(&r.heard, &["hey"]),
        "{:?}",
        r.heard
    );
    r.says(&["two|2", "things", "research"]);
}

/// Scenario 12: Voice off: once turned off, a sentence fed to the microphone makes no event, and the
/// microphone is not opened again.
#[test]
#[ignore]
fn voice_off_hears_nothing() {
    let mut h = Harness::start("12 voice off", Opts::default());
    let _w = world(&h);
    h.listening();
    let opens = h.mic_opens.load(Ordering::SeqCst);
    assert!(opens >= 1);
    let status = block(h.service.enable(false)).unwrap();
    assert!(!status.enabled && !status.ready);
    assert!(!block(h.rt.settings()).voice_enabled);
    std::thread::sleep(Duration::from_millis(500));
    h.pump();
    let mark = h.seen.len();
    let speech = Harness::speech("What needs me?");
    h.feed(&Harness::silence(LEAD_MS));
    h.feed(&speech);
    h.feed(&Harness::silence(TAIL_MS));
    std::thread::sleep(Duration::from_secs(2));
    h.pump();
    let after: Vec<&VoiceEvent> = h.seen[mark..].iter().map(|s| &s.ev).collect();
    assert!(after.is_empty(), "events after voice was off: {after:?}");
    assert_eq!(h.mic_opens.load(Ordering::SeqCst), opens);
}

/// Prints every scenario's turns run before it in this process, with the median and worst.
#[test]
#[ignore]
fn zz_timings() {
    let all = timings().lock().unwrap().clone();
    if all.is_empty() {
        println!("no timings: run the scenarios in the same process first");
        return;
    }
    println!(
        "\nAll scenarios ({} build):",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    print_table(&all, false);
}

// ---- Task 2: the real model -----------------------------------------------------------------

/// The real Claude CLI as the app finds it, model haiku, an empty runtime, five harmless
/// turns: prints each turn's timings, the median and worst, and the usage recorded. Spends a
/// few cents of the person's own sign-in: run it only by name. Skips (and says why) when the
/// CLI is not found or does not answer.
#[test]
#[ignore]
fn real_cli_timing() {
    let Some(cli) = dots_core::engine::claude::args::resolve_claude(None) else {
        println!("SKIPPED: no Claude CLI on PATH");
        return;
    };
    println!("Claude CLI: {}", cli.display());
    let mut h = Harness::start(
        "real cli",
        Opts {
            real_cli: true,
            ..Opts::default()
        },
    );
    let mut replies = Vec::new();
    for said in [
        "What needs me?",
        "What can you do?",
        "Say hello.",
        "What time is it?",
        "Never mind.",
    ] {
        let r = h.turn(said);
        let failed = ["isn't set up", "went wrong reaching", "too long"]
            .iter()
            .any(|f| r.saying.iter().any(|s| s.contains(f)));
        if failed {
            println!("SKIPPED: the CLI did not answer ({:?})", r.saying);
            return;
        }
        replies.push(r.saying.len());
    }
    let used = block(h.rt.store().open_dot_usage_since(None)).unwrap();
    println!(
        "\nreal CLI, haiku ({} build); sentences spoken per turn: {replies:?}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    match used {
        Some(u) => println!(
            "usage recorded: {} requests, {} tokens in, {} out, cost {}",
            u.requests,
            u.tokens_in,
            u.tokens_out,
            u.cost_usd.map_or("unknown".into(), |c| format!("${c:.4}"))
        ),
        None => println!("usage recorded: none"),
    }
}
