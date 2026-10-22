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
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::rpc::Peer;
use super::{McpError, MAX_MESSAGE_BYTES};
use crate::proc::Job;
use crate::vault::redact::global;

/// The stderr bytes of one server that reach the log; the rest is read and dropped.
const STDERR_LOG_CAP: usize = 64 * 1024;
/// Messages waiting for the server to read them. A server that reads nothing fills it, and
/// then sending fails instead of growing the queue.
const OUTBOX_CAP: usize = 1024;

/// What `Stdio::start` runs, with every `${VAR}` already expanded.
pub(super) struct Launch<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<PathBuf>,
    pub env: &'a BTreeMap<String, String>,
    pub grace: Duration,
}

pub(super) struct Stdio {
    outbox: mpsc::Sender<String>,
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
        // Only the base of Dots' environment, then the server's own (env vault spec §8).
        crate::vault::child_env::start_clean(&mut cmd, crate::vault::child_env::Child::Other);
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
        let (outbox, inbox) = mpsc::channel(OUTBOX_CAP);
        tokio::spawn(write(stdin, inbox, peer.clone(), shutdown.clone()));
        tokio::spawn(read(stdout, peer.clone(), outbox.clone(), shutdown.clone()));
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
            .try_send(message.to_string())
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    McpError::Protocol("the server does not read what Dots sends".into())
                }
                mpsc::error::TrySendError::Closed(_) => McpError::Stopped,
            })
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
    mut inbox: mpsc::Receiver<String>,
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

/// What `read_line_capped` found.
enum Read {
    Eof,
    Got,
    TooLong,
}

/// Reads up to the next `\n` into `buf` (cleared first), at most `cap` bytes: a longer line is
/// not buffered, and `TooLong` is returned. The last line may lack its `\n`.
async fn read_line_capped<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<Read> {
    buf.clear();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(if buf.is_empty() { Read::Eof } else { Read::Got });
        }
        let (take, found) = match chunk.iter().position(|b| *b == b'\n') {
            Some(at) => (at + 1, true),
            None => (chunk.len(), false),
        };
        if buf.len() + take > cap {
            return Ok(Read::TooLong);
        }
        buf.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if found {
            return Ok(Read::Got);
        }
    }
}

/// Gives each line to the peer, and sends back its replies to requests. The end of stdout is
/// the end of the server, and so is a line longer than `MAX_MESSAGE_BYTES`: the server is
/// then shut down.
async fn read(
    stdout: ChildStdout,
    peer: Arc<Peer>,
    outbox: mpsc::Sender<String>,
    shutdown: CancellationToken,
) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        match read_line_capped(&mut reader, &mut buf, MAX_MESSAGE_BYTES).await {
            Ok(Read::Eof) | Err(_) => break,
            Ok(Read::TooLong) => {
                tracing::warn!(server = %peer.name(), "the MCP server wrote a message that is too large; stopping it");
                shutdown.cancel();
                break;
            }
            Ok(Read::Got) => {}
        }
        let line = buf.trim_ascii();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(message) => {
                if let Some(reply) = peer.receive(message) {
                    let _ = outbox.try_send(reply.to_string());
                }
            }
            Err(_) => tracing::debug!(server = %peer.name(), "a line that is not JSON on stdout"),
        }
    }
    peer.stop();
}

/// Cuts a server's stderr into the lines that reach the log, up to a cap. A line is scrubbed of
/// secret values before it is logged. Where the cap cuts a line in two, the cut is made after
/// the scrub, from text read past the cap by the length of the longest secret: a secret across
/// the cap would otherwise leave a piece that no scrub matches.
struct StderrLines {
    cap: usize,
    line: Vec<u8>,
    seen: usize,
    capped: bool,
}

impl StderrLines {
    fn new(cap: usize) -> StderrLines {
        StderrLines {
            cap,
            line: Vec::new(),
            seen: 0,
            capped: false,
        }
    }

    fn flush(&mut self, out: &mut Vec<String>) {
        if !self.line.is_empty() {
            let text = String::from_utf8_lossy(&self.line);
            out.push(global().scrub(&text).trim_end().to_string());
            self.line.clear();
        }
    }

    /// The lines that `bytes` completes.
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let limit = self.cap + global().longest();
        for &b in bytes {
            if self.capped {
                break;
            }
            if b == b'\n' {
                self.flush(&mut out);
            } else {
                self.line.push(b);
            }
            self.seen += 1;
            if self.seen >= limit {
                self.capped = true;
                if !self.line.is_empty() {
                    let text = String::from_utf8_lossy(&self.line);
                    out.push(global().scrub_clipped_tail(&text).trim_end().to_string());
                    self.line.clear();
                }
            }
        }
        out
    }

    /// The last line, when the stream ends inside it.
    fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        self.flush(&mut out);
        out
    }
}

/// Logs stderr line by line, up to `STDERR_LOG_CAP` bytes, and drains the rest, inside a line
/// too, so the server never blocks on a full pipe and nothing grows.
async fn log_stderr(stderr: ChildStderr, name: String) {
    let mut reader = BufReader::new(stderr);
    let mut lines = StderrLines::new(STDERR_LOG_CAP);
    let mut warned = false;
    let log = |lines: Vec<String>| {
        for text in lines {
            tracing::info!(server = %name, "MCP server stderr: {text}");
        }
    };
    while let Ok(chunk) = reader.fill_buf().await {
        if chunk.is_empty() {
            break;
        }
        let n = chunk.len();
        log(lines.feed(chunk));
        if lines.capped && !warned {
            warned = true;
            tracing::warn!(server = %name, "the MCP server writes more to stderr; the rest is not logged");
        }
        reader.consume(n);
    }
    log(lines.finish());
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

#[cfg(test)]
mod stderr_tests {
    use super::*;

    /// Whether `text` holds any 8 bytes of `secret` in a row.
    fn holds_piece(text: &str, secret: &str) -> bool {
        secret
            .as_bytes()
            .windows(8)
            .any(|w| text.contains(std::str::from_utf8(w).unwrap()))
    }

    #[test]
    fn a_secret_across_the_cap_leaves_no_piece_in_the_log() {
        let secret = "stderr-cap-secret-0123456789";
        global().add("DOTS_STDERR_CAP", secret);
        for pad in 0..secret.len() {
            let mut lines = StderrLines::new(40);
            // The log stops `longest()` bytes past the cap; the secret straddles that point.
            let limit = 40 + global().longest();
            let input = format!("{}{secret}{}", "x".repeat(limit - pad), "z".repeat(200));
            let mut out = lines.feed(input.as_bytes());
            out.extend(lines.finish());
            let all = out.join("\n");
            assert!(!holds_piece(&all, secret), "pad {pad}: {all}");
            assert!(lines.capped);
        }
    }

    #[test]
    fn whole_lines_are_scrubbed_and_the_last_one_is_kept() {
        let secret = "stderr-line-secret-9876543210";
        global().add("DOTS_STDERR_LINE", secret);
        let mut lines = StderrLines::new(4096);
        let mut out = lines.feed(format!("a {secret}\nb").as_bytes());
        out.extend(lines.finish());
        assert_eq!(out, ["a [secret:DOTS_STDERR_LINE]", "b"]);
    }
}
