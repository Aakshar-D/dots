//! The short, soft tone played when Dot starts listening from rest.

use crate::engine::Clip;

/// The rate the tone is made at: the rate of the speech clips (the speaker model makes them
/// at 44.1 kHz), so the sink treats the tone exactly as it treats speech. A clip carries its
/// own rate and the sink resamples every clip to the device's rate.
pub const TONE_RATE: u32 = 44_100;

/// The two notes, rising a fourth.
const NOTES_HZ: [f32; 2] = [660.0, 880.0];
const NOTE_MS: u32 = 70;
const GAP_MS: u32 = 15;
/// Raised-cosine fade at each end of each note.
const FADE_MS: u32 = 12;
/// Of full scale: soft beside speech.
const PEAK: f32 = 0.2;

/// Two soft sine notes rising a fourth (660 Hz then 880 Hz), 70 ms each with 15 ms of
/// silence between, each faded in and out over 12 ms with a raised cosine, peaking at 0.2 of
/// full scale. The first and last samples are zero, so it starts and ends without a click.
/// 155 ms long.
pub fn listen_tone(rate: u32) -> Clip {
    let ms = |ms: u32| (u64::from(rate) * u64::from(ms) / 1000) as usize;
    let (note, gap, fade) = (ms(NOTE_MS), ms(GAP_MS), ms(FADE_MS).max(1));
    let mut samples = Vec::with_capacity(note * 2 + gap);
    for (i, hz) in NOTES_HZ.into_iter().enumerate() {
        if i > 0 {
            samples.extend(std::iter::repeat_n(0.0, gap));
        }
        samples.extend((0..note).map(|n| {
            let edge = n.min(note.saturating_sub(1) - n);
            let envelope = if edge < fade {
                0.5 - 0.5 * (std::f32::consts::PI * edge as f32 / fade as f32).cos()
            } else {
                1.0
            };
            let phase = std::f32::consts::TAU * hz * n as f32 / rate as f32;
            PEAK * envelope * phase.sin()
        }));
    }
    Clip { samples, rate }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The power of `hz` in `samples` (Goertzel), per sample squared.
    fn power(samples: &[f32], rate: u32, hz: f32) -> f32 {
        let w = std::f32::consts::TAU * hz / rate as f32;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in samples {
            let s = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        let p = s1 * s1 + s2 * s2 - coeff * s1 * s2;
        p / (samples.len() as f32).powi(2)
    }

    #[test]
    fn the_tone_is_two_notes_with_a_gap_under_200_ms() {
        for rate in [16_000, 22_050, 44_100, 48_000] {
            let tone = listen_tone(rate);
            assert_eq!(tone.rate, rate);
            let expected = (rate as usize * 70 / 1000) * 2 + rate as usize * 15 / 1000;
            assert_eq!(tone.samples.len(), expected, "{rate}");
            assert!(tone.seconds() < 0.2, "{rate}");
            assert!((tone.seconds() - 0.155).abs() < 0.001, "{rate}");
        }
    }

    #[test]
    fn the_tone_is_soft_and_starts_and_ends_at_zero() {
        let tone = listen_tone(TONE_RATE);
        let peak = tone.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= 0.2 && peak > 0.19, "peak {peak}");
        assert_eq!(tone.samples.first(), Some(&0.0));
        assert_eq!(tone.samples.last(), Some(&0.0));
        // Each note fades in and out: the samples next to the gap are near zero too.
        let note = TONE_RATE as usize * 70 / 1000;
        let gap = TONE_RATE as usize * 15 / 1000;
        assert!(tone.samples[note - 1].abs() < 1e-6);
        assert!(tone.samples[note..note + gap].iter().all(|s| *s == 0.0));
        assert_eq!(tone.samples[note + gap], 0.0);
        // No jump between neighbouring samples is larger than the steepest sine slope allows.
        let slope = 0.2 * std::f32::consts::TAU * 880.0 / TONE_RATE as f32;
        assert!(tone
            .samples
            .windows(2)
            .all(|w| (w[1] - w[0]).abs() <= slope * 1.05));
    }

    #[test]
    fn the_energy_sits_at_the_two_notes() {
        let tone = listen_tone(TONE_RATE);
        let note = TONE_RATE as usize * 70 / 1000;
        let gap = TONE_RATE as usize * 15 / 1000;
        let (first, second) = (&tone.samples[..note], &tone.samples[note + gap..]);
        let at = |s: &[f32], hz| power(s, TONE_RATE, hz);
        // The first note is 660 Hz, the second 880 Hz.
        assert!(at(first, 660.0) > 20.0 * at(first, 880.0));
        assert!(at(second, 880.0) > 20.0 * at(second, 660.0));
        // Nothing much anywhere else.
        for other in [220.0, 440.0, 1320.0, 1760.0, 3000.0] {
            assert!(at(first, 660.0) > 50.0 * at(first, other), "{other}");
            assert!(at(second, 880.0) > 50.0 * at(second, other), "{other}");
        }
    }
}
