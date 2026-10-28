//! Wires `update.rs` to `tauri-plugin-updater`, the runtime and the window: the checks, the
//! downloaded update, the strip's buttons and the install (updates spec §5-§7).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use dots_core::events::RuntimeEvent;
use dots_core::Runtime;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::broadcast::error::RecvError;

use crate::error::CmdError;
use crate::events::UPDATE_EVENT;
use crate::update::{
    self, idle_install_due, IdleInput, InstallSteps, Next, Purpose, UpdateNote, UpdateState,
};
use crate::update_record::{self, UpdateRecord};

/// A downloaded update whose signature the updater has checked.
struct Downloaded {
    update: Update,
    bytes: Vec<u8>,
    /// An update, a repair, or going back.
    purpose: Purpose,
    /// Going back: the update's backup it restores.
    backup: Option<PathBuf>,
}

/// Tauri-managed: what the strip shows, and the update it installs. Managed only when the
/// runtime started.
pub struct Updates {
    state: Mutex<UpdateState>,
    downloaded: Mutex<Option<Downloaded>>,
    /// The quiet copy an idle install downloaded (`/S`), kept until it installs.
    quiet: Mutex<Option<Downloaded>>,
    data_dir: PathBuf,
    /// One button or run change at a time, so the hold always matches the state.
    turn: tokio::sync::Mutex<()>,
    /// When a check last got an answer (RFC 3339 UTC), and why the last check failed.
    checked_at: Mutex<Option<String>>,
    check_error: Mutex<Option<String>>,
    /// The window's last keyboard or pointer input, and whether the composer holds unsent
    /// text (`report_activity`).
    activity: Mutex<(Instant, bool)>,
    /// Set once the window has rendered after this start (`app_rendered`).
    rendered: tokio::sync::watch::Sender<bool>,
}

impl Updates {
    /// `error`: what the last start left (`update::take_error`). `bad`: the versions this
    /// machine never takes again (`update_record`).
    pub fn new(data_dir: PathBuf, error: Option<String>, bad: Vec<String>) -> Self {
        Self {
            state: Mutex::new(UpdateState::new(error, bad)),
            downloaded: Mutex::new(None),
            quiet: Mutex::new(None),
            data_dir,
            turn: tokio::sync::Mutex::new(()),
            checked_at: Mutex::new(None),
            check_error: Mutex::new(None),
            activity: Mutex::new((Instant::now(), false)),
            rendered: tokio::sync::watch::Sender::new(false),
        }
    }

    /// The window had keyboard or pointer input, or the composer's text changed.
    pub fn report_activity(&self, composer_has_text: bool) {
        *self.activity.lock().unwrap() = (Instant::now(), composer_has_text);
    }

    /// The window rendered: the health check's last condition.
    pub fn report_rendered(&self) {
        self.rendered.send_replace(true);
    }

    fn activity(&self) -> (Duration, bool) {
        let (at, composer) = *self.activity.lock().unwrap();
        (at.elapsed(), composer)
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

/// What "Check for updates" found (updates UI spec section 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckOutcome {
    /// This copy never checks (a development build, or `DOTS_DATA_DIR` set).
    Off,
    UpToDate,
    Ready {
        version: String,
    },
    Failed {
        error: String,
    },
}

/// The outcome of a check that left `result`: the version ready after it, or why it failed.
pub fn outcome_of(checks_on: bool, result: &Result<Option<String>, String>) -> CheckOutcome {
    match (checks_on, result) {
        (false, _) => CheckOutcome::Off,
        (true, Ok(None)) => CheckOutcome::UpToDate,
        (true, Ok(Some(version))) => CheckOutcome::Ready {
            version: version.clone(),
        },
        (true, Err(error)) => CheckOutcome::Failed {
            error: error.clone(),
        },
    }
}

/// What Settings › Updates shows (updates UI spec section 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStatus {
    /// The version running.
    pub version: String,
    pub checks_on: bool,
    pub checked_at: Option<String>,
    /// The update downloaded and ready, if any.
    pub ready: Option<String>,
    pub last_error: Option<String>,
    /// The version Go back installs, while the last update's backup exists.
    pub go_back: Option<String>,
    pub auto_install: bool,
    /// This Dots publishes (updates UI spec section 5.1).
    pub publisher: bool,
    /// Why the last install, repair or go back did not happen.
    pub install_error: Option<String>,
}

/// A version's notes for "What's new".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseNotes {
    pub version: String,
    pub body: String,
    pub url: String,
}

pub const NOTHING_TO_GO_BACK: &str = "There is nothing to go back to.";
pub const NO_INSTALLER: &str = "This version has no published installer to repair from.";

/// Going back: the version it installs, the backup it restores, and whether there is a
/// runtime to close first (none on the fatal page).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoBackPlan {
    pub version: String,
    pub backup: PathBuf,
    pub close_runtime: bool,
}

pub fn go_back_plan(
    record: &UpdateRecord,
    current: &str,
    runtime_ok: bool,
) -> Result<GoBackPlan, String> {
    let (version, backup) = update_record::go_back_target(record, current)
        .ok_or_else(|| NOTHING_TO_GO_BACK.to_string())?;
    Ok(GoBackPlan {
        version,
        backup,
        close_runtime: runtime_ok,
    })
}

/// Whether a download is the one the state means to install: its purpose and version.
pub fn download_matches(purpose: Purpose, version: &str, state: &UpdateState) -> bool {
    state.ready_purpose() == Some((purpose, version))
}

/// Drops a download the state no longer means (a cancelled Repair or Go back).
pub fn clear_stale_download(app: &AppHandle) {
    let Ok(updates) = updates(app) else {
        return;
    };
    let mut slot = updates.downloaded.lock().unwrap();
    let stale = slot.as_ref().is_some_and(|d| {
        !download_matches(d.purpose, &d.update.version, &updates.state.lock().unwrap())
    });
    if stale {
        *slot = None;
    }
}

/// Repair downloads the running version's own installer, so it needs its release.
pub fn repair_allowed(release: Option<&dots_core::releases::Release>) -> Result<(), String> {
    release.map(|_| ()).ok_or_else(|| NO_INSTALLER.to_string())
}

fn conflict(message: impl Into<String>) -> CmdError {
    dots_core::Error::Conflict(message.into()).into()
}

fn checks_on() -> bool {
    update::checks_enabled(
        cfg!(debug_assertions),
        std::env::var_os(crate::startup::DATA_DIR_ENV),
    )
}

/// What Settings › Updates shows. `rt`: none after a failed start.
pub async fn status(app: &AppHandle, rt: Option<&Arc<Runtime>>) -> Result<UpdateStatus, CmdError> {
    let updates = updates(app)?;
    let record = update_record::read(&updates.data_dir);
    let auto_install = match rt {
        Some(rt) => rt.settings().await.auto_install_updates,
        None => false,
    };
    let ready = updates
        .state
        .lock()
        .unwrap()
        .ready_version()
        .map(str::to_string);
    let checked_at = updates.checked_at.lock().unwrap().clone();
    let last_error = updates.check_error.lock().unwrap().clone();
    let install_error = updates.state.lock().unwrap().install_error();
    Ok(UpdateStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        checks_on: checks_on(),
        checked_at,
        ready,
        last_error,
        publisher: crate::publishing::is_publisher(app),
        install_error,
        go_back: update_record::go_back_target(&record, env!("CARGO_PKG_VERSION"))
            .map(|(version, _)| version),
        auto_install,
    })
}

/// "Check for updates": a check now, its outcome kept for the status line.
pub async fn check_now(app: &AppHandle, rt: &Arc<Runtime>) -> CheckOutcome {
    if !checks_on() {
        return CheckOutcome::Off;
    }
    let result = match update::endpoint(std::env::var_os(update::ENDPOINT_ENV)) {
        Ok(url) => check(app, rt, &url).await,
        Err(e) => Err(e),
    };
    outcome_of(true, &result)
}

/// A version's notes, or `None` when it has no release.
pub async fn release_notes(version: &str) -> Result<Option<ReleaseNotes>, CmdError> {
    let release = dots_core::releases::fetch_release(dots_core::releases::API, version).await?;
    Ok(release.map(|r| ReleaseNotes {
        version: r.version,
        body: r.body,
        url: r.url,
    }))
}

/// Downloads `version`'s own installer (`releases/download/v<version>/latest.json`), whatever
/// its order against the version running: the same one for a repair, an older one for going
/// back. The signature is still the updater's to check.
async fn download_version(
    app: &AppHandle,
    version: &str,
    purpose: Purpose,
    backup: Option<PathBuf>,
) -> Result<Downloaded, String> {
    let url = Url::parse(&format!(
        "https://github.com/{}/releases/download/v{version}/latest.json",
        dots_core::releases::REPO
    ))
    .map_err(|e| e.to_string())?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map(|b| b.version_comparator(|_, _| true))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let found = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No installer was found for Dots {version}."))?;
    let bytes = found
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    Ok(Downloaded {
        update: found,
        bytes,
        purpose,
        backup,
    })
}

/// "Repair Dots": downloads this version again and installs it like an update, waiting for
/// working runs as "Restart to update" does.
pub async fn repair(app: &AppHandle, rt: &Arc<Runtime>) -> Result<UpdateNote, CmdError> {
    let current = env!("CARGO_PKG_VERSION");
    let release = dots_core::releases::fetch_release(dots_core::releases::API, current).await?;
    repair_allowed(release.as_ref()).map_err(conflict)?;
    let downloaded = download_version(app, current, Purpose::Repair, None)
        .await
        .map_err(conflict)?;
    // The download is kept in the same turn as the state change, so a check or a wait that
    // settles meanwhile can never swap it (final review I-7).
    let slot = updates(app)?;
    press(app, rt, move |s, working| {
        let next = s.begin_other(Purpose::Repair, current, working)?;
        *slot.downloaded.lock().unwrap() = Some(downloaded);
        Ok(next)
    })
    .await
}

/// "Go back": downloads the version before the last update and installs it with that
/// update's backup. With a runtime it waits for working runs like "Restart to update"; on the
/// fatal page (`rt` none) it begins at once.
pub async fn go_back(app: &AppHandle, rt: Option<Arc<Runtime>>) -> Result<UpdateNote, CmdError> {
    let updates = updates(app)?;
    let record = update_record::read(&updates.data_dir);
    let plan = go_back_plan(&record, env!("CARGO_PKG_VERSION"), rt.is_some()).map_err(conflict)?;
    let downloaded = download_version(
        app,
        &plan.version,
        Purpose::GoBack,
        Some(plan.backup.clone()),
    )
    .await
    .map_err(conflict)?;
    match rt {
        Some(rt) => {
            let version = plan.version.clone();
            let slot = &updates;
            press(app, &rt, move |s, working| {
                let next = s.begin_other(Purpose::GoBack, &version, working)?;
                *slot.downloaded.lock().unwrap() = Some(downloaded);
                Ok(next)
            })
            .await
        }
        None => {
            let steps = AppSteps {
                app: app.clone(),
                rt: None,
                data_dir: updates.data_dir.clone(),
                downloaded,
                quiet: false,
                stay_hidden: false,
                before: Mutex::new(None),
            };
            tauri::async_runtime::spawn(async move {
                update::install(&steps, Purpose::GoBack, Some(&plan.backup)).await
            });
            Ok(UpdateNote::Installing {
                version: plan.version,
                purpose: Purpose::GoBack,
            })
        }
    }
}

/// The idle install's view of the app (updates UI spec section 4.2), so `idle_tick` is tested
/// without it.
#[allow(async_fn_in_trait)]
pub trait IdleControls {
    /// "Install updates by itself when idle".
    async fn auto_install_on(&self) -> bool;
    /// Runs queued, running or waiting for an approval; an error counts as one.
    async fn unfinished_runs(&self) -> usize;
    /// The main window is hidden or minimised.
    fn window_hidden(&self) -> bool;
    async fn hold_runs(&self, held: bool);
    /// The version whose idle install failed before: never tried again by itself.
    fn idle_failed(&self) -> Option<String>;
    /// Downloads `version` again with the quiet installer and keeps it. False when it could
    /// not.
    async fn download_quiet(&self, version: &str) -> bool;
    /// Starts the install of what `download_quiet` kept. `stay_hidden`: the window was hidden,
    /// so the new Dots stays in the tray. False when it could not begin.
    fn start_quiet_install(&self, stay_hidden: bool) -> bool;
    /// Tells the window the note that follows.
    fn announce(&self, note: &UpdateNote);
}

/// Whether the idle install of the ready update is due now, and its version.
async fn idle_due(c: &impl IdleControls, updates: &Updates) -> Option<String> {
    let (ready, later, busy) = {
        let state = updates.state.lock().unwrap();
        (
            state
                .ready_purpose()
                .filter(|(p, _)| *p == Purpose::Update)
                .map(|(_, v)| v.to_string()),
            state.later_pressed(),
            state.holds_runs(),
        )
    };
    let version = ready?;
    if c.idle_failed().as_deref() == Some(version.as_str()) {
        return None;
    }
    let (input_idle, composer_has_text) = updates.activity();
    idle_install_due(&IdleInput {
        enabled: c.auto_install_on().await,
        ready: true,
        later,
        busy,
        unfinished_runs: c.unfinished_runs().await,
        composer_has_text,
        input_idle,
        window_hidden: c.window_hidden(),
    })
    .then_some(version)
}

/// One look at whether Dots may install the ready update by itself now; true when the
/// install began. The quiet installer is downloaded first, holding nothing; then, under the
/// turn with the queue held, everything is looked at again (runs, input, the composer,
/// "Later", the version), so a run or the person returning during the download stops it
/// (final review I-2).
pub async fn idle_tick(c: &impl IdleControls, updates: &Updates) -> bool {
    let Some(version) = ({
        let _turn = updates.turn.lock().await;
        idle_due(c, updates).await
    }) else {
        return false;
    };
    if !c.download_quiet(&version).await {
        tracing::warn!("downloading {version} for an idle install failed; tried again later");
        return false;
    }
    let _turn = updates.turn.lock().await;
    c.hold_runs(true).await;
    let still = idle_due(c, updates).await;
    if still.as_deref() != Some(version.as_str()) {
        c.hold_runs(false).await;
        return false;
    }
    let stay_hidden = c.window_hidden();
    if !matches!(
        updates.state.lock().unwrap().restart(&[]),
        Ok(Next::Install)
    ) {
        c.hold_runs(false).await;
        return false;
    }
    let installing = updates.state.lock().unwrap().note(&[]);
    c.announce(&installing);
    if c.start_quiet_install(stay_hidden) {
        return true;
    }
    updates.state.lock().unwrap().abandon(&format!(
        "Dots could not install {version} by itself. It tries again at the next check."
    ));
    c.hold_runs(false).await;
    false
}

/// Whether the window renders within `limit`.
pub async fn wait_rendered(updates: &Updates, limit: Duration) -> bool {
    let mut rx = updates.rendered.subscribe();
    tokio::time::timeout(limit, rx.wait_for(|done| *done))
        .await
        .is_ok_and(|r| r.is_ok())
}

/// The end of the health check of `version`: the record says whether it passed. Returns the
/// note it leaves: UpdatedItself after a passed idle install, Unhealthy after a failure,
/// None otherwise. The note is kept as the state's notice.
pub fn finish_health(updates: &Updates, version: &str, passed: bool) -> UpdateNote {
    let mut record = update_record::read(&updates.data_dir);
    let note = if passed {
        update_record::passed(&mut record);
        match record.by_itself_at.clone() {
            Some(at) => UpdateNote::UpdatedItself {
                version: version.to_string(),
                at,
            },
            None => UpdateNote::None,
        }
    } else {
        update_record::failed(&mut record, version);
        updates.state.lock().unwrap().mark_bad(version);
        UpdateNote::Unhealthy {
            version: version.to_string(),
            back_to: update_record::go_back_target(&record, version).map(|(v, _)| v),
        }
    };
    if let Err(e) = update_record::write(&updates.data_dir, &record) {
        tracing::warn!("could not write the update record: {e}");
    }
    let notice = (note != UpdateNote::None).then(|| note.clone());
    updates.state.lock().unwrap().set_notice(notice);
    note
}

/// The note for the version running when its developers withdrew it (`release` is its own
/// release), with Go back's version when the last update left a backup.
pub fn withdrawn_notice(
    release: Option<&dots_core::releases::Release>,
    record: &UpdateRecord,
) -> Option<UpdateNote> {
    let release = release.filter(|r| r.withdrawn)?;
    Some(UpdateNote::Withdrawn {
        version: release.version.clone(),
        back_to: update_record::go_back_target(record, &release.version).map(|(v, _)| v),
    })
}

/// After this start's record was read (`update_record::on_start`): a failed health check
/// shows Unhealthy; a check in progress waits for the window, at most `HEALTH_LIMIT`.
pub fn start_health(app: &AppHandle, health: update_record::Health) {
    let Ok(managed) = updates(app) else {
        return;
    };
    let record = update_record::read(&managed.data_dir);
    let notice = update_record::start_notice(&health, &record, env!("CARGO_PKG_VERSION"));
    match health {
        update_record::Health::Fine => {
            managed.state.lock().unwrap().set_notice(notice);
        }
        update_record::Health::Failed { version, .. } => {
            managed.state.lock().unwrap().mark_bad(&version);
            managed.state.lock().unwrap().set_notice(notice);
        }
        update_record::Health::Checking => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let Ok(updates) = updates(&app) else {
                    return;
                };
                let passed = wait_rendered(&updates, update_record::HEALTH_LIMIT).await;
                let note = finish_health(&updates, env!("CARGO_PKG_VERSION"), passed);
                if note != UpdateNote::None {
                    if let Err(e) = app.emit(UPDATE_EVENT, &note) {
                        tracing::warn!("emitting the update note failed: {e}");
                    }
                }
            });
        }
    }
}

/// "Dismiss" of "Dots updated itself": said once.
pub fn dismiss_updated(app: &AppHandle) -> Result<UpdateNote, CmdError> {
    let updates = updates(app)?;
    let mut record = update_record::read(&updates.data_dir);
    record.by_itself_at = None;
    update_record::write(&updates.data_dir, &record).map_err(CmdError::internal)?;
    let mut state = updates.state.lock().unwrap();
    if matches!(state.note(&[]), UpdateNote::UpdatedItself { .. }) {
        state.set_notice(None);
    }
    Ok(state.note(&[]))
}

/// The app's `IdleControls`.
struct AppIdle<'a> {
    app: &'a AppHandle,
    rt: &'a Arc<Runtime>,
}

impl IdleControls for AppIdle<'_> {
    async fn auto_install_on(&self) -> bool {
        self.rt.settings().await.auto_install_updates
    }

    async fn unfinished_runs(&self) -> usize {
        self.rt.unfinished_runs().await.unwrap_or(1)
    }

    fn window_hidden(&self) -> bool {
        self.app
            .get_webview_window("main")
            .is_none_or(|w| !w.is_visible().unwrap_or(false) || w.is_minimized().unwrap_or(false))
    }

    async fn hold_runs(&self, held: bool) {
        self.rt.hold_runs(held).await;
    }

    fn idle_failed(&self) -> Option<String> {
        let updates = updates(self.app).ok()?;
        update_record::read(&updates.data_dir).idle_failed
    }

    async fn download_quiet(&self, version: &str) -> bool {
        let Ok(updates) = updates(self.app) else {
            return false;
        };
        match download_quiet(self.app, version).await {
            Ok(downloaded) => {
                *updates.quiet.lock().unwrap() = Some(downloaded);
                true
            }
            Err(e) => {
                tracing::warn!("downloading {version} for an idle install failed: {e}");
                false
            }
        }
    }

    fn announce(&self, note: &UpdateNote) {
        if let Err(e) = self.app.emit(UPDATE_EVENT, note) {
            tracing::warn!("emitting the update note failed: {e}");
        }
    }

    fn start_quiet_install(&self, stay_hidden: bool) -> bool {
        let Ok(updates) = updates(self.app) else {
            return false;
        };
        let Some(downloaded) = updates.quiet.lock().unwrap().take() else {
            return false;
        };
        let steps = AppSteps {
            app: self.app.clone(),
            rt: Some(self.rt.clone()),
            data_dir: updates.data_dir.clone(),
            downloaded,
            quiet: true,
            stay_hidden,
            before: Mutex::new(None),
        };
        tauri::async_runtime::spawn(
            async move { update::install(&steps, Purpose::Update, None).await },
        );
        true
    }
}

/// The ready update again, from the usual endpoint, with the quiet installer (`/S`): the
/// updater fixes its installer's arguments when it checks, so a quiet install needs its own
/// download.
async fn download_quiet(app: &AppHandle, version: &str) -> Result<Downloaded, String> {
    let url = update::endpoint(std::env::var_os(update::ENDPOINT_ENV))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map(|b| b.installer_arg("/S"))
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let found = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .filter(|u| u.version == version)
        .ok_or_else(|| format!("Dots {version} is no longer offered."))?;
    let bytes = found
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    Ok(Downloaded {
        update: found,
        bytes,
        purpose: Purpose::Update,
        backup: None,
    })
}

/// A version's order: its number, then a release above its trials (`-rc.<n>`).
fn version_order(v: &str) -> Option<((u64, u64, u64), u32)> {
    let v = v.trim_start_matches('v');
    let (plain, rc) = match v.split_once("-rc.") {
        Some((plain, n)) => (plain, n.parse().ok()?),
        None => (v, u32::MAX),
    };
    Some((dots_core::publish::versions::parse(plain)?, rc))
}

/// For a publisher, the newest trial worth taking: a prerelease that is not withdrawn, not bad
/// on this machine, newer than the version running and than the newest release. Its own
/// manifest (updates UI spec section 5.5).
pub fn trial_endpoint(
    releases: &[dots_core::publish::github::ReleaseInfo],
    running: &str,
    bad: &[String],
) -> Option<Url> {
    let withdrawn = |r: &dots_core::publish::github::ReleaseInfo| {
        r.body
            .trim_start()
            .starts_with(dots_core::releases::WITHDRAWN_MARKER)
    };
    let latest = releases
        .iter()
        .filter(|r| !r.prerelease && !r.draft)
        .filter_map(|r| version_order(&r.tag))
        .max();
    let floor = std::cmp::max(version_order(running), latest)?;
    let trial = releases
        .iter()
        .filter(|r| r.prerelease && !r.draft && !withdrawn(r))
        .filter(|r| !bad.iter().any(|b| b == r.tag.trim_start_matches('v')))
        .filter_map(|r| Some((version_order(&r.tag)?, r)))
        .filter(|(order, _)| *order > floor)
        .max_by_key(|(order, _)| *order)?
        .1;
    Url::parse(&format!(
        "https://github.com/{}/releases/download/{}/latest.json",
        dots_core::releases::REPO,
        trial.tag
    ))
    .ok()
}

/// A publisher's trial endpoint now, if there is one to take.
async fn trial_url(app: &AppHandle, rt: &Arc<Runtime>, bad: &[String]) -> Option<Url> {
    if !crate::publishing::is_publisher(app) {
        return None;
    }
    let gh = crate::publishing::hub(app, rt).await.ok()?;
    let releases = gh.releases().await.ok()?;
    trial_endpoint(&releases, env!("CARGO_PKG_VERSION"), bad)
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
    let (idle_app, idle_rt) = (app.clone(), rt.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(IDLE_TICK).await;
            let Ok(updates) = updates(&idle_app) else {
                continue;
            };
            let controls = AppIdle {
                app: &idle_app,
                rt: &idle_rt,
            };
            if idle_tick(&controls, &updates).await {
                tracing::info!("installing the update by itself while Dots is idle");
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(update::FIRST_CHECK_AFTER).await;
        loop {
            if let Err(e) = check(&app, &rt, &url).await {
                tracing::warn!("checking for an update failed: {e}");
            }
            check_withdrawn(&app, &rt).await;
            tokio::time::sleep(update::CHECK_EVERY).await;
        }
    });
}

/// How often the idle install looks (updates UI spec section 4.2).
const IDLE_TICK: Duration = Duration::from_secs(60);

/// Whether the developers withdrew the version running: the strip offers Go back. A failed
/// read is logged; nothing is shown.
async fn check_withdrawn(app: &AppHandle, rt: &Arc<Runtime>) {
    let Ok(updates) = updates(app) else {
        return;
    };
    let release = match dots_core::releases::fetch_release(
        dots_core::releases::API,
        env!("CARGO_PKG_VERSION"),
    )
    .await
    {
        Ok(release) => release,
        Err(e) => {
            tracing::warn!("reading this version's release failed: {e}");
            return;
        }
    };
    let record = update_record::read(&updates.data_dir);
    let Some(notice) = withdrawn_notice(release.as_ref(), &record) else {
        return;
    };
    {
        let mut state = updates.state.lock().unwrap();
        if matches!(state.note(&[]), UpdateNote::Unhealthy { .. }) {
            return;
        }
        state.set_notice(Some(notice));
    }
    let working = rt.working_dots().await.unwrap_or_default();
    let note = updates.state.lock().unwrap().note(&working);
    AppControls { app, rt }.announce(&note);
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
async fn check(app: &AppHandle, rt: &Arc<Runtime>, url: &Url) -> Result<Option<String>, String> {
    let result = check_once(app, rt, url).await;
    if let Ok(updates) = updates(app) {
        match &result {
            Ok(_) => {
                *updates.checked_at.lock().unwrap() = Some(dots_core::util::now());
                *updates.check_error.lock().unwrap() = None;
            }
            Err(e) => *updates.check_error.lock().unwrap() = Some(e.clone()),
        }
    }
    result
}

async fn check_once(
    app: &AppHandle,
    rt: &Arc<Runtime>,
    url: &Url,
) -> Result<Option<String>, String> {
    // A publisher takes trials too (updates UI spec section 5.5).
    let bad = update_record::read(&updates(app).map_err(|e| e.message)?.data_dir).bad;
    let url = trial_url(app, rt, &bad)
        .await
        .unwrap_or_else(|| url.clone());
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .and_then(|b| b.build())
        .map_err(|e| e.to_string())?;
    let updates = updates(app).map_err(|e| e.message)?;
    let ready = || {
        updates
            .state
            .lock()
            .unwrap()
            .ready_version()
            .map(str::to_string)
    };
    let Some(found) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(ready());
    };
    if !updates.state.lock().unwrap().wants(&found.version) {
        return Ok(ready());
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
            return Ok(state.ready_version().map(str::to_string));
        }
        *updates.downloaded.lock().unwrap() = Some(Downloaded {
            update: found,
            bytes,
            purpose: Purpose::Update,
            backup: None,
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
    Ok(Some(version))
}

/// Spawns the install of the downloaded update. False when there is none.
fn start_install(app: &AppHandle, rt: &Arc<Runtime>) -> bool {
    let Ok(updates) = updates(app) else {
        return false;
    };
    let Some(downloaded) = updates.downloaded.lock().unwrap().take() else {
        return false;
    };
    // Only the download the state means is ever installed (final review C-1 and I-7).
    let meant = download_matches(
        downloaded.purpose,
        &downloaded.update.version,
        &updates.state.lock().unwrap(),
    );
    if !meant {
        tracing::error!("an install began with a download the state did not mean; dropped it");
        return false;
    }
    let (purpose, backup) = (downloaded.purpose, downloaded.backup.clone());
    let steps = AppSteps {
        app: app.clone(),
        rt: Some(rt.clone()),
        data_dir: updates.data_dir.clone(),
        downloaded,
        quiet: false,
        stay_hidden: false,
        before: Mutex::new(None),
    };
    tauri::async_runtime::spawn(async move {
        update::install(&steps, purpose, backup.as_deref()).await
    });
    true
}

struct AppSteps {
    app: AppHandle,
    /// None on the fatal page: there is no runtime to close.
    rt: Option<Arc<Runtime>>,
    data_dir: PathBuf,
    downloaded: Downloaded,
    /// An idle install: no installer window.
    quiet: bool,
    /// The window was hidden when the idle install began: the new Dots stays in the tray.
    stay_hidden: bool,
    /// The record as it was before `record`, for `restore_record`.
    before: Mutex<Option<UpdateRecord>>,
}

impl InstallSteps for AppSteps {
    async fn close_runtime(&self) -> bool {
        match &self.rt {
            Some(rt) => rt.close(update::CLOSE_LIMIT).await,
            None => true,
        }
    }

    async fn back_up(&self) -> Result<PathBuf, String> {
        // The backup the last update's Go back restores is never pruned (deferred minor M-4).
        let keep = update_record::read(&self.data_dir).backup;
        let made = update::back_up_keeping(
            &self.data_dir,
            env!("CARGO_PKG_VERSION"),
            Local::now(),
            keep.as_deref(),
        )
        .await?;
        tracing::info!("backed up the database to {}", made.display());
        Ok(made)
    }

    fn restart_current(&self, error: &str) {
        tracing::error!("the update was not installed: {error}");
        update::leave_error(&self.data_dir, error);
        if self.quiet {
            // An idle install that failed is never tried again by itself for this version,
            // and its restart keeps the window as it was (final review I-4).
            let mut record = update_record::read(&self.data_dir);
            record.idle_failed = Some(self.downloaded.update.version.clone());
            if let Err(e) = update_record::write(&self.data_dir, &record) {
                tracing::warn!("could not write the update record: {e}");
            }
        }
        self.mark_restart();
        self.app.restart();
    }

    fn mark_restart(&self) {
        if self.quiet && self.stay_hidden {
            crate::startup::mark_stay_hidden(&self.data_dir);
        } else {
            crate::startup::mark_restart(&self.data_dir);
        }
    }

    fn discard_backup(&self, backup: &Path) {
        if let Err(e) = std::fs::remove_file(backup) {
            tracing::warn!("could not delete the backup {}: {e}", backup.display());
        }
    }

    fn record(&self, purpose: Purpose, backup: Option<&Path>) -> Result<(), String> {
        let current = env!("CARGO_PKG_VERSION");
        let mut record = crate::update_record::read(&self.data_dir);
        *self.before.lock().unwrap() = Some(record.clone());
        match purpose {
            Purpose::GoBack => {
                update_record::went_back(&mut record);
                update_record::failed(&mut record, current);
            }
            Purpose::Repair => {
                // The earlier update's go-back target stays: going back from a repair goes to
                // the version before the last update, not to this one.
                record.installing = Some(current.to_string());
                record.pending = true;
                record.starts = 0;
                record.by_itself_at = None;
            }
            Purpose::Update => {
                record.leaving = Some(current.to_string());
                record.installing = Some(self.downloaded.update.version.clone());
                record.backup = backup.map(Path::to_path_buf);
                record.pending = true;
                record.starts = 0;
                record.by_itself_at = self.quiet.then(dots_core::util::now);
            }
        }
        crate::update_record::write(&self.data_dir, &record)
    }

    async fn set_aside_database(&self) -> Result<PathBuf, String> {
        update::set_aside_database(&self.data_dir, env!("CARGO_PKG_VERSION"), Local::now())
    }

    async fn restore_backup(&self, backup: &Path) -> Result<(), String> {
        std::fs::copy(backup, self.data_dir.join("dots.db"))
            .map(|_| ())
            .map_err(|e| format!("copying {}: {e}", backup.display()))
    }

    fn undo_restore(&self, set_aside: &Path) {
        if let Err(e) = update::undo_set_aside(&self.data_dir, set_aside) {
            tracing::error!(
                "could not put the database back from {}: {e}",
                set_aside.display()
            );
        }
    }

    fn restore_record(&self) {
        if let Some(before) = self.before.lock().unwrap().take() {
            if let Err(e) = update_record::write(&self.data_dir, &before) {
                tracing::warn!("could not put the update record back: {e}");
            }
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
                UpdateNote::Unhealthy { .. } => "announce unhealthy",
                UpdateNote::Withdrawn { .. } => "announce withdrawn",
                UpdateNote::UpdatedItself { .. } => "announce updated",
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
        let updates = Updates::new(PathBuf::new(), None, Vec::new());
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
            purpose: update::Purpose::Update,
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
                purpose: update::Purpose::Update,
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
        let updates = Updates::new(PathBuf::new(), None, Vec::new());
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

    #[test]
    fn check_now_reports_off_up_to_date_ready_or_the_error() {
        assert_eq!(outcome_of(false, &Ok(None)), CheckOutcome::Off);
        assert_eq!(outcome_of(true, &Ok(None)), CheckOutcome::UpToDate);
        assert_eq!(
            outcome_of(true, &Ok(Some("0.3.1".into()))),
            CheckOutcome::Ready {
                version: "0.3.1".into()
            }
        );
        // A bad version is never kept, so a check that found only it is up to date.
        let mut state = UpdateState::new(None, vec!["0.3.1".into()]);
        assert!(!state.offer("0.3.1"));
        assert_eq!(
            outcome_of(true, &Ok(state.ready_version().map(str::to_string))),
            CheckOutcome::UpToDate
        );
        assert_eq!(
            outcome_of(true, &Err("offline".into())),
            CheckOutcome::Failed {
                error: "offline".into()
            }
        );
    }

    #[test]
    fn go_back_is_offered_only_with_a_backup_and_works_without_a_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("dots-0.3.0-2026-10-27-031200.db");
        std::fs::write(&backup, "db").unwrap();
        let record = crate::update_record::UpdateRecord {
            leaving: Some("0.3.0".into()),
            installing: Some("0.3.1".into()),
            backup: Some(backup.clone()),
            ..Default::default()
        };
        let plan = go_back_plan(&record, "0.3.1", true).unwrap();
        assert_eq!((plan.version.as_str(), plan.close_runtime), ("0.3.0", true));
        assert_eq!(plan.backup, backup);
        // From the fatal page there is no runtime to close.
        assert!(!go_back_plan(&record, "0.3.1", false).unwrap().close_runtime);
        std::fs::remove_file(&backup).unwrap();
        assert_eq!(
            go_back_plan(&record, "0.3.1", true).err().as_deref(),
            Some("There is nothing to go back to.")
        );
    }

    #[test]
    fn repair_needs_a_published_release() {
        assert_eq!(
            repair_allowed(None),
            Err("This version has no published installer to repair from.".to_string())
        );
        let release = dots_core::releases::Release {
            version: "0.3.0".into(),
            name: "Dots 0.3.0".into(),
            body: String::new(),
            url: String::new(),
            prerelease: false,
            withdrawn: false,
        };
        assert_eq!(repair_allowed(Some(&release)), Ok(()));
    }

    /// The idle side of the app: the setting on, `runs` answered in turn, `touch` (when set)
    /// reporting input during the download.
    struct FakeIdle<'a> {
        runs: RefCell<Vec<usize>>,
        log: RefCell<Vec<String>>,
        install_ok: bool,
        hidden: bool,
        failed: Option<String>,
        touch: Option<&'a Updates>,
    }

    impl<'a> FakeIdle<'a> {
        fn new(runs: &[usize], install_ok: bool) -> Self {
            Self {
                runs: RefCell::new(runs.iter().rev().copied().collect()),
                log: RefCell::new(Vec::new()),
                install_ok,
                hidden: true,
                failed: None,
                touch: None,
            }
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.borrow_mut())
        }
    }

    impl IdleControls for FakeIdle<'_> {
        async fn auto_install_on(&self) -> bool {
            true
        }
        async fn unfinished_runs(&self) -> usize {
            self.runs.borrow_mut().pop().unwrap_or(0)
        }
        fn window_hidden(&self) -> bool {
            self.hidden
        }
        async fn hold_runs(&self, held: bool) {
            self.log.borrow_mut().push(format!("hold {held}"));
        }
        fn idle_failed(&self) -> Option<String> {
            self.failed.clone()
        }
        async fn download_quiet(&self, version: &str) -> bool {
            self.log.borrow_mut().push(format!("download {version}"));
            if let Some(updates) = self.touch {
                updates.report_activity(false);
            }
            true
        }
        fn announce(&self, note: &UpdateNote) {
            if matches!(note, UpdateNote::Installing { .. }) {
                self.log.borrow_mut().push("announce installing".into());
            }
        }
        fn start_quiet_install(&self, stay_hidden: bool) -> bool {
            let shown = if stay_hidden { "hidden" } else { "shown" };
            self.log.borrow_mut().push(format!("quiet install {shown}"));
            self.install_ok
        }
    }

    #[tokio::test]
    async fn an_idle_install_rechecks_the_runs_under_the_turn_before_it_begins() {
        let updates = ready();
        // No run when the tick decides, one at the recheck after the download: no install.
        let fake = FakeIdle::new(&[0, 1], true);
        assert!(!idle_tick(&fake, &updates).await);
        assert_eq!(fake.take(), ["download 0.2.0", "hold true", "hold false"]);
        assert!(matches!(
            updates.state.lock().unwrap().note(&[]),
            UpdateNote::Ready { .. }
        ));
    }

    // Final review I-2: the person returning during the download stops the install.
    #[tokio::test]
    async fn input_during_the_quiet_download_stops_the_idle_install() {
        let updates = ready();
        *updates.activity.lock().unwrap() = (Instant::now() - update::IDLE_AFTER, false);
        let mut fake = FakeIdle::new(&[0, 0], true);
        fake.hidden = false;
        fake.touch = Some(&updates);
        assert!(!idle_tick(&fake, &updates).await);
        assert_eq!(fake.take(), ["download 0.2.0", "hold true", "hold false"]);
    }

    #[tokio::test]
    async fn an_idle_install_is_quiet_and_keeps_the_window_as_it_was() {
        let updates = ready();
        let fake = FakeIdle::new(&[0, 0], true);
        assert!(idle_tick(&fake, &updates).await);
        assert_eq!(
            fake.take(),
            [
                "download 0.2.0",
                "hold true",
                "announce installing",
                "quiet install hidden"
            ]
        );
        assert!(matches!(
            updates.state.lock().unwrap().note(&[]),
            UpdateNote::Installing { .. }
        ));
        // Final review I-3: a window left open but untouched comes back shown.
        let updates = ready();
        *updates.activity.lock().unwrap() = (Instant::now() - update::IDLE_AFTER, false);
        let mut fake = FakeIdle::new(&[0, 0], true);
        fake.hidden = false;
        assert!(idle_tick(&fake, &updates).await);
        assert_eq!(
            fake.take(),
            [
                "download 0.2.0",
                "hold true",
                "announce installing",
                "quiet install shown"
            ]
        );
        // "Later" stops it for the rest of the session.
        let updates = ready();
        updates.state.lock().unwrap().later();
        let fake = FakeIdle::new(&[0, 0], true);
        assert!(!idle_tick(&fake, &updates).await);
        assert!(fake.take().is_empty());
        // An install that cannot begin lets the queue go and says why.
        let updates = ready();
        let fake = FakeIdle::new(&[0, 0], false);
        assert!(!idle_tick(&fake, &updates).await);
        assert_eq!(
            fake.take(),
            [
                "download 0.2.0",
                "hold true",
                "announce installing",
                "quiet install hidden",
                "hold false"
            ]
        );
        assert!(matches!(
            updates.state.lock().unwrap().note(&[]),
            UpdateNote::Failed { .. }
        ));
    }

    // Final review I-4: a version whose idle install failed is not tried again by itself.
    #[tokio::test]
    async fn a_failed_idle_install_of_a_version_is_not_tried_again() {
        let updates = ready();
        let mut fake = FakeIdle::new(&[0, 0], true);
        fake.failed = Some("0.2.0".into());
        assert!(!idle_tick(&fake, &updates).await);
        assert!(fake.take().is_empty());
    }

    #[tokio::test]
    async fn the_health_check_passes_once_the_window_has_rendered_and_fails_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("dots-0.1.0-2026-10-27-031200.db");
        std::fs::write(&backup, "db").unwrap();
        let record = UpdateRecord {
            leaving: Some("0.1.0".into()),
            installing: Some("0.2.0".into()),
            backup: Some(backup),
            pending: true,
            starts: 1,
            by_itself_at: Some("2026-10-27T03:12:00.000Z".into()),
            ..Default::default()
        };
        update_record::write(dir.path(), &record).unwrap();
        let updates = Updates::new(dir.path().to_path_buf(), None, Vec::new());
        let sender = updates.rendered.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            sender.send_replace(true);
        });
        assert!(wait_rendered(&updates, Duration::from_secs(5)).await);
        let note = finish_health(&updates, "0.2.0", true);
        assert!(!update_record::read(dir.path()).pending);
        assert_eq!(
            note,
            UpdateNote::UpdatedItself {
                version: "0.2.0".into(),
                at: "2026-10-27T03:12:00.000Z".into()
            }
        );

        update_record::write(dir.path(), &record).unwrap();
        let updates = Updates::new(dir.path().to_path_buf(), None, Vec::new());
        assert!(!wait_rendered(&updates, Duration::from_millis(50)).await);
        let note = finish_health(&updates, "0.2.0", false);
        assert_eq!(
            update_record::read(dir.path()).bad,
            vec!["0.2.0".to_string()]
        );
        assert_eq!(
            note,
            UpdateNote::Unhealthy {
                version: "0.2.0".into(),
                back_to: Some("0.1.0".into())
            }
        );
        assert!(!updates.state.lock().unwrap().wants("0.2.0"));
    }

    #[test]
    fn a_withdrawn_running_version_offers_go_back() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("dots-0.1.0-2026-10-27-031200.db");
        std::fs::write(&backup, "db").unwrap();
        let record = UpdateRecord {
            leaving: Some("0.1.0".into()),
            installing: Some("0.2.0".into()),
            backup: Some(backup),
            ..Default::default()
        };
        let release = |withdrawn| dots_core::releases::Release {
            version: "0.2.0".into(),
            name: "Dots 0.2.0".into(),
            body: String::new(),
            url: String::new(),
            prerelease: withdrawn,
            withdrawn,
        };
        assert_eq!(withdrawn_notice(None, &record), None);
        assert_eq!(withdrawn_notice(Some(&release(false)), &record), None);
        assert_eq!(
            withdrawn_notice(Some(&release(true)), &record),
            Some(UpdateNote::Withdrawn {
                version: "0.2.0".into(),
                back_to: Some("0.1.0".into())
            })
        );
        assert_eq!(
            withdrawn_notice(Some(&release(true)), &UpdateRecord::default()),
            Some(UpdateNote::Withdrawn {
                version: "0.2.0".into(),
                back_to: None
            })
        );
    }

    // Final review C-1 and I-7: an install only ever installs the download the state means.
    #[test]
    fn an_install_refuses_a_download_the_state_does_not_mean() {
        let mut state = UpdateState::new(None, vec![]);
        state.offer("0.3.2");
        assert!(download_matches(Purpose::Update, "0.3.2", &state));
        assert!(!download_matches(Purpose::GoBack, "0.3.0", &state));
        assert!(!download_matches(Purpose::Update, "0.3.1", &state));
        state
            .begin_other(Purpose::GoBack, "0.3.0", &names(&["digest"]))
            .unwrap();
        assert!(download_matches(Purpose::GoBack, "0.3.0", &state));
        assert!(!download_matches(Purpose::Update, "0.3.2", &state));
        state.cancel_wait();
        assert!(!download_matches(Purpose::GoBack, "0.3.0", &state));
    }

    #[test]
    fn a_publisher_takes_the_newest_good_trial() {
        use dots_core::publish::github::ReleaseInfo;
        let r = |tag: &str, prerelease: bool, body: &str| ReleaseInfo {
            id: 1,
            tag: tag.into(),
            name: tag.into(),
            body: body.into(),
            prerelease,
            draft: false,
            url: String::new(),
            published_at: None,
        };
        let url = |tag: &str| {
            Some(
                Url::parse(&format!(
                    "https://github.com/Aakshar-D/dots/releases/download/{tag}/latest.json"
                ))
                .unwrap(),
            )
        };
        let releases = vec![
            r("v0.3.0-rc.2", true, ""),
            r("v0.3.0-rc.1", true, ""),
            r("v0.2.0", false, ""),
        ];
        assert_eq!(trial_endpoint(&releases, "0.2.0", &[]), url("v0.3.0-rc.2"));
        // Bad here: the one before it.
        assert_eq!(
            trial_endpoint(&releases, "0.2.0", &["0.3.0-rc.2".into()]),
            url("v0.3.0-rc.1")
        );
        // Withdrawn: not taken.
        let withdrawn = vec![
            r("v0.3.0-rc.1", true, "<!-- dots:withdrawn -->\n- x"),
            r("v0.2.0", false, ""),
        ];
        assert_eq!(trial_endpoint(&withdrawn, "0.2.0", &[]), None);
        // Older than the version running, or than the newest release: not taken.
        assert_eq!(trial_endpoint(&releases, "0.3.0-rc.2", &[]), None);
        let promoted = vec![r("v0.3.0", false, ""), r("v0.3.0-rc.2", true, "")];
        assert_eq!(trial_endpoint(&promoted, "0.2.0", &[]), None);
    }
}
