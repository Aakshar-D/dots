//! Native confirmations for changes the webview must not make on its own (spec §12).

use std::future::Future;

use dots_core::library::{LibraryConfig, LibraryItem, McpServerConfig};
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
/// - A stdio MCP server whose `command`, `args`, `cwd` or `env` differ from the stored ones (a
///   new server differs): Dots would start that program. `env` is compared as stored, the
///   `${VAR}` text, and the question names its keys, never values.
/// - A plugin that starts stdio servers: `plugin_starts` holds their command lines, which the
///   caller read from the plugin's `.mcp.json` when the plugin is new or its path changed (and
///   left empty otherwise). It is combined with
/// - a plugin whose `run_hooks` goes from off to on (a new plugin counts as off): its hooks run
///   commands without approval.
///
/// Anything else, a `headers` change included, needs none. This function reads no disk.
pub fn library_confirmation(
    stored: Option<&LibraryConfig>,
    next: &LibraryConfig,
    plugin_starts: &[String],
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
                .map(|d| format!(" in {d}"))
                .unwrap_or_default();
            let keys = if env.is_empty() {
                String::new()
            } else {
                let keys: Vec<&str> = env.keys().map(String::as_str).collect();
                format!(" with env {}", keys.join(", "))
            };
            Some(format!(
                "Dots will run: {}{place}{keys}. Run it?\n\nAllow this only for a program you trust.",
                command_line(command, args)
            ))
        }
        LibraryConfig::Plugin(p) => {
            let was_on = matches!(stored, Some(LibraryConfig::Plugin(old)) if old.run_hooks);
            let hooks = p.run_hooks && !was_on;
            match (plugin_starts.is_empty(), hooks) {
                (true, false) => None,
                (true, true) => Some(
                    "This plugin's hooks run commands on their own, without approval, and can \
                     approve tool calls. Run them?"
                        .to_string(),
                ),
                (false, hooks) => {
                    let mut text = format!("This plugin starts: {}.", plugin_starts.join("; "));
                    if hooks {
                        text.push_str(
                            " Its hooks also run commands on their own, without approval, and \
                             can approve tool calls.",
                        );
                    }
                    text.push_str(" Add it?");
                    Some(text)
                }
            }
        }
        _ => None,
    }
}

fn command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command lines of the stdio servers in the plugin folder `config` names, for the question
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
            McpServerConfig::Http { .. } => None,
        })
        .collect()
}

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
        .map(|e| e.item.config);
    // An id the Library does not have is the core's `not_found`, before any question.
    if stored.is_some() {
        confirm_library(stored.as_ref(), &config, ask).await?;
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
        assert!(asked.starts_with("Dots will run: npx -y srv in C:/d. Run it?"));
        // Without a folder the question names none.
        assert!(q(None, &stdio("srv", &[], None))
            .unwrap()
            .starts_with("Dots will run: srv. Run it?"));
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
            asked.starts_with("Dots will run: npx srv with env NODE_OPTIONS, PATH. Run it?"),
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
            .starts_with("Dots will run: npx srv in C:/d with env A. Run it?"));
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
        assert_eq!(plugin_starts(None, &config), ["node a.js"]);
        // Same path: not read, not asked again.
        assert!(plugin_starts(Some(&config), &config).is_empty());
        // Another path: read again.
        let elsewhere = at(std::path::Path::new("C:/no/such/plugin"));
        assert_eq!(plugin_starts(Some(&elsewhere), &config), ["node a.js"]);
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
        assert!(asked[0].starts_with("Dots will run: dots-no-such-mcp-server a. Run it?"));

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
