#![allow(dead_code)]

pub mod clock;
pub mod edge_pipe;
pub mod fake_browser;
pub mod fake_mcp;
pub mod playwright;
pub mod secrets;
pub mod tz;

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
        has_webhook_token: true,
        created_at: "2026-10-08T00:00:00.000Z".into(),
        updated_at: "2026-10-08T00:00:00.000Z".into(),
        seen_at: None,
        org_id: None,
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
        reviewed_at: None,
        retry_of: None,
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

/// The directory of one lingering process of the fake CLI (`fake-claude --linger`, started by
/// its `--orphan` and `--tree` modes and by the `orphan` script step). The process is watched
/// through the lock it holds, not through its process id, and is told to stop when this is
/// dropped, so a failed test leaves nothing running and kills nothing.
pub struct Lingerer {
    dir: TempDir,
}

impl Lingerer {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The directory, as the fake CLI takes it.
    pub fn dir(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    /// Whether the process is running: it holds a lock on `alive` for as long as it lives.
    pub fn alive(&self) -> bool {
        match std::fs::File::open(self.dir.path().join("alive")) {
            Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            Err(_) => false,
        }
    }

    /// Waits until the process runs.
    pub async fn started(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !self.dir.path().join("ready").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the lingering process did not start"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(self.alive(), "the lingering process should be running");
    }

    /// Whether the process ends within `limit`.
    pub async fn gone_within(&self, limit: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + limit;
        while self.alive() {
            if std::time::Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        true
    }
}

impl Drop for Lingerer {
    fn drop(&mut self) {
        let _ = std::fs::write(self.dir.path().join("stop"), "");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.alive() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
