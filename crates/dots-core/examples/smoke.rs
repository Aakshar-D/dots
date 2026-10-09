//! Manual end-to-end check against the real `claude` CLI (uses your subscription) or, with
//! `--engine local --endpoint <url>`, a local OpenAI-compatible server (LM Studio, Ollama).
//!
//! cargo run -p dots-core --example smoke -- <workdir> "<instructions>"
//!     [--model haiku] [--wait <approval_wait_secs>] [--preset read-only|sandboxed|trusted] [--folder]
//!     [--engine claude|local] [--endpoint http://127.0.0.1:1234]

use std::time::Duration;

use anyhow::{bail, Context};
use dots_core::events::RuntimeEvent;
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::policy::{Policy, Preset};
use dots_core::{Config, Runtime};
use tokio::io::{AsyncBufReadExt, BufReader};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(6)..]
}

fn preview(v: &serde_json::Value) -> String {
    let s = v.to_string();
    match s.char_indices().nth(160) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        bail!(
            "usage: smoke <workdir> <instructions> [--model m] [--wait secs] [--preset p]              [--folder] [--engine claude|local] [--endpoint url]"
        );
    }
    let mut cfg = Config::new(std::env::temp_dir().join("dots-smoke"));
    cfg.port = 0;
    let rt = Runtime::start(cfg).await?;
    println!("claude: {:?}\nport: {}", rt.claude_program(), rt.port());

    let name = format!("smoke-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let mut spec = DotSpec::new(&name, &args[1], &args[0]);
    spec.model = flag(&args, "--model").unwrap_or_else(|| "haiku".into());
    if let Some(engine) = flag(&args, "--engine") {
        spec.engine = EngineKind::parse(&engine)?;
    }
    spec.endpoint_url = flag(&args, "--endpoint");
    if let Some(endpoint) = &spec.endpoint_url {
        println!("probe: {}", rt.test_endpoint(endpoint, &spec.model).await?);
    }
    if let Some(w) = flag(&args, "--wait") {
        spec.approval_wait_secs = w.parse().context("--wait must be a number of seconds")?;
    }
    if let Some(p) = flag(&args, "--preset") {
        spec.policy = Policy::preset(Preset::parse(&p)?);
    }
    if args.iter().any(|a| a == "--folder") {
        spec.workspace_mode = WorkspaceMode::Folder;
    }
    let dot = rt.create_dot(spec).await?;
    let mut events = rt.subscribe();
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    let first = rt.run_now(&dot.id).await?;
    println!("run {} queued", first.id);

    loop {
        match events.recv().await? {
            RuntimeEvent::RunEvent { event } => {
                println!(
                    "  [{}#{}] {} {}",
                    short(&event.run_id),
                    event.seq,
                    event.kind,
                    preview(&event.data)
                )
            }
            RuntimeEvent::ApprovalRequested { approval } => {
                println!(
                    "\nAPPROVAL {} — {} {}\n  answer: y | n [note]",
                    approval.id, approval.tool, approval.input
                );
                let line = stdin.next_line().await?.unwrap_or_default();
                let line = line.trim();
                let (approved, note) = match line.split_once(' ') {
                    Some((a, n)) => (a == "y", Some(n.to_string())),
                    None => (line == "y", None),
                };
                rt.decide_approval(&approval.id, approved, note).await?;
            }
            RuntimeEvent::RunUpdated { run } if run.dot_id == dot.id => {
                let err = run
                    .error
                    .as_deref()
                    .map(|e| format!(" ({e})"))
                    .unwrap_or_default();
                println!("run {} -> {}{err}", short(&run.id), run.status.as_str());
                let runs = rt.store().list_runs(Some(&dot.id), 50).await?;
                if run.status != RunStatus::Queued && runs.iter().all(|r| r.status.is_terminal()) {
                    for r in &runs {
                        println!(
                            "\n{} {} {}\n  summary: {}\n  workspace: {}",
                            short(&r.id),
                            r.trigger.as_str(),
                            r.status.as_str(),
                            r.summary.as_deref().unwrap_or("-"),
                            r.workspace_path.as_deref().unwrap_or("-")
                        );
                    }
                    break;
                }
            }
            _ => {}
        }
    }
    rt.shutdown();
    tokio::time::sleep(Duration::from_millis(200)).await;
    Ok(())
}
