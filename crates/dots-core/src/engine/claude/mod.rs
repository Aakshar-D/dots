use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::engine::{Engine, EngineEvent, RunContext};
use crate::vault::child_env::{is_cli_own, start_clean, Child};
use crate::{Error, Result};

pub mod args;
pub mod once;
pub mod parse;

/// The installed CLI's version from `claude --version` (`2.1.295 (Claude Code)` gives
/// `2.1.295`). `None` when it fails or takes longer than 5 s.
pub async fn cli_version(program: &Path, env: &[(String, String)]) -> Option<String> {
    let mut cmd = Command::new(program);
    start_clean(&mut cmd, Child::Cli);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    crate::proc::hide_window(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().next().map(str::to_string)
}

const STDERR_TAIL: usize = 4096;

pub struct ClaudeEngine {
    program: PathBuf,
    scratch_dir: PathBuf,
    env: Vec<(String, String)>,
}

impl ClaudeEngine {
    pub fn new(program: PathBuf, scratch_dir: PathBuf) -> Self {
        Self {
            program,
            scratch_dir,
            env: Vec::new(),
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }
}

/// Writes a file readable only by the owner on unix (it holds a bearer secret).
async fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut f = opts.open(path).await?;
    f.write_all(data).await?;
    f.flush().await
}

#[async_trait]
impl Engine for ClaudeEngine {
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>> {
        tokio::fs::create_dir_all(&self.scratch_dir).await?;
        let mcp_path = self.scratch_dir.join(format!("mcp-{}.json", ctx.run_id));
        write_private(
            &mcp_path,
            &serde_json::to_vec_pretty(&args::mcp_config(&ctx))?,
        )
        .await?;

        let mut cmd = Command::new(&self.program);
        // Dots' environment stays out; the dot's shell grants go in (env vault spec §8).
        start_clean(&mut cmd, Child::Cli);
        // The CLI's own variables (its login, its endpoint) are never set from Personal: it keeps
        // what Dots' environment passed through.
        let mut notices = Vec::new();
        for (k, v) in ctx.tools.shell_env() {
            if is_cli_own(k) {
                notices.push(format!(
                    "{k}: the claude CLI's own variable, not set from Personal"
                ));
            } else {
                cmd.env(k.as_str(), v.as_str());
            }
        }
        cmd.args(args::build_args(&ctx, &mcp_path))
            .current_dir(&ctx.workspace)
            .env(
                "MCP_TOOL_TIMEOUT",
                ((ctx.dot.spec.approval_wait_secs + 30) * 1000).to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        crate::proc::hide_window(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tokio::fs::remove_file(&mcp_path).await;
                return Err(Error::Other(format!(
                    "failed to start {}: {e}",
                    self.program.display()
                )));
            }
        };
        // Before anything else: what claude starts ahead of this is outside the job.
        let job = crate::proc::Job::assign(&child).unwrap_or_else(|e| {
            tracing::warn!(run = %ctx.run_id, "no job object for claude, using kill_tree: {e}");
            crate::proc::Job::fallback(&child)
        });
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let (tx, rx) = mpsc::channel(256);
        for text in notices {
            let _ = tx.try_send(EngineEvent::Notice { text });
        }

        tokio::spawn(async move {
            let out_tx = tx.clone();
            let saw_terminal = Arc::new(AtomicBool::new(false));
            let flag = saw_terminal.clone();
            let run_id = ctx.run_id.clone();
            let mut stdout_task = tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut buf: Vec<u8> = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let text = String::from_utf8_lossy(&buf);
                    let line = text.trim_end_matches(['\r', '\n']);
                    for ev in parse::parse_line(line) {
                        // A plugin the CLI could not load (`system/init`'s `plugin_errors`).
                        if let EngineEvent::Notice { text } = &ev {
                            tracing::warn!(run = %run_id, "claude: {text}");
                        }
                        let terminal = matches!(
                            ev,
                            EngineEvent::Finished { .. } | EngineEvent::Failed { .. }
                        );
                        if out_tx.send(ev).await.is_err() {
                            return;
                        }
                        if terminal {
                            flag.store(true, Ordering::SeqCst);
                        }
                    }
                }
            });
            let mut stderr_task = tokio::spawn(crate::proc::read_tail(stderr, STDERR_TAIL));

            let prompt = ctx.prompt.clone();
            let run_id = ctx.run_id.clone();
            tokio::spawn(async move {
                if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
                    tracing::warn!(run = %run_id, "writing prompt to claude failed: {e}");
                }
                drop(stdin);
            });

            let mut status = None;
            let cancelled = tokio::select! {
                s = child.wait() => { status = s.ok(); false }
                _ = ctx.cancel.cancelled() => true,
            };
            if cancelled {
                job.terminate().await;
                let _ = child.start_kill();
                status = child.wait().await.ok();
            }
            // Whatever claude left running goes now: it would hold the pipes read below open,
            // and must be gone before the run is reported as over.
            job.close();

            let cap = Duration::from_secs(5);
            if tokio::time::timeout(cap, &mut stdout_task).await.is_err() {
                stdout_task.abort();
            }
            let tail = match tokio::time::timeout(cap, &mut stderr_task).await {
                Ok(Ok(t)) => t,
                Ok(Err(_)) => String::new(),
                Err(_) => {
                    stderr_task.abort();
                    String::new()
                }
            };
            let terminal = saw_terminal.load(Ordering::SeqCst);
            if !terminal && !cancelled {
                let code = status
                    .and_then(|s| s.code())
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let error = if tail.trim().is_empty() {
                    format!("claude exited with code {code} without a result")
                } else {
                    format!("claude exited with code {code}: {}", tail.trim())
                };
                let _ = tx.send(EngineEvent::Failed { error }).await;
            }
            let _ = tokio::fs::remove_file(&mcp_path).await;
        });
        Ok(rx)
    }
}
