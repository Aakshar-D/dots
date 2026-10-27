//! What the last install left for the next start (updates UI spec section 4.4): the version
//! it left and the one it installed, the database backup it made, whether the new version's
//! health check is still pending, and the versions this machine found bad. A file beside
//! `dots.db`, not a table in it, because going back replaces the database.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const RECORD_FILE: &str = "update-state.json";
/// How long a new version has to start and show its window.
pub const HEALTH_LIMIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateRecord {
    pub leaving: Option<String>,
    pub installing: Option<String>,
    pub backup: Option<PathBuf>,
    /// The health check of `installing` has not passed yet.
    pub pending: bool,
    /// How many starts of `installing` began the health check.
    pub starts: u32,
    /// When an idle install began (RFC 3339 UTC); only set by one.
    pub by_itself_at: Option<String>,
    /// Versions never offered to this machine again.
    pub bad: Vec<String>,
    /// The version whose idle install failed: never installed by itself again (it can still
    /// be installed from the strip).
    pub idle_failed: Option<String>,
}

/// The new version's health, at a start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Fine,
    Checking,
    /// `back_to`: the version Go back installs, when it can.
    Failed {
        version: String,
        back_to: Option<String>,
    },
}

/// The record, or nothing pending when it is missing or cannot be read: a broken record never
/// stops Dots from starting.
pub fn read(data_dir: &Path) -> UpdateRecord {
    let file = data_dir.join(RECORD_FILE);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return UpdateRecord::default();
    };
    if text.trim().is_empty() {
        tracing::warn!("{} is empty, so nothing is pending", file.display());
        return UpdateRecord::default();
    }
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!(
            "{} cannot be read, so nothing is pending: {e}",
            file.display()
        );
        UpdateRecord::default()
    })
}

/// Writes the whole record: a temporary file first, then renamed over the old one.
pub fn write(data_dir: &Path, record: &UpdateRecord) -> Result<(), String> {
    let file = data_dir.join(RECORD_FILE);
    let tmp = data_dir.join(format!("{RECORD_FILE}.tmp"));
    let text = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("writing {}: {e}", file.display()))
}

/// What this start of `current` means for the health check. `runtime_ok`: the runtime started.
pub fn on_start(record: &mut UpdateRecord, current: &str, runtime_ok: bool) -> Health {
    if !record.pending {
        return Health::Fine;
    }
    if record.installing.as_deref() != Some(current) {
        // The install never happened: this is still the version it would have replaced.
        record.pending = false;
        record.starts = 0;
        return Health::Fine;
    }
    if !runtime_ok || record.starts >= 1 {
        return Health::Failed {
            version: current.to_string(),
            back_to: go_back_target(record, current).map(|(version, _)| version),
        };
    }
    record.starts += 1;
    Health::Checking
}

/// The new version started and showed its window.
pub fn passed(record: &mut UpdateRecord) {
    record.pending = false;
    record.starts = 0;
}

/// `version` failed its health check: never offered again. Still pending, so the next start
/// offers Go back too.
pub fn failed(record: &mut UpdateRecord, version: &str) {
    if !record.bad.iter().any(|v| v == version) {
        record.bad.push(version.to_string());
    }
}

/// The version Go back installs and the backup it restores: only on `current` as the last
/// update installed it, from another version, while that backup exists.
pub fn go_back_target(record: &UpdateRecord, current: &str) -> Option<(String, PathBuf)> {
    if record.installing.as_deref() != Some(current) {
        return None;
    }
    let version = record.leaving.clone().filter(|v| v != current)?;
    let backup = record.backup.clone()?;
    backup.is_file().then_some((version, backup))
}

/// This start's health check: the record read, `on_start` applied, a failed version marked
/// bad, and the record written only when that changed it (a record this Dots cannot read is
/// left as it is: a newer Dots may have written it).
pub fn settle_start(data_dir: &Path, current: &str, runtime_ok: bool) -> (UpdateRecord, Health) {
    let mut record = read(data_dir);
    let before = record.clone();
    let health = on_start(&mut record, current, runtime_ok);
    if let Health::Failed { version, .. } = &health {
        failed(&mut record, version);
    }
    if record != before {
        if let Err(e) = write(data_dir, &record) {
            tracing::warn!("could not write the update record: {e}");
        }
    }
    (record, health)
}

/// The note this start leaves: Unhealthy after a failed check; "updated itself" while an idle
/// install's note was never dismissed (the window may not have been opened since).
pub fn start_notice(
    health: &Health,
    record: &UpdateRecord,
    current: &str,
) -> Option<crate::update::UpdateNote> {
    match health {
        Health::Failed { version, back_to } => Some(crate::update::UpdateNote::Unhealthy {
            version: version.clone(),
            back_to: back_to.clone(),
        }),
        Health::Fine => {
            record
                .by_itself_at
                .clone()
                .map(|at| crate::update::UpdateNote::UpdatedItself {
                    version: current.to_string(),
                    at,
                })
        }
        Health::Checking => None,
    }
}

/// Dots went back: nothing is left to go back to, and nothing is pending.
pub fn went_back(record: &mut UpdateRecord) {
    record.leaving = None;
    record.installing = None;
    record.backup = None;
    record.pending = false;
    record.starts = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record of an install from 0.3.0 to 0.3.1 with its backup on disk.
    fn installed(dir: &Path) -> UpdateRecord {
        let backup = dir.join("dots-0.3.0-backup.db");
        std::fs::write(&backup, "db").unwrap();
        UpdateRecord {
            leaving: Some("0.3.0".into()),
            installing: Some("0.3.1".into()),
            backup: Some(backup),
            pending: true,
            ..UpdateRecord::default()
        }
    }

    #[test]
    fn a_missing_or_broken_record_reads_as_nothing_pending() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(RECORD_FILE);
        let mut readings = vec![read(dir.path())];
        for text in ["", "{not json", r#"{"pending":false,"future_field":1}"#] {
            std::fs::write(&file, text).unwrap();
            readings.push(read(dir.path()));
        }
        for mut record in readings {
            assert!(!record.pending);
            assert_eq!(on_start(&mut record, "0.3.0", true), Health::Fine);
        }
    }

    #[test]
    fn a_record_is_written_whole_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        record.bad = vec!["0.2.9".into()];
        record.by_itself_at = Some("2026-10-27T03:12:00.000Z".into());
        write(dir.path(), &record).unwrap();
        assert_eq!(read(dir.path()), record);
        assert!(!dir.path().join("update-state.json.tmp").exists());
    }

    #[test]
    fn an_install_that_never_happened_clears_pending() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        assert_eq!(on_start(&mut record, "0.3.0", true), Health::Fine);
        assert!(!record.pending);
    }

    #[test]
    fn the_first_start_of_the_new_version_checks_and_a_pass_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        assert_eq!(on_start(&mut record, "0.3.1", true), Health::Checking);
        assert_eq!(record.starts, 1);
        passed(&mut record);
        assert!(!record.pending);
        assert_eq!(record.starts, 0);
        assert_eq!(on_start(&mut record, "0.3.1", true), Health::Fine);
    }

    #[test]
    fn a_start_that_failed_to_start_the_runtime_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        assert_eq!(
            on_start(&mut record, "0.3.1", false),
            Health::Failed {
                version: "0.3.1".into(),
                back_to: Some("0.3.0".into())
            }
        );
    }

    #[test]
    fn a_start_that_never_rendered_fails_at_the_limit_and_the_next_start_offers_go_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        assert_eq!(on_start(&mut record, "0.3.1", true), Health::Checking);
        failed(&mut record, "0.3.1");
        failed(&mut record, "0.3.1");
        assert_eq!(record.bad, vec!["0.3.1".to_string()]);
        assert!(record.pending);
        assert_eq!(
            on_start(&mut record, "0.3.1", true),
            Health::Failed {
                version: "0.3.1".into(),
                back_to: Some("0.3.0".into())
            }
        );
    }

    #[test]
    fn go_back_needs_the_backup_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        assert_eq!(
            go_back_target(&record, "0.3.1"),
            Some(("0.3.0".to_string(), record.backup.clone().unwrap()))
        );
        std::fs::remove_file(record.backup.as_ref().unwrap()).unwrap();
        assert_eq!(go_back_target(&record, "0.3.1"), None);
        assert_eq!(
            on_start(&mut record, "0.3.1", false),
            Health::Failed {
                version: "0.3.1".into(),
                back_to: None
            }
        );
    }

    // Final review I-1: Go back only from the version the last update installed.
    #[test]
    fn go_back_is_only_from_the_version_the_update_installed() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = installed(dir.path());
        assert!(go_back_target(&record, "0.3.1").is_some());
        // An install that did not happen: still on the version it would have left.
        assert_eq!(go_back_target(&record, "0.3.0"), None);
        // After going back, nothing is left to go back to.
        went_back(&mut record);
        assert_eq!(go_back_target(&record, "0.3.0"), None);
        assert_eq!(go_back_target(&record, "0.3.1"), None);
    }

    // Deferred minor M-2: a start with nothing to change leaves the record as it is.
    #[test]
    fn a_start_with_nothing_to_change_leaves_the_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(RECORD_FILE);
        std::fs::write(&file, "{written by a newer Dots").unwrap();
        let (_, health) = settle_start(dir.path(), "0.3.0", true);
        assert_eq!(health, Health::Fine);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "{written by a newer Dots"
        );
        // A start that does change it writes it.
        write(dir.path(), &installed(dir.path())).unwrap();
        let (record, health) = settle_start(dir.path(), "0.3.1", true);
        assert_eq!(health, Health::Checking);
        assert_eq!(read(dir.path()), record);
    }

    // Deferred minor M-5: "updated itself" waits for the window, across starts, until dismissed.
    #[test]
    fn updated_itself_is_said_until_it_is_dismissed() {
        let mut record = UpdateRecord {
            by_itself_at: Some("2026-10-27T03:12:00.000Z".into()),
            ..UpdateRecord::default()
        };
        assert_eq!(
            start_notice(&Health::Fine, &record, "0.3.1"),
            Some(crate::update::UpdateNote::UpdatedItself {
                version: "0.3.1".into(),
                at: "2026-10-27T03:12:00.000Z".into()
            })
        );
        assert_eq!(start_notice(&Health::Checking, &record, "0.3.1"), None);
        record.by_itself_at = None;
        assert_eq!(start_notice(&Health::Fine, &record, "0.3.1"), None);
    }
}
