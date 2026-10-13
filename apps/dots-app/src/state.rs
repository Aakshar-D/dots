use std::path::PathBuf;
use std::sync::Arc;

use dots_core::Runtime;
use serde::{Deserialize, Serialize};

use crate::error::CmdError;

/// What the fatal-error page shows when the runtime did not start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartupError {
    pub message: String,
    pub log_dir: String,
}

/// Tauri-managed state: the runtime, or why it is missing.
pub struct AppState {
    runtime: Result<Arc<Runtime>, String>,
    log_dir: Option<PathBuf>,
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
        Self { runtime, log_dir }
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
    fn a_failed_start_answers_every_command_with_an_internal_error() {
        let state = AppState::new(Err("disk full".into()), Some(PathBuf::from("C:/logs")));
        let err = state.rt().err().unwrap();
        assert_eq!(err.kind, CmdErrorKind::Internal);
        assert_eq!(
            state.startup_error(),
            Some(StartupError {
                message: "disk full".into(),
                log_dir: "C:/logs".into(),
            })
        );
    }
}
