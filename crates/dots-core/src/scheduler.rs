use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Local};
use serde_json::json;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::model::{Dot, NewRun, Run, TriggerKind};
use crate::runner::Runner;
use crate::store::Store;
use crate::{Error, Result};

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
pub fn parse_cron(expr: &str) -> Result<cron::Schedule> {
    let fields = expr.split_whitespace().count();
    if !(6..=7).contains(&fields) {
        return Err(Error::Invalid(format!(
            "cron {expr:?}: expected 6 fields (sec min hour day month weekday) or 7 with year"
        )));
    }
    cron::Schedule::from_str(expr).map_err(|e| Error::Invalid(format!("cron {expr:?}: {e}")))
}

/// Next fire time strictly after `after`, in local time.
pub fn next_fire(expr: &str, after: DateTime<Local>) -> Result<Option<DateTime<Local>>> {
    Ok(parse_cron(expr)?.after(&after).next())
}

pub struct Scheduler {
    store: Store,
    runner: Arc<Runner>,
    reload: Notify,
    last_fired: Mutex<Option<DateTime<Local>>>,
}

impl Scheduler {
    pub fn new(store: Store, runner: Arc<Runner>) -> Arc<Scheduler> {
        Arc::new(Scheduler {
            store,
            runner,
            reload: Notify::new(),
            last_fired: Mutex::new(None),
        })
    }

    /// Call after any dot create/update/delete/enable change.
    pub fn reload(&self) {
        self.reload.notify_one();
    }

    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    r = this.tick() => {
                        if let Err(e) = r {
                            tracing::error!("scheduler error: {e}");
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                                _ = shutdown.cancelled() => break,
                            }
                        }
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        })
    }

    async fn tick(&self) -> Result<()> {
        let now = Local::now();
        let from = match *self.last_fired.lock().unwrap() {
            Some(last) if last > now => last,
            _ => now,
        };
        let mut due: Vec<(DateTime<Local>, Dot)> = Vec::new();
        for dot in self.store.list_dots().await? {
            let Some(expr) = dot.spec.schedule.clone() else {
                continue;
            };
            if !dot.spec.enabled {
                continue;
            }
            match next_fire(&expr, from) {
                Ok(Some(t)) => due.push((t, dot)),
                Ok(None) => {}
                Err(e) => tracing::warn!(dot = %dot.spec.name, "bad schedule: {e}"),
            }
        }
        let Some(next) = due.iter().map(|(t, _)| *t).min() else {
            self.reload.notified().await;
            return Ok(());
        };
        let wait = (next - Local::now()).to_std().unwrap_or_default();
        tokio::select! {
            _ = tokio::time::sleep(wait) => {
                *self.last_fired.lock().unwrap() = Some(next);
                for (_, dot) in due.iter().filter(|(t, _)| *t == next) {
                    self.fire(dot, next).await?;
                }
            }
            _ = self.reload.notified() => {}
        }
        Ok(())
    }

    /// Enqueues a scheduled run; `None` when coalesced into an already queued run.
    pub async fn fire(&self, dot: &Dot, at: DateTime<Local>) -> Result<Option<Run>> {
        if self.store.has_queued_run(&dot.id).await? {
            tracing::info!(dot = %dot.spec.name, "schedule tick coalesced into the queued run");
            return Ok(None);
        }
        let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
        new.payload = Some(json!({ "scheduled_for": at.to_rfc3339() }));
        Ok(Some(self.runner.enqueue(new).await?))
    }
}
