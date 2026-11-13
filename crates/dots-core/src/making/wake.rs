//! Waking a maker (dots-make-dots design sections 7.2 and 8): a message Dots has for a maker's
//! chat topic, the person's answer to a make request or the end of a job, becomes one new run
//! of the maker in that topic. A wake is stored first and delivered by exactly one run. While
//! the topic is busy it waits, since nothing continues a thread that has not ended; the loop
//! here delivers it when a run ends, and once at start for what waited across a restart.

use std::collections::HashSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::service::Makers;
use crate::events::RuntimeEvent;
use crate::limits::BACKGROUND;
use crate::scheduler::ERROR_WAIT;
use crate::{Error, Result};

/// The key of a wake's run's payload that holds the wake's id: the run's message is Dots' own,
/// not one the person typed, as `follow_up_id` says of a follow-up's.
pub const WAKE_ID: &str = "wake_id";

/// How often one failure that keeps coming back is logged again.
const LOG_AGAIN: std::time::Duration = std::time::Duration::from_secs(3600);

/// When each failure the wakes' loop keeps meeting was last logged, by a key of its own (a
/// wake's id, or a part of the loop): so a wake that fails on every pass, every 30 seconds, is
/// logged once and then at most hourly.
#[derive(Default)]
pub(crate) struct LogEvery(std::sync::Mutex<std::collections::HashMap<String, Instant>>);

impl LogEvery {
    /// Whether the failure `key` is to be logged at `now`: the first time, and once an hour
    /// has passed since it last was.
    pub(crate) fn due(&self, key: &str, now: Instant) -> bool {
        let Ok(mut seen) = self.0.lock() else {
            return true;
        };
        match seen.get(key) {
            Some(at) if now.saturating_duration_since(*at) < LOG_AGAIN => false,
            _ => {
                seen.insert(key.to_string(), now);
                true
            }
        }
    }

    /// Forgets `key`: it no longer fails, so a new failure is logged at once.
    pub(crate) fn forget(&self, key: &str) {
        if let Ok(mut seen) = self.0.lock() {
            seen.remove(key);
        }
    }
}

/// One wake: a message for the chat topic `root_run_id` of the maker `maker_dot_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakingWake {
    pub id: String,
    /// The maker.
    pub maker_dot_id: String,
    /// The maker's chat topic the wake is for, as it was when the wake was stored. The wake
    /// follows that run when the person moves it or merges its topic into another.
    pub root_run_id: String,
    /// What the maker's new run is told.
    pub message: String,
    pub created_at: String,
    /// The run that delivered it. `None` while it waits, and for a wake that was dropped.
    pub run_id: Option<String>,
    /// When it was delivered or dropped; `None` while it waits.
    pub delivered_at: Option<String>,
}

/// What came of trying one waiting wake.
enum Tried {
    /// Its run is stored and announced.
    Delivered,
    /// Its topic has a run that has not ended: it waits for the end of that run.
    Busy,
    /// Its topic is adding an app the person accepted, which ends with no run of its own when
    /// the adding fails: it is tried again after a wait.
    Deciding,
    /// This PC's role may not run the maker now, and its run would be one nobody holds: it is
    /// tried again after a wait.
    NotAllowed,
    /// Its topic is gone, or another pass had delivered it since the list was read.
    Done,
    /// An earlier wake of its topic was tried in this pass.
    Later,
}

impl Makers {
    /// Queues a wake of the maker's chat topic and delivers it now if the topic is free
    /// (design sections 7.2 and 8). Returns once the wake is stored: delivery never fails the
    /// caller.
    ///
    /// `root_run_id` is any run that is still in the topic, by rule its first one. An error
    /// means the wake was not stored: the maker is deleted, or the store failed. It takes the
    /// thread lock (`FollowUps::thread_lock`), so the caller must not hold it.
    pub async fn wake(&self, maker_dot_id: &str, root_run_id: &str, message: &str) -> Result<()> {
        let wake = self
            .parts
            .store
            .add_making_wake(maker_dot_id, root_run_id, message)
            .await?;
        tracing::debug!(wake = %wake.id, maker = %maker_dot_id, "a wake was stored");
        if let Err(e) = self.deliver_wakes().await {
            tracing::debug!(wake = %wake.id, "the wake waits; delivering it failed: {e}");
        }
        Ok(())
    }

    /// Delivers waiting wakes of every topic that is free now, oldest first, one per topic per
    /// pass. Called when a run ends, at startup, and by `wake`.
    ///
    /// Two passes at once deliver a wake once: each wake is tried under the thread lock, and
    /// its row is marked in the transaction that stores its run. A wake whose run could not be
    /// stored stays waiting, and the error of the first such wake is returned after the other
    /// topics were tried. When a wake waits for a topic that is adding an app, or a wake
    /// failed, the wakes' loop is told to try again after its wait: no end of a run may come
    /// to set it going.
    pub async fn deliver_wakes(&self) -> Result<()> {
        let pass = self.wake_pass().await;
        if !matches!(pass, Ok(false)) {
            self.parts.retry.notify_one();
        }
        pass.map(|_| ())
    }

    /// One pass. `true` when a wake waits for something whose end no event tells (`Deciding`,
    /// `NotAllowed`): the loop then tries again after a wait. A wake that fails is logged once,
    /// then at most hourly while it keeps failing (`LogEvery`).
    async fn wake_pass(&self) -> Result<bool> {
        let mut tried_topics = HashSet::new();
        let mut again = false;
        let mut failed = None;
        let waiting = match self.parts.store.waiting_making_wakes().await {
            Ok(waiting) => waiting,
            Err(e) => {
                // Logged here: `wake` logs a failed delivery only at debug.
                if self.parts.logged.due(WAITING_LIST, Instant::now()) {
                    tracing::warn!("the waiting wakes could not be read: {e}");
                }
                return Err(e);
            }
        };
        self.parts.logged.forget(WAITING_LIST);
        for wake in waiting {
            match self.deliver_wake(&wake, &mut tried_topics).await {
                Ok(Tried::Deciding | Tried::NotAllowed) => again = true,
                Ok(Tried::Delivered | Tried::Done) => self.parts.logged.forget(&wake.id),
                Ok(_) => {}
                Err(e) => {
                    if self.parts.logged.due(&wake.id, Instant::now()) {
                        tracing::warn!(wake = %wake.id, "the wake's run could not be started: {e}");
                    }
                    failed.get_or_insert(e);
                }
            }
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(again),
        }
    }

    /// Tries one waiting wake, under the thread lock, as `Runtime::decide_instruction_proposal`
    /// continues a chat: the topic's end is read, and a topic with a run that has not ended is
    /// left alone. Here that is no refusal: the wake waits. `tried_topics` are the topics an
    /// earlier wake of this pass was tried for; the wake's own is added, so of several wakes
    /// of one topic the oldest goes first, also when its run could not be stored.
    ///
    /// A wake for a maker the person switched off, or whose account was removed from this PC,
    /// is dropped with a line in the log: neither runs. The wake's run goes on at the topic's
    /// end; when that run has no session (it ended before the dot started working) it goes on
    /// in the same topic, so the topic keeps its history and its count of jobs, in the session
    /// of the newest run of the topic that has one, or without one when none has.
    /// It is background in an unattended topic (`jobs::unattended`, design section 8).
    ///
    /// A wake's run is one Dots starts by itself, so it meets the check every other new run
    /// meets (members spec section 6.3): in an unattended topic it is background, and the role
    /// hold keeps it queued while this PC may not run the maker (`Runner::role_hold`); in a
    /// topic the person watches, which no hold stops, it is not made while this PC may not run
    /// the maker (`Runner::require_run`). The wake then waits, and is tried again after the
    /// loop's wait. The wake that carries the person's answer to a request is one of these.
    async fn deliver_wake(
        &self,
        wake: &MakingWake,
        tried_topics: &mut HashSet<String>,
    ) -> Result<Tried> {
        let p = &self.parts;
        let lock = p.follow_ups.thread_lock();
        let one_at_a_time = lock.lock().await;
        // The topic the wake's run is in now: the person may have moved it or merged its topic
        // into another since.
        let first = match p.store.get_run(&wake.root_run_id).await {
            Ok(run) if run.dot_id == wake.maker_dot_id => run,
            Ok(_) | Err(Error::NotFound(_)) => {
                // History was pruned or the topic deleted: there is no chat to say it in.
                return self.drop_wake(wake, "its chat topic is gone").await;
            }
            Err(e) => return Err(e),
        };
        let maker = match p.store.get_dot(&wake.maker_dot_id).await {
            Ok(maker) => maker,
            Err(Error::NotFound(_)) => return self.drop_wake(wake, "its maker is gone").await,
            Err(e) => return Err(e),
        };
        if !maker.spec.enabled {
            return self.drop_wake(wake, "its maker is switched off").await;
        }
        if p.vault.org_removed(&maker).is_some() {
            return self
                .drop_wake(wake, "its maker's account is no longer on this PC")
                .await;
        }
        let topic = first.root_run_id.clone();
        if !tried_topics.insert(topic.clone()) {
            return Ok(Tried::Later);
        }
        if p.follow_ups.deciding(&topic) {
            return Ok(Tried::Deciding);
        }
        let thread = p.store.list_thread(&topic).await?;
        if thread.iter().any(|r| !r.status.is_terminal()) {
            tracing::debug!(wake = %wake.id, "topic busy: the wake waits");
            return Ok(Tried::Busy);
        }
        let background = super::jobs::unattended(&thread);
        if !background {
            match p.runner.require_run(&maker).await {
                Ok(()) => {}
                Err(Error::NotAllowed(why)) => {
                    tracing::debug!(wake = %wake.id, "the wake waits: {why}");
                    return Ok(Tried::NotAllowed);
                }
                Err(e) => return Err(e),
            }
        }
        let end = thread.last().unwrap_or(&first);
        let mut new = crate::checkins::continuation(&first, end, wake.message.clone(), None);
        if new.root_run_id.is_none() {
            // `continuation` starts a new topic after a run without a session; a wake stays,
            // after the topic's end, in the session of the newest run of the topic that has
            // one: the woken maker still knows why it gave the job, and both engines see the
            // same history (the local engine replays the parent chain, where a run without a
            // session left no messages).
            let had = thread
                .iter()
                .rev()
                .find(|run| run.session_id.is_some())
                .unwrap_or(end);
            new.parent_run_id = Some(end.id.clone());
            new.root_run_id = Some(end.root_run_id.clone());
            new.session_id = had.session_id.clone();
            new.workspace_path = had.workspace_path.clone();
            new.branch = had.branch.clone();
            new.base_commit = had.base_commit.clone();
        }
        if let Some(payload) = new.payload.as_mut() {
            payload[WAKE_ID] = wake.id.clone().into();
            if background {
                payload[BACKGROUND] = true.into();
            }
        }
        let Some(run) = p.store.deliver_making_wake(&wake.id, &new).await? else {
            return Ok(Tried::Done);
        };
        drop(one_at_a_time);
        tracing::info!(wake = %wake.id, run = %run.id, "a maker was woken");
        p.runner.enqueued(&run);
        Ok(Tried::Delivered)
    }

    /// Drops the waiting `wake`, with one line in the log saying `why`.
    async fn drop_wake(&self, wake: &MakingWake, why: &str) -> Result<Tried> {
        if self.parts.store.drop_making_wake(&wake.id).await? {
            tracing::warn!(
                wake = %wake.id, maker = %wake.maker_dot_id, topic = %wake.root_run_id,
                "a wake was dropped because {why}"
            );
        }
        Ok(Tried::Done)
    }

    /// Runs the wakes' loop until `shutdown`, or until the runtime that made the makers is
    /// gone: a pass at once, for the wakes that waited across a restart, then one after the end
    /// of each run, which is what frees a topic. After a pass that failed, or that left a wake
    /// waiting for a topic that is adding an app, the next comes after `ERROR_WAIT` at the
    /// latest; a pass outside the loop that did so tells the loop (`deliver_wakes`), which then
    /// passes at once and waits as after its own. It does not keep the makers alive. An error
    /// that keeps coming back is logged once, then at most hourly.
    ///
    /// Each pass first follows the jobs (`settle_jobs`, design section 8): the end of a run is
    /// what moves or ends a job, the person's answer to a question carries a waiting job on,
    /// and a dismissed question ends it. At start that settles the jobs whose runs ended
    /// while Dots was closed; the runtime has failed the interrupted runs by then.
    pub(crate) fn spawn_wakes(&self, shutdown: CancellationToken) -> JoinHandle<()> {
        let makers = self.downgrade();
        let retry = self.parts.retry.clone();
        // Subscribed before the first pass, so no end of a run is missed.
        let mut events = self.parts.bus.subscribe();
        tokio::spawn(async move {
            loop {
                let Some(this) = makers.upgrade() else { break };
                let logged = &this.parts.logged;
                let jobs_failed = match this.settle_jobs().await {
                    Ok(()) => {
                        logged.forget(JOBS_LOOP);
                        false
                    }
                    Err(e) => {
                        if logged.due(JOBS_LOOP, Instant::now()) {
                            tracing::error!("job loop error: {e}");
                        }
                        true
                    }
                };
                // A pass that failed logged what failed itself (`wake_pass`).
                let again = match this.wake_pass().await {
                    Ok(again) => again || jobs_failed,
                    Err(_) => true,
                };
                let retry_wait = this
                    .parts
                    .retry_wait
                    .lock()
                    .map(|wait| *wait)
                    .unwrap_or_else(|_| ERROR_WAIT.to_std().unwrap_or_default());
                drop(this);
                let wait = async {
                    if again {
                        tokio::time::sleep(retry_wait).await
                    } else {
                        std::future::pending().await
                    }
                };
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    open = run_ended(&mut events) => if !open { break },
                    _ = retry.notified() => {}
                    _ = wait => {}
                }
            }
        })
    }
}

/// The key of the loop's own errors in `LogEvery`.
const JOBS_LOOP: &str = "loop: jobs";
/// The key in `LogEvery` of reading the waiting wakes.
const WAITING_LIST: &str = "wakes: waiting list";

/// Returns `true` when a run has ended (its `RunUpdated`), when a question was answered or
/// dismissed (its `MessageUpdated`: a job may wait on it), or when events were lost and one of
/// these may have happened; `false` when no more events come.
async fn run_ended(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> bool {
    loop {
        match events.recv().await {
            Ok(RuntimeEvent::RunUpdated { run }) if run.status.is_terminal() => return true,
            Ok(RuntimeEvent::MessageUpdated { message })
                if message.answered_at.is_some() || message.dismissed_at.is_some() =>
            {
                return true
            }
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return true,
            Err(RecvError::Closed) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_that_keeps_coming_back_is_logged_once_then_hourly() {
        let logged = LogEvery::default();
        let start = Instant::now();
        let at = |secs| start + std::time::Duration::from_secs(secs);
        assert!(logged.due("w1", at(0)));
        // Every 30 seconds for the hour: not again.
        assert!((1..120).all(|n| !logged.due("w1", at(n * 30))));
        // Another wake's failure is its own.
        assert!(logged.due("w2", at(30)));
        assert!(logged.due("w1", at(3600)));
        assert!(!logged.due("w1", at(3630)));
        // A wake that no longer fails is logged at once when it fails again.
        logged.forget("w1");
        assert!(logged.due("w1", at(3660)));
    }
}
