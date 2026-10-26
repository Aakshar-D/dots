//! The org timers (env vault spec §6.4): each joined org is fetched every `refresh_minutes` of
//! its accepted policy, and once at start. There is one task per org. Its next fetch is
//! `refresh_minutes` after its last one, read from the policy that fetch left, so a policy that
//! changes the interval re-times the org from its next fetch on. A join starts the org's timer,
//! an Update or a **Refresh** re-times it from now, and a remove stops it. Every timer, and the
//! start-up fetch, stops with the runtime's shutdown token.
//!
//! The timers sleep on the runtime's `Clock`, at most `scheduler::MAX_SLEEP` at a time, as the
//! scheduler does: a timer does not follow the wall clock across a system sleep, so each one
//! looks at the clock again at least that often. After each fetch a timer publishes
//! `OrgUpdated`, so the screens read the org's status again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Duration, Utc};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::policy::DEFAULT_REFRESH_MINUTES;
use super::service::Orgs;
use crate::events::{Bus, RuntimeEvent};
use crate::scheduler::{Clock, MAX_SLEEP};
use crate::Error;

/// The timers of the joined orgs. Cheap to clone: every clone shares one set of timers.
#[derive(Clone)]
pub struct OrgTimers {
    inner: Arc<Inner>,
}

struct Inner {
    orgs: Orgs,
    clock: Arc<dyn Clock>,
    bus: Bus,
    shutdown: CancellationToken,
    timers: Mutex<HashMap<String, Timer>>,
    /// The start-up fetch of every org, while it runs.
    startup: Mutex<Option<JoinHandle<()>>>,
}

struct Timer {
    stop: CancellationToken,
    /// When the org is fetched next. Set by whoever moves it, then `moved` wakes the task, so
    /// the time holds from the call on, whenever the task runs.
    due: Due,
    moved: Arc<Notify>,
    task: JoinHandle<()>,
}

type Due = Arc<Mutex<DateTime<Utc>>>;

impl std::fmt::Debug for OrgTimers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrgTimers")
            .field("running", &self.running())
            .finish()
    }
}

impl OrgTimers {
    /// No timer runs yet: `start_up` starts one per joined org. They all stop with `shutdown`.
    pub fn new(
        orgs: Orgs,
        clock: Arc<dyn Clock>,
        bus: Bus,
        shutdown: CancellationToken,
    ) -> OrgTimers {
        OrgTimers {
            inner: Arc::new(Inner {
                orgs,
                clock,
                bus,
                shutdown,
                timers: Mutex::default(),
                startup: Mutex::default(),
            }),
        }
    }

    /// At start, after `Orgs::load`: starts every joined org's timer, then fetches every org
    /// once (`Orgs::refresh_all`) without holding the start up, and publishes `OrgUpdated` for
    /// each. A source that is slow or down must not keep the app from opening; until it
    /// answers, an org resolves from its offline copy.
    pub fn start_up(&self) {
        let this = self.clone();
        let stop = self.inner.shutdown.clone();
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = stop.cancelled() => {}
                _ = this.fetch_all() => {}
            }
        });
        *lock(&self.inner.startup) = Some(task);
    }

    /// Starts the timer of `org_id`, whose first fetch is `refresh_minutes` from now: it was
    /// fetched just now, by its join or the start-up fetch. An org whose timer runs already
    /// keeps that one, re-timed from now.
    pub fn start(&self, org_id: &str) {
        let next = next_fetch(&self.inner, org_id);
        let mut timers = lock(&self.inner.timers);
        if let Some(timer) = timers.get(org_id).filter(|t| !t.task.is_finished()) {
            *lock(&timer.due) = next;
            timer.moved.notify_one();
            return;
        }
        let stop = self.inner.shutdown.child_token();
        let due = Arc::new(Mutex::new(next));
        let moved = Arc::new(Notify::new());
        let task = tokio::spawn(tick(
            self.inner.clone(),
            org_id.to_string(),
            stop.clone(),
            due.clone(),
            moved.clone(),
        ));
        let timer = Timer {
            stop,
            due,
            moved,
            task,
        };
        timers.insert(org_id.to_string(), timer);
    }

    /// The org was fetched just now (**Update**, **Refresh**): its next fetch is
    /// `refresh_minutes` of its policy from now. An org without a timer gets one.
    pub fn retime(&self, org_id: &str) {
        self.start(org_id);
    }

    /// Stops the timer of `org_id` (**Remove**). A fetch it has begun runs to its end.
    pub fn stop(&self, org_id: &str) {
        if let Some(timer) = lock(&self.inner.timers).remove(org_id) {
            timer.stop.cancel();
        }
    }

    /// The orgs whose timer runs, sorted.
    pub fn running(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.inner.timers)
            .iter()
            .filter(|(_, timer)| !timer.task.is_finished())
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Whether the start-up fetch is still going.
    pub fn starting_up(&self) -> bool {
        lock(&self.inner.startup)
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    async fn fetch_all(&self) {
        let orgs = &self.inner.orgs;
        let ids: Vec<String> = orgs.summaries().await.into_iter().map(|s| s.id).collect();
        for id in &ids {
            self.start(id);
        }
        orgs.refresh_all().await;
        for id in ids {
            published(&self.inner.bus, id);
        }
    }
}

/// One org's timer: fetches it once `due` has come, and then `refresh_minutes` of its policy
/// after each fetch, until `stop`. A move of `due` wakes it to sleep anew. An org that is no
/// longer joined ends it.
async fn tick(
    inner: Arc<Inner>,
    org_id: String,
    stop: CancellationToken,
    due: Due,
    moved: Arc<Notify>,
) {
    loop {
        let next = *lock(&due);
        let wake = next.min(inner.clock.now() + MAX_SLEEP);
        tokio::select! {
            biased;
            _ = stop.cancelled() => return,
            _ = moved.notified() => continue,
            _ = inner.clock.sleep_until(wake) => {}
        }
        if inner.clock.now() < *lock(&due) {
            continue;
        }
        match inner.orgs.refresh(&org_id).await {
            Ok(_) => published(&inner.bus, org_id.clone()),
            Err(Error::NotFound(_)) => return,
            // A store failure; the next fetch tries again.
            Err(e) => tracing::warn!("org {org_id:?} was not refreshed by its timer: {e}"),
        }
        *lock(&due) = next_fetch(&inner, &org_id);
    }
}

/// `refresh_minutes` of the org's accepted policy from now; the policy's default while it has
/// none.
fn next_fetch(inner: &Inner, org_id: &str) -> DateTime<Utc> {
    let minutes = inner
        .orgs
        .policy(org_id)
        .map_or(DEFAULT_REFRESH_MINUTES, |p| p.refresh_minutes);
    inner.clock.now() + Duration::minutes(minutes.into())
}

fn published(bus: &Bus, org_id: String) {
    let _ = bus.send(RuntimeEvent::OrgUpdated { org_id });
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
