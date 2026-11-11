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
const WAKE_KEYWORDS: &str = "▁HE Y ▁DO T :1.5 #0.2 @HEY_DOT\n▁DO T ▁ST O P :1.5 #0.2 @DOT_STOP";
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
            .generate_with_config(sentence, &config, None::<fn(&[f32], f32) -> bool>)
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
        if clip.rate == 0 {
            return Err(EngineError::Failed("the clip has no sample rate".into()));
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
}

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
        Ok(Self { spotter, stream })
    }
}

impl WakeSpotter for SherpaWake {
    fn heard(&mut self, frame16k: &[f32]) -> Option<WakeWord> {
        self.stream.accept_waveform(EAR_RATE as i32, frame16k);
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
            self.spotter.reset(&self.stream);
            return Some(word);
        }
        None
    }

    fn reset(&mut self) {
        self.spotter.reset(&self.stream);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
