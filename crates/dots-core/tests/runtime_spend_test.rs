//! The person's side of the spending caps (spend caps design): a chat at its dot's cap waits
//! (it is not refused), shows in the Inbox with the cap named, and **Run once anyway** starts it,
//! noted on the run; a background run waits and has no Run once anyway; today's spend reads
//! each dot against its caps. The runs are the fake CLI's, which reports a cost as Claude does.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use common::{spec, wait_status};
use dots_core::model::{Dot, HoldReason, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::spend::{CapScope, DotCaps, BACKGROUND_WAITS, NOT_CAPPED};
use dots_core::util::timestamp;
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

struct T {
    dir: TempDir,
    rt: Runtime,
}

async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    let steps = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
            "total_cost_usd": 0.25, "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    std::fs::write(
        &script,
        Value::Array(steps.as_array().unwrap().clone()).to_string(),
    )
    .unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script.to_string_lossy().to_string(),
    ));
    T {
        rt: Runtime::start(cfg).await.unwrap(),
        dir,
    }
}

impl T {
    async fn dot(&self, name: &str, daily_usd: Option<f64>) -> Dot {
        let work = self.dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut s = spec(name);
        s.workdir = work.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.spend_cap = DotCaps {
            daily_usd,
            daily_tokens: None,
        };
        self.rt.create_dot(s, None, None).await.unwrap()
    }

    /// A finished run of `dot` a few seconds ago that cost `cost`.
    async fn spent(&self, dot: &Dot, cost: f64) {
        let store = self.rt.store();
        let r = store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
            .await
            .unwrap();
        let at = timestamp(Utc::now() - TimeDelta::seconds(5));
        store.set_run_times(&r.id, &at, Some(&at)).await.unwrap();
        store
            .finish_run(&r.id, RunStatus::Succeeded, Some("ok"), None)
            .await
            .unwrap();
        store
            .add_usage(&r.id, 1000, 100, 0, Some(cost))
            .await
            .unwrap();
    }

    async fn status(&self, id: &str) -> RunStatus {
        self.rt.store().get_run(id).await.unwrap().status
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
async fn a_chat_at_its_cap_waits_and_can_run_once_anyway() {
    let t = start().await;
    let scout = t.dot("Scout", Some(2.0)).await;
    t.spent(&scout, 2.0).await;

    // Waits, not refused.
    let chat = t.rt.chat(&scout.id, "hi", None).await.unwrap();
    settle().await;
    assert_eq!(t.status(&chat.id).await, RunStatus::Queued);
    let run = t.rt.get_run(&chat.id).await.unwrap();
    assert_eq!(
        run.wait_detail.as_deref(),
        Some("Scout reached its $2.00 daily cap")
    );

    // The Inbox lists it with the cap.
    let inbox = t.rt.inbox().await.unwrap();
    let held = inbox.held.iter().find(|h| h.run_id == chat.id).unwrap();
    assert_eq!(
        held.reason,
        HoldReason::SpendCap {
            text: "Scout reached its $2.00 daily cap".into(),
            scope: CapScope::Dot,
        }
    );

    // Today's spend shows the dot at its cap.
    let today = t.rt.spend_today().await.unwrap();
    let row = today.dots.iter().find(|d| d.dot_id == scout.id).unwrap();
    assert!((row.today.cost_usd - 2.0).abs() < 1e-9);
    assert_eq!(row.caps.daily_usd, Some(2.0));
    assert!((today.today.cost_usd - 2.0).abs() < 1e-9);

    // Run once anyway: it starts, and the release is noted on it.
    let released = t.rt.run_release_cap(&chat.id).await.unwrap();
    assert!(released.cap_released);
    let done = wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 10).await;
    assert!(done.cap_released);
    let events = t.rt.store().list_events(&chat.id, 0).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "notice"
        && e.data["text"] == "Run once past the cap: Scout reached its $2.00 daily cap"));
    // No longer waiting: nothing to release.
    assert_eq!(
        refusal(t.rt.run_release_cap(&chat.id).await.unwrap_err()),
        NOT_CAPPED
    );
}

#[tokio::test]
async fn a_background_run_at_its_cap_just_waits() {
    let t = start().await;
    let scout = t.dot("Scout", Some(1.0)).await;
    t.spent(&scout, 1.5).await;
    let id =
        t.rt.store()
            .create_run(&NewRun::new(&scout.id, TriggerKind::Schedule))
            .await
            .unwrap()
            .id;
    settle().await;
    assert_eq!(t.status(&id).await, RunStatus::Queued);
    assert_eq!(
        refusal(t.rt.run_release_cap(&id).await.unwrap_err()),
        BACKGROUND_WAITS
    );

    // Raised: it starts.
    let mut s = scout.spec.clone();
    s.spend_cap.daily_usd = None;
    t.rt.update_dot(&scout.id, s).await.unwrap();
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 10).await;
}
