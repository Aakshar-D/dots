//! Tauri commands: one-line wrappers over `Runtime`. Runs, dots and approvals are addressed by
//! id, so the UI cannot make the backend open, read or delete an arbitrary path. Two values
//! from the UI are paths, both checked in the core and never opened here: a dot's `workdir`
//! (`DotSpec::validate`, then the runner requires an existing folder) and the `claude_path`
//! setting (`Settings::validate`; the runtime runs that program as the Claude CLI from the
//! next start). The UI is first-party code under a `default-src 'self'` policy.

use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, Inbox, Run, RunEventRecord, SettingsSaved, Status,
    WorktreeDiff,
};
use dots_core::policy::{Policy, Preset};
use dots_core::settings::Settings;
use dots_core::Runtime;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::confirm;
use crate::error::CmdError;
use crate::state::{AppState, StartupError};
use crate::tray::Tray;

type Res<T> = Result<T, CmdError>;

#[tauri::command]
pub fn startup_error(state: State<'_, AppState>) -> Option<StartupError> {
    state.startup_error()
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> Res<Status> {
    Ok(state.rt()?.status().await)
}

#[tauri::command]
pub async fn mark_claude_tested(state: State<'_, AppState>) -> Res<Status> {
    Ok(state.rt()?.mark_claude_tested().await?)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Res<Settings> {
    Ok(state.rt()?.settings().await)
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Res<SettingsSaved> {
    // Normalized once, so the string compared, shown and saved is the same one.
    settings.claude_path = settings.claude_path.map(|p| p.trim().to_string());
    let rt = state.rt()?;
    // Pointing Dots at another program needs the user's own yes, in a dialog the webview
    // cannot answer.
    let stored = rt.settings().await;
    if let Some(program) = confirm::claude_program_to_confirm(
        stored.claude_path.as_deref(),
        settings.claude_path.as_deref(),
    ) {
        let message = confirm::claude_program_message(&program);
        if !confirm::ask(&app, "Change the Claude program?", message).await {
            return Err(CmdError::cancelled("the Claude program was not changed"));
        }
    }
    let saved = rt.update_settings(settings).await?;
    crate::autostart::apply(&app, saved.settings.autostart);
    if let Some(tray) = app.try_state::<Tray>() {
        tray.set_paused(saved.settings.schedules_paused);
    }
    Ok(saved)
}

#[tauri::command]
pub async fn set_schedules_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Res<Status> {
    let status = state.rt()?.set_schedules_paused(paused).await?;
    if let Some(tray) = app.try_state::<Tray>() {
        tray.set_paused(status.schedules_paused);
    }
    Ok(status)
}

/// Restarts the app so port, concurrency and `claude` path changes apply. Active runs are
/// interrupted; the UI warns first.
#[tauri::command]
pub fn restart_app(app: AppHandle, state: State<'_, AppState>) {
    if let Ok(rt) = state.rt() {
        rt.shutdown();
    }
    app.restart();
}

#[tauri::command]
pub async fn dot_overview(state: State<'_, AppState>) -> Res<Vec<DotOverview>> {
    Ok(state.rt()?.dot_overview().await?)
}

#[tauri::command]
pub async fn list_dots(state: State<'_, AppState>) -> Res<Vec<Dot>> {
    Ok(state.rt()?.list_dots().await?)
}

#[tauri::command]
pub async fn get_dot(state: State<'_, AppState>, id: String) -> Res<Dot> {
    Ok(state.rt()?.get_dot(&id).await?)
}

#[tauri::command]
pub async fn create_dot(state: State<'_, AppState>, spec: DotSpec) -> Res<Dot> {
    Ok(state.rt()?.create_dot(spec).await?)
}

#[tauri::command]
pub async fn update_dot(state: State<'_, AppState>, id: String, spec: DotSpec) -> Res<Dot> {
    Ok(state.rt()?.update_dot(&id, spec).await?)
}

#[tauri::command]
pub async fn delete_dot(state: State<'_, AppState>, id: String) -> Res<()> {
    Ok(state.rt()?.delete_dot(&id).await?)
}

#[tauri::command]
pub async fn set_dot_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> Res<Dot> {
    Ok(state.rt()?.set_dot_enabled(&id, enabled).await?)
}

#[tauri::command]
pub async fn regenerate_webhook_token(state: State<'_, AppState>, id: String) -> Res<String> {
    Ok(state.rt()?.regenerate_webhook_token(&id).await?)
}

#[tauri::command]
pub async fn webhook_url(state: State<'_, AppState>, id: String) -> Res<String> {
    let rt = state.rt()?;
    let dot = rt.get_dot(&id).await?;
    Ok(rt.webhook_url(&dot)?)
}

#[tauri::command]
pub fn preview_schedule(expr: String, count: usize) -> Res<Vec<String>> {
    Ok(Runtime::preview_schedule(&expr, count)?)
}

/// The rules of a policy preset, for the editor's preset picker.
#[tauri::command]
pub fn policy_preset(preset: Preset) -> Policy {
    Policy::preset(preset)
}

/// A dot with the core's defaults and nothing filled in, for the "New dot" form.
#[tauri::command]
pub fn new_dot_spec() -> DotSpec {
    DotSpec::new("", "", "")
}

#[tauri::command]
pub async fn test_endpoint(
    state: State<'_, AppState>,
    endpoint: String,
    model: String,
) -> Res<String> {
    Ok(state.rt()?.test_endpoint(&endpoint, &model).await?)
}

#[tauri::command]
pub async fn run_now(state: State<'_, AppState>, dot_id: String) -> Res<Run> {
    Ok(state.rt()?.run_now(&dot_id).await?)
}

#[tauri::command]
pub async fn chat(
    state: State<'_, AppState>,
    dot_id: String,
    message: String,
    parent_run_id: Option<String>,
) -> Res<Run> {
    Ok(state
        .rt()?
        .chat(&dot_id, &message, parent_run_id.as_deref())
        .await?)
}

#[tauri::command]
pub async fn cancel_run(state: State<'_, AppState>, run_id: String) -> Res<Run> {
    Ok(state.rt()?.cancel_run(&run_id).await?)
}

#[tauri::command]
pub async fn list_runs(
    state: State<'_, AppState>,
    dot_id: Option<String>,
    limit: i64,
) -> Res<Vec<Run>> {
    Ok(state.rt()?.list_runs(dot_id.as_deref(), limit).await?)
}

#[tauri::command]
pub async fn get_run(state: State<'_, AppState>, id: String) -> Res<Run> {
    Ok(state.rt()?.get_run(&id).await?)
}

#[tauri::command]
pub async fn run_thread(state: State<'_, AppState>, run_id: String) -> Res<Vec<Run>> {
    Ok(state.rt()?.run_thread(&run_id).await?)
}

#[tauri::command]
pub async fn list_run_events(
    state: State<'_, AppState>,
    run_id: String,
    after_seq: i64,
) -> Res<Vec<RunEventRecord>> {
    Ok(state.rt()?.list_run_events(&run_id, after_seq).await?)
}

#[tauri::command]
pub async fn approvals_for_run(state: State<'_, AppState>, run_id: String) -> Res<Vec<Approval>> {
    Ok(state.rt()?.approvals_for_run(&run_id).await?)
}

#[tauri::command]
pub async fn decide_approval(
    state: State<'_, AppState>,
    id: String,
    approved: bool,
    note: Option<String>,
) -> Res<Approval> {
    Ok(state.rt()?.decide_approval(&id, approved, note).await?)
}

#[tauri::command]
pub async fn inbox(state: State<'_, AppState>) -> Res<Inbox> {
    Ok(state.rt()?.inbox().await?)
}

#[tauri::command]
pub async fn inbox_count(state: State<'_, AppState>) -> Res<usize> {
    Ok(state.rt()?.inbox_count().await?)
}

#[tauri::command]
pub async fn worktree_diff(state: State<'_, AppState>, run_id: String) -> Res<WorktreeDiff> {
    Ok(state.rt()?.worktree_diff(&run_id).await?)
}

#[tauri::command]
pub async fn discard_worktree(state: State<'_, AppState>, run_id: String) -> Res<Run> {
    Ok(state.rt()?.discard_worktree(&run_id).await?)
}

#[tauri::command]
pub async fn mark_reviewed(state: State<'_, AppState>, run_id: String) -> Res<Run> {
    Ok(state.rt()?.mark_reviewed(&run_id).await?)
}

/// Opens the run's folder in Explorer. The path comes from the run record, never the UI.
#[tauri::command]
pub async fn open_run_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
) -> Res<()> {
    let path = state.rt()?.run_folder(&run_id).await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| CmdError::internal(format!("could not open {}: {e}", path.display())))
}
