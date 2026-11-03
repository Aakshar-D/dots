//! Delivery tracking (delivery tracking spec sections 2 and 3): why a queued run is still
//! waiting, and the sweeper that marks a run not delivered once it has waited 30 minutes.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeDelta, TimeZone, Utc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{Run, WaitReason};
use crate::runner::Runner;
use crate::scheduler::Clock;
use crate::store::Store;
use crate::util::timestamp;
use crate::Result;

/// How long a run waits before the sweeper marks it not delivered.
pub const UNDELIVERED_AFTER: TimeDelta = TimeDelta::minutes(30);
/// How often the sweeper looks.
const SWEEP_EVERY: TimeDelta = TimeDelta::seconds(30);

impl WaitReason {
    /// The words for the reason, in the run's waiting line and as a sweeper's
    /// `undelivered_reason`.
    pub fn text(self) -> &'static str {
        match self {
            WaitReason::Busy => "other runs are busy",
            WaitReason::UsageHold => "held for the usage limit",
            WaitReason::Updating => "Dots is updating",
            WaitReason::DotBusy => "this dot is busy with another run",
            WaitReason::Queued => "next in line",
        }
    }
}

/// What a queued run's wait depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitFacts {
    /// The usage limits hold it (a background Claude run).
    pub usage_held: bool,
    /// The runner's queue is held (an update is installing).
    pub runner_held: bool,
    /// Another run of its dot, not of a workflow step, is active in the runner: the dispatcher
    /// starts one run per dot.
    pub dot_has_active_run: bool,
    /// The runner's slots in use now.
    pub running: usize,
    pub max_concurrent: usize,
}

/// Why a queued run waits, the first that applies: the usage hold, an update, its dot busy
/// with another run, every slot taken, else it is next in line.
pub fn wait_reason(f: &WaitFacts) -> WaitReason {
    if f.usage_held {
        WaitReason::UsageHold
    } else if f.runner_held {
        WaitReason::Updating
    } else if f.dot_has_active_run {
        WaitReason::DotBusy
    } else if f.running >= f.max_concurrent {
        WaitReason::Busy
    } else {
        WaitReason::Queued
    }
}

/// The local day of `now` in `tz` as `[start, end)` in UTC (delivery tracking spec section
/// 3.8): local midnight of `now`'s local date and of the next date. A midnight that occurs
/// twice is its earlier instant; one in a gap is the first valid instant after it. Write both
/// with `util::timestamp` before comparing them with stored times.
pub fn local_day_bounds<Tz: TimeZone>(
    now: DateTime<Utc>,
    tz: &Tz,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let date = now.with_timezone(tz).date_naive();
    let next = date.succ_opt().unwrap_or(date);
    (midnight(tz, date, now), midnight(tz, next, now))
}

/// Local midnight of `date` in `tz` as an instant (`schedule::resolve`'s rules: earliest of
/// two, forward out of a gap). Should `tz` give no instant at all, `now`'s offset is used.
fn midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate, now: DateTime<Utc>) -> DateTime<Utc> {
    let local: NaiveDateTime = date.and_time(NaiveTime::MIN);
    match crate::schedule::resolve(tz, local) {
        Some(t) => t.with_timezone(&Utc),
        None => {
            let offset = now.with_timezone(tz).offset().fix().local_minus_utc();
            (local - TimeDelta::seconds(offset as i64)).and_utc()
        }
    }
}

/// Marks the runs that waited 30 minutes without reaching their dot (delivery tracking spec
/// section 3), every 30 seconds of its clock. A run held for the usage limit or for a member's
/// role is left alone: both holds are deliberate.
pub struct DeliverySweeper {
    store: Store,
    runner: Arc<Runner>,
    bus: Bus,
    clock: Arc<dyn Clock>,
}

impl DeliverySweeper {
    pub fn new(store: Store, runner: Arc<Runner>, bus: Bus, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            runner,
            bus,
            clock,
        }
    }

    /// Marks every run queued 30 minutes or more ago, still unmarked, with the reason it
    /// waits, and returns the runs it marked.
    pub async fn sweep(&self) -> Result<Vec<Run>> {
        let now = self.clock.now();
        let mut runs = self
            .store
            .overdue_undelivered(&timestamp(now - UNDELIVERED_AFTER))
            .await?;
        if runs.is_empty() {
            return Ok(Vec::new());
        }
        self.runner.fill_wait_reasons(&mut runs).await?;
        let mut marked = Vec::new();
        for run in runs {
            let reason = match run.wait_reason {
                Some(WaitReason::UsageHold) | None => continue,
                Some(reason) => reason,
            };
            // A run held for its member's role waits until the role returns (members spec
            // section 6.3), as deliberately as the usage hold.
            if self.runner.role_hold(&run).await?.is_some() {
                continue;
            }
            if let Some(row) = self
                .store
                .mark_undelivered(&run.id, reason.text(), now)
                .await?
            {
                let _ = self.bus.send(RuntimeEvent::RunUpdated { run: row.clone() });
                marked.push(row);
            }
        }
        Ok(marked)
    }

    /// Sweeps every 30 seconds of the clock until `shutdown`.
    pub fn spawn(self: Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let next = self.clock.now() + SWEEP_EVERY;
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = self.clock.sleep_until(next) => {}
                }
                if let Err(e) = self.sweep().await {
                    tracing::warn!("delivery sweep failed: {e}");
                }
            }
        })
    }
}
