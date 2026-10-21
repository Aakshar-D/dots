//! Follow-ups (check-ins spec sections 3 and 6): runs a dot schedules for itself. A follow-up
//! continues the thread of the run that scheduled it, with the dot's note, once it is due.
//! This module holds the limits, checked when the tool is called, and the follow-up loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::checkins::{local_label, When};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{FollowUp, NewRun, Run, TriggerKind};
use crate::runner::Runner;
use crate::scheduler::{Clock, ERROR_WAIT, MAX_SLEEP};
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

/// The loop (spec section 6).
impl FollowUps {
    /// Runs the loop until `shutdown`: a pass over the pending follow-ups, then a sleep until
    /// the soonest is due or `MAX_SLEEP`, whichever is first. `wake`, a pause or a resume, and
    /// the end of any run wake it at once. The first pass runs at once and fires what came due
    /// while Dots was closed. `runner` announces the runs it starts and wakes the dispatcher.
    pub fn spawn(
        self: &Arc<Self>,
        runner: Arc<Runner>,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let this = self.clone();
        // Subscribed before the first pass, so no end of a run is missed.
        let mut events = self.bus.subscribe();
        tokio::spawn(async move {
            loop {
                let wake = match this.step(&runner).await {
                    Ok(wake) => wake,
                    Err(e) => {
                        tracing::error!("follow-up loop error: {e}");
                        Some(this.clock.now() + ERROR_WAIT)
                    }
                };
                let sleep = async {
                    match wake {
                        Some(at) => this.clock.sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                // A wake that came during the pass is taken before the clock is asked to sleep.
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = this.wake.notified() => {}
                    _ = run_ended(&mut events) => {}
                    _ = sleep => {}
                }
            }
        })
    }

    /// One turn of the loop: `None` while schedules are paused (only a wake moves it on), else
    /// a pass and the time to wake for the next one.
    async fn step(&self, runner: &Runner) -> Result<Option<DateTime<Utc>>> {
        if self.is_paused() {
            return Ok(None);
        }
        let now = self.clock.now();
        let mut wake = now + MAX_SLEEP;
        for row in self.store.list_pending_follow_ups(None).await? {
            let at = parse_time(row.retry_at.as_deref().unwrap_or(&row.due_at))?;
            if at > now {
                wake = wake.min(at);
                continue;
            }
            self.handle(runner, &row, now).await;
        }
        Ok(Some(wake))
    }

    /// One due row: skipped with a note while its dot is off; left for later while its thread
    /// is busy (the end of the run wakes the loop); else fired. A failure while firing it is a
    /// failed start (`failed_start`).
    async fn handle(&self, runner: &Runner, row: &FollowUp, now: DateTime<Utc>) {
        let fired = match self.store.get_dot(&row.dot_id).await {
            // Deleted meanwhile: its rows went with it.
            Err(Error::NotFound(_)) => return,
            Err(e) => Err(e),
            Ok(dot) if !dot.spec.enabled => {
                let text = format!(
                    "A follow-up was skipped because the dot is off. It was due {}: {}",
                    due_label(row),
                    row.note
                );
                if let Err(e) = self.skip(row, &text, false).await {
                    tracing::warn!(follow_up = %row.id, "skipping a follow-up failed: {e}");
                }
                return;
            }
            Ok(_) => self.fire(row, now).await,
        };
        match fired {
            Ok(Some(run)) => {
                tracing::info!(follow_up = %row.id, run = %run.id, "follow-up fired");
                runner.enqueued(&run);
                self.updated(&row.dot_id);
            }
            Ok(None) => {
                tracing::debug!(follow_up = %row.id, "thread busy: the follow-up waits");
            }
            // Cancelled since the pass read it.
            Err(Error::Conflict(_)) => {}
            Err(e) => self.failed_start(row, &e, now).await,
        }
    }

    /// Under the thread lock: `None` while a run of the thread has not ended, else the run that
    /// continues it, stored with the row (`Store::fire_follow_up`).
    async fn fire(&self, row: &FollowUp, now: DateTime<Utc>) -> Result<Option<Run>> {
        let _one_at_a_time = self.thread_lock.lock().await;
        let from = match self.store.get_run(&row.from_run_id).await {
            Ok(run) => Some(run),
            Err(Error::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        let thread = match &from {
            Some(from) => self.store.list_thread(&from.root_run_id).await?,
            None => Vec::new(),
        };
        if thread.iter().any(|r| !r.status.is_terminal()) {
            return Ok(None);
        }
        let new = follow_up_run(row, from.as_ref(), thread.last(), now)?;
        self.store.fire_follow_up(&row.id, &new).await.map(Some)
    }

    /// A start of `row` failed with `e`: it is tried again after the next of `RETRY_DELAYS`;
    /// once they are used up, it is skipped with a note that says why.
    async fn failed_start(&self, row: &FollowUp, e: &Error, now: DateTime<Utc>) {
        tracing::warn!(
            follow_up = %row.id, attempt = row.attempts + 1,
            "the follow-up's run could not be started: {e}"
        );
        let written = match RETRY_DELAYS.get(row.attempts as usize) {
            Some(delay) => match self.store.retry_follow_up(&row.id, now + *delay).await {
                Ok(_) => {
                    self.updated(&row.dot_id);
                    Ok(())
                }
                Err(e) => Err(e),
            },
            None => {
                let text = format!(
                    "A follow-up was skipped because its run could not be started ({e}). It was \
                     due {}: {}",
                    due_label(row),
                    row.note
                );
                self.skip(row, &text, true).await
            }
        };
        if let Err(e) = written {
            tracing::error!(follow_up = %row.id, "recording a failed start failed: {e}");
        }
    }

    /// Skips `row` and tells the person why with a note of Dots' own. `failed`: the start that
    /// failed last is counted.
    async fn skip(&self, row: &FollowUp, text: &str, failed: bool) -> Result<()> {
        if !self.store.skip_follow_up(&row.id, failed).await? {
            return Ok(());
        }
        let message = self.store.add_note(&row.dot_id, None, text).await?;
        let _ = self.bus.send(RuntimeEvent::MessagePosted { message });
        self.updated(&row.dot_id);
        Ok(())
    }
}

/// The run that serves `row` at `now` (spec section 6): it continues the thread's `end` when
/// that has a session, as `chat` does; else it starts afresh and says what the run `from`
/// reported.
pub fn follow_up_run(
    row: &FollowUp,
    from: Option<&Run>,
    end: Option<&Run>,
    now: DateTime<Utc>,
) -> Result<NewRun> {
    let head = format!(
        "Follow-up you scheduled on {}: {}{}",
        local_label(parse_time(&row.created_at)?),
        row.note,
        late_words(parse_time(&row.due_at)?, now)
    );
    let mut new = NewRun::new(&row.dot_id, TriggerKind::FollowUp);
    let message = match end.filter(|end| end.session_id.is_some()) {
        Some(end) => {
            new.parent_run_id = Some(end.id.clone());
            new.root_run_id = Some(end.root_run_id.clone());
            new.session_id = end.session_id.clone();
            new.workspace_path = end.workspace_path.clone();
            new.branch = end.branch.clone();
            new.base_commit = end.base_commit.clone();
            head
        }
        None => {
            let reported = from
                .and_then(|run| run.summary.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("nothing");
            format!("{head}\nWhat the earlier run reported: {reported}")
        }
    };
    new.payload = Some(json!({ "message": message, "follow_up_id": row.id }));
    Ok(new)
}

/// What a follow-up's message adds when it fires more than `LATE_AFTER` after `due`: how many
/// hours late it is, rounded, at least 1. Nothing otherwise.
pub fn late_words(due: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let late = now - due;
    if late <= LATE_AFTER {
        return String::new();
    }
    let hours = ((late.num_minutes() + 30) / 60).max(1);
    let unit = if hours == 1 { "hour" } else { "hours" };
    // No cause is named: a pause, a busy thread or a retry make a follow-up late while Dots runs.
    format!(" (this is {hours} {unit} later than you asked)")
}

/// When `row` was due, as the person reads it.
fn due_label(row: &FollowUp) -> String {
    parse_time(&row.due_at).map_or_else(|_| row.due_at.clone(), local_label)
}

/// Returns when a run has ended (its `RunUpdated`), or when events were lost and one may have.
async fn run_ended(events: &mut Receiver<RuntimeEvent>) {
    loop {
        match events.recv().await {
            Ok(RuntimeEvent::RunUpdated { run }) if run.status.is_terminal() => return,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return,
            Err(RecvError::Closed) => std::future::pending().await,
        }
    }
}
