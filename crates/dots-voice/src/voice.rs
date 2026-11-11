//! The speaking queue: a reply, whole or streaming, is cut into sentences, each is synthesised
//! in turn on a worker thread and handed to a [`Sink`], and `stop` silences everything at once.
//!
//! Nothing here logs; no spoken text or audio leaves memory except through the events and the
//! sink.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::engine::{Clip, Speaker};
use crate::text::{speakable, trim_spoken, SentenceStream, MAX_SPOKEN_CHARS};

/// How often the worker asks the sink whether it has gone quiet.
const IDLE_POLL: Duration = Duration::from_millis(20);
const MIN_SPEED: f32 = 0.8;
const MAX_SPEED: f32 = 1.4;

/// What `Voice` says when the sink refuses a clip or has failed.
pub(crate) const SPEAKER_BROKE: &str = "The speaker stopped working.";

/// Where clips are played. Implemented by the speaker output.
pub trait Sink: Send + Sync {
    /// Queues the clip; returns at once. False when the clip was refused (the speaker failed,
    /// or the queue is full) and will not be heard.
    fn play(&self, clip: Clip) -> bool;
    /// Drops everything queued and playing, at once.
    fn stop(&self);
    /// True when nothing is queued or playing. A failed sink is idle.
    fn idle(&self) -> bool;
    /// 0.0..=1.0 of what is sounding now.
    fn level(&self) -> f32;
    /// True once the speaker has stopped working for good; a new sink is needed.
    fn failed(&self) -> bool;
}

#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    /// A sentence was handed to the sink.
    Saying(String),
    /// The reply was spoken to the end.
    Finished,
    /// `stop` cut the speech short.
    Stopped,
    /// Speech could not be made; the reply ends.
    Trouble(String),
}

enum Command {
    Delta { gen: u64, reply: u64, text: String },
    End { gen: u64, reply: u64 },
}

struct State {
    /// Bumped by every `stop`; work for an older generation is thrown away.
    gen: u64,
    next_reply: u64,
    /// The reply still taking text, if any.
    open: Option<u64>,
    /// A streamed reply was cut short (stop or failure) and its end has not come yet: its
    /// remaining deltas are dropped.
    dropping: bool,
    /// Replies begun and not yet finished, stopped or failed.
    pending: usize,
    voice: u8,
    speed: f32,
    /// Where clips go; replaced by `set_sink` after a failure.
    sink: Arc<dyn Sink>,
}

struct Shared {
    state: Mutex<State>,
    /// Woken by `stop`, so a wait for the sink to go idle ends at once.
    wake: Condvar,
    events: UnboundedSender<VoiceEvent>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, event: VoiceEvent) {
        // A closed receiver only means nobody is listening any more.
        let _ = self.events.send(event);
    }
}

/// The speaking queue. Every method returns at once; the speaker runs on a worker thread that
/// ends when this handle is dropped.
pub struct Voice {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl Voice {
    pub fn new(
        speaker: Arc<dyn Speaker>,
        sink: Arc<dyn Sink>,
        events: UnboundedSender<VoiceEvent>,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                gen: 0,
                next_reply: 0,
                open: None,
                dropping: false,
                pending: 0,
                voice: 0,
                speed: 1.0,
                sink,
            }),
            wake: Condvar::new(),
            events,
        });
        let (commands, inbox) = channel();
        let worker = Worker {
            shared: shared.clone(),
            speaker,
            inbox,
            reply: None,
        };
        std::thread::Builder::new()
            .name("dots-voice-speaking".into())
            .spawn(move || worker.run())
            .expect("the speaking thread starts");
        Self { shared, commands }
    }

    /// Applies to sentences synthesised after the call. Speed is kept within 0.8..=1.4.
    pub fn set_voice(&self, voice: u8, speed: f32) {
        let speed = if speed.is_nan() {
            1.0
        } else {
            speed.clamp(MIN_SPEED, MAX_SPEED)
        };
        let mut st = self.shared.lock();
        st.voice = voice;
        st.speed = speed;
    }

    /// Plays into `sink` from now on, typically after the old one failed. Whatever the old
    /// sink still held is stopped.
    pub fn set_sink(&self, sink: Arc<dyn Sink>) {
        let mut st = self.shared.lock();
        let old = std::mem::replace(&mut st.sink, sink);
        old.stop();
    }

    /// A whole reply: the same as `say_delta(text)` then `end_of_reply()`. Always a new reply,
    /// even right after a `stop` that cut a streamed reply short.
    pub fn say(&self, text: &str) {
        self.shared.lock().dropping = false; // a whole reply is always a new reply
        self.say_delta(text);
        self.end_of_reply();
    }

    /// Part of a reply still arriving. A reply spoken while an earlier one is still sounding
    /// queues behind it.
    /// Dropped while a reply cut short by `stop` or a failure waits for its `end_of_reply`.
    pub fn say_delta(&self, delta: &str) {
        let mut st = self.shared.lock();
        if st.dropping {
            return;
        }
        let reply = match st.open {
            Some(reply) => reply,
            None => {
                let reply = st.next_reply;
                st.next_reply += 1;
                st.open = Some(reply);
                st.pending += 1;
                reply
            }
        };
        let gen = st.gen;
        // Sent under the lock so commands reach the worker in the order they were made.
        let _ = self.commands.send(Command::Delta {
            gen,
            reply,
            text: delta.to_string(),
        });
    }

    /// The reply has no more text; whatever partial sentence is left is spoken. If the reply
    /// was cut short by `stop` or a failure, this only ends it: nothing is spoken, no event.
    pub fn end_of_reply(&self) {
        let mut st = self.shared.lock();
        if st.dropping {
            st.dropping = false; // the cut-short reply is over; no event
            return;
        }
        if let Some(reply) = st.open.take() {
            let gen = st.gen;
            let _ = self.commands.send(Command::End { gen, reply });
        }
    }

    /// Silences everything now. Sends `Stopped` once, and nothing when nothing was being said.
    /// If a streamed reply is still open (no `end_of_reply` yet), its later `say_delta` calls
    /// are dropped until `end_of_reply`, so speech never resumes mid-sentence.
    pub fn stop(&self) {
        let mut st = self.shared.lock();
        let was_speaking = st.pending > 0;
        if !was_speaking && st.open.is_none() {
            return;
        }
        st.gen += 1;
        st.pending = 0;
        if st.open.take().is_some() {
            st.dropping = true;
        }
        st.sink.stop();
        self.shared.wake.notify_all();
        if was_speaking {
            self.shared.send(VoiceEvent::Stopped);
        }
    }

    /// True from the first text of a reply until it finishes, is stopped or fails.
    pub fn speaking(&self) -> bool {
        self.shared.lock().pending > 0
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        // Silence the sink; the worker ends when the command channel closes.
        self.stop();
    }
}

/// What the worker has done so far for the reply it is on.
struct ReplyState {
    id: u64,
    stream: SentenceStream,
    /// Bytes of spoken text handed on so far.
    spoken: usize,
    /// The length cap was reached: later sentences are dropped silently.
    capped: bool,
    /// Speech failed: the rest of the reply is dropped.
    failed: bool,
}

struct Worker {
    shared: Arc<Shared>,
    speaker: Arc<dyn Speaker>,
    inbox: Receiver<Command>,
    reply: Option<ReplyState>,
}

impl Worker {
    fn run(mut self) {
        while let Ok(command) = self.inbox.recv() {
            match command {
                Command::Delta { gen, reply, text } => self.on_delta(gen, reply, &text),
                Command::End { gen, reply } => self.on_end(gen, reply),
            }
        }
    }

    fn current(&self, gen: u64) -> bool {
        self.shared.lock().gen == gen
    }

    /// The state of reply `id`, started afresh if the worker was on another one.
    fn reply_state(&mut self, id: u64) -> &mut ReplyState {
        if self.reply.as_ref().map(|r| r.id) != Some(id) {
            self.reply = Some(ReplyState {
                id,
                stream: SentenceStream::new(),
                spoken: 0,
                capped: false,
                failed: false,
            });
        }
        self.reply.as_mut().expect("just set")
    }

    fn on_delta(&mut self, gen: u64, reply: u64, text: &str) {
        if !self.current(gen) || self.reply_state(reply).failed {
            return;
        }
        let sentences = self.reply_state(reply).stream.push(text);
        self.speak_all(gen, reply, sentences);
    }

    fn on_end(&mut self, gen: u64, reply: u64) {
        if !self.current(gen) {
            return;
        }
        if !self.reply_state(reply).failed {
            let rest = self.reply_state(reply).stream.finish();
            self.speak_all(gen, reply, rest);
            if !self.reply_state(reply).failed && self.wait_until_idle(gen) {
                let mut st = self.shared.lock();
                if st.gen == gen {
                    st.pending = st.pending.saturating_sub(1);
                    // A sink that failed while sounding went idle by throwing the rest away.
                    if st.sink.failed() {
                        self.shared.send(VoiceEvent::Trouble(SPEAKER_BROKE.into()));
                    } else {
                        self.shared.send(VoiceEvent::Finished);
                    }
                }
            }
        }
        self.reply = None;
    }

    fn speak_all(&mut self, gen: u64, reply: u64, sentences: Vec<String>) {
        for sentence in sentences {
            if !self.current(gen) || self.reply_state(reply).failed {
                return;
            }
            self.speak(gen, reply, &sentence);
        }
    }

    fn speak(&mut self, gen: u64, reply: u64, sentence: &str) {
        let mut cleaned = speakable(sentence);
        let state = self.reply_state(reply);
        if cleaned.is_empty() || state.capped {
            return;
        }
        if state.spoken == 0 && cleaned.len() > MAX_SPOKEN_CHARS {
            cleaned = trim_spoken(&cleaned);
        }
        let separator = usize::from(state.spoken > 0);
        let length = cleaned.len();
        if state.spoken + separator + length > MAX_SPOKEN_CHARS {
            state.capped = true;
            return;
        }
        let (voice, speed) = {
            let st = self.shared.lock();
            (st.voice, st.speed)
        };
        match self.speaker.speak(&cleaned, voice, speed) {
            Ok(clip) => {
                let st = self.shared.lock();
                if st.gen != gen {
                    return; // stopped while synthesising: the clip is thrown away
                }
                if !st.sink.play(clip) {
                    drop(st);
                    self.fail(gen, reply, SPEAKER_BROKE.into());
                    return;
                }
                self.shared.send(VoiceEvent::Saying(cleaned));
                drop(st);
                self.reply_state(reply).spoken += separator + length;
            }
            Err(error) => self.fail(gen, reply, error.to_string()),
        }
    }

    /// The reply cannot go on: the rest of it is dropped and `Trouble` ends it, once.
    fn fail(&mut self, gen: u64, reply: u64, message: String) {
        self.reply_state(reply).failed = true;
        let mut st = self.shared.lock();
        if st.gen == gen {
            st.pending = st.pending.saturating_sub(1);
            if st.open == Some(reply) {
                st.open = None;
                st.dropping = true;
            }
            self.shared.send(VoiceEvent::Trouble(message));
        }
    }

    /// Waits for the sink to go quiet. False when `stop` ended the wait.
    fn wait_until_idle(&self, gen: u64) -> bool {
        let mut st = self.shared.lock();
        loop {
            if st.gen != gen {
                return false;
            }
            if st.sink.idle() {
                return true;
            }
            st = self
                .shared
                .wake
                .wait_timeout(st, IDLE_POLL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineError;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread::ThreadId;
    use std::time::Instant;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    const WAIT: Duration = Duration::from_secs(10);

    /// Records what it was asked; the first call can be held on a pair of barriers.
    struct FakeSpeaker {
        calls: Mutex<Vec<(String, u8, f32)>>,
        threads: Mutex<Vec<ThreadId>>,
        gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
        fail: AtomicBool,
        count: AtomicUsize,
    }

    impl FakeSpeaker {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::default(),
                threads: Mutex::default(),
                gate: None,
                fail: AtomicBool::new(false),
                count: AtomicUsize::new(0),
            })
        }

        /// Holds the first call: it reaches `entered`, then waits at `release`.
        fn gated() -> (Arc<Self>, Arc<Barrier>, Arc<Barrier>) {
            let entered = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            let speaker = Arc::new(Self {
                calls: Mutex::default(),
                threads: Mutex::default(),
                gate: Some((entered.clone(), release.clone())),
                fail: AtomicBool::new(false),
                count: AtomicUsize::new(0),
            });
            (speaker, entered, release)
        }

        fn sentences(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.0.clone())
                .collect()
        }
    }

    impl Speaker for FakeSpeaker {
        fn speak(&self, sentence: &str, voice: u8, speed: f32) -> Result<Clip, EngineError> {
            self.threads
                .lock()
                .unwrap()
                .push(std::thread::current().id());
            self.calls
                .lock()
                .unwrap()
                .push((sentence.to_string(), voice, speed));
            if self.count.fetch_add(1, Ordering::SeqCst) == 0 {
                if let Some((entered, release)) = &self.gate {
                    entered.wait();
                    release.wait();
                }
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(EngineError::Failed("no voice".into()));
            }
            Ok(Clip {
                samples: vec![0.0; 8],
                rate: 22_050,
            })
        }

        fn voices(&self) -> u8 {
            1
        }
    }

    struct FakeSink {
        clips: Mutex<Vec<Clip>>,
        idle: AtomicBool,
        stops: AtomicUsize,
        failed: AtomicBool,
    }

    impl FakeSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                clips: Mutex::default(),
                idle: AtomicBool::new(true),
                stops: AtomicUsize::new(0),
                failed: AtomicBool::new(false),
            })
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
            self.clips.lock().unwrap().push(clip);
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
        fn level(&self) -> f32 {
            0.0
        }
    }

    struct Rig {
        voice: Voice,
        events: UnboundedReceiver<VoiceEvent>,
        speaker: Arc<FakeSpeaker>,
        sink: Arc<FakeSink>,
    }

    fn rig_with(speaker: Arc<FakeSpeaker>) -> Rig {
        let sink = FakeSink::new();
        let (tx, events) = unbounded_channel();
        let voice = Voice::new(speaker.clone(), sink.clone(), tx);
        Rig {
            voice,
            events,
            speaker,
            sink,
        }
    }

    fn rig() -> Rig {
        rig_with(FakeSpeaker::new())
    }

    impl Rig {
        /// The next event, waiting up to `WAIT`; a timeout means something is broken.
        fn next(&mut self) -> VoiceEvent {
            let deadline = Instant::now() + WAIT;
            loop {
                if let Ok(event) = self.events.try_recv() {
                    return event;
                }
                assert!(Instant::now() < deadline, "no event arrived in time");
                std::thread::yield_now();
            }
        }

        fn nothing_pending(&mut self) -> bool {
            self.events.try_recv().is_err()
        }
    }

    fn say(s: &str) -> VoiceEvent {
        VoiceEvent::Saying(s.to_string())
    }

    #[test]
    fn a_reply_is_spoken_sentence_by_sentence_in_order() {
        let mut rig = rig();
        rig.voice.say("One. Two. Three.");
        assert_eq!(rig.next(), say("One."));
        assert_eq!(rig.next(), say("Two."));
        assert_eq!(rig.next(), say("Three."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert_eq!(rig.speaker.sentences(), ["One.", "Two.", "Three."]);
        assert_eq!(rig.sink.clips.lock().unwrap().len(), 3);
        assert!(!rig.voice.speaking());
        // The speaker only ever runs on the worker thread.
        let me = std::thread::current().id();
        assert!(rig.speaker.threads.lock().unwrap().iter().all(|t| *t != me));
    }

    #[test]
    fn streamed_text_starts_speaking_before_the_reply_ends() {
        let mut rig = rig();
        rig.voice.say_delta("Hello the");
        rig.voice.say_delta("re. Second par");
        assert_eq!(rig.next(), say("Hello there."));
        assert!(rig.voice.speaking());
        assert!(rig.nothing_pending());
        rig.voice.say_delta("t");
        rig.voice.end_of_reply();
        assert_eq!(rig.next(), say("Second part"));
        assert_eq!(rig.next(), VoiceEvent::Finished);
    }

    #[test]
    fn stop_during_synthesis_plays_nothing_from_that_sentence() {
        let (speaker, entered, release) = FakeSpeaker::gated();
        let mut rig = rig_with(speaker);
        rig.voice.say("Held back.");
        entered.wait(); // the speaker is inside the call, blocked; `say` has already returned
        rig.voice.stop();
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        assert!(!rig.voice.speaking());
        release.wait();
        // A following reply shows the worker has moved on, and that the old clip never played.
        rig.voice.say("Next.");
        assert_eq!(rig.next(), say("Next."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert_eq!(rig.sink.clips.lock().unwrap().len(), 1);
        assert!(rig.nothing_pending());
    }

    #[test]
    fn a_new_reply_after_stop_is_spoken() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst); // keeps the first reply sounding
        rig.voice.say("First.");
        assert_eq!(rig.next(), say("First."));
        rig.voice.stop();
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        assert!(rig.sink.stops.load(Ordering::SeqCst) >= 1);
        rig.voice.say("Second.");
        assert_eq!(rig.next(), say("Second."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert!(rig.nothing_pending());
    }

    #[test]
    fn stop_with_nothing_to_stop_sends_nothing() {
        let mut rig = rig();
        rig.voice.stop();
        assert!(rig.nothing_pending());
        rig.voice.say("Done.");
        assert_eq!(rig.next(), say("Done."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        rig.voice.stop();
        assert!(rig.nothing_pending());
    }

    #[test]
    fn only_markdown_or_empty_text_says_nothing_and_finishes() {
        let mut rig = rig();
        rig.voice.say("---\n\u{1F642}");
        assert_eq!(rig.next(), VoiceEvent::Finished);
        rig.voice.say("");
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert!(rig.speaker.sentences().is_empty());
        assert!(rig.nothing_pending());
    }

    #[test]
    fn a_code_only_reply_says_code_is_on_screen() {
        let mut rig = rig();
        rig.voice.say("```rust\nlet x = 1;\nprintln!(\"hi\");\n```");
        assert_eq!(rig.next(), say("(code is on screen)"));
        assert_eq!(rig.next(), VoiceEvent::Finished);
    }

    #[test]
    fn a_speaker_error_reports_trouble_and_ends() {
        let mut rig = rig();
        rig.speaker.fail.store(true, Ordering::SeqCst);
        rig.voice.say_delta("Broken. More text. ");
        assert_eq!(rig.next(), VoiceEvent::Trouble("no voice".into()));
        assert!(!rig.voice.speaking());
        rig.voice.say_delta("Even more.");
        rig.voice.end_of_reply();
        rig.speaker.fail.store(false, Ordering::SeqCst);
        // The failed reply is over; the next one speaks normally.
        rig.voice.say("Fine.");
        assert_eq!(rig.next(), say("Fine."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert_eq!(rig.speaker.sentences(), ["Broken.", "Fine."]);
    }

    #[test]
    fn speed_is_clamped() {
        let mut rig = rig();
        rig.voice.set_voice(3, 9.0);
        rig.voice.say("Fast.");
        assert_eq!(rig.next(), say("Fast."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        rig.voice.set_voice(0, 0.1);
        rig.voice.say("Slow.");
        assert_eq!(rig.next(), say("Slow."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        let calls = rig.speaker.calls.lock().unwrap().clone();
        assert_eq!(calls[0], ("Fast.".to_string(), 3, 1.4));
        assert_eq!(calls[1], ("Slow.".to_string(), 0, 0.8));
    }

    #[test]
    fn a_reply_started_while_another_sounds_queues_behind_it() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say("Alpha.");
        rig.voice.say("Beta.");
        assert_eq!(rig.next(), say("Alpha."));
        assert!(rig.nothing_pending());
        rig.sink.idle.store(true, Ordering::SeqCst);
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert_eq!(rig.next(), say("Beta."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
    }

    #[test]
    fn finished_waits_for_the_sink_and_stop_ends_the_wait() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say("Waiting.");
        assert_eq!(rig.next(), say("Waiting."));
        assert!(rig.voice.speaking());
        rig.voice.stop();
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        // Nothing follows: the wait ended without a Finished.
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say("Again.");
        assert_eq!(rig.next(), say("Again."));
        rig.sink.idle.store(true, Ordering::SeqCst);
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert!(rig.nothing_pending());
    }

    #[test]
    fn a_very_long_reply_is_cut_at_the_cap_and_still_finishes() {
        let mut rig = rig();
        for n in 0..200 {
            rig.voice
                .say_delta(&format!("This is sentence number {n}. "));
        }
        rig.voice.end_of_reply();
        let mut total = 0;
        let mut said = 0;
        loop {
            match rig.next() {
                VoiceEvent::Saying(text) => {
                    total += text.chars().count();
                    said += 1;
                }
                VoiceEvent::Finished => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(said > 1);
        assert!(total <= MAX_SPOKEN_CHARS, "{total} chars were spoken");
        assert_eq!(rig.speaker.sentences().len(), said);
    }

    #[test]
    fn dropping_the_voice_ends_the_worker() {
        let speaker = FakeSpeaker::new();
        let rig = rig_with(speaker.clone());
        assert!(Arc::strong_count(&speaker) > 2); // the test, the rig and the worker
        let Rig {
            voice,
            events,
            speaker: rig_speaker,
            sink,
        } = rig;
        drop(rig_speaker);
        drop(voice);
        let deadline = Instant::now() + WAIT;
        while Arc::strong_count(&speaker) > 1 {
            assert!(Instant::now() < deadline, "the worker did not end");
            std::thread::yield_now();
        }
        drop((events, sink));
    }

    #[test]
    fn a_closed_event_receiver_does_not_stop_the_speech() {
        let mut rig = rig();
        let Rig {
            voice,
            events,
            speaker,
            sink,
        } = {
            rig.voice.say("Hello.");
            assert_eq!(rig.next(), say("Hello."));
            rig
        };
        drop(events);
        voice.say("Still going.");
        voice.stop();
        voice.say("And again.");
        let deadline = Instant::now() + WAIT;
        while speaker.sentences().len() < 2 {
            assert!(Instant::now() < deadline, "the worker stopped working");
            std::thread::yield_now();
        }
        drop(sink);
    }

    #[test]
    fn the_cap_counts_bytes_so_multibyte_text_is_cut_too() {
        let mut rig = rig();
        let sentence = format!("{}.", vec!["\u{e9}\u{e9}"; 20].join(" "));
        for _ in 0..100 {
            rig.voice.say_delta(&format!("{sentence} "));
        }
        rig.voice.end_of_reply();
        let mut total = 0;
        let mut said = 0;
        loop {
            match rig.next() {
                VoiceEvent::Saying(text) => {
                    total += text.len();
                    said += 1;
                }
                VoiceEvent::Finished => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(said > 1);
        assert!(total <= MAX_SPOKEN_CHARS, "{total} bytes were spoken");
    }

    #[test]
    fn deltas_after_a_stop_are_dropped_until_the_reply_ends() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say_delta("First one. Sec");
        assert_eq!(rig.next(), say("First one."));
        rig.voice.stop();
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        rig.voice.say_delta("ond half. More. ");
        rig.voice.end_of_reply();
        rig.voice.say_delta("Fresh.");
        rig.voice.end_of_reply();
        rig.sink.idle.store(true, Ordering::SeqCst);
        assert_eq!(rig.next(), say("Fresh."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert!(rig.nothing_pending());
        assert_eq!(rig.speaker.sentences(), ["First one.", "Fresh."]);
    }

    #[test]
    fn stop_with_no_open_reply_does_not_drop_the_next_deltas() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say("Whole.");
        assert_eq!(rig.next(), say("Whole."));
        rig.voice.stop(); // the reply was already ended
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        rig.sink.idle.store(true, Ordering::SeqCst);
        rig.voice.say_delta("Next one.");
        rig.voice.end_of_reply();
        assert_eq!(rig.next(), say("Next one."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
    }

    #[test]
    fn a_whole_reply_after_a_mid_stream_stop_is_spoken() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say_delta("Start. And");
        assert_eq!(rig.next(), say("Start."));
        rig.voice.stop();
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        rig.sink.idle.store(true, Ordering::SeqCst);
        rig.voice.say("New.");
        assert_eq!(rig.next(), say("New."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
    }

    #[test]
    fn a_failure_mid_stream_closes_the_reply_like_a_stop() {
        let mut rig = rig();
        rig.speaker.fail.store(true, Ordering::SeqCst);
        rig.voice.say_delta("Broken. Rest of it");
        assert_eq!(rig.next(), VoiceEvent::Trouble("no voice".into()));
        assert!(!rig.voice.speaking());
        rig.speaker.fail.store(false, Ordering::SeqCst);
        rig.voice.say_delta(" goes on. Still.");
        rig.voice.say("Next."); // no end_of_reply for the failed one
        assert_eq!(rig.next(), say("Next."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert_eq!(rig.speaker.sentences(), ["Broken.", "Next."]);
    }

    #[test]
    fn a_streamed_code_fence_is_pointed_at_not_read() {
        let mut rig = rig();
        let text = "Here it is:\n```rust\nlet a = 1. ;\nfn b() {}\n```\nDone.";
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(7) {
            rig.voice.say_delta(&chunk.iter().collect::<String>());
        }
        rig.voice.end_of_reply();
        let mut said = Vec::new();
        loop {
            match rig.next() {
                VoiceEvent::Saying(t) => said.push(t),
                VoiceEvent::Finished => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        let joined = said.join(" ");
        assert_eq!(joined, "Here it is: (code is on screen) Done.");
        assert!(!joined.contains("fn") && !joined.contains("let"));
    }

    #[test]
    fn a_sink_that_fails_mid_reply_gives_one_trouble_instead_of_finished() {
        let mut rig = rig();
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say("One. Two.");
        assert_eq!(rig.next(), say("One."));
        assert_eq!(rig.next(), say("Two."));
        rig.sink.fail();
        assert_eq!(rig.next(), VoiceEvent::Trouble(SPEAKER_BROKE.into()));
        assert!(!rig.voice.speaking());
        rig.voice.stop(); // nothing left to stop: no event
        assert!(rig.nothing_pending());
    }

    #[test]
    fn a_clip_the_sink_refuses_ends_the_reply_with_trouble() {
        let mut rig = rig();
        rig.sink.fail();
        rig.voice.say_delta("Refused. And the rest. ");
        assert_eq!(rig.next(), VoiceEvent::Trouble(SPEAKER_BROKE.into()));
        assert!(!rig.voice.speaking());
        rig.voice.say_delta("Dropped too.");
        rig.voice.end_of_reply();
        // A new sink takes the next reply.
        let fresh = FakeSink::new();
        rig.voice.set_sink(fresh.clone());
        rig.voice.say("Again.");
        assert_eq!(rig.next(), say("Again."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert!(rig.nothing_pending());
        assert_eq!(fresh.clips.lock().unwrap().len(), 1);
        assert!(rig.sink.clips.lock().unwrap().is_empty());
        assert_eq!(rig.speaker.sentences(), ["Refused.", "Again."]);
    }

    #[test]
    fn stop_drops_the_rest_of_a_sounding_reply_and_the_one_queued_behind() {
        let (speaker, entered, release) = FakeSpeaker::gated();
        let mut rig = rig_with(speaker);
        rig.sink.idle.store(false, Ordering::SeqCst);
        rig.voice.say_delta("A one. A two.");
        entered.wait(); // A's first sentence is being synthesised, held
        rig.voice.end_of_reply();
        rig.voice.say("B one. B two.");
        rig.voice.stop();
        assert_eq!(rig.next(), VoiceEvent::Stopped);
        assert!(!rig.voice.speaking());
        release.wait();
        rig.sink.idle.store(true, Ordering::SeqCst);
        rig.voice.say("C.");
        assert_eq!(rig.next(), say("C."));
        assert_eq!(rig.next(), VoiceEvent::Finished);
        assert!(rig.nothing_pending());
        // Nothing from A or B reached the sink; the held sentence was thrown away.
        assert_eq!(rig.speaker.sentences(), ["A one.", "C."]);
        assert_eq!(rig.sink.clips.lock().unwrap().len(), 1);
    }
}
