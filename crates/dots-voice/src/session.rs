//! The listen, think, speak loop: one worker thread ties the microphone, the ears, the wake
//! phrase and the voice together, and tells the app what happens through [`SessionEvent`]s.
//!
//! Nothing here logs. Heard and spoken text leave only through the events; audio never
//! leaves memory.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::audio::{Mic, MicFrame};
use crate::ears::{level, EarEvent, Ears};
use crate::engine::{EngineError, WakeSpotter, WakeWord, FRAME};
use crate::tone::{listen_tone, TONE_RATE};
use crate::voice::{Sink, Voice, VoiceEvent};

/// The longest the worker waits for a command before it looks at the microphone, the voice
/// and the timers again. Commands themselves wake it at once.
const TICK: Duration = Duration::from_millis(20);
/// How long `open` (or "Hey Dot", or an interruption) listens with no speech before resting.
const LISTEN_AFTER_OPEN: Duration = Duration::from_secs(8);
/// How long a question waits for its answer.
const ASK_LIMIT: Duration = Duration::from_secs(20);
/// How long a sentence may wait for its reply before the session gives up on it.
const THINK_LIMIT: Duration = Duration::from_secs(120);
/// At most 30 `Level` events a second.
const LEVEL_GAP: Duration = Duration::from_micros(33_334);
/// How long `Speaking` may go without word from the voice before the session gives up on it.
const VOICE_LIMIT: Duration = Duration::from_secs(60);
/// Microphone frames handled in one pass, so commands and timers are seen between batches.
const FRAMES_PER_PASS: usize = 8;
/// How often a lost microphone is tried again while only the wake phrase wants it.
const MIC_RETRY: Duration = Duration::from_secs(5);
/// The longest the ears wait for the listening tone to finish sounding (it is 155 ms long);
/// a sink still not quiet by then is stopped.
const TONE_LIMIT: Duration = Duration::from_secs(2);
/// After "Hey Dot" the ears are open at once; the tone answers only a person who pauses this
/// long without starting to speak (waiting to be acknowledged).
const TONE_WAIT: Duration = Duration::from_millis(300);
/// Once the tone has stopped sounding, the ears wait this much longer before they hear: the
/// tone is short and soft, so its echo dies far sooner than speech's `reopen_after_ms`.
const TONE_TAIL: Duration = Duration::from_millis(60);
/// Idle frames kept for the ears (about a second): the spotter answers at most this late.
const RECENT_FRAMES: usize = 32;

/// When the speaker cannot tell how long its sound takes to leave the device, the ears wait
/// this much longer after it goes quiet, on top of `reopen_after_ms` (best effort: a speaker
/// with a longer delay than this can still be heard).
const UNKNOWN_DELAY: Duration = Duration::from_millis(350);
/// The longest output delay a speaker is believed: one that reports more (a clock jump, a
/// virtual device) would keep the ears shut until the voice is taken for stuck.
const MAX_DEVICE_DELAY: Duration = Duration::from_secs(2);
/// The most follow-up listens in a row with no hotkey, click or "Hey Dot" between: speech in
/// the room (a television, a call) cannot keep Dot taking turns without end.
const MAX_FOLLOW_UPS: u32 = 5;
/// A listen that got at least this many frames (about a second), every one of them exact
/// silence, ran on a microphone that gives nothing (blocked by the system, or muted).
const SILENT_FRAMES: usize = 32;

const NO_REPLY: &str = "No reply came back.";
const VOICE_STUCK: &str = "The voice stopped responding.";
const MIC_LOST: &str = "The microphone stopped working.";
const BROKE: &str = "Speech stopped working and was turned off.";
/// Said when a question's 20 seconds pass with no answer.
const NO_ANSWER: &str = "No answer, so I did nothing.";
/// Said when the microphone gave only silence for a whole listen.
const SILENT_MIC: &str = "I can't hear anything from the microphone. Check that Dots may use it.";
/// Said when listening was asked for and the microphone would not open (the details are shown).
const MIC_WONT_OPEN: &str =
    "I can't open the microphone. Check that one is connected and that Dots may use it.";
/// Said once when words were heard live but the finished sentence came out empty.
const DIDNT_CATCH: &str = "Sorry, I didn't catch that.";
/// Shown when a question was not said in full: it is not asked, and nothing waits on it.
const QUESTION_CUT: &str = "The question could not be said in full, so nothing was done.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Off,
    Idle,
    Listening,
    Thinking,
    Speaking,
    Asking,
}

impl State {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Idle,
            2 => Self::Listening,
            3 => Self::Thinking,
            4 => Self::Speaking,
            5 => Self::Asking,
            _ => Self::Off,
        }
    }

    /// The person is being listened to: frames go to the ears.
    fn hears(self) -> bool {
        matches!(self, Self::Listening | Self::Asking)
    }

    /// `Level` events are sent in this state.
    fn has_level(self) -> bool {
        matches!(self, Self::Listening | Self::Asking | Self::Speaking)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    /// Sent on every change of state or of `mic_open`, never twice the same in a row.
    /// `mic_open` is true exactly when a microphone stream exists.
    State {
        state: State,
        mic_open: bool,
    },
    /// At most 30 a second while Listening, Asking or Speaking, and one of zeros on leaving.
    Level {
        mic: f32,
        voice: f32,
    },
    /// Live words of the sentence the person is saying.
    Hearing {
        text: String,
    },
    /// A finished sentence from the person: the app sends it to the mind and tags the reply
    /// with `turn`.
    Heard {
        text: String,
        turn: u64,
    },
    /// A sentence handed to the speaker.
    Saying {
        text: String,
    },
    /// "Hey Dot" while idle.
    Woke,
    Trouble {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSettings {
    /// Listen for "Hey Dot" and "Dot, stop" whenever the session is not listening.
    pub wake_phrase: bool,
    /// The person talking over Dot stops it.
    pub interrupt_by_talking: bool,
    /// How long a follow-up waits for the person to speak.
    pub follow_up_secs: u32,
    /// How long after speech ends before the ears hear again, so Dot's own echo is not heard.
    pub reopen_after_ms: u32,
    /// Play a soft tone when listening starts from rest. On `open` while `Idle` it sounds at
    /// once and the ears open once it has finished sounding and a 60 ms tail has passed, so
    /// it is never heard as speech. After "Hey Dot" the ears open at once and the tone sounds
    /// only if no speech starts within 300 ms (the person paused, waiting); the ears are deaf
    /// only while it sounds plus the same 60 ms tail. Not on a follow-up, an ask, an
    /// interruption or a click; with no working speaker there is no tone.
    pub tone: bool,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            wake_phrase: false,
            interrupt_by_talking: false,
            follow_up_secs: 6,
            reopen_after_ms: 250,
            tone: true,
        }
    }
}

/// What happens once a reply has been spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    /// Rest: back to `Idle`.
    Rest,
    /// Listen for a follow-up for `follow_up_secs`.
    FollowUp,
    /// The reply asked something: listen for a short answer, for up to 20 seconds.
    Ask,
}

/// Where the session's timers take the time from.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Opens the microphone each time the session needs a stream.
pub type MicOpener = Box<dyn Fn() -> Result<Box<dyn Mic>, EngineError> + Send>;

/// Opens a fresh speaker when the one in use has failed.
pub type SinkOpener = Box<dyn Fn() -> Result<Arc<dyn Sink>, EngineError> + Send>;

/// Everything the session runs on. It takes ownership; all of it lives on the worker thread.
pub struct Parts {
    /// For sentences.
    pub ears: Ears,
    /// For one-word answers, used while `Asking`.
    pub short_ears: Ears,
    pub voice: Voice,
    /// The receiving end of the channel `voice` was made with.
    pub voice_events: UnboundedReceiver<VoiceEvent>,
    /// The sink `voice` plays into.
    pub sink: Arc<dyn Sink>,
    /// Called before speaking when `sink` has failed; the new sink replaces it in `voice`.
    pub open_sink: SinkOpener,
    /// `None` makes the wake phrase setting do nothing.
    pub wake: Option<Box<dyn WakeSpotter>>,
    /// Called when a stream is needed; the `Mic` is dropped when it no longer is.
    pub open_mic: MicOpener,
}

enum Command {
    Open,
    Close,
    Interrupt,
    Thinking,
    Say {
        turn: u64,
        text: String,
        then: After,
    },
    Delta {
        turn: u64,
        text: String,
    },
    Question {
        turn: u64,
        text: String,
    },
    End {
        turn: u64,
        then: After,
    },
    Answered {
        turn: u64,
        text: String,
    },
    Filler {
        turn: u64,
        text: String,
    },
    Notice(String),
    Settings(SessionSettings),
    SetVoice {
        voice: u8,
        speed: f32,
    },
    Shutdown,
    #[cfg(test)]
    Probe(Sender<Probe>),
}

/// What a test can ask the worker about after everything sent before has been handled.
#[cfg(test)]
#[derive(Debug)]
struct Probe {
    state: State,
    /// Speaking, the reply has been spoken and the sink is quiet: the tail is being waited out.
    speech_done: bool,
}

struct Shared {
    state: AtomicU8,
    /// The turn of the latest `Heard` sent.
    turn: AtomicU64,
    /// The last turn id handed out; spoken and clicked turns share it, so ids never repeat.
    next_turn: AtomicU64,
    /// Set by `shutdown`, the last handle going, or a panic in the worker.
    ended: AtomicBool,
}

struct Handle {
    commands: Sender<Command>,
    shared: Arc<Shared>,
}

impl Handle {
    fn end(&self) {
        if !self.shared.ended.swap(true, Ordering::SeqCst) {
            let _ = self.commands.send(Command::Shutdown);
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.end();
    }
}

/// A handle to the session; cheap to clone. Every method returns at once. Dropping the last
/// handle is the same as `shutdown`.
#[derive(Clone)]
pub struct Session {
    handle: Arc<Handle>,
}

impl Session {
    pub fn start(
        parts: Parts,
        settings: SessionSettings,
        events: UnboundedSender<SessionEvent>,
    ) -> Self {
        Self::start_with_clock(parts, settings, events, Arc::new(SystemClock))
    }

    /// `start` with the timers reading `clock`.
    pub fn start_with_clock(
        parts: Parts,
        settings: SessionSettings,
        events: UnboundedSender<SessionEvent>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: AtomicU8::new(State::Idle as u8),
            turn: AtomicU64::new(0),
            next_turn: AtomicU64::new(0),
            ended: AtomicBool::new(false),
        });
        let (commands, inbox) = channel();
        let worker = Worker::new(parts, settings, events.clone(), clock, shared.clone());
        let after_panic = shared.clone();
        std::thread::Builder::new()
            .name("dots-voice-session".into())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(move || worker.run(&inbox)));
                if outcome.is_err() {
                    // The worker and everything it held are gone; say so and stay Off.
                    after_panic.ended.store(true, Ordering::SeqCst);
                    after_panic.state.store(State::Off as u8, Ordering::SeqCst);
                    let _ = events.send(SessionEvent::Trouble {
                        message: BROKE.into(),
                    });
                    let _ = events.send(SessionEvent::State {
                        state: State::Off,
                        mic_open: false,
                    });
                }
            })
            .expect("the session thread starts");
        Self {
            handle: Arc::new(Handle { commands, shared }),
        }
    }

    fn send(&self, command: Command) {
        if !self.handle.shared.ended.load(Ordering::SeqCst) {
            let _ = self.handle.commands.send(command);
        }
    }

    /// `Idle` -> `Listening`. While `Speaking` or `Thinking` it is `interrupt`; otherwise it
    /// does nothing. Also lets a microphone that failed be tried again.
    pub fn open(&self) {
        self.send(Command::Open);
    }

    /// Any state -> `Idle`; stops speech first. A reply still coming for the current turn is
    /// no longer spoken.
    pub fn close(&self) {
        self.send(Command::Close);
    }

    /// `Speaking` or `Thinking` -> `Listening`; stops speech first. A reply still coming for
    /// the current turn is no longer spoken; the app cancels its own request.
    ///
    /// After interrupting speech, `Listening` shows at once but the ears hear nothing for
    /// `reopen_after_ms`, so Dot's own voice is never transcribed (the wake phrase is still
    /// heard). Known limit: a "Dot, stop" said while a finished sentence is being transcribed
    /// is thrown away with the rest of that backlog.
    pub fn interrupt(&self) {
        self.send(Command::Interrupt);
    }

    /// The app took the last `Heard` and is asking the mind. Only meaningful while
    /// `Thinking`: the first call restarts the two-minute limit for the reply.
    pub fn thinking(&self) {
        self.send(Command::Thinking);
    }

    /// A whole reply for `turn`. Ignored unless `turn` is the current turn and it has not
    /// been answered, cut short or given up on.
    pub fn say(&self, turn: u64, text: &str, then: After) {
        self.send(Command::Say {
            turn,
            text: text.to_string(),
            then,
        });
    }

    /// Part of a streamed reply for `turn`; ignored as `say` is.
    pub fn say_delta(&self, turn: u64, delta: &str) {
        self.send(Command::Delta {
            turn,
            text: delta.to_string(),
        });
    }

    /// A question the app built for `turn`, said after the words so far as a reply of its
    /// own: the words' length cap never cuts it. The turn's reply goes on (`end_of_reply`
    /// ends it, normally with `After::Ask`). If the question is not handed to the speaker
    /// whole, the session does not ask: it reports trouble and rests. Ignored as `say` is.
    pub fn question(&self, turn: u64, text: &str) {
        self.send(Command::Question {
            turn,
            text: text.to_string(),
        });
    }

    /// The streamed reply for `turn` is complete; ignored as `say` is.
    pub fn end_of_reply(&self, turn: u64, then: After) {
        self.send(Command::End { turn, then });
    }

    /// A click or key press on an option: it becomes a turn exactly as if it had been said.
    /// Listening or asking stops (and speech or a reply in progress is stopped, as
    /// `interrupt` does), `Heard { text, turn }` is sent with a new turn id, and the session
    /// is `Thinking`. Returns that turn id, or `None` once the session is `Off`.
    ///
    /// Turn ids never repeat, but when a spoken sentence ends at the same moment, the two
    /// `Heard` events can arrive in either order; the later one is the current turn.
    pub fn answered(&self, text: &str) -> Option<u64> {
        let shared = &self.handle.shared;
        if shared.ended.load(Ordering::SeqCst) {
            return None;
        }
        let turn = shared.next_turn.fetch_add(1, Ordering::SeqCst) + 1;
        self.send(Command::Answered {
            turn,
            text: text.to_string(),
        });
        Some(turn)
    }

    /// A short line ("One moment.") spoken while `Thinking` for `turn`, which stays
    /// `Thinking`: the two-minute limit keeps running from where it was, and the line is not
    /// the reply. The reply that follows (`say`, `say_delta`) is spoken after it. Dropped
    /// unless `turn` is the current turn, still waiting for its reply.
    pub fn filler(&self, turn: u64, text: &str) {
        self.send(Command::Filler {
            turn,
            text: text.to_string(),
        });
    }

    /// Something to say that answers no turn; spoken only while `Idle`, dropped otherwise.
    pub fn notice(&self, text: &str) {
        self.send(Command::Notice(text.to_string()));
    }

    /// Takes effect at once.
    pub fn settings(&self, settings: SessionSettings) {
        self.send(Command::Settings(settings));
    }

    /// The voice and speed for sentences synthesised from now on; see `Voice::set_voice`
    /// (speed is kept within 0.8..=1.4).
    pub fn set_voice(&self, voice: u8, speed: f32) {
        self.send(Command::SetVoice { voice, speed });
    }

    /// Final: speech stops, the microphone closes, the state becomes `Off` and the worker
    /// ends. Later calls do nothing.
    ///
    /// Returns at once and does not wait for the worker. The worker is gone, with the
    /// microphone and speaker released, when the event channel closes (the receiver's
    /// `recv` returns `None`); wait on that if it matters.
    pub fn shutdown(&self) {
        self.handle.end();
    }

    pub fn state(&self) -> State {
        let shared = &self.handle.shared;
        if shared.ended.load(Ordering::SeqCst) {
            State::Off
        } else {
            State::from_u8(shared.state.load(Ordering::SeqCst))
        }
    }

    /// The turn of the latest `Heard`; 0 before the first.
    pub fn turn(&self) -> u64 {
        self.handle.shared.turn.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn probe(&self) -> Probe {
        let (tx, rx) = channel();
        let _ = self.handle.commands.send(Command::Probe(tx));
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the worker answers")
    }
}

/// An open microphone. The receiver is dropped before the microphone.
struct MicStream {
    frames: Receiver<MicFrame>,
    _mic: Box<dyn Mic>,
}

struct Worker {
    ears: Ears,
    short_ears: Ears,
    voice: Voice,
    voice_events: UnboundedReceiver<VoiceEvent>,
    sink: Arc<dyn Sink>,
    open_sink: SinkOpener,
    wake: Option<Box<dyn WakeSpotter>>,
    open_mic: MicOpener,
    settings: SessionSettings,
    events: UnboundedSender<SessionEvent>,
    clock: Arc<dyn Clock>,
    shared: Arc<Shared>,

    state: State,
    mic: Option<MicStream>,
    /// The microphone failed. Only listening opens it at once; for the wake phrase (or
    /// talking over Dot) it is tried again every `MIC_RETRY`, from `mic_retry_at`.
    mic_broken: bool,
    mic_retry_at: Instant,
    /// The microphone was lost while Thinking or Speaking and nobody has been told yet: the
    /// next state that needs it tries it at once and reports if it still will not open.
    mic_owed: bool,
    published: Option<(State, bool)>,
    last_level: Option<Instant>,

    /// Listening and Asking: when the quiet began, how much of it ends the listening, and
    /// whether a clip is open (the limit is paused then; `clip_started` is when it opened).
    /// A dropped clip moves `quiet_since` on by the clip's length only, so noise cannot hold
    /// the listening open.
    quiet_since: Instant,
    listen_limit: Duration,
    in_speech: bool,
    clip_started: Instant,
    /// After speech was interrupted (not by talking), the ears hear nothing until then:
    /// frames from before it hold Dot's own voice. The wake spotter still hears them.
    deaf_until: Option<Instant>,
    /// The listening tone is sounding: the ears hear nothing until the sink is quiet (then
    /// `deaf_until` takes over for the tail), or until this limit, when it is stopped.
    tone_until: Option<Instant>,
    /// Listening after "Hey Dot" with the tone on and no speech yet: the tone is played then,
    /// unless speech starts first.
    tone_due: Option<Instant>,
    /// Idle with the wake phrase on: the last frames the spotter heard, newest last, so the
    /// words said after "Hey Dot" but before the spotter answered reach the ears.
    recent: VecDeque<Vec<f32>>,
    /// Stop handling frames for the rest of this pass.
    halt_frames: bool,
    /// Frames were left waiting at the end of the last pass.
    frames_left: bool,

    /// Speaking: the last word from the voice (or the start of speaking).
    voice_heard_at: Instant,

    /// Thinking: since when, and whether `thinking()` has restarted the limit.
    thinking_since: Instant,
    asked: bool,

    turn: u64,
    /// The current turn may still be answered.
    turn_open: bool,
    /// A streamed reply has text at the voice and no end yet.
    reply_open: bool,
    /// Replies ended at the voice whose `Finished` has not come.
    replies_pending: usize,
    /// Filler lines at the voice whose `Finished` has not come. They were queued before any
    /// reply, so the first `Finished` events are theirs.
    fillers_pending: usize,
    then: After,
    /// When the ears may hear again after speech.
    reopen_at: Option<Instant>,
    /// "The speaker could not be opened" was said for this turn (or notice) already.
    speaker_told: bool,

    /// Replies (not fillers) ended at the voice, and finished by it, since speech last
    /// stopped: the voice finishes them in order, so the n-th `Finished` is the n-th reply's.
    replies_started: u64,
    replies_done: u64,
    /// The number of the reply that is a question (`question`), and whether the voice said
    /// it could not hand all of it to the speaker.
    question_reply: Option<u64>,
    question_cut: bool,
    /// A short line owed to the person once the session rests (a microphone trouble, a
    /// question that went unanswered): said at the next rest when a speaker works.
    owed: Option<String>,
    /// Follow-up listens in a row since the last hotkey, click or "Hey Dot".
    follow_ups: u32,
    /// This listen's frames so far, and whether any held a sound at all.
    frames_heard: usize,
    any_sound: bool,
    /// This listen began from rest (hotkey or "Hey Dot"): one sentence heard live but empty
    /// when finished is answered "Sorry, I didn't catch that." and listened for once more.
    may_retry: bool,
    /// Words of this sentence were shown live.
    partial_seen: bool,
}

impl Worker {
    fn new(
        parts: Parts,
        settings: SessionSettings,
        events: UnboundedSender<SessionEvent>,
        clock: Arc<dyn Clock>,
        shared: Arc<Shared>,
    ) -> Self {
        let now = clock.now();
        Self {
            ears: parts.ears,
            short_ears: parts.short_ears,
            voice: parts.voice,
            voice_events: parts.voice_events,
            sink: parts.sink,
            open_sink: parts.open_sink,
            wake: parts.wake,
            open_mic: parts.open_mic,
            settings,
            events,
            clock,
            shared,
            state: State::Idle,
            mic: None,
            mic_broken: false,
            mic_retry_at: now,
            mic_owed: false,
            published: None,
            last_level: None,
            quiet_since: now,
            listen_limit: LISTEN_AFTER_OPEN,
            in_speech: false,
            clip_started: now,
            deaf_until: None,
            tone_until: None,
            tone_due: None,
            recent: VecDeque::new(),
            halt_frames: false,
            frames_left: false,
            voice_heard_at: now,
            thinking_since: now,
            asked: false,
            turn: 0,
            turn_open: false,
            reply_open: false,
            replies_pending: 0,
            fillers_pending: 0,
            then: After::Rest,
            reopen_at: None,
            speaker_told: false,
            replies_started: 0,
            replies_done: 0,
            question_reply: None,
            question_cut: false,
            owed: None,
            follow_ups: 0,
            frames_heard: 0,
            any_sound: false,
            may_retry: false,
            partial_seen: false,
        }
    }

    fn run(mut self, inbox: &Receiver<Command>) {
        self.refresh();
        loop {
            // With frames still waiting, only look for commands; do not wait.
            let wait = if self.frames_left {
                Duration::ZERO
            } else {
                TICK
            };
            let mut next = match inbox.recv_timeout(wait) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Command::Shutdown),
            };
            #[cfg(test)]
            let mut probes = Vec::new();
            while let Some(command) = next {
                match command {
                    Command::Shutdown => {
                        self.shut_down();
                        return;
                    }
                    #[cfg(test)]
                    Command::Probe(reply) => probes.push(reply),
                    command => self.command(command),
                }
                next = match inbox.try_recv() {
                    Ok(command) => Some(command),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(Command::Shutdown),
                };
            }
            self.voice_events();
            self.mic_frames();
            self.timers();
            #[cfg(test)]
            for reply in probes {
                let _ = reply.send(Probe {
                    state: self.state,
                    speech_done: self.state == State::Speaking && self.reopen_at.is_some(),
                });
            }
        }
    }

    fn send(&self, event: SessionEvent) {
        // A closed receiver only means nobody is listening any more.
        let _ = self.events.send(event);
    }

    fn trouble(&self, message: &str) {
        self.send(SessionEvent::Trouble {
            message: message.to_string(),
        });
    }

    // ---- commands ------------------------------------------------------------------------

    fn command(&mut self, command: Command) {
        match command {
            Command::Open => {
                self.mic_broken = false;
                self.follow_ups = 0;
                match self.state {
                    State::Idle => self.listen_from_rest(),
                    State::Thinking | State::Speaking => self.interrupt(),
                    _ => {}
                }
            }
            Command::Close => self.close(),
            Command::Interrupt => {
                if matches!(self.state, State::Thinking | State::Speaking) {
                    self.interrupt();
                }
            }
            Command::Thinking => {
                if self.state == State::Thinking && !self.asked {
                    self.asked = true;
                    self.thinking_since = self.clock.now();
                }
            }
            Command::Say { turn, text, then } => {
                if self.answers(turn) {
                    self.voice_heard_at = self.clock.now();
                    if !self.ready_sink() {
                        return self.speaker_gone();
                    }
                    self.voice.say(&text);
                    self.reply_open = false;
                    self.replies_pending += 1;
                    self.replies_started += 1;
                    self.reply_ended(then);
                }
            }
            Command::Question { turn, text } => {
                if self.answers(turn) {
                    self.voice_heard_at = self.clock.now();
                    if self.reply_open {
                        // The words before it are a reply of their own.
                        self.voice.end_of_reply();
                        self.reply_open = false;
                        self.replies_pending += 1;
                        self.replies_started += 1;
                    } else if !self.ready_sink() {
                        return self.speaker_gone();
                    }
                    self.voice.say(&text);
                    self.replies_pending += 1;
                    self.replies_started += 1;
                    self.question_reply = Some(self.replies_started);
                    self.question_cut = false;
                    self.enter(State::Speaking);
                }
            }
            Command::Delta { turn, text } => {
                if self.answers(turn) {
                    // A long streamed reply is not cut while its text is still arriving.
                    self.voice_heard_at = self.clock.now();
                    if !self.reply_open && !self.ready_sink() {
                        return self.speaker_gone();
                    }
                    self.reply_open = true;
                    self.voice.say_delta(&text);
                    self.enter(State::Speaking);
                }
            }
            Command::End { turn, then } => {
                if self.answers(turn) {
                    self.voice_heard_at = self.clock.now();
                    if self.reply_open {
                        self.voice.end_of_reply();
                        self.reply_open = false;
                        self.replies_pending += 1;
                        self.replies_started += 1;
                    }
                    self.reply_ended(then);
                }
            }
            Command::Answered { turn, text } => {
                if self.state != State::Off {
                    self.follow_ups = 0;
                    self.stop_speech();
                    self.heard(turn, text);
                }
            }
            Command::Filler { turn, text } => {
                if self.turn_open
                    && turn == self.turn
                    && self.state == State::Thinking
                    && self.ready_sink()
                {
                    self.voice.say(&text);
                    self.fillers_pending += 1;
                }
            }
            Command::Notice(text) => {
                // A notice is no turn's: a missing speaker is told for it on its own.
                self.speaker_told = false;
                if self.state == State::Idle && self.ready_sink() {
                    self.say_alone(&text, After::Rest);
                }
            }
            Command::Settings(settings) => {
                self.settings = settings;
                self.mic_broken = false;
                self.refresh();
            }
            Command::SetVoice { voice, speed } => self.voice.set_voice(voice, speed),
            Command::Shutdown => {}
            #[cfg(test)]
            Command::Probe(_) => {}
        }
    }

    /// Text for `turn` is taken: it is the current turn and its reply is still to come.
    fn answers(&self, turn: u64) -> bool {
        self.turn_open
            && turn == self.turn
            && matches!(self.state, State::Thinking | State::Speaking)
    }

    /// The reply for the current turn has all its text.
    fn reply_ended(&mut self, then: After) {
        self.turn_open = false;
        self.then = then;
        self.reopen_at = None;
        if self.speech_done() {
            self.after_speech(); // nothing to say: move on at once
        } else {
            self.enter(State::Speaking);
        }
    }

    /// Makes sure the voice has a working speaker, opening a fresh one if the last one
    /// failed. False (and reported) when none will open.
    fn ready_sink(&mut self) -> bool {
        if !self.sink.failed() {
            return true;
        }
        match (self.open_sink)() {
            Ok(sink) => {
                self.voice.set_sink(sink.clone());
                self.sink = sink;
                true
            }
            Err(e) => {
                // Once per turn: a filler and then the reply find the same missing speaker.
                if !self.speaker_told {
                    self.speaker_told = true;
                    self.send(SessionEvent::Trouble {
                        message: format!("The speaker could not be opened: {e}"),
                    });
                }
                false
            }
        }
    }

    /// The reply cannot be spoken: no speaker will open. Already reported; rest.
    fn speaker_gone(&mut self) {
        self.stop_speech();
        self.enter(State::Idle);
    }

    /// Says `text` as a reply of no turn (a notice, a line owed), then goes where `then` says.
    /// The caller made sure a speaker works.
    fn say_alone(&mut self, text: &str, then: After) {
        self.voice.say(text);
        self.replies_pending += 1;
        self.replies_started += 1;
        self.then = then;
        self.reopen_at = None;
        self.enter(State::Speaking);
    }

    /// A finished sentence, spoken or clicked, becomes turn `turn`.
    fn heard(&mut self, turn: u64, text: String) {
        self.turn = turn;
        self.shared.turn.store(turn, Ordering::SeqCst);
        self.turn_open = true;
        self.speaker_told = false;
        self.may_retry = false;
        self.send(SessionEvent::Heard { text, turn });
        if let Some(spotter) = &mut self.wake {
            spotter.reset();
        }
        self.enter(State::Thinking);
    }

    fn close(&mut self) {
        if self.state != State::Off {
            self.stop_speech();
            self.enter(State::Idle);
        }
    }

    /// Stops speech and listens. The state shows `Listening` at once, but when Dot was
    /// speaking, the frames already waiting are thrown away (they hold Dot's voice), the rest
    /// of this pass's frames are left for the next pass, and the ears stay deaf for
    /// `reopen_after_ms`, as after a normal end of speech. (Interruption by talking does not
    /// come here: that clip is kept.)
    fn interrupt(&mut self) {
        let was_speaking = self.state == State::Speaking || self.fillers_pending > 0;
        // Timed from the stop, before `Listening` is published.
        let tail = Duration::from_millis(u64::from(self.settings.reopen_after_ms));
        let deaf_until = was_speaking.then(|| self.clock.now() + tail);
        self.stop_speech();
        if was_speaking {
            if !self.discard_backlog() {
                return; // the microphone was lost: reported, and resting
            }
            self.halt_frames = true;
        }
        self.listen(LISTEN_AFTER_OPEN);
        self.deaf_until = deaf_until;
    }

    fn shut_down(&mut self) {
        self.stop_speech();
        self.enter(State::Off);
    }

    /// Silences the voice and forgets the reply in progress. The current turn is closed, so a
    /// late reply for it is dropped here and never reaches the voice. A streamed reply cut
    /// short is ended at the voice at once, so the voice does not wait for an
    /// `end_of_reply` that may never come before it speaks the next reply.
    fn stop_speech(&mut self) {
        self.voice.stop();
        if self.reply_open {
            self.voice.end_of_reply(); // only clears the voice's "cut short" latch
        }
        // Whatever the voice sent before `stop` returned belongs to the speech just stopped;
        // it sends nothing for that speech afterwards.
        while self.voice_events.try_recv().is_ok() {}
        self.turn_open = false;
        self.asked = false;
        self.reply_open = false;
        self.replies_pending = 0;
        self.fillers_pending = 0;
        self.reopen_at = None;
        self.replies_started = 0;
        self.replies_done = 0;
        self.question_reply = None;
        self.question_cut = false;
    }

    // ---- states --------------------------------------------------------------------------

    /// Listening starts from rest by `open` while `Idle` (hotkey, click, tray): with the tone
    /// on, it sounds first and the ears wait for it to end.
    fn listen_from_rest(&mut self) {
        self.listen(LISTEN_AFTER_OPEN);
        self.may_retry = true;
        self.play_tone();
    }

    /// "Hey Dot" while `Idle`: the ears open at once, so words said in the same breath are
    /// heard whole (nothing captured after the wake phrase is thrown away). With the tone on,
    /// it sounds only if no speech has started `TONE_WAIT` after the wake.
    fn woke(&mut self) {
        let after = self.wake.as_ref().map_or(0, |spotter| spotter.after_word());
        let recent = std::mem::take(&mut self.recent);
        self.send(SessionEvent::Woke);
        self.follow_ups = 0;
        self.listen(LISTEN_AFTER_OPEN);
        if self.state != State::Listening {
            return;
        }
        self.may_retry = true;
        if self.settings.tone {
            self.tone_due = Some(self.clock.now() + TONE_WAIT);
        }
        // The words after the wake phrase start at once, with no quiet before them: the ears
        // are given a moment of silence first, as their pre-roll would have held, without
        // which the recogniser returns nothing for about one such sentence in ten.
        self.ears.lead_in_silence();
        // The spotter answers a little after the word ends: what was said in between is
        // given to the ears first, so the sentence starts where the wake phrase ended.
        let keep = after.div_ceil(FRAME).min(recent.len());
        let now = self.clock.now();
        for frame in recent.range(recent.len() - keep..) {
            for event in self.ears.push(frame) {
                if !self.state.hears() {
                    return;
                }
                self.ear_event(event, now);
            }
        }
    }

    /// Plays the listening tone through the sink speech uses, and keeps the ears deaf while it
    /// sounds. Nothing when the tone is off, the session did not reach `Listening` (no
    /// microphone), or the sink has failed or refuses it: the ears are open at once then.
    fn play_tone(&mut self) {
        if !self.settings.tone || self.state != State::Listening || self.sink.failed() {
            return;
        }
        if self.sink.play(listen_tone(TONE_RATE)) {
            self.tone_until = Some(self.clock.now() + TONE_LIMIT);
        }
    }

    fn listen(&mut self, limit: Duration) {
        self.ears.reset();
        self.in_speech = false;
        self.deaf_until = None;
        self.tone_until = None;
        self.tone_due = None;
        self.quiet_since = self.clock.now();
        self.listen_limit = limit;
        self.fresh_listen();
        self.enter(State::Listening);
    }

    /// A listen begins: nothing heard yet in it.
    fn fresh_listen(&mut self) {
        self.may_retry = false;
        self.partial_seen = false;
        self.frames_heard = 0;
        self.any_sound = false;
        self.owed = None;
    }

    fn ask(&mut self) {
        self.fresh_listen();
        self.short_ears.reset();
        self.in_speech = false;
        self.deaf_until = None;
        self.tone_until = None;
        self.tone_due = None;
        self.quiet_since = self.clock.now();
        self.listen_limit = ASK_LIMIT;
        self.enter(State::Asking);
    }

    fn enter(&mut self, new: State) {
        let old = self.state;
        if new != old {
            if old.has_level() {
                self.send(SessionEvent::Level {
                    mic: 0.0,
                    voice: 0.0,
                });
            }
            self.last_level = None;
            self.tone_due = None;
            self.recent.clear();
            if !new.hears() {
                self.ears.reset();
                self.short_ears.reset();
                self.in_speech = false;
                self.deaf_until = None;
                if self.tone_until.take().is_some() {
                    self.sink.stop(); // closed while the tone sounds
                }
            }
            if new == State::Thinking {
                self.thinking_since = self.clock.now();
                self.asked = false;
            }
            if new == State::Speaking {
                self.voice_heard_at = self.clock.now();
            }
            self.state = new;
        }
        self.refresh();
    }

    /// Opens or closes the microphone to fit the state and settings, then publishes. A
    /// microphone that will not open is reported; when listening needed it, the session
    /// rests, and otherwise it is tried again every `MIC_RETRY`.
    fn refresh(&mut self) {
        if let Err(e) = self.fit_mic() {
            self.send(SessionEvent::Trouble {
                message: format!("The microphone could not be opened: {e}"),
            });
            self.mic_broken = true;
            self.mic_owed = false;
            self.mic_retry_at = self.clock.now() + MIC_RETRY;
            if self.state.hears() {
                self.stop_speech();
                self.ears.reset();
                self.short_ears.reset();
                self.in_speech = false;
                self.last_level = None;
                self.state = State::Idle;
                let _ = self.fit_mic(); // only closes now, or waits for the retry
                                        // The person asked to be heard: told aloud too, once resting.
                self.owed = Some(MIC_WONT_OPEN.into());
            }
        }
        self.publish();
    }

    fn wants_mic(&self) -> bool {
        let wake = self.settings.wake_phrase && self.wake.is_some();
        match self.state {
            State::Off => false,
            State::Listening | State::Asking => true,
            State::Idle | State::Thinking => wake,
            State::Speaking => wake || self.settings.interrupt_by_talking,
        }
    }

    fn fit_mic(&mut self) -> Result<(), EngineError> {
        if !self.wants_mic() {
            self.mic = None;
            return Ok(());
        }
        if self.mic.is_some() {
            return Ok(());
        }
        // A broken microphone is opened at once only to listen, or when its loss is still
        // to be reported and speech is over; otherwise the retry timer tries it.
        let busy = matches!(self.state, State::Thinking | State::Speaking);
        if self.mic_broken && !self.state.hears() && !(self.mic_owed && !busy) {
            return Ok(());
        }
        self.open_mic_now()
    }

    fn open_mic_now(&mut self) -> Result<(), EngineError> {
        let mut mic = (self.open_mic)()?;
        let frames = mic.frames();
        self.mic = Some(MicStream { frames, _mic: mic });
        self.mic_broken = false;
        self.mic_owed = false;
        self.recent.clear();
        // A fresh stream: nothing heard on the old one joins what the new one hears.
        if let Some(spotter) = &mut self.wake {
            spotter.reset();
        }
        self.ears.reset();
        self.short_ears.reset();
        self.in_speech = false;
        Ok(())
    }

    /// Every `MIC_RETRY` a broken microphone that the wake phrase (or talking over Dot)
    /// wants is tried again, quietly: its loss was reported once already, or will be.
    fn retry_mic(&mut self, now: Instant) {
        if !self.mic_broken || self.mic.is_some() || !self.wants_mic() || now < self.mic_retry_at {
            return;
        }
        if self.open_mic_now().is_ok() {
            self.publish();
        } else {
            self.mic_retry_at = now + MIC_RETRY;
        }
    }

    fn publish(&mut self) {
        let now = (self.state, self.mic.is_some());
        self.shared.state.store(self.state as u8, Ordering::SeqCst);
        if self.published != Some(now) {
            self.published = Some(now);
            self.send(SessionEvent::State {
                state: now.0,
                mic_open: now.1,
            });
        }
    }

    // ---- the microphone --------------------------------------------------------------------

    /// Handles up to `FRAMES_PER_PASS` waiting frames; more are left for the next pass, which
    /// then comes without waiting.
    fn mic_frames(&mut self) {
        self.halt_frames = false;
        self.frames_left = false;
        self.end_deafness();
        for _ in 0..FRAMES_PER_PASS {
            if self.halt_frames {
                return;
            }
            let Some(mic) = &self.mic else { return };
            match mic.frames.try_recv() {
                Ok(MicFrame::Audio(frame)) => self.frame(&frame),
                Ok(MicFrame::Lost(_)) | Err(TryRecvError::Disconnected) => {
                    self.mic_lost();
                    return;
                }
                Err(TryRecvError::Empty) => return,
            }
        }
        self.frames_left = true;
    }

    /// Once the listening tone has stopped sounding, its tail begins. Once the tail after an
    /// interruption (or the tone) has passed, the frames still waiting (captured before its
    /// end) are thrown away and the ears start clean.
    fn end_deafness(&mut self) {
        if let Some(limit) = self.tone_until {
            let now = self.clock.now();
            let quiet = self.sink.idle();
            if quiet || now >= limit {
                if !quiet {
                    self.sink.stop();
                }
                // The tone has stopped sounding: a short tail after it is waited out, then the
                // frames captured until then are thrown away and every later one is kept. The
                // listen limit counts from the moment the ears open.
                // The device's own output delay, when it tells, is waited out too.
                let tail = TONE_TAIL + self.device_delay().unwrap_or_default();
                self.tone_until = None;
                self.deaf_until = Some(now + tail);
                self.quiet_since = now + tail;
            }
        }
        if self.deaf_until.is_some_and(|at| self.clock.now() >= at) {
            self.deaf_until = None;
            if self.discard_backlog() {
                self.ears.reset();
                self.short_ears.reset();
            }
        }
    }

    /// Throws away the frames already waiting: they were captured before the moment the
    /// caller cares about (a sentence just ended, or speech just finished). Called before the
    /// state change is published, so nothing sent after the app saw it is lost. False when
    /// the microphone turned out to be lost and the session rested because of it.
    fn discard_backlog(&mut self) -> bool {
        loop {
            let Some(mic) = &self.mic else { return true };
            match mic.frames.try_recv() {
                Ok(MicFrame::Audio(_)) => {}
                Ok(MicFrame::Lost(_)) | Err(TryRecvError::Disconnected) => {
                    return !self.mic_lost();
                }
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    /// The open microphone stopped working. It is tried again every `MIC_RETRY` while the
    /// wake phrase wants it. Thinking and Speaking do not need it, so they carry on and the
    /// loss is reported once speech is over, if the microphone still will not open then.
    /// Otherwise it is reported now and the session rests. True when it rested.
    fn mic_lost(&mut self) -> bool {
        self.mic = None;
        self.mic_broken = true;
        self.mic_retry_at = self.clock.now() + MIC_RETRY;
        if matches!(self.state, State::Thinking | State::Speaking) {
            self.mic_owed = true;
            self.publish();
            return false;
        }
        let heard = self.state.hears();
        self.trouble(MIC_LOST);
        self.stop_speech();
        self.enter(State::Idle);
        if heard {
            self.owed = Some(MIC_LOST.into());
        }
        true
    }

    /// The wake word in this frame, if the wake phrase is on.
    fn spot(&mut self, frame: &[f32]) -> Option<WakeWord> {
        if !self.settings.wake_phrase {
            return None;
        }
        let spotter = self.wake.as_mut()?;
        let word = spotter.heard(frame);
        if word.is_some() {
            spotter.reset();
        }
        word
    }

    fn frame(&mut self, frame: &[f32]) {
        let now = self.clock.now();
        // The spotter hears every frame, the listening tone's too. Nothing is done about the
        // tone: it is two pure notes with no words in them, so it cannot match "Hey Dot" or
        // "Dot, stop" (the spotter needs the keywords' sounds in order), and keeping the
        // spotter fed means "Dot, stop" said over the tone still closes.
        let word = self.spot(frame);
        match self.state {
            State::Off => {}
            State::Idle => {
                if self.wake.is_some() && self.settings.wake_phrase {
                    if self.recent.len() >= RECENT_FRAMES {
                        self.recent.pop_front();
                    }
                    self.recent.push_back(frame.to_vec());
                }
                if word == Some(WakeWord::HeyDot) {
                    self.woke();
                }
            }
            State::Thinking => {
                if word.is_some() {
                    self.wake_over_dot(word);
                }
            }
            State::Speaking => {
                if word.is_some() {
                    self.wake_over_dot(word);
                } else if self.settings.interrupt_by_talking && !self.speech_done() {
                    // Only while the voice is still going: the tail after it is never heard.
                    if self.ears.push(frame).contains(&EarEvent::Started) {
                        self.stop_speech();
                        // The clip just begun carries on: the ears are not reset.
                        self.in_speech = true;
                        self.clip_started = now;
                        self.quiet_since = now;
                        self.listen_limit = LISTEN_AFTER_OPEN;
                        self.enter(State::Listening);
                    }
                }
            }
            State::Listening | State::Asking => {
                self.frames_heard += 1;
                self.any_sound |= frame.iter().any(|s| *s != 0.0);
                self.level(level(frame), 0.0, now);
                if word == Some(WakeWord::DotStop) {
                    self.close();
                    return;
                }
                if self.deaf_until.is_some() || self.tone_until.is_some() {
                    return; // the tone, or still the tail of Dot's own voice
                }
                let events = if self.state == State::Asking {
                    self.short_ears.push(frame)
                } else {
                    self.ears.push(frame)
                };
                for event in events {
                    if !self.state.hears() {
                        break;
                    }
                    self.ear_event(event, now);
                }
            }
        }
    }

    /// A wake word said while Dot thinks or speaks stops it and listens. "Hey Dot" is a wake
    /// like one from rest: the follow-up count starts again.
    fn wake_over_dot(&mut self, word: Option<WakeWord>) {
        if word == Some(WakeWord::HeyDot) {
            self.follow_ups = 0;
        }
        self.interrupt();
    }

    fn ear_event(&mut self, event: EarEvent, now: Instant) {
        match event {
            EarEvent::Started => {
                self.in_speech = true;
                self.clip_started = now;
                self.tone_due = None; // the person kept talking after "Hey Dot": no tone
            }
            EarEvent::Partial(text) => {
                self.partial_seen = true;
                self.send(SessionEvent::Hearing { text });
            }
            EarEvent::Final(text) => {
                // What queued up while the sentence was transcribed is not fresh sound. Known
                // limit: a "Dot, stop" said during that transcription is thrown away with it.
                if !self.discard_backlog() {
                    return;
                }
                let turn = self.shared.next_turn.fetch_add(1, Ordering::SeqCst) + 1;
                self.heard(turn, text);
            }
            EarEvent::Dropped => {
                // The limit was paused while the clip was open; it is not restarted.
                self.in_speech = false;
                self.quiet_since += now.saturating_duration_since(self.clip_started);
                // Words were shown as they were said, and then nothing came of them: never
                // in silence. Said once per start from rest, then listened for once more.
                let missed = std::mem::take(&mut self.partial_seen);
                if missed && self.may_retry && self.state == State::Listening && !self.sink.failed()
                {
                    self.may_retry = false;
                    self.say_alone(DIDNT_CATCH, After::FollowUp);
                }
            }
        }
    }

    fn level(&mut self, mic: f32, voice: f32, now: Instant) {
        let due = self
            .last_level
            .is_none_or(|last| now.saturating_duration_since(last) >= LEVEL_GAP);
        if due {
            self.last_level = Some(now);
            self.send(SessionEvent::Level { mic, voice });
        }
    }

    // ---- the voice and the timers ----------------------------------------------------------

    fn voice_events(&mut self) {
        while let Ok(event) = self.voice_events.try_recv() {
            // Any word from the voice shows it is alive.
            self.voice_heard_at = self.clock.now();
            match event {
                VoiceEvent::Saying(text) => {
                    if matches!(self.state, State::Speaking | State::Thinking) {
                        self.send(SessionEvent::Saying { text });
                    }
                }
                VoiceEvent::Finished => {
                    if self.fillers_pending > 0 {
                        self.fillers_pending -= 1;
                    } else {
                        self.replies_pending = self.replies_pending.saturating_sub(1);
                        self.replies_done += 1;
                    }
                }
                // Comes just before the `Finished` of the reply it is about.
                VoiceEvent::Cut => {
                    if self.fillers_pending == 0
                        && self.question_reply == Some(self.replies_done + 1)
                    {
                        self.question_cut = true;
                    }
                }
                VoiceEvent::Stopped => {}
                VoiceEvent::Trouble(message) => {
                    self.send(SessionEvent::Trouble { message });
                    match self.state {
                        State::Speaking => {
                            self.stop_speech();
                            self.enter(State::Idle);
                        }
                        // Only a filler can be at the voice: the reply may still come, and
                        // a fresh speaker is opened for it. What the voice sent before `stop`
                        // returned (another filler's failure, its `Stopped`) is this same
                        // failure: dropped, so it is reported once.
                        State::Thinking => {
                            self.voice.stop();
                            self.fillers_pending = 0;
                            while self.voice_events.try_recv().is_ok() {}
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// How long the sound queued so far takes to leave the device, as the speaker reports it,
    /// at most `MAX_DEVICE_DELAY`. `None` when it cannot tell.
    fn device_delay(&self) -> Option<Duration> {
        self.sink.still_audible().map(|d| d.min(MAX_DEVICE_DELAY))
    }

    /// Every reply has been spoken to its end.
    fn speech_done(&self) -> bool {
        self.replies_pending == 0 && self.fillers_pending == 0 && !self.reply_open
    }

    fn timers(&mut self) {
        let now = self.clock.now();
        self.retry_mic(now);
        match self.state {
            State::Listening | State::Asking => {
                if self.tone_due.is_some_and(|at| now >= at) {
                    // A pause after "Hey Dot": the person waits to be acknowledged.
                    self.tone_due = None;
                    if !self.in_speech {
                        self.play_tone();
                    }
                }
                if !self.in_speech
                    && now.saturating_duration_since(self.quiet_since) >= self.listen_limit
                {
                    let silent = self.frames_heard >= SILENT_FRAMES && !self.any_sound;
                    let unanswered = self.state == State::Asking;
                    self.enter(State::Idle);
                    if silent {
                        self.trouble(SILENT_MIC);
                        self.owed = Some(SILENT_MIC.into());
                    } else if unanswered {
                        // The app drops what waited on the answer when the session rests.
                        self.owed = Some(NO_ANSWER.into());
                    }
                }
            }
            State::Thinking => {
                if now.saturating_duration_since(self.thinking_since) >= THINK_LIMIT {
                    self.trouble(NO_REPLY);
                    self.stop_speech();
                    self.enter(State::Idle);
                }
            }
            State::Speaking => {
                if now.saturating_duration_since(self.voice_heard_at) >= VOICE_LIMIT {
                    // The voice thread died, or the sink never goes quiet.
                    self.trouble(VOICE_STUCK);
                    self.stop_speech();
                    self.enter(State::Idle);
                    return;
                }
                if !self.speech_done() {
                    let voice = self.sink.level();
                    self.level(0.0, voice, now);
                    return;
                }
                if self.reopen_at.is_none() && self.sink.idle() {
                    // The queue is empty, but the sound may not have left the device yet.
                    let tail = Duration::from_millis(u64::from(self.settings.reopen_after_ms));
                    let still = self.device_delay().unwrap_or(UNKNOWN_DELAY);
                    self.reopen_at = Some(now + still + tail);
                }
                if self.reopen_at.is_some_and(|at| now >= at) {
                    self.after_speech();
                }
            }
            State::Idle => {
                // Said only when a speaker works; a broken one was reported on its own.
                if let Some(line) = self.owed.take() {
                    if !self.sink.failed() {
                        self.say_alone(&line, After::Rest);
                    }
                }
            }
            State::Off => {}
        }
    }

    /// Speech is over (and its tail has passed): clear the ears and go where the reply said.
    fn after_speech(&mut self) {
        self.reopen_at = None;
        // Frames captured before this moment hold the tail of Dot's own voice.
        if !self.discard_backlog() {
            return;
        }
        self.ears.reset();
        self.short_ears.reset();
        if let Some(spotter) = &mut self.wake {
            spotter.reset();
        }
        let cut = std::mem::take(&mut self.question_cut);
        self.question_reply = None;
        match self.then {
            After::Rest => self.enter(State::Idle),
            // Rests quietly after `MAX_FOLLOW_UPS` in a row.
            After::FollowUp if self.follow_ups >= MAX_FOLLOW_UPS => self.enter(State::Idle),
            After::FollowUp => {
                self.follow_ups += 1;
                let limit = Duration::from_secs(u64::from(self.settings.follow_up_secs));
                self.listen(limit);
            }
            // A question not said in full is not asked: a yes must answer what was heard.
            After::Ask if cut => {
                self.trouble(QUESTION_CUT);
                self.enter(State::Idle);
            }
            After::Ask => self.ask(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Mic, MicFrame};
    use crate::ears::{EarSettings, Ears};
    use crate::engine::{
        Clip, EngineError, Recognizer, Speaker, SpeechDetector, WakeSpotter, WakeWord, FRAME,
    };
    use crate::voice::{Sink, Voice};
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    /// Long enough that only something broken runs out of it.
    const WAIT: Duration = Duration::from_secs(5);
    /// What a fed frame holds: the detector calls positive frames speech, and the spotter
    /// hears the two wake values.
    const SPEECH: f32 = 0.5;
    const QUIET: f32 = -0.001;
    const HEY_DOT: f32 = 2.0;
    const DOT_STOP: f32 = 3.0;
    /// "Hey Dot" spotted late: this frame (speech to the detector) and the two before it came
    /// after the wake phrase.
    const HEY_DOT_LATE: f32 = 4.0;

    // ---- fakes ---------------------------------------------------------------------------

    struct FakeClock {
        base: Instant,
        offset: Mutex<Duration>,
    }

    impl FakeClock {
        fn advance(&self, ms: u64) {
            *self.offset.lock().unwrap() += Duration::from_millis(ms);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + *self.offset.lock().unwrap()
        }
    }

    type Gate = (Arc<Barrier>, Arc<Barrier>);

    struct Detector {
        seen: Arc<AtomicUsize>,
        /// Holds the first call: it reaches `entered`, then waits at `release`.
        gate: Option<Gate>,
    }

    impl SpeechDetector for Detector {
        fn is_speech(&mut self, frame16k: &[f32]) -> bool {
            if let Some((entered, release)) = self.gate.take() {
                entered.wait();
                release.wait();
            }
            self.seen.fetch_add(1, Ordering::SeqCst);
            frame16k[0] > 0.0
        }
        fn reset(&mut self) {}
    }

    struct FakeRecognizer {
        text: &'static str,
        calls: AtomicUsize,
        /// The length, in samples, of every clip given.
        lens: Mutex<Vec<usize>>,
        panics: AtomicBool,
        /// Holds the first call: it reaches `entered`, then waits at `release`.
        gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
        /// From this call on (1-based), nothing is heard.
        empty_from: usize,
    }

    impl FakeRecognizer {
        fn new(text: &'static str) -> Arc<Self> {
            Arc::new(Self {
                text,
                calls: AtomicUsize::new(0),
                lens: Mutex::default(),
                panics: AtomicBool::new(false),
                gate: None,
                empty_from: usize::MAX,
            })
        }

        /// "hello" for its first `n - 1` clips, then nothing heard.
        fn empty_from(n: usize) -> Arc<Self> {
            Arc::new(Self {
                empty_from: n,
                ..Arc::into_inner(Self::new("hello")).unwrap()
            })
        }
    }

    impl Recognizer for FakeRecognizer {
        fn transcribe(&self, clip: &Clip) -> Result<String, EngineError> {
            self.lens.lock().unwrap().push(clip.samples.len());
            if self.panics.load(Ordering::SeqCst) {
                panic!("the recogniser broke");
            }
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 1 {
                if let Some((entered, release)) = &self.gate {
                    entered.wait();
                    release.wait();
                }
            }
            if call >= self.empty_from {
                return Ok(String::new());
            }
            Ok(self.text.to_string())
        }
    }

    #[derive(Default)]
    struct FakeSpeaker {
        said: Mutex<Vec<String>>,
        voices: Mutex<Vec<(u8, f32)>>,
    }

    impl FakeSpeaker {
        fn said(&self) -> Vec<String> {
            self.said.lock().unwrap().clone()
        }
    }

    impl Speaker for FakeSpeaker {
        fn speak(&self, sentence: &str, voice: u8, speed: f32) -> Result<Clip, EngineError> {
            self.said.lock().unwrap().push(sentence.to_string());
            self.voices.lock().unwrap().push((voice, speed));
            Ok(Clip {
                samples: vec![0.1; 8],
                rate: 16_000,
            })
        }
        fn voices(&self) -> u8 {
            1
        }
    }

    struct FakeSink {
        idle: AtomicBool,
        level: AtomicU32,
        stops: AtomicUsize,
        plays: AtomicUsize,
        failed: AtomicBool,
        /// The next clip played keeps the sink sounding until `sounding(false)`.
        hold_next: AtomicBool,
        clips: Mutex<Vec<Clip>>,
        /// What `still_audible` says, in ms; `u32::MAX` when it cannot tell. 0 by default.
        delay_ms: AtomicU32,
    }

    impl FakeSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                idle: AtomicBool::new(true),
                level: AtomicU32::new(0f32.to_bits()),
                stops: AtomicUsize::new(0),
                plays: AtomicUsize::new(0),
                failed: AtomicBool::new(false),
                hold_next: AtomicBool::new(false),
                clips: Mutex::default(),
                delay_ms: AtomicU32::new(0),
            })
        }
        /// How many times the listening tone was played.
        fn tones(&self) -> usize {
            let tone = crate::tone::listen_tone(crate::tone::TONE_RATE);
            self.clips
                .lock()
                .unwrap()
                .iter()
                .filter(|c| **c == tone)
                .count()
        }
        fn sounding(&self, on: bool) {
            self.idle.store(!on, Ordering::SeqCst);
        }
        fn stops(&self) -> usize {
            self.stops.load(Ordering::SeqCst)
        }
        fn plays(&self) -> usize {
            self.plays.load(Ordering::SeqCst)
        }
        /// The device went away: what was queued is gone and nothing more is taken.
        fn fail(&self) {
            self.failed.store(true, Ordering::SeqCst);
            self.idle.store(true, Ordering::SeqCst);
        }
    }

    impl Sink for FakeSink {
        fn play(&self, clip: Clip) -> bool {
            if self.failed.load(Ordering::SeqCst) {
                return false;
            }
            self.plays.fetch_add(1, Ordering::SeqCst);
            self.clips.lock().unwrap().push(clip);
            if self.hold_next.swap(false, Ordering::SeqCst) {
                self.idle.store(false, Ordering::SeqCst);
            }
            true
        }
        fn failed(&self) -> bool {
            self.failed.load(Ordering::SeqCst)
        }
        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.idle.store(true, Ordering::SeqCst);
        }
        fn idle(&self) -> bool {
            self.idle.load(Ordering::SeqCst)
        }
        fn still_audible(&self) -> Option<Duration> {
            let ms = self.delay_ms.load(Ordering::SeqCst);
            (ms != u32::MAX).then(|| Duration::from_millis(u64::from(ms)))
        }
        fn level(&self) -> f32 {
            f32::from_bits(self.level.load(Ordering::SeqCst))
        }
    }

    struct Spotter {
        after: usize,
    }

    impl WakeSpotter for Spotter {
        fn heard(&mut self, frame16k: &[f32]) -> Option<WakeWord> {
            self.after = 0;
            if frame16k[0] == HEY_DOT {
                Some(WakeWord::HeyDot)
            } else if frame16k[0] == HEY_DOT_LATE {
                // Heard three frames late: this frame and the two before came after the word.
                self.after = 3 * FRAME;
                Some(WakeWord::HeyDot)
            } else if frame16k[0] == DOT_STOP {
                Some(WakeWord::DotStop)
            } else {
                None
            }
        }
        fn reset(&mut self) {}
        fn after_word(&self) -> usize {
            self.after
        }
    }

    /// The test's side of the microphone: it feeds frames into whichever stream is open.
    #[derive(Default)]
    struct MicFeed {
        stream: Mutex<Option<(usize, SyncSender<MicFrame>)>>,
        opens: AtomicUsize,
        fail: AtomicBool,
    }

    impl MicFeed {
        fn open_mic(self: &Arc<Self>) -> Result<Box<dyn Mic>, EngineError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(EngineError::Failed("no microphone was found".into()));
            }
            let id = self.opens.fetch_add(1, Ordering::SeqCst) + 1;
            let (tx, rx) = sync_channel(4096);
            *self.stream.lock().unwrap() = Some((id, tx));
            Ok(Box::new(FakeMic {
                rx: Some(rx),
                feed: self.clone(),
                id,
            }))
        }

        fn is_open(&self) -> bool {
            self.stream.lock().unwrap().is_some()
        }

        fn opens(&self) -> usize {
            self.opens.load(Ordering::SeqCst)
        }

        fn send(&self, value: f32, frames: usize) {
            let stream = self.stream.lock().unwrap();
            let (_, tx) = stream.as_ref().expect("the microphone is open");
            for _ in 0..frames {
                let _ = tx.send(MicFrame::Audio(vec![value; FRAME]));
            }
        }

        fn lost(&self) {
            let stream = self.stream.lock().unwrap();
            let (_, tx) = stream.as_ref().expect("the microphone is open");
            let _ = tx.send(MicFrame::Lost("unplugged".into()));
        }
    }

    struct FakeMic {
        rx: Option<Receiver<MicFrame>>,
        feed: Arc<MicFeed>,
        id: usize,
    }

    impl Mic for FakeMic {
        fn frames(&mut self) -> Receiver<MicFrame> {
            self.rx.take().unwrap_or_else(|| sync_channel(1).1)
        }
    }

    impl Drop for FakeMic {
        fn drop(&mut self) {
            let mut stream = self.feed.stream.lock().unwrap();
            if stream.as_ref().map(|(id, _)| *id) == Some(self.id) {
                *stream = None;
            }
        }
    }

    // ---- the rig -------------------------------------------------------------------------

    fn test_ears() -> EarSettings {
        // Two frames of quiet end a sentence, two of speech make one, and a preview comes
        // every two speech frames.
        EarSettings {
            end_quiet_ms: 64,
            min_speech_ms: 64,
            pre_roll_ms: 0,
            partial_every_ms: 64,
            max_clip_ms: 30_000,
        }
    }

    struct Opts {
        wake: bool,
        talk: bool,
        /// Off unless a test is about the tone: the others listen at once.
        tone: bool,
        ears: EarSettings,
        recognizer: Option<Arc<FakeRecognizer>>,
        /// Holds the sentence ears' first detector call.
        detector_gate: Option<Gate>,
    }

    impl Default for Opts {
        fn default() -> Self {
            Self {
                wake: false,
                talk: false,
                tone: false,
                ears: test_ears(),
                recognizer: None,
                detector_gate: None,
            }
        }
    }

    struct Rig {
        session: Session,
        events: UnboundedReceiver<SessionEvent>,
        runtime: Runtime,
        clock: Arc<FakeClock>,
        mic: Arc<MicFeed>,
        seen: Arc<AtomicUsize>,
        recognizer: Arc<FakeRecognizer>,
        speaker: Arc<FakeSpeaker>,
        sink: Arc<FakeSink>,
        /// Sinks opened after `sink` failed, in order; none opens while `sink_fails` is set.
        reopened: Arc<Mutex<Vec<Arc<FakeSink>>>>,
        sink_fails: Arc<AtomicBool>,
        wake: bool,
    }

    fn rig(opts: Opts) -> Rig {
        let clock = Arc::new(FakeClock {
            base: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
        });
        let seen = Arc::new(AtomicUsize::new(0));
        let recognizer = opts
            .recognizer
            .unwrap_or_else(|| FakeRecognizer::new("hello"));
        let ears = Ears::new(
            Box::new(Detector {
                seen: seen.clone(),
                gate: opts.detector_gate,
            }),
            recognizer.clone(),
            opts.ears,
        );
        let short_ears = Ears::new(
            Box::new(Detector {
                seen: seen.clone(),
                gate: None,
            }),
            FakeRecognizer::new("yes"),
            opts.ears,
        );
        let speaker = Arc::new(FakeSpeaker::default());
        let sink = FakeSink::new();
        let (voice_tx, voice_events) = unbounded_channel();
        let voice = Voice::new(speaker.clone(), sink.clone(), voice_tx);
        let mic = Arc::new(MicFeed::default());
        let feed = mic.clone();
        let reopened: Arc<Mutex<Vec<Arc<FakeSink>>>> = Arc::default();
        let sink_fails = Arc::new(AtomicBool::new(false));
        let (opened, fails) = (reopened.clone(), sink_fails.clone());
        let parts = Parts {
            ears,
            short_ears,
            voice,
            voice_events,
            sink: sink.clone(),
            open_sink: Box::new(move || {
                if fails.load(Ordering::SeqCst) {
                    return Err(EngineError::Failed("no speaker was found".into()));
                }
                let fresh = FakeSink::new();
                opened.lock().unwrap().push(fresh.clone());
                Ok(fresh as Arc<dyn Sink>)
            }),
            wake: Some(Box::new(Spotter { after: 0 })),
            open_mic: Box::new(move || feed.open_mic()),
        };
        let settings = SessionSettings {
            wake_phrase: opts.wake,
            interrupt_by_talking: opts.talk,
            tone: opts.tone,
            ..SessionSettings::default()
        };
        let (tx, events) = unbounded_channel();
        let session = Session::start_with_clock(parts, settings, tx, clock.clone());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        Rig {
            session,
            events,
            runtime,
            clock,
            mic,
            seen,
            recognizer,
            speaker,
            sink,
            reopened,
            sink_fails,
            wake: opts.wake,
        }
    }

    /// The next event of any kind; a timeout or a closed channel means something is broken.
    fn raw_event(runtime: &Runtime, events: &mut UnboundedReceiver<SessionEvent>) -> SessionEvent {
        runtime
            .block_on(async { tokio::time::timeout(WAIT, events.recv()).await })
            .expect("no event arrived in time")
            .expect("the event channel closed")
    }

    /// Waits for the worker to drop its end of the event channel.
    fn ends(runtime: &Runtime, events: &mut UnboundedReceiver<SessionEvent>) -> bool {
        runtime
            .block_on(async {
                tokio::time::timeout(WAIT, async { while events.recv().await.is_some() {} }).await
            })
            .is_ok()
    }

    fn state(state: State, mic_open: bool) -> SessionEvent {
        SessionEvent::State { state, mic_open }
    }

    impl Rig {
        fn raw(&mut self) -> SessionEvent {
            raw_event(&self.runtime, &mut self.events)
        }

        /// The next event that is not a `Level`.
        fn next(&mut self) -> SessionEvent {
            loop {
                let event = self.raw();
                if !matches!(event, SessionEvent::Level { .. }) {
                    return event;
                }
            }
        }

        fn expect_state(&mut self, s: State, mic_open: bool) {
            assert_eq!(self.next(), state(s, mic_open));
        }

        fn expect_saying(&mut self, text: &str) {
            assert_eq!(
                self.next(),
                SessionEvent::Saying {
                    text: text.to_string()
                }
            );
        }

        /// Waits until the worker has handled everything sent so far.
        fn sync(&self) -> Probe {
            self.session.probe()
        }

        /// After everything sent so far is handled, no event but `Level` is waiting.
        fn nothing_more(&mut self) {
            self.sync();
            while let Ok(event) = self.events.try_recv() {
                assert!(
                    matches!(event, SessionEvent::Level { .. }),
                    "unexpected {event:?}"
                );
            }
        }

        fn sentence(&self) {
            self.mic.send(SPEECH, 3);
            self.mic.send(QUIET, 2);
        }

        /// The person heard from a fresh `open`: returns the turn of the `Heard`.
        fn hear(&mut self) -> u64 {
            self.session.open();
            self.expect_state(State::Listening, true);
            self.hear_sentence()
        }

        /// A sentence while already listening.
        fn hear_sentence(&mut self) -> u64 {
            self.sentence();
            self.hear_sentence_already_said()
        }

        /// The events of a sentence already fed.
        fn hear_sentence_already_said(&mut self) -> u64 {
            assert_eq!(
                self.next(),
                SessionEvent::Hearing {
                    text: "hello".into()
                }
            );
            let turn = match self.next() {
                SessionEvent::Heard { text, turn } => {
                    assert_eq!(text, "hello");
                    turn
                }
                other => panic!("expected Heard, got {other:?}"),
            };
            self.expect_state(State::Thinking, self.wake);
            turn
        }

        /// Waits until the reply has been spoken and the sink went quiet.
        fn until_speech_done(&self) {
            let deadline = Instant::now() + WAIT;
            while !self.sync().speech_done {
                assert!(Instant::now() < deadline, "the speech never finished");
                std::thread::yield_now();
            }
        }

        /// A line said with no turn while resting (a trouble, a question not answered): it
        /// is spoken, and the session rests again.
        fn said_alone(&mut self, text: &str, mic_open: bool) {
            self.expect_state(State::Speaking, mic_open);
            // one `Saying` per sentence
            let mut said: Vec<String> = Vec::new();
            while said.join(" ").len() < text.len() {
                match self.next() {
                    SessionEvent::Saying { text } => said.push(text),
                    other => panic!("expected Saying, got {other:?}"),
                }
            }
            assert_eq!(said.join(" "), text);
            self.until_speech_done();
            self.clock.advance(250);
            self.expect_state(State::Idle, mic_open);
        }

        /// Lets the 250 ms tail after an interruption pass, and the worker notice.
        fn past_tail(&self) {
            self.clock.advance(250);
            self.sync();
        }

        fn seen(&self) -> usize {
            self.seen.load(Ordering::SeqCst)
        }
    }

    // ---- rule 1: Idle ----------------------------------------------------------------------

    #[test]
    fn mic_is_closed_in_idle_without_the_wake_phrase() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.nothing_more();
        assert_eq!(r.mic.opens(), 0);
        r.session.open();
        r.expect_state(State::Listening, true);
        assert_eq!(r.mic.opens(), 1);
    }

    #[test]
    fn idle_with_the_wake_phrase_feeds_only_the_spotter_and_hey_dot_wakes() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.mic.send(SPEECH, 5);
        r.nothing_more();
        assert_eq!(r.seen(), 0, "idle frames reached the ears");
        r.mic.send(HEY_DOT, 1);
        assert_eq!(r.next(), SessionEvent::Woke);
        r.expect_state(State::Listening, true);
        assert_eq!(r.mic.opens(), 1, "the same stream is kept");
    }

    #[test]
    fn dot_stop_in_idle_and_hey_dot_while_listening_do_nothing() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.mic.send(DOT_STOP, 1);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Idle);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.send(HEY_DOT, 1);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Listening);
        // "Dot, stop" while listening is a close.
        r.mic.send(DOT_STOP, 1);
        r.expect_state(State::Idle, true);
    }

    // ---- the listening tone ------------------------------------------------------------------

    fn toned(wake: bool) -> Rig {
        rig(Opts {
            wake,
            tone: true,
            ..Opts::default()
        })
    }

    /// The tone is playing (and holds the sink); the ears hear nothing, however loud.
    fn deaf_during_the_tone(r: &mut Rig) {
        r.sync(); // `Listening` is published just before the tone is played
        assert_eq!(r.sink.tones(), 1);
        let seen = r.seen();
        r.sentence();
        r.mic.send(SPEECH, 6);
        r.nothing_more();
        assert_eq!(r.seen(), seen, "the ears heard the tone");
    }

    #[test]
    fn tone_on_open_then_ears_after_it() {
        let mut r = toned(false);
        r.expect_state(State::Idle, false);
        r.sink.hold_next.store(true, Ordering::SeqCst);
        r.session.open();
        r.expect_state(State::Listening, true); // shown at once
        deaf_during_the_tone(&mut r);
        // However long it takes to sound, the ears wait for the sink to go quiet.
        r.clock.advance(1_000);
        r.mic.send(SPEECH, 6);
        r.nothing_more();
        assert_eq!(r.seen(), 0);
        // The tone has finished: a short tail after it is waited out too.
        r.sink.sounding(false);
        r.sync();
        r.clock.advance(59);
        r.mic.send(SPEECH, 6);
        r.nothing_more();
        assert_eq!(r.seen(), 0, "a tail frame reached the ears");
        r.clock.advance(1);
        r.sync();
        // Then the ears hear, starting clean.
        r.hear_sentence();
        assert_eq!(r.sink.tones(), 1);
    }

    #[test]
    fn a_tone_that_never_ends_is_stopped_and_the_ears_open() {
        let mut r = toned(false);
        r.expect_state(State::Idle, false);
        r.sink.hold_next.store(true, Ordering::SeqCst);
        r.session.open();
        r.expect_state(State::Listening, true);
        deaf_during_the_tone(&mut r);
        let stops = r.sink.stops();
        r.clock.advance(2_000);
        r.sync();
        assert!(r.sink.stops() > stops, "the stuck tone was not stopped");
        r.past_tail();
        r.hear_sentence();
    }

    #[test]
    fn the_listen_limit_counts_from_when_the_ears_open() {
        let mut r = toned(false);
        r.expect_state(State::Idle, false);
        r.sink.hold_next.store(true, Ordering::SeqCst);
        r.session.open();
        r.expect_state(State::Listening, true);
        deaf_during_the_tone(&mut r);
        // A slow tone: a second before the sink is quiet, then the short tail.
        r.clock.advance(1_000);
        r.sync();
        r.sink.sounding(false);
        r.sync();
        r.clock.advance(60);
        r.sync();
        // Eight quiet seconds from here, not from the `open`.
        r.clock.advance(7_999);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Listening);
        r.clock.advance(1);
        r.expect_state(State::Idle, false);
    }

    /// "Hey Dot" fed on a toned rig with the wake phrase on.
    fn woken() -> Rig {
        let mut r = toned(true);
        r.expect_state(State::Idle, true);
        r.mic.send(HEY_DOT, 1);
        assert_eq!(r.next(), SessionEvent::Woke);
        r.expect_state(State::Listening, true);
        r
    }

    #[test]
    fn speech_right_after_the_wake_phrase_is_heard_whole() {
        let mut r = toned(true);
        r.expect_state(State::Idle, true);
        // "Hey Dot, read me the first message" in one breath: no frame between.
        r.mic.send(HEY_DOT, 1);
        r.mic.send(SPEECH, 3);
        r.mic.send(QUIET, 2);
        assert_eq!(r.next(), SessionEvent::Woke);
        r.expect_state(State::Listening, true);
        r.hear_sentence_already_said();
        // Every speech frame reached the clip, and no tone was played for this wake.
        let lens = r.recognizer.lens.lock().unwrap().clone();
        assert_eq!(lens.last().copied(), Some(5 * FRAME), "clips: {lens:?}");
        r.clock.advance(1_000);
        r.nothing_more();
        assert_eq!(r.sink.tones(), 0);
    }

    /// Words that follow "Hey Dot" at once reach the recogniser after a moment of silence, as
    /// the pre-roll gives any other sentence: without it the real recogniser heard nothing
    /// for about one such sentence in twenty.
    #[test]
    fn speech_right_after_the_wake_phrase_comes_after_a_lead_of_silence() {
        let mut r = rig(Opts {
            wake: true,
            ears: EarSettings {
                pre_roll_ms: 64,
                ..test_ears()
            },
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.mic.send(HEY_DOT, 1);
        r.mic.send(SPEECH, 3);
        r.mic.send(QUIET, 2);
        assert_eq!(r.next(), SessionEvent::Woke);
        r.expect_state(State::Listening, true);
        r.hear_sentence_already_said();
        // two frames of silence, then the speech and the quiet that ended it
        let lens = r.recognizer.lens.lock().unwrap().clone();
        assert_eq!(lens.last().copied(), Some(7 * FRAME), "clips: {lens:?}");
    }

    #[test]
    fn words_said_before_the_spotter_answers_are_kept() {
        let mut r = toned(true);
        r.expect_state(State::Idle, true);
        // The person goes on talking; the spotter answers on the third frame after the word.
        r.mic.send(QUIET, 4);
        r.mic.send(SPEECH, 2);
        r.mic.send(HEY_DOT_LATE, 1);
        r.mic.send(QUIET, 2);
        assert_eq!(r.next(), SessionEvent::Woke);
        r.expect_state(State::Listening, true);
        r.hear_sentence_already_said();
        // The three frames after the word, then the two quiet ones that ended the sentence.
        let lens = r.recognizer.lens.lock().unwrap().clone();
        assert_eq!(lens.last().copied(), Some(5 * FRAME), "clips: {lens:?}");
        assert_eq!(r.sink.tones(), 0);
    }

    #[test]
    fn a_pause_after_the_wake_phrase_gets_the_tone() {
        let mut r = woken();
        // The ears are open at once: quiet frames reach them.
        let seen = r.seen();
        r.mic.send(QUIET, 2);
        r.nothing_more();
        assert_eq!(r.seen(), seen + 2);
        r.clock.advance(299);
        r.nothing_more();
        assert_eq!(r.sink.tones(), 0, "the tone came before the pause");
        // A pause of 300 ms: the tone answers it, and the ears are deaf while it sounds.
        r.sink.hold_next.store(true, Ordering::SeqCst);
        r.clock.advance(1);
        deaf_during_the_tone(&mut r);
        r.sink.sounding(false);
        r.sync();
        // Then only the short tail.
        r.clock.advance(59);
        let seen = r.seen();
        r.mic.send(SPEECH, 6);
        r.nothing_more();
        assert_eq!(r.seen(), seen, "a tail frame reached the ears");
        r.clock.advance(1);
        r.sync();
        r.hear_sentence();
        assert_eq!(r.sink.tones(), 1);
    }

    #[test]
    fn no_tone_when_the_person_keeps_talking() {
        let mut r = woken();
        // Speech begins within the 300 ms (after a short breath).
        r.mic.send(QUIET, 1);
        r.clock.advance(200);
        r.mic.send(SPEECH, 3);
        assert_eq!(
            r.next(),
            SessionEvent::Hearing {
                text: "hello".into()
            }
        );
        r.clock.advance(500);
        r.nothing_more();
        assert_eq!(r.sink.tones(), 0);
        r.mic.send(QUIET, 2);
        assert!(matches!(r.next(), SessionEvent::Heard { .. }));
        assert_eq!(r.sink.tones(), 0);
    }

    #[test]
    fn tone_off_opens_ears_at_once() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.hear();
        assert_eq!(r.sink.tones(), 0);
        assert_eq!(r.sink.plays(), 0);
    }

    #[test]
    fn failed_sink_skips_the_tone() {
        let mut r = toned(false);
        r.expect_state(State::Idle, false);
        r.sink.fail();
        r.sink_fails.store(true, Ordering::SeqCst);
        r.hear();
        assert_eq!(r.sink.tones(), 0);
        // No speaker was opened (or reported) just for the tone.
        assert!(r.reopened.lock().unwrap().is_empty());
    }

    /// A toned rig past its first tone, with the turn of a sentence heard after it.
    fn heard_after_the_tone(r: &mut Rig) -> u64 {
        r.expect_state(State::Idle, r.wake);
        r.sink.hold_next.store(true, Ordering::SeqCst);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.sync(); // the tone is played
        r.sink.sounding(false);
        r.sync();
        r.past_tail();
        let turn = r.hear_sentence();
        assert_eq!(r.sink.tones(), 1);
        turn
    }

    #[test]
    fn no_tone_on_follow_up() {
        let mut r = toned(false);
        let turn = heard_after_the_tone(&mut r);
        r.session.say(turn, "Sure.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Sure.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
        // The ears hear at once, and no tone was played.
        let turn = r.hear_sentence();
        assert_eq!(r.sink.tones(), 1);
        // Nor when a reply asks.
        r.session.say(turn, "Shall I?", After::Ask);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Shall I?");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
        assert_eq!(r.sink.tones(), 1);
    }

    #[test]
    fn no_tone_after_interrupt() {
        let mut r = toned(true);
        let turn = heard_after_the_tone(&mut r);
        // Interrupted while thinking.
        r.session.interrupt();
        r.expect_state(State::Listening, true);
        assert!(r.hear_sentence() > turn);
        // Interrupted while speaking: by the call, by `open`, and by "Dot, stop".
        for how in 0..3 {
            let turn = r.session.turn();
            r.sink.sounding(true);
            r.session.say(turn, "A long answer.", After::Rest);
            r.expect_state(State::Speaking, true);
            r.expect_saying("A long answer.");
            match how {
                0 => r.session.interrupt(),
                1 => r.session.open(),
                _ => r.mic.send(DOT_STOP, 1),
            }
            r.expect_state(State::Listening, true);
            r.past_tail();
            r.hear_sentence();
        }
        // A click answers without listening: no tone either.
        r.session.answered("Yes");
        assert!(matches!(r.next(), SessionEvent::Heard { .. }));
        r.nothing_more();
        assert_eq!(r.sink.tones(), 1);
    }

    // ---- rule 2: Listening -----------------------------------------------------------------

    #[test]
    fn listening_turns_partials_into_hearing_and_a_final_into_heard_then_thinks() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        assert_eq!(turn, 1);
        assert_eq!(r.session.turn(), 1);
        assert_eq!(r.sync().state, State::Thinking);
        assert!(
            !r.mic.is_open(),
            "no wake phrase: the mic closes while thinking"
        );
    }

    #[test]
    fn a_dropped_clip_keeps_listening_and_eight_quiet_seconds_after_open_rest() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        // A clip in progress is never cut by the timer.
        r.mic.send(SPEECH, 1);
        r.sync();
        r.clock.advance(9_000);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Listening);
        // Too short to be speech: dropped, and the quiet is counted from here.
        r.mic.send(QUIET, 2);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Listening);
        r.clock.advance(7_999);
        r.nothing_more();
        r.clock.advance(1);
        r.expect_state(State::Idle, false);
    }

    #[test]
    fn a_follow_up_listens_for_follow_up_secs_then_rests() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.say(turn, "Sure.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Sure.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
        r.clock.advance(5_999);
        r.nothing_more();
        r.clock.advance(1);
        r.expect_state(State::Idle, false);
    }

    // ---- rule 3: Thinking ------------------------------------------------------------------

    #[test]
    fn thinking_keeps_the_mic_only_for_the_wake_phrase() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.hear();
        let seen = r.seen();
        r.mic.send(SPEECH, 6);
        r.mic.send(QUIET, 3);
        r.nothing_more();
        assert_eq!(r.seen(), seen, "thinking frames reached the ears");
        r.mic.send(HEY_DOT, 1);
        r.expect_state(State::Listening, true);
        r.hear_sentence();
        r.mic.send(DOT_STOP, 1);
        r.expect_state(State::Listening, true);
    }

    // ---- rule 4: Speaking ------------------------------------------------------------------

    #[test]
    fn speaking_hears_only_the_wake_phrase() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say(turn, "Hello there.", After::Rest);
        r.expect_state(State::Speaking, true);
        r.expect_saying("Hello there.");
        let seen = r.seen();
        r.mic.send(SPEECH, 10);
        r.nothing_more();
        assert_eq!(r.seen(), seen);
        r.mic.send(DOT_STOP, 1);
        r.expect_state(State::Listening, true);
        assert!(r.sink.stops() >= 1);
    }

    #[test]
    fn words_spoken_by_dot_are_never_transcribed() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        let turn = r.hear();
        let calls = r.recognizer.calls.load(Ordering::SeqCst);
        r.sink.sounding(true);
        r.session.say(turn, "I am talking now.", After::Rest);
        r.expect_state(State::Speaking, true);
        r.expect_saying("I am talking now.");
        r.recognizer.panics.store(true, Ordering::SeqCst);
        r.mic.send(SPEECH, 20);
        r.mic.send(QUIET, 5);
        r.mic.send(SPEECH, 20);
        r.nothing_more();
        r.recognizer.panics.store(false, Ordering::SeqCst);
        assert_eq!(r.session.state(), State::Speaking);
        assert_eq!(r.recognizer.calls.load(Ordering::SeqCst), calls);
    }

    #[test]
    fn talking_over_dot_interrupts_when_allowed_and_the_words_are_kept() {
        let mut r = rig(Opts {
            talk: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say(turn, "A long answer.", After::Rest);
        r.expect_state(State::Speaking, true);
        r.expect_saying("A long answer.");
        r.mic.send(SPEECH, 3);
        r.expect_state(State::Listening, true);
        assert!(r.sink.stops() >= 1);
        r.mic.send(QUIET, 2);
        // The clip began while Dot spoke and carries on: no new Started is needed.
        assert!(matches!(r.next(), SessionEvent::Hearing { .. }));
        match r.next() {
            SessionEvent::Heard { turn: t, .. } => assert_eq!(t, turn + 1),
            other => panic!("expected Heard, got {other:?}"),
        }
    }

    // ---- rule 5: after speech ----------------------------------------------------------------

    #[test]
    fn after_speech_the_mic_reopens_once_the_tail_has_passed() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.say(turn, "Done.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Done.");
        r.until_speech_done();
        r.clock.advance(249);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Speaking);
        r.clock.advance(1);
        r.expect_state(State::Listening, true);
    }

    #[test]
    fn a_rest_reply_goes_idle_and_an_ask_reply_asks() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.say(turn, "Bye.", After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Bye.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.say(turn, "Shall I?", After::Ask);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Shall I?");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
    }

    #[test]
    fn frames_in_the_tail_after_speech_never_reach_the_ears() {
        let mut r = rig(Opts {
            talk: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.say(turn, "Done.", After::FollowUp);
        r.expect_state(State::Speaking, true);
        r.expect_saying("Done.");
        r.until_speech_done();
        let seen = r.seen();
        r.mic.send(SPEECH, 3); // the room still rings with Dot's voice
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
        assert_eq!(r.seen(), seen, "a tail frame reached the ears");
        r.mic.send(QUIET, 3);
        r.nothing_more();
    }

    // ---- rule 6: Asking ----------------------------------------------------------------------

    #[test]
    fn asking_uses_the_short_answer_ears_and_rests_after_twenty_seconds() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.say(turn, "Shall I?", After::Ask);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Shall I?");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
        r.sentence();
        assert_eq!(r.next(), SessionEvent::Hearing { text: "yes".into() });
        match r.next() {
            SessionEvent::Heard { text, .. } => assert_eq!(text, "yes"),
            other => panic!("expected Heard, got {other:?}"),
        }
        r.expect_state(State::Thinking, false);
        let turn = r.session.turn();
        r.session.say(turn, "Really?", After::Ask);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Really?");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
        r.clock.advance(19_999);
        r.nothing_more();
        r.clock.advance(1);
        r.expect_state(State::Idle, false);
        // never in silence: what the lapse means is said
        r.said_alone(NO_ANSWER, false);
        r.nothing_more();
    }

    // ---- questions, the speaker's delay, follow-ups, silence ----------------------------------

    /// The words before a question may use the whole length cap: the question is a reply of
    /// its own, said whole after them, and then the session asks.
    #[test]
    fn a_question_is_said_whole_after_words_that_fill_the_cap() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        let words = "This is a sentence of words. ".repeat(20); // 580 bytes
        r.session.say_delta(turn, &words);
        r.session
            .question(turn, "Research wants to use Bash. Approve this request?");
        r.session.end_of_reply(turn, After::Ask);
        r.expect_state(State::Speaking, false);
        let mut said = Vec::new();
        loop {
            match r.next() {
                SessionEvent::Saying { text } => said.push(text),
                SessionEvent::State { .. } => panic!("left Speaking early: {said:?}"),
                other => panic!("unexpected {other:?}"),
            }
            if said.last().is_some_and(|s| s == "Approve this request?") {
                break;
            }
        }
        assert_eq!(said.len(), 22, "{said:?}");
        assert_eq!(said[20], "Research wants to use Bash.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
    }

    /// A question the speaker did not get whole (here: longer than the cap) is not asked: a
    /// yes must answer what was heard. The session says why and rests.
    #[test]
    fn a_question_not_said_in_full_is_not_asked() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        let long = format!("{}. Approve this request?", "word ".repeat(150).trim_end());
        r.session.question(turn, &long);
        r.session.end_of_reply(turn, After::Ask);
        r.expect_state(State::Speaking, false);
        r.until_speech_done();
        r.clock.advance(250);
        let mut next = r.next();
        while matches!(next, SessionEvent::Saying { .. }) {
            next = r.next();
        }
        assert_eq!(
            next,
            SessionEvent::Trouble {
                message: QUESTION_CUT.into()
            }
        );
        r.expect_state(State::Idle, false);
        // Words cut by the cap before a whole question do not stop it being asked.
        let turn = r.hear();
        r.session.say_delta(turn, &"More words here. ".repeat(50));
        r.session.question(turn, "Approve this request?");
        r.session.end_of_reply(turn, After::Ask);
        r.expect_state(State::Speaking, false);
        loop {
            if let SessionEvent::Saying { text } = r.next() {
                if text == "Approve this request?" {
                    break;
                }
            }
        }
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
    }

    /// The ears open only once the sound has left the device: the speaker's own delay, as it
    /// reports it, comes on top of `reopen_after_ms`. One that cannot tell gets a longer wait.
    /// A delay past `MAX_DEVICE_DELAY` (a clock jump, a virtual device) counts as that much.
    #[test]
    fn the_ears_wait_for_the_sound_to_leave_the_speaker() {
        for (delay, wait) in [(800, 1_050), (u32::MAX, 600), (60_000, 2_250)] {
            let mut r = rig(Opts::default());
            r.expect_state(State::Idle, false);
            r.sink.delay_ms.store(delay, Ordering::SeqCst);
            let turn = r.hear();
            r.session.say(turn, "Approve this request?", After::Ask);
            r.expect_state(State::Speaking, false);
            r.expect_saying("Approve this request?");
            r.until_speech_done();
            r.clock.advance(wait - 1);
            r.nothing_more();
            assert_eq!(r.session.state(), State::Speaking, "delay {delay}");
            r.clock.advance(1);
            r.expect_state(State::Asking, true);
        }
    }

    /// Speech in the room cannot keep Dot taking turns: after five follow-ups in a row it
    /// rests, saying nothing more. A hotkey (or click, or "Hey Dot") starts the count again.
    #[test]
    fn five_follow_ups_in_a_row_then_rest() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let mut turn = r.hear();
        for _ in 0..5 {
            r.session.say(turn, "Sure.", After::FollowUp);
            r.expect_state(State::Speaking, false);
            r.expect_saying("Sure.");
            r.until_speech_done();
            r.clock.advance(250);
            r.expect_state(State::Listening, true);
            turn = r.hear_sentence();
        }
        r.session.say(turn, "Sure.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Sure.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Idle, false);
        r.nothing_more();
        // the hotkey: follow-ups again
        let turn = r.hear();
        r.session.say(turn, "Sure.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Sure.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
    }

    /// "Hey Dot" said over Dot is a wake like any other: the follow-up count starts again.
    #[test]
    fn hey_dot_over_dot_starts_the_follow_up_count_again() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        let mut turn = r.hear();
        for _ in 0..5 {
            r.session.say(turn, "Sure.", After::FollowUp);
            r.expect_state(State::Speaking, true);
            r.expect_saying("Sure.");
            r.until_speech_done();
            r.clock.advance(250);
            r.expect_state(State::Listening, true);
            turn = r.hear_sentence();
        }
        // The sixth reply would end at rest; "Hey Dot" said over it.
        r.sink.sounding(true);
        r.session.say(turn, "Sure.", After::FollowUp);
        r.expect_state(State::Speaking, true);
        r.expect_saying("Sure.");
        r.mic.send(HEY_DOT, 1);
        r.expect_state(State::Listening, true);
        r.sink.sounding(false);
        r.past_tail();
        let turn = r.hear_sentence();
        r.session.say(turn, "Sure.", After::FollowUp);
        r.expect_state(State::Speaking, true);
        r.expect_saying("Sure.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
    }

    /// A microphone that gives only silence (blocked by the system, which then sends zeros)
    /// is reported and said, at the end of the listen.
    #[test]
    fn a_microphone_that_gives_only_silence_is_explained() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.send(0.0, 40);
        for _ in 0..8 {
            r.sync(); // a pass takes 8 frames
        }
        r.clock.advance(8_000);
        r.expect_state(State::Idle, false);
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: SILENT_MIC.into()
            }
        );
        r.said_alone(SILENT_MIC, false);
        r.nothing_more();
        // a quiet room is not silence: nothing is said
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.send(QUIET, 40);
        for _ in 0..8 {
            r.sync(); // a pass takes 8 frames
        }
        r.clock.advance(8_000);
        r.expect_state(State::Idle, false);
        r.nothing_more();
    }

    /// Words shown live, then nothing heard when the sentence ends: Dot says so (once) and
    /// listens once more, instead of going back to rest in silence.
    #[test]
    fn words_heard_live_then_lost_are_answered_and_listened_for_again() {
        let mut r = rig(Opts {
            recognizer: Some(FakeRecognizer::empty_from(2)),
            ..Opts::default()
        });
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.sentence();
        assert_eq!(
            r.next(),
            SessionEvent::Hearing {
                text: "hello".into()
            }
        );
        r.expect_state(State::Speaking, false);
        r.expect_saying(DIDNT_CATCH);
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
        // a second one in the same listen is not answered again
        r.sentence();
        r.sync();
        r.clock.advance(6_000);
        r.expect_state(State::Idle, false);
        r.nothing_more();
    }

    // ---- rule 7: stopping ----------------------------------------------------------------------

    #[test]
    fn close_interrupt_and_shutdown_stop_speech_first() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.sink.sounding(true);
        let turn = r.hear();
        r.session.say(turn, "One.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("One.");
        r.session.close();
        r.expect_state(State::Idle, false);
        assert_eq!(r.sink.stops(), 1);

        r.sink.sounding(true);
        let turn = r.hear();
        r.session.say(turn, "Two.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Two.");
        r.session.interrupt();
        r.expect_state(State::Listening, true);
        assert_eq!(r.sink.stops(), 2);

        r.sink.sounding(true);
        r.past_tail();
        let turn = r.hear_sentence();
        r.session.say(turn, "Three.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Three.");
        r.session.shutdown();
        r.expect_state(State::Off, false);
        assert_eq!(r.sink.stops(), 3);
    }

    // ---- rule 8: a broken microphone -------------------------------------------------------------

    #[test]
    fn the_mic_unplugged_while_listening_reports_trouble_and_rests() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.lost();
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Idle, false);
        assert!(!r.mic.is_open());
    }

    #[test]
    fn a_mic_that_will_not_open_reports_trouble_and_rests() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.mic.fail.store(true, Ordering::SeqCst);
        r.session.open();
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        // Listening was asked for: it is said aloud too, and Dot rests again.
        r.said_alone(MIC_WONT_OPEN, false);
        r.nothing_more();
        assert_eq!(r.session.state(), State::Idle);
        r.mic.fail.store(false, Ordering::SeqCst);
        r.session.open();
        r.expect_state(State::Listening, true);
    }

    #[test]
    fn with_the_wake_phrase_a_lost_mic_is_tried_again_every_five_seconds() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.mic.fail.store(true, Ordering::SeqCst);
        r.mic.lost();
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: MIC_LOST.into()
            }
        );
        r.expect_state(State::Idle, false);
        // Tries that fail are quiet: the loss was reported once.
        for _ in 0..3 {
            r.clock.advance(4_999);
            r.nothing_more();
            r.clock.advance(1);
            r.nothing_more();
        }
        assert_eq!(r.mic.opens(), 1);
        r.mic.fail.store(false, Ordering::SeqCst);
        r.clock.advance(4_999);
        r.nothing_more();
        r.clock.advance(1);
        r.expect_state(State::Idle, true);
        assert_eq!(r.mic.opens(), 2);
        // "Hey Dot" works again.
        r.mic.send(HEY_DOT, 1);
        assert_eq!(r.next(), SessionEvent::Woke);
        r.expect_state(State::Listening, true);
    }

    #[test]
    fn without_the_wake_phrase_a_lost_mic_is_not_tried_in_idle() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.lost();
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Idle, false);
        r.said_alone(MIC_LOST, false);
        r.clock.advance(60_000);
        r.nothing_more();
        assert_eq!(r.mic.opens(), 1);
    }

    #[test]
    fn a_mic_lost_while_speaking_lets_the_reply_finish_then_reports_once() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.speaking();
        r.mic.fail.store(true, Ordering::SeqCst);
        r.mic.lost();
        r.expect_state(State::Speaking, false);
        r.nothing_more(); // no Trouble yet, and the speech goes on
        assert_eq!(r.sink.stops(), 0);
        r.clock.advance(5_000); // a quiet try while speaking fails
        r.nothing_more();
        r.sink.sounding(false);
        r.until_speech_done();
        r.clock.advance(250);
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: "The microphone could not be opened: no microphone was found".into()
            }
        );
        r.expect_state(State::Idle, false);
        r.clock.advance(5_000);
        r.nothing_more(); // once only
        r.mic.fail.store(false, Ordering::SeqCst);
        r.clock.advance(5_000);
        r.expect_state(State::Idle, true);
    }

    #[test]
    fn a_mic_lost_while_speaking_and_back_by_the_end_reports_nothing() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.speaking();
        r.mic.lost();
        r.expect_state(State::Speaking, false);
        r.clock.advance(5_000);
        r.expect_state(State::Speaking, true); // the quiet try worked
        r.sink.sounding(false);
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Idle, true);
        r.nothing_more();
    }

    #[test]
    fn a_mic_lost_while_speaking_a_follow_up_is_tried_when_listening_starts() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say(turn, "And then?", After::FollowUp);
        r.expect_state(State::Speaking, true);
        r.expect_saying("And then?");
        r.mic.fail.store(true, Ordering::SeqCst);
        r.mic.lost();
        r.expect_state(State::Speaking, false);
        r.sink.sounding(false);
        r.until_speech_done();
        r.clock.advance(250);
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Idle, false);
        r.said_alone(MIC_WONT_OPEN, false);
        r.nothing_more();
    }

    // ---- rule 9: mic_open ------------------------------------------------------------------------

    #[test]
    fn every_state_event_tells_whether_the_mic_is_open() {
        let mut r = rig(Opts::default());
        let mut seen = Vec::new();
        let mut take = |r: &mut Rig| match r.next() {
            SessionEvent::State { state, mic_open } => {
                assert_eq!(mic_open, r.mic.is_open(), "in {state:?}");
                seen.push((state, mic_open));
            }
            other => panic!("expected State, got {other:?}"),
        };
        take(&mut r);
        r.session.open();
        take(&mut r);
        r.sentence();
        assert!(matches!(r.next(), SessionEvent::Hearing { .. }));
        assert!(matches!(r.next(), SessionEvent::Heard { .. }));
        take(&mut r);
        let turn = r.session.turn();
        r.session.say(turn, "Yes.", After::FollowUp);
        take(&mut r);
        r.expect_saying("Yes.");
        r.until_speech_done();
        r.clock.advance(250);
        take(&mut r);
        assert_eq!(
            seen,
            [
                (State::Idle, false),
                (State::Listening, true),
                (State::Thinking, false),
                (State::Speaking, false),
                (State::Listening, true),
            ]
        );
    }

    // ---- interruption and turns ------------------------------------------------------------------

    #[test]
    fn late_text_of_a_reply_cut_short_is_not_spoken_and_changes_nothing() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say_delta(turn, "First one. Sec");
        r.expect_state(State::Speaking, false);
        r.expect_saying("First one.");
        r.session.interrupt();
        r.expect_state(State::Listening, true);
        r.session.say_delta(turn, "ond half. More. ");
        r.session.end_of_reply(turn, After::FollowUp);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Listening);
        assert_eq!(r.speaker.said(), ["First one."]);
        // The voice is free for the next turn's streamed reply.
        r.past_tail();
        let next = r.hear_sentence();
        assert_eq!(next, turn + 1);
        r.session.say_delta(next, "Fresh.");
        r.session.end_of_reply(next, After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Fresh.");
    }

    #[test]
    fn a_late_reply_for_an_interrupted_turn_is_not_spoken() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let old = r.hear();
        r.session.interrupt();
        r.expect_state(State::Listening, true);
        r.session.say(old, "Old answer.", After::Rest);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Listening);
        assert!(r.speaker.said().is_empty());
        let new = r.hear_sentence();
        r.session.say(old, "Old answer.", After::Rest);
        r.session.say(new, "New answer.", After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("New answer.");
        assert_eq!(r.speaker.said(), ["New answer."]);
    }

    #[test]
    fn a_final_followed_by_a_backlog_gives_exactly_one_heard() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let recognizer = Arc::new(FakeRecognizer {
            text: "hello",
            calls: AtomicUsize::new(0),
            lens: Mutex::default(),
            panics: AtomicBool::new(false),
            gate: Some((entered.clone(), release.clone())),
            empty_from: usize::MAX,
        });
        let mut r = rig(Opts {
            wake: true,
            ears: EarSettings {
                partial_every_ms: 10_000, // no previews: the first transcription is the Final
                ..test_ears()
            },
            recognizer: Some(recognizer),
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.sentence();
        entered.wait(); // the worker is inside the Final's transcription
        r.sentence(); // queued meanwhile: speech, and a wake phrase
        r.mic.send(HEY_DOT, 1);
        r.sentence();
        release.wait();
        match r.next() {
            SessionEvent::Heard { text, turn } => assert_eq!((text.as_str(), turn), ("hello", 1)),
            other => panic!("expected Heard, got {other:?}"),
        }
        r.expect_state(State::Thinking, true);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Thinking);
        assert_eq!(r.recognizer.calls.load(Ordering::SeqCst), 1);
    }

    // ---- the other rulings ---------------------------------------------------------------------

    #[test]
    fn levels_are_throttled_and_settle_to_zero() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.send(SPEECH, 4);
        let first = loop {
            if let SessionEvent::Level { mic, voice } = r.raw() {
                break (mic, voice);
            }
        };
        assert_eq!(first, (1.0, 0.0));
        r.sync();
        while let Ok(event) = r.events.try_recv() {
            assert!(
                !matches!(event, SessionEvent::Level { .. }),
                "not throttled"
            );
        }
        r.clock.advance(34);
        r.mic.send(SPEECH, 1);
        loop {
            if let SessionEvent::Level { mic, .. } = r.raw() {
                assert_eq!(mic, 1.0);
                break;
            }
        }
        r.sync();
        while r.events.try_recv().is_ok() {}
        r.session.close();
        assert_eq!(
            r.raw(),
            SessionEvent::Level {
                mic: 0.0,
                voice: 0.0
            }
        );
        assert_eq!(r.raw(), state(State::Idle, false));

        // While speaking the level is the voice's.
        r.sink.level.store(0.5f32.to_bits(), Ordering::SeqCst);
        r.sink.sounding(true);
        let turn = r.hear();
        r.session.say(turn, "Loud.", After::Rest);
        loop {
            if let SessionEvent::Level { mic, voice } = r.raw() {
                if voice > 0.0 {
                    assert_eq!((mic, voice), (0.0, 0.5));
                    break;
                }
            }
        }
    }

    #[test]
    fn a_thinking_session_with_no_reply_gives_up_after_two_minutes() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.hear();
        r.clock.advance(119_999);
        r.nothing_more();
        r.clock.advance(1);
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: "No reply came back.".into()
            }
        );
        r.expect_state(State::Idle, false);
    }

    #[test]
    fn thinking_matters_only_after_heard_and_restarts_the_limit_once() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.thinking();
        r.nothing_more();
        assert_eq!(r.sync().state, State::Idle);
        r.hear();
        r.clock.advance(60_000);
        r.session.thinking();
        r.session.thinking();
        r.sync();
        r.clock.advance(60_000);
        r.session.thinking(); // idempotent: the limit is not pushed back again
        r.nothing_more();
        assert_eq!(r.sync().state, State::Thinking);
        r.clock.advance(60_000);
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Idle, false);
    }

    #[test]
    fn open_while_speaking_interrupts_and_open_while_listening_does_nothing() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say(turn, "Talking.", After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Talking.");
        r.session.open();
        r.expect_state(State::Listening, true);
        assert_eq!(r.sink.stops(), 1);
        r.session.open();
        r.nothing_more();
        assert_eq!(r.mic.opens(), 2);
    }

    #[test]
    fn an_empty_reply_moves_on_without_speaking() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.end_of_reply(turn, After::FollowUp);
        r.expect_state(State::Listening, true);
        assert!(r.speaker.said().is_empty());
    }

    #[test]
    fn a_notice_is_spoken_only_when_idle() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.notice("An update is ready.");
        r.expect_state(State::Speaking, false);
        r.expect_saying("An update is ready.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.session.notice("Not now.");
        r.nothing_more();
        assert_eq!(r.speaker.said(), ["An update is ready."]);
    }

    #[test]
    fn settings_take_effect_at_once() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.session.settings(SessionSettings {
            wake_phrase: false,
            ..SessionSettings::default()
        });
        r.expect_state(State::Idle, false);
        assert!(!r.mic.is_open());
        r.session.settings(SessionSettings {
            wake_phrase: true,
            ..SessionSettings::default()
        });
        r.expect_state(State::Idle, true);
    }

    #[test]
    fn shutdown_is_final_and_ends_the_worker() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.session.shutdown();
        assert_eq!(r.session.state(), State::Off);
        r.expect_state(State::Off, false);
        assert!(ends(&r.runtime, &mut r.events), "the worker did not end");
        assert!(!r.mic.is_open());
        r.session.open();
        r.session.shutdown();
        assert_eq!(r.session.state(), State::Off);
        assert_eq!(r.mic.opens(), 1);
    }

    #[test]
    fn dropping_the_last_handle_shuts_down() {
        let Rig {
            session,
            mut events,
            runtime,
            mic,
            ..
        } = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        assert_eq!(raw_event(&runtime, &mut events), state(State::Idle, true));
        let copy = session.clone();
        drop(session);
        assert_eq!(copy.probe().state, State::Idle, "a clone keeps it running");
        drop(copy);
        assert_eq!(raw_event(&runtime, &mut events), state(State::Off, false));
        assert!(ends(&runtime, &mut events));
        assert!(!mic.is_open());
    }

    #[test]
    fn a_panic_in_an_engine_turns_the_session_off_and_says_so() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.recognizer.panics.store(true, Ordering::SeqCst);
        r.sentence();
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Off, false);
        assert!(ends(&r.runtime, &mut r.events));
        assert_eq!(r.session.state(), State::Off);
        r.session.open(); // nothing to talk to any more; must not hang or panic
        assert_eq!(r.session.state(), State::Off);
    }

    // ---- fix round 1 ---------------------------------------------------------------------------

    impl Rig {
        /// From Idle (no wake phrase) to Asking after a question.
        fn asking(&mut self) {
            let turn = self.hear();
            self.session.say(turn, "Shall I?", After::Ask);
            self.expect_state(State::Speaking, self.wake);
            self.expect_saying("Shall I?");
            self.until_speech_done();
            self.clock.advance(250);
            self.expect_state(State::Asking, true);
        }

        /// From Idle with the wake phrase on to Speaking a reply that keeps sounding.
        fn speaking(&mut self) {
            let turn = self.hear();
            self.sink.sounding(true);
            self.session.say(turn, "A long answer.", After::Rest);
            self.expect_state(State::Speaking, true);
            self.expect_saying("A long answer.");
        }
    }

    #[test]
    fn a_wake_word_interruption_gives_no_speech_time_audio_to_the_ears() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.speaking();
        let seen = r.seen();
        // The stop word, and Dot's own voice queued right behind it.
        r.mic.send(DOT_STOP, 1);
        r.mic.send(SPEECH, 6);
        r.mic.send(QUIET, 3);
        r.expect_state(State::Listening, true);
        r.nothing_more();
        assert_eq!(r.seen(), seen, "speech-time audio reached the ears");
        // Once the tail has passed, the person is heard normally.
        r.past_tail();
        r.hear_sentence();
    }

    #[test]
    fn a_command_interruption_gives_no_speech_time_audio_to_the_ears() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.speaking();
        let seen = r.seen();
        r.mic.send(SPEECH, 6);
        r.mic.send(QUIET, 3);
        r.session.interrupt();
        r.expect_state(State::Listening, true); // shown at once
        r.mic.send(SPEECH, 6); // still inside the tail
        r.mic.send(QUIET, 3);
        r.nothing_more();
        assert_eq!(r.seen(), seen, "speech-time audio reached the ears");
        r.past_tail();
        r.hear_sentence();
    }

    #[test]
    fn set_voice_applies_to_the_next_sentence() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.set_voice(2, 1.2);
        r.session.notice("Hello.");
        r.expect_state(State::Speaking, false);
        r.expect_saying("Hello.");
        assert_eq!(r.speaker.voices.lock().unwrap().clone(), [(2, 1.2)]);
    }

    #[test]
    fn a_voice_that_stops_responding_is_given_up_after_a_minute() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true); // the sink never goes quiet, so no Finished comes
        r.session.say(turn, "Stuck.", After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Stuck.");
        r.clock.advance(59_999);
        r.nothing_more();
        r.clock.advance(1);
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: "The voice stopped responding.".into()
            }
        );
        r.expect_state(State::Idle, false);
        assert!(r.sink.stops() >= 1);
    }

    #[test]
    fn a_long_backlog_does_not_hold_up_commands() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let mut r = rig(Opts {
            detector_gate: Some((entered.clone(), release.clone())),
            ..Opts::default()
        });
        r.expect_state(State::Idle, false);
        r.session.open();
        r.expect_state(State::Listening, true);
        r.mic.send(QUIET, 100);
        entered.wait(); // the worker is on the first frame of the backlog
        r.session.close();
        release.wait();
        r.expect_state(State::Idle, false);
        assert!(r.seen() <= 8, "{} frames were handled first", r.seen());
    }

    #[test]
    fn coughs_do_not_extend_the_wait_for_an_answer() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.asking();
        for _ in 0..3 {
            r.clock.advance(5_000);
            r.mic.send(SPEECH, 1); // too short to be speech: Started, then Dropped
            r.mic.send(QUIET, 2);
            r.nothing_more();
        }
        assert_eq!(r.sync().state, State::Asking);
        r.clock.advance(4_999);
        r.nothing_more();
        r.clock.advance(1);
        r.expect_state(State::Idle, false);
    }

    #[test]
    fn a_sentence_begun_before_the_limit_is_still_heard() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.asking();
        r.clock.advance(19_000);
        r.mic.send(SPEECH, 3);
        assert_eq!(r.next(), SessionEvent::Hearing { text: "yes".into() });
        r.clock.advance(4_000); // the limit passes mid-sentence
        r.nothing_more();
        assert_eq!(r.sync().state, State::Asking);
        r.mic.send(QUIET, 2);
        match r.next() {
            SessionEvent::Heard { text, .. } => assert_eq!(text, "yes"),
            other => panic!("expected Heard, got {other:?}"),
        }
    }

    #[test]
    fn events_serialise_as_the_ui_expects() {
        let json = |e: SessionEvent| serde_json::to_string(&e).unwrap();
        assert_eq!(
            json(state(State::Listening, true)),
            r#"{"kind":"state","state":"listening","mic_open":true}"#
        );
        assert_eq!(
            json(SessionEvent::Level {
                mic: 0.5,
                voice: 0.25
            }),
            r#"{"kind":"level","mic":0.5,"voice":0.25}"#
        );
        assert_eq!(
            json(SessionEvent::Hearing { text: "hi".into() }),
            r#"{"kind":"hearing","text":"hi"}"#
        );
        assert_eq!(
            json(SessionEvent::Heard {
                text: "hi".into(),
                turn: 3
            }),
            r#"{"kind":"heard","text":"hi","turn":3}"#
        );
        assert_eq!(
            json(SessionEvent::Saying { text: "hi".into() }),
            r#"{"kind":"saying","text":"hi"}"#
        );
        assert_eq!(json(SessionEvent::Woke), r#"{"kind":"woke"}"#);
        assert_eq!(
            json(SessionEvent::Trouble {
                message: "m".into()
            }),
            r#"{"kind":"trouble","message":"m"}"#
        );
        for (s, name) in [
            (State::Off, "off"),
            (State::Idle, "idle"),
            (State::Thinking, "thinking"),
            (State::Speaking, "speaking"),
            (State::Asking, "asking"),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn settings_have_the_agreed_values() {
        let s = SessionSettings::default();
        assert_eq!((s.follow_up_secs, s.reopen_after_ms), (6, 250));
        assert!(!s.interrupt_by_talking);
        assert!(s.tone);
    }

    // ---- final fix round: a failed speaker -----------------------------------------------------

    #[test]
    fn a_speaker_that_fails_mid_reply_gives_one_trouble_and_the_next_reply_plays_on_a_new_one() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say(turn, "One. Two.", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("One.");
        r.expect_saying("Two.");
        r.sink.fail();
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: "The speaker stopped working.".into()
            }
        );
        r.expect_state(State::Idle, false);
        r.nothing_more();
        assert!(
            r.reopened.lock().unwrap().is_empty(),
            "opened before needed"
        );

        // Not stuck: the next turn is heard and its reply plays on a fresh speaker.
        let turn = r.hear();
        r.session.say(turn, "Three.", After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Three.");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Idle, false);
        let reopened = r.reopened.lock().unwrap().clone();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened[0].plays(), 1);
        assert_eq!(r.sink.plays(), 2);
    }

    #[test]
    fn a_speaker_that_will_not_reopen_is_reported_and_the_session_rests() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.sink.fail();
        r.sink_fails.store(true, Ordering::SeqCst);
        let turn = r.hear();
        r.session.say_delta(turn, "Lost words. ");
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: "The speaker could not be opened: no speaker was found".into()
            }
        );
        r.expect_state(State::Idle, false);
        r.session.end_of_reply(turn, After::Rest);
        r.nothing_more();
        assert!(r.speaker.said().is_empty());
        // Once a speaker is there again, a notice is spoken.
        r.sink_fails.store(false, Ordering::SeqCst);
        r.session.notice("Back.");
        r.expect_state(State::Speaking, false);
        r.expect_saying("Back.");
    }

    // ---- step 3, task 1: one trouble per failure --------------------------------------------

    #[test]
    fn one_trouble_for_two_fillers() {
        for _ in 0..5 {
            let mut r = rig(Opts::default());
            r.expect_state(State::Idle, false);
            let turn = r.hear();
            r.sink.sounding(true);
            r.session.filler(turn, "One moment.");
            r.expect_saying("One moment.");
            r.session.filler(turn, "Still working.");
            r.sync();
            // Both fillers are at the voice; the speaker breaks under them.
            r.sink.fail();
            assert_eq!(
                r.next(),
                SessionEvent::Trouble {
                    message: "The speaker stopped working.".into()
                }
            );
            // The second filler's failure is the same failure: not reported again.
            r.clock.advance(10);
            r.nothing_more();
            // The reply still comes, on a fresh speaker.
            r.session.say(turn, "Here.", After::Rest);
            r.expect_state(State::Speaking, false);
            r.expect_saying("Here.");
            r.until_speech_done();
            r.clock.advance(250);
            r.expect_state(State::Idle, false);
            r.nothing_more();
        }
    }

    #[test]
    fn one_trouble_per_turn_when_no_speaker_opens() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.sink.fail();
        r.sink_fails.store(true, Ordering::SeqCst);
        let turn = r.hear();
        r.session.filler(turn, "One moment.");
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: "The speaker could not be opened: no speaker was found".into()
            }
        );
        r.nothing_more();
        // The reply of the same turn finds no speaker either: said once already.
        r.session.say(turn, "Here.", After::Rest);
        r.expect_state(State::Idle, false);
        r.nothing_more();
        // A new turn is told again.
        let turn = r.hear();
        r.session.say(turn, "Here.", After::Rest);
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Idle, false);
        r.nothing_more();
        assert!(r.speaker.said().is_empty());
    }

    #[test]
    fn a_reopened_mic_starts_with_clean_ears() {
        let mut r = rig(Opts {
            talk: true,
            ears: EarSettings {
                pre_roll_ms: 64, // two frames
                ..test_ears()
            },
            ..Opts::default()
        });
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say(turn, "A long answer.", After::Rest);
        r.expect_state(State::Speaking, true);
        r.expect_saying("A long answer.");
        // Sound from the old microphone fills the ears' pre-roll, then the microphone goes.
        r.mic.send(QUIET, 2);
        r.mic.lost();
        r.expect_state(State::Speaking, false);
        r.clock.advance(5_000);
        r.expect_state(State::Speaking, true); // the quiet try worked
                                               // Talking over Dot on the new microphone: its clip holds nothing of the old one.
        r.mic.send(SPEECH, 3);
        r.expect_state(State::Listening, true);
        r.mic.send(QUIET, 2);
        assert!(matches!(r.next(), SessionEvent::Hearing { .. }));
        assert!(matches!(r.next(), SessionEvent::Heard { .. }));
        let last = *r.recognizer.lens.lock().unwrap().last().unwrap();
        assert_eq!(
            last,
            5 * FRAME,
            "three speech and two quiet frames, no old pre-roll"
        );
    }

    // ---- final fix round: clicks become turns ---------------------------------------------------

    fn expect_heard(r: &mut Rig, text: &str, turn: u64) {
        assert_eq!(
            r.next(),
            SessionEvent::Heard {
                text: text.into(),
                turn
            }
        );
    }

    #[test]
    fn a_click_while_idle_is_a_turn() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.session.answered("Yes").expect("a turn");
        assert_eq!(turn, 1);
        expect_heard(&mut r, "Yes", 1);
        r.expect_state(State::Thinking, false);
        assert_eq!(r.session.turn(), 1);
        // Its reply is taken like any other.
        r.session.say(turn, "Done.", After::Rest);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Done.");
    }

    #[test]
    fn a_click_while_listening_stops_listening_and_is_a_turn() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let spoken = r.hear();
        r.session.say(spoken, "Which one?", After::FollowUp);
        r.expect_state(State::Speaking, false);
        r.expect_saying("Which one?");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
        r.mic.send(SPEECH, 1); // a clip had begun; the click wins
        let turn = r.session.answered("The second one").unwrap();
        assert_eq!(turn, spoken + 1);
        expect_heard(&mut r, "The second one", turn);
        r.expect_state(State::Thinking, false);
        assert!(!r.mic.is_open());
        r.nothing_more();
    }

    #[test]
    fn a_click_while_asking_is_the_answer() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.asking();
        let turn = r.session.answered("No").unwrap();
        assert_eq!(turn, 2);
        expect_heard(&mut r, "No", 2);
        r.expect_state(State::Thinking, false);
        // A second question can be asked aloud after a clicked answer.
        r.session.say(turn, "And the other one?", After::Ask);
        r.expect_state(State::Speaking, false);
        r.expect_saying("And the other one?");
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Asking, true);
    }

    #[test]
    fn a_click_while_speaking_stops_the_speech_and_drops_the_old_reply() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.speaking();
        let old = r.session.turn();
        let turn = r.session.answered("Stop").unwrap();
        expect_heard(&mut r, "Stop", turn);
        r.expect_state(State::Thinking, true);
        assert!(r.sink.stops() >= 1);
        r.session.say(old, "Old.", After::Rest);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Thinking);
    }

    #[test]
    fn a_click_after_shutdown_is_not_a_turn() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        r.session.shutdown();
        r.expect_state(State::Off, false);
        assert_eq!(r.session.answered("Yes"), None);
        assert!(ends(&r.runtime, &mut r.events));
    }

    // ---- final fix round: a filler line, and the speaking limit -----------------------------------

    #[test]
    fn a_filler_is_spoken_while_thinking_and_the_reply_follows() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.filler(turn, "One moment.");
        r.expect_saying("One moment.");
        r.nothing_more();
        assert_eq!(r.sync().state, State::Thinking);
        r.session.say_delta(turn, "Here it is. ");
        r.expect_state(State::Speaking, false);
        r.expect_saying("Here it is.");
        r.session.end_of_reply(turn, After::Rest);
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Idle, false);
        assert_eq!(r.speaker.said(), ["One moment.", "Here it is."]);
    }

    #[test]
    fn an_empty_reply_waits_for_a_filler_still_sounding() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.filler(turn, "One moment.");
        r.expect_saying("One moment.");
        r.session.end_of_reply(turn, After::FollowUp);
        r.expect_state(State::Speaking, false); // not Listening while Dot still talks
        r.clock.advance(250);
        r.nothing_more();
        r.sink.sounding(false);
        r.until_speech_done();
        r.clock.advance(250);
        r.expect_state(State::Listening, true);
    }

    #[test]
    fn a_filler_keeps_the_think_limit_and_a_stale_filler_is_dropped() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.session.filler(turn + 1, "Wrong turn.");
        r.clock.advance(100_000);
        r.session.filler(turn, "Still working.");
        r.expect_saying("Still working.");
        r.clock.advance(19_999);
        r.nothing_more();
        r.clock.advance(1);
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: NO_REPLY.into()
            }
        );
        r.expect_state(State::Idle, false);
        r.session.filler(turn, "Too late.");
        r.nothing_more();
        assert_eq!(r.speaker.said(), ["Still working."]);
    }

    #[test]
    fn a_long_streamed_reply_is_not_cut_while_text_still_arrives() {
        let mut r = rig(Opts::default());
        r.expect_state(State::Idle, false);
        let turn = r.hear();
        r.sink.sounding(true);
        r.session.say_delta(turn, "First part. ");
        r.expect_state(State::Speaking, false);
        r.expect_saying("First part.");
        for _ in 0..3 {
            r.clock.advance(50_000);
            r.session.say_delta(turn, "and more"); // no sentence end: no Saying to re-arm it
            r.nothing_more();
        }
        r.clock.advance(50_000);
        r.session.end_of_reply(turn, After::Rest);
        r.expect_saying("and moreand moreand more");
        r.clock.advance(59_999);
        r.nothing_more();
        assert_eq!(r.sync().state, State::Speaking);
        r.clock.advance(1);
        assert_eq!(
            r.next(),
            SessionEvent::Trouble {
                message: VOICE_STUCK.into()
            }
        );
        r.expect_state(State::Idle, false);
    }
}
