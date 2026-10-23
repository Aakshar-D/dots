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
    /// One button or run change at a time, so the hold always matches the state.
    turn: tokio::sync::Mutex<()>,
}

impl Updates {
    /// `error`: what the last start left (`update::take_error`).
    pub fn new(data_dir: PathBuf, error: Option<String>) -> Self {
        Self {
            state: Mutex::new(UpdateState::new(error)),
            downloaded: Mutex::new(None),
            data_dir,
            turn: tokio::sync::Mutex::new(()),
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

/// What a button and a run change do to the rest of Dots, so `act` and `settle` are tested
/// without the app. Only this crate implements it, so the futures need no `Send` bound in the
/// signature.
#[allow(async_fn_in_trait)]
pub trait Controls {
    /// Holds (`true`) or releases the run queue; holding waits for a dispatch pass in flight.
    async fn hold_runs(&self, held: bool);
    /// The dots with a run working now (`Runtime::working_dots`).
    async fn working_dots(&self) -> dots_core::Result<Vec<String>>;
    /// Tells the window the note that follows.
    fn announce(&self, note: &UpdateNote);
    /// Begins the install of the downloaded update. False when there is none to install.
    fn start_install(&self) -> bool;
}

/// The app's `Controls`: the runtime and the window.
struct AppControls<'a> {
    app: &'a AppHandle,
    rt: &'a Arc<Runtime>,
}

impl Controls for AppControls<'_> {
    async fn hold_runs(&self, held: bool) {
        self.rt.hold_runs(held).await;
    }

    async fn working_dots(&self) -> dots_core::Result<Vec<String>> {
        self.rt.working_dots().await
    }

    fn announce(&self, note: &UpdateNote) {
        if let Err(e) = self.app.emit(UPDATE_EVENT, note) {
            tracing::warn!("emitting the update note failed: {e}");
        }
    }

    fn start_install(&self) -> bool {
        start_install(self.app, self.rt)
    }
}

/// A button of the strip, applied by `act` with the app's controls.
pub async fn press(
    app: &AppHandle,
    rt: &Arc<Runtime>,
    button: impl FnOnce(&mut UpdateState, &[String]) -> dots_core::Result<Next>,
) -> Result<UpdateNote, CmdError> {
    let updates = updates(app)?;
    act(&AppControls { app, rt }, &updates, button).await
}

/// Applies a button of the strip, holds or releases the run queue to match, announces the
/// note and, when the install begins, starts it. Returns the note. The queue is held first:
/// once the hold returns no run can start, so the working dots the button decides on are
/// all there are. It stays held when the install begins, and follows the state otherwise.
/// Buttons are applied one at a time (and not during a run change), so the hold applied last
/// is the one the state asks for.
pub async fn act(
    controls: &impl Controls,
    updates: &Updates,
    button: impl FnOnce(&mut UpdateState, &[String]) -> dots_core::Result<Next>,
) -> Result<UpdateNote, CmdError> {
    let _turn = updates.turn.lock().await;
    controls.hold_runs(true).await;
    let working = match controls.working_dots().await {
        Ok(working) => working,
        Err(e) => {
            let held = updates.state.lock().unwrap().holds_runs();
            controls.hold_runs(held).await;
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
            controls.hold_runs(held).await;
            return Err(e.into());
        }
    };
    controls.hold_runs(held).await;
    controls.announce(&note);
    if next == Next::Install {
        if !controls.start_install() {
            return Ok(abandon(controls, updates, &working).await);
        }
    } else if updates.state.lock().unwrap().holds_runs() {
        // A wait whose runs ended meanwhile begins the install at once.
        settle(controls, updates).await;
    }
    Ok(note)
}

/// Why an install with no downloaded update did not begin.
const NO_DOWNLOAD: &str =
    "The downloaded update was lost, so it was not installed. Dots downloads it again.";

/// An install that could not begin: the state lets go of it, the queue is released and the
/// note says why. Returns the note. The caller holds the turn.
async fn abandon(controls: &impl Controls, updates: &Updates, working: &[String]) -> UpdateNote {
    tracing::error!("an install began without a downloaded update");
    let note = {
        let mut state = updates.state.lock().unwrap();
        state.abandon(NO_DOWNLOAD);
        state.note(working)
    };
    controls.hold_runs(false).await;
    controls.announce(&note);
    note
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
    let _turn = updates.turn.lock().await;
    settle(&AppControls { app, rt }, &updates).await;
}

/// The shared step of a run change and of a button that leaves a wait: when an update is
/// ready, reads the working dots, lets the state follow them, announces the note and begins
/// the install if the wait has nothing left to wait for. The caller holds the turn.
async fn settle(controls: &impl Controls, updates: &Updates) {
    if updates.state.lock().unwrap().ready_version().is_none() {
        return;
    }
    // Fails once an install has closed the database; there is nothing left to show then.
    let Ok(working) = controls.working_dots().await else {
        return;
    };
    let (next, note) = {
        let mut state = updates.state.lock().unwrap();
        (state.runs_changed(&working), state.note(&working))
    };
    if note != UpdateNote::None {
        controls.announce(&note);
    }
    if next == Next::Install && !controls.start_install() {
        abandon(controls, updates, &working).await;
    }
}

/// One check: a newer version is downloaded, its signature checked, and offered. Whether a
/// found version is kept is `UpdateState::wants`, tested in `update.rs`; the rest needs the
/// updater.
async fn check(app: &AppHandle, rt: &Arc<Runtime>, url: &Url) -> Result<(), String> {
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
    // The offer is taken or refused under one lock: a wait may have begun during the download.
    let shown = {
        let mut state = updates.state.lock().unwrap();
        if !state.wants(&version) {
            return Ok(());
        }
        *updates.downloaded.lock().unwrap() = Some(Downloaded {
            update: found,
            bytes,
        });
        state.offer(&version)
    };
    tracing::info!("Dots {version} is downloaded and ready to install");
    if shown {
        let working = match rt.working_dots().await {
            Ok(working) => working,
            Err(e) => {
                tracing::warn!("reading the working dots failed: {e}");
                Vec::new()
            }
        };
        let note = updates.state.lock().unwrap().note(&working);
        AppControls { app, rt }.announce(&note);
    }
    Ok(())
}

/// Spawns the install of the downloaded update. False when there is none.
fn start_install(app: &AppHandle, rt: &Arc<Runtime>) -> bool {
    let Ok(updates) = updates(app) else {
        return false;
    };
    let Some(downloaded) = updates.downloaded.lock().unwrap().take() else {
        return false;
    };
    let steps = AppSteps {
        app: app.clone(),
        rt: rt.clone(),
        data_dir: updates.data_dir.clone(),
        downloaded,
    };
    tauri::async_runtime::spawn(async move { update::install(&steps).await });
    true
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

    fn discard_backup(&self, backup: &std::path::Path) {
        if let Err(e) = std::fs::remove_file(backup) {
            tracing::warn!("could not delete the backup {}: {e}", backup.display());
        }
    }

    fn install(&self) -> Result<(), String> {
        tracing::info!("installing Dots {}", self.downloaded.update.version);
        self.downloaded
            .update
            .install(&self.downloaded.bytes)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::error::CmdErrorKind;

    /// Records what `act` and `settle` do. `working`: the answers of `working_dots`, in turn;
    /// the last one repeats.
    struct Fake {
        working: RefCell<Vec<Vec<String>>>,
        log: RefCell<Vec<String>>,
        /// Whether `start_install` finds a downloaded update.
        downloaded: bool,
    }

    impl Fake {
        fn answering(working: &[&[&str]]) -> Self {
            Self {
                working: RefCell::new(working.iter().map(|w| names(w)).collect()),
                log: RefCell::new(Vec::new()),
                downloaded: true,
            }
        }
        fn log(&self, entry: &str) {
            self.log.borrow_mut().push(entry.to_string());
        }
        /// The log since the last call.
        fn take(&self) -> Vec<String> {
            self.log.take()
        }
    }

    impl Controls for Fake {
        async fn hold_runs(&self, held: bool) {
            self.log(if held { "hold" } else { "release" });
        }
        async fn working_dots(&self) -> dots_core::Result<Vec<String>> {
            self.log("working");
            let mut working = self.working.borrow_mut();
            Ok(if working.len() > 1 {
                working.remove(0)
            } else {
                working[0].clone()
            })
        }
        fn announce(&self, note: &UpdateNote) {
            self.log(match note {
                UpdateNote::None => "announce none",
                UpdateNote::Ready { .. } => "announce ready",
                UpdateNote::Waiting { .. } => "announce waiting",
                UpdateNote::Installing { .. } => "announce installing",
                UpdateNote::Failed { .. } => "announce failed",
            });
        }
        fn start_install(&self) -> bool {
            self.log("install");
            self.downloaded
        }
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    /// Updates with version 0.2.0 ready.
    fn ready() -> Updates {
        let updates = Updates::new(PathBuf::new(), None);
        updates.state.lock().unwrap().offer("0.2.0");
        updates
    }

    fn restart(s: &mut UpdateState, working: &[String]) -> dots_core::Result<Next> {
        s.restart(working)
    }

    fn cancel(s: &mut UpdateState, _: &[String]) -> dots_core::Result<Next> {
        s.cancel_wait();
        Ok(Next::Nothing)
    }

    fn later(s: &mut UpdateState, _: &[String]) -> dots_core::Result<Next> {
        s.later();
        Ok(Next::Nothing)
    }

    fn ready_note(working: &[&str]) -> UpdateNote {
        UpdateNote::Ready {
            version: "0.2.0".into(),
            working: names(working),
            error: None,
        }
    }

    fn installing() -> UpdateNote {
        UpdateNote::Installing {
            version: "0.2.0".into(),
        }
    }

    #[tokio::test]
    async fn a_wait_whose_last_run_ended_meanwhile_installs_once_with_the_queue_held() {
        let updates = ready();
        // The run ends between the button's read and the settle after it.
        let fake = Fake::answering(&[&["triage"], &[]]);
        let note = act(&fake, &updates, restart).await.unwrap();
        assert_eq!(
            note,
            UpdateNote::Waiting {
                version: "0.2.0".into(),
                waiting_for: names(&["triage"]),
            }
        );
        assert_eq!(
            fake.take(),
            [
                "hold",
                "working",
                "hold",
                "announce waiting",
                "working",
                "announce installing",
                "install"
            ]
        );
        // The watcher sees the run's end too: the install is not begun again, and the queue
        // stays held.
        settle(&fake, &updates).await;
        assert_eq!(fake.take(), ["working", "announce installing"]);
        assert!(updates.state.lock().unwrap().holds_runs());
    }

    #[tokio::test]
    async fn cancel_after_a_wait_releases_the_queue() {
        let updates = ready();
        let fake = Fake::answering(&[&["triage"]]);
        act(&fake, &updates, restart).await.unwrap();
        // The wait settles at once: the run still works, so it waits on.
        assert_eq!(
            fake.take(),
            [
                "hold",
                "working",
                "hold",
                "announce waiting",
                "working",
                "announce waiting"
            ]
        );
        let note = act(&fake, &updates, cancel).await.unwrap();
        assert_eq!(note, ready_note(&["triage"]));
        assert_eq!(
            fake.take(),
            ["hold", "working", "release", "announce ready"]
        );
    }

    #[tokio::test]
    async fn a_stale_cancel_or_later_after_the_install_began_keeps_the_queue_held() {
        let updates = ready();
        let fake = Fake::answering(&[&[]]);
        act(&fake, &updates, restart).await.unwrap();
        fake.take();
        for button in [cancel, later] {
            let note = act(&fake, &updates, button).await.unwrap();
            assert_eq!(note, installing());
            let log = fake.take();
            assert_eq!(log[..4], ["hold", "working", "hold", "announce installing"]);
            let holds: Vec<&String> = log.iter().filter(|e| e.contains("hold")).collect();
            assert!(!log.iter().any(|e| e == "release"), "{log:?}");
            assert!(!log.iter().any(|e| e == "install"), "{log:?}");
            assert_eq!(holds.len(), 2, "{log:?}");
        }
    }

    #[tokio::test]
    async fn restart_with_nothing_working_installs_with_the_queue_held() {
        let updates = ready();
        let fake = Fake::answering(&[&[]]);
        let note = act(&fake, &updates, restart).await.unwrap();
        assert_eq!(note, installing());
        assert_eq!(
            fake.take(),
            ["hold", "working", "hold", "announce installing", "install"]
        );
    }

    #[tokio::test]
    async fn an_install_with_no_download_releases_the_queue_and_says_why() {
        let updates = ready();
        let fake = Fake {
            downloaded: false,
            ..Fake::answering(&[&[]])
        };
        let note = act(&fake, &updates, restart).await.unwrap();
        assert!(matches!(note, UpdateNote::Failed { .. }), "{note:?}");
        assert_eq!(
            fake.take(),
            [
                "hold",
                "working",
                "hold",
                "announce installing",
                "install",
                "release",
                "announce failed"
            ]
        );
        assert!(!updates.state.lock().unwrap().holds_runs());

        // The same from a wait whose last run ended.
        let updates = ready();
        let fake = Fake {
            downloaded: false,
            ..Fake::answering(&[&["triage"]])
        };
        act(&fake, &updates, restart).await.unwrap();
        fake.take();
        *fake.working.borrow_mut() = vec![Vec::new()];
        settle(&fake, &updates).await;
        assert_eq!(
            fake.take(),
            [
                "working",
                "announce installing",
                "install",
                "release",
                "announce failed"
            ]
        );
        assert!(!updates.state.lock().unwrap().holds_runs());
    }

    #[tokio::test]
    async fn stop_and_restart_installs_nothing_while_a_dot_the_confirm_did_not_name_works() {
        let updates = ready();
        let fake = Fake::answering(&[&["digest", "triage"]]);
        let seen = names(&["triage"]);
        let stop = |s: &mut UpdateState, working: &[String]| s.stop_and_restart(&seen, working);
        let note = act(&fake, &updates, stop).await.unwrap();
        assert_eq!(note, ready_note(&["digest", "triage"]));
        assert_eq!(
            fake.take(),
            ["hold", "working", "release", "announce ready"]
        );

        let seen = names(&["digest", "triage"]);
        let stop = |s: &mut UpdateState, working: &[String]| s.stop_and_restart(&seen, working);
        let note = act(&fake, &updates, stop).await.unwrap();
        assert_eq!(note, installing());
        assert_eq!(
            fake.take(),
            ["hold", "working", "hold", "announce installing", "install"]
        );
    }

    #[tokio::test]
    async fn a_button_with_no_update_ready_is_a_conflict_and_the_hold_follows_the_state() {
        let updates = Updates::new(PathBuf::new(), None);
        let fake = Fake::answering(&[&[]]);
        let err = act(&fake, &updates, restart).await.unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Conflict);
        assert_eq!(fake.take(), ["hold", "working", "release"]);

        // Once the install began, a second Restart is refused and the queue stays held.
        let updates = ready();
        act(&fake, &updates, restart).await.unwrap();
        fake.take();
        let err = act(&fake, &updates, restart).await.unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Conflict);
        assert_eq!(fake.take(), ["hold", "working", "hold"]);
    }
}
