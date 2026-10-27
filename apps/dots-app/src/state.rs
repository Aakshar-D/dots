use std::path::{Path, PathBuf};
use std::sync::Arc;

use dots_core::Runtime;
use serde::{Deserialize, Serialize};

use crate::error::CmdError;

/// What the fatal-error page shows when the runtime did not start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartupError {
    pub message: String,
    pub log_dir: String,
    /// The newest backup an update made, empty when there is none (updates spec §8).
    pub backup: String,
    /// The version Go back installs, when the last update left a backup to restore (updates
    /// UI spec section 4.4).
    pub go_back: Option<String>,
}

/// Tauri-managed state: the runtime, or why it is missing, and the folders `setup` found.
pub struct AppState {
    runtime: Result<Arc<Runtime>, String>,
    log_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
}

/// The runtime, if the state is managed and the runtime started. For code that can run when
/// `setup` did not get as far as managing the state (the tray menu, the exit handler), where
/// `app.state` would panic: pass `app.try_state::<AppState>().as_deref()`.
pub fn runtime_of(state: Option<&AppState>) -> Option<Arc<Runtime>> {
    state.and_then(|s| s.rt().ok())
}

impl AppState {
    /// `log_dir` is `None` when no data folder was found, so there is no log.
    pub fn new(runtime: Result<Arc<Runtime>, String>, log_dir: Option<PathBuf>) -> Self {
        Self {
            runtime,
            log_dir,
            data_dir: None,
        }
    }

    /// Keeps the data folder `setup` found, whether or not the runtime started in it.
    pub fn with_data_dir(mut self, data_dir: Option<PathBuf>) -> Self {
        self.data_dir = data_dir;
        self
    }

    /// The data folder, for what a command leaves there itself (the restart mark). `None`
    /// when no data folder was found.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    pub fn rt(&self) -> Result<Arc<Runtime>, CmdError> {
        self.runtime
            .clone()
            .map_err(|_| CmdError::internal("Dots did not start; see the startup error"))
    }

    /// True only when the runtime started: then the tray exists and closing the window hides it.
    pub fn close_hides_window(&self) -> bool {
        self.runtime.is_ok()
    }

    /// `log_dir` is empty when there is no log folder; the page then leaves that line out.
    pub fn startup_error(&self) -> Option<StartupError> {
        self.runtime.as_ref().err().map(|message| StartupError {
            message: message.clone(),
            log_dir: self
                .log_dir
                .as_ref()
                .map(|dir| dir.to_string_lossy().into_owned())
                .unwrap_or_default(),
            backup: self
                .data_dir
                .as_deref()
                .and_then(crate::update::newest_backup)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            go_back: self.data_dir.as_deref().and_then(|dir| {
                crate::update_record::go_back_target(&crate::update_record::read(dir))
                    .map(|(version, _)| version)
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CmdErrorKind;

    #[test]
    fn closing_the_window_exits_after_a_failed_start() {
        let state = AppState::new(Err("disk full".into()), Some(PathBuf::from("C:/logs")));
        assert!(!state.close_hides_window());
    }

    #[test]
    fn a_start_that_found_no_data_folder_fails_like_any_other_without_a_log_folder() {
        let state = AppState::new(Err("no local data folder".into()), None);
        assert!(!state.close_hides_window());
        assert_eq!(state.rt().err().unwrap().kind, CmdErrorKind::Internal);
        // The fatal page leaves the log folder line out when the text is empty.
        assert_eq!(
            state.startup_error(),
            Some(StartupError {
                message: "no local data folder".into(),
                log_dir: "".into(),
                backup: "".into(),
                go_back: None,
            })
        );
    }

    #[test]
    fn there_is_no_runtime_without_a_managed_state_or_after_a_failed_start() {
        assert!(runtime_of(None).is_none());
        let failed = AppState::new(Err("disk full".into()), None);
        assert!(runtime_of(Some(&failed)).is_none());
    }

    #[test]
    fn the_data_folder_is_kept_also_after_a_failed_start() {
        let state = AppState::new(Err("port in use".into()), Some(PathBuf::from("C:/d/logs")));
        assert_eq!(state.data_dir(), None);
        // "Restart Dots" on the fatal-error page leaves its mark there too.
        let state = state.with_data_dir(Some(PathBuf::from("C:/d")));
        assert_eq!(state.data_dir(), Some(std::path::Path::new("C:/d")));
        assert!(state.startup_error().is_some());
    }

    #[test]
    fn a_failed_start_answers_every_command_with_an_internal_error() {
        let state = AppState::new(Err("disk full".into()), Some(PathBuf::from("C:/logs")));
        let err = state.rt().err().unwrap();
        assert_eq!(err.kind, CmdErrorKind::Internal);
        assert_eq!(
            state.startup_error(),
            Some(StartupError {
                message: "disk full".into(),
                log_dir: "C:/logs".into(),
                backup: "".into(),
                go_back: None,
            })
        );
    }

    #[test]
    fn a_failed_start_names_the_newest_backup_of_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join(crate::update::BACKUPS_DIR);
        std::fs::create_dir_all(&backups).unwrap();
        let newest = backups.join("dots-0.1.0-2026-10-20-091700.db");
        std::fs::write(backups.join("dots-0.1.0-2026-10-19-091700.db"), "").unwrap();
        std::fs::write(&newest, "").unwrap();
        let state = AppState::new(Err("migration 8 failed".into()), None)
            .with_data_dir(Some(dir.path().to_path_buf()));
        let shown = state.startup_error().unwrap();
        assert_eq!(shown.backup, newest.to_string_lossy());
    }

    #[test]
    fn the_fatal_page_offers_go_back_when_the_record_does() {
        let dir = tempfile::tempdir().unwrap();
        let state = |dir: &std::path::Path| {
            AppState::new(Err("migration 0017 failed".into()), None)
                .with_data_dir(Some(dir.to_path_buf()))
        };
        assert_eq!(state(dir.path()).startup_error().unwrap().go_back, None);
        let backup = dir
            .path()
            .join("backups")
            .join("dots-0.3.0-2026-10-27-031200.db");
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        std::fs::write(&backup, "db").unwrap();
        crate::update_record::write(
            dir.path(),
            &crate::update_record::UpdateRecord {
                leaving: Some("0.3.0".into()),
                installing: Some("0.3.1".into()),
                backup: Some(backup),
                pending: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            state(dir.path())
                .startup_error()
                .unwrap()
                .go_back
                .as_deref(),
            Some("0.3.0")
        );
    }
}
