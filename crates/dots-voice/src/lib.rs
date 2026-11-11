//! On-device speech for Dots: what is heard and what is spoken.
//!
//! Nothing in this crate writes audio or text to disk or to logs.

mod text;

pub use text::{speakable, trim_spoken, SentenceStream, MAX_SPOKEN_CHARS};
