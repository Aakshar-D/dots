//! The test MCP server `fake-mcp` (`src/bin/fake-mcp.rs`): a script file in a temp folder, its
//! call log, and the server over stdio or Streamable HTTP.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use dots_core::library::McpServerConfig;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// A tool as `fake-mcp` lists it. Add `result`, `error`, `delay_ms`, `exit`, `switch` or
/// `ask_client` to script what a call does.
pub fn tool(name: &str, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": format!("The {name} tool."),
        "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
        "annotations": {"readOnlyHint": read_only}
    })
}

/// One `fake-mcp` script. Its log and lock files are kept next to it.
pub struct FakeMcp {
    dir: TempDir,
}

impl FakeMcp {
    /// Writes `script` (see `fake-mcp.rs` for its fields) with `log` and `alive` set.
    pub fn new(mut script: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        script["log"] = json!(dir.path().join("log.jsonl"));
        script["alive"] = json!(dir.path().join("alive"));
        std::fs::write(dir.path().join("script.json"), script.to_string()).unwrap();
        Self { dir }
    }

    /// Replaces the script, keeping its log and lock files: a server started after this lists
    /// what `script` says.
    pub fn rewrite(&self, mut script: Value) {
        script["log"] = json!(self.dir.path().join("log.jsonl"));
        script["alive"] = json!(self.dir.path().join("alive"));
        std::fs::write(self.script(), script.to_string()).unwrap();
    }

    pub fn script(&self) -> PathBuf {
        self.dir.path().join("script.json")
    }

    /// A stdio config that starts `fake-mcp` on this script.
    pub fn stdio(&self) -> McpServerConfig {
        self.stdio_with(Vec::new(), BTreeMap::new())
    }

    /// As `stdio`, with more arguments after the script and an environment.
    pub fn stdio_with(&self, args: Vec<String>, env: BTreeMap<String, String>) -> McpServerConfig {
        let mut all = vec![self.script().to_string_lossy().to_string()];
        all.extend(args);
        McpServerConfig::Stdio {
            command: env!("CARGO_BIN_EXE_fake-mcp").to_string(),
            args: all,
            cwd: None,
            env,
        }
    }

    /// Starts `fake-mcp --http` on this script; it listens on 127.0.0.1 until dropped.
    pub async fn serve_http(&self) -> FakeHttp {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-mcp"));
        cmd.arg(self.script())
            .arg("--http")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        dots_core::proc::hide_window(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let url = tokio::time::timeout(Duration::from_secs(20), lines.next_line())
            .await
            .expect("fake-mcp did not print its URL")
            .unwrap()
            .expect("fake-mcp ended before it printed its URL");
        FakeHttp { child, url }
    }

    /// Every record of the call log: `{"start": ...}`, `{"msg": ...}` (with `http`, `session`,
    /// `protocol` and `auth` over http), `{"http": "GET"|"DELETE", ...}`.
    pub fn log(&self) -> Vec<Value> {
        match std::fs::read_to_string(self.dir.path().join("log.jsonl")) {
            Ok(text) => text
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The messages received with `method`, in order.
    pub fn received(&self, method: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter(|r| r["msg"]["method"] == method)
            .map(|r| r["msg"].clone())
            .collect()
    }

    /// Waits until the log has a record for which `pred` holds.
    pub async fn wait_for(&self, limit: Duration, pred: impl Fn(&Value) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if self.log().iter().any(&pred) {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Whether a `fake-mcp` process of this script runs: it holds a lock on `alive`.
    pub fn alive(&self) -> bool {
        match std::fs::File::open(self.dir.path().join("alive")) {
            Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            Err(_) => false,
        }
    }

    /// Whether the process ends within `limit`.
    pub async fn gone_within(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while self.alive() {
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    }
}

/// `fake-mcp` in http mode. Killed when dropped.
pub struct FakeHttp {
    pub child: Child,
    /// `http://127.0.0.1:<port>/mcp`
    pub url: String,
}

impl FakeHttp {
    pub fn config(&self, headers: BTreeMap<String, String>) -> McpServerConfig {
        McpServerConfig::Http {
            url: self.url.clone(),
            headers,
        }
    }
}
