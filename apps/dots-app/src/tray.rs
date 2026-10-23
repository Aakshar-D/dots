//! The tray icon: Open, Needs you, Inbox (n), Pause all schedules, Quit (spec §7; needs-you
//! spec section 4.3).

use std::sync::{Arc, Mutex, PoisonError};

use dots_core::model::DotOverview;
use dots_core::Runtime;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
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

/// The most dots Needs you lists; the Inbox has the rest.
pub const NEEDS_YOU_SHOWN: usize = 10;
/// The one item of Needs you when no dot waits for the person.
pub const NOTHING_WAITING: &str = "Nothing waiting";
/// A dot's item in Needs you has this menu id, followed by the dot's id.
const NEEDS_PREFIX: &str = "needs:";
const NOTHING_ID: &str = "needs-none";

/// The items of Needs you, as (menu id, label): each dot whose needs-you is above 0, as
/// "<dot> (<n>)", most waiting first, then by name, `NEEDS_YOU_SHOWN` at most.
pub fn needs_you_items(rows: &[DotOverview]) -> Vec<(String, String)> {
    let mut waiting: Vec<&DotOverview> = rows.iter().filter(|r| r.needs_you > 0).collect();
    waiting.sort_by(|a, b| {
        b.needs_you
            .cmp(&a.needs_you)
            .then_with(|| a.dot.spec.name.cmp(&b.dot.spec.name))
    });
    waiting
        .into_iter()
        .take(NEEDS_YOU_SHOWN)
        .map(|r| {
            (
                format!("{NEEDS_PREFIX}{}", r.dot.id),
                format!("{} ({})", r.dot.spec.name, r.needs_you),
            )
        })
        .collect()
}

/// The route an item of Needs you opens: its dot's conversation. `None` for another item.
pub fn needs_you_route(menu_id: &str) -> Option<String> {
    menu_id
        .strip_prefix(NEEDS_PREFIX)
        .map(|dot_id| format!("/home/{dot_id}"))
}

pub struct Tray {
    icon: TrayIcon,
    inbox: MenuItem<Wry>,
    pause: CheckMenuItem<Wry>,
    needs: Submenu<Wry>,
    /// The items Needs you shows now; empty while it shows "Nothing waiting".
    shown: Mutex<Vec<(String, String)>>,
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
        let nothing = MenuItem::with_id(app, NOTHING_ID, NOTHING_WAITING, false, None::<&str>)?;
        let needs = Submenu::with_id_and_items(app, "needs", "Needs you", true, &[&nothing])?;
        let menu = Menu::with_items(app, &[&open, &needs, &inbox, &pause, &separator, &quit])?;
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
        Ok(Tray {
            icon,
            inbox,
            pause,
            needs,
            shown: Mutex::new(Vec::new()),
        })
    }

    pub fn set_count(&self, count: usize) {
        let _ = self.icon.set_tooltip(Some(tooltip(count)));
        let _ = self.inbox.set_text(inbox_label(count));
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.pause.set_checked(paused);
    }

    /// Shows `items` (`needs_you_items`) in Needs you, or "Nothing waiting" for none. The
    /// submenu is rebuilt only when they changed.
    pub fn set_needs(&self, app: &AppHandle, items: Vec<(String, String)>) {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if *shown == items {
            return;
        }
        match self.fill_needs(app, &items) {
            Ok(()) => *shown = items,
            Err(e) => tracing::warn!("updating the tray's Needs you failed: {e}"),
        }
    }

    fn fill_needs(&self, app: &AppHandle, items: &[(String, String)]) -> tauri::Result<()> {
        for item in self.needs.items()? {
            self.needs.remove(&item)?;
        }
        if items.is_empty() {
            let nothing = MenuItem::with_id(app, NOTHING_ID, NOTHING_WAITING, false, None::<&str>)?;
            self.needs.append(&nothing)?;
        }
        for (id, label) in items {
            let item = MenuItem::with_id(app, id.as_str(), label, true, None::<&str>)?;
            self.needs.append(&item)?;
        }
        Ok(())
    }
}

/// Recomputes the Inbox count and the dots of Needs you shown by the tray.
pub async fn refresh_count(app: &AppHandle, rt: &Runtime) {
    match rt.inbox_count().await {
        Ok(n) => {
            if let Some(tray) = app.try_state::<Tray>() {
                tray.set_count(n);
            }
        }
        Err(e) => tracing::warn!("counting the inbox failed: {e}"),
    }
    match rt.dot_overview().await {
        Ok(rows) => {
            if let Some(tray) = app.try_state::<Tray>() {
                tray.set_needs(app, needs_you_items(&rows));
            }
        }
        Err(e) => tracing::warn!("listing the dots that need you failed: {e}"),
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
        id => {
            if let Some(route) = needs_you_route(id) {
                crate::show_main(app);
                let _ = app.emit(NAVIGATE_EVENT, route);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::affects_inbox;
    use dots_core::events::RuntimeEvent;
    use dots_core::model::{Dot, DotOverview, DotSpec};

    #[test]
    fn tray_texts_show_the_inbox_count() {
        assert_eq!(tooltip(0), "Dots");
        assert_eq!(tooltip(3), "Dots: 3 to review");
        assert_eq!(inbox_label(0), "Inbox");
        assert_eq!(inbox_label(12), "Inbox (12)");
    }

    /// A dot named `name` with `needs_you` things waiting for the person.
    fn row(name: &str, needs_you: u32) -> DotOverview {
        DotOverview {
            dot: Dot {
                id: format!("id-{name}"),
                has_webhook_token: true,
                created_at: "t".into(),
                updated_at: "t".into(),
                seen_at: None,
                spec: DotSpec::new(name, "Do it.", "C:/work"),
            },
            active_run: None,
            last_run: None,
            schedule_state: None,
            needs_you,
            unread: needs_you,
        }
    }

    #[test]
    fn needs_you_lists_the_dots_waiting_most_first_then_by_name() {
        let rows = [
            row("zeta", 1),
            row("idle", 0),
            row("alpha", 1),
            row("busy", 3),
        ];
        let items = needs_you_items(&rows);
        assert_eq!(
            items,
            [
                ("needs:id-busy".to_string(), "busy (3)".to_string()),
                ("needs:id-alpha".to_string(), "alpha (1)".to_string()),
                ("needs:id-zeta".to_string(), "zeta (1)".to_string()),
            ]
        );
        // Each opens its dot's conversation.
        assert_eq!(
            needs_you_route(&items[0].0).as_deref(),
            Some("/home/id-busy")
        );
        assert_eq!(needs_you_route("inbox"), None);
    }

    #[test]
    fn needs_you_lists_10_dots_at_most_and_nothing_when_none_waits() {
        let rows: Vec<DotOverview> = (0..12).map(|n| row(&format!("dot-{n:02}"), 1)).collect();
        let items = needs_you_items(&rows);
        assert_eq!(items.len(), NEEDS_YOU_SHOWN);
        assert_eq!(items[9].1, "dot-09 (1)");
        assert!(needs_you_items(&[row("idle", 0)]).is_empty());
        assert_eq!(NOTHING_WAITING, "Nothing waiting");
        // The tray is refreshed after a dot was seen too.
        assert!(affects_inbox(&RuntimeEvent::DotSeen {
            dot_id: "id-busy".into()
        }));
    }
}
