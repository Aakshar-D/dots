use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

/// Registers or removes the login entry (which starts Dots with `--hidden`). Debug builds
/// leave it alone: the entry would point at a development binary.
pub fn apply(app: &AppHandle, enabled: bool) {
    if cfg!(debug_assertions) {
        tracing::info!("debug build: autostart left unchanged (wanted {enabled})");
        return;
    }
    let launcher = app.autolaunch();
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    if let Err(e) = result {
        tracing::warn!("updating autostart failed: {e}");
    }
}
