//! The runner and the spending caps (spend caps design): a run whose cap is reached waits,
//! queued, with the cap named, and starts once the cap is raised or the person lets it run once;
//! yesterday's spend holds nothing back; a run whose own usage reaches the cap is stopped, or
//! ends as it was ending when that usage came with its end.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use common::{spec, temp_store, wait_status};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::new_bus;
use dots_core::model::{
    Dot, EngineKind, NewRun, RunStatus, TriggerKind, WaitReason, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::settings::Settings;
use dots_core::spend::{day_bounds_in, DotCaps};
use dots_core::store::Store;
use dots_core::util::timestamp;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn usage(cost: f64) -> Step {
    Step::Emit(EngineEvent::Usage {
        tokens_in: 100,
        tokens_out: 10,
        cache_read: 0,
        cost_usd: Some(cost),
    })
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

struct Env {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
}

async fn env(steps: Vec<Step>) -> Env {
    let (dir, store) = temp_store().await;
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(ScriptedEngine::new(steps)));
    let runner = Runner::new(
        store.clone(),
        new_bus(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env { dir, store, runner }
}

impl Env {
    async fn dot(&self, name: &str, daily_usd: Option<f64>) -> Dot {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.spend_cap = DotCaps {
            daily_usd,
            daily_tokens: None,
        };
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    /// A finished run of `dot` started `ago` before now that cost `cost`.
    async fn spent(&self, dot: &Dot, ago: TimeDelta, cost: f64) {
        // The queue is held while the run is made and ended: a dispatch pass in between would
        // run it.
        self.runner.set_held(true).await;
        let r = self
            .store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
            .await
            .unwrap();
        let at = Utc::now() - ago;
        self.store
            .set_run_times(&r.id, &timestamp(at), Some(&timestamp(at)))
            .await
            .unwrap();
        self.store
            .finish_run(&r.id, RunStatus::Succeeded, Some("ok"), None)
            .await
            .unwrap();
        self.store
            .add_usage(&r.id, 1000, 100, 0, Some(cost))
            .await
            .unwrap();
        self.runner.set_held(false).await;
    }

    async fn queue(&self, dot: &Dot, trigger: TriggerKind) -> String {
        self.runner
            .enqueue(NewRun::new(&dot.id, trigger))
            .await
            .unwrap()
            .id
    }

    async fn settle(&self) {
        self.runner.nudge();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    async fn status(&self, id: &str) -> RunStatus {
        self.store.get_run(id).await.unwrap().status
    }
}

#[tokio::test]
async fn a_run_waits_at_its_dots_cap_and_starts_once_it_is_raised() {
    let e = env(vec![finished()]).await;
    let scout = e.dot("Scout", Some(2.0)).await;
    e.spent(&scout, TimeDelta::seconds(5), 2.0).await;
    let id = e.queue(&scout, TriggerKind::Schedule).await;
    e.settle().await;
    assert_eq!(e.status(&id).await, RunStatus::Queued);

    // It says why it waits.
    let mut runs = vec![e.store.get_run(&id).await.unwrap()];
    e.runner.fill_wait_reasons(&mut runs).await.unwrap();
    assert_eq!(runs[0].wait_reason, Some(WaitReason::SpendCap));
    assert_eq!(
        runs[0].wait_detail.as_deref(),
        Some("Scout reached its $2.00 daily cap")
    );
    let hold = e.runner.cap_hold(&runs[0]).await.unwrap().unwrap();
    assert_eq!(hold.text, "Scout reached its $2.00 daily cap");

    // A dot without a cap is not held by Scout's.
    let other = e.dot("Other", None).await;
    let free = e.queue(&other, TriggerKind::Schedule).await;
    wait_status(&e.store, &free, RunStatus::Succeeded, 5).await;
    assert_eq!(e.status(&id).await, RunStatus::Queued);

    // Raised: it starts.
    let mut s = scout.spec.clone();
    s.spend_cap.daily_usd = Some(5.0);
    e.store.update_dot(&scout.id, &s).await.unwrap();
    e.runner.nudge();
    wait_status(&e.store, &id, RunStatus::Succeeded, 5).await;
}

#[tokio::test]
async fn yesterdays_spend_holds_nothing_back() {
    let e = env(vec![finished()]).await;
    let scout = e.dot("Scout", Some(1.0)).await;
    // Started a minute before local midnight today.
    let (start, _) = day_bounds_in(Utc::now(), &chrono::Local);
    let ago = Utc::now()
        - chrono::DateTime::parse_from_rfc3339(&start)
            .unwrap()
            .with_timezone(&Utc)
        + TimeDelta::minutes(1);
    e.spent(&scout, ago, 5.0).await;
    let id = e.queue(&scout, TriggerKind::Schedule).await;
    wait_status(&e.store, &id, RunStatus::Succeeded, 5).await;
}

#[tokio::test]
async fn the_cap_for_every_dot_holds_each_dot() {
    let e = env(vec![finished()]).await;
    e.store
        .put_settings(&Settings {
            spend_daily_usd: Some(3.0),
            ..Settings::default()
        })
        .await
        .unwrap();
    let (a, b) = (e.dot("a", None).await, e.dot("b", None).await);
    e.spent(&a, TimeDelta::seconds(5), 1.5).await;
    e.spent(&b, TimeDelta::seconds(5), 1.5).await;
    let id = e.queue(&a, TriggerKind::Manual).await;
    e.settle().await;
    assert_eq!(e.status(&id).await, RunStatus::Queued);
    let run = e.store.get_run(&id).await.unwrap();
    assert_eq!(
        e.runner.cap_hold(&run).await.unwrap().unwrap().text,
        "All dots reached the $3.00 daily cap"
    );

    // Cleared: it starts.
    e.store.put_settings(&Settings::default()).await.unwrap();
    e.runner.nudge();
    wait_status(&e.store, &id, RunStatus::Succeeded, 5).await;
}

#[tokio::test]
async fn a_run_let_past_the_cap_starts_and_is_not_stopped() {
    let e = env(vec![usage(1.0), finished()]).await;
    let scout = e.dot("Scout", Some(1.0)).await;
    e.spent(&scout, TimeDelta::seconds(5), 1.0).await;
    let id = e.queue(&scout, TriggerKind::Chat).await;
    e.settle().await;
    assert_eq!(e.status(&id).await, RunStatus::Queued);
    let released = e.store.set_cap_released(&id).await.unwrap();
    assert!(released.cap_released);
    e.runner.nudge();
    let run = wait_status(&e.store, &id, RunStatus::Succeeded, 5).await;
    assert!(run.error.is_none());
    // Not queued any more: it cannot be released again.
    assert!(e.store.set_cap_released(&id).await.is_err());
}

#[tokio::test]
async fn a_run_whose_usage_reaches_the_cap_is_stopped() {
    let e = env(vec![usage(0.6), usage(0.6), Step::WaitForCancel]).await;
    let scout = e.dot("Scout", Some(1.0)).await;
    let id = e.queue(&scout, TriggerKind::Schedule).await;
    let run = wait_status(&e.store, &id, RunStatus::Failed, 5).await;
    assert_eq!(
        run.error.as_deref(),
        Some("stopped: Scout reached its $1.00 daily cap")
    );
    assert!((run.cost_usd.unwrap() - 1.2).abs() < 1e-9);
    // The next run waits.
    let next = e.queue(&scout, TriggerKind::Schedule).await;
    e.settle().await;
    assert_eq!(e.status(&next).await, RunStatus::Queued);
}

#[tokio::test]
async fn usage_that_reaches_the_cap_with_the_runs_end_lets_it_finish() {
    let e = env(vec![usage(1.5), finished()]).await;
    let scout = e.dot("Scout", Some(1.0)).await;
    let id = e.queue(&scout, TriggerKind::Chat).await;
    let run = wait_status(&e.store, &id, RunStatus::Succeeded, 5).await;
    assert!(run.error.is_none());
    let events = e.store.list_events(&id, 0).await.unwrap();
    assert!(
        events.iter().any(|ev| ev.kind == "notice"
            && ev.data["text"] == "Scout reached its $1.00 daily cap: the next run waits"),
        "{events:#?}"
    );
}
