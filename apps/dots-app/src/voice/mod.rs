//! Open Dot's voice in the app: the speech models, the voice session and the hotkey that
//! opens it (Open Dot design spec, step 3). With voice off, nothing in here runs.
//!
//! `bridge` joins the voice session to Open Dot's mind; `events` is what the UI receives.

// The voice service (next) is the first caller of the bridge and the events.
#![allow(dead_code)]

pub mod bridge;
pub mod events;
