//! The listen, think, speak loop: one worker thread ties the microphone, the ears, the wake
//! phrase and the voice together, and tells the app what happens through [`SessionEvent`]s.
//!
//! Nothing here logs. Heard and spoken text leave only through the events; audio never
//! leaves memory.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::audio::{Mic, MicFrame};
use crate::ears::{level, EarEvent, Ears};
use crate::engine::{EngineError, WakeSpotter, WakeWord};
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

const NO_REPLY: &str = "No reply came back.";
const VOICE_STUCK: &str = "The voice stopped responding.";
const MIC_LOST: &str = "The microphone stopped working.";
const BROKE: &str = "Speech stopped working and was turned off.";

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
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            wake_phrase: false,
            interrupt_by_talking: false,
            follow_up_secs: 6,
            reopen_after_ms: 250,
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
    End {
        turn: u64,
        then: After,
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
    turn: AtomicU64,
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

    /// The streamed reply for `turn` is complete; ignored as `say` is.
    pub fn end_of_reply(&self, turn: u64, then: After) {
        self.send(Command::End { turn, then });
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
    wake: Option<Box<dyn WakeSpotter>>,
    open_mic: MicOpener,
    settings: SessionSettings,
    events: UnboundedSender<SessionEvent>,
    clock: Arc<dyn Clock>,
    shared: Arc<Shared>,

    state: State,
    mic: Option<MicStream>,
    /// The microphone failed: it is not reopened just for the wake phrase until the app
    /// asks (`open`, `settings`).
    mic_broken: bool,
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
    then: After,
    /// When the ears may hear again after speech.
    reopen_at: Option<Instant>,
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
            wake: parts.wake,
            open_mic: parts.open_mic,
            settings,
            events,
            clock,
            shared,
            state: State::Idle,
            mic: None,
            mic_broken: false,
            published: None,
            last_level: None,
            quiet_since: now,
            listen_limit: LISTEN_AFTER_OPEN,
            in_speech: false,
            clip_started: now,
            deaf_until: None,
            halt_frames: false,
            frames_left: false,
            voice_heard_at: now,
            thinking_since: now,
            asked: false,
            turn: 0,
            turn_open: false,
            reply_open: false,
            replies_pending: 0,
            then: After::Rest,
            reopen_at: None,
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
                match self.state {
                    State::Idle => self.listen(LISTEN_AFTER_OPEN),
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
                    self.voice.say(&text);
                    self.reply_open = false;
                    self.replies_pending += 1;
                    self.reply_ended(then);
                }
            }
            Command::Delta { turn, text } => {
                if self.answers(turn) {
                    self.reply_open = true;
                    self.voice.say_delta(&text);
                    self.enter(State::Speaking);
                }
            }
            Command::End { turn, then } => {
                if self.answers(turn) {
                    if self.reply_open {
                        self.voice.end_of_reply();
                        self.reply_open = false;
                        self.replies_pending += 1;
                    }
                    self.reply_ended(then);
                }
            }
            Command::Notice(text) => {
                if self.state == State::Idle {
                    self.voice.say(&text);
                    self.replies_pending += 1;
                    self.then = After::Rest;
                    self.reopen_at = None;
                    self.enter(State::Speaking);
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
        if self.replies_pending == 0 {
            self.after_speech(); // nothing to say: move on at once
        } else {
            self.enter(State::Speaking);
        }
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
        let was_speaking = self.state == State::Speaking;
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
        self.reopen_at = None;
    }

    // ---- states --------------------------------------------------------------------------

    fn listen(&mut self, limit: Duration) {
        self.ears.reset();
        self.in_speech = false;
        self.deaf_until = None;
        self.quiet_since = self.clock.now();
        self.listen_limit = limit;
        self.enter(State::Listening);
    }

    fn ask(&mut self) {
        self.short_ears.reset();
        self.in_speech = false;
        self.deaf_until = None;
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
            if !new.hears() {
                self.ears.reset();
                self.short_ears.reset();
                self.in_speech = false;
                self.deaf_until = None;
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
    /// microphone that will not open is reported and the session rests.
    fn refresh(&mut self) {
        if let Err(e) = self.fit_mic() {
            self.send(SessionEvent::Trouble {
                message: format!("The microphone could not be opened: {e}"),
            });
            self.mic_broken = true;
            self.stop_speech();
            self.ears.reset();
            self.short_ears.reset();
            self.in_speech = false;
            self.last_level = None;
            self.state = State::Idle;
            let _ = self.fit_mic(); // only closes now
        }
        self.publish();
    }

    fn wants_mic(&self) -> bool {
        let usable = !self.mic_broken;
        let wake = self.settings.wake_phrase && self.wake.is_some() && usable;
        match self.state {
            State::Off => false,
            State::Listening | State::Asking => true,
            State::Idle | State::Thinking => wake,
            State::Speaking => wake || (self.settings.interrupt_by_talking && usable),
        }
    }

    fn fit_mic(&mut self) -> Result<(), EngineError> {
        if !self.wants_mic() {
            self.mic = None;
        } else if self.mic.is_none() {
            let mut mic = (self.open_mic)()?;
            let frames = mic.frames();
            self.mic = Some(MicStream { frames, _mic: mic });
            self.mic_broken = false;
            if let Some(spotter) = &mut self.wake {
                spotter.reset();
            }
        }
        Ok(())
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

    /// Once the tail after an interruption has passed, the frames still waiting (captured
    /// before its end) are thrown away and the ears start clean.
    fn end_deafness(&mut self) {
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
    /// the microphone turned out to be lost (already reported, and the session rests).
    fn discard_backlog(&mut self) -> bool {
        loop {
            let Some(mic) = &self.mic else { return true };
            match mic.frames.try_recv() {
                Ok(MicFrame::Audio(_)) => {}
                Ok(MicFrame::Lost(_)) | Err(TryRecvError::Disconnected) => {
                    self.mic_lost();
                    return false;
                }
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    fn mic_lost(&mut self) {
        self.mic = None;
        self.mic_broken = true;
        self.trouble(MIC_LOST);
        self.stop_speech();
        self.enter(State::Idle);
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
        let word = self.spot(frame);
        match self.state {
            State::Off => {}
            State::Idle => {
                if word == Some(WakeWord::HeyDot) {
                    self.send(SessionEvent::Woke);
                    self.listen(LISTEN_AFTER_OPEN);
                }
            }
            State::Thinking => {
                if word.is_some() {
                    self.interrupt();
                }
            }
            State::Speaking => {
                if word.is_some() {
                    self.interrupt();
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
                self.level(level(frame), 0.0, now);
                if word == Some(WakeWord::DotStop) {
                    self.close();
                    return;
                }
                if self.deaf_until.is_some() {
                    return; // still the tail of Dot's own voice
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

    fn ear_event(&mut self, event: EarEvent, now: Instant) {
        match event {
            EarEvent::Started => {
                self.in_speech = true;
                self.clip_started = now;
            }
            EarEvent::Partial(text) => self.send(SessionEvent::Hearing { text }),
            EarEvent::Final(text) => {
                // What queued up while the sentence was transcribed is not fresh sound. Known
                // limit: a "Dot, stop" said during that transcription is thrown away with it.
                if !self.discard_backlog() {
                    return;
                }
                self.turn += 1;
                self.shared.turn.store(self.turn, Ordering::SeqCst);
                self.turn_open = true;
                self.send(SessionEvent::Heard {
                    text,
                    turn: self.turn,
                });
                if let Some(spotter) = &mut self.wake {
                    spotter.reset();
                }
                self.enter(State::Thinking);
            }
            EarEvent::Dropped => {
                // The limit was paused while the clip was open; it is not restarted.
                self.in_speech = false;
                self.quiet_since += now.saturating_duration_since(self.clip_started);
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
            match event {
                VoiceEvent::Saying(text) => {
                    self.voice_heard_at = self.clock.now();
                    if self.state == State::Speaking {
                        self.send(SessionEvent::Saying { text });
                    }
                }
                VoiceEvent::Finished => {
                    self.voice_heard_at = self.clock.now();
                    self.replies_pending = self.replies_pending.saturating_sub(1);
                }
                VoiceEvent::Stopped => {}
                VoiceEvent::Trouble(message) => {
                    self.send(SessionEvent::Trouble { message });
                    if self.state == State::Speaking {
                        self.stop_speech();
                        self.enter(State::Idle);
                    }
                }
            }
        }
    }

    /// Every reply has been spoken to its end.
    fn speech_done(&self) -> bool {
        self.replies_pending == 0 && !self.reply_open
    }

    fn timers(&mut self) {
        let now = self.clock.now();
        match self.state {
            State::Listening | State::Asking => {
                if !self.in_speech
                    && now.saturating_duration_since(self.quiet_since) >= self.listen_limit
                {
                    self.enter(State::Idle);
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
                    let tail = Duration::from_millis(u64::from(self.settings.reopen_after_ms));
                    self.reopen_at = Some(now + tail);
                }
                if self.reopen_at.is_some_and(|at| now >= at) {
                    self.after_speech();
                }
            }
            State::Off | State::Idle => {}
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
        match self.then {
            After::Rest => self.enter(State::Idle),
            After::FollowUp => {
                let limit = Duration::from_secs(u64::from(self.settings.follow_up_secs));
                self.listen(limit);
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
        panics: AtomicBool,
        /// Holds the first call: it reaches `entered`, then waits at `release`.
        gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
    }

    impl FakeRecognizer {
        fn new(text: &'static str) -> Arc<Self> {
            Arc::new(Self {
                text,
                calls: AtomicUsize::new(0),
                panics: AtomicBool::new(false),
                gate: None,
            })
        }
    }

    impl Recognizer for FakeRecognizer {
        fn transcribe(&self, _clip: &Clip) -> Result<String, EngineError> {
            if self.panics.load(Ordering::SeqCst) {
                panic!("the recogniser broke");
            }
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                if let Some((entered, release)) = &self.gate {
                    entered.wait();
                    release.wait();
                }
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
    }

    impl FakeSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                idle: AtomicBool::new(true),
                level: AtomicU32::new(0f32.to_bits()),
                stops: AtomicUsize::new(0),
            })
        }
        fn sounding(&self, on: bool) {
            self.idle.store(!on, Ordering::SeqCst);
        }
        fn stops(&self) -> usize {
            self.stops.load(Ordering::SeqCst)
        }
    }

    impl Sink for FakeSink {
        fn play(&self, _clip: Clip) {}
        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.idle.store(true, Ordering::SeqCst);
        }
        fn idle(&self) -> bool {
            self.idle.load(Ordering::SeqCst)
        }
        fn level(&self) -> f32 {
            f32::from_bits(self.level.load(Ordering::SeqCst))
        }
    }

    struct Spotter;

    impl WakeSpotter for Spotter {
        fn heard(&mut self, frame16k: &[f32]) -> Option<WakeWord> {
            if frame16k[0] == HEY_DOT {
                Some(WakeWord::HeyDot)
            } else if frame16k[0] == DOT_STOP {
                Some(WakeWord::DotStop)
            } else {
                None
            }
        }
        fn reset(&mut self) {}
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
        let parts = Parts {
            ears,
            short_ears,
            voice,
            voice_events,
            sink: sink.clone(),
            wake: Some(Box::new(Spotter)),
            open_mic: Box::new(move || feed.open_mic()),
        };
        let settings = SessionSettings {
            wake_phrase: opts.wake,
            interrupt_by_talking: opts.talk,
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
        r.nothing_more(); // still Idle with no microphone: nothing changed to report
        assert_eq!(r.session.state(), State::Idle);
        r.mic.fail.store(false, Ordering::SeqCst);
        r.session.open();
        r.expect_state(State::Listening, true);
    }

    #[test]
    fn with_the_wake_phrase_a_lost_mic_is_not_reopened_until_asked() {
        let mut r = rig(Opts {
            wake: true,
            ..Opts::default()
        });
        r.expect_state(State::Idle, true);
        r.mic.lost();
        assert!(matches!(r.next(), SessionEvent::Trouble { .. }));
        r.expect_state(State::Idle, false);
        r.nothing_more();
        assert_eq!(r.mic.opens(), 1);
        r.session.open();
        r.expect_state(State::Listening, true);
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
            panics: AtomicBool::new(false),
            gate: Some((entered.clone(), release.clone())),
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
    }
}
