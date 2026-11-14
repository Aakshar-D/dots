//! The real engines, wrapping the `sherpa-onnx` crate.
//!
//! The loaders trust that the caller checked `models::installed`; they only fail with
//! `NotInstalled` when a file they need is missing. Sherpa's debug output is off, and no
//! audio or text is logged.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use sherpa_onnx::{
    GenerationConfig, KeywordSpotter, KeywordSpotterConfig, OfflineMoonshineModelConfig,
    OfflineRecognizer, OfflineRecognizerConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsSupertonicModelConfig, OnlineStream, OnlineTransducerModelConfig,
    SileroVadModelConfig, VadModelConfig, VoiceActivityDetector,
};

use crate::engine::{
    resample, Clip, EngineError, Recognizer, Speaker, SpeechDetector, WakeSpotter, WakeWord,
    EAR_RATE, FRAME,
};
use crate::models::ModelPaths;

/// The wake phrases in sherpa's keyword format: `tokens :boost #threshold @name`.
///
/// The tokens are the model's own pieces: its `bpe.model` spells SAGE as `▁S AGE`. The stop
/// phrase is also listed in the two other ways the pieces can spell it, in case the model
/// gives one of them for a voice. The names after `@` are from the phrase before, "Hey Dot".
///
/// "Hey Sage" is boosted less and held to a higher score than "Sage, stop": a wrong
/// "Sage, stop" only stops Sage speaking; a wrong "Hey Sage" opens the microphone. Measured
/// over the ten voices, twice: "Hey Sage, what needs me?" woke it 20 times in 20, "Hey Sage."
/// 18 or 19, and four near misses ("Hey, say that again tomorrow.", "Hey, save each page of
/// the message." and two more) woke it 0 times in 80. The counts move by one from run to
/// run, as the synthetic voices do: at 0.25 and at 0.35 one near miss in 40 woke it. At 0.6
/// the phrase itself was missed.
///
/// The stop phrase is caught when it is said in one go: "Sage stop." 19 or 20 times in 20,
/// "Sage, stop!" 18 or 19, "Sage, stop please." 20, as "Dot" was. A pause after "Sage" loses
/// it: the voices put one in "Sage, stop." (13 to 16 in 20; "Dot, stop." 18 or 19) and a long
/// one in "Sage. Stop." (0 or 1; "Dot. Stop." 5). No boost, threshold or other spelling of the
/// pieces brought those clips back, and a wider search (`max_active_paths` 8 or 16) only added
/// wrong stops ("The stage stopped moving." 16 times in 20). "Sage, start the next one." is
/// taken for it 4 times in 20, as "Dot, start the next one." was.
const WAKE_KEYWORDS: &str = "▁HE Y ▁S AGE :1.0 #0.3 @HEY_DOT\n▁S AGE ▁ST O P :2.0 #0.1 @DOT_STOP\n▁SA GE ▁ST O P :2.0 #0.1 @DOT_STOP\n▁S A GE ▁ST O P :2.0 #0.1 @DOT_STOP";
const SPEAK_STEPS: i32 = 5;
const VOICES: u8 = 10;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere must not silence speech for good.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The path as text, or `NotInstalled` when there is no such file.
fn file(dir: &Path, name: &str) -> Result<String, EngineError> {
    let path = dir.join(name);
    if !path.is_file() {
        return Err(EngineError::NotInstalled);
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| EngineError::Failed("a model path is not valid text".into()))
}

/// The sentence as sherpa may take it: NUL characters would abort its C string
/// conversion, so they are dropped; nothing speakable or a bad speed is an error.
fn checked_request(sentence: &str, speed: f32) -> Result<String, EngineError> {
    if !speed.is_finite() || speed <= 0.0 {
        return Err(EngineError::Failed(
            "the speaking speed is not valid".into(),
        ));
    }
    let clean: String = sentence.chars().filter(|c| *c != '\0').collect();
    if clean.trim().is_empty() {
        return Err(EngineError::Failed("there is nothing to say".into()));
    }
    Ok(clean)
}

/// Clips under 100 ms hold no speech worth decoding.
fn too_short(clip: &Clip) -> bool {
    clip.rate == 0 || (clip.samples.len() as u64) * 10 < u64::from(clip.rate)
}

fn failed(what: &str) -> EngineError {
    EngineError::Failed(format!("could not start {what}"))
}

pub struct SherpaSpeaker {
    tts: Mutex<OfflineTts>,
    voices: u8,
}

impl SherpaSpeaker {
    pub fn load(paths: &ModelPaths, threads: i32) -> Result<Self, EngineError> {
        let dir = paths.speak_dir();
        let mut cfg = OfflineTtsConfig::default();
        cfg.model.supertonic = OfflineTtsSupertonicModelConfig {
            duration_predictor: Some(file(&dir, "duration_predictor.int8.onnx")?),
            text_encoder: Some(file(&dir, "text_encoder.int8.onnx")?),
            vector_estimator: Some(file(&dir, "vector_estimator.int8.onnx")?),
            vocoder: Some(file(&dir, "vocoder.int8.onnx")?),
            tts_json: Some(file(&dir, "tts.json")?),
            unicode_indexer: Some(file(&dir, "unicode_indexer.bin")?),
            voice_style: Some(file(&dir, "voice.bin")?),
        };
        cfg.model.num_threads = threads.max(1);
        cfg.model.debug = false;
        let tts = OfflineTts::create(&cfg).ok_or_else(|| failed("the voice"))?;
        let voices = u8::try_from(tts.num_speakers()).unwrap_or(VOICES).max(1);
        Ok(Self {
            tts: Mutex::new(tts),
            voices,
        })
    }
}

impl Speaker for SherpaSpeaker {
    fn speak(&self, sentence: &str, voice: u8, speed: f32) -> Result<Clip, EngineError> {
        let sentence = checked_request(sentence, speed)?;
        let mut extra = std::collections::HashMap::new();
        extra.insert("lang".to_owned(), serde_json::json!("en"));
        let config = GenerationConfig {
            sid: i32::from(voice.min(self.voices - 1)),
            speed,
            num_steps: SPEAK_STEPS,
            extra: Some(extra),
            ..Default::default()
        };
        let audio = lock(&self.tts)
            .generate_with_config(&sentence, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| EngineError::Failed("the voice could not say that".into()))?;
        let rate = u32::try_from(audio.sample_rate()).unwrap_or(0);
        if audio.samples().is_empty() || rate == 0 {
            return Err(EngineError::Failed("the voice made no sound".into()));
        }
        Ok(Clip {
            samples: audio.samples().to_vec(),
            rate,
        })
    }

    fn voices(&self) -> u8 {
        self.voices
    }
}

pub struct SherpaRecognizer {
    recognizer: Mutex<OfflineRecognizer>,
}

impl SherpaRecognizer {
    pub fn load(paths: &ModelPaths) -> Result<Self, EngineError> {
        let dir = paths.hear_dir();
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.moonshine = OfflineMoonshineModelConfig {
            encoder: Some(file(&dir, "encoder_model.ort")?),
            merged_decoder: Some(file(&dir, "decoder_model_merged.ort")?),
            ..Default::default()
        };
        cfg.model_config.tokens = Some(file(&dir, "tokens.txt")?);
        cfg.model_config.num_threads = 2;
        cfg.model_config.debug = false;
        let recognizer =
            OfflineRecognizer::create(&cfg).ok_or_else(|| failed("the speech recognizer"))?;
        Ok(Self {
            recognizer: Mutex::new(recognizer),
        })
    }
}

impl Recognizer for SherpaRecognizer {
    fn transcribe(&self, clip: &Clip) -> Result<String, EngineError> {
        if too_short(clip) {
            return Ok(String::new());
        }
        // Converted here so sherpa does not print about its own resampler.
        let clip = &resample(clip, EAR_RATE);
        let recognizer = lock(&self.recognizer);
        let stream = recognizer.create_stream();
        stream.accept_waveform(EAR_RATE as i32, &clip.samples);
        recognizer.decode(&stream);
        let result = stream
            .get_result()
            .ok_or_else(|| EngineError::Failed("nothing came back from the recognizer".into()))?;
        Ok(result.text.trim().to_owned())
    }
}

pub struct SherpaDetector {
    vad: VoiceActivityDetector,
}

impl SherpaDetector {
    pub fn load(paths: &ModelPaths) -> Result<Self, EngineError> {
        let model = paths.vad_file();
        if !model.is_file() {
            return Err(EngineError::NotInstalled);
        }
        let cfg = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: model.to_str().map(str::to_owned),
                threshold: 0.5,
                min_silence_duration: 0.1,
                min_speech_duration: 0.1,
                window_size: FRAME as i32,
                ..Default::default()
            },
            sample_rate: EAR_RATE as i32,
            num_threads: 1,
            debug: false,
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&cfg, 10.0)
            .ok_or_else(|| failed("the speech detector"))?;
        Ok(Self { vad })
    }
}

impl SpeechDetector for SherpaDetector {
    fn is_speech(&mut self, frame16k: &[f32]) -> bool {
        self.vad.accept_waveform(frame16k);
        let speaking = self.vad.detected();
        // Finished segments pile up inside the detector; only the live state matters here.
        if !self.vad.is_empty() {
            self.vad.clear();
        }
        speaking
    }

    fn reset(&mut self) {
        self.vad.reset();
    }
}

pub struct SherpaWake {
    spotter: KeywordSpotter,
    stream: OnlineStream,
    /// Samples fed since the stream was last reset: the spotter's times count from there.
    fed: usize,
    /// Of the samples fed up to the last word spotted, how many came after it ended.
    after: usize,
}

/// The words after the wake phrase are taken from this long before the time the spotter
/// gives for the phrase's last token. That token is the "age" of "Sage", which it times near
/// the start of that sound: with the 0.25 s that suited the "t" of "Dot", the recogniser
/// heard the rest of the word as a word of its own ("Age, please read...", "Stage") for 3
/// of the ten voices; with 0.05 s for none.
const WORD_END_LEAD_SECS: f32 = 0.05;

impl SherpaWake {
    pub fn load(paths: &ModelPaths) -> Result<Self, EngineError> {
        let dir = paths.wake_dir();
        let mut cfg = KeywordSpotterConfig::default();
        cfg.model_config.transducer = OnlineTransducerModelConfig {
            encoder: Some(file(
                &dir,
                "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            )?),
            decoder: Some(file(&dir, "decoder-epoch-12-avg-2-chunk-16-left-64.onnx")?),
            joiner: Some(file(
                &dir,
                "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx",
            )?),
        };
        cfg.model_config.tokens = Some(file(&dir, "tokens.txt")?);
        cfg.model_config.num_threads = 1;
        cfg.model_config.debug = false;
        cfg.keywords_buf = Some(WAKE_KEYWORDS.to_owned());
        let spotter = KeywordSpotter::create(&cfg).ok_or_else(|| failed("the wake phrase"))?;
        let stream = spotter.create_stream();
        Ok(Self {
            spotter,
            stream,
            fed: 0,
            after: 0,
        })
    }
}

/// Of `fed` samples, how many came after a word whose last token the spotter timed at
/// `last_token` seconds (counted from the same reset); 0 when the time makes no sense.
fn samples_after(fed: usize, last_token: Option<f32>) -> usize {
    let Some(at) = last_token.filter(|t| t.is_finite() && *t >= 0.0) else {
        return 0;
    };
    // Rounded down to whole samples on purpose; `as` saturates for huge values.
    let end = ((at - WORD_END_LEAD_SECS).max(0.0) * EAR_RATE as f32) as usize;
    fed.saturating_sub(end)
}

impl WakeSpotter for SherpaWake {
    fn heard(&mut self, frame16k: &[f32]) -> Option<WakeWord> {
        self.stream.accept_waveform(EAR_RATE as i32, frame16k);
        self.fed += frame16k.len();
        self.after = 0;
        while self.spotter.is_ready(&self.stream) {
            self.spotter.decode(&self.stream);
            let Some(result) = self.spotter.get_result(&self.stream) else {
                continue;
            };
            let word = match result.keyword.as_str() {
                "HEY_DOT" => WakeWord::HeyDot,
                "DOT_STOP" => WakeWord::DotStop,
                _ => continue,
            };
            let last = result.timestamps.last().map(|t| result.start_time + t);
            self.after = samples_after(self.fed, last);
            self.reset();
            return Some(word);
        }
        None
    }

    fn reset(&mut self) {
        self.spotter.reset(&self.stream);
        self.fed = 0;
    }

    fn after_word(&self) -> usize {
        self.after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_characters_are_dropped_from_a_sentence() {
        assert_eq!(checked_request("Hel\0lo.", 1.0).unwrap(), "Hello.");
        assert_eq!(
            checked_request("Okay, I'm on it.", 1.0).unwrap(),
            "Okay, I'm on it."
        );
    }

    #[test]
    fn nothing_to_say_or_a_bad_speed_is_refused() {
        for sentence in [
            "", "\0\0", " \0 
",
        ] {
            assert!(checked_request(sentence, 1.0).is_err(), "{sentence:?}");
        }
        for speed in [f32::NAN, 0.0, -1.0, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(checked_request("Hi.", speed).is_err(), "{speed}");
        }
    }

    #[test]
    fn the_samples_after_a_spotted_word_are_counted_from_its_last_token() {
        // 1.6 s fed, the last token timed at 1.2 s: what follows is taken from 1.15 s.
        let after = samples_after(25_600, Some(1.2));
        assert!((7_199..=7_201).contains(&after), "{after}");
        // A token timed at the very start takes everything fed.
        assert_eq!(samples_after(3_200, Some(0.04)), 3_200);
        // A time past the last sample fed, or no time at all: none.
        assert_eq!(samples_after(16_000, Some(1.5)), 0);
        assert_eq!(samples_after(16_000, None), 0);
        assert_eq!(samples_after(16_000, Some(f32::NAN)), 0);
        assert_eq!(samples_after(16_000, Some(-1.0)), 0);
    }

    #[test]
    fn a_clip_under_100_ms_is_too_short() {
        let clip = |n: usize, rate: u32| Clip {
            samples: vec![0.0; n],
            rate,
        };
        assert!(too_short(&clip(0, 16_000)));
        assert!(too_short(&clip(1_599, 16_000)));
        assert!(!too_short(&clip(1_600, 16_000)));
        assert!(too_short(&clip(10, 0)));
        assert!(!too_short(&clip(4_410, 44_100)));
    }

    #[test]
    fn loading_without_models_says_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ModelPaths {
            root: dir.path().to_path_buf(),
        };
        assert!(matches!(
            SherpaSpeaker::load(&paths, 2),
            Err(EngineError::NotInstalled)
        ));
        assert!(matches!(
            SherpaRecognizer::load(&paths),
            Err(EngineError::NotInstalled)
        ));
        assert!(matches!(
            SherpaDetector::load(&paths),
            Err(EngineError::NotInstalled)
        ));
        assert!(matches!(
            SherpaWake::load(&paths),
            Err(EngineError::NotInstalled)
        ));
    }
}
