//! Context collections in a local-engine run (context library spec sections 3 to 5): the
//! first prompt lists them, the `context` tool reads them under the policy name `Context`, and
//! a run without collections is not offered the tool.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::tools::{self, context as context_tool};
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::library::{ContextConfig, LibraryConfig};
use dots_core::library_service::LibraryService;
use dots_core::model::{DotSpec, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Setup {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
    lib: LibraryService,
}

async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    std::fs::create_dir(dir.path().join("ws")).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
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
    let lib = LibraryService::new(store.clone(), bus);
    Setup {
        dir,
        store,
        runner,
        lib,
    }
}

impl Setup {
    /// Adds the collection `name` of the folder `folder` below the test's folder.
    async fn collection(&self, name: &str, folder: &str, every_dot: bool) {
        let path = self.dir.path().join(folder);
        std::fs::create_dir_all(&path).unwrap();
        let config = LibraryConfig::Context(ContextConfig {
            path: path.to_string_lossy().into_owned(),
            description: format!("About {name}"),
            every_dot,
        });
        self.lib.add(name, config, None).await.unwrap();
    }

    fn spec(&self, llm: &Llm) -> DotSpec {
        let mut s = spec("reader");
        s.workdir = self.dir.path().join("ws").to_string_lossy().into_owned();
        s.workspace_mode = WorkspaceMode::Folder;
        s.engine = EngineKind::Local;
        s.endpoint_url = Some(llm.endpoint());
        s.model = "test-model".into();
        s
    }

    async fn run(&self, s: &DotSpec, status: RunStatus) -> String {
        let dot = self.store.create_dot(s).await.unwrap();
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
async fn a_local_run_lists_its_collections_and_reads_them_with_the_context_tool() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    t.collection("everyone", "all", true).await;
    t.collection("unused", "other", false).await;
    write(
        &t.dir.path().join("kb/style.md"),
        "# House style\nShort sentences.\nNo jargon.",
    );
    write(&t.dir.path().join("all/hello.md"), "# Hello");
    let llm = Llm::start(vec![
        call("c1", "context", json!({"collection": "company"})),
        call(
            "c2",
            "context",
            json!({"collection": "company", "path": "style.md", "offset": 2}),
        ),
        call(
            "c3",
            "context",
            json!({"collection": "company", "query": "jargon"}),
        ),
        text("Read it."),
    ])
    .await;
    let mut s = t.spec(&llm);
    // Every preset allows `Context`, Read-only too.
    s.policy = Policy::preset(Preset::ReadOnly);
    s.uses.context = vec!["company".into()];
    let run = t.run(&s, RunStatus::Succeeded).await;

    let reqs = llm.requests();
    let prompt = messages(&reqs[0])[1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        prompt.ends_with(
            "\n## Shared context\n\nShared context you can read. Call the tool `context` with a \
             collection name to list its documents, `query` to search it, or `path` to read \
             one. Look here before you guess how the person works.\n\
             - company: About company (1 document)\n\
             - everyone: About everyone (1 document)\n"
        ),
        "{prompt}"
    );
    let names = tool_names(&reqs[0]);
    assert!(names.iter().any(|n| n == "context"), "{names:?}");
    assert!(!names.iter().any(|n| n == "skill"), "{names:?}");
    assert_eq!(
        tool_result(&reqs[1], "c1"),
        "- style.md — House style: Short sentences."
    );
    assert_eq!(tool_result(&reqs[2], "c2"), "Short sentences.\nNo jargon.");
    assert!(
        tool_result(&reqs[3], "c3").contains("  line 3: No jargon."),
        "{}",
        tool_result(&reqs[3], "c3")
    );
    let calls: Vec<Value> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_call")
        .map(|ev| ev.data)
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["tool"], "Context");
    assert_eq!(calls[0]["input"], json!({"collection": "company"}));
}

#[tokio::test]
async fn a_run_without_collections_is_not_offered_the_tool() {
    let t = setup().await;
    let llm = Llm::start(vec![text("Nothing to read.")]).await;
    t.run(&t.spec(&llm), RunStatus::Succeeded).await;
    let reqs = llm.requests();
    assert!(!tool_names(&reqs[0]).iter().any(|n| n == "context"));
    let prompt = messages(&reqs[0])[1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!prompt.contains("Shared context"), "{prompt}");
}

#[tokio::test]
async fn a_missing_folder_leaves_the_collection_out_with_a_notice() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    std::fs::remove_dir_all(t.dir.path().join("kb")).unwrap();
    let llm = Llm::start(vec![text("Done.")]).await;
    let mut s = t.spec(&llm);
    s.uses.context = vec!["company".into()];
    let run = t.run(&s, RunStatus::Succeeded).await;
    let notices: Vec<String> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "notice")
        .map(|ev| ev.data["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        notices.contains(&"context company: folder not found".to_string()),
        "{notices:?}"
    );
    assert!(!tool_names(&llm.requests()[0])
        .iter()
        .any(|n| n == "context"));
}

#[tokio::test]
async fn a_dots_own_context_rule_still_decides() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    write(&t.dir.path().join("kb/a.md"), "secret plans");
    let llm = Llm::start(vec![
        call(
            "c1",
            "context",
            json!({"collection": "company", "path": "a.md"}),
        ),
        text("Could not read it."),
    ])
    .await;
    let mut s = t.spec(&llm);
    s.policy = Policy::preset(Preset::Sandboxed);
    s.policy.rules.insert(0, Rule::new("Context", Action::Deny));
    s.uses.context = vec!["company".into()];
    t.run(&s, RunStatus::Succeeded).await;
    let result = tool_result(&llm.requests()[1], "c1");
    assert!(!result.contains("secret plans"), "{result}");
    assert!(result.to_lowercase().contains("denied"), "{result}");
}

#[tokio::test]
async fn a_context_rule_naming_a_collection_refuses_only_that_collection() {
    let t = setup().await;
    t.collection("company", "kb", false).await;
    t.collection("secret", "sec", false).await;
    write(&t.dir.path().join("kb/a.md"), "open plans");
    write(&t.dir.path().join("sec/b.md"), "hidden plans");
    let llm = Llm::start(vec![
        call(
            "c1",
            "context",
            json!({"collection": "secret", "path": "b.md"}),
        ),
        call(
            "c2",
            "context",
            json!({"collection": "company", "path": "a.md"}),
        ),
        text("Done."),
    ])
    .await;
    let mut s = t.spec(&llm);
    s.policy = Policy::preset(Preset::Sandboxed);
    s.policy
        .rules
        .insert(0, Rule::new("Context(secret)", Action::Deny));
    s.uses.context = vec!["company".into(), "secret".into()];
    t.run(&s, RunStatus::Succeeded).await;
    let reqs = llm.requests();
    let denied = tool_result(&reqs[1], "c1");
    assert!(!denied.contains("hidden plans"), "{denied}");
    assert!(denied.to_lowercase().contains("denied"), "{denied}");
    assert_eq!(tool_result(&reqs[2], "c2"), "open plans");
}

#[test]
fn context_is_a_local_tool_with_the_policy_name_context() {
    assert_eq!(tools::alias("context"), Some("Context"));
    assert_eq!(tools::local_name("Context"), Some("context"));
    let ws = tempfile::tempdir().unwrap();
    let prepared = tools::prepare(
        "context",
        &json!({"collection": "kb", "path": "a.md", "offset": 3, "extra": 1}),
        ws.path(),
    )
    .unwrap();
    assert_eq!(
        prepared.input,
        json!({"collection": "kb", "path": "a.md", "offset": 3})
    );
    let err = tools::prepare("context", &json!({"query": "x"}), ws.path()).unwrap_err();
    assert!(err.contains("'collection'"), "{err}");
    let err = tools::prepare(
        "context",
        &json!({"collection": "kb", "offset": "3"}),
        ws.path(),
    )
    .unwrap_err();
    assert!(err.contains("non-negative integer"), "{err}");
    let schema = context_tool::schema();
    assert_eq!(schema["function"]["name"], "context");
    assert_eq!(
        schema["function"]["parameters"]["required"],
        json!(["collection"])
    );
}
