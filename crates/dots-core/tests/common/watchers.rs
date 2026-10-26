//! A watchers service on a fake clock, with a runner that has no engine: an event's run stays
//! queued, so a test reads it from the store.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{Dot, EngineKind, Run, TriggerKind, Watcher, WatcherConfig};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::vault::{EntryKind, Vault};
use dots_core::watchers::{ActingTracker, Sources, WatcherDeps, Watchers};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

use super::clock::FakeClock;
use super::{spec, temp_store};

pub fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-23T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

pub fn after(secs: i64) -> DateTime<Utc> {
    t0() + TimeDelta::seconds(secs)
}

pub struct Harness {
    pub dir: TempDir,
    pub store: Store,
    pub bus: Bus,
    pub clock: Arc<FakeClock>,
    pub acting: Arc<ActingTracker>,
    pub vault: Vault,
    pub runner: Arc<Runner>,
    pub sources: Sources,
    pub watchers: Arc<Watchers>,
    pub events: Receiver<RuntimeEvent>,
    pub shutdown: CancellationToken,
}

impl Harness {
    pub async fn new() -> Harness {
        Harness::with_sources(Sources::default()).await
    }

    pub async fn with_sources(sources: Sources) -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let acting = ActingTracker::new(clock.clone());
        let vault = Vault::in_memory(store.clone());
        let runner = Runner::new(
            store.clone(),
            bus.clone(),
            HashMap::<EngineKind, Arc<dyn Engine>>::new(),
            RunnerConfig {
                max_concurrent: 1,
                mcp_url: String::new(),
                worktrees_dir: dir.path().join("wt"),
                cancel_grace: Duration::from_millis(100),
                plugins_dir: dir.path().join("plugins"),
            },
        );
        let events = bus.subscribe();
        let watchers = Harness::service(
            &dir, &store, &bus, &clock, &acting, &vault, &runner, &sources,
        );
        Harness {
            dir,
            store,
            bus,
            clock,
            acting,
            vault,
            runner,
            sources,
            watchers,
            events,
            shutdown: CancellationToken::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn service(
        dir: &TempDir,
        store: &Store,
        bus: &Bus,
        clock: &Arc<FakeClock>,
        acting: &Arc<ActingTracker>,
        vault: &Vault,
        runner: &Arc<Runner>,
        sources: &Sources,
    ) -> Arc<Watchers> {
        Watchers::new(WatcherDeps {
            store: store.clone(),
            bus: bus.clone(),
            runner: runner.clone(),
            vault: vault.clone(),
            clock: clock.clone(),
            acting: acting.clone(),
            data_dir: dir.path().join("data"),
            sources: sources.clone(),
        })
    }

    /// A new service on the same database, clock and data folder: Dots closed and started again.
    pub fn restart(&mut self) {
        self.shutdown.cancel();
        self.shutdown = CancellationToken::new();
        self.watchers = Harness::service(
            &self.dir,
            &self.store,
            &self.bus,
            &self.clock,
            &self.acting,
            &self.vault,
            &self.runner,
            &self.sources,
        );
    }

    /// Starts the service's loop (folder watches and polls run from here on).
    pub fn start(&self) {
        self.watchers.spawn(self.shutdown.clone());
    }

    pub async fn dot(&self, name: &str) -> Dot {
        self.store.create_dot(&spec(name)).await.unwrap()
    }

    pub async fn secret(&self, name: &str, value: &str) {
        self.vault
            .add(name, EntryKind::Secret, value)
            .await
            .unwrap();
    }

    pub async fn watcher(&self, dot: &Dot, name: &str, config: WatcherConfig) -> Watcher {
        self.watchers.add(&dot.id, name, config).await.unwrap()
    }

    /// A folder under the harness's temp folder, made now.
    pub fn folder(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// The dot's watcher runs, oldest first.
    pub async fn runs(&self, dot_id: &str) -> Vec<Run> {
        let mut runs: Vec<Run> = self
            .store
            .list_runs(Some(dot_id), 500)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.trigger == TriggerKind::Watcher)
            .collect();
        runs.reverse();
        runs
    }

    /// Waits (at most `secs` real seconds) until the dot has `n` watcher runs, and returns them.
    pub async fn wait_runs(&self, dot_id: &str, n: usize, secs: u64) -> Vec<Run> {
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let runs = self.runs(dot_id).await;
            if runs.len() >= n {
                return runs;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{} watcher run(s) after {secs} s, not {n}",
                runs.len()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits `secs` real seconds and checks the dot still has exactly `n` watcher runs.
    pub async fn still(&self, dot_id: &str, n: usize, secs: u64) -> Vec<Run> {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let runs = self.runs(dot_id).await;
        assert_eq!(runs.len(), n, "{runs:#?}");
        runs
    }

    /// The `WatcherPaused` events since the last call: (watcher id, error).
    pub fn paused(&mut self) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if let RuntimeEvent::WatcherPaused {
                watcher_id, error, ..
            } = event
            {
                out.push((watcher_id, error));
            }
        }
        out
    }

    pub fn path_text(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
