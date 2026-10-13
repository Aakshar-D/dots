//! The tray icon: Open, Inbox (n), Pause all schedules, Quit (spec §7).

use std::sync::Arc;

use dots_core::Runtime;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::events::{NAVIGATE_EVENT, STATUS_EVENT};
use crate::state::{runtime_of, AppState};

pub fn tooltip(count: usize) -> String {
    match count {
        0 => "Dots".to_string(),
        n => format!("Dots: {n} to review"),
    }
}

pub fn inbox_label(count: usize) -> String {
    match count {
        0 => "Inbox".to_string(),
        n => format!("Inbox ({n})"),
    }
}

pub struct Tray {
    icon: TrayIcon,
    inbox: MenuItem<Wry>,
    pause: CheckMenuItem<Wry>,
}

impl Tray {
    pub fn build(app: &AppHandle, paused: bool) -> tauri::Result<Tray> {
        let open = MenuItem::with_id(app, "open", "Open Dots", true, None::<&str>)?;
        let inbox = MenuItem::with_id(app, "inbox", inbox_label(0), true, None::<&str>)?;
        let pause = CheckMenuItem::with_id(
            app,
            "pause",
            "Pause all schedules",
            true,
            paused,
            None::<&str>,
        )?;
        let quit = MenuItem::with_id(app, "quit", "Quit Dots", true, None::<&str>)?;
        let separator = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(app, &[&open, &inbox, &pause, &separator, &quit])?;
        let mut builder = TrayIconBuilder::with_id("main")
            .tooltip(tooltip(0))
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    crate::show_main(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        let icon = builder.build(app)?;
        Ok(Tray { icon, inbox, pause })
    }

    pub fn set_count(&self, count: usize) {
        let _ = self.icon.set_tooltip(Some(tooltip(count)));
        let _ = self.inbox.set_text(inbox_label(count));
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.pause.set_checked(paused);
    }
}

/// Recomputes the Inbox count shown by the tray.
pub async fn refresh_count(app: &AppHandle, rt: &Runtime) {
    match rt.inbox_count().await {
        Ok(n) => {
            if let Some(tray) = app.try_state::<Tray>() {
                tray.set_count(n);
            }
        }
        Err(e) => tracing::warn!("counting the inbox failed: {e}"),
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    let rt: Option<Arc<Runtime>> = runtime_of(app.try_state::<AppState>().as_deref());
    match id {
        "open" => crate::show_main(app),
        "inbox" => {
            crate::show_main(app);
            let _ = app.emit(NAVIGATE_EVENT, "/inbox");
        }
        "pause" => {
            let Some(rt) = rt else { return };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // Toggle from the runtime's state, not the menu's, so the two cannot disagree.
                let paused = !rt.status().await.schedules_paused;
                match rt.set_schedules_paused(paused).await {
                    Ok(status) => {
                        if let Some(tray) = app.try_state::<Tray>() {
                            tray.set_paused(status.schedules_paused);
                        }
                        let _ = app.emit(STATUS_EVENT, &status);
                    }
                    Err(e) => tracing::error!("pausing schedules failed: {e}"),
                }
            });
        }
        "quit" => {
            if let Some(rt) = rt {
                rt.shutdown();
            }
            app.exit(0);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_texts_show_the_inbox_count() {
        assert_eq!(tooltip(0), "Dots");
        assert_eq!(tooltip(3), "Dots: 3 to review");
        assert_eq!(inbox_label(0), "Inbox");
        assert_eq!(inbox_label(12), "Inbox (12)");
    }
}
