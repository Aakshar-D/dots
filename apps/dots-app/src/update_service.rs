//! Wires `update.rs` to `tauri-plugin-updater`, the runtime and the window: the checks, the
//! downloaded update, the strip's buttons and the install (updates spec §5-§7).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Local;
use dots_core::events::RuntimeEvent;
use dots_core::Runtime;
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::broadcast::error::RecvError;

use crate::error::CmdError;
use crate::events::UPDATE_EVENT;
use crate::update::{self, InstallSteps, Next, UpdateNote, UpdateState};

/// A downloaded update whose signature the updater has checked.
struct Downloaded {
    update: Update,
    bytes: Vec<u8>,
}

/// Tauri-managed: what the strip shows, and the update it installs. Managed only when the
/// runtime started.
pub struct Updates {
    state: Mutex<UpdateState>,
    downloaded: Mutex<Option<Downloaded>>,
    data_dir: PathBuf,
}

impl Updates {
    /// `error`: what the last start left (`update::take_error`).
    pub fn new(data_dir: PathBuf, error: Option<String>) -> Self {
        Self {
            state: Mutex::new(UpdateState::new(error)),
            downloaded: Mutex::new(None),
            data_dir,
        }
    }
}

fn updates(app: &AppHandle) -> Result<tauri::State<'_, Updates>, CmdError> {
    app.try_state::<Updates>()
        .ok_or_else(|| CmdError::internal("Dots did not start; see the startup error"))
}

/// The note as it stands, with the dots that have a run working now.
pub async fn note(app: &AppHandle, rt: &Arc<Runtime>) -> Result<UpdateNote, CmdError> {
    let working = rt.working_dots().await?;
    Ok(updates(app)?.state.lock().unwrap().note(&working))
}

/// The release page of the update that is ready ("What's new").
pub fn release_page(app: &AppHandle) -> Result<String, CmdError> {
    let updates = updates(app)?;
    let state = updates.state.lock().unwrap();
    match state.ready_version() {
        Some(version) => Ok(update::release_page(version)),
        None => Err(dots_core::Error::Conflict("no update is ready".into()).into()),
    }
}

/// Applies a button of the strip, holds or releases the run queue to match, announces the
/// note and, when the install begins, starts it. Returns the note. The queue is held first:
/// once the hold returns no run can start, so the working dots the button decides on are
/// all there are. It stays held when the install begins, and follows the state otherwise.
pub async fn act(
    app: &AppHandle,
    rt: &Arc<Runtime>,
    button: impl FnOnce(&mut UpdateState, &[String]) -> dots_core::Result<Next>,
) -> Result<UpdateNote, CmdError> {
    let updates = updates(app)?;
    rt.hold_runs(true).await;
    let working = match rt.working_dots().await {
        Ok(working) => working,
        Err(e) => {
            let held = updates.state.lock().unwrap().holds_runs();
            rt.hold_runs(held).await;
            return Err(e.into());
        }
    };
    let decided = {
        let mut state = updates.state.lock().unwrap();
        button(&mut state, &working).map(|next| {
            let held = state.holds_runs() || next == Next::Install;
            (next, held, state.note(&working))
        })
    };
    let (next, held, note) = match decided {
        Ok(decided) => decided,
        Err(e) => {
            let held = updates.state.lock().unwrap().holds_runs();
            rt.hold_runs(held).await;
            return Err(e.into());
        }
    };
    rt.hold_runs(held).await;
    announce(app, &note);
    if next == Next::Install {
        start_install(app, rt);
    }
    Ok(note)
}

fn announce(app: &AppHandle, note: &UpdateNote) {
    if let Err(e) = app.emit(UPDATE_EVENT, note) {
        tracing::warn!("emitting the update note failed: {e}");
    }
}

/// Starts the watch on runs and, unless this Dots does not check (`update::checks_enabled`),
/// the checks.
pub fn spawn(app: AppHandle, rt: Arc<Runtime>) {
    let mut rx = rt.subscribe();
    let watched = (app.clone(), rt.clone());
    tauri::async_runtime::spawn(async move {
        let (app, rt) = watched;
        loop {
            match rx.recv().await {
                Ok(RuntimeEvent::RunUpdated { .. }) | Err(RecvError::Lagged(_)) => {
                    runs_changed(&app, &rt).await
                }
                Ok(_) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
    if !update::checks_enabled(
        cfg!(debug_assertions),
        std::env::var_os(crate::startup::DATA_DIR_ENV),
    ) {
        tracing::info!("update checks are off in this build or under DOTS_DATA_DIR");
        return;
    }
    let url = match update::endpoint(std::env::var_os(update::ENDPOINT_ENV)) {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!("update checks are off: {e}");
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(update::FIRST_CHECK_AFTER).await;
        loop {
            if let Err(e) = check(&app, &rt, &url).await {
                tracing::warn!("checking for an update failed: {e}");
            }
            tokio::time::sleep(update::CHECK_EVERY).await;
        }
    });
}

/// A run changed: the note's list of working dots follows, and a waiting install begins once
/// none is left.
async fn runs_changed(app: &AppHandle, rt: &Arc<Runtime>) {
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    // Fails once an install has closed the database; there is nothing left to show then.
    let Ok(working) = rt.working_dots().await else {
        return;
    };
    let (next, note) = {
        let mut state = updates.state.lock().unwrap();
        (state.runs_changed(&working), state.note(&working))
    };
    if note != UpdateNote::None {
        announce(app, &note);
    }
    if next == Next::Install {
        start_install(app, rt);
    }
}

/// One check: a newer version is downloaded, its signature checked, and offered.
async fn check(app: &AppHandle, rt: &Runtime, url: &Url) -> Result<(), String> {
    let updater = app
        .updater_builder()
        .endpoints(vec![url.clone()])
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let Some(found) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let updates = updates(app).map_err(|e| e.message)?;
    if !updates.state.lock().unwrap().wants(&found.version) {
        return Ok(());
    }
    let bytes = found
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    let version = found.version.clone();
    *updates.downloaded.lock().unwrap() = Some(Downloaded {
        update: found,
        bytes,
    });
    let shown = updates.state.lock().unwrap().offer(&version);
    tracing::info!("Dots {version} is downloaded and ready to install");
    if shown {
        let working = rt.working_dots().await.unwrap_or_default();
        let note = updates.state.lock().unwrap().note(&working);
        announce(app, &note);
    }
    Ok(())
}

fn start_install(app: &AppHandle, rt: &Arc<Runtime>) {
    let Ok(updates) = updates(app) else {
        return;
    };
    let Some(downloaded) = updates.downloaded.lock().unwrap().take() else {
        tracing::error!("an install began without a downloaded update");
        return;
    };
    let steps = AppSteps {
        app: app.clone(),
        rt: rt.clone(),
        data_dir: updates.data_dir.clone(),
        downloaded,
    };
    tauri::async_runtime::spawn(async move { update::install(&steps).await });
}

struct AppSteps {
    app: AppHandle,
    rt: Arc<Runtime>,
    data_dir: PathBuf,
    downloaded: Downloaded,
}

impl InstallSteps for AppSteps {
    async fn close_runtime(&self) -> bool {
        self.rt.close(update::CLOSE_LIMIT).await
    }

    async fn back_up(&self) -> Result<PathBuf, String> {
        let made = update::back_up(&self.data_dir, env!("CARGO_PKG_VERSION"), Local::now()).await?;
        tracing::info!("backed up the database to {}", made.display());
        Ok(made)
    }

    fn restart_current(&self, error: &str) {
        tracing::error!("the update was not installed: {error}");
        update::leave_error(&self.data_dir, error);
        crate::startup::mark_restart(&self.data_dir);
        self.app.restart();
    }

    fn mark_restart(&self) {
        crate::startup::mark_restart(&self.data_dir);
    }

    fn install(&self) -> Result<(), String> {
        tracing::info!("installing Dots {}", self.downloaded.update.version);
        self.downloaded
            .update
            .install(&self.downloaded.bytes)
            .map_err(|e| e.to_string())
    }
}
