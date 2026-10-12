//! Native confirmations for changes the webview must not make on its own (spec §12).

use std::future::Future;

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
        assert_eq!(err.kind, "invalid");
        assert_eq!(err.message, "port must be between 1 and 65535");
        assert_eq!(asked, [] as [String; 0]);
        assert_eq!(rt.settings().await, before);
        rt.shutdown();
    }
}
