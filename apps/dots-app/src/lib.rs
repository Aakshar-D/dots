pub mod app_frame;
pub mod autostart;
pub mod commands;
pub mod confirm;
pub mod contract;
pub mod error;
pub mod events;
pub mod notify_routes;
pub mod startup;
pub mod state;
pub mod tray;
pub mod update;
pub mod update_service;
pub mod viewing;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

use crate::state::AppState;

/// The commands of `commands.rs`, listed once: `invoke_handler` registers exactly these, and
/// `COMMANDS` names them in the contract fixture, where the UI's tests compare them with the
/// names `ui/src/api/commands.ts` invokes.
macro_rules! command_list {
    ($($name:ident),+ $(,)?) => {
        /// The name of every registered command, as the UI invokes it.
        pub const COMMANDS: &[&str] = &[ $( stringify!($name) ),+ ];

        fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![ $( commands::$name ),+ ]
        }
    };
}

command_list![
    startup_error,
    status,
    mark_claude_tested,
    get_settings,
    chatgpt_account,
    chatgpt_sign_in,
    chatgpt_sign_out,
    update_settings,
    set_schedules_paused,
    restart_app,
    update_note,
    update_open_notes,
    update_later,
    update_restart,
    update_stop_and_restart,
    update_cancel_wait,
    dot_overview,
    list_dots,
    get_dot,
    dot_variables,
    create_dot,
    update_dot,
    delete_dot,
    set_dot_enabled,
    regenerate_webhook_token,
    webhook_url,
    preview_schedule,
    policy_preset,
    new_dot_spec,
    test_endpoint,
    run_now,
    chat,
    cancel_run,
    list_runs,
    get_run,
    run_thread,
    list_run_events,
    approvals_for_run,
    decide_approval,
    approval_rule_candidates,
    decide_run_approvals,
    inbox,
    inbox_count,
    dismiss_schedule_attention,
    worktree_diff,
    discard_worktree,
    mark_reviewed,
    open_run_folder,
    library_list,
    library_add,
    library_update,
    library_remove,
    library_probe,
    library_check,
    confirm_origin,
    library_accept,
    library_scan_skills,
    skill_draft,
    skill_draft_cancel,
    skill_check,
    skill_create,
    context_documents,
    installed_claude_plugins,
    environment_variables,
    env_list,
    env_add,
    env_set,
    env_remove,
    env_import_candidates,
    env_import,
    env_import_dismiss,
    env_old_backups,
    env_old_backups_answer,
    org_list,
    org_preview,
    org_sops_key,
    org_join,
    org_update,
    org_refresh,
    org_remove,
    org_entries,
    messages_list,
    message_reply,
    message_dismiss,
    message_mark_read,
    follow_ups_list,
    follow_up_cancel,
    browser_open,
    browser_status,
    browser_screenshot,
    mark_dot_seen,
    set_viewing,
    unread_total,
    memories_list,
    memory_add,
    memory_update,
    memory_delete,
    apps_list,
    app_call,
    app_files,
    app_calls,
    app_set_every_dot,
    app_set_can_start_dots,
    app_roll_back,
    app_remove,
    app_proposals_list,
    app_proposal_files,
    app_proposal_diff,
    app_proposal_decide,
    app_preview_open,
    app_preview_call,
    app_preview_close,
    apps_kept_list,
    app_kept_delete,
    watchers_list,
    watcher_add,
    watcher_update,
    watcher_delete,
    watcher_set_enabled,
    watcher_resume,
    limits_get,
    usage_summary,
    run_release_hold,
];

/// Keeps the log writer flushing until the app exits.
struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

/// Keeps the lock on the data folder until the process ends.
struct DataDirLock(#[allow(dead_code)] std::fs::File);

/// Shows and focuses the main window.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    viewing::apply(app);
}

/// `panics_logged`: `run` has installed the panic hook already, so `setup` must not chain a
/// second one (one line per panic).
fn setup(app: &AppHandle, panics_logged: bool) -> Result<(), Box<dyn std::error::Error>> {
    // The main window is made here, not by Tauri, so it starts with `app_frame::browser_args`
    // (`additionalBrowserArgs` in the config cannot differ between dev and release).
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .ok_or("tauri.conf.json has no main window")?;
    tauri::WebviewWindowBuilder::from_config(app, config)?
        .additional_browser_args(&app_frame::browser_args(tauri::is_dev()))
        .build()?;
    // Without a data folder there is nowhere to log and nothing to start: the error goes to
    // the fatal-error page like any other failed start.
    let data_dir = startup::data_dir(
        app.path().local_data_dir(),
        std::env::var_os(startup::DATA_DIR_ENV),
    );
    let log_dir = data_dir.as_deref().ok().map(startup::log_dir);
    if let Some(log_dir) = &log_dir {
        if let Some(guard) = startup::init_logging(log_dir) {
            app.manage(LogGuard(guard));
        }
        if !panics_logged {
            startup::log_panics(log_dir.clone());
        }
    }
    tracing::info!("starting Dots {}", env!("CARGO_PKG_VERSION"));
    // The lock comes before the runtime touches the database; failing to get it is a failed
    // start like any other.
    let mut restarted = false;
    let runtime = match &data_dir {
        Ok(dir) => match startup::lock_data_dir(dir, startup::LOCK_WAIT) {
            Ok(lock) => {
                app.manage(DataDirLock(lock));
                // Only the Dots that holds the data folder takes the mark "Restart Dots" left.
                restarted = startup::take_restart_mark(dir);
                tauri::async_runtime::block_on(startup::start_runtime(dir))
            }
            Err(e) => Err(e),
        },
        Err(e) => Err(e.clone()),
    };
    let failed = runtime.is_err();
    if let Err(e) = &runtime {
        tracing::error!("{e}");
    }
    if let Ok((rt, settings)) = &runtime {
        app.manage(tray::Tray::build(app, settings.schedules_paused)?);
        autostart::apply(
            app,
            settings.autostart,
            !startup::single_instance(std::env::var_os(startup::DATA_DIR_ENV)),
        );
        // A click on a notification opens what it is about (needs-you spec section 4.2).
        notify_routes::listen(app);
        events::spawn_forwarder(app.clone(), rt.clone());
        if let Ok(dir) = &data_dir {
            // What a failed install left is read while Dots holds the data folder.
            app.manage(update_service::Updates::new(
                dir.clone(),
                update::take_error(dir),
            ));
            update_service::spawn(app.clone(), rt.clone());
        }
    }
    app.manage(dots_core::chatgpt::ChatGptAuth::from_env());
    app.manage(AppState::new(runtime.map(|(rt, _)| rt), log_dir).with_data_dir(data_dir.ok()));
    // A failed start always shows its error page, even when launched hidden; so does a start
    // that "Restart Dots" asked for.
    let hidden = startup::start_hidden(std::env::args());
    if startup::show_window(failed, hidden, restarted) {
        show_main(app);
    }
    Ok(())
}

pub fn run() {
    // First thing: a panic while Tauri builds the app or runs `setup` is logged too, and a
    // release build has no console to read it from.
    let early = startup::early_log_dir(
        std::env::var_os("LOCALAPPDATA"),
        std::env::var_os(startup::DATA_DIR_ENV),
    );
    let panics_logged = early.is_some();
    if let Some(log_dir) = early {
        startup::log_panics(log_dir);
    }
    // The dot the webview reports as open (needs-you spec section 5), for `viewing::apply`.
    let mut builder = tauri::Builder::default().manage(viewing::Reported::default());
    // Must be registered first: a second launch only focuses the running app. An instance
    // with its own data folder does not take part (`startup::single_instance`).
    if startup::single_instance(std::env::var_os(startup::DATA_DIR_ENV)) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }));
    }
    let app = builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![startup::HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // The pages of the apps' screens (app library spec section 5).
        .register_asynchronous_uri_scheme_protocol(app_frame::SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let rt = state::runtime_of(app.try_state::<AppState>().as_deref());
                responder.respond(app_frame::respond(rt.as_deref(), &request).await);
            });
        })
        .setup(move |app| {
            setup(app.handle(), panics_logged)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // The window behind, minimized or hidden: no conversation is in front, so nothing is
            // viewed, whatever the webview has said (needs-you spec section 2).
            if viewing::rechecks(event) {
                viewing::apply(window.app_handle());
            }
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
                    viewing::apply(app);
                } else {
                    app.exit(1);
                }
            }
        })
        .invoke_handler(invoke_handler())
        .build(tauri::generate_context!())
        .expect("failed to build the Dots app");
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(rt) = state::runtime_of(app.try_state::<AppState>().as_deref()) {
                rt.shutdown();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    /// The name of every function in `commands.rs` that is marked as a command. Only the plain
    /// attribute counts: with `rename`, the name the UI invokes would not be the function's
    /// name, which is what `COMMANDS` lists.
    fn defined() -> Vec<&'static str> {
        include_str!("commands.rs")
            .split("#[tauri::command]")
            .skip(1)
            .map(|after| {
                let name = after.split_once("fn ").expect("a command is a function").1;
                &name[..name.find('(').expect("a function has parameters")]
            })
            .collect()
    }

    #[test]
    fn every_command_in_commands_rs_is_registered_once() {
        let mut defined = defined();
        let mut registered = super::COMMANDS.to_vec();
        defined.sort_unstable();
        registered.sort_unstable();
        assert_eq!(registered, defined);
    }
}
