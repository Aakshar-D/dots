//! Waking a maker (dots-make-dots design sections 7.2 and 8): a message Dots has for a maker's
//! chat topic, the person's answer to a make request or the end of a job, becomes one new run
//! of the maker in that topic. A wake is stored first and delivered by exactly one run. While
//! the topic is busy it waits, since nothing continues a thread that has not ended; the loop
//! here delivers it when a run ends, and once at start for what waited across a restart.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::service::Makers;
use crate::events::RuntimeEvent;
use crate::scheduler::ERROR_WAIT;
use crate::{Error, Result};

/// The key of a wake's run's payload that holds the wake's id: the run's message is Dots' own,
/// not one the person typed, as `follow_up_id` says of a follow-up's.
pub const WAKE_ID: &str = "wake_id";

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
        if let Err(e) = self.wake_pass().await {
            tracing::warn!(wake = %wake.id, "the wake waits; delivering it failed: {e}");
        }
        Ok(())
    }

    /// Delivers waiting wakes of every topic that is free now, oldest first, one per topic per
    /// pass. Called when a run ends, at startup, and by `wake`.
    ///
    /// Two passes at once deliver a wake once: each wake is tried under the thread lock, and
    /// its row is marked in the transaction that stores its run. A wake whose run could not be
    /// stored stays waiting, and the error of the first such wake is returned after the other
    /// topics were tried.
    pub async fn deliver_wakes(&self) -> Result<()> {
        self.wake_pass().await.map(|_| ())
    }

    /// One pass. `true` when a wake waits for something whose end no event tells (`Deciding`):
    /// the loop then tries again after a wait.
    async fn wake_pass(&self) -> Result<bool> {
        let mut tried_topics = HashSet::new();
        let mut again = false;
        let mut failed = None;
        for wake in self.parts.store.waiting_making_wakes().await? {
            match self.deliver_wake(&wake, &mut tried_topics).await {
                Ok(Tried::Deciding) => again = true,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(wake = %wake.id, "the wake's run could not be started: {e}");
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
                if p.store.drop_making_wake(&wake.id).await? {
                    tracing::warn!(
                        wake = %wake.id, maker = %wake.maker_dot_id, topic = %wake.root_run_id,
                        "a wake was dropped because its chat topic is gone"
                    );
                }
                return Ok(Tried::Done);
            }
            Err(e) => return Err(e),
        };
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
        let end = thread.last().unwrap_or(&first);
        let mut new = crate::checkins::continuation(&first, end, wake.message.clone(), None);
        if let Some(payload) = new.payload.as_mut() {
            payload[WAKE_ID] = wake.id.clone().into();
        }
        let Some(run) = p.store.deliver_making_wake(&wake.id, &new).await? else {
            return Ok(Tried::Done);
        };
        drop(one_at_a_time);
        tracing::info!(wake = %wake.id, run = %run.id, "a maker was woken");
        p.runner.enqueued(&run);
        Ok(Tried::Delivered)
    }

    /// Runs the wakes' loop until `shutdown`, or until the runtime that made the makers is
    /// gone: a pass at once, for the wakes that waited across a restart, then one after the end
    /// of each run, which is what frees a topic. After a pass that failed, or that left a wake
    /// waiting for a topic that is adding an app, the next comes after `ERROR_WAIT` at the
    /// latest. It does not keep the makers alive.
    pub(crate) fn spawn_wakes(&self, shutdown: CancellationToken) -> JoinHandle<()> {
        let makers = self.downgrade();
        // Subscribed before the first pass, so no end of a run is missed.
        let mut events = self.parts.bus.subscribe();
        tokio::spawn(async move {
            loop {
                let Some(this) = makers.upgrade() else { break };
                let again = match this.wake_pass().await {
                    Ok(again) => again,
                    Err(e) => {
                        tracing::error!("wake loop error: {e}");
                        true
                    }
                };
                drop(this);
                let wait = async {
                    if again {
                        tokio::time::sleep(ERROR_WAIT.to_std().unwrap_or_default()).await
                    } else {
                        std::future::pending().await
                    }
                };
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    open = run_ended(&mut events) => if !open { break },
                    _ = wait => {}
                }
            }
        })
    }
}

/// Returns `true` when a run has ended (its `RunUpdated`) or when events were lost and one may
/// have; `false` when no more events come.
async fn run_ended(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> bool {
    loop {
        match events.recv().await {
            Ok(RuntimeEvent::RunUpdated { run }) if run.status.is_terminal() => return true,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return true,
            Err(RecvError::Closed) => return false,
        }
    }
}
