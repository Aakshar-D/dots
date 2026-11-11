//! On-device speech for Dots: what is heard and what is spoken.
//!
//! Nothing in this crate writes audio or text to disk or to logs.

mod models;
mod text;

pub use models::{
    install, installed, remove, Manifest, ModelError, ModelFile, ModelPaths, Progress,
};
pub use text::{speakable, trim_spoken, SentenceStream, MAX_SPOKEN_CHARS};
