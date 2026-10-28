//! Apps waking dots (app library spec section 7): a run an app started is a topic of its own,
//! and when a run of that topic ends with nothing in the topic waiting (no run working or
//! parked, no open question, no pending review card), Dots calls the app's `on_run_end` as Dots.

use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use super::call::Actor;
use super::Apps;
use crate::events::{Bus, RuntimeEvent};
use crate::library::LibraryConfig;
use crate::model::Run;
use crate::{Error, Result};

/// A topic an app started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppTopic {
    pub root_run_id: String,
    pub app_id: String,
    pub dot_id: String,
    pub tag: String,
    pub created_at: String,
    /// The last run whose end the app was told of.
    pub told_run_id: Option<String>,
}

impl Apps {
    /// Tells the app that started `run`'s topic that the run ended, when nothing in the topic
    /// waits, once per run. A topic of an app that is gone, or that has no `on_run_end`, is
    /// left alone. The switch does not matter: a topic started before it went off still ends.
    pub async fn run_ended(&self, run: &Run) -> Result<()> {
        if !run.status.is_terminal() {
            return Ok(());
        }
        let Some(topic) = self.store.app_topic(&run.root_run_id).await? else {
            return Ok(());
        };
        if self.store.topic_busy(&topic.root_run_id).await? {
            return Ok(());
        }
        // Only the topic's newest run ends it: a review sends every run of the thread anew, and
        // an older run, or one told already, is not news.
        let thread = self.store.list_thread(&topic.root_run_id).await?;
        if thread.last().map(|r| r.id.as_str()) != Some(run.id.as_str()) {
            return Ok(());
        }
        if !self
            .store
            .mark_topic_told(&topic.root_run_id, &run.id)
            .await?
        {
            return Ok(());
        }
        let item = match self.store.get_library_item(&topic.app_id).await {
            Ok(item) => item,
            Err(Error::NotFound(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let LibraryConfig::App(config) = &item.config else {
            return Ok(());
        };
        let src = self.load(&item, config).await?;
        let Some(function) = src.manifest.on_run_end.clone() else {
            return Ok(());
        };
        let input = json!({
            "run_id": run.id,
            "root_run_id": run.root_run_id,
            "tag": topic.tag,
            "dot_id": topic.dot_id,
            "status": run.status.as_str(),
            "summary": run.summary,
            "error": run.error,
        });
        if let Err(e) = self.call(&item.name, &function, input, Actor::Dots).await {
            tracing::warn!(app = %item.name, run = %run.id, "on_run_end failed: {e}");
        }
        Ok(())
    }

    /// Tells the apps of the ends they missed (while Dots was not running, or when the bus
    /// lagged): each topic nothing waits in whose newest run ended and was not told.
    pub async fn catch_up(&self) -> Result<()> {
        self.store.prune_app_topics().await?;
        for topic in self.store.idle_app_topics().await? {
            let thread = self.store.list_thread(&topic.root_run_id).await?;
            let Some(newest) = thread.last() else {
                continue;
            };
            if newest.status.is_terminal()
                && topic.told_run_id.as_deref() != Some(newest.id.as_str())
            {
                if let Err(e) = self.run_ended(newest).await {
                    tracing::warn!(run = %newest.id, "telling the app of the run's end failed: {e}");
                }
            }
        }
        Ok(())
    }
}

/// Hears every run that ends and tells the apps (`Apps::run_ended`), until `shutdown`. A worker
/// tells them, one end at a time, so the bus is never held up by an app's code; it catches up
/// first (the ends of runs that stopped while Dots was not running) and after the bus lags.
pub fn spawn_run_ends(apps: Apps, bus: &Bus, shutdown: CancellationToken) {
    let mut rx = bus.subscribe();
    // `None` asks the worker to catch up.
    let (tell, mut ends) = tokio::sync::mpsc::unbounded_channel::<Option<Run>>();
    let stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                _ = stop.cancelled() => break,
                next = ends.recv() => next,
            };
            let told = match next {
                Some(Some(run)) => apps.run_ended(&run).await,
                Some(None) => apps.catch_up().await,
                None => break,
            };
            if let Err(e) = told {
                tracing::warn!("telling an app of a run's end failed: {e}");
            }
        }
    });
    let _ = tell.send(None);
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = shutdown.cancelled() => break,
                event = rx.recv() => event,
            };
            match event {
                Ok(RuntimeEvent::RunUpdated { run }) if run.status.is_terminal() => {
                    let _ = tell.send(Some(run));
                }
                // A follow-up kept a topic waiting: cancelled, skipped or fired into a thread of
                // its own, it no longer does, and no run of the topic ends to say so.
                Ok(RuntimeEvent::FollowUpsUpdated { .. }) => {
                    let _ = tell.send(None);
                }
                Ok(_) => {}
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("{n} events were missed; the apps catch up on the runs' ends");
                    let _ = tell.send(None);
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}
