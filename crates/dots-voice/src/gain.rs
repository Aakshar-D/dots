//! Makes a faint microphone louder before anything listens to it.
//!
//! A voice that reaches the microphone faintly (a person across the room, a closed laptop)
//! is missed by all three engines. Measured with the ten voices in room noise of 4 parts in
//! 32768: at a speech level of 33 "Hey Sage" was caught once in ten and the speech detector
//! lost about four frames in ten of a sentence; at 3277 it was caught ten times in ten. The
//! same faint sound made 30 times louder, noise and all, was caught eight times in ten and
//! the detector saw the whole sentence.
//!
//! So the level of the person's voice is followed, and frames are multiplied to bring it
//! to `TARGET`. Sound that is already loud enough is left exactly as it is.

/// The speech level frames are brought up to (rms, of 1.0 full scale): about 983 in 32768.
const TARGET: f32 = 0.03;
/// The most a frame is multiplied by.
const MAX_GAIN: f32 = 40.0;
/// The room's noise is never brought above this level.
const NOISE_CEILING: f32 = 0.004;
/// A frame this many times louder than the room's noise is taken for a voice.
const OVER_NOISE: f32 = 3.0;
/// A frame quieter than this is never taken for a voice, however quiet the room.
const QUIETEST_VOICE: f32 = 0.000_3;
/// How fast the followed level rises to a louder frame, and falls to a quieter one, per
/// voice frame. Rising fast keeps a loud sound from being made louder still; falling slowly
/// keeps the quiet between syllables from pulling the level down.
const RISE: f32 = 0.5;
const FALL: f32 = 0.03;
/// How fast the room's noise level creeps up when no frame is quieter than it.
const NOISE_CREEP: f32 = 1.002;
/// The most the gain changes from one frame to the next, as a factor.
const STEP: f32 = 1.25;

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// Follows how loud the person's voice is and how loud the room is, and says how much louder
/// to make the microphone.
#[derive(Debug, Clone)]
pub struct Gain {
    /// The room's noise level: the quietest frames seen lately.
    noise: Option<f32>,
    /// The level of the person's voice, once some of it was heard.
    voice: Option<f32>,
    gain: f32,
}

impl Default for Gain {
    fn default() -> Self {
        Self::new()
    }
}

impl Gain {
    pub fn new() -> Self {
        Self {
            noise: None,
            voice: None,
            gain: 1.0,
        }
    }

    /// What the frames are multiplied by now.
    pub fn factor(&self) -> f32 {
        self.gain
    }

    /// Takes one frame into account without changing it: for sound that is not the person
    /// (the session's own voice in the microphone). The room's noise is still followed.
    pub fn pass(&mut self, frame: &[f32]) {
        self.follow_noise(rms(frame));
    }

    /// `frame`, made louder when the voice heard lately was faint. Samples stay within -1..1.
    pub fn apply(&mut self, frame: &[f32]) -> Vec<f32> {
        let level = rms(frame);
        let noise = self.follow_noise(level);
        let first_voice = self.voice.is_none();
        if level >= QUIETEST_VOICE && level > noise * OVER_NOISE {
            self.voice = Some(match self.voice {
                None => level,
                Some(v) if level > v => v + (level - v) * RISE,
                Some(v) => v + (level - v) * FALL,
            });
        }
        let wanted = match self.voice {
            // Nothing heard yet: nothing is known about the voice.
            None => 1.0,
            Some(voice) => {
                let for_voice = TARGET / voice;
                let for_noise = if noise > 0.0 {
                    NOISE_CEILING / noise
                } else {
                    MAX_GAIN
                };
                for_voice.min(for_noise).clamp(1.0, MAX_GAIN)
            }
        };
        // The very first voice heard sets the gain at once: stepping up to it would take half
        // a second, and "Hey Sage" is over by then (caught 5 times in 10 stepping, at a level
        // where a gain already settled catches 8).
        // It comes down at once: a voice that turns out loud must not stay amplified while
        // the gain steps down (a normal voice whose first, soft frame set a high gain was
        // then caught 7 times in 10, not 9).
        self.gain = if (first_voice && self.voice.is_some()) || wanted < self.gain {
            wanted
        } else {
            wanted.min(self.gain * STEP)
        };
        // And no frame is made louder than the target, whatever the gain says.
        let now = if level > 0.0 {
            self.gain.min((TARGET / level).max(1.0))
        } else {
            self.gain
        };
        if now == 1.0 {
            return frame.to_vec();
        }
        frame.iter().map(|s| (s * now).clamp(-1.0, 1.0)).collect()
    }

    fn follow_noise(&mut self, level: f32) -> f32 {
        let noise = match self.noise {
            None => level,
            Some(n) if level < n => level,
            Some(n) => n * NOISE_CREEP,
        };
        self.noise = Some(noise);
        noise
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FRAME;

    /// A frame of a steady tone at `level` rms.
    fn tone(level: f32) -> Vec<f32> {
        (0..FRAME)
            .map(|i| level * std::f32::consts::SQRT_2 * (i as f32 * 0.3).sin())
            .collect()
    }

    fn feed(gain: &mut Gain, level: f32, frames: usize) -> Vec<f32> {
        let mut last = Vec::new();
        for _ in 0..frames {
            last = gain.apply(&tone(level));
        }
        last
    }

    #[test]
    fn a_voice_loud_enough_is_left_exactly_as_it_is() {
        let mut gain = Gain::new();
        feed(&mut gain, 0.000_1, 30);
        let frame = tone(0.1);
        for _ in 0..60 {
            assert_eq!(gain.apply(&frame), frame);
        }
        assert_eq!(gain.factor(), 1.0);
    }

    #[test]
    fn a_faint_voice_is_brought_up_to_the_target() {
        let mut gain = Gain::new();
        feed(&mut gain, 0.000_1, 30);
        let out = feed(&mut gain, 0.001, 60);
        assert!((gain.factor() - 30.0).abs() < 1.0, "{}", gain.factor());
        assert!((rms(&out) - TARGET).abs() < 0.003, "{}", rms(&out));
    }

    #[test]
    fn the_gain_is_capped_and_samples_stay_in_range() {
        let mut gain = Gain::new();
        feed(&mut gain, 0.000_01, 30);
        feed(&mut gain, 0.000_4, 80);
        assert!(gain.factor() <= MAX_GAIN);
        // A sudden loud sound on a high gain is clipped, never out of range.
        let out = gain.apply(&tone(0.5));
        assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn a_noisy_room_is_not_made_loud() {
        let mut gain = Gain::new();
        // Noise at 0.002: a voice three times that still gets at most 2x.
        feed(&mut gain, 0.002, 30);
        feed(&mut gain, 0.007, 60);
        assert!(
            gain.factor() <= NOISE_CEILING / 0.002 * 1.2,
            "{}",
            gain.factor()
        );
    }

    #[test]
    fn a_loud_sound_brings_the_gain_down_within_a_few_frames() {
        let mut gain = Gain::new();
        feed(&mut gain, 0.000_1, 30);
        feed(&mut gain, 0.001, 60);
        feed(&mut gain, 0.2, 20);
        assert_eq!(gain.factor(), 1.0);
    }

    #[test]
    fn silence_and_passed_frames_change_nothing() {
        let mut gain = Gain::new();
        let quiet = vec![0.0; FRAME];
        assert_eq!(gain.apply(&quiet), quiet);
        feed(&mut gain, 0.000_1, 30);
        feed(&mut gain, 0.001, 60);
        let before = gain.factor();
        for _ in 0..50 {
            gain.pass(&tone(0.3));
        }
        assert_eq!(gain.factor(), before);
    }
}
