//! Updates for installed Dots (updates spec): what the update strip shows and what its
//! buttons do, when Dots checks, the backup taken before an install, and the order an install
//! goes in. Free of the updater plugin, so all of it is tested without the network; the app
//! wires it to `tauri-plugin-updater` (`update_service.rs`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use tauri::Url;

/// Where every installed Dots looks for a new version (spec §4).
pub const DEFAULT_ENDPOINT: &str =
    "https://github.com/Aakshar-D/dots/releases/latest/download/latest.json";
/// Points a release build at another manifest, such as a prerelease's (spec §4).
pub const ENDPOINT_ENV: &str = "DOTS_UPDATE_ENDPOINT";
/// The first check after a start, then one every `CHECK_EVERY` (spec §5).
pub const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How long an install waits for the database to close (spec §7).
pub const CLOSE_LIMIT: Duration = Duration::from_secs(10);
/// The backups folder inside the data folder, and how many backups it keeps (spec §7).
pub const BACKUPS_DIR: &str = "backups";
pub const KEEP_BACKUPS: usize = 3;
/// Why the last install did not happen, left for the next start (spec §7).
pub const ERROR_MARK: &str = "update-error.txt";

/// What the update strip shows (spec §6). The UI gets it from `update_note` and the
/// `update` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateNote {
    /// No update is ready, or the person chose "Later".
    None,
    /// `working`: the dots with a run working now. `error`: why the last install did not
    /// happen, when it did not.
    Ready {
        version: String,
        working: Vec<String>,
        error: Option<String>,
    },
    /// "Restart when it's done": Dots installs once no dot has a run working. `purpose`: an
    /// update, a repair or going back (updates UI spec section 4).
    Waiting {
        version: String,
        waiting_for: Vec<String>,
        purpose: Purpose,
    },
    /// The install has begun; Dots is about to close.
    Installing { version: String, purpose: Purpose },
    /// The new version did not pass its health check (updates UI spec section 4.4). `back_to`:
    /// the version Go back installs, when it can.
    Unhealthy {
        version: String,
        back_to: Option<String>,
    },
    /// The developers withdrew the version running (updates UI spec section 4.5).
    Withdrawn {
        version: String,
        back_to: Option<String>,
    },
    /// Dots installed `version` by itself while idle, at `at` (RFC 3339 UTC); said once.
    UpdatedItself { version: String, at: String },
    /// The last install did not happen and no update is ready (yet) to try again: why, until
    /// the person dismisses it ("Later") or the update is downloaded again (spec §7).
    Failed { error: String },
}

/// What an install is for (updates UI spec section 4): a new version, the same version again,
/// or the version before the last update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    #[default]
    Update,
    Repair,
    GoBack,
}

/// How long the window has to go without input before an idle install (spec section 4.2).
pub const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);

/// What decides an idle install. `busy`: an install waits or has begun.
#[derive(Debug, Clone, Copy)]
pub struct IdleInput {
    pub enabled: bool,
    pub ready: bool,
    pub later: bool,
    pub busy: bool,
    pub unfinished_runs: usize,
    pub composer_has_text: bool,
    pub input_idle: Duration,
    pub window_hidden: bool,
}

/// Whether Dots may install the ready update by itself now: the person turned it on, did not
/// press "Later", no run is unfinished, nothing is being written, and the window is hidden or
/// untouched for `IDLE_AFTER`.
pub fn idle_install_due(i: &IdleInput) -> bool {
    i.enabled
        && i.ready
        && !i.later
        && !i.busy
        && i.unfinished_runs == 0
        && !i.composer_has_text
        && (i.window_hidden || i.input_idle >= IDLE_AFTER)
}

/// What the app does after a change of `UpdateState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Nothing,
    Install,
}

/// The state behind the strip. `later` lasts until the process ends, so the note is back at
/// the next start.
#[derive(Debug, Default)]
pub struct UpdateState {
    ready: Option<String>,
    later: bool,
    waiting: bool,
    installing: bool,
    error: Option<String>,
    /// Versions this machine never takes again (`update_record`).
    bad: Vec<String>,
    /// What the ready install is for.
    purpose: Purpose,
    /// Unhealthy, Withdrawn or UpdatedItself, set by the service.
    notice: Option<UpdateNote>,
}

impl UpdateState {
    /// `error`: what the last start left (`take_error`).
    /// `bad`: the versions this machine marked bad.
    pub fn new(error: Option<String>, bad: Vec<String>) -> Self {
        Self {
            error,
            bad,
            ..Self::default()
        }
    }

    /// Whether a check that found `version` should download it: not while an install waits or
    /// has begun (the person agreed to the version they saw), and not a second time.
    pub fn wants(&self, version: &str) -> bool {
        !self.waiting
            && !self.installing
            && self.ready.as_deref() != Some(version)
            && !self.bad.iter().any(|b| b == version)
    }

    /// `version` failed on this machine: never wanted again.
    pub fn mark_bad(&mut self, version: &str) {
        if !self.bad.iter().any(|b| b == version) {
            self.bad.push(version.to_string());
        }
    }

    /// Whether "Later" was pressed this session: no idle install until the next start.
    pub fn later_pressed(&self) -> bool {
        self.later
    }

    /// Unhealthy, Withdrawn or UpdatedItself, or none.
    pub fn set_notice(&mut self, notice: Option<UpdateNote>) {
        self.notice = notice;
    }

    /// Repair or Go back, whose download is ready under `label` (the version it installs):
    /// installs at once when no dot has a run working, else waits for them, like "Restart to
    /// update". Nothing when an install has begun.
    pub fn begin_other(&mut self, purpose: Purpose, label: &str, working: &[String]) -> Next {
        if self.installing {
            return Next::Nothing;
        }
        self.ready = Some(label.to_string());
        self.purpose = purpose;
        self.later = false;
        self.error = None;
        if working.is_empty() {
            return self.begin();
        }
        self.waiting = true;
        Next::Nothing
    }

    /// A downloaded update, checked against Dots' key. True when the note changed.
    pub fn offer(&mut self, version: &str) -> bool {
        if !self.wants(version) {
            return false;
        }
        self.ready = Some(version.to_string());
        self.purpose = Purpose::Update;
        !self.later
    }

    /// The note, first that applies: Installing, Waiting, Unhealthy, Withdrawn, Ready,
    /// UpdatedItself, Failed, None.
    pub fn note(&self, working: &[String]) -> UpdateNote {
        if let Some(version) = self.ready.clone() {
            let purpose = self.purpose;
            if self.installing {
                return UpdateNote::Installing { version, purpose };
            }
            if self.waiting {
                return UpdateNote::Waiting {
                    version,
                    waiting_for: working.to_vec(),
                    purpose,
                };
            }
        }
        if let Some(notice @ (UpdateNote::Unhealthy { .. } | UpdateNote::Withdrawn { .. })) =
            &self.notice
        {
            return notice.clone();
        }
        if let (Some(version), false) = (self.ready.clone(), self.later) {
            return UpdateNote::Ready {
                version,
                working: working.to_vec(),
                error: self.error.clone(),
            };
        }
        if let Some(notice @ UpdateNote::UpdatedItself { .. }) = &self.notice {
            return notice.clone();
        }
        match &self.error {
            Some(error) if self.ready.is_none() => UpdateNote::Failed {
                error: error.clone(),
            },
            _ => UpdateNote::None,
        }
    }

    /// The version that is ready, whether or not the note shows it.
    pub fn ready_version(&self) -> Option<&str> {
        self.ready.as_deref()
    }

    /// Whether the run queue is held: while an install waits (spec §6), and once it has begun,
    /// so a stale button cannot let a run start under the install.
    pub fn holds_runs(&self) -> bool {
        self.waiting || self.installing
    }

    /// "Later" hides a ready update until the next start. It also dismisses the last error;
    /// with no update ready that is all it does, so the next update still shows.
    pub fn later(&mut self) {
        self.error = None;
        if matches!(
            self.notice,
            Some(UpdateNote::Unhealthy { .. } | UpdateNote::Withdrawn { .. })
        ) {
            self.notice = None;
        }
        if self.ready.is_some() {
            self.later = true;
        }
    }

    /// An install that could not begin (its download is gone): the queue goes, the update is
    /// downloaded again at the next check, and the note says why.
    pub fn abandon(&mut self, error: &str) {
        self.ready = None;
        self.waiting = false;
        self.installing = false;
        self.purpose = Purpose::Update;
        self.error = Some(error.to_string());
    }

    /// "Restart to update" and "Restart when it's done": installs at once when no dot has a
    /// run working, else waits for those runs.
    pub fn restart(&mut self, working: &[String]) -> dots_core::Result<Next> {
        self.check_ready()?;
        self.error = None;
        if working.is_empty() {
            return Ok(self.begin());
        }
        self.waiting = true;
        Ok(Next::Nothing)
    }

    /// "Stop it and restart": installs, stopping the runs that are working. Those runs end as
    /// failed ("interrupted") at the next start, as after "Restart Dots". `seen`: the dots the
    /// confirm named. When a dot it did not name is working now, nothing is installed, and the
    /// note that follows names the dots working now.
    pub fn stop_and_restart(
        &mut self,
        seen: &[String],
        working: &[String],
    ) -> dots_core::Result<Next> {
        self.check_ready()?;
        self.error = None;
        if working.iter().any(|dot| !seen.contains(dot)) {
            return Ok(Next::Nothing);
        }
        Ok(self.begin())
    }

    /// The working runs changed: while waiting, the install begins once none is left.
    pub fn runs_changed(&mut self, working: &[String]) -> Next {
        if self.waiting && !self.installing && working.is_empty() {
            self.begin()
        } else {
            Next::Nothing
        }
    }

    /// "Cancel": stop waiting, and let the queue start again.
    pub fn cancel_wait(&mut self) {
        self.waiting = false;
    }

    fn check_ready(&self) -> dots_core::Result<()> {
        match (&self.ready, self.installing) {
            (None, _) => Err(dots_core::Error::Conflict("no update is ready".into())),
            (Some(_), true) => Err(dots_core::Error::Conflict(
                "the update is already being installed".into(),
            )),
            (Some(_), false) => Ok(()),
        }
    }

    fn begin(&mut self) -> Next {
        self.installing = true;
        Next::Install
    }
}

/// Whether this Dots checks for updates: never in a debug build, nor when `DOTS_DATA_DIR`
/// is set (a development or smoke instance; spec §5). An empty value counts as not set.
pub fn checks_enabled(debug: bool, data_dir_override: Option<OsString>) -> bool {
    !debug && data_dir_override.is_none_or(|v| v.is_empty())
}

/// The manifest to check: `DOTS_UPDATE_ENDPOINT` when set and not empty, else the GitHub
/// "latest" one. The error names the variable and the value.
pub fn endpoint(env_override: Option<OsString>) -> Result<Url, String> {
    match env_override.filter(|v| !v.is_empty()) {
        Some(raw) => {
            let text = raw.to_string_lossy();
            Url::parse(&text).map_err(|e| format!("{ENDPOINT_ENV} is not a URL ({text}): {e}"))
        }
        None => Ok(Url::parse(DEFAULT_ENDPOINT).expect("the default endpoint is a URL")),
    }
}

/// A version's release page, which the strip's "What's new" opens.
pub fn release_page(version: &str) -> String {
    format!("https://github.com/Aakshar-D/dots/releases/tag/v{version}")
}

/// `dots-<version>-YYYY-MM-DD-HHMMSS.db` (spec §7).
pub fn backup_file_name(version: &str, at: DateTime<Local>) -> String {
    format!("dots-{version}-{}.db", at.format("%Y-%m-%d-%H%M%S"))
}

/// The database files `set_aside_database` moves: the database and its WAL files.
const DB_FILES: [&str; 3] = ["dots.db", "dots.db-wal", "dots.db-shm"];

/// Moves the database and its WAL files into the backups folder as
/// `failed-dots-<version>-<stamp>.db` (and `-wal`, `-shm`), before going back puts the
/// update's backup in its place. Named so the backup pruning never counts or deletes them.
/// The path of the moved database.
pub fn set_aside_database(
    data_dir: &Path,
    version: &str,
    at: DateTime<Local>,
) -> Result<PathBuf, String> {
    let dir = data_dir.join(BACKUPS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("making {}: {e}", dir.display()))?;
    let name = format!("failed-{}", backup_file_name(version, at));
    let moved = dir.join(&name);
    for (i, file) in DB_FILES.iter().enumerate() {
        let from = data_dir.join(file);
        if !from.exists() {
            continue;
        }
        let to = dir.join(format!("{name}{}", &file["dots.db".len()..]));
        if let Err(e) = std::fs::rename(&from, &to) {
            // Put back what already moved, so the database stays whole.
            for earlier in &DB_FILES[..i] {
                let _ = std::fs::rename(
                    dir.join(format!("{name}{}", &earlier["dots.db".len()..])),
                    data_dir.join(earlier),
                );
            }
            return Err(format!("moving {}: {e}", from.display()));
        }
    }
    Ok(moved)
}

/// Puts back what `set_aside_database` moved to `moved`, replacing what is in place now.
pub fn undo_set_aside(data_dir: &Path, moved: &Path) -> Result<(), String> {
    let name = moved
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{} is not a set-aside database", moved.display()))?;
    let dir = moved.parent().unwrap_or(data_dir);
    for file in DB_FILES {
        let from = dir.join(format!("{name}{}", &file["dots.db".len()..]));
        let to = data_dir.join(file);
        if from.exists() {
            let _ = std::fs::remove_file(&to);
            std::fs::rename(&from, &to).map_err(|e| format!("moving {}: {e}", from.display()))?;
        } else {
            let _ = std::fs::remove_file(&to);
        }
    }
    Ok(())
}

/// The time in a backup's name, which orders backups whatever their versions.
fn stamp(name: &str) -> Option<&str> {
    let stem = name.strip_prefix("dots-")?.strip_suffix(".db")?;
    let at = stem.len().checked_sub(17)?;
    let (version, stamp) = stem.split_at(at);
    (version.ends_with('-') && stamp.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .then_some(stamp)
}

/// The backups in `dir`, newest first. Other files are left out.
fn backups(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(stamp) = stamp(name) {
            found.push((stamp.to_string(), path.clone()));
        }
    }
    found.sort_by(|a, b| b.cmp(a));
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

/// The newest backup in the data folder, for the fatal-error page (spec §8).
pub fn newest_backup(data_dir: &Path) -> Option<PathBuf> {
    backups(&data_dir.join(BACKUPS_DIR))
        .ok()?
        .into_iter()
        .next()
}

/// Backs up `dots.db` of `data_dir` into its backups folder under `backup_file_name`, then
/// deletes the older backups, keeping the new backup always and the `KEEP_BACKUPS - 1` newest
/// others. Returns the new backup's path. A backup that could not be pruned after is logged,
/// not an error. A backup that fails leaves no file behind it, so a part copy is never taken
/// for a backup.
pub async fn back_up(
    data_dir: &Path,
    version: &str,
    at: DateTime<Local>,
) -> Result<PathBuf, String> {
    let dir = data_dir.join(BACKUPS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(backup_file_name(version, at));
    let existed = dest.exists();
    let made =
        dots_core::store::backup_database(&data_dir.join(dots_core::store::DB_FILE), &dest).await;
    if let Err(e) = made {
        if !existed && dest.exists() {
            if let Err(gone) = std::fs::remove_file(&dest) {
                tracing::warn!(
                    "could not delete the part backup {}: {gone}",
                    dest.display()
                );
            }
        }
        return Err(e.to_string());
    }
    match backups(&dir) {
        Ok(all) => {
            for old in all.iter().filter(|p| **p != dest).skip(KEEP_BACKUPS - 1) {
                if let Err(e) = std::fs::remove_file(old) {
                    tracing::warn!("could not delete the old backup {}: {e}", old.display());
                }
            }
        }
        Err(e) => tracing::warn!("could not list the backups in {}: {e}", dir.display()),
    }
    Ok(dest)
}

/// Leaves `error` for the next start. A failure is logged: the restart goes on.
pub fn leave_error(data_dir: &Path, error: &str) {
    if let Err(e) = std::fs::write(data_dir.join(ERROR_MARK), error) {
        tracing::warn!(
            "could not leave the update error in {}: {e}",
            data_dir.display()
        );
    }
}

/// What the last start left with `leave_error`, removed so it counts for one start.
pub fn take_error(data_dir: &Path) -> Option<String> {
    let mark = data_dir.join(ERROR_MARK);
    let text = std::fs::read_to_string(&mark).ok()?;
    if let Err(e) = std::fs::remove_file(&mark) {
        tracing::warn!("could not remove {}: {e}", mark.display());
    }
    Some(text)
}

/// The steps an install is made of, so the order is tested without installing anything.
/// Only this crate implements it, so the futures need no `Send` bound in the signature.
#[allow(async_fn_in_trait)]
pub trait InstallSteps {
    /// Stops the runtime and waits, at most `CLOSE_LIMIT`, for the database to close.
    async fn close_runtime(&self) -> bool;
    async fn back_up(&self) -> Result<PathBuf, String>;
    /// Restarts into this version with `error` left for the next start. Does not return in
    /// the app.
    fn restart_current(&self, error: &str);
    /// The restart mark, so the new Dots shows its window.
    fn mark_restart(&self);
    /// Writes the update record for `purpose` (`update_record`): what is left and installed
    /// and `backup` for an update or a repair, the current version as bad for going back.
    fn record(&self, purpose: Purpose, backup: Option<&Path>) -> Result<(), String>;
    /// Moves `dots.db` and its WAL files aside into the backups folder; the path of the moved
    /// database.
    async fn set_aside_database(&self) -> Result<PathBuf, String>;
    /// Copies `backup` in as `dots.db`.
    async fn restore_backup(&self, backup: &Path) -> Result<(), String>;
    /// Puts the database `set_aside_database` moved back in place. A failure is logged.
    fn undo_restore(&self, set_aside: &Path);
    /// Starts the installer and ends the process. Returns only when it could not start.
    fn install(&self) -> Result<(), String>;
    /// Deletes a backup that no install followed. A failure is logged.
    fn discard_backup(&self, backup: &Path);
}

/// An install, in the order of spec §7. Every failure restarts into this version with the
/// reason, which the strip shows after that start. A backup no install followed is deleted:
/// the database is the same, and each retry would otherwise prune an older backup, until the
/// one from before the last real update is gone.
///
/// Going back (`purpose` GoBack, `backup` the update's backup) sets the database aside and
/// restores the backup instead of making one; any failure after that puts the database back.
pub async fn install(steps: &impl InstallSteps, purpose: Purpose, backup: Option<&Path>) {
    if !steps.close_runtime().await {
        steps.restart_current("Dots could not close its database in time, so no backup was made.");
        return;
    }
    if purpose == Purpose::GoBack {
        go_back(steps, backup).await;
        return;
    }
    let backup = match steps.back_up().await {
        Ok(backup) => backup,
        Err(e) => {
            steps.restart_current(&format!("The backup failed: {e}"));
            return;
        }
    };
    if let Err(e) = steps.record(purpose, Some(&backup)) {
        steps.discard_backup(&backup);
        steps.restart_current(&format!("The update could not be recorded: {e}"));
        return;
    }
    steps.mark_restart();
    if let Err(e) = steps.install() {
        steps.discard_backup(&backup);
        steps.restart_current(&format!("The update could not start: {e}"));
    }
}

async fn go_back(steps: &impl InstallSteps, backup: Option<&Path>) {
    let Some(backup) = backup else {
        steps.restart_current("There is nothing to go back to.");
        return;
    };
    let set_aside = match steps.set_aside_database().await {
        Ok(path) => path,
        Err(e) => {
            steps.restart_current(&format!("The database could not be set aside: {e}"));
            return;
        }
    };
    let failure = match steps.restore_backup(backup).await {
        Err(e) => Some(format!("The backup could not be restored: {e}")),
        Ok(()) => match steps.record(Purpose::GoBack, Some(backup)) {
            Err(e) => Some(format!("Going back could not be recorded: {e}")),
            Ok(()) => {
                steps.mark_restart();
                steps
                    .install()
                    .err()
                    .map(|e| format!("The update could not start: {e}"))
            }
        },
    };
    if let Some(reason) = failure {
        steps.undo_restore(&set_aside);
        steps.restart_current(&reason);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use chrono::TimeZone;

    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn ready(working: &[&str], error: Option<&str>) -> UpdateNote {
        UpdateNote::Ready {
            version: "0.2.0".into(),
            working: names(working),
            error: error.map(String::from),
        }
    }

    #[test]
    fn there_is_no_note_until_an_update_is_ready_and_later_hides_it() {
        let mut s = UpdateState::new(None, vec![]);
        assert_eq!(s.note(&[]), UpdateNote::None);
        assert_eq!(s.ready_version(), None);
        assert!(s.wants("0.2.0"));
        assert!(s.offer("0.2.0"));
        assert_eq!(s.note(&[]), ready(&[], None));
        assert!(!s.wants("0.2.0"), "the same version is downloaded once");
        s.later();
        assert_eq!(s.note(&[]), UpdateNote::None);
        assert_eq!(s.ready_version(), Some("0.2.0"), "hidden, still ready");
        // A newer version after "Later" stays hidden too, until the next start.
        assert!(!s.offer("0.3.0"));
        assert_eq!(s.note(&[]), UpdateNote::None);
    }

    #[test]
    fn the_note_names_the_working_dots_and_the_last_error() {
        let mut s = UpdateState::new(Some("The backup failed: disk full".into()), vec![]);
        s.offer("0.2.0");
        assert_eq!(
            s.note(&names(&["Daily digest"])),
            ready(&["Daily digest"], Some("The backup failed: disk full"))
        );
    }

    #[test]
    fn the_last_error_shows_with_no_update_ready_until_it_is_dismissed() {
        let mut s = UpdateState::new(Some("The backup failed: disk full".into()), vec![]);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Failed {
                error: "The backup failed: disk full".into()
            }
        );
        assert!(s.restart(&[]).is_err(), "nothing is ready to install");
        // "Dismiss" is Later: with nothing ready it hides only the error, not the next update.
        s.later();
        assert_eq!(s.note(&[]), UpdateNote::None);
        assert!(s.offer("0.2.0"));
        assert_eq!(s.note(&[]), ready(&[], None));
    }

    #[test]
    fn the_last_error_goes_once_the_person_acts_on_the_update() {
        let mut s = UpdateState::new(Some("The backup failed: disk full".into()), vec![]);
        s.offer("0.2.0");
        s.restart(&names(&["a"])).unwrap();
        s.cancel_wait();
        assert_eq!(
            s.note(&[]),
            ready(&[], None),
            "the reason is not shown again"
        );

        let mut s = UpdateState::new(Some("The backup failed: disk full".into()), vec![]);
        s.offer("0.2.0");
        s.later();
        assert_eq!(s.note(&[]), UpdateNote::None);
    }

    #[test]
    fn an_install_that_cannot_begin_lets_the_queue_go_and_says_why() {
        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.2.0");
        assert_eq!(s.restart(&[]).unwrap(), Next::Install);
        s.abandon("The downloaded update was lost; Dots downloads it again.");
        assert!(!s.holds_runs());
        assert_eq!(s.ready_version(), None);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Failed {
                error: "The downloaded update was lost; Dots downloads it again.".into()
            }
        );
        assert!(s.wants("0.2.0"), "the next check downloads it again");
        assert!(s.offer("0.2.0"));
        assert_eq!(
            s.note(&[]),
            ready(
                &[],
                Some("The downloaded update was lost; Dots downloads it again.")
            )
        );
    }

    #[test]
    fn restart_installs_at_once_when_nothing_works_and_waits_otherwise() {
        let mut s = UpdateState::new(None, vec![]);
        assert!(s.restart(&[]).is_err(), "nothing is ready");
        s.offer("0.2.0");
        assert_eq!(s.restart(&[]).unwrap(), Next::Install);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Installing {
                version: "0.2.0".into(),
                purpose: Purpose::Update,
            }
        );
        assert!(
            s.restart(&[]).is_err(),
            "an install that has begun is not begun twice"
        );

        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.2.0");
        let busy = names(&["Daily digest"]);
        assert_eq!(s.restart(&busy).unwrap(), Next::Nothing);
        assert!(s.holds_runs());
        assert_eq!(
            s.note(&busy),
            UpdateNote::Waiting {
                version: "0.2.0".into(),
                waiting_for: busy.clone(),
                purpose: Purpose::Update,
            }
        );
        assert!(
            !s.wants("0.3.0"),
            "a waiting install keeps the version the person saw"
        );
    }

    #[test]
    fn a_wait_ends_with_the_last_run_or_with_cancel() {
        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.2.0");
        s.restart(&names(&["a", "b"])).unwrap();
        assert_eq!(s.runs_changed(&names(&["b"])), Next::Nothing);
        assert_eq!(s.runs_changed(&[]), Next::Install);
        assert_eq!(s.runs_changed(&[]), Next::Nothing, "one install");

        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.2.0");
        s.restart(&names(&["a"])).unwrap();
        s.cancel_wait();
        assert!(!s.holds_runs());
        assert_eq!(s.runs_changed(&[]), Next::Nothing);
        assert_eq!(s.note(&[]), ready(&[], None));
    }

    #[test]
    fn stop_and_restart_installs_when_only_the_dots_it_named_work() {
        let mut s = UpdateState::new(None, vec![]);
        assert!(s.stop_and_restart(&[], &[]).is_err());
        s.offer("0.2.0");
        let seen = names(&["a", "b"]);
        // A dot the confirm did not name started meanwhile: nothing is stopped.
        let now = names(&["a", "c"]);
        assert_eq!(s.stop_and_restart(&seen, &now).unwrap(), Next::Nothing);
        assert!(!s.holds_runs());
        assert_eq!(s.note(&now), ready(&["a", "c"], None));
        // Fewer dots than it named, or none, is fine.
        assert_eq!(
            s.stop_and_restart(&seen, &names(&["b"])).unwrap(),
            Next::Install
        );

        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.2.0");
        s.restart(&names(&["a"])).unwrap();
        assert_eq!(
            s.stop_and_restart(&names(&["a"]), &[]).unwrap(),
            Next::Install
        );
        assert!(
            s.stop_and_restart(&[], &[]).is_err(),
            "an install begins once"
        );
    }

    #[test]
    fn the_queue_is_held_while_an_install_waits_and_once_it_has_begun() {
        let mut s = UpdateState::new(None, vec![]);
        assert!(!s.holds_runs());
        s.offer("0.2.0");
        assert!(!s.holds_runs(), "a ready update holds nothing");
        s.restart(&names(&["a"])).unwrap();
        assert!(s.holds_runs(), "waiting");
        s.cancel_wait();
        assert!(!s.holds_runs(), "cancelled");
        s.restart(&[]).unwrap();
        assert!(s.holds_runs(), "installing");
        // A stale Cancel or Later, sent before the strip showed the install, lifts nothing.
        s.cancel_wait();
        s.later();
        assert!(s.holds_runs(), "still installing");
    }

    #[test]
    fn a_check_downloads_a_version_only_when_none_waits_or_installs() {
        let mut s = UpdateState::new(None, vec![]);
        assert!(s.wants("0.2.0"), "nothing is ready");
        s.offer("0.2.0");
        assert!(!s.wants("0.2.0"), "already downloaded");
        assert!(s.wants("0.3.0"), "a newer one replaces it");
        s.restart(&names(&["a"])).unwrap();
        assert!(!s.wants("0.3.0"), "waiting");
        s.cancel_wait();
        s.restart(&[]).unwrap();
        assert!(!s.wants("0.3.0"), "installing");
        assert!(!s.offer("0.3.0"));
        assert_eq!(s.ready_version(), Some("0.2.0"));
    }

    #[test]
    fn checks_are_off_in_debug_builds_and_under_dots_data_dir() {
        assert!(checks_enabled(false, None));
        assert!(checks_enabled(false, Some("".into())));
        assert!(!checks_enabled(true, None));
        assert!(!checks_enabled(false, Some("C:/smoke".into())));
    }

    #[test]
    fn the_endpoint_is_github_latest_unless_the_variable_names_another() {
        assert_eq!(endpoint(None).unwrap().as_str(), DEFAULT_ENDPOINT);
        assert_eq!(
            endpoint(Some("".into())).unwrap().as_str(),
            DEFAULT_ENDPOINT
        );
        let rc = "https://github.com/Aakshar-D/dots/releases/download/v0.2.0-rc1/latest.json";
        assert_eq!(endpoint(Some(rc.into())).unwrap().as_str(), rc);
        let err = endpoint(Some("not a url".into())).unwrap_err();
        assert!(
            err.starts_with("DOTS_UPDATE_ENDPOINT is not a URL (not a url)"),
            "{err}"
        );
    }

    fn at(h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 20, h, 5, 9).unwrap()
    }

    #[test]
    fn whats_new_is_the_versions_release_page() {
        assert_eq!(
            release_page("0.2.0"),
            "https://github.com/Aakshar-D/dots/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn a_backup_is_named_after_the_old_version_and_the_time() {
        assert_eq!(
            backup_file_name("0.1.0", at(9)),
            "dots-0.1.0-2026-10-20-090509.db"
        );
        assert_eq!(
            stamp("dots-0.1.0-2026-10-20-090509.db"),
            Some("2026-10-20-090509")
        );
        assert_eq!(stamp("dots-0.1.0.db"), None);
        assert_eq!(stamp("notes.txt"), None);
    }

    #[tokio::test]
    async fn a_backup_keeps_the_three_newest_and_nothing_else_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let store = dots_core::store::Store::open(&dir.path().join("dots.db"))
            .await
            .unwrap();
        store.close().await;
        let backups_dir = dir.path().join(BACKUPS_DIR);
        std::fs::create_dir_all(&backups_dir).unwrap();
        // Older backups of two versions; the version does not decide the order, the time does.
        for (version, hour) in [("0.10.0", 1), ("0.9.0", 2), ("0.10.0", 3)] {
            std::fs::write(backups_dir.join(backup_file_name(version, at(hour))), "").unwrap();
        }
        std::fs::write(backups_dir.join("notes.txt"), "mine").unwrap();
        let made = back_up(dir.path(), "0.1.0", at(9)).await.unwrap();
        assert_eq!(made, backups_dir.join("dots-0.1.0-2026-10-20-090509.db"));
        let mut left: Vec<String> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "dots-0.1.0-2026-10-20-090509.db",
                "dots-0.10.0-2026-10-20-030509.db",
                "dots-0.9.0-2026-10-20-020509.db",
                "notes.txt",
            ]
        );
        assert_eq!(newest_backup(dir.path()), Some(made));
    }

    #[tokio::test]
    async fn a_backup_is_kept_even_when_others_have_later_stamps() {
        let dir = tempfile::tempdir().unwrap();
        let store = dots_core::store::Store::open(&dir.path().join("dots.db"))
            .await
            .unwrap();
        store.close().await;
        let backups_dir = dir.path().join(BACKUPS_DIR);
        std::fs::create_dir_all(&backups_dir).unwrap();
        // Three backups stamped after the new one, as after a clock that ran ahead.
        for hour in [20, 21, 22] {
            std::fs::write(backups_dir.join(backup_file_name("0.1.0", at(hour))), "").unwrap();
        }
        let made = back_up(dir.path(), "0.1.0", at(9)).await.unwrap();
        assert!(made.exists(), "the backup just made is never pruned");
        let mut left: Vec<String> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "dots-0.1.0-2026-10-20-090509.db",
                "dots-0.1.0-2026-10-20-210509.db",
                "dots-0.1.0-2026-10-20-220509.db",
            ]
        );
    }

    #[tokio::test]
    async fn a_backup_without_a_database_fails_and_there_is_no_newest() {
        let dir = tempfile::tempdir().unwrap();
        assert!(back_up(dir.path(), "0.1.0", at(9)).await.is_err());
        assert_eq!(newest_backup(dir.path()), None);
    }

    fn backups_left(data_dir: &Path) -> Vec<String> {
        std::fs::read_dir(data_dir.join(BACKUPS_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[tokio::test]
    async fn a_backup_of_a_file_that_is_not_a_database_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dots.db"), "not a database ".repeat(1000)).unwrap();
        assert!(back_up(dir.path(), "0.1.0", at(9)).await.is_err());
        assert_eq!(backups_left(dir.path()), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_backup_that_fails_part_way_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("dots.db");
        let made = dir.path().join("made.db");
        let store = dots_core::store::Store::open(&made).await.unwrap();
        store.close().await;
        // One file with no WAL beside it.
        dots_core::store::backup_database(&made, &db).await.unwrap();
        // The schema on the first page reads, the second page does not: SQLite has begun the
        // copy when it finds that out (without the clean-up, the part copy was left).
        let mut bytes = std::fs::read(&db).unwrap();
        bytes[4096..8192].fill(0xA5);
        std::fs::write(&db, bytes).unwrap();
        let err = back_up(dir.path(), "0.1.0", at(9)).await.unwrap_err();
        assert!(err.contains("malformed"), "{err}");
        assert_eq!(backups_left(dir.path()), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_failed_backup_keeps_a_file_that_was_there_before() {
        let dir = tempfile::tempdir().unwrap();
        let store = dots_core::store::Store::open(&dir.path().join("dots.db"))
            .await
            .unwrap();
        store.close().await;
        let backups_dir = dir.path().join(BACKUPS_DIR);
        std::fs::create_dir_all(&backups_dir).unwrap();
        let there = backups_dir.join(backup_file_name("0.1.0", at(9)));
        std::fs::write(&there, "mine").unwrap();
        assert!(back_up(dir.path(), "0.1.0", at(9)).await.is_err());
        assert_eq!(std::fs::read_to_string(&there).unwrap(), "mine");
    }

    #[test]
    fn an_error_left_for_the_next_start_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(take_error(dir.path()), None);
        leave_error(dir.path(), "The backup failed: disk full");
        assert_eq!(
            take_error(dir.path()).as_deref(),
            Some("The backup failed: disk full")
        );
        assert_eq!(take_error(dir.path()), None);
    }

    /// Records the steps; `fail` names the one that fails.
    struct Steps {
        fail: &'static str,
        done: RefCell<Vec<String>>,
    }

    impl Steps {
        fn new(fail: &'static str) -> Self {
            Self {
                fail,
                done: RefCell::new(Vec::new()),
            }
        }
        fn log(&self, step: &str) -> bool {
            self.done.borrow_mut().push(step.to_string());
            self.fail != step
        }
        fn done(&self) -> Vec<String> {
            self.done.borrow().clone()
        }
    }

    impl InstallSteps for Steps {
        async fn close_runtime(&self) -> bool {
            self.log("close")
        }
        async fn back_up(&self) -> Result<PathBuf, String> {
            self.log("back up")
                .then(|| PathBuf::from("b.db"))
                .ok_or_else(|| "disk full".to_string())
        }
        fn restart_current(&self, error: &str) {
            self.log(&format!("restart: {error}"));
        }
        fn mark_restart(&self) {
            self.log("mark");
        }
        fn discard_backup(&self, backup: &Path) {
            self.log(&format!("discard {}", backup.display()));
        }
        fn record(&self, purpose: Purpose, backup: Option<&Path>) -> Result<(), String> {
            let purpose = serde_json::to_value(purpose).unwrap();
            let backup = backup.map(|b| b.display().to_string()).unwrap_or_default();
            self.log(&format!("record {} {backup}", purpose.as_str().unwrap()))
                .then_some(())
                .ok_or_else(|| "read-only".to_string())
        }
        async fn set_aside_database(&self) -> Result<PathBuf, String> {
            self.log("set aside")
                .then(|| PathBuf::from("failed.db"))
                .ok_or_else(|| "locked".to_string())
        }
        async fn restore_backup(&self, backup: &Path) -> Result<(), String> {
            self.log(&format!("restore {}", backup.display()))
                .then_some(())
                .ok_or_else(|| "disk full".to_string())
        }
        fn undo_restore(&self, set_aside: &Path) {
            self.log(&format!("undo restore {}", set_aside.display()));
        }
        fn install(&self) -> Result<(), String> {
            self.log("install")
                .then_some(())
                .ok_or_else(|| "access denied".to_string())
        }
    }

    #[tokio::test]
    async fn an_install_closes_backs_up_marks_then_installs() {
        let steps = Steps::new("");
        install(&steps, Purpose::Update, None).await;
        assert_eq!(
            steps.done(),
            ["close", "back up", "record update b.db", "mark", "install"]
        );
    }

    #[tokio::test]
    async fn a_failed_step_restarts_on_this_version_with_the_reason() {
        let steps = Steps::new("close");
        install(&steps, Purpose::Update, None).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "restart: Dots could not close its database in time, so no backup was made."
            ]
        );
        let steps = Steps::new("back up");
        install(&steps, Purpose::Update, None).await;
        assert_eq!(
            steps.done(),
            ["close", "back up", "restart: The backup failed: disk full"]
        );
        let steps = Steps::new("install");
        install(&steps, Purpose::Update, None).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "back up",
                "record update b.db",
                "mark",
                "install",
                // Nothing changed since the backup: kept, a retry could prune an older one.
                "discard b.db",
                "restart: The update could not start: access denied"
            ]
        );
    }

    #[test]
    fn a_bad_version_is_never_wanted_but_a_newer_one_is() {
        let s = UpdateState::new(None, vec!["0.3.1".into()]);
        assert!(!s.wants("0.3.1"));
        assert!(s.wants("0.3.2"));
    }

    #[test]
    fn the_idle_install_is_due_only_when_everything_is_quiet() {
        let quiet = IdleInput {
            enabled: true,
            ready: true,
            later: false,
            busy: false,
            unfinished_runs: 0,
            composer_has_text: false,
            input_idle: IDLE_AFTER,
            window_hidden: false,
        };
        assert!(idle_install_due(&quiet));
        let not = |i: IdleInput| assert!(!idle_install_due(&i));
        not(IdleInput {
            input_idle: IDLE_AFTER - Duration::from_secs(1),
            ..quiet
        });
        assert!(idle_install_due(&IdleInput {
            input_idle: Duration::ZERO,
            window_hidden: true,
            ..quiet
        }));
        not(IdleInput {
            later: true,
            ..quiet
        });
        not(IdleInput {
            unfinished_runs: 1,
            ..quiet
        });
        not(IdleInput {
            composer_has_text: true,
            ..quiet
        });
        not(IdleInput {
            busy: true,
            ..quiet
        });
        not(IdleInput {
            enabled: false,
            ..quiet
        });
        not(IdleInput {
            ready: false,
            ..quiet
        });
    }

    #[test]
    fn later_stops_the_idle_install_until_the_next_start() {
        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.3.1");
        assert!(!s.later_pressed());
        s.later();
        assert!(s.later_pressed());
        assert!(!UpdateState::new(None, vec![]).later_pressed());
    }

    #[test]
    fn repair_and_go_back_wait_for_working_runs_like_restart() {
        let mut s = UpdateState::new(None, vec![]);
        assert_eq!(
            s.begin_other(Purpose::Repair, "0.3.0", &names(&["digest"])),
            Next::Nothing
        );
        assert_eq!(
            s.note(&names(&["digest"])),
            UpdateNote::Waiting {
                version: "0.3.0".into(),
                waiting_for: names(&["digest"]),
                purpose: Purpose::Repair
            }
        );
        assert_eq!(s.runs_changed(&[]), Next::Install);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Installing {
                version: "0.3.0".into(),
                purpose: Purpose::Repair
            }
        );
        let mut s = UpdateState::new(None, vec![]);
        assert_eq!(s.begin_other(Purpose::GoBack, "0.2.0", &[]), Next::Install);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Installing {
                version: "0.2.0".into(),
                purpose: Purpose::GoBack
            }
        );
    }

    #[test]
    fn the_notes_come_in_order() {
        let unhealthy = UpdateNote::Unhealthy {
            version: "0.3.1".into(),
            back_to: Some("0.3.0".into()),
        };
        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.3.2");
        s.set_notice(Some(unhealthy.clone()));
        assert_eq!(s.note(&[]), unhealthy);
        s.later();
        assert_eq!(s.note(&[]), UpdateNote::None);
        let mut s = UpdateState::new(None, vec![]);
        s.offer("0.3.2");
        s.set_notice(Some(UpdateNote::UpdatedItself {
            version: "0.3.1".into(),
            at: "2026-10-27T03:12:00.000Z".into(),
        }));
        assert!(matches!(s.note(&[]), UpdateNote::Ready { .. }));
        s.set_notice(Some(unhealthy));
        assert_eq!(s.restart(&[]).unwrap(), Next::Install);
        assert!(matches!(s.note(&[]), UpdateNote::Installing { .. }));
    }

    #[tokio::test]
    async fn an_update_records_then_marks_then_installs() {
        let steps = Steps::new("");
        install(&steps, Purpose::Update, None).await;
        assert_eq!(
            steps.done(),
            ["close", "back up", "record update b.db", "mark", "install"]
        );
    }

    #[tokio::test]
    async fn go_back_sets_the_database_aside_restores_the_backup_and_installs() {
        let steps = Steps::new("");
        install(&steps, Purpose::GoBack, Some(Path::new("old.db"))).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "set aside",
                "restore old.db",
                "record go_back old.db",
                "mark",
                "install"
            ]
        );
    }

    #[tokio::test]
    async fn a_go_back_whose_installer_cannot_start_puts_the_database_back() {
        let steps = Steps::new("install");
        install(&steps, Purpose::GoBack, Some(Path::new("old.db"))).await;
        let done = steps.done();
        assert_eq!(
            &done[done.len() - 2..],
            [
                "undo restore failed.db",
                "restart: The update could not start: access denied"
            ]
        );
    }

    #[test]
    fn setting_the_database_aside_moves_it_and_its_wal_and_undo_puts_them_back() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["dots.db", "dots.db-wal", "dots.db-shm"] {
            std::fs::write(dir.path().join(f), f).unwrap();
        }
        let moved = set_aside_database(dir.path(), "0.3.1", at(9)).unwrap();
        assert!(!dir.path().join("dots.db").exists());
        assert!(!dir.path().join("dots.db-wal").exists());
        let name = moved.file_name().unwrap().to_str().unwrap().to_string();
        assert!(name.starts_with("failed-dots-0.3.1-"));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "dots.db");
        // Not a backup: never counted or pruned, never named on the fatal page.
        assert_eq!(newest_backup(dir.path()), None);
        std::fs::write(dir.path().join("dots.db"), "restored").unwrap();
        undo_set_aside(dir.path(), &moved).unwrap();
        for f in ["dots.db", "dots.db-wal", "dots.db-shm"] {
            assert_eq!(std::fs::read_to_string(dir.path().join(f)).unwrap(), f);
        }
    }
}
