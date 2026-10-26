//! The person's side of Claude's usage limits (usage and limits spec sections 4 and 5): chat and
//! Run now refused while the limit is reached, **Run anyway**, held runs in the Inbox, a share
//! raised or a dot switched to a local model releasing what it held, and a reset that passes
//! without any new reading. Readings come from runs of the fake CLI, as they would from Claude.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveTime, TimeDelta, TimeZone, Utc};
use common::clock::FakeClock;
use common::{spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::limits::{HoldChange, NOT_HELD};
use dots_core::model::{Dot, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::util::timestamp;
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

/// Today at `h:m` local time.
fn local_today(h: u32, m: u32) -> DateTime<Utc> {
    let date = Local::now().date_naive();
    Local
        .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()))
        .unwrap()
        .with_timezone(&Utc)
}

struct T {
    dir: TempDir,
    rt: Runtime,
    clock: Arc<FakeClock>,
    script: PathBuf,
}

async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(local_today(12, 0));
    let script = dir.path().join("script.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script.to_string_lossy().to_string(),
    ));
    let t = T {
        rt: Runtime::start(cfg).await.unwrap(),
        dir,
        clock,
        script,
    };
    t.plays(None, 0);
    t
}

impl T {
    /// What the next runs of the fake CLI do: report `reading` (status, 5-hour utilization,
    /// reset), wait `sleep_ms`, then succeed.
    fn plays(&self, reading: Option<(&str, f64, DateTime<Utc>)>, sleep_ms: u64) {
        let mut steps: Vec<Value> =
            vec![json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})];
        if let Some((status, u, resets)) = reading {
            let at = resets.timestamp();
            steps.push(json!({"emit": {"type": "rate_limit_event", "rate_limit_info": {
                "status": status, "resetsAt": at, "rateLimitType": "five_hour",
                "unifiedWindows": {"five_hour": {"utilization": u, "resetsAt": at, "status": status}}
            }}}));
        }
        if sleep_ms > 0 {
            steps.push(json!({"sleep_ms": sleep_ms}));
        }
        steps.push(
            json!({"emit": {"type": "result", "subtype": "success", "is_error": false,
            "result": "done", "usage": {"input_tokens": 1, "output_tokens": 1}}}),
        );
        std::fs::write(&self.script, Value::Array(steps).to_string()).unwrap();
    }

    async fn dot(&self, name: &str) -> Dot {
        let work = self.dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut s = spec(name);
        s.workdir = work.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.rt.create_dot(s, None).await.unwrap()
    }

    /// A Run now of a fresh dot that brings `reading`, to its end.
    async fn read(&self, reading: (&str, f64, DateTime<Utc>)) {
        self.plays(Some(reading), 0);
        let dot = self
            .dot(&format!("reader-{}", (reading.1 * 100.0) as u32))
            .await;
        let run = self.rt.run_now(&dot.id).await.unwrap();
        wait_status(self.rt.store(), &run.id, RunStatus::Succeeded, 10).await;
        self.plays(None, 0);
    }

    async fn queue(&self, dot: &Dot, trigger: TriggerKind) -> String {
        let id = self
            .rt
            .store()
            .create_run(&NewRun::new(&dot.id, trigger))
            .await
            .unwrap()
            .id;
        tokio::time::sleep(Duration::from_millis(5)).await;
        id
    }

    async fn status(&self, id: &str) -> RunStatus {
        self.rt.store().get_run(id).await.unwrap().status
    }

    async fn runs(&self) -> usize {
        self.rt.store().list_runs(None, 100).await.unwrap().len()
    }
}

fn refusal(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

/// The dispatcher looks at the queue every 2 s at the latest.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(2600)).await;
}

#[tokio::test]
async fn chat_and_run_now_are_refused_under_the_limit() {
    let t = start().await;
    t.read(("rejected", 1.0, local_today(15, 10))).await;
    let dot = t.dot("a").await;
    let before = t.runs().await;
    let words = "Claude's limit is reached until 15:10.";
    assert_eq!(
        refusal(t.rt.chat(&dot.id, "hi", None).await.unwrap_err()),
        words
    );
    assert_eq!(refusal(t.rt.run_now(&dot.id).await.unwrap_err()), words);
    assert_eq!(t.runs().await, before);

    let mut s = spec("local");
    s.engine = EngineKind::Local;
    s.model = "m".into();
    s.endpoint_url = Some("http://127.0.0.1:1/v1".into());
    let local = t.rt.create_dot(s, None).await.unwrap();
    assert!(t.rt.chat(&local.id, "hi", None).await.is_ok());
}

#[tokio::test]
async fn run_anyway_starts_one_held_run() {
    let t = start().await;
    t.read(("allowed", 0.7, local_today(15, 10))).await;
    let (a, b) = (t.dot("a").await, t.dot("b").await);
    let first = t.queue(&a, TriggerKind::Schedule).await;
    let second = t.queue(&b, TriggerKind::Schedule).await;
    settle().await;
    assert_eq!(t.status(&first).await, RunStatus::Queued);

    let released = t.rt.run_release_hold(&first).await.unwrap();
    assert!(released.hold_released);
    wait_status(t.rt.store(), &first, RunStatus::Succeeded, 10).await;
    settle().await;
    assert_eq!(t.status(&second).await, RunStatus::Queued);

    let chat = t.dot("c").await;
    let mine = t.rt.run_now(&chat.id).await.unwrap();
    assert_eq!(
        refusal(t.rt.run_release_hold(&mine.id).await.unwrap_err()),
        NOT_HELD
    );
}

#[tokio::test]
async fn the_inbox_lists_held_runs_with_their_reset() {
    let t = start().await;
    let reset = local_today(15, 10);
    t.read(("allowed", 0.7, reset)).await;
    let a = t.dot("a").await;
    let held = t.queue(&a, TriggerKind::Watcher).await;
    settle().await;
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.held.len(), 1);
    assert_eq!(inbox.held[0].run_id, held);
    assert_eq!(inbox.held[0].dot_name, "a");
    assert_eq!(inbox.held[0].trigger, TriggerKind::Watcher);
    assert_eq!(
        inbox.held[0].starts_after.as_deref(),
        Some(timestamp(reset).as_str())
    );
    let limits = t.rt.limits_get().await.unwrap();
    assert!(limits.holding);
    assert_eq!(limits.held_run_ids, [held]);
}

#[tokio::test]
async fn raising_a_share_releases_held_runs_one_at_a_time() {
    let t = start().await;
    t.read(("allowed", 0.62, local_today(15, 10))).await;
    let mut events = t.rt.subscribe();
    t.plays(None, 1500);
    let (a, b) = (t.dot("a").await, t.dot("b").await);
    let first = t.queue(&a, TriggerKind::Schedule).await;
    let second = t.queue(&b, TriggerKind::Schedule).await;
    settle().await;
    assert_eq!(t.status(&first).await, RunStatus::Queued);

    let mut settings = t.rt.settings().await;
    settings.hold_share_five_hour = 65;
    t.rt.update_settings(settings).await.unwrap();
    wait_status(t.rt.store(), &first, RunStatus::Running, 5).await;
    assert_eq!(t.status(&second).await, RunStatus::Queued);
    wait_status(t.rt.store(), &first, RunStatus::Succeeded, 10).await;
    wait_status(t.rt.store(), &second, RunStatus::Succeeded, 10).await;

    let mut changes = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let RuntimeEvent::LimitsUpdated { change, .. } = ev {
            changes.extend(change);
        }
    }
    assert_eq!(
        changes,
        [
            HoldChange::Started {
                window: "five_hour".into(),
                utilization: 0.62
            },
            HoldChange::Ended
        ]
    );
}

#[tokio::test]
async fn a_held_run_of_a_dot_switched_to_local_starts() {
    let t = start().await;
    t.read(("allowed", 0.7, local_today(15, 10))).await;
    let a = t.dot("a").await;
    let held = t.queue(&a, TriggerKind::Schedule).await;
    settle().await;
    assert_eq!(t.status(&held).await, RunStatus::Queued);
    let mut s = a.spec.clone();
    s.engine = EngineKind::Local;
    s.model = "m".into();
    s.endpoint_url = Some("http://127.0.0.1:1/v1".into());
    t.rt.update_dot(&a.id, s).await.unwrap();
    settle().await;
    assert_ne!(t.status(&held).await, RunStatus::Queued);
}

#[tokio::test]
async fn a_reset_passing_releases_without_a_new_reading() {
    let t = start().await;
    let reset = local_today(15, 10);
    t.read(("allowed", 0.7, reset)).await;
    let a = t.dot("a").await;
    let held = t.queue(&a, TriggerKind::Schedule).await;
    settle().await;
    assert_eq!(t.status(&held).await, RunStatus::Queued);
    t.clock.set(reset + TimeDelta::seconds(1));
    wait_status(t.rt.store(), &held, RunStatus::Succeeded, 10).await;
}
