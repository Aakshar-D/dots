//! Sets the wake phrase up for one person's voice, from a few takes of them saying it.
//!
//! The takes are replayed through the spotter at each `WakeTuning`, strictest first, and the
//! strictest that catches enough of them is the one to use. How loud the voice was in the
//! takes is kept too, so the microphone's gain starts where that voice needs it. Nothing of
//! the takes themselves is kept: only the tuning, the level and the count.

use serde::{Deserialize, Serialize};

use crate::engine::{WakeSpotter, WakeWord, FRAME};
use crate::gain::Gain;
use crate::sherpa::WakeTuning;

/// How many takes a setup asks for.
pub const TAKES: u8 = 5;
/// Frames in one take: about three seconds.
pub const TAKE_FRAMES: usize = 94;
/// A tuning that catches this many of `TAKES` is good enough to use.
const ENOUGH: u8 = 4;
/// Frames of silence fed after a take, so the spotter decodes its last block.
const TAIL_FRAMES: usize = 24;
/// A voice whose level in the takes is below this (about 100 in 32768) reaches the
/// microphone faintly: the person is told so.
pub const FAINT: f32 = 0.003;
/// A frame this many times louder than the quietest frames of its take is the voice.
const OVER_NOISE: f32 = 3.0;
/// A take needs this many voice frames (about a quarter of a second) for its level to count.
const MIN_VOICE_FRAMES: usize = 8;

/// What a setup found.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// How many takes were recorded.
    pub takes: u8,
    /// How many of them the chosen tuning caught.
    pub heard: u8,
    pub tuning: WakeTuning,
    /// The level of the voice in the takes (rms, of 1.0 full scale); `None` when no take had
    /// a voice in it.
    pub level: Option<f32>,
}

impl Calibration {
    /// Whether the setup is worth keeping: the phrase was caught at least once.
    pub fn usable(&self) -> bool {
        self.heard > 0
    }

    pub fn faint(&self) -> bool {
        self.level.is_some_and(|level| level < FAINT)
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// The level of the voice in one take: the mean level of the frames that stand out of the
/// take's quietest tenth. `None` when too few do.
fn take_level(take: &[Vec<f32>]) -> Option<f32> {
    let mut levels: Vec<f32> = take.iter().map(|frame| rms(frame)).collect();
    if levels.is_empty() {
        return None;
    }
    let mut sorted = levels.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let noise = sorted[sorted.len() / 10];
    levels.retain(|level| *level > noise * OVER_NOISE && *level > 0.0);
    (levels.len() >= MIN_VOICE_FRAMES).then(|| levels.iter().sum::<f32>() / levels.len() as f32)
}

/// The level of the voice over the takes: the middle one of those that had a voice.
pub fn voice_level(takes: &[Vec<Vec<f32>>]) -> Option<f32> {
    let mut levels: Vec<f32> = takes.iter().filter_map(|take| take_level(take)).collect();
    levels.sort_by(|a, b| a.total_cmp(b));
    levels.get(levels.len() / 2).copied()
}

/// How many times `wake` catches the wake phrase in `takes`, made as loud as a session that
/// knows the voice's `level` would make them. The takes are heard one after the other as one
/// sound, as they were recorded: a phrase said across the end of one take and the start of
/// the next is still caught. Never more than there are takes.
pub fn heard_times(wake: &mut dyn WakeSpotter, takes: &[Vec<Vec<f32>>], level: Option<f32>) -> u8 {
    let mut gain = level.map_or_else(Gain::new, Gain::with_voice);
    wake.reset();
    let quiet = vec![0.0; FRAME];
    let mut caught = 0usize;
    for frame in takes.iter().flatten() {
        if frame.len() != FRAME {
            continue;
        }
        if wake.heard(&gain.apply(frame)) == Some(WakeWord::HeyDot) {
            caught += 1;
        }
    }
    for _ in 0..TAIL_FRAMES {
        if wake.heard(&quiet) == Some(WakeWord::HeyDot) {
            caught += 1;
        }
    }
    wake.reset();
    u8::try_from(caught.min(takes.len())).unwrap_or(u8::MAX)
}

/// The tuning for these takes. `caught(tuning)` says how many times the spotter at `tuning`
/// catches the phrase in them. The strictest tuning that catches `ENOUGH` wins; when none
/// does, the one that catches the most, the stricter of two that tie.
pub fn calibrate(takes: &[Vec<Vec<f32>>], mut caught: impl FnMut(WakeTuning) -> u8) -> Calibration {
    let level = voice_level(takes);
    let count = u8::try_from(takes.len()).unwrap_or(u8::MAX);
    let enough = ENOUGH.min(count);
    let mut best = (WakeTuning::Standard, 0u8);
    for tuning in WakeTuning::STRICTEST_FIRST {
        let heard = caught(tuning).min(count);
        if heard > best.1 {
            best = (tuning, heard);
        }
        if heard >= enough && heard > 0 {
            break;
        }
    }
    Calibration {
        takes: count,
        heard: best.1,
        tuning: best.0,
        level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(level: f32) -> Vec<f32> {
        (0..FRAME)
            .map(|i| level * std::f32::consts::SQRT_2 * (i as f32 * 0.3).sin())
            .collect()
    }

    /// A take: quiet, then `voice_frames` at `level`, then quiet.
    fn take(level: f32, voice_frames: usize) -> Vec<Vec<f32>> {
        let mut frames = vec![tone(0.000_1); 30];
        frames.extend(vec![tone(level); voice_frames]);
        frames.extend(vec![tone(0.000_1); TAKE_FRAMES - 30 - voice_frames]);
        frames
    }

    fn five_of(take: Vec<Vec<f32>>) -> Vec<Vec<Vec<f32>>> {
        vec![take; 5]
    }

    fn five(level: f32) -> Vec<Vec<Vec<f32>>> {
        five_of(take(level, 20))
    }

    #[test]
    fn the_strictest_tuning_that_catches_four_of_five_is_used() {
        let mut asked = Vec::new();
        let got = calibrate(&five(0.05), |tuning| {
            asked.push(tuning);
            5
        });
        assert_eq!(
            (got.tuning, got.heard, got.takes),
            (WakeTuning::Strict, 5, 5)
        );
        // Nothing looser was tried.
        assert!(asked.iter().all(|t| *t == WakeTuning::Strict));
    }

    #[test]
    fn a_looser_tuning_is_used_when_the_strict_one_misses_too_many() {
        let got = calibrate(&five(0.001), |tuning| match tuning {
            WakeTuning::Strict => 1,
            WakeTuning::Standard => 4,
            WakeTuning::Sensitive => 5,
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 4));
    }

    #[test]
    fn when_none_catches_enough_the_one_that_catches_most_is_used() {
        let got = calibrate(&five(0.001), |tuning| match tuning {
            WakeTuning::Strict => 0,
            WakeTuning::Standard => 1,
            WakeTuning::Sensitive => 3,
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Sensitive, 3));
        assert!(got.usable());
    }

    #[test]
    fn two_that_tie_give_the_stricter() {
        let got = calibrate(&five(0.001), |_| 0);
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 0));
        assert!(!got.usable());
        let got = calibrate(&five(0.001), |_| 2);
        assert_eq!((got.tuning, got.heard), (WakeTuning::Strict, 2));
    }

    #[test]
    fn the_level_is_that_of_the_voice_not_of_the_quiet_around_it() {
        let level = voice_level(&five(0.002)).unwrap();
        assert!((level - 0.002).abs() < 0.000_2, "{level}");
        let got = calibrate(&five(0.002), |_| 5);
        assert!(got.faint());
        assert!(!calibrate(&five(0.05), |_| 5).faint());
        // A spotter that fires more often than there are takes caught them all, no more.
        assert_eq!(calibrate(&five(0.05), |_| 9).heard, 5);
    }

    #[test]
    fn takes_with_no_voice_have_no_level() {
        let silent = five_of(vec![tone(0.000_1); TAKE_FRAMES]);
        assert_eq!(voice_level(&silent), None);
        // A click of three frames is not a voice.
        assert_eq!(voice_level(&five_of(take(0.05, 3))), None);
        let got = calibrate(&silent, |_| 0);
        assert_eq!((got.level, got.heard), (None, 0));
        assert!(!got.faint());
    }

    #[test]
    fn a_calibration_is_stored_as_plain_json() {
        let c = Calibration {
            takes: 5,
            heard: 4,
            tuning: WakeTuning::Sensitive,
            level: Some(0.5),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(
            json,
            r#"{"takes":5,"heard":4,"tuning":"sensitive","level":0.5}"#
        );
        assert_eq!(serde_json::from_str::<Calibration>(&json).unwrap(), c);
    }

    /// Catches the phrase when a frame at least `over` loud comes after a quieter one.
    struct Spots {
        over: f32,
        loud: bool,
    }

    impl WakeSpotter for Spots {
        fn heard(&mut self, frame: &[f32]) -> Option<WakeWord> {
            let loud = rms(frame) >= self.over;
            let began = loud && !self.loud;
            self.loud = loud;
            began.then_some(WakeWord::HeyDot)
        }
        fn reset(&mut self) {
            self.loud = false;
        }
    }

    #[test]
    fn takes_are_heard_as_loud_as_the_session_would_make_them() {
        let mut wake = Spots {
            over: 0.02,
            loud: false,
        };
        let faint = five(0.001);
        // Unknown level: the gain finds the voice as it goes.
        assert_eq!(heard_times(&mut wake, &faint, None), 5);
        // Known level: loud from the first frame.
        assert_eq!(heard_times(&mut wake, &faint, Some(0.001)), 5);
        // Silence is never the phrase.
        let silent = five_of(vec![tone(0.000_1); TAKE_FRAMES]);
        assert_eq!(heard_times(&mut wake, &silent, Some(0.001)), 0);
    }

    /// The person spoke late: the phrase began at the end of one take and ended in the next.
    #[test]
    fn a_phrase_said_across_two_takes_is_caught_once() {
        let mut wake = Spots {
            over: 0.02,
            loud: false,
        };
        let mut first = vec![tone(0.000_1); TAKE_FRAMES - 10];
        first.extend(vec![tone(0.05); 10]);
        let mut second = vec![tone(0.05); 10];
        second.extend(vec![tone(0.000_1); TAKE_FRAMES - 10]);
        assert_eq!(heard_times(&mut wake, &[first, second], Some(0.05)), 1);
    }
}
