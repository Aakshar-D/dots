//! The stdio transport: the server is a child process, one JSON-RPC message per line on its
//! stdin and stdout. It runs with no console window and in a Job Object (`proc::Job`), so
//! what it starts goes with it. Its stderr goes to the app log, capped.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Stdio as Piped;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::rpc::Peer;
use super::McpError;
use crate::proc::Job;

/// The stderr bytes of one server that reach the log; the rest is read and dropped.
const STDERR_LOG_CAP: usize = 64 * 1024;

/// What `Stdio::start` runs, with every `${VAR}` already expanded.
pub(super) struct Launch<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<PathBuf>,
    pub env: &'a BTreeMap<String, String>,
    pub grace: Duration,
}

pub(super) struct Stdio {
    outbox: mpsc::UnboundedSender<String>,
    /// Fired by `close`, or by dropping this: the server's input closes, and it is stopped.
    shutdown: CancellationToken,
    _on_drop: DropGuard,
    /// The task that owns the process; it ends once the process has gone.
    monitor: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Stdio {
    pub(super) fn start(peer: Arc<Peer>, launch: Launch<'_>) -> Result<Stdio, McpError> {
        let program = resolve_program(
            launch.command,
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("PATHEXT").as_deref(),
        );
        let mut cmd = Command::new(&program);
        cmd.args(launch.args)
            .envs(launch.env)
            .stdin(Piped::piped())
            .stdout(Piped::piped())
            .stderr(Piped::piped())
            .kill_on_drop(true);
        if let Some(dir) = &launch.cwd {
            cmd.current_dir(dir);
        }
        crate::proc::hide_window(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            McpError::Start(match e.kind() {
                std::io::ErrorKind::NotFound => format!("{} was not found", launch.command),
                _ => format!("{}: {e}", launch.command),
            })
        })?;
        // Before the server reads anything: what it starts ahead of this is outside the job.
        let job = Job::assign(&child).unwrap_or_else(|e| {
            tracing::warn!(server = %peer.name(), "no job object for the MCP server, using kill_tree: {e}");
            Job::fallback(&child)
        });
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let shutdown = CancellationToken::new();
        let (outbox, inbox) = mpsc::unbounded_channel();
        tokio::spawn(write(stdin, inbox, peer.clone(), shutdown.clone()));
        tokio::spawn(read(stdout, peer.clone(), outbox.clone()));
        tokio::spawn(log_stderr(stderr, peer.name().to_string()));
        let monitor = tokio::spawn(monitor(child, job, peer, shutdown.clone(), launch.grace));
        Ok(Stdio {
            outbox,
            _on_drop: shutdown.clone().drop_guard(),
            shutdown,
            monitor: tokio::sync::Mutex::new(Some(monitor)),
        })
    }

    pub(super) fn send(&self, message: &Value) -> Result<(), McpError> {
        self.outbox
            .send(message.to_string())
            .map_err(|_| McpError::Stopped)
    }

    pub(super) async fn close(&self) {
        self.shutdown.cancel();
        if let Some(monitor) = self.monitor.lock().await.take() {
            let _ = monitor.await;
        }
    }
}

/// Writes each message as one line. On shutdown it drops stdin, which ends the server's input.
async fn write(
    mut stdin: ChildStdin,
    mut inbox: mpsc::UnboundedReceiver<String>,
    peer: Arc<Peer>,
    shutdown: CancellationToken,
) {
    loop {
        let line = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            line = inbox.recv() => match line {
                Some(line) => line,
                None => return,
            },
        };
        let written = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        };
        if let Err(e) = written.await {
            tracing::warn!(server = %peer.name(), "writing to the MCP server failed: {e}");
            peer.stop();
            return;
        }
    }
}

/// Gives each line to the peer, and sends back its replies to requests. The end of stdout is
/// the end of the server.
async fn read(stdout: ChildStdout, peer: Arc<Peer>, outbox: mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = buf.trim_ascii();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(message) => {
                if let Some(reply) = peer.receive(message) {
                    let _ = outbox.send(reply.to_string());
                }
            }
            Err(_) => tracing::debug!(server = %peer.name(), "a line that is not JSON on stdout"),
        }
    }
    peer.stop();
}

/// Logs stderr line by line, up to `STDERR_LOG_CAP` bytes, and drains the rest so the server
/// never blocks on a full pipe.
async fn log_stderr(stderr: ChildStderr, name: String) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    let mut logged = 0;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) if logged < STDERR_LOG_CAP => {
                logged += n;
                let text = String::from_utf8_lossy(&buf);
                tracing::info!(server = %name, "MCP server stderr: {}", text.trim_end());
                if logged >= STDERR_LOG_CAP {
                    tracing::warn!(server = %name, "the MCP server writes more to stderr; the rest is not logged");
                }
            }
            Ok(_) => {}
        }
    }
}

/// Owns the process until it has gone. On shutdown the server has `grace` to exit by itself
/// once its input is closed (the writer closes it on the same signal), then the job is
/// terminated. Either way the job is closed after, so nothing the server started stays. A
/// server that exits by itself is stopped by the reader, at the end of its stdout, so that
/// answers it wrote before it exited still arrive.
async fn monitor(
    mut child: Child,
    job: Job,
    peer: Arc<Peer>,
    shutdown: CancellationToken,
    grace: Duration,
) {
    let closing = tokio::select! {
        status = child.wait() => {
            tracing::warn!(server = %peer.name(), "the MCP server exited: {status:?}");
            false
        }
        _ = shutdown.cancelled() => true,
    };
    if closing && tokio::time::timeout(grace, child.wait()).await.is_err() {
        job.terminate().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    job.close();
    if closing {
        peer.stop();
    }
}

/// The program to start for `command`. On Windows a bare name (no folder, no extension) is
/// looked up as cmd.exe does: in each folder of `path`, with each extension of `pathext`
/// (default `.COM;.EXE;.BAT;.CMD`), so `npx` finds `npx.cmd`. Otherwise, and when nothing is
/// found, the command as written.
pub fn resolve_program(command: &str, path: Option<&OsStr>, pathext: Option<&OsStr>) -> PathBuf {
    #[cfg(windows)]
    {
        let bare = std::path::Path::new(command);
        if bare.components().count() == 1 && bare.extension().is_none() {
            let exts = pathext
                .and_then(OsStr::to_str)
                .unwrap_or(".COM;.EXE;.BAT;.CMD")
                .to_ascii_lowercase();
            for dir in path.map(std::env::split_paths).into_iter().flatten() {
                for ext in exts.split(';').filter(|e| !e.is_empty()) {
                    let candidate = dir.join(format!("{command}{ext}"));
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (path, pathext);
    PathBuf::from(command)
}
