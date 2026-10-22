//! One request to the `claude` CLI outside a dot run (skill creator spec §5): no tools, no MCP
//! servers, no settings files and no saved session. The answer's text comes back.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::parse;
use crate::engine::EngineEvent;

const STDERR_TAIL: usize = 4096;

/// A request: the CLI, its environment and folder, and what to ask.
pub struct Once<'a> {
    pub program: &'a Path,
    pub env: &'a [(String, String)],
    pub cwd: &'a Path,
    pub model: &'a str,
    pub system_prompt: &'a str,
    /// Sent on stdin.
    pub prompt: &'a str,
    /// The process is ended after this long.
    pub limit: Duration,
}

/// Why a request gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnceFailure {
    Cancelled,
    TimedOut,
    /// The CLI failed: its error, or how it exited.
    Failed(String),
}

/// The command line of a request: one turn, no built-in tools (`--tools ""`), no MCP servers
/// (`--strict-mcp-config` without `--mcp-config`), none of the person's settings, hooks or
/// `CLAUDE.md` files (`--setting-sources ""`), and the default system prompt replaced.
pub fn once_args(model: &str, system_prompt: &str) -> Vec<String> {
    [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        model,
        "--max-turns",
        "1",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--system-prompt",
        system_prompt,
    ]
    .map(String::from)
    .to_vec()
}

enum End {
    Exited(Option<std::process::ExitStatus>),
    Cancelled,
    TimedOut,
}

/// What the output said: the result's text, the error, and the last assistant text.
#[derive(Default)]
struct Read {
    answer: Option<String>,
    failure: Option<String>,
    last_text: Option<String>,
}

/// Runs the request and returns the answer: the result's text, else the last assistant text.
/// Cancelling `cancel`, or `limit` passing, ends the CLI and everything it started.
pub async fn ask_once(req: &Once<'_>, cancel: &CancellationToken) -> Result<String, OnceFailure> {
    let mut cmd = Command::new(req.program);
    crate::vault::child_env::start_clean(&mut cmd, crate::vault::child_env::Child::Cli);
    cmd.args(once_args(req.model, req.system_prompt))
        .current_dir(req.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in req.env {
        cmd.env(k, v);
    }
    crate::proc::hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        OnceFailure::Failed(format!("failed to start {}: {e}", req.program.display()))
    })?;
    let job = crate::proc::Job::assign(&child).unwrap_or_else(|e| {
        tracing::warn!("no job object for claude, using kill_tree: {e}");
        crate::proc::Job::fallback(&child)
    });
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let prompt = req.prompt.to_string();
    tokio::spawn(async move {
        if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
            tracing::warn!("writing the request to claude failed: {e}");
        }
        drop(stdin);
    });
    let mut reading = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();
        let mut read = Read::default();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&buf);
            for ev in parse::parse_line(text.trim_end_matches(['\r', '\n'])) {
                match ev {
                    EngineEvent::Finished { summary } => read.answer = Some(summary),
                    EngineEvent::Failed { error } => read.failure = Some(error),
                    EngineEvent::AssistantText { text } => read.last_text = Some(text),
                    _ => {}
                }
            }
        }
        read
    });
    let mut stderr_task = tokio::spawn(crate::proc::read_tail(stderr, STDERR_TAIL));

    let end = tokio::select! {
        s = child.wait() => End::Exited(s.ok()),
        _ = cancel.cancelled() => End::Cancelled,
        _ = tokio::time::sleep(req.limit) => End::TimedOut,
    };
    if !matches!(end, End::Exited(_)) {
        job.terminate().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    // Whatever claude left running goes now: it would hold the pipes read below open.
    job.close();
    let cap = Duration::from_secs(5);
    let read = match tokio::time::timeout(cap, &mut reading).await {
        Ok(Ok(read)) => read,
        _ => {
            reading.abort();
            Read::default()
        }
    };
    let tail = match tokio::time::timeout(cap, &mut stderr_task).await {
        Ok(Ok(t)) => t,
        _ => {
            stderr_task.abort();
            String::new()
        }
    };
    let status = match end {
        End::Cancelled => return Err(OnceFailure::Cancelled),
        End::TimedOut => return Err(OnceFailure::TimedOut),
        End::Exited(status) => status,
    };
    if let Some(error) = read.failure {
        return Err(OnceFailure::Failed(error));
    }
    match (read.answer, read.last_text) {
        (Some(answer), _) if !answer.trim().is_empty() => Ok(answer),
        (Some(_), last) => Ok(last.unwrap_or_default()),
        (None, _) => {
            let code = status
                .and_then(|s| s.code())
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Err(OnceFailure::Failed(if tail.trim().is_empty() {
                format!("claude exited with code {code} without a result")
            } else {
                format!("claude exited with code {code}: {}", tail.trim())
            }))
        }
    }
}
