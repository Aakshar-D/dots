//! Tauri commands: one-line wrappers over `Runtime`. Runs, dots and approvals are addressed by
//! id, so the UI cannot make the backend open, read or delete an arbitrary path. Two values
//! from the UI are paths, both checked in the core and never opened here: a dot's `workdir`
//! (`DotSpec::validate`, then the runner requires an existing folder) and the `claude_path`
//! setting (`Settings::validate`; the runtime runs that program as the Claude CLI from the
//! next start). A folder watcher's `path` is checked in the core (`watchers::check_config`:
//! absolute, an existing folder) and only watched; its files are opened by the dot's run under
//! the dot's policy. The Library commands take more: a skill or plugin folder, a folder to scan for
//! skills, and the command of an MCP server, which `library_probe` starts and a dot's runs
//! start. Like `claude_path`, a new command line or env (and a plugin's servers and hooks) is confirmed in
//! a native dialog from Rust before it is run or stored (`confirm.rs`). The UI is first-party
//! code under a `default-src 'self'` policy.

use dots_core::apps::propose::{AppProposal, Decision};
use dots_core::apps::screen::{AppFile, AppRow};
use dots_core::apps::AppCall;
use dots_core::browser::BrowserStatus;
use dots_core::kit::InstalledPlugin;
use dots_core::library::{LibraryConfig, LibraryItem, McpCatalog, McpServerConfig};
use dots_core::library_service::{
    ContextDocuments, EnvVariable, LibraryEntry, McpProbe, ScannedSkill,
};
use dots_core::model::{
    Approval, Dot, DotOverview, DotSpec, FollowUp, Inbox, Memory, Message, Run, RunEventRecord,
    SettingsSaved, Status, Watcher, WatcherConfig, WorktreeDiff,
};
use dots_core::policy::{Policy, Preset, RuleCandidate};
use dots_core::schedule::Schedule;
use dots_core::settings::Settings;
use dots_core::skill_creator::{check_draft, DraftRequest, DraftSummary};
use dots_core::vault::org::{JoinPreview, OrgEntry, OrgSummary, SourceCreds};
use dots_core::vault::{EntryKind, EnvEntry, GrantStatus};
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
pub fn chatgpt_account(
    auth: State<'_, dots_core::chatgpt::ChatGptAuth>,
) -> dots_core::chatgpt::ChatGptStatus {
    auth.status()
}

#[tauri::command]
pub async fn chatgpt_sign_in(
    app: AppHandle,
    auth: State<'_, dots_core::chatgpt::ChatGptAuth>,
) -> Res<dots_core::chatgpt::ChatGptStatus> {
    auth.sign_in(|url| {
        app.opener().open_url(url, None::<&str>).map_err(|_| {
            dots_core::Error::Other(
                "Could not open your browser. Check your default browser and try again.".into(),
            )
        })
    })
    .await
    .map_err(|error| match error {
        dots_core::Error::Conflict(message) if message == "ChatGPT sign-in cancelled." => {
            CmdError::cancelled(message)
        }
        other => other.into(),
    })
}

#[tauri::command]
pub async fn chatgpt_sign_out(
    auth: State<'_, dots_core::chatgpt::ChatGptAuth>,
) -> Res<dots_core::chatgpt::ChatGptStatus> {
    Ok(auth.sign_out().await)
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
    let saved = confirm::save_settings_confirmed(&rt, settings, |title, message| {
        confirm::ask(&app, title, message)
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

/// Settings › Updates: the version, the last check, a ready update, Go back's version and the
/// idle-install switch. Answers after a failed start too.
#[tauri::command]
pub async fn update_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res<update_service::UpdateStatus> {
    update_service::status(&app, state.rt().ok().as_ref()).await
}

/// "Check for updates": checks now and says what it found.
#[tauri::command]
pub async fn update_check_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Res<update_service::CheckOutcome> {
    Ok(update_service::check_now(&app, &state.rt()?).await)
}

/// "What's new": a version's notes from its release, `None` when it has none.
#[tauri::command]
pub async fn update_release_notes(version: String) -> Res<Option<update_service::ReleaseNotes>> {
    update_service::release_notes(&version).await
}

/// "Repair Dots": reinstalls this version, waiting for working runs.
#[tauri::command]
pub async fn update_repair(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::repair(&app, &state.rt()?).await
}

/// "Go back": installs the version before the last update with that update's backup. Works on
/// the fatal page too.
#[tauri::command]
pub async fn update_go_back(app: AppHandle, state: State<'_, AppState>) -> Res<UpdateNote> {
    update_service::go_back(&app, state.rt().ok()).await
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

/// The status of each of the dot's grants in its scope, for its editor's Variables: whether
/// it resolves now, from where, and why not. No value is in it, and no org is fetched.
#[tauri::command]
pub async fn dot_variables(state: State<'_, AppState>, dot_id: String) -> Res<Vec<GrantStatus>> {
    Ok(state.rt()?.dot_variables(&dot_id).await?)
}

/// `org_id`, the UI's `orgId`: the org the new dot belongs to, `None` for Personal.
#[tauri::command]
pub async fn create_dot(
    state: State<'_, AppState>,
    spec: DotSpec,
    org_id: Option<String>,
) -> Res<Dot> {
    Ok(state.rt()?.create_dot(spec, org_id).await?)
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

/// Checks the server in `scope`, an org id (none for Personal): "Check now" and "Check as". A
/// server whose URL reaches an origin not confirmed in that scope asks in a native dialog first
/// (`confirm::check_confirmed`); a refusal answers `cancelled` and nothing is sent.
#[tauri::command]
pub async fn library_check(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    scope: Option<String>,
) -> Res<McpCatalog> {
    let rt = state.rt()?;
    confirm::check_confirmed(&rt, &id, &confirm::scope_of(scope), |message| {
        confirm::ask(&app, confirm::library_title(&message), message)
    })
    .await
}

/// Confirms, in a native dialog, the origin the http server `server_id` reaches in `scope`, and
/// records it: a run in that scope then starts the server there. Nothing is asked when it is
/// confirmed already; a refusal answers `cancelled`. A plugin's own server takes it too.
#[tauri::command]
pub async fn confirm_origin(
    app: AppHandle,
    state: State<'_, AppState>,
    server_id: String,
    scope: Option<String>,
) -> Res<()> {
    let rt = state.rt()?;
    confirm::origin_confirmed(&rt, &server_id, &confirm::scope_of(scope), |message| {
        confirm::ask(&app, confirm::library_title(&message), message)
    })
    .await
}

/// Accepts the list last seen in `scope` (none for Personal). Refused with kind `conflict` when
/// the fingerprint is no longer the one last seen there.
#[tauri::command]
pub async fn library_accept(
    state: State<'_, AppState>,
    id: String,
    fingerprint: String,
    scope: Option<String>,
) -> Res<McpCatalog> {
    Ok(state
        .rt()?
        .library()
        .accept(&id, &confirm::scope_of(scope), &fingerprint)
        .await?)
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

/// Which of the names Personal has. Never a value.
#[tauri::command]
pub async fn environment_variables(
    state: State<'_, AppState>,
    names: Vec<String>,
) -> Res<Vec<EnvVariable>> {
    Ok(state.rt()?.library().environment_variables(&names).await?)
}

/// The personal variables of the Env tab. A secret's value is never in the answer.
#[tauri::command]
pub async fn env_list(state: State<'_, AppState>) -> Res<Vec<EnvEntry>> {
    Ok(state.rt()?.vault().list().await?)
}

/// Adds a personal variable; a secret's value goes to Credential Manager.
#[tauri::command]
pub async fn env_add(
    state: State<'_, AppState>,
    name: String,
    kind: EntryKind,
    value: String,
) -> Res<EnvEntry> {
    Ok(state.rt()?.vault().add(&name, kind, &value).await?)
}

/// Edits a plain value, or replaces a secret's without showing it.
#[tauri::command]
pub async fn env_set(state: State<'_, AppState>, name: String, value: String) -> Res<EnvEntry> {
    Ok(state.rt()?.vault().set_value(&name, &value).await?)
}

#[tauri::command]
pub async fn env_remove(state: State<'_, AppState>, name: String) -> Res<()> {
    Ok(state.rt()?.vault().remove(&name).await?)
}

/// The names Library servers read that Dots' Windows environment sets and Personal lacks,
/// until the offer is answered. Never a value.
#[tauri::command]
pub async fn env_import_candidates(state: State<'_, AppState>) -> Res<Vec<String>> {
    Ok(state.rt()?.vault().import_candidates().await?)
}

/// Copies the names from Dots' Windows environment into Personal as secrets; answers the offer.
#[tauri::command]
pub async fn env_import(state: State<'_, AppState>, names: Vec<String>) -> Res<Vec<String>> {
    Ok(state.rt()?.vault().import(&names).await?)
}

#[tauri::command]
pub async fn env_import_dismiss(state: State<'_, AppState>) -> Res<()> {
    Ok(state.rt()?.vault().dismiss_import().await?)
}

/// The update backups made before the vault, by file name, until the offer is answered.
#[tauri::command]
pub async fn env_old_backups(state: State<'_, AppState>) -> Res<Vec<String>> {
    Ok(state.rt()?.old_backups().await?)
}

/// Deletes them when `delete`, and answers the offer. Returns the file names that could not be
/// deleted: then the offer stays.
#[tauri::command]
pub async fn env_old_backups_answer(state: State<'_, AppState>, delete: bool) -> Res<Vec<String>> {
    Ok(state.rt()?.answer_old_backups(delete).await?)
}

/// The orgs joined on this PC, for Settings → Organisations (env vault spec §9). Never a value.
#[tauri::command]
pub async fn org_list(state: State<'_, AppState>) -> Res<Vec<OrgSummary>> {
    Ok(state.rt()?.orgs().summaries().await)
}

/// What the join code `code` (its text, or a `.dotsorg` file's) would join, or change for an
/// org joined already. Nothing is fetched.
#[tauri::command]
pub fn org_preview(state: State<'_, AppState>, code: String) -> Res<JoinPreview> {
    Ok(state.rt()?.preview_org(&code)?)
}

/// This PC's age public key for the SOPS org `org_id`, made on the first call, for the person
/// to send to their admin. Never the private key.
#[tauri::command]
pub async fn org_sops_key(state: State<'_, AppState>, org_id: String) -> Res<String> {
    Ok(state.rt()?.orgs().sops_identity(&org_id).await?)
}

/// Joins the org of `code` with the person's sign-in details `creds` (`SourceCreds`, tagged
/// `kind`), none for a SOPS org whose key is made. Answers the org; the details are never
/// answered or logged.
#[tauri::command]
pub async fn org_join(
    state: State<'_, AppState>,
    code: String,
    creds: Option<serde_json::Value>,
) -> Res<OrgSummary> {
    let creds = creds_of(creds)?;
    Ok(state.rt()?.join_org(&code, creds).await?)
}

/// Replaces the org's settings with those of a new join code, with new sign-in details or the
/// saved ones. A code that brings a new policy key asks in a native dialog first
/// (`confirm::update_org_confirmed`); a refusal answers `cancelled` and nothing changes.
#[tauri::command]
pub async fn org_update(
    app: AppHandle,
    state: State<'_, AppState>,
    org_id: String,
    code: String,
    creds: Option<serde_json::Value>,
) -> Res<OrgSummary> {
    let creds = creds_of(creds)?;
    let rt = state.rt()?;
    confirm::update_org_confirmed(&rt, &org_id, &code, creds, |message| {
        confirm::ask(&app, confirm::KEY_TITLE, message)
    })
    .await
}

/// Fetches the org now (**Refresh**). A failed fetch is not an error: the org's status says it.
#[tauri::command]
pub async fn org_refresh(state: State<'_, AppState>, org_id: String) -> Res<OrgSummary> {
    Ok(state.rt()?.refresh_org(&org_id).await?)
}

/// Leaves the org (**Remove**): its offline copy, sign-in details and row go; its dots stay,
/// "org removed". The UI confirms first.
#[tauri::command]
pub async fn org_remove(state: State<'_, AppState>, org_id: String) -> Res<()> {
    Ok(state.rt()?.remove_org(&org_id).await?)
}

/// The org's entries for the Env tab: plain ones with their values, secrets by name only.
#[tauri::command]
pub async fn org_entries(state: State<'_, AppState>, org_id: String) -> Res<Vec<OrgEntry>> {
    Ok(state.rt()?.orgs().entries(&org_id).await?)
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

/// "Open the browser" (browser spec section 7): the dot's Edge, for the person to log in ahead
/// of time. It stays open until they close it.
#[tauri::command]
pub async fn browser_open(state: State<'_, AppState>, dot_id: String) -> Res<()> {
    Ok(state.rt()?.browser_open(&dot_id).await?)
}

/// Where Microsoft Edge and Node.js (`npx`) were found, for Settings.
#[tauri::command]
pub async fn browser_status(state: State<'_, AppState>) -> Res<BrowserStatus> {
    Ok(state.rt()?.browser_status().await)
}

/// A screenshot of a run's browser as a `data:` URL: only a file Playwright named in the run's
/// `downloads/` folder (`page-<time>.png`), so the UI cannot read any other file.
#[tauri::command]
pub async fn browser_screenshot(
    state: State<'_, AppState>,
    run_id: String,
    name: String,
) -> Res<String> {
    Ok(state.rt()?.browser_screenshot(&run_id, &name).await?)
}

/// The person looked at the dot's conversation (needs-you spec section 4.1): its `unread` drops
/// to what needs them.
#[tauri::command]
pub async fn mark_dot_seen(state: State<'_, AppState>, dot_id: String) -> Res<()> {
    Ok(state.rt()?.mark_dot_seen(&dot_id).await?)
}

/// The dot whose conversation is open in the focused window, or none: nothing notifies about
/// it, and what it does while viewed is not new.
#[tauri::command]
pub async fn set_viewing(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<()> {
    state.rt()?;
    // Held while the window is really in front (`viewing`): a minimized window is not.
    crate::viewing::report(&app, dot_id);
    Ok(())
}

/// The sum of every dot's `unread`, for the Home link of the rail.
#[tauri::command]
pub async fn unread_total(state: State<'_, AppState>) -> Res<u32> {
    Ok(state.rt()?.unread_total().await?)
}

/// What the dot remembers, pinned first, then newest (dot memory spec section 4).
#[tauri::command]
pub async fn memories_list(state: State<'_, AppState>, dot_id: String) -> Res<Vec<Memory>> {
    Ok(state.rt()?.memories_list(&dot_id).await?)
}

/// The person's memory of the dot. Refused, with the core's sentence, when the text is blank,
/// longer than 500 characters, a duplicate, past the 200 or a secret.
#[tauri::command]
pub async fn memory_add(state: State<'_, AppState>, dot_id: String, text: String) -> Res<Memory> {
    Ok(state.rt()?.memory_add(&dot_id, &text).await?)
}

/// Changes any memory's text and/or its pin; the text rules of `memory_add` apply.
#[tauri::command]
pub async fn memory_update(
    state: State<'_, AppState>,
    id: String,
    text: Option<String>,
    pinned: Option<bool>,
) -> Res<Memory> {
    Ok(state
        .rt()?
        .memory_update(&id, text.as_deref(), pinned)
        .await?)
}

#[tauri::command]
pub async fn memory_delete(state: State<'_, AppState>, id: String) -> Res<()> {
    Ok(state.rt()?.memory_delete(&id).await?)
}

/// The installed apps, with the dots that use each and the error that keeps one out of runs
/// (app library spec section 10).
#[tauri::command]
pub async fn apps_list(state: State<'_, AppState>) -> Res<Vec<AppRow>> {
    Ok(state.rt()?.apps().rows().await?)
}

/// A call from the app's screen, as the person (app library spec section 5).
#[tauri::command]
pub async fn app_call(
    state: State<'_, AppState>,
    name: String,
    tool: String,
    input: serde_json::Value,
) -> Res<serde_json::Value> {
    Ok(state.rt()?.apps().screen_call(&name, &tool, input).await?)
}

/// The active version's files: the frame's `ui/` and the Code view.
#[tauri::command]
pub async fn app_files(state: State<'_, AppState>, name: String) -> Res<Vec<AppFile>> {
    Ok(state.rt()?.apps().files(&name).await?)
}

/// The newest logged calls of the app (at most 500), newest first.
#[tauri::command]
pub async fn app_calls(state: State<'_, AppState>, name: String, limit: u32) -> Res<Vec<AppCall>> {
    Ok(state.rt()?.apps().calls(&name, limit).await?)
}

#[tauri::command]
pub async fn app_set_every_dot(
    state: State<'_, AppState>,
    name: String,
    on: bool,
) -> Res<LibraryItem> {
    Ok(state.rt()?.apps().set_every_dot(&name, on).await?)
}

/// "Can start dots" (app library spec section 7): refused for an app that does not ask for it.
#[tauri::command]
pub async fn app_set_can_start_dots(
    state: State<'_, AppState>,
    name: String,
    on: bool,
) -> Res<LibraryItem> {
    Ok(state.rt()?.apps().set_can_start_dots(&name, on).await?)
}

/// Back to the version before, offered while the update after it ran no migrations.
#[tauri::command]
pub async fn app_roll_back(state: State<'_, AppState>, name: String) -> Res<LibraryItem> {
    Ok(state.rt()?.apps().roll_back(&name).await?)
}

/// Removes the app's code, and its data too with `delete_data`; `confirm` is the person's yes
/// to "dots use it".
#[tauri::command]
pub async fn app_remove(
    state: State<'_, AppState>,
    name: String,
    confirm: bool,
    delete_data: bool,
) -> Res<()> {
    Ok(state
        .rt()?
        .apps()
        .remove(&name, confirm, delete_data)
        .await?)
}

/// The apps dots proposed, of one dot or of all, newest first (app library spec section 6.2).
#[tauri::command]
pub async fn app_proposals_list(
    state: State<'_, AppState>,
    dot_id: Option<String>,
) -> Res<Vec<AppProposal>> {
    Ok(state
        .rt()?
        .store()
        .list_app_proposals(dot_id.as_deref())
        .await?)
}

/// A waiting proposal's files, for the card's Code tab.
#[tauri::command]
pub async fn app_proposal_files(state: State<'_, AppState>, id: String) -> Res<Vec<AppFile>> {
    Ok(state.rt()?.apps().proposal_files(&id).await?)
}

/// The person's answer on the card: add (or update), ask for changes, or discard.
#[tauri::command]
pub async fn app_proposal_decide(
    state: State<'_, AppState>,
    id: String,
    decision: Decision,
) -> Res<AppProposal> {
    Ok(state.rt()?.decide_app_proposal(&id, decision).await?.0)
}

/// The dot's watchers, oldest first (watchers spec section 8).
#[tauri::command]
pub async fn watchers_list(state: State<'_, AppState>, dot_id: String) -> Res<Vec<Watcher>> {
    Ok(state.rt()?.watchers_list(&dot_id).await?)
}

/// Adds a watcher to the dot; the core checks its name and config.
#[tauri::command]
pub async fn watcher_add(
    state: State<'_, AppState>,
    dot_id: String,
    name: String,
    config: WatcherConfig,
) -> Res<Watcher> {
    Ok(state.rt()?.watcher_add(&dot_id, &name, config).await?)
}

/// Changes a watcher's name and config; a new config starts it again from now.
#[tauri::command]
pub async fn watcher_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    config: WatcherConfig,
) -> Res<Watcher> {
    Ok(state.rt()?.watcher_update(&id, &name, config).await?)
}

#[tauri::command]
pub async fn watcher_delete(state: State<'_, AppState>, id: String) -> Res<()> {
    Ok(state.rt()?.watcher_delete(&id).await?)
}

/// The person's pause switch of a watcher.
#[tauri::command]
pub async fn watcher_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Res<Watcher> {
    Ok(state.rt()?.watcher_set_enabled(&id, enabled).await?)
}

/// Claude's limit readings and what is held (usage and limits spec section 6).
#[tauri::command]
pub async fn limits_get(state: State<'_, AppState>) -> Res<dots_core::limits::LimitsState> {
    Ok(state.rt()?.limits_get().await?)
}

/// The Usage page's table for the period.
#[tauri::command]
pub async fn usage_summary(
    state: State<'_, AppState>,
    period: dots_core::model::UsagePeriod,
) -> Res<dots_core::model::UsageSummary> {
    Ok(state.rt()?.usage_summary(period).await?)
}

/// **Run anyway**: releases a held background run from the hold.
#[tauri::command]
pub async fn run_release_hold(state: State<'_, AppState>, run_id: String) -> Res<Run> {
    Ok(state.rt()?.run_release_hold(&run_id).await?)
}

/// Resumes a watcher Dots paused (too many runs, or an error).
#[tauri::command]
pub async fn watcher_resume(state: State<'_, AppState>, id: String) -> Res<Watcher> {
    Ok(state.rt()?.watcher_resume(&id).await?)
}

/// The sign-in details a command was given, read as `SourceCreds`. The words of a failure name
/// no part of them: serde's messages can quote the text they fail on.
fn creds_of(creds: Option<serde_json::Value>) -> Res<Option<SourceCreds>> {
    creds
        .map(|value| {
            serde_json::from_value(value).map_err(|_| {
                CmdError::from(dots_core::Error::Invalid(
                    "these sign-in details are not ones Dots reads: give a token, a username \
                     and password, a machine identity's client id and secret, or an age key"
                        .into(),
                ))
            })
        })
        .transpose()
}
