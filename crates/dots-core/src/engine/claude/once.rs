//! One request to the `claude` CLI outside a dot run (skill creator spec §5): no tools, no MCP
//! servers, no settings files and no saved session. The answer's text comes back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
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
    /// There is no CLI to ask (`OnceRunner` only; `ask_once` is given one).
    NoCli,
}

/// What a request without a CLI tells the person.
pub const NO_CLI: &str = "claude CLI not found: install Claude Code or set its path in Settings";

/// The requests being answered, at most one per id: the skill creator's drafts and the setup
/// agent's requests. Each runs `ask_once` in a new empty folder of its own under `dir`, removed
/// afterwards, and ends on `cancel`, on a newer request of the same id, on the time limit or
/// when Dots shuts down. Requests of other ids are left alone.
pub struct OnceRunner {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
    dir: PathBuf,
    limit: Duration,
    shutdown: CancellationToken,
    /// By id: a number for the request, and what cancels it.
    running: Mutex<HashMap<String, (u64, CancellationToken)>>,
    next: Mutex<u64>,
}

impl OnceRunner {
    /// `program` is the CLI the runtime found, `None` when there is none.
    pub fn new(
        program: Option<PathBuf>,
        env: Vec<(String, String)>,
        dir: PathBuf,
        limit: Duration,
        shutdown: CancellationToken,
    ) -> OnceRunner {
        OnceRunner {
            program,
            env,
            dir,
            limit,
            shutdown,
            running: Mutex::new(HashMap::new()),
            next: Mutex::new(0),
        }
    }

    /// Asks the CLI for `id` and returns the answer's text. The id's older request, if one is
    /// running, is cancelled first, also when there is no CLI (`NoCli`).
    pub async fn run(
        &self,
        id: &str,
        system_prompt: &str,
        model: &str,
        prompt: &str,
    ) -> Result<String, OnceFailure> {
        let Some(program) = self.program.as_deref() else {
            self.cancel(id);
            return Err(OnceFailure::NoCli);
        };
        let token = self.shutdown.child_token();
        let number = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        };
        let previous = self
            .running
            .lock()
            .unwrap()
            .insert(id.to_string(), (number, token.clone()));
        if let Some((_, previous)) = previous {
            previous.cancel();
        }
        let cwd = self.dir.join(crate::util::new_id());
        let asked = match std::fs::create_dir_all(&cwd) {
            Ok(()) => {
                let once = Once {
                    program,
                    env: &self.env,
                    cwd: &cwd,
                    model,
                    system_prompt,
                    prompt,
                    limit: self.limit,
                };
                let asked = ask_once(&once, &token).await;
                let _ = std::fs::remove_dir_all(&cwd);
                asked
            }
            Err(e) => Err(OnceFailure::Failed(format!(
                "could not make {}: {e}",
                cwd.display()
            ))),
        };
        let mut running = self.running.lock().unwrap();
        if running.get(id).is_some_and(|(n, _)| *n == number) {
            running.remove(id);
        }
        asked
    }

    /// Cancels the id's request, if one is running.
    pub fn cancel(&self, id: &str) {
        if let Some((_, token)) = self.running.lock().unwrap().remove(id) {
            token.cancel();
        }
    }
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

/// What the output said: the result's text, the error, the last assistant text and the usage
/// the CLI reported last.
#[derive(Default)]
struct Read {
    answer: Option<String>,
    failure: Option<String>,
    last_text: Option<String>,
    usage: Option<OnceUsage>,
}

/// What a request used, as the CLI's result reported it (`EngineEvent::Usage`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OnceUsage {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cache_read: u64,
    pub cost_usd: Option<f64>,
}

/// An answer with what it used; `usage` is `None` when the CLI reported none.
#[derive(Debug, Clone, PartialEq)]
pub struct OnceAnswer {
    pub text: String,
    pub usage: Option<OnceUsage>,
}

/// Runs the request and returns the answer: the result's text, else the last assistant text.
/// Cancelling `cancel`, or `limit` passing, ends the CLI and everything it started.
pub async fn ask_once(req: &Once<'_>, cancel: &CancellationToken) -> Result<String, OnceFailure> {
    ask_once_with_usage(req, cancel).await.map(|a| a.text)
}

/// `ask_once`, with the usage the CLI reported for the request.
pub async fn ask_once_with_usage(
    req: &Once<'_>,
    cancel: &CancellationToken,
) -> Result<OnceAnswer, OnceFailure> {
    run_once(req, cancel, &[], None).await
}

/// The flag that makes the CLI write each piece of text as it is made (`stream_event` lines).
const PARTIAL_MESSAGES: &str = "--include-partial-messages";

/// `ask_once_with_usage` with the answer's text streamed: `on_text` gets each piece of text as
/// the CLI writes it, while it still runs, in order. When the CLI wrote no piece, `on_text`
/// gets the whole answer once at the end (unless it is empty), so a caller always sees the
/// reply through `on_text`. Same command line as `once_args`, plus
/// `--include-partial-messages`.
///
/// The two outputs have different jobs. `on_text` receives what should be spoken, as it
/// arrives: pieces already handed over stay handed over, even when the request then fails,
/// times out or is cancelled. `OnceAnswer.text` is the authoritative full reply, and the one to
/// parse (for its `DO` line, say); only an `Ok` carries it. A caller that speaks the pieces
/// must still read the result: a failed `result` after some pieces is `Failed`.
pub async fn ask_once_streamed(
    req: &Once<'_>,
    cancel: &CancellationToken,
    on_text: &mut (dyn FnMut(&str) + Send),
) -> Result<OnceAnswer, OnceFailure> {
    let mut streamed = false;
    let mut each = |text: &str| {
        streamed = true;
        on_text(text);
    };
    let answer = run_once(req, cancel, &[PARTIAL_MESSAGES], Some(&mut each)).await?;
    if !streamed && !answer.text.is_empty() {
        on_text(&answer.text);
    }
    Ok(answer)
}

/// The text of a partial-message line: a `stream_event` whose event is a
/// `content_block_delta` with a `text_delta`. Nothing else (thinking or tool input deltas,
/// message starts and stops, other lines) and no empty text.
pub fn partial_text(line: &str) -> Option<String> {
    if !line.contains("stream_event") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "stream_event" {
        return None;
    }
    let event = v.get("event")?;
    if event.get("type")?.as_str()? != "content_block_delta" {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    let text = delta.get("text")?.as_str()?;
    (!text.is_empty()).then(|| text.to_string())
}

/// Runs a request: `once_args` plus `extra`. With `on_text`, each partial text the output
/// carries (`partial_text`) is handed to it as it is read, while the CLI runs.
async fn run_once(
    req: &Once<'_>,
    cancel: &CancellationToken,
    extra: &[&str],
    mut on_text: Option<&mut (dyn FnMut(&str) + Send + '_)>,
) -> Result<OnceAnswer, OnceFailure> {
    let mut cmd = Command::new(req.program);
    crate::vault::child_env::start_clean(&mut cmd, crate::vault::child_env::Child::Cli);
    cmd.args(once_args(req.model, req.system_prompt))
        .args(extra)
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
    // Partial texts go from the reader to `on_text` over this channel, in order.
    let (partials, mut partial_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let streaming = on_text.is_some();
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
            let line = text.trim_end_matches(['\r', '\n']);
            if streaming {
                if let Some(piece) = partial_text(line) {
                    let _ = partials.send(piece);
                    continue;
                }
            }
            for ev in parse::parse_line(line) {
                match ev {
                    EngineEvent::Finished { summary } => read.answer = Some(summary),
                    EngineEvent::Failed { error } => read.failure = Some(error),
                    EngineEvent::AssistantText { text } => read.last_text = Some(text),
                    EngineEvent::Usage {
                        tokens_in,
                        tokens_out,
                        cache_read,
                        cost_usd,
                    } => {
                        read.usage = Some(OnceUsage {
                            tokens_in,
                            tokens_out,
                            cache_read,
                            cost_usd,
                        })
                    }
                    _ => {}
                }
            }
        }
        read
    });
    let mut stderr_task = tokio::spawn(crate::proc::read_scrubbed_tail(stderr, STDERR_TAIL));

    let deadline = tokio::time::sleep(req.limit);
    tokio::pin!(deadline);
    let end = loop {
        // A cancel (also from `on_text`) or the time limit wins over more text.
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break End::Cancelled,
            _ = &mut deadline => break End::TimedOut,
            Some(piece) = partial_rx.recv() => {
                if let Some(on_text) = on_text.as_mut() {
                    on_text(&piece);
                }
            }
            s = child.wait() => break End::Exited(s.ok()),
        }
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
    // The text read after the CLI exited, in order: the reader is done, so all of it is here.
    if let Some(on_text) = on_text.as_mut() {
        while let Ok(piece) = partial_rx.try_recv() {
            on_text(&piece);
        }
    }
    if let Some(error) = read.failure {
        return Err(OnceFailure::Failed(error));
    }
    let usage = read.usage;
    let answered = |text: String| Ok(OnceAnswer { text, usage });
    match (read.answer, read.last_text) {
        (Some(answer), _) if !answer.trim().is_empty() => answered(answer),
        (Some(_), last) => answered(last.unwrap_or_default()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_text_reads_text_deltas_only() {
        let text = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}},"session_id":"s","uuid":"u"}"#;
        assert_eq!(partial_text(text), Some("Hel".to_string()));
        let minimal = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"lo there"}}}"#;
        assert_eq!(partial_text(minimal), Some("lo there".to_string()));
        for line in [
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"a\""}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"x"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":3}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"stream_event"}]}}"#,
            r#"{"type":"result","subtype":"success","result":"text_delta stream_event"}"#,
            r#"{"type":"stream_event""#,
            "stream_event",
            "",
        ] {
            assert_eq!(partial_text(line), None, "{line}");
        }
    }

    #[test]
    fn plain_requests_do_not_ask_for_partials() {
        assert!(!once_args("haiku", "sys").contains(&PARTIAL_MESSAGES.to_string()));
    }
}
