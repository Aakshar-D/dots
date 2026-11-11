//! Decides when the person started and stopped talking, one microphone frame at a time.
//!
//! Pure logic over the engine traits. The audio of a clip lives in memory only until the
//! clip ends; nothing is written to disk or to logs.

use crate::engine::{Clip, Recognizer, SpeechDetector, EAR_RATE, FRAME};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one `FRAME` lasts, in milliseconds.
const FRAME_MS: u32 = (FRAME as u32 * 1000) / EAR_RATE;

/// Whole frames that cover `ms`, rounded up.
fn frames_for(ms: u32) -> usize {
    ms.div_ceil(FRAME_MS) as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EarSettings {
    /// Quiet that ends a sentence.
    pub end_quiet_ms: u32,
    /// Less speech than this in a clip is a cough or a click, not a sentence.
    pub min_speech_ms: u32,
    /// Audio kept from before the first speech frame, so the first word is not clipped.
    pub pre_roll_ms: u32,
    /// How often a growing clip is transcribed for a live preview.
    pub partial_every_ms: u32,
    /// A clip is ended at this length even if the person is still talking.
    pub max_clip_ms: u32,
}

impl Default for EarSettings {
    fn default() -> Self {
        Self {
            end_quiet_ms: 700,
            min_speech_ms: 250,
            pre_roll_ms: 300,
            partial_every_ms: 400,
            max_clip_ms: 30_000,
        }
    }
}

impl EarSettings {
    /// For one-word answers such as yes or no: the sentence ends sooner.
    pub fn short_answer() -> Self {
        Self {
            end_quiet_ms: 450,
            ..Self::default()
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum EarEvent {
    Started,
    Partial(String),
    Final(String),
    Dropped,
}

pub struct Ears {
    detector: Box<dyn SpeechDetector>,
    recognizer: Arc<dyn Recognizer>,
    end_quiet: usize,
    min_speech: usize,
    pre_roll: usize,
    partial_every: usize,
    max_clip: usize,
    partial_budget: Duration,
    ring: VecDeque<Vec<f32>>,
    clip: Vec<f32>,
    in_clip: bool,
    speech_frames: usize,
    quiet_run: usize,
    speech_since_partial: usize,
    last_partial: String,
    /// How long the last partial took; a slow one makes the next interval skip.
    last_partial_cost: Option<Duration>,
}

impl Ears {
    pub fn new(
        detector: Box<dyn SpeechDetector>,
        recognizer: Arc<dyn Recognizer>,
        settings: EarSettings,
    ) -> Self {
        Self {
            detector,
            recognizer,
            end_quiet: frames_for(settings.end_quiet_ms).max(1),
            min_speech: frames_for(settings.min_speech_ms),
            pre_roll: frames_for(settings.pre_roll_ms),
            partial_every: frames_for(settings.partial_every_ms).max(1),
            max_clip: frames_for(settings.max_clip_ms).max(1),
            partial_budget: Duration::from_millis(u64::from(settings.partial_every_ms)),
            ring: VecDeque::new(),
            clip: Vec::new(),
            in_clip: false,
            speech_frames: 0,
            quiet_run: 0,
            speech_since_partial: 0,
            last_partial: String::new(),
            last_partial_cost: None,
        }
    }

    /// Feed exactly one `FRAME` of 16 kHz audio. Transcribes on the calling thread.
    pub fn push(&mut self, frame16k: &[f32]) -> Vec<EarEvent> {
        debug_assert_eq!(frame16k.len(), FRAME, "ears take exactly one frame");
        if frame16k.len() != FRAME {
            return Vec::new();
        }
        let speech = self.detector.is_speech(frame16k);
        let mut events = Vec::new();

        if !self.in_clip {
            if speech {
                self.begin(frame16k);
                events.push(EarEvent::Started);
            } else if self.pre_roll > 0 {
                if self.ring.len() >= self.pre_roll {
                    self.ring.pop_front();
                }
                self.ring.push_back(frame16k.to_vec());
            }
            return events;
        }

        self.clip.extend_from_slice(frame16k);
        if speech {
            self.speech_frames += 1;
            self.quiet_run = 0;
            self.speech_since_partial += 1;
        } else {
            self.quiet_run += 1;
        }

        let clip_frames = self.clip.len() / FRAME;
        if self.quiet_run >= self.end_quiet || clip_frames >= self.max_clip {
            events.push(self.finish());
        } else if speech && self.speech_since_partial >= self.partial_every {
            self.speech_since_partial = 0;
            if let Some(text) = self.partial() {
                events.push(EarEvent::Partial(text));
            }
        }
        events
    }

    /// Forget the clip in progress (no `Final` follows), the pre-roll and the detector.
    pub fn reset(&mut self) {
        self.ring.clear();
        self.end_clip();
        self.detector.reset();
    }

    fn begin(&mut self, first_speech_frame: &[f32]) {
        self.clip.clear();
        for frame in self.ring.drain(..) {
            self.clip.extend_from_slice(&frame);
        }
        self.clip.extend_from_slice(first_speech_frame);
        self.in_clip = true;
        self.speech_frames = 1;
        self.quiet_run = 0;
        self.speech_since_partial = 0;
        self.last_partial.clear();
        self.last_partial_cost = None;
    }

    fn end_clip(&mut self) {
        self.clip = Vec::new(); // let go of the memory, not just the length
        self.in_clip = false;
        self.speech_frames = 0;
        self.quiet_run = 0;
        self.speech_since_partial = 0;
        self.last_partial.clear();
        self.last_partial_cost = None;
    }

    fn clip_audio(&self) -> Clip {
        Clip {
            samples: self.clip.clone(),
            rate: EAR_RATE,
        }
    }

    /// A preview of the clip so far, if it is worth sending: the transcription must work,
    /// differ from the last preview, and not be late. A machine too slow for the interval
    /// skips every other one rather than falling behind.
    fn partial(&mut self) -> Option<String> {
        if self
            .last_partial_cost
            .is_some_and(|cost| cost > self.partial_budget)
        {
            self.last_partial_cost = None; // skip this one, try again next time
            return None;
        }
        let started = Instant::now();
        let result = self.recognizer.transcribe(&self.clip_audio());
        self.last_partial_cost = Some(started.elapsed());
        let text = result.ok()?.trim().to_string();
        if text.is_empty() || text == self.last_partial {
            return None;
        }
        self.last_partial = text.clone();
        Some(text)
    }

    fn finish(&mut self) -> EarEvent {
        let enough = self.speech_frames >= self.min_speech;
        let event = if enough {
            match self.recognizer.transcribe(&self.clip_audio()) {
                Ok(text) if !text.trim().is_empty() => EarEvent::Final(text.trim().to_string()),
                _ => EarEvent::Dropped,
            }
        } else {
            EarEvent::Dropped
        };
        self.end_clip();
        event
    }
}

/// How loud a frame is, from 0.0 (-60 dBFS or quieter) to 1.0 (-10 dBFS or louder),
/// linear in decibels between.
pub fn level(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let mean_square = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
    let db = 10.0 * mean_square.log10(); // 20 * log10(rms)
    if db.is_nan() {
        return 0.0;
    }
    ((db + 60.0) / 50.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A frame is speech when its first sample is positive, so a test says what each
    /// frame is by what it feeds. The `tag` tells the audio of one sentence from another.
    struct FrameDetector {
        resets: Arc<AtomicUsize>,
    }

    impl SpeechDetector for FrameDetector {
        fn is_speech(&mut self, frame16k: &[f32]) -> bool {
            frame16k[0] > 0.0
        }
        fn reset(&mut self) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Clone, Copy)]
    enum Mode {
        Count,
        Empty,
        Fail,
        Slow,
    }

    struct FakeRecognizer {
        mode: Mode,
        clips: Mutex<Vec<Clip>>,
    }

    impl Recognizer for FakeRecognizer {
        fn transcribe(&self, clip: &Clip) -> Result<String, EngineError> {
            self.clips.lock().unwrap().push(clip.clone());
            match self.mode {
                Mode::Count => Ok(format!("{} frames", clip.samples.len() / FRAME)),
                Mode::Empty => Ok(String::new()),
                Mode::Fail => Err(EngineError::Failed("no".into())),
                Mode::Slow => {
                    std::thread::sleep(Duration::from_millis(5));
                    Ok(format!("{} frames", clip.samples.len() / FRAME))
                }
            }
        }
    }

    struct Rig {
        ears: Ears,
        recognizer: Arc<FakeRecognizer>,
        resets: Arc<AtomicUsize>,
    }

    fn rig(mode: Mode, settings: EarSettings) -> Rig {
        let resets = Arc::new(AtomicUsize::new(0));
        let recognizer = Arc::new(FakeRecognizer {
            mode,
            clips: Mutex::new(Vec::new()),
        });
        let ears = Ears::new(
            Box::new(FrameDetector {
                resets: resets.clone(),
            }),
            recognizer.clone(),
            settings,
        );
        Rig {
            ears,
            recognizer,
            resets,
        }
    }

    fn feed(ears: &mut Ears, speech: bool, frames: usize, tag: f32) -> Vec<EarEvent> {
        let value = if speech { tag } else { -tag };
        let frame = vec![value; FRAME];
        (0..frames).flat_map(|_| ears.push(&frame)).collect()
    }

    fn finals(events: &[EarEvent]) -> Vec<&EarEvent> {
        events
            .iter()
            .filter(|e| matches!(e, EarEvent::Final(_)))
            .collect()
    }

    #[test]
    fn quiet_makes_no_events() {
        let mut r = rig(Mode::Count, EarSettings::default());
        assert!(feed(&mut r.ears, false, 200, 1.0).is_empty());
        assert!(r.recognizer.clips.lock().unwrap().is_empty());
    }

    #[test]
    fn speech_then_quiet_makes_started_partials_final() {
        let mut r = rig(Mode::Count, EarSettings::default());
        let mut events = feed(&mut r.ears, false, 20, 1.0);
        events.extend(feed(&mut r.ears, true, 40, 1.0));
        events.extend(feed(&mut r.ears, false, 30, 1.0));
        assert_eq!(events.first(), Some(&EarEvent::Started));
        // pre-roll 10 + speech 40 + quiet 22 until the end
        assert_eq!(events.last(), Some(&EarEvent::Final("72 frames".into())));
        let middle = &events[1..events.len() - 1];
        assert!(!middle.is_empty());
        assert!(middle.iter().all(|e| matches!(e, EarEvent::Partial(_))));
        assert_eq!(finals(&events).len(), 1);
    }

    #[test]
    fn speech_in_the_very_first_frame_has_no_pre_roll() {
        let mut r = rig(Mode::Count, EarSettings::default());
        let mut events = feed(&mut r.ears, true, 20, 1.0);
        events.extend(feed(&mut r.ears, false, 30, 1.0));
        assert_eq!(events.first(), Some(&EarEvent::Started));
        assert_eq!(events.last(), Some(&EarEvent::Final("42 frames".into())));
    }

    #[test]
    fn a_cough_is_dropped() {
        let mut r = rig(Mode::Count, EarSettings::default());
        let mut events = feed(&mut r.ears, false, 20, 1.0);
        events.extend(feed(&mut r.ears, true, 5, 1.0));
        events.extend(feed(&mut r.ears, false, 40, 1.0));
        assert_eq!(events, vec![EarEvent::Started, EarEvent::Dropped]);
    }

    #[test]
    fn a_pause_shorter_than_the_end_quiet_does_not_end_the_sentence() {
        let mut r = rig(Mode::Count, EarSettings::default());
        let mut events = feed(&mut r.ears, true, 20, 1.0);
        events.extend(feed(&mut r.ears, false, 19, 1.0)); // 600 ms
        events.extend(feed(&mut r.ears, true, 20, 1.0));
        events.extend(feed(&mut r.ears, false, 30, 1.0));
        assert_eq!(finals(&events).len(), 1);
        assert_eq!(
            events.iter().filter(|e| **e == EarEvent::Started).count(),
            1
        );
    }

    #[test]
    fn short_answer_settings_end_sooner() {
        let script = |ears: &mut Ears| {
            let mut events = feed(ears, true, 20, 1.0);
            events.extend(feed(ears, false, 16, 1.0)); // 500 ms
            events.extend(feed(ears, true, 20, 1.0));
            events.extend(feed(ears, false, 30, 1.0));
            events
        };
        let mut normal = rig(Mode::Count, EarSettings::default());
        assert_eq!(finals(&script(&mut normal.ears)).len(), 1);
        let mut short = rig(Mode::Count, EarSettings::short_answer());
        assert_eq!(finals(&script(&mut short.ears)).len(), 2);
    }

    #[test]
    fn a_very_long_clip_is_ended_at_the_cap() {
        let settings = EarSettings {
            max_clip_ms: 640, // 20 frames
            pre_roll_ms: 0,
            ..EarSettings::default()
        };
        let mut r = rig(Mode::Count, settings);
        let events = feed(&mut r.ears, true, 50, 1.0);
        assert!(events.contains(&EarEvent::Final("20 frames".into())));
        let longest = r
            .recognizer
            .clips
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.samples.len() / FRAME)
            .max()
            .unwrap();
        assert!(longest <= 20);
    }

    #[test]
    fn an_empty_transcript_is_dropped() {
        let mut r = rig(Mode::Empty, EarSettings::default());
        let mut events = feed(&mut r.ears, true, 30, 1.0);
        events.extend(feed(&mut r.ears, false, 30, 1.0));
        assert_eq!(events, vec![EarEvent::Started, EarEvent::Dropped]);
    }

    #[test]
    fn a_failing_recognizer_gives_no_partials_and_a_dropped_end() {
        let mut r = rig(Mode::Fail, EarSettings::default());
        let mut events = feed(&mut r.ears, true, 40, 1.0);
        events.extend(feed(&mut r.ears, false, 30, 1.0));
        assert_eq!(events, vec![EarEvent::Started, EarEvent::Dropped]);
    }

    #[test]
    fn a_slow_recognizer_gets_fewer_partials() {
        let settings = EarSettings {
            partial_every_ms: 1, // every frame, but each run takes longer than that
            ..EarSettings::default()
        };
        let mut r = rig(Mode::Slow, settings);
        let events = feed(&mut r.ears, true, 21, 1.0);
        let partials = events
            .iter()
            .filter(|e| matches!(e, EarEvent::Partial(_)))
            .count();
        assert!((1..=12).contains(&partials), "{partials} partials");
    }

    #[test]
    fn reset_in_the_middle_of_a_clip_ends_it_without_a_final() {
        let mut r = rig(Mode::Count, EarSettings::default());
        let events = feed(&mut r.ears, true, 20, 1.0);
        assert_eq!(events.first(), Some(&EarEvent::Started));
        r.ears.reset();
        assert_eq!(r.resets.load(Ordering::SeqCst), 1);
        assert!(feed(&mut r.ears, false, 40, 1.0).is_empty());
        let mut again = feed(&mut r.ears, true, 20, 1.0);
        again.extend(feed(&mut r.ears, false, 30, 1.0));
        assert_eq!(again.first(), Some(&EarEvent::Started));
        assert_eq!(finals(&again).len(), 1);
    }

    #[test]
    fn two_sentences_make_two_finals_and_the_second_clip_is_its_own() {
        let mut r = rig(Mode::Count, EarSettings::default());
        let mut events = feed(&mut r.ears, true, 20, 1.0);
        events.extend(feed(&mut r.ears, false, 30, 3.0)); // longer than the end quiet
        events.extend(feed(&mut r.ears, true, 20, 2.0));
        events.extend(feed(&mut r.ears, false, 30, 2.0));
        assert_eq!(finals(&events).len(), 2);
        let clips = r.recognizer.clips.lock().unwrap();
        let second = clips.last().unwrap();
        assert!(second.samples.iter().all(|s| s.abs() != 1.0));
        assert!(second.samples.iter().any(|s| s.abs() == 2.0));
        assert_eq!(second.rate, EAR_RATE);
    }

    #[test]
    fn a_frame_of_the_wrong_size_is_ignored() {
        if cfg!(debug_assertions) {
            return; // debug builds assert instead
        }
        let mut r = rig(Mode::Count, EarSettings::default());
        assert!(r.ears.push(&[1.0; 10]).is_empty());
    }

    #[test]
    fn settings_have_the_agreed_values() {
        let d = EarSettings::default();
        assert_eq!(
            (
                d.end_quiet_ms,
                d.min_speech_ms,
                d.pre_roll_ms,
                d.partial_every_ms,
                d.max_clip_ms
            ),
            (700, 250, 300, 400, 30_000)
        );
        assert_eq!(EarSettings::short_answer().end_quiet_ms, 450);
    }

    #[test]
    fn level_of_silence_is_zero_and_of_full_scale_is_one() {
        assert_eq!(level(&[0.0; FRAME]), 0.0);
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[1.0; FRAME]), 1.0);
        assert_eq!(level(&[0.0005; FRAME]), 0.0); // -66 dBFS
    }

    #[test]
    fn level_of_a_quiet_sine_sits_in_the_middle() {
        let frame: Vec<f32> = (0..FRAME)
            .map(|i| 0.05 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin())
            .collect();
        let l = level(&frame);
        assert!((0.3..=0.7).contains(&l), "{l}");
    }
}
