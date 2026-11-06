//! Auto in the runner (auto model spec sections 4 and 5): the model a run starts with is
//! recorded with Auto's reason; a run whose CLI could not use Auto's pick, and did nothing yet,
//! starts once more with another tier's model; any other failure is left as it is.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{spec, temp_store, wait_status};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent, RunContext};
use dots_core::events::new_bus;
use dots_core::model::WorkspaceMode;
use dots_core::model::{Dot, EngineKind, ModelTier, NewRun, Run, RunStatus, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const NO_MODEL: &str = "There's an issue with the selected model (opus). It may not exist or \
    you may not have access to it. Run /model to pick a different model.";

fn failed(error: &str) -> Step {
    Step::Emit(EngineEvent::Failed {
        error: error.into(),
    })
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

/// An engine that plays the script of the model it is started with, and remembers each model.
struct ByModel {
    scripts: HashMap<String, Vec<Step>>,
    started: Mutex<Vec<String>>,
}

impl ByModel {
    fn new(scripts: &[(&str, Vec<Step>)]) -> Arc<Self> {
        Arc::new(Self {
            scripts: scripts
                .iter()
                .map(|(m, s)| (m.to_string(), s.clone()))
                .collect(),
            started: Mutex::new(Vec::new()),
        })
    }

    fn started(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }
}

#[async_trait]
impl Engine for ByModel {
    async fn start(&self, ctx: RunContext) -> dots_core::Result<mpsc::Receiver<EngineEvent>> {
        let model = ctx.dot.spec.model.clone();
        self.started.lock().unwrap().push(model.clone());
        let steps = self
            .scripts
            .get(&model)
            .cloned()
            .unwrap_or_else(|| vec![finished()]);
        ScriptedEngine::new(steps).start(ctx).await
    }
}

struct Env {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
}

async fn env(engine: Arc<ByModel>) -> Env {
    let (dir, store) = temp_store().await;
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine);
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
    async fn dot(&self, model: &str) -> Dot {
        let mut s = spec("auto");
        s.model = model.into();
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    async fn chat(&self, dot: &Dot, message: &str, parent: Option<&Run>) -> Run {
        let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
        new.payload = Some(json!({ "message": message }));
        if let Some(parent) = parent {
            new.parent_run_id = Some(parent.id.clone());
            new.root_run_id = Some(parent.root_run_id.clone());
        }
        let id = self.runner.enqueue(new).await.unwrap().id;
        self.store.get_run(&id).await.unwrap()
    }

    async fn kinds(&self, run: &str) -> Vec<String> {
        let events = self.store.list_events(run, 0).await.unwrap();
        events.into_iter().map(|e| e.kind).collect()
    }
}

#[tokio::test]
async fn a_run_records_auto_s_pick_and_why() {
    let engine = ByModel::new(&[]);
    let e = env(engine.clone()).await;
    let dot = e.dot("auto").await;
    let run = e.chat(&dot, "Summarise the meeting notes", None).await;
    let done = wait_status(&e.store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(done.model.as_deref(), Some("haiku"));
    assert_eq!(done.model_tier, Some(ModelTier::Small));
    assert_eq!(done.model_reason.as_deref(), Some("light task: summarise"));
    assert_eq!(done.model_fallback_from, None);
    assert_eq!(engine.started(), ["haiku"]);
}

#[tokio::test]
async fn a_dot_with_its_own_model_records_it_without_a_reason() {
    let engine = ByModel::new(&[]);
    let e = env(engine.clone()).await;
    let dot = e.dot("opus").await;
    let run = e.chat(&dot, "Summarise the meeting notes", None).await;
    let done = wait_status(&e.store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(done.model.as_deref(), Some("opus"));
    assert_eq!((done.model_tier, done.model_reason), (None, None));
    assert_eq!(engine.started(), ["opus"]);
}

#[tokio::test]
async fn a_model_the_cli_cannot_use_is_tried_once_more_with_the_next_tier_down() {
    let engine = ByModel::new(&[("opus", vec![failed(NO_MODEL)])]);
    let e = env(engine.clone()).await;
    let dot = e.dot("auto").await;
    let run = e.chat(&dot, "use opus for this", None).await;
    let done = wait_status(&e.store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(engine.started(), ["opus", "sonnet"]);
    assert_eq!(done.model.as_deref(), Some("sonnet"));
    assert_eq!(done.model_tier, Some(ModelTier::Standard));
    assert_eq!(done.model_reason.as_deref(), Some("you asked for Opus"));
    assert_eq!(done.model_fallback_from.as_deref(), Some("opus"));
    assert_eq!(done.error, None);
    let kinds = e.kinds(&run.id).await;
    assert!(!kinds.contains(&"failed".to_string()), "{kinds:?}");
    let events = e.store.list_events(&run.id, 0).await.unwrap();
    let notice = events
        .iter()
        .find(|ev| ev.kind == "notice")
        .expect("a notice");
    let text = notice.data["text"].as_str().unwrap();
    assert!(
        text.starts_with("Opus could not be used") && text.ends_with("Trying again with Sonnet."),
        "{text}"
    );
}

#[tokio::test]
async fn the_fallback_is_tried_only_once() {
    let engine = ByModel::new(&[
        ("opus", vec![failed(NO_MODEL)]),
        ("sonnet", vec![failed("overloaded_error: Overloaded")]),
    ]);
    let e = env(engine.clone()).await;
    let dot = e.dot("auto").await;
    let run = e.chat(&dot, "use opus", None).await;
    let done = wait_status(&e.store, &run.id, RunStatus::Failed, 10).await;
    assert_eq!(engine.started(), ["opus", "sonnet"]);
    assert_eq!(done.error.as_deref(), Some("overloaded_error: Overloaded"));
}

#[tokio::test]
async fn a_run_that_already_used_a_tool_is_not_started_again() {
    let call = Step::Emit(EngineEvent::ToolCall {
        id: "t1".into(),
        tool: "Read".into(),
        input: json!({"file_path": "a.txt"}),
    });
    let engine = ByModel::new(&[("opus", vec![call, failed(NO_MODEL)])]);
    let e = env(engine.clone()).await;
    let dot = e.dot("auto").await;
    let run = e.chat(&dot, "use opus", None).await;
    let done = wait_status(&e.store, &run.id, RunStatus::Failed, 10).await;
    assert_eq!(engine.started(), ["opus"]);
    assert_eq!(done.model_fallback_from, None);
}

#[tokio::test]
async fn other_failures_and_fixed_models_are_not_started_again() {
    let engine = ByModel::new(&[("opus", vec![failed("error_max_turns")])]);
    let e = env(engine.clone()).await;
    let dot = e.dot("auto").await;
    let run = e.chat(&dot, "use opus", None).await;
    wait_status(&e.store, &run.id, RunStatus::Failed, 10).await;
    assert_eq!(engine.started(), ["opus"]);

    let engine = ByModel::new(&[("opus", vec![failed(NO_MODEL)])]);
    let e = env(engine.clone()).await;
    let dot = e.dot("opus").await;
    let run = e.chat(&dot, "Refactor it", None).await;
    let done = wait_status(&e.store, &run.id, RunStatus::Failed, 10).await;
    assert_eq!(engine.started(), ["opus"]);
    assert_eq!(done.error.as_deref(), Some(NO_MODEL));
}

#[tokio::test]
async fn a_conversation_keeps_its_model_for_a_light_reply() {
    let engine = ByModel::new(&[]);
    let e = env(engine.clone()).await;
    let dot = e.dot("auto").await;
    let first = e.chat(&dot, "use opus and plan the launch", None).await;
    let first = wait_status(&e.store, &first.id, RunStatus::Succeeded, 10).await;
    assert_eq!(first.model.as_deref(), Some("opus"));
    let reply = e.chat(&dot, "thanks!", Some(&first)).await;
    let reply = wait_status(&e.store, &reply.id, RunStatus::Succeeded, 10).await;
    assert_eq!(reply.model.as_deref(), Some("opus"));
    assert_eq!(
        reply.model_reason.as_deref(),
        Some("kept from earlier in this conversation")
    );
    assert_eq!(engine.started(), ["opus", "opus"]);
}
