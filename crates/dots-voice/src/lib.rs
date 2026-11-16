//! On-device speech for Dots: what is heard and what is spoken.
//!
//! Nothing in this crate writes audio or text to disk or to logs.

mod audio;
mod calibrate;
mod ears;
mod engine;
mod gain;
mod models;
mod session;
mod sherpa;
mod text;
mod tone;
mod voice;

pub use audio::{frames_from, microphones, speakers, CpalMic, CpalSink, Device, Mic, MicFrame};
pub use calibrate::{calibrate, heard_times, voice_level, Calibration, FAINT, TAKES, TAKE_FRAMES};
pub use ears::{level, EarEvent, EarSettings, Ears};
pub use engine::{
    resample, Clip, EngineError, Recognizer, Speaker, SpeechDetector, WakeSpotter, WakeWord,
    EAR_RATE, FRAME,
};
pub use gain::Gain;
pub use models::{
    install, installed, remove, Manifest, ModelError, ModelFile, ModelPaths, Progress,
};
pub use session::{
    After, Clock, MicOpener, Parts, Session, SessionEvent, SessionSettings, SinkOpener, State,
    SystemClock,
};
pub use sherpa::{SherpaDetector, SherpaRecognizer, SherpaSpeaker, SherpaWake, WakeTuning};
pub use text::{speakable, trim_spoken, SentenceStream, MAX_SPOKEN_CHARS};
pub use tone::{listen_tone, TONE_RATE};
pub use voice::{Sink, Voice, VoiceEvent};
