//! Tauri commands: one-line wrappers over `Runtime`. They take ids, never paths.

use dots_core::model::Status;
use tauri::State;

use crate::error::CmdError;
use crate::state::{AppState, StartupError};

type Res<T> = Result<T, CmdError>;

#[tauri::command]
pub fn startup_error(state: State<'_, AppState>) -> Option<StartupError> {
    state.startup_error()
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> Res<Status> {
    Ok(state.rt()?.status().await)
}
