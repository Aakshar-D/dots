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
    /// "Restart when it's done": Dots installs once no dot has a run working.
    Waiting {
        version: String,
        waiting_for: Vec<String>,
    },
    /// The install has begun; Dots is about to close.
    Installing { version: String },
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
}

impl UpdateState {
    /// `error`: what the last start left (`take_error`).
    pub fn new(error: Option<String>) -> Self {
        Self {
            error,
            ..Self::default()
        }
    }

    /// Whether a check that found `version` should download it: not while an install waits or
    /// has begun (the person agreed to the version they saw), and not a second time.
    pub fn wants(&self, version: &str) -> bool {
        !self.waiting && !self.installing && self.ready.as_deref() != Some(version)
    }

    /// A downloaded update, checked against Dots' key. True when the note changed.
    pub fn offer(&mut self, version: &str) -> bool {
        if !self.wants(version) {
            return false;
        }
        self.ready = Some(version.to_string());
        !self.later
    }

    pub fn note(&self, working: &[String]) -> UpdateNote {
        let Some(version) = self.ready.clone() else {
            return UpdateNote::None;
        };
        if self.installing {
            UpdateNote::Installing { version }
        } else if self.waiting {
            UpdateNote::Waiting {
                version,
                waiting_for: working.to_vec(),
            }
        } else if self.later {
            UpdateNote::None
        } else {
            UpdateNote::Ready {
                version,
                working: working.to_vec(),
                error: self.error.clone(),
            }
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

    pub fn later(&mut self) {
        self.later = true;
    }

    /// "Restart to update" and "Restart when it's done": installs at once when no dot has a
    /// run working, else waits for those runs.
    pub fn restart(&mut self, working: &[String]) -> dots_core::Result<Next> {
        self.check_ready()?;
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
    /// Starts the installer and ends the process. Returns only when it could not start.
    fn install(&self) -> Result<(), String>;
}

/// An install, in the order of spec §7. Every failure restarts into this version with the
/// reason, which the strip shows after that start.
pub async fn install(steps: &impl InstallSteps) {
    if !steps.close_runtime().await {
        steps.restart_current("Dots could not close its database in time, so no backup was made.");
        return;
    }
    if let Err(e) = steps.back_up().await {
        steps.restart_current(&format!("The backup failed: {e}"));
        return;
    }
    steps.mark_restart();
    if let Err(e) = steps.install() {
        steps.restart_current(&format!("The update could not start: {e}"));
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
        let mut s = UpdateState::new(None);
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
        let mut s = UpdateState::new(Some("The backup failed: disk full".into()));
        s.offer("0.2.0");
        assert_eq!(
            s.note(&names(&["Daily digest"])),
            ready(&["Daily digest"], Some("The backup failed: disk full"))
        );
    }

    #[test]
    fn restart_installs_at_once_when_nothing_works_and_waits_otherwise() {
        let mut s = UpdateState::new(None);
        assert!(s.restart(&[]).is_err(), "nothing is ready");
        s.offer("0.2.0");
        assert_eq!(s.restart(&[]).unwrap(), Next::Install);
        assert_eq!(
            s.note(&[]),
            UpdateNote::Installing {
                version: "0.2.0".into()
            }
        );
        assert!(
            s.restart(&[]).is_err(),
            "an install that has begun is not begun twice"
        );

        let mut s = UpdateState::new(None);
        s.offer("0.2.0");
        let busy = names(&["Daily digest"]);
        assert_eq!(s.restart(&busy).unwrap(), Next::Nothing);
        assert!(s.holds_runs());
        assert_eq!(
            s.note(&busy),
            UpdateNote::Waiting {
                version: "0.2.0".into(),
                waiting_for: busy.clone(),
            }
        );
        assert!(
            !s.wants("0.3.0"),
            "a waiting install keeps the version the person saw"
        );
    }

    #[test]
    fn a_wait_ends_with_the_last_run_or_with_cancel() {
        let mut s = UpdateState::new(None);
        s.offer("0.2.0");
        s.restart(&names(&["a", "b"])).unwrap();
        assert_eq!(s.runs_changed(&names(&["b"])), Next::Nothing);
        assert_eq!(s.runs_changed(&[]), Next::Install);
        assert_eq!(s.runs_changed(&[]), Next::Nothing, "one install");

        let mut s = UpdateState::new(None);
        s.offer("0.2.0");
        s.restart(&names(&["a"])).unwrap();
        s.cancel_wait();
        assert!(!s.holds_runs());
        assert_eq!(s.runs_changed(&[]), Next::Nothing);
        assert_eq!(s.note(&[]), ready(&[], None));
    }

    #[test]
    fn stop_and_restart_installs_when_only_the_dots_it_named_work() {
        let mut s = UpdateState::new(None);
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

        let mut s = UpdateState::new(None);
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
        let mut s = UpdateState::new(None);
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
        let mut s = UpdateState::new(None);
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
        fn install(&self) -> Result<(), String> {
            self.log("install")
                .then_some(())
                .ok_or_else(|| "access denied".to_string())
        }
    }

    #[tokio::test]
    async fn an_install_closes_backs_up_marks_then_installs() {
        let steps = Steps::new("");
        install(&steps).await;
        assert_eq!(steps.done(), ["close", "back up", "mark", "install"]);
    }

    #[tokio::test]
    async fn a_failed_step_restarts_on_this_version_with_the_reason() {
        let steps = Steps::new("close");
        install(&steps).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "restart: Dots could not close its database in time, so no backup was made."
            ]
        );
        let steps = Steps::new("back up");
        install(&steps).await;
        assert_eq!(
            steps.done(),
            ["close", "back up", "restart: The backup failed: disk full"]
        );
        let steps = Steps::new("install");
        install(&steps).await;
        assert_eq!(
            steps.done(),
            [
                "close",
                "back up",
                "mark",
                "install",
                "restart: The update could not start: access denied"
            ]
        );
    }
}
