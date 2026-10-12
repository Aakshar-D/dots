//! Forwards `RuntimeEvent`s to the webview.

use std::sync::Arc;

use dots_core::events::RuntimeEvent;
use dots_core::Runtime;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

/// Every `RuntimeEvent`, as the same JSON (`{"type": "run_updated", ...}`).
pub const RUNTIME_EVENT: &str = "runtime";
/// Events were dropped (the webview fell behind); the UI refetches everything.
pub const LAGGED_EVENT: &str = "runtime-lagged";

pub fn spawn_forwarder(app: AppHandle, rt: Arc<Runtime>) {
    let mut rx = rt.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => forward(&app, &ev),
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("UI event forwarder skipped {n} events");
                    let _ = app.emit(LAGGED_EVENT, ());
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn forward(app: &AppHandle, ev: &RuntimeEvent) {
    if let Err(e) = app.emit(RUNTIME_EVENT, ev) {
        tracing::warn!("emitting a runtime event failed: {e}");
    }
}
