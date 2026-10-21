//! Tauri commands: one-line wrappers over `Runtime`. Runs, dots and approvals are addressed by
//! id, so the UI cannot make the backend open, read or delete an arbitrary path. Two values
//! from the UI are paths, both checked in the core and never opened here: a dot's `workdir`
//! (`DotSpec::validate`, then the runner requires an existing folder) and the `claude_path`
//! setting (`Settings::validate`; the runtime runs that program as the Claude CLI from the
//! next start). The Library commands take more: a skill or plugin folder, a folder to scan for
//! skills, and the command of an MCP server, which `library_probe` starts and a dot's runs
//! start. Like `claude_path`, a new command line or env (and a plugin's servers and hooks) is confirmed in
//! a native dialog from Rust before it is run or stored (`confirm.rs`). The UI is first-party
//! code under a `default-src 'self'` policy.

use dots_core::kit::InstalledPlugin;
use dots_core::library::{LibraryConfig, LibraryItem, McpCatalog, McpServerConfig};
use dots_core::library_service::{
    ContextDocuments, EnvVariable, LibraryEntry, McpProbe, ScannedSkill,
};
use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, FollowUp, Inbox, Message, Run, RunEventRecord,
    SettingsSaved, Status, WorktreeDiff,
};
use dots_core::policy::{Policy, Preset, RuleCandidate};
use dots_core::schedule::Schedule;
use dots_core::settings::Settings;
use dots_core::skill_creator::{check_draft, DraftRequest, DraftSummary};
use dots_core::Runtime;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::confirm;
use crate::error::CmdError;
use crate::state::{AppState, StartupError};
use crate::tray::Tray;
use crate::update::{Next, UpdateNote};
use crate::update_service;

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
    settings: Settings,
) -> Res<SettingsSaved> {
    let rt = state.rt()?;
    // Pointing Dots at another program needs the user's own yes, in a dialog the webview
    // cannot answer.
    let saved = confirm::save_settings_confirmed(&rt, settings, |message| {
        confirm::ask(&app, "Change the Claude program?", message)
    })
    .await?;
    crate::autostart::apply(
        &app,
        saved.settings.autostart,
        !crate::startup::single_instance(std::env::var_os(crate::startup::DATA_DIR_ENV)),
    );
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
/// interrupted; the UI warns first. The restart passes the old arguments on, `--hidden` of a
/// start at login included, so a mark in the data folder tells the new process to show its
/// window: the person who clicked is waiting for it.
#[tauri::command]
pub fn restart_app(app: AppHandle, state: State<'_, AppState>) {
    if let Ok(rt) = state.rt() {
        rt.shutdown();
    }
    if let Some(data_dir) = state.data_dir() {
        crate::startup::mark_restart(data_dir);
    }
    app.restart();
}

#[tauri::command]
pub async fn update_note(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::note(&app, &state.rt()?).await
}

/// "What's new": opens the release page of the update that is ready. The page is the core's
/// choice, so the webview cannot open an address of its own.
#[tauri::command]
pub fn update_open_notes(app: AppHandle) -> Res<()> {
    let page = update_service::release_page(&app)?;
    app.opener()
        .open_url(&page, None::<&str>)
        .map_err(|e| CmdError::internal(format!("could not open {page}: {e}")))
}

/// "Later": the note is hidden until Dots next starts.
#[tauri::command]
pub async fn update_later(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::press(&app, &state.rt()?, |s, _| {
        s.later();
        Ok(Next::Nothing)
    })
    .await
}

/// "Restart to update" and "Restart when it's done": installs now when no dot has a run
/// working, else once the last of them has ended; no queued run starts meanwhile.
#[tauri::command]
pub async fn update_restart(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::press(&app, &state.rt()?, |s, working| s.restart(working)).await
}

/// "Stop it and restart": installs now. A run that is working ends as failed ("interrupted")
/// at the next start, as after "Restart Dots"; the UI asks first. `working`: the dots the
/// confirm named. When another dot is working now, nothing is installed and the note that
/// follows names the dots working now.
#[tauri::command]
pub async fn update_stop_and_restart(
    app: AppHandle,
    state: State<'_, AppState>,
    working: Vec<String>,
) -> Res<UpdateNote> {
    update_service::press(&app, &state.rt()?, |s, now| {
        s.stop_and_restart(&working, now)
    })
    .await
}

/// "Cancel" of a waiting install: the queue starts again.
#[tauri::command]
pub async fn update_cancel_wait(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::press(&app, &state.rt()?, |s, _| {
        s.cancel_wait();
        Ok(Next::Nothing)
    })
    .await
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
pub fn preview_schedule(schedule: Schedule, count: usize) -> Res<Vec<String>> {
    Ok(Runtime::preview_schedule(&schedule, count)?)
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
    allow_rule: Option<String>,
) -> Res<Approval> {
    Ok(state
        .rt()?
        .decide_approval(&id, approved, note, allow_rule)
        .await?)
}

#[tauri::command]
pub async fn approval_rule_candidates(
    state: State<'_, AppState>,
    id: String,
) -> Res<Vec<RuleCandidate>> {
    Ok(state.rt()?.approval_rule_candidates(&id).await?)
}

#[tauri::command]
pub async fn decide_run_approvals(
    state: State<'_, AppState>,
    run_id: String,
    approved: bool,
    note: Option<String>,
) -> Res<Vec<Approval>> {
    Ok(state
        .rt()?
        .decide_run_approvals(&run_id, approved, note)
        .await?)
}

#[tauri::command]
pub async fn inbox(state: State<'_, AppState>) -> Res<Inbox> {
    Ok(state.rt()?.inbox().await?)
}

#[tauri::command]
pub async fn inbox_count(state: State<'_, AppState>) -> Res<usize> {
    Ok(state.rt()?.inbox_count().await?)
}

/// Clears the "Needs attention" of the dot's schedule.
#[tauri::command]
pub async fn dismiss_schedule_attention(state: State<'_, AppState>, dot_id: String) -> Res<()> {
    Ok(state.rt()?.dismiss_schedule_attention(&dot_id).await?)
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

/// Every Library item with its users, catalog and, for a skill or plugin, what its folder holds.
#[tauri::command]
pub async fn library_list(state: State<'_, AppState>) -> Res<Vec<LibraryEntry>> {
    Ok(state.rt()?.library().list().await?)
}

/// `accept`: the fingerprint `library_probe` answered. The tool list the server gives now is
/// accepted only when it has that fingerprint; without it, a server is saved unchecked.
#[tauri::command]
pub async fn library_add(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    config: LibraryConfig,
    accept: Option<String>,
) -> Res<LibraryItem> {
    let rt = state.rt()?;
    confirm::add_confirmed(&rt, &name, config, accept.as_deref(), |message| {
        confirm::ask(&app, confirm::library_title(&message), message)
    })
    .await
}

#[tauri::command]
pub async fn library_update(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    config: LibraryConfig,
) -> Res<LibraryItem> {
    let rt = state.rt()?;
    confirm::update_confirmed(&rt, &id, config, |message| {
        confirm::ask(&app, confirm::library_title(&message), message)
    })
    .await
}

/// Refused with kind `conflict`, naming the dots, while dots use the item and `confirm` is not set.
#[tauri::command]
pub async fn library_remove(state: State<'_, AppState>, id: String, confirm: bool) -> Res<()> {
    Ok(state.rt()?.library().remove(&id, confirm).await?)
}

/// Starts the server, lists its tools and stops it again; nothing is saved. A stdio server needs the
/// person's yes in a native dialog first (`confirm::library_confirmation`), as do `library_add`
/// and `library_update` for a stdio command line or env they do not already hold, for the
/// stdio servers of a new plugin (or a new plugin path) and for plugin hooks switched on; a refusal answers `cancelled` and nothing is started or stored.
#[tauri::command]
pub async fn library_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    config: McpServerConfig,
) -> Res<McpProbe> {
    let rt = state.rt()?;
    confirm::probe_confirmed(&rt, config, |message| {
        confirm::ask(&app, confirm::library_title(&message), message)
    })
    .await
}

#[tauri::command]
pub async fn library_check(state: State<'_, AppState>, id: String) -> Res<McpCatalog> {
    Ok(state.rt()?.library().check(&id).await?)
}

/// Refused with kind `conflict` when the fingerprint is no longer the one last seen.
#[tauri::command]
pub async fn library_accept(
    state: State<'_, AppState>,
    id: String,
    fingerprint: String,
) -> Res<McpCatalog> {
    Ok(state.rt()?.library().accept(&id, &fingerprint).await?)
}

#[tauri::command]
pub async fn library_scan_skills(
    state: State<'_, AppState>,
    path: String,
) -> Res<Vec<ScannedSkill>> {
    Ok(state.rt()?.library().scan_skills(&path).await?)
}

/// Drafts a skill with the `claude` CLI for the Create skill dialog `dialog` (skill creator
/// spec §5): no tools, no MCP servers, none of the person's settings. A draft the same dialog
/// asked for before is cancelled.
#[tauri::command]
pub async fn skill_draft(
    state: State<'_, AppState>,
    dialog: String,
    request: DraftRequest,
) -> Res<String> {
    Ok(state.rt()?.skill_drafts().draft(&dialog, &request).await?)
}

/// Ends the dialog's draft, if one is being written; its `skill_draft` then fails.
#[tauri::command]
pub fn skill_draft_cancel(state: State<'_, AppState>, dialog: String) -> Res<()> {
    state.rt()?.skill_drafts().cancel(&dialog);
    Ok(())
}

/// The name and description a draft would be saved with, or, as `invalid`, what to change.
#[tauri::command]
pub fn skill_check(text: String) -> Res<DraftSummary> {
    Ok(check_draft(&text)?)
}

/// Saves a draft as a new skill in Dots' skills folder and the Library (skill creator spec §6).
#[tauri::command]
pub async fn skill_create(state: State<'_, AppState>, text: String) -> Res<LibraryItem> {
    Ok(state.rt()?.library().create_skill(&text).await?)
}

/// The first documents of a context collection, with their titles and summaries.
#[tauri::command]
pub async fn context_documents(state: State<'_, AppState>, id: String) -> Res<ContextDocuments> {
    Ok(state.rt()?.library().context_documents(&id).await?)
}

#[tauri::command]
pub fn installed_claude_plugins(state: State<'_, AppState>) -> Res<Vec<InstalledPlugin>> {
    Ok(state.rt()?.library().installed_plugins())
}

/// Which of the names Dots' environment sets. Never a value.
#[tauri::command]
pub fn environment_variables(
    state: State<'_, AppState>,
    names: Vec<String>,
) -> Res<Vec<EnvVariable>> {
    Ok(state.rt()?.library().environment_variables(&names))
}

/// Every message that waits for the person and the newest others, newest first; of one dot
/// with `dot_id` (check-ins spec section 10).
#[tauri::command]
pub async fn messages_list(
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<Vec<Message>> {
    Ok(state
        .rt()?
        .checkins()
        .list_messages(dot_id.as_deref())
        .await?)
}

/// Answers a dot's question: a run continues the question's thread.
#[tauri::command]
pub async fn message_reply(state: State<'_, AppState>, id: String, text: String) -> Res<Run> {
    Ok(state.rt()?.reply(&id, &text).await?)
}

#[tauri::command]
pub async fn message_dismiss(state: State<'_, AppState>, id: String) -> Res<Message> {
    Ok(state.rt()?.checkins().dismiss(&id).await?)
}

#[tauri::command]
pub async fn message_mark_read(state: State<'_, AppState>, id: String) -> Res<Message> {
    Ok(state.rt()?.checkins().mark_read(&id).await?)
}

/// The pending follow-ups, soonest first; of one dot with `dot_id`.
#[tauri::command]
pub async fn follow_ups_list(
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<Vec<FollowUp>> {
    Ok(state.rt()?.follow_ups().list(dot_id.as_deref()).await?)
}

#[tauri::command]
pub async fn follow_up_cancel(state: State<'_, AppState>, id: String) -> Res<FollowUp> {
    Ok(state.rt()?.follow_ups().cancel(&id).await?)
}
