pub mod autostart;
pub mod commands;
pub mod contract;
pub mod error;
pub mod events;
pub mod startup;
pub mod state;
pub mod tray;

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
    if let Ok((rt, settings)) = &runtime {
        app.manage(tray::Tray::build(app, settings.schedules_paused)?);
        autostart::apply(app, settings.autostart);
        events::spawn_forwarder(app.clone(), rt.clone());
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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![startup::HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // After a normal start, closing the window keeps Dots running in the tray. After a
            // failed start there is no tray, so closing the window is the only way out: exit.
            // `try_state`, because the state may not be managed yet.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if app
                    .try_state::<AppState>()
                    .is_some_and(|s| s.close_hides_window())
                {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    app.exit(1);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup_error,
            commands::status,
            commands::mark_claude_tested,
            commands::get_settings,
            commands::update_settings,
            commands::set_schedules_paused,
            commands::restart_app,
            commands::dot_overview,
            commands::list_dots,
            commands::get_dot,
            commands::create_dot,
            commands::update_dot,
            commands::delete_dot,
            commands::set_dot_enabled,
            commands::regenerate_webhook_token,
            commands::webhook_url,
            commands::preview_schedule,
            commands::test_endpoint,
            commands::run_now,
            commands::chat,
            commands::cancel_run,
            commands::list_runs,
            commands::get_run,
            commands::run_thread,
            commands::list_run_events,
            commands::approvals_for_run,
            commands::decide_approval,
            commands::inbox,
            commands::inbox_count,
            commands::worktree_diff,
            commands::discard_worktree,
            commands::mark_reviewed,
            commands::open_run_folder,
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
