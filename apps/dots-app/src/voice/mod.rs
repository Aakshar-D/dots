//! Open Dot's voice in the app: the speech models, the voice session and the hotkey that
//! opens it (Open Dot design spec, step 3). With voice off, nothing in here runs.
//!
//! For now this only links the `dots-voice` crate, so the app's build and its Windows C
//! runtime are settled before the service is written.

const _: fn() = || {
    let _ = dots_voice::Manifest::pinned;
};
