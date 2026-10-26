//! Claude usage limits (usage and limits spec sections 3 and 4). The `claude` CLI reports the
//! account's limit windows in its output; `Limits` keeps the latest reading of each, the
//! person's hold shares and whether held runs are draining, and answers the dispatcher's one
//! question: may this queued run start now (`admit`)? It never asks Anthropic anything: the
//! readings come only from runs that happen anyway.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

use crate::events::{Bus, RuntimeEvent};
use crate::model::{EngineKind, TriggerKind};
use crate::scheduler::Clock;
use crate::settings::Settings;
use crate::store::Store;
use crate::Result;

pub use crate::settings::SHARE_RULE;
pub use crate::store::LimitReading;

/// The windows that hold runs back; the others (weekly per model) are shown only.
pub const FIVE_HOUR: &str = "five_hour";
pub const WEEKLY: &str = "seven_day";
/// The meter shows a reading's age once it is older than this.
pub const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// The heaviest runs the Usage page lists per dot.
pub const HEAVIEST: usize = 5;

pub const STOPPED_BY_LIMIT: &str = "stopped by Claude's usage limit";
pub const HOLD_ENDED: &str = "Held runs are starting";
pub const NOT_HELD: &str = "This run is not held.";

/// "Claude's limit is reached until 15:10." `until` in local time: the time alone when it is
/// today, else the weekday too.
pub fn limit_reached(until: DateTime<Utc>, now: DateTime<Utc>) -> String {
    format!(
        "Claude's limit is reached until {}.",
        local_time(until, now)
    )
}

/// "Dots is holding background runs: Claude 5-hour usage is at 64%"
pub fn hold_started(window: &str, utilization: f64) -> String {
    format!(
        "Dots is holding background runs: Claude {} usage is at {}%",
        window_label(window),
        percent(utilization)
    )
}

/// "5-hour" or "weekly"; another window by its own name.
pub fn window_label(window: &str) -> &str {
    match window {
        FIVE_HOUR => "5-hour",
        WEEKLY => "weekly",
        other => other,
    }
}

/// The share used, in whole percent, at most 100.
pub fn percent(utilization: f64) -> u32 {
    (utilization * 100.0).round().clamp(0.0, 100.0) as u32
}

fn local_time(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (t, now) = (t.with_timezone(&Local), now.with_timezone(&Local));
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else {
        t.format("%a %H:%M").to_string()
    }
}

/// A run the person did not start this moment: these are the ones held back.
pub fn is_background(t: TriggerKind) -> bool {
    matches!(
        t,
        TriggerKind::Schedule | TriggerKind::Watcher | TriggerKind::FollowUp | TriggerKind::Webhook
    )
}

/// The person's hold settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HoldShares {
    pub enabled: bool,
    pub five_hour: u8,
    pub weekly: u8,
}

impl From<&Settings> for HoldShares {
    fn from(s: &Settings) -> Self {
        Self {
            enabled: s.hold_enabled,
            five_hour: s.hold_share_five_hour,
            weekly: s.hold_share_weekly,
        }
    }
}

/// What the meter, the Usage page and the Inbox show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitsState {
    /// Every stored reading, by window name.
    pub windows: Vec<LimitReading>,
    /// The 5-hour window's reading while it is known (not yet reset).
    pub five_hour: Option<LimitReading>,
    /// The weekly window's reading while it is known.
    pub weekly: Option<LimitReading>,
    pub shares: HoldShares,
    /// Some queued run is held by a share or by Claude's `rejected`.
    pub holding: bool,
    /// Held runs are starting one at a time after a hold.
    pub draining: bool,
    pub rejected_until: Option<String>,
    /// The queued runs that may not start now, oldest first.
    pub held_run_ids: Vec<String>,
    /// When the window that holds resets.
    pub hold_until: Option<String>,
}

/// A hold starting or ending; the app notifies on each.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HoldChange {
    Started { window: String, utilization: f64 },
    Ended,
}

/// A queued run as the guard sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedRun {
    pub id: String,
    pub trigger: TriggerKind,
    pub engine: EngineKind,
    pub hold_released: bool,
    pub queued_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    Go,
    /// Held by a share, or waiting its turn while held runs drain.
    Held,
    /// Claude's limit is reached: no Claude run starts until it resets.
    Rejected,
}

#[derive(Debug)]
struct Inner {
    readings: BTreeMap<String, LimitReading>,
    shares: HoldShares,
    draining: bool,
    rejected_until: Option<DateTime<Utc>>,
    /// What the last `publish` said of holding.
    holding: bool,
    /// What the last `publish` sent: the dispatcher publishes on every pass, and only a change
    /// is sent again.
    sent: Option<LimitsState>,
}

pub struct Limits {
    store: Store,
    bus: Bus,
    clock: Arc<dyn Clock>,
    inner: Mutex<Inner>,
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

impl Inner {
    /// The reading of `window` while it has not reset.
    fn known(&self, window: &str, now: DateTime<Utc>) -> Option<&LimitReading> {
        self.readings
            .get(window)
            .filter(|r| parse_time(&r.resets_at).is_some_and(|t| t > now))
    }

    /// A hold window read before and reset since.
    fn reset(&self, now: DateTime<Utc>) -> bool {
        [FIVE_HOUR, WEEKLY].iter().any(|w| {
            self.readings
                .get(*w)
                .is_some_and(|r| parse_time(&r.resets_at).is_none_or(|t| t <= now))
        })
    }

    /// The known hold window at or above its share, if holding is on.
    fn over_share(&self, now: DateTime<Utc>) -> Option<&LimitReading> {
        if !self.shares.enabled {
            return None;
        }
        [
            (FIVE_HOUR, self.shares.five_hour),
            (WEEKLY, self.shares.weekly),
        ]
        .into_iter()
        .find_map(|(w, share)| {
            self.known(w, now)
                .filter(|r| r.utilization * 100.0 + 1e-9 >= f64::from(share))
        })
    }

    fn rejected(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.rejected_until.filter(|t| *t > now)
    }

    fn admit(
        &self,
        run: &QueuedRun,
        running_background_claude: usize,
        now: DateTime<Utc>,
    ) -> Admit {
        if run.engine != EngineKind::Claude {
            return Admit::Go;
        }
        if self.rejected(now).is_some() {
            return Admit::Rejected;
        }
        if !is_background(run.trigger) || run.hold_released {
            return Admit::Go;
        }
        if self.over_share(now).is_some() {
            return Admit::Held;
        }
        if running_background_claude >= 1 && (self.draining || self.reset(now)) {
            return Admit::Held;
        }
        Admit::Go
    }

    /// The runs that may not start now, oldest first, as the dispatcher would go through them:
    /// each background Claude run let through counts as running for the next.
    fn held(&self, queued: &[QueuedRun], running: usize, now: DateTime<Utc>) -> Vec<String> {
        let mut running = running;
        let mut held = Vec::new();
        for run in queued {
            match self.admit(run, running, now) {
                Admit::Go => {
                    if run.engine == EngineKind::Claude && is_background(run.trigger) {
                        running += 1;
                    }
                }
                Admit::Held | Admit::Rejected => held.push(run.id.clone()),
            }
        }
        held
    }

    /// Whether a share or `rejected` holds some queued Claude run (draining does not count).
    fn holding(&self, queued: &[QueuedRun], now: DateTime<Utc>) -> bool {
        let claude = |r: &&QueuedRun| r.engine == EngineKind::Claude;
        if self.rejected(now).is_some() {
            return queued.iter().any(|r| claude(&r));
        }
        self.over_share(now).is_some()
            && queued
                .iter()
                .filter(claude)
                .any(|r| is_background(r.trigger) && !r.hold_released)
    }

    fn state(&self, queued: &[QueuedRun], running: usize, now: DateTime<Utc>) -> LimitsState {
        let hold_until = self
            .rejected(now)
            .map(crate::util::timestamp)
            .or_else(|| self.over_share(now).map(|r| r.resets_at.clone()));
        LimitsState {
            windows: self.readings.values().cloned().collect(),
            five_hour: self.known(FIVE_HOUR, now).cloned(),
            weekly: self.known(WEEKLY, now).cloned(),
            shares: self.shares,
            holding: self.holding(queued, now),
            draining: self.draining,
            rejected_until: self.rejected(now).map(crate::util::timestamp),
            held_run_ids: self.held(queued, running, now),
            hold_until,
        }
    }
}

impl Limits {
    /// The service, with the readings stored before.
    pub async fn load(
        store: Store,
        bus: Bus,
        clock: Arc<dyn Clock>,
        shares: HoldShares,
    ) -> Result<Arc<Self>> {
        let readings = store
            .limit_readings()
            .await?
            .into_iter()
            .map(|r| (r.window.clone(), r))
            .collect();
        let mut inner = Inner {
            readings,
            shares,
            draining: false,
            rejected_until: None,
            holding: false,
            sent: None,
        };
        // What the UI reads when it opens (`limits_get`) is not news: only a change is sent.
        inner.sent = Some(inner.state(&[], 0, clock.now()));
        Ok(Arc::new(Self {
            store,
            bus,
            clock,
            inner: Mutex::new(inner),
        }))
    }

    /// Keeps a reading from the CLI. `rejected` holds every Claude run until the limiting
    /// window resets (the latest reset among the windows it names as rejected, or among all).
    pub async fn record(&self, status: &str, readings: Vec<LimitReading>) -> Result<()> {
        self.store.save_limit_readings(&readings).await?;
        let mut inner = self.inner.lock().unwrap();
        if status == "rejected" {
            let rejected: Vec<&LimitReading> =
                readings.iter().filter(|r| r.status == "rejected").collect();
            let pool = if rejected.is_empty() {
                readings.iter().collect()
            } else {
                rejected
            };
            if let Some(until) = pool.iter().filter_map(|r| parse_time(&r.resets_at)).max() {
                inner.rejected_until = Some(until);
            }
        } else {
            inner.rejected_until = None;
        }
        for r in readings {
            inner.readings.insert(r.window.clone(), r);
        }
        Ok(())
    }

    pub fn set_shares(&self, shares: HoldShares) {
        self.inner.lock().unwrap().shares = shares;
    }

    /// May `run` start now, with `running_background_claude` background Claude runs running?
    pub fn admit(&self, run: &QueuedRun, running_background_claude: usize) -> Admit {
        let now = self.clock.now();
        self.inner
            .lock()
            .unwrap()
            .admit(run, running_background_claude, now)
    }

    pub fn state(&self, queued: &[QueuedRun], running_background_claude: usize) -> LimitsState {
        let now = self.clock.now();
        self.inner
            .lock()
            .unwrap()
            .state(queued, running_background_claude, now)
    }

    /// Sends `LimitsUpdated`, with the hold's start or end when it changed since the last
    /// publish. A hold that ends while background Claude runs are queued starts draining; no
    /// such run left ends it.
    pub fn publish(&self, queued: &[QueuedRun], running_background_claude: usize) {
        let now = self.clock.now();
        let (limits, change) = {
            let mut inner = self.inner.lock().unwrap();
            let holding = inner.holding(queued, now);
            let waiting = queued
                .iter()
                .any(|r| r.engine == EngineKind::Claude && is_background(r.trigger));
            let change = match (inner.holding, holding) {
                (false, true) => {
                    let (window, utilization) = match inner.over_share(now) {
                        Some(r) => (r.window.clone(), r.utilization),
                        None => {
                            let r = inner.readings.values().find(|r| r.status == "rejected");
                            r.map(|r| (r.window.clone(), r.utilization))
                                .unwrap_or_else(|| (FIVE_HOUR.to_string(), 1.0))
                        }
                    };
                    Some(HoldChange::Started {
                        window,
                        utilization,
                    })
                }
                (true, false) => {
                    inner.draining = waiting;
                    Some(HoldChange::Ended)
                }
                _ => None,
            };
            if !waiting {
                inner.draining = false;
            }
            inner.holding = holding;
            let limits = inner.state(queued, running_background_claude, now);
            if change.is_none() && inner.sent.as_ref() == Some(&limits) {
                return;
            }
            inner.sent = Some(limits.clone());
            (limits, change)
        };
        let _ = self
            .bus
            .send(RuntimeEvent::LimitsUpdated { limits, change });
    }

    /// When Claude's `rejected` ends, as stored.
    pub fn rejected_until(&self) -> Option<String> {
        let now = self.clock.now();
        self.inner
            .lock()
            .unwrap()
            .rejected(now)
            .map(crate::util::timestamp)
    }

    /// The next time the guard's answer may change by itself: the earliest future reset of a
    /// hold window or of `rejected`.
    pub fn next_change(&self) -> Option<DateTime<Utc>> {
        let now = self.clock.now();
        let inner = self.inner.lock().unwrap();
        [FIVE_HOUR, WEEKLY]
            .iter()
            .filter_map(|w| inner.readings.get(*w))
            .filter_map(|r| parse_time(&r.resets_at))
            .chain(inner.rejected_until)
            .filter(|t| *t > now)
            .min()
    }

    /// The clock the guard reads.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}
