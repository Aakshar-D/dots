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

/// Whether `wake` catches the wake phrase in `take`, made as loud as a session that knows
/// the voice's `level` would make it.
pub fn hears(wake: &mut dyn WakeSpotter, take: &[Vec<f32>], level: Option<f32>) -> bool {
    let mut gain = level.map_or_else(Gain::new, Gain::with_voice);
    wake.reset();
    let quiet = vec![0.0; FRAME];
    let mut caught = false;
    for frame in take {
        if frame.len() != FRAME {
            continue;
        }
        caught |= wake.heard(&gain.apply(frame)) == Some(WakeWord::HeyDot);
    }
    for _ in 0..TAIL_FRAMES {
        caught |= wake.heard(&quiet) == Some(WakeWord::HeyDot);
    }
    wake.reset();
    caught
}

/// The tuning for these takes. `caught(tuning, take)` says whether the spotter at `tuning`
/// catches the phrase in `take`. The strictest tuning that catches `ENOUGH` of them wins;
/// when none does, the one that catches the most, the stricter of two that tie.
pub fn calibrate(
    takes: &[Vec<Vec<f32>>],
    mut caught: impl FnMut(WakeTuning, &[Vec<f32>]) -> bool,
) -> Calibration {
    let level = voice_level(takes);
    let count = u8::try_from(takes.len()).unwrap_or(u8::MAX);
    let enough = ENOUGH.min(count);
    let mut best = (WakeTuning::Standard, 0u8);
    for tuning in WakeTuning::STRICTEST_FIRST {
        let heard = takes.iter().filter(|take| caught(tuning, take)).count();
        let heard = u8::try_from(heard).unwrap_or(u8::MAX);
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
        let got = calibrate(&five(0.05), |tuning, _| {
            asked.push(tuning);
            true
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
        let mut n = 0;
        let got = calibrate(&five(0.001), |tuning, _| {
            n += 1;
            match tuning {
                WakeTuning::Strict => n % 5 == 0,   // 1 of 5
                WakeTuning::Standard => n % 5 != 0, // 4 of 5
                WakeTuning::Sensitive => true,
            }
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 4));
    }

    #[test]
    fn when_none_catches_enough_the_one_that_catches_most_is_used() {
        let mut n = 0;
        let got = calibrate(&five(0.001), |tuning, _| {
            n += 1;
            match tuning {
                WakeTuning::Strict => false,
                WakeTuning::Standard => n % 5 == 0, // 1 of 5
                WakeTuning::Sensitive => n % 5 < 3, // 3 of 5
            }
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Sensitive, 3));
        assert!(got.usable());
    }

    #[test]
    fn two_that_tie_give_the_stricter() {
        let got = calibrate(&five(0.001), |_, _| false);
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 0));
        assert!(!got.usable());
        let mut n = 0;
        let got = calibrate(&five(0.001), |_, _| {
            n += 1;
            n % 5 < 2 // 2 of 5 at every tuning
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Strict, 2));
    }

    #[test]
    fn the_level_is_that_of_the_voice_not_of_the_quiet_around_it() {
        let level = voice_level(&five(0.002)).unwrap();
        assert!((level - 0.002).abs() < 0.000_2, "{level}");
        let got = calibrate(&five(0.002), |_, _| true);
        assert!(got.faint());
        assert!(!calibrate(&five(0.05), |_, _| true).faint());
    }

    #[test]
    fn takes_with_no_voice_have_no_level() {
        let silent = five_of(vec![tone(0.000_1); TAKE_FRAMES]);
        assert_eq!(voice_level(&silent), None);
        // A click of three frames is not a voice.
        assert_eq!(voice_level(&five_of(take(0.05, 3))), None);
        let got = calibrate(&silent, |_, _| false);
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

    struct Spots {
        /// Catches the phrase once a frame at least this loud came.
        over: f32,
        seen: bool,
    }

    impl WakeSpotter for Spots {
        fn heard(&mut self, frame: &[f32]) -> Option<WakeWord> {
            if !self.seen && rms(frame) >= self.over {
                self.seen = true;
                return Some(WakeWord::HeyDot);
            }
            None
        }
        fn reset(&mut self) {
            self.seen = false;
        }
    }

    #[test]
    fn a_take_is_heard_as_loud_as_the_session_would_make_it() {
        let mut wake = Spots {
            over: 0.02,
            seen: false,
        };
        let faint = take(0.001, 20);
        // Unknown level: the gain finds the voice as it goes.
        assert!(hears(&mut wake, &faint, None));
        // Known level: loud from the first frame.
        assert!(hears(&mut wake, &faint, Some(0.001)));
        // Silence is never the phrase.
        let silent = vec![tone(0.000_1); TAKE_FRAMES];
        assert!(!hears(&mut wake, &silent, Some(0.001)));
    }
}
