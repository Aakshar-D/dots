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
    log_dir: PathBuf,
}

impl AppState {
    pub fn new(runtime: Result<Arc<Runtime>, String>, log_dir: PathBuf) -> Self {
        Self { runtime, log_dir }
    }

    pub fn rt(&self) -> Result<Arc<Runtime>, CmdError> {
        self.runtime
            .clone()
            .map_err(|_| CmdError::internal("Dots did not start; see the startup error"))
    }

    pub fn startup_error(&self) -> Option<StartupError> {
        self.runtime.as_ref().err().map(|message| StartupError {
            message: message.clone(),
            log_dir: self.log_dir.to_string_lossy().into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_start_answers_every_command_with_an_internal_error() {
        let state = AppState::new(Err("disk full".into()), PathBuf::from("C:/logs"));
        let err = state.rt().err().unwrap();
        assert_eq!(err.kind, "internal");
        assert_eq!(
            state.startup_error(),
            Some(StartupError {
                message: "disk full".into(),
                log_dir: "C:/logs".into(),
            })
        );
    }
}
