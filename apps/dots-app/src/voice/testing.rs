//! For the end-to-end test of Open Dot only (`tests/open_dot_e2e.rs`); the app never uses it.
//! It builds the voice service exactly as `manage` does, with the real factory (real models,
//! real session, real bridge, the runtime's own settings and mind), except that the
//! microphone and the speaker are the test's, and the models folder is taken as installed.

use std::path::PathBuf;
use std::sync::Arc;

use dots_core::Runtime;
use dots_voice::Manifest;
use tokio::sync::mpsc::UnboundedSender;

pub use super::events::VoiceEvent;
pub use super::service::{OpenMic, OpenSink, Service};
use super::service::{RealFactory, RuntimeHost};

/// The voice service over `rt`, loading the models in `models` and opening `open_mic` and
/// `open_sink` in place of the system's devices; its events go to `ui`. Voice starts off:
/// `start` with the runtime's settings applies them. The folder has no download markers (it
/// was laid down by hand), so the manifest is empty and nothing is ever downloaded. Spoken
/// notices are not started (`manage` starts them).
pub fn service(
    rt: Arc<Runtime>,
    models: PathBuf,
    open_mic: OpenMic,
    open_sink: OpenSink,
    ui: UnboundedSender<VoiceEvent>,
) -> Arc<Service> {
    Service::new(
        Arc::new(RuntimeHost(rt)),
        Arc::new(RealFactory::with_openers(open_mic, open_sink)),
        models,
        Manifest { files: Vec::new() },
        ui,
    )
}
