//! The flood cap (watchers spec section 5): at most `FLOOD_CAP` runs per watcher in a sliding
//! `FLOOD_WINDOW`. The event past it pauses the watcher; the person's resume clears the count.

use chrono::{DateTime, TimeDelta, Utc};

use crate::store::Store;
use crate::Result;

pub const FLOOD_CAP: u32 = 20;
pub const FLOOD_WINDOW: TimeDelta = TimeDelta::hours(1);

/// Takes one of the watcher's fires at `now`: false when the hour is full.
pub(crate) async fn take(store: &Store, watcher_id: &str, now: DateTime<Utc>) -> Result<bool> {
    store
        .take_fire(watcher_id, now, FLOOD_WINDOW, FLOOD_CAP)
        .await
}
