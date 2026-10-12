pub mod commands;
pub mod error;
pub mod startup;
pub mod state;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

use crate::state::AppState;

/// Keeps the log writer flushing until the app exits.
struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

/// Shows and focuses the main window.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = startup::data_dir(
        &app.path().local_data_dir()?,
        std::env::var_os(startup::DATA_DIR_ENV),
    );
    let log_dir = startup::log_dir(&data_dir);
    if let Some(guard) = startup::init_logging(&log_dir) {
        app.manage(LogGuard(guard));
    }
    tracing::info!("starting Dots {}", env!("CARGO_PKG_VERSION"));
    let runtime = tauri::async_runtime::block_on(startup::start_runtime(&data_dir));
    let failed = runtime.is_err();
    if let Err(e) = &runtime {
        tracing::error!("{e}");
    }
    app.manage(AppState::new(runtime.map(|(rt, _)| rt), log_dir));
    // A failed start always shows its error page, even when launched hidden.
    if failed || !startup::start_hidden(std::env::args()) {
        show_main(app);
    }
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        // Must be registered first: a second launch only focuses the running app.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps Dots running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup_error,
            commands::status,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Dots app");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Ok(rt) = app.state::<AppState>().rt() {
                rt.shutdown();
            }
        }
    });
}
