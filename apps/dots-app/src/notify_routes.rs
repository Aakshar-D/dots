//! Where a click on a notification goes (needs-you spec section 4.2). Each notification Dots
//! shows gets an id, and the app keeps the routes of the last `KEPT`; one handler, registered
//! with the notification plugin at start, brings the window forward on the route of the
//! notification clicked. Windows reports a click only while the toast is on screen: a click in
//! the Action Center opens Dots where it was.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::{ActionPerformed, NotificationExt};

use crate::events::NAVIGATE_EVENT;

/// How many notifications' routes are kept.
pub const KEPT: usize = 100;

/// The action the plugin reports for a click on the notification itself.
pub const TAP: &str = "tap";

/// The routes of the last `KEPT` notifications, by id, oldest first.
#[derive(Debug, Default)]
pub struct Routes {
    last: i32,
    kept: VecDeque<(i32, String)>,
}

impl Routes {
    /// Keeps `route` under a new id (1, 2, …, and 1 again after `i32::MAX`), forgetting the
    /// oldest beyond `KEPT`.
    pub fn add(&mut self, route: String) -> i32 {
        self.last = if self.last == i32::MAX {
            1
        } else {
            self.last + 1
        };
        if self.kept.len() == KEPT {
            self.kept.pop_front();
        }
        self.kept.push_back((self.last, route));
        self.last
    }

    /// The route of notification `id`, while it is one of the last `KEPT`.
    pub fn route(&self, id: i32) -> Option<&str> {
        self.kept
            .iter()
            .find(|(kept, _)| *kept == id)
            .map(|(_, route)| route.as_str())
    }
}

/// What an action on a notification asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Not a click on the notification itself.
    Ignore,
    /// The window, where it was: the notification is not one of the last `KEPT`.
    Show,
    /// The window, on this route.
    Open(String),
}

/// What the action `action_id` on the notification `id` asks for.
pub fn click(routes: &Routes, action_id: &str, id: Option<i32>) -> Click {
    if action_id != TAP {
        return Click::Ignore;
    }
    match id.and_then(|id| routes.route(id)) {
        Some(route) => Click::Open(route.to_string()),
        None => Click::Show,
    }
}

/// The routes, as the app manages them.
#[derive(Default)]
pub struct NotifyRoutes(Mutex<Routes>);

impl NotifyRoutes {
    fn routes(&self) -> MutexGuard<'_, Routes> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Keeps the route of a notification about to show, and gives its id; 0 before `listen`.
pub fn remember(app: &AppHandle, route: String) -> i32 {
    app.try_state::<NotifyRoutes>()
        .map_or(0, |routes| routes.routes().add(route))
}

/// Manages the routes and registers the handler of clicks. A refusal is logged: notifications
/// still show, and a click on one does nothing.
pub fn listen(app: &AppHandle) {
    app.manage(NotifyRoutes::default());
    let handle = app.clone();
    let registered = app
        .notification()
        .on_action(move |performed| on_action(&handle, performed));
    if let Err(e) = registered {
        tracing::warn!("a click on a notification will not open Dots: {e}");
    }
}

fn on_action(app: &AppHandle, performed: &ActionPerformed) {
    let id = performed.notification().map(|n| n.id());
    let asked = match app.try_state::<NotifyRoutes>() {
        Some(routes) => click(&routes.routes(), performed.action_id(), id),
        None => click(&Routes::default(), performed.action_id(), id),
    };
    match asked {
        Click::Ignore => {}
        Click::Show => crate::show_main(app),
        Click::Open(route) => {
            crate::show_main(app);
            if let Err(e) = app.emit(NAVIGATE_EVENT, route) {
                tracing::warn!("opening a notification's route failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_routes_of_the_last_100_notifications() {
        let mut routes = Routes::default();
        let ids: Vec<i32> = (0..150)
            .map(|n| routes.add(format!("/home/dot-{n}")))
            .collect();
        assert_eq!((ids[0], ids[149]), (1, 150));
        // The oldest 50 are forgotten, the newest 100 kept.
        assert_eq!(routes.route(ids[49]), None);
        assert_eq!(routes.route(ids[50]), Some("/home/dot-50"));
        assert_eq!(routes.route(ids[149]), Some("/home/dot-149"));
        assert_eq!(routes.kept.len(), KEPT);
    }

    #[test]
    fn ids_start_again_after_the_largest() {
        let mut routes = Routes {
            last: i32::MAX - 1,
            ..Routes::default()
        };
        assert_eq!(routes.add("/home/a".into()), i32::MAX);
        assert_eq!(routes.add("/home/b".into()), 1);
        assert_eq!(routes.route(1), Some("/home/b"));
    }

    #[test]
    fn a_click_opens_its_route_and_an_unknown_notification_shows_the_window() {
        let mut routes = Routes::default();
        let id = routes.add("/chat/dot-a/run-1".into());
        assert_eq!(
            click(&routes, TAP, Some(id)),
            Click::Open("/chat/dot-a/run-1".into())
        );
        assert_eq!(click(&routes, TAP, Some(id + 1)), Click::Show);
        // The platform could not say which notification it was.
        assert_eq!(click(&routes, TAP, None), Click::Show);
        // A dismissal, or a button (Dots offers none), is not a click on the notification.
        assert_eq!(click(&routes, "dismiss", Some(id)), Click::Ignore);
    }
}
