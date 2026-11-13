//! Runs the real engines against the real models. Ignored by default: set
//! `DOTS_VOICE_MODELS` to a models folder in the manifest layout and run with `--ignored`.
//!
//! The session tests feed a fake microphone and pace the audio in real time (a frame every
//! 32 ms, as a microphone would), so they take minutes. Their hit rates over the ten voices
//! have floors: the wake phrase and the sentence after it at least 9 of 10, "Dot, stop" at
//! least 8 of 10, talking over Dot (when that is on) at least 9 of 10.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dots_voice::{
    resample, After, Clip, EarEvent, EarSettings, Ears, Mic, MicFrame, ModelPaths, Parts,
    Recognizer, Session, SessionEvent, SessionSettings, SherpaDetector, SherpaRecognizer,
    SherpaSpeaker, SherpaWake, Sink, Speaker, SpeechDetector, State, Voice, WakeSpotter, WakeWord,
    EAR_RATE, FRAME,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

fn paths() -> ModelPaths {
    let root = std::env::var_os("DOTS_VOICE_MODELS")
        .expect("set DOTS_VOICE_MODELS to the models folder to run these tests");
    ModelPaths { root: root.into() }
}

fn ears(clip: &Clip) -> Vec<f32> {
    resample(clip, EAR_RATE).samples
}

/// Feed `samples` frame by frame, then half a second of silence; the first word heard.
fn first_wake(wake: &mut SherpaWake, samples: &[f32]) -> Option<WakeWord> {
    wake.reset();
    let tail = vec![0.0f32; EAR_RATE as usize / 2];
    samples
        .chunks(FRAME)
        .chain(tail.chunks(FRAME))
        .map(|chunk| {
            let mut frame = chunk.to_vec();
            frame.resize(FRAME, 0.0);
            frame
        })
        .find_map(|frame| wake.heard(&frame))
}

/// The words of `text`, lower case, without punctuation.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// How many of the words of `said` are missing from `heard` (compared as words, without
/// case or punctuation).
fn words_missed(said: &str, heard: &str) -> usize {
    let heard = words(heard);
    words(said).iter().filter(|w| !heard.contains(w)).count()
}

/// The recogniser hears the speaker's sentence. One word may be heard as another (it once
/// heard "brief" as "breeze", one run in three): the sentence counts as heard when all its
/// words but one are there.
#[test]
#[ignore]
fn it_hears_what_it_says() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let recognizer = SherpaRecognizer::load(&paths).unwrap();
    assert_eq!(speaker.voices(), 10);
    let said = "I created a sol called Morning Brief.";
    for voice in [0u8, 5] {
        let clip = speaker.speak(said, voice, 1.0).unwrap();
        assert_eq!(clip.rate, 44_100);
        let heard = recognizer.transcribe(&clip).unwrap();
        assert!(words_missed(said, &heard) <= 1, "voice {voice}: {heard}");
    }
}

#[test]
#[ignore]
fn hey_dot_is_caught_and_near_misses_are_not() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let mut wake = SherpaWake::load(&paths).unwrap();
    let cases = [
        ("Hey Sage, what needs me?", Some(WakeWord::HeyDot)),
        (
            "Hey Sam, I saved the message on that page.",
            None::<WakeWord>,
        ),
        ("Sage, stop.", Some(WakeWord::DotStop)),
    ];
    let mut wrong = Vec::new();
    for voice in [0u8, 3, 7] {
        for (sentence, want) in cases {
            let samples = ears(&speaker.speak(sentence, voice, 1.0).unwrap());
            let got = first_wake(&mut wake, &samples);
            if got != want {
                wrong.push(format!(
                    "voice {voice}: {sentence:?} wanted {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
#[ignore]
fn silence_is_not_speech_and_speech_is() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let mut detector = SherpaDetector::load(&paths).unwrap();

    let silent = vec![0.0f32; EAR_RATE as usize];
    let in_silence = silent
        .chunks_exact(FRAME)
        .filter(|f| detector.is_speech(f))
        .count();
    assert_eq!(in_silence, 0);

    detector.reset();
    let samples = ears(
        &speaker
            .speak("Please read me the first message.", 0, 1.0)
            .unwrap(),
    );
    let in_speech = samples
        .chunks_exact(FRAME)
        .filter(|f| detector.is_speech(f))
        .count();
    assert!(in_speech >= 10, "only {in_speech} speech frames");
}

#[test]
#[ignore]
fn a_short_sentence_is_ready_fast() {
    let speaker = SherpaSpeaker::load(&paths(), 2).unwrap();
    speaker.speak("Warming up.", 0, 1.0).unwrap();
    let start = Instant::now();
    let clip = speaker.speak("Okay, I'm on it.", 0, 1.0).unwrap();
    let took = start.elapsed();
    assert!(!clip.samples.is_empty());
    println!("short sentence ready in {} ms", took.as_millis());
    assert!(took.as_millis() < 1500, "took {} ms", took.as_millis());
}

/// Not a pass/fail gate for the hit rate, but near-misses must never wake it. Prints the rates.
#[test]
#[ignore]
fn wake_hit_rates_over_all_voices() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let mut wake = SherpaWake::load(&paths).unwrap();
    let mut rates = Vec::new();
    for (sentence, want) in [
        ("Hey Sage, what needs me?", Some(WakeWord::HeyDot)),
        ("Sage, stop.", Some(WakeWord::DotStop)),
        ("Hey Sam, I saved the message on that page.", None),
        (
            "Please stop the music before the dog gets a hot donut.",
            None,
        ),
        // These two woke it now and then with the wake phrase boosted more (see
        // `WAKE_KEYWORDS`).
        ("Hey, say that again tomorrow.", None),
        ("Hey, save each page of the message.", None),
    ] {
        let hits = (0..speaker.voices())
            .filter(|&v| {
                first_wake(&mut wake, &ears(&speaker.speak(sentence, v, 1.0).unwrap())) == want
            })
            .count();
        println!("{hits}/{} right for {sentence:?}", speaker.voices());
        rates.push((want, hits));
    }
    for (want, hits) in rates {
        if want.is_none() {
            assert_eq!(hits, usize::from(speaker.voices()), "a near miss woke it");
        }
    }
}

/// The longest unbroken run of speech frames the real detector finds in `samples`.
fn longest_speech_run(detector: &mut SherpaDetector, samples: &[f32]) -> usize {
    detector.reset();
    let (mut run, mut longest) = (0, 0);
    for frame in samples.chunks_exact(FRAME) {
        if detector.is_speech(frame) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// One-word answers, as given after a question, go through the real detector and the
/// real recogniser with the short-answer settings and come out as a `Final`, in every voice.
/// Prints the longest speech run of each, in frames of 32 ms, which is what
/// `min_speech_ms` is judged against.
///
/// The speaker's output varies from call to call. About one clip in 200 has no speech the
/// detector can find at all (a run of 0); no `min_speech_ms` helps that, so such a clip is
/// made again (and counted) rather than failing the test.
#[test]
#[ignore]
fn one_word_answers_are_heard_with_the_short_answer_settings() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let recognizer: Arc<dyn Recognizer> = Arc::new(SherpaRecognizer::load(&paths).unwrap());
    let mut short = Ears::new(
        Box::new(SherpaDetector::load(&paths).unwrap()),
        recognizer,
        EarSettings::short_answer(),
    );
    let mut probe = SherpaDetector::load(&paths).unwrap();
    let lead = vec![0.0f32; EAR_RATE as usize * 3 / 10];
    let tail = vec![0.0f32; EAR_RATE as usize];
    let (mut wrong, mut unseen, mut shortest) = (Vec::new(), 0, usize::MAX);
    for voice in 0..speaker.voices() {
        for word in ["Yes.", "No.", "Stop."] {
            let (all, run) = (0..3)
                .map(|_| {
                    let speech = ears(&speaker.speak(word, voice, 1.0).unwrap());
                    let all: Vec<f32> = lead.iter().chain(&speech).chain(&tail).copied().collect();
                    let run = longest_speech_run(&mut probe, &all);
                    (all, run)
                })
                .inspect(|(_, run)| unseen += usize::from(*run == 0))
                .find(|(_, run)| *run > 0)
                .expect("the detector found speech in one of three tries");
            shortest = shortest.min(run);
            short.reset();
            let (mut finals, mut dropped) = (Vec::new(), 0);
            for frame in all.chunks_exact(FRAME) {
                for event in short.push(frame) {
                    match event {
                        EarEvent::Final(text) => finals.push(text),
                        EarEvent::Dropped => dropped += 1,
                        EarEvent::Started | EarEvent::Partial(_) => {}
                    }
                }
            }
            println!(
                "voice {voice} {word:?}: longest run {run} frames ({} ms), finals {finals:?}",
                run * 32
            );
            let want = word.trim_end_matches('.').to_lowercase();
            if !finals.iter().any(|f| f.to_lowercase().contains(&want)) {
                wrong.push(format!(
                    "voice {voice}: {word:?} gave {finals:?} ({dropped} dropped, run {run})"
                ));
            }
        }
    }
    println!("shortest run {shortest} frames; clips with no speech found, made again: {unseen}");
    assert!(
        wrong.is_empty(),
        "{}",
        wrong.join(
            "
"
        )
    );
}

#[test]
#[ignore]
fn an_empty_clip_hears_nothing() {
    let recognizer = SherpaRecognizer::load(&paths()).unwrap();
    let empty = Clip {
        samples: Vec::new(),
        rate: 16_000,
    };
    assert_eq!(recognizer.transcribe(&empty).unwrap(), "");
    let short = Clip {
        samples: vec![0.0; 800],
        rate: 16_000,
    };
    assert_eq!(recognizer.transcribe(&short).unwrap(), "");
}

// ---- the real session, fed by a fake microphone ---------------------------------------------

/// The session's settings for these tests. Built field by field on purpose: anything added
/// later stays at its default unless set here. The listening tone is off, so the ears open
/// as soon as listening starts and these tests measure what they measured before it.
fn session_settings(wake_phrase: bool, interrupt_by_talking: bool) -> SessionSettings {
    SessionSettings {
        wake_phrase,
        interrupt_by_talking,
        tone: false,
        ..SessionSettings::default()
    }
}

/// A microphone whose frames the test sends. Each open makes a new stream; the newest one's
/// sender is kept in the slot, and frames sent while it is closed are lost, as on a real one.
type MicSlot = Arc<Mutex<Option<Sender<MicFrame>>>>;

struct FakeMic(Option<Receiver<MicFrame>>);

impl Mic for FakeMic {
    fn frames(&mut self) -> Receiver<MicFrame> {
        self.0.take().unwrap_or_else(|| channel().1)
    }
}

/// A speaker that plays nothing: a clip keeps it busy for as long as it would sound.
#[derive(Default)]
struct FakeSink {
    busy_until: Mutex<Option<Instant>>,
}

impl Sink for FakeSink {
    fn play(&self, clip: Clip) -> bool {
        let length = Duration::from_secs_f64(clip.samples.len() as f64 / f64::from(clip.rate));
        let mut busy = self.busy_until.lock().unwrap();
        let from = busy
            .filter(|t| *t > Instant::now())
            .unwrap_or_else(Instant::now);
        *busy = Some(from + length);
        true
    }
    fn stop(&self) {
        *self.busy_until.lock().unwrap() = None;
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

/// The engines loaded once and shared by every session.
struct Engines {
    paths: ModelPaths,
    speaker: Arc<SherpaSpeaker>,
    recognizer: Arc<dyn Recognizer>,
}

impl Engines {
    fn load() -> Self {
        let paths = paths();
        Self {
            speaker: Arc::new(SherpaSpeaker::load(&paths, 2).unwrap()),
            recognizer: Arc::new(SherpaRecognizer::load(&paths).unwrap()),
            paths,
        }
    }

    /// `text` in `voice`, at the microphone's rate.
    fn said(&self, text: &str, voice: u8) -> Vec<f32> {
        ears(&self.speaker.speak(text, voice, 1.0).unwrap())
    }

    /// A real session on the real engines, with a fake microphone and a fake speaker.
    fn session(&self, settings: SessionSettings) -> Live {
        let paths = &self.paths;
        let sink = Arc::new(FakeSink::default());
        let (voice_tx, voice_events) = unbounded_channel();
        let speaker: Arc<dyn Speaker> = self.speaker.clone();
        let voice = Voice::new(speaker, sink.clone(), voice_tx);
        let slot: MicSlot = Arc::default();
        let opener = slot.clone();
        let reopened = sink.clone();
        let parts = Parts {
            ears: Ears::new(
                Box::new(SherpaDetector::load(paths).unwrap()),
                self.recognizer.clone(),
                EarSettings::default(),
            ),
            short_ears: Ears::new(
                Box::new(SherpaDetector::load(paths).unwrap()),
                self.recognizer.clone(),
                EarSettings::short_answer(),
            ),
            voice,
            voice_events,
            sink: sink.clone(),
            open_sink: Box::new(move || Ok(reopened.clone() as Arc<dyn Sink>)),
            wake: Some(Box::new(SherpaWake::load(paths).unwrap())),
            open_mic: Box::new(move || {
                let (tx, rx) = channel();
                *opener.lock().unwrap() = Some(tx);
                Ok(Box::new(FakeMic(Some(rx))) as Box<dyn Mic>)
            }),
        };
        let (events_tx, events) = unbounded_channel();
        let session = Session::start(parts, settings, events_tx);
        Live {
            session,
            events,
            mic: slot,
            sink,
        }
    }
}

struct Live {
    session: Session,
    events: UnboundedReceiver<SessionEvent>,
    mic: MicSlot,
    sink: Arc<FakeSink>,
}

impl Live {
    /// The first event from now on matching `want`, waiting up to `limit`.
    fn wait(
        &mut self,
        limit: Duration,
        mut want: impl FnMut(&SessionEvent) -> bool,
    ) -> Option<SessionEvent> {
        let end = Instant::now() + limit;
        loop {
            while let Ok(ev) = self.events.try_recv() {
                if want(&ev) {
                    return Some(ev);
                }
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_state(&mut self, limit: Duration, state: State) -> bool {
        self.wait(
            limit,
            |ev| matches!(ev, SessionEvent::State { state: s, .. } if *s == state),
        )
        .is_some()
    }

    /// Feeds `samples` to the open microphone in frames, at the pace a microphone delivers
    /// them, then `tail_ms` of silence.
    fn feed(&mut self, samples: &[f32], tail_ms: u64) {
        let tail = vec![0.0f32; (u64::from(EAR_RATE) * tail_ms / 1000) as usize];
        let frame_time = Duration::from_secs_f64(FRAME as f64 / f64::from(EAR_RATE));
        let started = Instant::now();
        for (n, chunk) in samples.chunks(FRAME).chain(tail.chunks(FRAME)).enumerate() {
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

    /// Starts a turn by a click and speaks a long reply to it; true once Dot is well into it
    /// with plenty still to say. (The fake speaker can be quiet for a moment between two
    /// sentences while the next is made, so the session's state is what tells.)
    fn speaking_a_long_reply(&mut self) -> bool {
        let turn = self.session.answered("Tell me about the week.").unwrap();
        if self
            .wait(Duration::from_secs(5), |ev| {
                matches!(ev, SessionEvent::Heard { .. })
            })
            .is_none()
        {
            return false;
        }
        self.session.say(turn, LONG_REPLY, After::Rest);
        let speaking = self.wait_state(Duration::from_secs(10), State::Speaking);
        std::thread::sleep(Duration::from_millis(800));
        speaking && self.session.state() == State::Speaking
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.session.shutdown();
    }
}

const LONG_REPLY: &str = "Here is your week, with everything your sols did for you from \
     Monday to Friday. On Monday Research looked at three new papers \
     about batteries and wrote a short summary for you. On Tuesday the mail sol sorted forty \
     messages and found two that need an answer. On Wednesday the build sol ran the tests \
     twice and both runs passed. On Thursday nothing new came in. On Friday Research asked \
     whether it should keep watching the battery papers every morning.";

/// Waits for the resting session to open its microphone for the wake phrase.
fn resting_with_the_mic_open(live: &mut Live) -> bool {
    live.wait(Duration::from_secs(10), |ev| {
        matches!(
            ev,
            SessionEvent::State {
                state: State::Idle,
                mic_open: true
            }
        )
    })
    .is_some()
}

/// "Hey Dot." then a sentence, said to the real session: `Woke`, then `Heard` with that
/// sentence and not the wake phrase. Prints the hit counts over the ten voices; each must be
/// at least 9 of 10. Paced in real time.
#[test]
#[ignore]
fn hey_dot_then_a_sentence_wakes_the_session_and_is_heard() {
    let engines = Engines::load();
    let (mut woke, mut heard, mut misses) = (0, 0, Vec::new());
    for voice in 0..engines.speaker.voices() {
        let wake = engines.said("Hey Sage.", voice);
        let sentence = engines.said("Please read me the first message.", voice);
        let mut live = engines.session(session_settings(true, false));
        assert!(
            resting_with_the_mic_open(&mut live),
            "voice {voice}: no mic"
        );
        live.feed(&[], 300);
        // A real microphone never stops: the spotter decodes in chunks and, for some voices,
        // names a phrase said on its own only once about a second more of sound has come in.
        live.feed(&wake, 1500);
        let got_woke = live
            .wait(Duration::from_secs(2), |ev| *ev == SessionEvent::Woke)
            .is_some();
        if !got_woke {
            misses.push(format!("voice {voice}: no Woke"));
            continue;
        }
        woke += 1;
        live.feed(&sentence, 1500);
        let text = live.wait(Duration::from_secs(10), |ev| {
            matches!(ev, SessionEvent::Heard { .. })
        });
        match text {
            Some(SessionEvent::Heard { text, .. }) => {
                let lower = text.to_lowercase();
                if lower.contains("first message") && !lower.contains("hey") {
                    heard += 1;
                } else {
                    misses.push(format!("voice {voice}: heard {text:?}"));
                }
            }
            _ => misses.push(format!("voice {voice}: no Heard")),
        }
    }
    let voices = engines.speaker.voices();
    println!("woke {woke}/{voices}, then heard the sentence {heard}/{voices}");
    for miss in &misses {
        println!("  {miss}");
    }
    let floor = usize::from(voices) * 9 / 10;
    assert!(woke >= floor, "the wake phrase woke it {woke}/{voices}");
    assert!(
        heard >= floor,
        "the sentence after the wake phrase was heard {heard}/{voices}"
    );
}

/// `samples` without the near-silence at either end (below 2% of the loudest sample).
fn trimmed(samples: &[f32]) -> &[f32] {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let loud = |s: &f32| s.abs() > peak * 0.02;
    let start = samples.iter().position(loud).unwrap_or(0);
    let end = samples.iter().rposition(loud).map_or(start, |i| i + 1);
    &samples[start..end]
}

/// "Hey Dot." and a sentence in one breath (no silence fed between them), with the listening
/// tone on: `Woke`, then `Heard` with the whole sentence, since the ears open at once after
/// the wake phrase and the tone waits for a pause that never comes. Paced in real time.
///
/// The speaker makes each clip anew and the spotter and the recogniser each miss a few in a
/// hundred, so a voice that misses is said once more: this test is about the session passing
/// the words on whole, not about how well the engines hear. Prints the hits on the first try
/// and after the second over the ten voices; the second must be at least 9 of 10.
#[test]
#[ignore]
fn hey_dot_and_a_sentence_in_one_breath_are_heard_whole_with_the_tone_on() {
    let engines = Engines::load();
    let said = "Please read me the first message.";
    let once = |voice: u8| -> Result<(), String> {
        // The speaker pads each clip with silence; without it the two run together.
        let mut breath = trimmed(&engines.said("Hey Sage.", voice)).to_vec();
        breath.extend_from_slice(trimmed(&engines.said(said, voice)));
        let mut live = engines.session(SessionSettings {
            tone: true,
            ..session_settings(true, false)
        });
        assert!(
            resting_with_the_mic_open(&mut live),
            "voice {voice}: no mic"
        );
        live.feed(&[], 300);
        live.feed(&breath, 1500);
        let mut woke = false;
        let text = live.wait(Duration::from_secs(10), |ev| {
            woke |= *ev == SessionEvent::Woke;
            matches!(ev, SessionEvent::Heard { .. })
        });
        match text {
            Some(SessionEvent::Heard { text, .. }) if woke => {
                let got = words(&text);
                if words_missed(said, &text) == 0 && !got.contains(&"hey".to_string()) {
                    Ok(())
                } else {
                    Err(format!("voice {voice}: heard {text:?}"))
                }
            }
            Some(_) => Err(format!("voice {voice}: no Woke")),
            None => Err(format!("voice {voice}: no Heard (woke: {woke})")),
        }
    };
    let (mut first, mut heard, mut misses) = (0, 0, Vec::new());
    for voice in 0..engines.speaker.voices() {
        match once(voice) {
            Ok(()) => {
                first += 1;
                heard += 1;
            }
            Err(miss) => {
                misses.push(format!("{miss} (first try)"));
                match once(voice) {
                    Ok(()) => heard += 1,
                    Err(miss) => misses.push(format!("{miss} (second try)")),
                }
            }
        }
    }
    let voices = engines.speaker.voices();
    println!(
        "in one breath with the tone on, the whole sentence was heard {first}/{voices} on the \
         first try, {heard}/{voices} after a second"
    );
    for miss in &misses {
        println!("  {miss}");
    }
    assert!(
        heard >= usize::from(voices) * 9 / 10,
        "the sentence in the same breath was heard whole {heard}/{voices}"
    );
}

/// While a long reply is spoken, "Dot, stop." said to the microphone stops it: the session
/// listens again and the speaker is quiet. Prints the hit count over the ten voices, which
/// must be at least 8 of 10. Paced in real time.
#[test]
#[ignore]
fn dot_stop_stops_a_long_reply() {
    let engines = Engines::load();
    let (mut stopped, mut misses) = (0, Vec::new());
    for voice in 0..engines.speaker.voices() {
        let stop = engines.said("Sage, stop.", voice);
        let mut live = engines.session(session_settings(true, false));
        assert!(
            resting_with_the_mic_open(&mut live),
            "voice {voice}: no mic"
        );
        assert!(live.speaking_a_long_reply(), "voice {voice}: not speaking");
        live.feed(&stop, 500);
        if live.wait_state(Duration::from_secs(2), State::Listening) && live.sink.idle() {
            stopped += 1;
        } else {
            misses.push(format!("voice {voice}: still speaking"));
        }
    }
    let voices = engines.speaker.voices();
    println!("\"Dot, stop.\" stopped the reply {stopped}/{voices}");
    for miss in &misses {
        println!("  {miss}");
    }
    assert!(
        stopped >= usize::from(voices) * 8 / 10,
        "\"Dot, stop.\" stopped the reply {stopped}/{voices}"
    );
}

/// With "Interrupt by just talking" on, any sentence stops a long reply; with it off (and the
/// wake phrase on, so the microphone is open), a sentence that is not the stop phrase does
/// not. Prints the hit counts over the ten voices: on, at least 9 of 10 stop it; off, none
/// may. Paced in real time.
#[test]
#[ignore]
fn talking_over_dot_stops_it_only_when_that_is_on() {
    let engines = Engines::load();
    let (mut stopped_on, mut kept_off, mut misses) = (0, 0, Vec::new());
    for voice in 0..engines.speaker.voices() {
        let sentence = engines.said("Please read me the first message.", voice);
        // On: the microphone opens for speaking alone.
        let mut live = engines.session(session_settings(false, true));
        assert!(live.speaking_a_long_reply(), "voice {voice}: not speaking");
        live.feed(&sentence, 500);
        if live.wait_state(Duration::from_secs(2), State::Listening) {
            stopped_on += 1;
        } else {
            misses.push(format!("voice {voice}, on: still speaking"));
        }
        drop(live);
        // Off, with the wake phrase on.
        let mut live = engines.session(session_settings(true, false));
        assert!(
            resting_with_the_mic_open(&mut live),
            "voice {voice}: no mic"
        );
        assert!(live.speaking_a_long_reply(), "voice {voice}: not speaking");
        live.feed(&sentence, 500);
        let cut = live.wait_state(Duration::from_secs(1), State::Listening);
        if !cut && live.session.state() == State::Speaking {
            kept_off += 1;
        } else {
            misses.push(format!("voice {voice}, off: the reply stopped"));
        }
    }
    let voices = engines.speaker.voices();
    println!(
        "talking over Sage, on: stopped {stopped_on}/{voices}; off: kept speaking {kept_off}/{voices}"
    );
    for miss in &misses {
        println!("  {miss}");
    }
    assert_eq!(
        kept_off,
        usize::from(voices),
        "a sentence stopped Sage with the setting off"
    );
    assert!(
        stopped_on >= usize::from(voices) * 9 / 10,
        "talking over Sage stopped it {stopped_on}/{voices}"
    );
}
