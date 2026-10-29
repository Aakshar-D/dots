//! The check-in tools in a local-engine run (check-ins spec section 2): `notify`, `ask` and
//! `follow_up` are built in, checked under their policy names, and do what Dots' own MCP
//! server does for Claude runs.

mod common;
mod llm;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::checkins::{local_label, CheckIns, ASKED, ASKED_ALREADY, SENT};
use dots_core::engine::local::tools;
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::followups::FollowUps;
use dots_core::model::{
    DotSpec, EngineKind, MessageKind, NewRun, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::SystemClock;
use dots_core::store::Store;
use llm::{call, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Setup {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
}

/// A runner with the local engine, which has the check-in tools when `check_ins`.
async fn setup(check_ins: bool) -> Setup {
    let (dir, store) = temp_store().await;
    std::fs::create_dir(dir.path().join("ws")).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let mut engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    if check_ins {
        let follow_ups = FollowUps::new(store.clone(), bus.clone(), Arc::new(SystemClock));
        engine = engine.with_checkins(CheckIns::new(store.clone(), bus.clone(), follow_ups));
    }
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus,
        engines,
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Setup { dir, store, runner }
}

impl Setup {
    fn spec(&self, llm: &Llm) -> DotSpec {
        let mut s = spec("checker");
        s.workdir = self.dir.path().join("ws").to_string_lossy().into_owned();
        s.workspace_mode = WorkspaceMode::Folder;
        s.engine = EngineKind::Local;
        s.endpoint_url = Some(llm.endpoint());
        s.model = "test-model".into();
        s
    }

    async fn run(&self, s: &DotSpec, status: RunStatus) -> String {
        let dot = self.store.create_dot(s, None, "default").await.unwrap();
        let r = self
            .runner
            .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        wait_status(&self.store, &r.id, status, 20).await;
        r.id
    }
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_local_run_checks_in_with_its_built_in_tools() {
    let t = setup(true).await;
    let llm = Llm::start(vec![
        call("c1", "notify", json!({"text": "Half way there."})),
        call(
            "c2",
            "ask",
            json!({"question": "Ship it?", "choices": ["yes", "no"]}),
        ),
        call("c3", "ask", json!({"question": "Really?"})),
        call(
            "c4",
            "follow_up",
            json!({"note": "Check the build.", "in": "2h"}),
        ),
        call("c5", "notify", json!({"text": " "})),
        text("Asked."),
    ])
    .await;
    let run = t.run(&t.spec(&llm), RunStatus::Succeeded).await;

    let reqs = llm.requests();
    let names = tool_names(&reqs[0]);
    for name in ["notify", "ask", "follow_up"] {
        assert!(names.iter().any(|n| n == name), "{names:?}");
    }
    assert_eq!(tool_result(&reqs[1], "c1"), SENT);
    assert_eq!(tool_result(&reqs[2], "c2"), ASKED);
    assert_eq!(tool_result(&reqs[3], "c3"), ASKED_ALREADY);
    let pending = t.store.list_pending_follow_ups(None).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].from_run_id, run);
    let due = DateTime::parse_from_rfc3339(&pending[0].due_at)
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(
        tool_result(&reqs[4], "c4"),
        format!(
            "Follow-up {} scheduled for {}.",
            pending[0].id,
            local_label(due)
        )
    );
    assert_eq!(tool_result(&reqs[5], "c5"), "`text` must not be blank.");
    let messages = t.store.list_messages(None).await.unwrap();
    assert_eq!(messages.len(), 2);
    let question = messages
        .iter()
        .find(|m| m.kind == MessageKind::Question)
        .unwrap();
    assert_eq!(
        question.choices,
        Some(vec!["yes".to_string(), "no".to_string()])
    );
    // The transcript names each call by its policy name, as for every built-in tool.
    let calls: Vec<Value> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_call")
        .map(|ev| ev.data["tool"].clone())
        .collect();
    assert_eq!(
        calls,
        [
            json!("Notify"),
            json!("Ask"),
            json!("Ask"),
            json!("FollowUp"),
            json!("Notify")
        ]
    );
}

#[tokio::test]
async fn a_dots_own_follow_up_rule_still_decides_on_the_local_engine() {
    let t = setup(true).await;
    let llm = Llm::start(vec![
        call("c1", "follow_up", json!({"note": "Later.", "in": "1h"})),
        text("Could not."),
    ])
    .await;
    let mut s = t.spec(&llm);
    s.policy
        .rules
        .insert(0, Rule::new("FollowUp", Action::Deny));
    t.run(&s, RunStatus::Succeeded).await;
    let result = tool_result(&llm.requests()[1], "c1");
    assert!(result.to_lowercase().contains("denied"), "{result}");
    assert!(t
        .store
        .list_pending_follow_ups(None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_engine_without_check_ins_offers_none() {
    let t = setup(false).await;
    let llm = Llm::start(vec![text("Nothing to say.")]).await;
    t.run(&t.spec(&llm), RunStatus::Succeeded).await;
    let names = tool_names(&llm.requests()[0]);
    assert!(
        !names
            .iter()
            .any(|n| n == "notify" || n == "ask" || n == "follow_up"),
        "{names:?}"
    );
}

#[test]
fn the_check_in_tools_are_local_tools_with_their_policy_names() {
    assert_eq!(tools::alias("notify"), Some("Notify"));
    assert_eq!(tools::alias("ask"), Some("Ask"));
    assert_eq!(tools::alias("follow_up"), Some("FollowUp"));
    assert_eq!(tools::local_name("FollowUp"), Some("follow_up"));
    let ws = tempfile::tempdir().unwrap();
    let prepared = tools::prepare(
        "ask",
        &json!({"question": "Ship?", "choices": ["y"], "extra": 1}),
        ws.path(),
    )
    .unwrap();
    assert_eq!(
        prepared.input,
        json!({"question": "Ship?", "choices": ["y"]})
    );
    let err = tools::prepare(
        "ask",
        &json!({"question": "Ship?", "choices": "y"}),
        ws.path(),
    )
    .unwrap_err();
    assert!(err.contains("'choices' must be an array"), "{err}");
    let err = tools::prepare("follow_up", &json!({"in": "1h"}), ws.path()).unwrap_err();
    assert!(err.contains("'note'"), "{err}");
    let names: Vec<String> = tools::checkins::schemas()
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["notify", "ask", "follow_up"]);
}
