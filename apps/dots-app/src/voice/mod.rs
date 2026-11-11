//! Open Dot's voice in the app: the speech models, the voice session and the hotkey that
//! opens it (Open Dot design spec, step 3). With voice off, nothing in here runs.
//!
//! `service` owns the session and the speech files' `download`; `bridge` joins the session
//! to Open Dot's mind; `events` is what the UI receives on the `voice` event.

pub mod bridge;
pub mod download;
pub mod events;
pub mod service;

use std::path::PathBuf;
use std::sync::Arc;

use dots_core::Runtime;
use dots_voice::Manifest;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::error::CmdError;
use events::VoiceEvent;
use service::{RealFactory, RuntimeHost, Service};

/// The UI's event stream for everything voice.
pub const VOICE_EVENT: &str = "voice";

/// The voice service, when the runtime started.
pub struct VoiceHandle(Option<Arc<Service>>);

impl VoiceHandle {
    pub fn get(&self) -> Result<Arc<Service>, CmdError> {
        self.0
            .clone()
            .ok_or_else(|| CmdError::internal("Dots did not start; see the startup error"))
    }

    /// Shuts the session down, when there is one (exit, tray quit, restart).
    pub fn shutdown(&self) {
        if let Some(service) = &self.0 {
            service.shutdown();
        }
    }
}

/// Makes the voice service for `rt` and applies the stored settings: with voice off nothing
/// is built. Managed even when the runtime did not start, so commands answer with its error.
pub fn manage(app: &AppHandle, rt: Option<Arc<Runtime>>, data_dir: Option<PathBuf>) {
    let service = match (rt, data_dir) {
        (Some(rt), Some(dir)) => {
            let (tx, rx) = unbounded_channel();
            let service = Service::new(
                Arc::new(RuntimeHost(rt.clone())),
                Arc::new(RealFactory),
                dir.join("voice").join("models"),
                Manifest::pinned(),
                tx,
            );
            spawn_forwarder(app.clone(), rx);
            let started = service.clone();
            tauri::async_runtime::spawn(async move {
                started.apply(&rt.settings().await).await;
            });
            Some(service)
        }
        _ => None,
    };
    app.manage(VoiceHandle(service));
}

/// Whether the main window is on screen: levels are only drawn there.
fn main_window_shown(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

/// Sends every voice event to the UI, levels only while the main window is shown.
fn spawn_forwarder(app: AppHandle, mut rx: UnboundedReceiver<VoiceEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if matches!(ev, VoiceEvent::Level { .. }) && !main_window_shown(&app) {
                continue;
            }
            let _ = app.emit(VOICE_EVENT, &ev);
        }
    });
}
