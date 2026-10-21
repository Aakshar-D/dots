//! Follow-ups (check-ins spec sections 3 and 6): runs a dot schedules for itself. A follow-up
//! continues the thread of the run that scheduled it, with the dot's note, once it is due.
//! This module holds the limits, checked when the tool is called, and the follow-up loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Mutex, Notify};

use crate::checkins::When;
use crate::events::{Bus, RuntimeEvent};
use crate::model::{FollowUp, Run};
use crate::scheduler::Clock;
use crate::store::Store;
use crate::{Error, Result};

/// A follow-up is due at least this long after it is scheduled.
pub const MIN_AHEAD: TimeDelta = TimeDelta::minutes(5);
/// A follow-up is due at most this long after it is scheduled.
pub const MAX_AHEAD: TimeDelta = TimeDelta::days(30);
/// The most pending follow-ups a dot may have.
pub const MAX_PENDING: usize = 3;
/// A follow-up that fires later than this after it was due says how late it is.
pub const LATE_AFTER: TimeDelta = TimeDelta::minutes(15);
/// How long after each failed start the next try waits. The start after the last of them is
/// the last: when it fails too, the follow-up is skipped.
pub const RETRY_DELAYS: [TimeDelta; 3] = [
    TimeDelta::minutes(1),
    TimeDelta::minutes(5),
    TimeDelta::minutes(30),
];

pub const TOO_SOON: &str = "A follow-up must be at least 5 minutes away.";
pub const TOO_FAR: &str = "A follow-up must be at most 30 days away.";
pub const TOO_MANY: &str =
    "This dot already has 3 pending follow-ups; that is the most it may have.";

/// When a follow-up asked for at `now` is due, or the limit it breaks (section 3, in order:
/// `MIN_AHEAD`, then `MAX_AHEAD`; `MAX_PENDING` is checked when it is stored).
pub fn due_at(when: When, now: DateTime<Utc>) -> std::result::Result<DateTime<Utc>, &'static str> {
    let ahead = match when {
        When::At(t) => t - now,
        When::In(d) => d,
    };
    if ahead < MIN_AHEAD {
        return Err(TOO_SOON);
    }
    if ahead > MAX_AHEAD {
        return Err(TOO_FAR);
    }
    Ok(now + ahead)
}

/// A time the store wrote (`util::timestamp`).
pub fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Invalid(format!("stored time {s:?}: {e}")))
}

/// The follow-ups of every dot, and the loop that fires them (spec section 6). Held by the
/// runtime, by `CheckIns`, which schedules them, and by the loop.
pub struct FollowUps {
    store: Store,
    bus: Bus,
    clock: Arc<dyn Clock>,
    /// Held from reading a thread's end until the run that continues it is stored.
    /// `Runtime::chat` and `CheckIns::reply` take it too, so no two of them give one run two
    /// children.
    thread_lock: Arc<Mutex<()>>,
    wake: Notify,
    paused: AtomicBool,
}

impl FollowUps {
    pub fn new(store: Store, bus: Bus, clock: Arc<dyn Clock>) -> Arc<FollowUps> {
        Arc::new(FollowUps {
            store,
            bus,
            clock,
            thread_lock: Arc::new(Mutex::new(())),
            wake: Notify::new(),
            paused: AtomicBool::new(false),
        })
    }

    /// The lock of every writer that continues a thread.
    pub fn thread_lock(&self) -> Arc<Mutex<()>> {
        self.thread_lock.clone()
    }

    /// Wakes the loop for a pass: a follow-up was added or cancelled, or a dot changed.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// While schedules are paused no follow-up fires; a due one fires once the pause ends.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        self.wake();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Schedules a follow-up of `run`'s dot that continues `run`'s thread (spec section 3).
    /// The limits are checked in order against the loop's clock; a broken one is `Invalid`
    /// with its sentence.
    pub async fn schedule(&self, run: &Run, note: &str, when: When) -> Result<FollowUp> {
        let now = self.clock.now();
        let due = due_at(when, now).map_err(|limit| Error::Invalid(limit.into()))?;
        let Some(row) = self
            .store
            .add_follow_up(&run.dot_id, &run.id, due, note, MAX_PENDING, now)
            .await?
        else {
            return Err(Error::Invalid(TOO_MANY.into()));
        };
        self.updated(&row.dot_id);
        self.wake();
        Ok(row)
    }

    /// The pending follow-ups, of one dot or of all, soonest first.
    pub async fn list(&self, dot_id: Option<&str>) -> Result<Vec<FollowUp>> {
        self.store.list_pending_follow_ups(dot_id).await
    }

    /// Cancels a pending follow-up (the person's; dots have no cancel tool). `Conflict` once
    /// it fired, was skipped or was cancelled.
    pub async fn cancel(&self, id: &str) -> Result<FollowUp> {
        let row = self.store.cancel_follow_up(id).await?;
        self.updated(&row.dot_id);
        self.wake();
        Ok(row)
    }

    fn updated(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::FollowUpsUpdated {
            dot_id: dot_id.to_string(),
        });
    }
}
