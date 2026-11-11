//! On-device speech for Dots: what is heard and what is spoken.
//!
//! Nothing in this crate writes audio or text to disk or to logs.

mod engine;
mod models;
mod sherpa;
mod text;

pub use engine::{
    resample, Clip, EngineError, Recognizer, Speaker, SpeechDetector, WakeSpotter, WakeWord,
    EAR_RATE, FRAME,
};
pub use models::{
    install, installed, remove, Manifest, ModelError, ModelFile, ModelPaths, Progress,
};
pub use sherpa::{SherpaDetector, SherpaRecognizer, SherpaSpeaker, SherpaWake};
pub use text::{speakable, trim_spoken, SentenceStream, MAX_SPOKEN_CHARS};
