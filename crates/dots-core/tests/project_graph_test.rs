//! The project graph in a run (project graph spec sections 8 and 9): a dot with the graph on
//! gets the `graph` tool under the policy name `Graph` and, once its project has a graph, the
//! map in its first prompt; a dot with it off, or a policy that denies `Graph`, gets neither;
//! a run that ends asks for a rebuild.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::tools;
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::graph::{GraphState, Graphs, Project};
use dots_core::model::{DotSpec, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Setup {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
    graphs: Graphs,
}

fn ws(dir: &Path) -> std::path::PathBuf {
    dir.join("ws")
}

async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    let root = ws(dir.path());
    // A git checkout: only those get a graph on their own (a README comes with it).
    common::init_repo(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn start() { helper(); }\nfn helper() {}\n",
    )
    .unwrap();
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
            quiet_unit: Duration::from_secs(1),
        },
    );
    let graphs = Graphs::new(dir.path().join("graphs"), Some(bus));
    runner.set_graphs(graphs.clone());
    runner.spawn_dispatcher(CancellationToken::new());
    Setup {
        dir,
        store,
        runner,
        graphs,
    }
}

impl Setup {
    fn spec(&self, llm: &Llm) -> DotSpec {
        let mut s = spec("reader");
        s.workdir = ws(self.dir.path()).to_string_lossy().into_owned();
        s.workspace_mode = WorkspaceMode::Folder;
        s.engine = EngineKind::Local;
        s.endpoint_url = Some(llm.endpoint());
        s.model = "test-model".into();
        s
    }

    async fn project(&self) -> Project {
        Project::resolve(&ws(self.dir.path())).await.unwrap()
    }

    /// Builds the project's graph and waits for it.
    async fn built(&self) -> Project {
        let project = self.project().await;
        self.graphs.rebuild(&project);
        self.settled(&project).await;
        project
    }

    async fn settled(&self, project: &Project) {
        for _ in 0..500 {
            if !self.graphs.is_building(project) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the graph did not finish building");
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

fn prompt(req: &Value) -> String {
    messages(req)[1]["content"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_run_gets_the_map_and_asks_the_graph_tool() {
    let t = setup().await;
    t.built().await;
    let llm = Llm::start(vec![
        call(
            "g1",
            "graph",
            json!({"action": "callers", "node": "helper"}),
        ),
        call("g2", "graph", json!({"action": "fly"})),
        text("Done."),
    ])
    .await;
    let mut s = t.spec(&llm);
    // Every preset allows `Graph`, Read-only too.
    s.policy = Policy::preset(Preset::ReadOnly);
    let run = t.run(&s, RunStatus::Succeeded).await;

    let reqs = llm.requests();
    let first = prompt(&reqs[0]);
    assert!(first.contains("\n## Project graph\n"), "{first}");
    assert!(first.contains("Use the tool `graph`"), "{first}");
    assert!(first.contains("# Project map\n"), "{first}");
    assert!(tool_names(&reqs[0]).iter().any(|n| n == "graph"));
    let answer = tool_result(&reqs[1], "g1");
    assert!(answer.contains("src/lib.rs#start"), "{answer}");
    assert!(tool_result(&reqs[2], "g2").contains("unknown action"));
    let calls: Vec<Value> = t
        .store
        .list_events(&run, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_call")
        .map(|ev| ev.data)
        .collect();
    assert_eq!(calls[0]["tool"], "Graph");
    assert_eq!(
        calls[0]["input"],
        json!({"action": "callers", "node": "helper"})
    );
}

#[tokio::test]
async fn the_first_run_starts_the_build_without_waiting_and_a_run_that_ends_rebuilds() {
    let t = setup().await;
    let project = t.project().await;
    let llm = Llm::start(vec![text("Done."), text("Again.")]).await;
    let s = t.spec(&llm);
    t.run(&s, RunStatus::Succeeded).await;
    // No graph when it started (the build may have caught up by the prompt): the tool is
    // offered either way.
    let reqs = llm.requests();
    assert!(tool_names(&reqs[0]).iter().any(|n| n == "graph"));
    t.settled(&project).await;
    assert_eq!(t.graphs.status(&project).state, GraphState::Ready);
    assert_eq!(t.graphs.status(&project).files, 2);

    // A file the run might have written; the rebuild after the next run finds it.
    std::fs::write(ws(t.dir.path()).join("src/more.rs"), "pub fn more() {}\n").unwrap();
    let dot = t.store.list_dots().await.unwrap().remove(0);
    let r = t
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&t.store, &r.id, RunStatus::Succeeded, 20).await;
    for _ in 0..500 {
        if t.graphs.status(&project).files == 3 && !t.graphs.is_building(&project) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the run's end did not rebuild the graph");
}

#[tokio::test]
async fn a_dot_in_a_subfolder_sees_only_its_folder_and_no_file_its_policy_hides() {
    let t = setup().await;
    let root = ws(t.dir.path());
    std::fs::write(root.join("src/secret.rs"), "pub fn hidden_key() {}\n").unwrap();
    t.built().await;
    let llm = Llm::start(vec![
        call(
            "g1",
            "graph",
            json!({"action": "search", "query": "README"}),
        ),
        call(
            "g2",
            "graph",
            json!({"action": "search", "query": "hidden_key"}),
        ),
        call("g3", "graph", json!({"action": "search", "query": "start"})),
        text("Done."),
    ])
    .await;
    let mut s = t.spec(&llm);
    s.workdir = root.join("src").to_string_lossy().into_owned();
    s.policy
        .rules
        .push(Rule::new("Read(./secret.rs)", Action::Deny));
    t.run(&s, RunStatus::Succeeded).await;
    let reqs = llm.requests();
    let first = prompt(&reqs[0]);
    assert!(!first.contains("README"), "{first}");
    assert!(!first.contains("secret.rs"), "{first}");
    assert!(tool_result(&reqs[1], "g1").starts_with("nothing matches"));
    assert!(tool_result(&reqs[2], "g2").starts_with("nothing matches"));
    assert!(tool_result(&reqs[3], "g3").contains("src/lib.rs#start"));
}

#[tokio::test]
async fn the_switch_off_or_a_deny_rule_leaves_the_graph_out() {
    let t = setup().await;
    t.built().await;
    let llm = Llm::start(vec![text("One."), text("Two.")]).await;
    let mut off = t.spec(&llm);
    off.use_graph = false;
    t.run(&off, RunStatus::Succeeded).await;
    let mut denied = t.spec(&llm);
    denied.name = "denied".into();
    denied.policy.rules.push(Rule::new("Graph", Action::Deny));
    t.run(&denied, RunStatus::Succeeded).await;
    for req in llm.requests() {
        assert!(!tool_names(&req).iter().any(|n| n == "graph"));
        assert!(!prompt(&req).contains("## Project graph"));
    }
}

#[test]
fn every_preset_allows_graph_and_a_rule_decides_otherwise() {
    let input = json!({"action": "map"});
    for preset in [
        Preset::ReadOnly,
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::Custom,
    ] {
        let p = Policy::preset(preset);
        assert_eq!(
            p.resolve_in("Graph", &input, None),
            Action::Allow,
            "{preset:?}"
        );
    }
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.push(Rule::new("Graph", Action::Ask));
    assert_eq!(p.resolve_in("Graph", &input, None), Action::Ask);
    p.rules.push(Rule::new("Graph", Action::Deny));
    assert_eq!(p.resolve_in("Graph", &input, None), Action::Deny);
    // The CLI never sees a `Graph` rule.
    let (allow, deny) = p.cli_lists();
    assert!(!allow.iter().chain(&deny).any(|r| r == "Graph"));
}

#[test]
fn the_local_tool_keeps_only_its_own_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let call = tools::prepare(
        "graph",
        &json!({"action": "node", "node": "Runner", "depth": 2, "file_path": "x"}),
        dir.path(),
    )
    .unwrap();
    assert_eq!(call.alias, "Graph");
    assert_eq!(
        call.input,
        json!({"action": "node", "node": "Runner", "depth": 2})
    );
    assert!(tools::prepare("graph", &json!({}), dir.path()).is_err());
}

/// The smoke of `docs/verification/2026-11-10-project-graph-smoke.md`: builds the graph of the
/// folder in `GRAPH_SMOKE_ROOT` into a fresh file, then again after one file changes, and
/// prints the times, counts and map size. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn smoke_build_a_real_repository() {
    use dots_core::graph::build::{build, Limits};
    let Some(root) = std::env::var_os("GRAPH_SMOKE_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("smoke.sqlite");
    let quiet = |_: usize, _: usize| {};
    let first = build(&root, &db, None, Limits::default(), &quiet).unwrap();
    println!("first build: {first:?}");
    let conn = dots_core::graph::store::open(&db).unwrap();
    let (graph, maps, meta) = dots_core::graph::store::load(&conn).unwrap().unwrap();
    println!(
        "communities: {}, langs: {:?}, map: {} bytes, full map: {} bytes",
        graph.communities.len(),
        meta.langs,
        maps.prompt.len(),
        maps.full.len()
    );
    println!("--- map ---\n{}\n--- end ---", maps.prompt);
    let again = build(&root, &db, None, Limits::default(), &quiet).unwrap();
    println!("no change: {again:?}");
    let file = root.join(std::env::var("GRAPH_SMOKE_TOUCH").unwrap_or("README.md".into()));
    let before = std::fs::read(&file).unwrap();
    let mut changed = before.clone();
    changed.extend_from_slice(b"\n// smoke\n");
    std::fs::write(&file, &changed).unwrap();
    let touched = build(&root, &db, None, Limits::default(), &quiet);
    std::fs::write(&file, &before).unwrap();
    println!("one file changed: {:?}", touched.unwrap());
}

/// The real `claude` CLI asks the graph through Dots' `dots` MCP server: a Claude dot in a
/// small repository with a built graph, one tiny haiku run. Needs `CLAUDE_SMOKE` set to the
/// CLI's path; run with `--ignored --nocapture`. It spends a few cents of the person's plan.
#[tokio::test]
#[ignore]
async fn smoke_the_real_cli_calls_the_graph_tool() {
    use dots_core::runtime::{Config, Runtime};
    let Some(cli) = std::env::var_os("CLAUDE_SMOKE") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    common::init_repo(&repo);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn start() { helper(); }\nfn helper() {}\n",
    )
    .unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(std::path::PathBuf::from(cli));
    let rt = Runtime::start(cfg).await.unwrap();
    let mut s = spec("grapher");
    s.workdir = repo.to_string_lossy().into_owned();
    s.workspace_mode = WorkspaceMode::Folder;
    s.model = "haiku".into();
    s.max_turns = 4;
    s.instructions = "Call the tool mcp__dots__graph once with action \"callers\" and node \
        \"helper\". Then answer with one line: the node id it names. Use no other tool."
        .into();
    let dot = rt.create_dot(s, None, None).await.unwrap();
    rt.graph_rebuild(&dot.id).await.unwrap();
    for _ in 0..500 {
        if rt.graph_status(&dot.id).await.unwrap().state == GraphState::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let run = rt.run_now(&dot.id).await.unwrap();
    let mut done = None;
    for _ in 0..600 {
        let r = rt.get_run(&run.id).await.unwrap();
        if !matches!(r.status, RunStatus::Queued | RunStatus::Running) {
            done = Some(r);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let done = done.expect("the run did not end in two minutes");
    let events = rt.list_run_events(&run.id, 0).await.unwrap();
    for ev in &events {
        if matches!(
            ev.kind.as_str(),
            "tool_call" | "tool_result" | "assistant_text"
        ) {
            println!("{}: {}", ev.kind, ev.data);
        }
    }
    println!("status {:?}, summary {:?}", done.status, done.summary);
    let asked = events
        .iter()
        .any(|ev| ev.kind == "tool_call" && ev.data.to_string().contains("graph"));
    assert!(asked, "the CLI did not call the graph tool");
}
