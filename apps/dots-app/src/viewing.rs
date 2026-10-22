//! Which dot the person is viewing (needs-you spec sections 2 and 5): the dot the webview
//! reports, while the window is in front. A minimized window can keep the focus on Windows, and
//! its webview then reports no change, so the app checks the window itself.

use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager, WindowEvent};

use crate::state::{runtime_of, AppState};

/// The dot the webview last reported as open on the home, while it said the window was in
/// front; `None` for none.
#[derive(Default)]
pub struct Reported(Mutex<Option<String>>);

/// What the core is told is viewed: the reported dot, while the window is in front.
pub fn effective(reported: Option<&str>, in_front: bool) -> Option<String> {
    reported.filter(|_| in_front).map(str::to_string)
}

/// Whether a window event may have moved the window in or out of the front: a focus change, or
/// a resize (minimizing and restoring are resizes).
pub fn rechecks(event: &WindowEvent) -> bool {
    matches!(event, WindowEvent::Focused(_) | WindowEvent::Resized(_))
}

/// The webview reports the dot open on the home, or none; the core learns it once the window
/// is in front.
pub fn report(app: &AppHandle, dot_id: Option<String>) {
    if let Some(reported) = app.try_state::<Reported>() {
        *reported.0.lock().unwrap_or_else(PoisonError::into_inner) = dot_id;
    }
    apply(app);
}

/// Tells the core what is viewed now: the reported dot while the main window is visible, not
/// minimized and focused; else none.
pub fn apply(app: &AppHandle) {
    let Some(rt) = runtime_of(app.try_state::<AppState>().as_deref()) else {
        return;
    };
    let in_front = app.get_webview_window("main").is_some_and(|w| {
        w.is_visible().unwrap_or(false)
            && !w.is_minimized().unwrap_or(false)
            && w.is_focused().unwrap_or(false)
    });
    let reported = app
        .try_state::<Reported>()
        .and_then(|r| r.0.lock().unwrap_or_else(PoisonError::into_inner).clone());
    let before = rt.viewing();
    let now = effective(reported.as_deref(), in_front);
    rt.set_viewing(now.as_deref());
    // A dot coming back into view (the window restored, shown from the tray) is seen, though
    // its webview had nothing new to report.
    if let Some(dot_id) = newly_viewed(before.as_deref(), now.as_deref()).map(str::to_string) {
        tauri::async_runtime::spawn(async move {
            if let Err(e) = rt.mark_dot_seen(&dot_id).await {
                tracing::warn!(dot = %dot_id, "marking a dot seen failed: {e}");
            }
        });
    }
}

/// The dot that has just come into view, if one has.
pub fn newly_viewed<'a>(before: Option<&str>, now: Option<&'a str>) -> Option<&'a str> {
    now.filter(|dot| before != Some(*dot))
}

#[cfg(test)]
mod tests {
    use tauri::{PhysicalSize, WindowEvent};

    use super::*;

    #[test]
    fn the_reported_dot_is_viewed_only_while_the_window_is_in_front() {
        assert_eq!(effective(Some("dot-a"), true), Some("dot-a".to_string()));
        assert_eq!(effective(Some("dot-a"), false), None);
        assert_eq!(effective(None, true), None);
    }

    #[test]
    fn a_dot_that_comes_into_view_is_seen() {
        // Restored from minimized: the webview reports nothing, the app marks it seen.
        assert_eq!(newly_viewed(None, Some("dot-a")), Some("dot-a"));
        assert_eq!(newly_viewed(Some("dot-b"), Some("dot-a")), Some("dot-a"));
        // Still in view, or out of view: nothing to mark.
        assert_eq!(newly_viewed(Some("dot-a"), Some("dot-a")), None);
        assert_eq!(newly_viewed(Some("dot-a"), None), None);
    }

    #[test]
    fn focus_changes_and_resizes_check_the_window_again() {
        assert!(rechecks(&WindowEvent::Focused(false)));
        assert!(rechecks(&WindowEvent::Focused(true)));
        // Minimizing and restoring resize the window.
        assert!(rechecks(&WindowEvent::Resized(PhysicalSize::new(0, 0))));
        assert!(!rechecks(&WindowEvent::Destroyed));
    }
}
