//! Native confirmations for changes the webview must not make on its own (spec §12).

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

/// Debug builds accept `DOTS_CONFIRM=yes` in place of the dialog, so a smoke run can change
/// the Claude program without a person at the screen. Release builds always ask.
pub const CONFIRM_ENV: &str = "DOTS_CONFIRM";

/// The program a settings change would make Dots run as the Claude CLI, when that needs the
/// user's confirmation: a non-empty `claude_path` that differs from the stored one. Clearing
/// the path (back to searching `PATH`) and saving it unchanged need none.
pub fn claude_program_to_confirm(current: Option<&str>, next: Option<&str>) -> Option<String> {
    let next = next.map(str::trim).filter(|p| !p.is_empty())?;
    (current.map(str::trim) != Some(next)).then(|| next.to_string())
}

pub fn claude_program_message(program: &str) -> String {
    format!(
        "Dots will run this program as the Claude CLI from the next start:\n\n{program}\n\n\
         Allow this only for a program you trust."
    )
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
        assert_eq!(
            ask(Some("C:/a.exe"), Some(" C:/b.exe ")),
            Some("C:/b.exe".into())
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
}
