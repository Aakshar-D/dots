//! On-device speech for Dots: what is heard and what is spoken.
//!
//! Nothing in this crate writes audio or text to disk or to logs.

mod audio;
mod ears;
mod engine;
mod models;
mod session;
mod sherpa;
mod text;
mod voice;

pub use audio::{frames_from, microphones, speakers, CpalMic, CpalSink, Device, Mic, MicFrame};
pub use ears::{level, EarEvent, EarSettings, Ears};
pub use engine::{
    resample, Clip, EngineError, Recognizer, Speaker, SpeechDetector, WakeSpotter, WakeWord,
    EAR_RATE, FRAME,
};
pub use models::{
    install, installed, remove, Manifest, ModelError, ModelFile, ModelPaths, Progress,
};
pub use session::{
    After, Clock, MicOpener, Parts, Session, SessionEvent, SessionSettings, SinkOpener, State,
    SystemClock,
};
pub use sherpa::{SherpaDetector, SherpaRecognizer, SherpaSpeaker, SherpaWake};
pub use text::{speakable, trim_spoken, SentenceStream, MAX_SPOKEN_CHARS};
pub use voice::{Sink, Voice, VoiceEvent};
