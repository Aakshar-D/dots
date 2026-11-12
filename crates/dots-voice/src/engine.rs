//! What the rest of the crate needs from speech engines, as small traits, so the logic
//! built on top can be tested without models, a microphone or a speaker.
//!
//! Nothing here logs; no audio sample or text leaves memory.

/// One frame fed to the speech detector and the wake spotter: 32 ms at 16 kHz.
pub const FRAME: usize = 512;
/// The rate the ears (detector, wake spotter, recognizer input) work at.
pub const EAR_RATE: u32 = 16_000;

/// Mono audio, samples in `-1.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples.len() as f32 / self.rate as f32
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeWord {
    HeyDot,
    DotStop,
}

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("speech models are not installed")]
    NotInstalled,
    #[error("{0}")]
    Failed(String),
}

/// Turns a sentence into audio. `voice` counts from 0 up to `voices() - 1`.
pub trait Speaker: Send + Sync {
    fn speak(&self, sentence: &str, voice: u8, speed: f32) -> Result<Clip, EngineError>;
    fn voices(&self) -> u8;
}

/// Turns a clip into text.
pub trait Recognizer: Send + Sync {
    fn transcribe(&self, clip: &Clip) -> Result<String, EngineError>;
}

/// Says whether a `FRAME` of 16 kHz audio belongs to speech.
pub trait SpeechDetector: Send {
    fn is_speech(&mut self, frame16k: &[f32]) -> bool;
    fn reset(&mut self);
}

/// Listens for the wake phrases, one `FRAME` of 16 kHz audio at a time.
pub trait WakeSpotter: Send {
    fn heard(&mut self, frame16k: &[f32]) -> Option<WakeWord>;
    fn reset(&mut self);
    /// Of the samples fed up to the word `heard` just returned, how many came after that
    /// word ended (the spotter answers a little late). 0 when not known.
    fn after_word(&self) -> usize {
        0
    }
}

/// Convert `clip` to `rate` by linear interpolation. Going down, each output sample
/// averages the input it covers, which keeps the highest frequencies from folding back.
pub fn resample(clip: &Clip, rate: u32) -> Clip {
    if clip.rate == 0 || rate == 0 {
        return clip.clone(); // no rate to convert from or to
    }
    if clip.rate == rate || clip.samples.is_empty() {
        return Clip {
            samples: clip.samples.clone(),
            rate,
        };
    }
    let input = &clip.samples;
    let step = clip.rate as f64 / rate as f64; // input samples per output sample
    let out_len = (input.len() as f64 / step).round() as usize;
    let last = input.len() - 1;
    let samples = (0..out_len)
        .map(|i| {
            let from = i as f64 * step;
            if step > 1.0 {
                let to = (from + step).min(input.len() as f64);
                let (a, b) = (from.floor() as usize, (to.ceil() as usize).min(last + 1));
                let window = &input[a.min(last)..b.max(a.min(last) + 1)];
                window.iter().sum::<f32>() / window.len() as f32
            } else {
                let a = (from.floor() as usize).min(last);
                let b = (a + 1).min(last);
                let t = (from - a as f64) as f32;
                input[a] * (1.0 - t) + input[b] * t
            }
        })
        .collect();
    Clip { samples, rate }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32, seconds: f32) -> Clip {
        let n = (rate as f32 * seconds) as usize;
        Clip {
            samples: (0..n)
                .map(|i| 0.8 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
                .collect(),
            rate,
        }
    }

    fn peak(c: &Clip) -> f32 {
        c.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn resample_keeps_duration() {
        let clip = sine(440.0, 44_100, 1.0);
        let out = resample(&clip, 16_000);
        assert_eq!(out.rate, 16_000);
        assert!((out.samples.len() as i64 - 16_000).abs() <= 1);
        assert!((peak(&out) - peak(&clip)).abs() <= 0.05 * peak(&clip));
    }

    #[test]
    fn resample_with_equal_rates_changes_nothing() {
        let clip = sine(440.0, 16_000, 0.1);
        assert_eq!(resample(&clip, 16_000), clip);
    }

    #[test]
    fn resample_of_an_empty_clip_is_empty() {
        let clip = Clip {
            samples: Vec::new(),
            rate: 44_100,
        };
        let out = resample(&clip, 16_000);
        assert!(out.samples.is_empty());
        assert_eq!(out.rate, 16_000);
    }

    #[test]
    fn upsampling_keeps_duration() {
        let clip = sine(440.0, 16_000, 1.0);
        let out = resample(&clip, 44_100);
        assert_eq!(out.rate, 44_100);
        assert!((out.samples.len() as i64 - 44_100).abs() <= 1);
        assert!((peak(&out) - peak(&clip)).abs() <= 0.05 * peak(&clip));
    }
}
