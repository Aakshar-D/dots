#![allow(dead_code)]

use std::path::Path;

use dots_core::model::{Dot, DotSpec, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::store::Store;
use serde_json::Value;
use tempfile::TempDir;

pub async fn temp_store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    (dir, store)
}

pub fn spec(name: &str) -> DotSpec {
    DotSpec::new(name, "Do the thing.", "C:/tmp/repo")
}

pub fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Creates a repo on branch `main` with one commit; hooks and signing disabled.
pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git_out(dir, &["init", "-q", "-b", "main"]);
    git_out(dir, &["config", "user.email", "dots@example.com"]);
    git_out(dir, &["config", "user.name", "dots"]);
    git_out(dir, &["config", "commit.gpgsign", "false"]);
    git_out(dir, &["config", "core.hooksPath", ".git/no-hooks"]);
    std::fs::write(
        dir.join("README.md"),
        "hello
",
    )
    .unwrap();
    git_out(dir, &["add", "."]);
    git_out(dir, &["commit", "-q", "-m", "init"]);
}

pub fn fake_dot(name: &str, workdir: &Path, mode: WorkspaceMode) -> Dot {
    let mut s = spec(name);
    s.workdir = workdir.to_string_lossy().to_string();
    s.workspace_mode = mode;
    Dot {
        id: format!("dot-{name}"),
        webhook_token: "t".into(),
        created_at: "2026-10-08T00:00:00.000Z".into(),
        updated_at: "2026-10-08T00:00:00.000Z".into(),
        spec: s,
    }
}

pub fn fake_run(trigger: TriggerKind, payload: Option<Value>, session_id: Option<&str>) -> Run {
    Run {
        id: "RUN1".into(),
        dot_id: "dot-x".into(),
        root_run_id: "RUN1".into(),
        parent_run_id: None,
        trigger,
        payload,
        status: RunStatus::Running,
        session_id: session_id.map(str::to_string),
        workspace_path: None,
        branch: None,
        base_commit: None,
        summary: None,
        error: None,
        tokens_in: 0,
        tokens_out: 0,
        queued_at: "2026-10-08T00:00:00.000Z".into(),
        started_at: None,
        ended_at: None,
    }
}

use dots_core::Error as DotsError;

/// Polls until the run reaches `status` or `secs` elapse (then panics with the last state).
pub async fn wait_status(store: &Store, run_id: &str, status: RunStatus, secs: u64) -> Run {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let run = store.get_run(run_id).await.unwrap();
        if run.status == status {
            return run;
        }
        if std::time::Instant::now() > deadline {
            panic!("run {run_id} did not reach {status:?}; last state: {run:#?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

pub fn is_not_found(e: &DotsError) -> bool {
    matches!(e, DotsError::NotFound(_))
}
